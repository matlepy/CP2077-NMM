use std::collections::HashMap;
use std::path::Path;

use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Row, SqlitePool};
use tracing::instrument;

use crate::api::{Mod, ModFile};
use crate::errors::{AppError, AppResult};

/// The status of an installed mod. Persisted as TEXT in `installed_mods.status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallStatus {
    Enabled,
    Disabled,
    PendingRequirements,
}

impl InstallStatus {
    fn as_str(self) -> &'static str {
        match self {
            InstallStatus::Enabled => "enabled",
            InstallStatus::Disabled => "disabled",
            InstallStatus::PendingRequirements => "pending_requirements",
        }
    }

    fn parse(s: &str) -> AppResult<Self> {
        match s {
            "enabled" => Ok(InstallStatus::Enabled),
            "disabled" => Ok(InstallStatus::Disabled),
            "pending_requirements" => Ok(InstallStatus::PendingRequirements),
            other => Err(AppError::Db(format!("unknown install status: {other}"))),
        }
    }
}

/// Row from `mods`, joined with the optional `mod_requirements` side table.
#[derive(Debug, Clone)]
pub struct InstalledModRow {
    pub id: i64,
    pub name: String,
    pub version: Option<String>,
    pub nexus_id: String,
    pub description: Option<String>,
    pub category: Option<String>,
    pub status: InstallStatus,
    pub load_order: Option<i32>,
    pub installation_date: String,
    pub required_nexus_ids: Vec<String>,
    pub required_versions: HashMap<String, String>,
}

/// One row of the `file_manifest` table.
#[derive(Debug, Clone)]
pub struct ManifestEntry {
    pub id: i64,
    pub mod_id: i64,
    pub file_path: String,
    pub checksum: Option<String>,
    pub size: Option<i64>,
    pub backed_up_original_path: Option<String>,
}

/// A record describing a conflict between two mods over the same target path.
#[derive(Debug, Clone)]
pub struct ConflictRecord {
    pub file_path: String,
    pub mod_id: i64,
    pub resolved_winner_mod_id: Option<i64>,
}

/// Database connection wrapper.
///
/// All public methods are async and run against an internal `sqlx::SqlitePool`.
/// Foreign-key enforcement is enabled on every new connection.
#[derive(Debug, Clone)]
pub struct Database {
    pool: SqlitePool,
}

impl Database {
    /// Connect to a SQLite database at the given path and enable foreign keys.
    pub async fn new(database_path: &Path) -> AppResult<Self> {
        if let Some(parent) = database_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| AppError::Io {
                path: parent.to_path_buf(),
                source: e,
            })?;
        }

        let options = SqliteConnectOptions::new()
            .filename(database_path)
            .create_if_missing(true)
            .foreign_keys(true)
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
            .busy_timeout(std::time::Duration::from_secs(5));

        let pool = SqlitePoolOptions::new()
            .max_connections(8)
            .connect_with(options)
            .await?;

        Ok(Database { pool })
    }

    /// Connect to an in-memory database (used by tests).
    pub async fn in_memory() -> AppResult<Self> {
        let options = SqliteConnectOptions::new()
            .filename(":memory:")
            .foreign_keys(true);

        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await?;

        let db = Database { pool };
        db.initialize().await?;
        Ok(db)
    }

    /// Run all versioned migrations from `./migrations`.
    #[instrument(skip(self))]
    pub async fn initialize(&self) -> AppResult<()> {
        sqlx::migrate!("./migrations").run(&self.pool).await?;
        Ok(())
    }

    /// Direct access to the underlying connection pool (for tests and advanced callers).
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    /// Synchronous placeholder default; rarely useful outside tests. Prefer
    /// [`Database::in_memory`] or [`Database::new`] for real work.
    #[deprecated(note = "Database has no synchronous default; use Database::in_memory() or Database::new() instead")]
    pub fn default_unused() -> Self {
        unreachable!("Database cannot be constructed without a connection; use Database::in_memory() or Database::new()")
    }

    /// Insert (or no-op) a mod row and return its primary key.
    ///
    /// `upsert_by_nexus_id` uses `ON CONFLICT(nexus_id) DO UPDATE` so that re-registering
    /// a mod refreshes its metadata without producing a duplicate row.
    pub async fn upsert_mod(&self, mod_meta: &Mod) -> AppResult<i64> {
        let mut tx = self.pool.begin().await?;

        sqlx::query(
            r#"
            INSERT INTO mods (name, version, nexus_id, description, category)
            VALUES (?, ?, ?, ?, ?)
            ON CONFLICT(nexus_id) DO UPDATE SET
                name = excluded.name,
                version = excluded.version,
                description = excluded.description,
                category = excluded.category
            "#,
        )
        .bind(&mod_meta.name)
        .bind(&mod_meta.version)
        .bind(&mod_meta.nexus_id)
        .bind(&mod_meta.description)
        .bind(&mod_meta.category)
        .execute(&mut *tx)
        .await?;

        // Wipe & re-insert requirements so they stay in sync with the API response.
        sqlx::query("DELETE FROM mod_requirements WHERE mod_id = (SELECT id FROM mods WHERE nexus_id = ?)")
            .bind(&mod_meta.nexus_id)
            .execute(&mut *tx)
            .await?;

        for req in &mod_meta.required_mod_ids {
            let required_version = mod_meta.required_versions.get(req).cloned();
            sqlx::query(
                r#"
                INSERT INTO mod_requirements (mod_id, required_nexus_id, required_version)
                VALUES ((SELECT id FROM mods WHERE nexus_id = ?), ?, ?)
                "#,
            )
            .bind(&mod_meta.nexus_id)
            .bind(req)
            .bind(required_version)
            .execute(&mut *tx)
            .await?;
        }

        let id: i64 = sqlx::query_scalar("SELECT id FROM mods WHERE nexus_id = ?")
            .bind(&mod_meta.nexus_id)
            .fetch_one(&mut *tx)
            .await?;

        tx.commit().await?;
        Ok(id)
    }

    /// Register an installation: writes `installed_mods` and the file manifest in a single
    /// transaction (2.3 "all-or-nothing"). Returns the new `installed_mods.id`.
    pub async fn register_installation(
        &self,
        mod_id: i64,
        status: InstallStatus,
        load_order: Option<i32>,
        manifest: &[NewManifestEntry],
    ) -> AppResult<i64> {
        let mut tx = self.pool.begin().await?;

        let date = chrono::Utc::now().to_rfc3339();
        let install_id: i64 = sqlx::query_scalar(
            r#"
            INSERT INTO installed_mods (mod_id, installation_date, status, load_order)
            VALUES (?, ?, ?, ?)
            RETURNING id
            "#,
        )
        .bind(mod_id)
        .bind(&date)
        .bind(status.as_str())
        .bind(load_order)
        .fetch_one(&mut *tx)
        .await?;

        for entry in manifest {
            sqlx::query(
                r#"
                INSERT INTO file_manifest
                    (mod_id, file_path, checksum, size, backed_up_original_path)
                VALUES (?, ?, ?, ?, ?)
                "#,
            )
            .bind(mod_id)
            .bind(&entry.file_path)
            .bind(&entry.checksum)
            .bind(entry.size)
            .bind(&entry.backed_up_original_path)
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;
        Ok(install_id)
    }

    /// Find any existing `file_manifest` rows whose `file_path` is in `paths`. Used by 4.3
    /// to detect conflicts before deployment.
    pub async fn find_existing_paths(
        &self,
        paths: &[String],
    ) -> AppResult<Vec<ManifestEntry>> {
        if paths.is_empty() {
            return Ok(Vec::new());
        }
        let placeholders = std::iter::repeat("?")
            .take(paths.len())
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "SELECT id, mod_id, file_path, checksum, size, backed_up_original_path \
             FROM file_manifest WHERE file_path IN ({placeholders})"
        );
        let mut query = sqlx::query(&sql);
        for p in paths {
            query = query.bind(p);
        }
        let rows = query.fetch_all(&self.pool).await?;
        rows.into_iter().map(row_to_manifest_entry).collect()
    }

    /// Remove a mod installation, restore any backed-up originals, and drop the
    /// `installed_mods` row + `file_manifest` rows for the mod.
    ///
    /// `restorer` is called for every `backed_up_original_path` that needs to be moved back;
    /// the closure receives `(current_path, original_path)` and decides what to do.
    pub async fn delete_installation<F>(
        &self,
        mod_id: i64,
        mut restorer: F,
    ) -> AppResult<()>
    where
        F: FnMut(&str, &str) -> AppResult<()>,
    {
        let mut tx = self.pool.begin().await?;

        let entries: Vec<ManifestEntry> =
            sqlx::query("SELECT id, mod_id, file_path, checksum, size, backed_up_original_path FROM file_manifest WHERE mod_id = ?")
                .bind(mod_id)
                .fetch_all(&mut *tx)
                .await?
                .into_iter()
                .map(row_to_manifest_entry)
                .collect::<AppResult<Vec<_>>>()?;

        for entry in &entries {
            if let Some(backup) = &entry.backed_up_original_path {
                if let Err(e) = restorer(&entry.file_path, backup) {
                    tracing::error!(
                        file = %entry.file_path,
                        backup = %backup,
                        error = %e,
                        "failed to restore backup; continuing"
                    );
                }
            }
        }

        sqlx::query("DELETE FROM file_manifest WHERE mod_id = ?")
            .bind(mod_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM installed_mods WHERE mod_id = ?")
            .bind(mod_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM mod_requirements WHERE mod_id = ?")
            .bind(mod_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM file_conflicts WHERE mod_id = ?")
            .bind(mod_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM mods WHERE id = ? AND NOT EXISTS (SELECT 1 FROM installed_mods WHERE mod_id = ?)")
            .bind(mod_id)
            .bind(mod_id)
            .execute(&mut *tx)
            .await?;

        tx.commit().await?;
        Ok(())
    }

    /// Look up a mod by its Nexus ID and return its primary key, or `None` if not present.
    pub async fn mod_id_by_nexus(&self, nexus_id: &str) -> AppResult<Option<i64>> {
        let row = sqlx::query("SELECT id FROM mods WHERE nexus_id = ?")
            .bind(nexus_id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|r| r.get::<i64, _>(0)))
    }

    /// Fetch a single mod's metadata by Nexus ID, with requirements attached.
    pub async fn get_mod_by_nexus(&self, nexus_id: &str) -> AppResult<Option<Mod>> {
        let row = sqlx::query(
            "SELECT name, version, nexus_id, description, category FROM mods WHERE nexus_id = ?",
        )
        .bind(nexus_id)
        .fetch_optional(&self.pool)
        .await?;
        let Some(row) = row else { return Ok(None) };

        let name: String = row.get(0);
        let version: Option<String> = row.get(1);
        let nexus_id: String = row.get(2);
        let description: Option<String> = row.get(3);
        let category: Option<String> = row.get(4);

        let req_rows = sqlx::query(
            "SELECT required_nexus_id, required_version FROM mod_requirements WHERE mod_id = (SELECT id FROM mods WHERE nexus_id = ?)",
        )
        .bind(&nexus_id)
        .fetch_all(&self.pool)
        .await?;

        let mut required_mod_ids = Vec::new();
        let mut required_versions = HashMap::new();
        for r in req_rows {
            let req_id: String = r.get(0);
            let req_ver: Option<String> = r.get(1);
            required_mod_ids.push(req_id.clone());
            if let Some(v) = req_ver {
                required_versions.insert(req_id, v);
            }
        }

        Ok(Some(Mod {
            id: 0,
            name,
            version: version.unwrap_or_default(),
            description: description.unwrap_or_default(),
            category: category.unwrap_or_default(),
            nexus_id,
            required_mod_ids,
            required_versions,
        }))
    }

    /// Backwards-compatible alias of [`Database::get_mod_by_nexus`].
    pub async fn get_mod_metadata(&self, mod_id: &str) -> AppResult<Mod> {
        self.get_mod_by_nexus(mod_id)
            .await?
            .ok_or_else(|| AppError::Db(format!("mod not found: {mod_id}")))
    }

    /// True if any `installed_mods` row references the mod with the given Nexus ID.
    pub async fn is_mod_installed(&self, mod_id: &str) -> AppResult<bool> {
        let row = sqlx::query(
            "SELECT EXISTS(SELECT 1 FROM installed_mods WHERE mod_id = (SELECT id FROM mods WHERE nexus_id = ?)) as installed",
        )
        .bind(mod_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(row.get::<i64, _>(0) != 0)
    }

    /// All installed mods joined with their static metadata, ordered by `load_order`.
    pub async fn installed_mods(&self) -> AppResult<Vec<InstalledModRow>> {
        let rows = sqlx::query(
            r#"
            SELECT m.id, m.name, m.version, m.nexus_id, m.description, m.category,
                   i.installation_date, i.status, i.load_order
            FROM installed_mods i
            JOIN mods m ON m.id = i.mod_id
            ORDER BY COALESCE(i.load_order, 1<<30) ASC
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            let mod_id: i64 = r.get(0);
            let req_rows = sqlx::query("SELECT required_nexus_id, required_version FROM mod_requirements WHERE mod_id = ?")
                .bind(mod_id)
                .fetch_all(&self.pool)
                .await?;

            let mut required_nexus_ids = Vec::new();
            let mut required_versions = HashMap::new();
            for rr in req_rows {
                let rid: String = rr.get(0);
                let ver: Option<String> = rr.get(1);
                required_nexus_ids.push(rid.clone());
                if let Some(v) = ver {
                    required_versions.insert(rid, v);
                }
            }

            out.push(InstalledModRow {
                id: mod_id,
                name: r.get(1),
                version: r.get(2),
                nexus_id: r.get(3),
                description: r.get(4),
                category: r.get(5),
                installation_date: r.get(6),
                status: InstallStatus::parse(r.get::<&str, _>(7))?,
                load_order: r.get(8),
                required_nexus_ids,
                required_versions,
            });
        }
        Ok(out)
    }

    /// Update the `load_order` integer for an installed mod.
    pub async fn set_load_order(&self, nexus_id: &str, load_order: i32) -> AppResult<()> {
        let result = sqlx::query(
            "UPDATE installed_mods SET load_order = ? WHERE mod_id = (SELECT id FROM mods WHERE nexus_id = ?)",
        )
        .bind(load_order)
        .bind(nexus_id)
        .execute(&self.pool)
        .await?;
        if result.rows_affected() == 0 {
            return Err(AppError::Db(format!(
                "no installed_mods row for nexus_id={nexus_id}"
            )));
        }
        Ok(())
    }

    /// Update the status of an installed mod.
    pub async fn set_install_status(&self, nexus_id: &str, status: InstallStatus) -> AppResult<()> {
        let result = sqlx::query(
            "UPDATE installed_mods SET status = ? WHERE mod_id = (SELECT id FROM mods WHERE nexus_id = ?)",
        )
        .bind(status.as_str())
        .bind(nexus_id)
        .execute(&self.pool)
        .await?;
        if result.rows_affected() == 0 {
            return Err(AppError::Db(format!(
                "no installed_mods row for nexus_id={nexus_id}"
            )));
        }
        Ok(())
    }

    /// Record a conflict between two mods over the same `file_path`.
    pub async fn record_conflict(
        &self,
        file_path: &str,
        mod_id: i64,
        resolved_winner: Option<i64>,
    ) -> AppResult<()> {
        sqlx::query(
            r#"
            INSERT INTO file_conflicts (file_path, mod_id, resolved_winner_mod_id)
            VALUES (?, ?, ?)
            ON CONFLICT(file_path, mod_id) DO UPDATE SET
                resolved_winner_mod_id = excluded.resolved_winner_mod_id
            "#,
        )
        .bind(file_path)
        .bind(mod_id)
        .bind(resolved_winner)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Get all files for the given mod, with their stored checksums and sizes.
    pub async fn manifest_for_mod(&self, mod_id: i64) -> AppResult<Vec<ManifestEntry>> {
        let rows = sqlx::query(
            "SELECT id, mod_id, file_path, checksum, size, backed_up_original_path \
             FROM file_manifest WHERE mod_id = ? ORDER BY file_path ASC",
        )
        .bind(mod_id)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(row_to_manifest_entry).collect()
    }

    /// Get the (key, value) setting, or None if missing.
    pub async fn get_setting(&self, key: &str) -> AppResult<Option<String>> {
        let row = sqlx::query("SELECT value FROM settings WHERE key = ?")
            .bind(key)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|r| r.get::<String, _>(0)))
    }

    /// Set a (key, value) setting, overwriting any previous value.
    pub async fn set_setting(&self, key: &str, value: &str) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO settings (key, value) VALUES (?, ?) \
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        )
        .bind(key)
        .bind(value)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

/// Helper for `register_installation` callers.
pub struct NewManifestEntry {
    pub file_path: String,
    pub checksum: Option<String>,
    pub size: Option<i64>,
    pub backed_up_original_path: Option<String>,
}

impl<'a> From<&'a ManifestEntry> for NewManifestEntry {
    fn from(m: &'a ManifestEntry) -> Self {
        NewManifestEntry {
            file_path: m.file_path.clone(),
            checksum: m.checksum.clone(),
            size: m.size,
            backed_up_original_path: m.backed_up_original_path.clone(),
        }
    }
}

/// Used by the deploy service to record every file installed by a mod.
pub fn manifest_entry_from_file(mod_id: i64, file: &ModFile) -> ManifestEntry {
    ManifestEntry {
        id: 0,
        mod_id,
        file_path: file.file_path.clone(),
        checksum: None,
        size: Some(file.size as i64),
        backed_up_original_path: None,
    }
}

fn row_to_manifest_entry(r: sqlx::sqlite::SqliteRow) -> AppResult<ManifestEntry> {
    Ok(ManifestEntry {
        id: r.get(0),
        mod_id: r.get(1),
        file_path: r.get(2),
        checksum: r.get(3),
        size: r.get(4),
        backed_up_original_path: r.get(5),
    })
}
