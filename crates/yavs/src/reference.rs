//! The full command-line reference of the user guide, `docs/cli-reference.rst`, generated from
//! the clap command tree so that it cannot drift from `--help`.
//!
//! The test `cli_reference_is_current` fails when the committed page differs from what the
//! command line defines now. Regenerate it after changing an option, a default, a value or a
//! help text:
//!
//! ```text
//! YAVS_BLESS=1 cargo test -p yavs --bin yavs cli_reference
//! ```
//!
//! Options hidden from `--help` are left out. The exit statuses and environment variables are
//! not part of the clap definition: they are the fixed text at the end of this file, kept
//! next to the generator so that a change of `docs/cli-contract.md` finds it.

use clap::{Arg, ArgAction, Command, CommandFactory};
use std::cell::Cell;
use std::collections::BTreeSet;
use std::fmt::Write;

thread_local! {
    /// Set while the page is generated: `--tier` says it for every build (`elab::tiers_built`).
    pub(crate) static GENERIC_TIERS: Cell<bool> = const { Cell::new(false) };
}

/// The page, relative to the repository root.
const PAGE: &str = "docs/cli-reference.rst";

/// A paragraph of help text as reStructuredText: Markdown code spans become literals.
fn rst(text: &str) -> String {
    let mut out = String::new();
    let mut open = false;
    for part in text.split('`') {
        if open {
            out.push_str("``");
            out.push_str(part);
            out.push_str("``");
        } else {
            out.push_str(part);
        }
        open = !open;
    }
    out
}

/// The text a help string holds, long form first.
fn help(arg: &Arg) -> String {
    arg.get_long_help()
        .or_else(|| arg.get_help())
        .map(|s| s.to_string())
        .map(|s| sentence(&s))
        .unwrap_or_default()
}

/// `text` ending in a full stop.
fn sentence(text: &str) -> String {
    let t = text.trim();
    if t.is_empty() || t.ends_with(['.', ':', '!', '?']) {
        t.to_string()
    } else {
        format!("{t}.")
    }
}

/// The value placeholder of an option or argument: its `value_name`, else the accepted values
/// of an enumeration, else the upper-case name.
fn value_name(arg: &Arg) -> String {
    let values: Vec<String> = arg
        .get_possible_values()
        .iter()
        .filter(|v| !v.is_hide_set())
        .map(|v| v.get_name().to_string())
        .collect();
    if !values.is_empty() && arg.get_value_delimiter().is_none() {
        return values.join("|");
    }
    if let Some(names) = arg.get_value_names()
        && let Some(n) = names.first()
    {
        return n.to_string();
    }
    arg.get_id().as_str().to_uppercase().replace('-', "_")
}

fn takes_value(arg: &Arg) -> bool {
    arg.get_action().takes_values()
}

/// `--name=VALUE` forms of an option for the `option` directive, `-s VALUE` first.
fn signature(arg: &Arg) -> String {
    let mut forms = Vec::new();
    let v = value_name(arg);
    let optional = arg
        .get_num_args()
        .is_some_and(|r| r.min_values() == 0 && takes_value(arg));
    if let Some(s) = arg.get_short() {
        forms.push(if takes_value(arg) && !optional {
            format!("-{s} {v}")
        } else {
            format!("-{s}")
        });
    }
    if let Some(l) = arg.get_long() {
        let mut names = vec![l];
        names.extend(arg.get_visible_aliases().unwrap_or_default());
        for n in names {
            if !takes_value(arg) {
                forms.push(format!("--{n}"));
            } else if optional {
                forms.push(format!("--{n}"));
                forms.push(format!("--{n}={v}"));
            } else {
                forms.push(format!("--{n}={v}"));
            }
        }
    }
    forms.join(", ")
}

/// The line after the description: values, default, repetition.
fn facts(cmd: &Command, arg: &Arg) -> Vec<String> {
    let mut out = Vec::new();
    if arg.is_required_set() {
        out.push("Required.".to_string());
    }
    if matches!(arg.get_action(), ArgAction::Append) || arg.get_value_delimiter().is_some() {
        out.push(if arg.get_value_delimiter().is_some() {
            format!(
                "A list, values separated by ``{}``.",
                arg.get_value_delimiter().unwrap()
            )
        } else {
            "May be repeated.".to_string()
        });
    }
    let defaults: Vec<String> = arg
        .get_default_values()
        .iter()
        .map(|d| d.to_string_lossy().into_owned())
        .collect();
    if !defaults.is_empty() && takes_value(arg) {
        out.push(format!("Default: ``{}``.", defaults.join(",")));
    }
    let conflicts: Vec<String> = cmd
        .get_arg_conflicts_with(arg)
        .iter()
        .filter(|a| !a.is_hide_set() && a.get_long().is_some())
        .map(|a| format!("``--{}``", a.get_long().unwrap()))
        .collect();
    if !conflicts.is_empty() {
        out.push(format!("Cannot be combined with {}.", conflicts.join(", ")));
    }
    out
}

/// Indented lines for `text`, one paragraph per blank-line separated block.
fn indent(out: &mut String, text: &str) {
    for line in text.lines() {
        if line.is_empty() {
            out.push('\n');
        } else {
            let _ = writeln!(out, "   {line}");
        }
    }
}

/// One option as an `option` directive with its description and facts.
fn option(out: &mut String, cmd: &Command, arg: &Arg) {
    let _ = writeln!(out, ".. option:: {}\n", signature(arg));
    let mut body = rst(&help(arg));
    let values: Vec<_> = arg
        .get_possible_values()
        .into_iter()
        .filter(|v| !v.is_hide_set())
        .collect();
    let with_help = values.iter().any(|v| v.get_help().is_some());
    if !values.is_empty() && !with_help && arg.get_value_delimiter().is_some() {
        let names: Vec<String> = values
            .iter()
            .map(|v| format!("``{}``", v.get_name()))
            .collect();
        body.push_str(&format!("\n\nAccepted: {}.", names.join(", ")));
    }
    let facts = facts(cmd, arg);
    if !facts.is_empty() {
        body.push_str("\n\n");
        body.push_str(&facts.join(" "));
    }
    indent(out, &body);
    if with_help {
        out.push_str("\n   Values:\n\n");
        for v in &values {
            let h = v
                .get_help()
                .map(|h| rst(&h.to_string()))
                .unwrap_or_default();
            if h.is_empty() {
                let _ = writeln!(out, "   * ``{}``", v.get_name());
            } else {
                let _ = writeln!(out, "   * ``{}``: {h}", v.get_name());
            }
        }
    }
    out.push('\n');
}

/// The synopsis of a command: required options and the arguments, written the way the guide
/// writes them.
fn synopsis(path: &str, cmd: &Command) -> String {
    let mut s = path.to_string();
    if cmd
        .get_subcommands()
        .any(|c| !c.is_hide_set() && c.get_name() != "help")
    {
        s.push_str(" COMMAND");
    }
    let opts: Vec<&Arg> = cmd
        .get_arguments()
        .filter(|a| !a.is_hide_set() && !a.is_global_set() && a.get_long().is_some())
        .filter(|a| !matches!(a.get_id().as_str(), "help" | "version"))
        .collect();
    if opts.iter().any(|a| !a.is_required_set()) {
        s.push_str(" [OPTIONS]");
    }
    for a in opts.iter().filter(|a| a.is_required_set()) {
        let _ = write!(s, " --{}={}", a.get_long().unwrap(), value_name(a));
    }
    for a in cmd.get_positionals().filter(|a| !a.is_hide_set()) {
        let n = value_name(a);
        let many = matches!(a.get_action(), ArgAction::Append);
        let _ = match (a.is_required_set(), many) {
            (true, true) => write!(s, " {n}..."),
            (true, false) => write!(s, " {n}"),
            (false, true) => write!(s, " [{n}...]"),
            (false, false) => write!(s, " [{n}]"),
        };
    }
    s
}

/// Section of one command: description, synopsis, arguments, options.
fn command(out: &mut String, path: &str, cmd: &Command, codes: &mut BTreeSet<String>) {
    let args: Vec<&Arg> = cmd
        .get_arguments()
        .filter(|a| !a.is_hide_set() && !a.is_global_set())
        .filter(|a| !matches!(a.get_id().as_str(), "help" | "version"))
        .collect();
    if args.is_empty() && cmd.get_subcommands().any(|c| c.get_name() != "help") {
        return;
    }
    let title = format!("``{path}``");
    let _ = writeln!(out, "{title}\n{}\n", "-".repeat(title.len()));
    let _ = writeln!(out, ".. program:: {path}\n");
    let about = cmd
        .get_long_about()
        .or_else(|| cmd.get_about())
        .map(|s| s.to_string())
        .unwrap_or_default();
    let _ = writeln!(out, "{}\n", rst(about.trim_end_matches('.')) + ".");
    let _ = writeln!(out, ".. code-block:: text\n\n   {}\n", synopsis(path, cmd));
    let (pos, opts): (Vec<&Arg>, Vec<&Arg>) = args.into_iter().partition(|a| a.is_positional());
    if !pos.is_empty() {
        out.push_str("Arguments:\n\n");
        for a in &pos {
            let h = rst(&help(a));
            let h = if h.is_empty() { h } else { format!(": {h}") };
            let many = matches!(a.get_action(), ArgAction::Append);
            let _ = writeln!(
                out,
                "* ``{}``{}{h}",
                value_name(a),
                if !many {
                    ""
                } else if a.is_required_set() {
                    " (one or more)"
                } else {
                    " (any number)"
                }
            );
            codes_in(&h, codes);
        }
        out.push('\n');
    }
    for a in &opts {
        option(out, cmd, a);
        codes_in(&help(a), codes);
    }
}

/// The diagnostic codes (`E0105`, `W0109`) a text names.
fn codes_in(text: &str, codes: &mut BTreeSet<String>) {
    let b = text.as_bytes();
    for i in 0..b.len().saturating_sub(4) {
        let boundary = i == 0 || !b[i - 1].is_ascii_alphanumeric();
        if boundary
            && matches!(b[i], b'E' | b'W')
            && b[i + 1..i + 5].iter().all(u8::is_ascii_digit)
            && b.get(i + 5).is_none_or(|c| !c.is_ascii_alphanumeric())
        {
            codes.insert(text[i..i + 5].to_string());
        }
    }
}

/// All subcommands depth first, with their paths.
fn walk<'a>(path: &str, cmd: &'a Command, into: &mut Vec<(String, &'a Command)>) {
    for c in cmd
        .get_subcommands()
        .filter(|c| !c.is_hide_set() && c.get_name() != "help")
    {
        let p = format!("{path} {}", c.get_name());
        into.push((p.clone(), c));
        walk(&p, c, into);
    }
}

/// The text of the whole page.
pub fn render() -> String {
    // The help of `--tier` names the tiers of this build; the page is the same for every build.
    GENERIC_TIERS.set(true);
    let mut root = crate::Cli::command();
    GENERIC_TIERS.set(false);
    root.build();
    let mut out = String::new();
    out.push_str(HEAD);

    let mut cmds = Vec::new();
    walk("yavs", &root, &mut cmds);
    out.push_str(
        "Commands\n--------\n\n.. list-table::\n   :header-rows: 1\n   :widths: 30 70\n\n",
    );
    out.push_str("   * - Command\n     - What it does\n");
    for (p, c) in &cmds {
        if c.get_subcommands().any(|s| s.get_name() != "help") {
            continue;
        }
        let about = c.get_about().map(|s| s.to_string()).unwrap_or_default();
        let _ = writeln!(out, "   * - ``{}``\n     - {}", synopsis(p, c), rst(&about));
    }
    out.push_str(
        "   * - ``yavs --explain CODE``\n     - Print the explanation of a diagnostic code.\n",
    );
    out.push_str("   * - ``yavs --version``\n     - Print the version and whether a compiled tier is built in.\n\n");

    let mut codes = BTreeSet::new();
    out.push_str("Options of every command\n------------------------\n\n.. program:: yavs\n\n");
    for a in root
        .get_arguments()
        .filter(|a| !a.is_hide_set() && !matches!(a.get_id().as_str(), "help" | "version"))
    {
        option(&mut out, &root, a);
        codes_in(&help(a), &mut codes);
    }
    out.push_str(".. option:: -h, --help\n\n   Print the help of the command and exit.\n\n");
    out.push_str(".. option:: -V, --version\n\n   Print the version and exit.\n\n");
    out.push_str(
        "``--color``, ``--message-format`` and ``--encoding`` are accepted before and after the \
         subcommand; ``--explain`` stands alone.\n\n",
    );

    for (p, c) in &cmds {
        let mut own = BTreeSet::new();
        command(&mut out, p, c, &mut own);
        if !own.is_empty() {
            let list: Vec<String> = own.iter().map(|c| format!("``{c}``")).collect();
            let _ = writeln!(
                out,
                "Diagnostics named above: {}; ``yavs --explain CODE`` explains each.\n",
                list.join(", ")
            );
        }
        codes.append(&mut own);
    }
    out.push_str(TAIL);
    out
}

const HEAD: &str = "\
.. This page is generated from the command-line definition (crates/yavs/src/reference.rs). Do
.. not edit it. After a change to an option, default, value or help text regenerate it with:
..
..     YAVS_BLESS=1 cargo test -p yavs --bin yavs cli_reference

Command-line reference
======================

Every command and option of ``yavs``, as ``yavs --help`` and ``yavs COMMAND --help`` print them.
:doc:`cli` is the shorter, task-oriented overview; the options of ``vunit-yavs`` are in
:doc:`vunit-yavs`.

Write an option with a value as ``--name=value``. Options go before or after the positional
arguments. An argument ``@FILE`` is replaced by the arguments in FILE, one per line, taken as
written, so that long file lists and generic values need no quoting.

";

const TAIL: &str = "\
Environment variables
---------------------

None is needed. Command-line options win over the variables.

.. list-table::
   :header-rows: 1
   :widths: 30 70

   * - Variable
     - Meaning
   * - ``NO_COLOR``
     - When set, diagnostics have no colour; ``--color=always`` still forces it.
   * - ``YAVS_STD_CACHE=DIR|off``
     - Where the analyzed ``std`` and ``ieee`` packages are kept, as ``--std-cache``. ``off``
       keeps them in each library. Default: ``yavs/std`` in the user's cache directory
       (``XDG_CACHE_HOME`` on Linux, ``LOCALAPPDATA`` on Windows).
   * - ``YAVS_WAVE_FLUSH=5s|500ms``
     - Also end a waveform block after that much run time, so that a killed run leaves more of
       the file (:doc:`waveforms`). Any other value is error ``E0801``.
   * - ``YAVS_TOKENS=PORT:SECRET``
     - The token server of a test runner that lends CPU slots: with ``--threads`` above 1 the
       extra threads run only on tokens it lends (:doc:`running-faster`). A value yavs cannot
       use is error ``E0128`` and exit status 2; a server that cannot be reached is warning ``W0118`` and the run has one thread.
   * - ``YAVS_HUGE_HEAP=0``
     - Do not ask for transparent huge pages on the heap at start (Linux only).

Exit status
-----------

.. list-table::
   :header-rows: 1
   :widths: 15 85

   * - Status
     - Meaning
   * - 0
     - Success. ``run``: the simulation ended without failure (no more events, ``stop`` or
       ``finish`` with 0, or ``--stop-time`` reached).
   * - 1
     - ``parse`` and ``analyze``: a file has errors. ``run``: a failed assertion or report at the
       exit severity, a runtime error, or ``stop(N)`` or ``finish(N)`` with N other than 0 (the
       status is N when N is from 1 to 125). ``cov``: a database or exclusion file could not be read or a report written, or the total is below ``--fail-under``. ``cache``: a file could not be written.
   * - 2
     - Usage error, or a library, analysis or elaboration error; also an unknown code for
       ``--explain``. A runtime error while the design is initialized, before time 0, is an
       elaboration error.
   * - 3
     - Internal error (``E9999``): please report it.
   * - 130
     - ``run``: stopped by Ctrl-C or SIGTERM.
   * - 141
     - ``run``: standard output was closed or could not be written.

Nothing runs after a failed step, and a status of 0 is never given after an error was printed.

Diagnostics
-----------

Errors and warnings go to standard error with a stable code: ``E`` for errors and ``W`` for
warnings. ``--message-format`` picks the form and ``yavs --explain CODE`` prints a longer
explanation. Codes by range: ``E0xxx`` syntax, command line, libraries, waveforms, coverage,
debugger and plugins; ``E1xxx`` semantic analysis; ``E2xxx`` elaboration; ``E3xxx`` runtime
errors; ``E9999`` an internal error. The messages and the common problems are in
:doc:`troubleshooting`.
";

#[cfg(test)]
mod tests {
    use super::*;

    /// The committed page is what the command line defines now.
    #[test]
    fn cli_reference_is_current() {
        let page = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(PAGE);
        let now = render();
        if std::env::var_os("YAVS_BLESS").is_some() {
            std::fs::write(&page, &now).unwrap();
            return;
        }
        let committed = std::fs::read_to_string(&page).unwrap_or_default();
        assert!(
            committed == now,
            "{PAGE} is out of date: the command line changed (an option, default, value or help \
             text).\nRegenerate it and commit the result:\n\n    \
             YAVS_BLESS=1 cargo test -p yavs --bin yavs cli_reference\n"
        );
    }

    /// The page is a user guide: help texts do not point into the repository or name internals.
    #[test]
    fn help_texts_are_for_users() {
        let page = render();
        let page = page.split_once("\n\n").unwrap().1; // after the generated-file comment
        for bad in [".md", "docs/", "crates/", "src/", ".rs", "lib/"] {
            assert!(
                !page.contains(bad),
                "a help text mentions `{bad}`: write it for a user of the command line, the \
                 reference page is generated from it ({PAGE})"
            );
        }
    }
}
