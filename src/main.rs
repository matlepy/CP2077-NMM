use std::sync::Arc;

use tokio::sync::Mutex;

use nexus_mod_manager::api::NexusClient;
use nexus_mod_manager::config::Config;
use nexus_mod_manager::db::Database;
use nexus_mod_manager::dependency::DependencyService;
use nexus_mod_manager::engine::ModEngine;
use nexus_mod_manager::logging;
#[cfg(feature = "ui")]
use nexus_mod_manager::ui::application::Application;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1.4: initialize logging first, so subsequent init errors are visible.
    logging::init();

    // 1.3: load config (fail-fast on missing API key or invalid paths).
    let config = Config::load()?;
    tracing::info!(
        game_dir = ?config.game_directory,
        cache_dir = ?config.cache_directory,
        "configuration loaded"
    );

    // 2.x: initialize database (runs migrations).
    let database = Database::new(&config.database_path).await?;
    database.initialize().await?;
    tracing::info!(db = ?config.database_path, "database initialized");

    // 3.3: validate API key at startup.
    let api_client = NexusClient::new(config.nexus_api_key.clone());
    api_client.validate_api_key().await?;
    tracing::info!("Nexus API key validated");

    // Phase 5: dependency service.
    let db_arc = Arc::new(Mutex::new(database.clone()));
    let _dependency_service = DependencyService::new(db_arc.clone());

    // Phase 4: mod engine.
    let _engine = ModEngine::new(config.clone(), db_arc.clone());

    // Phase 6: run UI (blocks on the GTK main loop).
    #[cfg(feature = "ui")]
    {
        let app = Application::new(config, database, api_client, db_arc);
        app.run();
    }

    // When built without the UI feature, just print a one-line summary and exit
    // (this is the headless mode used by integration tests / CI smoke checks).
    #[cfg(not(feature = "ui"))]
    {
        tracing::info!("running in headless mode (no ui feature)");
    }

    Ok(())
}
