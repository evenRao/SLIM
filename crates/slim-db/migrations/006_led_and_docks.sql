-- ═══════════════════════════════════════════════════════════
-- SLIM SCHEMA MIGRATION 006: LED DOCK TARGETING & PRESENCE SENSORS
-- ═══════════════════════════════════════════════════════════

-- Physical and logical LED states across Gridfinity baseplate docks
CREATE TABLE IF NOT EXISTS dock_led_states (
    dock_id     TEXT PRIMARY KEY NOT NULL,
    base_id     TEXT NOT NULL REFERENCES grid_bases(id) ON DELETE CASCADE,
    x           INTEGER NOT NULL CHECK (x >= 0),
    y           INTEGER NOT NULL CHECK (y >= 0),
    mode        TEXT NOT NULL,
    r           INTEGER NOT NULL DEFAULT 0 CHECK (r >= 0 AND r <= 255),
    g           INTEGER NOT NULL DEFAULT 0 CHECK (g >= 0 AND g <= 255),
    b           INTEGER NOT NULL DEFAULT 0 CHECK (b >= 0 AND b <= 255),
    expires_at  TEXT,
    reason      TEXT,
    updated_at  TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_dock_led_base ON dock_led_states(base_id);
CREATE INDEX IF NOT EXISTS idx_dock_led_mode ON dock_led_states(mode);
CREATE INDEX IF NOT EXISTS idx_dock_led_expires ON dock_led_states(expires_at) WHERE expires_at IS NOT NULL;

-- Physical dock anchor occupancy and presence states
CREATE TABLE IF NOT EXISTS dock_presence_states (
    dock_id      TEXT PRIMARY KEY NOT NULL,
    base_id      TEXT NOT NULL REFERENCES grid_bases(id) ON DELETE CASCADE,
    x            INTEGER NOT NULL CHECK (x >= 0),
    y            INTEGER NOT NULL CHECK (y >= 0),
    occupied     INTEGER NOT NULL DEFAULT 0,
    last_seen_at TEXT NOT NULL,
    status       TEXT NOT NULL DEFAULT 'present'
);

CREATE INDEX IF NOT EXISTS idx_dock_presence_base ON dock_presence_states(base_id);
