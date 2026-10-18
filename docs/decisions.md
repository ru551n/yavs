# Decisions

## Waveform writer: wavefst (rsim-wave)

- **Choice:** the `wavefst` crate (0.4.0, MIT) for FST output, with `default-features = false`,
  features `writer` and `lz4` (pure Rust, no C compiler needed). No FST writing code of our own.
- **Why:** wellen is reader only. `fst-writer` 0.3.1 (BSD-3) has no hierarchy attributes, so it
  cannot carry VHDL variable/data types or enum tables, its real-value path is unused and its
  flush only works at a time step boundary (`todo!`). `wavefst` has enum tables, VHDL supplemental
  variable metadata (`CreateVar2`), aliases, reals, `flush()` that patches the header, and is
  checked against libfst in both directions.
- **Risks:** a young single-author crate. All use is behind `rsim-wave`'s own API (`WaveWriter`),
  so it can be swapped or vendored. rsim-wave's round-trip tests read every file back with wellen.
- **FST mapping:** std_logic family and bit are written as characters (`u x 0 1 z w l h -`) with
  the VHDL data type set, so viewers show `U`, `X`, ...; other enums are bit vectors with an FST
  enum table (literal names in original spelling); integers and time are two's complement vectors;
  reals are doubles; vector bounds go into the name (`data[7:0]`, `rev[0:3]`, left first, so
  direction is kept); records and arrays are scopes with one signal per sub-element.
- **Durability:** a block is flushed on the next time advance after `flush_interval` (default
  5 s) or `block_change_limit` changes, and the header is patched each time, so a killed run is
  readable up to the last block. SIGINT/SIGTERM only set a flag (`interrupt::install`); the kernel
  stops between time steps and calls `finish`, which is also done on drop.

## Packed logic vectors: bit-planes (rsim-logic)

- **Choice:** `LogicVec` is stored as four bit-planes (`Vec<[u64; 4]>`, 64 elements per block),
  not 4-bit nibbles. The nibble layout stays behind `--features nibbles` since it costs about
  100 lines and shares all tests and benchmarks.
- **Why:** equal speed up to 32 elements, then 8x to 10x faster logic ops and resolution, and
  40x to 140x faster numeric add/compare at 1024+ elements (a 0/1 vector is plain `u64` words).
  Memory is the same 4 bits per element. Full table in `crates/rsim-logic/LAYOUT.md`.
- **Costs:** `to_string` is 2x slower (per-element decode); a 1-element vector takes 32 bytes, so
  the kernel keeps scalar signals as bare `StdULogic`. VHPI/FST need a transpose per element.

## LLVM bindings: llvm-sys 211, no inkwell (rsim-llvm)

- **Choice:** `llvm-sys` `=211.1.1` (MIT), the raw LLVM 21.1 C API, features `prefer-dynamic` and
  `strict-versioning`, optional behind the crate feature `llvm` (default off, so the workspace
  builds without LLVM). Test with `cargo test -p rsim-llvm --features llvm`.
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
  not register unwind info for them, see `crates/rsim-llvm/WINDOWS.md`).
- **Linking:** shared `libLLVM-21.so` on Linux (low link memory). On Windows llvm-sys refuses
  dynamic linking and uses the static libs of the official release tarball; see WINDOWS.md.
- **Size threshold (rule 5):** codegen time grows faster than linearly with function size (about
  0.15 s at 10k IR instructions, 3.8 s at 100k, 28 s at 300k at O2; numbers in STATUS.md). The
  proposal for M6: JIT a function at O2 up to 40k LLVM IR instructions (under 1 s, under 100 MiB),
  keep larger units in the interpreter, and lower big processes as several functions rather
  than one. To be confirmed with real lowered code in M6.

## Kernel crate and process interface (rsim-kernel)

- **Crate:** the LRM 14.7 cycle lives in `crates/rsim-kernel`, independent of the IR. rsim-rt
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

## VHPI ABI and symbol export (rsim-vhpi)

- **Header:** `crates/rsim-vhpi/include/vhpi_user.h` is the official IEEE 1076-2019 header
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
  libraries `rsim.lib` (naming `rsim.exe`) and `rsim-vhpi-host.lib` with `lib.exe` (MSVC), else
  `llvm-dlltool`, else MinGW `dlltool`; short-format import libraries work with both MSVC link
  and GNU ld. The import names the executable, so a renamed `rsim.exe` breaks plugins linked
  that way (shims that use `GetProcAddress` on the main module do not care).
- **One list:** `build.rs` turns `src/exports.txt` into the dynamic list, the `.def`, the import
  libraries and a Rust table that references every symbol, which keeps them linked from the rlib.
  Other packages (the `rsim` CLI) get the paths as `DEP_RSIM_VHPI_DYNAMIC_LIST`, `_DEF` and
  `_IMPORT_LIB_DIR` (the crate has `links = "rsim_vhpi"`) and must pass them to their own binary
  link. The spike binary `rsim-vhpi-host` gets them via `rustc-link-arg-bins`.
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
  starts: `E0901` (cannot load) and `E0902` (no entry point), rendered through rsim-diag.
  Each failed VHPI call sets the `vhpi_check_error` info (cleared by the next call, printf
  included). API misuse (NULL arguments, stale or wrong handles) is also reported on stderr every
  time as `W0903`; calls rsim does not implement yet as `W0904`, once per distinct message.
  A lookup that finds nothing (`vhpi_handle_by_name`) is only reported through
  `vhpi_check_error`, since cocotb probes names routinely. Codes `E09xx`/`W09xx` are reserved
  for VHPI.
- **Verified:** Linux x86_64 by `cargo test` (C plugin built against only the header, loaded
  through `vhpi_startup_routines` and through `LIB:ENTRY`, plus missing file, missing entry and
  unresolved-symbol errors). Windows x86_64 GNU by cross-building with MinGW and running under
  wine (the export table has exactly the 42 symbols, the plugin DLL imports from
  `rsim-vhpi-host.exe` through the llvm-dlltool import library, same output as Linux). The
  tests themselves cannot run under wine since they call the C compiler. Windows MSVC (`/DEF:`,
  `lib.exe`, `cl` in the tests) is only build-path code until Windows CI runs it.

## std value conversion: NVC formats, LRM parsing (rsim-std)

- **Text is Latin-1 bytes.** Every VHDL-level string in `rsim-std` is `[u8]`, one byte per
  CHARACTER, matching the runtime's STRING layout. Identifiers keep their declared spelling.
- **Output formats follow NVC byte for byte** where the LRM leaves them to the implementation,
  because NVC is the differential reference for report text: `real'image`/`to_string(real)` is
  C `%.15g` (`1e-05`, `0.333333333333333`), `to_string(real, 0)` is `%.17g` (round-trips),
  `write(real)` with `digits = 0` is `%e`, `time'image` is in fs, `to_string(time, unit)` is an
  integer when exact and `%.15g` otherwise, and `write(time, unit)` is exact decimal with at most
  `min(floor(log10(unit)), 17)` truncated digits (GHDL agrees on the last one). GHDL prints reals
  as shortest round-trip VHDL literals (`1.0e-1`) and Questa as `%e`; neither is adopted.
- **The LRM wins over NVC** where NVC is wrong or crashes, each case listed with its reason in
  `crates/rsim-std/golden/nvc-deviations.txt`: extended identifiers keep case and doubled
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
  are sorted, without `.` and `..`; TOOL_* name rsim.
- **Verification**: `golden/gen.py` writes testbenches, runs NVC 2008 and 2019 and records
  goldens (about 2,900 cases per revision); the Rust tests compare against them without needing
  NVC, and proptest checks `'value('image(x)) = x` and WRITE-then-READ round trips.

## Parser (rsim-syntax, M0)

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
  and `TOOL_NAME` are `rsim`, `TOOL_TYPE` is `SIMULATION`, `TOOL_EDITION` is empty.
- **2019 closing keywords:** 2019 makes only `end component` optional (LCS-2016-055a); every
  other ending keeps the 2008 rule. NVC, GHDL and Questa 2025.3 reject a bare `end;` for every
  other construct (`docs/design/lrm-checks.md` P1); rsim gives E0002 with a help naming the rule.
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
  files whose test sets `STD_19`, are parsed as 2019; everything else as 2008 (rsim has no 1993
  mode, so 93 code that uses 2008 reserved words is an expected failure). VESTs `vhdl-ams` is
  VHDL-AMS and is not parsed.

## Kernel round 2: K1 to K7 (rsim-kernel)

- **Bit-plane signals:** `SignalType { elem: Bytes(n) | Logic, len }`. Logic signals use
  rsim-logic's `Block` layout for every buffer (values, drivers, transactions, forces), with
  chunked shift/mask range ops; byte signals keep plain bytes. rsim-logic gained additive
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
- **User files (K6):** the interpreter opens the file and hands the kernel a writer; writes and
  close are buffered effects committed in elaboration order, flushed at end of simulation.
- **Protected types:** callers are `serial` through M3; the kernel needs no further support.
- **`SimError` is boxed** (`Deref` to `ErrorInfo`) to keep hot-path `Result`s one word; a panic
  in kernel code becomes an internal error like a panic in process code.

## Semantic analysis (rsim-sem, M1)

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
  packages (checked at the end of the package body) and protected types (s6). A package without
  any analyzed body is not checked at analysis.
- **Type names in messages** are qualified by their region (`work.i1.rec`) only when two names in
  one message collide.

## IR and interpreter (rsim-ir, rsim-interp, M2)

- **Memory model:** registers are untyped u64 cells; addresses are `[space: 3 | index: 29 |
  offset: 32]` over eight spaces (frame, instance, package, constant, heap, temp, shared,
  shared heap). Every data-dependent access is bounds-checked; an out-of-object access is an
  ICE (lowering must have emitted the VHDL check), a freed object is a runtime error.
- **Temporaries** (`Alloc`) live until the next statement of the frame that made them, so a
  function's unconstrained result survives the return without a copy.
- **Natives** are looked up by `lib/NATIVE.md` symbol, parsed from the file at build time: a
  symbol that NATIVE.md does not define fails `Runtime::new` (fail early), one that is defined
  but not implemented is a runtime error naming it. `'image` is `rsim.attr.image` (NATIVE.md
  has no attribute natives). Implemented now: `now`, `stop`, `finish`, `to_string` and `'image`
  of scalars; the rest (textio, file, env extras) come with M3.
- **Foreign calls from safe code:** `rsim_ffi::ForeignFn::from_fn` binds a typed Rust
  `extern "C" fn` without `unsafe`; real symbols are bound by the elaborator (the one unsafe
  step) and passed in `Bindings`. Scalars and scalar `out` parameters are marshalled; arrays,
  records and access values are a "not implemented" runtime error until M3.
- **Heap:** generational handles; mark and sweep of the private heap at suspensions with
  precise roots from `Func::roots`; collection is skipped (never unsound) for objects whose
  handles cannot be located. The shared heap is not collected yet.
- **Debugging like Riviera-PRO and Questa** (user requirement): conditional line breakpoints
  (the condition is an IR function nested in the stopped frame's function) with hit counts,
  step in/over/out per process, a global step across processes, pause inside endless loops,
  signal breakpoints as an event-bus observer that pauses `SimControl`, call stack and locals
  through `FrameView`, a process table with what each process waits on. Stops inside a process
  require `--threads 1`. API in docs/design/ir.md 3.4.
- **Verifier** is one linear walk; register liveness across suspension is checked within a
  block only (cross-block needs dataflow, LLVM tier).
