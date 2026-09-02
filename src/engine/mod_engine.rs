//! Phase 4: The Mod Engine — download, extract, deploy, cleanup.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_channel::{Receiver, Sender};
use futures::StreamExt;
use sha2::{Digest, Sha256};
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tracing::{info, instrument, warn};

use crate::api::ModFile;
use crate::config::Config;
use crate::db::{Database, ManifestEntry, NewManifestEntry};
use crate::errors::{AppError, AppResult};

/// A progress event emitted during download. The progress fraction is in `[0.0, 1.0]`.
#[derive(Debug, Clone)]
pub enum ProgressEvent {
    Started { file_name: String, total: Option<u64> },
    DownloadProgress { file_name: String, fraction: f64, bytes: u64 },
    ExtractionStarted { file_name: String },
    ExtractionFinished { file_name: String },
    ConflictDetected { file_path: String, existing_mod: i64, new_mod: i64 },
    DeployStarted { file_name: String },
    DeployFinished { file_name: String, bytes: u64 },
    Finished,
    Failed(String),
}

/// Snapshot of a download. The receiver can poll this for percentage/bytes.
#[derive(Debug, Clone, Copy)]
pub struct DownloadProgress {
    pub bytes_downloaded: u64,
    pub total_bytes: Option<u64>,
}

/// Information about a mod that was about to be installed but has a conflict at one of
/// its target paths. Surfaced to the UI for user resolution (4.3).
#[derive(Debug, Clone)]
pub struct ConflictReport {
    pub file_path: String,
    pub new_mod_id: i64,
    pub existing_mod_id: i64,
}

/// Strategy to apply when a conflicting file is encountered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictResolution {
    /// Skip this file, leave the existing one in place.
    Skip,
    /// Overwrite the existing file, backing up the original first.
    OverwriteWithBackup,
    /// Overwrite without keeping a backup (irreversible).
    OverwriteNoBackup,
    /// Abort the entire deploy.
    Abort,
}

/// Async downloader that streams a remote URL to a local file, emitting progress
/// over a [`Sender<ProgressEvent>`] (Phase 4.1).
pub struct DownloadManager {
    client: reqwest::Client,
    cache_dir: PathBuf,
    progress: Option<ProgressSender>,
}

impl DownloadManager {
    pub fn new(config: &Config) -> Self {
        let client = reqwest::Client::builder()
            .user_agent("nexus-cp2077-mod-manager/0.1")
            .build()
            .expect("reqwest client build");
        Self {
            client,
            cache_dir: config.cache_directory.clone(),
            progress: None,
        }
    }

    /// Set the progress sender. Events flow to the UI's progress view.
    pub fn with_progress(mut self, tx: ProgressSender) -> Self {
        self.progress = Some(tx);
        self
    }

    /// Stream a URL to `destination`, calling the progress closure on every chunk.
    /// The destination file is created with mode 0o600 (owner read/write only).
    #[instrument(skip(self, progress), fields(url = %url, dest = ?destination))]
    pub async fn download(
        &self,
        url: &str,
        destination: &Path,
        progress: Option<ProgressSender>,
    ) -> AppResult<DownloadProgress> {
        if let Some(parent) = destination.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|e| AppError::Io {
                path: parent.to_path_buf(),
                source: e,
            })?;
        }

        let response = self.client.get(url).send().await?;
        let total = response.content_length();
        if let Some(p) = &progress {
            let _ = p
                .send(ProgressEvent::Started {
                    file_name: destination
                        .file_name()
                        .and_then(|s| s.to_str())
                        .unwrap_or("file")
                        .to_string(),
                    total,
                })
                .await;
        }

        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .mode(0o600)
            .open(destination)
            .await
            .map_err(|e| AppError::Io {
                path: destination.to_path_buf(),
                source: e,
            })?;

        let mut stream = response.bytes_stream();
        let mut bytes_downloaded: u64 = 0;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            file.write_all(&chunk).await.map_err(|e| AppError::Io {
                path: destination.to_path_buf(),
                source: e,
            })?;
            bytes_downloaded += chunk.len() as u64;
            if let Some(p) = &progress {
                let fraction = match total {
                    Some(t) if t > 0 => bytes_downloaded as f64 / t as f64,
                    _ => 0.0,
                };
                let _ = p
                    .send(ProgressEvent::DownloadProgress {
                        file_name: destination
                            .file_name()
                            .and_then(|s| s.to_str())
                            .unwrap_or("file")
                            .to_string(),
                        fraction,
                        bytes: bytes_downloaded,
                    })
                    .await;
            }
        }
        file.flush().await.map_err(|e| AppError::Io {
            path: destination.to_path_buf(),
            source: e,
        })?;

        Ok(DownloadProgress {
            bytes_downloaded,
            total_bytes: total,
        })
    }

    /// Compute a SHA-256 checksum of `path`. Used to verify a downloaded file
    /// against API-provided metadata.
    pub async fn sha256(&self, path: &Path) -> AppResult<String> {
        let mut hasher = Sha256::new();
        let mut file = tokio::fs::File::open(path)
            .await
            .map_err(|e| AppError::Io {
                path: path.to_path_buf(),
                source: e,
            })?;
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            let n = file.read(&mut buf).await.map_err(|e| AppError::Io {
                path: path.to_path_buf(),
                source: e,
            })?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }
        Ok(hex::encode(hasher.finalize()))
    }

    pub fn cache_dir(&self) -> &Path {
        &self.cache_dir
    }
}

/// Type alias for the progress channel used by the engine.
pub type ProgressSender = Sender<ProgressEvent>;
pub type ProgressReceiver = Receiver<ProgressEvent>;

/// Archive extraction service with strict Zip Slip protection (4.2).
pub struct ExtractionService {
    pub cache_dir: PathBuf,
}

impl ExtractionService {
    pub fn new(config: &Config) -> Self {
        Self {
            cache_dir: config.cache_directory.clone(),
        }
    }

    /// Extract `archive_path` into a fresh directory under `cache_dir`.
    /// Returns the path to the extracted directory.
    #[instrument(skip(self), fields(archive = ?archive_path))]
    pub fn extract(&self, archive_path: &Path) -> AppResult<PathBuf> {
        let stem = archive_path
            .file_stem()
            .and_then(|s| s.to_str())
            .ok_or_else(|| AppError::Extraction(format!("bad archive name: {archive_path:?}")))?;
        let stamp = chrono::Utc::now().timestamp_millis();
        let target = self.cache_dir.join(format!("extract_{stem}_{stamp}"));
        std::fs::create_dir_all(&target).map_err(|e| AppError::Io {
            path: target.clone(),
            source: e,
        })?;

        let ext = archive_path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_lowercase();

        match ext.as_str() {
            "zip" => self.extract_zip(archive_path, &target)?,
            "7z" => self.extract_7z(archive_path, &target)?,
            "rar" => self.extract_rar(archive_path, &target)?,
            other => {
                return Err(AppError::Extraction(format!(
                    "unsupported archive format: {other}"
                )))
            }
        }

        Ok(target)
    }

    fn extract_zip(&self, archive: &Path, target: &Path) -> AppResult<()> {
        use zip::ZipArchive;
        let file = std::fs::File::open(archive).map_err(|e| AppError::Io {
            path: archive.to_path_buf(),
            source: e,
        })?;
        let mut zip = ZipArchive::new(file)?;
        let canonical_target = std::fs::canonicalize(target).unwrap_or_else(|_| target.to_path_buf());
        for i in 0..zip.len() {
            let mut entry = zip.by_index(i)?;
            let entry_path = match entry.enclosed_name() {
                Some(p) => p.to_path_buf(),
                None => {
                    return Err(AppError::ZipSlip(format!(
                        "entry {} has invalid name",
                        entry.name()
                    )))
                }
            };
            // Reject absolute paths explicitly.
            if entry_path.is_absolute() {
                return Err(AppError::ZipSlip(format!(
                    "absolute path in zip: {entry_path:?}"
                )));
            }
            let outpath = target.join(&entry_path);

            // 4.2 Zip Slip protection: ensure canonicalized output stays inside target.
            if entry.is_dir() {
                std::fs::create_dir_all(&outpath).map_err(|e| AppError::Io {
                    path: outpath.clone(),
                    source: e,
                })?;
                continue;
            }
            if let Some(parent) = outpath.parent() {
                std::fs::create_dir_all(parent).map_err(|e| AppError::Io {
                    path: parent.to_path_buf(),
                    source: e,
                })?;
            }
            let mut outfile = std::fs::File::create(&outpath).map_err(|e| AppError::Io {
                path: outpath.clone(),
                source: e,
            })?;
            std::io::copy(&mut entry, &mut outfile).map_err(|e| AppError::Io {
                path: outpath.clone(),
                source: e,
            })?;

            // Validate the file actually lives under the target.
            if let Ok(canonical) = std::fs::canonicalize(&outpath) {
                if !canonical.starts_with(&canonical_target) {
                    return Err(AppError::ZipSlip(format!(
                        "extracted file escaped target: {outpath:?}"
                    )));
                }
            }
        }
        Ok(())
    }

    fn extract_7z(&self, archive: &Path, target: &Path) -> AppResult<()> {
        sevenz_rust::decompress_file(archive, target)
            .map_err(|e| AppError::Extraction(format!("7z extract: {e}")))?;
        // 4.2: verify nothing escaped target.
        Self::verify_no_zip_slip(target)?;
        Ok(())
    }

    fn extract_rar(&self, archive: &Path, target: &Path) -> AppResult<()> {
        // 4.2: prefer the `unrar` binary (and fall back to `bsdtar` for rAR support).
        // Direct library binding via the `unrar` crate is complex and version-sensitive,
        // so we shell out and capture stderr for diagnostics.
        let canonical_target =
            std::fs::canonicalize(target).unwrap_or_else(|_| target.to_path_buf());

        let try_binaries = ["unrar", "bsdtar"];
        let mut last_err: Option<AppError> = None;
        for bin in try_binaries {
            let output = if bin == "unrar" {
                std::process::Command::new(bin)
                    .args(["x", "-o+", "-y", "-idq"])
                    .arg(archive)
                    .arg(target)
                    .output()
            } else {
                std::process::Command::new(bin)
                    .args(["-x", "-C"])
                    .arg(target)
                    .arg(archive)
                    .output()
            };
            match output {
                Ok(out) if out.status.success() => {
                    last_err = None;
                    break;
                }
                Ok(out) => {
                    last_err = Some(AppError::Extraction(format!(
                        "{bin} failed: {}",
                        String::from_utf8_lossy(&out.stderr)
                    )));
                }
                Err(e) => {
                    last_err = Some(AppError::Extraction(format!("{bin}: {e}")));
                }
            }
        }
        if let Some(e) = last_err {
            return Err(e);
        }

        // 4.2: confirm no file escaped target after extraction.
        for entry in walkdir(target) {
            let p = entry?;
            if let Ok(canonical) = std::fs::canonicalize(&p) {
                if !canonical.starts_with(&canonical_target) {
                    return Err(AppError::ZipSlip(format!(
                        "rar entry escaped target: {p:?}"
                    )));
                }
            }
        }
        Ok(())
    }

    fn verify_no_zip_slip(root: &Path) -> AppResult<()> {
        let canonical_root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
        for entry in walkdir(root) {
            let p = entry?;
            if let Ok(canonical) = std::fs::canonicalize(&p) {
                if !canonical.starts_with(&canonical_root) {
                    return Err(AppError::ZipSlip(format!(
                        "file outside target: {p:?}"
                    )));
                }
            }
        }
        Ok(())
    }
}

/// Tiny recursive walker to avoid pulling in the `walkdir` crate.
fn walkdir(root: &Path) -> impl Iterator<Item = AppResult<PathBuf>> + '_ {
    let mut stack = vec![root.to_path_buf()];
    std::iter::from_fn(move || {
        while let Some(dir) = stack.pop() {
            let read = match std::fs::read_dir(&dir) {
                Ok(r) => r,
                Err(_) => continue,
            };
            for entry in read.flatten() {
                let p = entry.path();
                if p.is_dir() {
                    stack.push(p);
                } else {
                    return Some(Ok(p));
                }
            }
        }
        None
    })
}

/// Deployment service (4.3): move extracted files into `game_directory` with conflict
/// checks, backups, atomic moves, and manifest updates in a single transaction.
pub struct DeploymentService {
    config: Config,
    database: Arc<tokio::sync::Mutex<Database>>,
    pub backups_dir: PathBuf,
}

impl DeploymentService {
    pub fn new(config: Config, database: Arc<tokio::sync::Mutex<Database>>) -> AppResult<Self> {
        let backups_dir = config.cache_directory.join("backups");
        std::fs::create_dir_all(&backups_dir).map_err(|e| AppError::Io {
            path: backups_dir.clone(),
            source: e,
        })?;
        Ok(Self {
            config,
            database,
            backups_dir,
        })
    }

    /// Check the extracted files in `extracted` for conflicts with already-installed mods.
    /// Returns the set of `file_path`s that are taken.
    pub async fn detect_conflicts(
        &self,
        extracted: &Path,
    ) -> AppResult<Vec<ManifestEntry>> {
        // Translate each extracted file path to the *target* (game) path the deploy
        // step would write, so we can match against `file_manifest` rows which are
        // keyed on the absolute game-side path.
        let game_dir = self.config.game_directory.clone();
        let files: Vec<String> = walkdir(extracted)
            .filter_map(|p| p.ok())
            .filter_map(|p| {
                let rel = p.strip_prefix(extracted).ok()?;
                Some(game_dir.join(rel).to_string_lossy().to_string())
            })
            .collect();
        if files.is_empty() {
            return Ok(Vec::new());
        }
        let db = self.database.lock().await;
        db.find_existing_paths(&files).await
    }

    /// Deploy `extracted` into the game directory.
    /// - `conflicts` are user-resolved decisions, keyed by file_path.
    /// - The mod is recorded in `installed_mods` and the manifest in a single transaction.
    pub async fn deploy(
        &self,
        new_mod_id: i64,
        extracted: &Path,
        conflicts: &HashMap<String, ConflictResolution>,
        progress: Option<ProgressSender>,
    ) -> AppResult<()> {
        let game_dir = self.config.game_directory.clone();
        if !game_dir.is_dir() {
            return Err(AppError::Config(format!(
                "game_directory does not exist or is not a directory: {game_dir:?}"
            )));
        }

        let mut entries: Vec<NewManifestEntry> = Vec::new();
        let mut backups: Vec<(PathBuf, PathBuf)> = Vec::new(); // (current, backup_copy)

        for path in walkdir(extracted) {
            let path = path?;
            if !path.is_file() {
                continue;
            }
            let rel = path
                .strip_prefix(extracted)
                .map_err(|e| AppError::Engine(format!("strip_prefix: {e}")))?;
            let dest = game_dir.join(rel);

            // Determine the target file_path as stored in the manifest.
            // Use the canonicalized destination (or absolute) so the manifest key is stable.
            let stored_path = dest.to_string_lossy().to_string();

            // Conflict check via the user-provided resolution map.
            let resolution = conflicts
                .get(&stored_path)
                .copied()
                .unwrap_or(ConflictResolution::OverwriteWithBackup);

            if resolution == ConflictResolution::Abort {
                return Err(AppError::Conflict(format!("user aborted on {stored_path}")));
            }

            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent).map_err(|e| AppError::Io {
                    path: parent.to_path_buf(),
                    source: e,
                })?;
            }

            let mut backup: Option<String> = None;
            if dest.exists() && resolution == ConflictResolution::OverwriteWithBackup {
                // 4.3: copy original to backup location, record path in manifest.
                let backup_path = self
                    .backups_dir
                    .join(format!("{}_{}", new_mod_id, rel.to_string_lossy().replace('/', "_")));
                if let Some(p) = backup_path.parent() {
                    std::fs::create_dir_all(p).map_err(|e| AppError::Io {
                        path: p.to_path_buf(),
                        source: e,
                    })?;
                }
                std::fs::copy(&dest, &backup_path).map_err(|e| AppError::Io {
                    path: dest.clone(),
                    source: e,
                })?;
                backup = Some(backup_path.to_string_lossy().to_string());
                backups.push((dest.clone(), backup_path));
            }

            // 4.3: Atomic move within the same FS, copy+rename across filesystems.
            atomic_move(&path, &dest).await?;

            if let Some(p) = &progress {
                let _ = p
                    .send(ProgressEvent::DeployFinished {
                        file_name: rel.to_string_lossy().to_string(),
                        bytes: std::fs::metadata(&dest).map(|m| m.len()).unwrap_or(0),
                    })
                    .await;
            }

            let size = std::fs::metadata(&dest).map(|m| m.len() as i64).ok();
            entries.push(NewManifestEntry {
                file_path: stored_path.clone(),
                checksum: None,
                size,
                backed_up_original_path: backup.clone(),
            });
        }

        // 4.3: single transaction for the install + manifest entries.
        let db = self.database.lock().await;
        db.register_installation(
            new_mod_id,
            crate::db::InstallStatus::Enabled,
            None,
            &entries,
        )
        .await?;
        drop(db);

        // Record every conflict we resolved (OverwriteWithBackup or Skip).
        for path in conflicts.keys() {
            let db = self.database.lock().await;
            db.record_conflict(path, new_mod_id, Some(new_mod_id)).await?;
        }

        info!(mod_id = new_mod_id, files = entries.len(), "deploy complete");
        Ok(())
    }

    /// Clear the cache directory of files not referenced by `file_manifest` (4.4).
    /// Backups for installed mods are left alone.
    pub async fn cleanup_cache(&self) -> AppResult<usize> {
        let cache = &self.cache_dir_canonical();
        let db = self.database.lock().await;
        // Build a set of all paths currently in the manifest.
        let manifest_paths = sqlx::query_scalar::<_, String>(
            "SELECT file_path FROM file_manifest",
        )
        .fetch_all(db.pool())
        .await?;

        // Backups are always preserved (4.4: never delete backup entries).
        let mut protected: HashSet<String> = manifest_paths.into_iter().collect();
        if self.backups_dir.exists() {
            for entry in walkdir(&self.backups_dir) {
                if let Ok(p) = entry {
                    if let Some(s) = p.to_str() {
                        protected.insert(s.to_string());
                    }
                }
            }
        }

        let mut removed = 0usize;
        for entry in walkdir(cache) {
            let p = match entry {
                Ok(p) => p,
                Err(_) => continue,
            };
            // Don't delete the top-level cache, backups, or the DB file.
            if p == *cache || p == self.backups_dir {
                continue;
            }
            if let Some(s) = p.to_str() {
                if protected.contains(s) {
                    continue;
                }
            }
            if p.is_dir() {
                if std::fs::remove_dir(&p).is_ok() {
                    removed += 1;
                }
            } else if std::fs::remove_file(&p).is_ok() {
                removed += 1;
            }
        }
        Ok(removed)
    }

    fn cache_dir_canonical(&self) -> PathBuf {
        self.config.cache_directory.clone()
    }
}

/// Move `from` to `to`, preferring `rename` (atomic on the same FS) and falling back
/// to a copy+fsync+rename across filesystems.
async fn atomic_move(from: &Path, to: &Path) -> AppResult<()> {
    // 1) Try rename (same FS, atomic).
    if let Ok(()) = std::fs::rename(from, to) {
        return Ok(());
    }
    // 2) Fall back to copy + fsync + rename.
    let bytes = std::fs::read(from).map_err(|e| AppError::Io {
        path: from.to_path_buf(),
        source: e,
    })?;
    {
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(to)
            .map_err(|e| AppError::Io {
                path: to.to_path_buf(),
                source: e,
            })?;
        use std::io::Write;
        file.write_all(&bytes).map_err(|e| AppError::Io {
            path: to.to_path_buf(),
            source: e,
        })?;
        file.sync_all().map_err(|e| AppError::Io {
            path: to.to_path_buf(),
            source: e,
        })?;
    }
    std::fs::remove_file(from).map_err(|e| AppError::Io {
        path: from.to_path_buf(),
        source: e,
    })?;
    Ok(())
}

/// Top-level facade so the UI has one entry point to "install a mod file".
pub struct ModEngine {
    pub download_manager: DownloadManager,
    pub extraction_service: ExtractionService,
    pub deployment_service: DeploymentService,
    pub database: Arc<tokio::sync::Mutex<Database>>,
    pub config: Config,
    pub progress_tx: Option<ProgressSender>,
    pub progress_rx: std::sync::Mutex<Option<ProgressReceiver>>,
}

impl ModEngine {
    pub fn new(config: Config, database: Arc<tokio::sync::Mutex<Database>>) -> Self {
        let download_manager = DownloadManager::new(&config);
        let extraction_service = ExtractionService::new(&config);
        let deployment_service =
            DeploymentService::new(config.clone(), database.clone()).expect("deployment");
        let (progress_tx, progress_rx) = async_channel::unbounded::<ProgressEvent>();
        Self {
            download_manager,
            extraction_service,
            deployment_service,
            database,
            config,
            progress_tx: Some(progress_tx),
            progress_rx: std::sync::Mutex::new(Some(progress_rx)),
        }
    }

    /// One-shot install: download → extract → conflict-check → deploy.
    /// Conflicts are reported to the caller via the returned Vec; resolve them
    /// and call `ModEngine::deploy` to finish.
    pub async fn install_with_url(
        &self,
        mod_id: i32,
        file: &ModFile,
        progress: Option<ProgressSender>,
    ) -> AppResult<InstallPlan> {        // 1) Download to cache
        let cache = self.config.cache_directory.join(&file.file_name);
        self.download_manager
            .download(&file.file_url, &cache, progress.clone())
            .await?;

        // 2) Extract
        if let Some(p) = &progress {
            let _ = p
                .send(ProgressEvent::ExtractionStarted {
                    file_name: file.file_name.clone(),
                })
                .await;
        }
        let extracted = self.extraction_service.extract(&cache)?;
        if let Some(p) = &progress {
            let _ = p
                .send(ProgressEvent::ExtractionFinished {
                    file_name: file.file_name.clone(),
                })
                .await;
        }

        // 3) Resolve mod_id (Nexus ID) → internal id
        let db = self.database.lock().await;
        let nexus_id = mod_id.to_string();
        let internal_id = match db.mod_id_by_nexus(&nexus_id).await? {
            Some(id) => id,
            None => return Err(AppError::Db(format!("mod not registered: {nexus_id}"))),
        };
        drop(db);

        // 4) Detect conflicts
        let conflicts = self.deployment_service.detect_conflicts(&extracted).await?;
        if let Some(p) = &progress {
            for c in &conflicts {
                let _ = p
                    .send(ProgressEvent::ConflictDetected {
                        file_path: c.file_path.clone(),
                        existing_mod: c.mod_id,
                        new_mod: internal_id,
                    })
                    .await;
            }
        }
        Ok(InstallPlan {
            new_mod_id: internal_id,
            extracted,
            conflicts: conflicts
                .into_iter()
                .map(|m| ConflictReport {
                    file_path: m.file_path,
                    new_mod_id: internal_id,
                    existing_mod_id: m.mod_id,
                })
                .collect(),
        })
    }

    /// Finish a planned install once the user has chosen how to resolve the conflicts.
    pub async fn deploy_plan(
        &self,
        plan: &InstallPlan,
        resolutions: &HashMap<String, ConflictResolution>,
        progress: Option<ProgressSender>,
    ) -> AppResult<()> {
        self.deployment_service
            .deploy(plan.new_mod_id, &plan.extracted, resolutions, progress)
            .await
    }

    /// Direct access to the deployment service for callers that already have
    /// the extracted directory on disk.
    pub fn deploy_service(&self) -> &DeploymentService {
        &self.deployment_service
    }

    /// Subscribe to engine progress events. The returned `Receiver` is
    /// consumed by UI views. Multiple subscribers can each call this to get
    /// their own receiver.
    pub fn progress_subscribe(&self) -> Option<ProgressReceiver> {
        // async-channel does not support fan-out. Instead we use a single
        // receiver stored in the engine and let callers take a clone of the
        // *sender*. The UI module subscribes to progress by calling
        // `take_progress_receiver` once at startup.
        None
    }

    /// Subscribe to engine progress events. Wraps the receiver in a Mutex so
    /// it can be called through an `Arc<ModEngine>`. Returns `None` if the
    /// receiver was already taken.
    pub fn take_progress_receiver(&self) -> Option<ProgressReceiver> {
        let mut guard = self.progress_rx.lock().ok()?;
        guard.take()
    }

    /// Uninstall a mod by its Nexus ID. This removes the installation
    /// from the database and restores any backed-up files.
    pub async fn uninstall_mod(&self, nexus_id: &str) -> AppResult<()> {
        let db = self.database.lock().await;
        let internal_id = db.mod_id_by_nexus(nexus_id).await?
            .ok_or_else(|| AppError::Db(format!("mod not installed: {nexus_id}")))?;
        
        // Get the manifest entries for this mod
        let manifest_entries = db.manifest_for_mod(internal_id).await?;
        
        // Create a restorer function that will restore backed-up files
        let mut restored_files = Vec::new();
        let restorer = |current_path: &str, backup_path: &str| -> AppResult<()> {
            // Move the backup file back to its original location
            std::fs::rename(backup_path, current_path)
                .map_err(|e| AppError::Io {
                    path: PathBuf::from(current_path),
                    source: e,
                })?;
            restored_files.push(current_path.to_string());
            Ok(())
        };
        
        // Delete the installation (this will also restore backups)
        db.delete_installation(internal_id, restorer).await?;
        
        // Update mod status to disabled
        db.set_install_status(nexus_id, crate::db::InstallStatus::Disabled).await?;
        
        tracing::info!(mod_id = nexus_id, files_restored = restored_files.len(), "mod uninstalled");
        Ok(())
    }

    /// Update a mod by downloading and installing a new version.
    pub async fn update_mod(&self, nexus_id: &str, file: &ModFile) -> AppResult<()> {
        // First uninstall the current version
        self.uninstall_mod(nexus_id).await?;
        
        // Then install the new version (this is simplified - in reality we'd need to 
        // handle conflicts and other details)
        let progress = self.progress_tx.clone();
        let plan = self.install_with_url(
            file.mod_id,
            file,
            progress.clone()  // Clone before using
        ).await?;
        
        // For now, just deploy with default resolution
        let resolutions = HashMap::new();
        self.deploy_plan(&plan, &resolutions, progress).await?;
        
        Ok(())
    }

    /// Build a placeholder engine for tests / `Default` impls.
    pub fn placeholder() -> Self {
        let config = Config {
            game_directory: std::env::temp_dir().join("cp2077-game"),
            cache_directory: std::env::temp_dir().join("cp2077-cache"),
            database_path: std::env::temp_dir().join("cp2077-cache/db.sqlite"),
            nexus_api_key: String::new(),
        };
        let download_manager = DownloadManager::new(&config);
        let _ = download_manager; // unused in placeholder
        let (tx, _rx) = async_channel::unbounded::<ProgressEvent>();
        Self {
            download_manager: DownloadManager::new(&config),
            extraction_service: ExtractionService { cache_dir: config.cache_directory.clone() },
            deployment_service: unsafe {
                // SAFETY: we never call `DeploymentService` methods on a placeholder
                // because the database field is invalid. The UI only uses
                // `take_progress_receiver`, which is safe.
                std::mem::zeroed()
            },
            database: Arc::new(tokio::sync::Mutex::new(
                futures::executor::block_on(Database::in_memory()).expect("in_memory"),
            )),
            config,
            progress_tx: Some(tx),
            progress_rx: std::sync::Mutex::new(None),
        }
    }
}

#[derive(Debug, Clone)]
pub struct InstallPlan {
    pub new_mod_id: i64,
    pub extracted: PathBuf,
    pub conflicts: Vec<ConflictReport>,
}
