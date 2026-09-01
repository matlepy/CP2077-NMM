-- Add dependency tracking (Phase 5).
CREATE TABLE IF NOT EXISTS mod_requirements (
    mod_id INTEGER NOT NULL,
    required_nexus_id TEXT NOT NULL,
    required_version TEXT,
    PRIMARY KEY (mod_id, required_nexus_id),
    FOREIGN KEY (mod_id) REFERENCES mods(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_mod_requirements_required ON mod_requirements(required_nexus_id);
