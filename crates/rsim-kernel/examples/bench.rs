//! Kernel throughput: N counters on one clock, single vs multi-thread.
//!
//! `cargo run --release --example bench -- [processes] [work] [threads]`
//! `work` is a per-activation busy loop standing in for process code.

use std::time::Instant;

use rsim_kernel::*;

fn run(n: usize, work: u32, threads: usize) -> Result<(Stats, f64), SimError> {
    let mut b = Builder::new();
    let clk = b.signal(
        "clk",
        SignalType {
            elem_size: 1,
            len: 1,
        },
        &[0],
    );
    let dclk = b.driver(clk);
    b.process(
        "clkgen",
        ProcFlags::default(),
        &[dclk],
        move |c: &mut Ctx<'_>| -> Result<(), SimError> {
            let v = c.value(clk)?[0] ^ 1;
            c.assign(dclk, None, &[(Time::ns(5), &[v])], DelayMode::INERTIAL)?;
            c.wait_for(Time::ns(5))
        },
    );
    for i in 0..n {
        let s = b.signal(
            &format!("cnt{i}"),
            SignalType {
                elem_size: 4,
                len: 1,
            },
            &[0; 4],
        );
        let d = b.driver(s);
        b.process(
            &format!("p{i}"),
            ProcFlags::default(),
            &[d],
            move |c: &mut Ctx<'_>| -> Result<(), SimError> {
                if c.event(clk)? && c.value(clk)? == [1] {
                    let mut v = u32::from_le_bytes(c.value(s)?.try_into().expect("4-byte counter"));
                    for k in 0..work {
                        v = std::hint::black_box(v.rotate_left(3) ^ k);
                    }
                    c.drive(d, &v.wrapping_add(1).to_le_bytes())?;
                }
                c.wait_on(clk)
            },
        );
    }
    let mut sim = b.build(Config {
        threads,
        ..Config::default()
    })?;
    let t0 = Instant::now();
    sim.run_until(Time::ns(10_000))?;
    Ok((sim.stats(), t0.elapsed().as_secs_f64()))
}

fn main() -> Result<(), SimError> {
    let args: Vec<usize> = std::env::args()
        .skip(1)
        .map(|a| a.parse().expect("numeric argument"))
        .collect();
    let n = args.first().copied().unwrap_or(10_000);
    let work = args.get(1).copied().unwrap_or(0) as u32;
    let max = std::thread::available_parallelism().map_or(1, |p| p.get());
    let threads = args.get(2).copied().unwrap_or(max.min(8));
    println!("{n} processes, work {work}, 10 us simulated (1000 rising edges)");
    for t in [1, threads] {
        let (st, secs) = run(n, work, t)?;
        println!(
            "threads {t:2}: {secs:7.3} s  {:10.0} events/s  {:10.0} process runs/s",
            st.events as f64 / secs,
            st.process_runs as f64 / secs
        );
    }
    Ok(())
}
