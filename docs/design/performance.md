# Performance: what the reference simulators do, and what yavs should take from it

Update (user, 2026-10-19): the no-cache premise of this study is lifted, see `docs/decisions.md` "Compilation cache" (#407).

Status: study (2026-10-19, issue #395 and the performance section of `docs/STATUS.md`). Nothing here
is implemented by this document; section 9 is the ranked list the crate owners work from.

Correction (2026-10-28): this is the study as of 2026-10-19 and is kept as history. Much of section 9 has
since been built or measured (compilation cache, compiled tiers with `--tier=auto` as the default of
a build with a compiled tier, Cranelift as a second generator, port collapsing, specialised copies of
processes); `docs/STATUS.md` has the outcome per item. The "measured gap" in section 0 below is the
2026-10-19 figure and no longer holds: the current comparison with conditions is `docs/performance.md`.
Statements below that say "no cache", "interpreter only by default" or "LLVM only" describe the state
of that day.

## 0. Scope, method, caveats

The user asked (2026-10-19): "regarding performance, take inspiration from how the reference
simulators handle it." Two reference simulators were studied by reading their sources and
documentation, for architecture only. Nothing was copied, and this document describes ideas in
our own words, with no code and no tables from them (root `AGENTS.md` rule 6).

- **Reference simulator A** runs designs through an interpreter over a compact register
  bytecode. A function that has been called about a hundred times is compiled to native code by
  a general compiler back end on a background thread. Everything before that is interpreted.
- **Reference simulator B** translates the whole elaborated design, and every library
  subprogram it can reach, to machine code in memory in one fast pass with almost no
  optimisation, before the simulation starts. There is no interpreter in the simulation path.

The measured gap (`docs/STATUS.md` "performance"): yavs interpreted is 7x (compute loop, against A)
to 80x (against B) slower, 90x to 200x on signal-heavy RTL, 20x to 75x on testbenches.

Caveats, stated once:

- Impact numbers below are **estimates from the structure of the mechanism and from the
  `docs/STATUS.md` measurements, not measurements**. Each recommendation therefore carries a
  measurement gate: profile first, land the change, record before and after in `docs/STATUS.md`.
- Where a statement about a reference simulator is an inference from reading (not from a run
  or from its documentation), it says "appears".
- Every recommendation keeps the design rules: the interpreter is the semantic reference
  (rule 5), `--threads 1` and `--threads N` give byte-identical output (rule 3), logic arrays
  stay bit-planes, the LLVM tier keeps the side-exit design, and no incremental cache is
  proposed (section 8 lists what needs the user).

## 1. IEEE library operators

### Mechanism in the reference simulators

Both reference simulators run `std_logic_1164` and `numeric_std` from their VHDL bodies in the
general case. The speed comes from two things.

1. **Native code for the bodies.** Reference B compiles the bodies to machine code like any
   other VHDL, so a vector `+` is a tight loop over bytes, one byte per `std_ulogic` element,
   without per-call heap work. Reference A would interpret them like any function, which is
   slow, so it does the next thing.
2. **Intrinsics bound by subprogram identity.** Reference A keeps a table keyed by the mangled
   name of a library subprogram (package, operator, parameter and result types). When a
   function with a listed name is first looked up, its entry point is a native routine instead
   of the bytecode of the VHDL body. The covered set is the hot one: the vector `and`, `or`,
   `xor`, `not`, `to_x01`, `=` on byte vectors; in `numeric_std` the `+`, `-`, `*`,
   comparisons, `to_integer`, `to_unsigned`, `to_signed`, `resize`, `to_01` and `divmod`; the
   legacy `std_logic_arith` and `std_logic_unsigned` arithmetic; a few `math_real` functions;
   and two `textio` helpers. Vectors are one byte per element, so the routines use table
   lookups and, where the CPU has them, 16-byte SIMD shuffles; a multiply is shift-and-add over
   a packed form. A per-CPU variant is picked once at start.
   Metavalues and warnings stay exact because the native routine implements the same decision
   tree as the body: it converts the operands to a two-valued form with a marker for "an
   element was a metavalue", returns the all-`X` result if the marker is set, and emits the
   body's own warning text through the normal diagnostic path with a synthesised stack frame
   (so the call stack in the message still names the VHDL caller). The whole set can be
   switched off by an option.
3. **Recognition for synthesis only:** reference B also tags these subprograms in its front end
   (by declaration shape) but uses the tags in its synthesis path, not in simulation.

The scalar `std_ulogic` operators are handled by tables (nine by nine), and the resolution
function of any small enumeration type is memoised (section 2).

### Impact

For the covered subprograms the cost drops from "interpret hundreds of instructions" to tens of
nanoseconds: a 16-bit `+` goes from about 250 us in yavs to well under a microsecond. Profiles
of the long verification-library testbenches (#386, #395) put 40% of instructions in
`numeric_std` and another 25% in per-bit scalar operators called from it, so a library
testbench would lose a large fraction of its interpreted instructions, and RTL with arithmetic
processes (design 2) gains per activation.

### yavs today

The ieee packages run from their VHDL bodies (`lib/AGENTS.md`). The IR already has the pieces:
`LogicOp` and `LNum` (`ir.md` 1.5) call bit-parallel plane operations in `yavs-logic`, and the
interpreter implements `LNum::{Add, Sub, Cmp}`, but lowering emits `LogicOp` only for the
predefined logical operators on arrays; no call of a `numeric_std` or `std_logic_1164`
subprogram is mapped to them. `lib/NATIVE.md` lists natives only for `std` (no bodies). The
bit-plane layout is better placed than a byte-per-element layout for this: "is every element 0
or 1" is one AND and compare per block, an add is a 64-bit carry chain per plane block, and a
comparison is a word compare.

### Recommendations

**R1 (rank 1): fast path with fallback to the VHDL body.** Map a closed list of ieee
subprograms (by library, package, name and parameter types, exact declaration match, so a
user's own package with the same names is untouched) to native fast paths that handle only the
two-valued case: all elements `0` or `1` (after the `L`/`H` mapping where the body does the
same). If any element is another value, or any other corner the fast path does not own (null
ranges, lengths for which the body warns), the instruction **executes the lowered VHDL body**
instead. This keeps metavalue results, warning text, severity, stack attribution and
`--ieee-warnings` behaviour identical by construction, with no second copy of the metavalue
logic to keep in sync; the cost is one lowered body kept available (lowered lazily on the first
fallback, as today). The reference simulators implement the whole decision tree natively; that
is more code and more drift risk than yavs needs, because the metavalue path is rare in real
runs. Order of work: scalar `std_ulogic` `and/or/xor/not` and the vector forms; `numeric_std`
`+ - * < <= > >= = /=` on `unsigned` and `signed`, `to_integer`, `to_unsigned`, `to_signed`,
`resize`, `to_01`, `shift_*`; `rising_edge`/`falling_edge` (used in nearly every clocked
process; a direct instruction removes a call per activation); then `std_logic_arith` and
`std_logic_unsigned`; `math_real` last (R1b). Crates: `yavs-lower` (recognise the call, emit
the instruction with the fallback function id), `yavs-interp` (execute), `yavs-logic` (plane
arithmetic: carry chain, multiply, divmod), `yavs-ir` (an `LNum`-style instruction with an
optional fallback). The LLVM tier lowers the same instruction as a call to the same runtime
function, so rule 5 holds for free.

**R1b (decision for the user, already #395): `math_real`.** Native `sin`, `cos`, `log`, `exp`,
`sqrt` and `ilogb` are the largest single item in the Gaussian random profile (the halving
loop for `ilogb` alone is about a quarter of all instructions there). The two choices are the C
library's results (fast, may differ in the last bit from the VHDL body's algorithm, which is
what the reference simulators do) or bit-exact native reproduction of the body's algorithm
(identical to today). Recommendation: the C library for the transcendental functions,
bit-exact native for integer-valued helpers such as `ilogb`. The user decides; the fallback
design of R1 does not help here because the result is never a metavalue.

## 2. Signals, drivers and the kernel cycle

### Mechanism in the reference simulators

- **Classification at elaboration.** Each signal is classified once from its driver count and
  shape: no driver, one driver, one driver "direct" (the value is read straight from the
  process's own storage at update time), resolved, or part of a connected net that needs an
  ordered propagation. The update code is a switch on the class, so the dominant case (one
  driver, no resolution) is a handful of instructions. Reference B builds the ordering of
  connected nets into a flat propagation table once, after elaboration.
- **Assignment fast path.** A zero-delay inertial assignment to a signal with a single driver
  and no pending future transactions does not allocate a transaction. It stores the value in
  the driver's inline slot and puts the signal on a per-delta chain (reference B) or a flat
  array queue (reference A). If the new value equals the current value and nobody can observe
  activity on the signal (no `'active`/`'transaction`/`'quiet`/`'stable` user, no activity
  callback), the assignment returns at once and the signal is not even queued. In clocked RTL
  most register assignments write the same value, so this removes most update work.
- **Slow path only when needed.** A second assignment in the same delta, a delayed assignment,
  a reject limit or a second driver switch the signal to the general transaction list. The
  switch is one flag and is not reversed per assignment.
- **Event queue.** Reference A keeps one binary heap keyed by time for processes, driver
  updates and timeouts, plus separate flat queues for "this delta" work (drivers, processes,
  resumptions). Zero-delay work never touches the heap. Reference B has chains of active
  signals and a separate list of signals with future transactions.
- **Finding processes.** A signal (or a contiguous slice of one, a "nexus": the maximal range
  with the same sources and sensitivity) holds a pending list of the things to wake: a single
  entry stored in the pointer itself, an array for more. A process with a static sensitivity
  list is registered **once at elaboration** and never re-registered or cleared; only `wait`
  statements with dynamic lists add and remove entries. Procedures that can never wait are
  called as plain functions, without suspendable-call state.
- **Wake-time filtering ("triggers").** Reference A recognises the process shapes
  `process (clk) begin if rising_edge(clk) then ... end if; end process` (and `clk = '1'`,
  `clk'event and ...`, a pure function of signals and literals, or an or of two such tests)
  and gives the *kernel* a small description of the condition. The kernel evaluates it in the
  update phase (a signal-versus-literal test is a direct byte compare, a function trigger a call
  without any process state) and does not schedule the process when the condition is false. The
  result is cached per cycle and shared by all processes with the same trigger. With 64
  processes on one clock, the falling edge costs nothing. It is disabled when statement or
  branch coverage would count the skipped test.
- **Effective versus driving value.** A signal has a separate driving value only when it has a
  port source, a force or a resolution; otherwise driving and effective are the same storage
  and one update writes both. Port edges carry a rank (depth), and updates are processed in
  rank order from two small heaps, so a port hierarchy settles in one delta (what yavs's
  worklists by port level already do).
- **Port collapsing.** An `in` port whose actual is a plain signal of the same constrained type
  with no conversion is lowered as an alias of the actual: no second signal, no port edge, no
  copy. The alias is remembered for names (waves, VHPI). Out, inout, converted and
  unconstrained ports keep real edges. An option turns collapsing off.
- **Resolution.** For a resolved scalar whose type has at most sixteen literals, the
  resolution function is **run by the interpreter once per pair of literals at elaboration**
  and the results stored in a lookup table (one table for a single non-null driver, one for
  two). An identity-like function is detected, so a single driver costs a copy. Longer driver
  lists call the function. This is generic: it works for any user resolution function over a
  small enumeration, and it is exact because the table is computed by the VHDL function itself.
- **Per-activation allocation.** A process runs against a per-thread bump allocator that is
  reset after the activation unless the process claims it (it keeps a live value across the
  wait). Stateless processes (no variable survives the wait) carry no state record at all.

### Impact

Reference A simulates 1.28M clocked-process activations (design 2b) in 0.07 s, about 55 ns per
activation including the compare, the shift/xor and the assignment; yavs needs 17 s, about 13 us.
Even allowing for the interpreter, 13 us is far more than the mechanisms of this section need.
We expect the cost to come from, in this order (to be confirmed by the profile of R6):

1. allocations per activation and per assignment (a waveform vector and a value buffer per
   `Drive`, a sensitivity vector per `wait`),
2. waiter-list churn (re-registering and invalidating a waiter by generation on every wake),
3. waking a process on both clock edges when it only acts on one,
4. the frame and heap-collection work at each suspension (section 3),
5. update work for assignments that do not change the value.

### yavs today

(`crates/yavs-kernel/DESIGN.md`, `src/process.rs`.) `Ctx::assign` builds a `Vec<Txn>` and a
`Buf` per call, then `schedule` does preemption in O(elements times pending transactions); a
`wait` pushes into `st.sens` each time and `Waiter` entries are invalidated by a generation
counter and compacted at twice their live size; there is no unchanged-value early-out, no
static-sensitivity registration and no trigger. The segments/bit-plane resolution for
`std_logic` is already plane-wise (3x faster than before, `DESIGN.md`) and better than a
per-element table. Ports are uncollapsed by decision (`ir.md` 4; component ports are their own
signals with two edges each). Batch order, serial processes and the commit protocol give the
determinism guarantee and constrain every item below to stay inside them.

### Recommendations

**R2 (rank 3, after the profile gate of R6): kernel assignment and wake fast paths.** Four
parts, in this order, each landing separately with byte-identical output at 1, 4 and 8 threads:

- *R2a: allocation-free single-driver zero-delay assignment.* A per-driver inline waveform
  slot holding the pending value in the signal's own storage kind; the general `Vec<Txn>` path
  is entered only for a second assignment in the delta, a delay, a reject limit, a partial
  overlap or a transaction already pending. The per-element preemption rules of the general
  path stay untouched. Crate: `yavs-kernel` (`process.rs`, `signal.rs`).
- *R2b: skip unchanged assignments (done, #420: `docs/decisions.md` "Skipped unchanged assignments").* The first version uses one global flag (the design reads neither `'active` nor `'last_active`) plus kernel-side per-signal facts instead of an elaborator flag per signal. The original plan: elaboration marks each signal "activity observable"
  (`'active`, `'last_active`, `'transaction`, `'quiet`, `'stable`, `'delayed` users, guarded
  signals, forces, activity-reporting observers such as VHPI activity callbacks and the
  waveform sink if it records activity). For other signals a zero-delay assignment whose value
  equals the current driving value and whose driver has nothing pending queues nothing and does
  not cost a delta. The decision uses static facts only, so it is independent of the thread
  count. The LRM defines activity when a transaction matures; skipping is allowed only because
  it is unobservable, so the "observable" set must be conservative (a new observer kind
  defaults to "observes activity"). Crates: `yavs-elab` (compute the flag from attribute use),
  `yavs-kernel` (consume it).
- *R2c: static sensitivity.* A process whose only suspension point is a final `wait on` of a
  static list (the lowered form of a sensitivity-list process; `Program::single_suspend`
  already identifies "one suspension point") registers its waiters once at build time as
  permanent entries and skips the per-wake `WaitOn`/`Suspend` bookkeeping: no per-wake
  registration, no generation bump, no compaction. Crates: `yavs-elab` and `yavs-ir` (flag the
  shape on the process), `yavs-kernel` (permanent waiters), `yavs-interp` (skip the `WaitOn`
  instructions for such a process; this must not change what the interpreter would have
  registered).
- *R2d: edge and level triggers.* For processes of the shapes listed above, `yavs-lower`
  attaches a small trigger descriptor to the process (signal element, literal, optional
  "event" requirement, or a pure function call with signal and literal arguments) and
  `yavs-kernel` evaluates it in the serial wake step before scheduling the process. A false
  trigger means the process would have executed an `if` with a false condition and
  re-suspended on the same list: no visible effect except statement counters. Triggers are off
  while a debugger is attached; with coverage they stay on (user, 2026-10-24, #707): the
  statement of a pure edge test counts the runs at initialisation and at its edge only.
  Evaluating in the serial wake step keeps
  determinism trivial. Crates: `yavs-lower` (shape recognition), `yavs-ir` (descriptor),
  `yavs-kernel` (evaluation), `yavs-interp` (provide the descriptor through `Process`).
  Expected: about half the activations of a single-clock design disappear; with R2a to R2c the
  per-activation kernel overhead should fall by an order of magnitude.

**R3 (rank 7; done for element-wise user functions, #420: a remembered result per set of driving values, `docs/decisions.md` "Remembered results of user resolution functions"): resolution memo tables.** For a resolved signal whose element type is an
enumeration of up to 16 literals and whose resolution function is not the `std_logic` one
(already plane-wise): at elaboration run the function through the interpreter for each single
literal and each pair (diagnostics silenced, as the table is a probe), store the table, and
resolve by lookup (single driver: table or identity; two drivers: pair table; more: fold
pairwise only if the table shows the function is associative and commutative, otherwise call
the function). Crates: `yavs-kernel` (`Resolution::Func` gets the table), `yavs-elab` (builds
it).

**R4 (rank 6): in-port collapsing and component pass-through.** Alias the formal of an `in`
port (and the hidden component port pair) to the actual when the actual is a signal name of the
same constrained type with no conversion, and keep the alias in the hierarchy for `'path_name`,
waves, VHPI and DAP. The two port edges per component port and the per-delta pass through them
disappear, which matters most for deep hierarchies and wide buses. Attributes read on a
collapsed `in` port (`'event`, `'last_value`, ...) are the actual's by LRM 14.7.3.3 (the
effective value of an `in` formal is the effective value of its actual), so they are
unchanged; `'driving` and `'driving_value` do not apply to `in` ports. This reverses the
current "uncollapsed ports" choice for the `in` case only; conversion ports (K1) stay
uncollapsed. It needs a `docs/decisions.md` entry and the user's confirmation (section 8).
Crates: `yavs-elab` (the `NetGraph`), `yavs-kernel` (fewer ports, no other change).
Extended to `out` and `buffer` ports that are the only source of their actual (#528, user,
2026-10-22): their effective value is their driving value, as is the actual's, so the formal's
drivers drive the actual directly (`docs/decisions.md` "Elaboration").

**R5 (rank 8): parallel batch policy.** The reference simulators run processes serially; yavs
runs a batch on a pool. At about 10 us per activation the pool hand-off cost is comparable to
the work. Run a batch inline when it has fewer processes than a threshold; the choice of inline
versus pool does not change results because commit order is the batch order. Crate:
`yavs-kernel`. Worth doing only after R2, when per-activation work is small enough for the
hand-off to dominate.

## 3. Process code: how a process resumes

### Mechanism in the reference simulators

- **No stack per process.** A process is a function with a state number and a record of the
  variables that live across waits. Entry switches on the saved state; a wait stores the next
  state number in the record and returns to the kernel. Reference A passes the record pointer
  in the first argument; a process that has no variable alive across a wait gets a dummy marker
  and no record. A procedure that can wait is compiled the same way with its own record chained
  to the caller's; a procedure that can never wait is an ordinary call.
- **Temporaries on a bump area.** Expression temporaries (unconstrained results,
  concatenations) come from a per-thread bump allocator that is reset to its mark at call
  boundaries and, for a process, after each activation. A process that must keep temporaries
  alive across a wait claims its own area. There is no per-value heap allocation for temporaries
  and no collection at suspension; the collector (mark and compact, for `new` objects only)
  runs from allocation pressure.
- **Frames.** Interpreter frames are arrays on the native stack, sized from the function's
  register and frame size; calls are native recursive calls with a small record linking frames
  for backtraces. Registers are untyped 64-bit cells, the same as yavs. Reference B uses the
  native stack for subprogram frames and one record per process.
- **Dispatch.** Reference A's interpreter is a plain switch loop over three-address ops. It is
  not threaded code. Its speed appears to come from the IR being low level (a check is an
  explicit compare and branch, no instruction does several things) and from short paths for
  common forms; it is not architecturally different from what yavs has.

### Impact

On the compute loop, reference A's whole run takes about 180 ns per iteration (design 1: 3.6 s
for 20M iterations) while yavs interpreted takes about 1.2 us, and yavs with the LLVM tier
540 ns, slower than reference A. Per-instruction cost in call-heavy code appears to be tens to
a hundred nanoseconds in yavs (a `numeric_std` call chain), where a lean interpreter is at 3 to
10 ns. This is a constant factor, not an architectural one, and it sits under everything else:
until it is fixed, the IEEE and kernel work buys less than it should.

### yavs today

(`ir.md` 1.4 and 3; `yavs-interp/AGENTS.md`.) A frame is a heap object with a boxed slice for
slots and a boxed slice for registers; memory accesses decode an `Addr` (space, index, offset)
and bounds-check it on every load and store; checked integer arithmetic goes through
`yavs-std`; process heaps are collected when their process suspends; temporaries live until the
next `Stmt`. A very recent commit keeps the top frame's registers in the machine, which is the
right direction.

### Recommendations

**R6 (rank 2): interpreter constant factors, profile first.** Before any structural change,
profile design 1 and the call-heavy `numeric_std` loop with a sampling profiler and a per-opcode
counter, and set a budget: 10 ns per simple instruction, 100 ns per call/return pair. Likely
items, each small and independently verifiable:

- calls: one contiguous per-process value stack for frames (registers and slots as windows into
  it, popped on return), so a non-suspending call allocates nothing; only frames of procedures
  that can suspend (`MAY_SUSPEND`, known statically) need to survive a wait. Today every call
  makes two heap allocations;
- memory access: resolve the frame, instance and constant spaces once per function entry to base
  slices held in locals, so `Load`/`Store` for the common spaces is one compare and one index;
  keep the full decode for heap and foreign spaces only;
- arithmetic: inline the checked `i64` add/sub/mul with the compiler intrinsics in the loop and
  call out only on overflow; avoid building a `Result` on the non-error path;
- suspension: do not run the heap mark phase at every suspension. Trigger collection from
  allocation volume since the last collection (a counter), and let the common process (no
  `new`, no composite temporaries alive across the wait) skip it entirely. This is a hypothesis
  for part of the 13 us per activation and the first thing to check in the profile of design 2b;
- temporaries: a bump area reset per activation and per statement for the non-heap cases,
  replacing per-object bookkeeping;
- confirm the fast loop has no hook test and no fuel counter (`Stmt` is a nop there already).

Crates: `yavs-interp` (all), `yavs-ir` only if a frame-layout fact is missing.

## 4. Elaboration

### Mechanism in the reference simulators

- **Units are lowered lazily, per function.** Reference A looks a function up by name when it is
  first called; the lowering of a package-body subprogram to its IR happens at that moment and
  only for that function. Unused subprograms of large packages are never lowered. Reference B
  marks, during semantic analysis, which declarations are referenced and, when it elaborates and
  runs in one step, generates code only for the referenced subprograms of each unit in the
  closure. Both therefore pay for what a design uses, not for the size of the libraries.
- **Packages are initialised once, on first use.** A package's elaboration routine runs when
  first linked, in the single run-time instance, not per instance and not per test.
- **Generics.** Reference A substitutes **literal** generic values into the instance before
  lowering (so a width generic is a constant in the code and loops and vector sizes fold);
  non-literal values, types, subprograms and packages stay as data or are mapped. Instances with
  the same architecture and the same literal generics **share** their lowered code (the
  process code is named by the first such instance, and each instance only creates its own
  storage and processes pointing at the shared code). The check is one hash of the generic
  values and the binding. Reference B appears to compile each architecture once and pass
  generics as run-time parameters of the instantiation routine, not folding them.
- **Static evaluation.** Folding of constant expressions, including calls of user functions in
  declarations, uses the same execution engine as the simulation (the interpreter) on small
  thunks, with no separate evaluator.
- **Storage of analysed units.** Reference A keeps a library unit as one contiguous block of
  objects that is read in one piece and relocated, and is read-only after freezing; units are
  loaded on demand by name, so loading `std` and `ieee` costs time proportional to the units
  referenced (appears). Reference B's start-up of 0.3 to 0.4 s on the library testbenches shows
  that translating the reachable subprograms with a one-pass generator is cheap enough that no
  cache is needed.
- **No persistent code cache.** Neither reference simulator caches lowered or compiled code
  between runs in the normal flow. Their small start-up comes from doing little (laziness) and
  doing it cheaply (one-pass code generation, or interpretation first), not from caching.

### Impact

Reference start-up on the verification-library testbenches is 0.2 to 0.4 s against yavs 0.6 to
8.8 s interpreted (and 20 to 57 s with the LLVM tier compiling everything). Per-test
elaboration of 3.2 s median on small test-runner tests (#383) is the same effect multiplied by
579 tests.

### yavs today

yavs already lowers subprograms lazily by reference (`yavs-lower/DESIGN.md` "Shape"), shares
one `Func` per unit across instances with generics as instance data (`ir.md` 2), and runs
elaboration-time calls in the interpreter. Where it differs: elaboration-time calls with
package state replay a log against a rebuilt runtime (`docs/decisions.md` "elaboration-time
session", #383, now grown incrementally); every signal of a loaded package is elaborated;
constants of each run-time package are checked at lowering; and no measurement separates the
cost of loading the analysed units of large libraries from lowering and from elaboration-time
evaluation.

### Recommendations

**R7 (rank 5): measure the start-up split, then remove the largest item.** Add a `--verbose`
timing breakdown (load of analysed units, lowering, package initialisation, elaboration-time
calls, instance building, kernel build, LLVM compile) and run it on designs 3, 4 and 6 and on
the 3.2 s median small test. Then, in the order the data suggests:

- *lowering granularity*: confirm that nothing lowers a whole package body when one function is
  referenced (constants of run-time packages are checked at lowering: check that this does not
  pull in every body);
- *package initialisation on first use*: initialise a package when the first process or
  subprogram that refers to it runs, rather than eagerly for every loaded package; elaborate
  package signals only when referenced;
- *one engine for static evaluation*: the elaboration-time session should be the simulation's
  runtime with package data initialised once and kept (no log replay);
- *literal-generic specialisation (optional, later)*: the reference that does it gets
  constant-folded vector sizes in hot code, but it multiplies code for instances with different
  values and costs lowering time. The yavs choice (generics as data, one `Func` per unit) is
  the right default for start-up; specialise only on the LLVM tier, for a function that is hot
  *and* whose generic values are literal, never in the interpreter;
- *analysed-unit loading*: if loading the analysed units dominates, store each unit so that
  loading is proportional to what is referenced (per-unit sections loaded on demand). That
  changes the stored format in `yavs-lib`/`yavs-sem`; it is not a cache of derived code.

Crates: `yavs-elab` (package initialisation, session, specialisation hook), `yavs-lower`
(laziness audit), `yavs-lib` and `yavs-sem` (loading), `yavs` (the timing breakdown).

## 5. JIT and AOT strategy

### Mechanism in the reference simulators

- **Reference A: interpret first, compile hot functions in the background.** Every function
  starts in the bytecode interpreter with a countdown. At about a hundred calls it is queued to
  a worker thread that translates **that one function** (in its own compiler module and
  context, so jobs are independent) and swaps the entry point atomically. The compiler runs at
  its cheapest setting: no optimisation but a small clean-up by default, and the first
  optimisation level only when callees were inlined into the function being compiled. Loop
  vectorisation, interleaving and unrolling are off. Compile time per function is therefore
  small, and there is no big up-front compile. A function that is called once (a process body
  running a long loop) is not counted up by calls and stays interpreted.
- **Reference B: compile everything reachable, fast, before time 0.** The code generator is its
  own single-pass emitter (no intermediate optimiser). It does not need a threshold because its
  cost per construct is tiny; the price is code that is not as fast as an optimised build (its
  manual says the in-memory back end is the fast-to-start one and the back ends that write
  executables the fast-running ones).
- **No precompiled `std` and `ieee`** on the fast-to-start path of either (A lowers and compiles
  them lazily each run; B generates them each run).
- **Intrinsics cover the hottest library code** (section 1), so the library does not depend on
  the compiler tier to be fast.

### Impact

For yavs, `docs/STATUS.md` shows compiling every function at the second optimisation level before
time 0 costs 19 s to 57 s for 120k to 220k instructions (0.15 to 0.26 ms each) and makes every
total worse than interpreting (the corpus goes from 9.4 s to 174 s). The tier gains nothing on
signal-heavy and testbench designs (simulation within noise) because 71% of what stays
interpreted is memory and address kinds that still call back, and the interpreter's cost is in
calls and kernel interaction, not in arithmetic. So: do not compile everything; compile what is
hot, cheaply, off the critical thread.

### Recommendations

**R8 (rank 4): tier-up as the reference does.** Implement #125/#126 in this shape:

- per-function counters in a side table (call count; for a process body, resumption count; for a
  long-running loop in a function called once, a back-edge count so it can tier up at the next
  suspension or call);
- a threshold of the order of a hundred calls (the user ratifies the number, #126; the
  reference figure is a starting point to measure);
- one function per compile job, own module, background thread(s), entry swap at function entry
  or process resumption (already the design in `ir.md` 5);
- the cheapest optimisation setting by default (no inlining, no vectorising, no unrolling),
  raised only for a function that benefits, and never above the second level;
- keep the size cap of #126 as a second guard.

With this, start-up with `--tier=llvm` returns to the interpreted level, and the compiled code
only has to be faster than the interpreter on what is hot. Crates: `yavs-llvm`, with
`yavs-interp` (counters, swap).

**R9 (rank 4, with R8): widen what compiled code runs natively.** The tier compiles 53% to 68%
of instructions inline but the memory and address kinds (`Load`, `AddrOf`, `Store`, `Alloc`,
`Copy`: 71% of what stays interpreted over the corpus, #334) are side exits. A compiled loop
that calls back for every load is slower than the reference's whole run (section 3). Lower
`Load`/`Store`/`AddrOf`/`Index` for the frame, instance and constant spaces inline with the
bounds check as a compare and a side exit, hold the space base pointers in function-entry
registers, then the descriptor kinds (#335) and calls (#336). The native fast paths of R1 are
called directly. Crate: `yavs-llvm`. The interpreter stays the reference: every guard failure
re-executes in the interpreter for the identical error, as now.

**R10: ahead-of-time compiled `std` and `ieee`.** Not recommended: neither reference simulator
does it on its fast-to-start path, and R1 plus laziness gives the same effect without a build
artefact. Listed so the question is closed on the record.

## 6. Memory layout of composites and logic vectors

### Mechanism in the reference simulators

- **Scalars are sized by their range.** Integers and enumerations use 1, 2, 4 or 8 bytes
  according to the declared range; reals 8 bytes. A `std_ulogic` is one byte holding its
  position; a vector of them is one byte per element, contiguous.
- **Arrays** are flat, row-major, elements contiguous, no per-element header. An unconstrained
  array parameter is a pointer plus a bounds record (left, length with the direction folded in).
  Records are laid out sequentially with natural alignment. Layouts are computed once per type
  and cached.
- **Signals have the same shape as variables.** The current value of a signal is read through a
  pointer to its storage, so reading a signal in compiled code is a load, not a call; flags such
  as "had an event" are cached in the signal header for the common test. A signal is split into
  pieces only as far as sources and sensitivity differ.
- **Vector logic by table or SIMD:** element-wise operations over bytes use lookup tables and,
  where available, 16-byte shuffles (16 elements per instruction).

### yavs today and comparison

Bit-planes (`ir.md` 1.3) hold 64 elements in four 64-bit words, so a bulk operation handles 64
elements per instruction, four times the width of the reference's SIMD, and "is every element
two-valued" is a single test. Element access (`LGet`/`LSet`) costs more than a byte load (block
index, four plane extractions or inserts), and a scalar `std_ulogic` is already a bare code in a
register or slot. Integers are always 8 bytes in memory by rule 7. For big integer arrays
(memory models in testbenches) the reference layout is 2x to 8x smaller; the peak RSS of design
3 (313 MB against 144 MB) includes such effects.

### Recommendations

**R11 (rank 9): keep bit-planes; make element access cheap.** No change of layout. Ensure
`LGet`/`LSet` compile to a few inline instructions in the LLVM tier (R9), that the
interpreter's `Index` on a logic array is a fused index-and-extract, and that bulk paths
(slices, concatenation, copies at any alignment, comparisons) stay word-wise (they are).
Crates: `yavs-llvm`, `yavs-interp`.

**R12 (rank 10; step 1 done, #415): narrow storage of integer arrays.** Done: integer types within
32 bits are 4 bytes as array elements and in kernel signals (docs/STATUS.md "Narrow integer storage": a
large integer array uses a quarter less peak memory, the library benchmarks are unchanged). Step 2, 1- and
2-byte elements from subtype ranges, is not worth it for the measured workloads. The original text:
Store integer
elements of large arrays (above a size threshold, in memory only, widened on load, narrowed
with the existing range check on store) in the smallest width that holds the subtype range.
Rule 7 says integers are `i64` in memory; this is a deliberate exception for memory size and
cache behaviour. Only if memory-model-heavy testbenches show a memory or cache problem after R1
to R9. Crates: `yavs-lower` (layout), `yavs-interp`, `yavs-llvm`.

## 7. Deterministic parallel kernel: what to keep

The reference simulators are serial in the simulation loop; their background threads only
compile. The yavs parallel run phase with ordered commit is an asset that neither has, but it
only pays when per-activation work is large. After R2 and R6, a batch of small processes is
cheaper to run inline than to hand to a pool (R5). Parallel update of independent signals
(#103) keeps the same rule: no new parallelism before the single-thread cost per activation is
within a small factor of the reference. Everything in sections 2 and 3 is per-process or
serial-step work and independent of the thread count by construction.

## 8. Decisions and approvals needed from the user

1. **Native `math_real` (R1b):** C library results (fast, may differ in the last bit from the
   VHDL body) or bit-exact reproduction of the body (already #395).
2. **Tier-up threshold, size cap and default optimisation level (R8, #126).**
3. **Collapsing `in` ports and component pass-through (R4):** changes the recorded choice of
   uncollapsed ports for the `in` case (conversion ports stay as they are); alias metadata
   keeps names for waves, VHPI and DAP.
4. **Activity-observable analysis (R2b):** skipping unchanged zero-delay assignments is
   unobservable only if the observer set is complete. Confirm that any new observer kind
   defaults to "observes activity".
5. **Narrow integer array storage (R12):** an exception to rule 7.
6. **No incremental cache** is proposed anywhere; the reference simulators have none either. If
   start-up still misses the target after R7, a cache of lowered library code is the next lever
   and needs the user's approval.

## 9. Ranked recommendations

Order is expected gain per unit of effort for the workloads in `docs/STATUS.md` (library testbenches
first, then RTL). Gains are estimates (section 0). "Gate" is the measurement that must show the
effect before the item is called done.

| Rank | Id | Recommendation | Crates | Expected effect | Gate |
|---:|---|---|---|---|---|
| 1 | R1 | Native fast paths for hot ieee subprograms, fallback to the VHDL body when any element is not 0/1 or a corner applies | yavs-lower, yavs-interp, yavs-logic, yavs-ir, yavs-llvm (same runtime call) | 100x on the operation; 30% to 60% fewer interpreted instructions in library testbenches | 16-bit `+` loop from 250 us to under 1 us; difftest identical including metavalue and warning texts |
| 2 | R6 | Interpreter constant factors: frame stack instead of two allocations per call, hoisted space bases, inline checked arithmetic, no mark phase per suspension | yavs-interp (yavs-ir if a bit is missing) | 3x to 5x on call-heavy code; possibly a large share of the 13 us per activation | design 1 under 400 ns per iteration; call and return under 100 ns; profile of design 2b before and after |
| 3 | R2 | Kernel: allocation-free single-driver assignment (a), skip unchanged (b), static sensitivity (c), edge triggers (d) | yavs-kernel, yavs-elab, yavs-lower, yavs-ir, yavs-interp | 5x to 10x on RTL designs (half the activations, near-free assignment) | designs 2a and 2b under 2 s; identical output at 1, 4, 8 threads and shuffle |
| 4 | R8, R9 | Tier-up of hot functions only, background, cheap optimisation, per function; inline memory kinds | yavs-llvm, yavs-interp | start-up with the tier back to the interpreted level; compiled loops faster than the interpreter | corpus total with `--tier=llvm` within 10% of interpreted; design 1 compiled under 100 ns per iteration |
| 5 | R7 | Start-up breakdown, package initialisation on first use, one runtime for static evaluation, on-demand unit loading | yavs-elab, yavs-lower, yavs-lib, yavs-sem, yavs | design 3 start-up from 8.8 s to under 2 s; 3.2 s median small test to under 1 s | `--verbose` breakdown recorded in docs/STATUS.md before and after |
| 6 | R4 | Collapse `in` ports and component pass-through | yavs-elab, yavs-kernel | less update work per delta in hierarchical RTL; fewer signals | port-heavy design before and after; names unchanged in waves and VHPI |
| 7 | R3 | Memoised resolution tables for small enumerations, identity detection | yavs-kernel, yavs-elab | buses of user resolved types from a call per element to a lookup | bus bench with a user resolution function |
| 8 | R5 | Run small batches inline | yavs-kernel | removes pool hand-off from the per-activation cost | only after R2 and R6; bench at 1 and 8 threads |
| 9 | R11 | Cheap logic element access in both tiers | yavs-llvm, yavs-interp | element loops on logic arrays | loop over a `std_logic_vector` by index |
| 10 | R12 | Narrow storage for large integer arrays | yavs-lower, yavs-interp, yavs-llvm | memory and cache for memory models | peak RSS of design 3; user decision |

Not recommended: a persistent code cache, ahead-of-time compiled libraries (R10), a second
interpreter, threaded-code dispatch (the reference interpreter is a plain switch too),
specialising instances by literal generics in the interpreter, and giving up the delta-cycle
structure for speed. The first two do not address the measured costs; the others give less than
the items above.

Hand-off: the elaboration agent takes R7 and R4 (and the analysis half of R2b and R2c); the
interpreter agent R6 and the execution half of R1, R2c and R2d; the kernel agent R2, R3 and R5;
the native-IEEE agent R1 (with `yavs-logic`); the LLVM agent R8, R9 and R11.
