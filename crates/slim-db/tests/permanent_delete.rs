use slim_core::{
    model::{actor::*, identifier::*, item::*, location::*, operation::*, stock::StockThreshold},
    service::*,
};
use slim_db::{
    connection::open_memory,
    migrate::run_migrations,
    repo::{
        actor::SqliteActorRepo,
        identifier::SqliteIdentifierRepo,
        inventory::{SqliteInventoryService, SqliteReconciliationService, SqliteThresholdRepo},
        item::SqliteItemRepo,
        location::SqliteLocationRepo,
    },
};

fn seed() -> (rusqlite::Connection, ActorId, ItemId, LocationId) {
    let c = open_memory().unwrap();
    run_migrations(&c).unwrap();
    let a = SqliteActorRepo::new(&c)
        .create(&NewActor::new("Owner").with_role(Role::Admin))
        .unwrap();
    let i = item(&c, "Erase me", "ERASE");
    let l = SqliteLocationRepo::new(&c)
        .create(&NewLocation {
            parent_id: None,
            name: "Store".into(),
            kind: LocationKind::Storage,
        })
        .unwrap();
    (c, a.id, i, l.id)
}
fn item(c: &rusqlite::Connection, name: &str, sku: &str) -> ItemId {
    SqliteItemRepo::new(c)
        .create(&NewItem {
            sku: Some(sku.into()),
            name: name.into(),
            unit: "each".into(),
            description: None,
            metadata: None,
        })
        .unwrap()
        .id
}
fn receive(c: &rusqlite::Connection, a: ActorId, i: ItemId, l: LocationId, q: u32) -> OperationId {
    SqliteInventoryService::new(c)
        .receive(ReceiveStock {
            actor_id: a,
            item_id: i,
            destination_id: l,
            quantity: q,
            reason: None,
            source: OperationSource::Tui,
            idempotency_key: Some(format!("receive-{i}")),
        })
        .unwrap()
        .operation_id
}
fn empty(c: &rusqlite::Connection, a: ActorId, i: ItemId, l: LocationId) {
    receive(c, a, i, l, 5);
    SqliteInventoryService::new(c)
        .issue(IssueStock {
            actor_id: a,
            item_id: i,
            source_id: l,
            quantity: 5,
            reason: None,
            source: OperationSource::Tui,
            idempotency_key: None,
        })
        .unwrap();
    SqliteItemRepo::new(c).delete_as_admin(&i, &a).unwrap();
}
fn count(c: &rusqlite::Connection, table: &str) -> i64 {
    c.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
        .unwrap()
}
fn guard(c: &rusqlite::Connection) -> String {
    c.query_row(
        "SELECT sql FROM sqlite_master WHERE name='movement_no_delete'",
        [],
        |r| r.get(0),
    )
    .unwrap()
}

#[test]
fn purge_erases_item_and_dependencies_but_preserves_other_inventory() {
    let (c, a, i, l) = seed();
    let r = SqliteItemRepo::new(&c);
    SqliteThresholdRepo::new(&c)
        .set(&StockThreshold {
            item_id: i,
            stock_location_id: None,
            min_qty: 3,
        })
        .unwrap();
    SqliteIdentifierRepo::new(&c)
        .create(&NewIdentifier {
            kind: IdentifierKind::Barcode,
            value: "12345".into(),
            target: IdentifierTarget::Item(i),
        })
        .unwrap();
    empty(&c, a, i, l);
    let other = item(&c, "Keep me", "KEEP");
    let other_op = receive(&c, a, other, l, 7);
    c.execute("INSERT INTO automation_rules(id,name,enabled,event_type,item_sku,action_type,action_config,created_at,updated_at) VALUES('erased-rule','Erase',1,'StockReceived','ERASE','notify','{}','now','now')",[]).unwrap();
    c.execute("INSERT INTO automation_runs(id,rule_id,outbox_id,event_type,action_type,status,executed_at) SELECT 'erased-run','erased-rule',id,'StockReceived','notify','success','now' FROM event_outbox WHERE aggregate_id=?1 LIMIT 1",[i.to_string()]).unwrap();
    let original_guard = guard(&c);
    r.permanently_delete_as_admin(&i, &a, "DELETE PERMANENTLY")
        .unwrap();
    assert!(r.get_by_id(&i).unwrap().is_none());
    assert!(r.get_by_sku("ERASE").unwrap().is_none());
    for table in [
        "stock_movements",
        "stock_balances",
        "stock_thresholds",
        "identifiers",
    ] {
        let remaining: i64 = c
            .query_row(
                &format!("SELECT count(*) FROM {table} WHERE item_id=?1"),
                [i.to_string()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(remaining, 0, "{table}");
    }
    assert_eq!(count(&c, "operations"), 1);
    assert_eq!(count(&c, "automation_runs"), 0);
    assert_eq!(count(&c, "automation_rules"), 0);
    let leaked: i64 = c
        .query_row(
            "SELECT count(*) FROM event_outbox WHERE instr(payload,?1)>0 OR aggregate_id=?1",
            [i.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(leaked, 0);
    let old_audits:i64=c.query_row("SELECT count(*) FROM audit_events WHERE instr(COALESCE(details,''),?1)>0 AND action<>'item.permanently_deleted'",[i.to_string()],|r|r.get(0)).unwrap();
    assert_eq!(old_audits, 0);
    assert_eq!(
        SqliteInventoryService::new(&c)
            .get_balance(&other, &l)
            .unwrap()
            .unwrap()
            .qty,
        7
    );
    assert!(
        slim_db::repo::inventory::SqliteHistoryQuery::new(&c)
            .by_operation(&other_op)
            .unwrap()
            .is_some()
    );
    assert!(
        SqliteReconciliationService::new(&c)
            .check()
            .unwrap()
            .mismatches
            .is_empty()
    );
    assert_eq!(guard(&c), original_guard);
    assert!(
        c.execute(
            "DELETE FROM stock_movements WHERE item_id=?1",
            [other.to_string()]
        )
        .is_err()
    );
    assert!(
        c.execute(
            "UPDATE stock_movements SET delta_qty=0 WHERE item_id=?1",
            [other.to_string()]
        )
        .is_err()
    );
    assert_eq!(count(&c, "pragma_foreign_key_check"), 0);
    assert!(r.restore_as_admin(&i, &a).is_err());
    assert!(
        r.create(&NewItem {
            sku: Some("ERASE".into()),
            name: "Replacement".into(),
            unit: "each".into(),
            description: None,
            metadata: None
        })
        .is_ok()
    );
}

#[test]
fn linked_observations_are_erased_but_incidental_text_references_are_preserved() {
    let (c, a, i, l) = seed();
    empty(&c, a, i, l);
    let other = item(&c, "Other", "OTHER");
    let other_operation = receive(&c, a, other, l, 2);
    c.execute("INSERT INTO devices(id,name,kind,created_at,updated_at) VALUES('sensor','Sensor','mock_sensor','now','now')",[]).unwrap();
    c.execute("INSERT INTO observations(id,device_id,observation_type,payload,processed_operation_id,created_at) SELECT 'erase-observation','sensor','test','{}',operation_id,'now' FROM stock_movements WHERE item_id=?1 LIMIT 1",[i.to_string()]).unwrap();
    c.execute("INSERT INTO observations(id,device_id,observation_type,payload,processed_operation_id,created_at) VALUES('keep-observation','sensor','test','{}',?1,'now')",[other_operation.to_string()]).unwrap();
    let payload =
        serde_json::json!({"item_id":other.to_string(),"reason":format!("Reference to {i}")})
            .to_string();
    c.execute("INSERT INTO event_outbox(id,event_type,aggregate_id,payload,created_at) VALUES('keep-event','StockReceived',?1,?2,'now')",rusqlite::params![other.to_string(),payload]).unwrap();
    c.execute("INSERT INTO audit_events(id,actor_id,action,details,created_at) VALUES('keep-audit',?1,'stock.changed',?2,'now')",rusqlite::params![a.to_string(),payload]).unwrap();
    SqliteItemRepo::new(&c)
        .permanently_delete_as_admin(&i, &a, "DELETE PERMANENTLY")
        .unwrap();
    assert_eq!(count(&c, "observations"), 1);
    assert_eq!(
        c.query_row("SELECT id FROM observations", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "keep-observation"
    );
    assert_eq!(
        c.query_row(
            "SELECT count(*) FROM event_outbox WHERE id='keep-event'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(
        c.query_row(
            "SELECT count(*) FROM audit_events WHERE id='keep-audit'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(count(&c, "pragma_foreign_key_check"), 0);
}

#[test]
fn purge_requires_current_active_admin_deleted_item_and_exact_confirmation() {
    let (c, a, i, _) = seed();
    let r = SqliteItemRepo::new(&c);
    assert!(
        r.permanently_delete_as_admin(&i, &a, "DELETE PERMANENTLY")
            .is_err()
    );
    r.delete_as_admin(&i, &a).unwrap();
    for role in [Role::Viewer, Role::Operator, Role::Manager] {
        let user = SqliteActorRepo::new(&c)
            .create(&NewActor::new(role.to_string()).with_role(role))
            .unwrap();
        assert!(
            r.permanently_delete_as_admin(&i, &user.id, "DELETE PERMANENTLY")
                .is_err()
        );
    }
    for confirmation in ["", "DELETE", "delete permanently"] {
        assert!(r.permanently_delete_as_admin(&i, &a, confirmation).is_err());
    }
    c.execute("UPDATE actors SET active=0 WHERE id=?1", [a.to_string()])
        .unwrap();
    assert!(
        r.permanently_delete_as_admin(&i, &a, "DELETE PERMANENTLY")
            .is_err()
    );
    assert!(r.get_by_id(&i).unwrap().is_some());
}

#[test]
fn purge_rejects_nonzero_cache_or_ledger_even_for_archived_items() {
    let (c, a, i, l) = seed();
    let r = SqliteItemRepo::new(&c);
    receive(&c, a, i, l, 5);
    c.execute(
        "UPDATE items SET archived_at='now' WHERE id=?1",
        [i.to_string()],
    )
    .unwrap();
    assert!(
        r.permanently_delete_as_admin(&i, &a, "DELETE PERMANENTLY")
            .is_err()
    );
    c.execute(
        "UPDATE stock_balances SET qty=0 WHERE item_id=?1",
        [i.to_string()],
    )
    .unwrap();
    assert!(
        r.permanently_delete_as_admin(&i, &a, "DELETE PERMANENTLY")
            .is_err()
    );
    assert_eq!(count(&c, "stock_movements"), 1);
}

#[test]
fn failed_purge_rolls_back_history_dependencies_and_ledger_guard() {
    for trigger in [
        "CREATE TRIGGER fail_purge BEFORE INSERT ON audit_events WHEN NEW.action='item.permanently_deleted' BEGIN SELECT RAISE(ABORT,'audit failed'); END;",
        "CREATE TRIGGER fail_purge BEFORE DELETE ON items BEGIN SELECT RAISE(ABORT,'item delete failed'); END;",
    ] {
        let (c, a, i, l) = seed();
        empty(&c, a, i, l);
        let r = SqliteItemRepo::new(&c);
        let original_guard = guard(&c);
        let audit_count = count(&c, "audit_events");
        let outbox_count = count(&c, "event_outbox");
        c.execute_batch(trigger).unwrap();
        assert!(
            r.permanently_delete_as_admin(&i, &a, "DELETE PERMANENTLY")
                .is_err()
        );
        assert!(r.get_by_id(&i).unwrap().is_some());
        assert_eq!(count(&c, "stock_movements"), 2);
        assert_eq!(count(&c, "operations"), 2);
        assert_eq!(count(&c, "audit_events"), audit_count);
        assert_eq!(count(&c, "event_outbox"), outbox_count);
        assert_eq!(guard(&c), original_guard);
        assert!(
            c.execute(
                "DELETE FROM stock_movements WHERE item_id=?1",
                [i.to_string()]
            )
            .is_err()
        );
        c.execute_batch("DROP TRIGGER fail_purge;").unwrap();
        r.permanently_delete_as_admin(&i, &a, "DELETE PERMANENTLY")
            .unwrap();
    }
}

#[test]
fn imported_shared_operations_are_blocked_without_losing_other_items_history() {
    let (c, a, i, l) = seed();
    empty(&c, a, i, l);
    let other = item(&c, "Other", "OTHER");
    c.execute("INSERT INTO stock_movements(id,operation_id,item_id,location_id,delta_qty,created_at) SELECT 'shared',operation_id,?1,location_id,0,created_at FROM stock_movements WHERE item_id=?2 LIMIT 1",rusqlite::params![other.to_string(),i.to_string()]).unwrap();
    assert!(
        SqliteItemRepo::new(&c)
            .permanently_delete_as_admin(&i, &a, "DELETE PERMANENTLY")
            .unwrap_err()
            .to_string()
            .contains("shares")
    );
    assert_eq!(count(&c, "stock_movements"), 3);
}
