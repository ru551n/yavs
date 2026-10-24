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
    /// A `bit`/`std_ulogic` scalar of a signal or port went 0 -> 1 (`true`) or 1 -> 0.
    Toggle(bool),
    /// Row `n` of a condition: input `n / 2` at value `n % 2` (focused expression coverage).
    Condition(u32),
    /// The same for a logic expression on the right side of an assignment.
    Expression(u32),
}

impl Kind {
    /// A condition or expression row.
    pub fn is_fec(self) -> bool {
        matches!(self, Kind::Condition(_) | Kind::Expression(_))
    }

    /// The row of a condition or expression: its input and value.
    pub fn row(self) -> Option<(u32, bool)> {
        match self {
            Kind::Condition(r) | Kind::Expression(r) => Some((r / 2, r % 2 == 1)),
            _ => None,
        }
    }
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
    /// Toggles: the scalar's name below `path` (`data(3)`, `r.valid`); hash and occ are 0.
    /// Conditions and expressions: the text of the row's input. Empty for statements and
    /// branches.
    pub sig: String,
}

/// Where an item is and how often it ran.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub file: String,
    pub line: u32,
    pub count: u64,
    /// First line of the statement, whitespace runs as one space, cut at 60 characters:
    /// display only (reports), never part of the identity. A condition or expression row: the
    /// result the row's input decides (`0`, `1`, or `?` when other inputs take part), a space
    /// and the text of the whole condition.
    pub text: String,
    /// The tests that hit it: indices into [`Database::tests`], sorted.
    pub tests: Vec<u32>,
}

pub mod exclude;
pub mod html;
pub mod toggle;

/// An unclosed `-- coverage off` or a `-- coverage on` without one.
pub const PRAGMA_UNCLOSED: &str = "W0112";
pub const PRAGMA_UNOPENED: &str = "W0113";

/// The coverage of one run, or of several merged.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Database {
    pub items: BTreeMap<Key, Entry>,
    /// The names of the tests the runs were (`yavs run --cov-test`), in the order seen.
    pub tests: Vec<String>,
}

const MAGIC: &str = "yavs-coverage 5";
/// Older versions [`Database::parse`] still reads: 2 has no toggles, 2 and 3 no tests, 4 no
/// conditions.
const OLD: &[&str] = &["yavs-coverage 2", "yavs-coverage 3"];
const V4: &str = "yavs-coverage 4";

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
    /// A condition row: the text of its input.
    input: String,
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
        // A condition row belongs to the condition's text, not to its statement.
        let span = match &c.row {
            Some(r) => r.whole,
            None => prog.stmts.get(c.stmt).map(|s| s.span).unwrap_or_default(),
        };
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
            .text()
            .get(f.text_offset(span.start)..f.text_offset(span.end))
            .unwrap_or("");
        let line = f.line_col(span.start).0 as u32;
        let rg = regs
            .entry(span.file)
            .or_insert_with(|| regions(&f.path, f.text(), translate_off, &mut warns));
        let at = |s: yavs_diag::Span| {
            f.text()
                .get(f.text_offset(s.start)..f.text_offset(s.end))
                .unwrap_or("")
        };
        items.push(Item {
            input: c
                .row
                .as_ref()
                .map(|r| display(at(r.input)))
                .unwrap_or_default(),
            text: match &c.row {
                Some(r) => format!("{} {}", r.result, display(text)),
                None => display(text),
            },
            skip: rg.iter().any(|&(a, b)| a <= line && line <= b),
            unit,
            kind: match c.kind {
                yavs_ir::CovKind::Statement => Kind::Statement,
                yavs_ir::CovKind::Branch(a) => Kind::Branch(a),
                yavs_ir::CovKind::Condition(r) => Kind::Condition(r),
                yavs_ir::CovKind::Expression(r) => Kind::Expression(r),
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
        sig: items[i].input.clone(),
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
            tests: Vec::new(),
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

/// `v` in decimal.
fn push_num(s: &mut String, mut v: u64) {
    let mut b = [0u8; 20];
    let mut i = b.len();
    loop {
        i -= 1;
        b[i] = b'0' + (v % 10) as u8;
        v /= 10;
        if v == 0 {
            break;
        }
    }
    s.extend(b[i..].iter().map(|&c| c as char));
}

/// The ids of `tests` (renumbered by `id`) in sorted order, comma-separated; `-` for none.
fn push_tests(s: &mut String, tests: &[u32], id: &[usize]) {
    match tests {
        [] => s.push('-'),
        [one] => push_num(s, id[*one as usize] as u64),
        _ => {
            let mut t: Vec<usize> = tests.iter().map(|&i| id[i as usize]).collect();
            t.sort_unstable();
            for (n, v) in t.iter().enumerate() {
                if n > 0 {
                    s.push(',');
                }
                push_num(s, *v as u64);
            }
        }
    }
}

impl Database {
    /// The text form: a header line, the tests section (`test`, id, name; ids number the
    /// names in sorted order), then one tab-separated line per item, sorted by key.
    /// Statements and branches: kind, path, hash, occurrence, count, tests, line, file, text;
    /// conditions (`c<row>`) and expressions (`e<row>`) the same with the row's input before
    /// the text;
    /// toggles (`t1` rise, `t0` fall): kind, path, scalar name, count, tests, line, file.
    /// `tests` is the comma-separated ids of the tests that hit the item, `-` for none.
    pub fn write(&self) -> String {
        let mut s = format!("{MAGIC}\n");
        let mut order: Vec<usize> = (0..self.tests.len()).collect();
        order.sort_by(|&a, &b| self.tests[a].cmp(&self.tests[b]));
        let mut id = vec![0; self.tests.len()];
        for (new, &old) in order.iter().enumerate() {
            id[old] = new;
            let _ = writeln!(s, "test\t{new}\t{}", self.tests[old]);
        }
        // About 150 bytes a line: one allocation for the database of a large design.
        s.reserve(self.items.len() * 150);
        for (k, e) in &self.items {
            if let Kind::Toggle(r) = k.kind {
                // The common line, written without the formatting machinery.
                s.push('t');
                s.push(if r { '1' } else { '0' });
                s.push('\t');
                s.push_str(&k.path);
                s.push('\t');
                s.push_str(&k.sig);
                s.push('\t');
                push_num(&mut s, e.count);
                s.push('\t');
                push_tests(&mut s, &e.tests, &id);
                s.push('\t');
                push_num(&mut s, u64::from(e.line));
                s.push('\t');
                s.push_str(&e.file);
                s.push('\n');
                continue;
            }
            let mut t = String::new();
            push_tests(&mut t, &e.tests, &id);
            let kind = match k.kind {
                Kind::Statement => "s".to_string(),
                Kind::Branch(a) => format!("b{a}"),
                Kind::Condition(r) | Kind::Expression(r) => {
                    let c = if matches!(k.kind, Kind::Condition(_)) {
                        'c'
                    } else {
                        'e'
                    };
                    let _ = writeln!(
                        s,
                        "{c}{r}\t{}\t{:016x}\t{}\t{}\t{t}\t{}\t{}\t{}\t{}",
                        k.path, k.hash, k.occ, e.count, e.line, e.file, k.sig, e.text
                    );
                    continue;
                }
                Kind::Toggle(_) => continue,
            };
            let _ = writeln!(
                s,
                "{kind}\t{}\t{:016x}\t{}\t{}\t{t}\t{}\t{}\t{}",
                k.path, k.hash, k.occ, e.count, e.line, e.file, e.text
            );
        }
        s
    }

    /// Read the text form of [`Database::write`] (or of an older version: version 2 has no
    /// toggles, versions 2 and 3 no tests).
    pub fn parse(text: &str) -> Result<Database, String> {
        let mut lines = text.lines();
        let head = lines.next();
        let old = head.is_some_and(|h| OLD.contains(&h));
        if head != Some(MAGIC) && head != Some(V4) && !old {
            return Err(format!("not a yavs coverage database (no `{MAGIC}` line)"));
        }
        let mut db = Database::default();
        for (n, l) in lines.enumerate() {
            let bad = || format!("line {}: malformed item `{l}`", n + 2);
            if let Some(t) = l.strip_prefix("test\t") {
                let (id, name) = t.split_once('\t').ok_or_else(bad)?;
                if id.parse::<usize>().ok() != Some(db.tests.len()) {
                    return Err(bad());
                }
                db.tests.push(name.to_string());
                continue;
            }
            let mut f: Vec<&str> = l.split('\t').collect();
            // Versions before 4 have no tests field (after the count).
            let at = if l.starts_with('t') { 4 } else { 5 };
            if old && f.len() >= at {
                f.insert(at, "-");
            }
            let tests = match f.get(at) {
                Some(&"-") => Vec::new(),
                Some(t) => t
                    .split(',')
                    .map(|i| {
                        i.parse()
                            .ok()
                            .filter(|&i: &u32| (i as usize) < db.tests.len())
                    })
                    .collect::<Option<Vec<u32>>>()
                    .ok_or_else(bad)?,
                None => return Err(bad()),
            };
            let (key, entry) = if l.starts_with('t') {
                let [kind, path, sig, count, _, line, file] = f[..] else {
                    return Err(bad());
                };
                let rise = match kind {
                    "t1" => true,
                    "t0" => false,
                    _ => return Err(bad()),
                };
                let key = Key {
                    path: path.to_string(),
                    kind: Kind::Toggle(rise),
                    hash: 0,
                    occ: 0,
                    sig: sig.to_string(),
                };
                let entry = Entry {
                    file: file.to_string(),
                    line: line.parse().map_err(|_| bad())?,
                    count: count.parse().map_err(|_| bad())?,
                    text: String::new(),
                    tests,
                };
                (key, entry)
            } else {
                let fec = l.starts_with('c') || l.starts_with('e');
                let (sig, f) = match (fec, &f[..]) {
                    (true, [a, b, c, d, e, g, h, i, sig, text]) => {
                        (*sig, vec![*a, *b, *c, *d, *e, *g, *h, *i, *text])
                    }
                    (false, _) => ("", f),
                    _ => return Err(bad()),
                };
                let [kind, path, hash, occ, count, _, line, file, text] = f[..] else {
                    return Err(bad());
                };
                let row = |p: char| {
                    kind.strip_prefix(p)
                        .and_then(|a| a.parse().ok())
                        .ok_or_else(bad)
                };
                let kind = match kind {
                    "s" => Kind::Statement,
                    _ if fec && kind.starts_with('c') => Kind::Condition(row('c')?),
                    _ if fec => Kind::Expression(row('e')?),
                    _ => Kind::Branch(row('b')?),
                };
                let key = Key {
                    path: path.to_string(),
                    kind,
                    hash: u64::from_str_radix(hash, 16).map_err(|_| bad())?,
                    occ: occ.parse().map_err(|_| bad())?,
                    sig: sig.to_string(),
                };
                let entry = Entry {
                    file: file.to_string(),
                    line: line.parse().map_err(|_| bad())?,
                    count: count.parse().map_err(|_| bad())?,
                    text: text.to_string(),
                    tests,
                };
                (key, entry)
            };
            if db.items.insert(key, entry).is_some() {
                return Err(format!("line {}: item listed twice", n + 2));
            }
        }
        Ok(db)
    }

    /// Add the counts of `other` and the tests that hit each item (tests are matched by
    /// name); its items not in `self` are added as they are.
    pub fn merge(&mut self, other: Database) {
        let id: Vec<u32> = other
            .tests
            .iter()
            .map(|t| match self.tests.iter().position(|x| x == t) {
                Some(i) => i as u32,
                None => {
                    self.tests.push(t.clone());
                    self.tests.len() as u32 - 1
                }
            })
            .collect();
        for (k, mut e) in other.items {
            for t in &mut e.tests {
                *t = id[*t as usize];
            }
            match self.items.get_mut(&k) {
                Some(x) => {
                    x.count = x.count.saturating_add(e.count);
                    x.tests.extend(e.tests);
                    x.tests.sort_unstable();
                    x.tests.dedup();
                }
                None => {
                    e.tests.sort_unstable();
                    self.items.insert(k, e);
                }
            }
        }
    }

    /// Make this the database of one run of test `name`: every item that ran was hit by it.
    pub fn attribute(&mut self, name: &str) {
        self.tests = vec![name.to_string()];
        for e in self.items.values_mut() {
            e.tests = if e.count > 0 { vec![0] } else { Vec::new() };
        }
    }

    /// `yavs cov rank`: a greedy set cover (the test hitting the most items not yet hit,
    /// until no test adds one; ties go to the first name), then per test its items and the
    /// items no other test hits. Items no test hit are not counted.
    pub fn rank(&self) -> String {
        let n = self.tests.len();
        let mut per: Vec<Vec<usize>> = vec![Vec::new(); n];
        let mut hit = 0;
        for (i, e) in self.items.values().enumerate() {
            for &t in &e.tests {
                per[t as usize].push(i);
            }
            hit += usize::from(!e.tests.is_empty());
        }
        let mut names: Vec<usize> = (0..n).collect();
        names.sort_by(|&a, &b| self.tests[a].cmp(&self.tests[b]));
        let mut done = vec![false; self.items.len()];
        let mut so_far = 0;
        let mut s = format!(
            "{n} tests hit {hit} of {} items\n\nminimal set (greedy):\n",
            self.items.len()
        );
        let w = self.tests.iter().map(String::len).max().unwrap_or(0).max(4);
        let mut picked = vec![false; n];
        loop {
            let best = names
                .iter()
                .map(|&t| (per[t].iter().filter(|&&i| !done[i]).count(), t))
                .filter(|&(c, t)| c > 0 && !picked[t])
                .fold(None, |b: Option<(usize, usize)>, x| match b {
                    Some(b) if b.0 >= x.0 => Some(b),
                    _ => Some(x),
                });
            let Some((c, t)) = best else { break };
            picked[t] = true;
            so_far += c;
            for &i in &per[t] {
                done[i] = true;
            }
            let _ = writeln!(s, "  {:w$}  +{c}  {so_far}/{hit}", self.tests[t]);
        }
        let rest: Vec<&str> = names
            .iter()
            .filter(|&&t| !picked[t])
            .map(|&t| self.tests[t].as_str())
            .collect();
        if !rest.is_empty() {
            let _ = writeln!(s, "\nnot needed: {}", rest.join(", "));
        }
        let _ = writeln!(s, "\n{:w$}  {:>8}  {:>8}", "test", "items", "unique");
        let items: Vec<&Entry> = self.items.values().collect();
        for &t in &names {
            let unique = per[t]
                .iter()
                .filter(|&&i| items[i].tests.len() == 1)
                .count();
            let _ = writeln!(s, "{:w$}  {:>8}  {unique:>8}", self.tests[t], per[t].len());
        }
        s
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
        // Per path: statements, branches, toggles, conditions, expressions (hit, total each).
        let mut per: BTreeMap<&str, [usize; 10]> = BTreeMap::new();
        for (k, e) in &self.items {
            let c = per.entry(&k.path).or_default();
            let o = match k.kind {
                Kind::Statement => 0,
                Kind::Branch(_) => 2,
                Kind::Toggle(_) => 4,
                Kind::Condition(_) => 6,
                Kind::Expression(_) => 8,
            };
            c[o + 1] += 1;
            c[o] += usize::from(e.count > 0);
        }
        let mut t = [0; 10];
        for c in per.values() {
            for (a, b) in t.iter_mut().zip(c) {
                *a += b;
            }
        }
        // Conditions and expressions show only when the database has them.
        let (fc, fe) = (t[7] > 0, t[9] > 0);
        let mut s = format!(
            "statements {}\nbranches   {}\ntoggles    {}\n",
            col(t[0], t[1]),
            col(t[2], t[3]),
            col(t[4], t[5])
        );
        if fc {
            let _ = writeln!(s, "conditions {}", col(t[6], t[7]));
        }
        if fe {
            let _ = writeln!(s, "expressions {}", col(t[8], t[9]));
        }
        s.push('\n');
        let w = per.keys().map(|p| p.len()).max().unwrap_or(0).max(4);
        // The toggles column is padded only when a column follows.
        let pad = if fc || fe { 20 } else { 0 };
        let _ = write!(
            s,
            "{:w$}  {:<20}  {:<20}  {:<pad$}",
            "path", "statements", "branches", "toggles"
        );
        if fc {
            let _ = write!(s, "  {:<20}", "conditions");
        }
        if fe {
            let _ = write!(s, "  expressions");
        }
        s = s.trim_end().to_string();
        s.push('\n');
        for (p, c) in &per {
            let _ = write!(
                s,
                "{p:w$}  {:<20}  {:<20}  {:<pad$}",
                col(c[0], c[1]),
                col(c[2], c[3]),
                col(c[4], c[5])
            );
            if fc {
                let _ = write!(s, "  {:<20}", col(c[6], c[7]));
            }
            if fe {
                let _ = write!(s, "  {}", col(c[8], c[9]));
            }
            s = s.trim_end().to_string();
            s.push('\n');
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
                Kind::Toggle(r) => format!("toggle {} {}", k.sig, arrow(r)),
                Kind::Condition(r) | Kind::Expression(r) => {
                    let w = if matches!(k.kind, Kind::Condition(_)) {
                        "condition"
                    } else {
                        "expression"
                    };
                    format!("{w} input {} is {}", k.sig, r % 2)
                }
            };
            let _ = writeln!(s, "  {}:{}  {what}  {}", e.file, e.line, k.path);
        }
        s
    }

    /// A Cobertura XML report: one class per source file, counts summed over every path. A
    /// line's hits are those of its most executed statement; a line with branches lists how
    /// many of their arms ran. Toggles are not in it (Cobertura has no notion of them). The
    /// timestamp is 0, so equal databases give equal reports.
    pub fn cobertura(&self) -> String {
        #[derive(Default)]
        struct Line {
            hits: u64,
            arms: BTreeMap<(u64, u32, u32), u64>,
        }
        // Counts per source statement (or arm), summed over paths.
        let mut stmts: BTreeMap<(&str, u32, u64, u32), u64> = BTreeMap::new();
        let mut files: BTreeMap<&str, BTreeMap<u32, Line>> = BTreeMap::new();
        for (k, e) in self
            .items
            .iter()
            .filter(|(k, _)| !matches!(k.kind, Kind::Toggle(_)))
        {
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
                // A condition row is a condition of the line, apart from the branch arms
                // (arm numbers from 2^31; an expression row from 2^30 more).
                Kind::Condition(r) | Kind::Expression(r) => {
                    let a = (1 << 31)
                        | r
                        | if matches!(k.kind, Kind::Expression(_)) {
                            1 << 30
                        } else {
                            0
                        };
                    let c = line.arms.entry((k.hash, k.occ, a)).or_default();
                    *c = c.saturating_add(e.count);
                }
                Kind::Toggle(_) => {}
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

/// What a condition or expression row is about: `a is 1 (result 1) in a and b` (input, value,
/// the result that input decides alone, the whole condition).
pub fn fec_text(k: &Key, e: &Entry) -> String {
    let (res, whole) = e.text.split_once(' ').unwrap_or(("?", &e.text));
    let v = k.kind.row().map_or(0, |r| u8::from(r.1));
    let res = if res == "?" {
        String::new()
    } else {
        format!(" (result {res})")
    };
    format!("{} is {v}{res} in {whole}", k.sig)
}

/// `0->1` or `1->0`.
pub fn arrow(rise: bool) -> &'static str {
    if rise { "0->1" } else { "1->0" }
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
                sig: String::new(),
            },
            Entry {
                file: "a&b.vhd".into(),
                line,
                count,
                text: "x := 1;".into(),
                tests: Vec::new(),
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
            ..Default::default()
        };
        let b = Database {
            items: [
                item(":top", Kind::Branch(0), 2, 3, 5),
                item("work.p", Kind::Statement, 3, 9, 0),
            ]
            .into(),
            ..Default::default()
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

    #[test]
    fn toggles_round_trip() {
        let mut d = Database::default();
        let (mut k, e) = item(":top:u1", Kind::Toggle(true), 0, 4, 3);
        k.sig = "r.d(7)".into();
        d.items.insert(
            k,
            Entry {
                text: String::new(),
                ..e
            },
        );
        let t = d.write();
        assert!(
            t.ends_with("t1\t:top:u1\tr.d(7)\t3\t-\t4\ta&b.vhd\n"),
            "{t}"
        );
        assert_eq!(Database::parse(&t).unwrap(), d);
        // Older versions still read: 2 (no toggles), 3 (no tests).
        assert!(Database::parse("yavs-coverage 2\n").is_ok());
        let v3 = Database::parse("yavs-coverage 3\nt1\t:top:u1\tr.d(7)\t3\t4\ta&b.vhd\n");
        assert_eq!(v3.unwrap(), d);
        assert!(d.cobertura().contains("lines-valid=\"0\""));
        assert!(d.summary().contains("toggles    1/1 100.0%"));
    }

    #[test]
    fn tests_merge_and_rank() {
        // Items 1..=4; test a hits 1 2 3, b hits 3 4, c hits 4: {a, b} is the set, c is not needed.
        let run = |name: &str, hits: &[u64]| {
            let mut d = Database::default();
            for h in 1..=5 {
                let (k, e) = item(
                    ":t",
                    Kind::Statement,
                    h,
                    h as u32,
                    u64::from(hits.contains(&h)),
                );
                d.items.insert(k, e);
            }
            d.attribute(name);
            d
        };
        let mut m = run("c", &[4]);
        m.merge(run("a", &[1, 2, 3]));
        m.merge(run("b", &[3, 4]));
        let back = Database::parse(&m.write()).unwrap();
        assert_eq!(back.tests, ["a", "b", "c"]);
        let names = |db: &Database, h: u64| -> Vec<String> {
            let e = db.items.values().nth(h as usize - 1).unwrap();
            e.tests
                .iter()
                .map(|&t| db.tests[t as usize].clone())
                .collect()
        };
        assert_eq!(names(&back, 3), ["a", "b"]);
        assert_eq!(names(&back, 4), ["b", "c"]);
        assert!(names(&back, 5).is_empty());
        // The same file whatever the merge order.
        let mut m2 = run("a", &[1, 2, 3]);
        m2.merge(run("b", &[3, 4]));
        m2.merge(run("c", &[4]));
        assert_eq!(m2.write(), m.write());
        let r = m.rank();
        assert!(r.starts_with("3 tests hit 4 of 5 items\n"), "{r}");
        assert!(r.contains("  a     +3  3/4\n  b     +1  4/4\n"), "{r}");
        assert!(r.contains("not needed: c"), "{r}");
        assert!(r.contains("a            3         2\n"), "{r}");
        assert!(r.contains("c            1         0\n"), "{r}");
    }
}
