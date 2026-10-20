# Status

## Milestones

| Milestone | State |
|---|---|
| M0: workspace, lexer and parser | done: `yavs parse`, corpora below |
| M1: semantic analysis | in progress: slices S1 to S3 mostly in, `docs/design/sem.md` |
| M2: IR, interpreter, elaboration, kernel | done: the differential corpus passes at 1 thread, 4 threads and 4 shuffled threads (`tests/corpus/RESULTS.md`); see the lower, ir and interp, elab and kernel sections. Not done: binary file read/write of records and arrays of composites (ir and interp section) |
| M4: external test runner end to end | first external test runner testbench runs (#110): `VUNIT_SIMULATOR=yavs python run.py` on a pass/fail `test_runner` and the runner's `examples/vhdl/user_guide` give the right pass/fail per test with the runner's `rsim-integration` branch; the runner's own suites: all 11 pass completely (1357 tests), table below (#112) |
| M7: waves | `yavs run --wave` writes an FST of the whole design (#129, #116): hierarchy, records/arrays as scopes, enum tables, globs and size cap, final value per time step, same file at any `--threads`, finished on Ctrl-C; signals left out by globs or the cap cost nothing at run time; instances in generates, 2-D arrays; read back with wellen in `crates/yavs/tests/wave.rs`. integers carry the subtype name and the width of their range, generate alternatives keep their label (#306). `--wave-variables` records process and non-protected shared variables, records and arrays as scopes, integers with the width of their elaborated range, sampled at the end of each time step through the observer accessors (#305, #348, #321). Open: protected-type shared variables |
| M3: full types and libraries | done per `docs/plan.md`: arrays, records, access and protected types, resolution (composite, access, protected and resolution groups in `tests/corpus/RESULTS.md`); `numeric_std`, fixed and float from source (#94); textio, `env`, package generics, external names; `--load` foreign subprograms (#95); bit-plane layout (user, 2026-10-18). Not done: binary file read/write of records and arrays of composites (ir and interp section) |
| M8: DAP server | first slice (#130): `yavs run --dap=PORT\|stdio` serves the Debug Adapter Protocol over the interpreter's debug session (`crates/yavs-dap`): threads per process, line breakpoints with hit counts, continue, pause (in a process or between cycles), run for or until a time, next delta and next time step (custom requests and VS Code commands, #324), signal breakpoints on a change or a value (DAP data breakpoints and `yavs/setSignalBreakpoints`, #322), force, deposit and release of signals and their fields, elements and slices (#364) and `setVariable` on signals and locals with values parsed as VHDL literals of the target type (`yavs_std::value::literal`, #323), conditions on line and data breakpoints (VHDL expressions evaluated by the server), composite variables expanded by field and index (paged), a Hierarchy scope and `yavs/hierarchy` (#326), step over/in/out, stack frames with source positions, locals, region signals and generics, evaluate for locals and signals by name or path, output events, exit status; one thread under `--dap`, so sessions are deterministic. Scripted sessions in `crates/yavs/tests/dap.rs`. VS Code extension `editors/vscode` (debugger type `yavs`, analyzes and launches over stdio): verified with the DAP test client (`test/client.js`) and in a real headless VS Code (`test/vscode`, breakpoint hit, frame, `setVariable`, force and release commands, a conditional breakpoint, scopes and an expanded array, continue to the end); not in CI. Live waves (#325): `yavs/waveSubscribe` records the client's selection (globs, array cap, variables) with the `--wave` recorder into a second sink and streams `yavs/waves` batches of completed time steps (values base64 of the recorder's bytes with `encoding` in the hierarchy, bounded queue, never blocks the simulation); `--wave` and `--vhpi` work under `--dap` (the FST equals a plain run's). Attach (#423): `--dap=PORT` prints `yavs: waiting for a debugger on port PORT` and holds the run until a client attaches (also from a response file); the extension has an "Attach to yavs" snippet and `docs/usage.rst` a recipe for debugging one test of an external test runner. Open: VHPI plugin stdout under `--dap=stdio` #389 |
| M9: coverage | statement and branch coverage (#131): `yavs run --cov[=FILE]` writes a per-run database keyed by instance path and statement text hash, `yavs cov merge`, `yavs cov report` (text summary, Cobertura XML); no counters without `--cov`; counts identical at any `--threads`; pinned in `crates/yavs/tests/coverage.rs`. Exclusions (#351): `-- coverage off/on` pragmas, `cov report --uncovered` rows and `--exclude FILE`. Toggle coverage (#343): 0->1 and 1->0 per `bit`/`std_logic` scalar of signals and ports from the value-change bus, `--cov-kind`, `--cov-toggle-max` (W0115), `toggle` exclusion rows. Per-test attribution (#353): `--cov-test`, tests per item kept through `cov merge`, `yavs cov rank` (greedy minimal set, unique items per test), `cov report --tests`; database version 4. HTML report (#344): `cov report --html DIR`, static pages with line counts, a missed-only page per file, missed items as exclusion rows, tests per item (E0113 for an unreadable source). |
| M6: JIT tier | in progress: `yavs run --tier=llvm` (feature `llvm`) compiles every function with LLVM 21 and passes the whole difftest corpus (348/348 at 1 and 4 threads, identical to the interpreter, rule 5); 96% of the compiled programs' instructions run as native code (memory, descriptors and layouts since #334/#335; simulation 2x to 5x faster than interpreted, compile time aside), the rest through the interpreter (side exits, `docs/decisions.md`); `--tier=auto` compiles only hot functions on a background thread with identical output (#125; opt-in: since the predecoded interpreter, #431, compiled code with its side exits is slower than interpreting on most benchmarks), functions over `--tier-max-size` stay interpreted (#126); open: native lowering of the rest (#336 to #341); CI builds and tests the feature on Linux x86_64, aarch64 and Windows; llvm section |
| M5, M10 | not started (M5: the `vhpi_*` export surface (#266), `yavs run --vhpi` (#267) and the design hierarchy object model (#269) are done; values and most callbacks are not; `yavs run` accepts `+plusargs` after the top level, readable through `vhpiArgcP` and the `vhpiArgvs` iterator (#298), `crates/yavs-vhpi`; M10: design `docs/design/gui.md` (#384, decisions answered 2026-10-19); slice G1 (#388): `gui/` workspace (EUPL-1.2) with `yavs-gui`, which starts `yavs run --dap=stdio` or attaches to a port, shows a dock layout of placeholder panels, a console of the run's output and Run/Pause/Stop, tested headless against a scripted DAP server (`gui/AGENTS.md`); `yavs run --gui` starts it (E0122 when missing); release archives ship it; slice G2 (#399): source view with breakpoint gutter (conditions, hit counts), call stack per process, locals and watch with lazy and paged expansion and edit, hover, step buttons, time and delta in the status bar, tested against a scripted DAP server and, ignored, the real `yavs`; G3 to G6 and the server slices S1 (#396), S2 (#397), S4 (#398) not started) |

### External test runner: its own VHDL suites (#112)

`VUNIT_SIMULATOR=yavs`, `-p 2`, release build of main at f2ab69be (re-measured 2026-10-20); pass / fail / skipped tests per
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

`tools/suites/uvvm.py` (without `--private-copies`), release build of main at 92dfc349 (#328, #329, #303 and the force of composite signals in), `-j 2`; one run per testbench
entity, architecture and generic set the component's own regression script lists (137 in all).
Pass = exit 0 and the library's final summary `>> Simulation SUCCESS`. 78 runs pass (76 at 666f5ee3 and b9dcddb6, 73 at 92dfc349, 45 at ba53e23d); re-measured at f2ab69be with `-j 2`, 1240 s wall (2100 s at 666f5ee3 on a more loaded host, 1560 s at b9dcddb6). Only the first root cause of each run is counted.
No run fails at elaboration on the record-resolution or composite-force cause any more; the remaining failures have four causes (#391 to #394).

| Component | Pass | Fail | Skip | First root cause (issues) |
|---|---|---|---|---|
| uvvm_util | 34 | 2 | 0 | not yavs results: a test that reads a file an earlier run wrote (1), a test that needs one reference simulator's extension (1); the three long random-distribution tests run within 5 minutes since the interpreter work of #386 (re-run of this component at `bc74c892`) |
| uvvm_assertions | 2 | 0 | 0 | none |
| uvvm_vvc_framework | 2 | 1 | 0 | resolution function that accesses a shared variable #392 |
| bitvis_irqc | 2 | 0 | 0 | none |
| bitvis_uart | 3 | 0 | 0 | none |
| bitvis_vip_scoreboard | 3 | 0 | 0 | none |
| bitvis_vip_avalon_mm | 0 | 5 | 0 | not yavs results: the testbenches' FIFO and SPI designs are third-party files absent from the pinned checkout, so the component stays unbound |
| bitvis_vip_avalon_st | 6 | 0 | 0 | none |
| bitvis_vip_axi | 2 | 0 | 0 | none |
| bitvis_vip_axilite | 1 | 1 | 0 | force through an external name of a child's output port has no effect #391 |
| bitvis_vip_axistream | 7 | 13 | 2 | 13 `axistream_vvc_tb` configurations used to be skipped (over 5 minutes) and now finish, failing the unwanted-activity checks (6 of 7 alerts, as in `uart_vvc_tb`, probably #391); 2 skipped: over 5 minutes #394 |
| bitvis_vip_clock_generator | 1 | 0 | 0 | none |
| bitvis_vip_error_injection | 2 | 0 | 0 | none |
| bitvis_vip_gmii | 2 | 0 | 0 | none |
| bitvis_vip_sbi | 2 | 0 | 0 | none |
| bitvis_vip_ethernet | 0 | 3 | 1 | #392; 1 skipped: third-party design files absent from the pinned checkout |
| bitvis_vip_gpio | 1 | 0 | 0 | none |
| bitvis_vip_i2c | 0 | 0 | 1 | skipped: third-party design files absent from the pinned checkout |
| bitvis_vip_rgmii | 2 | 0 | 0 | none |
| bitvis_vip_spec_cov | 0 | 1 | 0 | file object declared in a protected type body #393 |
| bitvis_vip_spi | 0 | 0 | 24 | skipped: third-party design files absent from the pinned checkout |
| bitvis_vip_uart | 1 | 2 | 0 | #392 (1); the unwanted-activity checks of `uart_vvc_tb` see 6 of 7 alerts, probably #391, not reduced separately (1) |
| bitvis_vip_wishbone | 0 | 2 | 0 | #391 |
| external_vip_apb | 3 | 3 | 0 | #391 |
| Total (f2ab69be) | 78 | 33 | 26 | per-component rows above are from b9dcddb6 |

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
| first (`external.py nvc`), before | 1070 | 20 | 791 | 52 | 3 | 37 | 64 | 52 | 51 |
| first, after | 1070 | 20 | 798 | 52 | 3 | 39 | 55 | 52 | 51 |
| second (`external.py ghdl`), before | 504 | 30 | 377 | 0 | 0 | 21 | 16 | 19 | 41 |
| second, after | 504 | 30 | 381 | 0 | 0 | 21 | 12 | 19 | 41 |

Skipped by the harness: 403 and 647 (other languages, VHPI, coverage, Tcl and shell scripts,
second-suite scripts with control flow or tool options, negative analysis tests). The passing
tests are listed in `tools/difftest/external-{nvc,ghdl}.pass`; the CI `corpus` job checks them
against the suites' own verdicts (exit status) without a reference simulator. Every listed test gives
identical output at `--threads 1` and `--threads 4`.

Regression guard 2026-10-19 (b9dcddb6): the pass lists check green with `--oracle --check` (813 first-suite and 382 second-suite tests pass), the differential corpus passes 349/349 at 1 thread, 4 threads and 4 shuffled threads, all analysis corpora match their expected lists and the discrepancy catalogue checks green.

Regression guard 2026-10-20 (f2ab69be, release build with the `llvm` feature): the pass lists check green with `--oracle --check` (813 first-suite and 382 second-suite tests), the differential corpus passes 354/354 at 1 thread, 4 threads and 4 shuffled threads on the interpreter and, at 1 and 4 threads, with `--tier=llvm`, all 30 analysis corpora match their expected lists, the discrepancy catalogue checks green, the external test runner's 11 suites pass (1344 tests), the second verification library's `.pro` regression passes 314/314 under both 2008 and 2019 and the verification library suite runs 78 pass, 33 fail, 26 skip (no new failure). One regression was found on the way and fixed already: 185 of its 314 tests under 2019 failed between fd35a265 and 33e5f57e (#429).

Remaining run-time causes, ranked by tests (yavs bugs have issues): reports in resolution and
conversion functions during the run print `(init)` and bypass the report path, #372 (12);
unsupported PSL beyond the subset, W0004 (17, plus 4 that differ only in the directive path);
missing run-time checks, #379 (about 11); PSL-free reference quirks and LRM-unclear cases (about
15: mirror identity in reflection, null array bounds, `'transaction` start value 2 to 2, textio
read of bits with spaces, real image format of the second suite, environment and time stamps);
ICE on arrays with element constraints, #380 (3); `'path_name` of block and generate labels,
#376 (3); binary files of records and arrays of composites (3); guarded assignments with a false
guard, #373 (2); access to protected types, #377 (2); predefined operations on arrays of
composites (2); aggregate longer than a non-static constant subtype is an ICE, #378 (2); failed
textio READ stops the run, #381 (1); a repeated force in one cycle, #382 (1); sensitivity of
aggregate choices, #374 (1); files of subprograms not closed on return, #375 (1); and single
tests (alias and partially constrained record bounds, `hread` digit count, an impure function in
a type evaluated twice, `log2` precision, 2019 mode view parameters). Analysis and elaboration
failures (about 160) are mostly "not lowered yet" (`E2001`) features and VHDL-93 code read as
2008; the analysis corpora cover them.

## performance (`tools/bench`, #387)

The shared benchmark harness (`tools/bench/AGENTS.md`) measures every performance change the same way:
`python3 -I tools/bench/run.py --compare A B`. Phase times come from `yavs run --time-report`.

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

Where the rest is: a project testbench peaks at the end of elaboration with the analysis results (about 29 MB,
restored from the compilation cache: 128 B per declaration, 56 B per expression, 152 B per statement), the
lowered IR (about 11 MB) and the package state; the run itself adds little. A design of 20000 small
instances needed about 17 KB per instance (kernel signals about 1.2 KB each with 25 allocations, the
elaborated model about 9 KB per instance; see "Memory of designs with many instances (#421)" for what is left); the shared heap of a long verification testbench is genuinely
live data (about 20 MB). A global allocator that keeps freed memory mapped (one was tried) doubled the peak.

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

### Native ieee operators (#395, #404)

`std_logic_1164` gates, `not`, conversions and `rising_edge`, and `numeric_std` `+ - *`, relational
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

### Compilation cache (#407)

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
36 packages, 7 to 10 ms; verifying that program, 3 ms. Caching the lowered program and the package
state after initialisation across runs (compilation cache stage 2) would remove all three; the
IR and the heap are not serialisable yet.

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
operators of #395 that landed in the same period); it now runs to the end and its alert summary fails (#419,
identical before and after). Static sensitivity (R2c) was prototyped in the kernel (waiters that stay
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
stdout for reports (the cost is the write system call, 6% in the sampled `osvvm_axistream` run with a stopped
process, not visible in CPU time of the real run; output ordering against C code in the process and a
lost tail on a kill are the price).

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
- Negative suite `tests/diag/sem` run by `tests/diag_sem.rs` (exact code, line and column; optional
  rendered `.stderr` snapshots); what still fails is in `tests/diag_sem_xfail.txt` (the
  aggregates and case choices group passes, #45; external names and literals, #49; aliases, #46).
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
  assignment (`E1905`); a record mixing protected and other elements is `E1905` (#371). A variable port of such a composite is `E9999` (#370). Sem suite
  `prot/composite_of_protected_*`, difftest `protected/composites_2019_hand`.
- Use clauses and visibility (#54): expanded names into labelled statements and subprograms
  (8.3, `E1103` outside them), and with a protected type prefix inside its body (`pt.n`, `pt.get`,
  #250), body declarations named from outside the package (`E1105`), use
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
  `E9999`; a call of such a failed alias reports nothing more, #245). View ports, nested and array views, and calls through method aliases simulate
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
  range, division/mod by zero and negative exponents, static length mismatch (`E1405`).

## syntax (`crates/yavs-syntax`, M0)

Hand-written lexer, conditional analysis on the token stream and recursive-descent parser for
VHDL-2008 and VHDL-2019 into an owned AST (`yavs parse [--std 2008|2019] files...`). Diagnostics
go through `yavs-diag` (human, short and JSON formats; snapshots in `tests/diag/`). A simple PSL
subset runs as implicit checker processes, also in `-- psl` comments with `--psl` (#226); other PSL is skipped
with one `W0004` warning per design unit (#138).

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
types, subprograms and packages on components and blocks go through `generic.rs`. Open:
subtype indications as generate specifications.

## lower (`crates/yavs-lower`, M2)

HIR to IR for every architecture the elaborator instantiates: generics, ports (unconstrained
array ports take the actual's bounds), blocks (guarded ones with their guard process), generates
(one instance data per region), processes and concurrent statements; scalar and array
expressions with runtime descriptors (bit-planes for logic arrays), aggregates, conversions,
attributes including 'path_name/'instance_name, 'driving/'driving_value and signal attributes
of elements and slices; all sequential statements including case over arrays and matching
case, aggregate targets, every wait form; subprograms with signal parameters of every mode
(arrays too, `DriveSlot` for out/inout), signals assigned in subprograms declared in a process (driven through that process's slots, #206), `signal out` actuals passed the driver slot of their own longest static prefix (#231), copy-out of logic elements and slices, individually associated parameters (`pr(x(0) => s)`, `swap(v(4) => a, v(5) => b)`, record elements, slices; unconstrained formals take their bounds from the parts; a signal formal of several parts, also in different signals and field by field for records, is a reference of several parts, #71); parameters associated through conversions (copy-in through the actual part, copy-out through the formal part, #229); nested names of signals of arrays of arrays
(`m(i)(j)`, `m(i)(7 downto 4)`, `g(i, j)`, below a slice `m(0 to 1)(0)`, through subtype
aliases) as targets, actuals and sensitivity (#89, #230); 2008 array-valued pieces of
positional aggregates as values and targets, of named aggregate targets with range or element choices (#244, #264; a lone non-static element choice too, every target length-checked against its value), and of named aggregates and arrays with open
element subtypes as values (#89, #230; without a constrained context they take the direction of their range choices, a mix is `E1334`; an array value under `others` is `E1323`, #242); `&` on arrays of arrays and of records (#234); package
objects at run time (`Lowerer::package`: constants evaluated once, composites with run-time
bounds too, shared protected variables, package signals and files); composite architecture
constants evaluated once per region (#73); aliases of record elements of ports (#292) and of static logic elements and slices of variables (#296); `'range` of a package signal in subtypes (#294); bounds of record elements of ports in constants, signal subtypes and generate ranges (#304, difftest `composite/record_port_bounds`, `composite/record_in_port_bounds`); `'subtype` of objects bounded by a port, and aliases of record parts of variables keeping their element bounds (#315, difftest `composite/alias_object_subtype`); aggregates with a port's range as choice in initial values (#311, difftest `composite/agg_port_range_choice`); a lowering failure without a diagnostic is an ICE; non-protected shared variables (`--relaxed`) in
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

Tier-up (`--tier=auto`, #125; `docs/decisions.md`
"LLVM tier: side exits"): the interpreter keeps a tier-up count per function while a compiled tier
is set: the predecoded loop adds the ops it runs between jumps, and every op compiled code
would leave to the interpreter (call, return, native, signal read, wait, generic step) takes 12
off, kept in a local and written to the shared counter only when it may cross the threshold or
the run ends. At 1M plus 20k per IR instruction of the function (about its O0 compile time in interpreted
instructions; the hidden `--tier-heat=BASE,PER_INSTR,COOL` sets it) the function is queued for one
background compile thread (O0, one module per function, the JIT made with the first hot function)
and swapped in at the next jump, call or return; the run never waits for compilation. Functions
over `--tier-max-size` (default 50000 IR instructions, user, 2026-10-19, #126) stay interpreted
in both compiling tiers. Compiled code enters at the IR pc of the op where the interpreter is and
returns the IR pc it goes on at, mapped back to an op (`Code::entry`; inside a fused op the
interpreter steps IR instructions up to the next op). Output is byte-identical to the
interpreter: difftest `--candidate yavs-auto --threads 4` 361/361 (1 and 4 threads), and with
every function compiled from its first call or jump (`--tier-heat=1,0,0`) 361/361 at 1 and 4
threads, at 4 threads shuffled (seeds 7 and
3, the latter at `--tier-heat=300,2,12`); `native_matches_bodies_2008` (debug build) takes 45 s with
`YAVS_IEEE_TIER=auto` against 43 s interpreted, and 169 s with `llvm` (it did not finish in 25
minutes before the size cap).

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

## lib (`crates/yavs-lib`)

Done: library directories (`yavs-lib.toml` manifest + `_YAVS_LIB` lock, rule 14), `NAME:PATH`
mapping, atomic manifest updates under an exclusive lock, shared-lock reads that never write,
on-demand closure query (unknown library/unit with suggestion, cycles, stale hashes), multi-process
concurrency test. The standard is stored per file (manifest format 2). The analyzer supplies `UnitRecord`s (kind, name, dependencies) per file. A directory with a manifest from before the rename to yavs fails with L0004 (re-analyze).

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
procedure calls (`Process::concurrent_call`) before the other several-wait ones (#281). Observers
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
tests (conditional line breakpoint with hit count, step over, step in with frame and locals,
global step across processes, signal breakpoint and process table), verifier and layout tests.
Also 5 foreign marshalling tests, 5 textio/file/env tests and a protected-type test.
Not done: `Elab*` instructions (yavs-elab), records with runtime layouts beyond `LayoutOf`,
binary file read/write of records and arrays of composites, the PSL natives, `SetVhdlAssertFormat` with a non-empty format.

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

In-port collapsing (R4, #413; `collapse.rs`, on by default, `--no-collapse-ports` turns it off): done for `in` ports and component pass-through ports whose actual is whole leaves or part of one leaf (element, slice). Verified identical with and without it: difftest corpus (353) at 1 and 4 threads and 4 threads shuffled on both tiers, UVVM suite (76 pass / 33 fail / 28 skip either way), OSVVM `.pro` 2008 and 2019 (314/314), external runner pass lists. Measured (release, median CPU of 5, collapsed vs not): elab_heavy total 0.66 s -> 0.56 s (0.85), peak RSS 254 -> 211 MiB (0.83); ff_rtl, resolved_bus, vai_pe_array, vai_window_gen, uvvm_axistream_vvc_tb within noise (0.97 to 1.03), peak RSS unchanged. Still apart: forced or released ports, ports with conversions or drivers, guarded, user-resolved or whole-resolved composites, every port under `--dap`/`--vhpi`; widening these was not measured as worthwhile (the designs above have none of them in a hot path).

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

Done (#266): the `yavs` executable exports the full `vhpi_*` C ABI of `vhpi_user.h` (42 symbols,
functions and the `vhpiFS`..`vhpiS` time constants) on Linux x86_64 and aarch64 (also in the
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
from `Observer::value_change`, once per cycle (one registered during a cycle's dispatch from the next cycle, #346), with the value filled in the requested format;
with none enabled the observer does not take the state lock. `crates/yavs/tests/vhpi_run.rs`
`vhpi_drives_a_clocked_design` (a counter clocked and reset only from VHPI, force, release and
deposit on a driven signal, value changes on a signal, a port, a bit and a record; identical at
`--threads 1` and `4`, `tests/put/expected.txt`) and `vhpi_put_value_misuse_is_reported`
(`tests/put/errors.txt`). Missing: `vhpi_schedule_transaction`.

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
Missing: equivalent processes of concurrent statements, aliases, files, `vhpiGenerateIndexP`;
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
Missing: variables' values, `vhpiPtrVal`, `vhpiRawDataVal`, `vhpiSmallPhys*`, records as a
whole (`W0904`); the element subtype of a resolved array subtype (`std_logic_vector`'s
elements report `STD_ULOGIC`).

Done (#273): `tools/vhpi-conformance/run.py` runs 5 C plugins (walk, props, values, cbs, put) on 2 small designs under yavs and the primary reference simulator and diffs the canonical traces, no install needed. Baseline at `ba53e23d` (reference lines matched): walk 28/172, props 158/328, values 159/230, cbs 78/114, put 108/149; put_value (#271) was still open.
Three-way run (#273, second reference simulator with VHPI, `SIM4_BIN`), on main with #269 to #272 (yavs lines matching: both references / only the first / only the second / neither): walk 9/64/19/61, props 152/24/92/60, values 102/57/56/15, cbs 64/15/5/3, put 60/41/22/26 (case-folded, LCS-aligned; many `neither` lines are yavs reporting a property both references give as -1). Real gaps where yavs differs from both: `vhpi_iterator` for block and instance statements (#316), end-of-processes callbacks (#317), `vhpiEnumVal` on integers (#318), integer put into a vector and the delta cycle after a propagating put (#319); release after deposit is #310. Reference quirks (the references differ from each other): upper-case names in one, a dummy root above the design in the other, `vhpi_put_value` on undriven signals, time resolution, string formats of enumerations. After #310, #316 to #319 (and the non-propagating put modes refused): walk 20/93/32/92 (yavs now lists block, instance and statement iterators the references answer differently, more lines), props 152/24/92/60, values 104/57/56/13, cbs 81/18/16/14 (the remaining `neither` lines are start-of-processes callbacks the second reference orders around after-delay callbacks and omits in a time step's last delta), put 66/59/9/15, nonprop (new) 56/50/6/5.

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
