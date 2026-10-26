# yavs-lower: typed HIR to yavs-ir

Binding inputs: `docs/plan.md` (rules 1, 5, 6, 7, 10, 12, the error motto, "Designed for the
GUI"), `docs/design/ir.md` (sections 1.2 to 1.5, 2, 7), `docs/design/sem.md` section 7. This
crate owns no IR types: it builds `yavs_ir::Program` through yavs-ir's builder and runs the
yavs-ir verifier on every function it emits.

## Modules and owners

| Module | Area | Owner |
|---|---|---|
| `lib.rs` | public API (`Lowerer`, `UnitIr`), `Cx`, `Obj` locations, codes | lower (additive changes welcome) |
| `func.rs` | `Fx`, frames, parameters, declarations, subprograms, processes, architectures | lower |
| `expr.rs` | scalar expressions, calls, implicit operations, attributes, names, 1-D arrays, aggregates of scalars | lower |
| `stmt.rs` | sequential statements, waits, signal assignments | lower |
| `assoc.rs` | individual association and conversions of subprogram parameters | lower |
| `ty.rs` | HIR type to IR `Ty`, widths, `signal_elem` (core); record/access/file/protected `Ty` arms | lower (core), lower2 (composite arms) |
| `composite.rs`, `access.rs`, `protected.rs`, `file.rs`, ... | records, nested arrays, access types, protected types, textio/files, resolution, force/release | lower2 |

Hooks for lower2: `Fx::{expr, scalar_expr, place, store_place, range, subtype_dope, layout,
check_scalar, call}`, `Obj` variants (add new ones rather than changing existing ones),
`Cx::unsupported`. Unsupported constructs in lower's files report `E2001`; replace such an arm by
a call into your module in a small commit.

## Shape

- Input: an analyzed `yavs_sem::Db` (no error diagnostics; lowering a unit with errors is an ICE)
  and the units to lower. Output: functions added to one `Program`, plus a `UnitIr` per design
  unit (decl slots, elab function, process functions).
- One `Func` per process, subprogram body and elab body **per design unit**, shared by every
  instance (ir.md 2). Generics, signals and declared constants of the unit are instance data
  (`Space::Inst`), never folded into code, unless sem folded the expression to a `ConstId`
  (locally static: same value in every instance).
- Each subprogram is lowered once, on first reference (worklist keyed by `DeclId`), so the std and
  ieee bodies a design never calls are not lowered. The caller loads the package bodies it needs
  (`Db::load_body`) before lowering.
- Every emitted instruction carries the current `StmtId`. HIR `StmtId`s map 1:1 to IR `StmtId`s
  (`StmtInfo { span, func, kind, parent }`); declaration initialisers and the implicit wait of a
  process get their own IR statements with the declaration's span. A `Stmt{id}` marker opens each
  statement (debug hook, ir.md 1.5).
- Coverage: with `LowerOptions::coverage` set, a `Cover{item}` is emitted at each statement and at
  the head of each branch arm (if/elsif/else, case alternative, loop body, conditional assignment
  arm); `CovInfo` gets the span. Off by default, so default code has no counters.

## Registers, slots, temporaries

- Objects (variables, constants with a value, loop parameters, parameters) are frame slots, never
  registers. Registers are temporaries of one statement (the builder hands out fresh ones; reuse
  is a later pass). No register is live across `Suspend`: a `wait` is a statement of its own, its
  deadline and a `for` loop's bounds live in slots.
- Composite temporaries (results of `&`, `'image`, slices of logic arrays) are `Alloc`ed: they live
  until the next `Stmt` of the frame that made them, so a callee's result survives until the
  caller's statement ends.
- An array object's slot is `[data, dope pointer, heap handle, dope (24 bytes per dimension)]`;
  the elements are a private heap object (`New`, rooted by the handle, rule 6), so the size can
  be a runtime value without a second static path.

## Values and descriptors (rule 1)

- Scalars: one register (i64, f64 bits, or the enumeration position).
- Arrays: `(data, dope)` register pairs. Every array subtype the code needs gets a descriptor:
  a pool constant when sem folded all bounds, else `MakeDope` from the lowered `RangeC`s (the same
  format either way).
- Logic arrays (`LogicKind` leaf: STD_ULOGIC, BIT, BOOLEAN) are bit-planes (ir.md 1.3); element
  `i` from the left of an `n`-element array is plane element `n - 1 - i`. Codes equal positions.
- String literals and other folded arrays are `ConstData` (`Bytes` used in place, `Runs` and
  `Logic` copied with `CopyConst`, rule 12).

## Contract with yavs-interp and yavs-elab

- Calls: one cell per scalar `in` parameter (8-byte slot); the address of the actual for a scalar
  `out`/`inout` variable (a frame temporary written back after the call for logic elements and
  slices); two cells (data, dope) per array parameter. Signal parameters: the signal reference
  (two cells), then the caller's driver slot for `out`/`inout`, then the actual's descriptor for
  arrays; assignments to an `out` signal parameter use `DriveSlot`. A signal parameter with
  records in its type (`RecParam`) is one reference per leaf of the formal, then the driver
  slot of the first leaf (`out`/`inout`; leaf `k` uses that slot plus `k`), then the actual's
  descriptor when the type needs one (#215). A formal with a mode view (#254) passes each leaf
  the view drives as a one-part `SigParts` reference with its own driver slot and `-1` in the
  slot cell (`Fx::view_leaves`; also per formal leaf of an individually associated view formal,
  `assoc.rs`, #265). A constrained formal gets
  the formal's descriptor after a length check. An individually associated parameter
  (`Actual::Individual`, 6.5.7.1, `assoc.rs`) of variable or constant class is a temporary of
  the formal's subtype: each part is copied in through its formal part (a name of the formal,
  bound to the temporary while it is lowered) and, for `out`/`inout`, copied back after the
  call. A signal formal of one part takes its actual's reference; one of several parts is a
  reference of several parts (`Inst::SigParts`, ir.md 1.8): each part's actual with its driver
  slot, at the element offset its formal part names (the formal part lowered with the formal
  bound to a reference from element 0), one such reference per leaf for a formal with
  records, and `-1` in the formal's driver slot cell. An unconstrained formal's bounds run
  from the lowest to the highest index named, in the index subtype's direction. A converted
  parameter (`Actual::Converted` with `copy_out`, an `out`/`inout` variable, #229) is a
  temporary of the formal's subtype holding `copy_in` (or the default for `out`); after the
  call `copy_out`, the formal part with the formal bound to the temporary, goes to the actual. A function's array result is the address of a
  fat pair `[data, dope]` in temporary memory (the dope copied out of the callee's frame).
  A function with a 2019 named return value (`return r of t`, `Db::ret_names`) takes one
  more parameter after the others, an array parameter whose descriptor is the call's result
  subtype (`Expr::subtype` of the call; the data cell repeats the descriptor and is never read);
  the hidden constant it binds gives `r` its bounds (`Constraint::FromObject`, #33). A
  record result passes its record descriptor the same way, and protected methods take it
  after their arguments too (`named_return_cells`, #239). A scalar result passes a
  one-dimensional descriptor of its range (`left`, `right`, direction; a floating point
  bound as its bits), read back by `r`'s range (`Db::scalar_range`, #327). The elaborator
  passes a value of that subtype in the same place (for a scalar: its bounds only).
- Natives: NATIVE.md symbols; implicit families via `yavs_sem::mangle::implicit_native` with the
  operated type as `NativeRef::ty`; `'image` is `yavs.attr.image` (type mark and 2019 object
  forms; a 2019 array or record value as a fat pair, formatted by the interpreter), `'value`
  `yavs.attr.value` (taking the string as a fat pair address; a composite also takes its
  constrained prefix descriptor, or each open index subtype's left and direction). Array and
  record arguments to natives are fat pair addresses, except for the `ieee` natives, whose arrays are the values
  `data, left, right, dir` (`Conv::Flat`, #697): the bounds are the registers the descriptor was made
  from in the same block (`Fx::made_dopes`, filled by `Inst::MakeDope`), else loaded from it.
- `PackageIr` per package with run-time objects (`Lowerer::package`, before the
  architectures): package data `AddrSpace::Pkg(index)` written once by its `init` thunk under
  `yavs_interp::init_package` (constants that are not locally static, composites of static
  layout in place, composites with run-time bounds in package data appended by `PkgAlloc`; shared protected variables); package signals are the first entries of every
  unit's signal table (an array one's descriptor is in package data, `PackageIr::signals`
  `dope`, written by the elaborator after `init`); package files and non-protected shared
  variables (`--relaxed`) live in the shared variables area (`Lowered::shared_size`), the latter
  in the slot layout of a process variable (`VarShape`, `Fx::var_init`, objects in the shared
  heap), initialised by `init`.
- `UnitIr` per architecture: `values` (generics, generate parameters and constants that are not
  locally static: the elaborator writes its value with `UnitIr::write_value`; arrays go after
  the fixed part, their element and descriptor offsets in the slot), `consts` (thunks for scalar
  constants elaboration could not fold), signals (ports and declared signals, mapped by
  declaration; array signals read their bounds from their subtype in this instance),
  processes of the architecture, its blocks and generates, with one driver slot per
  `ProcessInfo::drivers` entry. A subprogram declared in a process (at any depth) runs only
  in that process, so its signal assignments use the process's driver slots (`Cx::proc_drivers`,
  LRM 14.7.2, #206). An actual of a `signal out` parameter passes the driver slot of its own
  longest static prefix (`driver_of`, #231), so calls on different parts of one signal each
  drive their part. The elaborator builds one instance data per region (instance,
  block, generate iteration), looking values and signals up from the process's region
  outwards. Locally static constants have no slot: their folded value is used directly.

## Mapping table

### Expressions (slice 1)

| HIR | IR |
|---|---|
| `Const(c)` / any expr with `value: Some(c)` scalar | `Const`/`ConstF` |
| `Const(c)` composite | pool constant + `CopyConst` into a temp (or used in place when read only) |
| `Obj(d)` variable/constant/param | `AddrOf{Frame or Up(depth) or Inst or Pkg, slot}` then `Load` (scalar) or the address (composite) |
| `Obj(d)` signal | `SigVal` then `SigReadScalar` (scalar) or `SigRead` into a temp |
| `Obj(d)` loop / generate param | `Load` of its slot / instance slot |
| `Call` implicit, scalar int/phys `add sub mul div rem mod exp neg abs pos` | checked `Add`..`Pow`, `Neg`, `Abs` (rule 7), then `Check{Range}` against the result subtype when narrower than the base |
| `Call` implicit, real arithmetic | `FAdd`..`FPow` |
| `Call` implicit `eq neq lt le gt ge` on scalars | `Cmp` / `FCmp` |
| `Call` implicit `and or xor nand nor xnor not` on BOOLEAN/BIT | short-circuit `CondBr` for `and`/`or`/`nand`/`nor` (LRM 9.2.2), else `And/Or/Xor/Not` |
| `Call` implicit `minimum maximum` scalar | `Cmp` + select |
| `Call` implicit on arrays (`cat`, relational, logical, shifts) | `cat` (and the pieces of a positional aggregate): one `Cat` into the result block after the bounds, `Check{Index}` and `SubtypeCheck`; logic arrays: `LCopy` / `LogicOp`; others: `CallNative` with the family symbol |
| `Call` implicit `to_string`, `'image` and friends | `CallNative` (NATIVE.md section 2 symbol, yavs-std implements) |
| `Call` with `Body::Native(sym)` | `CallNative` |
| `Call` with `Body::Hir` | `Call{func}` with arguments per 3. Subprograms |
| `Call` with `Body::Foreign` | `CallForeign` (`foreign.rs`): one cell per parameter as for natives, records by data address |
| `Call` with `implicit_cond` | the `??` family (`Cond`) |
| `Index` | dope bound `Check{Index}` then `Index` / `LGet` |
| `Slice` | `Check{Index}` on both ends unless null, new dope, address arithmetic or `LCopy` |
| `Field` | `Field{layout, n}` (M3) |
| `Attr` on a type mark, scalar `left right high low ascending` | constant from the subtype's lowered range |
| `Attr` `pos val succ pred leftof rightof` | arithmetic on positions plus `Check{Range}` |
| `Attr` `image value` | `CallNative` (`yavs-std` `image`/`value`) with the type descriptor |
| `Attr` on an array object `left right high low length ascending` | `DopeLen` / dope loads for the dimension |
| `Attr` `range reverse_range` | only inside `RangeC::Attr`: the dope of the prefix, direction flipped |
| `Attr` on a signal `event active last_event last_active last_value` | `SigAttr` |
| Clock-edge idioms (`Db::edge_shape`: `rising_edge`/`falling_edge`, `x'event and x = '1'`/`'0'`, BOOLEAN `x'event and x`, `not x'stable and ...`; the level of a `wait until x = '1'` in a non-postponed process that waits on `x` alone) | one `Edge` (`edge.rs`) |
| `Attr` `simple_name path_name instance_name` | instance-data string (elab computes the path) |
| `Aggregate` | static: one pool constant; dynamic: dope from `AggBounds`, then per association `Store`/`LSet`/`LFill`/`Fill`/`LCopy` |
| `Qualified` | operand, then `SubtypeCheck`/`Check{Range}` against `to` |
| `Conv` int/real/enum | `IToF` / `FToI` (checked round) / position move, then `Check{Range}` |
| `Conv` array (closely related) | element conversion only when the element types differ, new dope from `to` or the operand |
| `ImplicitConv` | no code for universal_integer to integer types beyond a `Check{Range}`; `IToF` for universal_integer to a real type |
| `Conditional` (2019) | branches into one result register or temp |

Bound checks on assignment: every scalar store into an object with a narrower subtype than its
type gets `Check{Range, lo, hi}` with a `CheckMsgId` naming the object; arrays get
`Check{Length}` per dimension (implicit subtype conversion, LRM 14.7.3).

### Sequential statements (slice 1, 2)

| HIR | IR |
|---|---|
| `VarAssign` simple | value, checks, `Store` / `Copy` / `LCopy` into the target address |
| `VarAssign` conditional / selected | branches, one store per arm |
| `SignalAssign` | waveform elements into a temp of `(time, value)` pairs (after-times checked ascending and non-negative), `Drive{driver_slot of the target's Lsp, target SigVal, mode}`; `Delay::Default` = inertial with reject = first after-time |
| `If` | `CondBr` chain |
| `Case` with `intervals` | `Case{v, table}` over the folded intervals; array (string) cases: a `CallNative` compare chain |
| `Case` matching (`case?`) | `MEq` compare chain |
| `Loop Forever` / `While` | header block, `CondBr` |
| `Loop For` | range bounds evaluated once into slots, direction known statically or tested at run time, null range skips, the parameter slot written each iteration (no overflow at the last value: compare before increment) |
| `Next` / `Exit` | `Br` to the loop's continue / exit block (optional `CondBr`) |
| `Wait` | ir.md 1.5 sequence: `WaitEdge` first when the `until` condition has a clock edge as an `and` conjunct (`edge.rs` `wait_edge`; not in a postponed process or with coverage counters), `WaitOn` per `Lsp` (or the signals of `until` when `on` is empty, LRM 10.2), `WaitAt deadline`, `Suspend`, re-test `until`, loop |
| `Assert` | `CondBr` on the condition; the failing arm builds the message (`"Assertion violation."` default) and `Report{severity default error}` |
| `Report` | message, `Report{severity default note}` |
| `ProcCall` | as a call statement; a `MAY_SUSPEND` callee: arguments to slots first |
| `Return` | value into the return slot, checks against the return subtype, `Ret` |
| `Null` | `Stmt` marker only |
| `SeqBlock` (2019) | its declarations as slots of the enclosing frame, then its body |
| `Force` / `Release` | `Force` / `Release` (slice 4) |

### Subprograms (slice 1)

- Frame: one slot per parameter, local declaration and result. Nested subprograms reach outer
  slots with `Up{depth, slot}` through the static link.
- Parameters by class and mode:
  - constant / in variable scalar: value copied into the callee slot; composite in: by reference
    (fat pointer `{data, dope}`), copied only when it is an unaligned logic slice (ir.md 1.3).
  - out / inout variable scalar: caller passes the address; callee works on a local copy that is
    written back at return (copy-in/copy-out), so a range check failure leaves the actual intact.
  - signal parameters: a `SigVal` reference; `out`/`inout` signal parameters carry the caller's
    driver slot.
  - unconstrained parameters take the actual's dope; partially constrained ones merge the
    formal's constraint with the actual's (rule 1); constrained formals check the actual's length.
- `Actual::Default`: the default expression is lowered in the caller's context.
- Result: functions return scalars in a register, composites as a fat pointer into the callee's
  temp copied out by the caller before the callee's temp area is released.
- `MAY_SUSPEND` is computed by yavs-ir's propagation pass; lowering only emits `Wait`/calls.

### Processes and signal assignments (slice 2)

- `Process` with a sensitivity list: body, then `WaitOn` each signal and `Suspend` (LRM 11.3);
  `all` uses `ProcessInfo::reads_all`.
- Concurrent signal assignment, assert, procedure call: an implicit process (`IMPLICIT` flag)
  with sensitivity from `ProcessInfo`, guarded assignments behind the `GUARD` signal.
- Driver slots: one per entry of `ProcessInfo::drivers`, in order; a target's `Lsp` is matched to
  its slot. Non-static index parts of the target become the `SigVal` offset at run time.

## Testing

- Unit tests: lower small inline VHDL processes and assert the verifier passes and the expected
  instruction kinds appear with statement ids.
- Corpus: `tests/corpus.rs` analyzes every file of the `tests/corpus` groups in its `GROUPS`
  list against `lib/`, elaborates it with yavs-elab (which lowers through this crate) and runs it
  on yavs-kernel; files in `PASS` must match their `.expected` exactly. The full differential
  run is `tools/difftest` (`--candidate yavs`).

## Composites (lower2: `composite.rs`, `access.rs`, `protected.rs`, `file.rs`, `force.rs`)

- A composite value is `V::A(data, desc)` for records and arrays alike; `desc` is the type's
  descriptor (`Ty::desc_size` bytes: array dims, then the element part when the array type gives
  none; a record's fields without a static constraint). Static descriptors are pool constants,
  others are built from the constraint, unconstrained parts from the actual (rule 1).
- Record fields: `Field` over `LayoutOf`; a scalar field is loaded, a composite one is
  `V::A(field address, field descriptor)`. `Place::Array` serves record targets too
  (`dims` 0).
- Arrays of composites: element address `data + pos * stride` (stride from the layout);
  arrays of logic arrays are one plane vector, a row is an `LCopy` range.
- Signals with records in their type (`Obj::RecSignal`, `Cx::rec_leaves`): one instance signal
  per field path, holding that field of every array element (struct of arrays).
  `Fx::comp_select` turns a name (fields, indices, a final slice) into one kernel range per
  leaf with the rule of `yavs_elab::NetGraph::select`; `leaf_walk` copies a leaf's elements
  between a leaf buffer and the composite value (#200). The whole signal's descriptor is the
  instance-data slot `Cx::sig_dopes` (`SignalIr::dope` of the first leaf entry), written by
  the elaborator from the signal's shape (#215).
- Signals of arrays of arrays without records are one instance signal of all their scalars;
  `Fx::sig_steps` selects a name's range by the same walk (`walk_steps`) with that one leaf
  (#89). A subtype-alias step (`Step::Rebase`) takes the alias subtype's descriptor; an index
  or slice after a slice counts within the slice (`m(0 to 1)(0)`, #230).
- Objects of record and composite-array type are a slot `[data, desc, handle]` over a private
  heap object `[descriptor][value]`, the handle a GC root. Access variables are handle slots;
  allocators build the same `[descriptor][value]` object, so `p.all` keeps its bounds.
- Protected: the object is a handle to the state record (`Ty::Protected { state }`); a method
  gets `self` as its first parameter and a slot per state variable holding its address in the
  state (`Obj::ScalarRef` / `Obj::Array`), so the body's names need no new `Obj` kind. Calls
  are `CallProt`; a call without a prefix inside a method passes `self`. A subprogram
  declared (at any depth) inside a method finds that method's `self` slot by walking its
  enclosing regions (`Cx::method_self`), reaches it through the static link and binds the
  state variables again in its own frame, so its calls and names act on the enclosing
  method's object (LRM 5.6.2, #221).
- A new protected object (`prot_new`): the initial values that give a variable its
  constraint are evaluated first (they build the state descriptor); then the object is
  created with default values and the other initial values run in declaration order with
  that object as `self` and the state variables bound, so they may call methods of the body
  and name earlier variables (LRM 5.6.2, #204). A method call in one of the first kind is
  `E9999` not implemented.
- Files: a 4-byte cell (`Obj::Scalar`, `B4`); natives take its address. textio's `INPUT` and
  `OUTPUT` are the pre-opened cells 1 and 2 (constant cells).
- `force`/`release` (#68): every scalar subelement of the target (LRM 10.5.2.1); a target that
  is one kernel range (scalar, array, slice, a part with one record leaf) gets one `Force`
  with the value's elements (an array checked against the target's bounds), a part with
  records one `Force`/`Release` per leaf with that leaf's elements (`leaf_of_value`); simple,
  conditional and selected forms; default mode `out` for (a part of) a port of mode out,
  inout and buffer, `in` otherwise. sem rejects a member of a composite resolved as a whole
  (`E1362`, lrm-checks L97).
- Shared variables of a protected type in an architecture: an instance cell with the object's
  handle; its thunk creates the object in the shared heap (yavs-elab runs instance thunks with
  `call_shared`). One of a non-protected type (`--relaxed`, #285): an instance cell with the
  address of its slot (`VarShape`) in the shared heap, created and initialised by its thunk;
  names of it are `Space::Region(cell)`, whose `Fx::addr` loads the cell and flags the function
  `TOUCHES_SHARED`, so its processes are serial. Types holding access values are allowed (#303): a package's variable adds its handle words to `Program::shared_roots`, a region's slot is typed for the GC. Composite constants of an architecture elaboration did not fold are
  computed once per region by a thunk returning their fat pair (`InstConst::comp`, #73).
  A 2019 variable port is an instance cell too (`UnitIr::var_ports`), with no thunk: yavs-elab
  writes the handle of the shared variable its actual chain ends at (#188).
- User resolution functions: yavs-elab lowers them (`Ir::resolvers`) and the kernel resolver
  calls `yavs_interp::resolve` (driving values as an array `0 to n - 1`); bit-plane leaves
  are not handled yet.
- 2008 array-valued pieces of positional 1-D aggregates (`(a, '1', b)`, `(rows, row)`) are
  concatenated with bounds as for `&` (`aggregate_pieces`, `put_pieces`, shared with `&`); as
  aggregate targets each piece takes as many elements as its target has, the cover checked
  first (#89). A piece under a range choice fills the range in the aggregate's order
  (`put_at`); an element subtype the type leaves open takes its constraint from the context
  or the first value (#230). Named bounds come from sem's result subtype (direction of a
  constrained context, else of the range choices with pieces, L84).
- `&` on arrays of composites (#234): every operand is a piece of `put_pieces`, an element
  operand one row; an element subtype the type leaves open takes the left operand's (its
  element part, or the element itself), every operand checked against it.
- Not yet: named pieces in aggregate targets, methods of
  protected types in generic package instances (generic instantiation, #71), user
  resolution of logic leaves.

### Requests from lower2 to lower

1. `textio/std_logic_textio` needs case statements over arrays (`case tri is when o"0"`,
   #69) and object aliases (`alias sv : std_ulogic_vector ... is s`) in std_logic_1164's body.
2. `protected/generic_package_prot` needs subprograms of generic package instances (#71).

## Requests from yavs-elab

Answered (yavs-lower, 2026-10-18): 1 (`UnitIr::values`, `write_value`), 2 (ports and signals of
outer regions are `SignalIr`s), 3 (processes in blocks and generates, generate parameters as
values; yavs-elab keys instance data by region), 5 (array signals, element and slice targets
and sensitivity with kernel offsets as below). Open: 4 (`'path_name`), 6 (guarded blocks).
Added by yavs-elab (#77, 2026-10-18): `Lowerer::package_value(decl)`, a thunk returning a
package constant's value (a cell, or a composite's fat pair), so elaboration reads a package
constant with an impure initial value from the package data instead of evaluating it again.
Added by yavs-elab (#220, 2026-10-18): `Lowerer::implicit_native(op)`, the native of an
implicit operation, so elaboration runs predefined operations it does not evaluate itself
(shifts, `TO_STRING` of reals, ...) through yavs-interp (`call_native`).

yavs-elab (`Elaborated::lower`, `Elaborated::build`) lowers every architecture that owns a
process and builds one `InstData` per entity instance: constants from `UnitIr::consts`, the
signal table from `UnitIr::signals` mapped to the elaborated signal objects of that instance,
and one `ProcDriver` per `ProcessIr::drivers` slot with the range of the matching
`ProcessInfo::drivers` entry (`ProcInst::lsps`, in kernel offsets: row major, mirrored for logic
leaves; docs/decisions.md "Elaboration"). The `generics/` and `hierarchy/` difftest groups need:

1. **Entity generics:** instance slots the elaborator fills from its values, e.g.
   `UnitIr::generics: Vec<(DeclId, off, Width)>` (scalars; strings as a fat pair the
   elaborator writes into instance data). Elab already has every generic value
   (`Elaborated::value(scope, decl)`).
2. **Ports and outer signals:** ports as `SignalIr` entries (elab maps them to the formal's
   signal object, which is its own kernel signal, rule 3); signals of an enclosing region seen
   from a generate or block the same way.
3. **Processes in generates and blocks:** listed in `UnitIr::processes` too, with the generate
   parameter as an instance slot. Elab can build one `InstData` per generate iteration if
   `ProcessIr` says which region (sem `ScopeId`) it belongs to and where the parameter lives.
4. **`'path_name`/`'instance_name`:** strings per process scope in instance data, filled by
   elab from `Hierarchy::path_name`/`instance_name` (LRM spelling, `Case::Lrm`).
5. **Array signals:** elab gives each array signal one kernel leaf (`Elem::Logic` for
   std_ulogic/bit/boolean elements, else bytes), so `SigVal` offsets follow the element order
   above.
6. **Guarded blocks** (`resolution/bus_guarded`): elab creates the block's GUARD signal object
   and one process instance per guarded block, named `_guard`, whose statement is the block
   statement and whose only driver slot is GUARD (`GUARD <= guard_expression`, LRM 11.2).
   Lowering needs a `ProcessIr` for it (sensitive to the guard expression's signals) and the
   guarded assignments of the block (`Disconnect` when GUARD is false). `bus`/`register`
   signals are guarded in the kernel by elab (`Builder::guard`, with the time of the
   signal's disconnection specification from `Db::disconnect_times`, 0 without one, #260).
7. **Expression actuals of ports** (#74, LRM 6.5.6.3; answered 2026-10-18: elab calls
   `Lowerer::port_actual(stmt, expr, formal subtype, port)` before lowering the instantiating
   architecture, which then lists one `ProcessIr` with `actual: Some(PortActual)` per such
   actual of instance or block statement `stmt` (its port map, or its binding indication's,
   whose component ports and generics become objects of the unit): no driver slots of its
   own, it drives slot 0 with `target: None` (a type with records: slot `k` for leaf `k`),
   an array value is checked against the descriptor elab writes at `PortActual::dope` and a
   scalar with a non-static range against the bounds at `PortActual::bounds`. The original
   request follows.) An
   association `f => expr` (or `f => inertial expr`) whose actual is not a static signal name
   is a port driven by an equivalent process `f <= expr` in the instantiating region. An
   external name actual without `inertial` is a signal name actual, a plain port edge (#186).
   Elab will create the formal's signal object (bounds from its subtype, or from the
   expression's subtype when the formal is unconstrained) and one process instance in the
   instantiating region with one driver slot, the formal. Lowering would need, in the
   instantiating architecture's `UnitIr`, a list such as `actuals: Vec<ActualIr { stmt,
   formal: u16, process: ProcessIr }>` (the instance statement, the formal's index in the HIR
   association list, a process sensitive to the expression's signals that runs once at
   initialisation, LRM 11.3), and the formal as a `SignalIr` of that unit keyed by the formal's
   `DeclId` so elab maps it to the formal's object. Until then elab reports E9999 naming
   "non-static expressions as port actuals".

## Requests from yavs-interp

From yavs-interp (2026-10-18, #163), the implicit natives (`yavs-interp/src/implicit.rs`):

1. **Null concatenation bounds** (done; `implicit/hand_edges` checks it end to end). The inline `&` takes `S'LEFT` as the left bound also when
   both operands are null arrays; LRM 2008 9.2.5 says the result is then the right operand.
   `x & y` with `x : bit_vector(5 to 4)` and `y : bit_vector(9 downto 10)` gives `0 to -1`,
   the LRM wants `9 downto 10` (the reference simulator 1.23 fails on this case, so a corpus test needs
   `ref=hand`). The native `yavs.implicit.cat.*` already implements the rule.
2. **Native fallback contracts**, for any call that goes to the implicit natives instead of
   being inlined: `cat` takes three more cells after its operands (`S'LEFT`, `S'RIGHT`, the
   direction 0 `to` / 1 `downto` of the index subtype); `rising_edge`/`falling_edge` take the
   signal as its two `Sig` cells; `deallocate` takes the address of the access object (inline
   `Free` is used today, which is equivalent).
3. Records reach natives as fat pairs (what `cells` emits); the interpreter now reads every
   record argument (`directory`, `time_record`, `call_path_element`) that way and returns a
   record result as a fat pair too (yavs-ir README, native convention). Nothing to change in
   lowering; noted so the two stay in step.
