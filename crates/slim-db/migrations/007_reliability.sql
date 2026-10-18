-- Keep the most recently configured global threshold, then enforce uniqueness.
DELETE FROM stock_thresholds WHERE stock_location_id IS NULL AND rowid NOT IN
    (SELECT MAX(rowid) FROM stock_thresholds WHERE stock_location_id IS NULL GROUP BY item_id);
CREATE UNIQUE INDEX idx_threshold_global ON stock_thresholds(item_id) WHERE stock_location_id IS NULL;
ALTER TABLE operations ADD COLUMN request_fingerprint TEXT;
ALTER TABLE operations ADD COLUMN receipt_json TEXT;
ALTER TABLE event_outbox ADD COLUMN claimed_until TEXT;
CREATE TRIGGER balance_range_insert BEFORE INSERT ON stock_balances
WHEN NEW.qty > 4294967295 BEGIN SELECT RAISE(ABORT, 'quantity exceeds supported range'); END;
CREATE TRIGGER balance_range_update BEFORE UPDATE OF qty ON stock_balances
WHEN NEW.qty > 4294967295 BEGIN SELECT RAISE(ABORT, 'quantity exceeds supported range'); END;
CREATE UNIQUE INDEX idx_bin_one_active ON bin_placements(bin_id) WHERE removed_at IS NULL;
CREATE TRIGGER movement_no_update BEFORE UPDATE ON stock_movements
BEGIN SELECT RAISE(ABORT, 'stock movements are immutable'); END;
CREATE TRIGGER movement_no_delete BEFORE DELETE ON stock_movements
BEGIN SELECT RAISE(ABORT, 'stock movements are immutable'); END;
