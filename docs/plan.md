# Plan: yavs, a production VHDL simulator in Rust

## Context
yavs is a new VHDL simulator. The aim is that it can replace the other major simulators for users of external test runners, verification libraries and Python testbench frameworks, with a commercial-style debugging GUI. The first proof point is a real test-runner testbench running end to end. The name is yavs, "Yet Another VHDL Simulator" (user decision 2026-10-18; formerly the working name rsim). Every crate is `yavs-*` with `publish = false`.

Decisions made with the user:
- Written in Rust, with **its own parser and its own semantic analysis**.
- **VHDL-2008 and 2019 together**, designed in from the start.
- **An LLVM backend plus an IR interpreter** (the architecture of the major open source simulators).
- **Process evaluation runs in parallel**, and the kernel is designed for it from the start. Parsing, analysis and compilation run on a single thread.
- **Diagnostics with codes, source snippets, labels, notes and help**: coloured, with VHDL stacktraces.
- **A compilation cache** (user, 2026-10-19, replaces the earlier "no incremental compilation cache" decision; `docs/decisions.md` "Compilation cache"): analysis results are kept in the work library and read back by later `elab`/`run` of the same design, results byte-identical to a run without it.
- **Interfaces:** VHPI for Python testbench frameworks (**mandatory**), code coverage, and a DAP debug server with HDL extensions.
- **GUI:** egui with an embedded **waveform viewer library**, as a separate process. It is built last, but everything before it is designed for it.
- **Platforms (all tier 1, in CI from M0):** Linux x86_64, Linux aarch64 and Windows x86_64.
  - Windows includes LLVM and Python testbench framework support.
  - VHPI uses no separate DLL. `yavs.exe` exports the `vhpi_*` symbols. **No import library is built or shipped** (user, 2026-10-18): a plugin brings its own, as Python testbench frameworks do with their `.def` files.
  - LLVM on Windows needs a pinned prebuilt LLVM 21 package with static libs; the prebuilt packages lack `llvm-config`.
- **Dependencies** (user, 2026-10-18): any crate that cargo fetches and builds by itself is fine. For **any** dependency that has to be installed by hand, **ask the user first**. Approved so far: LLVM 21 (optional, behind the `llvm` feature) and a C compiler at build time (the VHPI variadic shim). The reference simulators are dev-only references.
- **Performance:** correctness first, no speed target yet. Benchmarks are tracked, not gated.
- **Embedded Python is deferred.** A Tcl-like `yavs` scripting module comes after the DAP server.
- **Foreign subprograms:** yavs needs `foreign` subprograms in the VHPIDIRECT style plus `--load <lib>` (M3), so test-runner packages that call C (and Python through C) from VHDL work.
- **Vendor libraries:**
  - UNISIM/UNIMACRO, Altera `altera_mf`/`lpm`, and Lattice/Microchip are supported, **functional only**: VITAL compiles and runs with zero delay, and there is no SDF.
  - Synopsys `std_logic_arith`/`unsigned`/`signed`/`misc` are **built in as the original files, unmodified** (user, 2026-10-19, #117; supersedes the clean-room plan of 2026-10-18). Black-box testbenches: `tests/corpus/synopsys/`.
- **Test-runner integration** (user, 2026-10-18):
  - **Mixing VHDL-2008 and 2019 files in one project is allowed.** The manifest records the standard per file. Each file is parsed with its own standard's syntax. A design is analyzed and elaborated against **one** `std`/`ieee` revision, the newest one in its closure, so types such as `INTEGER` are the same type everywhere. A construct that is illegal under that revision is a fail-early error naming the file and its standard. A construct whose **meaning changes** (e.g. `integer'high` going 64-bit) is a **warning**, given once per construct and escalatable with `-Werror`, so external test runners and verification libraries keep working from 2019 testbenches.
  - **Each yavs defaults to `--threads 1`** under a test runner's parallel option (`-p N`); users opt in through `yavs.sim_flags`.
  - **`@file` response files are supported from the start.** test runners use them on Windows for long `runner_cfg` generics.
  - Installation is documented as test-runner library recipes, with no installer command.
- **Repo:** `ru551n/yavs` on GitHub is **public** (made public 2026-10-18 after the rename and a history audit). **Initial phase:** agents push straight to `main`, with no PRs and no CI. Once something works, CI is added on Linux x86_64, Linux aarch64 and Windows, and each milestone then lands as a PR that the user merges.
- **External test inputs (never copied into the repo):** conformance suites and the reference simulators' own test suites, fetched in CI or read from local checkouts, for parse, sem and differential runs.
- **Licensing:** MIT/Apache, published once the repo goes public. The GUI crate alone is EUPL-1.2, because the embedded viewer library is.
- **Workflow:** one subagent per milestone.

What the research found locally:
- **No Rust simulator exists to build on.**
- **The existing Rust VHDL front end** is MPL. Its 2019 grammar is close to empty, and a global interner lock serializes parsing. It is a test-corpus and behaviour reference only, not a dependency.
- **LLVM 21.1.8** with dev headers and static libs is in `/usr/lib/llvm-21`. Use `llvm-sys` 211.x, falling back to `prefer-dynamic`.
- **The `wellen` crate** (BSD-3) provides public `HierarchyBuilder` and `wavemem::Encoder`. It works as yavs's in-memory waveform model and as the viewer's data source.
- **The primary reference simulator** (GPL) is an architecture reference only; never copy its code.
  - Its VHPI model, JIT pipeline (IR generation, interpreter, LLVM backend) and coverage are the structural references.
  - It passes the regression of the Python testbench frameworks, so its exported VHPI symbol set is the checklist to cover.
- **Library sources:**
  - The `std` and `ieee2008` sources of the existing Rust VHDL front end. The IEEE parts are Apache-2.0. `std` has declarations only, so yavs writes the bodies for standard, textio and env natively.
  - The VHDL sources of an external test runner and verification library, read from local checkouts.
- **Not installed:** a Python testbench framework and a waveform viewer checkout. Both are needed in later milestones.

## Architecture (Cargo workspace at `~/git/rsim`)
| Crate | Role |
|---|---|
| `yavs-diag` | One diagnostics model shared by every stage, rendered with `annotate-snippets` (codes, snippets, labels, notes, help). A diagnostic carries a severity and colour, a stable code (`yavs --explain E0xxx`), a primary span plus labelled secondary spans, and `note:`/`help:` lines with suggested fixes. Errors recover and are reported together, with cascading errors suppressed. **Runtime errors** add a VHDL stacktrace (one frame per subprogram, with its source line), the process, the instance path, and the time and delta, so an assert raised inside an IEEE library still points at the caller in user code. Colour turns off when the output isn't a TTY, with `--color` and `NO_COLOR` to override. Also `--message-format=json\|short` for test runners, editors and DAP. Internal compiler errors are caught and rendered at the span being processed. |
| `yavs-syntax` | Lexer and recursive-descent parser into an owned AST with spans. Handles 2008 and 2019, including mode views, private regions, the 2019 generic forms and the sequential block. Conditional analysis (`` `if ``) is evaluated in the token stream. Parsing and analysis run on a single thread. |
| `yavs-sem` | Libraries and design units, scopes and visibility (`use`, `context`, selected names), the type system, overload resolution, static expression evaluation, and diagnostics. Its output is a **typed HIR** in which every expression carries its type and every call names the overload it resolved to. |
| `yavs-lib` | Library manifests. `yavs analyze --work L f.vhd` checks the file and records it in `L/yavs-lib.toml` (the index, in compile order) and `L/yavs-lib.d/` (a record per file), under the library lock (rule 14), so parallel analysis is safe. `elab` and `run` analyze the closure of the top level from source, or read it back from the compilation cache of the work library (`yavs-cache`, `docs/decisions.md` "Compilation cache"). **Analysis is on demand:** the manifest maps unit names to files, and `elab` analyzes only the units the top level can reach. This keeps big vendor libraries such as UNISIM from being re-analyzed in full on every run. |
| `yavs-cache` | The on-disk store of the compilation cache: keys, atomic writes, validation of what is read, pruning, cleaning. It knows nothing about VHDL; `yavs-sem` makes and restores the analysis images (`image.rs`) and the CLI decides when. |
| `yavs-ir` | Lowers the HIR to a register-based IR. Each instruction carries a statement-location tag, used for stepping and for coverage. |
| `yavs-elab` | Elaboration. Folds generics through the interpreter, expands generates, binds components and configurations, and resolves external names. It builds the signal net graph (drivers, resolution, port conversions). |
| `yavs-interp` | IR interpreter. Used for elaboration-time evaluation, cold code, and debug stepping. |
| `yavs-llvm` | IR → LLVM, JIT through the ORC C API. The machine code of every compiled module is kept in the compilation cache of the work library and linked again by later runs (#505, `docs/decisions.md` "Compiled code in the compilation cache"). Tiered: a process starts in the interpreter and moves to LLVM once it is hot. Compilation runs on a single thread, in the background, so the simulation is never blocked waiting for it. |
| `yavs-rt` | The kernel. Implements the LRM 14.7 cycle with **parallel process evaluation** (described below). Holds the value heap (access types, protected types) and the native `std.standard`, `textio` and `env`. Logic values, **signals included**, use **bit-planes** everywhere (user, 2026-10-18; yavs-logic benchmarked planes as the winner). Protected-type callers run serial through M3. Foreign subprograms are supported (VHPIDIRECT style, plus `--load`). The calls go through **our own per-ABI call trampolines** (SysV x86_64, AArch64, Win64), with no libffi, so library dependencies stay minimal. |
| `yavs-wave` | Records values into a `wellen` `Hierarchy` and signals, and writes FST. |
| `yavs-cov` | Statement, branch and toggle counters, a database that can be merged, and HTML and Cobertura reports. |
| `yavs-vhpi` | IEEE 1076 VHPI C ABI (`vhpi_user.h`) and `vhpi_startup_routines` loading for Python testbench frameworks. |
| `yavs-dap` | Debug Adapter Protocol server: breakpoints, step in/over per process, stack, and variables. Extensions add hierarchy browsing, signal peek/force/deposit, run-to-time, and live waveform streaming. |
| `yavs` | The CLI: `yavs analyze`, `yavs elab`, `yavs run [--wave] [--cov] [--vhpi lib] [--dap port]`. The exit code follows `env.stop`/`finish` and severity, as test runners expect. |
| `yavs-gui` (EUPL-1.2, **built last**) | egui/eframe app embedding the waveform viewer library. It connects to `yavs --dap`. It provides the hierarchy and source panes, breakpoints, a console and live waves. **Debugging matches the other major simulators** (user, 2026-10-18): line breakpoints with conditions and hit counts; signal breakpoints (on change or on a value); step into/over/out within a process plus a global step to the next statement of any process; run for a time, to the next delta or to the next time step; call stack and locals per process; a list of active and waiting processes with what they wait on; time and delta shown at every stop. **Look: a modern commercial simulator style** (dockable panels, hierarchy tree, source view with breakpoint gutter, embedded waves), not a classic Tk look. |

**Designed for the GUI from the start** (none of it is built until M10):
- **The kernel is driven through a `SimControl` API**: run-for, run-until, step-delta, pause, plus breakpoint and watch hooks. The CLI, DAP and VHPI all call this one API, and nothing else drives simulation.
- **Every IR instruction carries a source span and a statement id**, and every elaborated object has a stable hierarchical path. The same ids feed DAP stack frames, VHPI handles, coverage and the hierarchy view.
- **Value change events go to a subscriber bus.** Waves (`wellen`), VHPI value-change callbacks and DAP live streaming are all subscribers, so a live GUI is just one more subscriber.
- **The simulation can be paused at defined points**: at the end of a delta, at a timestep, or at a statement while running in the interpreter. Signals can be peeked, forced and deposited while paused.

**Parallel kernel design**
- In each delta, the processes that are ready are evaluated on a thread pool (rayon). A process only reads signal values from the current cycle and writes its own driver queue, so there are no data races.
- **Side effects are buffered per process** (report, assert, file writes, `env.stop`), then committed in a deterministic order, sorted by the process's elaboration order. This keeps output byte-identical to a single-threaded run.
- **Shared state:** each protected type instance gets a lock. Processes that touch non-protected `shared variable`s (VHDL-93 style) are marked serial during elaboration and run alone.
- `--threads 1` must give identical results. That is the determinism check. `--shuffle` randomizes the serial order to expose testbenches that depend on process order (shared variables in verification libraries).
- **There is no prior art to copy.** One major simulator tried work-queue parallelism in 2022 and went back to a serial run queue in 2023, and another is serial. So `--threads` defaults to 1 until the differential suites pass at N threads, and the GC/heap safepoints are designed together with the thread pool (a deadlock between GC and threads is a known bug class in other simulators).

## Error-handling motto: fail early, fail clearly
Every error is reported through `yavs-diag` with a code: code, spans, labels, notes and help, and for runtime errors the VHDL stacktrace with instance path and time. Concretely:
- **Never continue past a failed stage.** If analysis has errors, there is no elaboration. If elaboration has errors, there is no simulation. Inside one stage, independent errors may be listed together as compilers usually do, but no guessed value is ever carried into the next stage.
- **No silent fallbacks.** Don't use `unwrap_or(default)` on anything that comes from the design, the user or the environment. A missing library, unit, file, generic, plugin or VHPI symbol is an error that names what is missing and where yavs looked.
- **Runtime errors stop the simulation at once** (bound check, overflow, null access, delta limit, assertion at or above `--exit-severity`), with a clear exit code. Nothing is clamped or wrapped.
- **Internal invariants are checked.** A broken invariant becomes an internal compiler error rendered at the span being processed, with "this is a bug in yavs" and a report hint. It is never undefined behaviour or a bare panic message.
- **External tools and plugins:** non-zero exit codes and error statuses are propagated and never swallowed.
- **LRM interpretation policy** (user, 2026-10-18): where the LRM is clear, follow it and fail early. Where it is unclear or silent, be permissive: accept the union of what the other major simulators accept, and pick the semantics that let the most designs elaborate. Warn only where those simulators give different results. The rulings per item are in `docs/design/lrm-checks.md`.

## Design rules from bug classes in other simulators
Other open source simulators kept running into these bug classes.
1. **Constraints are runtime data on every composite.** Partially constrained types, constraints known only at runtime, and records with unconstrained fields are the largest crash class in other simulators. There is one type/subtype representation, and no "maybe static" path.
2. **Signals are addressed by sub-element.** One mapping from longest static prefix to drivers and sources is shared by drivers, views, force/deposit, VHPI, waves and coverage. Storage and event tracking work on a whole signal or a contiguous vector, not on each scalar.
3. **Port collapsing is an optional optimisation** behind a flag, with a differential test against the uncollapsed design. Port conversions and associations become explicit implicit processes in the IR.
4. **Elaboration is a pure, re-entrant function** of (design, generics, configuration), with no global state, so restarting in-process works. External names are bound at elaboration. Generic overrides of any type are parsed with the normal value parser. Each generic instance gets its own mangled identity.
5. **The interpreter is the semantic reference.** LLVM must pass every suite when compared against it. The IR uses 32-bit ids and register reuse, with linear-time passes. A unit too big for LLVM stays interpreted. LLVM 21 is pinned, linking goes through ORC (no hand-written ELF loader), and code targets a portable CPU baseline.
6. **The heap uses precise roots**, with handles into a growable arena and no conservative scanning. Large locals and temporaries go on the heap, not the stack. Sizes are 64-bit everywhere.
7. **Time and integer semantics are the same in every tier.** Time resolution is a runtime setting, never baked into code. All arithmetic is checked i64, with 64-bit integers in 2019 mode. Storage is not semantics: an integer type whose range fits in 32 bits is stored in 4 bytes as an array element and in a signal (sign-extended on load, stored only after its range check), a scalar variable or record field in 8; no value changes (docs/decisions.md "Narrow integer storage"). A delta-cycle limit has a clear diagnostic.
8. **Identifiers keep their original spelling**, with lookup by a case-folded key. VHPI `CaseName`, waves and diagnostics need it, and another simulator had to retrofit this painfully.
9. **VHPI handles are generational ids into an arena.** They are cheap to create and release, and compared with `vhpi_compare_handles`. Callbacks that are disabled but never removed must stay cheap, because Python testbench frameworks never remove them. Records, arrays, generics and generates are native in the object model, not a Verilog-shaped shim.
10. **Coverage items are identified by hierarchical path plus a hash of the source span**, never by offset. Databases are named per run and can be merged. Instrumentation is ordinary IR, so both tiers have it.
11. **Waves come from the same sub-element change events.** There are include/exclude globs, a size cap for arrays and memories, and enum names kept in metadata. The FST is flushed on SIGINT and at the end of simulation.
12. **Constant aggregates are compact** (run-length or `others`), so a 64K-entry memory initialiser doesn't blow up elaboration.
13. **Diagnostics have a stable code** that can be suppressed or escalated per code (`-W`, `-Werror`). The few relaxations of LRM rules sit behind `--relaxed`.
14. **Process-safe on disk, with automatic library locking like other simulators.** Every library directory has a `_YAVS_LIB` lock file locked with an OS advisory lock (Rust `std::fs::File::lock`/`lock_shared`, which is `flock` on Unix and `LockFileEx` on Windows), so parallel `yavs analyze` runs into the same library are safe.
   - `analyze` takes the **exclusive** lock while it updates the manifest. Each write goes to a temp file followed by an atomic rename.
   - `elab` and `run` take the **shared** lock only while they read the manifest and its source list, then release it. A long simulation never blocks compilation.
   - `run` never writes into library directories.
   - **Read-only libraries never fail** (sandboxed build systems). If the lock file can't be opened for writing, yavs opens it read-only with a shared lock. If it can't be opened at all on a read-only directory, yavs skips locking, since nothing can change that library anyway.
   - A lock wait longer than a few seconds prints a note naming the library.
15. **Front-end fuzzing from M0.** cargo-fuzz runs on the parser and on semantic analysis; a week of fuzzing found 32 crashes in another simulator's front end.

## Milestones (one subagent each, run one at a time)
- **M0: workspace, lexer and parser.** Parse all of an external test runner's VHDL, an external verification library, the std/ieee sources of the existing Rust VHDL front end, and the primary reference simulator's `test/regress` and `test/parse` files with zero errors (those files are only read as test inputs). Also a set of 2019 constructs, with `` `if `` handling.
- **M1: semantic analysis.** Analyze std, ieee2008, the external test runner library and the verification library with zero diagnostics, in both 2008 and 2019 mode. Own negative tests cover the illegal cases (bad overloads, visibility, types), with snapshots of the rendered diagnostics. The type and constraint model (rule 1) is reviewed before M2 starts.
- **M2: IR, interpreter, elaboration and the parallel kernel on a scalar subset.** Check: a differential corpus (`tests/corpus/*.vhd`) gives the same report lines, times and exit status as the primary reference simulator, with both `--threads 1` and `--threads N`.
- **M3: full types and libraries.**
  - Arrays, records, access types, protected types, resolution.
  - `numeric_std` and the fixed and float packages from source.
  - textio and file I/O, `env`, package generics, external names.
  - Foreign subprograms with `--load`.
  - The packed-logic layout benchmark and decision.
- **M4: external test runner end to end.** A minimal testbench, then the runner's own VHDL test suites. Make the runner able to use yavs as its simulator.
- **M4b: ecosystem libraries.**
  - The Synopsys packages (built in, #117).
  - Functional VITAL.
  - UNISIM/UNIMACRO, altera_mf/lpm and Lattice/Microchip primitives analyze and simulate, each with a documented test-runner recipe.
  - Python called from VHDL through VHPIDIRECT foreign subprograms and `--load` works under a test runner.
- **M5: VHPI (mandatory).** The full VHPI object model over `SimControl` and the event bus. Install a Python testbench framework into a venv. Check: its own regression passes on yavs.
- **M6: JIT tier.** Lower to LLVM 21 with tiering. Check: the M2 to M5 suites pass under LLVM only, and benchmarks against the primary reference simulator are tracked in `docs/STATUS.md`.
- **M7: waves.** FST output through `wellen`, as an event-bus subscriber.
- **M8: DAP server.** Interpreter stepping, signal peek/force, and the wave-stream extension. Check: scripted DAP sessions in tests, plus manual use from a DAP-capable editor.
- **M9: coverage.** Statement, branch and toggle coverage, merge, HTML and Cobertura output.
- **M9b: embedded Python scripting (deferred).**
  - A pyo3 `yavs` module over `SimControl`: run, step, force, examine, breakpoints, waves.
  - libpython is loaded dynamically, and an interpreter that a Python testbench framework or a test-runner package has already started is reused (one CPython per process).
- **M10: GUI (later).** yavs-gui with the embedded waveform viewer library, connected over DAP. Nothing in it should need kernel changes.

## Working rules for the milestone subagents
- One background agent at a time (an earlier memory records OOM crashes when agents fanned out). Each one works on a branch, and I review and merge.
- No more than 8 HDL simulator processes in parallel host-wide, reference simulator runs included.
- Each simulation gets a timeout and a memory cap (`systemd-run --user --scope -p MemoryMax=…`).
- No testbench may run longer than 5 minutes.
- Never copy the source of the reference simulators or of the existing Rust VHDL front end. Reading it for reference is allowed.
- Every check's exit status must reach the caller: never pipe a check into `grep` or `tail`.
- No em dashes in docs or commits.

## Verification
- **Diagnostics snapshot tests** (`tests/diag/*.vhd` with expected rendered output, the way compiler UI tests work), for both analysis and runtime errors. A change to error text then shows up in review.
- `cargo test` and `cargo clippy --workspace -D warnings` in every milestone.
- **Parse and analysis corpus counts** (M0, M1): zero errors on the library corpora, run as a cargo test over the local corpora.
  - The conformance suite and the reference simulators' test suites have pass/fail lists checked into the repo, including the tests expected to fail. Any regression fails CI.
- **Differential tests against the primary reference simulator** (from M2): report text, report time, exit status, and thread-count invariance.
- **External suites, gated in CI once they first pass:**
  - An external test runner's own VHDL test suites (M4) and a Python testbench framework's regression (M5).
  - External verification libraries' own suites (from M4), which stress protected types, access types and generic packages together.
  - Pass counts are tracked in `docs/STATUS.md`.
- **Both tiers on every suite** (from M6). JIT-only and interpreter-only runs must give identical output.

## Out of scope (add when needed)
Verilog and mixed-language designs, PSL (skipped with a warning, user 2026-10-18), SystemC, VHDL-93-only semantic quirks, distributed simulation, a full LSP (existing language servers already cover that), SDF and VITAL timing, macOS (avoid gratuitously Unix-only code), and a speed target.
