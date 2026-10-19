# LRM checks for `sem.md`

Status: done 2026-10-18 for every **(unsure)** item and for Q1/Q2 of `sem.md` section 11; the open questions were decided by the user the same day (section "Decisions").

Clause numbers: IEEE 1076-2008 / 1076-2019 (the same unless both are given).

**Policy** (user, 2026-10-18; `docs/plan.md`, `sem.md` "LRM interpretation policy"): where the LRM
is clear, follow it and fail early; where it is unclear or silent, be permissive (the union of what
the reference simulators accept, semantics that let the most designs elaborate), and warn only
where the simulators give different results. Every item that was "still ambiguous" or open as a
question U-n is resolved below under that policy ("Decisions").

The third reference simulator Starter 2025.3 (`vcom -2008/-2019`) was added as a third black box after the first pass;
A commercial simulator was not available.

## Method and sources

No copy of the LRM was available. Evidence, in order of weight:

1. **LRM text quoted by others**: sentences of the LRM quoted in IEEE P1076 issues
   (<https://gitlab.com/IEEE-P1076/VHDL-Issues>, cited as `VI#n`) and in comments of the second reference simulator's
   source (the quotes are read from a local checkout, cited as "the second reference simulator quote").
   Only the quoted LRM sentences are used, never the code. Quotes are at most two sentences.
2. **IEEE package text**: `lib/std/2019/standard.vhdl` (opensource.ieee.org `vasg/Packages`).
3. **Black-box runs**: tiny VHDL files on the reference simulator 1.23-devel (ef5084a94) `--std=2008/2019` and the second reference simulator
   7.0.0-dev (753dfcf0b) `--std=08/19`, each under `timeout 60 systemd-run --user --scope -p
   MemoryMax=1G`. The files are in `~/.cache/rsim-lrmcheck/t/` (not in the repo); the runner is
   `~/.cache/rsim-lrmcheck/bin/run.sh`. Tool behaviour is practice, not the LRM.
4. "Recalled" marks LRM wording from memory that no source above confirms.

The EDA twiki that hosts the LCS-2016 documents returned HTTP 500 for the whole session, so no
LCS text was read.

## Verdicts

| # | sem.md item | Clause | Verdict | Rule for yavs |
|---|---|---|---|---|
| Q1 | universal vs INTEGER interpretation | 9.3.6 | **corrected** | an implicit conversion is used only when no interpretation without it exists; only convertible operands convert |
| Q1a | "named numbers (`constant N := 8;`)" | 6.4.2.2 | **corrected** | VHDL has no named numbers; the syntax is an error in both tools |
| Q2 | `t'base'high` of `type t is range L to R` | 5.2.3.1 | **corrected** (the "as the reference simulators" premise is false) | base range = declared range, intermediate results checked (U-1, decided) |
| L1 | 2019 reserved words | 15.10 | confirmed | 2019 adds exactly `private` and `view` |
| L2 | same entity through an alias (12.4 c) | 12.3, 12.4 | **corrected** | no exception for object aliases; non-object aliases of the same entity do not conflict (both tools) |
| L3 | `work` in context declarations | 13.3 (not 13.4) | confirmed, clause corrected | error for `library work`, and for `work.` prefixes in use clauses **and context references** |
| L4 | incomplete types in protected method profiles | 5.4.2 | unclear, **std only** | accept in subprogram profiles inside a protected type declaration, 2008 and 2019, only in library `std` (U-2, amended 2026-10-18); user code: `E1220` |
| L5 | multi-dimensional arrays as file element | 5.5 | confirmed | error in 2008 and 2019 |
| L6 | implicit ops of 2019 generic type classes | 6.5.3 | confirmed in principle, list not verified | an interface type of class C gets exactly the predefined ops every type of class C has |
| L7 | `<T>_RANGE_RECORD` per scalar type | 5.2.1, 16.2.6, 16.3 (2019) | **corrected** | every scalar type has an anonymous range record; the name `<T>_RANGE_RECORD` only for `std.standard` types; direction type `RANGE_DIRECTION` (U-3, decided) |
| L8 | aggregate element of element type or aggregate type | 9.3.3.3 | **corrected**, tie unclear, **permissive** | no LRM tie-break; aggregates and named non-range choices take the element type; a remaining tie takes the element type (the reference simulator), no `E1323` (decided) |
| L9 | case on a non-locally-static 1-D char array | 10.9 | confirmed | all choices the same length (analysis error), expression length checked at run time |
| L10 | locally static calls into ieee packages | 9.4.2 | confirmed | exactly `std_logic_1164`, `numeric_bit`, `numeric_std`, `numeric_bit_unsigned`, `numeric_std_unsigned` |
| L11 | where `is name` of a subprogram default resolves | 6.5.6.2 | confirmed (the reference simulator; the second reference simulator crashes) | resolved at the formal's declaration; `is <>` at the instantiation, by the formal's designator |
| L12 | spelling of the range record attribute | 16.2.6 (2019) | **corrected** | no `'RANGE_RECORD`; it is `R'RECORD` (and `R'VALUE`) with `R` a range attribute name; `R'RECORD` as a type mark implemented (#180), `R'VALUE` and the `std.standard` names `<T>_RANGE_RECORD` not yet |
| L13 | 2019 `'IMAGE` on objects | 16.2 (2019) | confirmed | `o'IMAGE` for an object of scalar type (the reference simulator 2019) |
| L14 | 2019 `'INDEX` | 16.2 (2019) | confirmed | `A'INDEX` / `A'INDEX(n)`, a subtype; only in a type mark; implemented (#174): the index subtype, narrowed to `A'RANGE(n)` for an object or a fully constrained subtype |
| L15 | 2019 private variables and method aliases | 5.6.2 | confirmed | quote below; `private shared variable` is an error |
| L16 | `buffer` under `'CONVERSE` | 6.5.2 / 16.2 (2019) | unclear (tools disagree), **permissive** | `buffer` stays `buffer`: both read and drive allowed (U-4, decided) |
| L17 | The reference simulator `--relaxed` list (R3) | n/a | **corrected** | The reference simulator does not relax case choices; drop R3 |
| L18 | Q9: implicit `=`/`/=` on a 2008 interface type `type T` | 6.5.3 | **corrected**; which `=` is used differs between tools | `=` and `/=` are implicitly declared; they map to the actual's predefined operators, warning `W1506` when the reference simulator would pick a different visible user `"="` (U-6, decided) |
| L19 | access types designating file and protected types | 5.4.1 | 2008 clear; 2019 tools disagree, **permissive** | 2008: `E1221` (the reference simulators reject both); 2019: accepted (the reference simulators accept both, the third reference simulator 2026.1 rejects both) |
| L20 | 2019 signals and variables with an unconstrained subtype and an initial value | 6.4.2.3, 6.4.2.4 (2019), LCS-2016-019 | confirmed (the reference simulator accepts) | the bounds come from the initial value, as for constants; 2008 keeps `E1212`. Elab builds such signals from the evaluated initial value, element bounds included (#187); ports are interface declarations and keep taking their bounds from the actual |
| L21 | 2019 ports of class variable | 6.5.2, 6.5.7.1 (2019), LCS-2016-047 | confirmed (the reference simulator accepts; the reference simulator rejects a non-protected type and a mode other than `inout`) | accepted in 2019 files: protected type and mode `inout` required (`E1360`), the actual is a shared variable or a variable port; 2008 keeps "a port is a signal". Elab reports them unsupported (#176) |
| L22 | generic subprogram instances whose actuals share a base type (`t => std_ulogic` and `t => std_logic`) | 12.3, 4.5.1 | clear (the reference simulator rejects "homograph", the second reference simulator rejects, the third reference simulator accepts) | homographs in one region: `E1101` (#179, vhdl-ideas `sim/frequency.vhdl` is a design error) |
| L23 | `open` actuals for sub-elements in an individual association | 6.5.7.1, LCS-2016-001 (2019) | clear; the reference simulator agrees | 2008: `E1311` (the reference simulators reject); 2019: the sub-element stays unassociated, except a port of mode `in` without a default (`E1311`, as the reference simulator) (#178) |
| L40 | attribute specification of a port in the architecture (#167) | 7.2, 12.1 | unclear, **permissive** | a port or generic of the entity may be decorated in the architecture's declarative part (the reference simulators accept) |
| L41 | attribute specification of a statement label (#167) | 7.2 | confirmed | written in the declarative part of the construct whose statement part holds the label; applied when the label is declared, `E1100` at the end of the statement part if none is; sequential-statement labels not yet |
| L42 | names in the formal part of a sub-element association (#168, #210) | 6.5.7.1 | confirmed | the formal designator names the formal; its index and range expressions are evaluated in the instantiating region (for a subprogram call: the calling region, per overload candidate); a non-static index or range in it is `E1410` |
| L43 | `'left` and friends of an object of a locally static subtype (#169) | 9.4.2 | confirmed | locally static (so `(buf'left => '0', others => '1')` is legal for `bit_vector(9 downto 0)`); with generic bounds it stays `E1326` (the reference simulators reject too) |
| L44 | sources of an element of a `std_logic_vector` signal (#170) | 4.6, 14.7.3 | confirmed | a part is resolved when its own subtype or that of an enclosing part is (element resolution of `(resolved) std_ulogic_vector`) |
| L45 | named array aggregate without `others` in a constrained context (#45) | 9.3.3.3 | confirmed | its bounds come from its choices (smallest and largest, direction of the context), so missing or extra indices at the ends are a length mismatch with the target (`E1405`, as the reference simulators); `E1328` only for a gap between choices, `E1403` only for a choice outside the index subtype |
| L46 | constants, signals, generics, ports and constant/signal parameters of an access or file type (#198) | 6.4.2.2, 6.4.2.3, 6.5.2 | confirmed | `E1223` in 2008 and 2019, also for a composite with such a subelement (the reference simulators reject all of them in both revisions). **Constant parameters of access type are legal in 2019**: the IEEE 2019 `std.env` source declares `DIR_CLOSE(Dir : in DIRECTORY)` (a record holding `LINE`) and uses `CALL_PATH_VECTOR_PTR`, so the exemption is by revision (file revision 2019, any library), not by library; in 2008 library `std` stays exempt for the same reason. The reference simulators still reject them in 2019 (the LRM wins, rule 2); the LRM text itself was not at hand, the evidence is the normative package source. Signal parameters stay `E1223`. **Protected methods (#212, 5.6.2)**: in 2008 a result or parameter (any class) of an access or file type, or a composite containing one, is `E1223` ("protected method result/parameter"; both reference simulators reject all of them); in 2019 the check is dropped (both accept access results, one accepts variable access and file parameters, the other rejects those: unclear, permissive); library `std` exempt **Questa 2025.3**: rejects constant parameters of access type (and of a record holding one) in a user package in both 2008 and 2019 (vcom-1458, vcom-1462), but its own 2019 `std.env` with a `DIR_CLOSE` taking a `DIRECTORY` analyzes and runs; the exemption by library `std` matches. |
| L47 | expanded names whose prefix is a process, block, generate or loop label or a subprogram (#54) | 8.3 | clear | legal only inside the named construct (`p1.v` in process `p1`); outside, a suffix declared in the construct is `E1103` at the suffix. A subprogram prefix outside it stays a selection from the call's value (`f.x` of a record result), and `f.v` is `E1103` only when no candidate returns a record or an access value |
| L48 | expanded names and use clauses naming a declaration of a package body (#54) | 12.3, 12.4 | clear | `work.p.hidden` and `use work.p.hidden` for an item of the body are `E1105` (not in the visible part) when the body is analyzed, else `E1100`; a use clause or context reference whose prefix is not a package, library or instance (or not a context declaration) is `E1104` at the offending suffix |
| L49 | an external name as the actual of an `in` port (#186) | 8.7, 6.5.6.3, 6.5.7.1, 14.7 | unclear (tools disagree), **permissive** | an expression actual: the formal keeps its own initial value and is driven by `formal <= actual`, one delta behind the actual (the reference simulator) **Questa 2025.3** (2008 and 2019): the port sees the external signal in the same delta (the monitor runs in the iteration of the update of the external signal, one before a monitor on the ordinary port path `leaf.o -> x`), which is what a collapsed port gives and contradicts the one-delta-behind reading above. |
| L50 | a constant of a constrained array subtype initialised by a value with other bounds (#209) | 6.4.2.2, 5.3.2.2 | clear | the constant has the bounds of its subtype, the value is converted (lengths must match, else a bound error): `constant c : string(5 to 7) := "abc"` has `c'left = 5` however it is read; applied where a folded value is attached to the name (sem), where elaboration evaluates a constant (elab) and where lowering evaluates the initialiser (lower) |
| L51 | static checks of array attribute dimensions, indices, slices and deferred constants (#53) | 16.2.3, 8.4, 8.5, 6.4.2.2 | clear | the dimension argument of an array attribute must be locally static (`E1410`, also for a generic) and name an existing dimension (`E1623`); a locally static index or non-null slice outside the prefix's static index range is warning `W1404` at the name (sem.md 4.3: the code may be dead, the run-time check stays; a null slice may have any bounds); a constant without a value (unconstrained or not) outside a package declaration is `E1224`, a code of its own because `E1212` is about signals and variables and its text would not fit a constrained constant (#213); a deferred constant of a package without a full constant declaration with a value in the package body is `E1230` at the deferred declaration (4.8; the full declaration is looked up by name only, its subtype is not compared with the deferred one) |
| L52 | a VHDL-2008 file in a design analyzed with the VHDL-2019 std/ieee (#50) | sem.md 0.1 (user ruling 2026-10-18) | yavs rule, no LRM text | a std/ieee declaration is new in 2019 when the 2008 copy of its package has no item with the same designator and the same profile type marks (or no such package, as `std.reflection`); a 2008 file does not see such declarations through use clauses (no new conflicts or overloads), and naming one (simple, expanded or in a use clause) is `E1007` at the name. `W1008`, once per span, is given for `'left`, `'right`, `'high`, `'low` of an INTEGER subtype whose value lies outside the 32-bit 2008 INTEGER (`natural'high`, not `natural'low`); the suite headers already follow the ruling (warning, not error) |
| L53 | where the implicit `??` of a condition comes from (#48) | 9.2.9 | unclear, **permissive** | a visible `??` for the condition's type first; when none is visible, the `??` declared in the region of the type (so `use ieee.std_logic_1164.std_ulogic` alone makes `if s then` legal, as the reference simulator accepts). Two different visible `??` declarations for the type are `E1341` |
| L54 | a range with one universal_integer bound and one bound of a non-integer type (#48) | 5.2.1, 9.3.6 | clear | implicit conversion of universal_integer is only to an integer type, so the range is of type `integer` and the other bound is `E1300` (`for i in 0 to true`) |
| L55 | type conversions (#48) | 9.3.6 | clear | legal between a type and itself, between abstract numeric types, between array types of the same dimensionality with closely related element types, and in 2019 files (LCS-2016-075) between record types with the same element names whose types are closely related; anything else is `E1350` at the conversion. Incomplete and generic types are not checked |
| L56 | actuals of variable, signal and file parameters (#48) | 4.2.2.1 | clear | the actual is a name of an object of the formal's class (a type conversion of one counts, a dereference is a variable); else `E1360` at the actual. Constant parameters take any expression |
| L57 | protected type misuse and unprotected shared variables (#52) | 5.6.1, 6.4.2.4 | clear | signals, constants and ports of a protected type (or with a protected subelement), an assignment to a protected variable and an operator applied to a protected operand are `E1905` (no predefined assignment or operators; an operator call with a protected operand reports `E1905` instead of `E1301`); a shared variable of a non-protected type is `E1904`, `W1904` under `--relaxed` (R1, sem.md 8.4), the reference simulators need their relaxed flag for such code too, so the open-logic corpus runs yavs with `--relaxed` like uvvm |
| L58 | a negated literal at the i64 minimum, `-9223372036854775808` or `-9223372036854775808 fs` (#214) | 9.3, 5.2.3, 5.2.4 | clear (range of `universal_integer` is implementation-defined), **permissive** | the minus applies to an abstract literal of `universal_integer`, so the literal alone may be one past `i64::MAX` when the unary minus is applied directly to it and the target is an integer or physical type; the folded value is `i64::MIN`. `9223372036854775808` without the minus and `-9223372036854775809` stay `E1402`. The reference simulators (and Questa, 32-bit integers in 2008) reject this literal, so no reference result exists (`ref=hand` in `tests/corpus/scalar/min_literal*`) **Questa 2025.3** (2008 and 2019) rejects both the integer and the `fs` literal (vcom-119, "Integer value exceeds INTEGER'high"; 32-bit integers). |
| L60 | typing and staticness of 2019 conditional expressions and `unaffected` (#33) | 9.1, 9.4, 10.5.2.1, 10.6.2.1, 10.14 (2019) | typing clear, staticness unclear, **permissive** | every alternative is of the type the context gives (each checked there, so a mismatch is `E1300` at the alternative) and takes the context subtype (`(others => '0') when c else x`); without a context type the alternatives' common types decide. Static, and folded, when every alternative and condition is (the reference simulator folds them too). A value of `unaffected`, and a `return` whose conditions all fail, leave the target and the control flow unchanged **Questa 2025.3**: in 2019 `v := unaffected`, `w := 3 when c else unaffected` and `s <= force unaffected` / `force 9 when c else unaffected` are accepted and leave the target unchanged; in 2008 all are rejected (vcom-1441). |
| L61 | a subprogram header with a generic list and a generic map, `function f generic (type t) generic map (t => integer) parameter (v : t) return t` (#227) | 4.2.1, 4.4 | clear (the reference simulators, the third too, treat the subprogram as uninstantiated and reject its calls) | the declaration declares an instance of the subprogram it describes (cloned as for `is new`, mangled with the instantiated profile); the uninstantiated subprogram is not declared; a later body with the same header completes it. A type, subprogram or package interface in a parameter list is `E1232` (4.2.2.1; the reference simulator accepts subprogram and package interfaces there) |
| L62 | where a call of a function with a named return value (`return r of t`) gets its result subtype (#33) | 4.2.1 (2019), LCS-2016-072b | unclear, **permissive** | from the fully constrained subtype the context gives the call: an assignment target, an initial value, a `return` (inside a named-return function: that function's result subtype), an actual of a constrained formal, a qualified expression. Elsewhere `E1304` at the call. The reference simulator accepts only assignments and initial values (it rejects `return` and actuals); the third reference simulator does not accept named return values. Only array result types for now (`E9999` otherwise) |
| L63 | generic types, subprograms and packages of components and blocks (#222) | 7.3.3, 6.5.6.2, 6.5.7.2, 11.2 | clear; formal subprograms as actuals unclear, **permissive** | a default binding maps a component's generic type, subprogram or package to the entity's of the same name; a constant or port of the component's generic type matches one of the entity's of that name, a subprogram matches with the same profile under that correspondence, a formal package with the same generic package; else `E1703`. A generic subprogram of a component or a block (or of an enclosing entity) is accepted as an actual in a generic map; the reference simulators reject it |
| L64 | a package and its body are one declarative region (#232) | 4.8, 12.1, 12.3 | clear | a declaration in the body that is a homograph of an explicit one in the package is `E1101` (constants, signals, types, subprograms of the same profile) with the first declaration as secondary label. Not homographs: the full declaration of a deferred constant, the body of a declared subprogram (a completion), an explicit declaration over an implicit operation of a package type (explicit hides implicit) |
| L65 | gaps and repeats in the individual association of an unconstrained array formal (#224) | 6.5.7.1 | clear; both reference simulators reject | `E1311` at the first association after a gap (note names the missing indices), `E1310` at an association that overlaps an earlier whole element. Checked in `partial` (calls and port maps) for one-dimensional formals with locally static integer or enumeration indices and ranges; constrained formals, multidimensional formals and non-folded bounds are not checked **Questa 2025.3** rejects a gap in a port map too (vcom-1182, "index range (4 to 9) implies 6 elements; only 2 were associated"), in 2008 and 2019. |
| L66 | conversions in subprogram call associations, `seti(to_r(x) => r)`, `inc(to_r(x) => to_i(r))` (#229) | 6.5.7.1, 4.2.2.2, 4.2.2.3 | clear (one reference simulator rejects them as not supported; the other agrees except that it ignores both conversions of composite formals) | a formal conversion only for `out` and `inout` variable parameters with an actual, an actual conversion (`conv(name)` of an object) only where data flows in, so for `out` never and for `inout` only together with a formal conversion, no conversion of a signal parameter (`E1313`); an actual conversion of an `in` variable parameter is legal (the second reference simulator accepts, the first does not). A conversion on an unconstrained formal is `E1312` as in port maps (F8). Overload resolution resolves the formal part per candidate, in a region holding that candidate's formal (L42) |
| L67 | initial values of implicit signals: does `S'transaction` toggle in the first cycles, where does `S'delayed(T)` start (#241) | 14.7.5.2, 16.2.3 | clear (the second reference simulator's initialisation comments quote the LRM: each signal's value "is assumed to have been the value of the signal for an infinite length of time prior to the start of the simulation"; `S'Stable(T)` and `S'Quiet(T)` are set to TRUE and `S'Delayed(T)` to the initial value of its prefix); the first and second reference simulators toggle `'transaction` in delta 1 of time 0, the first starts `'delayed(T)` at the type's default until `T` has passed; the third agrees with the LRM | no signal is active at initialisation, so `S'transaction` keeps its initial `'0'` until the first transaction of `S`; `S'stable(T)` and `S'quiet(T)` start TRUE, `S'delayed(T)` at the prefix's initial value (corpus `sigassign/transaction_init`, `ref=hand`) |
| P1 | parser: "every `end <keyword>` optional in 2019" (`decisions.md`) | 6.8 (2019), LCS-2016-055a | **corrected**, clear | only `end component` loses its keyword; every other ending keeps the 2008 rule (the reference simulators agree); implemented |
| P2 | parser: positions of 2019 conditional expressions (`decisions.md`) | 9.1, 6.4.2, 6.5.7.1, 10.5.2, 10.6.2, 10.14 (2019) | **corrected**; interface defaults unclear, **permissive** | add the parenthesized primary; interface defaults accept the bare and the parenthesized form (U-5, decided); implemented |

## Items

### Q1: universal operands and implicit conversion (9.3.6)

LRM (the second reference simulator quote of LRM-93 7.3.5, the same text is 2008/2019 9.3.6): "An implicit conversion of a
convertible universal operand is applied if and only if the innermost complete context determines a
unique (numeric) target type for the implicit conversion, and there is no legal interpretation of
this context without this conversion." Recalled from the same clause: only a numeric literal, an
attribute of universal type, or the division of two values of the same physical type is a
*convertible universal operand*.

Tools (`q1a.vhd`, an architecture that declares `function "+"(l, r : integer) return integer`
returning 100):

| Expression | LRM | The second reference simulator | The reference simulator |
|---|---|---|---|
| `constant B : boolean := 2**40 / 2**20 = 2**20;` | universal ops, `true` | `true` | `true` |
| `constant B2 : boolean := (2 + 3) = 5;` | universal `+` and `=`, `true` | `true` | **`false`** (user integer `+`) |
| `constant I : integer := 2 + 3;` | integer `+` (result of `+` is not convertible), 100 | 100 | 100 |
| `x := 2**40 / 2**20;` (`q1b`, 32-bit INTEGER) | integer `**`, overflow | overflow error | 1048576 (folds universally) |

Rule for yavs: an interpretation with fewer implicit conversions never loses to one with more;
precisely, if any interpretation of the complete context needs no implicit conversion, every
interpretation that needs one is dropped; conversions are inserted only at convertible operands and
only where the context gives a unique numeric type. This is the second reference simulator's behaviour; the reference simulator deviates. The old
recommendation ("prefer conversion at the leaves") chose B2 = `false` like the reference simulator and is replaced.

### Q1a: named numbers

`constant N := 8;` is a syntax error in the reference simulators (2008 and 2019): a VHDL constant declaration
always has a subtype indication (6.4.2.2). VHDL has no named numbers; universal values come only
from literals, attributes and universal operators. `sem.md` 2.8 said otherwise.

### Q2: base range of an integer type (5.2.3.1)

LRM (recalled): the base type of `type T is range L to R` is an anonymous type "whose range is
selected by the implementation" and that wholly contains `L to R`. So any choice is legal.

Tools (`q2b.vhd`, `q2c.vhd`, `type small is range 0 to 10`):

| Probe | The second reference simulator 08/19 | The reference simulator 2008/2019 |
|---|---|---|
| `small'base'high`, `'low` | 10, 0 | 10, 0 |
| `big'base'high` for `range 0 to 2**40` | 1099511627776 | 1099511627776 |
| `r := (s + s) / 2` with `s = 10` | accepted, 10 | fatal: "value 20 outside of SMALL range" |
| `r := s * 100 / 100` | accepted | error: literal 100 outside SMALL |
| `integer'high` | 2147483647 (also `--std=19`) | 2147483647 / 9223372036854775807 |

So neither tool uses "the smallest of 32 or 64 bit": both report the declared range as the base
range. The reference simulator also checks intermediate results against it; the second reference simulator computes in a wider type and checks
only on assignment, which is inconsistent with its own `'BASE'HIGH`. Decided (U-1): the declared
range, intermediate results checked against it, as the reference simulator.

### L1: 2019 reserved words (15.10)

The second reference simulator's 2019 keyword range is exactly `private`, `view`; the reference simulator's lexer marks exactly `VIEW` and
`PRIVATE` as 2019 tokens and warns "PRIVATE is a reserved word in VHDL-2019" in 2008 mode.
Confirmed; `sem.md` 0.1 keeps its rule.

### L2: potentially visible declarations that denote the same entity (12.3, 12.4)

LRM 12.4 c) (the second reference simulator quote): "Potentially visible declarations that have the same designator are not
made directly visible unless each of them is either an enumeration literal specification or the
declaration of a subprogram." 12.3 defines homographs as declarations that "denote different named
entities". 12.4 c) has no alias exception in its text; the second reference simulator comments that the rule "is not written
clearly in the LRM".

Tools (`u2.vhd`, `u2c.vhd`: `p1` declares `constant c` and `type t`, `p2` declares `alias c is
work.p1.c; alias t is work.p1.t;`, both used with `.all`):

| Reference | The second reference simulator | The reference simulator |
|---|---|---|
| `c` (object alias) | error "no declaration for c (due to conflicts)" | error "multiple conflicting visible declarations" |
| `t` (type alias) | accepted | accepted |

Rule for yavs: under 12.4 c), two potentially visible declarations whose non-object-alias targets
are the same named entity count as one (both tools); an object alias and its object, or two object
aliases, conflict like any two declarations.

### L3: `work` in context declarations (13.3)

LRM 2008 13.3 (the second reference simulator quote): "It is an error if a library clause in a context declaration defines the
library logical name WORK, or if a selected name in a use clause or a context reference in a context
declaration has the library logical name WORK as a prefix." Both tools reject both forms in 2008 and
2019 (`u3.vhd`). Confirmed; the clause is 13.3 (context declarations), not 13.4.

### L4: incomplete types in protected method profiles (5.4.2)

`lib/std/2019/reflection.vhdl` (IEEE text) uses `type VALUE_MIRROR;` as the return type of methods
of protected types declared before `type VALUE_MIRROR is access VALUE_MIRROR_PT;`. Both tools reject
the same pattern in user code in 2019 ("invalid use of incomplete type", `u4.vhd`) and also as the
return type of an ordinary function (`u4b.vhd`), so they special-case `std.reflection`. The 2008 rule
(recalled): before the full declaration, the only allowed use is as the type mark of an access type
definition. No 2019 text found that widens it, and the IEEE package itself relies on the wider use,
so the LRM is unclear. Decided (U-2, amended 2026-10-18): accepted in subprogram profiles inside a protected
type declaration, in 2008 and 2019, only in library `std` (the reference simulators reject it in user
code); an incomplete type elsewhere (an object, an ordinary function
profile) stays an error.

### L5: file element types (5.5)

Both tools reject a file of a constrained or unconstrained 2-D array in 2008 and 2019 (`u5.vhd`;
the reference simulator: "array type for file type must be one-dimensional"). Confirmed for both revisions.

### L6: implicit operations of 2019 generic type classes (6.5.3)

The second reference simulator does not parse 2019 type classes. The reference simulator 2019 (`u6.vhd`) accepts `=` on `private`; relational,
`minimum`/`maximum`, `'succ`, `'pos`, `'val`, `'image` on `(<>)`; `+ * mod abs` on `range <>`;
`deallocate`, `=`, `null` on `access`. It rejects `**` on `range <> . <>`, `<` on an array of a
discrete class, and file parameters of a file class: these are gaps in the reference simulator, since the predefined
operations of floating types include `**` (5.2.5) and arrays of discrete elements have relational
operators (9.2.3). The `<` on an array is also a 2019 change: LCS-2016-059a "Relational operators on arrays of
scalars" (title from the LCS index on vhdlwhiz.com) extends the array relational operators from
discrete to any scalar element type, and the reference simulator's parser switches on exactly that for 2019. Rule kept,
with the table read as "the predefined operations that every type of the class has"; the exact list
in 2019 6.5.3 was not read.

### L7 and L12: range records (2019)

`VI#97` (Verific, quoting 1076-2019): "LRM section 16.2 for pre-defined attributes does not mention
anything about 'range_record attribute. Section 16.2.6 mentions about range'record and
range'value." `VI#197` (open): the named `INTEGER_range_record` appears only in the comments of
`std.standard` and the record type should be anonymous. `VI#378` reports that the LRM makes a range
record available when each scalar type is created. `VI#97` also notes that the LRM names the
direction type `RANGE_DIRECTION`, while the Packages repository (and `lib/std/2019/standard.vhdl`)
says `DIRECTION`.

The reference simulator 2019 (`u13_3`, `u14_*`, `u7.vhd`): `integer'range'record` and `integer'range'value` work;
`integer'range_record` is "no attribute"; `INTEGER_RANGE_RECORD` and `RANGE_DIRECTION` are visible,
`DIRECTION` is not; for a user type `t` both `t'range'record` and `T_RANGE_RECORD` fail ("type T
does not have a range record"). The second reference simulator 19 supports none of it.

Rule for yavs: every scalar type (and the universal types) gets an anonymous range record type with
elements `LEFT`, `RIGHT` (of the type) and `DIRECTION`, created with the type; `R'RECORD` denotes it
and `R'VALUE` gives the value, for `R` an attribute name denoting a range. Decided (U-3): the names
`<T>_RANGE_RECORD` exist only for the `std.standard` types; the direction type is `RANGE_DIRECTION`
(LRM 16.3), and `tools/derive_libs.py` patches the IEEE `standard.vhdl` (which says `DIRECTION`)
to match.

### L8: array aggregate elements (9.3.3.3)

LRM 2008 9.3.3.3 (the second reference simulator quotes): "For an aggregate of a one-dimensional array type, each choice shall
specify values of the index type, and the expression of each element association shall be of either
the element type or the type of the aggregate." and "If the type of the expression of an element
association is the type of the aggregate, then either the element association shall be positional
or the choice shall be a discrete range." No tie-break exists. The second reference simulator analyzes an element that is
itself an aggregate with the element type, noting "This is not in the LRM, but otherwise this would
create a lot of ambiguities" (`((fs, "fs "), (ps, "ps "))` for an array of records).

`u8.vhd` (`f` overloaded to return `string` and `sa`, `sa` an array of `string(1 to 2)`, `(f, f)`):
The second reference simulator "type of element is ambiguous"; the reference simulator takes the element type.

Rule for yavs: the aggregate's own type is a candidate only for a positional association or a
discrete-range choice; an element that is an aggregate (or string or bit-string literal whose type
would be the aggregate type only through the context) takes the element type. The LRM is silent on
a remaining tie; decided (permissive): it takes the element type, as the reference simulator, with no `E1323`.

### L9: case on a non-locally-static 1-D character array (10.9)

`u9.vhd`: a `case s is` on an unconstrained `string` parameter with choices `"ab"`, `"cd"` and
`others` analyzes in both tools; a 3-character actual is a run-time error in both (the reference simulator "expression
length 3 does not match case choice length 2"; the second reference simulator "bound check failure"), not the `others` branch.
`u9b.vhd`: choices of different lengths are an analysis error in both. Rule: choices all the same
length (sem error), and elab/M2 check the expression length against it at run time.

### L10: locally static calls (9.4.2)

LRM 2008 9.4.2 e) (the second reference simulator quote): "A function call whose function name denotes an implicitly defined
operation or an operation defined in one of the packages STD_LOGIC_1164, NUMERIC_BIT, NUMERIC_STD,
NUMERIC_BIT_UNSIGNED, or NUMERIC_STD_UNSIGNED in library IEEE and whose actual parameters are each
locally static expressions." `u10.vhd`: the reference simulator accepts `to_unsigned(1, 4)` and `to_slv(3, 4)` as case
choices in 2008 and 2019; the second reference simulator rejects `to_slv` (it has a TODO for the other packages). Confirmed.

### L11: interface subprogram default `is name` (6.5.6.2)

LRM 2008 6.5.6.2 (the second reference simulator quote) on the box form: "there shall be a subprogram directly visible at the
place of the generic association list that has the same designator as the formal and that has a
conforming profile". `u11.vhd`/`u11b.vhd`: generic `function f return integer is g` declared where
`work.pa.g` (returns 1) is visible, instantiated where a local `g` (returns 2) is directly visible:
The reference simulator gives 1; the second reference simulator crashes ("the second reference simulator Bug occurred"). `u11c.vhd` confirms `is <>` looks for `f`, not `g`.
Rule: `is name` is resolved where the formal is declared (ordinary name resolution).

### L13 and L14: `'IMAGE` on objects, `'INDEX` (2019)

The reference simulator 2019: `v'image` for an integer variable works (2008: "prefix of attribute 'IMAGE must be a
type"); `bv'index` and `bv'index(1)` work as subtype marks, and the reference simulator states "INDEX attribute is only
allowed in a type mark"; `ptr'designated_subtype` works. The second reference simulator 19 supports none of them. Confirmed.

### L15: private variables and method aliases (2019 5.6.2)

LRM 2019 5.6.2 (quoted in `VI#237`): "An alias declared within a protected type declaration may make
a method of the private variable visible. It is an error if an alias declared within a protected
type declaration denotes anything other than a method of a protected type." `VI#251`: the grammar
`private_variable_declaration ::= private variable_declaration` admits `private shared variable`,
"which is clearly wrong". The reference simulator 2019 (`u15.vhd`, `u15b.vhd`) accepts private variables of protected
and of non-protected type, accepts method aliases with signatures without bodies for them in the
protected body, and rejects an alias of a constant. Rule: as quoted, plus `E` for `private shared`;
methods made visible by an alias need no body. A call through such an alias calls the method on
the private variable of the object (the reference simulator agrees, `protected/private_variables`
in the difftest corpus).

An alias of a method of any other object (`alias push is sv.push [integer];` for a shared
variable of an architecture or package, or a process variable) is legal by 6.6.3 (a nonobject
alias of a subprogram) and all three reference simulators analyze the declaration. Calls through
it differ: two reference simulators crash (one at analysis, one at run time), the third rejects
the call ("visible only by selection"). Decided (permissive, #237): the alias carries its object
and a call through it runs the method on that object, as `sv.push(..)` would; the object is a
simple or expanded name of an object or object alias, anything else is `E9999`
(`protected/method_alias_objects` in the difftest corpus, `.expected` by hand).

### L16: `'CONVERSE` of `buffer` (2019)

The reference simulator maps `in`↔`out` and keeps `inout` and `buffer`; the second reference simulator maps `in`→`out`, `out`→`in`,
`buffer`→`in`, `inout`→`inout` (both from their mode mapping, not from runs: the second reference simulator 19 does not
analyze views end to end). No LRM text found. Decided (U-4, permissive): `buffer` stays `buffer`
(the reference simulator), so an element of mode `buffer` may be read and driven through the converse view.

### L17: the reference simulator `--relaxed` (R3)

The primary reference simulator's manual, "RELAXED RULES": `--relaxed` downgrades to warnings: impure function called from a pure
function; file object declared in a pure function; default expression of an interface object not
globally static; shared variable not of protected type. It does not cover case choices: `u17.vhd`
(generic `G` as a case choice) is an error in the reference simulator 2008 with and without `--relaxed`, and in the second reference simulator.
So R3 has no reference-simulator precedent; per Q6 it is dropped.

### L18: equality on a 2008 interface type (Q9)

LRM 2008 6.5.3, quoted in `VI#317` and `VI#72`: "The following operations are defined for the
interface type: The basic operations of assignment, allocation, type qualification and type
conversion; The predefined equality (=) and inequality (/=) operators, implicitly declared as
formal generic subprograms immediately following the interface type declaration in the enclosing
interface list." So Q9's "2008 `type T` has no implicit `=`" contradicts the text; the reference simulators
both accept `a = b` and `a /= b` on `generic (type T)` (negative-test agent, and `q9.vhd`).

Which operator the instance uses is where the tools split. `q9.vhd`: `eq(a, b) := a = b` in a
generic package instantiated with `T => unsigned` (`numeric_std` used at the instantiation),
called with `"0001"` and `"001"`:

| | The second reference simulator 08/19 | The reference simulator 2008/2019 |
|---|---|---|
| `pu.eq("0001", "001")` | `false` (the predefined array `=` of `unsigned`) | `true` (`numeric_std."="`) |

The text says "predefined", which reads as the actual type's predefined operator, the second reference simulator result;
`VI#273` (open) asks the same question for the 2019 implicit `<` of an array interface type. Rule
for yavs: implicitly declare `=`/`/=` after every interface type (2008 and 2019 `private`); at
instantiation they denote the actual's predefined `=`/`/=`, matching the existing remap rule of
`sem.md` 5.1. Code that wants `numeric_std."="` declares a formal `function "=" ... is <>`.
Decided (U-6): since the tools give different results, the instantiation gets warning `W1506` when
a user `"="` for the actual type other than the predefined one is visible there.

Effect on yavs: any check that rejects `=` on a 2008 `type T` without an interface `"="` must be
removed, and a negative test that expects such an error must become a positive one.

### L49: an external name as the actual of an `in` port (8.7, 6.5.6.3, 14.7)

`hierarchy/external_port_actual` and `hierarchy/external_port_init`: `m : entity work.mon port
map (i => << signal .top.l.hidden : bit >>)`. Two readings:

- **A signal name actual.** 8.7 makes the external name denote the signal object itself, and
  6.5.7.1 allows a signal name as the actual of a signal port. The formal then gets the actual's
  effective value in the same update phase (14.7.3.3, port propagation without a delta), and
  starts at the actual's initial value.
- **An expression actual.** 6.5.6.3 associates a port whose actual is not a static signal name
  with an anonymous signal driven by an equivalent concurrent assignment `anon <= actual`. 8.1
  does not clearly list an external name among the static names, and the external name is only
  bound during elaboration (14.4.1). The equivalent process runs once at initialisation and
  again on every event of the actual (11.6); its zero-delay transaction matures in the next
  simulation cycle (14.7.5.3), so the formal follows the actual one delta later and starts at
  its own initial value (its default, else `'left` of its subtype).

The reference simulator takes the second reading: events at delta 1 where the actual changes at
delta 0, and with a port default `'1'` and an actual of `'0'` the formal is `'1'` at delta 0 and
`'0'` at delta 1. The second reference simulator follows the first reading (the formal starts at the
actual's value). The third reference simulator was not checked (no licence in that session).
Rule for yavs (orchestrator decision on #186, permissive, rule 2): the second reading. Elab
records the pair in `NetGraph::copies` and builds the equivalent process as a kernel process
instead of a port edge (`docs/decisions.md` "Elaboration"). Revisit if the third reference
simulator agrees with the first reading.

### P1: optional closing keywords (2019)

The LCS index (vhdlwhiz.com "What's new in VHDL-2019") lists LCS-2016-055a as "Optional component
end keyword": "This change makes the `end component;` optional. Now, you can write just `end;`."
A commercial tool vendor's 2019 support list says the same, and nothing else about endings. The reference simulator's parser has exactly
one 2019 ending rule (`require_std(STD_19, "optional end component")`).

Black-box (`~/.cache/rsim-lrmcheck/t/ends/`, each construct closed by a bare `end;`, with a control
file per construct that spells the keyword and passes the reference simulator 2019):

| Ending | The reference simulator 2008 | The reference simulator 2019 | The second reference simulator 19 | The third reference simulator 2025.3 `-2019` |
|---|---|---|---|---|
| `end component` | error | **accepted** | error | **accepted** |
| `end record`, `end units`, `end protected`, `end protected body`, `end view` | error | error | error | error ("expecting RECORD" etc.) |
| `end process`, `end block`, `end generate`, `end if`, `end case`, `end loop`, `end for` (block configuration) | error | error | error | error |

No simulator accepts the other forms and the LCS text is clear, so the LRM rule applies.
Parser change for `crates/yavs-syntax` (implemented; a bare `end;` elsewhere is E0002 with a help
naming LCS-2016-055a, and a 2008 `end;` for a component is E0004): in 2019 mode only the reserved word
`component` after `end` of a component declaration is optional; every other construct requires its
closing keyword exactly as in 2008. (Endings that are already optional in 2008, such as `end
[entity]`, `end [architecture]`, `end [package [body]]`, `end [function]`, are unchanged.)
Accepting the superset breaks the fail-early rule for 12 constructs.

### P2: where 2019 allows conditional expressions

LRM 2019 grammar, as quoted in IEEE issues:

- `primary ::= ... | ( conditional_expression )` and `qualified_expression ::= type_mark ' (
  expression ) | ...` (`VI#385`, open: "`std_logic'((a when b else c))` is legal but
  `std_logic'(a when b else c)` isn't").
- `variable_declaration ::= [ shared ] variable identifier_list : subtype_indication [
  generic_map_aspect ] [ := conditional_expression ] ;` (`VI#251`, 6.4.2.4).
- `simple_variable_assignment ::= target := conditional_or_unaffected_expression ;` and
  `simple_force_assignment ::= target <= force [ force_mode ] conditional_or_unaffected_expression ;`
  (`VI#194`), where `conditional_or_unaffected_expression ::= expression_or_unaffected { when
  condition else expression_or_unaffected } [ when condition ]` and `expression_or_unaffected ::=
  expression | unaffected` (`VI#193`, which also says these two are "the only places this is
  allowed" for the `unaffected` form).
- `selected_expressions ::= { expression when choices , } expression when choices` (`VI#194`): no
  `unaffected` and no conditional form inside selected variable or force assignments.
- From the reference simulator's 2019 grammar comments: `actual_designator ::= [ inertial ] conditional_expression |
  signal_name | ...` (6.5.7.1) and `return_statement ::= [ label : ] return
  conditional_or_unaffected_expression ;` (10.14, LCS-2016-094a "Conditional return statement").
- Constant and signal initial values: LCS-2016-036a "Conditional initial values" (title and example
  only; the grammar line was not seen).

Black-box the reference simulator 2019 (`~/.cache/rsim-lrmcheck/t/cond/`; the second reference simulator 19 accepts none of the 2019 forms):

| Position | The reference simulator 2019 | LRM source |
|---|---|---|
| constant, signal, variable initial value | accepted | LCS-036a, `VI#251` |
| variable assignment, `v := 1 when g else unaffected` | accepted | `VI#194`, `VI#193` |
| force assignment (`s <= force 1 when g else 2`) | accepted (also 2008: conditional force assignment) | `VI#194` |
| `return 1 when x else 2;`, `return when x;` | accepted | 10.14 |
| actuals: function call, procedure call, port map, generic map | accepted | 6.5.7.1 |
| parenthesized primary `3 + (1 when g else 2)`, `integer'((1 when g else 2))`, `if (a when g else b) then` | **rejected** (the reference simulator gap) | `VI#385` |
| unparenthesized in qualified expression, aggregate element, index, `report`, `wait for` | rejected | not in the grammar |
| generic, port and parameter defaults | rejected | not confirmed either way (U-5: accepted, permissive) |

Parser change for `crates/yavs-syntax` (implemented):
1. Add `( conditional_expression )` as a primary in 2019 mode: a parenthesized expression followed
   by `when` is a conditional expression; this covers every expression position, including
   `T'((a when c else b))`.
2. Unparenthesized `a when c else b` stays limited to: constant, signal and variable initial
   values; association actuals (generic, port, parameter maps, including `inertial`); `return`;
   simple variable assignments and simple force assignments.
3. `unaffected` (and the trailing `[ when condition ]` without `else`) only in simple variable
   assignments, simple force assignments and `return`; not in initial values or actuals.
4. Interface defaults: accepted bare and parenthesized (U-5, permissive: no grammar line was seen
   either way).
5. Unparenthesized in a qualified expression is an error with a help to add parentheses.

### L59: an open `in` port of an unconstrained array subtype with a default (6.5.6.3, 14.3.5)

`hierarchy/open_port_default`: `port (p : in bit_vector := "101")` left `open` or unassociated.
The LRM does not say what bounds the port has when no actual supplies them: 6.5.6.3 gives the
port its default as the value of an unassociated `in` port, and 14.3.5 (port elaboration) only
constrains an unconstrained port from its actual. The reference simulators accept it in 2008
and 2019 and give the port the bounds of its default. Rule 2: permissive. yavs takes the bounds
from the default value (`Model::shape_init`, as for 2019 signals, #187). An open unconstrained
`in` port without a default stays an error (`E1311`, sem). Other modes are unchanged (`E2111`).

## Changes to code paths

`crates/yavs-sem` on main (skeleton commit `e7c0491`, read only here) has no base range,
universal preference, aggregate or visibility logic yet. One implemented item is affected:

- **`crates/yavs-sem/src/attr.rs`** maps the name `"range_record"` to `RangeRecord`. There is no
  such attribute [L12]: it must become `record` (2019, prefix a range attribute name, type mark
  only) next to the existing `value`, which then also accepts a range prefix. `record` is a
  reserved word, so the parser must accept it after `'` (as the reference simulator does). The table also needs
  `image` on object prefixes [L13].
- `db.rs` keeps `direction: Option<TypeId>` for the std type; `lib/std/2019/standard.vhdl` now
  says `RANGE_DIRECTION` (patched in `tools/derive_libs.py`, U-3) and `WellKnown` looks it up by
  that name.

Future paths: Q1 (overload selection, S2), Q2 (`Constraint` of integer base types, S1), L2 (12.4
filter), L7 (`std.standard` implicit decls), L8 (aggregate elements); P1/P2 change
`crates/yavs-syntax` (M0 parser, implemented) and `docs/decisions.md` "Parser".

## Decisions (user, 2026-10-18)

Taken under the policy at the top. The options and evidence are in the items above.

- **U-1 (Q2) base range of `type T is range L to R`:** the declared range; intermediate results are
  checked against it (the reference simulator), with a help naming the type when an intermediate overflows.
- **U-2 (L4) incomplete types in protected method profiles:** allowed in subprogram profiles inside a
  protected type declaration, completed later in the same region, in **both** 2008 and 2019,
  **only in library `std`** (`std.reflection` needs it in 2019); user code gets `E1220`.
- **U-3 (L7) range record names:** `<T>_RANGE_RECORD` only for the `std.standard` types; user scalar
  types get only the anonymous record through `'RECORD`. The direction type is `RANGE_DIRECTION`
  (LRM 16.3); `lib/std/2019/standard.vhdl` is patched to match.
- **U-4 (L16) `buffer` under `'CONVERSE`:** stays `buffer` (the reference simulator; permissive: both read and drive are
  allowed).
- **U-5 (P2) conditional expressions as interface defaults:** accepted, both bare
  (`generic (h : integer := 1 when g else 2)`) and parenthesized (permissive).
- **U-6 (L18) which `=` a 2008 `type T` instance uses:** the actual type's predefined `=`/`/=`
  (the LRM says "predefined"), with warning `W1506` at the instantiation when a different user
  `"="` for the actual type is visible there, since the reference simulator would use that one.
- **L8 aggregate element tie:** the element type (the reference simulator); no `E1323` for that case.
