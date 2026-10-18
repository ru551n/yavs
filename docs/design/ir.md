# M2 middle end: rsim-ir, rsim-interp, rsim-elab

Status: design, not implemented. Binding inputs: `docs/plan.md` (architecture table, "Designed for
the GUI", "Parallel kernel design", rules 1 to 12, the error-handling motto) and
`docs/decisions.md`. Interfaces read from: the kernel worktree (`rsim-kernel`: `Process`, `Ctx`,
`Builder`, `SimControl`, `Observer`, `SimError { trace: Vec<StmtId> }`), `rsim-logic`,
`rsim-wave`, the LLVM spike (`rsim-llvm`, `STATUS.md`), `rsim-std` (`arith`), `lib/NATIVE.md`,
the VHPI stub. The HIR is specified in `docs/design/sem.md`; section 0 lists what this design
assumes of it.

```
HIR (rsim-sem) --lower, once per design unit--> Program (rsim-ir, immutable, Arc)
                                                    |
     elaborate(design, top, generics, cfg) ---------+--> Design { program, hier, nets, procs }
     (rsim-elab, runs elab code in rsim-interp)                       |
                                                    Design::build() --> rsim_kernel::SimControl
                                                    (InterpProcess per process instance)
```

## 0. Assumptions about the HIR (to reconcile with sem.md)

| # | Assumption | Why the IR needs it |
|---|---|---|
| H1 | Arena ids (`UnitId`, `DeclId`, `ExprId`, `SeqStmtId`), each node with a `rsim_diag::Span` | statement ids, spans on every instruction |
| H2 | Every expression carries a resolved `TypeId` and, where known, a `SubtypeId`; subtype constraints are HIR expressions (static or not), never pre-folded values | rule 1: lowering emits descriptor computations |
| H3 | Every call names one `SubprogramId`. Predefined operations are `Implicit { family, type }` (NATIVE.md section 2); bodiless std subprograms carry their native symbol; `attribute foreign` subprograms are marked foreign with the attribute string | three call kinds |
| H4 | Names are resolved to declarations; indexed/sliced/selected names stay structured; each name has a staticness flag (locally / globally / none) | longest static prefix, driver map |
| H5 | Predefined attributes are an enum on a resolved prefix (`'event`, `'length`, `'image`, ...) | direct lowering |
| H6 | Concurrent statements stay as written (not desugared to processes); sem gives their sensitivity sets for `process (all)` and concurrent assignments | the IR does the LRM 11 equivalence |
| H7 | Generic declarations of every kind (constant, type, subprogram, package) are declarations; instances and generic maps are not expanded | per-instance data, shared code |
| H8 | Identifiers keep original spelling plus a folded key | paths, VHPI CaseName, waves |
| H9 | Locally static expressions may be folded by sem, but any expression may still be lowered (sem keeps the tree) | elab folds the rest through the interpreter |
| H10 | 2019 mode views resolved to per-field modes on the port | port splitting |
| H11 | `wait` inside a function, and a procedure that can wait called from a function, are rejected by sem | functions never suspend |

## 1. The IR (`rsim-ir`)

### 1.1 Program, units, ids

All ids are `u32` newtypes (rule 5). A `Program` is built once from the HIR and is immutable and
`Send + Sync` afterwards.

```rust
pub struct Program {
    pub funcs: IdVec<FuncId, Func>,        // processes, subprograms, elab bodies, thunks
    pub types: IdVec<TyId, Ty>,            // runtime-shape descriptions (1.3)
    pub consts: ConstPool,                 // compact constant data (rule 12)
    pub stmts: IdVec<StmtId, StmtInfo>,    // global: StmtId is rsim_kernel::StmtId
    pub natives: IdVec<NativeId, NativeRef>,
    pub foreign: IdVec<ForeignId, ForeignRef>,
    pub cov: IdVec<CovItem, CovInfo>,
    pub units: IdVec<UnitIr, UnitIr>,      // per design unit: elab func, decl slots, children
}

pub struct Func {
    pub name: Ident,                       // original spelling (rule 8)
    pub kind: FuncKind,                    // Process | Procedure | Function | Elab | Thunk
    pub mangled: Mangled,                  // unique per generic instance identity (rule 4)
    pub params: Vec<Param>,                // (slot, class, mode, TyId)
    pub frame: FrameLayout,                // slot offsets/sizes + root map (1.4)
    pub nregs: u32,
    pub reg_ty: Vec<RegTy>,                // I64 | F64 | Ptr | Sig
    pub blocks: IdVec<BlockId, Block>,
    pub code: Vec<Inst>,                   // blocks index ranges into this
    pub loc: Vec<StmtId>,                  // parallel to `code`: statement of each instruction
    pub resume: Vec<InstIdx>,              // resume points (after each Wait / suspending call)
    pub flags: FuncFlags,                  // MAY_SUSPEND, TOUCHES_SHARED, PURE, HAS_COVER
    pub size: u32,                         // instruction count, for the LLVM threshold
}

pub struct StmtInfo { pub span: Span, pub func: FuncId, pub kind: StmtKind, pub parent: Option<StmtId> }
```

`StmtId` is global across the program, so the kernel's `SimError::trace: Vec<StmtId>` resolves
to span, statement kind and enclosing subprogram without any other table.

### 1.2 Registers and passes

- Registers are untyped 64-bit cells; `reg_ty` exists for the verifier and the LLVM tier.
  `RegTy::Ptr` is an `Addr` (1.4), `RegTy::Sig` a signal reference `SigVal` (packed
  `{base: u32 index into the instance's signal table, offset: u32, len: u32}` in two cells).
- Lowering allocates registers with a per-statement free list: temporaries die at the end of
  their statement, so allocation is linear and registers are reused across statements. Objects
  (variables, loop parameters, constants with non-trivial values) live in **frame slots**, never
  in registers.
- **Suspension invariant:** no register is live across a `Wait` or a call to a `MAY_SUSPEND`
  procedure. Arguments are written into the callee's slots before the call, out/inout write-back
  addresses are kept in caller slots. The interpreter keeps registers anyway (frames are heap
  objects), but the LLVM tier relies on this: resuming needs only the frame memory and a resume
  index.
- Every pass is one walk over blocks in order: verifier (types, invariant above, every instruction
  has a `StmtId`), `MAY_SUSPEND` and `TOUCHES_SHARED` propagation (worklist over the reverse call
  graph, each edge visited once), coverage numbering. No pass is quadratic; a unit over the LLVM
  size limit (5) is still lowered and run in linear time (rule 5).

### 1.3 Types and value layout

One runtime description per VHDL type (rule 1). A composite subtype is a **runtime value** (a
descriptor in memory), produced by instructions; when its constraints are static the descriptor
is a constant in the pool. There is no second, static-only path.

```rust
pub enum Ty {
    Int  { lo: i64, hi: i64 },            // integer & physical: always i64 in memory (rule 7)
    Enum { lits: u32, logic: Option<LogicKind> },   // u8 if <= 256 literals else u32
    Real,                                 // f64
    Array   { elem: TyId, dims: u8, elem_resolved: bool },
    Record  { fields: Vec<(Ident, TyId)> },
    Access  { target: TyId },             // Handle (u64)
    File    { elem: TyId },               // FileId (u32)
    Protected { unit: UnitIr },           // Handle to instance state
}
pub enum LogicKind { StdULogic, Bit, Boolean }       // gets the rsim-logic fast paths
```

| Value | In memory | Notes |
|---|---|---|
| integer, physical | `i64` | ranges checked by `Check` (1.5); 2008 INTEGER is 32-bit by its range only |
| enum | `u8` / `u32` | std_ulogic is the rsim-logic code (`StdULogic as u8`) |
| real | `f64` | |
| array | elements contiguous, row major | bounds in a **dope**: `[left, right, dir]` per dimension, `i64` each |
| unconstrained array object/param | `{data: Addr, dope: Addr}` (fat) | the only array form passed to subprograms |
| record | fields at offsets from a `Layout` | offsets are runtime data when a field is unconstrained |
| access | `Handle` (heap, 1.6) | `null` = 0 |
| protected | `Handle` to its state | method call = `Call` with `self` first |
| file | `FileId` into the owner's file table | |

Layout of a fully constrained subtype is computed by `LayoutOf { dst, desc }` (size, field
offsets, element stride; all `u64`, rule 6), folded at lowering when the descriptor is constant.
Sizes are `u64` everywhere.

Constant aggregates (rule 12) are stored as `ConstData::{Bytes, Repeat{elem, n}, Runs(Vec<(n, elem)>)}`
and materialized by `CopyConst`, which expands runs straight into the destination; a 64K
`others => x` initialiser is 1 element in the pool.

**Logic vector storage:** M2 stores arrays of a `LogicKind` enum as 1 byte per element, which is
what the kernel's `SignalType { elem_size, len }` already stores and what FST/VHPI read.
`Layout` has a `packing: Packing::{Bytes, Planes}` field; only load/store/slice/copy instructions
look at it, so moving logic arrays to the bit-plane `LogicVec` (decisions.md) is local to those
instructions plus a kernel signal type. See open question Q1.

### 1.4 Addresses and frames

```rust
pub struct Addr(u64);  // [space: 3 bits | index: 29 bits | offset: 32 bits]
pub enum Space { Frame, Up /* via static link */, Inst, Pkg, Const, Heap, Temp }
```

- A frame is `{ func, pc, slots: Box<[u8]> (FrameLayout.size), regs: Box<[u64]>, link: Option<FrameRef>, inst: InstRef }`.
  `link` is the static link for nested subprograms (`Up { depth, slot }` addressing); `inst` the
  instance data of the enclosing design unit.
- Large locals and temporaries are in `slots` or `Temp` (a per-process bump area reset per
  statement), never on the Rust stack (rule 6). Recursion depth is bounded by a setting with a
  clear runtime error, not by the native stack.
- `FrameLayout.roots: Vec<(offset, RootKind)>` lists every slot that holds a `Handle` or a fat
  pointer into the heap, with `RootKind::{Handle, Fat, Composite(TyId)}`; together with
  `reg_ty == Ptr` at suspension points (none, by the invariant) this gives precise roots (rule 6).

### 1.5 Instructions

Operands are `Reg` or small immediates; `Addr` operands come from registers. Every instruction
has a `StmtId` in `Func.loc`.

| Group | Instructions |
|---|---|
| move/const | `Const{dst, i64}`, `ConstF{dst, f64}`, `Move{dst, src}`, `AddrOf{dst, space, slot}`, `CopyConst{dst_addr, const}` |
| memory | `Load{dst, addr, w}`, `Store{addr, src, w}`, `Copy{dst, src, bytes}`, `Fill{dst, elem, n, size}`, `Index{dst, base, dope, idx...}` (bounds-checked), `Field{dst, base, layout, n}` |
| int (checked, rule 7) | `Add`/`Sub`/`Mul`/`Div`/`Rem`/`Mod`/`Pow`/`Neg`/`Abs` on i64: overflow and zero divisors raise `Runtime` through `rsim_std::arith`; never wrap |
| real | `FAdd`..`FPow`, `FCmp`, `IToF`, `FToI` (checked round per LRM, `real_to_int`) |
| compare/logic | `Cmp{op, dst, a, b}`, `And/Or/Xor/Not` on booleans, `LogicOp{op, dst, a, b, n}` on logic arrays via rsim-logic tables |
| checks | `Check{kind: Range\|Index\|Length\|Null\|Discrete, v, lo, hi, what: CheckMsgId}`; `lo/hi` are regs, so runtime bounds and static bounds use one instruction |
| subtypes | `MakeDope{dst, dims...}`, `DopeLen{dst, dope, dim}`, `LayoutOf{dst, desc}`, `SubtypeCheck{value, desc}` (composite conversion and constraint checks) |
| control | `Br{blk}`, `CondBr{c, t, f}`, `Case{v, table: CaseTableId, default}` (ranges and choices as sorted table), `Ret`, `Unreachable` |
| calls | `Call{func, args}` (VHDL), `CallNative{id, args}` (NATIVE.md symbols and implicit families), `CallForeign{id, args}` (VHPIDIRECT), `CallIndirect{func_reg, args}` (subprogram generics), `CallProt{func, obj, args}` (protected method: takes the instance lock, 3.5) |
| heap | `New{dst, ty, init}`, `Free{handle}` (`deallocate`), `Deref{dst, handle}` (null check included) |
| signal read | `SigVal{dst, inst_sig, offset, len}` builds a signal reference; `SigRead{dst_addr, sig}` (effective value), `SigReadScalar{dst, sig}`, `SigDriving{dst_addr, driver_slot}` (`'driving_value`, out-port read) |
| attributes | `SigAttr{dst, kind: Event\|Active\|LastEvent\|LastActive\|LastValue\|Driving, sig}`; `'stable/'quiet/'delayed/'transaction` are implicit signals made by elab and read with `SigRead` |
| signal write | `Drive{driver_slot, target: Option<sig>, waveform: Addr, n, mode: Transport\|Inertial{reject: Option<Reg>}}` (waveform = `n` `(time, value)` pairs in a temp, ascending); `Force{sig, mode: In\|Out, value}`, `Release{sig, mode}`; `Disconnect{driver_slot}` (null transaction, guarded, M3) |
| wait | `WaitOn{sig}` (adds to sensitivity), `WaitFor{t}`, `WaitAt{abs}`, `Suspend{resume}`; `TimedOut{dst}`, `Now{dst}`, `Delta{dst}` |
| effects | `Report{msg: Addr, len, severity}` (assert = branch + `Report`); `Output{stream, addr, len}` is only used by natives |
| coverage | `Cover{item: CovItem}` (statement/branch counter increment, rule 10), toggle coverage is an observer, not IR |
| debug | `Stmt{id}`: statement boundary; a nop in the fast dispatch loop, the breakpoint/step hook in the debug loop |
| elab only | `ElabSignal`, `ElabDriver`, `ElabProcess`, `ElabInstance`, `ElabPort`, `ElabExternal` (3.6, 4); the verifier rejects them outside `FuncKind::Elab` |

Lowering of the main statement forms:

| VHDL | IR |
|---|---|
| `wait on a, b until c for t;` | `deadline := Now + t` into a slot; `L: WaitOn a; WaitOn b; WaitAt deadline; Suspend r; TimedOut -> exit; c -> exit; Br L` (kernel contract: the process re-suspends with the same absolute deadline) |
| `process (s) ... end process` | body, then `WaitOn s; Suspend` at the end (LRM 11.3) |
| concurrent assignment / assert / procedure call | an equivalent process (LRM 11.4 to 11.6), sensitivity from H6, `Func.kind = Process`, flag `IMPLICIT` |
| `s(i) <= x after 5 ns;` (`i` not static) | driver slot = longest static prefix `s`; `target = SigVal(s, offset(i), 1)` computed at run time |
| `s <= x when c else y;` | branches + one `Drive` per arm into the same driver slot |

### 1.6 Heap, access types, protected types, files

- `Heap` is a growable arena of `Slot { gen: u32, ty: TyId, data: Box<[u8]> }`; a `Handle` is
  `{heap: u8, index: u24, gen: u32}`, so a stale handle after `deallocate` is a clean
  `null/dangling access` error, not UB. No conservative scanning (rule 6).
- **Ownership:** each process owns a private heap. Objects reachable from a shared variable or a
  protected instance live in the **shared heap**, used only by serial processes in M2/M3 (3.5).
  The `heap` tag in every handle lets the interpreter check this; a cross-heap handle is an ICE.
- **GC:** a process heap is collected when its process suspends and allocation since the last
  collection exceeds a threshold; roots are that process's frames (root maps). The shared heap is
  collected at the end of a delta, when no process runs. No GC ever waits for another thread
  (NVC #1649 cannot happen).
- **Protected:** an instance is a heap object holding its private variables; methods are `Func`s
  with `self` as the first parameter. `CallProt` takes the instance's lock (3.5).
- **Files:** `FileId` indexes the owner's file table (process or shared). Reads are immediate;
  writes to `OUTPUT` go through `Ctx::output` (buffered, committed in elaboration order); writes
  to user files are buffered the same way under a per-file stream id (kernel ask K6).

## 2. Processes, subprograms, suspension, shared code

- **One `Func` per process or subprogram per design unit**, shared by every instance. Generics
  (constant), signals, drivers and declared constants are per-instance data read through
  `Space::Inst`, never baked in. This keeps code size linear in the source, not in the number of
  instances (NVC #1331).
- Instance data `InstData { generics, consts, sigs: Box<[SigBase]>, child: Box<[InstRef]>, pkg: ... }`
  is computed by the unit's `Elab` function and frozen (read-only) before simulation.
- **Type generics and generic subprograms** are runtime data too: a type generic is a descriptor
  plus a `TyId` class, a subprogram generic is a `FuncId` in the instance data called through
  `CallIndirect`. One body per unit; specialisation per actual is an optional later pass (Q4).
- **Generic package instances** each get their own `InstData` and `Mangled` identity (rule 4,
  NVC #1492/#1408); code is shared with the uninstantiated package.
- **Suspension:** a process is a frame stack `Vec<Frame>` owned by its `InterpProcess`. `Suspend`
  returns from `Process::run` with the stack intact; the next `run` continues at the top frame's
  `pc`. A `wait` inside a procedure therefore needs nothing special: the procedure frame is just
  on the stack (frames are on the heap). Each `Suspend` names a resume index (`Func.resume`),
  which is the state number for the LLVM tier's resumable state machine.
- Functions cannot suspend (H11); a `MAY_SUSPEND` procedure called from a function is an ICE in
  the verifier.

## 3. Interpreter (`rsim-interp`)

### 3.1 Dispatch

- `Inst` is a Rust `enum` with `#[repr(u8)]`-sized tag and inline operands (8 to 16 bytes); a
  `match` loop over `code[pc]` with a local `pc`, `regs` and `slots` (rustc compiles it to a jump
  table). No threaded code, no `unsafe` in M2.
- Two loop instances: `exec::<false>` (fast: `Stmt` is a nop, no hook checks) and
  `exec::<true>` (debug: `Stmt` calls the debugger hook). The process switches loops when a
  debugger is attached.
- Fuel: the debug loop also counts instructions so a `pause` request interrupts an endless loop
  inside a process (fast loop: `run_for` timeouts are the user's guard).

### 3.2 Implementing the kernel `Process`

```rust
pub struct InterpProcess {
    prog: Arc<Program>,
    inst: Arc<InstData>,
    drivers: Box<[DriverId]>,          // driver slot -> kernel DriverId, from elab
    frames: Vec<Frame>,                // the resumable state
    heap: Heap,                        // private heap (1.6)
    files: FileTable,
    temp: Temp,
    shared: Option<Arc<SharedState>>,  // Some only for serial processes
    debug: Option<Arc<dyn Debugger>>,
}

impl rsim_kernel::Process for InterpProcess {
    fn run(&mut self, ctx: &mut Ctx<'_>) -> Result<(), SimError> {
        // continue frames.last(); signal ops -> ctx.value/assign/wait_on/wait_at/report/force
        // on error: ctx.error(kind) with trace = stmt of each frame, innermost first
    }
}
```

| Instruction | Kernel call |
|---|---|
| `SigRead`, `SigReadScalar` | `ctx.value(SigRef)` then copy/decode |
| `SigAttr` | `ctx.event/active/last_event/last_active/last_value` |
| `Drive` | `ctx.assign(drivers[slot], target, waveform, DelayMode)` |
| `Force`/`Release` | `ctx.force/release(r, ForceMode::Effective\|Driving)` |
| `WaitOn`/`WaitFor`/`WaitAt`/`TimedOut` | `ctx.wait_on/wait_for/wait_at/timed_out` |
| `Report` | `ctx.report(sev, msg)` (message formatting is rsim's, the kernel only buffers) |
| `Now`/`Delta` | `ctx.now()/delta()` |
| natives `env.stop/finish` | `ctx.stop/finish` |

### 3.3 Diagnostics and stack traces

- A runtime error builds `SimError` through `ctx.error(ErrorKind::Runtime(msg))` and fills
  `trace` with the current `StmtId` of every frame, innermost first. rsim-diag renders each frame
  from `Program.stmts` (subprogram name, file:line, snippet) and adds the process instance path,
  time and delta from `SimError`. An assert inside `numeric_std` therefore shows the user's call
  site as the second frame (plan, rsim-diag row).
- Check failures carry a `CheckMsgId` with the static part of the message ("index 8 out of range
  7 downto 0 for s") so the error names the object, the value and the bounds.
- A broken interpreter invariant returns `ErrorKind::Internal` at the current statement's span:
  "this is a bug in rsim" (exit 101), never a bare panic.

### 3.4 Breakpoints and stepping

```rust
pub trait Debugger: Send + Sync {
    /// Called at each `Stmt` in the debug loop. Blocks while the user inspects.
    fn on_stmt(&self, at: StmtId, view: &FrameView<'_>, ctx: &Ctx<'_>) -> Resume;
}
pub enum Resume { Continue, StepIn, StepOver, StepOut }
```

- Breakpoints are a bitset over `StmtId`, checked only in the debug loop. Step over/out compare
  the frame depth. While a process is stopped inside `run`, the DAP reads its frames
  (`FrameView`: slots decoded by `Layout`) and current signal values through `ctx`.
- Pausing inside a process requires `--threads 1` (the kernel is mid-delta); the CLI forces it
  when a debugger is attached with breakpoints. Pausing at a delta or time step goes through
  `SimControl` as usual.

### 3.5 Thread safety

| State | Owner | Access |
|---|---|---|
| `Program` (code, types, consts, stmt table) | `Arc`, built before the run | shared read-only |
| `InstData` of instances and packages (non-shared parts) | `Arc`, frozen after elab | shared read-only |
| frames, private heap, temps, files of process-local file objects | the `InterpProcess` | `&mut` in `run` only |
| driver queues, effects, sensitivity | kernel `Ctx` | per process (kernel design) |
| shared variables, protected instances, shared heap, files declared outside processes | `SharedState` | only from processes marked serial (below) |
| coverage counters | `Arc<[AtomicU64]>` indexed `inst.cov_base + item` | relaxed `fetch_add`; sums are order independent |

- **Serial marking:** at elaboration a process is `ProcFlags { serial: true }` when its body or any
  reachable subprogram has `TOUCHES_SHARED` (shared variable, protected call, shared file, or a
  package variable). Computed once per `Func` by the linear propagation in 1.2, looked up per
  instance.
- **Protected locking (M2/M3):** every protected caller is serial, so the kernel already runs it
  alone and `CallProt` takes no lock. `// ponytail: protected calls serialised via serial
  marking; per-instance locks when VUnit-scale benchmarks show serial processes dominate`. The
  per-instance lock design (Q3) only changes `CallProt` and the marking rule.
- `--threads 1` and `--threads N` produce identical output because all observable effects go
  through the kernel's buffered effects, and serial processes see a deterministic order.

### 3.6 Elaboration-time evaluation

The interpreter also runs `FuncKind::Elab` and `Thunk` functions without a kernel: an
`ElabCtx` implements the same signal-free subset (no `Wait`, no signal reads; using one is a
"signal read during elaboration" diagnostic). Generic actuals, generate ranges, port-map
expressions, constant declarations, subtype descriptors and signal initial values are all
computed this way, so elaboration and simulation share one evaluator (rule 4, rule 5).

## 4. Elaboration (`rsim-elab`)

```rust
pub fn elaborate(
    design: &rsim_sem::Design,           // analyzed HIR, read-only
    top: UnitId,
    overrides: &[(String, String)],      // -g name=value, parsed with the normal value parser
    cfg: &ElabConfig,                    // std revision, collapse ports, limits
) -> Result<Elaborated, Vec<Diagnostic>>;

pub struct Elaborated {
    pub program: Arc<Program>,
    pub hier: Hierarchy,                 // scopes and objects, stable ids and paths
    pub nets: NetGraph,                  // signals, drivers, ports, implicit signals
    pub procs: Vec<ProcInst>,            // in elaboration order = kernel ProcId order
}
impl Elaborated { pub fn build(&self, kcfg: rsim_kernel::Config) -> Result<SimControl, SimError>; }
```

Pure and re-entrant: no globals, no interior mutability shared across calls; `build` can be
called again for an in-process restart (rule 4, GHDL #1184).

| Step | What happens | Errors (fail early, with instance path) |
|---|---|---|
| 1. Reachability | units reachable from `top` through instances, configurations, packages; lowered once each into `Program` | missing unit names where rsim looked (library manifest) |
| 2. Packages | elab functions of packages in dependency order; generic package instances get `InstData` + mangled identity | |
| 3. Generics | `-g` overrides parsed as VHDL literals of the generic's type (rule 4, GHDL #1117), then generic map thunks evaluated in the parent instance | missing generic without default; type mismatch; bad override text |
| 4. Declarations | the unit's `Elab` function: constants, subtype descriptors, signals (`ElabSignal`), shared variables, protected instances | bound/constraint failure at a declaration |
| 5. Generates | `for`: evaluate range, one child scope per value, path `label(i)`; `if`/`case`: evaluate and pick one alternative (2008 alternative labels) | non-static range use errors |
| 6. Binding | component instance: configuration declaration, then configuration specification, then default binding (LRM 7.3.3); entity instance directly | unbound component (warning per LRM, then open); port/generic mismatch between component and entity |
| 7. Ports | association per formal sub-element (`ElabPort`), see below | type/mode mismatch, unassociated in-port without default |
| 8. Processes | `ElabProcess` per process: drivers from the process's static driver list (per `Drive` slot, evaluated for this instance), flags (postponed, serial) | multiple drivers on an unresolved signal (LRM 14.7.2), names both processes |
| 9. External names | bound in elaboration order against the partial hierarchy (GHDL #548) | target not found or not yet elaborated, with the path tried |
| 10. Check | net graph verified (every port connected or defaulted, every driver has a target) | |

**Ports (rule 3).** Each formal is its own signal in the net graph. Association kinds:

| Association | Net graph | Kernel |
|---|---|---|
| actual signal, no conversion, collapsing on | formal aliases actual's `SignalId` | nothing |
| actual signal, no collapse / mode `inout`/`buffer` | `Port { formal, actual, mode }` edge | effective/driving value relation in the update phase (kernel ask K1) |
| conversion function or type conversion | `Port { .., conv_in/conv_out: FuncId }` | converter callback runs the IR function in the update phase (K1) |
| expression actual (2008) | implicit process `formal <= expr` sensitive to the expression's signals, `IMPLICIT` | ordinary process + driver (LRM 6.5.6.3 defines it this way) |
| `open` / default | initial value only | |

Port collapsing is a separate pass over the net graph, off by default in M2 (`--collapse-ports`),
with every difftest group run both ways (rule 3, Q2).

**Signal net graph and sub-element driver map (rule 2).**

```rust
pub struct NetGraph {
    pub objs: IdVec<ObjId, SigObj>,      // VHDL signal objects: path, type, kind, kernel leaves
    pub leaves: IdVec<LeafId, Leaf>,     // one kernel signal each
    pub drivers: Vec<DriverDecl>,        // (process, LeafId, element Range)
    pub ports: Vec<PortEdge>,
    pub implicit: Vec<ImplicitSig>,      // 'stable/'quiet/'delayed/'transaction/guard
}
pub struct Leaf { pub kernel: SignalId, pub ty: SignalType, pub resolved: Option<FuncId>, pub obj: ObjId, pub path: SubPath }
pub struct SubElemMap;                    // (ObjId, &[Sel]) <-> Vec<(LeafId, Range)>, both directions
```

- **Leaves:** a signal is split into kernel signals by its resolution unit. A scalar or an array
  of scalars (resolved per element or not) is one leaf; a record is one leaf per field
  (recursively, arrays of records become one leaf per field, struct of arrays); a type resolved
  as a whole composite (`resolved_record_port`) is one leaf whose element is the whole composite
  value. Storage and event tracking are therefore per contiguous vector, never per scalar.
- `SubElemMap` is the one mapping from a VHDL name (object plus static index/field selectors) to
  `(leaf, element range)` pairs, and back. Drivers, force/deposit, VHPI handles, waves, coverage
  toggles and DAP all go through it.
- **Mode views (2019):** a port with a mode view is split per field (H10), each field getting its
  own mode and association; leaves line up with fields, so no extra machinery.

**Stable ids and paths.** `ScopeId`, `ObjId` and `LeafId` are assigned in elaboration order
(deterministic depth-first), so they are stable for one design and configuration. Every scope and
object has the LRM `'path_name` / `'instance_name` strings with original spelling; the same ids
are VHPI's object identity, the wave hierarchy's order, coverage's path (plus span hash, rule 10)
and the DAP variable references.

## 5. LLVM tier seam (M6)

What the spike (`rsim-llvm`, STATUS.md, decisions.md) fixes:

| Spike result | Consequence for the IR |
|---|---|
| Functions are `extern "C" fn(...) -> i32` status; runtime errors call a Rust callback and return a status; no unwinding through JIT frames | every `Check`, checked op and native call has an error edge to a "return status" exit; the interpreter's `Result` maps 1:1 |
| Runtime functions are absolute symbols | `CallNative`, signal ops and checked helpers lower to calls of the same `rsim-rt` functions the interpreter calls; no semantics are re-implemented in LLVM IR beyond i64/f64 arithmetic |
| A `#[repr(C)]` state pointer per process | frame slots and instance data have one byte layout (`FrameLayout`, `Layout`) used by both tiers |
| Background compile thread, atomic function pointer swap | `Func.native: AtomicPtr` (in a side table, `Program` stays immutable) |
| O2 time grows superlinearly: 0.15 s at 10k, 0.8 s at 40k, 3.8 s at 100k instructions | **threshold: `Func.size <= 40_000` IR instructions is compiled at O2, larger stays interpreted** (rule 5); an O0 band (40k to 300k) is an M6 option |

**Tier-up protocol.**

1. Each `Func` has an entry counter in a side table. When a process `Func` reaches N resumptions
   (default 1000) or a subprogram N calls, and `size <= 40k`, it is queued for compilation.
2. The compiled entry is `fn(frame: *mut u8, inst: *const u8, rt: *mut RtCtx, resume: u32) -> u32`;
   it switches on `resume` to the state after the matching `Suspend` (the suspension invariant
   makes the frame bytes the whole state). Return: `Done | Suspended(resume) | Error | CallInterp(func)`.
3. The swap happens only at a function entry or a process resumption, never mid-frame. Mixed
   stacks are allowed because both tiers use the same frame layout; a JIT frame calling an
   interpreted `Func` goes through an `rsim_rt_call_interp` callback.
4. The debug loop never runs JIT code: attaching a debugger with breakpoints pins processes to the
   interpreter.

**Identical semantics:** the interpreter is the reference (rule 5). CI runs every difftest group
`--tier interp` and `--tier llvm` (everything compiled, threshold ignored except over-size) and
requires identical canonical output.

## 6. Interfaces: kernel, event bus, VHPI, waves, DAP

| Consumer | Uses | From |
|---|---|---|
| CLI, DAP, VHPI | `SimControl` only (run_for, run_until, step_delta, pause, force, deposit, value, driving_value) | kernel |
| waves (`rsim-wave`) | `Observer::value_change(SignalId, Range)`; `HierarchyBuilder` filled from `Elaborated.hier` + `nets.leaves` (record fields as scopes, logic arrays as `SigType::Vector`, enum names from `Ty::Enum` literals, original spelling); wave `SignalId` = `LeafId` | elab, kernel bus |
| VHPI (M5) | object model built from `hier`, `nets`, `Program.types` (records, arrays, generics, generates native, rule 9); handle = generational id over `ObjId`/`ScopeId`; values decoded with `Layout`; `vhpi_put_value` = `SimControl::force/deposit` through `SubElemMap`; callbacks = `Observer` | elab, kernel |
| coverage (M9) | `Program.cov` items (path + span hash) and the counter array; toggle via `Observer` | ir, interp |
| DAP (M8) | `Debugger` hook, `FrameView`, `Program.stmts` for stack frames, `hier` for the hierarchy view | interp, elab |

Kernel `SignalId` order equals `LeafId` order, and `ProcId` equals elaboration order, so no
translation table is needed between elab ids and kernel ids.

**Kernel asks** (none blocks slices 1 to 3 below):

| # | Ask | Needed by |
|---|---|---|
| K1 | Port relations in the update phase: `Builder::port(formal, actual, mode, conv: Option<Converter>)`, effective/driving value of the formal from the actual in net order, converters run without a delta | uncollapsed ports, conversions (slice 4) |
| K2 | Implicit `'stable(T)`, `'quiet(T)` signals maintained by the kernel like `'transaction` (a process would add a delta) | `sigassign/stable_quiet` |
| K3 | `Ctx::driving_value(DriverId)` | `'driving_value`, reading `out` ports |
| K4 | `Ctx::driving(DriverId)` and null transactions (`disconnect`) | guarded signals, M3 |
| K5 | Null-range signals (`len == 0`) accepted as empty | null arrays as signals, M3 |
| K6 | Buffered writes to user files (stream ids beyond `OUTPUT`) | textio, M3 |
| K7 | A `SignalType` for bit-plane logic vectors, if Q1 picks planes | M3 |

## 7. M2 implementation order

Each slice ends with `cargo test`, `clippy -D warnings`, and the listed difftest groups green
against NVC at `--threads 1` and `--threads 4` (`tools/difftest/difftest.py --candidate rsim
--filter ...`). Groups not listed stay red and are not a slice failure.

| Slice | Content | Turns green |
|---|---|---|
| 1 | `rsim-ir` core: types for scalars and 1-D arrays with dope, frames, checked int/real/time ops, `Check`, control flow, `Report`, `Call`/`CallNative` for `'image`/`to_string`/implicit families on scalars; interpreter fast loop; elab of a single top entity with no ports; processes with `wait for`/`wait`; runtime stacktraces | `report/`, `arith/`, `scalar/` |
| 2 | Signals: `SigRead`, `Drive` (transport, inertial, reject), `WaitOn`, `wait until` lowering, sensitivity lists, concurrent statements as implicit processes, attributes `'event/'active/'last_*`, postponed processes, `std.env` natives | `delta/`, `sigassign/` (minus `stable_quiet` until K2), `process/`, `postponed/`, `env/` |
| 3 | Hierarchy: entity/component instances, generics (`-g` and generic maps through the interpreter), for/if/case generates, configurations, package generics, collapsed ports only, stable paths (`'path_name`) | `generics/`, `hierarchy/` (minus conversions) |
| 4 | Ports uncollapsed (K1), conversion functions, expression actuals, `force`/`release`, K2 implicit signals, collapse as a pass with both-way difftest | `force/`, rest of `hierarchy/` and `sigassign/` |
| 5 | Parallel: serial marking, `--threads N` invariance and `--shuffle` runs, ICE paths, diag snapshot tests for runtime errors (`tests/diag/`) | all of the above at N threads |

Groups for M3 (records, resolution, heap, protected, files): `composite/`, `resolution/`,
`access/`, `protected/`, `textio/`. Slice 1 already carries the descriptor model for them, so M3
adds instructions, not a second type path.

**M2 exit criterion:** every test in `report arith scalar delta sigassign process postponed env
generics hierarchy force` gives the same canonical output as its `.expected` (strict ignore set)
with `--threads 1` and `--threads 4`, with and without `--collapse-ports`; the IR verifier runs
on every lowered unit in debug builds; runtime-error diag snapshots exist for overflow, index,
range, division by zero and the delta limit.

## 8. Open questions (with recommendations)

| # | Question | Recommendation |
|---|---|---|
| Q1 | Logic arrays in memory and in kernel signals: bytes (kernel today) or bit-planes (decisions.md)? | Bytes in M2 (only scalars and small vectors in the M2 groups). In M3 measure a VUnit/OSVVM run with `Packing::Planes` for variables and signals behind the layout flag; switch only if it wins on real designs, since planes make every element-addressed signal op (kernel `Range` copies, resolution, VHPI, FST) a transpose. |
| Q2 | Port collapsing default | Off in M2 (reference semantics), the difftest runs both; turn on by default in M3 once both modes have been identical over the M2 and M3 groups. |
| Q3 | Protected types under `--threads N` | M2/M3: all protected callers serial (zero locking code). After M4, per-instance mutexes with a fixed acquisition order by instance id plus a deadlock check that reports both processes; only if VUnit benchmarks show serial processes are the bottleneck. |
| Q4 | Generic types: shared code with runtime descriptors, or specialise per actual? | Shared (one representation, rule 1); let the LLVM tier specialise hot instances later. |
| Q5 | VHPIDIRECT calls: dynamic C ABI calls need either `libffi` (new dependency) or per-signature trampolines | `libffi` crate (MIT/Apache) in M3; LLVM-generated trampolines are an M6 option. Windows needs it too, so check libffi's MSVC build early. |
| Q6 | Tier-up counter thresholds | 1000 resumptions / calls, settable by `--jit-threshold`; tune with M6 benchmarks. |
| Q7 | Kernel asks K1 to K7 | Assign to the kernel agent before slice 4; K1 and K2 are on the M2 critical path. |
