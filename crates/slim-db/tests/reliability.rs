use rusqlite::Connection;
use slim_core::{
    model::{
        actor::{NewActor, Role},
        item::NewItem,
        location::{LocationKind, NewLocation},
        operation::*,
        stock::StockThreshold,
    },
    service::*,
};
use slim_db::{
    connection::{open_connection, open_memory},
    migrate::run_migrations,
    repo::{
        actor::SqliteActorRepo,
        inventory::{SqliteInventoryService, SqliteReconciliationService, SqliteThresholdRepo},
        item::SqliteItemRepo,
        location::SqliteLocationRepo,
    },
};

fn seed(
    c: &Connection,
) -> (
    slim_core::model::actor::ActorId,
    slim_core::model::item::ItemId,
    slim_core::model::location::LocationId,
) {
    let a = SqliteActorRepo::new(c)
        .create(&NewActor::new("Owner").with_role(Role::Admin))
        .unwrap();
    let i = SqliteItemRepo::new(c)
        .create(&NewItem {
            sku: None,
            name: "Parts".into(),
            unit: "each".into(),
            description: None,
            metadata: None,
        })
        .unwrap();
    let l = SqliteLocationRepo::new(c)
        .create(&NewLocation {
            parent_id: None,
            name: "Storage".into(),
            kind: LocationKind::Storage,
        })
        .unwrap();
    (a.id, i.id, l.id)
}

#[test]
fn only_admin_can_delete_and_restore_with_audit_history() {
    let c = open_memory().unwrap();
    run_migrations(&c).unwrap();
    let (admin, item, _) = seed(&c);
    let repo = SqliteItemRepo::new(&c);
    for role in [Role::Viewer, Role::Operator, Role::Manager] {
        let actor = SqliteActorRepo::new(&c)
            .create(&NewActor::new(role.to_string()).with_role(role))
            .unwrap();
        assert!(repo.delete_as_admin(&item, &actor.id).is_err());
        assert!(repo.restore_as_admin(&item, &actor.id).is_err());
    }
    repo.delete_as_admin(&item, &admin).unwrap();
    assert!(repo.list(&Default::default()).unwrap().is_empty());
    assert!(
        repo.get_by_id(&item)
            .unwrap()
            .unwrap()
            .archived_at
            .is_some()
    );
    repo.restore_as_admin(&item, &admin).unwrap();
    assert!(
        repo.get_by_id(&item)
            .unwrap()
            .unwrap()
            .archived_at
            .is_none()
    );
    let events: i64 = c.query_row("SELECT count(*) FROM audit_events WHERE action IN ('item.deleted','item.restored') AND actor_id=?1", [admin.to_string()], |r| r.get(0)).unwrap();
    assert_eq!(events, 2);
}

#[test]
fn deleting_requires_zero_stock_and_preserves_the_movement_ledger() {
    let c = open_memory().unwrap();
    run_migrations(&c).unwrap();
    let (admin, item, location) = seed(&c);
    let inventory = SqliteInventoryService::new(&c);
    inventory
        .receive(ReceiveStock {
            actor_id: admin,
            item_id: item,
            destination_id: location,
            quantity: 5,
            source: OperationSource::Tui,
            reason: None,
            idempotency_key: None,
        })
        .unwrap();
    let repo = SqliteItemRepo::new(&c);
    assert!(repo.delete_as_admin(&item, &admin).is_err());
    inventory
        .issue(IssueStock {
            actor_id: admin,
            item_id: item,
            source_id: location,
            quantity: 5,
            source: OperationSource::Tui,
            reason: Some("Removed from inventory".into()),
            idempotency_key: None,
        })
        .unwrap();
    repo.delete_as_admin(&item, &admin).unwrap();
    let ledger: (i64, i64) = c
        .query_row(
            "SELECT count(*), sum(delta_qty) FROM stock_movements WHERE item_id=?1",
            [item.to_string()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(ledger, (2, 0));
    assert!(
        SqliteReconciliationService::new(&c)
            .check()
            .unwrap()
            .mismatches
            .is_empty()
    );
}

#[test]
fn delete_rolls_back_when_its_audit_cannot_be_written() {
    let c = open_memory().unwrap();
    run_migrations(&c).unwrap();
    let (admin, item, _) = seed(&c);
    c.execute_batch("CREATE TRIGGER reject_delete_audit BEFORE INSERT ON audit_events WHEN NEW.action='item.deleted' BEGIN SELECT RAISE(ABORT,'audit unavailable'); END;").unwrap();
    let repo = SqliteItemRepo::new(&c);
    assert!(repo.delete_as_admin(&item, &admin).is_err());
    assert!(
        repo.get_by_id(&item)
            .unwrap()
            .unwrap()
            .archived_at
            .is_none()
    );
}

#[test]
fn inactive_admin_and_repeated_delete_are_rejected() {
    let c = open_memory().unwrap();
    run_migrations(&c).unwrap();
    let (admin, item, _) = seed(&c);
    let repo = SqliteItemRepo::new(&c);
    c.execute(
        "UPDATE actors SET active=0 WHERE id=?1",
        [admin.to_string()],
    )
    .unwrap();
    assert!(repo.delete_as_admin(&item, &admin).is_err());
    c.execute(
        "UPDATE actors SET active=1 WHERE id=?1",
        [admin.to_string()],
    )
    .unwrap();
    repo.delete_as_admin(&item, &admin).unwrap();
    assert!(repo.delete_as_admin(&item, &admin).is_err());
    let events: i64 = c
        .query_row(
            "SELECT count(*) FROM audit_events WHERE action='item.deleted'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(events, 1);
}
#[test]
fn migration_upgrades_existing_data_and_keeps_latest_global_threshold() {
    let c = open_memory().unwrap();
    for sql in [
        include_str!("../migrations/001_initial.sql"),
        include_str!("../migrations/002_gridfinity.sql"),
        include_str!("../migrations/003_auth_and_sessions.sql"),
        include_str!("../migrations/004_automation.sql"),
        include_str!("../migrations/005_devices.sql"),
        include_str!("../migrations/006_led_and_docks.sql"),
    ] {
        c.execute_batch(sql).unwrap();
    }
    c.execute_batch("CREATE TABLE _schema_versions(version INTEGER PRIMARY KEY,name TEXT,applied_at TEXT); INSERT INTO _schema_versions VALUES(6,'previous','2026-01-01T00:00:00Z');").unwrap();
    let (_, item, location) = seed(&c);
    for (id, min) in [("old", 10), ("latest", 3)] {
        c.execute(
            "INSERT INTO stock_thresholds VALUES(?1,?2,NULL,?3)",
            rusqlite::params![id, item.to_string(), min],
        )
        .unwrap();
    }
    assert_eq!(
        run_migrations(&c).unwrap(),
        slim_db::migrate::latest_version() - 6
    );
    let t = SqliteThresholdRepo::new(&c).get_for_item(&item).unwrap();
    assert_eq!(t.len(), 1);
    assert_eq!(t[0].min_qty, 3);
    assert!(
        SqliteLocationRepo::new(&c)
            .get_by_id(&location)
            .unwrap()
            .is_some()
    );
    assert_eq!(run_migrations(&c).unwrap(), 0);
}
#[test]
fn independent_connections_cannot_over_issue_or_drift_balances() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("inventory.sqlite3");
    let c = open_connection(&path).unwrap();
    run_migrations(&c).unwrap();
    let (a, i, l) = seed(&c);
    SqliteInventoryService::new(&c)
        .receive(ReceiveStock {
            actor_id: a,
            item_id: i,
            destination_id: l,
            quantity: 20,
            source: OperationSource::Cli,
            reason: None,
            idempotency_key: None,
        })
        .unwrap();
    let workers = (0..8)
        .map(|_| {
            let p = path.clone();
            std::thread::spawn(move || {
                let conn = open_connection(&p).unwrap();
                SqliteInventoryService::new(&conn)
                    .issue(IssueStock {
                        actor_id: a,
                        item_id: i,
                        source_id: l,
                        quantity: 5,
                        source: OperationSource::Cli,
                        reason: None,
                        idempotency_key: None,
                    })
                    .is_ok()
            })
        })
        .collect::<Vec<_>>();
    let successful = workers
        .into_iter()
        .map(|w| usize::from(w.join().unwrap()))
        .sum::<usize>();
    assert_eq!(successful, 4);
    assert_eq!(
        SqliteInventoryService::new(&c)
            .get_balance(&i, &l)
            .unwrap()
            .unwrap()
            .qty,
        0
    );
    assert!(
        SqliteReconciliationService::new(&c)
            .check()
            .unwrap()
            .mismatches
            .is_empty()
    );
}
#[test]
fn retries_return_original_receipt_and_zero_balance_does_not_realert() {
    let c = open_memory().unwrap();
    run_migrations(&c).unwrap();
    let (a, i, l) = seed(&c);
    let svc = SqliteInventoryService::new(&c);
    SqliteThresholdRepo::new(&c)
        .set(&StockThreshold {
            item_id: i,
            stock_location_id: None,
            min_qty: 10,
        })
        .unwrap();
    let cmd = ReceiveStock {
        actor_id: a,
        item_id: i,
        destination_id: l,
        quantity: 2,
        source: OperationSource::Cli,
        reason: None,
        idempotency_key: Some("retry".into()),
    };
    svc.receive(cmd.clone()).unwrap();
    svc.issue(IssueStock {
        actor_id: a,
        item_id: i,
        source_id: l,
        quantity: 2,
        source: OperationSource::Cli,
        reason: None,
        idempotency_key: None,
    })
    .unwrap();
    let receipt = svc.receive(cmd).unwrap();
    assert_eq!(receipt.resulting_balances[0].qty, 2);
    svc.receive(ReceiveStock {
        actor_id: a,
        item_id: i,
        destination_id: l,
        quantity: 1,
        source: OperationSource::Cli,
        reason: None,
        idempotency_key: None,
    })
    .unwrap();
    let events: i64 = c
        .query_row(
            "SELECT COUNT(*) FROM event_outbox WHERE event_type='StockBelowThreshold'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(events, 1);
}

#[test]
fn webhook_posts_and_duplicate_dispatch_is_suppressed() {
    use slim_core::model::automation::*;
    use slim_db::repo::automation::{AutomationEngine, AutomationOptions, SqliteAutomationRepo};
    use std::io::{BufRead, BufReader, Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let responder = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .unwrap();
        // TCP can split headers and body across reads. Consume the complete
        // request before closing the connection so the client can finish writing.
        let mut request = String::new();
        let mut reader = BufReader::new(&mut stream);
        loop {
            let mut line = String::new();
            assert!(reader.read_line(&mut line).unwrap() > 0);
            request.push_str(&line);
            if line == "\r\n" {
                break;
            }
        }
        let length: usize = request
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("Content-Length")
                    .then(|| value.trim().parse::<usize>().unwrap())
            })
            .expect("Webhook request includes its body length");
        let mut body = vec![0; length];
        reader.read_exact(&mut body).unwrap();
        drop(reader);
        assert!(serde_json::from_slice::<serde_json::Value>(&body).is_ok());
        request.push_str(&String::from_utf8(body).unwrap());
        let request = request.to_lowercase();
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nOK")
            .unwrap();
        request
    });
    let c = open_memory().unwrap();
    run_migrations(&c).unwrap();
    let (a, i, l) = seed(&c);
    SqliteInventoryService::new(&c)
        .receive(ReceiveStock {
            actor_id: a,
            item_id: i,
            destination_id: l,
            quantity: 1,
            source: OperationSource::Cli,
            reason: None,
            idempotency_key: None,
        })
        .unwrap();
    let rule = AutomationRule {
        id: "delivery".into(),
        name: None,
        enabled: true,
        event: "StockReceived".into(),
        item_sku: None,
        location_name: None,
        condition: None,
        action: RuleAction::Webhook {
            url: format!("http://{address}/inventory"),
            payload: None,
        },
    };
    let engine = AutomationEngine::new(&c);
    let runs = engine
        .process_pending(
            std::slice::from_ref(&rule),
            &AutomationOptions::default(),
            10,
        )
        .unwrap();
    assert_eq!(runs[0].status, AutomationRunStatus::Success);
    let request = responder.join().unwrap();
    assert!(request.contains("post /inventory"));
    assert!(request.contains("idempotency-key:"));
    assert!(
        engine
            .process_pending(&[rule], &AutomationOptions::default(), 10)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        SqliteAutomationRepo::new(&c).list_runs(10).unwrap().len(),
        1
    );
}
