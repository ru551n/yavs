# AGENTS.md: yavs-std

Read the root `AGENTS.md` first. This file is for an agent about to change `crates/yavs-std`.

## 1. Purpose and boundaries

- The IR-independent half of the native `std` packages (the IEEE `std` sources have declarations
  only, so yavs writes the bodies): `'IMAGE`/`TO_STRING` and friends, `'VALUE`, C-style real
  formatting, textio READ/WRITE on a line buffer, TEXT file operations, `std.env`, and
  overflow-checked integer and TIME arithmetic. Pure Rust functions over plain values (`i64`,
  `f64`, Latin-1 `[u8]`, type descriptions).
- Does not: know the IR, registers, heap or kernel. `yavs-interp` (`native.rs`, `textio.rs`)
  marshals VHDL values and calls this crate; NATIVE.md symbol binding lives there.
- Depends on: `jiff` (LOCALTIME). Called by: `yavs-interp`, `yavs-sem` (`value::lex_abstract`
  for decimal literals), `yavs-elab` (`-g` generic overrides through the `'value` parser),
  `yavs` (`--stop-time` through `value::physical_value`), `yavs-dap` (`value::literal` for
  deposits, forces, `setVariable` and value breakpoints).

## 2. Map

| File | Responsibility |
|---|---|
| `src/types.rs` | `EnumType`/`EnumLit`, physical `Unit`s, `Bounds`, `time_type()`, `to_latin1` |
| `src/image.rs` | `'IMAGE`, `TO_STRING`, `TO_HSTRING`/`TO_OSTRING`/`TO_BSTRING` |
| `src/value.rs` | `'VALUE` (full literal syntax), `lex_abstract`/`Abstract`, `physical_value`, `time_literal` (`10ms`, `1.5 ns`: `--stop-time`, the debugger), `literal`/`literal_image` over `LitType`/`Lit` (any value as typed in the debugger: scalars, string and bit string literals, aggregates with named elements, ranges and `others`) |
| `src/printf.rs` | C99 printf for one real (`TO_STRING(real, format)`, `%.15g`, `%a`) |
| `src/textio.rs` | READ/WRITE of every textio type on a line, justify, hread/oread |
| `src/file.rs` | TEXT files: READLINE (CRLF), WRITELINE (LF), ENDFILE, 2019 file ops |
| `src/env.rs` | STOP/FINISH, RESOLUTION_LIMIT, getenv, directories, TIME_RECORD, LOCALTIME, TOOL_* |
| `src/arith.rs` | Checked `add/sub/mul/div/rem/modulo/neg/abs/pow`, `check_range`, `real_to_int`, physical times real, `TimeResolution` |
| `src/error.rs` | `ConvError` (kind, input, span, expected form), `What`, `ErrorKind` |
| `tests/golden.rs` | Differential test against the reference simulator goldens (`golden/sim1-2008.txt`, `sim1-2019.txt`) minus `golden/sim1-deviations.txt` |
| `tests/roundtrip.rs` | proptest: `'value('image(x)) = x`, WRITE-then-READ |
| `golden/gen.py`, `golden/mkdev.py` | Regenerate goldens with the reference simulator (`--sim2` for the second reference simulator; commands from `tools/sims.local.conf`); build the deviation list |

## 3. Design decisions that bind this module

| Decision | Why | Source |
|---|---|---|
| All VHDL text is Latin-1 `[u8]`, one byte per CHARACTER; identifiers keep declared spelling | Matches the runtime STRING layout; rule 8 | `docs/decisions.md` "std value conversion" |
| Output formats follow the reference simulator byte for byte where the LRM leaves them open (`%.15g` reals, `time'image` in fs, ...) | The reference simulator is the difftest reference | `docs/decisions.md` "std value conversion" |
| The LRM wins where the reference simulator is wrong or crashes; each case listed with its reason in `golden/sim1-deviations.txt` | LRM policy | same; `docs/plan.md` "LRM interpretation policy" |
| Real literals (any base) are correctly rounded, ties to even, subnormals included; the reference simulator flushes based subnormals | LRM: nearest value | `docs/decisions.md` "std value conversion" |
| READ of a bit_vector skips white space before each bit (sim1; the LRM is silent, sim2 rejects), a failed read still leaves the line unchanged (#623) | Permissive, rule 2 | `docs/design/lrm-checks.md` L131 |
| Leniencies kept: leading `+`, integer literal for a real, `5ns`, format effectors as whitespace | Accepted by the reference simulators | same |
| `string_len` exposes the decoded length of a string or bit string literal for open composite `T'VALUE` bounds | The runtime needs the length before it can build the parser's array type (#609) | `src/value.rs` |
| READ: failed READ leaves the line unchanged (the second reference simulator), TIME accepts decimals, CRs before LF dropped, WRITELINE writes LF everywhere | Permissive, deterministic across platforms | same |
| env: exact TIME_RECORD arithmetic, errors outside years 1..4095, sorted `DIRECTORY.items`, TOOL_* name yavs | Fail early, determinism | same |
| Checked i64 everywhere; overflow is an error, never wrapped or clamped | `docs/plan.md` rule 7, error motto | `src/arith.rs` |
| Time resolution is a runtime value (`TimeResolution`), never baked in | Rule 7 | `docs/plan.md` rule 7 |
| TIME mirror values in resolution-limit units (user, 2026-10-18) | An external test runner's reflection package | `docs/decisions.md` "Reflection: TIME values in mirrors" |

Module-local decisions go in this section in the same commit; a new the reference simulator deviation goes into
`golden/sim1-deviations.txt` with its reason (root `AGENTS.md` rule 24).

## 4. Invariants and pitfalls

- Never panic on bad input, never default a value: every failure is a `ConvError` or
  `ArithError` that the caller renders.
- Do not edit `golden/sim1-*.txt` by hand; regenerate with `gen.py` (needs the reference simulator, wrap it per root
  `AGENTS.md` rule 15) and review the diff. A deviation needs a reason line.
- Tests that touch the file system use pid-suffixed temp dirs (`yavs-std-file-<pid>`); env tests
  rely on unset variable names `YAVS_STD_SURELY_UNSET_VARIABLE`/`YAVS_TEXTIO_TEST_UNSET_VARIABLE`.
- Known perf debt: a textio read is O(len of the line) (one `drain`). Real conversion is exact in
  every base.

## 5. How to work here

```sh
CARGO_BUILD_JOBS=6 cargo test -p yavs-std          # goldens (no sim1 needed), round trips, unit tests
python3 crates/yavs-std/golden/gen.py              # regenerate goldens with the reference simulator (dev only)
```

- Add a golden case by extending `gen.py`, or a unit test next to the function. A behaviour that
  differs from the reference simulator on purpose gets a deviation row.
- No diagnostic codes: errors are structured values; the caller (interp, sem, elab, CLI) owns the
  code.

## 6. Coordination

- The textio and env runtime glue is in `crates/yavs-interp/src/{native,textio}.rs`; symbols are
  in `lib/NATIVE.md`.
- Open issues (snapshot 2026-10-18, `gh issue list -R ru551n/yavs --label crate:std`): #196 (interp)
  quadratic textio lines; #97
  reflection runtime with resolution-limit TIME mirrors.
