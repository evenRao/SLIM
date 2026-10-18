use serde_json::{Value, json};
use slim_core::{
    model::{actor::*, gridfinity::*, item::*, operation::*},
    service::*,
};
use slim_db::{
    connection::open_memory,
    migrate::run_migrations,
    repo::{
        actor::SqliteActorRepo,
        auth::SqliteAuthService,
        catalog::{CatalogKind as Kind, KINDS, SqliteCatalogService},
        grid::SqliteGridRepo,
        inventory::{SqliteHistoryQuery, SqliteInventoryService, SqliteReconciliationService},
        item::SqliteItemRepo,
    },
};
fn seed() -> (rusqlite::Connection, ActorId, String, String) {
    let c = open_memory().unwrap();
    run_migrations(&c).unwrap();
    let a = SqliteActorRepo::new(&c)
        .create(&NewActor::new("Owner").with_role(Role::Admin))
        .unwrap();
    let storage = SqliteCatalogService::new(&c)
        .save(
            &a.id,
            Kind::Storage,
            None,
            &json!({"name":"Workshop","kind":"storage"}),
        )
        .unwrap();
    let item = SqliteItemRepo::new(&c)
        .create(&NewItem {
            sku: Some("PART".into()),
            name: "Part".into(),
            unit: "each".into(),
            description: None,
            metadata: None,
        })
        .unwrap();
    (c, a.id, storage.id, item.id.to_string())
}
fn input(kind: Kind, storage: &str, item: &str) -> Value {
    match kind {
        Kind::Storage => {
            json!({"name":"Custom toolbox","kind":"custom","type_label":"Portable toolbox","notes":"Blue lid","width_mm":"400","depth_mm":"250","height_mm":"200"})
        }
        Kind::Base => json!({"name":"BASE","location":storage,"cols":"4","rows":"4"}),
        Kind::Bin => {
            json!({"name":"BIN","parent":storage,"width":"1","depth":"1","height":"3","compartments":"1"})
        }
        Kind::Rule => {
            json!({"name":"Low stock","event":"StockBelowThreshold","action":"notify","message":"Restock","sku":"PART"})
        }
        Kind::Device => json!({"name":"Scanner","kind":"mock_sensor","location":storage}),
        Kind::Barcode => json!({"name":"1234567","kind":"barcode","item":item}),
    }
}

#[test]
fn every_catalog_kind_supports_edit_delete_restore_and_permanent_delete() {
    for kind in KINDS {
        let (c, a, storage, item) = seed();
        let service = SqliteCatalogService::new(&c);
        let mut data = input(kind, &storage, &item);
        let record = service.save(&a, kind, None, &data).unwrap();
        data["name"] = json!("Renamed");
        let updated = service.save(&a, kind, Some(&record.id), &data).unwrap();
        assert_eq!(record.id, updated.id);
        assert_eq!(updated.name, "Renamed");
        assert!(
            service
                .lifecycle(&a, kind, &record.id, "purge", "DELETE PERMANENTLY")
                .is_err()
        );
        assert!(
            service
                .lifecycle(&a, kind, &record.id, "delete", "")
                .is_err()
        );
        service
            .lifecycle(&a, kind, &record.id, "delete", "DELETE")
            .unwrap();
        assert!(
            service
                .list(&a, kind, true)
                .unwrap()
                .iter()
                .any(|r| r.id == record.id)
        );
        assert!(service.save(&a, kind, Some(&record.id), &data).is_err());
        service
            .lifecycle(&a, kind, &record.id, "restore", "RESTORE")
            .unwrap();
        service
            .lifecycle(&a, kind, &record.id, "delete", "DELETE")
            .unwrap();
        service
            .lifecycle(&a, kind, &record.id, "purge", "DELETE PERMANENTLY")
            .unwrap();
        assert!(service.get(kind, &record.id).is_err());
        assert!(service.list(&a, kind, true).unwrap().is_empty());
        assert_eq!(
            c.query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |r| r
                .get::<_, i64>(
                0
            ))
            .unwrap(),
            0
        );
    }
}

#[test]
fn nonadmins_cannot_delete_restore_purge_and_revoked_permissions_are_fresh() {
    let (c, a, storage, item) = seed();
    let service = SqliteCatalogService::new(&c);
    for kind in KINDS {
        let record = service
            .save(&a, kind, None, &input(kind, &storage, &item))
            .unwrap();
        for role in [Role::Viewer, Role::Operator, Role::Manager] {
            let user = SqliteActorRepo::new(&c)
                .create(&NewActor::new(format!("{}-{role}", kind.key())).with_role(role))
                .unwrap();
            for (action, confirmation) in [
                ("delete", "DELETE"),
                ("restore", "RESTORE"),
                ("purge", "DELETE PERMANENTLY"),
            ] {
                assert!(
                    service
                        .lifecycle(&user.id, kind, &record.id, action, confirmation)
                        .is_err()
                );
            }
            assert!(service.list(&user.id, kind, true).is_err());
            if role != Role::Manager || kind == Kind::Rule {
                assert!(
                    service
                        .save(
                            &user.id,
                            kind,
                            Some(&record.id),
                            &input(kind, &storage, &item)
                        )
                        .is_err()
                );
            }
        }
    }
    c.execute(
        "UPDATE actors SET role='manager' WHERE id=?1",
        [a.to_string()],
    )
    .unwrap();
    assert!(
        service
            .lifecycle(&a, Kind::Storage, &storage, "delete", "DELETE")
            .is_err()
    );
}

#[test]
fn custom_storage_holds_stock_without_gridfinity_and_rejects_cycles_and_invalid_sizes() {
    let (c, a, storage, item) = seed();
    let service = SqliteCatalogService::new(&c);
    let data = json!({"name":"Travel case","kind":"toolbox","parent":storage,"notes":"Top shelf","width_mm":"350"});
    let record = service.save(&a, Kind::Storage, None, &data).unwrap();
    assert_eq!(record.data["kind"], "custom");
    assert_eq!(record.data["type_label"], "toolbox");
    assert_eq!(record.data["width_mm"], 350.0);
    let mut invalid = data.clone();
    invalid["width_mm"] = json!("-5");
    invalid["name"] = json!("Broken");
    assert!(
        service
            .save(&a, Kind::Storage, Some(&record.id), &invalid)
            .is_err()
    );
    assert_eq!(
        service.get(Kind::Storage, &record.id).unwrap().name,
        "Travel case"
    );
    let mut parent = service.get(Kind::Storage, &storage).unwrap().data;
    parent["parent"] = json!(record.id);
    assert!(
        service
            .save(&a, Kind::Storage, Some(&storage), &parent)
            .is_err()
    );
    SqliteInventoryService::new(&c)
        .receive(ReceiveStock {
            actor_id: a,
            item_id: item.parse().unwrap(),
            destination_id: record.id.parse().unwrap(),
            quantity: 3,
            source: OperationSource::Tui,
            reason: None,
            idempotency_key: None,
        })
        .unwrap();
    assert!(
        service
            .lifecycle(&a, Kind::Storage, &record.id, "delete", "DELETE")
            .is_err()
    );
    assert!(SqliteGridRepo::new(&c).list_bases(None).unwrap().is_empty());
    assert_eq!(
        SqliteInventoryService::new(&c)
            .get_balance(&item.parse().unwrap(), &record.id.parse().unwrap())
            .unwrap()
            .unwrap()
            .qty,
        3
    );
}

#[test]
fn placed_grid_geometry_is_protected_and_archived_bins_cannot_receive_stock() {
    let (c, a, storage, item) = seed();
    let service = SqliteCatalogService::new(&c);
    let base = service
        .save(&a, Kind::Base, None, &input(Kind::Base, &storage, &item))
        .unwrap();
    let bin = service
        .save(&a, Kind::Bin, None, &input(Kind::Bin, &storage, &item))
        .unwrap();
    let grid = SqliteGridRepo::new(&c);
    grid.place_bin(&PlaceBinCmd {
        bin_id: bin.id.parse().unwrap(),
        base_id: base.id.parse().unwrap(),
        x: 0,
        y: 0,
        rotation: Rotation::Rot0,
    })
    .unwrap();
    for (kind, id) in [(Kind::Base, &base.id), (Kind::Bin, &bin.id)] {
        let mut geometry = input(kind, &storage, &item);
        geometry[if kind == Kind::Base { "cols" } else { "width" }] = json!("2");
        assert!(service.save(&a, kind, Some(id), &geometry).is_err());
        assert!(
            service
                .save(
                    &a,
                    kind,
                    Some(id),
                    &json!({"name":format!("Renamed {}",kind.key())})
                )
                .is_ok()
        );
        assert!(service.lifecycle(&a, kind, id, "delete", "DELETE").is_err());
    }
    grid.remove_bin(&bin.id.parse().unwrap()).unwrap();
    service
        .lifecycle(&a, Kind::Bin, &bin.id, "delete", "DELETE")
        .unwrap();
    let location = grid
        .get_bin_by_id(&bin.id.parse().unwrap())
        .unwrap()
        .unwrap()
        .location_id;
    assert!(
        SqliteInventoryService::new(&c)
            .receive(ReceiveStock {
                actor_id: a,
                item_id: item.parse().unwrap(),
                destination_id: location,
                quantity: 1,
                source: OperationSource::Tui,
                reason: None,
                idempotency_key: None
            })
            .is_err()
    );
    assert!(
        grid.place_bin(&PlaceBinCmd {
            bin_id: bin.id.parse().unwrap(),
            base_id: base.id.parse().unwrap(),
            x: 0,
            y: 0,
            rotation: Rotation::Rot0
        })
        .is_err()
    );
    service
        .lifecycle(&a, Kind::Bin, &bin.id, "restore", "RESTORE")
        .unwrap();
    assert_eq!(grid.list_bins().unwrap().len(), 1);
}

#[test]
fn dependent_storage_must_be_removed_in_order_and_history_cannot_be_orphaned() {
    let (c, a, storage, item) = seed();
    let service = SqliteCatalogService::new(&c);
    let child = service
        .save(
            &a,
            Kind::Storage,
            None,
            &json!({"name":"Box","kind":"custom","parent":storage}),
        )
        .unwrap();
    assert!(
        service
            .lifecycle(&a, Kind::Storage, &storage, "delete", "DELETE")
            .is_err()
    );
    service
        .lifecycle(&a, Kind::Storage, &child.id, "delete", "DELETE")
        .unwrap();
    service
        .lifecycle(&a, Kind::Storage, &storage, "delete", "DELETE")
        .unwrap();
    assert!(
        service
            .lifecycle(&a, Kind::Storage, &child.id, "restore", "RESTORE")
            .is_err()
    );
    assert!(
        service
            .lifecycle(&a, Kind::Storage, &storage, "purge", "DELETE PERMANENTLY")
            .is_err()
    );
    service
        .lifecycle(&a, Kind::Storage, &child.id, "purge", "DELETE PERMANENTLY")
        .unwrap();
    service
        .lifecycle(&a, Kind::Storage, &storage, "restore", "RESTORE")
        .unwrap();
    let inventory = SqliteInventoryService::new(&c);
    inventory
        .adjust(AdjustStock {
            actor_id: a,
            item_id: item.parse().unwrap(),
            location_id: storage.parse().unwrap(),
            new_quantity: 0,
            reason: "Zero test".into(),
            source: OperationSource::Tui,
            idempotency_key: None,
        })
        .unwrap();
    service
        .lifecycle(&a, Kind::Storage, &storage, "delete", "DELETE")
        .unwrap();
    assert!(
        service
            .lifecycle(&a, Kind::Storage, &storage, "purge", "DELETE PERMANENTLY")
            .is_err()
    );
    SqliteItemRepo::new(&c)
        .delete_as_admin(&item.parse().unwrap(), &a)
        .unwrap();
    SqliteItemRepo::new(&c)
        .permanently_delete_as_admin(&item.parse().unwrap(), &a, "DELETE PERMANENTLY")
        .unwrap();
    service
        .lifecycle(&a, Kind::Storage, &storage, "purge", "DELETE PERMANENTLY")
        .unwrap();
}

#[test]
fn lifecycle_and_editor_audit_failures_roll_back_every_kind() {
    for kind in KINDS {
        let (c, a, storage, item) = seed();
        let service = SqliteCatalogService::new(&c);
        let mut data = input(kind, &storage, &item);
        let record = service.save(&a, kind, None, &data).unwrap();
        c.execute_batch("CREATE TRIGGER fail_catalog_audit BEFORE INSERT ON audit_events WHEN NEW.action LIKE 'catalog.%' BEGIN SELECT RAISE(ABORT,'audit failed'); END;").unwrap();
        data["name"] = json!("Must roll back");
        assert!(service.save(&a, kind, Some(&record.id), &data).is_err());
        assert_eq!(service.get(kind, &record.id).unwrap().name, record.name);
        assert!(
            service
                .lifecycle(&a, kind, &record.id, "delete", "DELETE")
                .is_err()
        );
        assert!(!service.get(kind, &record.id).unwrap().deleted);
        c.execute_batch("DROP TRIGGER fail_catalog_audit;").unwrap();
        service
            .lifecycle(&a, kind, &record.id, "delete", "DELETE")
            .unwrap();
        c.execute_batch("CREATE TRIGGER fail_catalog_audit BEFORE INSERT ON audit_events BEGIN SELECT RAISE(ABORT,'audit failed'); END;").unwrap();
        assert!(
            service
                .lifecycle(&a, kind, &record.id, "purge", "DELETE PERMANENTLY")
                .is_err()
        );
        assert!(service.get(kind, &record.id).unwrap().deleted);
    }
}

#[test]
fn permanently_deleting_user_erases_credentials_and_anonymizes_existing_stock_activity() {
    let (c, a, storage, item) = seed();
    let actors = SqliteActorRepo::new(&c);
    let user = actors
        .create_as_admin(&a, &NewActor::new("Staff"), Some("test-pin"))
        .unwrap();
    let login = SqliteAuthService::new(&c)
        .login_with_pin("Staff", "test-pin")
        .unwrap();
    let receipt = SqliteInventoryService::new(&c)
        .receive(ReceiveStock {
            actor_id: user.id,
            item_id: item.parse().unwrap(),
            destination_id: storage.parse().unwrap(),
            quantity: 5,
            source: OperationSource::Tui,
            reason: None,
            idempotency_key: None,
        })
        .unwrap();
    assert!(
        actors
            .permanently_delete_user(&a, &user.id, "DELETE PERMANENTLY")
            .is_err()
    );
    actors
        .update_as_admin(
            &a,
            &user.id,
            &ActorUpdate {
                active: Some(false),
                ..Default::default()
            },
        )
        .unwrap();
    assert!(
        actors
            .permanently_delete_user(&a, &user.id, "DELETE")
            .is_err()
    );
    actors
        .permanently_delete_user(&a, &user.id, "DELETE PERMANENTLY")
        .unwrap();
    assert!(actors.get_by_id(&user.id).unwrap().is_none());
    assert!(actors.get_pin_hash(&user.id).unwrap().is_none());
    assert_eq!(actors.list().unwrap().len(), 1);
    assert!(
        SqliteAuthService::new(&c)
            .validate_session(&login.token)
            .is_err()
    );
    let history = SqliteHistoryQuery::new(&c)
        .by_operation(&receipt.operation_id)
        .unwrap()
        .unwrap();
    assert_ne!(history.operation.actor_id, user.id);
    assert_eq!(history.movements.len(), 1);
    assert!(
        SqliteReconciliationService::new(&c)
            .check()
            .unwrap()
            .mismatches
            .is_empty()
    );
    assert_eq!(
        c.query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |r| r
            .get::<_, i64>(
            0
        ))
        .unwrap(),
        0
    );
}

#[test]
fn permanent_user_delete_rolls_back_if_audit_fails() {
    let (c, a, _, _) = seed();
    let actors = SqliteActorRepo::new(&c);
    let user = actors
        .create_as_admin(&a, &NewActor::new("Staff"), Some("test-pin"))
        .unwrap();
    actors
        .update_as_admin(
            &a,
            &user.id,
            &ActorUpdate {
                active: Some(false),
                ..Default::default()
            },
        )
        .unwrap();
    c.execute_batch("CREATE TRIGGER fail_user_purge BEFORE INSERT ON audit_events WHEN NEW.action='actor.permanently_deleted' BEGIN SELECT RAISE(ABORT,'audit failed'); END;").unwrap();
    assert!(
        actors
            .permanently_delete_user(&a, &user.id, "DELETE PERMANENTLY")
            .is_err()
    );
    assert!(actors.get_by_id(&user.id).unwrap().is_some());
    assert!(actors.get_pin_hash(&user.id).unwrap().is_some());
}
