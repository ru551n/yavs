//! Coverage exclusion files (`yavs cov report --exclude FILE`): rows copied from
//! `yavs cov report --uncovered`, plus coarse rules. `#` starts a comment (the reason), blank
//! lines are ignored. Rows:
//!
//! ```text
//! statement <instance|glob> <12hex>.<occ> [file:line]
//! branch    <instance|glob> <12hex>.<occ>.<arm> [file:line]
//! toggle    <instance|glob> <scalar|glob>    both directions of a signal's scalar
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

/// An excluded item moved to another line than the row says.
pub const MOVED: &str = "W0110";
/// A row matches nothing in the database (the statement changed, or never existed).
pub const STALE: &str = "W0111";
/// A row excludes an item that ran.
pub const HIT: &str = "W0114";

#[derive(Debug, PartialEq, Eq)]
enum Rule {
    Item {
        branch: bool,
        glob: String,
        hex: String,
        occ: u32,
        arm: u32,
        at: Option<(String, u32)>,
    },
    Toggle(String, String),
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
            [t @ ("statement" | "branch"), glob, id, ref at @ ..] if at.len() <= 1 => {
                let branch = t == "branch";
                let p: Vec<&str> = id.split('.').collect();
                let n = if branch { 3 } else { 2 };
                if p.len() != n || p[0].len() != 12 || !p[0].bytes().all(|b| b.is_ascii_hexdigit())
                {
                    return Err(bad(if branch {
                        "expected `<12 hex>.<occurrence>.<arm>`"
                    } else {
                        "expected `<12 hex>.<occurrence>`"
                    }));
                }
                let num = |s: &str| s.parse::<u32>().map_err(|_| bad("not a number"));
                let at = match at.first() {
                    None => None,
                    Some(a) => {
                        let (f, n) = a
                            .rsplit_once(':')
                            .ok_or_else(|| bad("expected `file:line`"))?;
                        Some((f.to_string(), num(n)?))
                    }
                };
                Rule::Item {
                    branch,
                    glob: glob.to_string(),
                    hex: p[0].to_ascii_lowercase(),
                    occ: num(p[1])?,
                    arm: if branch { num(p[2])? } else { 0 },
                    at,
                }
            }
            ["toggle", g, sig] => Rule::Toggle(g.to_string(), sig.to_string()),
            ["instance", g] => Rule::Instance(g.to_string()),
            ["unit", u] => Rule::Unit(u.to_string()),
            ["file", g] => Rule::File(g.to_string()),
            [
                "statement" | "branch" | "toggle" | "instance" | "unit" | "file",
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
            branch,
            glob: g,
            hex,
            occ,
            arm,
            ..
        } => {
            let kind_ok = match k.kind {
                Kind::Statement => !branch,
                Kind::Branch(a) => *branch && a == *arm,
                Kind::Toggle(_) => false,
            };
            kind_ok
                && k.occ == *occ
                && format!("{:016x}", k.hash).starts_with(hex)
                && glob(g, &k.path)
        }
        Rule::Toggle(g, sig) => {
            matches!(k.kind, Kind::Toggle(_)) && glob(g, &k.path) && glob(sig, &k.sig)
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
            glob(g, &e.file)
                || e.file
                    .match_indices('/')
                    .any(|(i, _)| glob(g, &e.file[i + 1..]))
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
                // A toggle row excludes both directions: it warns once, when both ran.
                let ran = match k.kind {
                    Kind::Toggle(true) => {
                        e.count > 0 && partner(db, k).is_some_and(|p| p.count > 0)
                    }
                    Kind::Toggle(false) => false,
                    _ => e.count > 0,
                };
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
        Kind::Toggle(_) => format!("toggle {} {}", k.path, k.sig),
    }
}

/// The trailing comment of an item's report row: the arm and the statement text, or the
/// toggle direction and the declaration.
pub fn comment(k: &Key, e: &Entry) -> String {
    match k.kind {
        Kind::Statement => e.text.clone(),
        Kind::Branch(a) => format!("arm {a}: {}", e.text),
        Kind::Toggle(r) => format!("{} ({}:{})", crate::arrow(r), e.file, e.line),
    }
}

impl Database {
    /// The report of `--uncovered`: one exclusion row per item that never ran, in source
    /// order. Change nothing, paste into an exclusion file. A scalar that never toggled
    /// either way is one row.
    pub fn uncovered(&self) -> String {
        let mut v: Vec<_> = self
            .items
            .iter()
            .filter(|(k, e)| {
                e.count == 0
                    && !(k.kind == Kind::Toggle(true)
                        && partner(self, k).is_some_and(|p| p.count == 0))
            })
            .collect();
        v.sort_by(|a, b| (&a.1.file, a.1.line, a.0).cmp(&(&b.1.file, b.1.line, b.0)));
        v.iter()
            .map(|(k, e)| {
                let c = match k.kind {
                    Kind::Toggle(false) if partner(self, k).is_some_and(|p| p.count == 0) => {
                        format!("never toggled ({}:{})", e.file, e.line)
                    }
                    Kind::Toggle(r) => format!("never {} ({}:{})", crate::arrow(r), e.file, e.line),
                    _ => comment(k, e),
                };
                format!("{}   # {c}\n", head(k, e))
            })
            .collect()
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
    fn syntax_errors_name_the_line() {
        assert!(parse("frobnicate x\n").unwrap_err().starts_with("line 1:"));
        assert!(
            parse("# c\n\nstatement a b\n")
                .unwrap_err()
                .starts_with("line 3:")
        );
        assert!(parse("branch * abcdef012345.0\n").is_err());
    }
}
