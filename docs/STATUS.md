# Status

## Milestones

| Milestone | State |
|---|---|
| M0: workspace, lexer and parser | done: `yavs parse`, corpora below |
| M1: semantic analysis | in progress: slices S1 to S3 mostly in, `docs/design/sem.md` |
| M2: IR, interpreter, elaboration, kernel | done: the differential corpus passes at 1 thread, 4 threads and 4 shuffled threads (`tests/corpus/RESULTS.md`); see the lower, ir and interp, elab and kernel sections. Not done: the PSL natives, out of scope (`lib/NATIVE.md` section 4) |
| M4: external test runner end to end | first external test runner testbench runs (#110): a pass/fail `test_runner` testbench and the runner's `examples/vhdl/user_guide` give the right pass/fail per test with yavs as the simulator; the runner's own suites: all 11 pass completely (1357 tests), table below (#112); the runner package `vunit-yavs` (`python/vunit-yavs`, 2026-10-28) selects yavs through `add_package`, with `--yavs-debug` (DAP), `yavs.max_threads` and an opt-in jobserver: unit and fake-binary tests, and a real-binary run, jobserver run and DAP handshake on a yavs built from main; a full DAP session of a runner testbench is an internal error E9999 (issue to file) |
| M7: waves | `yavs run --wave` writes an FST of the whole design (#129, #116): hierarchy, records/arrays as scopes, enum tables, globs and size cap, final value per time step, same file at any `--threads` and for any run speed (blocks end by size only; `YAVS_WAVE_FLUSH=5s` opts in to a wall-clock flush for killed runs, #666), finished on Ctrl-C; signals left out by globs or the cap cost nothing at run time; instances in generates, 2-D arrays; read back with wellen in `crates/yavs/tests/wave.rs`. integers carry the subtype name and the width of their range, generate alternatives keep their label (#306). `--wave-variables` records process and non-protected shared variables, records and arrays as scopes, integers with the width of their elaborated range, sampled at the end of each time step through the observer accessors (#305, #348, #321). Open: protected-type shared variables |
| M3: full types and libraries | done per `docs/plan.md`: arrays, records, access and protected types, resolution (composite, access, protected and resolution groups in `tests/corpus/RESULTS.md`); `numeric_std`, fixed and float from source (#94); textio, `env`, package generics, external names; `--load` foreign subprograms (#95); bit-plane layout (user, 2026-10-18). Not done: the PSL natives, out of scope (`lib/NATIVE.md` section 4) |
| M8: DAP server | first slice (#130): `yavs run --dap=PORT\|stdio` serves the Debug Adapter Protocol over the interpreter's debug session (`crates/yavs-dap`): threads per process, line breakpoints with hit conditions, continue, pause (in a process or between cycles), run for or until a time, next delta and next time step (custom requests and VS Code commands, #324), signal breakpoints on a change or a value (DAP data breakpoints and `yavs/setSignalBreakpoints`, #322), force, deposit and release of signals and their fields, elements and slices (#364) and `setVariable` on signals and locals with values parsed as VHDL literals of the target type (`yavs_std::value::literal`, #323), conditions on line and data breakpoints (VHDL expressions evaluated by the server), composite variables expanded by field and index (paged), a Hierarchy scope and `yavs/hierarchy` (#326), step over/in/out, stack frames with source positions, locals, region signals and generics, evaluate for locals and signals by name or path, output events, exit status; one thread under `--dap`, so sessions are deterministic. Scripted sessions in `crates/yavs/tests/dap.rs`. VS Code extension `editors/vscode` (debugger type `yavs`, analyzes and launches over stdio): verified with the DAP test client (`test/client.js`) and in a real headless VS Code (`test/vscode`, breakpoint hit, frame, `setVariable`, force and release commands, a conditional breakpoint, scopes and an expanded array, continue to the end); not in CI. Live waves (#325): `yavs/waveSubscribe` records the client's selection (globs, array cap, variables) with the `--wave` recorder into a second sink and streams `yavs/waves` batches of completed time steps (values base64 of the recorder's bytes with `encoding` in the hierarchy, bounded queue, never blocks the simulation); `--wave` and `--vhpi` work under `--dap` (the FST equals a plain run's). Attach (#423): `--dap=PORT` prints `yavs: waiting for a debugger on port PORT` and holds the run until a client attaches (also from a response file); the extension has an "Attach to yavs" snippet and `docs/usage.rst` a recipe for debugging one test of an external test runner. A VHPI plugin's prints under `--dap=stdio` go to stderr (#389). Known limits (2026-10-28): `--cov` and `--profile` cannot be combined with `--dap`; under `--dap` the interpreter runs whatever `--tier` says (the DAP branch returns before the tier is set up, so even a tier the build lacks gives no E0120); the Hierarchy, Objects, Processes and Waves panels of `yavs-gui` are not implemented, so signal breakpoints, force and deposit are available through the protocol (VS Code) and not yet in the GUI |
| M9: coverage | statement and branch coverage (#131): `yavs run --cov[=FILE]` writes a per-run database keyed by instance path and statement text hash, `yavs cov merge`, `yavs cov report` (text summary, Cobertura XML); no counters without `--cov`; counts identical at any `--threads`; pinned in `crates/yavs/tests/coverage.rs`. Exclusions (#351): `-- coverage off/on` pragmas, `cov report --uncovered` rows and `--exclude FILE`. Toggle coverage (#343): 0->1 and 1->0 per `bit`/`std_logic` scalar of signals and ports from the value-change bus, `--cov-kind`, `--cov-toggle-max` (W0115), `toggle` exclusion rows (optional `rise`/`fall`). YAML exclusion files (#687): `--exclude FILE.yaml`/`.yml`, `cov report --uncovered=yaml`. Per-test attribution (#353): `--cov-test`, tests per item kept through `cov merge`, `yavs cov rank` (greedy minimal set, unique items per test), `cov report --tests`; database version 4. HTML report (#344, #689): `cov report --html DIR [--root DIR]`, static pages with line counts, relative source paths, a hover/focus popup per line (per-instance counts, tests, click to pin), partial-instance lines marked `ran/total`, a missed-only page per file, missed items as exclusion rows with copy buttons (E0113 for an unreadable source; Windows-style paths made relative too, #693). Condition and expression coverage (#688): focused expression coverage per input and value of `if`/`elsif`/`when`/`while`/`exit`/`next` conditions and logic expressions of assignments (`--cov-kind=condition,expression`, `Inst::CoverIn`/`CoverCond`, `CovCond::hits`), inputs constant after elaboration have no rows, rows in text, HTML table and Cobertura, exclusion per row (text and YAML), database version 5; pinned in `crates/yavs/tests/coverage_fec.rs` Counter cost (#692): `Cover`, `CoverIn` and `CoverCond` are predecoded interpreter ops and inline in the LLVM tier (one atomic add on the instance's counters through `TierFrame::cov`); a statement counter went from about 130 to about 14 instructions, the LLVM tier no longer steps them; same counts in every tier (`tiers_agree` in `coverage.rs` and `coverage_fec.rs`). Per-instance specialised copies, memoised blocks and inlining work with counters (#699): the counters of an arm a copy folds away are dropped (count 0), the rest count into the running instance's row, counts byte-identical to shared code in every tier and at any `--threads` (`copies_count_alike`, `tiers_agree` in `coverage.rs`); edge triggers stay on under `--cov` (#707, user decision): a clocked process runs only at its edge, so the statement of a pure edge test counts one run at initialisation and one per edge (usage.rst, `edge_triggers_count_their_edges`); the toggle observer takes one lock and reads plane words in place per cycle (toggle-only overhead over a plain run, millions of instructions: rtl_ff 1776 to 26, rtl_lfsr 2253 to 366, neorv32 30 us 7258 to 5341, sc5_fifo_repack 14544 to 4530; default `--cov` of rtl_ff 5311 to 3434, neorv32 16445 to 14877); a coverage run no longer ends in an interpreter internal error when elaboration-time calls are replayed (#705, `replayed_calls_run_optimised`). |
| M6: JIT tier | in progress: `yavs run --tier=llvm` (feature `llvm`) compiles every function with LLVM 21 and passes the whole difftest corpus (348/348 at 1 and 4 threads, identical to the interpreter, rule 5); 96% of the compiled programs' instructions run as native code (memory, descriptors and layouts since #334/#335; simulation 2x to 5x faster than interpreted, compile time aside), the rest through the interpreter (side exits, `docs/decisions.md`); `--tier=auto` compiles only hot functions on a background thread with identical output (#125; opt-in: since the predecoded interpreter, #431, compiled code with its side exits is slower than interpreting on most benchmarks; superseded 2026-10-22, #510: `auto` is now the default of any build with a compiled tier, see the 2026-10-28 note below), functions over `--tier-max-size` stay interpreted (#126); compiled code is kept in the compilation cache and linked by later runs (#505); open: native lowering of the rest (#336 to #341); CI builds and tests the feature on Linux x86_64, aarch64 and Windows; llvm section |
| Cranelift tier (#481 item 1) | `yavs run --tier=clift`, `--tier=clift-auto` (feature `clift`, off by default, no LLVM needed, crate `yavs-clift`) and `--tier=auto3` (both features: Cranelift for warm functions, LLVM O2 for those that stay hot) run the optimised IR on the LLVM tier's frame view, callbacks and side exits; the difftest corpus is identical to the interpreter at 1 and 4 threads and under the collection stress build; Cranelift translates at 31 to 62 us per IR instruction against 115 to 536 for LLVM; since 2026-10-24 it lowers the kernel-facing instructions, protected calls and heap code in native frames; on the window generator testbench it is 24 percent below the interpreter in instructions (LLVM `auto` 49 to 62 percent below), level or up to 8 percent above the interpreter on neorv32, fletcher, poc_arith_addw and grlib, so it is not the first level of `auto`; measurements and the gap list in the clift section |
| M5, M10 | M5 in progress, partial (#143; what works and what does not is in `docs/vhpi.md`): the `vhpi_*` export surface (#266), `yavs run --vhpi` (#267), the hierarchy (#269), values and types (#270), puts (#271), callbacks (#272), foreign models (#274, #330, #357) and plusargs (#298) are done, `crates/yavs-vhpi`, coverage in `docs/vhpi.md`; cocotb 2.1.0's own simulator regression passes on yavs (`tools/cocotb/run.py`, 78 of 78 jobs, 43 of them run VHDL tests, at 1 and 4 threads, run in the `conformance` CI workflow, #122); M10 in progress, early (`gui/README.md`): design `docs/design/gui.md` (#384, decisions answered 2026-10-19); slice G1 (#388): `gui/` workspace (EUPL-1.2) with `yavs-gui`, which starts `yavs run --dap=stdio` or attaches to a port, shows a dock layout of placeholder panels, a console of the run's output and Run/Pause/Stop, tested headless against a scripted DAP server (`gui/AGENTS.md`); `yavs run --gui` starts it (E0122 when missing); release archives ship it; slice G2 (#399): source view with breakpoint gutter (conditions, hit conditions), call stack per process, locals and watch with lazy and paged expansion and edit, hover, step buttons, time and delta in the status bar, tested against a scripted DAP server and, ignored, the real `yavs`; G3 to G6 and the server slices S1 (#396), S2 (#397), S4 (#398) not started) |

### Documentation update (#826, 2026-10-28)

The site is a user guide (2026-10-28, maintainer): `index`, `install`, `getting-started` (quickstart checked against the default build: analyze, run, a failing check, an analysis error), `libraries`, `waveforms`, `coverage`, `debugging`, `vhpi`, `running-faster`, `cli` (checked against `yavs run --help`), `troubleshooting`, and the reference pages `language-support`, `language-choices`, `conformance` (the summary of `conformance.md`) and `performance`. `usage.rst` and `recipes.rst` were split into those pages; `parallel.md` and the full conformance report are off the site; no cocotb in user documents. `language-choices` is a short table (full list in `docs/design/language-choices.rst`), `vhpi` keeps the summary table and plugin loading (audit and call details in `crates/yavs-vhpi/STANDARD.md`), `debugging` covers DAP and VS Code only. The README and the landing page agree with the guides. Corrections made to this file: the M5 and M10 rows, the vhpi export count, the M6 "opt-in" wording, the M8 limits. Known gaps found while checking (dated notes in `docs/cli-contract.md`, to be filed as issues): `--ieee-warnings=off` has no effect; `analyze -D` is not applied by `elab` and `run`, so such a file fails to run with `E0007`; the `--work` library of `run` and `elab` without an index is `L0001`, where the contract says "top level not found"; `--message-format=json` is followed by plain text lines when `analyze` stops at one of several files; an explicit `--tier` is ignored under `--dap`; the language support page spelled `--std=08`, which is rejected (`93`, `2008`, `2019`; `1993` is an accepted alias): corrected the same day.

### CLI correctness fixes from the documentation audit (#845, #850, #835, #838, #839, #836, #837, 2026-10-29)

- `--tier` help and `--explain E0120` name the tiers this executable has and no developer documents (#845, #850); no other option help names a developer document.
- `--dap` checks `--tier` like `run` without it: a tier the build lacks is `E0120` (#835); the session still runs the interpreter.
- `run` and `elab` treat a missing or empty `--work` directory as an empty library (#838): a top level that is not there is `E0104` naming the directory (the contract said so; it was `L0001`); a wrong `TOP` no longer reports "unknown configuration", `work.top` is explained, and the `L0034` help gives a working `yavs analyze` command line.
- `--message-format=json` and `short` print the batch stop note of `analyze` as a note diagnostic, so the JSON stream has nothing else; `--threads=0` is a usage error; a bare number for `--stop-time` says a time needs a unit (#839). Not changed: `finish(N)` exits with N when it fits 1..125 (documented), so `finish(2)` looks like a usage error.
- `--ieee-warnings=off` works (natives and bodies, #836); a value other than `on` or `off` is a usage error.
- `elab` and `run` accept `-D NAME=VALUE` like `analyze` (#837): the definitions are not recorded in the library, the sources are read again, so the same ones are passed to every step (like `--psl`).

### Kernel gates (#779, 2026-10-26)

(`docs/decisions.md` "Kernel gates".) A concurrent assignment of one logical operator (`not and or xor nand nor xnor`) on static signal
names of `std_ulogic`, BIT, BOOLEAN or their vectors runs as the kernel's own `GateProcess`, with zero delay or a static `after`.
`rtl_chain` (1000 inverters): 3.29 G to 2.08 G instructions under `--tier=auto`, CPU 1.36 to 0.79 times sim1. Corpus
`sigassign/logic_gates` passes against sim1 at 1 and 4 threads on every tier. Muxes, relational operators and literal operands: see "Kernel muxes" below.

### Kernel muxes (#786, 2026-10-27)

(`docs/decisions.md` "Kernel muxes".) A concurrent `y <= a when c else b;` with one condition (a code set of one logic element, or `=`/`/=` of logic
signals and values), each arm a signal copy as `CopyProcess` takes one (also records and byte parts) or a literal; `y <= a = b;` and `y <= a and '1';`
run as the kernel's own `MuxProcess` and `GateProcess`. Corpus `sigassign/mux_gates` passes against sim1 at 1 and 4 threads on every tier. Open
(#788): expression trees, compound and multi-arm conditions, constant conditions.

### Kernel programs (#788, 2026-10-27)

(`docs/decisions.md` "Kernel programs".) A concurrent assignment of an expression of logical operators, relations and element tests (also of an enumeration or integer signal against a value), with a compound or constant condition or any number of arms, runs as the kernel's own `ProgProcess` when it is no gate or mux. neorv32 loses 4 percent of its instructions, poc_cache_cpu 4, the geometric mean over the real suite is 0.99. Corpus `sigassign/prog_exprs` passes against sim1 at 1 and 4 threads on every tier. Open: `numeric_std` operators and orderings, function calls, concatenations and conversions as operands, `selected` assignments.

### Concurrent copies made by the kernel without a process run (#519, 2026-10-28)

(`docs/decisions.md` "Concurrent copies committed by the kernel".) A plain concurrent copy (`a <= b;`, kernel `CopyProcess`) whose drivers are direct and whose waits are resident is made by the kernel after the wake loop: the source values are committed to its drivers and it never enters the batch. Half the process runs of the fletcher testbenches are such copies (18 percent in neorv32, PoC and SatCat5 designs). `--tier=auto`, instructions: `fletcher_bufferreader` 0.93, `fletcher_bufferwriter_512` 0.91 to 0.97, `neorv32_single_min` 0.97, `poc_cache_cpu` 0.98 (Cranelift build; LLVM build 0.92, 0.98, 0.98, 0.98). `tools/realbench --compare` (42 tests, Cranelift bench builds, run phase, core 13 at load 30 to 45): wall geometric mean 0.97 interp, 0.96 auto, 0.98 auto-cold, instructions 1.00, 0.99, 1.00; `fletcher_bufferwriter_512` wall 0.93 / 0.92 / 0.90, `fletcher_bufferreader` 0.92 / 0.97 / 0.93 (instructions 0.92 to 0.97); the few instruction ratios above 1.04 are `--tier=auto` compile outcomes (`poc_arith_addw` auto 1.07, auto-cold 0.96) and start-up of a 0.2 G run. The LLVM build on main after the parallel kernel's pooling change (core 13, 42 tests): wall 1.00 interp, 1.00 auto, 0.99 auto-cold, instructions 0.99 each; `fletcher_bufferwriter_512` wall 0.91 / 0.93 / 0.91, `fletcher_bufferreader` 0.90 / 0.96 / 0.92. Difftest 549/549 at 1 and 4 threads for interp, auto, llvm, auto with `--tier-heat=1,0,0` and the Cranelift build's auto. Work-counter designs `copy_chain` and `copy_forward`. Where the rest of the fletcher gap is: `docs/decisions.md` (same entry, "Left").

### Release archives with and without the compiled tier (#780, 2026-10-27)

`release.yml` builds ONE archive per platform, `yavs-<version>-<target>` (since 2026-10-28; before that two, the second named `-nollvm`),
with the Cranelift tier (`--features clift`, `--tier=auto` by default, `compiled tier: cranelift`; nothing bundled). The LLVM archive
and the libLLVM build (`tools/release/build-libllvm.sh`, kept) are no longer part of the release; LLVM stays in the code and in CI.
`yavs --version` names the tiers built in. A `workflow_dispatch` dry run builds and smoke-tests all three archives (centos:7 for the Linux
ones, `--tier=clift` from the extracted archive) without tagging. Decisions: `docs/decisions.md` "Release archives with and without the compiled tier".

### Cranelift tier on main (#481, 2026-10-28)

The Cranelift tier of `spike/clift2` (rounds 1 to 11, `crates/yavs-clift`, the generator-generic `--tier=auto` policy in
`yavs-llvm`) is on main behind the `clift` feature, which is not a default feature; the release archives are built
with it. Builds without `clift` are unchanged: realbench `--compare` of bench builds of main and the merged head (097f20ab, then again at f774d440)
over 7 tests (`hdlm_fifo_sync`, `poc_arith_addw`, `uvvm_sb_slv`, `fletcher_bufferwriter_512`, `sc5_fifo_repack`,
`uvvm_axistream_vvc`, `neorv32_single_min`): default build `interp` instructions equal in every test (elaboration too); `llvm`
build instruction geomeans 1.00 `interp`, 1.00 `llvm`, 1.00 codegen, `auto` level within its run-to-run spread
(`uvvm_axistream_vvc` warm, medians of 7 counted runs: 28.3 G main, 27.0 G merged), no slowdown flagged. The round-11 rules (`Tune::big`, `Tune::frame`, `YAVS_FIRST_TUNE`) apply only with a first-level
generator (`Tune::first()` when `first` is set, else `Tune::MAIN`). Gates on the merged head: fmt; clippy `-D warnings` for
default, `llvm`, `clift`, `llvm,clift`; `cargo test --workspace` 1232 / 1355 / 1266 / 1386; gc-stress unit tests of
`yavs-interp`, `yavs-llvm` and `yavs-clift`; difftest 544/544 at 1 and 4 threads for `yavs-interp`, `yavs-auto`, `yavs-llvm`
(debug `llvm` build) and `yavs` (debug `clift` build, `--tier=auto` = Cranelift); `yavs-auto-warm` with the pre-key check
543/544 (`file_append_2019`, the same on main: #806). CI: the `clift` job (clippy, tests,
gc-stress, difftest smoke) and the `auto3` step. Flaky once: #805.

### Conformance report (#780, 2026-10-26)

`tools/conformance/run.py` generates `docs/conformance.md` (and `.json`): pass counts of the differential corpus per group and thread count, the reference simulators' own tests, the discrepancy catalogue, the analysis corpora against the recorded sim1 lists, the UVVM and OSVVM suites, neorv32 and GRLIB LEON3 runs, platforms and known limits. The `conformance` workflow regenerates the CI-capable parts on `main` and release tags and checks the committed copy (`tools/conformance/AGENTS.md`). Open: waveform equivalence on real designs (a slot in the report), a public page (maintainer's decision).

### Waveform equivalence against the reference simulators (#781, 2026-10-26)

`tools/waveq/run.py` runs a design on yavs (interp, auto and llvm tiers) and on the references and compares the
waveforms signal by signal, the value after the last delta of each time step (`tools/waveq/AGENTS.md`). Run locally at
05aa8453 (2026-10-27; `tools/waveq/RESULTS.md`, `results.json`): the 8 h2h RTL designs (sim1 and sim2), neorv32 (13 466 signals,
15.9 M value changes, 300 us), GRLIB LEON3 (13 897 signals, 33.2 M value changes, the whole 1.06 ms system test) and
the UVVM and OSVVM testbenches of realbench (sim1) are equal on all three tiers in every matched signal, except OSVVM
verification-component IDs and UVVM activity-register entries (process order within one cycle, catalogue
`process-order-in-cycle`). The UVVM AXI-Stream random valid/ready-low sequence used to diverge from 266.87 us (89
signals) because the BFMs seed it from the `'instance_name` signature; since #784 yavs spells it as sim1 and all
signals are equal (catalogue `subprogram-signature-instance-name`). In all: 44 824 of 44 889 signals equal. The llvm tier compiles 137 to 226 142 IR instructions per design;
auto compiles little in these short runs (LEON3 4 functions). sim2 runs the h2h designs only (no realbench keys). Found on
the way: #782 (literals accepted without visible character literals), #783 (integer width in yavs FST).

### Per-op costs of the interpreter, round 1 (#778, #520, 2026-10-27)

(`docs/decisions.md` "Per-op costs, round 1".) `SubtypeCheck` 136 to 161 instructions an op down to 87 to 129, `AllocCat` 354 to 319, `CallProt` 415 to 393, `DerefImm` 90 to 81; new ops
`LoadPkg`, `LoadPair`, `CheckK`. Whole runs (`--tier=interp`): `vai_window_gen_interlock` 430.9 to 410.5 G instructions, `poc_arith_addw` 145.7 to 137.8 G. New
`tools/stepcount --ops-lines OP`. Open: `CallIeee` marshalling (318 to 394), the frame fill of a call (proportional to the register count), `SubtypeCheck` below 50.

### Per-op costs of the interpreter, round 2 (#778, #730, 2026-10-27)

(`docs/decisions.md` "Per-op costs, round 2".) The optimiser keeps linear forms of values by register and version and the length of native `ieee` results (`to_unsigned`, `to_signed`, `resize`, `+`, `*`), so the `SubtypeCheck` of a slice of a computed width (`v(8 * i + 7 downto 8 * i)`) and of a `numeric_std` result into a signal goes: `poc_arith_addw` 138.6 to 137.0 G instructions, `vai_window_gen_interlock` 412.5 to 406.1 G, `uvvm_axistream_vvc` 33.2 to 32.7 G (interp); `realbench --compare` geomean 1.00. Work-counter ceilings of 20 designs back to the measured `ops` plus 3 percent. The frame fill (1.7 percent of cycles in `memmove`) and the `LoadDerefImm`/`AffP` chain were measured and left (reasons in decisions.md). Open: `SubtypeCheck` of a call result against a frame-resident target (`fletcher_bufferwriter_512`) and of register-sized widths (`uvvm_axistream_vvc`).

### Per-op costs of the interpreter, round 3: the native `ieee` calls (#778, #520, 2026-10-27)

(`docs/decisions.md` "Per-op costs, round 3".) `Op::CallIeee` keeps its argument registers and tries a reportless fast path of the native first (`ieee::lookup_fast`): the one-word paths of the `numeric_std` operators and `std_logic_1164` gates, the scalar `is_x`/`to_x01..`, the reductions, and new word-wise paths (`ieee_wide.rs`) for vectors of 65 to 1024 elements (`+`, `-`, comparisons, `to_integer`, `to_unsigned`/`to_signed`, `resize`, `sll`/`srl`, gates), read from the planes in place and written straight into temporary memory. `CallIeee` 323 to 220 instructions an op in `poc_arith_addw`, 398 to 332 in `fletcher_bufferwriter_512`, 300 to 230 in `neorv32_single_min`; whole runs (interp) `poc_arith_addw` 136.4 to 130.8 G, `fletcher_bufferwriter_512` 49.8 to 48.6 G, `neorv32_single_min` 29.0 to 28.75 G, compiled tier (`--tier=auto`, main thread) `fletcher_bufferwriter_512` -1.4 percent, `poc_arith_addw` unchanged (its operators are inline in compiled code). New differential test `native_wide_matches_bodies`. Open: the slice copy before a native (not fused: 1.5 percent of a NEORV32 start-up window).

### Compiled library code keeps its heap objects; native code published past never-run callees (#520, #797, 2026-10-27)

(`docs/decisions.md` "llvm: a heap change drops only the objects it changed ...".) Each heap logs the slots of its last 16 deallocations and
edits (`Heap::log`); compiled code drops only those objects from its object table instead of all heap objects at every deallocation, and a
call from the interpreter into native-frame code keeps the calling machine's objects (`TierHost::tier_state`). `--tier=auto` publishes
native-frame code whose native callees that never ran are not compiled (#797: such code stayed unused for the whole run in some runs, its
callers stepping out of compiled code). Window generator (`vai_window_gen_interlock`) warm 0.90 to 0.93 wall, 0.96 to 0.98 instructions,
cold level; object-table callbacks 3.75 to 1.30 million a run; a test-runner queue loop 18 to 16.4 G instructions and no slow mode.
`tools/realbench --compare` (42 tests): geometric mean 1.00 / 0.99 wall, 1.00 / 1.00 instructions (warm / cold). Against sim1 (run cell,
elaboration plus run, not the full total of the head-to-heads; two sessions of one core 4 minutes apart, sim1 itself 4.27 and 4.92 s):
warm 1.77 to 1.76 (instructions 1.69 to 1.67), cold 2.56 to 2.65 (instructions 2.57 to 2.61, the compile outcome of cold runs varies).
Left: protected-method calls of the random package stepped from compiled code, native steps of `FToIRange`, `LFill`, `LogicOp`, the remaining
object-table misses (decisions.md).

### `wait until <edge>` as cheap as the sensitivity-list forms (#767, 2026-10-26)

(`docs/decisions.md` "`wait until <edge>` costs what the sensitivity-list forms cost".) The end of a `wait until rising_edge(clk)` on one
element is one interpreter op (`WaitEdgeSuspend`), also as the copy a jump back to the wait becomes: 128 instructions an activation fewer, the wait form now within
0.6 percent of `process (clk) if rising_edge(clk)` in the interpreter and in `--tier=auto` (was +12 percent). Work-counter test `edge_cost_parity` holds the four
spellings equal. Open: the same process under `--tier=llvm` still pays a host call for the `WaitEdge` (#772).

### DAP work counters (#696)

`YAVS_COUNTERS` reports the kernel and interpreter work counters on stderr at the end of a `--dap`
session, as it does for a plain run. The scripted-session test also checks that a debugger keeps
signal activity and full event histories (`skip_signals=0`, `lean_signals=0`).

### Stuck and hung simulations (#582)

E3002 names the processes and signals of the loop and `--max-deltas` raises the limit; `--hang-report` and SIGUSR1 print what the waiting processes wait for (W3101, with stuck signals, their drivers and a stopped clock); W3102 warns about a process that can never suspend; `--shuffle` prints its seed. Tests: `crates/yavs/tests/stuck.rs`, `tests/cli/stuck_*`. Not done: the wait statement's place.

### Elaboration fixes (#494, #495, #498, #499)

A function declared in a block or architecture and called during elaboration reads the generics and constants around it, also a generic with a default and an override (#494, #499); the constants of a generate branch that was not elaborated are not evaluated (#495); an explicit `--exit-severity=failure` makes only failure reports count for the exit status (#498). Tests: `generics/elab_block_function`, `generics/unelaborated_generate_constant`, `crates/yavs/tests/cli.rs` (`run_explicit_exit_severity`, `run_function_reads_generic_override`).

### External test runner: its own VHDL suites (#112)

yavs as the simulator, `-p 2`, release build of main at f2ab69be (re-measured 2026-10-20); pass / fail / skipped tests per
suite (`vunit/vhdl/<suite>/test`), including `tb_axi_stream_protocol_checker` with its generic configurations. All 11 suites pass: 1357 tests, no failure, no skip (the one failure of the previous measurement, #281, is fixed). Wall time per suite at `-p 2` (host load average about 20): check 23 s, com 23 s, data_types 38 s, logging 16 s, string_ops 3 s, the rest under 6 s, verification_components 134 s with its protocol checker configurations (238 s at 666f5ee3, 224 s plus 20 s at b9dcddb6) (the previous measurement: 671 s at `-p 4`).

| Suite | Pass | Fail | Skip | Root causes (issues) |
|---|---|---|---|---|
| check | 257 | 0 | 0 | none (#281 fixed; the 13 skipped tests of `tb_check_stable` now run) |
| com | 145 | 0 | 0 | none (#246 fixed; re-measured after it) |
| data_types | 210 | 0 | 0 | none |
| dictionary | 8 | 0 | 0 | none |
| logging | 113 | 0 | 0 | none |
| path | 2 | 0 | 0 | none |
| random | 4 | 0 | 0 | none |
| record_dict (2019) | 6 | 0 | 0 | none |
| run | 5 | 0 | 0 | none |
| string_ops | 15 | 0 | 0 | none |
| verification_components | 579 | 0 | 0 | none (#251, #248, #252 fixed; all 52 `tb_axi_stream_protocol_checker` configurations pass) |

### Verification library: its own testbenches (#277)

`tools/suites/uvvm.py`, release build of main at bbe56abf (#460 fixed), `-j 2`; one run per testbench entity, architecture and generic set the component's own regression script lists (137 in all).
Pass = exit 0 and the library's final summary `>> Simulation SUCCESS`. 131 runs pass (130 before the I2C slave design could elaborate, #559; 102 before the SPI and I2C third-party designs were fetched and the driver fixes below; 78 at f2ab69be, 45 at ba53e23d); the whole suite takes 1.5 minutes at `-j 2`, the slowest run 12 s, so no run needs the 5 minute limit (the old over-5-minutes skips of #394 are gone: `axistream_vvc_tb` takes 4.4 s on core 2). Only the first root cause of each run is counted.
The remaining non-passes: none fails on yavs (#559 fixed: a signal initial value may read an `in` port, as the I2C slave design does) and six are skipped because vendor simulation libraries or designs are not obtainable (five `avalon_mm` runs: the testbench's FIFO, SPI and SDRAM-controller components stay unbound; one ethernet MAC run).
Driver fixes of this round (not yavs bugs): `--exit-severity=failure` on every run (a design under test reports errors by design, as the project's tool and a sim1 run), `func_cov_tb` runs share a directory and run in order (a test reads what an earlier one wrote), `questa_extension_tb` runs with its extension switch off (sim1 behaves the same).

| Component | Pass | Fail | Skip | First root cause (issues) |
|---|---|---|---|---|
| uvvm_util | 36 | 0 | 0 | none (the two former fails were driver expectations, above) |
| uvvm_assertions | 2 | 0 | 0 | none |
| uvvm_vvc_framework | 3 | 0 | 0 | none (#392 fixed) |
| bitvis_irqc | 2 | 0 | 0 | none |
| bitvis_uart | 3 | 0 | 0 | none |
| bitvis_vip_scoreboard | 3 | 0 | 0 | none |
| bitvis_vip_avalon_mm | 0 | 0 | 5 | skipped: the FPGA vendor simulation libraries and the SDRAM controller the testbenches instantiate are not obtainable, so their components stay unbound |
| bitvis_vip_avalon_st | 6 | 0 | 0 | none |
| bitvis_vip_axi | 2 | 0 | 0 | none |
| bitvis_vip_axilite | 2 | 0 | 0 | none (#391 fixed) |
| bitvis_vip_axistream | 22 | 0 | 0 | none: the 13 stream testbench configurations pass their unwanted-activity checks (#419 fixed by #391) and the 2 runs that were over 5 minutes now finish |
| bitvis_vip_clock_generator | 1 | 0 | 0 | none |
| bitvis_vip_error_injection | 2 | 0 | 0 | none |
| bitvis_vip_gmii | 2 | 0 | 0 | none |
| bitvis_vip_sbi | 2 | 0 | 0 | none |
| bitvis_vip_ethernet | 3 | 0 | 1 | none (#460 fixed); 1 skipped: the vendor simulation libraries of the MAC testbench are not obtainable |
| bitvis_vip_gpio | 1 | 0 | 0 | none |
| bitvis_vip_i2c | 1 | 0 | 0 | none (#559 fixed) |
| bitvis_vip_rgmii | 2 | 0 | 0 | none |
| bitvis_vip_spec_cov | 1 | 0 | 0 | none (#393 fixed) |
| bitvis_vip_spi | 24 | 0 | 0 | none (the master and slave designs are fetched at a pinned commit) |
| bitvis_vip_uart | 3 | 0 | 0 | none (the demo run reports errors by design, driver fix above) |
| bitvis_vip_wishbone | 2 | 0 | 0 | none (#391 fixed) |
| external_vip_apb | 6 | 0 | 0 | none (#391 fixed) |
| Total | 130 | 1 | 6 | per-component rows above are from this commit |

The generated-VIP tests of the framework component (generator output compiled and simulated) are not run by the driver.

### Verification library: its own `.pro` regression (#276)

`tools/suites/pro.py` reads the library's build and test scripts (`OsvvmLibraries.pro`, `RunAllTests.pro`) with a Tcl subset, analyzes
521 files in script order and runs each test (`yavs run`, release build of main, 2 in parallel). Pass = exit 0 and the
library's own `DONE PASSED` summary line. The script settings package body is generated with the scripts' release as settings
revision, as the scripts do (the transcript layout the tests compare against the validated results follows it). 2008 (re-measured at f2ab69be, 200 s wall at 2 in parallel; 178 s at b9dcddb6) and 2019 (at f2ab69be, 218 s wall):
all 314 tests pass, the same test for test (2019 passed 272 of 314 at b9dcddb6, the #416 overflow in the native `to_integer`, fixed; it fell to 129 of 314 between fd35a265 and 33e5f57e, #429, fixed); the native co-simulation directories (`CoSim`, `CoSimPCIe`) are skipped, not run.
Root causes fixed on the way: #291, #292, #294, #295, #296, #304, #311 (= #313), #314, #315 (= #312).

### Reference simulators' own regression tests, run time (#354)

`tools/difftest/external.py` runs the self-checking regression tests of the two reference simulator
suites in place from the corpus cache, on yavs (release build) and on the reference simulator
that owns the suite, and classifies each test (`tools/difftest/README.md` "External regression
tests"). Tests for an earlier standard run as 2008 on yavs. Counts on main at fc46f5ed (before)
and after #365 to #368 (2026-10-19):

| Suite | Runnable | Reference bad here | pass | wording only (`fatalmsg`) | order | other output | outcome | elab | analysis |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| first (`external.py sim1`), before | 1070 | 20 | 791 | 52 | 3 | 37 | 64 | 52 | 51 |
| first, after | 1070 | 20 | 798 | 52 | 3 | 39 | 55 | 52 | 51 |
| second (`external.py sim2`), before | 504 | 30 | 377 | 0 | 0 | 21 | 16 | 19 | 41 |
| second, after | 504 | 30 | 381 | 0 | 0 | 21 | 12 | 19 | 41 |
| second, after #447 to #461 (2026-10-20) | 504 | 30 | 392 | 0 | 0 | 22 | 9 | 15 | 36 |
| first, after #450 to #476 (2026-10-20) | 1070 | 20 | 839 (+11 `early`) | 57 | 3 | 38 | 48 | 26 | 28 |

Run-time findings of the first suite, fixed on 2026-10-20 (#450 to #476; the pass list grows from 813 to 850 tests):
`'active` of an element behind a port (#450), `'last_value` of a composite signal as a whole (#451, the first
reference answers per net; catalogue `last-value-composite`), a use clause naming a type makes its
explicit operations visible (#452, L106), VHDL-93 shared variables read with `--relaxed` (#454),
`early` (an LRM error found at analysis or elaboration where the suite has the run fail, #456),
files declared in an architecture (#466), binding indication actuals see the names of the binding
(#468), IEEE function calls and slices locally static (#470, L109), a package body naming an item of
its package (#471), a block configuration seeing the components of its block (#473), a port actual
whose context subtype names the instance's generics (#475) and 2019 record conversion (#476). Still
open in the first suite (102 tests): PSL beyond the subset (psl11, strong operators; 21 of its 24 PSL tests pass), mirror identity in reflection
(4), guarded assignments (6, #373, #352), `T'value` of composites (2019, 4), `'driving` of
sub-elements and ports (3), aggregate targets that mix scalars and arrays (2), VHDL-93 code that
declares `integer_vector` or `boolean_vector` read as 2008 (4), external name subtype checks (ename5,
ename9) and the missing length checks of a case expression (case13) and of an index constraint
(bounds9), a resolution function that is an alias of an implicit function (wait21), nested
subprograms of generic subprograms (attr19), partially constrained element subtypes of port actuals
(issue1253, issue1366, vests37), `'range(n)'low` and `[signature]'instance_name` (attr16, attr21) and
the rest one by one.

Skipped by the harness: 403 and 647 (other languages, VHPI, coverage, Tcl and shell scripts,
second-suite scripts with control flow or tool options, negative analysis tests). The passing
tests are listed in `tools/difftest/external-{sim1,sim2}.pass`; the CI `corpus` job checks them
against the suites' own verdicts (exit status) without a reference simulator. Every listed test gives
identical output at `--threads 1` and `--threads 4`.

Correctness fixes found by reviewing the non-pass entries (2026-10-23):
- `x'element` of an array formal with open element bounds as the target of an `others` aggregate (`issue3204`, second suite): the aggregate takes the actual's element bounds.
- An aggregate with `others` as a case choice takes the case expression's subtype (9.3.3.3; `issue1360`, second suite).
- `--relaxed` rule R5 (`W1325`): an `others` aggregate assigned to an unconstrained port or formal takes the object's run-time bounds, as both reference simulators do under their relaxed switch (`issue1191`, `issue2421`, second suite). The harness reads `--std=93c` as relaxed.
- `--relaxed` rule R6 (`W1361`, #813): the actual of a signal parameter that is not a static name (`p(s(i))` with a variable or loop parameter `i`) is a warning instead of `E1360`, as the first reference simulator does under its relaxed switch.
- `x'subtype(value)` (and `'element`, `'base`) is a type conversion (9.3.6; `ieee9`, first suite, `issue2097`, second suite).
- The function form of `file_open` returned no value (internal error), and `file_position (f, FILE_ORIGIN_END)` was negative (a NATURAL error); it is the distance to the end now (`file12`, first suite).
- `T'VALUE` of an array type (2019, `value2`, `value4`) is E2001 (not implemented) instead of an internal error; `T'VALUE` of composites is still missing.
- An aggregate target with parts of a signal parameter (`issue3030`) is E2001 (not implemented) instead of an internal error.
- A concurrent procedure call with mode view parameters is sensitive to the elements the view reads (`view6`, first suite).
- A deferred constant of a package instance in an architecture read as zero (`genpack16`, first suite); it reads the body's full declaration.
- An alias with a partly constrained subtype (`alias y : arr(0 to n - 1) is x`, arrays of open arrays) keeps the aliased object's index range (`issue932`, first suite); it has its own index range now.
- A record constant with open fields kept the bounds of its initial value (`issue1274`, first suite); it has its subtype's field bounds, in analysis (folded values) and in elaboration (called values).
- `x'attr(1)` of a user-defined attribute of an array type is an index of its value (`attr11`, first suite).
- `p.st` (the label of the enclosing process or block as prefix) as a type mark and a loop range (`issue713`, second suite); `issue543` passes with the attribute index above.
- `s => "not"(a)` in a port map is an expression actual: a call of a predefined operator has no body to convert with, so it is not read as a conversion function (`bug0134`, second suite).
- `process (all)` is sensitive to the signals read by the subprograms declared in the process (`bug0129`, second suite).
- `issue1267` (first suite) leaves the pass list: its `case` runs an 8-bit choice list against a 16-bit expression and relies on the `others` branch; the length check of #463 (LRM 10.9) makes that a run-time error, as the first reference simulator does elsewhere (`bounds` tests).
- `vests1` and `vests7` (first suite) leave the pass list (2026-10-28, #846): these copies of VESTs `tc517` and its neighbours move function bodies that return a deferred constant before the constant's full declaration in the package body, which LRM 6.4.2.2 forbids and yavs now reports as `E1226` (the second reference simulator rejects both files at analysis, at an earlier construct); the VESTs originals keep the order and pass. `signal28` stays: its non-static signal actual is accepted under `--relaxed` (R6, `W1361`).
- PSL sequences, named sequences and properties and weak temporal properties run (#445): of the first suite's 24 PSL tests 21 pass, `psl11` fails on `eventually!` and two are not run by the harness, and `issue1347`, `issue1708`, `issue3134`, `ticket18` and `ticket92` of the second suite join the pass list; `issue2157` leaves it (its `a and b = x"4"` mixes a `std_logic` and a `boolean` operand, which the first reference simulator rejects as well). `psl12` (first suite) leaves the pass list on 2026-10-23: its `async_abort` condition pulses between two clock edges and the reference simulator aborts then, while yavs checks `abort` and `async_abort` at the tick (user decision, #636).
- Pass lists refreshed 2026-10-23 (4cd58590, release build, `--oracle --write-pass`): first suite 883 to 992 tests (110 added, `psl12` removed), second suite 406 to 455 (49 added, none removed); `--oracle --check` takes 4.7 s and 4.4 s with 4 jobs (was 4.1 s and 3.7 s).

Regression guard 2026-10-19 (b9dcddb6): the pass lists check green with `--oracle --check` (813 first-suite and 382 second-suite tests pass), the differential corpus passes 349/349 at 1 thread, 4 threads and 4 shuffled threads, all analysis corpora match their expected lists and the discrepancy catalogue checks green.

Regression guard 2026-10-20 (f2ab69be, release build with the `llvm` feature): the pass lists check green with `--oracle --check` (813 first-suite and 382 second-suite tests), the differential corpus passes 354/354 at 1 thread, 4 threads and 4 shuffled threads on the interpreter and, at 1 and 4 threads, with `--tier=llvm`, all 30 analysis corpora match their expected lists, the discrepancy catalogue checks green, the external test runner's 11 suites pass (1344 tests), the second verification library's `.pro` regression passes 314/314 under both 2008 and 2019 and the verification library suite runs 78 pass, 33 fail, 26 skip (no new failure). One regression was found on the way and fixed already: 185 of its 314 tests under 2019 failed between fd35a265 and 33e5f57e (#429).

Regression guard 2026-10-20, second (4d7b56bb, after the predecoded interpreter and calls, the IR optimiser, kernel round 3, narrow integers and the startup, analysis and elaboration work; release build with the `llvm` feature, host load average about 20): no new failure. The differential corpus passes 362/362 at 1 thread (18 s), 4 threads (35 s) and 4 shuffled threads (36 s) on the interpreter, with `--tier=auto` (21 s, 40 s) and with `--tier=llvm` (189 s, 398 s); the pass lists check green with `--oracle --check` (813 and 382 tests; 43 s and 30 s); all analysis corpora match their expected lists; the discrepancy catalogue checks green; the external test runner's 11 suites pass (all tests, `-p 2`; check 13 s, com 13 s, data_types 19 s, logging 8 s, verification_components 81 s, the rest 3 s or less); the second verification library's `.pro` regression passes 314/314 under 2008 (153 s) and 2019 (168 s); the verification library suite runs 78 pass, 33 fail, 26 skip (973 s at `-j 2`, 1240 s before), the same totals as before. The VHPI conformance traces are unchanged except that `libapp` now shows the library and application foreign models running (#357); the Python-framework VHPI regression gives 66 passing and 2 failing test directories, as before.

Regression guard 2026-10-20, third (78b1c398, release build with the `llvm` feature): the differential corpus passes 392/392 at 1, 4 and 4 shuffled threads on the interpreter (16 s, 33 s, 31 s), with `--tier=auto` (14 s, 28 s) and with `--tier=llvm` (166 s, 292 s); the pass lists check green with `--oracle --check` (839 and 393 tests); all analysis corpora match their expected lists; the discrepancy catalogue checks green; the second verification library's `.pro` regression passes 314/314 under 2008 (111 s) and 2019 (110 s); the verification library suite runs 102 pass, 9 fail, 26 skip (736 s at `-j 2`); the VHPI conformance traces and the Python-framework VHPI regression (66 passing, 2 failing test directories) are unchanged. One regression: the external test runner's suites `check` (1 of 257, `tb_check_stable`) and `verification_components` (7 of 579, mocked log items of the AXI slave and stream protocol checker tests) fail since 45d6d07b (resumption order of several-wait processes, #417), open as #462; the other nine suites pass.

Regression guard 2026-10-21 (ab4c4e6b, after the day's performance commits; release build with the `llvm` feature and a second release build with `llvm` and `gc-stress`, all suites pinned to cores 8 to 23): no regression. The pass lists check green with `--oracle --check` on the interpreter and with `--tier=auto` (857 first-suite and 393 second-suite tests). The differential corpus (416 tests, each run three times in one work directory so the second and third are warm; `textio/file_append_2019` appends to a file and is the one expected difference, 415/416 otherwise) passes at 1 thread, 4 threads and 4 shuffled threads on the interpreter (12 s), `--tier=auto` (16 s), `--tier=auto --tier-heat=1,0,0` (21 s) and `--tier=llvm` (283 s), and on the `gc-stress` build on the interpreter (409 s) and `--tier=llvm` (485 s). All 30 analysis corpora match their expected lists; the discrepancy catalogue checks green for yavs (192 case and standard rows; no reference simulator was available for the recorded-verdict drift check); the external test runner's 11 suites pass (1344 tests, `-p 2`); the second verification library's `.pro` regression passes 314/314 under 2008 and 2019 on the interpreter and with `--tier=auto` (22 s each); the verification library suite runs 102 pass, 9 fail, 26 skip on both (76 s at `-j 2`); `cargo test --workspace` (788 tests) and `cargo test -p yavs-llvm --features llvm` pass.

Remaining run-time causes, ranked by tests (yavs bugs have issues): reports in resolution and
conversion functions during the run print `(init)` and bypass the report path, #372 (12);
unsupported PSL beyond the subset, W0004 (17, plus 4 that differ only in the directive path);
missing run-time checks (#463, fixed 2026-10-23: record aggregate elements against generic-dependent fields, array case lengths, external name subtypes, integer literals outside INTEGER); PSL-free reference quirks and LRM-unclear cases (about
15: mirror identity in reflection, null array bounds, `'transaction` start value 2 to 2, textio
read of bits with spaces, real image format of the second suite, environment and time stamps);
ICE on arrays with element constraints, #380 (3); access to protected types, #377 (2); predefined operations on arrays of
composites (2); aggregate longer than a non-static constant subtype is an ICE, #378 (2); failed
textio READ stops the run, #381 (1); a repeated force in one cycle, #382 (1); files of subprograms not closed on return, #375 (1); and single
tests (alias and partially constrained record bounds, `hread` digit count, an impure function in
a type evaluated twice, `log2` precision, 2019 mode view parameters). Analysis and elaboration
failures (about 160) are mostly "not lowered yet" (`E2001`) features and VHDL-93 code read as
2008; the analysis corpora cover them.

Run-time fixes found by this harness (2026-10-20): a file declared in a subprogram is closed when the
subprogram returns (#375); a positional aggregate longer than the non-static subtype it initialises is a
run-time length error, not a write outside it (#378); a textio READ that fails without GOOD reports an
error and the run goes on, as three of the four references do (#381, L107, case `textio-read-failure`); a
locally static aggregate whose array elements differ in length is no longer folded (#380; under a context with ranges it is #455); the method call through an access to a protected type (#377) and the read outside a
constant of an array with element constraints (#380) no longer reproduce on main (regression tests
`protected/access_method_call_2019`, `composite/aggregate_element_lengths`; `issue3010` joined the second
suite's pass list).

## performance (`tools/bench`, #387)

The shared benchmark harness (`tools/bench/AGENTS.md`) measures every performance change the same way:
`python3 -I tools/bench/run.py --compare A B`. Phase times come from `yavs run --time-report`. Testbenches of real projects (41, against sim1 and between binaries): `tools/realbench`. Analysis cells are batched calls since #745 (section "Analysis in batched calls"; tables before it are one call per file and not comparable). Current comparison with sim1 (rule 23: `--tier=auto`, total wall incl. analysis, cold and warm): section "Real-world performance suite: yavs against sim1, sim2 and sim4, head-to-head 8, 2026-10-28" (0.20 warm, 0.58 cold with the LLVM build, 0.21 and 0.57 with the Cranelift build; head-to-head 7 and 6 below it are the earlier ones), on the batched-call analysis basis (#745, both tools analyse consecutive files of a library in one call; head-to-head 1 to 5 were one call per file and are not comparable): geomean of the 41 realbench totals (libraries analysed too) 0.22 warm, 0.64 cold (the same run on the old per-file basis 0.24 and 0.42; head-to-head 5 had 0.23 and 0.43); own files + elab + run 0.23 and 0.49, elab + run 0.23 and 0.47, executed instructions yavs / sim1 0.31 and 0.60, tiny-test corpus 0.11 and 0.25, the h2h designs 0.56 and 0.78. The tables below are older measurements on the earlier host.

Baseline of main at the commit that added the harness (release build, Intel Core Ultra 9 275HX, 24 CPUs,
host shared with other agents; median CPU seconds of 3 runs, phase CPU in 10 ms steps, `--threads 1`):

| Benchmark | analyze | elaborate | init | simulate | total CPU |
|---|---|---|---|---|---|
| `compute_loop` | 0.00 | 0.00 | 0.00 | 0.96 | 0.97 |
| `ff_rtl` | 0.00 | 0.03 | 0.00 | 1.51 | 1.56 |
| `resolved_bus` | 0.00 | 0.01 | 0.00 | 0.59 | 0.61 |
| `delta_chain` | 0.00 | 0.00 | 0.00 | 0.52 | 0.53 |
| `textio_heavy` | 0.00 | 0.00 | 0.00 | 0.59 | 0.60 |
| `protected_scoreboard` | 0.00 | 0.00 | 0.00 | 0.44 | 0.45 |
| `elab_heavy` | 0.00 | 0.19 | 0.58 | 0.60 | 1.60 |
| `vunit_vc30` | 8.61 | 10.32 | 4.37 | 3.04 | 27.03 |
| `uvvm_axistream_vvc_tb` | 1.56 | 0.22 | 0.09 | 6.99 | 8.89 |
| `uvvm_util_rand_tb` | 1.48 | 0.08 | 0.01 | 30.85 | 32.53 |
| `vai_pe_array` | 0.29 | 0.25 | 0.10 | 8.78 | 9.49 |
| `vai_window_gen` | 0.30 | 0.30 | 0.11 | 5.03 | 5.77 |
| `osvvm_axistream` | 0.44 | 0.12 | 0.00 | 3.41 | 3.99 |

Reading it: `vunit_vc30` is 30 `yavs run` calls, so its analyze and elaborate columns are the per-process
start-up cost times 30 (analysis of the design closure is a third of it); `uvvm_axistream_vvc_tb` is capped at
10 us of simulated time (the full run takes minutes); `elab_heavy` is the elaboration and initialisation stress
(20000 instances). Findings while building it: a numeric_std-style flip-flop lane costs about 150 us CPU per
process activation (`ff_rtl`, 512 lanes x 400 cycles takes 29 s); for the 30 short test cases of `vunit_vc30`
86 percent of the CPU time is spent before the first delta (analyze 32, elaborate 38, init 16 percent).

Init phase (#405): `vunit_vc30` spent its init time in package initialisation and the replay of
elaboration-time calls, and most of the package initialisation in VHDL code that fills long arrays one element
at a time (`new T(0 to 65536 => null)`, the default value of an array of records: 10 to 70 interpreted
instructions per element). Equal elements are now written by one `Fill` or by doubling `Copy` (lowering, see
`crates/yavs-lower/AGENTS.md`). Median CPU seconds, `--threads 1`, 3 to 5 runs:

| Benchmark, phase | before | after (this change plus the package state reuse of elaboration) |
|---|---|---|
| `vunit_vc30` init | 4.20 | 0.12 |
| `vunit_vc30` elaborate | 10.56 | 7.70 |
| `vunit_vc30` total | 27.89 | 20.92 |
| one verification-component test, package initialisation | 128 ms | 14 ms |

`elab_heavy` init (0.60 s) is the kernel model build (`Builder::build`, about 0.54 s of it for 20000 instances), not
package or instance data; it belongs to the kernel work (fixed by #411, see "Simulation kernel" below: 0.26 s). The UVVM and OSVVM benchmarks have no init cost (0.01 s).

### Memory footprint (#409)

Peak RSS is now in every harness result (`peak MiB`, measured by `time -f %M`). Findings and fixes, in the
order they were found with a counting heap profile:

- A testbench whose main process never waits (a queue and random-value testbench) runs in one delta cycle,
  and the shared heap was collected only at the start of a cycle: its slot table and dropped objects grew to
  400 MB. The heap is now also collected inside an activation, before a `new` or a native call, once the
  simulation has started (`Machine::gc_inline`, `docs/decisions.md` "Shared heap collection").
- A plain `yavs run` keeps the analysis results (HIR, scopes, lookup caches: about 30 MB for a project
  testbench) until it exits though it needs only the source map: `Db::release_analysis` frees them before the
  runtime is built (runs with a plugin, a waveform or coverage keep them). The package state elaboration leaves
  for the simulation to start from was copied twice; it is now moved.
- A suspended process held a register file of its own (1.6 KB for a small entity, dead between activations):
  at a suspension in the root frame it goes to a per-thread scratch (`Machine::release_scratch`).

Peak RSS (MiB) and total CPU (s), median of 3, release build, `--threads 1`; before is main at `a561b34a`,
after is main at `47407de9` (the CPU difference is noise plus other agents' commits in between):

| Benchmark | peak before | peak after | CPU before | CPU after |
|---|---|---|---|---|
| `uvvm_util_rand_tb` | 297 | 46 | 5.74 | 5.72 |
| `elab_heavy` | 388 | 335 | 0.63 | 0.57 |
| `uvvm_axistream_vvc_tb` | 121 | 93 | 5.20 | 5.25 |
| `vunit_vc30` (largest of 30 runs) | 97 | 74 | 6.73 | 5.09 |
| `vai_window_gen` | 79 | 53 | 2.09 | 2.05 |
| `vai_pe_array` | 65 | 51 | 0.31 | 0.27 |
| `osvvm_axistream` | 60 | 53 | 2.01 | 2.04 |
| `ff_rtl` | 20 | 14 | 0.07 | 0.05 |

Precise roots (#422, user decision 2026-10-19): the collection inside an activation no longer scans frames,
registers and temporaries for words that look like handles or addresses. Roots are the frames' root maps, the
temporaries `Inst::TempRoot` recorded and the `Handle`/`Ptr` registers live at each frame's safepoint
(`yavs_interp::roots`, a liveness analysis per function computed when a collection first meets it: about 4 ms
for the largest function of `uvvm_util_rand_tb`, 46k instructions); a suspension no longer searches every word
of the frames (`Machine::pinned`, about 4.5% of `uvvm_axistream_vvc_tb`). Verified with the `gc-stress` build
(collection at every safepoint after an allocation, no slot
reuse, a panic for a live handle in a register of another type): the differential corpus at both tiers
(361 of 363, the two `synopsys/arith` tests exceed the 300 s harness timeout under stress and pass when run
directly, also with collection at every safepoint), 212 of the 314 library `.pro` tests (all pass, the rest
not reached within the time cap) and the external test runner's `check` suite (these two on the tree before #438). Median of 9, release, A = main
at `4d7b56bb`, B = this change, total CPU B/A: `uvvm_util_rand_tb` 0.92, `protected_scoreboard` 1.00,
`osvvm_axistream` 1.05, `vunit_vc30` 1.03, `uvvm_axistream_vvc_tb` 0.96, `compute_loop` 0.99, `strings_heavy`
1.01, `access_heavy` 1.03 (geometric mean 0.998, busy host: single ratios move by about 5% between runs);
peak RSS equal or up to 3% lower.

Where the rest is: a project testbench peaks at the end of elaboration with the analysis results (about 29 MB,
restored from the compilation cache: 128 B per declaration, 56 B per expression, 152 B per statement), the
lowered IR (about 11 MB) and the package state; the run itself adds little. A design of 20000 small
instances needed about 17 KB per instance (kernel signals about 1.2 KB each with 25 allocations, the
elaborated model about 9 KB per instance; see "Memory of designs with many instances (#421)" for what is left); the shared heap of a long verification testbench is genuinely
live data (about 20 MB). A global allocator that keeps freed memory mapped (one was tried) doubled the peak.

### Zeroed large arrays (#486)

A random-cyclic testbench of a verification library peaked at 567 MB (`rand_cyclic_performance`, also with
`--no-cache`). Breakdown with a counting run: 54 MB is the floor of the library (`rand_report`), 33 MB the
queue and the 100000-value list of the later steps, and 480 MB one `std_logic` array of 2^29+1 elements
(one byte each) that the testbench allocates and marks sparsely. `Inst::NewFill` built every fill with
`Vec::resize`, touching all pages; an all-zero fill (`U` is code 0) is now `vec![0; n]`, a zeroed
allocation whose pages the OS maps on first touch. Peak RSS 567 to 87 MB, same output, same run time
(3.2 s pinned). The reference simulators allocate the same array: sim1 reaches the 2 GB cap, sim2 1.1 GB.
`bit` and `boolean` arrays have code bias 2 (default `0`/`false` is byte 2), so they are still filled
eagerly. Test: `crates/yavs/tests/rss.rs` (Linux).

### Memory of designs with many instances (#421)

`elab_heavy` (20000 instances: 40002 kernel signals, 40001 processes) peaked at 271 MiB, with 1.82 million live
allocations at the peak and 7.07 million allocations over the run. Counting heap profile (a counting global
allocator in a scratch build) showed where it went, and the changes:

- A kernel signal was a 584-byte record with about 25 allocations (three value buffers, four per-element stamp
  vectors, waiter lists, segments, driver list, scratch range lists). Buffers of up to 16 bytes or one logic block, the
  four stamp arrays (now one buffer), one waiter, one segment, one pending transaction and the scratch lists are
  stored inline (`SmallVec` with its `union` feature); the several-sources source map is freed before the
  simulation state is made; process names are stored once (the printer asks the kernel for them).
- A resolved signal allocated three lists for every driving value it computed (`resolved_bus` simulate CPU 0.51 s
  to 0.38 s on its own); an unguarded signal now resolves its one run with an inline list. The elaborator boxed
  each process twice (a wrapper around the builder's box, and a second virtual call per activation):
  `Builder::process_boxed`.
- `yavs run` without a plugin, waveform or coverage frees the elaborated model once the simulation is built
  (`Elaborated::release_for_run`): the run phase grows about 25 MiB (frames, queues, wait lists of 40000
  processes) and reuses what the model held. The release itself costs about 0.03 s CPU on this design.
- Elaboration: path names were built from three allocations per segment (1.3 million allocations), the
  several-sources check formatted a description of every driver and port (140000 strings) though it needs one only
  on a collision, per-scope values were hash maps of one to three entries, the port-expression data of a process
  instance (130 bytes) was inline.
- The shared-state field of every interpreter process held an empty shared heap (184 bytes): boxed (64).

Median of 7 runs, release build, `--threads 1`, interleaved A/B (`tools/bench/run.py --compare`), before is main
at that commit with the commits of #421 taken out (a scratch tree, so the other agents' commits in between are in both
binaries):

| Benchmark | peak MiB before | after | CPU s before | after |
|---|---|---|---|---|
| `elab_heavy` | 270.1 | 159.2 | 0.44 | 0.36 |
| `vai_pe_array` | 49.8 | 49.9 | 0.18 | 0.18 |
| `vai_window_gen` | 50.9 | 51.0 | 1.50 | 1.45 |
| `ff_rtl` | 12.9 | 12.8 | 0.04 | 0.05 |
| `resolved_bus` | 11.0 | 10.8 | 0.30 | 0.30 |
| `uvvm_axistream_vvc_tb` | 90.2 | 89.4 | 2.98 | 2.95 |
| `vunit_vc30` | 71.6 | 71.7 | 3.84 | 3.72 |

`elab_heavy` phases (CPU s, before / after): elaborate 0.14 / 0.12, init 0.16 / 0.12, simulate 0.11 / 0.09; live
allocations at the peak 1.82 million / 0.74 million, allocations over the run 7.07 million / 3.78 million. The
memory is first-touch cost too: a third of the elaboration samples sat in the copy that writes a new element of
the scope, object and leaf vectors into fresh pages. A 184 microsecond AXI-stream testbench of the verification library benchmark (200 s) runs in the
same time before and after (206 s against 223 s one at a time, 204 s against 196 s side by side on a loaded host:
repeat runs of long tests differ by that much).

Where the rest is (20000 instances, resident bytes at the end of the build, before the model is freed): the kernel
signals (600-byte record, 40000 of them: 24 MiB), the interpreter processes (560 bytes each: 22 MiB), the
elaborated hierarchy (152-byte scopes, 144-byte objects, 152-byte leaves, 80000 of each: 36 MiB), kernel process
slots and queues (13 MiB). Not done, and why: the interpreter machine of a process (private heap 128 bytes, layout
memos, wait list) belongs to the heap work of #422 and the dispatch work of #412, which edit the same lines;
sharing per-architecture data between instances (shapes, port tables) needs the elaborator to build the
instance's constants lazily, a larger change.

### Narrow integer storage (R12, #415)

Elements of arrays of an integer type within 32 bits, and kernel signals of such types, take 4 bytes instead of 8
(`docs/decisions.md` "Narrow integer storage"). Release builds, `--threads 1`, host shared with other agents (load
about 10); before is main at the commit below it, after is #415:

| Run | peak before | peak after | CPU before | CPU after |
|---|---|---|---|---|
| a 4M-element `integer` array variable written and summed 4 times, interpreted | 123 MB | 91 MB | 6.97 s | 7.08 s |
| the same, `--tier=llvm` | 142 MB | 109 MB | 0.44 s | 0.45 s |
| `tools/bench` (7 benchmarks: the internal scoreboard, compute, text and elaboration designs, `uvvm_util_rand_tb`, `osvvm_axistream`, `vunit_vc30`), median of 5 | unchanged (`vunit_vc30` 120.7 to 119.1 MiB) | | geometric mean of total CPU 1.006 (noise) | |

The library testbenches hold few large integer arrays, so their footprint does not change; 1- and 2-byte elements
(step 2 of R12) are not worth doing for them.

### Interpreter run time (#386)

Interpreter hot paths (`crates/yavs-interp`, commits of #386): constant expansions and layouts
reused instead of recomputed, logic elements read and written in place, dispatch with the function
looked up once per frame change and `step` inlined, calls without allocations (frame vectors and
argument cells reused), the top frame's registers held in the machine. Release builds, CPU seconds
(user + sys), host shared with other agents (load 15 to 30). Before: main at `4a551b39`; after: main
at `bc74c892` (which also contains other agents' commits of the same period).

Long verification-library tests (one run each; previously skipped at the 5-minute cap):

| Test | before | after | ref-A | ref-B |
|---|---|---|---|---|
| `rand_tb` `rand_gaussian` | 928 | 268 | 1.9 | 8.6 |
| `rand_tb` `rand_cyclic_performance` | 601 | 179 | needs a heap over the 2 GB cap | 4.9 |
| AXI-stream VVC testbench, first generic set, to 20 us | 35.8 | 13.2 | | |

`tools/bench --compare` (median CPU of 3 runs, total including analysis): `compute_loop` ratio 0.36,
`ff_rtl` 0.37, `resolved_bus` 0.48, `delta_chain` 0.32, `textio_heavy` 0.56, `protected_scoreboard` 0.36,
`uvvm_axistream_vvc_tb` 0.36 (23.8 s to 8.7 s), `uvvm_util_rand_tb` 0.38 (62.6 s to 23.7 s), `vai_pe_array`
0.36. The difftest corpus is unchanged at `--threads 1` and `4`. What is left per instruction is
dispatch itself (about 5 ns for a simple instruction, 8 ns in call-heavy code); most of the time of these
tests is spent inside a few ieee subprograms run from their VHDL bodies, which R1 of
`docs/design/performance.md` addresses (#395).

Analysis speed (#405), one `yavs run` of `vunit_vc30` (`tb_axi_stream`, one test case; median CPU seconds of 7
interleaved runs, release build, loaded host): analyze 0.49 -> 0.19, total 1.65 -> 1.15. Changes: overload sets
indexed by profile shape instead of pairwise scans (no profile vectors per comparison), an epoch-validated lookup
cache (`Db::lookup`; tracked arenas bump `ids::epoch()`, debug builds assert every hit against a fresh lookup),
a read-lock interner and a fast hasher for the scope, memo and cache tables, cached operator and character keys.
Still open: parse is about 8% and subprogram bodies of units that are never called about half of the remaining
analysis; analyzing bodies on demand needs a scope snapshot per body (visibility is positional) and is not done.

Cold analysis, allocation and copying (#424). A cold `yavs analyze` re-reads the source of every unit it
depends on, so parsing is about half of a library run and the allocator is the next cost (a sampling profile
of the process showed `malloc` at 15 to 20% of the time). Changes: ASCII identifiers are scanned and
lower-cased on the stack and keywords are looked up by first letter (no `String` per identifier), source text
skips the lossy UTF-8 decoder, the symbol table has a per-thread copy (a hit or `as_str` takes no lock),
list parsers (statements, declarations, interface and association lists) build on a scratch stack and move
into exact-size vectors (no doubling of 200 to 400 byte elements), the rare large AST variants are boxed
(association element 432 -> 184 bytes, declaration 560 -> 424, sequential statement 456 -> 344, interface
declaration 552 -> 416), mangled names are built in one buffer, a visibility lookup keeps its entity and
shape sets in small vectors and iterates used scopes in place, the lookup cache and interner start
presized, and `yavs analyze` and `yavs elab` do not free the database at exit. Median of 5 interleaved
runs, release build, host load about 25, one `yavs analyze` per file as in `tools/corpus/run.py` (the
manifest's units are re-analyzed per call; sum of user + system CPU, largest peak RSS of any call):

| Library set (files) | CPU before (s) | CPU after (s) | Peak RSS before (MiB) | Peak RSS after (MiB) |
|---|---|---|---|---|
| osvvm-2008 (174) | 10.82 | 6.95 (-36%) | 60 | 36 |
| uvvm-2008 (235) | 14.54 | 9.23 (-37%) | 56 | 33 |
| open-logic-2008 | 12.24 | 8.41 (-31%) | 48 | 35 |
| vunit-2008 (143) | 2.65 | 1.81 (-32%) | 39 | 27 |
| neorv32-2008 (67) | 0.43 | 0.27 (-37%) | 22 | 19 |

`vunit_vc30` with `yavs run --no-cache` (analyze phase, 30 test cases, `tools/bench/run.py --compare`, 5 runs):
5.49 -> 5.08 s CPU (-7%; 6.23 -> 5.58 and 6.67 -> 5.95 in two earlier runs). That workload is about 75%
semantic analysis (bodies, visibility lookups: 40% of lookups are uncached, 78% of those first-time keys),
so the lexer and parser changes help it less. Diagnostics of all analysis corpora (3881 calls) are
byte-identical before and after, the difftest corpus passes at `--threads 1` and `4`. Left: visibility lookups
(`Db::lookup_uncached`, about a fifth of the analysis of the `vunit_vc30` case), arena pushes of HIR nodes, and
the global allocator (a bump or thread-caching allocator gave 8 to 15% more CPU at the cost of 60 to 100%
more peak RSS; not adopted, user's decision, #424).

Cold semantic analysis, lookups and allocation (#442). A sampling profile of the 30 `vunit_vc30` cases
(frame pointers, 8700 samples) is flat: visibility lookups about 15% of the process, expression typing
(`complete_context`) 21%, declaration items 40%, parsing 17%. Changes: a region that has no use clauses and does
not declare the key is answered by its parent's cached lookup (74% of the uncached lookups were such), names
declared in the starting region are not cached, overload sets are shared `Arc`s, the lookup cache epoch is the
summed version of the three tracked arenas (no global atomic read-modify-write per mutable access), candidate sets
(`CandSet`), the temporaries of a lookup, `declare`, calls and operator candidate filtering use small vectors,
operator overloads are filtered by the type of their first parameter before the declaration is opened (a side
table, 37 overloads tested per operator on average, 8% of them fit), `complete_context` reuses emptied candidate
memos instead of two new hash tables per expression, the linkage check walks an expression once and only
looks further when a linkage port exists, body-to-spec matching and the newer-declaration passes of a lookup are
skipped when they cannot find anything. Heap allocations of the 30 cases: 56.4 M -> 29.5 M (-48%). Measured
on a host with load 17 to 30, so only repeated interleaved runs mean anything: `tools/bench/run.py --compare`
(`YAVS_BENCH_RUN_FLAGS=--no-cache`, 5 runs) analyze phase 5.04 -> 4.89 s CPU, peak RSS 78.98 -> 75.34 MiB;
the same 30 commands summed over 9 interleaved rounds (median / minimum): 6.06 -> 5.55 s / 5.66 -> 4.98 s of
analyze CPU (-8% / -12%). The per-file analysis corpora do not move beyond noise (osvvm-2008 7.40 -> 6.47 s,
uvvm-2008 10.46 -> 8.75 s in medians of 7 alternating runs, minima 6.09/6.20 and 8.57/8.56 s; peak RSS 36 MiB
before and after): 69% of those runs is parsing the dependencies, three quarters of that package bodies that
`yavs analyze` of one file never analyzes. Diagnostics of all analysis corpora (3851 calls) are byte-identical
before and after, the difftest corpus passes at `--threads 1` and `4` (378 of 378). Tried and dropped (no
measurable gain): presized arenas and lookup cache (page-touch cost of small runs: +6 MiB peak RSS), a per-list
operator index, one classification per name node and context, a flag-carrying pass over the use-clause results.

### Bit-plane kernels (#199)

`yavs_logic::wide` (`crates/yavs-logic/src/wide.rs`): word-parallel loops over runs of plane blocks used by the
kernel's signal buffers (resolution, copies, extraction, value comparison), `Logic` (slice, concat, shifts,
rotates, `is_x`, `?=`, resolve of N drivers) and the ieee natives (`resize`, shifts, relational operators,
`is_x`). Resolution folds all sources into class masks and resolves once; whole blocks are processed four at a
time as portable lane-wise code. No `unsafe`, no `std::arch`: an AVX2 build of the same loops measured no faster
(`docs/decisions.md` "Wide plane kernels"). Each kernel is checked against a per-element model for random
planes over all nine values, odd lengths and every offset (`crates/yavs-logic/tests/wide.rs`). Release build,
host load about 20, best CPU seconds of 3 (A = before, B = after; `cargo bench -p yavs-logic --bench wide` for the kernels):

| Measure | A | B |
|---|---|---|
| resolved bus design (32 drivers x 32 elements, 20000 cycles) | 0.61 | 0.41 |
| 1024-bit bus, 8 drivers, 20000 cycles | 0.67 | 0.32 |
| `shift_left`, `resize`, `rotate_right` of a 256-bit `unsigned`, 200000 calls | 0.27, 0.28, 0.34 | 0.07, 0.08, 0.07 |
| `is_x` of a 256-bit vector, 200000 calls (native since this change) | 2.06 | 0.02 |
| resolve 8 sources x 65536 elements (kernel) | 92.4 us | 30.8 us |
| copy 65536 elements, unaligned destination (kernel) | 6.8 us | 0.91 us |
| `resolved_bus` (tools/bench), total CPU | 0.20 | 0.17 |

The clocked-RTL designs (`lfsr`, `ff`), `ff_rtl` and the two external testbenches (`vai_window_gen`,
`vai_pe_array`) are interpreter-bound and unchanged (ratios 0.96 to 1.02). Regression guard: the differential
corpus passes 410/410 at 1 and 4 threads and 4 shuffled threads on the interpreter and, at 1 and 4 threads, with
`--tier=auto` and `--tier=llvm`; the external test runner's 11 suites pass (1344 tests); the `.pro` regression
passes 314/314 under 2008 and 2019; the verification library suite runs 102 pass, 9 fail, 26 skip (as before).

### Native ieee operators (#395, #404)

`std_logic_1164` gates, `not`, conversions, `is_x` and `rising_edge`, and `numeric_std` `+ - *`, relational
operators, `to_integer`, `to_unsigned`, `to_signed`, `resize` and the shift helpers, and `math_real` (`sqrt`, `exp`, `log*`, `sin`, `cos`, `**`, rounding functions), run natively
(`crates/yavs-interp/src/ieee.rs`); the `?=` family, `std_match`, the vector/integer forms of `/` `rem` `mod` and the `std_logic_unsigned`/`numeric_std_unsigned` wrappers still run from VHDL (round 2 below).
`crates/yavs/tests/ieee_native.rs` compares natives with the bodies (3 + 1 generated seeds, 2008 and
2019, errors, edges). Per operation on 16-bit vectors, process loop of 20000, median CPU of 5, release build,
host load about 16 (loop overhead about 2 us per iteration included):

| Operation | VHDL body (s) | native (s) |
|---|---|---|
| `unsigned +` | 1.82 | 0.05 |
| `unsigned xor` | 0.46 | 0.07 |
| `std_logic_vector xor` | 0.42 | 0.05 |
| `unsigned <` | 0.79 | 0.05 |
| `to_unsigned` | 0.24 | 0.05 |
| `resize` | 0.15 | 0.05 |
| `to_integer` | 0.45 | 0.04 |
| `shift_left` | 0.14 | 0.06 |

Synopsys `std_logic_arith` helpers (`plus`, `minus`, `mult`, `conv_*`; a metavalue hands the call back to the body):
`tests/corpus/synopsys/tb_arith_mul` 5.50 s to 1.12 s CPU, `tb_arith_addsub` 10.43 s to 7.80 s (its own helper
functions, not the arithmetic, are most of the rest); the 20 `synopsys` difftest cases pass at `--threads 1` and 4.

`tools/bench`: `ff_rtl` simulate 1.27 s to 0.05 s CPU (total 1.31 to 0.10), `compute_loop` 0.72 to 0.66.

#### Round 2 (#414)

Profile first (a per-function instruction count of each benchmark): after round 1 no `math_real` helper is
hot; the remaining `ieee` share is `math_real.uniform` (1 to 3%), `to_hstring` (0.6 to 6%) and the
`numeric_bit` conversions and shifts of the local design testbenches (8 to 13%). Natives added:
`numeric_bit` (all of the `numeric_std` natives, with the package's warning texts), `shift_left`,
`shift_right`, `rotate_left`, `rotate_right`, `/` `rem` `mod` of vectors (zero divisor handed back),
`to_hstring`/`to_ostring`, `math_real.uniform` (the first native procedure), the `std_logic_arith`
comparison helpers `is_less` ... and `make_binary` for every result type. Round 1's 32-bit INTEGER
assumptions were fixed first (#416). Release build, total CPU seconds, median of 3, shared host
(wall within 2%, noise about 5%):

| Benchmark | round 1 | round 2 |
|---|---|---|
| `vai_pe_array` | 0.30 | 0.24 |
| `vai_window_gen` | 2.26 | 2.13 |
| `uvvm_util_rand_tb` | 6.07 | 5.71 |
| `uvvm_axistream_vvc_tb` | 5.56 | 5.59 |
| `osvvm_axistream` | 2.08 | 2.11 |

Geometric mean of the ratio 0.94. The UVVM and OSVVM benches are dominated by their own string and
logging code, not by `ieee`. The differential tests (`ieee_native.rs`, 9 tests: random operands,
metavalues, null arrays, both directions, 64-bit INTEGER under 2019) pass natives on and off; with
`YAVS_IEEE_TIER=llvm YAVS_IEEE_SCALE=12` (shrunk testbenches) eight of them pass on the compiled tier; the
ninth (`native_matches_bodies_2008`) did not finish in 25 minutes of JIT compilation, which is the size
of its straight-line process, not the natives. The 353 difftest cases pass at `--threads 1` and 4.

### Elaboration of short library tests (#383, 2026-10-19)

Release builds, median CPU seconds (user+sys) of 3 runs on the busy shared host. The
elaboration-time session (calls of functions using package state) lowered every run-time package
again, initialised them and replayed the call log for each new function it called (13 times in the
stream testbench), and building the simulation did all of it once more. Now one program grows with
each new function and the simulation starts from a copy of the package state elaboration left.

| Testbench (one test case) | `yavs elab` CPU before / after the session change | elaborate before / after starting from it | init before / after |
|---|---|---|---|
| `tb_axi_stream` | 5.37 / 1.55 | 0.63 / 0.51 | 0.28 / 0.01 |
| `tb_wishbone_master` | 4.43 / 0.91 | 0.45 / 0.33 | 0.13 / 0.00 |
| `tb_avalon_master` | 3.87 / 1.01 | 0.43 / 0.35 | 0.13 / 0.00 |
| `tb_axi_stream_protocol_checker` | 2.21 / 0.97 | 0.42 / 0.35 | 0.15 / 0.00 |
| `tb_memory` | 0.38 / 0.38 | 0.08 / 0.07 | 0.10 / 0.05 |

The `yavs elab` column includes the analysis of the design closure (about 0.4 s then); the
elaborate and init columns are `--time-report` phases. Harness (`--compare`, 5 runs), `vunit_vc30`:
elaborate 9.98 to 8.42, init 3.76 to 0.25, total 26.27 to 21.84 CPU seconds. Every
external-runner `check` and verification component test passes; the difftest corpus output is
byte-identical at 1 and 4 threads. Left in elaborate: analysis of package bodies (now cached,
#407), the one initialisation of the run-time packages, and the interpreted elaboration-time calls
themselves (logger and checker factories). Whole suites at `-p 4` on the same host, wall seconds,
main before this work (7445284d) against main with it and the work that landed alongside (analysis
cache #407, array fills #405, native ieee operators #395, b9dcddb6): verification components 700 to
106, `check` 34 to 12; all tests pass with both.

### External test runner: where the wall time goes (2026-10-20)

The runner's own suites at `-p 1` (host load average about 25, so wall is noisy; CPU is the
comparison), median of 3 interleaved runs, before (one `yavs analyze` per file) against after (files
compiled in batches by the runner, and `analyze` keeping its manifest in memory and
syncing once per call): `string_ops` 3.2 s wall / 2.7 s CPU to 0.9 s / 0.9 s, `path` 3.7 / 2.7 to 1.1 / 0.9,
`run` 4.2 / 3.5 to 1.8 / 1.7, `logging` 19.5 / 14.8 to 16.3 / 13.0, `check` 28.1 / 24.7 to 24.7 / 22.4. Compile step
(62 to 91 files per suite): 1.4 to 2.7 s wall to 0.14 to 0.21 s. What is left is the runner's own Python (about
0.5 s of fixed start-up plus 10 to 25 ms per test, none of it in yavs) and one `yavs run` per test. A warm
`yavs run` of a short test (`check` suite, about 100 ms): analyze phase 17 ms (closure and key about 3 ms,
image restore about 14 ms), elaborate 23 ms, init 22 ms, simulate 38 ms; process start and exit under 3 ms. In the
verification-component suite: analyze 35 to 60 ms (closure 3.5 ms, image restore 30 ms), elaborate 35 to 140 ms.
The next levers are the image restore (#426) and caching elaboration (#407).

### Analysed-unit cache (#537, #539, #407)

`yavs analyze` stores every design unit it analyses from source in `yavs-cache/units/` of its library (`std`/`ieee`
in the work library's, with the bodies of the packages it loaded) and every later `analyze`, `elab` and `run` reads
the units it needs from there when source, build, options and dependencies are unchanged (`docs/decisions.md`
"Analysed-unit cache"). A replayed `Db` is the one analysis from source makes, id for id; whole pages of replayed
entries stay bytes until read. `YAVS_UNIT_CHECK=1 yavs analyze` compares the two analyses for every file and passes on
every analysis corpus.

Analysing a file again whose unit a later file replaced (the same entity name in two files) re-parses it and analyses all
its units like the first time (#577; the manifest marks such a file `partial`).

Analysing a library file by file (#581, #584): the head-to-head's "slower than sim1" cells (hdl-modules FIFO selection 1.45,
satcat5 1.37, Open Logic 1.16) were the per-call manifest fsync (#561, fixed in 60ebdc2e): at 65b7f573 the hdl-modules FIFO
selection took 5.5 s cold and 4.4 s warm, on the main before this work 2.3 s and 1.4 s (cores 8 and 9, same host load,
sim1 4.5 s). What was left per call was CPU: a warm file replayed its whole dependency closure (85 units, 2 MB for an Open
Logic testbench) to give back the records the manifest holds, the work library's manifest was parsed twice and rendered by
the `toml` serializer per call (satcat5's one 244 KB manifest made its warm call as dear as its cold one), and every
file repeated the warnings of the library units it read. Now a warm file whose units report nothing and whose manifest
record is the entry's is registered, not loaded (`Db::warm_records`), the manifest is parsed once and written directly,
and `analyze` shows warnings for the files named in the call only. `tools/realbench --compare --phases analyze` (cores 8,9,
5 runs, interleaved, 41 tests, loaded host): wall geometric mean cold 0.94, warm 0.53 (library part 0.94 and 0.54; own
files 0.97 and 0.71), instructions 0.91 and 0.38; against sim1 (lib + own, seconds, cold: sim1 / main / now, warm: sim1 /
main / now): hdl-modules FIFO 4.14 / 2.31 / 2.03 and 4.52 / 1.42 / 0.76, Open Logic 7.95 / 4.29 / 4.09 and 7.92 / 3.14 /
1.53, satcat5 6.72 / 3.82 / 3.34 and 7.22 / 3.47 / 1.89, OSVVM 11.70 / 6.90 / 6.73 and 12.31 / 4.38 / 2.36, UVVM 2.83 /
1.56 / 1.52 and 2.72 / 0.73 / 0.53. What is left per call is the size of the work library's manifest (read, parse, clone,
render, write per call), the table snapshots of `frame_end` (cold), and the dynamic loader relocating the 97 MB executable.

Real-world suite (`tools/realbench`, phases analyze, elab, run interp; P-core 2; main before the cache against main
with it, geometric mean of the wall ratio over 41 tests): analyze cold 0.68 (OSVVM compile order 16.1 s to 5.9 s,
UVVM 0.45 to 0.56), analyze warm 0.43, elab cold 0.71, elab warm 0.96, run unchanged (1.00). Against sim1 (P-core 4,
one step earlier): analyze cold 0.99 (was 1.23 in the baseline), analyze warm 0.60 (was 1.31), elab cold 1.03 (was
1.36), but the verification-library testbenches still elaborate cold at about 2 times sim1 (#549: their first run
writes prefix images).

### Elaboration-time call cache (#539)

The results of pure package functions called while elaborating (`docs/decisions.md` "Elaboration-time call cache") are
kept per closure (stage kind `ecall`) and per elaboration, with the reports they made replayed. `tools/realbench` phase
elab, P-cores 2 and 3 (CCD0), 5 runs interleaved, main against main with the cache, 41 tests, geometric mean of the
ratio (instructions / wall): elab warm 0.84 / 0.86, elab cold 0.96 / 0.96 (a repeat inside one elaboration hits too).
`poc_uart_rx` warm 1.78 G to 0.06 G instructions (0.16 s to 0.01 s), `poc_cache_cpu` warm 2.62 G to 0.13 G (0.24 s to
0.02 s) and cold 2.73 G to 0.50 G (0.26 s to 0.07 s); the other 39 tests are within noise.

### Package constants in elaboration-time calls (#539)

Functions called while elaborating that read a package constant with a computed initialiser (a configuration table, say) run
against the package state instead of rebuilding the constant at every call (`docs/decisions.md` "Package constants in
elaboration-time calls come from the package state"); pure ones are kept in the call cache. Cold `yavs elab` instructions
(`tools/realbench` worlds, bench builds): `poc_uart_rx` 1.787 G to 0.221 G, `olo_base_cam` 1.029 G to 0.999 G, the other six
tests of the set unchanged; warm `poc_uart_rx` 0.035 G both, `olo_base_cam` 0.410 G to 0.380 G.

### Elaboration-time calls run as lowered (#539)

Functions called while elaborating run as lowered and are optimised only when asked for 64 times, between two calls
(`docs/decisions.md` "Elaboration-time calls run as lowered; a function asked for often is optimised between two calls").
`yavs elab` user instructions, 40 `tools/realbench` tests, bench builds, main with the package-constant change before and
after: geometric mean cold 0.974, warm 0.918; `olo_base_cam` warm 0.380 G to 0.299 G, `hdlm_axi_fifo` warm 0.360 G to
0.264 G, `osvvm_axis_random` cold 0.598 G to 0.565 G; output unchanged on all 40.

A long loop in a function nothing asked for before (an elaboration-time table generator) no longer stays lowered (#695): a
function whose loops take 2048 jumps back is optimised, and a call that only computed (pure, no report, nothing tainted) restarts
with it. A million iterations cost 1435 M instructions of `yavs elab` before and 626 M after; 3000 iterations, just over the
budget, cost 12 percent more (the worst case), loops under it 1 to 2 percent more (the counting). The 42 `tools/realbench`
tests: geometric mean cold 1.0001, warm 1.0005, no restart, output identical (`docs/decisions.md` "Elaboration-time calls run
as lowered").

`Inst::visit` and `visit_mut` are generic over the closure instead of `dyn FnMut` (the optimiser, the predecoder and the verifier
call them for every operand): `yavs run --no-cache` to 0 fs, instructions over the 42 realbench tests, geometric mean 0.973
(`neorv32_*` 0.92 to 0.93, most others 0.97 to 0.98, `grlib_leon3mp` 1.003); the binary grows by 0.5 percent.

### Compilation cache (#407)

Cache keys name the build (a hash of all sources) and the optimiser and tier switches (#491, `docs/decisions.md`
"Build and switch fingerprint"): another build or setting is a miss.

`elab`/`run` keep the analyzed closure (and the package bodies elaboration loads) in
`<work dir>/yavs-cache/` (`docs/decisions.md` "Compilation cache"). `vunit_vc30`, median CPU seconds of
5 runs, main before the cache against main with it (warm; each of the 12 testbenches pays one cold run
in the first iteration only):

| | analyze | elaborate | init | simulate | total CPU |
|---|---|---|---|---|---|
| before | 7.82 | 6.36 | 0.15 | 2.20 | 17.30 |
| after (stage 1) | 1.12 | 2.03 | 0.10 | 2.28 | 6.17 |

The remaining analyze time is reading the image (about 40 ms per call). Elaboration is now the elaboration
walk of the design itself (about 0.2 s per call, design- and generics-specific; the lowering and
initialisation of the packages is about 30 ms), so caching lowered IR (stage 2) would gain little for
this benchmark. Analysis of `std`/`ieee` units a design uses is 10 to 20 ms, so a precomputed `std`/`ieee`
base was not built. Tests: `crates/yavs/tests/cache.rs`, `crates/yavs-cache`; the difftest corpus (342
tests at 1 and 4 threads) and the external runner's `check` suite pass with the cache on.

Stage 2, step 1 (#407): the lowered run-time packages are cached as well (`low-<key>.img` beside the
analysis image; `docs/decisions.md` "Compilation cache"). The IR (`yavs-ir`) and the lowering state
(`yavs-lower`) are serialisable; the in-memory forms are unchanged. Measured with the lowering of
the packages at 3 to 6 ms (4 ms, 238 functions and 48 000 instructions for `tb_axi_stream`; 1.8 ms to
verify and number it) and 2 ms to read and decode the 490 KB image. Release build, busy host (load
18 to 36), interleaved runs, `--time-report` elaborate phase:

| Run | elaborate, ms, min of 15 before / after | total, ms |
|---|---|---|
| `tb_axi_stream` (one test case) | 116 / 112 (104 / 96 in an earlier pair) | 225 / 219 |
| `tb_axi_lite_master` | 45 / 42 (40 / 36) | 116 / 123 (noise) |
| OSVVM AXI-stream testbench | 41 / 36 (39 / 33) | 95 / 90 |
| UVVM AXI-stream testbench (`--stop-time=1fs`) | 58 / 57 | 101 / 99 |

`vunit_vc30`, `--compare`, 5 runs, median CPU seconds (two sessions, A before / B after): elaborate
1.41 / 1.28 and 1.51 / 1.37, total 3.86 / 3.67 and 4.02 / 3.85 (4 to 5%, 4 ms per run). The
UVVM, OSVVM and `elab_heavy` totals are within the noise of the host (the same binary twice differs
by up to 8% there). Findings: a UVVM testbench lowers one 6.9 MB byte-string constant (a repeated
element written out); reading its 7.7 MB image cost 4 ms more than lowering, so an image is made
only for a constant pool up to 1 MiB (the constant is a repeated element: fixed by #446, see "Compact composite constants" below). Elaboration that instantiates
generics grows the `Db`, and then the entry is not used. Checks: output of the 30 test cases of
`vunit_vc30` byte-identical (md5 of stdout, stderr and exit status) with the cache off, cold and warm,
at `--threads 1` and `4`; the difftest corpus passes (362 of 362, 1 and 4 threads); waveform files of a
cold and a warm run are identical (`crates/yavs/tests/cache.rs`). Step 2 (the package state after initialisation) is below.

Stage 2, step 2 (#407): the package state after initialisation is cached (`init-<key>.img`, `docs/decisions.md`
"Compilation cache"): package data, the shared variables area and the shared heap are stored as they are and
restored instead of running the initialisers; an initialiser's reports are recorded and printed and counted again,
file operations, environment and clock reads and foreign calls keep the state from being stored (reason with
`YAVS_LOG=debug`). A restore of the 36 packages of a verification-component testbench takes 5 ms (3 MB image) against
17 to 25 ms of initialisation. Release build, `--compare` of main (4d0c9ae9) with this work, 7 runs, median CPU
seconds, pinned to one performance core: `vunit_vc30` elaborate 1.82 / 1.45, total 3.05 / 2.63; `osvvm_axistream`
total 0.69 / 0.68, `uvvm_axistream_vvc_tb` 0.47 / 0.44, `uvvm_util_rand_tb` and `elab_heavy` unchanged (their
packages initialise in a few ms). One process of a short test, min of 25 interleaved runs, wall ms: `tb_axi_stream`
163 / 151, `tb_axi_lite_master` 88 / 73, UVVM AXI-stream elaboration 92 / 89, OSVVM AXI-stream 573 / 555.
Checks: difftest corpus (410) at 4 threads (which also runs 1), 4 shuffled, with every `run` executed twice so the second is warm,
on the interpreter, `--tier=llvm` and `--tier=auto` and a `gc-stress` build (all pass; the one exception under the
twice-run wrapper is `textio/file_append_2019`, which appends to a file); the external runner's 11 suites (1344 tests) pass; the `.pro` regression passes 314/314 under 2008
and 2019; the verification library suite gives 102 pass, 9 fail, 26 skip, as main. Tests: `crates/yavs/tests/cache.rs`
(`init` entries: protected and heap state grown by elaboration-time calls, the simulation-start path and the time
resolution key, edit invalidates, reports replayed, a file-opening package not cached, damaged entry, parallel cold
runs), `state::tests` (block packing). The store keeps 96 entries now (three per design).

Compact composite constants (#446, `Fx::konst_comp`): a folded record or array-of-composites constant is
lowered as runs of repeated bytes (repeated elements, record fields with padding, one-valued logic arrays), and the
pool keeps `ConstData::Repeat`/`Runs` when that is four times smaller. The 6.9 MB constant of the UVVM
AXI-stream testbench is now about 12 KB (the pool of the lowered packages 7.6 MB to 12 KB), so its lowered packages are
cached (image 68 KB; before: not stored). `--compare`, 7 runs, median, same host: peak RSS 93.7 to 86.1 MiB, elaborate CPU
0.11 to 0.10 s, total CPU 3.16 to 3.18 s (noise); `vunit_vc30` (60 MiB, total 3.66 / 3.67 s) and `osvvm_axistream` (1.25 / 1.27 s)
unchanged. The interpreter expands a compact constant only when its address is used and writes `CopyConst` straight
to the copy; the optimiser reads pool bytes through `ConstData::read`. Tests: `composite/periodic_const` (difftest),
`periodic_constant_is_compact` (`yavs-lower`), `periodic_constant_does_not_block_the_lowering_cache`
(`crates/yavs/tests/cache.rs`, warm, cold and `--no-cache` identical), `ConstData::read` unit test.

Compiled code (#505, `docs/decisions.md` "Compiled code in the compilation cache"): the object file of every
module the LLVM tier compiles is kept in `<work dir>/yavs-cache/obj/`, keyed by the module's LLVM IR, the level,
the target and the build, and linked again by later runs (`yavs-llvm` `objcache.rs`). Measured with release builds
pinned to P-core 1, wall seconds, cold (compiled and stored) / warm (linked), output byte-identical in every row. With
the compile cost work of #432 (6ffcab58): `--tier=llvm` `uvvm_util_rand_tb` 140 / 2.3 (the program is one 17 MB
object), `osvvm_axistream` 59 / 1.5, `vai_window_gen` 43 / 1.1; before it, against the build without the cache (base):
`uvvm_util_rand_tb` base 296, cold 272, warm 3.5; `uvvm_sb_slv_tb` 231 / 203 / 1.9; `osvvm_axistream` 162 / 145 / 1.6;
`uvvm_axistream_vvc_tb` 161 / 179 / 1.9; `vai_window_gen` 85 / 89 / 1.4; `protected_scoreboard` 0.24 / 0.23 / 0.03;
`compute_h2h` 0.28 / 0.28 / 0.23. Storing costs nothing measurable (cold within the noise of base). `--tier=auto`
compiles few functions, so warm and cold differ little except for compute loops, which are linked as soon as they are
hot: `compute_h2h` 0.36 / 0.24, `uvvm_util_rand_tb` 1.26 / 1.12, `uvvm_sb_slv_tb` 1.28 / 1.26, `osvvm_axistream`,
`uvvm_axistream_vvc_tb`, `vai_window_gen`, `protected_scoreboard` unchanged (3 runs, median). Test cases of one
testbench share the `--tier=llvm` object once the design's `ir` entry has converged (its optimised copies change the
program the first runs see): `rand_tb` case 2 compiled, case 3 linked. The per-function modules of `--tier=auto` are
shared between testbenches whose library comes from one `lib` entry (the second random testbench of the utility
library linked the modules the first stored). Tests: `crates/yavs-llvm/tests/objcache.rs`
(cold, warm, damaged entry, the `--tier=auto` probe, unwritable directory), `crates/yavs/tests/cache.rs`
(`compiled_code_is_cached`, `parallel_runs_share_compiled_code`, `read_only_library_gets_no_compiled_code`),
`yavs-cache` (`objects_have_limits_of_their_own_and_are_cleaned_with_the_library`); the difftest corpus at 1 and 4
threads on the interpreter (422/422), `--tier=llvm` and `--tier=auto` cold (all pass) and warm (run twice in one work
directory: all pass but `textio/file_append_2019`, which appends to its own file); the second verification
library's `.pro` regression with `--tier=auto` cold and warm (314/314 each, `-j 2` into shared libraries).

### Per-run overhead (#426)

The fixed cost of one `yavs run`, which a testbench suite pays once per test case. Release builds, the main
of the day (before) against this work (after), same host at a load average of about 20, so CPU seconds and
medians. Decisions: `docs/decisions.md` "Per-run overhead".

| Measurement | before | after |
|---|---|---|
| `vunit_vc30`, analyze phase (30 runs, 5-run median CPU s) | 1.39 | 0.85 |
| `vunit_vc30`, total CPU s | 5.77 | 5.22 |
| one testbench of it (`tb_axi_stream`), CPU ms per run, 20-run loops | 404 | 389 |
| `elab_heavy` (20000 instances), total CPU s | 0.55 | 0.48 |
| design with `std_logic_1164`, `numeric_std`, `math_real`, `textio` only, warm run, ms | 6 to 8 | 6 to 8 |
| trivial testbench, process start to exit (200-run loop), ms | 3.0 | 3.0 |

Where one run of a testbench library of 130 000 expressions spends its analyze phase (ms, wall, warm):
manifests 6 (parse, twice) to 1 (once, small parser); closure hash of 60 source files 9 to under 1 (size and
modification time); entry read 14 to 5 (no copy, xxh3 instead of sha256); decode of the image 22 to 28
(unchanged: serde and postcard, 5.3 MB of arenas and 3.3 MB of source text, a third of it allocation); source
re-compare 1 to 7 to 0. A cold run (a miss) also encodes the image, 30 ms for the same library
(about 5 ms less with the dense symbol table).

Not done, with the numbers that decide it: decoding the arenas on several threads would take about 12 ms of
wall off the 22 but no CPU, so it does not help a suite that already fills the host; a different global allocator
(a general-purpose one with per-thread heaps) cut the decode by a quarter and a run by 10% but made `elab_heavy` 2.8
times slower and doubled the baseline RSS, so it is not adopted; lazily decoded arenas would save CPU but
change `Arena` in the analysis hot path.

Lazy image decode (follow-up): the image is no longer decoded as a whole. Arena entries decode 64 at a
time on first read, symbols are interned on first read, the source map keeps a digest instead of the text
(read again for a diagnostic; a source that changed or cannot be read costs the diagnostic its snippet only, with a note), the elaborator's whole-design scans come from facts stored in the image, and
processes are found by statement through an index (#435). Release builds, `yavs run` on a host at a load
average of about 30, so medians and CPU: a trivial testbench (`std_logic_1164`, `numeric_std`, `textio`), CPU
per run 7.7 to 4.8 ms; one `tb_axi_stream` test of the verification components, CPU per run 261 to 249 ms
(analyze phase wall 50 to 12 ms, elaborate 140 to 166 ms: what the run reads is decoded there);
`vunit_vc30` (30 runs, 5-run median), analyze phase wall 1.86 to 0.50 s, total CPU 4.09 to 3.76 s, total wall
5.87 to 5.45 s, peak RSS 68.5 to 60.4 MiB; `elab_heavy` unchanged. About 60% of the declaration and scope
entries and 30% of the expressions of a verification component testbench are read by a run: the run-time
package lowering reads all of its packages' declarations, which the lowered-IR cache (#407 stage 2) would
make unnecessary.

### Elaborate phase of library testbenches (#425)

Profile of `yavs run --time-report` elaborate with a sampling debugger (hardware counters are not
available on the shared host): for testbenches of the verification libraries the design walk is
small (about a fifth of the phase); the rest is the elaboration-time session. The first call that
needs package state lowers the run-time packages (36 for a verification component testbench) and
runs their initialisers (one interpreted pass that allocates and copies a few megabytes of pointer
storage), then each new function is added to the session program. Cuts: a logic array constant of
one value (an all-zero 2**19 bit default of a command record, 33 uses in one testbench) is built
in whole words instead of bit by bit; the session program moves between the runtime and the
lowering instead of being copied, numbered and verified again for every function added (about
2 ms each, 12 per testbench); the source files' line starts come from the source map, shared by
the runtimes; no scan of every package subprogram for external names when no loaded unit has one;
the run-time package list is found again only when the design database has grown.

Release builds, `--compare` of main (06dc880e) with this work, 5 runs, median CPU seconds
(`tools/bench/run.py`, own cache):

| Benchmark | elaborate before / after | total CPU before / after |
|---|---|---|
| `vunit_vc30` | 1.95 / 1.49 | 5.65 / 4.97 |
| `uvvm_axistream_vvc_tb` | 0.13 / 0.10 | 4.02 / 3.94 |
| `osvvm_axistream` | 0.06 / 0.05 | 1.92 / 1.96 (noise) |
| `uvvm_util_rand_tb` | 0.05 / 0.04 | 5.11 / 4.73 (simulate) |
| `elab_heavy` | 0.16 / 0.15 | 0.52 / 0.50 |

Elaborate wall time (minimum of 15 to 25 interleaved runs, ms), one run of the testbench: the
verification component testbenches `tb_axi_lite_master` 55 / 41, `tb_axi_stream` 185 / 149, the
`uvvm_axistream_vvc_tb` stream testbench (`--stop-time=1fs`) 124 / 87, `elab_heavy` 150 / 146. Checks: difftest corpus
at 1 and 4 threads, interpreter and LLVM tier, and with `--no-collapse-ports` (354 of 354); the
external runner's 11 suites (1357 tests) pass; the `.pro` regression (`tools/suites/pro.py`, 521 files) and 12 components of
`tools/suites/uvvm.py` (28 of 30 runs) give results identical to main's; the output of
`uvvm_axistream_vvc_tb` up to 10 us is byte-identical.

What is left, per call of a verification component testbench (about 40 ms of a 45 ms elaboration):
the initialisers of the run-time packages, 15 to 20 ms, are interpreted work (a `new` of an array
fills a temporary, copies it into the heap object, and the library's storage grows by 2**16 entries
at its first use: 8 to 10 MB of memory traffic, an interpreter and lowering matter); lowering the
36 packages, 7 to 10 ms; verifying that program, 3 ms. The lowered program is cached across runs
since #407 (stage 2, step 1, below); the package state after initialisation is not (the heap is
not serialisable yet).

### Waveform recording cost (#410)

`YAVS_BENCH_RUN_FLAGS="--wave={out}"` (and `--wave-variables`) runs any benchmark with waves; the report adds the
peak RSS and the FST size. Median total CPU seconds of 3 runs, `--threads 1`, same host load within a column
pair (the "before" column was measured while the host was busier, so compare the ratio to the no-wave run):

| Benchmark | no waves | `--wave` | `--wave-variables` before | `--wave-variables` after | FST (signals / with variables) |
|---|---|---|---|---|---|
| `ff_rtl` | 0.98 | 0.94 | 1.45 (x1.00) | 1.00 (x1.02) | 20 KB / 20 KB |
| `resolved_bus` | 0.42 | 0.44 | 0.77 (x1.31) | 0.46 (x1.10) | 151 KB / 227 KB |
| `delta_chain` | 0.22 | 0.23 | 0.35 (x1.06) | 0.23 (x1.05) | 23 KB / 23 KB |
| `vai_window_gen` | 3.47 | 3.51 | 6.58 (x1.19) | 3.54 (x1.02) | 132 KB / 271 KB |
| `uvvm_axistream_vvc_tb` | 6.31 | 6.52 | over 300 s | 23.7 (x3.8) | 27 KB / 430 MB |

Findings: recording signals costs 0 to 5 percent of the run (a stress design with 8000 toggling bits and
vectors: 3 to 5 percent at `--threads 1`, 2.5 percent at `--threads 4`; the kernel dominates by far), peak RSS
grows by 0.3 to 1 MiB on top of 13 to 82 MiB (the FST block buffer). `--wave-variables` was the cost: every variable of every process was read and decoded
at each time step. A process's variables are now read only after it ran (`InterpProcess::execs`), a value is
decoded only when its raw bytes changed, and logic vectors are decoded straight from the bit-planes. What is left
on the AXI-stream testbench is its 2.07 million variable entries (large records of arrays): 3 GB peak RSS (about
1.4 KB per entry: hierarchy, writer declarations, per-entry state), 2.8 s to declare them, and one raw compare of
every large variable per time step. The FST is read-back identical (hierarchy and every value change) before and
after, and identical at `--threads 1` and 4; its bytes can differ between runs of a run longer than the 5 s flush
interval, because a block ends on the wall clock.

### Waveform variable entries (#418)

`--wave --wave-variables` on the AXI-stream verification component testbench (2.07 million variable entries, the
largest a 7 MB record). Median total CPU seconds of 3 interleaved runs, `--threads 1`, host load about 20, binary
before this work against after (same work, `yavs run --stop-time=10us`); the no-wave run of that testbench is
5.9 s, so the ratio went from x5.8 under this load (x3.8 in #410) to x1.6, and x1.4 on a quieter host:

| Benchmark | before | after | ratio |
|---|---|---|---|
| `ff_rtl` | 0.07 | 0.07 | 0.99 |
| `resolved_bus` | 0.58 | 0.58 | 1.01 |
| `delta_chain` | 0.33 | 0.32 | 0.98 |
| `vai_window_gen` | 2.98 | 2.83 | 0.95 |
| `uvvm_axistream_vvc_tb` | 34.4 | 9.3 | 0.27 |

For the last one: init (declaring the entries) 3.5 to 1.1 s, simulate 29.8 to 7.5 s, peak RSS 2970 to 1497 MiB,
FST 443 to 80 MB. What changed, one commit each: variables compared in place with a heap write count
(`Heap::stamp`) so an unwritten composite is never touched (the shared 7 MB record was copied and compared at every
step); entries read from the raw bytes at places found once per variable (`yavs_elab::leaf_locs`) instead of a
decoded `Val` tree; only entries overlapping a changed 64-byte chunk are read; a flat hierarchy with shared types
and one declaration plan per type (from about 840 bytes of hierarchy and recorder state per entry to about 70); and
FST blocks that hold at least two values per signal. What is left is the writer library: about 460 bytes and
0.4 us per declared entry and 0.1 us per first value. Reading back with the reader library gives the same
hierarchy, types and every change before and after, at 1 and 4 threads (difftest corpus, 349 designs, FST
byte-identical); a run of up to 125 000 signals writes the same bytes as before, longer ones differ in block layout
only. No regression without waves (`vai_window_gen`, `compute_loop`, `protected_scoreboard`, `ff_rtl` within 4%).

### Waveform recording cost, round 2 (#410, 2026-10-23)

`--wave` was not free: +39 percent of a neorv32 run, a factor 3 on a design of 512-bit buses (the 0 to 5 percent
above was measured on designs with few changing signals). Where it went, from `perf` on neorv32: every changed range
cost a hook, a copy of the range list, a lock and a search; every value an allocation (`Value`), a `Vec`, a `String`
and two `Vec`s in the FST library; every port collapsed into its actual was a separate entry that was read,
converted and written again; and the recorder asked for six phase hooks in every delta cycle.

Cycles of `yavs run --tier=interp`, median of 5 (7 for the two short ones), core 11 of the 7950X3D, a loaded host,
before (`9be61682`) and after (`b5ab7b91`) interleaved in one batch; the overhead is `--wave` over no waves of the
same binary, so other agents' commits in the binaries do not count. The reference column is the reference
simulator's own overhead (`--wave` with `--dump-arrays`, elaboration plus run), the FST column MB before, after,
reference:

| Testbench | before | after | with `--wave-variables` before, after | reference | FST MB |
|---|---|---|---|---|---|
| `neorv32_single_min` | +39% | +4% | +47%, +9% | +27% | 19.3, 4.3, 0.19 |
| `neorv32_dual_full` | +29% | +3% | +33%, +9% | +10% | 28.2, 5.2, 0.22 |
| `sc5_fifo_repack` | +36% | +14% | +52%, +25% | +96% | 11.7, 10.2, 6.4 |
| `fletcher_bufferwriter_512` | +212% | +35% | | +1009% | 681, 238, 136 |
| `osvvm_axis_random` | +8% | +4% | +24%, +17% | +4% | 0.09, 0.08, 0.02 |
| `hdlm_axi_fifo` | +10% | +4% | +26%, +21% | -2% | 0.02, 0.01, 0.00 |

The head-to-head designs (`tools/bench/designs`; the reference with `--dump-arrays` writes the same names and the same
number of changes on the seven checked, `rtl_lfsr`, `rtl_chain`, `rtl_ram`, `lfsr64`, `delta_chain`, `rtl_bus`, `lfsr64_xor`), before, after, reference: `rtl_ff` +3, +3, +4; `rtl_lfsr` +194, +54, +93; `rtl_chain` +72, +18, +6;
`rtl_fan` +1, 0, 0; `rtl_bus` +11, +4, +1; `rtl_ram` +68, +39, +6741 (the reference dumps the 16384 elements
very slowly); `lfsr64_xor` +133, +47, +55; `lfsr64` +233, +74, +185; `ff_rtl` +34, +13, +4; `resolved_bus` +17, +8,
+4; `delta_chain` +76, +19, +4. Total wall of `--tier=auto` against the reference's elaboration plus run (yavs / reference),
without and with waves, warm cache: neorv32 2.67 and 2.13, `sc5_fifo_repack` 4.26 and 2.52, `osvvm_axis_random` 0.36 and 0.37;
cold cache (`--no-cache`): 2.77 and 2.28, 4.48 and 2.65, 0.59 and 0.58. The ratio with waves improves because the
reference's overhead is larger than ours.

What changed (`e1efd95d`, `f1f3900c`, `b5ab7b91`, `03330f88`): names that are the same value (same kernel
elements, conversion, type and class: a port collapsed into its actual at any depth) are FST aliases of one
chain, read, converted and written once (`--wave` only; the live stream of `--dap` keeps one id per name);
`Observer::value_changes` gives an observer a whole cycle in one call and `Hook::view` reads a range in place (no
`Value`, no copy); logic vectors become positions from the plane words eight elements at a time and are mapped to
characters eight at a time when binary; scalar 0/1 use the library's binary fast path and the library's SIMD
packing is on; a one-entry leaf is marked without a search and an array of equal elements by division; the recorder
asks for four phases only (the same fault was in the toggle coverage observer, #568: `--cov` on a long chain -15
percent); value chains are LZ4-compressed (files -50 to -60 percent on neorv32 and the bus testbench; the hierarchy
block was LZ4 already); a process's variables are skipped after one idle check (`03330f88`, not in the timings). Writer alone (`examples/writer_bench`,
3000 scalars and 2000 32-bit vectors): 96 to 59 ns per change. Read back with the reader library: every name and every
change equal to before on 445 designs of `tests/corpus` (with `--wave-variables`; 100 byte-identical, 345 with
aliases) and on the six benchmark testbenches, and the same file at `--threads 1` and `4`; files of runs under the
5 s flush interval that have no aliases are byte-identical to before.

Left (follow-up issue): the library's per-change work (two allocations per vector change; a prototype that takes the
ASCII bytes cut the writer from 121 to 77 ns per 32-bit change but needs a vendored copy), an off-thread writer (wall
time), the declaration of millions of variable entries (`uvvm_axistream_vvc_tb` with `--wave-variables`: 1.06 s,
1.5 GB), and the kernel's cost of watching every signal (`rtl_chain` +18 percent against +6).

### Simulation kernel (#385, #411)

Kernel and kernel/interpreter boundary costs for signal-heavy designs, one commit each, output byte-identical
(difftest corpus at 1 and 4 threads and shuffled, the external runner's `check` and `verification_components`
suites 257/257 and 579/579, the AXI-stream verification component testbench log identical before and after).
CPU seconds (user + sys), median of 5 interleaved runs unless noted, release builds, host shared with other
agents (load about 15). Each row compares the binary before and after that change only. Designs (scratch, not
in the repository): `ff` 4096 flip-flops with one process each (100 cycles), `lfsr` 512 16-bit LFSRs (300
cycles), `chain` a 1000-deep chain of concurrent assignments over one vector (1000 clock edges, 1000 deltas
each), `fan` 4000 processes on one clock, `bus` a 32-bit resolved bus with 32 drivers, plus the two long local
testbenches and the harness benchmarks.

| Change | Benchmark | Before | After |
|---|---|---|---|
| Signal reads copied straight into interpreter memory (`Ctx::read_into`) | `ff` / `lfsr` / `chain` | 16.16 / 13.01 / 8.90 | 8.05 / 3.95 / 7.12 |
| A dirty range finds its segments by binary search | `ff` / `chain` | 8.05 / 7.12 | 6.96 / 5.77 |
| Waiters bucketed by 64-element chunk | `chain` / `ff` | 5.77 / 6.96 | 4.26 / 6.65 |
| The three above, harness | `delta_chain` / `resolved_bus` | 0.53 / 0.60 | 0.31 / 0.56 |
| Zero-delay entries in a list, transaction buffers recycled | kernel example bench (`examples/bench.rs`, 10000 processes, all three parts) | 18.17 | 16.33 |
| Small batches run inline (`--threads 4`) | `chain`, 500 edges, 3 runs | 11.88 | 1.16 |
| Sources and segments of wide signals built in O(n log n) (#411) | `elab_heavy` init / total | 0.60 / 1.23 | 0.26 / 0.87 |
| Edge triggers (clocked processes not resumed on the other edge) | `ff` | 1.38 | 1.28 |
| One element of an array signal read, not the whole signal | `ff` | 1.32 | 0.81 |
| #420: edge triggers for `and`ed conditions (`fan`), no per-instruction allocation for one-range signal accesses, `rising_edge` without copying values, no shared reference count bump per activation | `fan` / `ff` / `chain` / `lfsr` (min of 5, CPU s) | 1.10 / 0.54 / 1.04 / 0.66 | 0.50 / 0.48 / 0.96 / 0.66 |

`bus`, `fan`, the two local testbenches (`vai_window_gen`, `vai_bias_requant`) and the harness's `ff_rtl` stay
within the noise for every change: their time is in process code, not in the kernel. Sampled profiles (a
debugger-based sampler; no hardware profiler on this host) put the kernel at 1 to 10 percent of those runs.

Against a reference simulator (elaborate and run, its analysis excluded), main at the last of these commits:

| Benchmark | yavs | reference |
|---|---|---|
| `ff` | 0.81 | 0.25 |
| `lfsr` | 1.12 | 0.06 |
| `bus` | 1.21 | 0.70 |
| `chain` | 1.43 | 0.10 |
| `fan` | 1.60 | 0.26 |
| `vai_window_gen` | 2.36 | 1.12 |
| `vai_bias_requant` | 0.33 | 0.47 |

The AXI-stream verification component testbench (first generic set, full run, 1 run each) took 577 s CPU on main
before this work (`7110f09f`) and 173 s after it (most of the gain is the interpreter work of #386 and the native
operators of #395 that landed in the same period); it now runs to the end (its alert summary failed until #391, #419). Static sensitivity (R2c) was prototyped in the kernel (waiters that stay
registered while a process suspends on the same list) and measured: no change on the benchmarks above or on the
kernel-only bench, so it was not kept. Sampled profiles after #420: 55 to 70 percent of `ff`/`lfsr`/`chain` is
interpreter dispatch (instruction fetch and `step`), 15 to 25 percent kernel (wake scan, driving values, `Ctx`
accessors), the rest allocation in the interpreter (`temp_alloc`, the part tables now removed).

Round 3 (#420, `docs/decisions.md` "Skipped unchanged assignments", "Remembered results of user resolution
functions", "Edge triggers for clocked processes"), CPU seconds, min of 3 to 5 interleaved runs, main before (`df071059`)
and after, difftest corpus (357 tests) identical at 1 and 4 threads, shuffled, both tiers, raw output byte for byte:

| Change | Benchmark | Before | After |
|---|---|---|---|
| R2b (unchanged zero-delay assignments queue nothing, their delta stays) and edge triggers for `if rst = '1' ... elsif rising_edge(clk)` | 2000 async-reset flip-flops, 500 cycles | 1.26 | 0.92 |
| R3: remembered results of a user resolution function | 32-bit bus, 8 drivers, user enumeration | 0.98 | 0.10 |
| all of the above, harness | `vai_window_gen` / `ff_rtl` / `resolved_bus` / `delta_chain` | 2.39 / 0.057 / 0.38 / 0.22 | 2.17 / 0.052 / 0.39 / 0.22 |

`ff`, `fan`, `chain` and `bus` (synthetic kernel designs) are unchanged within the noise (their assignments change
values and their clocked processes have no reset). Process slots reached without a lock outside the parallel batch (`slot_mut`): `fan` (4000 processes on one clock, default generics) 5.48 s to 4.70 s CPU. Measured and not done: a level condition as trigger
(`if rising_edge(clk) then if en = '1' then`): an activation that only tests a false enable costs 0.4 us, so even
a permanently low enable would save 0.4 s per million activations, less than the trigger's bookkeeping would
cost where the enable is often high; waiters grouped per trigger (a clock with 2000 ignoring processes costs 15 ns
per process and ignored edge, 0.01 s of 0.4 s in the same design). The activation of a process (instruction
dispatch and its assignments) stays the cost, 1 to 1.5 us for a flip-flop with an assignment.

### Interpreter dispatch, round 2 (#412)

One commit per change; each table is an interleaved A/B of the binary before and after that commit
(`tools/bench --compare`, median total CPU seconds of 5 runs, release builds, shared host).

1. Registers defined once by `Const`, `ConstF` or `AddrOf` (every constant and every variable address
   lowering emits at each use) are set when the frame is created and skipped by the dispatch loop
   (`crates/yavs-interp/src/pre.rs`); the instructions stay in the code, so compiled code, pc numbers and
   the debug loop are unchanged. Roughly 40% of the executed instructions of `compute_loop` are gone.

| Benchmark | before | after | ratio |
|---|---|---|---|
| `compute_loop` | 0.56 | 0.39 | 0.69 |
| `ff_rtl` | 0.99 | 0.87 | 0.88 |
| `resolved_bus` | 0.43 | 0.40 | 0.92 |
| `delta_chain` | 0.23 | 0.20 | 0.88 |
| `textio_heavy` | 0.42 | 0.38 | 0.90 |
| `protected_scoreboard` | 0.34 | 0.28 | 0.85 |
| `uvvm_util_rand_tb` | 17.74 | 15.46 | 0.87 |

2. The common instructions (loads, stores, index and position arithmetic, checked arithmetic, compares,
   checks, branches, moves) run in the dispatch loop itself instead of through the general `step` and its
   `Result<Flow>`; checks look their message up only when they fail; arithmetic that cannot fail is inlined
   (32-bit division for natural operands); loads and stores of 1, 4 and 8 bytes are fixed-size copies
   instead of `memcpy` calls; descriptors are read in one access.

| Benchmark | before | after | ratio |
|---|---|---|---|
| `compute_loop` | 0.39 | 0.36 | 0.92 |
| `ff_rtl` | 0.99 | 0.68 | 0.69 |
| `resolved_bus` | 0.38 | 0.36 | 0.93 |
| `delta_chain` | 0.20 | 0.17 | 0.86 |
| `textio_heavy` | 0.38 | 0.35 | 0.93 |
| `protected_scoreboard` | 0.30 | 0.28 | 0.93 |
| `uvvm_util_rand_tb` | 16.32 | 12.37 | 0.76 |

3. Lowering asks for the size or the element count of an array alone: the count of a one-dimensional array is
   one `DopeLen` instead of a layout and four more instructions, the size is the layout and one load; a zero
   offset emits nothing. Element access in loops over vectors (`numeric_std` bodies) is shorter.

| Benchmark | before | after | ratio |
|---|---|---|---|
| `compute_loop` | 0.35 | 0.35 | 1.01 |
| `ff_rtl` | 0.73 | 0.70 | 0.96 |
| `resolved_bus` | 0.33 | 0.31 | 0.95 |
| `delta_chain` | 0.19 | 0.17 | 0.90 |
| `textio_heavy` | 0.34 | 0.35 | 1.02 |
| `protected_scoreboard` | 0.27 | 0.23 | 0.84 |
| `uvvm_util_rand_tb` | 12.39 | 11.70 | 0.94 |

4. Unconditional jumps cost nothing (the skip table leads to the jump's target), a compare is executed
   together with the branch on its result, and a load, the offset from what it loaded and the load through
   that (a field of a descriptor or of a record behind a pointer) are one dispatch.

| Benchmark | before | after | ratio |
|---|---|---|---|
| `compute_loop` | 0.39 | 0.34 | 0.86 |
| `ff_rtl` | 0.92 | 0.79 | 0.85 |
| `resolved_bus` | 0.34 | 0.33 | 0.98 |
| `delta_chain` | 0.20 | 0.20 | 0.99 |
| `textio_heavy` | 0.38 | 0.39 | 1.03 |
| `protected_scoreboard` | 0.30 | 0.28 | 0.92 |
| `uvvm_util_rand_tb` | 12.25 | 11.70 | 0.95 |

### Interpreter dispatch, round 3 (#431)

1. The hidden zero-register dependency (#430): not a read before a write in the lowered IR. The sparse
   register fill of round 2 skipped preloaded registers whose value is 0, so a skipped `Const 0` kept the
   previous frame's value. The IR verifier now checks by dataflow (debug builds and tests) that every other
   register is written before it is read, in a process also after a `Suspend`; the corpus passes it. The
   interpreter still zeroes the other registers: starting them stale was 5-12% faster on call-heavy designs
   but kept garbage alive in the conservative register scan of the collection (`uvvm_util_rand_tb` +25% run
   time, +31% peak memory), so stale registers wait for precise roots (#422).
2. Predecoded code (`yavs-interp` `pre.rs`, `Code`): each function is converted once, on its first call, into
   24-byte ops that the fast loop dispatches on (no debugger, no compiled tier). Preloaded definitions and
   unconditional jumps are gone from it, jump targets are op indices, loads and stores through a variable's
   preloaded frame or instance address read the slot directly, an `Index` by a constant folds into the access,
   a compare fuses with its branch, a `LayoutOf` with the one `Field` or load that reads it, and `SigVal`,
   `DopeLen`, `Alloc` and the jump back to a process's body are ops of the loop; the rest steps the IR
   instruction out of line. Frames keep IR pcs (`Code::at`, `Code::entry`), so messages, traces, root maps,
   coverage, DAP stepping and the LLVM tier are unchanged; the debug loop runs the IR unfused.
3. `Copy` and `Fill` stage their bytes in a reused buffer instead of a new vector per execution.
4. `Call`, `Ret`, `CallNative`, `SigReadScalar`, `WaitOn` and `Suspend` are ops of the loop too (their bodies
   shared with `step`): the per-activation instructions of small clocked processes no longer go through the
   generic step.

Release builds, median CPU seconds of 5 interleaved runs (3 for the synthetic designs) at `--threads 1`, host
load about 24 on 24 cores; main at `5b524584` (with the IR optimiser of #427) against main with steps 2 to 4.
Synthetic designs (scratch, see "Simulation kernel"; `calls`: 3 million calls of a one-line function). Measured
per step on the way (same host, earlier main): step 2 alone gave 0.68 to 0.87 on the interpreter-bound
benchmarks; step 3 is within the noise except `osvvm_axistream` (0.82 to 0.77); step 4 gave `calls` 0.89 and
`fan` 0.88.

| Benchmark | main | steps 2-4 | ratio |
|---|---|---|---|
| `compute_loop` | 0.32 | 0.18 | 0.57 |
| `ff_rtl` | 0.039 | 0.034 | 0.87 |
| `delta_chain` | 0.17 | 0.15 | 0.86 |
| `protected_scoreboard` | 0.16 | 0.10 | 0.65 |
| `strings_heavy` | 0.35 | 0.26 | 0.74 |
| `access_heavy` | 0.17 | 0.12 | 0.71 |
| `uvvm_util_rand_tb` | 3.74 | 2.72 | 0.73 |
| `osvvm_axistream` | 1.23 | 1.01 | 0.83 |
| `vai_window_gen` | 1.23 | 0.88 | 0.72 |
| `ff` | 7.41 | 6.02 | 0.81 |
| `lfsr` | 11.39 | 9.29 | 0.82 |
| `chain` | 2.09 | 1.87 | 0.90 |
| `fan` | 6.30 | 5.70 | 0.90 |
| `calls` | 0.41 | 0.29 | 0.71 |

What is left in these profiles: `lfsr` reads its whole array signal for one element (#436, lowering); a call costs about 33 ns after #438 ("Interpreter calls"); `fan` and `ff` are dominated by the kernel and by per-activation work (scratch vectors, the
register fill, native `rising_edge`). The difftest corpus (359) passes at 1 and 4 threads, shuffled, and on
the LLVM tier; the external runner's `check` and `verification_components` suites pass, and the two external verification libraries' suites (one: 78 pass,
33 fail, 26 skipped; the other's scripts: 314/314 under 2008 and 2019) give the same verdict per test as main.
A build with the `llvm` feature runs `--tier=auto` by default, which keeps the IR loop (the tier-up counts live
there), so it does not use the predecoded code yet.

### Interpreter calls (#438)

Frames are slices of two stacks (all registers; all slots, frame 0's process variables apart) with base
offsets, not vectors of their own: a call allocates nothing and moves no vector. `Inst::Call` of a function
copies the arguments from the caller's registers into the callee's slots, builds the frame in place (no
`Result<Frame>` copy), and the fast loop enters the callee and returns to the caller without leaving
`run_ops`. `Pre::fill` patches frame addresses without the checked `mk_addr`. The `TierFrame` carries the
stacks with offsets; frames strictly between the root and the top are stepped by the interpreter in compiled
code. The root scans read the stacks.

Release builds, min CPU seconds of 7 alternating runs at `--threads 1` (`calls` with N = 4000000, 16 million
calls), host shared: before 1.23 s (77 ns per call, loop included), after 0.53 s (33 ns).

| Benchmark | before | after | ratio |
|---|---|---|---|
| `calls` (harness, 3 million calls) | 0.23 | 0.12 | 0.53 |
| `protected_scoreboard` | 0.10 | 0.09 | 0.90 |
| `access_heavy` | 0.11 | 0.11 | 1.05 |
| `strings_heavy` | 0.21 | 0.21 | 1.02 |
| `uvvm_util_rand_tb` | 2.24 | 2.15 | 0.96 |
| `osvvm_axistream` | 0.78 | 0.80 | 1.02 |

The difftest corpus (360 with the new `process/call_frames`) passes at 1 and 4 threads and on the LLVM tier
(`--tier=llvm`, `--tier=auto`); the external runner's `check` (257) and `verification_components` suites pass;
the verification-library suite gives 78 pass, 33 fail, 26 skipped as before.

### Interpreter, clocked processes (#443)

Instruction counts of one process activation (deterministic, stepping the process closure): a process that only waits 769 to 646, `fan` 1194 to 1095. Round 5 changes: the scalar `std_logic` natives, a signal element read, the edge test, a wait on one element and `Drive` are ops of the loop; a process of the same function as the last one on the thread skips the register refill; the activation outcome travels as a one-byte value (a copied result struct stalled on store forwarding). CPU seconds, min of 3 at similar load, before and after, and against the reference simulator (same host, load about 5):

| Design | before | now | now / before | reference | now / reference |
|---|---|---|---|---|---|
| ff | 3.49 | 3.00 | 0.86 | 0.52 | 5.8 |
| lfsr | 2.10 | 2.06 | 0.98 | 0.16 | 12.9 |
| chain | 1.01 | 1.00 | 0.99 | 0.14 | 7.1 |
| fan | 3.24 | 2.54 | 0.78 | 0.99 | 2.6 |
| bus | 0.81 | 0.65 | 0.80 | 0.68 | 1.0 |

What the instruction profile shows next (ff, about 5000 instructions per activation): the kernel assignment behind `Drive` is 1250, op fetch and register indexing about 20 per op, a vector element read about 500 (index arithmetic), the process exit path 330 and entry 250. These belong to the kernel work (scalar assignment, element read and edge entry points) and to fusing the index arithmetic of a signal element read.

### Interpreter, process entry and exit; direct side exits (#443, #432, #446)

Fixed cost per activation: the scratch swap, `run_quick`, `after_suspend` and the root frame's `suspend` took about 330 instructions of 1460 per activation of `fan`. A small process now keeps its vectors (`Machine::resident`), `run` goes straight to `run_quick` (`hot`), and the checks are inline with the cold parts out of line. `copy_const` no longer clones the runtime handle. Instructions and core cycles of a whole run (`perf stat`, `--threads 1`, pinned to performance core 2, both binaries built from main at 56744da3 and from main plus this change, `llvm` feature):

| Design | instructions before (G) | now | cycles before (G) | now |
|---|---|---|---|---|
| fan | 25.13 | 20.11 | 6.00 | 5.47 (5.33 in a second run) |
| ff | 30.95 | 30.25 | 6.04 to 6.12 | 6.08 to 6.28 (resident off: the same, so noise and code layout) |
| chain | 7.98 | 7.47 | 1.48 | 1.43 |
| lfsr | 16.96 | 16.72 | 3.37 | 3.36 |

`fan`: 250 instructions fewer per activation. The remaining entry and exit is about 90 (`run_ops` set-up with its spilled locals, the outcome travelling through memory). Shared benchmarks, `--compare` pinned, 3 runs, total CPU after / before: `--tier=interp` uvvm_util_rand_tb 0.99, osvvm_axistream 1.01, protected_scoreboard 1.02, calls 1.02, vunit_vc30 1.00; `--tier=auto` 1.00, 1.04, 0.99, 1.02, 0.96 (testbench code: activation cost is not what these run on).

Testbench side exits of the LLVM tier (`uvvm_util_rand_tb` under `--tier=llvm`, counted by instruction kind: fat `TempRoot` 4.2M, `NewFrame` 2.8M, `CallNative` 3.2M, `Call` 2.5M, `Deref` 1.1M, `Free` 0.8M, `MemEq` 0.5M, `Fill` 0.35M, `CallProt` 0.2M). `TempRoot`, `Stmt` with roots, `NewFrame`, `Deref`, `MemEq` and `Cat` run through `TierHost::signal` (no generic step, no loss of the cached objects), a fat `TempRoot` whose words point into no heap is a helper call and no step at all, `CallNative` is called directly. Instructions run after the last LLVM compile sample of the run (perf, one sample per million instructions): 19.8 G before, 18.2 G after (-8%). Left as generic steps: `Call` (a compiled call costs about what an interpreted one does, dominated by the frame push and the view refresh), `Free`, `Fill`, `CallProt`. Inline code for the fat-pair test was measured and dropped: +1.6 percent compile instructions for no run gain.

### Interpreter, index arithmetic and vector ops (#443)

Round 6 of the clocked-process work, driven by exact instruction counts of one process activation (stepping `InterpProcess::run` under a debugger, attributing every instruction to the op of the loop that ran it, callees included; scratch tooling, not in the repository). Changes, all in `pre.rs` and `exec.rs`:

- A run in one block of add, subtract and multiply-by-constant `Bin`s, `Pos`es and `DopeLen`s of the first dimension is a few linear ops (`Aff1`, `Aff2`, `AffP`, `AffL`, `AffPL`, `AffPR`): `c0 + sum coef * atom`, one op per result something else reads. Exact: the fast path takes atoms within 2^30, where no intermediate of the replaced instructions can overflow; an index outside its range, a long range or a big value runs the replaced IR instructions as they are (`aff_replay`), so every error has the same text and pc as before. A vector element read was `LoadInst`, `Pos`, `DopeLen`, four `Bin`s and `SigRead1` (nine ops); it is `LoadInst` and `ReadPL`/`ReadPR` now.
- The `peephole` pass makes `WaveBuf` (`Alloc` + `Store` + `Index` by 8), `ReadPL`/`ReadPR` and `LSetDrive` (the `LSet` of element 0 of a waveform `WaveBuf` made and the one-element `Drive` of it: a scalar logic assignment goes to the kernel without building the waveform's planes). `Bool`, `LCopy` and the vector `SigRead` are ops (they were `Step`s), a `Cmp` + `Not` + `CondBr` is one `CmpBr`, and a `BinRange` with 32-bit constant bounds reads no bound registers (`BinRangeK`).
- Memory paths: `lcopy` runs on stack buffers (and directly in the temporary area), a single-element `Drive` builds no vectors, `dim_of` reads a descriptor in one slice (constant descriptors included), a layout word of a constant descriptor needs no descriptor read, `make_dope` writes in place, the register scratch is a const-initialised thread local.

Instructions per activation (steady state, deterministic) and core cycles of a whole run (`perf stat`, `--threads 1`, pinned to one performance core, A and B on the same core) before and after, and against the reference simulator:

| Design | instructions before | now | cycles before (G) | now (G) | now / before | reference (G) | now / reference (before) |
|---|---|---|---|---|---|---|---|
| lfsr | 10719 | 7387 | 7.23 | 5.64 | 0.78 | 0.56 | 10.1 (13.0) |
| chain | 3382 | 2739 | 3.56 | 3.16 | 0.89 | 0.54 | 5.9 (6.6) |
| ff | 4907 | 4078 | 10.57 | 9.23 | 0.87 | 2.01 | 4.6 (5.3) |
| fan | 1075 | 1090 | 8.61 | 8.89 | 1.03 | 3.81 | 2.3 (2.3) |
| bus | | | 1.53 | 1.45 | 0.95 | 2.48 | 0.6 (0.6) |

Where the remaining instructions are (lfsr, 7400 per activation): the op fetch and dispatch 19%, the kernel assignment behind `Drive` 19% (ff and chain: 28% and 41%), single-element signal reads 160 each (the kernel read is inlined into the op), the vector concatenation sequence the lowering emits for `a(14 downto 0) & b` (about 30 ops: two descriptors, a length test, three `LayoutOf` + `Alloc` pairs, `LCopy`, `LSet`, `SubtypeCheck`, `Copy`), and a fixed 330 per activation for the scratch swap, `run_quick`, `exec_fast` and `after_suspend` (30% of `fan`). The kernel side of the same profile (assignment layers `schedule`/`own`/`same_as`/`unchanged`, the re-registration of the static sensitivity after every activation, the edge test of a process the kernel resumed because of that very edge) is commented on #443.

### Interpreter, concatenation and descriptor folds (#443)

Round 7 of the clocked-process work. The lowering of `a(14 downto 0) & b` was about 30 instructions and, as the profile of `lfsr` showed, most of the rest of its interpreter cost; the measured per-activation ops of that process were 93. Changes:

- `Inst::Cat` (`yavs-ir`, `docs/decisions.md` "Concatenation instruction and descriptor-length folds"): the pieces of a concatenation, or of a positional aggregate of arrays and elements, stored in order into the result block in one instruction; the interpreter combines the plane words of a result of up to 64 elements in registers. Bound checks, the null-operand rule and the `SubtypeCheck` of the result stay separate instructions, so every message and pc is as before. `LCopy` and `Copy` remain the slice copy; a slice assignment needed no instruction of its own.
- The optimiser knows descriptor lengths: a `MakeDope` of constants, a pool descriptor or a layout of a one-dimensional array type under such a descriptor fold `DopeLen`, the null test of a slice or concatenation, an equal-length `SubtypeCheck` and the size loads of `LayoutOf`; a block only `MakeDope` writes is dropped. The folds need facts to survive writes to other blocks (`Cls::Block`), a join after a folded branch (up to four more walks) and, in processes, blocks reached from the entry (and the arms of a folded diamond emptied like those of a function). Positions and checks repeated with another message are one expression.
- Register sharing kept the temporaries of index arithmetic apart no more, so the predecoder's linear runs (`ReadPL`, `ReadPR`, `Aff*`, round 6) fire in functions over 31 registers again: the process of `lfsr` had nine ops per element read in the previous build.
- `Inst::Select { dst, c, a, b }` (docs/decisions.md "Select instruction") replaces the diamond of a `CondBr` and two `Move`s that the lowering's `select` helper (28 sites) built for every conditional value: bounds of a range, the check value and the null-operands result of `&`, `minimum`/`maximum`, loop steps. A function returning `"item " & integer'image(i) & ": " & s` is 54 ops instead of 78; the random-number testbench of the utility library (one case, pinned) takes 5.77 G cycles before and 5.60 G after, 23.13 G and 22.86 G instructions.
- Instance-constant values of a process (`yavs-ir` `opt/memo.rs`, docs/decisions.md "Instance-constant values of a process are computed once per instance"): the index arithmetic, range checks and positions that depend only on generics, generate indices and constants run once per instance (a flag and frame slots in the process frame); the original code runs until the first complete pass, so failing checks keep message and time. Pinned (core 5) cycles on main 56744da3, memo off to on: ff 6.34 G to 5.58 G (instructions 30.8 G to 26.3 G); chain, fan, lfsr within 2 percent (their processes hold one or two instance-constant instructions).

Pinned `perf stat` (one performance core, `--threads 1`, A and B on the same core, median of 3), main at `fbb4f911` against this change, and the reference simulator (same core, `--jit`):

| Design | cycles before (G) | now (G) | instructions before (G) | now (G) | now / before | reference (G) | now / reference (before) |
|---|---|---|---|---|---|---|---|
| lfsr | 6.27 | 4.43 | 31.06 | 21.24 | 0.71 | 0.55 | 8.1 (11.5) |
| ff | 8.03 | 7.53 | 39.87 | 35.98 | 0.94 | 1.95 | 3.9 (4.1) |
| chain | 2.28 | 2.16 | 11.08 | 10.46 | 0.95 | 0.53 | 4.1 (4.3) |

Under `--tier=llvm` and `--tier=auto` `lfsr` takes 5.08 and 4.53 G cycles (5.98 and 6.35 before). The random-number case of the utility library's testbenches takes 5.75 G cycles and 23.12 G instructions before, 5.63 G and 22.86 G now; `strings_heavy`, `osvvm_axistream`, `protected_scoreboard`, `access_heavy`, `calls` are within noise of 1.0 (min of 5 interleaved runs). What the profile of `lfsr` shows now (about 8300 instructions per activation): the loop 31%, the kernel's sorting of the changed ranges of one 512-process signal 8%, `update_effective` 6%, `Buf::select` 5%, `Machine::cat` 4% (about 350 instructions a call), the rest kernel assignment and wake paths; `ff` and `chain` are dominated by the kernel and by ops that are not index arithmetic (`BinRangeK` of `(i+1) mod N`, `Gate`, `LSetDrive`).

Verification at this change: the differential corpus (413) passes at 1 and 4 threads on the interpreter, `--tier=llvm` and `--tier=auto`, and at 4 threads with the `gc-stress` feature (interpreter, LLVM tier and shuffled); the external runner's own suites pass (all eleven, no failure); its pass lists check green with `--oracle --check` (857 and 393 tests); the second verification library's `.pro` regression passes 314/314 under 2008 and 2019; the verification library suite runs 102 pass, 9 fail, 26 skip (as before); the discrepancy catalogue checks green.

### Interpreter dispatch, round 4 (#443)

Two measurements pick the work (scratch builds, not in the repository): a dynamic histogram of the ops
the predecoded loop runs and of the pairs and triples they form, and a time profile per op (cycles between
two dispatches, subtracting the cost of the probe). They showed, over the benchmark set: `Stmt` and frame
loads 20% to 35% of the ops, a `Br` or `Move` + `Br` after almost every block, `Pos` + `Index` + load in
array code, and of the time in the verification testbenches 11% to 14% in `Copy` (a few dozen bytes each, but
through the step, three memory lookups and a staging buffer), 15% to 25% in calls and returns, `CallProt`
(through the step and an argument vector) and `LayoutOf` words.

1. More ops for what stepped out of line: `Deref`, `Field`, `LGet`, `LSet`, a `Const` defined twice, `Copy`,
   `CopyConst`, `MakeDope` and `CallProt` (enters its callee in the loop like `Call`).
2. Single ops for the frequent sequences (`pre.rs`, never across a block start): statements in a row, `Stmt` +
   frame load, two frame loads, a frame load and the compare-branch on it, a frame store + `Stmt`, `Move` +
   jump, `Pos` + `Index` + load or store, `Index` + load or store. A compare is a mask over `Ord::cmp`
   instead of a second dispatch on the operator; an 8-byte frame load has no width dispatch.
3. `Copy` and `CopyConst` copy inside one object or from frame, temporary, constant, instance or heap memory
   into temporary, frame or heap memory without the staging buffer; stores and descriptor reads take the
   inline path of loads (heap objects too); a layout word of a memoized array of scalars comes from its
   bounds; `MakeDope` writes its words from the stack.
4. A call reads the callee's parameter slots and widths, frame size, nesting and first op from its `Pre` and
   no longer touches the function (calls into hundreds of functions missed the cache on their parameter
   lists), looks the callee up once instead of twice, and `Frame` is 44 bytes instead of 72.

5. `SubtypeCheck` is an op; without a tier the loop keeps the function, code and return op of the eight
   innermost callers it entered in locals, so a return looks nothing up; a frame store fuses with a following
   `Br` or `IndexImm`; the copy fast path checks its lengths for overflow.

Release builds with the `llvm` feature (default `--tier=interp`), `python3 -I tools/bench/run.py --compare`,
median total CPU seconds of 7 interleaved runs at `--threads 1`, host load about 5: main at `78b1c398` against
main with this change.

| Benchmark | main | now | ratio |
|---|---|---|---|
| `compute_loop` | 0.12 | 0.11 | 0.87 |
| `calls` | 0.10 | 0.09 | 0.84 |
| `protected_scoreboard` | 0.08 | 0.07 | 0.89 |
| `access_heavy` | 0.08 | 0.07 | 0.83 |
| `strings_heavy` | 0.17 | 0.15 | 0.88 |
| `uvvm_util_rand_tb` | 1.83 | 1.55 | 0.85 |
| `osvvm_axistream` | 0.66 | 0.54 | 0.81 |
| `vai_window_gen` | 0.56 | 0.47 | 0.83 |

Geometric mean 0.85. The five synthetic designs at ten times their default size (min of 7 interleaved runs, so
startup is out of the ratio): `compute_loop` 1.07 to 0.93 s, `calls` 0.84 to 0.71, `protected_scoreboard` 0.59 to
0.50, `access_heavy` 0.65 to 0.50, `strings_heavy` 1.53 to 1.32.

What was measured and left: not zeroing or copying a callee's registers (the preloaded ones only, a
high-water register stack; poisoned stale registers in a debug build pass the whole corpus) gained 0% to 4% and
was dropped; a frame has 255 registers on average in the library testbenches (23% of the numbered registers
are never referenced), so dense register numbering or live-range sharing in the IR optimiser is the way to a
smaller frame and fewer cache misses (#478). What the profile shows now: natives (`textio.write` about 700
cycles a call, `to_string` of vectors), `Ret`, `Call` and `CallProt` (cache misses on cold functions),
`NewFill`, `Free`, `SubtypeCheck`, `LCopy` and `Case` through the step (each under 1.5% of the ops).

The difftest corpus (392) passes at 1 and 4 threads on the interpreter, `--tier=llvm` and `--tier=auto`, and
with the `gc-stress` feature on the interpreter and the LLVM tier (two `synopsys/` tests over the harness
timeout, as documented); the external runner's `check` (243 pass, 1 fail, 13 skipped) and
`verification_components` (572 pass, 7 fail) suites give the same verdict per test as main (the failures are
#462); the two verification libraries' suites (one: 102 pass, 9 fail, 26 skipped, the other's scripts 314/314)
give the same verdict per test.

### Verification-testbench runtime library (#428)

Sampled profiles (stack samples every 2 to 4 ms of one thread) of the shared benchmarks, the runner's
`logging` and `data_types` suites and per-operation microbenchmarks. Where the time is, by instruction kind
(share of samples in the instruction's implementation; the rest is the dispatch loop itself):

| Benchmark | dispatch loop only | `CallNative` | `Call` | `New` | `LayoutOf` | `Copy` | `LSet` |
|---|---|---|---|---|---|---|---|
| `uvvm_util_rand_tb` | 51% | 15% | 8% | 5.5% | 5% | 5% | |
| `osvvm_axistream` | 55% | 4% | 4% | 1% | 18% | 2% | |
| `uvvm_axistream_vvc_tb` | 78% | 1% | 2% | 1% | 4% | 2% | 8% |

Natives total 14% of the random testbench (`textio.write` of strings: 1.7 million calls, 8%; `to_string` of
integers 0.8 million calls, 2%), 4% of `osvvm_axistream` and 1% of `uvvm_axistream_vvc_tb`; in the runner's
`logging` suite they are 0.5% (83 ms of 16 s; the rest is start-up). So the library side is small for these
testbenches; what is left is interpreter work (calls at about 200 ns, `LayoutOf`, `New`) and the
per-suspension address search for shared-heap slots (4% of the VVC testbench, #422).

Tried and not kept (measured, no gain on the real testbenches): routing string `=`/`<` to the native
`implicit.eq` (a 20-character compare gets 5 times faster, but the testbench compares are 3 to 8
characters and the native call costs more than the inline loop: `uvvm_util_rand_tb` 1.02); block-buffered
stdout for reports (first round; kept in round 2 below, where it is measured on a pipe).

1. A native call takes its argument cells from `Machine::cells` instead of allocating a vector; `WRITE`
   of a string copies it from the memory it is in straight into the line (no temporary string, no
   justification vector) except when the string is in a heap object, which may be the line itself;
   string results (`to_string`, `'image`, `to_hstring` ...) are built with one temporary allocation
   instead of three.

| Benchmark (median total CPU of 5, s) | before | after | ratio |
|---|---|---|---|
| `uvvm_util_rand_tb` | 5.59 | 4.96 | 0.89 |
| `osvvm_axistream` | 2.03 | 1.97 | 0.97 |
| `uvvm_axistream_vvc_tb` (to 10 us) | 4.22 | 4.29 | 1.02 |
| `textio_heavy` | 0.42 | 0.35 | 0.83 |
| `file_io` (new) | 0.31 | 0.27 | 0.86 |
| `strings_heavy` (new) | 0.46 | 0.44 | 0.97 |
| `report_heavy` (new) | 0.37 | 0.36 | 0.98 |
| `access_heavy` (new) | 0.25 | 0.25 | 0.97 |

The four new internal benchmarks (`strings_heavy`, `access_heavy`, `report_heavy`, `file_io`) change the
benchmark set: earlier JSON files do not contain them.

### Verification-testbench runtime library, round 2 (#428)

Measured with stack sampling on the benchmark designs, then A/B with `tools/bench/run.py --compare`
(median of 5, total CPU, one performance core, output to a pipe as a test runner reads it). What changed:

| Change | Why it was slow |
|---|---|
| `yavs run` writes reports and `OUTPUT` text in blocks (64 KiB, every 100 ms, before any stderr write and at the end) when stdout is a pipe or file and no plugin, `--load` library or foreign subprogram is present | One write system call per report line: 4 times the CPU of a report-heavy run into a pipe (the first round measured into a file, where it is 1.5 times). A terminal keeps line-by-line output |
| Operand type lookups of the implicit operations borrow the type | An enumeration type owns its literal names: each operation cloned all of them (256 strings for CHARACTER) |
| `to_string`, `'image` of a string-like vector or string are spelled from the element codes; `to_hstring`/`to_ostring` of a `bit_vector` and `integer'image` from a word or stack buffer | A text per element, three vectors per call |
| textio READ/WRITE/READLINE/WRITELINE edit the line object in its slot (new generation, no quarantine slot), READ parses and consumes in one lookup without building a conversion error, WRITELINE uses the staging buffer, no runtime handle clone per file operation | Free + allocate of a slot per call, the slot quarantine's address search, 100-byte error values moved on every successful READ |
| `numeric_std`/`std_logic_1164` operators on vectors of 1 to 64 elements without a metavalue: one word in, integer arithmetic, one temporary block out | Four to eight allocations per operation (operand planes, bit vector, extended copies, result planes, three temporary blocks) |

| Benchmark (median total CPU of 5, s, core 2) | before | after | ratio |
|---|---|---|---|
| `report_heavy` | 0.30 | 0.11 | 0.36 |
| `ieee_heavy` (new) | 0.48 | 0.28 | 0.59 |
| `resolved_bus` | 0.22 | 0.16 | 0.74 |
| `strings_heavy` | 0.19 | 0.15 | 0.81 |
| `file_io` | 0.25 | 0.20 | 0.81 |
| `textio_heavy` | 0.34 | 0.28 | 0.85 |
| `uvvm_util_rand_tb` | 1.93 | 1.67 | 0.86 |
| `osvvm_axistream` | 0.74 | 0.64 | 0.87 |
| `vunit_vc30` | 3.81 | 3.57 | 0.94 |
| `uvvm_axistream_vvc_tb` (to 10 us) | 0.53 | 0.51 | 0.96 |

Geometric mean over the 17 benchmarks 0.84. The per-operation cost into a pipe: `report_heavy` 3.1 s to 0.6 s for
600000 iterations. Microbenchmarks (300000 iterations): `to_string(bit_vector)` 0.35 s to 0.11, `to_string(std_logic_vector)`
0.34 to 0.11, `to_hstring` 0.16 to 0.11. Output identical: difftest corpus at 1 and 4 threads (410 of 410), the
reference simulators' own tests on their pass lists (857 and 393), 314 + 314 library regression tests and the second
library's suite (102 pass, 9 fail, 26 not run: the same as before), `ieee_native` against the VHDL bodies.

Not native: the two slow dictionary tests of the data-types suite (4 s for "set and get many keys") spend 95% in
interpreted library code (string scans and frame copies); natives are 4 of 92 samples. The remaining native cost in
the real testbenches is 10 to 12% of their time spread over `write` of strings, `to_unsigned`/`+` on vectors and `to_string`.

### Verification-library testbenches against the reference simulator (#444)

Why the library testbenches were 3x to 7x slower than the reference simulator's JIT while computation was at parity:
stack samples with the VHDL call stack (a local instrumented build reads the interpreter's frames from the sampler)
and, per function, the number of IR instructions executed by kind. For the scoreboard testbench (design 3: 640 million
instructions in 4 s, 6 ns each) the picture was:

- Six library functions that scan strings character by character (the log message length, identifier matching,
  `to_string`, line wrapping) executed two thirds of the instructions; one of them (the length of a fixed-size log
  buffer) alone 35 percent.
- 23 percent of the instructions were loads and 7 percent stores of frame slots (loop counters, loop bounds,
  parameters), and a `Stmt` marker per statement was 9 percent.
- The memo of `LayoutOf` held one descriptor per type, so strings of changing lengths missed on 71 percent of 3.5
  million layouts and every miss allocated four vectors: 12 percent of the time.
- Every call copied a frame template with a cell per register: 10 GB per run, 8 percent of the time (call setup);
  register numbers had gaps and duplicate constants.
- The AXI-stream VVC testbench spent 95 percent of its instructions in loops that set one `std_logic_vector`
  element at a time (record defaults, `others` choices): 98 million `LSet`.
- A verification library's hex formatting compared a 4-element vector with 21 `case` choices element by element, each choice
  copied into a temporary first.
- Natives (`to_string`, textio) are 6 percent, heap allocation and collection 7 percent, dispatch itself 11 percent:
  not where the gap is.

Changes (CPU seconds, minimum of 5 to 7 interleaved runs, release build, interpreter tier, `--threads 1`, host
load 10 to 30, so read the ratios):

1. `LayoutOf` of arrays of changing length: the memo refills in place and a one-dimensional array of scalars gets its
   words straight from the bounds (`implicit`/`exec.rs`).
2. The optimiser keeps scalar frame slots in registers (subprograms that cannot suspend, direct loads and stores
   only, no debugger for named variables), reruns its walk over the result, empties unreachable blocks, drops `Stmt`
   markers that free nothing, merges equal constants and renumbers registers densely (`opt.rs`, decisions.md
   "Variables in registers"). Executed instructions of design 3: 640 million to 391 million.
3. `LFill` (a run of logic elements set a plane word at a time) for defaults and `others` choices; array equality of
   logic arrays (`case` choices, `=`) by `LEq`; array literals read in place from the constant pool.

| Benchmark (yavs interp) | main before the changes | after | ratio | reference `--jit` | yavs / reference |
|---|---|---|---|---|---|
| design 3 queue/scoreboard testbench | 2.64 | 2.21 | 0.84 | 0.32 (wall, 1.14 CPU) | 8.3x to 6.9x |
| design 4 AXI-stream testbench | 0.76 | 0.64 | 0.84 | 0.28 (wall, 1.10 CPU) | 2.7x to 2.3x |
| `uvvm_axistream_vvc_tb` (to 10 us) | 1.68 | 0.47 | 0.28 | | |
| `uvvm_util_rand_tb` | 2.23 | 1.82 | 0.82 | | |
| `osvvm_axistream` | 0.85 | 0.70 | 0.82 | | |

Main is `7ca41adc` (with the layout memo change of the first commit of #444 and the dispatch round 4 of #443).
The reference simulator is run as `-H 64m -e <top> --no-save --jit -r`; its CPU time is higher than its wall time
because it compiles on other threads.

What is left in these
testbenches is dispatch (11 percent of the samples in the jump of the predecoded loop), the frame template copy
(4 percent), `LayoutOf` (3 percent) and collection (4 percent): the library functions run as 6 to 12 interpreted
instructions per character of every string they touch, which the reference simulator compiles. Findings for other
areas are on #443 (dispatch, frame templates), #422 (collection) and #338 (inline `LFill`). Not done: promotion
of process variables between suspensions, temporaries of constant size in frame slots, the dynamic-bound local
arrays that are heap objects per call (`New` + `Deref` + collection).

### Register sharing (#478)

`opt/share.rs` gives registers with disjoint live ranges one frame cell (decisions.md "Registers share cells by live range"),
functions with at least 32 registers only. Registers per function (average over the functions a run optimises; "used" is
after dense renumbering, which already existed; preloaded constants and addresses are about 45 percent of it and stay):

| Benchmark | functions | lowered | used | shared |
|---|---|---|---|---|
| `uvvm_util_rand_tb` | 253 | 529 | 243 | 152 |
| `osvvm_axistream` | 361 | 375 | 164 | 94 |
| `vunit_vc30` (30 runs) | 10491 | 137 | 61 | 39 |
| `protected_scoreboard` | 6 | 64 | 28 | 21 |
| `calls` | 3 | 36 | 32 | 23 |
| `compute_loop` | 2 | 88 | 72 | 53 |

CPU (`tools/bench/run.py --compare`, 9 runs, pinned to core 2, main at `383bb6b3` as A): total CPU B/A
`uvvm_util_rand_tb` 0.99, `osvvm_axistream` 1.00, `protected_scoreboard` 0.98, `calls` 1.00, `compute_loop` 1.00,
`vunit_vc30` 1.03 (the optimiser runs once per function and the 30 runs are start-up bound: the pass is about 3 percent of
their CPU); peak MiB 0.96 to 0.99. Geometric mean 1.000: the interpreter does not gain from the smaller frames
(skipping the frame fill altogether was measured at 0 to 4 percent in #478). A first version that shared the
temporaries the predecoder fuses was 2 to 5 percent slower. Not done: preloaded constants that the predecoder folds into
its operations still occupy template cells that are never read (pre.rs would have to renumber after encoding).
Checked: difftest 410/410 at 1 and 4 threads (interpreter, gc-stress, llvm, auto), workspace tests, `yavs-llvm` tests,
OSVVM scripts 314/314, UVVM suite identical to main (102 pass, 9 fail, 26 skipped), the external runner's 11 suites all passing.

### Verification-library testbenches, round 2 (#444)

CPU seconds, minimum of 9 interleaved runs, `--threads 1`, release binaries, interpreter tier, pinned to one
performance core (`taskset -c 3`); the reference simulator `-H 64m -e <top> --no-save --jit -r` on cores 2 to 5
(minimum wall of 3). Before is main at `a3b8e7aa` (end of round 1).

| Change | what it removes |
|---|---|
| Local composite variables of subprograms that cannot suspend live in the frame's temporary memory (`Inst::NewFrame`) | 2.5 million heap objects per run of design 3 (allocation, copy, collection) |
| Written handle and address slots live in registers | 12 and 5 million slot loads in design 3 |
| Interval analysis, branch threading, dead definitions (loops only, after the first version cost small tests 10 percent) | checks, `n = 0` selects of concatenations, boolean merges; executed instructions 335 to 281 million |
| `=` and `/=` of integer and enumeration arrays as one byte comparison (`Inst::MemEq`) | the element loop of key lookups: the two slow dictionary tests of the data-types suite 2.5 and 1.3 s to 2.2 and 1.3 s |

| Benchmark | round 1 | now | ratio | reference | now / reference |
|---|---:|---:|---:|---:|---:|
| design 3 queue/scoreboard testbench | 2.77 | 2.15 | 0.78 | 0.68 wall (1.2 CPU) | 3.2x (was 4.1x) |
| design 4 AXI-stream testbench | 0.77 | 0.70 | 0.90 | 0.55 wall (1.1 CPU) | 1.3x (was 1.4x) |
| `uvvm_util_rand_tb` (harness) | 2.07 | 1.72 | 0.83 | | |
| `osvvm_axistream` | 0.65 | 0.61 | 0.93 | | |
| `uvvm_axistream_vvc_tb` | 0.49 | 0.47 | 0.96 | | |
| `strings_heavy` / `access_heavy` / `protected_scoreboard` | 0.18 / 0.08 / 0.06 | 0.16 / 0.07 / 0.06 | 0.90 / 0.84 / 1.01 | | |

Cost: both new analyses run when a subprogram is first called. A first version ran in every subprogram and cost a
200 ms verification-component test case 15 ms and a small testbench of the second verification library 25 ms;
they now run only in subprograms with a loop (and 40 instructions), which leaves +2 and +4 percent on those two
(pinned, minimum of 25). `vunit_vc30` in the harness shows +8 percent unpinned. Left: concatenation of strings is
still about 13 instructions per `&` (a fused instruction would remove them), process variables are not promoted
across suspensions, constant-size temporaries still cost an `Alloc`, the frame template copy per call
(`Pre::fill`, registers could share cells), and optimising at the first call is the floor of small tests (a cache of
optimised code would remove it).

## performance: gap map against the reference simulator (sim1), 2026-10-20

Measurement only (no code changed). yavs at `d75ef11d`, release build with the `llvm` feature, `--threads 1`, tiers
`interp` and `auto`; sim1 run the way the external test runner starts it (`-H 64m -e <top> --no-save --jit -r`).
Host: 24 cores shared with other agents, load average 3 to 12 during the runs (mostly 3 to 5), 12 to 13 GB free;
median of 3 interleaved runs (order swapped every rep), each run under `systemd-run -p MemoryMax=2G`, one timed run
at a time. Analysis is excluded from the design rows: yavs from `--time-report` (startup = elaborate + init,
simulation = simulate), sim1 as one run minus a run stopped at 0 fs (or with the generic that empties the run).
Wall time is ranked; sim1's CPU time is above its wall time because it compiles on other threads. Libraries were
analysed afresh with this binary. Ratios are yavs / sim1, so above 1 is slower. Sub-0.1 s sim1 cells are good to
about 0.01 s, so the biggest ratios (12x) are +-15%.

### Designs (total excluding analysis; startup / simulation in s)

| Rank | Row | sim1 | yavs interp | yavs auto | wall interp / sim1 | wall auto / sim1 | CPU interp / sim1 | Dominant | Peak MiB yavs / sim1 |
|---:|---|---|---|---|---:|---:|---:|---|---|
| 1 | 2b 64 LFSR, 20000 cycles | 0.08 (0.02/0.06) | 0.96 (0.00/0.96) | 0.93 | 12.0x | 11.6x | 8.7x | simulation | 60 / 72 |
| 2 | `lfsr` (512 lanes) | 0.15 (0.03/0.12) | 1.60 (0.00/1.60) | 1.83 | 10.7x | 12.2x | 9.5x | simulation | 60 / 75 |
| 3 | `ff` (4096 registers) | 0.39 (0.16/0.23) | 3.02 (0.01/3.01) | 3.41 | 7.7x | 8.7x | 7.4x | simulation | 67 / 108 |
| 4 | 2a 64 LFSR, 5000 cycles, XOR reduce | 0.05 (0.02/0.03) | 0.36 (0.00/0.36) | 0.42 | 7.3x | 8.3x | 4.0x | simulation | 61 / 74 |
| 5 | `chain` | 0.12 (0.05/0.07) | 0.79 (0.00/0.78) | 0.89 | 6.6x | 7.4x | 6.0x | simulation | 61 / 75 |
| 6 | 3 library queue/scoreboard testbench | 0.28 (0.12/0.17) | 1.81 (0.02/1.78) | 1.91 | 6.5x | 6.8x | 1.8x | simulation | 89 / 193 |
| 7 | `fan` (4000 processes on one clock) | 0.73 (0.14/0.59) | 2.41 (0.01/2.40) | 3.13 | 3.3x | 4.3x | 3.3x | simulation | 66 / 109 |
| 8 | 6a window generator (one test case) | 0.19 (0.10/0.09) | 0.42 (0.04/0.38) | 0.51 | 2.2x | 2.7x | 0.5x | simulation | 92 / 133 |
| 9 | 4 AXI-stream testbench (other library) | 0.23 (0.06/0.18) | 0.44 (0.04/0.40) | 0.48 | 1.9x | 2.1x | 0.5x | simulation | 91 / 154 |
| 10 | `bus` (resolved 32-bit bus) | 0.47 (0.01/0.45) | 0.47 (0.00/0.47) | 0.51 | 1.0x | 1.1x | 0.9x | simulation | 59 / 75 |
| 11 | 6b bias/requant (one test case) | 0.11 (0.10/0.01) | 0.07 (0.04/0.03) | 0.07 | 0.6x | 0.6x | 0.2x | startup | 88 / 117 |
| 12 | 1 compute loop, 20M iterations | 2.19 (0.01/2.18) | 1.83 (0.00/1.82) | 0.62 | 0.8x | 0.3x | 0.8x | simulation | 58 / 56 |
| 13 | `elab_heavy` (20000 instances) | 5.14 (4.72/0.10) | 0.24 (0.19/0.04) | 0.23 | 0.05x | 0.04x | 0.05x | startup | 216 / 755 |

Notes. The 6a and 6b configurations enable 13 test cases but the testbenches run only the first one per
simulation (the same on both simulators, 14.9 us and 145 ns of simulated time), so they are start-up rows; the
longer test cases of those testbenches need an embedded interpreter. `--tier=auto` beats the interpreter only on
the compute loop (3x); on every RTL and library row it is 0 to 30% slower, and on design 3 it costs 40% more CPU
and 60 MiB more memory (background compile thread). Elaboration plus initialisation is below 0.05 s on every row
except `elab_heavy`, where yavs is 20x faster than sim1. Peak memory of yavs is below sim1's on every row except
the compute loop (equal).

### Suites and end-to-end runs (single job, wall seconds)

| Rank | Row | sim1 | yavs | yavs / sim1 | Note |
|---:|---|---:|---:|---:|---|
| 1 | Second library's `.pro` suite, 314 tests (test phase) | 59.4 | 106.8 | 1.8x | per test floor 0.1 s vs 0.26 s (76 tests under 0.2 s on sim1 cost 20 s on yavs vs 7.5 s); 8 slowest tests are the long random testbenches (1.2 to 2.3 s vs 0.2 to 0.4 s). One sim1 test fails (stale-library error, order of analysis) |
| 2 | Same suite, analysis of 521 files, one call each | 22.3 | 37.0 | 1.7x | CPU 18.0 s user + 9.2 s sys on yavs vs 7.2 + 7.1 on sim1 |
| 3 | First library's suite (maintenance testbenches, 138 runs) | 163.1 | 280.0 | 1.7x | analysis included; yavs passes 102 and fails 9, sim1 89 and 22 (different sets, 26 skipped on both) |
| 4 | Difftest corpus, 409 testbenches (analysis, elaboration and run, harness included) | 21.3 | 30.8 | 1.45x | CPU 20.3 s vs 23.4 s (1.15x); yavs uses three processes per test (analyze, elab, run) |
| 5 | External test runner, data-types suite (210 tests) | 12.6 | 15.4 | 1.22x | two dictionary tests are all of it: 3.1 s and 2.0 s on yavs vs 0.9 s and 0.1 s; without them yavs wins |
| 6 | External test runner, communication suite (145 tests) | 16.2 | 11.9 | 0.74x | |
| 7 | External test runner, check suite (257 tests) | 25.8 | 13.0 | 0.50x | CPU 46 s vs 13 s |
| - | Compile step of the three suites | 1.1 to 1.4 | 0.35 to 0.45 | 0.33x | warm compilation cache |
| - | `vunit_vc30` (30 verification-component test cases, one `yavs run` each, warm cache) | not run | 2.86 CPU: analyze 0.03, elaborate 1.45, init 0.15, simulate 0.72 | | peak 106 MiB; elaboration is half, 48 ms per process |

Per-test floor of a yavs process in the test runner: 45 ms total (elaborate 10 ms, init 14 ms, simulate 18 ms for a
tiny data-types test), about 77 MiB.

### Where yavs spends the time (gdb sampling of `--tier=interp`, 5 ms stops, 60 to 400 samples per row)

| Row | Top 3 (self time of the first yavs frame) |
|---|---|
| 2b, `lfsr`, 2a | `run_ops` dispatch 15 to 21%; `update_effective` + `schedule` + `drive` (kernel signal update) 7 to 12%; `lcopy`/`temp_alloc`/memcpy of logic vectors (interp exec.rs) 6 to 10%. By file: exec.rs 45%, signal.rs 10 to 18%, kernel.rs 8 to 12% |
| `ff` | `run_ops` 18%; `value.rs` closure (vector element read/write) 6.6%; `apply_driver`/`schedule` 6%, `call_native` 3% |
| `chain` | spread: `value.rs` closure 6.5%, `run_ops` 6.5%, `update_independent` 4.8%, `take_woken` 4%; kernel is 25% of the time |
| `fan` | `run_ops` 16%; `run_batch` wake scan (`add`/`order`/`get EdgeTrigger`) 9 + 9 + 5%; `exec` 7% |
| 3 library testbench | `run_ops` 30%; `cell_w` 8%; heap (`release`, `alloc_from`, libc malloc/free) 12%; `fill` 4% |
| 4, 6a | `run_ops` 14 to 20%; `temp_alloc` 7%, `rd`/`layout_words` 6%; heap `release` 6% |
| `bus` | resolution: `value.rs` closure 12%, `planes.rs` resolve 10 + 8%, `resolve_run` 8% |
| compute loop | `run_ops` 49%, `step` 18%, `fast_op` 10% (all exec.rs) |
| `elab_heavy` | `release_for_run` 28% (drop of the elaborated scopes, `unlink_chunk`), `push_frame` 16%, `layout_words` 12% |
| dictionary tests of the data-types suite | `run_ops` 17 to 24%, `cell_w` 8 to 10%, `layout_word(s)` 14% (one case), `obj` 5% |

Method: `tools/` sampler is not in the repository (a gdb Python script that stops the process on a SIGUSR1 every 5 ms
and counts the first frame under `crates/`); no hardware profiler on this host.

### Next levers by gap (largest first)

1. Clocked RTL (2b, `lfsr`, `ff`, 2a, `chain`: 6.6x to 12x). The per-activation cost (1 to 1.5 us) is interpreter
   dispatch plus logic-vector copies, and the LLVM tier does not help (`auto` is 10 to 15% slower). Lever: lower
   process bodies over `std_logic_vector` signals natively with the vector operations inline (no side exit per
   element access), so tier-up pays off; owner `yavs-llvm` (#336 to #341) with `yavs-lower` for the instruction forms.
   Second lever, `yavs-kernel`: signal update (`update_effective`, `schedule`, `drive`) is 7 to 12% and rises to 25%
   for `chain`; batch the driver update of vector signals.
2. Library testbench (design 3, 6.5x; design 4, 1.9x): string and queue code run as interpreted instructions plus
   heap churn (12% in `heap.rs` allocate/release, `cell_w`). Lever: make `--tier=auto` profitable on call-heavy
   code (it is now slower), owner `yavs-llvm`; and fewer heap objects per call, owner `yavs-interp` (`heap.rs`).
3. `fan` (3.3x): the wake scan of 4000 processes on one clock (`run_batch`, `order`, edge trigger lookup, about 30%).
   Lever: group the waiters of one edge so a clock wakes its processes without per-process lookup; owner `yavs-kernel`.
4. Test-runner and suite floor (suites 1.7x to 1.8x, difftest 1.45x, data-types dictionary tests): per process
   cost of 0.16 to 0.26 s for a small test of the second library (library load and the warnings of an
   instance printed on every run) and the dictionary string code (`layout_word(s)`, `cell_w`). Levers:
   `yavs-interp` (`LayoutOf` for string/record layouts in the dictionary tests), `yavs` (one process for
   analyze, elab and run; not printing repeated warnings is a CLI decision for the user).
5. Resolved buses (`bus`, 1.0x): at parity; resolution functions in `yavs-logic` (`planes.rs`) and `yavs-kernel`
   `resolve_run` are 30% and would be the next step if a wider bus is measured.
   Not a gap but worth keeping: elaboration (20x faster than sim1 on 20000 instances) and the compute loop under
   `--tier=auto` (3x faster).

Raw data: median tables above come from 3 interleaved runs per row (suites: 3 runs for the test-runner suites, 1
run for the two verification-library suites and the difftest corpus numbers are medians of 3).

## Interpreter round 8: fused sequences, search loops, chains of `&` (#443, 2026-10-21)

Fused ops over the predecoded code, a search loop in one op, memoisation across the pieces of an `if` and one
concatenation for a chain of `&` (`docs/decisions.md` "Interpreter round 8"). Pinned `perf stat` (core 2, performance
cores, `--tier=interp`, median of 3 to 5), main `85eb0470` to this change; the reference column is the sim1 figure of the
head-to-head below (`-H 64m -e <top> --no-save --jit -r`).

| Row | cycles before (G) | after (G) | instructions before (G) | after (G) | ratio to sim1 before | after |
|---|---:|---:|---:|---:|---:|---:|
| scoreboard testbench (`uvvm_sb_slv_tb`) | 6.51 | 4.59 | 24.27 | 18.10 | 3.0x | about 2.1x |
| lfsr (512 x 16-bit x 5000) | 3.19 | 2.86 | 14.90 | 13.67 | 6.9x | 6.2x |
| ff (4096 registers x 2000) | 4.59 | 4.45 | 21.55 | 21.13 | 2.6x | 2.5x |
| 2a (64 LFSR x 5000, `rtl`) | 1.00 | 0.87 | 4.83 | 4.28 | 6.0x | 5.2x |
| 2b (64 LFSR x 20000, `rtl2`) | 1.70 | 1.57 | 8.14 | 7.43 | 5.5x | 5.1x |
| fan, chain, bus, ram | | | | | unchanged within 3 percent | |

`tools/bench` (`--compare`, pinned, 3 runs, total CPU after / before): scoreboard testbench 0.68, `uvvm_util_rand_tb` 0.84,
`osvvm_axistream` 0.93, `vunit_vc30` 0.98, `protected_scoreboard` 0.99 (geometric mean 0.88).

What is where: the search loop of the string helpers (`pos_of_leftmost`, 26 percent of the executed ops of the scoreboard
testbench) now runs in `Machine::scan_loop`; chains of `&` build no intermediate arrays; the fused ops remove 15 percent of
the remaining dispatches. In lfsr the else arm of the clocked process was not memoised because lowering splits it into
several blocks; `merge_chains` joins them (31 ops per activation instead of 38). Findings for later rounds: dispatch is
still 14 to 20 percent of the instructions (the loop keeps its index and op slice on the stack: a split into a small
hot loop did not help, the compiler spilled the index there too); a branch-miss count of 0.4 percent says the cost is
instructions, not prediction; frame entry and exit are about 5 percent of the scoreboard testbench, so inlining small
subprograms would not pay (not built); the kernel side of an activation (wake, run_batch, cycle, assign) is 25 percent of
lfsr and ff.

Verification at this change (release build with the `llvm` feature): differential corpus 415/415 at 1 thread, 4 threads
and 4 shuffled threads on the interpreter, `--tier=llvm` and `--tier=auto`; 415/415 at 4 shuffled threads with
`gc-stress` (interpreter and LLVM tier); `cargo test --workspace` green; the pass lists check green with `--oracle --check`
(857 and 393); discrepancy catalogue green; the runner's own suites pass (10 at the default standard, `record_dict` under
2019); the `.pro` regression 314/314 under 2008 and 2019; UVVM suite 102 pass, 9 fail, 26 skip (as before). A new corpus
test (`composite/concat_chain`) covers chains of `&`.

## Interpreter round 9: waveform-free vector assignments, `numeric_std` logical natives (#443, 2026-10-21)

The waveform buffer of a one-element assignment is not built when only the copy or element set after it and the drive read
it; a signal read and a concatenation of up to 64 logic elements fill their block once; `numeric_std` `and`, `or`, `nand`,
`nor`, `xor`, `xnor` and `not` of `unsigned` and `signed` are natives; `ieee` natives run through a lighter call op;
`rd`, the fat pair and descriptor reads of the natives, element loads and stores, the one-block `lcopy` and the scalar gate
tables are cheaper; processes of up to 256 registers stay resident under a 96 MiB budget (`docs/decisions.md`
"Interpreter round 9"). Pinned `perf stat` (core 2, performance cores, `--tier=interp`, minimum of 5, `-gNREG=64
-gCYCLES=5000` for 2a, `-gNREG=64 -gCYCLES=20000` for 2b), main `5125c987` to this change, and the sim1 figure on the same
host and core (`-H 64m -e <top> --no-save --jit -r`):

| Row | cycles before (G) | after (G) | instructions before (G) | after (G) | ratio to sim1 before | after |
|---|---:|---:|---:|---:|---:|---:|
| lfsr (512 x 16-bit x 5000) | 2.28 | 2.07 | 11.16 | 9.01 | 4.3x | 3.9x |
| 2a (64 LFSR x 5000, `rtl`) | 0.72 | 0.42 | 3.79 | 2.04 | 3.3x | 1.9x |
| 2b (64 LFSR x 20000, `rtl2`) | 1.18 | 1.02 | 5.99 | 4.76 | 3.4x | 3.0x |
| ram (16384 x 32 x 20000) | 0.35 | 0.32 | 1.16 | 0.99 | 2.0x | 1.8x |
| ff (4096 registers x 2000) | 4.37 | 3.99 | 20.92 | 17.44 | 2.4x | 2.2x |
| chain (1000 x 2000) | 1.15 | 1.05 | 6.20 | 5.43 | 2.3x | 2.1x |
| fan (4000 processes x 5000) | 5.10 | 5.14 | 18.69 | 18.85 | 1.44x | 1.46x |
| bus (resolved, 32 x 20000) | 1.05 | 0.98 | 5.10 | 4.72 | 0.44x | 0.41x |
| scoreboard testbench (`uvvm_sb_slv_tb`) | 4.44 | 4.44 | 17.93 | 17.93 | about 2.1x | about 2.1x |
| `uvvm_util_rand_tb` (`rand_basic`) | 4.09 | 4.07 | 18.05 | 17.92 | about 1.8x | about 1.8x |

`tools/bench --compare` (pinned, 3 runs, internal group, total CPU after / before, geometric mean over 13): 0.95
(`ieee_heavy` 0.67, `ff_rtl` 0.94, `resolved_bus` 0.92, `delta_chain` 0.91, the rest 0.97 to 1.04).

What is where: 2a spent its time in the `numeric_std` `xor` body (a conversion, a native call and a copy back, about
12000 instructions per call, 320000 calls); the clocked vector rows lose the waveform set-up, two zero-filled temporaries
and some checks (about 600 of 3600 instructions per activation of lfsr). Findings for later rounds: the library testbenches
are flat (the scoreboard testbench has no op over 12 percent of its instructions; the string helpers `to_string`,
`replace_backslash_n_with_lf`, `prefix_lines`, `match_identifier` (a linked-list walk) are 14, 8, 6 and 14 percent of its
ops, 12 to 26 ops per character or element, and a `Call` with a UVVM-sized frame costs about 550 instructions); an op
averages 65 instructions of which 15 to 17 are the dispatch (`k` and the op slice are spilled), so only fewer ops help;
`bin_range`+`Move` (`i := i + 1`) is the most common adjacent pair (5 percent of the scoreboard ops) and belongs to
register coalescing in the optimiser; a single-element signal read is 65 instructions, an `AllocCat` of two pieces 360,
`CopyDrive` 390 of which the kernel's `assign_blocks` is about 250; ram is mostly start-up (0.21 of its 0.32 G cycles
are elaboration and initialisation of the 16384 x 32 array).

Verification at this change (release build with the `llvm` feature): differential corpus 417/417 at 1 thread, 4 threads and
4 shuffled threads on the interpreter, `--tier=llvm` and `--tier=auto`; 417/417 with the `gc-stress` feature on the
interpreter (plain and shuffled) and the LLVM tier (shuffled); `cargo fmt --check`, `cargo clippy -D warnings` and
`cargo test --workspace` green; the pass lists check green with `--oracle --check` (857 and 393); the runner's own suites
pass (all eleven, `record_dict` under 2019); the `.pro` regression 314/314 under 2008 and 2019; UVVM suite 102 pass, 9 fail,
26 skip (as before); discrepancy catalogue green. A new corpus test (`sigassign/vector_forms`) covers the vector and
scalar assignment forms the fusions handle and the `numeric_std` logical operators with metavalues and null ranges; the
native/body comparison test (`ieee_native`) now also runs those operators on `signed` and the length-mismatch failure.

## Interpreter round 10: statement markers inside the edge test and the wait, frame 0's slots direct (#443, 2026-10-21)

The `Stmt` before the edge test and before the wait of a clocked process is part of those ops, a jump to the wait is a
copy of it, a resumed process starts past its jump back, and the fast loop reads frame 0's slots without the frame lookup
(`docs/decisions.md` "Interpreter round 10"). Head-to-head (`tools/bench/run.py --ref sim1 --instructions --runs 5`,
pinned core 5, `--tier=interp`), main `a5db170d` to this change, instructions in G and the ratio to sim1:

| Design | before | after | ratio before | after | wall ratio before | after |
|---|---:|---:|---:|---:|---:|---:|
| rtl_ff | 16.17 | 15.48 | 2.06x | 1.97x | 1.62x | 1.39x |
| rtl_lfsr | 8.37 | 8.21 | 3.00x | 2.94x | 3.00x | 2.92x |
| rtl_chain | 4.40 | 4.35 | 2.18x | 2.15x | 1.33x | 1.33x |
| rtl_fan | 17.25 | 17.01 | 1.03x | 1.02x | 1.09x | 1.06x |
| rtl_bus | 4.54 | 4.53 | 0.55x | 0.55x | 0.38x | 0.36x |
| rtl_ram | 0.96 | 0.96 | 1.83x | 1.82x | 1.50x | 1.50x |
| lfsr64_xor | 1.84 | 1.83 | 2.46x | 2.45x | 1.40x | 1.40x |
| lfsr64 | 4.56 | 4.53 | 2.90x | 2.87x | 2.25x | 2.25x |

`tools/bench --compare` (pinned core 5, internal group, 3 runs): geometric mean of total CPU 0.995. Wall cells under
0.2 s carry about 10 percent noise.

Where an activation goes now (Intel PT, every tenth instruction, per rising activation): rtl_ff about 1840 instructions,
of which the ops about 890 (12 ops: the dispatch 12 per op with `k` and the op slice on the stack, register accesses about
7 each, a single-element signal read about 40, a scalar gate about 30), the activation wrapper (`InterpProcess::run`,
`run_quick`, the `run_ops` prologue) about 265, the kernel the rest; rtl_lfsr about 3300, of which the ops and `cat_words`
about 1680 (registers 250, dispatch 215). Tried and dropped: unchecked register access (-0.3 and -1 percent of instructions
on rtl_ff and rtl_lfsr, so the checks stay and the crate keeps no `unsafe`), an unchecked op fetch (+13 percent: the
compiler laid the loop out differently), a fused read-and-gate op (-0.3 percent, less than the noise of the codegen), a
direct single-element read helper (no gain), cold out-of-line error constructors (+4 percent). Small changes anywhere in
`run_ops` move the RTL ops by up to 4 percent (#508: main `3abd2fd7` to `a5db170d`, which changed no RTL op, cost
rtl_ff 3.6 percent), so the next lever is the shape of that function, not more fusions.

Verification at this change (release build with the `llvm` feature): differential corpus 425/425 at 1 and 4 threads on
the interpreter, `--tier=llvm` and `--tier=auto`, and 4 shuffled threads on the interpreter; 425/425 with `gc-stress` on the
interpreter and the LLVM tier (4 shuffled threads); `cargo fmt --check`, `cargo clippy -D warnings` (with and without
`llvm`), `cargo test --workspace`, the `llvm` and `gc-stress` unit tests green; pass lists 857 and 393 (`--oracle
--check`); `.pro` 314/314 under 2008 and 2019; UVVM 102 pass, 9 fail, 26 skip (as before).

## Interpreter round 11: the shape of the op loop (#508, 2026-10-22)

`run_ops` keeps the dispatch and the arms the RTL processes and testbench loops run all the time; the bodies of the other
ops are `Machine::op_*` methods (`#[inline(never)]`), every error with a message is built by a cold function (`ice_f!`
and its siblings), and the helpers the hot arms use are inlined on purpose (`docs/decisions.md` "The shape of the
interpreter's op loop"). Machine code of `run_ops` (`nm -S`, release build): `exec_fast::<false>` instance 124.6 KB
(25437 instructions) to 103.6 KB (21422 instructions); the `llvm`-tier instance 144.4 KB to 116.3 KB. Op frequencies
(executed ops over the head-to-head designs, the library testbenches and `vunit_vc30`) decided what stays in.

Executed instructions, pinned `perf stat` (core 2, `--tier=interp`, user mode, `tools/bench/run.py --ref sim1 --instructions
--runs 5`), main `08a3df30` to this change:

| Design | before (G) | after (G) | change |
|---|---:|---:|---:|
| rtl_ff | 13.55 | 13.36 | -1.4% |
| rtl_lfsr | 7.41 | 7.20 | -2.8% |
| rtl_chain | 4.22 | 4.19 | -0.7% |
| rtl_fan | 15.14 | 14.94 | -1.3% |
| rtl_bus | 4.51 | 4.52 | +0.2% |
| rtl_ram | 0.84 | 0.84 | 0.0% |
| lfsr64_xor | 1.70 | 1.63 | -4.1% |
| lfsr64 | 4.04 | 3.81 | -5.7% |
| compute_h2h | 28.73 | 26.72 | -7.0% |
| xor_sig | 23.61 | 23.15 | -1.9% |
| xor_types | 19.28 | 18.98 | -1.6% |

Library testbenches (total instructions of the run): `protected_queue` -1.2%,
`uvvm_sb_slv_tb` -1.3%, `uvvm_util_rand_tb` -1.8%, `osvvm_axistream` -0.9%, `vunit_vc30` -0.7%. The real-world suite
(`tools/realbench/run.py --compare`, core 2, `--tiers interp`, 41 tests): geometric means of wall, CPU and instructions
all 0.99 B/A, per category 0.96 (wide-vectors) to 1.01 (records), no test above the slowdown thresholds.

Stability: the instructions of a build with one perturbation against the same build without it, geometric mean over the
16 designs of the set (the largest single design in brackets), before and after. A new unused op: +0.01% (0.1%) and +0.02%
(0.3%); `#[cold]` on `ice` and `rt_err`: +0.39% (rtl_ff +5.7%) and -0.06% (0.7%); a check added to the `CatLCopy` arm: +0.03%
and +0.01%; a check added to `deref`: +0.18% (0.9%) and +0.12% (0.7%, protected_queue, which calls it); a new arm that
calls `pos_len`: +0.09% and +0.01%; `#[cold]` on `lcopy`: +0.74% (rtl_ff +3.2%) and -0.44% (-0.8%); a check in the `LSet` arm:
-0.01% and +0.03%. Before, four of the seven moved some design by 0.5 to 5.7%; after, none moves any design by more than 0.8%.

Sim1 ideas used (rule 22): its interpreter loop is a bare switch calling one small function per operation, and traps leave
through exits; see the decisions entry. Tried and dropped: a `hot()` function with the common ops (+12%), `cold_path()` on
the rare arms (+1%), outlining the descriptor ops that every library testbench runs (+2% to +4%).

Verification: differential corpus 431/431 at 4 threads and 4 shuffled threads on the interpreter, `--tier=llvm` and
`--tier=auto`; 431/431 with `gc-stress` (interpreter and llvm, 4 shuffled threads); fmt, clippy (with and without
`llvm`), `cargo test --workspace`, the `llvm` and `gc-stress` unit tests green; pass lists 857 and 393 (`--oracle --check`);
`.pro` 314/314 under 2008 and 2019; UVVM 102 pass, 9 fail, 26 skip (as before).

## Optimiser round 10: clean-up of finished subprograms (#444, 2026-10-21)

A late pass over the finished code of a subprogram that cannot suspend (`crates/yavs-ir/src/opt/cfg.rs`, `docs/decisions.md`
"Clean-up of finished subprograms"; `YAVS_NO_CFG=1` turns it off): a block that ends in a jump to a small block takes that
block's code over, so `and`/`or` chains and loop tests branch where their inputs are known; the emptiness test of a fresh slice
descriptor compares the bounds and the descriptor, its block and its length go; dead pure definitions are removed by
liveness; `op; move` writes the variable directly. Threading and the slice test run only in functions with a loop.

Executed interpreter operations of the scoreboard testbench (`uvvm_sb_slv_tb`, first of its three runs, `--no-cache`): 161.3 M
to 146.7 M (-9.1 percent; threading -5.4, slice tests -4.1). Pinned `perf stat` (core 3, performance cores, user-mode
`cpu_core/instructions` and `cpu_core/cycles`, `tools/bench` designs, `--tier=interp`, one run after one warm-up), main
`45a44b62` to this change:

| Row | instructions before (G) | after (G) | cycles before (G) | after (G) |
|---|---:|---:|---:|---:|
| `uvvm_sb_slv_tb` | 18.99 | 18.78 | 4.82 | 4.73 |
| `uvvm_util_rand_tb` | 19.48 | 19.25 | 4.68 | 4.64 |
| `osvvm_axistream` | 6.15 | 6.16 | 1.95 | 1.94 |
| `strings_heavy` | 1.57 | 1.55 | 0.38 | 0.38 |
| `protected_scoreboard` | 1.03 | 1.03 | 0.18 | 0.18 |
| `vunit_vc30` (30 short runs) | 18.78 | 18.93 | 7.34 | 7.49 |
| `rtl_ff`, `rtl_lfsr`, `rtl_chain`, `rtl_fan`, `rtl_bus`, `rtl_ram` | unchanged to 0.1 percent | | unchanged to 1 percent | |

The gain is small because the removed operations are the cheap ones: an operation of the scoreboard run costs 32 cycles on
average, most of it in calls (frame setup, argument slots, the parameter loads of every callee), string concatenation, layout
words and allocation. The optimiser's own time grows: a small verification-component test case executes 0.6 percent more
instructions (7 M of 1.26 G; `vunit_vc30` shows +0.8 percent). `--tier=auto` rows are not quoted: the instruction count of the
background compile thread moves them by more than the change. Left: loop-invariant code motion of the direction select of a
`for ... in s'range` loop and of descriptor lengths, sinking of frame loads used on one arm, and cheaper calls (parameters
passed in registers needs the compiled tier's frame contract changed).

Verification at this change: `cargo test --workspace` (795), clippy and fmt, `cargo test -p yavs-llvm --features llvm`;
differential corpus 420/420 at 1 and 4 threads and 4 shuffled threads on the interpreter, `--tier=llvm` and `--tier=auto`, and
at 4 shuffled threads with `gc-stress` (interpreter and LLVM tier); `external.py --oracle --check` 857 and 393; the second
verification library's `.pro` regression 314/314 under 2008 and 2019; verification library suite 102 pass, 9 fail, 26 skip (as
before); the external runner's own suites all pass (`record_dict` under 2019).

## Protected-method calls: pointer tables, record copies, shared memory (#493, 2026-10-21)

A verification library's queue pushes and pops one character at a time through thin package subprograms that call
methods of protected types (a pointer table: a table of `(id, mode, length)` records, access values to the data, a case on
the mode). One character of `push`/`pop` is five such pairs (two reads and a length of the queue's bookkeeping, two
writes), each a call of a package function that calls a protected method: ten calls and about 260 interpreter operations,
about 34 000 host instructions per character. A window-generator testbench of a local project that pushes and pops 576-bit
expected windows through such a queue ran 15 times slower than the reference simulator (sim1) and spent 76 percent of its
instructions in the queue.

Where the time went (`perf` sampling of a queue microbenchmark, 500 push/pop pairs of a 576-bit vector, about 600
characters each; shares rounded, taken at the start):

| Cost | Share before |
|---|---|
| Call and return machinery (`Call`, `CallProt`, `Ret`, frame fill, pop, slot zeroing) | about 31 percent |
| Dispatch of the operations (one indirect jump) | about 20 percent |
| Memory access to shared-heap objects through the slow object lookup (`rd_slow`, `obj`) and the copy of the record out of the table through a staging buffer (`copy_staged`) | about 20 percent |
| Layout words: the element stride and the size of a record of known layout read through `LayoutOf` for every table access (`layout_words`, `layout_word`, and the statement marker that kept the layout's temporary) | about 10 percent |
| `Case` through the generic step, `Deref` through `heap_of` (which may create a shared heap), the parameter loads of every callee | the rest |

What changed (each measured on its own; the commit messages have the details):

- The state of protected objects is in the shared heap and the shared variables: `mem_fast`, `load`, `store`, `put`,
  `store_idx` and `copy_fast` now handle `SHARED_HEAP` and `SHARED` (loads also `PKG`) without the error paths, `load` is
  inlined with the hot spaces and a cold `load_slow`, and `deref` builds the address straight from the heap it finds
  (-13 percent instructions of the queue).
- The optimiser folds the layout words that do not depend on the descriptor: size, alignment and field offsets of a record
  without descriptor and the stride of an array of such elements (`lstride`), so the element address of a table access is a
  multiplication by a constant and the statements that held a layout temporary lose their markers (-24 percent cycles).
- A `NewFrame` copy of constant size that only constant-offset loads read (`variable s : storage_t := idxs(ref)`) is not made
  when nothing between the copy and the load can change the source (`forward_frame_copies`): -6 percent cycles, and the
  methods no longer allocate temporary memory.
- `Op::Case`, the fused `DerefImm` / `LoadDerefImm`, and calls that write the registers the callee would load its parameters
  into and start after those loads and the entry's statement markers (-4 percent instructions for the parameters, -7 for
  the dereference, the rest small). Together with the optimiser changes the operations executed per push/pop pair go from
  157 000 to 96 000 (91 000 with inlining).
- Thin subprograms are inlined (`opt/inline.rs`, `YAVS_NO_INLINE=1` turns it off): a call of a function whose optimised
  body is straight-line code that reads its parameters, computes, checks and calls (the package function in front of a
  protected method) becomes that code. The body is optimised apart from the callee's published copy, so the result does
  not depend on the order functions are first used. Call stacks are unchanged: an inlined instruction keeps the callee's
  statement and `Func::via` the call it came from (`Machine::trace`, `std.env` call paths, the verifier; pinned by
  `tests/cli/runtime_error_thin` and `tests/corpus/env/call_path_thin`, both equal with and without inlining and the
  second equal to sim1). Parameters stay in the argument registers where the body never writes them and the result is
  made in the call's result register, so a call costs no move: the queue executes 10 percent fewer instructions and
  about 8 percent fewer cycles.

Measurements (pinned to core 2 of the performance cores, `perf stat` user-mode `cpu_core` counters, release build with the
`llvm` feature, `--tier=interp`, host load average 12 to 18, so wall times are noisy and counts are quoted; main at
`9d5a2438` before, this change after):

| Row | instructions before | after | cycles before | after |
|---|---:|---:|---:|---:|
| queue microbenchmark, per push/pop pair of a 576-bit vector | 20.4 M | 9.5 M | 3.86 M | 1.68 M |
| window-generator testbench, one test case (whole run) | 716 G | 416 G | 145 G | 88.3 G |
| `protected_queue` (new benchmark, 4000 rounds) | 72.6 G | 33.6 G | | |
| `uvvm_sb_slv_tb`, `osvvm_axistream`, `vunit_vc30`, `uvvm_util_rand_tb` | | | | CPU 0.92 to 0.96 of before |

Against sim1 (`tools/bench/run.py --ref sim1`, core 2, 3 runs): `protected_queue` 9.9 times slower (instructions 11.5
times) before, 4.6 times (5.3 times) after. The window-generator testbench: sim1 9.7 G cycles, yavs 15 times before and
9 times after. The gap of that testbench is not closed. The other 12 benchmarks of the internal group move between 0.88
and 1.04 (geometric mean of total CPU over the 14 of them 0.91; most run 0.1 s or less, so the small ones are noise).

Left, largest first (all measured on the queue):

1. The five protected-method calls per character remain, about 900 host instructions each with the callee's body. The
   method bodies have two or three paths (a case on the mode), so `opt/inline.rs` leaves them: inlining them needs blocks
   appended to the caller and the case turned into compares (case tables are shared by the program); the calls need no
   change to `shared_alloc` since every frame of a process that reaches a protected object has it.
2. Dispatch: 20 percent of the cycles are the indirect jump of the loop (misprediction), 45 M operations instead of 78 M
   for the 500-pair run; further fusions gain a few percent each (`Stmt` before a call, `Ret` followed by `Ret`,
   block-local constants of multiply-defined registers in the `assert` of `unsafe_push`).
3. `--tier=auto` compiles the hot functions of this testbench (`unsafe_push`, `get`, `set`, ...) and gains nothing: a
   compiled caller reaches the shared-heap objects through the object cache of `yavs_rt_host` and the callee through a
   side exit; `--tier=llvm` (everything compiled before time 0) runs the queue 1.3 times faster than the interpreter at
   this change (cycles per pair 1.77 M against 2.27 M in the same noisy session).

## performance: head-to-head against the two reference simulators, 2026-10-21

Reproduce with `tools/bench` (`run.py BIN --ref sim1,sim2 --tiers interp,auto --instructions --tiny`): the designs of the tables
below are checked in under `tools/bench/designs` (old names to new: `tools/bench/AGENTS.md` section 2), the method is section 7.

Measurement only (no code changed). yavs at `5ac6c1a6` (main), one release binary built with the `llvm` feature, so
`--tier=interp` and `--tier=auto` come from the same file; `--threads 1`. sim1 is a 1.23 development build with LLVM 21,
run the way the external test runner starts it (`-H 64m -e <top> --no-save --jit -r`, "jit") and as `-e` then `-r`
("-e/-r", the two commands summed, no `-H`); sim2 is a 7.0 development build with the mcode code generator (static
elaboration, no ahead-of-time or LLVM backend), run as `-r <top>` (analysis, elaboration and run in one command, as
before). Ratios are yavs / simulator, so above 1 is slower. Method as in the sections of 2026-10-19 and 2026-10-20:
analysis is excluded from the design rows (libraries analysed afresh with this binary, simulator libraries analysed
once); every timed run of every tool is pinned to the same two performance cores (`taskset -c 2,3`; builds and
analysis on 8 to 23), runs under `systemd-run -p MemoryMax=2G`, one timed run at a time, one untimed warm-up run
per cell (compilation cache, page cache), median of 3 runs with the tools interleaved (order swapped every rep).
CPU is user plus system of the whole process tree (sim1's is above its wall time because it compiles on other threads),
peak RSS is the largest process (`time -f %M`), instructions are `perf stat -e cpu_core/instructions/` of one extra run.
Host: 24 threads (8 performance cores), load average at the start of the design rows 0.55 to 1.27 (median 1.2), during
the suite runs 1.0 to 7.3 (median 1.8, our own `-j4` runs included), 16 to 24 GB available throughout. Seconds are
good to about 0.01 s, so cells below 0.1 s carry up to +-15%.

### Designs and testbenches (wall seconds, total excluding analysis)

| Row | yavs interp | yavs auto | sim1 jit | sim1 -e/-r | sim2 | interp / jit | auto / jit | interp / -e,-r | interp / sim2 |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| ff (4096 registers x 2000) | 0.89 | 0.88 | 0.34 | 0.47 | 0.38 | 2.6x | 2.6x | 1.9x | 2.3x |
| lfsr (512 x 16-bit x 5000) | 0.62 | 0.62 | 0.09 | 0.10 | 0.44 | 6.9x | 6.9x | 6.2x | 1.4x |
| chain (1000 stages x 2000) | 0.23 | 0.24 | 0.09 | 0.11 | 0.67 | 2.6x | 2.7x | 2.1x | 0.3x |
| fan (4000 processes x 5000) | 1.02 | 1.03 | 0.69 | 0.81 | 0.26 | 1.5x | 1.5x | 1.3x | 3.9x |
| bus (32 drivers x 20000, top bus_tb) | 0.23 | 0.23 | 0.44 | 0.46 | 0.16 | 0.5x | 0.5x | 0.5x | 1.4x |
| ram (16384 deep x 20000) | 0.08 | 0.08 | 0.02 | 0.04 | 0.25 | 4.0x | 4.0x | 2.0x | 0.3x |
| 2a 64 LFSR x 5000, XOR reduce (rtl) | 0.18 | 0.18 | 0.03 | 0.05 | 0.12 | 6.0x | 6.0x | 3.6x | 1.5x |
| 2b 64 LFSR x 20000 (rtl2) | 0.33 | 0.34 | 0.06 | 0.07 | 0.28 | 5.5x | 5.7x | 4.7x | 1.2x |
| 1 compute loop, 20M iterations | 1.88 | 1.90 | 2.16 | 2.16 | 0.19 | 0.9x | 0.9x | 0.9x | 9.9x |
| 3 queue/scoreboard testbench (`uvvm_sb_slv_tb`) | 1.22 | 1.25 | 0.40 | 0.44 | 0.41 | 3.0x | 3.1x | 2.8x | 3.0x |
| 4 AXI-stream testbench (`TbStream_AxiSendGetRandomParamAsync2`) | 0.32 | 0.33 | 0.34 | 0.37 | 0.26 | 0.9x | 1.0x | 0.9x | 1.2x |
| `osvvm_axistream` (`TbStream_AxiSendGetRandomParam2`) | 0.31 | 0.31 | 0.29 | 0.33 | 0.25 | 1.1x | 1.1x | 0.9x | 1.2x |
| `uvvm_util_rand_tb` | 0.94 | 0.96 | 0.53 | 0.54 | 0.73 | 1.8x | 1.8x | 1.7x | 1.3x |
| `uvvm_axistream_vvc_tb` (to 10 us) | 0.21 | 0.21 | 0.31 | 0.35 | 0.38 | 0.7x | 0.7x | 0.6x | 0.6x |
| `strings_heavy` | 0.09 | 0.09 | 0.07 | 0.08 | - | 1.3x | 1.3x | 1.1x | - |
| `report_heavy` | 0.09 | 0.09 | 0.15 | 0.16 | 0.35 | 0.6x | 0.6x | 0.6x | 0.3x |
| `textio_heavy` | 0.17 | 0.17 | 0.17 | 0.17 | 0.37 | 1.0x | 1.0x | 1.0x | 0.5x |
| `protected_scoreboard` | 0.04 | 0.04 | 0.02 | 0.03 | 0.01 | 2.0x | 2.0x | 1.3x | 4.0x |
| f `elab_heavy` (20000 instances) | 0.23 | 0.22 | 4.62 | 9.96 | 0.12 | 0.0x | 0.0x | 0.0x | 1.9x |

CPU seconds, instructions (billions, one core type) and peak RSS:

| Row | CPU yi | CPU ya | CPU jit | CPU -e/-r | CPU sim2 | instr. G yi | instr. G jit | instr. G sim2 | RSS MiB yi / ya | RSS jit / -e,-r / sim2 |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| ff (4096 registers x 2000) | 0.89 | 0.89 | 0.35 | 0.48 | 0.37 | 21.56 | 7.46 | 6.74 | 69 / 69 | 113 / 120 / 28 |
| lfsr (512 x 16-bit x 5000) | 0.61 | 0.61 | 0.09 | 0.11 | 0.43 | 14.91 | 2.74 | 6.59 | 61 / 61 | 76 / 76 / 16 |
| chain (1000 stages x 2000) | 0.23 | 0.23 | 0.09 | 0.11 | 0.67 | 6.33 | 1.90 | 6.01 | 62 / 62 | 81 / 82 / 10 |
| fan (4000 processes x 5000) | 1.01 | 1.04 | 0.70 | 0.82 | 0.25 | 19.62 | 16.43 | 6.28 | 68 / 68 | 111 / 118 / 11 |
| bus (32 drivers x 20000, top bus_tb) | 0.22 | 0.22 | 0.45 | 0.47 | 0.16 | 5.65 | 8.26 | 3.91 | 60 / 60 | 71 / 71 / 9 |
| ram (16384 deep x 20000) | 0.07 | 0.07 | 0.03 | 0.05 | 0.25 | 1.33 | 0.65 | 3.24 | 94 / 94 | 77 / 77 / 180 |
| 2a 64 LFSR x 5000, XOR reduce (rtl) | 0.17 | 0.18 | 0.04 | 0.06 | 0.12 | 4.67 | 0.97 | 1.94 | 61 / 61 | 73 / 73 / 11 |
| 2b 64 LFSR x 20000 (rtl2) | 0.33 | 0.34 | 0.07 | 0.08 | 0.28 | 8.35 | 1.71 | 3.37 | 61 / 61 | 73 / 73 / 11 |
| 1 compute loop, 20M iterations | 1.88 | 1.93 | 2.15 | 2.15 | 0.19 | 47.28 | 46.42 | 2.28 | 59 / 59 | 57 / 57 / 7 |
| 3 queue/scoreboard testbench (`uvvm_sb_slv_tb`) | 1.22 | 1.25 | 0.75 | 0.80 | 0.41 | 24.19 | 10.96 | 7.08 | 76 / 76 | 173 / 124 / 50 |
| 4 AXI-stream testbench (`TbStream_AxiSendGetRandomParamAsync2`) | 0.32 | 0.32 | 0.64 | 0.68 | 0.25 | 5.60 | 8.00 | 4.25 | 86 / 86 | 122 / 122 / 47 |
| `osvvm_axistream` (`TbStream_AxiSendGetRandomParam2`) | 0.30 | 0.31 | 0.54 | 0.59 | 0.25 | 5.27 | 7.01 | 4.21 | 86 / 86 | 122 / 122 / 47 |
| `uvvm_util_rand_tb` | 0.94 | 0.95 | 0.98 | 0.95 | 0.73 | 20.64 | 13.79 | 12.56 | 78 / 79 | 181 / 132 / 99 |
| `uvvm_axistream_vvc_tb` (to 10 us) | 0.21 | 0.22 | 0.54 | 0.58 | 0.37 | 3.23 | 7.22 | 6.49 | 128 / 129 | 220 / 220 / 139 |
| `strings_heavy` | 0.08 | 0.08 | 0.12 | 0.13 | - | 1.94 | 2.15 | - | 59 / 59 | 82 / 82 / - |
| `report_heavy` | 0.07 | 0.06 | 0.16 | 0.17 | 0.35 | 1.59 | 2.24 | 2.61 | 59 / 59 | 70 / 70 / 7 |
| `textio_heavy` | 0.17 | 0.17 | 0.20 | 0.21 | 0.37 | 4.14 | 4.18 | 6.14 | 60 / 60 | 134 / 86 / 87 |
| `protected_scoreboard` | 0.03 | 0.03 | 0.02 | 0.02 | 0.00 | 1.03 | 0.46 | 0.14 | 59 / 59 | 69 / 69 / 7 |
| f `elab_heavy` (20000 instances) | 0.22 | 0.22 | 4.62 | 9.98 | 0.12 | 2.54 | 8.55 | 1.71 | 233 / 233 | 756 / 789 / 95 |

`--tier=auto` equals `--tier=interp` within 3% on every row, including the compute loop, where `--tier=llvm`
(everything compiled before time 0, extra rows below) is 5.7x faster than either. The earlier gap map measured `auto` at
0.62 s against 1.82 s for this loop; here it no longer switches (the loop is one long-running process; `--verbose`
prints no compile note), see gap 3.

**Fixed 2026-10-21 (#432):** the compute loop (design 1) was no longer compiled by `--tier=auto` since `f4330993`
(tier-up within a budget; found by bisecting release builds with the `llvm` feature: `4d0c9ae9` and `e69dfe92` switch, `f4330993` does not).
It was not the sampler: the loop is sampled and the hand-over works, but the compile of the 338-instruction process
(about 100 ms at O2, estimated 135 ms) had to stay within 4% of the interpreter time seen, which a 1.9 s run only reaches at
3.4 s. A compute loop now gets 16 times the budget (`COMPUTE_BUDGET`), still after being seen for four times its compile
time. Design 1 (20M iterations, one performance core): interpreter 1.89 s, `--tier=auto` 0.83 s (it was 1.90 s), `--tier=llvm` 0.33 s;
`bench_compute_loop` at 20M iterations 3.30 s to 1.89 s. `--tier=auto` against `--tier=interp` elsewhere (median of 3,
ratio auto / interp): ff 1.02, lfsr 1.02, chain 1.01, fan 1.02, bus 1.01, ram 1.01, the internal benchmark designs 0.96 to 1.07
(1.06 `ieee_heavy` and 1.07 `elab_heavy` at 0.26 and 0.44 s, within the noise of cells that size).

Eager `--tier=llvm` against `--tier=interp`:

| Row | yavs `--tier=llvm` wall | CPU | RSS MiB | vs interp |
|---|---:|---:|---:|---:|
| 1 compute loop, 20M iterations | 0.33 | 0.32 | 77 | 0.2x |
| ff (4096 registers x 2000) | 0.97 | 0.96 | 86 | 1.1x |
| lfsr (512 x 16-bit x 5000) | 0.61 | 0.61 | 79 | 1.0x |
| chain (1000 stages x 2000) | 0.34 | 0.34 | 79 | 1.5x |
| fan (4000 processes x 5000) | 1.38 | 1.38 | 83 | 1.4x |
| bus (32 drivers x 20000, top bus_tb) | 0.30 | 0.30 | 77 | 1.3x |
| 2a 64 LFSR x 5000, XOR reduce (rtl) | 0.25 | 0.24 | 78 | 1.4x |
| 2b 64 LFSR x 20000 (rtl2) | 0.37 | 0.36 | 78 | 1.1x |
| 3 queue/scoreboard testbench (`uvvm_sb_slv_tb`) | 9.52 | 9.51 | 240 | 7.8x |
| 4 AXI-stream testbench (`TbStream_AxiSendGetRandomParamAsync2`) | 19.27 | 19.26 | 432 | 60.2x |
| `uvvm_util_rand_tb` | 25.34 | 25.33 | 592 | 27.0x |

Not runnable: sim2 cannot analyse `strings_heavy` (an attribute of a function call result, `real'image(x)'length`), so
its cells are empty. sim1 `-e` without `-H 64m` runs out of memory elaborating `uvvm_axistream_vvc_tb`; that one cell
(`-e/-r`) uses `-H 64m` on both commands. Rows 6a and 6b of the earlier sections (testbenches of an external checkout)
and `uvvm_util_queue_tb` were not repeated.

### Suites (wall seconds)

Cold is a fresh output directory (libraries analysed, compilation cache empty); second is the same command again on
the same directory. Both are one driver call, analysis included; `-j1` and `-j4` are the driver's job counts, all
tools pinned to the same two cores, so `-j4` shows scheduling, not four cores of speed-up. The runner suites start
each simulator as the runner does (sim1 `-H 64m ... --jit`, sim2 as installed); the two library suites are driven by
`tools/suites/pro.py` and `uvvm.py` through a one-line shell translator that gives sim1 and sim2 the same libraries,
files, generics and stop times as the recorded `yavs` calls. "To first run" is the time until the first simulation
starts (analysis of everything) for yavs and the library drivers. Peak RSS is the largest process of the suite.

| Suite | Tool | -j1 cold | -j1 second | -j4 cold | -j4 second | CPU -j1 cold | peak RSS MiB | pass / fail / skip (-j1 cold) | to first run (s, -j1 cold) |
|---|---|---:|---:|---:|---:|---:|---:|---|---:|
| runner `check` (257 tests) | yavs interp | 11.0 | 7.6 | 6.5 | 4.2 | 9.6 | 92 | 257 / 0 / 0 | 1.6 |
|  | yavs auto | 11.0 | 7.7 | 6.3 | 4.1 | 9.7 | 92 | 257 / 0 / 0 | 1.6 |
|  | sim1 | 22.6 | 21.6 | 20.3 | 19.2 | 30.4 | 120 | 257 / 0 / 0 | - |
|  | sim2 | 20.6 | 19.6 | 11.3 | 10.6 | 20.6 | 47 | 257 / 0 / 0 | - |
| runner `com` (145) | yavs interp | 12.9 | 7.1 | 6.0 | 4.1 | 9.2 | 125 | 145 / 0 / 0 | 1.5 |
|  | yavs auto | 12.9 | 7.2 | 6.0 | 4.1 | 9.2 | 125 | 145 / 0 / 0 | 1.5 |
|  | sim1 | 11.8 | 10.9 | 12.6 | 11.7 | 17.4 | 169 | 145 / 0 / 0 | - |
|  | sim2 | 24.7 | 23.6 | 13.6 | 12.5 | 24.8 | 52 | 145 / 0 / 0 | - |
| runner `data_types` (210) | yavs interp | 17.7 | 10.3 | 8.6 | 6.0 | 13.6 | 96 | 210 / 0 / 0 | 1.4 |
|  | yavs auto | 17.9 | 10.4 | 8.9 | 5.8 | 13.7 | 96 | 210 / 0 / 0 | 1.4 |
|  | sim1 | 11.0 | 10.2 | 12.7 | 12.0 | 15.7 | 317 | 210 / 0 / 0 | - |
|  | sim2 | 18.9 | 18.0 | 10.6 | 9.7 | 18.9 | 38 | 210 / 0 / 0 | - |
| second library `.pro` 2008 (314 tests) | yavs interp | 55.8 | 74.2 | 29.3 | 38.5 | 51.8 | 267 | 314 / 0 / 0 | 1.9 |
|  | yavs auto | 57.4 | 75.1 | 30.0 | 38.9 | 52.2 | 265 | 314 / 0 / 0 | 2.0 |
|  | sim1 | 46.9 | 46.8 | 34.0 | 34.1 | 61.8 | 149 | 313 / 1 / 0 | 4.6 |
|  | sim2 | 78.6 | 78.0 | 39.9 | 39.5 | 73.1 | 60 | 313 / 1 / 0 | 2.5 |
| first library suite (137 runs) | yavs interp | 127.3 | 124.0 | 73.2 | 70.5 | 127.5 | 588 | 102 / 9 / 26 | 8.5 |
|  | yavs auto | 126.9 | 124.1 | 73.8 | 71.5 | 129.0 | 604 | 102 / 9 / 26 | 8.4 |
|  | sim1 | 61.4 | 60.6 | 62.2 | 44.3 | 88.0 | 259 | 80 / 31 / 26 | 9.5 |
|  | sim2 | 119.7 | 118.9 | 70.0 | 69.7 | 120.2 | 1072 | 90 / 22 / 25 | 10.3 |

The runner suites pass completely on all four tools. The failing sets of the first library's suite differ (sim1 31
at -j1, 32/28/33 in later runs, sim2 22, yavs 9, the same 9 in all runs) and a failing sim1 run ends early, so its
wall time is not a like-for-like number; the `.pro` suite: sim1 and sim2 fail one test each. A second run of yavs on
the `.pro` driver is slower than the cold one (74 s against 56 s at -j1, 68 s on a third run, a fresh directory again
56 s): the driver re-analyses unchanged libraries into the existing directory, and the runs that follow are slower.

### Tiny-test corpus (`tests/corpus`, 414 files, the harness of the earlier tiny-test sections)

One analysis plus one timed run per testbench and tool, 3 reps, median of the per-rep sums, nanosecond clock with the
wrapper overhead (1.8 ms) subtracted, pinned to cores 2 and 3. Sums over the testbenches all four simulators can run
("common"); the 314 are the testbenches of the first run of this series. Per-test median in brackets.

| Set | n | yavs interp | yavs auto | sim1 -e/-r | sim1 jit | sim2 | interp / jit | interp / sim2 |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| 314 subset, common | 217 | 1.60 (7.2 ms) | 1.49 (6.6 ms) | 3.61 (14.0 ms) | 1.23 (5.6 ms) | 1.19 (2.3 ms) | 1.30x | 1.34x |
| all 414, common | 277 | 2.09 (7.2 ms) | 1.96 (6.6 ms) | 4.50 (13.9 ms) | 1.57 (5.6 ms) | 1.45 (2.4 ms) | 1.33x | 1.44x |
| all 414, each tool's runnable set | 414 / 414 / 374 / 381 / 296 | 4.57 | 4.40 | 7.70 | 2.19 | 1.51 | | |

Runnable means analysed and run without error, crash or timeout: sim1 cannot run 40 (33 with `--jit`), sim2 118 of the
414. yavs interp is faster than sim1 `-e/-r` on 276 of 277 common testbenches and never faster than sim1 `--jit`; the
per-test floor is 7 ms for yavs interp, 5.6 ms for sim1 jit and 2.3 ms for sim2. Slowest relative to sim2: the
garbage-collection stress test `access/gc_random` (65 ms against 14 ms).

### Where yavs is ahead

`bus` (0.5x of sim1 jit), `uvvm_axistream_vvc_tb` (0.7x), `report_heavy` (0.6x), `elab_heavy` (0.23 s against 4.6 s and
10 s, 20x to 43x faster; 233 MiB against 756 to 789 MiB), the AXI-stream testbench `TbStream_AxiSendGetRandomParamAsync2`
(0.9x), the external runner's suites (`check` 7.6 s against 21.6 s at -j1 second, `com` 7.1 against 10.9 and `data_types`
10.3 against 10.2 for sim1; sim2 19.6, 23.6 and 18.0), the `.pro` suite cold against sim2 (56 s against 79 s; sim1 is 47 s), peak RSS on
every design row below sim1's except `ram` (94 against 77 MiB) and the compute loop (59 against 57 MiB).

### Top 5 remaining gaps and where the time goes

`perf record --call-graph dwarf` (2.5 kHz, pinned to cores 4 and 5, 186 to about 1250 samples per row) of the three worst
rows and of the testbench row 3, self time, function names are those of the release binary.

1. Clocked RTL on `std_logic_vector` signals (yavs / sim1 jit: `lfsr` 6.9x, 2a 6.0x, 2b 5.5x, `ram` 4.0x, `ff` 2.6x, `chain`
   2.6x; instructions `lfsr` 14.9 G against 2.7 G). `lfsr` (58% in `yavs_interp`, 32% `yavs_kernel`): `Machine::run_ops`
   43.6%, `Signal::update_effective` 4.7%, `Machine::cat` 3.9%, `Ctx::assign` 3.1%, `Signal::apply_driver` 2.3%,
   `apply_txn` 2.3%, `Ctx::unchanged` 2.3%, `SimControl::run_batch` 2.1% and `cycle` 2.0%, `wide::eq_range` 2.0%. 2a (76%
   interpreter): `run_ops` 35.1%, `Machine::obj` 4.4%, `put` 3.3%, `subtype_check` closure 3.3%, `copy_bits` 2.8%,
   `ieee::small` 2.2%, `step_out` 2.2%, `drive` 2.2%. 2b (47% interpreter, 37% kernel, 11% allocator): `run_ops` 22.3%,
   `memmove` 5.5%, `update_effective` 5.3%, `drive` 3.6%, `lcopy` 3.6%, `Ctx::assign` 3.6%, `Buf::select` 3.3%, `cat` 3.3%,
   `apply_driver` 3.3%, `free` 3.0%, `mark_active` 3.0%. Neither `auto` nor eager `llvm` moves these rows (eager `llvm` is 1.0x to 1.5x slower than interp on them).
2. Library testbench with strings and queues (`uvvm_sb_slv_tb` 3.0x of sim1 jit and of sim2, instructions 24.2 G against
   11.0 G; `uvvm_util_rand_tb` 1.8x). `yavs_interp` is 91.6% of the profile: `run_ops` 64.6%, `Machine::load` 6.1%, `memmove`
   2.7%, `obj` 2.3%, `cat` 2.2%, `Pre::fill` (frame template copy) 2.0%, `layout_word` 1.4%, `step_out` 1.2%, `new_frame` 1.2%;
   allocator 5.5%. It is interpreter dispatch and frame set-up, the part sim1 compiles.
3. The `auto` tier does not pay anywhere (equal to interp on all 19 rows) and eager `llvm` is far too expensive for
   library code (9.5 s, 19.3 s and 25.3 s on three library testbenches, 8x to 60x interp) while it is 5.7x faster on the compute loop
   (0.33 s against 1.88 s interp and 2.16 s sim1). `auto` on the compute loop was 3x faster than interp in the gap map of
   2026-10-20 and is now equal: a regression or a missing switch for a single long-running process (no entry to the
   compiled code after the loop has started); worth a bisect.
4. sim2's mcode back end on small per-activation designs and on tight loops: the compute loop 9.9x (0.19 s against 1.88 s),
   `fan` 3.9x (instructions 19.6 G against 6.3 G: about 3x the work per wake-up of 4000 processes on one clock), `ff` 2.3x
   (21.6 G against 6.7 G) and `protected_scoreboard` 4x (0.04 against 0.01 s). yavs is faster than sim2 on `chain` (2.9x),
   `ram` (3.1x), `report_heavy` (3.9x) and `textio_heavy` (2.2x).
5. Floor of a run, cold and warm runs: 7 ms per tiny test against 5.6 ms (sim1 jit) and 2.3 ms (sim2); a cold runner suite
   costs yavs 3.4 s (`check`), 5.8 s (`com`) and 7.4 s (`data_types`) more than the second run, of which analysis is only 1.4
   to 1.6 s (cache fill of the first runs), so cold `com` (12.9 s) and `data_types` (17.7 s) are behind sim1 (11.8 s and
   11.0 s) although the second runs are ahead or equal; the first library's suite
   takes 127 s against 120 s for sim2 (sim1: 61 s with 31 failing tests, not comparable) at about 0.9 s per run,
   where the profile is that of gap 2; the `.pro` driver's second run is 33% slower than its first (74 s against 56 s).

Raw data: JSON per row set, the translator and the drivers are scratch and not committed.

## performance: floor of a run, 2026-10-22 (round 3, #426)

What a tiny warm `yavs run` costs in the default build (`--tier=auto`, the LLVM stubs), on two corpus tests: `scalar/array_attrs`
(no package) and `composite/concat_chain` (uses `ieee.std_logic_1164`, like most testbenches). Warm cache, run 5; `perf stat`
on a pinned core, medians of 80 interleaved runs, the whole process including its threads; cycles move by 10% with the load
of the host, instructions do not:

| Test and binary | cycles | instructions | page faults | task-clock |
|---|---:|---:|---:|---:|
| `array_attrs` before (e2cd4595) | 10.1 M | 9.36 M | 443 | 2.06 ms |
| `array_attrs` after | 8.3 M | 6.39 M | 389 | 1.70 ms |
| `concat_chain` before | 13.0 M | 14.78 M | 570 | 2.65 ms |
| `concat_chain` after | 10.0 M | 8.96 M | 471 | 2.04 ms |
| `yavs --version` (the fixed part) | 3.6 M | 2.86 M | 214 | 0.74 ms |
| `/bin/true` | 1.2 M | 0.88 M | 60 | 0.24 ms |

Where the cycles of `array_attrs` went before (sampled with frame pointers, share of all samples; `--tier=interp` of the same
binary has 3 to 12% fewer cycles, depending on the load, which is the cost of the auto tier's thread): the dynamic loader 25% (relocations of the position independent
executable and their page copies, the version checks of three libraries), the kernel's page faults (about 7 000 cycles each,
440 of them), `clap` 9.6% of the cycles (the instructions of building the whole command are small: pruning it to one subcommand saved 1.6% of a `run` and 3.7% of an `analyze`; the rest is the first touch of its code and data), the Rust
runtime's probe of the main thread's stack 7.6% (`pthread_getattr_np` reads `/proc/self/maps`, #563), the image decode 12%
(8 of 11 declaration pages decoded only by two scans of the elaborator), the two helper threads of a piped run 6% (#562),
the table of native symbols 4%, the thread that freed the analysis 3%. The 16 700 relative relocations of the executable are
7 600 pointers into read-only data (string and slice tables), 1 300 slots of the LLVM stubs and about 5 000 vtable and
function entries (1 100 of them derived `Debug`); `-z pack-relative-relocs` leaves the cost unchanged (the cost is the page
copies, not the processing).

The "before" rows are e2cd4595 and the "after" rows the code of 6f00c2fa, so they include the other agents' commits in between (the `array_attrs` gain is almost all this round's: 9.23 M to 6.71 M instructions on adjacent builds of e2cd4595 with and without it; the `concat_chain` row has an unmeasured share of upstream work).

What changed (`docs/decisions.md` "Floor of a run, round 3"): the compile thread sleeps its first 5 ms; the table of native
symbols is read only for a symbol without an implementation; the scans for signals, shared variables and implicit signals of
loaded packages skip std and ieee; the image holds the packages with run-time objects; `alias_external_uses` does nothing
without external names and `is_std_ulogic` reads the name first; a design under 50 000 entries is not freed by a thread;
the LLVM stub names are one string; `analyze`, `elab` and `run` parse their command line from a command with only that
subcommand; the symbol table starts at 1 024 names.

Per test as a runner calls it (`yavs analyze` of the file, then `yavs run` of the top, a fresh library for the cold row, the
same library for the warm row; the reference simulator's analysis, elaboration and run; pinned to core 10, 15 rounds,
median over tests of the median over rounds, wall and CPU in ms; the host was loaded, wall moves with it):

| Sample | | cold wall | cold CPU | warm wall | warm CPU |
|---|---|---:|---:|---:|---:|
| 60 corpus tests | yavs before | 10.5 | 6.8 | 2.25 | 2.18 |
| | yavs after | 10.0 | 6.5 | 1.84 | 1.75 |
| | the reference simulator (analyse, elaborate, run) | 30.4 | 29.9 | 19.6 | 19.3 |
| 40 corpus tests that use `ieee.std_logic_1164` | yavs before | 20.3 | 16.5 | 3.02 | 2.91 |
| | yavs after | 19.8 | 16.0 | 2.26 | 2.16 |
| | the reference simulator | 32.0 | 31.5 | 20.7 | 20.4 |

(Cold is the first `analyze` plus first `run` in a fresh library: 3.3 ms of CPU for `analyze`, most of it analyzing
`std.standard` from source, #564, and a sync of the manifest of 3 to 5 ms of wall, #561; the cold row of the tests that use
`std_logic_1164` also analyzes `ieee`.) With `tools/bench/tiny.py` (analysis excluded, the reference's elaborate and run; 411 tests all tools run): median per test 2.9 ms
for yavs at e9d80fff against 22.9 ms for the reference simulator; the median corpus test is mostly elaboration and simulation, not
fixed cost. Two invocations of that tool cannot be compared with each other on this host: it subtracts the cost of `true` under its
wrapper measured once at the start (3.59 ms in the invocation of the old binary, load average 60, and 2.71 ms in the new one, load
15), a constant that moved the tests under 3 ms by +13% and the rest by -1%, against -20% in the interleaved table above.
Runs of 0.2 to 2 s (`tools/bench` designs, `--tier=auto`, pinned to core 9, binary of e9d80fff against the one before this round):
instructions 12.31 G against 12.26 G for `rtl_ff`, 19.78 G against 19.77 G for `xor_sig` (`rtl_lfsr` 5.60 G against 7.38 G is another
agent's compile of its process); `--verbose` compile notes in three runs each: `rtl_ff` 2, `lfsr64` 2, `compute_h2h` 1 for both
binaries, `protected_queue` between 4 and 13 for both (it depends on timing). Checks on that binary (e9d80fff, before the external-name fix 3b29ba95; the external lists, the 30 analysis corpora, the discrepancy cases and the workspace tests were run again on 3b29ba95, the loop and difftest were not): the warm-identity loop (analyze
once, `run` four times in one library, so that runs 3 and 4 read the image: stdout, stderr and status of run 1 equal run 4 for all 451
corpus tests at `--threads 1` and `4`, on `--tier=auto`; the exception is `textio/file_append_2019`, which appends to a file and differs
the same way before) and `tools/difftest` 451/451 on `yavs-auto` at 1 and 4 threads, `yavs-interp` and `yavs-llvm` at 4.

## performance: floor of a run, 2026-10-21

Why a tiny test took 7.2 ms in the head-to-head against 5.6 ms (sim1 jit) and 2.3 ms (sim2): that binary was built with
the `llvm` feature, which linked libLLVM into the executable, and the dynamic loader relocated it (174 000 relative
relocations, 1 650 page faults, 33 M instructions) in every process, whatever the tier: `yavs --version` took
5.7 ms against 0.6 ms of a build without the feature. The executable now loads libLLVM on first use
(`docs/decisions.md` "LLVM bindings", #426). Without the feature a tiny `yavs run` was already at about 0.85 ms
above process creation; what was left, measured with `perf stat` (cycles and page faults of one warm run of a small
test, pinned to a performance core) and an early-exit probe per phase: 2.15 M cycles to the end of option parsing
(a Rust hello world is 1.03 M; `true` is 0.69 M), then analysis 1.0 M, elaboration 1.1 M, initialisation 0.9 M,
simulation 0.5 M; 560 page faults, of which 200 were the global symbol table (pre-sized for a whole library, written at
random), 43 the relocation of the executable (11 000 relative relocations in a position independent executable) and
the rest heap and code. Three helper threads (output flush, signal wait, freeing the analysis) cost 35 us together;
left as they are (changing when they start would change what a killed run prints).

What changed (`docs/decisions.md` "Floor of a run: symbol table size, pruning on demand, executable layout"): the symbol
table starts at 2048 names, not 16384 (-190 page faults, -0.13 ms); a `-no-pie` link of the executable on Linux was
tried (-45 page faults, -7% cycles) and reverted: the user chose PIE for hardening and packaging, so the executable
stays a position independent executable everywhere (the rows below marked "no-pie" were measured with that link);
`Store::put` no longer asks the size and age of every entry on every put (0.25 ms for 90
entries, once per put, so 0.5 to 1 ms of a first run in a library with 60 to 90 entries), nor `note`/`sightings` the age of
up to 512 notes.

Per-run time of the 279 corpus tests that all of yavs, sim1 and sim2 run (warm cache, `yavs run --tier=interp`, one
process per test, python `subprocess` timing, so each figure holds about 0.6 ms of process creation that every tool pays;
pinned to performance core 5; per-test median over rounds, then median over tests; the four yavs binaries interleaved per
test, 4 rounds, host load average 9):

| Binary | run 1 (empty cache) | run 2 | warm (run 3 on) |
|---|---:|---:|---:|
| built with `llvm`, before (the head-to-head's) | 8.38 ms | 8.51 ms | 6.90 ms |
| built with `llvm`, now (no-pie, reverted) | 2.32 ms | 2.50 ms | 1.29 ms |
| built without `llvm`, before | 2.62 ms | 2.81 ms | 1.48 ms |
| built without `llvm`, now (no-pie, reverted) | 2.34 ms | 2.49 ms | 1.26 ms |

Against the other two in the same harness (warm yavs now, sim1 `-H 64m -e --no-save --jit -r`, sim2 `-r`, 5 rounds, interleaved):
yavs 1.50 ms, sim1 jit 6.19 ms, sim2 2.31 ms per test (yavs is faster than sim1 on 274 and than sim2 on 272 of the 279; before,
the llvm-linked binary was slower than sim1 jit on all of them).

The cold extra cost of a runner suite is not the store: entries are written with one buffered write and a rename (no
`fsync`, no compression; encoding and writing the four entries of a closure is about 15 ms), and the pruning was 0.5 to 1 ms.
`check` suite (145 `yavs run`, one job, per-process time): the first run of a testbench (a closure) takes 88 ms against 39 ms
warm and the second 103 ms, from the third on 43 ms: 113 ms of extra per closure, 0.7 s over the 19 closures of the suite,
out of 2.1 s between the cold (`--clean`) and the second runner run; the rest is the runner itself (about 1.3 s: it rebuilds
its project database and calls `yavs analyze` 82 times). The 113 ms are two analyses from source and lowering of the
packages (the policy of the image on the second run, #426/#407: a test that runs once does not pay for an image) plus the
writing; making the image on the first run would turn them into one analysis plus about 15 ms of writing for a closure
that runs twice or more and cost a closure that runs once the 15 ms (decision for the cache owner, not taken here). CPU of
the runner suites' yavs processes cold against second, before and after: unchanged within the 3% noise (`check` 5.47/4.78 s
and 5.53/4.72 s, `com` 5.01/4.48 and 5.22/4.45, `data_types` 9.20/7.37 and 9.41/7.45): their libraries hold 66 entries and 19
notes, where pruning costs 0.15 ms a put.

Checks: difftest 414/414 at 1 and 4 threads on `--tier=interp`, `auto` and `llvm`; the runner suites `check`, `com`,
`data_types`, `logging` and `verification_components` pass; the second library's regression 314/314 under 2008 and 2019; the first library's suite 102 pass,
9 fail, 26 skip, as before; `cargo test --workspace`, clippy, fmt.

## performance: floor of a run, 2026-10-20

The per-test floor of the two verification-library suites (gap map rows 1 to 3): one `yavs run` per test, every
test a new closure. Measured with `--time-report` (`tools/bench`-style, min of 8 to 12 runs, `--tier=interp`,
`--threads 1`), the floor of a small test of the second library was cold analysis (0.11 to 0.16 s of 0.17 to 0.49 s),
then elaboration with lowering (0.04 s), package initialisation and the optimiser. The two guesses of the gap map
are not it: library load is 3 ms, and the four repeated instance warnings cost nothing measurable
(`-Wno-W1506` changes nothing).

What changed (`docs/decisions.md` "Prefix images and the image on the second analysis", "Floor of a run"):

- Closures that miss continue from a cached prefix of their files (`pre` entries in the cache of the last library of the
  prefix), so a new test case analyzes only its own files; settle finds its facts incrementally and the bodies of
  `ieee` and of the instances whose generic body is analyzed are loaded in the prefix.
- The analysis image and the lowered packages of a closure are made on its second run, not its first (a test case of a
  suite runs once: the serialisation was a fifth of its run). A closure run twice or more is unchanged from the third run.
- The analysis arenas are freed on a thread of its own (`release_analysis` took a fifth of a short run).
- The suite drivers analyze consecutive files of a library in one call (521 files: 37 s to 3 s).

Before and after on the same host, back to back (load average 11 to 13, single job; the base is the parent of these
commits, so both have the optimiser of bb322c79):

| Row | before | after | note |
|---|---:|---:|---|
| Second library's `.pro` suite, analysis of 521 files | 37.1 s | 2.9 s | gap map: reference simulator 22.3 s |
| Same suite, test phase (314 tests, 314 pass) | 150.6 s | 106.0 s | gap map: 106.8 s vs 59.4 s on a quieter host |
| First library's suite, 138 runs, analysis included | 316.5 s | 245.5 s | 102 pass, 9 fail, 26 skip both ways; gap map: reference simulator 163.1 s |
| First run of a one-process small test (cold work cache) | 171 ms | 83 ms | analysis 110 to 11 ms |
| First run of a mid-size AXI test | 489 ms | 369 ms | analysis 156 to 23 ms |
| Second and later runs | unchanged | unchanged | |
| Test-runner `check` suite (257 tests, one job) | 17 to 20 s | 18 to 20 s | within noise: the second test of a testbench now pays the analysis once more, the first saves it |

On a quieter host, before the optimiser change below, the test phase of the suite measured 109.8 s before and 76.2 s
after (the reference simulator: 59.4 s, so 1.8x became 1.3x).

Outputs: difftest 409/409 at 1 and 4 threads, the second library's suite 314/314 under 2008 and 2019, the first
library's 102/9/26, the three test-runner suites pass, `cargo test --workspace` and the cache tests (a new one
covers the prefix).

Left in the floor (not changed here, owners named):

1. The optimiser (`yavs-ir` `opt`) is now 45 to 60 percent of a small test: bb322c79 (interval analysis, #444) raised the
   simulate phase of a one-process small test from 20 to 51 ms (`YAVS_NO_IROPT=1`: 6 ms); `opt/range.rs:ranges` alone
   is a third of the optimiser. Reported on #444. Most of what is optimised runs once (library initialisation and report
   code); a call-count tier-up needs the version of a function in the frame (frames find their function by id; `pre` and
   `live` are cached per id), so it is an interpreter change.
2. Elaboration lowers every subprogram a testbench reaches from the packages again on each run (about 40 ms: lowering,
   `emit`, verify 9 ms); only the packages with run-time objects are cached (`low`). Verifying lazily per function
   would save 9 ms.
3. Package initialisation (20 to 28 ms for a library with a protected-type logger) waits for #422.

### Floor of a run: one signal thread; relocations and line tables (#562, #600, 2026-10-23)

A `yavs run` started three helper threads for Ctrl-C/SIGTERM, SIGUSR1 hang reports and writing out piped output; it starts
one (`yavs_wave::interrupt::start`). Tiny run (`array_attrs`, stdout a pipe, bench builds, pinned core, 100 interleaved
runs): 7.76 M to 7.54 M instructions, 10.96 M to 10.59 M cycles (-3.4%); with the `llvm` feature 8.08 M to 7.82 M and 10.25 M
to 9.81 M (-4.3%); a warm `analyze` 1 to 2% fewer instructions, cycles level. Packed relative relocations
(`-z pack-relative-relocs`) do not help (cycles +1 to 3%, 12 000 more instructions in `--version`) and cannot be linked by the
release toolchain (zig 0.14.1, x86_64 and aarch64 at glibc 2.17); release line tables were not loaded and changed nothing at
run time (they were 17.6 MB of the 24.9 MB gzipped executable); the user dropped them from release builds on 2026-10-23
(`debug = false`, `.symtab` stays; bench builds keep them). `docs/decisions.md` "Release executable: no packed
relocations, no line tables; one signal thread".

### floor of a run: the lowered design is cached (#479)

Elaboration lowered every subprogram a library testbench reaches again on each run and verified all of
them. The stage cache now keeps the lowered design (`ir` entries: program, architecture IR, the
elaboration-time lowering results) and the optimised copies of the functions earlier runs called; a
run that called more writes the entry again at its end. Release builds verify a function when it is
first called (debug builds, tests and `YAVS_VERIFY_IR` verify at build; stored programs are verified when
stored). Small library testbench, second run onwards, pinned to one performance core, `--stop-time=1ns`
(min of 15): total 191 to 111 ms (elaborate 110 to 72, init 21 to 3, simulate 35 to 7). Left: the
elaboration walk and session (about 40 ms); the prefix-keyed form for closures that run once is the next section.

### floor of a run: the library is shared between closures (#407)

A suite whose every test is a new closure (the second library's `.pro` regression: 314 tests, one process
each) got only the analysis prefix from the cache: the lowered library, the state of its packages after
initialisation and the optimised library functions were made again by every test. They are now kept per library
prefix (`docs/decisions.md` "Library stages": kinds `lib`, `libinit`, `libopt` next to the prefix image, made on the
fourth closure that asks for the prefix, read by the first run of every closure that starts with it), and a package body in a file of its own placed after its package in
the closure (the settings package of that library had its body last, after every prefix).

Measured against main at 56744da3 (with the mapped entries and lazily decoded bodies of #481), release builds, one
performance core (core 3) for single runs, the suites on cores 0-7 at four jobs, interleaved pairs, host load average 4
to 11 (CPU is the comparison; wall moves with the other agents' load):

| | main | this change | note |
|---|---:|---:|---|
| First run of a new small test (library cache warm, min of 15, `--time-report`) | 106 to 109 ms | 70 to 71 ms | analyze 10 to 10, elaborate 27 to 22, init 11 to 9, simulate 57 to 30 |
| Second and later runs of a test (`ir` hit) | 50 to 52 ms | 50 to 51 ms | unchanged |
| `.pro` regression, 2008, 314 tests (3 pairs, load average 4) | 21.5, 24.8, 21.7 s wall; 73.5, 83.4, 73.6 s CPU | 18.0, 17.5, 17.0 s; 59.1, 54.5, 54.1 s | wall -16 to -30 percent, CPU -20 to -35 percent; at load 8 to 11: 26.7, 24.7, 22.7 s to 22.8, 21.5, 20.1 s |
| `.pro` regression, 2019 (2 pairs) | 21.1, 23.3 s wall; 70.0, 78.2 s CPU | 19.0, 21.1 s; 60.4, 68.5 s | CPU -12 to -14 percent |

Peak memory of a test run is 58 MB (main 49 MB: the 10 MB entry is mapped); the run that makes the library for a prefix
peaks at 139 MB and takes 0.36 s once (two lowerings, one to find what depends on the design, and the image). Disk: the
`.pro` regression leaves 199 MB of cache in 50 entries (25 MB more than before, 174 MB in 34: 11 lowered libraries of
10 MB, under 1 MB of optimised copies each, fewer prefix images since bodies moved into the prefixes); a library
directory keeps at most 96 entries and 512 MiB.

Outputs: the logs of all 314 tests under 2008 (three pairs; also against a run with `--no-cache`) and under 2019 (two
pairs) and of the 111 runs of the first library's suite that run are byte-identical to main's (path-normalised); the
external runner's 11 suites pass (1344 tests); the differential corpus passes 413/413 at 1 and 4 threads, each time
three passes (cold, stored, warm) on the interpreter and with `--tier=auto`, two with `--tier=llvm`, two under
`gc-stress`, one with four shuffled threads, and 68 tests of the second library's regression pass under `gc-stress`
with the library stage in use; `cargo test --workspace` (781), fmt, clippy. New tests:
`crates/yavs/tests/cache_prefix.rs` (five: sharing, editing a package of the prefix, a body that follows the prefix,
parallel first runs, damaged entries), the closure order, the prefix state, `context_dependent`, `Store::claim`,
`Store::sightings` and the byte limit. The Windows `llvm` job failed three times of five since these commits
(`auto_tier_compiles_short_callees_of_compiled_code`, "never compiled" after 60 s, #485): a timing test of the tier that
uses no cache.

The library of a prefix is made by the fourth closure that asks for it (`libstage::SIGHTINGS`; making it costs 0.35 s of
CPU, a first run saves 40 ms): on the first library's suite, whose components have one to sixteen closures per
prefix, making it on the second request made four libraries, on the fourth one; on the `.pro` regression the two are the
same within noise.

Left: one lowered library per longest prefix (suites with several prefixes of one library make one each; sharing
between them would need a program delta); the entry is still 10 MB and reading it is a few ms of a 70 ms run (a
quarter of it is the statement position of each instruction, which could be run-length coded); a `Db` that
grew at elaboration lowers as before; the first run of the first closure of a prefix pays nothing and the
second pays 0.36 s once. Found on the way: release builds never verify functions (#482).

### floor of a run: zero-copy cache entries (#481 item 2, #407)

Where a warm run spent its time on the cache entries (timers inside the process, pinned to a performance core, median
of warm runs; UVVM `generic_queue_tb`, OSVVM `TbStream_AxiSendGetRandomParam2`, the first `vunit_vc30` case, and all
30): read the file 1.7 / 3.5 / 5.4 ms (127 ms for the 30), the checksum 0.2 / 0.6 / 0.9 (22), the extra copy in
`StageCache::get` 0.5 / 1.8 / 3.4 (83), decode 3.3 / 10.4 / 13.5 (303); together 7% / 4% / 17% (25% of the 30 cases).
Now (`docs/decisions.md` "Zero-copy cache entries"): entries are mapped and validated on open, no copy, the `init`
blocks and stored optimised copies are read in place, and function bodies of the `ir` and `low` entries are decoded
when a function first runs. Same host, pinned core, back to back with alternating order, CPU time of a warm run
(sum of the medians; host load average about 17): UVVM queue testbench 86.2 to 81.9 ms (-5.0%), OSVVM AxiStream 461.6
to 448.2 ms (-2.9%), the 30 `vunit_vc30` cases 2103 to 1783 ms (-15.2%). In-process timers after the change, for the
30 cases: read 6 ms, checksum 25 (page faults of the mapping moved here), copy 0, decode 165 (was 303: `init` 53,
`sem` 43 plus its symbol table 16, `ir` 38, `low` 15). What is left in decode is the `init` heap unpack (6 MB written
from 3 MB), the analysis arena chunks and symbol table, and the headers of the 900 functions of an `ir` entry.

### Heap collection threshold (`--gc-threshold`)

`yavs run --gc-threshold SIZE` sets `Limits::gc_threshold` (default 1 MiB): the heap is collected once more was
allocated since the last collection than both the threshold and what that collection kept. It also applies to
elaboration-time evaluation. A sweep of 1, 4, 16, 64 and 256 MiB with the shared harness (`tools/bench`, which now
also reports the collections of a run as `collections` and has two scoreboard and queue testbenches,
`uvvm_util_queue_tb` and `uvvm_sb_slv_tb`). Median of 3 rounds, one run per round, each round going through all
thresholds in turn, pinned to performance core 6 of a host with load average about 21 (wall time was twice the CPU
time, so CPU is reported). Cells: CPU seconds / peak MiB / collections.

| Benchmark | 1 MiB (default) | 4 MiB | 16 MiB | 64 MiB | 256 MiB |
|---|---|---|---|---|---|
| `access_heavy` | 0.07 / 11 / 11 | 0.07 / 12 / 2 | 0.07 / 12 / 0 | 0.07 / 12 / 0 | 0.07 / 12 / 0 |
| `protected_scoreboard` | 0.06 / 11 / 0 | 0.05 / 11 / 0 | 0.05 / 11 / 0 | 0.05 / 11 / 0 | 0.06 / 11 / 0 |
| `uvvm_util_rand_tb` | 1.63 / 38 / 372 | 1.63 / 38 / 93 | 1.56 / 38 / 23 | 1.58 / 38 / 5 | 1.62 / 38 / 1 |
| `osvvm_axistream` | 0.59 / 46 / 11 | 0.58 / 46 / 2 | 0.58 / 46 / 0 | 0.58 / 46 / 0 | 0.59 / 46 / 0 |
| `uvvm_util_queue_tb` | 0.11 / 25 / 1 | 0.11 / 24 / 0 | 0.10 / 25 / 0 | 0.11 / 24 / 0 | 0.11 / 25 / 0 |
| `uvvm_sb_slv_tb` | 2.21 / 32 / 27 | 2.23 / 32 / 8 | 2.19 / 32 / 2 | 2.21 / 33 / 0 | 2.22 / 32 / 0 |
| `vunit_vc30` (30 runs) | 2.72 / 57 / 30 | 2.70 / 58 / 30 | 2.68 / 57 / 0 | 2.64 / 57 / 0 | 2.62 / 57 / 0 |
| `access/gc_random` (corpus, median of 5) | 0.09 / 14 / 13 | 0.09 / 24 / 2 | 0.09 / 33 / 0 | 0.09 / 33 / 0 | 0.09 / 33 / 0 |

`access/gc_random` at 1 KiB: 0.11 s CPU, 773 collections, 12 MiB. The rest of the garbage-collection corpus groups
(`access`, `shared`, `protected`: 27 testbenches) run in 0.0 to 0.01 s each.

- **The threshold does not move run time.** Differences between columns are inside the run-to-run noise (about 3%);
  64 times fewer collections (`uvvm_util_rand_tb`: 372 to 5) change nothing measurable. A perf profile of
  `access/gc_random` (13 collections) and of `uvvm_sb_slv_tb` puts `Heap::collect` at 0.8% and below 0.1% of the
  samples; at 773 collections the whole run costs 0.02 s more. The 196 ms against 25 ms of the head-to-head
  testbench is not collection: it is the interpreter (`Machine::run_ops` 41%, `load` 7%, `layout_words` 5%) plus the
  process start.
- **Peak memory only grows for designs that allocate a lot of garbage** (`gc_random`: 14 MiB at 1 MiB, 24 at 4 MiB,
  33 MiB at 16 MiB and up, its whole allocation); the library testbenches do not allocate enough for it to show up
  to 256 MiB. The threshold is per heap, and every process has its own, so a design with N allocating processes can
  hold N times the threshold of garbage.
- **Cost of one collection** (profile): the mark array (`vec![false; slots.len()]`, a zeroed allocation), the root
  scan (frames of the process, `live_refs` cached per function) and the sweep over all slots, live or free; about
  13 microseconds per collection in `gc_random` at 1 KiB. Nothing of it shows in a profile at the default, so the
  mark buffer is not reused (a measured gain below noise would not pay for 1 byte per slot kept in every heap).
- **Default kept at 1 MiB** (`docs/decisions.md`, "Heap collection threshold").

## performance: compiled vector process bodies (#338, #339, #432, 2026-10-20)

The LLVM tier runs process bodies over `std_logic_vector` without leaving compiled code for the
vector work: `LCopy` inline (the plane blocks through `Gen::host`, copied by
`yavs_interp::copy_bits`, the interpreter's), constants read from a copy in the module (the object lookup each
activation was 5% of `rtl2`), and `SigRead`, `DriveSlot` and `WaitFor` as direct calls
(`TierHost::signal`, the interpreter's bodies of #443).
`--tier=auto` compiles a hot process without a loop of its own at O2 at once (its O0 code ran
no faster than the interpreter), keeps a one-statement process interpreted (a process's way
through its suspension is no loop; loops are now cycles of blocks), and compiles a short hot
function at O2 once compiled code calls it (`numeric_std`'s `"xor"` in a reduction loop left the
compiled caller and entered it again on every call). Still stepped: the numeric vector natives
(arithmetic, shifts, resize, conversions build vectors), forces, multi-part drives and reads.

RTL set, `--threads 1`, median CPU seconds of 5 runs (total without analysis), every run pinned
to performance cores 2 and 3 (`taskset -c 2,3`: the compile thread of `--tier=auto` gets the
second). Before = main at `1bea8e40`; after = main at `6dabc9fc`, this work together with the
interpreter's (#443) and the kernel's (#440) rounds and the block kernels of #199 that landed
in between, so the interp columns are mostly theirs and auto against interp of the same binary
is this work's share. sim1 run as `-H 64m -e <top> --no-save --jit -r`, pinned the same way.
Designs 2a/2b: 64 LFSR processes over `unsigned` (2a with a process XOR-reducing the 64
registers through `numeric_std`'s `"xor"`, 5000 cycles; 2b without it, 20000 cycles).

| Design | sim1 | interp before | interp after | auto before | auto after | auto after / sim1 | interp after / sim1 | auto before / sim1 |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| 2b 64 LFSR x 20000 | 0.14 | 0.94 | 0.75 | 1.06 | 0.75 | 5.4x | 5.4x | 7.6x |
| `lfsr` (512 x 16 bit x 5000) | 0.17 | 2.03 | 1.69 | 1.97 | 1.47 | 8.6x | 9.9x | 11.6x |
| `ff` (4096 x 2000) | 0.53 | 2.55 | 1.94 | 2.73 | 2.22 | 4.2x | 3.7x | 5.2x |
| 2a 64 LFSR x 5000, XOR reduce | 0.10 | 0.42 | 0.33 | 0.51 | 0.46 | 4.6x | 3.3x | 5.1x |
| `chain` (1000 x 2000) | 0.15 | 0.97 | 0.66 | 0.99 | 0.64 | 4.3x | 4.4x | 6.6x |
| `fan` (4000 x 5000) | 1.08 | 2.28 | 1.72 | 2.78 | 1.89 | 1.8x | 1.6x | 2.6x |
| `bus` (32 drivers x 20000) | 0.76 | 0.61 | 0.35 | 0.68 | 0.33 | 0.4x | 0.5x | 0.9x |

2a is short (0.3 s): its `--tier=auto` CPU includes three O2 compiles; its wall time is 0.33 s
in both tiers. `ff` under `--tier=auto` is now 14% slower than the interpreter: its 96-instruction
process is compiled at O2, and the interpreter's fused clocked-process ops (#443) run it faster
than compiled code that calls `TierHost::signal` per signal operation; `fan` stays interpreted
and pays the tier-mode loop (about 10%, #480).

Interleaved A/B pairs (7 to 9 pairs, median CPU ratio, at lower load): `--tier=auto` against
`--tier=interp` of the same binary is 0.88 on `lfsr` (wall 0.82), 0.91 wall on 2b and 1.03 wall
on 2a (the O2 compiles cost 0.2 s of CPU on a 0.3 s run); the interpreter after against before
is 0.93 (`lfsr`), 0.88 (2a), 0.93 (2b). With everything compiled before time 0 (`--tier=llvm`)
the simulate phase of 2b is 0.55 s against the interpreter's 0.67 s (O0 code: 0.67 s). Running
the interpreter with a tier set (`run_ops::<true>`) costs 4 to 13% on `ff`, `chain` and `fan`
even where nothing is compiled (#480, measured before 4d0c9ae9, which addresses it); that and the kernel (the update phase, `assign`, edge and wait
bookkeeping: half or more of every row) are what keep `--tier=auto` near the interpreter here.
Interpreted code under a tier is at the interpreter's speed (#480): functions the tier has not touched
run the plain loop even after other functions are compiled (the full loop only for a function with
compiled code, or one the tier is asked to compile), the sampling slot costs one thread-local load,
and the user cycles of `--tier=auto` against `--tier=interp` (7 interleaved pairs, core 6) are 1.010
(`fan`, was 1.06 to 1.12 once another function was compiled), 1.018 (`chain`), 1.025 (`bus`); the rest is a
fixed 15 to 20 million user cycles and 30 million kernel cycles of tier start-up (#432).
Shared benchmarks (`tools/bench/run.py --compare`, `--tier=auto` on both sides, CPU): internal
group geometric mean 0.948 over 12 (`compute_loop` 0.69, `resolved_bus` 1.18: one O2 compile on
a 0.3 s run, wall equal), external 1.014 over 5 (`vai_window_gen` 1.08); `--tier=interp`
internal 0.982. Difftest 409/409 for `yavs`, `yavs-llvm` and `yavs-auto` with
`--tier-heat=1,0,0` at 1 and 4 threads, also with a `gc-stress` build of the tier.

## CI and releases

`.github/workflows/ci.yml` runs fmt, clippy and `cargo test --workspace` (default features) on
ubuntu-24.04, ubuntu-24.04-arm and windows-latest (MSVC) for every push to main and every pull
request (#134); a `corpus` job (Linux x86_64) fetches the pinned external corpora and fails on any difference from `tools/corpus/*.expected` (#63) and on any test of `tools/difftest/external-*.pass` that no longer meets its suite's verdict (#354); not yet in CI: JIT tier. `.github/workflows/gui.yml` checks the `gui/` workspace
(#388): fmt, clippy and tests on Linux x86_64, a build on Linux aarch64 and Windows, only when
`gui/` changes. `.github/workflows/release.yml`
builds and packages `yavs` and `yavs-gui` (with `LICENSE-EUPL-1.2-yavs-gui`) with `lib/` for the three targets on a `v*` tag and publishes a
pre-release with SHA256SUMS (#165); `workflow_dispatch` with `dry_run` only builds the archives. The Linux archives are built with cargo-zigbuild against glibc 2.17 and smoke-tested in centos:7.
An installed binary finds `lib/` next to itself (#43).

## sem (`crates/yavs-sem`, M1)

- A negated abstract literal at the i64 minimum (`-9223372036854775808`, `-9223372036854775808 fs`) is `E1402` in user code (L58, #214, majority rule); only library `std` uses it (TIME's lower bound) and folds it to `i64::MIN`.
- Instances of one generic subprogram declared under one name (`function f is new f generic map (...)`)
  overload by their instantiated profiles, next to ordinary overloads and across use clauses (#28):
  `tests/diag/sem/gen/subprogram_instances_*`, difftest `generics/subprog_instance_overload` (2008, 2019).
- A subprogram header with a generic list and a generic map (`function f generic (type t) generic map
  (t => integer) parameter (v : t) return t`) declares an instance through the `is new` path, also
  with a separate body that repeats the header (#227, L61): `tests/diag/sem/gen/subprogram_header_map_*`,
  difftest `generics/subprog_header_map_2019`. A type, subprogram or package interface in a
  parameter list is `E1232` (4.2.2.1), in a port clause `E1234` (6.5.6.3); a generic list or map on a formal generic
  subprogram is `E1235` (6.5.4, #236).
- 2019 calls with a generic map (`f generic map (t => integer) (x)`, LCS-2016-049, #362, L103):
  functions, procedures and concurrent procedure calls instantiate the one generic subprogram the
  name denotes, once per call, and run: `tests/diag/sem/gen/call_generic_map_*`, difftest
  `generics/call_generic_map_2019`, `generics/call_generic_map_2019_hand`.
- 2019 signatures on generic map formals (`f [integer return integer] => inc`, LCS-2016-I03,
  #363, L104) pick one of overloaded formal subprograms in subprogram, package and entity
  instances; the signature may name a generic type of the same list; one that fits no formal is
  `E1630`: `tests/diag/sem/gen/generic_map_signature_*`, difftest `generics/generic_map_signature_2019`.
- A package declaration with a generic map aspect (4.7, 4.9, #598, L128) is analysed as an invisible
  generic package and instantiated, in a library and in a declarative part, and runs:
  `tests/diag/sem/unit/package_generic_map_*`, difftest `generics/package_generic_map`, catalogue
  `package-generic-map`; its package body is `E1999`.
- A call of a procedure that waits (directly or through other procedures) from a process with a
  sensitivity list is `E1801` at analysis when the bodies are in the process's design unit (#597,
  L129): `tests/diag/sem/expr/wait_in_procedure_called_by_*`; across units it stays a run-time error
  (corpus `process/wait_in_sensitivity_proc`, catalogue `wait-in-procedure-of-sensitivity-list-process-package`).
- Negative suite `tests/diag/sem` run by `tests/diag_sem.rs` (exact code, line and column; optional
  rendered `.stderr` snapshots); what still fails is in `tests/diag_sem_xfail.txt` (the
  aggregates and case choices group passes, #45; external names and literals, #49; aliases, #46).
- A construct the semantic analysis does not implement yet is `E1999`, naming the slice; `E9999` is
  only for internal errors (#842).
- Overload diagnostics (#51): association errors of the only candidate (`E1100` unknown formal,
  `E1310` formal twice, `E1311` missing actual, `E1301` at the first extra argument), procedures
  used as functions and functions used as statements (`E1300`), no function with the expected
  result type (`E1301`), an ambiguous procedure call reported at the one actual that decides it
  (`E1302` with a qualification help in the declaration's spelling), candidate lists in help, and
  hints for calls from several packages and for vector/integer operators.
- Subprogram calls with sub-element (individual) associations (`pr(x(0) => s)`, `f(r.i => 1)`,
  slices), also of unconstrained formals, resolve and give `Actual::Individual`; each actual is
  typed by its sub-element, a non-static formal index is `E1410`, whole and part `E1310` (#210); the elements of an unconstrained formal must form a
  contiguous range, each once: a gap is `E1311`, a repeat `E1310` (#224).
  Lowering runs them (#71), signal formals with several parts too.
- Conversions in subprogram call associations (`seti(to_r(x) => r)`, `inc(real(x) =>
  integer(r))`, #229) resolve to `Actual::Converted`, per overload candidate; conversions the
  formal's class or mode does not allow are `E1313` (6.5.7.1, 4.2.2.3). Lowering copies in
  through the actual part and back through the formal part (scalars, arrays, records).
- Mixed 2008/2019 designs (#50, sem.md 0.1, `src/rev.rs`): under a 2019 design revision a 2008
  file that names a std/ieee declaration the 2008 copy lacks (`std.env.tool_name`,
  `std.reflection`) gets `E1007` and does not see such declarations through use clauses;
  `'left`/`'right`/`'high`/`'low` of an INTEGER subtype whose bound differs (64-bit INTEGER) get
  `W1008` once per construct.
- Expression typing and conversions (#48): type conversions between types that are not closely
  related (`E1350`, 9.3.6), actuals of the wrong class for variable, signal and file parameters
  (`E1360`), an object actual of the wrong type for the only candidate (`E1300` at the actual),
  a universal bound against a non-integer one (`E1300` at that bound), ambiguous `??` (`E1341`),
  `??` found next to the condition's type when none is visible (L53), and conversion helps
  (`unsigned(x)`, `to_unsigned(x, n)`) on type mismatches.
- Object types: constants, signals, generics, ports and constant or signal parameters of an access
  or file type (or containing one) are `E1223` (#198; also in 2019, except in library `std`; 2008 protected method profiles too, 2019 accepts them, #212); protected ones `E1905`.
- Protected types (#52): signals and constants of a protected type, assignments to a protected
  variable and operators on protected operands are `E1905`; a shared variable of another type is
  `E1904` (`W1904` with `--relaxed`); a method without a body is `E1230` at its name. A shared variable of a protected type declared before its body in the same region (a package declaration) is `E1230` at the variable (#258).
- 2019 composites of protected types (#361, LCS-2016-014, 014a, L102): arrays, records and allocated
  objects whose elements are protected get one new object per element (variables, protected state,
  subprogram frames, allocators, shared variables whose scalar subelements are all protected);
  method calls through indexing, selection and `.all` run, and such types have no `=` or
  assignment (`E1905`); a record mixing protected and other elements is `E1905` (#371). A variable port of such a composite is `E1907`, not supported (#370). Sem suite
  `prot/composite_of_protected_*`, difftest `protected/composites_2019_hand`.
- Use clauses and visibility (#54): expanded names into labelled statements and subprograms
  (8.3, `E1103` outside them), and with a protected type prefix inside its body (`pt.n`, `pt.get`,
  #250) or with its own package as prefix inside the package or its body (`p.s`, #854), body
  declarations named from outside the package (`E1105`), use
  and context prefixes of the wrong kind (`E1104` at the suffix), and a missing-use-clause help
  for undeclared names that a loaded package (with `ieee` visible: `ieee_std_context`) declares.
- Static checks (#53): array attribute dimensions must be locally static (`E1410`) and exist
  (`E1623`), static indices and slices outside a static index range warn (`W1404`), and an
  constant without a value outside a package declaration is `E1224` (#213), and a deferred constant
  without a full declaration in the package body is `E1230`.
- A package body is one declarative region with its package (4.8): redeclaring a package name in the body is `E1101` (#232, lrm-checks L64).
- `tests/stdlib.rs`: std, ieee packages, and the bodies of std_logic_1164, math_real,
  math_complex, numeric_std, numeric_bit and numeric_*_unsigned analyze clean (2008 and 2019).
  `numeric_std_body_rule_1` is the rule 1 review: every object and object alias of the numeric_std
  body has its bounds from the one subtype representation.
- Declarations: objects, subprograms (operator arity `E1303`, function parameter modes, missing
  bodies `E1230`), components, generic and port clauses (components, blocks), attribute
  declarations and specifications, aliases, protected types; packages and package bodies declared
  in declarative parts (2008, generic ones too; a nested body in the outer package's body),
  signals and shared variables in processes and subprograms `E1237`, also through a nested
  package or a package instance, also one whose body declares them (#34, #284: also before the body); group templates and
  group declarations (`E1640`, `E1641`, label constituents, attributes on groups) and disconnection specifications
  (guarded signals, `others`, `all`, a static time; `E1650`, `E1651`; the time reaches the
  kernel signal) (#260).
- Types (S3): scalars, arrays, records, element/record/open constraints merged once, resolution
  indications, file element checks; the rule 1 representation is pinned on numeric_std
  (`numeric_std_subtypes_rule_1`, `numeric_std_body_rule_1`) and reviewed for the composite cases
  in `docs/design/rule1-review.md` (`tests/rule1.rs`; recommendation: approve with fixes, findings
  F1, F2, F4, F5, F7, F8 open as #98 to #111, F3 and F6 fixed); the user's approval is pending.
- Generics (S4, `generic.rs`): generic packages, subprograms and (2019) protected types; 2008
  `type T` and the 2019 classes; formal subprograms (`is <>`, `is name`) and formal packages
  (`(<>)`, `(default)`, generic map, checked against the actual instance; a parameter type through a
  formal package of a formal package, `s.r.arr`, is the actual instance's type, L76); package and subprogram
  instances, also inside generic units, by HIR cloning (visible part eager, bodies lazy via
  `Db::instance_body`); `W1501`, `W1506`; design entities with generic types, subprograms or
  packages cloned with their architecture per distinct map for elab (`generic/entity.rs`,
  `Db::instantiate_entity`, concurrent statements cloned too); components (`Db::instantiate_component`,
  per map, for elab) and blocks (in place, `Db::instantiate_blocks`) with them too, configuration
  specifications cloned with their region, default binding by name of generic types, subprograms
  and packages (#222). The fixed/float packages' declarations, generic bodies
  and instance bodies analyze clean in 2008 and 2019 (#37); mode views of generic packages and subprograms are
  cloned with them (#32). 2019 generic array types with anonymous index and element types
  (`array (type is range <>) of type is private`), an index constraint, and anonymous designated or
  file element types, in packages, subprograms and entities (#30; sem suite
  `v2019/generic_array_*`, difftest `generics/generic_arrays_2019`,
  `generics/generic_array_constrained_2019_hand`). Generic constants, ports and
  subprogram parameters of an anonymous type (`c : type is range <>`) in packages, subprograms, entities, components, blocks and
  calls (#283; sem suite `v2019/anonymous_object_*`, difftest `generics/anonymous_objects_2019_hand`). Anonymous parameter
  types also work in formal subprograms, in subprograms with a generic list and in user operators called with operator syntax
  (#308; sem suite `v2019/anonymous_{formal_subprogram,generic_subprogram,operator}*`, difftest
  `generics/anonymous_subprograms_2019_hand`). Generic protected types are instantiated with `type t is new g generic map (...)` (#29; sem suite
  `v2019/generic_protected_instance_*`, difftest `protected/generic_instance_2019`).
- 2019 mode views and private variables (S7, #32): view declarations (`E1901`, `E1902`),
  `'converse` (`E1621` on a non-view), view ports and signal parameters (`E1903`), views in
  generic units; private variables visible only in the protected body (`E1105`), method aliases
  that forward to a private variable (`E1906` for an alias of anything else), and aliases of
  methods of shared variables and variables (#237; prefixes other than an object name are
  `E1999`; a call of such a failed alias reports nothing more, #245). View ports, nested and array views, and calls through method aliases simulate
  (`hierarchy/mode_view_*`, `protected/private_variables`, `protected/method_alias_objects`), and so do signal
  parameters with a view (record, nested, `'converse` and array views, slices, several calling processes; #254), also associated element by element (#265).
  `buffer` and `linkage` subprogram parameters are `E1236`.
- 2019 front end (corpus fixes #173 to #180): `A'INDEX(n)` and `R'RECORD` type marks, `R'VALUE` of a range attribute (#282; `composite/range_value`), signals
  and variables with bounds from the initial value, variable ports of protected types, `open`
  sub-element actuals, formal packages of design entities; entity subprograms with bodies in an
  architecture. Initializer-bound signals elaborate and run (#187; records and arrays of records with such fields, #215);
  variable ports elaborate and run (#188): the actual is a shared variable or a variable port
  of the same type, never `open` or unassociated (`E1311`).
- Deferred constants at elaboration (#295): generic defaults, ranges and generate bounds read the full declaration in the package body (`generics/deferred_constant`).
- Differential corpus (#163, #181): `implicit/` covers the implicit operations end to end (radix strings, arithmetic and logic, access and file operations, edges, record equality, `hand_edges` for the LRM cases where the reference simulator deviates); `hierarchy/external_*` covers external names as port actuals (absolute, relative, generate-indexed, package). Implicit native symbols separate the operand-shape form with `#`.
- 2019 conditional expressions (`a when c else b` in initial values, assignments, actuals,
  `return` and parentheses), the conditional `return` and `unaffected` in variable and force
  assignments analyze, lower and run (#33; difftest `process/conditional_expr_2019`,
  `process/unaffected_2019`). Named return values (`return r of t`) of array type take the
  result subtype from the call's context and run (`process/named_return_2019`), so do record
  types and named return values in generic subprograms, generic packages and protected
  methods (#239, `process/named_return_record_2019`, `process/named_return_generic_2019`);
  so do scalar types, whose `r` takes the target's range (#327,
  `process/named_return_scalar_2019`). `return when c;` in procedures runs (#239,
  `process/return_when_2019`).
- Static folding: checked i64, values checked against the target subtype (`E1403`) and the base
  range, division/mod by zero and negative exponents, static length mismatch (`E1405`), static
  range and index constraints outside their type mark or index subtype (`E1215`, #465) and
  array conversions whose bounds leave the target's index subtype (`W1403`, run-time error kept).

## syntax (`crates/yavs-syntax`, M0)

Hand-written lexer, conditional analysis on the token stream and recursive-descent parser for
VHDL-2008 and VHDL-2019 into an owned AST (`yavs parse [--std 2008|2019] files...`). Diagnostics
go through `yavs-diag` (human, short and JSON formats; snapshots in `tests/diag/`). A simple PSL
subset runs as implicit checker processes, also in `-- psl` comments with `--psl` (#226) with the built-in functions `prev`, `stable`, `rose`, `fell`, `isunknown`, `countones`, `onehot`, `onehot0` and with sequences (`;`, `:`, `|`, `&&`, `&`, `within`, `[*]`, `[+]`, `[=n]`, `[->n]`, `|->`, `|=>`) in `cover`, `assert never` and suffix implications, named sequences and properties with `boolean`, `const`, `sequence` and `property` formals, and weak temporal properties (`next_a`, `next_e`, `next_event*`, `until*`, `before*`, `sync_abort`, `abort`, `async_abort` (checked at the tick), `<->`; #445); `restrict`, `restrict_guarantee`, `assume_guarantee`, `fairness` and `nondet` directives are skipped silently (formal-only); other PSL is skipped
with one `W0004` warning per design unit (#138).

Encrypted sources (#775): a `` `protect begin_protected `` envelope is one `E0012` error instead of
thousands of lexer warnings and an `E0002` (`yavs_syntax::protect`, tests in `src/tests.rs`, snapshot `tests/diag/encrypted.vhd`).

Vendor libraries (#119, #61): `tools/corpus/unisim-2008.toml` reads `unisim`/`unimacro` in place from `$YAVS_VENDOR_UNISIM` (skipped when unset): all 1073 files analyze (952 of the 1073 before the Synopsys packages; the 121 failures were exactly the missing `std_logic_arith`, `std_logic_signed` and `std_logic_unsigned`), with the VITAL (#118) and Synopsys (#117) packages built in. The Synopsys packages `std_logic_arith`, `std_logic_unsigned`, `std_logic_signed` and `std_logic_misc` are the original files, unmodified, in `lib/ieee/{2008,2019}` (`lib/SOURCES.md`, `lib/LICENSE-SYNOPSYS`), visible without a flag; `tests/corpus/synopsys/` (20 testbenches, reference simulator or the original package text for 6, `--threads 1` and `4`) passes 20 of 20. The 6 hdl-modules files that need `unisim` analyze against it.

Parse corpus: `tools/fetch-corpus.sh` fetches the corpora (default `~/.cache/rsim-corpus`), then
`YAVS_CORPUS=~/.cache/rsim-corpus cargo test -p yavs --features corpus` compares them with the
expected lists in `tests/parse-corpus/` and fails if `YAVS_CORPUS` is unset (`YAVS_BLESS=1`
updates the lists). Plain `cargo test --workspace` still parses `lib/` and runs the snapshots.

| Corpus | Files | Pass | Expected failures |
|---|---:|---:|---|
| `lib/` std and ieee, 2008 and 2019 (in repo) | 83 | 83 | none |
| External test runner library (`vunit`) | 194 | 194 | none |
| External verification library (`osvvm`) | 36 | 36 | none |
| Third-party parser library sources (std, ieee2008, synopsys, vital2000) | 37 | 37 | none |
| Reference simulator regression and parse tests | 1401 | 1366 | 35: tests of invalid syntax (most of them), VHDL-87 syntax, `abs x ** 2`, one `` `if `` that needs `--define` |
| Conformance suite `vhdl-93` | 3782 | 3244 | 538: 533 in `non_compliant/analyzer_failure`, 2 other non-compliant, 2 broken book excerpts, 1 VHDL-93 file using `default` |
| Second reference simulator testsuite | 3216 | 3009 | 207: `analyze_failure` tests and fuzz crashers, 93 code using 2008 reserved words |

Throughput on the `vunit` corpus (read, decode, lex, preprocess and parse; release build, one
thread, Intel Core Ultra 9 275HX, warm cache): about 5,600 files/s and 60 MB/s (best of three: 5,769 files/s, 62.4 MB/s).

Fuzzing: `cargo +nightly fuzz run parse` (lexer, directives, parser and rendering in both
standards); two 5-minute runs with 8 workers (210k executions in total) found no crashes or hangs.

## sem design units (`crates/yavs-sem`, M1 slice S5)

Entities, architectures (`of e` and `of lib.e`), components, instances (component, entity,
configuration; `l : c;`), generic and port maps (positional, named, sub-element, conversions,
`open`), configuration specifications and declarations (block, generate index, range and
alternative, component configurations with incremental binding), default binding
(`Db::default_binding`, `W1703`), manifest dependencies on named architectures and default-bound
entities. Negative tests in `tests/diag/sem/unit/`; unit tests in `src/unit/tests.rs`. Generic
types, subprograms and packages on components and blocks go through `generic.rs`. A subtype
indication as a generate specification (`for g(natural range 1 to 2)`) selects that range; a
range or subtype indication as the actual of an object formal is `E1361`.

## lower (`crates/yavs-lower`, M2)

`'driving` / `'driving_value` of a signal the process has no driver of is a run-time error (L116, #546); signal parameters and partly driven composites are still `E2001`.

HIR to IR for every architecture the elaborator instantiates: generics, ports (unconstrained
array ports take the actual's bounds), blocks (guarded ones with their guard process), generates
(one instance data per region), processes and concurrent statements; scalar and array
expressions with runtime descriptors (bit-planes for logic arrays), aggregates, conversions,
attributes including 'path_name/'instance_name (items of packages and package instances named by their package, #445), 2019 'length of discrete types and subtypes and constants of range record types as ranges (#445), 'driving/'driving_value and signal attributes
of elements and slices; all sequential statements including case over arrays and matching
case, aggregate targets, every wait form; subprograms with signal parameters of every mode
(arrays too, `DriveSlot` for out/inout), signals assigned in subprograms declared in a process (driven through that process's slots, #206), `signal out` actuals passed the driver slot of their own longest static prefix (#231), copy-out of logic elements and slices, individually associated parameters (`pr(x(0) => s)`, `swap(v(4) => a, v(5) => b)`, record elements, slices; unconstrained formals take their bounds from the parts; a signal formal of several parts, also in different signals and field by field for records, is a reference of several parts, #71); parameters associated through conversions (copy-in through the actual part, copy-out through the formal part, #229); nested names of signals of arrays of arrays
(`m(i)(j)`, `m(i)(7 downto 4)`, `g(i, j)`, below a slice `m(0 to 1)(0)`, through subtype
aliases) as targets, actuals and sensitivity (#89, #230); 2008 array-valued pieces of
positional aggregates as values and targets, of named aggregate targets with range or element choices (#244, #264; a lone non-static element choice too, every target length-checked against its value), and of named aggregates and arrays with open
element subtypes as values (#89, #230; without a constrained context they take the direction of their range choices, a mix is `E1334`; an array value under `others` is `E1323`, #242); `&` on arrays of arrays and of records (#234); package
objects at run time (`Lowerer::package`: constants evaluated once, composites with run-time
bounds too, shared protected variables, package signals and files, the files opened by their declaration staying open for the simulation, #834, difftest `textio/file_in_package`); composite architecture
constants evaluated once per region (#73); aliases of record elements of ports (#292) and of static logic elements and slices of variables (#296); `'range` of a package signal in subtypes (#294); bounds of record elements of ports in constants, signal subtypes and generate ranges (#304, difftest `composite/record_port_bounds`, `composite/record_in_port_bounds`); `'subtype` of objects bounded by a port, and aliases of record parts of variables keeping their element bounds (#315, difftest `composite/alias_object_subtype`); aggregates with a port's range as choice in initial values (#311, difftest `composite/agg_port_range_choice`); a constant that reads a signal or port takes its initial value (#453, difftest `hierarchy/constant_reads_signal`); elaboration-time calls of functions that read a non-protected (`--relaxed`) shared variable of their region (#524, difftest `shared/elab_call`); a lowering failure without a diagnostic is an ICE; non-protected shared variables (`--relaxed`) in
packages, architectures, blocks and generates, one per declaring region, whose processes run
serially (#285), including types holding access values, whose objects live in the shared heap (#303). `tests/corpus.rs` runs every corpus group plus
`crates/yavs-lower/tests/vhdl/run` (blessed from the reference simulator) through yavs-elab and the kernel; the external test runner's
minimal testbench runs (#110). Design and conventions: `crates/yavs-lower/DESIGN.md`.
`'element` attributes of objects with open element bounds (unconstrained ports, aliases of external names, parameters) read the object's descriptor (#235, #243; difftest `composite/element_attr_open`).
A `'element` chain of any depth (`x'element'element'left`) walks the object's descriptor level by level; sem counts the depth in `FromObject` (#256; difftest `composite/element_depth`).
A formal such as `ow(open)(7 downto 0)` has the formal's element bounds (`'element'left` is 7 whatever the actual's) and rejects an actual whose element length differs (#257, L79; difftest `composite/element_open_formal`, `element_open_formal_mismatch`).
A folded constant read as a whole carries its subtype's bounds, element bounds included, not its initialiser literals' (#247).
Composites (lower2): records (2008 unconstrained elements with runtime descriptors), arrays of
composites, multi-dimensional aggregates, composite constants, access types (`new`, `.all`,
`deallocate`), process-local protected objects (`CallProt`), protected state initial values
that call the body's methods (#204), subprograms nested in protected methods calling the
body's methods and naming its variables (#221), file objects and textio,
`force`/`release` of scalar and composite signals and their parts (every scalar subelement,
simple, conditional and selected forms, default mode from the target's port, a member of a
composite resolved as a whole is `E1362`, #68), signals of arrays of records and of records with
array-of-record fields (struct of arrays; whole, element, field, bit and slice targets,
element sensitivity, resolution as a whole, #200; field and element bounds from the initial
value, a partially constrained subtype or a port's actual, signal parameters of such types,
package signals of record types, attributes and implicit signals of parts over several
fields, #215), shared protected variables (created through `call_shared`),
user resolution functions (yavs-elab resolver over `yavs_interp::resolve`, once per element, on byte and logic leaves, #293). Difftest: access 3/3,
composite 6/6, force 8/8, protected 6/6, resolution 4/5, textio 2/3 at 1 and 4 threads.

- An alias of a constant with a static value in a subprogram has no storage; its uses lower the aliased name (#300; difftest `composite/alias_constant`).

- Entity statements (passive assertions and processes in an entity, LRM 3.2.4) lower with the architecture's, entity ones first (#301; difftest `hierarchy/entity_statements`).
- Block configurations of for-generates evaluate their index or range in the generate's scope, so a generic works as the index; errors are reported (#41; difftest `hierarchy/config_generate_index`).
- A configuration declaration may configure an entity of another library from VHDL-2019 on; before, it is `E1007` (LCS-2016-023, #445; `crates/yavs-sem/tests/config_library.rs`).
- Anonymous types in interface declarations (`a : in type is private`, LCS-2016-016) were done with #283 and #308; the LCS example is now a difftest, `generics/anonymous_ports_2019_hand` (#445).

- Aggregate variable targets of a record value, `(x, y) := f;`, positional elements (#302; difftest `composite/record_aggregate_target`).

- Aggregate targets of records with named fields, nested aggregate targets (records of records, arrays of arrays, records holding arrays) and matrix targets, for variables and signals (#350, L98; difftests `composite/aggregate_target_{logic_fields,named_record,nested_record,nested_array,matrix,matrix_cover}`, discrepancies `aggregate-target-{record-names,nested-record,nested-array,matrix}`). Still `E2001`: record signals, arrays of composites and arrays of logic arrays as elements of signal targets; arrays of logic arrays and matrices of composites as targets; non-static choices and pieces in matrix targets.

- Aggregate targets naming one object more than once (several choices in an association, or the same static name twice) are `E1335` (#307, L96; `tests/diag/sem/expr/agg_target_*`, discrepancies `aggregate-target-*`).

## sem robustness

`cargo +nightly fuzz run sem` (in `fuzz/`, seeds `tests/diag/sem`, `lib/`) parses and analyzes each input
in 2008 and 2019 against yavs's std/ieee and fails on a panic, a hang (`-timeout=20`), an RSS over 2 GB or
any diagnostic carrying "this is a bug in yavs". The negative suite fails on any ICE too, xfail or not.
`yavs analyze` over all ten `tools/corpus` manifests (debug build, so the HIR validator runs; `timeout 60`
per file) shows no panic and no timeout.

External library corpora (`tools/corpus/gen_external.py`, validated per file with the reference simulators; rejects and reasons in `tools/corpus/rejects/`): `en_cl_fix`, `open-logic`, `poc`, `json-for-vhdl`, `vhdl-extras`,
`vhlib`, `fletcher`, `satcat5`, `surf`, `neorv32`, `neotrng`, `vhdl-compliance`, `vhdl-interfaces`, `vhdl-tool-checks`,
`vhdl-ideas` and `vunit-tests`. Several carry a `[testbenches]` section (top units, runner, pass
condition) for the M2/M4 simulation tests. Results: `tools/corpus/RESULTS.md`.

Analysis corpus failures, classified 2026-10-19 (release build of main, all 29 manifests, cascades
followed to the first failing file): 3552 PASS, 299 FAIL. 292 stop on the missing Synopsys
packages (#117, #171; approach is the user's decision): `std_logic_misc` 126 (fletcher 89, vhlib
19, open-logic 18), `std_logic_unsigned` 85 and `std_logic_arith` 81 (surf); the reference
simulators accept these files. 6 (hdl-modules 2008 and 2019) need the vendor `unisim` library
(#61; they analyze with `--extra-map unisim:...`). 1 is intended (`vhdl-ideas/sim/frequency.vhdl`,
E1101, L22/L89). No other root cause is left.

| Run | Fuzz time | Executions | Crashes found | Fixed | Open |
|---|---:|---:|---:|---:|---|
| 4 workers, ~1.1 h CPU in total | ~27 min wall | ~280,000 | 3 distinct ICEs (procedure call left `ExprKind::Error`; physical-literal unit undeclared; alias of a selected object name) | 3 | none |
| 1 worker, 2 x 30 min (`parse` ~99,000 runs, `sem` ~98,000 after the fix; seeds `tests/diag/sem`, `tests/corpus`, `lib/`) | ~60 min wall | ~197,000 | `parse`: none. `sem`: `'record` on a range under 2008 was an ICE (#359) | 1 | none |

The procedure-call ICE was fixed on main in parallel; the other two are fixed here, all with
regression tests in `tests/diag/sem/fuzz/`. The last 3 chunks (~190,000 executions) found nothing.

Scheduled fuzzing: `.github/workflows/fuzz.yml` runs `parse` and `sem` nightly (and on demand) for 20
minutes each on nightly Rust with a cached corpus; a finding fails the job, uploads the minimised input
and files one `fuzz` issue per crash signature (`fuzz/AGENTS.md`). First runs found binary inputs
(`.gz` seeds, 58 to 243 KB) that take over 10 s in parse/sem (#406, #408).

## logic (`crates/yavs-logic`)

Done ahead of M3: `StdULogic` with the IEEE 1164 tables (checked against the vendored package
body), `Logic<R>`/`LogicVec` packed vectors in two layouts (nibbles, bit-planes), `BitVec` fast
path, numeric_std add/sub/compare/shift/rotate/integer conversion with X propagation, exhaustive
and property tests, criterion benchmarks. Default layout is bit-planes; see
`crates/yavs-logic/LAYOUT.md`; the user confirmed bit-planes everywhere, signals included (docs/plan.md, 2026-10-18).

## llvm: compiled clocked RTL, 2026-10-21 (#432, #339, #440)

Compiled process bodies now read vectors from the code bytes, concatenate, allocate and look up descriptors inline,
drive a vector through `Ctx::assign_blocks` and wait through one callback, and start straight from `run_quick`
(decisions.md "Compiled clocked RTL processes"). `--tier=auto` compiles a hot process body of 200 to 600 IR
instructions and no loop at O0 after a few milliseconds and again at O2 after a few more (it never compiled such
processes before: its budget needed minutes of run time). One performance core pinned (`taskset -c 2,3`), release
build with the `llvm` feature, best of 3, simulation phase (start-up subtracted): wall seconds / cycles (G).
"interp" is `--tier=interp` of the same main (kernel round 8 and interpreter round 8 included); the compile thread
runs on the same two cores here, so the cycles of `auto` include it.

| Design | interp | `--tier=auto` | `--tier=llvm` (compiled before time 0) | reference simulator (jit) |
|---|---:|---:|---:|---:|
| ff | 1.00 / 4.42 | 1.05 / 4.40 | 0.97 / 4.12 | 0.45 / 1.86 |
| lfsr | 0.54 / 2.29 | 0.48 / 2.12 | 0.37 / 1.57 | 0.13 / 0.55 |
| chain | 0.27 / 1.14 | 0.28 / 1.17 | 0.35 / 1.46 | 0.13 / 0.53 |
| fan | 1.22 / 5.10 | 1.22 / 5.14 | 1.28 / 6.11 | 0.88 / 3.60 |
| bus | 0.25 / 1.06 | 0.22 / 1.04 | 0.20 / 0.94 | 0.59 / 2.44 |
| ram | 0.052 / 0.22 | 0.047 / 0.22 | 0.045 / 0.20 | 0.05 / 0.23 |
| rtl (2a) | 0.17 / 0.72 | 0.15 / 0.72 | 0.14 / 0.66 | 0.06 / 0.28 |
| rtl2 (2b) | 0.28 / 1.19 | 0.25 / 1.20 | 0.23 / 1.09 | 0.09 / 0.39 |

`auto` is 10 to 12% faster than the interpreter on `lfsr`, `bus`, `rtl` and `rtl2`, level elsewhere, and never
slower; eager `llvm` is 1.2 to 1.5x faster on the vector designs and slower on `chain` and `fan`, whose processes
(40 to 150 instructions) touch so few cache lines per activation that entering compiled code (frame view, signal
table, registers: cold for 1000 to 4000 processes) costs more than the interpreter's fused ops. Both stay interpreted
under `auto` (`RTL_MIN`). What is left on the designs it helps is the kernel (35 to 50% of the instructions) and the
entry into compiled code (about 250 instructions per activation).

## llvm: compiled clocked RTL, round 2, 2026-10-21 (#432, #440)

Cheaper activation of compiled processes (decisions.md "Compiled clocked RTL processes, round 2"): a wait on the range
the last wait registered registers nothing (kernel, interpreter and compiled code), compiled one-element drives and
waits take the host's fast forms, a process keeps the heap object addresses it found between activations, compiled
processes use the thread scratch for registers, `LCopy` of up to 64 elements is inline, `--tier=auto` compiles
processes from 140 IR instructions. `tools/bench/run.py --ref sim1 --tiers interp,auto,llvm --instructions --runs 5
--cores 2,3` (performance cores 2 and 3, the host loaded by other builds, median of 5, analysis excluded, the compile
thread on the same two cores; `--tier=llvm` compiles everything before time 0): CPU seconds, ratio to the reference
simulator's JIT in brackets.

| Design | interp | `--tier=auto` | `--tier=llvm` | reference simulator |
|---|---:|---:|---:|---:|
| rtl_ff | 0.99 (1.52) | 0.98 (1.51) | 1.00 (1.54) | 0.65 |
| rtl_lfsr | 0.43 (3.3) | 0.44 (3.4) | 0.45 (3.5) | 0.13 |
| rtl_chain | 0.21 (1.5) | 0.22 (1.6) | 0.35 (2.5) | 0.14 |
| rtl_fan | 1.10 (1.2) | 1.11 (1.2) | 1.26 (1.4) | 0.92 |
| rtl_bus | 0.21 (0.4) | 0.23 (0.4) | 0.31 (0.5) | 0.59 |
| rtl_ram | 0.07 (0.6) | 0.07 (0.6) | 0.44 (3.7) | 0.12 |
| lfsr64_xor | 0.10 (1.1) | 0.10 (1.1) | 0.31 (3.4) | 0.09 |
| lfsr64 | 0.32 (2.5) | 0.32 (2.5) | 0.47 (3.6) | 0.13 |

Interpreter round 9 landed between the two rounds and made the interpreter as fast as the compiled code on most of
these (instructions per activation of `lfsr64`: interpreter 3600, compiled 3650). What this round bought against
main at its start (cycles, `perf stat`, cores 2 and 3, start-up included, `--tier=llvm`): `ff` -14%, `chain` -18%,
`lfsr64` -8%, `lfsr` -6%; the interpreter's `ff`, `lfsr`, `chain`, `fan` run 2 to 4% fewer instructions (the wait).
`--tier=auto` is level with the interpreter within noise on runs of 0.1 to 1 s (the compile and the 9 ms the first
use of the library costs are paid in CPU on the same cores; the wall time on a free second core is better), and
`compute_h2h` is 3.0 times faster under auto. The entry into compiled code is now about 15% of an `ff`
activation; the rest is the kernel (45%), the body (20%) and the drive and wait callbacks (20%), which a batch of
instances per call does not remove (decisions.md). Verified: difftest corpus (418) at 1 and 4 threads and 4 shuffled
threads on the interpreter, `--tier=llvm`, `--tier=auto` and `--tier=auto --tier-heat=1,0,0`; the same three tiers
on a `gc-stress` build; the external runner's pass lists (857 and 393) with `--oracle --check` on the interpreter and
`--tier=auto`; the verification library's `.pro` regression 2008 and 2019 and its suite (102 pass, 9 fail, 26 skip)
on both; `cargo test --workspace` (790) and `cargo test -p yavs-llvm --features llvm`.

## llvm: registers and frame variables across blocks, 2026-10-21 (#432)

decisions.md "Registers and frame variables across blocks". Instructions (`perf stat`, cpu_core, one performance
core, release build with the `llvm` feature; before and after):

| Design | `--tier=llvm` | `--tier=auto` | reference |
|---|---:|---:|---:|
| compute_h2h (20M iterations) | 3.14 G to 2.74 G (marginal 137 to 108 per iteration) | 6.1 G to 6.4 G (compile on the thread) | 2.29 G |
| compute_loop, calls, strings_heavy, ieee_heavy, access_heavy | run time equal, eager compile 3 to 17% more instructions | within 2% | |
| RTL group (rtl_ff ... lfsr64) | within 5% | equal | |

Verified: difftest corpus (420) on `--tier=llvm` at 1 and 4 threads and 4 shuffled, `--tier=auto` at 1 and 4 threads,
4 shuffled and with `--tier-heat=1,0,0`; stderr byte-identical between the interpreter and `--tier=llvm` on all 419
tests that write any; the external suites' pass lists (857 and 393) with `--oracle --check` on `--tier=auto` and
`--tier=llvm`; `cargo test -p yavs-llvm --features llvm` (19, with the new `frame_variables_match_the_interpreter`).

## llvm: compile cost (#432, #501, 2026-10-21)

Changes (`crates/yavs-llvm/AGENTS.md` "Compile cost"): `LEAN` pass pipeline except for compute loops, instruction scheduling and
machine-level motion passes off, unknown-space memory accesses through `yavs_rt_host`, `LCopy` inline only in processes,
exit blocks of at most 64 predecessors (a native frame's single exit phi had up to 800 incoming values: quadratic in the
simplifier), `OptLevel::Fast` (`LEAN` plus the unoptimised code generator) for functions of 300 IR instructions or more and
for `--tier=llvm` modules over 150000 LLVM instructions, native-frame code of an `auto` root only on demand, LLVM opened at
the first compile of `--tier=auto` instead of at its start.

Compile cost, `osvvm_axistream` library functions (every 8th function, one module each as `auto` compiles, 67 functions,
7208 IR instructions; user instructions of the whole process under `perf stat`, so independent of host load): 62.4 G before,
22.4 G after; LLVM instructions per IR instruction 26.8 to 19.4 (before the exit change), microseconds per IR instruction
(wall, loaded host) about 770 to 210. Where it went before: IR passes 43% and code generation 56%, the code generator 20 to 30
microseconds per optimised LLVM instruction up to 250 IR instructions per function, 41 at 300 to 500, 79 at 500 to 800, 155
above (superlinear: live-variable analysis over thousands of blocks); native-frame code cost 771 microseconds per IR
instruction, frame-mode code 230.

Eager `--tier=llvm` (one performance core, init phase = compile): `osvvm_axistream` 141 s to 6.8 s; `ieee/float_pkg_ops`
(#501) 80 to 92 s to 3.7 s; the difftest corpus (423 tests, 4 threads) 4 min 23 s to 42 s; the seven small designs of the
sum below 1.07 s to 0.41 s. Simulate phase of those seven designs (compute_h2h, calls x10, ieee_heavy x10, strings_heavy
x10, protected_scoreboard x10, rtl_lfsr, access_heavy x10), core 7, min of 3, two interleaved rounds: before 2.20 and 2.25 s,
after 2.24 and 2.23 s (interpreter 4.39 s); `compute_h2h` 0.13/0.15 to 0.16 s and `calls` 0.03 to 0.05 s are the two rows
that lost (eager mode has no inlining and no full pipeline in its one big module; `--tier=auto` keeps the full pipeline for
a compute loop).

`--tier=auto`, cores 6 and 7, median of 3 to 5: `uvvm_util_rand_tb` `rand_cyclic_performance` (the library testbench that
compiles something) 2.56 to 2.41 s (interpreter 2.92 s); the hot `match_element_data` (82 IR instructions, native-callable)
compiled in 259 ms before and 33 ms after; `uvvm_sb_slv_tb`, `osvvm_axistream`, `compute_h2h`, `calls`, `rtl_lfsr`,
`protected_scoreboard`, `strings_heavy` within noise (nothing to compile, or a compute loop that compiled in both). Fixed cost
of `--tier=auto` on a run that gets nothing hot (one-activation `rtl_fan`, core 7, user plus kernel cycles over
`--tier=interp`): 20 M plus 13 M before (ld.so relocations of the LLVM library), 2.5 M plus 1.6 M after.

## llvm: native frames, round two (#432, #337, #340, 2026-10-21)

Library functions on native frames (`docs/decisions.md` "Native frames, round two"): `CallNative` (pure natives, `writeline`,
file open and close; `stop`/`finish` materialise), `NewFrame`, `Fill`, `Cat` of non-logic elements, typed `TempRoot` and
`Stmt` with recorded roots, `CallProt`, nested subprograms (static links passed as parameters), `SubtypeCheck` and dynamic
`LayoutOf` from the descriptors, and through `TierHost::native_step` (the interpreter's own code on a transient frame) `New`,
`Free`, `LFill`, `LEq`, `Report`, `FToI`, `FToIRange`, `Phys`, `LNum`, `LogicOp`, `**`, logic `Cat`. Collections never run
while native frames exist (an instruction that can collect asks `Machine::reclaim_pending` first and materialises); the machine
reaches a native frame's slots by address through a per-thread table (`NativeSlots`), no copy.

Native-callable functions (`--tier=llvm --verbose`): `uvvm_sb_slv_tb` 21 of 288 before, 256 now; `uvvm_util_rand_tb` 43 of 440
before, 413 now (what is left: elaboration thunks, processes, `wait` in procedures, package allocation). Materialisations in
one eager run of `uvvm_sb_slv_tb`: 378 (before the table and the zero-copy windows 148 000 and 27 000).

Simulate phase of an eager `--tier=llvm` run (`--time-report`, one performance core, same host and core for all rows, compile
excluded), seconds:

| Testbench | interp | llvm before (main 45a44b62) | llvm now | sim1 (`-H 64m -e --no-save --jit -r`, startup included) |
|---|---:|---:|---:|---:|
| `uvvm_sb_slv_tb` | 0.88 | 0.61 | 0.41 | 0.74 |
| `uvvm_util_rand_tb` | 0.82 | 0.61 | 0.49 | 0.98 |

The eager compile of these two testbenches takes 150 s and 230 s (about 0.7 ms per IR instruction at O2, 23 LLVM instructions
per IR instruction before optimisation, 40 to 60 for a load or a descriptor read): not usable as a mode, and the reason
`--tier=auto` compiles nothing within its 2% budget in a run of a second. `--tier=auto` on the benchmarks, median of 5, two
cores, CPU seconds interp / auto before and after: `uvvm_sb_slv_tb` 0.85 / 0.89 and 0.88 / 0.91, `uvvm_util_rand_tb` 0.80 / 0.96
and 0.82 / 0.95, `osvvm_axistream` 0.29 / 0.31 both, `calls`, `strings_heavy`, `protected_scoreboard` within noise: no loss and no
gain (the extra 0.1 s of auto is the LLVM library and the compile thread's start, as before). A longer library testbench
(`rand_cyclic_performance`, 3.7 s) runs 2.7 s under auto both before and after: its hot function is a single small one the
earlier code already compiled. The auto tier now compiles a function that runs on native frames together with every function
it calls natively (native-frame code only for those) and publishes its native-frame code once all of it exists.

`tools/bench` head-to-head (`--ref sim1 --tiers interp,auto --instructions`, cores 5 and 6): `strings_heavy` yavs 0.88x wall of
sim1, `access_heavy` 0.50x, `calls` 1.5x, `protected_scoreboard` 1.0x.

Verification at the final commit: difftest 420/420 for `yavs-llvm` at 1 and 4 threads, shuffled at 4 threads, for `yavs-auto`
at 1 and 4 threads and with `--tier-heat=1,0,0` at both, `yavs` shuffled; stdout and stderr of all 420 identical between
`--tier=interp` and `--tier=llvm` (4 threads, shuffled); the `gc-stress` build gives 420/420 for llvm, auto and eager auto; the
external runner's suites `check`, `com`, `data_types`, `dictionary`, `logging`, `path`, `random`, `run`, `string_ops`,
`verification_components` pass under `--tier=auto`; the other verification library's `.pro` regression passes 314/314 under 2008
and 2019 under `--tier=auto` and its component suites give 102 / 9 / 26 as before. The `gc-stress` unit tests
`heap::tests::handles_and_collection`, `precise_roots_inside_an_activation` and `freed_slots_recycled_within_an_activation`
fail on main too (not looked at); `precise_roots_under_compiled_code` fails under `gc-stress` as well. All four asserted
slot counts, which the stress build changes (it never reuses a slot); fixed in #500 (they check live objects there) and
the unit tests run under `gc-stress` in the CI `llvm` job.

## llvm (`crates/yavs-llvm`)

LLVM tier (M6), behind the `llvm` feature (`cargo test -p yavs-llvm --features llvm`; `cargo build
-p yavs --features llvm` for `yavs run --tier=llvm`). `compile` lowers every non-elaboration IR
function to one LLVM function on the interpreter frame's registers, slots and temporary area:
register arithmetic, compares, checks, address arithmetic, branches, loads, stores, copies,
temporaries, descriptor reads and writes, static-element layouts and logic elements are native
(#334, #335; `LGet`/`LSet` of #338); other instructions go through a callback into the
interpreter or return to it (calls, waits, reports, every slow or error path).
Difftest corpus under `--tier=llvm` (`difftest.py --candidate yavs-llvm --threads 4`): 348/348 at
1 and at 4 threads, the same as the interpreter. Over the corpus programs (`--verbose`, static
counts) 993679 instructions are compiled inline and 39038 executed by the interpreter, 96.2%
native (60.8% before #334/#335): Call 7351, Ret 4974, CallNative 3649, SigVal 3336, LCopy 3078,
CopyConst 2517, Report 2473, Deref 2445, New 2155, ... (#336 to #341).

IR optimiser (#427, `crates/yavs-ir/src/opt.rs`, run by `ProgramBuilder::finish`/`snapshot`; `YAVS_NO_IROPT=1`
turns it off). `tools/bench/run.py --compare` at `--threads 1`, three runs, median CPU seconds, same release build
with the pass off (A) and on (B); instructions are the interpreter's dispatched instructions (preloaded constants
and skipped jumps not counted, fused pairs count once):

| Benchmark | Instructions A -> B | Total CPU A -> B | Simulate A -> B | Elaborate A -> B |
|---|---|---|---|---|
| compute_loop | 48.0M -> 44.0M | 0.42 -> 0.40 | 0.41 -> 0.39 | - |
| ff_rtl | 1.98M -> 1.33M | 0.06 -> 0.05 | 0.04 -> 0.04 | - |
| delta_chain | 10.3M -> 4.7M | 0.23 -> 0.20 | 0.22 -> 0.19 | - |
| protected_scoreboard | 24.9M -> 18.6M | 0.33 -> 0.18 | 0.32 -> 0.17 | - |
| resolved_bus | 22.3M -> 15.9M | 0.42 -> 0.35 | 0.41 -> 0.34 | - |
| textio_heavy | 13.6M -> 12.4M | 0.44 -> 0.43 | 0.43 -> 0.42 | - |
| vai_window_gen | 133.6M -> 97.4M | 2.41 -> 1.80 | 2.33 -> 1.68 | 0.04 -> 0.05 |
| uvvm_util_rand_tb | 672M -> 543M (earlier build) | 5.74 -> 4.82 | 5.66 -> 4.49 | 0.04 -> 0.07 |
| osvvm_axistream | - | 2.15 -> 1.73 | 2.04 -> 1.60 | 0.05 -> 0.07 |
| vunit_vc30 (30 runs) | - | 4.91 -> 4.74 | 1.62 -> 1.28 | 1.69 -> 1.91 |
| elab_heavy | - | 0.41 -> 0.47 | 0.10 -> 0.12 | 0.14 -> 0.15 |

Round 2 (#427, #425, #436): the optimiser runs per function the first time `Program::func` is asked for it
(the interpreter's predecoded form is built from that copy), a statically sized array variable reads its
descriptor from the constant pool, `NewFill` allocates an array object without a temporary, and a part of an
array-of-arrays signal is read alone (#436). `tools/bench/run.py --compare` of main (A) against this work (B), release
builds without the `llvm` feature, `--threads 1`, 7 runs, median CPU seconds (the first four rows are a second run of
the same pair):

| Benchmark | Total A -> B | Elaborate A -> B | Simulate A -> B |
|---|---|---|---|
| vunit_vc30 (30 runs) | 3.19 -> 3.23 | 1.32 -> 1.14 | 0.64 -> 0.79 |
| uvvm_util_rand_tb | 2.62 -> 2.65 | 0.05 -> 0.03 | 2.53 -> 2.58 |
| osvvm_axistream | 0.97 -> 0.95 | 0.05 -> 0.03 | 0.87 -> 0.87 |
| compute_loop | 0.18 -> 0.18 | - | 0.17 -> 0.17 |
| elab_heavy | 0.34 -> 0.34 | 0.11 -> 0.11 | 0.08 -> 0.08 |
| ff_rtl | 0.03 -> 0.03 | - | 0.02 -> 0.02 |
| delta_chain | 0.16 -> 0.15 | - | 0.15 -> 0.14 |
| protected_scoreboard | 0.11 -> 0.11 | - | 0.10 -> 0.10 |
| vai_window_gen | 0.92 -> 0.92 | 0.04 -> 0.03 | 0.82 -> 0.84 |
| 512-register array-signal design (#436), 5000 cycles | 11.07 -> 4.01 | - | - |

Geometric mean of the total CPU ratio over the nine benchmarks 0.98. Lazy optimisation moves the pass from
elaborate to the first call in simulate and does not shrink it: in a library testbench nearly every lowered
function is called (542 of 904 functions, 61% of the instructions, in a verification component test; lowering is
already demand driven), so a short run still pays about 6 ms of pass for 108k instructions. A function's version is
fixed at its first use because frames find their function by id; optimising only after a call count would need the
version in the frame (an interpreter change). Left: range-based check removal, loop-invariant hoisting, promoting
hidden frame slots to registers.

Performance (M6, #334/#335, 2026-10-19, `tools/bench/run.py --compare` of one release build run
with `--tier=interp` (A) and `--tier=llvm` (B), median CPU of 3 runs, shared host). The LLVM
compile of every function is in `init`; simulation alone:

| Benchmark | interp simulate | llvm simulate | speedup | llvm compile (init) |
|---|---|---|---|---|
| `compute_loop` | 0.80 s | 0.16 s | 5.0x | 0.09 s |
| `ff_rtl` | 1.25 s | 0.48 s | 2.6x | 1.07 s |
| `uvvm_axistream_vvc_tb` (to 10 us) | 6.31 s | 1.25 s | 5.0x | 114.9 s |
| `vai_pe_array` | 6.53 s | 2.57 s | 2.5x | 36.0 s |
| `vai_window_gen` | 3.30 s | 1.60 s | 2.1x | 39.7 s |

Compiling everything before time 0 costs far more than it saves on these runs (total CPU 3.4x
worse, geometric mean): tier-up of hot functions only is #125. The native memory paths made the
compile about a third slower (`uvvm_axistream_vvc_tb`: about 86 s before, 115 s after). What
still leaves compiled code on that testbench, per 10 us: 0.9 M entries into compiled code after a
call or return (#336), 1.5 M stepped instructions (`LCopy`, `LayoutOf` of records and nested
arrays, `Deref`, `New`, `SigSlice`, `CopyConst`, signal reads), 1.5 M object lookups.

Tier-up (`--tier=auto`, #125, #432; `docs/decisions.md` "LLVM tier: side exits"): the compile
thread samples every millisecond which function each simulation thread interprets (the fast loop
stores it in a per-thread slot when it enters a function without compiled code; time in compiled
code entered from interpreted code counts for the caller) and compiles a function once it was
seen for 2 ms plus 40 us per IR instruction (about twice its O0 compile time; the hidden
`--tier-heat=BASE,PER_INSTR,PERIOD` sets it, PERIOD 0 compiles every function the interpreter
enters). One background compile thread (O0, one module per function, the JIT made with the first
hot function); the interpreter swaps compiled code in on entry and within 256 jumps; the run never
waits for compilation. Functions over `--tier-max-size` (default 50000 IR instructions, user,
2026-10-19, #126) stay interpreted in both compiling tiers. The per-op counting this replaces
cost 12% to 25% of CPU on `calls` and `compute_loop` with nothing compiled.

Self-contained compiled code (#336, #337, user decision 2026-10-20): compiled code runs every
instruction it does not lower in place through `yavs_rt_step` (calls, natives, foreign calls,
reports, signal operations, waits) instead of returning to the interpreter; a call whose callee
has compiled code runs it directly on the frame the interpreter pushed and pops it on return
(`TierHost::ret`). It returns only at returns, suspensions, failing inline instructions and
frames it cannot finish (a callee without compiled code, more than 256 KiB of nested native
stack: `TIER_UNWIND`). `TierFrame` is a view of the machine's stacks, refreshed after steps,
instead of owned vectors swapped out and in at every step. Also inline now: `AddrOf` one and two
static links up, `SigVal`, frame accesses without a check where they lie within the frame size,
known constants as immediates, one exit block per function; the address lookup of other memory
is a Rust function (`yavs_rt_host`) instead of two generated functions per module. Output is
byte-identical: difftest 385/385 for `--candidate yavs-llvm` and `yavs-auto` with every function
compiled from its first entry (`--tier-heat=1,0,0`) at 1 and 4 threads; before the last rebase
362/362 for `yavs-auto` at the default heat, eager and shuffled (seed 7) and at a 20 us sampling
period shuffled (seed 3); a `gc-stress` build passes `yavs-llvm` and eager `yavs-auto` except
the two `synopsys/` tests that exceed the harness timeout under it.

Compile cost at O0 after these changes (release, `uvvm_util_rand_tb` with every function
compiled, 60 functions): about 20 us per IR instruction and well under 1 ms for small functions
(13 functions of about 30 IR instructions: 0.4 ms each; about 380: 6.8 ms; about 1440: 29 ms),
down from 2 ms plus 40 us. O2 costs about 7x O0 (a 188-instruction process: 73 ms against 9 ms),
O2 IR passes before O0 codegen gain little at run time, so O0 stays.

Measurements (2026-10-20, release build with the `llvm` feature, `--threads 1`, shared 24-core
host, `tools/bench/run.py --compare` of one binary with `--tier=interp` (A) and `--tier=auto` (B),
median of 3 runs, total CPU seconds, compile thread included):

| Benchmark | interp | auto | auto / interp | peak RSS MiB interp / auto |
|---|---:|---:|---:|---:|
| `compute_loop` | 0.18 | 0.15 | 0.82 | 59 / 69 |
| `ff_rtl` | 0.04 | 0.04 | 1.10 | 61 / 69 |
| `resolved_bus` | 0.26 | 0.27 | 1.04 | 60 / 70 |
| `delta_chain` | 0.16 | 0.17 | 1.05 | 59 / 69 |
| `textio_heavy` | 0.33 | 0.35 | 1.07 | 59 / 70 |
| `protected_scoreboard` | 0.12 | 0.13 | 1.06 | 59 / 69 |
| `calls` | 0.17 | 0.18 | 1.07 | 59 / 69 |
| `strings_heavy` | 0.28 | 0.25 | 0.89 | 60 / 73 |
| `access_heavy` | 0.14 | 0.13 | 0.93 | 59 / 71 |
| `report_heavy` | 0.27 | 0.26 | 0.95 | 59 / 71 |
| `file_io` | 0.25 | 0.26 | 1.05 | 59 / 69 |
| `elab_heavy` | 0.36 | 0.38 | 1.05 | 213 / 213 |
| `vunit_vc30` (30 runs) | 3.76 | 3.78 | 1.00 | 116 / 116 |
| `uvvm_axistream_vvc_tb` (to 10 us) | 2.69 | 2.07 | 0.77 | 140 / 165 |
| `uvvm_util_rand_tb` | 2.93 | 2.98 | 1.02 | 93 / 117 |
| `osvvm_axistream` | 1.15 | 1.25 | 1.09 | 98 / 113 |
| `vai_pe_array` | 0.16 | 0.18 | 1.11 | 95 / 95 |
| `vai_window_gen` | 0.84 | 1.14 | 1.36 | 96 / 111 |
| geometric mean | | | 1.016 | |

The kernel RTL designs (median simulate CPU of 3 interleaved runs, interp / auto): `ff` 5.53 /
5.98 s (1.08), `lfsr` 4.61 / 2.98 s (0.65), `chain` 1.40 / 1.56 s (1.11), `fan` 4.60 / 5.45 s
(1.18). Simulate phase with everything compiled at O2 before time 0 (`--tier=llvm`, compile time
excluded) against the interpreter: `compute_loop` 0.11 / 0.19 s, `strings_heavy` 0.16 / 0.29 s,
`access_heavy` 0.09 / 0.15 s, `uvvm_util_rand_tb` 2.08 / 3.52 s, `calls` 0.18 / 0.18 s. The
interpreter alone (`--tier=interp`) is unchanged by this work (geometric mean 0.97 of the same
binary before and after, within noise).

Where `--tier=auto` still loses: compiled code at O0 runs a lowered instruction little faster than
the predecoded interpreter, and an instruction it steps costs the callback (about the cost of an
interpreted step plus a few ns); short process activations pay the entry into compiled code each
time (the RTL designs with thousands of one-statement processes), and in `vai_window_gen` the
sampled functions run no faster compiled while their compilation adds CPU. Next: an O2 recompile
of the hottest functions, inline signal reads and edges (#339), `Deref` (#340), and natives
without the generic step (#337).

Clocked RTL (#339, #432, 2026-10-20): compiled code runs `SigReadScalar`, `Drive`, `WaitOn`
(with its `Suspend`), `Suspend` and the `rising_edge`/`falling_edge` natives as direct calls of the
interpreter's functions (`TierHost::signal`), the scalar `std_ulogic` natives as tables, and frame,
temporary and instance addresses inline; the entry fills its frame view in place and two
store-forwarding stalls of the entry are gone; `--tier=auto` compiles functions still hot in their
compiled code again at O2 and leaves short loop-free functions interpreted (docs/decisions.md "LLVM
tier"). Kernel designs, default generics, `--threads 1`, total CPU seconds, median of 5 interleaved
runs on a loaded host (load 5 to 6; ratios vary by about 0.05 between runs), `--tier=auto` of main
before this work (`d75ef11d`) for comparison:

| Design | interp | auto | auto before | sim1 | auto / interp | auto before / interp | auto / sim1 |
|---|---:|---:|---:|---:|---:|---:|---:|
| ff: 4096 registers x 2000 | 2.91 | 2.39 | 3.87 | 0.51 | 0.82 | 1.41 | 4.7x |
| lfsr: 512 x 16-bit x 5000 | 1.58 | 1.54 | 2.13 | 0.19 | 0.98 | 1.21 | 8.1x |
| chain: 1000 stages x 2000 | 0.92 | 0.94 | 1.05 | 0.15 | 1.02 | 1.15 | 6.3x |
| fan: 4000 x 5000 | 2.71 | 2.90 | 3.61 | 0.91 | 1.07 | 1.33 | 3.2x |
| bus: 32 drivers x 20000 | 0.49 | 0.52 | 0.54 | 0.56 | 1.08 | 1.11 | 0.9x |

(ff and lfsr from a second run of the same binaries; the first gave ff 0.98 and lfsr 1.03 at a
higher load, an earlier one 0.83 and 0.91.) Where the time of these designs goes now: the kernel
(update phase, wake filter, waiter lists) is over half; compiled process code at O2 is 13% of `ff`;
the per-activation entry (view, entry switch) and the callbacks are what keeps `fan` and `chain` at
the interpreter's speed. The shared benchmarks (`tools/bench/run.py --compare`, the same binary with
`--tier=interp` (A) and `--tier=auto` (B), 3 runs, total CPU): geometric mean 1.153 over 18;
`--tier=auto` against the same with main's tier: 1.015 before the short-function rule, which then
gave 0.94 on the seven call-heavy ones. The interpreter got much faster since the table below
(`uvvm_util_rand_tb` 1.13 s, `calls` 0.10 s): `access_heavy` 1.46, `uvvm_util_rand_tb` 1.42,
`protected_scoreboard` 1.30, `compute_loop` 1.26, `vai_window_gen` 1.25, `calls` 1.24, the
other twelve 0.98 to 1.16. So `--tier=interp` stays the
default: compiled code at O0 is no faster than the predecoded interpreter on testbench code, and
an entry from interpreted code costs about what a short body costs. Difftest 409/409 for
`yavs-llvm` and `yavs-auto` with `--tier-heat=1,0,0` (O0 and O2 swapped in mid-run) at 1 and 4
threads; a `gc-stress` build 407/409 for both, the two `synopsys/` tests over the harness timeout.

Tier-up within a budget (#432, 2026-10-21): `--tier=auto` was 1.23x the interpreter's CPU on the
harness (geometric mean of 16, worst `protected_scoreboard` 1.52, `calls` 1.56, `access_heavy` 1.57) because
it compiled every function seen for a few milliseconds, at O0 first, and a hot set of testbench
functions gains under 10% even compiled before time 0 (22 library functions: 1.56 s to 1.44 s of
simulation for 6 s of compile). Now: (1) the fast loop with a tier set but nothing compiled
samples every 128 events and costs about 1% (it was the full loop, 10% on `calls`); (2) compiled
calls no longer clone the runtime (a compiled call 0.73 s to 0.48 s per 3 million, now level with the
interpreter's 0.52 s); (3) compile time halved (addresses other than frame offsets by one call,
`LayoutOf`, `Alloc`, `DopeLen`, `SubtypeCheck` by helpers; the O2 compile of a 185-instruction
loop 98 ms to 39 ms); (4) O2 only, a function after four times its estimated compile time, with
its callees, and only while compile time is at most 2% (4% for compute loops) of the interpreted
time sampled. Pinned to performance cores 2 and 3 (`taskset -c 2,3`, so the compile thread has its own),
`tools/bench/run.py --compare` of one binary `--tier=interp` against `--tier=auto`, total CPU, median of 5:
geometric mean 1.006, `compute_loop` 1.02, `ff_rtl` 1.03, `calls` 1.03, `protected_scoreboard` 1.00,
`strings_heavy` 1.01, `uvvm_util_rand_tb` 1.02, `osvvm_axistream` 1.01, `uvvm_axistream_vvc_tb` 0.98,
`vunit_vc30` 1.02, the rest 0.95 to 1.02 (the 0.04 s runs are noise of a few percent). Kernel designs
(minimum of 5, same pinning): `ff` 1.02, `lfsr` 1.01, `chain` 1.02, `fan` 1.02, `bus` 1.04, `ram` 1.00, `rtl`
0.96, `rtl2` 0.97. Long runs (3 runs): `compute_loop` N=20M 0.95, `strings_heavy` N=400k 0.95,
`calls` N=15M 0.97, `protected_scoreboard` N=3M 0.97, `access_heavy` N=2M 0.99. Output is
byte-identical: difftest 410/410 at 1 and 4 threads for `yavs-auto` (default, `--tier-heat=1,0,0`,
`--tier-heat=1,0,20` shuffled) and `yavs-llvm`, on a normal and a `gc-stress` build. Not done:
per-function feedback that takes compiled code back (the budget and static rules stand in for it), a
direct call from the interpreter's `Call` to a compiled callee, side exits of `TempRoot`, `NewFrame`,
`CallNative` (they are 2.5, 2.0 and 1.8 million of the 9 million steps of `uvvm_util_rand_tb`).

Compiled frames and direct calls (#432, 2026-10-21; `docs/decisions.md` "Compiled frames and calls between
compiled functions"): a function that runs without a step on its fast path and calls only such functions
(`native_set`) also gets native-frame code, which compiled callers call directly (registers and slots on the
native stack, one module: by symbol, so LLVM may inline it; `--tier=auto`: through a table); the frame becomes an
interpreter frame only where the interpreter needs one (a step, a failing instruction, the depth limit, the native
stack allowance), and the interpreter goes on there. Frame-mode code is also entered after the preloaded
definitions the interpreter skips (before, a compiled function called from the interpreter ran interpreted up to
its first jump). `Free`, `Fill` and `CallProt` have direct arms in `TierHost::step`. Pinned (`taskset -c 3`),
`calls` at 30 million calls (`-gN=7500000`): `--tier=llvm` 22.37 G to 1.63 G instructions, 0.94 s to 0.09 s
(interpreter 23.5 G, 0.95 s); `--tier=auto` 23.86 G to 15.73 G instructions, 0.94 s to 0.51 s (the process is
compiled once its native-frame callees' time counts for it). On the library testbenches the effect is nil so far:
`generic_sb_slv_tb` under `--tier=llvm` has 7 native-callable functions (its calls go to functions with
`Cat`, `CallProt`, `CallNative`, `Deref`, typed `TempRoot`, dynamic `LayoutOf` or `NewFrame` on the fast path),
simulation 0.91 s to 0.95 s, compile 9.1 s to 9.8 s (the native-frame copies); `--tier=auto` and
`--tier=interp` within noise on the shared benchmarks (geometric means 1.05 and 1.006 at 3 to 5 runs on a loaded
host; instruction counts of `generic_sb_slv_tb` under `--tier=auto` equal, 24.6 G both). Output identical:
difftest 414/414 for `yavs-llvm` and `yavs-auto` (default, `--tier-heat=1,0,0`, shuffled seed 7) at 1 and 4
threads, the stderr of every corpus test byte-identical between `--tier=interp` and `--tier=llvm` (26 with
runtime errors), and a `gc-stress` build 414/414 for `yavs-llvm` and eager `yavs-auto`. Effect attributes on the runtime helpers
(`docs/decisions.md` "Compiled frames", effects): no measurable change pinned on `compute_loop` and the kernel
designs under `--tier=llvm` (within 1% in instructions and cycles) nor on `rand_tb` (simulation 0.85 to 0.98 s
in both builds, interpreter 1.00 s; its O2 compile 31 s before the native-frame copies, 33 to 35 s after).
What keeps the library testbenches from native frames, over the 260 functions of `generic_sb_slv_tb`: 199 have
several kinds of stepped instruction, the most common `Deref` (164), `CallProt` (150), `CallNative` (107), `Cat`
(97), `NewFrame` (89); 31 have none, of which 7 call only such functions. With `Deref` and `MemEq` lowered through frame-free helpers
(next commit) 21 are native-callable, and its simulation under `--tier=llvm` takes 1.00 to 1.01 s against 1.03 to
1.12 s before and 1.30 s interpreted (pinned core 3, two runs each). Verification of these commits: difftest 414/414
for `yavs` (4 threads, shuffled), `yavs-llvm` and `yavs-auto` (default, `--tier-heat=1,0,0` shuffled) at 1 and 4
threads, `gc-stress` 414/414 for `yavs-llvm` and eager `yavs-auto`; the external runner's own eleven suites pass
under `--tier=auto` (1344 tests); the `.pro` regression 314/314 under 2008 and 2019; the verification-library
suite 102 pass, 9 fail, 26 skip (as before).

Tiers on the final binary (f1787925, pinned): `tools/bench/run.py` with 2 performance cores (`taskset -c 2,3`),
median total CPU of 5 runs (3 for `--tier=llvm`), interp / auto / llvm (llvm: compile in seconds plus simulate):
`uvvm_util_rand_tb` 0.91 / 1.00 / 33.3 + 0.84, `uvvm_sb_slv_tb` 0.97 / 1.10 / 9.9 + 0.87, `osvvm_axistream`
0.33 / 0.37 / 26.3 + 0.38, `protected_scoreboard` 0.04 / 0.05 / 0.13, `calls` 0.09 / 0.10 / 0.05 (simulate 0.08 /
0.09 / 0.01), `access_heavy` 0.05 / 0.06 / 0.11, `strings_heavy` 0.09 / 0.10 / 0.19 (simulate 0.08 / 0.09 /
0.05), `compute_loop` 0.10 / 0.11 / 0.05, `vunit_vc30` 1.55 / 1.82 / not run. The harness's auto/interp ratio
is about 1.1 on this loaded host while `perf stat` of the same pair counts no difference (`generic_sb_slv_tb`:
19.95 G against 19.29 G instructions, 5.23 G against 5.00 G cycles). Kernel designs, cycles (core 3, minimum of 3),
interp / auto / llvm in G: `ff` 4.51 / 4.51 / 5.15, `lfsr` 2.32 / 2.33 / 2.70, `chain` 1.16 / 1.19 / 1.78, `fan`
5.09 / 5.19 / 7.61, `bus` 1.04 / 1.08 / 1.41, `ram` 0.35 / 0.39 / 1.14, `compute_loop` 0.43 / 0.46 / 0.24; auto
over interp before this work: `chain` 1.03, `fan` 1.02, `bus` 1.04, `ram` 1.07, the same. The external runner's
suites also pass under `--tier=llvm` (1344 tests, the build before the `Deref` commit) and under `--tier=auto`
on the final binary.

The measurements before this work, for reference:

Compile cost at O0 (release, per function, compile thread): about 2 ms plus 40 us per IR
instruction, nearly all of it codegen (37 IR instructions 3.6 ms, 1323 61 ms, 4169 243 ms, 20755
1.15 s). Each IR instruction becomes about 12.6 LLVM instructions. O1 codegen costs about 10x
O0; IR passes before O0 codegen (instcombine, early CSE, CFG simplification) shrink neither the
codegen time nor the machine code, so O0 stays the first tier (#432). A side exit (return to the
interpreter and re-entry) costs about 15 ns: the text I/O loop of `textio_heavy` (3.7
interpreted instructions per off-fast-path instruction) runs 0.43 s interpreted and 0.51 s with
`--tier=llvm`, which is why off-fast-path instructions count against tier-up. The first version
(100 calls and loop back-edges plus 300 per IR instruction) missed the loops the dispatch loop
threads (#412) and compiled such functions: up to 1.22x the interpreter's CPU
(`osvvm_axistream`, at a flat 3000 per IR instruction).

Measurements (2026-10-20, after the predecoded interpreter loop of #431, which tier-up now
counts in: release build with the `llvm` feature, `--threads 1`, shared 24-core host,
`tools/bench/run.py --compare`, median of 3 runs, total CPU seconds, compile thread included;
A `--tier=interp`, B `--tier=auto`):

| Benchmark | interp | auto | auto / interp | peak RSS MiB interp / auto |
|---|---:|---:|---:|---:|
| `compute_loop` | 0.20 | 0.27 | 1.34 | 58 / 70 |
| `ff_rtl` | 0.04 | 0.05 | 1.04 | 61 / 61 |
| `resolved_bus` | 0.24 | 0.24 | 1.03 | 60 / 59 |
| `delta_chain` | 0.16 | 0.16 | 1.05 | 59 / 59 |
| `textio_heavy` | 0.30 | 0.31 | 1.04 | 59 / 59 |
| `protected_scoreboard` | 0.12 | 0.13 | 1.07 | 59 / 59 |
| `strings_heavy` | 0.25 | 0.25 | 1.02 | 60 / 71 |
| `access_heavy` | 0.12 | 0.14 | 1.09 | 59 / 59 |
| `report_heavy` | 0.27 | 0.28 | 1.05 | 59 / 59 |
| `file_io` | 0.23 | 0.24 | 1.06 | 59 / 59 |
| `elab_heavy` | 0.34 | 0.35 | 1.02 | 212 / 212 |
| `vunit_vc30` (30 runs) | 3.70 | 3.75 | 1.01 | 116 / 116 |
| `uvvm_axistream_vvc_tb` (to 10 us) | 2.76 | 2.57 | 0.93 | 137 / 147 |
| `uvvm_util_rand_tb` | 2.93 | 2.79 | 0.95 | 98 / 104 |
| `osvvm_axistream` | 0.98 | 1.04 | 1.05 | 99 / 99 |
| `vai_pe_array` | 0.16 | 0.17 | 1.02 | 94 / 95 |
| `vai_window_gen` | 0.89 | 0.92 | 1.03 | 96 / 96 |
| geometric mean | | | 1.045 | |

The predecoded interpreter (#431) runs these 1.1x to 2x faster than the loop the measurements
before it used, and now outruns compiled code wherever that leaves to the interpreter often:
`compute_loop` simulates in 0.18 s interpreted, 0.24 s with everything compiled at O2
(`--tier=llvm`) and 0.29 s compiled at O0 from the first call; `protected_scoreboard` 0.11 s,
0.17 s and 0.17 s; only `strings_heavy` gains (0.23 s, 0.13 s at O2). Counting alone (nothing
compiled) costs 2% to 9%. So `--tier=interp` is the default again (docs/decisions.md "LLVM
tier"); tier-up pays once compiled code leaves less often (#336, #337) and costs less (#432).
Before #431 (old dispatch loop, same tier-up) `--tier=auto` was at most 1.01x and down to 0.89x
the interpreter's CPU (geometric mean 0.97). Every process of a yavs built with the feature
also pays about 11 ms of CPU and 47 MiB of RSS for loading the shared LLVM library, whatever the
tier (#432).

Spike (before M6), done: llvm-sys 211 bindings (docs/decisions.md), ORC LLJIT with a portable CPU, O2 through
`LLVMRunPasses`, Rust runtime callbacks as absolute symbols, a checked i64 sum that traps into
Rust on overflow, a process body reading and writing a `#[repr(C)]` struct, a 10k-instruction
function checked against its Rust reference semantics, an unresolved runtime symbol reported by
name, and background compilation on one thread with an atomic function-pointer swap while the
calling thread keeps running in the "interpreter". Windows: `crates/yavs-llvm/WINDOWS.md`
(CI-verified: `cargo test -p yavs-llvm -p yavs --features llvm` passes on `windows-latest`, `ubuntu-24.04-arm` and `ubuntu-24.04`).

JIT cost of one generated function (`examples/measure.rs`: `state[w] := state[a] + (state[b] and
255)` with an overflow check, 13 IR instructions and 2 basic blocks per statement). One process
per row, release build, Intel Core Ultra 9 275HX, LLVM 21.1.8, x86-64-v2. Time is IR build +
passes + codegen and link (first lookup); memory is peak RSS growth over a process that already
created the JIT.

| IR instructions | opt | passes (ms) | codegen + link (ms) | total (ms) | peak RSS growth (MiB) |
|---:|:--:|---:|---:|---:|---:|
| 1,002 | O2 | 6 | 12 | 18 | 12 |
| 10,011 | O2 | 50 | 95 | 146 | 28 |
| 20,021 | O2 | 103 | 225 | 330 | 46 |
| 40,041 | O2 | 206 | 589 | 799 | 79 |
| 100,101 | O2 | 630 | 3,100 | 3,800 | 231 |
| 299,001 | O2 | 3,761 | 24,489 | 28,280 | 650 |
| 10,011 | O1 | 30 | 92 | 123 | 25 |
| 100,101 | O1 | 281 | 3,009 | 3,301 | 199 |
| 1,002 | O0 | 0 | 3 | 3 | 8 |
| 10,011 | O0 | 0 | 21 | 22 | 15 |
| 100,101 | O0 | 2 | 181 | 193 | 92 |
| 299,001 | O0 | 6 | 637 | 674 | 264 |

Codegen at O1/O2 grows faster than linearly in the size of one function (10x the instructions
costs 25x to 30x the time); O0 stays close to linear. Proposed threshold: O2 up to 40k
instructions per function, larger units stay interpreted (docs/decisions.md).
## clift (`crates/yavs-clift`)

Cranelift tier (#481 item 1), behind the `clift` feature (`cargo test -p yavs-clift --features clift`;
`cargo build --release -p yavs --features clift` for `yavs run --tier=clift` and `--tier=clift-auto`; both
features for `--tier=auto3`). It needs no LLVM install. It generates code for the contract the LLVM tier defined and
shares through `yavs_llvm::rt` (the frame view `Mem`, the callbacks `yavs_rt_step`, `yavs_rt_signal`, `yavs_rt_host`,
`enter`, the per-instruction facts): inline are the integer and real operations with their checks, compares, branches,
`Case`, loads, stores, `Copy`, `Alloc`, `AddrOf`, `Index`, `Stmt`, descriptor reads and writes, `LayoutOf` of static
layouts, logic elements and logic copies (`LCopy`), `SigVal`, the `std_ulogic` table natives and the signal operations
that use `yavs_rt_signal`; everything else (calls, natives, reports, `**`, heap) runs in place through `yavs_rt_step`,
and a failing inline instruction returns its pc to the interpreter, so the output is the interpreter's. It uses frame
mode only: registers stay in the frame's register array (no native frames, no deferred register stores), so it works
with the interpreter's frames unchanged; it has entry points at the pcs after preloaded constants and addresses like
the LLVM tier (without them a tier-up could not enter a running loop). `--tier=clift-auto` translates a hot function
with the small functions it calls (as `Worker::tree`). Cranelift runs with `opt_level=speed` and backtracking
register allocation (`YAVS_CLIFT_OPT`, `YAVS_CLIFT_RA`, `YAVS_CLIFT_VERIFY` are measurement knobs).

`--tier=auto3` is the LLVM `AutoTier` with Cranelift as its first level (`Generator` in `yavs_llvm::generator`): the budget,
payback and sampling rules are the LLVM tier's, the first-level cost is learned from the Cranelift translations
(about 45 us per IR instruction to start with), a function that stays hot (30 times its threshold running translated
code) is compiled again by LLVM at O2, and LLVM is opened only then.

Correctness (merged head, release build with `llvm,clift`): `difftest.py --candidate yavs-clift`, `yavs-clift-auto`,
`yavs-auto3` and `yavs-auto3-eager` (`--tier-heat=0,0,0`) pass 420/420 at 1 and 4 threads; the collection stress build
(`--features gc-stress,llvm,clift`) passes 420/420 at 1 thread for `yavs-clift`, `yavs-clift-auto` and
`yavs-auto3-eager`; the 13 tier tests of `yavs-llvm` pass unchanged against this tier.

Measurements (2026-10-21, release build of the merged head, every timed run pinned to performance core 5, median of
5, CPU seconds including the background compile thread, host load 8 to 35 from other agents; the harness is
`tools/bench/run.py --ref ... --instructions`). Kernel-bound designs (interpreter / `auto` / `auto3` /
`clift-auto` / sim1 / sim2): rtl_ff 0.76 / 0.74 / 0.76 / 1.06 / 0.56 / 0.43, rtl_lfsr 0.41 / 0.41 / 0.46 / 0.51 / 0.13 /
0.48, rtl_chain 0.21 / 0.21 / 0.21 / 0.26 / 0.14 / 0.81, rtl_fan 1.12 / 1.13 / 1.13 / 1.09 / 0.94 / 0.30, rtl_bus 0.20 /
0.21 / 0.20 / 0.23 / 0.52 / 0.18, lfsr64 0.21 / 0.22 / 0.20 / 0.26 / 0.09 / 0.32, xor_types 0.90 / 0.98 / 1.16 / 1.22 /
0.84 / 1.55; the compute loop `compute_h2h` 1.96 / 0.37 / 0.63 / 0.57 / 2.42 / 0.22. Library testbenches (interpreter /
`auto` / `auto3` / `clift-auto`): uvvm_util_rand_tb 0.94 / 1.18 / 1.02 / 1.15, uvvm_sb_slv_tb 1.01 / 1.12 / 1.15 /
1.37, osvvm_axistream 0.37 / 0.37 / 0.38 / 0.39, vunit_vc30 1.69 / 2.01 / 1.70 / 1.73; geometric mean of the ratio
to the interpreter 1.14 (`auto`), 1.07 (`auto3`), 1.16 (`clift-auto`), 1.15 (a build without the `llvm` feature).
Single-pass register allocation: 1.16 (`auto3`) on the testbenches and no gain on the kernel designs.

Translation cost: Cranelift (`speed`, backtracking) translates 92958 IR instructions of the utility library's random
testbench in 3.9 s (42 us per IR instruction; mean 1.0 ms for functions under 64 instructions, 3.8 ms for 64 to
256, 13.9 ms for 256 to 1024, 546 ms for the four over 1024); LLVM O2 needs about 1 ms per IR instruction on the same
code (95 s for the verification library's `func_cov_tb`, 300 to 400 us per instruction on a 225-instruction loop).
Release build (`-p yavs`, clean target, cores 8 to 23, 2 jobs): default features 110 s wall, 209 s user, 0.85 GB peak
compiler memory, stripped binary 15.2 MB; `clift` 167 s, 319 s, 1.96 GB, 20.5 MB (+57 s, +5.2 MB); `llvm` 109 s,
15.8 MB; `llvm,clift` 160 s, 20.98 MB. Start-up of a tiny test (`arith/div_zero`, median of 40, one core): 2.2 ms
interpreter, 2.4 ms `clift-auto` and `auto3`, 10 ms `auto` (opening the LLVM library at start).

What the numbers say: in every compiled configuration the testbench sets and the kernel designs are within noise of
the interpreter or up to 20% to 35% slower; compiled code wins only where a function holds a loop without calls (the
compute loop: 5x with `auto`, 3.5x with Cranelift). Cranelift alone matches LLVM on that loop only after its code
runs (`clift` eager 0.5 s, LLVM O2 0.37 s). On clocked RTL it executes more instructions than the interpreter
(rtl_ff 26.5 G against 16 G): the entry and exit of compiled code and the callbacks cost more than the interpreter's
predecoded dispatch.

### clift: code quality, rounds 1 and 2 (2026-10-24, #481)

Registers of a native-frame variant in Cranelift variables, closed frames with slot variables, held objects across blocks,
`Index`, `Pos`, `Field` without overflow arithmetic on constants; then `SlicePos`, `LFill`, `LogicOp`, `NewFrame`, `Fill`,
a dynamic `Cat` of one block, the `numeric_std` unsigned `+`, `-` and `to_integer` on their small path, reads of constants
(a copy in the code), the temporary area, instance data and enclosing frames without a lookup, zeroing of temporaries by
stores, and the interpreter's own `xstep` as the slow path of those in a variant (decisions "Cranelift code quality").
Instructions of the whole process in G (compile thread included), one run each at the commit named, hosts shared, so
`auto` rows move by several percent from run to run; the interpreter (main of 2026-10-24 evening) is 450.0 / 37.5 / 56.4 / 188.0:

| design | clift-auto | auto3 | LLVM auto cold / warm |
|---|---:|---:|---:|
| window generator (`vai_window_gen_interlock`) | 195.9 | 204.1 | 235.5 / 188.4 |
| neorv32_single_min | 38.4 | 38.8 | 36.4 / 36.4 |
| fletcher_bufferwriter_512 | 57.5 | 57.7 | 56.9 / 56.9 |
| poc_arith_addw | 148.1 | 145.4 | 152.7 / 115.0 |

Where it stands: on the testbenches that run library code and numeric_std (window generator, poc) Cranelift is 56 and 21
percent below the interpreter and level with or below LLVM `auto` cold. On neorv32 and fletcher it does not beat the
interpreter (2 percent above; LLVM is 3 percent below on neorv32 and level on fletcher): the kernel is 35 to 47 percent of the instructions and the compiled clocked processes
cost what the interpreted ones do (eager Cranelift runs neorv32 in 39.3 G against 38.8 G interpreted, plus 51 G of
translation of all 1900 functions; with `YAVS_CLIFT_OPT=none YAVS_CLIFT_RA=single_pass` the translation is 27 G). Tools:
`YAVS_CLIFT_PERFMAP=1` with `perf record -e instructions:u` and `perf report` names the generated functions;
`YAVS_COUNTERS=pc` counts `xstep` and `native` entries by instruction and symbol.

### clift: native frames and inlining (2026-10-21)

Question (#481): do direct calls between compiled functions, and Cranelift's inliner on top of them, make Cranelift code
fast enough to matter. What was built (`crates/yavs-clift/src/lower.rs`, knobs `YAVS_CLIFT_NATIVE`, `YAVS_CLIFT_INLINE`):
a function of at most 400 IR instructions, 128 registers and 1 KiB of slots, without a static link, every instruction
of which the tier lowers inline, gets a second body `n<id>` with its registers and slots on the native stack and a
`(status, value)` result. Callers (frame-mode functions and other native bodies) call it directly: no interpreter frame,
no `tier_view`, no lookup. A callee that cannot finish (failing check, slow path, `Unreachable`) records its frame
(`yavs_rt_deopt`), each caller on the way records itself, and the frame-mode root has the interpreter push them all
(`TierHost::materialise`), so errors, call chains and the rest of the run are the interpreter's. Cranelift 0.137's
`Context::inline` (an `Inline` implementation handing out the callee's `ir::Function`, which has to be a direct `call`:
that is why the direct call comes first; before it every call was a `yavs_rt_step` callback and there was nothing to
inline) then inlines bodies of up to 150 Cranelift instructions, 2000 in all per caller. Not native: a function that
calls a function without a variant, passes an address of its own frame to a call, allocates heap, reports, or does a
signal operation; of 256 functions in the utility library's random testbench 150 get a variant (after `Alloc`, `Stmt`
with a native temporary base and `Select` were added; 6 before).

Correctness: difftest 422/422 (the corpus plus `process/call_chain`, `process/call_chain_error`) for `yavs-clift`,
`yavs-clift-auto`, `yavs-auto3`, `yavs-auto3-eager` at 1 and 4 threads, and 422/422 at 1 thread for the same
candidates under `--features gc-stress,llvm,clift`; `cargo test --workspace` and `-p yavs-llvm -p yavs-clift -p yavs
--features llvm,clift` green (965 tests), fmt and clippy `-D warnings` clean.

Measurements (release build, every timed run pinned to performance core 6, medians of 5, CPU seconds including the
compile thread, host load average 11 to 15; cells under 0.1 s carry +-15%; A/B pairs run interleaved by
`run.py --compare` / `--ref`). "n0" is `YAVS_CLIFT_NATIVE=0` (the behaviour before), "nat" native variants without the
inliner, "nat+inl" the default. The main-branch columns are a separate `--features llvm` build of origin/main at 9d5a2438
(LLVM native frames, cheaper pipeline); the branch itself has not been merged with main (see below).

| design (CPU s) | interp | clift n0 | clift nat | clift nat+inl | c-auto n0 | c-auto nat+inl | auto3 nat+inl | sim1 | main: interp / auto / llvm |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---|
| calls | 0.08 | 0.10 | 0.01 | 0.01 | 0.14 | 0.04 | 0.04 | 0.05 | 0.07 / 0.06 / 0.01 |
| compute_h2h | 1.72 | 0.33 | 0.32 | 0.32 | 0.34 | 0.34 | 0.40 | 2.87 | 1.32 / 0.17 / 0.15 |
| rtl_ff | 0.96 | 1.34 | 1.32 | 1.33 | 1.36 | 1.32 | 0.93 | 0.68 | 0.94 / 0.92 / 0.87 |
| rtl_lfsr | 0.50 | 0.65 | 0.68 | 0.67 | 0.65 | 0.66 | 0.55 | 0.17 | 0.43 / 0.34 / 0.32 |
| rtl_chain | 0.25 | 0.33 | 0.34 | 0.34 | 0.32 | 0.31 | 0.25 | 0.17 | 0.19 / 0.19 / 0.22 |
| rtl_fan | 1.18 | 1.60 | 1.55 | 1.65 | 1.23 | 1.23 | 1.27 | 1.16 | 1.08 / 1.08 / 1.22 |
| rtl_bus | 0.25 | 0.27 | 0.27 | 0.27 | 0.27 | 0.26 | 0.24 | 0.62 | 0.19 / 0.20 / 0.20 |
| lfsr64 | 0.26 | 0.38 | 0.38 | 0.38 | 0.35 | 0.36 | 0.26 | 0.11 | 0.18 / 0.19 / 0.18 |
| xor_types | 0.85 | 1.29 | 1.26 | 1.26 | 1.21 | 1.30 | 1.15 | 0.83 | 0.82 / 0.76 / 0.91 |

Library testbenches, interleaved A/B (A cpu s -> B cpu s, total including translation): interpreter -> `clift-auto`
nat+inl: uvvm_util_rand_tb 0.83 -> 0.99, uvvm_sb_slv_tb 0.85 -> 1.18, osvvm_axistream 0.30 -> 0.33, vunit_vc30 1.39 -> 1.40
(geometric mean over eight benchmarks 1.09); interpreter -> `auto3`: 0.79 -> 0.88, 0.84 -> 1.00, 0.29 -> 0.32,
1.38 -> 1.44 (1.03). Native frames off -> on: `clift-auto` 0.86 and `auto3` 0.85 (only `calls` moves: 0.11 -> 0.04 and 0.08
-> 0.02; the testbenches 0.94 to 1.00), eager `clift` 0.82 (the same, `calls` 0.08 -> 0.02; translation of the utility
library's random testbench 3.8 s either way). Native without inlining -> with: 1.00 (`calls` 0.95).

Where the time goes (perf, `--call-graph dwarf` / flat, core 6): `calls` under the old frame-mode lowering spent
80% in the interpreter's call protocol (`TierHost::step`, `tier_view`, `step_in`, `ret`, `Pre::fill`) and 20 times fewer
cycles are left with native frames and inlining (2.5 G -> 0.30 G cycles for 20 M calls, LLVM `--tier=llvm` 0.28 G).
`rtl_ff` under `clift-auto`: generated code 3% of the cycles; 45% in `TierHost::signal`, `sig_read_scalar`,
`yavs_rt_signal`, `enter`, `view_again` (signal reads and drives and the entry of each process activation), the rest
kernel and interpreter. The utility library's random testbench under `clift-auto`: generated code 14% to 18%, tier
callbacks and entry 15% to 17%, interpreter 40%, libc memory functions 10%; under eager `clift`, excluding translation,
generated code is about a quarter of the simulation phase, callbacks another quarter.

What the numbers say: (1) a direct call plus Cranelift's inliner takes call-heavy code from 1.3x slower than the
interpreter to 8x faster, where `--tier=llvm` is (0.01 s for both); the inliner adds little over the direct call
(calls 0.95 to 1.0, `calls` microbenchmark 0.30 G -> 0.25 G cycles), because the cost was the frame protocol, not the
call instruction. (2) It moves nothing else: the RTL designs and testbenches spend their time in signal callbacks, process
entry, strings and heap in the runtime, and in the interpreter for functions that stay interpreted; generated code is
3% to 25% of the time there, so even free generated code could not pay (Amdahl). (3) On a compute loop Cranelift code is
2.1x LLVM's (0.32 against 0.15 s, 4.1 G against 2.5 G instructions): registers live in the frame array, every definition
is stored, and nothing is carried across blocks.

### clift: merged with main and measured again (2026-10-22, #481)

`spike/clift2` merged with main at 72420889 (merge commits dfb80ad5, 3e44a48c). The runtime half in `crates/yavs-llvm/src/rt.rs` is rebuilt from main's callbacks (native frames, `NSlot`/`TabSlots`, direct-mapped object cache, `Mem::begin`/`end`, `enter_on`, `call`, `preloads`); `AutoTier::start` takes the first-level generator as its sixth argument; `yavs-clift` got a `gc-stress` feature. One real porting bug: `Pre::lean` leaves the register of a parameter's slot address unset, so the Cranelift tier now starts every block from the preloaded addresses as known (#566). Checks: fmt, clippy `-D warnings` (default, `llvm,clift`, `clift`, `llvm`), `cargo test --workspace` 851 passed, `-p yavs-llvm -p yavs-clift -p yavs --features llvm,clift` 207 passed, `--features clift` alone 163 passed; difftest corpus 449/449 identical to the interpreter at 1 and 4 threads for `--tier=llvm`, `auto`, `clift`, `clift-auto`, `auto3` and `auto3 --tier-heat=0,0,0`, and 449/449 at 1 thread under `--features gc-stress` for `llvm`, `clift` and `auto3-eager`; CI on all ten jobs green, including `clift` on Linux aarch64 and Windows x86_64 (clippy, tests, difftest smoke; no runtime numbers there).

Measurements (one release `llvm,clift` binary, cores 0 and 1 of the 7950X3D, `perf stat` cycles and instructions of the whole process including the compile thread, median of 5 to 7 interleaved runs, host load 8 to 110 from other agents, so cycles carry up to 50% spread and instructions are the figure to compare; every cell has its own copy of the work library; "cold" = empty compiled-code cache, "warm" = populated). Instructions relative to the interpreter, geometric mean:

| set | llvm cold | llvm warm | clift | clift-auto | auto cold | auto warm | auto3 |
|---|---:|---:|---:|---:|---:|---:|---:|
| clocked RTL (8 designs) | 1.60 | 1.09 | 1.68 | 1.37 | 1.05 | 1.03 | 1.06 |
| loops and subprograms (10 designs) | 1.02 | 0.51 | 0.98 | 0.98 | 0.87 | 0.73 | 0.88 |
| testbenches with a run phase (8) | 3.48 | 1.52 | 2.17 | 1.31 | 1.01 | 0.87 | 1.22 |

Compile time of the eager tiers (the `init` phase): Cranelift 19 to 76 us per IR instruction against 57 to 930 us for LLVM (typically 6 to 8 times faster; neorv32 testbench 3.5 s against 10.4 s, the verification-library testbenches 1 to 2 s against 8 to 14 s). Instructions per loop iteration of `compute_h2h`: interpreter 1357, LLVM 121, Cranelift 212 (native frames and inlining: `calls` 34 instructions per call in both). Cranelift is behind LLVM on: code quality of loops (1.75x the instructions of O2), clocked RTL and testbenches (1.3 to 2.2x the interpreter's instructions: entry and callbacks, no inline object-cache lookup, no `drive`/`wait` fast paths), calls from the interpreter to native frames, a compiled-code cache, and the tiering policy (`clift-auto` still has the first scheme). The full tables and the gap list are in the #481 comment of 2026-10-22.

### clift: merged with main a third time, kernel-facing lowerings, native frames, Cranelift as the first tier (2026-10-24, #481)

`spike/clift2` merged with main at cf32383b (213 commits; merge commit 629cee7e). The runtime half in `crates/yavs-llvm/src/rt.rs` took main's changes to the callbacks (the 256-entry object table with its occupancy bitmap and shared-heap key, native-frame exits that record only the live registers, the new `drive`, `drive_bit`, `scan`, `codes` and `planes` callbacks, `SlicePos`/`LEq`/`LFill`/`Edge` in the per-instruction facts); the generic `Known<V>`/`ObjRef<V>` keep the LLVM generator's held objects out of the shared layer; the first-level hook of `AutoTier` sits next to main's clocked-process band, fused-search and trial rules. Checks on the merge commit: fmt, clippy `-D warnings` (workspace, `llvm,clift`, `clift`, `llvm`), `cargo test --workspace` 998 passed, `-p yavs-llvm -p yavs-clift -p yavs --features llvm,clift` 306 passed, difftest 504/504 identical to the interpreter at 1 and 4 threads for `interp`, `llvm`, `auto`, `clift`, `clift-auto`, `auto3`, `auto3-eager`, and 504/504 at 1 thread for `llvm`, `clift`, `auto3-eager` under `--features gc-stress`; CI green on Linux x86_64, Linux aarch64 and Windows x86_64 from the next commit on (the merge commit itself failed the Windows `llvm` clippy on an unused import; tests, perf counters, difftest smoke; no runtime numbers from the last two).

What was built on top (each its own commit, difftest on the Cranelift tiers after each): (1) the kernel-facing instructions, as the LLVM generator lowers them: `Inst::Edge` and `rising_edge`/`falling_edge`, `SigReadScalar`, `SigRead`, `'event`, the fused scalar drive (`yavs_rt_drive_bit`), block drives (`yavs_rt_drive`), `WaitOn` plus `Suspend` (`yavs_rt_wait`, `_wait_on`, `_wait_same`), frame-mode natives, and `Cat`, `TempRoot`, `Stmt`, `Deref`, `MemEq` through `TierHost::signal` instead of the generic step: a clocked process made 4800 signal callbacks and 960 steps per 1000 activations (4 and 8 under LLVM) and now makes what LLVM makes. (2) The object-table entry inline in every heap or array access and an inline `Deref` hit. (3) Native frames for protected calls (`CallProt`), `Deref`, `TempRoot` with `endstmt`, natives, and any instruction the host steps on a frame of its own (`yavs_rt_xstep`), with the slots of every native frame registered in `Mem::nslots`: the window generator testbench went from 42 to 261 variants and from 139 million `CallProt` steps to 5 million. (4) Logic `Cat` of known lengths in registers, and constants defined once known in every block. Tests: `compiled_edges`, `compiled_drives`, `compiled_waits`, `compiled_natives` run the Cranelift tiers too, and `perf.rs` has the counters of four designs under `--tier=clift`. Measurement aids: `YAVS_COMPILE_CPUS=3,5-7` (the CPUs of the compile thread, which otherwise goes to every CPU outside the `taskset` mask, #681), `YAVS_FIRST_ONLY=1` (no recompile at O2 under `--tier=auto3`), `YAVS_FIRST_RTL_MIN`, `YAVS_FIRST_BAND=1`; `tools/realbench/run.py --tiers` takes `clift`, `clift-auto`, `auto3` and `tier+VAR=VALUE`.

Method. One release binary per row (`tools/build-bench.sh --features llvm,clift`), the worlds of `tools/realbench` (a world per cell; cold = a copy of the pristine world per run, so no compiled-code cache; warm = populated cache; Cranelift has no cache, so its cold and warm differ only by the analysis stage cache), `perf stat` instructions and user+system CPU of the whole process tree including the compile thread, median of 3, simulation thread pinned to one of the cores 1 to 3, compile thread to core +4 (`YAVS_COMPILE_CPUS`), host load 15 to 55 from other agents (no timing core was free: wall seconds are on shared cores and not given; CPU seconds carry 10 to 30 percent spread, instructions repeat within the compile timing noise of #673). Rows `fin-a` are medians of 3 at 66492608; the one-run rows (`cold`, fresh world) are the final binary c40f76a9.

Compile time, eager tiers (`--time-report` `init` CPU minus the interpreter's, `--no-cache`, one run; IR = IR instructions Cranelift translated):

| design | IR instr | Cranelift s (us/IR) | LLVM O2 s (us/IR) | LLVM / Cranelift | LLVM with `YAVS_LLVM_FAST_MIN=1` s |
|---|---:|---:|---:|---:|---:|
| poc_arith_addw | 18,758 | 0.95 (51) | 5.9 (315) | 6.2 | 5.3 |
| fletcher_bufferwriter_512 | 44,700 | 1.95 (44) | 9.6 (214) | 4.9 | 9.9 |
| vai_window_gen_interlock | 72,448 | 2.28 (31) | 38.8 (536) | 17 | 41.9 |
| neorv32_single_min | 178,032 | 5.75 (32) | 20.4 (115; module over 150k LLVM instructions, so already at `Fast`) | 3.5 | 20.5 |
| grlib_leon3mp (std 93; `comb` of 146,855 IR over the size cap stays interpreted) | 299,941 | 18.6 (62) | 103.5 (370) | 5.6 | |

Cranelift translates at 31 to 62 us per IR instruction, LLVM at 115 to 536; forcing LLVM's `Fast` level for the whole module (`YAVS_LLVM_FAST_MIN=1`) changes its time by -10 to +8 percent, so it does not make eager LLVM cheaper. Per function in the tiered modes (compile notes of one run; `auto` = LLVM O2, `auto3` = Cranelift then O2): window generator `auto` 80 functions / 6,452 IR / 3.1 s (476 us/IR); `auto3` Cranelift 116 / 10,858 / 1.1 s (102 us/IR) then O2 26 / 3,832 / 2.4 s; Cranelift only (`YAVS_FIRST_ONLY=1`) 93 / 6,326 / 0.7 s (111 us/IR); poc_arith_addw `auto` 17 / 2,371 / 1.15 s, Cranelift 15 / 2,656 / 0.15 s (56 us/IR); neorv32 and fletcher: `auto` compiles nothing, Cranelift 9 to 20 functions of 1,357 to 5,932 IR in 55 to 400 ms. A cheap first level does let `auto` take 1.7 times the IR for a third of the compile time on the window generator; on the other designs the policy's own rules (clocked-process band, instance and share rules, trial) decide what is compiled and the compiler's price does not enter.

Run totals, user+system CPU seconds / instructions in G (whole process tree, compile thread included). Cold = empty compiled-code cache, warm = populated:

| design | interp | auto (LLVM O2) warm | auto cold | clift eager (cold = warm) | auto3 warm | auto3 cold | Cranelift only (`auto3` + `YAVS_FIRST_ONLY=1`) warm | cold |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| vai_window_gen_interlock (fin-a) | 37.0 / 495.6 | 16.7 / 187.3 | 20.0 / 250.8 | 32.5 / 396.3 | 37.9 / 466.6 | 34.9 / 488.6 | 25.1 / 380.3 | 29.5 / 376.4 |
| vai_window_gen_interlock (final, cold) | 31.6 / 497.9 | | 15.9 / 253.1 | 26.5 / 394.6 | | 27.4 / 427.7 | | 24.0 / 373.6 |
| neorv32_single_min (fin-a) | 8.4 / 63.4 | 7.3 / 64.1 | 6.7 / 65.8 | 13.1 / 120.3 | 6.3 / 64.3 | 7.1 / 66.2 | 5.8 / 65.2 | 6.8 / 66.1 |
| fletcher_bufferwriter_512 (fin-a) | 10.0 / 98.7 | 9.6 / 99.3 | 10.2 / 99.3 | 15.4 / 131.8 | 9.6 / 105.7 | 12.9 / 102.6 | 11.7 / 104.8 | 12.7 / 104.4 |
| poc_arith_addw (fin-a) | 22.4 / 331.9 | 19.4 / 264.2 | 23.2 / 294.6 | 29.4 / 379.8 | 28.7 / 359.2 | 30.4 / 372.4 | 30.1 / 353.2 | 28.8 / 359.8 |
| poc_arith_addw (final, cold) | 18.3 / 332.4 | | 19.8 / 302.3 | 23.5 / 364.9 | | 21.3 / 358.0 | | 21.3 / 343.8 |
| grlib_leon3mp (one run, cold) | 148.8 / 1687.6 | | 168.7 / 1805.6 | 226.2 / 2102.5 | | 177.4 / 1846.4 | | 169.1 / 1846.5 |

After the second merge with main (a953fb77, merge commit 6e5cfcf5; main's #652 changes the band wait, the callee compile and the trial rules, and its interpreter rounds lower the interpreter's own counts), one cold run each of the final binary, CPU s / G (window generator: median of 3, the runs repeat within 1 percent):

| design | interp | auto (LLVM O2) | clift eager | auto3 | Cranelift only (`YAVS_FIRST_ONLY=1`) |
|---|---:|---:|---:|---:|---:|
| vai_window_gen_interlock | 28.1 / 498.2 | 15.0 / 241.5 | 25.1 / 394.0 | | 23.1 / 368.0 |
| poc_arith_addw | 14.1 / 236.7 | 13.6 / 200.1 | 15.6 / 253.9 | 16.3 / 212.6 | 15.1 / 232.0 |
| neorv32_single_min | 4.2 / 49.7 | 4.2 / 50.2 | 10.3 / 108.7 | 4.3 / 50.3 | 4.2 / 50.4 |
| fletcher_bufferwriter_512 | 6.9 / 92.2 | 6.8 / 92.8 | 11.8 / 127.2 | 7.9 / 99.5 | 7.8 / 99.6 |

The verdict stands after the merge: Cranelift only is 26 percent below the interpreter in instructions on the window generator (LLVM `auto` 52 percent), level on neorv32, and 2 percent below on poc_arith_addw (LLVM 15 percent) and 8 percent above on fletcher (LLVM 1 percent above); in CPU seconds it is 18 percent below, 7 percent above and 13 percent above (LLVM 47 percent below, 4 percent below, 1 percent below).

Clocked RTL and loops (`tools/bench` h2h designs, median of 5, CPU s / G; cold and warm are equal where nothing is compiled):

| design | interp | llvm eager | auto warm | auto cold | clift eager | auto3 | Cranelift only | Cranelift only with `YAVS_FIRST_BAND=1 YAVS_FIRST_RTL_MIN=64` |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| rtl_ff | 0.84 / 11.79 | 1.08 / 9.28 | 0.82 / 12.27 | 1.22 / 12.30 | 1.12 / 10.21 | 0.93 / 12.28 | 1.28 / 12.29 | 0.90 / 10.69 |
| rtl_lfsr | 0.45 / 6.47 | 0.35 / 4.86 | 0.38 / 4.69 | 0.62 / 7.14 | 0.74 / 6.65 | 0.46 / 6.13 | 0.63 / 6.11 | 0.45 / 6.66 |
| rtl_chain | 0.30 / 3.25 | 0.38 / 3.29 | 0.38 / 3.35 | 0.38 / 3.35 | 0.38 / 3.43 | 0.34 / 3.35 | 0.34 / 3.35 | 0.34 / 3.38 |
| rtl_fan | 1.05 / 13.80 | 1.04 / 14.10 | 1.06 / 14.77 | 0.93 / 14.76 | 1.35 / 14.97 | 1.13 / 14.75 | 1.38 / 14.77 | 0.92 / 14.76 |
| rtl_bus | 0.29 / 3.78 | 0.22 / 3.67 | 0.23 / 3.82 | 0.22 / 3.83 | 0.24 / 3.93 | 0.33 / 3.81 | 0.27 / 3.83 | 0.29 / 3.88 |
| rtl_ram | 0.09 / 0.63 | 0.09 / 0.87 | 0.09 / 0.64 | 0.10 / 0.66 | 0.13 / 0.99 | 0.09 / 0.64 | 0.07 / 0.64 | 0.07 / 0.64 |
| lfsr64 | 0.22 / 3.32 | 0.22 / 2.56 | 0.22 / 3.41 | 0.30 / 3.43 | 0.30 / 4.12 | 0.28 / 3.41 | 0.19 / 3.41 | 0.26 / 3.67 |
| lfsr64_xor | 0.07 / 1.48 | 0.07 / 1.23 | 0.07 / 1.50 | 0.07 / 1.53 | 0.10 / 1.95 | 0.07 / 1.50 | 0.14 / 1.51 | 0.15 / 1.60 |
| compute_h2h | 1.75 / 26.22 | 0.14 / 2.51 | 0.18 / 2.80 | 0.50 / 7.17 | 0.30 / 3.66 | 0.25 / 4.34 | 0.34 / 3.94 | 0.27 / 3.99 |
| xor_types | 1.49 / 17.96 | 0.85 / 15.57 | 1.53 / 18.20 | 1.59 / 18.21 | 1.30 / 22.19 | 1.52 / 18.19 | 1.50 / 18.20 | 1.08 / 18.20 |

What the numbers say. (1) Correctness is not a gap (504/504 in every tier, gc-stress included). (2) Cranelift's compile price per IR instruction is 3.5 to 17 times lower than LLVM's, and the cost model of the first level is right (45 us start value, 31 to 111 measured per function). (3) Cheap compile does not make `auto` better: Cranelift-first beats the interpreter on totals only on the window generator (CPU -20 to -32 percent, instructions -23 to -25 percent, against -46 to -55 and -49 to -62 percent for LLVM `auto`), is level on neorv32 (0 to +4 percent instructions), costs +4 to +6 percent instructions on fletcher and +3 to +8 percent on poc_arith_addw, and on the clocked h2h designs compiles nothing (the policy keeps small processes interpreted) or, relaxed, gains 9 percent on `rtl_ff` and loses up to 11 percent on `lfsr64`. LLVM-first stays ahead of Cranelift-first on every design where either compiles anything. (4) Eager mode: Cranelift beats the interpreter on the window generator only (26.5 s against 31.6; eager LLVM spends 39 s on compile alone there and was not run again) and loses on neorv32, fletcher, poc and grlib; eager LLVM (measured on the merge binary, cold / warm CPU s / G) is worse than the interpreter everywhere: poc 26.7 / 23.7 s, 374.8 / 343.4 G, fletcher 25.8 / 19.3 s, 155.9 / 102.8 G, neorv32 22.0 / 11.1 s, 172.4 / 73.3 G; the verdict for eager mode is the same as before. (5) The hybrid `auto3` is worse than Cranelift alone on the window generator (466 and 428 G against 380 and 374 G): an LLVM-compiled caller cannot call a Cranelift native variant (the native ABIs differ, status plus value against a flag in `Mem`), so after the O2 upgrade the call goes through a step. (6) The gap to LLVM is in the code and the protocol, not in the compile policy: the Cranelift-only run of the window generator executes 374 G against LLVM's 253 G.

Gap list for "Cranelift replaces LLVM" (this round; effort in days of one person, evidence from the tables above):

| gap | evidence | effort | blocks replacing LLVM? |
|---|---|---|---|
| Registers and slot variables carried across blocks, loop optimisation (LLVM `Plan`, deferred stores) | `compute_h2h` 3.66 G (eager Cranelift) against 2.51 G (LLVM) and 26.2 G (interpreter); `xor_types` 22.2 G against 15.6 G, both worse than the interpreter's 18.0 G | L, 5 to 10 | yes if peak loop speed matters (perf over power); loop unrolling and vectorisation do not exist in Cranelift |
| Calls from the interpreter into native variants and into LLVM-compiled code (`Tier::call`, `ntab`, the `f<id>_c` adapters), one native ABI for both tiers (the reference simulator runs its interpreter and every generator under one calling convention, arguments and results in one scalar array, entered through a per-function entry pointer that tier-up overwrites, so a tier swap never changes how a caller calls; ours would need Cranelift to adopt the LLVM variants' convention, a value in a register and the unwind flag in `Mem::ndeopt`, and to publish into `ntab`), nested subprograms and signal parameters on native frames, closed frames, held objects (`ObjRef`), the size limits of a variant (400 IR, 128 registers, 1 KiB of slots) | window generator: LLVM `auto` makes 43 thousand `Call` steps in a run, Cranelift only 5 million steps and 261 variants of ~830 functions; hybrid `auto3` +14 to +30 percent instructions against Cranelift only | L, 5 to 8 | yes for library testbenches |
| Tiering policy generic over the generator (`Worker` owns an LLVM `Jit` and `Part`s; a Cranelift-only build runs the old heat-only `clift-auto`, which is 826 G against 380 G on the window generator) | `auto3` + `YAVS_FIRST_ONLY=1` is the configuration; `clift-auto` 826.5 G | M, 3 to 5 | yes (otherwise a build without LLVM is the old scheme) |
| Code quality of what is lowered: inline `SigRead` planes from codes, `lcopy_inline`, `Cat` of unknown lengths, `SlicePos`, `NewFrame`, `LFill`, `LogicOp` (xstep or callback today), held objects across instructions | window generator Cranelift only: `SlicePos` 2.1 M, `NewFrame` 0.5 M steps left; poc_arith_addw `Cat` of unknown length 8.3 M x 6 callbacks; +3 percent instructions over the interpreter | M, 3 to 5 | no, but it is where the testbench cycles are |
| Compiled-code cache | eager Cranelift pays 2 to 19 s per run (window generator 2.3 s, grlib 18.6 s); tiered Cranelift compiles 0.1 to 0.7 s | M (object emission and relocation), or skip | no |
| Windows and aarch64 | CI green for tests, perf counters and difftest smoke; no runtime numbers; unwinding through JIT frames on Windows untested beyond the callbacks that catch unwinds | S for numbers | no evidence of a blocker |
| Test sharing | `yavs-clift/tests/tier.rs` is still a trimmed copy (13 against about 30 LLVM tier tests); the four `compiled_*` tests and four perf designs run both | S, 2 | no |
| Maintenance and size | Cranelift pinned `=0.137.0` (`Inline` and alias-region APIs are young); +57 s clean release build, +5 MB stripped binary (2026-10-21, not repeated); benefit: no LLVM 21 install, no 10 ms library load, no `llvm-sys` pin | S per bump | the reason for doing it |

Decision for the user (recommendation, not made): keep LLVM as the performance tier. Cranelift is correct, 3.5 to 17 times cheaper to compile, and as the only tier now saves a quarter of the work on the queue-heavy testbench, but it does not beat the interpreter on four of the five real designs and LLVM does not either on three of them, so there is no case for making it the first level of `auto`; replacing LLVM needs the first three gap rows (about 13 to 23 days) and pays in the install and start-up, not in speed.

### clift: one native ABI for both generators and the generic tier-up policy (2026-10-24, #481)

User decision 2026-10-24: start the shared native ABI and the generic `Worker`; LLVM stays the performance tier. Both done on `spike/clift2` (merged with main at 4b80445d first and at 95371ab5 last; main's `off_simulation_cpus` removal is followed, so `YAVS_COMPILE_CPUS` is gone). The refactor of `auto.rs` could not go to main on its own: main has no `rt.rs`, the split of the runtime half out of `lower.rs` that everything neutral stands on.

What was built. (1) `yavs-llvm` builds without LLVM `native.rs` (`Part`, `Compiled`, `native_set`, `native_local`, `call_fits`, `is_inline`, `nat_inline`, `leaves`), `regs.rs`, `generator.rs` (`Generator`, `OptLevel`, `StartError`) and `auto.rs`: `Worker` and `AutoTier` drive `Generator`s given as `Gens { main, first }` (`AutoTier::start_with`; `start` is the LLVM entry); `LlvmGenerator` owns the JITs per level, the compilation cache hooks and the library load. No change to what LLVM generates or to its decisions (perf work-counter tests with `llvm` unchanged, instructions of `auto` cold equal before and after on all four designs). (2) Cranelift's native-frame variants use the LLVM tier's calling convention `(env, mem, depth, [up1, up2], cells...) -> i64` with `Mem::ndeopt` for recorded frames, are chosen by `native_set` (not by their own size limits), take nested functions (static links as parameters, `AddrOf Up`) and signal parameters (two cells), call callees in their module directly (inlined when small) and all others (cycles, LLVM's) through `Mem::ntab`; `c<id>` adapters serve `Tier::call`; `ClifTier` has ntab and ctab. (3) `First` (Cranelift) is a `Generator`: it reports the variants it built on the way (`Compiled::extra`), the `Worker` publishes them and replaces them with LLVM's when the root is compiled again at O2 (`nat_by`), together with the first level's native callees (`upgrade_tree`); `--tier=clift-auto` is `AutoTier::start_with` and Cranelift alone (the old heat-only `yavs-clift` `AutoTier` is gone); the `too_short` exemption of `called_natively` now applies to the first level. (4) Registers of a variant are not zeroed (the LLVM frames do not either): -2.5 percent instructions on the window generator. (5) Tests: `tests/compiled_mixed.rs` and the difftest candidate `yavs-mixed` (`YAVS_MIXED=1 YAVS_CLIFT_NTAB=1`: LLVM compiles the functions of even id and Cranelift the others, every call through the table, eager): chains, a failing check at depth, nested functions, a protected counter, 507/507 identical to the interpreter. Aids: `YAVS_CLIFT_ASM=<id>` (machine code of one function), `YAVS_O2_AFTER`.

Correctness (release binaries with `llvm,clift`, `clift` alone and gc-stress, at the merged head): difftest 509/509 for `yavs-llvm`, `yavs-auto`, `yavs-clift`, `yavs-clift-auto`, `yavs-auto3`, `yavs-auto3-eager`, `yavs-mixed` at 1 and 4 threads, and `yavs-llvm`, `yavs-clift`, `yavs-auto3-eager`, `yavs-mixed` under `gc-stress` at 1 thread (the first 4-thread run of `yavs-auto3-eager` found a crash, fixed in the same round: a frame-mode compile of the first level reported its variant again and the worker put it back over LLVM's, so the next O2 compile defined the LLVM variant twice); fmt, clippy `-D warnings` (default, `llvm`, `clift`, `llvm,clift`), `cargo test --workspace` 1068 passed, `-p yavs-llvm -p yavs --features llvm` 334, `-p yavs-clift -p yavs --features clift` 267, `-p yavs-llvm -p yavs-clift -p yavs --features llvm,clift` 352 passed.

Measurements (instructions of the whole tree in G, `perf stat`, simulation thread on core 1, compile thread on core 5, median of 3, spread in brackets, host load 15 to 55; all columns are the merged head 20d675db, one `llvm,clift` binary and one `clift` binary built with `tools/build-bench.sh`; "LLVM auto" is the code of main, its warm figures depend on the compilation-cache history of the world and on the run-to-run compile timing, #673, so cold is the comparable one; "mixed" is `YAVS_MIXED=1 YAVS_CLIFT_NTAB=1`, LLVM for half of the functions and Cranelift for the other half):

| design | interp | LLVM auto warm / cold | auto3 | Cranelift only (`clift-auto`) | clift-only build | eager `clift` | mixed |
|---|---:|---:|---:|---:|---:|---:|---:|
| window generator | 495.2 | 180 to 205 / 244.9 | 367.4 | 362.5 | 361.7 | 393.3 | 290.9 |
| neorv32_single_min | 42.5 | 43.2 / 43.1 | 43.3 | 43.2 | 43.3 | 96.4 | 43.1 |
| fletcher_bufferwriter_512 | 84.6 | 85.3 / 85.3 | 90.4 | 88.3 | 88.5 | 119.7 | 85.2 |
| poc_arith_addw | 236.2 | 166.0 / 207.7 | 215.4 | 233.4 | 233.3 | 256.2 | 200.7 |

Before this round (the binary at the first merge, older main: interpreter 495.7, 47.5, 92.1, 236.1 G): `auto3` 452.1, 50.7, 98.6, 201.6 G; `clift-auto` (the old heat-only scheme) 368.0, 51.7, 106.6, 232.2 G; eager `clift` 392.4, 107.6, 126.9, 253.4 G. LLVM `auto` cold before and after the policy refactor (same main): 242.4 and 243.5 (window generator), 48.1 and 48.3, 92.6 and 92.7, 202.4 and 198.3 G: no change; its warm fletcher figure moved 90.6 to 91.7 G because the two binaries' worlds differ in which functions the compilation cache lists (the 744-instruction `reg_proc` is not probed in the new binary's world), not in the code.

Reading. The calling convention removes the hybrid's penalty (window generator `auto3` 452 -> 367 G, now equal to Cranelift only; the old heat-only `clift-auto` measured 368.0 G on this tip, so the 826 G of the earlier comment was not reproduced), and a split of the functions between the generators (`YAVS_MIXED`) runs correctly and at 290.9 G on the window generator and 200.7 G on poc_arith_addw, between the two (LLVM compiles half of the functions there; it says what the shared convention allows, not what the policy picks). A build without LLVM now runs the real policy: neorv32 and fletcher go from +9 and +16 percent over the interpreter to +2 and +4 percent (the same in the build without LLVM), because the policy's rules (band, instances, share, trial, payback) refuse what does not pay. The hybrid does not yet profit from LLVM O2 where LLVM auto does (window generator 367 against 180 to 205): the hot functions there are tiny accessors (`get`, `set`, `unsafe_pop`, `length`) whose Cranelift variants are heavy for what they do (the machine code of `get` has a 0x260-byte frame, 80 bytes of slot zeroing, overflow-checked index arithmetic on constants and registers kept in memory; 20 percent of the run is in `get` alone), where LLVM has closed frames, held objects and inlining for them; raising or lowering the O2 threshold (`YAVS_O2_AFTER` 3, 8, 30) and upgrading the first level's native callees with their caller moves nothing. That is the code-quality row below.

Gap list after this round (days of one person; the first two items of the earlier list are done):

| gap | evidence | effort | blocks replacing LLVM? |
|---|---|---:|---|
| Code quality of what is lowered, above all native frames of tiny functions: closed frames (no `nslots` registration, slots in SSA), held objects across `Deref`, constants folded into index arithmetic, a cheaper prologue, then inline `SigRead` planes, `lcopy_inline`, `Cat` of unknown length, `SlicePos`, `NewFrame`, `LFill`, `LogicOp` | window generator: `get` 20 percent, `set` 9, `unsafe_pop` 4 of the run; `SlicePos` 2.1 M and `NewFrame` 0.5 M steps; poc `Cat` of unknown length 8.3 M x 6 callbacks | L, 5 to 8 | yes for library testbenches (this is why the hybrid equals Cranelift only) |
| Registers and slot variables carried across blocks, loop optimisation | `compute_h2h` 3.7 G against 2.5 G (LLVM) and 26.2 G (interpreter); `xor_types` 22.2 G against 15.6 G | L, 5 to 10 | yes if peak loop speed matters |
| Policy constants for Cranelift (bands, `first_rtl_min`, payback, `O2_AFTER`) tuned for LLVM's price and gain | poc: Cranelift only 233 G against LLVM auto 167 G and hybrid 206 G | M, 2 to 3 | no |
| Native frames in Cranelift for what LLVM leaves in frame mode (none known) and a Cranelift variant calling an LLVM variant directly instead of through `ntab` after the O2 upgrade (callers keep their direct call to the old Cranelift body) | not measured | S | no |
| Compiled-code cache | eager Cranelift pays 2 to 19 s per run | M, or skip | no |
| Windows, aarch64 | CI green before this round, no runtime numbers; the variants now cross the generators on the platform's default calling convention, untested there | S | no evidence of a blocker |
| Test sharing | `yavs-clift/tests/tier.rs` is still a trimmed copy | S, 2 | no |
| Maintenance | Cranelift pinned `=0.137.0`; +57 s clean release build, +5 MB binary | S per bump | the reason for doing it |

### clift: round 3, frame-mode host accesses, first-level rules per generator, the memo (2026-10-24, #481)

`spike/clift2` merged with main at a670c3c1 (the build with `llvm` needed one line after the first merge: the twin path of
`compile_one` reports no `extra`; the merge with a670c3c1 follows the new guesses of frame-mode accesses, `OBJ_SHIFT` and
`track` taking the function). Decisions: `docs/decisions.md` "Cranelift tier, round 3". Method: instructions of the whole
process tree (`perf stat`, compile thread included), one release binary per row from `tools/build-bench.sh`, simulation and compile
thread on two cores of CCD0 (no timing cores: cycles were not measured, so nothing here says how the changed code runs, only how much
it executes). Cold = `--no-cache` (every run as cold as the first, the interpreter's figures repeat exactly), medians of 5 runs
(7 for `poc_arith_addw`); warm = stage cache and LLVM object cache populated, medians of 5. The panel is 51 designs of
`tools/bench` and `tools/realbench` of 1 G instructions or more, single cold runs. Instructions in G (ratio to the interpreter):

| design | interp | clift-auto before | clift-auto now | auto3 now | LLVM `auto` cold |
|---|---:|---:|---:|---:|---:|
| window generator | 450.3 | 194.1 (0.431) | 173.9 (0.386) | 174.1 (0.387) | 208.8 (0.464) |
| neorv32_single_min | 37.7 | 40.9 (1.086) | 38.3 (1.016) | 38.2 (1.013) | 38.3 (1.016) |
| fletcher_bufferwriter_512 | 55.6 | 56.7 (1.020) | 53.6 (0.964) | 54.7 (0.984) | 56.5 (1.016) |
| poc_arith_addw | 154.9 | 124.9 (0.806) | 112.5 (0.726) | 114.0 (0.736) | 119.8 (0.773) |
| geometric mean of the panel | | 0.931 | 0.912 | | |

Warm (interp / `clift-auto` / LLVM `auto`): window generator 447.7 / 171.9 / 155.5, neorv32 35.6 / 36.2 / 36.2, fletcher 55.1 / 54.2 / 56.1,
poc 154.3 / 113.5 / 112.4. `clift-auto` is 61 percent below the interpreter on the window generator (LLVM 65), 26 percent on
`poc_arith_addw` (level with LLVM), 2 percent on fletcher (LLVM: 2 percent above), and 1.7 percent above on `neorv32_single_min`, the
same as warm LLVM `auto`: with nothing compiled the same run is 38.2 G cold (0.43 G in the interpreter loop of a run with a tier, 0.07 G on
the tier-up thread, #544), so the floor is above the interpreter by the tier itself; compiling single functions moves it by at most
0.3 G and only `counter_increment` pays (-0.6 G), which the callback rule refuses (53 of its 399 IR instructions leave the compiled
code; the limit is one in ten). `poc_arith_addw` has a second mode: one run in four to six lands at 119 to 151 G, when the trial of
the 262-instruction process `_p8` measures 1.04 instead of 1.13 to 1.19 (the limit is 1.05) and takes it back, with all its copies
(#673).

What changed: (1) frame-mode accesses of unknown address space call `yavs_rt_host` instead of testing frame, temporary area and
instance data inline, with the one inline test the reference lowering has for a guess (fletcher: 69 000 to 49 000 Cranelift
instructions, compile thread 2.89 to 2.36 G; panel 0.920 to 0.912). (2) `Tune`: the clocked-process rules per generator (64 IR
instructions, no wait outside the band, four instances, a thirty-second of the run; second level after 300 times the first-level
threshold). (3) The policy tells the generator which functions it compiles (`Generator::tree`) and charges the variants a root's
compile made (`peek` of a verification library: 112 ms for 26 variants never seen running). (4) Compiled calls of memoised pure
functions ask the memo (the window generator -8 percent, `pure_memo` 0 to 435 hits). (5) A Cranelift error is a panic in debug builds
(`tests/verifier.rs`). (6) A measuring aid: `YAVS_CLIFT_CODEDIR` also writes `<name>.srcmap`, the IR pc of every stretch of machine
code, and with `perf script` addresses gives the instructions executed per IR instruction (`perf record -e instructions:u` samples
land on the instruction after a slow one, so they weigh stalls: use counts as a guide and `perf stat` for totals).

Panel remainder. Designs that run above the interpreter under `clift-auto` in the final binary, ratio to the interpreter's
instructions: `rtl_fan` 1.09 (processes of 160 to 300 IR instructions made many times, level in the trial), `uvvm_sb_slv`
1.3 to 1.5, `uvvm_util_rand` 1.2 to 1.5 and `poc_cache_cpu` 1.2 (library testbenches: the compile thread spends 4 G of a 13 G run
on native variants of utility functions whose compiled code executes the instructions the interpreter does; the spread between
runs is the policy's timing dependence, #673), `poc_fifo_ic_assembly` 1.16, `ieee_heavy` 1.10. `compute_h2h` is 0.15, `xor_types`
0.58, `xor_sig` 0.62 of the interpreter's instructions (warm LLVM `auto` in the same cache mode 0.10, 0.33, 0.36).

Registers across blocks in frame mode, measured and not built (decisions.md): in the two hot processes of `poc_arith_addw` the
stores to the register array are 6 percent of the instructions generated code runs; the rest is `Cat` of unknown length (22 percent),
entry and exit (19), `CallNative` (15), `SubtypeCheck` (10). The frame-mode loops that remain above warm LLVM `auto` (`xor_types`) spend their instructions in a conversion `CallNative`, a `SubtypeCheck` and an `LCopy` per iteration, not in register traffic.

Gap list after this round:

| gap | evidence | effort | blocks replacing LLVM? |
|---|---|---:|---|
| Policy by measured benefit instead of share and callback count (the work on main: work clock, benefit model) | neorv32 floor above the interpreter; UVVM testbenches 1.03 to 1.16 (round 4; was 1.2 to 1.5); `counter_increment` refused although it pays | M | no |
| Loop bodies of the compute designs: the conversion `CallNative`, `SubtypeCheck` and `LCopy` of `xor_types`, then registers and slot variables as phis in loops | `compute_h2h` 3.9 G against 2.6 G (warm LLVM `auto`), `xor_types` 7.1 G against 4.0 G, `xor_sig` 8.3 G against 4.8 G | M for the instructions, L for the phis | only for compute-bound testbenches |
| Compile cost: 6 percent of the window generator's run is translation (11 G), 4 percent of fletcher's | native variants: 30 Cranelift instructions per IR `Load`, 53 per `DopeLen`, 57 per `NewFrame`; frame mode 195 per `LCopy` | M | no |
| Compiled-code cache, Windows and aarch64 numbers, test sharing, Cranelift upgrades | as before | S to M | no |

### clift: round 4, the merge with main, what a first-level compile owes (2026-10-25, #481, #718, #544, #732)

`spike/clift2` merged with main at 9171b609 (the generic `Worker` with #715 `FastRa`, #725 the sampler start, #719, #711, #726, #730,
#519; `docs/decisions.md` "Cranelift tier, round 4"). Method as in round 3, re-baselined on this tree (the interpreter's figures moved
with main: `neorv32_single_min` 39.8 G, was 37.7), runs four at a time so the spread is larger than in round 3. Instructions in G
(ratio to the interpreter), medians of 3, cold:

| test | interp | clift-auto before | clift-auto now | auto3 now | LLVM `auto` before | LLVM `auto` now |
|---|---:|---:|---:|---:|---:|---:|
| window generator | 449.0 | 167.6 (0.373) | 171.9 (0.383) | 173.0 (0.385) | 199.7 | 198.8 |
| neorv32_single_min | 39.8 | 41.5 (1.040) | 40.9 (1.025) | 41.4 (1.040) | 40.4 | 40.4 |
| fletcher_bufferwriter_512 | 54.7 | 52.9 (0.966) | 53.6 (0.978) | 53.0 (0.969) | 55.5 | 55.6 |
| poc_arith_addw | 156.7 | 116.0 (0.740) | 112.0 (0.714) | 111.5 (0.711) | 118.0 | 117.6 |
| uvvm_sb_slv | 13.05 | 15.95 (1.22) | 14.01 (1.07) | 14.03 (1.08) | 13.35 | 13.32 |
| uvvm_util_rand | 15.16 | 18.69 (1.23) | 15.64 (1.03) | 15.68 (1.03) | 16.96 | 15.27 |
| uvvm_axistream_vvc | 37.9 | 41.5 (1.09) | 43.9 (1.16) | 44.0 (1.16) | 46.9 | 38.6 |
| osvvm_axis_random | 4.72 | 5.56 (1.18) | 4.95 (1.05) | 4.98 (1.06) | 4.85 | 4.76 |
| osvvm_axi4_random | 8.78 | 9.15 (1.04) | 8.98 (1.02) | 9.11 (1.04) | 8.95 | 8.83 |
| poc_cache_cpu | 8.80 | 9.54 (1.08) | 9.42 (1.07) | 9.40 (1.07) | 9.09 | 9.08 |
| hdlm_axi_fifo | 2.41 | 2.50 (1.04) | 2.41 (1.00) | 2.42 (1.00) | 2.51 | 2.42 |
| olo_base_cam | 2.68 | 2.81 (1.05) | 2.68 (1.00) | 2.68 (1.00) | 2.81 | 2.68 |

Geometric mean of the eight library testbenches (the rows from `uvvm_sb_slv` down): 1.115 before, 1.050 now for `clift-auto`, 1.066
and 1.012 for LLVM `auto`. The designs the round 3 targets name: the window generator and `poc_arith_addw` stay far below the interpreter
(0.38 and 0.71), fletcher 2 percent below, `neorv32_single_min` at the tier's floor (+2.5 percent: with nothing compiled 40.4 G). Worse
than before: `uvvm_axistream_vvc` (a 2.7 s run in which the old policy compiled the functions that gain: 1.09 to 1.16 where it was
1.04 to 1.09 in serial runs); its LLVM `auto` figure is bimodal (38.6 or 46.9 in the same binary, #673). What changed: the merge
(Cranelift lowers `SigReadScalar` in native frames), `Tune::FIRST` `tree_payback` 8 and `payback_rtl` 16 (a root owes 8 times the price
of the callees compiled with it, a clocked process 16 times its own), the class rules of the threshold are asked only of a function at
or above the base threshold (all generators: the compile thread optimised library functions a run never executes; short runs with
nothing compiled are level with the interpreter now), and the `cb_*` bounds of the Cranelift perf cases guard something. What is left: the
interpreter loop of a run with compiled code executes 5 percent more instructions per op than the watching loop of a run with a tier and
no code (shared interpreter code, the other half of #544), so a testbench whose compiled code saves less than that ends above the
interpreter (`uvvm_sb_slv`, `poc_cache_cpu` 1.07); a policy that prices by measured benefit (the work in progress on main) is where
`uvvm_axistream_vvc` and these are decided.

### clift: round 5, the merge with main and a compiled-code cache (2026-10-26, #481)

`spike/clift2` merged with main twice (6ec6356f, 11a5d5f0: the work clock and the per-kind `Model`s, compiled copies, frame
temporaries, native-frame drives, the stop poll and the bound of processes that never suspend, the run-to-run profile; decisions.md
"Cranelift tier, round 5"). The Cranelift lowering got `Inst::Zero`, `ScalarDrive::head`, the exit at the jump back of a process that
never suspends, and `Drive`/`DriveSlot`/`SigRead` in native variants. The run-to-run profile of the policy is kept in the store of
the generator of the first level, so a build without LLVM has it too. Cold instructions after the first merge, ratio to the
interpreter (medians of 3, `--no-cache`): window generator 0.388 (round 4: 0.383), `neorv32_single_min` 1.025 (1.025),
`fletcher_bufferwriter_512` 0.996 (0.978), `poc_arith_addw` 0.680 (0.714); LLVM `auto` 0.417, 1.023, 1.009, 0.686.

The compiled-code cache (`yavs-clift/src/cache.rs`): one `clif-<key>.img` per Cranelift function in the work library's objects
store, under Cranelift's own key of the body and the target; eager neorv32 translates 1 804 functions in 4 079 ms cold and links
them in 536 ms warm, every function a hit from the second run on. Gates on a9856e74: fmt, clippy `-D warnings` for default,
`llvm`, `clift`, `llvm,clift`; `cargo test --workspace` (1 153) and the feature sets (`llvm,clift` 441, `clift` 336, `llvm` 417);
difftest 532/533 for `yavs-clift`, `-clift-auto`, `-auto3`, `-mixed`, `-llvm`, `-auto` at 1 and 4 threads (the one failure,
`process/no_wait_infinite`, fails for the interpreter too: #763).

Full total (analyze, elaborate, run) of a Cranelift-only build (`--features clift`, `auto` is `clift-auto`) and an LLVM build
(`--features llvm`), both `tools/build-bench.sh` of a9856e74, against sim1; realbench `--compare --runs 5`, timing cores 8 and 9:

| test | Cranelift warm | LLVM warm | Cranelift cold | LLVM cold | run s Cranelift warm / cold | run s LLVM warm / cold |
|---|---:|---:|---:|---:|---|---|
| poc_arith_addw | 1.28 | 0.98 | 1.32 | 1.21 | 6.97 / 7.04 | 5.31 / 6.44 |
| fletcher_bufferwriter_512 | 2.11 | 2.06 | 2.22 | 2.10 | 4.18 / 4.33 | 4.09 / 4.09 |
| neorv32_single_min | 0.78 | 0.78 | 0.87 | 0.87 | 2.61 / 2.84 | 2.61 / 2.84 |
| osvvm_axis_random | 0.14 | 0.15 | 0.37 | 0.37 | 0.17 / 0.28 | 0.17 / 0.27 |
| uvvm_sb_slv | 0.50 | 0.50 | 0.73 | 0.69 | 0.65 / 0.78 | 0.65 / 0.72 |
| uvvm_util_rand | 0.50 | 0.49 | 0.80 | 0.71 | 0.79 / 1.04 | 0.77 / 0.91 |
| uvvm_axistream_vvc | 0.83 | 0.81 | 0.87 | 0.86 | 3.01 / 3.00 | 2.94 / 2.98 |
| osvvm_axi4_random | 0.26 | 0.24 | 0.45 | 0.45 | 0.55 / 0.66 | 0.51 / 0.64 |
| vai_window_gen_interlock | 1.88 | 1.68 | 2.17 | 2.39 | 8.92 / 9.82 | 7.97 / 10.88 |
| geometric mean | 0.678 | 0.643 | 0.920 | 0.895 | | |

Cranelift against LLVM (geometric mean of the full totals): warm 1.055, cold 1.028; before the cache the same nine tests were 1.174
warm and 1.017 cold, and a run of the cache before the second merge measured 1.023 warm and 1.039 cold. What is left warm is code
quality on heavy compute (`poc_arith_addw` 99 G instructions against 87 G warm, window generator 157 G against 135 G); the
verification-library tests are level.

### clift: round 11, the first level's rules for large subprograms and Frame functions, breadth, go/no-go (2026-10-27, #481)

`spike/clift2` merged with main up to ac8d4e88. With a first-level generator alone (a Cranelift-only build), large
subprograms are `Kind::Big`/`BigLoop` as LLVM's `OptLevel::Fast` makes them (`native::big`, `Tune::big`) and `Kind::Frame`
functions stay interpreted (`Tune::frame`): their frame-mode code moved whole processes into the interpreter's loop for
compiled code (`uvvm_axistream_vvc`: 8.8 percent more cycles than LLVM at level instructions). A Cranelift-only build runs
`--tier=auto` by default and names its tier in `--version`. Breadth, Cranelift-only / LLVM build, `--tier=auto`, loaded host,
instructions primary: realbench 42 tests 1.004 warm and 0.981 cold in instructions (cycles 1.013 and 0.987; `grlib_leon3mp` 1.15 and 1.27 in
cycles at level instructions); h2h 11 designs
1.179 warm and 0.992 cold (cycles 1.141 and 0.936): warm compute loops in frame-mode code run up to twice LLVM's
instructions (`xor_types` 2.02, `lfsr64` 1.29, `compute_h2h` 1.14). Go/no-go: no-go as the default compiled tier until the
frame-mode loop code (register array and `Mem` in registers, hoisted dope checks) closes that gap; the list is in
decisions.md "Cranelift tier, round 11".

### clift: round 10, the verdict on a quiet host, `Tune::FIRST`, search loops (2026-10-27, #481)

`spike/clift2` merged with main up to e790e0de; the Cranelift lowering got main's #766 and #772 lowering (`WaitEdge`
on the kernel, the edge waits' `wait_same`, a signal read from one plane word) and search loops through
`yavs_rt_scan`. The verdict table (Cranelift-only build / LLVM build, `--tier=auto`, `--runs 7`, every test started at
load under 4, highest load 3.6): wall 0.997 warm and 0.968 cold, cycles 0.996 and 0.963, instructions 0.983 and
0.953 (geometric means over the 9 tests). Slower warm: `uvvm_axistream_vvc` 1.10, `osvvm_axi4_random` 1.04 (1.00 in
instructions), `poc_arith_addw` 1.03; faster: the window generator 0.91, `uvvm_util_rand` 0.92, `uvvm_sb_slv` 0.94.
`Tune::FIRST` stays: LLVM's rules for clocked processes in the Cranelift-only build were 1.007 in wall and instructions
warm (window generator 1.058 instructions, `fletcher_bufferwriter_512` 0.971). Search loops: 0.986 instructions warm,
cycles level. Details and the table: decisions.md "Cranelift tier, round 10".

Gates on e6034249 (the code of this round; docs after it): fmt, clippy `-D warnings` for default, `llvm`, `clift`,
`llvm,clift`; `cargo test --workspace` (1 214) and the feature sets (`llvm` 1 332, `clift` 1 248, `llvm,clift` 1 363); difftest
543/543 for `yavs-clift`, `-clift-auto`, `-auto3`, `-mixed`, `-llvm`, `-auto` at 1 and 4 threads (debug build, verifier on).

### clift: round 9, objects in hand in frame mode, one branch per compare, the native chain by space (2026-10-26, #481)

`spike/clift2` merged with main at db3176aa (the shared std store, `WaitEdgeSuspend`, the callee-aware payback rule,
#768's likely-heap space facts, which the Cranelift lowering reads too). Frame-mode code keeps the object a `Deref`
found in hand (pointer and length as block parameters of the join; length 0 where the entry is not vouched for), so
`procChecker`'s load and store of one heap word no longer look up the object table each; it is now 1.03 times the
instructions of LLVM's eager O2 code. Additions and subtractions of a constant test overflow with a compare against
the operand, or not at all inside a `BinRange` whose bounds catch every wrapped sum. A branch on the `and`/`or` of
compares is one branch per compare (`brif_split`); `Pos` has one arm per direction; the native-frame chain goes by
the address's space; address parameters used in loops are resolved at the entry of a native frame (`param_addrs`),
as in the LLVM lowering. Eager `--tier=clift`: the window generator 148.3 G to 123.2 G instructions, `poc_arith_addw`
103.0 G to 99.7 G. Details: decisions.md "Cranelift tier, round 9".

Gates on 31bf2606 (the code of this round; docs after it): fmt, clippy `-D warnings` for default, `llvm`, `clift`, `llvm,clift`;
`cargo test --workspace` (1 189) and the feature sets (`llvm` 1 304, `clift` 1 222, `llvm,clift` 1 334, all test binaries
counted); difftest 537/537 for `yavs-clift`, `-clift-auto`, `-auto3`, `-mixed`, `-llvm`, `-auto` at 1 and 4 threads (debug
build, verifier on). The merge needed `#[cfg(feature = "llvm")]` on two new policy tests of `auto.rs` (helpers that
exist with LLVM only; main compiles `auto.rs` with LLVM only, this branch for Cranelift too).

Full total (analyze, elaborate, run) of a Cranelift-only build (`--features clift`) and an LLVM build (`--features llvm`), both
`tools/build-bench.sh` of 31bf2606, against sim1; realbench `--compare --runs 5`, timing cores 9 and 8, host load 9 to 22
at the start, falling to 6; last column the warm run instructions in G:

| test | Cranelift warm | LLVM warm | Cranelift cold | LLVM cold | G warm Cranelift / LLVM |
|---|---:|---:|---:|---:|---|
| poc_arith_addw | 1.06 | 0.99 | 1.10 | 1.15 | 89.0 / 84.6 |
| fletcher_bufferwriter_512 | 2.20 | 2.12 | 2.33 | 2.27 | 50.8 / 51.4 |
| neorv32_single_min | 0.77 | 0.76 | 0.87 | 0.84 | 30.8 / 30.4 |
| osvvm_axis_random | 0.15 | 0.15 | 0.37 | 0.36 | 2.6 / 2.6 |
| uvvm_sb_slv | 0.44 | 0.48 | 0.66 | 0.69 | 9.1 / 10.0 |
| uvvm_util_rand | 0.47 | 0.48 | 0.69 | 0.70 | 11.0 / 12.3 |
| uvvm_axistream_vvc | 0.77 | 0.68 | 0.88 | 0.82 | 32.1 / 30.3 |
| osvvm_axi4_random | 0.26 | 0.25 | 0.45 | 0.44 | 6.6 / 6.6 |
| vai_window_gen_interlock | 1.63 | 1.81 | 1.97 | 2.43 | 124.0 / 137.3 |
| geometric mean | 0.643 | 0.633 | 0.872 | 0.885 | |

Cranelift against LLVM: warm 1.015, cold 0.985 in wall (round 8: 1.003 and 0.985 at load 20 to 27); in instructions of the
run 0.980 warm and 0.956 cold. Cranelift now runs fewer instructions than LLVM on the window generator (0.90 warm, 0.79
cold), `uvvm_sb_slv` and `uvvm_util_rand` (0.91, 0.90), more on `poc_arith_addw` (1.05) and `uvvm_axistream_vvc` (1.06);
the wall ratios of the tests whose instructions are level (`osvvm_axi4_random` 1.00 in instructions, 1.09 in wall) say the
rest is cycles per instruction or noise at this load. What is left: `uvvm_axistream_vvc` and `poc_arith_addw` warm, the
frame-mode processes of the window generator (1.8 times LLVM O2's instructions eager, e.g. the element loop of
`bus_process` over `LGet`s), the native variants (1.2 times), and the `Mem` pointer in a stack slot.

### clift: round 8, the merge with main, the layout slots apart, constant descriptors (2026-10-26, #481)

`spike/clift2` merged with main at 9b5ce798 and f320c0dc: the Cranelift lowering uses the object table keyed by machine
(the tag and entry offset of `Mem::slot` in its inline look-ups and its `Deref` hit test), the `auto` policy of #762 links
frame-mode code along with cached native-frame code through the generator of the compile. User decision 2026-10-26:
`--tier=clift-auto` keeps the fused 32-byte blocks in the frame through a switch of its own (`opt::frame_blocks`) and no
longer gets the layout slots (`YAVS_CLIFT_LAYOUT_SLOTS=1` adds them; measured level in time and instructions on
`poc_arith_addw`). A copy that shares another's Cranelift body is charged as a cache link. A `Pos` in a constant-pool
descriptor and a range check with constant bounds are one unsigned compare (`crates/yavs/tests/clift_const_dims.rs`).
Details: decisions.md "Cranelift tier, round 8".

Gates on f63ddf3c (the code of this round; docs after it): fmt, clippy `-D warnings` for default, `llvm`, `clift`, `llvm,clift`; `cargo test --workspace` (1 170) and
the feature sets (`llvm` 1 283, `clift` 1 203, `llvm,clift` 1 312, all test binaries counted); difftest 533/533 for `yavs-clift`, `-clift-auto`, `-auto3`, `-mixed`, `-llvm`, `-auto` at 1 and 4 threads (debug build, verifier on).

Full total (analyze, elaborate, run) of a Cranelift-only build (`--features clift`) and an LLVM build (`--features llvm`), both
`tools/build-bench.sh` of f63ddf3c, against sim1; realbench `--compare --runs 5`, timing cores 10 and 11, host load 20 to 27
(round 7: about 10); last column the warm run instructions in G:

| test | Cranelift warm | LLVM warm | Cranelift cold | LLVM cold | G warm Cranelift / LLVM |
|---|---:|---:|---:|---:|---|
| poc_arith_addw | 1.01 | 0.95 | 1.07 | 1.14 | 89.7 / 84.6 |
| fletcher_bufferwriter_512 | 1.98 | 2.00 | 2.22 | 2.12 | 47.3 / 48.8 |
| neorv32_single_min | 0.73 | 0.77 | 1.00 | 0.89 | 30.6 / 30.3 |
| osvvm_axis_random | 0.15 | 0.15 | 0.37 | 0.36 | 2.6 / 2.6 |
| uvvm_sb_slv | 0.38 | 0.42 | 0.55 | 0.58 | 9.8 / 10.8 |
| uvvm_util_rand | 0.37 | 0.37 | 0.54 | 0.57 | 11.2 / 12.3 |
| uvvm_axistream_vvc | 0.72 | 0.62 | 0.75 | 0.72 | 33.5 / 32.7 |
| osvvm_axi4_random | 0.27 | 0.27 | 0.49 | 0.48 | 6.6 / 6.6 |
| vai_window_gen_interlock | 1.44 | 1.41 | 1.72 | 2.11 | 148.8 / 137.8 |
| geometric mean | 0.589 | 0.587 | 0.819 | 0.831 | |

Cranelift against LLVM: warm 1.003, cold 0.985 (round 7: 1.037 and 1.015; the same table on 6cc3c558, the merge and the
layout switch without the constant descriptors, cores 9 and 12, load about 8: 1.049 and 0.994). At this load single cells
move by 10 percent between runs (`neorv32_single_min` cold 1.00 against 0.84 on 6cc3c558, `uvvm_axistream_vvc` warm 0.72
against 0.67); warm instructions are steadier: `poc_arith_addw` 92.5 G to 89.7 G (LLVM 84.6 G), the window generator 152.9 G
to 148.8 G (LLVM 137.8 G). What is left: `procChecker` of `poc_arith_addw` under `auto` takes 1.55 times LLVM O2's
instructions (two object-table look-ups for the load and store of one heap word in frame-mode code, the `Mem` pointer
reloaded from a stack slot, flag-byte overflow tests), the native variants of the window generator's accessors
(`unsafe_pop`, `decode`: unchanged this round), and cold compile cost of the library testbenches.

### clift: round 7, deferred stores, values across entries, shared copies (2026-10-26, #481)

Frame-mode code of processes defers the stores of the registers quiet instructions define and stores them where the
interpreter or a callback can look at the frame; registers the interpreter preloads are never stored; a slow path's
join and the entry after a call take the values in hand as block parameters (the switch over the entries is made last,
its stubs load them). Copies of a process with the same Cranelift body are one definition (36 to 4 in `poc_arith_addw`).
`Copy` stores word pairs as 16 bytes and the drive callbacks read the plane block in two 16-byte halves (a stalled
store forwarding, 3.7 percent of the run). #765 fixed: Cranelift's inliner keeps a callee's symbol reference, so a body
with a symbol is no longer inlined (`crates/yavs/tests/clift_inline.rs`). Details and measurements: decisions.md
"Cranelift tier, round 7".

Gates on 10d1b7fd: fmt, clippy `-D warnings` for default, `llvm`, `clift`, `llvm,clift`; `cargo test --workspace` (1 158)
and the feature sets (`llvm,clift` 1 297, `clift` 1 190, `llvm` 1 268, all test binaries counted); difftest 533/533 for
`yavs-clift`, `-clift-auto`, `-auto3`, `-mixed` at 1 and 4 threads (verifier on).

Full total (analyze, elaborate, run) of a Cranelift-only build (`--features clift`) and an LLVM build (`--features llvm`),
`tools/build-bench.sh` of 10d1b7fd and of 1f5c4249 (the sources of the `llvm` build are the same), against sim1; realbench `--compare --runs 5`, timing cores 10 and 11, host load
about 10 (round 6: about 4); last column the warm run instructions in G:

| test | Cranelift warm | LLVM warm | Cranelift cold | LLVM cold | G warm Cranelift / LLVM |
|---|---:|---:|---:|---:|---|
| poc_arith_addw | 1.07 | 0.97 | 1.13 | 1.19 | 95.6 / 85.5 |
| fletcher_bufferwriter_512 | 2.10 | 2.07 | 2.23 | 2.15 | 48.8 / 51.2 |
| neorv32_single_min | 0.78 | 0.78 | 0.87 | 0.87 | 30.4 / 30.5 |
| osvvm_axis_random | 0.15 | 0.15 | 0.37 | 0.37 | 2.6 / 2.6 |
| uvvm_sb_slv | 0.47 | 0.47 | 0.69 | 0.66 | 11.1 / 10.7 |
| uvvm_util_rand | 0.49 | 0.47 | 0.78 | 0.70 | 11.2 / 12.3 |
| uvvm_axistream_vvc | 0.77 | 0.76 | 0.87 | 0.83 | 37.9 / 35.7 |
| osvvm_axi4_random | 0.26 | 0.25 | 0.46 | 0.44 | 6.7 / 6.6 |
| vai_window_gen_interlock | 1.88 | 1.67 | 2.11 | 2.35 | 152.4 / 133.8 |
| geometric mean | 0.655 | 0.632 | 0.895 | 0.882 | |

Cranelift against LLVM: warm 1.037, cold 1.015 (round 6: 1.056 and 1.029). A run of the same table on 1f5c4249 (before
the inliner fix, cores 11 and 12) gave 1.014 and 1.011; the window generator's warm cell moves by 4 percent between
the two (1.80 and 1.88), as warm `auto` runs do (#673). `poc_arith_addw` warm went from 1.28 to 1.07 (LLVM 0.97).
What is left: the window generator (the native variants of the test runner's accessors, unchanged this round; eager
`get` and `set` take fewer cycles than LLVM's eager code, `unsafe_pop` and `decode` 1.4 and 1.5 times, and under
`auto` LLVM's code is faster), `procChecker` under `auto` (1.5 times the cycles of LLVM's O2 code), and the cold
library testbenches (`uvvm_util_rand` 1.11 times LLVM cold).

### clift: round 6, merge, ports and frame-mode code quality (2026-10-26, #481)

`spike/clift2` merged with main at 504f86ad (7871ae39: #756, #757, #583). Ported to the Cranelift lowering (decisions.md
"Cranelift tier, round 6"): the register-held spin counter of a process that never suspends (`process/no_wait_infinite`
passes, #763), element loops through `yavs_rt_eloop`, the fused layout word, and the drive batch of zero-delay scalar drives
(`llvm_drive_batch`: 153 kernel calls, was 918; the kept-drive skip of 9c54654a is reached through it). `--tier=clift-auto` gets the
layout slots, without which the merge of #756 made warm `poc_arith_addw` 3 percent more instructions and 14 percent more cycles.
Frame-mode code: guesses across fall-through block starts, register facts for dope reads, small copies as words, 32-byte
temporaries zeroed by stores, `index_muls`. Eager `--tier=clift` `poc_arith_addw` (claimed cores): 113.0 G to 109.9 G
instructions, cycles level; window generator 144.6 G.

Gates on 3fca4ea4: fmt, clippy `-D warnings` for default, `llvm`, `clift`, `llvm,clift`; `cargo test --workspace` (1 158) and the
feature sets (`llvm,clift` 452, `clift` 316, `llvm` 424); difftest 533/533 for `yavs-clift`, `-clift-auto`, `-auto3`, `-mixed`,
`-llvm`, `-auto` at 1 and 4 threads (the code of bda1771b, the same code without a comment change).

Full total (analyze, elaborate, run) of a Cranelift-only build (`--features clift`) and an LLVM build (`--features llvm`), both
`tools/build-bench.sh` of bda1771b, against sim1; realbench `--compare --runs 5`, timing cores 9 and 11; last column the warm run
instructions in G:

| test | Cranelift warm | LLVM warm | Cranelift cold | LLVM cold | G warm Cranelift / LLVM |
|---|---:|---:|---:|---:|---|
| poc_arith_addw | 1.28 | 1.01 | 1.31 | 1.21 | 97.2 / 85.4 |
| fletcher_bufferwriter_512 | 2.15 | 2.08 | 2.24 | 2.13 | 48.6 / 51.2 |
| neorv32_single_min | 0.80 | 0.79 | 0.88 | 0.88 | 30.4 / 30.4 |
| osvvm_axis_random | 0.14 | 0.14 | 0.37 | 0.37 | 2.6 / 2.6 |
| uvvm_sb_slv | 0.50 | 0.50 | 0.73 | 0.69 | 11.0 / 10.7 |
| uvvm_util_rand | 0.47 | 0.49 | 0.78 | 0.71 | 11.1 / 12.3 |
| uvvm_axistream_vvc | 0.86 | 0.79 | 0.91 | 0.87 | 37.5 / 35.7 |
| osvvm_axi4_random | 0.26 | 0.25 | 0.45 | 0.44 | 6.7 / 6.6 |
| vai_window_gen_interlock | 1.85 | 1.72 | 2.18 | 2.38 | 151.0 / 134.8 |
| geometric mean | 0.679 | 0.644 | 0.924 | 0.898 | |

Cranelift against LLVM: warm 1.056, cold 1.029 (round 5: 1.055 and 1.028). The merge would have cost Cranelift more (the #756
blocks); the ports and the code changes bring it back to level with round 5 and take warm instructions down (`poc_arith_addw`
99.1 G to 97.2 G, window generator 157 G to 151 G), not the wall time of the two compute tests. Where the rest is: per process,
Cranelift's code of the adder processes of `poc_arith_addw` takes about 1.35 times LLVM's instructions and twice its cycles
(frame-mode code stores every register it defines and reloads it after every join and callback; the base pointers live in
stack slots); in the window generator the native variants of the test runner's accessors (`get`, `set`, `unsafe_pop`, `decode`)
take 1.5 to 3 times LLVM's cycles, on the chains of object-table look-ups and the checked arithmetic around them.

`grlib_leon3mp`, run phase, `--tier=auto` warm, the same two builds on core 8 (realbench `--compare --runs 3`): Cranelift 42.6 s,
LLVM 35.2 s (0.83, instructions 0.97); sim1 11.9 s in a run of the Cranelift build alone (Cranelift 44.5 s warm, 48.8 s cold
there). Level instructions at 1.2 times the time: the same IPC gap as in the adder processes.

## performance (tiers against two reference simulators, 2026-10-19)

Measurement only (#125, #383, #385, #386, #395). yavs at `d76a08b5`, release build with the
`llvm` feature, `--tier=interp` and `--tier=llvm`, against two reference simulators (ref-A: JIT
based; ref-B: in-process machine-code generator, which elaborates at the start of the run). Host:
24 cores, heavily loaded by other jobs (load average about 16), so every cell is the median of 3 runs
(5 for designs 4 and 6b) with the min-max range of the total; tools were interleaved, one
simulation at a time, each under a memory cap. Analysis is not counted. Startup: yavs `run
--stop-time 0fs` (elaboration, and for `llvm` the compilation of every function); ref-A `-e`; ref-B
`-r --stop-time=0fs`; for designs 1 and 2 a generic empties the run instead. Simulation is total minus
startup (median of the per-run differences, floored at 0); peak RSS is that of the full run. Native
share is the static share of IR instructions the LLVM tier compiles inline (`--verbose`); the share
of instructions *executed* natively is not reported (#387).

Designs (outside the repository): (1) one process, integer and real loop, 20M iterations; (2a) 64
clocked 16-bit LFSR processes and one process XOR-reducing the 64 registers each cycle, 5000
cycles; (2b) the same without the reduction, 20000 cycles; (3) the maintenance testbench of a public
verification library's queue and scoreboard packages; (4) an AXI-stream testbench of another public
verification library, built from its project scripts; (5) all 314 `tests/corpus` testbenches
summed (analysis of all of them: yavs 5.7 s, ref-A 4.5 s, ref-B 0.6 s); (6) two testbenches of a
private accelerator project (window generator, 13 tests in one run; bias/requant, 14 tests in
one run). That project's long testbenches (the conv core, the top level) need an embedded
interpreter through foreign calls; yavs stops at the first one (the foreign subprogram is not in the yavs executable and
needs `--load=LIB`) and the references need the same library, so they are not measured.

| Design | Tool | Startup s | Simulation s | Total s | Peak RSS MB | Native share |
|---|---|---:|---:|---:|---:|---:|
| 1 compute loop, 20M iterations (integer and real) | yavs interp | 0.01 | 24.85 | 24.86 (21.05-27.11) | 57 |  |
|  | yavs llvm | 0.08 | 10.77 | 10.85 (10.37-11.37) | 75 | 68% |
|  | ref-A | 0.02 | 3.61 | 3.64 (3.61-3.71) | 57 |  |
|  | ref-B | 0.00 | 0.31 | 0.31 (0.31-0.31) | 7 |  |
| 2a signal-heavy RTL, 64 LFSR registers x 5000 cycles, XOR-reduced each cycle | yavs interp | 0.05 | 20.19 | 20.24 (19.13-20.43) | 65 |  |
|  | yavs llvm | 0.33 | 16.43 | 16.78 (16.70-16.86) | 81 | 54% |
|  | ref-A | 0.05 | 0.02 | 0.09 (0.06-0.09) | 78 |  |
|  | ref-B | 0.03 | 0.19 | 0.22 (0.21-0.22) | 11 |  |
| 2b same without the reduction, 64 x 20000 cycles | yavs interp | 0.04 | 16.96 | 17.01 (16.92-18.03) | 64 |  |
|  | yavs llvm | 0.26 | 14.69 | 14.95 (14.52-15.09) | 79 | 53% |
|  | ref-A | 0.06 | 0.07 | 0.12 (0.12-0.29) | 77 |  |
|  | ref-B | 0.03 | 0.49 | 0.52 (0.52-0.53) | 11 |  |
| 3 verification-library testbench, queues and scoreboard | yavs interp | 8.83 | 23.71 | 32.54 (30.56-35.42) | 313 |  |
|  | yavs llvm | 33.56 | 22.23 | 54.96 (49.88-56.42) | 459 | 57% |
|  | ref-A | 0.23 | 0.21 | 0.43 (0.43-0.47) | 144 |  |
|  | ref-B | 0.42 | 0.29 | 0.72 (0.66-0.73) | 50 |  |
| 4 verification-library testbench, AXI-stream (6524 checks) | yavs interp | 0.58 | 9.37 | 9.95 (8.92-12.78) | 104 |  |
|  | yavs llvm | 56.58 | 8.68 | 67.81 (63.10-74.12) | 514 | 59% |
|  | ref-A | 0.15 | 0.32 | 0.45 (0.38-0.66) | 159 |  |
|  | ref-B | 0.31 | 0.09 | 0.47 (0.37-0.48) | 47 |  |
| 6a project testbench, window generator (13 tests in one run) | yavs interp | 1.50 | 9.82 | 11.32 (11.24-13.53) | 122 |  |
|  | yavs llvm | 19.24 | 9.22 | 28.46 (26.46-29.19) | 314 | 55% |
|  | ref-A | 0.20 | 0.16 | 0.36 (0.36-0.37) | 137 |  |
|  | ref-B | 0.30 | 0.09 | 0.38 (0.36-0.39) | 42 |  |
| 6b project testbench, bias/requant (14 tests in one run) | yavs interp | 1.65 | 0.99 | 2.71 (2.52-3.19) | 116 |  |
|  | yavs llvm | 23.99 | 0.04 | 24.84 (20.24-26.32) | 311 | 53% |
|  | ref-A | 0.34 | 0.00 | 0.33 (0.26-0.53) | 120 |  |
|  | ref-B | 0.32 | 0.00 | 0.31 (0.29-0.37) | 41 |  |
| 5 corpus, 314 testbenches, summed | yavs interp | 8.9 | 0.5 | 9.4 (9.3-11.9) | 80 |  |
|  | yavs llvm | 169.0 | 4.6 | 173.7 (169.1-179.6) | 183 | 62% |
|  | ref-A | 7.4 | 7.0 | 14.3 (14.0-19.9) | 93 |  |
|  | ref-B | 1.2 | 1.3 | 2.5 (2.4-3.2) | 20 |  |

Findings.

- Interpreted, yavs is 7x (loop, against ref-A) to 80x (against ref-B) slower than the references,
  90x to 200x on the RTL designs and 20x to 75x on the testbenches. Two causes show in the data. The
  IEEE vector operators run as interpreted VHDL: in a loop of 100000 operations a 16-bit `unsigned`
  XOR costs 42 us, an `unsigned` `+` 250 us and a `bit_vector` XOR 1.8 us (same under
  `--tier=llvm`), and a clocked process activation costs about 12 us (#395, #385). And elaborating
  large library packages: 8.8 s of the 32.5 s in design 3 is startup, against 0.2 s in the references
  (#383).
- LLVM helps only pure computation: the loop is 2.3x faster (24.9 s to 10.9 s). The signal-heavy
  designs gain 1.0x to 1.2x, and for the testbench designs the simulation phase is within noise of the
  interpreter (design 3 23.7 s against 22.2 s, design 4 9.4 s against 8.7 s, design 6a 9.8 s against
  9.2 s). Compiling every function at O2 before time 0 costs 19 s to 57 s for 120k to 220k IR
  instructions (0.15 to 0.26 ms each) and 1.5x to 5x the memory, so the totals of the testbench
  designs are 1.7x to 9x worse than interpreted and the corpus goes from 9.4 s to 174 s. The tier
  pays only with tier-up of hot functions (#125) and a size threshold (#126).
- The LLVM tier compiles 53% to 68% of the IR instructions inline (62% over the corpus). What stays
  interpreted is mostly the memory and address kinds of #334: 71% over the corpus (Load 90311,
  AddrOf 53015, Stmt 23256, Alloc 21362, Store 19879, Copy 12175), 75% in design 3, 83% in the
  loop. Next the descriptor and layout kinds of #335 (DopeLen, LayoutOf, MakeDope, SubtypeCheck,
  14% over the corpus), then calls and returns (#336, 3%) and natives and reports (#337, 1%). Signal
  and wait instructions (#339), heap (#340), logic arrays (#338) and real kinds (#341) are each
  below 3% in these designs. The loop gains only 2.3x with 68% of its instructions inline, so the
  executed native share is probably lower than the static one.
- Not measured: the long accelerator testbenches (above), a third reference simulator, executed
  instruction shares, and design 2 at 1e6 clock cycles (yavs needs 17 s to 20 s for 5000 to 20000).

A study of how the reference simulators reach their speed, with ranked recommendations per crate
(native ieee fast paths with fallback, interpreter constant factors, kernel assignment and wake
fast paths, hot-only tier-up, start-up breakdown), is in `docs/design/performance.md`. No
recommendation is implemented yet.

## Synopsys testbenches: where the time went (#439)

Profile of the 20 `tests/corpus/synopsys/` testbenches (release build, `--threads 1`, user+system CPU seconds,
one run each, shared host): 88% of `tb_arith_addsub` and `tb_arith_cmp` was not ieee at all but the assert
state of `std.env`: every report appended an entry to the process's change log and every report replayed
that whole log (`Asserts::with`), so the 129k reports of one process in one time step were quadratic.
Consecutive counts are now folded into one entry (`crates/yavs-interp/src/env.rs`; corpus test
`env/assert_count_many`). With the ieee natives off (`YAVS_NO_NATIVE_IEEE=1`) the same testbenches cost only
20 to 25% more (`tb_arith_cmp` 1.27 s against 1.05 s, `tb_unsigned_cmp` 0.39 s against 0.32 s), so
`std_logic_unsigned`/`std_logic_signed` wrappers need no natives; the 1.0 to 1.4 s that remain in the two
largest testbenches is their own helper functions (`fill`, `rnd`, `img`, string building), interpreter work
for #431. Output is byte-identical (md5 of every run unchanged).

| Testbench | before | after |
|---|---:|---:|
| `tb_arith_addsub` | 6.05 | 1.46 |
| `tb_arith_cmp` | 4.01 | 1.07 |
| `tb_arith_conv_integer` | 0.02 | 0.01 |
| `tb_arith_conv_vec` | 0.43 | 0.22 |
| `tb_arith_ext` | 0.11 | 0.08 |
| `tb_arith_mul` | 0.61 | 0.34 |
| `tb_arith_shift` | 0.10 | 0.08 |
| `tb_arith_unary` | 0.04 | 0.03 |
| `tb_signed_addsub` | 0.53 | 0.28 |
| `tb_signed_cmp` | 0.62 | 0.33 |
| `tb_signed_conv_integer` | 0.02 | 0.01 |
| `tb_signed_mul` | 0.06 | 0.06 |
| `tb_signed_shift` | 0.05 | 0.05 |
| `tb_signed_unary` | 0.02 | 0.02 |
| `tb_unsigned_addsub` | 0.59 | 0.30 |
| `tb_unsigned_cmp` | 0.65 | 0.33 |
| `tb_unsigned_conv_integer` | 0.01 | 0.01 |
| `tb_unsigned_mul` | 0.07 | 0.05 |
| `tb_unsigned_shift` | 0.05 | 0.04 |
| `tb_unsigned_unary` | 0.01 | 0.01 |
| **total** | 14.05 | 4.78 |

## performance against the two reference simulators, interpreter tier, 2026-10-20

Measurement only. yavs at `f2ab69be` (release build, `--tier=interp`) against reference simulator A
(a 1.23 development build with LLVM 21, JIT based) and B, same method as the section of 2026-10-19
above: median of 3 runs (5 for designs 4 and 6b in the earlier section, 3 here), tools interleaved, one simulation at a time under a memory cap, analysis not counted, startup
is `--stop-time 0fs` (yavs, reference A with `--jit`), reference A `-e` (ahead of time) or reference B
`-r --stop-time=0fs`, simulation is total minus startup, peak RSS of the full run. Host: 24 cores
shared with about ten other agents, load average 17 to 28 during the runs, so read ratios, not seconds. Reference A is measured
two ways: the ahead-of-time way of the earlier section (`-e` then `-r`, two commands, the elaboration is
the startup) and the way the external test runner starts it (`-H 64m -e <top> --no-save --jit -r` in one
command), which is the reference for test-runner use. The compilation cache is warm (one earlier run
of each design; the design 3, 4 and 6 libraries were analyzed once and reused). Designs 1 to 6 are those of the section above
(5: the corpus now holds 354 testbenches; the first 314 rows are those of 2026-10-19, and the 20 new
`synopsys/` testbenches of the original IEEE arithmetic packages cost yavs 17.8 s of the 21.3 s total
against 2.4 s in reference A, reference B does not run them). The RTL set (`ff`, `lfsr`, `chain`, `fan`, `bus`) is the
kernel benchmark set of the kernel rounds. `tools/bench` `vunit_vc30` (30 test cases of the external test
runner's verification components, one `yavs run` each, warm cache), median CPU seconds of 3 runs: analyze 0.91,
elaborate 1.87, init 0.19, simulate 1.56, total 5.04 (7.20 at 666f5ee3, 20.9 at the init-phase section above), peak 119 MiB.

| Design | Tool | Startup s | Simulation s | Total s (min-max) | Peak RSS MB | yavs / tool |
|---|---|---:|---:|---:|---:|---:|
| 1 compute loop, 20M iterations | yavs interp | 0.01 | 4.50 | 4.51 (4.39-4.67) | 59 |  |
|  | sim1 `-e`, `-r` | 0.04 | 3.59 | 3.61 (3.28-3.89) | 57 | 1.2x |
|  | sim1 `--jit` | 0.01 | 3.65 | 3.68 (3.53-3.91) | 57 | 1.2x |
|  | sim2 | 0.00 | 0.29 | 0.31 (0.29-0.31) | 7 | 14.5x |
| 2a 64 LFSR x 5000 cycles, XOR-reduced | yavs interp | 0.01 | 1.19 | 1.23 (1.20-1.27) | 62 |  |
|  | sim1 `-e`, `-r` | 0.04 | 0.03 | 0.07 (0.07-0.07) | 78 | 17.6x |
|  | sim1 `--jit` | 0.02 | 0.04 | 0.06 (0.05-0.07) | 79 | 20.5x |
|  | sim2 | 0.03 | 0.19 | 0.22 (0.21-0.22) | 11 | 5.6x |
| 2b 64 LFSR x 20000 cycles | yavs interp | 0.01 | 2.95 | 2.99 (2.89-3.05) | 61 |  |
|  | sim1 `-e`, `-r` | 0.04 | 0.07 | 0.11 (0.11-0.12) | 77 | 27.2x |
|  | sim1 `--jit` | 0.02 | 0.08 | 0.10 (0.10-0.13) | 78 | 29.9x |
|  | sim2 | 0.03 | 0.50 | 0.53 (0.49-0.54) | 11 | 5.6x |
| 3 queue/scoreboard testbench | yavs interp | 1.67 | 5.57 | 7.37 (7.05-7.56) | 87 |  |
|  | sim1 `-e`, `-r` | 0.21 | 0.27 | 0.48 (0.42-0.59) | 144 | 15.4x |
|  | sim1 `--jit` | 0.25 | 0.26 | 0.51 (0.45-0.79) | 193 | 14.5x |
|  | sim2 | 0.45 | 0.29 | 0.73 (0.70-0.75) | 50 | 10.1x |
| 4 AXI-stream testbench | yavs interp | 0.13 | 1.97 | 2.10 (2.06-2.16) | 100 |  |
|  | sim1 `-e`, `-r` | 0.14 | 0.34 | 0.48 (0.48-0.50) | 159 | 4.4x |
|  | sim1 `--jit` | 0.08 | 0.36 | 0.44 (0.43-0.51) | 160 | 4.8x |
|  | sim2 | 0.38 | 0.14 | 0.51 (0.50-0.54) | 47 | 4.1x |
| 6a window generator (13 tests) | yavs interp | 0.22 | 1.60 | 1.79 (1.71-2.03) | 98 |  |
|  | sim1 `-e`, `-r` | 0.21 | 0.22 | 0.45 (0.43-0.54) | 137 | 4.0x |
|  | sim1 `--jit` | 0.15 | 0.19 | 0.33 (0.32-0.36) | 137 | 5.4x |
|  | sim2 | 0.36 | 0.05 | 0.40 (0.38-0.41) | 42 | 4.5x |
| 6b bias/requant (14 tests) | yavs interp | 0.21 | 0.02 | 0.22 (0.22-0.23) | 98 |  |
|  | sim1 `-e`, `-r` | 0.25 | 0.04 | 0.29 (0.25-0.39) | 119 | 0.8x |
|  | sim1 `--jit` | 0.17 | 0.00 | 0.17 (0.14-0.26) | 120 | 1.3x |
|  | sim2 | 0.36 | 0.00 | 0.33 (0.32-0.37) | 41 | 0.7x |
| ff: 4096 flip-flops x 2000 cycles | yavs interp | 0.03 | 11.46 | 11.49 (10.99-12.44) | 68 |  |
|  | sim1 `-e`, `-r` | 0.51 | 0.37 | 0.88 (0.85-0.90) | 122 | 13.1x |
|  | sim1 `--jit` | 0.24 | 0.46 | 0.69 (0.65-0.70) | 114 | 16.7x |
|  | sim2 | 0.03 | 0.75 | 0.78 (0.77-0.78) | 28 | 14.7x |
| lfsr: 512 x 5000 | yavs interp | 0.01 | 15.44 | 15.46 (13.96-17.21) | 61 |  |
|  | sim1 `-e`, `-r` | 0.04 | 0.15 | 0.19 (0.19-0.19) | 79 | 81.4x |
|  | sim1 `--jit` | 0.02 | 0.15 | 0.17 (0.17-0.17) | 78 | 90.9x |
|  | sim2 | 0.04 | 0.77 | 0.81 (0.81-0.85) | 16 | 19.1x |
| chain: 1000 stages x 2000 | yavs interp | 0.01 | 2.89 | 2.90 (2.84-2.95) | 61 |  |
|  | sim1 `-e`, `-r` | 0.07 | 0.11 | 0.18 (0.16-0.19) | 83 | 16.1x |
|  | sim1 `--jit` | 0.04 | 0.10 | 0.14 (0.13-0.15) | 82 | 20.7x |
|  | sim2 | 0.02 | 1.41 | 1.43 (1.39-1.45) | 10 | 2.0x |
| fan: 4000 x 5000 | yavs interp | 0.02 | 7.56 | 7.58 (7.48-7.76) | 67 |  |
|  | sim1 `-e`, `-r` | 0.37 | 0.98 | 1.35 (1.33-1.39) | 120 | 5.6x |
|  | sim1 `--jit` | 0.19 | 1.05 | 1.22 (1.21-1.25) | 113 | 6.2x |
|  | sim2 | 0.01 | 0.46 | 0.47 (0.45-0.47) | 11 | 16.1x |
| bus: 32 drivers x 20000 | yavs interp | 0.01 | 1.07 | 1.09 (1.06-1.09) | 60 |  |
|  | sim1 `-e`, `-r` | 0.02 | 1.12 | 1.14 (0.98-1.15) | 75 | 1.0x |
|  | sim1 `--jit` | 0.01 | 1.04 | 1.05 (0.83-1.12) | 77 | 1.0x |
|  | sim2 | 0.01 | 0.32 | 0.33 (0.32-0.35) | 9 | 3.3x |
| 5 corpus, the 314 testbenches of 2026-10-19 | yavs interp | - | - | 3.33 | 74 |  |
|  | sim1 `-e`, `-r` | - | - | 8.81 | 103 | 0.4x |
|  | sim1 `--jit` | - | - | 4.14 | 141 | 0.8x |
|  | sim2 | - | - | 1.32 | 19 | 2.5x |
| 5 corpus, all 354 | yavs interp | - | - | 21.32 | 74 |  |
|  | sim1 `-e`, `-r` | - | - | 11.71 | 103 | 1.8x |
|  | sim1 `--jit` | - | - | 6.53 | 141 | 3.3x |
|  | sim2 | - | - | 1.35 | 19 | 15.8x |

Reading it, against reference A (`--jit`, the primary reference for test-runner use), total time, yavs / A:
the compute loop 1.2x (2026-10-19: 6.8x), the clocked RTL designs 17x to 30x (2a, 2b; was 225x and 140x), the
library testbenches 4x to 15x (3: 15x was 76x; 4: 4.8x was 22x; 6a: 5.4x was 31x), 6b 1.3x (was 8x), the larger RTL set 1x
(bus) to 91x (lfsr: 512 clocked processes, 15 s against 0.17 s), and the corpus 0.8x on
the 314 testbenches of the earlier run (yavs 3.3 s, A 4.1 s with `--jit`, 8.8 s ahead of time) but 3.3x with the 20 arithmetic-package testbenches.
Against reference B the RTL designs are 2x (chain) to 19x (lfsr) and the library testbenches 4x to 10x. Peak memory: yavs 57 to 100 MB on every design, reference A 57 to 193 MB, reference B 7 to 50 MB.
The remaining gaps are per-activation costs of clocked processes on `std_logic_vector` signals (lfsr, ff, 2a, 2b), library start-up
(design 3: 1.7 s of 7.4 s) and the interpreted IEEE arithmetic of the original packages (the `synopsys/` testbenches).

## performance against the two reference simulators, interpreter tier, second run 2026-10-20

Measurement only, repeated on `380b4c5e` (release build, no `llvm` feature, so the interpreter) against
the section of 2026-10-20 above (yavs at `f2ab69be`), same designs, parameters and method: median of 3
interleaved runs, one simulation at a time under a memory cap, analysis not counted, startup is
`--stop-time 0fs` (yavs, sim1 `--jit`, sim2 `-r`), sim1 `-e` (ahead of time), simulation is total minus startup, peak RSS of the full run. sim1 is measured both ways of
the section above; `--jit` is the way the external test runner starts it (`-H 64m -e --no-save --jit -r`).
The compilation cache is warm. Host: 24 cores shared with other agents, load average 8 to 14 during the
runs, so read ratios, not seconds. Total seconds of yavs at `f2ab69be` are in brackets on the yavs rows.
The designs are the same files and generics as before (the RTL set uses the defaults of the kernel benchmark files).

Corpus rows: one timed run of every `tests/corpus` testbench per tool per rep, summed, 3 reps, median of
the sums (359 testbenches now; 314 of them are those of 2026-10-19, the rest includes the 20
`synopsys/` ones). The earlier corpus rows timed each run with a 10 ms clock, which rounds the many
runs below 5 ms to zero and understates the cheap tools; these rows time each run with a nanosecond clock
and subtract 4.7 ms per run for the timing wrapper (measured on `true`), so the corpus rows are not
comparable to the earlier ones to better than about 1 s. `tools/bench` `vunit_vc30` (warm cache), median
CPU seconds of 3 runs: analyze 0.67, elaborate 1.61, init 0.20, simulate 0.83, total 3.76 (5.04 at
`f2ab69be`), peak 69 MiB (119).

| Design | Tool | Startup s | Simulation s | Total s (min-max) | Peak RSS MB | yavs / tool |
|---|---|---:|---:|---:|---:|---:|
| 1 compute loop, 20M iterations | yavs interp | 0.00 | 2.52 | 2.52 (2.52-2.55) [4.51] | 11 |  |
|  | sim1 `-e`, `-r` | 0.06 | 3.05 | 3.10 (3.09-3.12) | 54 | 0.8x |
|  | sim1 `--jit` | 0.03 | 3.05 | 3.08 (2.88-3.13) | 55 | 0.8x |
|  | sim2 | 0.00 | 0.27 | 0.27 (0.27-0.27) | 7 | 9.3x |
| 2a 64 LFSR x 5000 cycles, XOR-reduced | yavs interp | 0.00 | 0.85 | 0.85 (0.82-0.91) [1.23] | 15 |  |
|  | sim1 `-e`, `-r` | 0.06 | 0.01 | 0.07 (0.07-0.08) | 74 | 12.1x |
|  | sim1 `--jit` | 0.03 | 0.02 | 0.05 (0.05-0.05) | 75 | 17.0x |
|  | sim2 | 0.03 | 0.16 | 0.19 (0.19-0.19) | 11 | 4.5x |
| 2b 64 LFSR x 20000 cycles | yavs interp | 0.00 | 1.89 | 1.89 (1.80-1.98) [2.99] | 14 |  |
|  | sim1 `-e`, `-r` | 0.09 | 0.04 | 0.13 (0.11-0.31) | 74 | 14.5x |
|  | sim1 `--jit` | 0.03 | 0.08 | 0.11 (0.09-0.12) | 74 | 17.2x |
|  | sim2 | 0.02 | 0.37 | 0.39 (0.39-0.42) | 11 | 4.8x |
| 3 queue/scoreboard testbench | yavs interp | 0.91 | 3.39 | 4.20 (3.69-4.53) [7.37] | 42 |  |
|  | sim1 `-e`, `-r` | 0.19 | 0.26 | 0.49 (0.41-0.53) | 146 | 8.6x |
|  | sim1 `--jit` | 0.57 | 0.02 | 0.58 (0.57-0.59) | 194 | 7.2x |
|  | sim2 | 0.34 | 0.26 | 0.60 (0.53-0.61) | 50 | 7.0x |
| 4 AXI-stream testbench | yavs interp | 0.09 | 0.89 | 1.03 (0.98-1.04) [2.10] | 52 |  |
|  | sim1 `-e`, `-r` | 0.12 | 0.25 | 0.38 (0.37-0.39) | 158 | 2.7x |
|  | sim1 `--jit` | 0.07 | 0.26 | 0.33 (0.30-0.34) | 160 | 3.1x |
|  | sim2 | 0.26 | 0.11 | 0.38 (0.37-0.38) | 47 | 2.7x |
| 6a window generator (13 tests) | yavs interp | 0.10 | 0.74 | 0.88 (0.84-0.96) [1.79] | 48 |  |
|  | sim1 `-e`, `-r` | 0.20 | 0.13 | 0.32 (0.31-0.36) | 135 | 2.8x |
|  | sim1 `--jit` | 0.11 | 0.16 | 0.26 (0.25-0.28) | 136 | 3.4x |
|  | sim2 | 0.25 | 0.07 | 0.32 (0.30-0.32) | 42 | 2.8x |
| 6b bias/requant (14 tests) | yavs interp | 0.10 | 0.00 | 0.10 (0.10-0.11) [0.22] | 48 |  |
|  | sim1 `-e`, `-r` | 0.17 | 0.02 | 0.18 (0.18-0.22) | 119 | 0.6x |
|  | sim1 `--jit` | 0.10 | 0.02 | 0.12 (0.11-0.13) | 120 | 0.8x |
|  | sim2 | 0.23 | 0.00 | 0.21 (0.21-0.23) | 41 | 0.5x |
| ff: 4096 flip-flops x 2000 cycles | yavs interp | 0.01 | 5.64 | 5.66 (4.93-5.72) [11.49] | 21 |  |
|  | sim1 `-e`, `-r` | 0.35 | 0.31 | 0.66 (0.66-0.67) | 122 | 8.6x |
|  | sim1 `--jit` | 0.18 | 0.29 | 0.48 (0.38-0.49) | 115 | 11.8x |
|  | sim2 | 0.02 | 0.48 | 0.51 (0.46-0.56) | 28 | 11.1x |
| lfsr: 512 x 5000 | yavs interp | 0.00 | 8.98 | 8.98 (8.89-9.33) [15.46] | 13 |  |
|  | sim1 `-e`, `-r` | 0.04 | 0.13 | 0.17 (0.17-0.17) | 80 | 52.8x |
|  | sim1 `--jit` | 0.02 | 0.13 | 0.15 (0.14-0.15) | 78 | 59.9x |
|  | sim2 | 0.03 | 0.62 | 0.65 (0.65-0.71) | 15 | 13.8x |
| chain: 1000 stages x 2000 | yavs interp | 0.00 | 1.75 | 1.75 (1.74-1.82) [2.90] | 14 |  |
|  | sim1 `-e`, `-r` | 0.06 | 0.09 | 0.15 (0.15-0.16) | 84 | 11.7x |
|  | sim1 `--jit` | 0.03 | 0.09 | 0.12 (0.11-0.12) | 82 | 14.6x |
|  | sim2 | 0.01 | 0.99 | 1.00 (0.97-1.08) | 10 | 1.8x |
| fan: 4000 x 5000 | yavs interp | 0.01 | 5.35 | 5.36 (5.11-6.22) [7.58] | 19 |  |
|  | sim1 `-e`, `-r` | 0.33 | 0.78 | 1.13 (1.05-1.20) | 120 | 4.7x |
|  | sim1 `--jit` | 0.17 | 0.80 | 0.97 (0.93-1.05) | 113 | 5.5x |
|  | sim2 | 0.01 | 0.38 | 0.39 (0.34-0.41) | 11 | 13.7x |
| bus: 32 drivers x 20000 | yavs interp | 0.00 | 0.82 | 0.82 (0.81-0.84) [1.09] | 12 |  |
|  | sim1 `-e`, `-r` | 0.02 | 0.69 | 0.71 (0.68-0.89) | 75 | 1.2x |
|  | sim1 `--jit` | 0.01 | 0.72 | 0.73 (0.69-0.98) | 75 | 1.1x |
|  | sim2 | 0.01 | 0.26 | 0.27 (0.26-0.29) | 9 | 3.0x |
| 5 corpus, the 314 testbenches of 2026-10-19 | yavs interp | - | - | 1.18 (1.04-1.22) [3.33] | 28 |  |
|  | sim1 `-e`, `-r` | - | - | 9.20 (9.05-9.39) | 103 | 0.1x |
|  | sim1 `--jit` | - | - | 4.90 (4.73-5.01) | 141 | 0.2x |
|  | sim2 | - | - | 1.98 (1.83-2.06) | 19 | 0.6x |
| 5 corpus, all 359 | yavs interp | - | - | 12.94 (12.48-14.00) [21.32] | 28 |  |
|  | sim1 `-e`, `-r` | - | - | 11.96 (11.82-12.29) | 103 | 1.1x |
|  | sim1 `--jit` | - | - | 7.02 (6.87-7.22) | 141 | 1.8x |
|  | sim2 | - | - | 2.17 (2.01-2.27) | 19 | 6.0x |

Reading it, total time, yavs now / sim1 `--jit` (at `f2ab69be`): the compute loop 0.8x (1.2x), 2a 17x
(20x), 2b 17x (30x), design 3 7.2x (14.5x), design 4 3.1x (4.8x), 6a 3.4x (5.4x), 6b 0.8x (1.3x), ff 12x
(17x), lfsr 60x (91x), chain 15x (21x), fan 5.5x (6.2x), bus 1.1x (1.0x), the corpus 0.24x on the 314
testbenches and 1.8x over all 359. yavs is faster than at `f2ab69be` on every row (0.45x to 0.75x of the
earlier seconds) and its peak memory fell from 57 to 100 MB to 11 to 52 MB, below sim1 (54 to 194 MB) on
every design; sim2 stays at 7 to 50 MB. Against sim2 the RTL designs are 1.8x (chain) to 14x (lfsr) and
the library testbenches 2.7x to 7x. The remaining gaps are unchanged in kind: per-activation cost of
clocked processes on `std_logic_vector` signals (lfsr, ff, 2a, 2b, chain), library start-up (design 3:
0.9 s of 4.2 s) and the interpreted IEEE arithmetic of the original packages (the `synopsys/`
testbenches, 11.8 s of the 12.9 s corpus total).

## kernel: cost of a process wake-up in clocked RTL (#440), 2026-10-20

Single-threaded (`--threads 1`), release build, interpreter tier, designs of the kernel benchmark set with
their default generics, against sim1 `-e --no-save --jit -r`. Profiles are sampling stops under a debugger
(no profiler on this host): per wake-up of 4096 one-bit registers the time went to the interpreter 55 to 70%,
the update phase 15 to 20%, batch bookkeeping about 10%, the wake filter of the clock's waiters 5 to 14%, and
two stalls the kernel caused. Table: median of 5 interleaved runs, total CPU seconds (user + system, start-up
included), main at `4e12507f` against this work; the host was loaded (load average about 23), so read the
ratios, not the seconds (a quiet run of the same binaries is 1.6x to 2x faster).

| Design | main | this work | sim1 | main / sim1 | this work / sim1 | this work / main |
|---|---:|---:|---:|---:|---:|---:|
| ff: 4096 registers x 2000 cycles | 8.42 | 5.54 | 0.71 | 11.9x | 7.8x | 0.66 |
| lfsr: 512 x 16-bit x 5000 | 5.07 | 3.75 | 0.22 | 23.1x | 17.0x | 0.74 |
| chain: 1000 stages x 2000 | 2.39 | 1.56 | 0.19 | 12.9x | 8.4x | 0.65 |
| fan: 4000 x 5000 | 5.87 | 3.88 | 1.26 | 4.6x | 3.1x | 0.66 |
| bus: 32 drivers x 20000 | 0.98 | 0.91 | 1.27 | 0.8x | 0.7x | 0.92 |

What changed, each its own commit: a 16-byte load of a signal reference the interpreter had just written in
pieces waited for the stores (store forwarding), at every signal read, edge query and wait; the references are now
checked inline in the interpreter's reads (`Ctx::first_code`, `Ctx::edge`), which was the largest single item
(ff 3.5 to 3.1 s, fan 3.1 to 2.5 s quiet). A scalar `std_ulogic` assignment built a waveform list, a logic value
and a reference list (three allocations): `Ctx::assign_code`. Dirty ranges that continue each other are one range
(one process per element of a vector made one driving-value search, diff and event per element). Waiters are
bucketed by 8 elements (64 above 4096) and the bucket lists exist only for signals something waits on, so an event on
one element of a vector with a process per element scans a few waiters, not 64 (chain 1.0 to 0.74 s); the range
test precedes the liveness test, an event with one range skips the search. The edge check of a clock is computed
once per trigger, and a one-list signal whose waiters all share one edge trigger that did not happen is not scanned
(the falling edge). Observers say which phases and value changes they read (`Observer::wants_phase`): a plain run
locked the shared-heap collector's mutex five times per delta (chain 7%). The batch order is packed and sorted
once (it was sorted twice, with a table lookup per comparison). One-source `std_logic` driving values are a copy,
the segment search starts at the last segment or its neighbour, and the one-thread update phase takes no slot lock.
The same one-layout-per-type memo made lfsr recompute the layout of a vector and of its slice on every execution
(about 20% of its time); #444 fixed that in the meantime, and the lfsr row includes it.

Checks at this work's tip (rebased on main of 2026-10-20): difftest corpus 369 of 369 at 1 and 4 threads
and at 1 and 4 shuffled threads, for the interpreter and the LLVM tier; the reference simulators' own regression
lists (`external.py --oracle --check`) pass (813 and 382 tests); the external runner's `check` suite 256 of 257 and
`verification_components` 572 of 579, the same 8 failures as main without this work (#462). Before the last
rebase `check` was 257 of 257 and `verification_components` 579 of 579, the `.pro` regression
(`tools/suites/pro.py`, 314 tests) all pass with results identical to main's, and `tools/suites/uvvm.py`
gave 78 pass, 33 fail, 26 skip, the same failures as before (two axistream configurations that were over 5 minutes
now finish); the output of all five designs was identical to main's at 1 thread, 4 shuffled threads and 8 threads.

What is left, from the same profiles: the interpreter is 55 to 75% of the time of these designs (each activation
of `ff` runs predecoded operations and native calls, `xor`, `nor` and `rising_edge`, with their argument pushes); the process, instance and slot data of thousands of instances are cache misses per activation
(#434); array and slice assignments of logic still build a value and two lists in `Machine::drive`; the update
phase still touches each element of a vector once per cycle (`mark_active`, `set_event`).

### Second pass on the wake-up cost (#440), 2026-10-21

Same designs and method, but profiled with `perf` and timed pinned to one performance core (core 4, min of 9 runs
of user + system CPU seconds, `--threads 1`, release build with the `llvm` feature) against sim1
(`-H 64m -e --no-save --jit -r`, min of 5, same core). "Before" is main at `a65c180b`, "after" is this work on top
of main with the interpreter's round 7 (`f19068e4`), so the interpreter's gains are in the after column.

| Design | tier | before | after | sim1 | before / sim1 | after / sim1 |
|---|---|---:|---:|---:|---:|---:|
| ff: 4096 registers x 2000 cycles | interp | 2.66 | 2.05 | 0.54 | 4.9x | 3.8x |
| | llvm | 2.52 | 1.93 | | 4.7x | 3.6x |
| lfsr: 512 x 16-bit x 5000 | interp | 1.71 | 1.50 | 0.16 | 10.7x | 9.4x |
| | llvm | 1.79 | 1.57 | | 11.2x | 9.8x |
| chain: 1000 stages x 2000 | interp | 0.92 | 0.60 | 0.15 | 6.1x | 4.0x |
| | llvm | 1.07 | 0.78 | | 7.1x | 5.2x |
| fan: 4000 x 5000 | interp | 2.29 | 1.76 | 1.01 | 2.3x | 1.7x |
| | llvm | 2.93 | 2.30 | | 2.9x | 2.3x |
| bus: 32 drivers x 20000 | interp | 0.39 | 0.34 | 0.68 | 0.6x | 0.5x |
| | llvm | 0.50 | 0.46 | | 0.7x | 0.7x |

What changed, each its own commit: a zero-delay assignment of one `std_ulogic` element to a driver with nothing
pending is appended to the waveform directly (`Ctx::assign_code`: the general path was about 1250 instructions of
the roughly 5000 of one `ff` activation); a process that waits for the same signals twice in a row without a
timeout keeps its waiters (resident waiters: no removal, no list push, no sorted batch, since the processes wake
in registration order); the wake scan asks interned trigger ids in the waiters instead of the process table, and
a clock whose waiters all share a trigger that holds is not asked per waiter; per-process waveform, sensitivity
and schedule lists are inline; the commit is one pass; the sort of the batch for the delta-limit report is only
done when that error happens; a one-signal update has no job lists; one `next_time` per cycle; no atomic exchange
per cycle for callouts; observers have phase masks of their own and the shared-heap collector is only called after
a serial process ran (`Observer::phase_after_serial`), which took a mutex and a hook per delta cycle from every
design without a serial process; a logic element is read and copied by its four plane bits (`Buf::code`,
`Buf::copy` of one element); an edge the kernel resumed a process for is known to `Ctx::edge` (the interpreter's
test repeated the kernel's answer); a one-element event updates and scans with shorter paths.

Where the time is now (perf, `ff`, `--tier=llvm`): about 28% in the kernel (commit and batch 5%, the cycle 3%, the
wake scan 3%, driver update 3%, assignment 3%, wait 2%, element codes 3%, activity stamps 2%), the rest in the
interpreter and compiled code entry (`TierHost::signal`, `yavs_rt_signal`, `sig_read_scalar`, scratch buffers).
`lfsr` is 60% interpreter (vector slice operations; the assignment of a 16-element slice still builds a value and
two lists in `Machine::drive`). `chain` is the one design that is mostly kernel (about 50%: delta cycle set-up,
the waiter scan of one process per element, the element update). Left: the update of a vector with one driver per
element touches each element four times per cycle (`apply_driver`, `compute_driving`, `update_effective`,
activity stamps); a static per-clock process list would remove the wake scan of thousands of resident waiters
(3% of `ff`).

Checks at this work's tip: difftest corpus 410 of 410 at 1 and 4 threads and at 4 shuffled threads for the
interpreter, `--tier=llvm` and `--tier=auto`; `external.py --oracle --check` on both reference lists (857 and 393
tests); the external runner's 11 suites all pass (1344 tests, `-p 2`); the `.pro` regression 314 of 314 under 2008 and
2019; the verification library suite 102 pass, 9 fail, 26 skip, as before.

### Third pass on the delta cost (#440), 2026-10-21

Same designs and method (core 4, min of 9 runs of user + system CPU seconds, `--threads 1`, release build with the
`llvm` feature, host load about 12, so only the ratios mean something) against sim1 (`-H 64m -e --no-save --jit -r`,
min of 7). "Before" is main at `7e1307cf`, which already has the interpreter's concatenation work.

| Design | tier | before | after | sim1 | before / sim1 | after / sim1 | instructions before / after |
|---|---|---:|---:|---:|---:|---:|---:|
| ff: 4096 registers x 2000 cycles | interp | 2.02 | 1.28 | 0.55 | 3.7x | 2.3x | 36.0G / 30.8G |
| | llvm | 2.43 | 1.95 | | 4.4x | 3.5x | |
| lfsr: 512 x 16-bit x 5000 | interp | 1.28 | 0.80 | 0.16 | 8.0x | 5.0x | 21.5G / 16.9G |
| | llvm | 1.24 | 1.01 | | 7.8x | 6.3x | |
| chain: 1000 stages x 2000 | interp | 0.58 | 0.34 | 0.15 | 3.9x | 2.3x | 10.5G / 8.0G |
| | llvm | 0.80 | 0.57 | | 5.3x | 3.8x | |
| fan: 4000 x 5000 | interp | 1.84 | 1.26 | 1.03 | 1.8x | 1.2x | 29.2G / 25.0G |
| | llvm | 2.60 | 2.08 | | 2.5x | 2.0x | |
| bus: 32 drivers x 20000 | interp | 0.38 | 0.28 | 0.67 | 0.6x | 0.4x | 6.3G / 5.8G |
| | llvm | 0.50 | 0.41 | | 0.7x | 0.6x | |

What changed (`docs/decisions.md` "Kernel round 6"): the clock keeps its wake list (no scan of thousands of resident
waiters), a one-element logic assignment is a quick pending transaction and a vector assignment to an idle driver skips
the schedule layers, the vector driver update is done in blocks (one-element finish by code compare, gathered driving
values, runs of events), dirty ranges merge in descending order too (this removed the per-cycle sort of 2560 ranges in
lfsr), one waiter list per element up to 1024 elements, and the delta loop reuses its next-event time and its buffers.
Tried and dropped (no gain): a prefetch of the process table, a known-edge table, a sorted-batch shortcut.

Left: the interpreter is 50 to 55% of `ff`, `fan` and `chain`; `Buf::code` (about 30 instructions per element read) is
7% of `ff`; `update_effective` stamps per changed element (6% of lfsr).

Checks: difftest corpus 413 of 413 at 1 and 4 threads, at 4 shuffled threads, for the interpreter, `--tier=llvm` and
`--tier=auto`; `external.py --oracle --check` on both lists (857 and 393); the external runner's 11 suites pass; the
`.pro` regression 314 of 314 under 2008 and 2019; the verification library suite 102 pass, 9 fail, 26 skip as before;
`cargo test --workspace` 759 passed.

### Kernel round 7: element reads and one-element events (#440, 2026-10-21)

Instructions (user, `perf stat`, core-pinned) and wall time (min of 5, core 4 of the performance cores, host idle) of the
same five designs, before (`9e1ecffc`) and after, `-H 64m -e <top> --no-save --jit -r` for the reference simulator (sim1):

| Design | tier | instr. before / after (G) | wall before / after (s) | sim1 (s) | before / sim1 | after / sim1 |
|---|---|---:|---:|---:|---:|---:|
| ff: 4096 registers x 2000 cycles | interp | 25.62 / 21.48 | 1.018 / 0.871 | 0.35 | 2.93x | 2.47x |
| | llvm | 31.20 / 23.63 | 1.190 / 0.966 | | 3.42x | 2.74x |
| lfsr: 512 x 16-bit x 5000 | interp | 16.40 / 14.88 | 0.645 / 0.607 | 0.11 | 5.97x | 5.68x |
| | llvm | 18.87 / 15.37 | 0.697 / 0.609 | | 6.46x | 5.70x |
| chain: 1000 stages x 2000 | interp | 7.61 / 6.35 | 0.274 / 0.237 | 0.10 | 2.67x | 2.33x |
| | llvm | 9.23 / 8.22 | 0.374 / 0.348 | | 3.64x | 3.42x |
| fan: 4000 x 5000 | interp | 20.17 / 19.61 | 1.035 / 1.013 | 0.69 | 1.49x | 1.47x |
| | llvm | 31.65 / 25.60 | 1.447 / 1.368 | | 2.08x | 1.99x |
| bus: 32 drivers x 20000 | interp | 5.67 / 5.67 | 0.234 / 0.233 | 0.46 | 0.51x | 0.51x |
| | llvm | 6.90 / 6.90 | 0.301 / 0.307 | | 0.66x | 0.67x |

(The reference simulator ran 30% faster on this idle host than in the round 6 table, taken on a loaded one, so ratios are
not comparable with it; the "before" column is `9e1ecffc` on the same host.)

What changed (`docs/decisions.md` "Kernel round 7"): a byte per element for the current value of logic signals (an element
read is a load; `Buf::code` was 30 instructions), sole-source one-element transactions finished when applied (chain), runs of
changed elements listed only for watched signals (lfsr), event stamps of a run in one pass, and smaller per-delta and
per-process savings. Compiled code reads the byte itself (`TierFrame::sigtab`) and answers `rising_edge` from the edge the
kernel resumed the process for, so `SigReadScalar` and the edge are no callbacks any more (ff under `--tier=llvm`: 31.2G to
24.3G instructions). The predecoder had lost the `LSetDrive` fusion of a scalar assignment (the generic `Machine::drive`
was 8% of ff) when the optimiser began to share registers and to memoise instance constants: the check "register used
once" counted whole-function uses; it now looks at the waveform buffer's uses between its `Alloc` and the `Drive` and at what
is live after (`pre::live_after`), and a frame load that feeds an element read is one op (`SigRead1Slot`). ff and chain lose
a tenth of their instructions to it.

What the profiles show now. Interp: the interpreter is 47 to 57% of the instructions of ff, 40% of lfsr and 30 to 41% of
fan and chain (dispatch, index arithmetic, instance constants); the rest is spread thinly (no kernel function above 6%).
lfsr still computes about eight instance-constant values per activation (`LoadInst`, `Pos`, `DopeLen`, `Bin` for
`r(i)(k)`) that the memoisation does not take (its block is not hoisted, cause not found). The cycle counts, unlike the
instruction counts, are partly memory bound: fan has 4 L1 misses and 0.5 L2 misses per activation, mostly in the commit loop (process slot,
400 bytes) and the interpreter's per-process state. Tried and dropped, no gain: a reordered and aligned process slot, a vector
pending list, a batch order check without the process table, counting repeated waits, prefetch, a byte table for plane writes.
`--tier=llvm` for clocked RTL still pays the fixed cost of an entry and two callbacks (`Drive`, `WaitOn`+`Suspend`):
fan 25.6G against 19.6G interpreted, chain 8.2G against 6.4G.

Checks: difftest corpus 414 of 414 at 1 and 4 threads and at 4 shuffled threads for the interpreter, `--tier=llvm` and
`--tier=auto`; `external.py --oracle --check` on both lists (857 and 393); the external runner's 11 suites pass; the `.pro`
regression 314 of 314 under 2008 and 2019; the verification library suite 102 pass, 9 fail, 26 skip as before; the
discrepancy catalogue checks green; `cargo test --workspace` 782 passed.

### Kernel round 8: vector assignments without lists or buffers (#440, 2026-10-21)

Core cycles and instructions (`perf stat`, `cpu_core/`, min cycles of 5 runs pinned to performance core 6, host loaded by other
agents, so wall time is not quoted) of the head-to-head designs, `main` at `d8271b29` against the same plus this round, and
the reference simulators on the same designs and core (sim1 `-H 64m -e <top> --no-save --jit -r`, sim2 `-r <top>`). Ratios are
cycles of yavs over cycles of sim1. Peak RSS is unchanged in every row (within 1 MiB; 12 to 47 MiB interp, 59 to 78 auto).

| Design | tier | cycles G before / after | instr. G before / after | sim1 / sim2 cycles G | after / sim1 |
|---|---|---:|---:|---:|---:|
| lfsr: 512 x 16-bit x 5000 | interp | 3.15 / 2.54 | 14.87 / 12.26 | 0.54 / 2.36 | 4.7x (was 5.8x) |
| | auto | 3.20 / 2.61 | 15.13 / 12.51 | | 4.8x |
| 2b rtl2: 64 x 16-bit x 20000 | interp | 1.66 / 1.32 | 8.09 / 6.66 | 0.37 / 1.47 | 3.6x (was 4.5x) |
| | auto | 1.62 / 1.34 | 8.09 / 6.80 | | 3.6x |
| ff: 4096 registers x 2000 | interp | 4.56 / 4.53 | 21.52 / 21.08 | 1.89 / 1.98 | 2.4x |
| chain: 1000 stages x 2000 | interp | 1.16 / 1.18 | 6.30 / 6.24 | 0.52 / 3.63 | 2.3x |
| fan: 4000 x 5000 | interp | 5.31 / 5.29 | 19.59 / 19.41 | 3.63 / 1.37 | 1.5x |
| bus: 32 drivers x 20000 | interp | 1.15 / 1.05 | 5.62 / 5.12 | 2.41 / 0.87 | 0.44x |
| ram: 16384 x 32 x 20000 | interp | 0.40 / 0.37 | 1.29 / 1.20 | 0.18 / 1.33 | 2.1x |

What changed (`docs/decisions.md` "Kernel round 8"): a zero-delay assignment of up to 64 logic elements to an idle driver is a
`Waveform::vquick` (plane words, no `Txn`, no value buffer); on a sole source of a plain signal the driving value is written when
the transaction is applied and the update phase only marks the activity of the joined runs and updates the effective value
(no `compute_driving` per segment). New `Ctx::assign_blocks` takes the plane blocks as they sit in the interpreter's frame, which
removes a heap vector and a `LogicVec` per vector assignment (malloc and free were a tenth of the cycles of 2b). The reference check
of `Ctx::read_into` is inlined. The per-element time stamps hold the time plus one, so a new array is zeros (a memory of 500 000
elements no longer writes 8 MB of markers at elaboration; 16% fewer page faults on a 2 million element memory).

Where the cycles are now: ff, chain and fan are unchanged because their assignments are scalar and already on the quick path; the
kernel is 25 to 30% of ff and fan (commit loop, wake, one-element finish) and about 60% of chain (a fixed ~1900 instructions per
delta cycle spread over `update`, `cycle`, `run_batch`, `take_woken` and the plane writes of one element, none above 15%). IPC is 4.5 to 4.9 in
all of them, so the instruction count is what matters. Tried and dropped in this round, no gain in cycles: hot-first `repr(C)` field order of
`Waveform`, `ProcState` and `Slot`; loads ahead of the commit loop and the run loop (software prefetch by loads); committing each
process right after it ran; setting `timed_out` and the known edge when the process runs. The memory-array path of `ram` is not kernel
bound: per clock cycle 47 000 instructions, nearly all in the interpreter (`to_integer`, the 32-bit adder in `ieee::small`, slice
reads); the kernel part is `update_effective` and `finish_signal` of a 32-element range of a 524 288-element signal.

### Kernel round 9: fewer stages per activation and per delta (#440, 2026-10-21)

`perf stat` (`cpu_core/`, min cycles of 5 runs pinned to performance core 2, host load average 7 to 8, so wall time is only
indicative), interpreter, `main` at `7d17d344` against the same plus this round; sim1 is the reference simulator with its JIT on the
same core (`-e --jit`, then `-r`). Peak RSS is unchanged in every row (within 1 MiB). Wall-time ratios from
`tools/bench/run.py --ref sim1 --tiers interp,auto --instructions --cores 2,3 --runs 5` are in brackets (interp / auto, yavs over sim1).

| Design | instr. G before / after | cycles G before / after | sim1 cycles G | cycles after / sim1 (was) |
|---|---:|---:|---:|---:|
| ff: 4096 registers x 2000 | 17.03 / 16.05 (-5.8%) | 3.88 / 3.69 (-4.7%) | 1.91 | 1.93x (2.02x) [wall 1.57 / 1.38, was 1.66 / 1.48] |
| lfsr: 512 x 16-bit x 5000 | 8.76 / 8.54 (-2.5%) | 2.01 / 1.91 (-4.7%) | 0.54 | 3.5x (3.7x) [3.5 / 2.9, was 3.3 / 2.7] |
| 2b lfsr64: 64 x 16-bit x 20000 | 4.66 / 4.64 (-0.4%) | 0.98 / 0.95 (-2.4%) | 0.34 | 2.8x (2.8x) [2.8 / 2.8, was 2.4 / 2.5] |
| chain: 1000 stages x 2000 | 5.33 / 4.96 (-6.8%) | 0.99 / 0.92 (-6.9%) | 0.52 | 1.8x (1.9x) [1.7 / 1.7, was 1.7 / 1.7] |
| fan: 4000 x 5000 | 18.27 / 17.21 (-5.8%) | 5.08 / 4.47 (-12%) | 3.64 | 1.23x (1.40x) [1.04 / 1.10, was 1.22 / 1.23] |

What changed (`docs/decisions.md` "Kernel round 9"): drivers of one signal due in a delta are applied in place from the due list (no
job list, sort or second pass); a resident process that waited again and only scheduled drivers is committed right after it ran, while
its slot is still in the cache (the commit loop of `fan` read every slot a second time from the L3: -12% cycles at equal
instructions); the resumption flags (`timed_out`, known edge) are set when the process runs, not in the wake; a falling clock edge
whose waiters are all held back by their trigger touches no wake list; the processes with several suspension points wake into their
own list and the rest need no order scan; the interrupt flag is a plain load per delta; a changed element with one resident waiter
is woken without a list scan; the unchanged-assignment test of a scalar asks the signal's code array before the driver slot.
Wall ratios on this loaded host move by +-10% from run to run; `lfsr` and `lfsr64` did not move in wall time. Determinism: the same
output at 1 thread, 4 threads and 4 shuffled threads (difftest 420 of 420 on the interpreter, `--tier=llvm`, `--tier=auto` and
`--tier=auto --tier-heat=1,0,0`, and 420 of 420 on a `gc-stress` build with the interpreter and `--tier=llvm`).

Where the cycles are now: the kernel is still ~800 instructions per `ff` activation (wake and commit ~250, apply and update ~250, the
rest in a dozen functions of 20 to 60 instructions) and ~1300 per `lfsr` activation (assign_planes_fast ~250, update_effective ~210 for
16 elements, read_into ~180); `chain` is ~1250 per delta (apply_driver ~190, take_woken ~150, cycle ~160, run_batch ~130, next_time and
live ~100). sim1 runs these designs in 33 to 49% of our instructions (all but `fan`, where it is level) at a similar IPC (4 to 5), so what is left is a
constant factor spread over the stages, not one hot spot; the process side (compiled entry, drive and wait callbacks: about a third of an `ff` activation)
is the larger remaining lever. Tried and dropped: lifting `FUSE_MAX` for unwatched signals (+9% instructions on `ff`), asking the
signal first in the vector unchanged test (+1% on `lfsr`), `as_mut_slice` in `Buf::put_code` (0), `#[inline]` on `Waveform::idle` (0).

Checks at the commit: `cargo fmt`, `cargo clippy --workspace --all-targets -D warnings`, `cargo test --workspace` (790 passed) and
`cargo test -p yavs-llvm --features llvm` (28); `external.py --oracle --check` on both lists (857 and 393) on the interpreter and
`--tier=auto`; the external runner's own suites (check, com, data_types, dictionary, logging, path, random, run, string_ops,
verification_components, and record_dict under 2019) all pass; the `.pro` regression 314 of 314 under 2008 and 2019 on both tiers; the
verification library suite 102 pass, 9 fail, 26 skip as before on both tiers.

### Kernel round 10: fewer instructions per element, per delta and per batch (#440, 2026-10-21)

`tools/bench/run.py BIN --ref sim1 --instructions --cores 4 --runs 5` (interpreter, performance core 4, median of 5,
host loaded by other agents, so wall time is indicative and the instruction ratio is the stable one), `main` at
`634452a2` against the same plus this round; ratios yavs over sim1.

| Design | instr. G before / after | wall ratio before / after | instr. ratio before / after |
|---|---:|---:|---:|
| rtl_ff | 16.05 / 15.72 (-2.1%) | 1.51 / 1.48 | 2.04 / 2.00 |
| rtl_lfsr | 8.55 / 8.25 (-3.5%) | 3.15 / 3.08 | 3.07 / 2.96 |
| rtl_chain | 4.97 / 4.30 (-13.5%) | 1.54 / 1.29 | 2.46 / 2.13 |
| lfsr64 | 4.65 / 4.54 (-2.4%) | 2.22 / 2.22 | 2.95 / 2.89 |
| rtl_fan | 17.21 / 16.87 (-2.0%) | 1.07 / 1.02 | 1.03 / 1.01 |
| lfsr64_xor | 1.91 / 1.88 (-1.6%) | 1.14 / 1.33 (cells under 0.1 s) | 2.56 / 2.51 |
| rtl_bus | 4.56 / 4.54 | 0.39 / 0.37 | 0.55 / 0.55 |
| rtl_ram | 0.99 / 0.96 | 1.17 / 1.20 (cells under 0.1 s) | 1.89 / 1.84 |

What changed (`docs/decisions.md` "Kernel round 10"): the effective value of a plain logic range on a block boundary
is updated in whole plane words and a register read of at most 64 elements reads one block; a one-element plane write
is a masked blend from a table of plane masks and a finished quick transaction takes the old code from the code
array; a delta of driver entries of one independent signal takes a short update with the quick transaction applied
in place, and one changed element with one resident waiter wakes it without a list scan; the inline batch has one
unwind guard and skips the commit loop when every process was committed after it ran.

Where the instructions are now (Intel PT, per unit): `rtl_ff` activation ~1910 (interpreter `run_ops` ~1040 and its
activation wrapper ~390, kernel ~500); `rtl_chain` delta ~2120 (interpreter ~790, kernel ~1300); `rtl_lfsr` activation
~3300 (interpreter ~1900, kernel ~1250). sim1 runs them in about 960, 1000 and 1090 instructions. On `rtl_ff` the
kernel is about half of that and the interpreter (55 to 80 instructions per op, dispatch and register bounds checks
included, plus its per-activation wrapper) is the larger lever; on `rtl_chain` and `rtl_lfsr` the kernel alone still
exceeds the reference's total, spread over a dozen stages of 20 to 150 instructions each.

Checks at the commit: `cargo fmt`, `cargo clippy --workspace --all-targets -D warnings`, `cargo test --workspace` (798
passed) and `cargo test -p yavs-llvm --features llvm` (32); difftest 420 of 420 at 1 and 4 threads and 4 shuffled
threads on the interpreter, and at 1 and 4 threads under `--tier=auto` and `--tier=llvm`; `external.py --oracle --check`
on both lists (857 and 393) on the interpreter and `--tier=auto`; the `.pro` regression 314 of 314 under 2008 and 2019;
the verification library suite 102 pass, 9 fail, 26 skip as before.

### Kernel round 11: direct paths for sole-driver registers, clock batches (#440, 2026-10-22)

Four commits (`docs/decisions.md` "Kernel round 11"): direct drivers (a zero-delay assignment to a driver that is the only
source of its elements of a plain logic signal is kept by the process, written to the driver and driving values at commit
and finished a plane word at a time in the next update, no queue entry, waveform lookup or dirty range; unread activity not
stamped; the commit loop walks only the processes not committed in the batch loop; the resumption edge stored once),
compact direct entries and inlined register reads, the clock's resident processes run as one batch without a wake each,
and smaller per-delta items (the code of a one-element commit kept, the wake scan's buffer only where it is used).

Instructions per unit (activation; a delta for `rtl_chain`), slope of 1x and 10x runs pinned to core 6 (5 for sim1),
`main` at `1b7d6695` (before) against `ce791a89` plus the last commit (after), ratio to sim1's per-unit count in brackets:

| Design | interp before | interp after | auto before | auto after | sim1 |
|---|---:|---:|---:|---:|---:|
| rtl_ff | 1852 (2.22) | 1633 (1.96) | 1721 (2.06) | 1551 (1.86) | 835 |
| rtl_lfsr | 3164 (3.28) | 2871 (2.98) | 2463 (2.55) | 2244 (2.33) | 965 |
| lfsr64 | 3423 (3.49) | 3072 (3.13) | 2949 (3.00) | 2675 (2.72) | 982 |
| rtl_fan | 842 (1.06) | 750 (0.95) | 883 (1.12) | 791 (1.00) | 791 |
| rtl_chain | 2132 (2.57) | 2078 (2.50) | 2166 (2.61) | 2115 (2.55) | 830 |

`tools/bench/run.py --ref sim1 --tiers interp,auto --instructions --cores 5 --runs 5` (normal length; wall / instructions,
yavs over sim1), before (`6e91a1e8`) to after: rtl_ff interp 1.33 / 1.95 to 1.09 / 1.72, auto 1.27 / 1.90 to 1.04 / 1.68;
rtl_lfsr interp 2.83 / 2.92 to 1.82 / 2.52, auto 2.33 / 2.62 to 1.47 / 2.25; lfsr64 interp 2.25 / 2.86 to 2.12 / 2.56, auto
2.25 / 2.89 to 2.00 / 2.51; rtl_fan interp 1.04 / 1.01 to 0.99 / 0.90; rtl_chain interp 1.33 / 2.14 to 1.00 / 2.08; rtl_ram
interp 1.20 / 1.82 to 0.86 / 1.53; lfsr64_xor interp 1.40 / 2.44 to 1.17 / 2.27 (wall cells under 0.1 s carry +-15%; the host
load moved between the two runs, so read the instruction ratios). neorv32 single core, 30 us, interpreter: 41.2 G to 40.2 G
instructions (-2.4%; the interpreter is about 80% of it, the kernel's largest item the signal reads).

Real testbenches (`tools/realbench/run.py --compare`, cores 6 and 7, `main` at `5816d068` with the four commits of this
round reverted against `main` itself; instructions B/A, interpreter / `--tier=auto`): sc5_fifo_repack 0.73 / 0.70,
neorv32_single_min 0.92 / 0.92, neorv32_dual_full 0.96 / 0.96, poc_cache_cpu 0.97 / 0.96, poc_arith_addw 0.97 / 0.97;
no slowdown above the tool's thresholds. The whole suite against `1b7d6695` (31 tests that pass on both) moved by a wall
geometric mean of 0.96 on the interpreter, with no test above 1.05 instructions on the interpreter.

Kernel per `rtl_ff` activation (Intel PT, per function, signal reads included): ~670 to ~510 instructions: the register update (commit, apply,
driving and effective value) ~240 to ~150, the wake ~50 to nearly nothing, the batch loop ~180 to ~150; assignment ~115 and the reads
~70 unchanged. What is left is the same dozen stages of 15 to 120 instructions, now each the cost of the generic storage
(`Buf` enum, `SmallVec` and bounds checks per access) rather than of a path that could be skipped; the target of ~350 per
activation needs either a storage layout without those layers for direct signals or a process side that hands the kernel
its assignments and waits in one call per activation (#507).

Checks at each commit: `cargo fmt`, `cargo clippy --workspace --all-targets -D warnings` with and without `--features llvm`,
`cargo test --workspace` (819 passed at the last), `cargo test -p yavs-llvm --features llvm` (36) and `llvm,gc-stress`;
difftest 431 of 431 at 1 and 4 threads, 4 threads shuffled, `--tier=auto` and `--tier=llvm`; `external.py --oracle --check` on
both lists (857 and 393) on the interpreter and `--tier=auto`; the `.pro` regression 314 of 314 under 2008 and 2019 on both
tiers; the verification library suite 102 pass, 9 fail, 26 skip as before on both tiers; VHPI conformance traces identical to
`main` (23 of 23).

### Kernel round 12: unchanged values, lagging driver values, one-process deltas, kernel copies (#440, #529, 2026-10-22)

Eight commits (`docs/decisions.md` "Kernel round 12"), ideas from the reference simulator's kernel written for ours: a direct
one-element commit of the value the driver and the signal already have writes nothing (its delta still happens); a direct commit
writes only the driving value, which is the value of its sole-source driver (`Signal::lag`, copied back before a force, release or
deposit); one changed element with one resident waiter wakes it at finish; a batch of one process runs without the batch
machinery; the first pending element of a signal skips the bit mask; runs of direct entries of one signal are committed with one
lookup; the hot per-process lists, pending transactions and `Signal::changed` are `Vec`s; plain concurrent copies
(`target <= source;` of signal names) run as the kernel's own `CopyProcess` when nothing observes activations (#529).

Instructions per unit (activation; a delta for `rtl_chain`), slope of 1x and 10x runs pinned to core 5, `main` at `6f2b527d` with the
eight commits reverted (before) against `main` (after), ratio to sim1's per-unit count (STATUS round 11) in brackets:

| Design | interp before | interp after | auto before | auto after | sim1 |
|---|---:|---:|---:|---:|---:|
| rtl_ff | 1590 (1.90) | 1495 (1.79) | 1538 (1.84) | 1443 (1.73) | 835 |
| rtl_lfsr | 3679 (3.81) | 3568 (3.70) | 3725 (3.86) | 3615 (3.75) | 965 |
| lfsr64 | 2869 (2.92) | 2756 (2.81) | 2524 (2.57) | 2414 (2.46) | 982 |
| rtl_fan | 737 (0.93) | 725 (0.92) | 781 (0.99) | 769 (0.97) | 791 |
| rtl_chain | 2064 (2.49) | 1684 (2.03) | 2106 (2.54) | 1726 (2.08) | 830 |
| lfsr64_xor | 4884 | 4771 | 4638 | 4530 | |

`rtl_lfsr` lost about a third to an optimiser change between the rounds (#534), so its ratio is above round 11's.
`tools/bench/run.py --ref sim1 --group h2h --tiers interp,auto --instructions --cores 5 --runs 3` (normal length, instructions
yavs over sim1, before to after): rtl_ff interp 1.60 to 1.58, auto 1.58 to 1.57; rtl_chain 2.08 to 1.70, auto 2.12 to 1.74;
rtl_lfsr 3.39 to 3.29; lfsr64 2.36 to 2.27, auto 2.39 to 2.21; lfsr64_xor 2.15 to 2.10; rtl_fan 0.89 to 0.88; rtl_ram 1.59 to 1.57;
rtl_bus, compute_h2h, xor_types level.

Kernel alone (a bench of hand-written processes with the same shapes, exact counts): a chain delta 1325 to 946 instructions, a clocked register activation 472 to 363 (each including 70 to 100 instructions of the hand-written process itself).

Real testbenches (`tools/realbench/run.py --compare --phases run --tiers interp,auto --cores 5`, the reverted build against `main`,
all 41 tests; instructions B/A, interpreter / `--tier=auto`): geometric mean 0.99 / 0.99, cpu 0.94 / 0.94, wide vectors 0.95 / 0.95;
neorv32_single_min 0.93 / 0.92, neorv32_single_cache 0.95 / 0.94, neorv32_dual_full 0.95 / 0.96, fletcher_bufferreader 0.93 / 0.93,
fletcher_bufferwriter_512 0.95 / 0.95, sc5_fifo_repack 0.96 / 0.94, sc5_timestamp 0.95 / 0.95. Summed wall time 0.96 / 0.95. Flagged
by the tool: `hdlm_axi_crossbar` and `vai_pe_array` under `--tier=auto` at 1.05 and 1.06 instructions (0.6 G tests, their wall time
0.83 and 0.80; the interpreter cells are level), which is the compile in the run, not the kernel.

Checks at each commit: `cargo fmt`, `cargo clippy --workspace --all-targets -D warnings` with and without `--features llvm`,
`cargo test --workspace` (831 at the last), `cargo test -p yavs-llvm --features llvm` (36) and `llvm,gc-stress` (36); difftest 434 of
434 at 1 and 4 threads, 4 threads shuffled, `--tier=auto` and `--tier=llvm`; `external.py --oracle --check` on both lists (857 and
393, interpreter); the `.pro` regression 314 of 314 under 2008 and 2019 (interpreter); the verification library suite 102 pass, 9 fail,
26 skip as before (interpreter); VHPI conformance traces identical to `main` with the round reverted (the known `fparam.fp_sigin`
difference to sim1 in both).

### Kernel round 13: delayed assignments, byte signals, time-step bookkeeping (#440, 2026-10-22)

Three commits (`docs/decisions.md` "Kernel round 13"). The head-to-head designs (4096 registers on one clock) hide what a clock
costs, and every testbench has one: a bare `c <= not c after 5 ns` took about 4000 instructions per edge against about 1000 in the
reference simulator, and `sc5_timestamp` (three clocks) was 65% kernel. (1) A delayed assignment of one element or up to 64 logic
elements to an idle driver is a quick transaction at `now + T` (it took the general path: `LogicVec`, transaction list,
`Waveform::schedule`). (2) An enumeration, integer, boolean or real register (one byte element of at most 8 bytes) is a
`Waveform::bquick` finished when applied, assigned from the interpreter without a `Value` (`Ctx::assign_bytes`) and read without a
reference (`Ctx::elem_bytes`). (3) The future queue is a three-word heap, `last_delta` asks for the next time once, `update` does not
build and read back a `Result`, several signals due together are updated without job lists, a copy process of a byte signal uses
`assign_bytes`. New `crates/yavs-kernel/examples/clk.rs` counts the clock path with hand-written processes.

Micro designs (interpreter; instructions G / cycles G, min of 5 on core 4, `main` 4a586fd3 against round 13, the reference simulator
last; cycles are noisy on a shared host, instructions are not):

| Design | before | after | sim1 |
|---|---:|---:|---:|
| three bare clocks, 6 ms | 14.32 / 4.23 | 9.54 / 2.44 | 3.50 / 1.00 |
| 2048 enumeration registers, 1000 cycles | 4.30 / 1.50 | 3.24 / 1.06 | 1.42 / 0.48 |
| 2048 integer counters, 1000 cycles | 6.88 / 2.01 | 5.16 / 1.39 | 1.91 / 0.57 |
| `examples/clk.rs` (kernel alone, 3 clocks) | 7.67 / 1.95 | 7.10 / 1.60 | |

Real testbenches (`tools/realbench/run.py --compare --phases run --tiers interp,auto --cores 4,5`, instructions B/A, interpreter /
`--tier=auto`): `sc5_timestamp` 0.74 / 0.74 (14.7 G to 10.9 G; wall 1.33 s to 0.95 s under `auto`, noisy), `neorv32_single_min` 0.92 / 0.92,
`poc_cache_cpu` 0.92 / 0.92, `uvvm_axistream_vvc` 1.00 / 0.92, `uvvm_sb_slv` 1.00 / 0.94, `osvvm_axi4_random` 0.99 / 0.99,
`olo_axi_master_full` 0.99 / 1.00; wall times of the short tests are noise on this host (load 18 to 75 during the runs). Head-to-head
designs unchanged to within 3% in instructions (`rtl_ff` 11.86 G to 11.89 G, `rtl_bus` 4.00 G to 3.89 G, `rtl_ram` 0.79 G to 0.74 G).

Left in the time-step path (kernel alone, 3 clocks): about 1900 instructions per clock edge against about 1000 for the reference
simulator's whole edge. It is spread over `update`, `run_batch`, `cycle`, the wake of the waiting process and the finish of the signal
(each 100 to 400, much of it stalls from small structs copied through memory); a native process for `x <= not x after T` would remove
the interpreter's 600 to 700 per edge and is not done.

Checks on the pushed range: `cargo fmt`, `cargo clippy --workspace --all-targets -D warnings` with and without `--features llvm`,
`cargo test --workspace` (853 passed), `cargo test -p yavs-llvm -p yavs --features llvm` (194), difftest 447 of 447 on the interpreter,
`--tier=auto` and `--tier=llvm` (1 and 4 threads), 4 threads shuffled, and `--tier=auto` with `--no-collapse-ports`; UVVM suite 102 pass,
9 fail, 26 skip as before; OSVVM `.pro` 314 of 314 under 2008 and 2019.

### Kernel round 17: coincident timed entries, resident big processes, wide direct entries (#440, 2026-10-23)

(`docs/decisions.md` "Kernel round 17".) Baseline first (a ptrace single-stepper over `examples/clk.rs` and windows of
real-world designs, exact per-function and per-line counts; `perf record` cycles and L1 misses of the suite; bench builds, core 8):
a bare clock edge is 933 kernel instructions (round 15's number; the ~1330 of the brief was round 14), a near-idle interpreted
activation (`rtl_fan`) 678 (`run_ops` 422 of which ~80 entry and ~30 exit, `run_quick` 103, `run_batch` 97, `InterpProcess::run` 55),
a near-idle compiled one (`--tier=llvm`, `rtl_fan`, before #634's leaner activation) 926 of which the compiled code itself is ~90
(after it 702: `run_quick` 160, `enter` 134, `run_batch` 101, `view_again` 80, `Process::run` 55). The kernel is 37 to 41% of the
cycles of `neorv32_single_min` and the fletcher testbenches, 63% of `sc5_timestamp`, 54% of `rtl_chain`, 29% of `rtl_fan`, 14% of
`rtl_ff`, 17 to 21% of `sc5_fifo_repack`, `poc_cache_cpu` and `hdlm_lfsr`. Ranked by instructions saved times breadth:

| Change | Commit | Instructions | Cycles |
|---|---|---:|---:|
| Several timed entries due together (clocks that edge together, timeouts on edges) stay on the timed path | 648741bf | `sc5_timestamp` 0.959 | 0.903 |
| Processes of up to 2048 registers keep their register file (was 256) | c0a614a1 | `neorv32_single_min` 0.990, `fletcher_bufferreader` 0.977, `fletcher_bufferwriter_512` 0.989, `poc_cache_cpu` 0.999 | 0.977, 0.951, 0.973, 0.977 |
| A vector of more than 64 elements assigned to a direct driver is one entry per plane word | 0ad2e8a1 | `fletcher_bufferwriter_512` 0.957 | 0.893 |
| The idle skip records its state on the pool path (4 threads skip what 1 thread skips) | 173fae51 | equal skips at 1 and 4 threads (neorv32 at 4 threads: wall 1.67 s to 1.41 s) | - |
| The lists every finish, update and wake of a signal tests are `Vec`s, not `SmallVec`s | 4e595ac8 | `neorv32_single_min` 0.990, `fletcher_bufferreader` 0.989, `rtl_chain` 0.989, `sc5_timestamp` 0.987 | level |

All 15 designs together (`main` before this round against the pushed tip, bench builds, minimum of 5 alternating runs): instructions
geometric mean 0.985, cycles 0.977 (`neorv32_single_min` 0.978 / 0.968, `fletcher_bufferwriter_512` 0.933 / 0.881,
`fletcher_bufferreader` 0.966 / 0.959, `sc5_timestamp` 0.948 / 0.879, `poc_cache_cpu` 0.994 / 0.979, `sc5_fifo_repack` 0.991 / 0.994,
`hdlm_lfsr` 0.998 / 1.007; the head-to-head designs 0.991 to 1.000 in instructions, `rtl_chain` 0.991, `rtl_ram` 0.992, cycles level
within the noise of aligned builds).
Measured and not done: merging the processes of a clock into one batch (the fixed part of an activation the merge could remove,
`run_batch` + `Process::run` + `run_quick` + `run_ops` entry and exit, ~340 instructions, is at most 5.1% of `neorv32_single_min`,
6.7% of `fletcher_bufferreader`, 2.9% of the writer, 5.1% of `poc_cache_cpu`, 1.5% of `sc5_fifo_repack`, 2.1% of `hdlm_lfsr`); the idle
skip for processes with variables (clocked processes outside the pure shape that did nothing: 330 k of 9.9 M activations in
neorv32, 0.5 M of 8.2 M in the fletcher reader, 0.25 M of 2.6 M in `sc5_fifo_repack`, 0 in `hdlm_lfsr`, whose variable changes at
every edge); the general update path for designs with implicit signals (at most 5% of the cycles of `osvvm_axi4_random`, nothing
elsewhere); a cheaper empty delta (28% of the cycles of `sc5_timestamp` but ~130 of ~2300 instructions each); `#[inline(always)]` of
`Machine::run_quick` (-4% instructions of `rtl_fan`, +0.3% of the CPU designs, cycles geometric mean worse); hoisting the storage
look-up out of the per-word update of wide buses and a flat record list (-0.8% of the 512-bit design, no cycles).

### Idle skip, round 5: definite assignment, record variables by field, debug code under a false generic (#521, #519, 2026-10-24)

(`docs/decisions.md` "Idle skip, round 5".) The hottest process of `fletcher_bufferwriter_512` (the reshaper's `ctrl_comb_proc`) is a
two-process-style state machine that the purity analysis refused for three reasons, now lifted: a helper variable assigned in the
branch that uses it, a record variable assigned field by field, and `report`s under `if rising_edge(clk) and DEBUG` with `DEBUG` a
constant false. Exact instructions, interpreter, bench builds: writer 74.17 G to 69.0 G (0.93; 139 K of its 600 K wakes skipped, each
worth ~41 K instructions with the drives and bus updates it avoids), `poc_fifo_ic_assembly` 0.82, `fletcher_bufferwriter_prim32` 0.91,
`neorv32_dual_full` 0.97, `neorv32_single_cache` 0.98, `neorv32_single_min` 0.99; no design worse (the small ones read 1.2 on a first run
of a new binary, which fills the compilation cache: three runs give 1.00). Checked with `YAVS_IDLE_SKIP=check` over the differential corpus
(511 tests) and the 42 real-world designs; new corpus test `delta/idle_vars`, perf design `idle_vars`.

### Fletcher writer and reader against the reference simulator: the whole picture (#519, #523, 2026-10-24)

Measured on `ae1826dd` plus the kernel round 20 below (bench builds with `--features llvm`; `perf record` of the whole run;
`tools/stepcount` windows of 60 (writer) and 300 (reader) delta cycles for the exact shares; `YAVS_COUNTERS`; `op-census`; core 13
for every timed run, the host at load 19 to 32). Under `--tier=auto` neither design compiles anything (`YAVS_AUTO_TRACE`: the hot
processes are combinational and the policy takes none; `ctrl_comb_proc` of the reshaper, 2238 IR, is 10% of the run interpreted),
so auto equals the interpreter.

**What the design does** (writer): 795 K delta cycles for 69.5 K clock cycles (11.4 deltas a clock), 12 process runs and 64 plane-word
updates (43 of them changing) a delta; about 35 wide signals (each a 512-bit bus or a record carrying one) change once a clock
(some twice: concatenations), so 4.2 M bus changes in all. The reference simulator moves the same values: 2.9 K instructions a bus
change with the kernel at 45% of its 26.7 G, the compiled processes 50%.

Share of the instructions of one delta cycle (stepcount, user instructions of the program file; libc is another 2.5%):

| layer | writer (99 K a delta) | reader (12.6 K a delta) |
|---|---:|---:|
| interpreter dispatch (`run_ops`) | 33.0% | 23.7% |
| kernel: commit and finish of signals | 21.6% | 17.8% |
| record layout, copies, gather and scatter in the interpreter (`layout_word(s)`, `lcopy`, `subtype_check`, `sig_gather/scatter`) | 15.9% | 14.7% |
| kernel: cycle, wake-up, process dispatch (`cycle`, `run_batch`, `run_quick`, `arm_readers`, ...) | 11.6% | 27.9% |
| process-side signal I/O (`assign_*`, `read_into`, `Buf::write_to`) | 10.6% | 10.0% |
| native `ieee` calls | 3.7% | 1.7% |
| hashing (record gather plans) | 0.3% | 0.7% |

Against the reference simulator (writer, instructions): signal commit and finish 19 G (before round 20) against 9 G, cycle and
wake-up 8.6 G against 2.8 G, processes 38 G interpreted against 13 G compiled. The op census of an 8 us writer window (12.2 M ops, the
interpreter's side): `Stmt` 7.1%, `SigVal` 5.0%, `FieldOf` 4.9%, `DopeLen` 4.4%, `CmpBr` 4.1%, `NewPair` 3.7%, `SubtypeCheck` 3.5%,
`BinRangeK` 3.2%, `LCopy` 2.9%, `FatRoot` 2.8%, `LayoutWord` 2.7%, `CallIeee` 2.4%; the pairs `FieldOf -> SubtypeCheck -> DopeLen -> LCopy`
(a record variable with array fields copied field by field: `v := ctrl_r;`) and `NewPair -> FatRoot -> CallIeee` (the operand
convention of the native `ieee` calls, #697) are 1.6 to 2.8% of the ops each. Records with generic-bounded array fields keep their
layout in descriptors, so every access is a `LayoutOf`/`FieldOf` where the reference simulator has a constant offset: folding them
in the specialised copies of processes (#678) is the largest item of the interpreter side (the layout layer alone is 14 to 16%).

Measured and not taken (all with exact instruction counts, `perf stat` on core 13): fat LTO with one codegen unit (writer +0.6%
instructions, cycles level; reader level), `-C target-cpu=x86-64-v3` (writer -2.8% instructions, -1 to 2% cycles; reader -0.1 to
-2% instructions, cycles +5% in one set: not shippable for generic hosts and not a clear win), `--threads 2/4/8` (6.5 s to 7.6 / 8.3 /
8.3 s: the 12 runs a delta, 0.6 us each, do not pay for a hand-off), a single-pass `canonical_changed` and an aligned `Buf::write_to`
(each 0.0% instructions: the compiler's loops were as short), a skip of plane entries that hold the value (see "Kernel round 20").

Totals against the reference simulator, `tools/realbench` with analysis (library part and own files), elaboration and run, core 13,
medians of 5, warm and cold cache, the two sessions with sim1 timed in each (yavs / sim1; the wall times of the two sessions differ
because of the host load, the ratio inside a session is the figure):

| test | run phase before (warm / cold) | run phase after | total before (warm / cold) | total after |
|---|---:|---:|---:|---:|
| `fletcher_bufferwriter_512` | 3.20 / 3.29 | 3.07 / 3.22 | 1.90 / 2.04 | 1.79 / 1.98 |
| `fletcher_bufferreader` | 1.96 / 1.87 | 1.82 / 1.82 | 0.94 / 1.01 | 1.01 / 1.11 |

After the idle skip of round 5 below and the record-signal work that landed meanwhile (main at `4d53f7a4`, same method, a third
session): writer run phase 2.59 / 2.60 (warm / cold), total 1.56 / 1.61, instructions 62.2 G against the reference's 26.7 G (2.33, was
3.15); reader run phase 1.73 / 1.73, total 0.83 / 0.90, instructions 23.2 G against 15.3 G (1.51, was 1.73). The change of this and the next
two rounds is about half of that; the rest is the record gather and scatter of #677 and #686.

The reader's instruction count did not change in the kernel round; its rows show session noise (the reference simulator's own run took
1.15 s in the first session and 1.66 s in the second, yavs 2.25 s and 3.02 s), and the totals of the two sessions mix the reference's
1.5 s of library analysis with run times that moved by 40%. Only the writer's rows are an effect of the change (instructions 0.92).
Totals under 1 or near 1 for the reader come from the library analysis of yavs (0.2 s warm) against the reference's 1.5 s.

### Kernel round 25: repeated skipped drives (#720, #667, 2026-10-27)

(`docs/decisions.md` "Kernel round 25".) A zero-delay drive of a port or shared driver that already holds a skipped (unchanged)
assignment of the same activation is skipped on the fast paths when it changes nothing either, instead of taking the general
assignment path. `grlib_leon3mp` (bench builds with `llvm`, warm, 3 interleaved runs on one claimed core): auto instructions -5.6%,
cycles -4.1%; interp -5.8% and -4.8%. `neorv32_single_min` and `fletcher_bufferreader` level (the path is not taken there).
`Stats::general_assigns` (`YAVS_COUNTERS`) counts the general path; perf design `drive_repeat`. `realbench --compare` (42 tests, run
phase): summed wall 0.99 (interp and auto), the other tests level. Fixed on the way: a force lost its events beyond the first updated
range when another plane word of the signal was committed in the same cycle (#789). Measured and dropped: skipping held words of a wide
assignment (`fletcher_bufferwriter_512` +4.4% instructions), a swap instead of a copy of a wide entry list (level).

### Kernel round 24: the skip test that keeps missing, pending words in element order (#734, 2026-10-26)

(`docs/decisions.md` "Kernel round 24".) The small-RTL designs still slower than sim1 under `--tier=auto` at head-to-head 6. Bench builds with
`--features llvm`, `--threads 1`; instructions of warm whole runs (`perf stat instructions:u`, `--tier=interp`, deterministic); cycles as
the minimum of 7 interleaved runs on one claimed core; main at 504f86ad against the round. The ratio to sim1 is `tools/bench/run.py --ref
sim1` (elaboration and run, analysis excluded), whose wall column (10 ms steps) did not move for these four designs; it is not the basis of
the head-to-head 6 full totals.

| design | instructions, interp | cycles, auto | cycles, interp | instruction ratio to sim1, auto (before, after) |
|---|---:|---:|---:|---:|
| rtl_lfsr | -3.9% | -1.9% | -0.5% | 1.59, 1.52 |
| lfsr64 | -4.2% | -3.9% | -2.9% | 1.70, 1.61 |
| rtl_chain | -2.6% | -1.0% | -1.4% | 1.66, 1.62 |
| rtl_ff | -1.8% | -3.5% | -2.2% | 1.22, 1.20 |
| neorv32_single_min | -0.5% | +0.6% to +1.7% | +0.6% to +1.7% | |

Also `delta_chain` -6.1%, `lfsr64_xor` -2.9%, `ff_rtl` -1.7%, `rtl_ram` -1.2% instructions; the other h2h designs level. `realbench --compare`
(run phase, 42 tests): wall geometric mean over the tests of 1 s or more 1.01 (interpreter) and 0.99 (auto), summed wall 1.00; the `cpu` category 1.01
(interpreter) and 1.02 (auto), neorv32 +1 to 3% wall (builds that each took one change back recovered part of it, not additively). What changed: a
driver whose skip tests (`direct_same`, `direct_same_planes`) miss 8 times in a row leaves them out for 248 assignments; the pending words of a
signal nothing watches are finished in commit order (the processes of a vector of registers commit in descending element order, and the sort
of a reversed list ran at every update); a vector entry within one plane word commits with one mask.

Where an activation of a compiled small clocked process goes (about 1100 instructions on `rtl_ff`, 1530 on `rtl_lfsr`; sim1 about 1000 to 1100
in total): the kernel about 200 and 450, the compiled body about 215 and 340, the host layers between the kernel and the compiled code about 680
and 600 (#766). `#734`'s narrow-signal mask does not apply to the four designs (each keeps its registers in one signal of more than 64
elements). The four clock-edge forms cost the same in the kernel; `wait until rising_edge(clk)` runs 128 more interpreter instructions per
activation (#767).

### Kernel round 23: assignments in pieces stay direct entries (#521, #733, #735, 2026-10-25)

(`docs/decisions.md` "Kernel round 23".) Instruction counts only (no timing cores): whole runs under `perf stat instructions:u`, one thread,
`--no-cache`, bench builds with `--features llvm`; main at 27842f12 against the round; windows of 100 delta cycles from cycle 20000 with
`tools/stepcount`.

| design | before (G) | after (G) | change |
|---|---:|---:|---:|
| neorv32_single_min | 36.900 | 33.051 | -10.4% |
| fletcher_bufferreader | 20.853 | 20.685 | -0.8% |
| poc_arith_addw | 149.297 | 148.496 | -0.5% |
| hdlm_lfsr | 64.248 | 64.098 | -0.2% |
| rtl_fan | 13.656 | 13.175 | -3.5% |
| rtl_chain | 3.360 | 3.348 | -0.4% |

One delta cycle of neorv32_single_min, program file: 70.4 K to 63.7 K instructions (libc, the `memcpy` of the general assignment path, 1.3 K to
0.4 K); the stages that are the kernel, `Ctx` and `CopyProcess` included, 31.7 K to 25.8 K. What went: the assignments of `port_req(i)` (a record of
an array assigned whole, then its `stb` field) that left the direct path 2.4 times a delta cycle (`Waveform::schedule`, `apply_driver`,
`compute_driving`, `update_effective`, `spill_out`: about 1800 instructions each). `realbench --compare` (run phase, interpreter and llvm,
all 42 tests, instructions): geometric mean 1.00 and 0.99, the `cpu` category 0.97 and 0.95, no other test beyond noise.

Left, by share of a neorv32 delta cycle (instructions a delta cycle): `finish_direct` 5.6 K (of which the arm loop `arm_readers_of` inlined
about 0.9 K), `cycle` 4.5 K (the arm loop about 1.7 K), `CopyProcess::run` 2.5 K (2.65 runs of about 10 parts),
`run_batch` 1.9 K, `commit_out` 1.5 K, `take_woken` 2.5 K (12 calls), `commit_planes_run` 1.3 K. The empty delta cycle after skipped
assignments is 389 instructions of a 2400-instruction cycle (kept, see the decisions entry).

### Kernel round 22: the per-delta bookkeeping of CPU-style designs (#519, #521, 2026-10-25)

(`docs/decisions.md` "Kernel round 22".) Instruction counts only (no timing cores): whole runs under `perf stat instructions:u`, one thread,
`--no-cache`, bench builds with `--features llvm`, main at 2cf30ee4 against the round; windows of 100 delta cycles from cycle 20000 (rtl_fan
20 cycles from 1000, rtl_chain 200 from 3000) with `tools/stepcount`. Kernel includes the `Ctx` callbacks of `process.rs`.

Whole run, instructions in G (main at a133e50c against the rebased round; the windows and the cycles were measured on 2cf30ee4 before the rebase); cycles are the minimum of 4 alternating runs on one loaded core of the first CCD (indications, no timing core
was used), main against the last commit:

| design | before | after | change | cycles |
|---|---:|---:|---:|---:|
| neorv32_single_min | 39.82 | 37.12 | -6.8% | -6.9% |
| poc_arith_addw | 156.74 | 149.61 | -4.5% | -4.5% |
| fletcher_bufferreader | 22.60 | 21.33 | -5.6% | -4.3% |
| hdlm_lfsr | 67.21 | 66.41 | -1.2% | -0.9% |
| rtl_chain | 3.369 | 3.288 | -2.4% | |
| rtl_fan | 13.479 | 13.479 | 0.0% | |

One delta cycle, instructions before / after (stepcount windows). The `Ctx` callbacks (`read_into`, the assign fast paths, `commit_code`,
`wake_sole`, `arm_readers_of`) are inlined into their callers by the last commits, so the per-function rows of the table before no longer
exist; the stages are summed into the interpreter with its `Ctx` callbacks and `CopyProcess`, and the kernel proper:

| stage | neorv32_single_min | poc_arith_addw | fletcher_bufferreader | hdlm_lfsr |
|---|---:|---:|---:|---:|
| interpreter, `Ctx` callbacks, `CopyProcess` | 45736 / 43216 | 37562 / 36266 | 7171 / 6845 | 13578 / 13481 |
| kernel (wake, finish, commit, `run_batch`, `cycle`, `update`, ...) | 27547 / 23781 | 8827 / 8079 | 5126 / 4767 | 2030 / 1940 |
| total | 73283 / 66997 | 46389 / 44345 | 12297 / 11612 | 15608 / 15421 |

Before, by stage (neorv32 / addw / fletcher reader / lfsr): `Ctx` reads and assigns and `CopyProcess` 11274 / 4071 / 1391 / 642, wake
(`take_woken`, `arm_readers`, `wake_sole`) 8083 / 497 / 542 / 103, finish (`finish_direct`, `update_word`) 5810 / 2866 / 1403 / 426, commit
5247 / 1567 / 748 / 371, `run_batch` and `order` 4033 / 1976 / 724 / 245, the `cycle` wake loop and idle filter 3399 / 1645 / 1100 / 403,
`update`, `last_delta`, `run_until` and libc 976 / 276 / 609 / 482.

The fixed costs of a delta (rtl_chain, one process, one bit, 1594 instructions: interpreter 708, `run_one` 193, `finish_direct` 176, `wake_sole`
116, `cycle` 115, `update` and `last_delta` and `run_until` 104, `assign_code_fast` 94, `commit_code` 80) and of an activation (rtl_fan, 667
instructions: about 100 in the `run_batch` loop, the rest the interpreter) hardly moved (rtl_chain 1594 to 1553 instructions: the callbacks, `wake_sole` and `commit_code` inlined); they are already one load or compare each. What
moved is the work per changed signal and per woken waiter (the wake order sort, the range tests of waiters and readers, the guard walk of
`arm_readers`) and the call overhead of the process-side callbacks.
The shares of the instructions of a whole run that are the kernel (including `Ctx`): 54% neorv32, 35% addw, 61% fletcher reader, 16% hdlm_lfsr,
11% rtl_fan, 66% rtl_chain. Left, by whole-run share: `finish_direct` 7% (about 410 instructions per changed word of a narrow signal, `update_word`),
`cycle` 6%, `assign_planes_fast` 4%, `commit_out` 4%, `run_batch` 4%, `Ctx::read_into` 3.4% (85 instructions a call, a third of them decoding
the `SigRef`).

### Waits on a clock edge, and what the idle skip leaves (#519, 2026-10-25)

(`docs/decisions.md` "Waits on a clock edge".) Fresh profile of `fletcher_bufferwriter_512` (interpreter, bench build, core 13; per-process
time from a local patch, not committed): `reg_proc`-style clocked processes 19% of process time at ~500 ns a run, the padder `comb` 12%,
the testbench's `bus_proc` and `input_proc` 11% each, the reshaper `ctrl_comb_proc` 9%, `last_stage_proc` 5%. `perf`: `run_ops` 31%,
`finish_words_lean` 6.5%, `cycle` 4%, `commit_planes_run` 3.8%. Over all 42 realbench designs the processes the idle skip refuses
and that mostly drive nothing are checkers with variables kept between activations (~0.5% of process time): no new idle-skip rule
pays. Built instead: `Inst::WaitEdge`, so `wait until rising_edge(clk) and x` wakes only on the edge. Instructions
(interpreter): `hdlm_lfsr` 0.98 (auto 0.94), `fletcher_bufferreader` 0.98, `fletcher_bufferwriter_512` 0.99, the rest 1.00;
`realbench --compare` of 42 tests: instruction and wall geometric mean 1.00 in interpreter and auto. Difftest 524 of 524 (interp, llvm,
auto, auto heat 1,0,0 at 4 threads, interp shuffled); new corpus test `delta/wait_edge`, perf design `wait_edge`.
Writer total against the reference simulator is unchanged (about 1.4x warm): the remaining gap is the interpreter dispatch, kernel
commit and record layout items of the whole-picture table above.

### Kernel round 21: cells for a wide bus with a waiter on a part (#519, 2026-10-24)

(`docs/decisions.md` "Kernel round 21".) The split of such a bus into kernel signals at the waiters' boundaries (the user's task) was
investigated and not built: the five buses concerned in `fletcher_bufferwriter_512` are driven whole by one process each, so a leaf
would map to several kernel signals in the drive, read and wait paths of the interpreter and the compiled tier, waves, VHPI, DAP and
coverage. Instead the kernel keeps the signal whole and lists its changes per cell (`Signal::cuts`, `note_cells`): the offsets where
waited ranges start and end cut the signal, and a changed word notes the hull of its changes once per cell. Wake-ups are those of
the exact runs; value-change observers still get exact runs; `YAVS_CELLS=off` restores the run lists. Bench builds, core 13, minimum
of 6 alternating runs: writer 0.956 instructions, 0.946 cycles (61.5 G to 58.7 G); reader 1.002 / 0.990; `realbench --compare`
(interpreter and auto, run phase): see decisions. Difftest 520/520 on interp, llvm and auto at 1 and 4 threads; OSVVM and UVVM suites
unchanged; the writer's FST byte-identical.

### Kernel round 20: a lean wide bus is finished in one pass (#519, 2026-10-24)

(`docs/decisions.md` "Kernel round 20".) `Signal::finish_words_lean` finishes the pending words of a lean bus with no codes and no
activity stamps in one loop, and a whole plane-word entry is committed without shifts. `fletcher_bufferwriter_512`, bench builds,
core 13, minimum of 5 alternating runs: 83.8 G to 77.1 G instructions (0.918), 30.1 G to 28.6 G cycles (0.951); `realbench
--compare` (auto, warm and cold, 5 runs): the writer 6.44 s to 6.25 s warm, the other five tests level (instructions 1.00; the host
ran at load 19 to 32, so wall time of the others is noise); all 42 tests, interpreter: instructions geometric mean 0.98, only the
wide-vector category moves. Skipping a plane entry that holds its value already was built and measured: it costs as much as it saves.

### Coverage keeps the count-neutral skips (#707, 2026-10-24)

(`docs/decisions.md` "Coverage keeps the count-neutral skips".) Under `--cov` the skipping of assignments of the held value
(`skip_unchanged`) stays on for every kind, and the idle skip stays on for toggle coverage alone (off when statement, branch,
condition or expression counters are on). The databases are byte-identical to the runs with both off (`tests/cli/cov_*.vhd`,
`rtl_ff`, `rtl_lfsr`, neorv32 30 us, `sc5_fifo_repack`, `osvvm_axi4_random`, `uvvm_sb_slv`; 1 and 4 threads; interp, llvm, auto).
Instructions (interpreter, millions, before / after): neorv32 30 us toggle alone 10046 / 6725, default kinds 14141 / 13644;
`sc5_fifo_repack` default kinds 51891 / 50896; the small designs move by under 2%. This reverses the rule of the same day that
coverage observes activity.

### Kernel round 19: single-bit drives, Signal layout, record leaves, skipping and observers (#676, #519, #677, 2026-10-24)

(`docs/decisions.md` "Kernel round 19".) No speed-up landed: the three ranked ideas were built and measured and none lowered the
instructions of the set (output identical, counters equal but the entry kinds): neighbouring one-element drives of a driver as one
vector entry (`hdlm_lfsr` -0.9% instructions, `sc5_fifo_repack` +1.9%), the same at commit time (-0.07% / +0.5%), lean `*_now`
entries (-0.25% / +0.15%), `Signal` fields ordered hot first with `repr(C)` (L1 data misses -3%, instructions +0.1 to 0.3%, cycles
within the noise of a loaded host). The kernel is ~5% of the cycles of a `grlib_leon3mp` window; the 680 instructions a record
leaf are interpreter work (gather and scatter instructions, #677). Added: the counter `skip_signals` and tests that the skipping
of assignments of the held value follows the observers (a VHPI plugin, `'active`, `'last_active`, `'transaction`,
`'quiet`; a waveform file does not observe activity and keeps it: open question for the user).

### Kernel round 18: lean event bookkeeping of wide signals, exact-instruction tools (#519, #440, 2026-10-24)

(`docs/decisions.md` "Kernel round 18".) The user's decisions from round 17's questions: `RESIDENT_REGS` stays 2048, no kernel
forwarding of plain signal copies for now, a different event bookkeeping for wide buses, the measuring tools committed.
`tools/stepcount` (a ptrace single-stepper in C, a Python report with `nm` and `addr2line`; `tools/stepcount/AGENTS.md`) counts the
user instructions of a window of a run per function and per source line on a `tools/build-bench.sh` build; its first use here: a
bare clock edge of `examples/clk.rs` is 943 instructions, the 3 M-instruction window of `fletcher_bufferwriter_512` put 18% in
`finish_direct` and showed the event records of `update_word` (the pruned `SmallVec` list, ~7 records per block with random data) as
the cost to remove (an upper bound with no records and no `last_value` blend: -6.2% instructions, -9% cycles).

| Change | Commit | Instructions | Cycles |
|---|---|---:|---:|
| A wide signal that no `'last_event`/`'last_value` can ask the history of single elements of keeps only each block's last event | 2cb97aca | `fletcher_bufferwriter_512` 0.963, `fletcher_bufferreader` 0.999, `sc5_fifo_repack` 0.996, `neorv32_single_min` 1.001 | 0.921, 1.000, 1.017, 0.972 |

Core 8 (bench builds, minimum of 5 alternating runs; the host was loaded, cycles of single designs move by a few percent between
runs: `sc5_fifo_repack` +1.7% and `sc5_timestamp` +1.0% are within that, with equal instructions): `sc5_timestamp` 1.000 / 1.010,
`poc_cache_cpu` 1.000 / 1.004, `hdlm_lfsr` 1.000 / 1.011; cycle geometric mean of the seven 0.990. Whole real-world suite
(`tools/realbench --compare`, 42 tests, interpreter, core 8): instruction geometric mean 1.00 (only the writer moved), wall 0.98,
no slowdown by the suite's own criterion. Head-to-head designs: instructions level (geometric mean 0.997), CPU time 0.998
(9 interleaved runs of `xor_sig`, `compute_h2h`, `rtl_ff`, `rtl_chain`, `lfsr64`).
Checks on the pushed tip: fmt, clippy with and without llvm, `cargo test --workspace` (1013), the llvm tests (298), difftest 504 of 504
on interp, auto and llvm at 1 and 4 threads and on interp with `YAVS_LEAN_EVENTS=off` and with `YAVS_IDLE_SKIP=check` (debug build with
the llvm feature). On the release bench build one rebase earlier (before the cache FORMAT bump): difftest shuffled and with
`--no-collapse-ports`, the reference simulators' own tests (oracle) 992 and 455, OSVVM 314 of 314 (2008 and 2019), UVVM 131/0/6.
The FST of `wide_bus_attributes` and `wide_bus_lean` is byte-identical lean and with `YAVS_LEAN_EVENTS=off`.
Measured and not done: lazy unpruned records (instructions 0.999); no `last_value` blend for lean signals (-0.9% instructions,
-0.7% cycles of the writer, no sound rule); leaning designs with OSVVM or UVVM (their utility packages apply `'last_event` to
parameters: needs reachability over the lowered program).

### Idle activations of pure clocked processes, GUARD in the update phase (#440, #464, 2026-10-23)

(`docs/decisions.md` "Kernel round 16", "GUARD changes in the cycle of its condition".) A clocked process whose activation is a
function of signals and constants (no variable, call, report or delayed assignment) stays suspended at an edge when none of the
signals it reads had an event since its last activation; `YAVS_IDLE_SKIP=check` runs the activations it would skip and compares,
`off` runs them all. Instructions / cycles with the skip on against off (aligned bench builds, cores 10 and 11): `neorv32_single_min`
0.745 / 0.689, `fletcher_bufferreader` 0.80 / 0.82, `fletcher_bufferwriter_512` 0.97 / 0.96, `sc5_fifo_repack` 0.96 / 0.96, the head-to-head designs
1.00 (processes that always change stop being tracked after 128 activations). GUARD changes in the cycle of the event on a
signal of its condition (the guard process runs in the update phase); #464 is closed.

### One edge-test instruction for every clock-edge idiom (#632, #589, #590, #591, 2026-10-23)

`rising_edge`/`falling_edge` (IEEE, BIT, BOOLEAN), `x'event and x = '1'`/`'0'` either way round, BOOLEAN `x'event and x`,
`not x'stable and ...` and the level of `wait until x = '1'` are one IR instruction, `Edge`, in the interpreter and the
LLVM tier (`docs/decisions.md` "One edge-test instruction"). Instructions per process per clock cycle (rtl_ff-style,
1024 processes, 500 to 1500 cycles, `--tier=interp` / `--tier=llvm`, before to after, identical for `std_logic`,
`std_ulogic`, `bit` and `boolean` unless noted): `rising_edge` / `falling_edge` 1421 / 1132 to 1420 / 1113 (BIT and
BOOLEAN 1860 / 1140 to 1420 / 1115); `'event and = '1'` or `'0'`, either order 1798 / 1124 to 1420 / 1115 (BIT, BOOLEAN
1835 / 1135); `not 'stable and = '1'` 6490 / 5952 to 1420 / 1115; BOOLEAN `'event and clk` 2711 / 2045 to 1420 / 1117
(it is an edge trigger now); `wait until clk = '1'` 1451 / 1105 to 1410 / 1110; `wait until rising_edge(clk)` 1413 / 1127
to 1410 / 1108 (BIT and BOOLEAN 1841 to 1410). Tests: `edge_idioms.rs`, `edge_ir.rs`, `yavs-sem/tests/edge_shapes.rs`,
`compiled_edges.rs`, work-counter design `perf/edge_forms.vhd`.

### Kernel round 15: edge spellings, clock generators that re-arm, wide buses and CPU clocks (#440, 2026-10-23)

(`docs/decisions.md` "Kernel round 15".) (1) Every spelling of a clock edge gets the edge trigger: `wait until clk = '1'`,
`not clk'stable and clk = '1'`, `rising_edge`/`falling_edge` of BIT and BOOLEAN, `clk'event and clk = '1'` on BIT, and an enable
written before the edge (`tests/corpus/process/edge_forms.vhd`, std_logic, std_ulogic and bit). (2) A clock generator
(`ToggleProcess`) re-arms its next transaction in the kernel and its process stays suspended: a bare edge 1300 to 912
instructions. (3) Ranked by instructions saved times breadth after exact per-function counts of one clock edge, the
chain delta, a register activation and the real-world suite (tracer over `examples/clk.rs` and `yavs run`): neorv32's clock
had 375 waiters under more than four edge triggers and never used its wake cache; the 512-bit pipeline stages of the
fletcher writer noted ~64 runs per update; the assignment path scanned the run's entry lists. Instructions / cycles
(bench builds, cores 10 and 11): `neorv32_single_min` 0.936 / 0.957, `fletcher_bufferwriter_512` 0.915 / 0.938,
`sc5_timestamp` 0.918 / 0.933, `poc_cache_cpu` 0.990 / 0.963, `sc5_fifo_repack` 0.999 / 0.967, head-to-head designs 0.988 to 0.998.
Kernel share of what is left (`fletcher_bufferwriter_512`, 117 G instructions): `Signal::update_effective` 12% (the plane
words of the wide buses: records of `blk_event`, runs, codes), `take_woken` 5%. Not done: the reader's `'event` forms
(`x'event and x = '1'` costs 25% more instructions than `rising_edge(x)` per activation in `rtl_ff`, stepped out of the
interpreter's fast loop), `not x'stable` (an implicit signal per occurrence, 4.4 times `rising_edge`), conditional
generators (`clk <= not clk after T when running else ...`, 24 of ~650 in the corpora).

### Kernel round 14: clock generators in the kernel, the clock edge path (#440, 2026-10-22)

(`docs/decisions.md` "Kernel round 14".) Ranked from the round 13 list after an exact per-function count of one clock edge (a
single-stepping tracer over `examples/clk.rs`, ~2030 kernel instructions per edge): (1) the clock generator `x <= not x after T;`
is a kernel process (`ToggleProcess`, built by the elaborator when `T` is static and the signal is `std_ulogic`, BIT or BOOLEAN),
(2) the edge path of the kernel (a cycle with one due driver in the future queue is applied in place, no `run_batch` for an empty
postponed list, a clock with a few resident waiters wakes them without the scan): 2032 to 1330 instructions per edge,
kernel alone. Item 3 of the list, publishing the running process once per batch, is in `yavs-interp`'s activation and left to
that owner.

Instructions G / min cycles G, interpreter, core 4 (`main` 788fb084 against this commit; `perf stat`, min of 7, interleaved):

| Design | before | after |
|---|---:|---:|
| three bare clocks, 3 ms (`std_logic`, `std_ulogic`, 8 ns to 12 ns periods) | 4.77 / 1.09 | 2.59 / 0.62 |
| a `std_logic`, a BIT and a BOOLEAN clock and a counter process, 3 ms | 4.58 / 1.39 | 3.37 / 1.03 |
| `examples/clk.rs`, one clock, kernel alone (`ToggleProcess` in both) | 1.524 | 0.982 |

Real testbenches (`tools/realbench/run.py --compare --phases run --tiers interp,auto --cores 4,5`, instruction ratio): `sc5_timestamp`
0.77 / 0.77 (10.94 G to 8.42 G; wall 0.74 s to 0.57 s), `hdlm_lfsr` 0.96 / 0.92 (wall 0.97 / 0.90), `fletcher_bufferreader` 0.98 / 0.98,
`poc_cache_cpu` 0.99 / 0.99, `poc_uart_rx` 0.97 / 0.97, the rest of the 14 tests 1.00 (`neorv32_single_min`, `olo_*`, `hdlm_axi_*`,
`osvvm_axi4_random`, `sc5_fifo_repack`, `fletcher_bufferreader`); one cell of `neorv32_single_min` ran 1.22 slower in wall at the
same instruction count and was noise (`perf stat` cycles of four builds within 1% on 100 us of it, repeated). The suite's kernel share
is 15 to 20% (`run_ops` is 38 to 54%): what is left of the edge path is spread over `cycle` (the waiters of a clock with several
processes: ~25 instructions per waiter and ~12 per wake), `run_batch` (~100 per process of a batch), the plane writes of the
update (~130) and the heap.

(3) Direct byte transactions (item 5 of round 13): a zero-delay assignment to an enumeration, integer, boolean, real or time
register that is the only source of its signal is committed at the end of the run like a logic one (`OutB`, no queue entry, no
apply, no dirty range). Instructions of the head-to-head and internal designs, `main` before this commit against it (`perf
stat`, one run each, core 6): `ff_rtl` 0.212 G to 0.117 G, `rtl_ram` 0.783 G to 0.689 G, `lfsr64` 3.423 G to 3.306 G, `rtl_bus`
0.98, `resolved_bus` 0.97, `delta_chain` 0.98, `rtl_chain`, `rtl_ff`, `rtl_lfsr`, `rtl_fan` within 1%. 64 integer counters on one
clock, 1000 cycles: 245 M to 225 M instructions, per activation 3830 to ~3500 of which the interpreter has ~2650 (the kernel
part ~1150 to ~850).

Whole real-world suite (`realbench --compare`, `main` before this round b5ab7b91 against the pushed f932bbbb, 33 tests that pass on
both, `--phases run`, cores 4 and 5): geometric mean of the instructions 0.98 (interpreter) and 0.98 (`--tier=auto`); `fsm` 0.91
(`sc5_timestamp` 0.74), `hdlm_lfsr` 0.96 / 0.92, `poc_cache_cpu` 0.99, `neorv32_*` 0.99, every other test 0.98 to 1.00; no test above
1.00 in instructions (wall times of two tests ran 1.11 and 1.41 slower under a loaded host at 0.96 and 0.98 of the instructions).

Measured and not taken: the per-element event stamps are half of `update_word` of a 32-bit register (~150 of 600 kernel
instructions per activation), but records for signals of 8 to 64 elements cost more when the data is random (see the decision);
signal fields in hot-first order (with and without cache-line alignment) did nothing for neorv32. What the cycle costs per
activation that is left, from the exact counts: `run_batch` ~100 to 220 (per process of a batch: resumption, context, the
clocked commit), the assignment entry ~130, `commit_out` ~90 with the move into `directb`, `cycle` ~110, `finish_direct` ~80.

Checks: `cargo fmt`, `cargo clippy --workspace --all-targets -D warnings` with and without `--features llvm`, `cargo test --workspace`
(869 passed), difftest 451 of 451 on the interpreter, `--tier=auto` and `--tier=llvm` (1 and 4 threads), 4 threads shuffled and
`--tier=auto` with `--no-collapse-ports`; `external.py --oracle --check` 857 (sim1) and 393 (sim2); OSVVM `.pro` 314 of 314 under
2008 and 2019; UVVM suite 130 pass, 1 fail, 6 skip as before.

## lib (`crates/yavs-lib`)

Done: library directories (`yavs-lib.toml` manifest + `_YAVS_LIB` lock, rule 14), `NAME:PATH`
mapping, atomic manifest updates under an exclusive lock, shared-lock reads that never write,
on-demand closure query (unknown library/unit with suggestion, cycles, stale hashes), multi-process
concurrency test. The standard is stored per file. The analyzer supplies `UnitRecord`s (kind, name, dependencies) per file. A directory with a manifest from before the rename to yavs, or of another format version, fails with L0004 (delete the library, analyze again).

The library is an index (`yavs-lib.toml`, format 3: files in analysis order, hash, stamp, `partial`, the units each defines) plus a record per source file (`yavs-lib.d/<digest>.toml`: names and dependencies), #599 (user decision 2026-10-23): a call reads the index and the records it needs and writes only its own records and the index. A warm `analyze` of one file into the 246-file satcat5 library: 17.4 to 9.1 million instructions, 2.5 to 1.8 ms of wall; the index is 99 KB where the manifest was 244 KB. Realbench `analyze` phase (41 tests, one file per call): warm instructions x0.80 and wall x0.87 of before, cold x0.97 and x0.96.

The manifest is renamed into place without an fsync and not rewritten when unchanged (#561, user decision 2026-10-23): a cold `analyze` of a tiny file went from 6.0 ms wall (2.9 ms CPU) to 2.9 ms (2.8 ms CPU).

## kernel (`crates/yavs-kernel`)

LRM 14.7 cycle against an abstract `Process` trait, tested with hand-written processes: time with
runtime resolution and checked arithmetic, drivers with transport and inertial (reject limit)
preemption per element, resolution per source segment, `'event`/`'active`/`'last_event`/
`'last_value`/`'last_active`, force/release/deposit, postponed processes, delta limit, parallel
evaluation with ordered commit (threads and shuffle invariance tested on 1000+ processes),
`SimControl` with run/step, interrupt, value-change bus and phase callbacks. Design in
`crates/yavs-kernel/DESIGN.md`.

Kernel asks of docs/design/ir.md, all done: K1 uncollapsed ports with conversions in the update
phase (no extra delta; a port of several parts in different signals converts all of them in one
call, `Builder::port_parts`, #74), K2 kernel-maintained `'stable`/`'quiet`/`'delayed`/`'transaction`, K3
`'driving_value`, K4 `'driving`, null transactions and guarded `bus`/`register` signals with
disconnection time, K5 null-range signals, K7 bit-plane storage for logic signals (bytes for
everything else). K6 (buffered user-file streams) was dropped: disk files are the interpreter's
own and their callers serial (#82). Serial marking reaches the kernel through `Process::serial`,
so shared-variable, protected-type and file users run in order at any `--threads` (#107). The order
of the processes of one cycle is fixed (#252): elaboration order at initialisation; then the
processes with one suspension point in elaboration order, then the others
(`Process::several_waits`) in reverse, signal-resumed before timeout-resumed; concurrent
procedure calls (`Process::concurrent_call`) before the other several-wait ones (#281); a process a
signal resumes after earlier resumptions runs after them, deeper instances first, then in elaboration order (#417, #462). Observers
schedule and cancel wake-ups at an absolute time (`Hook::wake_at`, `Hook::cancel`,
`Observer::wake`, #268): a wake-up gives a cycle at its time with the usual phase callbacks,
forces and deposits from it apply in that cycle, a wake-up in the past is E3005, and the
simulation does not end as idle while one is pending (scenario tests, thread invariance at 1, 4
and 8 threads).

Benchmark (`cargo run --release -p yavs-kernel --example bench -- 10000 <work>`), 24-core host
under load from other agents (runs vary by about 15%), 8 worker threads. "Before" is the byte-only
kernel of the first round, run back to back with "after".

| case | threads | before | after |
|---|---|---|---|
| 10000 counters, work 0 | 1 | 3.3 s (3.0 M events/s) | 3.6 to 3.8 s (2.7 M events/s) |
| 10000 counters, work 0 | 8 | 3.1 to 3.3 s | 3.2 to 3.8 s |
| 10000 counters, work 1000 | 1 | 7.1 s (1.41 M events/s) | 7.8 s (1.29 M events/s) |
| 10000 counters, work 1000 | 8 | 4.3 s (2.35 M events/s) | 4.7 s (2.14 M events/s) |
| 1024-bit std_logic bus, 4 drivers, 10000 edges | 1 | 0.36 s (28 M resolved elements/s) | 0.115 s (89 M/s) |
| 1024-bit std_logic bus, 4 drivers, 10000 edges | 8 | 0.82 to 0.87 s | 0.33 to 0.39 s |

Before, the bus was 1 byte per element with a per-element resolution callback; after, it is
bit-planes with plane-wise resolution (3x faster). Scalar byte signals pay 10 to 15% for the
storage enum and the port/implicit machinery. With trivial processes the serial update and
commit phases dominate, so threads gain little.

Parallel update of independent signals (#103: no port, no composite group, one task per
signal). Same host and load (load average 12 to 29), median of 3 to 6 interleaved runs, before
against after:

| case | threads | before | after |
|---|---|---|---|
| 10000 counters, work 0 | 1 / 4 / 8 | 5.9 / 5.8 / 5.1 s | 5.4 / 3.6 / 3.9 s |
| 10000 counters, work 1000 | 1 / 4 / 8 | 13.0 / 8.4 / 7.8 s | 12.0 / 7.6 / 5.5 s |
| 5000 std_logic signals x 2 drivers | 1 / 4 / 8 | 5.6 / 5.4 / 5.0 s | 5.5 / 3.6 / 4.6 s |

The queue drain, port and group signals, wake and commit stay serial (DESIGN.md "Open
questions").

## ir and interp (`crates/yavs-ir`, `crates/yavs-interp`, M2)

`yavs-ir`: Program/Func/Block/Inst with 32-bit ids and a `StmtId` per instruction, runtime type
descriptions with descriptor-based layouts (bit-plane logic arrays, records, arrays with dope),
compact constants, natives by NATIVE.md symbol, foreign references, flag propagation
(`MAY_SUSPEND`, `TOUCHES_SHARED`), resume numbering, a linear verifier and a text dump
(`crates/yavs-ir/README.md` says what is stable). `yavs-interp`: `InterpProcess` implements the
kernel `Process` (heap frame stack, waits inside procedures), checked arithmetic through
yavs-std, checks, signal ops mapped 1:1 to `Ctx`, natives (`now`, `stop`, `finish`,
`to_string`/`'image` of scalars and, 2019, of arrays and records with the object form `x'image` (#360)), VHPIDIRECT calls of scalars, generational heaps with precise-root GC (descriptor-aware, deallocated slots quarantined until a suspension or, in a long activation, a recycle between instructions, #355; the shared heap, where serial processes allocate, collected between cycles from the shared variables, the objects elaboration made and every serial process's frames, #303),
serial marking, kernel-free `call` / `call_shared` for elaboration (allocations with the shared state, at any call depth, in the shared heap, #246), protected objects,
std.textio (read/write of every textio type, linear in the line length over many calls,
readline/writeline/tee, hread/oread/sread, justify), TEXT file ops including the 2019 ones, std.env (getenv, dir_*, tool and version
strings, time conversions, resolution_limit, the 2019 time_record functions, call paths and
the VHDL assert state), VHPIDIRECT calls with scalars, arrays,
records and access values (copy-back included), signal references of several parts in
different signals (`SigParts`, `SigSlice`, #71), runtime errors with the VHDL stack as
`SimError::trace`, and debug hooks in the style of commercial simulators (docs/design/ir.md 3.4).

Tests (`cargo test -p yavs-ir -p yavs-interp`): 11 end-to-end designs run through the kernel at
1 and 4 threads with identical traces (clock and counter, delta ordering, wait forms with a
wait inside a procedure, overflow with stack trace, bit-plane ops across blocks, access types
and stale handle, GC roots, natives, foreign call, elaboration call, serial marking), 5 debug
tests (conditional line breakpoint with hit condition, step over, step in with frame and locals,
global step across processes, signal breakpoint and process table), verifier and layout tests.
Also 5 foreign marshalling tests, 5 textio/file/env tests and a protected-type test.
Not done: `Elab*` instructions (yavs-elab), records with runtime layouts beyond `LayoutOf`,
the PSL natives (out of scope, `lib/NATIVE.md` section 4), `SetVhdlAssertFormat` with a non-empty format.

std.reflection (`yavs-interp/src/reflect.rs`, `yavs-lower/src/reflect.rs`): all 123 mirror
methods and `'reflect` (value and subtype mirrors, built from the value and its descriptor;
private-heap mirror objects traced by the GC). End to end through `yavs run` for every class:
enumeration, integer, floating, physical (TIME in resolution-limit units, `yavs run --resolution`, #253), record, array
(multi-dimensional, unconstrained parameters, null), access, file and protected mirrors, of
variables, constants, signals, ports and type marks, with mirror variables and generic
subprogram instances (`tests/corpus/reflect`, 12/12 at 1, 4 and 4 shuffled threads; 6 against
the reference simulator, 6 `ref=hand` where it is wrong or crashes). Scalar subtypes whose range
is not static (generic or parameter bounds) get their range at the `'reflect`, also as element,
index or designated subtypes. The CLI has no resolution option yet, so TIME mirrors are
exercised end to end in femtoseconds only (other resolutions: a unit test).

- A `report` in a function run without a kernel (elaboration time) prints `** Note: (init): msg` on stderr and goes on; severity failure stops (#299; difftest `report/elab_report`); it is counted in the VHDL assert counts, so an error exits 1 at the end (#349; `report/elab_report_error`, catalogue `elab-time-report-error`). Calls replayed on fresh package state (a session rebuild, building the simulation) print and count their reports no more (#390; `report/elab_report_replay`).

### Shared heap collector: overhead measurement (#303, 2026-10-19)

Release builds, `origin/main` (84bd5988) against the commit before the collector (0a89b92f),
busy 24-core host (load about 30), 5 alternating repetitions per case, median CPU seconds
(user+sys) and [min-max]. Collector counters from a temporary instrumented build (not
committed); no profiler was available (`perf_event_paranoid` 4).

| Case | Before | After | Delta | Collections | GC time | Peak RSS before / after |
|---|---|---|---|---|---|---|
| a. no access types (counter, shift register, 5000 clocks) | 5.34 [4.7-6.0] | 5.59 [5.0-5.7] | +4.7% (noise) | 0 shared, 5 private | 0 | 18 / 18 MB |
| b1. 2M textio lines, one activation, file output | 3.72 [3.6-4.0] | 3.73 [3.4-4.1] | +0.3% | 0 | 0 | 551 / 543 MB |
| b2. same, serial process (touches a protected object) | 4.74 [4.5-6.0] | 4.52 [4.4-4.8] | -4.6% | 0 | 0 | 551 / 543 MB |
| c1. 100k scoreboard push/check | 21.8 [18.0-23.8] | 21.5 [20.1-22.8] | -1.5% | 15 | 0.65 ms | 36 / 37 MB |
| c1b. scoreboard with a 50k-item backlog plus churn | 19.9 [17.0-23.5] | 20.1 [17.3-23.0] | +0.6% | 1 | 11.8 ms | 48 / 50 MB |
| c2. sparse memory model, 10k words x 3 passes (33 MB live) | 17.7 [12.3-19.8] | 17.3 [13.8-19.2] | -2.6% | 2 | 91 ms (0.6%) | 294 / 313 MB |
| c3. coverage model, 256 bins, 30k samples | 17.5 [16.1-19.6] | 18.6 [15.9-20.7] | +6.7% (noise) | 0 | 0 | 38 / 38 MB |
| d. library testbench with queues (scoreboard) | 31.6 [30.0-36.6] | 31.8 [30.8-34.1] | +0.6% | 2 | 16.7 ms (0.05%) | 314 / 266 MB |

Findings:

- The overhead is not measurable: every delta is inside the run-to-run spread (10-30%) on the
  shared host. The counters bound it: collection time is at most 0.6% of a run (c2), typically
  below 0.05%; `publish_shared` (run at every process suspension) costs 70 to 130 ns per call,
  below 0.1% even for the non-serial design (a, 30k calls, 2 ms). It builds the frame roots
  before it checks for the shared state, so non-serial processes pay it too; harmless, and
  since #356 it checks first.
- Per collection: mark is proportional to the live set (about 1.1 ms per MB of live data
  when one large object holds handles: c2 marks 33 MB in 38 ms; c1b marks 150k objects, 8 MB,
  in 11 ms; small scoreboards 35 us). Sweep and recycle are proportional to the slot count,
  not the live set (d: 14.5 ms for 527k slots). Large long-lived structures are re-marked on
  every collection, but the trigger (more allocated than both the threshold and what survived)
  makes the cost amortised: a collection happens only after allocating as much as it marks.
- Leak check, serial process dropping a 1 KB `new` string per iteration with a wait (400k
  iterations): before 10.2 MB, after 10.2 MB, after with collection disabled 434 MB (1.1 KB per
  iteration). Same process not serial: 10 MB in all three. Textio lines and scoreboard items
  are deallocated by their libraries, so they do not grow even without collection (200k
  lines with waits, 200k scoreboard items: flat at 9 and 37 MB in all three). The collector
  restores the boundedness serial processes had when they allocated in their private heap.
- Gaps: objects of elaboration are sealed and never collected (sealed set 9 KB for the
  scoreboard designs, 147 KB for the library testbench). No collection runs inside one
  activation of a serial process (b1: 2M lines in one loop without a wait never reach a
  safepoint, 550 MB of quarantined slots, about 270 bytes per deallocated line); this is the
  same before the collector. Fixed by #355: a long activation recycles freed slots between
  instructions; peak RSS of b1 and b2 went from 544 MB to 10 MB (3 runs each, CPU time
  unchanged within the spread, same output).
- Ranking by measured benefit: (1) a safepoint or slot reclaim inside long activations
  (memory only; 550 MB in b1, equal before and after); (2) dirty flags on protected-object
  writes, only worthwhile for live sets of hundreds of MB (worst pause measured 38 ms, 0.6%
  of a run); (3) allocation-volume triggers already exist; (4) per-process heaps collected in
  parallel: no measured benefit, the shared heap work is below 1% of every run.
  Recommendation: the current overhead is acceptable; no redesign needed.
### Startup against simulation per test (2026-10-19)

Question: how much of a small test's time is startup, to judge an analyzed-unit cache. Release
build of `origin/main` (92dfc349) with temporary phase timers (not committed), interpreter tier,
the external test runner's `verification_components` suite (579 tests, 576 reached the timer
line; the other 3 end through a different path), host load average about 15 from other agents
(absolute times are inflated, shares are what matters). The runner analyzes each file once up
front (7.6 s for the suite, `-p 4`); every `yavs run` then re-parses and re-analyzes the closure
of its top level from the sources.

| Phase (ms per test) | median | p90 | Share of summed per-test time (`-p 4`, 576 tests, 2672 s) |
|---|---|---|---|
| parse + analyze of the closure | 461 | 596 | 10.3% |
| of which std/ieee (23 units) | 178 | 185 | 3.7% |
| of which test runner library + testbench (about 112 units) | 212 | 233 | 4.6% |
| of which third-party verification library (about 18-30 units) | 70 | 189 | 1.9% |
| library manifest / closure | 8 | 9 | 0.2% |
| elaboration (expand hierarchy) | 3175 | 4138 | 56.8% |
| lowering | 18 | 30 | 0.4% |
| package and instance initialisation (`build_full`) | 294 | 596 | 7.4% |
| simulation | 418 | 3003 | 25.3% |
| LLVM compile (`--tier=llvm`, 3 tests) | 14000 to 43000 | | not in the default run |

- Suite wall time: 671 s at `-p 4` (plus 7.6 s runner analysis). The 50-test subset (every 11th
  test) at `-p 1` takes 231 s (in-process median 4.1 s, startup 3.5 s of it); a reference
  simulator takes 9.7 s for the same 50 tests (4.5 s of runner compile for the whole suite).
- Units re-analyzed: median 153 units from 110 files per test (at most 181), 89341 unit analyses
  over the suite for fewer than 200 distinct units; no user-edited file is among them, all are
  library or testbench sources that did not change.
- Cache estimate: a cache load cost of 10% of analyze time (not measurable: no serializer
  exists) saves 247 s of the 2672 s, 9.2%; the suite would take about 610 s instead of 671 s at
  `-p 4`. A free cache would save 10.3%. Elaboration is the dominant startup cost (56%), and it
  varies by testbench from 46 ms (`tb_memory`) to 4.4 s (`tb_axi_stream`); it is the thing to
  look at before a cache.
- Not measured: the two testbenches of the neural-network project (their runner needs the Python
  FFI build of the runner, not set up here).



## elab (`crates/yavs-elab`, M2)

Done (without processes, which need yavs-lower and yavs-interp): `find_top` (`ENTITY`,
`ENTITY.ARCH`, `CONFIGURATION`), `elaborate` as a pure function (restart test: two elaborations
and two kernel builds give identical results), hierarchy with LRM `'path_name`/`'instance_name`
in both LRM and original spelling, unlabeled processes `_pN`, generics (actuals, `-g` overrides
through the yavs-std `'value` parser, defaults, range check), constants on demand, for/if/case
generates, blocks, entity/component/configuration instances, binding (configuration declaration,
then specification, then default binding), signals split into kernel leaves (records as struct
of arrays, `std_ulogic`/`bit`/`boolean` as bit-planes), the sub-element map
(`NetGraph::select`), uncollapsed ports with unconstrained formals taking the actual's bounds (an open `in` port, the default's, #218),
static expression actuals, per-process merged drivers, multiple-source check across the
hierarchy, LRM 14.4.2 body-before-call check, `Elaborated::build_with` into the kernel.
Elaboration-time evaluation folds globally static expressions with `Db::fold_with`, evaluates
aggregates of any number of dimensions, indexing, slices, fields, concatenation and composite
equality itself, runs the other predefined operations (shifts, `TO_STRING` of reals and
physicals, ...) through their yavs-interp natives (#220), and runs user function calls in
yavs-interp (the function lowered alone; any composite, arrays of records and records with
array elements included, as arguments and results through `call_values`). A function that uses
package objects (impure registries in shared variables) runs against the elaborated packages in
elaboration order, constants with impure initial values are evaluated in declaration order,
package constants with impure initial values are read from the package data, and building the
simulation replays these calls (#77); an impure constant elaboration cannot evaluate at all
runs at simulation start with W2118 (#220); `'PATH_NAME`/`'INSTANCE_NAME` of a design unit are evaluated during elaboration (#314). A process's impure constants run once, when the process starts (#251; difftest `protected/impure_const_once`). Also done: package bodies and generic instance bodies
loaded before elaboration, a package needing a body without one (E2116), conversion functions
and type conversions in port maps (kernel converters; conversion functions of record ports and
arrays of records convert all leaves in one call in the same delta, #74; both conversions of
`inout` ports, #238; a formal-side-only conversion of an `inout` port, scalar and composite, #74), individual
association of unconstrained array ports and of record ports with unconstrained elements (#288), implicit signals (`'stable`,
`'quiet`, `'delayed`, `'transaction`, kernel-maintained, also named in process bodies and
subprograms, #240), package signals (`:lib:pkg:s`, every
signal of a loaded package, visible from every region), package data initialised once in
analysis order with `yavs_interp::init_package` (shared variables, constants computed once,
#183; a declaration constant calling its own package's function is E2114 before lowering, #197),
objects of generic package instances (design units with their own package data; instances in
packages, entities, architectures, blocks, generates, processes and subprograms add theirs to
that region, #101), objects of packages declared in those regions (generic or not, instances of
local generic packages in the same and nested regions and through an instance of an enclosing
generic package; `generics/nested_packages`, `local_generic_packages`,
`local_generic_packages_hand`, #34), shared variables of blocks and generates one object per block, generate
alternative or iteration, shared by the nested regions (#249),
external signal names bound in elaboration order (E2117), aliases of external signals in regions and processes (read, waited on, driven; #19), external names used directly in processes, concurrent statements and expression or `inertial` port actuals (bound once the hierarchy exists), external constants, external shared variables of protected types (method calls), external signals with records and aliases of parts of external names (#93), external names in subprograms of regions and packages, package paths of every class and external constants computed at run time (#255), guarded blocks and `bus`/`register`
signals, blocks with generics and ports, incremental binding, nested and record element
resolution, user resolution functions (one declared in an entity or architecture reads its
instance's data; in a block, generate or subprogram it is E9999, #164), records and arrays
resolved as a whole (every driving process drives every leaf, #193), also nested in another
composite (each outermost resolved subelement, every array element of one, resolved with its own
sources; resolution below it unused, #333), an external name as an `in` port actual as a signal name actual, same
delta (#186), linkage ports and mode-view ports, entities with generic types,
subprograms and packages (yavs-sem `Db::instantiate_entity` clones the entity and architecture
once per distinct map of those actuals, with its own mangled identity; generic constants stay
instance data; #75; components and blocks with them, configuration specifications in such an
architecture, #222), `others` in a configuration after component configurations naming
specification-bound instances, 2019 variable ports denoting the shared variable at the end of
their chain of actuals (architecture, block, generate or package shared variables, through
entities, components and blocks; a top-level one is E2119; #188), a TIME unit below `yavs run --resolution` anywhere in the elaborated design is E2120 (LRM 5.2.4.2, std and ieee exempt), an incremental binding repeating the entity aspect of its configuration specification (E1702 only for a different one), non-static expression actuals of `in` ports and of parts of
them driven by their equivalent IR process one delta later (records leaf by leaf, ranges from
the instance's generics checked, scalar parts checked against their subtypes (#263), expression actuals in a binding indication's port map),
static expression parts in the formal's initial value, unconstrained formals bounded by a
static value or a conversion function's result subtype, type conversions of record ports and
arrays of composites (#74), conversions on unconstrained ports bounded by the conversion's type
mark or subtype, composites included (#262), `inertial` signal-name actuals as expression actuals, whole
formals (#233) and parts of an individual association (#261).

Processes: `Elaborated::lower` lowers every architecture owning a process (yavs-lower) and
`Elaborated::build` creates one yavs-interp process per process instance with per-instance data
(restart test with the interpreter).

A named array aggregate whose elements are not static (`(0 => g0, 1 => g1)` with generics) evaluated during elaboration
takes the direction of sem's result subtype (that of a constrained context, LRM 9.3.3.3), as the lowered code does;
it used the index subtype's direction before, so a `(3 downto 0)` constant came out mirrored in generate conditions
(#513: every peripheral of the neorv32 I/O switch sat at the mirrored index). Test: `generics/named_aggregate_direction`.

In-port collapsing (R4, #413; `collapse.rs`, on by default, `--no-collapse-ports` turns it off): done for `in` ports and component pass-through ports whose actual is whole leaves or part of one leaf (element, slice). Verified identical with and without it: difftest corpus (353) at 1 and 4 threads and 4 threads shuffled on both tiers, UVVM suite (76 pass / 33 fail / 28 skip either way), OSVVM `.pro` 2008 and 2019 (314/314), external runner pass lists. Measured (release, median CPU of 5, collapsed vs not): elab_heavy total 0.66 s -> 0.56 s (0.85), peak RSS 254 -> 211 MiB (0.83); ff_rtl, resolved_bus, vai_pe_array, vai_window_gen, uvvm_axistream_vvc_tb within noise (0.97 to 1.03), peak RSS unchanged. Still apart: forced or released ports, ports with conversions or drivers, guarded, user-resolved or whole-resolved composites, every port under `--dap`/`--vhpi`; widening these was not measured as worthwhile (the designs above have none of them in a hot path). `out` and `buffer` ports that are the only source of their actual collapse too (#528, below).

CLI: `yavs elab` and `yavs run` per docs/cli-contract.md (closure from the manifests, design
revision = newest standard in the closure, `-g` after TOP, `@file`, exit codes, reports in the reference simulator's
format on stdout). Difftest: see `tests/corpus/RESULTS.md`; `generics/generic_types_2019` and the other generic-type cases pass since #75.

## ffi (`crates/yavs-ffi`)

Done: VHPIDIRECT call trampolines without libffi (SysV x86_64, Win64, AAPCS64), argument
classification in Rust, naked asm stubs, up to 64 arguments, and the VHDL to C mapping in the reference simulator's
convention (`vhdl::lower`). Tests: unit tests, hand-written ABI tests and 431 generated callees
compared against direct calls. All pass on Linux x86_64 natively, on aarch64 under qemu-user and
on Windows under wine. The cross commands (configured through the environment, no cargo config):

```sh
# aarch64 Linux (qemu-user, aarch64-linux-gnu-gcc, rustup target aarch64-unknown-linux-gnu)
CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc \
CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_RUNNER="qemu-aarch64 -L /usr/aarch64-linux-gnu" \
  cargo test -p yavs-ffi --target aarch64-unknown-linux-gnu
# Windows x86_64 (mingw-w64, wine, rustup target x86_64-pc-windows-gnu)
CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=x86_64-w64-mingw32-gcc \
CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUNNER=wine WINEDEBUG=-all \
  cargo test -p yavs-ffi --target x86_64-pc-windows-gnu
```

`yavs run --load=LIB` end to end (#95): `'FOREIGN "VHPIDIRECT symbol"` subprograms (placeholder
bodies allowed) are lowered to `CallForeign`, bound to symbols in the `--load` libraries, then
the executable (E0106 cannot load before the simulation; symbol not found is warning W0108 before time 0 and E0107 only at the call), and
called with scalars, scalar `out`/`inout`, arrays (with copy-back), records and access values.
Test `crates/yavs/tests/foreign.rs` compiles a C library at test time: passes on Linux x86_64
and on aarch64 under qemu-user (`QEMU_LD_PREFIX=/usr/aarch64-linux-gnu` so the spawned `yavs`
finds its loader); on Windows x86_64 GNU the same testbench and error cases pass under wine by
hand with a MinGW DLL (the test cannot spawn the compiler inside wine). Native Windows (MSVC CI)
runs the test as written.

## vhpi export (`crates/yavs-vhpi`, `crates/yavs`)

Done (#266): the `yavs` executable exports the full `vhpi_*` C ABI of `vhpi_user.h` (42 symbols:
34 functions and the eight time constants `vhpiFS` to `vhpiHR`; counted in `exports.txt` and `nm -D` of the default build on 2026-10-28; since the first VHPI audit round every function works except `vhpi_get_next_time`, `vhpi_get_data`, `vhpi_put_data` and `vhpi_protected_call`, which report "not implemented", see `docs/vhpi.md`) on Linux x86_64 and aarch64 (also in the
glibc 2.17 cargo-zigbuild release builds, where the linker exports more than the list) and in the
Windows `.exe` export table (MinGW checked; MSVC `/DEF:` and the `yavs run --vhpi` plugin load run in the `windows-latest` CI job, #124). No import library is built or shipped (user decision, 2026-10-18): a VHPI plugin brings its own.
`crates/yavs/tests/vhpi_exports.rs` checks the ELF `.dynsym` (Linux) and the PE export directory (Windows, in the `windows-latest` CI job, #124) against the header.

Done (#267): `yavs run --vhpi=LIB[:ENTRY]` (repeatable) loads plugins after elaboration,
fires start/end of simulation (and start of initialization) callbacks through a kernel
`Observer`, and ends the run on `vhpi_control(vhpiStop | vhpiFinish)` (status 0, new
`Hook::end`); load failures are `E0901`/`E0902` with exit 2. `crates/yavs/tests/vhpi_run.rs`
(2 tests, Unix only) loads the `yavs-vhpi` C test plugin through the CLI, both entry forms.

Done (#272, object-independent part; #123): simulation-phase and after-delay callbacks through
`yavs_vhpi::VhpiObserver` and two new kernel phases (`EndOfInit`, `NextCycle`): start/end of
initialization, next time step, start of next cycle, start and end of processes and start of
postponed (kernel phases `StartOfProcesses`, `EndOfProcesses` (also once after initialization), `StartOfPostponed`, #317), last known delta cycle (read-write), end
of time step (read-only; both also at time 0 after initialization when no delta follows, #347), after delay, each with its `Rep` form; `vhpi_remove_cb`,
`vhpi_enable_cb`, `vhpi_disable_cb`, `vhpi_get_cb_info` (removing a matured callback is legal);
disabled callbacks sit outside the per-phase sets and cost nothing per cycle. `vhpi_get_time`
and `vhpiResolutionLimitP` follow `--resolution`. `crates/yavs/tests/vhpi_run.rs` (7 tests) runs
`tests/plugin/sched.c` scenarios: timed finish at T in resolution units, after-delay chains and
a removed repeating one, read-write before postponed before read-only, enable/disable with
100000 disabled callbacks, misuse; output identical at `--threads 1` and `4`.

Done (#271, #272 value changes): `vhpi_put_value` on signals, ports and their record fields and
array elements, in every format `vhpi_get_value` produces (`Read::decode`) and integers into
logic vectors (#319) and `vhpiStrVal` text shorter or longer than a string (padded with NUL or cut, #345), queued in the state and handed to the kernel as
`Hook::deposit`/`force`/`release` when the callback returns, always in a new delta (#319):
`vhpiDepositPropagate` deposits, `vhpiForcePropagate` forces (the non-propagating modes are refused, user, 2026-10-19; force mode `out` for ports of
mode out/inout/buffer, `in` otherwise), `vhpiRelease` releases (back to the drivers' value, also after a deposit, #310); refused in the read-only phases;
constants, generics, records as a whole, bad values and `vhpiSizeConstraint` are `W0903`.
`vhpiCbValueChange` on signals, ports and sub-elements, keyed by kernel signal and dispatched
from `Observer::value_change`, once per cycle (one registered during a cycle's dispatch is not shown its own re-armed trigger's change again, #346, but sees the other changes of the cycle once, whichever the kernel reported first, #496), with the value filled in the requested format;
with none enabled the observer does not take the state lock. `crates/yavs/tests/vhpi_run.rs`
`vhpi_drives_a_clocked_design` (a counter clocked and reset only from VHPI, force, release and
deposit on a driven signal, value changes on a signal, a port, a bit and a record; identical at
`--threads 1` and `4`, `tests/put/expected.txt`) and `vhpi_put_value_misuse_is_reported`
(`tests/put/errors.txt`). `vhpi_schedule_transaction` works on the drivers of foreign models (#330, below).

Done (#309, variables): `vhpi_get_value` and `vhpi_put_value` (deposits) on process variables,
non-protected shared variables of architectures, blocks and packages, and their record fields
and array elements, from simulation callbacks (processes are suspended there). Process
variables through the kernel's `Hook::with_process` and `Process::as_any` into the
interpreter's frame (`InterpProcess::var`/`set_var`), shared variables at the slot address
elaboration computes (`Elaborated::build_full`, `yavs_interp::shared_var`); values cross as
elaborated values (`yavs_elab::ir_decode`/`ir_encode`). A deposit is written at once, so
later reads in the callback and the process when it resumes see it. Before the process first ran (#342) its declarative part runs on first access (`InterpProcess::prestart`), and every region shared variable has a slot even when no process uses it. Forces, releases, the
read-only phases and a process variable before its process first ran (before
`vhpiCbEndOfInitialization`) are `W0903`; a protected object has no value. Array bounds of
objects with a constrained subtype now come from the subtype, not the initial value.
`crates/yavs/tests/vhpi_run.rs` `vhpi_reads_and_deposits_variables` (`tests/plugin/vars.c`,
`tests/vars/expected.txt`, identical at `--threads 1` and `4`). Not yet: reading a process
variable before its process ran, a region's shared variable no process uses (#342), variables
of subprograms.

Done (#274): `vhpi_register_foreignf` and `vhpi_get_foreignf_info` (`crates/yavs-vhpi/src/foreign.rs`):
`'FOREIGN "VHPI library model"` functions and procedures (lowering: `ForeignRef::model`; interpreter:
`ForeignModel`, `Bindings::models`; CLI binds them after the plugins load) run the plugin's `execf`
with the call as `obj`; parameters through `vhpiParamDecls`, `vhpi_get_value`, and `vhpi_put_value`
on `out`/`inout` parameters and on the call for the function result; `elabf` runs once per bound
subprogram; an unregistered model is `W0108` then `E0107` at the call. Scalars and arrays of scalars
only. `crates/yavs/tests/vhpi_run.rs` `vhpi_foreign_subprograms` (functions of integer, real, time,
logic vector, boolean; procedures with `out`, `inout` and array `inout`; identical at `--threads 4`,
`tests/foreign/expected.txt`) and `vhpi_foreign_missing_model_and_result`. Missing:
`vhpi_get_data`/`vhpi_put_data`.
Done (#357): `vhpiLibF` and `vhpiAppF` models run: a library model's `elabf` when a foreign model of its
library is bound, its `execf` and an application model's `elabf` and `execf` at the start of the
simulation (`vhpi_library_and_application_models`, `tests/libapp/expected.txt`, same at `--threads 4`;
conformance case `libapp`, no reference runs them).
Done (#330, array results): a foreign function may return a one-dimensional array of scalars, sized
with `vhpiSizeConstraint` (unconstrained, indexed `1 to n`) or by its constrained result subtype;
tests in `vhpi_foreign_subprograms` (`rev`, `iota`, `word`, `nibble`) and
`vhpi_foreign_missing_model_and_result` (`nosize`); conformance plugin `tools/vhpi-conformance/plugins/fresult.c`.
Done (#330, parameters): record parameters (fields through `vhpiSelectedNames`, written for
`out`/`inout`), `in` access parameters (`vhpiPtrVal`), signal parameters (value at the call; for
`out`/`inout` a deposit and transactions on the caller's driver from `vhpiDrivers` with
`vhpi_schedule_transaction`); tests in `vhpi_foreign_subprograms` (`norm`, `flip`, `peek`,
`drive`), conformance plugin `tools/vhpi-conformance/plugins/fparam.c`.
Done (#330, foreign architectures): `"VHPI library model"` architectures elaborate as their ports
(`Elaborated::foreign_archs`); the models' `elabf` and `execf` run before the build, create
processes and drivers with `vhpi_create` (`Elaborated::ext_procs`) and drive them with
`vhpi_schedule_transaction` (kernel `Hook::assign`); an unregistered model is `E0905`. Test
`vhpi_foreign_architecture` (`tests/plugin/farch.c`, `tests/farch/expected.txt`, identical at
`--threads 4`); conformance plugin `tools/vhpi-conformance/plugins/farch.c`.

Done (#269): the design hierarchy object model (`crates/yavs-vhpi/src/design.rs`), a snapshot of
the elaborated design made before plugins load: root and component instances, blocks, `for`
iterations (`G(0)`), `if`/`case` generates, process statements, package instances (`@lib:pkg`),
generics, ports (with mode), signals, constants (generate parameters included) and variables;
record fields and array elements of any dimension and index type as `vhpiSelectedNameK` and
`vhpiIndexedNameK` objects made on demand. `vhpi_handle` (root, tool, upper and immediate region,
design and primary unit, prefix), `vhpi_handle_by_name` (absolute and relative, case-insensitive,
`.field` and `(index)` selections), `vhpi_handle_by_index`, `vhpi_iterator`/`vhpi_scan`
(`vhpiInternalRegions`, `vhpiBlockStmts`, `vhpiCompInstStmts`, `vhpiStmts` (#316), `vhpiDecls`, `vhpiSigDecls`, `vhpiPortDecls`, `vhpiGenericDecls`,
`vhpiConstDecls`, `vhpiVarDecls`, `vhpiPackInsts`, `vhpiIndexedNames`, `vhpiSelectedNames`),
`vhpiNameP`, `vhpiCaseNameP` (original spelling), full names, kind, file, line, mode,
`vhpiDefNameP`, `vhpiIsCompositeP`, `vhpiSizeP`; `vhpi_compare_handles` compares objects.
Checked by `crates/yavs/tests/vhpi_run.rs` `plugin_walks_the_design_hierarchy` (C plugin
`crates/yavs-vhpi/tests/plugin/hier.c` against `tests/hier/expected.txt`) and unit tests;
names and kinds agree with the reference simulator that has VHPI wherever it reports them.
Missing (see `docs/vhpi.md` "Coverage of the standard"): equivalent processes of concurrent statements, aliases, files (#829, #831);
sub-elements of variables whose bounds are only known at run time (`W0904`).

Done (#270, values): `vhpi_get_value` (`crates/yavs-vhpi/src/value.rs`) on signals, ports,
generics and constants and their record fields and array elements (any dimension), from callbacks
through the observer's `Hook` (signals) or the elaborated value (generics, constants), in
`vhpiObjTypeVal`, `vhpiBinStrVal`, `vhpiOctStrVal`, `vhpiDecStrVal`, `vhpiHexStrVal`,
`vhpiStrVal`, `vhpiCharVal`, `vhpiEnumVal` (also on integers, #318), `vhpiSmallEnumVal`, `vhpiLogicVal`, `vhpiIntVal`,
`vhpiLongIntVal`, `vhpiRealVal`, `vhpiTimeVal`, `vhpiPhysVal` and the vector forms
(`vhpiEnumVecVal`, `vhpiSmallEnumVecVal`, `vhpiLogicVecVal`, `vhpiIntVecVal`,
`vhpiLongIntVecVal`, `vhpiRealVecVal`, `vhpiTimeVecVal`, `vhpiPhysVecVal`), with the
buffer-size protocol. `crates/yavs/tests/vhpi_run.rs` `plugin_reads_values_in_every_format`
(C plugin `tests/plugin/values.c`, 27 objects by 22 formats, against `tests/values/expected.txt`)
plus unit tests; every value the reference simulator with VHPI reports correctly agrees.
Types (`crates/yavs-vhpi/src/types.rs`, a table built with the snapshot): `vhpiType` (and the
2008 `vhpiSubtype`) of declarations and sub-elements, `vhpiBaseType`, `vhpiElemType`,
`vhpiConstraints` (integer, enumeration, floating and physical ranges with `vhpiLeftBoundP`,
`vhpiRightBoundP`, `vhpiIsUpP`, `vhpiIsNullP`, the float and physical bound properties),
`vhpiEnumLiterals`, `vhpiRecordElems`, `vhpiUnitDecls`, `vhpiNumLiteralsP`,
`vhpiNumDimensionsP`, `vhpiNumFieldsP`, `vhpiIsUnconstrainedP`, `vhpiIsScalarP`,
`vhpiIsAnonymousP`, `vhpiPositionP`, `vhpiPhysPositionP`; the same plugin dumps every object's
type, and it agrees with the reference simulator on kinds, names, bases, bounds, dimensions,
literals and fields (yavs also gives enumeration and floating ranges and unit names).
Missing: `vhpiPtrVal` outside foreign access parameters, `vhpiRawDataVal`, `vhpiSmallPhys*`, records as a
whole (`W0904`, #833; variables' values came with #309); the element subtype of a resolved array subtype (`std_logic_vector`'s
elements report `STD_ULOGIC`).

Done (#143, VHPI audit, first round): `docs/vhpi.md` "Coverage of the standard" lists every area of the IEEE 1076-2019 VHPI as supported, partial or missing, with an issue per gap (#828 to #833). New: user attribute specifications of declarations (`vhpiAttrSpecs`, `vhpiAttrSpecK` with its value, `vhpiAttrDecl`), `vhpiCapabilitiesP`, `vhpiNumPortsP`, `vhpiNumGensP`, `vhpiLabelNameP`, `vhpiGenerateIndexP`, `vhpiIsSharedP`, `vhpiStartLineNoP`, the foreign-model reasons `vhpiCbTimeOut`, `vhpiCbRepTimeOut` and `vhpiCbSensitivity` (on a signal), and `vhpi_format_value`; tests in `crates/yavs/tests/vhpi_run.rs` (plugin `tests/plugin/extras.c`).

Done (#828): `vhpi_get_next_time` in the end-of-time-step phase (kernel `Hook::next_time`), test `vhpi_next_time_in_the_read_only_phase`.

Done (#122): `tools/cocotb/run.py` installs cocotb 2.1.0 (pinned wheel, tests from the pinned tag, test dependencies pinned) into a venv, builds cocotb's VHPI library from its source without simulator defines (`libcocotbvhpi_yavs`), adds yavs to cocotb's runner and pytest-plugin tables and runs what cocotb's own CI runs for a simulator: the 68 `tests/test_cases` directories, the examples through make and pytest, `pytest -k simulator_required` and the pytest plugin's tests, with cocotb's CI environment (`PYTHONWARNINGS=error`, debug switches). Result on main 2026-10-28: 78 of 78 jobs pass (43 run VHDL tests: 702 tests pass, the 10 failing tests are the expected ones of `test_failure` and `test_max_failures`; the other 35 jobs are Verilog only or error-path cases), identical at `--threads 1` and `4` and with 1 or 6 jobs in parallel; about 20 s. The two directories that failed before (66 of 68) failed in the local integration (time resolution and plusargs not passed, no runner entry), not in VHPI. With yavs added to name gates the recursive walk of `test_iteration_vhdl` finds 2663 objects, the count three reference simulators agree on. CI: the `conformance` workflow runs it on every push to main.

Done (#273): `tools/vhpi-conformance/run.py` runs 5 C plugins (walk, props, values, cbs, put) on 2 small designs under yavs and the primary reference simulator and diffs the canonical traces, no install needed. Baseline at `ba53e23d` (reference lines matched): walk 28/172, props 158/328, values 159/230, cbs 78/114, put 108/149; put_value (#271) was still open.
Three-way run (#273, second reference simulator with VHPI, `--refs sim4`), on main with #269 to #272 (yavs lines matching: both references / only the first / only the second / neither): walk 9/64/19/61, props 152/24/92/60, values 102/57/56/15, cbs 64/15/5/3, put 60/41/22/26 (case-folded, LCS-aligned; many `neither` lines are yavs reporting a property both references give as -1). Real gaps where yavs differs from both: `vhpi_iterator` for block and instance statements (#316), end-of-processes callbacks (#317), `vhpiEnumVal` on integers (#318), integer put into a vector and the delta cycle after a propagating put (#319); release after deposit is #310. Reference quirks (the references differ from each other): upper-case names in one, a dummy root above the design in the other, `vhpi_put_value` on undriven signals, time resolution, string formats of enumerations. After #310, #316 to #319 (and the non-propagating put modes refused): walk 20/93/32/92 (yavs now lists block, instance and statement iterators the references answer differently, more lines), props 152/24/92/60, values 104/57/56/13, cbs 81/18/16/14 (the remaining `neither` lines are start-of-processes callbacks the second reference orders around after-delay callbacks and omits in a time step's last delta), put 66/59/9/15, nonprop (new) 56/50/6/5.

## discrepancies (simulator catalogue)

`docs/design/simulator-discrepancies.md` lists 132 cases where the reference simulators
disagree with each other, the LRM or yavs, each with a reproducer in `tests/discrepancies/` and a
recorded verdict per simulator. `python3 tools/discrepancies/run.py --sims sim1,sim2,sim3,sim4 --check`
runs them all (yavs plus the configured simulators; the label mapping is local) and fails when yavs
or a recorded verdict drifts. A fourth simulator is added with one template and `--record`. The
user page `docs/language-choices.rst` is the user-facing subset.

`docs/design/vhdl2019-support.md` is the VHDL-2019 support matrix: one row per approved LCS with yavs's status and the result of the four references on a small probe (2026-10-19: 34 supported, 4 partial, 3 unsupported, 5 not tested or not applicable; gaps #360 to #363, #29).

## test runner recipes

`docs/recipes.rst` (user page): CLI use from an external Python test runner, libraries from source, per-file `--std`, response files, exit codes, and the vendor-library pattern (#275).

## difftest (yavs candidate)

`python3 tools/difftest/difftest.py --candidate yavs` drives `yavs analyze`, `yavs elab` and
`yavs run`; `tools/difftest/report.sh` writes `tests/corpus/RESULTS.md`, which has the current pass counts per
group (249 of 249 at 1 thread, 4 threads (identical to 1) and 4 threads shuffled). `std.env.stop`/`finish` end the calling
process at once; check messages print reals as `%.15g`; runtime errors are coded diagnostics
(`E3001`-`E3010`, kernel `ErrorKind::code`) with the VHDL call stack, which difftest reads back
as `fatal` events. Normalisations for the reference simulator's free choices (default assert message without a
path, per-test `ignore=order`) are in `docs/decisions.md`.

IEEE packages from source (`tests/corpus/ieee`, #94): std_logic_1164, numeric_std, numeric_bit,
math_real, math_complex, fixed_pkg and float_pkg bodies, 2008 and 2019, equal the reference simulator at 1, 4 and 4
shuffled threads (17 of 17, with textio of std_logic_vector, fixed and float values);
`ieee_warnings` (`ref=hand`) checks the numeric_std body's own warnings, which the reference simulator omits.

### Live reports and broken stdout (#483, #484)

- A process that runs serially (all of them at `--threads 1`, the serial phase at more threads) writes its
  reports and `OUTPUT` text to the output as it makes them instead of at suspension: the commit order is
  unchanged, a failing process stops the later ones of its batch from running, and parallel batches and
  `--shuffle` keep commit-at-suspension. Test: `long_process_reports_live` (first line in a pipe within 3 s, kept
  on SIGTERM).
- A write error on stdout (closed pipe) stops the run between cycles with a line on stderr, finishes wave and
  coverage files and exits 141 (a run still writing after 1 s exits at once). Test: `closed_stdout_stops_run`.

## second and third runs of a suite (#407, 2026-10-21)

Cause of the `.pro` driver's second run being a third slower (and its third run too): the second run of every closure
made entries of its own (analysis image 5.6 MB, lowered packages, state, IR: about 15 MB) and stopped using the shared library
stages, which only a first run of a closure read. The 68 closures of one library directory write four entries each into
its 96, so they pushed each other out and the third run wrote them again; read back they were no faster than the shared
entries (user CPU of 314 runs: first 43 s, second 59 s, third 54 s; per phase: analysis 2.0, 9.1, 7.0 s and simulation
28.4, 34.4, 33.0 s). A closure under a prefix whose lowered library exists now keeps using the shared stages at every run and
writes nothing (`docs/decisions.md` "Library stages"); `crates/yavs/tests/cache_prefix.rs` pins that three more runs of a closure
store nothing and print the same.
Peak resident size: ordinary library runs are 60 MB, those that call a lot of the library 155 MB (the same with `--no-cache`);
the one run that makes the lowered library peaked at 224 MB because the two lowerings (60 MB of IR) stayed resident through its
simulation, now 184 MB (the heap is given back after the entry is made). The 567 MB process of the first library's suite
(`rand_tb` `rand_cyclic_performance`) is the same with `--no-cache`: the simulation itself, not the cache.
Pinned to one performance core, `-j1`, release build, `.pro` suite (2008, 314 tests): user CPU of the runs first / second / third
42.9 / 39.8 / 39.6 s, wall 56.2 / 52.3 / 52.2 s (the head-to-head: 56 / 74 / 68 s). The first library's suite at `-j1`:
user CPU 123 / 119 / 118 s. Output of the `.pro` suite (2008 and 2019, every test log) and of the first library's suite
byte-identical between the first, second, third run and `--no-cache`. Difftest (each `run` three times in one work directory,
outputs compared): 413/414 at 1 and 4 threads on the interpreter, `--tier=llvm`, `--tier=auto` and the `gc-stress` build (the one
exception appends to a file in the work directory, so repeating it changes its output by design).

## performance: tight compute loops, 2026-10-21 (#432, #444)

Design 1 of the head-to-head (`compute_h2h`, 20M iterations of integer and real arithmetic in one process), pinned to
performance cores 1 to 3 (`tools/bench/run.py --ref sim1,sim2 --tiers interp,auto,llvm --instructions`, median of 3).
Before this change (main at 894a74b8) and after, wall seconds and instructions per iteration (billions / 20M):

| Tool | before wall | after wall | before instr/iter | after instr/iter |
|---|---:|---:|---:|---:|
| yavs interp | 1.89 | 1.71 | 2410 | 2160 |
| yavs auto (2 cores) | 0.68 | 0.29 | 838 | 328 |
| yavs llvm | 0.33 | 0.19 | 342 | 157 |
| sim1 jit | 2.19 / 2.16 | | 2323 | |
| sim2 | 0.20 | 0.19 | 115 | |

Cycles per iteration under `--tier=llvm`: 74.5 before, 41.7 after (sim2: 50). The loop body per iteration, compiled,
by step (instructions per iteration): 317 as found (12 `Stmt` markers at about 3 each, 40 register stores of
constants, addresses and temporaries with the loads that follow, a load and store of the frame variable address per access,
and the checks the language requires); 283 without the markers (`drop_statements` for processes); 214 with pure results
kept as values and stored only where the interpreter can look (`Gen::std`, `Gen::flush`); 158 without stores of
preloaded constants and addresses; 143 with frame-variable loads that need no slow path and addresses known in every
block; 137 with a static `for` loop stepping by a constant. What is left: the overflow and range checks of each
operation, the frame variables as memory, the registers carried between blocks through the frame (every block start is an
entry point of the interpreter), and the slot base reloaded per block. `--tier=auto` now reaches compiled code
after about 0.15 s (compile of the 238-instruction process 30 to 60 ms) instead of 0.4 s. `tools/bench` compare of
before and after, total CPU ratio (after / before): interp compute_loop 0.91, calls 0.94, strings_heavy 0.94, rtl_ff 1.01,
lfsr64 1.01, rtl_chain 0.99, rtl_fan 1.01; llvm 0.87, 0.94, 0.94, 0.97, 0.97, 0.96, 0.94; auto 0.98, 0.99, 0.97, 1.01,
1.00, 1.01, 1.01.

Verification on the pushed tree: difftest corpus (419) at 4 threads (1 against 4) and 4 shuffled on the interpreter,
`--tier=llvm`, `--tier=auto` and `--tier=auto --tier-heat=1,0,0`: all 419; `--tier=llvm` and `auto` of a build with
`gc-stress`: 419; the external runner lists (857 and 393) on the interpreter and `--tier=auto`; `.pro` regression 314/314
and UVVM 102 pass, 9 fail, 26 skip under `--tier=auto`; `cargo test --workspace` and `-p yavs-llvm --features llvm`
green. Not done: registers as phi values across blocks with entry stubs and frame variables kept as values with
write-back at exits (would remove most of the remaining loads and stores); interval analysis for processes (loop
variables of a process stay in memory, so the checks stay).

### Intervals and register promotion in processes (#444)

The interval analysis and the promotion of frame slots to registers now run on processes (`docs/decisions.md`
"Intervals and register promotion in processes"). Facts hold within one activation; at a suspension the registers
are unknown and each variable slot is reset to the interval all its stores keep it in (found by iterating), so a
variable that only holds 0 to 5 stays in 0 to 5 across a `wait`. A slot no suspension leaves live (a loop variable,
the variables of a loop between two waits) becomes a register; handles and addresses stay in memory. A process that
only has `wait;` is never woken, which the verifier knows too. Plugins (`--vhpi`) and `--wave-variables` read or
write the variables of a process from outside: `opt::external_variables` keeps named variables in memory and unknown
after a suspension and stops the store of optimised process copies. The memoisation of instance-constant values
needs a gain of 6 instead of 3 per copy under `--tier=llvm` (`opt::compiled_tier`).

`tools/bench` on the pinned performance core 2, total CPU seconds before / after (interleaved, 7 runs; ratio after / before):

| design | interp | auto | llvm |
|---|---|---|---|
| compute_h2h | 1.93 / 1.41 (0.73) | 0.33 / 0.28 (0.83) | 0.20 / 0.18 (0.90) |
| xor_sig | 0.94 / 0.85 (0.90) | 0.89 / 0.80 (0.89) | 0.94 / 0.94 (1.01) |
| xor_types | 0.86 / 0.73 (0.85) | 0.81 / 0.71 (0.88) | 0.85 / 0.84 (0.99) |
| lfsr64, lfsr64_xor | 1.00, 0.97 | 0.98, 0.95 | 0.97, 0.96 |
| rtl_* (7 designs) | 0.97 to 1.01 | 0.99 to 1.01 | 0.98 to 1.01 |

Geometric mean of the total over the h2h group: 0.943 interp, 0.955 auto, 0.984 llvm; internal group (13): 0.943
(compute_loop 0.77, calls 0.89). Executed instructions of compute_h2h: 43.2 G to 29.4 G interpreted (0.68), 3.14 G to 2.63 G
compiled. Against the reference simulators, compute_h2h (CPU, same run): sim1 2.25 s, sim2 0.20 s; yavs interp 1.37 s,
auto 0.25 s, llvm 0.18 s. A small stream testbench of one verification library (0.38 s), the queue testbench of another (0.05 s) and a
verification-component test case (1.72 s) did not change: the optimiser floor is the same (ratios 1.00).
`--tier=auto` had stopped compiling compute_h2h (the tighter loop made the compute test of `auto.rs` count the
process's way back through its suspension as part of the loop): that test now counts only real loops.

Memoisation gate (item 2): with the interpreter a copy won at every size tried (cycles and instructions down 2 to 5
percent even for a block that saves two instructions net), so the interpreter's gate stays at 3; compiled it loses
about 2 percent for blocks that save five net (lfsr64_xor +1.7 percent instructions, rtl_chain 4 percent at a gate
of 12 versus 9), so `--tier=llvm` asks for 6.

Not done: relational facts (a loop variable bounded by another variable stays unbounded for the analysis, which keeps
the `acc * 3 + i` check of compute_h2h), promotion of handle and address slots of processes.

## Multi-threaded scaling, measurement (#481 item 3, 2026-10-21)

Measurement only, nothing changed in the product (details and tables in the comment on #481). `--threads N` against 1,
interpreter and `--tier=auto`, each run pinned to N performance cores, median of 3. Flat RTL with big batches gains a
little (rtl_ff 1.33x, rtl_fan 1.20x, the soft-core testbench 1.75x at 4 threads, 2.3x at 6), designs with batches of
32 to 64 processes lose (rtl_bus, lfsr64, lfsr64_xor 0.5x to 0.65x, 5x the CPU), and testbench-style code (vai_*,
OSVVM, UVVM) does not move: no batch reaches 32 processes and 63 to 100 percent of the process time is in serial
processes. Batches under 32 take the single-thread path unchanged (rtl_chain: +0.15% instructions at 4 threads);
batches of 32 or more lose the round 9 early commit, lock each slot twice and pay a rayon install per delta (66 to 108
us per batch). Ideal-schedule ceilings at 4 threads: 2.1x to 2.7x for the large-batch designs, 1.0x for the testbench
rows. Verdict: not worth building on as a general speed-up; the cheap first step is a work-based inline/pooled
decision, then a cheaper pooled batch. The host's core 0 runs single-thread code about 50 percent slower than cores
1 to 7: do not pin a timed run to it.

Follow-up analysis and design (2026-10-21): `docs/design/parallel-kernel.md`. The loss at N>1 is mostly the kernel
thread walking every slot after a pooled batch (live-output scan, commit without the early commit); a prototype
(branch `spike/mt-chunks`) with a work-based inline/pooled choice and a run phase split into pieces of the slot
array, with the early commit in the worker, removes every regression and reaches 2.47x on rtl_fan and 1.67x on
rtl_ff at 4 threads. Testbench-style designs cannot gain inside a delta even with perfect conflict handling (ceilings
1.0x to 1.24x at 4 threads).

Caveat (#513, found 2026-10-22): every yavs that elaborated the soft-core testbench before the #513 fix ran a diverging
program (checked at `634452a2`, the commit of `docs/design/parallel-kernel.md`; the bug dates from `7e31f69e`, before
which the design did not elaborate). Its core trapped at the first peripheral store after about 70 instructions and slept
in `wfi` for the rest of the run, so the neorv32 rows above and in `docs/design/parallel-kernel.md` time a mostly idle
CPU and do not stand for the real program. Re-measure them on a fixed build before relying on them.

## Parallel kernel, step 1: pooled by estimated work (#497, #481 item 3, 2026-10-28)

A batch goes to the pool when the estimated work of its parallel processes, less the largest, passes 20 us (per-process
costs from timed inline activations), no longer from 32 processes; the live-output scan runs only with a serial process in
the batch; `YAVS_POOL_ALL=1` pools every batch of two or more (`docs/decisions.md` "Parallel kernel, step 1").

Speedup over one thread of the same binary (simulate phase, interpreter, bench builds, median of 5, interleaved; each
run pinned to N cores of 8 to 15, 1 thread on core 9), main before (`6ea17bde`) in parentheses:

| Design | 2 threads | 4 threads | 8 threads |
|---|---|---|---|
| rtl_fan | 1.14 (1.01) | 1.20 (1.02) | 1.11 (0.92) |
| rtl_ff | 0.92 (0.91) | 0.96 (0.84) | 0.92 (0.81) |
| rtl_lfsr | 0.86 (0.89) | 0.81 (0.80) | 0.71 (0.72) |
| lfsr64 | 0.99 (0.52) | 1.00 (0.51) | 0.98 (0.35) |
| lfsr64_xor | 1.01 (0.62) | 0.99 (0.60) | 1.00 (0.48) |
| rtl_bus | 0.99 (0.55) | 0.98 (0.51) | 1.00 (0.39) |
| rtl_chain | 0.97 (1.00) | 1.02 (1.01) | 0.99 (0.97) |
| neorv32_dual_full (200 us) | 0.83 (0.65) | 0.75 (0.63) | 0.57 (0.53) |
| neorv32_single_min (300 us) | 1.00 (0.63) | 0.94 (0.56) | 0.66 (0.31) |

CPU time of the mid-size designs at N threads is now that of one thread (lfsr64 0.25 s at 1 to 8 threads, before 0.81 to
3.10 s). The soft-core program runs (core 0 executes its loop, about 6000 instructions in 200 us; core 1 of the dual
configuration parks in `wfi` after 49 instructions by program design, not #513). One thread, `tools/realbench/run.py --compare` of main
before and after on the 42 tests (cores 9 and 10, `--count-runs`), geometric means B/A: wall 1.00 interpreted and 0.99
with `--tier=auto`, instructions 1.00 and 0.99, cycles 1.00 and 1.00.

## Parallel kernel, step 2: pieces of the slot array, commit in the worker (#481 item 3, 2026-10-28)

A pooled batch runs in pieces of consecutive process numbers that `threads - 1` workers and the kernel thread claim in
turn, without a lock per process; the usual clocked process is committed in the worker; serial processes run after the
join in batch order; the pooling threshold rises when pooled batches turn out no faster than their estimate
(`docs/decisions.md` "Parallel kernel, step 2").

Speedup over one thread of the same binary, simulate phase (whole-run wall in parentheses), interpreter, bench builds,
median of 5 rounds that interleave the thread counts and the two binaries, runs pinned to N free cores of 8 to 15 during a
reserved timing window; main before step 1 (`6ea17bde`) for comparison:

| Design | 2 threads | 4 threads | 8 threads | before: 2 / 4 / 8 | CPU s at 4 threads (1 thread) |
|---|---|---|---|---|---|
| rtl_fan | 1.70 (1.61) | 2.63 (2.39) | 3.89 (3.25) | 0.94 / 1.02 / 1.04 | 1.13 (0.85) |
| rtl_ff | 1.37 (1.33) | 2.10 (1.93) | 3.04 (2.64) | 1.15 / 1.45 / 1.56 | 1.01 (0.67) |
| rtl_lfsr | 1.41 (1.33) | 1.86 (1.69) | 2.41 (2.07) | 1.06 / 1.15 / 1.19 | 0.73 (0.38) |
| lfsr64 | 0.96 (0.97) | 0.95 (0.97) | 0.96 (0.97) | 0.49 / 0.52 / 0.56 | 0.18 (0.17) |
| lfsr64_xor | 0.98 (0.98) | 0.98 (0.96) | 0.98 (0.95) | 0.57 / 0.61 / 0.60 | 0.07 (0.07) |
| rtl_bus | 1.00 (0.96) | 0.99 (0.97) | 0.98 (1.00) | 0.53 / 0.54 / 0.53 | 0.21 (0.21) |
| rtl_chain | 1.03 (1.07) | 1.03 (1.02) | 1.03 (1.01) | 1.01 / 1.02 / 1.00 | 0.13 (0.13) |
| neorv32_dual_full (200 us) | 1.00 (1.00) | 0.98 (0.98) | 0.98 (0.98) | 0.70 / 0.74 / 0.74 | 1.86 (1.79) |
| neorv32_single_min (300 us) | 0.94 (0.94) | 0.96 (0.96) | 0.97 (0.97) | 0.74 / 0.71 / 0.68 | 2.52 (2.41) |

The designs that never pool (lfsr64, lfsr64_xor, rtl_bus, rtl_chain; the soft cores pool 12 to 320 batches as probes)
run 1.2 to 2 percent more instructions at N threads; timing one activation in 64 instead of 16 left that unchanged, so
the timed sample is not the cause (not yet identified, #858). At 4 threads the kernel
thread still spends 13 percent (rtl_ff: `commit_codes`, the batch gather) and 17 percent (rtl_lfsr: `finish_direct`,
`commit_planes_run`) of the run in the serial commit and update; the threads are otherwise balanced (each 25 percent of
the samples). The realbench set at 4 threads (`YAVS_BENCH_TIERFLAGS=--threads=4` for both, interpreter, each test on
cores 8 to 11 or 12 to 15), this step against main before step 1: geometric means wall 0.98, CPU 0.79, instructions 0.98,
summed wall 0.82; `poc_arith_addw` 0.49, `fletcher_bufferwriter_512` 0.65, the three neorv32 tests 0.71 to 0.78 (CPU 0.13
to 0.30), the short tests unchanged. One thread, `tools/realbench/run.py --compare` of main
before step 1 and this step on the 42 tests (cores 14 and 15, `--count-runs`), geometric means B/A: wall 0.99
interpreted and 1.00 with `--tier=auto`, instructions 1.00 and 1.00, cycles 1.00 and 1.00.

## llvm: native clocked RTL against the interpreter and sim1, cold and warm (#440, #432, #505, 2026-10-21)

Measurement only, nothing changed in the product. Release build with `--features llvm` of main at `3abd2fd7`, single
thread, every timed run pinned to performance core 4 (`taskset -c 4`, builds on 8 to 23), median of 5, interleaved with
the order swapped every round, host load average about 9 to 10 (wall time carries up to 10% noise: compare the
instruction counts, `perf stat -e cpu_core/instructions/`). Wall time is the whole `yavs run` (elaborate + initialise +
run + exit; analysis excluded) and sim1 is `-e` then `-r` (elaboration included). Each yavs cell is a fresh work
library analysed beforehand: "cold" is the first run in it (empty object cache), "warm" the second run in it (machine code
linked from the cache, #505). `--tier=llvm` compiles everything before time 0; `--tier=auto` tiers up on a compile
thread that shares the one pinned core. Cells are `wall s (ratio to sim1)` and `instructions G (ratio to sim1)`. The 10x
length is the default generic times ten (`-gCYC=` or `-gCYCLES=`). Script: a 60 line driver over `tools/bench/run.py`'s
`timed`, in the author's scratch directory (not committed); `run.py --ref` itself cannot pass generics to sim1 or time
cold and warm apart.

### Normal length (defaults)

| Design | interp | auto cold | auto warm | llvm cold | llvm warm | sim1 |
|---|---:|---:|---:|---:|---:|---:|
| rtl_ff wall s | 0.92 (1.46) | 0.87 (1.38) | 0.81 (1.29) | 0.87 (1.38) | 0.85 (1.35) | 0.63 |
| rtl_ff instr G | 15.69 (1.99) | 16.12 (2.04) | 15.90 (2.01) | 15.97 (2.02) | 15.62 (1.98) | 7.89 |
| rtl_lfsr wall s | 0.46 (2.09) | 0.41 (1.86) | 0.37 (1.68) | 0.42 (1.91) | 0.36 (1.64) | 0.22 |
| rtl_lfsr instr G | 8.23 (2.85) | 8.07 (2.80) | 7.62 (2.64) | 7.78 (2.69) | 7.23 (2.51) | 2.89 |
| rtl_chain wall s | 0.20 (1.33) | 0.21 (1.40) | 0.22 (1.47) | 0.29 (1.93) | 0.24 (1.60) | 0.15 |
| rtl_chain instr G | 4.34 (2.15) | 4.44 (2.20) | 4.45 (2.20) | 5.41 (2.68) | 5.06 (2.50) | 2.02 |
| rtl_fan wall s | 1.11 (1.02) | 1.15 (1.06) | 1.14 (1.05) | 1.30 (1.19) | 1.25 (1.15) | 1.09 |
| rtl_fan instr G | 16.95 (1.01) | 17.85 (1.06) | 17.85 (1.06) | 21.02 (1.25) | 20.91 (1.25) | 16.78 |
| rtl_bus wall s | 0.21 (0.36) | 0.22 (0.38) | 0.22 (0.38) | 0.25 (0.43) | 0.21 (0.36) | 0.58 |
| rtl_bus instr G | 4.57 (0.55) | 4.65 (0.56) | 4.66 (0.56) | 5.00 (0.60) | 4.67 (0.56) | 8.32 |
| rtl_ram wall s | 0.08 (0.89) | 0.09 (1.00) | 0.09 (1.00) | 0.99 (11.00) | 0.10 (1.11) | 0.09 |
| rtl_ram instr G | 1.10 (1.94) | 1.14 (2.02) | 1.16 (2.05) | 11.91 (21.14) | 1.37 (2.43) | 0.56 |
| lfsr64 wall s | 0.23 (2.56) | 0.23 (2.56) | 0.25 (2.78) | 0.25 (2.78) | 0.22 (2.44) | 0.09 |
| lfsr64 instr G | 4.65 (2.95) | 4.75 (3.02) | 4.77 (3.03) | 5.13 (3.26) | 4.76 (3.02) | 1.58 |
| lfsr64_xor wall s | 0.10 (0.83) | 0.11 (0.92) | 0.11 (0.92) | 0.19 (1.58) | 0.11 (0.92) | 0.12 |
| lfsr64_xor instr G | 1.99 (2.32) | 2.04 (2.39) | 2.06 (2.41) | 2.93 (3.42) | 2.14 (2.51) | 0.86 |

### 10x length

| Design | interp | auto cold | auto warm | llvm cold | llvm warm | sim1 |
|---|---:|---:|---:|---:|---:|---:|
| rtl_ff wall s | 9.06 (2.71) | 8.20 (2.46) | 8.27 (2.48) | 8.16 (2.44) | 7.86 (2.35) | 3.34 |
| rtl_ff instr G | 155.11 (2.23) | 154.69 (2.23) | 154.48 (2.22) | 154.25 (2.22) | 153.91 (2.22) | 69.46 |
| rtl_lfsr wall s | 4.65 (4.95) | 3.29 (3.50) | 3.17 (3.37) | 3.00 (3.19) | 2.92 (3.11) | 0.94 |
| rtl_lfsr instr G | 81.50 (3.26) | 70.32 (2.81) | 69.86 (2.79) | 71.40 (2.85) | 70.86 (2.83) | 25.03 |
| rtl_chain wall s | 1.80 (1.91) | 1.91 (2.03) | 1.94 (2.06) | 2.32 (2.47) | 2.24 (2.38) | 0.94 |
| rtl_chain instr G | 42.54 (2.50) | 43.26 (2.54) | 43.27 (2.55) | 49.51 (2.91) | 49.16 (2.89) | 17.00 |
| rtl_fan wall s | 12.25 (1.44) | 12.04 (1.41) | 12.40 (1.46) | 13.90 (1.63) | 13.26 (1.56) | 8.51 |
| rtl_fan instr G | 168.00 (1.06) | 176.71 (1.11) | 176.73 (1.11) | 207.30 (1.30) | 207.19 (1.30) | 159.13 |
| rtl_bus wall s | 2.50 (0.42) | 2.46 (0.41) | 2.42 (0.41) | 2.40 (0.40) | 2.37 (0.40) | 5.93 |
| rtl_bus instr G | 45.16 (0.56) | 45.68 (0.57) | 45.69 (0.57) | 45.93 (0.57) | 45.61 (0.57) | 80.23 |
| rtl_ram wall s | 0.44 (3.14) | 0.59 (4.21) | 0.53 (3.79) | 1.60 (11.43) | 0.55 (3.93) | 0.14 |
| rtl_ram instr G | 7.36 (3.55) | 8.54 (4.12) | 8.08 (3.90) | 19.16 (9.25) | 8.64 (4.17) | 2.07 |
| lfsr64 wall s | 2.55 (3.86) | 2.64 (4.00) | 2.73 (4.14) | 2.46 (3.73) | 2.40 (3.64) | 0.66 |
| lfsr64 instr G | 45.00 (3.44) | 45.74 (3.49) | 45.76 (3.50) | 45.79 (3.50) | 45.42 (3.47) | 13.09 |
| lfsr64_xor wall s | 1.03 (4.12) | 1.04 (4.16) | 1.05 (4.20) | 1.10 (4.40) | 1.01 (4.04) | 0.25 |
| lfsr64_xor instr G | 18.35 (4.12) | 18.59 (4.18) | 18.61 (4.18) | 19.85 (4.46) | 19.07 (4.29) | 4.45 |

### Fixed costs: a run of one clock cycle (`-gCYC=1`)

Wall seconds, same method (tier start-up, cold compile, cache link). The first column of each pair is `3abd2fd7`, the second
main with `9d5a2438` (#501 part 2) rebuilt afterwards; only `rtl_ram` moved beyond the 10 ms noise.

| Design | interp | auto cold | auto warm | llvm cold | llvm warm | sim1 |
|---|---:|---:|---:|---:|---:|---:|
| rtl_ff | 0.02 / 0.01 | 0.03 / 0.01 | 0.03 / 0.01 | 0.07 / 0.05 | 0.03 / 0.02 | 0.35 |
| rtl_lfsr | 0.01 / 0.00 | 0.01 / 0.00 | 0.02 / 0.00 | 0.08 / 0.06 | 0.02 / 0.01 | 0.04 |
| rtl_chain | 0.01 / 0.00 | 0.01 / 0.00 | 0.02 / 0.01 | 0.06 / 0.05 | 0.02 / 0.01 | 0.06 |
| rtl_fan | 0.01 / 0.01 | 0.02 / 0.01 | 0.02 / 0.01 | 0.04 / 0.03 | 0.03 / 0.02 | 0.33 |
| rtl_bus | 0.00 / 0.00 | 0.01 / 0.00 | 0.01 / 0.00 | 0.06 / 0.05 | 0.02 / 0.01 | 0.03 |
| rtl_ram | 0.05 / 0.04 | 0.06 / 0.04 | 0.06 / 0.04 | 1.16 / 0.54 | 0.07 / 0.06 | 0.06 |
| lfsr64 | 0.02 / 0.01 | 0.02 / 0.01 | 0.03 / 0.01 | 0.08 / 0.06 | 0.03 / 0.02 | 0.04 |
| lfsr64_xor | 0.02 / 0.01 | 0.02 / 0.01 | 0.03 / 0.01 | 0.12 / 0.09 | 0.03 / 0.02 | 0.04 |

Instructions of the one-cycle run: interp 0.06 to 0.38 G, `auto` cold +0.03 to +0.05 G (sampling and the compile
thread), warm +0.01 G on top, `llvm` cold +0.15 to +0.9 G (+11 G on `rtl_ram` before `9d5a2438`, +7 G after: 0.54 s of
compile for a 0.04 s design), `llvm` warm +0.05 to 0.17 G over the interpreter (object link, 10 to 20 ms). sim1 spends
0.2 to 1 G on elaboration of the 4096 and 4000 process designs (0.33 to 0.35 s), which is why the short rows of `rtl_ff`
and `rtl_fan` look better for yavs than their steady state.

### What native RTL buys

Native frames (6cadf280, 71032993) do not apply to processes: a process body stays in frame mode (crates/yavs-llvm
AGENTS.md; `0 functions callable on native frames` in the `--tier=llvm --verbose` note of all eight designs but
`rtl_ram`). Under `--tier=auto` only a process of 140 to 600 IR instructions without a loop is compiled (`auto.rs`
`rtl`): of the eight designs that is `rtl_ff` (149), `rtl_lfsr` (262) and `rtl_ram` (277). `rtl_fan` (44 instructions,
printed as "stays interpreted"), `rtl_chain` and `rtl_bus` (under 140) and `lfsr64`, `lfsr64_xor` (a loop in the process:
with the default heat they are not compiled in 2.6 s of run; with `--tier-heat=1,0,0` they are) run the interpreter under
`auto`, so their `auto` columns are the interpreter's plus the sampling cost.

Compared with the interpreter, in instructions (the wall times agree within noise): `rtl_ff` -0.8% (155.1 to 153.9 G at
10x), `rtl_lfsr` -13% (81.5 to 70.9 G), `lfsr64` +1%, `rtl_bus` +1%, `rtl_chain` +16%, `rtl_fan` +23% (`llvm`), `rtl_ram`
+17%, `lfsr64_xor` +4%. Ratio to sim1 at 10x, best of interp / auto / llvm warm (instructions; wall in brackets):
`rtl_ff` 2.22 (2.35), `rtl_lfsr` 2.79 (3.11), `rtl_chain` 2.50 (1.91, interpreter), `rtl_fan` 1.06 (1.41), `rtl_bus` 0.56
(0.40), `rtl_ram` 3.55 (3.14, interpreter), `lfsr64` 3.44 (3.64), `lfsr64_xor` 4.12 (4.04). So native RTL closes 0 to 13%
of the instruction gap to sim1 on the designs where it is used and widens it on the others; the warm cache removes
only the cold compile (40 to 130 ms per design, 0.5 s on `rtl_ram`), not any steady-state cost. Cold against warm at
10x differs by less than the noise on every design but `rtl_ram` (`llvm` 1.60 against 0.55 s) and `rtl_chain` (`llvm`
2.32 against 2.24 s).

### Where an activation's instructions go (`--tier=llvm`, warm; sampling profile `perf record -e cpu_core/instructions/upp`
at 25 M instruction intervals, the 10x runs, symbols grouped; instructions per activation = total / activations; sim1
total for the same unit last)

| Design | interp total | interp: body (`run_ops`) | llvm total | compiled body | native entry/exit | activation wrapper | callbacks (drive, wait, host pointer) | kernel | sim1 |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| rtl_ff | 1893 | 1043 | 1879 | 329 | 393 | 275 | 355 | 527 | 848 |
| rtl_lfsr | 3184 | 1590 | 2768 | 612 | 485 | 272 | 381 | 1017 | 978 |
| rtl_chain | 2127 | 546 | 2458 | 220 | 370 | 291 | 339 | 1231 | 850 |
| rtl_fan | 840 | 450 | 1036 | 138 | 345 | 357 | 40 | 155 | 796 |
| lfsr64 | 3515 | 1438 | 3548 | 653 | 526 | 253 | 832 | 1251 | 1023 |

"Native entry/exit" is `lower::enter`, `Host::view_again`, the `NSLOTS` thread-local table push and pop, `run_cached`,
the scratch borrow and release and the `memset` of it; "activation wrapper" is `Machine::run_quick` and
`InterpProcess::run` (177 to 216 in the interpreter rows, 253 to 357 in the compiled ones: the wrapper itself grows by
about 100); callbacks are `yavs_rt_drive`, `Host::drive_now`/`drive_block`,
`yavs_rt_wait`, `Host::wait_again` and `yavs_rt_host` (the host-pointer lookup of a load or store compiled code does not
inline: 110 per activation in `rtl_ff`, 502 in `lfsr64`); kernel is everything in `yavs_kernel`. The interpreter's
`run_ops` is absent from every compiled row (0.0 to 0.1%): at steady state no instruction is executed by the
interpreter, there are no side exits into it on these designs, and the "executed by the interpreter" counts in the
`--verbose` note are callbacks and one-off instructions. Only `rtl_ram` leaves instructions to the interpreter at run
time (#507). The activation count is `N x CYC` of the design (`rtl_chain` and `rtl_fan` have a few extra processes;
`lfsr64` is 64 processes).

The compiled body is 2.3x to 3.5x cheaper than the interpreted one (`rtl_ff` 1043 to 329), and then the entry and
exit (about 370 to 530), the larger activation wrapper (+100) and the drive and wait callbacks (340 to 830) give all of
it back. A native `rtl_ff` activation costs 329 body + 393 entry + 275 wrapper + 355 callbacks = 1352 outside the
kernel, the interpreted one 1043 body + 177 wrapper + about 160 drive and wait = 1380: the totals (1879 and 1893) are
level. The kernel (527 to 1251 per
activation; 155 on `rtl_fan`, where a wake is one scalar) is the same in both tiers and by itself equals or exceeds
sim1's whole activation on `rtl_ff`, `rtl_lfsr`, `rtl_chain` and `lfsr64`.

### Levers, ranked by expected gain

1. A lean activation path for process bodies. Entry plus wrapper is 660 to 780 instructions per activation (`rtl_ff`
   668, `rtl_fan` 702, `lfsr64` 779) against 177 to 216 for the interpreter's wrapper alone. A process has no native-frame
   callee in these designs, so the `NSLOTS` table (77 per activation in `rtl_ff`), the frame-view rebuild (88), the
   scratch borrow, `memset` and release (about 100) and `run_cached`/`enter` (about 160) are paid for nothing. Getting entry
   plus wrapper down to about 250 saves 400 per activation: `rtl_ff` 21%, `rtl_fan` 38% (it would then beat the
   interpreter and could join `auto`), `rtl_chain` 16%, `lfsr64` 11%. Expected wall gain 10 to 30% on compiled designs.
2. Fewer callbacks per activation: the host-pointer lookup `yavs_rt_host` (110 in `rtl_ff`, 502 in `lfsr64`, 4% of
   `rtl_lfsr`) should come from base addresses kept in the process's frame view (instance data, heap objects the process
   already holds, constants), and the scalar zero-delay drive and the repeated wait could be inline in the body with the
   kernel call only when the queue is touched (`drive_now` 131 plus `yavs_rt_wait` 46 plus `wait_again` 34 in `rtl_ff`).
   Expected 150 to 500 per activation (8% `rtl_ff`, 14% `lfsr64`).
3. The kernel itself (28 to 50% of the instructions): the compiled tier cannot touch it and it is the largest bucket on
   four designs (#440 continues). With levers 1 and 2 done and nothing else, `rtl_ff` would sit at about 1300 (329 body, 250
   entry and wrapper, about 200 callbacks, 527 kernel) against sim1's 848: ratio 1.5 instead of 2.2, and the remaining
   gap is the kernel and the body. Policy item on the way: `auto` should
   count instructions it would leave to the interpreter before compiling a process (#507) and consider the loop
   processes of `lfsr64` (compiled with `--tier=llvm` at the same speed as interpreted today, so only worth it after
   levers 1 and 2).

Verified: the instruction counts of the one-cycle and normal-length rows of `rtl_ff`, `rtl_fan` and `rtl_ram` on main
with `9d5a2438` (the only commit after `3abd2fd7`) are within 1% of the table; the 10x rows, the profiles and the other
five designs were not repeated on it. Not verified: the number of `yavs_rt_host` calls per activation (inferred from
about 40 instructions per call), why `lfsr64` is not compiled under `auto` with the default heat, the cost split inside
`enter` (symbol level only), and `--threads` above 1.

## llvm: a leaner compiled activation, round 3 (#440, #507, 2026-10-21)

Commits 986f5960 (per-thread native slot table without a borrow), 904edb03 (address facts kept after calls, the drive's
waveform known), 41665859 (`Alloc` zeroed by stores), 049c0e3b (no interpreter entry after an inline native), 8855f019
(one `Mem` per simulation thread), 868e9fac (a process finds its heap objects inline), c27b27b2 (no out-of-line call
after the suspension of a process holding heap memory, both tiers), 58833d90 (a process finds temporaries inline),
120714ac (`--tier=auto` leaves a process that leaves its compiled code too often interpreted, closes #507), 55772743
(`RTL_MIN` 140 to 120). Decisions: docs/decisions.md "Compiled clocked RTL processes, round 3".

Method as in the section above: release build with `--features llvm`, single thread, every timed run pinned to
performance core 4, median of 3; "before" is main at a5db170d (the start of this round), "after" main at 55772743
(which also has the interpreter's round 10 and the IR inliner of #493, so the interpreter columns move too); sim1
re-measured the same way (elaboration and run). Instructions are `perf stat -e cpu_core/instructions/u` of the whole
`yavs run` (elaboration included). Wall time carries more noise than usual (a suite run on the efficiency cores at the
same time): compare the instructions. Cells are instructions in G with the ratio to sim1 in brackets.

| Design (normal length) | interp before / after | auto warm before / after | llvm warm before / after | auto cold after | llvm cold after | sim1 |
|---|---:|---:|---:|---:|---:|---:|
| rtl_ff | 16.14 (2.09) / 15.28 (1.98) | 15.75 (2.04) / 14.42 (1.87) | 15.54 (2.02) / 14.23 (1.85) | 14.71 (1.91) | 14.65 (1.90) | 7.71 |
| rtl_lfsr | 8.36 (3.06) / 8.13 (2.98) | 7.60 (2.79) / 7.01 (2.57) | 7.16 (2.62) / 6.53 (2.39) | 7.58 (2.78) | 7.20 (2.64) | 2.73 |
| rtl_chain | 4.39 (2.25) / 4.31 (2.21) | 4.46 (2.29) / 4.37 (2.25) | 4.99 (2.56) / 4.68 (2.40) | 4.41 (2.27) | 5.19 (2.66) | 1.95 |
| rtl_fan | 17.22 (1.04) / 16.88 (1.02) | 18.09 (1.09) / 17.64 (1.06) | 20.83 (1.26) / 19.54 (1.18) | 17.68 (1.07) | 19.70 (1.19) | 16.57 |
| rtl_bus | 4.53 (0.55) / 4.51 (0.55) | 4.57 (0.56) / 4.55 (0.55) | 4.60 (0.56) / 4.50 (0.55) | 4.59 (0.56) | 4.96 (0.61) | 8.20 |
| rtl_ram | 0.88 (1.86) / 0.87 (1.85) | 0.88 (1.87) / 0.88 (1.86) | 1.11 (2.35) / 1.10 (2.32) | 1.01 (2.14) | 8.45 (17.90) | 0.47 |
| lfsr64 | 4.55 (2.99) / 4.46 (2.93) | 4.62 (3.03) / 4.16 (2.73) | 4.58 (3.01) / 4.09 (2.68) | 4.67 (3.07) | 4.81 (3.16) | 1.52 |
| lfsr64_xor | 1.83 (2.63) / 1.81 (2.60) | 1.85 (2.66) / 1.88 (2.71) | 1.96 (2.82) / 1.83 (2.63) | 2.29 (3.29) | 2.94 (4.24) | 0.69 |

| Design (10x length) | interp before / after | auto warm before / after | llvm warm before / after | auto cold after | llvm cold after | sim1 |
|---|---:|---:|---:|---:|---:|---:|
| rtl_ff | 160.14 (2.31) / 151.62 (2.19) | 154.20 (2.23) / 141.04 (2.04) | 153.76 (2.22) / 140.65 (2.03) | 141.29 (2.04) | 141.07 (2.04) | 69.27 |
| rtl_lfsr | 83.29 (3.34) / 80.98 (3.24) | 69.73 (2.79) / 63.52 (2.54) | 70.76 (2.83) / 64.42 (2.58) | 64.15 (2.57) | 65.09 (2.61) | 24.96 |
| rtl_chain | 43.55 (2.57) / 42.73 (2.53) | 44.20 (2.61) / 43.38 (2.56) | 49.07 (2.90) / 45.98 (2.72) | 43.42 (2.57) | 46.49 (2.75) | 16.92 |
| rtl_fan | 171.24 (1.08) / 167.84 (1.06) | 179.69 (1.13) / 175.29 (1.10) | 207.00 (1.30) / 194.18 (1.22) | 175.33 (1.10) | 194.33 (1.22) | 158.91 |
| rtl_bus | 45.20 (0.56) / 45.01 (0.56) | 45.64 (0.57) / 45.39 (0.57) | 45.52 (0.57) / 44.52 (0.56) | 45.44 (0.57) | 44.94 (0.56) | 80.13 |
| rtl_ram | 7.18 (3.56) / 7.13 (3.53) | 7.80 (3.87) / 7.21 (3.57) | 8.37 (4.15) / 8.13 (4.03) | 7.30 (3.62) | 15.47 (7.66) | 2.02 |
| lfsr64 | 45.38 (3.49) / 44.49 (3.42) | 46.03 (3.54) / 39.10 (3.00) | 45.22 (3.47) / 40.22 (3.09) | 39.58 (3.04) | 40.95 (3.15) | 13.01 |
| lfsr64_xor | 18.15 (4.13) / 17.90 (4.07) | 18.33 (4.17) / 16.87 (3.84) | 18.84 (4.29) / 17.48 (3.98) | 17.39 (3.96) | 18.59 (4.23) | 4.39 |

Per activation, `--tier=llvm` warm at 10x length, sampling profile as before (`cpu_core/instructions/upp`, symbols
grouped the same way: "native entry/exit" is `enter`, `view_again`, `run_cached` and the scratch release, "activation
wrapper" `Machine::run_quick` and `InterpProcess::run`, "callbacks" the drive, wait, host-pointer, signal and step
callbacks with the host functions they call), before (the section above) / after:

| Design | llvm total | compiled body | native entry/exit | activation wrapper | callbacks | kernel | interp total after |
|---|---:|---:|---:|---:|---:|---:|---:|
| rtl_ff | 1879 / 1717 | 329 / 350 | 393 / 319 | 275 / 262 | 355 / 269 | 527 / 515 | 1851 |
| rtl_lfsr | 2768 / 2518 | 612 / 547 | 485 / 365 | 272 / 296 | 381 / 280 | 1017 / 1018 | 3163 |
| rtl_chain | 2458 / 2301 | 220 / 217 | 370 / 285 | 291 / 301 | 339 / 249 | 1231 / 1243 | 2136 |
| rtl_fan | 1036 / 971 | 138 / 76 | 345 / 301 | 357 / 329 | 40 / 82 | 155 / 182 | 839 |
| lfsr64 | 3548 / 3154 | 653 / 887 | 526 / 373 | 253 / 310 | 832 / 310 | 1251 / 1235 | 3476 |

(`lfsr64`'s body grew because the heap-object lookups that were `yavs_rt_host` calls, 502 per activation, are now inline
in it; `rtl_fan`'s compiled-body and callback columns move between each other with the edge test.) Entry and exit went
from 345-526 to 285-373 instructions per activation; the wrapper did not move (262-329): together 580-680, not the
250 aimed at. What the remaining entry costs: the thread scratch swap of the registers (about 130 per activation; a
compiled process that keeps its own registers saves those instructions but ran 18% more cycles on `rtl_ff` and 14% on
`rtl_lfsr`, see decisions.md), the frame view (`view_root`, 75: 25 fields from the machine), `enter` (about 90:
`Mem`, `Env`, the object record) and the interpreter's `run_quick`/`run_compiled` checks and `Host` (about 150). The
callbacks are the drive (`yavs_rt_drive` 42 plus `drive_now` with the kernel's assignment, 135) and the wait
(`yavs_rt_wait` 40, `wait_again` 38, `wait_on_again` 20): layering, not work.

#507: `--tier=auto` no longer compiles the testbench process of `rtl_ram` (a note says 33 of its 258 instructions per
activation leave the compiled code): 10x 7.80 G before, 7.21 G after against 7.13 G interpreted. Its compiled form also
got cheaper (`--tier=llvm` 8.37 to 8.13 G, the temporaries a native leaves found inline), but stays slower than the
interpreter (stepped natives, layouts, signal reads, temporary roots). `--tier=llvm` cold costs more on `rtl_ram` (8.45 G
against 7.95 G at normal length: the inline lookups are more code to compile).

Library testbenches under `--tier=auto` (instructions, core 4, warm, median of 3 to 9, before / after): UVVM
`rand_tb` 16.10 / 15.98 G (interp 15.99), `generic_sb_slv_tb` 19.23 / 18.70 G (interp 15.59: auto 18% behind the
interpreter before and after, string helpers entered from interpreted code; #510), `generic_queue_tb` 0.68 / 0.67 G,
`axistream_vvc_tb` 63.2 / 62.4 G (interp 65.2). No regression.

Checks at 55772743: fmt, clippy with and without `--features llvm`, `cargo test --workspace`, `cargo test -p yavs-llvm -p
yavs --features llvm` and the `gc-stress` unit tests of the crate; difftest 426/426 for the interpreter, `--tier=llvm`,
`--tier=auto` and `--tier=auto --tier-heat=1,0,0`, at 1 and 4 threads, cold and warm cache; `external.py --oracle
--check` under `--tier=auto`: sim1 857/857, sim2 393/393; `tools/suites/pro.py` under `--tier=auto` 314/314 pass;
`tools/suites/uvvm.py` 102 pass, 9 fail, 26 skip under both `--tier=auto` and the interpreter, the same runs; the h2h
designs and four internal benchmarks print the same under interp, llvm, auto and auto with eager heat.

Not verified: `--threads` above 1 for the timings; aarch64 and Windows beyond CI; wall times on an idle host.

## llvm: cold compiles, where cold minus warm goes (#740, 2026-10-25)

Breakdown of head-to-head 4 (41 tests, summed seconds, 192 cold against 139 warm): library analysis one call per file 37 (cold parses, warm finds the units; about 13 M user instructions a file at the median,
flat profile), run phase 13.6 (two tests), elaboration 2.0. Compile cost of `--tier=auto` (window generator, 55 compiles): code generation 62 percent, the `LEAN` passes 34 (of them `instcombine` 78: quadratic in the
register buffer of the exits), the rest 4; the store of an object costs nothing measurable (no `fsync`). Changes: the passes without the alloca walk, exits that store fewer registers (decisions.md "Cold compiles").
Compile CPU of the same 54 functions of `vai_window_gen_interlock` 2 292 to 1 667 ms (-27 percent). Wall of cold runs, medians of 5 interleaved runs, core 12, host load 20 to 28, before / after: window generator
14.87 / 14.12 s (instructions 207 / 200 G), `poc_stat_histogram` 1.60 / 1.47 s, `fletcher_bufferwriter_512` 5.37 / 4.96 s, `poc_arith_addw` 9.58 / 9.37 s, neorv32 single min and cache, `hdlm_lfsr` within noise;
`uvvm_util_rand` 1.15 / 1.30 s (instructions 14.0 / 15.6 G: which functions the run compiles by the time it ends depends on when the compiles finish, #673; a variant that prices compiles at half
gave 17.5 G). Warm runs: instructions equal on every test (window generator 147.2 / 146.4 G), wall within the noise of the loaded host.
Not done, the larger cuts: the cold analysis of the libraries (a policy and a start-up question: #738, #553), a cheaper first compile level (1 to 3 percent slower code, `FastRa` pattern), the price constant
`US_PER_INSTR` (800) against 260 to 670 measured per IR instruction (posted to #728). Not verified: the full head-to-head basis (the loaded host made 41-test cold totals meaningless at this size).

## Analysis in batched calls: integrations, tools and the head-to-head method (#745, 2026-10-25)

User decision: integrations analyse consecutive files of one library and standard in one `yavs analyze F1 F2 ...` call by default (`docs/decisions.md` "Batched analysis calls"). Changed: `tools/corpus/run.py`
(yavs batches, a failing call is repeated file by file, `--per-file` for the old form; all 34 manifests give identical results and output either way; whole corpus 34 s against 47 s on a busy host),
`tools/realbench` analysis cells (both tools, `--analyze-calls batched|per-file`), `tools/bench/run.py` preparation, `docs/usage.rst`, `docs/cli-contract.md`. `tools/suites` already batched; `tools/difftest`,
`tools/discrepancies`, `tiny.py` and the release smoke test analyze one file by construction. The test runner's side of the yavs support (described in `docs/test-runner-requirements.md`) is not in the repository, so it is unchanged; the generic
`--batch-compile` is the open VUnit pull request #1236 (opt-in flag).

Method for the head-to-head (root rule 23): the analysis cells call BOTH tools the same way, consecutive files of one library, standard and flag set per call, with one rule more: a file that defines a unit an earlier file of the call
defined starts a new call (the corpus manifests put several projects into one library and the reference rejects a unit replaced within one call). The reference is called through its multi-file form (`{file}` expands to the files
in its untracked `corpus_cmd`); its own integration in the runner is one file per call, so the old basis (head-to-head 1 to 5) was one call per file for both tools. The two bases are not comparable; a table says which it is
(`--analyze-calls per-file` reproduces the old one). The untimed set-up of the worlds stays one file per call. The same note is kept with the head-to-head results outside the repository.

Realbench analysis cells, 8 tests (`hdlm_fifo_sync`, `neorv32_single_min`, `olo_base_cam`, `osvvm_axis_random`, `poc_strings`, `sc5_timestamp`, `uvvm_util_rand`, `vunit_logging`), same bench build (no `llvm` feature), core 12, 5 runs per
cell, two passes per mode with the lower one kept, host load 12 to 14. Wall seconds, one call per file / batched:

| test | yavs cold | yavs warm | sim1 cold |
|---|---|---|---|
| `hdlm_fifo_sync` | 1.15 / 0.45 | 0.55 / 0.20 | 4.09 / 0.77 |
| `neorv32_single_min` | 0.28 / 0.14 | 0.11 / 0.02 | 0.89 / 0.11 |
| `olo_base_cam` | 1.97 / 0.65 | 1.01 / 0.17 | 6.61 / 0.74 |
| `osvvm_axis_random` | 3.55 / 0.78 | 1.92 / 0.92 | 11.05 / 1.60 |
| `poc_strings` | 0.64 / 0.28 | 0.34 / 0.08 | 2.13 / 0.24 |
| `sc5_timestamp` | 1.05 / 0.31 | 0.54 / 0.11 | 3.00 / 0.27 |
| `uvvm_util_rand` | 0.41 / 0.26 | 0.09 / 0.02 | 1.01 / 0.49 |
| `vunit_logging` | 0.98 / 0.50 | 0.52 / 0.24 | 3.29 / 0.70 |

Geometric mean of batched / one call per file: yavs 0.40 cold and 0.27 warm in wall time (instructions 0.48 and 0.34), sim1 0.16 cold and warm (instructions 0.26). So the reference gains more than yavs, and the analysis
ratio yavs / sim1 (geometric mean of the 8) rises from 0.32 to 0.80 cold and from 0.14 to 0.23 warm; yavs cold is behind sim1 on `neorv32_single_min`, `poc_strings` and `sc5_timestamp` (#747); the warm cell of `osvvm_axis_random` (0.92 s) is as slow as its cold cell (0.78 s) because the OSVVM testbench libraries redefine units (#748). Not verified: the other 34
realbench tests' analysis cells timed (their batched call lists were built and ran without a failing call for both tools, untimed), the full head-to-head basis, the effect on the run phases (none expected: libraries are
the same).

## Batched analysis: shared work paid once per call (#747, #748, 2026-10-25)

Decisions and causes in `docs/decisions.md` "Batched analysis: shared work paid once per call". Changed: the library index is written once per call (`Session::record_deferred` and `flush`), the
entry store counts its puts instead of listing the directory at each, and a `Db` keeps the entries, source hashes and library indexes it read (`EntryCache`), so the files of a call that share a large dependency
closure stop reading it again. Tests: `deferred_records_are_published_by_the_flush` (yavs-lib), `counted_puts_still_prune` (yavs-cache), `a_batch_after_an_edit_reads_nothing_stale_and_records_what_it_did` (units.rs).

`tools/realbench --compare` of the bench build before and after, analysis phase, batched calls, all 42 tests, 5 runs, one claimed core, host load 6 (so wall times are higher than on a quiet host). After / before,
geometric mean over the tests: cold instructions 0.91, cold wall 0.81, warm instructions 0.40, warm wall 0.35 (36 tests with a measurable warm wall). Largest changes (wall, seconds, before / after):

| test | cold | warm | cold G instr | warm G instr |
|---|---|---|---|---|
| `osvvm_axis_random` | 1.37 / 0.77 | 1.81 / 0.18 | 5.81 / 5.45 | 5.70 / 1.33 |
| `osvvm_axi4_random` | 0.74 / 0.65 | 0.91 / 0.20 | 5.87 / 5.45 | 5.68 / 1.33 |
| `sc5_timestamp` | 0.32 / 0.22 | 0.10 / 0.02 | 2.88 / 1.93 | 0.56 / 0.19 |
| `hdlm_fifo_sync` | 0.56 / 0.40 | 0.19 / 0.10 | 3.83 / 3.64 | 1.14 / 0.53 |
| `olo_base_cam` | 0.67 / 0.55 | 0.18 / 0.07 | 5.65 / 5.09 | 1.04 / 0.38 |
| `poc_strings` | 0.24 / 0.19 | 0.08 / 0.02 | 1.62 / 1.36 | 0.42 / 0.15 |
| `neorv32_single_min` | 0.10 / 0.08 | 0.01 / 0.00 | 1.04 / 0.93 | 0.14 / 0.06 |
| `uvvm_util_rand` | 0.22 / 0.22 | 0.01 / 0.00 | 3.01 / 2.98 | 0.21 / 0.09 |

Against sim1 (batched calls for both, yavs / sim1 wall, before / after, runs on different host loads): `hdlm_fifo_sync` cold 0.72 / 0.56, warm 0.28 / 0.12; `neorv32_single_min` 1.00 / 0.80, 0.18 / 0.00;
`olo_base_cam` 0.97 / 0.69, 0.26 / 0.06; `osvvm_axis_random` 0.41 / 0.37, 0.48 / 0.11; `poc_strings` 0.81 / 0.82, 0.27 / 0.09; `sc5_timestamp` 1.25 / 0.71, 0.36 / 0.09; `uvvm_util_rand` 0.50 / 0.46, 0.04 / 0.00;
`vunit_logging` 0.86 / 0.50, 0.28 / 0.11. Geometric mean of the 8: cold 0.77 to 0.59, warm 0.23 to 0.05 (a floor of 0.01 for the warm cells that round to 0.00). The warm cell of `osvvm_axis_random` is cheaper than its cold cell again
(0.16 s against 0.55 s). All 42 against sim1 with the final build (host load 12): cold geometric mean 0.63 (instructions 0.60), warm 0.08; 38 tests below 1 cold, all below 1 warm; at par or above cold: `fletcher_bufferreader`, `fletcher_bufferwriter_512`, `fletcher_bufferwriter_prim32` (1.00) and `en_cl_fix_pkg` (1.10). Not verified: a quiet-host rerun of all 42 against sim1; the interpreter, run and elab phases
(nothing in them changed); Windows and aarch64 (the code is portable, CI covers it).

## Cold analysis: unit entries and records created in place (#747, 2026-10-26)

Decisions, measurements and what was tried in `docs/decisions.md` "Cold analysis: unit entries and records created in place". Changed: the unit store (`yavs-cache`) and the library records (`yavs-lib`) create
a file under its own name instead of a temporary file and a rename; `Store::payload_len` checks the header's length. Tests: `a_cut_short_unit_entry_is_replaced` (yavs-cache),
`a_cut_short_record_is_rewritten_and_an_intact_one_is_kept` (yavs-lib). Cold analysis of the seven small-library tests of #747, B/A of the bench builds: instructions 0.98, CPU 0.91, wall 0.98; yavs / sim1 cold
analysis wall 0.62 to 0.89 (`en_cl_fix_pkg` 0.82, `poc_strings` 0.62, `sc5_timestamp` 0.79, `neorv32_single_min` 0.73 to 0.80, the `fletcher_*` 0.89), instructions 0.52 to 0.89; the at-par readings in #747 came from a host at load 12. The cold total (analysis plus
elaboration plus run, `--tiers auto-cold`) is below sim1 on `en_cl_fix_pkg`, `poc_strings`, `neorv32_single_min` and `fletcher_bufferwriter_prim32` and above it on `sc5_timestamp` (0.59 s against 0.53), `fletcher_bufferreader` (1.64 against 1.13)
and `fletcher_bufferwriter_512` (4.1 against 1.9): there the run phase is the gap (#519, #523, #740), not the analysis. Not verified: Windows (the fast path is Unix-only), other file systems than ext4.

## Cold minus warm, by phase (#740, 2026-10-27)

Where the cold column of head-to-head 7 loses against the warm one, split per test from its raw cells (cold minus warm of
the analysis cells, of `interp0` = start-up to time 0 without compiling, and the rest of the `auto` run cell = compile CPU
on the pinned core plus the time interpreted before compiled code arrives), weighted by the change of the log ratio, so
by its weight in the geometric mean: **analysis of the libraries 75 percent, start-up (closure, elaboration, lowering)
19 percent, compiling and its late arrival 6 percent**. The short testbenches (about 30 of 41) lose almost all of their
cold factor in analysis; the tests above sim1 cold (`sc5_fifo_repack`, `poc_stat_histogram`, `vai_window_gen_interlock`,
`rtl_lfsr`, `lfsr64`) lose it in the run cell: the `--tier=auto` policy compiles their processes 0.2 to 8 s into the run
(the compile CPU itself is 7 to 17 percent of their cold CPU). Halving every price of the policy (`YAVS_AUTO_MODEL`,
compile cost unchanged) moved `poc_stat_histogram` 0.87 to 0.77 s, `sc5_fifo_repack` and the window generator not at all,
so a cheaper code generator alone does not close those; the policy is #679 (#800 filed: a second compile of a function
is priced 10 to 20 times below its cost).

Found and fixed: the analysed-unit entries were keyed by the session's `--relaxed`, so a relaxed run of a design whose
libraries were analysed without it (the open-logic, satcat5, window-generator and GRLIB tests) read no library unit and
analysed the closure from source in its first two runs. `--relaxed` is now a question a unit asks only where the
setting changes its analysis (`docs/decisions.md` "`--relaxed` is a question"): window generator cold run 68 unit
misses to 10 (the rest are bodies with assignments to unconstrained targets, which do depend on it). `tools/realbench
--compare` (15033264's change against its parent, bench builds, 42 tests, core 9, load 10 to 25): analysis cold 0.97,
elab cold 0.98 (instructions 0.96; `vai_weight_buffer` 0.65, `vai_pe_array` 0.72, `sc5_timestamp` instructions 0.70),
`auto-cold` 0.99, `auto` 1.00, no slowdown; full total (analysis + run) 0.975 cold, `vai_pe_array` 1.11 to 0.95 s,
`vai_weight_buffer` 0.74 to 0.64 s. Against sim1 of head-to-head 7 (other session) the cold geometric mean moves
0.578 to 0.564.

Also found: entries of files analysed by relative path are never read by `elab`/`run` (#799); in `tools/realbench`
the libraries' std/ieee entries live in the corpus tool's store, so a run of `--phases run` alone without the analysis
cells misses std/ieee units the analysis cells would have stored (the full suite is not affected).

## Real-world performance suite: yavs against sim1, sim2 and sim4, head-to-head 8, 2026-10-28 (`tools/realbench`)

Current headline: `1a8edb76` (main with the Cranelift tier behind the `clift` feature), method as head-to-head 7 (batched analysis calls for the tools, `--tier=auto`, total wall incl. analysis, 5 timed
runs, two complete passes merged by the best cell, three claimed cores, controls of the three long tests at the start and the end; the raw data, scripts and logs are kept locally, not committed).
Two builds against sim1: the bench build with the `llvm` feature (the h2h7 build; two passes, 09:08 to 10:03, host load average 2 to 12 with a median of 5 to 6, of which about 3 is the run's own timed
processes: the closest to a quiet host this day) and the bench build with the `clift` feature (the `-nollvm` archive: Cranelift is the compiled tier of `--tier=auto`; passes at 10:08 and 11:38, load median 17 and 25 with bursts to
200 and more from other jobs, and interactive use of the host from 10:32). Geometric mean over the same 41 tests, ratio yavs / sim1 (in brackets head-to-head 7):

| basis | LLVM build warm | LLVM build cold | Cranelift build warm | Cranelift build cold |
|---|---:|---:|---:|---:|
| **total, libraries analysed too** | **0.201 (0.215)** | **0.576 (0.587)** | 0.206 | 0.571 |
| own files + elab + run | 0.211 (0.219) | 0.481 (0.489) | 0.213 | 0.473 |
| elab + run only | 0.216 (0.224) | 0.479 (0.483) | 0.215 | 0.468 |
| with `grlib_leon3mp` (42) | 0.214 (0.229) | 0.597 (0.610) | 0.219 | 0.594 |
| executed instructions (run cell), yavs / sim1 | 0.29 (0.30) | 0.59 (0.59) | not counted | not counted |

Summed over the 41 tests the full total is 34.0 s (yavs) against 64.0 s (sim1) warm and 51.3 s against 64.0 s cold (h2h7: 37.9 / 70.4 and 56.9 / 69.9); faster than sim1: 37 of 41 warm, 34 of 41 cold
(36, 33). Cranelift build: 37.8 s against 69.2 s warm, 52.3 s against 68.8 s cold, faster on 36 and 34. The two LLVM passes agree (0.203 and 0.197 warm, 0.577 and 0.578 cold); the repeat pass of the Cranelift
build at load 25 gave 0.212 / 0.573, the merged cell (best of the two) is in the table. Instruction sums, run cell warm: 449 G (yavs) against 514 G (sim1). The h2h designs (10, `xor_sig` left out): 0.50 warm, 0.62 cold
(0.50 and 0.61 at h2h7); with `xor_sig` (11): 0.44 and 0.56. `rtl_lfsr` 1.37 / 1.95, `lfsr64` 1.12 / 1.41, `rtl_ff` 1.06 / 1.23 stay above sim1; `rtl_chain` 0.72, `rtl_ram` 0.91, `rtl_fan` 0.84.
Controls (run cell, sim1 / yavs seconds), start then end: `fletcher_bufferwriter_512` 1.77 / 3.59 then 1.91 / 3.90; `poc_arith_addw` 5.08 / 4.76 then 5.32 / 4.96; `vai_window_gen_interlock` 3.72 / 6.61
then 3.98 / 7.12 (the end ran at load 6 to 7: 5 to 8 percent up for both tools).

Tests above sim1 (LLVM build, full total warm / cold; Cranelift build in brackets): `grlib_leon3mp` 2.48 / 2.63 (2.89 / 2.91; outside the 41), `fletcher_bufferwriter_512` 1.89 / 2.03 (1.93 / 2.09),
`vai_window_gen_interlock` 1.51 / 2.21 (1.49 / 1.77), `fletcher_bufferreader` 1.27 / 1.35 (1.26 / 1.35), `sc5_fifo_repack` 1.08 / 2.03 (1.25 / 1.77); cold only: `poc_stat_histogram` 0.58 / 1.69 (1.00 / 1.29 with
Cranelift: warm level), `poc_arith_addw` 0.91 / 1.07 (0.98 / 1.01), `sc5_timestamp` 0.77 / 1.10 (0.69 / 1.00 with Cranelift). Warm above 1: 4 of the 41 plus grlib (h2h7: 5 plus grlib); cold above 1: 7 of the 41 with the LLVM build
(h2h7: 8), 6 with the Cranelift build. Movers since h2h7: `poc_stat_histogram` 1.04 to 0.58 warm (yavs 0.55 to 0.30 s); sim1 itself got 9 percent cheaper on the quiet host (sum of the 41 warm 70.4 to 64.0 s), yavs 10 percent
(37.9 to 34.0 s): `neorv32_dual_full` 2.02 to 1.75 s, `neorv32_single_min` 2.62 to 2.36 s, `uvvm_axistream_vvc` 2.53 to 2.16 s, `poc_arith_addw` 5.21 to 4.86 s, `fletcher_bufferwriter_512` 3.99 to 3.55 s, `vai_window_gen_interlock`
8.07 to 7.45 s. The ratios of the longest tests moved the other way where sim1 gained more: `vai_window_gen_interlock` 1.29 to 1.51 (sim1 6.24 to 4.93 s, the h2h7 sim1 cell had run at load 5 to 10), `uvvm_axistream_vvc` 0.52 to 0.59
(sim1 4.83 to 3.65 s). The 30 short tests are level (deltas of 0.00 to 0.02).

### Against sim2 (open-source, mcode code generator) and sim4 (a commercial simulator)

Added at the request of the maintainer; each is a separate pass of `tools/realbench --ref simN --phases analyze,run` with the Cranelift build, every pair timed on the same core (sim4: one claimed core, strictly one
test at a time, the single licence seat; the libraries of both are analysed once untimed, as their normal workflow would; neither has a compile cache, so cold and warm differ only in the analysis cells). Local, untracked
wrappers in the ignored `tools/sims.local.conf` (sim2 is run with `--elab-run` and `-frelaxed`, which the UVVM and Open Logic sources need; sim4 with one `vsim -c -do` script per test). **These passes ran while the host was in
interactive use (from 10:32) and other jobs ran: load median 20, yavs's own seconds 30 to 50 percent above the quiet LLVM pass (sum of the 41 warm: 51 s against 34 s), so only the ratios are used;** the
same ratios came out of the first passes with the LLVM build (sim2 0.171 / 0.475, sim4 0.159 / 0.398) within 0.011.

| reference | tests | warm | cold | faster warm / cold | sums warm (yavs / ref) |
|---|---:|---:|---:|---:|---:|
| sim2 | 41 | 0.182 | 0.482 | 41 / 40 | 51.1 / 237.0 s |
| sim4 | 29 | 0.164 | 0.397 | 24 / 22 | 36.5 / 93.7 s |
| (sim1, the same Cranelift build) | 41 | 0.206 | 0.571 | 36 / 34 | 37.8 / 69.2 s |

The same 29 tests against sim1: 0.234 / 0.607; against sim2: 0.192 / 0.472. sim4 elaboration plus run only (no analysis): 0.28 warm on 40 tests (its start-up is about 0.3 s per test); own files + elab + run on the 29:
0.30 / 0.58. n/a cells: `grlib_leon3mp` (sim2 exits 1 after about 100 s at elaboration or run in this setup, cause not looked at; sim4 reports 16 compile errors in the GRLIB libraries);
sim4 rejects `sine_calculator.vhd` (`'latency` of a user attribute on an entity, COMP96_0361: the testbench does not run); sim4 analyses 3 hdl-modules library groups with errors (`hard_fifo` needs the vendor primitives
library, `sine_generator`), which fails the analysis cells of the 8 other `hdlm_*` tests (their elab + run exists: 0.3 to 0.9 s); the analysis of the own files of the three `vai_*` tests fails under sim4 in this setup
(not looked at; elab + run exists, `vai_window_gen_interlock` 5.4 s). Slower than sim4 (Cranelift build, warm / cold): `fletcher_bufferwriter_512` 1.77 / 2.05, `poc_arith_addw` 1.34 / 1.44,
`neorv32_dual_full` 1.32 / 1.43, `neorv32_single_cache` 1.41 / 1.64, `neorv32_single_min` 1.29 / 1.44; cold only `fletcher_bufferreader` 1.11, `sc5_fifo_repack` 1.09. Above sim2: only `sc5_fifo_repack` cold 1.08.

Per test (full total ratio yavs / reference; "h2h7" is the same cell of head-to-head 7 and "delta" the change, LLVM build; the last three columns are the Cranelift build against sim1, sim2 and sim4, warm / cold):

| test | warm | h2h7 | delta | cold | h2h7 | delta | yavs warm s | sim1 warm s | Cranelift build warm / cold | sim2 warm / cold | sim4 warm / cold |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| grlib_leon3mp | 2.48 | 2.67 | -0.19 | 2.63 | 2.84 | -0.22 | 33.40 | 13.48 | 2.89 / 2.91 | n/a | n/a |
| fletcher_bufferwriter_512 | 1.89 | 1.98 | -0.09 | 2.03 | 2.15 | -0.13 | 3.55 | 1.88 | 1.93 / 2.09 | 0.09 / 0.10 | 1.77 / 2.05 |
| vai_window_gen_interlock | 1.51 | 1.29 | +0.22 | 2.21 | 1.98 | +0.23 | 7.45 | 4.93 | 1.49 / 1.77 | 0.60 / 0.77 | n/a |
| fletcher_bufferreader | 1.27 | 1.36 | -0.09 | 1.35 | 1.43 | -0.08 | 1.44 | 1.13 | 1.26 / 1.35 | 0.28 / 0.31 | 1.00 / 1.11 |
| sc5_fifo_repack | 1.08 | 1.12 | -0.04 | 2.03 | 1.97 | +0.05 | 1.16 | 1.07 | 1.25 / 1.77 | 0.79 / 1.08 | 0.81 / 1.09 |
| poc_arith_addw | 0.91 | 0.95 | -0.04 | 1.07 | 1.12 | -0.04 | 4.86 | 5.34 | 0.98 / 1.01 | 0.12 / 0.13 | 1.34 / 1.44 |
| hdlm_lfsr | 0.79 | 0.84 | -0.05 | 0.97 | 1.07 | -0.10 | 1.34 | 1.70 | 0.78 / 0.97 | 0.20 / 0.25 | n/a |
| sc5_timestamp | 0.77 | 0.72 | +0.05 | 1.10 | 1.02 | +0.08 | 0.40 | 0.52 | 0.69 / 1.00 | 0.14 / 0.19 | 0.26 / 0.37 |
| neorv32_single_min | 0.70 | 0.70 | -0.00 | 0.78 | 0.80 | -0.02 | 2.36 | 3.37 | 0.71 / 0.79 | 0.42 / 0.44 | 1.29 / 1.44 |
| poc_cache_cpu | 0.62 | 0.65 | -0.03 | 0.72 | 0.74 | -0.03 | 0.60 | 0.96 | 0.60 / 0.72 | 0.61 / 0.75 | 0.52 / 0.60 |
| uvvm_axistream_vvc | 0.59 | 0.52 | +0.07 | 0.72 | 0.61 | +0.11 | 2.16 | 3.65 | 0.60 / 0.69 | 0.41 / 0.49 | 0.75 / 0.88 |
| poc_stat_histogram | 0.58 | 1.04 | -0.46 | 1.69 | 2.00 | -0.31 | 0.30 | 0.52 | 1.00 / 1.29 | 0.65 / 0.85 | 0.47 / 0.63 |
| neorv32_single_cache | 0.53 | 0.55 | -0.02 | 0.62 | 0.65 | -0.03 | 2.05 | 3.88 | 0.56 / 0.63 | 0.34 / 0.39 | 1.41 / 1.64 |
| uvvm_sb_slv | 0.46 | 0.44 | +0.01 | 0.68 | 0.62 | +0.06 | 0.63 | 1.38 | 0.40 / 0.63 | 0.54 / 0.87 | 0.26 / 0.40 |
| uvvm_util_rand | 0.45 | 0.44 | +0.01 | 0.69 | 0.70 | -0.01 | 0.73 | 1.61 | 0.42 / 0.69 | 0.58 / 0.87 | 0.28 / 0.49 |
| neorv32_dual_full | 0.41 | 0.44 | -0.03 | 0.52 | 0.53 | -0.02 | 1.75 | 4.24 | 0.44 / 0.49 | 0.33 / 0.37 | 1.32 / 1.43 |
| osvvm_axi4_random | 0.24 | 0.25 | -0.01 | 0.45 | 0.46 | -0.01 | 0.68 | 2.81 | 0.25 / 0.45 | 0.25 / 0.45 | 0.11 / 0.22 |
| poc_fifo_ic_assembly | 0.24 | 0.26 | -0.02 | 0.50 | 0.50 | +0.00 | 0.10 | 0.42 | 0.24 / 0.51 | 0.25 / 0.52 | 0.10 / 0.21 |
| olo_fix_cordic_rot | 0.16 | 0.16 | +0.00 | 0.49 | 0.50 | -0.01 | 0.21 | 1.35 | 0.16 / 0.51 | 0.18 / 0.57 | 0.09 / 0.26 |
| osvvm_axis_random | 0.15 | 0.15 | -0.01 | 0.37 | 0.39 | -0.02 | 0.34 | 2.34 | 0.15 / 0.38 | 0.14 / 0.34 | 0.07 / 0.17 |
| hdlm_axis_bfm | 0.13 | 0.14 | -0.00 | 0.41 | 0.42 | -0.02 | 0.12 | 0.89 | 0.13 / 0.42 | 0.17 / 0.53 | n/a |
| fletcher_bufferwriter_prim32 | 0.12 | 0.12 | +0.00 | 0.25 | 0.24 | +0.01 | 0.08 | 0.67 | 0.11 / 0.24 | 0.27 / 0.62 | 0.07 / 0.17 |
| hdlm_axi_fifo | 0.12 | 0.12 | +0.00 | 0.42 | 0.42 | -0.00 | 0.12 | 1.02 | 0.11 / 0.43 | 0.14 / 0.51 | n/a |
| hdlm_axi_crossbar | 0.11 | 0.12 | -0.01 | 0.41 | 0.43 | -0.02 | 0.12 | 1.06 | 0.11 / 0.43 | 0.13 / 0.51 | n/a |
| hdlm_ring_buffer | 0.11 | 0.12 | -0.01 | 0.38 | 0.40 | -0.02 | 0.10 | 0.92 | 0.10 / 0.41 | 0.14 / 0.52 | n/a |
| uvvm_func_cov | 0.10 | 0.11 | -0.01 | 0.41 | 0.40 | +0.01 | 0.13 | 1.25 | 0.11 / 0.39 | 0.21 / 0.76 | 0.08 / 0.33 |
| hdlm_sine_generator | 0.10 | 0.12 | -0.01 | 0.37 | 0.38 | -0.00 | 0.10 | 0.98 | 0.09 / 0.38 | 0.15 / 0.53 | n/a |
| hdlm_unsigned_divider | 0.10 | 0.12 | -0.02 | 0.40 | 0.41 | -0.01 | 0.09 | 0.90 | 0.09 / 0.40 | 0.13 / 0.53 | n/a |
| vunit_axi_dma | 0.10 | 0.11 | -0.01 | 0.45 | 0.44 | +0.01 | 0.09 | 0.90 | 0.10 / 0.44 | 0.10 / 0.41 | 0.05 / 0.19 |
| hdlm_register_file | 0.10 | 0.11 | -0.01 | 0.39 | 0.42 | -0.03 | 0.10 | 1.01 | 0.10 / 0.41 | 0.13 / 0.49 | n/a |
| hdlm_fifo_sync | 0.10 | 0.10 | -0.00 | 0.40 | 0.43 | -0.03 | 0.09 | 0.92 | 0.09 / 0.43 | 0.14 / 0.52 | n/a |
| vai_weight_buffer | 0.09 | 0.09 | -0.00 | 0.41 | 0.46 | -0.04 | 0.11 | 1.24 | 0.10 / 0.42 | 0.12 / 0.55 | n/a |
| en_cl_fix_pkg | 0.09 | 0.10 | -0.02 | 0.47 | 0.47 | +0.01 | 0.05 | 0.58 | 0.10 / 0.48 | 0.10 / 0.49 | 0.04 / 0.20 |
| poc_uart_rx | 0.08 | 0.08 | +0.00 | 0.42 | 0.42 | +0.00 | 0.03 | 0.36 | 0.08 / 0.43 | 0.14 / 0.61 | 0.03 / 0.16 |
| vunit_logging | 0.08 | 0.09 | -0.01 | 0.39 | 0.39 | +0.00 | 0.07 | 0.85 | 0.08 / 0.39 | 0.10 / 0.45 | 0.04 / 0.16 |
| vai_pe_array | 0.08 | 0.09 | -0.01 | 0.33 | 0.38 | -0.05 | 0.14 | 1.75 | 0.10 / 0.35 | 0.13 / 0.53 | n/a |
| olo_base_cam | 0.07 | 0.07 | -0.00 | 0.42 | 0.43 | -0.01 | 0.09 | 1.28 | 0.07 / 0.43 | 0.08 / 0.53 | 0.04 / 0.22 |
| olo_fix_madd | 0.07 | 0.07 | -0.00 | 0.44 | 0.44 | -0.00 | 0.08 | 1.15 | 0.06 / 0.45 | 0.09 / 0.53 | 0.04 / 0.21 |
| olo_intf_i2c_master | 0.07 | 0.07 | -0.00 | 0.51 | 0.52 | -0.01 | 0.07 | 1.01 | 0.07 / 0.51 | 0.08 / 0.54 | 0.04 / 0.21 |
| poc_strings | 0.07 | 0.07 | +0.00 | 0.48 | 0.48 | +0.00 | 0.02 | 0.29 | 0.07 / 0.50 | 0.12 / 0.71 | 0.03 / 0.16 |
| olo_axi_master_full | 0.07 | 0.08 | -0.01 | 0.42 | 0.44 | -0.01 | 0.09 | 1.33 | 0.07 / 0.44 | 0.10 / 0.56 | 0.04 / 0.22 |
| uvvm_methods_log | 0.01 | 0.02 | -0.01 | 0.45 | 0.45 | +0.00 | 0.00 | 0.51 | 0.02 / 0.46 | 0.02 / 0.41 | 0.01 / 0.19 |

Not verified: the Cranelift passes were not on a quiet host (see above); sim2 and sim4 have one pass each with the Cranelift build; the cause of the sim2 `grlib_leon3mp` and the sim4 `vai_*` analysis failures was not
looked at; the output of the references was not compared with yavs's beyond the exit status.

## Real-world performance suite: yavs against sim1, head-to-head 7, 2026-10-27 (`tools/realbench`)

Current headline: `5b2beb73`, bench build with the `llvm` feature, method exactly as head-to-head 6 (batched analysis calls for both tools, `--tier=auto`, total wall incl. analysis, 5 timed
runs, two complete passes merged by the best cell, three claimed cores 9 to 11, controls of the three long tests at the start and the end). Host load average 5 to 10 (other agents; no quiet window). Raw
results are not committed. Geometric mean over the same 41 tests, ratio yavs / sim1 (in brackets the scan of 2026-10-26 on `1585475c`, then head-to-head 6):

| basis | total warm | total cold |
|---|---:|---:|
| **total, libraries analysed too** | **0.22 (0.21; 0.22)** | **0.59 (0.59; 0.64)** |
| own files + elab + run | 0.22 (0.22; 0.23) | 0.49 (0.49; 0.49) |
| elab + run only | 0.22 (0.23; 0.23) | 0.48 (0.48; 0.47) |
| executed instructions (run cell), yavs / sim1 | 0.30 (0.30; 0.31) | 0.59 (0.60; 0.60) |

Summed over the 41 tests the full total is 38 s (yavs) against 70 s (sim1) warm and 57 s against 70 s cold (42 / 72 and 60 / 71 in the scan, 43 / 76 and 64 / 76 at head-to-head 6). Faster than sim1: 36 of 41 warm,
33 of 41 cold. The two passes agree (0.213 and 0.214 warm, 0.587 and 0.582 cold). The h2h designs (10 that were counted before; `xor_sig` now runs in sim1 and is 0.12 warm, 0.20 cold): 0.50 warm, 0.61 cold
(0.54 and 0.65 in the scan, 0.56 and 0.78 at head-to-head 6): `rtl_chain` 1.21 to 0.70 warm, `rtl_ff` 1.24 to 1.02, `rtl_lfsr` 1.43 to 1.35, `lfsr64` 1.04 to 1.10 (sim1 itself moved 145 to 129 ms).

Above sim1 (full total warm / cold, run-cell instruction ratio): `grlib_leon3mp` 2.67 / 2.84 (2.3; outside the 41), `fletcher_bufferwriter_512` 1.98 / 2.15 (1.8), `fletcher_bufferreader` 1.36 / 1.43 (1.3),
`vai_window_gen_interlock` 1.29 / 1.98 (1.7), `sc5_fifo_repack` 1.12 / 1.97 (1.7), `poc_stat_histogram` 1.04 / 2.00 (2.8); cold only: `poc_arith_addw` 0.95 / 1.12, `hdlm_lfsr` 0.84 / 1.07,
`sc5_timestamp` 0.72 / 1.02; designs `rtl_lfsr` 1.35 / 1.86, `lfsr64` 1.10 / 1.37, `rtl_ff` 1.02 / 1.21. Movers since the scan: `vai_window_gen_interlock` yavs 10.0 to 8.1 s (ratio 1.83 to 1.29; sim1 5.5 to
6.2 s, host), `uvvm_axistream_vvc` 0.63 to 0.52 (3.0 to 2.5 s), `neorv32_single_min` 0.78 to 0.70, `uvvm_sb_slv` 0.49 to 0.44, `poc_arith_addw` 5.6 to 5.2 s; the 30 short tests are level. The controls
(run cell, sim1 / yavs seconds) were at the start 1.85 / 4.03, 5.23 / 5.19, 5.88 / 7.53 and at the end 1.91 / 3.93, 6.01 / 5.85, 7.81 / 9.26 (`fletcher_bufferwriter_512`, `poc_arith_addw`,
`vai_window_gen_interlock`; the end ran at load 10), so absolute seconds of the late cells carry drift for both tools.

## Real-world performance suite: yavs against sim1, head-to-head 6, 2026-10-26 (`tools/realbench`)

This was the headline of head-to-head 6 (`d3123c88`, bench build with the `llvm` feature, one binary for every cell; the previous run, head-to-head 5 at `9e75d84e`, is in brackets where it is comparable). **The basis of the
analysis cells changed (#745): both tools now analyse consecutive files of one library and standard in ONE call (`--analyze-calls batched`, the realbench default; the shared-work cost fix is `66219d63`), where
head-to-head 1 to 5 used one call per file.** The two bases are not comparable, so the tables give both: "batched" is the new headline, "per-file" is the old basis measured once (one pass of the analysis cells with
`--analyze-calls per-file`, the run cells taken from the batched passes, they are the same cells). Basis otherwise unchanged (root rule 23): `--tier=auto` only, TOTAL wall as a user runs it (analysis of every file of
the compile order plus elaboration and run, against sim1's analysis plus elaboration plus run); cold = empty compilation cache and fresh libraries, warm = cache present. Ratios are yavs / sim1, below 1 means yavs
is faster. Method exactly as head-to-head 5: 5 timed runs per cell, interleaved, two complete passes, the lower pass median per cell for both tools, a sim1-only control at the start and the end; one timed process per
core, the cores held through `tools/claim core` locks (9, 10, 11 and 12 of a Ryzen 9 7950X3D, SMT siblings idle; core 8 was kept away because another agent timed on it without a lock), host load average 7 to 28.
Raw results are not committed. A first attempt was discarded after 4 minutes because core 8 was shared with an unlocked timing job.

Geometric mean over the same 41 tests, ratio yavs / sim1 (the per-file basis of this run, then head-to-head 5, in brackets):

| basis | total warm | total cold |
|---|---:|---:|
| **total, libraries analysed too, batched calls** | **0.22 (0.24; 0.23)** | **0.64 (0.42; 0.43)** |
| own files + elab + run | 0.23 (0.24; 0.25) | 0.49 (0.46; 0.49) |
| elab + run only | 0.23 (0.23; 0.24) | 0.47 (0.47; 0.50) |
| executed instructions (run cell), yavs / sim1 | 0.31 (same cells; 0.32) | 0.60 (same cells; 0.60) |

Head-to-head 4 (per-file basis) had 0.25 and 0.46 for the total, 0.27 and 0.52 for own + elab + run, 0.26 and 0.53 for elab + run and 0.35 and 0.65 for the instructions. With `grlib_leon3mp` (VHDL-93 mode, kept out
of the means to keep the basis) the full total is 0.23 warm and 0.66 cold batched (0.26 and 0.44 per-file, 0.25 and 0.45 at head-to-head 5, 0.27 and 0.48 at head-to-head 4).

What the basis change does: batching helps sim1 far more than yavs, because sim1 pays a large fixed cost per call. Summed over the 41 tests the analysis (library + own files) is, per-file to batched, 156 s to 25 s
for sim1 and 25 s to 2 s for yavs warm, 154 s to 24 s for sim1 and 49 s to 14 s for yavs cold. The warm analysis ratio improves (0.15 to 0.08 geomean) because yavs' warm analysis is nearly free; the cold analysis
ratio rises from 0.33 to 0.63, because yavs parses every library file once (14 s of the 64 s cold total) while sim1's analysis fell to 24 s. With sim1's analysis cheap the fixed run phase weighs more in the total,
so the total cold ratio rises from 0.42 to 0.64 and the total warm ratio stays at 0.22 to 0.24; own + elab + run and elab + run, which batching barely touches, did not move (0.49 and 0.47 cold). Summed over the 41
tests the full total on the batched basis is 43 s (yavs) against 76 s (sim1) warm and 64 s against 76 s cold (per-file basis: 66 against 207 s and 99 against 205 s; head-to-head 5: 60 against 188 s and 92 against
187 s). The executed instructions sum to 545 G (yavs) against 522 G (sim1) (552 G against 522 G). Tests faster than sim1: batched 36 of 41 warm and 33 of 41 cold; per-file basis 40 and 39 (39 and 39 at head-to-head 5).

By category (batched; in brackets the per-file basis of this run, then head-to-head 5):

| category | n | total warm | total cold | own+elab+run warm | own+elab+run cold |
|---|---:|---:|---:|---:|---:|
| **overall** | 41 | 0.22 (0.24; 0.23) | 0.64 (0.42; 0.43) | 0.23 (0.24; 0.25) | 0.49 (0.46; 0.49) |
| bus-tristate | 1 | 0.06 (0.15; 0.14) | 0.56 (0.31; 0.32) | 0.06 (0.06; 0.06) | 0.32 (0.32; 0.33) |
| cpu | 3 | 0.59 (0.52; 0.56) | 0.67 (0.61; 0.63) | 0.60 (0.60; 0.66) | 0.66 (0.66; 0.70) |
| fsm | 4 | 0.31 (0.22; 0.21) | 0.70 (0.39; 0.37) | 0.41 (0.41; 0.44) | 0.56 (0.56; 0.61) |
| memory | 4 | 0.16 (0.20; 0.18) | 0.77 (0.38; 0.39) | 0.18 (0.21; 0.21) | 0.59 (0.49; 0.50) |
| numeric | 8 | 0.17 (0.21; 0.21) | 0.61 (0.38; 0.39) | 0.18 (0.20; 0.20) | 0.42 (0.39; 0.40) |
| protected | 8 | 0.19 (0.22; 0.22) | 0.50 (0.41; 0.45) | 0.23 (0.26; 0.27) | 0.46 (0.44; 0.48) |
| records | 7 | 0.15 (0.20; 0.18) | 0.52 (0.35; 0.35) | 0.14 (0.15; 0.16) | 0.35 (0.34; 0.34) |
| rtl-datapath | 2 | 0.37 (0.38; 0.38) | 0.94 (0.61; 0.69) | 0.31 (0.33; 0.38) | 0.73 (0.66; 0.74) |
| strings-textio | 2 | 0.07 (0.15; 0.14) | 0.51 (0.33; 0.31) | 0.04 (0.04; 0.05) | 0.21 (0.21; 0.24) |
| wide-vectors | 2 | 1.93 (0.93; 0.83) | 2.17 (1.10; 1.10) | 2.23 (2.23; 2.10) | 2.38 (2.38; 2.48) |

The h2h designs (10 that sim1 runs): 0.56 warm, 0.78 cold (0.71, 0.83 at head-to-head 5): the two designs that had lost their warm speedup are back, `compute_h2h` 477 to 153 ms warm (ratio 0.15 to 0.05) and
`xor_types` 374 to 130 ms (0.40 to 0.13), `xor_sig` 377 to 183 ms (#749); the others are level (`rtl_ff` 1.22 to 1.20, `rtl_fan` 0.85 to 0.83, `lfsr64_xor` 0.80 to 0.72, `rtl_lfsr` 1.59 to 1.60, `lfsr64` 1.42 to 1.42,
`rtl_chain` 1.24 to 1.29, `rtl_ram` 0.88 to 0.87, `rtl_bus` 0.28 to 0.28). Slower than sim1 warm: `rtl_lfsr`, `lfsr64`, `rtl_chain`, `rtl_ff`. The tiny-test corpus: the same 397 tests that both tools run (127 dropped
because a tool rejects or fails them): 0.114 warm, 0.245 cold (0.117 and 0.263 at head-to-head 5; analysis is one file per call here, unaffected by the basis change); all 397 faster than sim1 warm and 393 cold;
medians 4.6 against 38.3 ms warm. Per test both tools were 10 to 13 percent slower than at head-to-head 5 (yavs x1.10 warm, sim1 x1.13), the host.

Repeatability and drift. The single batched passes give a full total of 0.219 and 0.222 warm and 0.663 and 0.629 cold (merged 0.217 and 0.638). Tests whose two passes differ by more than 20 percent in the full
ratio: `vunit_logging`, `uvvm_util_rand`, `uvvm_axistream_vvc`, `sc5_fifo_repack`, `sc5_timestamp`, `poc_fifo_ic_assembly` (warm), `poc_strings`, `poc_uart_rx`, `fletcher_bufferwriter_prim32` and `poc_fifo_ic_assembly`
(cold), mostly tests whose total is a few tenths of a second. The host was noisier than at head-to-head 5, which the sim1 control shows: the run phase of the three long tests (`fletcher_bufferwriter_512`,
`poc_arith_addw`, `vai_window_gen_interlock`; sim1 / yavs seconds) was at the start 2.03 / 4.46, 5.67 / 5.97, 4.68 / 8.44 (head-to-head 5: 1.88 / 4.20, 5.41 / 5.80, 3.98 / 8.64), in the first pass 2.05 / 4.35,
5.82 / 6.13, 6.09 / 9.47, in the second 1.99 / 4.30, 5.60 / 5.74, 9.42 / 12.81 and at the end 2.77 / 5.62, 7.97 / 8.31, 8.05 / 12.73. So `vai_window_gen_interlock` slowed from 4.7 s (start) to 6.1 s (first pass) to 9.4 s (second pass) for sim1 and from 8.4 to 9.5 to 12.8 s for yavs (both move together,
so its ratio holds, but its absolute seconds and the sums above carry the drift), and the end control is 36 to 72 percent above the start on all three tests: part of the late cells were timed on a loaded host (load 27 at the end). Over the same tests sim1's run phase is x1.15 and yavs' x1.07 against head-to-head 5 (tests over 0.3 s; executed instructions of sim1 unchanged, x1.00). Sim1 slowed more than yavs,
which flatters the ratio slightly; the instruction ratio (0.32 to 0.31 warm), which is load-independent, moved less. A quiet-host rerun was not made.

Worst 10 by warm total ratio on the batched basis, with `grlib_leon3mp` (seconds are medians; run = elaboration + run, yavs vs sim1; the ratios on the per-file basis of this run and at head-to-head 5 in brackets,
instruction ratio yavs / sim1 in the sixth column):

| test | warm | cold | run s | instr | issue |
|---|---:|---:|---|---:|---|
| grlib_leon3mp | 3.31 (2.31; 2.17) | 3.18 (2.24; 2.32) | 48.35 vs 12.98 | 2.6 | #677 |
| fletcher_bufferwriter_512 | 2.06 (1.38; 1.35) | 2.11 (1.44; 1.47) | 4.30 vs 1.99 | 1.9 | #519 |
| sc5_fifo_repack | 1.81 (0.63; 0.52) | 2.23 (0.84; 0.82) | 2.08 vs 0.89 | 2.7 | #519 |
| vai_window_gen_interlock | 1.33 (0.91; 1.06) | 1.82 (1.30; 1.52) | 9.47 vs 6.09 | 2.0 | #520 |
| fletcher_bufferreader | 1.32 (0.76; 0.75) | 1.43 (0.86; 0.87) | 1.63 vs 1.14 | 1.3 | #519, #523 |
| poc_stat_histogram | 1.20 (0.38; 0.38) | 2.45 (0.74) | 0.65 vs 0.33 | 3.4 | none specific |
| poc_arith_addw | 0.99 (0.73; 0.82) | 1.23 (0.97; 0.96) | 5.74 vs 5.60 | 1.0 | #520 |
| hdlm_lfsr | 0.86 (0.45; 0.45) | 1.12 (0.56; 0.60) | 1.52 vs 1.13 | 1.3 | none specific |
| sc5_timestamp | 0.82 (0.30; 0.23) | 1.20 (0.50) | 0.44 vs 0.29 | 1.5 | none specific |
| neorv32_single_min | 0.76 (0.66; 0.76) | 0.83 (0.83) | 2.88 vs 3.70 | 0.8 | #521, #529 |

On the batched basis more tests are above sim1: warm 5 of 41 (`fletcher_bufferwriter_512`, `sc5_fifo_repack`, `vai_window_gen_interlock`, `fletcher_bufferreader`, `poc_stat_histogram`; 3 at head-to-head 5
with `grlib_leon3mp`), cold 8 (also `poc_arith_addw`, `sc5_timestamp`, `hdlm_lfsr`). The new ones are run-bound short tests (run phase 0.4 to 2.1 s, instruction ratio 1.3 to 3.4): sim1's cheap analysis no
longer hides their run phase. The yavs run phase is where the remaining gap is; the analysis of every test is still faster than sim1's. Every cell was
measured with the same bench binary.

## Run phase of `poc_stat_histogram`, `sc5_fifo_repack` and `sc5_timestamp` against sim1 (#757, 2026-10-26)

The three run-bound tests above sim1 in head-to-head 6 and not owned by an earlier issue. Bench build with `llvm`
(`tools/build-bench.sh`), `--tier=auto`, executed instructions of the whole process (`perf stat`, compile thread
included), G; sim1 = elaboration and run of the reference. `tools/realbench --compare`, run phase, medians of 5
interleaved, one claimed core (12) of a loaded host (wall is noisy, the instructions are not):

| test | sim1 | interpreter | auto cold, before / after | auto warm, before / after | run s warm, before / after | cold |
|---|---:|---:|---:|---:|---:|---:|
| poc_stat_histogram | 4.5 | 24.8 | 22.6 / 21.4 | 16.0 / 13.3 | 0.64 / 0.55 | 1.18 / 1.15 |
| sc5_fifo_repack | 12.8 | 36.4 | 36.8 / 36.8 | 31.3 / 31.3 | 2.0 / 1.9 (level) | 2.24 / 2.22 |
| sc5_timestamp | 4.6 | 6.6 | 6.8 / 6.8 | 6.7 / 6.7 | 0.41 / 0.41 | 0.40 / 0.40 |

What the three spend (perf, per clock), and what was done:

- **`poc_stat_histogram`** (the ratio of instructions to sim1 fell from 3.6 to 3.0 warm). Each clock `to_slm` rebuilds a 256 x 8
  matrix and `to_slvv_8` 256 `get_row` calls read it back (2048 elements each way, 2.6 million `get_row` calls in the run);
  sim1 does the same element by element at about 100 instructions an element. Warm, `get_row`, `to_slm` and
  `to_slvv_8` (compiled, native frames) were 70 percent of the instructions, the per-element loops 190 instructions an
  iteration (two object look-ups, checked index arithmetic, four plane words each way). Done: element loops
  on native frames run through a helper (decisions.md "Element loops in compiled code"), and the out-of-line copy of at most 64
  elements no longer stages its blocks. Left: about 1000 instructions of every `get_row` call outside the loop (frame, descriptor,
  result copy, layout; a call of a function returning an unconstrained array), the memo callback of a pure function two
  processes have given up (7 percent of the cycles, #762), and the helper itself (about 1000 instructions for an
  eight-element row). The cold run interprets for its first half second because the three functions are compiled one after
  the other, native and frame-mode part each (426 ms of compile CPU on the pinned core, 176 ms of it the frame-mode halves, which no
  native frame of these functions ever needs; posted on #754).
- **`sc5_fifo_repack`**: no hotspot. Interpreted, 630 million ops at about 50 instructions, 72 percent in the dispatch
  loop: the `uniform` loop of the testbench's `rand_next` (`AddrUp`, `Uniform`, `LoadUp`, `FCmp`, `Br`: 15 percent of the ops),
  loop latches (`CmpBr`, `Bin`, `Br`, `CondBr`), signal reads and slice drives; no single op is above 8 percent. `--tier=llvm` (everything compiled)
  is 22 G against 31 to 36 under `auto`, which closes about half of the gap to sim1: the processes `p_in`, `p_col`, `p_sreg`,
  `p_input` and `p_pack` stay interpreted under `auto` (budget and trial rules, #754). In compiled code 24 percent of the
  cycles are the signal assignment path (`drive_now`, `assign_codes_now`, `yavs_rt_drive`, `drive_bits`) and 12 percent
  `memmove` (not traced to its callers). Nothing changed here. Cold compiles only `rand_next`, at 2.6 s of a 2.8 s run.
- **`sc5_timestamp`**: kernel-bound (55 percent of the samples, 2000 instructions a delta cycle against 1400 in sim1, 3.25
  million cycles, spread evenly over `cycle`, `update`, `run_batch`, `run_until`, `drive`, `commit_out`). Compiled code
  does not help (`--tier=llvm` 5.7 G). Nothing was changed; the earlier kernel rounds (decisions.md "Kernel round 3" and after)
  took the single hotspots.

Compile cost, measured on the histogram's three functions (79, 107 and 153 IR instructions): 54 to 113 ms of CPU per native
compile, 62 percent of it code generation, 288 basic blocks after the passes for the 79-instruction `get_row`; the register
allocator choice moves it by up to 20 percent, FastISel by 27 (rejected for the code it makes). The budget charges a native compile of that
size 75 ms, so the first compile is allowed once the run has shown 460 ms of work (`YAVS_AUTO_TRACE`), which is most of a
cold run of this length. Posted on #740 and #754.

Follow-up (#757, 2026-10-27, decisions.md "Element loops planned in the compiled code itself"): the element-loop helper cost about 1000
instructions a call for an eight-element row (38 percent of the histogram's warm cycles); the loops are now planned in the generated
code and only the elements go through a call. Bench build with `llvm`, one claimed core, `tools/realbench --compare` against the
build before (`f877e389`), 5 runs, `--ref sim1`, run phase (yavs run against sim1 elaboration plus run; the analysis is not in it):

| test | sim1 s | auto warm s, before / after | instructions G | auto cold s, before / after | interp |
|---|---:|---:|---:|---:|---:|
| poc_stat_histogram | 0.34 | 0.56 / 0.48 | 13.0 / 10.4 | 0.99 / 0.92 | level |
| sc5_fifo_repack | 0.88 | 1.31 / 1.31 | 21.9 / 21.5 | 2.13 / 2.13 | level |
| sc5_timestamp | 0.30 | 0.40 / 0.40 | 6.7 / 6.7 | 0.40 / 0.40 | level |
| hdlm_lfsr | 1.14 | 1.42 / 1.39 | 23.0 / 23.0 | 1.55 / 1.53 | level |

The histogram's warm run is 1.65 times sim1's run phase before and 1.41 after (cycles best of 5, interleaved: 2.78 G to 2.24 G). Native-frame
compile of the three functions: LLVM instructions +11 percent for `get_row`, +5 percent for `to_slm`, compile time within noise. What is
left in the histogram: `get_row`, `to_slm`, `to_slvv_8` own code 40 percent of the cycles (native frames are compiled at the cheap level, the
plan's flags and values live in stack slots across the checks' blocks), 10 percent the call that finds the fresh result frame and the
matrix. The other three tests have no element loop: `sc5_fifo_repack` is the signal drive path (`yavs_rt_drive`, `assign_codes_now`
15 percent) and the interpreted processes, `sc5_timestamp` and `hdlm_lfsr` the kernel (`cycle`, `finish_direct`, `assign_codes_now`,
`commit_codes` 25 percent for `hdlm_lfsr`); their run-to-run instruction counts move by 5 percent with the policy's timing-dependent
decisions, so a single pair of runs of `hdlm_lfsr` can differ by that much in either direction. All 41 tests of the run phase `--compare` (auto warm and cold): wall and instruction geomeans 0.99 to 1.00; the four tests flagged once (instructions +6 to +8 percent) were level on a re-run of 5.

Second follow-up (#757, 2026-10-28, decisions.md "Native-frame calls find their fresh frames, results and argument regions inline"): what was
left was the path around each call of `get_row` and `to_slm` (callbacks for the fresh frame, the argument regions and the copy; a held descriptor
region that always missed). Bench builds with `llvm`, `--tier=auto`, against `8e4cd334`. Histogram warm, one claimed core, `perf stat`, interleaved:
instructions 10.42 to 6.97 G, cycles 2.21 to 1.49 G. `tools/realbench`, sim1 on the same cores (total = analysis of the compile order, cold, plus the
run cell, as in head-to-head 7):

| test | sim1 total s | auto warm total s (ratio), before / after | auto cold total s (ratio), before / after | run cell warm s, before / after (sim1) |
|---|---:|---:|---:|---:|
| poc_stat_histogram | 0.55 | 0.60 (1.07) / 0.46 (0.84) | 1.02 (1.82) / 0.95 (1.73) | 0.47 / 0.33 (0.32) |
| sc5_fifo_repack | 1.09 | 1.52 (1.36) / 1.46 (1.34) | 2.31 (2.06) / 2.21 (2.03) | 1.35 / 1.29 (0.84) |

`sc5_fifo_repack` has no element loop and none of its time is in the call path changed here (signal drives and interpreted processes, as above).
All 42 tests of the run phase `--compare` (auto warm and cold, 5 runs, cores 13 to 15): wall geomean 0.97 warm, 1.00 cold, instructions 0.99 and
1.00; no test above 1.05 instructions but `uvvm_methods_log` (0.05 G, x1.06). What is left in the histogram's warm cycles: the plans of the two
element loops at the head of `get_row` and `to_slm` (about 20 percent: overflow and bounds tests of every step), the interpreted processes that
call `to_slvv_8` and `to_slm` (13), the call memo probes of `get_row` from compiled code (4, #762), `yavs_rt_lcopy` of the rows into the result (3).

## Real-world performance suite: yavs against sim1, head-to-head 5, 2026-10-25 (`tools/realbench`)

This was the headline of the fifth run, superseded by head-to-head 6 above (one analysis call per file for both tools; `9e75d84e`, bench build with the `llvm` feature, one binary for every cell; the previous run, head-to-head 4 at `18b290ef`, is in brackets). Basis (root rule 23): `--tier=auto`
only, TOTAL wall as a user runs it, analysis of every file of the compile order (one tool call per file) plus elaboration and run, against sim1's analysis plus elaboration plus run; cold = empty compilation cache and
fresh libraries, warm = cache present. Interpreter times are never compared with sim1. Ratios are yavs / sim1, below 1 means yavs is faster. Method exactly as head-to-head 4: 5 timed runs per cell, interleaved, two
complete passes, the lower pass median per cell for both tools, a sim1-only control at the start and the end; one timed process per core, the cores held through `tools/claim core` locks (9, 10 and 11 of a Ryzen 9
7950X3D, SMT siblings idle), host load average 5 to 33 (mostly 12 to 20). Raw results are not committed.

Geometric mean over the same 41 tests (the full total first, own files + elaboration + run beside it):

| category | n | total warm | total cold | own+elab+run warm | own+elab+run cold |
|---|---:|---:|---:|---:|---:|
| **overall** | 41 | 0.23 (0.25) | 0.43 (0.46) | 0.25 (0.27) | 0.49 (0.52) |
| bus-tristate | 1 | 0.14 (0.17) | 0.32 (0.35) | 0.06 (0.08) | 0.33 (0.36) |
| cpu | 3 | 0.56 (0.57) | 0.63 (0.66) | 0.66 (0.68) | 0.70 (0.73) |
| fsm | 4 | 0.21 (0.23) | 0.37 (0.40) | 0.44 (0.46) | 0.61 (0.62) |
| memory | 4 | 0.18 (0.20) | 0.39 (0.43) | 0.21 (0.22) | 0.50 (0.57) |
| numeric | 8 | 0.21 (0.22) | 0.39 (0.43) | 0.20 (0.21) | 0.40 (0.45) |
| protected | 8 | 0.22 (0.23) | 0.45 (0.44) | 0.27 (0.27) | 0.48 (0.47) |
| records | 7 | 0.18 (0.21) | 0.35 (0.39) | 0.16 (0.16) | 0.34 (0.37) |
| rtl-datapath | 2 | 0.38 (0.44) | 0.69 (0.79) | 0.38 (0.46) | 0.74 (0.84) |
| strings-textio | 2 | 0.14 (0.15) | 0.31 (0.35) | 0.05 (0.06) | 0.24 (0.22) |
| wide-vectors | 2 | 0.83 (0.97) | 1.10 (1.12) | 2.10 (2.53) | 2.48 (2.63) |

| basis | warm | cold |
|---|---:|---:|
| total, libraries analysed too | 0.23 (0.25) | 0.43 (0.46) |
| own files + elab + run | 0.25 (0.27) | 0.49 (0.52) |
| elab + run only | 0.24 (0.26) | 0.50 (0.53) |
| executed instructions (run cell), yavs / sim1 | 0.32 (0.35) | 0.60 (0.65) |

Head-to-head 3 had 0.26 and 0.49 for the total, 0.32 and 0.59 for own + elab + run and 0.43 and 0.79 for the instructions. Summed over the 41 tests the full total is 60 s (yavs) against 188 s (sim1) warm and 92 s
against 187 s cold (76 against 218 s and 118 against 218 s before); the executed instructions sum to 552 G (yavs) against 522 G (sim1), 644 G against 525 G before. 39 tests are faster than sim1 warm and 39 cold
(38 and 38 before). With `grlib_leon3mp` (VHDL-93 mode, kept out of the means above to keep the basis) the full total is 0.25 warm and 0.45 cold (0.27 and 0.48). The h2h designs (10 that sim1 runs): 0.71 warm,
0.83 cold (0.59, 0.82): worse, because of three designs whose warm run no longer beats the cold one, `compute_h2h` (153 to 477 ms warm, ratio 0.04 to 0.15), `xor_types` (221 to 374 ms, 0.22 to 0.40) and `xor_sig`
(220 to 377 ms, sim1 does not run it), issue #749; the others stayed or moved a little (`lfsr64_xor` 0.96 to 0.80, `rtl_ff` 1.25 to 1.22, `rtl_fan` 0.78 to 0.85, `rtl_ram` 0.75 to 0.88).
Slower than sim1 are `rtl_lfsr` (1.59), `lfsr64` (1.42), `rtl_chain` (1.24) and `rtl_ff` (1.22) warm. The tiny-test corpus: 397 tests that both tools run (524 files, 127 dropped because a tool rejects or fails
them): 0.12 warm, 0.26 cold; on the 393 tests common to the fourth run 0.12 and 0.26 against 0.14 and 0.28 there, yavs 30 percent faster per test warm (geomean; medians 4.1 against 5.7 ms) and sim1 18 percent
faster (33.5 against 39.3 ms); all 397 are faster than sim1 warm and 393 cold (the four slower are `ieee/ieee_textio*` and `ieee/fixed_pkg_ops*`).

Repeatability: the single passes give a full total of 0.235 and 0.226 warm and 0.432 and 0.433 cold (merged 0.233 and 0.431). Tests whose two passes differ by more than 20 percent in the full ratio: `vunit_axi_dma`
(warm 0.13 against 0.10) and `hdlm_lfsr` (warm 0.46 against 0.32). Sim1 drift against head-to-head 4, stated so the move can be read: over the same tests sim1's full-total wall is x0.86 warm and x0.86 cold, its run
phase x0.90 (tests over 0.3 s; executed instructions unchanged, x0.99, so this is host speed and load, not different work), while yavs' full total is x0.79 and x0.80 and its executed instructions fell from 644 G to
552 G. So the ratio gain is yavs getting faster (the instruction ratio, load-independent, went 0.35 to 0.32 warm and 0.65 to 0.60 cold) and part of it is hidden by sim1 running faster on a quieter host. The sim1
control (the three long tests `fletcher_bufferwriter_512`, `poc_arith_addw`, `vai_window_gen_interlock`, run phase, same cores, at the start and at the end of the batch, then within the passes) was stable this time:
`fletcher_bufferwriter_512` 1.88, 1.89, 1.84 and 1.97 s (1.97 to 2.03 s before), `poc_arith_addw` 5.41, 5.32, 5.33 and 5.63 s (5.57 to 8.35 s before), `vai_window_gen_interlock` 3.98, 3.89, 4.00 and 3.83 s (4.16 to
6.13 s before).

Worst 10 by warm total ratio, with `grlib_leon3mp` (seconds are medians; run = elaboration + run, yavs vs sim1; the ratio at head-to-head 4 in brackets, instruction ratio yavs / sim1 in the fifth column):

| test | warm | cold | run s | instr | issue |
|---|---:|---:|---|---:|---|
| grlib_leon3mp | 2.17 (2.56) | 2.32 (2.65) | 39.71 vs 11.90 | 2.6 (3.7) | #677 |
| fletcher_bufferwriter_512 | 1.35 (1.45) | 1.47 (1.54) | 4.08 vs 1.84 | 2.0 (2.1) | #519 |
| vai_window_gen_interlock | 1.06 (1.19) | 1.52 (1.77) | 8.40 vs 3.83 | 2.0 (2.6) | #520 |
| poc_arith_addw | 0.82 (1.07) | 0.96 (1.53) | 5.82 vs 5.33 | 1.0 (1.3) | #520 |
| neorv32_single_min | 0.76 (0.71) | 0.83 (0.81) | 2.99 vs 3.26 | 0.9 (0.9) | #521, #529 |
| fletcher_bufferreader | 0.75 (0.79) | 0.87 (0.92) | 1.62 vs 1.07 | 1.4 (1.5) | #519, #523 |
| uvvm_axistream_vvc | 0.69 (0.54) | 0.78 (0.61) | 2.91 vs 3.33 | 1.3 (1.2) | none specific |
| neorv32_single_cache | 0.55 (0.59) | 0.62 (0.69) | 2.48 vs 3.87 | 0.6 (0.7) | #521, #529 |
| sc5_fifo_repack | 0.52 (0.65) | 0.82 (0.82) | 1.73 vs 0.86 | 2.3 (3.2) | #519 |
| hdlm_lfsr | 0.45 (0.43) | 0.60 (0.60) | 1.50 vs 1.04 | 1.5 (1.5) | none specific |

Slower than sim1 are only run-bound tests (`grlib_leon3mp`, `fletcher_bufferwriter_512`, `vai_window_gen_interlock`); `poc_arith_addw` is now faster than sim1 (1.07 to 0.82 warm, instruction ratio 1.3 to 1.0). The
analysis of every test is faster than sim1's. Every cell was measured with the same bench binary. A first attempt on an earlier tip was lost with the host session and discarded; this is a complete rerun.

## Real-world performance suite: yavs against sim1, head-to-head 4, 2026-10-25 (`tools/realbench`)

This was the headline of the fourth run, superseded by head-to-head 5 above (`479463c1`, bench build with the `llvm` feature; the previous run, head-to-head 3 at `4b80445d`, is in brackets). Basis (root rule 23): `--tier=auto` only, TOTAL wall as a user runs
it, analysis of every file of the compile order (one tool call per file) plus elaboration and run, against sim1's analysis plus elaboration plus run; cold = empty compilation cache and fresh libraries, warm = cache
present. Interpreter times are never compared with sim1. Ratios are yavs / sim1, below 1 means yavs is faster. Method exactly as head-to-head 3: 5 timed runs per cell, interleaved, two complete passes, the lower pass
median per cell for both tools; one timed process per core on the physical cores 13, 14 and 15 of a Ryzen 9 7950X3D (the other timing cores belonged to other work), host load average 15 to 20. The interpreter cell of the
previous runs was not repeated (it takes no part in any ratio). Raw results are not committed.

Geometric mean over the same 41 tests (the full total first, own files + elaboration + run beside it):

| category | n | total warm | total cold | own+elab+run warm | own+elab+run cold |
|---|---:|---:|---:|---:|---:|
| **overall** | 41 | 0.25 (0.26) | 0.46 (0.49) | 0.27 (0.32) | 0.52 (0.59) |
| bus-tristate | 1 | 0.17 (0.16) | 0.35 (0.33) | 0.08 (0.13) | 0.36 (0.45) |
| cpu | 3 | 0.57 (0.74) | 0.66 (0.82) | 0.68 (0.88) | 0.73 (0.97) |
| fsm | 4 | 0.23 (0.25) | 0.40 (0.43) | 0.46 (0.55) | 0.62 (1.10) |
| memory | 4 | 0.20 (0.19) | 0.43 (0.42) | 0.22 (0.22) | 0.57 (0.52) |
| numeric | 8 | 0.22 (0.24) | 0.43 (0.48) | 0.21 (0.27) | 0.45 (0.51) |
| protected | 8 | 0.23 (0.22) | 0.44 (0.45) | 0.27 (0.29) | 0.47 (0.48) |
| records | 7 | 0.21 (0.21) | 0.39 (0.41) | 0.16 (0.24) | 0.37 (0.44) |
| rtl-datapath | 2 | 0.44 (0.40) | 0.79 (0.67) | 0.46 (0.41) | 0.84 (0.71) |
| strings-textio | 2 | 0.15 (0.16) | 0.35 (0.34) | 0.06 (0.06) | 0.22 (0.23) |
| wide-vectors | 2 | 0.97 (1.12) | 1.12 (1.33) | 2.53 (3.05) | 2.63 (3.10) |

| basis | warm | cold |
|---|---:|---:|
| total, libraries analysed too | 0.25 (0.26) | 0.46 (0.49) |
| own files + elab + run | 0.27 (0.32) | 0.52 (0.59) |
| elab + run only | 0.26 (0.30) | 0.53 (0.61) |
| executed instructions (run cell), yavs / sim1 | 0.35 (0.43) | 0.65 (0.79) |

Summed over the 41 tests the full total is 76 s (yavs) against 218 s (sim1) warm and 118 s against 218 s cold (90 against 228 s and 130 against 224 s before); 38 tests are faster than sim1 warm and 38 cold (37 and 37
before). With `grlib_leon3mp` (VHDL-93 mode, kept out of the means above to keep the basis) the full total is 0.27 warm and 0.48 cold (0.28 and 0.51). The h2h designs (10 that sim1 runs): 0.59 warm, 0.82 cold (0.65, 0.80);
slower than sim1 are `rtl_lfsr` (1.57), `lfsr64` (1.43), `rtl_ff` (1.25) and `rtl_chain` (1.21) warm. The tiny-test corpus: 393 tests that both tools run (520 files, 127 dropped because a tool rejects or fails
them): 0.14 warm, 0.28 cold; on the 381 tests common to the third run 0.14 and 0.28 against 0.12 and 0.26 there, with yavs 7 percent slower per test (5.7 against 5.3 ms warm) and sim1 7 percent faster (38.9 against 44.0 ms),
so this basis moved by about 15 percent against yavs; not investigated.

Repeatability: the single passes give a full total of 0.252 and 0.266 warm and 0.464 and 0.475 cold (merged 0.254 and 0.462). Tests whose two passes differ by more than 20 percent in the full ratio: `hdlm_unsigned_divider`,
`hdlm_sine_generator`, `vunit_logging` (cold), `neorv32_single_cache` (cold), `vai_pe_array`. Sim1 drift against head-to-head 3, stated so the move can be read: over the same tests sim1's full-total wall is x0.97 warm and
x0.98 cold, its run phase x0.88 (tests over 0.3 s; executed instructions unchanged, x0.99, so this is host speed and load, not different work), while yavs' full total is x0.93 and its executed instructions fell from
805 G to 644 G (sim1 529 G to 525 G). So the ratio gain is yavs getting faster (the instruction ratio, load-independent, went 0.43 to 0.35) and part of it is hidden by sim1 running faster on these three cores. A sim1 control
(the three long tests `fletcher_bufferwriter_512`, `poc_arith_addw`, `vai_window_gen_interlock`, run phase, same cores, at the start and at the end of the batch, then within the passes) shows sim1 varying by up to
x1.5 between single runs at high load (`poc_arith_addw` 8.35 s at the start against 5.57 s at the end; passes A and B 5.86 and 5.86 s; head-to-head 3 5.88, 6.08 and 6.71 s), `fletcher_bufferwriter_512` stable at
1.97 to 2.03 s (2.02 to 2.30 before), `vai_window_gen_interlock` 4.16 to 6.13 s (6.63 to 8.84 before). Against head-to-head 2 sim1's full total is x1.09 warm and x1.07 cold at identical run-phase instructions and
unchanged run-phase wall, the rest is the analysis cells.

Worst 10 by warm total ratio, with `grlib_leon3mp` (seconds are medians; run = elaboration + run, yavs vs sim1; the ratio at head-to-head 3 in brackets, instruction ratio yavs / sim1 in the fifth column):

| test | warm | cold | run s | instr | issue |
|---|---:|---:|---|---:|---|
| grlib_leon3mp | 2.56 (3.71) | 2.65 (3.77) | 54.08 vs 13.47 | 3.7 (5.4) | #677 |
| fletcher_bufferwriter_512 | 1.45 (2.03) | 1.54 (2.12) | 4.78 vs 1.98 | 2.1 (3.5) | #519 |
| vai_window_gen_interlock | 1.19 (1.04) | 1.77 (1.36) | 10.96 vs 4.44 | 2.6 (2.4) | #520 |
| poc_arith_addw | 1.07 (1.45) | 1.53 (1.59) | 8.61 vs 5.86 | 1.3 (1.9) | #520 |
| fletcher_bufferreader | 0.79 (1.27) | 0.92 (1.36) | 1.82 vs 1.11 | 1.5 (2.4) | #519, #523 |
| neorv32_single_min | 0.71 (0.89) | 0.81 (0.93) | 3.28 vs 3.74 | 0.9 (1.2) | #521, #529 |
| sc5_fifo_repack | 0.65 (0.61) | 0.82 (0.83) | 2.33 vs 0.87 | 3.2 (3.6) | #519 |
| neorv32_single_cache | 0.59 (0.80) | 0.69 (1.00) | 3.04 vs 4.35 | 0.7 (1.0) | #521, #529 |
| uvvm_axistream_vvc | 0.54 (0.57) | 0.61 (0.69) | 3.27 vs 5.15 | 1.2 (1.3) | none specific |
| neorv32_dual_full | 0.44 (0.58) | 0.51 (0.60) | 2.45 vs 4.86 | 0.5 (0.8) | #521 |

Slower than sim1 are only run-bound tests (`grlib_leon3mp`, `fletcher_bufferwriter_512`, `vai_window_gen_interlock`, `poc_arith_addw`); `vai_window_gen_interlock` went from 1.04 to 1.19 warm (its yavs run 12.54 to 10.96 s but
sim1's 6.63 to 4.44 s: the sim1 side got faster, the instruction ratio 2.4 to 2.6 is the load-independent view). The analysis of every test is faster than sim1's. Every cell was measured with the same bench binary.

## Real-world performance suite: yavs against sim1, head-to-head 3, 2026-10-24 (`tools/realbench`)

This was the headline of the third run, superseded by head-to-head 4 above (`4b80445d`, bench build with the `llvm` feature; the previous run, `9a8d8f50`, is in brackets). Basis (root rule 23): `--tier=auto` only, TOTAL wall as a user runs it, analysis of
every file of the compile order (one tool call per file) plus elaboration and run, against sim1's analysis plus elaboration plus run; cold = empty compilation cache and fresh libraries, warm = cache present.
Interpreter times are never compared with sim1. Ratios are yavs / sim1, below 1 means yavs is faster. Method as in the previous run: 5 timed runs per cell, interleaved, two complete passes, the lower pass median per
cell for both tools; pinned to physical cores 12 to 15 of a Ryzen 9 7950X3D, host load average 25 to 33. Raw results are not committed.

Geometric mean over the same 41 tests (the full total first, own files + elaboration + run beside it):

| category | n | total warm | total cold | own+elab+run warm | own+elab+run cold |
|---|---:|---:|---:|---:|---:|
| **overall** | 41 | 0.26 (0.30) | 0.49 (0.60) | 0.32 (0.35) | 0.59 (0.70) |
| bus-tristate | 1 | 0.16 (0.18) | 0.33 (0.46) | 0.13 (0.11) | 0.45 (0.39) |
| cpu | 3 | 0.74 (1.00) | 0.82 (1.06) | 0.88 (1.19) | 0.97 (1.21) |
| fsm | 4 | 0.25 (0.30) | 0.43 (0.55) | 0.55 (0.65) | 1.10 (1.31) |
| memory | 4 | 0.19 (0.21) | 0.42 (0.53) | 0.22 (0.27) | 0.52 (0.64) |
| numeric | 8 | 0.24 (0.25) | 0.48 (0.56) | 0.27 (0.26) | 0.51 (0.57) |
| protected | 8 | 0.22 (0.26) | 0.45 (0.54) | 0.29 (0.32) | 0.48 (0.59) |
| records | 7 | 0.21 (0.22) | 0.41 (0.48) | 0.24 (0.23) | 0.44 (0.48) |
| rtl-datapath | 2 | 0.40 (0.53) | 0.67 (0.98) | 0.41 (0.56) | 0.71 (1.08) |
| strings-textio | 2 | 0.16 (0.17) | 0.34 (0.43) | 0.06 (0.07) | 0.23 (0.25) |
| wide-vectors | 2 | 1.12 (1.69) | 1.33 (1.93) | 3.05 (4.22) | 3.10 (4.32) |

| basis | warm | cold |
|---|---:|---:|
| total, libraries analysed too | 0.26 (0.30) | 0.49 (0.60) |
| own files + elab + run | 0.32 (0.35) | 0.59 (0.70) |
| elab + run only | 0.30 (0.34) | 0.61 (0.68) |

Summed over the 41 tests the full total is 90 s (yavs) against 228 s (sim1) warm and 130 s against 224 s cold (102 against 200 s and 152 against 202 s before); 37 tests are faster than sim1 warm and cold (35 and 34
before). The h2h designs: 0.65 warm, 0.80 cold (0.70, 0.83); the tiny-test corpus (368 tests): 0.12 warm, 0.26 cold (0.12, 0.27). The suite has 42 tests now: `grlib_leon3mp` (new, VHDL-93 mode) is left out of the
means above to keep the basis; with it the full total is 0.28 warm and 0.51 cold. Caveat: most of the move from 0.30 to 0.26 is sim1 running slower in this run (its total wall x1.12 on the same tests, yavs x0.98; the
host was busier and the timing cores differ), so within the repeatability of the method (about 10 percent) the ratio did not change. The load-independent gain is in the instruction ratio (yavs / sim1, geomean
0.43, 0.48 before; the yavs sum fell from 1117 G to 805 G at unchanged sim1 instructions).

Worst 10 by warm total ratio, with `grlib_leon3mp` (seconds are medians; run = elaboration + run, yavs vs sim1; the ratio at `9a8d8f50` in brackets, instruction ratio yavs / sim1 in the fifth column):

| test | warm | cold | run s | instr | dominant cost | issue |
|---|---:|---:|---|---:|---|---|
| grlib_leon3mp | 3.71 | 3.77 | 90.45 vs 15.25 | 5.4 | LEON3 template, VHDL-93 mode (new in the suite): record-signal leaf cost | #677 |
| fletcher_bufferwriter_512 | 2.03 (2.94) | 2.12 (3.09) | 6.84 (9.63) vs 2.02 (2.05) | 3.5 (4.24) | 512-bit buses, interpreter and kernel signal path | #519 |
| poc_arith_addw | 1.45 (2.07) | 1.59 (2.39) | 11.62 (16.96) vs 5.88 (6.14) | 1.9 (3.27) | exhaustive operand loops, interpreted | #520 |
| fletcher_bufferreader | 1.27 (1.43) | 1.36 (1.55) | 3.77 (3.28) vs 1.68 (1.15) | 2.4 (2.77) | records, wide vectors | #519, #523 |
| vai_window_gen_interlock | 1.04 (1.83) | 1.36 (2.23) | 12.54 (15.14) vs 6.63 (4.14) | 2.4 (3.46) | long pipelined array-heavy RTL | #520 |
| neorv32_single_min | 0.89 (1.24) | 0.93 (1.32) | 4.51 (5.42) vs 4.18 (3.58) | 1.2 (1.71) | concurrent signal-copy processes interpreted | #521, #529 |
| neorv32_single_cache | 0.80 (1.08) | 1.00 (1.10) | 4.39 (5.26) vs 4.66 (4.11) | 1.0 (1.46) | as neorv32_single_min | #521, #529 |
| sc5_fifo_repack | 0.61 (0.98) | 0.83 (1.21) | 2.67 (3.49) vs 0.95 (0.91) | 3.6 (4.62) | wide std_logic_vector slicing; its library analysis (yavs 0.88 s vs 4.81 s) hides most of it warm | #519 |
| neorv32_dual_full | 0.58 (0.75) | 0.60 (0.80) | 3.18 (4.28) vs 4.72 (4.89) | 0.8 (1.07) | faster than sim1 on the run phase (3.18 vs 4.72 s) | #521 |
| uvvm_axistream_vvc | 0.57 (0.82) | 0.69 (0.92) | 3.42 (3.75) vs 5.06 (3.73) | 1.3 (1.86) | run 3.42 vs 5.06 s (sim1 3.73 s at 9a8d8f50): yavs ahead on analysis and run | none specific |

Slower than sim1 are only run-bound tests; the analysis of every test is faster than sim1's. Every cell was measured with the same bench binary.

## Real-world performance suite: yavs against sim1 on the total-wall basis, 2026-10-23 (`tools/realbench`)

This was the headline of the second run (`9a8d8f50`; the first, at `65b7f573`, is quoted as "before"); the third run is above. Basis (root rule 23, user decision 2026-10-22): `--tier=auto` only,
TOTAL wall as a user runs it, analysis of every file of the compile order (one tool call per file, never batched) plus elaboration and run, against sim1's analysis plus
elaboration plus run. Cold = empty compilation cache and fresh libraries; warm = cache present, unchanged sources. Interpreter times are only compared with earlier interpreter
times, never with sim1. Ratios are yavs / sim1, below 1 means yavs is faster.

Method: origin/main at `9a8d8f50`, `tools/build-bench.sh --features llvm`, all 41 tests (`grlib_leon3mp` stays excluded), 5 timed runs per cell (medians, cells interleaved, order
swapped every round), two complete passes; per cell the lower of the two pass medians is used for yavs and sim1 alike, because other work on the host lands on the timing cores
(the geometric means of the single passes are within 10 percent: total warm 0.30 and 0.33, cold 0.58 and 0.63). Every timed run was pinned to one of the physical cores 8, 10 to 15
of a Ryzen 9 7950X3D (one CCD, one test per core at a time, the yavs and sim1 cells of a test on the same core; core 9 was occupied by a stray process), builds on cores 16 to 23,
host load average 9 to 22. Wall time comes in steps of 10 ms. The library part of the analysis is measured once per corpus selection and shared by its tests. Raw results are not committed.

Geometric mean of the total wall ratio (yavs `--tier=auto` / sim1). Left: the full total (analysis of the dependency libraries included). Right: the same tests with the
dependency-library analysis left out (own files + elaboration + run). In brackets the values at `65b7f573`.

| category | n | total warm | total cold | own+elab+run warm | own+elab+run cold |
|---|---:|---:|---:|---:|---:|
| **overall** | 41 | 0.30 (0.98) | 0.60 (1.14) | 0.35 (0.58) | 0.70 (0.92) |
| bus-tristate | 1 | 0.18 (1.13) | 0.46 (1.21) | 0.11 (0.14) | 0.39 (0.44) |
| cpu | 3 | 1.00 (1.69) | 1.06 (1.77) | 1.19 (1.91) | 1.21 (1.98) |
| fsm | 4 | 0.30 (0.81) | 0.55 (0.94) | 0.65 (0.67) | 1.31 (1.33) |
| memory | 4 | 0.21 (0.98) | 0.53 (1.18) | 0.27 (0.59) | 0.64 (0.99) |
| numeric | 8 | 0.25 (1.00) | 0.56 (1.18) | 0.26 (0.49) | 0.57 (0.76) |
| protected | 8 | 0.26 (0.64) | 0.54 (0.88) | 0.32 (0.59) | 0.59 (0.80) |
| records | 7 | 0.22 (0.94) | 0.48 (1.04) | 0.23 (0.39) | 0.48 (0.64) |
| rtl-datapath | 2 | 0.53 (1.76) | 0.98 (1.80) | 0.56 (1.38) | 1.08 (1.75) |
| strings-textio | 2 | 0.17 (0.58) | 0.43 (0.72) | 0.07 (0.06) | 0.25 (0.24) |
| wide-vectors | 2 | 1.69 (2.58) | 1.93 (2.59) | 4.22 (4.58) | 4.32 (4.56) |

| basis | warm | cold |
|---|---:|---:|
| total, libraries analysed too (first column above) | 0.30 (0.98) | 0.60 (1.14) |
| own files + elab + run | 0.35 (0.58) | 0.70 (0.92) |
| elab + run only (the earlier basis of this suite) | 0.34 (0.36) | 0.68 (0.71) |

Almost all of the change since `65b7f573` is analysis speed (#561, #581, #584, #577): summed over the 41 tests the analysis of the dependency libraries takes 23 s warm in yavs
(143 s before) against 137 s in sim1; the full total summed over the 41 tests is 102 s (yavs) against 200 s (sim1) warm and 152 s against 202 s cold. The run phase is
unchanged on most tests; it improved on the three neorv32 tests (warm run 9.8/9.5/9.9 s to 4.3/5.4/5.3 s) and `poc_arith_addw` (23.8 to 17.0 s). 35 of 41 tests are faster than sim1
warm (20 of 39 before), 34 cold. The tests that are slower than sim1 are all run-bound.

Worst 10 by warm total ratio (seconds are medians; run = elaboration + run, sim1 vs yavs; in brackets the ratio at `65b7f573`):

| test | warm | cold | dominant cost | issue |
|---|---:|---:|---|---|
| fletcher_bufferwriter_512 | 2.94 (3.46) | 3.09 (3.41) | run 2.05 vs 9.63 (512-bit buses, interpreter and kernel signal path) | #519 |
| poc_arith_addw | 2.07 (2.92) | 2.39 (2.72) | run 6.14 vs 16.96 (exhaustive operand loops, interpreter ops; auto does not help) | #520 |
| vai_window_gen_interlock | 1.83 (2.31) | 2.23 (2.53) | run 4.14 vs 15.14 (long pipelined array-heavy RTL) | #520 |
| fletcher_bufferreader | 1.43 (2.03) | 1.55 (2.11) | run 1.15 vs 3.28 (records, wide vectors) | #519, #523 |
| neorv32_single_min | 1.24 (2.02) | 1.32 (2.13) | run 3.58 vs 5.42 (processor testbench, signal-copy processes interpreted) | #521, #529 |
| neorv32_single_cache | 1.08 (1.70) | 1.10 (1.80) | run 4.11 vs 5.26 | #521, #529 |
| sc5_fifo_repack | 0.98 (1.92) | 1.21 (1.97) | run 0.91 vs 3.49 (wide vector slicing); library analysis 3.24 vs 0.57 hides most of it warm | #519 |
| uvvm_axistream_vvc | 0.82 (1.01) | 0.92 (1.13) | run 3.73 vs 3.75 (equal); yavs wins on analysis only | none |
| neorv32_dual_full | 0.75 (1.40) | 0.80 (1.45) | run 4.89 vs 4.28 (faster than sim1 on the run now) | #521 |
| poc_cache_cpu | 0.67 (0.96) | 0.87 (1.08) | run 0.85 vs 1.73 (cache controller FSM, interpreted) | none |

`poc_stat_histogram` run got slower: 0.78 to 0.95 s warm, 17.6 to 20.4 G instructions (sim1 unchanged); see #518. Found and fixed afterwards, see "A native frame no longer materialises for a wide fill" (#648).

Per test (total wall in seconds and ratio):

| test | category | sim1 warm s | auto warm s | warm | sim1 cold s | auto cold s | cold |
|---|---|---:|---:|---:|---:|---:|---:|
| hdlm_fifo_sync | rtl-datapath | 3.92 | 0.59 | 0.15 | 3.89 | 1.67 | 0.43 |
| hdlm_unsigned_divider | numeric | 4.23 | 0.65 | 0.15 | 4.57 | 1.76 | 0.39 |
| hdlm_sine_generator | numeric | 4.02 | 0.62 | 0.15 | 3.99 | 1.69 | 0.42 |
| hdlm_ring_buffer | memory | 3.94 | 0.60 | 0.15 | 3.87 | 1.67 | 0.43 |
| hdlm_axi_fifo | records | 4.13 | 0.65 | 0.16 | 4.07 | 1.80 | 0.44 |
| olo_fix_cordic_rot | numeric | 7.71 | 1.53 | 0.20 | 7.71 | 3.57 | 0.46 |
| olo_intf_i2c_master | bus-tristate | 7.35 | 1.33 | 0.18 | 7.30 | 3.37 | 0.46 |
| olo_axi_master_full | records | 7.96 | 1.36 | 0.17 | 8.00 | 3.70 | 0.46 |
| olo_base_cam | memory | 8.00 | 1.37 | 0.17 | 7.87 | 3.51 | 0.45 |
| vunit_axi_dma | records | 4.04 | 0.70 | 0.17 | 4.07 | 1.86 | 0.46 |
| vunit_logging | strings-textio | 3.93 | 0.68 | 0.17 | 3.92 | 1.82 | 0.46 |
| en_cl_fix_pkg | numeric | 3.03 | 0.42 | 0.14 | 3.06 | 1.35 | 0.44 |
| poc_arith_addw | numeric | 8.41 | 17.39 | 2.07 | 8.45 | 20.22 | 2.39 |
| poc_stat_histogram | memory | 2.64 | 1.33 | 0.50 | 2.62 | 2.11 | 0.81 |
| poc_cache_cpu | fsm | 3.16 | 2.13 | 0.67 | 3.10 | 2.69 | 0.87 |
| poc_strings | strings-textio | 2.39 | 0.40 | 0.17 | 2.42 | 0.98 | 0.40 |
| poc_uart_rx | fsm | 2.42 | 0.42 | 0.17 | 2.50 | 1.12 | 0.45 |
| fletcher_bufferwriter_512 | wide-vectors | 3.35 | 9.85 | 2.94 | 3.36 | 10.39 | 3.09 |
| neorv32_dual_full | cpu | 5.84 | 4.40 | 0.75 | 5.82 | 4.68 | 0.80 |
| neorv32_single_min | cpu | 4.47 | 5.54 | 1.24 | 4.49 | 5.94 | 1.32 |
| neorv32_single_cache | cpu | 4.97 | 5.38 | 1.08 | 5.05 | 5.58 | 1.10 |
| osvvm_axis_random | protected | 12.59 | 2.55 | 0.20 | 12.68 | 6.09 | 0.48 |
| uvvm_sb_slv | protected | 2.41 | 1.10 | 0.46 | 2.39 | 1.60 | 0.67 |
| uvvm_util_rand | protected | 2.38 | 1.05 | 0.44 | 2.37 | 1.60 | 0.68 |
| uvvm_axistream_vvc | protected | 4.80 | 3.92 | 0.82 | 4.82 | 4.42 | 0.92 |
| sc5_fifo_repack | wide-vectors | 4.16 | 4.06 | 0.98 | 4.08 | 4.92 | 1.21 |
| sc5_timestamp | fsm | 3.50 | 1.05 | 0.30 | 3.42 | 1.81 | 0.53 |
| uvvm_methods_log | protected | 1.24 | 0.14 | 0.11 | 1.21 | 0.60 | 0.50 |
| uvvm_func_cov | protected | 2.17 | 0.33 | 0.15 | 2.09 | 0.92 | 0.44 |
| osvvm_axi4_random | protected | 12.86 | 2.82 | 0.22 | 12.35 | 6.34 | 0.51 |
| hdlm_lfsr | numeric | 5.53 | 2.80 | 0.51 | 6.32 | 4.04 | 0.64 |
| hdlm_axi_crossbar | records | 4.93 | 0.70 | 0.14 | 5.71 | 1.87 | 0.33 |
| hdlm_register_file | records | 4.76 | 0.65 | 0.14 | 5.56 | 1.80 | 0.32 |
| hdlm_axis_bfm | protected | 4.67 | 0.68 | 0.15 | 5.47 | 1.77 | 0.32 |
| poc_fifo_ic_assembly | fsm | 2.48 | 0.59 | 0.24 | 2.56 | 1.17 | 0.46 |
| fletcher_bufferreader | records | 2.45 | 3.50 | 1.43 | 2.46 | 3.81 | 1.55 |
| fletcher_bufferwriter_prim32 | records | 1.91 | 0.37 | 0.19 | 1.92 | 0.70 | 0.36 |
| olo_fix_madd | numeric | 7.91 | 1.32 | 0.17 | 7.86 | 3.57 | 0.45 |
| vai_window_gen_interlock | rtl-datapath | 8.64 | 15.84 | 1.83 | 8.46 | 18.90 | 2.23 |
| vai_pe_array | numeric | 5.34 | 0.77 | 0.14 | 5.21 | 2.29 | 0.44 |
| vai_weight_buffer | memory | 5.02 | 0.71 | 0.14 | 4.66 | 2.31 | 0.50 |

Head-to-head designs of `tools/bench/designs` (group h2h), same basis (analysis + elaboration + run in one process chain, 5 runs, cores 8 and 10 to 15, times in ms unless
marked `s`, "range" = best yavs over worst sim1 .. worst over best). `xor_sig` is rejected by sim1 at analysis. Geometric mean over the ten designs both run: warm 0.69
(0.73 before), cold 0.83 (0.87 before).

| design | sim1 warm | auto warm | R warm | range | sim1 cold | auto cold | R cold |
|---|---:|---:|---:|---|---:|---:|---:|
| rtl_ff | 565.4 | 745.8 | 1.32 | 1.26-1.36 | 567.3 | 762.7 | 1.34 |
| lfsr64 | 130.6 | 189.8 | 1.45 | 1.37-2.26 | 131.3 | 215.0 | 1.64 |
| rtl_lfsr | 208.5 | 360.9 | 1.73 | 1.61-1.84 | 205.8 | 393.9 | 1.91 |
| compute_h2h | 3.63s | 169.7 | 0.05 | 0.04-0.08 | 3.59s | 455.6 | 0.13 |
| rtl_chain | 193.0 | 220.2 | 1.14 | 1.11-1.32 | 195.8 | 231.9 | 1.18 |
| xor_sig | - | 972.3 | - | - | - | 984.5 | - |
| rtl_fan | 1.07s | 906.6 | 0.85 | 0.60-1.18 | 1.09s | 882.7 | 0.81 |
| xor_types | 958.5 | 814.4 | 0.85 | 0.80-0.95 | 975.1 | 861.3 | 0.88 |
| rtl_bus | 788.1 | 221.7 | 0.28 | 0.27-0.30 | 786.9 | 231.2 | 0.29 |
| rtl_ram | 96.3 | 84.5 | 0.88 | 0.86-1.00 | 97.2 | 110.8 | 1.14 |
| lfsr64_xor | 111.4 | 92.4 | 0.83 | 0.78-0.92 | 108.0 | 115.4 | 1.07 |

Tiny-test corpus (`tests/corpus`, 368 of 491 tests run on both tools; 21 runs per cell, total wall of analysis + elaboration + run in one process chain, the cost of
`bash -c true` measured in every round and subtracted per test, #569): geometric mean auto / sim1 0.12 warm and 0.27 cold (without the subtraction 0.19 and 0.33; on the 346
tests of the earlier run 0.31 and 0.47 before); median per test 4.6 ms against 38.6 ms warm, 8.5 ms against 38.3 ms cold; yavs is faster on 368 of 368 tests warm and 364 of 368 cold.
The cold exceptions are the `ieee` group (textio, fixed-point and VITAL packages analysed from source into a fresh library; cold geomean 0.64, worst `ieee_textio` 1.48). The harness for
the designs and tiny tables is not in the repository: the stock `--ref` mode and `tiny.py` leave analysis out and have no cold cell (the cold cell and the interleaved `true` baseline are what #569 asks for).

Older figures below were measured on the earlier Intel host with the elaboration + run basis and an interpreter column that is compared with
sim1; they are kept as history, not as targets.

### VHDL-1993 mode, 2026-10-23 (`--std=93`; #629; design `docs/design/vhdl-93.md`)

`--std=93` (alias `1993`) selects the 1993 language and the 1993 packages (`lib/std/1993`, `lib/ieee/1993`): `std_logic_vector` is a type of its own, `numeric_std`
and `numeric_bit` with the 1076.3 types (derived from the Apache-2.0 2008 files, #650: the 1995 IEEE texts are not shipped), the Synopsys `std_logic_textio`, no `env`, none of the 2008 predefined names (`to_string`, `maximum`, `integer_vector`, ...), the 2008 reserved words are
identifiers, shared variables of any type are legal, `E0011` for the 2008 statements. Mixing follows the 2008/2019 rule (newest standard of the closure; a 93 file keeps its own
parsing and gates). `--std=87` and `02` are not added. Reading an `out` port or parameter is `E1360` (L138). Since 2026-10-28 (#814) the 1993 restrictions on object aliases of multidimensional arrays, unconstrained element subtypes, `buffer` port associations, packages in declarative parts and array pieces in aggregates are `E1007`; not enforced: the other 2008 relaxations (`docs/design/vhdl-93.md` section 5).

- **Corpora** (yavs, 2026-10-23, after the follow-up fixes): `grlib-1993` 454 of 454 (sim1 and sim2 also accept all 454; GRLIB's own `axi4mux.vhd` and `axi4mem.vhd` are excluded as errors of GRLIB); `vests-1993` (the compliant tests of the VESTs 1993 suite) 1936 of 1939 (sim1 accepts all 1939); `yavs-lib-1993` 28 of 28. The 3 vests failures are yavs bugs that also fail at 2008, each with an issue: #643 (access subtype with an index constraint, `tc537`; needs run-time checks of the designated object, not done), #657 (indexing a user attribute of a type, `tc400`) and #658 (a user attribute of a subtype as an aggregate choice, `tc2565`). Fixed since the first run (18 failures): #638 (user attribute values take the attribute's subtype), #639 (entity and architecture names as expanded-name prefixes), #640 (a type named `work`), #641 (a guarded assignment in a block without a guard expression), #642 (attributes through an alias), #644 (disconnection specification of a port), #645 (attribute specification of a statement label in a process), #646 (range attributes as a type definition or block configuration index). All other manifests (2008 and 2019) are unchanged against their expected lists, and the differential corpus (`tools/difftest`, 502 cases including the 6 `std93/` ones) passes.
- **The VESTs suite simulated** (`tools/suites/vests.py`, a CI step of the conformance report since 2026-10-28, #780): every list of the suite (billowitch compliant, `analyzer_failure`, `simulator_failure`; ashenden compliant and non-compliant; clifton-labs compliant), at `--std=93`, judged by the suite's own expectations: 3621 of 3694 counted tests pass (billowitch compliant 1607/1612, `analyzer_failure` 1429/1490 of which 24 are stopped by the right yavs error at elaboration or run time instead of at analysis, `simulator_failure` 37/38, ashenden compliant 448/451, ashenden non-compliant 7/7, clifton-labs 93/96); 44 tests are not counted (files that are not legal VHDL-1993, files every tool rejects under the suite's own build list, file formats left to the implementation), each with its reason in the driver. Since 2026-10-28 (#808 in part, #809, #811, #813 tc3058, #814, #815, #821, #822, #823, #824): 3651 of 3694 (billowitch compliant 1608/1612, `analyzer_failure` 1457/1490, `simulator_failure` 38/38), no test newly failing. Since #807, #808, #810, #812, #813 and #846 (2026-10-28): 3684 of 3694 (`analyzer_failure` 1490/1490), no test newly failing; the 10 left are valid designs that yavs does not run yet, #816 to #820. With #818 (`t'range'left`), 3687 of 3694. Failures by cause: #807 to #824 (accepted invalid code: lexical, subprogram, deferred constant, unit order, non-shared variables, aggregates, type rules, a disconnection in a process, the 1993 restrictions 2008 relaxed (#814, a recorded decision); wrong results: #815 aggregate variable target, #821 unchecked `others` value, `tc3058` alias length in #813; internal errors on invalid code: #822, #823, the stack overflow of #809; not implemented: #816, #817, #818; rejected valid code: #819, #820). The issues give sim1's and sim2's results on the same tests.
- **GRLIB**: the unmodified `designs/leon3mp` testbench runs on yavs at `--std=93 --relaxed` to `Test passed, halting with IU error mode` at the same point as sim1; the three
  elaboration banners of `irqmp`, `apbuart1` and `ahbuart7` come out in another order (time-zero reports of different processes, `ignore=order` in the differential harness).
  The same banners come out in the same order on sim2 (`--std=93c --ieee=synopsys -fexplicit`, stdout byte-identical to yavs, 53 lines), so sim1 is the odd one out; apart from
  that order the output of sim1 and yavs is identical (the system test prints the same lines and halts at the same point).
- **Timing** (`tools/realbench/run.py`, test `grlib_leon3mp`, `tools/build-bench.sh --features llvm`, `--tier=auto`, medians of 2, one process on core 14, host load average about 12;
  NOT a baseline: the core was picked by the agent, the host was busy). Run = sim1 elaboration plus run against yavs run including its code generation. Wall seconds:

| basis | sim1 | yavs auto | ratio | instruction ratio |
|---|---:|---:|---:|---:|
| run warm | 13.91 | 143.24 | 10.29 | 10.73 |
| run cold (empty compilation cache) | 13.91 | 132.72 | 9.54 | 11.03 |
| analysis of the 453 files (before #604 was fixed), one call each, cold / warm | 7.22 / 6.60 | 2.73 / 1.12 | 0.38 / 0.17 | 0.47 / 0.13 |
| elaboration alone, cold / warm | 0.34 / 0.33 | 0.18 / 0.21 | 0.55 / 0.64 | 0.68 / 0.80 |
| total (analysis + run), cold / warm | 21.13 / 20.51 | 135.45 / 144.36 | 6.4 / 7.0 | |

  The `codegen` cells fail (`--tier=llvm` to time 0 exceeds the 300 s limit: #630). The run is the cost, as in the adjusted 2008 copy above: the integer unit's process stays
  interpreted under `auto` (#630).

### GRLIB (LEON3) in the corpora and the real-world suite, 2026-10-23 (`grlib-2008`, `grlib_leon3mp`; #604, #629, #630, #628)

GRLIB 2026.3 build 4301 (Gaisler IP library, GPL-2.0, official tarball sha256 `0978edc1...2f2`) is fetched by `tools/fetch-corpus.sh` at the tagged
commit of a git mirror that is file-identical to the tarball (sparse: `lib/`, `bin/`, the `leon3mp`, `leon3-minimal` and `noelv-generic` templates, 29 MB). The CI
`corpus` job fetches it with the other corpora (same cache key) and runs `grlib-2008` against its expected list.

- **Analysis corpus `grlib-2008`**: the order of GRLIB's own script generator for `designs/leon3mp` with inferred technology (no vendor library), 410 files.
  sim1 and sim2 accept all 410, yavs too (`lib/opencores/can/can_top.vhd` failed with E0006 at 32 nested parentheses until #604: the parser now counts one level of parentheses once, so 64 levels are fine). 46 of GRLIB's 456 files are
  excluded: GRLIB is VHDL-93 code and the `std_logic_vector`/`std_ulogic_vector` overload pairs of its `stdio` and `testlib` packages are homographs under 2008
  (LRM 12.3), rejected by every tool; the processor core (`iu3` through `sparc_disas`), the bus controller and the simulation models depend on them.
- **VHDL-93**: with GRLIB's own flags the unmodified tree works on both references: sim1 (`--std=93 --relaxed`) analyses 454 of 456 library files (two are
  errors in GRLIB: `axi4mux.vhd`, `axi4mem.vhd`), elaborates and runs `leon3mp` to `Test passed` at 1060873 ns in 13 s; sim2 (`--std=93c --ieee=synopsys`) in 35 s CPU.
  yavs had no 1993 mode then, so `grlib_leon3mp` was `excluded`; since the VHDL-1993 mode (next section, #629) it runs unmodified at `--std=93` (stop time 1060500 ns, just before
  the template's own end-of-test failure assertion).
- **Candidates**: `leon3-minimal` (with `-gfabtech=0 -gmemtech=0 -gpadtech=0 -gclktech=0`) runs to `Test passed` on sim1 and starts the same system test on yavs (not run to the end), but it ends in the
  same delta cycle as its failure assertion, so the harness would need an expected-failure end; `noelv-generic` needs 140 s on sim1 for 145 us. Not added.
- **Bugs found**: #604 (parser nesting cap), #628 (a constant and a subprogram `flush` from `std.textio` are not hidden: LRM 12.4; sim2 rejects `sparc_disas.vhd`
  at 2008 for this, sim1 and yavs accept), #630 (the integer unit's `comb` process stays interpreted under `--tier=auto`; `--tier=llvm` needs 82 s CPU of code generation).

Exploratory timing, NOT part of the headline above or its category means. It runs on an adjusted copy, because the test cannot run on the unmodified sources at 2008: in a
scratch checkout (HEAD at the pinned commit, working tree edited, selected with `YAVS_CORPUS`) all `HRead`/`HWrite`/`Write` procedures of `stdio.vhd` and the
`std_logic_vector` overloads of `compare` and `check` in `testlib.vhd` were removed (2008 `ieee.std_logic_1164` has the former), and an untracked manifest of all 456 files of
GRLIB's list replaced `grlib-2008`. The adjusted design gives identical output and end time on sim1 and on yavs at every tier. `--tier=auto` against sim1, wall seconds (medians of
2, `tools/build-bench.sh --features llvm`, cores 14 and 15 idle but host load average about 28), analysis of all 450 files one call per file:

| basis | sim1 | yavs auto | ratio | instruction ratio |
|---|---:|---:|---:|---:|
| total warm (analysis + elaboration + run) | 21.54 | 153.65 | 7.13 | 8.69 |
| total cold (empty cache, fresh libraries) | 21.97 | 150.82 | 6.86 | 8.88 |
| analysis warm / cold | 8.61 / 9.04 | 3.41 / 4.31 | 0.40 / 0.48 | 0.58 / 0.69 |
| elaboration alone warm / cold | 0.38 / 0.38 | 0.24 / 0.21 | 0.63 / 0.55 | 0.79 / 0.67 |
| elaboration + run warm / cold | 12.93 / 12.93 | 150.24 / 146.51 | 11.62 / 11.33 | 11.25 / 11.46 |

Analysis and elaboration are faster than sim1; the run is the cost (#630). Interpreter (own before/after figure, not compared with sim1): 152.3 s, 1850 G instructions
for the same run; `--tier=auto` is 150.2 s warm, so it gains nothing here.

## Real-world performance suite, baseline 2026-10-21, earlier host (`tools/realbench`)

The designs of `tools/bench` were written for the harness and the tuning so far was done on them and on a few library
testbenches; this suite times testbenches of real projects, chosen for diversity (`tools/realbench/AGENTS.md`, list and
reasons in `tools/realbench/tests.toml`). 30 tests pass on yavs and sim1: hdl-modules (5), Open Logic (4), the test runner's
examples (2), en_cl_fix (1), PoC (5), fletcher (1), satcat5 (2), the two verification libraries (7: OSVVM and UVVM testbenches)
and the maintainer's own project (3). Categories: clocked RTL datapaths, FSMs, resolved bus (I2C), records, wide vectors,
`numeric_std` and fixed point, memories, protected types and scoreboards, string and textio code. Run lengths on sim1 are 0.06 s to 6 s;
23 of the 30 take under 1 s (start-up, elaboration and a few thousand clock cycles). The baseline has no CPU-core test: the neorv32
testbench diverged from sim1 then (#513). Fixed since 2026-10-22, its three configurations are back in the suite (not in the baseline
numbers below); a probe run puts them at 7x to 9x of sim1 with the interpreter and slower still with `--tier=auto` (#521).
Eight more tests (hdl-modules LFSR, crossbar, register file and stream BFM; PoC `fifo_ic_assembly`; fletcher buffer reader and
prim32 writer; Open Logic `olo_fix_madd`) were excluded for elaboration bugs and run again since 2026-10-22 (#512, #515, #516, #517,
#522; also not in the numbers below): the package bodies of units loaded while elaborating, the initialisation order of packages, a
function that reads a protected shared variable of its region called during elaboration, and `s(d'range) <= d` with a parameter. A
probe puts `hdlm_lfsr` at 10x and `fletcher_bufferreader` at 6.7x of sim1 (#523).

Method (as `tools/bench/AGENTS.md` section 7): release build with the `llvm` feature of main at `55772743` (only STATUS and
documentation commits since), `--threads 1`, every timed run pinned to performance core 6 or 7 (two tests at a time, all cells of
one test on the same core, builds elsewhere), host load about 2 to 3, median of 5, 3 or 2 runs by test length with the cells
interleaved and the order swapped every round, analysis excluded (sim1: elaboration plus run). Cells: `interp` and `auto` are warm (an
untimed run first fills the compilation cache of the test's own copy of the libraries); `auto-cold` copies the pristine libraries
before every timed run (empty compilation cache). Ratios are yavs / sim1, below 1 is faster. Wall and CPU time come in steps of 10 ms:
for the tests under 0.1 s read the instruction ratios. Raw results (not committed): `~/.cache/yavs-realbench/results/20261021-234609-baseline.json`.

Geometric mean of the wall time ratio per category:

| category | n | interp | auto | auto-cold |
|---|---:|---:|---:|---:|
| **overall** | 30 | 0.42 | 0.44 | 1.15 |
| bus-tristate | 1 | 0.15 | 0.19 | 0.78 |
| fsm | 3 | 1.92 | 1.95 | 2.27 |
| memory | 4 | 0.35 | 0.35 | 1.23 |
| numeric | 6 | 0.26 | 0.27 | 0.87 |
| protected | 7 | 0.36 | 0.39 | 0.82 |
| records | 3 | 0.18 | 0.18 | 0.78 |
| rtl-datapath | 2 | 0.61 | 0.74 | 1.72 |
| strings-textio | 2 | 0.08 | 0.08 | 0.47 |
| wide-vectors | 2 | 9.12 | 10.12 | 10.15 |

Geometric mean of the instruction ratio (`perf stat`, `cpu_core/instructions/`):

| category | n | interp | auto | auto-cold |
|---|---:|---:|---:|---:|
| **overall** | 30 | 0.63 | 0.64 | 1.59 |
| bus-tristate | 1 | 0.20 | 0.21 | 0.93 |
| fsm | 3 | 3.07 | 3.06 | 3.40 |
| memory | 4 | 0.50 | 0.47 | 1.54 |
| numeric | 6 | 0.39 | 0.40 | 1.14 |
| protected | 7 | 0.67 | 0.72 | 1.38 |
| records | 3 | 0.20 | 0.20 | 0.97 |
| rtl-datapath | 2 | 1.04 | 1.06 | 2.59 |
| strings-textio | 2 | 0.13 | 0.13 | 0.68 |
| wide-vectors | 2 | 9.34 | 9.08 | 9.82 |

Run-length view (the geometric mean weighs a 0.06 s test like a 6 s one):

| cell | sum ratio | n | geomean, basis >= 1 s | n |
|---|---:|---:|---:|---:|
| interp | 3.02 | 30 | 1.71 | 7 |
| auto | 3.10 | 30 | 1.81 | 7 |
| auto-cold | 3.30 | 30 | 2.08 | 7 |

Reading: a warm `yavs run` of a small testbench costs 0.02 s against 0.2 s for sim1's elaboration and run, and that decides the short
tests and the overall geometric mean of 0.42. The 7 tests that take a second or more on sim1 are all slower on yavs (summed time over
all 30: 3.0x; geometric mean over those 7: 1.7x), and the worst categories are wide vectors (9.1x) and FSMs (1.9x). `auto` is at best equal to
`interp` here: within 20% on every test of 0.5 s or more except `poc_stat_histogram` (2.2 s against 3.8 s). The cold cache costs 0.1 to 0.2 s on the short tests
(loading and compiling what the test uses; 3x to 7x their warm time) and up to 20% on the long ones, except `uvvm_axistream_vvc`, which pays 2.4 s
(53%).

Per test (wall seconds, ratio to sim1 in brackets; CPU seconds and instruction counts are in the JSON):

| test | category | sim1 | interp | auto | auto-cold |
|---|---|---:|---:|---:|---:|
| hdlm_fifo_sync | rtl-datapath | 0.23 | 0.02 (0.09) | 0.03 (0.13) | 0.15 (0.65) |
| hdlm_unsigned_divider | numeric | 0.22 | 0.02 (0.09) | 0.02 (0.09) | 0.13 (0.59) |
| hdlm_sine_generator | numeric | 0.29 | 0.04 (0.14) | 0.04 (0.14) | 0.14 (0.48) |
| hdlm_ring_buffer | memory | 0.23 | 0.02 (0.09) | 0.02 (0.09) | 0.13 (0.57) |
| hdlm_axi_fifo | records | 0.31 | 0.05 (0.16) | 0.05 (0.16) | 0.23 (0.74) |
| olo_fix_cordic_rot | numeric | 0.65 | 0.21 (0.32) | 0.25 (0.38) | 0.44 (0.68) |
| olo_intf_i2c_master | bus-tristate | 0.27 | 0.04 (0.15) | 0.05 (0.19) | 0.21 (0.78) |
| olo_axi_master_full | records | 0.58 | 0.14 (0.24) | 0.15 (0.26) | 0.44 (0.76) |
| olo_base_cam | memory | 0.51 | 0.07 (0.14) | 0.11 (0.22) | 0.32 (0.63) |
| vunit_axi_dma | records | 0.28 | 0.04 (0.14) | 0.04 (0.14) | 0.24 (0.86) |
| vunit_logging | strings-textio | 0.24 | 0.02 (0.08) | 0.02 (0.08) | 0.08 (0.33) |
| en_cl_fix_pkg | numeric | 0.24 | 0.03 (0.12) | 0.03 (0.12) | 0.18 (0.75) |
| poc_arith_addw | numeric | 4.51 | 30.11 (6.68) | 31.01 (6.88) | 29.52 (6.55) |
| poc_stat_histogram | memory | 0.24 | 3.79 (15.79) | 2.21 (9.21) | 2.64 (11.00) |
| poc_cache_cpu | fsm | 0.70 | 2.02 (2.89) | 2.08 (2.97) | 2.30 (3.29) |
| poc_strings | strings-textio | 0.06 | 0.00 (0.00) | 0.00 (0.00) | 0.04 (0.67) |
| poc_uart_rx | fsm | 0.12 | 0.10 (0.83) | 0.10 (0.83) | 0.14 (1.17) |
| fletcher_bufferwriter_512 | wide-vectors | 1.82 | 22.29 (12.25) | 24.00 (13.19) | 23.84 (13.10) |
| osvvm_axis_random | protected | 0.79 | 0.27 (0.34) | 0.28 (0.35) | 0.47 (0.59) |
| uvvm_sb_slv | protected | 1.26 | 0.82 (0.65) | 0.99 (0.79) | 1.04 (0.83) |
| uvvm_util_rand | protected | 2.20 | 0.89 (0.40) | 0.96 (0.44) | 1.14 (0.52) |
| uvvm_axistream_vvc | protected | 6.14 | 4.50 (0.73) | 4.50 (0.73) | 6.89 (1.12) |
| sc5_fifo_repack | wide-vectors | 0.79 | 5.36 (6.78) | 6.14 (7.77) | 6.21 (7.86) |
| sc5_timestamp | fsm | 0.23 | 0.68 (2.96) | 0.69 (3.00) | 0.70 (3.04) |
| uvvm_methods_log | protected | 0.06 | 0.00 (0.00) | 0.00 (0.00) | 0.11 (1.83) |
| uvvm_func_cov | protected | 0.70 | 0.16 (0.23) | 0.18 (0.26) | 0.36 (0.51) |
| osvvm_axi4_random | protected | 1.08 | 0.70 (0.65) | 0.72 (0.67) | 0.97 (0.90) |
| vai_window_gen_interlock | rtl-datapath | 3.65 | 15.48 (4.24) | 15.39 (4.22) | 16.47 (4.51) |
| vai_pe_array | numeric | 0.43 | 0.04 (0.09) | 0.04 (0.09) | 0.20 (0.47) |
| vai_weight_buffer | memory | 0.24 | 0.02 (0.08) | 0.02 (0.08) | 0.14 (0.58) |

The five worst against sim1 (interpreter tier; `perf record --call-graph dwarf` of one warm run pinned to an efficiency core, symbols
of the release build with line tables):

| test | wall x | instr x | where the time goes |
|---|---:|---:|---|
| `poc_stat_histogram` (#518) | 15.8 | 18.7 | interpreter ops 57%, then composite layout work at run time: `layout_words` 19%, `layout_word` 5%, `layout_of` 3%, `lcopy`, `lget` and `memcmp` 4%: whole arrays of words are copied and compared by walking their layout |
| `fletcher_bufferwriter_512` (#519) | 12.3 | 10.2 | 512 bit buses and records: interpreter ops 26% and the kernel signal path about 40% (`Signal::update_effective` 13%, `Waiters::take_woken` 9%, `SimControl::cycle` 5%, `mark_active` 5%, `Waveform::schedule` 3%) |
| `sc5_fifo_repack` (#519) | 6.8 | 8.6 | wide `std_logic_vector` slicing and compares: interpreter ops 44%, then vector assignment and scheduling (`assign_planes_fast`, `Waveform::schedule`, `update_effective`, `eq_range`) about 12% |
| `poc_arith_addw` (#520) | 6.7 | 7.7 | exhaustive operand loops in a clocked testbench: interpreter ops 50%, the rest spread thin (kernel `cycle` and `run_batch` 6%, `read_into`/`assign_planes_fast` 4%, `cat` 2%, `layout_word` 2%) |
| `vai_window_gen_interlock` (#520) | 4.2 | 8.7 | long pipelined run of array-heavy RTL: interpreter ops 78%, `memmove` 4%, `Pre::fill` 3% |

Followed by `sc5_timestamp` (3.0x) and `poc_cache_cpu` (2.9x, interpreter ops 48%). Left out of the means (kept in `tests.toml` as
`excluded`; run one with `--tests NAME`):

| test | reason |
|---|---|
| `hdlm_lfsr`, `hdlm_axi_crossbar`, `hdlm_register_file`, `hdlm_axis_bfm` | hdl-modules testbenches: E2106 at elaboration (#512) |
| `poc_fifo_ic_assembly` | E2106 at elaboration (#515) |
| `fletcher_bufferreader`, `fletcher_bufferwriter_prim32` | E2001 "call whose body is not analyzed" (#516) |
| `olo_fix_madd` | E2001 shared variable of the transcript package (#517) |
| `uvvm_rand_perf` | sim1 runs out of memory at a 1 GiB heap; yavs runs it (2.8 s) |
| hVHDL floating point (not listed) | yavs rejects `float_typedefs_generic_pkg.vhd` (#514) |

Tried and dropped because they fail or run for ever on sim1 as well: the satcat5 switch and MAC lookup testbenches (data mismatches),
the JSON-for-VHDL and vhdl-extras testbenches that need files, the fletcher array reader (no end), and the Open Logic testbenches
whose generics have no default. For a decision run `python3 -I tools/realbench/run.py BIN --ref sim1 --cores 6,7`
and `--compare A B` (section 5 of its guide); on a loaded host wall time is noise, quote the instruction ratios.

## Layout work in loops: `poc_stat_histogram` (#518, 2026-10-22)

The profile of the baseline named the layout walk (`layout_words`), but the arrays were not copied or compared whole: the
testbench's `to_slm` and `to_slvv_8` are element loops over a matrix and an array of vectors (about 43 million element
iterations), and every iteration asked for the layout of two or three types only for the element count word. Fixed in the
lowering and the layout memo, no IR change: the count of an array (fourth layout word) is the product of the lengths in its
descriptor (`DopeLen`s, times the element count of a logic array of arrays), never a `LayoutOf`; `a(i)(j)` of a logic row
reads the element in place (`LGet` at the row's plane offset) instead of copying row `a(i)` into a temporary; the
interpreter's layout memo answers a repeated query in place. Ideas taken from the reference simulator's lowering: array
length and element stride are products of per-dimension lengths computed inline (folded when the bounds are constant), and
an element of an array of arrays is pointer arithmetic into the row, with no layout object at run time. Ours differs: logic
arrays are bit planes, so the row is addressed by plane offset, not pointer.

Pinned core 1, median of 5, ratios yavs / sim1: `interp` 15.2 to 9.4 (instructions 89.3 G to 64.2 G), `auto` 9.6 to 3.9 (71.2 G
to 31.9 G). Whole suite `--compare` before / after (41 tests, interp and auto): geometric mean of wall 0.99 and 0.95, of
instructions 0.99 and 0.97; memory category 0.89 and 0.79; no test slower beyond noise (host load 10 to 20). `tools/bench`
groups `internal` and `h2h`: geometric mean of total CPU 0.998 and 0.994. What is left in the test is interpretation
(no loop-invariant code motion: #526) and, in `auto`, the host address call of the compiled code (#527). #518 stays open for
that.

## Wide vectors: kernel signal path and reductions (#519, 2026-10-22)

`fletcher_bufferwriter_512` (512-bit stream buses through stream slices, FIFOs and reshapers) updated 4.1 G signal elements in
11.5 M updates over 0.8 M delta cycles, and 54% of the updates were `out` port formals passing a bus one level up. What a wide
signal cost per update and per hop, and what changed (commits deea791d, db08ae4f, bf400376, e4c8d983):

- Activity: every port hop marked every element active, two 8-byte stamps each (66 GB of stores per run). A signal of more than
  64 elements keeps a stamp per block of 64 (`Signal::blk_active`); readers take the later of element and block.
- Events: the plane-word update wrote two stamps per changed element, touching most cache lines of the stamp arrays. Such a
  signal keeps per block the records (cycle, time, elements) of the events whose elements have not changed since
  (`Signal::blk_event`, disjoint masks, at most 64 per block).
- Wake scan: an event with many changed runs mapped each run to its per-element waiter list though every process waited for the
  whole signal; those lists are scanned only once a waiter was added to one (`Waiters::chunked`).
- Preemption: an assignment to a vector driver with something pending ran LRM 10.5.2.2 element by element and split the old
  transactions bit by bit; it is now 64 elements at a time with a chain bit per target element.
- Reductions: unary `and`/`or`/`xor` (and the negated forms) of `std_ulogic_vector`, `unsigned`, `signed` and the
  `std_logic_misc` `*_reduce` functions are one native on the plane words (a 512-bit `or_reduce` cost about 50 us in the body).

Idea taken from the reference simulator: it keeps event and activity stamps per run of elements updated together (its unit of
update), not per element, and refines on demand. Ours keeps the element granularity the LRM asks for through the per-block masks,
and keeps the per-element stamps for the other paths.

Pinned core 3, `cpu_core` user counters, interpreter, against main without these commits: `fletcher_bufferwriter_512` 256.4 G
to 193.2 G instructions, 97.8 G to 75.9 G cycles; `sc5_fifo_repack` 77.4 G to 76.4 G. Against sim1 (run phase, core 3, host
load about 12): `fletcher_bufferwriter_512` 8.6x wall, 7.2x instructions (baseline 12.3x and 10.2x); `sc5_fifo_repack` 5.0x
wall, 6.0x instructions (6.8x and 8.6x, mostly from the interpreter work of #520). Whole suite `--compare` before / after (41
tests, interp and auto): geometric mean of run wall 0.99 and 0.98, of instructions 0.99 and 0.99, wide-vectors 0.86
instructions; no test slower in instructions beyond 1% (`olo_axi_master_full` under `auto` read 1.12 once and 1.00 on a rerun of 5). `tools/bench` group `h2h`: geometric mean of total CPU 0.98.

What is left: `sc5_fifo_repack` is interpreter dispatch on byte-lane loops (the sort network process `p_col` is 39% of the
executed operations, `uniform` and `rand_next` 16%), not wide-vector work; in `fletcher_bufferwriter_512` the interpreter is
about 45% (records with array elements: `layout_words`, #518) and half of the kernel work is the `out` port hops, which
collapsing `out` ports would remove (#528, a decision for the maintainer: `--no-collapse-ports` doubles the instructions of this
test, so `in` port collapsing already halved them).

## Wide buses, round 2: byte codes and record signal reads (#519, #523, 2026-10-22)

A fresh profile of the 512 bit stream test (`fletcher_bufferwriter_512`, interpreter, core 11 of the 7950X3D; `perf` with frame
pointers, per-process timers and a per-op trace, none committed) found no single dominant cost left: `run_ops` 30% (dispatch
14% of that, the rest spread over 40 ops), the kernel 40% (`update_effective` 12%, then 40 functions under 3%). Two changes
(`docs/decisions.md` unchanged; `crates/yavs-kernel/AGENTS.md` and `crates/yavs-lower/AGENTS.md` decisions):

- A logic signal of more than 64 elements whose sources are wider than 16 elements keeps no byte code per element
  (`CODES_SOURCE_MAX`): the refresh of 64 codes per changed block was 12% of `update_effective`. Vectors of registers keep them
  (a blanket limit of 64 made `rtl_ff` and `rtl_chain` 1.5 times slower).
- A vector field of a record signal is read into the field of the record value, not into a temporary and copied (the buffer,
  its size arithmetic and an `LCopy` per field).

Interpreter, core 11, `tools/realbench --compare` before/after (41 tests, run phase): geometric mean wall 0.99, instructions
1.00, no slowdown in instructions; `fletcher_bufferwriter_512` 11.46 s to 10.87 s (instructions 0.94), `fletcher_bufferreader`
4.74 s to 4.60 s (0.94), `sc5_fifo_repack` and `hdlm_lfsr` unchanged (0.99 to 1.00). `tools/bench` group `h2h`: total CPU 0.96,
`xor_sig` and `xor_types` 0.78. A micro design that drives twenty 512 bit signals per clock: 31.1 G to 21.6 G instructions.

What was measured and not changed (ranked by what the profile says is left):

1. Compiled code of these processes is slower than the interpreter: `--tier=llvm` on `fletcher_bufferwriter_512` simulates in
   20.3 s against 11.5 s (38440 IR instructions compiled, 3436 run through the interpreter, `LayoutOf` 592 and `Drive` 500
   the largest); `--tier=auto` compiles none of the processes (`--verbose` shows only refusals), so auto equals the interpreter here;
   with `--tier-heat=1,0,0` everything compiles and the run takes 24 s. #532, #339.
2. Zero-delay drive and commit of small vectors: 20 drives of a 32 bit vector cost 2.9 K instructions each, 37% of the time in
   `finish_direct` (kernel scheduler, #440).
3. Composite statements: 14 interpreter operations per statement of `ctrl_comb_proc` (`DopeLen`, `SigVal`, `LayoutWord`,
   `Aff1`/`Div`/`Aff1` size arithmetic, `SubtypeCheck`) of which a third depend on instance constants only; hoisting them out of
   the process loop would save at most 5% of the time because each is a cheap operation (#518, #520).
4. The layout memo keeps one entry per type: 100 K real `Ty::layout` calls per 100 us of simulated time on this test, 2% (#518).
5. A wide `update_effective` word costs about 470 instructions on fully changing buses (popcount in software on the baseline
   target, event records, plane blends); lazy run lists would save 2.5% at most.

## Loop-invariant code motion (#526, 2026-10-22)

`crates/yavs-ir/src/opt/licm.rs` (`docs/decisions.md` "Loop-invariant code motion"): in a function or procedure that cannot
suspend, computations a loop does not change move into a preheader, inner loops first; nested loops that share a header get
one each. Instructions that can fail move only from the effect-free start of the header, whose first pass the preheader
replays, so runtime errors and their order are unchanged (tests `composite/loop_invariant*`). Descriptor reads move only
from loops that write no memory a descriptor can be in; loads stay (no alias analysis yet). The inner loop of `to_slm` in
`poc_stat_histogram` went from 27 IR instructions per element to 12.

Pinned core 1, `tools/realbench --compare` before / after (memory category, interp and auto, median of 5):
`poc_stat_histogram` wall 0.69 and 0.85 (instructions 0.66 and 0.81); the other memory tests and the wide-vector category
unchanged. Against sim1: `interp` 6.0 (instructions 8.5), `auto` 3.1 (5.0), from 9.4 and 3.9. `tools/bench`: `h2h` designs
instructions 1.000 (interp) and 0.996 (auto), `internal` geometric mean of CPU 0.988 (no loops of this shape in either).
Compile cost: cold `--no-cache` runs of `osvvm_axis_random` and `uvvm_util_rand` +0.03% and -0.2% instructions; the
pass is linear in the code times the loop depth.
Checked: differential corpus 441/441 (interpreter at 4 threads and shuffled, `--tier=auto`, `--tier=llvm`, `gc-stress` on
the interpreter and `--tier=llvm`), pass lists 857 and 393 (`--oracle --check`), `cargo test --workspace`.

## neorv32 processor testbench against sim1: where the time goes (#521, 2026-10-22)

The three CPU tests of the real-world suite (`neorv32_dual_full` 200 us, `neorv32_single_min` and `neorv32_single_cache`
300 us, the core runs its built-in program; traces match sim1). Release build with `--features llvm`, `tools/realbench`,
core 7, median of 3, load average 13 (other agents), so read the ratios.

| test | sim1 | interp | `--tier=auto` | `--tier=llvm` |
|---|---:|---:|---:|---:|
| baseline at `448e11beea` (main at d1a06176) | 4.55 / 4.08 / 5.02 s | 38.3 / 37.2 / 44.3 s (8.4x / 9.1x / 8.8x) | 39.0 / 43.6 / 51.6 s (8.6x / 10.7x / 10.3x) | 98.7 s (24x) on single_min; dual and cache pass 300 s |
| after the lowering fix (`bd2f2e43`), same order, sim1 not rerun | | 18.2 / 18.6 / 17.4 s (0.47 / 0.47 / 0.40 of before; 3x to 5x of sim1) | 20.2 / 20.2 / 20.1 s | |

Instructions per run: sim1 39 to 44 G; interpreter before 347 to 398 G, after 125 to 165 G.
With `52159b17` and `deea791d` on top (second run, same method, load 13, so absolute times are not comparable with the table): interpreter
14.0 s (dual_full) and 18.9 s (single_min), `--tier=auto` 14.5 s and 19.4 s, 3% above the interpreter instead of 8% to 15%.

**Profile** (`neorv32_single_min`, interpreter, before the fix; per-process time and activation counts from a temporary timer around
every `Process::run`, not committed): analysis 0.06 s, elaboration 0.08 s, init 0.3 s, simulation 41 s. 2413 processes, 3806
signals with 125.6k elements (1903 scalars, 785 of 2 to 8 elements, 867 of 9 to 32, 192 of 33 to 64, 59 wider), 18.1 M
activations in 30000 clock cycles = 605 per cycle. 1745 processes ran fewer than 1000 times; 388 run once per clock (the registers,
42% of process time), 86 run more than once per clock (combinational logic that settles over deltas, 38%); 1879 are anonymous
concurrent statements (copies such as `a_req_o <= port_req(0)`, 5.5 M activations, 29% of process time, 1.5 to 5 us each).
Hot processes: `cpu_counters.counter_increment` 12% (65 us per activation), the 87 processes of `cpu_control` 11%
(`exec_comb` 4.4%, 150 record-field `Drive`s), `bus_gateway` 8%, `cpu_trace` 5%, the bit-manipulation ALU 4%, `pmp`, debug module,
UART/TWI/SPI engines about 1.5% each, textio trace of the dual run 2.4%. `perf` by symbol: op dispatch (`run_ops`, `run_quick`)
42%, signal reads and assigns (`Ctx::read_into`, `assign_*`) 15%, kernel scheduler (cycle, commit, wake scan, update) 15%,
`memmove` 11%, other interpreter machine functions 10%, process entry and exit 4%.

**The main cost was a lowering defect** (fixed, `bd2f2e43`): a name below a record signal (`ctrl_i.cnt_event`, `port_rsp(i).data`)
was lowered as a read of the whole record followed by a field select. `ctrl_i` has 41 leaves; `counter_increment` read it
29 times per activation (3095 IR instructions, 65 us). `Fx::rec_part` now reads only the selected leaves (one `SigRead`, or
`SigReadScalar` for a scalar leaf); the process is 633 instructions and 22 us. After it: op dispatch 44%, kernel scheduler 25%,
`Ctx` 10%, other machine 9%, `memmove` 3.5%, entry 3%.

**Why `--tier=auto` was slower** (`neorv32_single_min`): it compiled 146 functions, 8.4 s of compile CPU on the thread that shares
the pinned core (20% of the run), and the compiled small RTL processes ran about three times slower per activation than the
interpreter (bus gateway copy processes 1.8 us interpreted, 5.6 us compiled): the instruction count is the same
(167 G against 170 G) but cycles are 16% higher (cache and branch misses), compiled code spends its time in the host callbacks
(`Host::signal`, `drive_now`, `view_again`, `yavs_rt_host`) for every signal read and drive. The `RTL_BUDGET` rule (16 times the
compile budget because RTL "runs a tenth faster compiled") did not hold here. The policy fix is `52159b17` (#510: a clocked
process is compiled only when 16 or more share its code). `--tier=llvm` compiles 815k IR instructions up front (14 to 17 s) and
then runs 2.2x slower than the interpreter (#532).

**Levers left, ranked** (owner; sim1 idea in brackets):

1. Op dispatch and record-heavy process bodies, 44% (interpreter agents: #508, #443, #467 drives, #518 layout). sim1 compiles
   every process to straight-line code.
2. Concurrent signal-copy processes, 29% of process time (#529, kernel and elab): execute a copy statement in the kernel without
   the interpreter. [sim1: processes are tiny compiled functions; a record signal is one handle per field and a copy moves
   the fields.]
3. Kernel scheduler, 25% (kernel round 11, #440): 605 activations per clock cycle, wake scan and commit per activation.
   [sim1: a process wakes through a trigger it evaluates on the signal update, before it is queued; we do this for edges since round 11.]
4. User functions in loops (`or_reduce_f` 29 times per activation; #531, interpreter call cost, #443).
5. Sensitivity lists of 45 signals re-run their `WaitOn` sequence on every activation (#530, interpreter and kernel).
6. Compiled tier: native signal read and drive for vectors without a host callback, compile of 14 to 17 s for `--tier=llvm`
   (#532, #339, #527, LLVM agent).
7. Slices of an array of records still read the whole array (`rec_part` handles fields and indexes only); not on the hot path here.

**Threads** (`--threads N`, N cores = P-core 7 plus N-1 of the efficiency cores 23, 22, ...; interpreter, 100 us, median of 3,
`main` at `52159b17`, load average 13 from other agents; the efficiency cores are slower than core 7, so the speed-up is a floor):
dual_full 8.00 s at 1 thread, 7.76 at 2, 6.65 at 4, 6.19 at 8 (1.0x, 1.03x, 1.20x, 1.29x; CPU time user plus system 8 s at 1 thread, 17 s at 8);
single_min 6.46 s, 7.40, 5.52, 5.61 (1.0x, 0.87x, 1.17x, 1.15x). The earlier 1.75x at 4 threads and 2.3x at 6
(`docs/design/parallel-kernel.md`) timed a core asleep in `wfi` (#513) and do not stand. With the real program the gain is 1.2x to 1.3x at 4 to 8 threads for twice the CPU time; batch sizes were not
measured again here.

## Reduction idiom as a native (#531, #521, 2026-10-22)

A user function that is exactly `v := identity; for i in d'range loop v := v and|or|xor d(i); end loop; return v` over a
`std_ulogic` array runs as the unary reduction native (`crates/yavs-lower/src/reduce.rs`, `docs/decisions.md` "Reduction
idiom as a native"). Re-baselined on the 7950X3D, `main` at `ce929024` (interpreter 2.0 / 2.9 / 2.6 of sim1 for dual_full /
single_min / single_cache, `--tier=auto` 9.3 / 10.4 / 10.5 s against 4.5 / 3.3 / 3.8 s). `tools/realbench --compare`, core 9
(second CCD), median of 3, interpreter and `--tier=auto`:

| test | interp wall | interp instructions | auto wall | auto instructions |
|---|---:|---:|---:|---:|
| neorv32_dual_full | 9.23 to 8.56 s (0.93) | 92.98 G to 86.66 G (0.93) | 9.32 to 8.94 s (0.96) | 0.93 |
| neorv32_single_min | 10.02 to 9.51 s (0.95) | 111.85 G to 102.90 G (0.92) | 10.41 to 9.87 s (0.95) | 0.92 |
| neorv32_single_cache | 10.15 to 9.62 s (0.95) | 108.16 G to 98.70 G (0.91) | 10.52 to 10.01 s (0.95) | 0.91 |
| sc5_fifo_repack (uses `or_reduce`) | 4.11 to 4.13 s (1.00) | 0.99 | 4.04 to 4.16 s | 1.03 |

`neorv32_cpu_counters.counter_increment` per activation: 13.5 us to 7.1 us. Where the neorv32 time is now (per-op timing by sampling,
100 us of `neorv32_single_min`): signal reads and drives (`SigRead1` 29 ns, `Drive` 51 ns, `LSetDrive` 41 ns, `CopyDrive` 55 ns,
`AllocReadVec` 29 ns per op) are about 23% of interpreted time, the kernel (commit, wake scan, cycle) 13% of cycles, `exec_comb`
and the gateway `request` (about 150 and 110 per-leaf record drives per activation) 6.4% and 6.1% of process time. `--tier=auto`
against the interpreter: 4% more cycles for 1.3% more instructions; no function is compiled in these runs (0.7% of samples in
generated code), the extra cycles are the sampling variant of the dispatch loop (`run_ops::<false, true>`: jump counting 1%, the rest
code layout of the 120 KB function, #508).

## Static waits and checked kernel copies (#530, #536, #529, 2026-10-22)

Decisions: docs/decisions.md "Static waits and checked copies". The one wait of a single-suspension process skips its signal
references once the kernel holds it (`Op::WaitStatic` with `Ctx::wait_same`); a kernel copy (`ProcInst::copy`) now needs the same
subtypes on both sides (#536: `natural <= integer` lost its range check) and takes static slices. Tests: corpus
`process/static_waits`, `sigassign/plain_copies`, elab `signal_copies_run_in_the_kernel`.

`tools/realbench --compare`, core 7, `main` at `6f2b527d` (kernel copies already in) against this change, median, load average 10 to 12:

| test | interp wall | interp instructions | auto wall | auto instructions |
|---|---:|---:|---:|---:|
| neorv32_dual_full | 13.25 to 12.24 s (0.92) | 113.3 G to 104.8 G (0.92) | 12.84 to 12.14 s (0.95) | 0.93 |
| neorv32_single_min | 14.41 to 14.09 s (0.98) | 142.0 G to 130.5 G (0.92) | 15.66 to 14.07 s (0.90) | 0.92 |
| neorv32_single_cache | 17.32 to 14.28 s (0.82) | 135.0 G to 123.4 G (0.91) | 17.25 to 16.53 s (0.96) | 0.92 |

Against sim1 (39 to 44 G instructions per run), interpreter instruction ratios went from 2.60 / 3.81 / 3.30 (dual, single_min,
single_cache, `main` at `e4c8d983` this morning) to 2.37 / 3.37 / 2.96 with the kernel copies and this change. Not done: compiled
code (`--tier=llvm`, `auto`) still names every signal of such a wait (LLVM tier).

## Real-world suite: analysis, elaboration and code generation vs sim1, 2026-10-22

`tools/realbench` now times four phases apart (`--phases analyze,elab,codegen,run`, `tools/realbench/AGENTS.md` section 3), for yavs and
sim1, with a ratio yavs / sim1 per test, per category and overall. Analysis: the whole compile order of a test, ONE FILE PER CALL
(never batched, as the integrations do), into fresh libraries; cold = empty libraries, warm = the same unchanged files analysed again
into the populated libraries; the VUnit/OSVVM/UVVM/project libraries (`-lib`) and the test's own files (`-own`) are timed apart and the
library part once per corpus selection. Elaboration: `yavs elab` against sim1's elaboration step, cold (no compilation cache) and warm
(cache entries present). Code generation: `yavs run --tier=llvm` minus `--tier=interp`, both stopped at time 0, cold and warm (object
cache hit); sim1 generates its code inside its elaboration, so these ratios divide by sim1's whole elaboration and say how many times
longer the llvm tier needs before time 0 than sim1 needs to be ready. Run: the existing cells.

Method: main at `6ce6245e` (only the suite's own commit on top), release build with `--features llvm`, `--threads 1`.
**Timing core: efficiency core 13 for both yavs and sim1, one test at a time, the same core for both tools.** All performance cores
(1 to 7) were taken by other agents' timed runs, so these numbers are NOT performance-core numbers: absolute times are larger than the
2026-10-21 baseline above (an efficiency core is slower), and the host load was 9 to 13 from other agents' builds during the run, so
treat wall ratios of 10 to 20 percent as noise and use the instruction ratios (`cpu_atom/instructions/`). The headline numbers should be
re-run on a performance core when one is free (not done). Median of 5, 3 or 2 runs by length, cells interleaved, 41 tests (the 30 of the
2026-10-21 baseline, neorv32 (3), and the eight tests that were excluded then). Wall and CPU time come in steps of 10 ms, which matters
for the elaboration cells (30 to 300 ms). Raw: `~/.cache/yavs-realbench/results/20261022-112922-phases-baseline.json`.

Geometric mean of the ratio yavs / sim1 (below 1 is faster; the `codegen` rows divide by sim1's elaboration; `-lib` / `-own` split the
analysis; `n` = tests in the mean; the three neorv32 configurations hit the 300 s limit in `codegen`, #532):

| cell | n | wall, overall | instructions, overall | bus-tristate | cpu | fsm | memory | numeric | protected | records | rtl-datapath | strings-textio | wide-vectors |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| analyze-cold | 41 | **1.04** | 1.27 | 1.43 | 0.73 | 0.99 | 1.04 | 1.04 | 1.51 | 0.91 | 0.86 | 0.92 | 0.86 |
| analyze-cold-lib | 41 | **1.02** | 1.24 | 1.43 | 0.73 | 0.99 | 1.05 | 1.03 | 1.34 | 0.90 | 0.93 | 0.92 | 0.86 |
| analyze-cold-own | 41 | **1.15** | 1.21 | 1.00 | 1.00 | 1.00 | 1.00 | 1.10 | 1.78 | 1.11 | 0.81 | 1.00 | 1.00 |
| analyze-warm | 41 | **1.11** | 1.36 | 1.37 | 0.79 | 1.04 | 1.06 | 1.08 | 1.53 | 1.03 | 0.91 | 1.14 | 0.97 |
| analyze-warm-lib | 41 | **1.09** | 1.34 | 1.37 | 0.79 | 1.04 | 1.07 | 1.07 | 1.38 | 1.02 | 0.98 | 1.14 | 0.97 |
| analyze-warm-own | 41 | **1.17** | 1.21 | 1.00 | 1.00 | 1.00 | 1.02 | 1.17 | 1.72 | 1.12 | 0.92 | 1.00 | 1.00 |
| elab-cold | 41 | **1.44** | 1.98 | 1.44 | 0.70 | 1.84 | 1.25 | 1.07 | 3.44 | 1.33 | 1.36 | 0.73 | 0.91 |
| elab-warm | 41 | **0.25** | 0.36 | 0.25 | 0.44 | 1.09 | 0.12 | 0.13 | 0.29 | 0.32 | 0.14 | 0.09 | 0.29 |
| codegen-cold | 38 | **75.52** | 95.05 | 87.25 | - | 35.01 | 54.54 | 49.92 | 248.56 | 74.86 | 60.52 | 42.71 | 63.72 |
| codegen-warm | 38 | **8.49** | 10.24 | 9.75 | - | 4.05 | 6.09 | 5.84 | 26.78 | 8.86 | 7.86 | 4.84 | 4.95 |
| interp | 41 | **0.54** | 0.76 | 0.15 | 9.84 | 1.70 | 0.35 | 0.33 | 0.34 | 0.27 | 0.55 | 0.10 | 7.48 |
| auto | 41 | **0.54** | 0.74 | 0.15 | 12.32 | 1.72 | 0.32 | 0.33 | 0.35 | 0.30 | 0.38 | 0.10 | 7.50 |
| auto-cold | 41 | **1.35** | 1.73 | 0.72 | 13.86 | 2.04 | 1.20 | 0.99 | 0.82 | 0.95 | 1.26 | 0.52 | 8.07 |

Reading: analysis is on par with sim1 on average (1.04 cold, 1.11 warm; summed time 1.23x and 1.31x) but "warm" is no faster than
"cold" for yavs (no cache for `analyze`), and the OSVVM compile orders are 2.4x. Cold elaboration is 1.4x, warm 0.25x (the
compilation cache works, except on `poc` testbenches that evaluate constants during elaboration). Code generation of the llvm tier
costs 75x sim1's elaboration cold and 8.5x with a warm object cache (summed over the tests: 79x and 9.3x), 249x cold on the protected
category: 8 to 25 s of machine-code generation before time 0 against 0.03 to 0.3 s for sim1 to elaborate. Note for UVVM testbenches:
they do all their work at 0 ns, so "stop at time 0" still simulates them and the llvm minus interp difference also contains the run
speed difference of the tiers (the simulation part is 1.8 s of the 4.0 s warm llvm run of `uvvm_util_rand`).

The five slowest tests per phase (ratio to sim1; wall seconds yavs / sim1; reason from a quick `perf`/`--time-report` look):

| phase | test | ratio | seconds | reason |
|---|---|---:|---|---|
| analyze cold | `osvvm_axis_random` | 2.42 | 31.6 / 13.1 | 449 library + 72 own files at 61 ms per file (sim1 25): dependency units are lexed, parsed and declared again in every call (#537, #477) |
| analyze cold | `osvvm_axi4_random` | 2.32 | 25.1 / 10.8 | same, 475 + 46 files |
| analyze cold | `uvvm_sb_slv` | 1.54 | 2.07 / 1.34 | same, 62 + 3 files at 32 ms (sim1 21) |
| analyze cold | `uvvm_axistream_vvc` | 1.52 | 1.54 / 1.01 | same, 31 + 18 files |
| analyze cold | `olo_intf_i2c_master` (and the other Open Logic tests, one shared library part) | 1.43 | 11.4 / 7.9 | 386 files at 29 ms (sim1 21); one VUnit file re-reads 57 dependency sources (326 M instructions, median file 81 M) |
| elab cold | `poc_cache_cpu` | 9.2 (instr) | 0.26 / 0.08 | elaboration evaluates constants in the interpreter (46% `run_ops`), then lowers again (#539, #479) |
| elab cold | `poc_uart_rx` | 7.5 (instr) | 0.20 / 0.05 | same; also not helped by the warm cache (0.15 s, 3.0x) |
| elab cold | `uvvm_methods_log` | 7.3 (instr) | 0.15 / 0.03 | cold `elab` re-analyses the design closure from source: `analyze` 0.16 s of 0.17 s (#539, #405, #407) |
| elab cold | `uvvm_sb_slv` | 6.3 (instr) | 0.16 / 0.03 | same |
| elab cold | `uvvm_axistream_vvc`, `osvvm_axis_random` | 5.1 (instr) | 0.22 / 0.06, 0.21 / 0.05 | same |
| codegen cold | `uvvm_util_rand` | 511 | 25.6 / 0.05 | `--tier=llvm` compiles every reachable function before time 0 (`init` 21.7 s of 23.6 s): the whole verification library (#538, #432) |
| codegen cold | `uvvm_sb_slv` | 389 | 11.7 / 0.03 | same |
| codegen cold | `uvvm_func_cov` | 380 | 19.0 / 0.05 | same |
| codegen cold | `osvvm_axi4_random` | 306 | 24.5 / 0.08 | same |
| codegen cold | `uvvm_axistream_vvc` | 263 | 15.8 / 0.06 | same |
| codegen cold | `neorv32_*` (3) | > 300 s | killed | 815k IR instructions compiled up front (#532) |
| codegen warm | `uvvm_util_rand` | 67 | 3.3 / 0.05 | an object-cache hit still builds the whole LLVM module (bitcode writer and `ValueEnumerator` dominate a perf profile of `hdlm_fifo_sync`: 1.3 s warm, 19 MB of objects) (#538) |
| codegen warm | `uvvm_sb_slv`, `uvvm_func_cov`, `osvvm_axi4_random`, `uvvm_axistream_vvc` | 42, 39, 29, 28 | 1.3 to 2.9 | same |
| run (interp) | `poc_stat_histogram` | 11.4 | 5.2 / 0.46 | composite layout work at run time (#518) |
| run (interp) | `fletcher_bufferwriter_512` | 11.1 | 30.1 / 2.7 | 512 bit buses: interpreter and kernel signal path (#519) |
| run (interp) | `neorv32_dual_full`, `neorv32_single_cache`, `neorv32_single_min` | 10.3, 9.6, 9.6 | 66.7 / 6.5, 45.3 / 4.7, 41.4 / 4.3 | interpreter-bound CPU model, `--tier=auto` is slower still (#521, #532) |
| run (interp) | `hdlm_lfsr` | 7.7 | 9.7 / 1.3 | #523 |

Run phase on this core (ratio to sim1 for `interp` / `auto` / `auto-cold`): overall 0.54 / 0.54 / 1.35, summed time 4.7x / 5.0x / 5.7x
(the long tests decide the sum); instruction ratios 0.76 / 0.74 / 1.73. The classes are as on 2026-10-21 (CPU model and wide vectors
slow, short testbenches fast). Issues filed from this run: #537 (analyze per call), #538 (llvm tier before time 0), #539 (elab cold
and constants), #540 (tracked manifests name a reference simulator in `[flags]`, rule 11).

## Cold front end: prefix images on the second closure, cached files not parsed (#549, #551, #479, 2026-10-22)

Cold `yavs elab` of a library testbench (`tools/realbench` `--phases elab`, cold cell = pristine world, empty compilation cache; the
analysed-unit cache of the libraries is present as after `yavs analyze`). Profile of `osvvm_axis_random` before (143 ms in-process
timers): closure 2.5, replay of the first 44 files 36, std/ieee bodies 11, `settle` 5, three prefix images 43 (15, 15 and 12), rest of the
analysis 12, elaborate 19, lower 5. Two findings: (1) the first run of a closure wrote three prefix images (#549); (2) `Db::warm_stub`
never matched, because every unit entry begins with an empty segment, so each of the 73 files of the closure was lexed and parsed
although all its units had entries (#551, 16 percent of the run).

Changes: a prefix image is made when the second different closure starts with its prefix, one maker at a time (`Store::sightings` and
`Store::claim` in `crates/yavs/src/cache.rs`; `docs/decisions.md` "Prefix images ..."); `warm_stub` skips leading empty segments.

Before / after, same binary pair interleaved on cores 6 and 7, `--phases elab`, host load 12 to 15 from other agents (wall ratios
are noisy, the instruction ratios are not): geometric mean over 38 tests, elab-cold wall 0.71, instructions 0.68 (protected 0.60
and 0.64, numeric 0.62, memory 0.63); elab-warm unchanged (1.00 instructions). Instructions of the cold elab (M): `osvvm_axis_random`
1554 to 792, `uvvm_sb_slv` 825 to 505, `hdlm_axi_fifo` 1739 to 1145. Cold elab wall, medians of 9 on core 7 (core 6 in brackets), ms:
`osvvm_axis_random` 207 (232) to 98 (134), `uvvm_sb_slv` 128 (130) to 60 (85), `hdlm_axi_fifo` 223 (284) to 147 (202).
#479 (cache the lowered design, verify on first call) was done earlier (4959a178); what remains of its 40 ms is decoding the analysis of
the subprograms a first run reaches (`lower_with` cold 5 to 38 ms, 1 to 2 ms warm), which the library stages remove from the fourth closure on.

Not done, in order of expected gain (see the issue comments): the `settle` fact scan decodes every expression and statement of the
replayed units (5 to 16 ms cold); the std/ieee bodies replayed at every cold run (12 to 18 ms); poc and neorv32 elaborate 100 to 200 ms in
the interpreter at every run (#539, warm).

## Cold front end: facts in the unit entries (#552, 2026-10-22)

`Db::settle` decoded every expression and statement of the replayed units to find the elaborator's facts (`'active` reads, force and
release statements, package-path external names): 5 to 16 ms of a cold elab. An entry now stores the facts of its own entries
(found when it is made, ids mapped by the replay, entry format 2); `settle` scans only the entries of loads without recorded facts
and merges the rest. `differs_from` and `YAVS_UNIT_CHECK=1` compare the facts: osvvm, uvvm, hdl-modules, open-logic, vunit,
neorv32, poc, surf, satcat5, vhdl-extras, en_cl_fix, fletcher, json-for-vhdl (2008, and 2019 where listed) analysed twice into one
output directory all pass as expected. Test: `entries_carry_the_facts_of_their_unit` (fails with empty stored facts).

Before / after, same pair interleaved on cores 6 and 7, `--phases elab`, 38 tests, 5 runs, host load from other agents (read the
instruction ratio): elab-cold instructions 0.92, wall 0.93; elab-warm unchanged (1.00). Cold elab instructions (M) `osvvm_axis_random`
800 to 750, `uvvm_sb_slv` 510 to 440, `uvvm_func_cov` 550 to 500, `hdlm_axi_fifo` 1150 to 1100; analysis phase of a cold
`osvvm_axis_random` run (`--time-report`, core 6) 68 to 57 ms.

## Cold front end: instance bodies of generic packages (#555, 2026-10-22)

`Db::instance_body` cloned the 13 generic package instances of a cold `osvvm_axis_random` elab in 9.4 ms (of 11.5 ms at the first
prefix end). Perf shows a third of it in page faults (#556), a sixth in decoding the generic body's lazy pages, the rest in the
clone, where the memo tables of `Remap` hashed with SipHash. Changes (`crates/yavs-sem/src/generic.rs`): `Remap` tables are
`FxHashMap`, `fill_scope` builds no hash map per region, the protected-body loop of `fix_bodies` is in id order. Decisions and the
options that were tried or left: `docs/decisions.md` "Instance bodies of generic packages".

Before / after, `--compare` interleaved on cores 6 and 7, `--phases elab`, 8 tests, 11 runs: elab-cold instructions 0.98 (protected
5 tests 0.98; `osvvm_axis_random` 0.96, `osvvm_axi4_random` 0.96, `uvvm_sb_slv` 0.98), elab-warm 0.99 (the prefix image holds the
memo tables). Cold elab instructions (M) `osvvm_axis_random` 742 to 716. In-process time of the 13 `instance_body`
calls at the first prefix end, core 7, min of 8: 9.6 to 7.4 ms. Wall medians are within the 10 ms resolution of the table.
YAVS_UNIT_CHECK=1 over osvvm, uvvm (2008, 2019), hdl-modules (2008, 2019), open-logic, vunit (2008, 2019), neorv32, poc, surf,
satcat5, vhdl-extras, en_cl_fix, fletcher, json-for-vhdl: all as expected.

Not done: cloning only the bodies a run reaches (needs the stage and prefix rules to change, see the decision), the page faults of
the whole run (#556).

## Huge pages for the heap (#556, 2026-10-23)

`yavs_cache::huge_heap()` at the start of `main`: top pad 64 MiB, one growth step of the `brk` heap, `madvise(MADV_HUGEPAGE)` on it (Linux
with glibc and transparent huge pages in `madvise` mode; no-op elsewhere; `YAVS_HUGE_HEAP=0` off). Decision and the options:
`docs/decisions.md` "Huge pages for the heap". Test: `crates/yavs-cache/tests/huge_heap.rs`.

Before / after (B = with, A = without; same sources), `tools/realbench` `--compare` of all 41 tests (the five uvvm tests included since #560),
phases and cells as named, cores 6 and 7, cycles, instructions and page faults from `perf stat` (median of the timed runs, 2 to 5 runs,
host load 2 to 13 from other agents), geometric means of B / A over the tests; peak RSS is the geometric mean of the per-cell maximum:

| cell | instr | cycles | page faults | CPU time | wall (sum) | peak RSS |
|---|---:|---:|---:|---:|---:|---:|
| elab, cold | 0.949 | 0.857 | 0.145 | 0.826 | 0.857 | 1.079 |
| elab, warm | 0.932 | 0.812 | 0.251 | 0.831 | 0.760 | 1.077 |
| run, interp (warm) | 0.972 | 0.908 | 0.208 | 0.905 | 0.959 | 1.084 |
| run, auto (warm) | 0.973 | 0.906 | 0.244 | 0.924 | 0.977 | 1.087 |
| run, auto cold | 0.979 | 0.934 | 0.173 | 0.922 | 0.959 | 1.063 |
| analyze, cold (library part) | 0.952 | 0.876 | 0.506 | 0.870 | 0.904 | 1.089 |
| analyze, warm (library part) | 0.966 | 0.915 | 0.648 | 0.908 | 0.928 | 1.114 |
| analyze, cold (own files) | 0.969 | 0.924 | 0.589 | 0.805 | 0.870 | 1.051 |
| codegen, cold (llvm minus interp, time 0) | 0.998 | 0.987 | 0.756 | 0.986 | 0.998 | 1.018 |
| codegen, warm | 0.985 | 0.954 | 0.785 | 0.955 | 0.973 | 1.034 |

Absolute: cold elab of `osvvm_axis_random` 8 536 to 863 page faults, 354 M to 291 M cycles, peak RSS 51 to 54 MiB; median peak RSS
increase 2 to 4 MiB per cell, at most +12 MiB for the interpreter cells and +39 MiB (4%) for the 900 MiB runs that compile with LLVM.
By category (cycles, B / A): elab 0.80 (protected, memory, numeric) to 0.91 (fsm, rtl-datapath), run 0.83 (strings-textio) to 0.99
(wide-vectors), analyze 0.85 (protected) to 1.10 (bus-tristate: one shared library cell, noise). The `tools/bench` h2h designs (9 runs,
interp and auto, core 6 and 7): long RTL runs unchanged within noise (cycles 0.976 geometric mean, instructions 0.992, page faults
0.59, peak RSS +1 to 3 MiB; `rtl_ram`, which builds 48 MiB, 0.85 cycles). Single cells above 1.05 in the first pass
(`olo_fix_cordic_rot` auto cold, `hdlm_fifo_sync` and `uvvm_util_rand` elab cold, `poc_arith_addw` auto cold, `hdlm_axi_crossbar`
codegen cold) were run again with 11 (5) runs: 0.95, 0.85, 0.78, 1.02 (a 28 s LLVM run, instructions 1.000) and 0.99; no test is slower beyond noise.

## sem: deterministic ids and cache bytes across processes (#557, 2026-10-22)

`Db::r_instance` cloned a nested instance's `GenericMap` in std hash order (seeded per process), so the ids of the cloned subtypes,
expressions and declarations could differ between processes. `GenericMap` is a `BTreeMap` now (cloned in key order), `GenericState`
and `Library::units` are `FxHashMap`/`FxHashSet` (they were the only std hash containers in the serialized `Db`, so the image
bytes followed the process seed), the `inst_types` loop of the formal-package actual sorts, and `Remap`-style loops that allocate
ids already sorted. Test: `entries_are_the_same_in_every_process` (`crates/yavs/tests/cache.rs`, fails before the change).

## llvm: protected-method calls compiled, calls from the interpreter on native frames (#510, 2026-10-22)

Decisions: docs/decisions.md "Native calls from the interpreter and the object table". What changed:

- The object cache of compiled code is a direct-mapped table of 16 objects looked up inline by every access of unknown
  space; it remembers the access value a `Deref` found each object for, so the next `Deref` of that value is inline, and it
  keeps heap objects across a host call that deallocated nothing (`Mem::forget_since`). Before, the four-entry cache missed
  half of the shared-heap accesses of a protected method and was emptied at every step.
- The interpreter calls a compiled function that has native-frame code on a native frame (`Tier::call`), without pushing a
  frame of its own: argument cells from its registers, the result in the call's register, frames left to the interpreter
  pushed on the caller. `--tier=auto` compiles a hot native-callable function without a loop as native-frame code only.
- `YAVS_LLVM_PERFMAP=1` names the compiled functions for `perf report`.

`protected_queue` (`tools/bench/designs`), single thread pinned to performance core 4, median of 3 interleaved rounds after
one warm-up, `perf stat` user-mode `cpu_core` counters of the whole run, warm compilation cache; "before" is main at
2cddec62:

| Tier | instructions before | after | cycles before | after | ratio to the interpreter (cycles) |
|---|---:|---:|---:|---:|---:|
| `--tier=interp` | 32.93 G | 32.93 G | 6.02 G | 6.02 G | 1.00 |
| `--tier=llvm` | 30.52 G | 9.93 G | 5.91 G | 2.09 G | 0.35 |

In the compiled code of the queue the shared-heap lookups were 70 percent of the cycles before (`yavs_rt_host`,
`TierHost::object`, `Machine::obj`); after, 62 percent of the cycles are compiled code. `--tier=auto` gains as much once
the queue's functions are compiled (2.76 G cycles warm), but its tier-up policy does not compile them reliably within a run
of one second yet (next section).

Second step (same section, #510): the objects of the table live as long as they stay where they were found (frames and
temporaries until the next host call; heap objects while neither heap deallocated anything, also into the next activation
or `Tier::call` of the same machine; constants, instance and package data while the same machine runs), a `Deref` enters
its object at once, and reads of the instance data are inline. On the window-generator testbench of the real-world suite
(`vai_window_gen_interlock`, `--tier=auto` with a warm compilation cache after five runs) `yavs_rt_host` and
`TierHost::object` went from 11.4 to 6.0 percent of the cycles.

### `--tier=auto`, round 2: the policy for testbench code (#510)

Decisions: docs/decisions.md "Native calls from the interpreter and the object table" (`--tier=auto`, round 2). In short:
a function the interpreter calls on a native frame is compiled when hot however short, with eight times the budget; compile
cost is estimated at 800 microseconds per IR instruction (learned from real compiles only) plus the opening of LLVM; every
hot function the compilation cache lists as compiled before is linked from it at once (a look-up refused by the budget was
lost for the whole run before, so warm runs varied between 0.5 and 1.0 times the interpreter); frame-mode functions that
leave their compiled code often stay interpreted.

Instructions and cycles (G, `perf stat` user-mode `cpu_core` counters of the whole run, all threads), pinned to performance
core 4 (the compile thread shares it), median of 3 interleaved rounds after one warm-up; "warm" reuses the compilation cache
of the earlier rounds, "cold" empties it before every run; "before" is `--tier=auto` of main at 2cddec62:

| Testbench | interp | auto warm | auto cold | auto before (warm) |
|---|---:|---:|---:|---:|
| window generator (`vai_window_gen_interlock`) | 378.2 / 78.4 | 180.5 / 41.2 (0.53) | 240.5 / 55.2 (0.70) | 383.3 / 81.7 (1.04) |
| `protected_queue` | 32.9 / 6.00 | 28.0 / 5.30 (0.88) | 30.0 / 5.70 (0.95) | 33.4 / 6.11 (1.02) |
| `protected_queue`, 10x | 330.2 / 59.9 | 193.4 / 37.5 (0.63) | 213.7 / 42.3 (0.71) | 333.2 / 62.1 (1.03) |
| `uvvm_sb_slv` | 15.6 / 3.83 | 15.9 / 3.99 (1.04) | 15.9 / 4.11 (1.07) | 19.8 / 4.55 (1.20) |
| `uvvm_util_rand` | 16.0 / 3.64 | 16.2 / 3.68 (1.01) | 16.2 / 3.73 (1.03) | 16.4 / 3.75 (1.03) |
| `osvvm_axis_random` | 4.10 / 1.27 | 4.19 / 1.30 (1.02) | 4.22 / 1.32 (1.04) | 4.24 / 1.33 (1.05) |
| `osvvm_axi4_random` | 10.2 / 3.39 | 10.4 / 3.40 (1.00) | 10.4 / 3.48 (1.03) | 10.4 / 3.47 (1.03) |
| `uvvm_func_cov` | 3.23 / 0.77 | 3.28 / 0.79 (1.02) | 3.34 / 0.80 (1.05) | 3.41 / 0.83 (1.10) |

(ratio: cycles against the interpreter.) The window-generator testbench was 9 times sim1 in cycles before; at 41 G cycles it
is about 4.2 times. The short library testbenches (0.15 to 0.9 s) compile next to nothing within the budget; what `--tier=auto`
still costs there (1 to 4 percent) is mostly the interpreter's own bookkeeping with a tier set (1 to 3 percent with nothing
compiled, measured with an infinite heat), not compilation.

Third step (#510): the frame-mode code of a function that does not run on native frames itself (a testbench process, a
procedure with a wait) calls the callees that do directly (`compile_one` takes its callees as roots of `native_set`; before,
`--tier=auto` gave such code no native calls at all, so a compiled queue loop stepped every call and the interpreter ran a
callee that has only native-frame code), a probe compiles the native-frame callees a native root needs before it is
published, and a look-up gets at least the budget of a function called natively. Same method as the table above (core 4,
median of 3, cycles G and ratio to the interpreter):

| Testbench | interp | auto warm | auto cold |
|---|---:|---:|---:|
| `protected_queue` | 5.99 | 2.26 (0.38) | 5.84 (0.97) |
| `protected_queue`, 10x | 59.9 | 19.6 (0.33) | 27.0 (0.45) |
| window generator | 78.9 | 43.9 (0.56) | 60.7 (0.77) |
| `uvvm_sb_slv`, `uvvm_util_rand`, `osvvm_axis_random`, `osvvm_axi4_random`, `uvvm_func_cov` | | 1.01 to 1.04 | 1.01 to 1.04 |

`--tier=llvm` runs `protected_queue` at 2.09 G cycles: warm `--tier=auto` now reaches the eager tier.

Fourth step (#510): a clocked process is compiled only when at least 16 processes are made of it. In the real-world suite
(`tools/realbench`, all 41 tests, core 4, before this step) `--tier=auto` ran the three CPU-design tests 5 to 8 percent and
the wide-vector writer 5 percent slower than the interpreter at the same instruction count: their processes, most made
once, were compiled; with the processes not compiled, both are level with the interpreter (CPU design 130.0 against 130.6 G
cycles; wide-vector writer 1.01 of the interpreter), while `rtl_ff` (0.93), `rtl_lfsr` (0.75) and `lfsr64` (0.90) keep their
gain (core 4, warm, host load 10 to 15 during these runs, so cycle ratios carry several percent of noise).

### `--tier=auto` as the default: the case (#510, 2026-10-22; the maintainer decides)

Real-world suite (`tools/realbench`, 18 tests where tier-up matters, main at 52159b17, core 4, median of 3, run phase wall
seconds; warm cells reuse the compilation cache, cold cells copy a pristine world, so they include building every cache):

| Test | interp | auto | auto / interp | interp cold | auto cold |
|---|---:|---:|---:|---:|---:|
| window generator (`vai_window_gen_interlock`) | 21.29 | 10.90 | 0.51 | 23.21 | 15.34 |
| `hdlm_lfsr` | 8.27 | 4.14 | 0.50 | 8.37 | 4.42 |
| `poc_stat_histogram` | 5.51 | 3.40 | 0.62 | 5.07 | 4.37 |
| `sc5_fifo_repack` | 4.93 | 4.60 | 0.93 | 5.15 | 5.42 |
| `uvvm_axistream_vvc` | 5.76 | 5.73 | 0.99 | 5.90 | 6.24 |
| `osvvm_axis_random`, `uvvm_sb_slv`, `uvvm_util_rand`, `uvvm_func_cov`, `osvvm_axi4_random` | 0.21 to 0.96 | | 0.99 to 1.03 | | |
| `poc_cache_cpu` | 2.46 | 2.51 | 1.02 | 2.53 | 2.85 |
| `poc_arith_addw` | 37.84 | 39.09 | 1.03 | 39.67 | 40.08 |
| `neorv32_dual_full` | 16.69 | 17.14 | 1.03 | 17.23 | 17.61 |
| `neorv32_single_cache` | 20.99 | 21.99 | 1.05 | 22.04 | 20.89 |
| `fletcher_bufferwriter_512` | 25.33 | 26.54 | 1.05 | 28.00 | 27.21 |
| `fletcher_bufferreader` | 7.33 | 7.68 | 1.05 | 7.66 | 8.03 |
| `neorv32_single_min` | 17.64 | 18.75 | 1.06 | 18.44 | 19.64 |
| `sc5_timestamp` | 0.82 | 0.87 | 1.06 | 0.86 | 0.87 |

The whole suite before the last two steps (41 tests, `--ref sim1`): geometric mean of wall against sim1 0.54 interpreted,
0.55 auto (instructions 0.76 and 0.74), cold 1.26 and 1.28. The h2h designs (core 4, warm, cycles against the interpreter):
`rtl_ff` 0.93, `rtl_lfsr` 0.75, `lfsr64` 0.90, `compute_h2h` 0.14; `rtl_chain`, `rtl_fan`, `rtl_bus`, `rtl_ram` 1.00 to 1.03;
`lfsr64_xor` 1.09 (80 ms: the LLVM library and one compile); at 10x length `rtl_ff` 0.91, `rtl_lfsr` 0.66, `lfsr64` 0.78,
`lfsr64_xor` 0.89, the others 1.00 to 1.03.

For: testbench-heavy runs and designs with many instances of one clocked process gain up to 2x (window generator, the
queue-based testbenches, `hdlm_lfsr`, the histogram, `rtl_lfsr`); compute loops 7x; output is identical in every tier
and thread count; the compilation cache makes repeat runs warm.

Against: the runs where nothing pays are 1 to 6 percent slower than the interpreter (the CPU and wide-vector designs, the
short library testbenches): about 1 to 3 percent was the interpreter's own bookkeeping with a tier set even when nothing is
compiled (#525, now noise except on designs of thousands of one-line processes), the rest is the compile thread (pinned to the same core in these measurements; on a free core it costs
CPU, not wall) and the LLVM library (10 ms at the first compile); cold runs of short testbenches pay up to 4 percent; the
compile thread adds CPU and memory to every run that compiles; when a function is compiled depends on sampling, so run
times (not results) vary more than with the interpreter.

**Decision (user, 2026-10-22): `--tier=auto` is the default** (builds with the `llvm` feature; `--dap` keeps the interpreter, as do builds without the feature, quietly; `--vhpi` follows the default, #125; `docs/decisions.md`). The blockers below were cleared by #525, #532 and #545.

Recommendation at the time: not yet. Make `--tier=auto` the default once a run in which nothing pays costs no more than noise
(#525), and once compiled clocked processes of single-instance designs no longer lose (#532, #527: signal operations and
array accesses without host calls); keep `--tier=interp` for runs below a second, where nothing can pay.

### `--tier=auto` on the CPU designs (#532, #545, 2026-10-22)

Default `--tier=auto` no longer compiles a clocked process that is under an eighth of the sampled time (`RTL_SHARE`):
the seven processes it compiled on neorv32 each cost 200 to 900 instructions more per activation than interpreted.
`hdlm_lfsr` compiles its tap loops again since the natural loops of #547 (#545); `--verbose` now says when the compile
budget refuses a hot function.
Main-thread instructions, core 4, `--tier=auto` against `--tier=interp` of the same binary: `neorv32_single_min` +2.5%
to +0.5%, `_single_cache` +1.7% to +0.6%, `_dual_full` +3.0% to +0.6% (cycles -0.7 to +0.5%; the rest is the
per-activation cost of #544); nothing is compiled there now. The h2h designs and `protected_queue` are unchanged within
0.5% (`rtl_ff`, `lfsr64`, `lfsr64_xor`, `rtl_lfsr` still compile their process).

Real-world suite (`tools/realbench --compare` before/after, `--ref sim1`, cores 4,5, loaded host, all threads counted;
the after binary had an innermost-loop `compute` rule since replaced by #547, which compiles `hdlm_lfsr` the same way):
geometric mean of auto/interp instructions 0.977 before and after; per test after: `hdlm_lfsr` 0.52 (1.01 before),
`poc_stat_histogram` 0.51, `vai_window_gen_interlock` 0.74, `poc_arith_addw` 0.95, neorv32 1.01 to 1.02 (1.03 to 1.04
before), the rest 0.94 to 1.03 except tests under 0.1 s, where sampling noise of the whole process is a few percent.
Against sim1 (rule: `--tier=auto` only, total wall of the run command against sim1's elaboration plus run): geometric
mean warm 0.49, cold 1.17 (1.22 before); the CPU designs 3.0 warm and cold.

### `--tier=auto` with nothing compiled (#525, 2026-10-22)

The interpreter's bookkeeping with a tier set is now a loop of its own (decisions.md "`--tier=auto` with nothing compiled
costs the interpreter almost nothing"), and the compile thread runs off the CPUs of a pinned run. `perf stat` instructions
(cycles alongside), core 4, min of 2, `auto0` = `--tier=auto` with a heat so high that nothing is compiled, against
`--tier=interp` of the same binary (before: main at 3015e1ab):

| Test | before auto0 / interp | after auto0 / interp | after, warm `auto` / interp |
|---|---:|---:|---:|
| `rtl_ff` | +2.9% (+6.0%) | +3.3% (+2.0%) | +0.6% (-1.8%) |
| `rtl_lfsr` | +1.3% (+0.2%) | +0.2% (-5.0%) | +0.2% (-4.3%) |
| `sc5_timestamp` | +1.1% (+1.5%) | +1.0% (+2.6%) | +1.0% (+1.3%) |
| `neorv32_single_min`, `_single_cache`, `_dual_full` | +1.1 to +1.2% | +0.4% (-0.3 to +0.3%) | +1.4 to +2.0% (#532) |
| `fletcher_bufferwriter_512`, `_bufferreader`, `_prim32` | +0.3 to +0.5% | -0.1 to 0.0% | -0.2 to +0.1% |
| `poc_arith_addw` (core 5, one run) | +0.4% | -0.3% | |
| `protected_queue` | +1.4% | +0.5% | -33% (was -18%) |
| `vai_window_gen_interlock` | +1.2% | +0.1% | -25% (unchanged) |
| `poc_stat_histogram` | +1.2% | -0.7% | -47% (unchanged) |

The interpreter itself (`--tier=interp`) is unchanged within noise (`rtl_ff` 2% fewer instructions). What is left is the
sampling slot of each activation (about 15 instructions): visible only on designs of thousands of one-line processes with
nothing compiled (`rtl_ff`, about 750 instructions per activation; default `auto` compiles that process). The compile
thread with nothing to compile costs about 5 us of CPU per millisecond on another core; the LLVM library is not loaded
in a run that compiles nothing. Cycles vary 2 to 5 percent between runs on the shared host; instructions are the measure.

### `--tier=auto` with nothing compiled: the cycles (#521, #544, 2026-10-22)

On neorv32 (`single_min`, 100 us, core 9, `perf stat` cycles:u, median of 11, interleaved) `--tier=auto` ran 4.3 to 5.4%
more cycles than `--tier=interp` for 0.9% more instructions, nothing compiled. Cause: the watching form of the dispatch
loop (`run_ops::<false, true>`) executed the same number of instructions as the plain one (12.6 G each) but needed 5.7%
more cycles; the every-128-events `watch_tick` (thread-local slot access, flag look) was inlined into the 100 KB function.
Out of line (`#[cold]`) the form costs the same as the plain one: auto / interp cycles 1.054 to 1.011 (+1.0% instructions,
the per-activation set-up and the compile thread). Disabling only the call look-up or the jump count recovered 1 to 2%.
Designs of thousands of tiny processes with nothing compiled (`--tier-heat=100000000,100000000,1000`, core 10, median
of 11): `rtl_ff` +3.8% cycles (+3.5% instructions, was +5.8% / +4.3%), `rtl_fan` +3.9% (+6.4%, was +4.8% / +8.4%),
`rtl_lfsr` +2.6% (+2.0%), `rtl_chain` +1.2% (+2.6%). The rest is the per-activation set-up of the
watching form, 20 to 25 instructions (#544).

## Interpreter cost on loops, arrays and calls: round 1 (#520, #523, #509, #511, #531, 2026-10-22)

What the four slow real-world tests cost (`perf stat` instructions of one `--tier=interp` run, release build with the `llvm` feature;
"before" is main at `6ce6245e`, "after" this round; the reference simulator's count in brackets) and their wall time against it
(`tools/realbench --ref`, core 6, median of 3, after / before in the run of the baseline day):

| test | instructions before | after | wall x reference, interp | auto |
|---|---:|---:|---:|---:|
| `poc_arith_addw` (#520) | 672 G (89 G) | 583 G | 5.5 (7.0) | 5.3 |
| `vai_window_gen_interlock` (#520) | 379 G (40 G) | 327 G | 4.3 (4.7) | 2.4 |
| `hdlm_lfsr` (#523) | 136 G (17.5 G) | 96 G | 5.1 (6.9) | 5.1 (#545) |
| `fletcher_bufferreader` (#523) | 98 G (15.3 G) | 75 G | 5.3 (7.2) | 5.4 |

Where it goes: `poc_arith_addw` builds an error message for every one of 8.4 million assertions (about 130 operations each) and
drives a 20-instance DUT; `vai_window_gen_interlock` is the verification library's queue (115 operations per `unsafe_push` after
inlining: pointer-table derefs, positions, range checks of `natural` arguments); `hdlm_lfsr` is the tap loop over a constant array
of the instance; `fletcher_bufferreader` is a design of small clocked processes (about 145 host instructions per operation, a
third of it kernel: #440, #519). The dispatch (one indirect jump) is 12 to 24 percent of the cycles in all four. A call cost
about 540 host instructions (the `calls` benchmark: 2289 M to 551 M instructions now). The whole suite, main before against
this round (`--compare`, 41 tests, interp): geometric mean of the wall ratio 0.85 (instructions 0.82), summed time 0.60; the
protected category 0.91, records 0.96, numeric 0.93 (others' kernel work in the same range included).

What changed (decisions in `docs/decisions.md` "Optimiser round 11", the commit messages have the numbers of each step):

- Optimiser: instance data and the constant pool are read-only for the whole function; registers defined more than once (loop
  counters, promoted variables) get versions, so positions, loads and sums on them are shared and constants they hold are known
  (#511); decided branches in threaded blocks and one more walk after the clean-up; one-byte variables (boolean, bit, enumeration)
  are promoted to registers.
- Inlining (`opt/inline.rs`, #509, #531): callees with branches (a case on a storage mode, an assert) become appended blocks
  (at most 60 instructions in 8 blocks: bigger bodies make units the compiled tier compiles slowly, see decisions.md);
  `CallProt` and process callers inline; a wrapper in front of a method is one callee (level 1); bodies may call natives and read
  logic arrays; the inlining record is a chain (traces and `std.env` call paths unchanged, corpus `env/call_path_*`).
- Lowering: the descriptor of an array constant or signal of locally static bounds is a constant of the pool, also when an index is
  a type mark, so lengths, directions and positions fold.
- Interpreter: a call that passes 8-byte parameters in registers writes no slots (`Pre::lean`).

Ideas of the reference simulator used (read for the design, no code): numbering the values of registers instead of the registers
(`Opt::vsn`), promoting one-byte slots like wider ones, protected methods as plain calls with the object as first argument (so
they inline like any call), arguments passed in registers without a callee frame store, and facts of the elaborated instance
available to the code generator (the static descriptors).

Left: the pointer-table navigation of the queue (facts do not survive the joins that inlined bodies make, #444); the cost of
message building in testbenches (a native result is three more operations to read its string, a concatenation an allocation, a
copy and a descriptor); loop-invariant loads of instance data in loops of processes (#541); under `--tier=auto` the compile
heuristic `compute()` stopped compiling the lfsr loop once the optimiser shrank it (#545); designs with many small clocked
processes (kernel, #440).

### Interpreter round 12: process loops, zero-delay drives, `'image` (#541, #547, #520, #440, 2026-10-22)

Four changes (`docs/decisions.md` "Loop-invariant code motion" and "Interpreter round 12"):

- Loop-invariant code motion also runs on the loops of a process that hold no suspension, and a load from read-only memory
  (instance data, constant pool) at a constant address moves like a constant (#541). The tap loop of the hdl-modules LFSR is
  two operations per pass instead of four.
- `--tier=auto` finds loops as natural loops (#547): the preheaders and inlined bodies the optimiser appends after the code made
  `compute()` count whole functions as loops and refuse them (`compute_h2h` stopped compiling with the first change; `hdlm_lfsr`
  `main` is compiled again, which #545 had lost).
- A zero-delay `LSetDrive` or `CopyDrive` of one range of a logic signal calls the kernel's fast case directly
  (`Ctx::assign_code_now`, new `Ctx::assign_planes_now`).
- `'image` of an integer or enumeration value is `Op::Image`, fused with the reads of its string (`Op::ImageStr`). The message a
  testbench builds for an assertion procedure is an actual parameter and stays eager (LRM); it cost 146 G of the 595 G
  instructions of `poc_arith_addw` (8.7 k per message), now about 106 G.

Instructions, interpreter, core 6 (`perf stat cpu_core/instructions/u`, warm), before the round against after:
`hdlm_lfsr` 94.7 G to 78.3 G; `poc_arith_addw` 594.7 G to 554.7 G (the last change alone); `rtl_ff` 12.14 G to 11.84 G, `rtl_chain`
3.37 G to 3.29 G, `rtl_lfsr` 7.32 G to 7.19 G, `lfsr64` 3.51 G to 3.45 G, `report_heavy` 1.16 G to 1.11 G; the other `tools/bench`
designs level. `hdlm_lfsr` under `--tier=auto`, warm: 4.3 s to 2.1 s wall.

Real testbenches (`tools/realbench/run.py --compare`, core 6, one run, `main` at 2ca775bc against the round on top of it, all 41
tests, B/A): instructions geometric mean 1.00 interpreter, 0.97 `--tier=auto`; `hdlm_lfsr` 0.81 / 0.47, `poc_arith_addw` 0.92 / 0.91,
`poc_stat_histogram` 0.99 / 0.83; summed wall 0.99 / 1.00 (one run each, host loaded). `en_cl_fix_pkg` (0.04 s) runs 1.15 times the
instructions on both tiers, left unexplained. Against sim1 with `--tier=auto` (total wall including compile, sim1 elaboration plus
run, core 6): `hdlm_lfsr` 2.25 warm, 2.49 cold; `poc_arith_addw` 4.26 / 4.66; `fletcher_bufferreader` 4.45 / 4.62.

Ideas of the reference simulator used (read for the design, no code): a separate entry for a scalar assignment at zero delay
that skips the delay and rejection checks (ours keeps one kernel fast case and shortens the way to it); its lowering hoists only
the bounds and step of a `for` loop, ours moves any invariant computation and read-only load out of a loop without a wait.

Left: the rest of the message cost (the concatenation, about 930 instructions per message, and the second `to_integer` of the
same signals, which a conversion's warnings keep from being shared); the drive path's kernel half (two driver lookups per
assignment, one on each side of `Ctx`); `tier_auto_matches_interp` still fails about one run in three (#533).

### Temporaries found inline in every compiled function (#527)

Array descriptors and plane blocks a caller makes as temporaries reached compiled functions only through `yavs_rt_host`
(a process found the temporary area inline since round 3, other functions did not): on `poc_stat_histogram` every one of the
80 million host-address calls of a run was a temporary. Every function now tests the temporary area inline after the object
table. Core 4, warm `--tier=auto`, cycles against the interpreter: `poc_stat_histogram` 0.42 (before about 1.0 in the same
session), window generator 0.50 (0.54), `uvvm_sb_slv` 1.02 (1.05).

### Compiled waits on several signals (#532, first step)

A process sensitive to several signals runs one `WaitOn` per signal before its `Suspend`; compiled code ran each through the
generic signal callback (`TierHost::signal`): 55 million callbacks in a run of `neorv32_single_min`. It now asks the kernel
first to keep the waiter of the last wait (`yavs_rt_wait_on`, `TierHost::wait_on_again`). `--tier=llvm` on
`neorv32_single_min` (core 4, warm): simulate 29.8 to 24.0 s, 116 to 105 G cycles; the interpreter takes 60 G. The rest of the
gap is the code footprint of the compiled processes: 5 times the instruction-cache misses (5.4 G against 1.1 G) and 4.5
times the instruction-TLB misses of the interpreter, from a 27 MB object for the design's processes compiled once each
(`--tier=auto` leaves single-instance clocked processes interpreted for that reason, round 2). h2h designs unchanged.

The wait of a process that names the same signals every time (the interpreter's `WaitStatic`, #530) takes the same short
cut in compiled code: at its first statement `yavs_rt_wait_same` asks the kernel whether it still holds the last wait
(`Ctx::wait_same`) and the process suspends without naming the signals. `--tier=llvm` on `neorv32_single_min` (core 4,
warm): 166 to 149 G instructions, 105 to 101 G cycles (interpreter 131.5 G / 55.9 G).

`rtl_lfsr` (`tools/bench/designs`, interpreter, core 5) is back to 2890 instructions per cycle and process from 3750 after
optimiser round 11 (#534): memoisation across a block and its sole successor, a cell of its own for shared linear-run
temporaries, and fusions across block starts nothing jumps to (`docs/decisions.md` "Shared offsets must stay memoisable and
fusable"). `YAVS_DUMPIR=1` dumps the finished IR and the predecoded ops of every function that runs.

## Out-port collapsing (#528, 2026-10-22)

`out` and `buffer` ports that are the only source of their actual's elements collapse into the actual like `in` ports
(`collapse.rs` `ports`): the formal's drivers drive the actual, which takes the formal's initial value; `inout` ports, actuals
with another source, a guard, a force, a conversion or an effective value from above stay apart (`docs/decisions.md`
"Elaboration"). In `fletcher_bufferwriter_512` 422 of 450 `out` leaves collapse (20 are open, 8 kept).
Release `--features llvm`, `tools/realbench`, core 7. Interpreter, before / after (`--compare`, median): `fletcher_bufferwriter_512`
instructions 0.80, wall 0.75; `neorv32_dual_full` 0.96 and 0.91 (CPU); `neorv32_single_min` 0.92 and 0.84 (CPU).
`--tier=auto` against sim1 (total wall of `yavs run`, compilation included, against sim1's elaboration plus run; instructions
in brackets), before and after:

| test | auto warm before | auto warm after | auto cold before | auto cold after |
|---|---:|---:|---:|---:|
| `fletcher_bufferwriter_512` | 7.9x (6.6x) | 5.8x (5.2x) | 8.5x (6.6x) | 6.2x (5.3x) |
| `neorv32_dual_full` | 2.5x (2.2x) | 2.4x (2.1x) | 2.6x (2.3x) | 2.5x (2.2x) |
| `neorv32_single_min` | 3.7x (3.2x) | 3.3x (3.0x) | 3.8x (3.3x) | 3.5x (3.1x) |

Whole suite (41 tests, cores 6 and 7): geometric mean of instructions 0.98 (interp and auto), wall 0.97; `fletcher_bufferreader`
0.81 instructions; no slowdowns (two flagged cells rerun on core 7 were noise). Checked: differential corpus 444/444 collapsed
and with `--no-collapse-ports` (4 threads), `--tier=auto` and `--tier=llvm`; new `hierarchy/out_port_collapse` matches sim1 both
ways; VHPI conformance traces identical before and after (collapsing is off under `--vhpi`); waves of the `hierarchy` and `force`
groups equal with and without collapsing (`crates/yavs/tests/wave.rs`); `cargo test --workspace`.

## llvm: exits of native frames record what is live (#554, 2026-10-22)

Decision: docs/decisions.md "Native frames: every exit records the registers live at its pc". `--tier=llvm` ran the
codec and queue loops of a verification library (576-bit `decode(encode(v))`, queue push and pop) and a window-generator
testbench slower than the interpreter because all guards of a native-frame function joined one block that took every
register through phis. Each exit now stores only the registers live at its pc. `--tier=llvm`, same core, interpreter
in brackets: codec loop 0.81 s to 0.13 s [0.16 s], queue 2.28 s to 0.77 s [1.34 s], window generator 46.4 s to 26.0 s
[31.6 s]; `--tier=auto` unchanged in run time (17.4 s; the reference simulator 4.1 s), its hot library functions compile
3 to 7 percent faster.
Memory access of compiled code (#554): a register `Deref` made is a known heap address, so its accesses skip the frame,
temporary and instance tests, and the object table key separates private and shared heap objects of the same index.
`--tier=llvm`, same core, interpreter in brackets: queue push and pop 0.79 s to 0.69 s (0.48 s with `YAVS_LLVM_FAST_MIN`
raised, the code `--tier=auto` compiles at `O2`) [1.33 s], 2 million accessor calls 0.114 s to 0.074 s [0.221 s].


## llvm: `--tier=auto` leftovers, search loops and the clocked-process band (#573, #575, 2026-10-23)

Decisions: docs/decisions.md "search loops stay interpreted, the clocked-process band is 160 to 300". Benchmark builds
(`tools/build-bench.sh --features llvm`), core 6, `perf stat --no-inherit`, cycles (median / minimum) and instructions
relative to `--tier=interp`, warm, 11 to 15 interleaved runs:

| design (IR instructions of the process) | auto before | auto now |
|---|---|---|
| `uvvm_sb_slv` (search function `pos_of_leftmost`, 63) | 1.093 cycles, 1.120 instr | 1.009 / 1.017, 1.015 instr |
| `rtl_ff` (136, 4096 instances; also 64 to 1024 and ten times the length) | 1.08 to 1.34 (minimum), 1.05 instr | 1.02 to 1.14, 1.028 instr |
| `lfsr64` (132, 64 instances) | 1.00 to 1.06 | 0.99 to 1.04 (refused now too: level within the noise) |
| `rtl_lfsr` (206) | 0.84 to 0.98 | 0.74 to 0.94 (same policy: compiled) |
| `rtl_ff` with 4 and 6 more terms (228, 274) | 0.95 and 0.93 | compiled as before |
| `rtl_ff` with 8 and 14 more terms (320, 458) | 1.12 and 1.14 to 1.20 | interpreted: 0.98 to 1.01 and 1.07 |
| `poc_arith_addw` main-thread instructions, warm | 384.4 G | 384.5 G (unchanged) |

`realbench --compare` of main against this change on 13 tests (cold and warm, interp and auto, cores 6 and 7): instructions
identical (within 1 percent) everywhere but `uvvm_sb_slv` (auto 14.84 G to 13.68 G, cold 17.56 G to 14.66 G); the short tests
(0.02 to 0.3 s) are unchanged cold and warm; wall deviations on `sc5_timestamp` and `hdlm_lfsr` are host noise (the interpreter
cell moves the same way at equal instructions).

Findings: `uvvm_sb_slv` spends half of its interpreted run in one search loop (`pos_of_leftmost`, 0.45 s of 0.9 s) that the
interpreter fuses (`scan_loop`, about 1.5 cycles an element) and compiled code reads at 3 to 4 cycles an element (#578). The compiled
clocked process of `rtl_ff` spends a third of its cycles in the plumbing around the body (`enter` 8 percent, `yavs_rt_drive`
10, `view_again` 6, `run_cached` 3.5, `wait_again` 3), the body itself 3 percent: the entry of a compiled activation costs
what the interpreter's `run_quick` path costs, so only bodies of 200 to 280 instructions repay it (#532). The three helper
threads of a run (#562) were not changed: see the issue for the options.

## llvm: `--tier=auto` compile policy (#572, #520, #493, 2026-10-23)

Decisions: docs/decisions.md "`--tier=auto` compile policy". In short: `Worker::pick` compiles the hottest candidate per
cost that the budget has room for (it used to stop at the best one and wait for it, holding up every other), a function
the interpreter calls on a native frame is compiled after twice its compile time seen (was four times) and gets 16
times the budget (was 8), and `YAVS_AUTO_TRACE=1` prints the compile thread's timeline. Where the time of the window
generator goes: warm `--tier=auto` (16.2 s) is already faster than the whole program compiled at `O2` by
`--tier=llvm` (17.0 s of run after 1.8 s of load and link, measured before #565) and a warm run that compiles everything
hot at once (payback 1, budgets of a thousand) is no faster (16.4 s against 16.0 s, same build), so the remaining distance to the reference simulator (3.9 s) is compiled-code quality and the
20 percent of the sampled time in big processes whose compiled code is not faster, not the tier-up policy; the gap between cold
and warm is the wait for evidence (a function has to be seen interpreted for twice its compile time, seconds for a 3 percent
function), not the queue (10 to 100 ms from candidate to compiled) or the compile.

Method (tools outside the repository, as `tools/realbench/AGENTS.md` describes the world): a fresh world per binary and tier,
warm = two untimed runs then the timed ones, cold = a copy of the pristine world per run; `perf stat --no-inherit` on the
exec'd `yavs`, so main-thread only (the compile thread's work is not in it; its CPU is the difference between total user
time and wall); median of 5, interleaved, order swapped every round; window generator on core 13, the rest on core 14.
Each cell is wall s / main-thread cycles G / main-thread instructions G; "main" is 311f78b7, "now" this change:

| test | interp warm | auto warm, main | auto warm, now | interp cold | auto cold, main | auto cold, now |
|---|---:|---:|---:|---:|---:|---:|
| `vai_window_gen_interlock` | 29.1 / 144.1 / 501.5 | 16.4 / 80.9 / 259.8 | 16.2 / 80.7 / 251.5 | 29.3 / 145.6 / 503.2 | 19.0 / 94.5 / 308.8 | 18.3 / 90.7 / 298.0 |
| `poc_stat_histogram` | 1.92 / 9.45 / 40.8 | 0.75 / 3.65 / 17.4 | 0.75 / 3.66 / 17.4 | 1.94 / 9.53 / 41.0 | 1.10 / 5.37 / 23.9 | 1.10 / 5.32 / 23.8 |
| `hdlm_lfsr` | 3.70 / 18.26 / 67.6 | 2.10 / 10.31 / 37.1 | 2.11 / 10.38 / 37.1 | 3.74 / 18.43 / 68.0 | 2.30 / 11.31 / 40.5 | 2.31 / 11.30 / 40.3 |
| `uvvm_sb_slv` | 0.82 / 3.96 / 13.6 | 0.89 / 4.34 / 15.3 | 0.88 / 4.24 / 15.3 | 0.90 / 4.36 / 14.6 | 0.98 / 4.69 / 15.7 | 0.96 / 4.61 / 15.8 |
| `uvvm_util_rand` | 0.93 / 4.49 / 15.0 | 0.95 / 4.60 / 15.0 | 0.92 / 4.49 / 15.0 | 1.03 / 4.96 / 16.2 | 1.08 / 5.14 / 16.3 | 1.06 / 5.08 / 16.3 |
| `uvvm_axistream_vvc` | 3.58 / 17.04 / 51.1 | 3.52 / 16.83 / 49.1 | 3.53 / 16.72 / 49.2 | 3.58 / 17.43 / 52.1 | 3.80 / 18.56 / 54.7 | 3.82 / 18.60 / 54.6 |
| `uvvm_func_cov` | 0.20 / 0.86 / 3.1 | 0.21 / 0.91 / 3.1 | 0.20 / 0.88 / 3.1 | 0.32 / 1.38 / 4.3 | 0.32 / 1.33 / 4.1 | 0.32 / 1.31 / 4.1 |
| `osvvm_axi4_random` | 0.60 / 2.86 / 8.0 | 0.61 / 2.89 / 8.0 | 0.61 / 2.88 / 8.0 | 0.74 / 3.46 / 9.2 | 0.74 / 3.48 / 9.2 | 0.74 / 3.47 / 9.2 |
| `osvvm_axis_random` | 0.28 / 1.26 / 4.0 | 0.29 / 1.32 / 4.0 | 0.28 / 1.28 / 4.0 | 0.38 / 1.71 / 5.0 | 0.40 / 1.75 / 4.9 | 0.39 / 1.71 / 4.9 |
| `hdlm_axis_bfm` | 0.06 / 0.20 / 0.5 | 0.07 / 0.20 / 0.4 | 0.07 / 0.20 / 0.4 | 0.11 / 0.39 / 0.9 | 0.12 / 0.39 / 0.9 | 0.12 / 0.39 / 0.9 |
| `uvvm_methods_log` | 0.02 / 0.02 / 0.1 | 0.02 / 0.02 / 0.1 | 0.02 / 0.02 / 0.1 | 0.04 / 0.13 / 0.3 | 0.05 / 0.13 / 0.3 | 0.05 / 0.13 / 0.3 |
| `poc_cache_cpu` | 1.76 / 8.63 / 26.6 | 1.80 / 8.83 / 26.9 | 1.77 / 8.66 / 26.9 | 1.81 / 8.86 / 27.1 | 1.85 / 9.04 / 27.4 | 1.82 / 8.91 / 27.4 |
| `sc5_fifo_repack` | 3.75 / 18.72 / 64.1 | 3.67 / 18.25 / 62.1 | 3.62 / 17.97 / 62.1 | 3.71 / 18.40 / 64.3 | 3.88 / 19.11 / 64.4 | 3.75 / 18.58 / 64.1 |
| `fletcher_bufferreader` | 4.14 / 20.57 / 55.5 | 4.14 / 20.54 / 55.9 | 4.11 / 20.34 / 55.9 | 4.13 / 20.52 / 55.4 | 4.11 / 20.35 / 55.8 | 4.10 / 20.31 / 55.8 |
| `rtl_ff` | 0.69 / 3.33 / 11.9 | 0.73 / 3.52 / 12.3 | 0.73 / 3.53 / 12.3 | 0.68 / 3.32 / 11.9 | 0.74 / 3.53 / 12.3 | 0.74 / 3.56 / 12.3 |
| `rtl_lfsr` | 0.39 / 1.86 / 6.6 | 0.34 / 1.60 / 5.8 | 0.33 / 1.59 / 5.8 | 0.39 / 1.88 / 6.7 | 0.35 / 1.69 / 6.0 | 0.36 / 1.70 / 5.9 |
| `rtl_chain` | 0.20 / 0.92 / 3.2 | 0.20 / 0.91 / 3.3 | 0.19 / 0.91 / 3.3 | 0.20 / 0.93 / 3.2 | 0.20 / 0.92 / 3.3 | 0.20 / 0.92 / 3.3 |
| `rtl_fan` | 0.92 / 4.52 / 14.1 | 0.96 / 4.70 / 15.0 | 0.96 / 4.71 / 15.0 | 0.94 / 4.60 / 14.2 | 0.95 / 4.72 / 15.1 | 0.96 / 4.73 / 15.1 |
| `rtl_bus` | 0.21 / 0.98 / 3.8 | 0.21 / 0.99 / 3.8 | 0.21 / 1.00 / 3.8 | 0.21 / 1.00 / 3.8 | 0.21 / 1.00 / 3.8 | 0.21 / 1.01 / 3.8 |
| `rtl_ram` | 0.09 / 0.34 / 0.6 | 0.09 / 0.34 / 0.6 | 0.09 / 0.34 / 0.6 | 0.10 / 0.35 / 0.6 | 0.10 / 0.35 / 0.7 | 0.10 / 0.35 / 0.7 |
| `lfsr64_xor` | 0.08 / 0.36 / 1.5 | 0.09 / 0.38 / 1.5 | 0.09 / 0.38 / 1.5 | 0.09 / 0.37 / 1.5 | 0.09 / 0.38 / 1.5 | 0.09 / 0.38 / 1.5 |
| `lfsr64` | 0.18 / 0.84 / 3.4 | 0.19 / 0.88 / 3.4 | 0.19 / 0.89 / 3.4 | 0.18 / 0.87 / 3.4 | 0.20 / 0.92 / 3.5 | 0.20 / 0.94 / 3.6 |
| `compute_h2h` | 1.45 / 7.25 / 27.3 | 0.16 / 0.76 / 2.8 | 0.16 / 0.77 / 2.8 | 1.45 / 7.29 / 27.3 | 0.42 / 2.03 / 7.5 | 0.42 / 2.03 / 7.5 |
| `protected_queue` | 1.38 / 6.90 / 28.8 | 0.53 / 2.59 / 9.5 | 0.51 / 2.51 / 9.0 | 1.38 / 6.87 / 28.8 | 1.21 / 6.05 / 24.0 | 0.94 / 4.67 / 17.8 |

Against the reference simulator (`tools/realbench`, `--tiers interp,auto,auto-cold`, run phase, cores 13 and 14, median
of the runs it chooses; wall seconds, ratio to the reference in brackets):

| test | reference | interp | auto | auto-cold |
|---|---:|---:|---:|---:|
| `vai_window_gen_interlock` | 3.89 | 29.83 (7.67) | 15.78 (4.06) | 18.13 (4.66) |
| `poc_stat_histogram` | 0.32 | 1.93 (6.03) | 0.74 (2.31) | 1.07 (3.34) |
| `hdlm_lfsr` | 1.06 | 3.74 (3.53) | 2.15 (2.03) | 2.36 (2.23) |
| `sc5_fifo_repack` | 0.84 | 3.79 (4.51) | 3.65 (4.35) | 3.81 (4.54) |
| `fletcher_bufferreader` | 1.06 | 4.13 (3.90) | 4.13 (3.90) | 4.13 (3.90) |
| `neorv32_single_min` | 3.33 | 8.86 (2.66) | 8.84 (2.65) | 9.19 (2.76) |
| `poc_cache_cpu` | 0.78 | 1.74 (2.23) | 1.77 (2.27) | 1.83 (2.35) |
| `uvvm_axistream_vvc` | 3.51 | 3.75 (1.07) | 3.77 (1.07) | 3.90 (1.11) |
| `uvvm_sb_slv` | 0.91 | 0.82 (0.90) | 0.85 (0.93) | 0.96 (1.05) |
| `uvvm_util_rand` | 1.10 | 0.90 (0.82) | 0.89 (0.81) | 1.05 (0.95) |
| `osvvm_axi4_random` | 1.14 | 0.59 (0.52) | 0.59 (0.52) | 0.73 (0.64) |
| `osvvm_axis_random` | 0.86 | 0.28 (0.33) | 0.28 (0.33) | 0.40 (0.47) |
| `uvvm_func_cov` | 0.76 | 0.19 (0.25) | 0.19 (0.25) | 0.31 (0.41) |

The cold column of the short tests is the cost of an empty compilation cache for every tier alike (analysed units and
call results are rebuilt: `uvvm_func_cov` 0.19 s warm, 0.31 s cold under the interpreter too). Geometric mean of the wall
ratios over the 13: interpreter 1.63, auto 1.38, auto-cold 1.62.

What the measurements show and what is left:

- `protected_queue` (the queue push and pop of #493) cold 1.21 s to 0.94 s, the window generator cold 19.2 s to 17.4 s
  (-9 percent main-thread instructions); everything else is level with main.
  Measured again on the pushed commit against its parent (main at 566a6e08, same method, 5 runs): window generator warm
  15.8 s to 15.4 s, cold 19.1 s to 17.1 s (main-thread instructions 336.5 G to 285.0 G); `protected_queue` warm 0.54 s to
  0.51 s, cold 1.29 s to 0.93 s.
- `--tier=auto` is not within noise of the interpreter on `rtl_fan` (+5 percent cycles, nothing compiled: #544),
  `rtl_ff` (+5 percent, +3 percent of it the compiled process: #575), `lfsr64` (+3 percent) and `uvvm_sb_slv` (+5 to
  +12 percent, four compiled string helpers: #573), warm and cold alike; the same on main.
- The closure of a native-callable function compiles callees nobody has seen (a fifth of the compile time on the window
  generator) and the function waits for them (`pop`: 2.5 s to 7.4 s from candidate to compiled); a never-seen callee at
  `Fast` would cost a seventh. Not built: the first level of a function is the same code as the last, so a cheaper
  first level would need an upgrade the sampler cannot trigger (decisions.md).
- Not measured: more than one compile thread; the tiny-test corpus (`tools/bench/tiny.py`), where nothing compiles and
  the policy has no effect.

## Slice positions, memoised descriptors, static waits over slices (#520, 2026-10-22)

Three changes in the interpreter's cost of RTL processes with generic widths (`docs/decisions.md` "Slice positions,
memoised descriptors and static waits over slices"):

- `Inst::SlicePos` is the start of a slice (null slice: 0 and no check) in one instruction instead of a branch and three
  (`yavs-lower` `slice_of`; folded in the optimiser, an op of its own in the interpreter, inline in the LLVM tier).
- The memoisation of instance constants (`opt/memo.rs`) keeps a descriptor built from instance constants in frame bytes,
  sees through an `if` on an instance-constant condition, memoises `LayoutOf`, and pays off sooner (half the roots plus 3
  dropped instructions, 40 roots).
- A wait over slices with generic bounds is a static wait (`pre::static_wait`).
- Found on the way: the `CopyDrive` fusion read the size register at the copy, so a register reused between the `Alloc` of a
  waveform and its drive gave a waveform of the wrong size (`pre.rs` `peephole`); the new fast paths of the memoisation
  exposed it on the window generator testbench. Test: `memoised_array_copies_drive_their_own_sizes`.

Core 15, `tools/realbench`, warm, `perf stat` instructions (G), before / after: `poc_arith_addw` interpreter 494.8 / 408.9
(`--tier=auto` 462.6 / 394.3), wall of the interpreter 27.2 s / 24.3 s (the host was noisier in the second run; the
`--compare` of all 41 tests on cores 15 and 8 has 27.4 s / 23.1 s); against the reference simulator (`--tier=auto`, total
wall) 5.33 to 4.03 warm, 5.33 to 4.05 cold. The window generator testbench and the histogram are unchanged in instructions
(504.97 G, 40.9 G): their time is the queue's protected-method chain (#493, #444) and element loops. Whole suite (41 tests,
`--compare`): geometric mean of wall 0.99 interpreter, 0.97 `--tier=auto`; no test slower beyond noise.
Checked: `cargo test --workspace`, the differential corpus (447/447) on `yavs-interp` at 1 and 4 threads, `yavs-llvm`
and `yavs-auto`.

Where `poc_arith_addw` stands: per activation of a combinational process 120 operations before, about 70 now; what is
left is the work (reads of the signals into buffers, concatenation, the native addition, the drive) and the kernel
between activations (about 150 ns, #523), plus the message built for every assertion (a third of the interpreter time:
the message is an actual parameter, so it stays eager, and the `to_integer` warnings keep it from sharing).

Ideas of the reference simulator used (read for the design, no code): it folds the generics of an instance into its
code at elaboration; ours computes the same facts once per instance in the shared code.


## llvm: host callbacks of compiled code, round 1 (#565, #532, #339, #337, 2026-10-23)

What each exit of compiled code cost and what was changed (`docs/decisions.md` "Host callbacks of compiled code"):
natives of frame-mode code run through `TierHost::native_frame` (argument cells from the registers, one result word,
nothing flushed or reloaded around the call), `Cat` of non-logic elements and `LayoutOf` of one-dimensional arrays are
lowered in frame mode, `Mem::forget_since` returns at once when it has nothing to drop, the optimiser drops the
`TempRoot` of a fat pair of temporaries, and a `SigRead` of a logic vector is inline whatever its length (it was a
callback, 220 host instructions, for every length that is no multiple of eight; a generic width calls a pure helper).

The calls counted on `poc_arith_addw` (`--tier=llvm`, whole run), before / after: fat roots 117 M / 20 M, natives through
the generic step 172 M / 0 (through the lean call), layout helpers 76 M / 0, `TierHost::signal` 105 M / 89 M (vector
`SigRead` of generic widths 64 M, logic `Cat` of generic widths 22 M; both are lengths that are instance constants the
generator does not know).

`poc_arith_addw`, the first 600 us of simulated time (of 2.6 ms), `--tier=llvm` with `YAVS_LLVM_FAST_MIN=99999999` (the
code `--tier=auto` makes at `O2`), core 2, `perf stat` cycles / instructions in G, median of 5 interleaved runs after an
untimed run that fills the object cache, each row on top of the one above (the interpreter: 25.9 / 93.7):

| step | cycles | instructions |
|---|---:|---:|
| main at 9be61682 | 27.4 | 93.1 |
| natives through `native_frame` | 24.8 | 88.7 |
| `Cat` of non-logic elements inline | 23.9 | 83.6 |
| `LayoutOf` of one dimension inline | 23.2 | 81.9 |
| lean `native_frame`, `forget_since` skip | 21.8 | 74.9 |
| clean fat roots dropped (IR) | 21.3 | 73.3 |
| `SigRead` of any length inline | 20.1 | 69.9 |

The whole test, default `--tier=llvm` (the module over 150000 LLVM instructions at the unoptimised code generator),
core 2, median of 3: 152.2 G cycles, 31.5 s before; 124.0 G, 25.7 s now (-19 percent cycles, -20 percent instructions);
the interpreter 111.5 G, 23.4 s before and 113.7 G, 23.9 s now (instructions 409 G to 401 G). Cold compile of the whole program (`--no-cache`, init phase) 2.71 s to 2.81 s, the generated LLVM IR is 7 percent
larger.

Other tests, `--tier=llvm` at `O2` code generation, cycles G before / after the first commit (3 runs, cores 2 and 3):
`fletcher_bufferreader` 43.6 / 39.0, `sc5_fifo_repack` 26.2 / 23.1, `poc_cache_cpu` 15.2 / 14.3, `neorv32_single_min` 77.9 / 75.8
(and 64.8 / 63.5 with the `SigRead` change), `poc_stat_histogram`, `hdlm_lfsr`, `sc5_timestamp` within noise. The
interpreter itself gains 1 percent in instructions from the dropped roots; a quarter of its time on `poc_arith_addw` is
still natives (`ieee::small`, `ret_string`, `fat_dim`) and the kernel.

What is left of the gap to the interpreter, by measurement: on clocked RTL (`fletcher_bufferwriter_512`, `neorv32_single_min`)
instructions are within 5 percent of the interpreter's but cycles are 40 to 60 percent higher (IPC 1.8 against 2.4: code
size and cache misses, #532); the exits left there are `LayoutOf` of records with descriptors (27 M in
`fletcher_bufferwriter_512`, each statement lays a process variable's descriptor out again, #518), `rising_edge` of a
clock the process was not resumed for (7 M in `neorv32_single_min`), `WaitOn` of waits on several signals, `Drive` of
more than 64 elements.

What is left in `poc_arith_addw` (89 M `signal` calls: vector `SigRead` and `Cat` of generic widths, whose lengths are
instance constants the generator does not know) and the shape of the rest of the gap to the interpreter on clocked RTL
(code size and cache misses, not exits) are in the issue comments of #565 and #532.

## Interpreter round 14: entry of the dispatch loop, memoised signal references, address and real-compare ops (#520, 2026-10-23)

Three commits (`docs/decisions.md` "Interpreter round 14"):

- Package data and the rare slot paths are out of the inline memory paths of `run_ops`: 17 to 11 address pairs that the
  compiler hoists out of the dispatch loop and stores on every entry (an activation of a small process enters it once;
  about 105 instructions and 60 stores before the first op, 18 to 25 percent of `run_ops` on the realistic designs).
- The memoisation of instance constants (`opt/memo.rs`) covers `SlicePos` and `SigVal` of instance-constant bounds (a signal
  reference is a root of two words, loaded by `LoadSlot8x2`); references of constant length and references read as
  scalars stay as they are (fusions of the interpreter, inline code of the compiled tier).
- `AddrOf` (frame, up the static chain, fixed spaces) and `FCmp` are ops of the fast loop instead of generic steps.

Core 15 (core 8 within 1 percent of these ratios), `--tier=interp`, `tools/bench/designs` and `tools/realbench` (windows of
the long tests: 100 to 300 us of simulated time), `perf stat` user instructions and cycles in G, min and median of 7
interleaved runs, main at f4f1512e against the three commits; the cycles ratio is the median:

| design | instructions | cycles | ratio |
|---|---|---|---:|
| `poc_arith_addw` | 45.94 / 43.38 | 12.91 / 11.86 | 0.919 |
| `sc5_fifo_repack` | 64.14 / 60.41 | 19.17 / 17.73 | 0.925 |
| `neorv32_single_min` | 97.20 / 94.68 | 43.43 / 42.10 | 0.969 |
| `fletcher_bufferwriter_512` | 46.92 / 46.31 | 19.11 / 18.69 | 0.978 |
| `fletcher_bufferreader` | 8.76 / 8.61 | 3.27 / 3.16 | 0.967 |
| `poc_cache_cpu` | 26.61 / 25.83 | 8.64 / 8.44 | 0.977 |
| `poc_stat_histogram` | 40.84 / 40.15 | 9.84 / 9.80 | 0.996 |
| `hdlm_lfsr` | 1.63 / 1.61 | 0.50 / 0.49 | 0.996 |
| `rtl_fan` (4000 near-idle processes) | 14.14 / 13.72 | 4.67 / 4.54 | 0.973 |
| `rtl_ff` | 11.87 / 11.65 | 3.63 / 3.40 | 0.937 |
| `rtl_lfsr`, `lfsr64`, `lfsr64_xor`, `rtl_chain` | -1.3 to -2.1 % | | 0.97 to 0.99 |
| `ieee_heavy` | 1.84 / 1.80 | 0.42 / 0.39 | 0.923 |
| `protected_queue`, `protected_scoreboard`, `report_heavy`, `strings_heavy`, `rtl_bus`, `rtl_ram` | -0.6 to -1.6 % | | 0.97 to 1.00 |
| `access_heavy` (40 ms) | 0.77 / 0.76 | 0.19 / 0.20 | 1.023 |

Geometric mean over the 22 designs: instructions 0.980, cycles 0.9735 (core 15) and 0.972 (core 8). `--tier=llvm` and
`--tier=auto` run `poc_arith_addw` 2 percent fewer cycles, nothing slower. Checked: `cargo test --workspace`, the
differential corpus (451/451) on `yavs-interp`, `yavs-auto` and `yavs-llvm` at 1 and 4 threads.

What the round learned about the loop (all in `docs/decisions.md`): the entry cost is the compiler hoisting addresses out
of the loop, not the work of the entry; a copy of the loop for functions of simple ops (const-generic guards, 63 arms
out, entry 105 to 70 instructions) gains 5 percent on near-idle processes and nothing on a realistic design; outlining
more (memory access, vector growth, the frame push) costs more calls than it saves; and the cycles of one design move by
up to 20 percent with the alignment of the same code (#576), so only the geometric mean over many designs and two cores
means anything. Ranking afterwards (mean share of cycles over the 22 designs): `run_ops` 50 percent (its entry about a
fifth of that on the small-process designs), the kernel's `finish_direct`, `run_batch`, `cycle`, `commit_out` and the
resolver 9 percent together, `run_quick` and `Process::run` 3 percent (51 and 22 instructions per activation of a
near-idle process), `cat_words`/`cat_block` 1.5, the `numeric_std` natives 2.3, `lcopy` 0.8, `subtype_check` 0.7.

Idea of the reference simulator used (read for the design, no code): it folds the generics of an instance into its code at
elaboration, so the slice bounds and signal references of an instance are constants; the memoisation computes the same
facts once per instance in the shared code, and now also for the references. Its interpreter keeps its hot state in a small
struct on the stack and the registers in a stack array, so a call stores a handful of words; yavs's machine is addressed
field by field, which is what the entry cost above comes from.

## Work-counter perf tests and the op census (#579, 2026-10-23)

`crates/yavs/tests/perf.rs`: 22 designs plus the LLVM ones (3, `--features llvm`) check counters of
work that are exactly repeatable (kernel `Stats`, interpreter `ops`, call-cache and unit-cache hits,
wave variable reads, compiled-code callbacks) against ceilings/floors; see root `AGENTS.md` section 4.
`op-census` (cargo feature) prints the op and adjacent-pair tables of a run (`crates/yavs-interp/AGENTS.md`
section 5). Sensitivity checked by turning optimisations off (`YAVS_NO_MEMO`, `YAVS_NO_IROPT`,
`YAVS_NO_INLINE`, `YAVS_NO_NATIVE_IEEE`) and by `--cov` (skipping and idle skip off): the matching designs fail
with the counter named.

## Sampling profiler, `yavs run --profile[=FILE]` (#580, 2026-10-23)

`--profile` prints the busiest processes (instance path, file:line), instances, subprograms and interpreted source lines to stderr
and writes all rows as JSON with `=FILE` (`docs/cli-contract.md` 5, `docs/decisions.md` "Sampling profiler"). Tests:
`crates/yavs/tests/profile.rs` (hot process and the loop line named, stdout unchanged at `--tier=interp`, default and `--threads=2`;
E0124; `--dap` is E0125; Ctrl-C prints the profile so far, #587), and `--profile` in the VHPI variables test (the wrapper keeps `as_any`). Overhead, `tools/bench` h2h designs, core 14,
7 runs, bench builds with the `llvm` feature (A = main before, B = this change):

- Off (B/A total CPU, geometric mean of the 11 designs): 0.992; every design 0.90 to 1.04 (`rtl_chain` 1.04: its instruction count
  is the same, cycles move with code layout, #574). Realbench `run` phase (core 15, 5 runs, `--tier=interp` and `auto`; the first 3 tests of a planned 8, the run was stopped for time): `hdlm_fifo_sync` and `olo_base_cam` 1.0, `poc_arith_addw` (23 s) 0.94 to 1.0, all within noise.
- On (B with `--profile` / B without, geometric mean of total CPU, two rounds): 1.03 with `--tier=interp`, 1.015 with
  `--tier=auto`; short designs show the fixed cost of the report (a few ms: resolving lines reads the source files).

## Null waveform elements in sequential assignments (#352, 2026-10-23)

`s <= v after a, null after b, ...` on a `bus` or `register` signal lowers to one transaction per
element (`yavs-lower` `drive_nulls`): the first with the assignment's delay mechanism, the rest
transport, a null as `Inst::Disconnect` at its own time. No IR, interpreter or kernel change.
Corpus `sigassign/null_waveform` (reference output) and `sigassign/null_waveform_mixed`
(`ref=hand`: sim1 loses a value before a null and a null in the middle of a waveform, sim2 cannot
run a null among elements; `tests/discrepancies/null-waveform-in-waveform`). Still `E2001`: null in
record signals (blocked by #595), aggregate targets and signal parameters.

## GUARD initial value (#464, 2026-10-23)

The implicit GUARD signal starts as the guard condition on the initial values (`guard_signal` in
`yavs-elab`), so a process reading it at time 0 sees the condition, not `false`. Corpus
`delta/guard_initial_value`. Open (#464): GUARD still changes a delta after the condition's
signals, where the references update it in the same cycle (needs a kernel update-phase
evaluation of the condition).





## Guarded record signals (#595, 2026-10-23)

A `bus` or `register` record signal whose subtype has a record resolution function elaborates
(`E3010 guarded signal p.a must be resolved` came from guarding the leaves before the
composite groups joined them) and simulates: the kernel resolves a guarded group over its
connected sources (`SimControl::resolve_group`), a false GUARD (`guarded_assign`) and a `null`
waveform element (`drive_record`) disconnect every leaf. Corpus `resolution/guarded_record`
(`ref=hand`, sim1 does not disconnect at initialisation), `resolution/guarded_record_rec2` (the
reproducer), `sigassign/null_record`; kernel test `guarded_composite_disconnects_as_a_unit`;
discrepancy `null-record-signal`.

## llvm: the window generator against the reference, round 1 (#493, #520, 2026-10-23)

Decisions: docs/decisions.md "Compiled code of queue and pointer-table libraries". Where the time of `vai_window_gen_interlock`
(`--tier=auto`, warm) went, by counting: 164 million protected-method calls in 16 s (string pointer table `get` 70 M, `set`
40 M, `length`, `unsafe_push`, `unsafe_pop` 13 M each; about 290 host instructions and 130 cycles per `get`), 46 percent of the
main thread's instructions in compiled code, 22 percent in `run_ops`, and before this change 22 percent in object look-up
callbacks (95 M `Deref`, 38 M `yavs_rt_host`, 36 M `TierHost::object`). Changes: a table of 256 compiled-code objects (was 16),
callees never seen running left out of a compile, the fat-pair `TempRoot` tested in place.

Window generator, `tools/realbench` world, interleaved runs on core 1 (cores 0/1 are the only timing cores of this task and
the host was loaded: 9 to 37 load average, the reference simulator ran 7 to 8 s instead of 3.9 s, so read the instruction
and cycle columns and the ratios, not the seconds). Main-thread `perf stat --no-inherit`, median of 7, G; base is
origin/main at 65b7f573, now the three changes together:

| cell | base wall / cycles / instructions | now | ratio (cycles, instructions) |
|---|---|---|---|
| auto warm | 23.0 s / 107.9 / 245.8 | 20.7 s / 97.5 / 213.8 | 0.90, 0.87 |
| auto cold | 29.6 s / 135.4 / 280.4 | 23.5 s / 108.9 / 255.6 | 0.80, 0.91 |

The reference simulator on the same host and day: 72.8 G instructions (the run: `realbench --ref`, core 1, 5 runs), so the
instruction ratio of `--tier=auto` to the reference simulator is 3.06 warm (3.64 before) and 4.23 cold (4.71 before); wall
7.01 s reference, 26.05 s before and 24.71 s after warm, 26.30 s before and 27.22 s after cold in that loaded run (cold has a
spread of 15 percent between runs of one binary). Single changes, `--tier=auto` warm, 5 to 7 interleaved runs, instructions
(cycles are noisy: an SMT sibling of the timing core runs other agents' builds): table of 256 entries 245.7 to 211.6 G (-14
percent; 64 entries 242.5), callees left out 219.3 to 219.3 warm (cold 259.3 to 254.7 and cycles 100.6 to 91.1), fat roots
219.3 to 217.4.

Breadth (`tools/realbench --compare`, 18 tests, interp, auto warm and cold, 5 runs, cores 0 and 1): the geometric mean of the
instruction ratio is 1.00 for the interpreter, and for `--tier=auto` 0.97 for the two rtl-datapath tests (window generator
0.94) and 1.00 for the other categories; `uvvm_util_rand`, `uvvm_axistream_vvc` and `uvvm_sb_slv` showed ratios up to 1.7
in single comparisons but their main-thread instructions differ by up to 40 percent between runs of one binary (the
instruction count of a short `--tier=auto` run depends on when the compile thread finishes), and in repeated A/B pairs of
9 to 21 runs there is no difference beyond that spread. Counters: `cargo test -p yavs --features llvm --test perf` passes; the
fat-root callbacks of `llvm_sigread` went from 54 to 1 (bound lowered).

Open (not done), in the order of expected gain on this test: native-frame call overhead of the accessors (prologue of 35
instructions, registration of the frame's slots, spill stores at each call site; 65 of about 290 instructions of a `get`) and
the dependent chain of 8 object look-ups per `get` (each hash, entry load, compare, bounds); the frame-mode part of loop
functions compiled only when needed (upper bound under 2 percent of a cold run: 9.4 percent of its compile time);
`LogicOp`/`LFill` of numeric_std vectors stepped on native frames (9.5 M steps, about 3 percent); big clocked processes
(`window_hold_check`: 23 of 211 instructions leave the compiled code, `walk_control` 4373 IR instructions) stay interpreted.
Idea of the reference simulator used (read for the design, no code): an access value there is a plain pointer, a dereference
a null test and a load, and a protected object is a context pointer passed to an ordinary call; yavs's generational handles
(a design rule of docs/plan.md) make every dereference a table look-up, which the larger table makes a hit instead of a callback but does not
make a load.

## Interpreter round 15: element loops, the activation state in three lines, a poll for hung processes (#520, #583, #579, 2026-10-23)

Four commits (`docs/decisions.md` "Interpreter round 15", "The activation state of a process in three cache lines",
"A process that never suspends polls"):

- **Element loops**: loops that copy or fill the elements of logic vectors (conversions, row extraction, shifts,
  reversals) run all iterations but the last at once when every index is in range (`pre::elem_loops`,
  `Machine::op_loop`); the generic ops run the last one, or the whole loop when a check fails, so failures stay at the same
  iteration. `poc_stat_histogram` (95 percent of its time in two such loops): instructions 40.15 G to 25.12 G (0.626),
  cycles 0.63 to 0.73. Over the 24 designs of `tools/bench` and `tools/realbench` (cores 4 and 5, 5 runs after two untimed
  runs, main at 65b7f573 against the commit): geometric mean of instructions 0.981, cycles 0.985 (0.964 on core 5 and
  1.006 on core 4: the host was loaded; every other design within 0.5 percent of instructions, `poc_cache_cpu` +0.4
  and `fletcher_bufferreader` +0.8 for loops too short to run at once). Checked: `cargo test --workspace`, the differential
  corpus (451/451) on `yavs-interp` at 1 and 4 threads, `yavs-llvm` and `yavs-auto`, `gc-stress` unit tests of
  `yavs-interp` and `yavs-llvm`, 5500 generated programs of loop shapes (directions, strides, overlaps, indices out of
  range, 1 to 200 elements) against the build without the loops: identical output and errors.
- **The op census hook** (`census::tick`, #579) in the dispatch loop was an empty call in release builds that moved the
  loop's code generation: instructions of `calls` +8.8 percent, `compute_loop` +6.7, `protected_queue` +4.7, neorv32
  +1.4 against the commit before it. It is cfg'd out now: `calls` 0.920, `compute_loop` 0.937, `protected_queue` 0.955,
  `rtl_fan` 0.983, neorv32 0.987 of the instructions of the commit with it. Any change inside `run_ops` moves other designs
  by percents (a poll in the `Br` arm did the same), so measure instructions of five designs after every edit of it.
- **Three cache lines**: `Machine` ordered by use (`repr(C, align(64))`): `rtl_fan` L1 data misses 597 M to 516 M, cycles
  0.85 to 0.91; others unchanged.
- **#583**: a process that never suspends polls `Ctx::stop_requested` at its jumps back (stepped `Br`), outside the dispatch
  loop: the first Ctrl-C with `--wave` ends it. Free by construction (instructions identical on 6 designs).

All four commits together, `tools/bench` designs and `tools/realbench` windows, main at ed0caf0c against 5247726d, aligned
bench builds (`tools/build-bench.sh`), cores 4 and 5 on a quiet host, `perf stat`, median of 3 after two untimed runs:
geometric mean over the 24 designs instructions 0.952, cycles 1.014 (best of 3: 1.004; core 4 1.007, core 5 1.021).
Per design instructions: `poc_stat_histogram` 0.601 (cycles 0.67), `protected_queue` 0.951, `calls` 0.912, `compute_loop`
0.936, `sc5_fifo_repack` 0.966, the rest 0.97 to 1.00. Cycles do not follow the instructions of the census fix: `calls` and
`compute_loop` run 3 to 10 percent more cycles for 9 and 6 percent fewer instructions (the code layout of the large
function, #576), `rtl_fan` 0.85 on core 4 and 1.01 on core 5. The three cache lines are the one change whose evidence
is in miss counts, not cycles.

Entry cost of `run_ops` (the task of the round): the disassembly shows 80 instructions before the first op of which 24
are hoisted addresses, 8 the zeroing of the eight saved callers and 14 spilled parameters; in a design of 4000 near-idle
processes 245 of 686 instructions per activation are outside the loop (`InterpProcess::run`, `run_quick`), a third of
their cycles waiting for lines of the process. Tried and not kept: `k` in a register through a by-value closure (+1 to
+7.6 percent of instructions: the compiler lays the large function out differently, as in round 14). The ranking of
`Op` pairs from the census (`SigVal->SigVal`, `SigRead1->CmpBr`, `WaveBuf->LCopy->Drive`) is open: an arm added to
`run_ops` has to pay for itself against that layout effect.

## Interpreter round 16: superinstructions from the op census (#520, #518, 2026-10-24)

Details, the ranking and what was measured and not landed in `docs/decisions.md` "Interpreter round 16". One commit per
change, each measured by `perf stat` user instructions and cycles on 32 designs of `tools/bench` and `tools/realbench`
(45 for the last two; aligned bench builds, one core of the quiet CCD, per-binary copies of the worlds, two untimed runs,
median of 5 to 7 interleaved runs). Together `LoadUp`, `Uniform`, the sunk messages and `LCopyDrive` against main at
cf32383b: instructions 0.982, cycles 0.983 (32 designs); `poc_arith_addw` 0.720 and 0.763, `sc5_fifo_repack` 0.852 and
0.856, `neorv32_single_min` 0.947 and 0.988. `LEq` 0.997 and 1.010 (the cycles of designs with identical instructions
move by up to 10 percent, #576), the buffer drop 0.9935 and 0.9974. Full-length tests under `--tier=auto` against sim1 (host
load average 38 while measuring, so the ratios are inflated): `poc_arith_addw` 3.1x to 2.4x, `sc5_fifo_repack` 2.7x to 2.0x,
`neorv32_single_min` 1.5x to 1.4x, `fletcher_bufferwriter_512` 4.1x to 3.9x, `fletcher_bufferreader` 2.5x to 2.4x.

- `LoadUp`/`StoreUp` (interpreter only): a variable of an enclosing subprogram is read or written in one op instead of
  `AddrUp` and `Load`/`Store`. `sc5_fifo_repack` instructions 0.969.
- `Uniform` (interpreter only): `math_real.uniform` of three variables steps the generator in place, the native call
  only for a seed out of range. `sc5_fifo_repack` instructions 0.890 of main with `LoadUp` (0.919 of the commit before);
  the other designs within 0.2 percent. Lesson of the round: a new arm with a body of its own in `run_ops` moves the
  other designs by percents of instructions; an arm that calls an out-of-line function does not.
- A check's message is built only for a failing call (IR, `opt/sink.rs`, `opt/inline.rs`): a call that hands over a string it
  built may inline a callee of up to four times the raw size, and the instructions that only build the message move into
  the branch that reads it. `poc_arith_addw` instructions 25.57 G to 18.41 G (0.720), cycles 0.76; the other designs
  within 0.2 percent of instructions. What can be seen of a message (a failure, a report, `to_integer` warning of a vector
  with `U`) is still done at every call; the one thing skipped that could fail is the length of a string of 2^31 elements.
- `LCopyDrive` (interpreter only): a constant, or anything else copied element by element into a waveform, goes to the
  kernel without the waveform. Instructions of `neorv32_single_min` 0.946, `fletcher_bufferreader` 0.977, `sc5_fifo_repack`
  0.956, `fletcher_bufferwriter_512` 0.995; the others within 0.2 percent.
- `LEq` (interpreter only): `=` of logic vectors compares the planes in place instead of making both vectors:
  `osvvm_axis_random` 0.949, `fletcher_bufferwriter_512` 0.965, `neorv32_single_min` 0.982 of the instructions.
- The waveform buffer of a drive is dropped across calls (`Code::no_tier`): compiled code is not entered between the
  dropped buffer and the op that drives it. `olo_axi_master_full` 0.865, `neorv32_single_min` 0.974, `poc_cache_cpu`
  0.981, geometric mean over 45 designs 0.9935.

## Interpreter round 21: lean calls under a tier (#750, 2026-10-26)

Details in `docs/decisions.md` "The loop with the tier calls untouched callees lean". The loop with the tier (`run_ops::<true, false>`)
makes a lean call and skips the prologue for a callee whose `Pre::native` byte is clear, and `run_quick`'s watching arm reads
`NativeFlags::any` once. `uvvm_sb_slv` with `check_value` forced: 12.44 G to about 11.8 G instructions; `--tier=auto` with nothing
compiled over `--tier=interp` 1.0070 to 1.0066 (geometric mean, eight designs). Left: the per-activation slot protocol (about 22
instructions) and the tick of a jump back, 0.3 to 1.1 percent.

## Interpreter round 20: the watching loop (#750, #544, 2026-10-25)

Details in `docs/decisions.md` "Interpreter round 20". The cost of a `--tier=auto` run that compiles nothing sat in the loop for
uncompiled functions while a tier is set (`run_ops::<false, true>`) and in `run_quick`'s set-up for it, not in the loop with the tier.
The look at a callee reads `Pre::native` (a byte mirrored from `NativeFlags`, set by `Runtime::sync_native`) instead of the flag table,
the fused loop ops run 128 iterations per dispatch there, and `exec_fast` clears the sampling slot with a guard. Main-thread
instructions of `--tier=auto` with nothing compiled over `--tier=interp`, seven designs, geometric mean 1.0137 to 1.0054 (from 1.0011
to 1.0261 per design before, 0.9998 to 1.0102 after); `--tier=interp` unchanged within 0.1 percent. Left: the per-activation set-up
(about 20 instructions) and the tick of a jump back.

## Interpreter round 19: statement markers by a forward dataflow (#520, 2026-10-25)

Details in `docs/decisions.md` "Interpreter round 19". `Stmt` markers stay only where a path reaches them through an instruction
that leaves temporary memory (`opt::leaves_temps`, callee flag `FuncFlags::NO_TEMPS`, set by `propagate`); waits and drive
statements keep theirs. Instructions of 25 bench designs 0.990 (`calls` 0.833, `textio_heavy` 0.980, `file_io` 0.983);
`hdlm_lfsr` 0.974, `neorv32_single_min` 0.979, `sc5_fifo_repack` 0.979. Tier-active path (+1.6 percent in an equal-work window,
per-op differences of 2 to 10 percent) measured, not fixed. Left: `FieldOf` chains in `grlib_leon3mp`, loop-invariant signal
reads of `sc5_fifo_repack`; `cat_words` is valmove's.

## Interpreter round 18: exact per-op counts, the usual case first (#520, #518, 2026-10-25)

Details in `docs/decisions.md` "Interpreter round 18". `tools/stepcount --ops` counts the instructions of each interpreter op
(callees included) and `--ops-detail OP` splits one op by function. With it: `Uniform` (one frame lookup), `SubtypeCheck`
(out of line, one-dimension fast path), the `u64` length of a dimension, `SlicePos` (one look at the descriptor), `LSet`,
`LCopy` (up to 64 elements in one block), `AllocLCopy` and `LGet` take their usual case before the general code. Instructions
against main: `sc5_fifo_repack` 0.950, `grlib_leon3mp` 0.956, `fletcher_bufferwriter_512` 0.983, geometric mean over 8 designs
0.983, cycles 0.991; `tools/bench` internal CPU 0.976; no design above 1.00 in instructions. `CallIeee` out of line was tried
and lost (+0.4 percent). Left: the kernel side of `Suspend`/`WaitSuspend`/`Drive` (kern22), `cat_words` for `lfsr64`, the
`FieldOf` chains of `grlib_leon3mp`.

## Interpreter round 17: loads from the other address spaces (#674, #518, 2026-10-24)

Details and the measurements in `docs/decisions.md` "Interpreter round 17". `op-census` has a table of `Machine::load_slow`
calls by running op and address space. `load_slow` (loads from instance data, package data, constants and shared variables
through a register that holds the address only at run time: descriptors of vector parameters) is a short match instead of a
30-instruction body: `uvvm_func_cov` instructions 0.973, `poc_cache_cpu` 0.994. `layout_word` answers from the memo itself:
`fletcher_bufferwriter_512` 0.982, `fletcher_bufferreader` 0.980 of the instructions. Over 41 designs of `tools/realbench`
(interpreter, one core of the quiet CCD, host load 25 to 30): wall geometric mean 0.98, instructions 1.00 (none above 1.01
but the short `fletcher_bufferwriter_prim32`, whose front-end work moves by 1 to 3 percent between worlds), CPU time of
`tools/bench` 0.979 (internal) and 0.997 (h2h). Left: `AddrUp -> Uniform`, the fat-pair calling convention of the vector
natives (`mux`), the repeated `to_integer` of the `poc_arith_addw` checks.

## Cold analysis: table writes are tracked, not hashed (#603, 2026-10-23)

`frame_end` hashed every table of `Db` (33 maps, sets and values) before and after each unit analysed from source, to find what the
unit changed: about a fifth of a cold analysis, growing with the closure. The maps and sets a unit may change are now `TrackedMap` and
`TrackedSet` (`crates/yavs-sem/src/tracked.rs`): a write appends the key and the hash of the value it replaced to a log, a unit's own
changes are the log since its mark, and a finished unit hands the keys it and its dependencies changed to the unit around it, so a
change is hashed once instead of once per level of loads. There is no `DerefMut`, so a write that bypasses the log does not compile.
The few value tables are still compared. Safety net: debug builds (all of `cargo test`) and `YAVS_UNIT_CHECK=1` also compare every
table as before and fail with an internal error naming the table when the log missed a write (checked by breaking the log on purpose).
Decision and reasoning: `docs/decisions.md` "Table writes are tracked" (replaces "checked, not audited").

Checks: `YAVS_UNIT_CHECK=1 yavs analyze` cold and warm over osvvm, uvvm (2008, 2019), vunit (2008, 2019), hdl-modules (2008, 2019),
open-logic, neorv32, poc, surf, satcat5, vhdl-extras, en_cl_fix, fletcher, json-for-vhdl: 34 runs, all as expected. The units cache entries
of those 17 selections are byte for byte the same as before this change apart from the build hash in the header (same number, same
content), so the stored units and the replay are unchanged.

Before / after, `tools/realbench/run.py --compare`, `--phases analyze`, cores 6 and 7, 3 runs, 10 tests (one per selection that has a test),
host load 20 to 26 from other agents: the instruction ratio is exact, the wall times are not (the short tests move by 10 to 15 percent
with load in either direction). The sim1 columns are two separate runs with `--ref sim1`, cold analysis one file per call, yavs / sim1
(below 1 is faster).

| selection (test) | cold wall s before / after | cold instructions after / before | yavs / sim1 wall | yavs / sim1 instructions |
|---|---:|---:|---:|---:|
| hdl-modules (hdlm_fifo_sync) | 2.50 / 2.26 | 0.83 | 0.40 / 0.45 | 0.69 / 0.57 |
| open-logic (olo_base_cam) | 5.42 / 4.29 | 0.79 | 0.55 / 0.48 | 0.74 / 0.58 |
| vunit (vunit_logging) | 2.32 / 1.92 | 0.81 | 0.35 / 0.48 | 0.75 / 0.61 |
| en_cl_fix (en_cl_fix_pkg) | 2.47 / 1.40 | 0.83 | 0.53 / 0.44 | 0.70 / 0.58 |
| poc (poc_arith_addw) | 1.35 / 1.17 | 0.86 | 0.45 / 0.44 | 0.65 / 0.55 |
| neorv32 (neorv32_single_min) | 0.50 / 0.58 | 0.87 | 0.44 / 0.41 | 0.62 / 0.54 |
| osvvm (osvvm_axis_random) | 10.38 / 7.37 | 0.77 | 0.74 / 0.44 | 0.79 / 0.62 |
| uvvm (uvvm_sb_slv) | 0.89 / 1.01 | 0.85 | 0.46 / 0.43 | 0.58 / 0.49 |
| satcat5 (sc5_timestamp) | 2.03 / 1.80 | 0.89 | 0.49 / 0.50 | 0.75 / 0.64 |
| fletcher (fletcher_bufferreader) | 0.71 / 0.74 | 0.86 | 0.42 / 0.41 | 0.64 / 0.55 |

Warm analysis (files already in the library) is 0.94 to 1.01 in instructions: it replays little and rarely records. Selections without a
realbench test, cold analysis of the whole corpus manifest one file per call (`perf stat`, core 7, instructions): surf 44.1 G to 42.1 G,
vhdl-extras 1.50 G to 1.22 G, json-for-vhdl 11.3 G to 9.3 G; osvvm (core 6) 16.1 G to 13.0 G. The wall columns are also noisy because
sim1's own time moved by up to 1.5x between the two runs; use the instruction columns.

## llvm: compiled clocked RTL, round 4, 2026-10-23 (#532, #339)

The layout slots are made under `--tier=auto` too (2026-10-23, user decision; measurements and cost in `docs/decisions.md` "The layout cache under `--tier=auto`": invisible by default, 42 MiB more peak RSS on `olo_axi_master_full` when everything is compiled at once, 10% fewer instructions on `fletcher_bufferwriter_512`).

Exits and layout of compiled clocked RTL (decisions.md "Compiled clocked RTL processes, round 4"). What compiled code
sends to the host (`YAVS_COUNTERS=kinds`, formerly `YAVS_TIER_CENSUS=1`, `--tier=llvm`, whole run; the "before" counts are the last snapshot of the
same census at 500 000-call steps, so they are lower bounds):

| Design | calls before | calls now | what was left |
|---|---:|---:|---|
| `neorv32_single_min` | 18.7 M and more (7.0 M `rising_edge`, 3.7 M `LEq`, 2.9 M `WaitOn`, 2.4 M and 2.0 M signal reads, 1.1 M and 20.5 M `Drive`) | 1.0 M | `Cat`, `TempRoot`/`Stmt` pairs of strings, `**` |
| `fletcher_bufferwriter_512` | 68 M (31.9 M `LayoutOf`, 8.5 M `WaitOn`, 8.4 M `TempRoot`, 5.9 M `Stmt`, 4.5 M and 4.0 M signal reads and drives) | 7.3 M | wide `Drive`s through `drive_wide` (3.6 M), `LFill`, `LCopy` over 64 elements |

Cost of the compiled run against the interpreter, `perf stat` of the simulation phase (the same run to time 0
subtracted), core 3, bench builds (`tools/build-bench.sh`), median of 5 interleaved, host load average 20 to 30 so the
spread of cycles is 2 to 37 percent: cycles (G) / instructions (G).

| Design, tier | main before | with the round | interp (both) |
|---|---:|---:|---:|
| `neorv32_single_min` `--tier=llvm`, optimised code generator | 74.2 / 81.8 | 66.0 / 73.6 | 59.5 / 94.4 |
| same, default `--tier=llvm` (unoptimised code generator over 150000 instructions) | 85.4 / 90.8 | 79.3 / 83.5 | |
| same, `--tier=auto --tier-heat=1,0,0` (every executed function compiled) | 80.5 / 87.3 | 71.4 / 79.7 | |
| `fletcher_bufferwriter_512` `--tier=llvm`, optimised | 102.9 / 123.2 | 92.2 / 106.9 | 80.4 / 128.8 |
| same, `--tier=auto --tier-heat=1,0,0` | 110.6 / 131.5 | 101.5 / 115.3 | |

Compiled clocked RTL executes 20 to 25 percent fewer instructions than the interpreter now and takes 11 to 15 percent
more cycles (IPC 1.1 against 1.6 on the CPU design): the branch misses (407 M against 219 M), instruction-cache misses
and instruction TLB misses of unique code per process remain, and so does the kernel's share (35 to 45 percent of the
run, identical in both tiers). The default `--tier=auto` still compiles none of these processes; its rules
(`RTL_INSTANCES`, `RTL_SHARE`, `too_many_callbacks`, which counts the signal operations compiled code now answers
itself as callbacks) are unchanged. The interpreter gained 1.2 percent of its instructions on `fletcher_bufferwriter_512`
(the waits of split sensitivity lists) and nothing elsewhere; the per-process frames grow only under `--tier=llvm`.

The code of a whole-program module in huge pages (`YAVS_LLVM_HUGE=0` off): `neorv32_single_min` `--tier=llvm` 70.3 to 65.7 G
cycles at equal instructions (instruction TLB misses 52 M to 9 M; the table above is without it).

## llvm: compiled clocked RTL, round 5: the front end, 2026-10-23 (#634)

Decisions: `docs/decisions.md` "Compiled clocked RTL processes, round 5". An exact instruction trace of one `rtl_ff`
activation (1231 instructions, 325 of them the compiled body) and `perf` front-end counters showed that the cost is
plumbing and branch layout, not the body: the scratch swap at every suspension, the second copy of a wait that never
short-cut, a 20 KB stack frame in `enter`, the waveform of every scalar assignment, `-disable-block-placement` (26 taken
branches in 316 instructions) and a chain of four inline lookups at every access of unknown space.

Cycles (G) of the simulation phase, bench builds, core 12 (13 for the wide design and the `rtl_*` pair), minimum of 5
interleaved runs, host shared; `--tier=auto --tier-heat=1,0,0` (every executed function compiled) against the
interpreter of the same binary:

| Design | before (main `f7904064`) auto / interp | now auto / interp | auto / interp now |
|---|---:|---:|---:|
| `neorv32_single_min` | 33.4 / 28.8 | 23.9 / 23.6 | 1.01 |
| `fletcher_bufferreader` | 21.4 / 14.7 | 14.6 / 13.0 | 1.12 |
| `fletcher_bufferwriter_512` | 73.4 / 51.7 | 43.3 to 53.6 / 36.3 to 40.4 | 1.12 to 1.2 |
| `sc5_fifo_repack` | 13.4 / 16.7 | 9.0 / 13.6 | 0.66 |
| `rtl_ff` | 3.48 / 3.34 | 2.81 / 3.36 | 0.84 |
| `rtl_lfsr` | 1.86 / 2.03 | 1.30 / 1.80 | 0.72 |
| `rtl_chain` | 1.02 / 0.84 | 0.91 / 0.84 | 1.08 |

Instructions: `rtl_ff` 12.2 G to 9.05 G (interpreter 11.6 G), `rtl_chain` 3.9 G to 3.17 G (3.2 G), `neorv32_single_min` 52.6 G to
47.3 G (61.9 G), `fletcher_bufferreader` 37.0 G to 32.0 G. Process-size sweep (a clocked process of 1024 instances, K
more `xor` terms, `--tier=auto` against interpreter, cycles): 144 IR lines 0.88, 221 0.71, 313 0.98, 497 0.85, 865 0.88,
1041 1.19 (code footprint over the cache); no crossover from below any more, so the lower end of `RTL_BAND` (#575) has no
measured reason left. The interpreter's own cost steps up 2.7 times between 32 and 40 terms (#662).


## llvm: compiled clocked RTL, round 6: the cache lines of an activation, 2026-10-24 (#634)

Decisions: `docs/decisions.md` "Compiled clocked RTL processes, round 6". Exact instruction counts per function of one
activation (`tools/stepcount`) showed the plumbing around a small compiled process at its floor for this structure (about 440
of 1550 instructions of an `rtl_chain` activation; `view_again` is 68 loads and stores); `perf record -e ibs_op// -d` (the data
source of each sampled load) showed what the floor costs: the compiled `rtl_fan` activation waited on L3 in `view_again` and
`run_quick`, five cache lines of a machine and its instance data that nothing else in an activation reads. They are one line now
(`Machine::vc`), and `spec_at` moved next to `execs` (both tiers read it at every activation).

Cycles (G) of the simulation phase, bench builds with `--features llvm`, minimum of 7 interleaved runs (4 for the three large
designs, 9 for the reader's `autoh`) on a quiet host, core 12 for the `rtl_*` designs and 13 for the others (the two CCD1 cores
differ: compare rows, not cores); before = main at 70ba4d90, after = the same plus the commits 40686ab2 and 41d88a3b. `auto` is
the default policy, `autoh` is `--tier=auto --tier-heat=1,0,0` (every executed function compiled); the ratio is to the
interpreter of the same binary:

| Design | interp before / after | auto before / after | autoh before / after | autoh / interp before / after |
|---|---:|---:|---:|---:|
| `rtl_fan` | 3.98 / 4.09 | 4.16 / 4.05 | 4.45 / 4.02 | 1.12 / 0.98 |
| `rtl_chain` | 0.85 / 0.84 | 0.85 / 0.87 | 0.89 / 0.86 | 1.04 / 1.02 |
| `rtl_ff` | 3.29 / 3.37 | 3.65 / 3.42 | 2.77 / 2.75 | 0.84 / 0.82 |
| `rtl_lfsr` | 1.83 / 1.79 | 1.34 / 1.32 | 1.29 / 1.28 | 0.71 / 0.71 |
| `fletcher_bufferreader` | 12.66 / 12.63 | 13.00 / 12.76 | 14.10 / 13.85 | 1.11 / 1.10 |
| `neorv32_single_min` | 22.45 / 22.15 | 22.20 / 22.07 | 25.00 / 25.18 | 1.11 / 1.14 |
| `fletcher_bufferwriter_512` | 31.58 / 32.03 | 32.42 / 32.43 | 38.50 / 38.27 | 1.22 / 1.20 |

Instructions (G) of `autoh` before / after: `rtl_fan` 13.68 / 13.30, `rtl_chain` 3.17 / 3.13, `rtl_ff` 8.98 / 8.78, `rtl_lfsr` 4.38 /
4.33, reader 30.44 / 30.33, `neorv32_single_min` 46.84 / 46.53, writer 80.64 / 81.14 (the interpreter: 13.2, 3.2, 11.4, 6.3, 36.7,
57.2, 92.6). The fills from L3 of compiled `rtl_fan` fell from 82 M to 65 M (the interpreter's from 64 M to 59 M), the samples of
loads that missed L1 in `view_again` and `run_quick` from 1.6 to 0.9 percent of the sampled operations. The scatter between batches on this shared host is
2 to 3 percent for the small designs and up to 8 percent for the large ones: a change below that needs the counters of the
tooling above (instructions, fills from L3, instruction-cache misses) to be believed.

What is left, by the numbers. The large designs run 14 to 20 percent more cycles compiled with every function compiled although
they execute 13 to 17 percent fewer instructions (`neorv32_single_min` IPC 1.9 against 2.6): L1 instruction-cache misses 1.6 G
against 0.6 G, branch mispredictions 112 M against 64 M. In a single-stepped window of 1.5 M instructions of `neorv32_single_min` (about 15 clock cycles) the compiled code
touched 933 KB of distinct cache lines (162756 distinct instruction addresses, each executed 2.7 times) and the runtime 102 KB;
the interpreter touched 131 KB in all: the footprint of unique code per process, not the entry, is the cost. Sending the
large expansions (`SigRead` of a known count, `LCopy`, variable `Alloc`) of every site through helpers cut the instruction-cache
misses by 8 to 12 percent (one variant not at all), added 2 to 5 percent instructions and ran 3 to 8 percent slower
(`neorv32_single_min`, four interleaved variants). `--tier=auto` leaves these designs interpreted (`auto` column), which is
right while the compiled ones are slower. Remaining ideas, none cheap: a code size budget in the policy's decision (the
footprint of what it compiles, not only the time it saves), profile-guided block layout for the processes of one clock edge, a
fused vector assignment in compiled code (the interpreter has `LCopyDrive` and `CopyDrive`; running them through the
interpreter's function was measured and costs more than the compiled group, the planes in registers and a lean callback are
the version to try).

Real-world suite (`tools/realbench`, 42 tests, interleaved A/B of the same two binaries, cores 12 and 13, 3 runs, tiers interp and
auto): instructions overall 1.00 (interpreter) and 0.99 (auto), wall 0.98 and 1.02; the tests of one second or more: 0.95 and 0.97.
Tests below 0.1 s vary by 30 percent run to run.

## `--tier=auto`: what leaves compiled code, and compiled clocked processes measured and taken back (#633, #596, 2026-10-23)

The callback count of the policy now takes the signal reads, `'event`, the clock-edge tests, the wait that ends an activation and
direct calls as staying in compiled code (`lower::leaves`); a clocked process is compiled at the final level at once and measured
after the swap (process runs per CPU second with the compiled code and with the interpreter, in turn), and the code is taken back
when the run did not progress at least 1.05 times as fast. The size band of #575 became a wait for bodies outside it (a second; a quarter of a second since #652).
Results on core 11 (cycles against `--tier=interp`, before / after): `sc5_fifo_repack` 1.013 / 0.778, `rtl_ff` at five times its
length 1.029 / 0.932, `rtl_lfsr` 0.797 / 0.763, `hdlm_lfsr`, `vai_window_gen_interlock`, `poc_arith_addw`, `neorv32_single_min`,
`poc_cache_cpu`, `fletcher_bufferwriter_512` and the other benchmark designs level; table and method in docs/decisions.md ("the
callback count sees what leaves compiled code"). Processes that loop are measured by their activations since #652 (functions stay on the static rules); `fused_search`
stays (the scan helper of #578 is not enough: `uvvm_sb_slv` 1.05 to 1.12 without it, after the callee fix).

## llvm: compiled protected-method accessors and search loops (#626, #578, 2026-10-23)

Bench builds (`tools/build-bench.sh --features llvm`), one performance core (core 14, CCD1), median of 5, main-thread
`perf stat` (`--no-inherit`). `protected_queue` (`tools/bench`) with `--tier=llvm` and the `LEAN` pipeline (the code
`--tier=auto` makes, no inlining; deterministic), each row on top of the one above:

| Change (tag) | instructions (G) | cycles (G) |
|---|---:|---:|
| main before | 8.78 | 2.55 |
| closed native frames are not registered, their slots stay in registers (LLVM-only) | 8.15 | 2.40 |
| a `Deref`ed object stays in hand, one-test `Index` guard, cheaper `Pos` (LLVM-only) | 6.44 | 1.53 |
| blocks with one predecessor keep the held objects (LLVM-only) | 5.88 | 1.38 |
| a `Deref` hit is one compare (LLVM-only) | 5.62 | 1.41 |
| the interpreter's top frame found inline by native frames (LLVM-only) | 5.62 | 1.39 |

One protected `get` of a pointer table (micro design, 1 million calls, instructions per call including the loop): 594, 555,
397, 359, 337. The accessor itself (the native function) went from 449 to 231 instructions (`gdb` trace).
`protected_queue`, `--tier=auto` warm: 9.26 to 6.20 G instructions, 2.65 to 1.48 G cycles (-44 percent); the interpreter is
unchanged (28.4 G). The window generator (`vai_window_gen_interlock`, `--tier=auto` warm): 230.5 to 187.6 G instructions, 74.5
to 59.5 G cycles (-20 percent); against the reference simulator (wall, 5 runs): 3.49x to 1.96x warm, 3.94x to 2.32x cold
(instructions 3.43x to 2.76x warm). Host callbacks of the window generator (`cb_*`, `op-census` build): `cb_host` 43.5 M to
5.3 M, `cb_fast` 13.7 M (6.8 M with a 1024-entry object table, not kept).
The 41 tests of the real-world suite that pass on both sides (`tools/realbench`, interp and auto): wall geomean 0.98 and 0.98, instructions 1.00 and
0.99; no test slower beyond the run-to-run spread.

Search loops (#578): a 100-element byte search called 400 thousand times through a native frame: compiled 887 to 143 M
cycles (4.15 to 0.45 G instructions), interpreter 518 to 170 M cycles (2.79 to 0.65 G). With the helper the compiled loop is
as fast as the interpreter's, but `uvvm_sb_slv` is still slower with the policy rule `fused_search` off (1.05 to 1.2 of the
interpreter's cycles, +10 to +28 percent instructions): the compiled `pos_of_leftmost` calls the interpreted `bitvis_assert`
twice per call, 111 thousand `Call`s through `yavs_rt_step` (`YAVS_COUNTERS=pc`), because that callee is left out of the
compile as never seen (`UNSEEN_MIN`) and compiled 340 ms later. The rule stays.

Not done: a flat heap-slot view in `TierFrame` (the remaining four `Deref`s of an accessor are 88 of its 231 instructions;
a flat view would take about 10 of the 22 each and the miss callbacks), inlining accessors across modules (a call and a
return cost about 50 instructions of 337).

## Specialised copies of processes (#651, 2026-10-23)

Six commits (`docs/decisions.md` "Specialised copies of processes", `docs/design/specialisation.md` sections 11 and 12): a process of
an instance with generics and constants switches, once it has run 1024 activations, to a copy of its code optimised for its instance's
frozen data (the optimiser folds the data's integer reads: lengths, slice positions, allocation sizes, checks, arms of `if generic then`).
Equal data share a copy, the parameters of `for` generates are not folded, the copies' optimised IR is capped at 2.5 times (1.5 until 2026-10-24) the shared
process IR, they are kept in the compilation cache (kind `spec`), and the tiers see them as ordinary functions. Shared code under
`--dap`, `--vhpi`, wave variables, coverage, `--tier=llvm`, serial processes and processes with several waits.

Bench builds with `llvm`, core 9, `tools/realbench/run.py --compare` (a world per binary, medians of 3), user instructions of the whole
process tree in G, main at d72ffdd9 before / after; the cycles and the wall times are noisy on the loaded host (the same binary and the same
instructions gave 16.8 to 26.6 G cycles under `--tier=auto` on `sc5_fifo_repack`: the compile thread shares the pinned core), the instruction
counts are exact:

| design | interp | auto warm | auto cold | copies (switched), IR growth |
|---|---|---|---|---|
| `poc_arith_addw` | 372.93 / 331.99 (-11.0 %) | 287.28 / 265.70 (-7.5 %) | 329.24 / 304.46 (-7.5 %) | 147 (147), 1.49x (cap) |
| `sc5_fifo_repack` | 57.64 / 48.63 (-15.6 %) | 47.27 / 49.58 (+4.9 %, #661) | 57.07 / 50.46 (-11.6 %) | 21 (34), 1.49x (cap) |
| `fletcher_bufferwriter_512` | 106.36 / 98.64 (-7.3 %) | 106.92 / 99.26 (-7.2 %) | 106.88 / 99.23 (-7.2 %) | 72 (94), 0.64x |
| `poc_cache_cpu` | 25.36 / 24.07 (-5.1 %) | 25.76 / 24.44 (-5.1 %) | 26.21 / 24.99 (-4.7 %) | 49 (274), 0.56x |
| `hdlm_lfsr` | 69.02 / 66.77 (-3.3 %) | 31.90 / 32.53 (+2.0 %, #661) | 35.91 / 36.23 (+0.9 %) | 1 (1), 0.23x |
| `neorv32_single_min` (300 us) | 64.97 / 63.36 (-2.5 %) | 65.61 / 63.98 (-2.5 %) | 67.78 / 65.70 (-3.1 %) | 91 (171), 0.24x |
| `osvvm_axis_random`, `hdlm_axi_crossbar`, `hdlm_fifo_sync`, `olo_fix_cordic_rot` | within 1.1 % | within 0.3 % | within 2 % | 0 |

Geometric mean over ten tests: instructions 0.96 (`interp`), 0.98 (`auto`), 0.97 (`auto-cold`); interleaved cycle medians
(`--tier=interp`) `poc_arith_addw` -10.4 %, `hdlm_lfsr` -4.6 %, `neorv32_single_min` -2.5 %. `tools/bench` h2h designs: geometric mean of CPU
1.01 (noise; instructions equal). Copies off (`YAVS_NO_SPEC=1`) and initialisation (`--stop-time=0fs`) within 0.1 and 0.4 percent of before.
The cap costs 0.5 percent on `poc_arith_addw` and 4.3 points on `sc5_fifo_repack` (-19.6 % without it). The prototype's
`neorv32_single_min` gain (-3.3 %) is -2.5 % now. #661 (2026-10-24): the tier counts the copies of a process as that
process (time, share and instance rules, classification, a trial's verdict) and keys compiled copies by what they are for, not by function id;
`sc5_fifo_repack` warm with copies is no worse than without (`docs/design/specialisation.md` section 12, "The tier policy by process").

Checked: `cargo test --workspace` (also with `YAVS_SPEC_AFTER=2`, every process switching at its second activation), the differential
corpus (499/499) on `yavs-interp`, `yavs-auto` and `yavs-llvm`, plain and with `YAVS_SPEC_AFTER=2`, at 1 and 4 threads; 38 realbench
testbenches (OSVVM, UVVM, hdl-modules, Open Logic, PoC, neorv32, fletcher, satcat5) byte-identical with copies off and forced, interp
and auto, 1 and 4 threads (`tests/spec.rs` does the same for every design of `tests/perf`).

Not verified: AArch64 and Windows (CI), a debugger or VHPI session with copies on (they are off there by design), copies with
`--threads` above 4.

## llvm: a flat view of the heap slots, built and not landed (#655, 2026-10-23)

A per-slot `{address, length, generation}` view of both heaps, read by compiled `Deref` and by accesses to the addresses it makes, was
built with the invariants of #655 (generation on every access, freed slots empty, no cloned or stored addresses, empty under
`TIER_STAMPS`, refreshed with `heap_epoch`, a cross-check against the handle path in debug and `gc-stress` builds). It was correct
(fuzz test, tier tests under `gc-stress`, 499 of 499 corpus tests on a `gc-stress` build on the interpreter, `auto` and `llvm`) and
slower than the object table it was to replace: one protected `get` 336 to 367 instructions, `protected_queue` +7 percent, the
window generator +3 percent (`--tier=llvm`), real testbenches under `--tier=auto` unchanged. Nothing is in the tree; numbers, the
update-point audit and what would change the verdict are in decisions.md "A flat view of the heap slots for compiled `Deref`".

## Cold front end, round 2: entries stay bytes one by one, type marks in declarations, generic bodies stored (#424, 2026-10-24)

Profile of a cold `yavs analyze`, one file per call (frame-pointer build, `perf record` with call graphs over a whole analysis corpus):
the replay of the dependencies' entries was 34 to 40 percent of the cycles, a fifth of that one table (`written_marks`, 12 percent), a
tenth the lookup of entries (`Key::hex` made 32 strings, 4 percent), and every cold `elab` of a UVVM testbench re-analysed four generic
package bodies from source. Changes (decisions in `docs/decisions.md` "Analysed-unit cache" and "Cold front end, round 2"):

- The type mark of a parameter and of a function result, as written, is a field of the declaration (`Param::mark`, `Subprogram::ret_mark`)
  instead of the replayed table `Db::written_marks`. Cold analysis instructions 0.79 (UVVM, vunit) and 0.86 (hdl-modules) of before.
- `Key::hex` without allocations, `UnitStore::note` takes a closure (nothing is formatted unless `YAVS_LOG=debug`, the flag is read once),
  `Enc` keeps the owner number of each (load, kind): a further 0.09.
- Every entry of a replayed unit stays bytes until it is read (`Arena::push_run`, `ids::SlotPage`): 0.58 of before for UVVM, 0.57
  vunit, 0.61 hdl-modules after all changes. Expressions and statements decode in runs of a page (a body is read whole by lowering),
  the rest one entry at a time (a window of 4 on every kind cost analysis 4 percent for 2.5 percent on `neorv32` elab; tried and not kept).
- The body of a generic package is stored (its entry records the removal of the generics its declaration put into
  `GenericState::unused`): five units of UVVM and OSVVM that every first `elab`/`run` analysed from source. Cold `elab` of
  `uvvm_sb_slv` 0.40 G to 0.28 G instructions; warm analysis of UVVM 4.32 G to 2.15 G (those files are verified instead of replayed).
- The check of a patch (a body completing its package's subprograms) compares raw bytes first (8 percent of a cold `elab`).

Cold analysis, one file per call, whole corpus manifest, instructions in G, `tools/build-bench.sh` build, `perf stat -e instructions:u`
over all calls (cores 0 to 7 and 16 to 23, instruction counts do not depend on the core):

| selection | before | after | ratio |
|---|---:|---:|---:|
| osvvm-2008 | 10.94 | 5.48 | 0.50 |
| open-logic-2008 | 21.93 | 11.26 | 0.51 |
| poc-2008 | 4.86 | 2.76 | 0.57 |
| vunit-2008 | 7.53 | 4.30 | 0.57 |
| uvvm-2008 | 16.93 | 9.73 | 0.57 |
| hdl-modules-2008 | 1.56 | 0.94 | 0.61 |
| satcat5-2008 | 6.22 | 3.97 | 0.64 |
| neorv32-2008 | 2.00 | 1.43 | 0.72 |
| grlib-1993 | 19.14 | 14.76 | 0.77 |

Warm analysis (the same files again): satcat5 1.51 to 1.45 G, hdl-modules 0.25 to 0.21, OSVVM 1.48 to 1.29, UVVM 4.32 to 2.15.

`tools/realbench/run.py --compare`, ten tests (one per selection that has a test), instruction ratios after / before (wall ratios on
a loaded host are not stable; the cold analysis wall geometric mean was 0.76): cold analysis 0.65 (OSVVM 0.53, Open Logic 0.56,
UVVM 0.73 and 0.85, hdl-modules 0.60, vunit 0.59, PoC 0.65, neorv32 0.76), warm analysis 0.84, cold `elab` 0.91 (`uvvm_sb_slv` 0.70,
`vunit_logging` 0.87, `en_cl_fix_pkg` 0.90, `osvvm_axis_random` 0.91, `hdlm_fifo_sync` 0.92; `neorv32_single_min` 1.06: its lowering reads
nearly every entry it replays, and decoding one by one costs more per entry than a page), warm `elab` 0.98.

Checked: `YAVS_UNIT_CHECK=1 yavs analyze` cold and warm over osvvm, uvvm (2008, 2019), vunit (2008, 2019), hdl-modules (2008, 2019),
open-logic, neorv32, poc, satcat5, grlib-1993, surf, vhdl-extras, en_cl_fix, fletcher, json-for-vhdl, neotrng, vhlib and
vhdl-compliance: as expected everywhere. Left, in order of weight: the index of a library (`yavs-lib.toml`) is parsed and rendered whole
by every call (a quarter and a tenth of a warm call); the bodies of generic packages are cloned per instance at every cold `elab`
(`fix_bodies`, 11 percent of `osvvm_axis_random`, #555); the entries of the closure are opened, checksummed and their heads parsed by every
call (10 percent of a cold call).

## llvm: a native frame no longer materialises for a wide fill or compare (#648, 2026-10-24)

`poc_stat_histogram` (`--tier=auto`, warm) ran 17.3 G instructions at b40752eb and 20.3 G at
337c5aa9 ("Compiled clocked RTL ... round 4"), found by bisecting main with `tools/build-bench.sh
--features llvm` builds (the 65b7f573 to 9a8d8f50 window of the h2h2 report; later main hid +3 G
of it behind other gains). Cause: that commit gave `LFill` and `LEq` an inline form for 1 to 64
logic elements whose slow path is `Gen::step`; in a native frame (`Cur::nat`) a step materialises
the whole frame (`deopt_here`) and the interpreter runs the rest of the call. The testbench's
`to_slvv_8` (a 256 row matrix to a vector of rows, on every change of the matrix) fills 2048
elements, so 9300 of its calls left compiled code at that `LFill` and interpreted the 256
`get_row` calls that follow. Before the commit the instruction was an `xstep_ok` kind and went
through `native_step` (the interpreter runs just it, on a frame of its own).
Fix: the slow path of `LFill`, `LEq` and the static `LayoutOf` goes through `Gen::slow_path_x`,
which uses `slow_inst` (native step in a native frame, plain step in frame mode). Counters
(`llvm_wide_fill`, debug build): `cb_deopt` 401 to 2, interpreted `ops` 281483 to 95.
Instructions, same warm world and `--tier=auto`, before to after: poc_stat_histogram 19.67 to 16.66 G, uvvm_axistream_vvc 58.0 to 49.8, uvvm_util_rand
15.84 to 14.77, sc5_fifo_repack 49.5 to 46.2, vai_window_gen_interlock 193 to 181 (single counted
runs of the realbench A/B on loaded cores; `--tier=auto` instruction counts move by several percent
between two runs of the same binary because the compile thread finishes at another time: poc_arith_addw
262 to 288 G for both binaries, and 327.9 G for both under `--tier=llvm`, which has no such timing).

## Library index: spliced when written, read without lines; removals of a set read from its log (#424, #664, 2026-10-24)

`yavs analyze` of one file into a library of a few hundred files read the index (`yavs-lib.toml`, 100 KB for satcat5) line by line
and wrote it whole again: a quarter and a tenth of a warm call. The index is now written by copying the entry of every file that did
not change (`Manifest::render`, `adopt`; a debug build writes it whole as well and asserts the same text), the reader scans for
the end of a string eight bytes at a time and reads unit kinds and numbers without allocating, and the session no longer reads the
index a second time to compare with the disk. Decision: `docs/decisions.md` "Library layout".

Warm analysis (the same files again, one per call, instructions in G): satcat5 (246 files) 1.45 to 0.90, Open Logic (386) 3.06 to
2.56, hdl-modules 0.21 to 0.20, UVVM 2.15 to 2.11; cold analysis of satcat5 3.97 to 3.70, grlib 14.76 to 14.29.

Also in this round: `YAVS_UNIT_CHECK=1` over the `yavs-lib` corpora found a package body in a file of its own, whose declaration is loaded
inside it, stored without the removal of the declaration's generics from `GenericState::unused` (the net change of the body is
nothing, so its entry recorded nothing and a replay left the keys in; #664, the same on main before this work). The removals of a
set are read from its write log now (`TrackedSet::removed_since`), whatever the net change; two tests in `tests/units.rs`.
`YAVS_UNIT_CHECK=1` cold and warm over 34 corpus selections (the 21 of the previous section, grlib-2008, vests-1993, the tool-check, ideas,
interfaces, vunit-tests, yavs-lib and compliance sets) is as expected.

Also: a lookup of a `std`/`ieee` unit that the work library lacks walked the mapped libraries in path order, sorted afresh each time
(190 failed `open` calls per cold call of UVVM, 30 libraries); it starts at the library that had the last one now (cold UVVM 9.72 to
9.30 G instructions, kernel cycles 4.38 to 3.90 G over the 235 calls). A cold `analyze` call spends about as many cycles in the kernel
as in user mode (UVVM: 4.4 G against 6.7 G, 730 page faults and 545 system calls per call, 230 faults and 3 M kernel instructions of them
for an empty call): instruction counts of user mode alone understate the wall time.

Final numbers after both pushes (cold analysis, whole manifest one file per call, G instructions before / after): osvvm 10.94 / 5.40,
Open Logic 21.93 / 10.94, PoC 4.86 / 2.68, vunit 7.53 / 4.26, UVVM 16.93 / 9.30, hdl-modules 1.56 / 0.92, satcat5 6.22 / 3.70, neorv32 2.00 / 1.41,
grlib-1993 19.14 / 14.21. Warm analysis: satcat5 1.51 / 0.90, Open Logic 3.06 / 2.55, UVVM 4.32 / 1.75, OSVVM 1.48 / 1.20, hdl-modules 0.25 / 0.19.
Cold `elab`: `uvvm_sb_slv` 0.40 / 0.28, `osvvm_axis_random` 0.65 / 0.60, `olo_base_cam` 1.09 / 1.04, `hdlm_fifo_sync` 0.64 / 0.58, `en_cl_fix_pkg`
0.50 / 0.43, `vunit_logging` 0.20 / 0.16, `neorv32_single_min` 0.63 / 0.67 (#665), `poc_uart_rx` 1.78 / 1.77 (90 percent of it evaluates
constants in the interpreter, a quarter of that in the IR optimiser at the first call of each function: not front end).

## Wide logic vectors: fills and copies as bytes (2026-10-24)

Found while looking for run-time instructions outside the dispatch loop of the worst realbench
tests (perf record of the main thread, `--tier=auto`, warm): `uvvm_axistream_vvc` spent 7 percent in
`bytes_of`, 6 percent in `Machine::lfill` and 3 percent in `obj_mut`. The verification component
fills and copies logic vectors of 131072 and 524288 elements per command; `lfill` did one address
lookup and four plane read-modify-writes per 64 elements, and a wide `lcopy` read both sides into
two block vectors, copied the bits and wrote a third vector back.
Change: `lfill_wide` (512 elements or more) writes the whole blocks as one slice of one 32-byte
pattern; `lcopy_aligned` (1024 elements or more, both sides at the same bit of a block, no shared
byte, checked by `disjoint`) is a partial block at each end and one `copy_mem` of the whole
blocks; the generic wide copy writes the result into the destination without the third vector.
The thresholds keep the new test off the small paths: a first version with the branches at 128 and
256 elements cost 0.1 to 0.3 percent of instructions on tests without wide vectors.
Instructions (`--tier=interp`, deterministic, warm world, main thread): uvvm_axistream_vvc 48.18 to
38.87 G (-19 percent); poc_fifo_ic_assembly 2.2606 to 2.2639 G, osvvm_axi4_random 7.6138 to 7.6178 G,
fletcher_bufferreader 38.37 to 38.44 G (+0.1 to +0.2 percent, the code layout of the loop, not the
new paths); uvvm_util_rand, uvvm_sb_slv, uvvm_func_cov, hdlm_lfsr, sc5_fifo_repack unchanged.
New work-counter design `wide_vec` (every alignment of fills and copies, overlapping shifts, each
element checked): the correctness test of the new paths. What sim1 does: it keeps one byte per
element, so a fill is a `memset` and a copy a `memmove`; ours are the same on the whole blocks and
the plane words only at the ends. Compiled code (`yavs_rt_lcopy`, up to `LCOPY_BLOCKS` blocks) steps
wider copies into this same function.


## IR optimiser round 12: statement markers of signal statements; what was tried for temporaries, checks and drives (#588, 2026-10-24)

`may_allocate` (the instructions after which a `Stmt` marker is not needed because they allocate no temporary memory) now lists signal
references, signal reads, edge and attribute tests and drives, so a process statement that only reads and drives signals loses its
marker; waits keep theirs (the predecoder's `static_wait` starts at the marker). Host instructions, bench builds, `--threads 1`, main at
35eec8cb before / after (`docs/decisions.md` "IR optimiser round 12" has the whole table and the method):

| design | `--tier=interp` | `--tier=llvm` |
|---|---|---|
| `sc5_timestamp` | -1.1 % | -0.3 % |
| `sc5_fifo_repack` | -0.6 % | -0.1 % |
| `neorv32_dual_full`, `neorv32_single_min` | -0.5 %, -0.4 % | 0.0 %, 0.0 % |
| `fletcher_bufferreader`, `fletcher_bufferwriter_512` | -0.5 %, -0.3 % | 0.0 %, -0.1 % |
| `poc_cache_cpu`, `poc_stat_histogram`, `poc_arith_addw`, `vai_window_gen_interlock` | -0.1 %, 0, 0, 0 | 0, -0.1 %, 0, 0 |
| h2h `rtl_fan`, `resolved_bus`, `rtl_ff`, `rtl_bus` | -3.2 %, -2.1 %, -1.5 %, -1.4 % | -1.0 %, -0.5 %, -0.7 %, -0.4 % |

Not landed, with numbers in the decisions entry: constant-size temporaries in the process frame (+4 to +19 percent interpreter
instructions on four of five designs: the fused ops match `Alloc`; #668), a wider inlining cut-off (-1.1 percent on the window generator
only), signal-value sharing across joins (within 0.5 percent), range-based check removal and promotion between suspensions (the ops they
would remove are 1 to 11 percent of the ops, cheap ones, and the hot checks compare loaded values). Findings for others: 17 percent of
the drives of `neorv32_single_min` are repeats within one activation (#667), the vector assignment is three unfused ops (#669), and
`--tier=auto` instruction counts vary by up to 7 percent between runs of one binary (#670).

Checked: `cargo test --workspace`, `-p yavs-llvm -p yavs --features llvm`, the `gc-stress` unit tests, clippy with and without `llvm`, the
differential corpus (502/502) on `yavs-interp`, `yavs-llvm` and `yavs-auto` (also `--tier-heat=1,0,0`) at 1 and 4 threads, OSVVM (521
files, 17 libraries) and UVVM (131 pass, 6 skipped) results equal to those of main, seven realbench outputs byte-identical to main (interpreter, 1 thread; three of them also at 4 threads and under `--tier=auto`).

## Compact constants are read through the fast memory lookup (2026-10-24)

`uvvm_func_cov` spent 6 percent of its instructions in `Machine::load_slow`, 4 percent in
`const_bytes` and 3 percent in `rd_slow`: a load from a compact constant (`ConstData::Repeat`,
`Runs`, `Logic`) found nothing in `mem_fast` (only byte constants are there), went on through
`load_slow` and `rd_slow` to `obj`, and `const_bytes` looked the constant and its expanded copy up
again on every load. `mem_fast` now also answers for a compact constant whose expanded copy exists
(the first access still expands it through `obj`).
Instructions, `--tier=interp` (deterministic), warm, main thread: uvvm_func_cov 3.052 to 2.396 G
(-21.5 percent), osvvm_axi4_random 7.608 to 7.543 G, uvvm_sb_slv 12.700 to 12.662 G, sc5_fifo_repack
42.43 to 42.30 G, uvvm_util_rand and fletcher_bufferreader unchanged within 0.1 percent;
hdlm_lfsr 66.84 to 67.16 G (+0.5 percent) and poc_fifo_ic_assembly 2.280 to 2.285 G (+0.2 percent),
which are the layout of the dispatch loop (a variant with the branch out of line moved them and
cost uvvm_func_cov 5 percent more). Cycles were not measured (no timing core free).

## lower: one copy of a record signal per activation (#630, 2026-10-24)

A process with a sensitivity list that reads a record signal as a whole in two places or more keeps one copy (made in the declarative
part, filled at the start of each activation); a `std_ulogic` field of a record signal is read with `SigReadScalar` in place
(`docs/decisions.md` "Composite signal reads: one copy per activation"). GRLIB LEON3 (`grlib_leon3mp`, `--std=93`, whole run, bench
build, `--tier=interp --threads 1`, core 10): 1685.8 G instructions and 150.8 s before, 993.3 G and 91.3 s after; the integer unit's
`comb` process 147 760 to 30 605 IR instructions. `sigrec_copy` in the work-counter tests: ops 105 380 to 52 919.

## CPU designs against the reference simulator: where the time goes, and idle skip round 2 (#521, #523, #529, 2026-10-24)

The three neorv32 processor tests, `poc_cache_cpu`, `sc5_fifo_repack` and `hdlm_lfsr` profiled on main at cf32383b (`--tier=auto`,
warm cache, release bench build, core 9 for the profiles, `perf record` cycles:u flat, `--profile`, `YAVS_COUNTERS=1`, a temporary
per-process timer and the `op-census` feature). Elaboration and initialisation are 1 to 2% of a run (7% in `neorv32_dual_full`,
0.3 s of 4.1 s); the rest is the simulation thread. Share of the cycles by layer:

| design | interpreter dispatch (`run_ops`) | interpreter other (ops, `copy_bits`, process glue) | kernel (cycle, commit, wake) | signal callbacks (`Ctx` read, assign) | compiled code and its callbacks | libc copy and allocation | native ieee ops | compile thread, load |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| `neorv32_single_min` | 38.0% | 11.8% | 24.9% (+0.9 sorts) | 16.1% | 0 | 4.6% | 2.7% | 0.8% |
| `neorv32_single_cache` | 39.3% | 14.1% | 20.4% | 13.4% | 0 | 7.2% | 3.1% | 1.9% |
| `neorv32_dual_full` | 37.7% | 14.6% | 19.8% | 12.6% | 0 | 8.2% | 3.5% | 2.0% |
| `poc_cache_cpu` | 51.3% | 13.0% | 15.9% | 9.3% | 0 | 4.3% | 4.6% | 1.0% |
| `sc5_fifo_repack` | 50.8% | 7.9% | 10.8% | 7.4% | 17.6% (callbacks 17.0) | 3.8% | 1.3% | 0.2% |
| `hdlm_lfsr` | 5.3% | 17.3% | 36.0% | 8.0% | 29.0% (callbacks 6.3) | 2.0% | 1.1% | 0.4% |

Nothing is compiled in the neorv32 and poc runs (the compile thread is 0.4% of the samples): the tier leaves the processes
interpreted ("a clocked process seen for 11768 of 268962 us ...", and the three biggest, `counter_increment`, `exec_comb` of 4479 IR
instructions and `request`, are neither compiled nor named in the notes). By process (neorv32_single_min, 300 us, 9.85 M activations):
`exec_comb` 12% of the time inside processes (6.0 us an activation, about 150 per-leaf record drives), `counter_increment` 10% (7.1 us,
woken by every event on `ctrl_i`, which has 41 leaves of which it reads two), `bus_gateway.request` 9% (5.3 us), `counter_core` of the
29 counters 5% (0.3 us, 652 k activations), then a tail of 2400 processes of at most 2.5% each. The kernel is 20 to 25% of the
cycles; the commit path (`finish_direct`, `commit_out`) holds 22% of the L1 data misses (a `Signal` is over 700 bytes). In
`poc_cache_cpu` 85% of the process time is two concurrent statements of the 32-element LRU (`mux` and `ffdre` calls on
unconstrained vectors, 1.6 and 0.95 us), 58% of the runs of the second being wakes on data inputs while the clock edge was false.
In `hdlm_lfsr` the testbench `main` (serial, a file, 22 operations an activation) is 16% of the run and is never compiled; the DUT
process `main` is interpreted for 190 ms before its specialised copy is compiled; the 22 single-bit drives of `state(i) <= ...`
cost `drive_bit_now`, `assign_code_fast`, `commit_codes` and `finish_direct` together 18%. In `sc5_fifo_repack` 11% is
`Host::object` and `yavs_rt_obj` (compiled code reaching the process variables of a procedure through the up-level chain) and
the interpreted `AddrUp -> Load` pairs are 5% of the ops.

Against the reference simulator (elaboration plus run, `--tier=auto` warm, wall of one thread on core 15, medians of 5; before
this change / after): `poc_cache_cpu` 2.00x / 1.42x (instructions 2.89x / 1.93x), `neorv32_single_min` 1.49x / 1.37x,
`neorv32_single_cache` 1.24x / 1.15x, `neorv32_dual_full` 0.93x / 0.80x (faster than the reference), `sc5_fifo_repack` 2.75x /
2.8x to 3.1x (the compile thread's timing moves it by 15% between runs), `hdlm_lfsr` 1.80x / 1.81x.

What changed (`docs/decisions.md` "Idle skip, round 2"): the idle skip of round 16 covers combinational processes (a wake nobody read
for is skipped), clocked concurrent assignments `q <= d when <edge>;` (edge trigger and idle skip), process variables assigned
before they are read, and pure functions of architectures; processes up to 4096 registers stay resident. Wall of the run phase,
main against the change (interpreter / auto): `poc_cache_cpu` 0.67 / 0.67, `neorv32_dual_full` 0.88 / 0.86,
`neorv32_single_min` 0.91 / 0.91, `neorv32_single_cache` 0.89 / 0.88; instructions 0.63, 0.84, 0.90, 0.85.

What is left, by cost: the interpreter's dispatch and the signal drive path (about 60% of the neorv32 cycles, a drive is about 190
cycles including its commit); the kernel's per-event work; nothing of these designs is compiled; per-bit drives of vector
signals; process variables read from compiled code (`Host::object`); `mux`/`ffdre`-style functions of unconstrained vectors
(5 000 instructions an activation).

Real-world suite after #630 (`tools/realbench/run.py --ref sim1 --cores 10 --runs 2 --phases elab,run --tiers interp,auto,auto-cold`, bench build
with `llvm` at 20264d5e, medians of 2, host load 22 to 77 so the wall times are indicative and the instruction ratios are the reliable part).
`--tier=auto` against the reference's elaboration plus run (the run cell of yavs includes its elaboration; the reference elaborates in 0.42 to 0.48 s
on the LEON3 and neorv32, 0.16 s on the window generator):

| test | reference elab + run (s, G instr of the run) | auto warm | auto cold |
|---|---|---|---|
| `grlib_leon3mp` | 0.48 + 14.09, 171.1 | 82.64 s, 5.7x (ins 5.51) | 94.58 s, 6.5x (ins 5.71) |
| `neorv32_single_min` | 0.45 + 3.88, 38.7 | 5.79 s, 1.34x (ins 1.47) | 5.33 s, 1.24x (ins 1.51) |
| `vai_window_gen_interlock` | 0.16 + 5.91, 70.5 | 11.10 s, 1.83x (ins 2.48) | 13.48 s, 2.22x (ins 3.50) |

Interpreter, before and after #630 on the LEON3 (own measure, `--threads 1`, user instructions): 1685.8 G, 150.8 s to 993.3 G, 91.3 s.
The LEON3 was 150.2 s warm and 146.5 s cold under `--tier=auto` (11.25 times the reference's instructions) when #630 was filed. `--tier=llvm` on the
whole design: 84 s of code generation (9.8 M LLVM instructions: generation 5.8 s, passes 50.6 s, code generation about 25 s), the run 174 s against 91 s
interpreted (1520 G instructions against 993 G): the compiled frame-mode code of the design's processes does not execute fewer instructions than
the interpreter (#679). What is left of the LEON3 gap is spread: `iu:comb` 18.9 percent of the interpreted run (record signal reads and drives, #677),
`dctrl` 9.7, the four `state_register` processes of the SDRAM model 14, the two TLB processes 7.5, `ictrl` 3.7, the kernel 16.5.

## Idle skip, round 3: reads under a condition (#521, 2026-10-24)

A process that reads a signal only under a condition (`if en = '1' then q <= d;`, the arm of an `if`, an `elsif` chain) is no longer
armed by an event on that signal while the condition is false (`docs/decisions.md` "Idle skip, round 3"). Instructions of the
interpreter run, before / after (bench builds, user instructions of the process tree in G): `neorv32_single_min` 52.68 / 48.30
(0.917), `neorv32_single_cache` 46.27 / 43.25 (0.935), `neorv32_dual_full` 37.42 / 35.40 (0.946), `osvvm_axis_random` 3.82 / 3.62
(0.948), `osvvm_axi4_random` 7.54 / 7.38 (0.979), `fletcher_bufferreader` 0.996; every other real-world design within 0.6%
(`poc_cache_cpu` 1.004: the guard tests of its registers cost a little where nothing is saved), the 11 head-to-head designs 1.000.
Cycles (core 15, medians of 5, interpreter, round 2 against round 3): `neorv32_dual_full` 0.921, `neorv32_single_cache` 0.928,
`neorv32_single_min` 0.914, `osvvm_axi4_random` 0.938, `osvvm_axis_random` 0.947, `uvvm_axistream_vvc` 0.951; geometric mean over 37
real-world designs 0.996 (the short designs move by 2 to 4% between runs of one binary; `poc_cache_cpu` 0.915 and 0.910 alone, interpreter
and auto).

## `--tier=auto`: callees with their caller, band wait 0.25 s, trials of looping processes (#652, 2026-10-24)

A callee that every call of a hot function makes is compiled with it (`must_calls`); a clocked process outside the 160 to 300
band is compiled after a quarter of a second of interpreted time (was a second); processes that loop are measured after the swap by their activations (functions are not: compiled callers and phases of the run make the count mislead).
Warm, instructions of auto against interp, main before / after: `rtl_ff` 1.041 / 0.922, 351 and 535 IR variants of `rtl_ff`
1.040 / 0.784 and 0.687, `lfsr64` at ten times its length 0.950 / 0.909, `rtl_ff` at five times 0.855 / 0.802,
`sc5_fifo_repack` 1.019 / 0.951; the window generator, `poc_arith_addw` and the other designs of the short set equal.
`fused_search` stays: `uvvm_sb_slv` without it 1.05 to 1.12 of the interpreter's cycles (1.22 before). Table, method and the
evidence in docs/decisions.md ("callees with their caller, `band_wait` 0.25 s, trials of looping processes").


## Record signals as packs of leaves: design (#686, 2026-10-24)

`docs/design/record-signal-layout.md`: runs of up to 64 logic leaves of one record signal in one
kernel signal, the physical leaf map, which leaves may share a signal, what changes per consumer,
and six landable steps with expected gains. Measured on a 233-leaf register record (interp): 55% of
the instructions are IR execution (#677's gather and scatter), 37% per-leaf kernel work (the target
of the packs). Finding: sim1 keeps one runtime signal per leaf too.

- Step 1: a leaf is stored at `NetGraph::loc` (kernel signal, base); the kernel build, instance
  signal tables, waves, coverage toggles, VHPI and DAP reach kernel signals only through it
  (`NetGraph::kref`, `kpart`, `kelem`). Every leaf still has a signal of its own.
- Step 2: runs of logic leaves of a record signal (at most 64 elements, no port, guard, force or
  implicit signal) share one kernel signal; idle-skip readers are armed by their elements.
- Step 4: one kernel driver per pack a whole record assignment names; `SigScatter` assigns a pack in
  one plane block, `SigGather` reads vector leaves from the planes. Instructions (interp, warm,
  on top of #677): 233-leaf register record 5.20 G to 3.40 G, `grlib_leon3mp` 809.6 G to 801.4 G,
  `neorv32_single_min` 42.79 G to 42.26 G. Step 3 (packs with ports) measured and not landed
  (`docs/decisions.md`); step 5 measured, the cap stays 64. Not done: the whole-record wait, step 6
  (byte leaves).

## `yavs_rt_host` a leaf, and no cliff in the instance-constant memoisation (#663, #662, 2026-10-24)

- #663: `hdlm_lfsr` under `--tier=auto` (warm, instructions of the process tree, bench builds, 8 fresh worlds interleaved) was 31.1 G before the
  two #634 commits, 32.7 G after: its hot loop reads a table of the instance data through the one `yavs_rt_host` call frame-mode code makes for an
  address of unknown space, and that call saved and restored 7 registers on every hit. The lookup is now a leaf and the interpreter's object table a
  cold function behind it: 30.7 G (cycles level with before: medians of 14 interleaved samples 9.26 G against 9.45 G, 9.63 G for the call).
  `fletcher_bufferreader`, `fletcher_bufferwriter_512`, `neorv32_single_min` unchanged, `sc5_fifo_repack` within the bimodal noise (#670).
- #662: the memoisation of instance-constant values (`opt/memo.rs`) gave up at 40 values or 800 instructions in a block, so one more xor tap
  than 32 in a clocked process cost 2.7 times the instructions of the whole run (interpreter, 1024 instances, 4000 cycles: 22.6 G at 32 taps,
  60.9 G at 40, 92.7 G at 64). Caps now 256 values and 4096 instructions, a block over 800 instructions copied only when the fast copy saves a
  fifth of it: 22.5 G, 26.7 G, 39.3 G, 73.8 G at 128 taps (linear, 0.53 G a tap). Every real-world and head-to-head design
  executes the same instructions as before (within 1 percent); with the caps alone `olo_axi_master_full` executed 12 percent more (its three
  processes of 1400 to 1600 instructions save 9 to 15 percent of their block and predecoding the second copy is dear, #683), hence the fifth.
- #671: stage entries (`init`, `low`, `ir`, `spec`, `ecall`) are keyed by the `Db`'s extent and a digest of its source files instead of
  being tied to the extent of the analysis, so designs that load units while they elaborate (components bound by default to entities
  analysed after their user: `fletcher_bufferwriter_512` 36 to 57 units) get entries. Instructions at time 0 (steady state, interp):
  `fletcher_bufferwriter_512` 497.1 M to 263.3 M, `fletcher_bufferreader` 364.9 M to 214.8 M; whole runs 91.94 G to 91.57 G and
  36.74 G to 36.50 G. `hdlm_lfsr` has no late units (it waits for its own entries on the second run as before).

- #665: replayed entries decoded one by one now live in a box of cells per page and a read directly after the last decoded run decodes
  twice as many entries (`ids.rs` `SlotPage`). Cold `elab` of `neorv32_single_min` 685.6 M to 674.7 M instructions (-1.6 %); cold analysis
  of `hdl-modules-2008` -0.2 %, `osvvm-2008` +0.3 %, `poc-2008` +0.2 %. About 4 percent of the #424 loss is still open (`docs/decisions.md`
  "Replayed entries: cells per page").

- #665: replayed entries decoded one by one now live in a box of cells per page and a read directly after the last decoded run decodes
  twice as many entries (`ids.rs` `SlotPage`). Cold `elab` of `neorv32_single_min` 685.6 M to 674.7 M instructions (-1.6 %); cold analysis
  of `hdl-modules-2008` -0.2 %, `osvvm-2008` +0.3 %, `poc-2008` +0.2 %. About 4 percent of the #424 loss is still open (`docs/decisions.md`
  "Replayed entries: cells per page").
- #683: predecode of a very large process was quadratic in its waveform builds (each searched on to the end of its block and scanned the
  rest of the function twice for liveness). `olo_axi_master_full` at time 0 (interp, steady state): 703.5 M to 660.0 M instructions (-6.2 %),
  a generated process of 1600 assignments 64.7 M to 60.8 M; outputs and work counters unchanged (`docs/decisions.md` "Predecode: one scan
  per waveform").
- #691 and the native `ieee` call path: `ieee::small` returns `Option` and reads with one bounds check per word, `put_small` appends the
  72 bytes of its result to temporary memory in one go, `closure()` uses sets for files seen, done and on the DFS stack. Instructions,
  bench build, warm cache, `realbench --compare` over all 42 tests (interp and llvm): `poc_arith_addw` interp 236.1 G to 220.2 G (-6.7 %),
  llvm 224.7 G to 208.7 G; cycles on core 14 interp 70.8 G to 68.4 G (-3.4 %), `--tier=auto` 57.0 G to 53.4 G (-6.5 %); `poc_cache_cpu` interp
  15.12 G to 14.51 G; `neorv32_single_min` 42.9 G to 42.3 G; `olo_axi_master_full` at time 0 623.4 M to 605.8 M; every other test within 1 %
  (no instruction ratio above 1.01). `tools/bench --compare` (31 designs) CPU geomean 0.991 (`docs/decisions.md` "Native ieee calls").

## llvm: address spaces across blocks, enclosing frames from the view (#684, #685, #679, 2026-10-24)

(`docs/decisions.md` "llvm: address spaces across blocks, and enclosing frames from the view".) Bench builds, `--tier=auto`, core 11,
`tools/realbench/run.py --compare` (medians of 3): instructions B/A `sc5_fifo_repack` 0.81 (45.7 G to 37.1 G, wall 2.79 s to 2.29 s),
`hdlm_lfsr` 0.92 (30.7 G to 28.2 G), `neorv32_single_min` 1.00, `fletcher_bufferreader` 1.00, `fletcher_bufferwriter_512` 1.00. With
`--tier-heat=1,0,0` (every hot function compiled early, instructions of the simulation thread): `sc5_fifo_repack` 34.8 G to 29.3 G (cycles
10.3 G to 7.8 G), `hdlm_lfsr` 33.8 G to 32.4 G. The head-to-head designs under `--tier=llvm` execute the same instructions
(`rtl_ff` 9.04 G, `rtl_lfsr` 4.66 G, `rtl_chain` 3.22 G, `rtl_fan` 13.59 G; cycles within the noise of the host, 5 percent either way
between identical runs of `rtl_fan`). Calls of compiled code into `yavs_rt_host` (`YAVS_COUNTERS=1` on an `op-census` build, `--tier-heat=1,0,0`), with the `Index` rule alone
and with the dataflow too: `hdlm_lfsr` 101.9 M and 1.3 M (its loop's address register is made in the pre-header block), `sc5_fifo_repack`
25.6 M and 23.8 M, `neorv32_single_min` 20.0 M and 17.3 M. The enclosing-frame accesses of `sc5_fifo_repack` went through the
interpreter's object table (13 percent of its cycles) and now read the view.

What is left of #679 (measured, not done). LEON3 60 us window, `iu:comb` forced (`YAVS_AUTO_FORCE=comb@1000#0`), instructions of the
simulation thread: interpreter 125.5 G, compiled 93 to 96 G (-24 percent), cycles level (IPC 3.0 against 2.2); branch misses 85 M to 154 M,
iTLB misses 0.09 M to 4.6 M, L1i misses 38.6 M to 47 M. The same instructions with `Fast` and with the optimised code generator
(`YAVS_LLVM_FAST_MIN`), so the quality of the machine code of the body is not the lever; callbacks are 6 percent of the cycles
(`yavs_rt_lcopy` 2.0, `native_direct` 1.5, `yavs_rt_host` 0.9, `forget_since` 0.5) and 1.4 M stepped `LayoutOf` without a cache (373 per
activation, #678) are the largest block of steps. `yavs_rt_host` is 2.5 percent of `fletcher_bufferwriter_512` (157 M calls: the addresses
are fat pairs returned by calls and natives and pointers loaded from slots, which have no static space). Tried and dropped because they moved
nothing: `Field` keeping the space of its base (reader 78 M to 68 M calls, instructions 0.995, writer level), constant tables read by a run-time
index (`Known::ConstAny`: no site in these designs).

## Whole record signals: `SigGather` and `SigScatter` (#677, 2026-10-24)

A record signal, or a part of one (a field that is a record or an array of records, an element or a slice of an array of records), is read by
one `SigGather` (also the per-activation copy of #630) and driven by one waveform element by one `SigScatter` (`docs/decisions.md` "Whole record
signals: gather and scatter"). Records may have arrays of records as fields, arrays of records may be nested; a value of other array lengths
than the signal's is a length error (#703). Both instructions are in the IR, so every tier and a later code generator run them; the interpreter
does a Rust loop over a plan made from the leaf table and the descriptor (kept by the machine: no lock on the hot path), the compiled tier steps
them. Signal parameters and drives of several waveform elements keep the per-leaf code: the census over ten designs found no waveform of
several elements in 515 drive sites, and a signal parameter in 9 (one hot: 12.7 percent of the executed ops of its statements in
`osvvm_axi4_random`, about 1 percent of that test's instructions). Bench builds (`tools/build-bench.sh --features llvm`), the commit before
(`fdef24b4`) and after, `--tier=interp --threads 1`, user instructions (`perf stat -e instructions:u`), steady state (five runs of each
cell, the last counted: the first runs fill the compilation cache and differ by up to 100 percent on a small test):

| test | before | after | change |
|---|---:|---:|---:|
| `grlib_leon3mp`, whole run | 780.77 G | 702.51 G | -10.0 % |
| `fletcher_bufferwriter_prim32` | 1.136 G | 1.102 G | -3.0 % |
| `fletcher_bufferwriter_512` (`stage_r`, an array of records, whole and by slice) | 74.17 G | 72.55 G | -2.2 % |
| `fletcher_bufferreader` | 24.82 G | 24.50 G | -1.3 % |
| `neorv32_single_cache` | 31.04 G | 30.74 G | -1.0 % |
| `neorv32_single_min` | 38.33 G | 38.00 G | -0.9 % |
| `neorv32_dual_full` | 26.36 G | 26.15 G | -0.8 % |
| `hdlm_axi_fifo`, `hdlm_axi_crossbar`, `hdlm_register_file`, `vunit_axi_dma`, `osvvm_axis_random`, `osvvm_axi4_random`, `uvvm_axistream_vvc`, `olo_axi_master_full`, `sc5_fifo_repack`, `poc_cache_cpu`, `hdlm_fifo_sync`, `hdlm_ring_buffer`, `poc_uart_rx` | | | within 0.2 % |

`--tier=llvm`: `grlib_leon3mp` 796.45 G to 744.60 G (-6.5 %), `neorv32_single_min` 51.91 G to 50.57 G (-2.6 %), `fletcher_bufferwriter_512`
80.68 G to 79.47 G (-1.5 %). A synthetic design of 32 instances with a record of 48 logic leaves (read and driven every clock): 120.39 G to 118.81 G
at one thread; the contention of the plan cache's lock at `--threads 4` was not measurable on a host with a load average of 20 to 70.
The commit before is the one with the record packs (#686) and the drive-deleting pass (#667): a part that another drive of the process
overrides stays leaf by leaf (`neorv32_single_min` was +5.9 percent with a scatter of `port_req(i)`).

The LEON3 gain comes from the machine's own plan list (no `RwLock` read, no clone of the plan and of the instance data at each instruction), from the
records the table now takes (the `timers` and `p` records of two units, which have arrays of records as fields, `tlbcami(i)` and the record fields
`r.f`) and from a leaner loop for a value without arrays of records; the shares are not separated. The census
before the change (executed IR ops per statement of 25 us to a whole run, the share of the ops of the statements that still read or drive a
record signal leaf by leaf, leaves above one): `fletcher_bufferwriter_512` 7.7 percent, `neorv32_single_min` 10.4 percent, `grlib_leon3mp` 0.4 percent,
every other design below 0.1 percent; the 495 single-field reads of the LEON3 integer unit are one `SigVal` and one read each, which a gather
cannot help. What is left in the LEON3 run is the kernel's per-leaf work (`assign_code_fast`, `commit_code`).


### First step: whole record signals only (#677, 2026-10-24)

A record signal whose fields are records, scalars or arrays of scalars is read as a whole by one `SigGather` (also the per-activation copy of #630) and
driven as a whole by one waveform element by one `SigScatter` (`docs/decisions.md` "Whole record signals: gather and scatter"). Both are in the IR, so
every tier and a later code generator run them; the interpreter does a Rust loop over a plan made from the leaf table and the signal's descriptor (kept
per runtime), the compiled tier steps them. Parts of a record signal, arrays of records, signal parameters and drives of several waveform elements keep
the per-leaf code. Bench builds (`tools/build-bench.sh --features llvm`) of the commit before and after, `--tier=interp --threads 1`, user instructions
(`tools/realbench/run.py --compare`, one run each; the cycle and wall figures of that tool ran on a loaded host and are not used):

| test | before | after | change |
|---|---:|---:|---:|
| synthetic record of 233 `std_logic` leaves, read and driven 20 000 times | 3 786 M | 1 735 M | -54 % |
| `grlib_leon3mp`, whole run | 916.7 G | 815.6 G | -11.0 % |
| `grlib_leon3mp`, first 150 us (cycles `perf stat`: 89.5 G to 82.0 G, -8.4 %) | 196.1 G | 182.2 G | -7.1 % |
| `fletcher_bufferreader` | 36.79 G | 26.97 G | -26.7 % |
| `neorv32_single_cache` | 42.33 G | 37.56 G | -11.3 % |
| `neorv32_single_min` | 47.48 G | 43.25 G | -8.9 % |
| `olo_axi_master_full` | 0.84 G | 0.77 G | -8.3 % |
| `fletcher_bufferwriter_512` | 92.04 G | 86.78 G | -5.7 % |
| `fletcher_bufferwriter_prim32` | 1.77 G | 1.65 G | -6.8 % |
| `sc5_fifo_repack`, `osvvm_axis_random`, `osvvm_axi4_random`, `uvvm_axistream_vvc`, `poc_cache_cpu`, `hdlm_axi_fifo`, `hdlm_axi_crossbar`, `hdlm_register_file`, `vunit_axi_dma`, `vai_window_gen_interlock` | | | within 0.3 % |

Cycles of the whole LEON3 run on core 15 (`perf stat` cycles:u, bench builds, two interleaved pairs, difftest running on the other CCD): before 427.6 G and 379.5 G, after 364.7 G and 369.7 G (-4 to -15 percent, instructions 912.5 G to 806.5 G, -11.6 percent).

The LEON3 integer unit's `comb` (VHDL-level `--profile`, 100 us, a loaded host so a hint): 1281 to 1011 samples (-21 %), 26 018 to 19 181 IR instructions;
what is left in it are the reads and drives of single fields of other records (495 reads, 93 drives). `sigrec_copy` ops 52 919 to 29 817. In the
233-leaf design the interpreter's two loops are now 32 % of the instructions and the kernel's per-leaf work (`assign_code_fast`, `commit_code`,
`run_batch`, `commit_out`) 54 %, which `docs/design/record-signal-layout.md` and the equal-value drop of the kernel lane address.

## Window generator against sim1: where the time is now, measured by layer (#493, #520, #673, #680, 2026-10-24)

`vai_window_gen_interlock`, bench build of main at 8a9cf87e, world of `tools/realbench`, core 9, host load average 20 to 30 (wall and cycles
swing by up to 50 percent between identical runs, instructions by 7 percent under `--tier=auto`, so instruction shares are quoted).
`tools/realbench/run.py --ref sim1 --runs 3 --tiers interp,auto,auto-cold`: sim1 7.52 s (68.2 G instructions), interp 47.2 s (6.3x wall, 7.3x
instructions), auto warm 14.3 s (1.90x, 2.63x), auto cold 16.7 s (2.22x, 3.64x). `perf stat` user counters on a quieter pass: sim1 20.2 G cycles
and 65.7 G instructions in 5.4 s, auto warm 61 G cycles and 171 to 191 G instructions in 12.8 s.

Layers of a warm `--tier=auto` run (`perf record -e instructions:u` with `YAVS_PERFMAP=1 YAVS_LLVM_PERFMAP=1`, share of instructions / of cycles):

| layer | instructions | cycles |
|---|---:|---:|
| compiled code (`[JIT]`), of which the pointer-table accessors `get` `set` `length` 18.3, `unsafe_push` `unsafe_pop` 5.3, the codec loops (`encode` `decode` `to_byte_array` `from_byte_array` `"and"` `"srl"` `pop_fix_string` `push_variable_string`) 12.5 | 37.9 | 35.5 |
| interpreter loops (`run_ops`, two instantiations) | 25.8 | 28.6 |
| interpreter helpers (`lcopy`, `copy_bits`, `subtype_check`, `obj_mut`, `drive`, `Pre::fill`) | 11.8 | about 8 |
| `TierHost` (`native_direct` 2.8, `native_step` 1.5, `object` 1.2) and `yavs_rt_*` callbacks | 9.7 | about 6 |
| kernel (`cycle`, `finish_direct`, `run_batch`, `commit_out`) | 4.3 | about 4 |
| libc (`malloc`, `free`, `memmove`, `memset`) | 3.6 | about 3 |
| `ieee` natives (`put_small`, `small`, `fat_dim`, `to_unsigned`) | 3.4 | about 2.5 |
| compile thread (links of cached objects and the few compiles of a warm run) | 1.9 | 2.8 |

The reference simulator on the same test: 61 percent compiled code (about 40 G instructions), 23 percent runtime, 12 percent code generation
(about 8 G); its compiled code is about 12 G cycles, ours about 22 G in the pointer-table accessors and the codec loops alone.
The VHDL-level profile (`--profile`): `bus_process` of the stream slave (with `check_axi_stream`) 35 percent of the samples, the testbench
`main` 30, `walk_control` 5, `window_hold_check` 5; 457 000 clock cycles, 2.9 M process runs, 19 M events: about 417 000 instructions per clock
cycle, nearly all of it testbench-side message passing (a 576-bit expected window per beat pushed through the queue byte by byte). The compiled
tier makes 75 M `get` calls of 285 instructions and about 130 cycles each (instruction histogram of `f46_n`, approximate: 30 instructions of frame entry, 4 object
look-ups of about 18 instructions, 3 dope tests of about 25, 3 offset guards of about 10); callbacks to the host by instruction kind
(`YAVS_COUNTERS=kinds`) are only 3 M in the whole run (`CallProt` 1.2 M, `Call` 0.5 M, `Drive` 0.4 M, `NewFill` 0.2 M, `LCopy` 0.15 M).

Cold against warm: 241 to 245 G instructions against 171 to 191 G. The compile thread is 13 to 14 G of the difference (85 compiles, 2.9 s of
compile thread time on the timing core; the 45 compiles with `YAVS_LLVM_TIMES=1` took 3.8 s of wall time shared with the main thread for 110 000 LLVM
instructions before the passes and 70 000 after, code generation 70 percent and the lean passes 25; the profile of the compile thread is flat: instruction selection, register allocation, nothing above 2 percent), the other
45 G is the main thread interpreting what is not compiled yet: `bus_process` and `check_axi_stream` are never compiled in a cold run of 15 s,
`encode` is first seen at 8 s (150 ms warm). On one pinned core the compile thread's CPU is wall time (cold 16.7 s against warm 14.3 s); a
second core (9,10) gave 17 to 18 s against 19 to 22 s on the same loaded host, a hint only.

What was measured and gives nothing (all deterministic where `--tier=llvm`, `perf stat` user instructions, second run on a filled cache):

- Forcing the interpreted processes and subprograms the policy refuses (`YAVS_AUTO_FORCE` on 15 names: `walk_control`, `window_hold_check`,
  `main`, `assemble`, `present`, `control`, `enqueue_frame_expected`, `check_equal`, `put_message`, `LocalRandInt`, `push_tile`, `random_frame`,
  `check_field`, `_p3`, `bus_process`): 173 to 178 G instructions against 175 G, cycles level.
- `--tier=llvm` (everything compiled before time 0): 236 G instructions and 87.5 G cycles in 18.4 s (init 1.8 s warm), against 191 G and 61 G
  of auto; with the lean pipeline everywhere 222.0 G; plus `loop-mssa(licm)` 226.4 G (+2.0 percent); `gvn` and `licm` 225.5 G (+1.6 percent).
  The loops of the codec do not hoist: their object look-ups sit behind calls and slow-path exits.
- Compiling `bus_process` (2 133 IR) gives the same sampled time as interpreting it (3.0 s compiled, 3.4 s interpreted), as #679 says.

## llvm: code arena in huge pages, entry dispatch tree, hot edge falls through (#679, 2026-10-24)

(`docs/decisions.md` "llvm: code in one arena of huge pages, ...".) Bench builds, one timed run at a time on core 11, `perf stat
--no-inherit` of the simulation thread, medians of 5 interleaved runs, `tools/realbench` worlds of a private cache. The host was
shared with other builds and runs: cycles vary by 10 to 30 percent between identical runs, so the instruction, iTLB, L1i and
branch-miss counts carry the comparison and cycles are shown with their minimum. `--tier-heat=1,0,0` compiles every hot function early and
deterministically; `--tier=auto` alone compiles at times that depend on the host (instructions of `sc5_fifo_repack` and the writer
differ by 8 and 1.3 percent between identical binaries), so its cycles are level within the noise.

| case (base to arena+dispatch+branch) | instructions | iTLB misses | L1i misses | branch misses | cycles median (min) |
|---|---:|---:|---:|---:|---:|
| `neorv32_single_min`, heat | 40.93 G to 40.97 G | 15.6 M to 1.1 M | 46 M to 50 M | 143 M to 101 M | 30.2 G to 22.7 G (24.1 to 21.4) |
| `fletcher_bufferreader`, heat | 30.97 G to 31.07 G | 3.8 M to 0.14 M | 65 M to 51 M | 60 M to 54 M | 20.8 G to 22.6 G (16.0 to 15.8) |
| `fletcher_bufferwriter_512`, heat | 82.77 G to 82.87 G | 17.3 M to 0.34 M | 140 M to 117 M | 274 M to 226 M | 45.5 G to 42.6 G (43.3 to 41.3) |
| `sc5_fifo_repack`, heat | 28.58 G to 28.44 G | 0.05 M to 0.02 M | 8.6 M to 7.3 M | 11.3 M to 11.3 M | 7.73 G to 7.41 G (7.52 to 7.38) |
| LEON3 60 us, `iu:comb` forced | 87.1 G to 90.2 G (auto timing) | 4.8 M to 0.29 M | 72 M to 66 M | 143 M to 131 M | 37.1 G to 38.5 G (36.1 to 37.5) |
| `neorv32_single_min`, auto | 44.6 G to 45.2 G | 1.0 M to 0.5 M | 20 M to 26 M | 81 M to 84 M | level |
| `rtl_fan`, `--tier=llvm` | 13.59 G to 13.69 G | level | level | level | 4.61 G to 4.64 G (4.49 to 4.42) |
| `rtl_ff`, `--tier=llvm` | 9.04 G to 9.06 G | level | level | level | 3.25 G to 3.24 G (3.10 to 3.09) |

An earlier run of the same code (before one dropped experiment, a quieter host) had heat-mode cycles at 22.7 G to 22.8 G, 18.2 G to
17.7 G, 47.9 G to 43.7 G and 7.78 G to 7.68 G for the four designs (medians, in that order): minus 3.4, 2.6, 8.7 and 1.2 percent. Reading:
the instruction TLB misses are gone (92 to 99 percent fewer) and the branch misses of compiled code are 10 to 25 percent fewer, which
is worth 1 to 9 percent of the cycles of the designs that spend their time in compiled code; the front-end loss of a large compiled
process (#679: IPC 3.0 to 2.2) is not closed: the LEON3 window stays level. `perf stat -M PipelineL1` of that window: front-end
latency is 13.6 percent of the dispatch slots with the default `--tier=auto` against 20.5 percent with `iu:comb` forced, retiring 43.1 against
36.6, back-end 25.4 against 27.7; the compiled `comb` copy is 3.7 MB (158 bytes per IR instruction, a 319 KB stack frame) of which a
sampled window touches about 150 KB, more than the L1i holds.
Difftest 505/505 on `yavs-llvm`, `yavs-auto` and `yavs-auto` with `--tier-heat=1,0,0` at 1 and 4 threads, and with `YAVS_LLVM_ARENA=0`.

## llvm: small code model, calls through the GOT, blocks in execution order, narrow masks (#679, 2026-10-24)

(`docs/decisions.md` "llvm: small code model, calls through the GOT, ...".) Bench builds (`tools/build-bench.sh --features llvm`) of main
(base) and of the change, `tools/realbench` worlds of a private cache (`--prepare`), one timed run at a time on core 11 (CCD1, a loaded
host: load average 18 to 22), `perf stat --no-inherit` of the simulation thread, medians of 7 interleaved runs, each world warm (two
untimed runs before). `--tier-heat=1,0,0` compiles every hot function early and deterministically; the LEON3 case is the 60 us window
(`--stop-time=60us`) with `YAVS_AUTO_FORCE=comb@1000#0`. Cycles are `cycles:u`.

| case (base to change) | instructions | cycles median (min) | frontend stall cycles | L1i misses | branch misses |
|---|---:|---:|---:|---:|---:|
| `neorv32_single_min`, heat | 36.04 G to 35.91 G | 16.95 G to 16.17 G (16.53 to 15.80), -4.6 % | 3.82 G to 3.03 G | 37.6 M to 30.0 M | 66 M to 55 M |
| `fletcher_bufferreader`, heat | 25.94 G to 25.80 G | 10.36 G to 10.11 G (10.07 to 9.85), -2.4 % | 2.09 G to 1.85 G | 53.8 M to 46.1 M | 27.3 M to 25.7 M |
| `fletcher_bufferwriter_512`, heat | 80.12 G to 79.52 G | 37.43 G to 33.61 G (36.72 to 33.38), -10.2 % | 11.46 G to 8.42 G | 124.9 M to 96.4 M | 165 M to 116 M |
| `sc5_fifo_repack`, heat | 28.50 G to 28.42 G | 7.36 G to 7.37 G (7.27 to 7.28), level | 0.51 G to 0.50 G | 11.5 M to 9.4 M | level |
| LEON3 60 us, `iu:comb` forced, heat | 91.9 G to 91.1 G | 43.1 G to 36.3 G (36.5 to 33.0), -16 % median, -9.5 % min | 11.86 G to 7.57 G | 103.9 M to 88.7 M | 108.7 M to 88.5 M |
| `neorv32_single_min`, `--tier=auto` | 39.77 G to 39.81 G | 17.51 G to 17.57 G, level | 2.45 G to 2.43 G | level | level |
| `sc5_fifo_repack`, `--tier=auto` | 33.86 G to 34.04 G | 9.39 G to 9.56 G (9.18 to 9.31), +1.8 % | 0.89 G to 0.92 G | 16.3 M to 15.5 M | 14.2 M to 15.0 M |
| `fletcher_bufferreader`, `--tier=auto` | 26.53 G to 26.57 G | 10.51 G to 10.57 G, level | 1.77 G to 1.76 G | 41.7 M to 33.2 M | level |
| `fletcher_bufferwriter_512`, `--tier=auto` | 82.64 G to 82.65 G | 32.13 G to 32.30 G (31.66 to 31.80), level | 5.76 G to 5.79 G | 81.5 M to 69.3 M | 118 M to 116 M |
| LEON3 60 us, `--tier=auto`, nothing forced (9 runs) | 72.1 G to 73.9 G | 30.8 G to 30.4 G (25.8 to 26.1), level; the host's spread is 26 to 46 G | 4.22 G to 4.11 G | 56.3 M to 49.9 M | level |

(`--tier=auto` alone compiles at host-dependent times: the instructions of `sc5_fifo_repack` are 33.1 G or 37.9 G from run to run in
both builds, which is the +1.8 percent; an earlier pair of the same builds had it at -2.8 percent. The LEON3 median of the base is
inflated by one slow sample: 5 runs of an earlier pair gave 37.0 G to 33.3 G, -10 percent.) Interim variants, same method, 5 runs:
the small model with PLT stubs and nothing else, cycles level (neorv32 +5 %, LEON3 -0.6 %, writer +3 %); with the block layout and
the narrow masks and `nonlazybind` -4.7 / -2.0 / -8.4 / -6.8 percent (neorv32, reader, writer, LEON3).

Code size (objects of the `Fast` level of the base and of the change, written with `YAVS_LLVM_OBJ` from a cold compile cache): LEON3 `comb`
copy, 20 122 IR instructions: `.text` 2 526 254 to 2 354 640 bytes (125.5 to 117.0 bytes per IR instruction, -6.8 %), machine instructions
484 870 to 448 257 (-7.6 %), `jmp` 39 937 to 24 616, `movabs` 26 041 to 10 554. `neorv32` `exec_comb` (4 531 IR instructions): `.text`
477 590 to 443 508 bytes (-7.1 %). Compile time of the `comb` copy at `Fast` (`YAVS_LLVM_TIMES=1`, 3 interleaved pairs, core 11): total 2.43 to 2.70 s
before, 2.22 to 2.62 s after (code generation 1.28 to 1.40 s against 1.03 to 1.12 s; the block order adds 20 to 90 ms to the generation).

The head-to-head designs under `--tier=llvm` (`tools/bench/designs`, 5 interleaved runs, core 11) execute 0.1 to 0.3 percent fewer instructions (`rtl_ff` 9.15 G to 9.12 G, `rtl_fan` 13.89 G to 13.85 G, `rtl_lfsr` 4.69 G, `rtl_chain` 3.24 G); cycles within the noise of this host (median `rtl_ff` -0.6, `rtl_fan` -3.9, `rtl_lfsr` +1.2, `rtl_chain` +9.7 percent, minimum -1.2).

Difftest 508/508 at 1 and 4 threads for `yavs-llvm`, `yavs-auto`, `yavs-auto` with `--tier-heat=1,0,0` and `yavs-auto` with
`YAVS_LLVM_ARENA=0`.

## IR optimiser round 13: repeated drives, copies without the declarative part, conditional assignments that idle (#667, #678, #521, 2026-10-24)

Three changes (`docs/decisions.md` "IR optimiser round 13", "Specialised copies start after the declarative part", "Idle skip, round 4"):

- `opt/drives.rs`: in a process, a zero-delay drive (with the waveform it builds) that a later zero-delay drive of the same elements
  deletes is delayed down the control flow and dropped on the paths that drive again (copies at the end of the paths that do not;
  before any instruction that can see the driver's transactions; never into a cycle). `YAVS_NO_DRIVES=1` turns it off. Tests:
  `crates/yavs/tests/drives.rs` (14 designs, output and exit status equal with and without the pass, fewer kernel drives), perf design
  `drive_defaults`.
- A specialised copy of a process jumps from its entry to the body (the declarative part is never run in it): the copies of the LEON3
  run are 16 percent smaller in IR; no change in interpreter instructions. Test `a_copy_has_no_declarative_part`.
- Idle skip for concurrent conditional assignments `t <= a when c else b;`, with conditions of generics worked out per instance
  (`neorv32` `... when TRACE_PORT_EN else ...`, the PMP `... and NAP_EN else '0'`): an arm that cannot be taken leaves no read. Tests:
  perf `idle_when`, `tests/corpus/delta/idle_when.vhd` (blessed from the reference simulator).

Host instructions, bench builds, `--threads 1`, G, main at c9abf712 / with the three changes: `neorv32_single_min` interp 42.55 / 39.74
(-6.6 %), llvm 55.99 / 52.91 (-5.5 %); `neorv32_dual_full` interp 28.40 / 28.02, llvm 45.81 / 45.76; `fletcher_bufferwriter_512` interp
84.41 / 83.27, llvm 88.71 / 88.74; `fletcher_bufferreader` interp 26.01 / 26.11; `poc_cache_cpu` interp 15.00 / 14.91; `sc5_fifo_repack`,
`sc5_timestamp`, `poc_stat_histogram` unchanged. One at a time on da793761 (before the record gather and scatter of #677): drives
44.85 to 42.76, copies without the declarative part 42.78 (nothing, it is code size), idle skip 41.66; `poc_arith_addw` +0.2 percent
(tracked assignments that never skip), `grlib_leon3mp` interp 891.98 / 892.53. Cycles (core 10, median of 3, interpreter):
`neorv32_single_min` 20.4 to 19.6 G, `neorv32_dual_full` 15.3 to 14.6, `fletcher_bufferwriter_512` 38.6 to 38.1, `poc_cache_cpu` 5.29 to
5.47 (+3 percent for -0.6 percent instructions, within the spread of a run of 5 G). Checked: the 505-test differential corpus (interp, llvm, auto
at 1 and 4 threads), UVVM (131 pass, 6 skipped) and OSVVM (521 files, 314 tests pass) as on main, `YAVS_IDLE_SKIP=check` on five
real-world designs, `cargo test --workspace` and `-p yavs-llvm -p yavs --features llvm`.

What is left: the repeats behind a join (tail duplication), a repeat across a loop's iterations of dynamic offsets that are not one
register, PoC `ffdre` (#701), sharing the two `to_integer` of
a check's condition and message (#520, a comment with the plan).

## IR optimiser round 14: record defaults (#667, #677, 2026-10-24)

`opt/drives.rs` sees `SigScatter` (`docs/decisions.md` "IR optimiser round 14"): a record driven whole is a footprint (the leaf signals of its
table and the part registers) that kills what it covers, is a barrier only for what it may overlap, and a scatter with a constant source
(a default such as `port_req(i) <= req_terminate_c`) is delayed and dropped where an override follows like a drive. With that the
leaf-by-leaf lowering of record parts that another drive may override (`Fx::overridden_signals`) is gone: every record part is one
`SigScatter` again. Tests: `crates/yavs/tests/drives.rs` (`a_record_default_that_is_overridden_is_driven_once`: 12 designs with whole
records, fields before and after, elements of arrays of records, slices, a procedure call as barrier; output and exit status equal with
and without the pass, packs on and off, and the pass saves a kernel drive or an interpreter operation in the 7 that must; it fails on the
previous `drives.rs`).

Host instructions (`perf stat` `instructions:u`, `--threads 1`, bench builds, warm world), main / with the change, G; interpreter on
855aefd8, llvm on a4afa75d (the earlier measurement, before the commits between):

| design | interp | llvm (compile thread included, spread about 0.3 G) |
|---|---|---|
| `neorv32_single_min` | 37.66 / 36.99 (-1.8 %) | 51.1 / 50.6 (-1.0 %) |
| `grlib_leon3mp` | 663.0 / 654.8 (-1.2 %) | 738.2 / 730.5 (-1.0 %) |
| `neorv32_single_cache` | 30.02 / 30.01 | 48.1 / 48.0 |
| `neorv32_dual_full` | 25.21 / 25.21 | 43.8 / 43.8 |
| `fletcher_bufferwriter_512` | 61.48 / 61.47 | 78.4 / 78.4 |
| `fletcher_bufferreader` | 22.64 / 22.64 | 29.9 / 29.9 |
| `sc5_fifo_repack`, `sc5_timestamp` | 0.0 % | 0.0 % |

(Binaries built from different commits differ by up to 0.35 percent in instructions with the same executed IR and kernel counters, in
`memmove` of the same copies; `fletcher_bufferreader` showed -0.34 percent with a commit that executes the same operations as main. Read
single-design changes below 0.5 percent as noise. `neorv32_single_cache`, `neorv32_dual_full` and the fletcher designs gain nothing: their
repeated drives are not record defaults of the shapes the pass handles, or there are none; not analysed further.) Checked: the 520-test differential corpus on `--tier=interp`, `llvm` and `auto` at 1 and
4 threads, the eight designs' stdout, stderr and exit status equal to main on `--tier=interp` at 1 and 4 threads, `cargo test --workspace`
and `-p yavs-llvm -p yavs --features llvm`.

What is left of #667: repeats behind a join (tail duplication needs the registers of the duplicated tail renamed: not cheap, 33 static
sites in `neorv32_single_min` have a kill behind a join), the leaf a field drive overrides in a scatter (a mask would split the pack
run: argued, not built, not measured), parts picked by registers that are equal but computed twice (value numbering), a default that reads a signal (`r <= other`).

## llvm: scalar drives kept for one host call (#676, 2026-10-24)

A process that assigns the elements of a vector one by one in a loop (`state(i) <= x`, the usual shift register, bit reversal or sorting
network) no longer makes a callback per element under `--tier=auto` and `--tier=llvm`: compiled code appends each zero-delay scalar logic drive
to a stack buffer and hands the buffer to the kernel in one call before it waits, calls, drives another way or leaves the function
(`docs/decisions.md` "Compiled clocked RTL processes, round 7"). Bench builds (`tools/build-bench.sh --features llvm`) of main before and
after, `--tier=auto` warm, core 12, user instructions and cycles of the process tree:

| test | instructions before | after | cycles before (median) | after |
|---|---:|---:|---:|---:|
| `hdlm_lfsr` | 28.09 G | 25.69 G (-8.5 %) | 8.53 G | 8.01 G (-6 %) |

Instructions over all 42 realbench tests: geometric mean 0.99 (CPU time 0.99; the changes beyond 1 % elsewhere are `--tier=auto` noise,
#673). The interpreter is unchanged (a deferred buffer there would save about 3 percent of `hdlm_lfsr`, recorded in the decision).

## Facts the declarative part leaves in a process copy (#678, 2026-10-24)

A copy of a process (#651) takes over between two activations, so what the declarative part made is in the frame and is a function of the
instance's data. `opt/decl.rs` runs that part per copy on an abstract machine and hands the walk the frame bytes, the address words and the
descriptors of the record variables' heap objects that the body never writes (`docs/decisions.md` "Facts the declarative part leaves in a
process copy"). The body's `DopeLen`, `Pos`, `SlicePos`, `SubtypeCheck`, descriptor loads, `LayoutOf` and `Field` of a process variable fold
like those of an object in the constant pool; `LayoutOf` also folds in shared code whenever the descriptor is a pool constant or instance data.

Bench builds with `llvm` (`tools/build-bench.sh`), `--threads 1`, user instructions of the whole process in G (`perf stat -e
instructions:u`, equal to 0.02 percent between runs), warm compilation cache (two untimed runs first), main at 0694dbe1 before / after:

| design | `--tier=interp` before / after |
|---|---|
| `fletcher_bufferwriter_512` | 83.23 / 75.72 (-9.0 %) |
| `grlib_leon3mp`, whole run (1060.5 us) | 781.65 / 744.47 (-4.8 %) |
| `grlib_leon3mp`, first 100 us | 135.83 / 133.66 (-1.6 %) |
| `neorv32_dual_full` (200 us), `sc5_fifo_repack`, `poc_cache_cpu` | within 0.05 % |

`--tier=llvm` is unchanged by design (no copies exist when the program is compiled before the run: `poc_cache_cpu` 16.901 / 16.897);
under `--tier=auto` the copies (and their gain) exist, but instruction counts there are not deterministic (the compile thread), so none are
claimed. The IR of the copy of the LEON3 integer unit's `comb` (`iu3.vhd`, interpreter): 9 656 to 8 539 instructions, `LayoutOf` 551 to 0,
`Field` 653 to 0; the copy of `dctrl`: 13 258 to 12 025, `LayoutOf` 372 to 10. The shared code of that process is 18 378 instructions
(1 235 `LayoutOf`). Initialisation and the cold (`--no-cache`) cost of making the copies are within 1 percent (41 copies of the LEON3
system, 100 us: 145.44 G before, 143.68 G after). `spec_record_var` (`tests/perf.rs`): 239 313 ops with the shared code, 151 530 with copies.

Not done: processes that branch on a package constant in the declarative part (6 of the 16 large processes of the LEON3 system:
`AddrSpace::Pkg` reads are not instance facts and the copies are stored across runs) keep their `LayoutOf`; a value a function call made
is unknown (its descriptor is not when the bounds are known).

Checked: output of `fletcher_bufferwriter_512`, `sc5_fifo_repack`, `poc_cache_cpu`, `neorv32_dual_full` (100 us) and `grlib_leon3mp` (300 us)
byte-identical (stdout and stderr) with copies off and with every process switching at its second activation; `cargo test --workspace`
plain and with `YAVS_SPEC_AFTER=2`; `-p yavs-llvm -p yavs --features llvm`.

## llvm: light natives get a callback of their own (#698, 2026-10-24)

`yavs_rt_native_light` / `TierHost::native_light` (decisions.md "Light natives in compiled code") for the `ieee` natives
`Op::CallIeee` runs. `tools/stepcount`, `poc_arith_addw` `--tier=llvm`, 3 M-instruction window, 1078 calls: host side 113 to 82
instructions per call, callback 68 to 73 (it gained `forget_end`, lost the epoch read); the native itself 45 to 115. Base and
candidate bench builds, core 12, `--compare`:

| test, tier | instructions before | after |
|---|---:|---:|
| `poc_arith_addw` llvm | 210.42 G | 207.93 G (-1.2 %) |
| `poc_arith_addw` auto | 150.99 G | 148.93 G (-1.4 %) |
| `vai_window_gen_interlock` llvm | 234.33 G | 232.48 G (-0.8 %) |

The layer (about 155 instructions a call) is still as large as the native: it is spread over two prologues and epilogues, the table
lookups, the result tuple and the argument slice. The remaining step is the native inline in compiled code.

### `--tier=auto` decisions (yavs-llvm, #670, #673, #680, #681, 2026-10-24)

Causes, the change and what is left: `docs/decisions.md` "`--tier=auto` decisions follow the looks of a sampler thread, prices are fixed".
Method: a fresh world per sample (copy of the pristine world), `--threads 1` (the default 4 threads spin, 1.4 percent noise in
instructions of the interpreter alone), `perf stat -e instructions:u` of the whole tree, warm = two untimed runs then the measured one,
cold = the measured run in an empty compilation cache, 8 worlds per cell, the two binaries interleaved per world. Before = main with
only the affinity fix (`m0`), after = this change (`v10`), same base. P1 = pinned to core 8 (the compile thread shares it), P2 = a mask
of two CCD0 cores that other jobs share (instructions only; the host was loaded, load 20 to 30). Median G instructions, spread =
(max - min) / median in percent, median wall seconds (P1 only; noisy).

| Design, run | P1 before | P1 after | P2 before | P2 after |
|---|---|---|---|---|
| `sc5_fifo_repack` cold | 42.85 (0.2%) 2.74 s | 42.89 (0.8%) 2.73 s | 43.19 (8.3%) | 42.87 (7.4%) |
| `sc5_fifo_repack` warm | 41.01 (3.9%) 2.52 s | 41.02 (0.1%) 2.50 s | 37.52 (13.2%) | 38.37 (19.7%) |
| `poc_arith_addw` cold | 196.4 (1.2%) 16.1 s | 185.7 (2.5%) 16.4 s | 184.7 (8.2%) | 181.7 (5.5%) |
| `poc_arith_addw` warm | 150.3 (0.4%) 11.9 s | 150.3 (0.4%) 11.3 s | 150.2 (0.8%) | 150.0 (0.9%) |
| `vai_window_gen_interlock` cold | 246.6 (3.3%) 17.1 s | 254.4 (4.3%) 18.3 s | 245.8 (4.0%) | 254.1 (1.5%) |
| `vai_window_gen_interlock` warm | 202.0 (8.0%) 14.1 s | 187.3 (10.3%) 12.8 s | 179.2 (10.7%) | 187.0 (12.3%) |
| `hdlm_lfsr` cold / warm | 29.54 (4.3%) / 25.48 (0.2%) | 29.57 (4.7%) / 25.50 (0.2%) | 28.63 (7.8%) / 25.52 (6.0%) | 28.70 (6.8%) / 26.29 (6.1%) |
| `neorv32_single_min` cold / warm | 40.75 (0.1%) / 38.71 (0.1%) | 40.75 (0.0%) / 38.71 (0.0%) | 40.76 / 38.72 (0.1%) | 40.76 / 38.73 (0.1%) |

The spread the issues describe (10 to 80 percent) came mostly from the affinity bug: with the compile thread free on another core
(2 CPUs allowed, before #681) `sc5_fifo_repack` warm was 40.2 to 45.5 G (12.5%), the window generator warm 16.5%, `hdlm_lfsr` 6.5%; with the
thread on the simulation's core (#681) the same binary gives 0.2% for `sc5_fifo_repack` warm. What this change adds: the decisions
of a run are the same function of the looks whatever the compile threads do (unit tests), the learned rate that made `sc5_fifo_repack`
bimodal is gone, what `p_in` costs is a constant, and `Fast` code is priced for the budget. What it does not remove: the CPU speed of a loaded host (the same window generator run saw
11.0 s or 13.4 s of simulation CPU for the same work) moves the time at which functions near their thresholds are compiled, and the
compilation cache amplifies a flip in the first run (window generator warm: the three worlds with 195 to 198 G are those whose second run
linked 61 to 65 modules where the others linked 96 to 99).

Calibration data (cold, `YAVS_AUTO_TRACE` lines "cpu"): CPU per IR instruction of one compile, by module: frame-mode code 210 (lean)
to 340 (full pipeline), native-frame code 445 (lean) to 620 (full), `Fast` code 90 to 115, cache hit 12 (frame) and 26 (native); native-frame
compile time per IR instruction grows with the size of the function (360 at 15 IR, 570 at 100, 1160 at 300).

### `--tier=auto` work clock, kinds of function, cached objects (yavs-llvm, #670, #673, #675, #717, #728, 2026-10-25)

Decisions and causes: `docs/decisions.md` "`--tier=auto` work clock, kinds of function, cached objects, shared code that waits for its copy".
Method: the bench builds of main (40cffec3, "before") and of this change ("after"), a fresh copy of the pristine world per sample,
default threads, pinned to core 8 (the compile thread shares it), `perf stat -e instructions:u` of the whole process tree, cold = the
first run of the world, warm = the third, 8 worlds per cell, the two binaries interleaved per world, `YAVS_AUTO_TRACE=1` for the
decisions. Loaded = a busy loop on core 8 and one on its SMT sibling for the whole sample. Cells: median G instructions (spread =
(max - min) / median), median wall, the median of the sampled work the policy saw (`sampled`, the clock of its decisions) and the number
of different sets of compiles (and trial verdicts) in the 8 worlds.

| design, run | idle before | idle after | loaded before | loaded after |
|---|---|---|---|---|
| `hdlm_lfsr` cold | 27.69 (6.8%), 1.87 s, 0.82 s, 2 | 27.66 (0.4%), 1.85 s, 1.13 s, 2 | 27.21 (2.2%), 5.14 s, 1.10 s, 1 | 27.53 (0.9%), 5.11 s, 1.23 s, 1 |
| `hdlm_lfsr` warm | 25.14 (6.4%), 1.75 s, 0.73 s, 1 | 25.12 (0.5%), 1.66 s, 1.00 s, 1 | 24.34 (3.1%), 4.56 s, 1.02 s, 2 | 24.03 (2.0%), 4.54 s, 1.18 s, 1 |
| `sc5_fifo_repack` cold | 41.55 (1.5%), 2.65 s, 2.21 s, 1 | 41.47 (0.4%), 2.61 s, 2.46 s, 1 | 42.56 (4.9%), 7.78 s, -, 4 | 41.29 (0.3%), 7.51 s, 2.48 s, 1 |
| `sc5_fifo_repack` warm | 39.50 (0.2%), 2.42 s, 2.04 s, 1 | 39.50 (0.2%), 2.43 s, 2.36 s, 1 | 38.02 (13.2%), 6.80 s, 2.66 s, 3 | 39.21 (0.1%), 6.88 s, 2.37 s, 1 |
| `neorv32_single_min` cold | 37.09 (0.0%), 3.44 s, 1.80 s, 1 | 37.07 (0.0%), 3.36 s, 1.85 s, 1 | 37.16 (0.1%), 9.17 s, 2.38 s, 1 | 37.08 (0.0%), 9.18 s, 2.03 s, 1 |
| `neorv32_single_min` warm | 35.57 (0.0%), 3.21 s, 1.75 s, 1 | 35.55 (0.0%), 3.26 s, 1.86 s, 1 | 35.63 (0.0%), 8.81 s, 2.35 s, 1 | 35.55 (0.1%), 8.56 s, 2.00 s, 1 |
| `poc_arith_addw` cold | 121.95 (1.3%), 7.92 s, 4.32 s, 2 | 108.69 (1.2%), 7.35 s, 4.85 s, 1 | 116.06 (1.7%), 21.68 s, 6.25 s, 1 | 106.79 (1.3%), 20.16 s, 5.55 s, 2 |
| `poc_arith_addw` warm | 112.52 (7.9%), 7.31 s, 3.94 s, 2 | 92.80 (2.7%), 6.23 s, 4.02 s, 1 | 122.91 (9.5%), 23.04 s, 6.98 s, 2 | 91.68 (1.6%), 17.47 s, 4.71 s, 1 |
| `vai_window_gen_interlock` cold | 210.57 (3.8%), 14.37 s, 10.26 s, 8 | 204.09 (2.3%), 13.99 s, 10.81 s, 8 | 202.52 (1.9%), 36.65 s, 14.54 s, 8 | 202.98 (2.5%), 36.66 s, 10.83 s, 7 |
| `vai_window_gen_interlock` warm | 149.85 (3.6%), 9.80 s, 8.74 s, 8 | 148.89 (1.1%), 9.88 s, 9.02 s, 6 | 146.40 (6.5%), 27.94 s, 11.85 s, 8 | 150.82 (1.6%), 28.15 s, 9.16 s, 7 |

What the numbers say. The clock: the work the policy saw moves from idle to loaded by 0 to 18 percent after (window generator cold
10.81 s to 10.83 s; `hdlm_lfsr` and `poc_arith_addw` +8 to +18 percent: the sampler takes its looks at other points of the run) where
the CPU-time clock moved it by 32 to 45 percent (window generator cold 10.26 s to 14.55 s; the cell without a number is a run that
ended inside a compile, which the trace does not print). The spread of instructions over 8 worlds is 0.0 to 2.7 percent (before 0.0 to
13.2). The sets of compiles: `sc5_fifo_repack` 1 set idle and loaded (before 1 and 3 or 4), `poc_arith_addw` warm 1 and 1 (before 2 and
2). What is left in the window generator (6 to 8 sets in 8 worlds, also idle) is sampling: the functions that differ are within a few
milliseconds of seen time of their thresholds. Wall: `poc_arith_addw` cold -7 percent (loaded -7), warm -15 (loaded -24), from the
processes that switch to their copies before they are compiled (`awaits_switch`) and the carry-chain process (`RtlFew`); the rest
within the spread of the wall (3 to 8 percent on this host), at most 2 percent either way.

Head-to-head designs (`tools/bench/designs`, warm, wall median of 8, core 8): `rtl_ff` 0.665 s to 0.623 s, `rtl_lfsr` 0.288 s to 0.281 s,
`lfsr64` and `rtl_chain` unchanged. The window generator under the benefit table alone (clock of the base): cold -0.5 to +2.6 percent
wall over 16 worlds in four batches, warm -6 to +3. `rtl_lfsr` with the shared code of processes of 64 instances or more waiting: warm
0.283 s to 0.351 s, the reason of `SPEC_INSTANCES`.

All 42 of `tools/realbench` (`--compare`, `--tiers auto,auto-cold`, run phase, core 8, default runs): geometric mean of the wall
ratio after/before 1.00 (auto, warm) and 0.97 (auto-cold), of the instructions 0.99 and 0.98, sum of the walls 0.97 and 0.94; per
category within 0.94 to 1.04. Largest changes: `poc_arith_addw` 0.89 / 0.90 (instructions 0.84 / 0.89), `hdlm_axi_fifo` cold 0.89,
`olo_intf_i2c_master` cold 0.87. The tests that came out +7 to +16 percent in the first pass (`neorv32_single_min`,
`neorv32_single_cache`, `osvvm_axis_random`, `fletcher_bufferwriter_512`, equal instructions) were run again with 7 runs: 0.97, 1.03,
1.07, 1.02 warm and 1.00, 1.00, 1.02, 1.02 cold, in the spread of the host.

## IR optimiser round 14: a repeated `to_integer` is made once (#520, 2026-10-24)

`docs/decisions.md` "IR optimiser round 14". A check such as `simAssertion(to_integer(unsigned(c & s)) = x, "..." &
image(to_integer(unsigned(c & s))))` converts one vector twice. After the message sank into the failing branch (round 16)
the second argument (signal read, concatenation, fat pair) and the second native call were what was left of the message
on the passing path. `opt/sink/calls.rs` finds two `to_integer` calls of an `unsigned` on the same value in one block
(value numbering of scalars and of the blocks the run allocates; forgotten at statement markers, suspensions, unknown
calls and stores) and makes the later one a branch on the earlier result: a result other than 0 is shared, 0 (also what
a null vector or a metavalue returns, with a warning) runs the second call as it was, so every warning keeps its text,
count, order and time. A second sinking round moves the second argument next to the replayed call.

Measured on `poc_arith_addw` (aligned bench builds, main at e0b51352 against the change, core 10; `perf stat` user counters of
a 300 us window, two untimed runs and the median of 5 interleaved runs; full length with `tools/realbench`, 3 runs):

| | before | after | ratio |
|---|---:|---:|---:|
| window, instructions G / cycles G | 25.61 / 7.94 | 23.65 / 7.07 | 0.923 / 0.89 |
| full, `--tier=interp` instructions G, wall s | 220.8, 14.00 | 203.6, 13.27 | 0.92, 0.95 |
| full, `--tier=auto` warm, instructions G, wall s | 151.1, 11.05 | 140.8, 10.71 | 0.93, 0.97 |
| full, `--tier=auto` cold, wall s | 14.89 | 14.43 | 0.97 |

Against the reference simulator (5.78 s, same run, host load about 20): `--tier=auto` warm 1.91x before, 1.85x after; cold 2.58x
before, 2.50x after. The checker process is 20 percent of the samples of the profile now; the 32 adder instances are the rest.
Whole suite (`tools/realbench --compare`, 42 tests, `--tier=interp` and `--tier=llvm`, one run each, core 10): geometric mean of
the instructions ratio 1.00 in both, no test above 1.01 but `poc_arith_addw` (0.921 interp, 0.929 llvm) and two short llvm cells
(`uvvm_func_cov` 1.067, `uvvm_sb_slv` 1.033) whose instructions include the compile thread; five runs of those two: 0.99 to 1.01.


- Tests: `crates/yavs/tests/sink.rs` (every case with and without the optimiser; interpreter and, with the `llvm`
  feature, compiled code): vectors with `U`, all zeros, null, another warning between the two calls, three calls, signed
  (not shared), signals changing between statements; the dump has the replay. Random programs: 1300 programs of checks over vectors of 0 to 8 elements with `0 1 U X Z W L H -`, concatenations, slices, `signed` and `unsigned`, three calls of one value, another conversion between the two calls and signal changes between the checks, output identical to the build without the optimiser (interpreter, and 350 of them under `--tier=llvm`).
- Left: signed `to_integer` (warns with a nonzero result for a one-element `'1'`); the other `numeric_std` natives (their
  result is a metavalue, not a sentinel the guard can test); calls in different blocks; the first call's own argument
  code (`AllocReadVec`, `SigReadVec`, `NewDope`, `MakeDope`, `AllocCat`, `Cat`, `NewPair`, two stores per call) is the
  calling convention of the natives (#697). A native whose arguments are not one fat pair (#697) is not shared until
  `shareable` and the pair test in `find_pairs` know its form.

## Memo of pure calls (yavs-ir `memo_fn.rs`, yavs-interp `pmemo.rs`, #701, 2026-10-24)

Calls of pure functions on plain scalars and vectors are answered from a per-process memo when their arguments repeat (decisions.md
"Memo of pure calls"). Measured with bench builds (`tools/build-bench.sh --features llvm`, main at 4d53f7a4 against main plus the memo),
`perf stat -e instructions:u`, no timing cores (instructions only: the cycle geomean the interpreter rule asks for is not measured),
`--threads 1`:

| Run | before | after | |
|---|---:|---:|---|
| `poc_cache_cpu` interp (`ffdre`, `mux`, `resize` on 2-bit vectors; 688 000 hits, 17 000 runs) | 14.32 G | 8.66 G | -40 % |
| `poc_cache_cpu` auto | 13.45 G | 8.96 G | -33 % |
| `grlib_leon3mp` 100 us, interp (10.9 M hits, 288 000 runs) | 137.47 G | 116.85 G | -15.0 % |
| `grlib_leon3mp` whole run, interp (14.9 M hits, 421 000 runs) | 673.55 G | 631.39 G | -6.3 % |
| `fletcher_bufferwriter_512` interp | 61.50 G | 59.58 G | -3 % |
| `neorv32_single_min` / `neorv32_dual_full` interp | 37.71 / 25.28 G | 37.18 / 23.94 G | -1 % / -5 % |
| `osvvm_axis_random`, `uvvm_sb_slv`, `uvvm_util_rand`, `olo_fix_cordic_rot` interp | 3.56, 12.73, 14.48, 3.62 G | 2.91, 10.75, 12.41, 2.88 G | -18, -16, -14, -20 % |

The 41 tests of `tools/realbench` (all but `grlib_leon3mp`, measured above, and the excluded one), `--compare`, one counted run each:
geometric mean of the instruction ratio 0.93 for `interp` and 0.93 for `auto` (wall and CPU ratios are not meaningful, the host was
loaded). The internal and head-to-head designs of `tools/bench` (25, `interp`): 0.994, the largest change `calls` 0.96, none above
1.005. An instruction count from one run after a change of the compilation cache can differ by tens of percent for a short test
(`en_cl_fix_pkg` read 1.39 in the first run and 0.92 in the steady state): look at the steady state before believing a ratio above 1.

What the memo does not reach: calls that compiled code makes through the host's step (a callee without native code), records, access
types and out parameters, and elaboration-time calls. Idea taken from the reference simulator: folding
a pure call with constant arguments once (here a runtime memo, because the PoC registers' arguments are values that repeat, not constants);
ours differs in keeping nothing across runs and verifying every hit by the argument bytes.

### Memo of pure calls from compiled code (#701, 2026-10-24)

Calls from compiled code to a native-callable function with a memo signature probe the memo through a callback before the call and keep
the result through a second callback after it returns (decisions.md "Memo of pure calls", "Compiled code"); the interpreter also asks the
memo before it offers a call to the compiled tier (`Tier::call`). No interpreter code runs inside a host step. Measured with bench builds
(`tools/build-bench.sh --features llvm`, origin/main against main plus this change, `perf stat -e instructions:u`, `--threads 1`,
instructions only, steady state: each binary run twice before the counted run, since a first run after a change of the binary fills the
stage cache and counts tens of percent more). `--tier=auto` instruction counts vary by tens of percent between identical runs (the compile
thread's timing), so only `interp` and `llvm` are quoted:

| Run | before | after | |
|---|---:|---:|---|
| `poc_cache_cpu` llvm (`ffdre`, `mux`, `resize`; 688 000 hits, 16 600 runs, as `interp`) | 15.86 G | 12.13 G | -23.5 % |
| `poc_cache_cpu` interp | 8.38 G | 8.38 G | 0 |
| `grlib_leon3mp` 100 us llvm | 149.03 G | 145.09 G | -2.6 % |
| `grlib_leon3mp` 100 us interp | 105.26 G | 105.29 G | 0 |
| `vai_window_gen_interlock` llvm / interp | 217.89 / 447.50 G | 191.25 / 447.76 G | -12.2 % / 0 |
| `osvvm_axis_random`, `uvvm_sb_slv`, `uvvm_util_rand` llvm | 15.75, 23.68, 42.53 G | 15.35, 23.04, 42.02 G | -2.5, -2.7, -1.2 % |
| `neorv32_dual_full`, `neorv32_single_min`, `fletcher_bufferwriter_512` llvm | 42.21, 48.53, 64.56 G | 41.35, 48.30, 63.49 G | -2.0, -0.5, -1.7 % |
| `poc_arith_addw` llvm / interp | 134.83 / 154.39 G | 134.61 / 154.45 G | 0 |
| a memoisable function whose arguments never repeat (200 000 calls, memo gives up after 64) llvm / interp | 2.57 / 4.64 G | 2.45 / 4.64 G | 0 |

`poc_cache_cpu` under `llvm` was 1.9 times the interpreter's count (the compiled calls bypassed the memo) and is now 1.45 times; the rest of
the difference was not analysed (the llvm count includes compiling the functions). `pmemo_hits` under llvm equals the interpreter's.
The cost of a probe that cannot hit (a function the memo gave up on, still asked on every compiled call) is not visible in the last row;
an inline test of the global off flag was not built. A bug found by the window generator testbench (CI `generics` would not have seen
it): the keep callback left `Mem::temp_seen` behind the length it had synced, so a later callback skipped the sync and `native_step`
read past the end of the machine's temporary area.

## Package constants in specialised copies (yavs-ir `copies.rs`, `opt/decl.rs`, #708, 2026-10-24)

The declarative part of a process copy is run on the abstract machine of #678; a load from `AddrSpace::Pkg` (a package constant, written during
initialisation only) now gives its value, taken from the package data the runtime hands `copy_of_pkgs`, and a digest of the data of the packages
the part names is in the copy's key (decisions.md "Package constants in specialised copies").

GRLIB LEON3 (`grlib_leon3mp`, `YAVS_SPEC_AFTER=2 YAVS_SPEC_CAP=0`, 2 us, the copies of the `comb` processes that branch on a package constant;
IR instructions of the copy and `LayoutOf` / `Field` in it, main against main plus this change):

| copy | before | after |
|---|---:|---:|
| `comb` 1214 | 3526, 20 / 19 | 3450, 0 / 0 |
| `comb` 1251 | 3645, 47 / 45 | 3432, 0 / 0 |
| `comb` 1252 | 3165, 48 / 41 | 3060, 42 / 38 |
| `comb` 1253 | 4459, 200 / 156 | 3871, 9 / 20 |
| `comb` 1259 | 2947, 47 / 42 | 2597, 1 / 0 |

Instructions of the whole run (interp, memo of #701 off in both columns, so only this change and the call arm of #701 differ): 100 us 137.47 G
to 137.79 G (+0.2 %), 1060.5 us 673.55 G to 670.57 G (-0.4 %): the processes it helps are a small part of the run, the memo of #701 is the
larger change there. `comb` 1252 keeps 42 `LayoutOf` (its cause was not looked into); the unit case is
`a_copy_knows_what_a_package_constant_made` (`crates/yavs-ir/tests/copies.rs`).

## Native `ieee` calls take array operands as values (#697, 2026-10-24)

(`docs/decisions.md` "Native ieee calls: array operands as values".) A call of a `numeric_std`, `std_logic_1164`, `numeric_bit` or
`math_real` native used to pass each array as the address of a fat pair built in temporary memory (`NewPair`, `FatRoot`, `NewDope`) that the
native read back through three dependent reads. An array argument is now four registers (`data, left, right, dir`), the verifier checks the
count from the symbol's key, lowering takes the bounds from the registers the descriptor was made from (else loads them), the write-only
descriptor goes away in the existing dead-code sweep, `Op::CallIeee` takes 8 cells and `ieee::small` reads only the planes. Other natives keep the
fat pair; results stay a fat pair. Bench builds (`tools/build-bench.sh --features llvm`) of main before and after, warm cache, core 14:

| test | instructions before | after | cycles before | after |
|---|---:|---:|---:|---:|
| `poc_arith_addw` interp | 220.5 G | 202.7 G (-8.1 %) | 67.0 G | 61.7 G (-7.9 %) |
| `poc_arith_addw` `--tier=auto` | 150.4 G | 139.2 G (-7.5 %) | 54.6 G | 48.9 G (-10 %) |
| `poc_arith_addw` `--tier=llvm` | 210.2 G | 196.7 G (-6.4 %) | | |
| `neorv32_single_min` interp / llvm | 38.3 G / 52.6 G | 37.8 G / 51.3 G | | |

`realbench --compare` wall, `poc_arith_addw`: interp 14.12 s to 12.66 s, llvm 15.96 s to 15.38 s, auto 10.99 s to 10.07 s. `poc_cache_cpu`,
`sc5_fifo_repack`, `uvvm_util_rand` and `en_cl_fix_pkg` move by less than 0.5 percent (few `ieee` calls; the PoC `ffdre` cost is #701). All 42
tests, interp and llvm: instruction geomean 1.00 and 0.99, no ratio above 1.02. The differential corpus (510 designs; interp, llvm, auto; 1 and 4
threads), `cargo test --workspace` and `-p yavs-llvm -p yavs --features llvm` are green; output is byte-identical.

Against main with the sharing of the repeated `to_integer` (#520, adapted to the value form; the optimiser now also gives the bounds of a
freshly made descriptor back without loads), `realbench --compare`, core 14: `poc_arith_addw` wall interp 12.19 s to 11.48 s (instructions
-7 %), llvm -6 %, auto 10.09 s to 9.33 s (-7 %); `poc_cache_cpu` llvm -7 % instructions, `neorv32_single_min` -1 to -3 %; the others within 1 %.

What is left: a result chained into the next native (`to_integer (a + b)`) loads the three bounds of its descriptor (a three-word load op, or
natives returning the bounds in registers, would remove that); the callback layer of the compiled tier is #698; sharing the two `to_integer`
of a check is #520.

## llvm: light `numeric_std` natives computed in compiled code (#698, 2026-10-24)

`Gen::ieee_fast` (decisions.md "Light `numeric_std` natives computed in compiled code") on top of the operands as values (#697):
the arithmetic of `ieee::small` and `put_small` emitted at the call site for 1 to 64 elements without a metavalue, the callback of
the same site for everything else. Bench builds, core 12, `--compare` of main with #697 against this change, median of 3, wall seconds
(instructions in brackets, G):

| test | auto before | auto after | llvm before | llvm after |
|---|---:|---:|---:|---:|
| `poc_arith_addw` | 9.81 (136.5) | 8.49 (118.2) -13 % | 13.13 (191.6) | 11.74 (174.8) -11 % |
| `vai_window_gen_interlock` | 11.12 (182.2) | 10.56 (179.1) -5 % | 15.11 (225.3) | 14.63 (219.6) -3 % |
| `poc_cache_cpu` | 0.96 (12.35) | 0.90 (12.07) -6 % | 1.47 (16.28) | 1.46 (15.98) |
| `neorv32_single_cache` | 2.73 (30.0) | 2.72 (30.0) | 5.28 (46.9) | 5.20 (46.6) |
| `sc5_fifo_repack`, `sc5_timestamp` | level | level | level | level |

Cold compile cost (empty compilation cache, `--tier=llvm`, run stopped at 1 us, instructions): `poc_arith_addw` world +1.0 %
(29.6 G to 29.9 G), the window generator +0.3 %. The numeric natives of `poc_arith_addw` are gone from the profile; what is left
there is the kernel, signal reads and concatenations. In the window generator the natives were a small share under
`--tier=llvm` (about 3 percent of the instructions); the callbacks of its VHDL codec functions, `get`, `set`, `decode`, remain
the bulk. The `cb_host` rise of `llvm_ieee` (1416 to 2019 for 1695 natives saved) is explained in "`yavs_rt_host` calls of frame-mode code" below (#711); `cb_native` of that design 1695 to 5.

### Inlining and the verifier (#709, 2026-10-24)

The moves and jumps inlining adds at the site of a call that is itself inlined code kept `via` 0 (#709); a debug build's
IR verifier rejected `procChecker` of `poc_arith_addw` ("statement belongs to another function") and the design could not
run in a debug build. Fixed in `opt/inline.rs`; release builds ran the design unchanged. Regression test in `tests/sink.rs`.

## llvm: #710 fixed, liveness per basic block, the deferral cap lifted; the optimised register allocator measured (#710, #679, 2026-10-24)

- **#710 (a cached `LayoutOf` read the register's stale deferred value after its slow path stepped the instruction) is fixed** with a regression test
  (`cached_layout_replaces_a_deferred_value_of_its_register`); the cap that hid it from the large processes is lifted: `Live::new` is per
  basic block (no 64 passes over every instruction: 11.8 percent of a warm-cache LEON3 run's instructions with the cap raised on the old code,
  nothing now), `MAX_WORDS` is a memory guard of 1 << 25 words. Instructions of the LEON3 comb window -0.3 percent, the other four designs
  level (decisions.md "llvm: liveness per basic block ...").
- **The argument reloads of #679 are fast-register-allocator spills at the 20 000 block boundaries of the comb copy**, 5.8 percent of its instructions,
  not something the IR can mark away. With the optimised register allocation path (`-optimize-regalloc -regalloc=greedy`, an LLVM option) the
  LEON3 window runs -9.0 percent instructions and -12.8 percent cycles (neorv32, the two fletcher designs -6 to -7 percent cycles), at about 10 s more code generation for that function. Not enabled: a
  compile-cost policy for the hot large functions is the user's decision (#715).
- Checks: difftest 520/520 at 1 and 4 threads for `yavs-interp`, `yavs-llvm`, `yavs-auto`, and `yavs-llvm`/`yavs-auto` with `YAVS_LLVM_ARENA=0`.

### A shared constant keeps its definition (#714, 2026-10-24)

`kill_unreachable` no longer removes the only definition of a constant or address another block reads; before, inlining
a procedure with a constant argument could leave such a register undefined (VUnit `exit_gate` wrote element 0 instead of
20; a debug build stopped with "read before it is written"). `vai_window_gen_interlock` runs in a debug build again.

## poc_arith_addw: carry-in aggregates, slice reads, conversion copies, copies compiled once (#713, #520, 2026-10-24)

Where the 32 adder instances of `poc_arith_addw` spent their time (main at 855aefd8, `--tier=auto` warm 10 s, core 10), by layer:
kernel 26 percent (diffuse: `finish_direct`, `cycle`, `run_batch`, `commit_out`), interpreted processes 33 percent, compiled processes
29 percent (the adder bodies and the checker), the rest the `ieee` natives and the glue between interpreter and compiled code. By
process (a per-process run counter, 40.1 M runs in 3.03 M delta cycles): the `z <= ... + ... + (0 to 0 => cin)` processes 47 percent
of the runs, the `s <= std_logic_vector(z(...))` copies 31 percent, the checker 1.3 percent (20 percent of the interpreted time).

What was changed (decisions: `docs/decisions.md` "Carry-in aggregates and copies of the same module"):

| change | interpreter, 400 us window, instructions G | `--tier=auto` |
|---|---:|---|
| main at 855aefd8 | 31.4 | warm 10.08 s, cold 13.41 s |
| conversion copies are kernel copies | 27.4 (with the aggregate) | |
| `(0 to 0 => cin)` straight code: no compute loop, so `LEAN` passes (58 to 6 ms a function), `CopyDrive` fuses | 27.4 | warm 9.10 s, cold 11.20 s |
| copies of one module compiled once (24 + 8 adder copies are 2 + 1 modules) | 27.4 | warm 8.41 s, cold 9.41 s |
| slices of signals read as slices (no whole-signal read and `LCopy`) | 26.2 | |

Rows are cumulative; wall seconds are medians of 5 interleaved runs of `tools/realbench --compare` (aligned bench builds with llvm, core
10, the host about 20 loaded), the interpreter column is `perf stat` instructions of a 400 us window. Operations of the interpreter per
activation of the adder process: 60 to 46. `perf.rs` design `agg_carry`: 11756 to 7627 operations.

On main at 479463c1 (with #697 and #698 landed) against the same plus these changes, `tools/realbench --compare`, median of 5:
`--tier=interp` 11.32 s to 9.09 s (instructions 0.82), `--tier=auto` warm 8.13 s to 7.05 s, cold 10.98 s to 7.57 s (instructions
0.99 and 0.81). Against the reference simulator (5.47 s, same session): `--tier=auto` warm 1.51x to 1.31x, cold 2.03x to 1.44x (it was
1.85x and 2.50x before #697, #698 and this).

Whole suite (42 tests of `tools/realbench`, `--compare`, `--tier=interp` and `--tier=auto`, median of 3, core 10, main at 479463c1
against the same plus these changes): geometric mean of wall 0.97 interpreter, 1.00 `--tier=auto`; instructions 1.00 and 1.03 (the
`--tier=auto` mean moves with tests of 0.03 s whose instructions include the compile thread; the tests of 1 s and more: 0.97 and
1.01). `neorv32_single_cache` (1.43) and `osvvm_axis_random` (1.34) were slower in the sweep and equal on 7 more runs of each (1.00,
1.00). Differential corpus 523/523 on `yavs-interp`, `yavs-auto` and `yavs-llvm`; `cargo test --workspace` and `-p yavs-llvm -p yavs
--features llvm` green.

Found, not fixed (comments on #697, #698, #710, #675):
- The loop-free carry-chain adder process (8 copies, 6.3 M runs) is an `rtl` process and `cold()` refuses it at the decision (share
  under one eighth); it stays interpreted and nothing says so. Forcing it saves 0.3 to 0.5 s of 8.4 s (#675).
- The checker process is compiled at 4.7 s of a cold run, after the adder copies (a warm run compiles it first, 218 ms).
- Entry into compiled code costs about 217 instructions per activation (`enter`, `view_again`, objects of the thread), against about
  500 instructions of the adder body (#710 lane).
- The kernel is 26 percent of the run with no single hot spot (`finish_direct` 2984 instructions per delta cycle in 11 pieces).
- Ideas of the reference simulator used: it reuses the lowered code of instances that are equal (cloned instances at elaboration);
  we do the same after specialisation, on the generated module. Its profile on this test: 20 percent in the compiler, 30 percent in
  generated code, the rest in a runtime whose `+` and `to_integer` are natives as ours are; nothing of its code was used.

### `yavs_rt_host` calls of frame-mode code (#711, 2026-10-24)

The calls did not come from the native frame that reads the by-reference parameters but from the frame-mode code of the
process: `llvm_ieee` 2116 of 2119. Spaces: the temporary area (the fat pair a call or native returned, the words
loaded from it, dope and planes, layouts) and heap objects (process variables). Frame-mode `Gen::host` now tests a
guess (`Known::LikelyTemp`, `Known::LikelyHeap`, and `Known::Heap(None)`) inline and falls back to the call.
`cb_host` (`--tier=llvm`, debug build): `llvm_ieee` 2119 to 8 (bound added to `tests/perf.rs`), `poc_arith_addw`
343 248 to 26 313 (3 us), `sc5_fifo_repack` 659 398 to 35 600 (10 us), window generator 104 834 to 54 875 (2 us).
Cycles (core 12, medians of 5): `--tier=llvm` -0.8, -2.4 and -1.0 percent on those three, `--tier=auto` -3.0 percent on
`poc_arith_addw`, -1.4 on the window generator and level (within 1.3 percent) on `sc5_fifo_repack`, `uvvm_sb_slv`,
`osvvm_axis_random`, `neorv32_single_min`, `fletcher_bufferwriter_512`. The price is code: +2 to +13 percent LLVM
instructions under `--tier=llvm` (`docs/decisions.md` "Frame-mode code tests the temporary area ..."). Open: package
and protected-object addresses in native frames still end in `host_miss`.

## llvm: element access in native frames (#493, 2026-10-24)

(`docs/decisions.md` "llvm: element access in native frames, and what the cycles of a protected accessor are".) Three changes in
the code `Gen` makes for array elements and heap objects, each measured alone on the window generator and on `protected_queue`
(`--tier=llvm`, main-thread `perf stat`, core 9, bench builds; instructions G / cycles G):

| step | window generator | `protected_queue` |
|---|---:|---:|
| main at 2fbfd7ee | 218.39 / 76.6 | 5.329 / 1.361 |
| `Pos` with one arm per direction | 214.33 / 76.9 | 4.932 / 1.247 |
| + a multiplication that feeds an `Index` is one test | 211.54 / 76.1 | 4.748 / 1.229 |
| + entries of the object table one cache line, key from the whole value | 210.98 / 73.9 | 4.655 / 1.165 |

`--tier=auto`, window generator, a private world after 12 untimed runs (fewer compare a settled build with an unsettled one:
with 4 untimed runs the same two builds differed by 10 percent in the other direction), 5 interleaved runs: 159.8 to 152.1 G
instructions, 50.6 to 48.3 G cycles. `tools/realbench --compare` warm: wall 11.77 to 10.84 s, cold 15.69 to 15.75 s (instructions
0.96); instructions against the reference simulator warm 2.47 to 2.26 times, cold 3.31 to 3.12. Seventeen tests that use the test
runner's queues, the other verification libraries' queues and scoreboards, `neorv32_single_min`, `poc_arith_addw`
and the wide-vector designs: instructions within 1 percent, wall within the spread. Difftest 520 of 520 on `yavs-interp`,
`yavs-llvm`, `yavs-auto` and `yavs-auto` with `--tier-heat=1,0,0`, each at 1 and 4 threads.

Not taken, with the numbers in decisions.md: LLVM inlining of the native callee (instructions -2.5 to -4.7 percent, cycles level), an
inline limit above 60 in the IR inliner, the full `O2` pipeline for the accessors. A protected `get` waits for a chain of dependent
loads (four object look-ups, three positions); the call and return are about 55 of its instructions and overlap with the chain.

## llvm: FastRa, the optimised register allocator for hot large frame-mode functions (#715, 2026-10-24)

- `OptLevel::FastRa` (user decision 2026-10-24): `Fast` code generation with the greedy register allocator, made per target machine
  (level `Less`, FastISel on, functions `optnone` after the passes), not by a process-wide LLVM option. A frame-mode function that
  `--tier=auto` compiled at `Fast` is compiled again in the background once it was seen running compiled for `PAYBACK_RA` (12) times the
  modelled cost of that compile (`ra_cost`, about 17 s of CPU time for a 17 500-instruction process); the O2 recompile queue carries the
  request. With `--tier-heat` every second compile of a function of 300 IR instructions or more is `FastRa` (it was a second `Fast` compile).
  Details, the rejected ways (the LLVM options; `Less` without `optnone`) and the cost table: decisions.md "llvm: `OptLevel::FastRa` ...".
- Forced (every function of 300 IR instructions or more compiled `FastRa` after `Fast`, `--tier=auto --tier-heat=1,0,0`, bench builds, core 11,
  medians of 5 interleaved, simulation thread, warm): cycles `neorv32_single_min` -3.8 percent, `fletcher_bufferreader` -5.0,
  `fletcher_bufferwriter_512` -4.3, `grlib_leon3mp` (whole run) -7.8, `vai_window_gen_interlock` -2.0, `sc5_fifo_repack` +0.6 and
  `poc_arith_addw` +1.7 (nothing over the size: noise); the LEON3 window alone (#715): -12.8 percent with the option route.
- Default `--tier=auto` (`realbench --compare`, `--phases run --tiers auto,auto-cold`, 7 tests, medians of 5, core 11, main at 3df7bc44 against
  the same plus this): geometric mean of wall 1.00 warm and 0.99 cold, instructions 0.99 and 1.00; per test warm `grlib_leon3mp` 1.03 (cold
  0.93), `fletcher_bufferwriter_512` 1.03, `vai_window_gen_interlock` 0.93, the others 0.99 to 1.01. The default runs compile nothing at `Fast` or
  `FastRa` in these tests (LEON3: 6 compiles, none of them over 300 instructions at `Fast`), so the differences are the host (load 12 to 15, other
  agents on the neighbouring cores). The gain is for runs that keep a big process compiled for minutes: with the threshold at 1 instead of 12 a
  LEON3 run with `YAVS_AUTO_FORCE` on its big `comb` processes compiled `f1222` (7 785 IR instructions) at `FastRa` after 4.8 s of compiled
  running, output identical.
- Checks: difftest 523/523 at 1 and 4 threads for `yavs-interp`, `yavs-llvm`, `yavs-auto`, `yavs-auto` with `--tier-heat=1,0,0`, and `yavs-llvm`,
  `yavs-auto`, `yavs-auto` with `--tier-heat=1,0,0` under `YAVS_LLVM_ARENA=0`; `cargo test --workspace` and `-p yavs-llvm -p yavs --features
  llvm` green; new tests `fast_ra_makes_smaller_correct_code`, `auto_tier_recompiles_a_big_function_at_fast_ra`,
  `a_fast_function_is_recompiled_when_the_register_allocator_repays`.
- Not done: a warm run waits for the payback time before it links the `FastRa` object from the cache; no trial after the swap (the first
  compile's trial measures the `Fast` code only); the price in `spent` is that of `Fast`; autoclk's pricing (#670) may want `ra_cost` and
  `PAYBACK_RA` in its own constants.

The object table has 1 024 entries (#719): the shared-heap and package host calls of the window generator were conflict misses; `--tier=auto` warm 149.3 to 148.3 G instructions, 44.1 to 43.4 G cycles (decisions.md "llvm: 1 024 entries in the object table").

## Short clocked RTL runs and the fixed cost of tiny tests (#725, 2026-10-25)

Two gaps of head-to-head 4 that no lane owned. Bench builds with `llvm`, tiny and designs on cores 13 and 14 (SMT siblings idle),
`tools/stepcount` for exact instructions, `perf stat` for the rest; the numbers are of cf616672 unless stated.

**Tiny corpus 5.3 to 5.7 ms per test (yavs warm).** Fixed. A bisect by user instructions and a look at `perf stat` found two
things. (1) Wall time, not CPU: since #670 the sampler thread slept in 1 ms steps and the end of the run joined it, so a short run
waited for the rest of a sleep (equal `task-clock`, longer elapsed time; `tests/corpus/delta/concurrent_assign.vhd` warm: 3.2 ms
elapsed to 1.9 ms). The compile thread now starts the sampler after `START_AFTER` and unparks and joins it when it ends (cf616672;
a run under 5 ms creates no sampler thread at all). (2) User instructions +171 k of 3.0 M (5.7 percent): the image decoding of a
bigger analysed unit (ae1826dd, 12a37bc7), 0.03 ms. Corpus (456 tests, warm analyze plus run, interleaved, geometric mean of the
per-test time): 4b80445d 1.00, head-to-head 4 build 1.065, with the fix 0.916; median test 5.1 ms to 4.3 ms. What is left of the
1.9 ms of a `run` is process start: `yavs --version` takes 1.1 ms against 0.67 ms for `true`; the kernel has 70 percent of the
cycles (exec, 435 page faults, thread creation); the dynamic loader is 0.05 ms by `LD_DEBUG=statistics`. Linking without PIE
saves 0.2 to 0.3 ms per process (-66 page faults, -7.7 percent of the corpus, no change in runs of 0.1 to 0.7 s): a decision for
the user, #727.

**h2h designs still slower than sim1 warm** (rtl_lfsr 1.63, lfsr64 1.44, rtl_ff 1.33, rtl_chain 1.20, medians of 10, total wall;
unchanged by this work). Nothing in the start-up dominates: analysis, elaboration and init are 2 to 4 percent of the wall of each
(`--time-report`), the clock generator is 2000 to 20000 edges. The simulate phase is the whole difference, per process activation:

| design | yavs instructions per activation | sim1 | where (exact, `--threads 1`) |
|---|---:|---:|---|
| rtl_ff (4096 one-bit processes) interpreted | 1511 (run_ops 930 for 12 ops, 17 per op in the dispatch alone) | 960 | kernel and drive about 400, run_quick and Process::run 170 |
| lfsr64 interpreted | 2460 (run_ops 1369, `cat_words` + `cat_block` 320) | 1300 | the slice is copied to a temporary and concatenated: three block copies per shift |
| rtl_lfsr, `--tier=llvm` | 1800 (compiled body 565) | 1100 | entry plumbing 620 (run_quick 175, enter 140, begin_kept 76, view_again 72, Process::run 60, load_objs 54, run_cached 28, save_objs 16), kernel and drive 550 |
| rtl_chain interpreted (per delta) | 1600 (run_ops 544 for 10 ops) | 1020 | kernel 830 (run_batch 193, finish_direct 176, wake_sole 118, cycle 113, assign_code_fast 96, commit_code 82) |

Wall of `--tier=auto` against all-compiled-at-first-call (`--tier-heat=1,0,0`, warm cache): rtl_ff 0.659 s to 0.593, lfsr64 0.170
to 0.143, rtl_lfsr 0.281 to 0.275, rtl_chain 0.188 to 0.202 (compiled is slower there). The default compiles rtl_ff at 370 ms of 647,
rtl_lfsr at 79 ms of 306 (refused at 23 ms by a price of 50.6 ms for a compile that is a 3 ms cache hit) and lfsr64 never: the
price of a function whose object is in the cache is the open question #728 (decision for the user). Every other lever is in a lane:
the kernel numbers are on #521, the policy on #675, the compiled-entry plumbing was taken to its floor for this structure in rounds
5 and 6 of #634 (a process-direct entry that skips the interpreter's `Machine`, `view` and the thread's object cache would be the next
step: about 400 of the 620 instructions, not started). Left alone on purpose: a `Cat` of a slice copied to a temporary (the optimiser
would have to give `Inst::Cat` parts an offset: cross-crate), unchecked register access (tried and dropped in round 10).
Not verified: cold tiny numbers after the change (only warm), Windows and aarch64 start-up, whether a non-PIE binary changes
`--tier=auto` instruction counts (it did not change the cycles of the head-to-head designs).

## performance: floor of a run, round 4 (#426, 2026-10-25)

The fixed cost of a tiny test after #725: no compile thread in a run under 5 ms, fewer system calls per cache entry (`docs/decisions.md` "Floor of a run, round 4").
Tiny corpus, every fifth test (100), warm, bench builds with `llvm`, the binaries copied fresh and interleaved per test, `perf stat -r 6`:

| per test, geometric mean against before | user instructions | kernel instructions | page faults | task-clock (hint) |
|---|---:|---:|---:|---:|
| `run` | 0.993 | 0.935 | 0.989 | 0.939 |
| `run`, `array_attrs` / `concat_chain` | 0.990 / 0.994 | 0.917 / 0.923 | 0.984 / 0.996 | 0.923 / 0.905 |
| `analyze` (warm) | 0.999 | 1.007 | 1.004 | 1.017 |

Median test 2.14 ms to 2.00 ms of CPU (cores 0-7, load 16: a hint; counts are what is reliable). `analyze` and `--version` do not start the threads this round removes.
What the rest of a tiny `run` is made of, and what was measured and rejected (heap pre-fault, section sorting, `arena_max`), is in the decision. The stack probe of the Rust
runtime (about 5 percent of a `run` and of an `analyze`) is a decision for the user, #738. Not verified: Windows and aarch64 start-up, cold numbers, a run between 5 and 6 ms
(where the compile thread now starts 5 ms in, from the signal thread: `--verbose` notes of `rtl_ff`, `compute_h2h` and `bench_lfsr64` show the same compiles as before).

## grlib_leon3mp whole-run profile and the unowned costs (#720, 2026-10-24)

Profile of `grlib_leon3mp` (std 1993) on main at 3df7bc44, `tools/build-bench.sh --features llvm`, `--tier=auto` warm, core 10 (the compile thread
shares it), `perf record --call-graph fp` (a bench build with `-C force-frame-pointers=yes`, seconds 20 to 36 of the run, `cycles:u`), `--profile`,
`YAVS_COUNTERS=1`, `op-census` (interpreter, the first 300 us), `YAVS_AUTO_TRACE=1`. The run is 57 to 59 s (49 s in the quiet runs below), 634 G
user instructions, IPC 2.1; the reference simulator needs 13 to 15 s and 171 G.

| layer (steady state, inclusive) | share of cycles |
|---|---:|
| kernel `update` (ports, effective values, commits) | 15 |
| processes on the hot path (`run_quick`) | 56 |
| processes on the cold path (`run_cold`: serial processes) | 18, of which activation overhead 5.5 (roots 3.8) |
| `--profile` view: kernel / processes | 21 / 79 |
| start-up (S-record load of the 12 memory models, first 1 us) | 8 (4.6 s; the reference simulator 1.5 s) |

By process (`--profile`): `iu:comb` 14.7 percent, the four SDRAM `state_register` 17.6 (they wake on a clock that toggles every 0.5 ns: 8.4 M wakes,
227 ops each; 90 percent are not a clock edge and assign 13 scalar and 3 vector signals the values they hold), `dctrl` 7.9, the two TLB processes 7.7,
`ictrl` 3.4, memory controller 2.3, AHB controller 2.2, MMU 2.1, the rest below 1.5 each.

- **Nothing big is compiled.** `YAVS_AUTO_TRACE`: `HRead` (247 and 36 IR) and `To_bitvector` are the only compiles of the run, 25 ms. The policy does not
  refuse the big processes (no refusal lines); `YAVS_AUTO_FORCE=state_register` makes the run slower (65.2 and 68.9 s against 56.9 and 58.8 s).
  That is #679 (frame-mode code of large processes does not execute fewer instructions than the interpreter) and the auto policy's #675, both
  reported there. The remaining gap to the reference simulator is the compiled code of the large processes, not these items.
- **92 percent of the 133 M direct code commits and 93 percent of the plane commits that went through `commit_planes` changed nothing**, nearly all of them
  the SDRAM models' tail drives.
- **Port edges**: 716 of 922 ports pushed in 200 us, 1.82 M pushes, all with one actual, one formal and no conversion; 1.65 M of them on `inout` nets (the
  data bus and the clock nets, about 30 formals each).
- Op census, steady state of 200 us: `Stmt` 9.7 percent of the ops, `SigVal` 7.0, `Aff1` 5.4, `CondBr` 4.5, `CmpBr` 4.3, `LSetDrive` 4.1, `LoadSlot8` 4.0,
  `SigRead1` 3.3, `Const` 3.1; per activation: `iu:comb` 4530 ops, `dctrl` 2442, `state_register` 227. `sig_gather`/`sig_scatter` visit
  about 19 leaves per call, 2.2 M and 2.4 M calls in 300 us (a quarter of the scatters with parts).

What changed (commits of #720; `docs/decisions.md` "Cold path of serial processes, port propagation, same-value drives"): the roots of a serial process
go into reused buffers; in-port values pass down without temporaries; an unchanged scalar assignment to a direct driver is skipped when it is made; a run of
scalar drives of process variables is one guard op; an unchanged vector assignment to a direct driver is skipped as the scalar one (direct plane commits of the whole run 83.4 M to 14.9 M; interpreter, whole run, instructions 574.1 G to 554.9 G, 0.967); materialising a native frame no longer walks the callee's IR. Interpreter, LEON3, 400 us, instructions,
bench builds, steady state: 257.5 G, 250.9 G (roots), 248.3 G (+ ports), 240.9 G (+ same-value drives and runs); whole run `--tier=auto` 625.7 G to 589.5 G.
Microbenchmark (one process, 8 scalar drives of unchanged variables per activation): 460 instructions a drive, about 150 in a run; start-up (first 1 us) 4.6 s to 3.65 s.

Before and after, `tools/realbench/run.py --compare`, `--tier=auto`, run phase (elaboration plus run), core 10, medians of 3, cells interleaved, host load 9 to 15:

| | warm | cold | instructions (warm / cold) |
|---|---:|---:|---:|
| main (3df7bc44) | 48.98 s | 50.14 s | 636.6 G / 645.1 G |
| this change | 45.02 s (0.92) | 47.03 s (0.94) | 578.3 G / 589.9 G (0.91 / 0.91) |

Against the reference simulator (`--ref sim1`, same method, this change): run 45.23 s warm and 46.31 s cold against 13.18 s (3.43 and 3.51 times, instructions
3.38 and 3.45). Totals (analysis of the 458 files one call each, elaboration and run; the analysis is unchanged by this work): reference simulator 20.1 s warm
(6.88 + 13.18) and 20.4 s cold (7.19 + 13.18), yavs 46.2 s (0.99 + 45.23) and 48.7 s (2.37 + 46.31): 2.30 times warm and 2.39 cold (2.49 and 2.58 on main
at the same load; 3.71 and 3.77 at the first measurement of this test).

Open (issues): processes with nested subprograms are never specialised, 25 percent of the ops of `state_register` (#721); the pure-call memo costs 10 percent
of the S-record start-up (#722); vector drives of a wake that does nothing (#723); the cold activation path of serial processes (#724). Tried and not landed:
the short-circuit clean-up of `opt/cfg.rs` on processes (decisions.md).

## sc5_fifo_repack: where the time goes, and `uniform` in compiled code (#519, 2026-10-25)

Profile of the satcat5 stream repacker (`fifo_repack_tb`: 5 instances of 1, 2, 3, 5 and 8 byte lanes, 45 clocked processes, 500 us at 10 ns: 50 000 clock
periods, 223 980 delta cycles, 2.54 M process runs, 21.3 M element drives, 12.6 M events), bench build, core 12, warm world, run phase. The reference simulator
runs it in 0.87 s and 12.6 G instructions: 17.4 us and 252 k instructions a clock period. Before this round: yavs `--tier=auto` 36.9 G (2.2 s, 738 k a clock), `--tier=llvm`
27.9 G (1.49 s, 558 k a clock), `--tier=interp` 40.2 G (2.38 s).

Share of the cycles by layer, `--tier=llvm` (`perf record` cycles, self time): generated code 40 percent (`p_col` 16.6, `p_sreg` 6.4, `rand_next` 6.4, `p_in` 3.4, `p_input` 3.3);
the drive and commit path 27 (`assign_planes_fast` 4.9, `assign_codes_now` 4.1, `finish_direct` 3.7, `commit_planes_run` 3.0, `drive_bits` 2.7, `commit_codes` 1.9); the
random numbers of the testbench 11 (8.3 M calls of `math_real.uniform` through `yavs_rt_native`, `native_frame` and the native); process entry 6; kernel cycle and batch 3.5. `--tier=interp` (`op-census`, 457 M ops): `BinRangeK` 8.6 percent of the ops, `Stmt` 8.5, `CmpBr` 8.2, `AffP` 8.0 (12 ns an op, 8 percent of the time of the ops),
`SigVal` 5.8, `SigRead1` 5.7 (15 ns), `LoadUp` 4.3; by time `WaitSuspend` 8.6 percent (216 ns, kernel included). By process, default `--tier=auto`: `p_col` of the five instances 35 percent of the run,
`p_in` 17 (interpreted, 713 IR instructions), `rand_next` 15 (compiled), `p_sreg` 10, `p_input` 6; `p_col`, `p_in` stay interpreted (above `RTL_MAX`, payback not reachable in a 2 s run: #673).

Change 1 (`math_real.uniform` computed in the compiled code, `Mem::find` finds the slots of the enclosing frames; `docs/decisions.md` "Uniform in compiled code ..."), base c7f56056,
medians of 3 (realbench `--compare`, run phase, core 12, per-binary worlds), wall / instructions:

| design | `--tier=llvm` | `--tier=auto` (warm) |
|---|---:|---:|
| `sc5_fifo_repack` | 1.49 to 1.35 s (0.91), 27.95 to 24.80 G (0.89) | 2.46 to 2.15 s (0.87), 39.8 to 36.8 G (0.92) |
| nine other designs (`poc_arith_addw`, `uvvm_sb_slv`, `uvvm_util_rand`, `osvvm_axi4_random`, `hdlm_lfsr`, `fletcher_bufferreader`, `poc_cache_cpu`, `hdlm_axi_fifo`, `sc5_timestamp`) | wall 0.99 to 1.02, instructions 0.99 to 1.00 | wall 0.98 to 1.03, instructions 1.00 to 1.01 |

Interleaved pairs on one world (7 runs, user cycles): `--tier=llvm` 7.51 to 6.72 G (-10.5 percent), instructions 28.4 to 25.3 G (-11 percent); default `--tier=auto` changes
little because only `rand_next` is compiled there and the plain policy's compile outcome moves the instructions by 10 percent between worlds (#673).

Change 2 (a copy splits `Pos` of an ascending instance descriptor into `Check` and `Bin Sub`; `docs/decisions.md` "A copy splits the position of an index ...", base c7f56056 plus change 1),
`--tier=interp` (copies are made there too; shared code under `--tier=llvm` is unchanged): `sc5_fifo_repack` instructions 40.19 to 38.31 G (-4.7 percent), user cycles 11.31 to 10.95 G (-3.2 percent), medians of 5
interleaved runs, core 12. The 17 realbench designs of the last table, `--tier=interp` (the run is deterministic): instructions 0.99 to 1.00 for every design that runs longer than 0.1 s (`poc_arith_addw`, `poc_cache_cpu`,
`fletcher_bufferwriter_512`, `fletcher_bufferreader`, `hdlm_lfsr`, `uvvm_sb_slv`, `sc5_timestamp`); 0.96 to 1.03 for the small designs, whose initialisation is all there is to measure.

Change 3 (functions and procedures that read signals run on native frames; "Functions that read signals run on native frames"), `--tier=llvm`, medians of 7 interleaved runs, user cycles / instructions:
`sc5_fifo_repack` 7.25 to 6.15 G (-15.1 percent) / 27.91 to 23.22 G (-16.8 percent) with changes 1 and 3 together (base c7f56056); change 3 alone 6.68 to 6.23 G / 24.77 to 23.22 G.

All three changes together, base c7f56056 (realbench `--compare`, 17 designs, run phase, core 12, medians of 3; wall ratio / instructions ratio, below 1 is faster):
`sc5_fifo_repack` `--tier=interp` 0.98 / 0.95, `--tier=llvm` 0.84 / 0.83 (1.46 to 1.22 s, 27.95 to 23.25 G), default `--tier=auto` warm 0.91 / 0.89 (2.26 to 2.06 s), `auto-cold` 0.98 / 0.95; all 17 together: geometric mean wall 0.99 to
1.01 in every tier, instructions 0.99 to 1.00 (`hdlm_lfsr` auto 1.06 is the compile outcome of #673).

Against the reference simulator, `sc5_fifo_repack`, `--tier=auto`, medians of 5, core 12, whole phases of the realbench (analysis of the compile order one call per file, then elaboration plus run): run phase 2.71 to 2.47
times (warm), 2.92 to 2.81 (cold); instructions 3.16 to 2.81 and 3.30 to 3.14 times. Total (analysis plus run; reference simulator 2.55 + 0.83 s warm, 2.50 + 0.83 s cold): warm 0.79 to 0.74, cold 1.00 to 0.98.
The default `--tier=auto` is still 2.5 times the reference simulator's run: 60 percent of its time is the interpreted clocked processes `p_col`, `p_in`, `p_sreg`, `p_input`, `p_pre`, `p_pack`, which the policy does
not compile (#675: the model price of `p_in` and `p_col` is 0.5 s against a 2 s run, and a world that never ran `--tier=llvm` never stores their code); compiled the run is 1.22 s.

Left, for the lanes (comments on the issues): the policy (#675, #673); the drive and commit path, 27 percent of the compiled run (#521, #440, #676); the generated code of the processes, 40 percent (#715, #679); calls that are
not native-frame calls cost 50 to 80 ns (#730); the native-frame code under `--tier-heat` stays at O0 (#729). Not done: a fused op for the element copy `sort_data(col + 1)(row) <= sort_data(col)(row)`
(`SigVal`, `AllocReadVec`, `CopyDrive`: 2 to 4 percent of the interpreter, nothing compiled), loop-invariant signal reads (`reset_p` read per row, `SigRead1 -> NotL`: 2 percent of the ops), descending ranges in
`pos_to_check` (needs a check message per direction at lowering).

## General gaps towards sim1: attribution across all designs (#519, 2026-10-25)

Cross-design attribution on main at 9171b609, bench build with `llvm`, `--tier=auto`: the 42 realbench tests and the 10 bench h2h designs that run on sim1
(52 tests), warm and cold. Per test `perf stat` (minimum of 2 to 3 runs) and a flat `perf record` of yavs and sim1 classified into layers; layer seconds are
the layer's share of main-thread samples times the CPU seconds of the run. No timing cores were used (cores 1 to 3, shared with builds), so the ranking is
reliable and the per-test seconds are not. Full tables and the ranked list with evidence: comment on #519 (agent gapscan).

Geomean against sim1, warm: instructions 0.41, wall 0.33; cold: 0.73 and 0.60. yavs is slower in 15 tests (summed main-thread excess 48.1 s, of which
`grlib_leon3mp` 36.1 s) and ahead by 25.4 s on the rest; sim1 spends 35 percent of its time generating code in the run, yavs 0.9 s of 107 s starting up.

| # | general item | weight (excess in slower tests, without grlib) | lane |
|---|---|---|---|
| 1 | interpreter execution of what stays interpreted under auto | 32.2 s (11.7) | interp6 |
| 2 | value movement of wide vectors, records, composites (gather, scatter, lcopy, copy_bits, cat, memmove); also the floor under compiled code | 15.1 s helpers + 2.4 bulk (3.2 + 0.4) | unowned |
| 3 | kernel per-activation orchestration: cycle, commit, event bookkeeping | 10.4 s (5.0) | kern22 |
| 4 | compiled code of processes over 1 000 IR is not faster than the interpreter (#679) | `grlib_leon3mp` forced compile 67.8 s against 52.4 s | unowned |
| 5 | auto policy: priced out or never considered (#675, #728, #673) | interpreted-only seconds 36.6 of 40.2 (grlib), 2.5 of 2.7 (`fletcher_bufferwriter_512`) | autoclk |
| 6 | process to kernel boundary: entry, scalar drives and waits, signal reads | 13 to 42 percent of small RTL under llvm; reads 2.0 s | cgdirect (entry); signal reads and the scalar drive callback measured at the floor after kernel round 22, nothing left to gain there (decisions "Signal access from processes", #519) |
| 7 | cold start: tier-up latency and cache-miss start-up | cold minus warm 22 s over 52 tests | coldc, startup |
| 8 | calls, frames, pure-call memo | 0.7 s (0.3) | calls730 |

Facts worth keeping: full `--tier=llvm` removes no instructions on `fletcher_bufferwriter_512` (57.4 G against 54.4 G interpreted, net of the load cost) or
`fletcher_bufferreader` (23.9 G against 22.3 G): the cost there is the value path of items 2 and 3, not op dispatch. Natives, allocation, warm start-up, wake and
drive are already ahead of sim1. A layer that is 0 for sim1 (cycle driver, signal reads, helpers) is structural: its generated code or one runtime function does that
work, so compare group totals.

## llvm: address parameters of native frames resolved once (#726, 2026-10-25)

A native frame resolves the region each address parameter points into once, at its entry, when the function uses it as an address three times or more with one use in a loop
(`yavs_rt_region`, `docs/decisions.md` "Address parameters of native frames are resolved once per activation"). Window generator (bench builds, fresh world per binary and tier, 12 untimed runs,
medians of 5 interleaved runs, core 9, host load 17 to 20): `--tier=llvm` 179.6 to 176.0 G instructions (-2.0 percent), 66.4 to 63.2 G cycles; `--tier=auto` warm 147.0 to 143.4 G, cycles level
(-3.3 percent on a quiet host); `decode` of the codec 6.6 to 3.5 percent of the instructions. Realbench `--compare` over all 42 tests (auto warm and cold): wall geomean 1.00 / 1.00, instructions 1.00 / 0.99;
only the window generator moves (warm 9.89 to 9.37 s). New: `crates/yavs/tests/compiled_param_regions.rs`; counter `cb_region`.

## llvm: smaller frame-mode code (#679, 2026-10-25)

(`docs/decisions.md` "llvm: smaller frame-mode code ...".) Bench builds, `--tier-heat=1,0,0`, core 7 (CCD0, shared with builds: cycles carry a
large spread, so instructions, L1i and iTLB misses, branch misses and frontend stall cycles from `perf stat --no-inherit` of the simulation
thread are the evidence; medians of 5 interleaved runs, base = main at 9171b609), the measuring aid `YAVS_LLVM_KINDLOC`.

Machine code of the cold-cache run of neorv32 (30 us), all objects: 8.42 MB to 5.38 MB (-36 %); `exec_comb` 301 769 to 174 253 bytes
(-42 %). LEON3 60 us window with `iu:comb` forced: 24.4 MB to 14.9 MB of code over the objects it compiles.

| case (base to change) | instructions | cycles (min) | L1i misses | iTLB misses | branch misses | frontend stall cycles |
|---|---:|---:|---:|---:|---:|---:|
| `neorv32_single_min` | 35.42 G to 35.06 G | -16 % (-6.8) | -19 % | -54 % | -7 % | -17 % |
| `fletcher_bufferreader` | 23.01 G to 22.61 G | -11.5 % (-12.1) | -21 % | -7 % | -2 % | -18 % |
| `fletcher_bufferwriter_512` | 52.72 G to 51.67 G | -6.6 % (+13.9) | -23 % | -28 % | -29 % | -20 % |
| `sc5_fifo_repack` | 27.01 G to 26.15 G | -0.6 % (+0.5) | -40 % | -29 % | level | level |
| LEON3 60 us, `iu:comb` forced (3 runs) | 74.3 G to 72.2 G | +5 % (+8.6) | -6 % | +80 % | +9 % | +6 % |

The interpreter against heat mode (same binary, 60 us LEON3): 82.2 G instructions against 71.5 G, cycles level (34.5 G against 35.5 G);
L1i misses +50 %, branch misses +56 %. So the goal "heat cycles at or below the interpreter" is met by instructions and by code bytes on every
design but not yet by cycles on LEON3 (the L1i and branch misses of the 15 MB of code) and `fletcher_bufferwriter_512`; the remaining bytes are
`LayoutOf` (10 %), `LCopy` (7 %), `Field` and `Load` in the LEON3 copy (per-kind table with `YAVS_LLVM_KINDLOC`).
Difftest 523/523 on `yavs-interp`, `yavs-llvm`, `yavs-auto` and `yavs-auto` with `--tier-heat=1,0,0`, 1 and 4 threads.

## grlib_leon3mp, second round: copies of nested and serial processes, scratch sets, implicit signals (#720, #721, #722, #723, #724, 2026-10-25)

Commits 44abf774 (copies of processes with nested subprograms and serial processes, file-only processes off the shared-state path, four scratch sets) and the kernel commit
after it (fast update paths with implicit signals, `'last_value` of one element). `--tier=auto` warm, bench build with `llvm`, core 10, medians of 3 interleaved, run phase
of the system (std 1993): 48.65 s to 44.47 s (-8.6 percent), interpreter instructions 564.7 G to 490.5 G (-13 percent), user cycles 230.0 G to 211.3 G. The first commit alone:
54.3 s to 49.3 s (host noisier). Cold not re-measured. Against the reference simulator the ratio of the run goes from about 3.6 to about 3.3 (reference run 12.3 s from the
previous round, not re-run).

Breadth (realbench `--compare`, 42 tests, run phase, `interp` and `auto`, core 10, load average about 20 on the host, first commit against its base): wall geomean 0.95 / 0.95,
instructions 1.01 / 1.00; per test within noise except `grlib_leon3mp` 0.92 / 0.94 (instructions 0.94) and the small tests that start up in 10 to 50 ms. `neorv32_single_cache` read 1.13
in wall at 1.00 in instructions; repeated by hand on core 10 (4 interleaved pairs of user cycles) it is +1 percent (layout noise). The kernel commit was checked on the system, the four SDRAM
models alone (100 us: 12.8 G to 10.5 G instructions) and the work-counter case `implicit_fast`; it changes nothing for designs without implicit signals.

What the profile says now (four SDRAM models alone, `op-census`): 85 ops and about 16 k instructions per wake; in time, the kernel's part of every wake (`Suspend` 38 percent), the
`DriveRun` of 13 scalar drives 8, `CopyDrive` 4.6, `EdgeBr` 4. Not done: vector drives in `DriveRun` (built, no gain, #723 closed in decisions.md), the pure-call memo give-up (#722: the memo
saves twice its cost there; the S-record start-up is the `HRead` loop itself, a table pass for pure functions of one enumeration argument would be the general fix), the empty kernel
cycle after an activation whose drives were all skipped (kernel lane), the cold path of processes that really use protected objects or the shared heap (#724 remains for those).

## Subprogram calls (#730, 2026-10-25)

Where a frame-mode call goes was measured in both tiers (`docs/decisions.md` "Calls: where the 50 to 80 ns go"): no piece is more than a tenth of a call.
Taken: `Machine::pop_ret` on `Op::Ret`, the saved caller without its return op, and no slots for a lean callee's unused locals. Measured and not taken: a larger inliner budget (window generator interpreter -9 percent, `--tier=auto` +6 percent), a pool of register files per depth.
Round 2 (`docs/decisions.md` "Calls, round 2"): `Drive`, `DriveSlot`, vector `SigRead` and `SigSlice` are native-frame instructions (signal-driving procedures 0.59 to 0.68 of the cycles of frame mode, vector-reading functions 0.42), the temporary area has capacity when a view is taken, and the lean path takes nested callees and, where no collection can run, `out`/`inout` references (nested function call 0.85 of the cycles). Real testbenches are level: their steps are protected calls and heap instructions. `Wait` as a native-frame instruction was measured and not taken (an always-waiting procedure 1.35 times the cycles).
Left: waits and `Edge` on native frames need a cheaper materialisation or a profile; the callers that step `New`/`NewFill`/`LayoutOf`/`Free` (osvvm_axis_random).

## llvm: compiled clocked RTL, round 7: the direct entry (#634, 2026-10-25)

Decisions: `docs/decisions.md` "Compiled clocked RTL processes, round 7". The process-direct entry that was left as the next step (the kernel
calling the compiled activation with one state pointer and a persistent per-process view) was designed and **not built in that shape**: a `Mem`
carries a 64 KB object table, a persistent view per process is the cold-lines cost round 2 measured, and `yavs-interp` forbids `unsafe`. What was
built is the same saving by removing the layers and the churn: `Machine::run_direct` is the first thing `InterpProcess::run` does for a compiled
process (one function frame instead of `Process::run`, `run_quick`, `run_compiled`), `rt::enter` is inlined into `Tier::run_cached`, and the
constants a process uses (`Life::Runtime`, at most 16, runtime id in the high bits of `TierFrame::owner`) stay in the thread's table from one
process to the next instead of being dropped, loaded from the process's record and saved again at every activation (230 instructions in
`rtl_lfsr`); the walks over the table skip empty words (`Mem::occ_any`). Entry plumbing per activation (`tools/stepcount`, `--tier=llvm`): `rtl_lfsr`
703 to 441 of 1 391 instructions, `rtl_ff` and `rtl_fan` 468 to 431, `lfsr64` 700 to 624.

Simulation phase, bench builds, core 14, `--tier=auto --tier-heat=1,0,0`, cycles and instructions (minimum of 5, interleaved), main before and after on the rebased tree:
`rtl_ff` 3.02 to 2.83 G and 9.11 to 8.83 G; `rtl_lfsr` 1.32 to 1.18 and 4.81 to 4.14; `rtl_chain` 0.90 to 0.86 and 3.15 to 3.09; `rtl_fan` 4.38 to 3.82 and 13.85 to 13.16;
`lfsr64` 0.67 to 0.65 and 2.68 to 2.58. The default `--tier=auto` is level with the interpreter on these designs (the policy compiles little of them; its
cycles move by noise only), the interpreter is unchanged to the instruction. Whole-run ratios to the reference simulator of the last head-to-head (rtl_lfsr 1.57,
lfsr64 1.43, rtl_ff 1.25, rtl_chain 1.21) were not re-measured here: the compiled body and the kernel and drive path (550 of 1 800 instructions in `rtl_lfsr`),
not the entry, are what is left, and with `--tier=auto` the policy decides whether these designs run compiled at all (#728, #675).

Left (ranked): the kernel and drive path per activation (`finish_direct`, `assign_planes_fast`, `commit_planes_run`: #521); `Process::run` 182
instructions (the `Host`, the sampling slot and the result, 60 before the split, the rest moved here from `run_quick`); `run_cached` 150; `view_again` 72;
a persistent per-process state needs `unsafe` in `yavs-interp` and a per-process object table (a decision for the user).

### `--tier=auto` run-to-run profile (yavs-llvm, yavs, #754, #755, 2026-10-25)

Decisions and causes: `docs/decisions.md` "`--tier=auto` run-to-run profile". A run leaves a small profile next to the compiled code (hot
processes, processes a trial took back, the simulation CPU time); the next run of the same program and command line compiles the listed
processes at their first look, and the run after it keeps them if the run as a whole took 3 percent less simulation CPU time, else loses
them for good. `YAVS_AUTO_PROFILE=off` turns it off; the first run, `--tier-heat`, `--no-cache` and runs under 100 ms decide as before. The wait of the
shared code of a process for its copy (`SPEC_WAIT`) is gone with it.

Method: bench builds of main (d3123c88 for these tables, "main") and of this change ("new"); per world a fresh copy of the pristine world, the analysis caches of the
binary filled by two untimed runs, the compiled-code cache emptied, then consecutive `--tier=auto` runs pinned to core 8 (the compile thread shares
it); medians of 6 worlds (4 for the window generator), wall and `perf stat` user cycles and instructions. Ratios new / main of the Nth run of a world
(below 1 is faster):

| design | 1st (no profile) | 2nd | 3rd | 4th |
|---|---|---|---|---|
| `sc5_fifo_repack` | 1.02 | 1.08 | 0.73 | 0.73 |
| `poc_arith_addw` | 1.00 | 0.98 | 1.00 | 0.99 |
| `vai_window_gen_interlock` | 1.04 | 1.00 | 0.98 | 0.96 |
| `uvvm_util_rand`, `uvvm_sb_slv`, `olo_fix_cordic_rot`, `neorv32_single_min` | 0.98 to 1.00 | 0.97 to 1.01 | 0.97 to 1.00 | 0.98 to 1.01 |

`sc5_fifo_repack` main: 2.40 s first, 2.35 second, 2.43 third (its own cache keeps changing the set the policy compiles); new 2.46, 2.54, 1.76 and 1.68 s
(cycles 12.0, 12.3, 8.5 and 7.7 G against 11.7, 11.5, 11.7 and 11.2 G). The first run with the profile compiles about twenty processes cold (40 to 65 ms of
CPU each) on the simulation's own core, which is the +8 percent. With the first threshold (a sixteenth of the sampled time) the third run was 0.89.
`poc_arith_addw` ended 5 percent faster in an earlier set of worlds (0.94 to 0.95 from the second run) and 1 to 2 percent in the later one: the
difference is main's own spread. The four last rows never compile a process early (the hot ones are functions, or kinds the model says do not gain),
their profile lists nothing and instructions are equal; the loss path was seen on `neorv32_single_min` when the kind filter was not there: one
large combinational process compiled early, the run 30 percent slower, lost for good by the end of that run.

Head-to-head designs (`tools/bench --compare`, 8 and 15 runs, core 8): geometric mean of CPU new / main 0.969 over the 11, `lfsr64` 0.82 (0.87 to 0.91
with the profile off: a process of fewer than 64 instances in a 170 ms run no longer waits 400 ms of sampled work for a switch its compiled shared
code makes at once), `rtl_ff` 0.99 to 1.02, `rtl_fan` 1.00 to 1.03 (the same with the profile off: layout spread), `rtl_lfsr` and `rtl_chain` 0.96.
The wait's removal alone: no change on any of the 42 designs of `tools/realbench`, nor on `sc5_fifo_repack` (32.06 against 32.06 G instructions).

All 42 of `tools/realbench` (`--compare`, `--tiers auto,auto-cold`, run phase, core 8, default runs): wall geometric mean new / main 1.02 (auto, warm: the ten-millisecond tests move by whole hundredths; 0.98 over the 11 tests of 1 s or more; sum
of the walls 1.01) and 1.00 (auto-cold, sum 1.01), instructions 0.99 and 1.00. Per test only `sc5_fifo_repack` moves: 1.82 to 1.43 s (instructions 0.80);
the others are host noise (load average 30 to 39 from other agents: `uvvm_axistream_vvc` 1.12 at equal instructions, `poc_arith_addw` 0.95 at
instructions 1.10, `leon3mp` 40.4 to 41.6 s). Cold equals main on every design (the profile of a first run is written at its end).

#755: `tier.rs::auto_tier_compiles_a_long_loop_of_one_call` waits (60 s at most) for the compile the call started instead of racing it; it
failed once under `gc-stress` on a loaded host (load average 8).

### `--tier=auto` first-run latency (yavs-llvm, yavs-interp, #757, #762, 2026-10-26)

Decisions and causes: `docs/decisions.md` "`--tier=auto` first-run latency". Frame-mode compute loops are priced as what they cost (`Kind::ComputeFrame`),
a function called on native frames that has a loop gets its frame-mode code only when it is seen interpreted again (`Worker::frame_at`), a copy of a
process that is not clocked RTL compiles once it repays its own price (and the cache list names copies apart), and the memo of a pure function is off
for all once every process that called it gave it up (#762). `Compute` (native frames) keeps its price: the measured, lower one compiled losers.

Method: bench builds of `0dcfa551` ("main") and of this change, a fresh copy of the pristine `tools/realbench` world per binary and round, the analysis
caches filled by two untimed `--tier=interp` runs, then four consecutive `--tier=auto` runs (run 1 cold, run 3 is what realbench times as warm), all cells of
a test on one claimed core of CCD1 (core 9), 5 rounds interleaved, medians, wall from `perf stat duration_time` (the compile thread shares the core).
Ratios new / main (below 1 is faster; instructions in brackets):

| design | run 1 (cold) | run 2 | run 3 | run 4 |
|---|---|---|---|---|
| `poc_stat_histogram` | 0.89 (0.95) | 1.16 (1.08) | 1.05 (0.98) | 0.97 (0.98) |
| `sc5_fifo_repack` | 0.98 (0.94) | 0.97 (0.96) | 0.89 (0.89) | 0.85 (0.86) |
| `sc5_timestamp` | 1.01 (1.00) | 1.01 (1.00) | 1.00 (1.00) | 1.01 (1.00) |
| `poc_arith_addw` | 0.96 (0.93) | 1.01 (1.00) | 1.02 (1.01) | 0.99 (1.00) |
| `vai_window_gen_interlock` | 1.00 (1.02) | 1.04 (1.00) | 0.99 (0.97) | 1.00 (0.98) |

Rebased on 9418e648 (#764 fixed), 4 rounds, same method: `poc_stat_histogram` 0.89 (0.95), 1.01 (1.01), 1.02 (1.01), 1.02 (1.01); `sc5_fifo_repack`
0.93 (0.92), 1.01 (0.98), 0.87 (0.88), 0.88 (0.89). Against main at b15bc241 (the parent of this change), two sets of 4 rounds of three runs: `poc_stat_histogram` 0.89, 0.99, 0.99;
`sc5_fifo_repack` 1.00 to 1.06 cold, 0.97 to 1.03, 0.85 to 0.87; `poc_arith_addw` 0.96, 0.98, 0.99; `vai_window_gen_interlock` 1.01 to 1.04 cold (instructions
1.01 to 1.03: frame-mode compute loops compiled late in its 11 s cold run, `check_axi_stream` among them), 1.01 to 1.02, 0.99 to 1.00. A `ComputeFrame` speedup of
1.5 instead of 1.8 did not change either cold run beyond that spread.

- `poc_stat_histogram` cold: the three element-loop functions are compiled as native-frame code only (426 ms of compile CPU before, about 150 now); the run 2 row
  of the table is #764 (a memoised function with published native-frame code kept running interpreted until `frame_at` compiled its frame-mode
  code), fixed in 9418e648 (the fast loop's tier bookkeeping is on once a function is flagged for native-frame calls). The memo callback that was 7.3 percent of the warm cycles is 0.7 percent (#762). Warm `perf record`:
  what is left is the element-loop helper (`eloop::run` and its object look-ups, about 40 percent) and the call of a function returning an unconstrained
  array (#757).
- `sc5_fifo_repack`: the processes are `ComputeFrame` now (15 ms + 110 us an IR instruction against 4 ms + 900), so the profile of the first run lists them
  and the warm runs compile and link them; cold, only the copies that repay themselves compile (with the pooled rule and the cheaper price twenty copies
  compiled, cold +8 to +13 percent).
- Tried and dropped (each measured on these designs plus `uvvm_sb_slv`, `uvvm_axistream_vvc`, `uvvm_util_rand`, `poc_cache_cpu`): `Compute` at its
  measured price (8 ms + 340) with speedup 2.4 (histogram cold 0.80, but `uvvm_sb_slv` 1.08 warm and cold, `uvvm_axistream_vvc` cold 1.16,
  `poc_cache_cpu` cold 1.11..1.21) or 1.3 (`poc_cache_cpu` cold 1.11); a kind for functions whose loops are all element loops (the histogram's functions
  have more loops than element loops).

All 42 of `tools/realbench` (`--compare`, `--tiers auto,auto-cold`, run phase, core 9, default runs): wall geometric mean new / main 1.01 warm, 1.00
cold (sums of the walls 0.99 and 1.00; tests of 1 s or more 0.99 and 0.98), instructions 0.99 and 1.00. Moved: `sc5_fifo_repack` warm 0.87,
`poc_stat_histogram` cold 0.88, `poc_arith_addw` cold 0.96, `hdlm_lfsr` cold 0.95; `uvvm_util_rand` 1.07 / 1.06 in that pass was 1.00 / 1.00 in a
rerun of 7 runs (with `uvvm_sb_slv` 0.98 / 1.00 and the histogram 1.00 / 0.90); the tests of 0.01 to 0.03 s move by whole hundredths. Head-to-head
designs (`tools/bench --compare`, 8 runs, core 9): CPU geometric mean 1.013, every design 0.99 to 1.03 (1.001 for an earlier build with the same
policy for these designs: host spread).

### `--tier=auto`: a compile pays for the callees it brings (yavs-llvm, #757, 2026-10-26)

Decisions and numbers: `docs/decisions.md` "`--tier=auto`: a compile pays for the callees it brings". A candidate is compiled once what its whole tree
(the native-frame callees `tree` brings) saves in a rest of the run like the part seen covers the tree's price, shared among four runs once the profile
says the program ran before; native-frame compute loops are priced at one and a half times what they measure (12 ms + 510 us an IR instruction, was
4 ms + 900). All 42 of `tools/realbench`: wall geometric mean 1.00 warm and cold, instructions 0.99 and 1.00; `poc_stat_histogram` cold 0.92,
`uvvm_axistream_vvc` warm 0.92. Left: the native kinds' models (measured speedups of 3 or more since the hand-over fix), `ComputeFrame`'s speedup for
processes like `check_axi_stream`, the profile key across runs (#774).

### The watching loop calls native-frame callees itself (yavs-interp, #757, 2026-10-26)

Decisions and numbers: `docs/decisions.md` "The watching loop calls native-frame callees itself". An interpreted caller no longer hands its activation
over to the loop with the tier's bookkeeping when it calls a function the tier runs on a native frame, so compiling such a function no longer makes the
rest of the caller's activation dearer (`uvvm_sb_slv` with `match_identifier` compiled: 8.5 percent more instructions before, 7 percent fewer now).
`YAVS_COUNTERS` prints `retiers`, the hand-overs that remain (callees with frame-mode code, #750). All 42 of `tools/realbench`: wall geometric mean 0.99
warm, 1.00 cold; `uvvm_axistream_vvc` warm instructions 0.91.

## Value movement: lcopy, wide copies, in-place concatenation (#519, 2026-10-25)

Gapscan item 2 (no timing cores: exact instruction counts, `perf stat instructions:u`, core 6, bench build with `llvm`, `--tier=interp --threads 1`; decision and numbers in
`docs/decisions.md` "Value movement"). Call shapes of `lcopy` (histogram of a throwaway build, not kept): grlib_leon3mp 115 M sampled calls, 85 to 95 percent of 1 to 64 elements in one
block on each side; fletcher_bufferwriter_512 6.7 M, half of them 4 to 64 elements into heap or shared heap, 10 percent of 512 elements; vai_window_gen_interlock 26 M of 8 elements.

| change | where | effect |
|---|---|---|
| `lcopy1`: one block each side, in registers, destination changed in place (frame, temporary, heap, shared heap) | `Machine::lcopy`, `AllocLCopy` | 126 instructions per 8-element copy statement; `lcopy` + `copy_bits` 417 to 230 per 2000 calls in grlib |
| `lcopy_wide`: three or more blocks, objects apart: memcpy or `shift_merge` between the objects | `Machine::lcopy_wide`, `copy_pair` | 629 per 512-element copy at another bit offset, 716 at the same offset |
| `Op::CatLCopy` for any size and offset, wide `&` built in its destination | `pre.rs`, `Machine::cat_apart` | 1071 per `&` of 8 x 64 into 512 elements; no temporary of the result size |

Whole runs, interpreter, instructions: fletcher_bufferwriter_512 -1.0 percent, grlib_leon3mp -0.55, poc_stat_histogram -0.9, osvvm_axis_random -0.5; hdlm_lfsr, poc_arith_addw, uvvm_sb_slv
+0.14 to +0.25 (loop layout). The gain per test is small because the wide-vector share of these runs is small once the kernel and dispatch costs are counted: the gapscan "helpers"
17.6 s are mostly calls of 1 to 64 elements, which now cost about 40 percent less each, and `sig_gather`/`sig_scatter` (about 100 and 130 instructions per leaf, 2.4 + 3.0 percent of grlib
cycles) are untouched.

Left, measured: `sig_gather` and `sig_scatter` per leaf (signal lookup, kind dispatch, code read) and per call (plan lookup compares descriptor bytes); they want a plan that knows runs of leaves in one
signal (record packs, #686, #677). `layout_word` (1.7 percent of grlib cycles, about 100 instructions per call) belongs to #518. The compiled tier keeps its own inline copies for one and two blocks; `CatLCopy` and `lcopy_wide` are interpreter code, so
`fletcher_bufferwriter_512` under `--tier=llvm` (above the interpreter) is not helped by this change.

## Compiled code polls the stop request (#737, 2026-10-25)

A process that never suspends now ends on Ctrl-C or SIGTERM whichever tier runs it: compiled code (`--tier=llvm`, and `--tier=auto` once it has compiled the process) polls
`TierFrame::stop` at the retreating `Br`s of such a process and leaves to the interpreter at that `Br`, which suspends the process as before (`docs/decisions.md` "Compiled code
polls the stop request"). Tests: `stuck::sigint_ends_a_process_that_never_suspends` in the default, `--tier=interp` and (llvm feature) `--tier=llvm` variants, each also checking the
exit status 1, "simulation interrupted" and the wave file; the llvm variant fails without the poll. Cost, user instructions of `--tier=llvm` runs on bench builds (warm object cache, minimum of 3), before to after:
`compute_h2h` 2 733.70 M to 2 733.72 M, `poc_arith_addw` 124.28 G to 124.34 G (+0.05 percent), the window generator 160.33 G to 160.33 G, because their processes wait and carry no poll. The worst case, a 20 M iteration compute loop in a process
without any wait that ends in `finish`: 2 714 M to 2 814 M (+3.7 percent, 5 instructions an iteration). Polling every `Br` that jumps back in the code (not only the retreating ones) cost +15 percent there: the
loop has several such `Br`s to joins laid out earlier. Not covered: SIGUSR1 (the hang report of a process that never suspends), loops ended by a `CondBr`, the Cranelift lowering (#481).

## Result tables for pure functions of one small discrete argument (yavs-ir `casetab.rs`, #722, 2026-10-25)

A pure function of a character, an enumeration or a small integer type that picks its result with a `case` (7 arms or more) or a long `if` chain gets a result table when it is
lowered; the inliner reads the table in the caller in every tier and keeps the call for values the table does not hold (`docs/decisions.md` "Result tables for pure functions of one
small discrete argument"). `grlib_leon3mp` (bench build with `llvm`, `--tier=interp`, no cache, core from `tools/claim core`), the first 1 us (elaboration and the S-record start-up
of the memory models): instructions 63.85 G to 59.9 G (-6.2 percent), cycles 18.43 G to 17.2 G (-6.7 percent; 3 interleaved pairs); `--stop-time=0fs` 38.2 G to 35.8 G; the whole run
476.8 G to 472.9 G instructions, output byte-identical, user cycles level to -4 percent over 3 interleaved pairs on a host with load average above 30. The start-up of the memory
models is the `HRead` loop itself (slices, `SubtypeCheck`, `LCopy` and `Is_X` per digit); the table removes only the call and the memo probe of `FromChar`.

Breadth (realbench `--compare`, 42 tests, run phase, `interp` and `auto`, one run per cell, load average 14 to 25, binaries of the base and of the change): instruction ratios 0.95 to 1.04,
the geometric mean 1.00; every test within 1 percent but `grlib_leon3mp` (0.99 interp, 0.95 auto) and `sc5_fifo_repack` (auto 0.91, the layout of an unrelated unit; not repeated).
Wall times of single runs in that table are host noise (`grlib_leon3mp` 1.15 and 1.19 there, level to faster in the interleaved cycle measurement above). `tools/bench --compare`
(31 benchmarks, 3 runs, one core): CPU geomean 1.005. Work-counter case `case_tab`; correctness case `crates/yavs/tests/casetab.rs` (every value of a hex-digit function, a nibble
function, an enumeration, an integer type with an `if` chain, functions that report for some values; output and reports equal with `YAVS_NO_CASETAB=1`, at 1 and 4 threads, in
`--tier=llvm` and `--tier=auto`). Not done: integer subtypes (the IR type is the base type), results whose length depends on the argument, a fat pair per call on the table path.
## The HRead digit loop of the S-record start-up (yavs-ir `opt.rs`, `opt/inline.rs`, yavs-interp `ieee.rs`, #744, 2026-10-26)

The digit loop of the `HRead` of GRLIB's `stdio` package (`SV(4*i to 4*i+3) := FromChar(S(i)); if Is_X(SV(4*i to 4*i+3))`) was more than half of the first 1 us of `grlib_leon3mp` (the
twelve memory models read an S-record image at start). Result-table reads are made before the first optimiser pass (the pair a table read builds is no longer made and read back); the optimiser keeps
its facts across a `TempRoot`, across a store to another word of the same block and across reads through a frame address; the length of `v(x to x + 3)` is a constant, so the slice's descriptor, layout and
length check fold; a repeated slice position is shared; a block that is only stored into goes; `is_x` of a vector reads the planes in place and `to_bitvector` is a native (`docs/decisions.md` "The HRead
digit loop"). 33 interpreter ops per digit became 13. Bench builds with `llvm`, `--tier=interp`, cores 8 to 15 (`tools/claim core`), before = the commit before:

| `grlib_leon3mp` | before | after |
|---|---:|---:|
| first 1 us, warm cache: instructions | 51.89 G | 31.54 G (-39.2 percent) |
| first 1 us, warm cache: cycles | 13.82 to 13.91 G | 8.33 to 8.48 G |
| first 1 us, warm cache: wall, same core, `--tier=interp` / `auto` / `llvm` | 2.70 s / 2.0 s / 7.5 s | 1.63 s / 1.45 s / 6.85 s |
| first 1 us, empty cache: instructions / CPU time | 60.69 G / 3.30 s | 40.39 G / 2.24 s |
| whole run, `--tier=interp`: wall / instructions | 36.02 s / 449.1 G | 34.87 s / 428.1 G |
| whole run, `--tier=auto`: wall / instructions | 36.54 s / 439.4 G | 35.38 s / 428.9 G |
| whole run, empty cache, `--tier=interp` / `--tier=auto`: wall | 36.95 s / 37.19 s | 35.68 s / 36.53 s |

Breadth: `tools/realbench --compare`, 42 tests, run phase, `interp` and `auto`, one run per cell: instruction ratio geometric mean 0.99 and 1.00, no test slower beyond noise; with an
empty cache (`interp-cold`, `auto-cold`, the optimiser's first run of each function) 1.00 and 1.00. The standard `hread` of `ieee.std_logic_textio` into 32 and 64 bit vectors: 7.30 G to 5.58 G instructions.
`tools/bench` designs `textio_heavy`, `file_io`, `strings_heavy`, `ieee_heavy`, `report_heavy` execute the same instructions as before; `textio_heavy` has 3.7 percent more cycles over 8 interleaved pairs
(7.6 percent more L1 instruction cache misses, no change in the executed instructions: the code layout of #576). Output of the `difftest` corpus equal at 1 and 4 threads in every tier (531 of 531).
Work-counter cases: `hread_loop` (new), `case_tab` (ceiling lowered). Left (issues): `Init_mem` of the SDRAM model is now 18 percent of the first 1 us (#768); the fixed cost of one `HRead` call (86 ops for
one digit, #771); a copy-free `Is_X` of a slice (#770); table reads with a range test or a validity mask (#769).

## Cold library analysis, round 4: lookups invalidated by name, one hash per symbol (#742, #740, 2026-10-25)

Cold analysis of a library, one file per `yavs analyze` call (70 percent of cold minus warm in the head-to-head, #740), after #424 rounds 1 to 3. Cuts and reasoning: `docs/decisions.md`
"Cold analysis, round 4". Whole-manifest cold analysis, user instructions in G, base build to new build (both `tools/build-bench.sh --features llvm`, cores 0-5, load 17 to 36;
the counts are exact, the kernel column and the wall times of a shared host are not):

| manifest (calls) | before | after | ratio |
|---|---:|---:|---:|
| osvvm-2008 (174) | 5.414 | 4.836 | 0.893 |
| uvvm-2008 (235) | 9.360 | 8.363 | 0.893 |
| open-logic-2008 (386) | 10.959 | 9.808 | 0.895 |
| vunit-2008 (143) | 4.270 | 3.872 | 0.907 |
| poc-2008 (181) | 2.664 | 2.407 | 0.904 |
| hdl-modules-2008 (86) | 0.924 | 0.826 | 0.894 |
| satcat5-2008 (246) | 3.622 | 3.288 | 0.908 |
| neorv32-2008 (67) | 1.415 | 1.264 | 0.893 |
| grlib-1993 (454) | 14.107 | 12.191 | 0.864 |

User cycles of the quiet OSVVM pairs 4.42 to 3.87 G and 4.23 to 3.73 G, task-clock 1.66 to 1.48 s (-11 percent); kernel cycles are about 40 percent of a cold call and vary from 3 to 8 G between
runs of one binary on this host (THP compaction, load), so no kernel claim is made. Warm analysis (the same files again, user instructions): osvvm 1.201 to 1.126 G, satcat5 0.914 to 0.892, hdl-modules 0.178 to 0.173.
`tools/realbench --compare --phases analyze,elab`, 10 tests (one per library selection with a test), cores 11, instruction ratios new / old: cold analysis 0.90 (lib 0.90, own 0.95), warm analysis 0.94, cold `elab` 0.98
(neorv32 1.03), warm `elab` 0.95; wall times on the loaded host ranged 0.66 to 2.14 and are not usable.

Checks: all 33 analysis corpora with a release build with debug assertions (the recomputation on every lookup cache hit, every table compared after each unit) as expected, 0 differences; `YAVS_UNIT_CHECK=1` cold and warm
(each file analysed twice, two calls) over 14 corpus selections (hdl-modules, neorv32, fletcher, en_cl_fix, poc, vunit, osvvm 2008 and 2019, uvvm 2008 and 2019, satcat5, open-logic, json-for-vhdl, grlib 1993): no difference between the cached and the
fresh analysis. The unit entries of hdl-modules (172), satcat5 (590) and vunit (254) written by the old and the new build have the same sizes pairwise and differ only in the hashes that
carry the build id.

Where a cold call goes now (frame-pointer profile, 9 corpora): the file's own analysis 40 percent (`Page::at` loads of the lazy arenas 3.8 percent, lookups 3 percent, the rest flat), dependencies' entries 25 percent (`Scope` pages
6 percent, of which 45 percent is interning the names; `Decl` pages 6 percent), writing the entry 10 percent, parsing 9 percent, library index 2.5 percent, dynamic loader 5 percent. Batching is a user decision: one call per library
instead of one per file takes UVVM from 9.36 to 6.08 G user and 2.38 to 0.36 G kernel instructions (`docs/decisions.md`). Not verified: Windows and aarch64 (CI).

## llvm: compiled copies of processes, constant `LCopy` (#679, 2026-10-25)

(`docs/decisions.md` "llvm: compiled copies of processes, ...".) A process whose shared code is compiled now switches to its specialised copy
(`Tier::compiles_copies`, `AutoTier`) and the copy is compiled, so the compiled code has the folded layouts and descriptors of the interpreter's
copies; a constant `LCopy` in one plane block is inline arithmetic with constant masks; a register defined once by a constant is not stored.
Bench builds (`tools/build-bench.sh --features llvm`) of main at 10a5dc8d and of the change, one timed core of CCD1 (`tools/claim core`), all
modes of a design interleaved in one session (`perf stat --no-inherit`, cycles:u of the simulation thread, medians of 3 after an untimed warm
run; a session differs from another by up to 25 percent, so only the rows of one table are compared). Heat is `--tier=auto --tier-heat=1,0,0`.

| design (cycles in G) | interpreter | heat, main | heat, change | auto, main | auto, change |
|---|---:|---:|---:|---:|---:|
| `fletcher_bufferwriter_512` | 20.3 | 23.0 | 17.5 (-24 %, instructions -15 %) | 20.8 | 20.4 |
| `grlib_leon3mp` 150 us, `iu:comb` etc. forced in heat | 37.3 | 47.5 | 41.9 (-12 %, instructions -6 %) | 43.4 | 40.2 (-7 %) |
| `sc5_fifo_repack` | 11.4 | 7.09 | 5.70 (-20 %) | 9.59 | 9.65 |
| `neorv32_single_min` | 14.9 | 13.95 | 13.47 (-3.5 %) | 14.9 | 15.2 |

Before the rebase onto the work-clock policy (f85fcee4 as base, same method): writer heat 26.2 to 20.95 G, LEON3 150 us heat 51 to 43 G, `sc5_fifo_repack`
`--tier=auto` 9.4 to 6.9 G with the wait removed as well (not kept, decisions). `tools/realbench --compare` of f85fcee4 and the change, `--tiers auto,auto-cold`, 42 tests,
one core: instructions equal within 1 percent except `poc_arith_addw` -16 % (120.8 to 100.8 G warm) and `sc5_fifo_repack` -4 %; the flagged wall slowdowns were
runs of 0.01 to 0.3 s and one 55 s LEON3 run with equal instructions (the whole LEON3 run, interleaved, `--tier=auto`: 291 to 259 G cycles median, 263 to 255 minimum).

Where the LEON3 gap to the interpreter is (heat 150 us, +12 %, instructions -6 %): frontend stall cycles 11.3 G against 5.9 G, L1i misses 108 M against 38 M, branch misses
219 M against 160 M. The compiled code is 62 bytes per IR instruction in the largest copy (112 in the shared code); a process that runs its whole straight-line body at each
activation (the integer unit's `comb` copy: 11 477 IR instructions, 700 KB of machine code) streams more instruction bytes through the 32 KB L1i and the 1 MB L2 than the
interpreter's loop and operation stream do. Top kinds of that copy by bytes (`YAVS_LLVM_KINDLOC`): `LCopy` 14 % (336 bytes each before this change, 266 after;
the destination's heap lookup is a large part of the rest), `Br` 7 % (the entry switch tree), `SigRead` 7 %, `Const` 7 % (before this change), `Alloc` 5 %. Left: one lookup for the several
`LCopy` of one record variable, `Alloc` and `SigRead` bytes, and shared subprogram code still has the unfolded `LayoutOf` and `Field` (stepped `LayoutOf` 467 K per 60 us in LEON3).
Difftest 524/524 on `yavs-interp`, `yavs-llvm`, `yavs-auto` (1 and 4 threads) and `yavs-auto --tier-heat=1,0,0` (1 and 4 threads, also with `YAVS_SPEC_AFTER=2`).

### `--tier=auto` round 5 (yavs-llvm, yavs-interp, #675, 2026-10-25)

Decisions and causes: `docs/decisions.md` "`--tier=auto` round 5". Method: bench builds of main (f3a280ea, "main") and of this change ("new"), a
fresh copy of the pristine world per sample, pinned to core 8 (the compile thread shares it), cold = first run, warm = third, 4 worlds, the three
tiers interleaved per world; wall of `--tier=auto` as a ratio to `--tier=interp` of the same build (below 1 is faster).

| design | main cold / warm | new cold / warm |
|---|---|---|
| `rtl_lfsr` | 1.11 / 1.05 | 0.99 / 0.80 |
| `rtl_ff` | 1.06 / 0.87 | 0.97 / 0.86 |
| `sc5_fifo_repack` | 1.07 / 0.94 | 1.03 / 0.89 |
| `lfsr64_xor` | 0.99 / 0.95 | 0.88 / 0.85 |
| `hdlm_lfsr` | 0.48 / 0.44 | 0.49 / 0.41 |
| `fletcher_bufferreader` | 1.04 / 1.04 | 1.02 / 1.02 |
| `fletcher_bufferwriter_512` | 1.04 / 1.04 | 1.03 / 1.03 |
| `neorv32_single_min` | 1.02 / 1.04 | 1.02 / 1.03 |
| `lfsr64`, `rtl_chain`, `poc_arith_addw`, `vai_window_gen_interlock` | equal | equal |

Geometric mean of new / main over the 47 designs (the 42 of realbench and the h2h designs): wall 0.987 cold, 0.970 warm, warm cycles 0.986.
Head-to-head designs (`tools/bench --ref sim1 --tiers interp,auto`, warm, median of 8, core 8), wall of `--tier=auto` against the reference
simulator: `rtl_lfsr` 2.33 to 1.80 (0.35 to 0.27 s; main had lost the 1.57 of head-to-head 4 to the trial of the shared code), `rtl_ff` 1.28 to
1.26, `compute_h2h` 0.15 to 0.14, the others equal (`rtl_chain` 1.38, `lfsr64` 1.60, `rtl_ram` 1.20, `lfsr64_xor` 1.00, `rtl_fan` 0.83, `rtl_bus` 0.28,
`xor_types` 0.39).

All 42 of `tools/realbench` (`--compare`, `--tiers auto,auto-cold`, run phase, core 8, default runs): wall geometric mean new / main 1.00 (auto,
warm) and 0.99 (auto-cold), instructions 0.99 and 0.99, sum of the walls 0.99 and 0.98; per category 0.95 (fsm) to 1.10 (memory, four runs of
0.01 to 0.03 s) warm and 0.97 to 1.00 cold; slowdowns flagged by the tool: none. Largest: `hdlm_lfsr` 1.44 to 1.36 s warm, `poc_fifo_ic_assembly`
0.09 to 0.08 s, `fletcher_bufferreader` 1.55 to 1.52, `leon3mp` 36.4 to 36.1 s.

## `--tier=auto` warm runs of one-loop processes (yavs-interp, yavs-llvm, #749, 2026-10-25)

Decisions and cause: `docs/decisions.md` "`--tier=auto`: a process that never comes back from its first wait is not waited for". Head-to-head 5
found `compute_h2h`, `xor_types` and `xor_sig` as slow warm as cold; `6d2f237f` (the shared code of a process that can have a copy waits for the
switch, 400 ms of sampled work) was the cause: those processes run one loop in their only activation, so the switch never comes. Fixed: the
interpreter reports the second activation of such a process (`NativeFlags::resumed`) and the tier waits only while one may still come.

Method: bench builds with the `llvm` feature of main (`93b2a89e`, A) and of the fix (B), `--tier=auto`, default threads, the default generics of
`tools/bench/designs`, one claimed core (the compile thread shares it), medians of 5 interleaved runs, warm = third run of the work library, cold = a
fresh copy of the analysed library per run; wall ms (CPU equals wall within 1 ms in every cell).

| design | warm A | warm B | B/A | cold A | cold B | B/A |
|---|---:|---:|---:|---:|---:|---:|
| `compute_h2h` | 450 | 140 | 0.31 | 519 | 374 | 0.72 |
| `xor_sig` | 357 | 145 | 0.41 | 414 | 303 | 0.73 |
| `xor_types` | 358 | 123 | 0.34 | 420 | 403 | 0.96 |
| `rtl_ff` | 599 | 602 | 1.00 | 729 | 725 | 0.99 |
| `rtl_lfsr` | 371 | 356 | 0.96 | 384 | 366 | 0.95 |
| `rtl_chain` | 193 | 194 | 1.01 | 193 | 189 | 0.98 |
| `rtl_fan` | 817 | 824 | 1.01 | 821 | 829 | 1.01 |
| `rtl_bus` | 193 | 194 | 1.01 | 194 | 194 | 1.00 |
| `rtl_ram` | 63 | 63 | 1.00 | 65 | 65 | 1.00 |
| `lfsr64_xor` | 63 | 63 | 0.99 | 65 | 64 | 0.99 |
| `lfsr64` | 168 | 171 | 1.02 | 169 | 172 | 1.01 |

Bisect (`compute_h2h`, `--tier=auto`, core 9, bench builds): `00b506e8`, the parent of `6d2f237f`, 0.43 s cold and 0.16 s warm; main before the fix 0.47 s warm. The trace of the fixed warm run links `_p0` from the cache after 17 to 21 ms of sampled work (400 085 us before). The ratio against
sim1 (head-to-head 5: 0.15 warm for `compute_h2h`, 0.40 `xor_types`) is not re-measured here; at sim1's 3168 ms the warm run is 140 ms (0.04 again).

`tools/realbench --compare` (all 42 tests, run phase, `--tiers auto,auto-cold`, cores 9 and 10, default runs): wall geometric mean B/A 1.00 (auto) and 1.00
(auto-cold), instructions 1.00 and 1.00, sum of walls 0.99 and 1.05 (the cold sum is `vai_window_gen_interlock` 11.8 s to 12.8 s in that pass and 12.7 s to 12.1 s in a
second pass of three tests, instructions 1.00 both: its decisions differ between worlds by sampling, #673). Flagged and re-run alone (9 runs): `osvvm_axis_random` and
`hdlm_ring_buffer` 1.00 / 0.84 and 1.00 / 0.89 (noise of the first pass). Not explained: `fletcher_bufferwriter_512` 1.01 to 1.08 warm and 1.06 to 1.15 cold wall at
equal instructions (52.2 G) in two passes, cycles not measured (code layout, #576, or the load of the host); no `--tier=auto` decision of that test was traced.

## Temporaries of a process in its frame (#668, 2026-10-25)

`yavs_ir` `opt/slots.rs` keeps constant-size temporaries of processes (waveforms of signal
assignments, vectors read from signals, concatenations, descriptors) in frame bytes, with frame
forms of the fused `Alloc` ops in the predecoder (`ZeroSlot`, `CopyDrive`/`LSetDrive`/`LCopyDrive`
with `pre::FRAME_WAVE`, `ZeroReadVec`, `ZeroCat`, `ZeroLCopy`, `ZeroCatLCopy`, `SlotBr`, drive runs
without a marker) and in the LLVM tier (`Zero` as `memset`, `ScalarDrive::head`); `YAVS_NO_SLOTS=1`
turns it off. Landed from two independent branches (decisions.md "Temporaries of a process in its
frame"). Also: 4-byte `LoadSlot`/`LoadInst` loads sign-extend (#746), compiled frame code fetches
the slots pointer before an instruction's own branches (#751), the predecoder's single-use counts
ignore the operand lists of removed instructions (#752).

Instructions (`perf stat`, bench builds, each cell the third run of its world so the compilation
cache is in its steady state; main = 1f267217, the change's base; "off"/"on": this binary with and
without `YAVS_NO_SLOTS=1`; percentages on/off, on/main):

| design | interp main / off / on (G) | llvm main / off / on (G) |
|---|---|---|
| `neorv32_single_min` | 30.82 / 30.90 / 30.18 (-2.3 %, -2.1 %) | 41.74 / 41.78 / 40.13 (-3.9 %, -3.9 %) |
| `sc5_fifo_repack` | 36.53 / 36.67 / 36.24 (-1.2 %, -0.8 %) | 22.42 / 22.42 / 22.28 (-0.6 %, -0.6 %) |
| `fletcher_bufferwriter_512` | 51.29 / 51.39 / 50.96 (-0.8 %, -0.6 %) | 58.74 / 58.76 / 58.41 (-0.6 %, -0.6 %) |
| `poc_cache_cpu` | 7.85 / 7.96 / 7.76 (-2.6 %, -1.1 %) | 11.07 / 11.07 / 11.01 (-0.6 %, -0.6 %) |
| `sc5_timestamp` | 6.88 / 6.88 / 6.64 (-3.5 %, -3.5 %) | 5.78 / 5.78 / 5.74 (-0.8 %, -0.8 %) |
| `grlib_leon3mp` (100 us) | 82.79 / 83.05 / 83.50 (+0.5 %, +0.9 %) | 122.28 / 122.69 / 121.74 (-0.8 %, -0.4 %) |
| `poc_arith_addw` | 147.73 / 146.90 / 149.02 (+1.4 %, +0.9 %) | 124.45 / 124.46 / 122.43 (-1.6 %, -1.6 %) |

(A measurement taken one run after the warm-up, as in the first version of this table in the
branch, is not steady: the second run of a world makes cache entries the third reuses, and which
variant ran first moved designs by up to 13 percent.) Peak RSS main / on: interpreter equal within
3 percent (`fletcher_bufferwriter_512` 38.0 / 37.8 MB, `grlib_leon3mp` 243 / 243 MB,
`neorv32_single_min` 72 / 75 MB); LLVM tier `neorv32_single_min` 527 / 444 MB, `grlib_leon3mp`
1409 / 1386 MB, `fletcher_bufferwriter_512` 258 / 258 MB.

Head-to-head designs (`tools/bench/designs` group `h2h`, pinned to one core, median of 5
interleaved runs, cycles and instructions new / main): interpreter geomean cycles 1.010,
instructions 1.007 (`rtl_lfsr` 1.039 / 1.024, `lfsr64` 1.039 / 1.017, `lfsr64_xor` 1.024 / 1.019,
`xor_sig` 1.028 / 1.025; `rtl_ff` 0.982 / 0.993, `rtl_chain` 0.987 / 0.994); LLVM tier 0.980 /
0.975. The `lfsr` designs concatenate into a temporary every cycle: `ZeroCat` is out of line where
`AllocCat` is an arm of the loop, and putting it (or `ZeroReadVec`) into the loop cost 1 to 3
percent of instructions on every design through code generation of `run_ops`, so `ZeroReadVec` is
out of line too (the binary with the pass off is within -0.6 to +1.5 percent of main on the seven
designs, equal on the head-to-head ones but `compute_h2h` +0.7 %).

All 42 of `tools/realbench` (`--compare` main against this change, run phase, `--tiers interp,llvm`,
core 9, default runs, host loaded): geomean new / main of wall 0.99 (interp) and 1.00 (llvm), CPU
time 0.99 and 1.00, instructions 1.00 and 1.00; category `cpu` 0.97 and 0.97 (wall), `numeric`
1.05 and 0.99, `memory` 0.84 and 1.00 (runs of 0.01 to 0.04 s); sum of the walls 0.98 and 1.02;
`neorv32_*` 0.94 to 0.97 interp and 0.92 to 0.95 llvm, `grlib_leon3mp` 0.98 interp and 1.05 llvm
(instructions 0.99), `poc_arith_addw` 0.97 and 1.01, `fletcher_bufferwriter_512` 1.02 and 0.96.

Checked: `cargo test --workspace`, `-p yavs-llvm -p yavs --features llvm`, both gc-stress suites
(one timing-dependent failure under load, #755, passes alone); the differential corpus 524/524 on
`yavs-interp`, `yavs-llvm`, `yavs-auto` and `yavs-auto --tier-heat=1,0,0` at 1 and 4 threads, and
on the first three with `YAVS_SPEC_AFTER=2`.

## llvm: callbacks of large compiled processes (#679, 2026-10-25)

(`docs/decisions.md` "llvm: large compiled processes lose to their callbacks, ...".) The compiled bodies of the LEON3 processes were already
cheaper than interpreting them (steady state of the window with everything compiled: 8.4 k cycle samples in compiled code against 15.1 k in
the interpreter's loop); the loss was the runtime work around them. Three changes: a `LayoutOf` read for one word is one call of the layout
memo (no native-frame step), kept scalar drives skip unchanged elements in the kernel, and the object table keeps constants for the run (4 096
entries, a process record of 64 objects). Host misses in the LEON3 150 us window 13.4 million to 2.8 million; native-frame steps of
`LayoutOf` gone (`llvm_layout_field`: `cb_xstep` 0, one `cb_layout` per field read); kernel entries of kept drives of `llvm_drive_batch` 817
to 103.

LEON3 150 us, bench builds of main (78f191a7) and of the change, `perf stat --no-inherit` of the simulation thread, one claimed CCD1 core,
interleaved; heat = `--tier=auto --tier-heat=1,0,0` (everything compiled), warm compilation cache. Quiet session, medians of 5 (G):

| | cycles | instructions | frontend stall cycles |
|---|---:|---:|---:|
| interpreter | 36.75 | 101.9 | 5.16 |
| heat, main | (+5 to +10 % over the interpreter in other sessions) | 86.4 | 11.0 to 13.0 |
| heat, change | 35.84 (-2.5 %) | 78.2 | 10.00 |

A noisier session with main's heat in it (medians of 5, minimum in brackets): interpreter 42.25 (35.56), heat main 44.23 (37.58), heat change
35.96 (33.10). The part before time 0 (elaboration and the S-record load of the memory models, `--stop-time=0fs`): interpreter 7.43, heat
4.91 cycles; so the steady state of the window is level with the interpreter in cycles (instructions 60.7 G against 72.9 G). A process record
of 16 objects instead of 64 (fewer instructions, more misses in the table) was 3 percent slower in the quiet session.
Difftest 524/524 on `yavs-interp`, `yavs-llvm`, `yavs-auto` and `yavs-auto --tier-heat=1,0,0`, each at 1 and 4 threads. `tools/realbench
--compare` (all 42 tests, `--tiers auto,auto-cold`, one core, main against the change): wall geometric mean 0.99 / 1.00, instructions 1.00 /
1.00 (the default policy compiles few large processes); flagged only `grlib_leon3mp` and `uvvm_sb_slv` auto-cold wall 1.11 at equal
instructions (two runs each on a loaded host).

The whole LEON3 run (to 1060.5 us, the same method, medians of 2, G cycles / G instructions): interpreter 208 to 211 / 452; heat 270 / 459
(main) and 226 / 405 (change); `--tier=auto` 257 / 445 (main) and 222 to 242 / 416 (change); `--tier=auto` with the large processes forced
(`YAVS_AUTO_FORCE` of `comb`, `dctrl`, `ictrl`, `state_register`, the TLB `p0` and `ctrl`) 263 / 456 (main) and 242 / 413 (change). So over the
whole run compiled code is still 7 to 8 percent above the interpreter in cycles at 10 percent fewer instructions: the part after the window is
dominated by the SDRAM models (`state_register`, four instances, 90 percent of their wakes change nothing) and the TLBs, whose activations are
short, so the cost of entering compiled code (`load_objs`, `begin_kept`, `view_again`, `run_cached`: 6 percent of the cycles) and the kept
drives (`assign_codes_now`, `drive_bits`, `drive_now`, `yavs_rt_drive`: 8 percent, the interpreter's `drive_run` and its skip tests 5.7) weigh
more than in the window. Over the whole run the tier's own runtime functions are 27 percent of the cycles (6.2 k of 23.2 k samples), as much as
the compiled code itself. Tried and not kept: entering only the first 8 objects of a process record and the rest on a miss (fewer
instructions; cycles higher in two runs per cell, which on this host does not separate 5 percent: not proven either way).

Whole run in a quieter session, 7 cells interleaved, medians of 4 (minimum), G cycles / G instructions: interpreter 188.6 (184.6) / 451;
everything compiled (heat) 195.2 (192.4) / 405, +3.5 percent; default `--tier=auto` 208.6 (198.8) / 416; auto with forced subsets
(`YAVS_AUTO_FORCE`): the big set 205.4 / 411, without `state_register` 206.2 / 411, `state_register` only 197.9 (196.6) / 416, the TLB `p0` only
206.4 / 412. So the target "compiled LEON3 below the interpreter in cycles" holds for the 150 us window and not for the whole run; the default
policy itself costs about 10 percent of main-thread cycles on this test (data on #754). Heat mode on other designs (`tools/realbench --compare`,
`YAVS_BENCH_TIERFLAGS=--tier-heat=1,0,0`, 5 runs, wall / instructions, change against main): `neorv32_single_min` 0.95 / 0.99,
`fletcher_bufferwriter_512` 0.99 / 1.01, `fletcher_bufferreader` 1.02 / 1.00, `sc5_fifo_repack` 1.02 / 1.00.

## llvm: an object table keyed by machine (#679, 2026-10-26)

(`docs/decisions.md` "llvm: an object table keyed by machine".) The compiled tier's object table (`rt.rs` `Mem::objs`) keys each entry by
the address high half and a tag of its space (machine, machine heap at its epoch, runtime constants, activation volatiles) and moves the
entry by an offset of the tag, so the objects of every process stay in the table while other processes run. A change of machine walks
and restores nothing (`load_objs`, `save_objs`, `begin_kept` and the drop walks are gone); a process's record is four words (table
identity, heap epoch, two tags); the table has 16 384 entries, built zeroed in place. Lookups the runtime had to ask the interpreter for
over the whole LEON3 run: 15.7 million to 7.2 million. Test `compiled_instances` (four instances of a process with heap variables and
deallocations, interleaved, every tier against the interpreter; it fails when the instances share a heap tag) and the `rt.rs` unit tests.

Whole LEON3 run (to 1060.5 us), bench builds, `perf stat --no-inherit` of the simulation thread, one claimed CCD1 core, cells
interleaved, medians of 4 (minimum in brackets), G cycles / G instructions; heat = `--tier=auto --tier-heat=1,0,0`; each cell in its own
world, auto worlds settled by four untimed runs (the run-to-run profile). Session 1 and 2: main a9db5d80 against the change on it;
session 3 (noisier): main 504f86ad against the change rebased on it:

| | session 1 | session 2 | session 3 |
|---|---:|---:|---:|
| interpreter | 184.2 (183.5) / 452 | 185.7 (184.0) / 452 | 192.0 (183.6) / 455 |
| heat, main | 189.8 (188.9) / 405 | | 197.2 (191.6) / 402 |
| heat, change | 180.3 (179.4) / 377 | 184.9 (183.3) / 377 | 190.0 (187.0) / 374 |
| default auto, main | | 198.1 (193.2) / 436 | 186.8 (185.3) / 437 |
| default auto, change | | 182.3 (180.3) / 438 | 185.9 (184.1) / 437 |
| auto with `state_register` forced, change | | 185.0 (182.7) / 437 | |

Heat is 4 to 5 percent below main in every session and level with or below the interpreter (-2.1 percent in the quiet session 1, -1
percent median and +1.9 percent minimum in session 3). Default auto is level with main (session 3 and `tools/realbench`; session 2's -8
percent did not repeat, at equal instructions) and level with or below the interpreter: the policy compiles few large processes of this
test, so the table change reaches it little. Forcing `state_register` under the default policy gains nothing now (data on #754). Left on this test: the kept drives
(`assign_codes_now`, `drive_now`, `yavs_rt_drive`, `drive_bits`: about 7 percent of the heat run's cycles), whose time is the
kernel's cache misses on the driver, waveform and signal of each assignment; the run makes 33.9 million batch flushes of 127.6 million
kept scalar drives, 55.9 million vector drives and 10.4 million single scalar drives in 22.9 million compiled activations.
Difftest 531/531 on `yavs-interp`, `yavs-llvm`, `yavs-auto` and `yavs-auto --tier-heat=1,0,0`, each at 1 and 4 threads.
`tools/realbench --compare` (main 504f86ad against the change, run phase, `auto` and `auto-cold`, 42 tests, core 10): wall geometric mean
1.01 / 1.01, instructions 1.00 / 1.00; flagged only `poc_stat_histogram` auto (wall 1.04, instructions 1.06 in a run of 0.5 s;
repeated: instructions +4.3 percent under `--tier=llvm`, at equal callback counts). That test's time is element loops on native frames
(`yavs_rt_eloop`, `Mem::region`) and the inline object lookups of native-frame code, which now index the tag table by the address's
space (two loads more per lookup of unknown space); `Mem::region` marked `#[inline]` (it stopped being inlined into the loop's lookup)
brings it to +2.8 percent instructions and +3.2 percent cycles. The `numeric` category's auto wall of 1.06 in that run (equal CPU time
and instructions) was 1.00 in a rerun of 7 runs. A single key word per space (tag and offset in one load) is the next
step if native-frame lookups show in a profile. Heat on
the medium designs (`YAVS_BENCH_TIERFLAGS=--tier-heat=1,0,0`, 5 runs, wall / instructions): `neorv32_single_min` 0.99 / 1.00,
`fletcher_bufferwriter_512` 1.00 / 1.00, `fletcher_bufferreader` 1.01 / 1.00, `sc5_fifo_repack` 1.00 / 1.01.

## CI repair after the correctness batch (#759, 2026-10-25)

Main CI was red from `02c455f2`. Causes and fixes: (1) `E1360` of #654 also fired for a `linkage` formal, which accepts an actual of any mode (VESTs `tc778`); it now covers `out`/`inout`/`buffer` only. (2) The `vests-1993` expected list still had `tc400`, `tc2565` and `tc537` as known failures, which #657, #658 and #643 fixed: all 1939 entries pass now. (3) `generics/generic_nested_proc` (W1501 on stderr) was fixed by #758. (4) The `process/no_wait_infinite` file (#583) ran unbounded and out of memory under `--tier=auto` and `--tier=llvm` (the llvm jobs on all three runners, the `corpus_waves` test): a never-suspending process now leaves compiled code at its jump back (`docs/decisions.md`).

## The interpreter keeps the fused 32-byte blocks out of the frame (#756, 2026-10-25)

Under `--tier=interp` an `Alloc` of 32 bytes that the next instruction fills as a signal read, a
logic concatenation or an `LCopy` stays an `Alloc` (`opt/slots.rs` `fused_in_loop`), so
`AllocReadVec`, `AllocCat` and `AllocLCopy` run in the dispatch loop as before the frame
temporaries; `auto` and `llvm` are unchanged (decisions.md "Temporaries of a process in its
frame"). A frame flag on the in-loop arms was tried first and cost 2.8 percent of instructions
with the pass off. Interpreter, bench builds, one core, interleaved, median of 15 (cycles /
instructions, new over the build before #668; main = #668 over the same):

| design | main | new |
|---|---|---|
| `rtl_lfsr` | 1.031 / 1.024 | 0.988 / 0.992 |
| `lfsr64` | 1.036 / 1.018 | 0.987 / 0.989 |
| `lfsr64_xor` | 1.017 / 1.019 | 0.986 / 0.994 |
| `xor_sig` | 1.020 / 1.025 | 0.997 / 1.003 |

The other head-to-head and internal designs have the instructions of main (within 0.1 percent;
`ff_rtl` -1.0, `rtl_ram` -0.4), their cycle differences between the builds are noise on a loaded
host. Realbench (42 tests, interpreter, `--compare` main against the change, one run): wall
geomean 1.00, instructions 1.00, CPU time 1.05 (runs of 0.01 to 0.05 s); `grlib_leon3mp` wall 1.06
at 0.99 instructions in that loaded run. LLVM tier: `--tier=llvm` instructions equal to main.
Work-counter ceilings `ops` of `reg_planes` and `out_port` raised (the blocks keep their
statement markers). Checked on this commit: fmt, clippy with and without `llvm`, both test suites, the
differential corpus on `yavs-interp`, `yavs`, `yavs-llvm` and `yavs-auto --tier-heat=1,0,0` at 1
and 4 threads: 530/531 on each (the difference is the stderr warning of `process/no_wait_infinite`,
a test of another commit).

## A process that never suspends stays compiled under a 10^9 bound; the entry leak (yavs-llvm, yavs-interp, yavs-kernel, #583, #761, 2026-10-26)

- The bound on the jumps back of a process without a wait is 10^9 in the interpreter and in compiled code (`YAVS_SPIN_LIMIT` lowers
  it for tests); compiled code no longer leaves at every jump back but keeps a register-held count, stored back at every exit, and
  leaves every 2^20 jumps (`docs/decisions.md` "stays in compiled code under a bound"). `E3009`, exit status and the finished wave file are the
  interpreter's, as before.
- #761: the memory growth (about 24 bytes per assignment in both tiers) was the direct entries one activation lists until it ends;
  `Ctx::trim_run` keeps the last of identical ones at each of those exits. 12 million `a <= not a` stay near 60 MB (interpreter) and 125 MB
  (compiled) instead of +290 MB.
- Cost against the stop poll alone, release bench build, `--tier=llvm`, core 10: compute loop of 10^8 iterations (real arithmetic) 0.56 s to
  0.58 s (+3.6 percent), integer loop of 3x10^8 iterations 1.23 s to 1.24 s (within noise).
- The full default bound on `process/no_wait_infinite` (release bench build): `--tier=llvm` 6.4 s and 104 MB peak, `--tier=interp` 27.6 s and 40 MB.
- New tests: `stuck` (exact limit per tier, a 3-million loop to `finish`), `rss` (flat memory), perf `llvm_nowait`; the
  `sigint_ends_a_process_that_never_suspends*` tests set the bound out of reach instead of assuming the run lasts 1.5 s.
  Difftest header key `spin=N`; `process/no_wait_infinite` uses it.


## A function with native-frame code alone is called through the tier at once (yavs-interp, #764, 2026-10-26)

`NativeFlags::set_call` did not switch `any` on, only `set` (frame-mode code) did, and `Machine::tiered` syncs `Pre::native` (the byte the watching loop reads at a call) only while `any` holds. When a run published native-frame code and no frame-mode code anywhere (a warm `--tier=auto` run that links the native objects of `to_slvv_8`, `to_slm` and `get_row` of the PoC histogram testbench), an interpreted caller with one activation per clock never saw the flag and called the function interpreted until some function's frame-mode code appeared; which one appeared first, and when, depended on the order the objects were linked in. `set_call` now switches `any` on and `refresh` counts `NAT_CALL`. Test: `a_callee_flagged_for_native_calls_alone_is_called_through_the_tier` (`yavs-interp` `tests/sim.rs`). Not the memo: the call of that function is an `Op::Call`, and `YAVS_NO_PMEMO=1` only changed which functions got frame-mode code first.

## --profile names native-frame callees (yavs-interp, yavs, #760, 2026-10-26)

`Runtime::sampler` reads the thread's sampling slot, which names the interpreted function that made a `Tier::call`, so the time of a function running compiled on a native frame (`to_slvv_8`, `to_slm`, `get_row` of the PoC histogram testbench) counted as interpreted time of its caller. `Runtime::profile_sampler` (used by `--profile` only; the tier's policy keeps `sampler`) also reads the thread's `pc_slot`, in which `Machine::tier_call` leaves the callee marked (`NATIVE_PC`) for the length of the call once a profile exists (one relaxed load per native-frame call otherwise). Calls between native frames inside the compiled code stay unseen: the outermost callee carries their time. Test: `the_profile_names_a_native_frame_callee_not_its_caller` (`yavs-interp` `tests/sim.rs`).

## std/ieee analysed units shared across libraries (yavs, #564, 2026-10-26)

The analysed units of the built-in `std`/`ieee` live in one store per user (`$YAVS_STD_CACHE`, else `yavs/std` in the user's cache directory) instead of every library's `yavs-cache/units/`; a fresh library reads them. `YAVS_STD_CACHE=off` keeps the old per-library placement; `E0126` for an unusable directory; a mapped (bootstrap) `ieee` is never shared; `crates/yavs/build.rs` hashes `lib/` into the build id. Cold batched analysis (shared store warm, bench builds, core 8): instructions 0.72 to 0.83 of before on `en_cl_fix_pkg`, `poc_strings`, `neorv32_single_min`, `sc5_timestamp`, `fletcher_bufferreader`, wall 0.75 to 0.92; first call into an empty store unchanged. Decision and numbers: `docs/decisions.md` "Shared std/ieee units". `--std-cache=DIR|off` on `analyze`/`elab`/`run`/`cache clean` (wins over the variable); `yavs cache clean` also clears the shared store; every harness in `tools/` uses a private store. Tests: `crates/yavs/tests/std_share.rs`.

## Byte codes only for signals with element reads (yavs-sem, yavs-elab, yavs-kernel, #734, 2026-10-26)

The elaborator decides which logic signals keep the per-element byte codes: those that have a read of one element (`Db::facts()` `elem_reads`, `yavs-elab` `keep_codes.rs`, `Builder::no_codes`); the source-width condition of #519 stays for signals above 64 elements (user, 2026-10-26). Signals without codes are read from the planes and still skip assignments of the value they hold. Instructions of whole runs (interpreter / llvm): `lfsr64` 0.97 / 0.95, `lfsr64_xor` 0.98 / 0.97, `rtl_ram` 0.99; `rtl_lfsr`, `rtl_ff`, `rtl_chain` read single elements of their registers and keep their codes (level, at most +0.5% from layout); `fletcher_bufferwriter_512` level thanks to the width condition. Details and the open items: `docs/decisions.md` "Byte codes by element reads".

## Element read facts without connections, sensitivity entries and parameter reads (yavs-sem, #734, 2026-10-26)

Port-map actuals (`v(3)`), sensitivity entries and element reads through signal parameters no longer make a signal keep its byte codes: a scalar port read whole is the read, and a signal passed to a parameter that reads an element of it is found by a fixpoint over the calls (`docs/decisions.md` "Element read facts without connections, sensitivity entries and parameter reads"). `no_code_signals`: `osvvm_axis_random` 0 to 21, `grlib_leon3mp` 0 to 317, `uvvm_axistream_vvc` 0 to 5; instructions level except `grlib_leon3mp` +0.7 percent.

The skip test of a signal without byte codes reads the planes inline (`plane_is`, no call): `grlib_leon3mp` instructions -0.9 percent against 30241dbc, and below the run that kept the 317 codes; other designs level (`docs/decisions.md` "The skip test of a signal without byte codes reads the planes inline").

## Table reads with a range test or a validity mask (yavs-ir `opt/inline.rs`, #769, 2026-10-26)

A result-table read for an argument of an integer type (range test) or with values that report (validity mask) now takes the rest of its block into the table arm before the first optimiser pass
(`docs/decisions.md` "Table reads with a range test or a validity mask"), so the fat pair of an array result is not made and read back: perf case `tab_tail` 22 072 to 15 764 ops, a digit loop 26.8 to 18.9
ops (11.8 with a proven range). The late inliner no longer expands table functions (a miss is the test and the call). `grlib_leon3mp` is unchanged (its table is whole). Left: #771, #770, #768.

## Invariant loads and dereferences leave fill loops (yavs-ir `opt/licm.rs`, #768, 2026-10-26)

`Init_mem` of the SDRAM model (rows of bit vectors behind access values, filled one element at a time) is no longer 18 percent of the first 1 us of `grlib_leon3mp`: the loads, the dereference and the
descriptor reads of such a loop move before it (`docs/decisions.md` "Loop-invariant loads and dereferences"), and the inner loop runs as one element-loop op. The reproducer of the issue: 5.08 M ops to
0.26 M; the first 1 us: 31.37 G to 25.96 G instructions (-17.2 percent) with `--tier=interp`. Left: #771, #770.

## `is_x` of a slice without a copy (yavs-interp, #770, 2026-10-26)

`Is_X (v (a to b))` tests the plane words of `v` where they are (`Op::IsXSlice`, `docs/decisions.md` "`is_x` of a slice of a logic vector"): no temporary, no native call. `grlib_leon3mp`, first 1 us, `--tier=interp`: 25.97 G to 23.07 G
instructions (-11.1 percent). The other natives on slices (`to_x01`, the numeric operators) still copy the slice first.

## Branches with equal arms (yavs-ir `opt/cfg.rs`, #771, 2026-10-26)

A `CondBr` whose arms lead to the same block is a `Br` (the null-range test of a local array). Part of the fixed cost of an `HRead` call (`docs/decisions.md` "A branch whose arms lead to the same block"):
the call is still about 80 ops; the rest is listed there. First 1 us of `grlib_leon3mp` with #768 to #771: 31.37 G to 22.99 G instructions (-26.7 percent), `--tier=interp`.

## kernel round 28, not landed: a staging buffer per direct driver (#802, 2026-10-28)

- Prototype (staged plane words per direct driver in place of the `out`/`outv` entry lists) measured and not landed:
  `sc5_fifo_repack` -0.9 percent instructions, `hdlm_lfsr` -1.5 to +0.9 percent, `grlib_leon3mp` +2.6 to +6 percent (its held drives
  lost the assign-time skip). A twin test of direct drives against the general paths is kept
  (`direct_paths::direct_drives_match_the_general_path`). Details: decisions.md "Kernel round 28, not landed".

## kernel round 27: adjacent vector entries of a direct driver join (#676, #793, 2026-10-28)

- A vector entry next to the run's last vector entry of the same direct driver joins it (a loop over the bytes of a vector, a sorting
  network's lanes): `sc5_fifo_repack` -2.9 percent instructions, -2 to -3 percent cycles; `realbench --compare` 42 tests level
  (instructions 1.00 / 1.00). #793 (store-forwarding stall of the vector entry) measured: removing the reload costs cycles, not kept.
  Staging per direct driver proposed in #802. Details: decisions.md "Kernel round 27".

## kernel round 26: the slot of the process ahead, element drive batches as one vector entry (#795, #676, 2026-10-27)

- The batch loop also prefetches the slot (kernel state) of the process three steps ahead: `rtl_ff` -15 to -25 percent cycles on a
  busy host, level on a quiet one. A batch of compiled element drives that covers a run of at most 64 elements (a shift register loop)
  is one vector entry: `hdlm_lfsr` one-element commits 16.1 M to 1.1 M, cycles -9 to -13 percent, run wall 1.42 to 1.34 s.
- `realbench --compare` 42 tests: wall 0.98 (interp) / 1.00 (auto), instructions 1.00 / 1.00. Head-to-head designs against sim1 unchanged
  (`rtl_lfsr` 262 ms, `lfsr64` 131 ms, `rtl_ff` 545 ms warm): the kernel is about a quarter of their activation; the rest is the
  interpreted start of warm runs (#728) and the host layers (#766). Details: decisions.md "Kernel round 26".

## llvm and clift: compiled clocked RTL, round 11: lean activations (#766, 2026-10-28)

- A process that calls nothing and reaches no heap, access, package or shared object (`yavs_llvm::regs::no_table`, an IR analysis both
  code generators share) is `lean`: its activations skip the object table's machine and heap tags (`Mem::begin`, `save_tag`) and the
  native-call set-up (`set_nslots`, `ntab`, `stack_lo`), under `--tier=auto`, `llvm`, `clift` and `clift-auto` alike, every clock-edge
  form alike (`tests/compiled_lean.rs`). Debug builds and `--features table-check` check that a lean activation never reaches the table.
- Instructions an activation (stepcount, base e1e9dc1f / round 11): `rtl_lfsr` 1 384.6 / 1 308.8 (LLVM build), 1 465.3 / 1 389.5
  (Cranelift build); `rtl_ff` 1 196.5 / 1 120.5 and 1 233.5 / 1 157.5; `lfsr64` (not lean: a vector variable on the heap, #851)
  1 442.6 / 1 446.6. The host layers 496 to 420. Cycles warm (min of 7, one core): `rtl_lfsr` -5.9 percent (LLVM build), -5.8 (Cranelift build),
  `rtl_ff` -3.9 and -5.9; against sim1 warm `rtl_lfsr` 1.33 to 1.26 and 1.45 to 1.43, `rtl_ff` 1.00 to 0.94 and 1.08 to 0.97.
  `realbench --compare` 42 tests: instructions 1.00 (interp and auto, both builds), wall 1.00 to 1.03 (load about 20).
- Checks: fmt, clippy (default, `llvm`, `clift`, both), `cargo test --workspace`, the `llvm` and `clift` tests, `gc-stress` of
  `yavs-interp`, `yavs-llvm` and `yavs-clift`; difftest 548/548 at 1 and 4 threads for `yavs-interp`, `yavs-auto` (also with
  `--tier-heat=1,0,0`), `yavs-llvm`, `yavs-clift`, `yavs-clift-auto` (also heat) with a `table-check` build, and `yavs-auto`/`yavs-clift-auto`
  heat with the bench builds.
- Measured, not kept: kept scalar drives across a vector drive of another kernel driver (`sc5_fifo_repack` -0.7 percent
  instructions with LLVM, +0.6 with Cranelift; `hdlm_lfsr` -0.1 / +1.1); `tests/compiled_drive_order.rs` pins the order of kept and
  vector drives of merged slots.
- Left (decisions.md "Compiled clocked RTL processes, round 11"): the view's fields, `Env`, the thread's `Mem`, the record's copy in
  `run_direct`; process variables on the heap (#851).

## llvm: compiled clocked RTL, round 10: the view made inside the tier (#766, 2026-10-28)

- `Tier::run_cached` takes the interpreter's `yavs_interp::Host` (now `pub`) instead of `dyn TierHost`; `rt::enter`/`enter_on` are generic
  over `rt::View`, so a process activation's view is `Host::view_root_again` inlined into the tier. `run_direct` reads the function inline
  (`Program::func_inline`), tests the host's fault in place and ends the host without drop calls; small trims in `enter_on`, `keep_heap`
  and the view's flags. A second code generator changes only its `run_cached` parameter type (#481).
- Instructions an activation (stepcount, base 3d0e1f3d / round 10): `rtl_lfsr` 1422.5 / 1380.6, `lfsr64` 1475.1 / 1433.2, `rtl_ff`
  1116.5 / 1074.5; the host layers (`InterpProcess::run` with `run_direct` and `Program::func`, `run_cached` with `enter_on` and the view,
  `yavs_rt_wait_same`) 538 / 496. Cycles, warm, min of 7, core 9: `--tier=auto` `rtl_ff` -2.6 percent, `lfsr64` -1.6, `rtl_lfsr` -0.9;
  `--tier-heat=1,0,0` `rtl_ff` -3.2, `rtl_lfsr` -1.9, `lfsr64` -1.4; `rtl_chain` (interpreted) level.
- `tools/realbench --compare` (42 tests, run phase, interpreter and auto, 3 runs, core 8): wall geometric mean 0.99 / 1.00, instructions
  1.00 / 1.00, CPU 0.99 / 1.00; `hdlm_lfsr` auto 0.94 wall and instructions; `vai_window_gen_interlock` auto instructions 1.10 in that run,
  0.98 in 5 more runs (the policy's compile timing).
- Checks: fmt, clippy with and without `llvm`, both test suites, `gc-stress` tests of `yavs-interp` and `yavs-llvm`; difftest 542/542 at
  1 and 4 threads for `yavs-interp`, `yavs-auto`, `yavs-llvm` and `yavs-auto` with `--tier-heat=1,0,0`.
- Left (decisions.md "Compiled clocked RTL processes, round 10"): the view's fields, the object table's tags, the policy's bookkeeping,
  `Env` and the thread's `Mem`, in steps of 3 to 15 instructions.

## kernel: prefetch of the next process in a batch (#766, 2026-10-27)

- A batch of 128 or more processes prefetches the five hot cache lines of the process three steps ahead (`yavs-prefetch`, the one
  `unsafe` call; no-op off x86_64; `InterpProcess` is `repr(C)` with its machine first). Cycles, warm, minimum of 7 on one core:
  `rtl_ff` -9.2 percent `--tier=auto` / -11.1 interpreter, `rtl_lfsr` -4.4 / -2.4, others -0.1 to -2.2; `realbench --compare` 42 tests
  level (wall 0.97 / 1.00, CPU 1.00 / 0.95). Details: decisions.md "Prefetch of the next process in a batch".

## llvm: compiled clocked RTL, round 9: the tier's record inline in the machine (#766, 2026-10-26)

- `Tier::run_cached` takes `&mut TierState`; the record lives inline in `Machine::tier_state`, in the machine's fifth cache line with
  `troots` (was a `Box` of its own per process). A second code generator only changes its `run_cached` signature.
- Instructions an activation (stepcount, base 1585475c / round 9): `rtl_ff` 1189.5 / 1178.5, `rtl_lfsr` 1378.8 / 1367.8, `lfsr64`
  1429.3 / 1418.2; L2 data misses of `rtl_ff` about one fewer an activation. Cycles, warm `--tier=auto`, two batches of 7 on one CCD1
  core: `rtl_ff` -4.4 / -2.0 percent, `lfsr64` -3.3 / -1.0, `rtl_lfsr` -1.0 / -0.5, `rtl_chain` +1.3 / -0.5 (interpreted).
- Checks: fmt, clippy with and without `llvm`, both test suites, `gc-stress` tests of `yavs-interp` and `yavs-llvm`; difftest 535/535
  at 1 and 4 threads for `yavs-interp`, `yavs-auto`, `yavs-llvm` and `yavs-auto` with `--tier-heat=1,0,0`.
- `tools/realbench --compare` (42 tests): run-phase wall geometric mean 1.00 (interpreter) and 1.00 (auto), instructions 1.00 / 1.01;
  `neorv32_*` level, `grlib_leon3mp` 0.87 wall at equal instructions (host load fell during the run; not claimed).
- Left (decisions.md "Compiled clocked RTL processes, round 9"): the entry is field copies of 3 to 15 instructions each; the cycles are
  first-touch misses of each process's separate allocations.

## llvm: compiled clocked RTL, round 8: drives and waits on the kernel, every edge form alike (#766, #772, 2026-10-26)

- Zero-delay drives (`yavs_rt_drive_bit`, `yavs_rt_drive` of 1 to 64 logic elements or one byte scalar, `yavs_rt_drive_flush`) and
  waits (`yavs_rt_wait_same`, `yavs_rt_wait`, new `yavs_rt_wait_edge`) of compiled code reach the kernel without the host
  (`TierFrame::kctx`, `drivers`; `yavs_interp::direct_*`); the host path stays for what the kernel's fastest path refuses.
- A compiled `wait until <edge>` is a static wait (`yavs_interp::static_wait_edge`): `rising_edge`, `'event`, `not 'stable` and
  `wait until rising_edge` run the same instructions under `--tier=llvm` and `--tier=auto` (`llvm_edge_cost_parity`).
- `SigRead` of up to 64 elements within one plane word reads the kernel's planes inline.
- Instructions an activation (stepcount, `--tier=auto --tier-heat=1,0,0`), main 7076335e / round: `rtl_ff` 1107 / 1067, `rtl_lfsr`
  1462 / 1385, `lfsr64` 1656 / 1618, `ff_waituntil` 1316 / 1067. Cycles of warm `--tier=auto` runs (min of 7, one CCD1 core):
  `rtl_ff` 0.925, `rtl_lfsr` 0.970, `lfsr64` 0.964, `rtl_chain` 1.009 (interpreted, unchanged code), `ff_waituntil` 0.842.
- Head-to-head against sim1 (`--tiers auto --instructions`, instruction ratio): `rtl_ff` 1.23 to 1.18, `rtl_lfsr` 1.52 to 1.49,
  `lfsr64` 1.52 to 1.48, `rtl_chain` 1.62.
- Checks: fmt, clippy with and without `llvm`, both test suites and the `gc-stress` llvm tests; difftest 531/531 at 1 and 4 threads for
  `yavs-interp`, `yavs-auto`, `yavs-llvm` and `yavs-auto` with `--tier-heat=1,0,0`.
- `tools/realbench --compare` (42 tests, interpreter and `--tier=auto`, one core, load 15): run-phase wall geometric mean 0.99
  (interpreter) and 1.01 (auto), instructions level; the one auto cell above noise (`grlib_leon3mp` 1.25 at equal instructions) repeated
  at 0.99 in five runs a cell at load 8, `poc_cache_cpu` 0.98. `neorv32_single_min` 2.75 / 2.78 s interpreter, 2.83 / 2.81 s auto.
- Every function compiled (`YAVS_BENCH_TIERFLAGS=--tier-heat=1,0,0`, three runs a cell, load 9): `grlib_leon3mp` 46.45 to 45.82 s
  (instructions 0.99: the kept drives' flush on the kernel), `neorv32_single_min` 4.37 to 4.40 s (instructions 1.01).

## The fixed cost of an `HRead` call and its digit loop (yavs-lower, yavs-ir, yavs-interp, #771, 2026-10-26)

READ of a string takes its target as data and bounds (no pair per call), a local array of run-time bounds keeps its descriptor in its own slot (three ops instead of five), the optimiser keeps the lengths it knows
for that slot and for a parameter's descriptor (the `SubtypeCheck` of `variable sv : v (0 to sl - 1)` against `sl` goes), the length of a vector leaves a loop that copies elements and calls `is_x`, and the digit
loop of `HRead` is five ops (was eleven): `PosIndexLoadIndex`, `SliceAff`, `AffLCopy`, `StmtIsXBr`, `CmpBrStep` (`docs/decisions.md` "The fixed cost of an `HRead` call and its digit loop"). An 8-bit `HRead` is 56 ops
(was 86). First 1 us of `grlib_leon3mp`, `--tier=interp`: 23.14 G to 20.37 G instructions (-12.0 percent), 6.67 G to 5.99 G cycles; `--tier=auto` -5.5 percent instructions; `--tier=llvm` -2.0 percent. Left: the other
natives on slices (`to_x01`, the numeric operators) still copy the slice first; `Call`/`Ret` and the textio natives (`CallNative` is 6 percent of the first 1 us) are the next cost of the call.

## `--tier=auto`: large functions with loops, `Kind::BigLoop` (yavs-llvm `auto.rs`, #679, 2026-10-26)

- A function compiled at `Fast` (300 IR instructions or more, not a compute loop below 1 500) with 10 percent or more of its IR in loops is `BigLoop`
  (speedup 1.15, a candidate at about 7.7 times its price); straight-line ones stay `Big` (1.037). `YAVS_AUTO_TRACE` `seen` lines print each
  function's loop share. Decision and measurements: `docs/decisions.md` "`--tier=auto`: large functions with loops are a kind of their own".
- Per process, compiled against interpreted (warm `--profile` pairs, forced at the first look): window generator `bus_process` 1.16, `walk_control`
  1.62, `present` 1.20, `control` 1.12; `fletcher_bufferwriter_512` `ctrl_comb_proc` 1.02; `grlib_leon3mp` 0.86 (`iu:comb`) to 1.36 (`ictrl`), level over the
  run; `neorv32_single_min` `counter_increment` 1.03.
- `tools/realbench --compare` (bench builds, 30241dbc against the change, run phase, cores 9 and 10, load of other agents on the host):
  42 tests, `auto` sum 1.00 and geometric mean (tests of 1 s or more) 0.99, `auto-cold` 1.02 and 1.02 at equal instructions (the cells above 1.05
  were noise: `poc_arith_addw`, `fletcher_bufferwriter_512`, `neorv32_dual_full` 0.98 to 1.00 in seven runs a cell). `vai_window_gen_interlock` warm 0.91
  wall, 0.87 instructions (seven runs), cold 1.01 to 1.03 wall, 1.01 instructions.
- The run-to-run profile takes `BigLoop` processes (or processes with a `BigLoop` copy) too (maintainer, 2026-10-27): `walk_control`
  compiles early from the third warm run of the window generator and is kept; window generator warm 0.93 wall, 0.96 instructions, cold level.
- Cache hits (#785): a copy is keyed by its twin key (one bitcode, not two: copies 25 to 30 percent cheaper to link), `Gen::func` resets only
  the registers a block wrote, `HIT` priced at 1 ms + 20 us an IR instruction as measured (`objtimes hit`); window generator warm 0.96 wall.
  Skipping generation on a hit needs a key decision (#785).
- Against 70cebdc0 (the per-op interpreter round) the window generator with these changes is warm 1.01 wall at 0.92 instructions, cold 1.01.
  42 tests against 463bae9b (`tools/realbench --compare`, run phase, core 8): `auto` sum 0.98, geometric mean (1 s or more) 0.99; `auto-cold`
  1.03 / 1.01 at equal instructions; window generator warm 1.05 wall at 0.92 instructions (three runs; consecutive warm runs of both binaries
  pairwise within 5 percent either way), GRLIB 0.97 / 1.06 cold at equal instructions. With today's faster interpreter `walk_control` is seen
  0.32 to 0.41 s a run, so 4 runs of it stay under its 1.70 s threshold and the profile lists it without compiling it.
  Against sim1 (run cell = elab + run, same session, 5b2beb73): window generator warm 1.56, cold 2.30; `fletcher_bufferwriter_512` 2.10 / 2.15;
  `fletcher_bufferreader` 1.48 / 1.49; `grlib_leon3mp` 3.07 / 3.19.
- Left: `iu:comb` and the other straight-line GRLIB processes stay level compiled (drives, signal reads and natives through the same runtime
  functions); running interpreted callees in place from compiled code was tried and dropped (window generator 1.12, decisions.md); a cache hit
  still generates the module.

## `--tier=auto`: compiled code found before it is generated, profile time summed over runs, warm cache price (yavs-llvm, yavs-ir, yavs-elab, #785, #679, #728, 2026-10-27)

- **Pre-key (#785, maintainer 2026-10-27):** a warm run links a function's object by a key made before generation (program identity from the
  stage cache, the function's optimised IR, level, part, target, build); no generation on a hit. `YAVS_LLVM_PREKEY_CHECK=1` (CI `llvm` jobs,
  difftest candidate `yavs-auto-warm`) generates anyway and aborts on another module. `HIT` is 1 ms + 1 us an IR instruction. GRLIB warm with
  the large copies forced: 52 hits cost the compile thread 0.07 s (35 hits by the IR key on main: 5.26 s); a 19 867 IR function links in
  9.5 ms (was about 790). Decision: `docs/decisions.md` "llvm: compiled code found before it is generated".
- **Profile (#679, maintainer 2026-10-27):** a listed process compiles early once the seen time of the earlier runs, summed, repays its
  price (was one run's times 4). Window generator: `walk_control` compiles early from run 5 (main: run 2) and is kept; steady state equal.
- `tools/realbench --compare` (bench builds, 097f20ab against all three changes, run phase, one claimed CCD1 core, host load 6 to 13):
  42 tests `auto` geometric mean 0.99, `auto-cold` 1.00, instructions 0.99 / 1.00; `grlib_leon3mp` 0.99 warm, 0.96 cold (34.4 to 32.8 s
  warm). Seven-run reruns of the cells off by more than 5 percent: `vai_window_gen_interlock` 0.98 / 0.98 (was 1.13 at equal instructions),
  `sc5_fifo_repack` 0.99 / 1.00, `uvvm_axistream_vvc` 0.98 / 1.00, `osvvm_axis_random` 1.06 / 1.03 (0.01 s), `poc_arith_addw` 1.06 / 0.92
  (instructions 1.02 / 0.99; 0.98 to 1.06 warm over three sessions, the same functions compiled, all earlier).
  Earlier session against 12e07dce (#785 and #679 only): 41 tests `auto` 0.97, `auto-cold` 1.01.
- **Warm runs and the cache price (#728, maintainer 2026-10-27, option (c)):** a function the cache holds waits only for its payback at the
  cache price (no `Heat::threshold` floor) and a fully cached tree is picked ahead of the budget; `--tier-heat` and `YAVS_AUTO_PRICE=model`
  keep the model price. Warm, 9 interleaved runs against the same build without it (on 12e07dce): `rtl_lfsr` 0.93 wall (0.91
  instructions), `rtl_ff` 0.92, `lfsr64` 0.99. Against 097f20ab with all three changes (15 runs, load spikes): `rtl_ff` 0.97, `rtl_lfsr`
  1.00 (its `_p1` links at 24 ms, was 52, of a 250 ms run), `lfsr64` 0.99.
- Left: with the sum a process that the 4-run projection compiled in run 2 waits until its seen time has repaid it (window generator runs
  2 to 4 0.1 to 0.5 s slower than main's); a process under about 5 percent of a run gains near the judgement's 3 percent when compiled
  early, so it is kept or lost by noise.

## Value movement: copies between heap objects, scratch sets (yavs-interp, #677, 2026-10-27)

A copy between two objects of one heap (access values, shared variables of protected types) goes straight across (`Heap::data_pair`) instead of through the staging buffer, and the thread scratch keeps 16 register sets (was 4) so the nine large non-resident processes of `grlib_leon3mp` find their registers filled (`docs/decisions.md` "Value movement"). Interleaved `realbench --compare` (42 tests, cores 10 and 11, 3 runs): `uvvm_axistream_vvc` 0.93 interp / 0.92 auto (instructions 0.96), `grlib_leon3mp` 0.97 / 0.98, geometric mean of the run phase 0.98 / 0.98, nothing worse than noise. Left: frame fills of a call (`Pre::fill`, 8 GB of `memcpy` in `grlib_leon3mp`), `wide::eq_range` and `Signal::direct_same_planes` of same-value drives, `sig_gather`/`scatter_leaves` per-leaf work (runs of scalar leaves tried: no gain, dropped).

## Signal I/O of large processes: whole values by groups, gathers and scatters through the signal callback (yavs-interp, yavs-llvm, #790, 2026-10-27)

A whole record value (`SigGather`, `SigScatter` without a part), arrays of records inside it included, is handled by the instances of its leaves
listed when the plan is bound (`Bound::items`, at most 4096): logic items of one plane word of one kernel signal are read from that word in
registers, logic items next to each other in one signal are driven as one run of the driver with their planes built in registers. The
instances of a logic leaf go last first, as the kernel mirrors them, so an array of records in a record is one run per leaf, not one assignment
per element (also for parts). The plan in the machine's list is boxed and a constant descriptor is recognised by its address. Compiled
frame-mode code runs both instructions through `yavs_rt_signal` instead of a step. Decision and measurements: `docs/decisions.md`
"Signal I/O of large processes". Test `instances_of_record_leaves` (`crates/yavs/tests/value_move.rs`, every tier).

- Bench builds, `perf stat` user instructions, warm, one claimed core of the second CCD (cycles on a host at load 13 to 16 vary by 10 to
  30 percent between identical runs and are not a result here): `grlib_leon3mp` whole run `--tier=interp` 389.61 G to 384.57 G (-1.3 percent),
  `--tier=auto` with `iu:comb` forced compiled (`YAVS_AUTO_FORCE=comb@1000#0`) 395.3 to 396.3 G against 389.8 to 391.7 G (-1.4 percent); a
  synthetic process reading and driving a 40-leaf record with an array of four records every clock: interpreter -19 percent instructions and
  -15 percent cycles, `--tier=llvm` -21 percent and -18 percent.
- `tools/realbench --compare` (12e07dce against the change, 41 tests, run phase, core 14; `grlib_leon3mp` failed on both sides from the tool,
  #794): wall geometric mean 1.01 interp / 0.99 auto, instructions 1.00 / 0.99; records category 0.97 / 0.98 wall at 1.00 instructions;
  `fletcher_bufferwriter_512`, `fletcher_bufferreader`, `neorv32_*` and `uvvm_axistream_vvc` within 2 percent either way. Difftest 542/542 at 1
  and 4 threads with interp, auto, llvm and auto with `--tier-heat=1,0,0`.
- `grlib_leon3mp` (after the #794 fix, `tools/realbench --compare`, core 10, interleaved): interp 0.97 wall / 0.99 instructions, auto 1.00 / 0.99.
  Against sim1 (run cell, `--tier=auto` warm / cold, same core, base 12e07dce then the change, consecutive sessions at load 12): `grlib_leon3mp`
  instructions 2.29 / 2.41 to 2.26 / 2.38 (wall 2.98 / 3.25 and 3.40 / 3.26: the wall of the two sessions differs by more than the change,
  see the interleaved figure above), `fletcher_bufferwriter_512` 2.13 / 2.21 to 2.02 / 2.18 (instructions 1.81 / 1.91 to 1.78 / 1.91),
  `fletcher_bufferreader` 1.33 / 1.34 to 1.41 / 1.43 (instructions 1.24 / 1.26 to 1.25 / 1.27, wall within the noise of the host).
- Left (#790): the kernel's per-drive work of scalar drives (`DriveRun` about 257 instructions a drive in GRLIB, `LSetDrive` 244), a store-forwarding
  stall in the kernel's vector entry of a direct driver (#793), a gather straight into its destination (under half a percent), and per-leaf code
  generated for static layouts, not started: the interpreted callees of a compiled `iu:comb` stay the larger part (#679).

## Large value copies: records built in their own object (yavs-ir, yavs-interp, #792, 2026-10-27)

A large initialised record variable or heap node is created in its own object and filled there instead of being built in a temporary and copied (`Opt::defaults_first`); `LFill`s of 'U' into that fresh zero object are dropped; `NewFrame` appends its copy without a zero fill first; small temporary zero fills are one fixed-size store (`docs/decisions.md` "Large value copies"). `uvvm_axistream_vvc`: cycles 0.92 (interp) and 0.92 (auto) of main, memory traffic 23 to 17.7 GB; `realbench --compare` (42 tests) instructions 0.99. Left: the required whole-record assignments of the command path (copy-on-write or sparse values), the frame fill of calls. Copy-on-write of large objects was measured and not built (#798): ceiling about 6 percent of `uvvm_axistream_vvc`, under 1.2 percent on the other verification-library tests, the test is 0.72 of sim1 (`docs/decisions.md` "Large value copies").
## Jump threading in processes (yavs-ir `opt/cfg.rs`, #791, 2026-10-27)

The clean-up of short-circuit chains (`cfg.rs`, #444) runs on processes too, after the memo: a constant flag that the join of an `and`/`or`
tests is decided where it is set, the large processes' dead definitions go (the liveness bound counts the tracked registers only), and the
dead blocks of a specialised copy jump to the next block so fusions run through them. `grlib_leon3mp` -2.9 percent instructions under
`--tier=interp` and `auto`, `fletcher_bufferwriter_512`, `fletcher_bufferreader` and `neorv32_single_min` -1 to -1.5 percent interpreted
(`docs/decisions.md` "Interpreter execution of large processes"). `op-census` prints ops by function and by position.

## Mode checks of the parts of a port in an individual association (yavs-sem, #654, 2026-10-28)

The actual of a part of an `out`/`inout`/`buffer` port (`o(0) => p1`) gets the checks of a whole port's actual (`check_written_actual`): an `in` port or parameter is `E1360`, and so is an expression that is not a signal name, which stopped elaboration with an internal error before.

## Conversions in individual port associations (yavs-sem, yavs-elab, #611, 2026-10-28)

`r.x => popcount(s)` (a conversion of a signal as the actual of a part of an `in` port) is a converted port edge, as `x => popcount(s)` on a whole port: the HIR's individual part carries the flag (analysed-unit cache `FORMAT` 9). Corpus test `hierarchy/part_conversion_actual`.

## Index constraints of element subtypes and of elaborated signals (yavs-lower, yavs-elab, #637, 2026-10-28)

The index ranges of record element and array element constraints are checked against the index subtype where a composite descriptor is filled (`fill_desc`, `E3009` at run time), and those elaboration evaluates (a signal's `string(g to 3)`) in `Model::shape_c` (`E2107`). An architecture constant nothing reads has its index constraint checked at the declaration too (`walk::decls`, #825; elab test `unused_constant_index_constraint_is_checked`, discrepancy `unused-constant-index-constraint`). Tests: corpus `composite/index_constraint_elem` (`ref=hand`: sim1 does not check it), elab `index_constraint_outside_the_index_subtype`, discrepancy `index-constraint-element-bound`.

## Bounds of local types and subtypes evaluated at the declaration (yavs-lower, #612, 2026-10-28)

A type or subtype declared in a process or subprogram evaluates its non-static ranges once, at the declaration, into frame slots its uses read (`Fx::declared_ranges`): reading a variable that changes later gave a different length at each use before. Tests: corpus `process/subtype_bounds_once`, discrepancy `subtype-range-bound-variable`.

Types and subtypes of packages, entities, architectures, blocks and generates whose bounds are not globally static (an impure call, `Db::dynamic_range`) evaluate them once at the declaration, used or not: a package's in its initialisation into package data (`Fx::pkg_ranges`; elaboration reads them through `Lowerer::package_range` and `Session::package_range`, and a package with such a type is a run-time package), the others in `walk::decls` (`Model::ranges`, the first evaluation of the declaration, which also gives signal and constant shapes), written into instance data for the processes (`UnitIr::ranges`, `Space::Inst` entries of `Cx::decl_ranges`). A subtype that only names another (`subtype s2 is s`) evaluates nothing again (`Db::declared_ranges`). Tests: corpus `hierarchy/type_bounds_once`, discrepancy `subtype-of-subtype-bound-once`.

## "A similar name exists" suggestions (yavs-sem, #853, 2026-10-28)

Unknown-name diagnostics share `suggest::similar` and `suggest::help`: edit distance within `max(1, len/3)` or an abbreviation (`cnt` for `count`), best one or two equally close, worded `a signal with a similar name exists: `count`` with the declaration's spelling and kind (`Db::decl_noun`), plus a label at the declaration when it is in the same file. Sites: undeclared names, selected names, record elements (also in expressions, where `r.valu` was a bare E1361), formals in port and generic maps, attributes, libraries, design units (including the built-in `std`/`ieee` units) and architectures. A package that declares exactly the name now takes precedence over a near miss. Snapshots: `tests/diag/sem/**/*help*.vhd`.

## Thread budget: a token pool in `MAKEFLAGS` (yavs, yavs-kernel, #801 M3, 2026-10-28)

`yavs run --threads=N` with N above 1 takes its extra threads from a token pool named in `MAKEFLAGS`
(`--jobserver-auth=R,W` on Linux, `fifo:PATH` on Unix): at most N - 1 tokens, taken without blocking only for a batch worth
pooling, an empty pool runs the batch inline; tokens go back after 64 batches in a row that did not pool, when a run
returns (also after an error) and on the first Ctrl-C/SIGTERM/SIGHUP; an unusable pool is error E0128 before analysis
(`docs/parallel.md` "Thread budget across parallel test runs", `docs/decisions.md` "Thread budget").
`crates/yavs/tests/jobserver.rs` checks, for a named pipe and for inherited descriptors, the cap, the tokens back after
the end, a failure and Ctrl-C (plain and with `--wave`), no pooled batch with an empty pool, identical output at 0, 1, 3
and 8 tokens and without a pool, and E0128 for unusable pools; a pipe made by a separate parent with distinct `R,W`
descriptors was checked by hand. Not
done: the Windows semaphore form, descriptors on Unix systems other than Linux, the producer side, and the M0/M1
measurements of #801 (speedup and tail gain); none of this was timed.
