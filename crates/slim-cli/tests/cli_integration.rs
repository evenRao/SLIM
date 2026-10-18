//! Comprehensive integration tests for SLIM CLI covering Milestones 1 through 6.

use std::path::PathBuf;
use std::process::Command;
use tempfile::NamedTempFile;

fn slim_bin() -> PathBuf {
    let mut path = std::env::current_exe().unwrap();
    path.pop(); // pop test binary name
    path.pop(); // pop 'deps'
    path.push(format!("slim{}", std::env::consts::EXE_SUFFIX));
    path
}

fn run_slim(db_path: &str, args: &[&str]) -> (bool, String, String) {
    let mut cmd = Command::new(slim_bin());
    let config_path = format!("{}.test-config.toml", db_path);
    std::fs::write(&config_path, "").unwrap();
    cmd.arg("--config").arg(&config_path);
    cmd.arg("--db").arg(db_path);
    cmd.env("SLIM_SESSION_FILE", format!("{}.session", db_path));
    for arg in args {
        cmd.arg(arg);
    }

    let output = cmd.output().expect("failed to execute slim binary");
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    (output.status.success(), stdout, stderr)
}

fn setup_db() -> (NamedTempFile, String) {
    let temp_file = NamedTempFile::new().unwrap();
    let db_path = temp_file.path().to_str().unwrap().to_string();
    let (ok, out, _) = run_slim(&db_path, &["init"]);
    assert!(ok, "init failed: {}", out);
    let user = std::env::var("USERNAME")
        .or_else(|_| std::env::var("USER"))
        .unwrap_or_else(|_| "test-owner".into());
    let (ok, _, error) = run_slim(
        &db_path,
        &[
            "actor",
            "add",
            "Test administrator",
            "--role",
            "admin",
            "--os-user",
            &user,
        ],
    );
    assert!(ok, "fixture administrator: {error}");
    (temp_file, db_path)
}

#[test]
fn catalog_cli_manages_custom_storage_and_admin_only_lifecycle() {
    let (_file, db) = setup_db();
    let (ok, out, error) = run_slim(
        &db,
        &[
            "catalog",
            "storage",
            "add",
            "--field",
            "name=Toolbox",
            "--field",
            "kind=custom",
            "--field",
            "type_label=Rolling chest",
        ],
    );
    assert!(ok, "{error}");
    let record: serde_json::Value = serde_json::from_str(&out).unwrap();
    let id = record["id"].as_str().unwrap();
    assert!(
        run_slim(
            &db,
            &[
                "catalog",
                "storage",
                "edit",
                "--id",
                id,
                "--field",
                "name=Renamed",
                "--field",
                "kind=custom"
            ]
        )
        .0
    );
    assert!(run_slim(&db, &["actor", "add", "Manager", "--role", "manager"]).0);
    assert!(
        !run_slim(
            &db,
            &[
                "--actor",
                "Manager",
                "catalog",
                "storage",
                "delete",
                "--id",
                id,
                "--confirm",
                "DELETE"
            ]
        )
        .0
    );
    assert!(
        run_slim(
            &db,
            &[
                "catalog",
                "storage",
                "delete",
                "--id",
                id,
                "--confirm",
                "DELETE"
            ]
        )
        .0
    );
    assert!(
        run_slim(
            &db,
            &[
                "catalog",
                "storage",
                "restore",
                "--id",
                id,
                "--confirm",
                "RESTORE"
            ]
        )
        .0
    );
    assert!(
        run_slim(
            &db,
            &[
                "catalog",
                "storage",
                "delete",
                "--id",
                id,
                "--confirm",
                "DELETE"
            ]
        )
        .0
    );
    assert!(
        run_slim(
            &db,
            &[
                "catalog",
                "storage",
                "purge",
                "--id",
                id,
                "--confirm",
                "DELETE PERMANENTLY"
            ]
        )
        .0
    );
    let c = rusqlite::Connection::open(&db).unwrap();
    assert_eq!(
        c.query_row("SELECT count(*) FROM locations WHERE id=?1", [id], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn permanent_delete_is_admin_only_and_requires_explicit_confirmation() {
    let (_file, db) = setup_db();
    assert!(
        run_slim(
            &db,
            &["item", "add", "--sku", "PURGE", "--name", "Erase me"]
        )
        .0
    );
    assert!(
        !run_slim(
            &db,
            &["item", "purge", "PURGE", "--confirm", "DELETE PERMANENTLY"]
        )
        .0
    );
    assert!(run_slim(&db, &["item", "delete", "PURGE"]).0);
    assert!(run_slim(&db, &["actor", "add", "Manager", "--role", "manager"]).0);
    assert!(
        !run_slim(
            &db,
            &[
                "--actor",
                "Manager",
                "item",
                "purge",
                "PURGE",
                "--confirm",
                "DELETE PERMANENTLY"
            ]
        )
        .0
    );
    assert!(!run_slim(&db, &["item", "purge", "PURGE", "--confirm", "DELETE"]).0);
    assert!(
        run_slim(
            &db,
            &["item", "purge", "PURGE", "--confirm", "DELETE PERMANENTLY"]
        )
        .0
    );
    let connection = rusqlite::Connection::open(&db).unwrap();
    assert_eq!(
        connection
            .query_row("SELECT count(*) FROM items WHERE sku='PURGE'", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert!(!run_slim(&db, &["item", "restore", "PURGE"]).0);
}

#[test]
fn account_edit_remove_restore_and_pin_are_admin_only_in_cli() {
    let (_file, db) = setup_db();
    assert!(run_slim(&db, &["actor", "add", "Staff", "--role", "manager"]).0);
    for args in [
        vec!["actor", "edit", "Staff", "--new-name", "Changed"],
        vec!["actor", "remove", "Staff"],
        vec!["actor", "set-pin", "Staff", "--pin", "4321"],
    ] {
        let mut denied = vec!["--actor", "Staff"];
        denied.extend_from_slice(&args);
        assert!(!run_slim(&db, &denied).0);
    }
    assert!(run_slim(&db, &["actor", "edit", "Staff", "--new-name", "Changed"]).0);
    assert!(run_slim(&db, &["actor", "set-pin", "Changed", "--pin", "4321"]).0);
    assert!(run_slim(&db, &["actor", "remove", "Changed"]).0);
    assert!(!run_slim(&db, &["session", "login", "Changed", "--pin", "4321"]).0);
    assert!(run_slim(&db, &["actor", "restore", "Changed"]).0);
    assert!(run_slim(&db, &["session", "login", "Changed", "--pin", "4321"]).0);
    assert!(!run_slim(&db, &["actor", "set-pin", "Changed", "--pin", "5678"]).0);
}

#[test]
fn delete_and_restore_are_admin_only_in_cli() {
    let (_file, db) = setup_db();
    assert!(
        run_slim(
            &db,
            &["item", "add", "--sku", "TRASH", "--name", "Test item"]
        )
        .0
    );
    for role in ["viewer", "operator", "manager"] {
        assert!(run_slim(&db, &["actor", "add", role, "--role", role]).0);
        let (ok, _, error) = run_slim(&db, &["--actor", role, "item", "delete", "TRASH"]);
        assert!(!ok, "{role} deleted inventory");
        assert!(error.contains("admin"));
    }
    assert!(run_slim(&db, &["item", "delete", "TRASH"]).0);
    assert!(!run_slim(&db, &["--actor", "manager", "item", "restore", "TRASH"]).0);
    assert!(run_slim(&db, &["item", "restore", "TRASH"]).0);
}

#[test]
fn test_cli_init_creates_database() {
    let temp = NamedTempFile::new().unwrap();
    let path = temp.path().to_str().unwrap();
    let (ok, out, _) = run_slim(path, &["init"]);
    assert!(ok);
    assert!(out.contains("Initialized SLIM database"));
}

#[test]
fn test_cli_init_idempotent() {
    let (_temp, path) = setup_db();
    let (ok, out, _) = run_slim(&path, &["init"]);
    assert!(ok);
    assert!(out.contains("Schema is up to date"));
}

#[test]
fn test_cli_actor_add_and_list() {
    let (_temp, path) = setup_db();
    let (ok, out, _) = run_slim(&path, &["actor", "add", "Alice", "--os-user", "alice"]);
    assert!(ok);
    assert!(out.contains("Created actor 'Alice'"));

    let (ok, out, _) = run_slim(&path, &["actor", "list"]);
    assert!(ok);
    assert!(out.contains("Alice"));
    assert!(out.contains("alice"));
}

#[test]
fn test_cli_actor_duplicate_rejected() {
    let (_temp, path) = setup_db();
    let (ok, _, _) = run_slim(&path, &["actor", "add", "Bob"]);
    assert!(ok);

    let (ok, _, err) = run_slim(&path, &["actor", "add", "Bob"]);
    assert!(!ok);
    assert!(err.contains("already exists"));
}

#[test]
fn test_cli_item_add_and_show() {
    let (_temp, path) = setup_db();
    let (ok, out, _) = run_slim(
        &path,
        &[
            "item",
            "add",
            "--sku",
            "M3X10",
            "--name",
            "M3x10 Screw",
            "--unit",
            "each",
        ],
    );
    assert!(ok);
    assert!(out.contains("Created item 'M3x10 Screw'"));

    let (ok, out, _) = run_slim(&path, &["item", "show", "M3X10"]);
    assert!(ok);
    assert!(out.contains("M3x10 Screw"));
    assert!(out.contains("M3X10"));
    assert!(out.contains("Unit:        each"));
}

#[test]
fn test_cli_item_duplicate_sku_rejected() {
    let (_temp, path) = setup_db();
    let (ok, _, _) = run_slim(
        &path,
        &["item", "add", "--sku", "DUP-SKU", "--name", "First"],
    );
    assert!(ok);

    let (ok, _, err) = run_slim(
        &path,
        &["item", "add", "--sku", "DUP-SKU", "--name", "Second"],
    );
    assert!(!ok);
    assert!(err.contains("duplicate SKU"));
}

#[test]
fn test_cli_item_archive_and_list_filtered() {
    let (_temp, path) = setup_db();
    run_slim(
        &path,
        &["item", "add", "--sku", "ACTIVE", "--name", "Active Item"],
    );
    run_slim(
        &path,
        &[
            "item",
            "add",
            "--sku",
            "ARCHIVED",
            "--name",
            "Archived Item",
        ],
    );

    let (ok, _, _) = run_slim(&path, &["item", "archive", "ARCHIVED"]);
    assert!(ok);

    let (ok, out, _) = run_slim(&path, &["item", "list"]);
    assert!(ok);
    assert!(out.contains("ACTIVE"));
    assert!(!out.contains("ARCHIVED"));

    let (ok, out, _) = run_slim(&path, &["item", "list", "--all"]);
    assert!(ok);
    assert!(out.contains("ACTIVE"));
    assert!(out.contains("ARCHIVED"));
}

#[test]
fn test_cli_location_hierarchy() {
    let (_temp, path) = setup_db();
    let (ok, _, _) = run_slim(
        &path,
        &["location", "add", "--name", "Building A", "--kind", "room"],
    );
    assert!(ok);

    let (ok, _, _) = run_slim(
        &path,
        &[
            "location",
            "add",
            "--parent",
            "Building A",
            "--name",
            "Shelf 1",
            "--kind",
            "shelf",
        ],
    );
    assert!(ok);

    let (ok, out, _) = run_slim(&path, &["location", "list"]);
    assert!(ok);
    assert!(out.contains("Building A"));
    assert!(out.contains("Shelf 1"));
}

#[test]
fn test_cli_location_duplicate_sibling_rejected() {
    let (_temp, path) = setup_db();
    run_slim(
        &path,
        &["location", "add", "--name", "Workshop", "--kind", "room"],
    );

    let (ok, _, err) = run_slim(
        &path,
        &["location", "add", "--name", "Workshop", "--kind", "room"],
    );
    assert!(!ok);
    assert!(err.contains("already exists"));
}

#[test]
fn test_cli_stock_receive_updates_balance() {
    let (_temp, path) = setup_db();
    run_slim(&path, &["actor", "add", "Operator"]);
    run_slim(
        &path,
        &["item", "add", "--sku", "RES-10K", "--name", "10k Resistor"],
    );
    run_slim(
        &path,
        &["location", "add", "--name", "Bin 1", "--kind", "bin"],
    );

    let (ok, out, _) = run_slim(
        &path,
        &[
            "stock",
            "receive",
            "RES-10K",
            "--qty",
            "50",
            "--to",
            "Bin 1",
            "--reason",
            "Purchased",
        ],
    );
    assert!(ok);
    assert!(out.contains("Received 50"));
    assert!(out.contains("New balance: 50"));

    let (ok, out, _) = run_slim(&path, &["stock", "balance", "RES-10K"]);
    assert!(ok);
    assert!(out.contains("50 each"));
}

#[test]
fn test_cli_stock_receive_zero_rejected() {
    let (_temp, path) = setup_db();
    run_slim(&path, &["actor", "add", "Operator"]);
    run_slim(
        &path,
        &["item", "add", "--sku", "CAP-10U", "--name", "10uF Cap"],
    );
    run_slim(
        &path,
        &["location", "add", "--name", "Bin 2", "--kind", "bin"],
    );

    let (ok, _, err) = run_slim(
        &path,
        &["stock", "receive", "CAP-10U", "--qty", "0", "--to", "Bin 2"],
    );
    assert!(!ok);
    assert!(err.contains("greater than zero"));
}

#[test]
fn test_cli_stock_issue_decrements_balance() {
    let (_temp, path) = setup_db();
    run_slim(&path, &["actor", "add", "Operator"]);
    run_slim(
        &path,
        &["item", "add", "--sku", "IC-555", "--name", "555 Timer"],
    );
    run_slim(
        &path,
        &["location", "add", "--name", "Drawer A", "--kind", "drawer"],
    );

    run_slim(
        &path,
        &[
            "stock", "receive", "IC-555", "--qty", "20", "--to", "Drawer A",
        ],
    );

    let (ok, out, _) = run_slim(
        &path,
        &[
            "stock", "issue", "IC-555", "--qty", "5", "--from", "Drawer A", "--reason", "Project",
        ],
    );
    assert!(ok);
    assert!(out.contains("Issued 5"));
    assert!(out.contains("Remaining balance: 15"));
}

#[test]
fn test_cli_stock_issue_insufficient_rejected() {
    let (_temp, path) = setup_db();
    run_slim(&path, &["actor", "add", "Operator"]);
    run_slim(&path, &["item", "add", "--sku", "IC-MCU", "--name", "MCU"]);
    run_slim(
        &path,
        &["location", "add", "--name", "Drawer B", "--kind", "drawer"],
    );

    run_slim(
        &path,
        &[
            "stock", "receive", "IC-MCU", "--qty", "5", "--to", "Drawer B",
        ],
    );

    let (ok, _, err) = run_slim(
        &path,
        &[
            "stock", "issue", "IC-MCU", "--qty", "10", "--from", "Drawer B",
        ],
    );
    assert!(!ok);
    assert!(err.contains("insufficient stock") || err.contains("available 5"));
}

#[test]
fn test_cli_stock_transfer_conserves_total() {
    let (_temp, path) = setup_db();
    run_slim(&path, &["actor", "add", "Operator"]);
    run_slim(
        &path,
        &["item", "add", "--sku", "LED-RED", "--name", "Red LED"],
    );
    run_slim(
        &path,
        &["location", "add", "--name", "Tray 1", "--kind", "bin"],
    );
    run_slim(
        &path,
        &["location", "add", "--name", "Tray 2", "--kind", "bin"],
    );

    run_slim(
        &path,
        &[
            "stock", "receive", "LED-RED", "--qty", "100", "--to", "Tray 1",
        ],
    );
    let (ok, out, _) = run_slim(
        &path,
        &[
            "stock", "transfer", "LED-RED", "--qty", "30", "--from", "Tray 1", "--to", "Tray 2",
        ],
    );
    assert!(ok);
    assert!(out.contains("Transferred 30"));
    assert!(out.contains("Tray 1: 70"));
    assert!(out.contains("Tray 2: 30"));
}

#[test]
fn test_cli_stock_adjust_mandatory_reason() {
    let (_temp, path) = setup_db();
    run_slim(&path, &["actor", "add", "Operator"]);
    run_slim(
        &path,
        &["item", "add", "--sku", "NUT-M4", "--name", "M4 Nut"],
    );
    run_slim(
        &path,
        &["location", "add", "--name", "Bin 5", "--kind", "bin"],
    );

    run_slim(
        &path,
        &["stock", "receive", "NUT-M4", "--qty", "40", "--to", "Bin 5"],
    );

    // Operator cannot adjust (Manager or Admin required)
    let (ok_op, _, err_op) = run_slim(
        &path,
        &[
            "--actor",
            "Operator",
            "stock",
            "adjust",
            "NUT-M4",
            "--at",
            "Bin 5",
            "--set",
            "35",
            "--reason",
            "Physical inventory",
        ],
    );
    assert!(!ok_op);
    assert!(err_op.contains("does not have permission to adjust inventory"));

    // Promote actor to Manager
    let (ok_role, _, _) = run_slim(&path, &["actor", "set-role", "Operator", "manager"]);
    assert!(ok_role);

    let (ok, out, _) = run_slim(
        &path,
        &[
            "stock",
            "adjust",
            "NUT-M4",
            "--at",
            "Bin 5",
            "--set",
            "35",
            "--reason",
            "Physical inventory",
        ],
    );
    assert!(ok);
    assert!(out.contains("Adjusted 'M4 Nut' at 'Bin 5' to 35"));
    assert!(out.contains("delta: -5"));
}

#[test]
fn test_cli_stock_threshold_and_low() {
    let (_temp, path) = setup_db();
    run_slim(&path, &["actor", "add", "Operator"]);
    run_slim(
        &path,
        &["item", "add", "--sku", "SCREW-M2", "--name", "M2 Screw"],
    );
    run_slim(
        &path,
        &["location", "add", "--name", "Bin 8", "--kind", "bin"],
    );

    run_slim(&path, &["stock", "threshold", "SCREW-M2", "--min", "50"]);
    run_slim(
        &path,
        &[
            "stock", "receive", "SCREW-M2", "--qty", "20", "--to", "Bin 8",
        ],
    );

    let (ok, out, _) = run_slim(&path, &["stock", "low"]);
    assert!(ok);
    assert!(out.contains("SCREW-M2"));
    assert!(out.contains("20"));
    assert!(out.contains("50"));
}

#[test]
fn test_cli_identifier_add_and_resolve() {
    let (_temp, path) = setup_db();
    run_slim(
        &path,
        &["item", "add", "--sku", "BAR-01", "--name", "Barcode Part"],
    );

    let (ok, _, _) = run_slim(
        &path,
        &[
            "identifier",
            "add",
            "--kind",
            "barcode",
            "--value",
            "0123456789",
            "--item",
            "BAR-01",
        ],
    );
    assert!(ok);

    let (ok, out, _) = run_slim(
        &path,
        &[
            "identifier",
            "resolve",
            "--kind",
            "barcode",
            "--value",
            "0123456789",
        ],
    );
    assert!(ok);
    assert!(out.contains("Resolved to Item: Barcode Part"));
}

#[test]
fn test_cli_history_item_and_operation() {
    let (_temp, path) = setup_db();
    run_slim(&path, &["actor", "add", "Operator"]);
    run_slim(
        &path,
        &["item", "add", "--sku", "HIST-01", "--name", "History Item"],
    );
    run_slim(
        &path,
        &["location", "add", "--name", "Bin H", "--kind", "bin"],
    );

    run_slim(
        &path,
        &[
            "stock",
            "receive",
            "HIST-01",
            "--qty",
            "10",
            "--to",
            "Bin H",
            "--reason",
            "RecReason",
        ],
    );
    run_slim(
        &path,
        &[
            "stock",
            "issue",
            "HIST-01",
            "--qty",
            "2",
            "--from",
            "Bin H",
            "--reason",
            "IssReason",
        ],
    );

    let (ok, out, _) = run_slim(&path, &["history", "--item", "HIST-01"]);
    assert!(ok);
    assert!(out.contains("receive"));
    assert!(out.contains("issue"));
    assert!(out.contains("RecReason"));
    assert!(out.contains("IssReason"));
}

#[test]
fn test_cli_find_command() {
    let (_temp, path) = setup_db();
    run_slim(&path, &["actor", "add", "Operator"]);
    run_slim(
        &path,
        &[
            "item",
            "add",
            "--sku",
            "FIND-ME",
            "--name",
            "Special Widget",
        ],
    );
    run_slim(
        &path,
        &["location", "add", "--name", "Bin F", "--kind", "bin"],
    );
    run_slim(
        &path,
        &["stock", "receive", "FIND-ME", "--qty", "7", "--to", "Bin F"],
    );

    let (ok, out, _) = run_slim(&path, &["find", "Widget"]);
    assert!(ok);
    assert!(out.contains("Special Widget"));
    assert!(out.contains("FIND-ME"));
    assert!(out.contains("7 each"));
}

#[test]
fn test_cli_db_reconcile_and_backup() {
    let (_temp, path) = setup_db();
    run_slim(&path, &["actor", "add", "Operator"]);
    run_slim(
        &path,
        &["item", "add", "--sku", "REC-01", "--name", "Reconcile Item"],
    );
    run_slim(
        &path,
        &["location", "add", "--name", "Bin R", "--kind", "bin"],
    );
    run_slim(
        &path,
        &["stock", "receive", "REC-01", "--qty", "10", "--to", "Bin R"],
    );

    let (ok, out, _) = run_slim(&path, &["db", "reconcile"]);
    assert!(ok);
    assert!(out.contains("Reconciliation OK"));

    let backup_temp = NamedTempFile::new().unwrap();
    let backup_path = backup_temp.path().to_str().unwrap();
    let (ok, out, _) = run_slim(&path, &["db", "backup", backup_path]);
    assert!(ok);
    assert!(out.contains("successfully backed up"));

    let (ok, out, _) = run_slim(backup_path, &["db", "status"]);
    assert!(ok);
    assert!(out.contains("SLIM Database Status"));
}

#[test]
fn test_cli_grid_create_and_list() {
    let (_temp, path) = setup_db();
    run_slim(
        &path,
        &["location", "add", "--name", "Drawer 1", "--kind", "drawer"],
    );

    let (ok, out, _) = run_slim(
        &path,
        &[
            "grid", "create", "--code", "BASE-A", "--parent", "Drawer 1", "--cols", "5", "--rows",
            "4",
        ],
    );
    assert!(ok);
    assert!(
        out.contains(
            "Created Gridfinity baseplate 'BASE-A' (5x4 cells, 42.0mm pitch) in 'Drawer 1'"
        )
    );

    let (ok, out, _) = run_slim(&path, &["grid", "list"]);
    assert!(ok);
    assert!(out.contains("BASE-A"));
    assert!(out.contains("5x4"));
    assert!(out.contains("Drawer 1"));
}

#[test]
fn test_cli_bin_create_and_list() {
    let (_temp, path) = setup_db();
    let (ok, out, _) = run_slim(
        &path,
        &[
            "bin",
            "create",
            "--code",
            "BIN-101",
            "--width",
            "1",
            "--depth",
            "2",
            "--height",
            "6",
            "--compartments",
            "2",
        ],
    );
    assert!(ok);
    assert!(out.contains("Created Gridfinity bin 'BIN-101' (1x2x6U, 2 compartments)"));
    assert!(out.contains("Physical dimensions: 42.0 x 84.0 x 42.0 mm"));

    let (ok, out, _) = run_slim(&path, &["bin", "list"]);
    assert!(ok);
    assert!(out.contains("BIN-101"));
    assert!(out.contains("1x2x6"));
    assert!(out.contains("(unplaced)"));
}

#[test]
fn test_cli_bin_place_and_grid_show() {
    let (_temp, path) = setup_db();
    run_slim(
        &path,
        &["location", "add", "--name", "Drawer 2", "--kind", "drawer"],
    );
    run_slim(
        &path,
        &[
            "grid", "create", "--code", "BASE-B", "--parent", "Drawer 2", "--cols", "3", "--rows",
            "3",
        ],
    );
    run_slim(
        &path,
        &[
            "bin", "create", "--code", "BIN-01", "--width", "1", "--depth", "2", "--height", "3",
        ],
    );

    let (ok, out, _) = run_slim(
        &path,
        &[
            "bin", "place", "BIN-01", "--grid", "BASE-B", "-x", "0", "-y", "0",
        ],
    );
    assert!(ok);
    assert!(out.contains(
        "Placed bin 'BIN-01' onto baseplate 'BASE-B' at coordinate (0, 0) with rotation 0°"
    ));

    let (ok, out, _) = run_slim(&path, &["grid", "show", "BASE-B"]);
    assert!(ok);
    assert!(out.contains("Gridfinity Baseplate: BASE-B"));
    assert!(out.contains("Active Bins Placed:   1"));
    assert!(out.contains("BIN-01"));
}

#[test]
fn test_cli_bin_place_collision_and_bounds_rejection() {
    let (_temp, path) = setup_db();
    run_slim(
        &path,
        &["location", "add", "--name", "Drawer 3", "--kind", "drawer"],
    );
    run_slim(
        &path,
        &[
            "grid", "create", "--code", "BASE-C", "--parent", "Drawer 3", "--cols", "3", "--rows",
            "3",
        ],
    );
    run_slim(
        &path,
        &[
            "bin", "create", "--code", "BIN-A", "--width", "2", "--depth", "2", "--height", "3",
        ],
    );
    run_slim(
        &path,
        &[
            "bin", "create", "--code", "BIN-B", "--width", "1", "--depth", "1", "--height", "3",
        ],
    );

    // Place BIN-A at (0, 0), covers (0,0), (1,0), (0,1), (1,1)
    let (ok, _, _) = run_slim(
        &path,
        &[
            "bin", "place", "BIN-A", "--grid", "BASE-C", "-x", "0", "-y", "0",
        ],
    );
    assert!(ok);

    // Try to place BIN-B at (1, 1) -> collision
    let (ok, _, err) = run_slim(
        &path,
        &[
            "bin", "place", "BIN-B", "--grid", "BASE-C", "-x", "1", "-y", "1",
        ],
    );
    assert!(!ok);
    assert!(err.contains("Collision") || err.contains("overlap") || err.contains("occupied"));

    // Try to place BIN-B out of bounds at (3, 0) on a 3x3 plate (indices 0..2)
    let (ok, _, err) = run_slim(
        &path,
        &[
            "bin", "place", "BIN-B", "--grid", "BASE-C", "-x", "3", "-y", "0",
        ],
    );
    assert!(!ok);
    assert!(err.contains("out of bounds") || err.contains("exceeds"));
}

#[test]
fn test_cli_bin_stock_containment_and_removal() {
    let (_temp, path) = setup_db();
    run_slim(&path, &["actor", "add", "Operator"]);
    run_slim(
        &path,
        &["item", "add", "--sku", "RES-10K", "--name", "10k Resistor"],
    );
    run_slim(
        &path,
        &["location", "add", "--name", "Desk Shelf", "--kind", "shelf"],
    );
    run_slim(
        &path,
        &[
            "grid",
            "create",
            "--code",
            "BASE-D",
            "--parent",
            "Desk Shelf",
            "--cols",
            "2",
            "--rows",
            "2",
        ],
    );
    run_slim(
        &path,
        &[
            "bin", "create", "--code", "BIN-R1", "--width", "1", "--depth", "1", "--height", "3",
        ],
    );

    // Place bin on baseplate
    let (ok, _, _) = run_slim(
        &path,
        &[
            "bin", "place", "BIN-R1", "--grid", "BASE-D", "-x", "0", "-y", "0",
        ],
    );
    assert!(ok);

    // Receive stock directly into the bin (since bin is registered as a location)
    let (ok, out, _) = run_slim(
        &path,
        &[
            "stock", "receive", "RES-10K", "--qty", "100", "--to", "BIN-R1",
        ],
    );
    assert!(ok);
    assert!(out.contains("Received 100"));

    // Check bin show
    let (ok, out, _) = run_slim(&path, &["bin", "show", "BIN-R1"]);
    assert!(ok);
    assert!(out.contains("Baseplate 'BASE-D' at X=0, Y=0"));
    assert!(out.contains("RES-10K: 100 units"));

    // Remove bin from baseplate
    let (ok, out, _) = run_slim(&path, &["bin", "remove", "BIN-R1"]);
    assert!(ok);
    assert!(out.contains("Removed bin 'BIN-R1'"));

    // Inspect bin show again: unplaced, but stock still 100
    let (ok, out, _) = run_slim(&path, &["bin", "show", "BIN-R1"]);
    assert!(ok);
    assert!(out.contains("(unplaced / in transit)"));
    assert!(out.contains("RES-10K: 100 units"));
}

#[test]
fn test_cli_label_qr_ansi_and_svg() {
    let (_temp, path) = setup_db();

    // Raw QR terminal ANSI
    let (ok, out, _) = run_slim(&path, &["label", "qr", "slim:item:M3X10"]);
    assert!(ok);
    assert!(!out.trim().is_empty());

    // Raw QR SVG
    let (ok, out, _) = run_slim(&path, &["label", "qr", "slim:item:M3X10", "--svg"]);
    assert!(ok);
    assert!(out.contains("<svg") && out.contains("</svg>"));
}

#[test]
fn test_cli_label_item_and_bin() {
    let (_temp, path) = setup_db();
    run_slim(
        &path,
        &[
            "item",
            "add",
            "--sku",
            "CAP-100U",
            "--name",
            "100uF Capacitor",
            "--unit",
            "each",
        ],
    );
    run_slim(
        &path,
        &[
            "bin", "create", "--code", "BIN-CAP", "--width", "1", "--depth", "1", "--height", "3",
        ],
    );

    // Item label
    let (ok, out, _) = run_slim(&path, &["label", "item", "CAP-100U"]);
    assert!(ok);
    assert!(out.contains("SLIM INVENTORY ITEM LABEL"));
    assert!(out.contains("CAP-100U"));
    assert!(out.contains("100uF Capacitor"));
    assert!(out.contains("slim:item:"));

    // Bin label
    let (ok, out, _) = run_slim(&path, &["label", "bin", "BIN-CAP"]);
    assert!(ok);
    assert!(out.contains("GRIDFINITY STORAGE BIN"));
    assert!(out.contains("BIN-CAP"));
    assert!(out.contains("slim:bin:BIN-CAP"));
}

#[test]
fn test_cli_scan_resolve_item_and_barcode() {
    let (_temp, path) = setup_db();
    run_slim(
        &path,
        &[
            "item",
            "add",
            "--sku",
            "SCAN-01",
            "--name",
            "Scannable Widget",
            "--unit",
            "each",
        ],
    );
    run_slim(
        &path,
        &[
            "identifier",
            "add",
            "--kind",
            "barcode",
            "--value",
            "880123456789",
            "--item",
            "SCAN-01",
        ],
    );

    // Resolve by direct SKU
    let (ok, out, _) = run_slim(&path, &["scan", "resolve", "SCAN-01"]);
    assert!(ok);
    assert!(out.contains("Catalog Item"));
    assert!(out.contains("Scannable Widget"));

    // Resolve by standard SLIM URI
    let (ok, out, _) = run_slim(&path, &["scan", "resolve", "slim:item:SCAN-01"]);
    assert!(ok);
    assert!(out.contains("Catalog Item"));
    assert!(out.contains("Scannable Widget"));

    // Resolve by scanned barcode
    let (ok, out, _) = run_slim(&path, &["scan", "resolve", "880123456789"]);
    assert!(ok);
    assert!(out.contains("Catalog Item"));
    assert!(out.contains("Scannable Widget"));
}

#[test]
fn test_cli_scan_resolve_gridfinity_bin() {
    let (_temp, path) = setup_db();
    run_slim(
        &path,
        &[
            "bin", "create", "--code", "BIN-S1", "--width", "2", "--depth", "1", "--height", "3",
        ],
    );

    // Resolve by bin code
    let (ok, out, _) = run_slim(&path, &["scan", "resolve", "BIN-S1"]);
    assert!(ok);
    assert!(out.contains("Gridfinity Bin"));
    assert!(out.contains("BIN-S1"));
    assert!(out.contains("2W x 1D x 3H"));

    // Resolve by SLIM bin URI
    let (ok, out, _) = run_slim(&path, &["scan", "resolve", "slim:bin:BIN-S1"]);
    assert!(ok);
    assert!(out.contains("Gridfinity Bin"));
    assert!(out.contains("BIN-S1"));
}

#[test]
fn test_cli_scan_resolve_unregistered() {
    let (_temp, path) = setup_db();
    let (ok, out, _) = run_slim(&path, &["scan", "resolve", "UNKNOWN-CODE-9999"]);
    assert!(ok);
    assert!(out.contains("Unregistered Code"));
    assert!(out.contains("UNKNOWN-CODE-9999"));
}

#[test]
fn test_cli_actor_role_and_credentials() {
    let (_temp, path) = setup_db();

    // 1. Add actor with role
    let (ok, out, _) = run_slim(&path, &["actor", "add", "AdminUser", "--role", "admin"]);
    assert!(ok);
    assert!(out.contains("Role: admin"));

    // 2. Set PIN
    let (ok, out, _) = run_slim(&path, &["actor", "set-pin", "AdminUser", "--pin", "4321"]);
    assert!(ok);
    assert!(out.contains("PIN successfully updated"));

    // 3. Set Badge
    let (ok, out, _) = run_slim(&path, &["actor", "set-badge", "AdminUser", "BADGE-123"]);
    assert!(ok);
    assert!(out.contains("Assigned badge 'BADGE-123'"));

    // 4. List actors shows role
    let (ok, out, _) = run_slim(&path, &["actor", "list"]);
    assert!(ok);
    assert!(out.contains("AdminUser"));
    assert!(out.contains("admin"));

    // 5. Update role
    let (ok, out, _) = run_slim(&path, &["actor", "set-role", "AdminUser", "manager"]);
    assert!(ok);
    assert!(out.contains("Updated role for actor 'AdminUser' to manager"));
}

#[test]
fn test_cli_session_login_whoami_logout() {
    let (_temp, path) = setup_db();

    run_slim(&path, &["actor", "add", "StationOp", "--role", "operator"]);
    run_slim(&path, &["actor", "set-pin", "StationOp", "--pin", "5555"]);

    // Failed login with wrong PIN
    let (ok_fail, _, _) = run_slim(&path, &["session", "login", "StationOp", "--pin", "0000"]);
    assert!(!ok_fail);

    // Successful login
    let (ok, out, _) = run_slim(&path, &["session", "login", "StationOp", "--pin", "5555"]);
    assert!(ok);
    assert!(out.contains("Successfully authenticated as 'StationOp'"));

    // whoami confirms active session
    let (ok, out, _) = run_slim(&path, &["session", "whoami"]);
    assert!(ok);
    assert!(out.contains("Active Session: StationOp [operator]"));

    // Logout
    let (ok, out, _) = run_slim(&path, &["session", "logout"]);
    assert!(ok);
    assert!(out.contains("Session logged out"));

    // whoami confirms logged out
    let (ok, out, _) = run_slim(&path, &["session", "whoami"]);
    assert!(ok);
    assert!(!out.contains("Active Session"));
}

#[test]
fn test_cli_session_badge_login() {
    let (_temp, path) = setup_db();

    run_slim(&path, &["actor", "add", "BadgeUser", "--role", "manager"]);
    run_slim(&path, &["actor", "set-badge", "BadgeUser", "NFC-9999"]);

    // Login with badge
    let (ok, out, _) = run_slim(&path, &["session", "badge", "NFC-9999"]);
    assert!(ok);
    assert!(out.contains("Badge recognized"));
    assert!(out.contains("BadgeUser") && out.contains("[manager]"));

    // Cleanup session
    run_slim(&path, &["session", "logout"]);
}

#[test]
fn test_cli_audit_trail_recorded_and_filtered() {
    let (_temp, path) = setup_db();

    run_slim(&path, &["actor", "add", "AuditedActor", "--role", "admin"]);
    run_slim(
        &path,
        &["actor", "set-pin", "AuditedActor", "--pin", "9876"],
    );
    run_slim(
        &path,
        &["actor", "set-badge", "AuditedActor", "BADGE-AUDIT"],
    );
    run_slim(
        &path,
        &["session", "login", "AuditedActor", "--pin", "9876"],
    );
    run_slim(&path, &["session", "logout"]);

    // Query audit list
    let (ok, out, _) = run_slim(&path, &["audit", "list"]);
    assert!(ok);
    assert!(out.contains("actor.updated"));
    assert!(out.contains("actor.set_badge"));
    assert!(out.contains("auth.login.pin"));
    assert!(out.contains("auth.logout"));

    // Filter audit by action
    let (ok, out, _) = run_slim(&path, &["audit", "list", "--action", "auth."]);
    assert!(ok);
    assert!(out.contains("auth.login.pin"));
    assert!(out.contains("auth.logout"));
    assert!(!out.contains("actor.set_badge"));

    // Query audit with JSON flag
    let (ok, out, _) = run_slim(&path, &["--json", "audit", "list"]);
    assert!(ok);
    assert!(out.contains("\"action\": \"auth.logout\""));
}

#[test]
fn test_cli_automation_rules_add_list_remove() {
    let (_temp, path) = setup_db();

    // List initially empty
    let (ok, out, _) = run_slim(&path, &["automation", "rules", "list"]);
    assert!(ok);
    assert!(out.contains("No automation rules found"));

    // Add a notify rule
    let (ok, out, _) = run_slim(
        &path,
        &[
            "automation",
            "rules",
            "add",
            "low-m3",
            "--name",
            "Low M3 Screw Alert",
            "--event",
            "stock.below_threshold",
            "--sku",
            "M3X10",
            "--quantity-below",
            "50",
            "--action-type",
            "notify",
            "--message",
            "M3x10 screws running low",
        ],
    );
    assert!(ok);
    assert!(out.contains("saved successfully"));

    // List rules
    let (ok, out, _) = run_slim(&path, &["automation", "rules", "list"]);
    assert!(ok);
    assert!(out.contains("low-m3"));
    assert!(out.contains("M3X10"));
    assert!(out.contains("notify"));

    // Test alias "hook list"
    let (ok, out, _) = run_slim(&path, &["hook", "rules", "list"]);
    assert!(ok);
    assert!(out.contains("low-m3"));

    // JSON output
    let (ok, out, _) = run_slim(&path, &["--json", "automation", "rules", "list"]);
    assert!(ok);
    assert!(out.contains("\"id\": \"low-m3\""));

    // Remove rule
    let (ok, out, _) = run_slim(&path, &["automation", "rules", "remove", "low-m3"]);
    assert!(ok);
    assert!(out.contains("removed"));

    let (ok, out, _) = run_slim(&path, &["automation", "rules", "list"]);
    assert!(ok);
    assert!(out.contains("No automation rules found"));
}

#[test]
fn test_cli_automation_worker_and_history() {
    let (_temp, path) = setup_db();

    run_slim(&path, &["actor", "add", "AutoOperator"]);
    run_slim(
        &path,
        &[
            "item",
            "add",
            "--sku",
            "AUTO-01",
            "--name",
            "Automation Test Item",
        ],
    );
    run_slim(
        &path,
        &["location", "add", "--name", "Auto Bin", "--kind", "bin"],
    );

    // Stock receive generates a StockReceived outbox event
    run_slim(
        &path,
        &[
            "stock", "receive", "AUTO-01", "--qty", "10", "--to", "Auto Bin",
        ],
    );

    // Check pending outbox events
    let (ok, out, _) = run_slim(&path, &["automation", "pending"]);
    assert!(ok);
    assert!(out.contains("StockReceived"));

    // Add automation rule for stock.received
    run_slim(
        &path,
        &[
            "automation",
            "rules",
            "add",
            "on-received",
            "--event",
            "stock.received",
            "--sku",
            "AUTO-01",
            "--action-type",
            "notify",
            "--message",
            "AUTO-01 stock received successfully",
        ],
    );

    // Run automation worker in --once mode
    let (ok, out, _) = run_slim(&path, &["automation", "worker", "--once"]);
    assert!(ok);
    assert!(out.contains("Processed"));
    assert!(out.contains("[success]"));
    assert!(out.contains("AUTO-01 stock received successfully"));

    // Check pending is now empty
    let (ok, out, _) = run_slim(&path, &["automation", "pending"]);
    assert!(ok);
    assert!(out.contains("All events delivered"));

    // Check automation history
    let (ok, out, _) = run_slim(&path, &["automation", "history"]);
    assert!(ok);
    assert!(out.contains("on-received"));
    assert!(out.contains("success"));
    assert!(out.contains("AUTO-01 stock received successfully"));

    // Test command
    let (ok, out, _) = run_slim(
        &path,
        &[
            "automation",
            "test",
            "on-received",
            "--sku",
            "AUTO-01",
            "--qty",
            "10",
        ],
    );
    assert!(ok);
    assert!(out.contains("MATCHES"));
    assert!(out.contains("[success]"));
}

#[test]
fn test_cli_device_and_observation_lifecycle() {
    let (_temp, path) = setup_db();

    // 1. Setup prerequisite actor, item, and location
    let (ok, _, _) = run_slim(&path, &["actor", "add", "Alice", "--os-user", "alice"]);
    assert!(ok);

    let (ok, _, _) = run_slim(
        &path,
        &[
            "item",
            "add",
            "--sku",
            "RES-100K",
            "--name",
            "100k Resistor",
            "--unit",
            "each",
        ],
    );
    assert!(ok);

    let (ok, _, _) = run_slim(
        &path,
        &["location", "add", "--name", "Bin A", "--kind", "bin"],
    );
    assert!(ok);

    // 2. Register hardware device
    let (ok, out, _) = run_slim(
        &path,
        &[
            "device",
            "add",
            "ir-bin-1",
            "--name",
            "Bin 1 Sensor",
            "--kind",
            "ir_breakbeam",
            "--location",
            "Bin A",
            "--port",
            "COM5",
        ],
    );
    assert!(ok);
    assert!(out.contains("Registered device 'ir-bin-1'"));

    // 3. List devices
    let (ok, out, _) = run_slim(&path, &["device", "list"]);
    assert!(ok);
    assert!(out.contains("ir-bin-1"));
    assert!(out.contains("Bin 1 Sensor"));
    assert!(out.contains("ir_breakbeam"));
    assert!(out.contains("COM5"));
    assert!(out.contains("Bin A"));

    // 4. Simulate beam interruption
    let (ok, out, _) = run_slim(
        &path,
        &[
            "device",
            "simulate",
            "ir-bin-1",
            "--event",
            "beam_interrupted",
            "--zone",
            "Bin A",
        ],
    );
    assert!(ok);
    assert!(out.contains("Recorded observation"));
    assert!(out.contains("pending_review"));

    // 5. Query pending observations via JSON
    let (ok, out, err) = run_slim(&path, &["observation", "list", "--pending", "--json"]);
    assert!(ok, "failed observation list: out={}, err={}", out, err);
    let obs_list: serde_json::Value = serde_json::from_str(&out).unwrap();
    let obs_arr = obs_list.as_array().unwrap();
    assert_eq!(obs_arr.len(), 1);
    let obs_id = obs_arr[0]["id"].as_str().unwrap();

    // 6. Receive stock so we can confirm observation with an issue transaction
    let (ok, _, _) = run_slim(
        &path,
        &[
            "stock", "receive", "RES-100K", "--qty", "10", "--to", "Bin A", "--actor", "Alice",
        ],
    );
    assert!(ok);

    // 7. Confirm observation and issue 3 units
    let (ok, out, _) = run_slim(
        &path,
        &[
            "observation",
            "confirm",
            obs_id,
            "--actor",
            "Alice",
            "--sku",
            "RES-100K",
            "--qty",
            "3",
            "--kind",
            "issue",
        ],
    );
    assert!(ok, "confirm failed: {}", out);
    assert!(out.contains("confirmed by actor 'Alice'"));
    assert!(out.contains("Triggered stock operation"));

    // 8. Verify stock was issued (10 - 3 = 7)
    let (ok, out, _) = run_slim(&path, &["stock", "balance", "RES-100K"]);
    assert!(ok);
    assert!(out.contains("Total: 7 each"));

    // 9. Simulate a second observation (presence changed)
    let (ok, out, _) = run_slim(
        &path,
        &[
            "device",
            "simulate",
            "ir-bin-1",
            "--event",
            "presence_changed",
            "--zone",
            "Bin A",
            "--present",
            "false",
        ],
    );
    assert!(ok);
    assert!(out.contains("Recorded observation"));

    // 10. Get pending observations and reject it
    let (ok, out, _) = run_slim(&path, &["observation", "list", "--pending", "--json"]);
    assert!(ok);
    let obs_list2: serde_json::Value = serde_json::from_str(&out).unwrap();
    let obs_arr2 = obs_list2.as_array().unwrap();
    assert_eq!(obs_arr2.len(), 1);
    let obs_id2 = obs_arr2[0]["id"].as_str().unwrap();

    let (ok, out, _) = run_slim(
        &path,
        &[
            "observation",
            "reject",
            obs_id2,
            "--actor",
            "Alice",
            "--reason",
            "False trigger from dust",
        ],
    );
    assert!(ok);
    assert!(out.contains("rejected by actor 'Alice'"));
    assert!(out.contains("False trigger from dust"));

    // 11. Verify all observations list has both Confirmed and Rejected
    let (ok, out, _) = run_slim(&path, &["observation", "list"]);
    assert!(ok);
    assert!(out.contains("Confirmed"));
    assert!(out.contains("Rejected"));

    // 12. Remove device
    let (ok, out, err) = run_slim(&path, &["device", "remove", "ir-bin-1"]);
    assert!(ok, "device remove failed: out={}, err={}", out, err);
    assert!(out.contains("unregistered successfully"));

    let (ok, out, _) = run_slim(&path, &["device", "list"]);
    assert!(ok);
    assert!(out.contains("No devices registered"));
}

#[test]
fn test_cli_led_locate_and_priority_lifecycle() {
    let (_temp, path) = setup_db();

    // 1. Setup prerequisite actor, item, and location
    run_slim(&path, &["actor", "add", "Alice", "--os-user", "alice"]);
    run_slim(
        &path,
        &[
            "item",
            "add",
            "--sku",
            "LED-M3X10",
            "--name",
            "LED Test Screw",
            "--unit",
            "each",
        ],
    );
    run_slim(
        &path,
        &[
            "location",
            "add",
            "--name",
            "Hardware Lab",
            "--kind",
            "room",
        ],
    );

    // 2. Register a baseplate (3x3)
    let (ok, out, _) = run_slim(
        &path,
        &[
            "grid",
            "create",
            "--code",
            "BASE-LED-1",
            "--parent",
            "Hardware Lab",
            "--cols",
            "3",
            "--rows",
            "3",
        ],
    );
    assert!(ok, "grid create failed: {}", out);
    assert!(out.contains("Created Gridfinity baseplate 'BASE-LED-1'"));

    // 3. Create a 2x1 bin
    let (ok, out, _) = run_slim(
        &path,
        &[
            "bin",
            "create",
            "--code",
            "BIN-LED-2X1",
            "--width",
            "2",
            "--depth",
            "1",
            "--height",
            "3",
        ],
    );
    assert!(ok, "bin create failed: {}", out);
    assert!(out.contains("Created Gridfinity bin 'BIN-LED-2X1'"));

    // 4. Place bin at X=0, Y=0 (occupies (0,0) and (1,0))
    let (ok, out, _) = run_slim(
        &path,
        &[
            "bin",
            "place",
            "BIN-LED-2X1",
            "--grid",
            "BASE-LED-1",
            "-x",
            "0",
            "-y",
            "0",
        ],
    );
    assert!(ok, "bin place failed: {}", out);
    assert!(out.contains("Placed bin 'BIN-LED-2X1' onto baseplate 'BASE-LED-1'"));

    // 4b. Receive stock of LED-M3X10 into the bin
    let (ok, out, _) = run_slim(
        &path,
        &[
            "stock",
            "receive",
            "LED-M3X10",
            "--qty",
            "50",
            "--to",
            "BIN-LED-2X1",
        ],
    );
    assert!(ok, "stock receive failed: {}", out);

    // 5. Initially LED status is idle / empty
    let (ok, out, _) = run_slim(&path, &["led", "status"]);
    assert!(ok);
    assert!(out.contains("No active LED states"));

    // 6. Test find with --locate flag
    let (ok, out, _) = run_slim(&path, &["find", "LED-M3X10", "--locate", "--ttl", "45"]);
    assert!(ok, "find --locate failed: {}", out);
    assert!(out.contains("Physical dock LEDs illuminated"));
    assert!(out.contains("BASE-LED-1:0:0"));
    assert!(out.contains("BASE-LED-1:1:0"));

    // 7. Test led status shows both multi-cell docks illuminated
    let (ok, out, _) = run_slim(&path, &["led", "status"]);
    assert!(ok);
    assert!(out.contains("BASE-LED-1:0:0"));
    assert!(out.contains("BASE-LED-1:1:0"));
    assert!(out.contains("locate"));

    // 8. Test led status in JSON mode
    let (ok, out, _) = run_slim(&path, &["led", "status", "--json"]);
    assert!(ok);
    let json_val: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert!(json_val.is_array());
    assert_eq!(json_val.as_array().unwrap().len(), 2);

    // 9. Simulate dock presence missing (dock sensor detects bin removed without system move)
    let (ok, out, err) = run_slim(
        &path,
        &[
            "led",
            "simulate-presence",
            "BASE-LED-1:0:0",
            "--occupied",
            "false",
            "--status",
            "missing",
        ],
    );
    assert!(ok, "simulate-presence failed: out={}, err={}", out, err);
    assert!(out.contains("Updated dock 'BASE-LED-1:0:0' presence: EMPTY"));
    assert!(out.contains("Raised Missing Bin warning"));

    // 10. Test led evaluate with serial output
    // Missing state has higher priority than locate, so dock 0:0 becomes Missing!
    let (ok, out, _) = run_slim(
        &path,
        &["led", "evaluate", "--base", "BASE-LED-1", "--serial"],
    );
    assert!(ok, "led evaluate --serial failed: {}", out);
    // Serial protocol frames are JSON lines with v=1
    assert!(out.contains("\"v\":1"));
    assert!(out.contains("\"type\":\"set_led\""));
    assert!(out.contains("\"dock_id\":\"BASE-LED-1:0:0\""));
    assert!(out.contains("\"mode\":\"missing\""));

    // 11. Clear single dock
    let (ok, out, _) = run_slim(&path, &["led", "clear", "--dock", "BASE-LED-1:1:0"]);
    assert!(ok);
    assert!(out.contains("Cleared LED state for dock 'BASE-LED-1:1:0'"));

    // 12. Clear all remaining LEDs
    let (ok, out, _) = run_slim(&path, &["led", "clear", "--all"]);
    assert!(ok);
    assert!(out.contains("Cleared all"));

    // 13. Verify status is clear
    let (ok, out, _) = run_slim(&path, &["led", "status"]);
    assert!(ok);
    assert!(out.contains("No active LED states"));
}
