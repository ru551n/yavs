# AGENTS.md: tools/stepcount/ (exact instruction counts)

Read the root `AGENTS.md` and `tools/AGENTS.md` first. This file is for an agent that needs the exact
number of user instructions a function or a source line executes in a window of a run, where `perf` is
too coarse or too noisy (a kernel round that saves 20 instructions of 900 per clock edge).

## 1. Purpose and boundaries

- `step.c` is a ptrace single-stepper: it runs a program, counts every executed instruction address in a
  window and writes `address count` lines. `stepcount.py` compiles it with the system C compiler into
  `~/.cache/yavs-stepcount` (`YAVS_STEPCOUNT_CACHE`), resolves symbols with `nm` and lines with
  `addr2line`, and prints the tables. No dependency beyond binutils and a C compiler (root rule 9).
- Counts are exact and repeatable (ASLR is off for the child), unlike `perf stat`, but they are
  instructions, not cycles: a change that moves instructions must still be timed (root rule 19, tools/bench).
- x86-64 Linux only (registers, `ptrace_scope` 1 is enough: the tool starts the child itself). Only the main
  thread is traced: run with `--threads 1`. Stepping costs about 100 K instructions per second, so a window
  is a few million instructions: skip warm-up with `--from NAME:SKIP`, stop with `--max` or `--to`.
- Not a profiler of time: library code (libc `memcpy`) is counted per mapping, not per function.

## 2. How to use it

```sh
tools/build-bench.sh -p yavs-kernel --example clk          # release + aligned functions + line tables
python3 -I tools/stepcount/stepcount.py --self-test       # compiles a tiny program, checks the exact count
# the kernel cost of one clock edge: 1 clock, 20 us = 5000 edges, window from run_until to the end
python3 -I tools/stepcount/stepcount.py --from '<yavs_kernel::kernel::SimControl>::run_until' --per 5000 \
    --lines 25 $CARGO_TARGET_DIR/release/examples/clk 1 20
# a window of a real design: skip 3000 cycles, then 3 M instructions (run in the world directory)
python3 -I tools/stepcount/stepcount.py --from '<yavs_kernel::kernel::SimControl>::cycle:3000' --max 3000000 \
    --lines 30 --in update_word BIN run --std=2008 --work=work:W --tier=interp TOP
```

- `--from NAME[:SKIP]` opens the window at the (SKIP+1)-th entry to the function, `--to NAME[:N]` closes it at the
  N-th entry after that, `--max N` after N instructions; without any of them the whole run is counted. NAME is a part
  of the demangled name without the hash (`nm -C`), or `0xADDRESS`; the full name wins over a longer match, and a name
  that matches several symbols is an error that lists them (generic code has many copies: name one).
- `--ops-lines OP` (with `--ops`) adds the instructions per op of the arm named OP by source line, callees included (`step -w ARM`
  keeps the pcs of that arm); `--ops-pcs FILE` writes the relative pc and count of every instruction of it, for an annotated listing with
  `objdump -d -l`. The arm named exactly OP is taken, else the first op whose name contains OP.
- `--per N` divides every count (edges, cycles, activations). `--top N` functions are shown (self instructions, callees
  not included), `--lines N` the busiest source lines, `--in NAME` only lines of functions containing NAME.
- A line is the innermost inlined frame that is not the standard library or a registry crate, so `Vec::retain` inside
  `update_word` is attributed to the line of `update_word` that calls it. `--inner` shows the library frame itself.
- `--ops[=N]` charges every instruction after a dispatch of the interpreter loop (`yavs-interp`'s `run_ops`) up to the
  next dispatch to the op that dispatch started, callees included, and prints the N busiest ops (share of the window,
  instructions per op, ops, name); `--ops-detail OP` adds where the instructions of the ops whose name contains OP are spent,
  by function. It finds the dispatch jump (`movzbl tag; lea table; movslq; add; jmp *`) and its jump table in the binary and
  names the arms from the order of `pre::Op` in `crates/yavs-interp/src/pre.rs` of this checkout, so the binary must be built
  from it (`--ops-fn SYM` names another function holding the dispatch). Use it before choosing a fusion or an out-of-line body:
  the time column of `op-census` includes the kernel, this does not guess. Window with `--from '<yavs_kernel::kernel::SimControl>::run_batch:SKIP'`
  and `--max`, run with `--threads 1`.
- A build without line tables (a plain release build) gives functions only; with `tools/build-bench.sh` the same
  code and the same addresses as the timed binary, which is the point (root rule 19, bench builds).
- A function that is always inlined has no row of its own; its instructions are in its caller, its lines are not.

## 3. Design decisions that bind this module

| Decision | Why | Source |
|---|---|---|
| Single-stepping with ptrace, not an instruction-count sampler | An exact per-function and per-line count of a window, repeatable to the instruction | user, 2026-10-24 (#440) |
| C for the stepper, Python (stdlib) for symbols and lines | A C compiler is an approved build-time dependency; `nm` and `addr2line` already resolve DWARF | root rule 9 |
| `--ops` attributes by position in the dispatch loop, not by symbol: the first instruction after the dispatch jump names the op | An op's cost is its arm plus the helpers it calls (`read_into`, `assign_code_fast`, natives); symbols alone split it | #520, interpreter round 18 |
| Windows start at a function entry (a breakpoint) and end at an entry, a count or the exit | Skip start-up and warm-up cheaply, count one phase | |

## 4. Invariants and pitfalls

- The program's own output goes to the terminal as usual; the report follows it.
- The self-test is the only test (it needs `cc`, `nm`, `addr2line`); it is not part of `cargo test`.
- PIE and non-PIE executables both work (the load base is read from `/proc/PID/maps` at the exec stop); the self-test builds both.
- Do not commit the pc files or reports.
