-- ═══════════════════════════════════════════════════════════
-- SLIM SCHEMA MIGRATION 005: HARDWARE DEVICES AND OBSERVATIONS
-- ═══════════════════════════════════════════════════════════

-- Registered hardware devices (IR breakbeams, presence sensors, scanners, RFID)
CREATE TABLE IF NOT EXISTS devices (
    id              TEXT PRIMARY KEY NOT NULL,
    name            TEXT NOT NULL,
    kind            TEXT NOT NULL,
    port_or_address TEXT,
    location_id     TEXT REFERENCES locations(id),
    settings_json   TEXT,
    active          INTEGER NOT NULL DEFAULT 1,
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL
);

-- Raw physical sensor observations
CREATE TABLE IF NOT EXISTS observations (
    id                      TEXT PRIMARY KEY NOT NULL,
    device_id               TEXT NOT NULL REFERENCES devices(id) ON DELETE CASCADE,
    observation_type        TEXT NOT NULL,
    payload                 TEXT NOT NULL,
    status                  TEXT NOT NULL DEFAULT 'pending_review',
    processed_operation_id  TEXT REFERENCES operations(id),
    created_at              TEXT NOT NULL,
    confirmed_at            TEXT,
    confirmed_by            TEXT REFERENCES actors(id)
);

CREATE INDEX IF NOT EXISTS idx_observations_device ON observations(device_id);
CREATE INDEX IF NOT EXISTS idx_observations_status ON observations(status);
CREATE INDEX IF NOT EXISTS idx_observations_created ON observations(created_at);
