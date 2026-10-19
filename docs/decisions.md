# Decisions

## Simulator discrepancy catalogue (user, 2026-10-19)

- **Choice:** every known disagreement between the reference simulators, with the LRM or with
  yavs, is catalogued in `docs/design/simulator-discrepancies.md`, one reproducer per distinct
  behaviour under `tests/discrepancies/`, run by `tools/discrepancies/run.py` on yavs and on any
  configured simulator (labels `sim1` to `sim4`; the label to tool mapping stays local). A new
  disagreement found by anyone gets a row and a case in the same commit as the decision.
- **Majority rule (user, 2026-10-19):** yavs is never counted in a majority; a majority means 2
  of the 3 reference simulators.
- **Why:** the fourth simulator can then be checked with one command, and each choice shows who
  made it and when.

## Project name: yavs (user, 2026-10-18), formerly the working name rsim

- **Choice:** the project is yavs, "Yet Another VHDL Simulator". Crates are `yavs-*`, the
  binary is `yavs`, environment variables are `YAVS_*` (the external test runner: `VUNIT_YAVS_PATH`), native symbols
  start with `yavs.`, and library directories hold `yavs-lib.toml` and `_YAVS_LIB`.
- **How:** one mechanical sweep, `tools/rename-yavs.sh` (idempotent; other worktrees run it on
  their own changes). The GitHub repository is `ru551n/yavs` (the old URL redirects).
- **Kept:** the `~/.cache/rsim-*` scratch directory names and the `~/git/rsim` checkout (both exist
  on disk), the test runner integration branch names, and the vendored `lib/std` and `lib/ieee` files, which keep their old
  comments so `lib/SHA256SUMS` stays valid. A library directory that still has the old
  manifest name fails with L0004 and asks to re-analyze: there is no reader for old names.

## Release trigger and first release (user, 2026-10-18)

When yavs simulates a test runner testbench end to end (issue #110: `run.py --simulator yavs` passes a
minimal test runner testbench on the real binary), CI is added (#134) together with a release workflow
(#165), and the first release is cut. Rationale: a release is only useful once the first
milestone users care about works.

- Release workflow: triggered by a `v*` tag; builds `yavs` for Linux x86_64, Linux aarch64 and
  Windows x86_64, runs the CI
  checks on the tagged commit first, and attaches archives plus SHA256SUMS to a GitHub release.
- Packaging (agent, 2026-10-18, #165; revised by the user the same day):
  `yavs-<version>-<target>.tar.gz` (Linux) or `.zip` (Windows) holding the `yavs` executable,
  `LICENSE-MIT`, `LICENSE-APACHE`, `LICENSE-IEEE` (the Apache-2.0 licence of the built-in IEEE
  sources) and `README.md`. The tag must equal `v<workspace version>`. `workflow_dispatch` with
  `dry_run` builds the archives as workflow artifacts and publishes nothing.
- **std/ieee are built into the binary** (user, 2026-10-18: "how can we avoid packaging all the
  stdlibs?"). `yavs-sem/build.rs` embeds `lib/std` and `lib/ieee` (2.9 MB of source) under the
  virtual root `<yavs>/lib`; `$YAVS_STDLIB` replaces it with a tree on disk. This supersedes
  the `lib/`-next-to-the-executable lookup of #43. Nothing is precompiled: analysis stays on
  demand from source, so there is still no cache.
- **IEEE 1076.4 VITAL packages are built in, unmodified** (user, 2026-10-19, #118: "do as ghdl and
  nvc"; supersedes reading them in place). `ieee.vital_timing`, `vital_primitives` and `vital_memory`
  (VITAL 2000) live in `lib/ieee/{2008,2019}/vital_*` byte for byte from the IEEE text (origin,
  cross-check and licence in `lib/SOURCES.md`, `lib/LICENSE-IEEE-VITAL`, shipped in the release
  archive). They run as plain VHDL: functional, zero-delay use (no SDF back-annotation). Their pure
  functions call `NOW` (`GlitchMinTime`, ...), which LRM 4.3 forbids: units of `ieee` named
  `vital_*` are exempt from E1830, user code is not. `tools/update-ieee-libs.sh` preserves them.
- **No VHPI import library** (user, 2026-10-18). The release builds and ships no `yavs.lib`; a VHPI plugin brings its own
  import library for `yavs.exe`'s exports (Python testbench frameworks ship `.def` files per simulator). #124 is
  reduced to checking that the exports are there.
- **Release notes with towncrier** (user, 2026-10-18, "for easier parallel work"), with fragments in `release_notes/news.d/` (AGENTS.md rule 25); a release runs
  `uvx towncrier build --version X.Y.Z --yes` into `release_notes/unreleased.md` and renames it
  `release_notes/X.Y.Z.md` in the version-bump commit. The GitHub release notes are that file.
  Fragments describe features, not numbers.
- **No document names other projects** (user, 2026-10-18: "it is not related to this project, this is
  just a simulator"; "avoid mentioning other projects in the docs, except in license notes"; widened
  the same day to the developer docs: "clean the developer docs too"). Applies to every document in
  the repository: README, `docs/*.rst`, release notes, plan, decisions, design documents, CLI
  contract, `STATUS.md`, every `AGENTS.md`, and tool and test docs. Use neutral terms ("reference
  simulators", "other simulators", "external test runners", "Python testbench frameworks").
  Exceptions: licence notes, and commands, flags, environment variables, file and directory names,
  crate and dependency names, code identifiers and corpus identifiers, which stay as they are.
- **When to release** (delegated by the user, 2026-10-18: "decide when to cut new releases.
  often is not bad"): the orchestrator cuts a release whenever main is green (CI and docs) and
  `release_notes/news.d/` holds at least one `feature`, `bugfix` or `breaking` fragment,
  checked each time a batch of agent work has landed. While the version is 0.x: a `breaking` or
  `feature` fragment bumps the minor version, only fixes bump the patch; every 0.x release is a
  GitHub pre-release. Steps: set `[workspace.package] version`, `cargo build` (Cargo.lock),
  `uvx towncrier build --version X.Y.Z --yes`, rename `release_notes/unreleased.md` to
  `release_notes/X.Y.Z.md` with a one-line summary on top, run the rule 13 checks, commit
  "Release X.Y.Z", push, then an annotated tag `vX.Y.Z` on that commit. The tag triggers
  release.yml and the Read the Docs version.
- **User documentation** (user, 2026-10-18): Sphinx (sphinx_rtd_theme, MyST, sphinxcontrib-towncrier for the "Unreleased" section) in `docs/`,
  published on Read the Docs as project `yavs` (`.readthedocs.yaml`). `.github/workflows/docs.yml`
  builds with `-W` on every push and, through the Read the Docs API (secret
  `READTHEDOCS_TOKEN`), rebuilds `latest` on main and activates a version per release tag plus
  `stable`. The developer documents in `docs/` (plan, decisions, design, CLI contract) are
  excluded from the site.
- CI (agent, 2026-10-18, #134): fmt, clippy and tests with default features on ubuntu-24.04,
  ubuntu-24.04-arm and windows-latest (MSVC). No LLVM, no external corpora (#63), no qemu/Wine
  runs (the aarch64 and Windows runners are native). The release workflow calls the same
  workflow on the tagged commit.
- First release: `v0.1.0` (the workspace version), marked pre-release, notes listing what works
  and the known gaps. Once the trigger is met, the user's request counts as approval to publish it.

## Waveform writer: wavefst (yavs-wave)

- **Choice:** the `wavefst` crate (0.4.0, MIT) for FST output, with `default-features = false`,
  features `writer` and `lz4` (pure Rust, no C compiler needed). No FST writing code of our own.
- **Why:** the `wellen` crate is reader only. `fst-writer` 0.3.1 (BSD-3) has no hierarchy attributes, so it
  cannot carry VHDL variable/data types or enum tables, its real-value path is unused and its
  flush only works at a time step boundary (`todo!`). `wavefst` has enum tables, VHDL supplemental
  variable metadata (`CreateVar2`), aliases, reals, `flush()` that patches the header, and is
  checked against libfst in both directions.
- **Risks:** a young single-author crate. All use is behind `yavs-wave`'s own API (`WaveWriter`),
  so it can be swapped or vendored. yavs-wave's round-trip tests read every file back with `wellen`.
- **FST mapping:** std_logic family and bit are written as characters (`u x 0 1 z w l h -`) with
  the VHDL data type set, so viewers show `U`, `X`, ...; other enums are bit vectors with an FST
  enum table (literal names in original spelling); integers and time are two's complement vectors
  (an integer has its subtype's name, else that of the subtype it derives from, and the width its
  elaborated range needs: unsigned ranges `ceil(log2(hi + 1))` bits, others the signed width; #306);
  reals are doubles; vector bounds go into the name (`data[7:0]`, `rev[0:3]`, left first, so
  direction is kept); records and arrays are scopes with one signal per sub-element; the chosen
  alternative of a generate with a label is the scope `label.alt` (#306).
- **Variables (#305):** opt in with `--wave-variables`. Only scalar process variables; they sit
  in the process's scope. Sampled once per time step (`NextTimeStep`, `EndOfSim`) from a snapshot
  the process publishes when it suspends (`yavs_interp::ProcVars`, `InterpProcess::with_vars`,
  `Elaborated::build_probed`), so nothing is published for variables the globs drop and the
  file does not depend on `--threads`. A process that ends keeps the values of its last
  suspension. Composite variables and shared variables are not recorded yet.
- **Durability:** a block is flushed on the next time advance after `flush_interval` (default
  5 s) or `block_change_limit` changes, and the header is patched each time, so a killed run is
  readable up to the last block. SIGINT/SIGTERM only set a flag (`interrupt::install`); the kernel
  stops between time steps and calls `finish`, which is also done on drop.

## Packed logic vectors: bit-planes (yavs-logic)

- **Choice:** `LogicVec` is stored as four bit-planes (`Vec<[u64; 4]>`, 64 elements per block),
  not 4-bit nibbles. The nibble layout stays behind `--features nibbles` since it costs about
  100 lines and shares all tests and benchmarks.
- **Why:** equal speed up to 32 elements, then 8x to 10x faster logic ops and resolution, and
  40x to 140x faster numeric add/compare at 1024+ elements (a 0/1 vector is plain `u64` words).
  Memory is the same 4 bits per element. Full table in `crates/yavs-logic/LAYOUT.md`.
- **Costs:** `to_string` is 2x slower (per-element decode); a 1-element vector takes 32 bytes, so
  the kernel keeps scalar signals as bare `StdULogic`. VHPI/FST need a transpose per element.

## LLVM bindings: llvm-sys 211, no inkwell (yavs-llvm)

- **Choice:** `llvm-sys` `=211.1.1` (MIT), the raw LLVM 21.1 C API, features `prefer-dynamic` and
  `strict-versioning`, optional behind the crate feature `llvm` (default off, so the workspace
  builds without LLVM). Test with `cargo test -p yavs-llvm --features llvm`.
- **Why not inkwell:** inkwell 0.10.0 (Apache-2.0) has an `llvm21-1` feature on top of llvm-sys
  211, but its only JITs are MCJIT and the legacy ORCv1 (`LLVMOrcCreateInstance`), which no longer
  exists. ORC LLJIT (rule 5) would need raw llvm-sys anyway, and mixing inkwell's lifetimes with
  raw ORC handles costs more than it saves. The C API surface we use is small (core IR builder,
  `LLVMRunPasses`, LLJIT, absolute symbols), wrapped in `jit.rs`.
- **JIT shape:** one LLJIT per simulation, driven from one background compile thread. Every
  module has its own `LLVMContext` (`LLVMOrcCreateNewThreadSafeContextFromLLVMContext`, since LLVM
  21 removed `ThreadSafeContextGetContext`). Host runtime functions are absolute symbols in the
  main dylib, so the executable exports nothing for the JIT and nothing is looked up in the process.
  Target: host triple, CPU `x86-64-v2` (x86_64) or `generic` (aarch64), never the host CPU.
  Optimisation: `default<O2>` through `LLVMRunPasses`.
- **Runtime errors** in JIT code call a Rust `extern "C"` callback that records the error and
  returns; the generated code then returns a status. No unwinding through JIT frames (Win64 does
  not register unwind info for them, see `crates/yavs-llvm/WINDOWS.md`).
- **Linking:** shared `libLLVM-21.so` on Linux (low link memory). On Windows llvm-sys refuses
  dynamic linking and uses the static libs of the official release tarball; see WINDOWS.md.
- **Size threshold (rule 5):** codegen time grows faster than linearly with function size (about
  0.15 s at 10k IR instructions, 3.8 s at 100k, 28 s at 300k at O2; numbers in STATUS.md). The
  proposal for M6: JIT a function at O2 up to 40k LLVM IR instructions (under 1 s, under 100 MiB),
  keep larger units in the interpreter, and lower big processes as several functions rather
  than one. To be confirmed with real lowered code in M6.

## Kernel crate and process interface (yavs-kernel)

- **Crate:** the LRM 14.7 cycle lives in `crates/yavs-kernel`, independent of the IR. yavs-rt
  (heap, std natives) builds on it.
- **Process interface:** `Process::run(&mut Ctx) -> Result<(), SimError>` runs to the next
  `wait`; the wait is stated through `Ctx::wait_on/wait_for/wait_at`. A `wait until` condition is
  evaluated by the process, which re-suspends with its original absolute deadline.
- **Event tracking:** per-element flat arrays (`event_cycle`, `last_event`, `last_value`, ...)
  over the whole signal, with `Range` operations; inertial preemption is decided per element and
  transactions are split. This is how rule 2 is read: no per-scalar objects, exact per-scalar
  semantics.
- **Forces and deposits** (from processes, `SimControl` or callbacks) are queued and applied in
  the next cycle, as VHDL-2008 forces are. A deposit sets the driving value until a driver next
  updates those elements.
- **Delta numbering:** initialization is delta 0 at 0 fs; the first cycle at 0 fs is delta 1.
  Default delta limit 10000.
- **Delays** are truncated to the resolution limit (`Config::resolution`).
- **Exit status of kernel errors:** 101 for internal errors (also a panic inside process code,
  which is caught), 1 for every other error. Normal ends map from `EndReason` in the CLI.
- **End of simulation:** `Idle` (nothing scheduled) is terminal like `stop`/`finish`;
  `EndOfSim` fires once.

## VHPI ABI and symbol export (yavs-vhpi)

- **Header:** `crates/yavs-vhpi/include/vhpi_user.h` is the official IEEE 1076-2019 header
  ("Copyright 2019 IEEE P1076 WG Authors", Apache-2.0), vendored unchanged with its license
  header from rust-vhpi (`vhpi-sys/vhpi_user.h`, commit `65484e3`; the reference simulator ships the same file with
  `(void)` prototypes). Plugins build against it. We do not write our own: names, values and
  layouts are fixed by the standard, and the IEEE text is what every other simulator ships.
- **Rust side:** hand-written `#[repr(C)]` types and one constant per enumerator in `sys.rs`
  (C enums are `c_int`), no bindgen or libclang build dependency (painful on Windows). The test
  `abi_matches_c_header` compiles a C program generated from the Rust tables against the header
  and compares all ~610 constants and every struct size and field offset; a second test checks
  the header has no constant the Rust side lacks. Same result on Linux and Win64 (MinGW, wine).
- **Export, Linux:** the `yavs` executable (its own `build.rs`, #266: cargo does not forward a
  dependency's `rustc-link-arg-bins`) is linked with `-rdynamic` plus
  `-Wl,--version-script` listing `src/exports.txt` as `global` and `*` as `local`, so GNU ld
  exports exactly the `vhpi_*` list. `--dynamic-list` was dropped because cargo-zigbuild
  (the glibc 2.17 release) rejects it, as it does `--export-dynamic-symbol`. Under zig's linker
  the version script does not narrow `-rdynamic`, so the release binary also exports its Rust
  symbols (harmless, larger `.dynsym`). `yavs_vhpi::keep_exports()`, called first in the CLI
  `main`, references every symbol so the linker keeps them. A test
  (`crates/yavs/tests/vhpi_exports.rs`) reads the binary's `.dynsym` and fails if any symbol
  of `vhpi_user.h` is missing. Plugins leave `vhpi_*` unresolved; plugin loading uses
  `RTLD_NOW | RTLD_GLOBAL`, so a missing symbol fails at load, naming it.
- **Export, Windows:** no VHPI DLL. The executable is linked with a generated `.def`
  (`/DEF:` for MSVC, the `.def` as linker input for MinGW). `yavs`'s `build.rs` passes it; no `yavs.lib` is built.
  `yavs-vhpi`'s `build.rs` writes only the test import library `yavs-vhpi-host.lib` with `lib.exe` (MSVC), else
  `llvm-dlltool`, else MinGW `dlltool`; short-format import libraries work with both MSVC link
  and GNU ld. The import names the executable, so a renamed `yavs.exe` breaks plugins linked
  that way (shims that use `GetProcAddress` on the main module do not care).
- **One list:** `build.rs` turns `src/exports.txt` into the version script, the `.def`, the host test
  import library and a Rust table that references every symbol, which keeps them linked from the rlib.
  Other packages (the `yavs` CLI) get the paths as `DEP_YAVS_VHPI_VERSION_SCRIPT`, `_DEF` and
  `_IMPORT_LIB_DIR` (the crate has `links = "yavs_vhpi"`) and must pass them to their own binary
  link. The spike binary `yavs-vhpi-host` gets them via `rustc-link-arg-bins`.
- **Variadics:** `vhpi_printf`, `vhpi_vprintf`, `vhpi_assert`, `vhpi_control` are in `src/shim.c`
  (stable Rust cannot define C-variadic functions) and call plain Rust functions.
  `vhpi_control` ignores the optional argument after `vhpiStop`/`vhpiFinish` (the Python testbench framework passes
  `vhpiDiagTimeLoc`).
- **Design hierarchy (#269):** `yavs_vhpi::set_design(&db, &elaborated)` builds an owned
  snapshot of the elaborated hierarchy before plugins load; yavs-elab exposes the yavs-sem
  regions of each elaborated scope (`Elaborated::regions`) so VHPI lists declarations without a
  second walk. Classes, names and lookup rules are in `crates/yavs-vhpi/AGENTS.md`.
- **Unsafe:** the crate does not inherit the workspace lints, since `forbid(unsafe_code)` cannot
  be relaxed per crate; unsafe stays in `abi.rs` and `plugin.rs`, `model.rs` is safe Rust.
- **State and handles:** VHPI has no context argument, so the model is one process-wide state
  behind a mutex that is never held while plugin code runs. Handles are `(index, generation)`
  packed into the pointer value (rule 9), never NULL and never dereferenced; a released handle
  goes stale even after its slot is reused. 64-bit targets only (compile error otherwise).
- **Errors (fail early, fail clearly):** a plugin that does not exist, does not load (with the
  OS loader's message and error code) or has no entry point is a hard error before simulation
  starts: `E0901` (cannot load) and `E0902` (no entry point), rendered through yavs-diag.
  Each failed VHPI call sets the `vhpi_check_error` info (cleared by the next call, printf
  included). API misuse (NULL arguments, stale or wrong handles) is also reported on stderr every
  time as `W0903`; calls yavs does not implement yet as `W0904`, once per distinct message.
- **Simulation callbacks (#272):** a kernel `Observer` in the crate (`VhpiObserver`) runs them;
  the kernel got two phases for it, `EndOfInit` and `NextCycle`, and three more for #317:
  `StartOfProcesses` and `EndOfProcesses` around every cycle's processes (both references with
  VHPI fire end of processes there; one also start of processes) and `StartOfPostponed` (read-only,
  only when postponed processes run, as the one reference that accepts it fires it). IEEE VHPI has no read-write or
  read-only synch reason: the read-write phase is `vhpiCb[Rep]LastKnownDeltaCycle`
  (`Phase::LastKnownDelta`, puts there make one more delta) and the read-only phase
  `vhpiCb[Rep]EndOfTimeStep` (`Phase::EndOfTimeStep`, after postponed processes), which is what
  Python testbench frameworks register and what the reference simulator fires; the earlier plan
  of a wake-up at `now` for read-write was dropped because it always costs one extra delta and
  runs before the deltas still due at this time have settled. VHPI times count in units of `--resolution`. Disabled
  callbacks are kept out of the per-phase sets, so they cost nothing per cycle (#123, plan rule 9).
  Full rules: `crates/yavs-vhpi/AGENTS.md` section 3.
- **Puts and value changes (#271, #272):** `vhpi_put_value` goes through the kernel's observer
  forces and deposits, queued until the callback returns, and lands in the next delta: puts from callbacks before
  the update (after delay, next time step, start of cycle) are handed to the kernel at the
  start-of-processes phase of the same cycle, so they make a new delta as both references with
  VHPI do (#319). Identical at any thread count. An integer (`vhpiIntVal`) put into a logic
  vector is its two's complement with the rightmost element least significant (the references
  both accept it and disagree on the bits; the VHPI text gives no rule, this is the numeric
  reading, agent, 2026-10-19). The non-propagating modes `vhpiDeposit` and `vhpiForce` act like their propagating
  forms: the kernel has no value change without a cycle, and the reference simulator with VHPI
  refuses them. A deposit sets the driving value until a source next updates those elements;
  a release recomputes the driving value from the sources, so a force and release after a
  deposit return to the drivers' value, as both references with VHPI do (#310; a signal
  without sources keeps the deposit). Value-change callbacks repeat until
  removed and run once per cycle.
  A lookup that finds nothing (`vhpi_handle_by_name`) is only reported through
  `vhpi_check_error`, since Python testbench frameworks probes names routinely. Codes `E09xx`/`W09xx` are reserved
  for VHPI.
- **Verified:** Linux x86_64 by `cargo test` (C plugin built against only the header, loaded
  through `vhpi_startup_routines` and through `LIB:ENTRY`, plus missing file, missing entry and
  unresolved-symbol errors). Windows x86_64 GNU by cross-building with MinGW and running under
  wine (the export table has exactly the 42 symbols, the plugin DLL imports from
  `yavs-vhpi-host.exe` through the llvm-dlltool import library, same output as Linux). The
  tests themselves cannot run under wine since they call the C compiler. Windows MSVC (`/DEF:`,
  `lib.exe`, `cl` in the tests) is only build-path code until Windows CI runs it.

## std value conversion: the reference simulator formats, LRM parsing (yavs-std)

- **Text is Latin-1 bytes.** Every VHDL-level string in `yavs-std` is `[u8]`, one byte per
  CHARACTER, matching the runtime's STRING layout. Identifiers keep their declared spelling.
- **Output formats follow the reference simulator byte for byte** where the LRM leaves them to the implementation,
  because the reference simulator is the differential reference for report text: `real'image`/`to_string(real)` is
  C `%.15g` (`1e-05`, `0.333333333333333`), `to_string(real, 0)` is `%.17g` (round-trips),
  `write(real)` with `digits = 0` is `%e`, `time'image` is in fs, `to_string(time, unit)` is an
  integer when exact and `%.15g` otherwise, and `write(time, unit)` is exact decimal with at most
  `min(floor(log10(unit)), 17)` truncated digits (the second reference simulator agrees on the last one). The second reference simulator prints reals
  as shortest round-trip VHDL literals (`1.0e-1`) and the third reference simulator as `%e`; neither is adopted.
- **The LRM wins over the reference simulator** where the reference simulator is wrong or crashes, each case listed with its reason in
  `crates/yavs-std/golden/nvc-deviations.txt`: extended identifiers keep case and doubled
  backslashes; `'VALUE` takes full literal syntax (based literals, exponents, underscores only
  between digits, decimal physical literals rounded to the primary unit, unit-only `ns`, extended
  unit names) and rejects what is not one literal (`Fo o`, `1__0`, `.5`, `5.`); overflow is an
  error, never wrapped; `to_string(real, format)` is full C99 printf for one real (flags, `%a`,
  literal text), with anything else an error; no fixed output buffer limits.
- **Real literals are correctly rounded in every base** (ties to even, subnormals rounded at
  2^-1074, `Abstract::to_f64` through exact big-integer arithmetic), for source literals (sem)
  and `'VALUE` alike (#105). The reference simulator agrees for decimal and normal-range based literals and flushes a
  based subnormal it should round (`2#1.1#E-1075` is 0 in the reference simulator, 2^-1074 here); the LRM's "nearest
  value" wins.
- **Leniencies kept from the reference simulators**: a leading `+`, an integer literal for a real, no separator
  in `5ns`, and format effectors (HT, CR, LF, VT, FF) as whitespace around a value.
- **READ**: numbers are `[sign] digits [. digits] [E [sign] digits]` with no underscores or bases
  (the reference simulators agree), TIME accepts decimals (`1.5 ns`, as the second reference simulator; the reference simulator rejects), the unit is the
  longest unit name prefixing the rest (`5 nsx` reads 5 ns), a failed READ leaves the line
  unchanged (the second reference simulator; the reference simulator consumes part of it) and returns a structured error that the GOOD form
  maps to `false`. READLINE drops the CRs just before LF (CRLF files) but keeps inner CRs (the reference simulator
  deletes every CR); WRITELINE writes LF on all platforms.
- **env**: TIME_RECORD arithmetic is exact in microseconds and years outside 1..4095 are errors
  (the reference simulator wraps epochs past 2**31 and loses microseconds in `-`); `seconds_to_time` truncates like
  the reference simulator but errors on overflow; LOCALTIME uses the system zone through `jiff`; `DIRECTORY.items`
  are sorted, without `.` and `..`; TOOL_* name yavs.
- **Verification**: `golden/gen.py` writes testbenches, runs the reference simulator 2008 and 2019 and records
  goldens (about 2,900 cases per revision); the Rust tests compare against them without needing
  the reference simulator, and proptest checks `'value('image(x)) = x` and WRITE-then-READ round trips.

## Parser (yavs-syntax, M0)

Choices made while writing the M0 parser that the plan does not settle. Each is easy to revisit.

- **Interner:** one global table behind a mutex, strings leaked (`Symbol::as_str` gives
  `&'static str`). Parsing is single-threaded, so the lock is never contended, and symbols stay
  valid on runtime worker threads. The AST keeps both the original spelling and the folded key of
  every identifier (rule 8); extended identifiers fold to themselves.
- **Fail early:** `parse_file` returns the tree only when there was no error. Inside one file
  the parser still recovers (skip to `;`, local recovery to `then`/`loop`/`generate`, a missing
  `;` at a line end is reported and assumed) so independent errors are listed together. Syntax
  errors at a token that had a lexical error, or at the same token as the previous error, are
  dropped as cascades.
- **Conditional analysis in both modes:** `` `if `` etc. are evaluated for 2008 files too
  (`VHDL_VERSION` is `"2008"`), since 2008 already allows tool directives and the evaluation is
  harmless. Unknown directives (`` `protect ``) warn (W0002) and skip their line. `TOOL_VENDOR`
  and `TOOL_NAME` are `yavs`, `TOOL_TYPE` is `SIMULATION`, `TOOL_EDITION` is empty.
- **2019 closing keywords:** 2019 makes only `end component` optional (LCS-2016-055a); every
  other ending keeps the 2008 rule. The reference simulators 2025.3 reject a bare `end;` for every
  other construct (`docs/design/lrm-checks.md` P1); yavs gives E0002 with a help naming the rule.
- **2019 conditional expressions** (`a when c else b`, `docs/design/lrm-checks.md` P2): bare in
  constant, signal and variable initial values, interface defaults (U-5, permissive), association
  actuals (also after `inertial`) and `return`, plus anywhere as the 2019 primary
  `( conditional_expression )` (VI#385). The operand of a qualified expression is not a primary,
  so `T'(a when c else b)` is an error with a help to write `T'((a when c else b))`. `unaffected`
  and a final `when c` without `else` only in simple variable and force assignments and `return`;
  selected variable assignments take plain expressions. In a 2008 file a conditional expression is
  `E0004` at its `when` (#33). The plain `return when c;` (10.14) is its own AST statement
  (`SeqStmtKind::ReturnWhen`), an HIR `If` around a `return` in sem (#239).
  A `generic map` after a name is part of the name (a call of an uninstantiated subprogram) only
  inside expressions.
- **Strict where tools differ:** `abs x ** 2`, chained relations (`a > b = c`) and mixed logical
  operators without parentheses are errors, as in the LRM grammar. `10ns` (no space before the
  unit) is accepted with warning W0003, because it is common in real code.
- **Ambiguity left to sem:** `label: p;` in a concurrent region is a `ProcedureCall`; sem turns it
  into a component instantiation when `p` is a component. Choices and generic actuals that look
  like expressions may be subtype names.
- **Source encoding (user decision):** `--encoding=auto` (default) reads a file as UTF-8 when it
  is valid UTF-8 and as ISO-8859-1 bytes otherwise; `latin1` always takes each byte as one
  character; `utf8` makes invalid UTF-8 an error (E0010). Characters are ISO-8859-1 values either
  way, so `"é"'length` is 1 in a UTF-8 file as in a Latin-1 one. In a UTF-8 file, a character
  outside ISO-8859-1 in a string, bit string or character literal, or in an identifier, is error
  E0009 naming it (U+XXXX) with a help line pointing at `--encoding=latin1`; comments may contain
  anything. Spans are always byte offsets into the file on disk; `SourceMap` keeps the bytes of a
  Latin-1 file and maps offsets when rendering.
- **PSL is skipped with a warning (user, 2026-10-18, #138):** PSL declarations (`default clock`,
  `property`, `sequence`), directives (`assert`/`assume`/`assume_guarantee`/`cover`/`restrict`/
  `restrict_guarantee`/`fairness` with a PSL property) and verification units (`vunit`, `vmode`,
  `vprop`, `use vunit` in a binding) are skipped and give one `W0004` warning per design unit
  (`-Werror` or `-Werror=W0004` makes it an error). A statement or declaration is skipped to its
  `;` outside parentheses, brackets and braces, a verification unit to its closing `}`, so the
  rest of the file is analyzed, elaborated and simulated; the PSL itself never runs. A concurrent
  `assert`/`cover` is PSL when its property starts with `{`, `strong` or a PSL operator word
  (`always`, `never`, `eventually`, `forall`, `next*`) followed by something other than `;`,
  `report` or `severity`. `E0005` ("PSL is not
  supported") is retired and never reused.
- **A PSL subset runs as implicit checker processes (agent, 2026-10-19, #226, on the user's
  request in the issue):** `default clock is c;` (in an architecture, block or generate; it holds
  in nested blocks and generates), `assert`/`assume` with `always b`, `never b`, `always a -> b`,
  `always a -> next b`, `always a -> next[n] b` (n up to 256) on VHDL conditions, a property in
  parentheses, `cover b` or `cover {b}`, an `@c` clock after the property, `report` and `severity`
  clauses. The parser turns each such directive into a process in the tree (`parser/psl.rs`):
  `wait until c;` then the check, `next[n]` with `n` boolean variables as a shift register, so
  sem, lowering and the kernel need nothing new, failures are ordinary assertion reports at the
  clock event's delta (the values the signals have then, as the reference simulators sample) and
  the output is the same at any thread count. `b` is a VHDL condition (`??` applies), so `never b`
  fails only when `b` is true (`'X'` passes). Defaults: severity `error`, message `PSL assertion
  failed` (the first reference simulator's text); a `cover` prints its `report` text as a note on
  every hit and nothing without one (both references). A directive that does not fit (no clock,
  SEREs, other operators, a PSL built-in call such as `rose(a)`, a name of a skipped `property`/
  `sequence`, anything left after the property) is parsed again as before and skipped with
  `W0004`; the attempt's diagnostics are dropped. `-- psl` comments are read only with the option
  `--psl` (`parse`, `analyze`, `elab`, `run`; user, 2026-10-19, as the references need one; the
  first version read them by default; native statements are always read) when the text after `psl` starts with an optional `label :` and a PSL
  directive or declaration keyword and reaches a `;` before a line that is not a comment or is
  another `-- psl` comment; the following comment lines continue the statement up to its `;`.
  Prose such as `-- PSL assert failed` (in the 2019 `std.env` source) stays a comment. `->` is
  lexed as one token (never valid VHDL). yavs is permissive where the references reject: `@`
  clocks, `assume` and `severity` (catalogue rows `psl-*`).
- **Corpus lists** live in `tests/parse-corpus/` (`tests/corpus/` is the differential
  simulation corpus). The reference simulator `test/regress` files tagged `2019` in `testlist.txt`, and `test/parse`
  files whose test sets `STD_19`, are parsed as 2019; everything else as 2008 (yavs has no 1993
  mode, so 93 code that uses 2008 reserved words is an expected failure). The conformance suite's `vhdl-ams` part is
  VHDL-AMS and is not parsed.

## Kernel round 2: K1 to K7 (yavs-kernel)

- **Bit-plane signals:** `SignalType { elem: Bytes(n) | Logic, len }`. Logic signals use
  yavs-logic's `Block` layout for every buffer (values, drivers, transactions, forces), with
  chunked shift/mask range ops; byte signals keep plain bytes. yavs-logic gained additive
  accessors for this: `Planes::blocks`, `Planes::from_blocks`, `Planes::resolve_block`,
  `Logic::repr`, `Logic::from_repr`.
- **Ports (K1):** an `out`/`inout`/`buffer` formal is one more source in the actual's source list;
  an `in`/`inout` formal takes its effective value from a port layer. Driving values propagate
  bottom-up and effective values top-down by port level inside one update phase. Conversions see
  and return whole ranges.
- **Implicit signals (K2):** `'stable`, `'quiet`, `'delayed` and `'transaction` are kernel state
  updated in step c of the cycle; `'stable(0)` is FALSE for exactly the delta of the event.
- **Null transactions (K4):** disconnection is per element. A `bus` with no connected source
  resolves the empty set (`std_logic`: `Z`), a `register` keeps its value; `'driving_value` of a
  disconnected driver is a runtime error.
- **User files (K6):** dropped in kernel round 3 (#82), see there.
- **Protected types:** callers are `serial` through M3; the kernel needs no further support.
- **`SimError` is boxed** (`Deref` to `ErrorInfo`) to keep hot-path `Result`s one word; a panic
  in kernel code becomes an internal error like a panic in process code.

## Kernel round 3: files, serial marking (yavs-kernel, yavs-ir, 2026-10-18)

- **K6 dropped (#82):** the kernel has no user-file streams; `Ctx::output` is textio `OUTPUT`
  only. The interpreter reads and writes disk files directly (write, close, reopen-for-read in
  one delta works, `textio/file_roundtrip`). Why: K6 had no user, and buffering breaks that idiom.
- **File users are serial (#82, #107):** a `CallNative` of a file or directory native (implicit
  file operations, `readline`/`writeline`/`tee`, `env.dir_*`) sets `TOUCHES_SHARED`. Why: file
  objects of different processes can name one external file, and their bytes on disk depended
  on `--threads`. Ponytail: processes that only write to `OUTPUT` become serial too.
- **`Process::serial` (#107):** a process reports its own serial need; `Builder::process` ORs it
  into `ProcFlags`. Why: yavs-elab built every process with `serial: false`, so shared-variable
  and protected-type callers ran in parallel in arbitrary order (outputs differed run to run at
  `--threads 8`).
- **Composites resolved as a whole (#83):** they keep their usual leaves (logic fields on
  planes) and are resolved as one group (`Builder::resolved_composite`); the resolver sees whole
  composite values, the leaves concatenated in leaf order. Why: bit-planes everywhere (user,
  2026-10-18) rules out one byte leaf per composite. The leaves must share their sources and
  port level; guarded or partly driven composites are refused (not implemented). Amends the
  ir.md 5 leaf rule. A group's parts are leaf slices (#333: an element of an array of
  resolved records is one element of each leaf); a signal may hold several groups, which must
  cover it, and each group is resolved at most once per cycle.
- **Implicit BIT/BOOLEAN signals on planes (#185):** `'transaction`, `'stable` and `'quiet` are
  one logic element with the codes of '0'/'1', as for every BIT and BOOLEAN signal (the 1-byte
  0/1 form reached the interpreter as BOOLEAN position -2). yavs-elab mirrors the type.

## Semantic analysis (yavs-sem, M1)

- **Static values outside the base range** of their type (the declared range, Q2) are overflow:
  `E1401` in required contexts, `W1401` elsewhere. Array type definition bounds are required
  contexts (Q5), so `array (0 to integer'high * 2)` is an error.
- **Division by zero, `mod`/`rem` by zero, negative integer exponent** in a static expression go
  through the same rule: `E1401` where the value is required, `W1401` with the run-time check kept
  otherwise.
- **Operator declarations (4.5.2, 9.2):** logical operators take 1 or 2 parameters (2008 unary
  reductions), `+`/`-` 1 or 2, `not`/`abs`/`??` 1, the rest 2; other symbols are `E1303`.
- **Function parameter modes:** `in` only (`E1360`), except that 2019 files allow `out`/`inout` on
  impure functions.
- **Missing subprogram bodies (`E1230`)** are checked at the end of every declarative part except
  packages (checked at the end of the package body), protected types (s6) and entities (checked at
  the end of an architecture, whose declarative part may hold the body: 12.1, #177). A package or
  entity without any analyzed body or architecture is not checked at analysis. A spec in an entity
  takes the body of the last architecture analyzed in the session (ponytail in `decl.rs`).
- **A failed file hides all its units in the session** (`Db::analyze_file`): one session over
  many files gives the per-file result of one `yavs analyze` per file. A file fails when a unit
  reports an error, fails silently (8.2), or needs a unit that failed; then every unit of the file
  is `Failed`, since the CLI records no unit of a failed file. Before, in-session counts were
  inflated by dependents of failed units (vunit-2008: 133 in-session against 108 per file).
- **Type names in messages** are qualified by their region (`work.i1.rec`) only when two names in
  one message collide.

## IR and interpreter (yavs-ir, yavs-interp, M2)

- **Memory model:** registers are untyped u64 cells; addresses are `[space: 3 | index: 29 |
  offset: 32]` over eight spaces (frame, instance, package, constant, heap, temp, shared,
  shared heap). Every data-dependent access is bounds-checked; an out-of-object access is an
  ICE (lowering must have emitted the VHDL check), a freed object is a runtime error.
- **Temporaries** (`Alloc`) live until the next statement of the frame that made them, so a
  function's unconstrained result survives the return without a copy.
- **Natives** are looked up by `lib/NATIVE.md` symbol, parsed from the file at build time: a
  symbol that NATIVE.md does not define fails `Runtime::new` (fail early), one that is defined
  but not implemented is a runtime error naming it. `'image` is `yavs.attr.image` (NATIVE.md
  has no attribute natives). Implemented now: `now`, `stop`, `finish`, `to_string` and `'image`
  of scalars; the rest (textio, file, env extras) come with M3.
- **Foreign calls from safe code:** `yavs_ffi::ForeignFn::from_fn` binds a typed Rust
  `extern "C" fn` without `unsafe`; real symbols are bound by the elaborator (the one unsafe
  step) and passed in `Bindings`. Scalars and scalar `out` parameters are marshalled; arrays,
  records and access values are a "not implemented" runtime error until M3.
- **Heap:** generational handles; mark and sweep of the private heap at suspensions with
  precise roots from `Func::roots`; collection is skipped (never unsound) for objects whose
  handles cannot be located. Handles inside objects are found through the object's descriptor,
  so arrays whose elements need their own descriptor are scanned too (#80). The shared heap is
  collected between cycles (#303, "Shared heap collection" below).
- **Heap addresses carry no generation; deallocated slots are quarantined instead** (#81,
  yavs-interp). The address format has no room for a generation next to a 24-bit slot index
  and a 32-bit offset, so a deallocated slot is not reused until its process suspends
  (registers are dead then) and no aligned word in the process's frames or temporaries decodes
  to an address of it. The same address search keeps such objects alive across a collection
  (a by-reference parameter of a waiting procedure). An access through a stale address
  therefore always meets the freed slot and is the runtime error `access to a deallocated
  object`. For the shared heap the addresses of every serial process's frames count, as each
  published them at its last suspension (#303).
- **2019 `std.env` assert state, call paths and time_record** (#99, yavs-interp `env.rs`):
  - The assert state (counts, enables, read severity) is global but read deterministically
    (rule 3): a process sees the state as of the start of its cycle plus its own changes;
    all processes' changes are applied in process order when the next cycle first touches the
    state. The reference simulator updates at once, so a same-cycle read across processes is order dependent there
    (`tests/corpus/env/assert_count_cycles` is `ref=hand`).
  - As the reference simulator: report statements count too; a disabled severity is neither shown nor counted; the
    no-argument `GetVhdlAssertCount`/`IsVhdlAssertFailed` cover warning, error and failure;
    the read severity starts at ERROR. The exit status follows the error and failure counts
    at the end of the run, so `ClearVhdlAssert` clears a pending exit 1, as the reference simulator (#191).
  - `SetVhdlAssertFormat` with a non-empty format is a "not implemented" runtime error; the
    format stays the default and `GetVhdlAssertFormat` returns "".
  - Call path elements are innermost first, names upper case, the line of the calling
    statement, `file_path` the absolute directory; `to_string` of an element is the reference simulator's
    `<path>/<file>:<line>:<NAME>`. `to_string` of a vector honours `Separator` (LRM); the reference simulator
    1.23 ignores it and always uses LF.
  - `DELTA - TREC` moves the record back by DELTA, like `TREC - DELTA`.
  - Records are passed to natives as fat pairs, like arrays (what lowering emits; the README
    said "by address", which broke `dir_open` end to end).
- **Debugging like the other major simulators** (user requirement): conditional line breakpoints
  (the condition is an IR function nested in the stopped frame's function) with hit counts,
  step in/over/out per process, a global step across processes, pause inside endless loops,
  signal breakpoints as an event-bus observer that pauses `SimControl`, call stack and locals
  through `FrameView`, a process table with what each process waits on. Stops inside a process
  require `--threads 1`. API in docs/design/ir.md 3.4.
- **Verifier** is one linear walk; register liveness across suspension is checked within a
  block only (cross-block needs dataflow, JIT tier).
- **Disk files bypass the kernel's K6 streams:** writes go straight to the file. With K6
  buffering a process that writes a file, closes it and reopens it for reading in the same
  delta (a common testbench idiom) would read nothing. Determinism holds because a file object
  belongs to one process or to serial processes. `STD_OUTPUT` still goes through the kernel's
  OUTPUT stream so it stays ordered with `report`. K6 was removed in kernel round 3 (#82).
- **Open, needs a decision: BIT/BOOLEAN in bit-planes.** yavs-elab stores BIT leaves as their
  positions (0/1, codes of 'U'/'X'); the interpreter's natives write the codes of '0'/'1'
  (2/3), with which `LogicOp`/`LNum` (std_ulogic tables) compute BIT results correctly. The
  interpreter reads both for now. Proposal: '0'/'1' codes everywhere.
- **Foreign arrays** are passed as C-layout copies left to right (the reference simulator convention), logic
  elements one code byte each (BIT/BOOLEAN 0/1); records as naturally aligned structs;
  access values as a pointer into the object (yavs additions). Copy-back range-checks.

## Generics (yavs-sem, S4)

- **Formal packages with a generic map** (6.5.6.3, 6.5.7.2): the actual instance must have been
  instantiated with the same actuals (static values, base types, subprograms); a mismatch is
  `E1503`. The LRM is clear and the third reference simulator rejects; the reference simulator does not check. Non-static values are not
  compared. Inside the generic the formal package stays opaque.
- **A type through a formal package of a formal package is distinct (L76, #101; user, 2026-10-19:
  majority of the reference simulators):** a subprogram parameter of type `s.r.arr` (`s` a formal
  package of the generic package, `r` the formal package of `s`'s generic) in an instance does not
  accept the actual instance's type (`int_ring.arr`): `E1300` at the actual of the call, with a
  note. Two of the three reference simulators reject the call; the third rejects the name itself.
  One formal package deep is unchanged. The check lives in `call_args` (parameters flagged while
  the generic unit is analyzed, cloned with the instance); it applies only when the formal's type
  belongs to a concrete instance, so a call inside a generic unit with the unit's own `s.r.arr` is
  accepted.
- **W1501 (unused generic)** applies to the generics of packages and subprograms, reported once
  the unit is complete: after the package body, after the package declaration when the library
  holds no body for it, after a generic subprogram body. Entity, component and block generics are
  not checked (they are used across units).
- **Generic protected types (2019):** analyzed with opaque generics; an object of one is `E1505`
  until the parser accepts the generic map on variable declarations.
- **An unknown generic in a generic map does not hide a missing actual:** both `E1100` and
  `E1502` are reported, as the reference simulator does.
- **Entities with generic types, subprograms or packages (#75):** the entity and architecture
  are cloned once per distinct (entity, architecture, map of those actuals), not per instance:
  like an ordinary architecture, the instances share code and differ in instance data (the
  generic constants). The clone's mangled name carries the actuals (sem.md 5.3), so rule 4
  identity holds per map. Rationale: shapes and lowered code depend on the actual types only.
- **Components and blocks with generic types, subprograms or packages (#222):** a component is
  cloned once per distinct map (`Db::instantiate_component`), elab elaborates the clone's
  generics and ports and passes the actuals through the binding to the entity's clone; a block
  (one generic map) is replaced in place by its clone at the end of its architecture, after the
  configuration specifications are resolved (`Db::instantiate_blocks`), so configuration
  declarations analyzed later and lowering see an ordinary block. Configuration specifications
  in a cloned region are cloned with it. Rationale: the same mechanism as entities (#75), no
  formal type reaches elab or lowering.

## Design units and binding (yavs-sem, S5)

- **Default binding (Q4, 7.3.3):** an entity made directly visible by a use clause wins, then the
  one in the library of the design unit that declares the component (#291; majority of the
  reference simulators, one of three picks the instantiating unit's library instead), then the one in
  the instantiating design unit's library (`W1703` when a different one exists there); the architecture is the last one in
  manifest or analysis order. `Db::default_binding(scope, component)` with the instance's
  `StmtKind::Instance::scope`.
- **Selected entity names:** `architecture a of lib.e` is accepted (LRM 3.3.1 gives a name; the reference simulator,
  the reference simulators accept it); `lib` must be a visible library name and the architecture's own.
- **Block configuration of a default-bound instance** (`for u : c  for rtl end for;  end for;`
  with no binding indication): `E1704` (user, 2026-10-19, majority rule: two of the three
  reference simulators reject it; it was accepted before). A default binding (7.3.3) is no binding
  for a block configuration; bind the instance explicitly or by a configuration specification.
- **`others` in a component configuration** (7.3.1) leaves out the instances that earlier
  component configurations of the same block configuration name, so `for others : c use entity
  ...` next to `for u2 : c ... end for;` does not rebind a `u2` bound by a configuration
  specification (`E1702` only when an instance it covers is spec-bound). The second reference
  simulator agrees; the reference simulator checks only at elaboration (#75).

## Reflection: TIME values in mirrors (user, 2026-10-18)

`PHYSICAL_VALUE_MIRROR.value` for a `TIME` value is expressed in units of
`std.env.resolution_limit`, as a commercial simulator does (the reference simulator returns femtoseconds). The LRM is unclear,
and the test runner's `record_dict_pkg` (PR #1239) relies on this reading. With the default femtosecond
resolution both readings give the same number. Applies to the M3 runtime that builds mirrors.
`PHYSICAL_VALUE_MIRROR.unit_index` is then the index of the unit equal to the resolution limit
(the primary unit for other physical types).

## Reflection: names, indices and case (user and yavs-interp, 2026-10-18)

- **Physical unit indices are 1-based** (`unit_name(1)` is the primary unit), as the reference simulator; the LRM
  does not say (user).
- **A physical mirror's `image`** uses the reference simulator's unit case: `5000000 FS` (user). Other `image`
  results are the type's `'image`.
- `simple_name`, `element_name` and `unit_name` are upper case, as the reference simulator returns them (LRM silent;
  the test runner's `record_dict_pkg` accepts any case). Enumeration literals, element and unit names are
  looked up case-insensitively (the reference simulator compares `enumeration_literal(name)` case-sensitively; the
  permissive reading accepts both spellings).
- Record element indices and `enumeration_literal(idx)` count from 0 (the element / literal
  position within the subtype), as the reference simulator.
- **Index bases checked (#156, yavs-interp, 2026-10-18):** the 2019 package declaration types
  only `enumeration_literal(literal_idx : NATURAL_INDEX)`, consistent with `pos` from 0;
  `element_name`, `element_index`, `unit_name`, `unit_index` and `scale` take plain `INDEX`, and
  the LRM prose fixes no base for them (unclear, so rule 2: match the simulators). The reference simulator 1.23
  gives literal 0 = first literal, element 0 = first element, unit 1 = primary unit; yavs
  keeps exactly these bases. They differ between classes, but changing either would break
  designs written against the reference simulator (the test runner's `record_dict_pkg` walks elements `0 .. length - 1`).
  Pinned by `tests/corpus/reflect/index_bases`.
- `array_value_mirror.get` takes LRM indices (`get(left - i)` on a `downto` array is element `i`
  from the left); the reference simulator 1.23 normalises some indices differently (`tests/corpus/reflect/multidim`
  is `ref=hand`).
- A conversion to the wrong class (`value_mirror.to_record` of an integer) is a runtime error.
- `p'reflect` of an access object mirrors the access value (`class_access`), not the designated
  object the prefix is also appropriate for (LRM 8.1); the reference simulator agrees
  (yavs-sem, 2026-10-19, #97).
- A scalar subtype whose range is not static (generic or parameter bounds) is reflected with its
  range evaluated where `'reflect` is evaluated, also as an element, index or designated subtype;
  mirrors made from that mirror keep it (yavs-lower, yavs-interp, 2026-10-19, #97).
## Incomplete types in protected method profiles (user, 2026-10-18; amends U-2)

Allowed only in units of library `std` (`std.reflection` needs it). In user code it is `E1220` with
a note and a help suggesting an access type, as the reference simulators reject it. Not extended to
`ieee`: nothing there needs it.

## Elaboration (yavs-elab, M2)

- **Element order inside a kernel leaf:** composites flatten row major, leftmost first; a logic
  leaf (bit-planes) mirrors it, kernel offset `n - 1 - p` for row-major position `p` (element 0 of
  yavs-logic is the rightmost element). `NetGraph::select` is the one implementation; lowering
  computing offsets of non-static indices must use the same rule.
- **Logic leaves:** every leaf whose scalar is `std_ulogic`, `bit` or `boolean` (scalars too) is
  `Elem::Logic` with the enumeration position as the code, matching `yavs_ir::LogicKind`. Other
  enums are 1 or 4 bytes, integers, physicals and reals 8.
- **Records** become one leaf per field (arrays of records: struct of arrays).
- **Unlabeled processes** are named `_pN`, N counting the unlabeled process-like statements of
  the region (the reference simulator's names, so difftest paths match).
- **Component instances** get a hidden `Component` scope holding the component ports as their own
  signals (two port edges per port, uncollapsed); it is not part of path names, the bound entity
  instance carries the label.
- **Interim evaluator:** globally static expressions are folded with sem's static folder over
  generic and constant values (`Db::fold_with`); calls of user functions with scalar arguments
  and results run in yavs-interp after lowering the function alone (ir.md 3.6). Composite
  values (#77, elab agent, 2026-10-18): structural operations (one-dimensional and record
  aggregates, indexing, slices, fields, concatenation, composite `=`/`/=`, logical operators
  on BIT and BOOLEAN arrays, conversions to constrained array subtypes) are evaluated on
  `ConstValue`s in yavs-elab; everything else (ieee and user functions) runs in yavs-interp,
  with arrays of scalars and records of scalars marshalled by `yavs_interp::call_values` in the
  IR layout. Not chosen: lowering a thunk per expression, which would need instance data for
  every scope's generics before elaboration has built it. Since #220 (elab agent,
  2026-10-18): aggregates of any number of dimensions, and any composite (arrays of records,
  arrays of arrays, records with array elements) marshalled in the IR layout (`Ty::layout`);
  the predefined operations not evaluated structurally (shifts, array ordering, matching
  operators, `TO_STRING` of reals and physicals, ...) run their yavs-interp native on the
  marshalled operands (`call_native`), so elaboration and simulation share one implementation.
  Not chosen: more structural operations on `ConstValue`s, a second implementation of each.
- **Elaboration-time calls with package state** (#77, elab agent, 2026-10-18; LRM 14.4): a
  function that cannot be lowered alone (it uses package objects, say a shared variable
  holding a registry) runs in a session program: every run-time package lowered and initialised
  first (as for the simulation), then the function, against that package state, so each call
  sees the side effects of those before it. Constants whose initial value calls an impure
  function are evaluated in declaration order when their region is elaborated (others when
  first needed), and a package constant with an impure initial value is read from the package
  data, never evaluated again. The calls are logged with their argument values; building the
  simulation initialises the packages again and replays the log before any instance data, so
  the processes see the same state and `build` stays repeatable (rule 4); a replayed call
  returning another value is an internal error. Not chosen: handing the session's heap and
  package data to the simulation program, whose type, function and constant-pool ids differ.
  `NOW` is 0 fs during elaboration (yavs-interp). ponytail: a new function relowers the
  session and replays the log (O(functions x calls)); a constant with an impure initial value
  elaboration cannot evaluate at all (E2106: `'PATH_NAME`, a null multi-dimensional aggregate)
  is left to its run-time thunk with warning W2118 (#220), the calls made for it are undone,
  and the thunk runs after all replays, so it sees the elaboration-time calls of later
  declarations and regions first.
  A process's own declarative part is not evaluated here: the process elaborates it when it
  starts (its lowered code), once, after the elaboration-time calls of every region, as the
  reference simulators do; evaluating it during elaboration as well ran its calls twice (#251).
- **2019 variable ports** (#188, elab agent, 2026-10-19; LRM 6.5.2, 6.5.6.3, LCS-2016-047):
  the formal denotes its actual's protected object. Elaboration resolves each formal to the
  shared variable at the end of its chain of actuals (`Elaborated::var_ports`: a region's
  shared variable or a package's); lowering gives the formal an 8-byte instance cell
  (`UnitIr::var_ports`), and building writes the object's handle into it, read from the
  declaring region's instance data (built first, so the region's architecture is lowered even
  without processes) or from a package thunk (`Lowerer::package_value`). Calls through the
  port are ordinary protected calls, so the processes run serially like any other user of a
  shared object. Sem rejects an `open` or unassociated variable port (`E1311`; one
  reference simulator crashes on the latter), elaboration a top-level one (`E2119`).
- **Package objects at run time** (#183, elab agent with the lowering agent, 2026-10-18):
  packages outside std with signals, shared variables, files or constants sem did not fold are
  lowered (`Lowerer::package`) before the architectures and initialised once at build time
  (`yavs_interp::init_package`), in analysis order (a package after those its declaration uses;
  ponytail: a body using a later-analyzed package would need dependency order), sharing the
  processes' `SharedState`. Every signal of a loaded package is elaborated, driven or not.
- **Objects of generic package instances** (#101, 2026-10-19; LRM 4.9, 14.4.2): an
  uninstantiated generic package has no objects; only its instances do. A package instance
  that is a design unit is a package with run-time objects (its cloned visible part and body,
  `Db::package_regions`), with its own package data and its own signals (`:lib:inst:s`). A
  package instance declared in another region (package, package body, entity, architecture,
  block, generate, process, subprogram) adds its objects to that region, in declaration order
  at the instantiation (`Db::decls_with_instances`): instance data per elaborated architecture
  instance, block or generate iteration, frame slots per process or call, package data of the
  enclosing package. Every region kind therefore needs no separate instance storage.
- **Objects of nested packages** (#34, 2026-10-19; LRM 4.7, 4.8): a package declared in a
  declarative part (`DeclKind::Package`) adds the objects of its declaration and body to the
  declaring region, like a package instance (`Db::decls_with_instances`); a generic one has
  none, its instances do. An instance of a generic package nested in a generic package
  instantiates the clone made with the enclosing instance, so the inner body sees the outer
  actuals; elab clones instance bodies until no new instance appears (a cloned body declares
  instances of its own).
- **2019 signals with bounds from the initial value** (#187, elab agent, 2026-10-18; LRM
  6.4.2.3, LCS-2016-019): the elaborator evaluates the initial value first and fills every
  bound the subtype leaves open from it (`Model::shape_init`), element bounds from the first
  element and record element bounds per field, so the signal's shape is the value's, as for
  constants. Architecture, block, generate and package signals; a package array signal's
  descriptor lives in package data (`PackageIr::signals[..].dope`), written by the elaborator
  after the package's initialisation. Ports are interface declarations, not signal
  declarations: the LCS does not cover them, an unconstrained port still takes its bounds from
  its actual (an open one with a default takes the default's bounds, #218). Record signals
  whose fields take bounds this way have a descriptor slot too (#215, below).
- **Linkage ports and mode views** (#75, elab agent, 2026-10-18): a linkage port is an
  uncollapsed port edge with the kernel mode `in` (VHDL code cannot read or update it, LRM
  6.5.2, so it never drives its actual). A port with a 2019 mode view takes each record leaf's
  mode from `Db::port_mode_tree` (arrays of records: the element view for every element).
- **Package constant calling a function of its own package** whose body is not elaborated yet
  (LRM 14.4.2): E2114 with both spans, as the reference simulators do. Since package data is initialised
  at run time (#183), `Elaborated::lower` checks every constant of each run-time package's
  declaration before lowering, so a constant only processes read is E2114 too, never
  evaluated at package initialisation (#197, the LRM is clear).
- **Composites resolved as a whole** (#193, elab agent, 2026-10-18; LRM 4.6, 14.7.2,
  14.7.3): a signal whose record or array subtype has its own resolution function keeps its
  usual leaves (`LeafRes::Whole`), joined by the kernel's `Builder::resolved_composite`. A
  process driving any subelement of it gets a driver over every leaf, so every source is a
  whole composite (subelements it never assigns keep the signal's initial value). The resolver
  (`whole.rs`) repacks the concatenated leaves into the IR layout of an array of the composite,
  calls the function through `call_values` and unpacks the result. `(f) t` with a composite
  element resolves each element as a whole.
- **Composites resolved as a whole inside other composites** (#333, elab agent, 2026-10-19;
  LRM 4.6, 14.7.2, 14.7.3.1): the outermost resolved composite on each path of a signal is a
  basic signal (`NetGraph::wholes`, one `Whole` per such subelement with one instance per
  element of the arrays above it: a record field of a record, every element of an array of
  resolved records or bit vectors). Each instance is one kernel group over slices of its
  leaves (one element of each struct-of-arrays leaf), so each array element has its own
  sources; a process driving part of an instance gets a driver over that instance only.
  Resolution functions below a resolved composite (a `std_logic` field of a resolved record,
  an inner resolved record) are not used: a subelement of a resolved signal is not a basic
  signal. Two reference simulators agree; the other two crash, give wrong values or refuse
  the partial drivers (catalogue `record-nested-resolved-whole`, `array-nested-resolved-whole`).
- **User resolution of scalar leaves** (#293, elab agent, 2026-10-19; LRM 4.6, 14.7.3.2):
  the resolver of a leaf resolved element by element (`LeafRes::Func`) calls the function
  once per element of the leaf, with that element of every source. On a logic leaf
  (`std_ulogic`, `bit`, `boolean`) the sources are passed as a logic array of bit-planes, the
  IR layout of the parameter, and the returned position becomes the kernel code (plus 2 for
  BIT and BOOLEAN); the kernel stays plane-based.
- **External name as a port actual** (#186, user, 2026-10-19: same delta, majority of the
  reference simulators; LRM 8.7, 6.5.7.1, 14.7.3; `docs/design/lrm-checks.md` L49):
  `i => << signal ... >>` on an `in` port is a signal name actual, since the external name
  denotes the signal itself (8.7). It is an ordinary port edge: the formal starts at the
  actual's value and follows it in the same delta, as two of the three reference simulators
  do. Replaces the orchestrator decision of 2026-10-18 (an expression actual driven by a copy
  process, one delta later, as the third does). `inertial << signal ... >>` stays an
  expression actual (6.5.6.3).
- **Expression actuals of ports** (#74, elab agent, 2026-10-18; LRM 6.5.6.3, 6.5.7.1): an `in`
  port (or a part of one in an individual association) associated with an expression that is
  not a signal name gets the expression's value as its initial value when the expression is
  globally static. Otherwise it is driven by the equivalent process `formal <= expr` of the
  instantiating region, an ordinary IR process (yavs-lower `PortActual`, flag `IMPLICIT`)
  sensitive to the expression's signals, with one driver, the formal's part; the formal keeps
  its own initial value until delta 1, as the reference simulators do. The process
  runs with its region's instance data plus the formal as one more signal and, for an array,
  the formal's descriptor, against which the value is checked. A conversion function is only
  the syntax `name(signal)`: an operator such as `not s` is an expression actual (sem).
- **The rest of the expression actuals** (#74, portrest agent, 2026-10-19; LRM 6.5.6.3,
  7.3.2): a formal with records in its type is driven leaf by leaf, driver slot `k` for leaf
  `k` in `NetGraph::select` order. An expression actual in the port map of a binding
  indication (configuration specification or declaration) is the equivalent process of the
  component's region: it reads the component's ports and generics, which yavs-lower makes
  objects of the instantiating architecture's unit and yavs-elab binds in that region
  (`Lowerer::port_actual` names the instance statement). A scalar formal whose range comes
  from the instance's generics is checked against bounds yavs-elab evaluates in the instance
  and writes into the process's instance data (`PortActual::bounds`); the message names the
  port, as the reference simulator. An unconstrained formal takes the bounds of a static
  value, and those of a conversion function's constrained result subtype; a non-static
  expression without a globally static subtype is E2111 (LRM 6.5.6.3 is clear, both
  reference simulators reject). `inertial` signal-name actuals are expression actuals (#233),
  also on a part of an individual association (#261, LRM 6.5.6.3 is clear; the reference
  simulators split: one follows the LRM, one ignores `inertial` on a part, one does not parse
  it; `docs/design/lrm-checks.md` L80): the HIR keeps the reserved word per part
  (`IndividualPart`), and an `inertial` part of a port that is not `in` is E1360.
- **Conversions on unconstrained ports** (#262, portleft agent, 2026-10-19; LRM 5.3.2.2,
  `docs/design/lrm-checks.md` L87): a formal that is not fully constrained takes its bounds
  from its conversion, the actual part's result subtype or type mark for modes `in` and
  `inout`, the formal part's parameter subtype or type mark for `out` and `buffer`. sem keeps
  the type mark's own declaration for a type conversion actual (a subtype, not the base type)
  and rejects a conversion that does not define the bounds (E1312 on both sides). User, 2026-10-19:
  this narrows #111's 2026-10-18 ruling (reject every type conversion on an unconstrained formal):
  a type conversion on an unconstrained formal is accepted when it defines the bounds (a constrained
  type mark or result subtype, LRM 5.3.2.2); only a conversion that gives no bounds is E1312. The reference simulators split
  (one gives the formal the actual's bounds, one does so for formal-part type conversions,
  one crashes on several forms), so the clear LRM text decides. User, 2026-10-19: subprogram
  call formal conversions (#229) follow the same rule: the formal gets the conversion's bounds
  at run time (the actual part's result subtype for `inout`, the formal part's type mark or
  parameter subtype for `out`), `E1312` only when the conversion defines none.
- **Range check of expression actuals of parts** (#263, portleft agent, 2026-10-19; LRM
  6.5.6.3, 14.7.3.1, `docs/design/lrm-checks.md` L88): the equivalent process of a non-static
  expression actual of a scalar part (`r.f => x + 1`, `v(0) => x - 3`) checks the value
  against the part's subtype, as for a whole formal. The subtype comes from the elaborated
  port's own subtype along the part's path (`scalar_part`), not from sem's formal part: that
  one was resolved against the uninstantiated entity, so a formal package's generic would
  have no value. The message names no port, as the reference simulator's.

- **Aliases of external signal names** (#19, LRM 6.6.2, 8.7, 14.4.1): `alias s is
  <<signal .tb.dut.x : t>>;` in an architecture, block, generate or process binds when the
  elaborator reaches the alias declaration, in elaboration order, so an alias declared before
  the instance it names is `E2117`, as the reference simulator reports. The alias is an
  instance signal slot of its region (a process's alias belongs to the enclosing region's
  instance data) that the elaborator maps to the bound target's kernel signal
  (`Elaborated::sigs` keyed by the alias), its descriptor written from the target's bounds; the
  process's drivers and sensitivity rooted at the external name go through that slot
  (`Cx::ext_sigs`, `Cx::root_decl`).

- **External names used directly, external constants and variables, records** (#93, LRM 8.7,
  `docs/design/lrm-checks.md` L71): an external name in the statements of a process or
  concurrent statement, in the initial values of its declarations, in its sensitivity, wait and
  driver names, in an expression or `inertial` port actual, or inside the target of an alias of
  a part of one gets an implicit alias (`Db::alias_external_uses`, one per name and owner,
  `implicit`), and the use is rewritten to name the alias (`ExprKind::Obj`); everything after
  treats it like a declared alias. Elab binds the implicit aliases of statements and actuals
  after the whole hierarchy (`Elab::bind_late`), and processes whose drivers are rooted at
  external names get their drivers then too, in their original process order; declarations bind
  where they are elaborated. An alias of an external constant is an instance value holding the
  target's elaborated value (a constant initialised from an external name is evaluated by elab,
  never by a run-time thunk); of an external shared variable (protected types only) an instance
  cell holding the variable's object, copied from the target region's data when the instance
  data is built (`Elaborated::ext_vars`; regions may name each other's variables); of a signal
  with records one instance signal per leaf. External names in subprograms and packages,
  external shared variables in packages and constants without an elaboration-time value: next
  entry (#255).

- **External names in subprograms and packages** (#255, LRM 8.7, `docs/design/lrm-checks.md`
  L86): a subprogram's external names (its body, its declarations, the subprograms and
  protected bodies nested in it, and its declared aliases of whole external names) get implicit
  aliases owned by the subprogram (`ExtOwner::Sub`) that belong to the region declaring it, so
  the shared code finds them where it finds that region's objects. In an architecture, block,
  generate or process they are instance objects bound per elaborated region after the whole
  hierarchy, as statement aliases are (`E2117` when a name names nothing, called or not: two of
  the three reference simulators). In a package they are package objects bound once per design
  after the hierarchy (`Elaborated::pkg_ext`): a signal is a package signal, a constant's value
  and a shared variable's object are cells in the shared variables area written by `build`
  (package data is complete before the regions holding the targets exist; ponytail: processes
  reading them run serially). A name that cannot be bound (`Elaborated::ext_unbound`) makes its
  subprogram fail when called, which is how two of the three reference simulators behave; before
  elaboration is complete (elaboration-time calls) every such subprogram fails the same way. A
  package path (`@lib.pkg.obj`) is resolved once by `Db::bind_package_paths` and becomes a name
  of that package object (LRM 8.7: it denotes the object), so package shared variables named
  that way are called directly. A package declaration naming the hierarchy is `E2117`. An
  external constant whose target only a run-time thunk computes (`'path_name`) is bound to that
  target (`Elaborated::ext_consts`) and takes its value when the instance data is built. A
  procedure outside a process driving an external signal is `E1810` (LRM 10.5.2.1).

- **Open unconstrained `in` port with a default** (#218, elab agent, 2026-10-18; LRM 6.5.6.3,
  14.3.5, `docs/design/lrm-checks.md` L59): accepted in 2008 and 2019; the port takes the
  bounds of its default value (`Model::shape_init`). Without a default it stays `E1311`.
- **Deferred constants at elaboration (#295):** the value of a deferred constant read during elaboration (generic default, range, generate bound) is that of the full declaration in the package body (LRM 4.8), evaluated like any package constant; the body is elaborated before the design that uses the package (LRM 14.4.2). A deferred constant without a full declaration is `E2106`.

## Runtime: default exit severity (user, 2026-10-18)

`yavs run` defaults to `--exit-severity=failure`, like the reference simulators. A report or assert of
severity ERROR is printed and the simulation continues; the run exits 1 at the end if any ERROR or
higher was reported and not cleared by `ClearVhdlAssert` (the std.env assert counts, #191). FAILURE stops the simulation at once. yavs's own runtime errors (bound,
overflow, null access, delta limit, internal) still stop immediately, per the error motto.
`--exit-severity=error|warning|note` lowers the threshold. The test runner passes its own setting.
Reports go to stdout in the reference simulator's format (`** Note: <fs>fs+<delta>: msg`, then `    Process <path>`),
which tools/difftest parses.

## Report during elaboration (yavs-interp, #299)

- **Decision:** a `report` or assertion run without a kernel (a function called while
  elaborating, e.g. in a signal's initial value) prints `** <Severity>: (init): <message>` on
  stderr at once, before the first cycle, and elaboration goes on. Severity failure stops
  elaboration with a runtime error (`E2107`). Why: LRM 10.4 allows report in any subprogram;
  the reference simulator prints the same line. Not counted in the VHDL assert counts (no
  process context yet). Source: #299.

## Interpreter: BIT/BOOLEAN codes and file writes (orchestrator, 2026-10-18)

- BIT and BOOLEAN elements of bit-plane arrays use the std_ulogic codes for '0'/'1' (2/3), not
  their enum positions 0/1, so the shared plane operations are correct for `bit_vector`.
  yavs-elab follows; VHPI, waves and foreign calls translate at the boundary.
- Disk files are written directly by the owning process, not through the kernel's buffered
  streams, so write, close and reopen-for-read in one delta works. Output stays independent of
  `--threads` because every caller of a file native is serial (kernel round 3). STD_OUTPUT
  still goes through the kernel output stream, ordered with `report`.
- **Globally static calls and port attributes (9.4.3):** a call of a pure function whose actuals
  are globally static is globally static (sem never folds it; only implicit operations become
  locally static). Array attributes of a port or generic whose subtype is not fully constrained,
  or of a sub-element of one (`bus.dat'length`), are globally static too: its bounds are fixed at
  elaboration, and the reference simulators accept such constants in static names (seen in
  AXI-stream and Wishbone verification components).
- **Implicit versus explicit:** a call ambiguous only between implicit operations and explicit
  subprograms picks the explicit ones (2019: a user `to_string(a; sep := " ")` beside the
  implicit composite `TO_STRING`, in the same package or through a use clause). The reference simulators
  accept the random packages of two verification libraries this way; the LRM does not settle it.

## Implicit operation natives (yavs-interp `implicit.rs`)

- **Symbol form separator** (#163): the operand-shape suffix of an implicit native follows `#`
  (`yavs.implicit.cat.<type>#ss`), because a dot made a type named `pi`, `ss` or `reduce`
  ambiguous with a form. Records never reach the natives: lowering inlines record equality.
  Contracts for calls that do reach a native (`cat`, `rising_edge`, `deallocate`) are in
  `crates/yavs-lower/DESIGN.md` "Requests from yavs-interp".
- **Shifts with a negative count** follow LRM 9.2.4 (`sla -k` is `sra k`, `sra -k` is `sla k`),
  as the second reference simulator does. The reference simulator fills from the wrong end for negative arithmetic shifts.
- **Concatenation bounds** follow LRM 2008 9.2.5: two null operands give the right operand;
  otherwise the result starts at `S'LEFT` in the direction of `S` and both bounds must belong
  to `S`. The second reference simulator agrees; the reference simulator gives `S'LEFT to S'LEFT - 1` for two null operands and a wrong
  (null) range for downto index subtypes. The `cat` native takes `S'LEFT`, `S'RIGHT` and the
  direction as three extra cells.
- **`?<` .. `?>=` with a `'-'` operand** is a runtime error (LRM 9.2.3 says it is an error);
  the reference simulator returns `'X'` silently.
- **2019 `TO_STRING` of an array** whose element type is not a character enumeration is
  `(e1,e2,...)` as the reference simulator prints it; a null such array gives `()` (the reference simulator prints `)`).
- **`MINIMUM`/`MAXIMUM` of a null array** give the element type's `'HIGH`/`'LOW` (the reference simulator agrees).

## Differential testing (tools/difftest, M2 conformance, 2026-10-18)

- **Default assert message (#12):** for `assert c;` without a report expression the reference simulator prints a source
  snippet instead of the `Process` line. yavs keeps its CLI format (cli-contract section 5: the
  `Process` line after every report), and the difftest normaliser drops the path of an
  `Assertion violation` event on both sides.
- **`relaxed` header (#285):** a test of a construct only `--relaxed` accepts (VHDL-93 style
  shared variables) runs each candidate with its own switch; analysis output is shown only when
  analysis fails, and yavs also gets `-Wno-W1904` because it repeats analysis warnings when it
  loads the units. The second reference simulator rejects these shared variables even with
  `-frelaxed`.
- **Exit 2 is not always an elaboration failure:** `stop(2)` exits 2 like the reference simulator, so the yavs template
  runs `yavs elab` first and maps only its failure to "elaboration failed".
- **Report order within one cycle (#11), superseded by "Process order within a cycle" below
  (#252, user 2026-10-19): yavs kept committing in elaboration order.** the reference simulator's
  order is neither elaboration nor wakeup (FIFO) order: six processes whose timeouts all fall on
  3 ns, scheduled at 0, 1 and 2 ns, report as p2 p4 p1 p5 p6 p3 in the reference simulator, an artefact of its event
  heap; processes woken by one signal event report in reverse registration order. Matching that
  would mean reproducing the reference simulator's heap layout, which the LRM does not ask for (the order is
  unspecified) and which no other simulator shares. A deterministic wakeup order would match
  the reference simulator no better than ProcId order, so the kernel is unchanged and `--threads N` stays
  byte-identical by construction. difftest instead gained a per-test `ignore=order` header; the
  `order` mask now sorts a cycle's events stably by process path, so each process's own order is
  still compared. Strict comparison stays the default for yavs.
## Lowering (yavs-lower, M2)

- **Instance data per region:** an architecture's `UnitIr` covers its blocks and generates.
  yavs-elab builds one instance data per region (instance, block, generate iteration) and
  looks values and signals up from the process's region outwards. Slots of nested regions'
  values and signals stay empty in outer regions, whose processes never name them.
  Constants and shared variables declared in the entity or architecture itself
  (`InstConst::instance_level`) are computed once per instance and shared by all its regions
  (#190). A shared variable of a block or generate is created once per elaboration of its
  declarative part (the block, the chosen alternative, each `for` iteration) and shared by
  the regions nested in it, found through `Elaborated::shared_vars` (LRM 14.4, 6.4.2.4, #249).
- **Instance values:** generics, generate parameters and non-static constants are written by
  the elaborator from its folded values (`UnitIr::write_value`); constants it cannot
  fold run a thunk once per region (a composite's returns its fat pair, appended to the
  instance data by yavs-elab, #73). Locally static constants have no slot.
- **Driver slots** are one per `ProcessInfo::drivers` entry, in order; a target picks the entry
  with the same root and path, else the entry with the same root whose path is a prefix of the
  target's. A subprogram declared in a process drives through that process's slots (14.7.2,
  #206). In a driver that sem collects from such a subprogram, the prefix ends before the first
  index or slice that reads an object of a subprogram (a parameter, or a constant computed from
  one): its value exists only when the call runs, so elaboration never folds it and the process
  drives the shorter prefix (#205). Indices are evaluated at run time as everywhere else.
- **GUARD:** sem keeps a guarded block's condition as the GUARD signal's default; lowering
  emits the guard process (`GUARD <= condition`, zero delay) and guarded assignments that
  disconnect `bus`/`register` targets when GUARD is false, after the signal's disconnection
  time: elab takes it from `Db::disconnect_times` (a disconnection specification, 7.4) and
  gives it to the kernel signal, 0 fs without one (#260).
- **Individual association of parameters** (#71, LRM 6.5.7.1): a variable or constant formal
  associated part by part is a temporary of its subtype; every part is copied in before the
  call (also for `out`: a composite formal shows its actual's value) and back after it for
  `out`/`inout`. A signal formal of several parts is a reference of several parts (see
  "Multi-part signal references and port converters"; one reference simulator rejects the
  form, the other agrees on the values). An unconstrained
  formal's bounds run from the lowest to the highest index named, in the index subtype's
  direction, as yavs-elab does for ports.
- **Individually associated record ports with unconstrained elements** (#288, LRM 6.5.7.1):
  the formal is shaped element by element from its parts; an element associated as a whole
  takes its actual's bounds (a signal name's, else the expression's subtype's or static value's,
  else E2111 as for a whole port), an element named index by index the lowest to the highest
  index named, as for array ports. An unconstrained element left out is E2111. Two reference
  simulators agree on nested records; the third fails their elaboration.
- **Conversions in call associations** (#229, LRM 6.5.7.1, lrm-checks L66): sem gives
  `Actual::Converted` with the actual part (`copy_in`, `inout` only) and the formal part naming
  the formal (`copy_out`) as expressions, so lowering needs no conversion logic of its own: the
  formal is a temporary of its (fully constrained) subtype holding `copy_in`, or the subtype's
  default for `out`, and `copy_out` is stored to the actual after the call with the formal bound
  to the temporary, as for individual association. The copy is taken even when a type
  conversion could share the actual's data, so the actual does not change during the call.
- **Matching case** (`case?`, `select?`) compares with `?=` on BIT and STD_ULOGIC elements:
  '-' matches anything, metavalues match nothing, L/H match 0/1 (LRM 9.2.3).
- **`'path_name`/`'instance_name`** are strings in instance data (`UnitIr::names`), written by
  yavs-elab per region: a design unit names its instance, a signal or constant the region that
  declares it, a process label its process. Objects declared in a process or subprogram add
  the path elements between the region and the object (#207, LRM 16.2.5): the process label
  (an unlabeled process gives an empty element: `:t::v`, `:t:b::v`, as the reference
  simulators print it) and each enclosing subprogram as `name[type,type return type]` in
  lower case (`:t:main:pr[integer]:x`, a subprogram's own name ends in `:`). Loops and
  sequential blocks add nothing. Ponytail: a subprogram declared in an enclosing region of the
  process's own region is named from the process's region.
- **Shared variables of non-protected types** (`--relaxed`, W1904, #285): stored like a process
  variable (a scalar in place; an array or record as its fat pair over an object in the shared
  heap), in shared state. A package's lives in the shared variables area (`Space::Shared`),
  initialised by the package's thunk. One of an architecture, block or generate is one object per
  declaring region like a protected one (#249): its thunk creates the slot in the shared heap and
  returns its address, which the elaborator writes into an instance cell; names reach it through
  that cell (`Space::Region`). Every function naming either is `TOUCHES_SHARED`, so the processes
  reaching it, through any chain of calls, are serial: output is identical at any `--threads`,
  and `--shuffle` exposes testbenches that depend on process order (`docs/plan.md` "Parallel
  kernel design"). A type holding access values is allowed since #303 (below).
- **Signal aliases** (#203, LRM 6.6.2) get no storage and no slot: every use lowers the aliased
  name (`Fx::alias_target`), with the alias subtype's descriptor when it is constrained (a
  length check at each use, `SubtypeCheck`). Sem's LSP of a name through an alias with an array
  subtype carries `LspStep::Rebase`; yavs-elab (`Model::rebase`) maps the following indices and
  slices by element position onto the aliased part, so drivers, port actuals and sensitivity
  cover exactly the named elements.
- **Array signal descriptors** live in instance data (`SignalIr::dope`), written by yavs-elab from
  each signal object's bounds, so unconstrained ports take their actual's bounds. Array signal
  parameters pass the signal reference, the driver slot (`out`/`inout`) and the actual's
  descriptor; their elements and slices are sub-ranges of the reference.

## CLI: JSON diagnostics, `-W` forms, `--explain` (yavs CLI, 2026-10-18)

- **JSON diagnostics follow the common compiler shape** (`level`, `children`, `rendered`, plus `code` as a plain string
  and byte/line/column `spans`), as `docs/cli-contract.md` section 0 promised. The contract won over
  the first renderer (`severity`, `notes`, `help`) because nothing consumed that shape (#158).
- **`-W` forms:** `-Werror`, `-Werror=CODE`, `-Wno-CODE`, `-WCODE`, for `analyze`, `elab` and
  `run`; per code the last option wins; `CODE` must be a registered warning code, else `E0103`.
  Same spelling as the contract's `yavs.a_flags` list, GCC-like rather than the `-A/-W/-D` style (#137).
- **`--explain CODE`** lives in the CLI crate, not in `yavs-diag` (a leaf): one table for all stages
  but `yavs-sem`, which keeps its own. A test fails for any code literal without a text (#136).
- **A panic is `E9999`, exit 3** in every command (#159).
- **Package objects (#183):** each package with run-time objects gets package data
  (`AddrSpace::Pkg`, `PackageIr`): an initialisation thunk evaluates its constants that are not
  locally static once (composites of static layout in place) and creates its protected shared
  variables, run once by the elaborator with `yavs_interp::init_package` (the package's data is
  writable only there). A declaration constant calling a function of its own package never
  reaches the thunk: yavs-elab rejects it first with E2114 (#197, see "Elaboration").
  Package signals take the first indices of every unit's signal table.
  Composite package constants whose bounds are known only at run time are evaluated there
  too, their elements in package data appended by `PkgAlloc` (#73): every constant is
  evaluated once, so an impure initialiser's side effects happen once.
- **Allocators of shared objects (#184, #246):** every allocation (allocators, textio lines,
  mirrors) made by elaboration code run with the shared state (`call_shared`, `init_package`,
  `call_values_shared`: the initialisers of shared protected objects and of package objects)
  or inside a method of a protected object in the shared heap, and by anything those call at
  any depth, lands in the shared heap, so shared state never points into a private heap that
  dies with its process or with an elaboration call. The interpreter decides it per frame
  (`Frame::shared_alloc`, inherited by callees, `Machine::alloc_heap`), not the lowering, so a
  function lowered once allocates in the right heap for every caller. The shared heap is
  collected (#303, "Shared heap collection").
- **Shared heap collection** (#303, user, 2026-10-19): objects reached through a non-protected
  shared variable live in the shared heap, like those of protected objects and of elaboration
  code, and the shared heap is collected by precise mark and sweep at a serial safepoint: the
  start of a cycle (`Phase::NextCycle`), when no process runs (`yavs_interp::SharedGc`, an
  observer yavs-elab subscribes). Rationale: allocating into the shared heap without collecting
  it lets long runs of logging code grow without bound; copying objects on store would cost
  every store. How:
  - A serial process allocates everything in the shared heap (its root frame is
    `shared_alloc`, inherited by callees), since a value it makes may end up in a shared
    variable through any chain of calls; it already runs serially (#285).
  - Roots: the shared variables area (`Program::shared_roots`, a root map like a frame's), the
    objects alive when the simulation starts (`Heap::seal`: package and instance data, which
    hold protected objects and region shared variables, have no root maps), and the frames of
    every serial process, which each publishes at its suspensions (`SharedState::procs`: handles
    from its root maps and the slots addresses reach). Signals cannot hold access values;
    VHPI and observers hold none. A region's shared variable slot is typed for the GC (a
    record whose third word is the handle of the variable's object).
  - Trigger: once more was allocated since the last collection than both `Limits::gc_threshold`
    and what the last collection kept, so the work stays proportional to the allocation
    (ponytail: a fixed rule, no tuning by pause time). Never during a cycle; output identical at
    any `--threads` and with `--shuffle`.
  - Ceiling (ponytail in `Heap::seal`): garbage made during elaboration is kept with the sealed
    objects; root maps for package and instance data would let it go.
- **Record signals** have one `SignalIr` per leaf (`SignalIr::path`, field indices), mapped by
  yavs-elab to the kernel leaf with that path. A longest static prefix covers one driver slot
  per leaf below its fields, so `ProcessIr::drivers` is the flattening of `ProcInst::lsps`.
  Reading a record signal reads it leaf by leaf into a record value; driving it drives each
  leaf with its field.
- **Arrays of records** (any signal type with a record in it, #200) use the same `SignalIr`
  per field path; each leaf holds its field of every element (struct of arrays). A name
  below the signal (fields, indices, one slice at the end) selects one contiguous range per
  leaf, by the rule of `NetGraph::select` (row major, logic leaves mirrored), computed at run
  time from the signal subtype's descriptor (rule 1). A longest static prefix covers the leaves
  below all its field selections (indices select elements, not leaves), one driver slot each.
  Reading any name below such a signal reads the whole signal and then selects (ponytail:
  read only the selected ranges if large memories of records need it); driving a part copies
  each leaf's elements of every waveform value into one transaction per leaf.
  The descriptor of such a signal's whole type, when the type needs one, is a slot in instance
  data written by the elaborator from the signal's shape (`SignalIr::dope` on its first leaf
  entry, with the IR type it describes), so bounds from the initial value, a partially
  constrained subtype or a port's actual reach every name below it (#215, rule 1).
- **A target's driver slot** is the `ProcessInfo::drivers` entry with the same prefix, compared
  structurally with folded indices (`Db::same_lsp`, sem's deduplication), so a static element
  assigned twice in one process finds the one entry sem kept (#215).
- **2008 array-valued aggregate pieces** (#89, LRM 9.3.3.3): a positional 1-D aggregate
  whose elements are slices or elements is the concatenation of its pieces; its left bound
  and direction are those of the index subtype, as for `&`, and a constrained context checks
  the length and gives the bounds. As a variable or signal target, each piece takes as many
  elements as its own target has, from the left, and the pieces must cover the value exactly
  (checked before anything is assigned). A piece under a range choice
  (`(7 downto 4 => a, others => '0')`, #230) fills the range's elements in the aggregate's
  order whatever the choice's direction, as both reference simulators do; its length must
  match the range. Without a constrained context such an aggregate takes the direction of
  those range choices, not of the index subtype, so `(3 downto 2 => a, 1 downto 0 => b)` is
  `3 downto 0`; choices of different directions are `E1334` (all three reference simulators
  take the choices' direction, two reject a mix; lrm-checks L84). An element subtype the type leaves open takes its constraint from a fully
  constrained context, else from the first value, and every value is checked against it.
  Pieces exist in 1-D aggregates only (LRM 9.3.3.3; sem rejects them in more dimensions, as
  the reference simulators do). Range choices in aggregate *targets* are accepted (#244, user, 2026-10-19: accept,
  majority; this replaces the earlier ruling that made them `E1333`), with the mapping of the
  two reference simulators that accept them: the aggregate runs from its lowest to its highest
  choice in the direction of its range choices, its leftmost index takes the value's
  leftmost element, and each piece takes the elements of its range in that order (so
  `(0 to 1 => a, 2 to 5 => b) := v` gives `a` the two leftmost elements of `v` whatever `v`'s
  bounds). Where the two disagree yavs keeps choices order-independent (associations written
  out of index order); range choices of both directions are `E1334`, as in aggregate values
  (2 of 3 reference simulators reject them; lrm-checks L68, L84). A piece must have as many
  elements as its choice, checked before it is assigned. Element-only targets, `(0 => a, 1 => b) := v;`,
  use the same mapping (#264, 2026-10-19: LRM 9.3.3.3 gives the aggregate its index subtype's
  direction, two of the three reference simulators put the aggregate's leftmost index on the value's
  leftmost element, the third does not accept the form): the choices are not the value's own
  indices, whatever its bounds. A lone non-static element choice, `(i => a) := v;` (LRM 9.3.3.3
  allows no other choice with it), covers one element: `a` takes the value's single element. Every
  aggregate target must cover its value exactly (LRM 10.5.2.1, 10.6.2.1), checked at run time before
  anything is assigned, positional and element-only targets included.
- **`&` on arrays of composites** (#234, LRM 9.2.5): arrays of arrays and of records
  concatenate like arrays of scalars (bounds from the index subtype, two null operands give
  the right operand), each element copied as a row with the piece copy shared with
  aggregates. An element subtype the type leaves open takes the left operand's constraint
  (its element part, or the left element itself), and every operand is checked against it,
  as the reference simulators do (they reject a mismatch at analysis where it is static).
- **Named aggregate bounds take the direction of a constrained context** (#230, LRM
  9.3.3.3: assignment to a constrained target, constrained initial value, qualified
  expression, ...): lowering uses sem's result subtype for `AggBounds::Named`, not the
  index subtype's direction, so `v := (7 => o, 6 downto 0 => '0')` with
  `v : bit_vector(7 downto 0)` sets `v(7)`.
- **Signal parameters with records in their type** (#215) pass one signal reference per leaf
  of the formal (in leaf order, each two cells, so the interpreter's parameter copying is
  unchanged), then for `out`/`inout` the caller's driver slot of the first leaf (leaf `k`
  drives through that slot plus `k`: the leaves below a prefix are consecutive driver slots),
  then the actual's descriptor when the type needs one (a constrained array formal: its own,
  after a length check). Names below the formal select within those references by the same
  walk as signals (`comp_select`), so actuals may be whole signals, elements, slices, fields
  or other such parameters. A part of a composite signal as an `out` actual (of either
  kind of formal) drives its first leaf through the slot of the actual's longest static
  prefix plus the leaf's position below that prefix (`Fx::leaf_slot`).
- **Signal parameters with a mode view** (#254, LRM 6.5.2): a call drives only the actual's
  `out`, `inout` and `buffer` elements, so sem gives the calling process one driver per such
  element of the view's mode tree (`Db::view_driven_paths`: the actual's longest static prefix
  plus the element's field steps; after an array the `Field` step means that element of every
  array element, which `NetGraph::select` maps to the leaf's elements over the whole array).
  The formal is passed like an `inout` record signal parameter, except that each driven leaf is
  a reference of one part carrying its own driver slot (`Inst::SigParts`) and the slot cell is
  `-1`: the driven leaves' slots need not be consecutive. A view parameter passed on keeps those
  slots. An individually associated view formal (#265, 6.5.7.1) does the same per formal leaf:
  the leaf's mode comes from the view by the leaf's full path, and its driver is the one sem
  collected for the part's actual plus the leaf's path below the formal part
  (`Db::view_driven_below`, `Fx::view_leaf_slot`); the other leaves have no slot. Driving a non-static name below a
  view (`bs(i).ready`) is checked against the view by the name's own steps, not its longest
  static prefix (`E1903` only for an `in` element below the name).
- **`buffer` and `linkage` subprogram parameters** are `E1236` (LRM 4.2.2.1; the reference
  simulators reject them too); a view may still give an element those modes.
- **Package signals with records in their type** are one package signal per leaf, like
  architecture signals; the whole type's descriptor is in package data (#215).
- **Attributes of a part over several leaves** (#215, LRM 16.2.4): `'event` and `'active`
  are true when they are for any leaf's selected elements, `'last_event` and `'last_active`
  are the smallest of the leaves', `'last_value` is read leaf by leaf like the value.
  `'stable(T)`, `'quiet(T)` and `'transaction` of such a part are one kernel implicit signal
  with a source per leaf (`Builder::implicit_source`), `'delayed(T)` one delayed leaf per
  leaf of the prefix.
- **Implicit signals named in a process** (#240, LRM 16.2.4): sem declares the implicit signal
  of `'stable`, `'quiet`, `'delayed` or `'transaction` named in a process body (also inside
  a loop or sequential block), or in a subprogram declared in the process, in the process's
  scope. Lowering makes it an instance
  signal of the region enclosing the process (`region`), as for one named in a concurrent
  statement, and yavs-elab binds it under that region, so the process reads and waits on it
  like any other signal. Named in a subprogram of a region, it belongs to that region
  (yavs-sem); in a package body, to the package: a package signal that yavs-elab creates in
  the package's scope after the package's own signals. One reference simulator rejects all four in any subprogram body; LRM 4.2.2.2
  forbids them only of formal signal parameters, which the other one and yavs follow.
- **Arrays of arrays without records** (#89) stay one kernel signal of all their scalars, row
  major (logic mirrored). Every name of indices and slices below such a signal (`m(i)`,
  `m(i)(j)`, `m(i)(7 downto 4)`, `g(i, j)`), as target, actual, alias or sensitivity, selects
  one contiguous range by the same walk as composite signals (`Fx::sig_steps`), so a whole
  element is all its scalars, not one. A subtype alias step (`LspStep::Rebase`, `Step::Rebase`)
  takes the alias subtype's descriptor (length-checked) for the steps after it, and an index or
  slice after a slice (`m(0 to 1)(0)`) counts within the slice, so sensitivity through such an
  alias and names below a slice use the same walk (#230).
- **`x'element` of an object with open element bounds** (`FromObject` with `elem_depth` 1)
  lowers from the element part of the object's descriptor, after the object's own dimensions
  (`Fx::subtype_dope`); instance data now gets the signal descriptors before the constant
  thunks run, so `constant w : natural := p'element'length` (or `p'length`) of a port sees the
  actual's bounds (#235, #243).
- **Bounds of a record element of a signal** (`p.data'length`, `b.resp.user'range`, also below
  an index into an array of records) come from the signal's descriptor in lowering
  (`Fx::dope_of` through `Fx::comp_walk`) and from its elaborated shape in yavs-elab
  (`Model::sig_shape`), never from its value: constants, signal subtypes and generate ranges
  of an instance use the bounds the actual gives a record port (#304).
- **`x'subtype` of an object whose bounds are open** (`variable v : port'subtype`,
  `alias a : v.f'subtype is v.f`) is the object's own subtype: lowering takes the object's
  descriptor, yavs-elab its shape (`Model::shape_c` on `Constraint::FromObject`: a signal's
  elaborated shape, else the shape of its value). An alias of a composite variable part keeps
  its whole descriptor (record and element bounds too), checked against the alias subtype
  element by element (#315).
- **`'left`/`'right`/`'low`/`'high`/`'ascending` of a scalar subtype during elaboration**
  (integer, enumeration and physical types) evaluate the subtype's range in the instance
  (`Model::scalar_bounds`) instead of folding it, so a range naming a port
  (`integer range p'range`, the index subtype sem gives a range choice `(p'range => '0')`)
  has the actual's bounds (#311).
- **A folded constant read as a whole** (`Fx::expr`, `e.value` of an `Obj`) takes the descriptor
  of its own subtype when that has one, so the element bounds of an array constant are the
  declared ones, not those of the literals that fold into its value (#247).

## Foreign subprograms and `--load` (yavs-lower, yavs-vhpi, CLI, #95, 2026-10-18)

- **Attribute value:** `"VHPIDIRECT [library] symbol"`, keyword case-insensitive. The last
  word is the symbol and a library word is ignored, as in the reference simulator (symbols come from `--load`).
  Without a symbol the subprogram's name is used (as another simulator does; the reference simulator fails at the call). Any other
  foreign kind is `E2001` when a call is lowered.
- **Placeholder bodies:** a body for a `'FOREIGN` subprogram completes the spec but the
  subprogram stays foreign (other simulators require or ignore such a body). No body is needed.
- **C types** follow `yavs_ffi::vhdl` with the formal's own subtype: `integer range 0 to 50`
  is `int8_t`, as in the reference simulator. The function result is range-checked against the result subtype
  (`E3009`), so a C `uint8_t` 7 for a `boolean` is a runtime error, not a wrong value.
- **Binding:** after lowering, every `Program::foreign` entry is bound by `yavs run`: the
  `--load` libraries in order (`dlopen` with `RTLD_NOW | RTLD_GLOBAL`, `LoadLibraryExW`), then
  the executable. A missing symbol is warning `W0108` before time 0, one per
  unbound subprogram naming it, its symbol and the places searched (escalates with `-Werror`);
  `E0107` (same "searched" note, VHDL stacktrace) only when the subprogram is called, as the
  reference simulator does. User decision 2026-10-18: "Warn early, fail at call". A `--load`
  library that cannot be loaded stays `E0106` before the simulation. Only subprograms that some lowered code
  calls are bound. Libraries are never unloaded.
- **Loader home:** `yavs_vhpi::load_foreign`/`bind_foreign` share the plugin path rules and
  hold the one `unsafe` step (`ForeignFn::new`); `--load` runs no VHPI registration.
- **Verified:** Linux x86_64 (`crates/yavs/tests/foreign.rs`, C compiled at test time),
  aarch64 under qemu-user (same test, `QEMU_LD_PREFIX` set for the child process), Windows
  x86_64 GNU under wine by hand (a MinGW DLL; the test itself cannot spawn the compiler from
  inside wine).

- **VHPI models (#274, 2026-10-19):** `'FOREIGN "VHPI library model"` (exactly three words,
  keyword case-insensitive) binds to the model a `--vhpi` plugin registered with
  `vhpi_register_foreignf`, after the plugins load and before `build`; unbound follows "warn
  early, fail at call" (`W0108`, `E0107`). Parameters and results are scalars or arrays of
  scalars; the interpreter hands the plugin layer neutral cells (`yavs_interp::ForeignModel`),
  so `yavs-interp` knows nothing of VHPI. Details: `crates/yavs-vhpi/AGENTS.md`.

## Binary files (yavs-interp `binfile.rs`, #79, 2026-10-18)

`read` and `write` on `file of T` (LRM 5.5.2) are implemented for `T` a scalar type or an array of
scalars; the on-disk format is implementation-defined, so it is fixed here (yavs, no user ruling).

- **Scalars:** little-endian in the fewest whole bytes the type's range needs, unsigned when the
  range has no negative value, else two's complement (`character` and small enumerations 1 byte,
  `integer` 4, `real` and `time` 8). Enumerations of more than 256 literals use 4 bytes. This is
  byte for byte what the reference simulator writes, so files of scalars are exchangeable.
- **Arrays:** a `u32` element count (little-endian, all dimensions flattened), then the elements
  in the scalar format; `bit`/`boolean` elements are 0/1 and `std_ulogic` elements their position.
  The reference simulator writes no count, which makes the end of an unconstrained array
  unrecoverable (a read takes whatever follows); LRM 5.5.2 needs the length, so yavs stores it,
  for constrained element types too (the interpreter cannot tell them apart). Files of arrays are
  therefore not exchangeable with that simulator.
- **READ of an array:** the unconstrained form (with `LENGTH`) takes `min(stored, VALUE'LENGTH)`
  elements, skips the rest of the record and sets `LENGTH` to the number taken; the constrained
  form is a runtime error when the stored count differs. A value outside the type's range, or
  reading past the end of the file, is a runtime error.
- Records and arrays of composites are not implemented (runtime error naming #79).

## Linux release archives (2026-10-18)

- **glibc 2.17 floor:** the Linux archives are built with cargo-zigbuild against glibc 2.17 and
  stay dynamically linked; CI fails if `objdump -T` shows a higher `GLIBC_` version and smoke-tests
  each archive in a `centos:7` container. "As compatible with as many Linux distros as possible"
  (user, 2026-10-18); 2.17 covers RHEL/CentOS 7+, Debian 8+, Ubuntu 14.04+.
- **No static musl build:** considered and rejected (user, 2026-10-18). `--load` and VHPI plugins
  need `dlopen` and exported `vhpi_*` symbols, which need a dynamic glibc binary, and glibc 2.17
  already covers nearly all distros. Alpine users install `gcompat`.
- **Distribution sweep (2026-10-18):** the 0.4.0 Linux archives were run once in 36 distribution
  images on x86_64 and aarch64 (RHEL-family 7 to 9, Amazon Linux, Debian 8 to 13, Ubuntu 14.04 to
  24.04, Fedora, openSUSE, Arch, Gentoo, Void, Mageia, Photon, distroless, Alpine with `gcompat`):
  checksum, `yavs --version`, and analyze plus run of a design using `ieee.numeric_std` and
  `std.textio` passed in all others. Only plain Alpine (musl, no glibc loader; expected) and
  `busybox:glibc` (the image has no `libdl`) failed; both are image limitations.
- **Archive names:** the Rust target triples are kept (`x86_64-unknown-linux-gnu`,
  `aarch64-unknown-linux-gnu`, `x86_64-pc-windows-msvc`), without the glibc version, so installers
  that match on target triples find them; OS-arch names such as `linux-arm64` were considered
  (user, 2026-10-18). The docs say that aarch64 is also known as arm64.

## Multi-part signal references and port converters (yavs-ir, yavs-interp, yavs-kernel, #71, #74, 2026-10-18)

- **Port converters over several leaves** (#74): a port edge in the kernel is a list of parts on
  each side (`Builder::port_parts`; `Builder::port` is the one-part case), and `Converter` maps
  the values of all source parts to all target parts. A record port (or an array of records)
  with a conversion function is one such edge in yavs-elab (`ConvPort`), so the conversion runs
  once per cycle with every leaf current, in the update phase of the source's delta (LRM
  6.5.7.1, 14.7.3: the conversion is part of the port's driving/effective value). Rejected: one
  edge per leaf (a function of the whole record cannot be split per leaf), and an equivalent
  process per conversion (one delta late, unlike the reference simulators). Ordering: the edge
  converts at the part that the level-ordered update worklist reaches last, so the result does
  not depend on which part changed or on the thread count (docs/design/ir.md 4.1).
- **Type conversions of composite ports** (#74, 2026-10-19; LRM 9.3.6): closely related
  composites have the same leaves, so a type conversion of a record port or an array of
  composites is the identity leaf by leaf (an integer leaf to a real one converted element by
  element), with no function to call; leaf lengths that differ are E2110 in elaboration. An
  `inout` port converted on the actual side only is E1313 in sem (lrm-checks L72); one
  converted on the formal side only is accepted: copy-out converts, copy-in passes the actual
  unchanged (majority, user rule 2026-10-19: 2 of 3 reference simulators accept it, 2 of 3
  reject the actual-side form).
- **Signal references of several parts** (#71): an individually associated signal formal whose
  parts lie in different signals (`drive2(x(1) => s1, x(0) => s0)`, a record formal field by
  field) is a reference of several parts in the IR (`SigParts`, ir.md 1.8): still two
  registers, the second the address of a table of `[reference, length, driver slot]`, ordered
  by the formal's kernel elements. The interpreter maps every signal instruction onto the parts
  in order within one process step, so reads, drives, `'event` and the other attributes behave
  as for one signal in the same delta, at any thread count, without kernel support. Each part
  carries the caller's driver slot of its own actual, since parts in different signals have
  different drivers. Rejected: copying a signal formal like a variable (wrong for `'event`,
  `wait on` and for drives that must reach the actuals' drivers), and a reference per element
  (O(n) per array access). Slices of references are an instruction (`SigSlice`), so names below
  a formal work on both forms.


## Sem: user decisions of 2026-10-19

- **Named return values in `return` and actuals (L62, #33; user, 2026-10-19):** a call of a function
  with a named return value stays accepted in a `return` statement and as the actual of a
  constrained formal (permissive), although only some reference simulators accept those contexts.
  Recorded in `docs/design/lrm-checks.md` L62.
- **Implicit operations of a generic type in positional generic maps (L69, #223; user, 2026-10-19):**
  they take no position, with no warning; two of three reference simulators agree.
- **Calls through aliases of methods of shared and process variables (L15, #237; user, 2026-10-19):**
  stay accepted (permissive), although the reference simulators reject or crash on the calls.
- **Generic subprograms of components and blocks as actuals (L63, #222; user, 2026-10-19):** a generic
  subprogram of a component or block as the actual of an entity's generic subprogram in a generic map
  is `E1506` at the actual (all three reference simulators reject it). The default binding by name
  is unaffected.
- **Initial `'transaction` (L67, #241; user, 2026-10-19):** `S'transaction` does not toggle at
  initialization (LRM 14.7.5.2), as the third reference simulator and unlike the other two.
- **Scalar named return values (L77, #327; user, 2026-10-19, reverses the earlier ruling of the
  same day that kept them rejected with `E1233`, commit 2462e0aa):** `function f return r of t`
  with a scalar `t` is accepted; `r` is the call's target subtype and has its range, passed like
  array and record bounds. `E1233` stays for access and file types. Of the references that run
  it, one takes the target's range and one the type mark's; yavs keeps the reading of array and
  record named returns.
- **Incremental binding repeating the entity aspect (config-rebinding, 7.3.2.1; user, 2026-10-19: LRM
  and majority):** a component configuration may repeat the entity aspect of the configuration
  specification that binds the instance (same entity and architecture, or same configuration, after
  resolution); only a different entity aspect is `E1702`. Two of three reference simulators accept.
- **TIME units below the resolution limit (L78, #253; user, 2026-10-19: follow the LRM):** under
  `yavs run --resolution=U`, a non-zero TIME literal or unit name below `U` anywhere in the elaborated
  hierarchy (packages it uses, code that never runs) is `E2120` before time 0 (LRM 5.2.4.2); the
  built-in std and ieee sources are exempt (`std.textio` defaults `UNIT` to `ns`; user, 2026-10-19). One reference
  simulator only warns. Recorded in `docs/design/lrm-checks.md` L78.
- **Constant parameters of access type in 2019 (L46, #198; user, 2026-10-19, majority rule):** `E1223`
  in 2019 user code as in 2008, because all three reference simulators reject them. Only library
  `std` stays exempt (its `std.env` declares such parameters).
- **Constant parameters of access type in 2019 (L46, #198; user, 2026-10-19, majority rule):** `E1223`
  in 2019 user code as in 2008, because all three reference simulators reject them. Only library
  `std` stays exempt (its `std.env` declares such parameters).
- **Access parameters of protected methods in 2019 (L46, #212; user, 2026-10-19, majority rule):**
  `E1223` in 2019 as in 2008, because at least two of the three reference simulators reject them.
  Access results and file parameters stay accepted in 2019 (the simulators disagree).
- **Negated literal at the i64 minimum (L58, #214; user, 2026-10-19, majority rule):** `-9223372036854775808`,
  for integer and physical types, is `E1402` in user code (all three reference simulators reject it;
  the range of `universal_integer` is implementation-defined). Library `std` stays exempt: its `TIME`
  declaration uses the literal.
- **Implicit condition operator (L53, #48; user, 2026-10-19, majority rule):** only a `??` visible at
  the condition counts, so `use ieee.std_logic_1164.std_ulogic` alone gives `E1340`; two of the
  three reference simulators reject it (it was accepted before).
- **Formal conversions overloaded by result type (L66, #229; user, 2026-10-19, majority rule):**
  `set(cv(x) => r)` with two `cv(integer)` returning different types is `E1302`: the actual's type
  does not choose the overload (two of the three reference simulators reject it). Port maps already
  required a unique conversion.
- **Static index and slice out of range (L51, sem.md 4.3; user, 2026-10-19, majority rule):** a
  locally static index or non-null slice outside the static index range of its array is the new
  error `E1404`, also in code that never runs, because two of the three reference simulators reject
  it. Warning `W1404` is retired; the run-time check stays for everything not static.
- **`'element` chains of open objects** (`FromObject { elem_depth }`): sem adds one to the
  prefix's depth for each `'element`; `subtype_dope` walks the object's type one array level per
  depth, summing the dimension cells, to find the descriptor part (#256).
- **`'element` chains of open objects** (`FromObject { elem_depth }`): sem adds one to the
  prefix's depth for each `'element`; `subtype_dope` walks the object's type one array level per
  depth, summing the dimension cells, to find the descriptor part (#256).

## Process order within a cycle (user, 2026-10-19, #252)

- **Choice ("split by process shape"):** the processes of one batch run (serial ones) and commit
  their effects (reports, textio `OUTPUT`, `stop`/`finish`, forces, transactions, waits) in one
  fixed order, the same at any `--threads`. At initialisation: elaboration order. After that:
  the processes with one suspension point (a sensitivity list, or exactly one `Suspend` and no
  call of a procedure that may suspend: `yavs_ir::Program::single_suspend`) first, in
  elaboration order; then all others in reverse elaboration order, those resumed by a signal
  before those resumed by a timeout. Postponed processes follow the same rule. `--shuffle`
  still permutes only the execution order. The kernel gets the shape as
  `ProcFlags::several_waits` or `Process::several_waits`.
- **Why:** the order is unspecified (LRM 14.7.5.3), and the reference simulators differ.
  Measured per sub-case (issue #252), the majority of the three is reverse elaboration order
  for processes with several waits (instances, nesting, generates, blocks; events before
  timeouts; deltas after the first; postponed processes) and elaboration order for
  sensitivity-list and one-wait processes. This rule matches the majority in every sub-case
  that has one, and verification component testbenches that check a value an instance reports
  in the same cycle pass. Rejected: plain reverse order after initialisation (against the
  majority for sensitivity-list monitors), numbering child scopes first (minority for the
  hierarchy sub-cases), and keeping elaboration order (fails those testbenches).
- **Concurrent procedure calls (user, 2026-10-19, #281):** among the processes with several
  suspension points, the concurrent procedure calls (IR `IMPLICIT` and not `single_suspend`: the
  procedure loops on a wait) run before the other ones; each group in reverse elaboration order,
  signal-resumed before timeout-resumed as before. One-suspension-point processes stay first,
  in elaboration order. Why: a protocol checker written as a concurrent call must see a
  testbench process's report of the same edge after its own, as all three reference simulators
  do for the real `tb_check_stable` test (the external runner's `check` suite goes from 243 to
  257 of 257). The rule goes against sim2 and sim3 in a reduced reproducer (checkers before a
  one-wait testbench at the first edge) and matches the majority in the shapes of the real test;
  no rule over process shape, order and resume reason matches every sub-case. Kernel:
  `ProcFlags::concurrent_call`, `Process::concurrent_call`.
- Corpus tests whose processes report in one cycle and whose order the reference simulator does
  not share carry `ignore=order`; `process/process_order` is `ref=hand`.
- **Formals with open dimensions and constrained elements** (`Fx::param_dope`, #257): the
  callee's descriptor is built from the actual's dimensions and the formal's element
  constraint, and every constrained level is length-checked against the actual (signal and
  non-signal array parameters alike); `x'element` of such a formal is the formal's constrained
  element subtype, not a `FromObject` one (L79).

## DAP server (yavs-dap, M8, #130, 2026-10-19)

- **`yavs run --dap=PORT|stdio`:** the design on the command line is the one debugged; the
  client's `launch`/`attach` only starts it (`stopOnEntry` is a global step before the run).
  `stdio` is for clients that start the adapter; a port is for clients that attach (and for the
  later GUI, `docs/cli-contract.md` 6). Why: test runners and the GUI already build the command
  line; a second way to name the design would be a second contract.
- **One thread under `--dap`:** processes run serially (`--threads` above 1 is warning `W0109`),
  so a breakpoint stops the whole simulation at a defined statement, and a session is the same
  at any `--threads`. Why: a stop inside a process is mid-delta (ir.md 3.4); parallel workers
  cannot all stop at one point without a kernel barrier the interpreter does not need otherwise.
- **DAP threads are processes**, frames are VHDL subprogram calls, and the variables of a frame
  are its locals plus the signals of the process's enclosing regions and the generics of its
  instance. Why: the plan's debugging list (call stack and locals per process).
- Details of ids, request routing and pause handling: `crates/yavs-dap/AGENTS.md` section 3.
