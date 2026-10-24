//! Write FST files with yavs-wave and read them back with wellen.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use wellen::simple::{Waveform, read};
use wellen::{ScopeType, VarDirection};
use yavs_wave::*;

fn sl(c: char) -> u8 {
    "UX01ZWLH-".find(c).unwrap() as u8
}

fn int(v: i64) -> [u8; 8] {
    v.to_le_bytes()
}

/// (time, value) for every change of the variable at `path`/`name`.
fn changes(w: &mut Waveform, path: &[&str], name: &str) -> Vec<(u64, String)> {
    let h = w.hierarchy();
    let var = h
        .lookup_var(path, name)
        .unwrap_or_else(|| panic!("no var {path:?} {name}"));
    let sr = h[var].signal_ref();
    w.load_signals(&[sr]);
    let tt = w.time_table().to_vec();
    w.get_signal(sr)
        .unwrap()
        .iter_changes()
        .map(|(t, v)| {
            (
                tt[t as usize],
                v.to_bit_string().unwrap_or_else(|| v.to_string()),
            )
        })
        .collect()
}

fn ch(v: &[(u64, &str)]) -> Vec<(u64, String)> {
    v.iter().map(|(t, s)| (*t, s.to_string())).collect()
}

struct Ids {
    clk: SignalId,
    data: SignalId,
    rev: SignalId,
    state: SignalId,
    flag: SignalId,
    count: SignalId,
    ratio: SignalId,
    when: SignalId,
    rec_a: SignalId,
    rec_b: SignalId,
    e0_x: SignalId,
    e1_x: SignalId,
}

fn build(sel: Selection) -> (Hierarchy, Ids) {
    let slogic = EnumType::std_logic();
    let state_t = EnumType::new("state_t", &["IDLE", "RUN", "Done_State", "weird name"]);
    let boolean = EnumType::boolean();
    let n = |i| SignalId(i);
    let ids = Ids {
        clk: n(0),
        data: n(1),
        rev: n(2),
        state: n(3),
        flag: n(4),
        count: n(5),
        ratio: n(6),
        when: n(7),
        rec_a: n(8),
        rec_b: n(9),
        e0_x: n(10),
        e1_x: n(11),
    };
    let logic = |e: &Arc<EnumType>| SigType::Scalar(ScalarType::Enum(e.clone()));
    let mut b = HierarchyBuilder::new(sel);
    b.scope("Top", ScopeKind::Instance, "tb(rtl)");
    b.signal(
        "Clk",
        ids.clk,
        logic(&slogic),
        PortDir::In,
        ObjClass::Signal,
    );
    b.signal(
        "data",
        ids.data,
        SigType::Vector {
            elem: slogic.clone(),
            bounds: Bounds {
                left: 7,
                right: 0,
                dir: Dir::Downto,
            },
            type_name: None,
        },
        PortDir::Out,
        ObjClass::Signal,
    );
    b.signal(
        "rev",
        ids.rev,
        SigType::Vector {
            elem: slogic.clone(),
            bounds: Bounds {
                left: 0,
                right: 3,
                dir: Dir::To,
            },
            type_name: Some("unsigned".into()),
        },
        PortDir::None,
        ObjClass::Signal,
    );
    b.signal(
        "state",
        ids.state,
        logic(&state_t),
        PortDir::None,
        ObjClass::Signal,
    );
    b.signal(
        "flag",
        ids.flag,
        logic(&boolean),
        PortDir::None,
        ObjClass::Signal,
    );
    b.signal(
        "count",
        ids.count,
        SigType::Scalar(ScalarType::Integer {
            name: "integer".into(),
            bits: 32,
        }),
        PortDir::None,
        ObjClass::Signal,
    );
    b.signal(
        "ratio",
        ids.ratio,
        SigType::Scalar(ScalarType::Real),
        PortDir::None,
        ObjClass::Signal,
    );
    b.signal(
        "when_t",
        ids.when,
        SigType::Scalar(ScalarType::Time),
        PortDir::None,
        ObjClass::Signal,
    );
    b.scope("rec", ScopeKind::Record, "");
    b.signal(
        "a",
        ids.rec_a,
        logic(&slogic),
        PortDir::None,
        ObjClass::Signal,
    );
    b.signal(
        "b",
        ids.rec_b,
        SigType::Scalar(ScalarType::Integer {
            name: "natural".into(),
            bits: 31,
        }),
        PortDir::None,
        ObjClass::Signal,
    );
    b.up();
    // array of records
    if b.array("arr", 2) {
        for (i, id) in [(0, ids.e0_x), (1, ids.e1_x)] {
            b.scope(&format!("arr({i})"), ScopeKind::Record, "");
            b.signal("x", id, logic(&slogic), PortDir::None, ObjClass::Signal);
            b.up();
        }
    }
    b.up();
    b.scope("proc", ScopeKind::Process, "");
    b.signal(
        "v",
        SignalId(20),
        logic(&slogic),
        PortDir::None,
        ObjClass::Variable,
    );
    b.up();
    // an alias: the same kernel signal seen through a port
    b.scope("u1", ScopeKind::Instance, "sub");
    b.signal(
        "clk_in",
        ids.clk,
        logic(&slogic),
        PortDir::In,
        ObjClass::Signal,
    );
    b.up();
    b.up();
    (b.finish(), ids)
}

fn write_demo(path: &Path, h: &Hierarchy, ids: &Ids) {
    let mut w = WaveWriter::create(path, h, WaveConfig::default()).unwrap();
    let v = |s: &str| s.chars().map(sl).collect::<Vec<_>>();
    w.change(0, ids.clk, &[sl('U')]).unwrap();
    w.change(0, ids.data, &v("UUUUUUUU")).unwrap();
    w.change(0, ids.rev, &v("0000")).unwrap();
    w.change(0, ids.state, &0u32.to_le_bytes()).unwrap();
    w.change(0, ids.flag, &0u32.to_le_bytes()).unwrap();
    w.change(0, ids.count, &int(0)).unwrap();
    w.change(0, ids.ratio, &0.0f64.to_le_bytes()).unwrap();
    w.change(0, ids.when, &int(0)).unwrap();
    w.change(0, ids.rec_a, &[sl('0')]).unwrap();
    w.change(0, ids.rec_b, &int(0)).unwrap();
    w.change(0, ids.e0_x, &[sl('Z')]).unwrap();
    w.change(0, ids.e1_x, &[sl('W')]).unwrap();
    w.change(0, SignalId(20), &[sl('X')]).unwrap();
    w.change(5_000, ids.clk, &[sl('0')]).unwrap();
    w.change(5_000, ids.data, &v("01XZLH-1")).unwrap();
    w.change(5_000, ids.state, &2u32.to_le_bytes()).unwrap();
    w.change(5_000, ids.count, &int(-7)).unwrap();
    w.change(10_000, ids.clk, &[sl('1')]).unwrap();
    w.change(10_000, ids.rev, &v("1010")).unwrap();
    w.change(10_000, ids.state, &3u32.to_le_bytes()).unwrap();
    w.change(10_000, ids.flag, &1u32.to_le_bytes()).unwrap();
    w.change(10_000, ids.count, &int(i32::MIN as i64)).unwrap();
    w.change(10_000, ids.ratio, &2.5f64.to_le_bytes()).unwrap();
    w.change(10_000, ids.when, &int(1_000_000_000)).unwrap();
    w.change(10_000, ids.rec_b, &int(123)).unwrap();
    w.change(10_000, SignalId(99), &[0]).unwrap(); // undeclared: ignored
    w.finish().unwrap();
}

#[test]
fn hierarchy_types_and_values_round_trip() {
    let (h, ids) = build(Selection::default());
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.fst");
    write_demo(&path, &h, &ids);
    let mut w = read(&path).unwrap();

    // hierarchy
    {
        let hi = w.hierarchy();
        let top = hi.first_scope().unwrap();
        assert_eq!(top.name(hi), "Top", "spelling is kept");
        assert_eq!(top.scope_type(), ScopeType::VhdlArchitecture);
        assert_eq!(top.component(hi), Some("tb(rtl)"));
        let scope = |p: &[&str]| {
            &hi[hi
                .lookup_scope(p)
                .unwrap_or_else(|| panic!("no scope {p:?}"))]
        };
        assert_eq!(scope(&["Top", "rec"]).scope_type(), ScopeType::VhdlRecord);
        assert_eq!(scope(&["Top", "proc"]).scope_type(), ScopeType::VhdlProcess);
        assert_eq!(scope(&["Top", "arr"]).scope_type(), ScopeType::SvArray);
        assert_eq!(
            scope(&["Top", "arr", "arr(1)"]).scope_type(),
            ScopeType::VhdlRecord
        );

        let var =
            |p: &[&str], n: &str| &hi[hi.lookup_var(p, n).unwrap_or_else(|| panic!("no var {n}"))];
        let clk = var(&["Top"], "Clk");
        assert_eq!(clk.var_type(), wellen::VarType::StdLogic);
        assert_eq!(clk.direction(), VarDirection::Input);
        assert_eq!(clk.length(hi), Some(1));
        let data = var(&["Top"], "data");
        assert_eq!(data.var_type(), wellen::VarType::StdLogicVector);
        assert_eq!(data.direction(), VarDirection::Output);
        assert_eq!(data.length(hi), Some(8));
        let ix = data.index().unwrap();
        assert_eq!((ix.msb(), ix.lsb()), (7, 0), "downto");
        let rev = var(&["Top"], "rev");
        let ix = rev.index().unwrap();
        assert_eq!((ix.msb(), ix.lsb()), (0, 3), "to keeps left first");
        assert_eq!(rev.vhdl_type_name(hi), Some("unsigned"));
        let st = var(&["Top"], "state");
        let (ename, map) = st.enum_type(hi).unwrap();
        assert_eq!(ename, "state_t");
        assert_eq!(
            map,
            [
                ("00", "IDLE"),
                ("01", "RUN"),
                ("10", "Done_State"),
                ("11", "weird\\040name")
            ]
        ); // wellen does not undo the FST octal escape of spaces
        assert_eq!(st.vhdl_type_name(hi), Some("state_t"));
        assert_eq!(var(&["Top"], "flag").var_type(), wellen::VarType::Boolean);
        let (bname, bmap) = var(&["Top"], "flag").enum_type(hi).unwrap();
        assert_eq!(
            (bname, bmap),
            ("boolean", vec![("0", "false"), ("1", "true")])
        );
        assert_eq!(var(&["Top"], "count").var_type(), wellen::VarType::Integer);
        assert_eq!(var(&["Top"], "count").length(hi), Some(32));
        assert_eq!(var(&["Top"], "ratio").var_type(), wellen::VarType::Real);
        assert_eq!(var(&["Top"], "when_t").var_type(), wellen::VarType::Time);
        assert_eq!(var(&["Top", "proc"], "v").length(hi), Some(1));
        // alias shares the signal of Top.Clk
        assert_eq!(var(&["Top", "u1"], "clk_in").signal_ref(), clk.signal_ref());
    }

    // values: std_logic keeps all nine states
    assert_eq!(
        changes(&mut w, &["Top"], "Clk"),
        ch(&[(0, "u"), (5_000, "0"), (10_000, "1")])
    );
    assert_eq!(
        changes(&mut w, &["Top"], "data"),
        ch(&[(0, "uuuuuuuu"), (5_000, "01xzlh-1")])
    );
    assert_eq!(
        changes(&mut w, &["Top"], "rev"),
        ch(&[(0, "0000"), (10_000, "1010")])
    );
    assert_eq!(
        changes(&mut w, &["Top"], "state"),
        ch(&[(0, "00"), (5_000, "10"), (10_000, "11")])
    );
    assert_eq!(
        changes(&mut w, &["Top"], "flag"),
        ch(&[(0, "0"), (10_000, "1")])
    );
    assert_eq!(
        changes(&mut w, &["Top"], "count"),
        ch(&[
            (0, "00000000000000000000000000000000"),
            (5_000, "11111111111111111111111111111001"),
            (10_000, "10000000000000000000000000000000"),
        ])
    );
    assert_eq!(
        changes(&mut w, &["Top"], "ratio"),
        ch(&[(0, "0"), (10_000, "2.5")])
    );
    assert_eq!(changes(&mut w, &["Top"], "when_t").len(), 2);
    assert_eq!(
        changes(&mut w, &["Top", "rec"], "b")[1].1,
        "0000000000000000000000001111011"
    );
    assert_eq!(
        changes(&mut w, &["Top", "arr", "arr(0)"], "x"),
        ch(&[(0, "z")])
    );
    assert_eq!(
        changes(&mut w, &["Top", "arr", "arr(1)"], "x"),
        ch(&[(0, "w")])
    );
    assert_eq!(changes(&mut w, &["Top", "proc"], "v"), ch(&[(0, "x")]));
}

#[test]
fn selection_and_array_cap() {
    let sel = Selection {
        include: vec!["top.**".into()],
        exclude: vec!["TOP.dbg".into(), "top.*.hidden".into()],
        max_array_elems: 4,
    };
    let bit = EnumType::bit();
    let sig = |_id: u32| SigType::Scalar(ScalarType::Enum(bit.clone()));
    let mut b = HierarchyBuilder::new(sel);
    b.scope("top", ScopeKind::Instance, "e");
    b.signal("keep", SignalId(0), sig(0), PortDir::None, ObjClass::Signal);
    b.scope("dbg", ScopeKind::Block, "");
    b.signal("x", SignalId(1), sig(1), PortDir::None, ObjClass::Signal);
    b.up();
    b.scope("u", ScopeKind::Block, "");
    b.signal(
        "hidden",
        SignalId(2),
        sig(2),
        PortDir::None,
        ObjClass::Signal,
    );
    b.signal("seen", SignalId(3), sig(3), PortDir::None, ObjClass::Signal);
    b.up();
    b.scope("only_hidden", ScopeKind::Block, "");
    b.signal(
        "hidden",
        SignalId(4),
        sig(4),
        PortDir::None,
        ObjClass::Signal,
    );
    b.up();
    assert!(b.array("small", 4));
    for i in 0..4 {
        b.signal(
            &format!("small({i})"),
            SignalId(10 + i),
            sig(0),
            PortDir::None,
            ObjClass::Signal,
        );
    }
    b.up();
    assert!(!b.array("mem", 1 << 20), "over the cap");
    b.signal(
        "never",
        SignalId(30),
        sig(0),
        PortDir::None,
        ObjClass::Signal,
    );
    b.up();
    b.up();
    b.scope("other", ScopeKind::Instance, "e");
    b.signal(
        "nope",
        SignalId(40),
        sig(0),
        PortDir::None,
        ObjClass::Signal,
    );
    b.up();
    let h = b.finish();
    assert_eq!(
        h.skipped,
        [Skipped {
            path: "top.mem".into(),
            elems: Some(1 << 20)
        }]
    );

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("s.fst");
    let mut w = WaveWriter::create(&path, &h, WaveConfig::default()).unwrap();
    for id in [0, 1, 2, 3, 4, 10, 30, 40] {
        w.change(0, SignalId(id), &[1]).unwrap(); // filtered ids are ignored, not errors
    }
    w.finish().unwrap();
    let wf = read(&path).unwrap();
    let hi = wf.hierarchy();
    let names: Vec<_> = hi.all_vars().map(|v| hi[v].full_name(hi)).collect();
    assert_eq!(
        names,
        [
            "top.keep",
            "top.u.seen",
            "top.small.(0)",
            "top.small.(1)",
            "top.small.(2)",
            "top.small.(3)"
        ]
    );
}

fn small_design() -> (Hierarchy, SignalId) {
    let mut b = HierarchyBuilder::new(Selection::default());
    b.scope("t", ScopeKind::Instance, "t");
    b.signal(
        "n",
        SignalId(0),
        SigType::Scalar(ScalarType::Integer {
            name: "integer".into(),
            bits: 32,
        }),
        PortDir::None,
        ObjClass::Signal,
    );
    b.up();
    (b.finish(), SignalId(0))
}

#[test]
fn killed_run_is_readable_up_to_last_flush() {
    let (h, n) = small_design();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("k.fst");
    let snap = dir.path().join("snapshot.fst");
    let mut w = WaveWriter::create(&path, &h, WaveConfig::default()).unwrap();
    for t in 0..10u64 {
        w.change(t * 10, n, &int(t as i64)).unwrap();
    }
    w.flush().unwrap();
    // The process dies here: only what is on disk now survives.
    std::fs::copy(&path, &snap).unwrap();
    for t in 10..20u64 {
        w.change(t * 10, n, &int(t as i64)).unwrap();
    }
    w.finish().unwrap();

    let mut cut = read(&snap).unwrap();
    let got = changes(&mut cut, &["t"], "n");
    assert_eq!(got.len(), 10, "{got:?}");
    assert_eq!(got.last().unwrap().0, 90);
    let mut full = read(&path).unwrap();
    assert_eq!(changes(&mut full, &["t"], "n").len(), 20);

    // truncated mid-block (cut inside the data written after the flush point)
    let bytes = std::fs::read(&path).unwrap();
    let flushed = std::fs::metadata(&snap).unwrap().len() as usize;
    assert!(bytes.len() > flushed);
    std::fs::write(&snap, &bytes[..flushed]).unwrap();
    assert!(read(&snap).is_ok());
}

#[test]
fn periodic_flush_without_calling_flush() {
    let (h, n) = small_design();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("p.fst");
    let snap = dir.path().join("snap.fst");
    let cfg = WaveConfig {
        flush_interval: Some(Duration::ZERO),
        ..WaveConfig::default()
    };
    let mut w = WaveWriter::create(&path, &h, cfg).unwrap();
    for t in 0..2000u64 {
        w.change(t, n, &int(t as i64)).unwrap();
        if t == 1500 {
            std::fs::copy(&path, &snap).unwrap();
        }
    }
    let mut cut = read(&snap).unwrap();
    let got = changes(&mut cut, &["t"], "n");
    assert!(got.len() >= 256 && got.len() <= 1501, "{}", got.len());
    for (i, (t, _)) in got.iter().enumerate() {
        assert_eq!(*t, i as u64, "no gaps or reordering");
    }
    drop(w); // finish on drop
    let mut full = read(&path).unwrap();
    assert_eq!(changes(&mut full, &["t"], "n").len(), 2000);
}

/// By default the bytes depend on the changes only, however long the run takes (#666: a cold
/// compiled run is slower than a warm one and crossed a wall-clock flush differently); a flush
/// interval is opt-in and then does split blocks by the clock.
#[test]
fn file_bytes_do_not_depend_on_the_speed_of_the_run() {
    let (h, n) = small_design();
    let dir = tempfile::tempdir().unwrap();
    let write = |name: &str, interval: Option<Duration>, slow: bool| {
        let path = dir.path().join(name);
        let mut cfg = WaveConfig::default();
        assert_eq!(cfg.flush_interval, None, "the default is reproducible");
        cfg.flush_interval = interval;
        let mut w = WaveWriter::create(&path, &h, cfg).unwrap();
        for t in 0..2000u64 {
            if slow && t == 1000 {
                std::thread::sleep(Duration::from_millis(20));
            }
            w.change(t, n, &int(t as i64)).unwrap();
        }
        drop(w);
        std::fs::read(path).unwrap()
    };
    let steady = write("a.fst", None, false);
    assert_eq!(steady, write("b.fst", None, true));
    // The clock does split blocks when a flush interval is set.
    assert_ne!(steady, write("c.fst", Some(Duration::ZERO), false));
}

#[test]
fn bad_value_length_is_an_error() {
    let (h, n) = small_design();
    let dir = tempfile::tempdir().unwrap();
    let mut w = WaveWriter::create(dir.path().join("e.fst"), &h, WaveConfig::default()).unwrap();
    assert!(w.change(0, n, &[1, 2]).is_err());
}

/// More signals than half the default block limit: the writer holds more changes per block,
/// and every value still reads back.
#[test]
fn many_signals_round_trip() {
    const N: u32 = 140_000;
    let mut b = HierarchyBuilder::new(Selection::default());
    let bit = Arc::new(SigType::Scalar(ScalarType::Enum(EnumType::bit())));
    b.scope("t", ScopeKind::Instance, "t");
    for i in 0..N {
        b.signal_shared(
            &format!("s{i}"),
            SignalId(i),
            &bit,
            PortDir::None,
            ObjClass::Signal,
        );
    }
    b.up();
    let h = b.finish();
    assert_eq!(h.signals(), N as usize);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("m.fst");
    let mut w = WaveWriter::create(&path, &h, WaveConfig::default()).unwrap();
    for t in 0..3u64 {
        for i in 0..N {
            w.change(t, SignalId(i), &[((i as u64 + t) % 2) as u8])
                .unwrap();
        }
    }
    w.finish().unwrap();
    let mut r = read(&path).unwrap();
    for i in [0u32, 1, 70_000, N - 1] {
        let want: Vec<_> = (0..3u64)
            .map(|t| (t, ((i as u64 + t) % 2).to_string()))
            .collect();
        assert_eq!(changes(&mut r, &["t"], &format!("s{i}")), want);
    }
}
