# AGENTS.md: yavs-lower

Read the root `AGENTS.md` first. This file is for an agent about to change `crates/yavs-lower`.
The design, the module ownership table, the HIR to IR mapping table, the contract with
yavs-interp and yavs-elab and the Requests are in `crates/yavs-lower/DESIGN.md`; read it.

## 1. Purpose and boundaries

- Lowers yavs-sem's typed HIR to one `yavs_ir::Program`: one `Func` per process, subprogram body
  and elaboration body **per design unit** (shared by every instance, ir.md 2), plus a `UnitIr`
  per architecture (instance values, constants, signals, processes, driver slots) for the
  elaborator. Subprograms are lowered on first reference, so unused std/ieee bodies are not.
- Does not: own IR types (it only uses `yavs_ir::ProgramBuilder`/`FuncBuilder` and the
  verifier), analyze (input must be an error-free `Db`), fold generics or build the hierarchy
  (`yavs-elab`), or execute (`yavs-interp`).
- Depends on: `yavs-diag`, `yavs-ffi` (C type mapping), `yavs-syntax`, `yavs-sem`, `yavs-ir`, `yavs-kernel` (dev: `yavs-elab`,
  `yavs-interp`, `yavs-logic`). Called by: `yavs-elab` (`Elaborated::lower`).

## 2. Map

| File | Responsibility |
|---|---|
| `src/lib.rs` | `Lowerer` (`new`, `architecture`, `package`, `package_value`, `var_slots` (the value type of each non-protected shared variable and, for a package's, a thunk returning its slot's address, #309), `implicit_native`, `subprogram`, `subprograms_of`, `finish -> Lowered`; `snapshot` (the program so far, lowering goes on) and `suspend`/`resume` (`Suspended`: the lowering without its `Db`, #383)), `LowerOptions` (`coverage`), `UnitIr`, `InstValue::write_value`, `InstConst`, `SignalIr`, `ProcessIr`, `Cx` (`error`, `unsupported`, `ice`, `cover`), `Obj`, `Space`, `top_architecture`, codes |
| `src/func.rs` | `Fx`: frames, parameters, declarations, subprograms, processes, architectures |
| `src/expr.rs` | Scalar expressions, calls, implicit operations, attributes, names, 1-D arrays |
| `src/stmt.rs` | Sequential statements, waits, signal assignments, case |
| `src/assoc.rs` | Individual association and conversions of subprogram parameters: temporary formal, copy-in, copy-out (`writeback_parts`), formal bounds from the parts |
| `src/ty.rs` | HIR type to IR `Ty`, widths, `signal_elem` |
| `src/composite.rs`, `access.rs`, `protected.rs`, `file.rs`, `force.rs` | Records and composite arrays, access types, protected types, files/textio, force/release (lower2) |
| `src/reflect.rs` | `'reflect` and mirror lowering |
| `src/foreign.rs` | Calls of `'FOREIGN` (VHPIDIRECT and `"VHPI library model"`, `ForeignRef::model`) subprograms: one `ForeignRef` per subprogram (`Cx::foreigns`), arguments as `cells(.., native)`, result range check |
| `tests/corpus.rs` | `tests/corpus` groups through sem, elab (lowering), verifier, interpreter and kernel vs the reference simulator `.expected` (honours `ignore=order` and `ignore=path`); `PASS` list must match exactly |

## 3. Design decisions that bind this module

| Decision | Why | Source |
|---|---|---|
| An alias of a logic element or slice of a variable with a static name has no storage: uses lower the aliased name (reads and writes), seen with the alias subtype's bounds; a non-static name stays `E2001` | Logic arrays are bit planes, so the part is not addressable; a static name denotes the same part at every use (#296) | LRM 6.6.2 |
| A `Failed` that reaches a `Lowerer` entry point with no diagnostic is an ICE (`Cx::take_failed`), never an empty error list | A failed run must always say why (#292) | root `AGENTS.md` rule 1 |
| One code per design unit; generics, signals and non-locally-static constants are instance data (`Space::Inst`), never folded into code | Shared code, rule 4 identity | ir.md 2, DESIGN.md "Shape" |
| Objects live in frame slots; registers are per-statement temporaries; nothing live across `Suspend` | Simple verifier, re-entrant waits | DESIGN.md "Registers, slots, temporaries", ir.md 1.2 |
| Composite temporaries are `Alloc`ed and live to the frame's next statement | Results survive return without copies | `docs/decisions.md` "IR and interpreter (M2)" |
| Every array subtype gets a runtime descriptor (pool constant when static, `MakeDope` otherwise), one format | Rule 1 | `docs/plan.md` rule 1, DESIGN.md "Values and descriptors" |
| Logic arrays are bit-planes; element `i` from the left is plane element `n - 1 - i`; BIT/BOOLEAN use std_ulogic codes | Shared plane ops; one sub-element map | `docs/decisions.md` "Elaboration", "Interpreter: BIT/BOOLEAN codes" |
| Checked arithmetic (`BinRange`, `Check{Range}`), reference-simulator-compatible check texts | Rule 7, strict difftest | `docs/plan.md` rule 7, `crates/yavs-ir/README.md` Requests |
| Compact constants (`ConstData` runs/logic) | Rule 12 | `docs/plan.md` rule 12 |
| Every instruction carries the current `StmtId`; a `Stmt{id}` marker opens each statement | Debug hooks, coverage, traces | `docs/plan.md` "Designed for the GUI", DESIGN.md "Shape" |
| Coverage counters only with `LowerOptions::coverage`: a statement counter per `stmt_start` except `Decl`/`Process`/`Subprogram`/`Implicit` markers, branch counters `CovKind::Branch(arm)` per arm in source order (`Fx::cover_branch` with the statement captured before the arms lower nested statements; an implicit `else` arm for `if` and conditional assignments without one; loop body arm 0); none in std/ieee files (`Cx::cov_skip`) or the `_actual`/`_guard` processes (`Fx::cov`) | Default code has no counters; users want their own code's coverage | DESIGN.md "Shape", rule 10, `docs/decisions.md` "Code coverage" |
| Unlowered constructs are `E2001` naming the construct; broken HIR invariants `E9999`; any error means no program | Error motto | crate docs, DESIGN.md |
| `force` default mode `out` for (a part of) an out/inout/buffer port, `in` otherwise; a composite target forces its kernel range, one with records each leaf's range (`force_leaves`), with the value checked against the target's bounds first | LRM 10.5.2.1, #68 | DESIGN.md "Composites", lrm-checks L97 |
| Instance data per region (instance, block, generate iteration); values written by the elaborator (`write_value`, `write_string`), thunks only for unfoldable scalar constants | Generate parameters, generics per instance, shared code | `docs/decisions.md` "Lowering" |
| One driver slot per `ProcessInfo::drivers` entry, in order | yavs-elab maps slots to LSP ranges | `docs/decisions.md` "Lowering" |
| A subprogram declared in a process drives through that process's driver slots | LRM 14.7.2: the process owns those drivers, #206 | DESIGN.md "Contract with yavs-interp and yavs-elab" |
| Signals with records in their type: one instance signal per field path (struct of arrays); names select one kernel range per leaf by the `NetGraph::select` rule, at run time, from the whole signal's descriptor slot the elaborator writes; reads read the whole signal | Rule 2, one sub-element map; rule 1 | `docs/decisions.md` "Lowering", DESIGN.md "Composites", #200, #215 |
| Port expression actuals the elaborator asks for (`Lowerer::port_actual`, by instance statement) are `ProcessIr`s with `actual: Some(PortActual)`: they drive slot 0 with `target: None` (a type with records: slot `k` per leaf `k`), arrays checked against `PortActual::dope`, a scalar with a non-static range against `PortActual::bounds`; the scalar check of a part (#263) names no port; an actual of a binding indication makes the component's ports and generics objects of the unit | #74, LRM 6.5.6.3 | DESIGN.md "Requests from yavs-elab" 7 |
| Signal parameters with records in their type: one reference per leaf of the formal, then the first leaf's driver slot (`out`/`inout`), then the actual's descriptor (`RecParam`); package signals with records: one package signal per leaf; attributes of parts over several leaves combine the leaves' | Rule 2, one sub-element map; the interpreter's two cells per signal parameter stay | `docs/decisions.md` "Lowering", DESIGN.md "Contract with yavs-interp and yavs-elab", #215 |
| Signal parameters with a mode view: each leaf the view drives (`out`, `inout`, `buffer`) is a one-part `SigParts` reference with the caller's driver slot of that leaf (sem's `view_driven_paths` drivers), slot cell `-1`; other leaves are plain references; the same per formal leaf when the formal is associated individually (`view_leaf_slot`) | The process drives only those leaves, whose slots need not be consecutive | `docs/decisions.md` "Lowering", #254, #265 |
| A part of a composite signal as an `out` signal actual: its prefix's driver slot plus its first leaf's position below the prefix (`leaf_slot`) | One driver slot per leaf below a prefix | `docs/decisions.md` "Lowering", #215 |
| Implicit signals named in a process body or in a subprogram declared in it (sem declares them in the process's scope) are instance signals of the region enclosing the process (`region`), skipped by `Fx::decls`; yavs-elab registers them under that region | LRM 16.2.4: the implicit signal is declared in the enclosing region, #240 | `docs/decisions.md` "Lowering" |
| Guarded blocks: GUARD's default is the condition; a `_guard` process drives GUARD | LRM 11.2, 11.6 | `docs/decisions.md` "Lowering" |
| Signals of arrays of arrays without records: one instance signal of all scalars; every index/slice name selects one range by the same walk (`sig_steps`) | Rule 2, one sub-element map | `docs/decisions.md` "Lowering", #89 |
| 2008 array-valued aggregate pieces (1-D only): positional ones concatenated, bounds as for `&`; under a range choice they fill the range in the aggregate's order, which without a constrained context is the choices' direction (sem, `E1334` for a mix); open element subtypes from the context or the first value; as targets each piece takes its target's length, cover checked first; a named target runs over its choices in the direction of its first range choice, else of the index subtype, leftmost index to the value's leftmost element (`named_target_positions`, #244, #264); a lone non-static element choice covers one element (`lone_choice`); every target covers its value exactly, checked first (`cover_dope`) | LRM 9.3.3.3, 10.6.2.1 | `docs/decisions.md` "Lowering", #89, #230 |
| `&` on arrays of composites: rows through `put_pieces`, bounds from the index subtype; open element subtypes from the left operand, every operand checked | LRM 9.2.5 | `docs/decisions.md` "Lowering", #234 |
| Aggregate targets are lowered by one walk (`stmt.rs` `walk_agg`): a record value hands each element its field (by name or position), an array of composites each element its element, a matrix each sub-aggregate its row; aggregate elements are walked on, other elements and 1-D scalar-array aggregates go to the variable or signal callback (`assign_aggregate`, `drive_aggregate`) | #350, `docs/design/lrm-checks.md` L98 | `composite/aggregate_target_*` |
| Named aggregate bounds: sem's result subtype (direction of a constrained context) | LRM 9.3.3.3 | `docs/decisions.md` "Lowering", #230 |
| An object alias of a composite variable part stores the target's whole descriptor (`desc_size`, record field and element bounds included), checked against the alias subtype with `check_desc` | A record or element bound read through the alias must be the object's (#315) | LRM 6.6.2 |
| Signal aliases have no storage: uses lower the aliased name with the alias subtype's descriptor; `LspStep::Rebase` steps are walked by `walk_steps` (`Step::Rebase`) | LRM 6.6.2, #203, #230 | `docs/decisions.md` "Lowering" |
| An alias of a whole external name (declared, or implicit for a direct use: `Db::external_uses`) is an object of its region bound by the elaborator: a signal an instance signal (one per leaf with records, descriptor slot when the type needs one), a constant an `InstValue`, a protected shared variable a cell in `UnitIr::ext_vars`; LSPs rooted at the external name map to it (`Cx::root_decl`); a subprogram's aliases (`ExtOwner::Sub`) are objects of the region declaring it, lowered with that region, or of its package (`pkg_external`: package signals, cells in the shared variables area for constants and variables, `PackageIr::ext_consts`/`ext_vars`); a subprogram with an unbound one (`Lowerer::externals_bound`, all of them before elaboration ends) fails on entry; `ExprKind::External` is left only where elab rejects it first (`E2001`) | LRM 8.7, #19, #93, #255 | `docs/decisions.md` "Elaboration" |
| Non-protected shared variables (`--relaxed`): the slot of a process variable (`VarShape`, `Fx::var_init`) with heap objects in the shared heap; a package's in the shared variables area (`Space::Shared`), a region's behind an instance cell its thunk writes (`Space::Region`, one per declaring region); functions naming one are `TOUCHES_SHARED`; each is listed in `Cx::shared_vars` with its value's IR type for `Lowerer::var_slots` (VHPI, #309); types holding access values give the GC their handle words (`Program::shared_roots`, typed region slots, #303) | Serial processes keep `--threads N` identical (plan "Parallel kernel design"); their objects live in the shared heap, collected between cycles | `docs/decisions.md` "Lowering", DESIGN.md "Composites", #285 |
| Matching case compares with `?=` on BIT/STD_ULOGIC elements | LRM 10.9, 9.2.3 | `docs/decisions.md` "Lowering" |
| Foreign calls: symbol is the last word after `VHPIDIRECT` (else the name), C types from the formal's own subtype, result range-checked; signal and file parameters, composite results are `E2001` (a VHPI model may return a one-dimensional array of scalars, `VhpiModel::ret_index`, and take records of scalars and constrained arrays of scalars, `in` access values and signal parameters of scalars or arrays of scalars, `ForeignArg::Signal` with its cells, #330) | Reference simulator conventions, fail early | `docs/decisions.md` "Foreign subprograms and `--load`" |
| Converted parameters (`Actual::Converted` of a call): a temporary formal holding sem's `copy_in` (or the subtype's default for `out`), `copy_out` stored to the actual after the call with the formal bound to the temporary | LRM 6.5.7.1; sem builds the conversions as HIR expressions (#229) | DESIGN.md "Contract with yavs-interp and yavs-elab", `docs/decisions.md` "Lowering" |
| Individually associated parameters: a temporary formal of variable or constant class, parts copied in for every mode and back for `out`/`inout`; a signal formal of several parts is a reference of several parts (`SigParts`, one per leaf for a record formal), each part's offset found by lowering the formal part against the formal bound to a reference from element 0; unconstrained bounds lowest to highest index named, index subtype direction | LRM 6.5.7.1; one sub-element walk (rule 2); same bounds rule as elab's ports | DESIGN.md "Contract with yavs-interp and yavs-elab", ir.md 1.8, #71 |
| A 2019 variable port is an 8-byte instance cell (`Obj::Scalar`, `UnitIr::var_ports`) holding the handle of its actual's protected object, written by the elaborator; calls through it are ordinary protected calls | #188, LRM 6.5.2 | `docs/decisions.md` "Elaboration" |
| Protected state initial values run on the new object (`self` and the state bound), except those giving a variable its constraint, which run before it exists | LRM 5.6.2: methods called from initial values act on the object being elaborated (#204) | DESIGN.md "Composites" |
| Objects of a package declared in a declarative part (#34) are objects of the declaring region too, through `Db::decls_with_instances`; a generic one has none | LRM 4.7, one storage scheme per region kind | `docs/decisions.md` "Elaboration" |
| Objects of a package instance are objects of the region declaring the instance: `Db::decls_with_instances` in architectures, blocks, generates, frames and packages; a design unit instance is lowered by `Lowerer::package` like a package (`Db::package_regions`), an uninstantiated generic package never | LRM 4.9, 14.4.2; one storage scheme per region kind (#101) | `docs/decisions.md` "Elaboration" |
| A protected subelement of a composite gets a new object wherever a default value is built (`default_into`, `default_array`: variables, frames, allocators, shared variables); in a protected state `prot_new` lays the state out with null handles (`Fx::prot_defaults` off) and `prot_init_late` creates them in declaration order | 2019 composites of protected types (#361) | lrm-checks L102 |
| A subprogram nested in a protected method takes the method's `self` through the static link and binds the state in its own frame | LRM 5.6.2: calls and names in it act on the enclosing method's object (#221) | DESIGN.md "Composites" |

Module-local decisions go in this section and DESIGN.md in the same commit (root `AGENTS.md`
rule 24).

## 4. Invariants and pitfalls

- Lowering a unit with errors is an ICE: callers must stop after sem errors.
- Every function goes through the yavs-ir verifier; a verifier error is an ICE of this crate,
  never "fixed" by relaxing the verifier.
- The calling and native conventions (fat pair `[data, dope]`, cells per parameter, signal
  parameter = two cells) are shared with `yavs-interp`: change both, note it in DESIGN.md and
  `crates/yavs-ir/README.md`.
- Kernel offsets of non-static indices must use the same rule as `yavs_elab::NetGraph::select`.
- `tests/corpus.rs` writes its library to a fixed `temp_dir()/yavs-lower-work`, which collides
  between worktrees and parallel runs (#160); until fixed, do not run two of them at once.
- A folded logic-array constant of arrays of unconstrained arrays (`Fx::encode`/`codes`) must carry the descriptor of every level, not only the first element level (#329).
- A shared `CARGO_TARGET_DIR` can link another worktree's crates into the test binary (#7).

## 5. How to work here

```sh
CARGO_BUILD_JOBS=6 cargo test -p yavs-lower                     # unit tests and the corpus PASS list
cargo test -p yavs-lower --test corpus -- --nocapture           # print the non-passing files with their diff
python3 tools/difftest/difftest.py --candidate yavs             # full differential run (tools/difftest)
tools/difftest/report.sh                                        # rewrite tests/corpus/RESULTS.md
```

- A newly passing corpus file goes into `PASS` in `tests/corpus.rs` in the same commit; refresh
  `tests/corpus/RESULTS.md` with `tools/difftest/report.sh`.
- Codes owned: `E2000`-`E2099` (in use: `E2001` not lowered yet), plus `E9999`.

## 6. Coordination

- Owners per module: DESIGN.md "Modules and owners" (lower: core; lower2: composites). Replace an
  `E2001` arm in another owner's file with a call into your module in a small commit.
- Requests from yavs-elab and their answers: DESIGN.md "Requests from yavs-elab"; requests to the
  IR/interpreter: `crates/yavs-ir/README.md` "Requests".
- Open issues (snapshot 2026-10-18, `gh issue list -R ru551n/yavs --label crate:lower`): #64 wire
  lowering into `yavs run`; #67 array signals, sub-elements, aggregate targets, signal out
  parameters; #68 guarded assignments, null waveforms, force/release, implicit signal
  attributes; #69 matching case, case over arrays, non-static choices; #70 expression gaps; #71
  generic and instantiated subprograms (foreign, open actuals, individual association, signal formals with several parts, conversions in calls #229: done); #72 remaining attributes; #89 composite arrays and aggregates; #93 external names;
  #160 fixed scratch dir in `tests/corpus.rs`. Older unlabeled: #5, #8,
  #9, #13 to #17.
