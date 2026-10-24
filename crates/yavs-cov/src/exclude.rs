//! Coverage exclusion files (`yavs cov report --exclude FILE`): rows copied from
//! `yavs cov report --uncovered`, plus coarse rules. `#` starts a comment (the reason), blank
//! lines are ignored. Rows:
//!
//! ```text
//! statement <instance|glob> <12hex>.<occ> [file:line]
//! branch    <instance|glob> <12hex>.<occ>.<arm> [file:line]
//! condition <instance|glob> <12hex>.<occ> <row> [file:line]   a row of a condition (2 * input + value)
//! expression <instance|glob> <12hex>.<occ> <row> [file:line]  the same for a logic expression
//! toggle    <instance|glob> <scalar|glob> [rise|fall]   a signal's scalar, both directions unless one is named
//! instance  <glob>          that instance and everything below it
//! unit      <lib.package>   a package's items
//! file      <glob>          every item of matching source files
//! ```
//!
//! Items match by instance, kind, hash prefix, occurrence and arm; `file:line` is display only
//! and a changed line is a warning. Exclusions apply when reporting: the database keeps raw
//! counts.

use crate::{Database, Entry, Key, Kind};
use yavs_diag::Diagnostic;

mod yaml;
pub use yaml::parse as parse_yaml;

/// An excluded item moved to another line than the row says.
pub const MOVED: &str = "W0110";
/// A row matches nothing in the database (the statement changed, or never existed).
pub const STALE: &str = "W0111";
/// A row excludes an item that ran.
pub const HIT: &str = "W0114";

/// What an item rule excludes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ItemKind {
    Statement,
    Branch,
    Condition,
    Expression,
}

impl ItemKind {
    fn word(word: &str) -> Option<ItemKind> {
        Some(match word {
            "statement" => ItemKind::Statement,
            "branch" => ItemKind::Branch,
            "condition" => ItemKind::Condition,
            "expression" => ItemKind::Expression,
            _ => return None,
        })
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Rule {
    Item {
        kind: ItemKind,
        glob: String,
        hex: String,
        occ: u32,
        /// The branch arm, or the row of a condition or expression.
        arm: u32,
        at: Option<(String, u32)>,
    },
    /// Path glob, scalar glob, direction (`Some(true)` rise, `Some(false)` fall, `None` both).
    Toggle(String, String, Option<bool>),
    Instance(String),
    Unit(String),
    File(String),
}

/// One row of an exclusion file.
#[derive(Debug)]
pub struct Row {
    rule: Rule,
    reason: String,
    /// 1-based line in the exclusion file.
    line: usize,
}

/// The result of [`apply`].
pub struct Applied {
    /// The database without the excluded items.
    pub kept: Database,
    /// The excluded items with the reason of the row that took them out.
    pub excluded: Vec<(Key, Entry, String)>,
    pub warnings: Vec<Diagnostic>,
}

fn glob_match(g: &[u8], s: &[u8]) -> bool {
    match g.split_first() {
        None => s.is_empty(),
        Some((b'*', r)) => (0..=s.len()).any(|i| glob_match(r, &s[i..])),
        Some((b'?', r)) => !s.is_empty() && glob_match(r, &s[1..]),
        Some((c, r)) => {
            s.first().is_some_and(|d| d.eq_ignore_ascii_case(c)) && glob_match(r, &s[1..])
        }
    }
}

fn glob(g: &str, s: &str) -> bool {
    glob_match(g.as_bytes(), s.as_bytes())
}

/// `rise` (0->1) or `fall` (1->0) as the `bool` of [`Kind::Toggle`].
fn direction(d: &str, bad: &dyn Fn(&str) -> String) -> Result<bool, String> {
    match d {
        "rise" => Ok(true),
        "fall" => Ok(false),
        _ => Err(bad("expected `rise` or `fall`")),
    }
}

/// A `statement`, `branch`, `condition` or `expression` rule from its fields (shared by the text
/// and YAML formats): `id` is `HEX12.OCC`, with `.ARM` for a branch; `row` is the row of a
/// condition or expression.
fn item_rule(
    kind: ItemKind,
    glob: &str,
    id: &str,
    row: Option<&str>,
    at: Option<&str>,
    bad: &dyn Fn(&str) -> String,
) -> Result<Rule, String> {
    let p: Vec<&str> = id.split('.').collect();
    let branch = kind == ItemKind::Branch;
    let n = if branch { 3 } else { 2 };
    if p.len() != n || p[0].len() != 12 || !p[0].bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(bad(if branch {
            "expected `<12 hex>.<occurrence>.<arm>`"
        } else {
            "expected `<12 hex>.<occurrence>`"
        }));
    }
    let num = |s: &str| s.parse::<u32>().map_err(|_| bad("not a number"));
    let at = match at {
        None => None,
        Some(a) => {
            let (f, n) = a
                .rsplit_once(':')
                .ok_or_else(|| bad("expected `file:line`"))?;
            Some((f.to_string(), num(n)?))
        }
    };
    let arm = match (kind, row) {
        (ItemKind::Branch, _) => num(p[2])?,
        (ItemKind::Condition | ItemKind::Expression, Some(r)) => num(r)?,
        (ItemKind::Condition | ItemKind::Expression, None) => return Err(bad("missing row")),
        _ => 0,
    };
    Ok(Rule::Item {
        kind,
        glob: glob.to_string(),
        hex: p[0].to_ascii_lowercase(),
        occ: num(p[1])?,
        arm,
        at,
    })
}

/// Parse an exclusion file; the error names the line.
pub fn parse(text: &str) -> Result<Vec<Row>, String> {
    let mut rows = Vec::new();
    for (i, l) in text.lines().enumerate() {
        let (body, reason) = match l.split_once('#') {
            Some((b, r)) => (b, r.trim()),
            None => (l, ""),
        };
        let w: Vec<&str> = body.split_whitespace().collect();
        if w.is_empty() {
            continue;
        }
        let bad = |m: &str| format!("line {}: {m}: `{}`", i + 1, body.trim());
        let rule = match w[..] {
            [t @ ("statement" | "branch"), glob, id, ref at @ ..] if at.len() <= 1 => item_rule(
                ItemKind::word(t).unwrap_or(ItemKind::Statement),
                glob,
                id,
                None,
                at.first().copied(),
                &bad,
            )?,
            [t @ ("condition" | "expression"), glob, id, row, ref at @ ..] if at.len() <= 1 => {
                item_rule(
                    ItemKind::word(t).unwrap_or(ItemKind::Condition),
                    glob,
                    id,
                    Some(row),
                    at.first().copied(),
                    &bad,
                )?
            }
            ["toggle", g, sig] => Rule::Toggle(g.to_string(), sig.to_string(), None),
            ["toggle", g, sig, d] => {
                Rule::Toggle(g.to_string(), sig.to_string(), Some(direction(d, &bad)?))
            }
            ["instance", g] => Rule::Instance(g.to_string()),
            ["unit", u] => Rule::Unit(u.to_string()),
            ["file", g] => Rule::File(g.to_string()),
            [
                "statement" | "branch" | "condition" | "expression" | "toggle" | "instance"
                | "unit" | "file",
                ..,
            ] => {
                return Err(bad("wrong number of fields"));
            }
            [t, ..] => return Err(bad(&format!("unknown exclusion type `{t}`"))),
            [] => unreachable!(),
        };
        rows.push(Row {
            rule,
            reason: reason.to_string(),
            line: i + 1,
        });
    }
    Ok(rows)
}

fn matches(r: &Rule, k: &Key, e: &Entry) -> bool {
    match r {
        Rule::Item {
            kind,
            glob: g,
            hex,
            occ,
            arm,
            ..
        } => {
            let kind_ok = match k.kind {
                Kind::Statement => *kind == ItemKind::Statement,
                Kind::Branch(a) => *kind == ItemKind::Branch && a == *arm,
                Kind::Condition(r) => *kind == ItemKind::Condition && r == *arm,
                Kind::Expression(r) => *kind == ItemKind::Expression && r == *arm,
                Kind::Toggle(_) => false,
            };
            kind_ok
                && k.occ == *occ
                && format!("{:016x}", k.hash).starts_with(hex)
                && glob(g, &k.path)
        }
        Rule::Toggle(g, sig, d) => {
            matches!(k.kind, Kind::Toggle(r) if d.is_none_or(|d| d == r))
                && glob(g, &k.path)
                && glob(sig, &k.sig)
        }
        Rule::Instance(g) => {
            k.path.starts_with(':')
                && (glob(g, &k.path)
                    || k.path
                        .match_indices(':')
                        .skip(1)
                        .any(|(i, _)| glob(g, &k.path[..i])))
        }
        Rule::Unit(u) => k.path.eq_ignore_ascii_case(u),
        Rule::File(g) => {
            // Either separator in the row and in the recorded path (Windows paths use `\`).
            let (g, f) = (g.replace('\\', "/"), e.file.replace('\\', "/"));
            glob(&g, &f) || f.match_indices('/').any(|(i, _)| glob(&g, &f[i + 1..]))
        }
    }
}

/// Take the items the rows match out of `db`, with warnings for rows that match nothing,
/// moved lines and excluded items that ran.
pub fn apply(db: &Database, rows: &[Row], exfile: &str) -> Applied {
    let mut out = Applied {
        kept: Database {
            tests: db.tests.clone(),
            ..Default::default()
        },
        excluded: Vec::new(),
        warnings: Vec::new(),
    };
    let mut seen = vec![0usize; rows.len()];
    for (k, e) in &db.items {
        let mut hit = None;
        for (i, r) in rows.iter().enumerate() {
            if matches(&r.rule, k, e) {
                seen[i] += 1;
                hit.get_or_insert(i);
                if let Rule::Item {
                    at: Some((_, l)), ..
                } = &r.rule
                    && *l != e.line
                {
                    out.warnings.push(Diagnostic::warning(
                        MOVED,
                        format!(
                            "{exfile}:{}: the excluded statement is now at {}:{}, not line {l}",
                            r.line, e.file, e.line
                        ),
                    ));
                }
                let ran = e.count > 0;
                if ran {
                    out.warnings.push(Diagnostic::warning(
                        HIT,
                        format!(
                            "{exfile}:{}: the excluded item at {}:{} ran {} times",
                            r.line, e.file, e.line, e.count
                        ),
                    ));
                }
            }
        }
        match hit {
            Some(i) => out
                .excluded
                .push((k.clone(), e.clone(), rows[i].reason.clone())),
            None => {
                out.kept.items.insert(k.clone(), e.clone());
            }
        }
    }
    for (r, n) in rows.iter().zip(seen) {
        if n == 0 {
            out.warnings.push(Diagnostic::warning(
                STALE,
                format!(
                    "{exfile}:{}: the exclusion matches nothing in the coverage database",
                    r.line
                ),
            ));
        }
    }
    out
}

/// The other direction of a toggle item.
fn partner<'a>(db: &'a Database, k: &Key) -> Option<&'a Entry> {
    let Kind::Toggle(r) = k.kind else { return None };
    db.items.get(&Key {
        kind: Kind::Toggle(!r),
        ..k.clone()
    })
}

/// An item as an exclusion row, without the trailing comment.
pub fn head(k: &Key, e: &Entry) -> String {
    let id = format!("{:012x}.{}", k.hash >> 16, k.occ);
    match k.kind {
        Kind::Statement => format!("statement {} {id} {}:{}", k.path, e.file, e.line),
        Kind::Branch(a) => format!("branch {} {id}.{a} {}:{}", k.path, e.file, e.line),
        Kind::Toggle(r) => format!("toggle {} {} {}", k.path, k.sig, dir_word(r)),
        Kind::Condition(r) => format!("condition {} {id} {r} {}:{}", k.path, e.file, e.line),
        Kind::Expression(r) => format!("expression {} {id} {r} {}:{}", k.path, e.file, e.line),
    }
}

fn dir_word(rise: bool) -> &'static str {
    if rise { "rise" } else { "fall" }
}

/// The trailing comment of an item's report row: the arm and the statement text, or the
/// toggle direction and the declaration.
pub fn comment(k: &Key, e: &Entry) -> String {
    match k.kind {
        Kind::Statement => e.text.clone(),
        Kind::Branch(a) => format!("arm {a}: {}", e.text),
        Kind::Toggle(r) => format!("{} ({}:{})", crate::arrow(r), e.file, e.line),
        Kind::Condition(_) | Kind::Expression(_) => crate::fec_text(k, e),
    }
}

/// The comment of an item that never ran (`None` for a hit item, and for the rise direction
/// of a scalar whose fall row already says it never toggled).
fn miss_comment(db: &Database, k: &Key, e: &Entry) -> Option<String> {
    let both = both_missed(db, k);
    if e.count > 0 || (k.kind == Kind::Toggle(true) && both) {
        return None;
    }
    Some(match k.kind {
        Kind::Toggle(false) if both => format!("never toggled ({}:{})", e.file, e.line),
        Kind::Toggle(r) => format!("never {} ({}:{})", crate::arrow(r), e.file, e.line),
        _ => comment(k, e),
    })
}

/// The `--uncovered` row of an item that never ran (`None` for a hit item, and for the rise
/// direction of a scalar whose fall row already says it never toggled).
pub fn miss_row(db: &Database, k: &Key, e: &Entry) -> Option<String> {
    let c = miss_comment(db, k, e)?;
    let h = match k.kind {
        // Both directions missed: one row for the scalar, no direction.
        Kind::Toggle(_) if both_missed(db, k) => format!("toggle {} {}", k.path, k.sig),
        _ => head(k, e),
    };
    Some(format!("{h}   # {c}\n"))
}

fn both_missed(db: &Database, k: &Key) -> bool {
    matches!(k.kind, Kind::Toggle(_)) && partner(db, k).is_some_and(|p| p.count == 0)
}

/// The same as a YAML list entry (`--uncovered=yaml`), the comment after `kind`.
pub fn miss_yaml(db: &Database, k: &Key, e: &Entry) -> Option<String> {
    use yaml::quote as q;
    let c = miss_comment(db, k, e)?.replace(['\r', '\n'], " ");
    let id = format!("{:012x}.{}", k.hash >> 16, k.occ);
    let at = q(&format!("{}:{}", e.file, e.line));
    Some(match k.kind {
        Kind::Statement => format!(
            "  - kind: statement   # {c}\n    instance: {}\n    hash: {}\n    line: {at}\n",
            q(&k.path),
            q(&id)
        ),
        Kind::Branch(a) => format!(
            "  - kind: branch   # {c}\n    instance: {}\n    hash: {}\n    line: {at}\n",
            q(&k.path),
            q(&format!("{id}.{a}"))
        ),
        Kind::Condition(r) | Kind::Expression(r) => format!(
            "  - kind: {}   # {c}\n    instance: {}\n    hash: {}\n    row: {r}\n    line: {at}\n",
            if matches!(k.kind, Kind::Condition(_)) {
                "condition"
            } else {
                "expression"
            },
            q(&k.path),
            q(&id)
        ),
        Kind::Toggle(r) => format!(
            "  - kind: toggle   # {c}\n    instance: {}\n    signal: {}\n{}",
            q(&k.path),
            q(&k.sig),
            if both_missed(db, k) {
                String::new()
            } else {
                format!("    direction: {}\n", dir_word(r))
            }
        ),
    })
}

impl Database {
    fn in_source_order(&self) -> Vec<(&Key, &Entry)> {
        let mut v: Vec<_> = self.items.iter().collect();
        v.sort_by(|a, b| (&a.1.file, a.1.line, a.0).cmp(&(&b.1.file, b.1.line, b.0)));
        v
    }

    /// The report of `--uncovered`: one exclusion row per item that never ran, in source
    /// order. Change nothing, paste into an exclusion file. A scalar that never toggled
    /// either way is one row.
    pub fn uncovered(&self) -> String {
        let v = self.in_source_order();
        v.iter().filter_map(|(k, e)| miss_row(self, k, e)).collect()
    }

    /// `--uncovered=yaml`: the same items as a YAML exclusion file.
    pub fn uncovered_yaml(&self) -> String {
        let v = self.in_source_order();
        let rows: String = v
            .iter()
            .filter_map(|(k, e)| miss_yaml(self, k, e))
            .collect();
        if rows.is_empty() {
            "exclude: []\n".into()
        } else {
            format!("exclude:\n{rows}")
        }
    }
}

impl Database {
    /// The report of `--tests`: every item some test hit, as an exclusion row with the tests
    /// that hit it, in source order.
    pub fn hits_by_test(&self) -> String {
        let mut v: Vec<_> = self
            .items
            .iter()
            .filter(|(_, e)| !e.tests.is_empty())
            .collect();
        v.sort_by(|a, b| (&a.1.file, a.1.line, a.0).cmp(&(&b.1.file, b.1.line, b.0)));
        v.iter()
            .map(|(k, e)| {
                let t: Vec<&str> = e
                    .tests
                    .iter()
                    .map(|&t| self.tests[t as usize].as_str())
                    .collect();
                format!("{}   # tests: {}\n", head(k, e), t.join(", "))
            })
            .collect()
    }
}

/// The text block listing excluded items and their reasons.
pub fn excluded_text(ex: &[(Key, Entry, String)]) -> String {
    if ex.is_empty() {
        return String::new();
    }
    let mut v: Vec<_> = ex.iter().collect();
    v.sort_by(|a, b| (&a.1.file, a.1.line, &a.0).cmp(&(&b.1.file, b.1.line, &b.0)));
    let mut s = format!("\nexcluded ({}, not in the totals):\n", v.len());
    for (k, e, why) in v {
        s.push_str(&format!(
            "  {}   # {}{}\n",
            head(k, e),
            comment(k, e),
            if why.is_empty() {
                String::new()
            } else {
                format!("  [{why}]")
            }
        ));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Database {
        let mut d = Database::default();
        for (p, kind, hash, line, count) in [
            (
                ":t:u1",
                Kind::Statement,
                0xabcd_ef01_2345_0000u64 | 7,
                10,
                0,
            ),
            (":t:u1", Kind::Branch(2), 0x1111_2222_3333_0000, 11, 0),
            (":t:u2", Kind::Statement, 0xabcd_ef01_2345_0000 | 7, 10, 4),
            ("work.pkg", Kind::Statement, 9 << 16, 3, 0),
        ] {
            d.items.insert(
                Key {
                    path: p.into(),
                    kind,
                    hash,
                    occ: 0,
                    sig: String::new(),
                },
                Entry {
                    file: "src/a.vhd".into(),
                    line,
                    count,
                    text: "x;".into(),
                    tests: Vec::new(),
                },
            );
        }
        d
    }

    #[test]
    fn report_rows_round_trip() {
        let d = db();
        // `--uncovered` rows with MISS-free shape are the exclusion file.
        let rows = parse(&d.uncovered()).unwrap();
        let a = apply(&d, &rows, "ex");
        assert_eq!(a.excluded.len(), 3);
        assert_eq!(a.kept.items.len(), 1);
        assert!(
            a.warnings.is_empty(),
            "{:?}",
            a.warnings.iter().map(|w| &w.message).collect::<Vec<_>>()
        );
    }

    #[test]
    fn warnings_and_coarse_rules() {
        let d = db();
        let t = "statement * abcdef012345.0 src/a.vhd:99 # moved, and u2 ran\n\
                 branch :t:u1 111122223333.0.3\n\
                 instance :t:u1\nunit WORK.pkg\nfile a.vhd\n";
        let a = apply(&d, &parse(t).unwrap(), "ex");
        let codes: Vec<_> = a.warnings.iter().map(|w| w.code).collect();
        // moved twice (u1, u2), hit once (u2), stale once (arm 3).
        assert_eq!(
            codes.iter().filter(|c| **c == MOVED).count(),
            2,
            "{codes:?}"
        );
        assert_eq!(codes.iter().filter(|c| **c == HIT).count(), 2, "{codes:?}");
        assert_eq!(
            codes.iter().filter(|c| **c == STALE).count(),
            1,
            "{codes:?}"
        );
        assert!(a.kept.items.is_empty());
    }

    #[test]
    fn file_rows_ignore_path_separator_style() {
        let mut d = db();
        for e in d.items.values_mut() {
            e.file = "src\\sub\\a.vhd".into();
        }
        for row in [
            "file a.vhd",
            "file sub/a.vhd",
            "file sub\\a.vhd",
            "file src/*/a.vhd",
        ] {
            let a = apply(&d, &parse(&format!("{row}\r\n")).unwrap(), "ex");
            assert!(a.kept.items.is_empty(), "{row}");
        }
    }

    #[test]
    fn syntax_errors_name_the_line() {
        assert!(parse("frobnicate x\n").unwrap_err().starts_with("line 1:"));
        assert!(
            parse("# c\n\nstatement a b\n")
                .unwrap_err()
                .starts_with("line 3:")
        );
        assert!(parse("branch * abcdef012345.0\n").is_err());
    }

    #[test]
    fn yaml_round_trips_the_uncovered_report() {
        let d = db();
        let y = d.uncovered_yaml();
        let a = apply(&d, &parse_yaml(&y).unwrap(), "ex");
        let b = apply(&d, &parse(&d.uncovered()).unwrap(), "ex");
        assert_eq!(a.excluded.len(), 3);
        assert_eq!(
            a.excluded.iter().map(|x| &x.0).collect::<Vec<_>>(),
            b.excluded.iter().map(|x| &x.0).collect::<Vec<_>>()
        );
        assert!(a.warnings.is_empty(), "{y}");
        assert_eq!(parse_yaml("").unwrap().len(), 0);
        assert_eq!(parse_yaml("exclude: []\n").unwrap().len(), 0);
        assert_eq!(parse_yaml("# only\nexclude:\n").unwrap().len(), 0);
    }

    #[test]
    fn yaml_every_kind_and_the_string_pitfalls() {
        let d = db();
        let t = r#"exclude:
  - kind: statement
    instance: ":t:u2"
    hash: abcdef012345.0
    line: src/a.vhd:99
    reason: no
  - {kind: branch, instance: ':t:u1', hash: 111122223333.0.2}
  - kind: unit
    unit: work.pkg
  - kind: toggle
    instance: ':t:*'
    signal: 'd(*)'
  - kind: instance
    instance: ':nothing'
    reason: on
  - kind: file
    file: "*/zzz/*.vhd"
"#;
        let rows = parse_yaml(t).unwrap();
        assert_eq!(rows.len(), 6);
        assert_eq!(rows[0].reason, "no");
        assert_eq!(rows[4].reason, "on");
        assert_eq!(rows[0].line, 2);
        let a = apply(&d, &rows, "ex");
        // u2 statement (ran, line moved), u1 branch, pkg; toggle, instance and file match nothing.
        assert_eq!(a.excluded.len(), 3);
        let codes: Vec<_> = a.warnings.iter().map(|w| w.code).collect();
        assert_eq!(
            codes.iter().filter(|c| **c == MOVED).count(),
            1,
            "{codes:?}"
        );
        assert_eq!(codes.iter().filter(|c| **c == HIT).count(), 1, "{codes:?}");
        assert_eq!(
            codes.iter().filter(|c| **c == STALE).count(),
            3,
            "{codes:?}"
        );
        // All-digit and float-looking hashes stay text: `123456789012.0` is a float in YAML.
        let n = parse_yaml("exclude:\n  - {kind: statement, instance: x, hash: 123456789012.0}\n");
        assert!(n.is_ok(), "{n:?}");
        let n = parse_yaml("exclude:\n  - {kind: statement, instance: x, hash: 1e5000000000.0}\n");
        assert!(n.is_ok(), "{n:?}");
    }

    #[test]
    fn yaml_errors_name_the_line() {
        for (t, line, what) in [
            (
                "exclude:\n  - kind: frob\n",
                "line 2:",
                "unknown exclusion kind",
            ),
            (
                "exclude:\n  - kind: instance\n    instance: x\n    colour: red\n",
                "line 4:",
                "unknown key",
            ),
            (
                "exclude:\n  - kind: instance\n    hash: x\n",
                "line 3:",
                "unknown key",
            ),
            (
                "exclude:\n  - kind: instance\n",
                "line 2:",
                "needs `instance`",
            ),
            ("exclude:\n  - instance: x\n", "line 2:", "missing `kind`"),
            (
                "exclude:\n  - kind: statement\n    instance: x\n    hash: zz\n",
                "line 2:",
                "expected",
            ),
            ("excludes: []\n", "line 1:", "unknown key"),
            ("exclude: 5\n", "line 1:", "list"),
            ("exclude:\n  - [a]\n", "line 2:", "map"),
            ("exclude:\n  - kind: [a]\n", "line 2:", "single value"),
            ("exclude:\n  - kind: file\n    file: *x\n", "line 3:", ""),
            ("exclude:\n  - kind: file\n    file: [\n", "line", ""),
            ("- a\n", "line 1:", "top-level"),
            ("exclude: []\n---\nexclude: []\n", "line", "document"),
        ] {
            let e = parse_yaml(t).unwrap_err();
            assert!(e.starts_with(line) && e.contains(what), "{t:?}: {e}");
        }
    }

    #[test]
    fn toggle_directions() {
        let mut d = Database::default();
        for (sig, rise, fall) in [("a", 0, 3), ("b", 0, 0), ("c", 2, 2)] {
            for (r, n) in [(true, rise), (false, fall)] {
                d.items.insert(
                    Key {
                        path: ":t".into(),
                        kind: Kind::Toggle(r),
                        hash: 0,
                        occ: 0,
                        sig: sig.into(),
                    },
                    Entry {
                        file: "s.vhd".into(),
                        line: 4,
                        count: n,
                        text: String::new(),
                        tests: Vec::new(),
                    },
                );
            }
        }
        // A scalar missed one way names the direction; missed both ways, none.
        let u = d.uncovered();
        assert!(u.contains("toggle :t a rise   # never 0->1"), "{u}");
        assert!(u.contains("toggle :t b   # never toggled"), "{u}");
        assert_eq!(u.lines().count(), 2, "{u}");
        let y = d.uncovered_yaml();
        assert!(y.contains("signal: \"a\"\n    direction: rise\n"), "{y}");
        assert_eq!(y.matches("direction").count(), 1, "{y}");
        for rows in [parse(&u).unwrap(), parse_yaml(&y).unwrap()] {
            let a = apply(&d, &rows, "ex");
            assert_eq!(a.excluded.len(), 3, "a rise, b rise and fall");
            assert!(a.warnings.is_empty());
        }
        // One direction only; W0114 per excluded direction that ran.
        let a = apply(&d, &parse("toggle :t * fall\n").unwrap(), "ex");
        assert_eq!(a.excluded.len(), 3);
        assert_eq!(a.warnings.iter().filter(|w| w.code == HIT).count(), 2);
        let a = apply(&d, &parse("toggle :t c\n").unwrap(), "ex");
        assert_eq!(a.warnings.iter().filter(|w| w.code == HIT).count(), 2);
        let y = "exclude:\n  - {kind: toggle, instance: ':t', signal: c, direction: rise}\n";
        let a = apply(&d, &parse_yaml(y).unwrap(), "ex");
        assert_eq!(a.excluded.len(), 1);
        assert!(parse("toggle :t c up\n").is_err());
        assert!(parse_yaml(&y.replace("rise", "up")).is_err());
    }
}
