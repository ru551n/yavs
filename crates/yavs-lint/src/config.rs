//! `yavs-lint.yaml` (the `rule:` section) and `yavs-lint-waivers.yaml`. Unknown keys and rule ids
//! are errors naming them. Values are read as the text they were written as.

use crate::{Certainty, Rule};
use saphyr_parser::{Event, Parser};
use std::path::{Path, PathBuf};
use yavs_diag::Severity;

enum Node {
    Scalar(String),
    Seq(Vec<Node>),
    Map(Vec<(usize, String, Node)>),
}

type Events<'a> = Parser<'a, saphyr_parser::StrInput<'a>>;

fn next<'a>(p: &mut Events<'a>) -> Result<(Event<'a>, usize), String> {
    match p.next() {
        Some(Ok((ev, sp))) => Ok((ev, sp.start.line())),
        Some(Err(e)) => Err(format!("line {}: {}", e.marker().line(), e.info())),
        None => Err("unexpected end of the YAML stream".into()),
    }
}

fn node<'a>(p: &mut Events<'a>, ev: Event<'a>, line: usize) -> Result<Node, String> {
    match ev {
        Event::Scalar(v, ..) => Ok(Node::Scalar(v.to_string())),
        Event::SequenceStart(..) => {
            let mut v = Vec::new();
            loop {
                let (ev, l) = next(p)?;
                if matches!(ev, Event::SequenceEnd) {
                    return Ok(Node::Seq(v));
                }
                v.push(node(p, ev, l)?);
            }
        }
        Event::MappingStart(..) => {
            let mut v: Vec<(usize, String, Node)> = Vec::new();
            loop {
                let (ev, l) = next(p)?;
                let key = match ev {
                    Event::MappingEnd => return Ok(Node::Map(v)),
                    Event::Scalar(k, ..) => k.to_string(),
                    _ => return Err(format!("line {l}: a key must be a plain word")),
                };
                if v.iter().any(|(_, k, _)| *k == key) {
                    return Err(format!("line {l}: duplicate key `{key}`"));
                }
                let (ev, vl) = next(p)?;
                v.push((l, key, node(p, ev, vl)?));
            }
        }
        _ => Err(format!("line {line}: aliases are not supported")),
    }
}

/// The document's top-level map (empty for an empty file).
fn top(text: &str) -> Result<Vec<(usize, String, Node)>, String> {
    let mut p = Parser::new_from_str(text);
    let mut root = None;
    loop {
        let (ev, l) = next(&mut p)?;
        match ev {
            Event::StreamStart | Event::DocumentStart(_) | Event::DocumentEnd => {}
            Event::StreamEnd => break,
            ev if root.is_none() => root = Some((l, node(&mut p, ev, l)?)),
            _ => return Err(format!("line {l}: more than one YAML document")),
        }
    }
    match root {
        None => Ok(Vec::new()),
        Some((_, Node::Map(m))) => Ok(m),
        Some((_, Node::Scalar(s))) if s.is_empty() => Ok(Vec::new()),
        Some((l, _)) => Err(format!("line {l}: expected a map at the top level")),
    }
}

/// `disable` and `severity` as set at one level.
#[derive(Clone, Copy, Default)]
struct Layer {
    disable: Option<bool>,
    severity: Option<Severity>,
}

fn layer(line: usize, name: &str, n: &Node) -> Result<Layer, String> {
    let Node::Map(fields) = n else {
        return Err(format!(
            "line {line}: the settings of `{name}` must be a map"
        ));
    };
    let mut out = Layer::default();
    for (l, key, v) in fields {
        let text = match v {
            Node::Scalar(s) => s.as_str(),
            _ => return Err(format!("line {l}: `{name}.{key}` must be a single value")),
        };
        let bad = || format!("line {l}: `{name}.{key}` has the invalid value `{text}`");
        match key.as_str() {
            "disable" => {
                out.disable = Some(match text {
                    "true" | "True" => true,
                    "false" | "False" => false,
                    _ => return Err(bad()),
                })
            }
            "severity" => {
                out.severity = Some(match text.to_ascii_lowercase().as_str() {
                    "error" => Severity::Error,
                    "warning" => Severity::Warning,
                    _ => return Err(bad()),
                })
            }
            _ => {
                return Err(format!(
                    "line {l}: unknown key `{key}` in the settings of `{name}` (expected `disable` or `severity`)"
                ));
            }
        }
    }
    Ok(out)
}

/// Which rules run, and at which severity. A rule's own setting wins over its class's, which wins
/// over `rule.global`; with none, the definite class is on and everything reports as an error.
#[derive(Default)]
pub struct Config {
    global: Layer,
    groups: Vec<(Certainty, Layer)>,
    rules: Vec<(&'static str, Layer)>,
}

impl Config {
    /// Read a `yavs-lint.yaml`; the error names the line and the unknown key or rule.
    pub fn parse(text: &str) -> Result<Config, String> {
        let mut cfg = Config::default();
        for (l, key, v) in top(text)? {
            if key != "rule" {
                return Err(format!("line {l}: unknown key `{key}` (expected `rule`)"));
            }
            let Node::Map(rules) = &v else {
                return Err(format!("line {l}: `rule` must be a map"));
            };
            for (l, id, v) in rules {
                match id.as_str() {
                    "global" => cfg.global = layer(*l, id, v)?,
                    "group" => {
                        let Node::Map(groups) = v else {
                            return Err(format!("line {l}: `rule.group` must be a map"));
                        };
                        for (l, name, v) in groups {
                            let class = [
                                Certainty::Definite,
                                Certainty::Advisory,
                                Certainty::Experimental,
                                Certainty::Policy,
                            ]
                            .into_iter()
                            .find(|c| c.name() == name)
                            .ok_or_else(|| {
                                format!(
                                    "line {l}: unknown rule group `{name}` (expected definite, advisory, experimental or policy)"
                                )
                            })?;
                            cfg.groups.push((class, layer(*l, name, v)?));
                        }
                    }
                    _ => {
                        let rule = crate::rule(id).ok_or_else(|| {
                            format!(
                                "line {l}: unknown rule `{id}` (`yavs lint --list-rules` lists the rules)"
                            )
                        })?;
                        cfg.rules.push((rule.id, layer(*l, id, v)?));
                    }
                }
            }
        }
        Ok(cfg)
    }

    fn layers(&self, r: &Rule) -> [Layer; 3] {
        let own = self.rules.iter().rev().find(|(id, _)| *id == r.id);
        let class = self.groups.iter().rev().find(|(c, _)| *c == r.certainty);
        [
            own.map_or(Layer::default(), |x| x.1),
            class.map_or(Layer::default(), |x| x.1),
            self.global,
        ]
    }

    /// Whether this configuration runs the rule.
    pub fn enabled(&self, r: &Rule) -> bool {
        match self.layers(r).iter().find_map(|l| l.disable) {
            Some(disable) => !disable,
            None => r.certainty == Certainty::Definite,
        }
    }

    /// The severity of the rule's findings.
    pub fn severity(&self, r: &Rule) -> Severity {
        self.layers(r)
            .iter()
            .find_map(|l| l.severity)
            .unwrap_or(Severity::Error)
    }
}

struct Waiver {
    /// A rule id, or `*`.
    rule: String,
    files: Option<String>,
    /// Empty: the whole file.
    lines: Vec<usize>,
}

/// Findings a project has decided to accept, with a reason.
#[derive(Default)]
pub struct Waivers(Vec<Waiver>);

fn normalize(name: &str) -> String {
    name.replace('\\', "/").trim_start_matches("./").to_owned()
}

/// `*` and `?` stop at `/`, `**` does not.
fn glob(pattern: &[u8], text: &[u8]) -> bool {
    match pattern {
        [] => text.is_empty(),
        [b'*', b'*', rest @ ..] => {
            let rest = rest.strip_prefix(b"/").unwrap_or(rest);
            (0..=text.len()).any(|i| glob(rest, &text[i..]))
        }
        [b'*', rest @ ..] => (0..=text.len())
            .take_while(|&i| i == 0 || text[i - 1] != b'/')
            .any(|i| glob(rest, &text[i..])),
        [b'?', rest @ ..] => text.first().is_some_and(|&c| c != b'/') && glob(rest, &text[1..]),
        [c, rest @ ..] => text.first() == Some(c) && glob(rest, &text[1..]),
    }
}

impl Waivers {
    /// Read a `yavs-lint-waivers.yaml`; the error names the line.
    pub fn parse(text: &str) -> Result<Waivers, String> {
        let mut out = Vec::new();
        for (l, key, v) in top(text)? {
            if key != "waivers" {
                return Err(format!(
                    "line {l}: unknown key `{key}` (expected `waivers`)"
                ));
            }
            let items = match v {
                Node::Seq(items) => items,
                // `waivers:` with every entry commented out.
                Node::Scalar(s) if s.is_empty() => Vec::new(),
                _ => return Err(format!("line {l}: `waivers` must be a list")),
            };
            for item in items {
                let Node::Map(fields) = item else {
                    return Err(format!("line {l}: each waiver must be a map"));
                };
                let mut w = Waiver {
                    rule: String::new(),
                    files: None,
                    lines: Vec::new(),
                };
                for (l, key, v) in fields {
                    match (key.as_str(), v) {
                        ("rule", Node::Scalar(s)) => {
                            if s != "*" && crate::rule(&s).is_none() {
                                return Err(format!("line {l}: unknown rule `{s}` in a waiver"));
                            }
                            w.rule = s;
                        }
                        ("files", Node::Scalar(s)) => w.files = Some(normalize(&s)),
                        ("reason", Node::Scalar(_)) => {}
                        ("lines", Node::Seq(lines)) => {
                            for n in lines {
                                match n {
                                    Node::Scalar(s) if s.parse::<usize>().is_ok() => {
                                        w.lines.push(s.parse().expect("checked"))
                                    }
                                    _ => {
                                        return Err(format!(
                                            "line {l}: `lines` must be a list of line numbers"
                                        ));
                                    }
                                }
                            }
                        }
                        ("rule" | "files" | "reason" | "lines", _) => {
                            return Err(format!("line {l}: `{key}` has the wrong form"));
                        }
                        _ => {
                            return Err(format!(
                                "line {l}: unknown key `{key}` in a waiver (expected rule, files, lines or reason)"
                            ));
                        }
                    }
                }
                if w.rule.is_empty() {
                    return Err(format!("line {l}: a waiver needs a `rule`"));
                }
                out.push(w);
            }
        }
        Ok(Waivers(out))
    }

    /// Whether a waiver covers this finding. A bare file name also matches that file under any
    /// directory.
    pub fn waives(&self, file: &str, rule: &str, line: usize) -> bool {
        let file = normalize(file);
        self.0.iter().any(|w| {
            (w.rule == rule || w.rule == "*")
                && w.files.as_ref().is_none_or(|p| {
                    glob(p.as_bytes(), file.as_bytes()) || Path::new(&file).ends_with(p)
                })
                && (w.lines.is_empty() || w.lines.contains(&line))
        })
    }
}

/// The nearest file called `name` in `dir` or its ancestors.
pub fn find_up(dir: &Path, name: &str) -> Option<PathBuf> {
    dir.ancestors().map(|d| d.join(name)).find(|p| p.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_of_the_configuration() {
        let definite = crate::rule("lint_770").unwrap();
        let on = Config::default();
        assert!(on.enabled(definite));
        assert!(on.severity(definite) == Severity::Error);
        let cfg = Config::parse(
            "rule:\n  global:\n    severity: warning\n  group:\n    definite:\n      disable: true\n  lint_771: {disable: false, severity: error}\n",
        )
        .unwrap();
        assert!(!cfg.enabled(definite));
        assert!(cfg.severity(definite) == Severity::Warning);
        let own = crate::rule("lint_771").unwrap();
        assert!(cfg.enabled(own));
        assert!(cfg.severity(own) == Severity::Error);
    }

    #[test]
    fn unknown_names_are_errors_that_name_them() {
        for (text, needle) in [
            ("vsg_rs: {}\n", "unknown key `vsg_rs`"),
            ("rule:\n  lint_100:\n    disable: false\n", "`lint_100`"),
            ("rule:\n  entity_019:\n    disable: false\n", "`entity_019`"),
            ("rule:\n  lint_770:\n    fixable: false\n", "`fixable`"),
            (
                "rule:\n  group:\n    style:\n      disable: true\n",
                "`style`",
            ),
            ("rule:\n  lint_770:\n    disable: maybe\n", "`maybe`"),
            ("rule:\n  lint_770:\n    severity: fatal\n", "`fatal`"),
            ("rule: [\n", "line"),
        ] {
            match Config::parse(text) {
                Err(e) => assert!(e.contains(needle), "{e} / {needle}"),
                Ok(_) => panic!("accepted: {text}"),
            }
        }
    }

    #[test]
    fn waivers_match_rule_file_and_line() {
        let w = Waivers::parse(
            "waivers:\n  - rule: lint_770\n    files: 'src/**/*.vhd'\n    lines: [7]\n    reason: legacy\n",
        )
        .unwrap();
        assert!(w.waives("src/a/b.vhd", "lint_770", 7));
        assert!(w.waives("./src/b.vhd", "lint_770", 7));
        assert!(!w.waives("src/a/b.vhd", "lint_770", 8));
        assert!(!w.waives("src/a/b.vhd", "lint_771", 7));
        assert!(!w.waives("test/b.vhd", "lint_770", 7));
        let w = Waivers::parse("waivers:\n  - rule: '*'\n    reason: all\n").unwrap();
        assert!(w.waives("any/file.vhd", "lint_771", 99));
        let w = Waivers::parse("waivers:\n  - rule: lint_770\n    files: b.vhd\n").unwrap();
        assert!(w.waives("src/b.vhd", "lint_770", 4000));
    }

    #[test]
    fn a_bad_waiver_file_is_an_error() {
        assert!(Waivers::parse("waivers:\n  - reason: nothing\n").is_err());
        assert!(Waivers::parse("waivers:\n  - rule: no_such\n").is_err());
        assert!(Waivers::parse("waivers:\n  - rule: lint_770\n    line: 3\n").is_err());
        assert!(Waivers::parse("waive: []\n").is_err());
        assert!(Waivers::parse("waivers:\n").unwrap().0.is_empty());
    }
}
