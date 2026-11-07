//! The preprocessor (IEEE 1364-2005 clause 19): text macros, conditional compilation, `include,
//! `timescale, `default_nettype, `celldefine, protected envelopes. It works on bytes and keeps a
//! map from every output byte back to the original source, so later stages report original spans.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::{Diag, Options, Span, Timescale};

/// Marker byte left in the output where a protected envelope was removed.
pub(crate) const ENVELOPE: u8 = 1;

const MAX_DEPTH: u32 = 64;
/// A macro bomb stops here (bytes of preprocessed text).
const MAX_OUT: usize = 1 << 26;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Nettype {
    Wire,
    Tri,
    Tri0,
    Tri1,
    Wand,
    Triand,
    Wor,
    Trior,
    Trireg,
    Uwire,
    None,
}

impl Nettype {
    fn parse(s: &[u8]) -> Option<Nettype> {
        Some(match s {
            b"wire" => Nettype::Wire,
            b"tri" => Nettype::Tri,
            b"tri0" => Nettype::Tri0,
            b"tri1" => Nettype::Tri1,
            b"wand" => Nettype::Wand,
            b"triand" => Nettype::Triand,
            b"wor" => Nettype::Wor,
            b"trior" => Nettype::Trior,
            b"trireg" => Nettype::Trireg,
            b"uwire" => Nettype::Uwire,
            b"none" => Nettype::None,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Macro {
    params: Option<Vec<String>>,
    body: Vec<u8>,
}

/// The reserved-word set of `` `begin_keywords `` (clause 19.11).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum KwSet {
    #[default]
    V2005,
    V2001,
    V2001NoConfig,
    V1995,
}

/// What persists from one file to the next of one analysis.
#[derive(Clone, Debug)]
pub(crate) struct State {
    /// Open `` `begin_keywords `` regions, innermost last, with the directive's place.
    pub keywords: Vec<(KwSet, Span)>,
    pub macros: HashMap<String, Macro>,
    pub timescale: Option<Timescale>,
    pub nettype: Nettype,
    pub celldefine: bool,
}

impl State {
    pub(crate) fn new(opts: &Options) -> State {
        let mut macros = HashMap::new();
        for (n, v) in &opts.defines {
            macros.insert(
                n.clone(),
                Macro {
                    params: None,
                    body: v.clone().unwrap_or_default().into_bytes(),
                },
            );
        }
        State {
            macros,
            keywords: Vec::new(),
            timescale: None,
            nettype: Nettype::Wire,
            celldefine: false,
        }
    }

    /// A stable digest of the state, for the cache key of a file analysed in this context.
    pub(crate) fn digest(&self, opts: &Options) -> String {
        let mut h: u128 = 0x6c62272e07bb014262b821756295c58d;
        let mut feed = |bytes: &[u8]| {
            for &b in bytes {
                h ^= b as u128;
                h = h.wrapping_mul(0x0000000001000000000000000000013B);
            }
            h ^= 0xff;
            h = h.wrapping_mul(0x0000000001000000000000000000013B);
        };
        let mut names: Vec<&String> = self.macros.keys().collect();
        names.sort();
        for n in names {
            let m = &self.macros[n];
            feed(n.as_bytes());
            match &m.params {
                Some(p) => feed(p.join(",").as_bytes()),
                None => feed(b"-"),
            }
            feed(&m.body);
        }
        feed(format!("{:?}", self.timescale).as_bytes());
        feed(format!("{:?}", self.nettype).as_bytes());
        feed(
            format!(
                "{:?}",
                self.keywords.iter().map(|k| k.0).collect::<Vec<_>>()
            )
            .as_bytes(),
        );
        feed(&[self.celldefine as u8]);
        // What else decides how a file reads: the default timescale and where includes are searched.
        feed(format!("{:?}", opts.default_timescale).as_bytes());
        for d in &opts.include_dirs {
            feed(d.to_string_lossy().as_bytes());
        }
        format!("{h:032x}")
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum Event {
    Timescale(Option<Timescale>),
    Nettype(Nettype),
    CellDefine(bool),
    Keywords(KwSet),
}

#[derive(Clone, Copy, Debug)]
struct Seg {
    out: u32,
    file: u32,
    src: u32,
    src_end: u32,
    /// Output produced by a macro or include: all of it maps to the whole call site.
    expanded: bool,
}

/// A protected envelope removed from the text.
#[derive(Clone, Debug)]
pub(crate) struct Envelope {
    pub span: Span,
    /// Number of key blocks (tool vendors) in the envelope.
    pub keys: usize,
    pub terminated: bool,
}

#[derive(Default)]
pub(crate) struct Pre {
    pub text: Vec<u8>,
    segs: Vec<Seg>,
    pub events: Vec<(u32, Event)>,
    pub includes: Vec<PathBuf>,
    pub diags: Vec<Diag>,
    pub envelopes: Vec<Envelope>,
    /// The keyword set in force at the start of the text.
    pub kw_in: KwSet,
}

impl Pre {
    fn locate(&self, off: u32) -> Option<(Seg, u32)> {
        let i = self.segs.partition_point(|s| s.out <= off);
        let s = *self.segs.get(i.checked_sub(1)?)?;
        let d = off - s.out;
        Some((s, if s.expanded { 0 } else { d }))
    }

    /// The original span of the output bytes `start..end`.
    pub(crate) fn span(&self, start: u32, end: u32) -> Span {
        let Some((s, d)) = self.locate(start) else {
            return Span::default();
        };
        if s.expanded {
            return Span {
                file: s.file as usize,
                start: s.src,
                end: s.src_end,
            };
        }
        let a = s.src + d;
        let last = end.max(start + 1) - 1;
        let b = match self.locate(last) {
            Some((e, de)) if !e.expanded && e.file == s.file => (e.src + de + 1).max(a + 1),
            _ => (a + 1).min(s.src_end.max(a + 1)),
        };
        Span {
            file: s.file as usize,
            start: a,
            end: b.max(a + 1),
        }
    }

    /// The state in force at output offset `off`, from the incoming state.
    pub(crate) fn state_at(
        &self,
        off: u32,
        incoming: &State,
    ) -> (Option<Timescale>, Nettype, bool) {
        let mut t = incoming.timescale;
        let mut n = incoming.nettype;
        let mut c = incoming.celldefine;
        for (o, e) in &self.events {
            if *o > off {
                break;
            }
            match e {
                Event::Timescale(x) => t = *x,
                Event::Nettype(x) => n = *x,
                Event::CellDefine(x) => c = *x,
                Event::Keywords(_) => {}
            }
        }
        (t, n, c)
    }
}

#[derive(Clone, Copy)]
enum Origin {
    File(usize),
    /// Text of a macro expansion: the span of the call.
    Exp(Span),
}

impl Origin {
    fn span(self, a: usize, b: usize) -> Span {
        match self {
            Origin::File(f) => Span {
                file: f,
                start: a as u32,
                end: b.max(a + 1) as u32,
            },
            Origin::Exp(s) => s,
        }
    }
}

struct Frame {
    parent_active: bool,
    active: bool,
    taken: bool,
    saw_else: bool,
    span: Span,
}

#[derive(Default)]
struct Cond {
    frames: Vec<Frame>,
}

impl Cond {
    fn active(&self) -> bool {
        self.frames.last().is_none_or(|f| f.active)
    }
}

struct Pp<'a> {
    opts: &'a Options,
    files: &'a mut Vec<PathBuf>,
    st: &'a mut State,
    out: Pre,
    blown: bool,
}

/// Preprocess file number `file` (already in `files`) whose contents are `src`.
pub(crate) fn preprocess(
    opts: &Options,
    files: &mut Vec<PathBuf>,
    st: &mut State,
    file: usize,
    src: &[u8],
) -> Pre {
    let mut pp = Pp {
        opts,
        files,
        st,
        out: Pre::default(),
        blown: false,
    };
    pp.out.kw_in = pp.st.keywords.last().map_or(KwSet::V2005, |k| k.0);
    let mut cond = Cond::default();
    pp.process(src, Origin::File(file), &mut cond, 0);
    pp.end_cond(&mut cond);
    pp.out
}

/// The compiler directives of clause 19.
const DIRECTIVES: &[&str] = &[
    "begin_keywords",
    "celldefine",
    "default_nettype",
    "define",
    "else",
    "elsif",
    "end_keywords",
    "endcelldefine",
    "endif",
    "ifdef",
    "ifndef",
    "include",
    "line",
    "nounconnected_drive",
    "pragma",
    "resetall",
    "timescale",
    "unconnected_drive",
    "undef",
];

fn is_id_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_'
}
fn is_id(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c == b'$'
}

fn line_end(b: &[u8], mut i: usize) -> usize {
    while i < b.len() && b[i] != b'\n' {
        i += 1;
    }
    i
}

fn skip_ws(b: &[u8], mut i: usize) -> usize {
    while i < b.len() && matches!(b[i], b' ' | b'\t' | b'\r') {
        i += 1;
    }
    i
}

fn skip_string(b: &[u8], mut i: usize) -> usize {
    i += 1;
    while i < b.len() && b[i] != b'"' && b[i] != b'\n' {
        if b[i] == b'\\' {
            i += 1;
        }
        i += 1;
    }
    (i + 1).min(b.len().max(i))
}

fn ident_at(b: &[u8], i: usize) -> usize {
    let mut j = i;
    while j < b.len() && is_id(b[j]) {
        j += 1;
    }
    j
}

fn text(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

impl Pp<'_> {
    fn emit(&mut self, b: &[u8], from: usize, to: usize, origin: Origin) {
        if from >= to {
            return;
        }
        let out = self.out.text.len() as u32;
        self.out.text.extend_from_slice(&b[from..to]);
        // A stray marker byte in the source must not read as an envelope.
        if b[from..to].contains(&ENVELOPE) && b != [ENVELOPE] {
            for c in &mut self.out.text[out as usize..] {
                if *c == ENVELOPE {
                    *c = 0x02;
                }
            }
        }
        let seg = match origin {
            Origin::File(f) => Seg {
                out,
                file: f as u32,
                src: from as u32,
                src_end: to as u32,
                expanded: false,
            },
            Origin::Exp(s) => Seg {
                out,
                file: s.file as u32,
                src: s.start,
                src_end: s.end,
                expanded: true,
            },
        };
        if let Some(last) = self.out.segs.last_mut() {
            let len = out - last.out;
            let same = last.file == seg.file
                && last.expanded == seg.expanded
                && if seg.expanded {
                    last.src == seg.src && last.src_end == seg.src_end
                } else {
                    last.src_end == seg.src
                };
            if same {
                if !seg.expanded {
                    last.src_end = seg.src_end;
                }
                let _ = len;
                return;
            }
        }
        self.out.segs.push(seg);
    }

    fn event(&mut self, e: Event) {
        let at = self.out.text.len() as u32;
        self.out.events.push((at, e));
    }

    fn end_cond(&mut self, cond: &mut Cond) {
        while let Some(f) = cond.frames.pop() {
            self.out.diags.push(
                Diag::error("E4004", "`ifdef without a matching `endif", f.span)
                    .label("opened here")
                    .help("close the conditional with `endif in the same file"),
            );
        }
    }

    fn err(&mut self, code: &'static str, msg: String, span: Span) {
        self.out.diags.push(Diag::error(code, msg, span));
    }

    fn process(&mut self, b: &[u8], origin: Origin, cond: &mut Cond, depth: u32) {
        let n = b.len();
        let mut i = 0;
        let mut run = 0;
        while i < n {
            match b[i] {
                b'/' if i + 1 < n && b[i + 1] == b'/' => {
                    if cond.active() {
                        self.emit(b, run, i, origin);
                    }
                    i = line_end(b, i);
                    run = i;
                }
                b'/' if i + 1 < n && b[i + 1] == b'*' => {
                    if cond.active() {
                        self.emit(b, run, i, origin);
                    }
                    let start = i;
                    i += 2;
                    loop {
                        if i + 1 >= n {
                            i = n;
                            let s = origin.span(start, (start + 2).min(n));
                            self.err("E4014", "unterminated block comment".into(), s);
                            break;
                        }
                        if b[i] == b'*' && b[i + 1] == b'/' {
                            i += 2;
                            break;
                        }
                        i += 1;
                    }
                    if cond.active() {
                        self.emit(b" ", 0, 1, origin);
                    }
                    run = i;
                }
                b'"' => i = skip_string(b, i),
                b'`' => {
                    if cond.active() {
                        self.emit(b, run, i, origin);
                    }
                    i = self.directive(b, i, origin, cond, depth);
                    run = i;
                }
                _ => i += 1,
            }
        }
        if cond.active() {
            self.emit(b, run, n.min(i), origin);
        }
    }

    /// Handle the directive or macro use at `b[at] == '`'`; returns the index after it.
    fn directive(
        &mut self,
        b: &[u8],
        at: usize,
        origin: Origin,
        cond: &mut Cond,
        depth: u32,
    ) -> usize {
        let n = b.len();
        let ns = at + 1;
        let ne = ident_at(b, ns);
        let name = &b[ns..ne];
        let span = origin.span(at, ne.max(at + 1));
        if name.is_empty() {
            let c = b.get(ns).copied().unwrap_or(b' ');
            if cond.active() {
                if matches!(c, b'`' | b'"' | b'\\') {
                    // Both ends of a stringification map to the call: say it once.
                    let sp = origin.span(at, (ns + 1).min(n));
                    if self
                        .out
                        .diags
                        .last()
                        .is_some_and(|d| d.code == "E4003" && d.span == Some(sp))
                    {
                        return (ns + 1).min(n).max(ns);
                    }
                    self.out.diags.push(
                        Diag::error(
                            "E4003",
                            "SystemVerilog macro construct (`` or `\" in a macro body)",
                            origin.span(at, (ns + 1).min(n)),
                        )
                        .help("IEEE 1364-2005 has no token pasting or stringification in macros"),
                    );
                } else {
                    self.err(
                        "E4009",
                        "stray ` without a directive or macro name".into(),
                        span,
                    );
                }
            }
            return (ns + 1).min(n).max(ns);
        }
        let name_s = String::from_utf8_lossy(name).into_owned();
        // Conditionals are processed whether or not the current region is active.
        match name {
            b"ifdef" | b"ifndef" => {
                let (arg, end) = self.word_arg(b, ne);
                let parent = cond.active();
                let defined = arg.as_ref().is_some_and(|a| self.st.macros.contains_key(a));
                if parent && arg.is_none() {
                    self.err("E4009", format!("`{name_s} needs a macro name"), span);
                }
                let on = defined == (name == b"ifdef");
                cond.frames.push(Frame {
                    parent_active: parent,
                    active: parent && on,
                    taken: on,
                    saw_else: false,
                    span,
                });
                return end;
            }
            b"elsif" => {
                let (arg, end) = self.word_arg(b, ne);
                if arg.is_none() && cond.frames.last().is_some_and(|f| f.parent_active) {
                    self.err("E4009", "`elsif needs a macro name".into(), span);
                }
                let defined = arg.as_ref().is_some_and(|a| self.st.macros.contains_key(a));
                match cond.frames.last_mut() {
                    Some(f) if !f.saw_else => {
                        f.active = f.parent_active && !f.taken && defined;
                        f.taken |= defined;
                    }
                    _ => self.err(
                        "E4004",
                        "`elsif without an open `ifdef or after `else".into(),
                        span,
                    ),
                }
                return end;
            }
            b"else" => {
                match cond.frames.last_mut() {
                    Some(f) if !f.saw_else => {
                        f.saw_else = true;
                        f.active = f.parent_active && !f.taken;
                        f.taken = true;
                    }
                    _ => self.err(
                        "E4004",
                        "`else without an open `ifdef or after `else".into(),
                        span,
                    ),
                }
                return ne;
            }
            b"endif" => {
                if cond.frames.pop().is_none() {
                    self.err("E4004", "`endif without an open `ifdef".into(), span);
                }
                return ne;
            }
            _ => {}
        }
        if !cond.active() {
            // Skip the rest of a definition line so that its body is not read as text.
            return if name == b"define" {
                self.define_body_end(b, ne)
            } else {
                ne
            };
        }
        match name {
            b"define" => self.define(b, at, ne, origin),
            b"undef" => {
                let (arg, end) = self.word_arg(b, ne);
                match arg {
                    Some(a) => {
                        self.st.macros.remove(&a);
                    }
                    None => self.err("E4009", "`undef needs a macro name".into(), span),
                }
                end
            }
            b"undefineall" => {
                self.st.macros.clear();
                ne
            }
            b"include" => self.include(b, at, ne, origin, depth),
            b"timescale" => self.timescale(b, at, ne, origin),
            b"default_nettype" => {
                let s = skip_ws(b, ne);
                let e = ident_at(b, s);
                match Nettype::parse(&b[s..e]) {
                    Some(t) => {
                        self.st.nettype = t;
                        self.event(Event::Nettype(t));
                    }
                    None => self.err(
                        "E4009",
                        format!(
                            "`default_nettype needs a net type or `none`, found `{}`",
                            text(&b[s..e])
                        ),
                        origin.span(at, e),
                    ),
                }
                e
            }
            b"celldefine" | b"endcelldefine" => {
                self.st.celldefine = name == b"celldefine";
                self.event(Event::CellDefine(self.st.celldefine));
                ne
            }
            b"resetall" => {
                self.st.timescale = None;
                self.st.nettype = Nettype::Wire;
                self.st.celldefine = false;
                self.event(Event::Timescale(None));
                self.event(Event::Nettype(Nettype::Wire));
                self.event(Event::CellDefine(false));
                ne
            }
            b"nounconnected_drive" => line_end(b, ne),
            b"unconnected_drive" => {
                let e = line_end(b, ne);
                if !matches!(
                    text(&b[ne..e]).split("//").next().map(str::trim),
                    Some("pull0" | "pull1")
                ) {
                    self.err(
                        "E4009",
                        "`unconnected_drive needs `pull0` or `pull1`".into(),
                        origin.span(at, e),
                    );
                }
                e
            }
            b"line" => {
                let e = line_end(b, ne);
                let arg = text(&b[ne..e]);
                // Clause 19.7: `line number "filename" level, all three, nothing else on the line.
                let w: Vec<&str> = arg.split_whitespace().collect();
                let ok = w.len() == 3
                    && w[0].parse::<u64>().is_ok_and(|n| n > 0)
                    && w[1].len() >= 2
                    && w[1].starts_with('"')
                    && w[1].ends_with('"')
                    && matches!(w[2], "0" | "1" | "2");
                if !ok {
                    self.err(
                        "E4009",
                        "`line needs a positive number, a \"file name\" and a level 0, 1 or 2, and nothing else on the line".into(),
                        origin.span(at, e),
                    );
                }
                e
            }
            b"begin_keywords" => {
                let e = line_end(b, ne);
                let arg = text(&b[ne..e]);
                let spec = arg
                    .trim()
                    .strip_prefix('"')
                    .and_then(|r| r.split_once('"'))
                    .map(|(v, _)| v);
                let set = match spec {
                    Some("1364-2005") => Some(KwSet::V2005),
                    Some("1364-2001") => Some(KwSet::V2001),
                    Some("1364-2001-noconfig") => Some(KwSet::V2001NoConfig),
                    Some("1364-1995") => Some(KwSet::V1995),
                    _ => None,
                };
                match set {
                    Some(k) => {
                        self.st.keywords.push((k, span));
                        self.event(Event::Keywords(k));
                    }
                    None if arg.contains("1800") => self.out.diags.push(
                        Diag::error(
                            "E4003",
                            "SystemVerilog keyword set selected by `begin_keywords",
                            span,
                        )
                        .note(format!("selected: {}", arg.trim())),
                    ),
                    None => {
                        // keep the pair balanced: the matching `end_keywords is not a second error
                        let cur = self.st.keywords.last().map_or(KwSet::V2005, |k| k.0);
                        self.st.keywords.push((cur, span));
                        self.err(
                        "E4009",
                        "`begin_keywords needs \"1364-1995\", \"1364-2001\", \"1364-2001-noconfig\" or \"1364-2005\"".into(),
                        span,
                    );
                    }
                }
                e
            }
            b"end_keywords" => {
                if self.st.keywords.pop().is_none() {
                    self.err(
                        "E4004",
                        "`end_keywords without a `begin_keywords".into(),
                        span,
                    );
                }
                let k = self.st.keywords.last().map_or(KwSet::V2005, |k| k.0);
                self.event(Event::Keywords(k));
                ne
            }
            b"pragma" => self.pragma(b, at, ne, origin),
            b"protect" => self.envelope(b, at, ne, origin, b"`endprotect"),
            b"protected" => self.envelope(b, at, ne, origin, b"`endprotected"),
            b"endprotect" | b"endprotected" => ne,
            b"uselib" | b"config" => {
                self.err(
                    "E4010",
                    format!("unsupported compiler directive `{name_s}"),
                    span,
                );
                line_end(b, ne)
            }
            b"__FILE__" | b"__LINE__" => {
                self.err("E4003", format!("SystemVerilog directive `{name_s}"), span);
                ne
            }
            _ => self.use_macro(b, at, ne, &name_s, origin, cond, depth),
        }
    }

    fn word_arg(&self, b: &[u8], from: usize) -> (Option<String>, usize) {
        let s = skip_ws(b, from);
        let e = ident_at(b, s);
        if e == s {
            (None, s)
        } else {
            (Some(text(&b[s..e])), e)
        }
    }

    fn marker_warning(&mut self, span: Span) {
        self.out.diags.push(
            Diag::warning("W4009", "protection marker is ignored", span)
                .note("the plaintext after the marker is compiled"),
        );
    }

    /// The end of a `define starting after the directive name: the end of the logical line.
    fn define_body_end(&self, b: &[u8], mut i: usize) -> usize {
        let n = b.len();
        while i < n && b[i] != b'\n' {
            if b[i] == b'\\'
                && i + 1 < n
                && (b[i + 1] == b'\n' || (b[i + 1] == b'\r' && b.get(i + 2) == Some(&b'\n')))
            {
                i += 2;
                if b[i - 1] == b'\r' {
                    i += 1;
                }
                continue;
            }
            if b[i] == b'"' {
                i = skip_string(b, i);
                continue;
            }
            i += 1;
        }
        i
    }

    fn define(&mut self, b: &[u8], at: usize, ne: usize, origin: Origin) -> usize {
        let n = b.len();
        let s = skip_ws(b, ne);
        let e = ident_at(b, s);
        if e == s || !is_id_start(b[s]) {
            self.err(
                "E4008",
                "`define needs a macro name".into(),
                origin.span(at, ne),
            );
            return self.define_body_end(b, ne);
        }
        let name = text(&b[s..e]);
        if DIRECTIVES.contains(&name.as_str()) {
            self.err(
                "E4008",
                format!("`{name} is a compiler directive, it cannot be redefined as a macro (clause 19.3.1)"),
                origin.span(s, e),
            );
            return self.define_body_end(b, e);
        }
        let mut i = e;
        let mut params = None;
        if i < n && b[i] == b'(' {
            let mut list = Vec::new();
            i += 1;
            loop {
                i = skip_ws(b, i);
                let pe = ident_at(b, i);
                if pe == i {
                    self.err(
                        "E4008",
                        format!("bad formal argument list of macro `{name}`"),
                        origin.span(i, i + 1),
                    );
                    return self.define_body_end(b, i);
                }
                list.push(text(&b[i..pe]));
                i = skip_ws(b, pe);
                match b.get(i) {
                    Some(b',') => i += 1,
                    Some(b')') => {
                        i += 1;
                        break;
                    }
                    Some(b'=') => {
                        self.err(
                            "E4003",
                            format!("default argument in the definition of macro `{name}` (SystemVerilog)"),
                            origin.span(i, i + 1),
                        );
                        return self.define_body_end(b, i);
                    }
                    _ => {
                        self.err(
                            "E4008",
                            format!("bad formal argument list of macro `{name}`"),
                            origin.span(i.min(n), (i + 1).min(n)),
                        );
                        return self.define_body_end(b, i);
                    }
                }
            }
            params = Some(list);
        }
        // Body: to the end of the logical line, comments dropped, continuations kept as newlines.
        let mut body = Vec::new();
        while i < n && b[i] != b'\n' {
            match b[i] {
                b'\\'
                    if i + 1 < n
                        && (b[i + 1] == b'\n'
                            || (b[i + 1] == b'\r' && b.get(i + 2) == Some(&b'\n'))) =>
                {
                    body.push(b'\n');
                    i += if b[i + 1] == b'\r' { 3 } else { 2 };
                }
                b'/' if i + 1 < n && b[i + 1] == b'/' => i = line_end(b, i),
                b'/' if i + 1 < n && b[i + 1] == b'*' => {
                    let mut j = i + 2;
                    while j + 1 < n && !(b[j] == b'*' && b[j + 1] == b'/') {
                        j += 1;
                    }
                    i = (j + 2).min(n);
                    body.push(b' ');
                }
                b'"' => {
                    let j = skip_string(b, i);
                    body.extend_from_slice(&b[i..j]);
                    i = j;
                }
                b'\r' if b.get(i + 1) == Some(&b'\n') => i += 1,
                c => {
                    body.push(c);
                    i += 1;
                }
            }
        }
        let t = body.trim_ascii();
        self.st.macros.insert(
            name,
            Macro {
                params,
                body: t.to_vec(),
            },
        );
        i
    }

    fn include(&mut self, b: &[u8], at: usize, ne: usize, origin: Origin, depth: u32) -> usize {
        let s = skip_ws(b, ne);
        // The name is a quoted string, or a macro without arguments whose text is one.
        let from_macro = if b.get(s) == Some(&b'`') {
            let me = ident_at(b, s + 1);
            let m = self.st.macros.get(&text(&b[s + 1..me]));
            m.filter(|m| m.params.is_none())
                .map(|m| (m.body.clone(), me))
        } else {
            None
        };
        let (name_text, after) = match from_macro {
            Some((body, me)) => (body, me),
            None => (b[s..line_end(b, s)].to_vec(), 0),
        };
        let nb = name_text.trim_ascii();
        let close = match nb.first() {
            Some(b'"') => b'"',
            Some(b'<') => b'>',
            _ => {
                let e = line_end(b, s);
                self.err(
                    "E4009",
                    "`include needs a \"file name\"".into(),
                    origin.span(at, e.max(s)),
                );
                return e;
            }
        };
        let Some(len) = nb[1..].iter().position(|&c| c == close) else {
            let e = line_end(b, s);
            self.err(
                "E4009",
                "unterminated file name after `include".into(),
                origin.span(at, e),
            );
            return e;
        };
        let rel = text(&nb[1..=len]);
        // Written in the source itself: the directive ends at the closing quote.
        let after = if after == 0 { s + len + 2 } else { after };
        let span = origin.span(at, after);
        // Clause 19.5: only white space or a comment may follow on the line.
        let rest = text(&b[after.min(b.len())..line_end(b, after.min(b.len()))]);
        let rest = rest.trim();
        if !(rest.is_empty() || rest.starts_with("//") || rest.starts_with("/*")) {
            self.err(
                "E4009",
                "only white space or a comment may follow `include on its line (clause 19.5)"
                    .into(),
                span,
            );
        }
        if depth >= MAX_DEPTH {
            self.err("E4011", "`include nested too deeply".into(), span);
            return after;
        }
        let rel_p = Path::new(&rel);
        let mut tried: Vec<PathBuf> = Vec::new();
        let mut found = None;
        if rel_p.is_absolute() {
            tried.push(rel_p.to_path_buf());
        } else {
            let cur = &self.files[span.file];
            if let Some(d) = cur.parent() {
                tried.push(d.join(rel_p));
            }
            for d in &self.opts.include_dirs {
                tried.push(d.join(rel_p));
            }
        }
        for t in &tried {
            if let Ok(bytes) = std::fs::read(t) {
                found = Some((t.clone(), bytes));
                break;
            }
        }
        let Some((path, bytes)) = found else {
            let mut d = Diag::error("E4005", format!("include file `{rel}` not found"), span);
            let dirs: Vec<String> = tried
                .iter()
                .filter_map(|t| t.parent().map(|p| p.display().to_string()))
                .map(|p| if p.is_empty() { ".".to_string() } else { p })
                .collect();
            d = d.note(format!("searched: {}", dirs.join(", ")));
            self.out.diags.push(d.help("add the directory with -I DIR"));
            return after;
        };
        let abs = std::path::absolute(&path).unwrap_or(path);
        let idx = match self.files.iter().position(|f| *f == abs) {
            Some(k) => k,
            None => {
                self.files.push(abs.clone());
                self.files.len() - 1
            }
        };
        if !self.out.includes.contains(&abs) {
            self.out.includes.push(abs);
        }
        let mut inner = Cond::default();
        self.process(&bytes, Origin::File(idx), &mut inner, depth + 1);
        self.end_cond(&mut inner);
        // A token must not run into the text after the include.
        self.emit(b"\n", 0, 1, Origin::Exp(span));
        after
    }

    fn timescale(&mut self, b: &[u8], at: usize, ne: usize, origin: Origin) -> usize {
        let mut i = skip_ws(b, ne);
        let mut parts: Vec<u64> = Vec::new();
        for k in 0..2 {
            i = skip_ws(b, i);
            let ds = i;
            while i < b.len() && b[i].is_ascii_digit() {
                i += 1;
            }
            let mag: Option<u64> = std::str::from_utf8(&b[ds..i])
                .ok()
                .and_then(|s| s.parse().ok());
            i = skip_ws(b, i);
            let us = i;
            while i < b.len() && b[i].is_ascii_alphabetic() {
                i += 1;
            }
            let unit = match &b[us..i] {
                b"s" => Some(1_000_000_000_000_000u64),
                b"ms" => Some(1_000_000_000_000),
                b"us" => Some(1_000_000_000),
                b"ns" => Some(1_000_000),
                b"ps" => Some(1_000),
                b"fs" => Some(1),
                _ => None,
            };
            match (mag, unit) {
                (Some(m @ (1 | 10 | 100)), Some(u)) => parts.push(m * u),
                _ => {
                    self.err(
                        "E4009",
                        "`timescale needs `<1|10|100> <s|ms|us|ns|ps|fs> / <1|10|100> <unit>`"
                            .into(),
                        origin.span(at, i.max(at + 1)),
                    );
                    return line_end(b, i);
                }
            }
            if k == 0 {
                i = skip_ws(b, i);
                if b.get(i) != Some(&b'/') {
                    self.err(
                        "E4009",
                        "`timescale needs a `/` between unit and precision".into(),
                        origin.span(at, i.max(at + 1)),
                    );
                    return line_end(b, i);
                }
                i += 1;
            }
        }
        if parts[1] > parts[0] {
            self.err(
                "E4009",
                "the precision of `timescale must not be larger than its unit (clause 19.8)".into(),
                origin.span(at, i),
            );
            return i;
        }
        let ts = Timescale {
            unit_fs: parts[0],
            precision_fs: parts[1],
        };
        self.st.timescale = Some(ts);
        self.event(Event::Timescale(Some(ts)));
        i
    }

    fn pragma(&mut self, b: &[u8], at: usize, ne: usize, origin: Origin) -> usize {
        let le = line_end(b, ne);
        let line = text(&b[ne..le]);
        let mut w = line.split(|c: char| c.is_whitespace() || c == '=' || c == '(');
        let first = w.find(|s| !s.is_empty());
        if first.is_none_or(|f| f.starts_with("//")) {
            self.err(
                "E4009",
                "`pragma needs a pragma name".into(),
                origin.span(at, le),
            );
        }
        if first == Some("protect") {
            let second = w.find(|s| !s.is_empty());
            match second {
                Some("begin_protected") => {
                    return self.envelope(b, at, ne, origin, b"end_protected");
                }
                Some("begin" | "end") => self.marker_warning(origin.span(at, le)),
                _ => {}
            }
        }
        le
    }

    /// Skip a protected envelope starting at `at`: to the line that holds `end_marker`.
    fn envelope(
        &mut self,
        b: &[u8],
        at: usize,
        ne: usize,
        origin: Origin,
        end_marker: &[u8],
    ) -> usize {
        let n = b.len();
        let begin_end = line_end(b, ne);
        let mut keys = 0;
        let mut i = begin_end;
        let mut terminated = false;
        let mut stop = n;
        while i < n {
            let ls = i + 1;
            let le = line_end(b, ls.min(n));
            let line = &b[ls.min(n)..le];
            let t = line.trim_ascii();
            // Only pragma lines matter; the base64 body is skipped at once.
            let is_pragma = t.first() == Some(&b'`');
            if is_pragma && t.windows(11).any(|w| w == b"key_keyowne") {
                keys += 1;
            }
            let hit = is_pragma
                && if end_marker == b"end_protected" {
                    let s = String::from_utf8_lossy(t);
                    let mut w = s.split_whitespace();
                    w.next() == Some("`pragma")
                        && w.next() == Some("protect")
                        && w.next().is_some_and(|x| x.starts_with("end_protected"))
                } else {
                    t.starts_with(end_marker)
                };
            if hit {
                terminated = true;
                stop = le;
                break;
            }
            if le >= n {
                break;
            }
            i = le;
        }
        let span = origin.span(at, begin_end.max(at + 1));
        self.out.envelopes.push(Envelope {
            span,
            keys,
            terminated,
        });
        self.emit(&[ENVELOPE], 0, 1, Origin::Exp(span));
        stop.min(n)
    }

    #[allow(clippy::too_many_arguments)]
    fn use_macro(
        &mut self,
        b: &[u8],
        at: usize,
        ne: usize,
        name: &str,
        origin: Origin,
        cond: &mut Cond,
        depth: u32,
    ) -> usize {
        let call_span = origin.span(at, ne);
        if self.blown || self.out.text.len() > MAX_OUT {
            if !self.blown {
                self.blown = true;
                self.out.diags.push(
                    Diag::error("E4011", format!("the expansion of macro `{name} is too large"), call_span)
                        .note("the preprocessed text passed 64 MiB; a macro that uses itself several times per level grows without end"),
                );
            }
            return ne;
        }
        let Some(m) = self.st.macros.get(name).cloned() else {
            self.out.diags.push(
                Diag::error("E4007", format!("undefined macro `{name}"), call_span)
                    .help("define it with `define or -D"),
            );
            return ne;
        };
        let _ = cond;
        let mut end = ne;
        let body = match &m.params {
            None => m.body.clone(),
            Some(formals) => {
                let mut j = ne;
                while j < b.len() && b[j].is_ascii_whitespace() {
                    j += 1;
                }
                if b.get(j) != Some(&b'(') {
                    self.out.diags.push(
                        Diag::error(
                            "E4008",
                            format!("macro `{name} takes {} argument(s)", formals.len()),
                            call_span,
                        )
                        .help("call it as `NAME(arg, ...)"),
                    );
                    return ne;
                }
                let Some((args, after)) = gather_args(b, j) else {
                    self.err(
                        "E4008",
                        format!("unterminated argument list of macro `{name}"),
                        origin.span(at, b.len()),
                    );
                    return b.len();
                };
                end = after;
                if args.len() != formals.len() {
                    self.err(
                        "E4008",
                        format!(
                            "macro `{name} takes {} argument(s), {} given",
                            formals.len(),
                            args.len()
                        ),
                        origin.span(at, after),
                    );
                    return after;
                }
                subst(&m.body, formals, &args)
            }
        };
        if depth >= MAX_DEPTH {
            self.out.diags.push(
                Diag::error(
                    "E4011",
                    format!("macro `{name} expands too deeply"),
                    call_span,
                )
                .note("a macro that uses itself never ends"),
            );
            return end;
        }
        let exp = Origin::Exp(origin.span(at, end));
        let mut inner = Cond::default();
        self.process(&body, exp, &mut inner, depth + 1);
        self.end_cond(&mut inner);
        end
    }
}

/// Arguments of a macro call; `b[open] == '('`. Returns the arguments and the index after `)`.
fn gather_args(b: &[u8], open: usize) -> Option<(Vec<Vec<u8>>, usize)> {
    let n = b.len();
    let mut i = open + 1;
    let mut depth = 0i32;
    let mut cur: Vec<u8> = Vec::new();
    let mut args = Vec::new();
    while i < n {
        let c = b[i];
        match c {
            b'"' => {
                let j = skip_string(b, i);
                cur.extend_from_slice(&b[i..j]);
                i = j;
                continue;
            }
            b'/' if b.get(i + 1) == Some(&b'/') => {
                i = line_end(b, i);
                continue;
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                let mut j = i + 2;
                while j + 1 < n && !(b[j] == b'*' && b[j + 1] == b'/') {
                    j += 1;
                }
                i = (j + 2).min(n);
                cur.push(b' ');
                continue;
            }
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => {
                if depth == 0 {
                    if c == b')' {
                        args.push(cur.trim_ascii().to_vec());
                        return Some((args, i + 1));
                    }
                } else {
                    depth -= 1;
                }
            }
            b',' if depth == 0 => {
                args.push(cur.trim_ascii().to_vec());
                cur = Vec::new();
                i += 1;
                continue;
            }
            _ => {}
        }
        cur.push(c);
        i += 1;
    }
    None
}

/// Replace the formal arguments in a macro body (identifiers outside strings).
fn subst(body: &[u8], formals: &[String], args: &[Vec<u8>]) -> Vec<u8> {
    let n = body.len();
    let mut out = Vec::with_capacity(n);
    let mut i = 0;
    while i < n {
        let c = body[i];
        if c == b'"' {
            let j = skip_string(body, i);
            out.extend_from_slice(&body[i..j]);
            i = j;
        } else if is_id_start(c) {
            let e = ident_at(body, i);
            let w = &body[i..e];
            match formals.iter().position(|f| f.as_bytes() == w) {
                Some(k) => out.extend_from_slice(&args[k]),
                None => out.extend_from_slice(w),
            }
            i = e;
        } else if c == b'`' && i + 1 < n && is_id_start(body[i + 1]) {
            // A nested macro name is not a formal: keep it whole.
            let e = ident_at(body, i + 1);
            out.extend_from_slice(&body[i..e]);
            i = e;
        } else {
            out.push(c);
            i += 1;
        }
    }
    out
}
