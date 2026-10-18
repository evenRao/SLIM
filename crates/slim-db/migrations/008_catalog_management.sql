ALTER TABLE locations ADD COLUMN archived_at TEXT;
ALTER TABLE grid_bases ADD COLUMN archived_at TEXT;
ALTER TABLE gridfinity_bins ADD COLUMN archived_at TEXT;
ALTER TABLE automation_rules ADD COLUMN archived_at TEXT;
ALTER TABLE devices ADD COLUMN archived_at TEXT;
ALTER TABLE identifiers ADD COLUMN archived_at TEXT;
ALTER TABLE actors ADD COLUMN system_actor INTEGER NOT NULL DEFAULT 0;
CREATE TABLE storage_profiles (
 location_id TEXT PRIMARY KEY REFERENCES locations(id) ON DELETE CASCADE,
 type_label TEXT NOT NULL DEFAULT '',
 notes TEXT NOT NULL DEFAULT '',
 width_mm REAL CHECK(width_mm>0),
 depth_mm REAL CHECK(depth_mm>0),
 height_mm REAL CHECK(height_mm>0)
);
CREATE INDEX idx_locations_active ON locations(archived_at);
CREATE TRIGGER storage_parent_active_insert BEFORE INSERT ON locations
WHEN EXISTS(SELECT 1 FROM locations WHERE id=NEW.parent_id AND archived_at IS NOT NULL)
BEGIN SELECT RAISE(ABORT,'parent storage is deleted'); END;
CREATE TRIGGER storage_parent_active_update BEFORE UPDATE OF parent_id ON locations
WHEN EXISTS(SELECT 1 FROM locations WHERE id=NEW.parent_id AND archived_at IS NOT NULL)
BEGIN SELECT RAISE(ABORT,'parent storage is deleted'); END;
CREATE TRIGGER movement_active_storage BEFORE INSERT ON stock_movements
WHEN EXISTS(SELECT 1 FROM locations WHERE id=NEW.location_id AND archived_at IS NOT NULL)
BEGIN SELECT RAISE(ABORT,'storage is deleted'); END;
CREATE TRIGGER base_active_storage_insert BEFORE INSERT ON grid_bases
WHEN EXISTS(SELECT 1 FROM locations WHERE id=NEW.location_id AND archived_at IS NOT NULL)
BEGIN SELECT RAISE(ABORT,'storage is deleted'); END;
CREATE TRIGGER base_active_storage_update BEFORE UPDATE OF location_id ON grid_bases
WHEN EXISTS(SELECT 1 FROM locations WHERE id=NEW.location_id AND archived_at IS NOT NULL)
BEGIN SELECT RAISE(ABORT,'storage is deleted'); END;
CREATE TRIGGER placement_active_records BEFORE INSERT ON bin_placements
WHEN EXISTS(SELECT 1 FROM grid_bases WHERE id=NEW.base_id AND archived_at IS NOT NULL)
 OR EXISTS(SELECT 1 FROM gridfinity_bins WHERE id=NEW.bin_id AND archived_at IS NOT NULL)
BEGIN SELECT RAISE(ABORT,'base or bin is deleted'); END;
CREATE TRIGGER observation_active_device BEFORE INSERT ON observations
WHEN EXISTS(SELECT 1 FROM devices WHERE id=NEW.device_id AND (archived_at IS NOT NULL OR active=0))
BEGIN SELECT RAISE(ABORT,'device is inactive or deleted'); END;
CREATE TRIGGER identifier_active_target BEFORE INSERT ON identifiers
WHEN EXISTS(SELECT 1 FROM locations WHERE id=NEW.location_id AND archived_at IS NOT NULL)
 OR EXISTS(SELECT 1 FROM items WHERE id=NEW.item_id AND archived_at IS NOT NULL)
BEGIN SELECT RAISE(ABORT,'identifier target is deleted'); END;
