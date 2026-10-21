# AGENTS.md: yavs-llvm

Read the root `AGENTS.md` first. This file is for an agent about to change `crates/yavs-llvm`.
Binding choices are in `docs/decisions.md` "LLVM bindings: llvm-sys 211, no inkwell"; Windows
in `crates/yavs-llvm/WINDOWS.md`; measurements in `STATUS.md` "llvm".

## 1. Purpose and boundaries

- The **LLVM tier** (M6): `compile(&Program, OptLevel, max_size) -> LlvmTier` lowers every
  non-elaboration IR function to LLVM 21 and JITs it through ORC LLJIT (portable CPU, O2, Rust
  runtime callbacks as absolute symbols). `LlvmTier` implements `yavs_interp::Tier`; `yavs run
  --tier=llvm` sets it on the runtime. Design: `docs/decisions.md` "LLVM tier: side exits to the
  interpreter".
- Compiled code runs on the interpreter frame's registers, slots and temporary area
  (`yavs_interp::TierFrame`, a `#[repr(C)]` view of the machine's stacks the interpreter keeps
  current). Inline: `Const`, `ConstF`, `Move`, `Bin`/`BinRange` (not `**`), `Un`, `FBin` (not
  `**`), `FUn`, `IToF`, `Cmp`, `FCmp`, `Bool`, `Check`, `FCheck`, `Index`, `Br`, `CondBr`,
  `Case`, and with a slow path that steps the instruction: `AddrOf` (`Up` one or two levels
  through the view), `Load`, `Store`, `Copy`, `Alloc`, `Stmt`, `MakeDope`, `DopeLen`, `Pos`,
  `SubtypeCheck`, `Field`, `LayoutOf` (types whose element layout is static), `LGet`, `LSet`,
  `LCopy` (up to eight plane blocks each side, through `yavs_rt_lcopy`, which runs the
  interpreter's `yavs_interp::copy_bits`), `Deref` (through `yavs_rt_fast`, `TierHost::deref`), `MemEq` (`yavs_rt_memeq` on the bytes `Mem` found), `SigVal`, and the scalar `std_ulogic` natives as
  tables (`yavs_interp::code_table`). Memory is reached through `Mem` (the view plus a small
  cache of objects `yavs_rt_host` asked `TierHost::object` for; frame, temporary and instance
  addresses are found inline, and a constant the generator knows (`Known::Const`) is read from a
  private copy in the module, `Gen::const_global`). Through the `yavs_rt_signal` callback
  (`TierHost::signal`: the interpreter's own functions without its generic step):
  `SigRead` (one range), `Drive`, `DriveSlot` (one range), `WaitOn` and
  `WaitFor` (each with the `Suspend` after it), `Suspend` and the `rising_edge`/`falling_edge`
  natives (#339); `SigReadScalar` of a logic signal that keeps its codes, and `rising_edge` /
  `falling_edge` of the edge the kernel resumed the process for, are read in place from
  `TierFrame::sigtab` and `known_*` (`Gen::sig_read_scalar`, `Gen::edge`; the callback is their slow
  path, #440). The same callback runs the instructions that only change the temporary area or one register without the generic step and without forgetting the cached objects: `TempRoot`, `Stmt` (when roots are recorded), `NewFrame`, and the slow paths of `Deref` and `MemEq` (`Gen::step`); a `TempRoot` of a fat pair asks `yavs_rt_fast` (`FAST_FAT_ROOT`) first and records nothing when no word points into a heap (#432). Every other instruction runs in place through
  the `yavs_rt_step` callback (`TierHost::step`): calls, natives, foreign calls, reports, the
  other signal operations and waits; a frame a call pushed runs its compiled code directly from
  the callback and is popped by `TierHost::ret` (#336). Compiled code returns to the interpreter
  at `Ret`, every guard failure (overflow, check, zero divisor, non-finite real, address over
  4 GiB, re-executed by the interpreter for the identical error), and with `TIER_UNWIND` /
  `TIER_SUSPEND` when frames it pushed are left to the interpreter or the process suspends. The open native lowerings are issues (section 6).
- `AutoTier::start` (`yavs run --tier=auto`) compiles at O2, on one background thread, the
  functions where the sampled interpreter time reaches `PAYBACK` times their estimated compile
  time, with their small callees (`Worker::tree`), and only while the compile time spent is at
  most `Heat::budget` percent (2; sixteen times that for a compute loop, `compute`) of the interpreter
  time the sampler saw (`Worker::pick`, #432). With the hidden `--tier-heat` there is no budget
  and the first level is O0, those still hot in their compiled code compile again at O2
  (`Heat::o2_threshold`);
  `AutoTier::stop` ends that at the end of the run. The
  spike (`spike.rs`) stays as the JIT smoke test.
- Everything is behind the `llvm` feature (off by default), so the workspace builds without
  LLVM 21.
- Depends on: `llvm-sys`, `yavs-ir`, `yavs-interp` (all optional, feature `llvm`). Called by: the
  `yavs` binary (feature `llvm`).

## 2. Map

| File | Responsibility |
|---|---|
| `src/lib.rs` | Feature gate, crate-root `#![allow(unsafe_code)]` with its comment, re-exports |
| `src/lower.rs` | `compile`, `compile_one` (one function in a module of its own), `too_big` (size cap), `enter` (with the `Lookup` of compiled callees), `new_jit`, `LlvmTier` (`Tier` impl, `notes` of functions left interpreted, inline/interpreted counts), `Mem` (the `TierFrame` view, the object cache, field offsets `MEM_*`), `Gen` (IR to LLVM per function, entry switch, one exit block, known values (`Known`: integers, frame and constant addresses), guard exits, slow paths, `host`: an address to a host pointer, inline for frame addresses known in the block (no check within the frame size) and known constants (`const_global`), else `yavs_rt_host`), `keeps_objects` (steps after which the object cache stays), `static_layout`, `native_kind` (table and edge natives), the `yavs_rt_signal` and `yavs_rt_lcopy` callbacks, the `yavs_rt_step` trampoline (`step_in`: steps, runs a pushed compiled callee, `NEST_STACK`) and `yavs_rt_host` (panics carried over the JIT frames) |
| Native frames (`src/lower.rs`) | `native_set` (functions that can run on native frames: small, no static link, every instruction lowered without a step, calls only to such functions), `Gen::func` with `native` (`f<id>_n(env, mem, depth, cells...) -> i64`: registers and slots in `alloca`s, `Nat`), `Gen::native_call` (direct call by symbol in one module, through `Mem::ntab` across modules; depth and native-stack guards), materialisation (`yavs_rt_deopt`, `yavs_rt_deopt_slots`, `yavs_rt_deopt_end` into `TierHost::materialise`), `LlvmTier::native_frames` |
| `src/auto.rs` | `AutoTier` (`--tier=auto`): `Worker` (the compile thread's state: seen times, compile cost learned per IR instruction, `pick`, `tree`, `compile`, the budget), compile thread that samples `Runtime::sampler` every `Heat::period` and compiles what passes `Heat::threshold` (also fed by `Tier::hot` for `--tier-heat=..,..,0`; the JIT made with the first hot function; a process without a loop of its own at O2), and again at O2 what passes `Heat::o2_threshold` running compiled (a second JIT); `too_short` and `loops` (a cycle of blocks, not counting a process's way through its suspension), short hot functions compiled at O2 once a compiled function calls them (`callers_of`, `callees`); per-function atomic `Entry` (`Tier::ready`), `Note` callback (with the level), `stop`, `exit_now` |
| `build.rs`, `src/lazy.rs` | Linux: stubs for the `LLVM*` exports of libLLVM, so the executable does not depend on it; `lazy::ensure` opens the library and fills the stub table on first use (`Module::new`, `Jit::new`, `compile`, `AutoTier::start`; a new way into LLVM must call it first) |
| `src/jit.rs` | `Jit` (LLJIT, absolute symbols, lookup), `Module` (own `LLVMContext` per module), `OptLevel`, `portable_cpu`, `Error` |
| `src/spike.rs` | Generated test functions (checked sum, `#[repr(C)]` process state, 10k-instruction body) |
| `tests/tier.rs` | Tier against the interpreter: every integer op and range-checked op, loads and stores of every width in every reachable space, temporaries, copies, descriptors, layouts, logic elements and logic copies (`LCopy` at every block edge, overlapping, from a constant, past the inline limit), all on edge values (results and error texts), a loop that runs entirely compiled, calls between compiled functions (an error in the callee, recursion past the native stack allowance, temporaries a callee returns), precise GC roots, the table `std_ulogic` natives on every code, `AutoTier` compiling a sampled hot loop, recompiling it at O2, compiling a short callee of compiled code and skipping one over the size cap |
| `tests/lazy.rs` | The library is not mapped at process start and is after the first module (Linux) |
| `tests/jit.rs` | Spike tests: overflow trap into Rust, struct state, reference semantics, unresolved symbol by name, background compile and swap |
| `examples/measure.rs` | JIT cost per function size (table in STATUS.md) |
| `WINDOWS.md` | Where LLVM 21 comes from on Windows, llvm-sys static linking, ORC on COFF, CI recipe |

## 3. Design decisions that bind this module

| Decision | Why | Source |
|---|---|---|
| The interpreter is the semantic reference: every suite must give identical output LLVM-only and interpreter-only | Rule 5 | `docs/plan.md` rule 5, "Verification" |
| LLVM 21 pinned (`llvm-sys =211.1.1`, `strict-versioning`, `prefer-dynamic`), optional feature; LLVM is an approved manual dependency | Rule 5; root `AGENTS.md` rule 9 | `docs/decisions.md` "LLVM bindings" |
| Raw llvm-sys, no inkwell | inkwell lacks ORC LLJIT | same |
| ORC LLJIT, no hand-written object loader; host functions as absolute symbols, nothing looked up in the process | Rule 5 | same |
| Portable CPU: `x86-64-v2` or `generic` aarch64, never the host CPU; `default<O2>` | Rule 5 | same |
| One `LLVMContext` per module (`LLVMOrcCreateNewThreadSafeContextFromLLVMContext`) | LLVM 21 removed `ThreadSafeContextGetContext` | same |
| Runtime errors call a Rust `extern "C"` callback and return a status; no unwinding through JIT frames | Win64 has no unwind info for them | same; WINDOWS.md |
| Shared `libLLVM-21.so` on Linux, opened on first use through generated stubs (`build.rs`); static libs from the release tarball on Windows | Link memory; llvm-sys refuses dynamic on Windows; loading it at start cost 5 ms per process (#426) | same |
| Self-contained compiled code on interpreter frames: every instruction it does not lower runs in place through `yavs_rt_step` (calls and natives included); a pushed callee with compiled code runs directly and `TierHost::ret` pops it; it returns to the interpreter only at `Ret`, `Suspend`, failing inline instructions and frames it cannot finish (`TIER_UNWIND`) | Rule 5 by construction; one implementation of frames, heap and traces; no exit per call (user decision, 2026-10-20) | `docs/decisions.md` "LLVM tier: side exits", #336, #337 |
| Memory instructions run natively only where the bytes lie in memory `Mem` reaches and no error is possible; anything else steps the whole instruction, before any side effect | Rule 5: one implementation of address spaces and errors | `docs/decisions.md` "LLVM tier: side exits" (Memory) |
| Vector work compiled code does not inline calls the interpreter's own Rust functions directly (`yavs_rt_lcopy` runs `copy_bits`; `TierHost::signal` runs `sig_read_vec`, `drive`, `wait_for`), never a second implementation in LLVM IR; a constant known at generation time (`Bytes` form, at most 4 KiB) is read from a private copy in the module, the bytes the interpreter reads | Rule 5; constants never change; the object lookup per activation cost 5% of clocked RTL | `docs/decisions.md` "LLVM tier: side exits" (Vector process bodies) |
| GC roots: `New`, `Free`, `TempRoot`, natives and calls are stepped with the registers, slots and temporaries in the machine (compiled code stores every register at once and hands the temporary length back before a step), so a collection inside an activation sees them; frames compiled code called through wait at their call's pc; `Stmt` steps while `Mem::temp_roots` (`TierFrame::temp_roots`) is non-zero so the interpreter drops the ending statement's recorded temporaries | Precise roots in both tiers (#422) | `docs/decisions.md` "Shared heap collection" |
| Native frames for native-callable functions (`native_set`): compiled callers call them directly; the frame becomes an interpreter frame (`TierHost::materialise`) only at a step, a failing instruction, the depth limit or the native stack allowance, and the interpreter goes on from there (`TIER_UNWIND`); processes and other functions stay frame-mode; frame-mode code has entries after preloaded definitions | A compiled call cost what an interpreted one did; materialised frames keep traces, roots and errors the interpreter's | `docs/decisions.md` "Compiled frames and calls between compiled functions", #432 |
| A function whose generated LLVM does not verify, or that names a register beyond its frame, stays interpreted with a note (`--verbose`); never a guess | Fail clearly, per function | `src/lower.rs` `compile` |
| Size cap: a function of more than 50k yavs IR instructions stays interpreted unless `--tier-max-size` raises it (both tiers); `--tier=auto` compiles only hot functions | Superlinear codegen time (user, 2026-10-19) | `docs/decisions.md` "LLVM bindings"; #126 |
| `--tier=auto` (opt-in, default `interp`): the compile thread samples which function each simulation thread interprets (`Runtime::sampler`, every `Heat::period`, 1 ms) and compiles one once seen for `Heat::DEFAULT` (2 ms plus 40 us per IR instruction, about twice its O0 compile time); time in compiled code entered from interpreted code counts for the caller; ONE compile thread, one module per function at O0 (the CLI picks it: O2 codegen costs about 7x), compiled again at O2 once seen running compiled for 30 times its `Heat`; a process without a loop of its own is compiled at O2 at once; a sampled function under 64 IR instructions without a loop (a cycle of blocks; a process's way through its suspension does not count) stays interpreted (`too_short`) until a compiled function calls it, then it is compiled at O2; entries published through an atomic word, swapped in on entry and within 256 jumps; `stop(false)` drops the queue and joins an idle thread, and when it is busy the CLI closes the design's open files (`Runtime::close_files`) and ends the process with `exit_now` (`_exit` after flushing) | Compile time is paid in CPU and compiled code of testbenches gains little (a hot set of 22 library functions compiled before time 0 ran 8% faster at 6 s of compile), so tier-up must not cost more than a few percent of the run: compile time at most `Heat::budget` percent of the interpreted time seen, a function only after `PAYBACK` times its compile time, callees with their caller (a call into an interpreted function leaves the compiled code); the simulation and the exit never wait for compilation; output independent of compile timing (rule 5); LLVM static destructors never run under the compile thread; sampling costs the simulation one store per function entered, the per-op counting it replaced 12% to 25%; first-level (O0) code of clocked RTL processes ran no faster than the interpreter, and a call from compiled code to an interpreted function leaves the caller and enters it again; not the default while it is slower than the predecoded interpreter anywhere (STATUS.md "llvm") | `docs/decisions.md` "LLVM tier: side exits", #125, #432 |
| Time resolution and integer semantics identical in every tier (checked i64) | Rule 7 | `docs/plan.md` rule 7 |
| `unsafe` denied in `Cargo.toml`, allowed at the crate root only; raw handles never leave `jit`/`lower`/`spike` (`auto` only declares `fflush` and `_exit` for `exit_now`) | Root `AGENTS.md` rule 11 | `src/lib.rs`, `Cargo.toml` |

Module-local decisions go in this section in the same commit (root `AGENTS.md` rule 24).

- Clocked RTL (#432, #339): `Drive` of one waveform element runs `yavs_rt_drive` (`TierHost::drive_block`, kernel `Ctx::assign_blocks`), `WaitOn`+`Suspend` runs `yavs_rt_wait`, `SigRead` of a known count up to 64 and `Cat` of known piece lengths are inline (`Gen::sig_read_block`, `Gen::cat_block`), `Alloc`/`DopeLen` and temporary and instance memory need no call; registers defined once by a `Const` are known everywhere (`fixed_consts`). `--tier=auto` compiles `rtl` process bodies at O0 (`auto.rs`). Alias scopes separate slots, temporaries, instance data, constants and code bytes. Rules: decisions.md "Compiled clocked RTL processes".

- Clocked RTL, round 2 (#432): `Tier::run_cached` keeps the heap objects `Mem::objs` found between activations (`TierState`, `Mem::load_objs`/`save_objs`/`forget_end`; valid while `TierFrame::heap_epoch` is unchanged; only constants and `space::HEAP`), `yavs_rt_drive` asks `TierHost::drive_now` first and `yavs_rt_wait` `TierHost::wait_again` (no view, no result), `LCopy` of up to 64 elements is inline (`Gen::lcopy_inline`). `--tier=auto` compiles process bodies of 140 to 600 IR instructions (`RTL_MIN`). Rules: decisions.md "Compiled clocked RTL processes, round 2".

## 4. Invariants and pitfalls

- `cargo test --workspace` does **not** build this crate's code (feature off): run the feature
  build yourself after any change here.
- `build.rs` runs `llvm-config` (`LLVM_SYS_211_PREFIX/bin`, else the path) and `llvm-nm`/`llvm-readelf` from its bindir on Linux; a stub exists for every `LLVM*` export, so an unused one costs nothing (the linker drops it).
- LLVM 21.1.8 with static libs is in `/usr/lib/llvm-21` on the dev host; `llvm-sys` finds it via
  `llvm-config` (Windows packages have none, see WINDOWS.md).
- Codegen cost grows faster than linearly with function size; measure with
  `examples/measure.rs` before changing thresholds, one process per row.
- CI (`llvm` job in `.github/workflows/ci.yml`, #127, #128) clippys, tests and runs 47 difftest cases
  with `--tier=llvm` and with `--tier=auto --tier-heat=1,0,0` (at 1 and 4 threads) on Linux x86_64, Linux aarch64 (native runner) and Windows x86_64 (tests only,
  the difftest harness needs a POSIX shell). Windows needs the steps in WINDOWS.md section 4: static
  CRT, empty `libxml2s.lib`, SDK import libs copied into the LLVM `lib` dir.

## 5. How to work here

```sh
CARGO_BUILD_JOBS=6 cargo test -p yavs-llvm --features llvm
cargo build -p yavs --features llvm                    # yavs with --tier=llvm
cargo clippy -p yavs-llvm --features llvm --all-targets -- -D warnings
cargo run --release -p yavs-llvm --features llvm --example measure -- <stmts> [O0|O1|O2]
```

- No diagnostic codes of its own: the `yavs` binary reports `--tier=llvm`/`auto` problems (`E0120`,
  `E0121`); a code of this crate would take a range in `crates/yavs-diag/src/lib.rs`.

## 6. Coordination

- Measure with the difftest: `YAVS_BIN=<yavs built with --features llvm> python3
  tools/difftest/difftest.py --candidate yavs-llvm` (must equal `--candidate yavs`), and
  `yavs run --tier=llvm --verbose` for what each design leaves to the interpreter.

- `--candidate yavs-auto` runs the corpus with `--tier=auto` (also must equal `yavs`);
  `YAVS_RUN_FLAGS=--tier-heat=1,0,0` (period 0) compiles every function from its first call or jump, so the
  swap-ins happen in the middle of each test.

- Open issues (snapshot 2026-10-20, `gh issue list -R ru551n/yavs --label crate:llvm`): #125
  LLVM tier milestone (tier-up done; open until the M2 to M5 suites pass LLVM-only); #336 (calls
  between compiled functions) is done through `TierHost::step`/`ret`; native lowering still to
  do, by IR construct: #337 natives, foreign calls and reports (run in place through the
  callback, no exit; inline lowering open), #338 logic arrays (`LGet`, `LSet`, `LCopy` done; `LEq`, `LFill`, the logic
  operators and the numeric vector natives stepped), #339 signals and waits (direct calls for
  single-range reads, drives and waits; multi-part references, forces and `WaitAt` stepped), #340 heap (`Deref` through a helper, `New` and `Free` stepped), #341 real
  conversions and powers; #432 compile and step cost. Epic: #144 (M6).
