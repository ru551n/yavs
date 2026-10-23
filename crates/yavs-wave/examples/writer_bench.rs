//! Writer throughput: `cargo run --release -p yavs-wave --example writer_bench -- out.fst [changes]`
//! writes `changes` (default 4 million) value changes of 3000 scalars and 2000 32-bit vectors
//! (`SCALARS` and `VECTORS` in the environment change the counts).

use std::time::Instant;
use yavs_wave::*;

fn main() -> Result<(), Error> {
    let mut a = std::env::args().skip(1);
    let path = a.next().unwrap_or_else(|| "bench.fst".into());
    let total: u64 = a.next().map_or(4_000_000, |n| n.parse().expect("a count"));
    let slogic = EnumType::std_logic();
    let mut b = HierarchyBuilder::new(Selection::default());
    b.scope("tb", ScopeKind::Instance, "tb(sim)");
    let env = |k: &str, d: u32| std::env::var(k).map_or(d, |v| v.parse().expect("a count"));
    let (scalars, vectors) = (env("SCALARS", 3000), env("VECTORS", 2000));
    for i in 0..scalars {
        let ty = SigType::Scalar(ScalarType::Enum(slogic.clone()));
        b.signal(
            &format!("s{i}"),
            SignalId(i),
            ty,
            PortDir::None,
            ObjClass::Signal,
        );
    }
    for i in 0..vectors {
        let bounds = Bounds {
            left: 31,
            right: 0,
            dir: Dir::Downto,
        };
        let ty = SigType::Vector {
            elem: slogic.clone(),
            bounds,
            type_name: None,
        };
        b.signal(
            &format!("v{i}"),
            SignalId(scalars + i),
            ty,
            PortDir::None,
            ObjClass::Signal,
        );
    }
    b.up();
    let h = b.finish();
    let mut w = WaveWriter::create(&path, &h, WaveConfig::default())?;
    let (mut x, mut t) = (0x9e37_79b9_7f4a_7c15u64, 0u64);
    let mut bytes = [0u8; 32];
    let start = Instant::now();
    for n in 0..total {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        if n % 700 == 0 {
            t += 10_000_000;
        }
        let id = (x >> 20) as u32 % (scalars + vectors);
        if id < scalars {
            w.change(t, SignalId(id), &[2 + (x & 1) as u8])?;
        } else {
            for (k, v) in bytes.iter_mut().enumerate() {
                *v = 2 + (x >> k & 1) as u8;
            }
            w.change(t, SignalId(id), &bytes)?;
        }
    }
    w.finish()?;
    let s = start.elapsed().as_secs_f64();
    eprintln!(
        "{total} changes in {s:.3} s: {:.0} ns each",
        s * 1e9 / total as f64
    );
    Ok(())
}
