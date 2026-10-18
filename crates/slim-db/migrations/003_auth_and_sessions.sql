-- Migration 003: Authentication, sessions, and audit trail

-- Extend actors table with role, hashed PIN, and NFC/barcode badge token
ALTER TABLE actors ADD COLUMN role TEXT NOT NULL DEFAULT 'operator';
ALTER TABLE actors ADD COLUMN pin_hash TEXT;
ALTER TABLE actors ADD COLUMN badge_token TEXT;

CREATE UNIQUE INDEX IF NOT EXISTS idx_actors_badge ON actors(badge_token) WHERE badge_token IS NOT NULL;

-- Sessions table for local station authentication
CREATE TABLE IF NOT EXISTS sessions (
    id         TEXT PRIMARY KEY NOT NULL,
    actor_id   TEXT NOT NULL REFERENCES actors(id) ON DELETE CASCADE,
    token      TEXT UNIQUE NOT NULL,
    created_at TEXT NOT NULL,
    expires_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_sessions_token ON sessions(token);
CREATE INDEX IF NOT EXISTS idx_sessions_actor ON sessions(actor_id);

-- Audit log for administrative and security operations
CREATE TABLE IF NOT EXISTS audit_events (
    id         TEXT PRIMARY KEY NOT NULL,
    actor_id   TEXT REFERENCES actors(id) ON DELETE SET NULL,
    action     TEXT NOT NULL,
    details    TEXT,
    created_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_audit_actor ON audit_events(actor_id);
CREATE INDEX IF NOT EXISTS idx_audit_created ON audit_events(created_at);
