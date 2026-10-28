# M2 middle end: yavs-ir, yavs-interp, yavs-elab

Status (corrected 2026-10-28): all four crates are implemented and run every `yavs run`; the
interpreter is the semantic reference and two optional code generators (`yavs-llvm`, `yavs-clift`)
sit behind it. The sections below are the M2 design as written, kept as history; where the
implementation differs, section 9 and the dated notes in sections 4, 5 and 8 say how. The first
section after this header is the architecture as built.
Binding inputs: `docs/plan.md` (architecture table, "Designed for
the GUI", "Parallel kernel design", rules 1 to 12, the error-handling motto) and
`docs/decisions.md`. Interfaces read from: the kernel worktree (`yavs-kernel`: `Process`, `Ctx`,
`Builder`, `SimControl`, `Observer`, `SimError { trace: Vec<StmtId> }`), `yavs-logic`,
`yavs-wave`, the LLVM spike (`yavs-llvm`, `docs/STATUS.md`), `yavs-std` (`arith`), `lib/NATIVE.md`,
the VHPI stub. The HIR is specified in `docs/design/sem.md`; section 0 lists what this design
assumes of it.

```
HIR (yavs-sem) --lower, once per design unit--> Program (yavs-ir, immutable, Arc)
                                                    |
     elaborate(design, top, generics, cfg) ---------+--> Design { program, hier, nets, procs }
     (yavs-elab, runs elab code in yavs-interp)                       |
                                                    Design::build() --> yavs_kernel::SimControl
                                                    (InterpProcess per process instance)
```

## Architecture as built (2026-10-28)

Everything in this section was checked against the source at commit `ac2c94ff`. "Implemented"
means on `main` and run by `yavs run`; where something is design intent or future work it says so.

### The pipeline

```
files --yavs-syntax--> AST --yavs-sem (with yavs-lib manifests)--> typed HIR in a Db
   Db --yavs-elab: find_top, elaborate--> Elaborated { hierarchy, nets, process instances }
   Elaborated::lower_with --yavs-lower--> yavs_ir::Program   (immutable, shared)
   Elaborated::build_full_once --> yavs_kernel::SimControl + yavs_interp::Runtime (InterpProcess per process)
   use_tier (crates/yavs/src/elab.rs) --> optional compiled code installed on the Runtime
   SimControl::run_until --> kernel cycle, processes run in the interpreter or in compiled code
```

| Stage | Crate | Input to output | Notes |
|---|---|---|---|
| Parse | `yavs-syntax` | text to owned AST (1993, 2008, 2019) | single-threaded; no name resolution |
| Analyse | `yavs-sem`, `yavs-lib` | AST to typed HIR (`Db`) | libraries and unit closure from manifests; diagnostics `E1xxx` |
| Elaborate | `yavs-elab` | HIR to hierarchy, net graph, process instances | generics, constants and generate ranges are evaluated by running IR in `yavs-interp`, so elaboration is always interpreted |
| Lower | `yavs-lower`, called from `Elaborated::lower_with` | typed HIR to `Program` | one function per process or subprogram per design unit, shared by all instances; subprograms are lowered on first reference |
| Optimise | `yavs-ir` `opt` | per function, at first use | `Program::funcs` keeps the unoptimised code; the optimised copy is made when a function is first asked for |
| Build | `yavs-elab` `sim.rs` | `Elaborated` to a kernel model | package initialisation and elaboration-time calls are interpreted here; the kernel knows processes only as `Process` trait objects |
| Tier | `yavs` `use_tier` | installs a compiled tier on the runtime | runs after the build, so elaboration and package initialisation never use compiled code |
| Run | `yavs-kernel` | LRM 14.7 cycle | process evaluation may use a thread pool (`--threads`, see `docs/parallel.md`). A few simple concurrent statements (a plain copy, a clock toggle, small gates and multiplexers and expression trees of them) become kernel-native processes (`CopyProcess`, `ToggleProcess`, `GateProcess`, `MuxProcess`, `ProgProcess` in `process.rs`) that run neither in the interpreter nor in compiled code, so no tier applies to them |

Consumers of the finished model: `yavs-wave` (FST), `yavs-cov`, `yavs-vhpi`, `yavs-dap` subscribe to the
kernel's event bus or read the elaborated hierarchy; `yavs-cache` stores cache entries and is VHDL-agnostic.

### Crate boundaries that are easy to get wrong

* `yavs-kernel` does not know the IR, VHDL types or the heap. `yavs-ir` depends on `yavs-kernel`
  only for `StmtId`.
* `yavs-lower` depends on `yavs-sem`, `yavs-ir` and `yavs-kernel`; it does not depend on `yavs-elab`
  or `yavs-interp` (those are dev-dependencies for its tests). `yavs-elab` depends on `yavs-lower`
  and `yavs-interp`.
* `yavs-clift` depends on `yavs-llvm` (built without LLVM) for the shared `rt` module, the policy of
  `--tier=auto` (`AutoTier`) and the `Generator` trait; the Cranelift generator is `yavs_clift::First`.
* `yavs` (the binary) is the only place that decides which tier a run uses.

### Execution tiers (implemented)

| Tier | Where | What it does |
|---|---|---|
| Interpreter | `yavs-interp` | Executes IR (a predecoded form of each function in the fast loop, the plain IR when a debugger or a compiled tier is attached). The semantic reference: every other tier must give identical output (root rule 3). |
| LLVM | `yavs-llvm`, feature `llvm`, LLVM 21 | `--tier=llvm` compiles every non-elaboration function before time 0; `--tier=auto` compiles functions the interpreter is seen to spend time in, one at a time on one background thread, and swaps them in at the next call, return or within 256 jumps. Compiled code runs on the interpreter frame's registers and returns to the interpreter for everything it does not compile. |
| Cranelift | `yavs-clift`, feature `clift` | The same compiled-code contract with a different code generator: `--tier=clift` (everything before time 0) and `--tier=clift-auto` (the `auto` policy with Cranelift as the only generator). |
| Both | features `llvm` and `clift` | `--tier=auto3`: Cranelift for warm functions, LLVM O2 for those that stay hot. Built in CI; not what a release archive contains. |

How the CLI picks a tier, and what `--tier=auto` means in each build, is in `crates/yavs/AGENTS.md`
and `docs/running-faster.rst` and `docs/install.rst`. Summary: `yavs` has no default cargo features, so
a plain `cargo build -p yavs` is interpreter-only; with `llvm` or `clift` the default is `auto`
(`auto` with LLVM when `llvm` is built in, else `clift-auto`), except under `--dap`, which never
installs a compiled tier.

### Compilation cache (implemented)

All entries live under `<work library>/yavs-cache/` (store: `yavs-cache`; policy: `crates/yavs/src/cache.rs`
and `yavs-elab` `libstage.rs`): the analysed design (`sem-*.img`), analysed units (`units/`), the lowered
design and package state (`ir-*`, `low-*`, `init-*`, and the library-prefix kinds `lib-*`, `libinit-*`,
`libopt-*`), compiled code (`obj/obj-*.img` for LLVM, `obj/clif-*.img` for Cranelift) and the shared
analysed `std`/`ieee` units in a per-user store. Every entry is keyed by everything it depends on, damaged or
foreign entries are misses, and `--no-cache` neither reads nor writes. The behaviour is specified in
`docs/cli-contract.md` and described for users in `docs/running-faster.rst`.

### Parallel kernel (implemented, and what is not)

Implemented: a rayon pool of `--threads` workers evaluates the processes of a delta cycle when the batch
has at least 32 processes (`PAR_BATCH`), serial processes run afterwards on the kernel thread, the
update phase is parallel from 256 independent signals, and the commit order is fixed so output does
not depend on the thread count. Not implemented: a work-based inline or pooled choice, in-worker commit,
a shared thread budget (`docs/design/parallel-kernel.md`, `docs/parallel.md`).

### Design intent and future work (not implemented)

* Replacing LLVM by Cranelift as the only bundled generator (the README states the intention; both exist today).
* `--threads auto`, a token pool across test runs, element-range parallel update (parallel-kernel.md sections 6.4, 7).
* A tier-up protocol based on resumption counters (section 5 below): superseded by the sampled-time policy.

## 0. Assumptions about the HIR (to reconcile with sem.md)

| # | Assumption | Why the IR needs it |
|---|---|---|
| H1 | Arena ids (`UnitId`, `DeclId`, `ExprId`, `SeqStmtId`), each node with a `yavs_diag::Span` | statement ids, spans on every instruction |
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

## 1. The IR (`yavs-ir`)

### 1.1 Program, units, ids

All ids are `u32` newtypes (rule 5). A `Program` is built once from the HIR and is immutable and
`Send + Sync` afterwards.

```rust
pub struct Program {
    pub funcs: IdVec<FuncId, Func>,        // processes, subprograms, elab bodies, thunks
    pub types: IdVec<TyId, Ty>,            // runtime-shape descriptions (1.3)
    pub consts: ConstPool,                 // compact constant data (rule 12)
    pub stmts: IdVec<StmtId, StmtInfo>,    // global: StmtId is yavs_kernel::StmtId
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

- Registers are untyped 64-bit cells; `reg_ty` exists for the verifier and the JIT tier.
  `RegTy::Ptr` is an `Addr` (1.4), `RegTy::Sig` a signal reference `SigVal` (packed
  `{base: u32 index into the instance's signal table, offset: u32, len: u32}` in two cells),
  or a reference of several parts (1.8).
- Lowering allocates registers with a per-statement free list: temporaries die at the end of
  their statement, so allocation is linear and registers are reused across statements. Objects
  (variables, loop parameters, constants with non-trivial values) live in **frame slots**, never
  in registers.
- **Suspension invariant:** no register is live across a `Wait` or a call to a `MAY_SUSPEND`
  procedure. Arguments are written into the callee's slots before the call, out/inout write-back
  addresses are kept in caller slots. The interpreter keeps registers anyway (frames are heap
  objects), but the JIT tier relies on this: resuming needs only the frame memory and a resume
  index.
- **Definition invariant:** every register is written before it is read on every path from the
  entry (in a process: from the last `Suspend`), except the registers defined once by a
  constant or an address, which a frame starts with (`yavs-interp` `pre.rs`). The verifier
  checks it by dataflow in debug builds and tests (`verify::undefined_uses`, #430), so a frame
  may start with stale registers; the interpreter zeroes them anyway (appending the table to
  the register stack costs the same), and its collection reads only registers live at a
  safepoint (#422).
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
    Int  { lo: i64, hi: i64 },            // integer & physical: i64 in registers; 4 bytes as an array element within 32 bits (rule 7)
    Enum { lits: u32, logic: Option<LogicKind> },   // u8 if <= 256 literals else u32
    Real,                                 // f64
    Array   { elem: TyId, dims: u8, elem_resolved: bool },
    Record  { fields: Vec<(Ident, TyId)> },
    Access  { target: TyId },             // Handle (u64)
    File    { elem: TyId },               // FileId (u32)
    Protected { unit: UnitIr },           // Handle to instance state
}
pub enum LogicKind { StdULogic, Bit, Boolean }       // gets the yavs-logic fast paths
```

| Value | In memory | Notes |
|---|---|---|
| integer, physical | `i64` | ranges checked by `Check` (1.5); 2008 INTEGER is 32-bit by its range only. An array element of an integer type whose range fits in 32 bits is an `i32` (`Ty::elem_size`, `Width::B4` loads sign-extend), and so is a kernel signal element of it |
| enum | `u8` / `u32` | a scalar std_ulogic is the bare yavs-logic code (`StdULogic as u8`, decisions.md) |
| logic array | bit-planes (below) | arrays whose scalar leaf is a `LogicKind` enum |
| real | `f64` | |
| other array | elements contiguous, row major | bounds in a **dope**: `[left, right, dir]` per dimension, `i64` each |
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

**Logic arrays are bit-planes everywhere** (decisions.md): variables, constants, heap objects,
signals (the kernel stores logic leaves as yavs-logic planes, K7) and temporaries.

| Aspect | Rule |
|---|---|
| Format | yavs-logic `Planes`: block `i` = elements `64i..64i+63` as `[u64; 4]` (plane 0 = code bit 0); element 0 is the rightmost element. `bit` and `boolean` arrays use the same blocks (only planes 0 and 1 vary) so one code path serves all `LogicKind`s |
| Nesting | an array of logic arrays (memories, `array (0 to 1023) of std_logic_vector(7 downto 0)`, 2-D logic arrays) is **one** plane vector over all elements, rows contiguous; a row is an element range, not a padded block |
| In records | a logic-array field is its own plane vector at its field offset, padded to whole 32-byte blocks |
| Size | `ceil(n / 64) * 32` bytes (`LayoutOf` computes it; `u64`) |
| Element access | `LGet{dst, base, idx}` / `LSet{base, idx, v}`: block `idx / 64`, bit `idx % 64` in each plane; `Index` on a logic array lowers to these; `LFill{base, idx, n, v}` is `LSet` of one code at the `n` plane elements from `idx` (a default, an `others` choice: one instruction, whole plane words at a time, instead of a loop) |
| Slices, concatenation, aggregates | `LCopy{dst, dst_off, src, src_off, n}`: word-wise copy with funnel shifts (64 elements per step, any alignment); `Copy` when both ends are block aligned; `Cat{dst, n, m, parts, scalars, bias, logic, w}`: the pieces of a concatenation (or of a positional aggregate of arrays and elements) stored in order into the result block, as the `LCopy`/`LSet` (planes) or `Copy`/`Store` (bytes) of each piece would (docs/decisions.md "Concatenation instruction") |
| Operators | `LogicOp`/`LNum` call yavs-logic bit-parallel ops (`and`..`xnor`, `not`, `to_x01`, `resolve`, `match_eq`, shifts, `add_num`, `cmp_num`); 0/1-only values take the `BitVec` word path |
| Parameters | in-mode: by reference when block aligned, else `LCopy` into a temp; out/inout of an unaligned slice: copy-in/copy-out (LRM 6.5.6.2 allows either) |
| Constants | `ConstData::Runs` expand per block (`others => '0'` is a block fill) |
| Transpose | only at the edges, per changed element range, 64 elements per step: waves (`value_change` range to one char per element for `WaveWriter::change`), VHPI `vhpi_get_value`/`put_value` (`vhpiLogicVecVal`, `vhpiEnumVecVal`), DAP display, foreign calls (1.7), `'image`/`to_string` |


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
- `FrameLayout.roots: Vec<(offset, RootKind)>` lists every slot that holds a `Handle` or an
  address into the heap, with `RootKind::{Handle, Composite(TyId), Addr}` (`Addr`: a parameter
  passed by reference, the data or descriptor of an array parameter, an alias). Registers have
  types (`RegTy::{I64, F64, Ptr, Sig, Handle}`: access and protected values are `Handle`), and a
  temporary that holds handles or heap addresses is recorded by `TempRoot` until its statement
  ends. Together these give precise roots (rule 6, #422): at a suspension the frames' root maps
  and the recorded temporaries (no register is live there, by the invariant); between
  instructions also the `Handle` and `Ptr` registers live where each frame is (1.6).

### 1.5 Instructions

Operands are `Reg` or small immediates; `Addr` operands come from registers. Every instruction
has a `StmtId` in `Func.loc`.

| Group | Instructions |
|---|---|
| move/const | `Const{dst, i64}`, `ConstF{dst, f64}`, `Move{dst, src}`, `Select{dst, c, a, b}` (`a` if `c` is not zero, else `b`), `AddrOf{dst, space, slot}`, `CopyConst{dst_addr, const}` |
| memory | `Load{dst, addr, w}`, `Store{addr, src, w}`, `Copy{dst, src, bytes}`, `Fill{dst, elem, n, size}`, `Index{dst, base, dope, idx...}` (bounds-checked), `Field{dst, base, layout, n}` |
| int (checked, rule 7) | `Add`/`Sub`/`Mul`/`Div`/`Rem`/`Mod`/`Pow`/`Neg`/`Abs` on i64: overflow and zero divisors raise `Runtime` through `yavs_std::arith`; never wrap |
| real | `FAdd`..`FPow`, `FCmp`, `IToF`, `FToI` (checked round per LRM, `real_to_int`) |
| compare/logic | `Cmp{op, dst, a, b}`, `And/Or/Xor/Not` on booleans, `LogicOp{op, dst, a, b, n}`, `LNum{op, dst, a, b, signed}` on logic arrays via yavs-logic (bit-planes, 1.3); `LGet`/`LSet`/`LCopy` element and slice access, `Cat` concatenation |
| checks | `Check{kind: Range\|Index\|Length\|Null\|Discrete, v, lo, hi, what: CheckMsgId}`; `lo/hi` are regs, so runtime bounds and static bounds use one instruction |
| subtypes | `MakeDope{dst, dims...}`, `DopeLen{dst, dope, dim}`, `LayoutOf{dst, desc}`, `SubtypeCheck{value, desc}` (composite conversion and constraint checks) |
| control | `Br{blk}`, `CondBr{c, t, f}`, `Case{v, table: CaseTableId, default}` (ranges and choices as sorted table), `Ret`, `Unreachable` |
| calls | `Call{func, args}` (VHDL), `CallNative{id, args}` (NATIVE.md symbols and implicit families), `CallForeign{id, args}` (VHPIDIRECT, 1.7), `CallIndirect{func_reg, args}` (subprogram generics), `CallProt{func, obj, args}` (protected method: takes the instance lock, 3.5) |
| heap | `New{dst, ty, init}`, `NewFill{dst, ty, desc, dsize, data, size, fill}` (descriptor bytes then a copy of `data`, or with `fill` > 0 that element repeated: no temporary to build and copy, #427), `Free{handle}` (`deallocate`), `Deref{dst, handle}` (null check included); the handle operands are `Handle` registers. `TempRoot{addr, desc, ty}`: the temporary at `addr` holds a value of `ty` (constrained by `desc`) whose handles are roots, or with `ty` `None` a fat pair whose addresses keep their heap objects, until the statement ends (#422) |
| signal read | `SigVal{dst, inst_sig, offset, len}` builds a signal reference; `SigParts{dst, parts, n}` one of several parts and `SigSlice{dst, sig, offset, len}` a slice of either form (1.8); `SigRead{dst_addr, sig}` (effective value), `SigReadScalar{dst, sig}`, `SigDriving{dst_addr, driver_slot}` (`'driving_value`, out-port read) |
| attributes | `Edge{dst, kind: Rising\|Falling\|To1\|To0, sig}` (the one clock-edge test, #632: `rising_edge`, `x'event and x = '1'`, the level of a `wait until`, ...; `sig` one element of a `std_ulogic`, BIT or BOOLEAN signal), `SigAttr{dst, kind: Event\|Active\|LastEvent\|LastActive\|LastValue\|Driving, sig}`; `'stable/'quiet/'delayed/'transaction` are implicit signals made by elab and read with `SigRead` |
| signal write | `Drive{driver_slot, target: Option<sig>, waveform: Addr, n, mode: Transport\|Inertial{reject: Option<Reg>}}` (waveform = `n` `(time, value)` pairs in a temp, ascending); `Force{sig, mode: In\|Out, value}`, `Release{sig, mode}`; `Disconnect{driver_slot}` (null transaction, guarded, M3) |
| record signal | `SigGather{dst, desc, tab, part}` (every leaf of a record signal, or of an array of records, into the value), `SigScatter{driver, src, desc, tab, ops, mode, has}` (every leaf driven from the value with one waveform element, leaf `k` through slot `driver + k`; `ops` = delay, reject limit, part); `tab` is a leaf table constant (`yavs_ir::gather`), `part` the first element and element count of the outermost array when a name selects a part of the signal; same meaning as the per-leaf `SigRead`/`Drive`, independent of how the kernel stores the leaves (decisions.md "Whole record signals: gather and scatter") |
| wait | `WaitOn{sig}` (adds to sensitivity), `WaitEdge{sig, kind}` (before the `WaitOn`s of a `wait until` with an edge as an `and` conjunct: the wait resumes the process only in a cycle with that edge), `WaitFor{t}`, `WaitAt{abs}`, `Suspend{resume}`; `TimedOut{dst}`, `Now{dst}`, `Delta{dst}` |
| effects | `Report{msg: Addr, len, severity}` (assert = branch + `Report`); `Output{stream, addr, len}` is only used by natives |
| coverage | `Cover{item: CovItem}` (statement/branch counter increment, rule 10), toggle coverage is an observer, not IR |
| debug | `Stmt{id}`: statement boundary; a nop in the fast dispatch loop, the breakpoint/step hook in the debug loop |
| elab only | `ElabSignal`, `ElabDriver`, `ElabProcess`, `ElabInstance`, `ElabPort`, `ElabExternal` (3.6, 4); the verifier rejects them outside `FuncKind::Elab` |

Lowering of the main statement forms:

| VHDL | IR |
|---|---|
| `wait on a, b until c for t;` | `deadline := Now + t` into a slot; `L: WaitOn a; WaitOn b; WaitAt deadline; Suspend r; TimedOut -> exit; c -> exit; Br L` (kernel contract: the process re-suspends with the same absolute deadline) |
| `process (s) ... end process` | body, then `WaitOn s; Suspend` at the end (LRM 11.3); flag `SENS_LIST`: a `Suspend` executed in a procedure below the process is a run-time error (LRM 11.3) |
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
  collection exceeds a threshold; roots are that process's frames (root maps) and recorded
  temporaries. The shared heap is collected at the start of a cycle, when no process runs;
  serial processes allocate there, and its roots are the shared variables area, the objects
  elaboration made and the roots each serial process published at its last suspension (#303).
  A process that runs long without a wait also collects inside its activation, before a `new`,
  a `deallocate` or a native call (user, 2026-10-19, #409, #422): its roots are then its
  frames' root maps, its recorded temporaries and the `Handle` registers live at the
  safepoint each frame is at (the top frame's instruction, the call every caller waits in),
  from a liveness analysis of the function computed when a collection first meets it; live
  `Ptr` registers and `Addr` slots keep the heap objects they point into. Nothing is scanned
  conservatively (rule 6). Deallocated slots wait in quarantine until a suspension, or, in a
  long activation, until a recycle between instructions, finds no address to them in the same
  precise set (#81, #355). No GC ever waits for another thread (the deadlock class seen in
  other simulators cannot happen): a process's private heap is collected only by its process,
  and the shared heap only by the one serial process that holds the shared state, while the
  other serial processes are suspended.
- **Protected:** an instance is a heap object holding its private variables; methods are `Func`s
  with `self` as the first parameter. `CallProt` takes the instance's lock (3.5).
- **Files:** `FileId` indexes the owner's file table (process or shared). Reads are immediate;
  writes to `OUTPUT` go through `Ctx::output` (buffered, committed in elaboration order); user
  files are read and written directly, and every caller of a file operation is serial (3.5;
  kernel ask K6 was dropped, #82).

### 1.7 Foreign subprograms (VHPIDIRECT), no libffi

Few dependencies: yavs calls C through **its own trampolines**, one per ABI, in a small crate
`yavs-ffi` (the only `unsafe` besides `yavs-vhpi`; `forbid(unsafe_code)` elsewhere).

```rust
pub enum CType { I8, I16, I32, I64, U8, U16, U32, F32, F64, Ptr }   // the whole C type set
pub struct Signature { pub params: Vec<CType>, pub ret: Option<CType> }
pub enum Arg { I8(i8), .., F64(f64), Ptr(*mut c_void) }              // Ret is the same plus Void
impl ForeignFn {
    pub unsafe fn new(ptr: *const c_void, sig: Signature) -> Result<Self, FfiError>; // null, > 64 args
    pub fn call(&self, args: &[Arg]) -> Result<Ret, FfiError>;   // safe: callers forbid unsafe
}
```

- **Classification is Rust, the stub is asm.** `call` assigns arguments to registers and stack
  slots per ABI into a `CallFrame`; a per-target stub (`#[unsafe(naked)]` + `naked_asm!`, stable
  since Rust 1.88; `core::arch::global_asm!` if a target needs it) loads the registers, copies the
  stack slots below a 16-byte aligned `sp`, calls `func`, and stores both result registers.

| ABI (target) | Integer/pointer regs | Float regs | Stack and rules |
|---|---|---|---|
| SysV x86_64 (Linux x86_64) | rdi rsi rdx rcx r8 r9 | xmm0-xmm7, independent counters | rest in order, 8-byte slots; `al` = float regs used; `rsp` 16-aligned at the call |
| AAPCS64 (Linux aarch64) | x0-x7 | d0-d7, independent counters | rest in order, 8-byte slots; `sp` 16-aligned |
| Win64 (Windows x86_64) | rcx rdx r8 r9 **by position** | xmm0-xmm3 by the same position | 32-byte shadow space, then args 5+ in 8-byte slots; `rsp` 16-aligned |

- **VHDL to C mapping** (checked at elaboration; anything else is an error naming the subprogram
  and parameter, fail early). It follows the primary reference simulator's user manual ("VHPIDIRECT" section), which
  defines integers, reals, enumerations and both array kinds; records and access values are
  yavs additions. `yavs_ffi::vhdl::lower` implements it:

| VHDL | C | Passing |
|---|---|---|
| integer type or subtype | smallest of `int8_t`..`int64_t` holding the range (`integer` is `int32_t`) | value, sign-extended to 64 bits |
| enumeration | smallest of `uint8_t`/`uint16_t`/`uint32_t` holding the literals (`boolean`, `character`, `bit`, `std_ulogic` are `uint8_t`) | value, zero-extended |
| physical (`time`) | `int64_t` | value (in resolution units, as stored) |
| `real` | `double` | value |
| access | `void *` | value |
| constrained array | element pointer | marshalled into a C-layout temp (`int32_t`/`uint8_t`/`double` elements, logic arrays transposed from planes to one code byte per element), copied back for `out`/`inout` |
| unconstrained array | element pointer, then one `int64_t` length argument per dimension | same marshalling; the reference simulator's convention, so C written for the reference simulator works unchanged (bounds and direction are not passed) |
| record of the scalar kinds above | pointer to a C struct with natural alignment | marshalled both ways |
| scalar `out`/`inout` | pointer to a C-typed temp | written back after the call |
| function result | `void` or any scalar C type above | `yavs-ffi` narrows the result register to the declared type (sign/zero extension); the interpreter range-checks |

- Not supported (elaboration error): composite results, struct-by-value, varargs, callbacks
  from C into VHDL (VHPI covers that).
- Symbols: `attribute foreign of f : function is "VHPIDIRECT [lib] symbol"`; looked up in
  `--load` libraries (reusing `yavs-vhpi`'s plugin loader), then in the executable. A missing
  symbol is warning `W0108` before time 0 and runtime error `E0107` at the call, both listing
  every place searched (an unbound entry is a `None` binding in the interpreter); a library word
  in the attribute is ignored, as in the reference simulator.
- **Tests on all three ABIs** without a C compiler: Rust `extern "C" fn` callees with mixed
  signatures (interleaved int/float, more arguments than registers to force stack slots,
  `int32_t`/`uint8_t` results with garbage upper bits, pointer write-back), run in CI on Linux
  x86_64, Linux aarch64 and Windows x86_64. The JIT tier calls foreign functions directly with
  the same mapping.

### 1.8 Signal references of several parts (#71)

A signal parameter is a reference, and an individually associated signal formal
(`drive2(x(1) => s1, x(0) => s0)`, a record formal whose fields come from different signals)
covers ranges of several kernel signals. Such a reference keeps the two-register shape of
`RegTy::Sig`, so parameter passing, frames and the calling convention do not change:

- **Form.** First register `0xffff_ffff << 32 | count` (no instance has that many signals),
  second register the address of `count` entries `[ref lo, len, driver slot]` (24 bytes each) in
  temporary memory, in order of the referenced object's kernel elements (rule 2: the same
  element order as the one sub-element map).
- **Building.** `SigParts{dst, parts, n}` reads `n` entries of `SIG_PART_BYTES` (32): a reference
  (16 bytes), the caller's driver slot for that part (`-1` for `in`), and the part's first
  element in the whole object. Lowering computes the first element by binding the formal to a
  reference from element 0 of signal 0 while it lowers the formal part (`x(1)`, `x.a.v(0)`),
  so the walk over signal names is the one walk (rule 2). The interpreter flattens parts that
  are themselves multi-part references, orders the parts by first element and checks that they
  tile the object (an ICE otherwise: sem checks coverage, LRM 6.5.7.1).
- **Slicing.** `SigSlice{dst, sig, offset, len}` replaces the arithmetic on the packed cell: on
  a single range it moves the offset, on several parts it clips them (a reference of several
  parts stays one, so every piece keeps its driver slot). Names below a signal parameter (index,
  slice, record leaf) go through it.
- **Use.** Every signal instruction maps the parts in order onto the kernel `Ctx` calls of one
  process step: `SigRead`/`SigLastValue` concatenate (all parts of one element kind),
  `SigAttr` combines (`'event`/`'active`: any part; `'last_event`/`'last_active`: the most
  recent, LRM 16.2.4), `WaitOn`/`Release` take every part, `Force` splits the value, and
  `Drive`/`DriveSlot` split each waveform element and drive each part through the slot it
  carries (the instruction's own slot cell is `-1` for such a formal). So reads, drives and
  attributes stay in the same delta, need no kernel change, and do not depend on the thread
  count. `Disconnect` of such a reference is an ICE (no guarded signal parameters yet).
- A record formal (one reference per leaf, #215) gets one such reference per leaf; its first
  driver slot cell is `-1` as well, since each leaf's parts carry their own slots.

## 2. Processes, subprograms, suspension, shared code

- **One `Func` per process or subprogram per design unit**, shared by every instance. Generics
  (constant), signals, drivers and declared constants are per-instance data read through
  `Space::Inst`, never baked in. This keeps code size linear in the source, not in the number of
  instances.
- Instance data `InstData { generics, consts, sigs: Box<[SigBase]>, child: Box<[InstRef]>, pkg: ... }`
  is computed by the unit's `Elab` function and frozen (read-only) before simulation.
- **Type generics and generic subprograms** are runtime data too: a type generic is a descriptor
  plus a `TyId` class, a subprogram generic is a `FuncId` in the instance data called through
  `CallIndirect`. One body per unit; specialisation per actual is an optional later pass (Q4).
- **Generic package instances** each get their own `Mangled` identity and their own cloned HIR (rule 4, sem.md 5.1). A design unit instance has its own package data; an instance declared in another region adds its objects to that region's storage (instance data, frame or package data, `docs/decisions.md` "Objects of generic package instances", #101).
- **Suspension:** a process is a frame stack `Vec<Frame>` owned by its `InterpProcess`. `Suspend`
  returns from `Process::run` with the stack intact; the next `run` continues at the top frame's
  `pc`. A `wait` inside a procedure therefore needs nothing special: the procedure frame is just
  on the stack (frames are on the heap). Each `Suspend` names a resume index (`Func.resume`),
  which is the state number for the JIT tier's resumable state machine.
- Functions cannot suspend (H11); a `MAY_SUSPEND` procedure called from a function is an ICE in
  the verifier.

## 3. Interpreter (`yavs-interp`)

### 3.1 Dispatch

- `Inst` is a Rust `enum` with `#[repr(u8)]`-sized tag and inline operands (8 to 16 bytes); a
  `match` loop over `code[pc]` with a local `pc`, `regs` and `slots` (the Rust compiler compiles it to a jump
  table). No threaded code, no `unsafe` (foreign calls are the one exception, isolated in `yavs-ffi`, 1.7).
- Two loop instances: `exec::<false>` (fast: `Stmt` is a nop, no hook checks) and
  `exec::<true>` (debug: `Stmt` calls the debugger hook). The process switches loops when a
  debugger is attached. The fast loop without a compiled tier runs a predecoded form of each
  function instead of `code` (section 9, #431).
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

impl yavs_kernel::Process for InterpProcess {
    fn run(&mut self, ctx: &mut Ctx<'_>) -> Result<(), SimError> {
        // continue frames.last(); signal ops -> ctx.value/assign/wait_on/wait_at/report/force
        // on error: ctx.error(kind) with trace = stmt of each frame, innermost first
    }
}
```

| Instruction | Kernel call |
|---|---|
| `SigRead`, `SigReadScalar` | `ctx.value(SigRef)` then copy; logic leaves arrive as planes and are copied with `LCopy` (no transpose); a reference of several parts reads each part in order (1.8) |
| `SigAttr` | `ctx.event/active/last_event/last_active/last_value` |
| `Edge` | `ctx.edge_by(sig, kind code)` (the kernel answers true without looking when it resumed the process for that very edge) |
| `Drive` | `ctx.assign(drivers[slot], target, waveform, DelayMode)` |
| `Force`/`Release` | `ctx.force/release(r, ForceMode::Effective\|Driving)` |
| `WaitOn`/`WaitFor`/`WaitAt`/`TimedOut` | `ctx.wait_on/wait_for/wait_at/timed_out` |
| `WaitEdge` | `ctx.wait_edge(sig, kind code)` |
| `Report` | `ctx.report(sev, msg)` (message formatting is yavs's, the kernel only buffers) |
| `Now`/`Delta` | `ctx.now()/delta()` |
| natives `env.stop/finish` | `ctx.stop/finish` |

### 3.3 Diagnostics and stack traces

- A runtime error builds `SimError` through `ctx.error(ErrorKind::Runtime(msg))` and fills
  `trace` with the current `StmtId` of every frame, innermost first. yavs-diag renders each frame
  from `Program.stmts` (subprogram name, file:line, snippet) and adds the process instance path,
  time and delta from `SimError`. An assert inside `numeric_std` therefore shows the user's call
  site as the second frame (plan, yavs-diag row).
- Check failures carry a `CheckMsgId` with the static part of the message ("index 8 out of range
  7 downto 0 for s") so the error names the object, the value and the bounds.
- A broken interpreter invariant returns `ErrorKind::Internal` at the current statement's span:
  "this is a bug in yavs" (exit 101), never a bare panic.

### 3.4 Debug hooks (the reference simulators style)

Implemented in `yavs-interp` (`debug.rs`); the DAP server (M8) and the GUI (M10) build on it
without interpreter changes.

```rust
pub trait Debugger: Send + Sync {
    /// Called at a stop. Blocks while the user inspects.
    fn on_stop(&self, stop: &Stop<'_>, session: &DebugSession) -> Resume;
}
pub enum Resume { Continue, StepIn, StepOver, StepOut, GlobalStep, Skip }  // Skip: not a stop after all
pub struct Stop<'a> {
    pub reason: StopReason,      // Breakpoint { hits } | Step | GlobalStep | Pause | ConditionError(msg)
    pub stmt: StmtId,
    pub process: usize,          // index in the session's process table
    pub now: Option<Time>, pub delta: Option<u32>,   // time and delta at every stop
    pub frames: FrameView<'a>,   // call stack: func names, current StmtId, locals (name, TyId, bytes)
    pub ctx: Option<&'a Ctx<'a>>,// current signal values
}
impl DebugSession {   // one per simulation, shared by all attached processes
    fn new(prog, Box<dyn Debugger>) -> Arc<Self>;
    fn set_breakpoint(&self, StmtId, LineBp) -> Option<Arc<LineBp>>;  // LineBp { cond: Option<FuncId>, hit: HitCond }
    fn clear_breakpoint(&self, StmtId);
    fn request_global_step(&self); fn request_pause(&self); fn cancel_pause(&self);
    fn processes(&self) -> Vec<ProcStatus>;  // name, NotStarted|Running|Waiting|Terminated, wait stmt, sensitivity, absolute timeout
}
InterpProcess::with_debug(self, Arc<DebugSession>, name) -> InterpProcess
Elaborated::build_debug(cfg, &Arc<DebugSession>)   // every process attached (yavs-elab)
SignalBreak::new(sim.interrupt()).watch(signal, Option<Value>)   // a kernel Observer
SignalBreak::watches(&self) -> Watches   // the shared watch list, changed while running (DAP)
```

| Feature | How |
|---|---|
| line breakpoint | bitset over `StmtId` (checked at each `Stmt` in the debug loop only), plus a `LineBp` per set bit |
| condition | an IR `Function` without parameters whose `parent` is the function containing the statement; it reads the stopped frame through `AddrSpace::Up(1)`. Lowering produces it from the HIR expression typed in the debugger. An error in it is a stop with `ConditionError`, not a simulation error |
| hit count | `HitCond::{Always, Eq(n), Ge(n), Multiple(n)}` over the hits where the condition held; `LineBp::hits()` |
| step in / over / out | per process, by frame depth (`StepMode`) |
| global step | a session flag: the next `Stmt` executed by any attached process stops (the third reference simulator `step`, a commercial simulator trace) |
| pause inside a process | session flag polled every 4096 instructions, stops at the next statement (endless loops) |
| signal breakpoint | `SignalBreak` observer on the event bus: on a change of the signal (or to a value) it records the hit and calls `Interrupt::pause`; `SimControl::run_*` returns `Paused` after that cycle |
| run for / next delta / next time step | `SimControl::run_for`, `run_until`, `step_delta`, `step_timestep` |
| stack and locals | `FrameView`: per frame the function, current statement and `Func::locals` (variables and typed parameters); `format_scalar` gives `'image` text; `FrameView::value`/`set_value` read and write a local whole (composites as data and descriptor bytes, `CallVal`) |
| process list | `DebugSession::processes()`: each process publishes its state and what it waits on (sensitivity, absolute timeout, the wait statement whose span shows the condition) at every suspension |

- Stops inside a process need `--threads 1` (the kernel is mid-delta); the CLI forces it when
  a session is attached. Processes without a session run the fast loop (`exec::<false>`), which
  has no hook checks; the debug loop is `exec::<true>`.
- The debug loop never runs JIT code (5): attaching a session pins processes to the
  interpreter.

### 3.5 Thread safety

| State | Owner | Access |
|---|---|---|
| `Program` (code, types, consts, stmt table) | `Arc`, built before the run | shared read-only |
| `InstData` of instances and packages (non-shared parts) | `Arc`, frozen after elab | shared read-only |
| frames, private heap, temps | the `InterpProcess` | `&mut` in `run` only |
| driver queues, effects, sensitivity | kernel `Ctx` | per process (kernel design) |
| shared variables, protected instances, shared heap, files (any file object: two can name the same external file) | `SharedState` / the file system | only from processes marked serial (below) |
| coverage counters | `Arc<[AtomicU64]>` indexed `inst.cov_base + item` | relaxed `fetch_add`; sums are order independent |

- **Serial marking:** at elaboration a process is `ProcFlags { serial: true }` when its body or any
  reachable subprogram has `TOUCHES_SHARED` (shared variable, protected call, a disk-file or
  directory native, or a package variable; textio `OUTPUT` users included, a ponytail).
  The interpreter reports it through `Process::serial`, which the kernel ORs into the flags. Computed once per `Func` by the linear propagation in 1.2, looked up per
  instance.
- **Process shape (#252):** `Program::single_suspend` is true for a process with one resume
  point that is a `Suspend` (a sensitivity list or one wait statement; a call of a
  `MAY_SUSPEND` procedure is a resume point but counts as several). The interpreter reports the
  others through `Process::several_waits`; the kernel orders a cycle's processes by it
  (yavs-kernel DESIGN.md "Parallel evaluation").
- **Protected locking (M2/M3):** every protected caller is serial, so the kernel already runs it
  alone and `CallProt` takes no lock. `// ponytail: protected calls serialised via serial
  marking; per-instance locks when benchmarks at the scale of large test-runner suites show serial processes dominate`. The
  per-instance lock design (Q3) only changes `CallProt` and the marking rule.
- `--threads 1` and `--threads N` produce identical output because all observable effects go
  through the kernel's buffered effects, and serial processes see a deterministic order.

### 3.6 Elaboration-time evaluation

The interpreter also runs `FuncKind::Elab` and `Thunk` functions without a kernel: an
`ElabCtx` implements the same signal-free subset (no `Wait`, no signal reads; using one is a
"signal read during elaboration" diagnostic). Generic actuals, generate ranges, port-map
expressions, constant declarations, subtype descriptors and signal initial values are all
computed this way, so elaboration and simulation share one evaluator (rule 4, rule 5).
Calls that use package state run against the elaborated packages, in elaboration order, and
are replayed when the simulation is built (yavs-elab `pkgcall.rs`, `docs/decisions.md`
"Elaboration", #77).

## 4. Elaboration (`yavs-elab`)

```rust
pub fn elaborate(
    design: &yavs_sem::Design,           // analyzed HIR, read-only
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
impl Elaborated { pub fn build(&self, kcfg: yavs_kernel::Config) -> Result<SimControl, SimError>; }
```

Pure and re-entrant: no globals, no interior mutability shared across calls; `build` can be
called again for an in-process restart (rule 4).

| Step | What happens | Errors (fail early, with instance path) |
|---|---|---|
| 1. Reachability | units reachable from `top` through instances, configurations, packages; lowered once each into `Program` | missing unit names where yavs looked (library manifest) |
| 2. Packages | elab functions of packages in dependency order; generic package instances get `InstData` + mangled identity | |
| 3. Generics | `-g` overrides parsed as VHDL literals of the generic's type (rule 4), then generic map thunks evaluated in the parent instance | missing generic without default; type mismatch; bad override text |
| 4. Declarations | the unit's `Elab` function: constants, subtype descriptors, signals (`ElabSignal`), shared variables, protected instances | bound/constraint failure at a declaration |
| 5. Generates | `for`: evaluate range, one child scope per value, path `label(i)`; `if`/`case`: evaluate and pick one alternative (2008 alternative labels) | non-static range use errors |
| 6. Binding | component instance: configuration declaration, then configuration specification, then default binding (LRM 7.3.3); entity instance directly | unbound component (warning per LRM, then open); port/generic mismatch between component and entity |
| 7. Ports | association per formal sub-element (`ElabPort`), see below | type/mode mismatch, unassociated in-port without default |
| 8. Processes | `ElabProcess` per process: drivers from the process's static driver list (per `Drive` slot, evaluated for this instance), flags (postponed, serial) | multiple drivers on an unresolved signal (LRM 14.7.2), names both processes |
| 9. External names | bound in elaboration order against the partial hierarchy | target not found or not yet elaborated, with the path tried |
| 10. Check | net graph verified (every port connected or defaulted, every driver has a target) | |

**Ports (rule 3).** Each formal is its own signal in the net graph. Association kinds:

| Association | Net graph | Kernel |
|---|---|---|
| actual signal, no conversion, collapsing on | formal aliases actual's `SignalId` | nothing |
| actual signal, no collapse / mode `inout`/`buffer` | `Port { formal, actual, mode }` edge | effective/driving value relation in the update phase (kernel ask K1) |
| conversion function or type conversion | `Port { .., conv_in/conv_out: FuncId }` | converter callback runs the IR function in the update phase (K1) |
| conversion of a composite with several leaves (record, array of composites) | `ConvPort { formal: [(leaf, range)], actual: [(leaf, range)], .. }` | `Builder::port_parts`: one converter call over all source parts per cycle, writing all target parts, in the same update phase (section 4.1) |
| expression actual (2008) | implicit process `formal <= expr` sensitive to the expression's signals, `IMPLICIT` | ordinary process + driver (LRM 6.5.6.3 defines it this way) |
| `open` / default | initial value only | |

Port collapsing is a separate pass over the net graph, off by default in M2 (`--collapse-ports`),
with every difftest group run both ways (rule 3, Q2).

*Correction 2026-10-28:* collapsing is on by default since 2026-10-19 (`--no-collapse-ports` keeps
every port apart; `--dap` and `--vhpi` also switch it off): `docs/decisions.md` "`in` ports collapse
into their actuals", `ElabConfig::collapse_ports`.

### 4.1 Multi-part ports (#74)

A converted association whose formal or actual is more than one kernel leaf (a record port, an
array of records: struct of arrays, rule 2) is one kernel port edge with a list of parts on each
side, in leaf order (`NetGraph::select`). The converter receives one value per source part and
returns one per target part; yavs-elab packs the parts into the IR value of the function's
parameter and unpacks its result the same way as a composite resolved as a whole (`whole::Map`:
leaves concatenated, a logic element one code byte, a byte element its bytes).

Same delta, deterministic: the update phase stays one pass by port level (formal parts deeper
than every actual part). Each edge names the part its worklist reaches last: for driving values
(deepest first, then lowest id) the formal part of lowest level and highest id, for effective
values (shallowest first) the actual part of highest level and highest id. Updating any other
part only queues that one; updating it runs the conversion once, with every part current, and
marks the targets active. So the conversion runs once per cycle in which any source part is
active, in the delta of the source, independent of thread count (the port update is serial).
An equivalent process would be a delta late, which is why this is kernel state and not IR.

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
  as a whole composite (`resolved_record_port`) keeps these leaves, joined for resolution by
  `Builder::resolved_composite`: its resolver gets each source as the leaves' values
  concatenated in leaf order (logic leaves one code byte per element), which the elaborator's
  wrapper rearranges into the IR's composite layout (#83; bit-planes everywhere). Storage and
  event tracking are therefore per contiguous vector, never per scalar.
- **Element order** (implemented in `yavs_elab::NetGraph::select`): a leaf's elements are row
  major, leftmost first; logic leaves mirror it (kernel offset `n - 1 - p`), since yavs-logic's
  element 0 is the rightmost.
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

## 5. JIT tier seam (M6)

What the spike (`yavs-llvm`, docs/STATUS.md, decisions.md) fixes:

| Spike result | Consequence for the IR |
|---|---|
| Functions are `extern "C" fn(...) -> i32` status; runtime errors call a Rust callback and return a status; no unwinding through JIT frames | every `Check`, checked op and native call has an error edge to a "return status" exit; the interpreter's `Result` maps 1:1 |
| Runtime functions are absolute symbols | `CallNative`, signal ops and checked helpers lower to calls of the same `yavs-rt` functions the interpreter calls; no semantics are re-implemented in LLVM IR beyond i64/f64 arithmetic |
| A `#[repr(C)]` state pointer per process | frame slots and instance data have one byte layout (`FrameLayout`, `Layout`) used by both tiers |
| Background compile thread, atomic function pointer swap | `Func.native: AtomicPtr` (in a side table, `Program` stays immutable) |
| O2 time grows superlinearly: 0.15 s at 10k, 0.8 s at 40k, 3.8 s at 100k instructions | **threshold: `Func.size <= 50_000` IR instructions is compiled at O2, larger stays interpreted** unless `--tier-max-size` raises it (user, 2026-10-19, #126); an O0 band above it is an M6 option |

**Tier-up protocol.**

1. Each `Func` has an entry counter in a side table. When a process `Func` reaches N resumptions
   (default 1000) or a subprogram N calls, and `size <= 50k`, it is queued for compilation.
2. The compiled entry is `fn(frame: *mut u8, inst: *const u8, rt: *mut RtCtx, resume: u32) -> u32`;
   it switches on `resume` to the state after the matching `Suspend` (the suspension invariant
   makes the frame bytes the whole state). Return: `Done | Suspended(resume) | Error | CallInterp(func)`.
3. The swap happens only at a function entry or a process resumption, never mid-frame. Mixed
   stacks are allowed because both tiers use the same frame layout; a JIT frame calling an
   interpreted `Func` goes through an `yavs_rt_call_interp` callback.
4. The debug loop never runs JIT code: attaching a debugger with breakpoints pins processes to the
   interpreter.

**As implemented (M6, `docs/decisions.md` "LLVM tier: side exits to the interpreter", accepted
by the user, 2026-10-19):**
compiled code runs on the interpreter frame's registers, slots and temporary area (`TierFrame`,
a view of the machine's stacks) and executes the instructions it does not lower in place through
a `yavs_rt_step` callback, calls and natives included: a callee with compiled code runs directly
on the frame the interpreter pushed (#336). It returns to the interpreter at returns,
suspensions, failing inline instructions and calls of functions without compiled code
(`TIER_UNWIND`: the frames are the interpreter's from there); memory instructions reach other
objects through the interpreter's own lookup (`TierHost::object`). `--tier=llvm` compiles
everything before time 0. `--tier=auto` (#125) is the tier-up protocol above with these
differences: hotness is the interpreted time the compile thread samples per function (#432), the
side table is a per-function `OnceLock` in the tier, and the swap happens at the next call,
return or (every 256) jumps rather than only at entries and resumptions (safe because both tiers
share the frame and compiled code enters at any pc). Registers a frame starts with: the preloaded `Const`/`ConstF`/`AddrOf` values, zero elsewhere, except that the registers holding the address of a lean function's parameter slot are zero too (`Pre::next` skips their `AddrOf`): compiled code entering at any pc must not read them (#566). The size cap is 50k yavs IR instructions by default, `--tier-max-size=N`
(user, 2026-10-19, #126).

*Update 2026-10-28:* a second generator, Cranelift (`yavs-clift`, feature `clift`), implements the same
compiled-code contract and the same `--tier=auto` policy (`yavs-llvm` `AutoTier` with a different
`Generator`); the release archives use it. The hot-function size cap and the `--tier-max-size`
option apply to both.

**Identical semantics:** the interpreter is the reference (rule 5). CI runs every difftest group
`--tier interp` and `--tier llvm` (everything compiled, threshold ignored except over-size) and
requires identical canonical output.

## 6. Interfaces: kernel, event bus, VHPI, waves, DAP

| Consumer | Uses | From |
|---|---|---|
| CLI, DAP, VHPI | `SimControl` only (run_for, run_until, step_delta, pause, force, deposit, value, driving_value) | kernel |
| waves (`yavs-wave`) | `Observer::value_change(SignalId, Range)`; `HierarchyBuilder` filled from `Elaborated.hier` + `nets.leaves` (record fields as scopes, logic arrays as `SigType::Vector`, values transposed from planes per changed range, enum names from `Ty::Enum` literals, original spelling); wave entries are keyed by the kernel signal that stores the leaf (`NetGraph::kpart`) | elab, kernel bus |
| VHPI (M5) | object model built from `hier`, `nets`, `Program.types` (records, arrays, generics, generates native, rule 9); handle = generational id over `ObjId`/`ScopeId`; values decoded with `Layout` (logic arrays transposed from planes); `vhpi_put_value` = `SimControl::force/deposit` through `SubElemMap`; callbacks = `Observer` | elab, kernel |
| coverage (M9) | `Program.cov` items (path + span hash) and the counter array; toggle via `Observer` | ir, interp |
| DAP (M8) | `Debugger` hook, `FrameView`, `Program.stmts` for stack frames, `hier` for the hierarchy view | interp, elab |

A leaf is stored at `NetGraph::loc[leaf]`: a kernel signal and the element its range starts at
(docs/design/record-signal-layout.md, #686); kernel signals are in the order of their first leaf.
`ProcId` equals elaboration order.

**Kernel asks** (none blocks slices 1 to 3 below):

| # | Ask | Needed by |
|---|---|---|
| K1 | Port relations in the update phase: `Builder::port(formal, actual, mode, conv: Option<Converter>)`, effective/driving value of the formal from the actual in net order, converters run without a delta | uncollapsed ports, conversions (slice 4) |
| K2 | Implicit `'stable(T)`, `'quiet(T)` signals maintained by the kernel like `'transaction` (a process would add a delta) | `sigassign/stable_quiet` |
| K3 | `Ctx::driving_value(DriverId)` | `'driving_value`, reading `out` ports |
| K4 | `Ctx::driving(DriverId)` and null transactions (`disconnect`) | guarded signals, M3 |
| K5 | Null-range signals (`len == 0`) accepted as empty | null arrays as signals, M3 |
| K6 | Buffered writes to user files (stream ids beyond `OUTPUT`): dropped (#82), disk files are direct and their callers serial | textio, M3 |
| K7 | Logic leaves stored as yavs-logic planes (`SignalType` variant), element `Range` copies, resolution and events on planes | slice 2 (in progress, kernel agent) |

## 7. M2 implementation order

Each slice ends with `cargo test`, `clippy -D warnings`, and the listed difftest groups green
against the reference simulator at `--threads 1` and `--threads 4` (`tools/difftest/difftest.py --candidate yavs
--filter ...`). Groups not listed stay red and are not a slice failure.

| Slice | Content | Turns green |
|---|---|---|
| 1 | `yavs-ir` core: types for scalars and 1-D arrays with dope, frames, checked int/real/time ops, bit-plane logic arrays (`LGet/LSet/LCopy`), `Check`, control flow, `Report`, `Call`/`CallNative` for `'image`/`to_string`/implicit families on scalars; interpreter fast loop; elab of a single top entity with no ports; processes with `wait for`/`wait`; runtime stacktraces | `report/`, `arith/`, `scalar/` |
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

## 8. Decisions (user answers to the open questions)

| # | Question | Decision |
|---|---|---|
| Q1 | Logic array layout | Bit-planes everywhere now, signals included (1.3, K7). Transposes only at waves, VHPI, DAP, foreign calls and string conversion. |
| Q2 | Port collapsing default | Off in M2; every difftest group runs collapsed and uncollapsed. Superseded 2026-10-19: on by default, `--no-collapse-ports` to keep ports apart (`docs/decisions.md`). |
| Q3 | Protected types under `--threads N` | All protected callers serial through M3; per-instance locks later only if benchmarks ask for it. |
| Q4 | Generic types | Shared code with runtime descriptors; specialisation is a later LLVM-tier option. |
| Q5 | VHPIDIRECT calls | No libffi: own per-ABI trampolines in `yavs-ffi` (1.7), tested on SysV x86_64, AAPCS64 and Win64. |
| Q6 | Tier-up threshold | Sampled interpreted time of about twice the O0 compile time: 2 ms plus 40 us per IR instruction of the function, one sample a millisecond (`--tier=auto`, #125, #432; hidden `--tier-heat` for tests). |
| Q7 | Kernel asks | K1 to K7 handed to the kernel agent. |

## 9. Implementation notes and deviations (yavs-ir, yavs-interp)

What the implementation does differently from sections 1 to 3, and what producers must emit.

| Area | Implemented as |
|---|---|
| bounds-checked `Index{..idx..}` | split: `Pos{dst, dope, dim, idx, what}` (checked position from the left, message from the dope's own direction) then `Index{dst, base, pos, stride}`; logic arrays index from the right, so lowering computes `len - 1 - pos` for `LGet`/`LSet` |
| slice start | `SlicePos{dst, dope, left, right, dir, what, dir_what}` (added, #520): 0 for a null slice, else the checked `Pos` of `left`, `right` checked as well, and `dir` against the array's direction; one instruction instead of a branch and three |
| temporaries | `Alloc{dst, size}` (added) bumps the per-process temp area; `Stmt` resets it to the current frame's base. `Ret` does not reset, so a callee's result in temp memory lives until the caller's next statement |
| temporaries of a process in its frame | the optimiser (`opt/slots.rs`, #668) turns an `Alloc` of a constant size in a process whose block every use reaches within the basic block into `AddrOf` of frame bytes past the process's own slots (a fresh register defined once, and so every address derived from it by a constant) plus `Zero{addr, bytes}` unless stores, a `MakeDope` or a `Copy` write every word first; blocks not alive together share bytes; the markers that free nothing any more go |
| package data of run-time size | `PkgAlloc{dst, size}` (added, #73) appends zeroed bytes to the data of the package being initialised (package initialisation thunks only, an ICE elsewhere), for constants whose bounds are known only at run time |
| `Stmt{id}` | `Stmt` has no operand: its statement is `Func::loc[pc]` |
| address spaces | `Up(d)` is only an `AddrOf` operand (it yields a frame address). Added `Shared` (shared variables, serial processes only). Runtime addresses use 8 spaces: frame, instance, package, constant, heap, temp, shared, shared heap; the heap tag is the space |
| `Ty` | `Enum` keeps literal names (for `'image`), `Physical {lo, hi, units}` is its own variant, `Array.elem_desc` / `Field.desc` hold static constraints as pool constants, `Protected {state: TyId}` names the record of its variables (units are elab's) |
| frames | `Func.frame` (size), `Func.roots`, `Func.locals`; `Param {slot, width, class, mode, ty}`; a signal parameter is `Width::Sig` (16 bytes, two registers) |
| fast dispatch | Without a debugger or a compiled tier the interpreter runs `pre::Code`, made once per function on its first call: 24-byte ops, constants and variable addresses a frame starts with left out, unconditional jumps threaded, targets as op indices, frame and instance accesses through preloaded addresses as slot offsets, constant `Index` offsets folded into the access, `Cmp` fused with its `CondBr`, `LayoutOf` with the one `Field` or load that reads it; everything else steps the IR instruction. Frames keep IR pcs (`Code::at`, `Code::entry`), so traces, messages, root maps, coverage, DAP and compiled code are unchanged; fusion never crosses a block start, so no frame stops inside a fused op (#431) |
| operand lists | `Regs {start, len}` into `Func::operands`, so `Inst` is `Copy`; it is 32 bytes (the largest, `Drive`, has optional operands), not 8 to 16 |
| attributes | `SigAttr {Event, Active, LastEvent, LastActive}`, `SigLastValue`, `Driving` and `DrivingValue` per driver slot |
| logic ops | added `LEq` (element-wise `=`); `LNum::{Add, Sub, Cmp(op)}`; a compare with a metavalue is FALSE (numeric_std) |
| real ops | `FBin {Add, Sub, Mul, Div, PowI, Pow}`, `Phys {MulReal, DivReal}`; a non-finite result or a division by zero is a runtime error |
| natives | `NativeRef {symbol, ty}`: implicit families carry their type; `yavs.attr.image` (added, `'image`). Strings return the address of a fat pair `[data, dope]` (dope `1 to len`). An unknown symbol fails `Runtime::new`; a known but unimplemented one is a runtime error naming it |
| foreign | `ForeignRef {name, symbol, args: Vec<ForeignArg>, ret, model: Option<VhpiModel>}` (`model` set for `"VHPI library model"` subprograms: library, model, parameter names, modes and types; the call goes to the plugin's model, not a C function) with `ForeignArg::{Value(CType), ScalarRef(CType, Width), Array {ty, lengths, copy_back}, Record {ty, copy_back}, Access}`. Arrays: the register is a fat-pair address (constrained arrays too); C gets a C-layout copy left to right (logic elements as one code byte, BIT/BOOLEAN as 0/1) and `lengths` `int64_t` lengths. Records of scalars: a naturally aligned C struct. Access: a pointer to the object's data in VHDL layout (after the descriptor of an array object), NULL for null. Copy-back range-checks what C wrote. The elaborator binds pointers through `Bindings.foreign`; `Runtime::new` fails on unbound or mismatched signatures |
| heap | handle `{tag: 1 private / 2 shared, index: u24, generation: u32}`. A heap address carries no generation; a deallocated slot is quarantined until a suspension finds no address to it in the owner's `Addr` slots and recorded temporaries (or, once 4096 wait, a recycle before a native call, `new` or `deallocate` that also counts the live `Ptr` registers, #355, #422), so a stale address always meets the freed slot (runtime error, #81). Handles are found through each object's descriptor (#80). GC: mark and sweep of the private heap at a suspension once `Limits.gc_threshold` bytes were allocated, and inside a long activation (#409); roots are precise (1.4, 1.6, #422); objects that those addresses reach are kept; an object whose handles cannot be located from its type skips the collection (never unsound). The shared heap is collected at the start of a cycle by `SharedGc` (#303, docs/decisions.md "Shared heap collection"). `RegTy::Handle` (added, #422) types the registers that hold handles; the verifier wants one for `New`, `Deref` and `CallProt` and takes it wherever an `I64` goes |
| std.env 2019 | call paths from the frame stack and `Bindings::sources`; the VHDL assert state is per-cycle snapshots plus each process's own changes, merged in process order (deterministic at any thread count); records go to natives as fat pairs (docs/decisions.md, #99) |
| verifier | the "no register live across a suspension" check is intra-block only; the "written before read" dataflow check (debug builds, #430) covers cross-block liveness across a `Suspend` of a process for registers that are not constants |
| serial marking | `TOUCHES_SHARED` from `AddrOf Shared`, `CallProt`, `New {shared: true}` and `CallNative` of a file or directory native (#82); `InterpProcess::new` refuses a serial process without a `SharedState`, and touching shared state without one is an ICE |
| elaboration | `Elab` instructions are a runtime "needs yavs-elab" error; `InstData {data, sigs, cov_base}` has no child or package links yet. `call_shared` runs elaboration code with the `SharedState` (shared variables, shared protected objects) |
| native arguments | `in` scalars by value; `out`/`inout` scalars and access variables by cell address; unconstrained arrays as a fat-pair address, except for the natives of `ieee` (`yavs.ieee.*`), which take each array as four registers `data, left, right, dir` (`yavs_ir::ieee::ARRAY_REGS`, checked by the verifier; #697), and `yavs.textio.read.line_string*`, whose string target is four registers after the line (no fat pair per call, #771); files as the address of their 4-byte cell; records by address. Results: scalars by value, strings as a fat-pair address, `line` as a handle |
| files | a file cell holds index + 1 into the runtime's file table (0: never opened); `FILE_INPUT` = 1 and `FILE_OUTPUT` = 2 are textio's `INPUT`/`OUTPUT`. Disk files are read and written directly (not via K6 streams), so a process that writes, closes and reopens a file in one delta reads what it wrote; every caller of a file native is serial, so the bytes do not depend on `--threads`. `STD_OUTPUT` goes through the kernel's OUTPUT stream, ordered with `report`. Implicit file ops are implemented for TEXT; binary `read`/`write` handle scalars and arrays of scalars (`src/binfile.rs`, format in decisions.md "Binary files"), not records or arrays of composites yet |
| lines | `line` objects are `[1 to len][bytes]`; natives replace and deallocate them. A line cell in shared memory gets a shared-heap object |
| logic BIT | BIT and BOOLEAN elements of logic arrays: the interpreter writes the codes of '0' and '1' (so the std_ulogic plane ops give correct BIT results) and reads either those or the positions 0/1 that yavs-elab uses for BIT leaves. **Open: one encoding must be chosen** (element 0 is the rightmost) |
| protected | the object is a handle to its state record (`New {shared}`), methods get `self` first (`CallProt`); a shared object is created at elaboration through `call_shared` |
| checks | `CheckMsg {what, dir, subtype}`; messages follow the reference simulator (`value 101 outside of PCT range 0 to 100 for variable P`). Added `BinRange` (integer op checked against the base type, `result of .. cannot be represented as T`), `FToIRange` (real to integer against the subtype, message shows the real), `FCheck` (real subtype constraints). Integer division by zero reads `division by zero` |
| thunks | may return a value or not |
| stop severity | after a `Report` at `Ctx::stop_severity()` the process executes no further |
| constants | compact constants (`Repeat`, `Runs`, `Logic`) are expanded on first use by address (`AddrOf Const`) |
| debug | section 3.4 (rewritten for the reference simulators requirements) |
| optimiser | `yavs-ir` `opt.rs` (#427) runs on processes, procedures and functions after flag propagation, per function the first time `Program::func` is asked for it (never-called library subprograms cost nothing; `Program::funcs` keeps the unoptimised code, `Func::opt` the optimised copy with its own numbering; a function's version is fixed at first use because frames re-find their function by id): constant folding and identities, branch folding, `Index` of a known address to `AddrOf`, constant-pool descriptor/scalar reads, `Field` of a descriptor-free record to an `Index` by a constant (new registers and `Const` instructions are added), available expressions and store-to-load forwarding per extended basic block, dead pure definitions. Scalar frame slots of subprograms that cannot suspend and that only direct loads and stores reach (address never passed, indexed or copied; not a GC root, unless every access is a register of the root's own type; no nested subprogram) become registers (`Opt::promote`): loads and stores turn into moves that fold into the neighbouring instructions (copies propagated across blocks where they are still equal, a stored value computed straight into the register), the first pass's folding runs again over the result, and blocks no branch reaches are emptied. In a subprogram, with no debugger running, a `Stmt` marker with nothing to free is dropped (a marker only frees the temporaries made since the one before it, so it stays only where a path reaches it through an instruction that leaves some: `opt::leaves_temps`, with the callee flag `NO_TEMPS`), registers defined once by the same constant or frame/instance/constant address become one register, and the registers in use get consecutive numbers (every call copies a frame template with one cell per register). The storage of a local composite variable of a subprogram that cannot suspend becomes `NewFrame` (frame-pinned temporary memory, docs/decisions.md "Local composite variables live in the frame's temporary memory"). A call of a function with a result table (docs/decisions.md "Result tables for pure functions of one small discrete argument") whose read is one path is replaced by the read before the first pass, where the registers of the read are defined once (`opt/inline.rs` `tab_calls`, #744). Interval analysis (`opt/range.rs`) drops provable checks and decided tests, liveness (`opt/live.rs`) removes dead definitions of multiply-defined registers (docs/decisions.md "Interval analysis, jump threading and dead register definitions"). A named variable's slot is promoted only while no debugger runs (`opt::keep_variables`, set by `--dap` and by `DebugSession::new`). Tables are dropped at `Suspend` and suspending calls (suspension invariant), memory facts at calls and at `Stmt` for named locals and pointer-reached memory (a debugger may write a variable). Stores to slots that stay in memory, `Stmt` and `Cover` stay; a check goes only when it provably passes, so runtime errors and locations are unchanged. Last, calls of thin functions (one path, reads, computations, checks, calls) are replaced by their code (`opt/inline.rs`; `Func::via` keeps what a trace and a call path show; `YAVS_NO_INLINE=1` disables it). `YAVS_NO_IROPT=1` disables it |

**What lowering and elaboration must produce for the interpreter:**

- A `Program` through `ProgramBuilder::finish` (it propagates flags, numbers resume points and
  verifies); every instruction inside a statement (`FuncBuilder::stmt`), spans on statements.
- Per process instance: `InstData` (generic/constant bytes, the instance's kernel leaves as
  `InstSig {signal, ty}`), one `ProcDriver {id, sig, range}` per `Drive` slot (kernel
  `Builder::driver` per longest static prefix), `proc_flags` for the kernel, and a `SharedState`
  for serial processes.
- Waveforms in memory as `n` pairs `[time: i64][value]`, value padded to 8 bytes (bytes or
  bit-planes per the leaf's `SignalType`).
- `wait on .. until .. for` as in 1.5 with the deadline in a frame slot; `process (s)` ends with
  `WaitOn; Suspend; Br`.
- `Func::locals` for every named variable and typed parameters, so the debugger can show them.
