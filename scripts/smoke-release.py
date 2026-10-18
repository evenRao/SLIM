"""Exercise a built SLIM executable without touching a user's configuration or data."""
import json
from contextlib import closing
import pathlib
import sqlite3
import subprocess
import sys
import tempfile
import urllib.error
import urllib.request

binary = pathlib.Path(sys.argv[1]).resolve()
with tempfile.TemporaryDirectory(prefix="slim-release-test-") as temporary:
    directory = pathlib.Path(temporary)
    config = directory / "config.toml"
    config.write_text("", encoding="utf-8")
    process = subprocess.Popen(
        [str(binary), "--config", str(config), "--db", str(directory / "inventory.sqlite3"), "app", "--no-open"],
        cwd=directory, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
    )
    base = key = None
    try:
        address = process.stdout.readline().strip()
        assert address.startswith("SLIM is ready: "), address
        base, key = address.removeprefix("SLIM is ready: ").split("/#", 1)
        def request(route, data=None, session=""):
            body = None if data is None else json.dumps(data).encode()
            req = urllib.request.Request(base + "/api/" + route, data=body, headers={
                "X-Slim-Key": key, "X-Slim-Session": session, "Content-Type": "application/json",
            })
            with urllib.request.urlopen(req, timeout=10) as response:
                return json.load(response)

        assert request("state")["setup"] is True
        session = request("setup", {"name": "Release test", "pin": "test-only-9876"})["session"]
        state = request("state", session=session)
        location = state["locations"][0]["location"]["id"]
        item = request("items", {"name": "Test fasteners", "sku": "TEST-M3", "minimum": 3}, session)["id"]
        movement = {"kind": "receive", "item": item, "to": location, "quantity": 10, "request_id": "release-receive"}
        request("stock", movement, session)
        request("stock", movement, session)
        assert request("state", session=session)["items"][0]["total"] == 10
        request("stock", {"kind": "issue", "item": item, "from": location, "quantity": 1, "request_id": "release-issue"}, session)
        assert request("state", session=session)["items"][0]["total"] == 9
        assert request("check", {}, session)["mismatches"] == []
        backup = request("backup", {}, session)["path"]
        assert pathlib.Path(backup).is_file()
        disposable = request("items", {"name": "Disposable release test"}, session)["id"]
        request("users", {"name": "Release manager", "role": "manager", "pin": "test-only-4321"}, session)
        manager = request("login", {"name": "Release manager", "pin": "test-only-4321"})["session"]
        try:
            request("items/delete", {"id": disposable}, manager)
            raise AssertionError("Manager was allowed to delete inventory")
        except urllib.error.HTTPError as error:
            assert error.code in (400, 403)
        request("items/delete", {"id": disposable}, session)
        deleted = next(row for row in request("state", session=session)["items"] if row["item"]["id"] == disposable)
        assert deleted["item"]["archived_at"] is not None
        request("items/restore", {"id": disposable}, session)
        restored = next(row for row in request("state", session=session)["items"] if row["item"]["id"] == disposable)
        assert restored["item"]["archived_at"] is None
        request("stock", {"kind": "receive", "item": disposable, "to": location, "quantity": 2, "request_id": "purge-receive"}, session)
        request("stock", {"kind": "issue", "item": disposable, "from": location, "quantity": 2, "request_id": "purge-issue"}, session)
        request("items/delete", {"id": disposable}, session)
        for who, confirmation in ((manager, "DELETE PERMANENTLY"), (session, "DELETE")):
            try:
                request("items/purge", {"id": disposable, "confirmation": confirmation}, who)
                raise AssertionError("Permanent deletion was allowed without Admin and exact confirmation")
            except urllib.error.HTTPError as error:
                assert error.code in (400, 403)
        request("items/purge", {"id": disposable, "confirmation": "DELETE PERMANENTLY"}, session)
        assert not any(row["item"]["id"] == disposable for row in request("state", session=session)["items"])
        with closing(sqlite3.connect(directory / "inventory.sqlite3")) as database:
            for table in ("items", "stock_movements", "stock_balances", "stock_thresholds", "identifiers"):
                column = "id" if table == "items" else "item_id"
                assert database.execute(f"SELECT count(*) FROM {table} WHERE {column}=?", (disposable,)).fetchone()[0] == 0
            assert database.execute("PRAGMA foreign_key_check").fetchall() == []
        assert request("check", {}, session)["mismatches"] == []
        staff = next(a for a in request("state", session=session)["actors"] if a["name"] == "Release manager")
        for route in ("users/edit", "users/remove", "users/pin"):
            try:
                request(route, {"id": staff["id"], "name": "Changed", "role": "admin", "pin": "test-only-new-pin"}, manager)
                raise AssertionError("Manager was allowed to manage accounts")
            except urllib.error.HTTPError as error:
                assert error.code in (400, 403)
        request("users/edit", {"id": staff["id"], "name": "Renamed manager", "role": "manager", "pin": "test-only-new-pin"}, session)
        assert request("state", session=manager)["actor"] is None
        request("users/remove", {"id": staff["id"]}, session)
        try:
            request("login", {"name": "Renamed manager", "pin": "test-only-new-pin"})
            raise AssertionError("Removed user could sign in")
        except urllib.error.HTTPError as error:
            assert error.code in (400, 403)
        request("users/restore", {"id": staff["id"]}, session)
        assert request("login", {"name": "Renamed manager", "pin": "test-only-new-pin"})["session"]
        request("logout", {}, session)
        assert request("state", session=session)["actor"] is None
        request("quit", {})
        process.wait(timeout=15)
        assert process.returncode == 0
        print("Release smoke passed: onboarding, stock, health, backup, admin-only permanent deletion with raw database checks, user management, session revocation, logout, clean exit.")
    finally:
        if process.poll() is None:
            process.kill()
            process.wait()
