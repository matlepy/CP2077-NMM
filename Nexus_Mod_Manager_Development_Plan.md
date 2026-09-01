# Project Roadmap: Nexus Mod Manager (CP2077)

## Project Overview
A high-performance, modular mod manager written in Rust for Linux (Arch/Wayland). The application manages Cyberpunk 2077 mods by interfacing with the Nexus Mods API, handling downloads via a local cache, and managing installations through a local SQLite database.

### Tech Stack
- **Language:** Rust
- **UI Toolkit:** GTK4 (`gtk4-rs`) with `libadwaita` (for modern GNOME/Wayland look)
- **Database:** SQLite via `sqlx` (async)
- **Networking:** `reqwest` (async)
- **Async Runtime:** `tokio`
- **Serialization:** `serde` / `serde_json`
- **Configuration:** `config` or custom TOML-based implementation
- **Archives:** `zip`, `sevenz-rust`, `unrar` (or shelling out to `unrar`/`bsdtar` — see 4.2)
- **Secrets:** read from `NEXUS_API_KEY` environment variable (see 1.3)

---

## Phase 1: Project Scaffolding & Configuration
*Goal: Establish the project structure and the ability to handle variable paths.*

- [ ] **1.1 Initialize Workspace:** Create a new Cargo project with a modular directory structure (e.g., `src/api`, `src/db`, `src/engine`, `src/ui`, `src/config`).
- [ ] **1.2 Dependency Management:** Add `tokio`, `reqwest`, `sqlx`, `gtk4`, `libadwaita`, `serde`, `anyhow`, `thiserror`, `tracing`, `tracing-subscriber`, and `config` to `Cargo.toml`.
- [ ] **1.3 Config Module:**
    - Implement a `Config` struct that loads from a file (e.g., `$XDG_CONFIG_HOME/cp2077-manager/config.toml`, falling back to `~/.config/...`).
    - Fields must include: `game_directory`, `cache_directory` (default under `$XDG_CACHE_HOME`), and `database_path`.
    - **`nexus_api_key` is NOT stored in the TOML file.** Read it exclusively from the `NEXUS_API_KEY` environment variable at startup. Fail fast with a clear error if unset. Never log its value; redact it in `tracing` spans and error messages.
    - Validate `game_directory` and `cache_directory` as absolute paths that exist and are writable; create `cache_directory` if missing.
- [ ] **1.4 Logging Setup:** Initialize `tracing-subscriber` for debugging and error tracking; ensure a redaction layer/filter keeps the API key out of logs even at `TRACE` level.
- [ ] **1.5 Error Strategy:** Define a top-level `AppError` enum (`thiserror`) with variants for `Api`, `Io`, `Db`, `Extraction`, `Config`; use `anyhow` only at the outermost call sites (UI/main). This makes 7.1's "no silent crashes" requirement enforceable instead of aspirational.

**Definition of Done (DoD):** Project compiles; application reads `NEXUS_API_KEY` from the environment (refusing to start with a clear error if missing) and loads a config file, printing `game_directory` and `cache_directory` to the console.

---

## Phase 2: Data Layer (Persistence)
*Goal: Implement a robust local database for tracking mods and files.*

- [ ] **2.1 Database Setup:** Configure `sqlx` with the SQLite driver; use `sqlx-cli` (or `sqlx::migrate!`) for versioned migrations, enable `PRAGMA foreign_keys = ON`.
- [ ] **2.2 Schema Design (Migrations):** Create SQL migration files for:
    - `mods`: (id, name, version, nexus_id UNIQUE, description, category).
    - `installed_mods`: (mod_id FK→mods, installation_date, status ENUM-like TEXT CHECK: `enabled|disabled|pending_requirements`, load_order INTEGER).
    - `file_manifest`: (id, mod_id FK→mods ON DELETE CASCADE, file_path, checksum, size, backed_up_original_path NULLABLE) — *Crucial for clean uninstallation and rollback.*
    - `file_conflicts`: (file_path, mod_id, resolved_winner_mod_id NULLABLE) — tracks when two installed mods claim the same target path.
    - `settings`: (key, value) — optional, if not using a separate config file.
    - Add indices on `mods.nexus_id`, `file_manifest.mod_id`, and `file_manifest.file_path` (the last is what makes conflict detection a cheap query instead of a full scan).
- [ ] **2.3 Repository Pattern:** Implement a `Database` module with async functions to:
    - Query installed mods (with load order).
    - Register a new mod installation, wrapped in a single transaction with its file_manifest inserts (all-or-nothing).
    - Remove a mod and its associated files from the manifest, restoring any `backed_up_original_path` entries.
    - Fetch mod metadata.
    - Query `file_manifest` by `file_path` to detect conflicts before deployment.

**DoD:** Migrations run successfully; basic CRUD operations on `mods` and `installed_mods` can be tested via a CLI test script; a manual test proves a failed multi-file insert rolls back cleanly.

---

## Phase 3: Nexus API Integration
*Goal: Fetch mod metadata and files from Nexus Mods, and support one-click `nxm://` downloads.*

- [ ] **3.1 API Client:** Implement a `NexusClient` using `reqwest`, built around the **REST API v1** (`api.nexusmods.com`) as the primary, fully key-authenticated surface — this is the long-term-supported endpoint for search, mod/file listing, changelogs, and download-link generation.
- [ ] **3.2 GraphQL Supplement:** Use the GraphQL v2 endpoint (`graphql.nexusmods.com`) only for data REST doesn't cover well (e.g., richer filtered search, dependency graphs). Note: most GraphQL queries work unauthenticated, but some fields require an OAuth token that must be requested from Nexus support separately from a plain API key — don't assume parity with REST auth.
- [ ] **3.3 Authentication:** Read the key from `NEXUS_API_KEY` (passed down from Config, per 1.3) and include it in the `apikey` header on every REST request. Call `/v1/users/validate.json` at startup to confirm the key is valid and surface a clear UI error if not.
- [ ] **3.4 Rate Limit Handling:** Implement retry/backoff for HTTP 429s, and parse the `X-RL-Hourly-Remaining` / `X-RL-Daily-Remaining` response headers so the UI can show users their remaining quota before they hit it.
- [ ] **3.5 `nxm://` Protocol Handler:** Register the app as the OS handler for the `nxm://` URI scheme (via a `.desktop` file `MimeType=x-scheme-handler/nxm;`). Parse incoming links (`nxm://<game_domain>/mods/<mod_id>/files/<file_id>?key=...&expires=...`), exchange them via the REST API for a real download URL, and hand off to the Download Manager (Phase 4). This is what makes "Mod Manager Download" buttons on the website work — without it, users are limited to manual copy-paste of links.

**DoD:** Client can validate the API key, search for "Cyberpunk 2077" mods, return mod metadata, and — separately — resolve a sample `nxm://` link into a downloadable URL.

---

## Phase 4: The Mod Engine (IO & Filesystem)
*Goal: Handle the lifecycle of a mod file: Download → Cache → Extract → Deploy.*

- [ ] **4.1 Download Manager:**
    - Implement an async downloader that streams files to the `cache_directory` specified in Config, keyed off both manual searches and `nxm://` link resolution (3.5).
    - Support progress reporting (percentage/speed) via a channel/callback.
    - Verify downloaded file size/checksum against API-provided metadata before treating it as complete.
- [ ] **4.2 Extraction Service:**
    - Implement extraction for `.zip`, `.7z`, and `.rar` (a large share of CP2077 mods ship as RAR — via `unrar` bindings or shelling out to a system `unrar`/`bsdtar` binary; document the runtime dependency).
    - **Security:** Implement "Zip Slip" protection (validate that extracted files do not attempt to write outside the target directory) for all three formats.
    - Detect the correct archive root (many mods nest content one or two folders deep instead of placing `archive/`, `r6/`, `red4ext/` at the top level).
- [ ] **4.3 Deployment Logic:**
    - **Conflict check first:** before moving any file, query `file_manifest` (2.3) for existing entries at the same target path. If found, surface the conflicting mod(s) to the UI for user resolution (overwrite / skip / cancel) rather than silently clobbering.
    - **Backup on overwrite:** when a file is about to be replaced, copy the original to a per-mod backup location under `cache_directory` and record its path in `file_manifest.backed_up_original_path`, so uninstall can restore it.
    - Move extracted files from `cache_directory` to `game_directory`.
    - Implement an "Atomic Move" strategy where possible (rename within the same filesystem; fall back to copy+fsync+rename across filesystems) to prevent corrupt installations.
    - **Manifest Update:** on successful move, record every file path, checksum, and size in `file_manifest` within the same transaction as the `installed_mods` row (2.3).
- [ ] **4.4 Cleanup Service:** Implement a way to clear the `cache_directory` of old/unused files, but never delete active backup entries referenced by `file_manifest`.

**DoD:** A file can be downloaded to cache, extracted (including a `.rar` sample), and moved to a dummy directory, with all paths recorded in the DB; deploying a second mod that targets an already-installed file path triggers a conflict flag instead of a silent overwrite.

---

## Phase 5: Dependency & Load Order Logic
*Goal: Manage mod requirements, load order, and prevent broken installations.*

- [ ] **5.1 Requirement Model:** Extend the `Mod` metadata to include a list of `required_mod_ids` and, where available, minimum required versions.
- [ ] **5.2 Dependency Resolver:**
    - Implement a function that takes a `ModID` and returns a list of missing `ModIDs` (and any that are present but below the required version) based on the local DB.
    - Detect circular dependencies and reject/flag them rather than looping.
- [ ] **5.3 Implementation of the "Prompt":**
    - Create logic that flags a mod for installation as "Pending Requirements" if dependencies are missing.
    - Logic must support "Opt-out" (skipping the requirement and proceeding with the installation, acknowledging the risk).
- [ ] **5.4 Load Order:**
    - Persist an explicit `load_order` integer per installed mod (schema already supports this — 2.2).
    - Provide a way to reorder mods and have that ordering influence conflict resolution defaults in 4.3 (e.g., "last in load order wins" as the default, user-overridable).

**DoD:** Attempting to "install" a mod with a missing dependency returns a list of required mods; reordering two conflicting mods changes which one's files win on redeploy.

---

## Phase 6: GTK4 UI Implementation
*Goal: Provide a polished, native-feeling Linux interface.*

- [ ] **6.1 Main Window & Layout:**
    - Use `libadwaita` for a modern sidebar/content area layout.
    - Implement a `HeaderBar`.
- [ ] **6.2 View: Mod Browser:**
    - Implement a `ListView` or `GridView` to show mods.
    - Add a search bar and filter dropdowns (Category, Sort By).
- [ ] **6.3 View: Settings:**
    - Fields for `game_directory` (using `GtkFileChooser`) and `cache_directory`.
    - Show whether `NEXUS_API_KEY` is detected/valid (via 3.3's validate call) — read-only status, not an editable text field, since the key lives in the environment, not the config file.
- [ ] **6.4 View: Progress & Notifications:**
    - Use `GtkProgressBar` to show download/extraction status.
    - Use `AdwToast` or `GtkMessageDialog` for errors, success notifications, and file-conflict prompts (4.3).
- [ ] **6.5 View: Load Order:** A reorderable list (drag-and-drop) of installed mods reflecting 5.4.
- [ ] **6.6 Signal Integration:** Connect UI buttons (Install, Uninstall, Search, Reorder) to the `NexusClient` and `ModEngine`.
- [ ] **6.7 Async/UI Integration:** Bridge `tokio` tasks to GTK's `glib` main loop via `glib::MainContext::spawn_local` and `async_channel`/`glib::Sender` — do not touch GTK widgets directly from a raw `tokio::spawn`'d task, as GTK is not thread-safe. This is the single most common integration bug in `gtk4-rs` + `tokio` apps.

**DoD:** The application opens, allows setting the game directory, shows API-key validity status, searches for mods, reorders load order, and shows a progress bar during a mock download — all without UI freezes or thread-safety panics.

---

## Phase 7: Integration & Robustness
*Goal: Final hardening and error handling.*

- [ ] **7.1 Global Error Handling:** Ensure all errors (API, IO, DB) surface through the `AppError` type (1.5) and are displayed to the user via the UI (no silent crashes, no `unwrap()`/`expect()` on fallible paths outside tests).
- [ ] **7.2 Integrity Check:** Implement a "Verify Files" button that compares the `file_manifest` in the DB against the actual files in `game_directory` (checksum + existence), flagging missing or modified files.
- [ ] **7.3 Wayland Testing:** Verify window scaling and input work correctly in a Wayland compositor (Sway/GNOME).
- [ ] **7.4 Automated Tests:** Unit tests for the dependency resolver (5.2), conflict detector (4.3), and Zip Slip protection (4.2); integration tests against a temp SQLite DB for the repository layer (2.3).

**DoD:** The full workflow (Search → Detect Requirements → Download/Extract → Conflict Check → Install → Verify) works end-to-end, including at least one `nxm://` link and one deliberate file conflict.

---

## Phase 8: Packaging & Distribution
*Goal: Make the app installable the way its target audience (Arch/Wayland users) expects.*

- [ ] **8.1 AUR Package:** Write a `PKGBUILD` for distribution via the AUR.
- [ ] **8.2 Desktop Integration:** Ship a `.desktop` file registering both the app launcher and the `nxm://` scheme handler (3.5), plus an icon.
- [ ] **8.3 (Optional) Flatpak:** Evaluate a Flatpak manifest for portability beyond Arch, noting GTK4/libadwaita sandboxing considerations for filesystem access to arbitrary `game_directory` paths.

**DoD:** A user can install the package from the AUR, and clicking a "Mod Manager Download" button on the Nexus Mods website launches the app with the correct mod queued for download.
