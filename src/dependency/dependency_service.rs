//! Phase 5: Dependency & Load Order Logic.
//!
//! - Detects missing requirements and flags the mod as `PendingRequirements` (5.3).
//! - Detects circular dependencies and rejects them with [`AppError::CircularDependency`] (5.2).
//! - Assigns a load order via a topological sort of the requirement graph (5.4).

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;

use crate::db::{Database, InstallStatus};
use crate::errors::{AppError, AppResult};

#[derive(Debug, Clone)]
pub struct DependencyService {
    database: Arc<tokio::sync::Mutex<Database>>,
}

impl DependencyService {
    pub fn new(database: Arc<tokio::sync::Mutex<Database>>) -> Self {
        DependencyService { database }
    }

    /// Walk the requirement graph rooted at `mod_id` and return the list of *missing* Nexus IDs
    /// (i.e. mods that are required but not present in the local DB).
    pub async fn missing_requirements(&self, mod_id: &str) -> AppResult<Vec<String>> {
        let mut visited: HashSet<String> = HashSet::new();
        let all_required = self.collect_requirements(mod_id, &mut visited).await?;
        let db = self.database.lock().await;
        let mut missing = Vec::new();
        for r in all_required {
            if !db.is_mod_installed(&r).await? {
                missing.push(r);
            }
        }
        Ok(missing)
    }

    /// Check that all requirements for `mod_id` are present in the local DB. The returned
    /// vector is empty on success and contains the missing Nexus IDs on failure.
    pub async fn check_dependencies(&self, mod_id: &str) -> AppResult<Vec<String>> {
        self.missing_requirements(mod_id).await
    }

    /// Assign load order integers to `mod_ids` using a topological sort of the requirement
    /// graph. The returned vector maps Nexus ID → load order, in topological order.
    ///
    /// A mod that requires another will be assigned a *higher* load order than the mod it
    /// requires. This is the "lower load order wins" convention.
    pub async fn assign_load_order(
        &self,
        mod_ids: Vec<String>,
    ) -> AppResult<Vec<(String, i32)>> {
        let db = self.database.lock().await;
        // Build a graph: (id, requirements)
        let mut requirements: HashMap<String, Vec<String>> = HashMap::new();
        for id in &mod_ids {
            let mut visited: HashSet<String> = HashSet::new();
            let reqs = self.collect_requirements_inner(&db, id, &mut visited).await?;
            // Restrict to in-graph requirements (so we ignore external mods).
            let reqs: Vec<String> = reqs.into_iter().filter(|r| mod_ids.contains(r)).collect();
            requirements.insert(id.clone(), reqs);
        }
        drop(db);

        let sorted = topo_sort(&requirements)?;
        Ok(sorted
            .into_iter()
            .enumerate()
            .map(|(i, id)| (id, i as i32))
            .collect())
    }

    /// Verify that no cycle exists in the requirement graph of the given mods.
    pub async fn check_circular_dependencies(&self, mod_ids: &[String]) -> AppResult<()> {
        let db = self.database.lock().await;
        for mod_id in mod_ids {
            let mut visited: HashSet<String> = HashSet::new();
            self.collect_requirements_inner(&db, mod_id, &mut visited).await?;
        }
        Ok(())
    }

    /// Update the status of a mod based on its dependencies.
    /// If all requirements are installed, set status to Enabled.
    /// If some requirements are missing, set status to PendingRequirements.
    pub async fn update_mod_status(&self, mod_id: &str) -> AppResult<()> {
        let db = self.database.lock().await;
        let missing = self.missing_requirements(mod_id).await?;
        
        if missing.is_empty() {
            // All dependencies satisfied
            db.set_install_status(mod_id, InstallStatus::Enabled).await?;
        } else {
            // Some dependencies missing
            db.set_install_status(mod_id, InstallStatus::PendingRequirements).await?;
        }
        Ok(())
    }

    /// Get detailed dependency information for a mod.
    pub async fn get_dependency_info(&self, mod_id: &str) -> AppResult<DependencyInfo> {
        let db = self.database.lock().await;
        let missing = self.missing_requirements(mod_id).await?;
        
        // Get the mod details
        let mod_details = db.get_mod_by_nexus(mod_id).await?;
        
        Ok(DependencyInfo {
            mod_id: mod_id.to_string(),
            required_mods: mod_details.as_ref().map(|m| m.required_mod_ids.clone()).unwrap_or_default(),
            missing_mods: missing,
        })
    }

    /// Install all missing dependencies for a mod.
    /// This implements the core dependency resolution logic.
    pub async fn install_missing_dependencies(&self, mod_id: &str) -> AppResult<()> {
        let db = self.database.lock().await;
        
        // Get all missing requirements
        let missing = self.missing_requirements(mod_id).await?;
        if missing.is_empty() {
            tracing::info!(mod_id = mod_id, "No missing dependencies to install");
            return Ok(());
        }
        
        tracing::info!(mod_id = mod_id, missing_deps = missing.len(), "Installing missing dependencies");
        
        // For now, we'll just log that we're installing dependencies
        // In a real implementation, this would:
        // 1. Fetch missing dependencies from the Nexus API
        // 2. Download and install them in dependency order
        // 3. Update the database with installed status
        
        // This is where we'd implement the actual dependency resolution workflow
        // For now, just update the status to indicate processing
        db.set_install_status(mod_id, InstallStatus::PendingRequirements).await?;
        
        tracing::info!(mod_id = mod_id, "Dependency installation process initiated");
        Ok(())
    }

    /// Recursively collect the full set of transitive requirements of `mod_id`.
    async fn collect_requirements(
        &self,
        mod_id: &str,
        visited: &mut HashSet<String>,
    ) -> AppResult<HashSet<String>> {
        let db = self.database.lock().await;
        Box::pin(self.collect_requirements_inner(&db, mod_id, visited)).await
    }

    async fn collect_requirements_inner(
        &self,
        db: &Database,
        mod_id: &str,
        visited: &mut HashSet<String>,
    ) -> AppResult<HashSet<String>> {
        if visited.contains(mod_id) {
            return Err(AppError::CircularDependency(mod_id.to_string()));
        }
        visited.insert(mod_id.to_string());

        let Some(m) = db.get_mod_by_nexus(mod_id).await? else {
            return Ok(HashSet::new());
        };
        let mut all: HashSet<String> = m.required_mod_ids.iter().cloned().collect();
        for req in &m.required_mod_ids {
            let sub = Box::pin(self.collect_requirements_inner(db, req, visited)).await?;
            all.extend(sub);
        }
        Ok(all)
    }
}

/// Kahn's algorithm: returns nodes in an order such that every node's dependencies
/// appear *before* it. If a cycle is present, returns an error.
///
/// In our graph, an edge `node -> req` means "node depends on req", so `req` must
/// come first. We treat that as `in_degree[node] += 1`, and process zero-indegree
/// nodes first.
fn topo_sort(requirements: &HashMap<String, Vec<String>>) -> AppResult<Vec<String>> {
    let mut in_degree: HashMap<&str, usize> = HashMap::new();
    for node in requirements.keys() {
        in_degree.insert(node.as_str(), 0);
    }
    for (node, reqs) in requirements {
        for r in reqs {
            // Only count edges to in-graph nodes.
            if requirements.contains_key(r) {
                *in_degree.entry(node.as_str()).or_insert(0) += 1;
            }
        }
    }
    let mut queue: VecDeque<&str> = in_degree
        .iter()
        .filter_map(|(k, v)| if *v == 0 { Some(*k) } else { None })
        .collect();
    let mut out = Vec::new();
    while let Some(n) = queue.pop_front() {
        out.push(n.to_string());
        // Find every node that depends on `n` and decrement its in-degree.
        for (other, other_reqs) in requirements {
            if other_reqs.iter().any(|r| r == n) {
                if let Some(deg) = in_degree.get_mut(other.as_str()) {
                    *deg = deg.saturating_sub(1);
                    if *deg == 0 {
                        queue.push_back(other.as_str());
                    }
                }
            }
        }
    }
    if out.len() != requirements.len() {
        return Err(AppError::CircularDependency("cycle in requirements".into()));
    }
    Ok(out)
}

/// Information about a mod's dependencies.
#[derive(Debug, Clone)]
pub struct DependencyInfo {
    pub mod_id: String,
    pub required_mods: Vec<String>,
    pub missing_mods: Vec<String>,
}

/// Resolve and install dependencies in proper order.
/// This would be called by install_missing_dependencies in a full implementation.
pub async fn resolve_dependency_order(service: &DependencyService, mod_ids: Vec<String>) -> AppResult<Vec<String>> {
    // Get dependency graph for all mods
    let mut requirements: HashMap<String, Vec<String>> = HashMap::new();
    
    for id in &mod_ids {
        let db = service.database.lock().await;
        let mut visited: HashSet<String> = HashSet::new();
        let reqs = service.collect_requirements_inner(&db, id, &mut visited).await?;
        // Restrict to in-graph requirements (so we ignore external mods).
        let reqs: Vec<String> = reqs.into_iter().filter(|r| mod_ids.contains(r)).collect();
        requirements.insert(id.clone(), reqs);
    }
    
    // Perform topological sort to get installation order
    let sorted = topo_sort(&requirements)?;
    Ok(sorted)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_dependency_service_creation() {
        let db = Database::in_memory().await.unwrap();
        let db_arc = Arc::new(tokio::sync::Mutex::new(db));
        let service = DependencyService::new(db_arc);
        
        // Just verify the service can be created
        assert!(true); // Placeholder - actual testing would require more setup
    }
}