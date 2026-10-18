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
