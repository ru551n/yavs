# VHDL-1993 mode (`--std=93`)

Status: design and implementation, #629 (user decision 2026-10-23: add a VHDL-93 mode). Written
before the code; the "Implemented" column of section 3 is kept current in the same commits.
Checked against the code on 2026-10-28 (see the dated notes in sections 3.3, 5 and 6).

Why: a large third-party processor IP library (the `grlib-1993` corpus and the `grlib_leon3mp` real design) is VHDL-93 code. Two of its packages (`stdio`,
`testlib`) overload `HREAD`, `HWRITE`, `compare` and `check` on both `std_logic_vector` and
`std_ulogic_vector`. In 1993 these are two distinct types; in 2008 `std_logic_vector` is a subtype
of `std_ulogic_vector` and the overloads are homographs (LRM 12.3), which every reference simulator
rejects at 2008. The processor core, the bus controller and the simulation models depend on those
packages, so the `grlib_leon3mp` test cannot run at 2008 and was `excluded` in
`tools/realbench/tests.toml` (it now runs at `--std=93`). The same code runs unmodified on the two reference
simulators in their relaxed 1993 modes (the library's own build flags).

## 1. Scope

| Item | Decision |
|---|---|
| `--std=93` (value `1993` too) | In: `yavs analyze`, `parse`, `elab`, `run`, library manifests (`std = "1993"`) |
| `--std=87` | **Out.** No corpus needs it. 1987 differs from 1993 by a few extra keywords (`xnor`, `sll`..., `shared`, `pure`, `impure`, `group`, `postponed`, `inertial`, `reject`, `literal`, `unaffected`), no `'ascending`/`'path_name` family and the old file declaration form, which 1993 still accepts. A later `--std=87` would be the 1993 mode with a smaller keyword table; nothing here blocks it |
| `--std=02` (VHDL-2002) | **Out.** 2002 only added protected types and a few relaxations; code that needs them is 2008 code |
| A relaxed 1993 mode as another simulator spells it | Not a yavs value. yavs is permissive where the LRM is unclear (rule 2), which covers what that mode relaxes |
| PSL in a 93 file | Not read (the words are not reserved in 1993). Correction 2026-10-28, from probes only (no regression test pins it): without `--psl` a `-- psl` comment is a comment; with `--psl` it is read in a 93 file in part: `-- psl assert always true @ (clk'event);` analyzes (exit 0), an assert that uses `rising_edge` fails only with `E1100` (the function is not in 1993), and only `-- psl default clock is clk'event;` fails with `E0002` |

## 2. Principles

1. **One analyzer, revision gates.** `Std::Vhdl1993` is the lowest value of the existing `Std`
   (`Vhdl1993 < Vhdl2008 < Vhdl2019`). The parser, the sem gates (`since`) and the libraries are
   chosen from it exactly as for 2008 and 2019.
2. **Valid 1993 code must mean what it meant in 1993.** That is the part that is not optional:
   libraries, predefined operations, reserved words, shared variables.
3. **Invalid 1993 code (2008 constructs in a 93 file) is rejected where that is cheap and the LRM
   is clear** (`E0011`, `E1007`), and accepted where the LRM does not clearly forbid it (rule 2).
   Nothing in the corpora needs these rejections, so they are the last tier.
4. **Permissive homograph rule, as the references do with their relaxed flags.** An explicit
   declaration hides an implicit one when both are potentially visible (the 2008 rule, 12.4); the
   references need a relaxed-mode switch for the same effect in 1993, and the library above uses it.
   yavs applies the rule in every mode (35 of its files use `std_logic_unsigned` with it).

## 3. The differences yavs honours

"Ref" columns: what sim1 and sim2 do at 1993 (probes in `tests/discrepancies/` and
`docs/design/simulator-discrepancies.md`; sim3 and sim4 are not available for 1993).

### 3.1 Libraries

| # | Difference | 1993 | Implemented |
|---|---|---|---|
| L1 | `ieee.std_logic_vector` | a type of its own (`array (natural range <>) of std_logic`), not a subtype of `std_ulogic_vector`; resolution is per element | `lib/ieee/1993/std_logic_1164` |
| L2 | `std_logic_1164` content | no `to_string`/`to_hstring`/`to_ostring`, no `read`/`write`/`hread` (those are in `std_logic_textio`), no `??`, no matching operators, no shifts, no `to_01` for unresolved vector types | vendored text |
| L3 | `numeric_std`, `numeric_bit` | the IEEE 1076.3 function set (`unsigned`/`signed` are arrays of `std_logic` / `bit` as types of their own); none of the 2008 additions (`resize` with a size model, `maximum`/`minimum`, `?=`, `to_string`, read/write, scalar operand forms, `to_x01`) | derived from the Apache-2.0 2008 files (`numeric93()` in `tools/derive_libs.py`, #650); the 1995 text is not shipped (restrictive notice) |
| L4 | `numeric_std_unsigned`, `numeric_bit_unsigned`, `fixed_pkg`, `float_pkg`, `fixed_float_types`, the two contexts, `std.env` | do not exist | not shipped in `lib/*/1993` |
| L5 | `std_logic_textio` | the Synopsys package (`read`/`write`/`hread`/`oread`/`hwrite` on `std_logic_vector` and `std_ulogic_vector`); in 2008 it is aliases into `std_logic_1164` | vendored (Synopsys text) |
| L6 | `math_real`, `math_complex`, `std_logic_arith`, `std_logic_unsigned`, `std_logic_signed`, `std_logic_misc`, VITAL 2000 | the same texts as in the 2008 tree; `std_logic_misc` is the 1993 text (with the `Drive`/`Sense` overloads on `std_logic_vector` that the 2008 text comments out) | copies |
| L7 | `std.standard` | no `BOOLEAN_VECTOR`, `INTEGER_VECTOR`, `REAL_VECTOR`, `TIME_VECTOR`; 32-bit `INTEGER`, the same `TIME` as in the 2008 tree (a 64-bit `TIME` is within the 1993 minimum) | `lib/std/1993/standard.vhdl` derived from the 2008 copy |
| L8 | `std.textio` | no `JUSTIFY`, `TEE`, `SREAD`, `BREAD`/`OREAD`/`HREAD` (and their aliases) on `bit_vector`, `SWRITE`/`BWRITE`/`OWRITE`/`HWRITE`, no `WRITE (real, format)` | `lib/std/1993/textio.vhdl` derived |
| L9 | Implicit predefined operations | none of: `MINIMUM`/`MAXIMUM`, `TO_STRING` (and the `TO_*STRING` aliases), `??`, `?=` and the other matching operators, reduction operators and array-scalar logical operators (`bv and '1'`), `RISING_EDGE`/`FALLING_EDGE` for `bit`/`boolean`, file `FLUSH`. A 93 design that declares its own `to_string`, `maximum` or `minimum` must not meet a second visible homograph | `since: V2008` on those implicit specs (`implicit.rs`) |

Provenance and licences are in `lib/SOURCES.md` ("1993 packages"). `std_logic_1164`, `math_real`
and `math_complex` are the unmodified IEEE P1076 WG Apache-2.0 texts of the 1993 era; the Synopsys
and VITAL packages are taken the way the 2008 tree has them. The two `std` files and `numeric_std`/
`numeric_bit` are derived from the VASG sources (`tools/derive_libs.py`): the `std` files by
deleting what 2008 added, like `std/2008` from 2019; the numeric packages by giving `unsigned`/
`signed` the 1076.3 types and deleting what 2008 added, with a few small additions (a `maximum`/
`minimum` of `integer`, `to_01` on `std_ulogic_vector`, `std_match` of `std_logic_vector`, the edge
functions of `bit`), all listed in `lib/SOURCES.md`. The 1995 IEEE 1076.3 text is not shipped: its
notice forbids sale or distribution for profit (#650, user 2026-10-23). A random testbench over all
operators gives the same results with the derived and the 1995 text.

### 3.2 Language

| # | Difference | 1993 | sim1 | sim2 | Implemented |
|---|---|---|---|---|---|
| S1 | Reserved words | `assume assume_guarantee context cover default fairness force parameter property protected release restrict restrict_guarantee sequence strong vmode vprop vunit` are identifiers (1993 13.9; 2008 15.10); `view`/`private` already 2019-only | warns `X is a reserved word in VHDL-2008` and accepts | accepts | `token.rs` |
| S2 | Shared variables | any type: `shared variable v : integer;` is legal (1993 4.3.1.3). 2008 6.4.2.4 requires a protected type | accepts | accepts | no `E1904`/`W1904` in a 93 file |
| S3 | Reading an `out` port or parameter | error (1993 4.3.2) | `cannot read output port` | `port cannot be read` | `E1360` in a 93 file (L138); the attributes that read the value count as reads |
| S4 | `process (all)` | not 1993 | error | error | `E0011` |
| S5 | Conditional and selected assignments in a process; conditional variable assignment | not 1993 | error | error | `E0011` |
| S6 | `elsif`/`else`, `case` generate | not 1993 | error | sim2 accepts | `E0011` |
| S7 | Matching operators, `??`, `case?`/`select?` | `?` is not a character of the language | error | error | lexer: `E0001` |
| S8 | `<<`/`>>` external names | not 1993 | error | error | lexer (two tokens, so a syntax error) |
| S9 | Delimited comments `/* */` | not 1993 (`/` `*`) | warns, accepts | error | lexer: not a comment |
| S10 | Bit string literals `8x"FF"`, `UB`, `SX`, `D` | only `b`, `o`, `x` without length | error | error | lexer |
| S11 | Package generics, package instantiation, subprogram generics | not 1993 | error | error | `E0011` for packages; generic subprograms and interface packages are 2008 syntax and fall out as ordinary syntax errors |
| S12 | Protected types, `context`, `force`/`release`, `default` in generic maps | not 1993 | error | error | the words are identifiers, so a syntax error |
| S13 | Unconstrained/partially constrained element rules, relaxed static rules, `case` choice rules, relaxed port map conversions, `others` in more places | 2008 relaxed them | | | **accepted** (rule 2: the 1993 text does not make valid 1993 code mean anything else). Superseded for a 93 file (user, 2026-10-28, #814): the 1993 restrictions on object aliases of multidimensional arrays, unconstrained element subtypes, `buffer` port associations, packages in declarative parts and array pieces in aggregates are `E1007`; `out` port reads were already `E1360` |
| S14 | Homograph conflict of an implicit and an explicit declaration through two `use` clauses | the 1993 rule does not hide the implicit one; the references need a relaxed-mode switch | error without the flag | error without the flag | yavs always hides the implicit one (same as 2008) |
| S15 | `TIME`, `INTEGER` ranges | 32-bit `INTEGER` is the minimum | | | unchanged from 2008 |
| S16 | Attributes `'subtype`, `'element` | 2008 | | | accepted in 93 (permissive; `since` rows stay 2008 for documentation) |
| S17 | A pure function with an access type result (`E1830`) | no such rule in 1993 (2008 4.2.1) | accepts | accepts | not checked in a 93 file |

### 3.3 How the revisions mix (follows `docs/cli-contract.md` section 2 and `sem.md` 0.1)

| Situation | Behaviour |
|---|---|
| All files of a design closure are 93 | design revision D = 1993: `std` and `ieee` from `lib/{std,ieee}/1993`, `STRING`/`INTEGER` as in the 1993 packages |
| 93 and 2008 (or 2019) files in one closure | D = the newest standard recorded in the closure (as for 2008 and 2019). The 93 file is parsed with 1993 keywords and checked against its own revision F = 1993, but the types come from the 2008/2019 packages: `std_logic_vector` is then a subtype of `std_ulogic_vector`, so 93 code that overloads on both fails with the ordinary homograph error (observed 2026-10-28: `E1101` when the closure is analysed at 2008). A 2008-only library unit named from a 93 file (for example `numeric_std_unsigned`) was written here as `E1007`; correction 2026-10-28: `yavs analyze --std=93` reports `E1002` (unit not found in `ieee`), also when the library already holds 2008 files, and no test or run reached `E1007` for a 93 file |
| `--std=93` given to `run`/`elab` | forces D = 1993 as `--std=2008` forces 2008 |
| The test runner | passes each file's standard; it refuses VHDL-93 projects itself, so nothing changes on that side (cli-contract section 2) |

Both reference simulators refuse to mix standards in one library or elaboration ("depends on 1993
version of STD.STANDARD but conflicting 2008 version has been loaded" for sim1). yavs keeps its
existing rule instead of refusing: a mixed design elaborates with the newest packages (user
decision, 2026-10-23; the same rule as for 2008 with 2019). This is deliberate, not a gap: the
test runner passes each file's own standard and projects mix them freely.
`crates/yavs/tests/vhdl93.rs` (`a_93_file_in_a_2008_design_is_checked_as_93`) pins the behaviour.

## 4. Implementation plan

1. `Std::Vhdl1993`, keyword table, lexer gates, `E0011` and the parser gates; `--std=93` for
   `parse` and every command; manifests record `1993`.
2. Libraries: `lib/std/1993` (derived), `lib/ieee/1993` (vendored), `SHA256SUMS`, `ORDER.md`,
   `update-ieee-libs.sh` keeps the vendored tree, embedded in the binary (`build.rs`).
3. Sem: the `since` baseline is `Vhdl1993` (was `Vhdl2008`); implicit operations new in 2008
   get `since: Vhdl2008`; `E1904` is not given in a 93 file; messages name the file's revision.
   Natives keyed by `std_ulogic_vector` also serve `std_logic_vector`.
4. Tests: unit tests per gate, `tests/corpus/` differential cases that run at 93 against both
   references, the `tests/diag/` snapshots, the library parse/analysis tests for the new trees.
5. Corpora: `grlib-1993` and `vests-1993` analysis manifests (`tools/corpus`), expected lists.
6. `tools/realbench/tests.toml`: `grlib_leon3mp` at `std = "1993"`.

## 5. Not done

- `E1007` for 2008 *items* of a package a 93 file names in a 2008 design (items such as `to_string` of
  `std_logic_1164` are not checked; units named from a 93 file are not found at analysis, `E1002`, see 3.3).
- A native fast path for the 1995 `numeric_std` `std_logic_vector` forms beyond the shared keys.
- Enforcement of S13 (2008 relaxations): done for the five groups the 1993 suite tests (#814, see S13); other static
  and `case` rules that 2008 relaxed are still accepted in 93 mode.
- `--std=87`, `--std=02`: out of scope by decision (section 1); the CLI rejects them with exit status 2.

## 6. Results

See `docs/STATUS.md` "VHDL-1993 mode": `grlib-1993` 454 of 454, `vests-1993` 1936 of 1939 (the three failures are bugs that also exist at
2008: #643, #657, #658), `grlib_leon3mp` runs unmodified.

Correction 2026-10-28: the counts above are from 2026-10-23 and are superseded by `docs/conformance.md` (generated). It lists
`grlib-1993` 454 of 454 and `vests-1993` 1939 of 1939 as analysis corpora, and the 1993 language suite simulated at `--std=93`: 3621 of
3694 (44 not counted), with the failures grouped by issue (#807 to #820). Among them are invalid 1993 programs that yavs accepts
(the `analyzer_failure` group: 61 of 1490 are not rejected with a proper diagnostic, which includes runs that stop with `E2001`, `E9999` or a crash) and the missing S13 enforcement (#814).
Update 2026-10-28 (#808 in part, #809, #811, #813 tc3058, #814, #815, #821, #822, #823, #824): 3651 of 3694; `analyzer_failure` 1457 of 1490, `simulator_failure`
38 of 38, billowitch compliant 1608 of 1612.
`std93/` in `tests/corpus` has 6 testbenches that pass at 1 and 4 threads (observed 2026-10-28), and the `v93-*` cases of
`tests/discrepancies` pass.
