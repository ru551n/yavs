//! The run-time LRM checks: the files of `tests/verilog/lrm` with `// output:` or `// elab:`
//! header lines are elaborated (the last module of the file, no ports) and run in a kernel of
//! their own. `// output: TEXT` lines are the exact output of the run, in order; `// elab: CODE`
//! is the one error elaboration must give. The analysis side of these files (`// expect:`) is
//! `lrm.rs`'s.
//!
//! A module without ports is the test's own top: the command line takes only VHDL top levels.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use yavs_kernel::{Builder, Config, Hook, Observer, ProcFlags, ProcId, Time};
use yavs_verilog::{Design, Instance, Options, ast, read_unit, setup};

fn dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/verilog/lrm")
}

#[derive(Clone, Default)]
struct Out(Arc<Mutex<Vec<u8>>>);

impl Observer for Out {
    fn output(&mut self, _h: &mut Hook<'_>, _p: ProcId, bytes: &[u8]) {
        self.0.lock().unwrap().extend_from_slice(bytes);
    }
}

/// The design of the last module of `path`, or the codes of the elaboration errors.
fn design(path: &Path) -> Result<Design, Vec<String>> {
    let src = read_unit(path, &setup(&Options::default())).expect("a setup");
    assert!(
        src.diagnostics
            .iter()
            .all(|d| d.severity != yavs_verilog::Severity::Error),
        "{}: {:?}",
        path.display(),
        src.diagnostics
    );
    let m = src
        .decls
        .iter()
        .rev()
        .find_map(|d| match d {
            ast::Decl::Module(m) => Some(m),
            _ => None,
        })
        .expect("a module");
    let mut design = Design::default();
    design
        .add(&Instance {
            path: "top",
            source: &src,
            module: m,
            generics: &[],
            ports: &[],
        })
        .map_err(|ds| ds.iter().map(|d| d.code.to_string()).collect::<Vec<_>>())?;
    Ok(design)
}

/// Runs the last module of `path`: its output, or the codes of the elaboration errors.
pub fn run(path: &Path) -> Result<String, Vec<String>> {
    let design = design(path)?;
    let mut b = Builder::new();
    let d = design.domain(&|_| unreachable!("no ports"), &[]);
    b.process("top.verilog", ProcFlags::default(), &[], d);
    let mut sim = b.build(Config::default()).unwrap();
    let out = Out::default();
    sim.subscribe(Box::new(out.clone()));
    match sim.run_until(Time::ns(1_000_000)) {
        Ok(_) => {}
        Err(e) => return Err(vec![format!("run: {}", e.kind)]),
    }
    let text = String::from_utf8_lossy(&out.0.lock().unwrap()).into_owned();
    Ok(text)
}

#[test]
fn every_run_check_gives_its_output() {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir())
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "v"))
        .collect();
    files.sort();
    let mut bad = Vec::new();
    let mut n = 0;
    for f in &files {
        let text = std::fs::read_to_string(f).unwrap();
        let head: Vec<&str> = text.lines().take_while(|l| l.starts_with("//")).collect();
        let want: Vec<&str> = head
            .iter()
            .filter_map(|l| l.strip_prefix("// output:"))
            .map(|l| l.strip_prefix(' ').unwrap_or(l))
            .collect();
        let elab: Vec<&str> = head
            .iter()
            .filter_map(|l| l.strip_prefix("// elab: "))
            .collect();
        if want.is_empty() && elab.is_empty() {
            continue;
        }
        n += 1;
        let name = f.file_name().unwrap().to_string_lossy().into_owned();
        match (run(f), elab.is_empty()) {
            (Ok(got), true) => {
                let got: Vec<&str> = got.lines().collect();
                if got != want {
                    bad.push(format!("{name}: expected {want:#?}\ngot {got:#?}"));
                }
            }
            (Err(codes), false) => {
                if codes != elab {
                    bad.push(format!(
                        "{name}: expected the errors {elab:?}, got {codes:?}"
                    ));
                }
            }
            (Ok(got), false) => bad.push(format!("{name}: expected {elab:?}, it ran: {got}")),
            (Err(codes), true) => bad.push(format!("{name}: {codes:?}")),
        }
    }
    assert!(n >= 10, "only {n} run checks");
    assert!(
        bad.is_empty(),
        "{} failures:\n{}",
        bad.len(),
        bad.join("\n")
    );
}

/// Nets with one driver, drivers on disjoint bits and aliases get no driver net, so their writes
/// never reach resolution; a three-state bus gets one per driver.
#[test]
fn only_resolved_nets_get_driver_nets() {
    let d = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("run-drv-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    let count = |name: &str, text: &str| {
        let f = d.join(name);
        std::fs::write(&f, text).unwrap();
        design(&f).unwrap().driver_nets()
    };
    let plain = "module m;
  reg a, b;
  wire [1:0] w;
  wire c, e;
  assign w[0] = a;
  assign w[1] = b;
  assign c = a & b;
  assign e = c;
endmodule
";
    assert_eq!(count("plain.v", plain), 0);
    let bus = "module m;
  reg a, en;
  wire y;
  tri1 p;
  assign y = en ? a : 1'bz;
  assign y = en ? 1'bz : a;
  assign p = en ? a : 1'bz;
endmodule
";
    assert_eq!(count("bus.v", bus), 3);
    std::fs::remove_dir_all(&d).unwrap();
}
