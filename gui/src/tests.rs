//! Headless tests: a scripted DAP server on a socket (`--attach`), the app driven through its
//! accessibility tree with egui_kittest.

use std::io::BufReader;
use std::net::TcpListener;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};

use crate::app::App;
use crate::dap::{Target, read_message, write_message};

#[test]
fn args() {
    let p = |a: &[&str]| super::parse_args(a.iter().map(|s| s.to_string()));
    match p(&["--yavs", "y", "--", "--work=w", "tb"]) {
        Ok(Target::Launch { yavs, args }) => {
            assert_eq!(yavs.unwrap().to_str(), Some("y"));
            assert_eq!(args, ["--work=w", "tb"]);
        }
        _ => panic!("expected launch"),
    }
    assert!(matches!(p(&["--attach", "h:1"]), Ok(Target::Attach(a)) if a == "h:1"));
    assert!(p(&[]).is_err());
    assert!(p(&["--"]).is_err());
    assert!(p(&["--attach", "h:1", "--", "tb"]).is_err());
    assert!(p(&["--bogus"]).is_err());
}

/// The mock: answers each request in order, then sends the events listed for it.
fn serve(listener: TcpListener) -> Vec<String> {
    let (s, _) = listener.accept().unwrap();
    let mut w = s.try_clone().unwrap();
    let mut r = BufReader::new(s);
    let mut seen = Vec::new();
    let mut send = |m: Value| write_message(&mut w, &m).unwrap();
    while let Some(req) = read_message(&mut r).unwrap() {
        let command = req["command"].as_str().unwrap().to_string();
        let response = json!({"type": "response", "request_seq": req["seq"], "command": command, "success": true, "body": {}});
        let event = |e: &str, body: Value| json!({"type": "event", "event": e, "body": body});
        match command.as_str() {
            "attach" => {
                send(event("initialized", json!({})));
                send(response);
            }
            "configurationDone" => {
                send(response);
                send(event(
                    "output",
                    json!({"category": "stdout", "output": "hello from the mock\n"}),
                ));
            }
            "pause" => {
                send(response);
                send(event(
                    "stopped",
                    json!({"reason": "pause", "description": "pause at 5 ns, delta 1", "threadId": 1}),
                ));
            }
            "continue" => {
                send(response);
                send(event("continued", json!({"threadId": 1})));
                send(event(
                    "output",
                    json!({"category": "stderr", "output": "late report\n"}),
                ));
            }
            "disconnect" => {
                send(event("exited", json!({"exitCode": 0})));
                send(event("terminated", json!({})));
                send(response);
                seen.push(command);
                return seen;
            }
            _ => send(response),
        }
        seen.push(command);
    }
    seen
}

/// Step the harness until `label` (a substring) is on screen.
fn wait_for(h: &mut Harness<'_, App>, label: &str) {
    let end = Instant::now() + Duration::from_secs(10);
    loop {
        h.step();
        if h.query_all_by_label_contains(label).next().is_some() {
            return;
        }
        assert!(Instant::now() < end, "`{label}` never appeared");
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn session_against_mock() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let server = thread::spawn(move || serve(listener));

    let app = App::new(&Target::Attach(addr), Arc::new(|| {}));
    let mut h = Harness::builder()
        .with_size([1200.0, 800.0])
        .build_ui_state(|ui, app: &mut App| app.show(ui), app);
    wait_for(&mut h, "hello from the mock");
    wait_for(&mut h, "running");
    h.get_by_label("Pause").click();
    wait_for(&mut h, "stopped: pause at 5 ns, delta 1");
    h.get_by_label("Run").click();
    wait_for(&mut h, "late report");
    h.get_by_label("Stop").click();
    wait_for(&mut h, "session ended: exited with code 0");
    assert!(
        h.query_by_label_contains("Waves: not implemented yet")
            .is_some()
    );
    drop(h);
    assert_eq!(
        server.join().unwrap(),
        [
            "initialize",
            "attach",
            "configurationDone",
            "pause",
            "continue",
            "disconnect"
        ]
    );
}

#[test]
fn connection_failure_is_shown() {
    // A port nobody listens on: bind, read the port, close.
    let addr = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .to_string();
    let app = App::new(&Target::Attach(addr.clone()), Arc::new(|| {}));
    let mut h = Harness::builder().build_ui_state(|ui, app: &mut App| app.show(ui), app);
    wait_for(&mut h, &format!("cannot connect to `{addr}`"));
}

/// The real simulator on the editor sample: `YAVS_BIN=<path to yavs> cargo test -- --ignored`.
#[test]
#[ignore = "needs YAVS_BIN, a built yavs"]
fn launch_real_yavs() {
    let yavs = std::path::PathBuf::from(std::env::var("YAVS_BIN").expect("YAVS_BIN"));
    let work = std::env::temp_dir().join(format!("yavs-gui-{}", std::process::id()));
    let tb = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../editors/vscode/sample/tb.vhd"
    );
    let lib = format!("--work=work:{}", work.display());
    let ok = std::process::Command::new(&yavs)
        .args(["analyze", &lib, tb])
        .status()
        .unwrap();
    assert!(ok.success());
    let target = Target::Launch {
        yavs: Some(yavs),
        args: vec![lib, "tb".into()],
    };
    let app = App::new(&target, Arc::new(|| {}));
    let mut h = Harness::builder().build_ui_state(|ui, app: &mut App| app.show(ui), app);
    wait_for(&mut h, "done");
    wait_for(&mut h, "session ended: exited with code 0");
    drop(h);
    let _ = std::fs::remove_dir_all(work);
}
