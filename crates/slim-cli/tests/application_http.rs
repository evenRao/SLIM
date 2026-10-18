use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader},
    process::{Child, Command, Stdio},
    time::Duration,
};

struct Running(Child);
impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn real_application_onboarding_authentication_and_stock() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("config.toml"), "").unwrap();
    let mut process = Running(
        Command::new(env!("CARGO_BIN_EXE_slim"))
            .current_dir(dir.path())
            .args([
                "--db",
                dir.path().join("app.sqlite3").to_str().unwrap(),
                "app",
                "--no-open",
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let mut line = String::new();
    BufReader::new(process.0.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    let url = line
        .trim()
        .strip_prefix("SLIM is ready: ")
        .expect("server address");
    let (base, key) = url.split_once("/#").unwrap();
    let http = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(5))
        .build();
    let raw = http.get(&format!("{base}/api/state")).call();
    assert!(
        matches!(raw, Err(ureq::Error::Status(403, _))),
        "Unexpected response: {raw:?}"
    );
    let foreign = http
        .get(&format!("{base}/api/state"))
        .set("X-Slim-Key", key)
        .set("Origin", "https://other.example")
        .call();
    assert!(matches!(foreign, Err(ureq::Error::Status(403, _))));
    let page = http.get(base).call().unwrap();
    assert!(page.header("Content-Security-Policy").is_some());
    assert!(page.into_string().unwrap().contains("SLIM · Inventory"));
    let request = |route: &str, data: Value, session: &str| -> Value {
        let result = http
            .post(&format!("{base}/api/{route}"))
            .set("X-Slim-Key", key)
            .set("X-Slim-Session", session)
            .set("Content-Type", "application/json")
            .send_string(&data.to_string())
            .unwrap()
            .into_string()
            .unwrap();
        serde_json::from_str(&result).unwrap()
    };
    let login = request("setup", json!({"name":"Test Owner","pin":"test-pin"}), "");
    let session = login["session"].as_str().unwrap();
    let get_state = || -> Value {
        serde_json::from_str(
            &http
                .get(&format!("{base}/api/state"))
                .set("X-Slim-Key", key)
                .set("X-Slim-Session", session)
                .call()
                .unwrap()
                .into_string()
                .unwrap(),
        )
        .unwrap()
    };
    let state = get_state();
    let location = state["locations"][0]["location"]["id"].clone();
    let item = request("items", json!({"name":"A part","minimum":3}), session);
    let change = json!({"kind":"receive","item":item["id"],"to":location,"quantity":8,"request_id":"http-receive"});
    request("stock", change.clone(), session);
    request("stock", change, session);
    assert_eq!(get_state()["items"][0]["total"], 8);
    request("quit", json!({}), session);
    for _ in 0..50 {
        if process.0.try_wait().unwrap().is_some() {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!("application did not quit cleanly");
}
