# M1 design: semantic analysis (`yavs-sem`)

Status: design, binding for M1 once the user has answered the open questions in section 11.
Inputs: `docs/plan.md` (rules 1, 2, 4, 7, 8, 12, 13, the error-handling motto, 2008+2019
together, on-demand analysis), `docs/decisions.md`, `lib/README.md`, `lib/NATIVE.md`,
`crates/yavs-diag`, and the M0 AST (`yavs-syntax`, `ast.rs` on the M0 branch).

LRM references are to IEEE 1076-2008 clause numbers, which 1076-2019 mostly keeps. The former
**(unsure)** items and Q1/Q2 were checked on 2026-10-18; `docs/design/lrm-checks.md` has the
evidence per item (tagged `[L<n>]`, `[Q<n>]` here) and the user decisions (`U-<n>`, section 11). Nothing
here is taken from the code of the reference simulators or of other VHDL front ends.

## LRM interpretation policy (user, 2026-10-18)

- **LRM clear:** follow it and fail early, even where a simulator is more lenient.
- **LRM unclear or silent:** be permissive. Accept the union of what the reference simulators and
  A commercial simulator accept, and pick the semantics that let the most designs elaborate.
- **Warn only when simulators give different results** for code yavs accepts (for example
  W1506, U-6), never just because a rule is unclear.

Each ruling is in `lrm-checks.md` (verdict table) and section 11 (U-1 to U-6, L8).

## 0. Scope and shape

`yavs-sem` turns M0 ASTs into a **typed HIR**: every name is resolved to a declaration id, every
expression carries a subtype and a staticness, every call names its overload, every statement has
an id. It runs single-threaded. It owns no files on disk: `yavs-lib` gives it a `LibrarySet`
(library name to manifest), and it asks `yavs-syntax` to parse files on demand.

| In scope for M1 | Not in M1 |
|---|---|
| libraries, units, context clauses, visibility, overloading | elaboration, generic folding through the interpreter (M2) |
| the full type and constraint model (rule 1) | building driver/source nets (M2/M3, from the LSPs sem computes) |
| expression typing, aggregates, literals, attributes | default binding of component instances (sem provides the rule, elab calls it) |
| static classification and folding of locally static expressions | external name binding (elab, rule 4) |
| generic packages, types, subprograms, instantiation by HIR cloning | code generation, mangled symbol emission (M2 uses sem's names) |
| configurations (checked), protected types, views, reflection types | |
| diagnostics with stable codes, `--relaxed` | |

Language revision: see 0.1. The analyzer is one code path with revision gates.

### 0.1 Mixed 1993, 2008 and 2019 files (user decision, `docs/plan.md` "Test-runner integration"; 1993 added by #629)

A project may mix 1993, 2008 and 2019 files; `yavs-lib` records the standard per file.
`Vhdl1993 < Vhdl2008 < Vhdl2019`. The 1993 rules and what a 93 file sees in a 2008 design are in `docs/design/vhdl-93.md`;
the baseline of `since` is `Vhdl1993`, and the implicit operations new in 2008 are `since: Vhdl2008`.

| Step | Revision used |
|---|---|
| parsing | each file with **its own** recorded revision (keywords, grammar, `` `if `` tool directives) |
| sem and elab of a design | **one** design revision `D`: the newest revision recorded for any file in the design's closure; std and ieee are loaded from `lib/{std,ieee}/<D>`, so INTEGER, STRING, `std_ulogic` etc. are one type across the whole design |
| revision gates in sem | a construct is checked against the file's own revision `F` (is it legal syntax and semantics for `F`) **and** against `D` (does it mean the same) |

The closure is found before analysis: parse the top's file, scan its ASTs for unit references
(context clauses, `lib.unit` selected names, entity/configuration instantiations, component names
for default binding) and follow them through the manifest, parsing only. Then `D` is fixed and
analysis starts. `yavs analyze f.vhd` uses the same rule over `f`'s closure. A 2008 unit used in a
2019 design is analyzed against the 2019 std/ieee.

Every declaration in std and ieee carries `since: LangRev`, computed by comparing the 2008 and
2019 sources (a declaration with the same designator and profile in both is `V2008`). A reference
from a file with `F < since` is a revision error, which covers new subprograms, new overloads that
would change overload resolution, and new types.

Known 2008-vs-2019 differences and their handling:

| Difference | Handling |
|---|---|
| INTEGER is 32-bit in 2008 (`-2147483648 to 2147483647`), 64-bit in 2019 (`-(2**63-1) to 2**63-1`); REAL is f64 in both yavs copies | design-wide: a 2008 unit in a 2019 design gets 64-bit INTEGER. Uses whose value depends on the range in a 2008 file (`integer'high`, `'low`, `'left`, `'right`, `'range`, `'base` of INTEGER and its subtypes NATURAL/POSITIVE, and of `universal_integer`-typed static bounds that fold differently) get warning `W1008`; plain arithmetic is not diagnosed, since its results agree whenever the 2008 run would not overflow |
| implicit operations new in 2019 (file `file_state`, `file_mode`, `file_position`, `file_size`, `file_seek`, `file_truncate`, `file_rewind`; range record types; anything else NATIVE.md section 2 marks 2019) | created with `since: V2019`; a reference from a 2008 file is `E1007`; if the 2008 file declares its own homograph, its explicit declaration hides the implicit one in the same region as usual, and a use-visible user homograph that now meets a directly visible implicit 2019 op gets warning `W1008` instead of a silent change of the chosen overload |
| std/ieee declarations new in 2019 (`std.env` additions, `std.reflection`, new ieee overloads) | `since: V2019`, `E1007` from a 2008 file |
| reserved words new in 2019: exactly `view` and `private` (15.10) [L1] | a 2008 file may declare and use them as identifiers (M0 lexes per file); a 2019 file cannot name such a declaration (the word is a keyword there, and the extended identifier `\view\` is a different name), so a 2008 declaration named `view` or `private` that is reachable from a 2019 file through `use ... .all` is invisible there; a 2019 reference that fails only because of this gets the usual `E1100` with a help naming the 2008 declaration |
| 2019-only constructs (views, private variables, generic type classes, 2019 attributes, conditional expressions) in a 2008 file | `E1910` (normally already a syntax error from M0) |
| 2008 constructs illegal under 2019 rules | none known; if one is found it is `E1007` |

`since` is set when a std/ieee package is loaded under a 2019 design (`src/rev.rs`): an item of
the 2019 package whose kind, designator and (for subprograms) profile type marks have no match in
the 2008 copy is `V2019`, and so is a whole unit without a 2008 copy. Declarations newer than the
file are left out of potential visibility for that file, so a 2008 file keeps its 2008 overload
sets and conflicts; when nothing else is visible the name is `E1007`. `W1008` for INTEGER is given
for `'left`, `'right`, `'high` and `'low` whose value lies outside the 32-bit range (L52).

`E1007`: "`<name>` is not available in VHDL-2008", primary at the reference, note "file `<path>`
is analyzed as VHDL-2008 (`--std=2008` in the manifest); the design is elaborated with the
VHDL-2019 std/ieee because `<unit>` (`<path>`) is VHDL-2019". `E1007` is an error (fail early).
`W1008` is a **warning** (user decision 2026-10-18): same notes, message "`<construct>` means
something different under VHDL-2019"; reported once per construct (per source span), analysis
continues, and `-Werror=W1008` escalates it. The same two codes apply the other way round when
`--std=2008` on `run`/`elab` forces a 2008 design revision over 2019 files. Neither is affected by
`--relaxed`.

`docs/cli-contract.md` section 2 states the same rule for `run`/`elab`.

Crate layout (files, not crates):

| Module | Content |
|---|---|
| `db.rs` | `Db`: arenas, libraries, the on-demand unit loader |
| `ids.rs` | typed ids (one macro) |
| `scope.rs` | declarative regions, visibility, lookup |
| `types.rs` | types, subtypes, constraints, resolution, implicit operations |
| `expr/` | `cands.rs` (bottom-up candidate sets), `resolve.rs` (top-down selection), `names.rs` (Apply/Selected interpretation), `aggregate.rs`, `literal.rs` |
| `stat.rs` | staticness and the folder, `ConstValue` |
| `generic.rs` | instantiation and HIR cloning |
| `decl.rs`, `stmt.rs`, `unit.rs` | declarations, statements, design units, configurations |
| `lsp.rs` | longest static prefixes, drivers, `all` sensitivity |
| `hir.rs` | HIR node definitions plus `validate()` |
| `diag.rs` | code constants, suggestion helpers |

## 1. Data model

### 1.1 Ids and arenas

All ids are 32-bit indexes into arenas owned by one `Db` per session. `Option<Id>` is free
(`NonZeroU32` inside).

```rust
id!(LibId, UnitId, DeclId, ScopeId, TypeId, SubtypeId, ExprId, StmtId, ConstId, InstId, ViewId);

pub struct Db {
    pub rev: LangRev,
    pub libs: Arena<LibId, Library>,
    pub units: Arena<UnitId, Unit>,
    pub decls: Arena<DeclId, Decl>,
    pub scopes: Arena<ScopeId, Scope>,
    pub types: Arena<TypeId, TypeDef>,
    pub subtypes: Arena<SubtypeId, SubtypeDef>,
    pub exprs: Arena<ExprId, Expr>,
    pub stmts: Arena<StmtId, Stmt>,
    pub consts: Arena<ConstId, ConstValue>,
    pub insts: Arena<InstId, Instance>,
    pub well_known: WellKnown,      // BOOLEAN, BIT, CHARACTER, STRING, INTEGER, NATURAL, REAL, TIME,
                                    // SEVERITY_LEVEL, FILE_OPEN_KIND, ..., universal_integer/real
    diags: Vec<Diagnostic>,
}
```

Ids are deterministic for a given input (same manifest, same top, same files), because analysis
order is deterministic. They are **not** stable across different inputs; anything that must
survive across runs (coverage, rule 10) uses hierarchical path plus span hash, never an id.

The AST of a unit is dropped after the unit is analyzed; the HIR keeps spans and `Ident`s
(`text` for original spelling, `key` for lookup, rule 8).

### 1.2 Libraries and the unit graph

```rust
pub struct Library { name: Ident, manifest: Manifest, units: HashMap<UnitKey, UnitSlot> }
pub enum UnitKey { Primary(Symbol), Secondary(Symbol /*primary*/, Symbol /*arch or "body"*/) }
pub enum UnitSlot { NotLoaded { file: FileRef }, InProgress, Done(UnitId), Failed }

pub struct Unit {
    lib: LibId, key: UnitKey, kind: UnitKind, decl: DeclId, scope: ScopeId,
    context: ScopeId,                 // the context clause region (library/use/context items)
    deps: Vec<UnitId>,                // units named by its context clause or by selected names
    file: FileId, span: Span,
}
pub enum UnitKind { Entity, Architecture, Package, PackageBody, PackageInst, Context,
                    Configuration, SubprogramUnit /*2008 subprogram as primary*/, SubprogramInst }
```

On-demand loading (analyze only what is reached; the result of a whole closure can be kept as an image, `image.rs`, `docs/decisions.md` "Compilation cache"):
- `Db::unit(lib, key)` returns `Done` at once, otherwise looks the key up in the manifest, parses
  the whole file (all units, cheap) and analyzes **only** the requested unit. Other units of the
  same file stay `NotLoaded` and keep their parsed AST in a per-file cache until the session ends
  or they are analyzed.
- `InProgress` on entry means a dependency cycle: `E1003` with the chain as secondary labels.
- `Failed` units poison: a reference to one gives no further diagnostic (the unit's own errors
  were already reported).
- `yavs analyze --work L f.vhd` calls `analyze_file`, which analyzes every unit in the file in
  textual order; a later unit in the same file sees earlier ones through `work`.
- A secondary unit loads its primary first. A package body is loaded only when someone asks for
  it (elab). Deferred constant completion is checked when the body is analyzed.
- std and ieee come from the `lib/` tree; `lib/ORDER.md` is not needed for on-demand loading,
  only for the corpus test that analyzes in order.

### 1.3 Declarative regions and declarations

```rust
pub struct Scope {
    kind: ScopeKind,                 // Unit, Package, Body, Subprogram, Process, Block, Generate,
                                     // Loop, Record, Protected, ProtectedBody, Instance, Context
    parent: Option<ScopeId>,         // lexical parent; a package body's parent is its package
    owner: DeclId,                   // for expanded names (`proc_label.v`, `pkg.x`)
    names: HashMap<Symbol /*key*/, SmallVec<[DeclId; 2]>>,
    order: Vec<DeclId>,              // declaration order, for cloning and for HIR consumers
    uses: Vec<UseItem>,              // use clauses in this region, in order
    private_from: Option<usize>,     // 2019: index into `order` where `private variable`s start (5.6.2)
}
pub enum UseItem { All(ScopeId), One(SmallVec<[DeclId; 2]>), Library(LibId) }

pub struct Decl {
    pub name: Designator,            // Ident | Operator(Symbol) | Char(char), original spelling kept
    pub kind: DeclKind,
    pub scope: ScopeId,
    pub span: Span,
    pub implicit: bool,              // implicitly declared (predefined ops, range records, ...)
}
pub enum DeclKind {
    Library(LibId), Unit(UnitId),
    Type(TypeId), Subtype(SubtypeId),
    Object(Object),                  // constant, signal, variable, shared variable, file, interface objects
    Subprogram(Subprogram),
    EnumLit { ty: TypeId, pos: u32 },
    PhysUnit { ty: TypeId, index: u32 },
    Alias(Alias),
    Attribute { ty: SubtypeId },     // attribute declaration
    Component(Component),
    View(ViewId),
    PackageInst(InstId),
    GenericType(TypeId), GenericSubprogram(Subprogram), GenericPackage(InstId),
    Group { template: DeclId }, GroupTemplate,
    Label(StmtId),                   // statement labels are declarations (implicitly declared, 11.x)
    Element { record: TypeId, index: u32 }, // only reachable through selection, never in `names`
    Error,                           // poison, see 8.2
}
```

Analysis is sequential in textual order, so inserting declarations as they are analyzed gives the
LRM scope rule (12.2: scope starts at the declaration) without position bookkeeping. Completions
(subprogram body for a spec, deferred constant full declaration, incomplete type completion,
protected body) **reuse** the earlier `DeclId` and fill in the body; they never add a second entry.

### 1.4 Lookup and visibility (LRM 12.3, 12.4)

`lookup(scope, key) -> Visible` where

```rust
pub enum Visible { None, One(DeclId), Overloads(SmallVec<[DeclId; 4]>), Hidden(Conflict) }
```

Algorithm:
1. **Direct visibility.** Walk `scope` to the root. In each region take `names[key]`.
   - A non-overloadable hit (anything but subprograms and enumeration literals, and aliases of
     those) stops the walk if nothing overloadable was collected yet; if overloadables were
     collected it is hidden by them (12.3).
   - Overloadable hits accumulate; an outer one is dropped if an inner one is its **homograph**
     (same designator, and for overloadables the same parameter and result type profile, 4.5.3).
   - Inside one region, an explicit declaration hides an implicit homograph (12.3, 2008), so a
     user `"="` on a type hides the predefined one declared with the type.
2. **Potential visibility (use clauses).** Collect from `uses` of every enclosing region (and the
   unit's context region). `All(s)` contributes `s.names[key]`, `One` contributes its decls.
   Then filter (12.4):
   - (a) drop a potentially visible decl if a directly visible homograph exists;
   - (b) of two potentially visible homographs, if one is explicit and the other implicit, drop
     the implicit one (2008). An implicit subprogram or literal is dropped only for an explicit
     homograph **subprogram**, never for a non-overloadable: a constant and a subprogram of one
     designator are not homographs that hide each other here, they fall to (c) (#628, [L141]);
   - (c) two potentially visible non-overloadable decls with the same key, or a non-overloadable
     and an overloadable one, make **all** of them invisible unless they are the same declaration
     or non-object aliases (or the declaration itself) that denote the same named entity. Object
     aliases get no exception: `alias c is p1.c` against `p1.c` conflicts (12.4 c has no alias
     exception in its text; the reference simulators both treat type aliases as the same entity and object
     aliases as a conflict) [L2]. Report only at the use: `E1102` with both use clauses as labels.
   - Overloadable potentially visible decls from different packages that are not homographs, or
     are explicit homographs, stay visible; ambiguity is an overload resolution error at the call
     (the classic `numeric_std` + `std_logic_arith` clash).
3. Direct and potential results are merged: potential overloadables join direct overloadables.

Implicit context (13.2): every unit's context region starts with `library std, work;` and
`use std.standard.all;`. 2019 `std.reflection` is not used implicitly; the `'reflect` attribute
loads it on demand (6.3).

Context clauses and declarations (13.3, 13.4):
- `library l;` adds `Decl::Library` to the context region. Unknown library: `E1001`, help lists
  the libraries in the `LibrarySet` and the closest one.
- `use l.p.all`, `use l.p.x`, `use p.x` (p visible), `use inst.all` for a package instance
  (top-level `PackageInst` unit or a local one), and `use g.all` where `g` is an interface package
  (2008). The prefix of a use clause is resolved as an expanded name (1.5); the suffix must
  denote a package or an instance, or a declaration in one.
- `context l.c;` splices the items of context declaration `c` into the current context region,
  in order. Inside a context declaration, a library clause naming `work`, and a use clause or
  context reference whose selected name has the prefix `work`, are errors (13.3) [L3].
- `use` of a whole library (`use l.all`) makes its primary units potentially visible; entity
  names visible this way matter for default binding (7.3).

### 1.5 Selected and expanded names

`Selected { prefix, suffix }` is classified by what the prefix denotes:

| Prefix denotes | Meaning | Suffix |
|---|---|---|
| library | primary unit | identifier; loads the unit on demand |
| package, package instance, interface package | declaration in its visible part | identifier, operator, character, `all` (use clauses only) |
| an enclosing construct (process/block/generate label, subprogram, entity, architecture, loop label) | expanded name (8.3) of a declaration in that region | as above |
| object of record type | element | element name |
| object of protected type | method call (prefix becomes `self`) | method name |
| object of access type | implicit dereference then element/method, or `.all` | as above |
| function name callable without arguments | call, then select on the result (8.3) | as above |

Expanded names into a package see only its visible part; 2019 private variables of a protected
type are not selectable from outside the protected body (`E1105`).

### 1.6 Overloading

Overloadable entities: subprograms (explicit, implicit, instantiated, interface subprograms),
enumeration literals (identifier and character), and aliases of those. A name in an expression
context therefore yields a set (`Overloads`) that the expression resolver (section 3) reduces.
Non-overloadable names yield exactly one decl or a conflict.

Profile (4.5.3): the base types of the parameters in order, plus the base type of the result.
Parameter names, modes, classes and defaults are not part of it. Two subprograms in one region
with the same designator and profile are `E1101` (redeclaration), except explicit-hides-implicit.

## 2. Type system (rule 1)

### 2.1 Principle

**There is exactly one subtype representation, and every constraint in it is data**: ranges are
HIR expressions, not numbers. An expression that happens to be static also has a folded `ConstId`,
which is a cache on the expression, not another representation. M2 materializes a subtype
descriptor for every subtype it needs at runtime; a static subtype just gets a constant descriptor.
No code in sem or M2 may branch on "is this subtype static" to choose a layout or a code path;
staticness decides only *when* a value is known (analysis, elaboration, runtime) and which
diagnostics are possible early.

### 2.2 Types and subtypes

```rust
pub struct TypeDef {
    pub decl: DeclId,
    pub name: Ident,              // for an anonymous base type: the declared name, `anonymous = true`
    pub anonymous: bool,          // diagnostics print "the base type of T"
    pub kind: TypeKind,
    pub first_subtype: SubtypeId, // T in `type T is ...`
}
pub enum TypeKind {
    UniversalInteger, UniversalReal,
    Enum { lits: Box<[DeclId]> },                     // EnumLit decls, pos order
    Integer, Floating,                                // ranges live in subtypes
    Physical { units: Box<[PhysUnit]> },              // PhysUnit { decl, scale: i64 in base units }
    Array { indexes: Box<[SubtypeId]>, elem: SubtypeId }, // index subtypes (`natural range <>`)
    Record { elems: Box<[RecordElem]> },              // RecordElem { name: Ident, subtype: SubtypeId }
    Access { designated: SubtypeId },
    File { elem: SubtypeId },
    Protected { decl_scope: ScopeId, body: Option<ScopeId>, generics: Option<GenericList> },
    Incomplete { completion: Option<TypeId> },
    Generic { class: GenericClass },                  // opaque inside a generic unit, 2.9
    Error,
}

pub struct SubtypeDef {
    pub base: TypeId,
    pub name: Option<Ident>,          // None: anonymous subtype indication
    pub parent: Option<SubtypeId>,    // the type mark it was derived from (diagnostics, 'base)
    pub constraint: Constraint,       // the EFFECTIVE constraint, parent's merged in
    pub resolution: Option<Resolution>,
    pub span: Span,
}

pub enum Constraint {
    None,                             // scalars: base range; composites: nothing added at this level
    Range(RangeC),
    Array { dims: Option<Box<[RangeC]>> /* None = `open` or `<>` */, elem: Box<Constraint> },
    Record { elems: Box<[(u32, Constraint)]> },       // only the constrained elements
    FromObject { expr: ExprId, elem_depth: u8 },      // `x'subtype`, `x'element`: taken at runtime
}

pub enum RangeC {
    Explicit { left: ExprId, dir: Direction, right: ExprId },
    Attr { prefix: AttrPrefix, reverse: bool, dim: u32 },   // `a'range`, `t'reverse_range(2)`
}
```

Notes:
- **Constraint merging** (2008 5.3.2.2, 6.3): `subtype a is t(open)(7 downto 0)` then
  `subtype b is a(0 to 3)` gives `b.constraint = Array{dims: [0 to 3], elem: Range(7 downto 0)}`.
  Constraining an already constrained dimension is `E1210`. Merging is done once, at the subtype
  declaration; consumers never walk `parent` to find constraints.
- **Constrainedness** is computed, never stored: `fn constrainedness(st) -> Full | Partial | Unconstrained`
  (5.1: fully constrained means every index range and every element, recursively, is constrained).
- **Where an object's bounds come from**, stored on the object, so M2 never guesses:

```rust
pub enum BoundsFrom {
    Subtype,              // fully constrained subtype indication
    Initializer,          // constants (6.4.2.2) and allocators `new T'(x)`
    Actual,               // interface objects with not fully constrained subtypes
    Aliased,              // object alias without subtype indication
}
```
  Signals and variables need a fully constrained subtype (`E1212`); a constant may take missing
  bounds from its initializer (6.4.2.2). Partially constrained interface objects take the missing
  parts from the actual.

### 2.3 Scalar types

| Declaration | Entities created |
|---|---|
| `type E is (a, 'b')` | `TypeDef{Enum}`, first subtype `E` (Range over all literals), one `EnumLit` decl per literal |
| `type I is range L to R` | anonymous base `TypeDef{Integer}`, named first subtype `I` with `Range(L to R)` (5.2.3.1) |
| `type F is range L to R` (real bounds) | same with `Floating` |
| `type P is range L to R units u; v = n u; end units` | anonymous base `Physical{units}`, first subtype `P`, one `PhysUnit` decl per unit; scales folded to i64 base units (checked, `E1401` on overflow) |
| `subtype S is T range L to R` | `SubtypeDef{base, Range}` |

Bounds of an integer or physical type declaration must be locally static (5.2.3.1) and are folded.
The base range of an anonymous integer base type is implementation-defined (it must contain the
declared range); yavs takes the declared range and checks intermediate results against it, as
the reference simulator does (user decision U-1) [Q2].

### 2.4 Array types (5.3.2)

| Declaration | Entities |
|---|---|
| `type A is array (natural range <>) of T` | `TypeDef{Array}`, first subtype unconstrained |
| `type A is array (0 to 7) of T` | anonymous unconstrained base `Array` with index subtype = the base type of the range (`integer`), named first subtype `A` constrained `0 to 7` (5.3.2.1) |
| `type M is array (natural range <>) of std_logic_vector` (2008) | element subtype unconstrained; `M(0 to 3)(7 downto 0)` constrains both |
| `subtype S is A(open)(3 downto 0)` | partially constrained |

Index subtype of an index constraint given as a range with universal bounds: INTEGER (5.3.2.2
implicit conversion rule), otherwise the type of the bounds.

### 2.5 Record types (5.3.3)

Element subtypes may be unconstrained or partially constrained (2008). Record constraints name
elements: `subtype s is r_t(data(7 downto 0), keep(open))`. `Constraint::Record` stores only the
constrained elements; the rest inherit the element's declared subtype. Duplicate or unknown
element names in a record constraint: `E1213`.

### 2.6 Access, file, protected, incomplete

- Access (5.4): `designated` may be any subtype, including unconstrained (`type line is access string`).
  Allocators: `new S` needs `S` fully constrained (or a protected type), `new T'(x)` takes bounds
  from the qualified expression.
- A subtype indication of an access type may carry an index constraint when the designated type
  is an array (`arr(0 to 3)`): it is merged against the designated type and stored on the access
  subtype (`merge_constraint`), so the dimension checks and the "already constrained" error of a
  subtype of it run as for an array subtype; a range or record constraint stays `E1211` (#643,
  [L139]).
- The designated type of a 2008 access type cannot be a file or protected type (`E1221`); 2019
  allows both (permissive, [L19]).
- Dereference: `p.all`, and implicitly a selected, indexed or sliced name or an attribute whose
  prefix is an access value (8.1 "appropriate for"), are `Deref` with the designated subtype.
- Incomplete type `type T;` must be completed by a full declaration of `T` in the same region
  (`E1220` otherwise). Before completion it may be used only as the designated type of an access
  type, and, per `lib/README.md`, as a return type of methods in a protected type declaration
  whose completion appears later in the region (std 2019 `reflection` needs this). No LRM text
  for it was found; yavs accepts it in subprogram profiles inside a protected type declaration,
  in 2008 and 2019, but only in library `std` (user decision U-2, amended 2026-10-18); user code gets `E1220` with a help suggesting an access type [L4].
- File types (5.5): element type cannot be access, protected, file, or contain those (`E1222`);
  multi-dimensional arrays as file element are an error in 2008 and 2019 (`E1222`) [L5].
- Constants, signals, generics, ports and constant or signal parameters cannot be of an access
  or file type or contain one (6.4.2.2, 6.4.2.3, 6.5.2; `E1223`, #198); protected types are `E1905`.
- Protected types (5.6): see 6.6.

### 2.7 Resolution indications (6.3)

```rust
pub enum Resolution {
    Function(DeclId),                                 // `resolved std_ulogic`
    Element(Box<Resolution>),                         // `(resolved) std_ulogic_vector` (2008)
    Record(Box<[(u32, Resolution)]>),                 // `(a resolved_a, b resolved_b) rec_t`
}
```

A resolution function must be a pure function with one parameter of class constant whose type
is a one-dimensional unconstrained array of the resolved subtype's type, returning that type
(4.6). `Element` applies to the element subtype of an array; `Record` per element. The resolved
subtype keeps the resolution; `std_logic` is `SubtypeDef{base: std_ulogic, resolution: Function(resolved)}`.

### 2.8 Universal types and implicit conversion (5.2, 9.3.6)

- Literals: abstract integer literal is `universal_integer`, real literal `universal_real`.
  VHDL has no named numbers: `constant N := 8;` is a syntax error (6.4.2.2) [Q1]. Universal values
  come from literals, attributes such as `'pos` and `'length`, and universal operators.
- Predefined universal operators exist (`*` of universal_integer and universal_real, `/` real by
  int, `**`, arithmetic, relational), declared implicitly in `std.standard` like any type's ops.
- Implicit conversion (9.3.6) applies only to a *convertible universal operand* (a numeric
  literal, an attribute of universal type, or the division of two values of the same physical
  type), only where the innermost complete context determines a unique numeric target type, and
  only if no interpretation of the complete context without the conversion exists. So
  `(2 + 3) = 5` uses universal `+` and `=` even when a user `"+"` on INTEGER is visible, and
  `x := 2**40 / 2**20` uses INTEGER `**` and `/` (the universal result of `/` is not
  convertible). The resolver inserts an explicit HIR node `ExprKind::ImplicitConv` at each
  converted operand, so M2 never infers conversions [Q1].
- Discrete ranges whose bounds are both universal_integer (`for i in 0 to 7`, `array (0 to 7)`)
  are of type INTEGER (5.3.2.2 / 10.10).

### 2.9 Generic types (6.5.3, 2019 classes)

`generic (type T ...)` declares `TypeDef{Generic{class}}` plus a first subtype. Inside the generic
unit `T` is opaque: only the operations of its class are available, declared implicitly like for
any type, plus whatever interface subprograms are declared.

```rust
pub enum GenericClass {           // M0 `GenericTypeDef`
    Private,                       // `type T` (2008) and `type T is private` (2019)
    Scalar,                        // `is <>`
    Discrete,                      // `is (<>)`
    Integer,                       // `is range <>`
    Physical,                      // `is units <>`
    Floating,                      // `is range <> . <>`
    Access { designated: SubtypeId },
    File { elem: SubtypeId },
}
```

| Class | Implicit operations inside the generic: the predefined operations every type of the class has (2019 6.5.3; the exact LRM list was not read, the reference simulator 2019 agrees where it implements them) [L6] |
|---|---|
| Private, and 2008 `type T` | `=`, `/=`: 2008 6.5.3 declares them implicitly "as formal generic subprograms immediately following the interface type declaration"; no interface `"="` is needed (Q9) [L18] |
| Scalar | relational, `minimum`, `maximum`, `'image`/`'value` and scalar attributes |
| Discrete | Scalar plus `'pos`, `'val`, `'succ`, `'pred`, `'leftof`, `'rightof` |
| Integer, Floating | Scalar plus arithmetic of that class |
| Physical | Scalar plus physical arithmetic |
| Array | `=`, `/=`, `&`, relational if the element class is scalar (2019, LCS-2016-059a; discrete in 2008), array attributes |
| Access | `=`, `/=`, `deallocate`, `.all` |
| File | the file operations |

An actual must belong to the class (`E1503`). A generic array type (`array (natural range <>)
of e`, `array (type is (<>)) of type is private`, `array (0 to 3) of e`) is a `TypeKind::Array`
declared by a `GenericType` declaration, so inside the generic it indexes, slices and takes array
attributes like any array type; with an index constraint its first subtype is constrained and
the actual must be constrained too (L92). Anonymous generic types (`type is ...` as an index,
element or designated type, M0 `IncompleteSubtype::Anonymous`) are fresh generic types named
after their owner (`vt'index`, `vt'element`), not declared by name, with their class operations
in the generic list's region; `check_class` checks their actual parts and `seed_type` maps them
to the actual's index, element or designated subtype (#30). A literal converts implicitly to a
generic type of the integer or floating class. Generic constants, ports and parameters with an
anonymous type (`c : type is range <>`, #283, L94) get their own anonymous generic type, bound to the type of the
actual: package and subprogram instances through `bind_const`, entities, components and blocks through a hidden generic type
appended to their generics (`with_hidden_args` adds its `Actual::Subtype`), subprogram calls and operators through `implicit_instance` (instances of generic subprograms keep their unbound anonymous types; the anonymous types of a formal subprogram match the actual's profile, `profile_fits`, #308).

### 2.10 Implicitly declared operations (5.x, `lib/NATIVE.md` section 2)

**Rule: every predefined operation is a real `Decl::Subprogram` entity**, created in the same
region right after the type declaration, `implicit = true`, so visibility, hiding, aliases,
`use p."+"` and overload resolution need no special cases.

```rust
pub struct Subprogram {
    pub kind: SubprogramKind,             // Function | Procedure
    pub purity: Purity,
    pub params: Box<[Param]>,             // Param { name, class, mode, subtype, default: Option<ExprId> }
    pub ret: Option<SubtypeId>,
    pub generics: Option<GenericList>,    // uninstantiated subprogram (2008)
    pub body: Body,
    pub mangled: Symbol,                  // 5.3
}
pub enum Body {
    Pending,                              // spec analyzed, body not yet seen
    Hir(ScopeId, StmtId),                 // VHDL body
    Implicit(ImplicitOp),                 // predefined
    Native(Symbol),                       // explicit declaration without body in std (NATIVE.md s.1)
    Foreign(ExprId),                      // 'FOREIGN attribute (VHPIDIRECT, M3)
    Instance(InstId),                     // instantiated subprogram
    Interface,                            // formal generic subprogram
}
pub struct ImplicitOp { pub family: Family, pub ty: TypeId, pub form: Form }   // native `yavs.implicit.<family>.<type>[#<form>]`
```

Which families each type gets (from NATIVE.md section 2, with the LRM type-class rules):

| Type class | Families |
|---|---|
| every type except file, protected | `eq`, `neq` |
| scalar | `lt le gt ge minimum maximum`, `to_string` |
| integer, floating | `add sub mul div neg pos abs exp`; integer also `rem mod`; `**` with INTEGER right operand |
| physical | `add sub neg pos abs`, `mul`/`div` by INTEGER and REAL, `div` P/P to universal_integer, `mod rem` (2008) |
| BOOLEAN, BIT | `and or nand nor xor xnor not`, BIT: `meq mneq mlt mle mgt mge cond` |
| 1-D array of discrete element | `lt le gt ge`, `minimum maximum` (array form 2008), `cat` (4 forms) |
| 1-D array of BOOLEAN/BIT | logical ops (array-array, array-scalar, scalar-array, unary reduction, 2008), `sll srl sla sra rol ror`, BIT arrays: matching ops |
| 1-D array of a character-literal enum | `to_string`; BIT_VECTOR also `to_ostring`, `to_hstring` (aliases `to_bstring` etc. are declared in std) |
| other 1-D arrays | `cat` |
| access | `deallocate` (procedure) |
| file | `file_open` (2 forms), `file_close`, `read`, `write`, `endfile`, `flush`; 2019 `file_state file_mode file_position file_size file_seek file_truncate file_rewind` |
| BOOLEAN, BIT signals | `rising_edge`, `falling_edge` (2008, `std.standard` lists them) |
| scalar types (2019) | an anonymous range record type (`LEFT`, `RIGHT` of the type, `DIRECTION`), created eagerly with every scalar type and reached through `R'RECORD`; the names `<T>_RANGE_RECORD` only for the `std.standard` types that `standard.vhdl` lists (U-3) [L7] |

The native symbol for an implicit op is `yavs.implicit.<family>.<type mangled>[#<form>]`; `form` tells
apart the overloads one type has in one family (`&` array-element `as`, element-array `sa`,
element-element `ss`; reductions `reduce`; physical `*` `/` by INTEGER/REAL `pi pr ip rp`, P/P `pp`;
`file_open` with status `status`, the 2019 function `function`; `to_string` `digits format unit`).
For explicit
bodiless std subprograms sem computes the NATIVE.md symbol (`yavs.<package>.[<pt>.]<name>.<param
types>[.r_<ret>]`) by the same rule as `tools/gen_native_md.py`; a test asserts that analyzing
std in both revisions yields exactly the 235 symbols of NATIVE.md section 1, and that every
bodiless subprogram outside std is an error (`E1230`, missing body) unless it has `'FOREIGN`.

Cost: about 30 decls per array type and 20 per scalar type; fine. Instances (5.2) clone them.

### 2.11 Attributes yielding subtypes and reflection

- `x'subtype` (2008): subtype of object `x`. Static if `x`'s subtype is fully constrained by a
  static subtype indication; otherwise `Constraint::FromObject{expr: x, elem_depth: 0}`.
- `x'element` / `A'element`: element subtype; `FromObject{elem_depth: 1}` when it must come from
  the object.
- An element of a record subtype `FromObject{x}` whose declared subtype has open bounds
  (`Db::field_subtype`) is `FromObject` of the field name `x.elem` (an `ExprKind::Field`
  expression), so an aggregate for `x'subtype` gives its `others` the bounds of that element
  (L114, #514).
- `T'base`: only as prefix of another attribute (16.2).
- 2019 `'reflect` (16.2.?): `obj'reflect` has type `std.reflection.VALUE_MIRROR`,
  `T'reflect` has `SUBTYPE_MIRROR`. Sem loads `std.reflection` on demand, needs no use clause.
  The mirror methods are natives (NATIVE.md, reflection rows). `'reflect` is not static.

### 2.12 Anonymous types summary

Anonymous types arise from: integer/floating/physical type declarations (base type), constrained
array type declarations (base type), universal types, and 2019 anonymous generic types. Every
anonymous type has a `TypeDef` with `anonymous = true` and the declared name, so messages say
`` the base type of `byte` `` instead of `<anonymous>`.

## 3. Overload resolution and expression typing

### 3.1 Two passes

Resolution follows the classic two-pass scheme over each **complete context** (LRM 12.5: a
declaration, a statement, or a specification).

1. **Bottom-up `cands(e) -> CandSet`**: the set of possible result types of AST expression `e`,
   memoized per AST node address during the complete context, so each node is visited once.
2. **Top-down `resolve(e, exp: Expected) -> ExprId`**: picks exactly one interpretation that
   fits `exp`, recurses into children with the formal types of the chosen interpretation, and
   builds HIR.

```rust
pub enum CandSet {
    Types(SmallVec<[Interp; 2]>),       // concrete interpretations
    Class(ClassMask),                   // literal forms typed only by context
}
pub struct Interp { pub ty: TypeId, pub via: Via }  // Via: Object | Call(DeclId) | EnumLit(DeclId) | Attr | ...
bitflags! ClassMask { ANY_INTEGER, ANY_REAL, ANY_ACCESS /*null, allocator*/, ANY_COMPOSITE /*aggregate*/,
                      STRING_LIKE /*1-D array of char-literal enum*/, ANY_ARRAY, ANY_RECORD }

pub enum Expected {
    Type(TypeId),                       // assignment target, formal, index type, ...
    Condition,                          // BOOLEAN, else implicit `??` (3.6)
    Any,                                // case expression, qualified operand already typed, ...
    Discrete,                           // range bounds without a type mark
}
fn compatible(c: &CandSet, t: TypeId) -> bool  // exact base type match, universal -> numeric class,
                                               // Class(mask) admits t, Error admits everything
```

To keep `a & b & c & ...` linear, an operator node's `cands` keeps only operator decls whose
every operand's `CandSet` is `compatible` with the formal; the top-down pass then filters by the
expected type and re-checks. No interpretation is enumerated twice.

### 3.2 Selection at a node

```
resolve(e, exp):
  ints = cands(e) filtered by compatible-with(exp)
  0  -> E1301 "no interpretation of <e> matches <exp>": per candidate, one secondary label at its
        declaration with its signature and the first failing argument (3.9)
  1  -> build HIR, recurse into children with the formals of that interpretation
  >1 -> apply preference (Q1), then if still >1: E1302 ambiguous, every candidate as a label
```

Contexts and their `Expected`:

| Context | Expected |
|---|---|
| `target := e`, `target <= e` | `Type(base(target))`; the target is resolved first, standalone; an aggregate target takes its type from the right side (10.6.2) |
| actual of formal | `Type(formal)` (per candidate subprogram, inside 3.3) |
| `if`/`while`/`when`/`assert`/`exit when`/`next when`/conditional assignment conditions, `wait until` | `Condition` |
| `case e is` | `Any`; must resolve alone, without the choices (10.9) |
| index expression | `Type(index type)` |
| range `l to r` without type mark | `Discrete`: bounds resolved together, both universal gives INTEGER |
| `report s severity l` | `Type(STRING)`, `Type(SEVERITY_LEVEL)` |
| `after t`, `wait for t` | `Type(TIME)` |
| generic/port actual | `Type(formal)` with formal generic types substituted |

### 3.3 Name interpretation (the parser leaves names ambiguous)

M0 produces `Apply { prefix, args }` for calls, indexing, slicing and conversions, and
`Selected`/`Attribute`/`Signature` names. Sem first resolves the prefix chain to a `NameRes`:

```rust
enum NameRes {
    Overloads(SmallVec<[DeclId; 4]>),   // functions, procedures, enum literals
    Object(ExprId),                     // already a typed prefix
    TypeMark(SubtypeId),
    Unit(UnitId), Library(LibId), Scope(ScopeId),
    AttrFunc(PredefAttr, AttrPrefix),   // `T'image`, `a'length`, `s'delayed`
    Error,
}
```

then classifies `Apply`:

| Prefix | Args shape | Interpretations offered |
|---|---|---|
| `TypeMark` | one positional expression | type conversion (9.3.6); `cands` = `{T}` |
| `TypeMark` | a discrete range / `open` | subtype indication with index constraint (only in subtype contexts) |
| `Overloads` | any | function calls whose formals accept the association list (named, positional, defaults); **plus**, for each candidate callable with no arguments whose result is an array, indexing or slicing of the result (8.3, `f(i)` where `f` is parameterless) |
| `Object` of array type | args are expressions, one per dimension | indexed name |
| `Object` of array type | one discrete range, a subtype name, or a range attribute | slice |
| `Object` of access-to-array | as above | implicit dereference then index/slice |
| `AttrFunc` | per attribute table | attribute function call (`T'image(x)`, `a'length(2)`) |
| `Overloads` of procedures | statement context only | procedure call |

Slice versus index is decided syntactically when the argument is a range or `'range` attribute,
and by resolving a bare name argument: a type mark means slice. Association lists:
- Positional then named; a formal named twice or missing without default: `E1310`, `E1311`.
- Named association of a sub-element of a composite formal (`f(x.a => 1, x.b => 2)`) is
  individual association (6.5.7.1); all sub-elements must be covered. Port and generic maps
  and subprogram calls (`assoc_map` gives `Slot::Parts`) build `Actual::Individual`; coverage
  is not checked yet.
- Formal/actual conversion functions and type conversions in port and generic maps
  (`to_int(f) => sig`) are recognized when the formal part is a call whose single argument is a
  formal name.
- Defaults are filled in the HIR as `Actual::Default` (the formal's default expression id), so
  M2 sees a full argument list in formal order.

A procedure call statement with `l: p;` concurrent form (M0 `ConcStmtKind::ProcedureCall`) is
re-interpreted as a component instantiation if `p` resolves to a component, entity
(`entity` keyword absent is then an error), or configuration.

### 3.4 Aggregates (9.3.3)

The type of an aggregate comes **only** from context (`Class(ANY_COMPOSITE)` bottom-up). With
`Expected::Any`, an aggregate is `E1320`. Given the type:

**Record aggregate**: choices are element simple names or `others`; each element exactly once
(`E1321` missing, `E1322` duplicate or more positional elements than the record has, `E1100` no
such element, `E1329` a choice that is not an element name, `E1326` positional after named);
`others` must cover at least one element (`E1325`) and all elements one value covers must have
the same type (`E1323`) (9.3.3.2).

**Array aggregate**: for an n-dimensional array, each element is an (n-1)-dimensional
aggregate or, for 1-D (2008), an expression of either the element type **or the aggregate's own
type** (a slice-like positional piece). For each element, intersect `cands(expr)` with
`{elem type, aggregate type}`. The LRM has no tie-break [L8]. The aggregate's own type is a candidate only for a positional
association or a discrete-range choice (9.3.3.3), and an element that is itself an aggregate takes
the element type (otherwise `((fs, "fs "), (ps, "ps "))` for an array of records would be
ambiguous; the reference simulators do the same). A remaining tie, such as an overloaded `f` returning both
types, takes the element type (the reference simulator; user decision L8, permissive), with no `E1323`. `E1323`
stays for an element that fits neither type.

Choices: index expressions, discrete ranges, subtype names, `others`. Rules (9.3.3.3):
- `others` last and alone in its association; needs a constrained context subtype (`E1325`).
- Named and positional associations do not mix (except a trailing `others`): `E1326` (`E1324` is
  retired).
- An n-dimensional aggregate whose elements are not (n-1)-dimensional aggregates: one `E1323`, at
  the first such element.
- A named aggregate without `others` takes its bounds from its choices, also in a constrained
  context: missing or extra indices at the ends are the target's length check (`E1405`), not
  `E1328` (lrm-checks L45).
- A non-locally-static choice is allowed only as the single choice of a single association;
  otherwise `E1326`.
- Static choices: duplicate/overlap `E1327`, uncovered index values when no `others` and the
  context subtype is constrained `E1328`. Choice sets are folded to sorted `(lo, hi)` intervals
  of positions (enums by pos); the same folding checks case statements, which report `E1820`
  instead (`E1327`/`E1328` are for aggregates only). A case subject that is neither discrete nor
  a one-dimensional array of a discrete type is `E1300` at the subject, a misplaced `when others`
  `E1820` at the `others`, an array choice of the wrong length `E1405`.

Bounds of the result (stored, not recomputed by M2):

```rust
pub enum AggBounds {
    Context,                                   // fully constrained context subtype gives them
    Positional { count: ExprId },              // left = index subtype 'left, direction of index subtype
    Named { low: ExprId, high: ExprId },       // from the choices; direction of the index subtype
    Concat,                                    // 2008 slice-valued elements: runtime sum of lengths
}
```

Implemented in `expr/aggregate.rs`. When the bounds are static (and for a named aggregate with one
non-static choice) the aggregate's `Expr::subtype` is also an anonymous subtype carrying them, so
M2 reads the bounds there; otherwise `AggBounds` alone describes them. In an n-dimensional
aggregate the value of an association for dimension `k < n-1` is a nested `ExprKind::Aggregate`
(or a string literal `Const` for the last dimension) whose `subtype` is the outermost context
subtype; its dimension is its nesting depth. Folded array values use the canonical form of
`aggregate::array_const` (bytes for 1-D enumeration arrays up to 64K elements, merged runs
otherwise, rule 12).

### 3.5 Literals

| Literal | Bottom-up | Resolved with type T |
|---|---|---|
| abstract integer | `{universal_integer}` | value i64 checked (`E1402` overflow) |
| abstract real | `{universal_real}` | f64, nearest, ties-to-even |
| based literal `16#FF#`, `2#1.0#E3` | as above | exact integer arithmetic, then f64 for reals |
| character `'a'` | every visible enum type that has `'a'` as a literal | `EnumLit`; an enumeration context without `'a'` is `E1330` at the literal |
| string `"abc"` | `Class(STRING_LIKE)` | each char must be a literal of T's element enum (`E1330` at the bad char, naming the element type); 1-D only; elements that are not of an enumeration type are `E1330` at the literal |
| bit string `8x"FF"`, `b"1_0"`, `ux"F"`, `sx"F"`, `d"12"` | `Class(STRING_LIKE)` | expanded to characters by `literal::expand_bitstring` (15.8: base digits to `0/1`, other graphic characters replicated per digit, length adjust with zero/sign extension, `E1331` on dropping non-zero digits); then as a string literal |
| physical `10 ns` | `{type of unit}` | value * unit scale in base units, checked i64 |
| `null` | `Class(ANY_ACCESS)` | |

Bit-string expansion is type independent in 2008 (15.8), so it is folded once to a `Symbol`.

### 3.6 Conditions and the implicit `??` (9.2.9)

`resolve(e, Condition)`: if some interpretation of `e` has type BOOLEAN, resolve as
`Type(BOOLEAN)`. Otherwise take the candidate types `T` of `e` for which a visible
`"??"(T) return BOOLEAN` exists; exactly one must remain, and the HIR is
`Call("??", [resolve(e, Type(T))])` with `implicit_cond = true` (for diagnostics and VHPI). None:
`E1340` "condition is not BOOLEAN" with help "no `??` is visible for `std_ulogic`; is
`ieee.std_logic_1164` used?". More than one: `E1341`.

### 3.7 Qualified expressions, conversions, allocators

- `T'(e)`: `e` resolved with `Type(base(T))`; the result subtype is `T` (2008: the value must
  belong to `T`, a runtime check that M2 inserts because the HIR node is `Qualified`).
- Type conversion `T(e)` (9.3.6): `e` resolved with `Any` (its type must be determined without
  context); legal between closely related types: numeric to numeric, same base type, arrays with
  the same dimensionality, closely related index types and element types (2008 relaxation: element
  types closely related, not identical). Otherwise `E1350`.
- `new S` / `new T'(e)`: `cands = Alloc(base type of S)` (9.3.7): it fits the access types whose
  designated base type is that one, so `new integer = new integer` or an overloaded call picks the
  matching access type (#27); on resolution, the designated subtype of the expected access type
  is checked as before.

### 3.8 2008 rule changes to implement (not relaxations)

| Feature | Clause |
|---|---|
| reading `out` ports | 6.5.2 |
| expressions as port actuals (implicit signal, M2 makes a process) | 6.5.6.3 |
| `process (all)` | 11.3, sem computes the set (6.9) |
| conditional and selected signal/variable assignments in sequential code | 10.5, 10.6 |
| `case?`, `select?`, matching relational operators | 10.9, 9.2.3 |
| unary reduction operators, array-scalar logical operators | 9.2.2 |
| case expression of a 1-D character array type whose subtype is not locally static, when all choices have the same length (analysis error otherwise); the expression length is checked at run time, a mismatch is an error, not `others` | 10.9 [L9] |
| `if`/`case` generate with alternatives and labels | 11.8 |
| generics referring to earlier generics, generic types/subprograms/packages | 6.5.6 |
| `force`/`release` | 10.5.2 |
| `'subtype`, `'element`, `'simple_name`, `'instance_name` on more prefixes | 16.2 |
| context declarations | 13.4 |

### 3.9 Ids of resolved references

Every resolved reference in the HIR is an id: objects and subprograms by `DeclId`, types by
`TypeId`/`SubtypeId`, enumeration literals by `DeclId` plus position, record elements by
`(TypeId, u32)`, statements by `StmtId`. No HIR node keeps a name to be looked up again.

## 4. Static evaluation

### 4.1 Classification (9.4)

```rust
#[derive(PartialOrd, Ord)]
pub enum Staticness { None, Globally, Locally }
```

Computed for every `Expr` bottom-up while building HIR:
- **Locally static**: literals; constants with locally static subtype and value declared in this
  unit or a package; locally static subtype attributes; implicit operations on locally static
  operands; qualified expressions and conversions to locally static subtypes; aggregates whose
  choices and elements are locally static. 2008 adds calls to functions of `std_logic_1164`,
  `numeric_std`, `numeric_bit`, `numeric_std_unsigned`, `numeric_bit_unsigned` with locally static
  actuals; exactly these five packages (9.4.2 e) [L10].
- **Globally static**: generics, generic-dependent constants, deferred constants, pure function
  calls with globally static actuals, `'path_name`/`'instance_name`, everything in a generate
  parameter's scope that depends on it.
- **None**: everything else.

### 4.2 Folding

`stat::fold(ExprId) -> Result<Option<ConstId>, Diagnostic>`:

```rust
pub enum ConstValue {
    Int(i64), Real(f64), Enum(u32), Phys(i64),
    Array(ArrayConst), Record(Box<[ConstValue]>), Null,
}
pub struct ArrayConst {
    pub bounds: Box<[(i64, Direction, i64)]>,       // one per dimension
    pub elems: Elems,
}
pub enum Elems {
    Bytes(Box<[u8]>),            // 1-D arrays of enums with <= 256 literals: strings, bit/std_ulogic vectors
    Runs(Box<[(u64, ConstValue)]>), // run-length, rule 12: `(others => (others => '0'))` of 64K entries is one run
}
```

- Integer and physical arithmetic is **checked i64** (rule 7); overflow is an error (`E1401`,
  see Q5 for non-required contexts). The result is also checked against the result subtype when
  that subtype is static (`E1403`).
- Reals: f64 arithmetic, no fast-math; `**` with integer exponent by repeated multiplication in
  the same order the runtime uses (M2 must match, rule 5).
- What sem folds itself: scalar implicit operations, relational and logical ops on enums and
  static arrays, `&` of static 1-D arrays, scalar and array attributes of static subtypes,
  qualified expressions, numeric conversions (real to integer rounds half away from zero, 9.3.6),
  aggregates, string and bit-string literals.
- What sem does **not** fold: user subprogram calls, including the 2008 locally static ieee
  calls (Q3: deferred to M2's interpreter hook).
- Required folding (an error if not possible): integer/floating/physical type ranges and unit
  scales, case and aggregate choices for duplicate/coverage checks, attribute dimension
  arguments, the static index checks of 4.3.
- Folded values are a cache on the `Expr`; the HIR expression stays, so M2 can always emit code
  from the tree (rule 1).

### 4.3 What is checked where

| Check | Analysis (sem) | Elaboration | Runtime |
|---|---|---|---|
| scalar range of a locally static value into a static subtype | error `E1403` | | |
| index/slice out of a static array's static range | warning `W1404` (code may be dead) | | error |
| globally static values (generics) | classified | folded by the interpreter (M2) | |
| array length mismatch in assignment, both static | error `E1405` | | |
| everything else | | | checked, rule 7 |

## 5. Generics

### 5.1 Model

A generic unit (package, subprogram, 2019 protected type with generics) is analyzed **once**
with opaque generics (2.9). An instance is a substitution plus a clone:

```rust
pub struct Instance {
    pub generic: DeclId,                     // uninstantiated package/subprogram
    pub site: DeclId,                        // the instantiation declaration
    pub map: GenericMap,
    pub scope: ScopeId,                      // cloned visible part (eager)
    pub body: OnceCell<ScopeId>,             // cloned body (lazy, when M2 asks)
    pub mangled: Symbol,
}
pub struct GenericMap {
    pub types: HashMap<TypeId, SubtypeId>,       // formal generic type -> actual subtype
    pub consts: HashMap<DeclId, ExprId>,         // formal constant -> actual expression
    pub subprograms: HashMap<DeclId, DeclId>,    // formal subprogram -> actual subprogram
    pub packages: HashMap<DeclId, InstId>,       // formal package -> actual instance
}
```

**Instantiation clones the analyzed HIR, it does not re-analyze the AST.** Re-analysis would
re-run overload resolution with the actual types and can pick different overloads than the
generic body did (inside the generic, `a = b` on a formal type resolves to the formal `"="`;
with `T => integer` re-analysis would also see the predefined integer `"="` and become
ambiguous). Cloning keeps every resolved reference and only remaps ids:
- every declaration in the cloned region gets a fresh id (types declared in an instance are
  distinct per instance: two `fixed_generic_pkg` instances have different `ufixed` types);
- references to formals are replaced by the actuals (`types`, `consts`, `subprograms`, `packages`);
- calls to an implicit op of a formal generic type are remapped to the same family's implicit op
  of the actual type; for a 2008 `type T`, `=`/`/=` become the actual's **predefined** `=`/`/=`,
  and `W1506` warns at the instantiation when a different user `"="` for the actual type is
  visible there (the reference simulator would use that one; U-6);
- folded constants that depended on generics are dropped and re-folded if the actual is locally
  static.

The visible part is cloned eagerly (clients need its types), the body lazily.

### 5.2 Generic maps and defaults

| Formal | Actual | Default |
|---|---|---|
| constant | expression, `Type(formal subtype after substitution)` | the formal's default expression |
| `type T` (any class) | a subtype indication; checked against the class (`E1503`) | none (2008), `E1502` if missing |
| subprogram | a subprogram name, with optional signature, whose profile conforms after substitution | `is <>`: the unique subprogram with the formal's designator visible **at the instantiation** whose profile conforms; `is name`: `name` resolved at the formal's declaration (the reference simulator; the second reference simulator crashes on the case), then checked at the instantiation [L11]; none or several: `E1504` listing the candidates |
| package `package P is new G generic map (<>)` | an instance of `G` | `generic map (default)`: actual instance must use the defaults; explicit partial maps must match the actual's generics |

Generic maps resolve in order, so a later formal's subtype can use an earlier formal type (2008).

### 5.3 Identity and mangling (rule 4)

Every instance is a distinct entity, even with identical actuals. Mangled names
are stable text built from declaration paths:

| Entity | Mangled name |
|---|---|
| unit | `<lib>.<unit>` (`ieee.numeric_std`, `work.tb.arch` for an architecture) |
| declaration in a region | `<region>.<name>` (`work.tb.arch.p1.counter`, unlabelled process: `_p<n>` by statement order) |
| overloaded subprogram | `<region>.<name>(<param type>,...)[<ret type>]` with each type by its mangled base type name (`ieee.numeric_std."+"(ieee.numeric_std.unsigned,ieee.numeric_std.unsigned)ieee.numeric_std.unsigned`) |
| package instance unit | `<lib>.<inst>` (`ieee.fixed_pkg`) |
| local package instance | `<region>.<inst>` |
| subprogram instance | as an overloaded subprogram, with the instantiated profile (`<region>.<inst>(<param type>,...)[<ret type>]`), so instances under one name stay distinct; a header with a generic list and a generic map (4.2.1) is such an instance of an undeclared uninstantiated subprogram (`header_instance`, #227) |
| a declaration inside an instance | `<instance mangled>.<name>` |
| implicit op | `<region of type>.<op>(<types>)`, native link `yavs.implicit.<family>.<type>` |

Per elaborated instance storage (one local package instance in an architecture instantiated 8
times) is elab's concern; the code identity is the site's mangled name. For M2, `Db::package_regions` gives a package
unit's regions (none for an uninstantiated generic package; a design unit instance's cloned
visible part and body) and `Db::decls_with_instances` a region's declarations with each package
instance's declarations in place, so an instance's objects are stored with the region declaring
it (#101).

Packages declared in declarative parts (2008, #34) are `DeclKind::Package { scope, generics,
body }`: the package region is a `Package` child of the declaring region, the body a
`PackageBody` child of the package region, written in the same region or, for a package
declared in a package declaration, in that package's body (lookup then continues in the body's
own region through `Scope::outer`). `decls_with_instances` puts a non-generic one's
declarations in place like an instance's. A generic one instantiates like a library generic
package, its body cloned from `body` by `instance_body`; inside a clone of an enclosing generic
package the nested generic is cloned too (its body by `fix_bodies` with the enclosing body), and
an instance of it instantiates that clone (`r_instance`, keys of the composed remap mapped as
well). A signal or shared variable in a process or subprogram is `E1237`, also in a package or
package instance there (L82); instances may precede the generic body in the region (L81).

A design entity with generic types, subprograms or packages is cloned with its architecture by
`Db::instantiate_entity` (`generic/entity.rs`, #75), which elab calls per instance: one clone per
distinct (entity, architecture, map of those actuals), like an ordinary architecture whose code
every instance shares. Generic constants stay generics of the clone (instance data); the
clone's generic list holds only them. Mangled names: `work.e[t=>std.standard.integer,f=>...]`
for the entity, plus `.<arch>` for the architecture, so declarations inside get distinct names.

Components and blocks with generic types, subprograms or packages (#222): `Db::instantiate_component`
clones the component declaration per distinct map for elab (no code, so no mangled name of its
own); a block has one generic map, so `Db::instantiate_blocks` replaces the block statement in
place by its clone at the end of the architecture (its names stay the site's). Configuration
specifications inside a cloned region are cloned with the remap (`clone_specs`).

### 5.4 2019 additions

| Feature | Sem work |
|---|---|
| generic type classes (2.9) | class check on actuals, class ops inside |
| anonymous generic types in interface lists | fresh generic type per object |
| generic lists on protected types (M0 `ProtectedDecl.generics`, `generic_map`) | the protected type is a generic unit; a declaration with `generic_map` is an instance |
| array generic with generic element (`type A is array (natural range <>) of type is private`) | two formals, element formal mapped from the actual's element subtype |

## 6. External names, aliases, attributes, signatures, configurations, protected types, views

### 6.1 External names (8.7)

`<<signal .tb.dut.s : std_logic>>`, `<<constant ^.^.c : integer>>`, `<<variable @lib.pkg.v : t>>`.
Sem resolves the subtype indication and the class; the path is kept as data and bound by elab
(rule 4). HIR: `ExprKind::External(ExternalId)` with `class`, `path`, `subtype`.
Usage checks by class at sem (a constant external name is not a target, `E1601`), through
aliases too: `alias s is <<signal .tb.dut.s : t>>;` is an object alias of the external name
(6.6.2, no subtype indication, `E1612`), bound by elab where the alias is declared. Package paths
(`@lib.pkg.obj`) could be checked at analysis; for one code path they are also bound at elab.
An external name used directly in a process, a concurrent statement, an expression port actual
or inside an alias target is given an implicit alias by `Db::alias_external_uses` (called by
elab, idempotent): the use's `ExprKind::External` becomes `ExprKind::Obj` of a hidden
`Alias::Object` (`implicit: true`) whose target is a new `External` expression, so M2 lowers it
as a declared alias (#93). Sem's own analysis never sees the rewritten form. A subprogram
(`ExtOwner::Sub`) owns the names of its body, the subprograms and protected bodies nested in it
and its declared aliases of whole external names, which elab binds per elaborated copy of the
region declaring it, or once per design for a package (#255). `Db::bind_package_paths` (called
by elab) turns every package path that names an analyzed package object of its class and type
into `ExprKind::Obj` of that object (`Db::package_target` keeps the binding for LSP roots).

### 6.2 Aliases (6.6)

```rust
pub enum Alias {
    Object { target: ExprId /* a static name */, subtype: Option<SubtypeId> },
    NonObject { target: DeclId },        // type, subtype, subprogram (with signature), enum literal, unit, ...
}
```

- Object alias: the target must be a static name (`E1610`); with a subtype indication, the
  subtype must match the target's base type, and an unconstrained alias subtype takes the target's
  bounds (`BoundsFrom::Aliased`).
- Subprogram and enum literal aliases need a signature when the name is overloaded (`E1611`).
- Alias of a type implicitly declares aliases for its predefined operations and literals (6.6.3);
  sem creates `Alias::NonObject` decls for them in the alias's region.
- Lookup is transparent: `Alias::NonObject` resolves to the target entity, but diagnostics name
  the alias spelling the user wrote (`` `str_write` (alias of `std.textio.write`) ``).
- 2019: aliases in a protected type declaration may name methods of a private variable (method
  forwarding); see 6.6.

### 6.3 Attributes (16, 6.7)

Predefined attributes are a table keyed by `(name, prefix kind)`:

| Prefix kind | Attributes |
|---|---|
| scalar type/subtype | `left right high low ascending image value pos val succ pred leftof rightof base`, 2008 `subtype` |
| range (2019, prefix is an attribute name denoting a range, e.g. `t'range`) | `record` (the range record subtype, type mark only), `value` (its value); there is no `'RANGE_RECORD` attribute (16.2.6) [L12] |
| array object/type | `left right high low range reverse_range length ascending element` (with dimension argument), 2008 `subtype` |
| signal | `event active last_event last_active last_value driving driving_value`; implicit signals `delayed(t) stable(t) quiet(t) transaction` |
| named entity | `simple_name instance_name path_name` |
| object (2019) | `image` (scalar objects), `reflect` [L13] |
| type (2019) | `reflect`, `designated_subtype` (access/file), `index` and `index(n)` (array, the index subtype; type mark only) [L14] |
| mode view (2019) | `converse` |

- The table, prefix forms per revision and typing are `attr::TABLE` (lrm-checks L12 to L14). An
  implicit signal object keeps its defining `ExprKind::Attr` as its `default`.
- An attribute name is a primary whose value can be indexed or sliced (8.4, 8.5):
  `takes_args` is true only for a predefined attribute with a parameter, so `x'simple_name(1)`
  indexes the string value; a user attribute with arguments gets index/slice candidates of its
  value's type. A type-mark prefix's user attribute is looked up on the type or subtype
  declaration itself (`type_mark_decl`), so an attribute of a subtype is found (#653, #657,
  #658, [L140]).
- `s'delayed(t)`, `s'stable(t)`, `s'quiet(t)`, `s'transaction` create **implicit signal**
  entities (`DeclKind::Object` with `ImplicitSignal{attr, prefix_lsp, param}`) in the enclosing
  region, one per distinct (prefix LSP, attribute, static parameter) occurrence, so M2 creates
  their processes. They are illegal in subprograms (`E1620`).
- User-defined attributes: `attribute a : T;` declares; `attribute a of x, y : signal is e;`
  (and `others`, `all`) attaches value `e` (resolved with `Type(T)`) to each named entity. The
  HIR stores `AttrValue { attr: DeclId, entity: DeclId, value: ExprId }` per entity; `x'a` is an
  `ExprKind::AttrValue` reading it. Entity class must match the named entity (`E1622`); a
  subprogram or literal needs a signature when overloaded. The value of an attribute of a signal
  is a constant, not a signal. `'FOREIGN` on a subprogram sets `Body::Foreign`.

### 6.4 Signatures (4.5.3)

`[t1, t2 return r]`: each type mark resolved to a subtype; matching compares base types of the
profile. Used by aliases, attribute specifications, generic subprogram actuals and `use` of a
single overload. No match `E1630`. Several matches cannot happen: a full signature (parameters
and result) is unique among homographs, so there is no code for it.

### 6.5 Configurations and binding (3.4, 7.3)

- Configuration declarations: the entity must exist; the block configuration names an
  architecture of it; nested `for label` / `for label(i)` / `for label(1 to 3)` select generate
  blocks (index or range must be within the generate's static range when static); component
  configurations name instance labels (`all`, `others`, list) that are instances of the named
  component. Binding indications: `use entity l.e(a)`, `use configuration l.c`, `use open`,
  with generic and port maps checked against the entity. Incremental binding (7.3.2.? 2008): a
  configuration declaration may add maps to an instance already bound by a configuration
  specification and may repeat its entity aspect, but may not bind a different one (`E1702`).
- Configuration specifications in architecture declarative parts are checked the same way.
- **Default binding** (7.3.3) is computed by elab through `sem::default_binding(instance)`:
  1. an entity with the component's simple name that is **directly visible** at the instance
     (through `use l.e` or `use l.all`);
  2. else the entity of that name in the library that contains the design unit declaring the
     component (a component from a package of another library binds to that library's entity,
     #291);
  3. else the entity of that name in the library that contains the instantiating design unit
     (`work`);
  4. architecture: the most recently analyzed one (7.3.3); with no cache, "most recently
     analyzed" is defined by manifest order (Q4);
  5. generics and ports associated by name; a component port with no entity port of the same
     name is an error, an entity port with no component port is `open` if it has a default or is
     an `out` (7.3.3).
  One reported case is the ambiguity between rules 1 or 2 and rule 3 (the work library) when both
  exist with different targets: yavs takes the first and emits `W1703` naming both.
- M1 checks every configuration that is analyzed; elab applies them.

### 6.6 Protected types (5.6)

- Declaration region: subprogram declarations, use clauses, attribute specs, aliases; 2019 adds
  `private variable v : other_pt;` (M0 `ObjectDecl.private`), visible only in the protected body,
  and aliases that forward methods of a private variable as methods of this type. An alias in a
  protected type declaration that denotes anything other than a method of a protected type is an
  error (2019 5.6.2); `private shared variable` is an error (a grammar slip, VI#251); private
  variables may be of any type; methods made visible by an alias need no body [L15].
- A call through such an alias runs on the private variable: `Db::forwarding` maps the alias to
  the variable, and the call's `method_of` is `PrivateVar { object, var }` (from outside, `object`
  the protected object) or `Obj(var)` (the implicit object inside the body), nested once per
  forwarding alias of a chain. An alias of a method of any other object (`alias push is
  sv.push [integer];`, a shared variable or a variable, by simple or expanded name) forwards the
  same way: the call's `method_of` is a fresh object reference to it, replacing the object the
  call had (#237, [L15]). Other prefixes (`f(x).m`, `p.all.m`) report `E1999` (S7).
- Body: bodies for every declared method (`E1230` for missing), plus any declarations.
- Methods get an implicit first parameter `self` in the HIR (matches NATIVE.md's native calling
  convention); calls are `Call{callee, method_of: Some(obj)}`.
- Restrictions: no signals, ports, files of protected type; no `=`; function methods follow the
  normal rule, pure unless declared `impure` (LRM 4.2.1; the reference simulator rejects a pure method reading the
  body's variables, `E1830`); shared variables must be of a protected type (6.4.2.4, `E1904`,
  `--relaxed` makes it `W1904`, R1 in 8.4). Parameters of protected type are class variable.
- Incomplete types as method return types, 2.6.

### 6.7 Mode views and `'converse` (2019 6.5.2)

```rust
pub struct ViewDef { pub decl: DeclId, pub record: SubtypeId, pub elems: Box<[ElemMode]> }
pub enum ElemMode { Simple(Mode), View { view: ViewId, converse: bool, array: bool } }
```

- `view v of r_t is a, b : in; c : out; d : view w; end view;` every element of `r_t` exactly once
  (`E1901` missing/duplicate); `d : view w` requires `w`'s record subtype to match the element's.
- `v'converse` is a view with `in`/`out` swapped and `inout` and `buffer` unchanged (the reference simulator; user
  decision U-4, permissive: a `buffer` element may be read and driven) [L16], nested views
  conversed recursively. Sem materializes it as a `ViewDef`
  on first use.
- Ports `p : view v` (record) and `p : view (v) of arr_t` (array of record) carry a **mode tree**:
  per element the effective mode. Port association checks per element (an `in` element is not
  driven inside, etc.) use the same mode tree, by the target name's own steps (a non-static
  `bs(i).ready` drives only `ready`).
- A call with a view signal parameter drives, in the calling process, one longest static prefix
  per `out`/`inout`/`buffer` element of the mode tree (`Db::view_driven_paths`, `call_drivers`):
  the actual's prefix plus the element's field steps, a `Field` after an array meaning that
  element of every array element (#254). For an individual association the parts count one by
  one: the elements below the part's formal name (`Db::view_driven_below`, #265).
- `'converse` of anything but a mode view is `E1621` at the attribute. A nested element view of
  another record type is `E1901` at the view name.
- In generic units views are cloned with their declaration (record subtype and nested views
  remapped, `Mode::View` of ports and parameters too); a clone's converse is materialized again.
- **Driver overlay** (rule 2): sem does not build drivers but gives M2 everything to
  build the one longest-static-prefix map (6.9): each port's mode tree and each assignment's LSP.

### 6.8 Shared variables, files, access deallocate

Trivial mappings; listed for completeness: file declarations with `open kind is name` resolve
both expressions (`FILE_OPEN_KIND`, `STRING`); `deallocate` is an implicit procedure per access
type.

### 6.9 Longest static prefixes, drivers and `all` (8.1, 11.3, 14.7.2)

```rust
pub struct Lsp { pub root: ObjRoot, pub path: Box<[LspStep]>, pub stat: Staticness, pub span: Span }
pub enum ObjRoot { Decl(DeclId), External(ExternalId), ImplicitSignal(DeclId) }
pub enum LspStep { Field(u32), Index(Box<[ExprId]>), Slice(RangeC), Rebase(SubtypeId) }
```

- A name through an object alias is the aliased name's LSP (6.6.2). When the alias has an array
  subtype indication, a `Rebase(subtype)` step follows: the index and slice steps after it are
  in that subtype's index space (its bounds may differ from the aliased name's), and elab maps
  them by element position. Two paths compare (`overlap`) only up to a `Rebase` of different
  subtypes, then `Maybe`.

- The LSP of a name is its longest prefix whose every index/slice expression is static
  (globally static counts, 8.1; elab evaluates the globally static ones, one reported case was this).
- Each process gets `ProcessInfo { drivers: Vec<Lsp>, reads_all: Option<Vec<Lsp>> }`: the LSPs of
  all signal targets in the process (including targets in procedures declared inside the process,
  14.7.2; such a driver ends before the first index or slice that reads an object of the
  subprogram, whose value exists only when the call runs, #205), and for `process (all)` the LSPs of signals read in the process (11.3: names read in
  the statement part, actuals of mode `in`/`inout` in procedure calls; not signals read inside
  subprograms declared outside the process).
- Concurrent statements get the same information for their equivalent processes. Sem does not
  build the equivalent processes; M2 does (rule 3).
- A signal driven from a subprogram outside any process must be a parameter (`E1810`).
- `span` is the name the prefix was computed from, for driver and sensitivity diagnostics. A slice
  followed by an index or slice at the same array level is stored as the later step alone
  (`s(1 to 2)(2)` is `s(2)`), so steps of two prefixes of one signal line up level by level.
- `wait until c` without `on`: sem stores the signals read by `c` (10.2) in `Wait::on`.
- Concurrent statements store their equivalent process's sensitivity (11.4 to 11.6: signals read
  by the waveforms, conditions, selector, `GUARD`; actuals of `in`/`inout` formals) in
  `ProcessInfo::reads_all`.
- Multiple sources (`E1812`, 14.7.2, 6.4.2.3): sem checks one architecture's concurrent part
  (blocks included, generate bodies not: whether they exist is elab's) for two sources whose
  prefixes overlap on locally static values and whose overlapping part is unresolved; the error
  names both. Everything else (generates, globally static indices, ports driven from outside) is
  elab's, which builds the full map. The reference simulators report the same cases at elaboration.
- `return e` in a function where `e` is a 2019 conditional expression without a final `else` is
  the conditional return (10.14): when no condition holds, execution continues. Sem lowers every
  conditional `return`, and every variable or force assignment with an `unaffected` value, to an
  HIR `If` with one plain statement per alternative and an empty body for `unaffected` (#33), so
  M2 needs nothing new; other conditional expressions are `ExprKind::Conditional` (lrm-checks L60).
- A 2019 named return value `return r of t` (4.2.1) declares `r` in the parameter region as a
  subtype `Constraint::FromObject` of a hidden constant of `t`; `Db::ret_names` maps the region
  to both. A call of such a function has the context subtype as its `Expr::subtype` (lrm-checks
  L62, `E1304` without one), and M2 passes its descriptor as a trailing parameter
  (`crates/yavs-lower/DESIGN.md`). No HIR shape changed. The type is a scalar, array or record
  type or a generic type (`E1233` otherwise, L77); `r` of a scalar type has the range of its
  hidden constant (`Db::scalar_range`), which M2 passes as a one-dimensional descriptor (#327); instance cloning (`fill_scope`) copies the
  `Db::ret_names` entry of every cloned region and checks a generic type's actual, and a
  generic body that completes its spec does not reuse the spec's `r` (#239).

## 7. Typed HIR (what M2 consumes)

### 7.1 Expressions

```rust
pub struct Expr {
    pub kind: ExprKind,
    pub subtype: SubtypeId,          // the result subtype; type is `subtype.base`
    pub span: Span,
    pub stat: Staticness,
    pub value: Option<ConstId>,      // folded value, a cache
}
pub enum ExprKind {
    Const(ConstId),                              // literals and folded enum literals
    Obj(DeclId),                                 // object, interface object, generic constant
    External(ExternalId),
    Call { callee: DeclId, args: Box<[Arg]>, method_of: Option<ExprId>, implicit_cond: bool },
    Index { prefix: ExprId, indices: Box<[ExprId]> },
    Slice { prefix: ExprId, range: RangeC },
    Field { prefix: ExprId, elem: u32 },
    Deref { prefix: ExprId },                    // `.all` and implicit dereference
    PrivateVar { object: ExprId, var: DeclId },  // 2019: private variable of a protected object (6.6)
    Attr { attr: PredefAttr, prefix: AttrPrefix, args: Box<[ExprId]> },
    AttrValue { spec: AttrValueId },             // user-defined attribute
    Aggregate { assocs: Box<[AggAssoc]>, bounds: AggBounds },
    Qualified { to: SubtypeId, operand: ExprId },
    Conv { to: SubtypeId, operand: ExprId },     // explicit type conversion
    ImplicitConv { operand: ExprId },            // universal -> `subtype`
    New { init: NewInit },                       // Subtype(SubtypeId) | Value(ExprId)
    Null,
    Open,                                        // port/generic map `open`
    Error,                                       // never reaches M2: errors stop the pipeline
}
pub struct Arg { pub formal: u16, pub actual: Actual }
pub enum Actual {
    Expr(ExprId),
    Default,                                     // the formal's default
    Open,
    Individual(Box<[(Box<[LspStep]>, ExprId, ExprId, bool, bool)]>), // sub-element association: path, actual, formal part, written `inertial` (#261), the actual of an `in` port's part is a conversion `conv(signal)` (#611)
    Converted { formal_conv: Option<DeclId>, actual_conv: Option<DeclId>, actual: ExprId,
                copy_in: Option<ExprId>, copy_out: Option<ExprId> }, // calls: actual part, formal part as expressions
    Subtype(SubtypeId),     // actual of a generic type in an instance's generic map
    Subprogram(DeclId),     // actual of a generic subprogram in an instance's generic map
    Package(InstId),        // actual of a formal package in an instance's generic map
}
```

### 7.2 Statements and units

```rust
pub struct Stmt { pub id: StmtId, pub label: Option<Ident>, pub span: Span, pub kind: StmtKind }
```

`StmtKind` mirrors the M0 statement set after resolution: sequential (`Assign{target: Target,
value, delay: Waveform/Option<ExprId>}`, `If`, `Case{matching: bool, choices: ChoiceSet}`, loops
with `LoopParam(DeclId)`, `Next/Exit{loop: StmtId}`, `Wait{sensitivity: Vec<Lsp>, until, for}`,
`Return`, `ProcCall(Call)`, `Assert`, `Report`, `Force/Release`, `Null`, 2019 `SequentialBlock`)
and concurrent (`Process{info: ProcessInfo, decls: ScopeId, body}`, `Block{guard, ports, generics}`,
`ConcAssign{guarded, guard: Option<DeclId>}` (the `GUARD` signal of a guarded assignment: the block's implicit one or a visible signal of that name, 11.6, #641), `ConcProcCall`, `ConcAssert`, `Instance{target: InstTarget, scope, generic_map, port_map}`
(`scope`: the instance's region, for `Db::default_binding(scope, component)` in elaboration),
`ForGenerate{param, range}`, `IfGenerate`, `CaseGenerate`). Targets keep their `Lsp` next to the
`ExprId`. Every statement has a `StmtId`; M2 tags IR instructions with it (plan: statement-location
tags) and coverage combines it with path and span hash (rule 10).

Units: `Unit` (1.2) plus kind-specific HIR (entity ports/generics/statements, architecture decls
and statements, package scope, configuration tree). Spans everywhere; identifiers as `Ident`
(spelling plus key, rule 8), so VHPI `CaseName`, waves and diagnostics get the user's spelling.

### 7.3 Invariants (`hir::validate`, run in debug builds and in every corpus test)

- No `ExprKind::Error`, `DeclKind::Error`, `TypeKind::Error` or `Body::Pending` (outside std
  natives) in a unit that analyzed without errors.
- Every `Call` argument list is complete and in formal order.
- Every expression's subtype base equals the type its parent expects, except under `Conv`,
  `ImplicitConv`, `Qualified`.
- Every universal-typed expression whose parent expects a specific type is wrapped in `ImplicitConv`.
- Every signal target has an `Lsp`.

## 8. Diagnostics

### 8.1 Code ranges

Existing: `E00xx` syntax (M0), `E01xx` driver/CLI, `E9999` internal error, `W00xx` syntax.
Sem takes `E1xxx` / `W1xxx`; `E1999` is a construct sem does not implement yet, naming the slice
of section 10 (`Db::unsupported`; #842: `E9999` is only for internal errors):

| Range | Area | Examples |
|---|---|---|
| E10xx | libraries, units, context | E1001 unknown library, E1002 unit not found in library (names where it looked: library path and manifest), E1003 dependency cycle, E1004 secondary unit without primary, E1005 `work` in a context declaration, E1007 declaration or construct not available under the file's or the design's revision (0.1) |
| E11xx | scopes, visibility | E1100 not declared, E1101 redeclaration (homograph), E1102 use clause conflict, E1103 not visible here (exists but hidden), E1104 prefix of a use clause, context reference or expanded name is not a library, package, package instance or context declaration (a missing suffix in a package is E1100), E1105 private/not in visible part, E1106 end label mismatch |
| E12xx | types and subtypes | E1201 not a type, E1210 dimension constrained twice, E1211 constraint on a non-composite/non-scalar, E1212 object needs fully constrained subtype, E1213 bad record constraint, E1214 index constraint arity, E1215 static range or index constraint outside its type mark or index subtype, E1220 incomplete type not completed or used early, E1221 2008 access to file/protected, E1222 illegal file element, E1223 constant, signal, generic, port or constant/signal parameter of an access or file type, E1224 constant without a value outside a package declaration, E1225 bad resolution function, E1226 deferred constant named before its full declaration (a package initial value, or anywhere in the package body), E1227 full declaration of a deferred constant whose subtype indication does not conform, E1230 missing body, E1232 type, subprogram or package interface in a parameter list (4.2.2.1), E1233 named return value of a type that is not scalar, array or record (2019, 4.2.1), E1236 parameter of mode `buffer` or `linkage` (4.2.2.1), E1237 signal or shared variable in a process or subprogram, also in a package or package instance there (11.3, 4.7), E1238 plain variable outside a process, subprogram or protected type (6.4.2.4), E1239 component declaration, configuration or disconnection specification in a process or subprogram (4.3, 11.3), E1240 subprogram body not conforming to its declaration (4.10) |
| E13xx | expressions, overloading | E1300 type mismatch, E1301 no matching overload, E1302 ambiguous, E1303 illegal operator declaration (not an operator symbol, or wrong arity, LRM 4.5.2), E1304 named return value without a constrained context subtype (2019, 4.2.1), E1310 formal associated twice, E1311 missing actual, E1312 conversion that does not give a formal that is not fully constrained its bounds (5.3.2.2, F8, #262), E1313 conversion the formal's class or mode does not allow, E1320 aggregate type not determined, E1321-E1328 aggregate/choice errors, E1330 character not in type, E1331 bit string loses digits, E1340/E1341 condition, E1350 illegal conversion, E1360 class/mode misuse (assign to `in`, read `linkage`, read an `out` port or parameter in a 1993 file, variable assignment to signal) |
| E14xx | static evaluation | E1401 overflow, E1402 literal out of range, E1403 static value outside subtype, E1405 static length mismatch, E1410 choice/range not locally static where required |
| E15xx | generics | E1501 not a generic unit, E1502 missing generic actual, E1503 actual outside generic class, E1504 no/ambiguous `is <>` default, E1505 call of uninstantiated subprogram |
| E16xx | names, aliases, attributes, signatures | E1601 external name class misuse, E1610 alias of a non-static name, E1611 alias needs a signature, E1620 implicit signal attribute in subprogram, E1621 unknown attribute, E1622 entity class mismatch, E1623 attribute dimension argument out of range for the prefix, E1630 signature matches nothing |
| E17xx | configurations, binding, components | E1700 instantiated name is not a component, entity or configuration, E1701 label is not an instance of the component, E1702 rebinding, E1703 port/generic mismatch between component and entity, E1704 no entity bound where one is needed |
| E18xx | statements, processes, drivers | E1801 `wait` in a process with sensitivity list or in a function, E1802 `return` placement, E1803 sensitivity name not static, E1810 signal driven from outside a process, E1811 `next`/`exit` outside loop, E1812 unresolved signal with multiple sources, E1813 entity statement not passive, E1820 case coverage/duplicates, E1830 pure function side effect |
| E19xx | protected types, views, 2019 | E1904 shared variable not protected, E1905 illegal use of a protected type (signal, port, `=`), E1907 variable port of a composite of protected types (not supported), E1901 view element missing/duplicate, E1902 view on non-record, E1903 view port mode misuse, E1910 2019 feature in 2008 mode (normally caught by M0) |
| W1xxx | warnings | W1008 construct means something different under the design revision (0.1), W1401 static overflow where the value is not required (Q5; the runtime check stays), W1403 a static operand of a qualified expression or type conversion outside its subtype, or an array converted to an unconstrained array type with bounds outside the target's index subtype (the runtime check fails), W1404 static index out of range in possibly dead code, W1501 unused generic, W1506 an instance of a 2008 `type T` uses the actual's predefined `=`/`/=` while a different user `"="` for the actual type is visible at the instantiation (U-6; the reference simulator uses the user one), W1703 default binding ambiguity, W1830 R2 relaxation applied (pure function calls something impure), W1904 R1 relaxation applied (shared variable of a non-protected type); one warning code per relaxation (8.4), no shared W1990 |

Codes are `pub const` in `yavs-sem::diag` with an `--explain` text each (one paragraph plus a
minimal example), checked by a test that every constant has an explain entry.

### 8.2 Cascade suppression

- Poison values: `DeclKind::Error`, `TypeKind::Error`, `ExprKind::Error`. `compatible()` with an
  Error type is true; every check with an Error operand is silent.
- An undeclared name is reported once per (design unit, key); later uses get no new error.
- A failed unit (`UnitSlot::Failed`) is silent to its dependents.
- An overload error inside an argument marks the call `Error` without a second "no matching
  overload" at the call.
- Motto: any error in any reachable unit means no elaboration; sem returns `Err` and the driver
  prints "aborting due to N previous errors" (`E0101`).

### 8.3 Help and suggestion patterns

| Situation | Rendering |
|---|---|
| undeclared name (E1100 and its siblings: selected name, record element, formal in a map, attribute, library, unit, architecture) | help `a signal with a similar name exists: `count``, kind noun included (`suggest::help`), spelling as declared; with a single candidate in the same file a label "similar name declared here" at its declaration. Candidates: names visible from the scope (direct and potential), same kind of namespace per site. Match (`suggest::similar`, only on the error path): Damerau-Levenshtein distance <= max(1, len/3), or an abbreviation (`cnt` for `count`: 3+ letters, same first letter, letters in order, name at most twice as long), ranked after the edit matches; best one, two when equally close ("similar names exist: `a` (constant), `b` (constant)"); nothing when nothing is close. A package that declares exactly the name (not a near miss) gives instead "`unsigned` is declared in `ieee.numeric_std`; add `use ieee.numeric_std.all;`" from a search over loaded packages |
| no matching overload | primary at the call; one secondary label per candidate (max 8, then "and N more") at its declaration, message = its signature (`function "+" (l: unsigned; r: natural) return unsigned`), plus a note per candidate with the first failing argument ("argument 2: expected `natural`, found `std_logic_vector`") |
| ambiguous overload | primary at the call, each remaining candidate as a labelled secondary span with its signature and the package that declares it; help: "qualify the operand (`unsigned'(x)`) or use a selected name (`ieee.numeric_std."+"`)" |
| type mismatch | "expected `T`, found `U`"; secondary label at what made `T` expected (the target declaration, the formal); help for known conversions (`std_logic_vector` to `unsigned`: "use `unsigned(x)`"; integer to vector: "`to_unsigned(x, n)`") |
| use clause conflict | both use clauses as secondary labels, "both make `x` potentially visible" |
| condition not BOOLEAN | 3.6 |
| missing generic `is <>` default | candidates listed as for overloads |

Signatures are rendered by one function `sem::display::signature(DeclId)` with original spelling.

### 8.4 `--relaxed` (rule 13)

Each relaxation turns an error into its own warning code; nothing else differs.

| Id | Relaxation | Why |
|---|---|---|
| R1 | shared variable of non-protected type (6.4.2.4) | verification libraries and VHDL-93-style code; the kernel already marks such processes serial |
| R2 | pure function reading or calling something impure (4.?/9.4.?) | common in vendor libraries |
| R3 | dropped: the reference simulator's `--relaxed` does not relax case choices (its list is impure calls and files in pure functions, non-globally-static interface defaults, non-protected shared variables) [L17] | per Q6 |
| R4 | default binding to an entity in `work` when it is not visible (implementation choice) | none needed if Q4 picks the rule above |
| R5 | an `others` aggregate assigned to an object of unconstrained subtype (a port, a formal) takes the object's run-time bounds (LRM 9.3.3.3 needs a constrained context; `W1325`) | both reference simulators accept it under their relaxed switch (second suite `issue1191`, `issue2421`); agent, 2026-10-23, confirm | 
| R6 | the actual of a signal parameter that is not a static name (`p(s(i))` with a variable or loop parameter `i`; LRM 4.2.2.3; `W1361`) | sim1 accepts it under its relaxed switch (first suite `signal28`); agent fix7, 2026-10-28, #813, confirm |

Q6 asks the user to confirm the list.

## 9. Testing plan

### 9.1 Positive corpora (zero diagnostics, both revisions where the source allows)

| Corpus | Where | How |
|---|---|---|
| std, ieee 2008 and 2019 | `lib/` | `lib/ORDER.md` blocks, analyzed in order; plus on-demand (analyze `ieee.float_pkg` alone and let it pull its deps) |
| NATIVE.md cross-check | `lib/NATIVE.md` | 2.10 |
| External test runner library (incl. its copy of a verification library) | local checkout of its `vhdl` directory | compile order from the runner's own dependency scan (run its compile-style ordering offline once and check the order list into `tests/corpus/vunit.order`) |
| External verification library | the copy inside the runner's `vhdl` directory | its `.pro` order, checked in as `tests/corpus/osvvm.order` |
| Second external verification library | `~/.cache/rsim-corpora/uvvm` (shallow clone, read-only, commit `90d56e9`) | each component's `script/compile_order.txt`; needs `--relaxed` (R1) |
| Primary reference simulator `test/regress` | local checkout, `test/regress` | pass/fail list checked into the repo (expected-fail entries included) |
| Primary reference simulator `test/sem`, `test/parse` | local checkout, `test/sem` (184 files) | files the reference simulator expects to fail: list with "must produce >= 1 error"; others must analyze clean |
| Second reference simulator `testsuite/gna` | fetched in CI, not local now | pass/fail list |
| Conformance suite | fetched in CI | pass/fail list |

All corpus tests are cargo tests that skip with a printed reason when a local checkout is
missing; CI fetches them (plan). Every corpus test also runs `hir::validate`.

### 9.2 Negative tests

`tests/diag/sem/<area>/<name>.vhd` with the expected rendered output in `<name>.stderr` (plain
format, no colour), blessed with `YAVS_BLESS=1`. At least one test per error code; for codes with
help text, one test per help pattern. Each file starts with a comment naming the LRM clause.
Revision is chosen by a `-- yavs: --std=2019` first line.

### 9.3 Fuzzing (rule 15)

- `cargo fuzz` target `sem` taking bytes, parsing, and analyzing in a fresh `Db` with std/ieee
  loaded once per process (cloned `Db` snapshot). Invariant: no panic; ICEs only through the
  `E9999` path; on success `hir::validate` passes.
- Seed corpus: all positive corpora files plus the negative tests.
- A structure-aware mutator on the AST is out of scope for M1 (add when byte fuzzing plateaus).

## 10. Implementation order (each slice ends green: `cargo test`, `clippy -D warnings`)

| Slice | Content | Green when |
|---|---|---|
| S1 | `Db`, ids, arenas, libraries, on-demand loader, context clauses, scopes, lookup, enum/integer/floating/physical types, objects, subprogram specs | `std.standard` analyzes in both revisions |
| S2 | implicit operations, natives, universal types, literals, overload resolution core (3.1-3.3, 3.5, 3.6), subprogram bodies, sequential statements | `std.textio`, `std.env` (2008, 2019), `std_logic_1164` + body; NATIVE.md cross-check passes |
| S3 | arrays, records, the full constraint model (2.2-2.7), aggregates and choices, attributes, static folding (section 4) | numeric_std/bit (+unsigned), math_real, math_complex, std_logic_textio; **rule 1 review checkpoint with the user** |
| S4 | generics: generic packages/types/subprograms, instantiation by cloning, mangling | fixed/float generic packages and `fixed_pkg`/`float_pkg` instances, both revisions; all of `lib/` green |
| S5 | design units: entity, architecture, concurrent statements, processes, LSPs, drivers, `all`, components, instances, configurations | External test runner corpus green |
| S6 | protected types, access, files, shared variables, aliases complete, user attributes, external names | external verification library green |
| S7 | 2019: views/`'converse`, private variables, reflection, range records, generic classes complete | `std/2019/reflection.vhdl`, `env.vhdl` (2019) and the 2019 corpus subsets green |
| S8 | negative suite to full code coverage, help patterns, `--relaxed`, fuzz target, reference simulator and conformance suite lists, external verification libraries | M1 exit criteria in the plan met |

The plan's gate "the type and constraint model (rule 1) is reviewed before M2" happens at the
end of S3 on real code (numeric_std), so a rework does not cost the later slices.

## 11. Decisions and remaining checks

Q3 to Q9 and Q11 were accepted by the user as recommended (2026-10-18). Q10 was replaced by the
mixed-revision rule in 0.1. Q1 and Q2 were checked on 2026-10-18 (`lrm-checks.md`): Q1 follows the
LRM text, Q2's premise was wrong and is back with the user as U-1. The user answered U-1 to U-6
and L8 on 2026-10-18 under the LRM interpretation policy (top of this file); the rows below record
them.

| # | Topic | Decision | Status |
|---|---|---|---|
| Q1 | universal preference (`x := 2 ** 3`), LRM 9.3.6 | 2.8: convert only convertible universal operands, only with a unique numeric target, and only when no interpretation without conversion exists (the second reference simulator agrees; the reference simulator prefers converting) | checked, corrected to the LRM text |
| Q2 | base range of an anonymous integer base type (`t'base'high`) | base range = declared range, intermediate results checked against it (the reference simulator; the reference simulators both report the declared range for `'base`, neither uses 32/64-bit) | decided (U-1) |
| Q3 | 2008 locally static ieee calls without an interpreter in M1 | classified in M1; their choice checks deferred to elab through a `ConstEval` hook implemented by M2's interpreter | decided |
| Q4 | default binding | last architecture in manifest order, note when several; directly visible entity preferred over `work`, `W1703` | decided |
| Q5 | overflow folding where the value is not required | error `E1401` only in required contexts (type and array bounds, physical unit scales, choices, attribute dimension arguments); `W1401` elsewhere, e.g. a constant initializer, with the runtime check kept | decided |
| Q6 | `--relaxed` | R1 and R2 now; R3 only after checking the reference simulator's list | decided |
| Q7 | generic instances | clone the HIR: visible part eager, body lazy | decided |
| Q8 | mangled names | long readable form of 5.3; NATIVE.md names only for natives | decided |
| Q9 | equality on generic types | class-implicit ops map to the actual's predefined op; 2008 `type T` has implicit `=` and `/=` (6.5.3), mapped to the actual's **predefined** `=`/`/=` (the second reference simulator; the reference simulator uses the `"="` visible at the instantiation, e.g. `numeric_std."="`; U-6) | corrected 2026-10-18 (`lrm-checks.md` L18); the "no implicit `=`" half is withdrawn |
| Q10 | revisions | replaced by 0.1: per-file parsing, one design revision (newest in the closure), `E1007` error, `W1008` warning | decided |
| Q11 | bit-string expansion | in sem, `E1331` at the literal | decided |
| U-1 | base range of `type T is range L to R` | declared range; intermediate results checked (the reference simulator) | decided |
| U-2 | incomplete types in protected method profiles | allowed in subprogram profiles inside a protected type declaration, 2008 and 2019, **only in units of library `std`** (`std.reflection` needs it); an error `E1220` in user code, as the reference simulators (user, 2026-10-18) | decided |
| U-3 | range record names | `<T>_RANGE_RECORD` only for `std.standard` types; the direction type is `RANGE_DIRECTION` (LRM 16.3, `lib` patched) | decided |
| U-4 | `buffer` under `'converse` | stays `buffer`, so both reading and driving are allowed (permissive) | decided |
| U-5 | conditional expressions as interface defaults | accepted bare and parenthesized (permissive) | decided |
| U-6 | `=` of a 2008 `type T` instance | the actual's predefined `=`/`/=`; `W1506` at the instantiation when a different user `"="` for the actual type is visible there | decided |
| L8 | aggregate element: element type vs aggregate type tie | the element type (the reference simulator), no `E1323` for that case | decided |
