-- Migration 002: Gridfinity Storage Architecture
-- Adds baseplates, Gridfinity bins, and spatial placement tracking.

-- ═══════════════════════════════════════════════════════════
-- GRID BASES (baseplates mounted inside drawers/shelves/cabinets)
-- ═══════════════════════════════════════════════════════════
CREATE TABLE grid_bases (
    id          TEXT PRIMARY KEY NOT NULL,   -- UUID
    location_id TEXT NOT NULL REFERENCES locations(id),
    code        TEXT NOT NULL,               -- Unique human code e.g. 'BASE-A'
    cols        INTEGER NOT NULL CHECK (cols > 0),
    rows        INTEGER NOT NULL CHECK (rows > 0),
    pitch_mm    REAL NOT NULL DEFAULT 42.0 CHECK (pitch_mm > 0),
    created_at  TEXT NOT NULL                -- ISO 8601 UTC
);
CREATE UNIQUE INDEX idx_grid_bases_code ON grid_bases(code);
CREATE INDEX idx_grid_bases_location ON grid_bases(location_id);

-- ═══════════════════════════════════════════════════════════
-- GRIDFINITY BINS (standard footprint metadata for bin locations)
-- ═══════════════════════════════════════════════════════════
CREATE TABLE gridfinity_bins (
    id                TEXT PRIMARY KEY NOT NULL,   -- UUID
    location_id       TEXT NOT NULL REFERENCES locations(id),
    bin_code          TEXT NOT NULL,               -- Unique human code e.g. 'B-017'
    width_units       INTEGER NOT NULL CHECK (width_units > 0),
    depth_units       INTEGER NOT NULL CHECK (depth_units > 0),
    height_units      INTEGER NOT NULL CHECK (height_units > 0),
    compartment_count INTEGER NOT NULL DEFAULT 1 CHECK (compartment_count > 0),
    created_at        TEXT NOT NULL                -- ISO 8601 UTC
);
CREATE UNIQUE INDEX idx_gridfinity_bins_code ON gridfinity_bins(bin_code);
CREATE UNIQUE INDEX idx_gridfinity_bins_location ON gridfinity_bins(location_id);

-- ═══════════════════════════════════════════════════════════
-- BIN PLACEMENTS (spatial tracking of bins on baseplates)
-- ═══════════════════════════════════════════════════════════
CREATE TABLE bin_placements (
    id          TEXT PRIMARY KEY NOT NULL,   -- UUID
    bin_id      TEXT NOT NULL REFERENCES gridfinity_bins(id),
    base_id     TEXT NOT NULL REFERENCES grid_bases(id),
    x           INTEGER NOT NULL CHECK (x >= 0),
    y           INTEGER NOT NULL CHECK (y >= 0),
    rotation    INTEGER NOT NULL DEFAULT 0 CHECK (rotation IN (0, 90)),
    placed_at   TEXT NOT NULL,                -- ISO 8601 UTC
    removed_at  TEXT                          -- NULL while actively placed
);
CREATE INDEX idx_bin_placements_base ON bin_placements(base_id);
CREATE INDEX idx_bin_placements_bin ON bin_placements(bin_id);
CREATE INDEX idx_bin_placements_active ON bin_placements(base_id, removed_at) WHERE removed_at IS NULL;
