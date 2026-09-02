//! GTK4 UI for the Nexus Mod Manager.
//!
//! Each view is a plain function `build(&AppState) -> gtk4::Widget` rather than
//! a GObject subclass. This keeps the UI code simple and avoids the
//! `CompositeTemplate` macro entirely.

use std::sync::Arc;

use crate::api::NexusClient;
use crate::config::Config;
use crate::db::Database;
use crate::engine::ModEngine;

pub mod application;
pub mod load_order;
pub mod main_window;
pub mod mod_browser;
pub mod progress;
pub mod settings;
pub mod dependency_manager;

pub use main_window::AppState;
pub use main_window::MainWindow;
