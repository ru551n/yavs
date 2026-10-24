//! The YAML form of an exclusion file (`*.yaml`, `*.yml`):
//!
//! ```yaml
//! exclude:
//!   - kind: statement          # statement | branch | condition | expression | toggle | instance | unit | file
//!     instance: ":tb:dut:u_fsm"
//!     hash: "4d2f0a91c3be.0"   # branch: HEX12.OCC.ARM
//!     line: "src/fsm.vhd:61"   # optional, never matched
//!     reason: free text        # optional
//! ```
//!
//! A `condition` or `expression` entry has `hash: "HEX12.OCC"` and `row: N` (2 * input + value).
//! A `toggle` entry has `signal` instead of `hash` and an optional `direction: rise|fall`.
//!
//! Every value is read as the text it was written as, so `no`, `on` and a hash of digits only
//! stay strings. The parser is `saphyr-parser` (events with line numbers), not a data model.

use super::{ItemKind, Row, Rule, direction, item_rule};
use saphyr_parser::{Event, Parser};

enum Node {
    Scalar(String),
    Seq(Vec<(usize, Node)>),
    Map(Vec<(usize, String, Node)>),
}

type Events<'a> = Parser<'a, saphyr_parser::StrInput<'a>>;

fn err(line: usize, m: &str) -> String {
    format!("line {line}: {m}")
}

/// The next event with its 1-based line.
fn next<'a>(p: &mut Events<'a>) -> Result<(Event<'a>, usize), String> {
    match p.next() {
        Some(Ok((ev, sp))) => Ok((ev, sp.start.line())),
        Some(Err(e)) => Err(err(e.marker().line(), e.info())),
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
                v.push((l, node(p, ev, l)?));
            }
        }
        Event::MappingStart(..) => {
            let mut v: Vec<(usize, String, Node)> = Vec::new();
            loop {
                let (ev, l) = next(p)?;
                let key = match ev {
                    Event::MappingEnd => return Ok(Node::Map(v)),
                    Event::Scalar(k, ..) => k.to_string(),
                    _ => return Err(err(l, "a key must be a plain word")),
                };
                if v.iter().any(|(_, k, _)| *k == key) {
                    return Err(err(l, &format!("duplicate key `{key}`")));
                }
                let (ev, vl) = next(p)?;
                v.push((l, key, node(p, ev, vl)?));
            }
        }
        Event::Alias(_) => Err(err(line, "aliases are not supported")),
        _ => Err(err(line, "unexpected YAML structure")),
    }
}

/// Parse a YAML exclusion file; the error names the line.
pub fn parse(text: &str) -> Result<Vec<Row>, String> {
    let mut p = Parser::new_from_str(text);
    let mut root = None;
    loop {
        let (ev, l) = next(&mut p)?;
        match ev {
            Event::StreamStart | Event::DocumentStart(_) | Event::DocumentEnd => {}
            Event::StreamEnd => break,
            ev if root.is_none() => root = Some((l, node(&mut p, ev, l)?)),
            _ => return Err(err(l, "more than one YAML document")),
        }
    }
    let Some((_, root)) = root else {
        return Ok(Vec::new());
    };
    let Node::Map(top) = root else {
        return Err(err(1, "expected a top-level `exclude:` list"));
    };
    let mut rows = Vec::new();
    for (kl, key, v) in top {
        if key != "exclude" {
            return Err(err(
                kl,
                &format!("unknown key `{key}` (expected `exclude`)"),
            ));
        }
        match v {
            Node::Seq(items) => {
                for (il, item) in items {
                    rows.push(entry(il, item)?);
                }
            }
            // `exclude:` with every entry commented out
            Node::Scalar(s) if s.is_empty() => {}
            _ => return Err(err(kl, "`exclude` must be a list of entries")),
        }
    }
    Ok(rows)
}

fn entry(line: usize, n: Node) -> Result<Row, String> {
    let Node::Map(fields) = n else {
        return Err(err(
            line,
            "an entry must be a map with `kind:` and its fields",
        ));
    };
    let mut get: Vec<(usize, &str, &str)> = Vec::new();
    for (l, k, v) in &fields {
        match v {
            Node::Scalar(s) => get.push((*l, k, s)),
            _ => return Err(err(*l, &format!("`{k}` must be a single value"))),
        }
    }
    let find = |k: &str| get.iter().find(|f| f.1 == k).map(|f| f.2);
    let kind = find("kind").ok_or_else(|| err(line, "missing `kind`"))?;
    let (allowed, need): (&[&str], &[&str]) = match kind {
        "statement" | "branch" => (
            &["kind", "instance", "hash", "line", "reason"],
            &["instance", "hash"],
        ),
        "condition" | "expression" => (
            &["kind", "instance", "hash", "row", "line", "reason"],
            &["instance", "hash", "row"],
        ),
        "toggle" => (
            &["kind", "instance", "signal", "direction", "reason"],
            &["instance", "signal"],
        ),
        "instance" => (&["kind", "instance", "reason"], &["instance"]),
        "unit" => (&["kind", "unit", "reason"], &["unit"]),
        "file" => (&["kind", "file", "reason"], &["file"]),
        _ => {
            let l = get.iter().find(|f| f.1 == "kind").map_or(line, |f| f.0);
            return Err(err(l, &format!("unknown exclusion kind `{kind}`")));
        }
    };
    for (l, k, _) in &get {
        if !allowed.contains(k) {
            return Err(err(*l, &format!("unknown key `{k}` for kind `{kind}`")));
        }
    }
    for k in need {
        if find(k).is_none_or(str::is_empty) {
            return Err(err(line, &format!("kind `{kind}` needs `{k}`")));
        }
    }
    let bad = |m: &str| err(line, &format!("{m}: kind `{kind}`"));
    let rule = match kind {
        "statement" | "branch" | "condition" | "expression" => item_rule(
            ItemKind::word(kind).unwrap_or(ItemKind::Statement),
            find("instance").unwrap_or_default(),
            find("hash").unwrap_or_default(),
            find("row"),
            find("line").filter(|s| !s.is_empty()),
            &bad,
        )?,
        "toggle" => Rule::Toggle(
            find("instance").unwrap_or_default().into(),
            find("signal").unwrap_or_default().into(),
            find("direction").map(|d| direction(d, &bad)).transpose()?,
        ),
        "instance" => Rule::Instance(find("instance").unwrap_or_default().into()),
        "unit" => Rule::Unit(find("unit").unwrap_or_default().into()),
        _ => Rule::File(find("file").unwrap_or_default().into()),
    };
    Ok(Row {
        rule,
        reason: find("reason").unwrap_or_default().trim().to_string(),
        line,
    })
}

/// `s` as a YAML double-quoted scalar.
pub fn quote(s: &str) -> String {
    let mut o = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            c if c.is_control() => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}
