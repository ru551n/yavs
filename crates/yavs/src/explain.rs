//! `yavs --explain CODE`: the long text of a diagnostic code.
//!
//! `yavs-sem` keeps its own table (`yavs_sem::diag::EXPLAIN`); every other stage's codes are here.
//! The test at the bottom scans the sources of all crates for code literals and fails for a code
//! without a text, so a new code cannot be added without an explanation.

const EXPLAIN: &[(&str, &str)] = &[
    // yavs-syntax
    (
        "E0001",
        "The lexer found an invalid character, an unterminated string, character literal or comment, or a malformed number.",
    ),
    (
        "E0002",
        "A token the grammar does not allow at this point; the label says what was expected.",
    ),
    (
        "E0003",
        "The name after `end` does not repeat the name of the construct it closes.",
    ),
    (
        "E0004",
        "A VHDL-2019 construct was used in a VHDL-2008 file. Analyze with `--std=2019` or rewrite it.",
    ),
    // E0005 (PSL is not supported) is retired: PSL is skipped with W0004. Never reuse it.
    (
        "E0006",
        "Constructs are nested deeper than the parser's limit.",
    ),
    (
        "E0007",
        "A conditional analysis directive (`` `if ``, `` `elsif ``, `` `else ``, `` `end ``) is malformed or unbalanced.",
    ),
    (
        "E0008",
        "A `` `error `` conditional analysis directive was reached; the message is the one in the directive.",
    ),
    (
        "E0009",
        "A literal or identifier contains a character outside ISO-8859-1, which VHDL does not allow.",
    ),
    (
        "E0010",
        "The file is not valid UTF-8 and `--encoding=utf8` was given. Use `--encoding=auto` or `latin1`.",
    ),
    (
        "E0011",
        "A VHDL-2008 construct was used in a VHDL-1993 file. Analyze with `--std=2008` or rewrite it.",
    ),
    (
        "E0012",
        "The file holds an encrypted (IEEE 1735 protected) envelope, `` `protect begin_protected `` \
         or `` `pragma protect begin_protected ``. yavs never decrypts protected IP, so it stops \
         reading the file at the envelope and reports this one error. Analyze the vendor's \
         unencrypted simulation model instead, or simulate the block with a tool that holds the \
         vendor's key.",
    ),
    (
        "E0013",
        "A PSL property is outside the simple subset of PSL that VHDL-2008 incorporates (IEEE \
         1076-2008 1.3.5, IEEE 1850 4.4.4). yavs rejects an `or` whose operands are both \
         properties (`(a -> next b) or (a -> next c)`): the left operand must be a Boolean. Put \
         the Boolean on the left, or write two directives.",
    ),
    (
        "W0001",
        "A `` `warning `` conditional analysis directive was reached; the message is the one in the directive.",
    ),
    (
        "W0002",
        "A tool directive that yavs does not know was ignored.",
    ),
    (
        "W0003",
        "A number is directly followed by an identifier, as in `10ns`; write a space between them.",
    ),
    (
        "W0004",
        "A design unit contains PSL (IEEE 1850) outside the subset yavs runs. yavs runs clocked `assert`, `assume` and `cover` over booleans, sequences and the weak temporal operators, with declared sequences and properties, written as VHDL-2008 statements or in `-- psl` comments (the user guide's PSL page lists the subset). Everything else (strong operators, `forall`, unclocked directives, vunit/vmode/vprop units) is skipped, so that property is not checked and the simulation is otherwise unaffected. Directives for formal tools only (`restrict`, `restrict_guarantee`, `assume_guarantee`, `fairness`, `strong fairness`, and any directive that calls `nondet` or `nondet_vector`) are skipped without this warning. One warning is given per design unit. Use `-Werror=W0004` to make it an error, or `-Wno-W0004` to silence it.",
    ),
    (
        "W0005",
        "A design file declares the same library unit twice (two architectures `a` of entity \
         `e`, or two primary units of one name). The units of a file are analysed in textual \
         order (LRM 13.1), so the second replaces the first in the library, and a unit between \
         the two that names it was analysed against the first and is then obsolete (LRM 13.5). \
         yavs runs the second copy and analyses the units between against it. Remove or rename \
         one of the two.",
    ),
    // yavs command line
    (
        "E0100",
        "A source file named on the command line cannot be read.",
    ),
    (
        "E0101",
        "A response file (`@FILE`) cannot be read or is not valid UTF-8.",
    ),
    (
        "E0102",
        "`std` and `ieee` are provided by yavs and cannot be mapped with `--work` or `--map` (except `yavs analyze --bootstrap`).",
    ),
    (
        "E0103",
        "An unsupported `-W` option. Use `-Werror`, `-Werror=CODE`, `-Wno-CODE` or `-WCODE` with a warning code such as `W1008`.",
    ),
    (
        "E0104",
        "The top level named on the command line (`ENTITY`, `ENTITY.ARCH` or a configuration, with no library prefix) is not in the work library. The note says what was looked for and the help what the library holds; an empty or missing library directory is a valid empty library, so the cause may be a wrong `--work` path or a file not analyzed yet.",
    ),
    ("E0105", "`--stop-time` is not a valid time such as `10ms`."),
    (
        "E0120",
        "`yavs run --tier=...` names a compiled tier that this yavs was built without. `clift`, `clift-auto` and, in a build without LLVM, `auto` need the `clift` cargo feature (Cranelift, which the release archives are built with); `llvm` and `auto` in a build without Cranelift need the `llvm` feature (LLVM 21); `auto3` needs both. Use a tier the build has, or `--tier=interp`, which every build has. The user guide page \"Running faster\" describes the tiers: https://yavs.readthedocs.io/en/latest/running-faster.html",
    ),
    (
        "E0121",
        "A compiled tier could not compile the design for `--tier=clift` or `--tier=llvm`, or could not start compiling for `--tier=auto`. This is a bug in yavs; `--tier=interp` runs the design.",
    ),
    (
        "E0123",
        "`yavs cache clean` could not remove a compilation cache entry of a library directory (permissions, or the directory is in use by a read-only mount). Entries are only ever an optimisation: the next run analyzes from source.",
    ),
    (
        "E0124",
        "`yavs run --profile=FILE` could not write the profile file (missing directory, permissions, full disk). The tables on standard error are printed anyway; the run exits at least 1.",
    ),
    (
        "E0125",
        "`yavs run --profile` was combined with a debug session (`--dap`, `--interactive`, `--do`). The profiler samples which process is running; a stop at a breakpoint would be counted as time in the stopped process. Profile a run without them. Exit status 2.",
    ),
    (
        "E0126",
        "The directory of the std/ieee units shared by all libraries cannot be used: `--std-cache` or `YAVS_STD_CACHE` is empty, no user cache directory is known, or (for `yavs analyze`) the directory cannot be created or written. Name a writable directory with `--std-cache` or `YAVS_STD_CACHE`, or use `off` to keep the std/ieee units in each library. Exit status 2.",
    ),
    (
        "E0127",
        "`yavs run --tier=llvm` could not load the LLVM 21 shared library (Linux). It is looked for in `lib/` next to the yavs executable (the release archives with the compiled tier keep it there), then through the dynamic loader, then at the path the build used; the note lists what was tried. Keep the `lib` directory with `yavs`, install LLVM 21, or use `--tier=interp`. With `--tier=auto` the same problem is a warning and the hot code stays interpreted. Exit status 2.",
    ),
    (
        "E0128",
        "`--threads` is above 1 and `YAVS_TOKENS` names a token server that yavs cannot use: the value is not `<port>:<secret>`, or the server did not accept the secret (often a `YAVS_TOKENS` left over from a test runner that has ended). The run stops before analysis (exit 2). Unset `YAVS_TOKENS` to run without a token server. A server that cannot be reached is warning `W0118`.",
    ),
    (
        "W0118",
        "`--threads` is above 1 and the token server named in `YAVS_TOKENS` cannot be reached (nothing listens on that port of 127.0.0.1, it does not answer, or the connection broke during the run; the note says which). Tokens only speed a run up, so the run goes on with one thread and its output is the same. A value that is not `<port>:<secret>` or a secret the server does not accept is error `E0128`.",
    ),
    (
        "E0141",
        "An option of Verilog files was given and no file of the call is Verilog, or a value of one is malformed: `-I`, `--timescale` and `-D NAME` (a definition without a value) apply to Verilog files only, and `--timescale` takes `UNIT/PRECISION`, each 1, 10 or 100 of s, ms, us, ns, ps or fs with the precision no coarser than the unit. Verilog files end in `.v` or `.vh`; `--lang=verilog` takes any file as Verilog.",
    ),
    (
        "E0199",
        "A command option or combination is not implemented yet. The message names the option or combination; the help line says what to run instead. This is a limitation of yavs, not an internal compiler error. Exit status 2.",
    ),
    // yavs-wave
    (
        "E0801",
        "The waveform file given to `--wave` cannot be created or written (missing directory, permissions, full disk).",
    ),
    (
        "W0802",
        "An array or vector has more elements than `--wave-max-array` and is left out of the waveform.",
    ),
    (
        "E0803",
        "The Ctrl-C handler for `--wave` cannot be installed.",
    ),
    (
        "E0106",
        "A `--load` library cannot be loaded: the file is missing, has the wrong architecture, or one of its own dependencies cannot be found.",
    ),
    (
        "W0108",
        "A VHPIDIRECT foreign subprogram's symbol is defined in no `--load` library and not in the executable, or a `\"VHPI library model\"` subprogram's model was registered by no `--vhpi` plugin. The run goes on; calling the subprogram is `E0107`.",
    ),
    (
        "E0107",
        "A foreign subprogram cannot be bound or called: its VHPIDIRECT C signature cannot be called, its VHPI model is registered as the other kind of subprogram, or (at the call) no `--load` library and not the executable defines its symbol or no `--vhpi` plugin registered its model.",
    ),
    (
        "W0109",
        "`--threads` is ignored with `--dap`, `--interactive` and `--do`: a debug session stops inside processes, which needs the simulation on one thread. The run is the same as with `--threads=1`.",
    ),
    // yavs-dap
    (
        "E0701",
        "The Debug Adapter Protocol connection (`--dap`) cannot be opened: the port is in use or not allowed, or accepting the client failed.",
    ),
    (
        "E0130",
        "`--surfer` cannot use Surfer: `surfer` is not on PATH, it ended or did not connect within 30 seconds, the address \
         does not answer, it refuses the greeting, or it does not load the file. Install Surfer 0.6.0 or later on PATH. With \
         `--surfer=HOST:PORT`, start its WCP server (`wcp_server_start`, or `[wcp] autostart = true` in its configuration; \
         the default address is 127.0.0.1:54321).",
    ),
    (
        "W0132",
        "A `files` pattern in a `yavs lint` waiver file covers no file of this run and no VHDL file under the waiver file: it is relative to that file. The run goes on, as a shared waiver file may name generated files that do not exist yet.",
    ),
    (
        "W0131",
        "The connection to Surfer ended during the run, or Surfer did not answer at the end of the run. A connection that \
         ended means Surfer was closed or crashed: the viewer stops following, and the run must start again to show it. \
         Surfer handles commands only while it draws its window, so a covered or minimized window may leave it silent: \
         bring the window to the front. Both are also told in the console when they happen. A Surfer that answers again \
         during the run catches up and this warning does not appear. The simulation and its exit status are not affected, \
         and `-W error` does not make this an error.",
    ),
    (
        "E0703",
        "The console hook named in `YAVS_CONSOLE_HOOK` cannot be used: the value is not `<port>:<secret>`, nothing \
         answers on that port within 30 seconds, or what it registers is not valid (a yavs command's name). A test \
         runner sets this variable while it serves the hook; unset it to run without one.",
    ),
    (
        "E0702",
        "The live waves of a `yavs/waveSubscribe` subscription failed while the simulation ran (a value the recorder cannot encode); the note says why. The run fails.",
    ),
    (
        "E0110",
        "A coverage database or report cannot be written (`yavs run --cov-file=FILE`, `yavs cov merge -o FILE`, `yavs cov report --cobertura FILE`): the note gives the operating system's reason.",
    ),
    (
        "E0111",
        "A coverage database given to `yavs cov merge`, `report` or `rank` cannot be read, is not a database `yavs run --cov` wrote, or has a format version this yavs does not read: the note says which, naming both versions. The format is private to yavs: re-run the simulation with this yavs, or report with the yavs that wrote the file.",
    ),
    (
        "E0113",
        "A source file named by the coverage database cannot be read, so `yavs cov report --html DIR` cannot show it; the note gives the reason. Run the command where the paths in the database resolve, or run the simulation with `--cov-embed-sources` so the database holds the text.",
    ),
    (
        "E0114",
        "The total coverage of `yavs cov report` is below the percentage of `--fail-under=PCT`, or a kind is below its own in `--fail-under=KIND:PCT,...`. The normal report was printed first; the message names each total or kind that failed and its requirement. Run more tests, or lower the requirement. Percentages are rounded down, so `--fail-under=100` needs every item covered.",
    ),
    (
        "E0115",
        "`yavs cov report --total` or `--fail-under` has no item to count: the coverage database holds none of the kinds selected with `--kind` or named in `--fail-under=KIND:PCT` (or nothing is left after the exclusions). Run with `--cov-kind` naming the kinds, or change `--kind`.",
    ),
    (
        "E0112",
        "An exclusion file given to `yavs cov report --exclude` cannot be read or has a malformed row or entry (a `.yaml` or `.yml` file is read as YAML, any other as text rows); the note names the line. Rows start with `statement`, `branch`, `condition`, `expression`, `combination`, `toggle`, `instance`, `unit`, `file`, `files`, `files_exclude`, `library`, `entity` or `process`; a YAML entry has the same word as its `kind`. A `library`, `entity` or `process` row over a database written by an older yavs is this error too: such a database does not record those names, so run the simulations again.",
    ),
    (
        "E0140",
        "The coverage options of `yavs run` do not name a coverage kind to count, or contradict each other. No kind is on by default: `--cov=LETTERS` turns coverage on and selects kinds (`s` statement, `b` branch, `c` condition, `e` expression, `t` toggle, `f` fsm, `a` assert, `m` combination), `--cov-kind=LIST` does the same with words. Bare `--cov`, a letter that is none (an old `--cov=FILE` is this error: the database file is `--cov-file=FILE` now), `--cov` together with `--cov-kind`, and `--cov-file` or `--cov-embed-sources` without coverage on are this error. The run stops before analysis (exit status 2).",
    ),
    (
        "E0138",
        "The file of `yavs cov report --config` or `yavs cov rank --config` cannot be read or is not a coverage configuration file. It is YAML with the keys `exclude`, `root` and `kind`; the note names the line. Nothing is ever read as a configuration without this option.",
    ),
    (
        "E0137",
        "`yavs cov merge` or `yavs cov report` was given databases that disagree about an in-source coverage exclusion: a combination or a condition or expression row that a `-- yavs coverage exclude` directive waives in one run is a requirement in another (the other run was made from other sources, or with `--coverage-directives=ignore`). Merging them would let the waiver hide a requirement, so nothing is merged. Run all the databases from the same sources and options.",
    ),
    (
        "E2020",
        "A `-- yavs coverage exclude` directive is malformed: a missing `)`, an input without `=value`, text after the pattern that is not a reason (a reason follows `:`), an unknown word where `condition`, `expression` or `row` may stand, or a `row` without a pattern or an `INPUT = VALUE`. The directive is rejected as a whole, never repaired. Directives are checked at analysis, whether or not coverage is collected; `--coverage-directives=ignore` reads none.",
    ),
    (
        "E2021",
        "A positional exclusion (`-- yavs cov exclude 01-`) has a symbol other than `0` (evaluated false), `1` (evaluated true) or `-` (anything, also not evaluated), or a different number of positions than the expression has inputs. The inputs are the operands of the logic operators of the condition or assignment, left to right in the source; the message lists them. Write `-` for an input that is unrestricted: a pattern is never padded. A `row` pattern has exactly one `0` or `1`: one input at one value.",
    ),
    (
        "E2022",
        "A named exclusion (`(valid=false, ready=true)`) names something that is not exactly one input of the expression: an unknown name, a name that is several inputs, or an input given twice. An input is named by its text in the source, in any formatting: `(valid=false)`, `(count > 3 = false)`, `(f(a, b) = true)`, and so is the input of `row count > 3 = 0`. The list after a pattern, `01 (en, count < 15)`, is checked too: inputs that no longer match in order or text are this error, with what it found and expected and the directive to write. Case in a literal or extended identifier counts; other case and blanks do not. The note lists the inputs; an operand that appears twice can only be addressed by its position. `exclude state NAME` and `exclude transition FROM -> TO` name states of the register's type: a name that is not one is this error, with the states listed and a near name if there is one.",
    ),
    (
        "E2023",
        "A value in a named exclusion is not `true` or `false`.",
    ),
    (
        "E2024",
        "A coverage exclusion has nothing to attach to or cannot be used where it is: it stands after code on its line, or no `if`, `elsif`, `while`, `exit`, `next` or assignment (sequential or concurrent) follows it (comments and blank lines may stand between), the construct has several conditions or choices, `exclude condition` is on an assignment or `exclude expression` on a condition, or the expression has no combinations to exclude (fewer than two inputs, an input that is constant after elaboration, more than 8 inputs, or too many combinations). `exclude state` and `exclude transition` belong right before the `case` on a state register or one of its `when` alternatives: anywhere else, or on a `case` that is no detected state machine, is this error.",
    ),
    (
        "E2025",
        "A `-- yavs coverage` (or `-- yavs cov`) comment has no command or an unknown one. The commands are `exclude`, `off`, `on` and `off-line`. Ordinary comments are never read as directives.",
    ),
    (
        "W2020",
        "A coverage exclusion repeats an earlier one of the same expression or state machine, or is already covered by a broader one. It does nothing more. Remove it, or silence the warning with `-Wno-W2020`.",
    ),
    (
        "W2021",
        "The exclusions of an expression together waive every combination of it, or those of a state machine every state or every transition, so nothing is left to cover. Check that the directives say what was meant.",
    ),
    (
        "W2022",
        "A coverage exclusion matches no combination the expression can show. The right operand of `and`, `or`, `nand` and `nor` on `boolean` and `bit` is not evaluated when the left one decides, so a pattern that gives it a value with the left operand deciding matches nothing; write `-` for it. `std_ulogic` operators evaluate both operands. An `exclude transition` matches nothing when no `case` alternative assigns that transition; if it happens in a run all the same, a reset say, it is excluded there.",
    ),
    (
        "W2023",
        "With `--cov-kind=combination`, a decision or logic expression with two or more inputs has no combinations to measure: an input is constant after elaboration, it has more than 8 inputs, or it has more combinations than the limit. The note says which. Its other coverage is not affected.",
    ),
    (
        "W0110",
        "A row of the exclusion file names a line (`file:line`) that is not where the excluded statement is now. The exclusion still applies, matched by the statement's text; update the line to silence the warning.",
    ),
    (
        "W0111",
        "A row of the exclusion file matches nothing in the coverage database: the statement changed (its text hash with it), was removed, or the instance path is wrong. A row with a `file:line` also names the statement of its kind and instance at or near that line, which probably replaced the excluded one, and the row that would exclude it: that statement is only named, never excluded, so check it and copy the row. A pattern of `files` or `files_exclude` that covers no source file of the database is this warning too.",
    ),
    (
        "W0112",
        "A `-- coverage off` pragma has no `-- coverage on` after it, so it excludes the rest of the file from coverage.",
    ),
    (
        "W0113",
        "A `-- coverage on` pragma has no `-- coverage off` before it.",
    ),
    (
        "W0114",
        "A row of the exclusion file excludes an item that ran in the merged runs (for a `toggle` row: the scalar toggled both ways). It is excluded anyway; check that the exclusion is still wanted.",
    ),
    (
        "W0115",
        "A signal or port has more `bit`/`std_ulogic` scalars than `--cov-toggle-max` (default 65536), so its toggles are not counted. Raise the limit to count them; large memories are usually left out on purpose.",
    ),
    (
        "W0120",
        "With `--cov-kind=fsm`, something looks like a state machine but is not one the coverage measures: a register of an enumeration type that a `case` does not select on, a `case` on a part of a signal, on a port or on a variable of a clocked process. It has no `fsm` items, and the coverage report lists it under \"not measured as state machines\". The other kinds are not affected.",
    ),
    (
        "W0121",
        "Coverage databases given to `yavs cov merge` or `report` embed different text for one source file (`--cov-embed-sources`): the file changed between the runs. Items still merge, since they are identified by their own text; the HTML report shows the last text, so a line number of an older run can be off. Run all databases from the same sources.",
    ),
    (
        "W0116",
        "The signal handlers of a running simulation (Ctrl-C and SIGTERM writing out the buffered output, SIGUSR1 printing a hang report) could not be installed (the note gives the operating system error). The run goes on without them, and its output is written line by line; `--hang-report` is not affected.",
    ),
    (
        "W0117",
        "The compile thread of `--tier=auto` could not be started (the note gives the operating system error). The thread starts a few milliseconds into the run, from the helper thread of the signal handlers. The run goes on interpreted.",
    ),
    // yavs-vhpi
    (
        "E0901",
        "A VHPI plugin (`--vhpi`) cannot be loaded: the file is missing, has the wrong architecture, or imports a `vhpi_*` symbol yavs does not export.",
    ),
    (
        "E0902",
        "A VHPI plugin has neither the named entry point nor a `vhpi_startup_routines` array.",
    ),
    (
        "W0903",
        "A VHPI call was misused (a bad handle, property or argument); the call failed.",
    ),
    ("W0904", "A VHPI call that yavs does not support yet."),
    (
        "E0905",
        "An architecture's `foreign` attribute names a VHPI model (`\"VHPI library model\"`) that no plugin given with `--vhpi` registered with `vhpi_register_foreignf` (`vhpiArchF`); the architecture would have no behaviour.",
    ),
    // yavs-lib
    (
        "L0000",
        "A note from the library layer, such as a lock that is being waited for.",
    ),
    (
        "L0001",
        "A mapped library does not exist. Analyze a file into it first or fix the `NAME:PATH` mapping.",
    ),
    (
        "L0002",
        "A library manifest or record cannot be read, written or resolved.",
    ),
    (
        "L0003",
        "A library manifest (the index `yavs-lib.toml`) or a record of a file in `yavs-lib.d/` is corrupt, damaged or missing. Delete the library directory and analyze its files again.",
    ),
    (
        "L0004",
        "A library manifest has a format version this yavs does not read, or was written before the rename to yavs. Libraries are not converted: delete the library directory and analyze its files again.",
    ),
    (
        "L0010",
        "A library directory cannot be created or is not writable.",
    ),
    ("L0011", "The library lock file could not be taken."),
    ("L0020", "A library mapping is not of the form `NAME:PATH`."),
    ("L0030", "A unit refers to a library that is not mapped."),
    (
        "L0031",
        "A design unit is not in the library it was looked up in.",
    ),
    ("L0032", "Design units depend on each other in a cycle."),
    (
        "L0033",
        "A source file recorded in a library manifest cannot be read.",
    ),
    (
        "L0034",
        "A source file, or a file a Verilog source includes, changed on disk since it was analyzed into the library. Analyze it again.",
    ),
    (
        "L0035",
        "A Verilog module or primitive and a VHDL design unit have the same name (case ignored) in one library. Verilog names keep their case, but a VHDL component that names a module must find exactly one unit, so the two languages may not share a name in a library. Rename one of them, or analyze them into different libraries.",
    ),
    // yavs-lower
    (
        "E2001",
        "The design uses a construct that yavs does not implement yet; the message names it. This is a limitation of yavs, not an error in the design.",
    ),
    // yavs-elab
    (
        "E2101",
        "The top level named on the command line does not exist in the work library or cannot be elaborated.",
    ),
    ("E2102", "An entity has no architecture to elaborate."),
    (
        "E2103",
        "A generic has no value: no actual, no `-g` override and no default.",
    ),
    (
        "E2104",
        "The text of a `-gNAME=VALUE` override is not a value of the generic's type.",
    ),
    (
        "W2105",
        "A `-gNAME=VALUE` override names a generic the top level does not have.",
    ),
    (
        "E2106",
        "An expression that must be evaluated during elaboration cannot be.",
    ),
    (
        "E2107",
        "Evaluation during elaboration failed: overflow, division by zero or a value out of range.",
    ),
    (
        "W2108",
        "A component instance is not bound to an entity (LRM 7.3.3) and stays open.",
    ),
    (
        "E2109",
        "Instances are nested deeper than the limit; the design instantiates itself recursively.",
    ),
    (
        "E2110",
        "A port actual does not match its formal in shape or length.",
    ),
    (
        "E2111",
        "A port of an unconstrained subtype has nothing to take its bounds from.",
    ),
    (
        "E2112",
        "An unresolved signal has several sources (LRM 14.7.2).",
    ),
    (
        "E2113",
        "A static name selects elements outside its prefix.",
    ),
    (
        "E2114",
        "A subprogram is called before its body is elaborated (LRM 14.4.2).",
    ),
    (
        "E2115",
        "A generic value is outside the generic's subtype, or a part of it has another length.",
    ),
    (
        "E2116",
        "A package needs a body (subprograms, deferred constants) and has none (LRM 4.8).",
    ),
    (
        "E2117",
        "An external name names no elaborated object (LRM 8.7, 14.4.1).",
    ),
    (
        "E2119",
        "A top-level entity has a variable port (VHDL-2019): a variable port denotes its actual's protected object, and the top level has no actual (LRM 6.5.6.3).",
    ),
    (
        "E2120",
        "A TIME physical literal or unit name has a unit below the resolution limit selected with `yavs run --resolution` and appears in the design hierarchy, in a package it uses or in code that never runs (LRM 5.2.4.2). A literal whose abstract literal is zero (`0 fs`) is legal, and so is a value such as `0.4 ns` under `ns`: the rule is about the unit, not the value.",
    ),
    (
        "E2121",
        "An external name's subtype indication does not match the object it names: the type differs, or a constrained array dimension has another length (LRM 8.7).",
    ),
    (
        "W2118",
        "A constant whose initial value calls an impure function cannot be evaluated during elaboration yet; it is evaluated when the simulation starts, so its calls run after the elaboration-time calls instead of in declaration order.",
    ),
    (
        "E2199",
        "A feature needed during elaboration is not implemented yet. The message names the limitation and its location in the design. This is a limitation of yavs, not an internal compiler error.",
    ),
    // yavs-kernel runtime errors
    (
        "E3001",
        "A run-for time, `now + delay`, is beyond `TIME'HIGH`. (A wait timeout or a transaction past `TIME'HIGH` is not an error: it never matures.)",
    ),
    (
        "E3002",
        "More delta cycles than the limit ran at one simulated time, usually a zero-delay feedback loop. The labels and the note name the processes of the loop and the signals through which they wake each other, and label the declaration of each signal still changing: two processes (or concurrent assignments) that assign each other's inputs with no delay, a process that waits only on a signal it assigns, a combinational loop through concurrent statements, or a process that waits for 0 ns in a loop. `--max-deltas=N` raises the limit (default 10000) for a design that needs more deltas at one time.",
    ),
    (
        "E0136",
        "The argument layout in `--load=LIB:args=LAYOUT` does not exist: `length` is the default, `fat-pointer` the other one (see the VHPI guide).",
    ),
    (
        "E0133",
        "`yavs lint` was given nothing to check, a path that does not exist, or a rule that does not exist: name files or directories, or give `--work` with an analyzed library.",
    ),
    (
        "E0134",
        "`yavs lint` cannot use its configuration or waiver file: it cannot be read, is not valid YAML, a key it knows has a bad value, or it names a key, rule or rule group `yavs lint` does not know. `yavs lint` is a static analysis tool, not a style guide or formatter, so a file written for one is not accepted as it is. Every such name is listed with its line in one run, with a suggestion when a close name exists, so that one pass fixes the file. A misspelled `files` or `lines` in a waiver would otherwise widen it, and a misspelled rule id would waive nothing.",
    ),
    (
        "E0135",
        "`yavs lint` cannot write a report file, named by `--json`, `--sarif`, `--junit`, `--quality-report` or `--sonarqube`.",
    ),
    (
        "E3003",
        "A postponed process scheduled a delta cycle (LRM 14.7.5.3).",
    ),
    ("E3004", "A malformed signal assignment reached the kernel."),
    ("E3005", "A wait deadline lies before the current time."),
    (
        "E3006",
        "An id or element range that does not exist in the model was used.",
    ),
    ("E3007", "A resolution function failed."),
    (
        "E3008",
        "A port conversion function failed or returned a value of the wrong size.",
    ),
    (
        "E3009",
        "A VHDL runtime error: bound check, length mismatch of an array, record field or element, overflow, null access, division by zero and the like. So is reading the bounds of a deferred constant before its full declaration has been elaborated (a subprogram of the package body that reads `U'length` is called to initialise a constant above the full declaration of `U`: move that constant below it). The message and the call stack say where.",
    ),
    (
        "E3010",
        "The model, the simulation configuration or a control request is invalid.",
    ),
    // yavs-kernel run-time reports (W3101, W3102 are warnings of `yavs run`)
    (
        "W3101",
        "A report of a simulation that did not finish, printed with `--hang-report` when the run ends at `--stop-time` without `stop` or `finish` or runs out of events with a process still waiting, and on SIGUSR1 (Unix) for a running simulation. It gives the time and delta, how many processes wait for a timeout, for signals that can still change, or for signals nothing can change any more, then names the processes that can never run again with what they wait for, and for each such signal its value, its last event and the processes that drive it (\"it has no driver\" is the usual cause). A signal that an edge-triggered process waits on, that had an event and that nothing can change any more is reported as a clock that stopped. `--hang-report=MAX` lists at most MAX processes per part (default 30).",
    ),
    (
        "W3102",
        "A process has no wait statement, no sensitivity list and calls no procedure that waits. It never gives control back to the simulator, so time cannot advance and the run does not end by itself; it can only end through `stop` or `finish`, a failed assertion or an error. A process that should run once ends with `wait;`.",
    ),
    // yavs-verilog: its codes are registered by the crate itself (`yavs_verilog::CODES`); E4500 is the CLI's
    (
        "E4500",
        "The design reaches Verilog: elaborating and simulating Verilog is not implemented. A Verilog file can be analyzed into a library, but a design whose top level or instances lead to a Verilog module stops here.",
    ),
    // every stage
    (
        "E9999",
        "An internal compiler error (\"this is a bug in yavs\"). Please report it with the input that triggers it.",
    ),
];

/// The codes `yavs-verilog` registers that this table does not already describe.
fn verilog_extra() -> impl Iterator<Item = (&'static str, &'static str)> {
    yavs_verilog::CODES
        .iter()
        .map(|c| (c.code, c.explain))
        .filter(|(c, _)| !EXPLAIN.iter().any(|(t, _)| t == c))
}

/// The long text for `code`, from this table, from `yavs-sem` or from `yavs-verilog`.
pub fn explain(code: &str) -> Option<&'static str> {
    EXPLAIN
        .iter()
        .find(|(c, _)| *c == code)
        .map(|(_, e)| *e)
        .or_else(|| yavs_sem::diag::explain(code))
        .or_else(|| verilog_extra().find(|(c, _)| *c == code).map(|(_, e)| e))
}

/// Which diagnostic codes a name may be (`suggest_code`).
#[derive(Clone, Copy, PartialEq)]
pub enum Codes {
    None,
    Warnings,
    All,
}

/// The help line for `input`, which names no diagnostic code: the code close to it or, when
/// `input` starts like a lint rule id (always so with `Codes::None`), the lint rule. `None`
/// when nothing is close.
pub fn suggest_code(input: &str, codes: Codes, lint: bool) -> Option<String> {
    let key = input.to_ascii_lowercase();
    if lint && (codes == Codes::None || key.starts_with("lint")) {
        let id = yavs_lint::similar_rule(&key)?;
        return Some(yavs_diag::suggest::help(&[(id.into(), "lint rule")]));
    }
    // Codes are numbered densely: a well-formed one that does not exist (`E0999`) is no typo of
    // its neighbours, and a malformed one that fits several (`E010`) names none of them.
    if matches!(key.as_bytes(), [b'e' | b'w' | b'l', d @ ..] if d.len() == 4 && d.iter().all(u8::is_ascii_digit))
    {
        return None;
    }
    let names = EXPLAIN
        .iter()
        .chain(yavs_sem::diag::EXPLAIN)
        .copied()
        .chain(verilog_extra())
        .map(|(c, _)| c)
        .filter(|c| codes == Codes::All || (codes == Codes::Warnings && c.starts_with('W')))
        .map(|c| (c, "diagnostic code"))
        .chain((codes == Codes::Warnings).then_some(("error", "warning option")));
    let cands: Vec<_> = names
        .map(|(c, noun)| (c.to_ascii_lowercase(), (c, noun)))
        .collect();
    let found = yavs_diag::suggest::similar(&key, cands.iter().map(|(k, p)| (k.as_str(), p)));
    let [(code, noun)] = found[..] else {
        return None;
    };
    Some(yavs_diag::suggest::help(&[((*code).into(), noun)]))
}

/// Every code with its explanation, for the index page of the user guide.
#[cfg(test)]
pub(crate) fn all() -> Vec<(&'static str, &'static str)> {
    EXPLAIN
        .iter()
        .chain(yavs_sem::diag::EXPLAIN)
        .copied()
        .chain(verilog_extra())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn scan(dir: &Path, found: &mut std::collections::BTreeSet<String>) {
        for e in std::fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                scan(&p, found);
            } else if p.extension().is_some_and(|x| x == "rs") {
                let t = std::fs::read_to_string(&p).unwrap();
                let b = t.as_bytes();
                for i in 1..b.len().saturating_sub(6) {
                    if b[i - 1] == b'"'
                        && matches!(b[i], b'E' | b'W' | b'L')
                        && b[i + 1..i + 5].iter().all(u8::is_ascii_digit)
                        && b[i + 5] == b'"'
                    {
                        found.insert(t[i..i + 5].to_string());
                    }
                }
            }
        }
    }

    #[test]
    fn every_code_in_the_crates_has_an_explain_text() {
        let mut found = Default::default();
        let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        for c in std::fs::read_dir(crates).unwrap() {
            let src = c.unwrap().path().join("src");
            if src.is_dir() {
                scan(&src, &mut found);
            }
        }
        assert!(found.len() > 100, "scan found only {}", found.len());
        let missing: Vec<_> = found.iter().filter(|c| explain(c).is_none()).collect();
        assert!(
            missing.is_empty(),
            "codes without an explain text: {missing:?}"
        );
    }

    #[test]
    fn table_codes_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for (c, e) in EXPLAIN {
            assert!(seen.insert(*c), "duplicate {c}");
            assert!(!e.is_empty());
        }
    }
}
