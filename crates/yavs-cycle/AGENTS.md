# AGENTS.md: yavs-cycle

Read the root `AGENTS.md` first. This file is for an agent about to change `crates/yavs-cycle`.
Cycle mode lives on the side branch `Lukas-mode` only, milestone by milestone; this is milestone
4a: two-state values, the types, expressions and statements of RTL (`numeric_std`, `case`, nested
arrays, enumerations, concurrent assignments, `wait until` processes, asynchronous resets, unrolled
loops), and the bench RTL designs of `tools/bench/designs` inside the region.
Milestone 3 (in parallel) added the subset checker (`check.rs`), which is also the selector, and
the element-level dependence graph (`dep.rs`).

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
- Selection is automatic and never an error under `--cycle`. A process stays out for one of three
  kinds of reason, kept apart in the code (`region::Why`) and the report: outside the subset (a
  code `E4110` to `E4126`, the construct's position), behavioural (the testbench kind: timed or
  several waits, `wait;`, a procedure that waits, files, text I/O, access values, protected
  objects, simulator procedures), or in the subset but not implemented yet (the lowering's reason).
  Under `--cycle-strict` every reason of a process below the top-level design unit that is not
  behavioural is an error with its code (`E4101` for not implemented yet), all of them listed.
- Does not: change the normal mode (without the option no code of this crate runs; the only hook is
  `yavs_elab::Elaborated::region`), use `yavs-lower`, the IR or the interpreter for region code (it
  lowers the elaborated HIR itself), or keep anything between runs (no code cache yet).
- Depends on: `yavs-sem` (HIR, `reads_deep`, `stmt_exprs`, `edge_shape`),
  `yavs-elab` (net graph, `NetGraph::select` also on layouts of variables and of ports collapsed into parts, process instances, `var_shapes`, `value`, `range`, the `Region` hook), `yavs-kernel`
  (`Process`, `Ctx`, `publish_planes`, `Builder::quiet`), `yavs-ir` (`ieee::parse`: which `ieee`
  subprogram a call is), `yavs-rt` (`perfmap`), `yavs-diag`, `yavs-syntax` (identifiers of layout objects), the Cranelift crates at the version
  `yavs-clift` pins (unconditional dependencies: the default test suite runs cycle mode). Called by:
  `yavs` (`run`).

## 2. Map

| File | Responsibility |
|---|---|
| `src/lib.rs` | `plan` (selection, `--cycle-strict` errors with the code of each reason, the region report in sections by kind of reason, the loop groups and the asserts not evaluated, `YAVS_CYCLE_VERDICTS`, `build`), `codes` (every code of the mode), `Metas` (what was read as 0, the summary line the CLI prints after the run), `build` (image layout: the words of each kernel signal, its leaves at their kernel offsets, next-state words, the edge word, member slots, two error words; which words hold state; the three functions as pieces, one per member; the resets the region wakes on; the quiet signals; the check sites; the `RegionMake` closure: one export per kernel driver), `Resolve` |
| `src/check.rs` | The subset checker, per process: shape (clocked `if <edge>` with an optional asynchronous reset branch of any condition on listed signals, `wait until <edge>` first, combinational processes and concurrent statements), behavioural processes, every construct outside the subset (`codes`), and an abstract interpretation of the body over element sets: what each driven run depends on (`Dep`: a set, plus an element-by-element copy), what is assigned on every path, variables read before assignment, reads against the wait set; static `for` loops enumerated with the parameter known (`BUDGET` statement instances), other loops in rounds until the dependences settle (`ROUNDS`), conditions elaboration knows pruned (`static_bool`), pure functions of constants run (`call_value`) |
| `src/dep.rs` | `ESet` (element runs per leaf), `Dep`, `Out`; `analyse`: atoms, hubs, the element-level graph, its loops (printed), the process loop groups, their pass count K and pieces after a split, the schedule order |
| `src/region.rs` | `Why`, `subset` (the checker over every process, the set rules: an element with another source, a clock made inside, element loops), `select` (the lowering of what fits, as `Why::NotYet` when it fails or sees another shape; the set rules of what is implemented: port or constant sources, signals split between drivers, one clock and edge, loop groups; the order from `dep`), `boundary` (driven, outputs, waited, inputs, wakes, next-state leaves), `outside_reads` (what non-members may read, and may wait on), `in_top_unit`, `where_` |
| `src/lower.rs` | HIR to `Op`s on two-state words (`Lx`), only for processes the checker fits (`region::select`). Shapes: clocked `if <edge> [and c] then`, `if <reset> then .. elsif <edge> then` (the edge word decides the second branch), `wait until <edge> [and c];` as first statement, `q <= d when <edge>;`; combinational processes and concurrent simple, conditional and selected assignments (no latch, no state variable, a complete sensitivity list: the checker's proof, not tested again here); concurrent asserts (not evaluated). Statements: assignments (simple, conditional, selected), `if` and `case` (folded conditions take their branch alone), `for` loops with a range known during elaboration, unrolled (`UNROLL_MAX`), `exit` and `next` (folded, or a flag the later iterations and statements run behind: `Jump`, `guard`), procedure calls and concurrent procedure calls. Calls of the design's subprograms (not of `ieee` or `std`) are inlined (`inline`): formals bound to the actuals' values and places (`Bind`; scalars known during elaboration as constants, so loop bounds, lengths and `if`s on them fold; `out` and `inout` by reference, as the normal mode passes them; scalar `in` copied for procedures and impure functions), constrained formals and results given their subtype's index ranges (`rebound`), locals fresh at every call (`locals`, `layout`), `return` into a result slot or, for one final `return`, its value itself; the formals of a body declared apart mapped by position; recursion as deep as elaboration fixes (`DEPTH_MAX`); every inlined statement counts against `UNROLL_MAX`. `ieee` functions without a native key: `is_x` false, the conversions identities. Values (`Val`): logic parts with nested index ranges (`Dims`, up to four levels), integers and enumeration positions (`IntV`), records. Names: fields, static and run-time indices, static slices and slices with run-time bounds of a length known during elaboration (`dynamic_slice`: the length probed, checked at run time), ports collapsed into part of their actual's leaves (`place_sig`); constants (package ones included), generics and generate parameters folded with the elaboration's values (`fold`), `'length` and its friends, `'range` loops. Expressions: aggregates (array, in the shape of their target; record), qualified expressions and conversions (integer ones checked), predefined operators on logic, BIT/BOOLEAN (short-circuit where the right operand can fail a check), integers and enumerations, `std_logic_1164` gates, reductions, the identities of two-state values (`to_x01`, `is_x` false, ...), `numeric_std`/`numeric_bit` `+ - *` (any length, integers converted), `/ mod rem` (one word), comparisons, `maximum`/`minimum`, shifts and rotations (any width by a count known at elaboration, one word by a run-time count), `resize`, `to_integer` (checked), `to_unsigned`/`to_signed`, `std_match` and `?=` (a `'-'` of a literal matches anything). A comparison or `numeric_std` operation with a literal holding another metavalue keeps the process out. The logical operators of `numeric_bit` and `numeric_std` with no native key are gates by name. `Check` (the messages of failed checks, the normal mode's texts); `Op::map` |
| `src/codegen.rs` | The only `unsafe`: Cranelift functions `settle`, `edge` and `init` (`Fun`) on the image, each cut between members into functions of about `CHUNK` operations; image words in registers within a member (`Gen::cache`: one load, one store at the member's end for words that hold state, none for scratch words); multi-word arithmetic (carry chains, schoolbook products, compares from the top word); elements and slices at run-time indices of any width (`dyn_read`, `dyn_write`); checks as a compare and a cold exit returning the site; `YAVS_CYCLE_CHECKS=off`, `YAVS_CYCLE_DISASM=1`, perf map names (`YAVS_CLIFT_PERFMAP`) |
| `src/rt.rs` | `RegionProc`: the boundary conversion (logic, and integers and enumerations of 1, 4 or 8 bytes), initialisation (every kernel signal from the kernel, `init`, then the edge function with no edge where a reset may hold, else settle, export all), wakes (inputs, edge test into the edge word, the function, export of what changed through `publish_planes`, `assign_wide_now` or `assign_bytes_now`), a failed check as the normal mode's error in the member process, waits |
| `crates/yavs/tests/cycle.rs` | `tests/cycle/calls.vhd` (inlined functions and procedures, early returns, `exit`/`next`, recursion, record results, `out`/`inout`, signal formals, a concurrent call) against the normal mode at widths 1, 63, 64, 65, 128; failed checks inside callees (result, parameter, index); a `'Z'` made in a called function; the checker's verdicts on `tests/cycle/subset.vhd` (every reason with code and position, behavioural processes, every shape that fits, the loop group of `levels`; `YAVS_CYCLE_VERDICTS=1`), `force`, latches, state variables and incomplete lists left out with the normal result, `--cycle-strict` codes; rec_pair, `tests/cycle/partial.vhd`, `quiet.vhd` (outputs read late, waited on, read by an idle-skipped process), `ints.vhd`, `shapes.vhd` (the clocked and combinational shapes), `ports.vhd` (ports collapsed into parts of a vector) and the bench RTL designs (shortened) equal to the normal mode under `--cycle` (and `--cycle-strict`; tiers, threads), with their membership; the bench designs that stay out and why; every two-state and `numeric_std` operation (`numeric.vhd`) against the normal mode at widths 1, 63, 64, 65, 128, 200; all nine values at the boundary and the metavalue counts; failed checks; a member driving a null array; `--cycle-strict` errors, `E4102`, the two options exclusive |

## 3. Design decisions that bind this module

| Decision | Why | Source |
|---|---|---|
| Members keep their kernel process (no code, no drivers); the region process comes after all others and owns their drivers | Process ids index `Elaborated::procs` (profiles, hang reports, run-time diagnostics) | `yavs-elab` `build.rs` |
| Inside the region the mode is for speed: `assert` and `report` of members are compiled to nothing (counted per process in the report), and no simulation-only bookkeeping (ieee warnings, history of region signals) is built into region code | The user's direction for this mode (2026-11-15); processes outside keep full semantics | user direction, 2026-11-15 |
| Integer overflow (against the result type), subtype range and index checks stay: a compare and a never-taken branch to a cold exit; the run stops with the normal mode's message, time and process (no source label). `YAVS_CYCLE_CHECKS=off` is for developers measuring them: range and overflow checks left out, dynamic indices clamped into the vector, divisions still guarded | The user kept overflow checks (2026-11-15) and with them the checks of the same price | user direction, 2026-11-15 |
| Values are two-state: a logic value of n elements is ceil(n / 64) words (element 0 in bit 0, the bits above n zero), an integer one word (i64; the kernel's 4 or 8 bytes); uninitialised logic state, metavalue literals and metavalues at the inputs read as 0 and are counted | User decisions D3 to D5 (zero, counted) | design amendments A2 |
| A member whose driven leaf has any source the kernel resolves for it (another driver, a constant source, a port edge or converted port, a resolution function, a guard) stays out | It would read its own driving value from the image, not the resolved one | `src/region.rs` `select` |
| A signal leaf is exported when anything outside may read it: a non-member's names (whole objects), a port edge, an implicit signal, a guard, a package signal; a non-member that calls a subprogram declared outside a package, or an external name anywhere, exports every leaf | Sound by over-approximation: a missed reader would see a stale value | `src/region.rs` `outside_reads` |
| Soundness by direction: what a value depends on is over-approximated (a dynamic index reads the whole prefix, a call depends on all its arguments and on the signals its body reads), what is assigned on every path and the wait set are under-approximated; an imprecision can only leave a process out | A false acceptance gives a wrong result with nothing to catch it | `src/check.rs` |
| A combinational process fits when every driven element any path may assign is assigned on every path (an element no path assigns keeps its initial value), every variable element it assigns anywhere is assigned before each read, every signal element it reads is in its wait set (its list resolved exactly, or what elaboration took for `all`), and every element the analysis sees assigned is one of its elaborated drivers (a cross-check of the element mapping) | Latches, state variables and incomplete lists compute something else in the normal mode | `src/check.rs` `finish_comb` |
| The element graph over everything that fits must be acyclic (an element loop is `E4122`, printed); process loop groups (a process reading what it drives through others or itself, without an element loop) get K = 1 + the most back edges on a path in the element order, and the pieces a split per pass level would need; a group is not implemented yet at run time | The design's static schedule; the run time runs one ordered pass | `src/dep.rs`, `src/region.rs` |
| An asynchronous reset may be made inside the region (a reset synchroniser): its branch is combinational logic of what it reads while it holds, and its edges are in the graph; a clock may not: a combinational maker stays out (the kernel makes the clock), the users of a clock made by a register stay out | Most of a CPU's processes use an internal reset; an internal clock needs an inner phase loop | `src/region.rs` `subset` |
| A clocked process whose asynchronous reset a member drives is not implemented yet (`region::select`) | The lowering applies a reset branch only at the region's wakes, before it settles: a reset made inside would act one wake late | `src/region.rs` |
| A process the checker fits is lowered only when the lowering sees the same shape and clock; otherwise it is not implemented yet | The two must agree on what runs at an edge | `src/region.rs` `same_shape` |
| A clocked write goes to a next-state block only when a clocked member reads the leaf | Reads in a clocked body see the value before the edge | `src/lib.rs` `build` |
| The image holds one block per kernel signal, with each leaf at its kernel offset: an export is one compare and one block per kernel driver; a record copy between two signals of one record type is one copy or blend per pair of blocks | The gather of leaf parts into driver blocks was a fifth of a clock of `rec_pair` | `src/lib.rs` `build`, `src/lower.rs` `assign` |
| An exported kernel signal none of whose leaves anything outside waits on (`outside_reads`: sensitivity lists, `all`, wait statements, statements that are not processes, port edges, implicit signals, guards, package signals; everything when an external name or a local subprogram call is seen) is quiet: no byte codes, and the kernel changes it without an event while it is not watched at run time (`yavs-kernel` AGENTS.md, quiet signal) | Soundness rests on the kernel's run-time test; the static set only keeps codes where an event will be needed anyway | `src/lib.rs` `build` |
| With one clock and no combinational inputs the region has the kernel's edge trigger (with the reset when the members have one asynchronous reset); otherwise it waits on the clock, the resets and the inputs and tests the edge itself | `wait_edge` cannot be combined with data wakes | `src/rt.rs` |
| A member with an asynchronous reset tests the edge word itself; then the edge function runs at every wake and at initialisation (a reset that holds at time 0 applies), and the other clocked bodies sit behind the edge word | The reset branch runs on any event that leaves the reset level holding, as the normal mode's process does | `src/lib.rs` `build`, `src/lower.rs` |
| The drivers of members on adjacent elements of one kernel signal are one kernel driver (`yavs-elab` `build.rs`), unless one has an initial value of its own | 4096 one-element processes of one vector exported 4096 drivers per clock | `yavs-elab` `build.rs` |
| A comparison (`=`, `/=`, a `case` choice, a `numeric_std` operator) with a literal holding `'U'`, `'X'`, `'W'`, `'Z'` (or `'-'` outside a matching comparison) keeps the process out | A two-state value never equals it, while the normal mode can (`rtl_lfsr` initialises on `= 'U'`); reading the literal as 0 (D4) would give a wrong result | `src/lower.rs` `match_codes` |
| An array signal of more than 4096 elements written at a run-time index keeps its process out | A RAM: its next-state copy would cost the whole array per edge; RAMs are the next milestone | `src/lower.rs` `assign` |
| Calls are inlined at every call site, no shared bodies or frames: a callee called at many sites (or in many instances) is compiled that many times | Fits the lowering (values in words, loop parameters already folded per iteration); shared per-shape bodies are a later step | `src/lower.rs` `inline` |
| `out` and `inout` variable formals alias their actual, an early `return` leaves it unchanged; a function that may reach its end without `return` keeps its process out | What the normal mode does (it passes them by address); the end without `return` is a run-time error there, which region code does not model | `src/lower.rs` `formals`, `inline` |
| The trigger functions are cut into pieces, one per member, and into functions of about 4000 operations; registers hold words only within a piece | The compile time of one function grows faster than its size, and a branch saves and forgets the words in registers | `src/codegen.rs` |

## 4. Invariants and pitfalls

- Every reason a process is left out is a plain sentence a user reads in the report (rule 35); no
  selection path may panic. A failure to compile what was selected is `E9999`.
- The bits above a logic value's length are zero in every word: every op keeps it, and
  `Gen::bits` relies on it to leave masks out.
- `Op` operands are read before the result is written; the peephole in `Lx::assign` retargets the
  operation that made a fresh temporary.
- A word the code addresses at run time (`IndexRead`, `IndexWrite`) must be in memory: the cache
  spills it first and forgets it after a write (an element may reach into the word after the
  value, which the image always has). A branch stores what it changed before its join, and every
  word it forgot is stale after the join too (milestone 2's code kept the old value: a run-time
  write or a nested branch inside a branch was lost).
- Scratch words never cross members: a piece starts from memory.
- Integers export their low 4 or 8 bytes: correct because every assignment to a signal is range
  checked (with `YAVS_CYCLE_CHECKS=off` a value out of the subtype is exported truncated).

## 5. How to work here

```sh
cargo test -p yavs --test cycle
yavs run --cycle --work=work:W top          # the region report on stderr
YAVS_CLIFT_PERFMAP=1 perf record ... yavs run --cycle ...   # region functions named cycle_*
YAVS_CYCLE_DISASM=1 yavs run --cycle ...                     # machine code of the region on stderr
YAVS_CYCLE_CHECKS=off yavs run --cycle ...                   # without range and overflow checks
YAVS_BIN=<release yavs> python3 -I tools/difftest/cyclegate.py   # the correctness gate
```

- Run `tools/difftest/cyclegate.py` before every push of cycle code (30 seconds): it runs the difftest corpus,
  `tests/cycle` and the bench designs in normal and cycle mode with one binary and fails on any difference the
  documented losses (A6) or the reviewed list `tools/difftest/cyclegate.expected` do not explain; a new entry in that list is a decision to accept the loss, not a way to silence the gate. It also shows how many
  processes joined per test, so a pass is not vacuous: read the summary's count of tests with a member.

- Codes owned: `E4100`-`E4199`, `W4100`-`W4199` (in use: `E4101`, `E4102`, `E4110` to `E4126`, `lib.rs` `codes`).
- `YAVS_CYCLE_VERDICTS=1 yavs run --cycle ...` prints the report and stops before compiling or
  simulating: the checker's verdicts on any design, whatever the lowering supports.

## 6. Coordination

- The hooks in other crates are kept small (`yavs-elab` `Region`, `build.rs`; `yavs` `run_design`) so the branch follows main with cheap merges.
