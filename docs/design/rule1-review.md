# Rule 1 review: constraints as runtime data

For the user to approve before M2 lowering and elaboration rely on it (plan.md M1: "the type
and constraint model (rule 1) is reviewed before M2 starts"). Issue #96.

Rule 1 (plan.md): constraints are runtime data on every composite; there is one subtype
representation and no "maybe static" path. sem.md section 2 makes it concrete: a `SubtypeDef`
carries one effective, already merged `Constraint`; range bounds are HIR expressions whose folded
value is only a cache; `x'subtype`/`x'element` of an object that is not fully constrained is
`Constraint::FromObject`; every object records `BoundsFrom` (`Subtype`, `Initializer`, `Actual`).

The M1 lead had pinned this on numeric_std (`tests/stdlib.rs` `numeric_std_subtypes_rule_1`,
`numeric_std_body_rule_1`). This review covers the composite cases numeric_std does not have.
Every case below is a test in `crates/yavs-sem/tests/rule1.rs`: 14 shape tests and 5 ignored finding
tests on VHDL written fresh for the review, plus 4 tests on real code with `--features corpus`. Tests marked `ignore` assert the correct
shape for a finding and name its issue.

## How to read the dumps

The tests print subtypes in a compact form, built only from public HIR data:

```
[resolution] base(index constraint)(element constraint) [constrainedness]
```

- `7 downto 0`: both bounds folded (locally static, `Expr::value` is set).
- `{W - 1} downto 0`: the left bound is an expression evaluated at run time (the source text in
  braces); the expression is always there, folded or not.
- `{x'range(1)}`: `RangeC::Attr`, the range of a named object or subtype, read at run time.
- `(open)`: no index constraint at this level; `rec_t(data(7 downto 0), keep(0 to 0))`: a
  record constraint (only the constrained elements are stored).
- `from-object(bus_p.data, depth 0)`: `Constraint::FromObject`; depth 1 is `'element`.
- `[Full] / [Partial] / [Unconstrained]`: `Db::constrainedness`, computed, never stored.
- Objects: `role class: subtype, bounds from Subtype | Initializer | Actual`.

"M2 gets the bounds from" says where lowering and elaboration must read them at run time. The
simulator columns are analysis (and elaboration plus run where a top exists) with reference simulators 1, 2 and 3 (versions 1.23-devel,
7.0.0-dev and 2025.3 (2008 mode)), each under
`timeout 60 systemd-run --user --scope -p MemoryMax=1G`. The VHDL in the test file is the input
of those runs.

## 1. Records with unconstrained array elements

```vhdl
type rec_t is record
  data : std_ulogic_vector; keep : std_ulogic_vector; last : std_ulogic;
end record;
subtype rec_full_t  is rec_t(data(7 downto 0), keep(0 to 0));
subtype rec_part_t  is rec_t(data(open), keep(0 to 0));
subtype rec_part2_t is rec_part_t(data(15 downto 0));
type outer_t is record hdr : rec_t; payload : std_ulogic_vector; end record;
subtype outer_full_t is outer_t(hdr(data(3 downto 0), keep(0 to 0)), payload(31 downto 0));
port (p_full : in rec_full_t; p_part : in rec_part_t; p_un : out rec_t);
signal s_dyn : rec_t(data(W - 1 downto 0), keep(W / 8 - 1 downto 0));
```

| Name | Representation | M2 gets the bounds from |
|---|---|---|
| `rec_t` | `rec_t [Unconstrained]` | (type) |
| `rec_full_t` | `rec_t(data(7 downto 0), keep(0 to 0)) [Full]` | constant descriptor |
| `rec_part_t` | `rec_t(data(open), keep(0 to 0)) [Partial]` | `open` stays `open` |
| `rec_part2_t` | `rec_t(data(15 downto 0), keep(0 to 0)) [Full]` | merged once at the declaration |
| `outer_full_t` | `outer_t(hdr(data(3 downto 0), keep(0 to 0)), payload(31 downto 0)) [Full]` | nested record constraint |
| `c_init : rec_t := (...)` | `rec_t [Unconstrained]`, bounds from Initializer | the initial value |
| `p_full` | `[Full]`, bounds from Subtype | the subtype |
| `p_part`, `p_un` | `[Partial]` / `[Unconstrained]`, bounds from Actual | the actual at instantiation |
| `s_dyn` | `rec_t(data({W - 1} downto 0), keep({W / 8 - 1} downto 0)) [Full]` | the expressions, evaluated at elaboration |

Record aggregates have `AggBounds::Record`: each element value carries its own bounds, e.g.
`(data => (W - 1 downto 0 => '0'), ...)` keeps `{W - 1}` as an expression; a static one is
folded. The reference simulators: all accept. Test `records_with_unconstrained_elements`.

A selected element takes the prefix's record constraint: `s_full.data` is
`std_ulogic_vector(7 downto 0) [Full]`, `v_full.keep := p_part.keep` is `(0 to 0)` on both sides
(F3, fixed on main by f1582e5 while this review ran). Not right yet: an element value of a record
aggregate is resolved in the element's declared subtype instead of the context's record
constraint (F2), so `payload => (others => '0')` in an `outer_full_t` context is E1325.

## 2. Arrays of unconstrained arrays

```vhdl
type slvv_t is array (natural range <>) of std_ulogic_vector;
subtype byte_vec_t    is slvv_t(open)(7 downto 0);
subtype four_t        is slvv_t(0 to 3)(open);
subtype four_bytes2_t is byte_vec_t(0 to 3);
subtype four_bytes3_t is four_t(open)(7 downto 0);
type mat_t is array (natural range <>, natural range <>) of std_ulogic_vector;
subtype mat23_t is mat_t(0 to 1, 0 to 2)(3 downto 0);
type cube_t is array (natural range <>) of slvv_t;
subtype cube_full_t is cube_t(0 to 1)(0 to 3)(7 downto 0);
subtype cube_mid_t  is cube_t(open)(0 to 3)(open);
constant c_rows : slvv_t(0 to 1) := ("UX01", "ZWLH");
signal s_ram : slvv_t(0 to N - 1)(W - 1 downto 0);
```

| Name | Representation |
|---|---|
| `byte_vec_t` | `slvv_t(open)(7 downto 0) [Partial]` |
| `four_t` | `slvv_t(0 to 3)(open) [Partial]` |
| `four_bytes2_t`, `four_bytes3_t` | `slvv_t(0 to 3)(7 downto 0) [Full]` (merging from either side gives the same) |
| `mat23_t` | `mat_t(0 to 1, 0 to 2)(3 downto 0) [Full]` |
| `cube_full_t` / `cube_mid_t` | `cube_t(0 to 1)(0 to 3)(7 downto 0) [Full]` / `cube_t(open)(0 to 3)(open) [Partial]` |
| `c_rows` | `slvv_t(0 to 1) [Partial]`, bounds from Initializer (element bounds from the value) |
| `bytes : in byte_vec_t` | `[Partial]`, bounds from Actual; `bytes(0)` is `std_ulogic_vector(7 downto 0) [Full]` |
| `s_ram` | `slvv_t(0 to {N - 1})({W - 1} downto 0) [Full]`; `(others => (others => '0'))` gets `AggBounds::Context` |

M2 gets index bounds from the outer constraint and element bounds from the element constraint;
`Partial` levels from the initial value or the actual. The reference simulators: all accept. Test
`arrays_of_unconstrained_arrays`.

## 3. Interface objects, function results, constants, aliases

```vhdl
function invert(x : std_ulogic_vector) return std_ulogic_vector is
  variable r : std_ulogic_vector(x'range);
begin r := not x; return r; end function;
procedure fill(o : out std_ulogic_vector; v : std_ulogic) is begin o := (o'range => v); end;
procedure flip(io : inout std_ulogic_vector) is constant tmp : std_ulogic_vector := io; ...
constant c_word : std_ulogic_vector := x"AB";
entity leaf is generic (g_init : std_ulogic_vector); port (d : in std_ulogic_vector; q : out std_ulogic_vector);
alias a_all is d;
alias a_rev : std_ulogic_vector(0 to d'length - 1) is d;
alias a_un  : std_ulogic_vector is d;
u : entity work.leaf generic map (g_init => x"0F") port map (d => sd, q => sq);
```

| Name | Representation | M2 gets the bounds from |
|---|---|---|
| `g_init`, `d`, `q`, `x`, `o`, `io` | `std_ulogic_vector(open) [Unconstrained]`, bounds from Actual | the generic or port actual (`x"0F"` is folded, its value has the bounds; `sd` is `(7 downto 0)`), the call's actual |
| `invert` result | `std_ulogic_vector(open) [Unconstrained]` | the returned value: `return r` is `std_ulogic_vector({x'range(1)}) [Full]` |
| `r` | `std_ulogic_vector({x'range(1)}) [Full]` | `x'range` at run time |
| `c_word`, `tmp` | `[Unconstrained]`, bounds from Initializer | the initial value (`tmp` at run time) |
| `a_all` | alias of `d`, no own subtype | the object |
| `a_rev` | own subtype `std_ulogic_vector(0 to {d'length - 1}) [Full]` | the alias subtype (a view with new bounds) |
| `a_un` | own subtype `std_ulogic_vector(open)` | the object |

The reference simulators: all accept and run. Test `interface_objects_results_constants_aliases`.
`o := (others => v)` for the unconstrained `out` parameter is E1325 in yavs, the reference simulators
(`out_parameter_others_is_an_error`); the same holds for `o <= (others => '1')` on an
unconstrained port (commented on #2 and #38, which assumed the reference simulator accepts it; it does only with
`--relaxed`).

Aliases have no `BoundsFrom`: `Alias::Object { target, subtype }` already says it (no subtype or
a not fully constrained one: the target's bounds; a fully constrained one: its own).
`BoundsFrom::Aliased` (sem.md 2.2) is never used.

## 4. Subtype attributes and generic types

```vhdl
port (bus_p : inout rec_t; rows : in slvv_t);
signal s_bus  : bus_p'subtype;
signal s_data : bus_p.data'subtype;
signal s_row  : rows'element;
signal s_rng  : std_ulogic_vector(bus_p.data'range);
signal s_same : s_full'subtype;      -- s_full : rec_full_t
package inst_un is new work.gen_pkg generic map (t => std_ulogic_vector);
```

| Name | Representation | M2 gets the bounds from |
|---|---|---|
| `s_bus` | `rec_t from-object(bus_p, depth 0) [Full]` | the object `bus_p` |
| `s_data` | `std_ulogic_vector from-object(bus_p.data, depth 0) [Full]` | the element of the object |
| `s_row` | `std_ulogic_vector from-object(rows, depth 1) [Full]` | the element subtype of `rows` |
| `s_rng` | `std_ulogic_vector({bus_p.data'range(1)}) [Full]` | the range of the element |
| `s_same` | `rec_t(data(7 downto 0), keep(0 to 0)) [Full]` | static: `'subtype` of a fully constrained object is its subtype |
| `inst_un` | map `t -> std_ulogic_vector(open)`; `ident` returns `std_ulogic_vector(open) [Unconstrained]` | the actual |

`'base` can only be the prefix of another attribute, and those are not implemented yet (F4).
The reference simulators: all accept. Test
`subtype_attributes_and_generic_types`.

Not right yet: `rows'element'range`, `rows'element'length` and `natural'base'high` (F4); a
constrained generic type actual `t => std_ulogic_vector(7 downto 0)` (F5); after instantiation
the cloned objects keep the generic's `BoundsFrom` (F7).

## 5. The reproducers behind rule 1 (written fresh)

| Case | Situation | yavs | Reference simulator 1 | Reference simulator 2 | Reference simulator 3 |
|---|---|---|---|---|---|
| Subtype of a record port element | `axi : inout bus_rec_t` (unconstrained elements); `signal address : std_logic_vector(axi.address'range)`; `signal data2 : axi.data'subtype` | `{axi.address'range(1)}` and `from-object(axi.data)`, both `[Full]` (`sim2_641_subtype_of_record_port_element`) | ok | ok | ok |
| Subtype of an alias of a record element | `alias axi_data is axi.data; signal data1 : axi_data'subtype;` | `from-object(axi_data)` `[Full]` (F6: an internal error until 260729c landed during this review; `f6_alias_of_record_element`) | ok | ok | ok |
| Element bounds from the initial value | `constant m : mat_t(0 to 1) := ("UX01Z", "WLH-0")`, loops over `m'range` and `m(i)'range` | `mat_t(0 to 1) [Partial]` from Initializer; loop `j` ranges over `{m(i)'range(1)}` (`sim2_1224_element_bounds_from_initial_value`) | ok, runs | ok, runs | ok, runs |
| Formal conversion on an unconstrained port | `port map (std_logic_vector(o) => sig)`, `o : out unsigned` | accepted: `Actual::Converted` with the conversion to `STD_LOGIC_VECTOR`, `o` bounds from Actual (`sim2_2422_formal_conversion_on_unconstrained_port`); F8 | analysis ok, elaboration fails | rejects | rejects (sim3 message 1189) |
| `others` for a record port element | `p.elem <= (others => '0')` for `p : out rec` (unconstrained element); `s1.elem`, `s2 : s1'subtype` | `p.elem`: E1325 (right, all three agree); `s2` is `rec(elem(3 downto 0)) [Full]`; `s1.elem`, `s2.elem` accepted since F3 was fixed (`sim2_2179_others_for_record_port_element`) | rejects line 16 only | rejects line 16 only | rejects line 16 only |
| Record aggregate with `others` over several element subtypes | `(x => '0', inner => c_inner, others => (others => '1'))` over `slv(5 downto 0)`, `slv(9 downto 0)`, `slv(31 downto 0)` | accepted, **folded wrong**: `b` and `c` get 6 bits (F1) | elaboration fails: "value length 6 does not match field B length 10" | run fails: bound check failure | ok |
| Alias of an external name | `alias ram_un is << signal .outer1543.u.ram : ram_t >>`, `ram_un'element'length` | E9999 "aliases of external names is not implemented yet" (S6, #19): explicit, no guessed bounds (`sim1_1543_external_name_alias_is_reported_not_implemented`) | ok, runs | ok, runs | ok, runs |
| Allocator of a constrained record | `subtype container_c_t is container_t(data(0 to cap - 1)(7 downto 0)); container := new container_c_t;` | `container_t(data(0 to 3)(7 downto 0)) [Full]` (`cap - 1` folded from a locally static constant), `NewInit::Subtype` of it (`sim1_1576_allocator_of_constrained_record`) | ok, runs | ok, runs | ok, runs |
| Array of records constrained in a port | `sig : in rec_arr_t(0 to 1)(data(0 to 1))` | `rec_arr_t(0 to 1)(data(0 to 1)) [Full]`; `sig(0)` is `my_rec_t(data(0 to 1)) [Full]`, `sig(0).data` keeps `(0 to 1)` since F3 was fixed (`sim1_1463_array_of_records_constrained_in_port`) | ok | ok | ok |

M2: for #1576 the allocator's designated object gets the full subtype's descriptor and the
access value's `.all` is `container_t` unconstrained, so lowering must take the bounds from the
heap object, which the representation allows. For #1463 VHPI and waves get the element bounds
from the port's subtype (rule 9).

## 6. Real code (corpora from `tools/fetch-corpus.sh`, `--features corpus`)

| Source | Lines | Representation |
|---|---|---|
| external library `modules/common/src/types_pkg.vhd:20` | `type slv_vec_t is array (integer range <>) of std_ulogic_vector;` | `slv_vec_t(open) [Unconstrained]` |
| external library `modules/axi_stream/src/axi_stream_pkg.vhd:109-114` | `function to_slv(...) return std_ulogic_vector is variable result : std_ulogic_vector(axi_stream_m2s_sz(data_width, user_width) - 1 downto 0);` | result unconstrained, bounds from the returned `result`, whose bound is the call expression `{axi_stream_m2s_sz(data_width, user_width) - 1}` |
| external library `uvvm_util/src/types_pkg.vhd:38,42` | `type t_slv_array is array (natural range <>) of std_logic_vector;` `subtype t_byte_array is t_slv_array(open)(7 downto 0);` | `t_slv_array(open)(7 downto 0) [Partial]` |
| external library `osvvm/ScoreboardPkg_slv.vhd:52-55` | `package ScoreBoardPkg_slv is new work.ScoreboardGenericPkg generic map (ExpectedType => std_logic_vector, ActualType => std_logic_vector, ...` | map `ExpectedType`, `ActualType -> (resolved) std_ulogic_vector(open)`; but 53 cloned objects such as `Item` of `Push` (`ScoreboardGenericPkg.vhd:194-197`) keep `bounds from Subtype` on an unconstrained subtype (F7) |
| The test runner `verification_components/src/axi_stream_pkg.vhd:369-377, 1044-1050` | `type axi_stream_transaction_t is record tdata : std_logic_vector; ...`; `variable axi_stream_transaction : out axi_stream_transaction_t`; `axi_stream_transaction.tdata := pop_std_ulogic_vector(msg);` | record `[Unconstrained]`; all six parameters of that type bounds from Actual; element assignments are checked against the actual's bounds at run time |

The reference simulator accepts all five files (`tools/corpus/*.sim1.expected`). Tests `corpus::*`. The external AXI4
record types (`Axi4InterfacePkg.vhd`) and the external VIP records are not covered: those files do
not analyze in yavs yet for unrelated reasons (missing units, #60 and a cascade in the other library).

## Findings

| | Finding | Where | Issue |
|---|---|---|---|
| F1 | A record aggregate association covering several elements (`others`) is resolved and folded once, in the first element's subtype: silent wrong value, a bug that some reference simulators still have (the third one is right). | `expr/aggregate.rs` `record_agg` | #98 |
| F2 | Record aggregate element values are resolved in the element's declared subtype, not the context's record constraint: `payload => (others => '0')` in an `outer_full_t` context is E1325 (all three accept). | `expr/aggregate.rs` `record_agg` | #100 |
| F3 | A selected element (`s.data`, `sig(0).data`) had the element's declared subtype, dropping the prefix's record constraint: false E1325 on `s.data <= (others => '0')` (lines 14 and 15 of the `sim2_2179_others_for_record_port_element` reproducer). **Fixed on main by f1582e5 (#150) during the review**; `f3_selected_element_keeps_record_constraint` now passes. | `expr/names.rs` `Db::field_subtype` | #102 (closed) |
| F4 | Attributes of attribute prefixes: `'element'range` (E1361), `'element'length`, `T'base'high` (E9999). | `attr.rs`, `types.rs` range attributes | #104 |
| F5 | A constrained generic type actual `t => std_ulogic_vector(7 downto 0)` is E1104. | `generic.rs` type actuals | #106 |
| F6 | `alias d is axi.data` ended in an internal error. **Fixed on main by 260729c during the review**; `f6_alias_of_record_element` now passes. | `decl.rs` `alias_decl` | #108 (closed) |
| F7 | Instances copy `BoundsFrom` from the generic, where a generic type counts as fully constrained: wrong `BoundsFrom` in instances (53 in an external scoreboard package), and `signal s : t` instantiated with `std_ulogic_vector` is accepted (the reference simulators reject). This is the "maybe static" path rule 1 forbids. | `generic.rs` `r_object`; `constrainedness` of `TypeKind::Generic` | #109 |
| F8 | A type conversion on an unconstrained formal is accepted, but rule 1 has no representation for bounds that flow back from the actual through the conversion. The reference simulators reject, the reference simulator crashes at elaboration. Needs a decision: reject, or define the bounds as the actual's. | `unit.rs` port maps | #111 (decision) |

No finding is in `types.rs`: constraint merging, record and element constraints, partial
constraints and `constrainedness` were right in every case, so nothing was fixed in this review.
Minor: `BoundsFrom::Aliased` is dead (aliases carry their target instead); string and bit
string literals have an unconstrained subtype and carry their bounds in the folded value, which
is consistent with rule 1 but worth knowing in M2.

## Recommendation

**Approve with fixes.** The representation itself holds: one `SubtypeDef` with a merged
constraint, bounds as expressions with a folded cache, `FromObject` for run-time subtypes and
`BoundsFrom` on every object cover records with unconstrained elements, arrays of unconstrained
arrays, partial constraints at any depth, interface objects, function results, allocators and the
reproducers from both trackers, and none of the cases needs a second, static path. The defects
are in the code that derives subtypes for aggregates and instances, not in the model: F1 (a
silent wrong value, fix before M2 lowers record aggregates), F2 (record aggregate elements must use
the context's record constraint, which `Db::field_subtype` from the F3 fix already computes) and
F7 (recompute `BoundsFrom` and the fully constrained check after generic substitution) should land
before M2 relies on these subtypes; F4 and F5 are missing features that fail clearly; F3 and F6
were fixed on main while the review ran; F8 is yours to decide.
