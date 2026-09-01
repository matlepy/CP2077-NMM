//! Phase 4: Mod Engine — extraction tests with strict Zip Slip protection.
//!
//! DoD: deploying a second mod that targets an already-installed file path
//! triggers a conflict flag instead of a silent overwrite.

use std::fs;
use std::io::Write;

use nexus_mod_manager::api::Mod;
use nexus_mod_manager::config::Config;
use nexus_mod_manager::db::Database;
use nexus_mod_manager::engine::{DeploymentService, ExtractionService, ModEngine};
use std::sync::Arc;
use tempfile::TempDir;
use tokio::sync::Mutex;

fn make_zip_with_entries(path: &std::path::Path, entries: &[(&str, &[u8])]) {
    use zip::write::FileOptions;
    let file = std::fs::File::create(path).unwrap();
    let mut zip = zip::ZipWriter::new(file);
    for (name, data) in entries {
        zip.start_file(*name, FileOptions::default()).unwrap();
        zip.write_all(data).unwrap();
    }
    zip.finish().unwrap();
}

fn make_config(cache: &std::path::Path, game: &std::path::Path) -> Config {
    Config {
        game_directory: game.to_path_buf(),
        cache_directory: cache.to_path_buf(),
        database_path: cache.join("db.sqlite"),
        nexus_api_key: String::new(),
    }
}

#[tokio::test]
async fn extract_zip_creates_files() {
    let tmp = TempDir::new().unwrap();
    let cache = tmp.path().join("cache");
    let game = tmp.path().join("game");
    fs::create_dir_all(&cache).unwrap();
    fs::create_dir_all(&game).unwrap();

    let archive = cache.join("mod.zip");
    make_zip_with_entries(
        &archive,
        &[("archive/file1.bin", b"hello"), ("archive/file2.bin", b"world")],
    );

    let svc = ExtractionService {
        cache_dir: cache.clone(),
    };
    let extracted = svc.extract(&archive).unwrap();
    assert!(extracted.join("archive/file1.bin").is_file());
    assert!(extracted.join("archive/file2.bin").is_file());
}

#[tokio::test]
async fn extract_zip_rejects_zip_slip() {
    let tmp = TempDir::new().unwrap();
    let cache = tmp.path().join("cache");
    fs::create_dir_all(&cache).unwrap();
    let archive = cache.join("evil.zip");
    make_zip_with_entries(&archive, &[("../escapee.bin", b"bad")]);

    let svc = ExtractionService {
        cache_dir: cache.clone(),
    };
    let result = svc.extract(&archive);
    assert!(result.is_err(), "expected ZipSlip error");
    match result {
        Err(nexus_mod_manager::errors::AppError::ZipSlip(_)) => {}
        other => panic!("expected ZipSlip, got {other:?}"),
    }
}

#[tokio::test]
async fn deploy_detects_conflict_on_existing_path() {
    let tmp = TempDir::new().unwrap();
    let cache = tmp.path().join("cache");
    let game = tmp.path().join("game");
    fs::create_dir_all(&cache).unwrap();
    fs::create_dir_all(&game).unwrap();

    let config = make_config(&cache, &game);
    let db = Database::new(&config.database_path).await.unwrap();
    db.initialize().await.unwrap();
    let db_arc = Arc::new(Mutex::new(db));

    // First mod: A installs archive/file1.bin
    let archive_a = cache.join("a.zip");
    make_zip_with_entries(&archive_a, &[("archive/file1.bin", b"version-A")]);
    let svc = ExtractionService {
        cache_dir: cache.clone(),
    };
    let extracted_a = svc.extract(&archive_a).unwrap();
    let deploy = DeploymentService::new(config.clone(), db_arc.clone()).unwrap();
    {
        let db = db_arc.lock().await;
        let mod_id = db
            .upsert_mod(&Mod {
                id: 0,
                name: "A".into(),
                version: "1".into(),
                description: String::new(),
                category: String::new(),
                nexus_id: "100".into(),
                required_mod_ids: vec![],
                required_versions: Default::default(),
            })
            .await
            .unwrap();
        drop(db);
        deploy
            .deploy(mod_id, &extracted_a, &Default::default(), None)
            .await
            .unwrap();
    }

    // Second mod: B targets the same file
    let archive_b = cache.join("b.zip");
    make_zip_with_entries(&archive_b, &[("archive/file1.bin", b"version-B")]);
    let extracted_b = svc.extract(&archive_b).unwrap();
    let conflicts = deploy.detect_conflicts(&extracted_b).await.unwrap();
    assert_eq!(conflicts.len(), 1, "expected exactly one conflict");
    assert!(conflicts[0].file_path.contains("file1.bin"));
}

#[tokio::test]
async fn engine_one_shot_install_flow() {
    let tmp = TempDir::new().unwrap();
    let cache = tmp.path().join("cache");
    let game = tmp.path().join("game");
    fs::create_dir_all(&cache).unwrap();
    fs::create_dir_all(&game).unwrap();

    let config = make_config(&cache, &game);
    let db = Database::new(&config.database_path).await.unwrap();
    db.initialize().await.unwrap();
    let db_arc = Arc::new(Mutex::new(db));
    let engine = ModEngine::new(config.clone(), db_arc.clone());

    // Pre-register a mod
    {
        let db = db_arc.lock().await;
        db.upsert_mod(&Mod {
            id: 0,
            name: "X".into(),
            version: "1".into(),
            description: String::new(),
            category: String::new(),
            nexus_id: "200".into(),
            required_mod_ids: vec![],
            required_versions: Default::default(),
        })
        .await
        .unwrap();
    }

    // Stage a "downloaded" archive in the cache.
    let archive = cache.join("x.zip");
    make_zip_with_entries(&archive, &[("file.bin", b"content")]);

    // Drive the install via the engine (skipping the download step).
    let svc = ExtractionService {
        cache_dir: cache.clone(),
    };
    let extracted = svc.extract(&archive).unwrap();

    {
        let db = db_arc.lock().await;
        let id = db.mod_id_by_nexus("200").await.unwrap().unwrap();
        drop(db);
        engine
            .deploy_service()
            .deploy(id, &extracted, &Default::default(), None)
            .await
            .unwrap();
    }

    // File should now be in the game directory.
    assert!(game.join("file.bin").is_file());
}

#[tokio::test]
async fn cleanup_cache_preserves_installed_files() {
    let tmp = TempDir::new().unwrap();
    let cache = tmp.path().join("cache");
    let game = tmp.path().join("game");
    fs::create_dir_all(&cache).unwrap();
    fs::create_dir_all(&game).unwrap();

    let config = make_config(&cache, &game);
    let db = Database::new(&config.database_path).await.unwrap();
    db.initialize().await.unwrap();
    let db_arc = Arc::new(Mutex::new(db));
    let engine = ModEngine::new(config.clone(), db_arc.clone());

    // Stage an archive, install it.
    let archive = cache.join("inst.zip");
    make_zip_with_entries(&archive, &[("installed.bin", b"keep me")]);
    let svc = ExtractionService {
        cache_dir: cache.clone(),
    };
    let extracted = svc.extract(&archive).unwrap();
    {
        let db = db_arc.lock().await;
        let id = db
            .upsert_mod(&Mod {
                id: 0,
                name: "Y".into(),
                version: "1".into(),
                description: String::new(),
                category: String::new(),
                nexus_id: "300".into(),
                required_mod_ids: vec![],
                required_versions: Default::default(),
            })
            .await
            .unwrap();
        drop(db);
        engine
            .deploy_service()
            .deploy(id, &extracted, &Default::default(), None)
            .await
            .unwrap();
    }

    // Add some junk to the cache.
    let junk = cache.join("junk.txt");
    fs::write(&junk, b"unused").unwrap();
    assert!(junk.is_file());

    engine
        .deployment_service
        .cleanup_cache()
        .await
        .unwrap();
    // Game file is untouched.
    assert!(game.join("installed.bin").is_file());
    // Junk should be removed.
    assert!(!junk.is_file(), "junk should be cleaned up");
}
