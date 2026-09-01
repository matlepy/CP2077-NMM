-- Initial schema for the Nexus Mod Manager (CP2077).
-- Phase 2: Data Layer.

CREATE TABLE IF NOT EXISTS mods (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL,
    version TEXT,
    nexus_id TEXT UNIQUE NOT NULL,
    description TEXT,
    category TEXT
);

CREATE TABLE IF NOT EXISTS installed_mods (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    mod_id INTEGER NOT NULL,
    installation_date TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('enabled', 'disabled', 'pending_requirements')),
    load_order INTEGER,
    FOREIGN KEY (mod_id) REFERENCES mods(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS file_manifest (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    mod_id INTEGER NOT NULL,
    file_path TEXT NOT NULL UNIQUE,
    checksum TEXT,
    size INTEGER,
    backed_up_original_path TEXT,
    FOREIGN KEY (mod_id) REFERENCES mods(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS file_conflicts (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    file_path TEXT NOT NULL,
    mod_id INTEGER NOT NULL,
    resolved_winner_mod_id INTEGER,
    FOREIGN KEY (mod_id) REFERENCES mods(id) ON DELETE CASCADE,
    UNIQUE (file_path, mod_id)
);

CREATE TABLE IF NOT EXISTS settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_mods_nexus_id ON mods(nexus_id);
CREATE INDEX IF NOT EXISTS idx_file_manifest_mod_id ON file_manifest(mod_id);
CREATE INDEX IF NOT EXISTS idx_file_manifest_file_path ON file_manifest(file_path);
