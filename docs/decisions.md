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
- **How:** one mechanical sweep (the script, `tools/rename-yavs.sh`, was removed after the rename; see git history; other worktrees ran it on
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
- **IEEE 1076.4 VITAL packages are built in, unmodified** (user, 2026-10-19, #118: "do as sim2 and
  sim1"; supersedes reading them in place). `ieee.vital_timing`, `vital_primitives` and `vital_memory`
  (VITAL 2000) live in `lib/ieee/{2008,2019}/vital_*` byte for byte from the IEEE text (origin,
  cross-check and licence in `lib/SOURCES.md`, `lib/LICENSE-IEEE-VITAL`, shipped in the release
  archive). They run as plain VHDL: functional, zero-delay use (no SDF back-annotation). Their pure
  functions call `NOW` (`GlitchMinTime`, ...), which LRM 4.3 forbids: in units of `ieee` named
  `vital_*` a call of `std.standard.now` is exempt from E1830, user code is not. The exemption is
  exactly that call: reads of signals, variables and files and calls of other impure functions in
  a vital pure function are still E1830. The same units are also exempt from the 4.2.1 rule that
  a pure function's result is not of an access type and has no access subelement
  (`VitalTimingDataInit`), which user code gets (E1830, L95). `tools/update-ieee-libs.sh` preserves them.
- **The original Synopsys packages are built in, unmodified** (user, 2026-10-19, #117; supersedes the
  clean-room plan deferred to M4b, and the "do not reproduce" rule). `ieee.std_logic_arith`,
  `std_logic_unsigned`, `std_logic_signed` and `std_logic_misc` (2008 text for both revisions) are the
  original files, byte for byte with their copyright header, always visible in `ieee` with no flag
  (as in the reference simulator), so `use ieee.std_logic_arith.all` just works. Package and body of
  the first three are one upstream file; they are cut at the line between the two units for the
  loader's `p.vhdl` / `p-body.vhdl` convention. `std_logic_textio` is not needed (2008 moved its
  procedures into `std_logic_1164`). Origin, cross-check and licence: `lib/SOURCES.md`,
  `lib/LICENSE-SYNOPSYS` (shipped in the release archive). The unmodified text passes yavs-sem
  without an exemption. Black-box testbenches: `tests/corpus/synopsys/` (reference: the reference
  simulator, whose own copy of the package differs in warning texts).
- **No VHPI import library** (user, 2026-10-18). The release builds and ships no `yavs.lib`; a VHPI plugin brings its own
  import library for `yavs.exe`'s exports (Python testbench frameworks ship `.def` files per simulator). #124 is
  reduced to checking that the exports are there.
- **Release notes with towncrier** (user, 2026-10-18, "for easier parallel work"), with fragments in `release_notes/news.d/` (AGENTS.md rule 25); a release runs
  `uvx towncrier build --config release_notes/towncrier.toml --version X.Y.Z --yes` into `release_notes/unreleased.md` and renames it
  `release_notes/X.Y.Z.md` in the version-bump commit. The GitHub release notes are that file.
  Fragments describe features, not numbers.
- **No document names other projects** (user, 2026-10-18: "it is not related to this project, this is
  just a simulator"; "avoid mentioning other projects in the docs, except in license notes"; widened
  the same day to the developer docs: "clean the developer docs too"). Applies to every document in
  the repository: README, `docs/*.rst`, release notes, plan, decisions, design documents, CLI
  contract, `docs/STATUS.md`, every `AGENTS.md`, and tool and test docs. Use neutral terms ("reference
  simulators", "other simulators", "external test runners", "Python testbench frameworks").
  Exceptions: licence notes, and commands, flags, environment variables, file and directory names,
  crate and dependency names, code identifiers and corpus identifiers, which stay as they are.
- **When to release** (delegated by the user, 2026-10-18: "decide when to cut new releases.
  often is not bad"): the orchestrator cuts a release whenever main is green (CI and docs) and
  `release_notes/news.d/` holds at least one `feature`, `bugfix` or `breaking` fragment,
  checked each time a batch of agent work has landed. While the version is 0.x: a `breaking` or
  `feature` fragment bumps the minor version, only fixes bump the patch; every 0.x release is a
  GitHub pre-release. Steps: set `[workspace.package] version`, `cargo build` (Cargo.lock),
  `uvx towncrier build --config release_notes/towncrier.toml --version X.Y.Z --yes`, rename `release_notes/unreleased.md` to
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
- **Variables (#305, #348, #321):** opt in with `--wave-variables`. Process variables sit in the
  process's scope, non-protected shared variables in their region or package (a package
  without an elaborated scope gets `lib.pkg` scopes). Read once per time step (`NextTimeStep`,
  `EndOfSim`), when every process is suspended, through the observer accessors of #309
  (`Hook::with_process`, `InterpProcess::var`, `yavs_interp::shared_var`); nothing runs at a
  suspension, so a run without the option pays nothing, and the file does not depend on
  `--threads`. A variable is first compared in place with its last raw value
  (`InterpProcess::var_is`; a composite in a heap object whose write count, `Heap::stamp`, is
  unchanged is not compared), and a changed one is read from its raw bytes at the places
  `yavs_elab::leaf_locs` finds once per variable, only where the bytes differ from the value
  before (#418). A sub-element is written when it differs from its last written value. Shapes are the elaborated ones
  (`Elaborated::var_shapes`, computed with the signals' `Model::shape` in the declaring
  region), so generic-dependent bounds and integer widths match signals'. A shape whose
  evaluation would call a function with side effects on package state is not computed (the
  call is undone: the process makes it itself when it starts, #251) and the variable is left
  out with `W0802` (user, 2026-10-19, confirmed: a variable whose bounds need a state-changing
  function call during elaboration is left out of the waveform). Protected-type shared variables are not recorded: the accessor exposes no
  protected object state. A process that ends keeps the values of its last suspension (its
  frame is gone). Reading a serial process's variables holds the shared state: its composite
  variables live in the shared heap.
- **Equal values are one chain (#410):** `--wave` declares a signal that has the same kernel
  elements, conversion, type and class as one already declared (a port collapsed into its actual,
  at any depth of the hierarchy) as an FST alias of it, with its own name, direction, enum table
  reference and type metadata: the recorder reads, converts and writes the value once. A viewer
  or reader sees both names with identical changes; the file is smaller (neorv32: 44 percent).
  The live stream of `--dap` keeps one id per name. The writer reads a vector straight from the
  kernel's planes (`Hook::view`, eight elements per step), and the kernel hands an observer all
  changes of a cycle in one call (`Observer::value_changes`).
  Value chains are LZ4-compressed (until then they were stored raw; the hierarchy block was
  LZ4 already, so a viewer needs it anyway): neorv32 50 percent, a 512-bit bus testbench 62
  percent smaller, 0 to 6 percent more instructions with waves. Read-back is identical.
- **Durability:** a block is flushed on the next time advance after `block_change_limit` changes
  (and after `flush_interval` of wall clock, off by default, see below), and the header is
  patched each time, so a killed run is readable up to the last block. SIGINT/SIGTERM only set a flag (`interrupt::install`); the kernel
  stops between time steps and calls `finish`, which is also done on drop. With `--dap` the
  recorder also flushes (`Recorder::sync`, through the `yavs_dap::Sync` hook) before every
  `stopped` event, so a viewer reading the file at a stop sees every completed time step; the
  step in progress at a stop inside a process appears when it ends (#423).
  Reproducible by default (user decision 2026-10-24, #666): `WaveConfig::flush_interval` is
  `None`, so blocks end by `block_change_limit` only and the bytes depend on the recorded
  changes alone. A wall-clock interval would split blocks by run speed (a cold `--tier=auto`
  run and a warm one, or two interpreter runs, would differ). `YAVS_WAVE_FLUSH=5s` or `500ms`
  opts in to the clock for durability of a killed run (the bytes then depend on speed);
  `never` is the default spelled out; any other value is E0801. A run that is stopped (SIGINT,
  SIGTERM, `--stop-time`) still finishes the file; only a run killed with SIGKILL or a crash
  loses the changes since the last block.

## Packed logic vectors: bit-planes (yavs-logic)

- **Choice:** `LogicVec` is stored as four bit-planes (`Vec<[u64; 4]>`, 64 elements per block),
  not 4-bit nibbles. The nibble layout stays behind `--features nibbles` since it costs about
  100 lines and shares all tests and benchmarks.
- **Why:** equal speed up to 32 elements, then 8x to 10x faster logic ops and resolution, and
  40x to 140x faster numeric add/compare at 1024+ elements (a 0/1 vector is plain `u64` words).
  Memory is the same 4 bits per element. Full table in `crates/yavs-logic/LAYOUT.md`.
- **Costs:** `to_string` is 2x slower (per-element decode); a 1-element vector takes 32 bytes, so
  the kernel keeps scalar signals as bare `StdULogic`. VHPI/FST need a transpose per element.

## Wide plane kernels: portable vector code, no per-CPU variants (yavs-logic, #199)

- **Choice:** the hot loops over plane blocks live in `yavs_logic::wide` as functions over block
  slices and element offsets (copy, equality, `is_x`, `?=`, many-source resolution); the kernel's
  `Buf` and the ieee natives call them. Resolution folds all sources into class masks and
  resolves once instead of resolving pairwise. Whole blocks are handled four at a time with the
  planes held apart, one lane per block (`Lane`/`L4`), written as plain lane-wise loops that the
  compiler vectorises at the baseline target. No `unsafe`, no `std::arch`, no runtime CPU
  detection.
- **Why:** an AVX2 build of the same four-block loops measured no faster than the baseline one
  (the plane transpose of the block layout, not the arithmetic, is the cost, and a run that
  does not fit in cache is bound by memory), and the simple gates (`and`, `or`, `xor`, `not`)
  are slower with the transpose than as scalar words. A hand-written intrinsics version would need
  `unsafe` and a build per CPU for a gain the measurements do not show. Revisit if the block
  layout changes to planes stored contiguously per signal (no transpose), where 256-bit loads
  would apply directly.
- **Rules kept:** results are bit-identical to the scalar tables (`crates/yavs-logic/tests/wide.rs`
  checks every kernel against a per-element model for random planes over all nine values, odd
  lengths and every offset); one driver is the identity, not a resolution (`-` stays `-`).

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
- **Loaded on first use (Linux x86_64 and aarch64, glibc):** the executable does not depend on
  libLLVM. `crates/yavs-llvm/build.rs` lists the `LLVM*` functions the shared library exports and
  writes one stub per function (a jump through a table); the stubs satisfy `llvm-sys`'s
  references, so the linker leaves the library out of the dependencies, and `lazy::ensure`
  (called by `Module::new`, `Jit::new`, `compile`, `AutoTier::start`) opens it and fills the table
  once. Why: the dynamic loader relocated the whole library (174 000 relative relocations, 1 650
  page faults) in every process, 5 ms of CPU for any command, 4 times the rest of a short
  `yavs run`, although only `--tier=llvm|auto` uses it (#426). A library that cannot be opened
  is an LLVM `Error` at the first use instead of a failure to start. Other targets link as before.
- **Size threshold (rule 5):** codegen time grows faster than linearly with function size (about
  0.15 s at 10k IR instructions, 3.8 s at 100k, 28 s at 300k at O2; numbers in docs/STATUS.md).
  Decided (user, 2026-10-19, #126): compile only functions that become hot (`--tier=auto`), and
  leave a function of more than about 50k IR instructions to the interpreter unless
  `--tier-max-size=N` raises the cap (default 50000, counted in yavs IR instructions, applied by
  `--tier=llvm` too). Why: hot code is what pays back compile time, and one huge function would
  keep the single compile thread busy for seconds.

## LLVM tier: side exits to the interpreter (yavs-llvm, yavs-interp)

- **Shape:** one LLVM function per IR function, `fn(env, regs, mem, pc) -> u32`, running on the
  registers of the interpreter's own frame. It enters at `pc` (block starts and the instruction
  after each call or suspension), executes register arithmetic, compares, checks, address and
  signal-reference arithmetic, memory accesses and branches itself, and executes every other
  instruction in place through the `yavs_rt_step` callback (`TierHost::step`, the interpreter's
  `step`): calls, natives, foreign calls, reports, signal operations, waits and the slow paths.
  It returns only at `Ret`, at `Suspend` and at an inline instruction that fails (the
  interpreter executes those). `yavs_interp::Tier` is the seam; the interpreter calls it from
  its fast loop only, so a debugger keeps a process interpreted.
- **Self-contained (user decision, 2026-10-20; #336, #337):** compiled code no longer returns to
  the interpreter at calls, natives, reports, signal reads and waits. A call the callback
  executes pushes the callee's frame with the interpreter's own code (`call_regs`, `push_frame`:
  static link, depth limit, temporary base, register prefill, arguments); when the callee has
  compiled code (the per-function slot table, `Tier::run`'s lookup) the callback runs it
  directly on that frame, and when it returns the callback pops the frame and writes the result
  (`TierHost::ret`, as `exec_fast` returns) and the caller goes on. Anything else (a callee
  without compiled code, a callee that stops elsewhere, a register stack that moved, more than
  256 KiB of native stack in nested calls) leaves the frames to the interpreter: compiled code
  returns `TIER_UNWIND` through every compiled caller, every frame's pc is the instruction it is
  at, and the interpreter goes on with the top frame (stepping up to the next op when the pc lies
  inside a fused op). A stepped instruction that suspends the process returns `TIER_SUSPEND`.
  Why: the frames stay one implementation and are always materialised, so traces, root maps
  (#422), debugging and errors are unchanged, while a call between compiled functions costs one
  callback instead of two exits and two entries. The frame push itself stays the interpreter's:
  compiled code writing the machine's `Frame` vector would duplicate `push_frame` in LLVM IR.
- **Memory (#334, #335):** `Tier::run` gets a `TierFrame`, a `#[repr(C)]` view of the top frame
  (pointers to its registers and slots, the temporary area with its length and capacity, the
  instance data, the frames one and two static links up, the number of instance signals): the
  machine keeps its stacks and refreshes the view after every step and return (#336 measured
  the swap of the old owned vectors as the largest share of a call). Compiled code keeps the
  temporary area's length in the view and hands it back (`TierHost::temp`) before a step and at
  the end. It loads, stores, copies, allocates temporaries, resets them at `Stmt` (stepped while
  temporaries are recorded for the GC, #422), reads descriptors (`DopeLen`, `Pos`,
  `SubtypeCheck`, `Field`), writes them (`MakeDope`), builds layouts whose element layout is
  static (`LayoutOf`), reads and writes logic elements (`LGet`, `LSet`), takes addresses one
  and two static links up and builds signal references (`SigVal`) itself when the bytes lie in
  the frame, the temporary area, the instance data or an object `TierHost::object` returned (a
  small cache, emptied at every step that may free or resize an object and before a compiled
  callee runs; constants stay); every other address, every bounds failure and every error steps
  the whole instruction in the interpreter, so messages stay identical. A frame access the
  generator can prove within the function's frame size has no check at all.
  Why: the interpreter keeps the one implementation of address spaces, heaps and errors; the
  common accesses cost a compare instead of a callback. Decided by the M6 agent, 2026-10-19.
- **Address lookup (llvmtb, 2026-10-21, #432):** an address that is not a known frame offset or
  a constant compiled code has a copy of goes through one call, `yavs_rt_host`, instead of an
  inline lookup of the frame, temporary and instance data: the inline form was two thirds of
  the generated instructions and made compile time twice as long, and the code ran no faster.
- **Tier-up budget (llvmtb, 2026-10-21, #432):** `--tier=auto` compiles at O2 only (O0 code
  ran no faster than the interpreter on testbench code), and only while its own compile time
  stays within `Heat::budget` (2) percent of the interpreter time the sampler saw, sixteen times
  that for compute loops (a long loop in one process that never suspends is seen only by the
  sampler, and at twice the budget a 440-instruction loop was not compiled before the run
  was over: 1.9 s interpreted against 0.33 s compiled); a function must also have been seen for four times its estimated compile
  time (learned per IR instruction), and its small callees compile with it. Why: a compile
  costs CPU, a hot set of testbench functions gains under 10%, and the requirement is that
  `--tier=auto` never runs more than 2% longer than `--tier=interp`. With the hidden
  `--tier-heat` (tests, tuning) there is no budget. Without any compiled code the fast loop
  samples every 128 events and hands over to the full loop once there is some, so a tier
  that has not compiled costs about 1%. Proposal (user to decide): with these rules `--tier=auto`
  stays within 4% of `--tier=interp` on every benchmark and kernel design measured (geometric
  mean 1.006) and gains 3% to 5% on runs of several seconds, so it could become the default of a
  build with the `llvm` feature; against it, loading the LLVM library costs every process about 11 ms
  and 47 MiB, and the gain on testbenches is small.
- **Tiers:** `yavs run --tier=llvm` compiles every non-elaboration function at O2 before time 0;
  `--tier=auto` compiles hot functions on a background thread (below). Both compiling tiers
  leave functions over `--tier-max-size` (default 50000 IR instructions) interpreted (user,
  2026-10-19, #126).
- **Tier-up (`--tier=auto`, #125, #432):** the compile thread samples, every millisecond, which
  function each simulation thread interprets (one relaxed store per function the fast loop
  enters into a per-thread slot, `Runtime::sampler`) and compiles a function once it was seen
  for its `Heat` (2 ms plus 40 us per IR instruction, about twice its O0 compile time). Time in
  compiled code entered from an interpreted function counts for that caller, which compiled
  would call it in place. ONE background compile thread (single-threaded LLVM, one module per
  function, O0, the JIT made with the first hot function); the entry is published through a
  per-function atomic entry, which the interpreter asks for on entry and every 256 jumps while a
  function has none. Why sampling: the per-op counting it replaces cost 12% to 25% of CPU on
  the call-heavy benchmarks when nothing compiled (it was in every jump, call and return of the
  predecoded loop); a sample costs the compile thread microseconds. O0 stays: O2 codegen costs
  about 7x (75 ms for a 188-instruction process at O2, 9 ms at O0), and IR passes before O0
  codegen gain little. The simulation never waits for compilation, and output does not depend
  on when it finishes (rule 5). At the end of the run the queue is dropped and the files the
  design left open are closed (`Runtime::close_files`); if a function is still being compiled,
  the CLI flushes the standard streams and ends the process at once without exit handlers
  (`exit_now`): LLVM's static destructors must not run under the compile thread, and waiting
  would cost up to one compilation. With `--load` or `--vhpi` libraries the end of the run waits
  for the compilation instead, so their exit handlers run. `--tier-heat=BASE,PER_INSTR,PERIOD`
  (hidden, for tests and tuning): microseconds, nanoseconds per IR instruction, sampling period
  in microseconds; PERIOD 0 compiles every function the interpreter enters. The LLVM agent,
  2026-10-20.
- **Signal operations as direct calls (#339, 2026-10-20):** compiled code runs `SigReadScalar`,
  `Drive`, `WaitOn`, `Suspend` and the `rising_edge`/`falling_edge` natives through
  `TierHost::signal` (`yavs_rt_signal`), which calls the interpreter's own functions for them
  (`sig_read_scalar`, `drive`, `wait_on`, `suspend`, the native's `edge_of`, so the kernel's
  `Ctx::first_code`, `Ctx::edge` and `Ctx::assign_code` paths) without the generic step; a
  `wait on` followed by the suspension is one call, and a suspension no longer returns to the
  interpreter's loop (`TIER_SUSPEND`, the frame's pc at the `Suspend`). An error is recorded with
  the pc of the instruction that failed, as a step records it. The natives of one or two
  `std_ulogic` operands (the `std_logic_1164` gates, `not`, `to_x01`, `to_x01z`, `to_ux01`, `??`)
  are table lookups: `yavs_interp::code_table` computes the table with the natives' own
  functions, and an operand beyond the nine codes steps the call (the native's error). These calls
  skip the collection point a native call is (they allocate nothing; the next `new`, call or
  other native collects). Why: on clocked RTL every activation is a few signal operations, and the
  generic step cost more than the work. `Tier::run` fills compiled code's frame view in place
  (`TierHost::view`) and `tier_go` returns one word: copies of structs the callee had just written
  in narrower pieces stalled on store forwarding. The LLVM agent, 2026-10-20.
- **O2 recompile (#432, 2026-10-20):** the sampling slot also names compiled functions (bit 31);
  a function seen running its first-tier code for 30 times its `Heat` (about ten times its
  O2 compile time) is compiled again at O2 on the same thread, in a second JIT made at O2, and its
  entry (an atomic word per function, code never freed) is replaced; frames running the O0 code
  finish in it. With `--tier-heat=..,..,0` the O2 recompile follows the first compile at once, so
  the tests swap both in mid-run. The LLVM agent, 2026-10-20.
- **Short functions stay interpreted (#432, 2026-10-20):** a function the sampler finds hot that
  has fewer than 64 IR instructions and no jump back is not compiled (`--verbose` says why):
  entering compiled code from the interpreter (frame view, entry switch, the way back) costs
  about what interpreting such a body costs, and the helpers a verification library calls
  millions of times ran slower compiled (`vai_window_gen` 0.93x, `calls` 0.82x of the CPU with
  them compiled). Processes always loop (see "Tier-up of processes" below for the rule now). Eager tier-up (`--tier-heat=..,..,0`, the tests) still
  compiles everything. Compiling such a callee whenever compiled code calls it was measured
  too and lost (`vai_window_gen` 1.18x). The LLVM agent, 2026-10-20.
- **Vector process bodies (#338, #339, 2026-10-20):** compiled code runs `LCopy` itself (the
  plane blocks found through `Gen::host`, at most eight each side, copied by `yavs_rt_lcopy`,
  which calls `yavs_interp::copy_bits`, the function the interpreter's `lcopy` uses; source and
  destination may be one vector, so both are copied to the stack first), reads constants it knows at
  generation time from a private copy in the module (only the `Bytes` form, up to 4 KiB: the
  bytes the interpreter reads; constants never change), and runs `SigRead` and `DriveSlot` of
  one range and `WaitFor` (with its `Suspend`) through `TierHost::signal` (`sig_read_vec`,
  `drive`, `wait_for`); the interpreter's own bodies of these do without vectors (#443). Why: on
  clocked RTL over `std_logic_vector` the per-activation cost was the generic step of these
  instructions, not the arithmetic; one implementation each keeps rule 5 by construction. The other vector natives (numeric
  arithmetic, shifts, resize, conversions) still run through the generic step and build
  vectors. The LLVM agent, 2026-10-20.
- **Tier-up of processes and short callees (#432, 2026-10-20):** `--tier=auto` compiles a hot
  process without a loop of its own (clocked RTL) at O2 at once: its O0 code ran no faster than
  the interpreter, and it runs for the whole simulation, so the second compile is saved; a
  process with loops (a testbench) still starts at O0, which already gains there. A loop is a
  cycle of blocks (`loops`, which replaces the test for a jump to an earlier instruction: join
  blocks laid out before their branches looked like loops), and a process's way through its
  suspension is none, so a one-statement process stays interpreted under the short-function
  rule (the kernel designs `fan`, `chain`, where its compiled code was slower); a short hot
  function stays interpreted only while no compiled function calls it directly: once one does
  (or one that calls it is compiled later) it is compiled at O2, since each call from compiled
  code to an interpreted function leaves the caller and enters it again (`"xor"` of
  `numeric_std` in a reduction loop). The LLVM agent, 2026-10-20.
- **Default:** `--tier=interp`, also in a yavs built with the `llvm` feature. After #336, #337
  and the sampling tier-up, `--tier=auto` is 0.77x to 1.36x the interpreter's CPU on the shared
  benchmarks (geometric mean 1.016, docs/STATUS.md "llvm"): it wins on compute and verification
  component code (`uvvm_axistream_vvc_tb` 0.77x, `compute_loop` 0.82x) and loses where compiled
  code steps often or only compile time is added (`vai_window_gen` 1.36x, `ff_rtl` 1.10x, the
  RTL designs of many short process activations). It becomes the default only when measured
  never slower (the rule the user set for #125). The LLVM agent, 2026-10-20.
- Decided by the M6 agent, 2026-10-19; **accepted by the user (user, 2026-10-19)**: compiled
  code runs native operations and hands slow or erroring instructions back to the interpreter.

## Kernel crate and process interface (yavs-kernel)

- **Crate:** the LRM 14.7 cycle lives in `crates/yavs-kernel`, independent of the IR. yavs-rt
  (heap, std natives) builds on it.
- **Process interface:** `Process::run(&mut Ctx) -> Result<(), SimError>` runs to the next
  `wait`; the wait is stated through `Ctx::wait_on/wait_for/wait_at`. A `wait until` condition is
  evaluated by the process, which re-suspends with its original absolute deadline.
- **Event tracking:** per-element flat arrays (`event_cycle`, `last_event`, `last_value`, ...)
  over the whole signal, with `Range` operations; inertial preemption is decided per element and
  transactions are split. This is how rule 2 is read: no per-scalar objects, exact per-scalar
  semantics. `'last_value` of an array part is the value of the part before its last event (LRM
  16.2.4): elements that did not change in that event keep their current value (#451; the
  elements of one leaf, a record's leaves are read leaf by leaf).
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
  `StartOfProcesses` and `EndOfProcesses` around every cycle's processes (all three references
  with VHPI accept end of processes; sim1 and sim3 also fire it once after initialization's
  process run, at cycle 0, sim4 does not: rule 15, majority of sim1/sim3/sim4, so yavs fires it
  there too, before `EndOfInit`; registering start of processes is accepted by sim4 only (sim1 and sim3
  refuse the registration) and stays accepted, user, 2026-10-19 ("keep it permissive": the
  rule 15 majority applies to run-time semantics, not to accepting a registration); it never fires after initialization in sim4 or yavs) and `StartOfPostponed` (read-only,
  only when postponed processes run, as the one reference that accepts it fires it). IEEE VHPI has no read-write or
  read-only synch reason: the read-write phase is `vhpiCb[Rep]LastKnownDeltaCycle`
  (`Phase::LastKnownDelta`, puts there make one more delta) and the read-only phase
  `vhpiCb[Rep]EndOfTimeStep` (`Phase::EndOfTimeStep`, after postponed processes), which is what
  Python testbench frameworks register and what the reference simulator fires; the earlier plan
  of a wake-up at `now` for read-write was dropped because it always costs one extra delta and
  runs before the deltas still due at this time have settled. VHPI times count in units of `--resolution`. Disabled
  callbacks are kept out of the per-phase sets, so they cost nothing per cycle (#123, plan rule 9).
  Both phases also run at time 0 right after initialization when no delta follows it, so a
  write a Python testbench framework queues for its read-write phase at time 0 lands at time 0:
  the two references that fire these reasons do so (the third fires neither at time 0; agent,
  2026-10-19, #347, `tools/vhpi-conformance` plugin `tport`).
  Full rules: `crates/yavs-vhpi/AGENTS.md` section 3.
- **Puts and value changes (#271, #272):** `vhpi_put_value` goes through the kernel's observer
  forces and deposits, queued until the callback returns, and lands in the next delta: puts from callbacks before
  the update (after delay, next time step, start of cycle) are handed to the kernel at the
  start-of-processes phase of the same cycle, so they make a new delta as both references with
  VHPI do (#319). Identical at any thread count. An integer (`vhpiIntVal`) put into a logic
  vector is its two's complement with the rightmost element least significant (rule 15: all
  three references with VHPI were run (`tools/vhpi-conformance` plugin `intvec`). sim1 and sim4
  accept the put and disagree on the bits (sim1 stores the integer as an enumeration position in
  the leftmost element only, giving non-logic characters for most values), sim3 refuses it for
  every value; acceptance is the majority. The bit reading is a 1-1 tie that sim3 does not
  break, so it stays the numeric reading, which the VHPI text does not rule out; a value that
  does not fit is refused, as no two references agree on it. Agent, 2026-10-19, flagged to the
  user). `vhpiStrVal` text for an array fills it from the left: longer text is cut and
  shorter text padded with NUL, as two of the three references with VHPI do (the third refuses
  longer text and leaves the rest unchanged; agent, 2026-10-19, #345, `tools/vhpi-conformance`
  plugin `strput`). The non-propagating modes `vhpiDeposit` and `vhpiForce` on a
  signal are misuse (`W0903`, return 1): two of the three references with VHPI refuse them, the
  third changes the value with no event and no delta (user, 2026-10-19: check that reference,
  then follow the majority; `tools/vhpi-conformance` plugin `nonprop`). A deposit sets the driving value until a source next updates those elements;
  a release recomputes the driving value from the sources, so a force and release after a
  deposit return to the drivers' value, as both references with VHPI do (#310; a signal
  without sources keeps the deposit). Value-change callbacks repeat until
  removed and run once per cycle. One registered (or enabled) while a cycle's value changes
  are dispatched, e.g. a framework re-arming its callback from inside it, is *fresh*. The
  references with VHPI (all three, #346, `tools/vhpi-conformance` plugin `vecedge`) never show
  a fresh callback any change of the cycle, so a framework waiting repeatedly for the first of
  several triggers that fire together loses the later triggers' edges (cocotb gh-5112). yavs
  does, and the result does not depend on the order the kernel reports the signals in (user,
  2026-10-21, #496, confirmed: a deliberate difference from all three, catalogue rows
  `vecedge` and `lateedge`). The rule: (1) the kernel reports a cycle's changed signals in
  signal order and a vector's changed runs one by one; a callback runs at most once per cycle.
  (2) A callback not fresh sees the change of the cycle when its signal is reported, as
  before. (3) A fresh callback sees a change of the cycle once, however the report order
  falls: a signal reported after its registration is delivered at the report, one reported
  before it right after the callback batch that registered it (`Callbacks::late`, repeated
  until no new one is due). (4) Not a fresh trigger but a re-arm, and not delivered: a fresh
  callback watching exactly the same elements as a callback already delivered a change in the
  cycle (`vc_served`; the identity of a trigger is its watch set). So a callback re-armed on
  the signal or vector being reported sees neither its own triggering change (#346) nor the
  later runs of the vector, and a repeated first-of-two re-arm cannot loop: each watch set is
  served once per cycle. A fresh callback on other elements (another signal, a bit of the
  vector) is a new trigger and sees the cycle's changes of those elements. (5) Callbacks of
  one batch watching the same elements all get the change (the set is updated after the
  batch). (6) Only registrations made while value changes are dispatched are fresh; one made
  in a read-write, read-only, timed or any other callback sees nothing of the cycle's changes
  and everything from the next cycle on, as in the references. (7) Determinism: dispatch runs
  in the kernel's serial value-change bus, in signal order, so the order of delivery is the
  same at any `--threads`. Known limit: two independent fresh callbacks watching the same
  elements registered in different batches after those elements were reported are told apart
  from a re-arm only by the batch, so in some report orders the later one is not delivered (a
  trigger has no other identity at the C interface).
  A lookup that finds nothing (`vhpi_handle_by_name`) is only reported through
  `vhpi_check_error`, since Python testbench frameworks probes names routinely. Codes `E09xx`/`W09xx` are reserved
  for VHPI.
- **Variables (#309):** observers reach a suspended process through `Hook::with_process` and
  `Process::as_any` (the kernel stays ignorant of the interpreter; another process tier
  implements `as_any` for its own state); the interpreter reads and writes the slot of a
  process variable by name in the process's frame, and a shared variable at its slot (`Built::vars`): a
  region's address is what its instance cell holds once its thunk ran, a package's comes from
  a slot thunk (`Lowerer::var_slots`); no thunk runs only for VHPI, so nothing changes for a
  design without plugins; a region variable no process uses gets its slot when the design is built (#342). Values
  cross as elaborated values (`ConstValue`, the IR memory codec of yavs-elab), so variables of
  any composite type share one path. Only from simulation callbacks: every process is
  suspended there, so reads and writes are the same at any `--threads`, and VHPI calls from a
  foreign subprogram (a process running, the shared state taken) never touch it. A deposit is
  written at once rather than queued like signal puts: a variable has no driver or delta, and
  the one reference simulator that exposes variables through VHPI also writes at once (the
  reference simulator with VHPI used elsewhere here finds no handle for them). Forces and releases of
  variables are `W0903` (that simulator treats a force as a deposit and has no release); the
  read-only phases refuse writes as for signals. A process variable is readable from the
  start of simulation (#342, below). Array bounds of an object with a constrained subtype come
  from the subtype, not from its initial value (`x"00"` of a `(7 downto 0)` variable).
- **Variables before their process ran (#342):** a process whose variables VHPI touches before
  its first run has its declarative part run then (`InterpProcess::prestart`: the entry stops at
  the block `Func::body` names, where the statements start) and continues with its statements at
  the first run, so a variable holds its initial value from the start of simulation and a
  deposit before the first run is seen by it. It runs outside the kernel: a declarative part
  that needs the running simulation (a signal value) is `W0903` with its reason, and the process
  stays unstarted. Only processes VHPI touches are affected, at any thread count (observer
  calls run with every process suspended). Reference simulators: both return the initial values
  at the start of simulation; one has run the processes' first cycle by then (a deposit comes
  after it), the other shows a deposit at the first run, which yavs does (user rule: majority of
  those that run it; `tools/vhpi-conformance` `vars`). The reference without variable handles
  gives no vote.
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
  `crates/yavs-std/golden/sim1-deviations.txt`: extended identifiers keep case and doubled
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
  other operators, a PSL built-in call other than the ones below (`ended`, `nondet`, `nondet_vector`, `next`), a name of a skipped `property`/
  `sequence`, anything left after the property) is parsed again as before and skipped with
  `W0004`; the attempt's diagnostics are dropped. `-- psl` comments are read only with the option
  `--psl` (`parse`, `analyze`, `elab`, `run`; user, 2026-10-19, as the references need one; the
  first version read them by default; native statements are always read) when the text after `psl` starts with an optional `label :` and a PSL
  directive or declaration keyword and reaches a `;` before a line that is not a comment or is
  another `-- psl` comment; the following comment lines continue the statement up to its `;`.
  Prose such as `-- PSL assert failed` (in the 2019 `std.env` source) stays a comment. `->` is
  lexed as one token (never valid VHDL). yavs is permissive where the references reject: `@`
  clocks, `assume` and `severity` (catalogue rows `psl-*`).
- **PSL built-in functions in the subset (agent, 2026-10-20, #445):** `prev(x[, n])`, `stable(x)`,
  `rose(x)`, `fell(x)`, `isunknown(x)`, `countones(x)`, `onehot(x)` and `onehot0(x)` work inside
  the expressions of the supported directives (`assert`/`assume`/`cover`, both sides of `->`). The
  parser rewrites each call into variables of the checker process (`parser/psl.rs`, `Gen`),
  set from the values at the clock event before the check and updated after it: `prev(x, n)` is
  a chain of `n` variables of `x'subtype`, `rose`/`fell` keep whether the last value was true or
  false (a value that is neither, such as `'X'`, is no edge), `countones` is a loop over `x'range`
  and counts the true elements, `isunknown` is `ieee.std_logic_1164.is_x`. At the first tick
  `prev(x)` is `x` (so no edge and `stable` is true; two references agree). Ceilings: `prev` and
  `stable` need an object name, `countones`/`onehot`/`onehot0` a name of a one-dimensional array,
  `isunknown` a std_logic-based argument and `prev(x, n)` a literal `n` of at most 256; any other
  argument skips the directive with `W0004` as before. `ended`, `nondet`, `nondet_vector` and the
  `next` function stay skipped. Catalogue row `psl-edge-unknown-value`.
- **PSL sequences in the checker subset (agent, 2026-10-23, #445, L123):** braced sequences (SEREs)
  run in `cover {s}`, `assert never {s}`, `assert [always] {s}`, `assert [always] {l} |-> r` and
  `{l} |=> r` (`r` a boolean, `{s}`, or either behind `next`/`next[n]`), also with `assume` and in
  parentheses. Operators: `;`, `:` (fusion), `|`, `&&`, `&`, `within`, `[*]`, `[+]`, `[*n]`,
  `[*m to n]`, `[*m to inf]`, `[=n]` and `[->n]` (with ranges) on booleans; mixing `|`, `&`, `&&`
  and `within` at one level needs braces (the LRM precedence table was not at hand). The parser
  (`parser/psl_seq.rs`) turns a sequence into a position automaton: one boolean process variable
  per position of the sequence (copies for a count), updated at each clock event, so the output
  is the same at any thread count and nothing changes in sem, lowering or the kernel. The left
  side of `|->`/`|=>`, `cover` and `never` ask "does a match end now", which the union of the
  matches in progress answers. The right side is an obligation that fails when no match is left:
  it is made deterministic at parse time (subset construction over the minterms of its booleans;
  at most 6 distinct booleans, 64 states, 256 positions, counts up to 64, else the directive is
  skipped with `W0004`) and the process keeps one variable per state, so the failure of one start
  is told from the others alive. A failed obligation reports at the tick where it can no longer
  match, once per tick however many starts fail. A boolean used twice is evaluated once per tick.
  Without `always` the directive starts at the first clock tick only. A sequence of one boolean,
  and `{a} |-> b`, `{a} |=> b`, keep the cheaper process of the simple subset. Empty matches (a
  sequence that can match the empty word) are ignored, except that a right side that is a star
  from zero (`{b[*]}`) holds at once, as in both references. Fusion needs non-empty operands. A `[*n]` that starts a sequence is `true[*n]`, and a boolean takes the place of a sequence of one before `|->`/`|=>` and, with a repetition, without braces (`never a[*3]`, `cover (a = '1')[->2]`).
  Not in this step: `ended`, strong sequences (`{s}!`), `eventually!`, `until`, `before`, `abort`,
  `next_e` and relatives, named sequences/properties, local variables, `forall`, units. Catalogue
  rows `psl-sere-*`; `tests/corpus/psl/psl_sequences{,_ops}.vhd`.
- **`default clock` among the concurrent statements (agent, 2026-10-23, #445):** accepted like the
  declaration in the declarative part, and it holds for the directives after it in the same
  region. Permissive (the LRM places it with the declarations): the first reference simulator's
  own PSL regression tests write it after `begin`.
- **Named PSL sequences and properties (agent, 2026-10-23, #445):** `sequence s [(formals)] is
  body;` and `property p [(formals)] is body;` in a declarative part or as a statement run when
  used. A declaration is remembered as the range of its body tokens and a use parses that range
  again with the formals bound (`parser/psl_named.rs`), so nothing is copied. Formals: `boolean`
  (the actual is evaluated at each clock tick into a process variable the body's name is renamed
  to), `const` (a number: a count, or a literal in an expression), and `sequence` and `property`
  (the actual's tokens, parsed where the formal is used). A typed formal (`bit`, `numeric`, ...),
  a built-in function on a formal, a use before the declaration, the wrong
  number of actuals, a recursive declaration, and more than 200 expansions in one directive skip
  the directive with `W0004`; a declared name outside a sequence or whole-property position
  does too. A declared property is used as the whole property of `assert`/`assume`
  (`assert p(a, b) @clk report r;`), a declared sequence in `cover`, as an operand of a suffix
  implication, or inside sequences. A declaration is visible in its design unit.
- **Temporal PSL properties in the checker subset (agent, 2026-10-23, #445, L133):** `next_a`,
  `next_e`, `next_event`, `next_event_a`, `next_event_e`, `until`, `until_`, `before`, `before_`,
  `sync_abort`, `abort`, `async_abort`, `<->`, `iff`, `and`/`or` of properties, nested `next` and `always`, and any of them
  as the right side of `|->`/`|=>` (`parser/psl_prop.rs`). A directive that has one of these words,
  or that the simple parsers cannot read, goes to this parser. The property becomes terms (formula
  progression over finite traces, weak: an obligation open at the end of the run is no failure);
  at parse time every term reachable over the values of the booleans it tests is enumerated and
  the process keeps one variable per term (at most 64 terms, 6 booleans per term, 4096 steps, else
  the directive is skipped with `W0004`). Conjunctions are separate obligations: each conjunct
  that fails is reported at its own tick, and the failures of all starts at one tick are one
  report, as in the second reference simulator (`next_a[1 to 3] b` with `b` never true fails at
  three ticks). `next_event(c)(p)` counts the current cycle (the second reference simulator).
  `until` stops at its first failure (one report); the second reference simulator goes on
  reporting. Not run, skipped with `W0004`: the strong forms (`eventually!`, `next!`, `until!`,
  `before!`, `next_a!`, `{s}!`), which need a check when the run ends (user, 2026-10-23: they
  stay skipped). `abort` and `async_abort` run exactly as `sync_abort` (user, 2026-10-23, a
  deliberate deviation from the LRM, whose abort is asynchronous and reacts between clock
  ticks; the reference simulators do abort asynchronously for a condition that changes between ticks, which the decision had not assumed, see #636). Catalogue rows
  `psl-temporal-operators` and `psl-abort`; `tests/corpus/psl/psl_abort.vhd`,
  `tests/corpus/psl/psl_temporal{,_ext}.vhd`.
- **Formal-only PSL is skipped without `W0004`** (user, 2026-10-23, #445): `restrict`,
  `restrict_guarantee`, `assume_guarantee`, `fairness`, `strong fairness` and any directive that calls `nondet` or `nondet_vector` have
  nothing to check in a simulation, so the parser skips them silently (`psl_formal_only` in
  `parser/psl.rs`; the skip is the one of an unsupported directive, only the warning is left
  out). The strong operators and the verification
  units (`vunit`, `vmode`, `vprop`) stay skipped with `W0004`. Test `tests/cli/psl_formal_only`
  and the unit test `psl_formal_only_directives_are_skipped_silently`.
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

## VHDL-1993 mode (`--std=93`; yavs-syntax, yavs-sem, lib/, CLI; #629, user, 2026-10-23)

The user decided to add a VHDL-93 mode so GRLIB (VHDL-93 code) runs unmodified. The design, with the references' verdicts per
difference, is `docs/design/vhdl-93.md`.

- **Scope**: `--std=93` (alias `1993`) only. `--std=87` and `02` are not added (no corpus needs them; 87 would be the 93 mode
  with a smaller keyword table). The test runner never passes `93` (it refuses 93 projects itself).
- **One analyzer, `Std::Vhdl1993` below 2008**: keyword table, lexer (no `?` delimiters, `<<`/`>>`, `/* */`, sized or based bit
  strings) and `E0011` for the 2008 statements that are not 93 (`process (all)`, conditional and selected assignments in a
  process, `elsif`/`else`/`case` generate, package generics and instantiation). The `since` baseline of sem declarations is
  `Vhdl1993` (it was 2008); implicit operations 2008 added (`MINIMUM`/`MAXIMUM`, `TO_STRING`, matching operators, `??`,
  reductions, array-scalar logical operators, `RISING_EDGE` of `bit`/`boolean`, file `FLUSH`) are `since: Vhdl2008`, so a 93 design
  may declare its own `to_string` or `maximum`.
- **Libraries**: `lib/std/1993` (derived from 2008 by deletion) and `lib/ieee/1993` (vendored: `std_logic_1164` with its own
  `std_logic_vector` type, the Synopsys `std_logic_textio`, the 1993 `std_logic_misc`, the unchanged math, Synopsys and VITAL
  packages; `numeric_std`/`numeric_bit` are derived from the Apache-2.0 2008 files, user 2026-10-23, #650: the 1995 IEEE 1076.3
  texts forbid sale or distribution for profit and are not shipped). Licences and every edit in `lib/SOURCES.md`.
- **Mixing** follows the 2008/2019 rule: design revision = the newest standard in the closure, each file keeps its own revision for
  parsing and gates. A 93 file in a 2008 design sees the 2008 packages (so overloads on both vector types are homographs there) and
  does not see what 2008 added (`E1007` for `to_string`, a 2008-only unit). The reference simulators refuse to mix standards;
  yavs keeps one rule for all revisions and never refuses a mixed design (user, 2026-10-23).
- **Shared variables** of non-protected types are legal in a 93 file (no `E1904`/`W1904`).
- **Homographs**: an explicit declaration hides an implicit one of a use clause in every mode (the references need `--relaxed` /
  `-fexplicit` for this at 1993; GRLIB passes both).
- **Not enforced** (rule 2): the 2008 relaxations (reading `out` ports, case and static rules) are accepted in a 93 file, since valid
  93 code never depends on them being errors.
- **Natives**: the `std_logic_1164` natives keyed by `std_ulogic_vector` serve `std_logic_vector` too (`yavs-ir` `ieee.rs`): both are
  arrays of `std_ulogic`.

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
- **A suspended process owns no registers or temporaries** (#409, yavs-interp). The IR keeps no
  register live across a suspension (ir.md 1.2) and temporaries are per statement, so at a
  suspension in the root frame the process hands its register stack and temporary area to a
  per-thread scratch and takes the scratch's back before it runs again.
  Memory is then proportional to the threads, not to the processes (a design with 20000
  small processes dropped 45 MB). A debugged process keeps its own (the debugger reads frames
  between steps) and so does a process waiting inside a procedure (its callers' frames are
  below the root).
- **Frames are slices of two stacks, not vectors of their own** (#438, yavs-interp). All
  frames' registers are one stack and their slots another (frame 0's, the process variables, in
  a vector of their own so a suspended process keeps nothing else); a frame holds base offsets.
  A call allocates nothing, fills the callee's registers from the function's initial image and
  copies the arguments from the caller's registers into the callee's slots. Frame addresses
  name the frame index, never a pointer, so the stacks may grow. A compiled function sees the
  top frame in the `TierFrame`, which carries the whole stack with offsets; its access to a
  frame between the root and the top is stepped by the interpreter.
- **Heap addresses carry no generation; deallocated slots are quarantined instead** (#81,
  yavs-interp). The address format has no room for a generation next to a 24-bit slot index
  and a 32-bit offset, so a deallocated slot is not reused until its process suspends
  (registers are dead then) and no address in the process's `RootKind::Addr` slots and
  recorded temporaries points into it (precise since #422; before, any aligned word of the
  frames and temporaries that decoded to an address counted). The same addresses keep such
  objects alive across a collection (a by-reference parameter of a waiting procedure). An access through a stale address
  therefore always meets the freed slot and is the runtime error `access to a deallocated
  object`. For the shared heap the addresses of every serial process's frames count, as each
  published them at its last suspension (#303). A process that runs long without a wait
  also recycles inside its activation (#355): before a native call or a `deallocate`, once
  at least 4096 slots (and twice what the last recycle kept) wait, with the same addresses
  plus, live between instructions, its `Ptr` registers (#422). Rationale: a loop
  that writes millions of textio lines kept every freed slot until it ended. Deterministic:
  it depends only on the process's own instruction stream (the shared heap's other users,
  serial processes, are suspended then).
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
- **W1501 (unused generic)** applies to the generics of packages and subprograms, reported once
  the unit is complete: after the package body, after the package declaration when the library
  holds no body for it, after a generic subprogram body. Entity, component and block generics are
  not checked (they are used across units).
- **Generic protected types (2019):** analyzed with opaque generics; an object of one is `E1505`.
  `type t is new g generic map (...)` (5.6.2) instantiates it like a generic package: the
  declarative region is cloned at the declaration, the body (also one in a package body) with
  the other instance bodies at elaboration (#29). A generic map in the protected type header
  stays not implemented (a not-implemented diagnostic).
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
  with no binding indication): accepted, the default binding (7.3.3) supplies the entity (user,
  2026-10-19, rule 15: accept what the LRM does not clearly forbid; it replaces the morning's
  majority-rule `E1704`). LRM 3.4.3 does not say whether a default binding counts for a block
  configuration. With no entity to bind at all it stays `E1704`.
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

- **Instances without processes run their declarative part** (#449, LRM 14.4.1, 14.5): only
  architectures that own processes were lowered, so a constant that fails when evaluated (a call,
  an overflow, `'value` of a bad string) went unnoticed in an instance with none. An instance whose
  entity or architecture declares a constant with a run-time initial value (no analysis-time
  value) is lowered and its instance data built at the start (`Ir::init_regions`). Ceiling:
  blocks and generates without processes are not covered, nor are other failing declarations
  (subtype ranges, signal initial values).
- **`in` ports collapse into their actuals** (R4 of `docs/design/performance.md`, #413, user,
  2026-10-19; plan rule 3): after `bind_late` (`collapse.rs`), the formal of an `in` port (or a
  component port) whose actual is whole kernel leaves of the same type (one consecutive range
  of one object), or, for a formal of one leaf, part of one leaf (`d => v(i)`, `s => b(3 downto
  2)`: `SigObj::part`, the kernel range; yavs-interp `InstSig::off` and every reader of an
  object's elements add its offset), with no conversion, takes the actual's leaves: its own
  leaves, port edges and the copy per delta are gone, and the leaves are renumbered in their
  old order. The formal object keeps its
  name, scope and shape, so waves, `'path_name` and instance data name it as before; a chain
  through a component port collapses to the signal at its end. The effective value of an `in`
  formal is its actual's (LRM 14.7.3.3), so values, events, activity, `'last_value` and implicit
  signals are unchanged; `'driving` does not apply to `in` ports. Not collapsed: a formal some
  VHDL force or release names (directly, through an alias or an external name: LRM 10.5.2.1, a
  force of an `in` port acts on the port only; a force of a signal parameter keeps every port
  apart), a formal with a driver, a guard, a user resolution function or in a composite resolved
  as a whole, and every port under `--dap` or `--vhpi` (a debugger or plugin may force or
  deposit any port; edge triggers make the same choice). On by default, `--no-collapse-ports`
  (`ElabConfig::collapse_ports`) keeps every port apart; the difftest corpus runs both ways
  (`YAVS_RUN_FLAGS=--no-collapse-ports`).
- **`out` and `buffer` ports collapse into their actuals too** (#528, user, 2026-10-22; extends
  plan rule 3 and the entry above): a formal of mode `out` or `buffer` that is the only source
  of its actual's elements (no driver, other port or conversion on them), with the same
  conditions on the association as for `in` ports, takes the actual's leaves, and its drivers
  drive the actual. The effective value of an `out` or `buffer` port and of a signal is its
  driving value (LRM 14.7.3.3), and the driving value of an actual with one source is that
  source's (unresolved, or `resolved`, the identity on one value; a formal resolved by
  `resolved` with several drivers resolves them the same way in the actual), so values,
  events, activity, `'transaction` and `'last_value` seen through either name are the same;
  `'driving` and `'driving_value` read the process's own driver, which only moves. The actual
  takes the formal's initial value (the deepest formal's through a chain): it is the actual's
  value after initialisation either way (LRM 14.7.5.2) and the drivers start from it. Not
  collapsed: `inout` ports; an actual that is the formal of an `in` or `inout` port or of a
  conversion (its effective value comes from above), guarded, in a composite resolved as a
  whole, or forced or released by a VHDL statement; a resolved formal on an unresolved actual.
  The same `--no-collapse-ports`, `--dap` and `--vhpi` switch it off; the difftest corpus runs
  both ways and `hierarchy/out_port_collapse` covers the cases.

- **The elaboration-time session grows one program** (#383, elab agent, 2026-10-19): a function
  that uses package state and is called for the first time is lowered into the session's
  program by the kept lowering (`Lowerer::suspend`/`resume`, `snapshot`), and the runtime moves
  to the grown program with the package data, shared state, open files and assert counts as they
  are (`Runtime::extend`). The program itself moves between the runtime and the lowering
  (`Lowerer::snapshot_take`, `Runtime::take_program`) instead of being copied for every function,
  and only the functions added since the last snapshot are numbered and verified again; the
  state is rebuilt when a lowering fails (#425). Lowering every run-time package again, initialising them and replaying
  the log per new function was most of elaboration on library testbenches. A full rebuild stays
  for `Session::undo`, a failed lowering or call, and a change of the run-time packages.
  The simulation's program then goes on from the session's lowering, and every build starts
  from a copy of the package data, shared state and assert state elaboration left, instead of
  initialising the packages and replaying the calls once more. Both stay as the fallback with
  coverage counters (another program), other run-time packages, a subprogram with external
  names lowered during elaboration (it fails on entry there), a file opened during elaboration
  (open files cannot be copied) or a failed call not undone.
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
  signals (two port edges per port, uncollapsed except `in` ports, see above); it is not part of path names, the bound entity
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
  elaboration cannot evaluate at all (E2106: a null multi-dimensional aggregate, `'PATH_NAME` of
  a named entity other than a design unit)
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

- **Package bodies of units loaded during elaboration** (#515, #516): an entity or architecture is
  loaded when its instance is elaborated, and with it the packages it names. `Elab::instance`
  loads the bodies of the packages that appeared since the last time (`load_new_bodies`, the same
  `load_bodies` the start of elaboration runs), so a function of such a package called during
  elaboration (a signal initialiser, a generic map) has its body. Without it the call stopped
  with E2106 "whose body is not analyzed" for designs whose component is bound by default to an
  entity that no unit of the analysed closure names.

- **Run-time packages are lowered and initialised in dependency order** (#517;
  LRM 14.4.2): `run_time_packages` lists the packages in unit order and then puts every package
  after the packages its declaration or body names (`dependency_order`, a depth-first walk of
  `Unit::deps`; packages in a cycle of body dependencies keep their unit order). Units are
  numbered in the order they were first loaded, and the body of a package loads packages that
  its users did not name before, so a package could come before one it calls into and its
  lowering stopped with E2001 for the other package's shared variable.

- **Shared variables of regions in elaboration-time calls** (#512; LRM 14.4, 6.4.2.4): a
  function declared in an architecture, block or generate that reads a protected shared variable
  of its region (a testbench's random generator, say) is called during elaboration in the
  session that holds the package state (`Session::call_in`), not in a program of its own. The
  lowering captures the variable as a cell of instance data holding the handle of its object
  (`Lowerer::capture_free_objects(true)`, `FreeObj::var`), and `Lowerer::region_var_init` is the
  thunk that creates the object in the shared heap; the elaborator creates it the first time a
  call needs it (`Model::fill_items`, one object per elaborated declaring region), so later
  calls and constants see its state. The simulation starts with those objects (the session's
  state moves to the build, `StartState::vars` seeds the instance data's `once` map); when it
  cannot start from the session's state, the logged calls are replayed and create the
  objects again (`Call::fill`, `Create`, `pkgcall::replay`). A call's log entry keeps the
  objects it read by declaration (`FillItem`), not as bytes, since any lowering lays them
  out. Functions lowered this way read instance data laid out for the call: they are given up
  (`forget_free_objects`) before the design is lowered, which lowers them again for the
  architecture. Such a lowering is never read from or stored in the stage cache. A
  non-protected (`--relaxed`) shared variable of a region is captured the same way (#524): the
  cell holds the address of its slot in the shared heap (`Space::Region`, shape from
  `shared_shape`), and the creation thunk (`Fx::new_shared_slot`, shared with the lowering of
  the region's own declaration) returns that address instead of a handle.
- **Signal initial values that read signals (#559, L117):** the initial value of a signal may read an `in` port or a signal declared before it (e.g. `signal r : std_logic := port_in;`, a registered input in a debounce or synchroniser). The value is the declared initial value of the signal; an `in` formal starts with its actual's (`Elab::edges`), so a port reads as its actual's initial value, through any chain of ports. The same holds for a constant of a region that reads a signal (#453): it is folded where its declaration is elaborated, with `sig_init` set. Only then (`Model::sig_init`): a port actual, constant or generic that reads a signal is still not static (`E2106`, or the equivalent process for an expression actual). One reference accepts, one rejects at analysis; LRM unclear, so permissive.

## Runtime: default exit severity (user, 2026-10-18)

`yavs run` defaults to `--exit-severity=failure`, like the reference simulators. A report or assert of
severity ERROR is printed and the simulation continues; the run exits 1 at the end if any ERROR or
higher was reported and not cleared by `ClearVhdlAssert` (the std.env assert counts, #191). FAILURE stops the simulation at once. yavs's own runtime errors (bound,
overflow, null access, delta limit, internal) still stop immediately, per the error motto.
`--exit-severity=error|warning|note` lowers the threshold. The test runner passes its own setting.
Refinement (user, 2026-10-21, #498): the end-of-run rule above holds only without the option. Given
explicitly (also as `failure`), the level is the whole threshold and only reports at or above it
affect the exit status, as the reference simulators do: a run that prints `error` reports and ends
normally exits 0.
Reports go to stdout in the reference simulator's format (`** Note: <fs>fs+<delta>: msg`, then `    Process <path>`),
which tools/difftest parses.

## Report during elaboration (yavs-interp, #299)

- **Decision:** a `report` or assertion run without a kernel (a function called while
  elaborating, e.g. in a signal's initial value) prints `** <Severity>: (init): <message>` on
  stderr at once, before the first cycle, and elaboration goes on. Severity failure stops
  elaboration with a runtime error (`E2107`). Why: LRM 10.4 allows report in any subprogram;
  the reference simulator prints the same line. Source: #299.
- **Decision (#349, agent, 2026-10-19):** a report or assertion of severity error or higher
  during elaboration is counted in the VHDL assert counts like a run-time one, so the run
  exits 1 at the end (`ClearVhdlAssert` does not clear it: process-wide count); a disabled severity is neither shown
  nor counted. Elaboration still goes on after an error. Why: of four reference simulators two
  stop at an elaboration-time error (exit 1) and two print it and simulate on (exit 0): a tie,
  so the permissive reading (go on, rule 15) with the exit status yavs gives a run-time error
  report. Warning and note are counted per severity but do not change the exit status, as everywhere. The user may
  prefer stopping at the error (the other half of the tie). Catalogue row
  `elab-time-report-error`.

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
  A guarded concurrent assignment tests the block's guard condition itself on the current
  values, not the GUARD signal, which updates a delta after the signals the condition reads
  (LRM 11.6; the references update GUARD in the cycle of the event); its sensitivity is the
  condition's signals instead of GUARD (#373). GUARD starts as the condition on the initial
  values of its signals (`guard_signal` in yavs-elab, evaluated like a signal's initial value,
  #464). A process reading GUARD still sees later changes a delta late (open, #464: the
  references update it in the cycle of the event, which needs the kernel to evaluate the
  condition in its update phase).
- **Null waveform elements** (LRM 10.5.2.2, #352): a waveform with a `null` element is driven
  one transaction per element in order. The first has the assignment's delay mechanism, so it
  preempts as the whole waveform would; the rest are transport, which only appends after it
  (they are later than the first, so nothing is left to delete). A value element is an
  `Inst::Drive` of one element, a null element an `Inst::Disconnect` at its own time (the
  explicit `after`, 0 fs without one; the disconnection specification time belongs to the
  GUARD of a guarded concurrent assignment only). Still `E2001`: a null in an aggregate target
  or a signal parameter, a pulse rejection limit with a null first element, and in a record
  signal (blocked by #595). sim1 loses the value before and the null itself in some
  multi-element forms and sim2 cannot run them: the corpus case `sigassign/null_waveform_mixed`
  is `ref=hand`, `tests/discrepancies/null-waveform-in-waveform`.
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
  declares it, a process label its process. During elaboration `'path_name` and
  `'instance_name` of a design unit evaluate to the same text (`Model::eval`), so a constant
  built from them can be a generic actual or a bound (#314). Objects declared in a process or subprogram add
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
    (ponytail: a fixed rule, no tuning by pause time). Output identical at any `--threads`
    and with `--shuffle`. Between cycles at the safepoint, and also inside the activation of a
    serial process before a `new`, a `deallocate` or a native call (`Machine::gc_inline`, same
    trigger, once the simulation has started; elaboration code is never collected): a
    testbench whose main process never waits runs in one cycle and would keep every object it
    drops (a 400 MB run became 54 MB, #409). Private heaps are collected the same way.
  - **Collection inside an activation is kept, with precise roots** (user, 2026-10-19, #422):
    the running process's roots are its frames' root maps, the temporaries recorded by
    `Inst::TempRoot` and the `Handle` registers live at the safepoint each of its frames is at
    (the top frame's `new`/`deallocate`/native call, the call each caller waits in), from a
    per-function liveness analysis over the register types (`yavs_interp::roots`, computed
    when a collection first meets the function and kept per runtime). Live `Ptr` registers,
    `RootKind::Addr` slots (parameters passed by reference, array parameters, aliases) and
    recorded fat pairs keep the heap objects they point into. Nothing is scanned
    conservatively (plan rule 6 holds again); the earlier version scanned every aligned word
    of the frames, registers and temporaries. Rationale: a word that only looks like a handle
    kept garbage, and plan rule 6 asks for precise roots. The other serial processes are
    suspended, so what they published is current.
  - Why it is safe under `--threads N`: collection inside an activation only runs while the
    running process is the only one that can touch the heap it collects. The shared heap is
    reachable only from serial processes (touching shared state makes a process serial,
    `proc_flags`, and a serial process allocates everything there); the kernel runs the serial
    processes of a batch one after another on one thread once the parallel ones are done, and
    a serial process holds the shared state, taken from its mutex, for its whole activation
    (`InterpProcess::exec`). The other serial processes are suspended, with their roots
    published at their last suspension. A private heap belongs to one non-serial process: its
    handles cannot reach shared state, signals or another process, so only its own process
    ever collects it. What is collected depends only on the process's own instruction stream
    and the published roots, so the output is identical at any thread count.
  - Contract for the dispatch loop and compiled code: when a collection runs, every frame's
    `pc` is the IR pc of its safepoint (the top frame's `new`, `deallocate` or native call,
    the call instruction each caller waits in) and the frame's registers, slots and the
    temporary area are in the machine. A frame at any other pc has no root map entry
    (`LiveRefs::get`), so the collection is skipped, never unsound. Only registers live at
    the safepoint are read: the verifier proves every register written before it is read
    (#430), so a live register has been written there and a stale value is never a root
    (frames still start with zeroed registers, `pre.rs`).
  - Testing: the `gc-stress` feature of yavs-interp (`yavs` forwards it) collects at every
    safepoint after an allocation, never reuses a slot and panics when roots cannot be located or a live register
    that is not a `Handle` holds a live handle (a lowering that types a handle wrongly); the
    differential corpus, the library suites and the external test runner suites run with it
    (`crates/yavs-interp/AGENTS.md`).
  - Compiled code (LLVM tier) reaches `new` and `deallocate` only through `TierHost::step`
    (they are not inlined), with the top frame's registers, slots and temporaries swapped
    back into the machine and the register file written before the call; natives and calls
    are exits. So a collection there sees the same precise roots. Compiled `Inst::Stmt` steps
    while temporaries are recorded (`TierFrame::temp_roots`), so the interpreter drops the
    ending statement's records.
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
- **VHPI model array results (#330, 2026-10-19):** a function may return a one-dimensional
  array of scalars. The IR carries the result's index (`VhpiModel::ret_index`: left bound,
  direction, length when the result subtype is constrained); the interpreter builds the fat
  result in statement temporaries from the cells the model put. An unconstrained result is
  sized with `vhpiSizeConstraint` before its value (a put without it fails, as in both
  reference simulators that run array results) and indexed `1 to n` (both references); one
  reference refuses composite results at analysis. Rule 15 majority, recorded in the
  discrepancy catalogue (`fresult`).
- **VHPI model record, access and signal parameters (#330, 2026-10-19):** records of scalars
  and constrained arrays of scalars (fields as their own cells), `in` access values (the
  address of the designated data, `vhpiPtrVal`), and signal parameters of scalars or arrays of
  scalars (`ForeignArg::Signal`: the reference, the caller's driver slot, a descriptor). A
  signal parameter crosses as its kernel `Value`; what the model drives comes back as a deposit
  and transactions (`yavs_interp::ModelSignal`) that the interpreter applies through the
  calling process's `Ctx` after the call (new `Ctx::deposit`), so foreign code never touches the
  kernel and runs are the same at any thread count. Writes to record fields are accepted (one
  reference refuses, one accepts: permissive); signal parameter behaviour follows the one
  reference that runs it (the other rejects the class). Catalogue: `fparam`.
- **VHPI foreign architectures (#330, 2026-10-19):** the elaborator elaborates a `"VHPI
  library model"` architecture (also `"VHPI library;model"`, the form one reference needs) as
  its ports and its entity's declarations and statements only (`Elaborated::foreign_archs`).
  After the plugins load, `yavs_vhpi::elaborate_foreign` runs every `elabf`, then every
  `execf`; the processes and drivers the models create (`vhpi_create`) become
  `Elaborated::ext_procs`, built as processes that own the drivers and run no code. A model
  drives them with `vhpi_schedule_transaction`, which goes through the new kernel
  `Hook::assign` (a signal assignment through a driver, from an observer): the transaction is
  scheduled at the callback's time like a process's, deterministically at any thread count. A
  model no plugin registered is `E0905` before the simulation (fail early: the architecture has
  no behaviour otherwise). The one reference that runs foreign architectures preempts a vector
  transaction as a whole; yavs preempts per element as for every signal assignment (LRM
  10.5.2.2), recorded in the catalogue (`farch`).
- **VHPI library and application models (#357, 2026-10-19):** `vhpiLibF` and `vhpiAppF`
  registrations are run, which they were not before. A library model's `elabf` runs once when
  the first foreign subprogram or architecture of its library (the `'FOREIGN` library word) is
  bound, its `execf` once at the start of the simulation; an application model's `elabf` and
  `execf` run once at the start of the simulation, with or without foreign models in the
  design; all before the start-of-simulation callbacks, with the registration as `obj`. A library
  no foreign model of the design names never runs. No reference simulator runs these (one
  refuses the registration, one accepts it and never calls either function, the third has no
  foreign models), so there is no majority to follow and the standard's description decides.
  Catalogue: `libapp`.

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
- **Records and arrays of composites** (#79): the descriptor and the memory image of the value,
  `u32` descriptor size, descriptor, `u32` data size, data (the value's layout in the interpreter,
  logic planes included; no access values can be in a file). Only yavs reads it back, and only
  into an object with the same bounds: another descriptor or size is a runtime error (`a value
  read from a file has other bounds than the object it is read into`); `LENGTH` of the
  unconstrained form is the stored length of the first dimension. Not exchangeable with other
  simulators, which write their own layout (the format is implementation-defined, LRM 5.5.2).
- **`std.textio.TEXT` is the exception** (#365, lrm-checks L100): its implicit `WRITE` stores the
  characters alone and `READ (F, VALUE, LENGTH)` reads `VALUE'LENGTH` characters, so `READLINE`
  and `WRITELINE` share the file; `WRITE (OUTPUT, s)` prints `s`. Other files of `string` keep the
  element count.

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
  array and record bounds. `E1233` stays for access types (user, 2026-10-19: keep reject) and
  file types (never a return type). Of the references that run
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
- **Access parameters of protected methods in 2019 (L46, #212, LCS-2016-004; user, 2026-10-19,
  rule 15):** accepted in 2019, as access results and file parameters are: 2019 drops the 5.6.2
  restriction on method profiles. 2008 keeps `E1223`. It replaces the morning's majority-rule
  `E1223` in 2019.
- **Records mixing protected and other elements (L102, #371, 2019 5.3.1; user, 2026-10-19, "reject,
  follow LRM"):** `E1905` at the first non-protected element; all four references reject it. Records
  whose elements are all protected, arrays of protected types and access types to them stay accepted (#361).
- **Negated literal at the i64 minimum (L58, #214; user, 2026-10-19, majority rule):** `-9223372036854775808`,
  for integer and physical types, is `E1402` in user code (all three reference simulators reject it;
  the range of `universal_integer` is implementation-defined). Library `std` stays exempt: its `TIME`
  declaration uses the literal.
- **Implicit condition operator (L53, #48; user, 2026-10-19, rule 15):** a visible `??` for the
  condition's type first; when none is visible, the `??` declared beside the type, so
  `use ieee.std_logic_1164.std_ulogic` alone is enough. 9.2.9 does not say where the operator is
  looked up, so the permissive reading wins; it replaces the morning's majority-rule `E1340`.
- **A type through a formal package of a formal package (L76, #101; user, 2026-10-19, rule 15):**
  `s.r.arr` in a generic package whose formal package `s` has its own formal package `r` is the
  type of the actual instance, so a call in an instance accepts an actual of that type. The LRM
  does not make it a distinct type; this replaces the morning's majority-rule `E1300`.
- **Rows kept rejected under rule 15 (user, 2026-10-19: keep reject):** `overload-by-conversion-type`
  (`E1302`), `port-conv-inout-one-way-in` (`E1313`), `formal-conversion-unconstrained-port`
  (`E1312`), `external-name-architecture-subprogram-unbound` (`E2117`),
  `aggregate-piece-mixed-direction` and `aggregate-target-range-directions` (`E1334`),
  `i64-min-literal` (`E1402`), `formal-package-per-element-box` (not parsed), `div-by-zero-in-range`
  (`E1401`), `generic-subprogram-of-block` (`E1506`) and a 2019 named return value of an access
  type, `return r of t` (`E1233`). Listed in `docs/design/lrm-checks.md`
  "Rule 15 rows".
- **Static index and slice out of range (L51, sem.md 4.3; user, 2026-10-19, rule 15):** a locally
  static index or non-null slice outside the static index range of its array is warning `W1404` at
  analysis, an error only when the code runs (the run-time check). The code may be dead; this
  replaces the morning's majority-rule error `E1404`, which is retired.
- **Formal conversions overloaded by result type (L66, #229; user, 2026-10-19, majority rule):**
  `set(cv(x) => r)` with two `cv(integer)` returning different types is `E1302`: the actual's type
  does not choose the overload (two of the three reference simulators reject it). Port maps already
  required a unique conversion.
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
- **Later resumptions (#417, rule 15):** a process with several waits that a signal resumes
  after it has been resumed before (any reason, any earlier cycle after initialisation) runs
  like a one-wait process, in elaboration order. Why: for two loops with several waits on one
  clock edge (a monitor declared before a testbench process, the common shape) three of the
  four reference simulators run the earlier declared one first and the fourth varies, while
  the reverse rule above made a testbench that tests a queue the monitor empties on that
  edge end one clock period later than all of them (`process/process_order_resumed`). The
  first resumption after initialisation, timeouts and concurrent procedure calls keep the
  rule above (there the references are split or agree with it). Kernel: `ProcMeta::resumed`.
- **Later resumptions, refined (#462, #469; rule 15):** the processes of the previous rule
  (the "settled" ones) run after the concurrent procedure calls and in the order deeper
  instances first, then elaboration order. Why: the first form put them ahead of everything with
  several waits, which broke two shapes of the external runner's own suites that pass in all four
  reference simulators: a protocol checker written as a concurrent procedure call (declared
  before the testbench process) and a checker process inside an instance declared after the
  testbench process must both report on a clock edge before the testbench process that reads
  their log on that edge (`check` suite `tb_check_stable`, seven `verification_components`
  tests). Reduced cases, all four references: a concurrent call that loops on a nested wait
  before a testbench process that drives a signal: the call first in all four; the same with an
  inline wait, no drive: first in two, varying in one, last in one; an instance process declared
  after the testbench process: first in the three references with a fixed order (the fourth
  alternates), the instance processes before the parent's own in all three. Processes of the
  same instance level keep declaration order (`process/process_order_resumed`); the concurrent
  call and instance shapes are `process/process_order_call` and `process/process_order_instance`.
  Kernel: `ProcMeta::depth` (colons in the path name).
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
- **Values typed in the debugger are VHDL literals of the target's type** (#323): value
  breakpoints, force, deposit and `setVariable` share one parser, `yavs_std::value::literal`
  (scalars as `T'VALUE` reads them, string and bit string literals, aggregates with named
  elements, ranges and `others`), and the Variables view writes values the same way. Why: one
  syntax a VHDL user already knows; text comparison could not match `x"A5"` with `"10100101"`.
- **Force, deposit and release from the debugger go through `SimControl`** and take effect in
  the next cycle, from a stop between cycles or inside a process alike (#323). A field, element
  or slice acts on its kernel elements only (#364), so a force of `d(7 downto 4)` leaves the
  other elements to their drivers. Why: the plan
  lets nothing else drive the kernel, and a stop inside a process is mid-cycle.
- **Breakpoint conditions are evaluated by the DAP server, not lowered to IR** (#326): the
  condition is parsed as a VHDL expression and evaluated over the locals and signals the
  debugger shows, with the predefined operators (relations, logic on boolean and bit, integer,
  real and physical arithmetic, `??`); function calls, attributes and overloaded operators are
  rejected when the breakpoint is set. Why: ir.md 3.4 plans a condition function nested in the
  statement's function, but the program is fixed once the simulation runs; this covers the
  conditions users write (`count = 3 and state = IDLE`), and lowering stays possible later.
- **Live waves are a second sink of the `--wave` recorder** (#325): `yavs/waveSubscribe` builds a
  `yavs_wave::Recorder` (`record_to`, a `Sink` beside the FST `WaveWriter`) for the client's
  selection, so the stream has the hierarchy, selection rules, final value per time step and
  entry order of the file. Values go out in the compact binary encoding (user, 2026-10-19,
  "binary now"; it first went out as text): base64 of the recorder's value bytes, the layout
  the FST path gets (`yavs_wave` crate docs: one byte per element of a bit/`std_logic` vector,
  `u32` enumeration position, `i64`, `f64`, little endian), one string per change. The subscribe
  response gives every signal an `encoding` (`layout`, `width`) beside its type, which is all a
  client needs to decode. One base64 blob per batch with offsets is not smaller (the values are
  a few bytes each), so it is not used. Batches go through a bounded queue and are merged when it
  is full (only the last change of each value kept, `merged`), so a slow client never blocks the
  simulation (user decision, 2026-10-19: never block the simulation, merge queued batches);
  `docs/design/gui.md` 5.1 is updated to match.
- **`--wave` and `--vhpi` work under `--dap`** (#325): the CLI attaches the same observers to the
  debug build and the recorder finishes before `exited`. Under `--dap=stdio` the CLI
  moves file descriptor 1 (the C runtime's stdout, also on Windows) to standard error before
  anything loads and gives the server a private duplicate of the original stdout, so a plugin's
  prints cannot corrupt the framing (#389).
- Details of ids, request routing and pause handling: `crates/yavs-dap/AGENTS.md` section 3.

## Code coverage (yavs-cov, yavs-lower, yavs-elab, CLI, M9, #131, 2026-10-19)

- **Counters are IR:** lowering with `LowerOptions::coverage` emits `Inst::Cover` (plan rule 10),
  so every tier counts them; without it the program has no counters and the runtime allocates
  none (zero cost). Statement items: every `stmt_start` except `Decl`, `Process`, `Subprogram`
  and `Implicit` markers. Branch items: `CovKind::Branch(arm)`, arms in source order; an `if`
  without `else` and a conditional assignment without a final `else` get one more arm (no
  condition held); a `for`/`while` loop has one arm, its body, counted per iteration. Files of
  the std/ieee libraries yavs provides and the implicit processes of port actuals and guards
  get no counters. Why: users want their own code's coverage, and a counter in `numeric_std`
  is noise and run-time cost.
- **Counter layout:** one row of `Program::cov.len()` counters per hierarchy scope plus row 0
  (code without instance data); a region's instance data has `cov_base = (region + 1) * items`
  (`Elaborated::cov_rows`). Atomic relaxed adds, so counts do not depend on `--threads`.
  ponytail: dense rows, most unused; rows only for regions with instance data if memory matters.
- **Identity (rule 10):** an item is (path, kind, FNV-1a of its statement's source text with
  whitespace runs as one space, occurrence among same-text items of its unit in source order).
  The path is the instance path (`:top:u1`) for items of architectures and entities, summed
  over the instance's blocks and generate iterations, and `library.package` for everything
  else, summed over all callers (a subprogram passed as a generic subprogram counts under the
  unit declaring it). A unit owns the statements between its name and the next unit's name in
  the file. Why: offsets would break merging after any edit above the statement; per-iteration
  generate paths would make generate-heavy designs hard to read.
- **Database:** text, `yavs-coverage 2` then one tab-separated line per item, sorted; one file
  per run (`--cov=FILE`, default `yavs.ycov`, overwritten), written whatever the run's outcome;
  `yavs cov merge` adds counts per key and keeps items only one input has. Cobertura output has
  timestamp 0 so equal databases give byte-identical reports.
- **HTML (#344, reworked #689):** `cov report --html DIR`: static files (inline CSS and one
  small inline script, no external resources, light and dark by `prefers-color-scheme`, no
  timestamp so equal databases give equal bytes). `index.html` has totals and a row per instance
  or package and per file; `src-N.html` (N in file-name order) shows the source with the largest
  statement count per line. Sources are read at report time from the database's paths; an
  unreadable one is `E0113`, nothing is written, rather than a silently thinner report. A
  hand-written template keeps the crate free of dependencies.
- **Report paths are compared as strings (#693):** `cov.rs::relative` strips the root from
  report paths lexically after folding `\` and `/`, drive letter and directory case on Windows
  style paths, `\\?\` prefixes and `.`/`..` components; a path outside the root stays as it
  is. Why: `Path::strip_prefix` failed when one side had `..` (a root built from
  `CARGO_MANIFEST_DIR`) and the other was normalised by Windows, so Windows reports showed
  absolute paths. Symlinks are not resolved.
- **HTML interaction (user, 2026-10-24, #689; replaces the earlier "no scripts" rule):** (1)
  source paths are shown relative to the current directory (`--root DIR` strips another prefix),
  in headings, the index and exclusion rows; matching never uses the path. (2) No item lists
  under source lines: a line with items is clickable and keyboard-focusable and its popup lists
  the items per instance with count and tests (covered-only lines too). (3) A line
  that ran in some instances and not in others has its own colour and a `ran/total` instance
  marker, explained by the legend. (4) A missed item's exclusion row and YAML entry in the popup
  each have a copy button (the script adds the buttons and the open/close). The popup is a list of groups (title,
  table), so a new item kind (condition rows, #688) adds groups without a new layout.
- **Not yet:** the LLVM tier does not lower IR yet, so the interpreter is
  the only tier today.

## Coverage exclusions (yavs-cov, CLI, #351, user, 2026-10-19)

- **Requirement (user):** "it should be easy to exclude by reading the report". Choices of the
  user, in order: identify an item by its text hash, not its line ("if the line moves then it
  is still valid"); keep the line number optional ("we can have a line number optional, but
  warn if the placement changes"); make the line "dead simple", with `#` comments and no
  required reason; start every row with the type of exclusion.
- **Rows:** `statement INSTANCE|GLOB HEX12.OCC [FILE:LINE]`, `branch INSTANCE|GLOB
  HEX12.OCC.ARM [FILE:LINE]`, `instance GLOB` (that instance and everything below it), `unit
  LIB.PACKAGE`, `file GLOB`. HEX12 is the first 12 hex digits of the statement-text hash.
  `yavs cov report --uncovered` prints rows of exactly this shape with the line and a trailing
  comment (arm and statement text), so pasting a report line into the exclusion file is the
  whole workflow. Items match by instance, kind, arm, hash and occurrence; `FILE:LINE` is
  never used for matching.
- **Warnings (stable codes, `-Werror`-able):** `W0110` the item is now on another line than the
  row says (the exclusion still applies), `W0111` a row matches nothing (stale: the statement
  text changed), `W0114` an excluded item ran. Malformed row or unknown type word: `E0112`
  naming the line. Why warnings: a stale or surprising exclusion should be seen, not fail the
  report.
- **In source:** `-- coverage off[: reason]` ... `-- coverage on`, `-- coverage off-line` (the
  statements starting on that line), alias `-- yavs coverage ...`. The items are left out of
  the database when it is written (the counters still run); an `off` never closed is `W0112`,
  an `on` without `off` `W0113`. `translate_off` regions only with `--cov-exclude-translate-off`.
  ponytail: the pragma scan treats a `--` inside a string literal as a comment start.
- **Applied when reporting:** the database keeps raw counts (so one database serves different
  exclusion files); excluded items are out of the totals and the Cobertura report and are listed
  with their reasons. `cov merge` does not take exclusions. `unit` names packages only:
  entity and architecture items are excluded with `instance`.
- **HTML popups open on click only (user, 2026-10-24, #689; supersedes hover/focus popups):** a
  popup never opens on hover or focus. A click on a line opens it, a click on another line moves
  it, a click on the open line or outside any line closes it; a focused line (`tabindex`) opens
  and closes it with Enter or Space, Escape closes. Hover only shows a pointer cursor and a
  light highlight. Without the script nothing opens (accepted); the popup text stays selectable
  because clicks inside it change nothing.
- **In the HTML report (user, 2026-10-24, #689):** a missed item's popup shows its exclusion row
  with a copy button (the text row and the YAML entry of `--uncovered=yaml`), so the workflow stays
  "copy the row from the report".

- **YAML exclusion files (#687, user, 2026-10-24):** `--exclude FILE` also reads YAML; the
  one-line text rows stay, both are accepted. The parser is chosen by extension: `.yaml` and
  `.yml` (any case) are YAML, anything else the text rows. Shape: `exclude:` is a list of maps
  with `kind` (`statement`, `branch`, `toggle`, `instance`, `unit`, `file`) and the fields of
  that kind: `instance` + `hash` (`HEX12.OCC`, branch `HEX12.OCC.ARM`) + optional `line`
  (`FILE:LINE`, never matched) for statement and branch; `instance` + `signal` + optional
  `direction` for toggle; `instance` for instance; `unit`; `file`; every kind takes an optional
  `reason`. Same matching and warnings as the rows. An unknown kind, an unknown key (also a key
  of another kind), a missing field, a duplicate key or malformed YAML is `E0112` naming the line.
  Every value is read as the text it was written as (`no`, `on`, a hash of digits only, a hash
  that looks like a float), never typed. `--uncovered=yaml` prints a standalone file of the
  misses (`exclude:` header, every value double-quoted, the item's text as a YAML comment, no
  `reason`); plain `--uncovered` stays the text rows. Why: reviewable files with reasons, tool
  friendly. The HTML report keeps the text row. Why a new entry kind is cheap: add its key
  list and rule in `exclude/yaml.rs::entry`.
- **YAML crate: `saphyr-parser` (yavs-cov only).** Pure safe Rust, YAML 1.2, two tiny
  dependencies, released 2026-10, and it yields events with line numbers and the raw scalar
  text: that gives the `E0112` line and avoids typing. Rejected: `serde_yaml` (archived,
  deprecated), `serde_yml` (deprecated shim), `serde_norway` (serde_yaml fork on `unsafe-libyaml`,
  last release 2024-12), serde front ends (they type scalars, so `no` or `123456789012.0` need
  workarounds, and give no line).
- **Toggle direction (#687, user, 2026-10-24):** a `toggle` exclusion may name one direction:
  text `toggle PATH SCALAR rise|fall`, YAML `direction: rise|fall` (`rise` is 0->1). Omitted
  means both, so existing files stay valid. `--uncovered` prints the direction when only one
  direction never happened (both missed: one row without it). `W0114` is now per excluded
  direction that ran (it was once per scalar that toggled both ways).

## Toggle coverage (yavs-cov, CLI, #343, 2026-10-19)

- **An observer, not IR:** `yavs_cov::toggle::Toggles` subscribes to the kernel's value-change
  bus only when toggle coverage is on (`--cov` with `toggle` in `--cov-kind`, the default), so
  a run without it has no toggle cost. It keeps the last level of every tracked scalar and
  compares on each change; changes reach observers once per cycle on the main thread, so the
  counts do not depend on `--threads`. Why: docs/design/ir.md puts toggle coverage on the bus.
- **What counts:** every scalar `bit` or `std_ulogic` subelement of signals and ports (not
  `boolean`, not implicit signals, not component ports). Levels: '0'/'L' are 0, '1'/'H' are 1,
  every other value is neither; a toggle is a change from 0 straight to 1 or 1 to 0, so a change
  through 'U', 'X', 'Z', 'W' or '-' is none, and the initial value never counts. Why: a
  transition through an unknown or high impedance value is not a proven 0/1 toggle; this is the
  common default of other simulators' toggle coverage.
- **Identity:** two items per scalar (`t1` 0->1, `t0` 1->0): the path of the declaring scope
  (`:top:g(3)`; package signals `library.package`, as package statements) and the scalar's
  name (`data(3)`, `r.valid`, `m(1,2)`), never offsets. A scalar is covered when both ran.
- **Cap:** a signal with more scalars than `--cov-toggle-max` (default 65536, as
  `--wave-max-array`) is left out with `W0115`. Why: memories cost a counter pair per bit and
  are rarely toggle targets.
- **Database version 3:** toggle lines; `yavs-coverage 2` is still read. Cobertura has no
  toggle notion: toggles are left out of it (text report only). Exclusion row `toggle PATH
  SCALAR [rise|fall]` (globs allowed) takes out both directions or the named one; `W0114`
  for each excluded direction that ran.

## Per-test coverage attribution (yavs-cov, CLI, #353, user, 2026-10-19)

- **Requirement (user):** a test name per run, the tests that hit each item kept through
  `cov merge`, `yavs cov rank` (greedy set cover plus per-test unique contribution) and a
  report of which tests hit an item; keep the text database with a versioned header and a
  tests section.
- **Name:** `--cov-test NAME`; default the `enabled_test_cases` field of a `runner_cfg`
  generic, else `top` or `top(g=v,...)` from the `-g` generics in command-line order. Runs with
  the same name are one test. Why: external test runners pass the test name in that generic;
  without one, the generics are what tells two runs of a testbench apart.
- **Storage:** per item the sorted ids of the tests that hit it (count above 0 in that run),
  ids into a `test` section; `write` numbers tests in name order, so the merged file does not
  depend on the order of the inputs. Database version 4; versions 2 and 3 read with no tests.
  The set, not per-test counts: ranking needs only who hit what, and per-test counts would
  multiply the file.
- **Rank:** greedy (the test adding the most items not yet hit, ties to the first name), which
  is within a log factor of the minimal set; an exact set cover is NP-hard. Items no test hit
  are not counted; `--exclude` applies before ranking.

## Condition and expression coverage (yavs-lower, yavs-ir, yavs-cov, CLI, #688, user, 2026-10-24)

- **Requirement (user):** focused expression coverage (FEC) with exclusion per row. For every
  boolean condition with two or more inputs, one row per input and value; input X at value v is
  covered when, in some evaluation, flipping X alone would flip the result while the other inputs
  hold their values (focused, masking per input, like MC/DC unique cause with masking).
- **What is measured:** a *condition* is the boolean of an `if`/`elsif`, a conditional
  (`when ... else`) signal or variable assignment, `while`, `exit when`, `next when`; an
  *expression* is the whole value of a signal or variable assignment (each waveform element) of
  type `bit`, `boolean` or `std_ulogic`. Only a tree of `and`/`or`/`nand`/`nor`/`xor`/`xnor`/`not`
  (the predefined ones, and the `std_logic_1164` gates on `std_ulogic`) with 2 to 31 inputs
  counts; every operand that is not such an operator (relation, name, function call, edge idiom
  such as `rising_edge(clk)` or `clk'event and clk = '1'`, literal) is one input (a constant one has no rows, see below). An inserted
  `??` is looked through. Vector operands, conditions of `assert`, `wait until`, 2019
  conditional expressions and logic expressions nested in other expressions (arguments, indices)
  are not measured.
- **Constant inputs (user, 2026-10-24):** an input that is constant after elaboration (a
  generic, a constant, a literal, any folded or globally static expression) is evaluated but
  gets no rows, so no row of a constant input is left to exclude; a condition with fewer than two
  other inputs has no item at all. Open: a row of another input can still be unreachable because a
  constant masks it (`(a or b) and kf` with `kf` false); not detected yet.
  `CovCond::consts` marks them; counters are compact (`first + 2 * k + v`, `k` the number of
  non-constant inputs before the input) while the row number in the key stays `2 * input +
  value`, so adding a constant input never renumbers the exclusions of the others. Constness is
  `Fx::cov_const`: a folded value, or a static expression of literals, generics, generate
  parameters and constants of those. Parameters of a subprogram are static to sem but new at
  every call, so they stay inputs, as does a constant in a process or subprogram whose value
  elaboration cannot know. Condition and expression coverage stay off
  by default, and Cobertura keeps folding the rows into `condition-coverage` (user, 2026-10-24).
- **Values:** `bit`/`boolean` false and true; `std_ulogic` `'0'`/`'L'` are 0 and `'1'`/`'H'` are 1.
  An evaluation in which an evaluated input has another value (`'U'`, `'X'`, `'Z'`, `'W'`, `'-'`)
  counts no row. Why: the focused definition needs two-valued inputs.
- **Short-circuit:** `and`/`or`/`nand`/`nor` on `bit` and `boolean` do not evaluate the right
  operand when the left decides (LRM 9.2.2), and lowering keeps that. An input that was not
  evaluated is masked: it may take any value, so it never blocks another input's row. `a and b`
  with `a` false and `b` not evaluated covers row `a = 0` (flipping `a` could change the
  result). Evaluation order and side effects are the ones of the code without coverage: a
  function-call input is evaluated once, when the program would, never extra.
- **Counting:** `Inst::CoverIn` adds each evaluated input to a state register (2 bits: not
  evaluated, 0, 1, other) and `Inst::CoverCond` counts the covered rows from the tree
  (`CovCond::hits`, in `yavs-ir`; rows computed with the set of results each subtree can have,
  unevaluated inputs free). Rows are ordinary coverage counters (`CovKind::Condition(row)` and
  `Expression(row)`, row `2 * input + value`), so counts are the same at any `--threads` and in
  every tier (the LLVM tier steps the two instructions in the interpreter). Without the kinds
  there is no code and no cost. Lowered code is not cached with any coverage kind on.
- **Identity:** (path, kind with row, FNV-1a of the *condition's* source text, occurrence among
  same-text conditions of the unit), not the statement's: an edit of an `if` body keeps its
  exclusions. All rows of a condition share hash and occurrence; the row is the last number.
  The input's text is part of the key (`Key::sig`); the entry text is the result the input
  decides alone (`0`, `1`, or `?` when an `xor` lies between) and the whole condition.
- **Database version 5:** lines `c<row>` (condition) and `e<row>` (expression): path, hash,
  occurrence, count, tests, line, file, input, text. Versions 2 to 4 are still read.
- **Reports:** the text summary adds `conditions` and `expressions` lines and columns when the
  database has such rows; `--uncovered` gives one row per missed row (`condition INSTANCE
  HEX12.OCC ROW FILE:LINE   # a is 0 (result 0) in a and b`); HTML shows, under the line of a
  condition, a table `row input result hit`; Cobertura counts the rows of a line as conditions of
  the line (`condition-coverage`), together with its branch arms.
- **Exclusions:** per row, `condition INSTANCE HEX12.OCC ROW [FILE:LINE]` (and `expression`), or
  YAML `kind: condition|expression` with `hash: HEX12.OCC` and `row: N`; matching, `W0110`,
  `W0111`, `W0114` as for statements. `instance`, `unit` and `file` rules take rows out too.
- **Default of `--cov-kind`:** unchanged (`statement,branch,toggle`); `condition` and
  `expression` are asked for. Measured (user-mode instructions, interpreter, release build, one
  run each): over the default kinds, `condition` costs +27 percent and `expression` +17 percent
  (both +44 percent) on a loop where every iteration has two multi-input conditions and one
  expression among about 14 statements; designs without multi-input conditions or expressions
  cost +0.0 percent (compute loop, LFSR array), one with a logic expression in its main process
  +17 percent for `expression` (4096 flip-flop processes). Each evaluation runs one `CoverIn` per
  evaluated input and one `CoverCond` through the interpreter's generic step (about 100
  instructions each; the covered rows of a state are remembered for conditions of up to 5
  inputs), the same path statement counters use. Not small enough to make default; a predecoded
  op for the counters would cut all coverage kinds (#692).
- **Counter ops (#692):** `Inst::Cover`, `CoverIn` and `CoverCond` are predecoded ops
  (`Op::Cover`, `Op::CoverIn`, `Op::CoverCond`) whose bodies are `#[inline(never)]`
  `Machine::op_cover*` methods that `step` shares, so the arms in `run_ops` are a call and a step
  of `k` (the loop grew by 0.15 percent, designs without coverage move by less than 0.2 percent of
  instructions). Counting costs about 14 instructions per statement counter instead of about 130.
  The LLVM tier compiles `Cover` as one atomic add (monotonic) on the counters of the instance
  (`TierFrame::cov`, `cov_len`: the pointer with `cov_base` applied, filled by `tier_view` and
  `view_root`, never a constant in the module, so the object cache is unaffected) and `CoverIn` as
  arithmetic on the register; `CoverCond` calls the interpreter's own function through the signal
  callback. A missing counter leaves to the interpreter, which reports it. A function with a
  `Cover` is still not native-frame code (`nat_inline`). Counts are the same in every tier and at
  any `--threads` (`coverage.rs` `tiers_agree`, `coverage_fec.rs` `tiers_agree`). Remaining cost of
  coverage runs is mostly not the counters: processes with counters got no specialised per-instance
  copies and functions with counters were not inlined (done in #699, below).
  Measured as user-mode instructions (`perf stat`, bench builds; the LLVM tier minus its compile),
  in millions, before / after; plain run, default kinds, `condition,expression` alone:
  `fecbench` (a condition-dense loop, 300000 iterations) HEAVY=1 interp 414, 911 / 598, 1029 / 776,
  llvm 30, 1099 / 50, 1114 / 295; HEAVY=0 interp 326, 830 / 511, 739 / 582, llvm 25, 1109 / 48,
  724 / 269; `rtl_ff` (CYC=500) interp 3055, 7421 / 6823, 5890 / 5319, llvm 2246, 6238 / 4510,
  5884 / 4223; `rtl_lfsr` (CYC=1000) interp 1311, 4111 / 3901, llvm 919, 3746 / 3130;
  `compute_loop` interp 1051, 1980 / 1685, llvm 235, 2047 / 302; neorv32 single-core minimal,
  30 us, interp 5515, 17464 / 16425, llvm 3996, 13276 / 10053; one OSVVM AXI-stream test
  (`TbStream_AxiSendGetRandomParam2`) interp 3613, 4756 / 4429, llvm 3413, 4184 / 3141. Plain runs
  moved by 0.0 to 0.1 percent (interpreter) and 0.0 to 0.4 percent (LLVM). No cycles were taken.
- **Counters in copies, memoised blocks and inlined functions (#699, user 2026-10-24):** a process
  with counters may have per-instance specialised copies, a block with a `Cover` may be memoised
  and a function with counters may be inlined. Counter identity stays exact because a counter is
  `Cover { item }` against the `cov_base` of the instance that runs the code (interpreter:
  `Machine::inst`, compiled code: `TierFrame::cov`), never part of the code or of a copy's key, so
  equal-data instances that share a copy still count into their own rows, and a callee inlined
  into a caller counts into the callee's items of the running instance, as the call did. The
  counters of code a copy folds away (an arm of a generic test, a dead branch) go with it and
  stay 0 for that instance, which is the correct count. Counters do not count against the size
  limits of an inlined body, so a run with counters inlines what a run without them does. Counts
  are byte-identical to the shared code's in every tier, at `--threads` 1 and 4 (`copies_count_alike`
  and `tiers_agree` in `coverage.rs`, over `tests/cli/cov_copies.vhd`). `sink` and `drives` still
  skip a function with counters (not part of this change). Measured as user-mode instructions
  (`perf stat`, bench builds, default `--cov` kinds), in millions, plain / before / after, interpreter:
  `rtl_ff` (CYC=500) 3111 / 6874 / 5310, `rtl_lfsr` (CYC=1000) 1353 / 3931 / 3743, `compute_loop`
  1059 / 1694 / 1233, neorv32 single-core minimal 30 us 6998 / 16754 / 16670, `sc5_fifo_repack`
  41926 / 77478 / 61754, `uvvm_sb_slv` 14923 / 21456 / 20947, `osvvm_axi4_random` 9294 / 9927 / 9966;
  LLVM tier minus its compile: `rtl_ff` 2283 / 4544 / 3982, `rtl_lfsr` 927 / 3139 / 3126,
  `sc5_fifo_repack` 28285 / 43519 / 42714, neorv32 3731 / 10262 / 9719. Plain runs execute none of the changed
  code (interpreter identical to 0.1 percent; the LLVM figures differ by compile-time noise of up to
  2 percent). `osvvm_axi4_random` is the one case that got 0.4 percent slower (9927 to 9966), within
  what a few more inlined counter ops cost where nothing is specialisable. Databases of base and new
  builds were compared with `cmp` for all five kinds on `cov_copies`, `rtl_ff`, `rtl_lfsr`,
  `compute_loop` (interpreter and LLVM at 1 and 4 threads, auto at 1), `sc5_fifo_repack`, neorv32
  30 us and `uvvm_sb_slv` (interpreter at 1 and 4 threads, auto at 1): all identical. The overhead left is not specialisation: the toggle observer (1.8 to 7.4 G
  instructions alone on these designs) and the edge triggers that coverage switches off, which
  makes every `rising_edge` process also run on the falling edge (#707). The OSVVM AXI-stream test
  of the #692 numbers now ends in an interpreter internal error under `--cov` on main (#705), so
  `osvvm_axi4_random` stands for OSVVM here.
- **Edge triggers stay on under coverage (#707, user, 2026-10-24):** `Config::edge_triggers` is
  off only with `--dap` and `--vhpi` (coverage and the skipping: see "Coverage keeps the
  count-neutral skips"). A clocked process of any trigger shape (`rising_edge`,
  `falling_edge`, `x'event and x = '1'`, `not x'stable` which the analyser already reads as
  `x'event`, `wait until` on an edge or a level, with or without an asynchronous reset) is
  resumed at its edge, so the statement of a pure edge test counts one run at initialisation and
  one per edge, and the arm "no condition held" of an edge-only test shows the initialisation
  run, where it showed every other event on the clock. The statements inside the `if` count the
  same as before. The same holds for the reset test of the asynchronous-reset form (it runs at
  the reset level and at the edge). Accepted by the user; documented in `usage.rst`; pinned by
  `edge_triggers_count_their_edges` (`tests/cli/cov_edge.vhd`, all forms, 1 and 4 threads). What
  stays interpreted with statement, branch, condition or expression counters on: the plain
  signal copy and the clock generator `x <= not x after T;` that the kernel otherwise runs itself
  (`build.rs`, `counters_off`: their statement would count 0); with toggle coverage alone they
  run in the kernel and the observer sees their events. Measured as above (interpreter,
  user-mode instructions in millions, default `--cov` kinds, plain / before / after):
  `rtl_ff` (CYC=500) 3121 / 5311 / 3434, `rtl_lfsr` (CYC=1000) 1356 / 3741 / 1831, neorv32
  single-core minimal 30 us 6861 / 16445 / 14877, `sc5_fifo_repack` 41863 / 61805 / 52097,
  `osvvm_axi4_random` 9309 / 9963 / 9779 (the last needs the #705 fix to run). Per kind, after:
  statement alone 3342 / 1496 / 12761 / 50646 / 9670, toggle alone 3147 / 1722 / 12202 / 46393 /
  9362 (same design order).
- **Toggle observer cost (#707):** where it went, per design (toggle kind alone, millions of
  instructions over a plain run, before / after): `rtl_ff` +1776 / +26, `rtl_lfsr` +2253 / +366,
  neorv32 30 us +7258 / +5341, `sc5_fifo_repack` +14544 / +4530. Most of it was not the bus: (1)
  the edge triggers that coverage switched off (every `rising_edge` process ran at both edges;
  `edge_slow`, `run_quick` and `Process::run` three to five times the plain run); (2) per
  value-change range the observer took a lock, copied the range into a freshly allocated
  `Value` (`Hook::value`: `extract`, `to_value`, a `SmallVec`, a `malloc`) and compared element
  by element. It now takes one lock per cycle (`Observer::value_changes`), reads the plane words
  in place (`Hook::view`, `SigView::planes`), treats the changed ranges of a signal as their
  hull (an element that did not change holds its last level, so the counts are the same) and
  keeps the last levels as two bitsets, so a range costs a few word operations and one add
  per toggled bit. (3) What is left on a big design is fixed, not per value change: building and
  writing the database (a `Key` and an `Entry` with three strings for each scalar and
  direction: about 1.7 G instructions for the 134 thousand scalars of neorv32, a 49 MB file),
  of which the bulk insert in `Toggles::add_to` and the unformatted `Database::write` of toggle
  lines took about 130 M. The rest of what neorv32 and `sc5_fifo_repack` kept over a plain run
  was the skipping that coverage switched off (`skip_unchanged`, hence the idle skip): see
  "Coverage keeps the count-neutral skips". Counts are byte-identical to the old observer
  (`cmp` of databases of `rtl_ff`, `rtl_lfsr` and neorv32).
- **Coverage keeps the count-neutral skips (#707, user, 2026-10-24; replaces the earlier rule
  "coverage observes activity").** Under `--cov`: (1) `Config::skip_unchanged` stays on (the
  assignment of the value a driver holds queues no transaction) for every kind, because it
  changes no count: the delta still happens, the process still runs, and no event exists
  either way. (2) The idle skip (`Config::idle_skip`) stays on for toggle coverage alone and
  is off when statement, branch, condition or expression counters are on
  (`LowerOptions` counters; same test as `counters_off` in `build.rs`): it leaves activations
  out, and the counters would miss them. (3) `observed_free` is now `--dap` and `--vhpi` only
  for the skip (`others_free` in `elab.rs`); the lean wide-signal events stay off with
  `--cov` (unchanged). Proof for (1): `cmp` of the databases (and the identical output) of
  the old and new binary over `tests/cli/cov_{run,toggle,edge,copies,fec,replay}.vhd`,
  `rtl_ff` (CYC=500), `rtl_lfsr` (CYC=1000), neorv32 single-core minimal 30 us,
  `sc5_fifo_repack`, `osvvm_axi4_random` and `uvvm_sb_slv`, for the kinds `toggle`,
  `statement,branch,toggle` and `statement,branch,condition,expression,toggle`, at 1 and 4
  threads, on `--tier` interp, llvm and auto: 216 pairs of runs, all byte-identical.
  Measured (interpreter, user-mode instructions in millions, 1 thread, before / after; plain
  run in brackets): default kinds: `rtl_ff` 3393 / 3385 (3077), `rtl_lfsr` 1790 / 1757 (1310),
  neorv32 30 us 14141 / 13644 (4855), `sc5_fifo_repack` 51891 / 50896 (41537),
  `osvvm_axi4_random` 8646 / 8654 (7274), `uvvm_sb_slv` 19396 / 19397 (12640). Toggle alone:
  `rtl_ff` 3103 / 3150, `rtl_lfsr` 1676 / 1651, neorv32 10046 / 6725, `sc5_fifo_repack`
  46067 / 43186, `osvvm_axi4_random` 7355 / 7362, `uvvm_sb_slv` 12632 / 12634. Statement and
  branch only: `rtl_ff` 3302 / 3294, `rtl_lfsr` 1454 / 1422, neorv32 12036 / 11541,
  `sc5_fifo_repack` 50441 / 49448. Only the designs whose processes drive many unchanged
  values gain; the idle skip needs toggle coverage alone and only fires where the process is
  pure. Pinned by `skipping_follows_observers` (`skip_signals` is 3 under every cov kind set,
  0 for a plugin, `'active`, `'last_active`) and `idle_skip_changes_nothing` (0 idle skips with
  counters, the plain run's number with toggle alone, same output).

## GUI (M10, #384, #388, user, 2026-10-19)

Design: `docs/design/gui.md` (section 12 lists the same decisions).

- **libsurfer as a git dependency pinned to a revision** (user, 2026-10-19), not a local
  checkout. Why: it is not on crates.io, and a pinned revision builds the same everywhere with
  nothing installed by hand.
- **`gui/` is its own Cargo workspace, EUPL-1.2, with its own CI workflow** (user, 2026-10-19).
  It depends on no yavs crate and talks only DAP. Why: libsurfer is EUPL-1.2, so the licence
  boundary is a process boundary, and the simulator's checks never build the GUI.
- **Live waves stream over DAP** into an in-memory store in the GUI, not a growing FST (user,
  2026-10-19). Why: the FST writer flushes late and patches its header, and reloading re-reads
  the whole file (`docs/design/gui.md` 5).
- **`egui_dock` for docking** (the user left it open, 2026-10-19; the design's recommendation
  stands). Why: tabs, drag to dock and saved layouts out of the box.
- **One release archive** (user, 2026-10-19): `yavs-gui` ships next to `yavs` in the same
  archive, with its licence as `LICENSE-EUPL-1.2-yavs-gui`, which applies to the `yavs-gui`
  binary only. Why: one download; the file name says which binary the EUPL covers.
- **`yavs run --gui`** (user, 2026-10-19) starts `yavs-gui --yavs <this yavs> -- <run
  arguments>`: `yavs-gui` next to the yavs executable, else on PATH, else `E0122` (exit 2); the
  exit status is the GUI's. A process start only, never linking. `--gui` conflicts with `--dap`.
- **`yavs-gui` command line:** `yavs-gui [--yavs PATH] -- <yavs run arguments>` starts `yavs run
  <arguments> --dap=stdio` (`yavs` from `--yavs`, else next to `yavs-gui`, else PATH);
  `yavs-gui --attach HOST:PORT` connects to a `yavs run --dap=PORT`. Closing the window sends
  `disconnect` and kills the child if it has not exited after 2 s.
- **G2 client state (#399):** the GUI owns the line breakpoint list and resends a file's whole
  set (enabled entries only) on every change and, for all files, before `configurationDone`; a
  breakpoint the server moved follows to the line it answered. Until the server sends structured
  time (P1) the GUI parses `"<reason> at <time>, delta <n>"` from the `stopped` description.
  Source files are read from disk by the path of a frame, or typed into the Source panel until
  `loadedSources` (P5) exists. Answers to requests of an earlier stop are dropped. Why: the
  server holds no list the client could read back, and the design keeps all state on the UI
  thread.

## Compilation cache (yavs-cache, yavs-sem `image.rs`, CLI, #407, user, 2026-10-19)

- **The "no incremental compilation cache" decision is lifted** (user, 2026-10-19: "you may disregard
  my no-cache decision"; goal: "cache as much as possible for our gain"). It was the basis of the earlier
  "so there is still no cache" statements in this file and of the `docs/plan.md` rows for `yavs-lib` and
  `yavs-llvm`; those now point here. The cost that motivated it (a short test run spends most of its CPU
  analyzing and elaborating libraries, #405) is the reason it is lifted.
- **Stage 1 stores the whole analyzed `Db` of a closure, not per-unit pieces** (yavs decision under the
  user's goal): the typed ids index arenas shared by all units (a type of one package is a `TypeId`
  that other units' expressions hold), so a unit's analyzed form is only meaningful together with the
  units analyzed before it, and units are analyzed in an order that fixes every id. An image of the
  settled `Db` after `analyze_closure` restores exactly the value a cold analysis produces, id for id,
  which is what makes a warm run byte-identical to a cold one; per-unit relocation of ids would need an
  audit of every mutation of earlier arena entries in `yavs-sem` and would have no such guarantee.
  Invalidation of dependents is implicit: the key covers every file of the closure.
- **Written by `elab`/`run`, not by `analyze`** (the analysis image; the analysed units of each library are
  a separate store that `analyze` writes, see "Analysed-unit cache"). `yavs analyze` loads a file's dependencies on demand in
  demand order; the closure that `elab`/`run` analyze is in dependency order, so the two histories are
  different `Db`s and an image from one cannot serve the other. Writing from `analyze` would also cost a
  second analysis per file. The first `elab` or `run` of a design notes it; the second stores the image
  ("Prefix images and the image on the second analysis", below); every later one hits.
- **Key and validation.** Key = sha256 over the cache format, the yavs build id (version, size and
  modification time of the executable), `--std`/`--relaxed`/`--psl`/`--encoding`/defines, the std/ieee
  tree, and the closure's files (library, path, recorded content hash, standard; the working directory
  for a relative path). The image also holds a digest of every source file read (std/ieee included; the text
  itself is not stored, see "Lazy image decode" below); the files of the closure were verified by the
  closure itself (content hash, or size and modification time, see "Per-run overhead" below) and the key
  holds their hashes, so on restore only the built-in files are compared again (by digest), and a file the manifests do not list cannot make an entry stale. Library
  manifests are never stored: they are read again on restore.
  Anything that does not match, or does not decode, is a miss and analysis runs from source; the cache
  can only make a run faster.
- **The image is taken after `yavs_elab::load_bodies`** (the bodies of every loaded package, std/ieee included), because loading them was a third of the per-run cost; a body error is reported with the analysis errors.
- **What is stored.** The arenas and tables of `Db`, the source map and the diagnostics (warnings replay
  identically); never an analysis with errors; never one with pending work. Transient analysis state
  (memo tables, the parsed-file cache) is cleared by `Db::settle` after the closure on both paths, so a
  restored `Db` and a cold one continue from the same state. Symbols are written as indices into a
  per-image text table and interned when first read (`yavs_syntax::encode_table`/`DecodeTable`).
- **Format.** `serde` with `postcard` (a maintained, specified, compact serde format), wrapped by
  `yavs-cache` in a header with magic, format version, key, length and an xxh3 checksum of the payload. Stored in
  `<work library dir>/yavs-cache/sem-<key>.img`; a write is a temporary file plus an atomic rename (no
  lock needed; the library lock rules of rule 14 are unchanged and the manifest is never touched by
  `run`); the 96 most recently used entries are kept; a directory that cannot be written is skipped
  silently. `--no-cache` and `yavs cache clean` (`E0123`) control it. A custom `$YAVS_STDLIB` disables
  the cache.
- **Not stored in the library but per user:** nothing yet. A precomputed std/ieee base shared by all
  designs is part of a later slice of #407.
- **Stage 2, step 1: the lowered run-time packages (`low-<key>.img`, #407).** Elaboration lowers the
  packages with run-time objects first (36 for a verification component testbench) and verifies and
  numbers that program: 4 to 6 ms of a 40 to 100 ms elaboration. `Lowerer::image` serialises the lowering
  state after the packages (`yavs-ir` `Program`/`ProgramBuilder`, `yavs-lower` `Cx`, `UnitIr`, `PackageIr`;
  serde and postcard as for the analysis image, ids unchanged, the in-memory forms untouched, the lazily
  optimised copies of functions, #427, and the predecoded interpreter form, #431, are built from it at
  run time as before) and `Lowerer::from_image` restores it, already verified; the lowering of the
  elaboration-time calls, the architectures and the rest goes on from there as it does from a cold
  lowering. Reading and decoding an image takes 2 ms for 490 KB.
  - *Key:* the analysis key (so: the sources, options and yavs build) plus the list of package units
    and the lowering options, in the same store (`yavs-cache` kind `low`). It holds for a `Db` of the
    extent it was made at (`Db::stage_extent`: the size of every arena and a digest of the source
    files read, #671; before that only for a `Db` that had not grown since the analysis, so a design
    that loads units while it elaborates had no entries): instantiating a generic during
    elaboration appends to the arenas and the packages' ids are then not those of the
    image, the key says so. The CLI hands the stage a
    `yavs_sem::StageCache` through the `Db` (`Db::set_stages`); a run without a store (`--no-cache`,
    custom std/ieee tree) is unchanged.
  - *When not:* with coverage counters; when the images would be under 256 bytes (packages with only
    constants); when the constant pool holds over 1 MiB of byte strings (reading that back costs more
    than lowering the packages, so nothing is stored). A periodic constant no longer counts: see
    "Compact composite constants" below (#446).
  - *Compact composite constants (#446):* a folded record or array-of-composites constant
    (`Fx::konst_comp`) is encoded as `(count, bytes)` runs, never written out: an array's repeated
    elements are one run, a record is its fields' runs with zero padding, a logic array of one value
    is one run of its 64-element plane word, and an element of several large runs repeats as those
    runs. The pool gets `ConstData::Repeat` or `Runs` when that is at least four times smaller than
    the bytes (`compact_runs`), else `Bytes`. One testbench library's 6.9 MB constant is now about
    12 KB, so its packages are cacheable. Readers: the interpreter expands a compact constant once per
    runtime on first address use (`const_bytes`), but `CopyConst` writes it straight to the copy;
    code that reads pool bytes at compile time uses `ConstData::read` (the optimiser's load folding),
    never the raw `Bytes`. Descriptors stay `Bytes`.
  - *Cold and warm take the same path:* the cold run verifies and numbers the packages' program before
    it stores, so a cold and a warm run lower everything after the packages from the same state.
- **Stage 2, step 2: the package state after initialisation (`init-<key>.img`, #407).** The
  initialisers of the run-time packages were 17 to 25 ms of a verification-component testbench's
  elaboration (one interpreted pass that allocates and fills the library's tables). The state they
  leave is plain data: each package's data bytes, the shared variables area and the shared heap
  (protected objects, access-type objects), whose handles are `u64`s naming heap slots. It is
  written as it is, slot by slot with generations, free list and write counts, so nothing is
  relocated and the precise roots of #422 (the shared-variables root map, the sealed set made at
  the start of the simulation) see the same heap as after a cold initialisation (`yavs-interp`
  `state.rs`, serde and postcard like the other images; blocks are packed as literals, runs of zeros
  and repeats of a pattern up to 64 bytes: the library's tables are megabytes of zeros and of one
  repeated record, 6 MB become 3 MB and a restore costs 5 ms instead of 25).
  - *Key:* the stage-1 key plus the lowering part (package list, options) plus the time resolution
    (the one run-time setting initialisers can see; the revision and the options are in the stage-1
    key). Generics of package instances are in the sources, so in the key; top-level generics and
    configuration cannot reach a package initialiser (no state of the design is read before the
    packages are initialised, external names in them bind later). Like `low` it holds only for a
    `Db` that has not grown since its analysis and only when the lowered packages are cached.
  - *Both paths use it:* the elaboration session's first initialisation (and each rebuild after an
    undone call) and the simulation start when no session ran (`init_packages` with a key). A
    design whose packages have an array signal with open bounds (`PackageIr::signals[].dope`) is
    not served at the simulation start: its descriptors are written between two initialisers
    (`fix`) and a later initialiser may read them.
  - *Reports are replayed:* what an initialiser reports (`report`, an ieee warning; the line
    `** Note: (init): text` on stderr, counted for the exit status) is recorded with the state and
    printed and counted again, in order, when the state is restored (`SharedState::replay_init_reports`),
    at the point where the initialisers would have run. A verification library's packages report at
    initialisation, so they would otherwise never be cached.
  - *Not cached, run every time:* an initialiser that did anything else but compute. Every effect is
    recorded while a package is initialised (`SharedState::tainted`, with the reason, shown by
    `YAVS_LOG=debug` as `no init entry: ...`): textio writes and file operations, `std.env` calls other
    than the constant ones (environment, clocks, assert state, call paths), the reflection natives,
    foreign calls (`--load`) and any native not on the pure list (`native::pure`, an allow-list: a new
    native is impure until listed). A heap holding a reflection mirror or sealed objects is not
    stored either. Coverage runs have no lowered cache and so none of this. A damaged or foreign
    entry is a miss.
  - *Warm == cold:* tests in `crates/yavs/tests/cache.rs` compare the run without the cache, cold and
    warm at `--threads 1` and `4` (stdout, stderr, exit status, waveform bytes) for protected
    objects, heap objects grown by elaboration-time calls, and the simulation-start path; editing a
    package replaces its state; reports of initialisers are printed again and counted; an initialiser
    that opens a file is never stored.
  - *Entries per design:* now three (`sem`, `low`, `init`), so the store keeps the 96 most recently
    used instead of 48.

- **Stage 2: the lowered design (`ir-<key>.img`, #479).** After the packages, elaboration lowers the
  architectures that own processes and every subprogram they reach, verifies and numbers the program
  (a library testbench: 530 functions, 12 to 18 ms of lowering and 5 to 7 ms of verification in a
  40 to 60 ms elaboration). The whole result is stored in one entry: the program, the `UnitIr` of the
  architectures, the package descriptions, and what elaboration asks of the lowering besides (the
  thunks of package variable ports, the functions of the elaboration-time calls, the shared variable
  slots, the user resolution functions and their types). A hit replaces lowering and `finish` of
  that run; decoding 2 MB takes about 3 ms.
  - *Key:* the analysis key, and the part that names every request elaboration made of the lowerer
    (the packages, what the elaboration session lowered after them, the unbound external names, the
    port actuals that get an equivalent process, the package variable ports, the logged calls, the
    resolution functions, the architectures), so the same requests on the same analysis give the
    same program, ids and all. Like `low`, only for a `Db` that has not grown since its analysis and
    never with coverage counters. A design whose constant pool is over 1 MiB is not stored (same
    reason as `low`). A run that resumes the elaboration session's program (package state left by
    elaboration-time calls) and one that does not have different keys; the hit keeps the session's
    start state, which the program extends.
  - *Verification:* a stored program is verified when it is stored (second run of a closure) and
    read back without it. A program that is not stored is verified function by function when
    `Program::func` first hands the function out, so the functions a run never calls cost nothing;
    that holds in release builds only: debug builds and runs with `YAVS_VERIFY_IR` set verify
    everything when the program is finished (tests keep their internal errors for broken IR).
    A function that fails the check at first use panics with the verifier message (the kernel turns
    it into an internal error).
    `ProgramBuilder::finish` sets `Program::lazy_verify` for this (it never did until #482, so release
    builds verified nothing). Measured at that fix, release build, pinned to one performance core,
    interleaved A/B, 3 runs: total CPU of the 30-case `vunit_vc30` benchmark 1.47 s before and after,
    `osvvm_axistream` 0.40 s to 0.41 s, `elab_heavy` 0.24 s both; the cost is inside the noise, so
    verification at first use stays on (user rule: broken IR is an internal error, never undefined
    behaviour).
  - *Optimised copies:* a hit that read the program but not the optimised copies would optimise the
    functions again (the optimiser was 20 to 25 ms of a small test, and the elaboration session had
    already done it for some). The entry therefore holds the optimised copy of every function an
    earlier run called, serialised one by one and decoded on the function's first use
    (`Program::export_optimized`, `load_optimized`; the copy is a function of the program alone, so
    it is what the run would make). A run that optimised functions the entry lacks writes the entry
    again at its end (`Elaborated::learn`); a run converges after one or two hits. Copies are not
    stored or used with `YAVS_NO_IROPT` or while a debugger keeps variables in their slots
    (`opt::copies_are_portable`).
  - *Entries per design:* `ir` is one more beside `sem` and `low`, so the store keeps the 96 most
    recently used entries instead of 48.

## Lazy image decode (#426, yavs decisions under the user's single-thread performance priority)

Decoding the whole analysis image (serde and postcard: about 5 MB of arenas, 3 MB of source text, 13 000
symbol texts) was the largest fixed cost of a run, and a run reads only part of the design. So the restore
decodes on demand:

- **Arenas decode 64 entries at a time on first read** (`yavs-sem` `ids.rs`). The image form of an arena is
  its entries in chunks, one postcard sequence per chunk, plus the chunk start offsets; a restored arena
  keeps the bytes (a range of the buffer the entry was read into, not a copy) and a `OnceLock` per chunk.
  `Index` decodes the chunk, `IndexMut` decodes it and hands out the entry, `push` appends after the lazy
  entries, `iter` reads every entry. Ids are unchanged, so a warm run is still the same value, id for id.
  Ceiling: an entry read decodes its 63 neighbours; a decode error cannot be returned from `Index`, so it
  is an internal-error panic (the entry was checksummed, a chunk that does not decode is a bug).
- **Whole-design scans must not run on the run path**, because they decode everything. The three that did
  (`'active`/`'last_active` reads, force and release statements, package-path external names) are answered
  from `Db::facts()`, found by `Db::settle` and stored in the image; `facts()` recomputes when the
  expression or statement arena grew since.
- **The source map stores no text**: per file the path, the line starts and an xxh3 digest. `SourceFile::text()`
  and `bytes()` read the file again on first use (built-in files from the binary) and check the digest. A
  file that changed since (same size and modification time, which the closure trusts, "Per-run overhead")
  or that cannot be read is never an internal error and never shown as stale text (user decision
  2026-10-20): the renderer (`SourceFile::try_text`) omits the snippet and keeps code, message, the
  `--> file:line:col` location, the labels as notes ("label at L:C: text") and a note "source of FILE
  changed since it was analysed; snippet omitted" (or "could not be read"), in the human, short and json
  formats (json gets no snippet lines in `rendered`; its spans are unchanged). Ceiling: without the text the
  column of such a location counts bytes, not characters. `text()`/`bytes()` stay for code that needs the
  text itself (re-scans in `yavs-sem`, coverage) and still panic for these files.
  The line starts stay, so source locations of run-time messages never read a file.
- **Symbols are interned on first read** (`DecodeTable`): the table is one string plus end offsets and an
  `OnceLock<Symbol>` per entry. Symbol numbers now depend on the order of first use (they are never
  compared across runs; elaboration is single-threaded).
- **Versioning.** The cache key holds the yavs build id (see "Build and switch fingerprint"), so an
  entry written by a build with another image layout is a miss, as before; nothing else reads old images.

## Per-run overhead (#426)

The fixed cost of one `yavs run` (a testbench suite starts one process per test case) is measured by
`tools/bench` (`vunit_vc30`, `--time-report`); the decisions that cut it, all yavs decisions under the
user's performance priority:

- **A source file is not hashed again when its size and modification time are unchanged** (yavs-lib
  `Stamp`). `analyze` takes the stamp before it reads the file and records it in the manifest
  (`size`, `mtime_ns`) only if the file had not been modified for two seconds (a coarse file system clock
  can give an edit of the same size the same time). `closure` skips the read and the sha256 of a file
  that still has both. Ceiling: content of the same size written back under the recorded time goes
  unnoticed (re-analyze), as with build tools generally. The manifest format stays 2: both fields are
  optional and an older yavs ignores them.
- **A manifest is parsed once per run.** `Library::read` keeps the parsed manifest of a library and
  returns it again while the manifest file has its size and time and is two seconds old; the library lock
  is still taken for every read. The closure and the restore of the analysis image both read each library.
- **A manifest is read by a parser for the text yavs writes, with `toml` as the fallback** (yavs-lib
  `parse_written`; superseded by the line reader of the index and the records, "Library layout", #599): the `toml` crate took 5 ms for the manifests of a testbench library and a run parses
  them at every start. The parser accepts only the exact shape `Manifest::to_text` produces (simple
  strings, no escapes, known keys, all required fields) and returns `None` for anything else, so
  other format versions, hand edits and unusual paths are read by `toml` with the same result and the
  same diagnostics; a test compares both on written manifests.
- **The restored image does not compare the closure's source files again** (`Db::from_image` takes a
  `verified` predicate): the closure verified them and the key holds their hashes. Only the built-in
  `std`/`ieee` text is compared.
- **Cache entries carry an xxh3 checksum of the payload instead of sha256** (format 2) and are read into one
  buffer without copying the payload out (mapped since #481, "Zero-copy cache entries"). Keys stay sha256. The checksum finds a damaged or truncated file;
  nothing here defends against someone who can write the library directory (they could edit the sources).
- **The library manifest is renamed into place without an fsync, and not rewritten when unchanged
  (#561, user, 2026-10-23).** Each `record` writes a temp file in the library directory and renames
  it over `yavs-lib.toml` (the replace-existing rename on Windows too): readers and a crash see the
  old or the new manifest, never a torn one. A power loss may lose the last `analyze`, which the next
  `analyze` records again (`run` reports L0034 or unit not found meanwhile). The fsync that `Session`
  did when it dropped cost 3 to 5 ms of wall per recording `analyze` (4.6 ms median, more than the 3.3 ms of
  CPU of the analysis); the analysed-unit cache and image entries were never synced. A record whose
  manifest text equals the file's is not written at all.
- **`run` does not free the design and the runtime before the process ends** (not with `--wave` or
  `--vhpi`, whose teardown is orderly): the operating system takes the memory back; for a large design
  it was a tenth of the run time.

## Native ieee operators (#395)

- **Replace by identity, not by name (user, 2026-10-19):** lowering turns a call of an `ieee`
  subprogram into a native call (`yavs.ieee.<key>`) when its mangled name (library, package,
  designator, parameter and result base types) reduces to a key `yavs_ir::ieee::parse` knows;
  the VHDL body stays in the library and is the reference. Implemented in `yavs-interp`
  `src/ieee.rs` on bit-plane vectors: `std_logic_1164` gates, `not`, `to_x01`/`to_x01z`/`to_ux01`,
  `to_01`, `??`, `rising_edge`/`falling_edge` of `std_ulogic`; `numeric_std` `+ - *`, the six
  relational operators (vector/vector, vector/integer, integer/vector), `to_integer`,
  `to_unsigned`, `to_signed`, `resize` and the local `xsll`..`xror` the shift and rotate
  functions call. Everything else (division, `?=`, string conversions, `numeric_bit`,
  Synopsys packages) still runs from VHDL.
- **Same results by construction of the test, not by fallback:** each native ports its body,
  including metavalue handling (a metavalue anywhere gives an all-`X` result), result bounds
  (`(n-1 downto 0)`, `(1 to n)`, the null constants `(0 downto 1)`) and warning texts, severities
  and conditions. `crates/yavs/tests/ieee_native.rs` runs generated testbenches with natives on
  and off (`YAVS_NO_NATIVE_IEEE`, any value, also for elaboration-time calls) and compares
  stdout, stderr and exit status. Reports of a native stop the process at the stop severity
  like `Inst::Report` (`Machine::ieee_stop`).
- **Known difference:** a run-time error inside a replaced body (only `to_integer` of a value
  beyond INTEGER) has the same message and exit status but no frames for the body in its
  `called from` trace. `--ieee-warnings` is still ignored, so natives always warn like the bodies.
- **math_real on `f64` (orchestrator, 2026-10-19):** `SQRT`, `EXP`, `LOG` (also with a base),
  `LOG2`, `LOG10`, `SIN`, `COS`, both `**`, `CEIL`, `FLOOR`, `ROUND`, `TRUNC` and `SIGN` use the
  platform's `f64` functions, as the reference simulators do. The bodies are series and Newton
  iterations, so a transcendental result can differ in its last digits; `CEIL`/`FLOOR`/`ROUND`/
  `TRUNC`/`SIGN` are exact ports, and every out-of-domain argument gives the body's report text,
  severity and returned value (`REAL'LOW`, `0.0`). `YAVS_NO_NATIVE_IEEE` restores the bodies.
- **Synopsys `std_logic_arith` helpers with a hand-back (orchestrator, 2026-10-19):** `plus`,
  `minus`, `unsigned_plus`, `unsigned_minus`, `mult`, `make_binary` and the `conv_unsigned`/
  `conv_signed`/`conv_std_logic_vector` conversions (the cost under the package's operators, and
  so under `std_logic_unsigned`/`std_logic_signed`) run natively for operands of `0 1 L H` in
  the shapes the operators produce. Anything else (a metavalue, other bounds, a null array, an
  overflowing integer) is handed back: the lowered call is `yavs.ieee.<key>@<body function id>`,
  the native sets `Machine::ieee_fallback` and `exec.rs` calls the body, so results, warnings
  and errors of those cases are the body's by construction.
- **Why not the existing `LNum`/`LogicOp` instructions:** they cover neither the bounds and
  warnings of the library functions nor `std_ulogic` tables per operand shape; the native call
  keeps one place that mirrors the library.

### Round 2 (#414)

- **Profile first:** a per-function instruction count of the benchmark set (`tools/bench` designs
  and the external ones) showed that after round 1 no `math_real` helper is hot any more
  (`ILOGB` and the series helpers were the cost of `log`/`exp`, now native); what is left of
  `ieee` is `math_real.uniform` (1 to 3% of the instructions of the random and the verification
  benches), `to_hstring` (3 to 6%) and the `numeric_bit` conversions and shifts of the local
  design testbenches (8 to 13%). Those are the first natives of round 2.
- **`numeric_bit` by the same natives:** a `bit` array is stored with the codes of `'0'` and `'1'`
  of `std_ulogic` (bias 2), so the `numeric_std` natives run on it unchanged. The key of a
  `numeric_bit` subprogram is the `numeric_std` one with `bit_vector` for `std_ulogic_vector`;
  lowering gives such a native a type argument (the type of its first parameter) so the native
  can name its warnings `NUMERIC_BIT.` instead of `NUMERIC_STD.`. There are no metavalues in
  `bit` arrays, so the metavalue paths never run.
- **Procedures and string results:** a native may be a procedure (`uniform`: seeds by reference,
  `x` out, the body's two ERROR reports); `to_hstring`/`to_ostring` return a string and hand a
  null vector back to the body (its index error is the answer). `shift_left` ... `rotate_right`
  replace the wrapper functions around `xsll` ... too: the null array constant for a null
  argument, else the shift.
- **Division and the Synopsys comparisons:** `/`, `rem`, `mod` of `unsigned`/`signed` (both
  packages) divide magnitudes as `DIVMOD` does (a `u64` fast path, shift-subtract beyond) and
  negate as the bodies do; a zero divisor is handed back to the body (its report and the index
  error after it). `is_less`, `is_less_or_equal`, `unsigned_is_less`, `unsigned_is_less_or_equal`
  of `std_logic_arith` run natively for `(n-1 downto 0)` operands of one length with `0`/`1`
  elements only (`L`/`H` compare differently in the bodies) and are handed back otherwise;
  `make_binary` now covers every result type. The vector/integer forms of `/` `rem` `mod`,
  `std_match`, the `?=` family and the `*_unsigned`/`std_logic_unsigned` wrappers stay VHDL:
  none of them was above 0.5% of the instructions of any benchmark.
- **Compiled tier:** `YAVS_IEEE_TIER=llvm` (with `YAVS_IEEE_SCALE=n` to shrink the straight-line
  testbenches, which take the JIT many minutes at full size) makes `ieee_native.rs` run the
  on/off comparison on `--tier=llvm`.
- **Reductions (#519, 2026-10-22):** the unary `and nand or nor xor xnor` of
  `std_ulogic_vector` (`std_logic_1164`) and of `unsigned`/`signed` (`numeric_std`) and the
  `*_reduce` functions of the Synopsys `std_logic_misc` are one native (`Op::Reduce`) that works on
  the plane words: the 1164 tables fold to "`0`/`L` decides `and`, `1`/`H` decides `or`, else `U`
  if any element is `U`, else `X` if any is not `0 1 L H`", and `xor` to "`U`, else `X`, else the
  parity of the ones", in any order. The bodies loop over the elements; a 512-bit `or_reduce`
  cost about 50 us in the interpreter.

## Skipped unchanged assignments (yavs-kernel, yavs-elab, #420, 2026-10-20)

- A zero-delay assignment (inertial without a rejection limit, or transport) that gives a
  range the value its driver and its signal already hold queues no transaction when the
  driver has nothing pending, the range has exactly one source and the signal allows it
  (`Signal::skip_ok`, `Config::skip_unchanged`). The cycle that would have applied it still
  happens (`SimControl::empty_delta`): delta numbers in reports, phases and `'last_event` are
  unchanged. A later assignment of the same run to that driver first puts the skipped ones
  back as real transactions (`Ctx::flush_phantoms`), so preemption is exactly the old one.
- Allowed signals: not guarded, not resolved by a user function, not in a composite group, not
  the source of an implicit signal, and not connected through ports to a signal that breaks one
  of these or through a conversion function (activity passes through ports and calls
  conversions and resolution functions). Static facts only, so the choice does not depend on
  `--threads`.
- Off unless `yavs_elab::reads_activity` finds no `'active` or `'last_active` anywhere in the
  analysed units (they are the only activity readers the kernel cannot see), and with `--dap`,
  `--vhpi`. (The edge triggers are off with `--dap` and `--vhpi` only; `--cov` keeps both the
  edge triggers and this skip, and the idle skip when it counts only toggles, #707.) A new
  observer of activity must switch it off.

## Remembered results of user resolution functions (yavs-elab, #420, 2026-10-20)

- The element-wise resolver of a user resolution function (one call per element, a scalar
  leaf) remembers the result per set of driving values, up to 16384 sets per signal. The
  function is pure (E1225), so equal inputs give equal results; a call that made a report is not
  remembered (the report must come every time), and nothing is remembered outside a run's
  update phase (initialisation reports at once). Resolution of composites as a whole and
  `std_logic` (plane-wise, native) are unchanged. Rationale: one interpreter call per element
  and delta cost 10 times a native resolution (`docs/design/performance.md` R3).
  The lookup replaces the plan's tables built at elaboration by probing the function: nothing
  to probe, nothing silenced, and it covers integer and larger types too.

## Resolution functions that touch shared state (yavs-elab, yavs-kernel, #392, 2026-10-20)

- A resolution function that reaches a shared variable of a non-protected type (accepted with
  `--relaxed`), a protected object or a file, however deep in its calls, runs with the shared
  state locked (all four reference simulators run it). Rationale: the LRM does not forbid it
  and a verification library logs through shared state from a resolution function.
- Such a function is not pure in effect: its element-wise results are not remembered, and the
  kernel (`Builder::serial_resolution`) never updates its signal in parallel, so the calls come
  in signal order at any `--threads`. Composites resolved as a whole were already updated
  serially.

## Default force mode of an external name (yavs-lower, yavs-kernel, #391, 2026-10-20)

- `force`/`release` without a mode through an external name: the mode depends on the named
  signal, which is known only at elaboration. Lowering emits `ForceKind::Default` and the
  kernel takes `out` for the formal of a port of mode out, inout or buffer and `in` otherwise
  (LRM 10.5.2.1, as for a port named directly). Without it the force was an `in` force of the
  child's port and never reached the connected net. The four reference simulators agree.

## Edge triggers for clocked processes (yavs-elab, yavs-kernel, #385, 2026-10-19)

- A process `process (c) begin if rising_edge(c) then ... end if; end process;` (or
  `falling_edge`, the IEEE 1164 functions; `c` a whole scalar signal, the only name in the
  sensitivity list, no `else`/`elsif`, not postponed) is recognised at elaboration and handed to
  the kernel as an edge trigger. The kernel does not resume it for an event that does not make
  the edge true: running it would evaluate the condition and suspend again, nothing else.
  Rationale: half the activations of a single-clock design are of that kind
  (`docs/design/performance.md` R2d).
- Off (`Config::edge_triggers`) with `--cov`, `--dap` and `--vhpi`: statement coverage, a
  debugger and foreign callbacks can see the skipped activation. Output is otherwise unchanged;
  the statistics count fewer process runs.
- More shapes (#420): the edge may be on a static element of a signal (`v(2)`, `r.f`, a
  generate index), the sensitivity list may hold other signals (they resume the process as
  before; the trigger only keeps a resumption without the edge from running it), the condition
  may be `rising_edge(c) and X` with the edge as the first operand (the predefined BOOLEAN
  `and` does not evaluate `X` after a false edge), and a process with one `wait [on ...] until
  E;` and no other suspension point (decided after lowering: `Process::several_waits`) is
  recognised in the same way.
- Asynchronous reset and the legacy edge (#420): `if R then ... elsif E then ... end if;`
  (no `else`, `R` an `=` test of a `std_ulogic` element against a literal, `E` as above) is
  recognised with the reset as part of the trigger: the process also runs whenever `R` holds
  at the event, so the reset branch runs for every event during reset exactly as before (no
  need to skip its unchanged assignments), and only events with neither `R` nor `E` are
  skipped. `x'event and x = '1'` (`'0'` for falling; either operand order; `x` a `std_ulogic`
  element) is an edge too, as an event ending on that value from any value (not the IEEE 1164
  function, which also needs the old value to be '0' or 'L'). Other literals ('H', ...) and
  non-`std_ulogic` signals are not recognised.

## One edge-test instruction for every clock-edge idiom (yavs-sem, yavs-ir, yavs-lower, yavs-interp, yavs-llvm, yavs-kernel, yavs-elab, #632, #589, #590, #591, 2026-10-23)

- **What:** `Inst::Edge { dst, kind, sig }` with `EdgeKind` `Rising`/`Falling` (the IEEE function:
  event, `To_X01` of the value and of `'last_value`) and `To1`/`To0` (`x'event and x = '1'`/`'0'`:
  event and exactly that value, whatever it was before). `sig` is one element of a `std_ulogic`,
  BIT or BOOLEAN signal. `yavs-lower` (`edge.rs`) makes it from every shape `Db::edge_shape`
  (yavs-sem `edge.rs`) knows: `rising_edge`/`falling_edge` of the IEEE function and of BIT and
  BOOLEAN, `x'event and x = lit` either way round, on BOOLEAN also `x'event and x` and
  `x'event and not x`, and the level of a `wait until x = '1'`/`'0'`. The elaborator's edge triggers
  use the same `edge_shape`, so the kernel and the code agree on what an edge is.
- **Why:** a spelling that is not an edge instruction costs more in every tier (the interpreter
  stepped `'event` out of its loop, ~380 instructions; the compiled tiers had to recognise call and
  attribute sequences), and a second backend would have had to recognise the idioms again.
  User rule 2026-10-23: every clock-edge idiom gets the same fast paths.
- **Look-alikes stay ordinary expressions:** no `'event` (a level), another signal in the compare
  (`x'event and y = '1'`: `Db::same_name` of the two mentions), `'event` alone, a user-defined
  `rising_edge`, `'stable(T)` with a time, `x'last_value` terms. Under `YAVS_NO_NATIVE_IEEE` the
  IEEE function runs from its body as before.
- **`wait until x = '1'`** becomes `Edge To1` only in a non-postponed process (not a procedure,
  which both kinds share) whose wait has no timeout and whose sensitivity is `x` alone as a whole
  object: it resumes on an event of `x` in that cycle, so "event and `x` is '1'" is the same test.
  A postponed process may resume in a later cycle than the event.
- **`not x'stable` is `x'event`** (yavs-sem, `Db::not_stable_as_event`): a stable time of zero is
  false in exactly the cycle of an event. The analyser makes the `'event` and does not declare the
  implicit signal (`attr::discard_implicit_signal`), which the kernel updated at every event of
  `x` (4.4 times the instructions of `rising_edge`, #590). `'stable(T)` with an argument keeps its
  implicit signal.
- **Known edge for the legacy forms** (#589, #591): a trigger of the `'event` form or the plain
  level of a `wait until` is now `ProcState::known` too (a key of signal, element and kind,
  `edge_key`; `EDGE_LEVEL` in the kind says "event ending on the level"), so `Ctx::edge_by` of that
  kind answers true without reading the signal. One word is compared (interpreter and compiled
  code), not three fields. `x'event and x` on BOOLEAN is an edge trigger now too.
- **Cost** (instructions per process per clock cycle, rtl_ff-style, 1024 processes, `--tier=interp`
  / `--tier=llvm`): every spelling on every type 1420 / 1115 (the wait forms 1410 / 1110);
  before: 1421 to 1860, `'event` 1798 to 1835, `not 'stable` 6490, BOOLEAN `'event and clk` 2711 /
  2045 interpreted / compiled.
- **Tests:** `crates/yavs/tests/edge_idioms.rs` (a self-checking design, 4 types x ~25 forms
  against an oracle spelled out in VHDL, every tier, edge triggers off, threads), `edge_ir.rs`
  (each form is one `Edge` of the right kind, look-alikes are not), `yavs-sem/tests/edge_shapes.rs`,
  `compiled_edges.rs` (no host call left), work-counter design `perf/edge_forms.vhd`.

## Memory of designs with many instances (yavs-elab, yavs-kernel, #421, 2026-10-20)

- **A plain run frees the elaborated model once the simulation is built.** `yavs run` without a
  plugin, waveform or coverage reads, after `build_full_once`, only the IR (run-time diagnostics
  with the VHDL call stack): `Elaborated::release_for_run` frees the hierarchy, net graph,
  process instances and per-instance values. Process names for reports come from the kernel
  (`Hook::process_name`), stored once. The debugger, plugins, the waveform recorder and coverage
  keep the model.
- **Small signal and driver data is stored inline** (yavs-kernel, `smallvec` with its `union`
  feature). A scalar signal took about 25 allocations and 580 bytes of record; buffers of up to 16
  bytes or one logic block, the per-element stamps (one buffer of four arrays), one waiter, one
  segment, one pending transaction and the scratch range lists live in the signal or driver, and
  larger ones spill to the heap unchanged. A resolved unguarded signal resolves its one run
  without lists, an assignment's scratch list is a local, and the elaborator hands the builder
  its already boxed process.
- **The shared heap in `SharedState` is boxed** (yavs-interp): each process carries the
  shared-state fields it fills in only while it runs; 120 bytes less per process.

## Narrow integer storage (R12; yavs-ir, yavs-lower, yavs-interp, yavs-elab, VHPI, waves, DAP, LLVM; #415, user approval 2026-10-19)

- **What:** an integer type whose range fits in 32 bits (2008 `INTEGER`, `NATURAL`,
  `POSITIVE`, user integer types) is stored in 4 bytes as an array element (`Ty::elem_size`,
  so `Ty::layout` strides) and as a kernel signal element, scalar signals included
  (`yavs_elab::elem_of`, `yavs_lower::narrow_elem`). Scalar variables, constants, parameters,
  generics and record fields stay 8 bytes. Physical types, reals and 2019 `INTEGER` (64-bit)
  are unchanged. Registers stay `i64` and arithmetic stays checked `i64` (rule 7, reworded).
- **Why:** integer arrays (memories, scoreboards, `integer_vector`) took twice the memory and
  cache they need (`docs/design/performance.md` R12).
- **How values stay the same:** every store into a narrow cell comes after the range check of
  its subtype, which is inside 32 bits; `Width::B4` loads sign-extend, as does every reader of
  a 4-byte integer cell (natives, VHPI, waves, DAP, reflection, binary files, elaboration
  values). A 4-byte load reads either form of a narrow integer (little-endian, the value fits),
  which kernel attribute reads use. A narrow element passed to a scalar `out`/`inout` formal or
  native goes through an 8-byte temporary (copy-in/copy-out). Conversions between closely
  related arrays with different element types were already element-wise.
- **Unchanged outside the process:** the binary file format (an integer is still written in the
  fewest whole bytes its range needs), C layouts of foreign arrays and records, and the VHPI
  value formats.
- **Not done (step 2):** 1- and 2-byte elements from narrower subtype ranges need the element
  subtype in lowering (elements are lowered from the base type today) and would clash with
  1-byte enumerations in the kernel; only if step 1 measures worthwhile.

## Variables in registers; a debugger keeps them in memory (yavs-ir `opt.rs`, yavs-interp, CLI, #444)

- **What:** the optimiser keeps a scalar frame slot of a function or procedure that cannot
  suspend in a register when only direct loads and stores reach it (`Opt::promote`). It also
  drops `Stmt` markers of statements that allocate no temporary memory, merges registers
  holding one constant, and renumbers the registers densely. A slot inside a named variable is
  promoted only while no debugger runs: `yavs_ir::opt::keep_variables` (called by the CLI for
  `--dap`, and by `DebugSession::new`) turns off the promotion of named variables and the
  dropping of markers, before any function is optimised.
- **Why:** the loops of verification libraries loaded and stored the same slots on every pass
  and executed a marker per statement (a quarter and a ninth of the executed instructions of
  the scoreboard testbench). A debugger reads and writes variables in their slots and stops at
  markers, so it needs them; nothing else reads a subprogram's slots (VHPI, waves and coverage
  see processes, signals and counters).
- **Not promoted:** processes and anything that may suspend (a register is not live across a
  suspension), functions with nested subprograms (they reach the frame through static links),
  slots whose address is passed, indexed or copied, slots covered by a composite root map or
  written while a root map covers them. A slot a root map covers that the function never writes
  (a parameter) is read from a register and keeps its value in memory for the collector.
- **Logic arrays, lowering side (#444):** a default value or an `others` choice for a logic
  array is one `LFill` (the instruction sets whole plane words), not a loop of `LSet`; the
  equality that `case` choices and `array_cmp` ask for on logic arrays compares the planes
  with `LEq` after the lengths; the value of an array literal or aggregate constant is read in
  place from the constant pool (as string literals always were: the runtime expands a compact
  constant once) instead of being copied into a temporary first. The verification libraries'
  command records and `case` statements over `std_ulogic_vector` ran thousands of
  instructions per default value and per choice.
- **For a future cache of optimised code:** the key must include whether `keep_variables` was
  called; today only unoptimised IR is cached.

## Local composite variables live in the frame's temporary memory (yavs-ir `opt.rs`, yavs-interp, #444)

- **What:** the storage of a local array or record variable (the `[data, dope, handle, descriptor]`
  slot over a heap object) of a function or procedure that cannot suspend becomes `Inst::NewFrame`:
  the object `[descriptor][value]` is allocated in the machine's temporary memory and the frame's
  temporary base is raised over it, so the frame's own `Stmt` markers keep it. The handle slot
  stays null. Only the optimiser makes it (`Opt::frame_objects`), only for objects made once per
  call (not on a cycle of the flow graph), of types without handles, whose handle nothing uses
  but the `Deref` and the store into the variable's handle slot, never while `opt::keep_variables`
  is set (a debugger).
- **Why:** such a variable cost a heap allocation, a copy, a table entry and a share of every
  collection per call (2.5 million objects, 12 percent of the queue/scoreboard testbench).
  Nothing can outlive the call that holds a pointer into the memory except a result, and a
  result in temporary memory (a concatenation, a string returned by a native) already lives only
  until the caller's next statement; a callee's frame leaves its temporary memory in place on
  return, so a returned pointer into the variable is valid exactly as long as the other results.
- **Not changed:** objects of a process, of a shared variable, of a subprogram that may wait
  (a suspended frame keeps its temporaries, but the rule is kept simple) and allocators
  (`new`), which stay heap objects.

## Written root slots may live in registers (yavs-ir `opt.rs`, #444)

- **What:** a frame slot that a root map covers as a handle or an address, and that the function
  writes, is promoted to a register (`Opt::promote`) when every access has exactly the register
  type of the root: `Handle` for a handle slot, `Ptr` for an address slot. Before, only slots
  the function never wrote (parameters) were read from registers.
- **Why:** the collector finds the `Handle` and `Ptr` registers live at each safepoint of a frame
  (`roots::live_refs`), so a live register of that type is a root as the slot is, and a register
  that is no longer live keeps nothing alive that nothing reads. The pointer to an aliased
  parameter and the access variables that walk a linked list were loaded from their slot on every
  iteration (12 and 5 million loads in the queue/scoreboard testbench).
- **Not changed:** slots under a composite root, slots with an integer-typed access (the
  collector would not see the register), everything while a debugger runs.

## Interval analysis, jump threading and dead register definitions (yavs-ir `opt/range.rs`, `opt/live.rs`, yavs-interp `pre.rs`, #444)

- **What:** in a subprogram that cannot suspend, the optimiser finds an interval for every integer
  register at every instruction (forward dataflow over the blocks, hull at joins, widening after
  three visits, the compared registers narrowed on each edge of a branch on a comparison) and
  uses it to drop a `Check` it proves, turn a `BinRange` that cannot leave its range or overflow
  into a `Bin`, replace a `Cmp` it decides by a constant, fold a `CondBr` on a decided condition,
  and send an edge to a block that is only a `CondBr` straight to the successor the interval picks.
  A liveness pass then removes pure definitions of registers defined on several paths that no path
  reads (never of a register defined once: a frame starts with the value of a register defined
  once by a constant or an address, and merged constants rely on it). What is left of an
  unreachable block jumps to the block after it, and the predecoder (`pre.rs`) runs a forward
  `Br` to where execution goes on anyway as no operation.
- **Why:** lowering checks every subtype conversion and incremented counter and selects on
  `n = 0` for every concatenation, all decided by "a length is not negative, a checked counter
  stays in range". Arithmetic overflow is a runtime error, so the continuing path only holds
  results inside `i64`; that is what makes the intervals sound. Executed instructions of the
  queue/scoreboard testbench fell by 13 percent.
- **Not changed:** processes and anything that may suspend, functions without a loop or of fewer than 40
  instructions, or over a size budget (registers times blocks above 2^15), every check whose
  interval does not prove it. Both passes cost optimiser time at the first call of every
  subprogram, and most of a library runs a few times: without the loop condition they added 15
  ms to a 200 ms verification-component test case and 25 ms to a small testbench of the second
  verification library (a third of the optimiser's time, #444 comment). The analysis tracks only
  the registers a check, compare or branch depends on, and the liveness pass only registers
  defined on several paths.

## Intervals and register promotion in processes (yavs-ir `opt/range.rs`, `opt.rs`, `verify.rs`, #444)

- **What:** the interval analysis and the promotion of scalar frame slots to registers also run
  on processes, per activation. After a suspension (`Suspend`, a call that may suspend) the
  registers are unknown except the constants a frame starts with, and so is every tracked
  variable slot except for the interval that all its stores, and its zero start, keep it in:
  the analysis iterates the solution until the stored intervals stay inside the assumed ones
  (widening to the full range after two rounds), so a variable that only ever holds 0 to 5
  stays in 0 to 5 across a `wait`. Branch edges narrow registers as before; a slot is tracked
  when every access is a direct 8-byte integer load or store and its address never escapes.
  A slot is promoted to a register when no suspension leaves it live (some path from the
  suspension reads it before writing it): loop variables and variables a loop works on between
  two waits. Handles and addresses stay in their slots (the collector finds them there).
  Only processes with 40 or more instructions and a loop that does not pass a suspension are
  analysed (the outer loop of a process is not a loop of work).
- **`wait;`:** a process that registers no wait condition anywhere (no `WaitOn`, `WaitFor`,
  `WaitAt`) is never woken: the code behind its `Suspend` does not run, which makes its variables
  dead at the suspension and lets the analysis start the statements from the declarations. The
  verifier knows the same (`undefined_uses` stops at such a `Suspend`), because the back jump
  behind it reads registers that only the declarations define.
- **External readers and writers:** a plugin can deposit into process variables between two
  activations (and before the first, at the start of the statements), and the waveform recorder
  with `--wave-variables` reads them from their slots. `opt::external_variables` (the CLI calls it
  for `--vhpi` and `--wave-variables`) keeps named variables in their slots, makes them unknown
  after every suspension and at the start of the statements, and neither stores nor loads
  optimised copies of processes (they are made under that assumption). A debugger
  (`opt::keep_variables`) keeps named variables in their slots as before.
- **Why:** a loop in a process (`for i in 1 to N loop x := (x * 75 + 74) mod 65537; ...`) loaded and
  stored its variables on every pass and checked every operation: with the facts above the
  range-checked operations become plain ones, and the loop touches no memory. Check failures keep
  their message, statement and time (a check goes only when the interval proves it).
- **Not changed:** a variable that is live across a wait stays in memory (its value must
  survive, and the interval is the invariant over all stores); loop variables bounded only by
  another variable are unbounded for the analysis (no relational facts); the liveness pass for
  dead definitions (`opt/live.rs`) stays on subprograms.
- **Memoisation of instance-constant values:** measured with the interpreter a copy wins at every
  size tried (a block that saves two instructions net of the flag test still gave 2 to 5 percent
  fewer cycles), but compiled with `--tier=llvm` a block that saves five net loses about 2
  percent (more code to compile, loads of the slots for work that costs a few cycles native). The
  gain needed per copy is therefore 3 net of the flag test and the slot loads, and 6 when the
  whole design is compiled (`opt::compiled_tier`, called for `--tier=llvm`; copies made then are
  not stored). `--tier=auto` keeps the interpreter's value: the functions it compiles run
  interpreted first, and the measured difference there is below the noise.

## Clean-up of finished subprograms: threading through short-circuit tests, slice emptiness tests, coalesced moves (yavs-ir `opt/cfg.rs`, #444)

- **What:** after the optimiser's walks and before registers are numbered, a linear pass over the finished code of a
  `function` or `procedure` that cannot suspend (`YAVS_NO_CFG=1` turns it off for differential testing):
  (1) a block that ends in a jump to a small block (up to three effect-free instructions, `Move`, `Const`, `Cmp`,
  `Bool`, `Select`, then a `Br`, `CondBr` or `Ret`) takes that block's code over, with constants and copies tracked
  through the straight-line code: `t := a; if t then t := b end if; if t then ...` becomes a branch on `a` and a branch
  on `b`, a `CondBr` on a known constant becomes a `Br`, and an edge into a block that only tests the same register again
  goes to the arm it picks; registers defined once and read only in the copied block get fresh names in the copy
  (the predecoder fuses a compare with its branch only when the compare is defined once and read once);
  (2) the emptiness test of a slice, `Cmp Eq|Ne|Gt|Le (DopeLen d) 0` with `d` a fresh `MakeDope` of a constant direction
  whose bounds were not overwritten, becomes a comparison of the bounds; (3) pure definitions no path reads are removed,
  including the length, the descriptor write and the allocation of a slice nobody reads any more (only when no copy of
  the block's address can exist before the last reader: the readers between the allocation and the next one are
  exactly `MakeDope` and `DopeLen`); (4) `t = a op b; p = t` with nothing else reading `t` and nothing touching `p` in
  between writes `p` directly (the move is what remains after promotion when a range check between them was dropped
  later). Blocks that no branch reaches any more are reduced to a jump to the next block, as `kill_unreachable` does.
- **Why:** per-character library loops (`and`/`or` chains over the character class, `s(i to i+1)` tests) spent a third of
  their operations on flags kept in registers only to be tested by the next block, and on building a descriptor to
  compare its length with zero. Executed interpreter operations of the scoreboard testbench fell by 9 percent.
- **Invariants it keeps:** a register defined once by a `Const`, `ConstF` or `AddrOf` is a value the frame starts with
  (other blocks read it without having run the definition): such a definition is never copied (the copy would make
  the register defined twice and the frame would no longer start with it) and is kept, at the entry, when its block
  becomes unreachable. A block followed by its step block (`Cmp`/`CondBr`, then `Bin add|sub`, `Br`) is not copied: the
  predecoder makes one operation of the loop latch only in that order. Nothing is done to processes, functions
  that may suspend, or with a debugger attached beyond what the walk did (no `Stmt`, `Cover`, store or check is
  removed or moved; the copies keep their instructions' statements).
- **Cost:** threading and the slice test run only in functions with a loop (a branch to a block at or before its own);
  the rest of a library runs a few times and the optimiser's time counts at every subprogram's first call. A small
  verification-component test case executes 0.6 percent more instructions with the pass (7 million of 1.26 billion).
- **Not changed:** loop-invariant code motion, a layout that lets more jumps fall through, passing arguments in
  registers (the compiled tier reads the frame slots).

## Registers share cells by live range (yavs-ir `opt/share.rs`, #478)

- **What:** the last step of the optimiser (`renumber`) computes the liveness of every register over
  the finished code, gives each a hull of instruction indices, and hands out frame cells by linear scan,
  one cell per simultaneously live value. Cells are numbered in order of first use. Never shared: signal
  references (two adjacent cells), registers a frame starts with (defined once by a constant or an
  address: the interpreter preloads them and a merged constant is read where its definition has not run),
  and a compare, index, position, layout or signal-value temporary defined once and read once soon after
  (the predecoder fuses such pairs by counting the reads of a register; a shared cell would stop it, which
  made the first version 2 to 5 percent slower). Classes never mix: `I64`, `F64`, `Ptr` and `Handle` cells
  stay separate, so the collector's register types (#422) and the must-defined check (#430) hold on the
  result; the verifier runs on it in debug builds. A value defined by a simple one-result instruction may
  take the cell of a value that instruction reads last; nothing else touches at a point.
- **Why:** frames of the library testbenches had about 235 cells after dense renumbering, a copy of the
  frame template per call and a cold stack. Sharing takes the average frame of the verification-library
  benchmarks down by a fifth to a quarter; what remains is mostly preloaded constants and addresses.
- **Not changed:** the number of instructions executed; processes and subprograms that may suspend are
  treated alike (no register is live across a suspension, so their hulls never span one).

## Equality of arrays of integers and enumerations compares bytes (yavs-lower, yavs-interp, #444)

- **What:** `=` and `/=` of one-dimensional arrays whose elements are integers, physical or
  enumeration values (not logic arrays, which compare planes) test the lengths and then run one
  `Inst::MemEq` over the bytes, instead of a loop of `Pos`/`Index`/`Load`/`Cmp` per element. Ordering
  comparisons keep the loop.
- **Why:** these values are equal exactly when their stored bytes are; the key comparison of the
  data-types dictionary tests spent its time in that loop.

## Floor of a run: freeing the analysis (#426, yavs decision under the user's single-thread performance priority)

- **`Db::release_analysis` frees the arenas on a thread of its own.** Dropping every node of the analysed
  design one by one took a fifth of a short run (#409 freed them for the peak, the drop sat in the run). The
  arenas are moved to a detached thread; a failed spawn drops them in place. Peak memory is unchanged.

## Zero-copy cache entries (yavs-cache, yavs-diag `shared.rs`, yavs-ir, #481 item 2, #407, yavs decisions under the user's single-thread performance priority)

A warm run of a short test spent 3% (small UVVM testbench) to 17% (one VUnit component test; 25% over the 30 test
cases of the `vunit_vc30` benchmark) of its CPU on the cache entries: reading the file into a buffer, copying that
buffer once more for the stages (`StageCache::get`), and decoding. Decoding was 14% of the 30-test suite. Decisions:

- **An entry is mapped, not read** (`yavs-cache` `map.rs`, `memmap2`, the crate's only `unsafe`; a file that cannot
  be mapped is read instead). `Store::get` still validates before it hands anything out: magic, format, key, exact
  length, xxh3 checksum of the payload; a damaged, truncated or empty entry is a miss. The mapped bytes do not change
  while a run holds them: a writer renames a new file over the entry and a prune or `yavs cache clean` removes the
  name, which on Unix leaves a held mapping valid (on Windows, removing or replacing a file another run has mapped
  fails and is ignored by the writer, as for an open file before). Something outside yavs that truncates an entry in
  place could make a read of it fault; nothing in yavs does.
- **No copy between the store and the decoders.** `yavs_diag::Buffer` (a shared immutable byte buffer, here the mapped
  entry) is what `StageCache::get` returns and what `Image` holds. `yavs_diag::Slice` (moved there from the analysis
  arenas) is a byte string deserialized as a range of the buffer set by `yavs_diag::sharing`, else a copy. The
  packed blocks of the `init` entry are read in place and the stored optimised copies of functions (`OptCopy`)
  are ranges of the buffer.
- **Function bodies are decoded when a function first runs.** A `Program` writes each function as its header and
  the body (`code`, `loc`, `operands`, `resume`) as one byte string. Read through `yavs_ir::lazy_bodies` (the `ir`
  entry and the lowered packages, `low`), the body stays bytes in the buffer and `Func::full` decodes it once, when
  `Program::func` hands the function out or the optimiser or the verifier needs its raw code (a restored function
  that has a stored optimised copy never decodes its own body). Without `lazy_bodies` (the builder's own use, tests)
  the bodies are decoded with the program. The format is the same either way, and a program that was read lazily
  writes back the bytes of the bodies it never decoded. Code outside `yavs-ir` reads `Program::func`, never the
  `code` of `Program::funcs`: `funcs` entries of a restored program may hold their header only.
- **Not done, because it did not pay or does not apply:** rkyv or another archived layout of the analysis image (the
  arenas already decode a chunk at a time and the rest of its cost is the symbol table and the library re-read, 1 ms
  of a 100 ms test); unpacking the `init` entry (6 MB of heap bytes from 3 MB) is a memory-bandwidth and page-fault
  cost that a mapping would only move; a copy-on-write mapping of the heap is not attempted.

## Prefix images and the image on the second analysis (#426, #407, floor of a run, yavs decisions under the user's single-thread performance priority)

A suite runs one `yavs run` per test case and every test case is a new closure, so each run was a cold
analysis of the library packages it shares with all the others (0.14 s of a 0.26 s small test of a verification library).
Decisions that cut that, with the measured effect in `docs/STATUS.md` "floor of a run":

- **A closure that misses continues from the longest cached prefix of its files** (`crates/yavs/src/cache.rs`
  `bounds`). The files of a closure are in dependency order, so the testbenches of one library start with the
  same files. The places where the library of the file changes (at least 8 files in, 8 files apart, at most
  3) are the candidate ends of a prefix. `Db::from_image` restores the prefix image of the longest one that
  exists, the rest of the files are analyzed on top of it, and every candidate end beyond the restored one
  gets an image of its own (kind `pre`, key = the analysis key of the prefix's files). The entry is stored in
  the cache of the library of the prefix's last file, because every closure with that prefix maps it; the work
  library's cache is per testbench. Ceiling: a prefix is found only at a library change, and a shared library
  directory keeps the usual 96 entries.
- **The state at a prefix end is the same whether it was restored or computed**: the analysis of the files
  up to the end, then the bodies of the packages outside the closure's own libraries
  (`yavs_elab::load_bodies_in`: `ieee`, and the generic package instances whose generic body is analyzed
  already, in creation order up to the first that is not; never a unit of the closure loaded out of its
  compile order), then `Db::settle`. A closure's `Db` therefore depends on its files and the ends
  only, not on which images happened to exist, which the `low` entries (keyed by the analysis key and the
  arena sizes) rely on.
- **`Db::settle` finds the facts incrementally**: the facts of the first entries of the arenas are kept and
  only the entries added since are scanned (`Db::scan_facts`), so settling a restored prefix does not decode
  it. Analysis never changes an entry after the unit that made it is done.
- **The image of a closure is made on its second analysis.** The first run of a closure writes an empty note
  (`Store::note`, `<kind>-<key>.seen`, newest 512 kept, removed by `yavs cache clean`) and makes no image and
  no lowered-package entry; the run that finds the note makes both. A run that happens once (a test case of a
  suite) no longer pays the serialisation (a fifth of its run); a design run twice or more has its image from
  the third run on. Supersedes "The first `elab` or `run`
  of a design stores the image".
- **A prefix image is made when the second different closure starts with its prefix** (#549, supersedes "made at
  once: they are shared"; `Store::sightings`, group = the prefix key, member = the closure's key, and one maker at a
  time through `Store::claim`). Why: with the analysed-unit cache a closure's first run replays its units cheaply, and
  the three images (4.7 to 5.6 MB, 11 to 17 ms of CPU each: `to_image` serialises the whole `Db` again) were a third
  of a cold run of a verification-library testbench (`osvvm_axis_random` 143 ms, 43 of them images; `uvvm_sb_slv` 75 ms,
  11 of them), for entries that only a later closure reads. The first closure of a suite pays nothing, the second pays
  what the first did, every later one restores. A suite of one closure per prefix never writes an image and replays its
  units every run (what a cold run does anyway). The claim also stops testbenches that start together from each writing
  the same three images. The library stages are unaffected: they need four closures (`libstage::SIGHTINGS`), so the
  image exists by then.

## Library stages: what a closure shares with others that start with the same files (#407, yavs decisions under the user's single-thread performance priority)

A suite that runs one test per process gives every test a new closure, so the closure-keyed entries (`low`, `init`,
`ir`) never serve a run that happens once; only the analysis prefix (`pre`) did. The library under such tests is the
same files in the same order, so what is lowered from it can be shared too. Three more kinds are kept in the cache of
the library of the prefix's last file, next to its `pre` image; the key of that image names the files, the options,
the revision and the build. A closure uses it on every run while the library is made
(#407, run twice and three times in a row, 2026-10-21): the closure's own `sem`, `low`, `init` and `ir` entries repeat
the library (5 MB of analysis and 10 MB of IR each), were slower to read than the shared ones, and 68 closures of a
library directory writing four of them each pushed one another out of the 96 entries, so a suite's second run paid
for writing them and its third run for writing them again (a quarter more CPU than the first run). A closure under
a prefix whose `lib` entry exists (and is not the empty "cannot be lowered ahead" note) gets no entries of its own;
a closure under a library that is not made yet (fewer than four closures have asked) makes its own on the second
run, as before, and keeps its `ir` compact. Ceiling: a closure the made library does not serve (a package body
follows the prefix) lowers its packages every run. The run that makes the entry gives the heap back afterwards
(`yavs_cache::trim_heap`, `malloc_trim` on glibc): the two lowerings of the library hold 60 MB that the
allocator would keep resident through the rest of the run (peak 224 MB of a cold `.pro` suite process to 181 MB).

- **`lib`: the lowered library.** The packages' objects, then every subprogram the packages declare, lowered in a
  fixed order from the prefix's own image (`StageCache::prefix_db`, never from the closure's `Db`, so that the result
  depends on the prefix alone) and stored with the lowering state (`Lowerer::image`). The ids it hands out (functions,
  types, constants, natives) are the same in every closure of the prefix, which is what lets the next two kinds name them.
  Subprograms that cannot be lowered ahead of any design are left out, found by lowering everything once
  (`Lowerer::context_dependent`): those that read a `'path_name` or `'instance_name` (the name is a cell of the instance
  data of the architecture being lowered), those that use an external name (they fail on entry until it is bound), those
  that fail to lower, and every subprogram that calls one of them. They are lowered by the closure that reaches
  them, as before. Nothing is stored when external names are involved or the lowering is under 256 bytes; an empty
  entry says "cannot be lowered ahead" so that no run tries again. The header names the package units, the function
  count, the bytes of the shared variables area and a digest of the ids (`ProgramBuilder::fingerprint`: the identity of
  every function in id order and the size of every table; the image's bytes are no digest because maps are written in the
  order of the writing process). The next two kinds are keyed by that digest, and a run that reads an entry whose digest
  is not the one in its header does not use it.
- **Reused only when provably the same:** the prefix key equals; no package body follows the prefix whose package is
  inside it (`libstage::closed`: such a body completes subprograms and deferred constants the prefix declared, and a
  prefix lowered without it could differ); the closure's `Db` has not grown since its analysis (the same rule as `low`);
  the packages of the closure that belong to the prefix are the packages the entry lowered, in the same order; the
  digest matches; no coverage counters; not `--tier=llvm` (it compiles every function, and the library has thousands);
  the lowering options are in the key. A closure that fails one of these lowers as before.
- **When it is made:** the first run of every closure that asks for a prefix and finds no entry is recorded
  (`Store::sightings`: one empty `.seen` file per different closure, newest 512 kept); the run that makes the count
  four makes the entry, at the longest prefix of the closure only, and one run makes it at a time (`Store::claim`,
  20 s; the others go on without). Making it costs about 0.35 s of CPU (two lowerings, one to find what depends on
  the design, and the image) and 10 MB of disk for a library of 3500 subprograms, and the first run of a closure saves
  about 40 ms, so a library under fewer than about ten closures costs more than it saves; waiting for four is the
  hedge (ski rental: a suite with that many closures of a prefix loses three runs' savings, one with fewer pays
  nothing). Measured: on the second library's `.pro` regression (314 tests, 11 prefixes of 28 closures on average, four
  jobs) creating on the first sighting costs 0.4 to 1.7 s of wall and 1 to 3 s of CPU more than creating on the second,
  and the second and the fourth are the same within noise (17.3, 17.8, 17.2 s against 18.0, 17.5, 17.0 s); on the
  first library's suite (one library directory per component, 137 runs of 1 to 16 closures per prefix, three prefixes
  with two closures and one with sixteen) the second sighting makes four libraries and the fourth one.
- **Decoding it:** the entry is mapped and the bodies of its functions stay bytes until a function runs ("Zero-copy
  cache entries"), so a library of thousands of functions costs its function headers and tables (about 5 ms of a
  100 ms run) and a body only when its function is called, and not even then when it has a stored optimised copy.
  Decoding the 10 MB of bodies eagerly cost 26 to 38 ms, more than lowering what a test reaches, so this is what makes
  the entry pay. A debug build verifies the program when it is stored; release builds do not verify functions (#482).
- **`libopt`: the optimised copies of library functions.** A function's optimised copy reads only the function, the
  flags and kinds of the functions it calls and the type, constant and case tables, all of which are below the
  library's counts, so a copy of a function with a library id is the same in every closure. A run that made copies the
  entry lacks writes the entry again with them (read, merge, write; two runs that do it together may lose one's
  additions, which the next run makes again). A closure that runs once pays the optimiser for the library functions it
  calls: this is 40 of the 80 ms of the simulate phase of a small test.
- **`libinit`: the state of the first packages after initialisation.** Those the library lowered, up to the first with
  an array signal whose descriptor `fix` writes at the simulation start, restored like `init` (data, the first bytes of
  the shared variables area, the shared heap) and the rest initialised after them; the state of a package depends on
  the packages before it and on nothing after it, and a later package's initialiser cannot change what an earlier one
  saw. Everything `init` refuses (`tainted`, sealed or reflected heap objects) is refused here; reports are replayed.
  The session of elaboration-time calls uses it too (it initialises the packages first).
- **A package body in a file of its own follows its package** in the order of the closure (`yavs-lib` `closure`): the
  depth-first order put such a body (a settings package whose body is given separately) after everything else, which
  made it a body that follows every prefix and kept the library from being shared at all. The body's file now follows
  its package's file as soon as everything the body needs is placed (it is hoisted only when all its order edges are
  done, so no cycle can arise). A body that needs a unit placed later stays where it was, and the stage is not used for
  a prefix it completes. The order of files in a closure changes for designs with such bodies (the elaborated
  design does not: package bodies are not the "last architecture"); difftest, the analysis corpora and the external
  suites are unchanged.
- **Entries per directory:** `lib` (about 10 MB for a big library), `libopt` (under 1 MB) and `libinit` (a few KB) per
  prefix, with the prefix's `pre` image (5 MB): the 96-entry limit now has a byte limit beside it, 512 MiB per library
  directory, the least recently used go first and the newest stays (`yavs-cache` `MAX_BYTES`). The `.pro` regression
  (11 prefixes) leaves 199 MB of cache in all, 25 MB more than before (the library under eight suites holds 133 MB:
  eight prefixes of 10 MB, 1 MB and 5 MB of state and copies, 5.6 MB of analysis each; 16 MB, 11 MB and 5 MB in the
  others).
- **Ceilings:** one `lib` per longest prefix (suites with several prefixes of one library lower the library once for
  each; sharing between prefixes would need a program delta); a design that instantiates a generic during
  elaboration (the `Db` grows) lowers as before; a library with a subprogram that cannot be lowered ahead of the design
  keeps it for the closure (it is skipped, not the library).

## Kernel round 5: the cost of a wake-up, second pass (yavs-kernel, yavs-interp, #440, 2026-10-20)

- **A zero-delay assignment of one `std_ulogic` element to a driver with nothing pending or skipped
  is appended to the driver's waveform directly** (`Ctx::assign_code`): nothing to preempt, no
  waveform list, no value buffer. Anything else, and every error, takes the general path.
- **A process that waits for the same signals twice in a row without a timeout has resident waiters**
  (`ProcMeta::perm`): the wake scan keeps them and the commit registers nothing. Ceiling: a wait on
  different signals, or with a timeout, ends the epoch and the stale waiters go at the next scan of
  their lists. Processes of one signal now wake in the order of their first wait. The batch order is
  sorted by process, so output is the same; only the permutation of resumed processes under `--shuffle`
  differs from earlier builds.
- **`Observer::phase_after_serial(phase)`**: the kernel calls such an observer's `phase` only in a cycle
  after which a serial process ran, and keeps a phase mask per observer. The shared-heap collector uses it
  for `NextCycle`: only serial processes allocate in the shared heap, so a design without any pays no
  mutex and no hook per delta cycle.
- **The wake scan asks interned trigger ids stored in the waiters**, not the process table, and a signal
  whose waiters all share one edge trigger that holds skips the per-waiter question.

## Kernel round 6: the cost of a delta cycle (yavs-kernel, #440, 2026-10-21)

- **A signal whose waiters are all resident and wait for the whole signal keeps its wake list**
  (`Waiters::cache`): an event wakes it in registration order without a scan. A waiter added or a
  process that leaves its resident wait drops it. Same wake order as the scan.
- **A zero-delay assignment of one `std_ulogic` element to an idle driver is a `Waveform::quick`**,
  not a transaction list entry; code that edits `Waveform::pending` goes through `spill`/`idle`.
  A zero-delay vector assignment to an idle driver appends its one transaction directly.
- **One dirty element with one source is finished by comparing codes; a run of one-element,
  one-driver segments is gathered into plane words; runs of changed elements are stamped as
  runs.** Results are the same as the generic paths.
- **Dirty ranges of one signal also merge when the next range ends where the last starts**
  (processes in descending element order): one range, one pass, and the value-change callbacks of a
  cycle then see the runs in ascending element order where they came in driver order before. The
  set of changes is the same; the order of the runs within one signal and one cycle is not
  specified.
- **Signals of 9 to 1024 elements have a waiter list per element** (8 or fewer: one list; to 4096:
  chunks of 8; beyond: 64), so an event on one element of a vector scans the waiters of that
  element only. Ceiling: 64 KiB of list headers per such signal that has waiters.
- **`run_until` takes the next-event time from the end of the previous delta** (`known_next`), and the
  batch and update worklists are kept between cycles instead of being built and dropped per delta.

## Interpreter round 8: fused sequences, search loops, chains of `&` (yavs-interp, yavs-ir, yavs-lower, #443, 2026-10-21)

- **Sequences of predecoded ops are fused in a pass over the resolved ops** (`pre::superops`): the
  first op of a known sequence is replaced by one that does all of it and steps over the rest, which
  stay in place (a jump, a resumed frame or compiled code that enters in the middle runs them one by
  one, so no op index, `Code::entry` or `Code::at` changes). The sequences: the latch of a loop
  (compare and branch, add, jump back), a load and compare of an array element, a fat pair or a
  descriptor built in temporary memory, a compare and the select on it, a dereference and the load
  through it, a move and the branch on the moved value, the edge test and the branch on it, the wait
  and the suspension after it, a memo flag and its branch, a slot and the signal reference it
  indexes, and a concatenation of logic elements into a temporary that is only copied on (written to
  its target directly when it fits one block of planes, read before written).
- **A loop that searches an array** (element load and compare, then the latch of the same index)
  runs its iterations in one op (`pre::SCAN`, `Machine::scan_loop`): the descriptor and the array are
  looked up once, anything the generic ops handle (an index out of bounds, memory outside the common
  spaces, an overflow) ends the run and the generic op takes that iteration. With a compiled tier
  set the iterations count as jumps, so compilation is still looked for every `RECHECK` of them.
- **The optimiser's memoisation pass first joins blocks that only a jump enters** (`opt/memo.rs`
  `merge_chains`), so the pieces lowering leaves between the arms of an `if` are planned as one
  block. Blocks entered by falling through (after a `Suspend` or a call), blocks that suspend or call,
  the entry and the body are never joined; dead blocks do not count as predecessors and are not
  joined into.
- **A chain `a & b & c` of elements of a scalar type is one concatenation of all its operands**
  (`yavs-lower` `cat_items`), not one per `&` with an array in between. The result's bounds and the
  null-operand rule (all operands null: the last one) are those of the nested form; only the index
  check of the result is made once, on the whole. (rationale: the intermediate arrays were most of
  the work of message building)
- Inlining small subprograms in the optimiser was measured before building it: frame entry and exit
  (`Pre::fill`, `push_args`, `new_frame`, `pop_frame`, `Call`, `Ret`) are about 5 percent of the
  instructions of the scoreboard testbench, so even removing all of it is below what the fused ops
  gave, and error traces would need a record of the inlined frames. Not done.

## Interpreter round 9: whole-statement ops for clocked RTL, logical operators of `numeric_std` (yavs-interp, yavs-ir, #443, 2026-10-21)

- **The waveform buffer of a one-element assignment is not built when only the assignment reads
  it.** The predecoder drops the `Alloc`, `Store` and `Index` of a waveform (`WaveBuf`) when later in
  the same block the value is copied into it (`Copy`) or set in element 0 (`LSet`) and the
  one-element `Drive` of it follows, nothing else reads the buffer and the delay register is not
  written in between (`pre::waveform_is_private` and a definition scan). The `Copy` and the `Drive`
  are one op (`CopyDrive`), the `LSet` and the `Drive` another (`LSetDrive`). The fast path hands
  the value (the planes at `src`, or the code in `v`) and the delay to the kernel
  (`assign_blocks`/`assign_code`) without the copy. Every other target (a signal of several parts,
  elements that are not `std_ulogic`, a size that is not the planes of the target, a bad code) makes
  the waveform the dropped instructions would have made in temporary memory and runs the
  instructions as before (`Machine::drive`, `drive_parts`, `replay_lset_drive`), so values, errors
  and their texts are those of the unfused code; a fault is reported at the pc of the first
  fused instruction.
- **The temporary of a signal read and of a concatenation of at most 64 logic elements is made
  once, not zeroed and read back** (`AllocReadVec`: `AllocK` of one block and the `SigReadVec` into
  it; `AllocCat`: `AllocK` of one block and the `Cat` into it, `Machine::cat_words` is the piece of
  `cat_block` that only computes the planes). Anything that does not fit (several parts, more than
  64 elements, a piece that is not a plane block) allocates and goes on with the generic op.
- **`numeric_std` `and`, `or`, `nand`, `nor`, `xor`, `xnor` of two `unsigned` or two `signed`, and
  `not`, are native** (`yavs_ir::ieee::Op::NumGate`, `NumNot`; not for `numeric_bit`). Each is the
  `std_logic_1164` operator on the same elements with the result range `(n-1 downto 0)`; operands of
  different lengths hand the call back to the body, whose assertion reports the length error as
  before (`has_fallback`). The body was a conversion, a native call and a copy back, about 12000
  instructions for 16 elements.
- **A call of a native `ieee` subprogram is `Op::CallIeee`** when it has at most four arguments,
  none a signal, and is not the procedure `uniform`: it skips what other natives need (the heap
  recycling check, the taint check of package initialisation, the symbol tests for `std.env`, the
  argument vector) and keeps the hand-back to the body and the stop of the process. These natives
  work on temporary memory only.
- **`Machine::rd` takes the common spaces inline** (`rd_slow` for the rest and every error);
  `ieee::small` and `vh` read a fat pair and its descriptor with it and `dim_of` instead of five
  loads. `load_idx`/`store_idx` add the element offset to the address words without making the
  address; `lcopy` of one block to one block changes the destination in place; `scalar_gate` and
  `scalar_not` are table lookups; the text of an enumeration literal (`to_string`, `'image`) is made
  once per process; a move of a register onto itself is not an op.
- **A process with up to 256 registers keeps its register and temporary vectors between
  activations** (was 48), as long as all such processes together hold at most 96 MiB
  (`RESIDENT_BUDGET`, given back when the process is dropped); the others swap through the thread's
  scratch as before. A clocked process with an `ieee` vector expression has well over 48 registers.
- Tried and dropped (no gain or worse in instructions or cycles): a thin `Fault` (one pointer, so a
  `Result<u64, Fault>` fits two registers; +2 percent instructions), reading a single signal
  element without building the reference, a block of zeros through `extend_from_slice` in
  `temp_alloc`, `call_pre` out of line.
- What is left in the clocked rows (instructions per activation, lfsr: about 3500): the dispatch
  (15 to 17 instructions per op, `k` and the op slice are spilled by the compiler), the kernel side
  (wake, assignment, update: about 40 percent), the five single-element reads, the two signal
  references (memoised in frame slots) and the three `Gate`s of a feedback; the library testbenches
  are flat (no op over 12 percent, string loops of 20 to 25 ops per character).

## Kernel round 7: cheaper reads and one-element events (yavs-kernel, #440, 2026-10-21)

- **A logic signal of at most 2^20 elements keeps the `std_ulogic` code of every element of its
  current value as one byte** (`Signal::codes`), refreshed wherever `cur` is written. A read of one
  element (`Ctx::elem_code`, the scalar read of the interpreter, the edge checks, the
  unchanged-assignment test, edge triggers) is a load. Debug builds check each read against the
  planes.
- **A quick transaction on an element that has no other source is finished when it is applied**
  (driving value, effective value, event), for at most four per signal and cycle; more of them in a
  cycle go through the dirty ranges as before. The runs of such a cycle are sorted and joined, so
  the value-change callbacks of a cycle that took this path see ascending, maximal runs (the order
  of the runs of one signal and cycle was already unspecified).
- **The runs of changed elements are listed only for signals something watches** (a waiting process
  or an observer that wants value changes); for the others they are only counted. `Stats::events`
  counts one per update of a run for those signals instead of one per merged run.
- **Compiled code reads the element codes itself.** `TierFrame::sigtab` has one `TierSig` per
  instance signal (the address and length of its codes, its kernel signal and first element,
  built at the first entry of a machine from `Ctx::code_view`), and `known_*` the edge the
  kernel resumed the process for (`Ctx::known_edge`). `SigReadScalar` and `rising_edge` /
  `falling_edge` of one element become a table lookup and a byte load or compare in the compiled
  function; a signal without codes, a null range, a part table or an edge that is not the known
  one goes through `TierHost::signal` as before. The pointers are valid for the whole run and
  the bytes change only in the update phase.
- **The `LSetDrive` fusion of the predecoder tested "register used once" on whole-function use
  counts, which the optimiser's register sharing (#443) broke for a scalar assignment in a
  process with an assignment in two branches**; it now tests the buffer's reads between its `Alloc`
  and the `Drive` and that nothing reads it afterwards (`pre::live_after`, a walk over the blocks
  after the `Drive`). A frame load of an element offset and the `SigRead1` it feeds became one op
  (`SigRead1Slot`). The clocked designs lose a tenth of their instructions.
- A scalar logic assignment in compiled code (`Drive` right after the `LSet` of element 0 of a
  one-element waveform, `TierHost::signal`) drives from the code in the `LSet`'s register as the
  predecoder's `LSetDrive` does, instead of reading the waveform back from memory.
- Smaller: a per-driver flag for "another source on some element" replaces the segment search in the
  unchanged-assignment test; the known edge of a trigger is looked up in a table; a batch of one
  process or entry is not ordered or sorted; a phase wanted only after a serial process is skipped
  without scanning the observers; the event stamps of a run are written in one pass (of an unwatched signal, per changed element).
- Tried and dropped (no gain beyond noise in instructions or cycles): counting repeated waits at the
  wait instead of comparing the sensitivity at the commit, a one-byte "nothing happened" flag per
  process, a cache-line layout of the process slot and its metadata, a global flag for batches
  without a process of several waits (the testbench process has several), cheaper bit-plane
  writes by a table.

## Heap collection threshold (yavs-interp, CLI, user request 2026-10-20)

- `yavs run --gc-threshold SIZE` (plain bytes or a `k`, `m`, `g` suffix of powers of 1024, at least 1; zero and
  anything else is a usage error) sets `Limits::gc_threshold` for the elaboration-time evaluation and the
  simulation. It is a process-wide setting (`yavs_interp::set_default_gc_threshold`) read by `Limits::default`,
  so no entry point takes a new argument. Test runners pass it in their simulator flags.
- There is still no off switch: memory stays bounded (#409, #422).
- **The default stays 1 MiB.** A sweep of 1 to 256 MiB (`docs/STATUS.md`, "Heap collection threshold") shows no run
  time difference beyond noise on the library testbenches and the corpus, and 64 times fewer collections. Peak
  memory is the same for the library testbenches and grows with the threshold for designs that allocate heavily.
  The threshold applies to each process heap, so a larger default would multiply the worst case by the number of
  allocating processes for no measured gain. Collection is not why the tiny access test is slower than in the
  reference simulator (the interpreter is).

## Concatenation instruction and descriptor-length folds (yavs-ir, yavs-interp, yavs-lower, yavs-llvm, #443)

- **`Inst::Cat { dst, n, m, parts, scalars, bias, logic, w }` is the IR contract for `&` of
  one-dimensional arrays and elements (and for positional aggregates with array-valued pieces).**
  `dst` is the result block as lowering made it (`Alloc`, `alloc_val`, with the `TempRoot` typing of the
  collector unchanged: `Cat` allocates nothing and roots nothing), `n` the result's element count, `m` the
  units per element (plane elements of a logic array when `logic`, else bytes). `parts` holds two registers per
  piece in order from the left: the address of an array and its element count, or, with bit `k` of `scalars` set,
  one element in the register itself (the count register is ignored, lowering passes a constant). A scalar of a
  logic array is the `std_ulogic` code position plus `bias` (2 for `bit` and `boolean`), of a byte array stored
  with width `w`. The result is exactly that of the instructions it replaces, in order: per array piece an `LCopy`
  from plane element 0 of the source to plane `(n - k - len) * m` (planes hold the leftmost element last), an
  `LSet` for a scalar piece, or `Copy` and `Store` at byte `k * m`. Index bound errors, the null-operand rule and
  `SubtypeCheck` of the result stay separate instructions before and after it, so messages and pcs are those of
  the old sequence. Up to 16 pieces; lowering stores more piece by piece as before.
  Why: the old sequence was about 30 instructions per concatenation (position arithmetic per piece) and was most
  of the interpreter cost of a shift register; the interpreter writes a logic result of up to 64 plane elements as
  one block of combined plane words (`Machine::cat_block`), and falls back to `lcopy`/`lset`.
- **The optimiser treats `Cat` like `LCopy`:** it writes its block and nothing else (`effect` forgets facts about
  that block only), reads its piece registers and `n`, `m`, allocates nothing (`may_allocate`).
- **The LLVM tier runs `Cat` through the step side exit** (`keeps_objects` lists it); no code is generated for it.
  A direct helper call can follow when a compiled body needs it.
- **Descriptor lengths are facts.** A `MakeDope` whose parts are constants records the length of each dimension
  (`Key::Dope`), so `DopeLen`, the null test of a slice or concatenation and the `SubtypeCheck` against another
  descriptor of known length (a pool constant or another such `MakeDope`) fold; an equal-length
  `SubtypeCheck` is dropped. `LayoutOf` of a one-dimensional array type under such a descriptor is computed at
  optimisation time (the layout depends on the length alone) and the loads of its words become constants; an
  `Alloc` block that only `MakeDope` writes goes with its writers (`dce`). Writes by `SigRead`, `LSet`, `LFill`,
  `LCopy` and `Cat` forget only the facts about their own block: blocks made by different `Alloc`s are disjoint
  (`Cls::Block`).
- **A position or check repeated with another message is the same available expression** (`Key::Pos`,
  `Key::Check`, `Key::BinRange` no longer include the message): the earlier one dominates and fails first with its
  own message, or this one passes.
- **Folded branches pay off in the same call:** when a walk replaced a branch on a constant by a jump, up to
  four more walks run (the join after it has one predecessor less, so facts reach past it). Reachability for
  code that suspends is from the entry (and the block after a suspension that ends a block) instead of "every
  block": dead diamonds in processes are emptied like those in functions.
- **Register sharing leaves the temporaries of linear runs alone** (`opt/share.rs`): a `Bin` (add, subtract,
  multiply), `Pos` or first-dimension `DopeLen` defined once and read only within its block keeps its own cell,
  because the predecoder fuses such runs (`pre::lin_run`) only for registers defined once; with shared cells every
  element access of a function over 31 registers stayed nine instructions. Cost: a few more cells per frame.
- **A slice assignment needs no instruction of its own:** `LCopy` (planes, any alignment) and `Copy` are the
  slice copy, the checks around them are the shared `SubtypeCheck`; what a slice costs is the position arithmetic,
  which the linear runs and the folds above remove.

## Select instruction (yavs-ir, yavs-interp, yavs-lower, yavs-llvm, #443)

- **`Inst::Select { dst, c, a, b }`: `dst` is `a` when `c` is not zero, else `b`**, for any register class but a
  signal reference. The lowering's `select` helper (28 sites: bounds of a range, the check value and the
  both-operands-null result of `&`, `minimum`/`maximum`, loop steps, conditional expressions) emitted a diamond
  of a `CondBr` and two `Move`s with a join block, three dispatches and a branch the interpreter's indirect jump
  mispredicts; both values are computed before the call, so nothing is lazy. The result is defined once, which
  also lets the optimiser's common-subexpression pass and the predecoder's linear runs see it.
- The optimiser folds a `Select` on a constant condition or with equal operands to a `Move`, drops an unread one,
  and gives it the hull of its operands' intervals (`opt/range.rs`); the interpreter has an op for it; the LLVM
  tier compiles it to an LLVM `select`.
- The concatenation lowering tests `n /= 0` directly (one `Cmp` instead of `Cmp` and `Not`) and selects the
  result of two null operands (LRM 9.2.5) with that.

## Process entry and exit, direct side exits (yavs-interp, yavs-llvm, #443, #432, #446, 2026-10-21)

- **A small process keeps its register and temporary vectors between activations** (`Machine::resident`: no
  debugger, at most 48 registers in its root frame, so at most 384 bytes). The swap with the thread's scratch
  (two thread-local accesses, two vector swaps and a key check per activation) is for the processes above that
  size, and for a resident one whose temporary area outgrows 1 KiB (it becomes a scratch process for good).
  Ceiling: a design of tens of thousands of small processes holds up to 384 bytes of registers each instead of
  nothing between activations; the scratch stays for the large frames of testbench processes.
- **`InterpProcess::run` is a flag test and a call of `Machine::run_quick`** once a process is started, not
  ended, without shared state and without a debugger (`hot`); the checks of `borrow_scratch`, `release_scratch`,
  `after_suspend` and of the root frame's `suspend` are inline, their cold parts out of line.
- **`copy_const` takes the program of the running runtime** instead of cloning the runtime handle per call (#446).
- **Compiled code runs `TempRoot`, `Stmt` with recorded roots, `NewFrame`, `Deref`, `MemEq` and `Cat` through
  `TierHost::signal`**, the interpreter's own bodies without the generic step, and `CallNative` through
  `call_native` directly. None of them frees or moves a memory object, so compiled code keeps the objects it
  found. A `TempRoot` of a fat pair first asks a helper (`FAST_FAT_ROOT`) whether any word points into a heap
  (the interpreter's `fat_root` test); only then is the root recorded, so the precise roots (#422) are the same.
  Why a helper and not inline code: inline code for the test made compiling the testbench library 1.6 percent
  more expensive for no run time gain.
## Instance-constant values of a process are computed once per instance (yavs-ir `opt/memo.rs`, #443)

- A process function is shared by every instance of its design unit, so generics, generate
  indices and constants are read from instance data at every activation. The optimiser's last
  step for a `Process` (off for a debugger and with `YAVS_NO_MEMO`) splits a block of the process
  loop at its first instance-constant instruction (pure, operands from constants, addresses,
  loads from instance data and the constant pool, and what is computed from them, including
  checks, checked arithmetic, positions) into a head testing a flag in the process frame, a slow
  copy that is the original code and stores the values the rest needs and then sets the flag, and
  a fast copy without those instructions that loads the values from frame slots. Reason: a
  failing check must fail with the same message at the same time, which only the original code
  (run until the first complete pass) guarantees; no speculative evaluation is needed, and no
  elaboration-time specialisation, so the code stays shared (rule 4) and compile cost does not
  grow with the instance count. Cost: 8 frame bytes per value plus one, an extra `Load` and
  `CondBr` per activation of the block, roughly twice the code of the block from the first
  instance-constant instruction on. Only blocks on a cycle, at most 256 values and 4096
  instructions, and only when the saving is at least half the value count plus three instructions
  (six compiled); a block over 800 instructions must save a fifth of itself (#662).
- **No cliff at 40 values or 800 instructions** (#662, 2026-10-24): the caps were 40 values and 800
  instructions, and a clocked process xoring 40 taps of a vector (`q((i + 7 + 3 j) mod N)`, one value each) executed
  2.7 times the instructions of one with 32 (22.6 G to 60.9 G for 1024 instances and 4000 cycles in the interpreter):
  the pass gave up and every activation recomputed the positions. With 256 values and 4096 instructions the cost is
  linear (26.7 G at 40 taps, 39.3 G at 64, 73.8 G at 128). A long block of a long process that runs few activations
  loses by the second copy (predecoding a long process costs more per instruction, #683: `olo_axi_master_full` +12 percent
  instructions with the caps alone), so a block over 800 instructions is copied only when the fast copy saves a
  fifth of it (the taps: 26 percent; the testbench processes there: 9 to 15). Work-counter test `memo_wide`.
- The slow copy renames the registers only it uses, so the fast copy keeps the register read and
  definition counts the predecoder's fusions rely on.
- `Program::single_suspend` (the kernel's process order, #252) reads the lowered function, not the
  optimised copy, because the optimiser may copy a suspension point.

## Floor of a run: symbol table size, pruning on demand, executable layout (#426, yavs decisions under the user's single-thread performance priority, 2026-10-21)

- **The global symbol table starts small** (2048 names, was 16384). A table pre-sized for a whole
  library is written at random places, so every name of a short run touched a page of its own: 200
  page faults, 0.13 ms of a 1.3 ms run. A longer run grows it by rehashing (a rehash of 20 000
  names is under 1 ms).
- **`Store::put` prunes on demand.** The prune asks the size and age of every entry (a system call
  each: 0.25 ms for 90 entries) and ran on every put; a run that stores three entries paid it three
  times, and a cold test suite pays it in every first run of a test. Now the names are counted first
  (one directory read), and the entries are looked at when there are more than `MAX_ENTRIES`, when
  the entry just stored is big enough to fill the byte limit with 15 others (32 MiB), and, for a
  directory of more than 16 entries, at most every 5 seconds (`.swept` marks the time). A directory
  of 16 entries of at most 32 MiB cannot be over `MAX_BYTES`, so the byte limit holds exactly there
  and is soft by what is stored within 5 seconds elsewhere. Notes (`.seen` files, `note`,
  `sightings`) are trimmed to `MAX_NOTES` once the directory is 64 past it, by names for counting
  and ages only for the trim; `sightings` counts by name without asking for any age.
- **The executable stays position independent (PIE) on every platform (user decision, 2026-10-21).**
  A `-no-pie` link on Linux (glibc) had saved the loader's 11 000 relative relocations and 45 page
  faults, about 7% of the cycles of a short run. The user chose PIE for hardening and packaging, so
  that gain is given up: `crates/yavs/build.rs` links no `-no-pie`, and the executable's code keeps
  address space layout randomisation.

## Compiled frames and calls between compiled functions (yavs-llvm, yavs-interp, #432, #337, #340, 2026-10-21)

- **Problem:** a call from compiled code to a compiled callee went through `yavs_rt_step`: the generic
  step, the interpreter's frame push (`Pre::fill`, zeroed slots, argument cells), two full frame views and
  `TierHost::ret`. Profile of `calls` under `--tier=llvm`: 95% of the time in that protocol, 5% in compiled
  code; a compiled call cost what an interpreted one does.
- **Two frame kinds.** A *frame-mode* function (every process, and any function that steps an instruction
  on its fast path) runs as before on an interpreter frame (`TierFrame` view, registers in the machine's
  register stack). A *native-callable* function (`lower::native_callable`: a function or procedure, not
  nested, at most 256 registers and 4 KiB of slots, whose every instruction is lowered without a step on
  its fast path, and whose calls are all to native-callable functions) also gets a second LLVM function,
  `f<id>_n(env, mem, depth, args...) -> i64`, with a *native frame*: registers in an `alloca` (promoted to
  machine registers and SSA values at O2), slots in an `alloca` of the frame size, arguments as LLVM
  parameters stored into the parameter slots, the result as the return value. Its frame index is the
  caller's plus one (`depth`), so every frame address it makes is the one its interpreter frame would have.
  Temporaries stay in the machine's temporary area (the view's length), with the frame's temporary base in an
  SSA value. Compiled callers call it with an LLVM `call`: directly in one module (`--tier=llvm`, where LLVM
  may inline it), through the per-function entry table `Mem::ntab` across the modules of `--tier=auto`
  (an empty entry steps the call as before).
- **Materialisation (deoptimisation).** A native frame becomes an interpreter frame only when something
  needs one: an instruction that must be stepped (a slow path, an address `Mem` does not reach), a failing
  inline instruction (the interpreter re-executes it for the identical error and trace), a call deeper than
  the depth limit or the native-stack allowance. Then the function records its registers, slots, pc and
  temporary base (`yavs_rt_deopt`, into a list in `Env`) and returns with `Mem::ndeopt` set; each native
  caller records itself at its call; the first frame-mode caller hands the list to
  `TierHost::materialise`, which sets its own pc to the call, pushes the recorded frames bottom-up with the
  interpreter's `push_frame` (depth check, static link, `Pre::fill`, result register from the caller's
  `Call`) and copies the recorded registers (except preloaded ones, which only ever hold their preload)
  and slots; compiled code then returns `TIER_UNWIND` and the interpreter goes on with the top frame. The
  frame-mode version of each function resumes a materialised frame at the instruction after a call.
- **Invariants kept by construction.** A native frame never reaches a safepoint (new, deallocate, natives,
  calls into frame-mode code, suspension, `TempRoot` recording): those are steps, so the frame is
  materialised before them and the precise roots (#422) see interpreter frames only, at the pcs they wait
  in. Error messages and VHDL stack traces come from the interpreter on materialised frames. A debugger,
  coverage and VHPI see only interpreter frames: debugged processes are never compiled, `Cover` is stepped
  (so a function with coverage counters is frame-mode; the counters themselves are compiled since #692). Output is independent of which frames were native
  (rule 5).
- **Helpers without the generic step:** `Free`, `Fill` and `CallProt` run through direct arms of
  `TierHost::step` (the interpreter's own `deallocate`, fill and `call_with` bodies).
- **`Deref` and `MemEq` without a frame (2026-10-21):** compiled code dereferences an access value through
  `yavs_rt_fast` (`FAST_DEREF`: `TierHost::deref`, the heap's own lookup, no fault recorded; null or
  deallocated takes the slow path, where the interpreter reports it) and compares memory it found itself
  (`yavs_rt_memeq`, argument memory only), so neither needs an interpreter frame and functions using them
  can run on native frames (7 to 21 of the functions of a verification-library scoreboard testbench).
- **Entry after preloaded definitions:** the interpreter enters a function after the constants and
  addresses it preloads into the frame's registers (`Pre::next`), so frame-mode code also has an entry at
  each pc that follows such definitions at a block start; before, it was entered only at its first jump,
  and a short loop-free function called from the interpreter never ran compiled.
- **Tier-up (`--tier=auto`):** a loop counts as compute when its native-frame calls are left out (they are
  direct calls), and the sampled interpreted time of a short native-callable function counts for the
  interpreted caller seen longest that calls it (`Worker::charge`), which then compiles with it (its callees
  compile with a caller), since compiling the short function alone gains nothing.
- **Effects of the runtime helpers (2026-10-21, coordinator request):** the helper declarations say what
  they touch, so LLVM keeps registers and loaded values across them: none unwinds (they catch panics);
  `yavs_rt_host` is `memory(argmem: readwrite, inaccessiblemem: readwrite)` and `willreturn` (it reads
  only `Mem`, `Env` and the interpreter's bookkeeping, never object bytes, slots or registers);
  `yavs_rt_lcopy` touches only its argument blocks (`nofree`, `nosync`); the `yavs_rt_fast` call sites
  read (dope length, subtype, fat root) or write (alloc) other memory only; `yavs_rt_step`,
  `yavs_rt_signal`, `yavs_rt_layout` and the materialisation helpers keep no memory attribute: they run
  interpreter code that may collect, so a handle compiled code holds is reloaded after them and the
  precise roots (#422) do not depend on any attribute. Measured pinned (core 3, minimum of 3,
  `--tier=llvm`): `compute_loop`, `ff`, `lfsr`, `chain`, `fan`, `bus`, `ram` within 1% in instructions and
  cycles; `rand_tb` simulation within run-to-run noise (0.85 to 0.98 s either way). The time of
  compiled testbench and RTL code is in the callbacks that must clobber memory. Not done: an alias scope
  for the own frame's slots against other memory (frame-mode `host()` would need the inline own-frame
  check native frames have, so a host pointer never points into the own slots) and for signal value
  storage (the RTL agent's arms); worth it once the clobbering callbacks are rarer. For a Cranelift
  generator the same facts are its memory flags: frame, temporary and object accesses as separate alias
  regions (`heap`, `table`, `vmctx`-style regions), constant reads `readonly` and `notrap` where the generator
  checked the bounds; Cranelift has no per-call effects, so helper calls stay full clobbers there.
- **RTL processes** stay frame-mode: a process is entered by the kernel per activation on its own
  interpreter frame and suspends there; the per-activation fast paths are the RTL agent's (#432, rtljit).
  A process calling a native-callable function gets the native call.
- **Second code generator:** the ABI is the generator's contract with the runtime: the helpers
  (`yavs_rt_deopt`, `TierHost::materialise`) are plain C-ABI Rust functions and the native frame is a
  generator-local choice, so a Cranelift backend can produce the same `f<id>_n` with `call` and stack
  slots. Why not lazy frames for every function: spilling a frame-mode function's registers at every step
  would cost more than the push on testbench code, where most functions step something (natives, `new`,
  string temporaries); and why not keep native frames materialised in place: frame addresses would then
  have two homes. Decided by the compiled-calls agent (ccall), 2026-10-21.

## Kernel round 8: vector assignments without lists or buffers (yavs-kernel, yavs-interp, #440, 2026-10-21)

- **A zero-delay assignment of at most 64 logic elements to an idle driver is a `Waveform::vquick`**
  (time, first element, length, the four plane words), the vector counterpart of `quick`: no `Txn`, no
  value buffer, no spare-buffer bookkeeping. Everything that schedules, preempts or inspects
  `Waveform::pending` goes through `spill`/`idle` first, which turn it into the transaction it stands for.
  The unchanged-assignment test (`Config::skip_unchanged`) compares plane words instead of building a
  value.
- **A vector quick transaction on a driver that is the only source of its elements, on a signal with no
  layer, group, guard or resolution function, writes the driving value when it is applied**
  (`Signal::apply_driver`) and lists the run in `Signal::fast_dirty` (runs join as `dirty` ones do);
  `finish_signal` then marks the activity of the joined runs and updates their effective values, without
  `compute_driving`. Order and results are unchanged. What differs is how the runs of changed elements
  are cut when a cycle mixes such vectors with others on the same signal (the joined `fast_dirty` ranges
  and the `dirty` ranges are updated one list after the other): the value-change callbacks and
  `Stats::events` can see different run boundaries in that mixed case, at any thread count.
- **`Ctx::assign_blocks(driver, target, delay, blocks, mode)`**: `assign` of a logic value given as the
  plane blocks in the layout `Ctx::read_into` writes (four little-endian `u64` planes per 64 elements).
  Up to 64 elements it goes to the vquick path with no value built; longer values and every error go
  through `assign`. The interpreter's one-waveform-element `Machine::drive` of a logic signal calls it
  with the bytes of its frame, which removes a heap vector and a `LogicVec` per vector assignment.
- **The per-element time stamps of a signal (last event, last activity) hold the time plus one**, 0 meaning
  never, so a new array is all zeros and costs no fill: a 500 000 element memory no longer writes 8 MB of
  "never" markers when it is elaborated.
- **Tried and dropped, no gain in cycles** (instruction counts moved a few tenths of a percent, run times
  did not): hot-first field order (`repr(C)`) of `Waveform`, `ProcState` and `Slot`; reading the next
  slots and process records of the commit loop ahead; committing each process right after it ran
  (needs the shared view rebuilt per process); setting `timed_out` and the known edge when the process
  runs instead of when it is woken.

## Compiled clocked RTL processes: direct kernel entry points (yavs-kernel, yavs-interp, yavs-llvm, #432, #339, #440, 2026-10-21)

- **Vector drive:** compiled code reaches `Ctx::assign_blocks` (kernel round 8) through `TierHost::drive_block`
  and `yavs_rt_drive`: the one-waveform-element `Drive` of a logic target of 1 to 64 elements, the time and the
  first plane block read from the waveform by the caller; anything else is the generic `signal` step.
- **`TierHost::wait_suspend`** runs a `WaitOn` and the `Suspend` after it from plain words (`yavs_rt_wait`); a
  multi-part reference or a debugger goes the generic way. `TierHost::view_again` fills the frame view of a
  one-frame process straight from the machine's fields. Both fall back to `signal`/`view`.
- Compiled code now does inline: `Alloc`, `DopeLen`, loads and stores in the temporary area and the instance
  data (a pointer in the view, bounds checked, no call), `SigRead` of 1 to 64 elements from the code bytes
  (`Gen::sig_read_block`), and `Cat` of at most 64 logic elements whose piece lengths are known
  (`Gen::cat_block`; same result as `Machine::cat_block`, a code beyond 'U' to '-' or unreachable memory steps
  the whole instruction before any store). Registers defined once by a `Const` count as known constants
  everywhere (`fixed_consts`, the interpreter's loop relies on the same), and the length word of a `SigVal`
  is known with its `len`.
- A process activation that ends in a suspension runs compiled code straight from `Machine::run_quick`
  (`run_compiled`: no `exec_fast`/`run_ops` set-up, nothing returned by value) and then keeps its vectors
  (`HOT_RESIDENT_REGS`, 256 registers) instead of swapping them with the thread's scratch.
- Alias scope metadata (frame slots, temporary area, instance data, constants, code bytes are distinct
  scopes) and `nounwind willreturn` on the runtime callbacks: no measurable change on the kernel designs
  (every drive, wait and step stays a full clobber), kept because they are free.
- `--tier=auto` compiles a process body (kind process, no loop, 200 to 600 IR instructions: `auto.rs`
  `rtl`) at O0 as soon as it was seen running for `Heat::threshold`, with the compute budget, and again at O2
  after six times that (the compile thread is another core). O0 code is a tenth slower than O2 on these bodies
  and compiles in a tenth of the time. Below 200 instructions entering compiled code costs more than the
  interpreter's fused ops (`chain`, `fan`, `ff` ran 5 to 30% slower compiled), so they stay interpreted. `loops`
  ignores blocks the optimiser left unreachable (a dead self-jump made every process look like a loop and kept
  it out of tier-up). Decided by measurement; not a user ruling.

## Compiled clocked RTL processes, round 2: a wait that registers nothing, kept object addresses (yavs-kernel, yavs-interp, yavs-llvm, #432, #440, 2026-10-21)

- **A wait on exactly the range the last wait registered registers nothing.** `Ctx::wait_on_again(signal, range)`
  succeeds when `sens` is empty, no timeout was set and `sens_prev` is that one range; it leaves `sens` empty
  and sets `ProcState::again`. The commit (`run_batch`) treats `again` as the same wait it already registered
  (`ProcMeta::perm`, the resident waiter), and a process that adds anything else to the wait after it (another
  signal, a timeout) gets `sens` rebuilt as `sens_prev` followed by what it added, before the usual logic. For
  the usual clocked process the commit loop then only moves the scheduled drivers into the queue. The
  interpreter's `WaitOn`/`wait_elem` and the compiled tier (`TierHost::wait_again`, taken first by
  `yavs_rt_wait`) use it; kernel tests `a_wait_on_the_last_range_is_the_same_wait` and
  `a_wait_that_adds_to_the_last_one_keeps_everything`. No signature changed.
- **A zero-delay drive of one element of a logic signal from compiled code** is `TierHost::drive_now` (no frame
  view, no result) into the new `Ctx::assign_code_now` (the scalar fast path); `drive_block` sends a one-element
  drive of any delay through `Ctx::assign_code` instead of building plane bytes (the apply side of a scalar
  quick transaction was 4% cheaper than the vector one on `ff`).
- **`Tier::run_cached` and `TierState`.** A process keeps, between activations, the host addresses of the heap
  objects compiled code found (up to four entries: `Mem::objs`, constants and `space::HEAP` only), valid while
  `Heap::epoch` is unchanged (it counts deallocations and edits in place; `TierFrame::heap_epoch` carries it).
  Without it a process with an array variable (a heap object) looked its object up twice per activation
  through the interpreter. The entries a read found are replaced, not repeated, by the same object found for
  a write; the objects it may write count as written in each activation (`TierHost::wrote`, the write count
  the waveform recorder compares). The frame view itself is not kept: a persistent copy needed fewer
  instructions but touched cold lines (`fan` +3% cycles, +14M L2 misses), the stack copy is rebuilt from the
  machine's fields (`Machine::view_root`, shared with `view_again`, which now also sets `max_depth`).
- **A process that suspends compiled hands its registers to the thread's scratch**, resident or not
  (`run_compiled` gives its `RESIDENT_USED` share back): compiled code writes a register per IR result, a dozen
  lines of the process's own register file per activation, and thousands of those files do not stay in the L1
  cache. Measured pinned, `--tier=llvm`: `ff` -11% cycles (+6% instructions for the swap), `lfsr` -7%, `rtl2`,
  `chain` and `fan` level (the register files of the 64 to 1000 processes of those fit the cache). `borrow_scratch`/`release_scratch` are inline, the cold part (`borrow_fill`)
  out of line.
- **Compiled `LCopy` of up to 64 elements** is inline (the planes of at most two blocks each side shifted as
  128-bit words, the source read before the destination is written); longer copies still go through
  `yavs_rt_lcopy`.
- **`--tier=auto`:** `RTL_MIN` is 140 IR instructions (was 200): a 150 instruction process (`ff`) compiles and
  wins about 7%, the 100 to 135 instruction processes of `rtl` and `rtl2` do not (the interpreter's fused vector
  ops of round 9 are as fast as the compiled code there). The O2 recompile stays at six times the first
  threshold; a later one (ten times the O2 compile time seen) lost 9% on `ff` and won 10% on `lfsr`, so no rule
  by compile time alone is better on this set. A run shorter than about 0.2 s pays its compile (the library
  use of the library alone costs 9 ms) and ends up a few percent behind the interpreter in total cycles, ahead in wall time
  (the compile thread is another core).
- **Not built, with the measurement:** one compiled entry per body that runs a batch of ready instances. What an
  activation of a compiled `ff` process costs (1900 instructions at 4096 processes): the process body 20%, the
  kernel's wake, commit and update 45%, the drive and wait callbacks 20%, the entry 15% (frame view, scratch
  swap, `Mem` set-up, the call). All of the entry is per-instance state (view of this process's slots, registers,
  signal table, instance data), so a batch loop saves the dynamic call and the `Ctx` set-up, 25 of 1900
  instructions; it would need the kernel to hand compiled code the instances' `Ctx`s, and a callback per drive
  and wait remains. What was tried and dropped, all within 1% or worse: a persistent frame view per process
  (above), fusing every quick transaction of a cycle (`FUSE_MAX`, +6% instructions), an alias scope for the `Mem` fields, branch
  weights on the compiled code's guards, hot-first field order of `Machine`, an unrolled `get_block`, a fused
  `update_effective` for one chunk (the runs the kernel passes are merged and longer than 64 elements),
  fat LTO with one codegen unit (0 to -3%).

## Tight compute loops: deferred register stores, statement markers in processes (yavs-llvm, yavs-ir, #432, #444, 2026-10-21)

- **Frame-mode compiled code stores a register only where the interpreter can look at the frame.** Compiled code
  runs on the interpreter's register file, and every definition used to be a store (an `Const` of the loop's
  bounds, each address, each temporary: 40 stores in a 14-statement loop body). The result of a pure instruction
  (`Const`, `ConstF`, `Move`, `Select`, `Bin`, `BinRange`, `Un`, `FBin`, `FUn`, `IToF`, `Cmp`, `FCmp`, `Bool`,
  an inline `AddrOf`, a `Load` of a frame variable at a known offset) is now an SSA value (`Gen::std`,
  `Defer::pend`). `Gen::flush` stores the deferred registers that are live (`regs.rs`: backward liveness over the
  instructions, `Live::row(pc)` = registers live before `pc`) where compiled code leaves the frame to the
  interpreter: the exit block of a failing guard (`exit_if`, live at the instruction the interpreter re-executes),
  every callback that gets the register pointer (`call_go`: steps, signals, drives, waits), the native call
  (`native_call`, so a collection in the callee finds the live references) and its unwind path, `Ret`, and every
  block edge (`Br`, `CondBr`, `Case`, a fall-through into an entry point: live at the successors). Dead
  temporaries are never stored. A register the interpreter starts every frame with (defined once by a
  constant or an address that is not `Up`: `regs::preloaded`, the same rule as `Pre::new`) is never stored by
  compiled code, and an address register of that kind is a known frame, constant or instance address in every
  block, not only in the block of its definition. The slow path of an instruction (`slow_path`) flushes without
  forgetting; an instruction with a fast and a slow path stores its result at once (`Gen::st`), because a value
  defined inside the diamond does not dominate the join. A function with more than 2^20 liveness words and
  native-frame code keep the old stores. Errors are identical: the interpreter re-executes the failing
  instruction from a frame that holds what it reads (tests: `arith/loop_overflow_last`, `arith/loop_range_last`).
  Why: design 1 of the head-to-head (a 3-million-iteration compute loop) took 317 instructions per iteration
  compiled against 113 for the reference code generator. Done next ("Registers and frame variables across blocks"):
  registers carried across blocks as phis with entry stubs, frame variables as SSA values with write-back at exits.
- **`Stmt` markers are dropped in processes too** (`Opt::drop_statements`): a marker of a statement that allocates
  no temporary memory frees nothing (kept with a debugger). It cost 6 compiled instructions and a dispatch per
  statement, 12 per iteration of the loop above.
- **`--tier=auto` compiles a compute loop after twice its estimated compile time** (`PAYBACK_COMPUTE`, budget
  factor 50, so the payback rule alone limits it; it was four times with 32 percent) and the first estimate is
  250 microseconds per IR instruction (was 400; the measured O2 cost of a process body is 120 to 250). A process
  that interprets one loop for the whole run reaches compiled code after about 0.15 s instead of 0.4 s.
  `rtl` processes keep their factor (16).
- `YAVS_LLVM_DUMP=<dir>` writes, for each compiled module, `<n>-pre.ll` and `<n>-post.ll` (before and after the
  pass pipeline) and `f<id>.ir` (the yavs IR of each function) for reading the generated code.

## Kernel round 9: fewer stages per activation and per delta (yavs-kernel, #440, 2026-10-21)

- **Every driver entry of a delta on one independent signal, in driver order, is applied in place** (`update`):
  with no force or deposit queued and nothing from the later-time heap, the entries are checked in one pass
  (ascending driver ids, one signal) and `apply_driver` runs straight from the entry list, then `finish_signal`.
  No `indep` list, no sort of it, no second pass in `update_independent`; anything else takes the general path
  unchanged. Same application order, so same events.
- **A resident process that waited again and only scheduled drivers is committed right after it ran** (the inline
  batch loop: always at `--threads 1`, small batches otherwise; `ProcMeta::ready` is cleared and the commit loop skips
  it): its slot is still in the cache, the commit loop of 4000 processes read each slot a second time from the L3.
  The queue pushes of such processes come before those of the processes that need the full commit; the queue
  sorts its due entries (`update`), so nothing observable changes. Initial runs and postponed batches always
  take the full commit.
- **`ProcState::timed_out` and `known_edge` are set when the process runs** (`resumption`), from `ProcMeta::by_timeout` and
  `tid`, instead of in `wake`: a wake touches only the process record. (Round 8 found no gain from this alone; with
  the commit above it is 2 to 4% of the cycles of `fan` and `ff`.)
- **A falling edge of a clock whose waiters are all held back by their edge trigger returns before the wake list is
  touched**; when none is held back the list is walked with no per-waiter question.
- **Batch order without a scan:** the processes with one suspension point wake into `ready`, the others into
  `ready_several`; `ready_mono` (ascending process numbers) says `order` has nothing to do for the first, and the
  few others are ordered alone and appended (their keys all sort after the plain ones). Same order as `order`.
- **`poll_end` reads the interrupt flag with a plain load** and exchanges it only when set (a locked exchange per delta
  was a few percent of the cycles of `chain`); `next_time` skips the wake-up map when it is empty.
- **The unchanged-assignment test of a scalar asks the signal's code array first** (dense, in cache) and the driver
  slot (one per process) only when the value would not change. The vector test is left as it was: asking the
  signal first made `lfsr` 1% worse in instructions.
- **Tried and dropped:** FUSE_MAX removed for signals nobody waits on (+9% instructions on `ff`: the block pass of the
  dirty ranges is cheaper than a fused finish per element); `#[inline]` on `Waveform::idle`/`first_time` is kept but
  moved nothing.
- Measured (`perf stat`, cores 2 and 3, interpreter, instructions / cycles before to after): `ff` 17.03 G to 16.05 G / 4.0 G to 3.7 G,
  `fan` 18.27 G to 17.21 G / 5.2 G to 4.5 G, `chain` 5.33 G to 5.15 G / 1.05 G to 0.95 G, `lfsr` 8.77 G to 8.55 G, `lfsr64` unchanged
  instructions, -8% cycles. What is left of the kernel per activation of `ff` is ~800 instructions: wake and
  commit ~250, apply and update ~250, the rest scattered over a dozen functions of 20 to 60 instructions each; the
  lever that remains is the process side (entry, drive and wait callbacks) and the per-element stamps, not the cycle.

## Kernel round 10: fewer instructions per element, per delta and per batch (yavs-kernel, #440, 2026-10-21)

Exact per-function instruction counts of one activation or delta (Intel PT traces of the h2h designs) placed the
kernel and its calls at about 900 of 1960 instructions per `rtl_ff` activation, 1400 of 3400 per `rtl_lfsr`
activation and 1650 of 2450 per `rtl_chain` delta. The changes below take the general costs found there; output is
unchanged at any thread count.

- **The effective value of a range on a block boundary of a plain logic signal is updated in whole plane words**
  (`Signal::update_planes`): one XOR per plane gives the changed elements of 64, `last_value` and `cur` are blended
  with that mask, then the stamps and runs of the changed elements and the codes as before. No `Buf` helper per
  chunk (`diff_chunk`, two `select`s). Off a block boundary, byte storage and layered signals (forces, port input)
  take the old path. `Buf::write_to` of at most 64 logic elements (a register read, `Ctx::read_into`) reads one
  block without the chunk loop. `rtl_lfsr` -2.8% instructions, `lfsr64` -1.7%.
- **A quick transaction finished at once writes three planes and the code byte directly** (`finish_quick`): the old
  code comes from `Signal::codes` instead of the planes, and a plane write is one masked blend with the plane masks
  of the code from a 16-entry table (`put_plane_code`, two SSE2 operations instead of four shift-and-merge
  sequences, about 31 to 15 instructions); `Buf::put_code` uses it too. `rtl_chain` -3.6% instructions.
- **A delta whose due entries are all drivers of one independent signal, all from this time's list, with no force,
  deposit, implicit signal or later-time entry due, takes a short update** (`update`): no work lists taken and given
  back, no independent list, no `update_independent` call; a driver's quick transaction is applied without the
  general `apply_driver` entry (`Signal::apply_quick`). The changed signals are walked in place. One element that
  changed with one resident waiter in its list and none in the wide list wakes that process directly
  (`Waiters::sole_resident`, the case `take_woken` already special-cased, without its call and closures).
  `rtl_chain` -8%, `rtl_ff` -0.7%.
- **The inline batch has one unwind guard** instead of one per process: a panic in process code still becomes an
  internal error of the process that panicked (`process_panic`, the index of the running process is kept), and the
  batch ends there as before. When every process of the batch was committed right after it ran, the commit loop is
  skipped. `rtl_ff` -1.1%, `rtl_fan` -2.1%, `rtl_chain` -2.5%.
- **Tried and dropped:** writing the driving value of a sole-source quick transaction when it is applied past the
  fused four (no `compute_driving`): level or worse, the gather of the drivers into plane words is as cheap as a
  write per element; a flag that the next delta's list holds a live entry (no `live` lookup): -1.3% on `rtl_chain`
  but +1.2% on `rtl_ff`; `#[inline]` on `Ctx::assign_code` with the general path split off, a zero-delay shortcut
  from the interpreter's `drive_bit_at` to `assign_code_now`, fixed-size stamp arrays in `update_planes`: no change.
- **What is left** (instructions per unit after this round): `rtl_ff` activation ~1910, of which the interpreter's
  `run_ops` ~1040 (19 ops at 55 each, dispatch ~10 and register bounds checks ~11 of them) and its activation wrapper
  (`InterpProcess::run`, `run_quick`, the wait, edge and statement ops) ~390; the kernel ~500 (run loop ~150,
  scalar assignment ~130, apply and update of the element ~220). `rtl_chain` delta ~2120: interpreter ~790, kernel
  ~1300 spread over the update (~560 including the quick apply), the run loop (~250), assignment (~130),
  `next_time`/`live`/`last_delta`/`run_until` (~140). `rtl_lfsr` activation ~3300: interpreter ~1900 (`run_ops`
  ~1600, `cat_words` ~230), kernel ~1250 (vector assignment ~205, apply ~190, effective value ~240, read ~120, run
  loop ~150). The reference simulator runs these in 960, 1000 and 1090 instructions: on `rtl_ff` the kernel is
  about half of that and the interpreter's per-op and per-activation costs are the larger lever; on `rtl_chain` and
  `rtl_lfsr` the kernel alone still exceeds it, spread over a dozen stages of 20 to 150 instructions each.

## Kernel round 11: direct paths for sole-driver registers (yavs-kernel, #440, 2026-10-22)

Round 10 found no single hot spot left: per activation the kernel was a dozen stages of 20 to 150 instructions
each, the same for every net. This round gives the common net shape its own path, chosen at elaboration, instead
of the generic queue, waveform and dirty-range stages. Output is unchanged at any thread count and with
`--shuffle`.

- **Direct drivers** (`DriverInfo::direct`, set by `SimControl::new`): the driver is the only source of its
  elements of a logic signal with no port, composite group, guard or user resolution function (std_logic
  resolution of one source is the identity). A zero-delay assignment of at most 64 elements to it, with an idle
  waveform, is a `ProcState::out` entry (signal, source slot, elements, plane words). The commit writes the
  driver value and the driving value at once and sets the elements in a per-signal bit mask (`Signal::pend`); no
  queue entry, no `live` lookup of the waveform, no apply stage. The next update finishes them a plane word at a
  time (`Signal::finish_pend`: activity, the changed elements by XOR of `cur` and the driving value, blend of
  `cur` and `last_value`, event stamps, runs, codes), one element through its code byte. Why writing early is
  safe: the update phase of the cycle is over at the commit, processes read neither a driver value other than
  their own (`'driving_value`, and the owner has run) nor the driving value, and observers between the phases
  only reach drivers through `Hook::assign` (which takes the general path, below).
- **Preemption stays exact**: a second zero-delay assignment to the same driver in a run is another entry and the
  last one wins, which is what zero-delay preemption gives; any assignment that needs the waveform (a delay, a
  waveform list, `null`, a phantom to flush) first moves the driver's entries into it in order
  (`Ctx::spill_out`), so the general rules apply to them.
- **Forces and deposits**: a signal with a force layer, or with a force or deposit in that cycle, takes the
  elements to its dirty ranges and the general update instead (the driving value then comes from the driver
  values again, which the commit wrote).
- **Activity nobody reads is not stamped**: with `Config::skip_unchanged` (nothing reads `'active` or
  `'last_active`, no observer) a signal that is no implicit signal's prefix (`Signal::inactive`) keeps no activity
  stamps on this path.
- **Batch loop**: the commit loop walks only the processes not committed right after they ran (`run_batch`
  `late`), not the whole batch; a process that skipped an unchanged assignment is committed there too (its
  phantoms only set `empty_delta`); the edge a signal resumes a process for is stored once in its state and a flag
  per activation says whether it holds (`ProcState::known_edge`, `edge_ok`), instead of a table lookup per run.
- **Smaller direct entries and register reads** (second commit): a one-element direct entry is 16 bytes (signal,
  slot, element, code; `ProcState::out`), vectors have their own list (`outv`), one driver's entries always in one
  list so their order holds; `Ctx::read_into` of at most 64 logic elements copies the plane words itself, inlined
  into the caller. `rtl_ff` -1.7%, `rtl_chain` -2.1%, `rtl_lfsr` -1.6%, `lfsr64` -3.6% instructions, neorv32
  (single core, 30 us) -2.4%.
- **The clock's processes run as one batch** (third commit): the cached wake list of a signal (all waiters resident,
  for the whole signal) is split once into its plain processes (one suspension point, not postponed, ascending, also
  grouped by trigger) and the others (`Waiters::cache_batch`, `SimControl::batch_of`). When that signal is the only
  one that changed and nothing else woke a process, the plain processes whose trigger holds become the batch as they
  are, without a `wake` per process (a resident waiter needs no new generation: no plain waiter or timeout of such a
  process exists); the others wake one by one and run after them as before. A falling edge that holds back every
  plain process touches only the others. The batch order is the one `wake` and `order` gave. `rtl_fan` -6.6%,
  `rtl_ff` -3.2% instructions.
- **A one-element commit keeps its code** for the finish (`Signal::last_commit`), and the runs of a direct signal
  are joined only when there are several. `rtl_chain` -1.6%.
- **The wake scan's buffer** is taken only on the path that scans (`rtl_chain` -1.0%).
- Tried and dropped: writing a plane code with four shifts instead of the mask table (+1.3% on `rtl_ff`);
  `wait_on_again` for every range of a multi-signal sensitivity in order (neorv32 -0.6%, the clocked designs +0.4 to
  0.7%: the extra state moved the interpreter's code).
- Measured over the round (instructions per unit, interpreter, slope of 1x and 10x runs, before to after): `rtl_ff`
  activation 1852 to 1633, `rtl_lfsr` 3164 to 2871, `lfsr64` 3423 to 3072, `rtl_fan` 842 to 750, `rtl_chain` delta 2132
  to 2078; under `--tier=auto` 1721 to 1551, 2463 to 2244, 2949 to 2675, 883 to 791, 2166 to 2115. Kernel per `rtl_ff`
  activation ~670 to ~510 instructions (signal reads included). Tables in docs/STATUS.md "Kernel round 11".
- What is left: the kernel per activation is the same dozen stages of 15 to 120 instructions, now each the cost of the
  generic storage (`Buf` enum, `SmallVec`, bounds checks per access); reaching ~350 per `rtl_ff` activation needs a plain
  plane-word layout for direct signals or one kernel call per activation from the process side (#507).

## Kernel round 12: unchanged values, lagging driver values, one-process deltas, kernel copies (yavs-kernel, yavs-elab, #440, #529, 2026-10-22)

Round 11 left a dozen stages of 15 to 120 instructions per activation, most of them the generic storage (`SmallVec`
and `Buf` layers) and the per-batch and per-delta machinery. This round measured the kernel alone (a bench of
hand-written processes, exact counts from Intel PT traces of short stepped runs) and took ideas from the reference
simulator's kernel (sim1): a zero-delay assignment of the value a driver already has schedules nothing; a net with
one source keeps no separate driving value; the waiters of a value are woken when it changes; a plain signal copy
has no process body. Output is unchanged at any thread count and with `--shuffle`.

- **Unchanged direct values** (`Signal::commit_code`): a one-element direct entry whose code the driver and the
  signal already have writes nothing at commit, on a signal whose activity nothing reads (`skip_ok`); its delta
  still happens (`empty_delta`). Vectors are committed as before (the check cost more than it saved on registers
  that change). A design whose registers keep their value: -4.7% instructions.
- **One pending element** (`Signal::pend_one`): the first one-element entry of a signal in a cycle stays in
  `last_commit`; a second one, or a vector, moves it to the bit mask. `finish_direct` finishes such a signal before
  its loop over the general cases (`finish_pend_one`).
- **The driving value is the driver's value** (`Signal::lag`): a direct commit writes only the driving value.
  Before anything reads the sole-source drivers or makes the driving value differ from them (a force, release or
  deposit applied to the signal, the dirty-range path of `finish_pend`), `sync_drivers` copies the driving value
  back; `'driving_value` of a direct driver reads the driving value while the signal lags. A deposit makes the
  signal `exact`: its commits write the driver value again. A test runs forces, releases and deposits between
  direct commits against the same design on identity-resolved signals (no direct driver): same trace.
- **Runs of one signal**: consecutive direct entries of one signal (a vector or an array of vectors with a process
  per element) are committed with one lookup of the signal and its storage (`commit_codes`, `commit_planes_run`).
- **Wake at finish** (`SimControl::wake_sole`): one changed element with one resident waiter without an edge
  trigger is woken in `finish_direct`, without the changed list and the scan in `cycle`; not with an observer of
  value changes.
- **A batch of one process** (`SimControl::run_one`): no order, batch loop, late list or unwind guard around a loop;
  the clocked commit takes the direct entries from the process's own lists (`commit_rest` is the old tail).
- **Lists**: the per-process wait, waveform and schedule lists, a driver's pending transactions and
  `Signal::changed` are `Vec`s (a `SmallVec` access tests where its data lives on every use); the commit swaps the
  new wait into `sens_prev` instead of copying it. Memory is level (45 bytes more per process, 56 less per driver).
- **Kernel copies** (#529): a concurrent `target <= source;` of a static signal name, part for part, with no delay,
  guard or `after`, is `ProcInst::copy`; `build` runs it as `yavs_kernel::CopyProcess` when nothing observes
  activations (`Config::edge_triggers`), reading each source part and assigning it to its driver, then keeping the
  wait with `Ctx::wait_same` (the several-range form of `wait_on_again`). neorv32: 381 of 1879 anonymous
  statements; -2.6% instructions.
- **Tried and dropped**: the unchanged-value test at assignment time with a phantom (as costly as the commit it
  saved); the same test for vector entries (+1.6% on `rtl_lfsr`); `#[inline]` on `Ctx::assign_code` with the
  general path split off (+0.6%); inlined phase and callout checks (+1%); `finish_signal` out of line (no change).
- **Measured** (instructions per unit, slope of 1x and 10x runs, core 5; `main` with the round reverted against
  `main`): interpreter `rtl_ff` 1590 to 1495, `rtl_chain` 2064 to 1684 per delta, `rtl_lfsr` 3679 to 3568, `lfsr64`
  2869 to 2756, `rtl_fan` 737 to 725, `lfsr64_xor` 4884 to 4771; `--tier=auto` 1538 to 1443, 2106 to 1726, 3725 to
  3615, 2524 to 2414, 781 to 769, 4638 to 4530. Kernel alone (hand-written processes): a chain delta 1325 to 946 instructions, a clocked register activation 472 to 363 (each including 70 to 100 instructions of the hand-written process itself)..
- **What is left**: per `rtl_ff` activation the kernel is the batch loop (~95), the assignment (~90), the commit
  (~45) and the finish (~15) plus reads; per chain delta the finish and wake (~250), the batch of one (~150), the
  commit (~125), the assignment (~90) and the cycle skeleton (~200). The interpreter is now the larger part of both
  (`rtl_ff` about two thirds). `rtl_lfsr` lost a third to an optimiser change (#534).

## Kernel round 13: delayed assignments, byte signals (yavs-kernel, yavs-interp, #440, 2026-10-22)

Every testbench has a clock, and the head-to-head designs (4096 registers on one clock) hide what one costs: a bare
`c <= not c after 5 ns` took about 4000 instructions per edge against about 1000 in the reference simulator, and the
testbench of a timestamp counter with three clocks (`sc5_timestamp`) spent 65% of its time in the kernel. A delayed
assignment always took the general path (a `LogicVec`, a transaction list, `Waveform::schedule`); only zero-delay
assignments had the quick shapes.

- **A delayed assignment to an idle driver is a quick transaction** (`Ctx::assign_code_fast`, `assign_planes_fast`,
  `Ctx::fast_time`): one element, or up to 64 logic elements, `after T` (transport, or inertial without a rejection
  limit) to a driver with nothing pending and no skipped assignment is `Waveform::quick` / `vquick` with time
  `now + T` and a queue entry at that time. With nothing pending there is nothing to preempt, so the one transaction is
  exact for transport and inertial alike; anything else (a pending transaction, a rejection limit, a time past
  `TIME'HIGH`, a delay that quantizes to zero) takes the general path, which also produces every error. A direct driver
  takes it only when the run has no direct entry for the driver yet (`Ctx::direct_entries`): those commit at the end of
  the run, before the delayed one, and inertial preemption must see them. `Waveform::apply_due`, `Signal::apply_driver`
  and `first_time` already handled a quick transaction with a later time; `tests/direct_paths.rs` runs random mixes of
  zero-delay and delayed assignments against the same program with an explicit rejection limit (general path).
- **Byte signals have a quick transaction too** (`Waveform::bquick`, `Ctx::assign_bytes`, `Signal::apply_bquick`): one
  element of at most 8 bytes (an enumeration, an integer, a boolean, a real, a time) assigned to an idle driver that
  is the signal's sole source is kept as the bytes, with zero or a positive delay; on a signal with no layer, group,
  guard or resolution function, applying it writes the driver, the driving and the effective value, the event stamps
  and `'last_value` at once (`Signal::finish_bquick`). An unchanged zero-delay assignment is skipped as for logic
  (the delta stays). Before, an enumeration or integer register took the general path: a `Value` per assignment, the
  transaction list, `compute_driving` and `update_effective`: 3.0 and 3.6 times the reference simulator's instructions
  on 2048 registers of each, against 1.5 for `std_logic`. The interpreter reads a scalar byte element through
  `Ctx::elem_bytes` (no reference to build) and passes the assignment's bytes as they are in memory. A test compares
  random zero-delay and delayed assignments on plain signals with the same program on identity-resolved signals
  with explicit rejection limits (`tests/direct_paths.rs`).
- **Time-step bookkeeping** (measured with `crates/yavs-kernel/examples/clk.rs`, three hand-written clock generators,
  no interpreter in the count): the future queue is a small binary heap of three words per entry (`TimeHeap`, ordered by
  time then driver, timeout, implicit kind and index, as `Entry` was) because the generic heap moved a 24-byte item
  that the caller had stored in narrower pieces (a store-forwarding stall per push and per peek after a pop);
  `last_delta` asks for the next time once when no observer wants the phases between (it asked three times per time
  step); `update` returns a plain `Ok(())` when there is no implicit signal (the `Result` was built on the stack and read
  back wide); `update_independent` updates several signals one after the other, with no job and result lists, when
  the pool would not take them anyway (fewer than 256 due drivers; the same order, so the same errors). A copy
  process of a byte signal uses `assign_bytes`.
- **Tried and dropped**: update of heap entries through the fused in-place path of `update` (`from_delta == 0`): +2%
  instructions on a clock generator, the independent-signal route is cheaper.
- **Measured** (instructions, core 4/5, interpreter, `main` 4a586fd3 against the commit; the bare generator is three
  clocks for 6 ms): bare clock generator 14.36 G to 10.56 G, with `when not done` 15.05 G to 11.3 G; `sc5_timestamp`
  14.68 G to 12.82 G (wall 1.13 s to 0.96 s); head-to-head designs unchanged.

## Kernel round 14: clock generators in the kernel, the clock edge path (yavs-kernel, yavs-elab, #440, 2026-10-22)

Round 13 left a clock edge at ~1900 kernel instructions plus 600 to 700 of interpreter for the process `x <= not x after T;`
(238 clock generators of the corpora are this form) against ~1000 for the reference simulator's whole edge. Ranked by the
baseline (an exact per-function count of the instructions of one edge, from a single-stepping tracer over `examples/clk.rs`):
the native process first, then the edge path itself.

- **`ToggleProcess`** (kernel, `process.rs`), built by the elaborator (`ProcInst::toggle`, `Walker::toggle_source`) for a
  concurrent `x <= not x after T;` (also `transport`; not guarded, postponed or with a rejection limit) on one element of a
  signal of `std_ulogic`, BIT or BOOLEAN itself, with the IEEE `not` of `std_ulogic` or the predefined one of BIT or BOOLEAN
  on the same static element, a static delay that evaluates at elaboration to a positive time without calling a
  subprogram, and the process's one driver exactly that element. Anything else (a signal-dependent or function-computed
  delay, zero or negative, a constrained subtype, another source, a vector `not`) stays an interpreted process, so every
  error message and every LRM corner is the interpreter's. Like the plain signal copy it is built only when
  `Config::edge_triggers` is on, i.e. when nothing observes activations (coverage, a debugger breakpoint on that line,
  foreign callbacks). The process reads the element (`elem_code`), assigns the complement and waits again
  (`wait_on_again`); the assignment is `Ctx::assign_later` (the checks of `assign_code_fast` once, `T` truncated to the
  resolution once) and falls back to `assign_code` for anything unusual (a pending transaction of a second source's
  event, a delay that truncates to zero). Wake-ups, forcing, `'event`, `'last_value` and the waveform output are the
  kernel's own, so they are those of the interpreted process: `tests/toggle.rs` compares the two in the kernel (alone, with a
  second source, with a delay below the resolution), `clock_generators_run_in_the_kernel` in the elaborator on a design with
  `std_logic`, `std_ulogic`, BIT, BOOLEAN, a vector element, `transport` and the shapes that must stay processes.
- **Clock edge path** (ideas from the reference simulator's kernel: timed events are one heap of driver pointers drained
  in one pass per time step, the waiters of a value are woken when it changes, an empty phase costs nothing): (1) `last_delta`
  no longer runs `run_batch` for an empty postponed list (~180 instructions, and `last_ran` kept); (2) a cycle whose one due
  entry is a driver of an independent signal in the future queue pops the heap and applies the transaction in place
  (`update_timed`; with a second item due see round 17), and finishes it without `finish_signal` when nothing else is
  pending; (3) a signal with one waiter list of at most 8 resident, alive waiters wakes them in `cycle` without
  `take_woken` and its scan (`Waiters::small_list`; any stale or plain waiter takes the scan as before), and `build_cache`
  is asked only for 16 or more waiters; (4) `Ctx::assign_later`. `examples/clk.rs` (one clock, kernel alone) goes from 2032
  to 1330 instructions per edge.
- **Direct byte transactions** (round 13 item 5): a zero-delay assignment of one element of at most 8 bytes of a byte
  signal to its sole-source driver is a per-process entry committed at the end of the run, as the logic ones are
  (`OutB`, `DriverInfo::bdirect`, `Signal::commit_bytes`, `finish_pend_bytes`): no queue entry, no `apply_bquick`, no
  dirty range, no `finish_applied`. Same rules as the logic direct path: the driving value is the driver's value
  (`lag`), an unchanged value writes nothing and only makes the delta happen, a force layer or a force or deposit in the
  cycle takes the element to the dirty ranges, anything that needs the waveform spills the entries first. Tests compare
  the direct path with the same program on identity-resolved signals (forces, releases, deposits, `'driving_value`,
  repeats, delayed assignments in between) and `skip_unchanged` on and off. An integer register (`n <= n + 1`) goes from
  3830 to 3500 instructions per activation of which the interpreter has ~2650; `ff_rtl` 0.55, `rtl_ram` 0.88, `lfsr64` 0.97
  of the instructions, the other head-to-head designs within 1%.
- **Tried and dropped**: event records (`Signal::blk_event`) for signals of 8 to 64 elements when many elements change
  (the stamp loop was half of `update_word` of a 32-bit register): +400 instructions per update with random data, since the
  superseded records pile up (each update leaves the elements that did not change in the older ones). Signal fields in
  the order the commit and finish paths touch them, with and without 64-byte alignment: neorv32 cycles within 1% (the
  thousands of signals are in L2/L3 whatever the order).

## Kernel round 15: edge spellings, the kernel part of a clock edge (yavs-kernel, yavs-elab, #440, 2026-10-23)

- **Every spelling of a clock edge gets the edge trigger** (user requirement: no form favoured over `rising_edge`).
  `Walker::edge_trigger` now takes `rising_edge`/`falling_edge` of BIT and BOOLEAN (the predefined functions of
  STANDARD), `x'event and x = '1'` and the other legacy shapes on BIT (the literals of BIT and BOOLEAN are looked up as
  their storage codes, which is also right for a BIT or BOOLEAN reset level), `not x'stable and x = '1'` (a stable time
  of zero; `'stable` is read through its implicit signal), the plain level `wait until x = '1'` / `'0'` (a trigger of the
  legacy kind: an event ending on the level; allowed when the `on` list, implicit or written, names only that element),
  and the edge as a later operand of an `and` when the earlier ones are levels of static signal names (they cannot fail,
  so skipping them on a false edge changes nothing). `tests/corpus/process/edge_forms.vhd` runs every form on std_logic,
  std_ulogic and bit (the first reference simulator runs the process of `x'event and x = '0'` as true once at time 0, which the
  second one and the LRM do not: `ref=hand`). Not covered, and why: `wait on clk; if rising_edge(clk)` (two
  statements), `x'event and x` on BOOLEAN, `x'last_value = '0'` spellings, and the edge as the second operand after an
  operand that can fail. Costs the trigger does not change: `x'event and x = '1'` is stepped out of the interpreter's fast
  loop (about 380 instructions more per activation than `rising_edge` in `rtl_ff`), and every `not x'stable` is an
  implicit signal the kernel updates per event (4.4 times the instructions of `rising_edge` on `rtl_ff`).
  `Signal::event_in` of one element is a compare instead of a scan.
- **A clock generator re-arms itself** (`SimControl::rearm`, `Process::generator`). When the transaction of a
  `ToggleProcess` driver matures through the quick path of `update_timed` and makes an event (so the signal is plain
  and the generator its sole source), the kernel does what the process would do on that event: the next quick transaction
  with the complement of the new value, its queue entry, and the process stays suspended. The process is not woken: its
  `ready` flag is set until the end of the cycle's wake phase (`wake` ignores a ready process), it is taken out of the
  clock's cached batch (`skip_wake`), and a signal whose only waiter it is skips the waiter scan (`Waiters::only_resident`).
  Anything else keeps the process: a force, deposit or release (the general update path), a second source, a delay that
  needs `assign_code`, a process whose waiter is not resident yet (the first edge), two entries due at one time. A forced
  clock resumes its process on that event and the process preempts the pending transaction as before
  (`tests/toggle.rs`, native and closure traces equal through deposits, a force and a release). `stats.process_runs` no
  longer counts the generator's edges. `examples/clk.rs` one clock, kernel alone: 1330 to 915 instructions per edge;
  `sc5_timestamp` (three clocks) 0.926 of the instructions.
- **An empty batch is skipped** in `cycle`: nothing resumed (the falling edge of a clock, a transaction nobody waits for)
  means `run_batch` would only commit nothing; it still ends `batch_mono` (the postponed batch after it must be
  ordered, `tests/scenarios.rs`).
- **Direct entries have a per-driver mark for a process of many drivers** (`ProcState::marks`, more than
  `MARKED_DRIVERS` = 4 drivers): "this driver has `out`/`outv`/`outb` entries in this run" was a scan of the run's entry
  lists at every assignment, O(entries) in a CPU's processes. The mark is the run number (bumped at every resumption) and
  the kind of entry: no clearing, a mark of an older run means none, and a stale mark of the same run only sends the
  assignment to the general path. A process of up to four drivers has no marks and scans its short lists as before (a
  mark per driver in `ProcState` cost one-bit registers 1% in cycles: the slot stride of 4096 processes grew). neorv32
  2.3% fewer instructions.
- **The wake list of a clock keeps up to 32 distinct edge triggers** (was 4) and its batch is sorted: a CPU's clock has a
  trigger per reset signal and edge, so the 375 waiters of neorv32's clock never made the cache and the clock was
  scanned waiter by waiter at every edge (`take_woken` and `build_cache` were 3.5% of the instructions, 3.0% fewer
  instructions now). The batch of plain processes runs in process order whatever the order the waiters registered
  in (`order` sorts such a batch anyway), and once each (a process waiting twice on a signal ran twice in the cached path).
- **A wide bus whose waiters all wait for all of it lists one covering run** (`Signal::fine`, `cover`): `changed` holds
  the exact runs only when a process waits for a part of the signal or an observer wants the value changes; else it
  holds the cover of the runs and `nruns` counts them (`stats.events` counts the cover once more). The 512-bit
  pipeline stages of `fletcher_bufferwriter_512` changed ~64 runs per update and each was noted, then looked up in
  the waiters. A signal whose waiters are all in the wide list (`Waiters::only`, whatever the number of chunk lists
  it has) takes the short wake paths of a one-list signal, and an event looks only at the chunk lists that have
  waiters (`chunk_ix`, up to 16) instead of every chunk it spans. `fletcher_bufferwriter_512` 8.5% fewer instructions
  together.
- Small ones, each counted: `Signal::static_plain` (the part of `plain` that cannot change: 12 instructions per quick
  update), `To_X01` through a table, the `small_list` wake without a list of woken processes (`wake_proc` on the fields),
  a one-entry commit in `run_one` (a chain stage 27 instructions), `wait_on_again` inlined into the interpreter (8 per
  activation), one `OutV` pushed instead of copied.
- **Tried and dropped**: committing a process's direct entries right after it runs (instead of after the batch): 17
  instructions fewer per activation for a changed order of commit against the other processes of the batch.
- **Measured** (aligned bench builds, `main` 897d19ad against this commit; instructions of `--tier=interp --no-cache`
  runs, then `perf stat` cycles:u, minimum of five alternating runs on cores 10 and 11): head-to-head designs
  instructions `rtl_ff` 0.998, `rtl_lfsr` 0.991, `rtl_chain` 0.988 (cycles 0.972), `rtl_fan` 0.993, `rtl_bus` 0.997,
  `rtl_ram` 0.989 (0.981), `lfsr64` 0.990; real-world suite instructions / cycles: `neorv32_single_min` 0.936 / 0.957,
  `fletcher_bufferwriter_512` 0.915 / 0.938, `sc5_timestamp` 0.918 / 0.933, `poc_cache_cpu` 0.990 / 0.963,
  `sc5_fifo_repack` 0.999 / 0.967, `fletcher_bufferreader` 0.991 / 1.005, `hdlm_lfsr` 0.989, the rest 1.000 within 0.1%.
  A bare clock edge (`examples/clk.rs`, kernel alone): 1300 to 912 instructions.

## Kernel round 16: idle activations of pure clocked processes (yavs-sem, yavs-elab, yavs-kernel, #440, 2026-10-23)

User decision 2026-10-23 (the study of another simulator's idea: skip a clocked process whose inputs had no event since its last
run). Measured first: a clocked activation assigned nothing, or the value the signal had, in 75% of neorv32's 17 M resident
activations, 41% of `fletcher_bufferreader`'s, 22% of the writer's, 63% of `poc_cache_cpu`'s.

- **Which processes** (`Db::pure_clocked_reads`, `Walker::idle_reads`): a process with an edge trigger of the shapes
  `process (c) begin if <edge> then B end if; end process;` (also `if R then A elsif <edge> then B`) and
  `process begin wait until <edge>; B end process;`, whose declarative region has no variable or file, whose statements are
  signal assignments without delay (also conditional), `if`, `case`, `for` loops, `next`, `exit` and `null`, and whose
  expressions have constants, signals (not implicit ones or GUARD), predefined operators, conversions and pure functions of
  packages without a signal parameter: an activation is then a function of the signals it reads and of constants. What the
  edge test also reads (`rising_edge(c) and en = '1'`) is read too. The signals read are whole objects and record fields
  (an index or a slice is not worked out per instance: the process is armed by a change anywhere in the array); the
  analysis is made once per process statement (`idle_cache`).
- **Arming** (`SimControl::arm_readers`, `readers`): the clocked processes that read a signal are armed when it has an event
  (the cycle's changed signals, and `wake_sole`, which hides a change from them), and by a deposit on a signal they drive (the
  value a deposit leaves is not what they assigned). A process that runs is disarmed at the end of its plain activation
  (`note_idle`), which also records whether it assigned anything (`idle_state` 1 or 2). A process that is woken
  (`idle_filter`, before the batch) with `!armed`, a resident waiter and a recorded state stays suspended.
- **Why it is the same run.** The activation would do what the last one did: same values to the same drivers. On a signal that
  allows skipping (`skip_ok`, so every signal the process drives must; `Config::skip_unchanged`, `edge_triggers`) that queues
  nothing and changes nothing, except the delta cycle an assignment of an unchanged value makes
  (`empty_delta`): the skip makes it when the last activation assigned (`idle_state` 2). Forces and releases do not change
  what the driver holds; a deposit arms. `Config::idle_check` (`YAVS_IDLE_SKIP=check`) runs the activations it would skip and
  fails (an internal error naming the process) when one assigns a value that is not the signal's current one (the net value
  of its entries: a field set after `r <= r_next` counts once) or assigns after a quiet one. Under it the whole corpus
  (452), the real-world suite, OSVVM and UVVM runs are the same. `YAVS_IDLE_SKIP=off` runs every activation.
  `tests/idle_skip.rs` (the run with the skip, without it and checked: same changes and phases, deltas included; mutations of
  the arming, the deposit and the delta were each caught), `perf.rs` `idle_regs` (a floor on `idle_skips`) and
  `idle_skip_changes_nothing` (all spellings of the edge).
- **Tracking pays or stops** (`IDLE_WINDOW`): a tracked activation costs ~40 instructions and a skip saves ~1000, so a process
  skipped in fewer than 1 of 32 of its activations over a window of 128 stops being tracked (`idle_ok` false), and with
  the last of them the arming and filtering stop (`idle_active`); every 65536 cycles they are tracked again
  (`idle_reprobe`). Without that the 4096 always-changing registers of `rtl_ff` paid 5%.
- **Only the serial path** records the state: a batch run in the pool (threads > 1, 32 or more processes) commits through
  `commit_rest`, which forgets it. The same run either way; only `idle_skips` and `process_runs` depend on the threads
  (changed in round 17: the pool path records it too).
- **Not covered**: processes with variables (`hdlm_lfsr`, most testbenches), calls of user subprograms, `report`, `assert`,
  delayed assignments, `wait` anywhere but first, postponed processes.
- **Measured** (aligned bench builds, same commit with the skip off and on; instructions, then `perf stat` cycles, minimum of 3
  to 5 alternating runs on cores 10 and 11): `neorv32_single_min` 0.745 / 0.689 (8.3 M of 18.1 M activations skipped),
  `fletcher_bufferreader` 0.80 / 0.82, `fletcher_bufferwriter_512` 0.97 / 0.96, `sc5_fifo_repack` 0.96 / 0.96, `poc_cache_cpu` 1.00 /
  0.99, `sc5_timestamp` 1.01 / 1.00, `rtl_ff` 1.006 / 1.007, `rtl_lfsr` 1.002 / 1.000.

## Idle skip, round 2: combinational processes, clocked concurrent assignments, variables (yavs-sem, yavs-elab, yavs-kernel, #521, #523, 2026-10-24)

The profile of the CPU designs of the real-world suite (`neorv32_*`, `poc_cache_cpu`) put most of the interpreter's time into
activations that did what the last one did: a process with a wide sensitivity list woken by an event on a part of a record it never
reads (`counter_increment` of neorv32, `ctrl_i` has 41 leaves and the body reads two), and the registers of `poc_cache_cpu`, written
as concurrent assignments `q <= f(d) when rising_edge(clk);`, which had no edge trigger (58% of their wakes were events on the
data inputs, the edge was false) and, as a spelling of a clocked process, no idle skip (round 16 knew only `process`).

- **Edge trigger for a concurrent conditional assignment** `target <= wave when <edge>;` (one waveform, no `else`, not guarded or
  postponed): it does nothing on a false edge, as `if <edge> then target <= wave; end if`, so the kernel need not resume it
  (`Walker::edge_trigger`, the same edge spellings as for a process). `poc_cache_cpu`: 0.7 M of 1.2 M activations of these gone.
- **Idle skip for it** (`Db::pure_conc_reads`): the reads are those of the waveform and the target's indices, not the edge; the
  delay is the default or `transport` and there is no `after`.
- **Idle skip for combinational processes** (`Db::pure_comb_reads`, `Walker::idle_comb_reads`): `process (list)` and `process (all)`
  of the pure shape of round 16 (no `wait`). A wake that no signal the body reads had an event for, since the last activation,
  leaves the process suspended: the activation would assign what the last one did. `Builder::idle_skip` no longer needs an edge
  trigger, nor does the kernel (`SimControl` builds the readers of a process that has `idle`).
- **Variables**: a process variable is allowed in the pure shapes when each branch that one activation runs assigns all of it
  before anything else mentions it (the first statement of the branch that does is a top-level `v := ...;` that does not read
  it: `Db::vars_assigned_first`), so that no value is kept from one activation to the next. A process that counts in a variable, or
  assigns it in one arm of an `if` and reads it after, is not pure and runs every time (`tests/corpus/delta/idle_comb.vhd`).
- **Functions**: a pure function declared in an entity, architecture, block or generate is accepted like a package function, when
  its body passed the purity check: E1830 stops a pure function from reading a signal or variable outside it, but under
  `--relaxed` it is only a warning (W1830), so the check records the failure in `Subprogram::impure_reads` (analysed-unit cache
  format 6) and a function with it is not trusted, in a package either (it was, since round 16: #682).
- **The same run**: `YAVS_IDLE_SKIP=check` runs the activations the kernel would skip and fails when one assigns something else;
  the corpus (503 tests; interpreter, `--tier=auto`, `--tier=llvm`; 1 and 4 threads; checked) and the output of the 41
  real-world testbenches (also checked) are the same as without. `tests/corpus/delta/idle_comb.vhd` (blessed from the reference
  simulator) and `perf.rs` `idle_comb`, `idle_skip_changes_nothing` (off, on and checked on both designs).
- **Measured** (`tools/realbench --compare` of main and the change, core 15, medians of 5, run phase, interpreter / `--tier=auto`;
  instruction ratio in brackets): `poc_cache_cpu` 1.64 to 1.10 s (0.67, 0.63) / 1.59 to 1.06, `neorv32_dual_full` 4.16 to 3.66
  (0.88, 0.84) / 4.17 to 3.58, `neorv32_single_min` 5.25 to 4.79 (0.91, 0.90) / 5.27 to 4.80, `neorv32_single_cache` 4.95 to 4.39
  (0.89, 0.85) / 4.95 to 4.38; `hdlm_lfsr` and `sc5_fifo_repack` unchanged (instructions 1.00, 0.99). Instructions over the 41
  real-world designs, geometric mean 0.979 (every design but those four within 0.5%), over the 11 head-to-head designs 1.000: a
  tracked process nobody skips costs nothing visible (`IDLE_WINDOW` stops tracking it).
- **Not covered**: a pure process whose branch reads a signal it does not always use (`counter_core` of neorv32 reads `data_i` in
  one arm: the CPU bus changes it every instruction), user functions that are not pure or take a signal, `report`, `assert`, and
  the rest of round 16's list.

## Idle skip, round 3: reads under a condition (yavs-sem, yavs-elab, yavs-kernel, #521, 2026-10-24)

A register with an enable reads its data only while the enable is set (`if en = '1' then q <= d;`), and the 29 counters of neorv32
read the bus data `data_i` in one arm of an `if` while the CPU puts a new word on it every instruction: round 2 armed them on
every one of those events (`counter_core`: 652 k activations in 300 us, 5% of the time inside processes).

- **What is recorded** (`Db::pure_*_reads` return `PureRead`s): with each signal a pure process reads, the condition under
  which it is read: the conjunction of the literals of the `if`s around the read, `x = c`, `x /= c`, a BIT or BOOLEAN signal
  name, `not` of one and `and` of those (`Db::cond_lits`, `Db::level_cmp`), and for the `else` and `elsif` arms the negation of
  an earlier condition that is one literal. Several reads of one signal join as alternatives (more than four: unconditional).
  A condition the walk cannot express adds no literal: a weaker condition only makes the read count more often.
- **Why it is the same run.** An activation does what the last one did when every signal the last one read is unchanged. A
  signal read only under a condition was not read when the condition was false at the last activation, and the signals
  the condition reads are read unconditionally (or under the outer condition, by the same argument), so a change that
  makes the condition true is an event on a signal that arms the process itself. The kernel evaluates the literals when it arms
  the readers of a changed signal (`SimControl::arm_readers`, `guard_holds`): an event on a conditionally read signal arms
  the process only while one of the alternatives holds; a literal on a value that has not been updated yet in the cycle
  (the update phase is under way) can only be older than the activation's, and the change that outdated it arms by itself.
  The elaborator maps each literal to a static element of a logic signal (`IdleRead`, `IdleLit`; collapsed ports remap them);
  one it cannot map is left out. `YAVS_IDLE_SKIP=check` and `tests/corpus/delta/idle_comb.vhd` (registers with an enable, an
  `if`/`else`, nested conditions, an `elsif` chain, BOOLEAN conditions and a conjunction; blessed from the reference simulator)
  cover it; `perf.rs` `idle_comb` pins the skips.
- **Measured**: see "Idle skip, round 3" in `docs/STATUS.md`.

## Kernel round 17: coincident timed entries (yavs-kernel, #440, 2026-10-23)

The baseline of this round (exact per-function counts from a ptrace single-stepper over `examples/clk.rs` and over windows of
`sc5_timestamp`, `neorv32_single_min` and `rtl_fan`, `perf record` cycles of the real-world suite) put the kernel at 37 to 40% of
the cycles of the CPU and fletcher designs, 63% of `sc5_timestamp` (three clocks) and 54% of `rtl_chain`. A bare clock edge is 933
instructions of the kernel (`examples/clk.rs`), so the edge path of round 15 holds; the cost the exact counts found next was the
general update path for every cycle where two clocks edge together (`update_general` 400000 of 3.25 M cycles in
`sc5_timestamp`, 4700 instructions per such cycle against 2800 for a lone edge).

- **A cycle with several timed entries due stays on the timed path** (`SimControl::update_timed`, `update_timed_many`): up to four
  entries popped from the future queue, each a driver of a distinct independent signal (applied in signal order, as the general
  path does, an error taken at the first signal that has one after all were updated) or a timeout (the process wakes if its
  generation matches). An implicit-signal entry, a signal with a port or group, two drivers of one signal, a fifth entry: the
  entries popped so far go back on the heap and the general path runs. Every generator that makes an event re-arms itself
  (`rearm`), so `skip_wake` is a list of up to four processes (`skip_n`); a lone due entry is the old path. `perf.rs`
  `clk_together` (three clocks that edge together, a process whose timeouts fall on the edges): `update_general` 68 to 0.
  `tests/toggle.rs` `clocks_that_edge_together`: four generators (two with equal periods) with readers, a timeout process, a
  deposit and a force and release on one: native generators and their closures give the same trace (and the same trace as
  the kernel before this change, checked once by running the test on both).
- **Processes of up to 2048 registers are resident** (`RESIDENT_REGS`, was 256; `yavs-interp`): the bigger processes of a CPU
  design swapped their register file with the thread's scratch at every activation (`borrow_fill`: a copy of the pristine
  frame from a template that is cold in the cache, `memmove` 14% of neorv32's L1 data misses). The 96 MB budget stays;
  peak memory of neorv32 is unchanged. Instructions / cycles (bench builds, core 8, min of 3): `neorv32_single_min` 0.990 /
  0.977, `fletcher_bufferreader` 0.977 / 0.951, `fletcher_bufferwriter_512` 0.989 / 0.973, `poc_cache_cpu` 0.999 / 0.977;
  1024 registers gave back the neorv32 gain, 8192 no more.
- **A wide vector assigned to a direct driver is one entry per plane word** (`Ctx::assign_wide_fast`, from `assign_blocks`): a
  zero-delay assignment of more than 64 logic elements to a driver that is the only source of its elements of a plain
  logic signal (`DriverInfo::direct`) with nothing pending pushes an `OutV` per 64 elements, committed at the end of the run
  as the entries of a short vector are (`commit_planes_run`, `finish_pend`). It took the general path before (a `LogicVec`,
  a transaction, `schedule`, `apply_txn`, `compute_driving`, `update_effective`, `mark_active`: ~300 of the 3000
  instructions of a 512-bit update in `fletcher_bufferwriter_512`, 10% of a stepped window). `fletcher_bufferwriter_512`
  0.957 instructions / 0.893 cycles (bench builds, core 8). The ranges of one cycle's changes may be listed in another
  order than the general path lists them when a deposit is applied in the same cycle (`tests/delta_paths.rs`
  `wide_registers_are_direct_entries_per_plane_word` compares the set and the reports, forces, releases and deposits
  included). What is left of that design: the per-word update (`update_word`, ~470 instructions per changed plane word,
  half of it the event records of `blk_event`), 17% of its cycles.
- **More per-signal lists are `Vec`s** (`Signal::dirty`, `fast_dirty`, `eff_dirty`, `force_ev`, `pend_words`, the outer list of
  `Waiters`): the same trade as round 12 (a `SmallVec` access tests where the data lives; an empty `Vec` allocates nothing).
  Instructions against the tip before them, bench builds: `neorv32_single_min` 0.990, `fletcher_bufferreader` 0.989,
  `rtl_chain` 0.989, `sc5_timestamp` 0.987, `poc_cache_cpu` 0.996; cycles level within the noise (minimum of 3 to 7 runs, core 8);
  peak memory of neorv32 -1 MB, `elab_heavy` level. The list of generators skipped in a cycle (`skip_wake`) is cleared only when it has entries (it had cost `rtl_chain`
  3 instructions a delta).
- **Measured and not taken** (numbers in `docs/STATUS.md` "Kernel round 17"): merging the processes of a clock into one batch
  (at most 1.5 to 6.7% of the instructions of the real-world designs, needs shared frames and error-order care); the idle skip
  for processes with variables or constant-written variables (under 3% of the activations that were not skipped already, none
  in `hdlm_lfsr`); `#[inline(always)]` on `Machine::run_quick` (-4% instructions of the near-idle `rtl_fan`, +0.3% of the CPU
  designs and a worse cycle geometric mean: the interpreter rule says no); a hoisted per-word update for wide buses and an
  inline record list (-0.8% instructions of the 512-bit design, cycles level); a cheaper empty delta cycle and a sorted array
  for the future queue (~130 and ~25 instructions of ~2300 per cycle of a three-clock testbench). Idea of the reference
  simulator's kernel used (rewritten for ours): one pass drains every due heap event of a time step (here several timed
  entries: a clock edge and the timeouts that fall on it).
- **The idle skip records its state on the pool path** (`commit_rest`): a plain activation of a tracked process calls
  `note_idle` before the commit takes its lists, as the serial batch loop does, instead of forgetting the state. Runs with
  4 threads skip the same activations as runs with 1 (`idle_skips` and `process_runs` equal; `tests/idle_skip.rs`
  `skips_in_the_pool_are_those_of_the_serial_run`, a mutation that drops the call fails it).

## Kernel round 18: lean event bookkeeping of wide signals (yavs-kernel, yavs-elab, yavs-sem, #519, #440, 2026-10-24)

User decisions of this round (2026-10-24, from round 17's questions): `RESIDENT_REGS` stays 2048; the kernel does not forward plain
signal copies for now; wide buses get a different event bookkeeping (below); the exact-instruction measuring tools are committed
(`tools/stepcount`, `tools/stepcount/AGENTS.md`).

The cost left in `fletcher_bufferwriter_512` after round 17 was the per-word update of its 512-bit bus (`Signal::update_word`): a
stepped window of 3 M instructions puts ~100 instructions per changed word in the event records of a block (`blk_event`: the
record list of a block with random data holds ~7 records, each changed word walks, masks and prunes it, and the `SmallVec` spills to
the heap past four) and ~12 in the blend of `last_value`. Removing both gave an upper bound of -6.2% instructions and -9% to -11%
cycles for the design.

- **A wide signal can be lean** (`Builder::lean_events`, `Signal::lean`): it keeps one record per block of 64 elements, the block's
  last event (cycle, time stamp, the elements that changed in it, `Signal::blk_last`), instead of the pruned list. Everything but two
  questions stays exact: `'event` and `'active` of any range, `'last_active`, edge tests, `'last_value` of one element (the plane
  holds it per element) and the answers of the other signals. `'last_event` of a range and `'last_value` of more than one element
  are exact for whole blocks (and so for the whole signal); for other ranges the kernel cannot answer, and `Ctx::last_event` and
  `Ctx::last_value` return an internal error naming the signal (`Signal::history_exact`): lean is the elaborator's promise that no
  such use exists, and a wrong promise is a bug that shows, not a wrong answer.
- **The elaborator decides** (`yavs-elab/src/lean.rs`, `Elaborated::lean`, from `Db::facts()`): a wide leaf (more than 64
  elements) is lean unless `'last_event` or `'last_value` (of more than one element) is applied to a part of its object (an
  element, slice, field or non-static selection, also through an alias, which follows to its object; an alias with a subtype
  indication counts as a part), to a whole object that a port collapse made part of its actual's leaf or that is an array of
  composites (the interpreter asks those element by element), or to something that cannot be traced to an object: a signal
  parameter or an external name, which turns the cheap path off for every signal of the design. `'last_value` of a scalar needs
  nothing (the standard packages' edge functions are that), so the IEEE packages leave everything lean. The facts cover every unit
  analysed into the design, used or not: the OSVVM and UVVM utility packages apply `'last_event` to parameters, so designs that
  load them keep every history (`lean_signals` 0 in `uvvm_axistream_vvc` and `osvvm_axi4_random`; 56 in `neorv32_single_min`, 14 in
  `hdlm_axi_fifo`, 8 in `fletcher_bufferreader`, 6 in `sc5_fifo_repack`). Only the subprograms a design can reach would lift that
  (a call graph over the lowered program); not done. Coverage, a debugger or a VHPI plugin (`observed_free` false) keep every
  history too, and `YAVS_LEAN_EVENTS=off` does (hidden test aid, like `YAVS_IDLE_SKIP`).
- **Measured** (bench builds, core 8, minimum of 5 alternating runs, instructions / cycles of `main` before against this round):
  `fletcher_bufferwriter_512` 0.963 / 0.921 (98.6 G to 95.0 G instructions), `fletcher_bufferreader` 0.999 / 1.000,
  `sc5_fifo_repack` 0.996 / 1.017, `sc5_timestamp` 1.000 / 1.010, `neorv32_single_min` 1.001 / 0.972, `poc_cache_cpu` 1.000 / 1.004,
  `hdlm_lfsr` 1.000 / 1.011 (cycle geometric mean of the seven 0.990); the whole real-world suite (42 tests, interpreter) has an
  instruction geometric mean of 1.00 and only the writer moved; the head-to-head designs are level in instructions (geometric
  mean 0.997) and in CPU time (0.998 over 9 interleaved runs of five designs).
- **Tests.** `tests/corpus/rtl/wide_bus_attributes` (every attribute on wide buses through signal parameters: nothing lean, the full
  history) and `wide_bus_lean` (the attributes in the shapes the elaborator accepts, plus a bus per way to ask a part: element,
  slice, alias, record field, port formal that is a part of the bus), both equal to the reference simulator (`'last_value` of
  vectors is left out: the references differ, catalogue `last-value-composite`; the first reference also reports a signal
  assigned in the same delta as active, catalogue `active-in-delta-of-assignment`); `crates/yavs-kernel/tests/lean_events.rs` and the
  signal unit test `lean_events_of_a_wide_signal` (a lean and a full signal give the same answers to every promised question, a
  mutation of the same-cycle merge is caught, refusals are internal errors); `yavs-elab` tests for the marking rules;
  `crates/yavs-sem/tests/history_facts.rs`; `perf.rs` `lean_events` (floor on `lean_signals`) and `lean_events_change_nothing`
  (same report lean, full, and under `--cov`, which keeps the history).
- **Idea of the reference simulator's kernel used (rewritten for ours):** events are kept per net, a run of elements that move
  together, not per element. Different: the reference refines a net lazily when a query covers part of it, so the history from
  before the split stays the net's; ours keeps the exact history of every element a design can ask and the block's last event
  of the others, decided at elaboration, with an internal error as the backstop.
- **Measured and not taken.** Unpruned records with a compaction every four pushes (exact for every query, no analysis): instructions
  0.999 of the pruned lists, because the records of a random-data bus are inherently ~7. Also skipping the `last_value` blend of lean
  signals: a further -0.9% instructions and -0.7% cycles of the writer, but edge tests read those planes and signal parameters reach
  `rising_edge`, so no static rule is sound.

## Compiled clocked RTL processes, round 7: scalar drives kept for one host call (yavs-llvm, yavs-interp, yavs-kernel, #676, 2026-10-24)

Decisions (agent drvbatch; the interpreter verdict is the open question at the end):

- **A zero-delay scalar logic assignment in compiled code is appended to a buffer, not sent to the kernel.** `Gen::drive_bit` (the
  `Drive` of a `yavs_interp::ScalarDrive` with a time register that is a constant 0) checks inline that the target is a one-element
  reference into a signal the kernel offers (`TierSig::codes` or `planes` set, so a logic signal; the reference is not `MULTI`) and
  that the code is at most 8, then writes `(reference word, driver | code << 16 | inertial << 24 | pc << 32)` to a per-activation
  stack buffer (`Batch`, `yavs_interp::DRIVE_BATCH` = 32 entries) and increments a count. Anything else (a full buffer, a case the
  kernel's fastest path does not know) hands the buffer over first and then drives as before, so the order of drives is the program's.
- **The buffer is handed over by `yavs_rt_drive_flush` (`TierHost::drive_bits`, `Ctx::assign_codes_now`) before every instruction that
  could look at a driver, drive another way, wait or run other code (`batch_pure` is the whitelist: constants, arithmetic, compares,
  memory, `SigVal`, `SigReadScalar`, `SigRead`, `Edge`, the table natives) and at every exit of the function** (`ret_exit`: the exit
  blocks `exit_for` makes, so a guard that leaves for the interpreter, a suspension and a fault all pass it). The kernel takes a run
  of one driver in one pass: the driver, its waveform and the idle test once, then one `Out` entry per element, the entries
  `assign_code_fast` would have made; an element the fast case refuses goes through the general entry as `drive_bit` did. No frame
  is left at the flush, so only cases whose general path needs none are kept; `drive_bit_replay` stays for the unkept ones.
- **A site is kept only when a second kept drive can follow it in the same activation** (`batched_drives`: it is in a loop, or another
  kept site is reachable without passing a `Suspend`). A process that drives once per activation (`rtl_ff`) keeps the direct call: the
  flush costs more than it saves for one entry.
- **Why not coalescing adjacent drives or a Rust-side buffer.** The drives of `hdlm_lfsr` are one `Drive` in a loop (`state(i) <= x`),
  with signal reads between the iterations, so no two are adjacent in the IR. A buffer in the compiled function's own stack needs no
  layout shared with the host and no check in every callback; the flush points are the generator's, by instruction kind.
- **Interpreter: not done** (considered, as the issue asks). `LSetDrive` is one op there already and the fast kernel entry is one call;
  a deferred buffer would need a flush at every op that suspends, calls, reads a driver or ends the activation, and the saving is the
  lookups of `assign_code_fast` (about 27 instructions an element, as measured for the compiled tier): about 3 percent of `hdlm_lfsr` in
  the interpreter (22 drives per clock, 4.2 M clocks, 67 G instructions), less elsewhere. Not worth the flush points.
- **Measured** (bench builds with `--features llvm`, `--tier=auto` warm, user instructions and cycles of the whole process tree, core 12,
  the host at load 20 to 40, main before and after, three alternating pairs of five runs): `hdlm_lfsr` 28.09 G -> 25.69 G instructions
  (-8.5 percent), median cycles 8.53 G -> 8.01 G (-6 percent). `perf` of the warm run: `Ctx::assign_code_fast`, `drive_bit_now` and
  `yavs_rt_drive_bit` were 11.3 percent of the cycles, `assign_codes_now` and `drive_bits` are 4.3 percent now, and the compiled
  process itself went from 20.5 to 16.5 percent. `tools/realbench/run.py --compare` over 42 tests (first version, before the
  "second drive can follow" rule): instructions geometric mean 0.99 (`hdlm_lfsr` 0.91, the rest within `--tier=auto` noise,
  #673), CPU time 0.99. `rtl_lfsr`, `lfsr64`, `lfsr64_xor`, `rtl_ram`, `sc5_fifo_repack`, `neorv32_single_min` do not change:
  their processes drive once per activation or run interpreted under auto. `rtl_ff` (one drive per activation) is not
  kept, by the rule above.
- **Tried, not landed:** not counting kept drives as ways out in the policy (`auto::callbacks`, `too_many_callbacks`): instructions
  geometric mean 1.00 over the 42 realbench tests (no process crossed the line), so the policy stays.
- Tests: `compiled_drives::element_drives_in_loops_match_the_interpreter` (element drives in loops against vector drives, delayed
  drives, a wait and a report in the loop, a procedure that drives the same signal, two drivers, a run longer than the buffer; with
  the flush before non-pure instructions switched off it fails), `perf` `llvm_drive_batch` (`cb_drive`).

## Kernel round 19: single-bit drives, Signal layout, record leaves; assignment skipping and observers (yavs-kernel, #676, #519, #677, 2026-10-24)

No kernel speed-up landed this round: each of the three ranked ideas was built and measured (bench builds, `tools/stepcount`
windows, `perf stat` on core 9, minimum of alternating runs; the host ran at load 20 to 30 with the SMT sibling of core 9 busy, so
cycle differences below about 5% are noise and only the instruction counts, which are exact, decide). What was found is recorded so
the next round does not repeat it.

- **Single-element drives of one vector in a loop (#676).** Three designs of the idea, all with identical output and the same
  counters except `direct_codes`/`direct_planes`: (1) at assignment time, neighbouring one-element `Out` entries of a driver become
  one `OutV` (a pair, the third of a run, extension of the driver's last `OutV` upwards or downwards, a one-element `OutV` behind
  other vector entries instead of the general path): `hdlm_lfsr` 67.05 G to 66.45 G instructions (-0.9%, interpreter; its ten
  codes per edge become one plane entry), `sc5_fifo_repack` 41.62 G to 42.43 G (+1.9%: its runs are short, 5.6 M of the 10.7 M codes
  became plane entries that commit singly, and the entry lists grew, which the linear `has_entries` scans pay for); with pairs
  promoted too, +8%. (2) At commit time, `Signal::commit_codes` writes neighbouring entries as one plane block (the skip test on the
  block): `hdlm_lfsr` -0.07%, `sc5_fifo_repack` +0.5%. (3) A lean `assign_code_now` / `assign_planes_now` / `assign_bytes_now`
  that skips the `Option<Range>` argument and the time of the general fast path (a store-forwarding stall showed on that load in a
  `perf annotate` of the compiled tier): -0.25% on `hdlm_lfsr`, +0.15% on the CPU and fletcher designs. None meets the landing rule
  (instructions down on the set), none was taken. Why the ticket's 21% does not turn into instructions: it is cycles of the compiled
  tier, where `Ctx::assign_code_fast` alone is 8% of the cycles for ~40 instructions a call (latency of the driver, waveform and
  signal look-ups behind each call, three layers deep from the compiled code), and `commit_codes` is 1.6% of the instructions
  (5.6% of the cycles) of the interpreter and compiled runs; the batching has to happen in the compiled code (one host call per
  run of the loop), not behind the host call.
- **Signal layout (#519).** `Signal` is 744 bytes (the three value buffers are 56 bytes each, inline for up to 64 elements, the
  stamps 40 more). With `#[repr(C, align(64))]` and the fields ordered by how often `commit_code`, `finish_pend`, `update_word`
  and `finish_code` touch them (flags, `last_commit`, `ty`, the force options, then `driving`, `cur`, `last_value`, `stamps`,
  `codes`, `pend`, the cold lists last), L1 data misses fell by 3% (`neorv32_single_min` 1.539 G to 1.485 G, the 512-bit writer
  2.178 G to 2.123 G) and instructions rose by 0.1 to 0.3% (larger displacements); cycles moved by less than the noise (-6% to +2%
  between runs of the same binary). A second ordering with the force options and `ty` in the first line: the same. Not taken. A
  split into a hot struct and a boxed cold one would not change the lines the hot paths touch (the reorder already leaves the cold
  lines unread), so it was not built.
- **Record leaves (#677), the kernel part.** A 40 us window of `grlib_leon3mp` (interpreter) has the kernel at ~5% of the cycles
  (`update` 1.0, `commit_out` 0.7, `finish_direct` 0.7, `commit_code` 0.6, `assign_code_fast` 0.4, `read_into` 0.4) and
  `run_ops` 48%, `copy_bits` 5%, `lcopy` 4%, `memmove` 4%: the ~680 instructions a leaf are the interpreter's. Per-leaf event
  stamps in the kernel would skip at most a third of the 5%; the gather and scatter instructions of the issue (and the use of the
  stamps by a skipping gather) are interpreter work. Nothing started in the kernel.
- **Assignment skipping and observers (user decision of 2026-10-24: drop an assignment of the value a driver holds only when nobody
  observes the signal's activity).** The mechanism already exists and has been on by default since #420 ("Skipped unchanged
  assignments"): `Signal::skip_ok` (decided by `Builder::mark_skippable` from the signal's guard, resolution function, composite
  group, implicit signals and port conversions) and `Config::skip_unchanged`, which `yavs` sets only when no debugger or
  VHPI plugin is attached (coverage keeps it since 2026-10-24, #707, see "Coverage keeps the count-neutral skips") and no `'active` or `'last_active` exists in the analysed units; the direct paths (`commit_code`,
  `assign_*_fast` phantoms) are those that drop. `grlib_leon3mp` runs with it on (`idle_skips` > 0). This round only adds the
  evidence: the counter `skip_signals` (signals that allow it, printed by `YAVS_COUNTERS=1`) and tests that show it goes to 0 with
  a VHPI plugin (`vhpi_run.rs`) and `'active` or `'last_active`, and drops by the prefix and the implicit signal for
  `'transaction` and `'quiet` (`perf.rs` `skipping_follows_observers`). A debugger session (`--dap`) shares the `others_free`
  expression with VHPI but cannot be tested this way: the DAP path ends without printing counters.
  **Open: waves.** A waveform file (`--wave`) is not in `observed_free` and keeps the skipping and the edge triggers: it records
  value changes, which an equal-value drop does not change (the FST of the design is identical). Making it an observer of
  activity, as the user's list of 2026-10-24 reads, is one condition (`a.wave.is_none()` in `skip_unchanged`) and costs wave runs
  the idle skip (10 to 15% of the instructions of a CPU testbench); it was not made.
  A per-signal 'activity read' flag instead of the global `Facts::reads_activity` would only help designs that use `'active` or
  `'last_active` somewhere: of the corpora only the UVVM SPI BFM package (one file) and the conformance suites do, not OSVVM, VUnit,
  hdl-modules, Open Logic, PoC, neorv32, GRLIB or fletcher; not done.

## GUARD changes in the cycle of its condition (yavs-kernel, yavs-elab, #464, 2026-10-23)

LRM 11.2 and 14.7.5: the implicit GUARD signal is updated in the cycle where a signal of its condition has an event. The
`_guard` process of a block drove it with a zero-delay assignment, which matures a delta later (the references report
`guard=false` once, in the cycle of the event; yavs reported `guard=true` then `false` a delta later).

- **The kernel runs the guard process in the update phase** (`Builder::guard_process`, `SimControl::run_guards`): the elaborator
  gives the `_guard` process of a block the signal parts its condition reads (`Walker::guard_reads`: the sem `reads` of the
  condition, resolved like any name; when one cannot be resolved the process stays a normal one). Its `ProcMeta::ready` stays true,
  so no wake queues it. In `cycle`, right after the update, the guards whose condition signals are in `changed_sigs` run as a
  batch (`run_batch`, so its waits are registered as before), `finish_direct` makes GUARD's event, and the loop looks at the
  new changes for the guards that read another GUARD (nested blocks). The notification of observers, the wake of GUARD's
  waiters and the processes of the cycle come after it: they see the new GUARD in the delta of the event. A GUARD with guard
  readers is not woken through `wake_sole`, which would hide its change. Designs without a guarded block pay one test of an empty
  list per cycle.
- `tests/corpus/delta/guard_same_cycle.vhd` (nested blocks, a guarded assignment and a process on the guard; reference
  output), `crates/yavs-kernel/tests/guard.rs` (the guards as processes and as kernel guards: the same values, the change in
  the delta of the event instead of one or two later). `delta/guard_initial_value` (f60def5e) is the other half.

## Registers and frame variables across blocks (yavs-llvm, #432, 2026-10-21)

Follow-up of "Tight compute loops": in compiled loops the registers and the frame's variables no longer go through
the frame at each block boundary. Frame-mode code only (native frames already keep both in `alloca`s that LLVM
promotes). Why: after the deferred stores, design 1 of the head-to-head (`compute_h2h`) still stored and loaded
each variable and every live register at every block edge, and reloaded the base of the slots per block.

- **Registers as phis in loops.** In the natural loops of a function (`regs::reach`: depth-first retreating edges
  and what reaches each without the header) a deferred register (`regs::deferrable`) that is live into a block,
  not preloaded and not a constant, is a phi there (`Plan`); an edge passes the deferred value, or loads the
  register from the frame at the end of the predecessor. A block reached by exactly one edge, from an earlier
  instruction, takes that edge's deferred values as they are (`Plan::single`, `inherit`). Every other block
  takes its registers from the frame as before and `edge_prep` stores a deferred value that no phi takes, so
  correctness never depends on which registers got a phi. A function whose phis would pass 100000 keeps the old
  stores.
- **Entry stubs.** The interpreter enters compiled code at the start, after a call, wait or report (`resumes`),
  and at loop headers (and after the preloaded definitions of those). Only these have a switch case, each through
  a stub that loads the phi values from the frame. Where the interpreter lands otherwise (a jump target that is no
  loop header) it gets its pc back and runs to the next jump (`tier_go`, "No progress"). Before, every block start
  was an entry, so every loop with more than one block was a multi-entry cycle that LLVM does not treat as a loop.
- **Frame variables as values** (`regs::Scalars`, virtual register `nregs + k`). A slot is kept as a value when
  every access is a `Load` or `Store` of one width through an address register defined once by `AddrOf` of that
  slot, no other use of such a register exists, no escaping address can reach the bytes (an escaped address stands
  for its whole variable, `Func::locals`, else the rest of the frame), the function has no nested function
  (`Func::nested`), and an access is in a loop without a loud instruction. At most 12 slots. A `Load` is the
  value (loaded once, kept through the block and the phis), a `Store` defines it with the width's truncation and
  extension. Write-back uses the register machinery: `Live` has a bit per slot, used at every instruction that can
  leave to the interpreter or call out (`Scalars::observes`), killed by a `Store`; `flush_row` writes the live ones
  where the frame is looked at. A slot the function never writes is never written back.
- **Quiet steps and calls.** A callback that returns to compiled code cannot read a variable no address of which
  was made, unless it collects garbage or runs other code: only `Call*`, `New*`, `Free`, `Report`, `Output`,
  waits, `Suspend`, `Elab`, `PkgAlloc` (`regs::loud`). Other stepped instructions write variables back only on the
  path where they fail; a direct call to a native frame (no step, no allocation) likewise, on its
  materialisation path. A loop with a loud instruction gets no variables as values.
- **Base of the slots.** The pointer to the frame's slots is a pseudo register carried through loops
  (`own_slots_cached`), forgotten after every callback (`call_go`: a pushed frame may move the slot stack) and,
  because a value found in a branch does not dominate the join, restored at the join of a slow path to its
  pre-branch value (`slow_path`, `slow_signal`; forgotten if the slow step is loud). A first version that kept
  the value found in the branch produced IR that did not verify.
- **Constants.** A register defined once by `ConstF` is its value everywhere, like `Const` (`fixed_consts`).
- **Errors and traces are unchanged**: every exit stores what the interpreter re-executes from.
- **Tried and dropped**: one shared exit block per set of written-back variables (LLVM moved its phi operands into
  the loop: +8 instructions per iteration); phis in every block (compile time +20% for nothing outside loops).
- **Result** (core 6, `perf stat`): `compute_h2h` under `--tier=llvm` 3.14 G to 2.74 G instructions for 20 million
  iterations (157 to 137 per iteration with the start-up, 137 to 108 per iteration at the margin; the reference
  code generator: 115 in total). Other rows are unchanged (`compute_loop` and the RTL rows are bound by heap
  element access and the kernel). Cost: eager compilation of library-heavy designs takes 5 to 20% longer (IR +10 to
  25% in loops); `--tier=auto` totals are within 5% (`compute_h2h` +5% from the longer compile).
## Native frames, round two: library functions run on native frames (yavs-llvm, yavs-interp, #432, #337, #340, 2026-10-21)

- **Problem:** in a verification-library scoreboard testbench 21 of 260 functions could run on native frames; 199
  stepped one of `Deref`, `CallProt`, `CallNative`, `Cat`, `NewFrame`, typed `TempRoot` (and `Free`, `LayoutOf`,
  `LFill`, nested subprograms, `FToIRange`, `**`) on their fast path, so every call of them used the interpreter's
  frames. Now 256 of 288 (and 413 of 440 in the random-number testbench) are native-callable; what is left is
  elaboration thunks, processes, `wait` inside procedures and package allocation.
- **Inline in a native frame** (frame-mode code keeps stepping these): `NewFrame` (the frame-pinned temporary, its
  `temp_base` a cell of the native frame), `Fill` (a loop or `memset`), `Cat` of non-logic elements (every address is
  found before anything is written, so a failing guard changes nothing), `SubtypeCheck` of up to four dimensions and
  `LayoutOf` from the descriptors read in place (own slots included), `CallProt` and `Call` of nested subprograms as
  direct native calls. A register the interpreter preloads (defined once by `Const`, `ConstF`, `AddrOf`) is stored at
  entry of a native frame: its definition may not have run on the way to a use (this alone removed most
  materialisations).
- **Collections never run while native frames exist** (decision, replaces the stack-map design the issue suggested):
  every instruction that can collect (`New`, `NewFill`, `Free`, a native call) first asks `Machine::reclaim_pending`
  (the heaps' own `gc_due`/`reclaim_due` test, so it holds in the `gc-stress` build too, where it makes native frames
  materialise more often) and leaves to the interpreter when a collection is due; it runs there with every frame's
  registers in the machine, so precise roots (#422) hold by construction and no spill map is needed. Cost: a
  collection materialises the native frames once. Revisit with spilled root maps if materialisations at collection
  show in a profile.
- **`CallNative` from a native frame** (`TierHost::native_direct`): the argument cells come from the registers (cell k
  an address when the register is `Ptr`), no interpreter frame. Allowed natives are `yavs_interp::direct_native`
  (`pure`, `now`, `writeline`, file open/close); `stop`/`finish` are a materialisation (`cold_native`); any other
  native keeps the function frame-mode. A native that fails, hands its call to the VHDL body (`ieee_fallback`) or finds
  a collection due changed nothing the interpreter's second execution does not see: compiled code materialises and the
  interpreter executes the call (replay, so errors and traces are the interpreter's). A native that ends the process
  (`ieee_stop`) returns `NativeCall::Stop`: the frames materialise after the call and the process suspends.
- **Addresses into native frames.** A native frame's slots are an `alloca`; the machine cannot reach them by itself.
  Each native frame registers its slots in a per-thread table by frame index (`Mem::nslots`, two stores at entry). A
  native callee finds the slots of its native callers inline (`Gen::host`, by-reference parameters), and while a host
  call from a native frame runs the machine reaches them too: `Machine::obj`/`obj_mut` take the slots of a frame index
  between the machine's top frame and the calling native frame from `NativeSlots` (the table), by address, no copy
  (pointers in the arguments or stored in memory the native reads, strings of a caller's frame, resolve the same way).
  Registers of frame-mode callers are pointed to by compiled code: the register stack may not move during such a call
  (`native_step` leaves when its capacity would grow). A first design copied all native frames' slots into machine
  frames for the call and retried after a failure; it cost a fifth of a library testbench.
- **`TierHost::native_step`**: instructions with no inline form in a native frame (`New`, `NewFill`, `Free`, `LFill`,
  `LEq`, `Report`, `FToI`, `FToIRange`, `Phys`, `LNum`, `LogicOp`, `**`, logic `Cat` of unknown lengths) run the
  interpreter's own `Machine::step` on a transient frame holding the registers they name (passed by index); the
  registers they write come back. A `Report` needs the call stack and is made only when no native frame lies between
  it and the machine's top. A failure is a replay as above.
- **Typed `TempRoot` and `Stmt` with recorded roots** go through `TierHost::temp_root` and `end_stmt` (the machine's
  `troots`), not a materialisation; the roots are the machine's, so they are as precise as the interpreter's.
- **Protected calls.** `TierFrame::flags` carries `shared_alloc` of the top frame and whether the process may use the
  shared state. `CallProt` from compiled code checks the second (else the interpreter's error), sets the first for the
  callee when the object's handle is in the shared heap, and restores it after; materialisation recomputes it from the
  `CallProt` the parent waits in. Natives called from a native frame allocate with the frame's flag.
- **Nested subprograms** take their static link as two extra parameters (the frames one and two links up, as
  `TierFrame::up`); a call passes the caller's own frame when it encloses the callee, else the caller's links (the callee
  must be enclosed by the caller or by the caller's own enclosing subprogram, else the caller is frame-mode).
  `NativeFrame::link` restores `Frame::link` on materialisation.
- **`--tier=auto`:** a function that runs on native frames compiles with every function it calls natively (those as
  native-frame code only, `Part::Native`, no entry from the interpreter); its native-frame code is published to the
  call table only once all of that exists, since a missing callee would materialise at every call. Budget rules are
  unchanged: the code is large (an eager `--tier=llvm` compile of a library testbench takes minutes), so nothing is
  compiled in a run of a second; see docs/STATUS.md "llvm" for the measured steady-state gain.

## Build and switch fingerprint of cache entries (#491, #407)

Entries that hold generated code (`low`, `init`, `ir`, `lib`, `libopt`, `libinit`) were keyed by the build
(version, executable size and modification time) but not by the optimiser, so toggling `YAVS_NO_CFG` or
`YAVS_NO_MEMO` against a library directory loaded optimised copies another configuration had made.

- **Switches.** `yavs_ir::opt::fingerprint()` (`iropt`, `memo`, `cfg`, keep-variables for a debugger,
  the compiled tier, external variables for VHPI and `--wave-variables`) is mixed into the key of every stage
  entry (`stage_key` in `crates/yavs/src/cache.rs`), also of the `lib` existence check. Coverage is not a key
  part: a run with coverage counters uses no stage entry at all. `sem` and `pre` do not depend on the
  switches and stay shared. Another setting is a miss, never an error, and its entries are pruned by the normal
  policy. Toggling `YAVS_NO_IROPT` therefore makes entries of its own (it used to read the `ir` entry and
  ignore its copies).
- **Build.** `crates/yavs/build.rs` hashes the sources of all crates (`src`, `Cargo.toml`, `build.rs`) and
  `Cargo.lock` into `YAVS_SRC_HASH`; `yavs_cache::set_sources` makes it the build id's source part, with the
  executable size (a toolchain or dependency change) and the version. Copies of one build, and rebuilds
  of unchanged sources, share entries; any source change is a miss. Without the workspace (a packaged crate) the
  hash is empty and the id falls back to the executable's modification time. `YAVS_CACHE_SALT` is mixed in as
  well (stands in for another build in tests).
- **Cost.** A fingerprint is six booleans formatted per key (a few keys per run): no measurable startup change.

## Compiled code in the compilation cache (yavs-llvm, yavs-cache, CLI, #505, #407; maintainer decision 2026-10-21: the cache pays off, build it)

The compiled tier compiled every module again on every run: an eager `--tier=llvm` compile of a library testbench
is one module of minutes, and every test case of a suite paid it again. Decisions:

- **What is kept.** The object file of every module the tier compiles: `--tier=llvm`, the program in one module;
  `--tier=auto`, one module per hot function (with the native-frame code of what it calls, `Worker::tree`), at
  each level. It is stored as kind `obj` in `<work dir>/yavs-cache/obj/` (`yavs-cache`
  `Store::objects_of_library`) and a later module with the same key is linked from it with ORC
  (`LLVMOrcLLJITAddObjectFile`) instead of being optimised and compiled. The CLI names the work library
  (`yavs_llvm::set_cache`) unless `--no-cache` is given.
- **Key: the generated LLVM IR, not the yavs IR** (yavs decision). The key is the module's bitcode before the
  pass pipeline, the level and the pipeline chosen (`optimize_as`'s `full`, a custom `YAVS_LLVM_PIPELINE`), the
  target (triple, the CPU baseline, the LLVM version, the code generator switches `CODEGEN_ARGS` or
  `YAVS_LLVM_ARGS`), the build and switch fingerprint of #491 (`yavs_ir::opt::fingerprint`) and the build id and cache format (`KeyBuilder`). The
  generator reads more than the function it compiles (which callees run on native frames, constant bytes it
  copies into the module, static layouts, the tier switches), so a key of the yavs IR would have to name all
  of that; the LLVM IR is everything the machine code depends on, so a hit is by construction the code a
  compile makes, and anything that changes the code is a miss. Ceiling: the generation itself still runs on
  a hit (a small part of a compile).
- **Nothing absolute in the code** (audit of `lower.rs`, 2026-10-21). The runtime callbacks (`yavs_rt_*`) are
  external symbols that ORC resolves to this process's addresses when it links the object (`new_jit`);
  constants and tables known at generation time are private data of the module (`const_global`, `table`,
  `u32_table`); compiled callees in other modules are reached through `Mem::ntab` at run time; offsets in
  `Mem` and `TierFrame` are constants of the build, which is in the key. No host pointer is an IR constant.
  The tests check it: a warm run is a new process (another address layout) and links the stored object.
  New code must keep it so (`crates/yavs-llvm/AGENTS.md` section 4).
- **Cold and warm make the same code.** With a store, every module is compiled to an object by
  `Module::emit` with the target machine the JIT uses (portable CPU, JIT code model, the level) and linked
  through the object layer, on a miss as on a hit; without one (`--no-cache`) the module goes to ORC as IR as
  before, which compiles it with the same target machine.
- **The store.** A subdirectory with limits of its own (4096 entries, 256 MiB, least recently used first),
  so that one small object per compiled function never pushes the stage entries out. Writes are a temporary
  file and a rename, so parallel runs (a suite at `-p N` into one library) need no lock and at worst compile
  one module twice and write the same bytes; a directory that cannot be written gets nothing and the run
  compiles as before; an entry is validated (header, length, checksum) and one ORC does not take is a miss,
  compiled again and replaced. `yavs cache clean` removes it with the library's entries.
- **Tiered mode: a hit links instead of compiling.** Whatever `--tier=auto` decides to compile (the first level,
  the O2 recompile, the eager `--tier-heat` mode) is linked from the cache when it is there, and the budget is
  charged the time that took. A compute loop (`compute`), which runs several times faster compiled, does not
  wait for the payback rule when it is stored: once it is as hot as compiling it needs at all
  (`Heat::threshold`), the compile thread looks it up (`Worker::probe`: generation and lookup,
  `objcache::cached_only`) and links a hit at once, outside the budget, with what compiles with it; a miss
  waits for the payback rule and its generation counts as compile time spent. Other functions keep the payback
  rule: linked that early, the compiled testbench functions of three library testbenches made their warm runs
  10% to 22% slower (pinned A/B, 7 runs), while the compute benchmark ran 21% faster.
- **What is shared.** A module is shared when its LLVM IR is identical. Within a work library: every later run
  of a design and every test case of a testbench (top-level generics are not part of the code). Across
  designs: the code names function, constant, type and table ids (symbol names, the call table, arguments of
  the runtime calls), which depend on the lowering of the design, so the program of `--tier=llvm` is never
  shared between testbenches, and a per-function module of `--tier=auto` is when the library's functions come
  from one lowered library (`lib`, "Library stages"). Ceiling: one store per run, the work library's
  (`ponytail` in `objcache.rs`); sharing library code between designs in different work libraries, or
  between the programs of different testbenches, needs code that names no design-dependent ids.

## Interpreter round 10: the statement marker inside the edge test and the wait, the jump back skipped, frame 0's slots direct (yavs-interp, #443, 2026-10-21)

- **A `Stmt` before the edge test or the wait of a clocked process is part of that op.** `EdgeBr` and
  `WaitSuspend` carry `stmt`: the op ends the statement's temporaries as the `Stmt` did and goes on as the
  op after it, so faults and the suspension pc stay those of the unfused instructions. The `Stmt` op stays
  in place behind it for any other way in (`pre::superops`, second pass).
- **A `Br` to a `WaitSuspend` is a copy of it.** The copy carries `at`, the op it stands for, and runs as
  that op (`k = at`): the end of every activation of a clocked process (the jump from the end of the `if`
  to the wait) is one op instead of two or three. Nothing else changes: a jump has no effect of its own,
  and compiled code is entered again at the next activation.
- **A resumed process does not run its jump back.** `run_ops` starts past the `Br`, or the `BrBody` when
  no pause is set (`prestart` sets one and runs the step path), that a process stands at after `Suspend`.
- **The fast loop reads frame 0's slots from `root` without looking up the frame** (`fslots!` in
  `run_ops`; other frames go through `slots_of`): a process variable, a memoised value or the flag of
  `FlagBr` is one bounds check and a load.
- **Register accesses stay bounds-checked; the interpreter keeps no `unsafe`.** Measured (pinned core 5,
  `perf stat`, 3abd2fd7): register reads and writes through `get_unchecked` change rtl_ff by -0.3 percent
  and rtl_lfsr by -1 percent of instructions; an unchecked op fetch on top made rtl_ff 13 percent worse
  (the compiler laid out the loop differently). The check is two instructions of the seven a register
  access costs (the frame base, the vector pointer and its length are reloaded from the machine after
  every call); only a register pointer held in a local would remove the rest, which needs `unsafe` whose
  soundness depends on every op that can grow the register stack. Not worth it for a few percent.
- Rationale: the predecoded loop is instruction-bound (IPC near 4, branch misses under 0.1 percent); a
  clocked activation is about 12 ops, and each removed op saves its dispatch and its bookkeeping, 7 to 25
  instructions. Small code changes in `run_ops` move rtl_ff by up to 4 percent either way through the
  compiler's register allocation of the one large function (#508), so every change here is measured on
  several designs.

## The shape of the interpreter's op loop: outlined ops, errors off the hot paths (yavs-interp, #508, 2026-10-22)

- **Problem.** `Machine::run_ops` was one function of about 125 KB of machine code, 90 percent of it inlined helpers
  (`mem_fast`, `deref`, `copy_fast`, the heap and `ieee` code, error constructors). Changes that touched no RTL op moved
  the instructions of a clocked process by up to 5 percent through the register allocation of that one function (#493's
  commits +3.6 percent, `#[cold]` on `ice` and `rt_err` +5.5 percent on rtl_ff, one unused arm +0.7 percent).
- **Op frequencies decide what stays in the loop.** Counting executed ops over the head-to-head designs, the library
  testbenches and `vunit_vc30` gives: the RTL ops (`SigRead1Slot`, `SigRead1`, `Gate`, `EdgeBr`, `WaitSuspend`, `FlagBr`,
  `SigValSlot`, `LSetDrive`, `Cmp`, `MoveCondBr`, `LGet`), the scalar and address ops (`Bin`, `Aff1`, `Load`, `Move`, `Br`,
  `CmpBr`, `CmpBrStep`, `BinRangeK`, `Index*`, `Load*`/`Store*`, `Stmt`) and the call, return and protected-call ops. The
  bodies of every op outside that set (the descriptor ops, `Drive`, `WaveBuf`, `Layout`, `Field`, `MemEq`, `Cat`-adjacent
  ops, `CopyConst`, `MakeDope`, `BinRange`, `PosIndexStore`, `IndexStore`, `WaitOn`, `SigReadScalar`) are methods
  `Machine::op_*` with `#[inline(never)]`; the arm calls them and does `k += n`. A fault keeps the IR pc of the op (`t!`).
- **Every error is built out of line.** `ice_f!`, `rt_err_f!`, `runtime_f!` (with `format!`) and `ice_s!`, `rt_err_s!`,
  `runtime_s!` (a literal) run the constructor in `cold`, a `#[cold] #[inline(never)]` function; `ice` and `rt_err`
  themselves are plain. A unit test (`errors_are_built_out_of_line`) fails on an error built the old way in `exec.rs`.
  The fault path of `t!` is `fault_at`, also cold.
- **Helpers are inlined or not on purpose.** The ones the hot ops use (`add_addr`, `split`, `mk_addr`, `is_multi`,
  `driver`, `sigref`, `delay_mode`, `fill_blocks`, `new_pair`, `fat_root`, `temp_alloc`, `rd`) are `#[inline(always)]`; the ones
  the rare ops use (`dim_of`, `layout_of`, `layout_words`, `copy_bits`, `cat`, `drive_parts`, `lcopy`, ...) are
  `#[inline(never)]`. The one-element signal reads of a clocked process (`read_sig1`, `edge_sig1`, `wait_sig1`) read the
  instance signal table and the kernel mirror directly, with the general code as `*_slow`.
- **What was taken from sim1's interpreter** (ideas only, rule 22): its loop is a bare dispatch `switch` whose cases call one
  small function per operation, so the compiler sizes and allocates each op on its own and the loop stays small (here:
  the `op_*` methods for everything but the hot arms); operand access is a few always-inlined helpers over a flat register
  array while errors and traps leave through out-of-line exits (here: `cold` and `fault_at`); and tier-up counting happens
  per call, not per op. Not taken: a register pointer held in a local (needs `unsafe`, this crate forbids it), and the
  per-call counting (the counters here already sit in `go!` and are `TIER`-only).
- **Tried and dropped.** A separate `hot()` function with the common ops, called from `run_ops` in a loop (+12 percent: a call
  per op and the same register pressure); `std::hint::cold_path` on the arms of the rare ops (+1 percent instructions, no gain
  in stability); outlining more of the descriptor ops (the ones that run in every library testbench, `PosIndex`, `Pos`,
  `AffPL`, got slower: +2 to +4 percent on the testbenches).
- **Stability.** Measured as the change of executed instructions after a perturbation that touches no RTL op (a new unused arm,
  a check added to an unrelated arm or helper, the error constructors marked cold, a new arm that calls `pos_len`): see
  docs/STATUS.md "Interpreter round 11". Marking a helper that a hot-ish arm calls `#[cold]` still moves the loop (the call
  site becomes a cold block); do not mark helpers cold that run in a benchmark.

## Compiled clocked RTL processes, round 3: a leaner activation and fewer callbacks (yavs-llvm, #440, #507, 2026-10-21)

- **One `Mem` per simulation thread** (`lower::MEM`, a thread-local pointer): an activation takes it, resets
  the four object entries and the deoptimisation flag, and puts it back; one inside another activation (a
  native running VHDL code) finds none and builds its own. Building the 384-byte `Mem` on the stack was
  about 45 instructions per activation. The per-thread `NSLOTS` table is found through a `Cell` with its
  address and length (no `RefCell` borrow, about 40 instructions, also for processes without a native
  callee).
- **What the generator knows about a register holds until the next block start** (`block_start`), not
  until the next resume point: the interpreter reaches the pc after a call only by running the block up to
  it, so the address facts (frame, constant, instance, temporary) are still true there. A table or edge
  native that compiled code runs itself is no resume point at all (`resumes`): it leaves only with an
  error. Both took per-activation `yavs_rt_host` calls and frame stores and reloads out of clocked bodies.
- **In frame-mode code of a process an address of unknown space is looked up inline**, first in the
  objects `Mem` keeps between activations (`Mem::objs`, a select over the four entries with the write
  flag and the bounds), then in the temporary area, and `yavs_rt_host` is called only when neither holds
  it (`Cur::objs_inline`). Only processes: the inline code costs about 20 LLVM instructions per access,
  which a function with many accesses of unknown space would pay in compile time; processes are where an
  access repeats every activation (`lfsr64` -157 instructions per activation).
- **`--tier=auto` leaves a process interpreted when it leaves its compiled code too often**
  (`too_many_callbacks`): more than one in ten (`CALLBACKS`) of the instructions an activation runs (those
  reached from after each `Suspend`) not inline, each `Stmt` counted where `TempRoot`s record temporaries
  (it is stepped then). The testbench process of `rtl_ram` (33 of 258) ran 10% slower compiled (#507); the
  clocked processes that gain have one in eleven or fewer. A static count, not weighted by which path runs
  (`ponytail:` in `auto.rs`).
- **`RTL_MIN` is 120 IR instructions (was 140):** with the cheaper entry the 134-instruction processes of
  `lfsr64` gain 13% of instructions and 18% of cycles under `auto` at ten times its default length (5% at
  the default length); the 90-instruction process of `rtl_chain` still loses compiled (+8%), so the limit
  stays above it. A run of under 0.1 s pays the compile or the link (`lfsr64_xor` at its default length, 3%
  more instructions).
- **Measured and not built:** compiled processes keeping their own registers (`resident`) instead of the
  thread's scratch: 133 fewer instructions per activation on `rtl_ff`, but 18% more cycles (the register
  files of 4096 processes do not stay in the cache; the compiled code stores a few registers per
  activation before its callbacks), 14% more on `rtl_lfsr`, level or 2 to 5% fewer on the designs of 64 to
  4000 small processes; the same with only the temporary area taken from a per-thread buffer, so the
  register lines are the cost. Reading a drive's waveform one word at a time (a 16-byte load spans two
  8-byte stores): 1% fewer cycles on `rtl_ff`, 2% more on `rtl_lfsr`. `#[inline]` on `enter`: no change.

## Native calls from the interpreter and the object table (yavs-llvm, yavs-interp, #510, 2026-10-22)

- **Problem:** compiled code of a protected-call heavy testbench ran no faster than the interpreter (`--tier=llvm`
  30.5 G instructions against 32.9 G on `protected_queue`): every access to a shared-heap object went through
  `yavs_rt_host`, and the four-entry object cache, refilled round robin and emptied at every host call, missed for half
  of them (each miss a `TierHost::object` call). Under `--tier=auto` a compiled function entered from interpreted code
  paid a frame push, the frame view and its frame-mode code, which steps `NewFrame`, `Cat`, `Fill` and typed
  `TempRoot`: string helpers called often on short strings ran slower compiled.
- **Object table.** `Mem::objs` is a direct-mapped table of 16 entries (256 since #520/#493, see "`Mem::objs` with 256 entries"; `obj_at`: the low bits of the object index,
  which are also bits 32 and up of an access value), looked up inline by every access of unknown space in every
  function (one load and compare; then, on a miss, the temporary area in a process and the instance data for a read).
  An entry also holds the access value a `Deref` found it for: a `Deref` of that value is inline (`Gen` compares the
  entry's value), and `FAST_DEREF` enters the object at once, writable, so the accesses after it are inline too.
  How long an entry stays (`Life`): frame and temporary objects until the next host call; heap objects (process and
  shared heap) while `TierFrame::heap_epoch`, the sum of both heaps' epochs refreshed with the temporary area, is
  unchanged, so across host calls that deallocate nothing (`Mem::forget_since`) and from one activation or
  `Tier::call` to the next on the same machine (`Mem::begin`); constants, instance data, package data and the shared
  variables while the same machine runs (`TierFrame::owner`, a number unique to each machine: another machine may run
  another program). Writable heap objects kept into a new activation or call count as written again
  (`TierHost::wrote`, now also for the shared heap), so the waveform recorder's write counts stay right. ponytail:
  direct-mapped, two objects whose indices agree in the low four bits evict each other; ways if a profile shows
  conflicts.
- **Calls from the interpreter on native frames (`Tier::call`).** Every function with native-frame code and no static
  link gets an adapter `f<id>_c(env, mem, depth, cells)`. The fast loop's `Op::Call` and `Op::CallProt` of a function
  `NativeFlags::set_call` marks ask `Tier::call` first (after a protected call's shared-state check): the argument
  cells come from the caller's registers, the callee runs at frame index top plus one with the checks of a frame-mode
  caller's `Gen::native_call` (depth limit, `TIER_SERIAL`, `TIER_SHARED_ALLOC` for a shared object), and its value
  goes to the call's register. A materialisation pushes the native frames on top of the caller, which waits at its
  call (`TierHost::materialise`, as for a frame-mode caller), and the interpreter goes on with them (`Flow::Call`).
  The tier declines (the interpreter makes the call) beyond 16 cells or the depth limit. Errors and traces stay the
  interpreter's (replay), output is the same whatever is compiled (rule 5).
- **`--tier=auto`:** a hot native-callable root without a static link and without a loop is compiled as native-frame
  code only (it is reached through `Tier::call` and from compiled callers); one with a loop gets its frame-mode code as
  well, for the interpreter to enter again after a materialisation inside the loop.
- **Tuning aid:** `YAVS_LLVM_PERFMAP=1` writes `/tmp/perf-<pid>.map` with the compiled functions, so `perf report`
  names them.
- **`--tier=auto`, round 2 (#510).** With native calls from the interpreter a compiled testbench function gains two to
  three times instead of a few percent, and the policy follows:
  - A function the interpreter calls on a native frame (`called_natively`: runs on native frames, no static link) is
    never `too_short` and its sampled time counts for itself (`charge` moves only nested functions' time to a caller);
    a frame-mode function whose instructions leave the compiled code more than one in ten times stays interpreted, as a
    process does (`too_many_callbacks`).
  - Its tree compiles callees that are called natively (no loop) as native-frame code only, others as both; the
    native-frame and the frame-mode code of a function are modules of their own, so what the cache holds for a function
    does not depend on what it was compiled with.
  - Budget: compile cost starts at 800 microseconds per IR instruction (measured, was 250) and learns only from real
    compiles (a cache hit is no measurement); opening the LLVM library counts 10 ms; a function called natively gets
    eight times the budget (`NATIVE_BUDGET`), as `rtl` processes get 16 and compute loops 50; `pick` waits for the
    sampled time its last refused candidate needs instead of recomputing every millisecond.
  - Cache first: every function seen for `Heat::threshold` that the store names as compiled before
    (`objcache::compiled_list`, a list of names and sizes per library directory, written at every compile) is looked up
    in the cache at once, within the budget for its code generation, and linked when there (`probe`); a function the
    budget refused is asked again (it was asked once only, and a refusal lost the look-up for the whole run, which
    made warm runs vary from 0.5 to 1.0 times the interpreter). A run whose functions were never compiled does not
    open the LLVM library.
  - Write counts: compiled code reports the writes of heap objects it keeps into a new activation or call only once
    someone compares write counts (`TIER_STAMPS`, set by `Machine::stamp`, the waveform recorder).
  - A frame-mode function's own callees are roots of `native_set` in `compile_one`, so its code calls those that run on
    native frames directly (a testbench process or a procedure with a wait stepped every call before).
  - A clocked process (`rtl`) is compiled only when at least `RTL_INSTANCES` (16) processes are made of it
    (`NativeFlags::instances`, counted by `InterpProcess::new`): the processes of a CPU design (most made once) ran 6 to 8
    percent slower compiled at the same instruction count, and a wide-vector design 7 percent, while designs with many
    instances of one clocked process ran 7 to 34 percent faster. A per-process comparison of sampled time before and
    after compiling was tried and dropped: at a few percent of the run per process, the sample counts are too small to
    see a 5 percent difference.
  - Ideas taken from the reference simulator (sim1; read for its design, no code taken): it has one way into compiled
    code from everywhere (a function's entry, arguments in an array of 64-bit cells), which `Tier::call` and the
    `f<id>_c` adapters follow, while yavs keeps its two frame kinds and materialisation; its compiled code holds raw
    pointers to heap objects with no lookup per access, which the object table approximates under yavs's generational
    handles (design rule); its protected objects need no lock because processes that touch shared state run serially,
    as yavs's do (`TIER_SERIAL` is the only check); it swaps one entry pointer to tier up, as `Entry` and `ctab` do.
    Unlike it, yavs samples instead of counting calls and keeps compiled code across runs.

## Optimiser round 11: read-only instance data, value numbering of multiply defined registers (yavs-ir, #511, #520, #523, 2026-10-22)

Decisions:

- **Instance data and the constant pool are read-only for the whole function** (`opt.rs` `read_only`): memory facts about
  them (loads, descriptor lengths, positions through a descriptor in instance data) survive statement markers, stores
  through pointers and calls. The interpreter refuses writes to them (`obj_mut`: "write to read-only memory"), so no
  instruction of a subprogram or process can change them; the elaboration bodies that fill them are left alone by the
  optimiser. Why: the loop of a testbench that indexes a constant array of the instance reloaded the array's address and
  descriptor at every statement, and a repeated `a(i, j, k)` of an instance signal recomputed its position twice.
- **A register defined more than once gets a version at every definition on the walk's path** (`Opt::vsn`, undone when
  the walk leaves a block, `Opt::vlog`). Keys use `Opn::V(reg, version)`, so `Pos`, `Index`, `Load`, `Bin`, ... on a
  loop counter or a promoted variable are shared between statements until the register is written again, and a result held
  in such a register (`Val::V`) is reused through a `Move` while its version is current. Why: promoted variables and loop
  counters are exactly the registers of loops, and none of this applied to them.
- **A constant or address a multiply defined register holds since its last definition on the path is a known operand**
  (`Opt::mval`, only where the register has the type of the value, and not past a suspension), which is #511: the assert
  chains of a library's queue (`Const r = 0; Not; CondBr`) fold.
- **The clean-up of a subprogram that copied blocks (`cfg::thread`) is followed by one more walk** (`opt::tidy`): what is
  known when a block is entered decides the branches of its copy. The clean-up also drops what nothing reads, a decided
  condition's flag and the constants that fed it, and knows the result of `Bool` of known operands.
- **Inlining takes callees with branches and protected methods** (`opt/inline.rs`, #509): a callee of up to 60 instructions in
  8 blocks whose branches are conditional jumps and a `Case` of up to 6 arms (compares: a case table cannot be added to a
  program that is being optimised) is copied into the caller as appended blocks; the caller's block is cut at the call, a
  `Ret` becomes a move of the result and a jump back. `CallProt` is a call whose first argument is the object, and the
  caller may be a process. A level-1 callee has its own inlinable calls inlined first (a wrapper in front of a method is
  one body); level 0 is the callee as optimised, so cycles end and the result does not depend on the order functions are first
  used. The record of where an instruction came from is a chain (`Func::via_nodes`, `ViaNode::up`): the stack trace and the
  `std.env` call path list every inlined function as a frame of its own. Why: a protected-method call cost about 500 host
  instructions in the call and return machinery alone, and one character of a verification library's queue makes five.
- **A call that passes its parameters in registers leaves their slots alone** (`Pre::lean`, #443): the prologue the call
  skips was their only reader. Ideas taken from the reference simulator (sim1; read for its design, no code taken): its
  calls pass arguments in one array of cells and the callee's frame is not zeroed or sized beyond what it uses; protected
  methods are plain calls with the object as the first argument, and its optimiser numbers the values of registers, not the
  registers (the versions of `Opt::vsn` follow that idea).

## `--tier=auto` as the default: not yet (yavs-llvm, #510, 2026-10-22; recommendation, the maintainer decides)

- **Recommendation:** keep `--tier=interp` the default for now. Evidence in docs/STATUS.md "`--tier=auto` as the default: the
  case": auto halves the run of testbench-heavy and many-instance designs and is level on protected-call library
  testbenches, but runs where nothing pays are 1 to 6 percent slower (the interpreter's bookkeeping with a tier set, #525,
  plus the compile thread and the LLVM library), and single-instance RTL designs (a CPU) do not gain.
- **When to switch:** once a run in which nothing is compiled costs no more than noise (#525, done 2026-10-22 except
  designs of thousands of one-line processes, see the #525 entry below) and compiled clocked code of
  single-instance designs no longer loses (#532, #527). The policy is ready for it: cold runs compile within a budget of
  their own time, warm runs link what earlier runs compiled at once.

## Static waits and checked copies (yavs-interp, yavs-elab, #530, #529, 2026-10-22)

Decisions:

- **A wait that names the same signals every time registers nothing** (`Op::WaitStatic`, using `Ctx::wait_same` of
  kernel round 12): the one wait of a process with a single suspension point (and a sensitivity list, or no call of
  anything that may suspend) names static signal names only (LRM 10.2, 11.3, and the equivalent processes of
  concurrent statements), so when the kernel still holds the last wait, the interpreter skips its signal references and
  goes to the suspension; the commit keeps the resident waiters and takes the fast path of a process that only
  scheduled drivers. Only address, length, position and arithmetic instructions, signal references and `WaitOn`s may
  lie between the wait's `Stmt` and its `Suspend` (no timeout); skipping them writes no memory, and no register is live
  across a suspension. Why: a process sensitive to a 41-leaf record ran 45 `SigVal`/`WaitOn` pairs per activation.
- **A kernel copy (`ProcInst::copy`) needs the same subtypes on both sides**: target and source must be static signal
  names whose elaborated shapes have the same types, array lengths and scalar subtypes or bounds (`same_values`), so no
  check of the assignment can fail. The first version took `nn <= n1` (`natural` from `integer`) as a copy and lost the
  range check. Static explicit slices qualify too.
- Idea taken from the reference simulator (sim1; read for its design, no code taken): it registers the triggers of a
  static wait once, when the process is reset, and the wait only suspends; yavs registers at the first activation and
  keeps the waiters resident (`ProcMeta::perm`), so only the interpreter's re-execution of the wait had to go.

## Loop-invariant code motion (yavs-ir `opt/licm.rs`, #526, 2026-10-22)

Decisions:

- **Only in a function or procedure that cannot suspend, without a debugger, after the clean-up (`cfg.rs`) and before
  register numbering and inlining**; `YAVS_NO_CFG=1` turns it off with the clean-up. When it moved something, the walk of
  `opt::tidy` runs again over the result. Why: the element loops of conversion functions are where the time goes (#518,
  #526), a process loop would need registers live across a suspension, and a debugger expects every instruction at its
  statement.
- **Also in processes, for loops without a suspension** (#541, 2026-10-22): a loop of a process whose blocks hold no
  `Suspend` and no call of a subprogram that may suspend gets the same treatment (no clean-up and no `tidy` walk
  before or after), so no moved register lives across a suspension. The tap loop of a clocked process over an instance
  constant was four operations per pass, two of them the constant's address.
- **Loops are natural loops with a single entry, found from the back edges of one depth-first walk**, taken inner first; a
  header that several back edges share (the outer loop's body starts with the inner loop, which the clean-up's threading
  produces) is split: the latch with the smallest loop keeps it, the entry and the other back edges go through a new block
  that only jumps there. A new preheader block is made unless the one outside predecessor only jumps to the header.
- **Failures stay where they were.** Almost every instruction can fail at run time (positions, checked arithmetic, checks,
  descriptor lengths). One that can fail moves only from the start of the header up to its first instruction with an
  effect, and the preheader replays that start as the loop's first pass would execute it: the moved instructions in their
  order, between copies with fresh result registers of the instructions that stay in the loop and can fail (copies that
  cannot fail are kept only where a kept copy reads them). The header runs whenever the loop is entered, so a failure of
  the first pass happens in the preheader at the same instruction, with the same message and statement, after the same
  effects; when the first pass passes, the loop runs as before. Moving such instructions stops at the point where the moved
  instructions most outweigh the copies (a loop of one pass pays the copies). An instruction that can neither fail nor read
  memory (constant, move, compare, select, address) moves from anywhere in the loop.
- **Memory**: descriptor reads (`Pos`, `DopeLen`) move only when no instruction of the loop writes memory a descriptor can
  be in (stores, copies, calls and the like; writes of logic elements cannot reach a descriptor). Loads stay in the loop,
  except a load from read-only memory (instance data or the constant pool, which nothing writes while code runs) at a
  constant address (`AddrOf` of a register defined once), which cannot fail and moves from anywhere in the loop (#541).
  Why: no alias analysis yet; the loops in question write logic elements only.
- **A register is invariant when the loop does not define it; a moved instruction's result register is defined once** (so
  every use is still dominated by its definition, the verifier's must-defined rule).
- Idea from the reference simulator: its lowering of a `for` loop computes the step and the bounds once, before the loop.
  It has no general loop-invariant motion (its optimiser numbers values over the dominator tree); ours moves computations
  out of loops after lowering, and keeps failures identical by replaying the first pass's start instead of proving that
  checks pass.

- **One-byte scalar variables are promoted to registers like 8-byte ones** (`Opt::promote`): a boolean, bit or other
  enumeration variable (loads zero-extend, and the lowering stores valid positions only), so `r := r or a(i)` over a
  `std_ulogic` accumulator is a register move (#531).
- **The descriptor of an array constant or signal whose bounds are locally static is a constant of the pool** (`Cx::const_dope`,
  `Cx::static_dope`), also when an index is named by a type mark (`range_value` resolves `t'range` of a scalar subtype), instead of
  a cell of the instance's data: the optimiser folds the lengths, directions and positions (a multi-dimensional index of an instance
  signal array became a few multiplications by constants, a `for i in taps'range` loop lost its direction test).
- Inlined bodies may call natives and read logic arrays (`CallNative`, `LGet`, `LEq`, `MemEq`, `SubtypeCheck`, real arithmetic): the
  native runs the same in the caller's frame and the record of the inlined call keeps its call path (#531, user function calls
  in a loop).
- **The size limit of an inlined body that branches is 60 instructions (8 blocks), not more**, because of `--tier=auto`
  (the default): the queue of a verification library with its 61-instruction table accessors inlined five times into
  `unsafe_push` (516 instructions) ran 11.9 s interpreted (15.0 before) but 10.6 s with the tier (7.65 before), as the
  compiled tier took 10 times as long to compile the bigger units and compiled them at lower quality; with the accessors left
  as calls (they compile small and run on native frames) it runs 13.2 s interpreted and 7.3 s with the tier. Wrappers and
  thin methods are still inlined.

## Shared offsets must stay memoisable and fusable (yavs-ir `opt/memo.rs`, `opt/share.rs`, yavs-interp `pre.rs`, #534, 2026-10-22)

- Optimiser round 11 shares the position and length of an array element across the arms of an `if`, so the offsets of
  `r(i)(14 downto 0) & (r(i)(15) xor ...)` were computed in the block that tests `r(i)(0)` and read in the blocks after it. Three
  consumers had assumed one block: the memoisation of instance constants (`memo.rs`) looked at a block alone and found too
  little to save, so every activation recomputed the offsets; the register sharing gave a shared product a cell that other values
  also use, which stopped the predecoder's linear-run fusion (it needs a register defined once); and every block start stopped a
  fusion in `pre.rs`, so `Alloc`, `Copy` and `Drive` of one assignment, split by a jump that only fell through, stayed three ops.
  `rtl_lfsr` went from about 2870 to about 3750 instructions per cycle and process.
- `memo.rs`: a block whose only predecessor is one reachable block (not the entry, the body or a resume point) starts from the
  instance-constant state at the end of that predecessor (chains of up to 8 blocks), so offsets computed once in the head of an
  `if` are known in its arms.
- `share.rs`: the temporaries of a linear run (`Bin` add/sub/mul, `Pos`, `DopeLen`) defined once keep a cell of their own wherever
  they are read, not only within their block.
- `pre.rs`: a block start stops a fusion only where a jump that is not skipped (or the entry, the body or a resume point) can
  enter; a block reached only by a skipped jump (to where execution goes on anyway) is run through.
- `YAVS_DUMPIR=1` prints the finished code of each function when it first runs, and the predecoded ops of it: the way to see why
  an operation count changed. `crates/yavs/tests/memo.rs` keeps the shape of this assignment (slot-reading ops, `CopyDrive`, no
  `WaveBuf`). Instructions per cycle and process, `rtl_lfsr` (`--tier=interp`): 3550 before, 2890 after; whole design 9.20 G to
  7.46 G; the other `tools/bench` designs and the real testbenches of round 11 are within 0.3 percent.

## `--tier=auto` with nothing compiled costs the interpreter almost nothing (yavs-interp, yavs-llvm, #525, 2026-10-22)

- **The watching loop is a monomorphised form of its own** (`exec_fast::<false, true>`, `run_ops::<false, true>`): the
  loop without a tier has none of the tier's code, and the one with a tier set does only this for code without compiled
  code: name the function in the thread's sampling slot when an activation enters a frame (two stores per activation, the
  slot fetched once), count jumps back and calls (every `WATCH` of them the slot is stored again and the running function
  is looked up), and at a call read the callee's flag byte once, before the frame is pushed: compiled, to be compiled or
  called on a native frame, the loop with the tier makes the call. Forward jumps and returns cost nothing. Why: per-jump
  and per-return counting, two flag reads per call and the tier object passed through the loop cost 1 to 3 percent with
  nothing compiled (#525).
- **`NativeFlags` is one byte per function with three bits** (`NAT_CODE`, `NAT_ASK`, `NAT_CALL`) instead of two arrays,
  so the call check is one load.
- **The compile thread stays inside the CPUs the process was started with** (user, 2026-10-24, #681; it moved itself to the
  complement of the mask by `sched_setaffinity` from #525 to then). The complement of a harness's single core is every
  core of other pinned runs on a shared host. Under single-core pinning the compile thread now shares the simulation's
  core, so wall totals of `--tier=auto` in `tools/bench` and `tools/realbench` include the compile CPU time; give the
  run a mask of two CPUs to let them overlap. Test: `compile_thread_keeps_the_affinity_of_the_process` (`tests/tier.rs`).
- Idea from the reference simulator: it decides tier-up from a per-function entry counter and never looks for compiled
  code inside a running loop, so interpreted code pays nothing per jump. Ours keeps time sampling and the hand-over
  inside loops (jumps back only) and pays per activation and per call instead of per jump.
- Left: the per-activation slot stores, about 15 instructions, are 2 to 3 percent of a design of thousands of one-line
  processes (`rtl_ff`, about 750 instructions per activation) when nothing is compiled; default `--tier=auto` compiles
  that process, so the real run is level with the interpreter.

## Analysed-unit cache (yavs-sem `units.rs`, yavs-cache, CLI, #537, #539, #407; user decision 2026-10-22: build it)

Integrations call `yavs analyze` once per file, and every call parsed and analysed the units the file
depends on again (a verification library's compile order was 2.4 times sim1); a first `elab` of a
library testbench analysed the whole closure from source (3.4 times sim1). The user decided (2026-10-22)
to keep analysed units in the library directories, keyed on their source, the build and their
dependencies; this overrides "no incremental compilation cache" for analysis too. Decisions:

- **One entry per design unit**, `<lib dir>/yavs-cache/units/unit-<key>.img`, written by `analyze` when it
  analyses the unit from source (also a dependency it had to analyse, into that library's directory, and
  `std`/`ieee` units into the work library's), read by `analyze`, `elab` and `run`; `elab` and `run` never
  write one (plan rule 14). The key names the library, the unit, its file path, the design revision, the
  options and the build (`yavs-cache` adds the build); the entry holds the *address* (path, content hash,
  standard of the file) and the *full key* (the address and the full keys of the units it loaded, the
  libraries it named and the answers it got about libraries, below). A reader computes the address
  (the manifest's hash while the file keeps its size and time) and compares the full key of every unit
  the entry loaded with the unit loaded now or the one its own entry would give: an edit upstream
  invalidates everything below it. Limits of their own (16384 entries, 512 MiB) beside the 96 entries;
  a read does not refresh the entry's time (a call reads a hundred).
- **An entry is the recorded analysis, replayed id for id.** The ids of `yavs-sem` index arenas shared by
  all units, so an entry is not a free-standing tree: it records, in order, segments of the unit's own
  arena entries, diagnostics and TIME unit uses, separated by the points where the unit loaded another
  unit, registered a source file or named a library; then its writes to entries of other units (a package
  body completing its package's subprograms) and its table changes, each with the value it found before.
  A replay does the same in the same order and loads a unit it reaches the way analysis would (from its
  own entry). So the `Db` is the one analysis from source makes, id for id (the stage entries keyed by
  extent stay valid); `Db::differs_from` and `YAVS_UNIT_CHECK=1` (`analyze` analyses each file a second
  time without the cache and fails on any difference) check that over the analysis corpora.
- **Ids are written as owner and offset** (`ids::Reloc`): every arena entry records which load made it
  (`Arena::runs`, `Db::loads`), and an id is written as an index into the entry's table of owners (the
  unit itself, a unit it loaded directly or through others, a library, `Db::new`'s entries) and its position
  among that owner's entries; file ids the same way by the files each load read. This is sim1's idea of a
  unit's objects living in an arena of their own that is frozen once analysed, references between units
  being an (arena, offset) pair and dependencies being named with a checksum; yavs keeps its shared arenas
  for analysis and applies the idea only at the boundary of an entry. A unit that refers to anything else
  (an entry of a unit it did not load, such as a clone another unit made) is not stored.
- **Read only what replays exactly.** A replay is planned before anything changes (every unit it will load,
  where their entries land, the libraries and files it will make), so a miss leaves the session untouched;
  the few checks that need values (an entry or table key the unit wrote holds what it held when the entry
  was made) run during the replay, and a failed one undoes it (`Db::rollback`) and the unit is analysed
  from source. A corrupt or stale entry is never an error. Questions a unit asks about libraries without
  loading a unit (whether a unit exists, an entity, the last architecture: `Db::unit_known`, `has_entity`,
  `last_architecture`) are recorded with their answer and asked again before a replay.
- **Table writes are tracked, arena writes are watched; both are audited in tests** (user, 2026-10-23, #603; it replaces
  "checked, not audited", under which a unit's changes were found by hashing every table before and after it, 20% of a cold
  analysis and growing with the closure). Arena entries a unit writes below its own are seen by `Arena::watch` (the bytes
  before the first write). The maps and sets of `Db` that a unit may change are `TrackedMap`/`TrackedSet` (`tracked.rs`):
  while a unit is recorded each write appends the key and the hash of the value it replaced to a log of the table, and the
  unit's own changes are the keys written since its mark whose value now differs (hashes of the raw bytes, xxh3-128, as
  before). When the unit ends its frame forgets its part of the logs and hands the keys it and the units below it changed
  (with the hash of the value after) to the frame around it (`Frame::nested_keys`, smaller map into the larger), so a
  change is hashed once, not once per level of loads; a replayed change is not logged either (the replay already hashes
  it for `nested_keys`). Before, a key a unit loaded inside it added and the unit itself removed again was no change and the
  entry replayed it wrongly; now it is a change of both and the unit is not stored (the conservative side).
  There is no `DerefMut`, so a write that does not go through `insert`, `remove` or `edit` does not compile; a new write path
  cannot bypass the log by accident. The value tables (`well_known`, `validated`, `facts`, `errors`, `generic.reuse`,
  `generic.blocks`: a few bytes each) are still compared by their bytes. The tables are listed in `units::tables!` (a test
  fails when a new serialized field of `Db` or `GenericState` is not listed). A unit stores the keys it added and the keys it removed
  from a set (`GenericState::unused`: the body of a generic package takes out the generics its declaration put in; five
  units of the UVVM and OSVVM libraries were never stored and so analysed from source at every first `elab`/`run`, a
  quarter of that call for a UVVM scoreboard testbench, #424); the removals are read from the set's log
  (`TrackedSet::removed_since`) and written after the added keys, and replayed after the units the entry loads, because
  the net change hides a key the unit's own loads put in and it took out again (a body in a file of its own, whose
  declaration is loaded inside it; the entry recorded nothing and a replay left the key in, found by
  `YAVS_UNIT_CHECK` on the `yavs-lib` corpus, #664); one that changes a key another unit made, or removes one of a
  map, is not stored. A change it shares with a unit loaded inside it, work left pending, an error, or an id it
  cannot name makes it not stored too (`YAVS_LOG=debug` says why). **Reasoning:** the unit's own writes are few, the
  tables are large, so finding them by comparison paid for the whole database per unit; the log costs one append on a write
  and nothing on a read. The risk of a log is a missed write, which would store an entry that replays wrongly; that is why the
  full comparison stays as an audit: `Options::unit_audit` (on in debug builds, so all of `cargo test`; `YAVS_UNIT_CHECK=1`
  turns it on for `yavs analyze`) takes the old snapshot of every table and fails with an internal error naming the tables
  when the comparison finds a change the logs do not. `YAVS_UNIT_CHECK=1` over all the analysis corpora, cold and warm, is part
  of changing the cache. A new table is a `Tracked*` type in `Db`; a new kind of write is a method of `tracked.rs`.
- **Every entry of a replayed unit stays bytes until it is read** (`Arena::push_run`, #424; it replaces "whole pages stay
  bytes", which kept only the full pages inside a run). A run of a unit's entries of one kind is written as the length of
  each entry (a varint) followed by the entries; the replay makes one slot per entry (`ids::SlotPage`: the offset into the
  entry's bytes, an empty `OnceLock<Box<T>>`) that holds the bytes with what decodes them (symbol table, `Reloc`, file map)
  and decodes the one entry the first time it is read. Why: analysing one verification component reads about 1 200 of the
  14 000 declarations and 60 of the 3 400 regions its dependencies replay, and the old scheme decoded every entry of a run
  that did not fill a page (all the scopes, a third of the declarations, most expressions) and the whole page around any
  other. Measured over the analysis corpora (cold, one file per call, instructions): 0.58 to 0.62 of before with the
  other changes of the same round; a cold `elab` reads a quarter to a third of the entries it replays (28 000 of 125 000
  for a UVVM scoreboard testbench) and costs about the same as with pages. The ids are those of an eager replay; reading
  every entry (as `differs_from` does) gives the same `Db`. A decode error is an internal error (as for a page of an
  image): the entry was checksummed when it was read and the plan checked its owners, so a failure is a bug in yavs.
- **An entry holds the elaborator's facts about its own expressions and statements** (`Db::facts`: `'active` reads,
  force and release statements, package-path external names; format 2). They are found when the entry is made, from the
  unit's own segments (those entries are decoded then anyway), written last in the body as ids (so they are mapped by the
  replay like the other ids) and kept in `Db::unit_facts` by load until `settle`. `Db::scan_facts` skips the entries of
  those loads and merges their facts (sorted by id, equal to a scan), so a cold `settle` reads no page of a replayed
  unit (it decoded every expression and statement of the closure: 5 to 16 ms of a 60 to 100 ms cold elab, #552).
  `differs_from` and `YAVS_UNIT_CHECK=1` compare the facts too.
- **The bodies of `std`/`ieee` packages** are never loaded by `analyze` but by every `elab` and `run`; when an
  `analyze` session loaded a package whose body has no entry in any mapped library, it analyses the body
  once in a session of its own, only to store it (its output is not shown: it is the body's, replayed where
  elaboration loads it). The first library of a project pays this once (about 30 ms).
- **Warm analysis of a file** whose manifest record has the same content and whose units all have
  entries registers the file without parsing it; a unit that misses is parsed then. A file whose units a later file replaced (the
  same entity name in two files) is marked `partial` in the manifest (`FileEntry::partial`, an optional key of format 2) and
  is never registered this way: its record lists only the units that remain, and analysing just those met the other
  file's unit instead of the file's own (#577). **A first `elab`/`run`**
  replays the closure's units the same way (`Db::analyze_file_hashed` with the closure's hashes).
- **A file analysed again is registered, not loaded, when nothing could differ** (`Db::warm_records`,
  `Db::analyze_file_lazy`, #581; only `yavs analyze`, never `elab`/`run`, never with `YAVS_UNIT_CHECK`). The
  manifest of a library holds the records of the file (kind, names, standard, dependencies) from the call that
  analysed it; a replay of its units gave the same records back after loading the whole closure of the file
  (85 units and 2 MB for an Open Logic testbench). Now the call plans the replay (the entries of the units and
  of everything they load, their full keys, the answers to the library questions) and takes the records from the
  manifest when (1) every unit of the file has an entry that holds, (2) no entry of the file's units reports anything
  (no diagnostic of the file's own units or file steps; those of the units they load are not shown by
  `analyze`, see below) and (3) the entry's `Head::record`, a hash
  of the unit's manifest record made when the entry was written, equals that of the record the manifest holds
  (a record written by another build is never trusted; `FORMAT` is 3). The units stay `NotLoaded`; a later file
  of the same call that needs one loads it the usual way. Output, exit status and the manifest are those of the
  replay (checked over the 41 realbench selections: records, order and stderr of every call identical).
  Idea from the reference simulator: a unit names what it depends on with a checksum, so that whether a
  dependency changed is answered without loading it; ours checks the full key chain of the entries and still
  loads what a later file needs.
- **`yavs analyze` shows warnings and notes only for the files named in the call** (`analyze.rs`, user decision
  2026-10-23, #584). Before, every diagnostic of the session was shown, so each file repeated the warnings of
  the library units it read (OSVVM: two `W1506` in 400 of 449 calls), and a warm file whose closure reported
  anything had to replay it. A diagnostic whose primary file is not one of the call's files is dropped when it
  is a warning or a note and kept when it is an error (an error in a dependency that changed on disk would
  otherwise fail the call silently); cached and uncached analysis still print the same. `-Werror` counts only
  what is shown. `elab` and `run` are unchanged.
- **Manifest handling per call** (`yavs-lib`, #581): the manifest `Library::read` parses under the session's
  own lock is shared with the session (parsed once, not by the analyzer and again by the session; a parse is
  stale after any session of the process wrote, `WRITES`), `Manifest::to_text` writes the text directly
  (byte-identical to the `toml` text, `toml` renders anything that needs an escape) and `parse_written` splits
  a line at the first space. `analyze` checks that the body of a std/ieee package it loaded has an entry by the
  file's presence (`UnitStore::has`) instead of reading and decoding it, and forgets its session at exit
  instead of freeing the manifest.
- **Found on the way:** a generic instance cloned the formal types of an instance in a hash set's order,
  which differs per process (two analyses of the same files gave different ids); `Remap::formal_firsts` is
  ordered now. The default span (`Span::default()`, file 0 meaning no file) is written as such inside an
  entry (`yavs_diag::map_file_ids`).

## Cold front end, round 2: type marks in the declarations, cheaper entry lookups (yavs-sem, yavs-cache, CLI, #424, 2026-10-24)

Profile of a cold `yavs analyze`, one file per call, over the analysis corpora (frame-pointer build, `perf record`, share of
the cycles of all calls): the replay of the dependencies' entries was 34 to 40 percent, a fifth of it one table.

- **The type mark of a parameter and of a function result, as written, is a field of the declaration** (`Param::mark`,
  `Subprogram::ret_mark`), not the table `Db::written_marks` (a `TrackedMap<DeclId, Span>`). It exists for the spelling in
  messages (`integer`, not `INTEGER`, user rule 8) and was one entry per parameter of every subprogram of the closure:
  replayed one by one, with three hashes each for the frame around the unit, and found again by a scan of the whole table
  for every unit written (12 percent of a UVVM or Open Logic cold analysis). As a field it travels with the declaration's
  bytes. The clones of generic subprograms and the implicit operations carry `None`, as they had no table entry before
  (messages are byte for byte the same).
- **A lookup of an entry makes no strings**: `Key::hex` was 32 `format!` calls (4 percent of a cold analysis, once per entry
  looked up), the notes of `YAVS_LOG=debug` were formatted and `YAVS_LOG` read for every unit replayed, and every id written
  into an entry hashed its owner with the process-seeded `std` hasher. Now `hex` is a table lookup, a note is a closure the
  store calls only when it prints (`UnitStore::note`; the flag is read once), and `Enc` keeps the owner number with the runs of
  each (load, kind).
- **The check of a patch compares bytes first.** An entry that completes another unit's declaration (a package body
  filling in its package's subprograms) is replayed only if that entry still holds what it held when the body was analysed;
  that compared canonical forms (a tree built by a serializer of its own) of both, 8 percent of a cold `elab`. Equal raw
  bytes now settle it at once; only a difference falls back to the canonical forms (which sort the keys of maps).

## Instance bodies of generic packages: what a cold run pays and why it is cloned (yavs-sem `generic.rs`, #555, 2026-10-22)

`load_bodies_in` at the first prefix end of a cold `osvvm_axis_random` elab spent 9.4 of 11.5 ms in `Db::instance_body` (13
instances of 3 generic packages, 35 000 cloned arena entries). Findings and decisions:

- **Where the time went** (perf, cycles, frame-pointer build): a third in the kernel (page faults on the growing heap, #556),
  a sixth decoding the generic body's lazy pages (the first instance of each generic only), the rest in the clone. In the clone
  the memo tables of `Remap` (original id to clone id, 3 000 expressions per instance) used std `HashMap` (SipHash): they cost
  about 26 M of the 742 M instructions of the cold elab. They are `FxHashMap` now (also deterministic in iteration order), `fill_scope` no longer builds a hash map per cloned region, and the
  protected-body loop of `fix_bodies` visits types in id order like the declaration loop (the order decides the ids of the
  cloned bodies; it was the order of a std hash map). Instance body part of a cold elab: 9.5 to 7.5 ms (min of 8), cold elab
  instructions 742 M to 716 M.
- **Every cold run clones every instance.** `load_bodies` ("every loaded package") needs all of them, and the prefix end only
  moves the work there (skipping it at the prefix end made the final `bodies` pass do the same clones). So the cost is paid per
  cold run, not per prefix; a warm run restores the cloned bodies from the prefix image as lazy pages.
- **Tried and rejected: clone at analysis time of the instantiating unit** (so that the unit's entry in the analysed-unit
  cache holds the clone and a replay maps it as pages). The generic's body must then be loaded while the instance's unit is
  analysed, out of compile order in a closure run: the unit's entry records the body and what it loaded, a replay in another
  session loads them in another order, and 43 of 134 entries of a cold osvvm elab missed ("changed", full keys differ).
  A condition that fires only when the body is already analysed does not fire in `yavs analyze` runs (one file per call).
- **Not done, needs the owners of lowering and the stage rules:** cloning a subprogram body only when something reaches it.
  Lowering reads `Body::Hir` through `&Db`, the `low`/`lib` stage entries are valid only for a `Db` that has not grown since
  its analysis, and the state at a prefix end must be the same whether restored or computed, so a body materialised during
  lowering would break all three. A deterministic variant (a per-instance lazily decoded clone stored in the prefix image or
  a stage entry, written by `elab`) is a new cache stage kind; left to a decision.

## Interpreter round 12: drives at zero delay, `'image` and message building (yavs-interp, yavs-kernel, #520, #440, 2026-10-22)

Decisions:

- **A zero-delay `LSetDrive` or `CopyDrive` of one range of a logic signal calls the kernel's fast case directly**
  (`Ctx::assign_code_now`, and the new `Ctx::assign_planes_now` for up to 64 elements): no time conversion, delay mode or
  plane bytes. Anything the fast case declines runs the old path, which also makes every error. Idea from the reference
  simulator: a separate entry for a scalar assignment at zero delay that skips the delay and rejection checks; ours keeps
  one kernel fast case and only shortens the way to it.
- **The message of a procedure call is not evaluated lazily.** An actual parameter is evaluated before the call (LRM
  4.2.2.1 and 14.6, association), and the expressions it holds may have effects (a `numeric_std` conversion of a
  metavalue warns), so a message built for a library assertion procedure (`simAssertion(cond, "..." & integer'image(i))`)
  is built on every call, as the references do. The report expression of an assertion statement is already evaluated only
  when the condition is false (LRM 10.3). So the building is made cheap instead: `'image` of an integer or enumeration value
  is an op of its own (`Op::Image`, no argument cells, no reclaim, no symbol checks: it reads only its argument and writes
  only temporary memory), fused with the three ops that read its string (`Op::ImageStr`).

## `--tier=auto`: clocked processes that are a small part of the run (yavs-llvm, #532, #545, 2026-10-22)

- **A clocked process (`rtl`) is compiled only when it was seen for at least an eighth of the sampled time**
  (`RTL_SHARE`, `cold` in `auto.rs`), besides the 16 instances (`RTL_INSTANCES`). Why: on the CPU designs default
  `--tier=auto` ran 1.4 to 2.0 percent more instructions than `--tier=interp`, all of it in the seven processes it
  compiled (FIFO pointers and memories, the hardware counters, a ring oscillator model: 16 to 63 instances each, 0.2 to
  6.5 percent of the run when decided). Compiled one at a time, each cost 200 to 900 more instructions per activation than
  interpreted (activations counted per function, main thread only): an activation runs a few dozen IR instructions, and
  between two activations so much other code runs that the compiled code starts cold. The processes that gain
  (`rtl_ff`, `lfsr64`, `lfsr64_xor`, `rtl_lfsr`) are more than half of the run. The share is the one when the decision is
  made, early in the run (`ponytail:`).
- **A refused function does not open the LLVM library**: the reasons that keep a hot function interpreted are checked
  before the load, so a run that compiles nothing never pays for it.
- **Compute loops (#545):** the natural loops of #547 already make the tap loops of `hdlm_lfsr` compute again
  (`main` compiled, 0.52 of the interpreter's instructions). Measured before #547 landed and dropped with it: the
  ratio over the innermost loops without a wait (also called clock generators and string helpers compute), over each
  loop on its own (most CPU processes and library helpers), over loops without a wait (the protected-queue testbench
  lost its compiled process, 4 percent more instructions) and without counting waits (clock generators).
- **`--verbose` says when the compile budget refuses a hot function**, once per function, with the compile time it needs
  and when the budget will allow it.
- Measured and not built: counting activations per function in the interpreter (the diagnostic above) to decide on
  time per activation; it costs a few instructions per activation of every process, which #544 is removing.
- Idea from the reference simulator (sim1; read for its design, no code taken): it tiers up on a per-function entry
  counter. yavs keeps sampling (no cost per entry) and uses the share of sampled time instead of a count.

## `--tier=auto` is the default tier (yavs CLI, #125, user decision 2026-10-22)

- **`yavs run` defaults to `--tier=auto` in a build with the `llvm` feature** (user, 2026-10-22); `--tier=interp` stays
  available for the interpreter alone. Why: the real-world suite runs 0.5 to 0.6 of the interpreter's wall on the designs
  where compiling pays (window generator, `hdlm_lfsr`, histogram, compute loops 7x), and the runs where nothing pays are
  within noise since #525, #532 and #545 (docs/STATUS.md "`--tier=auto` as the default: the case").
- **A build without the feature quietly defaults to the interpreter**: no error, no warning. An explicit
  `--tier=auto` or `--tier=llvm` on it is still E0120.
- **`--dap` defaults to the interpreter** (`RunArgs::tier()`): a debugger reads and writes variables in their frame
  slots and `--dap` never starts a tier. An explicit `--tier=` is honoured with `--dap`.
- **`--vhpi` follows the default (`auto`)** (user, 2026-10-22, #125; first slice of this decision kept it on the
  interpreter): the Python testbench framework's regression (68 test cases, 553 test results) gives the same results on
  `--tier=interp`, on the default `auto` and on `auto` with every function compiled at once (`--tier-heat=1,0,0`, 38 cases
  compile). Plugins write variables between activations through `external_variables()`, which the tier already honours.
- **Tools and tests name the tier where it matters**: difftest keeps `yavs` (the build's default), `yavs-interp`,
  `yavs-llvm` and `yavs-auto` (CI runs the first, the second and the last two on the `llvm` build); `tools/bench` passes
  `--tier=interp` in its binary-versus-binary mode (its `--ref` mode and `tools/realbench` already named tiers).

## Reduction idiom as a native (yavs-lower, #531, #521, 2026-10-22)

- **A call of a user function whose body is exactly `v := <identity>; for i in d'range loop v := v <gate> d(i); end loop;
  return v;` is lowered to the native of the unary `std_logic_1164` operator** (`Op::Reduce`, the word-parallel fold the
  `and`/`or`/`xor` of a vector and the `std_logic_misc` reduce functions already use). Conditions (`crates/yavs-lower/src/reduce.rs`):
  one `constant` parameter that is an unconstrained one-dimensional array of `std_ulogic` (also `std_logic`), result
  `std_ulogic`, a local variable of that type, the gate the `std_logic_1164` `and`/`or`/`xor` of two `std_ulogic` (a user
  operator never matches), either operand order, the start value the gate's identity (`'1'` for `and`, `'0'` for `or`
  and `xor`) assigned before the loop or given as the variable's default, `'range` not `'reverse_range`, nothing else in the body. Anything else stays a call (a start value that is not
  the identity, `nand`/`nor`/`xnor`, a second statement, a constrained parameter).
- Why: the inlined loop cost about 7 interpreter operations per bit; neorv32's `counter_increment` (29 reductions per
  activation) ran 13.5 us per activation, now 7.1 us. The fold of the nine values is the one the library body computes
  (the table is the same one the native reduction documents), checked over every vector of up to three values in
  `ieee/user_reduce_idiom`.
- Off with `YAVS_NO_NATIVE_IEEE` (the same switch as the library natives) and with coverage on (the function's own
  statements are then counted). With a debugger the call is not stepped into, like a library native.
- Idea from the reference simulator (sim1; read for its design, no code taken): it has no idiom recognition, it compiles
  the loop to native code and the loop costs a nanosecond per bit there. Ours is a recogniser of one shape because the
  interpreter pays per operation; the compiled tier gets the same call.

## Library layout: an index and a record per source file (yavs-lib, #599; user decision 2026-10-23: per-file records plus a small index)

- **Why.** Every `yavs analyze` read, parsed, cloned, rendered and rewrote the whole library manifest (244 KB
  for the 246 files of satcat5; 30 to 40 percent of a warm call of one file, growing with the library).
  Most of it was `depends_on`, which only the closure of `elab`/`run` reads.
- **Layout.** `<lib>/yavs-lib.toml` (the index, `format = 3`) lists the files in analysis order, each with
  path, standard, content hash, the optional `size`/`mtime_ns` stamp, `partial`, the units it still defines
  as `[kind, key]` (`[kind, key, primary]` for architectures and package bodies) and `record`, a digest.
  `<lib>/yavs-lib.d/<digest>.toml` is the record of one file: its path and, per unit, kind, original name,
  key, primary, standard and `depends_on`. A call reads the index and the records it needs: `analyze` the
  record of the file it analyses again (the analysed-unit cache compares it) and `elab`/`run` those of the
  files in the closure; it writes the records that changed and the index. The index is plain TOML (the
  text `yavs-lib.toml` has always been valid TOML); the library reads it with its own line reader and
  keeps every string in one buffer (a parse is a few allocations and a clone a copy; `toml` is only a
  dev-dependency now). A string with a quote, backslash or control character is escaped as TOML does.
- **Records are never rewritten.** A record is named by the digest (first 16 bytes of the SHA-256) of its text
  and the index names the digest. A write is: the record (temp file and rename), then the index (temp file and
  rename, the commit point), then the unlink of the records the index no longer names (the file's earlier one,
  files that lost every unit). No fsync (#561). A crash between the steps leaves the old index and a record
  nobody names, which is harmless; a reader never meets a record that is not the one its index means, and a
  damaged or missing record is `L0003` naming the record, its source file and the fix. `elab`/`run` hold the
  shared lock of each library they read until the closure is done, so a writer cannot replace a record
  under them (a library that cannot be locked, a read-only one, is not written by anyone).
- **`partial` (#577)** is in the index only: when a later file takes some units of a file, the index drops
  them from that file and marks it `partial`; its record is not rewritten (it still lists the lost units,
  which nothing looks up through the index), and a file left with no unit leaves the index and its record is
  deleted. The analysed-unit cache still parses a `partial` file instead of using its record.
- **The default architecture** (LRM 7.3.3, the last analyzed) is still the last architecture of the entity in
  index order; a file analysed again moves to the end of the order, as before.
- **Old libraries are not converted (user rule "fail early", `yavs-lib` AGENTS.md section 6).** A `yavs-lib.toml`
  of another format version is `L0004` naming the file and saying to delete the library directory and analyze
  its files again (the earlier help said to re-analyze, which `analyze` refused as well). Migration was
  rejected: `elab`/`run` never write a library, so they could not do it, and a library is rebuilt by
  re-analyzing its files.
- **Where the time went.** One warm `analyze` of one file into the satcat5 library: 17.4 to 9.1 million
  instructions, 2.5 to 1.8 ms of wall (pinned, `perf stat`); the index is 99 KB instead of 244 KB. The realbench
  `analyze` phase, one file per call, 41 tests (`--compare`, ratios new / old): warm instructions 0.80
  (satcat5 0.57 to 0.65, Open Logic 0.72 to 0.78, hdl-modules 0.84, PoC 0.74 to 0.77), warm wall 0.87; cold (a
  library built from nothing, dominated by the analysis itself) instructions 0.97, wall 0.96. What is left of
  the library layer in a warm call is rendering and parsing the index, about 40 percent of its user-mode
  time (the rest being startup and the analysis); the index is written by splicing the text of the changed
  file (below, #424). `Manifest::parse_written` and the `toml` fallback of "Per-run overhead" (#426) and #581 are
  gone with the single file.
- **Considered:** splicing the text of one manifest (the analyzer still parses all of it); a `depends_on`
  parsed lazily (the text is still read and written whole); records addressed by path (a record is rewritten
  in place, so an index and a record can disagree after a crash); a database file (an unlisted dependency).

- **The index is written by splicing, and read without lines (#424, 2026-10-24).** `Manifest` keeps the index text it was
  read from or last wrote (`src`) and, per file, where that file's entry is in it (`IFile::block`, set only when the entry
  is the whole text from its blank line to the next one, so a hand-edited index is still read and its entries are written
  afresh). `render` copies the entry of a file that did not change and writes the others (a file analysed again, a
  `partial` one, a new one), so the index of a 400-file library after one more file is a copy and one entry; `adopt` takes
  the text for the one on disk once it is written, so a batch splices too. A debug build writes the whole index as well
  and asserts the two are the same text (all of `cargo test` does). The session no longer reads the index a second time
  to compare the new text with the one on disk: that is `src`. The reader does not split into lines: it reads
  `key = value` where it stands, finds the end of a string eight bytes at a time (`text::plain_len`), and reads unit
  kinds and numbers without allocating; the errors and their line numbers are those of before (the line is counted only
  when there is an error). Measured on a warm `analyze`, one file per call, instructions (G): satcat5 (246 files)
  1.45 to 0.90, Open Logic (386) 3.06 to 2.56, hdl-modules 0.21 to 0.20; cold satcat5 3.97 to 3.70.

## Elaboration-time call cache (yavs-elab `callcache.rs`, yavs-interp, #539; user decision 2026-10-22: build it)

A PoC testbench spent 100 of its 177 ms of warm `elab` in the interpreter running pure functions on constants (a
device string parsed by nine string functions, a time converted to cycles): the same calls with the same arguments at
every elaboration, and again for every instance. Decisions:

- **The result of a pure elaboration-time call is kept** in the stage cache (kind `ecall`, one entry per closure,
  made like `ir` on the second run of it, `Stages::key`: the analysis key, which covers every source file of the
  closure, the build and the optimiser switches, and the `Db` extent). An entry is a list of
  `(callee DeclId and argument values (postcard), result value and the reports the call made)`. A call is looked up
  by exactly that key, in the entry and in the elaboration's own table (a repeat inside one elaboration is a hit too,
  also on the first run of a closure). The entry keeps the newest 8192 calls and 4 MiB; a key or result above 64 KiB
  is not kept. It is written once at the end of an elaboration that learnt a call (no re-read: two runs learning
  different calls at once keep the later one's).
- **What counts as pure (conservative, all of it must hold).** (1) The callee is declared `pure` and its declaring
  region and all regions around it are a package, package body or context (no instance, block, architecture or
  package instance). (2) `lower_one` lowered it on its own with no free object of an enclosing region (no
  instance data: nothing but the arguments goes in); a function that needs package state (shared variables, package
  objects, protected types) runs in the session as before and is never kept. (3) The run itself did nothing but
  compute: `yavs_interp::call_values_traced` reports a taint as soon as the call reaches a native that is not on the
  list `native::pure` (the list package initialisation caching already uses, #407: files, `getenv`, clocks, assert
  state, reflection) or foreign code, and a call that failed (a report at failure severity included) is not kept.
  Nothing else can differ between two runs of the same call: no kernel (so no `now`, signal, wait or driver), no
  shared heap, no access value in or out (arguments and results must marshal, which excludes access and file types),
  fixed default time resolution, the same lowered code (the key holds the build).
- **Reports are replayed.** A pure function may `report` or `assert`: the reports of the call (severity and text) are
  stored with the result and printed and counted again on a hit (`yavs_interp::replay_reports`), in call order, so a
  warm elaboration prints what a cold one does, once per call.
- **Invalidation.** Editing a package constant or body, or any other file of the closure, changes the analysis key and
  with it the key of the entry; a `Db` that grew since the analysis (instantiation) has no entry and looks nothing
  up. Nothing is stored across closures: the library-prefix caches are not used, because a function of a prefix can
  read a constant of a later unit only through the closure. Tests: `crates/yavs/tests/elabcache.rs`.
- **Measured** (`tools/realbench`, phase elab, P-cores 2 and 3, wall and instructions, docs/STATUS.md "Elaboration-time
  call cache"). Ideas from the reference simulator (read, nothing copied): it too does not run a unit's constant
  evaluation again for an unchanged unit; yavs keeps the results of calls, not of units, because its elaboration is
  interpreted and the same call recurs per instance.

## Package constants in elaboration-time calls come from the package state (yavs-lower, yavs-elab, #539, 2026-10-24)

- **Problem:** an elaboration-time call lowers its callee on its own (`lower_one`). A package constant the callee reads
  has no data there, so the lowering evaluated the constant's initialiser at every read. A PoC testbench called
  functions that read `C_BOARD_INFO_LIST` (51 boards built by record constructor functions): each call rebuilt the whole
  table (129 constructor calls, 6300 IR instructions of the callee alone, 35000 constructor calls and 200000 string
  conversions per elaboration), 1.8 G instructions of the 2.0 G of a cold `yavs elab`.
- **Decision:** a function lowered alone for an elaboration-time call (`capture_free_objects(false)`) that reads a
  package or package body constant whose initialiser is not a folded value fails the lowering ("package constant without
  a folded value"); `Model::call` then runs it in the session, against the package state, where the constant was
  built once at package initialisation (the existing fallback for functions that need package objects). Folded
  constants are still inlined. Why not capture the constant as a free object: its value would be encoded into
  instance data at every call, the size of the table.
- **Pure session calls are kept in the call cache too** (`Model::session_call`): a function declared `pure` in a package
  that reads no object of a region reads only its arguments and the package constants, which are the closure's. Its
  result and reports are stored like a lowered call's, unless the call reached an impure native or foreign code
  (`CallTrace::tainted`) or the package state itself was tainted by its initialisation (printed, read the environment,
  a clock or a file: `SharedState::is_tainted`), the same condition under which the state is not stored (#407).
  `call_values_traced` takes the shared state for this.
- **Measured** (`tools/realbench` worlds, `yavs elab`, user instructions of the process tree, bench builds): cold
  `poc_uart_rx` 1.787 G to 0.221 G, warm unchanged (0.035 G); `olo_base_cam` cold 1.029 G to 0.999 G, warm 0.410 G to
  0.380 G; the other six tests of the set are within 0.2 percent. Test: `crates/yavs/tests/elabcache.rs`
  (`a_function_reading_a_package_table_is_answered_from_the_package_state`), work counter `pkg_table`
  (`ops_elab` 3752 to 464).
- **Ideas from the reference simulator (read, nothing copied):** it evaluates a package constant once, at the
  elaboration of its package, and every use reads that object.

## Elaboration-time calls run as lowered; a function asked for often is optimised between two calls (yavs-ir `ElabRaw`, yavs-interp, yavs-elab, #539, 2026-10-24)

- **Problem:** the optimiser runs on a function at its first use. Elaboration-time evaluation (constants, generics and
  ranges computed by library functions; the creation of a verification library's loggers and queues) runs most
  functions once or twice, and the optimiser's time (about 2.4 microseconds per instruction of the function, 17 ms for
  the 134 functions of a small FIFO testbench) was a fifth of a cold `yavs elab` there and three quarters of what the
  calls themselves cost (`olo_base_cam`, `osvvm_axis_random`, `hdlm_fifo_sync`, `poc_uart_rx`).
- **Decision:** the machines of `pkgcall::run_with` (the calls of `Model::call`: a function lowered on its own and a
  call of the session) run under `yavs_ir::ElabRaw`. While it is alive `Program::func` hands out a function or procedure
  that cannot suspend as lowered and does not set its optimised copy; the function's `fetches` count how often it was
  asked for. At 64 (and every 64 after) it is listed (`Program::take_hot`), and `Runtime::promote_hot` optimises the
  listed functions and drops what the runtime derived from their lowered code (`pre`, `live`) at the start of the next
  call, when no frame runs that code (`Arc::get_mut` of the runtime: nothing happens while something else holds it).
  A function whose first request finds an optimised copy stored by an earlier run (`libopt`, the stage cache) takes it
  as before. Verification at first use is kept (`verify_lazily`). The run that follows elaboration optimises what it
  calls at its first call, as before.
- **Why not everywhere:** the version of a function must not change while a frame of it exists (frames hold a pc into one
  version, and `Runtime::pre` is a `OnceLock` per function), so a switch can only happen between two calls. Package
  initialisation and the thunks of the design's own build run as before (one machine runs them all: a function called
  thousands of times inside it could not be promoted, which a first measurement of running everything lowered showed as 2.5
  times the instructions of `poc_uart_rx` before #539's fix).
- **A long loop in one call (#695, 2026-10-24):** the interpreter runs lowered code about 1.5 to 2.3 times slower than
  optimised (a table generator of a million iterations cost +570 M to +800 M instructions). The jumps back of a lowered
  function are counted: `pre::count_back_edges` sends each jump back to a loop head (the retreating `Br`s of a depth-first
  walk of the blocks, also when fused into `CmpBrStep`, `StoreSlotBr` and `MoveBr`) through an `Op::Step` of that `Br`
  placed after the end of the code, and `Machine::step_out` calls `Program::back_edge`. At 2048 jumps back (over all calls
  of the function) the function is listed as hot; if the call has only computed (`Machine::back_budget`: the entry
  function is `PURE` and does not touch shared state, no report was made, nothing tainted, no coverage, a traced call under
  `ElabRaw`, which is what the call cache requires too) it stops with `Fault::Restart`, `pkgcall::run_restarting` (which
  owns the runtime again, no clone alive) optimises the listed functions with `Runtime::promote_hot` and runs the call again
  from its arguments. At most 2 restarts per call (`RESTARTS`, then `ElabRaw::no_restart`); any other call finishes
  lowered and the function is optimised before its next call. 2048 is about the break-even: the optimiser costs about 1.7 M
  instructions for a function of this size and a lowered iteration of the measured loop costs 0.8 k more than an optimised one.
  The price of a restart is the iterations run lowered before it (at most 2048 of them) plus the optimiser; the price of
  counting is about 8 percent on the iterations of a lowered loop (a stepped jump instead of the fused one), nothing on
  any other code. Why the fused ops are redirected, not the dispatch loop changed: a new arm or a check in the jump macro
  moves the codegen of `run_ops` for every design (#583). Not counted: element loops (`Op::Loop`, they run at once), and
  jumps back that are conditional branches (the lowering emits none).
- **Measured** (`tools/realbench` worlds of 40 tests, `yavs elab` user instructions, bench builds, cold = empty cache,
  warm = third run; main with the package-constant change before, after): geometric mean cold 0.974, warm 0.918;
  `poc_uart_rx` cold 0.217 G to 0.182 G, `olo_base_cam` 0.991 G to 0.949 G and warm 0.380 G to 0.299 G,
  `osvvm_axis_random` 0.598 G to 0.565 G and 0.287 G to 0.234 G, `hdlm_fifo_sync` warm 0.122 G to 0.105 G,
  `hdlm_axi_fifo` warm 0.360 G to 0.264 G, `fletcher_bufferwriter_prim32` cold 0.335 G to 0.240 G; nothing above +1 percent
  except two tiny designs (`sc5_timestamp` +2 percent cold, 0.8 M instructions). Output of `yavs elab` and of a run to
  1 us byte-identical on all 40. Work counters: `opt_funcs` (functions the optimiser ran on), design `elab_raw`.
- **Test changes:** `cache_prefix.rs` got its `libopt` from functions optimised by an elaboration-time call; its
  testbenches now call a library function with a loop at run time.
- **Measured, the budget (#695, `yavs elab` user instructions, bench builds, a package function of a 30-operation loop body called
  once from a generic, main before, after):** 1000 iterations 13.48 M to 13.60 M, 2000 14.90 M to 15.14 M (under the budget: the
  counting), 3000 16.32 M to 18.09 M (+11.9 percent, the worst case: a restart just after the budget), 4096 17.88 M to 18.76 M,
  8000 23.44 M to 21.14 M, 20000 40.5 M to 28.5 M, 50000 83.2 M to 46.8 M, 1 000 000 1435 M to 626 M. Budgets of 1024, 4096, 16384
  and 65536 were swept the same way: 4096 lost 47 percent at 5000 iterations, 1024 lost 36 percent at 2000, 2048 is the
  break-even of the optimiser's cost. The 42 `tools/realbench` tests (`yavs elab`, own copies of the worlds): cold geometric mean
  1.0001 (largest +0.12 percent), warm 1.0005 (largest +0.29 percent, `fletcher_bufferwriter_prim32`), no call restarted, `yavs elab`
  and a run to 1 us byte-identical on all. Work counter `elab_restarts`; designs `elab_restart` and `elab_restart_report` (a call
  that has reported finishes lowered, one that has not restarts and reports once).
- Ideas from the reference simulator (read, nothing copied): it too does not generate code for what elaboration runs
  once; it evaluates it in its interpreter.

## The watching loop's tick is out of line (yavs-interp, #521, #544, 2026-10-22)

- **`watch_tick` (name the running function for the sampler, look for compiled code) is a cold function** called every
  `WATCH` jumps back and calls from `run_ops::<false, true>`; the flag tables and the run id are read there, not kept in
  locals of the loop. Why: with its thread-local access inlined, the watching form of the 100 KB dispatch loop ran the
  same instructions as the plain one but 5.7% more cycles, which made `--tier=auto` with nothing compiled 4 to 5%
  slower than `--tier=interp` on neorv32 (now about 1%). A cost that does not show in instruction counts: check cycles
  on every change to the loop's watching code.
- Left (#544): the per-activation set-up of the watching form, 20 to 25 instructions (flag check, thread-local slot,
  two slot stores, loop prologue), about 3% of cycles for designs of thousands of tiny processes with nothing compiled.
  Idea, not built: let the kernel publish the running process once per batch instead of each activation storing the slot.

## Native frames: every exit records the registers live at its pc (yavs-llvm, #554, 2026-10-22)

- **Problem:** `--tier=llvm` ran the array loops of a verification library's codec slower than the interpreter (a
  576-bit `decode(encode(v))` 0.85 s against 0.16 s, queue push and pop 2.3 s against 1.3 s, a window-generator
  testbench 47 s against 29 s). Every guard of a native-frame function branched to the one materialisation block, which
  stored all registers of the frame; the registers were `alloca` words, so after promotion that block was a phi per
  register with one incoming value per guard (about 130 phis and 475 blocks for a 100-instruction procedure). Every
  value defined before any guard stayed live up to the last one, and the phi elimination and register allocator of the
  code generator turned each guard edge into copies. At the unoptimised code generator level (`OptLevel::Fast`, which a
  `--tier=llvm` module over 150000 LLVM instructions and every function of 300 IR instructions or more gets) that was
  the whole cost; at `O2` it was spills inside the loops.
- **Decision:** a native frame leaves through `Gen::leave`, which now stores the registers live before the exit's pc
  (`regs::Live`, the analysis frame mode flushes by) into a per-function spill buffer, then branches with the pc only
  (the chunked pc phi of `exit_for` is unchanged). The one materialisation block passes the buffer to `yavs_rt_deopt`,
  which copies it into the recorded frame; the interpreter resumes at the pc and reads exactly the registers that were
  live. The buffer is an `alloca` that escapes to that call only, so the frame's registers stay SSA values. The blocks of
  failing guards (`exit_if`) are moved behind the rest of the function, so the hot path falls through.
- Registers dead at the pc are recorded as zero. Nothing reads them: the interpreter's liveness (`yavs_interp::roots`)
  decides which registers are roots, and the instruction at the pc is re-executed from live registers only. A test
  (`materialised_frames_keep_the_live_registers`) fails when the stores are left out.
- Measured (`--tier=llvm`, warm cache, core 14, simulate phase of 3000 iterations, median of 5; the interpreter in
  brackets): codec `decode(encode(v))` 0.81 s to 0.132 s [0.164 s], queue push and pop 2.28 s to 0.77 s [1.34 s];
  instructions per 1000 iterations 1.71 G to 0.72 G and 7.5 G to 4.7 G; at `O2` code generation the same change saves
  about 5 percent of the instructions (`integer_vector_ptr.get` 1085 to 1060 per call). Window-generator testbench
  46.4 s to 26.0 s [31.6 s]. `--tier=auto` on that testbench is unchanged (17.4 s,
  interpreter 28.9 s, the reference simulator 4.1 s): the share of its time in compiled code is small
  (about a quarter) and what is compiled is decided by the compile budget; the compile time of the hot
  library functions is 3 to 7 percent lower (`YAVS_LLVM_TIMES`, `get` 19.2 ms to 17.9 ms).
- Shared with a second code generator: the liveness rows (`regs.rs`) and the per-pc spill are generator-independent;
  only the emission is LLVM.
- Idea from the reference simulator (sim1; read for its design, no code taken): a failing check there stores a position
  constant and calls an out-of-line helper per site, so no value flows into a shared failure block. Ours differs in
  that the interpreter resumes the frame, so each site also stores its live registers.

## Floor of a run, round 3: what a tiny run still pays for (yavs-llvm, yavs-interp, yavs-elab, yavs-sem, #426, 2026-10-22)

Measured on a tiny warm test (`perf stat`, pinned, interleaved, 80 runs, medians; docs/STATUS.md "performance: floor of a run,
2026-10-22"). Decisions:

- **The `--tier=auto` compile thread waits 5 ms before it does anything** (`START_AFTER`): it neither reads the cache's list, nor samples earlier, and `stop` ends the wait. Those, and the first wake-ups, cost
  1.1 million cycles of 11 in a tiny run; the sleeping thread alone costs about 0.4 (5%). Why 5 ms: nothing a sampler sees
  in less pays for a compile (`Heat::DEFAULT` is 2 ms plus 40 us per IR instruction). Tried and reverted (0fdb10ee): not
  creating the thread until a tick of the interpreter's watching loop (`watch_tick`, every 128 jumps and calls) after 5 ms.
  A clocked RTL process makes a handful of jumps per activation, so the tick never came and `rtl_ff`, `lfsr64` compiled
  nothing: check `--verbose` notes of the `tools/bench` designs for any change to when tier-up starts. A late start of
  the thread itself needs a tick that does not depend on the shape of an activation (a counter in the per-activation slot
  store of the interpreter, or an observer of the kernel's time steps, which costs the kernel about 50 ns a step).
- **`native::lookup` finds the implementation first and reads `lib/NATIVE.md` only when there is none** (to tell "defined,
  not implemented" from "not a native symbol"). The old order parsed the 38 KB table in every run (4% of a tiny run). Debug
  builds assert that an implemented symbol is documented, so the check the old order made for every run still happens in
  the tests.
- **The scans for the signals and shared variables of loaded packages skip std and ieee** (`walk.rs`): none of their
  packages declares one (test `builtin_packages_declare_no_signals_or_shared_variables`), and the scans decoded every
  declaration of every loaded package (8 of the 11 declaration pages of a tiny run, 12% of its cycles).
- **`Db::release_analysis` leaves a small design to the process exit** (fewer than 50 000 declarations, expressions and
  statements together): the thread and the freeing cost 3% of a tiny run; the memory is a few megabytes. A bigger design
  is freed on a thread of its own, as before.
- **`analyze`, `elab` and `run` parse their command line from a command that holds only that subcommand** (`parse_hot` in
  `main.rs`): the whole command has 70 arguments. Help, `--version`, every error and every other command go through the
  whole command, so their text is unchanged; the test `hot_parse_equals_the_whole_command` compares the results. Saves
  1.6% of a tiny `run` and 3.7% of an `analyze`. The attributes of `Cli` and the `about` texts of the three variants are
  repeated in `parse_hot`.
- **The symbol table and its per-thread mirror start with room for 1 024 names, not 2 048** (a tiny test interns 450 to 900,
  a test of `ieee.fixed_pkg` 2 300, which then grows once): the mirror started empty and doubled, the table was touched at
  random over 100 KB (about 30 page faults, 1.8% of a tiny run).
- **The image holds the packages with run-time objects** (`Db::run_time_pkgs`, filled by `Db::to_image`, read through
  `Db::stored_run_time_packages` while the units, declarations and expressions are as many as when it was made, the
  criterion `pkgcall.rs` uses for its own copy). Finding them decoded every declaration page of every loaded package,
  ieee's among them (a test that uses `std_logic_1164`: 8.9 M instructions against 11.0 M; 14 to 23% on four corpus
  tests that use it). A design that gets implicit aliases of external names (a package subprogram with one) grows the
  declarations and finds the set again. Not used at all in a design with external names: a subprogram's external name is
  bound at elaboration without the design growing (its alias is declared already) and that makes its package a run-time
  one (CI, the second reference's `ename6`, after 6f00c2fa).
- **`alias_external_uses` does nothing in a design without external names**, and `is_std_ulogic` compares the name before it
  reads the declaration: both decoded pages of the cached analysis (an expression page per statement, a declaration page
  per enumeration type) for nothing (4.7% of the instructions of a tiny run).
- **The LLVM stub generator writes the names as one string** (no 1 300 relocations and 5 pages of pointers in a position
  independent executable).
- Measured and not worth it: `-z pack-relative-relocs` (DT_RELR) leaves the cycles and instructions of a tiny run as they
  are (the 16 700 relative relocations cost their page copies, not their processing) and needs glibc 2.36; `arena_max=1`
  saves 1%.
- Not done, for the maintainer: the Rust runtime's probe of the main thread's stack (`pthread_getattr_np` reads and parses
  `/proc/self/maps`) is 6 to 7% of a tiny run; avoiding it needs a C `main` (`#![no_main]`), which loses the runtime's
  stack overflow report and needs the exit status as a number. The two helper threads that start with every piped run
  (the output flusher and the Ctrl-C handler) cost 6% together; starting them late needs a time source that does not
  depend on interpreted activity (the two threads were merged into one with the SIGUSR1 thread: "Release executable: no packed
relocations, no line tables; one signal thread", #562).

## Release executable: no packed relocations, no line tables; one signal thread (#600, #562, 2026-10-23)

- **Packed relative relocations (`-C link-arg=-Wl,-z,pack-relative-relocs`, DT_RELR) are not used** (user, 2026-10-23: PIE
  stays, this option was to be measured). Measured on a release build of main (bench build, GNU ld 2.46, glibc 2.43, pinned
  core 12 and 13, 100 interleaved runs, `perf stat`): the 13 819 relative relocations become 3 960 bytes of RELR and 4 008
  bytes of RELA, a tiny `run` executes 0.4% fewer instructions and 1% MORE cycles (the spread of one run is 5 to 10%), a
  warm `analyze` 0.4% fewer instructions and 2 to 3% more cycles, `yavs --version` has 12 000 more user instructions (the RELR
  applier) and the same 280 page faults: the cost of the relocations is the copy-on-write of the pages they touch, which
  packing does not remove (the finding of round 3 holds). It also cannot be shipped: the Linux release archives are linked by
  zig 0.14.1 (cargo-zigbuild), whose linker fails with "unsupported linker extension flag: -z pack-relative-relocs" (checked
  with a hello-world crate, `cargo zigbuild --target x86_64-unknown-linux-gnu.2.17` and `aarch64-unknown-linux-gnu.2.17`, the
  same flag as in the release job; DT_RELR is an ELF feature, the Windows archive is not concerned), and DT_RELR needs a glibc 2.36 loader
  (GNU ld marks the executable with `GLIBC_ABI_DT_RELR`), against the 2.17 baseline of the archives.
- **Release builds have no line tables** (`profile.release.debug = false`; user, 2026-10-23, supersedes "line tables stay"
  of the same day). They were never loaded: an executable with the debug sections stripped runs the same (cycles 0.99 to 1.01
  of the original, instructions within 0.5%, same page faults). What they cost is download size: the Linux executable gzips
  to 24.9 MB with them and 7.3 MB without. `.symtab` stays (cargo strips only the debug sections), so `perf` and `gdb` still
  name functions on a downloaded binary. The artefacts need no backtrace: both panic hooks are silent (`main.rs`,
  `analyze.rs`), an internal error is a diagnostic. For lines, profiling and benchmark builds add them with
  `CARGO_PROFILE_RELEASE_DEBUG=line-tables-only`; `tools/build-bench.sh` sets that by default (line tables change no code).
- **One thread serves Ctrl-C/SIGTERM, SIGUSR1 and the output flusher** (`yavs_wave::interrupt::Handlers`, started once by
  `yavs run` just before the simulation, #562; user, 2026-10-23: merge, no semantic change, no new unsafe). A run used to
  start three threads for them (the `ctrlc` crate's, `stdout-flush`, `yavs-sigusr1`; the first two cost 6.7% of a tiny run).
  On Unix the signal handlers of `signal-hook` set a flag and write a byte to a socket pair (both safe code); the thread waits
  on the socket with a read timeout of 100 ms (`stdout::TICK`) and after each wake handles the signals, then writes out the
  buffer. What it does is as before: the first SIGINT/SIGTERM/SIGHUP calls `first` (flush and exit 130; or stop between
  cycles with a recorder or `--profile`), the second runs `at_exit` and exits 1, SIGUSR1 pauses the kernel and the run reports.
  The handlers are installed where the Ctrl-C handler was (so from before the first instruction of the simulation), SIGUSR1
  from there instead of a few statements later (a pause before `run_until` is a pause request, answered with the report at
  once). A socket gets one byte per Ctrl-C/SIGTERM, so two signals that arrive before the thread runs count twice (the second
  exits), as with the `ctrlc` crate. If the thread cannot be started, the output is written line by line and W0116 says so (a recorder: E0803, as
  before). Windows keeps the `ctrlc` handler and a tick thread (the console handler runs on a thread of its own there).
  Measured (bench builds, core 12 and 13, 100 interleaved runs each, stdout a pipe): a tiny `run` (`array_attrs`) 7.76 M to
  7.54 M instructions, 10.96 M to 10.59 M cycles (-3.4%) without the `llvm` feature, and 8.08 M to 7.82 M instructions, 10.25
  M to 9.81 M cycles (-4.3%) with it; a warm `analyze` 1 to 2% fewer instructions (the executable has 570 relocations fewer), cycles
  level within the spread. The compile thread of `--tier=auto` is the one helper thread left (#562).
- **The signal thread ends the process without exit handlers when the compiled tier is built in** (`exit_after_signal`,
  #625, agent, 2026-10-23): `exit` ran the static destructors of the LLVM library under a compile thread that was inside
  it: the second Ctrl-C of a hung `--profile` run ended in SIGSEGV instead of status 1 (eight times in ten as reported,
  one in twenty in a debug build here). The buffered output is written first, then `yavs_llvm::exit_now` (`_exit`) ends the process
  with the same status (1 for the second signal, 130 for the first one of a plain run). With foreign libraries or VHPI
  plugins loaded (`--load`, `--vhpi`) the ordinary `exit` stays, so their exit handlers run.

## `--tier=auto` compile policy: no candidate holds up another, natively called functions repay after twice their compile time (yavs-llvm, #572, #520, #493, 2026-10-23)

Found with `YAVS_AUTO_TRACE=1` (new, below) on the window-generator testbench of the real-world suite, cold cache: its hot
library accessors were compiled seconds after they were hot, and between 11 s and 17 s of the run nothing was compiled
at all.

- **`Worker::pick` takes the hottest candidate per compile cost that the budget has room for, and a candidate without
  room waits alone.** It took the best candidate by sampled time per cost and, when the budget had no room for it, set
  `pick_at` and returned nothing until the sampled time had grown enough for that one: every cheaper candidate behind it
  waited too. The one refused on the window generator was a protected-method `pop` whose closure of native callees costs
  400 to 480 ms to compile; the compile thread was idle from 11.4 s to 17.2 s of the run with cheaper hot candidates
  waiting (the verbose note said "possible after 4 s of the run seen" for functions that were ready long before). Now
  every candidate is priced (`tree` cost, `factor`), the first
  in heat order that fits is compiled (`first_fit`), and `pick_at` is the earliest time any refused one fits (reset when
  a new candidate arrives). A candidate that compiled as a callee of another (`done`, native code) is dropped.
- **A function the interpreter calls on a native frame is compiled once it was seen interpreted for twice its compile
  time (`PAYBACK_NATIVE`, was `PAYBACK`, 4), and gets 16 times the budget (`NATIVE_BUDGET`, was 8).** Their compiled code
  runs two to three times faster than the interpreter (#510), the same reasoning as the compute loop's `PAYBACK_COMPUTE`.
  The sampled times of all functions add up to at most the sampled time of the run, so a payback of 2 bounds the compile
  time of this class to half of it and the budget (32 percent) is the cap that binds first; at eight times the budget the
  window generator waited for room for its accessors early in the run. A payback of 1 and a budget of 32 gave no more (window generator, cold, 5 runs each).
- **`YAVS_AUTO_TRACE=1` prints the compile thread's timeline on stderr** (a tuning aid like `YAVS_LLVM_TIMES`): per
  compile its time into the run, level, compile time, how long ago the function was first seen and became a candidate, its
  sampled times and the budget spent; per refusal what it needs and when it fits; at the end the 40 most sampled
  functions. It showed that the wait for the payback rule (hundreds of ms to seconds, the function has to be seen) is
  what separates a cold run from a warm one, not the queue or the compile (10 to 100 ms from candidate to compiled).
- Measured (window generator, `tools/realbench` world, a fresh world per cell, cold = a copy of the pristine world per
  run, pinned to core 13, median of 7, main-thread wall / cycles / instructions from `perf stat --no-inherit`, one binary
  with the settings as environment switches, main at 311f78b7): before 19.2 s / 96.5 G / 320.9 G; the pick change alone
  18.6 s / 94.0 G / 310.2 G (-3 percent); with `PAYBACK_NATIVE` 2 at the old budget (8) the same, 309.9 G; with both
  (this change) 17.4 s / 87.8 G / 293.5 G (-9 percent, total CPU 20.8 s against 20.7 s). The payback alone does nothing
  because the old budget (16 percent of the sampled time) is what the earlier compile of the class runs into. A second
  batch of 5 gave 318.2 / 299.3 / 307.2 / 298.3 G for old, payback 2 (budget 8), budget 16 (payback 4), both:
  batches differ by up to 3 percent, so only the sum is solid. Warm (the cache holds everything): 16.4 s / 259.8 G to
  16.2 s / 251.5 G; the interpreter 29.1 s / 501 G. The queue microbenchmark (`protected_queue`, #493), cold: 1.21 s /
  24.0 G to 0.94 s / 17.8 G (-26 percent instructions; warm 0.53 s to 0.51 s). With the compiled code of before #565 the
  pick change alone gave -12 percent instructions cold on the window generator (340.5 G to 298.8 G): slower compiled
  code makes a late compile cost more. Everything else measured is level with main (12 library and design tests, 9
  head-to-head designs, within 2 percent either way, docs/STATUS.md), which is not level with the interpreter on four of them:
  `rtl_fan` (nothing compiles; +5 percent cycles, the 45 instructions per activation of the watching loop, #544),
  `rtl_ff` (+5 percent; its process compiles and gains nothing now, +3 percent against not compiling it: #575),
  `lfsr64` (+3 percent, level with not compiling it, #575) and `uvvm_sb_slv` (#573).
- **Tried and not built (window generator, same method):**
  - *A cheaper first level (`OptLevel::Fast`) for functions that are not loops* (open question (a) of #520), with no
    upgrade to O2 (`AutoTier::start` upgrades only when the first level is below O2): every function at `Fast` made the
    warm run 21.1 s against 17.0 s (+24 percent). The code is slower and
    an upgrade has little to trigger on: a function called from compiled code is called natively, so the sampler
    attributes its time to the compiled caller and never sees it interpreted or compiled (reasoning, not measured here;
    it is why the previous try, 25 s against 20 s, never upgraded). And a cheaper first level does not shorten the wait:
    compile is 10 to 100 ms from candidate to compiled, the payback wait hundreds of ms to seconds.
  - *Lower payback for other functions* (2, 1): +3 percent instructions cold; *payback 1 for native*: the same as 2.
  - *A budget per class of function* (compute, clocked process, native, plain: each spends its own part): +3 percent,
    noise; *a fixed grant on top of the budget*: no effect on any test; *payback 1 and a budget of 1000 times* in a
    warm run (compile everything hot): 16.4 s against 16.0 s, no faster: the 20 percent of the run still interpreted is
    big processes whose compiled code leaves the code too often to be faster.
  - *`-regalloc=basic`* (a quarter cheaper compile in `YAVS_LLVM_TIMES`): the compiled code is 9 percent slower;
    *`-fast-isel`*: 4 percent slower code; the `CALLBACKS` rule off: nothing more compiles in `neorv32` (the instance and
    share rules refuse the same processes).
  - *Parallel compile threads*: not tried (one thread is the design of #125; the compile thread is busy 15 to 35 percent
    of a run).
- Open: the closure of a native-callable function (`Worker::tree`) compiles callees nobody has seen (a fifth of the
  compile time on the window generator: the logging and string helpers behind `pop`) and the function is published only
  once all exist; at `Fast` they would cost a seventh. The frame-mode code of a function with a loop is compiled with its
  native-frame code (a second module, 60 percent of the first's cost) although the interpreter needs it only to enter the
  loop again after a native frame was materialised inside it. `uvvm_sb_slv` compiles four small string helpers that run slower compiled than interpreted (+5 to
  +12 percent of the main thread's instructions, +22 percent when one of two is left out): #573.
- Idea from the reference simulator (sim1; read for its design, no code taken): it tiers a function up when its own
  entry counter reaches a threshold (100 by default) and hands the compile to a pool of worker threads, each function an
  independent task, with no budget; a candidate never waits for another. The first half is what `pick` now does for
  candidates the budget refuses; yavs keeps its sampled time (no cost per entry), the payback rule and the CPU budget,
  because its compile thread competes with the simulation on a loaded host and compiled code of testbench functions
  gains little unless it repays.

## Compiled memory access: heap addresses are known, objects of different spaces do not share an entry (yavs-llvm, #554, 2026-10-22)

- **A register defined by `Deref`, and every `Index`/`Move` of it, is a known heap address** (`Known::Heap`, the
  generator's per-block knowledge of address registers). `Gen::host` then skips what only a frame, the temporary area or
  the instance data can need: the native frame's own-slots test and its callers'-slots test, and the temporary and
  instance tries of the object table. What is left of an access in a pointer-chasing function (a protected object's
  storage, a table of access values) is the entry compare, the bounds test and the load. Frame-mode code gains the
  same two tries.
- **`Mem::objs` is still 16 direct-mapped entries, but the key mixes the space into the index** (`obj_at`, `Gen::obj_entry`).
  It was the object index alone, so a private and a shared heap object of the same index, the first of every space and so
  the hot ones of a protected-type library (the storage of the queue and of its pointer tables are shared objects, the
  loop's own data private ones), evicted each other at every access: two lookups through `yavs_rt_host` and
  `TierHost::object` per element. The key is the index with bit 3 flipped for a shared heap (space 7, bit 29 of the high
  half); a `Deref` looks the entry up from the access value before it knows the address, whose shared flag is bit 25 of
  its high half (tag 2), so the two sides compute the same key.
- Measured (`--tier=llvm`, warm cache, simulate phase, median of 5, one pinned core; the interpreter in brackets): the
  codec loop, queue push and pop and 2 million calls of an array accessor, with the exit change of the previous
  section alone, then with this one, then with the whole module at `O2` code generation (`YAVS_LLVM_FAST_MIN`):
  0.132 / 0.787 / 0.114 s, 0.137 / 0.685 / 0.074 s, 0.097 / 0.483 / 0.074 s [0.157 / 1.326 / 0.221 s].
- `YAVS_LLVM_FAST_MIN=<n>` replaces the 150000 LLVM instructions above which a `--tier=llvm` module is compiled without
  optimisation (a tuning aid like `YAVS_LLVM_PIPELINE`): a large value gives the code `--tier=auto` makes for the
  functions it compiles at `O2`, which is what a code quality measurement wants.
- **Tried and dropped:** a two-way table (32 entries, as 16 sets of two, a new object in the first way and the old
  one in the second) removed the object callbacks from the profile (`yavs_rt_host` and `yavs_rt_fast` from 5 percent to
  under 1) and made the codec queue slower (0.48 s to 0.50 s), the accessor loop slower (0.074 s to 0.087 s): the second
  compare costs more than the misses it saves. A 32-entry direct-mapped table: no change. The misses that remain are
  the object table emptied by an edit of the heap (`Mem::forget_since`), which refills one object per access.

## Slice positions, memoised descriptors and static waits over slices (yavs-ir, yavs-interp, #520, 2026-10-22)

Decisions:

- **`Inst::SlicePos{dst, dope, left, right, dir, what, dir_what}` replaces the null-slice diamond of a slice**: 0 for a null
  slice (no element, no check, whatever the bounds), else the checked position of `left` with `right` and the direction
  checked like the three instructions it replaces (same messages). Why: the diamond cut every statement with a slice into
  five blocks, so the instance-constant memoisation could not plan it as a whole (each piece paid its own flag test) and
  the interpreter ran six operations where one does. `opt.rs` folds it like `Pos` (a null range, or constant bounds inside
  a known descriptor), the interpreter has an op of its own (`Op::SlicePos`, reads the instruction) and the LLVM tier
  lowers it inline with the same guards (a failure leaves to the interpreter, which raises the error).
- **A descriptor a process builds from instance constants lives in frame bytes** (`opt/memo.rs`, `K::Dope`): an `Alloc` of a
  constant size, one `MakeDope` of it in the same block with instance-constant parts, and from there on only reads of the
  descriptor or its address on every path until the register is defined again (`dope_region`). The slow copy writes it
  once, the fast copy has the address and builds nothing; `DopeLen`, `Pos` and `LayoutOf` of it are instance-constant.
  Not done when the register is also written (`Copy` into it, `Store` through it, `Index` of it): that block keeps the
  `Alloc`.
- **The join of an `if` on an instance-constant condition inherits what holds on both arms** (`Flow::entry`): the arm taken
  is the same at every activation, so a register either arm defines with an instance-constant value is instance-constant
  after the join. Recursion over predecessors is cached per block (`Flow::exits`).
- **Memo plans pay when the dropped instructions exceed half the roots plus 3** (was all the roots plus 3), and a block may
  have 40 roots (was 12). Why: with the descriptors gone, a statement has many small roots (sizes, offsets) that replace
  an equal number of computations one for one; only the dropped instructions are saved. A `LayoutOf` still needed by a
  kept instruction stays (its memory is temporary).
- **A static wait may contain slice descriptors** (`static_wait`): `Alloc`, `MakeDope`, `LayoutOf`, `SlicePos`, compares and
  selects between the wait's `Stmt` and its `Suspend`, so `process (a(HI downto LO))` takes the same short cut as a
  wait on whole signals (they write temporary memory and registers only; a failing check would have failed at the first
  activation).
- Idea from the reference simulator (sim1; read for its design, no code taken): it elaborates every instance with its
  generics as constants, so the slice bounds of an instance fold at elaboration. yavs keeps one function per unit (ir.md
  2) and computes the instance-constant parts once per instance instead.

## Host callbacks of compiled code: natives, concatenations, layouts, fat roots (yavs-llvm, yavs-ir, yavs-interp, #565, 2026-10-23)

Counted on `poc_arith_addw` under `--tier=llvm` (callbacks by instruction kind, whole run): 117 million fat `TempRoot`
helper calls, 105 million `TierHost::signal` calls (64 million vector `SigRead`, 39 million `Cat`), 100 million
`attr.image` and 72 million other natives through the generic `TierHost::step`, 76 million `LayoutOf` helper calls. The
time was not in the instruction counts (they were 14 percent above the interpreter's) but in what each exit costs:
the frame view, the dispatch, the flush and reload of the live registers around the call, the object table swept.

Decisions:

- **A `CallNative` of a native `yavs_interp::direct_native` allows runs through `TierHost::native_frame` in frame-mode
  code too** (`yavs_rt_native` with `NATIVE_FRAME_MODE | pc` as the depth; the argument cells are the registers' values in
  an `alloca`, the result a register). The same natives native frames already ran, so the set and the replay rule are the
  same: a native that fails, hands its call to the VHDL body, meets a due collection or a package initialisation is
  declined (nothing the second run, the interpreter's step, does not see) and the call is stepped. No register is flushed
  or reloaded around a call that succeeds. A native that reports at the stop severity (`ieee_stop`) flushes the
  registers, returns `TIER_SUSPEND` and leaves the frame's pc at the call, which `native_frame` sets (the interpreter
  adds one on `TIER_SUSPEND`). Why not all natives: those outside `direct_native` read the world (files, the
  environment, assert counters) or may call VHDL, which moves the stacks compiled code holds pointers into. Shared with a
  second code generator: `TierHost::native_frame` and the calling convention of `yavs_rt_native`; the generator's part is
  the argument buffer and the result load.
- **`Cat` of non-logic elements and `LayoutOf` of a one-dimensional array of static element layout are lowered in
  frame-mode code** as they were on native frames (`cat_bytes` with address lookup and guards, `layout_words` with the
  descriptor read inline); a failing guard steps the instruction as before. Arrays of two or more dimensions keep
  `yavs_rt_layout`. LLVM-only (a Cranelift tier would lower them the same way or keep stepping).
- **`Mem::forget_since` does nothing when no volatile object (frame, temporary) was found and the heap epoch is
  unchanged**: it swept the 16 entries after every host call. All callbacks gain.
- **A `TempRoot` of a fat pair whose two words are known not to point into a heap is dropped by the optimiser**
  (`opt.rs` `drop_clean_roots`, see the module's decision row): `make_fat` of a string built in temporary memory and a
  descriptor in the constant pool or the frame, the pair every native call and report of a string takes. Shared (IR): both
  tiers; the interpreter saves the instruction, compiled code the helper call and, because no root is recorded, the stepped
  `Stmt` after it. A word of unknown origin keeps the root.
- **A `SigRead` of a logic vector is inline whatever its length** (`Gen::sig_read_block`, `Gen::sig_read_dyn`, `TierSig::cap`,
  `Ctx::code_cap`). The inline form read the byte codes eight at a time and required the whole last group of eight inside
  the signal's elements (`TierSig::len`), so a vector whose length is no multiple of eight (1, 4, 9, 12 elements, nearly
  every control signal and generic-width datapath) always took the callback. The kernel keeps the codes in whole plane
  blocks and writes whole groups (the bytes past the last element are 0), so the guard now is that the elements are
  there (`len`) and the groups are inside the allocation (`cap`); the planes are masked to the elements as before. A count
  that is a run-time value (a generic width) calls `yavs_rt_codes`, a pure function of the codes, the group count and the
  mask, after the same guards. The callback this removes costs 220 host instructions. Shared: the `TierSig` field and the
  kernel accessor; the second code generator calls the same helper.
- Not changed: `is_inline` and the `--tier=auto` callback count (`too_many_callbacks`) still treat these natives as
  leaving the compiled code, the policy owner's call (#507 was tuned with the dearer callback).
- Idea from the reference simulator (sim1; read for its design, no code taken): its `numeric_std` and
  `std_logic_1164` operations are intrinsics found by subprogram name and called with the same argument array as any
  compiled function, so a call builds no frame object and goes through no generic dispatcher; the one generic exit
  (for signal scheduling, assertion failures and the like) takes a flat array of scalar arguments and is not on the
  path of data operations. `native_frame` is the same shape on top of our frames: the argument cells in an array, one
  result word, the generic step only for the cases the lean call declines.

## Interpreter round 14: what the dispatch loop keeps for every entry (yavs-interp, yavs-ir, #520, 2026-10-23)

`Machine::run_ops` is entered once per activation of a process. What the compiler hoists out of its loop (addresses of
fields of the machine, loads of the runtime) is stored on the stack on every entry: about 105 instructions and 60 stores
before the first op, 17 of them the address spills (`lea` and store) measured by disassembling the loop. In a design of
many small processes that is a fifth of the interpreter's cycles (`perf annotate`), on the realistic ones (neorv32, the
window generator, hdl-modules) 18 to 25 percent of `run_ops`. Decisions:

- **Package data (`PKG`) is not one of the spaces `mem_fast` and `load` know inline.** Its branch (a returned slice of
  `pkg_init`'s vector next to those of the temporary area, the heap objects and the instance data) was merged by the
  compiler into one load through a selected address, so the address of every vector had to live in the frame of the
  loop. Package data goes through `load_slow` (`obj`); `store` never knew it. Measured by `run_ops`' disassembly: 17
  hoisted address pairs before, 11 after.
- **The rare paths of the slot accesses are out of line** (`slot8!`, `store_slot!`, `Op::LoadSlot`: `load_slow` and the new
  `store_slow`; the waveform head of `Op::LSetDrive`: `waveform_head`). Each was an inline copy of the memory access
  (`load`, `store`, `temp_alloc`) in a branch that is taken only for an error or an unusual frame.
- **The start of a slice (`SlicePos`) of a descriptor in frozen memory is instance-constant** (`opt/memo.rs`), like `Pos`:
  the checks fail the same way at every execution, and the `Aff` chains that follow it become instance-constant too.
- **A signal reference (`SigVal`) with an instance-constant offset and length is a root of two words** (loaded and stored
  with `Width::Sig`, two slots; the interpreter loads it with one `LoadSlot8x2`, `pre.rs`). Not remembered: a reference
  of constant length, and one that a `SigReadScalar` reads. The interpreter fuses a single element with the read, the
  wait or the edge test that uses it (`SigRead1`, `WaitSuspend`, `EdgeBr`), and the compiled code knows the length word
  of a constant length and drives and reads such a reference inline; remembering them cost 4 percent of instructions on
  the chain of 1000 one-line processes (the wait loses its fusion) and 6 to 11 percent of cycles with `--tier=auto` and
  `--tier=llvm` on the shift-register designs. The gain thresholds of the plan are unchanged: lowering them
  (`gain >= roots / 4 + 1`) costs 4 percent of instructions on `poc_arith_addw` (more roots than computations dropped).
- **`AddrOf` and `FCmp` are ops of the fast loop** (`Op::AddrFrame`, `Op::AddrUp`, `Op::FCmp`; the addresses of the other
  spaces are `Op::Const`): a register that is defined more than once is not preloaded, and an address up the static chain
  never is, so a nested procedure's reads of the variables of its enclosing subprogram, and every comparison of reals,
  were generic steps (a copy of the instruction, `step_out`, the big `match`): 4.4 percent of the operations of
  `sc5_fifo_repack` (its random-number procedure) and 1.1 percent for the reals. An address the machine cannot make
  stays a step, which reports it.
- Tried and not done, each measured on the realistic set (see `docs/STATUS.md` "Interpreter round 14"): a copy of the loop
  for functions of simple ops (`LITE`, 63 arms excluded by const guards; the entry of an activation 105 to 70 instructions)
  gains 5 percent on a design of 4000 near-idle processes and nothing on any realistic design, because their processes use
  memory ops; out-of-line memory access (`mem_fast`, `load`, `store_idx` as `inline(never)`) costs 5 percent of
  instructions; outlining the growth of the vectors, the frame push and `temp_alloc` costs more calls than it saves
  spills; `k` in a register (a by-value closure for `bin_range_slow`) is 20 percent slower on `compute_loop`.
- The speed of the loop depends on the layout of the code: the same instructions, built with 64-byte aligned functions
  (`-C llvm-args=-align-all-functions=6`), run `calls` 19 to 33 percent, `compute_loop` 10 percent and the histogram
  testbench 6 percent slower. Changes to the loop are judged by instructions and by the geometric mean over many designs
  on both cores, never by one design (`tools/bench`, `tools/realbench`; #576).
- Idea from the reference simulator (read for the design, no code taken): its interpreter keeps the hot state in a small
  struct on the stack and the registers in a stack array, so entering a function stores a handful of words; the shared
  interpreter state of yavs lives in the machine, which the compiler addresses field by field.

## Interpreter round 15: element loops (yavs-interp, #520, 2026-10-23)

`poc_stat_histogram` spent 95 percent of its time in two functions (`get_row`, `to_slm`) whose loops copy the
elements of logic vectors one by one: 8 to 9 interpreter operations per element (a position, two linear forms, `LGet`,
two more, `LSet`, the compare, the step and the jump), about 500 instructions per element. Decisions:

- **An element loop is entered once and runs all its iterations but the last at once** (`pre::elem_loops`,
  `Machine::op_loop`). The recognised loop has a body of linear index arithmetic (`Pos`, `Aff*`, `Bin`, `BinRange` of
  add, subtract and multiply, `DopeLen`, loads of the bounds and of the process variable's slots, `Cmp`/`Select` on
  values the loop does not change) with one `LGet` and the `LSet` of it (a copy) or one `LSet` of a code the loop
  does not change (a fill), and a latch that counts the loop variable by 1 or -1 against a bound (the compare fused
  with the step, the step before or after the compare, either edge going round again). Every value of such a body is
  affine in the iteration number, so one evaluation of the body's steps at the entry (value at iteration 0, change per
  iteration) gives every index of every iteration; the positions, the atoms of the linear forms (within 2^30 as the
  `Aff` ops require), the bounds of `BinRange` and the elements' memory are checked at the first and last iteration
  (affine, so everything between is in range).
- **Failures stay identical because the kernel never fails.** If any check does not hold, nothing is done and the
  generic ops run the loop from its first iteration, so a bounds error comes at the iteration, with the message and
  the earlier writes, it always did. If they hold, the kernel does the iterations that go round again, and the loop
  variable is set to where the generic ops go on: they run the last iteration (the one that leaves the loop) from the
  loop head, so every register, the exit and the compare's result are exactly theirs and nothing has to be written
  back. A code in memory above 8 (never made by the interpreter) stops the kernel at that element for the generic
  ops to report. With a tier set, the iterations are capped by the loop-count budget of the watching loop as the
  search loops are.
- **The entry is a new op placed after the end of the code** (`Op::Loop`), and every branch to the loop head from
  outside the loop is sent to it (the entry is often a compare that falls through to the head, not a jump). It goes on at
  the head when the kernel did nothing. A loop entered by falling through, or through a `Case`, stays generic.
- **The copy of a run of elements in the same order** (both indices count the same way) is a copy of planes (inside
  one block of planes words, else `lcopy`) when the two runs are in different objects or do not touch; a copy within
  one object that overlaps (a shift: `v(i + 1) := v(i)`) or in opposite directions is an element loop that reads and
  writes in order, so the sequential result is the plain loop's. A fill of a run is `lfill`.
- **Not done, measured or judged:** more than one `LGet`/`LSet` in a body, a body with the loop variable stored in a
  variable (`j := j + 1`) or an element kept in a variable (the value carried to the next iteration), loops over
  integer arrays (`Index` with loads and stores): none of them is among the hot loops of the benchmark designs and
  testbenches counted (`YAVS_DUMPIR=1` says why a loop is not one: "is not an element loop: op N").

## The activation state of a process in three cache lines (yavs-interp, #520, 2026-10-23)

A design of thousands of processes (4000 near-idle processes on one clock) takes a data cache miss on every line of
every process it touches: 30 L1 misses and 4.3 L2 misses per activation, and 56 percent of the instructions of an
activation are outside the dispatch loop's own work (`InterpProcess::run`, `run_quick`: 245 of 686 instructions, a third
of their cycles waiting for the first load of each line). The fields were spread over ten lines of the 848 bytes of
`Machine`. Decisions:

- **`Machine` is `repr(C, align(64))` with the fields an activation of a small process reads and writes in its first
  three lines** (frames, registers, pause; root slots, temporaries, runtime handles; instance data, drivers,
  debugger, execution count and the flags), the heap's `since_gc` and quarantine next (`Heap` is `repr(C)` with them
  first), the rest after. `hot` and `execs` moved from `InterpProcess` into `Machine` to sit there; the debugger state is a
  `Box` (a null check instead of 40 bytes). `InterpProcess` is 896 bytes, as before, and the peak RSS of 20000 instances is
  unchanged (178.3 MB, 177.8 MB).
- **Measured** (`tools/bench` designs, cores 4 and 5, `perf stat`): `rtl_fan` L1 data cache misses 597 M to 516 M, L2
  misses 78.5 M to 61.1 M, cycles 0.85 to 0.91 in four paired runs; `rtl_ff` 0.97, `rtl_chain` 0.93; the designs of a few
  processes (neorv32, compute_loop, calls, protected_queue) are within the noise of the host (instructions equal to the
  per-mille). Cycles on a shared host move by 10 percent between runs, so the evidence is the miss counts.
- **Not done:** frames, registers and root slots of a process are still three separate allocations (one line each at
  least); the kernel's per-process state is another set of lines (`run_batch`, `wait_on_again`). Prefetching the next
  process of a batch needs `unsafe` or the kernel.

## Interpreter round 16: superinstructions from the op census (yavs-interp, #520, #518, 2026-10-24)

The user decided (2026-10-24) that superinstructions are wanted, one at a time, aimed at the biggest gaps to the reference
simulator; a second code generator may replace the LLVM tier later, so a fusion that every backend could lower belongs in
the IR and one that only the interpreter's dispatch gains from is marked interpreter-only here. The ranking comes from
`op-census` (crates/yavs-interp/AGENTS.md section 5), which now also has a time column (nanoseconds from the end of one op
to the start of the next, the clock reads subtracted; ops that suspend or return include the kernel) and
`YAVS_CENSUS_TOP=N` for the length of the tables. Each change lands only if the executed instructions drop and the
cycle geometric mean over the designs of `tools/bench` and `tools/realbench` (32, later 45; windows of 50 to 200 us for the long ones; one core of the quiet CCD, median of 5 to 7 interleaved runs with per-binary copies of the worlds) is not worse.

- **`LoadUp` and `StoreUp` (interpreter only).** An `AddrUp` (the address of a variable of an enclosing subprogram, up the
  static chain) followed by the `Load` or `Store` through that address, which nothing else reads, is one op that walks the
  links and touches the slots of that frame directly. A nested procedure reads its parent's variables this way on every
  statement (`sc5_fifo_repack`: `AddrUp -> Load` was 4.7 percent of its ops, `Stmt -> AddrUp` 4.5). A failing access reports
  what the plain `Load`/`Store` reports (the slow path is the old one). The compiled tiers are unaffected.

- **`Uniform` (interpreter only).** `math_real.uniform(seed1, seed2, x)` is the stimulus generator of many testbenches
  (`sc5_fifo_repack` calls it 1.1 M times in 30 us of simulated time). As a generic native call it cost 16 percent of that
  run's instructions (`call_native`, `push_args`, `obj_mut` and the native itself) for 70 instructions of arithmetic. The
  op loads both seeds, and when they load and are in range (`<= 2147483562` and `<= 2147483398`) steps the generator in
  place (`ieee::uniform_step`, shared with the native, so the numbers are the native's) and stores the seeds and the real;
  in any other case it is the native call, which has changed nothing by then and reports or faults as before. The arm of
  `run_ops` calls an out-of-line function: the same op with its body inline in the arm moved the other designs by +0.2 to
  +2.9 percent of instructions (the large function is laid out differently, #576), the call keeps them within 0.2.

- **A check's message is built only for a failing call (IR, every backend; `opt/sink.rs`, `opt/inline.rs`).**
  `poc_arith_addw` spends a third of its time building the message of `simAssertion(cond, "..." & image(x) & ...)` for
  calls that pass (30 of 100 percent of its instructions with the message replaced by a constant). The message is an
  actual parameter and the LRM evaluates it before the call, so the evaluation may be skipped only where nothing of it
  can be seen. Two parts. (1) A call that passes a string a `Cat` filled just before it may inline a callee of up to
  four times the usual raw size (`assertion`, 139 raw instructions for 41 after optimisation, was never inlined); the
  wider callee is looked at only for such a call, so other designs do not pay for the optimisation of the callees they
  never inline (`hdlm_axis_bfm` +14 percent of its instructions with a general widening, +1 percent now), and a second
  round (`run_wide`) takes the call a wrapper (`simAssertion` in front of the protected method) brought in. (2) After an
  inlining, `sink::run` moves instructions from the straight run in front of a `CondBr` to the start of the branch
  that reads their results. What moves: register computations, loads, descriptor lengths and index arithmetic over
  memory the run built itself (temporary blocks, the strings `'image` of an integer or enumeration returns), frame
  slots and the constant pool, `Alloc`, `MakeDope`, `LayoutOf`, `Cat` and `Store` into such blocks, `+`, `-`, `*` and
  the length check of `&` where all operands are lengths. What stays: every call (`to_integer` of a vector that holds
  `U` reports at every call, so it is evaluated at every call), a division, a range check of a counter, an index or
  slice check, a read of memory the run did not build. The one thing that moves although it can fail is the length
  arithmetic and the index check of the concatenation: it fails only for a string of 2^31 elements or more, which no
  run can hold (the same stop the allocation gives), so skipping it where the string is never read changes nothing a
  run can show; the operands are lengths by construction (`DopeLen`, constants, sums of them), never a value from the
  design. Moved results read nowhere but the target, writes go only into blocks a moved instruction made, a read moves
  past no write that may reach its block and an allocation past no call or statement end, no register lives across a
  suspension, and the moved code writes fresh registers (the code before the branch reuses the originals). Checked on
  `poc_arith_addw` (instructions 25.57 G to 18.41 G, 0.72; the other designs within 0.2 percent), on twelve designs of
  failing and passing checks with messages that divide by zero, overflow, index and slice out of range, call an impure
  function and call `to_integer` of a vector with a `U` (identical output with the optimiser off:
  `crates/yavs/tests/sink.rs`), and on 850 random programs of checks with messages of up to eight pieces in nested loops
  against the build without the optimiser (no difference).

- **`LCopyDrive` (interpreter only).** An assignment of a constant, or of anything else the compiler copies element by
  element into the waveform (`ctrl <= ctrl_zero_c` field by field in a reset branch: 589 K times in 20 us of
  `neorv32_single_min`, 355 K in 50 us of `fletcher_bufferreader`), was a `WaveBuf`, an `LCopy` into it and a `Drive`
  that reads it: the waveform is built in temporary memory and read back. The peephole that already drops the
  waveform for a `Copy` or an `LSet` of element 0 (`CopyDrive`, `LSetDrive`) takes the `LCopy` of element 0 too; at zero
  delay, for a target that is a logic signal of exactly the copied length within one block of planes (2 to 64
  elements), the planes of the source go to the kernel as they are (`assign_planes_now`, the kernel's fast case);
  anything else (a delay, a longer vector, one element, a reference of several parts) is the three instructions as
  the IR has them, made from the registers of the dropped ones. The arm calls an out-of-line function.

- **`LEq` (interpreter only).** `=` of two logic vectors was a generic step (a copy of the instruction, `step_out`, and
  both vectors made as heap objects before they were compared): 100 to 220 ns, 1.6 percent of the time of
  `sc5_fifo_repack` and 1.7 of `neorv32_single_min` for 0.03 and 0.8 percent of their ops. It is an op that compares the
  plane words of the two blocks in place (bits beyond the length are not compared, as `Planes::from_blocks` clears
  them) in an out-of-line function. `osvvm_axis_random` 0.949 of the instructions, `fletcher_bufferwriter_512` 0.965,
  `neorv32_single_min` 0.982, the others within 1 percent.

- **A waveform buffer is dropped across calls too (interpreter; `Code::no_tier`).** The peephole that does not run the
  `WaveBuf` of a drive whose value is copied in (`CopyDrive`, `LSetDrive`, `LCopyDrive`) refused any run with an
  instruction in it after which compiled code may take over the frame (a call, a native that can hand its call to a VHDL
  body: `CallIeee`, a protected call), because the registers of the buffer are undefined there. That was every drive of a
  value that a native computes (`s <= a + b`, `x <= f(y)`, 133 sites in the neorv32 testbench). The ops from the dropped
  buffer to the op that drives it are now a range in which `tier_go` does not enter compiled code (the entry is asked
  again after the next jump; one comparison of two numbers, only on the way into compiled code), and the fusion goes
  across the call. The guard is needed: without it `generics/generic_blocks_2019`, `rtl/wide_bus_attributes` and
  `rtl/wide_bus_lean` fail under `--tier=auto --tier-heat=1,0,0` (compile everything at its first call, hand over at
  every return); the CI smoke runs that on `generics/`. The old rule also left one case open: a call directly before the
  dropped buffer returns at the buffer's first instruction, which is no op any more; the range starts there. Instructions
  `olo_axi_master_full` 0.865, `rtl_ram` 0.933, `neorv32_single_min` 0.974, `poc_cache_cpu` 0.981, `fletcher_bufferwriter_512`
  0.988, `fletcher_bufferreader` 0.990, `poc_uart_rx` 0.969, geometric mean over 45 designs 0.9935; `poc_arith_addw` does
  not change (its drives use a waveform made once per instance, not one per activation).

- **What the census says about the six tests, and what was measured and not landed.** Instructions per executed op are
  150 to 220 on the clocked designs (kernel and the bodies of the heavy ops), the dispatch of one op about 14: a fusion
  of two cheap ops is worth 0.2 to 0.3 percent of a design whose ops are heavy and 2 to 5 percent of one whose ops are
  cheap (`sc5_fifo_repack`, `poc_cache_cpu`), so the ranking is by time per op (the census time column), not by pair
  counts. `poc_arith_addw`: 31 percent of the instructions were the messages of passing checks (sink, above); the rest is
  `CallIeee` 9.5 percent of the time, `Drive` 5.3, `AllocCat` 5.0, `AllocReadVec` 4.5 and the kernel (20, `Suspend`).
  `neorv32_single_min`: kernel and process entry (`Suspend`, `Ret`, `WaitSuspend`) about half of the census time, then
  `SigRead1` 4.3, `Drive` 3.2, `AllocReadVec` 3.2, `LSetDrive` 2.8, `CallIeee` 2.6 percent; flat, so only `LCopyDrive`,
  `LEq` and the buffer drop landed. `fletcher_bufferwriter_512`: `finish_direct` 10 to 17 percent of the cycles (the
  kernel's wide commit, #519), `run_ops` 29, then `layout_words`, `layout_word`, the `SubtypeCheck` closure and `memcmp`
  together 7 percent (records with vector fields: the layout memo holds one descriptor per type). `fletcher_bufferreader`:
  `SigVal`, `Field`, `FieldOf` and `SigReadVec` chains of records, 3 percent of the time each. `sc5_fifo_repack`: 49 percent
  of its time was `rand_next` (`Uniform`, `LoadUp`, `LEq`). `poc_cache_cpu`: flat; `Load` is 7.5 percent of the time and
  7.6 M of its 8.1 M plain `Load`s read instance data through `load_slow` (#674).
  Measured and not landed (aligned bench builds, `--tier=interp`, instructions): `SigVal` pairs (static neighbours are
  rare: `SigVal -> SigVal` is 8.5 percent of the executed pairs of `fletcher_bufferreader` and 262 pairs next to each
  other in its code; +0.09 percent on `neorv32`), `SigRead1` with the compare-branch on it (-0.2 percent), a latch whose
  step is not the next op (`sc5_fifo_repack` -1.5 percent, the geometric mean 0.9998; the arm moved `compute_h2h` by
  +0.5), instance data in the inline match of `Machine::load` (`poc_cache_cpu` -3.7 percent, every other design +0.0 to
  +1.0, 45 designs 1.0022: the entry cost of `run_ops`), the statement markers of signal statements (landed by #588 at
  the same time, same list), and, to make `poc_arith_addw` cheaper, `to_integer` of the message (its warning for a vector
  with `U` is an effect at every call: not skipped, not shared with the identical call of the condition).

## Interpreter round 17: loads from the other address spaces (yavs-interp, #674, #518, 2026-10-24)

Measured as in round 16 (aligned bench builds, `tools/realbench/run.py --compare` with per-binary worlds on one core of the
quiet CCD, `tools/bench/run.py --compare`); the cycles of single designs move by percents with the code layout (#576) and
with host load, so the rule is: instructions drop, the geometric mean does not get worse.

- **`load_slow` checks the spaces it is called for in place (#674).** The op census has a new table, `slow loads`
  (`Machine::load_slow` calls by running op and address space). It shows what the issue guessed only partly: the loads
  that leave the inline match of `Machine::load` are plain `Load`s and `LoadOff`s through a register that holds an
  address only at run time. `poc_cache_cpu` (9.3 M of them, all instance data) reads the descriptor of a vector
  parameter, whose address the caller passed: `reverse`, `mux` and the other unconstrained-vector functions load the
  three words of the descriptor of a signal's vector, which lives in the instance data. `uvvm_func_cov` (5.5 M, constants)
  and `poc_arith_addw` (17 M, package data) read descriptors of constants and package variables the same way. A register
  defined once by an `AddrOf` is already a `LoadInst`/`LoadSlot`; the others have no address space known at predecode,
  so the specialisation by space at predecode that the issue proposed has nothing to specialise on, and the body of
  `load_slow` was the cost: 30 instructions with six saved registers (the error paths of the match were in it). It now
  has one short match on INST, PKG, CONST (stored as bytes or expanded, the same lookup as `mem_fast`) and SHARED and
  a slice of the right width; everything else and every error go to `load_cold`, the old body. The call sites in
  `run_ops` are unchanged (an arm that calls an out-of-line function does not move the layout). INST in the
  inline `load` match stay rejected (round 14, #674: +0.0 to +1.0 percent of instructions of the designs that never
  read instance data through a pointer). Instructions: `uvvm_func_cov` 0.973, `poc_cache_cpu` 0.994, the rest within 0.2
  percent.
- **A layout the memo already holds is read without `layout_words` (#518).** `FieldOf` (25.7 M per `fletcher_bufferwriter_512`
  run) and `LayoutWord` ask for one word of the layout of a record whose unconstrained fields keep their descriptor in a
  process variable; the memo (one per type) is compared with the descriptor bytes at every call. `layout_word` now does that
  comparison itself and returns the word, instead of calling `layout_words`, which looked the memo up again and wrote
  `ckey`; a constant descriptor still goes through `layout_words` once (it sets `ckey`, after which the `ckey` test in
  `layout_word` answers without reading the descriptor). A descriptor that differs from the memo's goes straight to
  `layout_build` (the old tail of `layout_words`), so a record type used with several descriptors does not compare twice.
  Not done: an associative memo keyed by descriptor, or a layout kept in frame bytes beside a variable whose descriptor is
  fixed after its declaration; only 1 percent of the calls miss, and the remaining cost is the comparison itself.
  Instructions: `fletcher_bufferwriter_512` 0.982, `fletcher_bufferreader` 0.980. `SubtypeCheck` as an out-of-line
  `op_*` method was tried: +0.1 percent instructions on `poc_arith_addw`, no gain elsewhere, dropped.
- **Not landed.** `AddrUp` followed by the `Uniform` op (1.8 percent of the ops of `sc5_fifo_repack`; the `AddrUp -> Load`
  fusion of the task is `LoadUp`, round 16, and the census shows no other `AddrUp` pair); the `mux`/`ffdre` functions of
  PoC (about 50 operations per call, mostly `NewPair`, `FatRoot`, `CallIeee` and `Alloc` for the fat pairs of the
  `numeric_std` natives: an IR-level change of the calling convention, not a fusion); the second `to_integer` of
  `poc_arith_addw`'s checks (a native that warns at every call cannot be evaluated once without knowing whether the
  first call warned: it needs the native to report that, and the optimiser to give a call two results).

## A process that never suspends polls for a stop request (yavs-interp, yavs-kernel, #583, 2026-10-23)

The kernel looks at `Interrupt` between cycles, so a process without a wait statement (W3102) kept a run that
had to end cleanly (the first Ctrl-C with `--wave`, which finishes the file) inside one activation for ever. Decisions
(user, 2026-10-23: only if it costs nothing measurable):

- **Only a process whose function has no suspension** (`FuncKind::Process` without `MAY_SUSPEND`) gets the check, and only
  on its jumps back: they are `Op::Step`s of the IR `Br` (`pre::Code::new`), and `Machine::step_out`, which already runs
  out of line for every stepped instruction, answers a `Br` with `Flow::Suspend` when `Ctx::stop_requested` (one relaxed
  load of the kernel's flag word, which `Shared` now carries). The activation ends as a `wait;` would, the process never
  runs again and the kernel ends the run at the end of the cycle. No op, arm or loop of the dispatch loop changed: an
  attempt with a check in the `Br` and `BrBody` arms moved the instructions of unrelated designs by +1 to +9 percent
  (`calls` +8.8, `compute_loop` +6.8, `protected_queue` +4.7: one more arm moves the loop's register allocation, as
  `docs/decisions.md` "The shape of the interpreter's op loop" found), so the check lives outside it.
- **Not covered:** a loop without a wait inside a process that does wait, loops in compiled code and under the debugger,
  and `SIGUSR1` (the hang report reads kernel state that an activation in progress holds) and pause (the process
  would have to be resumable at the jump back, which needs its resume point exposed). Each needs a poll in the loops of
  every process or kernel support, which is not free on the dispatch loop (a check at every jump back is a load, a compare and a
  branch on the hottest path).

## Huge pages for the heap (yavs-cache `map.rs`, yavs CLI `main`, #556; user decision 2026-10-22: in-process, no re-exec)

A cold elaboration of `osvvm_axis_random` took 8 270 page faults (a fifth of its CPU time, mostly in the kernel) because
glibc's `malloc` grows the heap in small steps. glibc's own tunable `glibc.malloc.hugetlb=1` removes most of them but is read
at process start only, so the binary would have had to re-exec itself. Decisions:

- **`yavs_cache::huge_heap()` runs first in `main`**: `mallopt` sets the top pad to 64 MiB (and the trim threshold to twice
  that, the mapping threshold to 32 MiB, glibc's own upper limit, because setting any of them turns off the allocator's
  tuning of that threshold), a 1 MiB block is allocated and freed so that the allocator extends the `brk` heap by one
  64 MiB step now, and that range (from the first 2 MiB boundary) gets `madvise(MADV_HUGEPAGE)`. Only `mallopt`, `sbrk(0)`
  and `madvise` (no new dependency, same `unsafe extern "C"` style as `trim_heap`). The kernel needs no setting beyond
  transparent huge pages in `madvise` mode (the default of common distributions); the heap is the same glibc heap, only the
  page size of its first 64 MiB differs, so #437 (no allocator replacement) stands.
- **Harmless where it does not apply**: compiled only for Linux with glibc (Windows, macOS, musl: no-op); it does nothing
  when `/sys/kernel/mm/transparent_hugepage/enabled` does not show `[madvise]` (`never`: nothing to ask for; `always`: the
  kernel already does it); a failed call changes nothing. `YAVS_HUGE_HEAP=0` switches it off (`docs/cli-contract.md` 7).
- **Why 64 MiB**: it covers the whole heap of a typical elaboration (median peak RSS of the realbench cells is 35 MiB). 256 MiB
  was tried on the heaviest interpreted run (`neorv32_dual_full`): 83% fewer page faults again, cycles unchanged, RSS up.
  Memory beyond the first step is ordinary 4 KiB pages as before (the kernel does not merge a later `brk` extension into the
  advised range), and so are the arenas of other threads (`--threads N`; a spot check of `--threads 4` shows no change in
  cycles). An allocator that asked for huge pages on every growth step (the tunable) would cover them; not worth a re-exec.
- **Cost**: peak RSS rises by the huge page granularity: median +2 to 4 MiB (+8% geometric mean, 46 to 49 MiB for a cold
  elab), at most +12 MiB on cells that run the interpreter and +39 MiB (4%) on runs that compile with LLVM (about 900 MiB). Zeroing a huge
  page is part of the first touch, so a very small run (`--version`) pays one 2 MiB clear: not measurable in its cycles.
- **Test**: `crates/yavs-cache/tests/huge_heap.rs` (own `main`: the allocator's `brk` heap belongs to the main thread) checks
  the `hg` flag of the heap in `/proc/self/smaps`, with and without `YAVS_HUGE_HEAP=0`; it skips itself where huge pages are
  not in `madvise` mode.

Measured with `tools/realbench` `--compare` (41 tests, all phases, cold and warm, interleaved, cores 6 and 7, `perf stat`
instructions, cycles and page faults as medians of every timed run, 2 to 5 runs) and the `tools/bench` h2h designs (9 runs):
see docs/STATUS.md "Huge pages for the heap". No test or phase gets slower beyond noise.

## Work-counter perf tests, op census (yavs, yavs-interp, yavs-kernel, #579, 2026-10-23)

- **Decision**: perf regressions are guarded in `cargo test` by counters of work that do not depend on the
  host (`crates/yavs/tests/perf.rs`, `YAVS_COUNTERS=1`), each with a ceiling (a floor where fewer is the
  regression); timing stays for measurement (#576 shows it is noisy). Counters pin the slow path a fast path
  avoids (update path, quick/direct commits, callbacks of compiled code, cache hits) and the interpreter ops
  executed. Why: the perf rounds of #440, #520, #523, #541 and #565 had nothing but `perf stat` behind them.
- **Cost**: kernel counters are plain adds once per cycle or on a slow path; `ops` and `cb_*` exist in debug
  builds and with `op-census` only, so release builds (the benchmarked ones) pay nothing.
- **Census**: `op-census` (off by default) counts executed ops and adjacent pairs at the dispatch loop to choose
  fusions from data (idea from a study of another simulator's opcode census).

## Stuck and hung simulations (#582)

- **E3002 causes without recording**: the loop is found when the error is made, from the drivers
  (`DriverInfo::owner`) and the live waiters: from each process of the last delta, follow driver to
  waiter until it returns (shortest cycle); processes with a timeout due now are `wait for 0 ns`
  loops. No history is kept, so the run pays nothing (`yavs-kernel/src/kernel/hang.rs`). User
  request, 2026-10-23.
- **Hang report on demand and opt-in at the end**: SIGUSR1 (Unix) pauses the kernel through the
  existing `Interrupt::pause` between two cycles from a signal thread; the run then reports and
  continues. At `--stop-time` or idle the report is printed only with `--hang-report`, because a
  stop time is a normal end of many testbenches (VUnit, OSVVM and UVVM end with `stop`/`finish`
  and a watchdog; plain testbenches end at a stop time or idle). An idle end with nothing but
  finished processes is clean and silent. The report is a warning (`W3101`) so JSON and `-W`
  control apply. The profiler's sampler uses no signal, so SIGUSR1 is ours.
- **Never woken** is a fixed point over pending transactions, timeouts, port edges and implicit
  signals: a waiting process with no timeout whose signals have no pending event and no live
  driver (transitively) cannot run again. Conservative: edge triggers and conditions are ignored.
- **Dead clock**: only signals an edge trigger (`Builder::edge_trigger`) watches, that had an
  event after time 0 and cannot change any more while a process waits for them. A `wait until`
  on several signals registers no trigger, so such a clock is reported as an ordinary stuck
  signal.
- **Process without a wait** is `W3102` (a warning: the LRM allows it, and `stop`/`finish` can end
  it). The silent hang itself (#583) needs a check on the interpreter loop and is not fixed.
- **Process places** in diagnostics are the first statement of the body (`ProcTable`, built before
  the elaborated model is freed); the place of the wait statement a process is suspended at
  would need the interpreter to expose its resume point and is not reported.

## Sampling profiler: `yavs run --profile[=FILE]` (yavs CLI, yavs-kernel `profile.rs`, yavs-interp, #580, 2026-10-23)

- **Decision**: a thread of the CLI samples 1000 times a second and tallies (a) the process each simulation thread runs, (b) the
  subprogram the interpreter or compiled code is in (`Runtime::sampler`, the slot the tier-up sampler already reads) and (c) the IR pc
  at which the interpreter's watching loop last looked (`Runtime::line_sampler`), resolved through the optimised function's statement
  table to file, line and the owning (possibly inlined) subprogram. Tables on stderr, all rows as JSON with FILE; stdout and exit status
  unchanged. Rationale: "which process, which line" was only answerable with a machine-level profiler; this is the first tool, `perf`
  the second.
- **Zero cost off**: a process is wrapped (`Profile::wrap`, one relaxed store of its number before `Process::run` and 0 after) only when
  a profile is installed (`Profile::install`, a global read once per kernel build). A `Config` field was tried first and moved the layout
  of the hot `SimControl`: a long delta chain (`rtl_chain`) ran 7% more cycles with identical instructions (#574-style layout effect).
  With `--tier=interp` a tier with no compiled code is installed so that the interpreter publishes its function; the only new work
  with the option off is the pc store in `watch_tick`, which runs once per 128 loop iterations or calls and only in a tier-less or
  tier-waiting run.
- **Limits, by design**: lines exist for interpreted code only (compiled code has no per-statement store, and one would cost every
  statement); the pc is where the interpreter last looked (every 128 events), not an exact sample; processes and subprograms are
  separate tables (a sample cannot read both atomically), paired in the JSON only when exactly one thread runs one process; Ctrl-C
  prints the profile so far on the first Ctrl-C/SIGTERM (user, 2026-10-23, #587); `--dap` is `E0125` (a stop would be sampled as time in the stopped process). No signal or timer is
  used, so it cannot clash with a signal-driven report.
- **Cost on** (tools/bench h2h designs, one performance core, geometric mean of total CPU, B with the option / B without): 1.03 with `--tier=interp`, 1.015 with `--tier=auto` (worst design 1.10, `rtl_fan`: many tiny activations)..

## `--tier=auto`: search loops stay interpreted, the clocked-process band is 160 to 300 instructions (yavs-llvm, #573, #575, 2026-10-23)

Found with the policy tuning of #572. Measured with benchmark builds (`tools/build-bench.sh --features llvm`, both sides the
same flags), core 6, `perf stat --no-inherit` cycles and instructions, medians and minima of 11 to 15 interleaved runs, a
fresh world per binary and tier, warm.

- **A function whose loops are all search loops stays interpreted** (`auto.rs` `fused_search`): every loop reads one array
  position per iteration (`Pos`, `Index`, `Load`), compares it and steps the index, with no other work. The interpreter runs
  that loop as one fused op (`pre::SCAN`, `scan_loop`: about 1.5 cycles an element), compiled it takes 3 to 4 cycles and
  14 instructions an element (the position check against the descriptor and the object lookup of every access). `uvvm_sb_slv`
  spends half of its interpreted time in one such function (`pos_of_leftmost`, a search for a line feed in log text): compiled,
  that function was the whole of the +5 to +12 percent of #573 (its time in compiled code is invisible to the sampler, which
  counts it for the caller). Excluding it alone gives 1.009 of the interpreter's cycles (1.093 before), excluding the
  other three helpers that compile as well changes nothing measurable. The test is the shape of the IR loop (`Pos`,
  `Index`, `Load` in equal numbers, at most one `Add`/`Sub`, at most two compares, branches and constants), not the
  interpreter's own test of its ops (`ponytail:`: a loop the interpreter does not fuse that has this shape stays
  interpreted too, and a fused loop with a check or a statement in its body is compiled). Only a sampled function is
  refused: a callee compiled with its caller is still compiled (one that leaves compiled code costs more).
- **A clocked process (`rtl`, 120 to 600 IR instructions) is compiled only from 160 to 300** (`RTL_BAND`, `outside_band`,
  an explicit refusal with a note; `cold` still applies to the whole class) (#575). The interpreter's clocked-process path got
  faster (kernel rounds 13 and 14, #440) and the compiled path did not: a process of 136 instructions (`rtl_ff`) is 8 to 14
  percent slower in cycles compiled (1.08 to 1.14 of the interpreter's minimum, 4 percent more instructions) and 2 to 4
  percent slower left interpreted (the watching loop's bookkeeping, #544); the same at 64, 256 and 1024 instances (the
  body, not the instance count), and at ten times the length. Bodies of 200 to 280 instructions gain (`rtl_lfsr`, 206:
  0.94 to 0.98 of the cycles; a variant of `rtl_ff` with 4 more terms, 228: 0.95 to 0.96; with 6 more terms, 274: 0.91
  to 1.0), and bodies of 320 and 458 instructions lose 12 to 20 percent in cycles at 7 to 20 percent fewer instructions (the
  compiled code runs at a lower IPC: its size, #532); interpreted they are level (0.98 to 1.07). Without the explicit
  refusal a process outside the band is an ordinary function for the policy and is compiled after the budget allows it
  (seconds into the run, and for a longer run just the same), which is why the first version of this change (a narrower
  `rtl`) was wrong: it also took the processes outside the band out of `cold` (`poc_arith_addw`'s `_p6`, `_p9`, `_p33`
  compiled). Not compiled in the realbench designs of the clocked class before or after (neorv32, `poc_cache_cpu`, the
  fletcher and satcat5 designs: same compile sets, `--verbose` notes compared before and after on ten tests); the change is
  seen in the benchmark designs; `poc_arith_addw` is unchanged (main-thread instructions warm 384.4 G against 384.5 G). `lfsr64` (132 instructions, 64 instances) is refused too and stays level with the interpreter within the noise
  of the measurement (0.99 to 1.04). Length stands for the work of an activation, which it does not: `rtl_ff` and `lfsr64`
  have the same length and a different result (scalar bit operations on a long vector against 64-bit logic vector
  operations); a cost model of the activation, or a check of the progress before and after compiling that takes
  compiled code back (the same check would cover #573), is the better rule and is not built (`ponytail:`).
- **Natives of frame-mode code count as leaving compiled code, as before** (the question of #565): a `CallNative` that
  `frame_direct` runs through `yavs_rt_native` is still not `is_inline` for `callbacks`, `too_many_callbacks` and `compute`.
  `poc_arith_addw` does not exercise the rule: with such natives counted as staying, the two clocked processes it refused
  for it (`_p11` and `_p36`, 25 and 23 of 221 and 210 instructions leaving) are refused by the instance rule instead (made 8
  times), and the main thread's instructions are identical to four digits (373.29 G against 373.27 G). The rule stays as
  it is; the question is open until a design with such a process that passes `cold` exists.

## Compiled code of queue and pointer-table libraries: a table of 256 objects, callees never seen, fat roots in place (yavs-llvm, #493, #520, 2026-10-23)

Found with call counts and `perf` on the window-generator testbench of the real-world suite (`vai_window_gen_interlock`,
warm `--tier=auto`): 164 million protected-method calls in a 16 s run (the string pointer table's `get` 70 million, `set`
40 million, `length`, `unsafe_push`, `unsafe_pop` 13 million each), about 290 host instructions and 130 cycles per `get`.
The profile before this change: 95 million `Deref` callbacks, 38 million `yavs_rt_host`, 36 million object look-ups
through `TierHost::object` (22 percent of the main thread's instructions in the callbacks), against 12 / 15 / 5 million after.

- **`Mem::objs` has 256 entries (was 16)**, still direct-mapped on the low bits of the object index with the shared flag
  flipped in at bit 3 (`obj_at`). The pointer tables of the test runner's queues keep one data object per queue next to
  the tables' own objects, a protected object of each kind and the buffers: 16 entries missed three heap objects in four.
  32 and 64 entries (also tried before, with "no change" on the microbenchmarks, which have few live objects) are in
  between: 64 entries 27 million `Deref` callbacks, 256 12 million (remaining misses are objects dropped after a
  deallocation). `Mem::occ` marks the entries in use and `retain`, `save_objs` and `begin` walk only those, so the
  bigger table does not make the walks longer; `Mem` grows by 10 KB per simulation thread. The key is computed in three
  places (`obj_at`, `handle_at`, `Gen::obj_entry` for addresses and for access values); they share the constants
  `KEY_FLAG`, `KEY_SHIFT_ADDR`, `KEY_SHIFT_HANDLE` and the test `obj_keys_agree` checks the Rust pair. (A key that
  disagrees between the two sides loses nothing but speed: while changing the size I saw 565 million callbacks and
  1.5 billion from exactly that.)
  Tag: LLVM-only (the table is the generator's cache in front of the heap's own look-up; a Cranelift tier would need
  the same cache or a flat view of the heap slots in `TierFrame`; the view was built and measured in #655 and loses on
  instructions, see "A flat view of the heap slots for compiled `Deref`" at the end).
- **A callee of a hot function that was never seen running is left out of its compile** (`left_out`, `Worker::unseen`,
  `Worker::left`; with a budget, more than `UNSEEN_MIN` = 32 IR instructions). `Worker::tree` brought the whole closure
  of native callees along (and the callees of a process), so a 27-instruction `pop` priced at 330 to 490 ms because its
  closure held the string splitting and counting helpers of the error messages (`split`, `count`, `resize`,
  `check_external`, 170 to 200 IR instructions each, never executed): a fifth of the compile time, and the budget (which
  binds in the first seconds of a cold run, 31 percent of the sampled time at 3.4 s) went to code that never ran. The
  compiled caller's native-frame code is published without them (`publish`); a call to one leaves the native frame to
  the interpreter, as a missing callee always did, and the callee compiles on its own once it is hot. The assumption
  is that a callee not sampled while its caller was seen for twice its compile time is called rarely from now on; if it
  is not, each call costs one materialisation until the callee's own threshold. Not applied without a budget (eager
  tiers, where nothing is sampled). `YAVS_AUTO_TRACE=1` refusals now name the number and size of the functions of the
  tree and the biggest four. Tag: shared (the policy is independent of the code generator).
- **Not done: the frame-mode part of a loop function compiled only when needed** (open question (b) of #520): in the cold
  trace the second compile of the functions with a loop is 9.4 percent of all compile time (357 of 3801 ms), so the
  upper bound of the gain is under 2 percent of the run, against the risk that a call in flight when the native code
  appears has no frame-mode loop to swap in.
- **A `TempRoot` of a fat pair reads both words in place** (`Gen::host` of 16 bytes, then the heap tags test of the two
  words) and calls `yavs_rt_fast` (`FAST_FAT_ROOT`) only where they cannot be reached inline: 22 million callbacks in the
  window generator, 1 percent of its instructions. LLVM-only.

## Compiled clocked RTL processes, round 4: the exits, the cold paths, the frame's layouts (yavs-llvm, yavs-interp, yavs-kernel, yavs-ir, #532, #339, 2026-10-23)

Compiled clocked RTL ran about the interpreter's instructions in more cycles (`neorv32_single_min` and
`fletcher_bufferwriter_512` under `--tier=llvm`: IPC 1.4 to 1.8 against 2.2 to 2.4, branch misses 3 to 4 times, L1
instruction-cache misses 3 times, instruction TLB misses 30 times). Two causes, found with `perf` and the census
(the census, below): every exit from compiled code to the host is 400 to 600 instructions with cold branches on
both sides (14 million of them on the CPU design, 64 million on the wide-vector one), and the code of a process
interleaved its rare paths with the hot ones. Decisions, each tagged by what a second code generator shares:

- **`YAVS_COUNTERS=kinds` (or `=pc`)** counts the calls compiled code makes into `Host::step`, `signal`, `drive_block`,
  `drive_wide`, `wait_suspend` and the refusals of `wait_same`, by IR instruction kind (with `pc`: function and pc), and
  `yavs run` prints them as `counters: cb_<entry>.<Kind>[.<function>.pc<N>]=N` with the other work counters. The tool for
  choosing the next exit and the test of a fast path (a count that stays out). Shared (the host side, `tier_census.rs`).
  Until 2026-10-23 it was a second mechanism, `YAVS_TIER_CENSUS` with its own `tier-census:` lines; the maintainer had it
  merged into the `YAVS_COUNTERS` counters (`cb_*`, per callback kind in `yavs-llvm`), so there is one. What it keeps that
  the totals do not have: the split by instruction kind and pc, the refusal reasons of `wait_same`, and every build (the
  totals are counted in debug and `op-census` builds only).
- **`rising_edge`, `falling_edge` and `'event` are answered in compiled code** from the kernel's event stamps (the cycle
  of the last event of each element, signals of at most 64 elements) and the plane block of `'last_value`
  (`TierSig::stamps`, `last`, `TierFrame::cycle`), after the known-edge test that stays for processes the kernel
  resumes for their edge. A process behind an asynchronous reset or written as `clk'event and clk = '1'` has no known
  edge and called the host for every test (7 million times on the CPU design). The forms `rising_edge (clk)`,
  `clk'event and clk = '1'` (and `'0'`), `wait until clk = '1'` and `wait until rising_edge (clk)` all end in these two
  primitives, so they take the same fast path, and the look-alikes (`rising_edge` is not `'event and = '1'`: it needs a
  `0` or `L` before the `1`) keep their meaning (`crates/yavs/tests/compiled_edges.rs`). Shared: the kernel accessors
  (`Signal::event_view`, `last_view`), the `TierSig` and `TierFrame` fields and the semantics; LLVM: `Gen::edge`,
  `Gen::sig_event`.
- **Signal reads without a call**: `SigReadScalar` of a signal that keeps no codes reads the kernel's plane blocks
  (`TierSig::planes`) or, for an enumeration, integer, boolean, real or time signal, its bytes (`TierSig::bytes`, `es`; a
  four-byte element sign-extended as `cell` does); `SigRead` of any length builds the planes from the kernel's blocks with
  the pure helper `yavs_rt_planes` where the codes are not there (wider than 64 elements, or a bus). Shared: the
  accessors and fields; LLVM: `sig_read_scalar`, `sig_read_planes`.
- **`Drive` of one waveform element**: `drive_now` takes 1 to 64 logic elements (`Ctx::assign_planes_now`, before only one
  element) and one scalar of a byte signal (`Ctx::assign_bytes_now`; the waveform element is 16 bytes there, bit 1 of the
  flags of `yavs_rt_drive`), and more than 64 elements go to `TierHost::drive_wide` (bit 2) instead of the generic
  `signal` fallback. Shared: the two kernel entry points and the host method; LLVM: `drive_block`.
- **`LEq` of 1 to 64 elements** is the four planes under a mask, **`LFill`** of a run inside one plane block four masked
  stores. LLVM only (a second generator would lower the same way or keep stepping).
- **A wait of several ranges is named once** (kernel): `Ctx::wait_on_again` answers true for the next range of the last
  wait in order (`ProcState::matched`), `again` once all are named, so a process that waits on `a, b, c` registers nothing
  after its first activation, in the interpreter and from compiled code (one callback per name before: 2.9 million of 14
  on the CPU design). A wait that ends after a part, or names another range, takes the part from `sens_prev` ahead of
  the rest (`Ctx::wait_on`, `commit_rest`). Shared.
- **`static_wait` accepts the split sensitivity lists** (shared, `pre.rs`): the instance-constant pass copies the block that
  ends in a sensitivity list's wait (one copy stores the signal references, one loads them), so the process had two
  suspension points and none of its waits was skipped (`ctrl_comb_proc` of the wide-vector design: 15 `WaitOn`
  callbacks per activation). A process with a sensitivity list waits for the same list at every suspension point; the
  copy that qualifies stands for them (`compiled_waits.rs`).
- **The layout of a record is kept in the frame** (shared IR, `opt/layout_cache.rs`, `Inst::LayoutOf::cache`): a
  `LayoutOf` of a record or of an array whose element layout the descriptor decides is a walk over the descriptor, and
  compiled code stepped it (32 million times in the wide-vector design: every field access of a record whose arrays have
  generic widths). The process's frame gets one slot per such type (flag, descriptor bytes, layout words); a tier that
  keeps layouts compares the descriptor bytes with the slot's copy at each use and answers with the slot's address, else
  makes the layout and keeps it. Nothing is assumed about the descriptor (an earlier idea, treating the descriptor
  pointer loaded from a frame slot as instance-constant, needs a contract the IR does not have: descriptors of heap
  objects are memory like any other). The interpreter ignores the slot (its per-type memo and the `FieldOf` fusion do
  the same); compiled code hits without a call (`layout_keep` fills it after the interpreter's step).
  Under `--tier=auto` too since 2026-10-23 ("The layout cache under `--tier=auto`", at the end).
- **`TempRoot` of a fat pair and a `Stmt` with recorded roots use the host's light functions in frame-mode code too**
  (`yavs_rt_troot`, `yavs_rt_endstmt`, as native frames do), not the generic `signal` step. LLVM.
- **The rare paths are out of line** (LLVM only): the slow path of every inline fast path (`slow_path`) and the failed
  lookup of `Gen::host` are in `Cur::cold`, behind the rest of the function, and their callbacks carry the `cold`
  attribute, so the optimising code generator places them away too; the hot path falls through. Branch misses on the CPU
  design fell by 25 to 30 percent from this alone, the instructions are unchanged.
- **A constant `SigVal` is two deferred registers** (LLVM only): the index of the instance signal and the offset are
  known, so the pair is a constant to every later guard and the instruction leaves to the interpreter (`exit_if`) in the
  one case that fails, instead of a stored pair and a guard with a slow path.

- **The code of a whole-program module is in huge pages** (LLVM, Linux; `huge_code`): `--tier=llvm` links megabytes of
  code that every clock cycle runs through, a page of 4 KiB each in the instruction TLB; `madvise (MADV_HUGEPAGE,
  MADV_COLLAPSE)` on the 2 MiB-aligned part of the code makes huge pages at once (a failure changes nothing,
  `YAVS_LLVM_HUGE=0` leaves it out). The CPU design: instruction TLB misses 52 M to 9 M, cycles -6 percent, instructions
  equal. Per-function modules of `--tier=auto` are small and scattered and are left as they are.

Not changed, for the policy owner: `lower::is_inline` still counts the signal operations among the instructions that
leave compiled code (the `too_many_callbacks` rule of `--tier=auto`), and the rules of a clocked process made few times or
a small part of the run (`RTL_INSTANCES`, `RTL_SHARE`) are as before. Measured and not built: `optsize` on the process
functions (no change in size or cycles), the small code model with position-independent code (17 percent fewer branch
misses, cycles equal).

Ideas taken from the reference simulator (sim1; read for the design, no code taken): its `'event` and `rising_edge` are
a flag of the signal the runtime keeps up to date and the compiled code tests, not a call (ours: the event stamp and
the last-value planes, tested in place); and a process there is compiled once per architecture and takes its
instance by pointer, which is what our one function per process of a design unit already is.

## textio and formal packages, 2026-10-23 (#622, #623; user: follow the LRM, else accept what a major simulator accepts)

- **READ of a bit_vector skips white space before each bit** (L131, #623): the LRM (16.4) is silent
  on white space inside the characters, the references split (sim1 accepts, sim2 rejects), so the
  permissive reading holds. A failed READ still leaves the line and the value as they were.
- **A formal package with a generic map still needs an actual** (L132, #622): an interface
  package has no default (6.5.6.2, 6.5.7.2), so an unassociated one stays `E1502`; the formal's
  generic map only constrains the actual instance (`E1503`). Not implemented: instantiating it
  implicitly, as sim2 does.

## 2019 `INTEGER` is the full 64-bit range (lib/std, #601, 2026-10-23)

- **`INTEGER'LOW` under `--std=2019` is -9223372036854775808** (agent, on the maintainer's instruction to fix #601). The 2019
  `STANDARD` had the symmetric range -(2^63-1) to 2^63-1, which the LRM demands as a minimum; the LRM allows any range that
  contains it, 2008 `INTEGER` already has the extra negative value (-2^31), `TIME` has -2^63 fs, and the reference simulator
  that runs 2019 as 64-bit has -2^63. The other reference stays at 32 bits (`integer-low-2019`). The edit is in
  `tools/derive_libs.py` and `lib/std/2019/standard.vhdl` (`lib/SOURCES.md`); arithmetic is checked as before, so
  `integer'low - 1` and `abs integer'low` are `E3009`.

## The layout cache under `--tier=auto` (yavs-ir `opt::layout_slots`, yavs, #532, 2026-10-23)

- **The frame slots of `opt/layout_cache.rs` are made under `--tier=auto` as well as `--tier=llvm`** (user, 2026-10-23, despite
  the peak memory it costs: the slots are per process instance, so a design of thousands of instances pays them in every
  run). `yavs run` calls `yavs_interp::layout_slots()` for the tier (`--tier=interp` has none); the switch is part of the
  optimiser fingerprint (`layouts`), so cache entries of the interpreter and of `auto` never meet. Optimised copies stay
  portable between runs of the same configuration.
- **Measured** (bench builds with the `llvm` feature of main before and after, `tools/realbench/run.py --compare`, phase
  `run`, pinned to cores 10 and 11, median of 3 to 7; peak RSS in MiB largest process, before / after; G = billions of
  instructions of the whole process, compile thread included). "default" is `--tier=auto` as it ships; "compile at once" is
  `--tier=auto --tier-heat=1,0,0`, which compiles every process as soon as it is seen and is the case `--tier=llvm` shows:

| test | default: RSS | default: G instr | compile at once: RSS | compile at once: G instr |
|---|---|---|---|---|
| `hdlm_axi_fifo` | 50 / 51 | 0.57 / 0.61 | 123 / 122 | 8.97 / 9.05 |
| `olo_axi_master_full` | 54 / 55 | 1.02 / 1.01 | 219 / 261 | 22.42 / 23.83 |
| `vunit_axi_dma` | 51 / 51 | 0.53 / 0.53 | 122 / 123 | 7.28 / 7.40 |
| `poc_cache_cpu` | 37 / 36 | 25.83 / 25.83 | 107 / 107 | 25.71 / 25.72 |
| `fletcher_bufferwriter_512` | 40 / 40 | 112.93 / 112.92 | 151 / 151 | 121.01 / 108.53 |
| `neorv32_single_min` | 76 / 76 | 66.20 / 66.19 | 224 / 225 | 87.35 / 87.54 |
| `hdlm_axi_crossbar` | 51 / 51 | 0.56 / 0.56 | 121 / 121 | 8.09 / 8.07 |
| `vai_window_gen_interlock` | 136 / 136 | 232.69 / 231.44 | 162 / 163 | 462.62 / 462.62 |

  In the default policy the change is invisible: processes get hot late and few are compiled, so the slots of the others
  stay in zeroed pages that are never touched (peak RSS within 1 MiB everywhere, instructions within the spread of a run).
  Where compilation is eager the slots are filled: `fletcher_bufferwriter_512` runs 10% fewer instructions (the 32 million
  `LayoutOf` calls compiled code made), `olo_axi_master_full` needs 42 MiB more (+19%) and 7% more instructions for no gain
  (many small record-heavy processes, each with its slots filled and checked once or twice), the rest does not move.
  The earlier "40 to 180 MB" of the fletcher testbench under `--tier=llvm` is the compile-everything cost (150 MiB here before
  and after), not the slots'.
- **Work-counter bounds did not move** (`cargo test -p yavs --features llvm --test perf`: the counters are measured under
  `--tier=interp` and `--tier=llvm`, where nothing changed); there is no deterministic counter for `auto` (what it compiles
  depends on the timing of the compile thread), so the cost above is not gated. `compiled_natives` checks the output of a
  record-heavy design under `--tier=auto --tier-heat=1,0,0`.

## Compiled protected-method accessors, round 2: closed frames (yavs-llvm, #626, 2026-10-23)

Measured on `protected_queue` (`tools/bench`, `--tier=llvm` with the `LEAN` pipeline, so no inlining as in `--tier=auto`,
main-thread instructions, deterministic) and a micro design calling one protected `get` a million times: a `get` of the
test runner's pointer tables was about 450 instructions and 130 cycles, of which 35 in the prologue of the native frame.
The study of the reference simulator's own optimiser gave the idea (it promotes stack allocations whose address is only
used by direct loads and stores to registers); ours differs in that a native frame is also found by its frame index.

- **A closed frame is not registered** (`closed_frame`, `Nat::closed`, tag LLVM-only; the analysis is shared in nature,
  a second code generator needs the same registration rule): a native frame is entered in `Mem::nslots` so callees and
  the interpreter's natives can reach its slots by the frame index in an address. When no address of the frame can leave
  the function (no function nested in it, every `AddrOf Frame` defined once and used only as the address of a `Load` or
  `Store`: never data, never an operand of a call, `Index`, `Copy`, a native or a stepped instruction) nothing can ask, so
  the slots are an `alloca` LLVM promotes (`sroa`): the zeroing, the parameter stores and the reloads of the prologue go,
  and the unknown-address lookup of such a function leaves out the test of its own frame (the dynamic offset kept the
  `alloca` in memory). Its `nslots` entry is written with length 0 instead, so a stale entry of an earlier frame at that
  depth can never be read. The bytes the exit sites hand to a materialised frame (`yavs_rt_deopt_slots`, already zero)
  are only the ranges the function touches, and so is the zeroing at entry. A native frame also knows its preloaded
  `AddrOf` registers in every block now (frame mode already did): a load through one in a later block is a direct slot
  access instead of a lookup. Test: `closed_frames_hand_their_slots_to_the_interpreter` (a recursion past the native
  stack allowance materialises the frames mid-way).
- Measured (instructions of the main thread, `protected_queue` / micro `get`): 8.776 G to 8.143 G (-7.2 percent), 594 to
  555 instructions per `get` of the micro design.
- **A heap object a `Deref` found stays in hand in a native frame** (`Known::Heap(Some(ObjRef))`, `Cur::objnew`, tag
  LLVM-only; a second code generator needs the same rule): the generated code keeps the object's pointer and length (read
  from its entry of `Mem::objs` after the `Deref`) and the offset of each address register derived from it by `Index`,
  `Field` or `Move`, and `Gen::host` reads and writes through them with only the bounds test of the access: no key
  hash, no entry compare, no second lookup. The facts live up to the next instruction outside `keeps_objects` (the rule the
  object table's own entries follow across steps), a call included, and to the end of the basic block; any redefinition of
  the register drops its fact. Every such object is writable: a `Deref` hit requires the entry's write flag (an entry
  found for reading goes through the callback, which enters the object writable) and a callback that found the address
  but did not enter the object leaves the frame (checked, cold). A stale handle is the same slow-path error as before
  (the access value must match the entry). Frame-mode code keeps the old form (its slow path steps the instruction and
  goes on, so the entry is not known to hold the object afterwards). Test: `heap_object_held_between_instructions`.
- **`Index` guards**: the 4 GiB checks were a multiply and an add with overflow flags and a compare; with a position
  under 2^32 and a stride under 2^32 neither wraps, so `(pos | sum) >> 32 != 0` is the whole test (a stride of 0 tests the
  sum only): identical failures, four instructions fewer per `Index`. **`Pos`**: the range test is `lo <= hi` and
  `(i - lo) <= (hi - lo)` unsigned with `lo`, `hi` picked by the direction word, instead of two signed range tests
  and a select (12 instructions fewer; shared by the scan loop work, #578).
- A closed frame must be decided on every operand: `Inst::visit` leaves out the operand lists of calls, natives and
  concatenations (`yavs_interp::inst_regs` has them). The first version missed `CallNative`'s argument list and the corpus
  caught it (`textio/std_logic_textio`, a frame address passed to a native `READ` helper).

## Compiled clocked RTL processes, round 5: the entry cost of a small process (yavs-interp, yavs-llvm, #634, 2026-10-23)

Compiled clocked RTL took more cycles than the interpreter (#634). The first measurement of this round
(`rtl_ff`, 4096 one-line processes, exact instruction trace of one activation: 1231 instructions, 325 of them the
compiled body) showed that for small processes the plumbing around the body is the bigger part, and that
two of the leanest paths were not taken. Decisions, tagged shared (S, any code generator) or LLVM only (L):

- **A resident process stays resident when it suspends compiled** (S, `Machine::run_compiled`): the downgrade at
  every `TIER_SUSPEND` handed the registers and the temporary area to the thread's scratch (a thread-local borrow
  and two vector swaps per activation, about 120 instructions). The compiled body never reads the register file in
  its fast paths, so the cache argument for the scratch does not apply to registers; a process whose temporary
  area is the interpreter's too stays as small as before (`RESIDENT_TEMP`). Cost: the first `Alloc` of each resident
  process grows its temporary area through the step callback (`cb_step` of `llvm_clocked` 6 to 25, once per process).
- **The wait of one signal short-cuts too in compiled code, and in every copy of a wait** (S for the analysis,
  L for the use; `pre::static_wait(prog, f, min)`): `static_wait` required two `WaitOn`s (the interpreter's fused
  wait is cheap for one) and found only the first copy of a wait. A process of a concurrent assignment or with a
  sensitivity list that names array elements has two copies of its wait (the lowering makes the first activation's copy
  keep the references), and every activation after the first runs the second, which never short-cut. Compiled code
  now asks for `min` 1 and for all copies (`IMPLICIT` processes with no call that can suspend count as a list, as
  `SENS_LIST` ones do), and calls `yavs_rt_wait_same` (about 50 instructions) instead of `yavs_rt_wait`,
  `wait_again` and `wait_on_again` (about 140). The interpreter keeps `min` 2 and gains the later copies.
- **`Mem::begin`'s common case is inline** and a new `Mem` is made out of line (L): `enter` had a 20 KB frame with
  five stack probes in every call, and a six-register prologue for a function that returns at its first test.

Measured (bench builds, instructions of the simulation phase, main thread, `--tier=auto --tier-heat=1,0,0` warm):
`rtl_ff` 12.17 G to 10.32 G (interpreter 11.57 G), `rtl_chain` 3.93 G to 3.49 G (interpreter 3.20 G), `rtl_lfsr` 5.09 G
to 4.56 G; cycles on core 12 (median of 3 interleaved runs): `rtl_ff` 3.31 G to 3.02 G (interpreter 3.34 G).

- **A native frame's block with a single way in takes what its predecessor knew at the jump** (the known registers and held
  objects, `inherit` in `Gen::func`; the values defined in the predecessor dominate such a block). The arms of a `Case` of
  a protected method (one block each) found their objects again with a full lookup each; now they are held from the
  `Deref`s before the `Case`. Measured: one protected `get` 397 to 359 instructions, `protected_queue` 6.17 G to 5.88 G.
- **Block placement is on, and a guard that holds says so** (L, `jit.rs` `CODEGEN_ARGS`, `Gen::cond_br`/`weights`):
  `-disable-block-placement` was one of the switches that cut the code generator's time (#432); with it the
  hot path of a compiled process jumped over its own rare paths 14 to 26 times per activation (`rtl_ff`: 26 taken
  branches in 316 instructions) and the front end paid for each (branch misses and fetch redirects: compiled code
  runs at an IPC of 1.4 to 1.7 against the interpreter's 2.5). Every `guard`, `exit_if`, host-call result test and the
  signal-read alternatives carry branch weights (2000 to 1) for the common edge, so the placement makes the common
  edge the fall-through: 6 taken branches in 310 instructions. `--tier=auto` (per-function `O2` modules)
  on `fletcher_bufferreader`, `fletcher_bufferwriter_512` and `neorv32_single_min`: branch misses -20 to -25
  percent, cycles -5 to -9 percent (minimum of 5 interleaved runs on core 12/13, bench builds); compile time of the
  761 `O2` modules of `neorv32_single_min` unchanged (codegen 12.79 s against 12.81 s). `--tier=llvm` compiles
  its large module with the unoptimised code generator, which has no block placement: unchanged. Shared:
  nothing (a second generator lays out its own blocks); the idea carries over (cold exits last).
- **A scalar logic assignment is one step in compiled code, at the sites the interpreter fuses** (S for the sites, L for the
  call; `yavs_interp::scalar_drives`, `TierHost::drive_bit_now`/`drive_bit`, `yavs_rt_drive_bit`, `Gen::drive_bit`): the
  lowering makes a waveform for every `sig <= v` (`Alloc` 40 bytes, `Store` the time, `Index`, `LSet`, `Drive`), the
  interpreter's predecoder runs the five as `Op::LSetDrive` and compiled code made the waveform, wrote the four
  planes and read them back in `yavs_rt_drive` (about 115 instructions, with the zeroing, the bounds checks and the
  flush of the live registers). `scalar_drives(prog, f)` returns the sites from the predecoder itself (so the interpreter,
  which continues where compiled code leaves, never sees a half-made waveform), the generator skips the four
  instructions and calls `yavs_rt_drive_bit(delay, code)` at the `Drive`; the host takes the kernel's fastest path for a
  zero delay (`assign_code_now`), else `assign_code` with the delay, else (several parts, not a logic scalar, a bad code)
  makes the waveform from the values and steps the instructions (`Host::drive_bit_replay`). The call is `quiet`: the live
  registers are written back only if it fails. `rtl_ff` 10.16 G to 9.06 G instructions, `rtl_chain` 3.48 G to 3.25 G
  (interpreter 3.20 G). Test: `crates/yavs/tests/compiled_drives.rs` (every shape of assignment, three tier settings).
- **Frame-mode code asks the runtime for an address of unknown space, once** (L, `Gen::host`): the inline chain (the
  object table entry, then the frame, the temporary area and the instance data, a branch site and 10 to 25 LLVM
  instructions each) was most of the code of a process (`LayoutOf` and `Load` were 19 and 12 percent of the LLVM
  instructions of the wide-vector design) and ran through branch sites the predictor had never seen; `yavs_rt_host`
  does the same lookup (`Mem::find`) in one hot function. Native frames keep the chain (the hot loops of library code).
  `--tier=auto`, cycles, minimum of 7 interleaved runs on core 12: `fletcher_bufferreader` 16.7 G to 15.6 G,
  `fletcher_bufferwriter_512` 48.0 G to 43.6 G, `neorv32_single_min` 24.5 G to 23.7 G; instructions +0 to +2.7 percent,
  LLVM instructions of the wide-vector design 1.15 M to 0.83 M (compile time follows). `--tier=llvm` code of a
  process is generated the same way, so its object is smaller too.
- **The kept-layout test of a long descriptor is one `memcmp`** (L, `LayoutOf` with a frame slot, descriptors over 32 bytes):
  a record of arrays has descriptors of 100+ bytes, and the word-by-word compare was 82 LLVM instructions per
  `LayoutOf` (1265 of them in the wide-vector design). `fletcher_bufferreader` another -5 percent cycles.
- **`SigRead` of a signal of enumeration, integer, boolean, real or time elements is inline** (L, `Gen::sig_read_planes`):
  the kernel's bytes (`TierSig::bytes`, `es`, as `Buf::write_to` copies them) are copied with one `memcpy` of
  `count * element size`; it was a 220-instruction callback (800 thousand in `fletcher_bufferreader`, one per
  integer field of a record read from signals). Instructions of that testbench -2.4 percent.
- **`yavs_rt_host` is a leaf on a hit** (L, #663): the view and object-table lookup is the function, the interpreter's
  object table (`yavs_rt_obj`) is a cold function behind it, so the common hit pushes and pops no callee-saved registers
  (7 pushes, 7 pops and their moves per call before). `hdlm_lfsr` under `--tier=auto`, warm, instructions: 32.7 G with
  the call (+5 percent over before the two #634 commits), 30.7 G now (before them 31.1 G); the other #634 designs are
  within noise (`hdlm_lfsr` spends it on the instance-data read of a loop over a table).

## Search loops in compiled code: one shared scan (yavs-interp `scan.rs`, yavs-llvm, #578, 2026-10-23)

A compiled loop that reads one array element per iteration (`Pos`, `Index`, `Load`, compare, index step) paid the
position check and the object lookup at every element: 95 instructions an element in a native frame (a benchmark of a
100-character search; the interpreter's fused loop about 60), against 14 to 5 the issue measured on real string helpers.

- **`yavs_interp::scan_bytes` is the one implementation** (tag: shared; the interpreter's `Machine::scan_loop` and the
  LLVM helper both call it; a second code generator calls the same function): the iterations of a search loop while neither
  exit is taken, over the descriptor's `(left, right, direction)` and the bytes of the object, stopping at anything the
  generic code should handle (an index outside the range, bytes outside the object, an overflow, a zero stride) so errors
  come at the same iteration as before. The position is kept as a count from the end of the range, so the range test is
  one unsigned compare and the offset a multiply-add; loops of their own for byte, 4-byte and 8-byte elements and for `=`
  and `/=` against the element; byte elements at stride 1 searched by `=` with `=` on the index and steps of one are a
  slice search (`window`: the iterations are a window of the object bounded by the range, the object, the limit and the
  index `e`, searched in one go). The unit test `scan_bytes_matches_the_generic_loop` compares 400 thousand random shapes,
  ranges, bytes and limits with the iteration-by-iteration loop (half of them the byte-search shape).
- **`yavs_interp::search_loops` finds the loop in the IR** (shared): a head block of exactly `Pos`, `Index`, `Load`,
  `Cmp`, `CondBr`, a block (jump-only blocks skipped, as the interpreter skips a `Br`) with the compare of the index and
  the exit, and a block `i := i +- constant` and the jump back, with the registers the iterations define besides the element
  and the index dead at both exits.
- **Compiled code calls `yavs_rt_scan`** at the head of such a loop in a native frame (LLVM-only glue: the descriptor and the
  array's object are looked up once, `Mem::span`); what it leaves (`hit` 0) is the generic code of the head, so a fault is
  the interpreter's at the same iteration. Frame-mode code (processes) keeps the generic loop. Test:
  `search_loops_match_the_interpreter` (both directions and steps, found, ended, outside the descriptor, outside the array).
- **Numbers** (instructions of the main thread, a 100-element search called 400 thousand times through a native frame):
  compiled generic loop 3.83 G, with the helper 0.43 G; interpreter 2.69 G before, 0.65 G after. Policy: the
  `--tier=auto` rule `fused_search` (#573) existed because the compiled loop was twice as slow as the interpreter's; with
  the helper a native-frame search loop is as fast, so the rule can go (the policy is `auto.rs`'s owner's to change, #572).

- **A `Deref` hit is one compare** (`Obj::h`, `NO_H`): an entry that has an access value is writable (`yavs_rt_obj` keeps the write
  permission of the entry it replaces for the same object) and no entry holds the null handle's value (the sentinel for "none" is
  1, not 0), so the load of the write flag and the test of the handle are gone: five instructions less per `Deref`, one protected
  `get` 359 to 337.
- **A native frame finds the interpreter's top frame inline** (`Gen::host`, unknown-address chain): the variables of the
  process or function that called into compiled code, reached by the by-reference parameters of protected methods and
  helpers, were a callback each (`yavs_rt_host`, 33 million of the window generator's 43 million, found with the callback
  counters of an `op-census` build: `YAVS_COUNTERS=1`, `cb_host`, by address space). The frame-mode form tested the view's
  frame; native frames now do after a miss of the object table. `cb_host` 43.5 M to 5.3 M, main-thread instructions of the
  window generator (warm `--tier=auto`) about -2.5 percent. A table of 1024 objects instead of 256 halves `cb_fast`
  (13.7 M to 6.8 M) but moves `cb_host` by 8 percent and was not kept.

## `--tier=auto`: the callback count sees what leaves compiled code, and compiled code is measured and taken back (yavs-llvm, yavs-interp, yavs-kernel, #633, #596, #575, #573, 2026-10-23)

Policy owner's change. The static rules of `auto.rs` stay as the first guess about what to compile; a measurement after each swap
corrects what they cannot see. Measured with benchmark builds (`tools/build-bench.sh --features llvm`, both sides the same flags),
core 11, `perf stat --no-inherit` main-thread cycles and instructions, medians of interleaved runs, warm compilation cache.

- **`lower::leaves` counts what really leaves compiled code (#633).** `is_inline` stays what it was (it decides which functions run on
  native frames and feeds the `llvm_inline_insts` counter); the policy (`callbacks`, `too_many_callbacks`, `compute`) uses `leaves`,
  which also takes as staying: `SigReadScalar`, `SigRead` of any length, `'event`, the clock-edge natives answered from the kernel's
  event stamps, the `WaitOn` that ends an activation and its `Suspend` (the way out of the activation, not a callback in the middle
  of it), and a call of a function that runs on native frames (a direct call, as `compute` and the function rule assumed already).
  Still leaving, by the `cb_*` counters of `--tier=llvm` runs (`YAVS_COUNTERS=1`, a build with `op-census`): `Drive` (`cb_drive`, one
  `yavs_rt_drive` per drive: `rtl_ff` 8.2 million for 8.2 million activations), `CallNative` through `yavs_rt_native` (`cb_native`,
  `rtl_ram` 120 000 for 100 000 activations), a wait in the middle (`cb_wait_on`), `WaitFor`, `DriveSlot`, reports, `Cat` of a
  logic vector whose lengths the generator does not know (counted always: the policy has no constant tracking), every stepped
  instruction. Evidence: the static count of the clocked processes of the benchmark designs fell from 13 to 2 (`rtl_ff`), 20 to 5
  (`rtl_lfsr`), 6 to 3 (`lfsr64`) leaving instructions of 135, 205 and 92 on a path (a union of all paths: `rtl_ff` has two
  drives on different branches, runs one: `cb_drive` per activation 1.0). The rule stays at one in `CALLBACKS` = 10; with the new
  count nearly every clocked process passes it (neorv32's hot processes 1 to 6 percent, `rtl_ram`'s testbench process 9, which
  `RTL_INSTANCES` refuses), so it is a guard against stepped-instruction-heavy bodies now, and the measurement below is the judge.
  Compile sets changed on the realbench designs: `poc_arith_addw` compiles `_p8` (341 IR) instead of `procChecker`, `sc5_fifo_repack`
  `p_col` and `p_in` (657 and 717 IR, processes with loops), `vai_window_gen_interlock` `check_axi_stream`: processes with signal
  reads that were refused for them.
- **A trial after the swap of a clocked process (`rtl`) seen for at least an eighth of the sampled time (#596)** (`Worker::trial`,
  `run_trial`, `decide`). The compile thread measures the process runs the kernel did per CPU second of the simulation threads (the
  process's CPU time less the compile thread's own, `CLOCK_PROCESS_CPUTIME_ID` less `CLOCK_THREAD_CPUTIME_ID`), in windows of at
  least 25 ms, with the compiled code (a warm-up window first), with the interpreter running the process (its entry, `NAT_CODE`,
  `NAT_CALL` and its native-frame table slot cleared; compiled callers find a null slot and leave to the interpreter, as for a
  function compiled later), with the compiled code again, up to three pairs. A first pair below 0.9 takes the code back and one at or
  above 1.15 keeps it; two pairs below 0.95 take it back, at or above 1.15 keep it; else a third pair, and the code stays when the
  run progressed at least 1.05 times as fast with it (at parity a coin flip would keep code that runs at a lower IPC: a 504 IR
  variant of `rtl_ff` kept at 1.03 ran 8 percent slower than the interpreter, 25 percent fewer instructions). A process taken back
  is `lost`: never compiled again, not as a callee either, and when nothing else is compiled the fast loop goes back to the one
  without the tier's bookkeeping (`NativeFlags::clear`). Cost: 75 to 175 ms of the compile thread's time per trial and the
  windows in the slower mode, about 40 ms of the run for code that is taken back. The progress is `yavs_kernel::progress()`, the
  process runs counted when each delta cycle starts: one relaxed store per cycle in the kernel (`rtl_chain`, 2 million cycles: 0.12
  percent more instructions; nothing measurable elsewhere). A run in which the windows cannot reach 1000 runs and 30 times the
  largest step the counter makes in a millisecond within 250 ms (no clock to speak of) is not measurable: its process stays compiled
  and no more are tried. Not tried with `--tier-heat` (budget 0) or the eager tier of the tests.
- **Only clocked processes are measured.** The counter is the process runs of the kernel, so it measures a function whose own activations
  are the progress. A procedure, or a process that loops between its waits, does its work in stretches the counter does not see: the
  checker of `poc_arith_addw` (a procedure, 594 IR, 15 s of a 35 s run in a few hundred thousand runs) got verdicts of 0.89 to 0.96
  for code that makes the whole run a fifth faster (main before this change: 0.81 of the interpreter's cycles), and a trial that
  takes it back is a regression. Those stay on the static rules; a count of the function's own entries in both modes (the
  interpreter would count them while a trial runs) is the way to measure them and is not built.
- **Clocked processes are compiled at the final level at once.** The class (`rtl`, 120 to 600 IR instructions) compiled at O0
  first and at O2 a few hundred milliseconds later; in trials O0 code was 15 to 25 percent slower than the same code at O2
  (`rtl_lfsr` 0.99 to 1.04 against 1.3 to 1.5, `lfsr64` 0.86 against 1.19, a 228 IR variant of `rtl_ff` 0.78 against 1.16,
  loaded host), so a verdict on O0 code would have judged the wrong code. Now `compile` uses the tier's level (O2, `Fast` from
  `FAST_MIN`) for them, and the budget prices them at 200 us an IR instruction (31 to 92 ms measured for 130 to 270, 28 to 34 ms
  with `Fast` for 360 to 500; was 30 us at O0).
- **`RTL_BAND` is a wait, no longer a refusal.** The 160 to 300 band of #575 (measured on a mix of O0 and O2 code) refused bodies
  that gain: `lfsr64` (132 IR) at ten times its length 0.89 of the interpreter's cycles compiled at once, 1.02 with the band.
  Length stands for the work of an activation, which it does not, so the trial judges. A trial that takes the code back costs the
  run about 40 ms (`rtl_ff`: +4 percent of its 0.75 s, +0 at ten times that), so outside the band a process is tried only
  after it has been seen for a second (`band_wait`, also for the look-up in the compilation cache). `RTL_MIN`, `RTL_MAX`,
  `RTL_INSTANCES` and `RTL_SHARE` stay: their effects (a few percent of a run in many processes) are below what a trial can see.
- **`fused_search` stays a static rule**, as do the size, share and callback rules for functions and looping processes: a trial cannot
  take them back (`uvvm_sb_slv` with the rule off: "not measurable" after 250 ms, the testbench has no clock to speak of). The
  scan helper of #578 does not make it obsolete: with the rule off `uvvm_sb_slv` ran 1.22 times the interpreter's cycles (1.03 with
  the rule), 28 percent more instructions.
- **Results (core 11, warm, cycles of the main thread against `--tier=interp` of the same build; B = main before this change,
  N = after).**

  | Design (warm, medians of 5 to 9) | main: auto / interp cycles | now: auto / interp cycles | instructions main / now |
  |---|---:|---:|---:|
  | `rtl_ff` (no gain by body) | 1.000 | 0.996 | 1.028 / 1.028 |
  | `rtl_ff` at 5 times the length | 1.029 | 0.932 (kept, 19 percent fewer cycles of compiled code) | 1.028 / 0.939 |
  | `rtl_lfsr` | 0.797 | 0.763 | 0.747 / 0.742 |
  | `rtl_chain`, `rtl_fan`, `rtl_bus`, `rtl_ram` | 0.986, 1.037, 1.015, 1.006 | 1.003, 1.026, 1.032, 1.007 | 1.023, 1.067, 1.010, 1.007 / same |
  | `lfsr64`, `lfsr64_xor` (0.19 and 0.09 s) | 0.992, 0.994 | 1.006, 1.001 | 1.018, 1.012 / same |
  | `lfsr64` at 10 times the length | 1.000 | 0.997 | 1.018 / 0.990 |
  | 366 and 504 IR variants of `rtl_ff` | 1.049, 1.045 | 1.033, 1.041 | 1.014, 1.011 / 1.014, 0.972 |
  | `sc5_fifo_repack` (processes of 657 and 717 IR with loops, refused by the callback count before) | 1.013 | 0.778 | 1.017 / 0.834 |
  | `hdlm_lfsr` | 0.521 | 0.504 | 0.510 / 0.510 |
  | `poc_arith_addw` | 0.780 | 0.790 | 0.765 / 0.768 |
  | `vai_window_gen_interlock` | 0.417 | 0.407 | 0.386 / 0.387 |
  | `neorv32_single_min`, `poc_cache_cpu`, `fletcher_bufferwriter_512`, `osvvm_axis_random` | 1.009, 0.998, 1.004, 1.007 | 1.000, 1.003, 1.000, 1.005 | within 1 percent |
  | `uvvm_sb_slv` (two runs) | 0.991, 1.034 | 1.053, 1.032 | 1.019 / 1.020 |

  The interpreter tier of the new build against the old one (the kernel's store, one per cycle): instructions equal except
  `rtl_chain` +0.1 percent; cycles within the spread of the runs (0.98 to 1.04, a different binary moves single designs by a
  few percent, #576). Taken back in the runs: `rtl_ff` (0.95 to 1.03 in single runs, a coin flip around parity), the 366 IR
  variant (0.86 to 0.98), mostly the 504 IR one; kept: `rtl_lfsr` (1.3 to 1.5), the 228 and 274 IR variants, `rtl_ff` at 5
  times the length. Cores: all timed runs on core 11 alone (the compile thread shares it), builds on 0-7 and 16-23.
- Not done: counting activations per function in the interpreter to compare time per activation (a cost on every run, the
  point of #596 was none); a trial of every swap (a function below an eighth of the run moves the progress less than the
  measure resolves, and a trial of each costs the compile thread its time); weighting the static count by which path runs.

## Specialised copies of processes (yavs-ir `copies.rs`, yavs-interp, yavs-elab, #651, user, 2026-10-23)

- **Decision (user, 2026-10-23: start per-instance specialisation now; reason 3 of the compiled-versus-interpreted gap):**
  option (a) of `docs/design/specialisation.md`: a process runs a copy of its code specialised to its instance's frozen data,
  one copy per distinct (process, data), as a yavs IR pass so every backend sees the folded code. The study's recommended
  defaults stand (lazy, `for` generate parameters not folded, growth cap 1.5 (2.5 since 2026-10-24), shared code under `--dap`, `--cov` and `--vhpi`
  variable access, a stage cache kind, copies compiled by the existing tier policy). Reported as the answers to its section 9.
- **Lazy means a switch at a suspension after `YAVS_SPEC_AFTER` (1024) activations**, not "optimise on first use": every
  process runs once in initialisation, so the first use of a copy is at time 0 (the prototype's +1 G instructions in the
  initialisation of a CPU design). One compare per activation is the cost for a process that does not switch.
  Only a process with one wait statement, no nested subprogram, no shared state, no coverage and an instance-data
  address in its code qualifies (the resume point maps to the copy's one `Suspend`).
- **Reserved function slots instead of tables that grow** (deviation from the study's section 8.2, reported): the elaborator
  reserves one empty function per instance of a qualifying process (at most 4096) at the end of the program before the runtime
  is made; a copy takes the next free one. A chunked append-only table for the program and for the runtime's per-function tables
  was built first and measured: 50 to 100 instructions per activation of a switched process for the lookups (+0.5 percent
  of `neorv32_single_min`), where a slot is an ordinary function id. Costs of the slots: about 1 KB each in the
  tables, 0.35 percent of the initialisation of `neorv32_single_min`. A design with more qualifying instances than slots keeps
  the rest shared (`spec_full`).
- **A copy folds reads of instance data into integer registers only** (a pointer or handle register keeps its load, the roots
  rule), never the bytes of a `for` generate's parameter (so the iterations of a generate share a copy), and runs through the
  existing pipeline (inlining, memo, layout slots), so the memo machinery that stays for what cannot fold is still there.
- **Growth cap 2.5 times** (user, 2026-10-24; 1.5 at first) the optimised IR of the shared process functions that ran; a copy
  over it is refused (`spec_capped`). At 1.5 it bound on `poc_arith_addw` (312 candidate copies of 40 functions, 3.5 times the
  shared IR; 147 allowed, 0.5 percent of the gain) and on `sc5_fifo_repack` (4.3 points); at 2.5 only `poc_arith_addw` is capped (244 of 312, within 0.1 percent of no cap) and `sc5_fifo_repack` interprets
  48.63 -> 46.12 G (-20.0 percent against no copies), peak RSS +2 MB there and within 1 MB elsewhere, initialisation equal.
- **Shared code is kept** with `--dap` (`keep_variables`), `--vhpi` and `--wave-variables` (`external_variables`), with coverage
  counters, under `--tier=llvm`, with `YAVS_NO_IROPT`, for serial processes and for processes with several waits; the
  stage cache holds copies only where optimised copies are portable (`copies_are_portable`).
- **Tiers**: under `--tier=auto` a copy is a function like any, sampled under its own id, but every policy decision about it is its
  process's (#661, user, 2026-10-24: "the auto tier counts samples, and the instance and share rules, by base process, so all copies of one
  process are treated as one for policy decisions, then compiles the copy that runs"): `Worker::group` is the interpreted time of the
  process and all its copies and the threshold, payback, band wait, probe and share and instance rules (`cold`) read it, a copy is
  classified (`rtl`, band, budget factor) and its threshold taken from the shared code of its process, every copy seen running is a candidate once
  the process is hot and is compiled from its own code, and a trial that takes one copy's compiled code back keeps the other copies not compiled
  yet interpreted. Compiled copies are named by `Program::copy_tag` (a hash of process and masked data) in the symbol names, so the
  compilation cache key (the bitcode) is the same in every run, whatever order the processes switched in (it used the function id, which
  follows the order, and so differed between runs with `--threads` above 1). A process whose shared code is compiled already does not switch, and none switches in the 150 ms
  after the tier is set (`SPEC_TIER_WAIT`: the tier needs about 30 ms of samples of a process before it compiles it, and a compiled process beats
  its interpreted copy by far); `YAVS_SPEC_AFTER` forces the switch, for tests, which also compile copies at once under `--tier-heat=1,0,0`.
- **Stage cache**: kind `spec`, one entry per lowering with the copies of all runs, read when the first copy is optimised and
  written at the end of a run that optimised one the entry lacked; a warm run optimises none.
- **Test aids**: `YAVS_NO_SPEC=1`, `YAVS_SPEC_AFTER=n`, `YAVS_SPEC_CAP=p`; counters `spec_*` (`YAVS_COUNTERS=1`).
  `crates/yavs/tests/spec.rs` is the shadow check: every perf design with copies off and with every process switching at its
  second activation, 1 and 4 threads, byte-identical output.
- **Measured** (`docs/STATUS.md` "Specialised copies of processes", `docs/design/specialisation.md` section 12; user instructions of the
  whole process tree, bench builds, main before and after): `sc5_fifo_repack` -15.6 %, `poc_arith_addw` -11.0 %, `fletcher_bufferwriter_512`
  -7.3 %, `poc_cache_cpu` -5.1 %, `hdlm_lfsr` -3.3 %, `neorv32_single_min` -2.5 % (the prototype's -3.3 % was against an older, slower
  base), single-instance testbenches (OSVVM, UVVM) 0; under `--tier=auto` cold -0.9 to -11.6 % (`hdlm_lfsr` +0.9 %), warm -2.5 to -7.5 %
  (`sc5_fifo_repack` +4.9 %, `hdlm_lfsr` +2.0 %: #661); initialisation within 0.4 %; with copies off within 0.1 % of before; the cap binds on two
  designs (`poc_arith_addw`, `sc5_fifo_repack`: 0.5 and 4.3 points of gain). The interpreter rule (instructions down, cycles not worse in
  the geometric mean) holds on `interp`: instructions 0.96 over ten realbench tests, interleaved cycle medians -2.5 to -10 % where measured;
  under `auto` the instructions are 0.98 and the two exceptions are filed.
- **Done after the first report (user, 2026-10-24):** the cap is 2.5; the tier policy counts copies by their process and compiled copies are
  keyed by content (#661, `docs/design/specialisation.md` section 12). **Tried and not landed** (measured, no gain, the patches are on #661):
  processes with several wait statements (a resume point mapped by statement admits 11 more processes over 42 realbench testbenches, +0.12 % to
  -0.01 % instructions) and folding the loads of pointer-typed instance data (0 to 2 such loads left per copy, at most 0.14 % fewer
  instructions). **Open**: designs the stage cache held no entries of re-optimised their copies in every run (`fletcher_bufferwriter_512`
  loads 21 units outside its analysed closure while it elaborates: fixed by #671, see "Stage entries of a design that loads units late");
  `hdlm_lfsr` runs under a lowered library (`library_made`), whose closures have no entries of their own so that a suite does not fill the 96
  entries of a library directory, which a `spec` entry per long-running testbench would do too.

## A flat view of the heap slots for compiled `Deref`: built, measured, not landed (yavs-interp, yavs-llvm, #655, 2026-10-23)

User decision 2026-10-23: build the view of #655 (`{ptr, len, generation}` per heap slot in the shared layer, so compiled code resolves
a handle without the object table) with strict invariants and a cross-check against the handle path. It was built, passed every safety
net, and lost on instructions, so nothing of it is in the tree (user decision 2026-10-23, #576: land a change if instructions drop
and the cycle geomean over many designs is not worse). The prototype is a patch against d72ffdd9, posted on #655 (the comment of
2026-10-23 that opens with the table below) for the day a second code generator or a different measurement asks again.

- **Design (all of it worked).** `Heap` keeps a `Vec<ViewSlot>` next to `slots`, one 16-byte entry per slot: host address and length
  of `data[start..]` (length under 4 GiB), and the generation. Every other slot (never used, deallocated, collected, being edited, too
  long) is `ViewSlot::NONE`: length 0 and generation 0, which no handle carries (generations start at 1). `TierFrame::heaps` held, per
  handle tag, a pointer to a header `{table address, index bound << 32, space bits}` that lives inside the heap (a `Machine` does not
  move during an activation), so the table growing and moving needed no refresh of the frame view. A `Deref` was: tag clamp, header
  loads, `handle & index bits < bound`, entry at `index * 16`, `entry.generation == handle as u32`, address from the space bits; a miss
  of any kind went to the old callback. Heap addresses (no generation) were looked up the same way with a liveness test on the entry, so
  an access of any size, none included, meets the freed slot's length 0.
- **Invariants and where each was kept.** Generation compared on every access (a stale, null or foreign handle misses and fails in the
  interpreter's words). A freed slot shows length 0 and generation 0 (`release`, which `take`, `free` and `collect` end in). `edit`
  hides the slot before it hands out the `Vec` (the caller may reallocate or return early without `reissue`: textio's error paths do),
  `reissue` shows it again. `alloc_from` and `reissue` are the only places that show an object. No raw pointer is cloned or
  serialised: `Heap` has a hand-written `Clone` and `restore_state` that rebuild the view for the new buffers (`#[serde(skip)]`), and
  addresses are `u64`s so a machine stays `Send`. `TIER_STAMPS` (writes through the view bypass `Heap::writes`): `tier_heaps` offers
  empty headers while the flag is set. The frame view is refreshed at the three places that set `heap_epoch` (`tier_view`,
  `view_root`, `tier_temp`, now together in `tier_heap`), which every host entry ends in; that covers a host call that brings the shared
  heap into being (`SharedState`'s heap is created lazily in `alloc_heap`). Process and thread switch: the view is rebuilt at every
  activation. The shared state is moved between a machine and the global guard only at the entry points of `lib.rs`, never during
  compiled code, so a header pointer cannot go stale inside an activation. VHPI and DAP reach heap objects through `Machine::obj` and
  `obj_mut` (`data`, `data_mut`: no buffer moves). Protected-object calls across threads: serial processes only, one at a time, each
  with its own activation. No update point was found that could not be covered.
- **Safety net.** A heap fuzz test (40 seeds x 400 random allocate, free, take, edit with growth, collect, recycle, clone and store
  round trip operations; after each: the view equals the slots, a handle the view finds resolves to the same bytes, a stale, null,
  foreign or out-of-range handle misses, a live one hits; removing the `hide` in `edit` fails it at step 4). A cross-check in debug
  and `gc-stress` builds: every hit of generated code called `TierHost::check_view` (a callback of its own, in no work counter) which
  compared the frame's header pointer with the heap's, the handle path's slot and the bytes' address and length, and panicked on a
  difference. The unit tests of yavs-interp and yavs-llvm passed under `gc-stress` (the `yavs gc-stress` feature does not switch on
  yavs-llvm's: it needs `"yavs-llvm?/gc-stress"`, which the check wants).
- **Measured (instructions of the main thread; `--tier=llvm` warm, the only deterministic tier: `--tier=auto` runs differ by 5 percent
  from run to run in what they compile when; builds from `tools/build-bench.sh --features llvm`, core 11).**

  | Change | one protected `get` (4 `Deref`s), instr | `protected_queue` | window generator, row bank case | Cranelift too? |
  |---|---:|---:|---:|---|
  | main (object table) | 336 to 339 | 5.392 G | 257.98 G | |
  | `Deref` through the view, miss to the old callback | 367 (+9 percent) | 5.760 G (+6.8) | 266.3 G (+3.2) | yes: the view is TierFrame data |
  | plus addresses a `Deref` made through the view | 365 to 367 (+8) | 5.777 G (+7.1) | 266.7 G (+3.4) | yes |
  | the view used only by `TierHost::deref` and `object` (no change to generated code) | 336 | 5.391 G | 257.46 G (-0.2) | yes |

  Callbacks of the window generator (`--tier=auto`): `cb_fast` 7.28 M to 0, `cb_host` 3.42 M to 1.85 M; `protected_queue` has 142 and
  49. `TierHost::object` is 2.5 percent of that run's cycles and `yavs_rt_fast` 0.4, so the callbacks the view removes are worth at
  most 1 to 3 percent, and the table that costs them is gone only if the hit path is as short as the table's.
  Real testbenches (`tools/realbench`, `--tier=auto` warm, view in the `Deref` and addresses, 3 runs, cores 11 and 12): instructions
  within 1 percent on the window generator (190.5 G to 190.2 G), `uvvm_sb_slv`, `uvvm_axistream_vvc`, `osvvm_axis_random`, `hdlm_axis_bfm`
  and `hdlm_fifo_sync`; `sc5_fifo_repack` +5 percent and `uvvm_util_rand` -40 percent (probably the functions `auto` had
  compiled when the run ended, which differs from run to run; not checked); wall within the spread.
- **Why it loses.** The estimate of #655 (about 22 instructions per `Deref` to about 12) counted the index, the bound and the
  generation compare. A `Deref` also has to choose the heap by the handle's tag, reach its table, form the address's space bits
  and load pointer and length: 24 LLVM instructions in the optimised function against 18 for the table hit (key from index and tag,
  entry address, one compare of the stored access value, three loads), which already is one compare because an entry that has an
  access value is writable and no entry holds the null handle (`Obj::h`, round 14). About two of the six extra are this
  prototype's choice: the table is reached through a header inside the heap (a dependent load, and the tag-indexed address of the
  pointer), which was chosen so that no refresh of the frame view can be missed; copying base and bound into `TierFrame` and
  branching on the tag is estimated at parity with the table hit and was not measured, and parity does not land. The table
  misses on conflicts (direct-mapped, 256 entries) and after a deallocation, and the misses of the real runs cost 1 to 3
  percent; the flat view buys those back and pays four to seven instructions at every hit.
- **The host-side variant (h0)** used the view only in `TierHost::deref` and `object`, which leaves the generated code as it is:
  -0.2 percent of the window generator's instructions (deterministic), nothing on the others. The cycle half of the landing rule
  was not measured for it, and 0.2 percent does not pay for a second copy of the slot state whose one missed update would be a host
  use-after-free in `object`.
- **What would change the verdict.** Misses becoming a larger part of a run (a protected library with thousands of live objects in
  its hot loop; a counter: `cb_fast` plus `cb_host` over 5 percent of the instructions): then the table first, with the view
  filling it through a small Rust helper instead of the host call, is the design (ceiling measured here: about 1 percent of the
  window generator, so not before the misses grow). A second code generator with no object table can use the prototype as it is
  (`TierFrame::heaps` and `ViewSlot` are backend-neutral) and does not need the table's per-Deref hash.
- **Prototype checks.** `cargo test -p yavs-interp --features gc-stress` (24 + helpers) and `-p yavs-llvm --features llvm,gc-stress`
  (31 tier tests with the cross-check) green; the differential corpus on a release `gc-stress` build with the cross-check on:
  499 of 499 tests on `yavs-interp`, `yavs-auto` and `yavs-llvm` (three tests added to the corpus while it ran need a newer build
  than the prototype's). The measured binaries are the same code without the check.

## IR optimiser round 12: statement markers of signal statements, and what was tried for temporaries, checks and drives (yavs-ir `opt.rs`, #588, 2026-10-24)

Decision (landed): **signal reads, signal references, edge and attribute tests and drives are on `may_allocate`'s list of instructions
that never allocate temporary memory**, so `drop_statements` removes the `Stmt` marker of a statement made of them (a process that only
reads and drives signals had a marker per statement; the marker frees temporaries, and there were none). Waits and `Suspend` stay off
the list: a wait starts at its `Stmt` for `static_wait` (the predecoder's `WaitStatic`, compiled code's short cut), and with the markers
dropped `neorv32_single_min` ran 446 M ops instead of 360 M (the static waits stopped matching). Host instructions (`perf stat`
`instructions:u`, `--threads 1`, bench builds, warm world, deterministic to 0.05 percent), main at 35eec8cb before / after:

| design | interp | llvm |
|---|---|---|
| `sc5_timestamp` | 6.928 / 6.850 G (-1.1 %) | 5.763 / 5.744 G (-0.3 %) |
| `sc5_fifo_repack` | 46.085 / 45.794 (-0.6 %) | 35.636 / 35.605 (-0.1 %) |
| `neorv32_dual_full` / `neorv32_single_min` | 45.805 / 45.580 (-0.5 %), 63.288 / 63.039 (-0.4 %) | 59.684 / 59.699 (0.0 %), 69.928 / 69.929 (0.0 %) |
| `fletcher_bufferreader` / `fletcher_bufferwriter_512` | 38.353 / 38.170 (-0.5 %), 98.593 / 98.339 (-0.3 %) | 39.844 / 39.829 (0.0 %), 99.284 / 99.187 (-0.1 %) |
| `poc_cache_cpu`, `poc_stat_histogram`, `poc_arith_addw`, `vai_window_gen_interlock` | -0.1 %, 0, 0, 0 | -0.1 % (a spread of 0.3 percent between runs), -0.1 %, 0, 0 |
| `tools/bench` h2h: `rtl_fan` / `resolved_bus` / `rtl_ff` / `rtl_bus` / `rtl_lfsr` / `lfsr64` | -3.2 / -2.1 / -1.5 / -1.4 / -0.9 / -0.9 % | -1.0 / -0.5 / -0.7 / -0.4 / -0.9 / -0.4 % |

No design is worse beyond the spread of the tier (0.3 percent under `--tier=llvm`, where `poc_cache_cpu` read +0.3 and -0.1 in two
runs). Geometric mean of the instruction ratios, interpreter 0.9965 over the ten real testbenches and 0.9955 over the 24 h2h designs of
over 0.1 G, `--tier=llvm` 0.9996 and 0.9983. `--tier=auto` counts vary between two runs of one binary (`poc_arith_addw` 255.9 and 252.3 G, `vai_window_gen_interlock` 195.4
and 182.2 G on the same build: which functions are compiled when the run ends), so no auto figure is quoted. The `ops` bounds of five
`tests/perf` designs fall by 1.6 to 4.6 percent (`reg_bytes`, `lfsr_array`, `delayed`, `wait_static`, `idle_regs`).

What was measured and not landed (all deterministic instruction counts, `--threads 1`; why they lose, what would change the verdict):

- **Constant-size temporaries in the frame** (the first item of #588): a pass for processes turned every `Alloc` of a constant size
  whose pointer stays in its block (never stored but into another such temporary, never returned or rooted) into bytes of the process
  frame (`AddrOf` of the frame, the blocks of one basic block that are not alive together sharing bytes, the zeroing as a new
  `Inst::Zero` that the stores around it often make unnecessary), then `drop_statements` removed the markers. It was correct (the
  differential corpus, OSVVM, UVVM) and **slower on the interpreter**: `neorv32_single_min` 63.28 to 75.25 G (+18.9 %), `sc5_fifo_repack`
  +17.5 %, `fletcher_bufferwriter_512` +8.8 %, `poc_cache_cpu` +4.5 %, `sc5_timestamp` -3.3 %. Every common temporary has a fused op that
  matches the `Alloc` (`WaveBuf`, `LSetDrive`, `CopyDrive`, `AllocReadVec`, `AllocLCopy`, `AllocCat`, `NewPair`) and the frame form
  matches none: on `neorv32_single_min` 360 M ops became 413 M, 53 M of them `Zero` through the generic step and the `Drive` ops
  doubled (17 M to 35 M) because the three-op wave sequences came apart. Under `--tier=llvm` the run took 116 G instructions
  against 71 G, with the compiled code at 4 percent of the samples against 35 percent (so the compiled processes mostly did not run;
  the cause was not examined, nor whether `Inst::Zero` or the lost `ScalarDrive` sites decided it). The static LLVM size is where the
  gain would be (`YAVS_LLVM_KINDS`: `Alloc` 10 percent and `LCopy` 9 percent of the instructions generated for the neorv32 testbench,
  `SigRead` 13), and the dynamic interpreter gain is small even for perfect temporaries: the ops involved are 50 M of 360 M and the
  `Stmt` markers saved 7 M. The verdict would change with frame-address forms of the fused ops (predecoder) and of `ScalarDrive` and
  `Zero` in the LLVM tier, which are the interpreter's and the tier's (#668, with the pass). The reference simulator lowers a scalar signal assignment to a call
  with the value as an argument (no waveform buffer) and keeps constant-size temporaries as fixed stack slots of the unit: the
  buffer-free drive is the first thing to try (below).
- **A drive from a value block** (not built; the idea above): `Alloc`, header `Store`, `Index`, `Copy` or `LCopy`, `Drive` of one
  transaction at time zero would be one instruction that drives from the block the value already is in. In `neorv32_single_min` the
  sequence `WaveBuf`, `LCopy`, `Drive` (not fused) runs 9.0 M times in 300 us (2.5 percent of the ops; `op_wave_buf`, `lcopy` and the
  copy are about 4 percent of the instructions), and `CopyDrive` (fused) 4.9 M times; statically 395 of 4065 drive sites are fed by
  `LCopy` and 1490 by `Copy` in that design. A fused `LCopyDrive` op in the predecoder gets part of it without an IR change (#669).
- **Inlining policy** (`inline.rs`): the cheap exit on the lowered size (120 instructions) rejects callees that shrink below the limit
  once optimised (`assertion` of the simulation package, `get` of the pointer tables). With 400: `vai_window_gen_interlock` 495.0 to
  489.6 G (-1.1 %), the others within 0.02 percent, ops +0.9 percent (a `Case` becomes compares and moves). With the body limit at 70 as
  well: +0.02 percent. The two accessors that matter in the window generator are 61 and 66 instructions; the limit is tuned for
  `--tier=auto` (above). Not changed.
- **Signal values as available expressions and walks across joins**: `SigReadScalar`, `Edge` and `SigAttr` of a constant signal reference are
  one value per activation (until a call or a force), so a repeat is the first read; the walk is over extended basic blocks, so the
  value read before an `if` is read again after it. A first version keyed the three instructions; a second started the join blocks
  from their dominator (pure values and signal values kept, memory facts and the versions of registers defined in between dropped).
  Dynamic repeats of scalar reads in one activation: 8 percent on `neorv32_single_min`, 12 on `fletcher_bufferwriter_512`, 58 on
  `sc5_fifo_repack`. Instructions: `poc_cache_cpu` -0.5 %, `sc5_fifo_repack` -0.3 %, `fletcher_bufferwriter_512` -0.2 %, `neorv32_single_min`
  +0.1 %, the rest within 0.03 percent: not worth the dominator walk's complexity.
- **Check removal by ranges** (second item of #588): `Check` and `BinRange` are 11 percent of the ops of `vai_window_gen_interlock`
  but the hot ones compare a value loaded from memory against a subtype that is narrower than the one the field was declared with
  (`index_t` to `natural`), which no interval proves; loop counters are already handled. Cheap ops: not pursued.
- **Promotion between suspensions** (third item): `LoadSlot`/`StoreSlot` are under 1 percent of the ops of `neorv32_single_min` and about
  1.5 percent of `fletcher_bufferwriter_512`. Not pursued.
- **Eager messages** of `assert`-like procedures (`simAssertion` in `poc_arith_addw`: 43 percent of that run is the checker process, most of
  it the message built for every call): moving the message into the failing branch is blocked by `to_integer` inside the message (a
  warning on a metavalue is output the passing assertion would no longer print) and by the length check of the concatenation (a failure
  only above 2 GiB, but not provable). Needs either a purity flag on natives or lazy arguments in lowering.
- **Repeated drives** (outside the list of #588, the largest redundancy found): 17 percent of the drives of `neorv32_single_min` and 12
  percent of `fletcher_bufferwriter_512` drive a driver that the same process already drove in the same activation (the default
  assignment of a combinational process before the `if` that overrides it); `sc5_fifo_repack` none. In the IR only 2 of 2075 repeat
  sites are in one block, so removing the first drive needs sinking it into the arms that do not override it (partial dead
  store elimination) or a pending-drive merge in the runtime (#667). The frame temporaries are #668, the unfused vector drive #669,
  the spread of `--tier=auto` counts #670.

## Compiled clocked RTL processes, round 6: the cache lines of an activation (yavs-interp, yavs-llvm, #634, 2026-10-24)

The entry plumbing of round 5 was measured in instructions; this round measured it in cache lines, because `rtl_fan` (4000
near-idle processes) ran 1.2 times the interpreter's cycles with the same instructions. `perf record -e ibs_op// -d` (the
data source of each sampled load: L1, L2, L3) showed where the compiled activation waits: loads that hit L3 in `view_again`
and `run_quick` (94 and 93 samples of loads that missed L1 in those two functions; 100 in the whole interpreted run against 222 compiled), and
`ls_any_fills_from_sys.local_ccx` (fills from L3) 82 M against 64 M. `view_root` read machine state from five cache lines that
nothing else in an activation touches (the heap's epoch line, `owner`, `troots`, `tier_sigs` and the instance data's vectors,
reached through the `Arc`), and `InterpProcess::run` read `spec_at` from the thirteenth line of the machine at every
activation (both tiers). Decisions, tagged shared (S, any code generator) or LLVM only (L):

- **A fourth line, `Machine::vc`, holds what compiled code's frame view reads and the interpreter does not** (S,
  `ViewCache`, `repr(C, align(64))`: the instance signal table `tier_sigs`, `owner`, the instance data's pointer and
  length, the number of signals; the heap starts at the fifth line as it started at the fourth). The instance data pointer
  and length are numbers copied at `Machine::new` (an `Arc<InstData>` never changes its vectors). A new read of machine
  state in `view_root` goes in `vc`, or its line is counted first (`activation_fields_are_in_the_first_lines` pins the
  offsets).
- **`spec_at` is a `u32` next to `execs`** (S): the comparison `InterpProcess::run` makes at every activation read a line of its own;
  it is `u32::MAX` for "never" (`specialise` ignores a switch it did not plan).
- **The scalar drive's slow path is out of line** (L, `yavs_rt_drive_bit_slow`): inline, it gave the hot function a six-register
  prologue and `catch_unwind` set-up for the case that never happens (one `drive_bit_now` call, 43 instructions to 38).
- **`Mem::end` stores the owner and epoch only while objects are kept, and `nslots` is looked up once** (L): two stores and a
  thread-local read fewer per activation of a process that keeps no objects.

- **`view_root` writes what changed** (S): the root frame's `frame`, `frame_hi` and `up` stay in the view until a deeper frame's view
  replaces them (`tier_view` writes the three together), `max_depth` is in `vc`, and a `hot` process (no shared state) takes its
  heap epoch and flags without looking at `sh`. 77 to 68 instructions per activation.
- **`yavs_rt_drive` is a small hot function and a cold slow path** (L, `yavs_rt_drive_slow`: the wide target and the assignments
  the kernel's fastest path does not take), as the scalar drive is: 143 to 125 instructions per activation of
  `neorv32_single_min`.
- **An `LCopy` is inline only where its count is a known constant of at most 64** (L): a count known only at run time had the inline
  body (125 LLVM instructions) and the call both, and in the wide-vector processes of the fletcher designs (copies of 512
  elements) the inline body is never taken: `LCopy` 67 K to 24 K of the 816 K LLVM instructions of `fletcher_bufferreader`, L1
  instruction-cache misses -5 to -17 percent, fills from L3 -15 percent in the first batches. Cycles on a quiet host (nine
  interleaved runs, minimum): -1.7 percent on the reader (14.10 G to 13.85 G), -0.6 percent on the writer, +0.6 percent
  instructions there (the helper call); a loaded host showed -2 to -8 percent in two batches, which was noise. `neorv32_single_min`
  and the `rtl_*` designs keep their inline copies (equal instructions: the counts are constants there). Forcing every known count of an `LCopy` or a `SigRead`
  through the helpers was measured too and loses on `neorv32_single_min` (+3 to +8 percent cycles, fewer cache misses and more
  instructions): the inline expansions pay where they run every activation.
- **The native call of a `hot` process reads no cold machine state** (S, `native_frame`, `heap_epochs`, `reclaim_pending`): the two
  fields of package initialisation (`pkg_init`, `init_reports`, two lines of their own) and the shared state are not looked at for a
  process that is `hot` (no shared state, not initialising). Cycles of `autoh`: writer -1.0, reader -1.1, `neorv32_single_min` level
  (minimum of 7 runs, quiet host); the small designs make no native call and are level.
- **Measured and not taken**: warming the next process of a batch (a `Process::warm` that reads the next machine's first lines
  while the current one runs: +50 instructions per activation, twice the fills from L3 and +15 percent cycles on `rtl_fan`); the
  interpreter's fused vector assignment (`Op::LCopyDrive`) run by compiled code through the interpreter's own function
  (`neorv32_single_min` +2.3 percent instructions: the function and the host lookup of the op cost more than the compiled
  group they replace); a persistent per-process view in the tier's state (round 2 measured it, see above).

Measured (bench builds, main thread; cycles of the simulation phase, minimum of 7 interleaved runs, core 12; `autoh` is
`--tier=auto --tier-heat=1,0,0`): see `docs/STATUS.md` "llvm: compiled clocked RTL, round 6". Fills from L3 of `rtl_fan`
compiled 82 M to 65 M (interpreter 64 M to 59 M).

## Record signals as packs of leaves, step 2 (yavs-elab, yavs-kernel, #686, user, 2026-10-24)

- **Decision (user, 2026-10-24): option A of `docs/design/record-signal-layout.md`.** Runs of consecutive leaves of one signal object
  share one kernel signal (a pack) when every signal-wide kernel property agrees; one byte blob per record in the IR layout (option B)
  is not pursued, it would undo "bit-planes everywhere" (#83).
- **The logical model stays:** `LeafId` per field, `NetGraph::select`, `DriverDecl` per leaf. A leaf is stored at `NetGraph::loc`
  (kernel signal, base) and every consumer turns a leaf into a kernel reference through `NetGraph::kref`/`kpart`/`kelem`, never
  `SignalId(leaf.0)`; `NetGraph::packs` lists the leaves of each kernel signal. Kernel signals are in first-leaf order.
- **Which leaves pack (`NetGraph::packable`):** logic elements, unresolved or resolved by `resolved` (an unresolved leaf has at most one
  source per element, elaboration checks it before, and `resolved` of one source is the identity), at most `PACK_MAX` = 64 elements in a
  pack (one plane block: codes always kept, element stamps for compiled code, no block records or lean history), and none of: a guard, an
  implicit signal on it or being one, a force or release target (no packs at all when one is untraceable), a port edge or converted port
  (step 3), a composite resolved as a whole. A pack is named by the path of the innermost record holding all its leaves (hang reports).
- **Drivers stay one per leaf** (several drivers of one process on disjoint ranges of a pack are one source per element as before), so a
  whole-driver assignment (`Drive` without a target) still means the leaf. Merging them is step 4, with the bulk assignment.
- **Idle skip reads carry their range:** a pure process is armed by an event in the elements it reads (the hull of its reads of the
  signal), not by any event of the signal, so packing a record does not wake readers of one field for another; a signal whose changed
  runs are not listed arms all its readers.
- `YAVS_PACKS=off` (hidden test aid, `ElabConfig::packs`) gives every leaf a signal of its own; the work counter `signals` counts kernel
  signals (`crates/yavs/tests/perf.rs` bounds it for the record designs).

## Record signals as packs of leaves, step 4: one driver and one assignment per pack (yavs-elab, yavs-interp, yavs-kernel, #686, #677, 2026-10-24)

- **Drivers of one process on adjacent elements of one kernel signal that one target names together** (the leaves of a pack in a whole
  record assignment) are one kernel driver (`build.rs`); fields assigned one by one keep a driver each, because a direct driver takes
  either single-element or vector entries in one run and a mix sends every assignment to the general path (measured: +3.7 percent
  instructions on `grlib_leon3mp` when all adjacent drivers of a process were merged).
- `ProcDriver::range` holds kernel elements (the leaf's base added once, at the build; not offset again by `InstSig::off`), and a `Drive`
  without a target passes that range, since the kernel driver may be wider than the slot; `'driving_value` takes the slot's part
  (`Ctx::driving_value_of`).
- `SigScatter` (#677) collects logic leaves that are adjacent in one kernel driver into one plane block and assigns it once (one `OutV`
  entry, one commit, one finish word per pack); `SigGather` reads a logic vector leaf inside one plane block from the signal's planes
  (`Ctx::planes`); a scalar leaf keeps `elem_code` (a plane extract per scalar was slower).
- Measured (bench builds, `--tier=interp`, instructions, warm): synthetic 233-leaf register record 5.20 G to 3.40 G; `grlib_leon3mp`
  809.6 G to 801.4 G; `neorv32_single_min` 42.79 G to 42.26 G (steps 2 and 4 together on top of #677).
- **Step 3 measured, not landed:** packing port leaves too (leaves with the same port edges: mode, side and the other side's object; no
  conversion; their port components free of anything that stops skipping) with coalesced edges took `grlib_leon3mp` from 2516 to 1826
  kernel signals but its idle skips from 2.09 M to 1.75 M (+2 percent instructions; `neorv32_single_min` level, 2021 to 1596 signals),
  also with reader ranges kept apart per read. The processes that lost skips are the combinational TLB tag comparators that read a few
  fields of a record in-port fed from an element of an array of records (not collapsed); why their arming grew is open.
- **Step 5 measured: the cap stays 64.** `PACK_MAX` 256 (a pack beyond one plane block keeps no codes once a whole-record driver
  covers it, and compiled code gets no element stamps) took the synthetic 233-leaf register record from 3.30 G to 3.46 G instructions
  and `neorv32_single_min` from 41.94 G to 41.89 G.
- **Idle skip reads carry their range:** a pure process is armed by an event in the elements it reads (the hull of its reads of the
  signal), not by any event of the signal, so packing a record does not wake readers of one field for another; a signal whose changed
  runs are not listed arms all its readers.
- `YAVS_PACKS=off` (hidden test aid, `ElabConfig::packs`) gives every leaf a signal of its own; the work counter `signals` counts kernel
  signals (`crates/yavs/tests/perf.rs` bounds it for the record designs).

## Hang and loop reports per part of a signal (yavs-kernel `hang.rs`, yavs-elab, #704, 2026-10-24)

- **Decision: the stuck report and the delta loop cause work on parts of signals, not whole signals.** The elaborator names the
  leaves of a record pack as parts (`SimControl::name_parts`); waits, drivers, pending transactions, port links, values and the last
  event are taken per part, and the report's `SignalId`s number the parts (`StuckReport::parts` maps them to the kernel signal and
  elements). With whole signals, a process waiting for one field was reported against the whole pack, with another field's value,
  event and driver. Output for designs without packs is unchanged.

## Composite signal reads: one copy per activation, scalar leaves read in place (yavs-lower, yavs-ir `FuncBuilder`, #630, #538, 2026-10-24)

Shared layer: every tier and a second code generator see the smaller IR. Found with the LEON3 template of GRLIB (`grlib_leon3mp`,
`--std=93`): the integer unit's `comb` process was 147 760 IR instructions, 32 percent of the run, and no tier could take it. The IR
showed what it was made of: 54 whole reads of the register record `r` (233 leaves, about 2 700 IR instructions each, 98 percent of the
process), one for every call that passes `r` to a function or procedure (`dbgexc (r, ..)`, `npc_find (r)`, `sp_write (r, ..)`) and for
`v := r`. A record signal has one kernel signal per leaf, so a whole read is a `SigRead` and a field address per leaf.

- **A process with a sensitivity list keeps one copy of each record signal it reads as a whole in two places or more** (a read in a
  loop counts twice; `Fx::whole_reads` over `Db::stmt_exprs`; a name that selects a field, an element or an attribute is no whole
  read of its prefix). Such a process runs between two suspensions without a wait (LRM 11.3), so the signal has one value for the whole
  activation. The copy is an object like a variable's (`NewFill` of zero bytes in the declarative part, `[data, desc, handle]` in a
  frame slot that is a handle root); the leaves are read into it by one fill at the start of each activation (`SigCopies`,
  `Fx::sigcopy_of`), and every whole read of the signal in the body is the address of the copy. The code of the copy is made at the
  first read, in place: `FuncBuilder::{current, block_len, take, splice, set_stmt}` move the prologue before the jump that ends the
  declarative part and the fill to the start of the first block, new blocks for control flow in the leaf copies (arrays of records)
  included. A signal read as a whole once is read as before. Not for processes with explicit waits (a wait ends the activation) or
  the processes of concurrent statements (their bodies are one statement).
- **A `std_ulogic` field below records only is read with `SigReadScalar` and stored where it lies** (`read_leaves_into`), not through
  a 32-byte plane buffer, `SigRead` and `LGet`, and the address of the records on the path is made once for the leaves that share
  it. Every composite signal read gets this, in any function.
- **Ceilings** (`ponytail:`): the fill reads every leaf at every activation, also when only a few leaves changed (a per-leaf event
  stamp would skip them); a whole read in a branch that is almost never taken, in two places, still pays the fill; the drive side
  of `rin <= v` is still one `Drive` per leaf (233 per activation, 4 400 IR instructions in `comb`); a procedure that reads the
  signal as a whole keeps its own reads.
- **Measured** (bench builds, `--tier=interp --threads 1`, core 10, the whole run to 1060.5 us, "Test passed"; `perf stat` user cycles
  and instructions of the process): 1685.8 G instructions, 604.8 G cycles, 150.8 s before; 993.3 G, 428.1 G, 91.3 s after
  (-41, -29, -39 percent). `comb` 147 760 to 30 605 IR instructions, the program 473.8 k to 354.3 k, `dctrl` 24 376 to 24 112.
  The process `iu:comb` fell from 37 percent of the samples at 150 us to 16. `crates/yavs/tests/perf/sigrec_copy.vhd` (a process that
  passes a record with nested records to four functions, in branches and a loop, and checks the results against field-wise reads):
  ops 105 380 to 52 919 (a bound with 3 percent), the other 29 work-counter designs unchanged.

## llvm: the compile cost of a very large process (yavs-llvm, #630, #538, #532, 2026-10-24)

Found with the integer unit of the GRLIB LEON3 (`iu3` `comb`, `--tier=llvm` spent 82 s in code generation for the design, 171 s for the
one process on a loaded host). L: LLVM only; S: also a second code generator.

- **Where the 171 s went** (one function of 147 760 IR instructions, 7.16 M LLVM instructions before the passes, `YAVS_LLVM_ONLY`,
  `YAVS_LLVM_TIMES`, bench build, `Fast`): generation 5.8 s, verification 1.6 s, the `LEAN` passes 143.7 s, code generation 28 s. `opt
  -time-passes` on the dumped module: `simplifycfg` 66 s, `adce` 31 s, `instcombine` 8.7 s, bitcode writer 6 s, `early-cse` 2 s; the same
  passes took 0.69, 0.63 and 0.94 s on a 24 000-instruction process of a tenth of the size, so two passes are quadratic in the module and
  the others linear. (L) A module of more than `HUGE_MIN` (100 000 LLVM instructions) gets `LEAN_HUGE`, `dce` instead of `simplifycfg` and
  `adce`: 12 s for the passes of that function, the code 6 percent larger. A 520 000-instruction copy of the process: 8.0 s to 3.3 s
  with the module read from text; the code generator takes 3.8 s of 3.4 MB of x86 either way.
- **`auto::compute` called the process a compute loop** (two short `for` loops with nothing that leaves the compiled code), so
  `level_for` kept `O2`: optimised code generation (the register allocator is more than linear in the size) took 35 s for 670 000 LLVM
  instructions where `Fast` takes 8 s. (L) `COMPUTE_MAX` (1 500 IR instructions): from there on a function is `Fast` whatever its loops.
  `auto::compute` itself is the policy's and is unchanged.
- **Together with the smaller IR of "Composite signal reads: one copy per activation"**: the compile thread's time for the copy
  `--tier=auto` makes of the process (23 641 IR instructions, 406 000 LLVM instructions): generation 0.31 s, verification 0.10 s, passes
  0.57 s, code generation 1.56 s, 2.5 s in all, against 39.6 s for the shared function at `O2` codegen before (core 10, loaded host).
  The object is 3.9 MB for 28 000 IR instructions (142 bytes an IR instruction). The window generator's large processes at `Fast`
  (copies, forced with `YAVS_AUTO_FORCE`): `control` 4 578 IR instructions 0.40 s, `walk_control` 2 563 0.28 s, `bus_process` 2 133 0.27 s
  (80 to 130 us an IR instruction; the cost model prices them at 800, #680). `crates/yavs-llvm/tests/jit.rs`
  `a_huge_module_matches_reference_semantics` (120 000 LLVM instructions through `LEAN_HUGE`) and `auto::tests::a_huge_function_with_a_short_loop_is_compiled_fast`.
- **What the compile cost does not buy yet: compiled code of such a process is not faster than the interpreter.** Measured with
  `YAVS_AUTO_FORCE=comb@10000#1211` (the copy compiled at the first sample) against `--tier=interp`, same binary, `--threads 1`,
  `perf stat --no-inherit` user cycles and instructions of the simulation thread between 300 us and 500 us, core 10: interpreter 72.2 G
  cycles / 162.4 G instructions, compiled copy 91.3 G / 168.2 G (before the smaller IR; the shared function of 147 760 IR instructions
  was 2 to 3 times slower than the interpreter). The compiled body is not the cost: a profile of the window has `f1223_comb` at 1.2 percent
  of the cycles and 15 percent in the callbacks it makes (`read_into` of `SigRead`, `lcopy`/`copy_bits` of `LCopy`, `Host::step` of
  `LayoutOf`, `assign_code_fast` of `Drive`, `yavs_rt_host`), against 26 percent for the interpreter loop, so the leaf reads and the
  233 drives per activation cost the same in both. The instruction mix of the compiled code (an `instructions:u` profile of the window
  mapped to the object with `objdump`): 51 percent `mov`, `sub` 8, `cmp` 7, `je` 6; `Fast` spills every value. A shared
  `neorv32_single_min` process of 4 994 IR instructions (`exec_comb`) compiled at `Fast` and at `O2` code generation (1.8 s against
  0.35 s) runs level with the interpreter (cycles +1 to +5 percent, instructions -3 percent) either way. So raising the number of
  large processes `--tier=auto` compiles needs the code to get cheaper first (callbacks per record leaf, the address resolution of
  every memory access, spills), not the compile.

## `--tier=auto`: callees with their caller, `band_wait` 0.25 s, trials of looping processes; `fused_search` stays (yavs-llvm, yavs-interp, #652, #573, 2026-10-24)

Policy owner's change, three parts. Measured with benchmark builds (`tools/build-bench.sh --features llvm`, both sides the same
flags, built from the same main), main-thread `perf stat --no-inherit`, medians of 5 to 9 interleaved runs, one core of CCD1 per
run. The host was loaded by other agents while these ran (load average 20 to 30): cycles of the same binary moved by up to 10 to 25
percent between invocations, so the instruction counts are the reliable column and cycles are read only where they agree.

- **`band_wait` is 0.25 s (`BAND_WAIT`, user decision 2026-10-23; was 1 s).** A clocked process outside `RTL_BAND` (160 to 300
  IR instructions) is compiled and tried once it was seen interpreted for a quarter of a second. Gain on runs of 0.4 s and
  more (auto against interp of the same build, instructions; main before this change / after; warm, cold in brackets):
  `rtl_ff` itself (136 IR, 0.7 s) 1.041 / 0.922 (1.040 / 0.893), a 351 IR variant of `rtl_ff` (12 xor terms, 1.2 s) 1.040 / 0.784
  (0.853 / 0.766), a 535 IR variant (1.7 s) 1.040 / 0.687 (0.729 / 0.661), `lfsr64` at ten times its length (2 s) 0.950 / 0.909,
  `rtl_ff` at five times its length 0.855 / 0.802. Cycles agree where the host load allowed a reading (`rtl_ff` 1.035 / 0.938,
  `ffv20` 1.033 / 0.792, wall 0.84 to 0.92 of the old build). Cost of a
  trial that loses: the compile thread's compile (2 to 90 ms) and the windows with the slower of the two codes, 100 to 350 ms in
  all and at most 10 percent slower than the interpreter for the part of the run they last, so below 1 percent of any run long
  enough to reach the trial. No clocked h2h design lost its trial in these runs (compiled clocked code gained in every variant
  tried), so the cost of a taken-back clocked process was not measured again here. Short runs (`rtl_chain`, `rtl_fan`, `rtl_bus`, `rtl_ram`, `lfsr64`,
  `lfsr64_xor`, 0.09 to 0.8 s, cold and warm): instructions equal to 0.5 percent, wall within the spread (a 15-run check of the
  two shortest cold: 0.388 and 0.184 s on both sides). The lower end of `RTL_BAND` (160) has no reason left after the compiled
  clocked code of #634 (cg634's synthetic crossover: 144 IR gains); it is left as it is, the wait is what the user asked to change.
- **A callee that every call of its caller makes is compiled with the caller (`must_calls`, `Worker::hot`).** `Worker::tree`
  left out a callee never seen running (`UNSEEN_MIN`, a70c7a14), which is right for error paths but wrong for a small callee on the
  caller's main path: it is as hot as the caller, but the sampler cannot see it (it is short, and the caller's time covers it), and
  compiled code that calls an interpreted function steps out of its native frame for each call. `uvvm_sb_slv` with `fused_search`
  off: the compiled `pos_of_leftmost` called the interpreted `bitvis_assert` twice per call through `yavs_rt_step` (`cb_step.Call`
  at pc 34 and 42, 168 in the sampled window and 111 k over the run), and the callee compiled 340 ms later. Now the callees of
  at most `CALLEE_MAX` IR instructions that every path from the entry to a `Ret` (a `Suspend` for a process) passes, transitively,
  count as seen (`must_calls`: a call is on every path when removing the instructions that make it leaves no path to an exit; a
  function of over 4000 IR instructions is not searched). Effect on the realbench set (final interleaved run, auto
  against interp, instructions, warm): equal within 1 percent on all nine testbenches of the short set; `vai_window_gen_interlock`
  0.379 / 0.374 and `poc_arith_addw` 0.799 / 0.788 (warm, 5 runs). Builds made earlier in the work showed `uvvm_util_rand` 1.069 / 1.008 and
  `poc_cache_cpu` 1.012 / 0.972 from this change; the final run did not reproduce them (1.068 / 1.068, 1.016 / 1.018): which
  functions are compiled before the run ends varies with the compile thread's timing, so claim nothing from them.
- **`fused_search` stays.** With the rule off and the callee fix `uvvm_sb_slv` ran 1.05 to 1.12 times the interpreter's cycles
  (instructions 1.12; without the fix 1.12 to 1.19 and 1.19 to 1.25), a gain over the 1.22 of 7751ab51 but not level: the
  compiled `pos_of_leftmost` gains 1.2 to 1.25 times in the entry trial when its windows are measured, yet the run does not
  (the loops run in `yavs_rt_scan` either way, and the function's own cost is a small part of the 111 k calls of its whole
  statement path, `cb_native` 0.94 M and `cb_xstep` 0.2 M callbacks in the rest of its callers). Its entry trial is also erratic
  there (verdicts 0.2 to 1.26 from the same code, a burst of the testbench in one window), so no trial could be trusted to take
  it back. With the rule on: 1.00 to 1.03, as before. Not removed; the gap is the decision's evidence if the next change to
  the search helper makes the compiled function level.
- **A process that loops is measured by its activations; a function is not (#652).** `NativeFlags::count_on(func)` makes the
  interpreter count the activations of the process function `func` (`run_quick`: one relaxed load per activation while a tier is
  set, nothing without a tier, no change in the work counters `ops` and `cb_*`). `Worker::trial` now also runs for a process that
  is not `rtl` (it loops between its waits, or is too long): the clocked processes by the kernel's process runs as before
  (`BY_RUNS`), the others by their activations per CPU second (`BY_ENTRIES`). Activations come in bursts: windows of the same
  code differed by 10 to 25 percent, so this trial takes all three pairs of windows before deciding and takes the code back only
  below 0.95. Verdicts in the runs: `_p8` of `poc_arith_addw` kept (1.05, the case of #596), `procChecker` of the same design and
  `bus_process`, `check_axi_stream` of the window generator not measurable (too few activations in a window of 250 ms: kept).
  **Functions stay on the static rules.** A trial of function entries was built (the interpreter counted `Op::Call`/`CallProt`)
  and tried on the window generator, `uvvm_util_rand` and `sc5_fifo_repack`: (1) a function is mostly called from compiled
  callers, which the interpreter's count does not see, and taking its native code away for the interpreter's window (as for a
  process) makes each of their calls materialise a frame: the window generator's trial of `get` cost 3 percent of its
  instructions and a trial that kept the callers' native code instead measured only the interpreter's callers; (2) the entries
  of one function vary with the phase of the run (`get`: 1.5 M a second early, 50 k late), and a trial in a late window took it
  back at 0.93 and doubled the run's instructions (385 G against 186 G); (3) the entry trial is erratic where it was wanted
  (`pos_of_leftmost`: verdicts 0.2 to 1.26 from the same code). Where the static rules mislead for a function, the fix is the
  rule (#573, the callee fix above), not a measurement of one function.
- **Checks.** Unit tests `calls_on_every_path_are_must_calls`, `a_trial_of_entries_takes_three_pairs`, `a_trial_decides_on_the_progress_ratio`,
  the band test (0.25 s), `activations_of_a_process_are_counted_while_a_trial_asks` (`yavs-interp/tests/sim.rs`); workspace tests, the
  `llvm` crate tests, the perf work-counter tests with and without `llvm`, and the 502-test differential corpus under
  `yavs-interp`, `yavs-auto`, `yavs-llvm` and `yavs-auto` with `--tier-heat=1,0,0` green.

## Stage entries of a design that loads units late (yavs, yavs-sem, yavs-elab, #671, 2026-10-24)

- **Decision: a stage entry (`low`, `init`, `ir`, `spec`, `ecall`) is keyed by the analysis' key, the stage's part and
  `Db::stage_extent` (the size of every arena plus a digest of the paths and content digests of every source file the `Db` holds, in load
  order), and no longer needs the extent of the analysis.** A component bound by default to an entity analysed after its user is not in the
  closure (the closure follows what analysis loaded), so elaboration loads it: `fletcher_bufferwriter_512` goes from 36 to 57 units, and
  its stage entries were never made or read (`Stages::key` compared the extent with the analysed one). The loading is deterministic (the
  analysed-unit cache replays id for id, `units.rs`), so a `Db` of the same extent made from the same files has the same ids at that
  point; a changed late unit changes the file digest, so its entries are not read. The library prefix entries (`lib*`, `Stages::level_key`)
  keep the old rule (`analysed() == extent`): they are shared between closures and cannot hold anything a late unit made.
- Measured (bench build, `--tier=interp`, steady state, `--stop-time=0fs`, instructions): `fletcher_bufferwriter_512` 497.1 M to 263.3 M,
  `fletcher_bufferreader` 364.9 M to 214.8 M; the whole runs 91.94 G to 91.57 G and 36.74 G to 36.50 G (the optimised copies are no longer
  made in every run). Test: `units_loaded_late_do_not_block_the_stage_entries` (`tests/cache.rs`, with an edit of the late unit).
- `hdlm_lfsr` (the lowered-library case) has no late units: its first runs find `a package body follows the prefix` (`vunit_lib.run_pkg`)
  and the closure's own entries are made on the second run as before; nothing to change there.

## llvm: address spaces across blocks, and enclosing frames from the view (#684, #685, 2026-10-24)

- Frame-mode code reset its facts about address registers to the preloaded ones at every block start, and an `Index` by a run-time position
  lost the space of its base, so a loop over a table of the instance data called `yavs_rt_host` for every element. `Known::InstAny` is
  an instance-data address at an unknown offset (an `Index` of an instance address keeps the space, as `Temp` and `Heap` did), and
  `regs::spaces` computes a forward dataflow over the blocks (meet: both sides agree, entry: the preloaded instance addresses) of the
  registers that hold an instance, temporary or heap address at a block start; `Gen::func` seeds `known` with them where it
  used to take only the preloaded ones. Sound because the facts are properties of the IR (a register whose definitions all give the
  space, on every path from the start), not of the code compiled; the entry points the interpreter jumps into see the same
  values. A function whose blocks times candidates exceed 2^27 bits keeps block-local facts (`ponytail:` in `spaces`).
- `TierFrame::up_slots`/`up_len` give the slots of the frames one and two static links up (0 length: a frame a native window holds elsewhere);
  frame-mode code reads and writes a variable of an enclosing subprogram in place (`Known::Up`), where it asked `yavs_rt_host` and the
  interpreter's object table and forgot the object again at the next host call (13 percent of `sc5_fifo_repack`). Native frames keep their
  own lookup (`Mem::nslots`). No alias tag: the bytes are the interpreter's own frame slots, reachable by other addresses too.

## Replayed entries: cells per page, decode ahead on reads in order (yavs-sem, #665, 2026-10-24)

- **Decision: the entries of a replayed unit that are decoded one by one (`SlotPage`) live in one box of `PAGE` cells made when the
  page is first read, not in a `Box` each; and a read that directly follows the last run decoded on that page decodes twice as many
  entries (up to the page).** Cold `yavs elab` of `neorv32_single_min`, whose lowering reads nearly every entry it replays, paid an
  allocation, a free and a decode context per entry (#424 took it from 0.634 G to 0.669 G instructions). Reads that follow each other
  directly are a lowering walking a package; the analysis of a file reads a twelfth of the entries here and there and stays at one.
  Tried: also counting a read up to 4, 8 or 64 entries past the last run as following it: the same elaboration (-0.1 %) and +0.5 to
  +4 % cold analysis (`hdl-modules-2008`), so only the entry directly after counts.
- Measured (bench build, instructions, children included): cold `elab` of `neorv32_single_min` 685.6 M to 674.7 M (-1.6 %; -0.8 % of it the
  cells, the rest the decode ahead); cold per-file analysis of the corpus manifests `hdl-modules-2008` -0.2 %, `osvvm-2008` +0.3 %,
  `poc-2008` +0.2 % (the python runner is in the totals); peak RSS of that elab 78.7 to 77.2 MB. **Open:** the elaboration is still about 4
  percent above the whole-run decode of before #424; the rest is per-read cost (`Page::at` checks two cells) and the per-entry decode
  context in the kinds that read here and there; recovering it without costing cold analysis needs a hint from the caller.

## Predecode: one scan per waveform, not one per function (yavs-interp, #683, 2026-10-24)

- **Decision: the peephole that fuses a waveform build, its `Copy` and its `Drive` (`pre.rs` `peephole`) stops looking for the `Drive`
  at the first `Drive` of that waveform, and `waveform_is_private` decides "nothing reads the registers after the `Drive`" by a scan of at
  most 256 instructions, else by a count of the reads of the register in the whole function (computed once per function),
  else by the full scan.** Before, every waveform of a large process searched on to the end of its block and scanned the rest of the
  function twice (`live_after`, which also cleared a flag array of the function's length per call): quadratic in the size of the process,
  which is why doubling the code of three processes of `olo_axi_master_full` cost 118 M instructions (#683). Nothing enters the
  range between the waveform's `Alloc` and its `Drive` but at the `Alloc` (no block start inside, checked), which defines the waveform
  register, so a register read only inside the range is dead after it. A later `Drive` of the same waveform is never fused (the
  waveform would be read four times), so ending the search at the first loses no fusion; the work counters and outputs are unchanged.
- Measured (bench build, `--tier=interp --stop-time=0fs`, steady state, instructions): `olo_axi_master_full` 703.5 M to 660.0 M (-6.2 %;
  `Pre::new` was 15.8 % of the run, 7.9 % after); a generated process of 1600 vector assignments in a row 64.7 M to 60.8 M. The
  remaining `live_after` callers in `Code::new` scan once per candidate too and were not measurable on these designs.

## Whole record signals: gather and scatter (yavs-ir, yavs-interp, yavs-lower, #677, 2026-10-24)

Shared layer: every tier and a second code generator see two instructions instead of a sequence per leaf. A record signal has one
kernel signal per leaf; a read of the whole record was a signal reference, a read, a field address and a store per leaf, a drive of
the whole record a load, a signal reference and a drive per leaf with the kernel's assignment path behind it: about 680 host
instructions per leaf for a read and a drive (the integer unit of the GRLIB LEON3 has 233 leaves).

- **`Inst::SigGather { dst, desc, tab, part }` reads every leaf of a record signal, or of an array of records, into the value at
  `dst`; `Inst::SigScatter { driver, src, desc, tab, ops, mode, has }` drives every leaf from the value at `src` with one waveform
  element at delay `after`, leaf `k` through driver slot `driver + k`, in leaf order.** Their meaning is that of the
  `SigRead`/`SigReadScalar` of each leaf stored into the value, and of the `Drive` of each leaf's waveform: values, delay mode, reject
  limit, errors and the order of the kernel's assignments are those of the per-leaf code. They say nothing about how the kernel keeps
  the leaves: this is the IR access path of a record signal, and a storage of record signals as one object (a contiguous layout,
  #686) can serve the same instructions without a change in lowering or in the tiers. `SigScatter` keeps its operands that are not
  always there in an operand list (`ops`: the delay, then the reject limit if `has & SCATTER_REJECT`, then a part if
  `has & SCATTER_PART`) so that `Inst` stays 32 bytes; every pass that moves operand lists goes through `Inst::list` and
  `Inst::list_mut` (the memoising pass had its own copy of that match and renamed the operands of a hoisted scatter wrongly,
  found by the reference simulators' own regression test `issue962`; corpus test `composite/sigrec_array_fn_width`).
- **The leaf table** (`yavs_ir::gather`, a byte constant): the type of the value and, per leaf in leaf order, the instance signal and
  how the leaf is stored (`LogicScalar`: a `std_ulogic`, BIT or BOOLEAN position with its code bias; `LogicVec`: plane blocks;
  `Scalar`: one element of a byte signal, a narrow integer sign extended; `Bytes`: an array of scalars as the kernel keeps them). The
  type of the value is a record, or an array (any nesting) of records, and a record may have arrays of records as fields. A leaf is a
  field of the innermost records and stands for that field of every instance of them: the instances in row-major order (the rule of
  `NetGraph::select`), logic leaves mirrored over all instances, so instance `j` of `T` instances (`T = outer * inner` for the
  `outer` elements of the whole signal's outermost array) starts at kernel element `(T - 1 - j) * len` for a logic leaf and `j * len`
  for another (`Leaf::kstart`). Where each instance lies in the value, and how many kernel elements it has, are not in the table: they
  follow from the type and the descriptor at `desc` (`gather::plan`, from `Ty::layout`), so a record whose field widths come from
  generics or constants of the design is served like a static one. A machine makes (or takes from the runtime's `Plans`, compared by the
  descriptor's bytes) the plan for a table and descriptor the first time and keeps it, with the instance signal of each leaf, in its own
  list; an instruction takes the plan out of the list and gives it back, so a gather costs no lock and no shared reference count (the
  runtime lock and the clone of the plan were taken at every instruction before; a record of 48 leaves in 32 instances: -1.3 percent
  instructions at one thread, the contention itself is not measurable on a loaded host).
- **Parts** (`Part { first, outer }`, `SigPart::part`): a gather or scatter works on a part of a signal of arrays of records when a name
  selects it: a field that is a record or an array of records (`r.a`, only the leaves below it are in the table, the type is the
  field's), an element (`s(i)`, `s(i, j)`, `r.a(i)`), a slice at the end (`s(i to j)`). `first` and `outer` are the first element and
  the number of elements of the outermost array of the part's value, within the whole signal's, as the walk of the steps counts them
  (`Fx::walk_steps`); none for a whole signal, which is `first` 0 and `outer` the value's element count. An element of a slice is one
  instance of it. The value's own descriptor `desc` is that of the part.
- **Shapes covered** (`gather::shape`, `Fx::table_for`): a record signal, or a signal with records in its type, as a whole, a field of
  it, or a part that an index or a slice selects when the part is a record or an array of records (`Fx::part_value_ok`; an array of
  arrays of records selected by an index would need the element counts of every level in `Part`). Not covered, and read and driven leaf
  by leaf as before: an array of arrays of a byte type as a field, a signal parameter, a drive by several waveform elements, a drive of
  a part whose leaves are not consecutive among the leaves of the process's driver, and a slice that is not the last step. A whole read
  goes through `read_obj`'s callers as before and the per-activation copy of #630 is filled by one `SigGather`.
- **A part is driven leaf by leaf when another drive of the process may override it** (`Fx::overridden_signals`: two targets of one
  signal in the process whose field paths are the same or one below the other, such as `port_req(i) <= a; if c then port_req(i).stb <= b`).
  The drive-deleting pass (`opt/drives.rs`, #667) drops the first of two drives on the paths where the second follows, which it can for
  single-leaf `Drive`s and not inside a `SigScatter` (its targets are known only at run time). Measured on `neorv32_single_min`
  (`port_req(i)`, a default and an override in a loop over five ports): as one scatter +5.9 percent instructions over the leaf-by-leaf code
  with the pass (more assignments: the default of the selected port is done as well), as leaf-by-leaf code -0.9 percent against the
  commit before. A whole record (no step) keeps the scatter as in the first step (#677). Corpus: `composite/sigrec_arrays` (the same
  signals driven again and again: the leaf-by-leaf code), `sigrec_arrays_scatter` (one drive of each part: the scatter), `sigrec_arrays_nested`,
  `sigrec_arrays_kinds` (every kind of leaf in the elements) and `sigrec_arrays_null` (arrays of records without an element).
- **Length check** (LRM 10.5.2.1): a record value driven onto a record signal has the signal's length in every array field; the check
  is `Fx::check_desc` in `drive_record`, ahead of the scatter and of the per-leaf code (it used to be only the outer dimensions of an
  array). A mismatch is the length error `value length N does not match <object> length M` with the name the reference simulators use for a record: the
  object a value is (`variable W`), else `target`; for an array it stays the target. Before the check a value of other field lengths in
  the same plane block was driven silently (`0101UUUU`); corpus tests `composite/sigrec_length_error`, `_part`, `_waves` (several waveform
  elements: the per-leaf code). The signal parameter case reads `parameter V` where the reference simulators say `constant V`.
- **Measured, not done**: drives with several waveform elements (0 of 515 drive sites over ten designs), signal parameters (9 sites;
  `osvvm_axi4_random` has one in 12.7 percent of the executed ops of its statements, about 1 percent of its instructions, which an
  elaboration three times that size hides), a per-leaf event stamp to skip the leaves a gather finds unchanged (the kernel's share is
  `assign_code_fast`/`commit_code` and does not depend on the instructions), a plane block for the `n` instances of a logic scalar leaf
  (`ponytail:` in `gather.rs`: one assignment per instance), and the single-field reads, which are one `SigVal` and one read each.
  The census (executed IR ops per statement and the share of the ops of the statements that still read or drive a record signal leaf
  by leaf): `fletcher_bufferwriter_512` 7.7 percent (`stage_r`, an array of records), `neorv32_single_min` 10.4 percent (`port_req(i)`),
  `grlib_leon3mp` 0.4 percent, the other designs below 0.1 percent.

## Window generator: no pass pipeline or forced compile changes the run; what is left is the accessor chain (#493, #679, #680, 2026-10-24)

Measurements in docs/STATUS.md "Window generator against sim1: where the time is now". Decisions (none changes code):

- **No `licm` or `gvn` in the lean pipeline.** Whole program under `--tier=llvm` with `YAVS_LLVM_PIPELINE`: lean 222.0 G instructions, with
  `loop-mssa(licm)` 226.4 G, with `gvn` and `licm` 225.5 G. The loop-invariant object look-ups of the codec loops are not hoisted because the
  calls and slow-path exits of the loop body may change the object table. A hoist needs the generator to know which instructions keep the
  table (`keeps_objects`), as a known object across a loop, not a general pass.
- **Compile policy facts for #680 and #673** (the policy is not changed here): forcing the 15 refused or interpreted functions leaves a warm run
  where it was; in a cold run the compile thread is 13 to 14 G instructions of the 55 to 70 G cold overhead and the rest is code that is not
  compiled yet, so lower compile cost per LLVM instruction (70 000 LLVM instructions after the passes in 45 timed compiles, a flat profile) helps the second
  half only through the latency.
- **The remaining distance to sim1 is the pointer-table and codec chain of the test runner**: 36 percent of the instructions of a warm run are
  75 M `get`, 40 M `set` and 13 M each of `length`, `unsafe_push`, `unsafe_pop` (call counts of #493), `get` at about 285 instructions per call. The four object look-ups and three dope tests of one `get` are the design of generational handles and bounds
  checked descriptors; the flat heap view (#655) and larger inlining (#493) were tried and lost. The option left is the one on #493 (native
  implementation of the test runner's pointer tables and queue), which is the maintainer's to decide.

## llvm: code in one arena of huge pages, entry dispatch as a tree, the hot edge falls through (yavs-llvm, #679, 2026-10-24)

- **Decision: on Linux x86_64 all compiled code lives in one contiguous region and is backed by huge pages** (`arena.rs`). ORC's
  default linker (JITLink) maps each module wherever the kernel puts it, in 4 KiB pages, and its allocator cannot be replaced through
  the C API; the object layer is now RuntimeDyld with our own section allocator (`LLVMOrcCreateRTDyldObjectLinkingLayerWithMCJITMemoryManagerLikeCallbacks`).
  Code sections are bump allocated in compile order (the hot functions of a run are next to each other) in a 2 MiB-aligned 1 GiB
  reservation that is read-write-execute (nothing to finalise, no instruction cache to flush on x86), data sections in a second zone of
  the same reservation (code and data within 2 GiB). A 2 MiB chunk is asked to be a transparent huge page (`MADV_HUGEPAGE`, then
  `MADV_COLLAPSE`) once it holds 256 KiB, so a small run keeps small pages and a big one has two or three iTLB entries for its code; a
  collapse that fails is tried again at twice the fill. It replaces `huge_code` (the whole-program module only) where it applies. Every
  other target, a failed reservation and `YAVS_LLVM_ARENA=0` keep LLJIT's default linker; `YAVS_LLVM_HUGE=0` keeps the arena without huge
  pages. Memory of dropped JITs stays in the arena (a `ponytail:` in the code). Test: `crates/yavs-llvm/tests/arena.rs`.
- **Decision: the entry switch of frame-mode code is a tree** (`Gen::entry_dispatch`): range compares on the sorted entry pcs with
  `switch` leaves of at most four cases. The unoptimised code generator (`Fast`, every function of 1 000 IR instructions or more) lowers a
  `switch` to a chain of `mov; sub; je; jmp` per case, and a process is entered at one of its resume pcs at every resume: 31 percent of the
  instructions executed inside the compiled `iu:comb` copy of the LEON3 design (1 357 entries) were that chain. Heat-mode L1i misses of
  `fletcher_bufferwriter_512` fell by a quarter with it.
- **Decision: `Gen::cond_br` with `likely` branches on the negation of a condition that is not a compare**, so the rare edge is the
  `jcc` and the hot path falls through. For `br (a & b), hot, cold` the code generator emits `jne hot; jmp cold` with `hot` the next
  block: a taken branch over a jump at every such guard (a bounds check of every memory access). Compares keep their form (the code
  generator fuses them and inverts the predicate itself). The 5 478 `jcc`-over-`jmp` pairs in the LEON3 `comb` copy are 1 879 now;
  branch misses fall 10 to 25 percent, instructions rise 0.1 to 0.4 percent (the `xor`).
- **Measured, not built (machine function splitting):** a made-up profile summary and an entry count per function make LLVM
  (`-enable-split-machine-functions -mfs-count-threshold`) put the blocks its branch weights call rare into a second section, and the
  arena can send a repeated section name to a cold zone. The hot part of the 24 000-instruction `comb` copy was 1.1 MB instead of
  3.7 MB, but L1i misses rose by a fifth (neorv32 47 to 60 M, reader 64 to 74 M, writer 127 to 144 M, heat mode) and cycles did not
  move: the rare edges are taken more often than 1:2000, and each trip lands in a far section. Not kept. The cold blocks of the
  slow paths already sit at the end of their function (`Cur::cold`), about a tenth of the code. `zext(trunc x)` for the 32-bit masks
  (a 2-byte `mov` instead of a 10-byte `movabs` and an `and`) was tried and dropped: instcombine turns it back into the mask.
- **What the machine code of a large process is made of** (LEON3 `comb` copy, 23 641 IR instructions, 3.7 MB, 158 bytes per IR
  instruction, `Fast`): 20.6 percent reloads from stack slots, 7.6 spills, 7.3 loads and 4.0 stores through pointers, 12.6 immediates
  (40 000 `movabs`: 20 000 are the callee address of a `callq *%rax`, 10 000 a 32-bit mask), 2.4 register copies, 45.5 everything else;
  the frame is 110 KB (one slot per value). (The small code model and the spills: next section.)
## llvm: small code model, calls through the GOT, blocks in execution order, narrow masks (yavs-llvm, #679, 2026-10-24)

- **Decision: with the code arena the code is generated for the small code model with position independent code** (`jit.rs` `model`;
  the arena is the precondition: code, stubs and data are within 2 GiB of each other). A call of a runtime callback is `call [rip+got]`
  (6 bytes) where it was `movabs rax, imm64; call rax` (12), globals and tables are RIP-relative. The callbacks are declared
  `nonlazybind` (`Gen::effects`, `Gen::declare`): without it the linker makes a PLT stub per callback and object and every call pays a
  second jump (measured: the stubs cost what the shorter call saves, cycles level). Without the arena (other targets,
  `YAVS_LLVM_ARENA=0`) and with `YAVS_LLVM_CODEMODEL=large` the JIT default is kept. The model is part of the key of the object cache
  (`target_id`). A section that does not fit the arena is a hard stop in the small model (`arena.rs` `overflow`: a 32-bit displacement
  to memory outside the arena would link to wrong code and the release linker does not check), where the large model maps it apart.
- **Decision: the blocks of a function are put in execution order before the cold ones go last** (`lower.rs` `layout`: from each block
  on to its likelier successor, by branch weights, else the earliest). Blocks were laid out in the order they were created, which puts
  the join block of a slow path before the fast path that leads to it: the unoptimised code generator emits blocks in layout order, so
  every guarded instruction cost `jne slow; jmp go` and a `jmp join` at the end of the fast path. The `jmp`s of the LEON3 `comb` copy
  fall from 39 937 to 24 616 (the rest are the cold paths' way back).
- **Decision: the low half of a load is a 4-byte load** (`jit.rs` `narrow_masks`, after the passes, unoptimised code generation only):
  `and (load i64 p), 0xffffffff` becomes `zext (load i32 p)`, a 2-instruction saving where the mask is a 10-byte `movabs` and an `and`
  (3 000 of the 26 000 `movabs` of the `comb` copy; the small model took 12 400, the callee addresses and constant addresses). It runs after the passes because a narrower load blocks the forwarding of a
  store to the same bytes (early-cse), which made the first version (at generation time) worse in the places where the stored value was
  a constant.
- **Measured, not kept:** the address of an access (`gep` of the base and the checked offset) made before the guard instead of after it,
  so that one value instead of two crosses the branch: 0.4 percent more code, cycles level or worse (the reloads of the three function
  arguments, the frame views, are a third of the stack traffic and a new block reloads them again, which this does not touch).
  Other optimiser pipelines on the same IR (`early-cse<memssa>`, a second round, `gvn`): code size level or larger.
- **Where the instructions of a large process go now** (LEON3 `comb` copy, 20 122 IR instructions, `Fast`): 448 257 machine
  instructions, 30 percent of them stack accesses (spills and reloads); a third of those are the reloads of the function's three
  arguments (the frame views) at the start of every block, 10.5 percent of all instructions. Registers cannot be reserved for them (the
  x86 target has no reserved-register switch) and the fast register allocator reloads what is live across a block at each block. **The
  deferred register stores (`regs.rs` `Live`, the suspect of #679) are not the source** for this process: they are off for a function
  whose liveness matrix is over `MAX_WORDS`, so every register is stored to the frame; where they are on (`exec_comb` of neorv32,
  4 531 IR instructions) they save: 29 248 stack accesses and 90 901 instructions against 33 308 and 95 577 with them off
  (`YAVS_NODEFER`, a temporary switch, not kept). Raising the cap made the 20 000-instruction process read outside a heap object (#710, see "llvm: a stepped definition is read back from the frame").
  The spills that are left are values of the guards themselves (a checked offset, an address) that cross the branch of each guard, one
  new block per guard: the levers left are fewer blocks per IR instruction (guards that share a slow path, redundant bounds checks of the
  temporary area: the same `len` and constant bounds are tested instruction after instruction) or a block-local allocator that keeps the
  three views in registers.

## IR optimiser round 13: repeated drives of one driver are done once (yavs-ir `opt/drives.rs`, #667, 2026-10-24)

Decision (landed): **a process's zero-delay drive that a later zero-delay drive of the same elements deletes is delayed down the control
flow and dropped on the paths that drive again** (partial dead code elimination, `opt/drives.rs`, after the walk and the loop motion of
processes and before `memo`; processes only, off under a debugger, `YAVS_NO_DRIVES=1` turns it off, a switch of the fingerprint). The
LRM reading (10.5.2.2): a new transaction deletes the transactions of its driver at or after its time, so with both drives at delay zero
and the second reached in the same activation the first never reaches the update phase, whatever the modes (transport, inertial) and
whatever pulse-rejection limit the second has (it only deletes more). Hence:

- **What is dropped and copied.** The drive and the instructions that build its waveform (the allocated block, the stores and copies into
  it, the temporaries and constants only they read; none of them can fail) move as a unit. A drive is a candidate when its waveform is one
  element whose first word is a stored constant 0, it has no `reject`, and its target is a `SigVal` of a constant length and an offset
  that is a constant or a register defined once (a loop over the elements of a signal array has its offsets in such registers: the
  second drive of an iteration uses the same register). Registers defined once that the unit reads stay where they are. Two drives are
  the same when instance signal, offset (the same constant or the same register) and length are, **without the driver slot**: a record
  assigned whole and then by field gets two slots of one kernel driver.
- **Where the copies go.** The blocks in which the drive is still delayed are found like a topological order (a block joins when every
  predecessor joined and passes the drive on, so cycles and joins entered by an undelayed path never join); a block that ends with the drive
  delayed into a block that did not join gets a copy at its end (a `CondBr` edge is split by a new block, a `Case` cannot be split, so
  its arms all take the drive or the copy goes in front of the `Case`). The copy goes before the first instruction that can see or change
  the driver's transactions: a drive onto elements that may overlap (different instance signals, or constant disjoint ranges, are passed),
  `DriveSlot`, `SigScatter` (a whole record driven by one instruction, #677: a barrier, and not recognised as the drive that
  deletes the earlier one, so a record default driven that way stays), `Disconnect`, `Force`, `Release`, `Driving`, `DrivingValue`, a call of a procedure, a protected method, an indirect or
  foreign call; before a wait it goes in front of the wait's `Stmt` (`static_wait` wants the marker first). Reports and function calls do not
  matter. At most six copies per drive, and the drive must be dropped on at least one path, so code never grows for nothing. Each path
  still does the drive exactly once or not at all, and never later than the instruction that could tell.
- **What stays.** Drives with a delay, a `reject` limit, several waveform elements, a target of a dynamic offset not held in a register
  defined once, a drive after which the same block drives other elements through a shared waveform block, a drive whose statement reads
  memory (a variable, a loaded value). A default followed by an override after a join (`if a then ..; end if; if b then x <= ..`) is
  a mixed join: the drive is done on the path that did not skip it (tail duplication would be needed).
- **Cost.** The pass runs on a process only when two drives of it have the same static target, over blocks that are cut into per-block
  event lists (drives, barriers) so that a drive costs one walk over the blocks it is still delayed in: about +0.1 G host instructions in
  start-up of `neorv32_dual_full` (4.5 G for 1 us of simulated time), none of it per activation.

Host instructions (`perf stat` `instructions:u`, `--threads 1`, bench builds, warm world, deterministic), main at da793761 before / after, G:

| design | interp | llvm |
|---|---|---|
| `neorv32_single_min` | 44.85 / 42.76 (-4.7 %) | 61.35 / 59.14 (-3.6 %) |
| `neorv32_dual_full` | 29.94 / 29.61 (-1.1 %) | 50.59 / 50.77 (+0.4 %) |
| `fletcher_bufferwriter_512` | 91.54 / 90.37 (-1.3 %) | 92.21 / 92.25 (0.0 %) |
| `fletcher_bufferreader` | 36.48 / 36.37 (-0.3 %) | 38.07 / 38.04 (-0.1 %) |
| `sc5_fifo_repack`, `sc5_timestamp`, `poc_stat_histogram` | 0.0 % | 0.0 % |
| `poc_cache_cpu` | 15.10 / 15.01 (-0.6 %) | 16.85 / 16.81 (-0.2 %) |
| `vai_window_gen_interlock` | 495.21 / 495.22 (0.0 %) | 236.45 / 236.43 (0.0 %) |
| `poc_arith_addw` | 235.89 / 235.89 (0.0 %) | 224.18 / 224.16 (0.0 %) |
| `grlib_leon3mp` | 891.98 / 892.53 (+0.1 %) | |

(`--tier=llvm` of `neorv32_dual_full`: the 0.4 percent is in the tier's compile and run mix, the interpreter does 1.1 percent less.)

Why less than the 2 to 4 percent #667 estimated: the 17 percent repeated drives it counted were by driver, not by element. In
`neorv32_single_min` 5.6 M of the 5.7 M repeats are one process (the bus gateway's `request`, a loop over five ports with ten field
drives each, enabled ports drive twice) and this pass removes them; `counter_increment` drives the elements of one vector driver one by
one (not repeats, the kernel's marks cannot tell), `exec_comb` keeps the repeats behind joins. After the pass 4.8 M of 16 M scalar
drives of `neorv32_single_min` still count as repeats by the kernel's marks, almost all of them element-wise drives of one vector driver.
The kernel side (a pending-drive merge, option 2 of #667) was not touched: a repeat there already costs one push.

## Specialised copies start after the declarative part (yavs-ir `opt.rs` `drop_declarations`, #678, 2026-10-24)

Decision (landed): **the optimised copy of a process for one instance (`Program::copy_of`) jumps from its entry straight to the
body; the blocks of the declarative part are one jump to themselves.** A copy takes over only between two activations of the shared
code (`Machine::specialise`: the process waits at its one wait statement in its root frame, the variables are in their slots, the
registers are the copy's own), so the declarative part never runs in it, and the optimiser's walk skips it anyway (`Opt::pass`:
blocks before `Func::body` are not walked). For a process of records with generic bounds it was the largest part of the copy: the
descriptors and initial values of the variables (the LEON3 integer unit's `comb`: 34 164 lines of IR dump in the copy, 24 486 without;
`spec_copy_ir` of 72 copies at 20 us of the LEON3 run 143 387 to 120 115, 16 percent; `neorv32_dual_full` 39 013 to 38 262).
Registers a frame starts with (defined once by a constant or an address) that the body reads keep their definition, moved to the
entry block, since the frame template is built from the definitions. No instruction counts of the interpreter change (`docs/STATUS.md`): the gain is code the tiers need not compile and the predecoder need not scan, and a copy under the memo's 6000
instructions.

What this does **not** do (#678 stays open): the 1 127 `LayoutOf` and 1 570 `Field` that remain in the copy of the LEON3 `comb` are
in the body. They read the descriptors of the process variables (`registers`, records with array fields whose bounds are generics)
through frame slots the declarative part filled, so the walk knows nothing of their contents, and the instance data being frozen
does not help. Folding them needs the declarative part evaluated for the instance: the frame slots it writes once and the body never
writes (the descriptors) as constant facts at the body entry and after each suspension, and a layout of a record type from a
descriptor whose words are known (`Ty::layout` takes the bytes). The interpreter's per-type layout memo and the `FieldOf` fusion
already make a `LayoutOf` plus `Field` cheap, so the gain is mostly in the compiled code (the address checks).

## Idle skip, round 4: concurrent conditional assignments, conditions of generics (yavs-sem, yavs-elab, #521, #678, 2026-10-24)

Round 2 and 3 gave the pure processes the idle skip, with the conditions their reads are under, but `t <= a when c else b;` (a
concurrent conditional assignment, a process with the sensitivity list of every signal of every arm) was left out, and a condition
of generics (`... when TRACE_PORT_EN else ...`, `... and NAP_EN else '0'`, as the CPU designs write them for features a configuration
leaves out) was no literal. `neorv32_single_min` woke those on every event of the signals their dead arms read (`cpu_trace`, `i_addr_i`
of every PMP region) and assigned the constant again.

- **`Db::pure_cond_reads`**: the reads of a concurrent conditional assignment (not guarded or postponed, delay default or `transport`,
  no `after`, every waveform a value, the expressions of the pure shapes of round 2). A waveform is read under its own condition and the
  negation of the earlier conditions that are one literal, a condition under those negations; the target's indices unconditionally.
  A condition the walk cannot express adds no literal (a weaker condition only makes the read count more often).
- **Conditions of constants** (`Db::const_cond`): a boolean expression of constants and generics declared outside the process (entity,
  architecture, block, generate, package), predefined operators only, with at least one name in it, is a literal (`GuardLit` with `pos`
  `GUARD_CONST`, `sig` the expression, `eq` whether it must be true). The elaborator evaluates it for the instance (`Walker::idle_parts`,
  `Model::eval`, as `if generate` does): a conjunction with a literal that is false never holds and goes, a true literal drops out of
  its conjunction, and a read with no conjunction left is no read, so an event on a signal only a dead arm reads arms nothing. A value
  that cannot be worked out leaves the literal in place (weaker).
- **`and` in a condition**: the literals of the parts that are literals are necessary for the condition to be true, so the body or arm
  is read only under them (`cond_lits(c, false)`); the negation for the later arms still needs the exact form (`cond_lits(c, true)`,
  one literal), since `not (a and b)` is not `not a`. This also lets `if sig_a = sig_b and GENERIC then` carry the generic.
- **Why it is the same run**: as in round 3, an activation does what the last one did when every signal the last one read, under the
  conditions it read it, is unchanged; a constant does not change, so an arm it rules out is never read. `YAVS_IDLE_SKIP=check`
  runs the activations the kernel would skip and fails when one assigns something else: `perf.rs` `idle_skip_changes_nothing` with the
  new `idle_when` (off, on, check equal; 586 of 2715 activations skipped), `tests/corpus/delta/idle_when.vhd` (blessed from the
  reference simulator), and the real-world designs `neorv32_single_min`, `neorv32_dual_full`, `poc_cache_cpu`, `fletcher_bufferreader`
  and `sc5_fifo_repack` in check mode print what main prints.
- **Not covered**: selected assignments (`with s select`), simple concurrent assignments (every read is unconditional), an
  assignment inside a generate whose condition is not a constant of the instance, conditions through a user function. `resize(INIT,
  q'length)` in PoC's `ffdre` (the other item of the hand-off) is a per-call computation inside a subprogram, not a read: it needs the
  call's constant arguments to reach the callee (#701).
- **Measured**: see "IR optimiser round 13" in `docs/STATUS.md`.



## Native ieee calls: one-word operands read and written in place (yavs-interp `ieee.rs`, `exec.rs`, yavs-lib `closure.rs`, #691, 2026-10-24)

- **Decision: `ieee::small` (the one-word view of an array operand of 1 to 64 elements without a metavalue) returns `Option`, not
  `R<Option>`, and reads the fat pair, the descriptor and the first plane block with `words` (three bounds-checked reads of the
  common spaces, temporary memory tested before the jump table, `Machine::mem_temp_first`). `None` for anything unusual (unreadable
  address, longer array, metavalue, a missing argument): the general path then reads the operand again and words the error, so the
  fast path has no error plumbing and no `Fault` in its return value.** `put_small` writes the 72 bytes of the result (fat pair,
  descriptor, one plane block) with one append to temporary memory (`Machine::temp_words`: the addresses follow from the start of
  the block) instead of `temp_alloc` (zero fill), two address additions and a `wr` through `obj_mut`. Results and warnings are
  unchanged (`ieee_native.rs` differential tests); the interpreter's op sequence and the work counters are unchanged.
- Per call, stepped instructions of `a + b` on two 9-element `unsigned` (`poc_arith_addw`, interpreter): `ieee::add` with its two
  operand reads, `fat_dim` and the result write 537 to 177; `to_integer` of one operand about 200 to 109. The rest of a native call
  is outside the function: about 60 instructions of `Op::CallIeee` (argument cells, function lookup, result and flag handling) and, on
  the compiled tier, about 190 of the callback layer (`yavs_rt_native`, `native_frame`) per call.
- **#691: `closure()` keeps a `HashSet` next to the ordered `Vec` of files (discovered, done) and of the files on the DFS stack
  (cycle check only when the file is on the stack).** The output order is unchanged. `olo_axi_master_full` at time 0 (interp, steady
  state): 623.4 M to 605.8 M instructions (-2.8 %).
- Not done, with the reason: (1) `std_match` and `to_hstring` as natives: none of the measured designs (`uvvm_util_rand`,
  `osvvm_axis_random`, `sc5_fifo_repack`) spends time in them, their time is in the libraries' own VHDL string code and `textio`
  natives. (2) The operand convention (a fat pair in temporary memory built by `NewDope` + `NewPair` from the `AllocReadVec` /
  `AllocCat` result: `NewPair` and `NewDope` are 10 percent of the ops of `poc_arith_addw`): a native taking data address and
  bounds in registers would save those ops and the two dependent reads in the native; that is an IR change (#697). The callback layer of the compiled tier is #698.

## Kernel round 20: a lean wide bus is finished in one pass; whole-word entries; what a skip of unchanged plane entries costs (yavs-kernel, #519, 2026-10-24)

Source of the numbers: `fletcher_bufferwriter_512` (512-bit buses through ~35 signals that each change about once a clock), bench
builds, `perf stat` on core 13, minimum of 5 alternating runs; instruction counts are exact, cycles are those of a host at load 20 to 30.

- **Where the kernel time of a wide bus went.** Counters (a temporary build, not kept): 34 M plane words change over the run
  (43 per delta cycle), 80% of them on a lean signal that keeps no codes and no activity stamps (`lean`, `codes` empty, `inactive`).
  `finish_pend` finished each of them through `finish_word` and `update_word`: ~250 instructions a changed word, of which the real
  work (xor, two blends, the block's event record) is ~40. The rest was per-word dispatch on flags that do not change within a bus
  (`lean`, `watched`, `fine`, `codes.is_empty()`), three `SmallVec` derefs and `Option` bounds checks per word, a sort check of the
  pending words, and a `cover` per word.
- **Decision:** `Signal::finish_words_lean` finishes every pending word of such a signal in one loop (slices taken once, flags tested
  once, the words in pending order unless a waiter of a part needs the runs in element order, one `cover` for the whole bus when
  only whole-signal waiters exist; the runs of a part-waited bus are noted per word as before). The cover of the words is the hull
  and the run counts add up, so `changed`, `nruns` (the `events` counter) and the wake-ups are those of the per-word path. A plane
  word entry (element 64k, 64 elements) in `commit_planes_run` is stored and marked whole, without the shift and mask paths.
  Test: `delta_paths::a_lean_wide_bus_wakes_and_changes_like_one_with_full_history` (changes, wake-ups of whole-bus, inner and
  block-crossing waiters, at 1 and 4 threads, against a bus that keeps every element's history).
- **Result:** writer 83.8 G to 77.1 G instructions (-8.2%), 30.1 G to 28.6 G cycles (-4.9%); the rest of the realbench set is
  unchanged in instructions (geometric mean 0.99, only the wide-vector tests move). `lean` stays an elaboration decision (round 18).
- **Measured and not taken: skipping a plane entry that holds the value already** (as `commit_code` does for one element, #420):
  at commit time 37% of the writer's plane entries (21 M of 58 M) and 49% of the reader's find the driving and effective value equal
  (`skip_ok` signals). Skipping them costs a block compare on every entry and saves little, because the `m == 0` path of
  `update_word` was already cheap (~35 instructions): writer +0.8% instructions, reader -1.6%, cycles level. Not kept.
- **Measured, not changed:** 6 M of the writer's 58 M plane entries repeat an entry of the same driver within one batch (10%; a
  default and its override): removing them at commit is worth ~0.7%; at the source that is #667 (IR optimiser).
- **Remaining kernel cost of a bus** (writer, per delta cycle of 100 K instructions): `finish_words_lean` 7.8%, `commit_planes_run` 4.0%,
  `finish_direct` 3.1%, `cycle` 3.0%, `assign_wide_fast` 2.7%, `commit_out` 2.7%, `Buf::write_to` 2.5%, `canonical_changed` 1.8% (a
  bus with a waiter on a part keeps a run list: ~8 runs a word with random data, sorted, merged and scanned by the waiters three
  times a cycle; ~4% of the writer in all). Exact run lists could only be replaced by a split of the signal at the waiters'
  boundaries (nexus splitting in the reference simulator), a larger change that also touches ports and observers: not started.
- **Measured and not taken (whole-picture round, same designs):** fat LTO with one codegen unit (writer +0.6% instructions, cycles
  level, reader level), `-C target-cpu=x86-64-v3` (writer -2.8% instructions and 1 to 2% cycles, reader level or +5% cycles: not
  shippable to generic hosts, not a clear win), `--threads 2/4/8` (6.5 s to 7.6/8.3/8.3 s), a single-pass `canonical_changed` and an
  aligned `Buf::write_to` (0.0% instructions each). Neither design compiles anything under `--tier=auto`; the layers of the
  interpreter side (dispatch 24 to 33%, record layout and copies 15 to 16%) and the shares of the kernel are in `docs/STATUS.md`
  "Fletcher writer and reader against the reference simulator".


## Facts the declarative part leaves in a process copy (yavs-ir `opt/decl.rs`, #678, 2026-10-24)

- **Decision**: a copy of a process (`copies.rs`) takes over between two activations, so what its declarative part made is in the frame
  and is a function of the instance's frozen data. The optimiser runs that part per copy on a small abstract machine (`Ev`: registers as
  unknown, constant, address, pointer into a tracked block or heap handle; frame bytes, temporary blocks and heap objects as known or
  unknown bytes; pure arithmetic, `Load`/`Store`/`Copy`/`MakeDope`/`DopeLen`/`Pos`/`LayoutOf`/`Field`/`Alloc`/`New`/`NewFill`/`Deref`,
  and the branches those decide) and reports the frame bytes, the words that hold an address and the words that point to the
  descriptor of a heap object (`[data, desc, handle]` of a record variable) that the body never writes. The walk (`Opt::wf`,
  `Opt::rdesc`) answers the body's loads of those: a scalar word is a constant, an address word is an `AddrOf`, a descriptor pointer is
  a register whose bytes are known, so `DopeLen`, `Pos`, `SlicePos`, `SubtypeCheck`, the loads of descriptor words, `LayoutOf` (every
  record or array type whose descriptor bytes are known, in shared code too: the pool's and the instance's descriptors) and `Field` fold.
  The facts are keyed by frame offset, not by register, so they hold after every suspension without being re-made.
- **Why it is sound**: a fact needs the word to be unwritten by the body (a direct store, copy or `MakeDope` to a frame address in a
  body block, or an address of it used other than by a load, anywhere: `frame_writes`) and every unmodelled instruction, call or store
  of a pointer into memory the machine cannot see makes what its pointer operands reach unknown for good (`Mem::poison`: the bytes stay
  unknown whatever is stored later); a branch or `Case` on an unknown value, an unknown word loaded out of memory that holds pointers, or
  more than 50 000 instructions give no facts. A check that fails would have stopped the run in initialisation, so after the switch every
  check passed. Reads of the bytes of a `for` generate's parameter are unknown, as before. The one fact taken on the language's word is
  that the descriptor of a heap object (the constraint of an object) is never written after the object is made, and that of an array
  variable in the frame (`in_descriptor`) likewise; a pointer to either may go anywhere.
- **Not done**: a branch on a package constant (`AddrSpace::Pkg`) stopped the machine (6 of the 16 big processes of the GRLIB LEON3 system
  have one): done since, see "Package constants in specialised copies" (#708). Calls are unknown, so a variable initialised by a function
  call has no fact (its descriptor does when the bounds are known).
- **Measured**: `docs/STATUS.md` "Facts the declarative part leaves in a process copy". `spec_record_var` (`crates/yavs/tests/perf.rs`) is
  the work-counter case (239 313 ops with the shared code, 151 530 with copies), `crates/yavs-ir/tests/copies.rs` the unit cases.


## Light natives in compiled code: a callback of their own (yavs-llvm, yavs-interp, #698, 2026-10-24)

Counted with `tools/stepcount` on `poc_arith_addw` under `--tier=llvm` (1078 calls of a light `ieee` native in a window of
3 M instructions): `TierHost::native_frame` 113 instructions and `yavs_rt_native` 68 per call, as much as the native (`add`
115, `to_integer` 45) it surrounds.

- **A `CallNative` of a light native (`yavs_interp::light_native`, the set `Op::CallIeee` already runs: an `ieee` native,
  few cells, no signal, not `uniform`) calls `yavs_rt_native_light`** with the arguments and results of `yavs_rt_native`, and
  `TierHost::native_light` on the host. Such a native allocates only temporary memory and never collects, so, as for
  `Op::CallIeee`: no collection check (`reclaim_pending`), no allocation-mode swap of the frame, the heap epoch is not
  read (`Mem::forget_end` replaces `forget_since`: only volatile objects can be stale), and the view update stores
  only the temporary pointer, length and capacity (no roots, base or epoch change). Kept: the frame windows of native
  frames (an operand may live in a slot of a native frame), the `ieee_fallback`/`ieee_stop` handling, the package
  initialisation and elaboration-report decline, `catch_unwind`. Why a second symbol and not a flag: no branch in the shared
  callback, and the code generator picks it at compile time with `light_native`. Shared with a second code generator:
  `TierHost::native_light` and the argument convention of `yavs_rt_native`.
- **Measured:** 113 + 68 to 82 + 73 instructions per call (-26, the rest is spread over many small pieces: two prologues and
  epilogues, the table lookups, the result tuple and the argument slice); instructions of `poc_arith_addw` -1.2 percent
  under `--tier=llvm`, -0.8 percent on the window generator. The callback layer is now about as large as the native;
  removing it needs the native inline in compiled code (#698 idea 2).

## Idle skip, round 5: variables assigned on every path, fields one by one, debug code under a false generic (yavs-sem, yavs-elab, #521, #519, 2026-10-24)

Found with the whole-picture profile of `fletcher_bufferwriter_512`: its hottest process (`ctrl_comb_proc` of the reshaper, 11.6% of
the run, a finite-state machine in the two-process style) was not pure for the idle skip, so it ran at every wake, 139 K of them
(it is woken by `clk`, which it names "for debugging", and by every input); each skipped activation saves the 13 drives and the bus
updates it would make (41 K instructions an activation). Doing it by hand in the source (assign the helper variable first, drop the
debug `report`s) took the writer from 74.2 G to 68.5 G instructions (-7.7%). Three limits of the purity analysis were the cause, and are
lifted:

- **Definite assignment, not "first statement".** A variable of the process is allowed when it is assigned on every path before it is
  mentioned otherwise (`Db::vars_assigned_first`, now `defined_before_use`): a flow analysis over `if`, `case` (what all arms assign),
  loops (nothing, they may run no iteration) and sequences, where round 2 asked for a top-level `v := ...;` first in the statement list.
  `variable t : ...; if c then t := a; x <= t; end if;` and `if a then v := 1; else v := 2; end if; y <= v;` are pure now.
- **Fields of a record variable one by one.** `v.f := ...;` (a chain of field selections from a variable) assigns that field and a
  mention of `v.f.g` is met by an assignment of `v.f.g` or of a prefix of it; a mention of the whole `v` needs a whole assignment. A
  partial update `v.f(3) := ...` is a mention (it reads the rest). `vo.a := ...; vo.b := ...;` followed by reads of single fields is pure.
- **Debug code under a condition of generics.** An `if` branch (condition and body) that is not pure, but can only run where a
  boolean of constants and generics holds (`if rising_edge(clk) and DEBUG then report ...`), is no longer a failure but a guard under
  which the process is not pure (`PureRead` with `lsp` `None`): the elaborator works the literals out per instance (as it does for
  guarded reads) and the process is pure in every instance where one is false. Where it is true (`DEBUG => true`) the process runs at
  every wake as before. The condition itself must have no effect (`Db::effect_free`: no impure function, no protected method), since
  `f(x) and DEBUG` evaluates `f` first (the reference simulator folds `f(x) and false` and drops the call, a deviation from the LRM
  (`tests/discrepancies/short-circuit-constant-false-effect`), which is why that case is only in `perf.rs`, not in the corpus).
- **Why it is the same run.** The rule of round 2 holds unchanged: an activation does what the last one did when every signal it
  reads is unchanged and no variable carries a value from one activation to the next; the three changes only accept more processes
  that satisfy it. `YAVS_IDLE_SKIP=check` runs every skipped activation and fails when it assigns something else: the 511 tests of the
  differential corpus in check mode, the new `delta/idle_vars` (blessed from the reference simulator; five pure shapes, three that
  must run every time, and an instance with the debug report on and one off), `perf.rs` `idle_vars` (off, on and check print the same,
  an impure condition reports at every wake), and the 42 real-world designs in check mode.
- **Result** (bench builds, exact instructions, interpreter): `fletcher_bufferwriter_512` 74.17 G to 69.0 G (0.93), `poc_fifo_ic_assembly`
  0.82, `fletcher_bufferwriter_prim32` 0.91, `neorv32_dual_full` 0.97, `neorv32_single_cache` 0.98, `neorv32_single_min` 0.99, the rest of the
  42 within 0.5%.
- **Left (blocks of the other hot processes of the writer):** a `report`/`assert` that is not under a condition of generics
  (`last_stage_proc`, `command_proc`: output has to come at every activation), a variable field read before it is assigned on some path
  (`vo` of a BusWriteBuffer process, `stage_v` of a pipeline barrel: they keep a value, so they are not pure), procedure calls.

## `--tier=auto` decisions follow the looks of a sampler thread, prices are fixed (yavs-llvm, #670, #673, #680, #681, 2026-10-24)

- **Causes found** (a fresh world per sample, `--threads 1`, instructions of the whole process tree, traces with `YAVS_AUTO_TRACE=1`):
  1. The compile cost was a rate learned from the compiles of the run (`us_per_instr`: half the old value plus the wall time per IR
     instruction of the last O2 compile that did not hit the cache). It started at 800, fell with the first compiles of a cold run and stayed
     at 800 in a warm run (hits teach nothing), and it priced every function of a run with one number for native-frame code, frame-mode code,
     full-pipeline compute loops and `Fast` code alike. What a function costs, so when it is compiled, depended on the order and the wall
     times of the compiles before it: `sc5_fifo_repack` ended at 35 G or 47 G according to whether `p_in` and the clocked processes were
     priced under their threshold when they crossed it (#673, #670).
  2. The sampler ran on the compile thread, so while it compiled (10 ms to 600 ms) or measured a trial (250 ms) it saw nothing, and
     the first look after that gave the whole gap to the one function it happened to catch: a function could be "seen" for 400 ms in a
     sample. Which functions crossed their thresholds, and in which order, depended on how long the compile thread had been busy.
  3. The clock was wall time. With the compile thread on the simulation's core (below) or a loaded host it counted the time the
     simulation lost to the compile as time spent in the function.
  4. `Worker::cost` priced `Fast` code like the rest (800 per IR instruction, real 70 to 130, #680).
  5. The compile thread moved to every CPU outside the mask (#681), so the compile of a run overlapped the simulation on another run's core.
  6. Smaller: a `HashSet` iterated in an order that differs in every process decided the order of callees in `tree`.
- **Decision: a sampler thread (`yavs-sampler`, `sampler_loop`) sends looks (the CPU time of the simulation threads since the last look and
  what they run) in batches of 8 to the compile thread, which takes one batch at a time and decides after each (`Worker::see`).** The
  sampler runs during compiles and sends nothing during a trial (the swaps of a trial would show as the code running). So the sequence of
  decisions depends on the sequence of looks and nothing else; no compile or trial duration reaches them. The clock is the CPU time of the
  simulation threads (process less the sampler's and the compile thread's own clocks), wall time where there is no such clock (not Linux).
  Why not the one thread of decisions.md "Tier-up (`--tier=auto`)": its reason (CPU) holds, the sampler wakes once per period as the compile
  thread did.
- **Decision: prices are constants.** `US_PER_INSTR` (800) is the price the learned rate started at and stayed near in cold runs with the
  compile thread on the simulation's core (cold compiles of the 5 designs and 19 more of tools/realbench, per-function CPU: frame-mode
  code 200 to 450 and native-frame code 350 to 1100 per IR instruction, rising with the size of the function; wall time on a shared core is
  twice that). `Fast` code has its own, 110, in the budget (`Worker::price`), a module linked from the cache 20 (`US_PER_INSTR_HIT`).
  `spent` is charged by the model, never by the time a compile took. The payback rule (`PAYBACK*`, the order of candidates) keeps 800 for
  `Fast` code too (`payback_cost`): cheap prices for it made the 2 133 IR process of the window generator compile after 0.47 s instead of never
  in a cold run and cost that design 8 to 16 percent of its cold wall time; whether big processes compile is the user's decision
  (#679, 2026-10-24: allowed, no size cap yet), not a side effect of #680.
- **Tried and not taken:** per-class and per-module prices from the measurements (`lean`, `full`, `Fast`, frame and native: 210/470, 450/720,
  110/220 CPU per IR instruction, also doubled to wall time on a shared core): more small functions compile (the tiny ones cost 3 to 9
  ms each), cold wall time of the window generator +14 to +27 percent against the learned rate, +5 percent against the flat 800. The
  benefit of compiling a small native function is lower than the payback factors assume; the flat 800 happens to stand for both its cost
  and its low benefit. A wall clock for the looks instead of CPU time: no better. Budget prices scaled 0.3 to 0.7: within 2 percent of each other, 1.0: 4 percent more
  instructions.
- **Affinity (#681, user, 2026-10-24):** the compile thread keeps the CPU mask of the process. Under single-core pinning it shares the core:
  auto walls of `tools/bench` and `tools/realbench` include the compile CPU time from here on (`tools/bench/AGENTS.md`).
- **What is still timing dependent** (measured, see docs/STATUS.md "`--tier=auto` decisions"): (a) the CPU speed of the host itself: the same
  simulation costs 1.8 to 3.4 s of CPU on a busy host (SMT neighbours, frequency), and a function whose seen time sits near its
  threshold flips (`rand_next` of `sc5_fifo_repack`, `push`, `check_axi_stream` of the window generator); (b) through the compilation
  cache: what the first run compiled is linked early in the next ones, so a flip in the first run is amplified (window generator warm: 178
  to 198 G with the same code); (c) the compile that is still running when the run ends is lost; (d) trial verdicts near their keep line. The
  way to remove (a) and so (b) is a work clock (instructions retired of the simulation threads, `perf_event_open`, with the CPU time where
  it is denied); it is not built: Linux only, a counter per simulation thread registered by the interpreter, and a warning where it is
  refused. Open question for the user.
- Tests: `what_is_hot_depends_on_the_looks_only`, `a_look_shares_its_time_between_the_running_functions` (`auto.rs`),
  `compile_thread_keeps_the_affinity_of_the_process` (`tests/tier.rs`); the 510-test differential corpus under `yavs-auto`, with
  `--tier-heat=1,0,0` and with 4 threads, passes.

## IR optimiser round 14: a repeated `to_integer` is made once, the second call replays only when it can warn (yavs-ir `opt/sink/calls.rs`, `opt/sink.rs`, #520, 2026-10-24)

User decision 2026-10-24 (#520): share the `to_integer` of a check's condition and message. `simAssertion(to_integer(unsigned(a & b)) = x,
"..." & image(to_integer(unsigned(a & b))))` converts one value twice; the lowering builds each argument as a signal read, a
concatenation and a fat pair of its own, so the optimiser's value numbering (which forgets memory facts at every call) never sees one
value. After the message was sunk (round 16) the second chain and call were what remained of the message on the passing path.

- **Decision: after `sink::run` has moved the message, `Sink::share_calls` finds, in one block, a later `to_integer (unsigned)` whose
  argument is the same value as an earlier one's, and turns the later call into a branch on the earlier call's result:
  `r1 = call(x); ...; CondBr r1, FAST, R`, `FAST: r2 = r1`, `R: r2 = call(x')` (the original second call), both to `J`, the rest
  of the block. A second `sink` run then moves what built the second argument into `R`. Why: the native returns 0 exactly for a
  null or a metavalue argument (both with a warning) and for the value 0, so a first result other than 0 proves that the second call
  is silent and returns the same; for 0 the real second call runs and warns or not by itself, so message text, count, order and
  time of every warning are those of the unshared code.** The guard is the result itself (`CondBr` tests `!= 0`): no `LNum`, no new
  instruction, no change to the natives. Only `ieee::Op::ToInteger { signed: false }` qualifies (`numeric_bit` included: it warns
  only for a null vector); `signed` warns with a result other than 0 for a one-element `'1'` (`TO_SIGNED: vector truncated`, result
  -1), so it stays as it is (a test checks that).
- **"The same value" is value numbering of one block (`calls.rs`, `Vn`).** Scalars are numbered by operation and operands (a register
  the function defines once by a constant or an address is that constant or address, so `r13` and `r40` that both hold 1 are one
  number); a block `Alloc` makes is numbered by what is written into it, in order (stores, `MakeDope`, `SigRead`, `Cat`), compared
  by structure at the call (nested blocks included, depth 6). A call qualifies only when its argument is a fat pair this block
  allocated. Everything that may change what was numbered forgets it and the first calls with it: a statement marker, a suspension, a
  call other than `'image` and the light `ieee` natives (they write only a block of their own, and warn, which the replay keeps in
  order), a store through a pointer that is not a numbered block, a copy, fill, free or any instruction `writes_nothing` does not
  list. Signal reads of one block are of one value because nothing in the block can end an activation.
- `sink.rs` changes that make the second chain movable: `SigVal`, `SigReadScalar` and `SigRead` into a block the moved set made
  and `Cat` of logic vectors are movable like the string `Cat`; a moved `SigVal` takes two fresh registers (a signal reference is
  two cells); a branch made by `calls.rs` (a "replay" block) is looked at although no string `Cat` is in front of it and moves from
  3 instructions on (the call is rare); a result the target overwrites is not live out of it (the first version refused to move a
  definition of the register the replayed call writes). Moved code writes fresh registers as before.
- Tests (`crates/yavs/tests/sink.rs`, with the optimiser and without, interp and llvm): the same value in condition and message with
  every check passing and failing; a vector with `U` (both calls warn, twice per check); a vector of zeros (the replay runs and is
  silent); a null vector; another warning between the two calls; three calls of one value; a signed vector (left alone); the signal
  changing between statements; and the dump has the replay. 1300 random programs of checks over vectors of 0 to 8 elements with
  `0 1 U X Z W L H -`, concatenations, slices, `signed` and `unsigned`, signal changes between checks, compared with the build
  without the optimiser on the interpreter (and 350 under `--tier=llvm`): identical output.
- Not done: signed `to_integer` (a guard on the length and the sign bit would do); other natives that warn for a metavalue
  (`+`, `-` and compares of `numeric_std` return a metavalue result, so their result cannot be the guard); sharing across blocks
  (a call in a loop body and one after it); sharing the first chain with chains that other code built (`/ ` of numeric_std etc.).
- **Measured** (`poc_arith_addw`, bench builds, core 10): instructions 0.92 in the interpreter and under `--tier=auto`, cycles 0.89
  (window of 300 us), wall 0.95 (full length); 42 tests of `tools/realbench`, interpreter and compiled: instructions geometric mean
  1.00. Details in `docs/STATUS.md` "IR optimiser round 14: a repeated `to_integer` is made once".
- **Coordination (#697, #520):** the pass takes a native with one argument that is a fat pair the block allocated and stored
  the data address and the descriptor into. A native whose arguments are in registers (#697) is not shared until
  `Sink::shareable` and the argument test in `find_pairs` know that form; nothing else in the pass depends on the pair layout.


## llvm: a stepped definition is read back from the frame (yavs-llvm, #710, 2026-10-24)

- **Root cause of #710.** The slow path of a `LayoutOf` with a frame cache is generated at the start of the instruction, with the
  deferred registers (`Defer::pend`) as they are then. After the interpreter stepped the instruction, `layout_keep` reads the layout it made
  through `Gen::ld(dst)`, which returns a value deferred for an EARLIER definition of the same register when there is one: LEON3 reuses a
  register for a descriptor and later for the layout, so the layout words were copied from the descriptor's address into the kept
  slot, every later `Field` of that type used wrong offsets and the interpreter read outside the object. Not a limit of the deferral
  itself: it needs a register that a loop defines (deferred) and uses, and that a cached `LayoutOf` then redefines; the cap hid it for the
  large processes only. **Rule: code generated after a step in a slow path must not read a register the stepped instruction defines
  through `ld`/`pend`; `layout_keep` takes the deferred value out for its length and puts it back** (the fast path redefines the register
  with `st`, which clears it, and may still read the old value as an input). The test is `cached_layout_replaces_a_deferred_value_of_its_register`
  (`crates/yavs-llvm/tests/tier.rs`).

## Memo of pure calls (yavs-ir `memo_fn.rs`, yavs-interp `pmemo.rs`, #701, 2026-10-24)

Found with the PoC cache testbench: `ffdre(q, d, rst, INIT)` and `mux(sel, a, b)` on 2-bit vectors cost 12 000 to 20 000 instructions a call
(`resize(INIT, q'length)` alone is 60 percent of `ffdre`) and are called with the same few values over and over. Decision (user,
2026-10-24, option 3 of #701): **the interpreter keeps a runtime memo of the pure calls it ran, keyed by the bytes of their arguments, and
answers a repeat from it.** Options 1 (a function copy per constant facts of its arguments) and 2 (a once-per-site memo of `resize`) were
not taken: the first needs a second kind of copy and reserved slots and only folds the lengths, the second a persistent place for result bytes.
The reference simulator folds a pure call with constant arguments once, before the run; this memo also serves arguments that vary but repeat
(register state), which no folding can.

- **What qualifies** (`Program::memo_sig`, once per function, on the lowered code so the answer does not depend on which function ran first):
  a `pure` function (`FuncFlags::PURE`) that is not nested, cannot suspend, touches no shared state and no coverage, has no signal
  parameter, takes only `in` scalars of integer, physical, enumeration or real type and `in` arrays of those (descriptor and elements are the
  key, `Param::arr` names the array type), and returns one of those; whose instructions, and those of every function it calls, are
  computations, checks, frame, constant-pool and package-constant reads (`Inst::Alloc`, `NewFrame` and local heap objects included) with no
  signal, instance-data, enclosing-frame, shared-variable, clock, file, foreign, indirect or protected access, calling only natives from
  `native_is_pure` (the list package-initialisation caching uses); and that is worth the key: a loop, a call or 120 instructions (a
  function of a few instructions runs faster than its key is built).
- **Exactness.** The key holds every byte the call can read from its arguments (the argument words; for an array its descriptor, so bounds
  and direction count, and its element bytes, whole blocks for bit-planes) and is compared whole on a hit: the hash only picks the slot.
  Everything else the body reads is constant for the run (`Pkg` data are written during initialisation only, `Machine::wr` refuses
  later). A hit hands out a copy (a fat pair in temporary memory for an array), never the stored bytes. **Reports**: a `Report` in the
  body is allowed; `State::reports` counts the reports made (`Inst::Report`, `ieee::report`, before the severity filter), a call during which
  it moved is not kept, so a function that reports reports on every call, and a call that fails or stops is never kept.
- **Where.** Only simulation code in the interpreter: not elaboration-time evaluation (`ctx` is `None`), not under a debugger or with
  coverage counters, and `YAVS_NO_PMEMO=1` turns it off. The predecoder makes `Op::CallM` of an `Inst::Call` whose callee qualifies and runs
  every other call as before (one macro, `call_arm!`, is both arms, so an ordinary call has no test); `Machine::step` and `call_regs`
  (calls from compiled code that the host steps) ask the memo too. A miss runs the call to its end from inside the call
  (`Machine::call_to_end`: `exec` with the frame count as base), at most `NEST` = 8 deep (a recursive function memoises its top levels, the
  rest run as usual and the native stack stays small).
- **Bounds.** Per process and function a direct-mapped table of 32 entries that doubles up to 256 while keys evict each other; a key or
  result over 1 KiB is not kept. The first 64 calls of a function in a process decide: fewer than one in eight repeating switches it off
  for that process, and for every process when three have (`Pre::memo_off`); a function that is off costs a flag test in the call. Per
  process and without locks, so hits and counters are the same at every thread count. `YAVS_COUNTERS` shows `pmemo_hits` and `pmemo_runs`.
- **Not done.** A function the tier compiled for native frames is called directly from compiled code and bypasses the memo (making it
  step the call would slow a function whose calls do not repeat, and the tier decides statically); the memo applies to every call the
  interpreter makes, also from compiled callers that call interpreted code. Records, access types, signal parameters and out parameters
  are not keyed. A table shared by the processes of one design unit would save the first call of each (about 4 percent of the PoC run is
  misses).
- **Measured** (bench builds, instructions of `--tier=interp`, `perf stat`): numbers in `docs/STATUS.md` "Memo of pure calls".
  Work-counter case `pure_memo` (`crates/yavs/tests/perf.rs`), correctness case `crates/yavs/tests/pmemo.rs` (bit-identical runs with
  and without the memo at 1 and 4 threads and with early specialised copies, a function that reports on every call).

## Package constants in specialised copies (yavs-ir `copies.rs`, `opt/decl.rs`, #708, 2026-10-24)

Decision (user, 2026-10-24): **package constant values may become facts of process copies.** The six large processes of the GRLIB LEON3 system
that branch on a package constant (`AddrSpace::Pkg`) in their declarative part had no frame facts because the abstract machine of #678 stopped
at the unknown load. Package data are written during package initialisation only (`Machine::wr` refuses a store later), and a copy is made
while the design runs, so the machine now reads them (`Opt::const_read` of `AddrSpace::Pkg`, only for the packages the process's declarative
part names: `Program::pkg_deps`, the `AddrOf Pkg` of the blocks before the body).
- **The key.** The packages' values are not instance data, and an initialiser may read the environment, so the same design can make other
  values in another run. The data of the packages a copy depends on come from a `PkgSource` (`copy_of_pkgs`; the runtime's package data), a
  digest of each (made once per package) is part of the copy's key, of `copy_tag` (compiled code of a copy is named and cached by it) and of
  the stored `CopyBlob` of the stage cache (`spec`): a changed package makes, and reads, another copy. `Copy` keeps the data it was made with
  for the lazy optimisation; `copy_of` is `copy_of_pkgs` without package data (no package fact, as before).
- **Folds also in the body** of a copy for a read of the same packages' constants, into integer registers only (the rule for instance data).
- **Measured**: `docs/STATUS.md` "Package constants in specialised copies"; unit case `a_copy_knows_what_a_package_constant_made`
  (`crates/yavs-ir/tests/copies.rs`).

## Native ieee calls: array operands as values (#697, 2026-10-24)

- **Decision (user, 2026-10-24: "IR-level operand convention for native calls"): a `CallNative` of a native `yavs.ieee.*` takes each array
  argument as four registers `data, left, right, dir` (`yavs_ir::ieee::ARRAY_REGS`: the address of the first plane block, the bounds of
  the one dimension and its direction as the descriptor stores them) and each scalar as one. The argument list is the formals in order, so
  `a + b` of two vectors has 8 registers, `resize (a, n)` 5. The verifier checks the register count from the key of the symbol
  (`ieee::arg_regs`; the `@<function id>` of a native that may give the call back is ignored); every other native keeps the fat pair.**
  Why: the fat pair `[data, dope]` in temporary memory cost `NewPair` + `FatRoot` per operand (the root is recorded until the statement
  ends: the interpreter keeps the temporary and `Stmt` does more work while one is recorded, and compiled code made a host callback for it),
  `NewDope` where lowering had to build the descriptor, and three dependent reads in the native (pair, descriptor, planes). With values,
  nothing is built and the native reads only the planes; the registers are live at the call, so the heap objects an operand points into stay
  pinned as for any `Ptr` register and no `TempRoot` is needed.
- Lowering (`expr.rs` `Conv::{Body, Fat, Flat}`, `Fx::array_cells`): `Flat` for the `ieee` natives and the user reduction idiom (which runs
  the `Reduce` native), `Fat` for every other native, `Body` for subprograms. The bounds of an operand are the registers its descriptor was
  made from when the `MakeDope` is in the same block (`Fx::made_dopes`, cleared by a `Move`, so a part register is never stale), else three
  loads from the descriptor (`dope_parts`). A descriptor that only the native read was a block that only a `MakeDope` writes: the existing
  dead-code sweep (`opt.rs` `dce`) removes the `Alloc` and the `MakeDope` once the call no longer reads them, so lowering does not have to
  know whether something else reads the descriptor.
- The old path is not kept next to the new one for "unusual" operands: an operand without a known descriptor (a parameter, a variable, the
  result of another native, an individually associated or converted formal) takes the three loads, so there is one convention to implement in
  every tier and in a Cranelift backend; a call that gives the body back (`Syn`, `Str`, `Div`, `NumGate` natives) makes the descriptors the
  body needs in temporary memory (`Machine::ieee_body`).
- Interpreter: `Op::CallIeee` copies up to `pre::IEEE_CELLS` (8) cells (was 4). `ieee::small` reads only the planes (it took the pair and
  the descriptor too), `dim_at` replaces `fat_dim`; the positions after an array operand are `A1` (4). `small_fat` is the fat-pair form the
  implicit `to_string`-family natives still use. The compiled tier needed nothing: it passes the cells of the registers to the same functions
  (`native_direct`, `step`), and a call of at most 8 registers still qualifies for the direct form.
- Optimiser: a `Load` of a word of a block that a `MakeDope` wrote on the path is the register the word was made from (`Key::Part`,
  `Opt::iat` finds the block and offset of an `Index` at a constant offset, killed by stores and calls like the other memory facts, `opt.rs`
  `forward`). Without it the loads that lowering emits for the bounds of a concatenation (a `Select` between two descriptors folded away)
  stayed in the code; with it the bounds of a freshly made vector are the registers or constants they were made from.
- The sharing of a repeated `to_integer` (#520, `opt/sink/calls.rs`) keys a call on `native, canon(data), left, right, dir` instead of the one
  fat-pair argument, so it still finds the pairs. The dump-shape test in `tests/sink.rs` does not tell (it passed with the sharing lost);
  the work-counter design `ieee_share` (ceiling 3630 ops, 3920 without the sharing, 4720 before both) does.
- Results stay a fat pair (the natives that give the call back return what the VHDL body does, somewhere else). Tried and dropped: a
  lowering that takes the data and descriptor addresses of the result block (`dst + 40`, `dst + 16`, the contiguous layout `put` writes)
  instead of loading them: the op count is the same (two `IndexImm` for `Load` + `LoadOff`) and the instruction count moved by -0.5 percent
  only, for a layout contract between lowering and the natives. A result chained into the next native (`to_integer (a + b)`) now costs the
  three loads of its descriptor (the bounds a fat pair let the native read); a three-word load op or natives returning the bounds in
  registers would remove them, not done (see STATUS.md).
- Not done: the result written into a buffer of the caller (the native allocates and appends its result block in one go, 15 to 25
  instructions with `put_small`, and the size is dynamic for most natives, so the caller would need arithmetic to size it), natives
  returning the bounds in registers (needs a multi-register result in the IR), and a three-word load op for the bounds of a result that is
  chained into the next native.
- Measured, `poc_arith_addw`, interpreter, first 300 us, `op-census` bench build before and after: ops 200.1 M to 185.8 M (`NewPair`
  11.9 M, `NewDope` 7.8 M and `FatRoot` 2.3 M gone; 5.2 M `Load`, 1.8 M `LoadOff`, 2.8 M `IndexImm` added), instructions 281.6 G to 261.1 G
  (-7.3 percent), output identical. Whole test, bench build, warm cache, core 14: interpreter instructions 220.5 G to 202.7 G (-8.1 percent),
  cycles (median of 3) 67.0 G to 61.7 G (-7.9 percent); `--tier=auto` instructions 150.4 G to 139.2 G (-7.5 percent), cycles 54.6 G to
  48.9 G; `realbench --compare` wall, interp 14.12 s to 12.66 s, llvm 15.96 s to 15.38 s, auto 10.99 s to 10.07 s. `neorv32_single_min`
  interp -1.4 percent, llvm -2.5 percent instructions; `poc_cache_cpu`, `sc5_fifo_repack`, `uvvm_util_rand`, `en_cl_fix_pkg` within 0.5
  percent (their `ieee` calls are few). Over all 42 realbench tests (interp and llvm): instruction geomean 1.00 and 0.99, no instruction
  ratio above 1.02. Work counters of the perf designs that call `ieee` natives fell by up to 29 percent (`reduce_idiom`), `slice_memo`
  rose by 1.4 percent (its bound has room); the bounds were lowered and `ieee_operands` added. Differential tests (510 designs, interp,
  llvm and auto, 1 and 4 threads) are identical.
