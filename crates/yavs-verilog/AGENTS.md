# AGENTS.md: yavs-verilog

Read the root `AGENTS.md` first. This file is for an agent about to change `crates/yavs-verilog`.

## 1. Purpose and boundaries

- Verilog (IEEE 1364-2005) front end: preprocessor, lexer, parser, module analysis and detection of
  encrypted envelopes and SystemVerilog (M0 of the plan in #776).
- The scheduling domain (#776 step A, `docs/design/verilog-scheduling.md`): the Verilog regions inside one
  serial kernel process.
- Elaboration of the modules VHDL instantiates (#776 step C): parameters, nets and variables, processes compiled to
  the process code of `exec.rs`, lowered to `yavs-ir` process functions (`lower.rs`, step E) that the interpreter's
  `Machine` runs inside the domain through the Verilog runtime of `host.rs`; the 4-state operators are `yavs_logic::v4`
  (`value`).
- Does not: render diagnostics (they are plain data, `Diag`, with codes in `CODES` for the caller to
  register), touch the library manifest or the cache (the caller does, from `Unit`), or decide the
  language of a file for the CLI (`is_verilog_path`, `is_systemverilog_path` are helpers).
- Depends on `yavs-diag` for `suggest` only (did-you-mean), on `yavs-kernel` and `yavs-logic` for the
  domain (`Process`, `Ctx`, `LogicVec`, `v4`), on `yavs-ir` and `yavs-interp` for process code (`Inst::VUn` to
  `Inst::VBack`, `InterpProcess`, `VerilogHost`). Called by `crates/yavs` (`analyze`, and `elab`/`run` through
  `read_unit` and `Design`).

## 2. Map

| File | Responsibility |
|---|---|
| `src/lib.rs` | The API: `analyze`, `Options`, `Unit` (with `setup`), `Diag`, `Span`, `CODES`; per-file driver and E4001; `read_unit` (a file read again under a recorded setup: `Source`), `setup` |
| `value` | `yavs_logic::v4` re-exported: `V4`, the operators of clause 5, `w64` (the same operators on two words, 1 to 64 bits), `logic_planes`/`logic_of_planes` (kernel vectors as planes, word by word) |
| `src/sem.rs` | Expressions of an instance: self-determined types (`Ty`, Table 5-22) and the context-determined build (`build`, 5.4 and 5.5) into `exec::E`; numbers (3.5.1), strings, constants (`konst`), selects |
| `src/exec.rs` | The process code elaboration makes: `E` (fixed-width trees), `Lhs`, `Op` (one process's code), `eval` (constants, and the oracle of the lowering's tests), `$display` formatting (`format` on values, `ArgV`) |
| `src/lower.rs` | `Op` and `E` to IR (`Lowerer`): one process function per distinct body (the ops with the process's own net numbers and the time units as key), `ProcInfo` (function, the domain's net of each of its nets, instance), `Task` (`$display`, `$write`, `$finish` of `Inst::VTask`), `MAX_WIDTH`; values in runs of registers, what lives past a wait in frame slots; tests: random trees against `eval`, sharing |
| `src/host.rs` | `Tables` (runtime, tasks, processes, `%t` unit), `Host` (`yavs_interp::VerilogHost`: net reads and writes as planes, the waits a process registers, the tasks), `body` (a domain process that runs its `InterpProcess` with the domain's `Core` swapped into its host and gives back the `Wait`) |
| `src/elab.rs` | `Design` (every instance of a run; `domain` per build, `prepare` and `runtime`: the program of its processes, `functions`), `Instance`, `Generic`, `ParamValue`, `PortSpec`; `Elab`: parameters, declarations, net aliases, port binding, continuous assignments, `always`/`initial` compiled to `Op`s and lowered as they are made (`push`), the driver check |
| `src/pre.rs` | Preprocessor on bytes: macros, conditionals, `include`, `timescale`, `default_nettype`, `celldefine`, `begin_keywords` (`KwSet`, the lexer reads the set per token), `line`, protected envelopes; output-to-source map (`Pre::span`); `State` persists across files; context digest |
| `src/lex.rs` | Lexer: tokens with offsets into the preprocessed text; keywords, `(*)` and `*)` rules; UDP table body is skipped |
| `src/ast.rs` | The syntax tree (public, in memory only; M1 lowers it) |
| `src/parse.rs`, `src/stmt.rs` | Recursive-descent parser: modules, UDPs, declarations, instances, generate, specify; statements and expressions |
| `src/analysis.rs` | Ports (ANSI and non-ANSI), duplicate declarations per scope (E4203: module, task, function, named block, generate block, specify block), undeclared names under `default_nettype none` (E4201) |
| `src/diag.rs` | `Diag` builders and the code table with `--explain` text |
| `src/runtime.rs` | `Domain` (nets and regs, processes, `always`, `assign`), `Exec` (what a process body reads and writes: `get`, blocking `set`, `nba`, `display`, `finish`), `Wait`, `Edge` and `edge` (Table 9-9), the region loop (`settle`), future work, the event limit |
| `src/bridge.rs` | `Domain::input`/`output` and `impl Process for Domain`: inputs as `TO_X01Z`, the publication queue (`Core::record`), one batch per delta, the waits |
| `examples/vcheck.rs` | Developer tool: analyze files, print diagnostics, `--each` for corpus runs |
| `tests/frontend.rs` | Tests through the public API with sources written to a scratch directory |
| `tests/lrm.rs` | Runs every `tests/verilog/lrm/*.v` check (one LRM rule per file, header with the exact expected diagnostics) and checks that each is a row of `docs/design/lrm-checks.md` |
| `tests/sched.rs` | Scheduling: the kernel behaviour the bridge rests on, the regions, D4 at the boundary, edges, initialisation, the event limit, stop requests, 1 against 4 threads |
| `tests/props.rs` | `proptest` properties: number literals of every base and width, macro chains and argument counts, nested conditionals, damaged real sources (no panic, spans inside the file) |
| `tests/ops.rs` | Every operator of `value` against a reference model of the clause 5 tables (strings of 0 1 x z, big integers), widths 1 to 130; every word form (`w64`) against the `V4` operator, widths 1 to 64, with inputs that hold one x or z bit; kernel vectors of every code as planes |
| `tests/run.rs` | The run-time LRM checks: files of `tests/verilog/lrm` with `// output:` or `// elab:` header lines, elaborated and run with no ports in a kernel of their own |

## 3. Design decisions that bind this module

| Decision | Why | Source |
|---|---|---|
| Diagnostics are data (`code`, severity, message, span, label, notes, help); spans are byte ranges of the original files (`AnalysisResult::files`), also through macros (the call site) and includes | The crate has no renderer or source map; the caller owns both | #776 M0 split |
| The preprocessor works on bytes and keeps an output-to-source map; the lexer and parser see preprocessed text only | Non-UTF-8 comments exist in vendor sources; offsets stay exact | |
| Macros, `timescale` and `default_nettype` persist over the files of one call; each unit records a digest of the state entering its file | 1364 compilation-unit behaviour; reproducible single-file re-analysis | plan 3.7 |
| An envelope (`pragma protect begin_protected`, legacy `protect ... `endprotect or `protected ... `endprotected) is replaced by a marker byte and is never an error at analysis: a module whose header precedes it is recorded as an encrypted unit (ports from the readable header), a file with an envelope outside any module records nothing, and either way one W4007 per file. E4001 is reserved for the error at elaboration (M1) | D8 and the coordinator's ruling: record at analysis, error when instantiated | D8 |
| `pragma protect begin/end` plaintext markers warn per marker (W4009) and the plaintext is compiled | The plaintext is readable | plan 5 |
| SystemVerilog words are ordinary identifiers to the lexer; the parser reports E4003 naming the word where it appears as a construct (item or statement start, port or parameter type, `++`, `+=`, `.*`, `end : label`, time literals) | `logic`, `ref`, `bit` are legal 1364 identifiers elsewhere | plan 6 |
| A syntax error abandons the module it is in (skip to `endmodule`); a file with an error yields no units; the first failing file stops the batch (`stopped_at`) | No guessed AST, no cascades | AGENTS.md rule 1 |
| `$error`, `$warning`, `$info` and `$fatal` are accepted as module and generate-block items (`Item::ElabTask`, kept and not executed before M1); any other system task at item level is `E4100` | Deliberate extension beyond 1364-2005: vendor and cell models use them; `docs/design/lrm-checks.md` V4 | |
| `begin ... end` standing alone in a module (inside or outside `generate`) is accepted as an unnamed generate block | Vendor IP (MIG, blk_mem_gen) contains it and the other tools accept it; LRM policy rule 2 | |
| A decimal size of 0, or a number right after `#`, is not glued to a following `'b`/`'d` literal: `#1 'd0` is a delay and an unsized literal | A size is never zero (3.5.1); `` `define DLY #1 `` is used before literals in vendor IP | |
| `*)` closes an attribute only while one is open; `(*)` and `@( *)` are star lists | `@(/*AS*/*)` after comment removal | |
| UDP tables are read as text between `table` and `endtable` (symbols like `(1x)` are not tokens) | Lexing them as Verilog fails | |
| E4201 (undeclared) uses one flat set of every name declared anywhere in the module and is only on under `default_nettype none` | Loose by design: no false errors on vendor code; scoping is M1's job | `ponytail:` ceiling: a name declared in another scope is accepted |
| Specify blocks and timing checks are parsed into the tree and not evaluated; with `XIL_TIMING` defined one W4015 per file says so | D14 | plan 6 |
| A front-end rule of the LRM gets a file in `tests/verilog/lrm` and a row in `docs/design/lrm-checks.md` in the same commit; where the LRM is clear and the front end accepts, the file is pinned with a `// gap:` header and listed as a known gap | Rule 14 and the LRM policy; the VHDL side does the same with `tests/diag/sem` | `lrm-checks.md` "Per-clause checks" |
| `` `include `` takes a quoted name, `<name>` or an argument-less macro that expands to one; a gate has its terminals by position, a fixed number per gate; the variable of a generate loop must be a genvar (`E4204`) | LRM 7.1, 12.4.1; the include forms are the other tools' | `lrm-checks.md` V5, V8 |
| The checks of `tests/verilog/lrm` were audited against the text of IEEE 1364-2005 on 2026-11-07: clause numbers are the standard's, the table in `lrm-checks.md` is generated from the file headers, and the choices where yavs differs from the text are the readings V5 to V14 and the gaps G1 to G11 there. Parser state that limits what is legal (`in_func`, `local`, `in_gen`, `genvars`) is reset in `module()` | 164 of the first checks had a wrong clause number; one error and one position per file | `lrm-checks.md` "Per-clause checks" |
| `macromodule` is `module`; a port of an ANSI header declared again in the body is W4209 (V6), and a unary operator on a unary operator W4208 (V18); the same implicit port twice in a non-ANSI header is legal; a block with declarations is named (W4206 when not, V14); `begin_keywords` sets are read per token | LRM 12.1, 12.3.3, 12.3.4, 9.8.1, 19.11 | `lrm-checks.md` V6, V10, V12, V14 |
| A word of `SV_WORDS` followed by `,`, `;`, `)` or `=` in a port or parameter head is the name (`output int;`, `input bit, b;`, `localparam string = 1;`); the preprocessor skips an escaped identifier up to white space, so a backtick or quote inside one is not read | `int`, `bit` and `string` are legal 1364 identifiers; the ivtest suite and a design from OpenCores use them | `lrm-checks.md` 3.7.1, 3.7.2 |
| An empty port, `module m (a, , b)` and `module n (,)`, is a `NonAnsiPort` with `name` and `expr` both `None` (M1 must keep its position for ordered connections) | Grammar A.1.3 | `lrm-checks.md` 12.3.2 |
| `c1 ? a1 : c2 ? a2 : e` is read in a loop and not by recursion, so a table of a thousand rows is not "nesting too deep" | Generated designs have long chains; the depth cap stays for parentheses and unary operators | `tests/frontend.rs` |
| The formal arguments of a `` `define `` may hold comments and line ends, and a line comment that ends in a backslash inside a macro text continues the macro | Unclear in 19.3.1, the other tools read it (policy rule 2) | `lrm-checks.md` V15 |
| `W4016` for the fault-simulation directives, `E4010` for the Annex D delay modes and `` `uselib ``; `W4017` (once per file) for a value on a port that is not an output variable | They are not in the standard; the first group changes nothing a functional simulation computes, the second does; cell models use the value | `lrm-checks.md` V16, V17 |
| An envelope in the module header skips to the module's `endmodule` like one in the body, so plaintext after it stays in the module | A legacy `` `protect `` can end before the body does | `lrm-checks.md` 28 |
| A non-ANSI `input` or `inout` port declared again as `reg` or `integer` is `E4202`, as `input reg` is | 12.3.9.1 | `lrm-checks.md` 12.3.9.1 |
| Spans of diagnostics are clamped to the file: an unexpected end points at the empty range at the end | A span past the end of the file breaks renderers | `tests/props.rs` |
| `Unit::timescale` is the directive in force or `Options::default_timescale` (W4006 once per file) | D10 | |
| All Verilog is one `Domain`, one serial kernel process; its regions run inside one activation and `SimControl` is untouched | Blocking writes and continuous assignments cost no delta (a clock buffer must not skew); the VHDL cycle stays as it is | `docs/design/verilog-scheduling.md` 1 |
| Output changes go out as ordered batches, one per kernel delta: Active results at `d+1`, each NBA region one batch later (an empty batch 0 still costs its delta); a port written again goes to the batch after its last entry and the cursor follows | D4; no change of a port is lost and the order across ports holds | same, 2; open question: skip empty batches |
| Future NBAs and `#d` resumptions are in the domain's own time map, never kernel transactions | `Waveform::schedule` drops later transactions even for transport | same, 3 |
| `posedge`/`negedge` by Table 9-9 on element 0 (`edge`), inputs read as `TO_X01Z`; a VHDL initial value is no event | 9.7.2; `Ctx::edge` has the VHDL rules | same, 1 and 2 |
| A process that runs more than `EVENT_LIMIT` (10000) times at one time is `E3009`; the loop polls `Ctx::stop_requested` every 1024 activations | The kernel's delta limit cannot see a loop inside one activation; a per-process count does not grow with the design | same, 4; `ponytail:` a dedicated code later |
| Elaboration builds process code of its own (`exec.rs`) and lowers it to IR (`lower.rs`, step E): one program for the design's Verilog, its own `Runtime`; a process is an `InterpProcess` of a function shared by every process of the same body; the domain stays the one scheduler (each process is one of its bodies, `host::body`) | Verilog runs on the interpreter and the compiled tiers like VHDL (maintainer's ruling); the VHDL program, its cache and its optimiser are untouched | `docs/decisions.md` "Verilog processes on the IR" |
| Fast paths: every value of 1 to 64 bits is computed on two words (`value::w64`, `Machine::v64` in the interpreter; no allocation), a wider one with `V4`; a word form that cannot give the `V4` result (a real conversion of a value of 2^53 or more) returns `None` and the instruction takes the `V4` path. Arithmetic, relations and shifts by an amount with an x or z bit give x at once; bitwise operators, `?:` merge, extension, selects and `case` matches compute x and z bit for bit in the same words. Net reads and writes are plane words (`logic_planes`, `logic_of_planes`), never element by element; a blocking write that leaves a net of up to 64 elements as it is does nothing (`host.rs`), and a write of a whole net compares and copies it whole (`Core::write`). Edges stay Table 9-9 per write (`edge`, element 0) | Plan 9: no slow path for Verilog; the step C interpreter allocated a `V4` per operator | `tests/ops.rs` `word_forms_match_the_vectors`, `lower.rs` tests |
| Widths and signs are decided in `sem.rs`; the operators of `value.rs` take operands of one width (shifts and `**`: a self-determined right operand) | 5.4, 5.5: the long tail (R1) stays in one place | same |
| A module's VHDL view (`interface`, `elab.rs`; text in `yavs_lib::Interface`) is made at analysis from the parameters' defaults and the port declarations, and recorded in the unit record; bounds that are constant expressions of integer parameters (the `IF_OPS` operators, `?:`, `$clog2`, local parameters) are kept as expressions, others as `Opaque` | `entity lib.module` is typed by VHDL analysis, which has no Verilog front end | `docs/decisions.md` "`entity lib.module`" |
| A unit's `setup` (options and incoming preprocessor state, `State::encode`) is recorded at analysis; `read_unit` reads the file again under it | Elaboration must read a file as its analysis did, macros of earlier files included | same |
| Processes start in the order continuous assignments, `always`, `initial`; a uwire bit with two drivers is `E4502`; any other net with drivers on one bit, a non-strong driver or a pull is resolved in the domain (`design/verilog-scheduling.md` 3a), an input port the module also drives is `E4501`; a pure copy onto a plain internal net with one driver is an alias | `lrm-checks.md` S2, S7 to S9; step D (`inout`) joins the same driver list | `docs/decisions.md` "Several drivers per net" |
| Every construct elaboration does not implement is `E4501` naming it; an `always` with no timing control is `E4503`; a process that jumps back 2^26 times without waiting is a run-time error | Fail early, no silent subset | same |

Ceilings of the domain (each a `ponytail:` note in the code): `docs/design/verilog-scheduling.md` section 5.

### Fast paths and their fallbacks (#776)

Every Verilog instruction has a two-state path on machine words; the 4-state `V4` operator is the
fallback and the reference (`tests/ops.rs` holds each word form against it, also on inputs with one x or
z bit; `lower.rs` tests hold the lowered code against `exec::eval`; `crates/yavs/tests/verilog_tiers.rs`
holds the compiled tiers against the interpreter). "Interpreter" is `Machine::v64` (`yavs-interp`
`exec/verilog.rs`, the word forms of `yavs_logic::v4::w64`), "compiled" the arms of both code generators
(`lower/verilog.rs` of `yavs-clift` and `yavs-llvm`, the set `yavs_llvm::rt::verilog_inline`).

| Operation | Interpreter, values of up to 64 bits | Compiled code, up to 64 bits | Falls back when |
|---|---|---|---|
| `~ & \| ^ ~^`, `?:` merge, extension, concatenation, `===`, `!==`, `case`/`casez`/`casex` item, truth of a condition | words, x and z bit for bit | in place, no guard | never (wider than 64 bits: `V4`) |
| `+ - *`, unary `-`, `< <= > >=`, `== !=`, `&& \|\| !`, reductions | words; any x or z operand bit gives x at once | in place on operands whose `b` planes are 0 | an operand with an x or z bit (compiled: slow path, the interpreter's word form) |
| `<< >> <<< >>>` | words, x and z of the value shifted with it | in place on known value and amount (amount clamped, 64 or more fills) | x or z in value or amount (compiled) |
| `/ % **`, `$clog2` | words | the interpreter's function (`yavs_rt_verilog`) | never in the interpreter |
| select with a run-time index, index start | words; an unknown or out-of-range start reads x | in place when the select lies inside the value / the index is known | outside the value, unknown index |
| `repeat` count, delays, `$time`, `$realtime`, real conversions | words (a real conversion of a value of 2^53 or more: `V4`) | `yavs_rt_verilog` (real `+ - * /` in place) | value of 2^53 or more |
| net read | plane words of the kernel vector (`logic_planes`), no allocation | `yavs_rt_verilog` | never |
| blocking write | a write of a whole net of up to 64 elements that changes nothing does nothing; else plane words, `Core::write` compares and copies a whole net at once | `yavs_rt_verilog` | a part of a net: element by element in `Core::write` |
| non-blocking write | plane words into the NBA queue | `yavs_rt_verilog` | never |
| `@(...)`, `@*`, `#d` | one registration per item, edges by Table 9-9 on element 0 at each write (`edge`) | `yavs_rt_verilog`, then `Suspend` | never |
| `$display`, `$write`, `$finish` | values from registers, formatted by `exec::format` | `yavs_rt_verilog` | never |

Constants: subtrees that read no net and no time are folded when a process is lowered (`lower::fold`).
Pure copies onto an internal net with one driver are aliases (no process). A process's first activation runs
in the interpreter on every tier, as VHDL's does.

## 4. Commands

```sh
cargo test -p yavs-verilog
cargo clippy -p yavs-verilog --all-targets -- -D warnings
cargo run -p yavs-verilog --example vcheck -- --each --quiet FILE...   # corpus-style run
```

## 5. Pitfalls

- The domain starts every process at initialisation in declaration order: a process that must see an
  initial write has to be declared (and so waiting) before the writer.
- Boundary signals are logic signals (`SignalType::logic`) of the net's width; `Ctx::logic` refuses byte signals.
- Offsets in the AST are into the preprocessed text; map them with `Pre::span`, never use them on the original file.
- `Parser::unexpected` decides between E4003 and E4100 from the token and the one before it; add words to `SV_WORDS` with care (a word that is a legal identifier in 1364 must only count at statement or item start).
