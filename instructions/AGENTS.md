# AGENTS.md

## Project Structure
This is a Rust application for managing Cyberpunk 2077 mods using the Nexus API. It's a GTK-based UI application with a headless mode.

Key directories:
- `src/` - Main source code
- `src/api/` - Nexus API client implementation
- `src/db/` - Database handling with SQLx and SQLite
- `src/ui/` - GTK-based user interface
- `migrations/` - Database schema migrations
- `tests/` - Test files

## Build & Run Commands
- Build: `cargo build`
- Build with UI: `cargo build --features ui` 
- Build without UI (headless): `cargo build --no-default-features`
- Run: `cargo run`
- Run without UI: `cargo run --no-default-features`
- Test: `cargo test`
- Test specific module: `cargo test <module_name>`
- Format code: `cargo fmt`

## Key Features
- Uses GTK4 with libadwaita for UI
- SQLite database with migrations via SQLx
- Async/await with Tokio runtime
- Nexus API integration
- Mod dependency resolution

## Environment Requirements
- Rust toolchain (stable)
- Cargo package manager
- SQLite development libraries (may be needed for build)

## Testing Notes
- Run individual tests with `cargo test <test_name>`
- Headless mode can be enabled with `--no-default-features`
- Database migrations run automatically on startup

## Architecture Notes
- Main entrypoint is `src/main.rs`
- Configuration loading happens early in the startup process
- Database initialization runs migrations automatically
- API key validation occurs at startup
- The application uses Arc<Mutex<>> for shared state between components