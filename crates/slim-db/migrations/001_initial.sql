-- SLIM Schema Migration 001: Initial Schema
-- Creates all tables for Phase 1-2 (project foundation + inventory core).

-- ═══════════════════════════════════════════════════════════
-- ACTORS
-- ═══════════════════════════════════════════════════════════
CREATE TABLE actors (
    id           TEXT PRIMARY KEY NOT NULL,
    display_name TEXT NOT NULL,
    os_username  TEXT,
    active       INTEGER NOT NULL DEFAULT 1,
    created_at   TEXT NOT NULL
);
CREATE UNIQUE INDEX idx_actors_display_name ON actors(display_name);

-- ═══════════════════════════════════════════════════════════
-- ITEMS (catalog)
-- ═══════════════════════════════════════════════════════════
CREATE TABLE items (
    id          TEXT PRIMARY KEY NOT NULL,
    sku         TEXT,
    name        TEXT NOT NULL,
    unit        TEXT NOT NULL DEFAULT 'each',
    description TEXT,
    metadata    TEXT,
    archived_at TEXT,
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL
);
CREATE UNIQUE INDEX idx_items_sku ON items(sku) WHERE sku IS NOT NULL;

-- ═══════════════════════════════════════════════════════════
-- LOCATIONS (hierarchical, bins are subtypes per ADR-01)
-- ═══════════════════════════════════════════════════════════
CREATE TABLE locations (
    id             TEXT PRIMARY KEY NOT NULL,
    parent_id      TEXT REFERENCES locations(id),
    name           TEXT NOT NULL,
    kind           TEXT NOT NULL,
    can_hold_stock INTEGER NOT NULL DEFAULT 1,
    created_at     TEXT NOT NULL,
    updated_at     TEXT NOT NULL
);
CREATE UNIQUE INDEX idx_locations_parent_name ON locations (COALESCE(parent_id, '__ROOT__'), name);

-- ═══════════════════════════════════════════════════════════
-- IDENTIFIERS (barcode, QR, manual alias)
-- ═══════════════════════════════════════════════════════════
CREATE TABLE identifiers (
    id          TEXT PRIMARY KEY NOT NULL,
    kind        TEXT NOT NULL,
    value       TEXT NOT NULL,
    item_id     TEXT REFERENCES items(id),
    location_id TEXT REFERENCES locations(id),
    created_at  TEXT NOT NULL,
    CHECK (
        (item_id IS NOT NULL AND location_id IS NULL) OR
        (item_id IS NULL AND location_id IS NOT NULL)
    )
);
CREATE UNIQUE INDEX idx_identifiers_kind_value ON identifiers(kind, value);

-- ═══════════════════════════════════════════════════════════
-- STOCK THRESHOLDS (ADR-04: surrogate PK + UNIQUE index)
-- ═══════════════════════════════════════════════════════════
CREATE TABLE stock_thresholds (
    id                TEXT PRIMARY KEY NOT NULL,
    item_id           TEXT NOT NULL REFERENCES items(id),
    stock_location_id TEXT REFERENCES locations(id),
    min_qty           INTEGER NOT NULL CHECK (min_qty >= 0),
    UNIQUE(item_id, stock_location_id)
);

-- ═══════════════════════════════════════════════════════════
-- OPERATIONS (transaction header)
-- ═══════════════════════════════════════════════════════════
CREATE TABLE operations (
    id              TEXT PRIMARY KEY NOT NULL,
    kind            TEXT NOT NULL,
    actor_id        TEXT NOT NULL REFERENCES actors(id),
    source          TEXT NOT NULL DEFAULT 'cli',
    reason          TEXT,
    idempotency_key TEXT,
    created_at      TEXT NOT NULL
);
CREATE UNIQUE INDEX idx_operations_idempotency
    ON operations(idempotency_key) WHERE idempotency_key IS NOT NULL;

-- ═══════════════════════════════════════════════════════════
-- STOCK MOVEMENTS (immutable ledger lines)
-- ═══════════════════════════════════════════════════════════
CREATE TABLE stock_movements (
    id           TEXT PRIMARY KEY NOT NULL,
    operation_id TEXT NOT NULL REFERENCES operations(id),
    item_id      TEXT NOT NULL REFERENCES items(id),
    location_id  TEXT NOT NULL REFERENCES locations(id),
    delta_qty    INTEGER NOT NULL,
    created_at   TEXT NOT NULL
);
CREATE INDEX idx_movements_operation ON stock_movements(operation_id);
CREATE INDEX idx_movements_item ON stock_movements(item_id);
CREATE INDEX idx_movements_location ON stock_movements(location_id);
CREATE INDEX idx_movements_created ON stock_movements(created_at);

-- ═══════════════════════════════════════════════════════════
-- STOCK BALANCES (transactionally maintained cache)
-- ═══════════════════════════════════════════════════════════
CREATE TABLE stock_balances (
    item_id     TEXT NOT NULL REFERENCES items(id),
    location_id TEXT NOT NULL REFERENCES locations(id),
    qty         INTEGER NOT NULL DEFAULT 0 CHECK (qty >= 0),
    updated_at  TEXT NOT NULL,
    PRIMARY KEY (item_id, location_id)
);

-- ═══════════════════════════════════════════════════════════
-- EVENT OUTBOX (durable post-commit events)
-- ═══════════════════════════════════════════════════════════
CREATE TABLE event_outbox (
    id            TEXT PRIMARY KEY NOT NULL,
    event_type    TEXT NOT NULL,
    aggregate_id  TEXT NOT NULL,
    payload       TEXT NOT NULL,
    created_at    TEXT NOT NULL,
    delivered_at  TEXT,
    attempt_count INTEGER NOT NULL DEFAULT 0,
    last_error    TEXT
);
CREATE INDEX idx_outbox_pending ON event_outbox(delivered_at) WHERE delivered_at IS NULL;
