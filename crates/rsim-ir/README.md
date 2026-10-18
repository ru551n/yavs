# rsim-ir

The register IR (docs/design/ir.md section 1) and, in `crates/rsim-interp`, its interpreter.
For producers (`rsim-lower`, `rsim-elab`): build with `ProgramBuilder` / `FuncBuilder`,
`finish()` propagates `MAY_SUSPEND` / `TOUCHES_SHARED`, numbers resume points and runs the
verifier; its errors are ICEs of the producer. `Program`'s `Display` and `dump_func` give a
textual dump.

## Stable (build on it)

- Ids: 32-bit newtypes (`FuncId`, `BlockId`, `TyId`, `ConstId`, `NativeId`, `ForeignId`,
  `CheckMsgId`, `CaseTableId`, `CovItem`) in `IdVec`; `StmtId` is `rsim_kernel::StmtId`.
- `Program`, `Func` (kind, parent for static links, params as frame slots, locals, root map,
  register types, blocks over `code`, `loc` = one `StmtId` per instruction), `StmtInfo`
  (span, func, kind, parent).
- Value model: `Ty` (one description per type; constraints are runtime descriptors, static
  ones are pool constants), `Ty::layout` / `desc_size`, the descriptor format (`[left, right,
  dir]` i64 per dimension), bit-plane logic arrays, `ConstData` compact constants.
- Calling convention: arguments are register cells stored into the callee's parameter slots
  (a signal parameter is two cells); composites are passed by address; a function result is
  one cell, a string/unconstrained array result is the address of a fat pair `[data, dope]`.
- Natives are `lib/NATIVE.md` symbols (`NativeRef`), implicit families carry their `TyId`;
  `rsim.attr.image` is the `'image` attribute. Foreign calls are `ForeignRef` (the elaborator
  binds the function pointer through `rsim_interp::Bindings`).
- Interpreter API: `Runtime::new(Arc<Program>, Bindings)`, `InterpProcess::new(rt, func,
  Arc<InstData>, Vec<ProcDriver>, shared)`, `proc_flags`, `call` (elaboration-time
  evaluation), `DebugSession`.

## Still moving

- Exact instruction operand shapes for records with runtime layouts, protected types,
  files and `Elab*` instructions (their semantics belong to `rsim-elab`).
- `InstData` gains child instances and package links when elaboration needs them.
- Foreign arrays/records/access marshalling (scalars and scalar `out` work).

## Requests

From `rsim-lower` (2026-10-18), found running `tests/corpus/{scalar,arith,process,report}`
through lowering and the interpreter (`crates/rsim-lower/tests/corpus.rs`):

1. **Thunks with a value.** The verifier rejects `Ret { v: Some(_) }` in a `FuncKind::Thunk`,
   but a thunk evaluates an expression. Lowering uses parameterless `Function`s until thunks may
   return their cell.
2. **`rsim.attr.value`** is accepted by `Runtime::new` but not implemented (`scalar/*_attrs`).
   Lowering passes the string as the address of a fat pair `[data, dope]`, `NativeRef::ty` = the
   prefix's base type.
3. **Report at the stop severity.** A `failure` report is committed but the process keeps
   running to its next suspension, so `report/failure_exit` prints a report NVC never reaches.
4. **NVC-compatible check texts** (difftest is strict for rsim): range checks read
   `value 101 outside of PCT range 0 to 100 for variable P` (subtype name, then the object),
   real to integer `value 3e10 outside of INTEGER range ...`, integer overflow
   `result of 2147483647 + 1 cannot be represented as INTEGER`, division `division by zero`.
   The overflow text needs the operands and the type in the instruction: a checked `Bin` with a
   result range (or a `CheckKind::Overflow` taking `a`, `b` and the op) would do. Lowering emits
   `Bin` then `Check { Range }` against the base type today.
5. **Range checks on reals**: `Check` compares i64 only, so real subtype constraints are not
   checked yet.
6. **`to_ostring`/`to_hstring` symbols**: sem names them `rsim.implicit.to_hstring.<type>`,
   NATIVE.md lists them under the `to_string` family, so `Runtime::new` rejects the program
   (`scalar/string_attrs`). One of the two needs to change (sem's `mangle.rs` or NATIVE.md).
