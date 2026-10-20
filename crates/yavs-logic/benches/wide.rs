//! The block-slice kernels of `yavs_logic::wide` against the per-block pairwise loops they
//! replaced. Run: `cargo bench -p yavs-logic --bench wide`.

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use std::hint::black_box;
use std::time::Duration;
use yavs_logic::wide::{self, Z_BLOCK, chunks, get_block, put_block};
use yavs_logic::{Block, Planes};

fn rng(seed: &mut u64) -> u64 {
    *seed ^= *seed << 13;
    *seed ^= *seed >> 7;
    *seed ^= *seed << 17;
    *seed
}

/// Random blocks over the nine values (codes 0 to 8).
fn rand_blocks(n: usize, seed: u64) -> Vec<Block> {
    let mut s = seed;
    let mut b = vec![[0u64; 4]; n.div_ceil(64)];
    for i in 0..n {
        let c = rng(&mut s) % 9;
        for (k, plane) in b[i / 64].iter_mut().enumerate() {
            *plane |= ((c >> k) & 1) << (i % 64);
        }
    }
    b
}

/// The loop `resolve_into` replaced: pairwise `resolve_block` per chunk.
fn resolve_pairwise(dst: &mut [Block], doff: usize, srcs: &[(&[Block], usize)], n: usize) {
    for (c, k) in chunks(n) {
        let acc = srcs
            .iter()
            .map(|(s, so)| get_block(s, so + c, k))
            .reduce(Planes::resolve_block)
            .unwrap_or(Z_BLOCK);
        put_block(dst, doff + c, k, acc, !0);
    }
}

fn all(c: &mut Criterion) {
    let mut g = c.benchmark_group("wide");
    g.sample_size(20)
        .warm_up_time(Duration::from_millis(200))
        .measurement_time(Duration::from_millis(600));
    for (n, nsrc) in [
        (32usize, 32usize),
        (64, 4),
        (1024, 4),
        (65536, 2),
        (65536, 8),
    ] {
        let srcs: Vec<Vec<Block>> = (0..nsrc)
            .map(|i| rand_blocks(n + 64, 1 + i as u64))
            .collect();
        let args: Vec<(&[Block], usize)> = srcs.iter().map(|b| (b.as_slice(), 0)).collect();
        let off: Vec<(&[Block], usize)> = srcs.iter().map(|b| (b.as_slice(), 5)).collect();
        let mut dst = rand_blocks(n + 64, 99);
        let (args, off) = (black_box(args), black_box(off));
        let id = |op: &str| BenchmarkId::new(op, format!("{n}x{nsrc}"));
        g.throughput(Throughput::Elements((n * nsrc) as u64));
        g.bench_function(id("resolve/pairwise"), |bn| {
            bn.iter(|| resolve_pairwise(&mut dst, 0, black_box(&args), n))
        });
        g.bench_function(id("resolve/wide"), |bn| {
            bn.iter(|| wide::resolve_into(&mut dst, 0, args.len(), |i| args[i], n))
        });
        g.bench_function(id("resolve/blockwise"), |bn| {
            bn.iter(|| {
                for (j, d) in dst.iter_mut().enumerate().take(n.div_ceil(64)) {
                    *d = args
                        .iter()
                        .map(|(s, _)| s[j])
                        .reduce(Planes::resolve_block)
                        .unwrap_or(Z_BLOCK);
                }
            })
        });
        g.bench_function(id("resolve/pairwise_unaligned"), |bn| {
            bn.iter(|| resolve_pairwise(&mut dst, 3, black_box(&off), n))
        });
        g.bench_function(id("resolve/wide_unaligned"), |bn| {
            bn.iter(|| wide::resolve_into(&mut dst, 3, off.len(), |i| off[i], n))
        });
    }
    for n in [64usize, 1024, 65536] {
        let (a, b) = (rand_blocks(n + 64, 3), rand_blocks(n + 64, 4));
        let mut d = a.clone();
        let id = |op: &str| BenchmarkId::new(op, n);
        g.throughput(Throughput::Elements(n as u64));
        g.bench_function(id("copy/aligned"), |bn| {
            bn.iter(|| wide::copy_range(&mut d, 0, black_box(&b), 0, n))
        });
        g.bench_function(id("copy/unaligned"), |bn| {
            bn.iter(|| wide::copy_range(&mut d, 3, black_box(&b), 7, n))
        });
        g.bench_function(id("eq/equal"), |bn| {
            bn.iter(|| wide::eq_range(black_box(&d), 3, black_box(&d), 3, n))
        });
        g.bench_function(id("has_x"), |bn| bn.iter(|| wide::has_x(black_box(&a), n)));
        g.bench_function(id("match_eq"), |bn| {
            bn.iter(|| wide::match_eq(black_box(&a), 0, black_box(&b), 0, n))
        });
    }
    g.finish();
}

criterion_group!(benches, all);
criterion_main!(benches);
