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
        "A design unit contains PSL (IEEE 1850): default clock, property and sequence declarations, assert/assume/cover/restrict directives or vunit/vmode/vprop units. yavs does not run PSL; the constructs are skipped, so no property is checked and the simulation is unaffected. One warning is given per design unit. Use `-Werror=W0004` to make it an error, or `-Wno-W0004` to silence it.",
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
    ("E0104", "The design has no top level to elaborate."),
    ("E0105", "`--stop-time` is not a valid time such as `10ms`."),
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
        "A VHPIDIRECT foreign subprogram's symbol is defined in no `--load` library and not in the executable. The run goes on; calling the subprogram is `E0107`.",
    ),
    (
        "E0107",
        "A VHPIDIRECT foreign subprogram cannot be bound or called: its C signature cannot be called, or (at the call) no `--load` library and not the executable defines its symbol.",
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
        "A library manifest cannot be read, written or resolved.",
    ),
    (
        "L0003",
        "A library manifest is corrupt. Delete the library directory and analyze its files again.",
    ),
    (
        "L0004",
        "A library manifest has a format version this yavs does not read, or was written before the rename to yavs. Re-analyze the library's files.",
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
        "W2118",
        "A constant whose initial value calls an impure function cannot be evaluated during elaboration yet; it is evaluated when the simulation starts, so its calls run after the elaboration-time calls instead of in declaration order.",
    ),
    // yavs-kernel runtime errors
    (
        "E3001",
        "The time of a scheduled event, `now + delay`, is beyond `TIME'HIGH`.",
    ),
    (
        "E3002",
        "More delta cycles than the limit ran at one simulated time, usually a zero-delay feedback loop. The note lists the processes that ran in the last delta.",
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
    // every stage
    (
        "E9999",
        "An internal compiler error (\"this is a bug in yavs\"), or a feature that is not implemented yet and names the slice or milestone. Please report the former with the input that triggers it.",
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
