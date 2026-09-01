//! Phase 5: Dependency & Load Order Logic tests.

use std::sync::Arc;
use tokio::sync::Mutex;

use nexus_mod_manager::api::Mod;
use nexus_mod_manager::db::Database;
use nexus_mod_manager::dependency::DependencyService;

fn mod_with_deps(id: &str, deps: &[&str]) -> Mod {
    Mod {
        id: 0,
        name: id.into(),
        version: "1.0".into(),
        description: String::new(),
        category: String::new(),
        nexus_id: id.into(),
        required_mod_ids: deps.iter().map(|s| s.to_string()).collect(),
        required_versions: Default::default(),
    }
}

async fn service_with(mods: Vec<Mod>) -> (Arc<Mutex<Database>>, DependencyService) {
    let db = Database::in_memory().await.unwrap();
    let db_arc = Arc::new(Mutex::new(db));
    let service = DependencyService::new(db_arc.clone());
    {
        let db = db_arc.lock().await;
        for m in mods {
            db.upsert_mod(&m).await.unwrap();
        }
    }
    (db_arc, service)
}

#[tokio::test]
async fn missing_requirements_lists_uninstalled_deps() {
    let (_db, svc) = service_with(vec![mod_with_deps("A", &["B", "C"])]).await;
    let missing = svc.missing_requirements("A").await.unwrap();
    assert_eq!(missing.len(), 2);
    assert!(missing.contains(&"B".to_string()));
    assert!(missing.contains(&"C".to_string()));
}

#[tokio::test]
async fn missing_requirements_excludes_installed_deps() {
    let (_db, svc) = service_with(vec![
        mod_with_deps("A", &["B"]),
        mod_with_deps("B", &[]),
    ])
    .await;
    // Mark B as installed
    let db = _db.clone();
    {
        let db = db.lock().await;
        let id = db.mod_id_by_nexus("B").await.unwrap().unwrap();
        db.register_installation(id, nexus_mod_manager::db::InstallStatus::Enabled, Some(0), &[])
            .await
            .unwrap();
    }
    let missing = svc.missing_requirements("A").await.unwrap();
    assert!(missing.is_empty(), "expected no missing deps; got {missing:?}");
}

#[tokio::test]
async fn circular_dependency_is_detected() {
    let (_db, svc) = service_with(vec![
        mod_with_deps("A", &["B"]),
        mod_with_deps("B", &["A"]),
    ])
    .await;
    let result = svc.check_dependencies("A").await;
    assert!(result.is_err(), "expected circular dep error");
    match result {
        Err(nexus_mod_manager::errors::AppError::CircularDependency(_)) => {}
        other => panic!("expected CircularDependency, got {other:?}"),
    }
}

#[tokio::test]
async fn load_order_toposorts_by_dependencies() {
    let (_db, svc) = service_with(vec![
        mod_with_deps("A", &["B"]),
        mod_with_deps("B", &["C"]),
        mod_with_deps("C", &[]),
    ])
    .await;
    let order = svc
        .assign_load_order(vec!["A".into(), "B".into(), "C".into()])
        .await
        .unwrap();
    let pos = |id: &str| order.iter().position(|(x, _)| x == id).unwrap();
    assert!(pos("C") < pos("B"));
    assert!(pos("B") < pos("A"));
}
