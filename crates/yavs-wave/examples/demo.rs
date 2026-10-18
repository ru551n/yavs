//! Writes a demo waveform: `cargo run -p yavs-wave --example demo -- out.fst`

use yavs_wave::*;

fn main() -> Result<(), Error> {
    let path = std::env::args().nth(1).unwrap_or_else(|| "demo.fst".into());
    let slogic = EnumType::std_logic();
    let state = EnumType::new("state_t", &["IDLE", "LOAD", "RUN", "DONE"]);
    let mut b = HierarchyBuilder::new(Selection::default());
    b.scope("tb", ScopeKind::Instance, "tb(sim)");
    b.signal(
        "clk",
        SignalId(0),
        SigType::Scalar(ScalarType::Enum(slogic.clone())),
        PortDir::None,
        ObjClass::Signal,
    );
    b.signal(
        "data",
        SignalId(1),
        SigType::Vector {
            elem: slogic,
            bounds: Bounds {
                left: 7,
                right: 0,
                dir: Dir::Downto,
            },
            type_name: None,
        },
        PortDir::None,
        ObjClass::Signal,
    );
    b.signal(
        "state",
        SignalId(2),
        SigType::Scalar(ScalarType::Enum(state)),
        PortDir::None,
        ObjClass::Signal,
    );
    b.signal(
        "count",
        SignalId(3),
        SigType::Scalar(ScalarType::Integer {
            name: "integer".into(),
            bits: 32,
        }),
        PortDir::None,
        ObjClass::Signal,
    );
    b.signal(
        "level",
        SignalId(4),
        SigType::Scalar(ScalarType::Real),
        PortDir::None,
        ObjClass::Signal,
    );
    b.up();
    let h = b.finish();

    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = stop.clone();
    interrupt::install(move || flag.store(true, std::sync::atomic::Ordering::SeqCst))
        .expect("signal handler");
    let mut w = WaveWriter::create(&path, &h, WaveConfig::default())?;
    // std_ulogic positions: U X 0 1 Z W L H -
    w.change(0, SignalId(0), &[2])?;
    w.change(0, SignalId(1), &[0; 8])?; // all 'U'
    w.change(0, SignalId(2), &0u32.to_le_bytes())?;
    w.change(0, SignalId(3), &0i64.to_le_bytes())?;
    w.change(0, SignalId(4), &0.0f64.to_le_bytes())?;
    for cycle in 1..=64u64 {
        if stop.load(std::sync::atomic::Ordering::SeqCst) {
            break;
        }
        let t = cycle * 5_000_000; // 5 ns in fs
        w.change(t, SignalId(0), &[if cycle % 2 == 1 { 3 } else { 2 }])?;
        if cycle % 2 == 1 {
            let v = cycle / 2;
            let bits: Vec<u8> = (0..8)
                .rev()
                .map(|i| if (v >> i) & 1 == 1 { 3 } else { 2 })
                .collect();
            w.change(t, SignalId(1), &bits)?;
            w.change(t, SignalId(2), &(((cycle / 16) % 4) as u32).to_le_bytes())?;
            w.change(t, SignalId(3), &(v as i64 - 10).to_le_bytes())?;
            w.change(t, SignalId(4), &((v as f64) / 4.0).to_le_bytes())?;
        }
    }
    w.finish()
}
