//! Code coverage (docs/plan.md rule 10, M9): statement and branch counts of a run, keyed by
//! hierarchical path and a hash of the source text, in a database that runs merge into, with a
//! text summary and a Cobertura XML report.
//!
//! Lowering emits the counters as ordinary IR (`Inst::Cover`, `LowerOptions::coverage`), so
//! every tier counts them; the runtime holds one row of counters per region
//! (`Elaborated::cov_rows`). [`collect`] turns the rows into a [`Database`]:
//!
//! - an item of an architecture or entity is counted per instance, under the instance's path
//!   (`:top:u1`), summed over its blocks and generate iterations;
//! - an item of a package (subprograms) is counted once, under `library.package`, summed over
//!   every caller.
//!
//! An item's identity ([`Key`]) is that path, its kind and branch arm, a hash of its
//! statement's source text (whitespace runs count as one space) and, for statements with the
//! same text in one unit, their order. Byte offsets never enter it, so an edit elsewhere in
//! the file keeps the databases of older runs mergeable.

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;

use yavs_diag::Diagnostic;
use yavs_elab::{Case, Elaborated, ScopeId, ScopeKind};
use yavs_sem::Db;
use yavs_sem::loader::UnitKind;

/// What an item counts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Kind {
    Statement,
    /// Arm `n` of an `if`, `case`, conditional or selected assignment, or the body of a loop
    /// (arm 0). An `if` without `else` and a conditional assignment without a final `else`
    /// have one more arm: no condition held.
    Branch(u32),
}

/// The identity of an item, the same in every run of the design.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Key {
    /// Instance path (`:top:u1`) or package (`work.pkg`).
    pub path: String,
    pub kind: Kind,
    /// FNV-1a of the statement's source text.
    pub hash: u64,
    /// Among statements of one unit with the same text: the position in source order.
    pub occ: u32,
}

/// Where an item is and how often it ran.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub file: String,
    pub line: u32,
    pub count: u64,
    /// First line of the statement, whitespace runs as one space, cut at 60 characters:
    /// display only (reports), never part of the identity.
    pub text: String,
}

pub mod exclude;

/// An unclosed `-- coverage off` or a `-- coverage on` without one.
pub const PRAGMA_UNCLOSED: &str = "W0112";
pub const PRAGMA_UNOPENED: &str = "W0113";

/// The coverage of one run, or of several merged.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Database {
    pub items: BTreeMap<Key, Entry>,
}

const MAGIC: &str = "yavs-coverage 2";

fn fnv1a(text: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut first = true;
    for w in text.split_whitespace() {
        for b in (!first).then_some(b' ').into_iter().chain(w.bytes()) {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
        first = false;
    }
    h
}

/// One counter of the program: its unit (file and the start of the unit's name) and identity.
struct Item {
    unit: (yavs_diag::FileId, u32),
    kind: Kind,
    hash: u64,
    start: u32,
    file: String,
    line: u32,
    text: String,
    /// Inside a `-- coverage off` region: counted but left out of the database.
    skip: bool,
}

/// First line of a statement's text, for display.
fn display(text: &str) -> String {
    let l = text.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
    let s = l.split_whitespace().collect::<Vec<_>>().join(" ");
    match s.char_indices().nth(60) {
        Some((i, _)) => format!("{}...", &s[..i]),
        None => s,
    }
}

/// The 1-based line ranges of a source text that `-- coverage off`/`on` pragmas (alias
/// `-- yavs coverage ...`) leave out; `off-line` is its own line. With `translate_off` the
/// synthesis `translate_off`/`translate_on` pragmas count as `off`/`on`. ponytail: a `--`
/// inside a string literal is read as a comment start.
fn regions(
    path: &str,
    text: &str,
    translate_off: bool,
    warns: &mut Vec<Diagnostic>,
) -> Vec<(u32, u32)> {
    let mut out = Vec::new();
    let mut open: Option<u32> = None;
    for (i, l) in text.lines().enumerate() {
        let n = i as u32 + 1;
        let Some(at) = l.find("--") else { continue };
        let c = l[at + 2..].trim().to_ascii_lowercase();
        let c = c.strip_prefix("yavs ").unwrap_or(&c).trim_start();
        let word = |s: &str| {
            s.split(|ch: char| ch == ':' || ch.is_whitespace())
                .next()
                .unwrap_or("")
                .to_string()
        };
        let (off, on) = match c.strip_prefix("coverage") {
            Some(r) if r.starts_with(char::is_whitespace) => match word(r.trim_start()).as_str() {
                "off-line" => {
                    out.push((n, n));
                    continue;
                }
                "off" => (true, false),
                "on" => (false, true),
                _ => continue,
            },
            _ if translate_off => {
                let w: Vec<&str> = c.split_whitespace().collect();
                match w[..] {
                    [_, "translate_off", ..]
                        if matches!(w[0], "synthesis" | "pragma" | "synopsys") =>
                    {
                        (true, false)
                    }
                    [_, "translate_on", ..]
                        if matches!(w[0], "synthesis" | "pragma" | "synopsys") =>
                    {
                        (false, true)
                    }
                    _ => continue,
                }
            }
            _ => continue,
        };
        if off && open.is_none() {
            open = Some(n);
        } else if on {
            match open.take() {
                Some(o) => out.push((o, n)),
                None => warns.push(Diagnostic::warning(
                    PRAGMA_UNOPENED,
                    format!("{path}:{n}: coverage `on` without a matching `off`"),
                )),
            }
        }
    }
    if let Some(o) = open {
        out.push((o, u32::MAX));
        warns.push(Diagnostic::warning(
            PRAGMA_UNCLOSED,
            format!("{path}:{o}: coverage `off` is never closed; it excludes the rest of the file"),
        ));
    }
    out
}

/// The database of a run: `counts` is `Runtime::coverage` of the design `e` built after
/// `Elaborated::lower_with` with coverage on. Items inside `-- coverage off` regions are left
/// out; the warnings are about unbalanced pragmas.
pub fn collect(
    db: &Db,
    e: &Elaborated,
    counts: &[u64],
    translate_off: bool,
) -> (Database, Vec<Diagnostic>) {
    let mut out = Database::default();
    let mut warns = Vec::new();
    let Some(ir) = &e.ir else {
        return (out, warns);
    };
    let prog = &ir.program;
    let n = prog.cov.len();
    if n == 0 {
        return (out, warns);
    }
    let mut regs: HashMap<yavs_diag::FileId, Vec<(u32, u32)>> = HashMap::new();
    // Units do not nest: a statement belongs to the unit whose name is the last one before it
    // in its file.
    let mut units: HashMap<yavs_diag::FileId, Vec<(u32, yavs_sem::UnitId)>> = HashMap::new();
    for (id, u) in db.units.iter() {
        units.entry(u.file).or_default().push((u.span.start, id));
    }
    for v in units.values_mut() {
        v.sort();
    }
    let mut items = Vec::with_capacity(n);
    let mut owner = HashMap::new();
    for (_, c) in prog.cov.iter() {
        let span = prog.stmts.get(c.stmt).map(|s| s.span).unwrap_or_default();
        let unit = units
            .get(&span.file)
            .and_then(|v| v[..v.partition_point(|&(s, _)| s <= span.start)].last())
            .map(|&(s, id)| {
                owner.insert((span.file, s), id);
                (span.file, s)
            })
            .unwrap_or((span.file, 0));
        let f = db.sources.get(span.file);
        let text = f
            .text
            .get(f.text_offset(span.start)..f.text_offset(span.end))
            .unwrap_or("");
        let line = f.line_col(span.start).0 as u32;
        let rg = regs
            .entry(span.file)
            .or_insert_with(|| regions(&f.path, &f.text, translate_off, &mut warns));
        items.push(Item {
            text: display(text),
            skip: rg.iter().any(|&(a, b)| a <= line && line <= b),
            unit,
            kind: match c.kind {
                yavs_ir::CovKind::Statement => Kind::Statement,
                yavs_ir::CovKind::Branch(a) => Kind::Branch(a),
            },
            hash: fnv1a(text),
            start: span.start,
            file: f.path.clone(),
            line,
        });
    }
    // Statements with the same text in one unit, numbered in source order (the same
    // statement lowered twice, in two package instances say, is one item).
    let mut starts: BTreeMap<(_, Kind, u64), Vec<u32>> = BTreeMap::new();
    for it in &items {
        starts
            .entry((it.unit, it.kind, it.hash))
            .or_default()
            .push(it.start);
    }
    for v in starts.values_mut() {
        v.sort();
        v.dedup();
    }
    let occ: Vec<u32> = items
        .iter()
        .map(|it| {
            let v = &starts[&(it.unit, it.kind, it.hash)];
            v.partition_point(|&s| s < it.start) as u32
        })
        .collect();
    let design_entity = |unit: (yavs_diag::FileId, u32)| {
        owner.get(&unit).is_some_and(|&id| {
            matches!(db.units[id].kind, UnitKind::Entity | UnitKind::Architecture)
        })
    };
    let unit_path = |unit: (yavs_diag::FileId, u32)| match owner.get(&unit) {
        Some(&id) => {
            let u = &db.units[id];
            format!("{}.{}", db.libs[u.lib].name.text, u.name.text).to_ascii_lowercase()
        }
        None => String::from("?"),
    };
    let loc = |id: yavs_sem::UnitId| (db.units[id].file, db.units[id].span.start);
    // An instance: its path and the units whose items it counts.
    let instance = |s: ScopeId| match e.hier.scope(s).kind {
        ScopeKind::Instance { entity, arch } => {
            let p = e.hier.path_name(s, Case::Lrm);
            let p = p.strip_suffix(':').filter(|p| !p.is_empty()).unwrap_or(&p);
            Some((p.to_string(), [loc(entity), loc(arch)]))
        }
        _ => None,
    };
    let key = |path: String, i: usize| Key {
        path,
        kind: items[i].kind,
        hash: items[i].hash,
        occ: occ[i],
    };
    let mut add = |k: Key, i: usize, c: u64| {
        if items[i].skip {
            return;
        }
        let e = out.items.entry(k).or_insert_with(|| Entry {
            file: items[i].file.clone(),
            line: items[i].line,
            count: 0,
            text: items[i].text.clone(),
        });
        e.count = e.count.saturating_add(c);
    };
    // Every item, with no count yet: per instance for design entities, else per unit.
    for s in 0..e.hier.scopes.len() {
        if let Some((path, units)) = instance(ScopeId(s as u32)) {
            for (i, it) in items.iter().enumerate() {
                if units.contains(&it.unit) {
                    add(key(path.clone(), i), i, 0);
                }
            }
        }
    }
    for (i, it) in items.iter().enumerate() {
        if !design_entity(it.unit) {
            add(key(unit_path(it.unit), i), i, 0);
        }
    }
    // Row 0 has no instance; row r + 1 is region r's instance data.
    for (row, cs) in counts.chunks(n).enumerate() {
        let inst = row
            .checked_sub(1)
            .and_then(|r| instance(e.instance_of(ScopeId(r as u32))));
        for (i, &c) in cs.iter().enumerate().filter(|(_, c)| **c > 0) {
            let unit = items[i].unit;
            let path = match &inst {
                Some((p, units)) if units.contains(&unit) => p.clone(),
                // A package subprogram (or one passed as a generic subprogram): counted under
                // the unit declaring it.
                _ => unit_path(unit),
            };
            add(key(path, i), i, c);
        }
    }
    (out, warns)
}

impl Database {
    /// The text form: a header line, then one tab-separated line per item, sorted by key.
    pub fn write(&self) -> String {
        let mut s = format!("{MAGIC}\n");
        for (k, e) in &self.items {
            let kind = match k.kind {
                Kind::Statement => "s".to_string(),
                Kind::Branch(a) => format!("b{a}"),
            };
            let _ = writeln!(
                s,
                "{kind}\t{}\t{:016x}\t{}\t{}\t{}\t{}\t{}",
                k.path, k.hash, k.occ, e.count, e.line, e.file, e.text
            );
        }
        s
    }

    /// Read the text form of [`Database::write`].
    pub fn parse(text: &str) -> Result<Database, String> {
        let mut lines = text.lines();
        if lines.next() != Some(MAGIC) {
            return Err(format!("not a yavs coverage database (no `{MAGIC}` line)"));
        }
        let mut db = Database::default();
        for (n, l) in lines.enumerate() {
            let bad = || format!("line {}: malformed item `{l}`", n + 2);
            let f: Vec<&str> = l.splitn(8, '\t').collect();
            let [kind, path, hash, occ, count, line, file, text] = f[..] else {
                return Err(bad());
            };
            let kind = match kind {
                "s" => Kind::Statement,
                _ => Kind::Branch(
                    kind.strip_prefix('b')
                        .and_then(|a| a.parse().ok())
                        .ok_or_else(bad)?,
                ),
            };
            let key = Key {
                path: path.to_string(),
                kind,
                hash: u64::from_str_radix(hash, 16).map_err(|_| bad())?,
                occ: occ.parse().map_err(|_| bad())?,
            };
            let entry = Entry {
                file: file.to_string(),
                line: line.parse().map_err(|_| bad())?,
                count: count.parse().map_err(|_| bad())?,
                text: text.to_string(),
            };
            if db.items.insert(key, entry).is_some() {
                return Err(format!("line {}: item listed twice", n + 2));
            }
        }
        Ok(db)
    }

    /// Add the counts of `other`; its items not in `self` are added as they are.
    pub fn merge(&mut self, other: Database) {
        for (k, e) in other.items {
            match self.items.get_mut(&k) {
                Some(x) => x.count = x.count.saturating_add(e.count),
                None => {
                    self.items.insert(k, e);
                }
            }
        }
    }

    /// A plain text summary: totals, one line per path, then every item never reached.
    pub fn summary(&self) -> String {
        let pct = |h: usize, t: usize| {
            if t == 0 {
                "-".to_string()
            } else {
                format!("{:.1}%", 100.0 * h as f64 / t as f64)
            }
        };
        let col = |h: usize, t: usize| format!("{h}/{t} {}", pct(h, t));
        // Per path: statements (hit, total), branches (hit, total).
        let mut per: BTreeMap<&str, [usize; 4]> = BTreeMap::new();
        for (k, e) in &self.items {
            let c = per.entry(&k.path).or_default();
            let o = if k.kind == Kind::Statement { 0 } else { 2 };
            c[o + 1] += 1;
            c[o] += usize::from(e.count > 0);
        }
        let mut t = [0; 4];
        for c in per.values() {
            for (a, b) in t.iter_mut().zip(c) {
                *a += b;
            }
        }
        let mut s = format!(
            "statements {}\nbranches   {}\n\n",
            col(t[0], t[1]),
            col(t[2], t[3])
        );
        let w = per.keys().map(|p| p.len()).max().unwrap_or(0).max(4);
        let _ = writeln!(s, "{:w$}  {:<20}  branches", "path", "statements");
        for (p, c) in &per {
            let _ = writeln!(s, "{p:w$}  {:<20}  {}", col(c[0], c[1]), col(c[2], c[3]));
        }
        let mut missed: Vec<_> = self.items.iter().filter(|(_, e)| e.count == 0).collect();
        missed.sort_by(|a, b| (&a.1.file, a.1.line, a.0).cmp(&(&b.1.file, b.1.line, b.0)));
        if !missed.is_empty() {
            s.push_str("\nnot covered:\n");
        }
        for (k, e) in missed {
            let what = match k.kind {
                Kind::Statement => "statement".to_string(),
                Kind::Branch(a) => format!("branch {a}"),
            };
            let _ = writeln!(s, "  {}:{}  {what}  {}", e.file, e.line, k.path);
        }
        s
    }

    /// A Cobertura XML report: one class per source file, counts summed over every path. A
    /// line's hits are those of its most executed statement; a line with branches lists how
    /// many of their arms ran. The timestamp is 0, so equal databases give equal reports.
    pub fn cobertura(&self) -> String {
        #[derive(Default)]
        struct Line {
            hits: u64,
            arms: BTreeMap<(u64, u32, u32), u64>,
        }
        // Counts per source statement (or arm), summed over paths.
        let mut stmts: BTreeMap<(&str, u32, u64, u32), u64> = BTreeMap::new();
        let mut files: BTreeMap<&str, BTreeMap<u32, Line>> = BTreeMap::new();
        for (k, e) in &self.items {
            let line = files.entry(&e.file).or_default().entry(e.line).or_default();
            match k.kind {
                Kind::Statement => {
                    let c = stmts.entry((&e.file, e.line, k.hash, k.occ)).or_default();
                    *c = c.saturating_add(e.count);
                }
                Kind::Branch(a) => {
                    let c = line.arms.entry((k.hash, k.occ, a)).or_default();
                    *c = c.saturating_add(e.count);
                }
            }
        }
        for ((f, l, _, _), c) in stmts {
            if let Some(line) = files.get_mut(f).and_then(|m| m.get_mut(&l)) {
                line.hits = line.hits.max(c);
            }
        }
        let rate = |h: usize, t: usize| if t == 0 { 0.0 } else { h as f64 / t as f64 };
        let count = |lines: &BTreeMap<u32, Line>| {
            let mut c = [0; 4];
            for l in lines.values() {
                c[0] += usize::from(l.hits > 0);
                c[1] += 1;
                c[2] += l.arms.values().filter(|&&n| n > 0).count();
                c[3] += l.arms.len();
            }
            c
        };
        let mut classes = String::new();
        let mut t = [0; 4];
        for (f, lines) in &files {
            let c = count(lines);
            for (a, b) in t.iter_mut().zip(c) {
                *a += b;
            }
            let name = std::path::Path::new(f)
                .file_stem()
                .map_or(f.to_string(), |s| s.to_string_lossy().into_owned());
            let _ = writeln!(
                classes,
                "        <class name=\"{}\" filename=\"{}\" line-rate=\"{}\" branch-rate=\"{}\" \
                 complexity=\"0\">\n          <methods/>\n          <lines>",
                xml(&name),
                xml(f),
                rate(c[0], c[1]),
                rate(c[2], c[3])
            );
            for (n, l) in lines {
                let _ = write!(
                    classes,
                    "            <line number=\"{n}\" hits=\"{}\"",
                    l.hits
                );
                if l.arms.is_empty() {
                    classes.push_str(" branch=\"false\"/>\n");
                } else {
                    let (h, t) = (l.arms.values().filter(|&&n| n > 0).count(), l.arms.len());
                    let _ = writeln!(
                        classes,
                        " branch=\"true\" condition-coverage=\"{}% ({h}/{t})\"/>",
                        100 * h / t
                    );
                }
            }
            classes.push_str("          </lines>\n        </class>\n");
        }
        format!(
            "<?xml version=\"1.0\" ?>\n\
             <!DOCTYPE coverage SYSTEM \"http://cobertura.sourceforge.net/xml/coverage-04.dtd\">\n\
             <coverage line-rate=\"{lr}\" branch-rate=\"{br}\" lines-covered=\"{}\" \
             lines-valid=\"{}\" branches-covered=\"{}\" branches-valid=\"{}\" complexity=\"0\" \
             version=\"yavs {}\" timestamp=\"0\">\n  <sources>\n    <source>.</source>\n  \
             </sources>\n  <packages>\n    <package name=\"design\" line-rate=\"{lr}\" \
             branch-rate=\"{br}\" complexity=\"0\">\n      <classes>\n{classes}      </classes>\n    \
             </package>\n  </packages>\n</coverage>\n",
            t[0],
            t[1],
            t[2],
            t[3],
            env!("CARGO_PKG_VERSION"),
            lr = rate(t[0], t[1]),
            br = rate(t[2], t[3]),
        )
    }
}

fn xml(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\'' => o.push_str("&apos;"),
            c => o.push(c),
        }
    }
    o
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(path: &str, kind: Kind, hash: u64, line: u32, count: u64) -> (Key, Entry) {
        (
            Key {
                path: path.into(),
                kind,
                hash,
                occ: 0,
            },
            Entry {
                file: "a&b.vhd".into(),
                line,
                count,
                text: "x := 1;".into(),
            },
        )
    }

    #[test]
    fn hash_ignores_layout() {
        assert_eq!(fnv1a("a  :=\n  b;"), fnv1a("a := b;"));
        assert_ne!(fnv1a("a := b;"), fnv1a("a := c;"));
    }

    #[test]
    fn write_parse_merge() {
        let a = Database {
            items: [
                item(":top", Kind::Statement, 1, 3, 2),
                item(":top", Kind::Branch(0), 2, 3, 0),
            ]
            .into(),
        };
        let b = Database {
            items: [
                item(":top", Kind::Branch(0), 2, 3, 5),
                item("work.p", Kind::Statement, 3, 9, 0),
            ]
            .into(),
        };
        assert_eq!(Database::parse(&a.write()).unwrap(), a);
        let mut m = a.clone();
        m.merge(b);
        let counts: Vec<u64> = m.items.values().map(|e| e.count).collect();
        assert_eq!(counts, [2, 5, 0]);
        assert!(Database::parse("yavs-coverage 2\ns\tx\n").is_err());
        assert!(Database::parse("other").is_err());
        let x = m.cobertura();
        assert!(x.contains("filename=\"a&amp;b.vhd\""), "{x}");
        assert!(x.contains("lines-covered=\"1\" lines-valid=\"2\""), "{x}");
        assert!(x.contains("condition-coverage=\"100% (1/1)\""), "{x}");
        let s = m.summary();
        assert!(
            s.starts_with("statements 1/2 50.0%\nbranches   1/1 100.0%\n"),
            "{s}"
        );
        assert!(s.contains("a&b.vhd:9  statement  work.p"), "{s}");
    }
}
