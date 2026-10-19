//! The HTML report (`yavs cov report --html DIR`): static files with inline CSS, no scripts and
//! no external resources, so a page opens from disk. `index.html` has the totals, one line per
//! instance or package and one per source file; `src-N.html` (N numbers the files in name
//! order) shows a file's source with the hit count of each line and, under a line, its items
//! (statements, branch arms, toggles) with their paths, counts and tests. A missed item shows
//! its `--uncovered` row, ready to copy into an exclusion file; an excluded one is greyed with
//! the reason of its row. No timestamp: equal inputs give byte-identical files.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use crate::exclude::{head, miss_row};
use crate::{Database, Entry, Key, Kind, arrow};

const CSS: &str = "\
:root{--bg:#fff;--fg:#1f2328;--dim:#6e7781;--line:#d0d7de;--miss:#ffdce0;--part:#fff5b1;\
--ok:#dafbe1;--ex:#eaeef2;--bar:#2da44e}\
@media (prefers-color-scheme:dark){:root{--bg:#0d1117;--fg:#e6edf3;--dim:#8b949e;\
--line:#30363d;--miss:#5a1d24;--part:#4d3d00;--ok:#12361f;--ex:#21262d;--bar:#3fb950}}\
body{background:var(--bg);color:var(--fg);font:14px/1.45 system-ui,sans-serif;margin:1rem 1.5rem}\
a{color:inherit}h1{font-size:1.4rem}h2{font-size:1.1rem;margin-top:1.5rem}\
table{border-collapse:collapse}th,td{padding:.15rem .7rem;text-align:left;\
border-bottom:1px solid var(--line)}td.n{text-align:right}\
code,pre,.src td{font:13px/1.4 ui-monospace,monospace}\
.bar{display:inline-block;width:5rem;height:.6rem;background:var(--miss);vertical-align:middle;\
margin-right:.4rem}.bar i{display:block;height:100%;background:var(--bar)}\
.src{width:100%}.src td{border:0;padding:0 .5rem;white-space:pre}\
.src td.ln,.src td.c{text-align:right;color:var(--dim);width:1%}\
tr.hit td{background:var(--ok)}tr.miss td{background:var(--miss)}tr.part td{background:var(--part)}\
.it td{white-space:normal;color:var(--dim);padding-left:6rem}\
.it.miss td{color:var(--fg)}.it.ex td{background:var(--ex);color:var(--dim)}\
.it code{user-select:all}.dim{color:var(--dim)}";

fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\'' => o.push_str("&#39;"),
            c => o.push(c),
        }
    }
    o
}

fn page(title: &str, body: &str) -> String {
    format!(
        "<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n\
         <meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\n\
         <title>{}</title>\n<style>{CSS}</style>\n</head>\n<body>\n{body}</body>\n</html>\n",
        esc(title)
    )
}

/// `hit/total`, a bar and the percentage, as the cells of a table row.
fn cell(h: usize, t: usize) -> String {
    if t == 0 {
        return "<td class=\"n dim\">-</td>".to_string();
    }
    let p = 100.0 * h as f64 / t as f64;
    format!("<td><span class=\"bar\"><i style=\"width:{p:.1}%\"></i></span>{h}/{t} {p:.1}%</td>")
}

/// Hit and total of statements, branches and toggles.
#[derive(Default, Clone, Copy)]
struct Tally([usize; 6]);

impl Tally {
    fn add(&mut self, k: &Key, e: &Entry) {
        let o = match k.kind {
            Kind::Statement => 0,
            Kind::Branch(_) => 2,
            Kind::Toggle(_) => 4,
        };
        self.0[o + 1] += 1;
        self.0[o] += usize::from(e.count > 0);
    }
    fn cells(&self) -> String {
        (0..3)
            .map(|i| cell(self.0[2 * i], self.0[2 * i + 1]))
            .collect()
    }
}

const HEAD: &str = "<tr><th>{}</th><th>statements</th><th>branches</th><th>toggles</th></tr>\n";

fn table_head(first: &str) -> String {
    HEAD.replace("{}", first)
}

/// The source files the report needs: every file of the database and of the excluded items.
pub fn files<'a>(db: &'a Database, ex: &'a [(Key, Entry, String)]) -> BTreeSet<&'a str> {
    let kept = db.items.values().map(|e| e.file.as_str());
    kept.chain(ex.iter().map(|x| x.1.file.as_str())).collect()
}

/// The pages of the report as (file name, content). `db` is the database without the
/// excluded items `ex`; `sources` has the text of every file [`files`] lists.
pub fn pages(
    db: &Database,
    ex: &[(Key, Entry, String)],
    sources: &BTreeMap<String, String>,
) -> Vec<(String, String)> {
    let names: Vec<&str> = files(db, ex).into_iter().collect();
    let mut out = Vec::new();
    let mut total = Tally::default();
    let mut per_path: BTreeMap<&str, Tally> = BTreeMap::new();
    let mut per_file: BTreeMap<&str, Tally> = BTreeMap::new();
    for (k, e) in &db.items {
        total.add(k, e);
        per_path.entry(&k.path).or_default().add(k, e);
        per_file.entry(&e.file).or_default().add(k, e);
    }
    let mut b = String::from("<h1>Coverage report</h1>\n");
    let _ = write!(
        b,
        "<table>\n{}<tr><th>total</th>{}</tr>\n</table>\n",
        table_head(""),
        total.cells()
    );
    if !ex.is_empty() {
        let _ = writeln!(
            b,
            "<p class=\"dim\">{} excluded items are not in the totals.</p>",
            ex.len()
        );
    }
    if !db.tests.is_empty() {
        let t: Vec<String> = db.tests.iter().map(|t| esc(t)).collect();
        let _ = writeln!(
            b,
            "<p class=\"dim\">{} tests: {}</p>",
            t.len(),
            t.join(", ")
        );
    }
    let _ = writeln!(
        b,
        "<h2>Instances and packages</h2>\n<table>\n{}",
        table_head("path")
    );
    for (p, t) in &per_path {
        // Instance paths indent by depth.
        let depth = if p.starts_with(':') {
            p.matches(':').count() - 1
        } else {
            0
        };
        let _ = writeln!(
            b,
            "<tr><td style=\"padding-left:{}rem\">{}</td>{}</tr>",
            0.7 + depth as f64,
            esc(p),
            t.cells()
        );
    }
    let _ = writeln!(
        b,
        "</table>\n<h2>Source files</h2>\n<table>\n{}",
        table_head("file")
    );
    for (i, f) in names.iter().enumerate() {
        let t = per_file.get(f).copied().unwrap_or_default();
        let _ = writeln!(
            b,
            "<tr><td><a href=\"src-{i}.html\">{}</a></td>{}</tr>",
            esc(f),
            t.cells()
        );
    }
    b.push_str("</table>\n");
    out.push(("index.html".to_string(), page("Coverage report", &b)));
    for (i, f) in names.iter().enumerate() {
        out.push((format!("src-{i}.html"), file_page(db, ex, f, &sources[*f])));
    }
    out
}

/// An item of a line with the reason of the exclusion row that took it out, if any.
type LineItem<'a> = (&'a Key, &'a Entry, Option<&'a str>);

fn file_page(db: &Database, ex: &[(Key, Entry, String)], file: &str, text: &str) -> String {
    // Items by line: (key, entry, exclusion reason).
    let mut by: BTreeMap<u32, Vec<LineItem>> = BTreeMap::new();
    for (k, e) in db.items.iter().filter(|(_, e)| e.file == file) {
        by.entry(e.line).or_default().push((k, e, None));
    }
    for (k, e, why) in ex.iter().filter(|x| x.1.file == file) {
        by.entry(e.line).or_default().push((k, e, Some(why)));
    }
    let mut b = format!(
        "<p><a href=\"index.html\">index</a></p>\n<h1>{}</h1>\n<table class=\"src\">\n",
        esc(file)
    );
    let lines: Vec<&str> = text.lines().collect();
    let last = by.keys().next_back().copied().unwrap_or(0) as usize;
    for n in 1..=lines.len().max(last) {
        let mut its = by.remove(&(n as u32)).unwrap_or_default();
        its.sort_by(|a, b| {
            (&a.0.path, &a.0.sig, a.0.kind, a.0).cmp(&(&b.0.path, &b.0.sig, b.0.kind, b.0))
        });
        let live = || its.iter().filter(|i| i.2.is_none());
        let stmts: Vec<u64> = live()
            .filter(|i| i.0.kind == Kind::Statement)
            .map(|i| i.1.count)
            .collect();
        let miss_stmt = stmts.contains(&0);
        let miss_other = live().any(|i| i.1.count == 0 && i.0.kind != Kind::Statement);
        let class = if miss_stmt {
            "miss"
        } else if miss_other {
            "part"
        } else if live().next().is_some() {
            "hit"
        } else {
            ""
        };
        let count = stmts.iter().max().map_or(String::new(), u64::to_string);
        let _ = writeln!(
            b,
            "<tr id=\"L{n}\" class=\"{class}\"><td class=\"ln\">{n}</td><td class=\"c\">{count}</td><td>{}</td></tr>",
            esc(lines.get(n - 1).copied().unwrap_or(""))
        );
        for (k, e, why) in its {
            let what = match k.kind {
                Kind::Statement => "statement".to_string(),
                Kind::Branch(a) => format!("branch arm {a}"),
                Kind::Toggle(r) => format!("toggle {} {}", k.sig, arrow(r)),
            };
            let mut d = format!("{} in {}: {}", esc(&what), esc(&k.path), e.count);
            if !db.tests.is_empty() && !e.tests.is_empty() {
                let t: Vec<String> = e
                    .tests
                    .iter()
                    .map(|&t| esc(&db.tests[t as usize]))
                    .collect();
                let _ = write!(d, " (hit by tests: {})", t.join(", "));
            }
            let class = match (why, e.count) {
                (Some(_), _) => "ex",
                (None, 0) => "miss",
                _ => "",
            };
            if let Some(w) = why {
                let _ = write!(
                    d,
                    " (excluded{}{})",
                    if w.is_empty() { "" } else { ": " },
                    esc(w)
                );
            }
            if let Some(r) = miss_row(db, k, e).filter(|_| why.is_none()) {
                let _ = write!(d, "<br><code>{}</code>", esc(r.trim_end()));
            } else if why.is_some() {
                let _ = write!(d, "<br><code>{}</code>", esc(&head(k, e)));
            }
            let _ = writeln!(
                b,
                "<tr class=\"it {class}\"><td colspan=\"3\">{d}</td></tr>"
            );
        }
    }
    b.push_str("</table>\n");
    page(file, &b)
}
