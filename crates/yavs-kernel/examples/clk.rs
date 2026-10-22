//! Kernel cost of clock generators: `c <= not c after T` as `ToggleProcess`es, so the count
//! has no interpreter in it. Run under `perf stat -e instructions:u`.
//!
//! `cargo run --release --example clk -- [clocks] [simulated microseconds]`

use yavs_kernel::*;
use yavs_logic::StdULogic;

fn main() -> Result<(), SimError> {
    let args: Vec<u64> = std::env::args()
        .skip(1)
        .map(|a| a.parse().expect("numeric argument"))
        .collect();
    let clocks = args.first().copied().unwrap_or(3) as u32;
    let us = args.get(1).copied().unwrap_or(6000);
    let mut b = Builder::new();
    for k in 0..clocks {
        let c = b.signal(
            &format!("c{k}"),
            SignalType::logic(1),
            &LogicVec::new(1, StdULogic::Zero),
        );
        let d = b.driver(c);
        let half = Time::ns(4 + u64::from(k));
        b.process_boxed(
            &format!("gen{k}"),
            ProcFlags::default(),
            &[d],
            Box::new(ToggleProcess::new(c, 0, d, half, DelayMode::INERTIAL)),
        );
    }
    let mut sim = b.build(Config::default())?;
    sim.run_until(Time::ns(us * 1000))?;
    let st = sim.stats();
    println!("{} cycles, {} process runs", st.cycles, st.process_runs);
    Ok(())
}
