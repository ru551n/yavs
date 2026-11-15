# AGENTS.md: yavs-cycle

Read the root `AGENTS.md` first. This file is for an agent about to change `crates/yavs-cycle`.
Cycle mode lives on the side branch `Lukas-mode` only, milestone by milestone; this is milestone 2,
two-state values on the slice of `tools/bench/designs/rec_pair.vhd` plus integers, a clocked `if`
and dynamic indices.

## 1. Purpose and boundaries

- `yavs run --cycle` (or `--cycle-strict`, its strict form, given instead): the processes the mode
  can take join one region, run by one kernel process on a static schedule (clocked bodies, commit,
  combinational bodies in dependency order) over a flat image of two-state words, compiled with
  Cranelift.
  Every other process stays in the event kernel and is listed in the region report with its reason.
- The region reads the inputs its members read from the kernel at every wake, and drives with zero
  delay, through the member processes' own drivers, every member-driven signal anything outside may
  read, only where it changed. At the boundary a kernel value becomes two-state (`'1'` and `'H'`
  are 1, every other value 0, a metavalue counted) and outputs go back as `'0'` and `'1'`. An exported kernel signal nothing outside waits on is quiet
  (`yavs_kernel::Builder::quiet`, no byte codes): the kernel makes its change without an event
  while nothing waits on it at run time.
- Selection is automatic and never an error under `--cycle`. Under `--cycle-strict` a process left
  out that is below the top-level design unit and not behavioural is `E4101`, all of them listed.
- Does not: change the normal mode (without the option no code of this crate runs; the only hook is
  `yavs_elab::Elaborated::region`), use `yavs-lower`, the IR or the interpreter for region code (it
  lowers the elaborated HIR itself), or keep anything between runs (no code cache yet).
- Depends on: `yavs-sem` (HIR, `Db::pure_comb_reads`, `reads_deep`, `stmt_exprs`, `edge_shape`),
  `yavs-elab` (net graph, process instances, `var_shapes`, the `Region` hook), `yavs-kernel`
  (`Process`, `Ctx`, `publish_planes`, `Builder::quiet`), `yavs-ir` (`ieee::parse`: which `ieee`
  subprogram a call is), `yavs-rt` (`perfmap`), `yavs-diag`, the Cranelift crates at the version
  `yavs-clift` pins (unconditional dependencies: the default test suite runs cycle mode). Called by:
  `yavs` (`run`).

## 2. Map

| File | Responsibility |
|---|---|
| `src/lib.rs` | `plan` (selection, `--cycle-strict` errors, the region report, `build`), `codes` (`E4101`, `E4102`), `Metas` (what was read as 0, the summary line the CLI prints after the run), `build` (image layout: the words of each kernel signal, its leaves at their kernel offsets, next-state words, member slots, two error words; which words hold state; the two schedules; the quiet signals; the check sites; the `RegionMake` closure: one export per kernel driver), `Resolve` |
| `src/region.rs` | `classify` (the one membership decision per process; milestone 3's checker replaces its body), `select` (the fixpoint over the set: a leaf also driven outside, resolution or guard or constant source, an internal clock, a second clock, combinational loops; then the dependency order), `boundary` (driven, outputs, waited, inputs, wakes, next-state leaves), `outside_reads` (what non-members may read, and may wait on), `behavioural`, `in_top_unit`, `where_` |
| `src/lower.rs` | HIR to `Op`s on two-state words (`Lx`): the clocked shape `if <edge> then ... end if;` with the kernel's edge trigger and `if` inside it, combinational processes (`pure_comb_reads`, sensitivity list complete, no `if` yet); variable and signal assignment, names (fields, static index and slice, an element chosen at run time), logic literals (metavalues read as 0, counted per site; `'Z'` keeps the process out), logical operators (predefined and `std_logic_1164`, `numeric_std`), `not`, `&`, `=`/`/=` of logic values, `numeric_std` unsigned `+`/`-` of two vectors or a natural constant, integers (literals, `+ - * / mod rem`, comparisons, subtype ranges), conversions between logic vectors; `Check` (the messages of failed checks, the normal mode's texts) |
| `src/codegen.rs` | The only `unsafe`: Cranelift functions `settle` and `edge` on the image; image words in registers (`Gen::cache`: one load, one store at the end for words that hold state, none for scratch words); checks as a compare and a cold exit returning the site; `YAVS_CYCLE_CHECKS=off`, `YAVS_CYCLE_DISASM=1`, perf map names (`YAVS_CLIFT_PERFMAP`) |
| `src/rt.rs` | `RegionProc`: the boundary conversion, initialisation (every kernel signal from the kernel, settle, export all), wakes (inputs, edge test, the function, export of what changed through `publish_planes`, `assign_wide_now` or `assign_bytes_now`), a failed check as the normal mode's error in the member process, waits |
| `crates/yavs/tests/cycle.rs` | rec_pair, `tests/cycle/partial.vhd`, `quiet.vhd` (outputs read late, waited on, read by an idle-skipped process) and `ints.vhd` equal to the normal mode under `--cycle` (and `--cycle-strict`; tiers, threads); every two-state operation against the normal mode at widths 1, 63, 64, 65, 128, 200; all nine values at the boundary and the metavalue counts; failed checks; `--cycle-strict` errors, `E4102`, the two options exclusive |

## 3. Design decisions that bind this module

| Decision | Why | Source |
|---|---|---|
| Members keep their kernel process (no code, no drivers); the region process comes after all others and owns their drivers | Process ids index `Elaborated::procs` (profiles, hang reports, run-time diagnostics) | `yavs-elab` `build.rs` |
| Inside the region the mode is for speed: `assert` and `report` of members are compiled to nothing (counted per process in the report), and no simulation-only bookkeeping (ieee warnings, history of region signals) is built into region code | The user's direction for this mode (2026-11-15); processes outside keep full semantics | user direction, 2026-11-15 |
| Integer overflow (against the result type), subtype range and index checks stay: a compare and a never-taken branch to a cold exit; the run stops with the normal mode's message, time and process (no source label). `YAVS_CYCLE_CHECKS=off` is for developers measuring them: range and overflow checks left out, dynamic indices clamped into the vector, divisions still guarded | The user kept overflow checks (2026-11-15) and with them the checks of the same price | user direction, 2026-11-15 |
| Values are two-state: a logic value of n elements is ceil(n / 64) words (element 0 in bit 0, the bits above n zero), an integer one word (i64; the kernel's 4 or 8 bytes); uninitialised logic state, metavalue literals and metavalues at the inputs read as 0 and are counted | User decisions D3 to D5 (zero, counted) | design amendments A2 |
| A member whose driven leaf has any source the kernel resolves for it (another driver, a constant source, a port edge or converted port, a resolution function, a guard) stays out | It would read its own driving value from the image, not the resolved one | `src/region.rs` `select` |
| A signal leaf is exported when anything outside may read it: a non-member's names (whole objects), a port edge, an implicit signal, a guard, a package signal; a non-member that calls a subprogram declared outside a package, or an external name anywhere, exports every leaf | Sound by over-approximation: a missed reader would see a stale value | `src/region.rs` `outside_reads` |
| A combinational member must satisfy `Db::pure_comb_reads` (with its asserts and reports left out) and its sensitivity list must name every leaf it reads, by whole signals (a list naming a part keeps it out) | It runs on every settle, not only on events of its list | `src/lower.rs` |
| A clocked write goes to a next-state block only when a clocked member reads the leaf | Reads in a clocked body see the value before the edge | `src/lib.rs` `build` |
| The image holds one block per kernel signal, with each leaf at its kernel offset: an export is one compare and one block per kernel driver; a record copy between two signals of one record type is one copy or blend per pair of blocks | The gather of leaf parts into driver blocks was a fifth of a clock of `rec_pair` | `src/lib.rs` `build`, `src/lower.rs` `assign` |
| An exported kernel signal none of whose leaves anything outside waits on (`outside_reads`: sensitivity lists, `all`, wait statements, statements that are not processes, port edges, implicit signals, guards, package signals; everything when an external name or a local subprogram call is seen) is quiet: no byte codes, and the kernel changes it without an event while it is not watched at run time (`yavs-kernel` AGENTS.md, quiet signal) | Soundness rests on the kernel's run-time test; the static set only keeps codes where an event will be needed anyway | `src/lib.rs` `build` |
| With one clock and no combinational inputs the region has the kernel's edge trigger; otherwise it waits on the clock and the inputs and tests the edge itself | `wait_edge` cannot be combined with data wakes | `src/rt.rs` |

## 4. Invariants and pitfalls

- Every reason a process is left out is a plain sentence a user reads in the report (rule 35); no
  selection path may panic. A failure to compile what was selected is `E9999`.
- The bits above a logic value's length are zero in every word: every op keeps it, and
  `Gen::bits` relies on it to leave masks out.
- `Op` operands are read before the result is written; the peephole in `Lx::assign` retargets the
  operation that made a fresh temporary.
- A word the code addresses at run time (`IndexRead`, `IndexWrite`) must be in memory: the cache
  spills it first and forgets it after a write. A branch stores what it changed before its join.
- Integers export their low 4 or 8 bytes: correct because every assignment to a signal is range
  checked (with `YAVS_CYCLE_CHECKS=off` a value out of the subtype is exported truncated).

## 5. How to work here

```sh
cargo test -p yavs --test cycle
yavs run --cycle --work=work:W top          # the region report on stderr
YAVS_CLIFT_PERFMAP=1 perf record ... yavs run --cycle ...   # region functions named cycle_*
YAVS_CYCLE_DISASM=1 yavs run --cycle ...                     # machine code of the region on stderr
YAVS_CYCLE_CHECKS=off yavs run --cycle ...                   # without range and overflow checks
```

- Codes owned: `E4100`-`E4199`, `W4100`-`W4199` (in use: `E4101`, `E4102`).

## 6. Coordination

- The hooks in other crates are kept small (`yavs-elab` `Region`, `build.rs`; `yavs` `run_design`) so the branch follows main with cheap merges.
