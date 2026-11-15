# AGENTS.md: yavs-cycle

Read the root `AGENTS.md` first. This file is for an agent about to change `crates/yavs-cycle`.
Cycle mode lives on the side branch `Lukas-mode` only, milestone by milestone; this is milestone 1,
the thin slice on `tools/bench/designs/rec_pair.vhd`.

## 1. Purpose and boundaries

- `yavs run --cycle` (or `--cycle-strict`, its strict form, given instead): the processes the mode
  can take join one region, run by one kernel process on a static schedule (clocked bodies, commit,
  combinational bodies in dependency order) over a flat image of blocks, compiled with Cranelift.
  Every other process stays in the event kernel and is listed in the region report with its reason.
- The region reads the inputs its members read from the kernel at every wake, and drives with zero
  delay, through the member processes' own drivers, every member-driven signal anything outside may
  read, only where it changed.
- Selection is automatic and never an error under `--cycle`. Under `--cycle-strict` a process left
  out that is below the top-level design unit and not behavioural is `E4101`, all of them listed.
- Does not: change the normal mode (without the option no code of this crate runs; the only hook is
  `yavs_elab::Elaborated::region`), use `yavs-lower`, the IR or the interpreter for region code (it
  lowers the elaborated HIR itself), or keep anything between runs (no code cache yet).
- Depends on: `yavs-sem` (HIR, `Db::pure_comb_reads`, `reads_deep`, `stmt_exprs`, `edge_shape`),
  `yavs-elab` (net graph, process instances, `var_shapes`, the `Region` hook), `yavs-kernel`
  (`Process`, `Ctx`), `yavs-logic` (four-plane gates), `yavs-ir` (`ieee::parse`: which `ieee`
  subprogram a call is), `yavs-rt` (`perfmap`), `yavs-diag`, the Cranelift crates at the version
  `yavs-clift` pins (unconditional dependencies: the default test suite runs cycle mode). Called by:
  `yavs` (`run`).

## 2. Map

| File | Responsibility |
|---|---|
| `src/lib.rs` | `plan` (selection, `--cycle-strict` errors, the region report, `build`), `codes` (`E4101`, `E4102`), `build` (image layout: current blocks of signal leaves, next-state blocks, member slots; the two schedules; the `RegionMake` closure), `resolve` |
| `src/region.rs` | `classify` (the one membership decision per process; milestone 3's checker replaces its body), `select` (the fixpoint over the set: a leaf also driven outside, resolution or guard or constant source, an internal clock, a second clock, combinational loops; then the dependency order), `boundary` (driven, outputs, inputs, wakes, next-state leaves), `outside_reads` (what non-members may read), `behavioural`, `in_top_unit`, `where_` |
| `src/lower.rs` | HIR to `Op`s (`Lx`): the clocked shape `if <edge> then ... end if;` with the kernel's edge trigger, combinational processes (`pure_comb_reads`, sensitivity list complete); variable and signal assignment, names (fields, static index and slice), logic literals, logical operators (predefined and `std_logic_1164`, `numeric_std`), `not`, `&`, `numeric_std` `unsigned + natural constant`, conversions between logic vectors |
| `src/codegen.rs` | The only `unsafe`: Cranelift functions `settle` and `edge` on the image, the four-plane gate helpers, perf map names (`YAVS_CLIFT_PERFMAP`) |
| `src/rt.rs` | `RegionProc`: initialisation (every leaf from the kernel, settle, export all), wakes (inputs, edge test, the function, export of what changed), waits |
| `crates/yavs/tests/cycle.rs` | rec_pair and `tests/cycle/partial.vhd` equal to the normal mode under `--cycle` and `--cycle-strict` (tiers, threads), `--cycle-strict` errors, `E4102`, the two options exclusive |

## 3. Design decisions that bind this module

| Decision | Why | Source |
|---|---|---|
| Members keep their kernel process (no code, no drivers); the region process comes after all others and owns their drivers | Process ids index `Elaborated::procs` (profiles, hang reports, run-time diagnostics) | `yavs-elab` `build.rs` |
| Values are four planes in milestone 1 (`Block`), gates are calls into `yavs-logic`; uninitialised state is `U`, as the kernel holds it | The milestone measures the image and the schedule before two-state values (design 5.8) | design.md 5.8, 9 |
| A signal leaf is exported when anything outside may read it: a non-member's names (whole objects), a port edge, an implicit signal, a guard, a package signal; a non-member that calls a subprogram declared outside a package, or an external name anywhere, exports every leaf | Sound by over-approximation: a missed reader would see a stale value | design.md 1.3 |
| A combinational member must satisfy `Db::pure_comb_reads` and its sensitivity list must name every leaf it reads | It runs on every settle, not only on events of its list | design.md 3.1, D7 |
| A clocked write goes to a next-state block only when a clocked member reads the leaf | Reads in a clocked body see the value before the edge | design.md 6.3 |
| With one clock and no combinational inputs the region has the kernel's edge trigger; otherwise it waits on the clock and the inputs and tests the edge itself | `wait_edge` cannot be combined with data wakes | design.md 1.2 |

## 4. Invariants and pitfalls

- Every reason a process is left out is a plain sentence a user reads in the report (rule 35); no
  selection path may panic. A failure to compile what was selected is `E9999`.
- Logic leaves of at most 64 elements only: one block each, and a kernel driver of the region at
  most 64 elements (checked when the process is made).
- `Op` operands are read before the result is written; the peephole in `Lx::assign` retargets the
  operation that made a fresh temporary.

## 5. How to work here

```sh
cargo test -p yavs --test cycle
yavs run --cycle --work=work:W top          # the region report on stderr
YAVS_CLIFT_PERFMAP=1 perf record ... yavs run --cycle ...   # region functions named cycle_*
```

- Codes owned: `E4100`-`E4199`, `W4100`-`W4199` (in use: `E4101`, `E4102`).

## 6. Coordination

- The hooks in other crates are kept small (`yavs-elab` `Region`, `build.rs`; `yavs` `run_design`) so the branch follows main with cheap merges.
