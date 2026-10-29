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
        "A design unit contains PSL (IEEE 1850) outside the subset yavs runs. yavs runs `default clock is ...;`, clocked `assert`/`assume` directives with `always b`, `never b`, `always a -> b` and `always a -> next[n] b` on VHDL booleans, and `cover {b}`, written as VHDL-2008 statements or in `-- psl` comments. Everything else (property and sequence declarations, SERE sequences, other operators, unclocked directives, vunit/vmode/vprop units) is skipped, so that property is not checked and the simulation is otherwise unaffected. Directives for formal tools only (`restrict`, `restrict_guarantee`, `assume_guarantee`, `fairness`, `strong fairness`, and any directive that calls `nondet` or `nondet_vector`) are skipped without this warning. One warning is given per design unit. Use `-Werror=W0004` to make it an error, or `-Wno-W0004` to silence it.",
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
        "The LLVM tier could not compile the design for `--tier=llvm`, or could not start compiling for `--tier=auto`. This is a bug in yavs; `--tier=interp` runs the design.",
    ),
    (
        "E0122",
        "`yavs run --gui` cannot start the graphical debugger `yavs-gui`: it is neither next to the yavs executable nor on PATH, or it cannot be executed. The release archives ship it next to `yavs`.",
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
        "`yavs run --profile` was combined with `--dap`. The profiler samples which process is running; a stop at a breakpoint would be counted as time in the stopped process. Profile a run without `--dap`. Exit status 2.",
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
        "`--threads` is above 1 and `MAKEFLAGS` names a token pool (`--jobserver-auth=R,W` or `fifo:PATH`) that yavs cannot use: the descriptors are closed or not a pipe (the parent did not pass them on), the path is not a named pipe, or the platform has no client for the form (descriptors work on Linux, `fifo:PATH` on Unix). The run stops before analysis (exit 2). Pass the pool on to yavs; a parent that does not share its pool with this run removes `--jobserver-auth` from `MAKEFLAGS` for it.",
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
        "`--threads` is ignored with `--dap`: the debugger stops inside processes, which needs the simulation on one thread. The run is the same as with `--threads=1`.",
    ),
    // yavs-dap
    (
        "E0701",
        "The Debug Adapter Protocol connection (`--dap`) cannot be opened: the port is in use or not allowed, or accepting the client failed.",
    ),
    (
        "E0702",
        "The live waves of a `yavs/waveSubscribe` subscription failed while the simulation ran (a value the recorder cannot encode); the note says why. The run fails.",
    ),
    (
        "E0110",
        "A coverage database or report cannot be written (`yavs run --cov=FILE`, `yavs cov merge -o FILE`, `yavs cov report --cobertura FILE`): the note gives the operating system's reason.",
    ),
    (
        "E0111",
        "A coverage database given to `yavs cov merge` or `yavs cov report` cannot be read, or is not a database `yavs run --cov` wrote: the note says which.",
    ),
    (
        "E0113",
        "A source file named by the coverage database cannot be read, so `yavs cov report --html DIR` cannot show it; the note gives the reason. Run the command where the paths in the database resolve.",
    ),
    (
        "E0112",
        "An exclusion file given to `yavs cov report --exclude` cannot be read or has a malformed row or entry (a `.yaml` or `.yml` file is read as YAML, any other as text rows); the note names the line. Rows start with `statement`, `branch`, `toggle`, `instance`, `unit` or `file`; a YAML entry has the same word as its `kind`.",
    ),
    (
        "W0110",
        "A row of the exclusion file names a line (`file:line`) that is not where the excluded statement is now. The exclusion still applies, matched by the statement's text; update the line to silence the warning.",
    ),
    (
        "W0111",
        "A row of the exclusion file matches nothing in the coverage database: the statement changed (its text hash with it), was removed, or the instance path is wrong.",
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
        "The lowering to IR does not handle this construct yet; the message names it.",
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
    ("E2115", "A generic value is outside the generic's subtype."),
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
    // yavs-kernel runtime errors
    (
        "E3001",
        "A run-for time, `now + delay`, is beyond `TIME'HIGH`. (A wait timeout or a transaction past `TIME'HIGH` is not an error: it never matures.)",
    ),
    (
        "E3002",
        "More delta cycles than the limit ran at one simulated time, usually a zero-delay feedback loop. The labels and the note name the processes of the loop and the signals through which they wake each other: two processes (or concurrent assignments) that assign each other's inputs with no delay, a process that waits only on a signal it assigns, a combinational loop through concurrent statements, or a process that waits for 0 ns in a loop. `--max-deltas=N` raises the limit (default 10000) for a design that needs more deltas at one time.",
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
        "A VHDL runtime error: bound check, overflow, null access, division by zero and the like. The message and the call stack say where.",
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
    // every stage
    (
        "E9999",
        "An internal compiler error (\"this is a bug in yavs\"), or a feature of elaboration or of a command that is not implemented yet and names the milestone (semantic analysis reports those as E1999, lowering as E2001). Please report the former with the input that triggers it.",
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
