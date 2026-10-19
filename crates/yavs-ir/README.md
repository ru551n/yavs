# yavs-ir

The register IR (docs/design/ir.md section 1) and, in `crates/yavs-interp`, its interpreter.
For producers (`yavs-lower`, `yavs-elab`): build with `ProgramBuilder` / `FuncBuilder`,
`finish()` propagates `MAY_SUSPEND` / `TOUCHES_SHARED`, numbers resume points and runs the
verifier; its errors are ICEs of the producer. `Program`'s `Display` and `dump_func` give a
textual dump.

## Stable (build on it)

- Ids: 32-bit newtypes (`FuncId`, `BlockId`, `TyId`, `ConstId`, `NativeId`, `ForeignId`,
  `CheckMsgId`, `CaseTableId`, `CovItem`) in `IdVec`; `StmtId` is `yavs_kernel::StmtId`.
- `Program`, `Func` (kind, parent for static links, params as frame slots, locals, root map,
  register types, blocks over `code`, `loc` = one `StmtId` per instruction), `StmtInfo`
  (span, func, kind, parent).
- Value model: `Ty` (one description per type; constraints are runtime descriptors, static
  ones are pool constants), `Ty::layout` / `desc_size`, the descriptor format (`[left, right,
  dir]` i64 per dimension), bit-plane logic arrays, `ConstData` compact constants.
- Calling convention: arguments are register cells stored into the callee's parameter slots
  (a signal parameter is two cells; one with records in its type is one such parameter per
  leaf, see yavs-lower DESIGN.md); composites are passed by address; a function result is
  one cell, a string/unconstrained array result is the address of a fat pair `[data, dope]`.
- Native argument convention (ir.md 9): `in` scalars by value; `out`/`inout` scalars and
  access variables (`line`) by the address of the cell; an unconstrained array (`string`,
  `bit_vector`) as the address of a fat pair `[data, dope]`; a file object as the address of
  its 4-byte cell (`yavs_interp::FILE_INPUT` / `FILE_OUTPUT` are textio's pre-opened files);
  a record (`directory`, `time_record`) as a fat pair like an array (its data address first; a
  record result is returned the same way). A heap array object is `[descriptor][elements]`. BIT
  and BOOLEAN elements of logic arrays are the std_ulogic codes of '0' and '1'.
- Natives are `lib/NATIVE.md` symbols (`NativeRef`), implicit families carry their `TyId`;
  `yavs.attr.image` is the `'image` attribute. Foreign calls are `ForeignRef` (the elaborator
  binds the function pointer through `yavs_interp::Bindings`).
- Interpreter API: `Runtime::new(Arc<Program>, Bindings)`, `InterpProcess::new(rt, func,
  Arc<InstData>, Vec<ProcDriver>, shared)`, `proc_flags`, `call` (elaboration-time
  evaluation), `call_values` (the same with composite arguments and results, `CallVal`), `call_shared` (the same with the `SharedState`, to create shared protected
  objects), `DebugSession`.
- Protected types: the object is a handle (`New { shared: true }` for a shared variable,
  `false` for a process-local one) to the state record; methods are subprograms whose first
  parameter is `self`, called with `CallProt`.

## Changes

- `opt.rs` (#427): `ProgramBuilder::finish`/`snapshot` optimise every process, procedure and
  function once (not `Elab`/`Thunk`) after flag propagation and before resume numbering and
  verification: constant folding, identities, branch folding, `Index` of a known address
  becomes `AddrOf`, constant-pool descriptor and scalar reads, `Field` of a record whose
  layout does not depend on a descriptor becomes an `Index` by a constant (the `LayoutOf` goes
  when unread; the pass adds registers and `Const` instructions), available-expression
  sharing per extended basic block (checks that cannot fail a second time included),
  store-to-load forwarding, dead pure definitions. The declarative part of a process (blocks
  before `Func::body`) runs once and is left alone. `YAVS_NO_IROPT=1` disables it. `Inst::visit_mut` is the
  writable form of `Inst::visit`.
- Narrow integer elements (#415): `Ty::elem_size` is the size of an array element; an integer
  type whose range fits in 32 bits takes 4 bytes there (`Ty::layout` strides follow it), every
  other scalar its `scalar_size`. `Width::B4` loads sign-extend. Producers load and store
  array elements with the element width, scalars with the scalar width.
- `Inst::SigParts` and `Inst::SigSlice` (#71): signal references of several parts, possibly in
  different signals, in the same two registers (ir.md 1.8); `SIG_PART_BYTES` is the size of a
  `SigParts` entry. Producers slice references with `SigSlice`, never with arithmetic on the
  packed cell.
- `Program.reflect` (`Refl`, `ReflKind`, `ReflId`): subtype descriptions for `std.reflection`
  mirrors, built by `ProgramBuilder::reflect` / `reflect_set`; `yavs.attr.reflect` takes
  `(refl, is_value, v, ranges)`, `ranges` the run-time bounds of reachable scalar subtypes with
  a non-static range or 0 (yavs-interp `reflect.rs`).

- `ForeignArg` gained `Array { ty, lengths, copy_back }`, `Record { ty, copy_back }` and
  `Access` (still `Copy`); an exhaustive `match` on it needs the new arms.
- `Bindings` gained `revision` and `resolution` (and implements `Default` by hand).
- `FuncBuilder::param` takes `ty: Option<TyId>` (typed parameters are debugger locals).

## Still moving

- Exact instruction operand shapes for records with runtime layouts and `Elab*` instructions
  (their semantics belong to `yavs-elab`).
- `InstData` gains child instances and package links when elaboration needs them.
- Binary (non-TEXT) file `read`/`write` handle scalars and arrays of scalars; records and
  arrays of composites, and the PSL natives, are not implemented (runtime error
  naming the symbol); `SetVhdlAssertFormat` accepts only the empty (default) format.
- Call paths need `Bindings::sources` (`yavs_interp::Sources::of(program, source_map)`;
  yavs-elab passes it); without it they are a runtime error.

## Requests

From `yavs-lower` (2026-10-18), found running `tests/corpus/{scalar,arith,process,report}`
through lowering and the interpreter (`crates/yavs-lower/tests/corpus.rs`):

1. **Thunks with a value.** The verifier rejects `Ret { v: Some(_) }` in a `FuncKind::Thunk`,
   but a thunk evaluates an expression. Lowering uses parameterless `Function`s until thunks may
   return their cell.
2. **`yavs.attr.value`** is accepted by `Runtime::new` but not implemented (`scalar/*_attrs`).
   Lowering passes the string as the address of a fat pair `[data, dope]`, `NativeRef::ty` = the
   prefix's base type.
3. **Report at the stop severity.** A `failure` report is committed but the process keeps
   running to its next suspension, so `report/failure_exit` prints a report the reference simulator never reaches.
4. **reference-simulator-compatible check texts** (difftest is strict for yavs): range checks read
   `value 101 outside of PCT range 0 to 100 for variable P` (subtype name, then the object),
   real to integer `value 30000000000 outside of INTEGER range ...`, integer overflow
   `result of 2147483647 + 1 cannot be represented as INTEGER`, division `division by zero`.
   The overflow text needs the operands and the type in the instruction: a checked `Bin` with a
   result range (or a `CheckKind::Overflow` taking `a`, `b` and the op) would do. Lowering emits
   `Bin` then `Check { Range }` against the base type today.
5. **Range checks on reals**: `Check` compares i64 only, so real subtype constraints are not
   checked yet.
6. **`to_ostring`/`to_hstring` symbols**: sem names them `yavs.implicit.to_hstring.<type>`,
   NATIVE.md lists them under the `to_string` family, so `Runtime::new` rejects the program
   (`scalar/string_attrs`). One of the two needs to change (sem's `mangle.rs` or NATIVE.md).

Answers (yavs-ir/yavs-interp, 2026-10-18):

1. Done: a `Thunk` may `Ret` with or without a value (and be called either way).
2. Done: `yavs.attr.value` for integer, real, physical and enumeration types (fat-pair string,
   `NativeRef::ty` the base type); a bad string is a runtime error `'value: ...`.
3. Done: after a `Report` at `Ctx::stop_severity()` (new, yavs-kernel) the process stops
   executing.
4. Done: `ProgramBuilder::check_msg_for(what, dir, subtype)`; `Check`/`Pos` read `value 101
   outside of PCT range 0 to 100 for variable P` (`index ...` for index checks); new
   `Inst::BinRange {op, dst, a, b, lo, hi, what}` gives `result of 2147483647 + 1 cannot be
   represented as INTEGER`; new `Inst::FToIRange {dst, src, lo, hi, what}` gives `value 30000000000
   outside of INTEGER range ...`; every integer division by zero reads `division by zero`.
5. Done: `Inst::FCheck {v, lo, hi, what}` on F64 registers.
6. Accepted on this side: `yavs.implicit.to_hstring.<type>` and `.to_ostring.<type>` are valid
   and implemented for bit arrays (as is `to_string` of character and bit arrays). NATIVE.md
   now lists `to_ostring`, `to_hstring` and `file_canseek` as families of their own (#163).

From yavs-elab (2026-10-18):

1. **`std.env.finish`/`stop` end the process at once.** Like 3: after
   `finish(3)` the process runs on to its next `wait`, so `env/finish_status` and
   `env/finish_in_function_call_order` print a report the reference simulator never reaches.

More requests from `yavs-lower` (2026-10-18):

7. **`out` signal parameters**: `Drive`/`Disconnect` name a static driver slot, but a procedure
   with `signal s : out T` drives the caller's slot. A driver slot passed in a register (one
   more cell next to the signal reference) would do (`process/subprograms`).
   Done (yavs-lower, 2026-10-18): `Inst::DriveSlot { slot, target, .. }` drives through the
   driver slot in a register; `out`/`inout` signal parameters pass it as a cell after the
   signal reference.
8. **Composite instance data**: constants and signal initial values of array type are computed
   by parameterless functions returning a fat pair; the elaborator needs the bytes behind it
   (`yavs_interp::call` returns only the cell). A `call` variant returning the data and dope
   bytes of an array result would let `yavs-lower` place array signals and constants in
   instance data (`scalar/array_attrs`, `delta/delta_chain`).
   Done (yavs-elab, 2026-10-18, #77): `yavs_interp::call_values(rt, func, inst, args, ret)`
   takes and returns `CallVal::{Cell, Composite { data, desc }}`; a composite result (`ret`
   = its IR type) is read through the fat pair with `Ty::desc_size` and `Ty::layout`.
