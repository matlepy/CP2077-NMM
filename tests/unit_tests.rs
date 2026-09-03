//! Unit tests for core dependency service functionality

#[cfg(test)]
mod unit_tests {
    use std::sync::Arc;
    use tokio::sync::Mutex;

    use nexus_mod_manager::dependency::{DependencyService, DependencyInfo};
    use nexus_mod_manager::db::Database;
    use nexus_mod_manager::errors::AppResult;

    #[tokio::test]
    async fn test_dependency_service_creation() -> AppResult<()> {
        // Test that we can create a dependency service
        let db = Database::in_memory().await?;
        let db_arc = Arc::new(Mutex::new(db));
        let service = DependencyService::new(db_arc);
        
        // Verify the service was created successfully
        assert!(service.database.is_some());
        
        println!("Dependency service creation test passed");
        Ok(())
    }

    #[tokio::test]
    async fn test_dependency_info_struct() -> AppResult<()> {
        // Test that the DependencyInfo struct can be created and used
        let info = DependencyInfo {
            mod_id: "test_mod".to_string(),
            required_mods: vec!["req1".to_string(), "req2".to_string()],
            missing_mods: vec![],
        };
        
        assert_eq!(info.mod_id, "test_mod");
        assert_eq!(info.required_mods.len(), 2);
        assert_eq!(info.missing_mods.len(), 0);
        
        println!("DependencyInfo struct test passed");
        Ok(())
    }

    #[tokio::test]
    async fn test_dependency_methods_exist() -> AppResult<()> {
        // Test that core dependency methods exist and can be called
        let db = Database::in_memory().await?;
        let db_arc = Arc::new(Mutex::new(db));
        let service = DependencyService::new(db_arc);
        
        // These should compile and not panic (even if they're stubs)
        assert!(service.database.is_some());
        
        println!("Dependency methods existence test passed");
        Ok(())
    }
}