//! Each clock-edge idiom lowers to one `Edge` instruction of the right kind (#632), and the
//! look-alikes do not (`YAVS_DUMPIR`). Behaviour is `edge_idioms.rs`; this pins the shape the
//! interpreter, the compiled tier and any other backend rely on.

mod common;
use common::tempdir;

use std::process::Command;

const SRC: &str = r#"library ieee; use ieee.std_logic_1164.all;
entity e is end;
architecture a of e is
  signal clk, other : std_logic := '0';
  signal cb : bit := '0';
  signal cbool : boolean := false;
  signal v : std_logic_vector(1 downto 0) := "00";
  signal ck : integer := 0;
  signal q : std_logic := '0';
  function rising_edge(signal s : integer) return boolean is begin return s'event; end;
  procedure wait_clk(signal s : std_logic) is begin wait until s = '1'; end;
begin
  rise:   process (clk) begin if rising_edge(clk) then q <= not q; end if; end process;
  fall:   process (clk) begin if falling_edge(clk) then q <= not q; end if; end process;
  ev1:    process (clk) begin if clk'event and clk = '1' then q <= not q; end if; end process;
  ev1r:   process (clk) begin if clk = '1' and clk'event then q <= not q; end if; end process;
  ev0:    process (clk) begin if clk'event and clk = '0' then q <= not q; end if; end process;
  nstab:  process (clk) begin if not clk'stable and clk = '1' then q <= not q; end if; end process;
  elem:   process (v) begin if v(1)'event and v(1) = '1' then q <= not q; end if; end process;
  ribit:  process (cb) begin if rising_edge(cb) then q <= not q; end if; end process;
  evbit:  process (cb) begin if cb'event and cb = '0' then q <= not q; end if; end process;
  ribool: process (cbool) begin if rising_edge(cbool) then q <= not q; end if; end process;
  evbool: process (cbool) begin if cbool'event and cbool then q <= not q; end if; end process;
  evnot:  process (cbool) begin if cbool'event and not cbool then q <= not q; end if; end process;
  w1:     process begin wait until clk = '1'; q <= not q; end process;
  w0:     process begin wait until clk = '0'; q <= not q; end process;
  wrise:  process begin wait until rising_edge(clk); q <= not q; end process;
  wev:    process begin wait until clk'event and clk = '1'; q <= not q; end process;
  both:   process (clk, other) begin if rising_edge(clk) and other = '1' then q <= not q; end if; end process;
  level:  process (clk) begin if clk = '1' then q <= not q; end if; end process;
  evonly: process (clk) begin if clk'event then q <= not q; end if; end process;
  twosig: process (clk, other) begin if clk'event and other = '1' then q <= not q; end if; end process;
  user:   process (ck) begin if rising_edge(ck) then q <= not q; end if; end process;
  wtime:  process begin wait until clk = '1' for 10 ns; q <= not q; end process;
  wlist:  process begin wait on clk, other until clk = '1'; q <= not q; end process;
  wpost:  postponed process variable c : natural := 0; begin wait until clk = '1'; c := c + 1; end process;
  wproc:  process begin wait_clk(clk); q <= not q; end process;
  stab5:  process (clk) begin if not clk'stable(5 ns) and clk = '1' then q <= not q; end if; end process;
  clkgen: process begin wait for 5 ns; clk <= not clk; cb <= not cb; cbool <= not cbool; v <= not v; ck <= ck + 1; if now > 30 ns then wait; end if; end process;
end;
"#;

/// (process label, the kind of its one `Edge`, or none).
const WANT: &[(&str, Option<&str>)] = &[
    ("rise", Some("Rising")),
    ("fall", Some("Falling")),
    ("ev1", Some("To1")),
    ("ev1r", Some("To1")),
    ("ev0", Some("To0")),
    ("nstab", Some("To1")),
    ("elem", Some("To1")),
    ("ribit", Some("Rising")),
    ("evbit", Some("To0")),
    ("ribool", Some("Rising")),
    ("evbool", Some("To1")),
    ("evnot", Some("To0")),
    ("w1", Some("To1")),
    ("w0", Some("To0")),
    ("wrise", Some("Rising")),
    ("wev", Some("To1")),
    ("both", Some("Rising")),
    ("level", None),
    ("evonly", None),
    ("twosig", None),
    ("user", None),
    ("wtime", None),
    ("wlist", None),
    ("wpost", None),
    ("wproc", None),
    ("stab5", None),
];

#[test]
fn every_edge_idiom_is_one_edge_instruction() {
    let dir = tempdir("edge_ir");
    std::fs::write(dir.join("e.vhd"), SRC).expect("write");
    let yavs = env!("CARGO_BIN_EXE_yavs");
    let a = Command::new(yavs)
        .current_dir(&dir)
        .args(["analyze", "--std=2008", "--work=work:work", "e.vhd"])
        .output()
        .expect("analyze");
    assert!(a.status.success(), "{}", String::from_utf8_lossy(&a.stderr));
    let o = Command::new(yavs)
        .current_dir(&dir)
        .env("YAVS_DUMPIR", "1")
        .args([
            "run",
            "--no-cache",
            "--work=work:work",
            "--tier=interp",
            "e",
        ])
        .args(["--stop-time=100ns"])
        .output()
        .expect("run");
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let err = String::from_utf8_lossy(&o.stderr);
    // The dump of a function starts at a line `f<N> Process `label` ...`.
    let mut dumps: Vec<(&str, String)> = Vec::new();
    for l in err.lines() {
        if let Some(rest) = l.strip_prefix('f')
            && let Some((_, name)) = rest.split_once(" Process `")
        {
            dumps.push((name.split('`').next().unwrap_or(""), String::new()));
        } else if let Some((_, text)) = dumps.last_mut() {
            text.push_str(l);
            text.push('\n');
        }
    }
    for &(label, want) in WANT {
        let text = &dumps
            .iter()
            .find(|(n, _)| *n == label)
            .unwrap_or_else(|| panic!("no dump of process {label}"))
            .1;
        let edges: Vec<&str> = text.lines().filter(|l| l.contains(" Edge {")).collect();
        match want {
            Some(kind) => {
                assert_eq!(edges.len(), 1, "{label}: {text}");
                assert!(
                    edges[0].contains(&format!("kind: {kind},")),
                    "{label}: {text}"
                );
                assert!(!text.contains("SigAttr"), "{label}: {text}");
            }
            None => assert!(edges.is_empty(), "{label}: {text}"),
        }
    }
}
