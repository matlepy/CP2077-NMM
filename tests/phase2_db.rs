//! Phase 2: Repository-layer tests against an in-memory SQLite database.
//!
//! DoD: a manual test proves a failed multi-file insert rolls back cleanly.

use std::collections::HashMap;

use nexus_mod_manager::api::Mod;
use nexus_mod_manager::db::{Database, InstallStatus, NewManifestEntry};

fn mod_(id: &str, name: &str) -> Mod {
    Mod {
        id: 0,
        name: name.into(),
        version: "1.0".into(),
        description: String::new(),
        category: "test".into(),
        nexus_id: id.into(),
        required_mod_ids: Vec::new(),
        required_versions: HashMap::new(),
    }
}

#[tokio::test]
async fn migrations_apply_on_initialize() {
    let db = Database::in_memory().await.expect("in_memory");
    // After initialize, the tables are present.
    let row: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM mods")
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(row, 0);
}

#[tokio::test]
async fn upsert_mod_persists_metadata_and_requirements() {
    let db = Database::in_memory().await.unwrap();
    let mut m = mod_("42", "Foo");
    m.required_mod_ids = vec!["99".into()];
    m.required_versions.insert("99".into(), "2.0".into());

    let id = db.upsert_mod(&m).await.unwrap();
    assert!(id > 0);

    let round = db.get_mod_by_nexus("42").await.unwrap().unwrap();
    assert_eq!(round.nexus_id, "42");
    assert_eq!(round.required_mod_ids, vec!["99".to_string()]);
    assert_eq!(round.required_versions.get("99").unwrap(), "2.0");
}

#[tokio::test]
async fn register_installation_writes_manifest_atomically() {
    let db = Database::in_memory().await.unwrap();
    let id = db.upsert_mod(&mod_("1", "Alpha")).await.unwrap();
    let entries = vec![
        NewManifestEntry { file_path: "a/b.bin".into(), checksum: Some("aaaa".into()), size: Some(10), backed_up_original_path: None },
        NewManifestEntry { file_path: "a/c.bin".into(), checksum: Some("bbbb".into()), size: Some(20), backed_up_original_path: None },
    ];
    let install = db
        .register_installation(id, InstallStatus::Enabled, Some(0), &entries)
        .await
        .unwrap();
    assert!(install > 0);

    let manifest = db.manifest_for_mod(id).await.unwrap();
    assert_eq!(manifest.len(), 2);
    assert!(manifest.iter().any(|m| m.file_path == "a/b.bin"));
}

#[tokio::test]
async fn register_installation_rolls_back_on_bad_input() {
    let db = Database::in_memory().await.unwrap();
    let id = db.upsert_mod(&mod_("2", "Beta")).await.unwrap();
    // UNIQUE(file_path) — second insert with the same path must fail and roll back the
    // whole transaction (so the installed_mods row should NOT exist).
    let entries = vec![
        NewManifestEntry { file_path: "x/y.bin".into(), checksum: None, size: None, backed_up_original_path: None },
        NewManifestEntry { file_path: "x/y.bin".into(), checksum: None, size: None, backed_up_original_path: None },
    ];
    let result = db
        .register_installation(id, InstallStatus::Enabled, Some(0), &entries)
        .await;
    assert!(result.is_err(), "expected unique constraint failure");

    // The installed_mods row should not exist because the tx was rolled back.
    let installed = db.installed_mods().await.unwrap();
    assert!(installed.is_empty());
    let manifest = db.manifest_for_mod(id).await.unwrap();
    assert!(manifest.is_empty(), "manifest must roll back too");
}

#[tokio::test]
async fn find_existing_paths_detects_conflicts() {
    let db = Database::in_memory().await.unwrap();
    let id = db.upsert_mod(&mod_("3", "Gamma")).await.unwrap();
    let entries = vec![NewManifestEntry {
        file_path: "conflict/path".into(),
        checksum: None,
        size: None,
        backed_up_original_path: None,
    }];
    db.register_installation(id, InstallStatus::Enabled, Some(0), &entries)
        .await
        .unwrap();

    let found = db
        .find_existing_paths(&["conflict/path".into(), "free/path".into()])
        .await
        .unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].file_path, "conflict/path");
}

#[tokio::test]
async fn delete_installation_drops_rows_and_invokes_restorer() {
    let db = Database::in_memory().await.unwrap();
    let id = db.upsert_mod(&mod_("4", "Delta")).await.unwrap();
    let entries = vec![NewManifestEntry {
        file_path: "d/file.bin".into(),
        checksum: None,
        size: None,
        backed_up_original_path: Some("/backup/d/file.bin".into()),
    }];
    db.register_installation(id, InstallStatus::Enabled, Some(0), &entries)
        .await
        .unwrap();

    let mut restored = Vec::new();
    db.delete_installation(id, |current, backup| {
        restored.push((current.to_string(), backup.to_string()));
        Ok(())
    })
    .await
    .unwrap();
    assert_eq!(restored, vec![("d/file.bin".to_string(), "/backup/d/file.bin".to_string())]);

    let installed = db.installed_mods().await.unwrap();
    assert!(installed.is_empty());
    let manifest = db.manifest_for_mod(id).await.unwrap();
    assert!(manifest.is_empty());
}
