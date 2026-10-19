//! Debugger state as plain structs, updated from DAP messages and tested without a window
//! (docs/design/gui.md section 9): breakpoints, the stack of every process, the variables tree,
//! the time and delta of a stop, and the small text helpers of the source view.

use std::collections::{BTreeMap, HashMap, HashSet};

use serde_json::{Value, json};

/// Children requested per `variables` page.
pub const PAGE: u64 = 100;

#[derive(Clone, Debug, PartialEq)]
pub struct Bp {
    pub line: u32,
    pub condition: String,
    pub hit_condition: String,
    pub enabled: bool,
    /// The answer of the last `setBreakpoints`; `None` until it came.
    pub verified: Option<bool>,
    pub message: String,
}

/// The line breakpoints, per source file. The GUI owns the list and resends a file's whole set.
#[derive(Default)]
pub struct Breakpoints {
    pub files: BTreeMap<String, Vec<Bp>>,
}

impl Breakpoints {
    pub fn get(&self, path: &str, line: u32) -> Option<&Bp> {
        self.files.get(path)?.iter().find(|b| b.line == line)
    }

    /// Add a breakpoint on `line`, or remove the one there.
    pub fn toggle(&mut self, path: &str, line: u32) {
        let v = self.files.entry(path.to_string()).or_default();
        match v.iter().position(|b| b.line == line) {
            Some(i) => {
                v.remove(i);
            }
            None => v.push(Bp {
                line,
                condition: String::new(),
                hit_condition: String::new(),
                enabled: true,
                verified: None,
                message: String::new(),
            }),
        }
        if v.is_empty() {
            self.files.remove(path);
        }
    }

    /// Create or change the breakpoint on `line`; empty texts mean none.
    pub fn set(&mut self, path: &str, line: u32, condition: &str, hit: &str) {
        if self.get(path, line).is_none() {
            self.toggle(path, line);
        }
        if let Some(b) = self
            .files
            .get_mut(path)
            .and_then(|v| v.iter_mut().find(|b| b.line == line))
        {
            b.condition = condition.trim().to_string();
            b.hit_condition = hit.trim().to_string();
        }
    }

    pub fn set_enabled(&mut self, path: &str, line: u32, on: bool) {
        if let Some(b) = self
            .files
            .get_mut(path)
            .and_then(|v| v.iter_mut().find(|b| b.line == line))
        {
            b.enabled = on;
        }
    }

    /// The arguments of `setBreakpoints` for `path`: the enabled breakpoints, in file order.
    pub fn request(&self, path: &str) -> Value {
        let bps: Vec<Value> = self.enabled(path).map(bp_json).collect();
        json!({"source": {"path": path}, "breakpoints": bps})
    }

    fn enabled<'a>(&'a self, path: &str) -> impl Iterator<Item = &'a Bp> {
        self.files
            .get(path)
            .into_iter()
            .flatten()
            .filter(|b| b.enabled)
    }

    /// Apply the response to `request(path)`: the server moves a breakpoint to the next line
    /// with a statement, so the entry follows it (two that meet become one).
    pub fn apply(&mut self, path: &str, answer: &Value) {
        let Some(v) = self.files.get_mut(path) else {
            return;
        };
        let got = answer["breakpoints"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let mut got = got.iter();
        for b in v.iter_mut().filter(|b| b.enabled) {
            let Some(g) = got.next() else { break };
            b.verified = Some(g["verified"] == true);
            b.message = g["message"].as_str().unwrap_or("").to_string();
            if g["verified"] == true
                && let Some(l) = g["line"].as_u64()
            {
                b.line = l as u32;
            }
        }
        let mut seen = HashSet::new();
        v.retain(|b| seen.insert(b.line));
        v.sort_by_key(|b| b.line);
    }
}

fn bp_json(b: &Bp) -> Value {
    let mut j = json!({"line": b.line});
    if !b.condition.is_empty() {
        j["condition"] = json!(b.condition);
    }
    if !b.hit_condition.is_empty() {
        j["hitCondition"] = json!(b.hit_condition);
    }
    j
}

#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    pub id: i64,
    pub name: String,
    pub path: Option<String>,
    pub line: u32,
}

#[derive(Clone, Debug)]
pub struct Thread {
    pub id: i64,
    pub name: String,
    /// `None` until its `stackTrace` came.
    pub frames: Option<Vec<Frame>>,
}

/// The processes of the design with their stacks, and the selected frame.
#[derive(Default)]
pub struct Stack {
    pub threads: Vec<Thread>,
    /// (thread id, index into its frames)
    pub selected: Option<(i64, usize)>,
}

impl Stack {
    pub fn set_threads(&mut self, body: &Value) {
        self.threads = body["threads"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|t| Thread {
                id: t["id"].as_i64().unwrap_or(0),
                name: t["name"].as_str().unwrap_or("?").to_string(),
                frames: None,
            })
            .collect();
    }

    pub fn set_frames(&mut self, thread: i64, body: &Value) {
        let frames = body["stackFrames"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|f| Frame {
                id: f["id"].as_i64().unwrap_or(0),
                name: f["name"].as_str().unwrap_or("?").to_string(),
                path: f["source"]["path"].as_str().map(str::to_string),
                line: f["line"].as_u64().unwrap_or(0) as u32,
            })
            .collect();
        if let Some(t) = self.threads.iter_mut().find(|t| t.id == thread) {
            t.frames = Some(frames);
        }
    }

    pub fn name(&self, thread: i64) -> Option<&str> {
        Some(&self.threads.iter().find(|t| t.id == thread)?.name)
    }

    pub fn frame(&self) -> Option<&Frame> {
        let (t, i) = self.selected?;
        self.threads
            .iter()
            .find(|x| x.id == t)?
            .frames
            .as_ref()?
            .get(i)
    }

    /// The line of the selected frame, if it is in `path`.
    pub fn current_line(&self, path: &str) -> Option<u32> {
        let f = self.frame()?;
        (f.path.as_deref() == Some(path)).then_some(f.line)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Var {
    pub name: String,
    pub value: String,
    pub ty: String,
    pub reference: i64,
    /// Elements of an array, for paging.
    pub indexed: u64,
}

/// The variables of the selected frame: its scopes, and the children fetched so far by
/// `variablesReference`.
#[derive(Default)]
pub struct VarTree {
    pub scopes: Vec<(String, i64)>,
    pub children: HashMap<i64, Vec<Var>>,
    /// References with a `variables` request on the way.
    pub requested: HashSet<i64>,
    /// References fetched by page (arrays).
    pub paged: HashSet<i64>,
}

impl VarTree {
    pub fn clear(&mut self) {
        *self = VarTree::default();
    }

    /// The scopes worth fetching at once: all but the expensive Hierarchy.
    pub fn set_scopes(&mut self, body: &Value) -> Vec<i64> {
        self.scopes = body["scopes"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|s| s["expensive"] != true)
            .map(|s| {
                (
                    s["name"].as_str().unwrap_or("?").to_string(),
                    s["variablesReference"].as_i64().unwrap_or(0),
                )
            })
            .collect();
        self.scopes.iter().map(|s| s.1).collect()
    }

    /// Store a `variables` answer; a page after the first (`start` > 0) is appended.
    pub fn set_children(&mut self, reference: i64, start: u64, body: &Value) {
        self.requested.remove(&reference);
        let vars = body["variables"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|v| Var {
                name: v["name"].as_str().unwrap_or("?").to_string(),
                value: v["value"].as_str().unwrap_or("").to_string(),
                ty: v["type"].as_str().unwrap_or("").to_string(),
                reference: v["variablesReference"].as_i64().unwrap_or(0),
                indexed: v["indexedVariables"].as_u64().unwrap_or(0),
            });
        let slot = self.children.entry(reference).or_default();
        if start == 0 {
            slot.clear();
        }
        slot.extend(vars);
    }

    /// The arguments of the request that fetches the next page of `var`, if it needs one.
    pub fn more(&self, var: &Var) -> Option<(i64, u64)> {
        let have = self.children.get(&var.reference)?.len() as u64;
        (var.indexed > have).then_some((var.reference, have))
    }
}

/// `"breakpoint at 1250 ns, delta 3"` -> `("1250 ns", 3)`; the form of the description of
/// `stopped` until the server sends the structured time (docs/design/gui.md P1).
pub fn parse_time(description: &str) -> Option<(String, u64)> {
    let (_, rest) = description.split_once(" at ")?;
    let (time, delta) = rest.rsplit_once(", delta ")?;
    Some((time.trim().to_string(), delta.trim().parse().ok()?))
}

/// The identifier (with `.` selections) around byte column `col` of `line`.
pub fn word_at(line: &str, col: usize) -> Option<&str> {
    let ok = |c: char| c.is_alphanumeric() || c == '_';
    let chars: Vec<(usize, char)> = line.char_indices().collect();
    let mut i = col.min(chars.len());
    if i == chars.len() || !ok(chars[i].1) {
        i = i.checked_sub(1).filter(|&j| ok(chars[j].1))?;
    }
    let mut a = i;
    while a > 0 && ok(chars[a - 1].1) {
        a -= 1;
    }
    let mut b = i;
    while b + 1 < chars.len() && ok(chars[b + 1].1) {
        b += 1;
    }
    let end = chars.get(b + 1).map_or(line.len(), |c| c.0);
    let w = &line[chars[a].0..end];
    (!w.starts_with(|c: char| c.is_ascii_digit())).then_some(w)
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Kind {
    Plain,
    Keyword,
    Comment,
    Text,
}

const KEYWORDS: &[&str] = &[
    "architecture",
    "array",
    "assert",
    "begin",
    "block",
    "case",
    "component",
    "constant",
    "else",
    "elsif",
    "end",
    "entity",
    "exit",
    "for",
    "function",
    "generate",
    "generic",
    "if",
    "in",
    "inout",
    "is",
    "library",
    "loop",
    "map",
    "next",
    "null",
    "of",
    "others",
    "out",
    "package",
    "port",
    "procedure",
    "process",
    "range",
    "record",
    "report",
    "return",
    "severity",
    "signal",
    "subtype",
    "then",
    "to",
    "downto",
    "type",
    "until",
    "use",
    "variable",
    "wait",
    "when",
    "while",
    "with",
    "and",
    "or",
    "not",
    "xor",
    "nand",
    "nor",
    "xnor",
    "after",
    "body",
    "all",
    "open",
    "select",
    "units",
];

/// Split one line into coloured spans: keywords, `--` comments, string and character literals.
/// A small lexer of its own, no parser dependency (docs/design/gui.md section 6).
pub fn highlight(line: &str) -> Vec<(Kind, &str)> {
    let mut out = Vec::new();
    let b = line.as_bytes();
    let (mut i, mut start) = (0, 0);
    fn flush<'a>(out: &mut Vec<(Kind, &'a str)>, line: &'a str, from: usize, to: usize) {
        if to > from {
            out.push((Kind::Plain, &line[from..to]));
        }
    }
    while i < b.len() {
        let c = b[i];
        if c == b'-' && b.get(i + 1) == Some(&b'-') {
            flush(&mut out, line, start, i);
            out.push((Kind::Comment, &line[i..]));
            return out;
        } else if c == b'"' {
            flush(&mut out, line, start, i);
            let end = line[i + 1..].find('"').map_or(line.len(), |e| i + 2 + e);
            out.push((Kind::Text, &line[i..end]));
            (i, start) = (end, end);
        } else if c == b'\'' && b.get(i + 2) == Some(&b'\'') && i + 3 <= b.len() {
            flush(&mut out, line, start, i);
            out.push((Kind::Text, &line[i..i + 3]));
            (i, start) = (i + 3, i + 3);
        } else if c.is_ascii_alphabetic() {
            let mut j = i;
            while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b'_') {
                j += 1;
            }
            if KEYWORDS.contains(&line[i..j].to_ascii_lowercase().as_str()) {
                flush(&mut out, line, start, i);
                out.push((Kind::Keyword, &line[i..j]));
                start = j;
            }
            i = j;
        } else {
            i += 1;
        }
    }
    flush(&mut out, line, start, line.len());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn breakpoints_toggle_edit_and_slide() {
        let mut b = Breakpoints::default();
        b.toggle("/a.vhd", 9);
        b.toggle("/a.vhd", 12);
        b.set("/a.vhd", 12, " i = 2 ", "3");
        assert_eq!(
            b.request("/a.vhd"),
            json!({"source": {"path": "/a.vhd"}, "breakpoints": [
                {"line": 9}, {"line": 12, "condition": "i = 2", "hitCondition": "3"}]})
        );
        // Line 9 slides to 11 (verified); line 12 is refused and stays with the message.
        b.apply(
            "/a.vhd",
            &json!({"breakpoints": [
                {"verified": true, "line": 11}, {"verified": false, "line": 12, "message": "no"}]}),
        );
        assert_eq!(b.get("/a.vhd", 11).unwrap().verified, Some(true));
        assert_eq!(b.get("/a.vhd", 12).unwrap().message, "no");
        b.set_enabled("/a.vhd", 11, false);
        assert_eq!(
            b.request("/a.vhd")["breakpoints"].as_array().unwrap().len(),
            1
        );
        b.toggle("/a.vhd", 11);
        b.toggle("/a.vhd", 12);
        assert!(b.files.is_empty());
        assert_eq!(b.request("/a.vhd")["breakpoints"], json!([]));
    }

    #[test]
    fn stack_and_current_line() {
        let mut s = Stack::default();
        s.set_threads(&json!({"threads": [{"id": 1, "name": "p"}, {"id": 2, "name": "q"}]}));
        s.set_frames(
            1,
            &json!({"stackFrames": [{"id": 65536, "name": "p", "line": 14,
                "source": {"path": "/a.vhd"}}]}),
        );
        s.selected = Some((1, 0));
        assert_eq!(s.current_line("/a.vhd"), Some(14));
        assert_eq!(s.current_line("/b.vhd"), None);
        assert_eq!(s.name(2), Some("q"));
        assert!(s.threads[1].frames.is_none());
    }

    #[test]
    fn variables_page_and_scopes() {
        let mut t = VarTree::default();
        let ids = t.set_scopes(&json!({"scopes": [
            {"name": "Locals", "variablesReference": 5},
            {"name": "Hierarchy", "variablesReference": 4, "expensive": true}]}));
        assert_eq!(ids, [5]);
        let v = |n: &str| json!({"name": n, "value": "0", "variablesReference": 0});
        t.set_children(9, 0, &json!({"variables": [v("0"), v("1")]}));
        let arr = Var {
            name: "a".into(),
            value: String::new(),
            ty: String::new(),
            reference: 9,
            indexed: 3,
        };
        assert_eq!(t.more(&arr), Some((9, 2)));
        t.set_children(9, 2, &json!({"variables": [v("2")]}));
        assert_eq!(t.children[&9].len(), 3);
        assert_eq!(t.more(&arr), None);
    }

    #[test]
    fn time_and_words() {
        assert_eq!(
            parse_time("breakpoint at 1250 ns, delta 3"),
            Some(("1250 ns".into(), 3))
        );
        assert_eq!(parse_time("paused"), None);
        assert_eq!(word_at("  i := twice(k);", 3), Some("i"));
        assert_eq!(word_at("  i := twice(k);", 10), Some("twice"));
        assert_eq!(word_at("  i := twice(k);", 5), None);
        assert_eq!(word_at("x <= 12;", 5), None);
    }

    #[test]
    fn highlighting() {
        use Kind::*;
        assert_eq!(
            highlight("if x = '1' then -- go"),
            [
                (Keyword, "if"),
                (Plain, " x = "),
                (Text, "'1'"),
                (Plain, " "),
                (Keyword, "then"),
                (Plain, " "),
                (Comment, "-- go")
            ]
        );
        assert_eq!(highlight("report \"a--b\";")[2], (Text, "\"a--b\""));
        assert_eq!(highlight("endx"), [(Plain, "endx")]);
    }
}
