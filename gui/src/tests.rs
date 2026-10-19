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
        if Instant::now() >= end {
            let d = h.state_mut().dbg();
            let log: Vec<_> = d.console.iter().map(|l| l.text.as_str()).collect();
            panic!(
                "`{label}` never appeared; status `{}`, frame {:?}, console {log:?}",
                d.status,
                d.stack.frame()
            );
        }
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
            "threads",
            "continue",
            "disconnect"
        ]
    );
}

/// A scripted debugger session: stopped at line 3 of a six line file in process `p` (thread 1)
/// with `q` waiting. Records every request (command and arguments) and returns them on
/// `disconnect`.
fn serve_debug(listener: TcpListener, source: String) -> Vec<(String, Value)> {
    let (s, _) = listener.accept().unwrap();
    let mut w = s.try_clone().unwrap();
    let mut r = BufReader::new(s);
    let mut seen = Vec::new();
    let mut line = 3;
    let mut send = |m: Value| write_message(&mut w, &m).unwrap();
    let event = |e: &str, body: Value| json!({"type": "event", "event": e, "body": body});
    let frame = |id: i64, name: &str, line: u32| json!({"id": id, "name": name, "line": line, "column": 1, "source": {"path": source}});
    let var = |n: &str, v: &str, r: i64| json!({"name": n, "value": v, "variablesReference": r});
    while let Some(req) = read_message(&mut r).unwrap() {
        let command = req["command"].as_str().unwrap().to_string();
        let a = &req["arguments"];
        seen.push((command.clone(), a.clone()));
        let mut body = json!({});
        match command.as_str() {
            "attach" => send(event("initialized", json!({}))),
            "threads" => {
                body = json!({"threads": [{"id": 1, "name": "p"}, {"id": 2, "name": "q"}]})
            }
            "stackTrace" => {
                let f = if a["threadId"] == 1 {
                    frame(1 << 16, "p", line)
                } else {
                    frame(2 << 16, "q", 1)
                };
                body = json!({"stackFrames": [f], "totalFrames": 1});
            }
            "scopes" => {
                body = json!({"scopes": [
                    {"name": "Locals", "variablesReference": 5, "expensive": false},
                    {"name": "Signals", "variablesReference": 6, "expensive": false},
                    {"name": "Hierarchy", "variablesReference": 4, "expensive": true}]});
            }
            "variables" => {
                let (start, count) = (a["start"].as_u64().unwrap_or(0), a["count"].as_u64());
                let vars = match a["variablesReference"].as_i64().unwrap() {
                    5 => vec![
                        var("i", "0", 0),
                        var("rec", "(a, b)", 100),
                        json!({"name": "arr", "value": "array", "variablesReference": 101,
                               "indexedVariables": 150}),
                    ],
                    6 => vec![var("cnt", "2", 0)],
                    100 => vec![var("a", "1", 0), var("b", "2", 0)],
                    101 => (start..150.min(start + count.unwrap()))
                        .map(|k| var(&k.to_string(), "0", 0))
                        .collect(),
                    _ => vec![],
                };
                body = json!({"variables": vars});
            }
            "setBreakpoints" => {
                // Line 4 has no statement: the server moves it to 5.
                let bps: Vec<Value> = a["breakpoints"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|b| {
                        let l = b["line"].as_u64().unwrap();
                        json!({"verified": true, "line": if l == 4 { 5 } else { l }})
                    })
                    .collect();
                body = json!({"breakpoints": bps});
            }
            "evaluate" => body = json!({"result": "42", "variablesReference": 0}),
            _ => {}
        }
        let ok = json!({"type": "response", "request_seq": req["seq"], "command": command,
                        "success": true, "body": body});
        match command.as_str() {
            "configurationDone" => {
                send(ok);
                send(event(
                    "stopped",
                    json!({"reason": "breakpoint", "threadId": 1,
                           "description": "breakpoint at 10 ns, delta 2"}),
                ));
            }
            "next" => {
                line += 1;
                send(ok);
                send(event("continued", json!({"threadId": 1})));
                send(event(
                    "stopped",
                    json!({"reason": "step", "threadId": 1,
                           "description": "step at 10 ns, delta 3"}),
                ));
            }
            "disconnect" => {
                send(event("terminated", json!({})));
                send(ok);
                return seen;
            }
            _ => send(ok),
        }
    }
    seen
}

fn sent<'a>(seen: &'a [(String, Value)], command: &str) -> Vec<&'a Value> {
    seen.iter()
        .filter(|(c, _)| c == command)
        .map(|(_, a)| a)
        .collect()
}

#[test]
fn debugging_against_mock() {
    let file = std::env::temp_dir().join(format!("yavs-gui-tb-{}.vhd", std::process::id()));
    std::fs::write(
        &file,
        "-- top\nentity tb is end;\n\tx <= 1;\ny <= 2;\nz <= 3;\nwait;\n",
    )
    .unwrap();
    let path = file.to_str().unwrap().to_string();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let p = path.clone();
    let server = thread::spawn(move || serve_debug(listener, p));

    let app = App::new(&Target::Attach(addr), Arc::new(|| {}));
    let mut h = Harness::builder()
        .with_size([1400.0, 900.0])
        .build_ui_state(|ui, app: &mut App| app.show(ui), app);

    // The stop: time and delta, the current line, the process, locals, the stack.
    wait_for(&mut h, "stopped: breakpoint at 10 ns, delta 2");
    wait_for(&mut h, "time 10 ns  delta 2");
    wait_for(&mut h, "process p");
    wait_for(&mut h, "▶    3");
    wait_for(&mut h, "i = 0");
    wait_for(&mut h, "p  ");
    wait_for(&mut h, "entity");

    // Click in the gutter: the breakpoint slides from 4 to 5 and is drawn there.
    h.get_by_label("     4").click();
    wait_for(&mut h, "●    5");

    // The dialog sets a condition and a hit count.
    h.state_mut()
        .dbg()
        .act(crate::panels::Action::EditBp(path.clone(), 5));
    h.step();
    {
        let e = h.state_mut().dbg().bp_edit.as_mut().expect("dialog open");
        e.condition = "i = 2".into();
        e.hit = "3".into();
    }
    h.step();
    h.get_by_label("OK").click();
    h.step();
    h.state_mut().focus(crate::app::Tab::Breakpoints);
    wait_for(&mut h, "if i = 2");
    wait_for(&mut h, "hit 3");

    // Composites expand lazily, arrays by page.
    h.get_by_label_contains("rec = ").click();
    wait_for(&mut h, "b = 2");
    h.get_by_label_contains("arr = ").click();
    wait_for(&mut h, "more (100 of 150)");
    h.get_by_label_contains("more (100").click_accesskit();
    let end = Instant::now() + Duration::from_secs(10);
    while h.state_mut().dbg().vars.children.get(&101).map(Vec::len) != Some(150) {
        h.step();
        assert!(
            Instant::now() < end,
            "second page never arrived: {:?}",
            h.state_mut().dbg().vars.children.get(&101).map(Vec::len)
        );
        thread::sleep(Duration::from_millis(10));
    }

    // Hover and watch evaluate in the selected frame; a local can be written.
    {
        let d = h.state_mut().dbg();
        d.act(crate::panels::Action::Hover("i".into()));
        d.act(crate::panels::Action::AddWatch("cnt".into()));
        d.act(crate::panels::Action::SetVar(5, "i".into(), "7".into()));
    }
    wait_for(&mut h, "stopped: breakpoint");
    let end = Instant::now() + Duration::from_secs(10);
    while h.state_mut().dbg().hover.get("i").map(String::as_str) != Some("42") {
        h.step();
        assert!(Instant::now() < end, "hover never answered");
        thread::sleep(Duration::from_millis(10));
    }
    h.state_mut().focus(crate::app::Tab::Watch);
    wait_for(&mut h, "cnt = 42");

    // Step over: the status, time and current line move.
    h.get_by_label("Over").click();
    wait_for(&mut h, "stopped: step at 10 ns, delta 3");
    wait_for(&mut h, "time 10 ns  delta 3");
    wait_for(&mut h, "▶    4");

    h.get_by_label("Stop").click();
    h.state_mut().focus(crate::app::Tab::Console);
    wait_for(&mut h, "session ended");
    drop(h);
    let seen = server.join().unwrap();
    let _ = std::fs::remove_file(file);

    let bps = sent(&seen, "setBreakpoints");
    assert_eq!(
        bps[0],
        &json!({"source": {"path": path}, "breakpoints": [{"line": 4}]})
    );
    assert_eq!(
        bps[1],
        &json!({"source": {"path": path},
                "breakpoints": [{"line": 5, "condition": "i = 2", "hitCondition": "3"}]})
    );
    let vars = sent(&seen, "variables");
    let paged: Vec<_> = vars
        .iter()
        .filter(|a| a["variablesReference"] == 101)
        .collect();
    assert_eq!(
        paged[0],
        &&json!({"variablesReference": 101, "start": 0, "count": 100})
    );
    assert_eq!(
        paged[1],
        &&json!({"variablesReference": 101, "start": 100, "count": 100})
    );
    assert_eq!(sent(&seen, "next"), [&json!({"threadId": 1})]);
    assert_eq!(
        sent(&seen, "setVariable"),
        [&json!({"variablesReference": 5, "name": "i", "value": "7"})]
    );
    let evals = sent(&seen, "evaluate");
    assert!(evals.contains(&&json!({"expression": "i", "frameId": 65536, "context": "hover"})));
    assert!(evals.contains(&&json!({"expression": "cnt", "frameId": 65536, "context": "watch"})));
    // Every process got a stack request after each stop.
    assert_eq!(sent(&seen, "stackTrace").len(), 4);
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

/// Analyze the editor sample; returns the yavs binary, the `--work` argument and the source.
fn analyze_sample(tag: &str) -> (std::path::PathBuf, String, std::path::PathBuf) {
    let yavs = std::path::PathBuf::from(std::env::var("YAVS_BIN").expect("YAVS_BIN"));
    let work = std::env::temp_dir().join(format!("yavs-gui-{tag}-{}", std::process::id()));
    let tb = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("editors/vscode/sample/tb.vhd");
    let lib = format!("--work=work:{}", work.display());
    let ok = std::process::Command::new(&yavs)
        .args(["analyze", &lib])
        .arg(&tb)
        .status()
        .unwrap();
    assert!(ok.success());
    (yavs, lib, tb)
}

/// The real simulator on the editor sample: `YAVS_BIN=<path to yavs> cargo test -- --ignored`.
#[test]
#[ignore = "needs YAVS_BIN, a built yavs"]
fn launch_real_yavs() {
    let (yavs, lib, _) = analyze_sample("run");
    let target = Target::Launch {
        yavs: Some(yavs),
        args: vec![lib, "tb".into()],
    };
    let app = App::new(&target, Arc::new(|| {}));
    let mut h = Harness::builder().build_ui_state(|ui, app: &mut App| app.show(ui), app);
    wait_for(&mut h, "done");
    wait_for(&mut h, "session ended: exited with code 0");
}

/// Breakpoint, locals, step over and in, call stack, run to the end on the real simulator.
#[test]
#[ignore = "needs YAVS_BIN, a built yavs"]
fn debug_real_yavs() {
    let (yavs, lib, tb) = analyze_sample("debug");
    let path = tb.to_str().unwrap().to_string();
    let target = Target::Launch {
        yavs: Some(yavs),
        args: vec![lib, "tb".into()],
    };
    let mut app = App::new(&target, Arc::new(|| {}));
    // Line 20 is `i := twice(k);`; the breakpoint is sent before the run starts.
    app.dbg().bps.toggle(&path, 20);
    let mut h = Harness::builder()
        .with_size([1400.0, 900.0])
        .build_ui_state(|ui, app: &mut App| app.show(ui), app);
    wait_for(&mut h, "stopped: breakpoint at 0 fs, delta 0");
    wait_for(&mut h, "time 0 fs  delta 0");
    wait_for(&mut h, "▶   20");
    wait_for(&mut h, "i = 0");
    h.get_by_label("Over").click();
    wait_for(&mut h, "▶   21");
    wait_for(&mut h, "i = 2");
    // Back to line 20 in the next iteration after 10 ns, then into the function.
    h.get_by_label("Run").click();
    wait_for(&mut h, "time 10 ns");
    wait_for(&mut h, "▶   20");
    h.get_by_label("In").click();
    wait_for(&mut h, "▶    9");
    wait_for(&mut h, "x = 2");
    wait_for(&mut h, "twice  tb.vhd:9");
    wait_for(&mut h, "p  tb.vhd:20");
    // Remove the breakpoint and run to the end.
    // (line 20 is scrolled out of the small source panel)
    h.state_mut()
        .dbg()
        .act(crate::panels::Action::ToggleBp(path, 20));
    h.get_by_label("Run").click();
    h.state_mut().focus(crate::app::Tab::Console);
    wait_for(&mut h, "done");
    wait_for(&mut h, "session ended: exited with code 0");
}
