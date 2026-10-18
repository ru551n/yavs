# Decisions

## Project name: yavs (user, 2026-10-18), formerly the working name rsim

- **Choice:** the project is yavs, "Yet Another VHDL Simulator". Crates are `yavs-*`, the
  binary is `yavs`, environment variables are `YAVS_*` (VUnit: `VUNIT_YAVS_PATH`), native symbols
  start with `yavs.`, and library directories hold `yavs-lib.toml` and `_YAVS_LIB`.
- **How:** one mechanical sweep, `tools/rename-yavs.sh` (idempotent; other worktrees run it on
  their own changes). The GitHub repository is `ru551n/yavs` (the old URL redirects).
- **Kept:** the `~/.cache/rsim-*` scratch directory names and the `~/git/rsim` checkout (both exist
  on disk), the VUnit branch names, and the vendored `lib/std` and `lib/ieee` files, which keep their old
  comments so `lib/SHA256SUMS` stays valid. A library directory that still has the old
  manifest name fails with L0004 and asks to re-analyze: there is no reader for old names.

## Release trigger and first release (user, 2026-10-18)

When yavs simulates a VUnit testbench end to end (issue #110: `run.py --simulator yavs` passes a
minimal VUnit testbench on the real binary), CI is added (#134) together with a release workflow
(#165), and the first release is cut. Rationale: a release is only useful once the first
milestone users care about works.

- Release workflow: triggered by a `v*` tag; builds `yavs` for Linux x86_64, Linux aarch64 and
  Windows x86_64 (with `yavs.lib` for VHPI plugins and `lib/` std/ieee sources), runs the CI
  checks on the tagged commit first, and attaches archives plus SHA256SUMS to a GitHub release.
- First release: `v0.1.0` (the workspace version), marked pre-release, notes listing what works
  and the known gaps. Once the trigger is met, the user's request counts as approval to publish it.

## Waveform writer: wavefst (yavs-wave)

- **Choice:** the `wavefst` crate (0.4.0, MIT) for FST output, with `default-features = false`,
  features `writer` and `lz4` (pure Rust, no C compiler needed). No FST writing code of our own.
- **Why:** wellen is reader only. `fst-writer` 0.3.1 (BSD-3) has no hierarchy attributes, so it
  cannot carry VHDL variable/data types or enum tables, its real-value path is unused and its
  flush only works at a time step boundary (`todo!`). `wavefst` has enum tables, VHDL supplemental
  variable metadata (`CreateVar2`), aliases, reals, `flush()` that patches the header, and is
  checked against libfst in both directions.
- **Risks:** a young single-author crate. All use is behind `yavs-wave`'s own API (`WaveWriter`),
  so it can be swapped or vendored. yavs-wave's round-trip tests read every file back with wellen.
- **FST mapping:** std_logic family and bit are written as characters (`u x 0 1 z w l h -`) with
  the VHDL data type set, so viewers show `U`, `X`, ...; other enums are bit vectors with an FST
  enum table (literal names in original spelling); integers and time are two's complement vectors;
  reals are doubles; vector bounds go into the name (`data[7:0]`, `rev[0:3]`, left first, so
  direction is kept); records and arrays are scopes with one signal per sub-element.
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
  header from rust-vhpi (`vhpi-sys/vhpi_user.h`, commit `65484e3`; NVC ships the same file with
  `(void)` prototypes). Plugins build against it. We do not write our own: names, values and
  layouts are fixed by the standard, and the IEEE text is what every other simulator ships.
- **Rust side:** hand-written `#[repr(C)]` types and one constant per enumerator in `sys.rs`
  (C enums are `c_int`), no bindgen or libclang build dependency (painful on Windows). The test
  `abi_matches_c_header` compiles a C program generated from the Rust tables against the header
  and compares all ~610 constants and every struct size and field offset; a second test checks
  the header has no constant the Rust side lacks. Same result on Linux and Win64 (MinGW, wine).
- **Export, Linux:** the executable exports exactly the symbols in `src/exports.txt` through
  `-Wl,--dynamic-list` (the executable form of a version script; `--export-dynamic` would also
  export every Rust and std symbol). Plugins leave `vhpi_*` unresolved; plugin loading uses
  `RTLD_NOW | RTLD_GLOBAL`, so a missing symbol fails at load, naming it.
- **Export, Windows:** no VHPI DLL. The executable is linked with a generated `.def`
  (`/DEF:` for MSVC, the `.def` as linker input for MinGW). `build.rs` also writes import
  libraries `yavs.lib` (naming `yavs.exe`) and `yavs-vhpi-host.lib` with `lib.exe` (MSVC), else
  `llvm-dlltool`, else MinGW `dlltool`; short-format import libraries work with both MSVC link
  and GNU ld. The import names the executable, so a renamed `yavs.exe` breaks plugins linked
  that way (shims that use `GetProcAddress` on the main module do not care).
- **One list:** `build.rs` turns `src/exports.txt` into the dynamic list, the `.def`, the import
  libraries and a Rust table that references every symbol, which keeps them linked from the rlib.
  Other packages (the `yavs` CLI) get the paths as `DEP_YAVS_VHPI_DYNAMIC_LIST`, `_DEF` and
  `_IMPORT_LIB_DIR` (the crate has `links = "yavs_vhpi"`) and must pass them to their own binary
  link. The spike binary `yavs-vhpi-host` gets them via `rustc-link-arg-bins`.
- **Variadics:** `vhpi_printf`, `vhpi_vprintf`, `vhpi_assert`, `vhpi_control` are in `src/shim.c`
  (stable Rust cannot define C-variadic functions) and call plain Rust functions.
  `vhpi_control` ignores the optional argument after `vhpiStop`/`vhpiFinish` (cocotb passes
  `vhpiDiagTimeLoc`).
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
  A lookup that finds nothing (`vhpi_handle_by_name`) is only reported through
  `vhpi_check_error`, since cocotb probes names routinely. Codes `E09xx`/`W09xx` are reserved
  for VHPI.
- **Verified:** Linux x86_64 by `cargo test` (C plugin built against only the header, loaded
  through `vhpi_startup_routines` and through `LIB:ENTRY`, plus missing file, missing entry and
  unresolved-symbol errors). Windows x86_64 GNU by cross-building with MinGW and running under
  wine (the export table has exactly the 42 symbols, the plugin DLL imports from
  `yavs-vhpi-host.exe` through the llvm-dlltool import library, same output as Linux). The
  tests themselves cannot run under wine since they call the C compiler. Windows MSVC (`/DEF:`,
  `lib.exe`, `cl` in the tests) is only build-path code until Windows CI runs it.

## std value conversion: NVC formats, LRM parsing (yavs-std)

- **Text is Latin-1 bytes.** Every VHDL-level string in `yavs-std` is `[u8]`, one byte per
  CHARACTER, matching the runtime's STRING layout. Identifiers keep their declared spelling.
- **Output formats follow NVC byte for byte** where the LRM leaves them to the implementation,
  because NVC is the differential reference for report text: `real'image`/`to_string(real)` is
  C `%.15g` (`1e-05`, `0.333333333333333`), `to_string(real, 0)` is `%.17g` (round-trips),
  `write(real)` with `digits = 0` is `%e`, `time'image` is in fs, `to_string(time, unit)` is an
  integer when exact and `%.15g` otherwise, and `write(time, unit)` is exact decimal with at most
  `min(floor(log10(unit)), 17)` truncated digits (GHDL agrees on the last one). GHDL prints reals
  as shortest round-trip VHDL literals (`1.0e-1`) and Questa as `%e`; neither is adopted.
- **The LRM wins over NVC** where NVC is wrong or crashes, each case listed with its reason in
  `crates/yavs-std/golden/nvc-deviations.txt`: extended identifiers keep case and doubled
  backslashes; `'VALUE` takes full literal syntax (based literals, exponents, underscores only
  between digits, decimal physical literals rounded to the primary unit, unit-only `ns`, extended
  unit names) and rejects what is not one literal (`Fo o`, `1__0`, `.5`, `5.`); overflow is an
  error, never wrapped; `to_string(real, format)` is full C99 printf for one real (flags, `%a`,
  literal text), with anything else an error; no fixed output buffer limits.
- **Leniencies kept from NVC/GHDL**: a leading `+`, an integer literal for a real, no separator
  in `5ns`, and format effectors (HT, CR, LF, VT, FF) as whitespace around a value.
- **READ**: numbers are `[sign] digits [. digits] [E [sign] digits]` with no underscores or bases
  (NVC and GHDL agree), TIME accepts decimals (`1.5 ns`, as GHDL; NVC rejects), the unit is the
  longest unit name prefixing the rest (`5 nsx` reads 5 ns), a failed READ leaves the line
  unchanged (GHDL; NVC consumes part of it) and returns a structured error that the GOOD form
  maps to `false`. READLINE drops the CRs just before LF (CRLF files) but keeps inner CRs (NVC
  deletes every CR); WRITELINE writes LF on all platforms.
- **env**: TIME_RECORD arithmetic is exact in microseconds and years outside 1..4095 are errors
  (NVC wraps epochs past 2**31 and loses microseconds in `-`); `seconds_to_time` truncates like
  NVC but errors on overflow; LOCALTIME uses the system zone through `jiff`; `DIRECTORY.items`
  are sorted, without `.` and `..`; TOOL_* name yavs.
- **Verification**: `golden/gen.py` writes testbenches, runs NVC 2008 and 2019 and records
  goldens (about 2,900 cases per revision); the Rust tests compare against them without needing
  NVC, and proptest checks `'value('image(x)) = x` and WRITE-then-READ round trips.

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
  other ending keeps the 2008 rule. NVC, GHDL and Questa 2025.3 reject a bare `end;` for every
  other construct (`docs/design/lrm-checks.md` P1); yavs gives E0002 with a help naming the rule.
- **2019 conditional expressions** (`a when c else b`, `docs/design/lrm-checks.md` P2): bare in
  constant, signal and variable initial values, interface defaults (U-5, permissive), association
  actuals (also after `inertial`) and `return`, plus anywhere as the 2019 primary
  `( conditional_expression )` (VI#385). The operand of a qualified expression is not a primary,
  so `T'(a when c else b)` is an error with a help to write `T'((a when c else b))`. `unaffected`
  and a final `when c` without `else` only in simple variable and force assignments and `return`;
  selected variable assignments take plain expressions. Not yet parsed: the plain `return when c;`.
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
- **PSL** is out of scope: PSL declarations, directives and verification units get E0005 and are
  skipped (to `;`, or to the closing `}` of a `vunit`).
- **Corpus lists** live in `tests/parse-corpus/` (`tests/corpus/` is the differential
  simulation corpus). NVC `test/regress` files tagged `2019` in `testlist.txt`, and `test/parse`
  files whose test sets `STD_19`, are parsed as 2019; everything else as 2008 (yavs has no 1993
  mode, so 93 code that uses 2008 reserved words is an expected failure). VESTs `vhdl-ams` is
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
  not collected yet.
- **Heap addresses carry no generation; deallocated slots are quarantined instead** (#81,
  yavs-interp). The address format has no room for a generation next to a 24-bit slot index
  and a 32-bit offset, so a deallocated slot is not reused until its process suspends
  (registers are dead then) and no aligned word in the process's frames or temporaries decodes
  to an address of it. The same address search keeps such objects alive across a collection
  (a by-reference parameter of a waiting procedure). An access through a stale address
  therefore always meets the freed slot and is the runtime error `access to a deallocated
  object`. Ceiling: for the shared heap only the suspending process's frames are searched.
- **Debugging like Riviera-PRO and Questa** (user requirement): conditional line breakpoints
  (the condition is an IR function nested in the stopped frame's function) with hit counts,
  step in/over/out per process, a global step across processes, pause inside endless loops,
  signal breakpoints as an event-bus observer that pauses `SimControl`, call stack and locals
  through `FrameView`, a process table with what each process waits on. Stops inside a process
  require `--threads 1`. API in docs/design/ir.md 3.4.
- **Verifier** is one linear walk; register liveness across suspension is checked within a
  block only (cross-block needs dataflow, LLVM tier).
- **Disk files bypass the kernel's K6 streams:** writes go straight to the file. With K6
  buffering a process that writes a file, closes it and reopens it for reading in the same
  delta (a common testbench idiom) would read nothing. Determinism holds because a file object
  belongs to one process or to serial processes. `STD_OUTPUT` still goes through the kernel's
  OUTPUT stream so it stays ordered with `report`. K6 was removed in kernel round 3 (#82).
- **Open, needs a decision: BIT/BOOLEAN in bit-planes.** yavs-elab stores BIT leaves as their
  positions (0/1, codes of 'U'/'X'); the interpreter's natives write the codes of '0'/'1'
  (2/3), with which `LogicOp`/`LNum` (std_ulogic tables) compute BIT results correctly. The
  interpreter reads both for now. Proposal: '0'/'1' codes everywhere.
- **Foreign arrays** are passed as C-layout copies left to right (NVC convention), logic
  elements one code byte each (BIT/BOOLEAN 0/1); records as naturally aligned structs;
  access values as a pointer into the object (yavs additions). Copy-back range-checks.

## Generics (yavs-sem, S4)

- **Formal packages with a generic map** (6.5.6.3, 6.5.7.2): the actual instance must have been
  instantiated with the same actuals (static values, base types, subprograms); a mismatch is
  `E1503`. The LRM is clear and Questa rejects; NVC does not check. Non-static values are not
  compared. Inside the generic the formal package stays opaque.
- **W1501 (unused generic)** applies to the generics of packages and subprograms, reported once
  the unit is complete: after the package body, after the package declaration when the library
  holds no body for it, after a generic subprogram body. Entity, component and block generics are
  not checked (they are used across units).
- **Generic protected types (2019):** analyzed with opaque generics; an object of one is `E1505`
  until the parser accepts the generic map on variable declarations.
- **An unknown generic in a generic map does not hide a missing actual:** both `E1100` and
  `E1502` are reported, as NVC does.

## Design units and binding (yavs-sem, S5)

- **Default binding (Q4, 7.3.3):** an entity made directly visible by a use clause wins over the
  one in the design unit's library (`W1703` when both exist); the architecture is the last one in
  manifest or analysis order. `Db::default_binding(scope, component)` with the instance's
  `StmtKind::Instance::scope`.
- **Selected entity names:** `architecture a of lib.e` is accepted (LRM 3.3.1 gives a name; NVC,
  GHDL and Questa accept it); `lib` must be a visible library name and the architecture's own.
- **Block configuration of a default-bound instance** (`for u : c  for rtl end for;  end for;`
  with no binding indication): accepted, the default binding (7.3.3) supplies the entity. LRM
  3.4.3 does not say whether an implicit binding makes the instance "fully bound"; Questa accepts,
  NVC and GHDL reject, so the permissive reading applies. With no entity to bind it is `E1704`.

## Reflection: TIME values in mirrors (user, 2026-10-18)

`PHYSICAL_VALUE_MIRROR.value` for a `TIME` value is expressed in units of
`std.env.resolution_limit`, as Riviera-PRO does (NVC returns femtoseconds). The LRM is unclear,
and VUnit's `record_dict_pkg` (PR #1239) relies on this reading. With the default femtosecond
resolution both readings give the same number. Applies to the M3 runtime that builds mirrors.
`PHYSICAL_VALUE_MIRROR.unit_index` is then the index of the unit equal to the resolution limit
(the primary unit for other physical types).

## Reflection: names, indices and case (user and yavs-interp, 2026-10-18)

- **Physical unit indices are 1-based** (`unit_name(1)` is the primary unit), as NVC; the LRM
  does not say (user).
- **A physical mirror's `image`** uses NVC's unit case: `5000000 FS` (user). Other `image`
  results are the type's `'image`.
- `simple_name`, `element_name` and `unit_name` are upper case, as NVC returns them (LRM silent;
  VUnit's `record_dict_pkg` accepts any case). Enumeration literals, element and unit names are
  looked up case-insensitively (NVC compares `enumeration_literal(name)` case-sensitively; the
  permissive reading accepts both spellings).
- Record element indices and `enumeration_literal(idx)` count from 0 (the element / literal
  position within the subtype), as NVC.
- `array_value_mirror.get` takes LRM indices (`get(left - i)` on a `downto` array is element `i`
  from the left); NVC 1.23 normalises some indices differently (`tests/corpus/reflect/multidim`
  is `ref=hand`).
- A conversion to the wrong class (`value_mirror.to_record` of an integer) is a runtime error.
## Incomplete types in protected method profiles (user, 2026-10-18; amends U-2)

Allowed only in units of library `std` (`std.reflection` needs it). In user code it is `E1220` with
a note and a help suggesting an access type, as NVC, GHDL and Questa reject it. Not extended to
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
  the region (NVC's names, so difftest paths match).
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
  every scope's generics before elaboration has built it. Still E2106: multi-dimensional
  aggregates, composite arguments with composite elements, shifts and other array operators.
- **Package objects at run time** (#183, elab agent with the lowering agent, 2026-10-18):
  packages outside std with signals, shared variables, files or constants sem did not fold are
  lowered (`Lowerer::package`) before the architectures and initialised once at build time
  (`yavs_interp::init_package`), in analysis order (a package after those its declaration uses;
  ponytail: a body using a later-analyzed package would need dependency order), sharing the
  processes' `SharedState`. Every signal of a loaded package is elaborated, driven or not.
- **Package constant calling a function of its own package** whose body is not elaborated yet
  (LRM 14.4.2): E2114 with both spans, as NVC and Questa do.

## Runtime: default exit severity (user, 2026-10-18)

`yavs run` defaults to `--exit-severity=failure`, like NVC, GHDL and Questa. A report or assert of
severity ERROR is printed and the simulation continues; the run exits 1 at the end if any ERROR or
higher was reported. FAILURE stops the simulation at once. yavs's own runtime errors (bound,
overflow, null access, delta limit, internal) still stop immediately, per the error motto.
`--exit-severity=error|warning|note` lowers the threshold. VUnit passes its own setting.
Reports go to stdout in NVC's format (`** Note: <fs>fs+<delta>: msg`, then `    Process <path>`),
which tools/difftest parses.

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
  elaboration, and NVC, GHDL and Questa accept such constants in static names (OSVVM
  AxiStreamReceiver, WishboneRegisterSubordinate).
- **Implicit versus explicit:** a call ambiguous only between implicit operations and explicit
  subprograms picks the explicit ones (2019: a user `to_string(a; sep := " ")` beside the
  implicit composite `TO_STRING`, in the same package or through a use clause). NVC, GHDL and
  Questa accept OSVVM's RandomBasePkg and UVVM's rand_pkg this way; the LRM does not settle it.

## Implicit operation natives (yavs-interp `implicit.rs`)

- **Shifts with a negative count** follow LRM 9.2.4 (`sla -k` is `sra k`, `sra -k` is `sla k`),
  as GHDL does. NVC fills from the wrong end for negative arithmetic shifts.
- **Concatenation bounds** follow LRM 2008 9.2.5: two null operands give the right operand;
  otherwise the result starts at `S'LEFT` in the direction of `S` and both bounds must belong
  to `S`. GHDL agrees; NVC gives `S'LEFT to S'LEFT - 1` for two null operands and a wrong
  (null) range for downto index subtypes. The `cat` native takes `S'LEFT`, `S'RIGHT` and the
  direction as three extra cells.
- **`?<` .. `?>=` with a `'-'` operand** is a runtime error (LRM 9.2.3 says it is an error);
  NVC returns `'X'` silently.
- **2019 `TO_STRING` of an array** whose element type is not a character enumeration is
  `(e1,e2,...)` as NVC prints it; a null such array gives `()` (NVC prints `)`).
- **`MINIMUM`/`MAXIMUM` of a null array** give the element type's `'HIGH`/`'LOW` (NVC agrees).

## Differential testing (tools/difftest, M2 conformance, 2026-10-18)

- **Default assert message (#12):** for `assert c;` without a report expression NVC prints a source
  snippet instead of the `Process` line. yavs keeps its CLI format (cli-contract section 5: the
  `Process` line after every report), and the difftest normaliser drops the path of an
  `Assertion violation` event on both sides.
- **Exit 2 is not always an elaboration failure:** `stop(2)` exits 2 like NVC, so the yavs template
  runs `yavs elab` first and maps only its failure to "elaboration failed".
- **Report order within one cycle (#11): yavs keeps committing in elaboration order.** NVC's
  order is neither elaboration nor wakeup (FIFO) order: six processes whose timeouts all fall on
  3 ns, scheduled at 0, 1 and 2 ns, report as p2 p4 p1 p5 p6 p3 in NVC, an artefact of its event
  heap; processes woken by one signal event report in reverse registration order. Matching that
  would mean reproducing NVC's heap layout, which the LRM does not ask for (the order is
  unspecified) and which no other simulator shares. A deterministic wakeup order would match
  NVC no better than ProcId order, so the kernel is unchanged and `--threads N` stays
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
  (#190).
- **Instance values:** generics, generate parameters and non-static constants are written by
  the elaborator from its folded values (`UnitIr::write_value`); scalar constants it cannot
  fold run a thunk. Locally static constants have no slot. Composite constants of an
  architecture are re-evaluated at each use until elaboration folds them all (#73).
- **Driver slots** are one per `ProcessInfo::drivers` entry, in order; a target picks the entry
  with the same root and path.
- **GUARD:** sem keeps a guarded block's condition as the GUARD signal's default; lowering
  emits the guard process (`GUARD <= condition`, zero delay) and guarded assignments that
  disconnect `bus`/`register` targets when GUARD is false (disconnection after 0 fs until
  disconnection specifications reach the HIR).
- **Matching case** (`case?`, `select?`) compares with `?=` on BIT and STD_ULOGIC elements:
  '-' matches anything, metavalues match nothing, L/H match 0/1 (LRM 9.2.3).
- **`'path_name`/`'instance_name`** are strings in instance data (`UnitIr::names`), written by
  yavs-elab per region: a design unit names its instance, a signal or constant the region that
  declares it, a process label its process; other objects (variables) are named from the
  process's region (ponytail: the process segment is missing for process variables).
- **Array signal descriptors** live in instance data (`SignalIr::dope`), written by yavs-elab from
  each signal object's bounds, so unconstrained ports take their actual's bounds. Array signal
  parameters pass the signal reference, the driver slot (`out`/`inout`) and the actual's
  descriptor; their elements and slices are sub-ranges of the reference.

## CLI: JSON diagnostics, `-W` forms, `--explain` (yavs CLI, 2026-10-18)

- **JSON diagnostics follow rustc** (`level`, `children`, `rendered`, plus `code` as a plain string
  and byte/line/column `spans`), as `docs/cli-contract.md` section 0 promised. The contract won over
  the first renderer (`severity`, `notes`, `help`) because nothing consumed that shape (#158).
- **`-W` forms:** `-Werror`, `-Werror=CODE`, `-Wno-CODE`, `-WCODE`, for `analyze`, `elab` and
  `run`; per code the last option wins; `CODE` must be a registered warning code, else `E0103`.
  Same spelling as the contract's `yavs.a_flags` list, GCC-like rather than rustc's `-A/-W/-D` (#137).
- **`--explain CODE`** lives in the CLI crate, not in `yavs-diag` (a leaf): one table for all stages
  but `yavs-sem`, which keeps its own. A test fails for any code literal without a text (#136).
- **A panic is `E9999`, exit 3** in every command (#159).
- **Package objects (#183):** each package with run-time objects gets package data
  (`AddrSpace::Pkg`, `PackageIr`): an initialisation thunk evaluates its constants that are not
  locally static once (composites of static layout in place) and creates its protected shared
  variables, run once by the elaborator with `yavs_interp::init_package` (the package's data is
  writable only there). Package signals take the first indices of every unit's signal table.
  Composite package constants whose bounds are known only at run time are still re-evaluated
  at each use (ponytail).
