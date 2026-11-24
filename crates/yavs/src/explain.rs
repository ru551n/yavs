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
        "`yavs run --tier=...` names a compiled tier that this yavs was built without, or one that was removed. `auto`, `clift` and `clift-auto` need the `clift` cargo feature (Cranelift, which the release archives are built with). `llvm` and `auto3` were removed together with the LLVM tier: Cranelift is the compiled tier, `--tier=auto` (the default) or `--tier=clift`. Use a tier the build has, or `--tier=interp`, which every build has. The user guide page \"Running faster\" describes the tiers: https://yavs.readthedocs.io/en/latest/running-faster.html",
    ),
    (
        "E0121",
        "The compiled tier could not compile the design for `--tier=clift`, or could not start compiling for `--tier=auto`. This is a bug in yavs; `--tier=interp` runs the design.",
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
    // E0127 (the LLVM library cannot be loaded) is retired with the LLVM tier. Never reuse it.
    (
        "E0128",
        "`--threads` is above 1 and `YAVS_TOKENS` names a token server that yavs cannot use: the value is not `<port>:<secret>`, or the server did not accept the secret (often a `YAVS_TOKENS` left over from a test runner that has ended). The run stops before analysis (exit 2). Unset `YAVS_TOKENS` to run without a token server. A server that cannot be reached is warning `W0118`.",
    ),
    (
        "W0118",
        "`--threads` is above 1 and the token server named in `YAVS_TOKENS` cannot be reached (nothing listens on that port of 127.0.0.1, it does not answer, or the connection broke during the run; the note says which). Tokens only speed a run up, so the run goes on with one thread and its output is the same. A value that is not `<port>:<secret>` or a secret the server does not accept is error `E0128`.",
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
        "A signal is left out of the waveform: an array or vector with more elements than `--wave-max-array`, a variable whose bounds are known only when the simulation runs, or, under `--cycle`, a memory of integers or enumerations that the cycle region writes at run-time indices and cannot export. Raise the cap, or run without `--cycle`, to record it.",
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
        "A source file changed on disk since it was analyzed into the library. Analyze it again.",
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
        "E2122",
        "A path given to `--two-state=PATH`, `--cycle=PATH` or `--cycle-strict=PATH` names no instance, block, generate or labelled process of the elaborated design. A path starts at the top-level unit (`tb.dut`), names a generate's every iteration without an index (`tb.g`) and one with it (`tb.g(1)`), and is compared without case. The help names a similar path when there is one; `yavs elab --print-hierarchy TOP` lists them all.",
    ),
    (
        "E2123",
        "The cycle scope and the `--two-state` scope overlap in part: some processes of the cycle scope are in the two-state scope and some are not. The cycle region computes with one kind of values, two-state when its scope lies inside the `--two-state` scope and nine-valued when it lies outside it, so one of the two must hold. Give `--two-state` the cycle scope's paths (or more), or keep the scopes apart.",
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
        "A VHDL runtime error: bound check, length mismatch of an array, record field or element, overflow, null access, division by zero and the like. So is reading the bounds of a deferred constant before its full declaration has been elaborated (a subprogram of the package body that reads `U'length` is called to initialise a constant above the full declaration of `U`: move that constant below it). The message and the call stack say where. A range or index error shows the value and the bounds as VHDL writes them: an enumeration literal (`value WHITE outside of RGB range RED to BLUE`), a physical value with its unit (`value 20 NS outside of SHORT range 1 NS to 10 NS`), a real in full, and `high downto low` for a descending range.",
    ),
    (
        "E3010",
        "The model, the simulation configuration or a control request is invalid.",
    ),
    (
        "E3011",
        "A source of a guarded signal whose composite subtype is resolved as a whole has some of the elements its process drives disconnected by a null transaction and others not (LRM 14.7.3.2): the resolution function would get a value that is partly undefined. Disconnect all of them at once (`target <= null;` on the whole name the process drives, or the same `after` time for each part), or none. A process that drives a slice whose bounds are not static drives every element of the signal; elements it does not name in a static target are not taken into account.",
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
    // yavs-cycle: cycle mode (`yavs run --cycle`)
    (
        "E4101",
        "Cycle mode can run this kind of logic, but yavs does not implement the construct the error names in cycle-based code yet: a limitation of yavs, not a problem in the design. `yavs run --cycle-strict` reports it for a process below the top-level design unit; the headline names the construct and the label points at the statement. Run with `--cycle` to leave the process in the event simulation, or write the construct another way (the label often says how). Every other reason a process stays out has a code of its own (E4110 to E4140). A behavioural process (one that waits for a time, waits forever, has several waits, calls a procedure that waits, or uses files or text input and output) and every process of the top-level design unit, the testbench, stay in the event kernel without an error. `--cycle-report` lists every process with its reason.",
    ),
    (
        "E4102",
        "`yavs run --cycle` (or `--cycle-strict`) cannot be used with an option that shows, counts or changes signals or variables inside the cycle region: a debug session (`--dap`, `--interactive`, `--do`), `--wave-variables`, `--cov`, `--vhpi` or `--activity`. Cycle mode keeps those signals and variables in an image of its own and updates the kernel only with what processes outside the region read. Run without `--cycle` for these. `--wave` and `--surfer` are allowed: the region exports the signals it records, per time step. `--profile`, `--assert-summary`, `--shuffle`, `--hang-report` and `--time-report` are allowed: they do not read the signals of the region, but `--profile` shows the region's time as one entry and `--assert-summary` counts only the asserts that run.",
    ),
    (
        "E4103",
        "The file given to `yavs run --cycle-report=FILE` cannot be created or written (missing directory, permissions, full disk). The check is made before any work, so a run does not end with its report lost.",
    ),
    (
        "E4104",
        "An environment variable of cycle mode has a value it does not take, such as `YAVS_CYCLE_CACHE_MIB=0` (it takes a whole number of MiB of at least 1). The message names the values it takes; unset the variable, or leave it empty, to turn the check off.",
    ),
    (
        "E4106",
        "`yavs run --cycle-strict --two-state` without a cycle PATH found no design under test: no instance of the design holds RTL (clocked or combinational processes), so a strict run would check nothing. Give the part of the design to check as a path, `--cycle-strict=tb.dut`; the note lists the instances of the top-level unit.",
    ),
    (
        "E4105",
        "`yavs run --cycle-report` is given without `--cycle` or `--cycle-strict`. The report describes a cycle region, and the cycle option says which scope it covers (`--cycle` alone: the whole design, with `--two-state` the design under test yavs finds; `--cycle=PATH`: that part of the design). Add the cycle option; the report still stops before anything is compiled or simulated.",
    ),
    (
        "W4101",
        "Retired: cycle mode no longer gives this warning. It said that cycle-based processes below the top-level design unit held `assert` or `report` statements of severity error or failure that cycle mode did not evaluate. Cycle mode now evaluates the asserts and reports of its processes (a process with one it cannot evaluate stays in the event kernel, E4140), and `--no-asserts` drops those below the top-level unit on request, in both modes, with a note at the start of the run that counts them. `-Werror=W4101` and `-Wno-W4101` are still accepted and change nothing.",
    ),
    (
        "E4110",
        "Cycle mode takes a process with a wait only in one shape: `wait until <edge>;` as the first statement, the only wait, a clocked process (`rising_edge(clk)`, `falling_edge(clk)` or `clk'event and clk = '1'`, optionally followed by `and` more conditions). Any other wait (`wait on a;`, a wait on a level) keeps the process in the event kernel. A process that waits for a time, waits forever, waits more than once or calls a procedure that waits is behavioural and stays in the event kernel without this code.",
    ),
    (
        "E4111",
        "A cycle-based process has no time inside a clock cycle: an `after` delay (other than zero), `transport`, a `reject` limit or a waveform of several elements keeps it in the event kernel.",
    ),
    (
        "E4112",
        "Cycle mode keeps no signal history: `'last_value`, `'last_event`, `'last_active`, `'stable`, `'quiet`, `'delayed`, `'transaction`, `'active`, `'driving`, `'driving_value`, `'event` anywhere but in the process's own edge test, or an edge test (`rising_edge`, `falling_edge`, a `'event` form) of a signal other than the process's clock inside the body of a clocked process or of a subprogram it calls, keep the process in the event kernel. An edge test in a process that is not of the clocked shape, such as a combinational process, or after an `elsif` or `else`, is E4123.",
    ),
    (
        "E4113",
        "Cycle mode computes two-state values, integers and enumerations: a real or physical (time) value computed at run time, or a signal or variable of such a type, keeps the process in the event kernel. Constant expressions elaboration folds are fine.",
    ),
    (
        "E4114",
        "Shared variables used other than by the clock-edge body of one clocked process, methods of protected objects, allocators and access values in expressions keep a process in the event kernel. A memory in a shared variable (not of a protected type) that one clocked process alone reads and writes, in its clock-edge body and not in a subprogram, runs cycle-based as a variable of that process; when another process uses it too, the order the two run in on a common edge decides the normal mode's result, and both stay out. A process that declares a protected object, a file or an access value (text lines) is behavioural instead, the testbench kind, and has no error under `--cycle-strict`.",
    ),
    (
        "E4115",
        "Postponed processes, guarded blocks, guarded signals and assignments and `null` waveforms keep a process in the event kernel: cycle mode has no end of a time step and models no guarded signals.",
    ),
    (
        "E4116",
        "`force` and `release` change a signal from outside its drivers: a process using them stays in the event kernel.",
    ),
    (
        "E4117",
        "A call cycle mode cannot follow keeps the process in the event kernel: a foreign subprogram, a subprogram whose body is not known, or, called by a combinational process, a subprogram that changes a variable or signal it is not given (a procedure of a clocked process may change the process's own registers). The label names the subprogram and what it does.",
    ),
    (
        "E4118",
        "Two-state values have no high impedance: a process that uses the value `'Z'` stays in the event kernel.",
    ),
    (
        "E4119",
        "A combinational process that does not assign a signal element it drives on every path keeps that element's value from one run to the next: a latch, which is state, not combinational logic. The label names the element, and where it can, the statement that leaves it unassigned. Assign every driven element on every path, for example a default assignment at the start of the process. An element the process never assigns at all keeps its initial value and is no latch.",
    ),
    (
        "E4120",
        "A variable of a combinational process read before it is assigned on some path keeps its value from the previous run of the process: state, not combinational logic. The label names the variable and the read. Assign it before reading it on every path.",
    ),
    (
        "E4121",
        "A combinational process that reads a signal its sensitivity list does not name, or a clocked process whose asynchronous reset branch does, is not run by the normal mode when that signal changes, so its result is not a function of what it reads. Cycle mode computes it from every read and would give another result: the process stays in the event kernel. Name every signal it reads, or write `process (all)`.",
    ),
    (
        "E4122",
        "Combinational logic in which an element depends on itself (through other processes or within one) has no single settled value: a combinational loop. The label prints the loop, element by element and process by process. The processes in it stay in the event kernel.",
    ),
    (
        "E4123",
        "A clocked process of a shape cycle mode does not take: statements outside the edge test, an edge test inside other statements, an `else` after it, statements before a `wait until` (they run once at initialisation), a `wait` inside another statement, a clock that is not one element of a signal, a reset branch whose condition reads no signal or reads the clock, a sensitivity list that does not name the clock or every signal of the asynchronous reset, or a `wait until` whose `on` list does not name the clock. A clocked process is `if <edge> then ... end if;`, optionally after an asynchronous reset branch `if <reset> then ... elsif <edge> then` (any condition on signals of the list), or `wait until <edge>;` first.",
    ),
    (
        "E4124",
        "Cycle mode needs every element a process of the region drives to have that process as its only source: an element another process drives too, a guarded signal or a signal with a resolution function of its own (other than `std_logic`'s) keeps the process in the event kernel.",
    ),
    (
        "E4125",
        "Clocks come from outside the region. When combinational logic of the region makes one (a gated clock, a copy), that process stays in the event kernel, which computes the clock; when a register of the region makes one (a divider), the processes that use it stay in the event kernel. An asynchronous reset may be made inside the region.",
    ),
    (
        "E4126",
        "yavs cannot yet follow a construct of the process far enough to show that it is clocked or combinational logic: an `out` actual, an assignment target or a statement of an unusual form, a signal it cannot place in the design, or an internal mismatch it asks you to report. A limitation of yavs, not a problem in the design: run with `--cycle` to leave the process in the event simulation, or write the construct more simply.",
    ),
    (
        "E4127",
        "A process outside the cycle region reads the history of a signal this process drives: `'active`, `'last_active`, `'last_event`, `'last_value`, `'transaction`, `'stable`, `'quiet` or `'delayed`, directly or in a subprogram it calls (an edge test such as `rising_edge` is not history here). The region publishes only the values that change and keeps no history of what it drives, so every process driving that signal stays in the event kernel. The label names the reading attribute.",
    ),
    (
        "E4128",
        "A `force` or `release` the design can run (in a process, or in a subprogram a process calls) changes a signal this process drives, which cycle-based code would not see, so the process stays in the event kernel; members that only read the forced signal still join. A force whose target yavs cannot tell (a signal parameter) keeps every process out, and so does any force without `--two-state`, whose rules for `'Z'` do not follow forced values. Forces in units the design does not instantiate and in subprograms nothing calls do not count. Under `--cycle-strict` each such process below the top-level unit is an error that points at the force. A limitation of yavs: drive the signal from the testbench instead of forcing it, or run with `--cycle`.",
    ),
    (
        "E4129",
        "Combinational logic (or a register with an asynchronous reset) feeds a process outside the region whose result depends on the delta cycles its inputs change in: one that keeps a value from its previous run (a latch, a variable read before it is assigned, a combinational loop), has an incomplete sensitivity list, or stays out for another reason. The event simulation changes the logic over several delta cycles and the reader sees each step; cycle-based code changes it in one step, so the reader would see other values or run another number of times. The second label names the reader. Fix the reader so that it can join the region too, or make it a clocked process.",
    ),
    (
        "E4130",
        "Logic between registers of two clocks whose edges come in different delta cycles of one time step: a clock gated or computed by an assignment comes one delta cycle after its source (a plain copy is its source). The event simulation samples such logic before its new value arrives; cycle-based code would hand over the settled value. Clock both registers from one clock, or make the second clock a plain copy of the first.",
    ),
    (
        "E4131",
        "Logic that computes part of a clock from a signal the region drives. Cycle-based code gives its outputs one delta cycle after its inputs change, where the event simulation takes one delta cycle per level of logic, so the clock edge would come early. Make the clock in the testbench, or use a clock enable instead of logic on the clock.",
    ),
    (
        "E4132",
        "A register whose asynchronous reset another register makes, and whose output is the clock or the asynchronous reset of a process. The event simulation can change it at the clock edge and again one delta cycle later; that pulse clocks or resets the other process, and cycle-based code, which settles both at once, would not give it. The second label names the process it clocks or resets. Use a synchronous reset, or a reset that does not come from another register.",
    ),
    (
        "E4133",
        "A memory (an array signal of more than 4096 logic elements or 64 integers) that is written in an asynchronous reset branch, written both at run-time indices and at fixed ones, or read outside the region. yavs keeps a large memory in cycle-based code only when it is written at run-time indices in the clock-edge branch and read inside the region. A limitation of yavs: write and read it that way, or run with `--cycle`.",
    ),
    (
        "E4134",
        "Some elements of a signal are driven by this process and others by a process outside the region. yavs does not split one signal between cycle-based code and the event simulation yet. The second label names the other driver. A limitation of yavs: drive all elements from processes that can run cycle-based, or split the signal in two.",
    ),
    (
        "E4135",
        "The process drives a signal that also gets its value from a port connection or a constant source, which yavs does not model in cycle-based code yet. A limitation of yavs: drive the signal from one process only, or run with `--cycle`.",
    ),
    (
        "E4136",
        "An assert or report of the top-level unit reads a signal the region does not drive alone. Its process stays in the event simulation so that the check sees the values it sees there. This rule holds when the cycle scope is the whole design (`--cycle` without PATH and without `--two-state`); in a scope a path or the design under test gives, processes follow one set of rules, E4140 among them. Processes of the top-level unit are never an error under `--cycle-strict`; the code appears in `--cycle-report`. Nothing to fix.",
    ),
    (
        "E4137",
        "Without `--two-state`, a process assigns a metavalue literal (`'U'`, `'X'`, ...) to a signal the testbench reads, or passes on a `'Z'` from a process outside to such a signal. The testbench must then see the metavalue the event simulation gives it, and cycle-based code gives only '0' and '1'. Run with `--two-state` (two-state values at the boundary), or assign '0' or '1' instead.",
    ),
    (
        "E4138",
        "An external name (`<<signal .tb.dut.x : std_logic>>`) in the process. yavs does not follow external names in cycle-based code yet. A limitation of yavs: pass the signal through a port, or run with `--cycle`.",
    ),
    (
        "E4139",
        "A loop cycle mode cannot unroll. Cycle-based code unrolls every loop, so a loop needs bounds known during elaboration (constants, generics), its process at most 65 536 statement runs of loops, and `exit` and `next` of the innermost loop only. A loop without an exit cannot run in a process that must finish each run. Give the loop static bounds, split a very large loop, and jump only out of the innermost loop.",
    ),
    (
        "E4140",
        "An assert or report statement that cycle mode cannot evaluate in its compiled code keeps its process in the event kernel, so that it runs as in the normal mode. Cycle mode evaluates a clocked process's asserts and reports at its clock edge and a combinational process's once its inputs are settled, and takes a message made of string literals, `&`, `'image` and `to_string` of integers, enumerations, `std_ulogic` and `boolean` values, and `to_string`, `to_hstring` and `to_ostring` of vectors, with a severity known during elaboration. It does not take one in the asynchronous reset branch of a clocked process, or in a combinational process whose sensitivity list names a signal the process does not read. Write the message in that form, move the check out of the reset branch or into the testbench, or run with `--no-asserts` to drop the asserts and reports below the top-level unit.",
    ),
    // every stage
    (
        "E9999",
        "An internal compiler error (\"this is a bug in yavs\"). Please report it with the input that triggers it. In the cycle-mode region report it is a reason yavs's own earlier checks should rule out: the process stays in the event kernel, and `--cycle-strict` gives it as an error.",
    ),
];

/// The long text for `code`, from this table or from `yavs-sem`.
pub fn explain(code: &str) -> Option<&'static str> {
    EXPLAIN
        .iter()
        .find(|(c, _)| *c == code)
        .map(|(_, e)| *e)
        .or_else(|| yavs_sem::diag::explain(code))
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
        .map(|(c, _)| *c)
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
