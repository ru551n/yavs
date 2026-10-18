# LRM checks for `sem.md`

Status: done 2026-10-18 for every **(unsure)** item and for Q1/Q2 of `sem.md` section 11; the open questions were decided by the user the same day (section "Decisions").

Clause numbers: IEEE 1076-2008 / 1076-2019 (the same unless both are given).

**Policy** (user, 2026-10-18; `docs/plan.md`, `sem.md` "LRM interpretation policy"): where the LRM
is clear, follow it and fail early; where it is unclear or silent, be permissive (the union of what
NVC, GHDL, Questa and Riviera accept, semantics that let the most designs elaborate), and warn only
where the simulators give different results. Every item that was "still ambiguous" or open as a
question U-n is resolved below under that policy ("Decisions").

Questa Starter 2025.3 (`vcom -2008/-2019`) was added as a third black box after the first pass;
Riviera was not available.

## Method and sources

No copy of the LRM was available. Evidence, in order of weight:

1. **LRM text quoted by others**: sentences of the LRM quoted in IEEE P1076 issues
   (<https://gitlab.com/IEEE-P1076/VHDL-Issues>, cited as `VI#n`) and in comments of the GHDL
   source (`~/git/ghdl` is not used; the quotes are from `~/ghdl/src/vhdl`, cited as "GHDL quote").
   Only the quoted LRM sentences are used, never the code. Quotes are at most two sentences.
2. **IEEE package text**: `lib/std/2019/standard.vhdl` (opensource.ieee.org `vasg/Packages`).
3. **Black-box runs**: tiny VHDL files on NVC 1.23-devel (ef5084a94) `--std=2008/2019` and GHDL
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
| Q2 | `t'base'high` of `type t is range L to R` | 5.2.3.1 | **corrected** (the "as NVC/GHDL" premise is false) | base range = declared range, intermediate results checked (U-1, decided) |
| L1 | 2019 reserved words | 15.10 | confirmed | 2019 adds exactly `private` and `view` |
| L2 | same entity through an alias (12.4 c) | 12.3, 12.4 | **corrected** | no exception for object aliases; non-object aliases of the same entity do not conflict (both tools) |
| L3 | `work` in context declarations | 13.3 (not 13.4) | confirmed, clause corrected | error for `library work`, and for `work.` prefixes in use clauses **and context references** |
| L4 | incomplete types in protected method profiles | 5.4.2 | unclear, **std only** | accept in subprogram profiles inside a protected type declaration, 2008 and 2019, only in library `std` (U-2, amended 2026-10-18); user code: `E1220` |
| L5 | multi-dimensional arrays as file element | 5.5 | confirmed | error in 2008 and 2019 |
| L6 | implicit ops of 2019 generic type classes | 6.5.3 | confirmed in principle, list not verified | an interface type of class C gets exactly the predefined ops every type of class C has |
| L7 | `<T>_RANGE_RECORD` per scalar type | 5.2.1, 16.2.6, 16.3 (2019) | **corrected** | every scalar type has an anonymous range record; the name `<T>_RANGE_RECORD` only for `std.standard` types; direction type `RANGE_DIRECTION` (U-3, decided) |
| L8 | aggregate element of element type or aggregate type | 9.3.3.3 | **corrected**, tie unclear, **permissive** | no LRM tie-break; aggregates and named non-range choices take the element type; a remaining tie takes the element type (NVC), no `E1323` (decided) |
| L9 | case on a non-locally-static 1-D char array | 10.9 | confirmed | all choices the same length (analysis error), expression length checked at run time |
| L10 | locally static calls into ieee packages | 9.4.2 | confirmed | exactly `std_logic_1164`, `numeric_bit`, `numeric_std`, `numeric_bit_unsigned`, `numeric_std_unsigned` |
| L11 | where `is name` of a subprogram default resolves | 6.5.6.2 | confirmed (NVC; GHDL crashes) | resolved at the formal's declaration; `is <>` at the instantiation, by the formal's designator |
| L12 | spelling of the range record attribute | 16.2.6 (2019) | **corrected** | no `'RANGE_RECORD`; it is `R'RECORD` (and `R'VALUE`) with `R` a range attribute name; `R'RECORD` as a type mark implemented (#180), `R'VALUE` and the `std.standard` names `<T>_RANGE_RECORD` not yet |
| L13 | 2019 `'IMAGE` on objects | 16.2 (2019) | confirmed | `o'IMAGE` for an object of scalar type (NVC 2019) |
| L14 | 2019 `'INDEX` | 16.2 (2019) | confirmed | `A'INDEX` / `A'INDEX(n)`, a subtype; only in a type mark; implemented (#174): the index subtype, narrowed to `A'RANGE(n)` for an object or a fully constrained subtype |
| L15 | 2019 private variables and method aliases | 5.6.2 | confirmed | quote below; `private shared variable` is an error |
| L16 | `buffer` under `'CONVERSE` | 6.5.2 / 16.2 (2019) | unclear (tools disagree), **permissive** | `buffer` stays `buffer`: both read and drive allowed (U-4, decided) |
| L17 | NVC `--relaxed` list (R3) | n/a | **corrected** | NVC does not relax case choices; drop R3 |
| L18 | Q9: implicit `=`/`/=` on a 2008 interface type `type T` | 6.5.3 | **corrected**; which `=` is used differs between tools | `=` and `/=` are implicitly declared; they map to the actual's predefined operators, warning `W1506` when NVC would pick a different visible user `"="` (U-6, decided) |
| L19 | access types designating file and protected types | 5.4.1 | 2008 clear; 2019 tools disagree, **permissive** | 2008: `E1221` (NVC, GHDL and Questa reject both); 2019: accepted (NVC and GHDL accept both, Questa 2026.1 rejects both) |
| P1 | parser: "every `end <keyword>` optional in 2019" (`decisions.md`) | 6.8 (2019), LCS-2016-055a | **corrected**, clear | only `end component` loses its keyword; every other ending keeps the 2008 rule (NVC, GHDL, Questa agree); implemented |
| P2 | parser: positions of 2019 conditional expressions (`decisions.md`) | 9.1, 6.4.2, 6.5.7.1, 10.5.2, 10.6.2, 10.14 (2019) | **corrected**; interface defaults unclear, **permissive** | add the parenthesized primary; interface defaults accept the bare and the parenthesized form (U-5, decided); implemented |

## Items

### Q1: universal operands and implicit conversion (9.3.6)

LRM (GHDL quote of LRM-93 7.3.5, the same text is 2008/2019 9.3.6): "An implicit conversion of a
convertible universal operand is applied if and only if the innermost complete context determines a
unique (numeric) target type for the implicit conversion, and there is no legal interpretation of
this context without this conversion." Recalled from the same clause: only a numeric literal, an
attribute of universal type, or the division of two values of the same physical type is a
*convertible universal operand*.

Tools (`q1a.vhd`, an architecture that declares `function "+"(l, r : integer) return integer`
returning 100):

| Expression | LRM | GHDL | NVC |
|---|---|---|---|
| `constant B : boolean := 2**40 / 2**20 = 2**20;` | universal ops, `true` | `true` | `true` |
| `constant B2 : boolean := (2 + 3) = 5;` | universal `+` and `=`, `true` | `true` | **`false`** (user integer `+`) |
| `constant I : integer := 2 + 3;` | integer `+` (result of `+` is not convertible), 100 | 100 | 100 |
| `x := 2**40 / 2**20;` (`q1b`, 32-bit INTEGER) | integer `**`, overflow | overflow error | 1048576 (folds universally) |

Rule for yavs: an interpretation with fewer implicit conversions never loses to one with more;
precisely, if any interpretation of the complete context needs no implicit conversion, every
interpretation that needs one is dropped; conversions are inserted only at convertible operands and
only where the context gives a unique numeric type. This is GHDL's behaviour; NVC deviates. The old
recommendation ("prefer conversion at the leaves") chose B2 = `false` like NVC and is replaced.

### Q1a: named numbers

`constant N := 8;` is a syntax error in NVC and GHDL (2008 and 2019): a VHDL constant declaration
always has a subtype indication (6.4.2.2). VHDL has no named numbers; universal values come only
from literals, attributes and universal operators. `sem.md` 2.8 said otherwise.

### Q2: base range of an integer type (5.2.3.1)

LRM (recalled): the base type of `type T is range L to R` is an anonymous type "whose range is
selected by the implementation" and that wholly contains `L to R`. So any choice is legal.

Tools (`q2b.vhd`, `q2c.vhd`, `type small is range 0 to 10`):

| Probe | GHDL 08/19 | NVC 2008/2019 |
|---|---|---|
| `small'base'high`, `'low` | 10, 0 | 10, 0 |
| `big'base'high` for `range 0 to 2**40` | 1099511627776 | 1099511627776 |
| `r := (s + s) / 2` with `s = 10` | accepted, 10 | fatal: "value 20 outside of SMALL range" |
| `r := s * 100 / 100` | accepted | error: literal 100 outside SMALL |
| `integer'high` | 2147483647 (also `--std=19`) | 2147483647 / 9223372036854775807 |

So neither tool uses "the smallest of 32 or 64 bit": both report the declared range as the base
range. NVC also checks intermediate results against it; GHDL computes in a wider type and checks
only on assignment, which is inconsistent with its own `'BASE'HIGH`. Decided (U-1): the declared
range, intermediate results checked against it, as NVC.

### L1: 2019 reserved words (15.10)

GHDL's 2019 keyword range is exactly `private`, `view`; NVC's lexer marks exactly `VIEW` and
`PRIVATE` as 2019 tokens and warns "PRIVATE is a reserved word in VHDL-2019" in 2008 mode.
Confirmed; `sem.md` 0.1 keeps its rule.

### L2: potentially visible declarations that denote the same entity (12.3, 12.4)

LRM 12.4 c) (GHDL quote): "Potentially visible declarations that have the same designator are not
made directly visible unless each of them is either an enumeration literal specification or the
declaration of a subprogram." 12.3 defines homographs as declarations that "denote different named
entities". 12.4 c) has no alias exception in its text; GHDL comments that the rule "is not written
clearly in the LRM".

Tools (`u2.vhd`, `u2c.vhd`: `p1` declares `constant c` and `type t`, `p2` declares `alias c is
work.p1.c; alias t is work.p1.t;`, both used with `.all`):

| Reference | GHDL | NVC |
|---|---|---|
| `c` (object alias) | error "no declaration for c (due to conflicts)" | error "multiple conflicting visible declarations" |
| `t` (type alias) | accepted | accepted |

Rule for yavs: under 12.4 c), two potentially visible declarations whose non-object-alias targets
are the same named entity count as one (both tools); an object alias and its object, or two object
aliases, conflict like any two declarations.

### L3: `work` in context declarations (13.3)

LRM 2008 13.3 (GHDL quote): "It is an error if a library clause in a context declaration defines the
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
type declaration, in 2008 and 2019, only in library `std` (NVC, GHDL and Questa reject it in user
code); an incomplete type elsewhere (an object, an ordinary function
profile) stays an error.

### L5: file element types (5.5)

Both tools reject a file of a constrained or unconstrained 2-D array in 2008 and 2019 (`u5.vhd`;
NVC: "array type for file type must be one-dimensional"). Confirmed for both revisions.

### L6: implicit operations of 2019 generic type classes (6.5.3)

GHDL does not parse 2019 type classes. NVC 2019 (`u6.vhd`) accepts `=` on `private`; relational,
`minimum`/`maximum`, `'succ`, `'pos`, `'val`, `'image` on `(<>)`; `+ * mod abs` on `range <>`;
`deallocate`, `=`, `null` on `access`. It rejects `**` on `range <> . <>`, `<` on an array of a
discrete class, and file parameters of a file class: these are gaps in NVC, since the predefined
operations of floating types include `**` (5.2.5) and arrays of discrete elements have relational
operators (9.2.3). The `<` on an array is also a 2019 change: LCS-2016-059a "Relational operators on arrays of
scalars" (title from the LCS index on vhdlwhiz.com) extends the array relational operators from
discrete to any scalar element type, and NVC's parser switches on exactly that for 2019. Rule kept,
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

NVC 2019 (`u13_3`, `u14_*`, `u7.vhd`): `integer'range'record` and `integer'range'value` work;
`integer'range_record` is "no attribute"; `INTEGER_RANGE_RECORD` and `RANGE_DIRECTION` are visible,
`DIRECTION` is not; for a user type `t` both `t'range'record` and `T_RANGE_RECORD` fail ("type T
does not have a range record"). GHDL 19 supports none of it.

Rule for yavs: every scalar type (and the universal types) gets an anonymous range record type with
elements `LEFT`, `RIGHT` (of the type) and `DIRECTION`, created with the type; `R'RECORD` denotes it
and `R'VALUE` gives the value, for `R` an attribute name denoting a range. Decided (U-3): the names
`<T>_RANGE_RECORD` exist only for the `std.standard` types; the direction type is `RANGE_DIRECTION`
(LRM 16.3), and `tools/derive_libs.py` patches the IEEE `standard.vhdl` (which says `DIRECTION`)
to match.

### L8: array aggregate elements (9.3.3.3)

LRM 2008 9.3.3.3 (GHDL quotes): "For an aggregate of a one-dimensional array type, each choice shall
specify values of the index type, and the expression of each element association shall be of either
the element type or the type of the aggregate." and "If the type of the expression of an element
association is the type of the aggregate, then either the element association shall be positional
or the choice shall be a discrete range." No tie-break exists. GHDL analyzes an element that is
itself an aggregate with the element type, noting "This is not in the LRM, but otherwise this would
create a lot of ambiguities" (`((fs, "fs "), (ps, "ps "))` for an array of records).

`u8.vhd` (`f` overloaded to return `string` and `sa`, `sa` an array of `string(1 to 2)`, `(f, f)`):
GHDL "type of element is ambiguous"; NVC takes the element type.

Rule for yavs: the aggregate's own type is a candidate only for a positional association or a
discrete-range choice; an element that is an aggregate (or string or bit-string literal whose type
would be the aggregate type only through the context) takes the element type. The LRM is silent on
a remaining tie; decided (permissive): it takes the element type, as NVC, with no `E1323`.

### L9: case on a non-locally-static 1-D character array (10.9)

`u9.vhd`: a `case s is` on an unconstrained `string` parameter with choices `"ab"`, `"cd"` and
`others` analyzes in both tools; a 3-character actual is a run-time error in both (NVC "expression
length 3 does not match case choice length 2"; GHDL "bound check failure"), not the `others` branch.
`u9b.vhd`: choices of different lengths are an analysis error in both. Rule: choices all the same
length (sem error), and elab/M2 check the expression length against it at run time.

### L10: locally static calls (9.4.2)

LRM 2008 9.4.2 e) (GHDL quote): "A function call whose function name denotes an implicitly defined
operation or an operation defined in one of the packages STD_LOGIC_1164, NUMERIC_BIT, NUMERIC_STD,
NUMERIC_BIT_UNSIGNED, or NUMERIC_STD_UNSIGNED in library IEEE and whose actual parameters are each
locally static expressions." `u10.vhd`: NVC accepts `to_unsigned(1, 4)` and `to_slv(3, 4)` as case
choices in 2008 and 2019; GHDL rejects `to_slv` (it has a TODO for the other packages). Confirmed.

### L11: interface subprogram default `is name` (6.5.6.2)

LRM 2008 6.5.6.2 (GHDL quote) on the box form: "there shall be a subprogram directly visible at the
place of the generic association list that has the same designator as the formal and that has a
conforming profile". `u11.vhd`/`u11b.vhd`: generic `function f return integer is g` declared where
`work.pa.g` (returns 1) is visible, instantiated where a local `g` (returns 2) is directly visible:
NVC gives 1; GHDL crashes ("GHDL Bug occurred"). `u11c.vhd` confirms `is <>` looks for `f`, not `g`.
Rule: `is name` is resolved where the formal is declared (ordinary name resolution).

### L13 and L14: `'IMAGE` on objects, `'INDEX` (2019)

NVC 2019: `v'image` for an integer variable works (2008: "prefix of attribute 'IMAGE must be a
type"); `bv'index` and `bv'index(1)` work as subtype marks, and NVC states "INDEX attribute is only
allowed in a type mark"; `ptr'designated_subtype` works. GHDL 19 supports none of them. Confirmed.

### L15: private variables and method aliases (2019 5.6.2)

LRM 2019 5.6.2 (quoted in `VI#237`): "An alias declared within a protected type declaration may make
a method of the private variable visible. It is an error if an alias declared within a protected
type declaration denotes anything other than a method of a protected type." `VI#251`: the grammar
`private_variable_declaration ::= private variable_declaration` admits `private shared variable`,
"which is clearly wrong". NVC 2019 (`u15.vhd`, `u15b.vhd`) accepts private variables of protected
and of non-protected type, accepts method aliases with signatures without bodies for them in the
protected body, and rejects an alias of a constant. Rule: as quoted, plus `E` for `private shared`;
methods made visible by an alias need no body.

### L16: `'CONVERSE` of `buffer` (2019)

NVC maps `in`↔`out` and keeps `inout` and `buffer`; GHDL maps `in`→`out`, `out`→`in`,
`buffer`→`in`, `inout`→`inout` (both from their mode mapping, not from runs: GHDL 19 does not
analyze views end to end). No LRM text found. Decided (U-4, permissive): `buffer` stays `buffer`
(NVC), so an element of mode `buffer` may be read and driven through the converse view.

### L17: NVC `--relaxed` (R3)

`man nvc`, "RELAXED RULES": `--relaxed` downgrades to warnings: impure function called from a pure
function; file object declared in a pure function; default expression of an interface object not
globally static; shared variable not of protected type. It does not cover case choices: `u17.vhd`
(generic `G` as a case choice) is an error in NVC 2008 with and without `--relaxed`, and in GHDL.
So R3 has no NVC precedent; per Q6 it is dropped.

### L18: equality on a 2008 interface type (Q9)

LRM 2008 6.5.3, quoted in `VI#317` and `VI#72`: "The following operations are defined for the
interface type: The basic operations of assignment, allocation, type qualification and type
conversion; The predefined equality (=) and inequality (/=) operators, implicitly declared as
formal generic subprograms immediately following the interface type declaration in the enclosing
interface list." So Q9's "2008 `type T` has no implicit `=`" contradicts the text; NVC and GHDL
both accept `a = b` and `a /= b` on `generic (type T)` (negative-test agent, and `q9.vhd`).

Which operator the instance uses is where the tools split. `q9.vhd`: `eq(a, b) := a = b` in a
generic package instantiated with `T => unsigned` (`numeric_std` used at the instantiation),
called with `"0001"` and `"001"`:

| | GHDL 08/19 | NVC 2008/2019 |
|---|---|---|
| `pu.eq("0001", "001")` | `false` (the predefined array `=` of `unsigned`) | `true` (`numeric_std."="`) |

The text says "predefined", which reads as the actual type's predefined operator, the GHDL result;
`VI#273` (open) asks the same question for the 2019 implicit `<` of an array interface type. Rule
for yavs: implicitly declare `=`/`/=` after every interface type (2008 and 2019 `private`); at
instantiation they denote the actual's predefined `=`/`/=`, matching the existing remap rule of
`sem.md` 5.1. Code that wants `numeric_std."="` declares a formal `function "=" ... is <>`.
Decided (U-6): since the tools give different results, the instantiation gets warning `W1506` when
a user `"="` for the actual type other than the predefined one is visible there.

Effect on yavs: any check that rejects `=` on a 2008 `type T` without an interface `"="` must be
removed, and a negative test that expects such an error must become a positive one.

### P1: optional closing keywords (2019)

The LCS index (vhdlwhiz.com "What's new in VHDL-2019") lists LCS-2016-055a as "Optional component
end keyword": "This change makes the `end component;` optional. Now, you can write just `end;`."
Sigasi's 2019 support list says the same, and nothing else about endings. NVC's parser has exactly
one 2019 ending rule (`require_std(STD_19, "optional end component")`).

Black-box (`~/.cache/rsim-lrmcheck/t/ends/`, each construct closed by a bare `end;`, with a control
file per construct that spells the keyword and passes NVC 2019):

| Ending | NVC 2008 | NVC 2019 | GHDL 19 | Questa 2025.3 `-2019` |
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
- From NVC's 2019 grammar comments: `actual_designator ::= [ inertial ] conditional_expression |
  signal_name | ...` (6.5.7.1) and `return_statement ::= [ label : ] return
  conditional_or_unaffected_expression ;` (10.14, LCS-2016-094a "Conditional return statement").
- Constant and signal initial values: LCS-2016-036a "Conditional initial values" (title and example
  only; the grammar line was not seen).

Black-box NVC 2019 (`~/.cache/rsim-lrmcheck/t/cond/`; GHDL 19 accepts none of the 2019 forms):

| Position | NVC 2019 | LRM source |
|---|---|---|
| constant, signal, variable initial value | accepted | LCS-036a, `VI#251` |
| variable assignment, `v := 1 when g else unaffected` | accepted | `VI#194`, `VI#193` |
| force assignment (`s <= force 1 when g else 2`) | accepted (also 2008: conditional force assignment) | `VI#194` |
| `return 1 when x else 2;`, `return when x;` | accepted | 10.14 |
| actuals: function call, procedure call, port map, generic map | accepted | 6.5.7.1 |
| parenthesized primary `3 + (1 when g else 2)`, `integer'((1 when g else 2))`, `if (a when g else b) then` | **rejected** (NVC gap) | `VI#385` |
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

## Changes to code paths

`crates/yavs-sem` on main (skeleton commit `e7c0491`, read only here) has no base range,
universal preference, aggregate or visibility logic yet. One implemented item is affected:

- **`crates/yavs-sem/src/attr.rs`** maps the name `"range_record"` to `RangeRecord`. There is no
  such attribute [L12]: it must become `record` (2019, prefix a range attribute name, type mark
  only) next to the existing `value`, which then also accepts a range prefix. `record` is a
  reserved word, so the parser must accept it after `'` (as NVC does). The table also needs
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
  checked against it (NVC), with a help naming the type when an intermediate overflows.
- **U-2 (L4) incomplete types in protected method profiles:** allowed in subprogram profiles inside a
  protected type declaration, completed later in the same region, in **both** 2008 and 2019,
  **only in library `std`** (`std.reflection` needs it in 2019); user code gets `E1220`.
- **U-3 (L7) range record names:** `<T>_RANGE_RECORD` only for the `std.standard` types; user scalar
  types get only the anonymous record through `'RECORD`. The direction type is `RANGE_DIRECTION`
  (LRM 16.3); `lib/std/2019/standard.vhdl` is patched to match.
- **U-4 (L16) `buffer` under `'CONVERSE`:** stays `buffer` (NVC; permissive: both read and drive are
  allowed).
- **U-5 (P2) conditional expressions as interface defaults:** accepted, both bare
  (`generic (h : integer := 1 when g else 2)`) and parenthesized (permissive).
- **U-6 (L18) which `=` a 2008 `type T` instance uses:** the actual type's predefined `=`/`/=`
  (the LRM says "predefined"), with warning `W1506` at the instantiation when a different user
  `"="` for the actual type is visible there, since NVC would use that one.
- **L8 aggregate element tie:** the element type (NVC); no `E1323` for that case.
