//! Nibbles vs bit-planes at widths 1..65536. Run: `cargo bench -p yavs-logic`.

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use std::hint::black_box;
use std::time::Duration;
use yavs_logic::{ALL, Logic, Nibbles, Planes, Repr, StdULogic};

const WIDTHS: [usize; 6] = [1, 8, 32, 64, 1024, 65536];

fn rng(seed: &mut u64) -> usize {
    *seed ^= *seed << 13;
    *seed ^= *seed >> 7;
    *seed ^= *seed << 17;
    *seed as usize
}

/// Random vector over all nine values (`full`) or only `0 1`.
fn rand_vec<R: Repr>(n: usize, seed: u64, full: bool) -> Logic<R> {
    let mut s = seed;
    let v: Vec<StdULogic> = (0..n)
        .map(|_| {
            if full {
                ALL[rng(&mut s) % 9]
            } else {
                ALL[2 + rng(&mut s) % 2]
            }
        })
        .collect();
    Logic::from_fn(n, |i| v[i])
}

fn run<R: Repr>(c: &mut Criterion, layout: &str) {
    let mut g = c.benchmark_group("logic");
    g.sample_size(20)
        .warm_up_time(Duration::from_millis(200))
        .measurement_time(Duration::from_millis(600));
    for n in WIDTHS {
        let id = |op: &str| BenchmarkId::new(format!("{op}/{layout}"), n);
        g.throughput(Throughput::Elements(n as u64));
        let (a, b) = (rand_vec::<R>(n, 1, true), rand_vec::<R>(n, 2, true));
        let (c3, d4) = (rand_vec::<R>(n, 3, true), rand_vec::<R>(n, 4, true));
        let (a01, b01) = (rand_vec::<R>(n, 5, false), rand_vec::<R>(n, 6, false));
        let mut m = a.clone();
        let mut seed = 9;
        g.bench_function(id("get_set"), |bn| {
            bn.iter(|| {
                let i = rng(&mut seed) % n;
                let v = m.get(black_box(i));
                m.set(
                    i,
                    if v == StdULogic::One {
                        StdULogic::Zero
                    } else {
                        StdULogic::One
                    },
                );
            })
        });
        g.bench_function(id("and"), |bn| bn.iter(|| black_box(&a).and(black_box(&b))));
        g.bench_function(id("or"), |bn| bn.iter(|| black_box(&a).or(black_box(&b))));
        g.bench_function(id("xor"), |bn| bn.iter(|| black_box(&a).xor(black_box(&b))));
        g.bench_function(id("resolve2"), |bn| {
            bn.iter(|| Logic::resolve(black_box(&[&a, &b])))
        });
        g.bench_function(id("resolve4"), |bn| {
            bn.iter(|| Logic::resolve(black_box(&[&a, &b, &c3, &d4])))
        });
        g.bench_function(id("add"), |bn| {
            bn.iter(|| black_box(&a01).add_num(black_box(&b01), false, false))
        });
        g.bench_function(id("compare"), |bn| {
            bn.iter(|| black_box(&a01).cmp_num(black_box(&b01), false))
        });
        g.bench_function(id("to_string"), |bn| bn.iter(|| black_box(&a).to_string()));
    }
    g.finish();
}

fn all(c: &mut Criterion) {
    run::<Nibbles>(c, "nibbles");
    run::<Planes>(c, "planes");
}

criterion_group!(benches, all);
criterion_main!(benches);
