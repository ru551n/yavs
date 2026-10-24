//! The HTML report (`yavs cov report --html DIR`): static files with inline CSS and one small
//! inline script, no external resources, so a page opens from disk. `index.html` has the totals,
//! one line per instance or package and one per source file; `src-N.html` (N numbers the files
//! in name order) shows a file's source with the hit count of each line. A line with items
//! (statements, branch arms, toggles) is clickable and keyboard-focusable: its popup lists the
//! items per instance with counts and tests and opens only on a click (Enter or Space on a
//! focused line); hover and focus open nothing. A line hit in some instances and
//! missed in others has its own colour and a `ran/total` instance marker. A missed item shows
//! its `--uncovered` row and YAML entry, each with a copy button; an excluded one is greyed with
//! the reason of its row. `src-N-missed.html` lists only the lines with a missed or excluded
//! item. No timestamp: equal inputs give byte-identical files. The popup is a list of groups
//! (title, table), so other item kinds add groups without a new layout.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use crate::exclude::{head, miss_row, miss_yaml};
use crate::{Database, Entry, Key, Kind, arrow};

const CSS: &str = "\
:root{--bg:#fff;--fg:#1f2328;--dim:#6e7781;--line:#d0d7de;--miss:#ffdce0;--part:#fff5b1;\
--inst:#e3d8ff;--ok:#dafbe1;--ex:#eaeef2;--bar:#2da44e;--pop:#f6f8fa}\
@media (prefers-color-scheme:dark){:root{--bg:#0d1117;--fg:#e6edf3;--dim:#8b949e;\
--line:#30363d;--miss:#5a1d24;--part:#4d3d00;--inst:#3b2a66;--ok:#12361f;--ex:#21262d;\
--bar:#3fb950;--pop:#161b22}}\
body{background:var(--bg);color:var(--fg);font:14px/1.45 system-ui,sans-serif;margin:1rem 1.5rem}\
a{color:inherit}h1{font-size:1.4rem}h2{font-size:1.1rem;margin-top:1.5rem}\
table{border-collapse:collapse}th,td{padding:.15rem .7rem;text-align:left;\
border-bottom:1px solid var(--line)}td.n{text-align:right}\
code,pre,.src td{font:13px/1.4 ui-monospace,monospace}\
.bar{display:inline-block;width:5rem;height:.6rem;background:var(--miss);vertical-align:middle;\
margin-right:.4rem}.bar i{display:block;height:100%;background:var(--bar)}\
.src{width:100%}.src td{border:0;padding:0 .5rem;white-space:pre}\
.src td.ln,.src td.c,.src td.m{text-align:right;color:var(--dim);width:1%}\
tr.hit td{background:var(--ok)}tr.miss td{background:var(--miss)}tr.part td{background:var(--part)}\
.pop th{text-align:left;font-weight:normal;color:var(--dim);padding-right:.8rem}\
tr.inst td{background:var(--inst)}tr.ex td{background:var(--ex)}\
tr[tabindex]{cursor:pointer}tr[tabindex]:hover td{filter:brightness(.93)}tr[tabindex]:focus{outline:2px solid var(--dim);outline-offset:-2px}\
.src td.t{position:relative}\
.pop{display:none;position:absolute;left:2rem;top:100%;z-index:5;min-width:24rem;max-width:56rem;\
white-space:normal;cursor:auto;background:var(--pop);color:var(--fg);border:1px solid var(--dim);\
padding:.4rem .7rem;box-shadow:0 4px 14px rgba(0,0,0,.3)}\
tr.pin .pop{display:block}\
.pop table{width:100%}.pop td.n{width:3rem;padding-right:1rem}.pop h4{margin:.4rem 0 .1rem;font-size:.85rem}.pop p{margin:.2rem 0}\
.pop table td{white-space:normal;padding:.05rem .5rem .05rem 0;border:0;background:none}\
.pop tr.miss td:first-child{border-left:.3rem solid #cf222e;padding-left:.3rem}\
.pop tr.x td{padding-bottom:.4rem}.pop div{display:flex;align-items:flex-start}.pop code,.pop pre{user-select:all}.pop pre{margin:.1rem 0}\
.pop .ex{color:var(--dim)}\
.cp{display:none;margin-left:.5rem;font:12px system-ui,sans-serif}.js .cp{display:inline}\
.legend span{padding:0 .5rem;margin-right:.3rem;color:var(--fg)}\
.legend .hit{background:var(--ok)}.legend .miss{background:var(--miss)}\
.legend .inst{background:var(--inst)}.legend .part{background:var(--part)}\
.legend .ex{background:var(--ex)}.dim{color:var(--dim)}";

const JS: &str = "\
(function(){var d=document,pin=null;d.documentElement.className+=' js';\
function set(t){if(pin)pin.classList.remove('pin');pin=t==pin?null:t;\
if(pin)pin.classList.add('pin')}\
function fb(s){var a=d.createElement('textarea');a.value=s;d.body.appendChild(a);a.select();\
try{d.execCommand('copy')}catch(x){}d.body.removeChild(a)}\
function copy(s,b){var o=b.textContent;function ok(){b.textContent='copied';\
setTimeout(function(){b.textContent=o},1200)}\
if(navigator.clipboard&&navigator.clipboard.writeText)\
navigator.clipboard.writeText(s).then(ok,function(){fb(s);ok()});else{fb(s);ok()}}\
d.addEventListener('click',function(e){var t=e.target,b=t.closest('.cp');\
if(b){copy(b.previousElementSibling.textContent,b);return}\
if(!t.closest('.pop'))set(t.closest('tr[tabindex]'))});\
d.addEventListener('keydown',function(e){\
if(e.key=='Escape'&&pin)set(pin);\
else if((e.key=='Enter'||e.key==' ')&&e.target.matches('tr[tabindex]')){e.preventDefault();set(e.target)}})})();";

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
         <title>{}</title>\n<style>{CSS}</style>\n</head>\n<body>\n{body}<script>{JS}</script>\n</body>\n</html>\n",
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
struct Tally([usize; 10]);

impl Tally {
    fn add(&mut self, k: &Key, e: &Entry) {
        let o = match k.kind {
            Kind::Statement => 0,
            Kind::Branch(_) => 2,
            Kind::Toggle(_) => 4,
            Kind::Condition(_) => 6,
            Kind::Expression(_) => 8,
        };
        self.0[o + 1] += 1;
        self.0[o] += usize::from(e.count > 0);
    }
    /// The cells of the kinds shown: conditions and expressions only when `fec` says so.
    fn cells(&self, fec: [bool; 2]) -> String {
        (0..3 + usize::from(fec[0]) + usize::from(fec[1]))
            .map(|i| {
                // Skip a column the report has no rows for.
                let i = if i == 3 && !fec[0] { 4 } else { i };
                cell(self.0[2 * i], self.0[2 * i + 1])
            })
            .collect()
    }
}

fn table_head(first: &str, fec: [bool; 2]) -> String {
    format!(
        "<tr><th>{first}</th><th>statements</th><th>branches</th><th>toggles</th>{}{}</tr>\n",
        if fec[0] { "<th>conditions</th>" } else { "" },
        if fec[1] { "<th>expressions</th>" } else { "" }
    )
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
    let fec = [total.0[7] > 0, total.0[9] > 0];
    let mut b = String::from("<h1>Coverage report</h1>\n");
    let _ = write!(
        b,
        "<table>\n{}<tr><th>total</th>{}</tr>\n</table>\n",
        table_head("", fec),
        total.cells(fec)
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
        table_head("path", fec)
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
            t.cells(fec)
        );
    }
    let _ = writeln!(
        b,
        "</table>\n<h2>Source files</h2>\n<table>\n{}",
        table_head("file", fec)
    );
    for (i, f) in names.iter().enumerate() {
        let t = per_file.get(f).copied().unwrap_or_default();
        let _ = writeln!(
            b,
            "<tr><td><a href=\"src-{i}.html\">{}</a> <a class=\"dim\" href=\"src-{i}-missed.html\">missed only</a></td>{}</tr>",
            esc(f),
            t.cells(fec)
        );
    }
    b.push_str("</table>\n");
    out.push(("index.html".to_string(), page("Coverage report", &b)));
    for (i, f) in names.iter().enumerate() {
        for missed in [false, true] {
            let n = if missed { "-missed" } else { "" };
            out.push((
                format!("src-{i}{n}.html"),
                file_page(db, ex, f, &sources[*f], (i, missed)),
            ));
        }
    }
    out
}

/// An item of a line with the reason of the exclusion row that took it out, if any.
type LineItem<'a> = (&'a Key, &'a Entry, Option<&'a str>);

const LEGEND: &str = "<p class=\"legend dim\">click a line (or focus it and press Enter) for details: <span class=\"hit\">hit</span><span class=\"miss\">missed</span>\
<span class=\"inst\">hit in some instances only (hit/total)</span>\
<span class=\"part\">branch arm or toggle missed</span><span class=\"ex\">excluded</span></p>\n";

/// What an item counts, as a popup group title.
fn what(k: &Key) -> String {
    match k.kind {
        Kind::Statement => "statement".to_string(),
        Kind::Branch(a) => format!("branch arm {a}"),
        Kind::Toggle(r) => format!("toggle {} {}", k.sig, arrow(r)),
        Kind::Condition(_) | Kind::Expression(_) => String::new(),
    }
}

/// The popup of a line: one group (title, table of instances) per item kind, each instance
/// with its count and tests; a missed item has its exclusion row with a copy button, an
/// excluded one the row that excluded it and the reason.
fn popup(db: &Database, n: usize, its: &[LineItem], summary: &str) -> String {
    let mut b = format!(
        "<div class=\"pop\" id=\"p{n}\" role=\"group\" aria-label=\"coverage of line {n}\">"
    );
    if !summary.is_empty() {
        let _ = write!(b, "<p>{summary}</p>");
    }
    let mut title = String::new();
    for (k, e, why) in its {
        let fec = k.kind.is_fec();
        // A condition or expression: one table of its rows (row, input, result, hit).
        let t = if fec {
            let w = if matches!(k.kind, Kind::Condition(_)) {
                "condition"
            } else {
                "expression"
            };
            format!(
                "{w} in {}: {}",
                k.path,
                e.text.split_once(' ').map_or(&e.text[..], |x| x.1)
            )
        } else {
            what(k)
        };
        if t != title {
            if !title.is_empty() {
                b.push_str("</table>");
            }
            let _ = write!(b, "<h4>{}</h4><table>", esc(&t));
            if fec {
                b.push_str(
                    "<tr><th>row</th><th>input</th><th>result</th><th>hit</th><th></th></tr>",
                );
            }
            title = t;
        }
        let class = match (why, e.count) {
            (Some(_), _) => "ex",
            (None, 0) => "miss",
            _ => "",
        };
        let tests: Vec<String> = e
            .tests
            .iter()
            .map(|&t| esc(&db.tests[t as usize]))
            .collect();
        let tests = if tests.is_empty() {
            String::new()
        } else {
            format!("tests: {}", tests.join(", "))
        };
        let reason = why.map_or(String::new(), |w| {
            format!("excluded{}{}", if w.is_empty() { "" } else { ": " }, esc(w))
        });
        if let Some((_, v)) = k.kind.row() {
            let row = match k.kind {
                Kind::Condition(r) | Kind::Expression(r) => r,
                _ => 0,
            };
            let _ = write!(
                b,
                "<tr class=\"{class}\"><td>{row}</td><td>{} = {}</td><td>{}</td><td class=\"n\">{}</td><td class=\"dim\">{tests}{reason}</td></tr>",
                esc(&k.sig),
                u8::from(v),
                esc(e.text.split_once(' ').map_or("?", |x| x.0)),
                e.count
            );
        } else {
            let _ = write!(
                b,
                "<tr class=\"{class}\"><td>{}</td><td class=\"n\">{}</td><td class=\"dim\">{tests}{reason}</td></tr>",
                esc(&k.path),
                e.count
            );
        }
        let span = if fec { 5 } else { 3 };
        if let Some(r) = miss_row(db, k, e).filter(|_| why.is_none()) {
            let y = miss_yaml(db, k, e).unwrap_or_default();
            let _ = write!(
                b,
                "<tr class=\"x\"><td colspan=\"{span}\"><div><code>{}</code><button type=\"button\" class=\"cp\">copy row</button></div>\
<div><pre>{}</pre><button type=\"button\" class=\"cp\">copy YAML</button></div></td></tr>",
                esc(r.trim_end()),
                esc(y.trim_end())
            );
        } else if why.is_some() {
            let _ = write!(
                b,
                "<tr class=\"x ex\"><td colspan=\"{span}\"><code>{}</code></td></tr>",
                esc(&head(k, e))
            );
        }
    }
    if !title.is_empty() {
        b.push_str("</table>");
    }
    b.push_str("</div>");
    b
}

/// `view` is the file's number and whether the page lists only the lines with a missed or
/// excluded item (`src-N-missed.html`), which keeps big designs usable.
fn file_page(
    db: &Database,
    ex: &[(Key, Entry, String)],
    file: &str,
    text: &str,
    view: (usize, bool),
) -> String {
    let (num, missed) = view;
    // Items by line: (key, entry, exclusion reason).
    let mut by: BTreeMap<u32, Vec<LineItem>> = BTreeMap::new();
    for (k, e) in db.items.iter().filter(|(_, e)| e.file == file) {
        by.entry(e.line).or_default().push((k, e, None));
    }
    for (k, e, why) in ex.iter().filter(|x| x.1.file == file) {
        by.entry(e.line).or_default().push((k, e, Some(why)));
    }
    let other = if missed {
        format!("<a href=\"src-{num}.html\">all lines</a>")
    } else {
        format!("<a href=\"src-{num}-missed.html\">missed only</a>")
    };
    let mut b = format!(
        "<p><a href=\"index.html\">index</a> {other}</p>\n<h1>{}</h1>\n{}<table class=\"src\">\n",
        esc(file),
        LEGEND
    );
    let lines: Vec<&str> = text.lines().collect();
    let last = by.keys().next_back().copied().unwrap_or(0) as usize;
    for n in 1..=lines.len().max(last) {
        let mut its = by.remove(&(n as u32)).unwrap_or_default();
        // Groups: statement, branch arms, then toggles by signal and direction.
        let order = |i: &LineItem| (matches!(i.0.kind, Kind::Toggle(_)), i.0.sig.clone());
        // Rows of one condition together, in row order.
        let group = |i: &LineItem| {
            let k = i.0;
            if k.kind.is_fec() {
                (true, k.hash, k.occ, k.path.clone())
            } else {
                (false, 0, 0, String::new())
            }
        };
        its.sort_by(|a, b| {
            (order(a), group(a), a.0.kind, &a.0.path, a.0).cmp(&(
                order(b),
                group(b),
                b.0.kind,
                &b.0.path,
                b.0,
            ))
        });
        let live = || its.iter().filter(|i| i.2.is_none());
        if missed && !its.iter().any(|i| i.2.is_some() || i.1.count == 0) {
            continue;
        }
        // Statements per instance: the line ran there if all its statements did.
        let mut inst: BTreeMap<&str, bool> = BTreeMap::new();
        for i in live().filter(|i| i.0.kind == Kind::Statement) {
            *inst.entry(&i.0.path).or_insert(true) &= i.1.count > 0;
        }
        let ran = inst.values().filter(|&&r| r).count();
        let miss_other = live().any(|i| i.1.count == 0 && i.0.kind != Kind::Statement);
        let class = if !inst.is_empty() && ran == 0 {
            "miss"
        } else if ran < inst.len() {
            "inst"
        } else if miss_other {
            "part"
        } else if live().next().is_some() {
            "hit"
        } else if !its.is_empty() {
            "ex"
        } else {
            ""
        };
        let max = live()
            .filter(|i| i.0.kind == Kind::Statement)
            .map(|i| i.1.count)
            .max();
        let count = max.map_or(String::new(), |c| c.to_string());
        let partial = ran > 0 && ran < inst.len();
        let (marker, summary) = if partial {
            (
                format!("{ran}/{}", inst.len()),
                format!("executed in {ran} of {} instances", inst.len()),
            )
        } else {
            (String::new(), String::new())
        };
        let (attrs, pop) = if its.is_empty() {
            (String::new(), String::new())
        } else {
            (
                format!(" tabindex=\"0\" aria-describedby=\"p{n}\""),
                popup(db, n, &its, &summary),
            )
        };
        let _ = writeln!(
            b,
            "<tr id=\"L{n}\" class=\"{class}\"{attrs}><td class=\"ln\">{n}</td><td class=\"c\">{count}</td>\
<td class=\"m\" title=\"{summary}\">{marker}</td><td class=\"t\">{}{pop}</td></tr>",
            esc(lines.get(n - 1).copied().unwrap_or(""))
        );
    }
    b.push_str("</table>\n");
    page(file, &b)
}
