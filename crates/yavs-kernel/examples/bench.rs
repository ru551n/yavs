//! Kernel throughput: N counters on one clock, single vs multi-thread.
//!
//! `cargo run --release --example bench -- [processes] [work] [threads]`
//! `work` is a per-activation busy loop standing in for process code.

use std::time::Instant;

use yavs_kernel::*;

/// A 10 ns clock.
fn clock(b: &mut Builder) -> SignalId {
    let clk = b.signal("clk", SignalType::bytes(1, 1), &[0]);
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
    clk
}

fn run(n: usize, work: u32, threads: usize) -> Result<(Stats, f64), SimError> {
    let mut b = Builder::new();
    let clk = clock(&mut b);
    for i in 0..n {
        let s = b.signal(&format!("cnt{i}"), SignalType::bytes(4, 1), &[0; 4]);
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

/// A 1024-element std_logic bus with 4 drivers taking turns, resolved per element, for
/// 10000 rising edges. Logic signals are bit-planes with plane-wise resolution.
fn run_wide(threads: usize) -> Result<f64, SimError> {
    use yavs_logic::StdULogic;
    let mut b = Builder::new();
    let clk = clock(&mut b);
    let init = LogicVec::new(1024, StdULogic::U);
    let bus = b.resolved_signal("bus", SignalType::logic(1024), &init, Resolution::StdLogic);
    for k in 0..4u32 {
        let d = b.driver(bus);
        let mut n = 0u32;
        let z = LogicVec::new(1024, StdULogic::Z);
        b.process(
            &format!("drv{k}"),
            ProcFlags::default(),
            &[d],
            move |c: &mut Ctx<'_>| -> Result<(), SimError> {
                if c.event(clk)? && c.value(clk)? == [1] {
                    n += 1;
                    if n % 4 == k {
                        let v = LogicVec::from_fn(1024, |i| {
                            StdULogic::from_bool((i as u32 + n) % 2 == 1)
                        });
                        c.drive(d, &v)?;
                    } else {
                        c.drive(d, &z)?;
                    }
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
    sim.run_until(Time::ns(100_000))?;
    Ok(t0.elapsed().as_secs_f64())
}

/// `n` one-element std_logic signals with two drivers each, taking turns on every rising
/// edge: 2n processes and n resolutions per edge, for 1000 rising edges.
fn run_resolved(n: usize, threads: usize) -> Result<f64, SimError> {
    use yavs_logic::StdULogic;
    let mut b = Builder::new();
    let clk = clock(&mut b);
    let init = LogicVec::new(1, StdULogic::U);
    for i in 0..n {
        let s = b.resolved_signal(
            &format!("r{i}"),
            SignalType::logic(1),
            &init,
            Resolution::StdLogic,
        );
        for k in 0..2u32 {
            let d = b.driver(s);
            let mut n = 0u32;
            let vals = [StdULogic::Z, StdULogic::Zero, StdULogic::One].map(|v| LogicVec::new(1, v));
            b.process(
                &format!("r{i}_{k}"),
                ProcFlags::default(),
                &[d],
                move |c: &mut Ctx<'_>| -> Result<(), SimError> {
                    if c.event(clk)? && c.value(clk)? == [1] {
                        n += 1;
                        let v = if n % 2 == k {
                            1 + (n / 2 % 2) as usize
                        } else {
                            0
                        };
                        c.drive(d, &vals[v])?;
                    }
                    c.wait_on(clk)
                },
            );
        }
    }
    let mut sim = b.build(Config {
        threads,
        ..Config::default()
    })?;
    let t0 = Instant::now();
    sim.run_until(Time::ns(10_000))?;
    Ok(t0.elapsed().as_secs_f64())
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
    for t in [1, threads] {
        let secs = run_resolved(n / 2, t)?;
        println!(
            "{} resolved signals x 2 drivers, threads {t:2}: {secs:7.3} s",
            n / 2
        );
    }
    for t in [1, threads] {
        let secs = run_wide(t)?;
        println!(
            "wide bus 1024 x 4 drivers, threads {t:2}: {secs:7.3} s  {:10.0} resolved elements/s",
            10_000.0 * 1024.0 / secs
        );
    }
    Ok(())
}
