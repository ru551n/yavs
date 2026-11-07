//! The parser: modules, UDPs, declarations, instances, generate, specify blocks.
//! Statements and expressions are in `stmt.rs`. A syntax error abandons the module it is in
//! (no cascades); parsing resumes after its `endmodule`.

use crate::ast::*;
use crate::lex::{K, Tok};
use crate::pre::{Nettype, Pre};
use crate::{Diag, Dir, Span, Timescale};

pub(crate) type R<T> = Result<T, ()>;

const MAX_DEPTH: u32 = 200;

/// Words that only exist in SystemVerilog; lexed as identifiers, named in E4003 where they matter.
const SV_WORDS: &[&str] = &[
    "logic",
    "bit",
    "byte",
    "int",
    "shortint",
    "longint",
    "string",
    "chandle",
    "shortreal",
    "enum",
    "struct",
    "union",
    "typedef",
    "always_comb",
    "always_ff",
    "always_latch",
    "final",
    "import",
    "export",
    "modport",
    "class",
    "package",
    "program",
    "interface",
    "checker",
    "clocking",
    "assert",
    "assume",
    "cover",
    "property",
    "sequence",
    "return",
    "break",
    "continue",
    "unique",
    "unique0",
    "priority",
    "inside",
    "do",
    "foreach",
    "join_any",
    "join_none",
    "var",
    "const",
    "void",
    "virtual",
    "covergroup",
    "bind",
    "timeunit",
    "timeprecision",
];

/// Punctuation that only exists in SystemVerilog.
const SV_PUNCT: &[&str] = &[
    "++", "--", "+=", "-=", "*=", "/=", "%=", "&=", "|=", "^=", "<<=", ">>=", "<<<=", ">>>=", "::",
    "##", "==?", "!=?", "|->", "|=>", "<->", "->>", "'",
];

pub(crate) struct Parser<'a> {
    pub pre: &'a Pre,
    toks: &'a [Tok],
    i: usize,
    pub diags: Vec<Diag>,
    depth: u32,
    /// Inside a function body (clause 10.4.4 limits the statements).
    pub(crate) in_func: bool,
    /// Inside a task, function or named block: declarations are local.
    local: u32,
    /// Inside a generate region or block (clause 12.4 limits the items).
    in_gen: u32,
    /// The genvars of the loop generate constructs being parsed.
    genvars: Vec<String>,
}

/// The timing checks of clause 15.1: name, fewest and most arguments, first argument needs an edge.
const TIMING_CHECKS: &[(&str, usize, usize, bool)] = &[
    ("$setup", 3, 4, false),
    ("$hold", 3, 4, false),
    ("$recovery", 3, 4, false),
    ("$removal", 3, 4, false),
    ("$skew", 3, 4, false),
    ("$setuphold", 4, 9, false),
    ("$recrem", 4, 9, false),
    ("$timeskew", 3, 6, false),
    ("$fullskew", 4, 7, false),
    ("$period", 2, 3, true),
    ("$width", 2, 4, true),
    ("$nochange", 4, 5, false),
];

impl<'a> Parser<'a> {
    pub(crate) fn new(pre: &'a Pre, toks: &'a [Tok]) -> Parser<'a> {
        Parser {
            pre,
            toks,
            i: 0,
            diags: Vec::new(),
            depth: 0,
            in_func: false,
            local: 0,
            in_gen: 0,
            genvars: Vec::new(),
        }
    }

    // ----- token access -----

    pub(crate) fn peek(&self) -> Tok {
        self.toks[self.i.min(self.toks.len() - 1)]
    }
    pub(crate) fn peek_at(&self, k: usize) -> Tok {
        self.toks[(self.i + k).min(self.toks.len() - 1)]
    }
    pub(crate) fn prev(&self) -> Option<Tok> {
        self.i.checked_sub(1).map(|j| self.toks[j])
    }
    pub(crate) fn bump(&mut self) -> Tok {
        let t = self.peek();
        if self.i < self.toks.len() - 1 {
            self.i += 1;
        }
        t
    }
    pub(crate) fn text(&self, t: Tok) -> String {
        match t.k {
            K::Ident => {
                let s = &self.pre.text[t.start as usize..t.end as usize];
                let s = if t.n == 1 { &s[1..] } else { s };
                String::from_utf8_lossy(s).into_owned()
            }
            K::Kw | K::P => t.s.to_string(),
            K::Eof => "end of file".into(),
            _ => String::from_utf8_lossy(&self.pre.text[t.start as usize..t.end as usize])
                .into_owned(),
        }
    }
    pub(crate) fn p(&self, s: &str) -> bool {
        let t = self.peek();
        t.k == K::P && t.s == s
    }
    pub(crate) fn p_at(&self, k: usize, s: &str) -> bool {
        let t = self.peek_at(k);
        t.k == K::P && t.s == s
    }
    pub(crate) fn kw(&self, s: &str) -> bool {
        let t = self.peek();
        t.k == K::Kw && t.s == s
    }
    pub(crate) fn eat_p(&mut self, s: &str) -> bool {
        let r = self.p(s);
        if r {
            self.bump();
        }
        r
    }
    pub(crate) fn eat_kw(&mut self, s: &str) -> bool {
        let r = self.kw(s);
        if r {
            self.bump();
        }
        r
    }
    pub(crate) fn expect_p(&mut self, s: &str) -> R<Tok> {
        if self.p(s) {
            Ok(self.bump())
        } else {
            self.fail(&format!("`{s}`"))
        }
    }
    pub(crate) fn expect_kw(&mut self, s: &str) -> R<Tok> {
        if self.kw(s) {
            Ok(self.bump())
        } else {
            self.fail(&format!("`{s}`"))
        }
    }
    pub(crate) fn ident(&mut self) -> R<Ident> {
        let t = self.peek();
        if t.k == K::Ident {
            self.bump();
            Ok(Ident {
                name: self.text(t),
                pos: t.start,
            })
        } else {
            self.fail("an identifier")
        }
    }

    pub(crate) fn span(&self, t: Tok) -> Span {
        self.pre.span(t.start, t.end)
    }

    pub(crate) fn push(&mut self, d: Diag) {
        self.diags.push(d);
    }

    pub(crate) fn sv_word(&self, t: Tok) -> bool {
        t.k == K::Ident && t.n == 0 && SV_WORDS.contains(&self.text(t).as_str())
    }

    /// Record a syntax error at the current token (E4003 when it, or the one before, is SystemVerilog).
    pub(crate) fn unexpected(&mut self, expected: &str) {
        let t = self.peek();
        let name = |s: &Self, t: Tok| s.text(t);
        let sv_cur = (t.k == K::P && SV_PUNCT.contains(&t.s)) || self.sv_word(t);
        let sv_prev = self
            .prev()
            .is_some_and(|p| self.sv_word(p) && t.k == K::Ident);
        if sv_cur || sv_prev {
            let w = if sv_cur { t } else { self.prev().unwrap() };
            self.push(
                Diag::error(
                    "E4003",
                    format!(
                        "SystemVerilog construct `{}` in a Verilog file",
                        name(self, w)
                    ),
                    self.span(w),
                )
                .label("not part of IEEE 1364-2005")
                .help("yavs reads Verilog (IEEE 1364-2005) only"),
            );
        } else {
            let found = match t.k {
                K::Eof => "end of file".to_string(),
                _ => format!("`{}`", self.text(t)),
            };
            self.push(
                Diag::error(
                    "E4100",
                    format!("expected {expected}, found {found}"),
                    self.span(t),
                )
                .label(format!("expected {expected}")),
            );
        }
    }

    pub(crate) fn enter(&mut self) -> R<()> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            let t = self.peek();
            self.push(Diag::error("E4100", "nesting too deep", self.span(t)));
            return Err(());
        }
        Ok(())
    }
    pub(crate) fn leave(&mut self) {
        self.depth -= 1;
    }

    /// Skip attribute instances `(* ... *)`.
    /// How many tokens the attribute instances at the cursor take (0 when there are none).
    pub(crate) fn attrs_len(&self) -> usize {
        let mut k = 0;
        while self.p_at(k, "(*") && !self.p_at(k + 1, ")") {
            k += 1;
            while !self.p_at(k, "*)") {
                if self.peek_at(k).k == K::Eof {
                    return k;
                }
                k += 1;
            }
            k += 1;
        }
        k
    }

    pub(crate) fn attrs(&mut self) -> R<()> {
        while self.p("(*") {
            // `@(* )` style star lists are handled by the event control parser, never reach here
            let open = self.bump();
            if self.p("*)") {
                self.push(Diag::error(
                    "E4100",
                    "an attribute instance names at least one attribute (clause 3.8)",
                    self.span(open),
                ));
                return Err(());
            }
            loop {
                let t = self.peek();
                if t.k == K::P && t.s == "*)" {
                    self.bump();
                    break;
                }
                if t.k == K::Eof {
                    self.push(Diag::error(
                        "E4100",
                        "unterminated attribute",
                        self.span(open),
                    ));
                    return Err(());
                }
                self.bump();
            }
        }
        Ok(())
    }

    // ----- file -----

    pub(crate) fn file(
        &mut self,
        state_at: &dyn Fn(u32) -> (Option<Timescale>, Nettype, bool),
    ) -> Vec<Decl> {
        let mut out = Vec::new();
        loop {
            if self.attrs().is_err() {
                break;
            }
            let t = self.peek();
            match (t.k, t.s) {
                (K::Eof, _) => break,
                (K::Env, _) => {
                    self.bump();
                    out.push(Decl::Envelope(t.n));
                }
                (K::Kw, "module" | "macromodule") => {
                    let (ts, nt, cd) = state_at(t.start);
                    match self.module(ts, nt, cd) {
                        Ok(m) => out.push(Decl::Module(m)),
                        Err(()) => self.skip_to(&["endmodule"]),
                    }
                }
                (K::Kw, "primitive") => match self.udp() {
                    Ok(u) => out.push(Decl::Udp(u)),
                    Err(()) => self.skip_to(&["endprimitive"]),
                },
                (K::Kw, "config") => {
                    self.push(
                        Diag::error("E4101", format!("`{}` is not supported", t.s), self.span(t))
                            .help("configurations and library maps are outside what yavs reads"),
                    );
                    self.skip_to(&["endmodule", "endconfig"]);
                }
                _ => {
                    // Nothing but module-level constructs is allowed here.
                    if t.k == K::Ident && self.text(t) == "library"
                        || t.k == K::Ident && self.text(t) == "include"
                    {
                        self.push(Diag::error(
                            "E4101",
                            format!("`{}` (library map) is not supported", self.text(t)),
                            self.span(t),
                        ));
                    } else {
                        self.top_level_error();
                    }
                    self.skip_to(&["endmodule", "endprimitive"]);
                }
            }
        }
        out
    }

    /// Something other than a module at the top level. A file that goes on with SystemVerilog
    /// declarations (a fragment of a SystemVerilog design) is reported as that.
    fn top_level_error(&mut self) {
        let mut k = 0;
        loop {
            let t = self.peek_at(k);
            if t.k == K::Eof || (t.k == K::Kw && matches!(t.s, "module" | "primitive")) {
                break;
            }
            if self.sv_word(t)
                && matches!(
                    self.text(t).as_str(),
                    "typedef"
                        | "enum"
                        | "struct"
                        | "union"
                        | "package"
                        | "import"
                        | "class"
                        | "interface"
                        | "program"
                        | "always_comb"
                        | "always_ff"
                        | "always_latch"
                )
            {
                self.push(
                    Diag::error(
                        "E4003",
                        format!(
                            "SystemVerilog construct `{}` in a Verilog file",
                            self.text(t)
                        ),
                        self.span(t),
                    )
                    .label("not part of IEEE 1364-2005")
                    .help("yavs reads Verilog (IEEE 1364-2005) only"),
                );
                return;
            }
            k += 1;
        }
        let t = self.peek();
        let found = if t.k == K::Eof {
            "end of file".to_string()
        } else {
            format!("`{}`", self.text(t))
        };
        self.push(
            Diag::error(
                "E4100",
                format!("expected `module` or `primitive`, found {found}"),
                self.span(t),
            )
            .label("expected `module` or `primitive`"),
        );
    }

    /// After an error: skip past the next of `stops`, or to the end of the file.
    fn skip_to(&mut self, stops: &[&str]) {
        loop {
            let t = self.peek();
            if t.k == K::Eof {
                return;
            }
            self.bump();
            if t.k == K::Kw && stops.contains(&t.s) {
                return;
            }
        }
    }

    // ----- module -----

    fn module(&mut self, ts: Option<Timescale>, nt: Nettype, cd: bool) -> R<Module> {
        self.bump();
        (self.in_func, self.local, self.in_gen) = (false, 0, 0);
        self.genvars.clear();
        self.attrs()?;
        let name = self.ident()?;
        // An envelope inside the header: only the name is readable.
        let mut k = 0;
        loop {
            let t = self.peek_at(k);
            match (t.k, t.s) {
                (K::Env, _) => {
                    for _ in 0..=k {
                        self.bump();
                    }
                    self.eat_kw("endmodule");
                    return Ok(Module {
                        name,
                        params: Vec::new(),
                        ports: Ports::None,
                        items: Vec::new(),
                        timescale: ts,
                        nettype: nt,
                        celldefine: cd,
                        encrypted: Some(t.n),
                    });
                }
                (K::Eof, _) | (K::P, ";") => break,
                _ => k += 1,
            }
        }
        let mut params = Vec::new();
        if self.eat_p("#") {
            self.expect_p("(")?;
            if !self.p(")") {
                loop {
                    self.attrs()?;
                    let local = self.kw("localparam");
                    if self.kw("parameter") || local {
                        self.bump();
                        params.push(self.param_head(local, false)?);
                    } else if params.is_empty() {
                        return self.fail("`parameter`");
                    } else {
                        // Another name sharing the previous declaration's type.
                        let prev = params.last().cloned().unwrap();
                        params.push(ParamDecl {
                            assigns: Vec::new(),
                            ..prev
                        });
                    }
                    let id = self.ident()?;
                    self.expect_p("=")?;
                    let v = self.mintypmax()?;
                    params.last_mut().unwrap().assigns.push((id, v));
                    if !self.eat_p(",") {
                        break;
                    }
                }
            }
            self.expect_p(")")?;
        }
        let mut ports = Ports::None;
        if self.eat_p("(") {
            ports = self.port_list()?;
            self.expect_p(")")?;
        }
        self.expect_p(";")?;
        let mut items = Vec::new();
        let mut encrypted = None;
        loop {
            self.attrs()?;
            let t = self.peek();
            match (t.k, t.s) {
                (K::Kw, "endmodule") => {
                    self.bump();
                    if self.p(":") {
                        return self.sv_end_label();
                    }
                    break;
                }
                (K::Env, _) => {
                    self.bump();
                    encrypted = Some(t.n);
                    // The envelope holds the body and usually the `endmodule`; plaintext after it
                    // belongs to the module, up to its `endmodule` (the next module is not swallowed).
                    loop {
                        let n = self.peek();
                        match (n.k, n.s) {
                            (K::Kw, "endmodule") => {
                                self.bump();
                                break;
                            }
                            (K::Eof, _) | (K::Kw, "module" | "primitive") => break,
                            _ => {
                                self.bump();
                            }
                        }
                    }
                    break;
                }
                (K::Eof, _) => {
                    let at = self.pre.span(name.pos, name.pos + name.name.len() as u32);
                    self.push(
                        Diag::error(
                            "E4100",
                            format!("module `{}` has no `endmodule`", name.name),
                            at,
                        )
                        .label("this module"),
                    );
                    return Err(());
                }
                (K::Kw, "module" | "macromodule" | "primitive") => {
                    self.push(if t.s == "primitive" {
                        Diag::error(
                            "E4100",
                            "a UDP is declared outside modules (clause 8.1)",
                            self.span(t),
                        )
                        .help("move the primitive before or after the module")
                    } else {
                        Diag::error(
                            "E4003",
                            "nested module declaration (SystemVerilog)",
                            self.span(t),
                        )
                        .help("declare the module outside; IEEE 1364-2005 has no nested modules")
                    });
                    // Skip the inner declaration, so the outer `endmodule` is the one the caller skips to.
                    self.skip_to(&[if t.s == "primitive" {
                        "endprimitive"
                    } else {
                        "endmodule"
                    }]);
                    return Err(());
                }
                _ => self.item(&mut items)?,
            }
        }
        Ok(Module {
            name,
            params,
            ports,
            items,
            timescale: ts,
            nettype: nt,
            celldefine: cd,
            encrypted,
        })
    }

    pub(crate) fn fail<T>(&mut self, expected: &str) -> R<T> {
        self.unexpected(expected);
        Err(())
    }

    fn sv_end_label<T>(&mut self) -> R<T> {
        let t = self.peek();
        self.push(
            Diag::error(
                "E4003",
                "block end label (`: name`) is SystemVerilog",
                self.span(t),
            )
            .help("IEEE 1364-2005 has no label after `end`, `endmodule` and the like; remove it"),
        );
        Err(())
    }

    fn port_list(&mut self) -> R<Ports> {
        self.attrs()?;
        if self.p(")") {
            return Ok(Ports::None);
        }
        let t = self.peek();
        if t.k == K::Kw && matches!(t.s, "input" | "output" | "inout") {
            let mut v: Vec<PortDecl> = Vec::new();
            loop {
                self.attrs()?;
                let t = self.peek();
                if t.k == K::Kw && matches!(t.s, "input" | "output" | "inout") {
                    let mut d = self.port_head()?;
                    d.names.push(self.declarator(false)?);
                    v.push(d);
                } else {
                    // Same type and direction as the port before.
                    let Some(prev) = v.last().cloned() else {
                        return self.fail("a port direction");
                    };
                    let mut d = PortDecl {
                        names: Vec::new(),
                        ..prev
                    };
                    d.names.push(self.declarator(false)?);
                    v.push(d);
                }
                if !self.eat_p(",") {
                    break;
                }
            }
            return Ok(Ports::Ansi(v));
        }
        let mut v = Vec::new();
        loop {
            self.attrs()?;
            if self.eat_p(".") {
                let name = self.ident()?;
                self.expect_p("(")?;
                let e = if self.p(")") {
                    None
                } else {
                    Some(self.expr()?)
                };
                self.expect_p(")")?;
                v.push(NonAnsiPort {
                    name: Some(name),
                    expr: e,
                    explicit: true,
                });
            } else if self.p("{") {
                let e = self.expr()?;
                v.push(NonAnsiPort {
                    name: None,
                    expr: Some(e),
                    explicit: false,
                });
            } else {
                let name = self.ident()?;
                // `a[3:0]` as a port reference
                if self.p("[") {
                    let mut e = Expr::Ident(name.clone());
                    e = self.selects(e)?;
                    v.push(NonAnsiPort {
                        name: Some(name),
                        expr: Some(e),
                        explicit: false,
                    });
                } else {
                    v.push(NonAnsiPort {
                        name: Some(name),
                        expr: None,
                        explicit: false,
                    });
                }
            }
            if !self.eat_p(",") {
                break;
            }
        }
        Ok(Ports::NonAnsi(v))
    }

    /// `input|output|inout [net|reg|...] [signed] [range]` with no names yet.
    fn port_head(&mut self) -> R<PortDecl> {
        let d = self.bump();
        let dir = match d.s {
            "input" => Dir::Input,
            "output" => Dir::Output,
            _ => Dir::Inout,
        };
        let mut net = None;
        let mut var = None;
        if let Some(n) = self.net_kind() {
            net = Some(n);
        } else if let Some(v) = self.var_kind() {
            var = Some(v);
        } else if self.peek().k == K::Ident {
            // `input logic a`, `input my_type a`
            let t = self.peek();
            if self.peek_at(1).k == K::Ident || self.peek_at(1).s == "[" || self.sv_word(t) {
                self.push(
                    Diag::error(
                        "E4003",
                        format!("data type `{}` on a port is SystemVerilog", self.text(t)),
                        self.span(t),
                    )
                    .help("use a net type or `reg`; IEEE 1364-2005 has no other port types"),
                );
                return Err(());
            }
        }
        let signed = self.signed();
        let range = self.opt_range()?;
        Ok(PortDecl {
            dir,
            net,
            var,
            signed,
            range,
            names: Vec::new(),
        })
    }

    pub(crate) fn net_kind(&mut self) -> Option<NetKind> {
        let t = self.peek();
        if t.k != K::Kw {
            return None;
        }
        let k = match t.s {
            "wire" => NetKind::Wire,
            "tri" => NetKind::Tri,
            "tri0" => NetKind::Tri0,
            "tri1" => NetKind::Tri1,
            "wand" => NetKind::Wand,
            "triand" => NetKind::Triand,
            "wor" => NetKind::Wor,
            "trior" => NetKind::Trior,
            "trireg" => NetKind::Trireg,
            "uwire" => NetKind::Uwire,
            "supply0" => NetKind::Supply0,
            "supply1" => NetKind::Supply1,
            _ => return None,
        };
        self.bump();
        Some(k)
    }

    pub(crate) fn var_kind(&mut self) -> Option<VarKind> {
        let t = self.peek();
        if t.k != K::Kw {
            return None;
        }
        let k = match t.s {
            "reg" => VarKind::Reg,
            "integer" => VarKind::Integer,
            "time" => VarKind::Time,
            "real" => VarKind::Real,
            "realtime" => VarKind::Realtime,
            _ => return None,
        };
        self.bump();
        Some(k)
    }

    fn signed(&mut self) -> bool {
        if self.kw("signed") {
            self.bump();
            true
        } else {
            self.eat_kw("unsigned");
            false
        }
    }

    pub(crate) fn opt_range(&mut self) -> R<Option<Range>> {
        if !self.p("[") {
            return Ok(None);
        }
        self.bump();
        let msb = self.expr()?;
        self.expect_p(":")?;
        let lsb = self.expr()?;
        self.expect_p("]")?;
        Ok(Some(Range { msb, lsb }))
    }

    /// `name {[range]} [= expr]`
    fn declarator(&mut self, dims_ok: bool) -> R<Declarator> {
        let name = self.ident()?;
        let mut dims = Vec::new();
        while dims_ok && self.p("[") {
            dims.extend(self.opt_range()?);
        }
        let init = if self.eat_p("=") {
            Some(self.expr()?)
        } else {
            None
        };
        Ok(Declarator { name, dims, init })
    }

    fn declarators(&mut self, dims_ok: bool) -> R<Vec<Declarator>> {
        let mut v = vec![self.declarator(dims_ok)?];
        while self.eat_p(",") {
            v.push(self.declarator(dims_ok)?);
        }
        Ok(v)
    }

    /// After `parameter`/`localparam`/`specparam`: type, sign and range (no names).
    fn param_head(&mut self, local: bool, spec: bool) -> R<ParamDecl> {
        let t = self.peek();
        if t.k == K::Ident && (self.peek_at(1).k == K::Ident || self.sv_word(t)) {
            self.push(
                Diag::error("E4003", format!("parameter type `{}` is SystemVerilog", self.text(t)), self.span(t))
                    .help("IEEE 1364-2005 parameters take `integer`, `real`, `realtime`, `time`, `signed` and a range"),
            );
            return Err(());
        }
        let kind = self.var_kind();
        let signed = self.signed();
        let at = self.peek();
        let range = self.opt_range()?;
        // Clause 4.10.1: a parameter has a type or a sign and a range, not both.
        if matches!(kind, Some(k) if k != VarKind::Reg) && (signed || range.is_some()) {
            self.push(Diag::error(
                "E4100",
                "a parameter with a type (integer, real, realtime, time) has no sign or range (clause 4.10.1)",
                self.span(at),
            ));
            return Err(());
        }
        Ok(ParamDecl {
            local,
            spec,
            kind,
            signed,
            range,
            assigns: Vec::new(),
        })
    }

    fn param_decl(&mut self, spec: bool) -> R<ParamDecl> {
        let t = self.bump();
        let mut d = self.param_head(t.s == "localparam", spec)?;
        loop {
            let id = self.ident()?;
            self.expect_p("=")?;
            let v = if spec && self.p("(") && self.paren_list_ahead() {
                self.bump();
                let mut es = vec![self.mintypmax()?];
                while self.eat_p(",") {
                    es.push(self.mintypmax()?);
                }
                self.expect_p(")")?;
                Expr::Concat(es)
            } else {
                self.mintypmax()?
            };
            d.assigns.push((id, v));
            if !self.eat_p(",") {
                break;
            }
        }
        Ok(d)
    }

    /// At `(`: whether a comma follows at the same depth before its `)`.
    fn paren_list_ahead(&self) -> bool {
        let mut depth = 0;
        let mut k = 0;
        loop {
            let t = self.peek_at(k);
            match (t.k, t.s) {
                (K::Eof, _) => return false,
                (K::P, "(" | "[" | "{") => depth += 1,
                (K::P, ")" | "]" | "}") => {
                    depth -= 1;
                    if depth == 0 {
                        return false;
                    }
                }
                (K::P, ",") if depth == 1 => return true,
                (K::P, ";") => return false,
                _ => {}
            }
            k += 1;
        }
    }

    pub(crate) fn strength(&mut self) -> R<Option<Strength>> {
        if !self.p("(") {
            return Ok(None);
        }
        let t1 = self.peek_at(1);
        let is = |t: Tok| {
            t.k == K::Kw
                && matches!(
                    t.s,
                    "supply0"
                        | "strong0"
                        | "pull0"
                        | "weak0"
                        | "supply1"
                        | "strong1"
                        | "pull1"
                        | "weak1"
                        | "highz0"
                        | "highz1"
                        | "small"
                        | "medium"
                        | "large"
                )
        };
        if !is(t1) {
            return Ok(None);
        }
        self.bump();
        let a = self.bump();
        if matches!(a.s, "small" | "medium" | "large") {
            self.expect_p(")")?;
            return Ok(Some(Strength::Charge(a.s)));
        }
        if self.eat_p(")") {
            // `pullup (strong1)`: one strength
            return Ok(Some(Strength::Drive(a.s, "")));
        }
        self.expect_p(",")?;
        let b = self.peek();
        if !is(b) {
            return self.fail("a drive strength");
        }
        self.bump();
        self.expect_p(")")?;
        // Clauses 6.1.4 and 7.1.2: one strength0 and one strength1, never both highz.
        let zero = |s: &str| s.ends_with('0');
        if zero(a.s) == zero(b.s) || (a.s.starts_with("highz") && b.s.starts_with("highz")) {
            self.push(Diag::error(
                "E4100",
                format!(
                    "`({}, {})` is not a drive strength: it is one strength0 and one strength1, and not both highz (clause 6.1.4)",
                    a.s, b.s
                ),
                self.span(a),
            ));
            return Err(());
        }
        Ok(Some(Strength::Drive(a.s, b.s)))
    }

    pub(crate) fn delay(&mut self) -> R<Option<Delay>> {
        if !self.eat_p("#") {
            return Ok(None);
        }
        if self.eat_p("(") {
            let mut v = vec![self.mintypmax()?];
            while self.eat_p(",") {
                v.push(self.mintypmax()?);
            }
            self.expect_p(")")?;
            return Ok(Some(Delay(v)));
        }
        let t = self.peek();
        let e = match t.k {
            K::Num => {
                self.bump();
                Expr::Num(self.text(t))
            }
            K::Real => {
                self.bump();
                Expr::Real(self.text(t))
            }
            K::Ident => {
                let id = self.ident()?;
                let mut e = Expr::Ident(id);
                while self.eat_p(".") {
                    e = Expr::Dot(Box::new(e), self.ident()?);
                }
                e
            }
            _ => return self.fail("a delay value"),
        };
        Ok(Some(Delay(vec![e])))
    }

    // ----- items -----

    pub(crate) fn item(&mut self, out: &mut Vec<Item>) -> R<()> {
        self.attrs()?;
        let t = self.peek();
        // Clause 12.4: a generate block has no port or parameter declarations, specify blocks or specparams.
        if self.in_gen > 0
            && self.local == 0
            && t.k == K::Kw
            && matches!(
                t.s,
                "input" | "output" | "inout" | "parameter" | "specparam" | "specify"
            )
        {
            self.push(Diag::error(
                "E4100",
                format!(
                    "a generate block has no `{}` declaration (clause 12.4)",
                    t.s
                ),
                self.span(t),
            ));
            return Err(());
        }
        match (t.k, t.s) {
            (K::P, ";") => {
                self.bump();
            }
            (K::Kw, "input" | "output" | "inout") => {
                let mut d = self.port_head()?;
                d.names = self.declarators(false)?;
                self.expect_p(";")?;
                out.push(Item::Port(d));
            }
            (K::Kw, "reg" | "integer" | "time" | "real" | "realtime") => {
                let kind = self.var_kind().unwrap();
                let signed = self.signed();
                let at = self.peek();
                let range = self.opt_range()?;
                if range.is_some() && kind != VarKind::Reg {
                    self.push(Diag::error(
                        "E4100",
                        format!("`{}` has no range, only `reg` does (clause 4.8)", t.s),
                        self.span(at),
                    ));
                    return Err(());
                }
                let names = self.declarators(true)?;
                self.expect_p(";")?;
                // Clause 6.2.1: a variable declaration assignment is for a scalar or vector at module level.
                for d in names.iter().filter(|d| d.init.is_some()) {
                    let why = if !d.dims.is_empty() {
                        "a variable declaration assignment is not allowed on an array"
                    } else if self.local > 0 {
                        "a variable declaration assignment is only allowed at module level"
                    } else {
                        continue;
                    };
                    let at = self
                        .pre
                        .span(d.name.pos, d.name.pos + d.name.name.len() as u32);
                    self.push(Diag::error("E4100", format!("{why} (clause 6.2.1)"), at));
                    return Err(());
                }
                out.push(Item::Var(VarDecl {
                    kind,
                    signed,
                    range,
                    names,
                }));
            }
            (
                K::Kw,
                "wire" | "tri" | "tri0" | "tri1" | "wand" | "triand" | "wor" | "trior" | "trireg"
                | "uwire" | "supply0" | "supply1",
            ) => {
                let kind = self.net_kind().unwrap();
                let st_tok = self.peek();
                let strength = self.strength()?;
                let vs_tok = self.peek();
                let vectored = if self.eat_kw("vectored") {
                    Some(true)
                } else if self.eat_kw("scalared") {
                    Some(false)
                } else {
                    None
                };
                let signed = self.signed();
                let range = self.opt_range()?;
                let delay = self.delay()?;
                let names = self.declarators(true)?;
                self.expect_p(";")?;
                self.net_strength(kind, st_tok, &strength, &names)?;
                if vectored.is_some() && range.is_none() {
                    self.push(Diag::error(
                        "E4100",
                        format!("`{}` goes with a range (clause 4.3.2)", vs_tok.s),
                        self.span(vs_tok),
                    ));
                    return Err(());
                }
                out.push(Item::Net(NetDecl {
                    kind,
                    strength,
                    vectored,
                    signed,
                    range,
                    delay,
                    names,
                }));
            }
            (K::Kw, "event") => {
                self.bump();
                let names = self.declarators(true)?;
                self.expect_p(";")?;
                out.push(Item::Event(names));
            }
            (K::Kw, "genvar") => {
                self.bump();
                let mut v = vec![self.ident()?];
                while self.eat_p(",") {
                    v.push(self.ident()?);
                }
                self.expect_p(";")?;
                out.push(Item::Genvar(v));
            }
            (K::Kw, "parameter" | "localparam") => {
                let d = self.param_decl(false)?;
                self.expect_p(";")?;
                out.push(Item::Param(d));
            }
            (K::Kw, "specparam") => {
                let d = self.param_decl(true)?;
                self.expect_p(";")?;
                out.push(Item::Param(d));
            }
            (K::Kw, "assign") => {
                self.bump();
                let st_tok = self.peek();
                let strength = self.strength()?;
                if matches!(
                    strength,
                    Some(Strength::Charge(_)) | Some(Strength::Drive(_, ""))
                ) {
                    self.push(Diag::error(
                        "E4100",
                        "a continuous assignment has a drive strength of two strengths (clause 6.1.4)",
                        self.span(st_tok),
                    ));
                    return Err(());
                }
                let delay = self.delay()?;
                let mut assigns = Vec::new();
                loop {
                    let l = self.lvalue()?;
                    self.expect_p("=")?;
                    assigns.push((l, self.expr()?));
                    if !self.eat_p(",") {
                        break;
                    }
                }
                self.expect_p(";")?;
                out.push(Item::Assign(ContAssign {
                    strength,
                    delay,
                    assigns,
                }));
            }
            (K::Kw, "defparam") => {
                self.bump();
                let mut v = Vec::new();
                loop {
                    let l = self.lvalue()?;
                    self.expect_p("=")?;
                    v.push((l, self.mintypmax()?));
                    if !self.eat_p(",") {
                        break;
                    }
                }
                self.expect_p(";")?;
                out.push(Item::Defparam(v));
            }
            (K::Kw, "always") => {
                self.bump();
                out.push(Item::Always(self.stmt()?));
            }
            (K::Kw, "initial") => {
                self.bump();
                out.push(Item::Initial(self.stmt()?));
            }
            (K::Kw, "task") => out.push(Item::Task(self.task()?)),
            (K::Kw, "function") => out.push(Item::Function(self.function()?)),
            (K::Kw, "generate") => {
                if self.in_gen > 0 {
                    self.push(Diag::error(
                        "E4100",
                        "generate regions do not nest and stand directly in a module (clause 12.4)",
                        self.span(t),
                    ));
                    return Err(());
                }
                self.bump();
                let mut v = Vec::new();
                self.in_gen += 1;
                while !self.kw("endgenerate") {
                    if self.peek().k == K::Eof {
                        return self.fail("`endgenerate`");
                    }
                    self.item(&mut v)?;
                }
                self.in_gen -= 1;
                self.bump();
                out.push(Item::Generate(v));
            }
            (K::Kw, "if" | "for" | "case" | "casex" | "casez") => out.push(self.gen_construct()?),
            // Accepted by the other tools: a generate block that stands alone.
            (K::Kw, "begin") => {
                let b = self.gen_body()?;
                out.push(Item::GenBlock(b));
            }
            (K::Kw, "specify") => out.push(self.specify()?),
            (K::Sys, _)
                if matches!(
                    self.text(t).as_str(),
                    "$error" | "$warning" | "$info" | "$fatal"
                ) =>
            {
                self.bump();
                let name = self.text(t);
                let mut args = Vec::new();
                if self.eat_p("(") {
                    args = self.args()?;
                    self.expect_p(")")?;
                }
                self.expect_p(";")?;
                out.push(Item::ElabTask { name, args });
            }
            (
                K::Kw,
                "and" | "nand" | "or" | "nor" | "xor" | "xnor" | "buf" | "not" | "bufif0"
                | "bufif1" | "notif0" | "notif1" | "pullup" | "pulldown" | "nmos" | "pmos"
                | "rnmos" | "rpmos" | "cmos" | "rcmos" | "tran" | "rtran" | "tranif0" | "tranif1"
                | "rtranif0" | "rtranif1",
            ) => {
                self.bump();
                let st_tok = self.peek();
                let strength = self.strength()?;
                let d_tok = self.peek();
                let delay = self.delay()?;
                self.gate_spec(t, st_tok, &strength, d_tok, &delay)?;
                let mut insts = Vec::new();
                loop {
                    insts.push(self.instance(false)?);
                    if !self.eat_p(",") {
                        break;
                    }
                }
                self.expect_p(";")?;
                self.gate_terminals(t, &insts)?;
                out.push(Item::Gate(GateInst {
                    gate: t.s,
                    strength,
                    delay,
                    insts,
                }));
            }
            (K::Ident, _) => {
                if self.sv_word(t) {
                    let inst_shape =
                        (self.peek_at(1).k == K::Ident && self.p_at(2, "(")) || self.p_at(1, "#");
                    let strong = matches!(
                        self.text(t).as_str(),
                        "assert"
                            | "assume"
                            | "cover"
                            | "always_comb"
                            | "always_ff"
                            | "always_latch"
                            | "final"
                            | "import"
                            | "export"
                            | "typedef"
                            | "interface"
                            | "class"
                            | "package"
                            | "program"
                            | "property"
                            | "sequence"
                    );
                    if strong || !inst_shape {
                        self.unexpected("a module item");
                        return Err(());
                    }
                }
                let module = self.ident()?;
                // `udp (strong0, weak1) #2 u (o, a);`: a drive strength can only belong to a UDP instance.
                let st_tok = self.peek();
                let strength = self.strength()?;
                if matches!(
                    strength,
                    Some(Strength::Charge(_)) | Some(Strength::Drive(_, ""))
                ) {
                    self.push(Diag::error(
                        "E4100",
                        "an instance has a drive strength of two strengths (clause 8.6)",
                        self.span(st_tok),
                    ));
                    return Err(());
                }
                let mut params = Vec::new();
                if self.eat_p("#") {
                    if self.eat_p("(") {
                        if !self.p(")") {
                            params = self.conns(true)?;
                        }
                        self.expect_p(")")?;
                    } else {
                        let d = self.delay_value_tail()?;
                        params.push(Conn::Ordered(d));
                    }
                }
                let mut insts = Vec::new();
                loop {
                    insts.push(self.instance(true)?);
                    if !self.eat_p(",") {
                        break;
                    }
                }
                self.expect_p(";")?;
                out.push(Item::Inst(ModInst {
                    module,
                    strength,
                    params,
                    insts,
                }));
            }
            _ => return self.fail("a module item"),
        }
        Ok(())
    }

    /// After a consumed `#`: a bare delay value (number or name).
    fn delay_value_tail(&mut self) -> R<Expr> {
        let t = self.peek();
        match t.k {
            K::Num => {
                self.bump();
                Ok(Expr::Num(self.text(t)))
            }
            K::Real => {
                self.bump();
                Ok(Expr::Real(self.text(t)))
            }
            K::Ident => Ok(Expr::Ident(self.ident()?)),
            _ => self.fail("a delay value or `(`"),
        }
    }

    /// `[name] [range] ( connections )`
    fn instance(&mut self, named: bool) -> R<Instance> {
        let _ = named;
        let mut name = None;
        let mut range = None;
        if self.peek().k == K::Ident {
            name = Some(self.ident()?);
            range = self.opt_range()?;
        }
        self.expect_p("(")?;
        let conns = if self.p(")") {
            Vec::new()
        } else {
            self.conns(false)?
        };
        self.expect_p(")")?;
        Ok(Instance { name, range, conns })
    }

    /// Ordered or named connections up to the closing `)`.
    fn conns(&mut self, params: bool) -> R<Vec<Conn>> {
        let mut v: Vec<Conn> = Vec::new();
        loop {
            self.attrs()?;
            let named_next = self.p(".");
            if let Some(first) = v.first() {
                // Clauses 12.3.6 and 12.2.2: a list is all by position or all by name.
                if matches!(first, Conn::Named(..)) != named_next && !(self.p(",") || self.p(")")) {
                    let t = self.peek();
                    let msg = if params {
                        "parameter assignments are all by position or all by name (clause 12.2.2)"
                    } else {
                        "port connections are all by position or all by name (clause 12.3.6)"
                    };
                    self.push(Diag::error("E4100", msg, self.span(t)));
                    return Err(());
                }
            }
            if self.p(".") {
                let dot = self.bump();
                if self.p("*") {
                    self.push(Diag::error(
                        "E4003",
                        "`.*` port connection is SystemVerilog",
                        self.span(dot),
                    ));
                    return Err(());
                }
                let name = self.ident()?;
                if !self.p("(") {
                    self.push(
                        Diag::error(
                            "E4003",
                            format!("`.{}` implicit port connection is SystemVerilog", name.name),
                            self.span(dot),
                        )
                        .help(format!("write `.{0}({0})`", name.name)),
                    );
                    return Err(());
                }
                self.bump();
                let e = if self.p(")") {
                    None
                } else {
                    Some(self.mintypmax()?)
                };
                self.expect_p(")")?;
                if v.iter()
                    .any(|c| matches!(c, Conn::Named(n, _) if n.name == name.name))
                {
                    let msg = if params {
                        format!(
                            "parameter `{}` is assigned twice (clause 12.2.2)",
                            name.name
                        )
                    } else {
                        format!("port `{}` is connected twice (clause 12.3.6)", name.name)
                    };
                    self.push(Diag::error(
                        "E4100",
                        msg,
                        self.pre.span(name.pos, name.pos + name.name.len() as u32),
                    ));
                    return Err(());
                }
                v.push(Conn::Named(name, e));
            } else if self.p(",") || self.p(")") {
                v.push(Conn::Ordered(Expr::Empty));
            } else {
                v.push(Conn::Ordered(self.mintypmax()?));
            }
            if !self.eat_p(",") {
                break;
            }
            if self.p(")") {
                v.push(Conn::Ordered(Expr::Empty));
                break;
            }
        }
        Ok(v)
    }

    // ----- tasks and functions -----

    /// Declarations in a task, function or block: returns whether one was parsed.
    pub(crate) fn block_decl(&mut self, out: &mut Vec<Item>, ports_ok: bool) -> R<bool> {
        let t = self.peek_at(self.attrs_len());
        if t.k == K::Kw
            && (ports_ok || !matches!(t.s, "input" | "output" | "inout"))
            && matches!(
                t.s,
                "input"
                    | "output"
                    | "inout"
                    | "reg"
                    | "integer"
                    | "time"
                    | "real"
                    | "realtime"
                    | "event"
                    | "parameter"
                    | "localparam"
            )
        {
            self.local += 1;
            let r = self.item(out);
            self.local -= 1;
            r?;
            return Ok(true);
        }
        Ok(false)
    }

    fn tf_ports(&mut self, decls: &mut Vec<Item>) -> R<()> {
        // `( input a, b, output c )`
        self.expect_p("(")?;
        if self.eat_p(")") {
            return Ok(());
        }
        let mut last: Option<PortDecl> = None;
        loop {
            self.attrs()?;
            let t = self.peek();
            let mut d = if t.k == K::Kw && matches!(t.s, "input" | "output" | "inout") {
                self.port_head()?
            } else if let Some(p) = &last {
                PortDecl {
                    names: Vec::new(),
                    ..p.clone()
                }
            } else {
                return self.fail("a port direction");
            };
            d.names.push(self.declarator(false)?);
            last = Some(d.clone());
            decls.push(Item::Port(d));
            if !self.eat_p(",") {
                break;
            }
        }
        self.expect_p(")")?;
        Ok(())
    }

    /// A case statement or generate case has at most one `default` (clauses 9.5 and 12.4).
    pub(crate) fn second_default(&mut self, seen: u32) -> R<()> {
        if seen > 1 {
            let t = self.peek();
            self.push(Diag::error(
                "E4100",
                "a case has at most one `default` item (clause 9.5)",
                self.span(t),
            ));
            return Err(());
        }
        Ok(())
    }

    fn task(&mut self) -> R<Task> {
        self.bump();
        let automatic = self.eat_kw("automatic");
        let name = self.ident()?;
        let mut decls = Vec::new();
        let ansi = self.p("(");
        if ansi {
            self.tf_ports(&mut decls)?;
        }
        self.expect_p(";")?;
        while self.block_decl(&mut decls, !ansi)? {}
        let mut body = Vec::new();
        while !self.kw("endtask") {
            if self.peek().k == K::Eof {
                return self.fail("`endtask`");
            }
            body.push(self.stmt()?);
        }
        self.bump();
        if self.p(":") {
            return self.sv_end_label();
        }
        Ok(Task {
            automatic,
            name,
            decls,
            body,
        })
    }

    fn function(&mut self) -> R<Function> {
        self.bump();
        let automatic = self.eat_kw("automatic");
        let signed = self.signed();
        let t = self.peek();
        if t.k == K::Ident && (self.peek_at(1).k == K::Ident || self.p_at(1, "[")) {
            self.push(
                Diag::error("E4003", format!("function return type `{}` is SystemVerilog", self.text(t)), self.span(t))
                    .help("IEEE 1364-2005 functions return a range, `integer`, `real`, `realtime` or `time`"),
            );
            return Err(());
        }
        let range = self.opt_range()?;
        let ret = self.var_kind();
        let name = self.ident()?;
        let mut decls = Vec::new();
        let ansi = self.p("(");
        if ansi {
            self.tf_ports(&mut decls)?;
        }
        self.expect_p(";")?;
        while self.block_decl(&mut decls, !ansi)? {}
        let mut body = Vec::new();
        self.in_func = true;
        while !self.kw("endfunction") {
            if self.peek().k == K::Eof {
                self.in_func = false;
                return self.fail("`endfunction`");
            }
            body.push(self.stmt()?);
        }
        self.in_func = false;
        self.bump();
        if self.p(":") {
            return self.sv_end_label();
        }
        // Clauses 10.4.1 and 10.4.4: at least one input argument and no output or inout.
        let ports = decls.iter().filter_map(|d| match d {
            Item::Port(p) => Some(p.dir),
            _ => None,
        });
        let (mut inputs, mut others) = (0, 0);
        ports.for_each(|d| {
            if d == Dir::Input {
                inputs += 1
            } else {
                others += 1
            }
        });
        if inputs == 0 || others > 0 {
            let at = self.pre.span(name.pos, name.pos + name.name.len() as u32);
            let why = if others > 0 {
                "a function has input arguments only, no output or inout"
            } else {
                "a function has at least one input argument"
            };
            self.push(Diag::error("E4100", format!("{why} (clause 10.4.4)"), at));
            return Err(());
        }
        Ok(Function {
            automatic,
            name,
            signed,
            range,
            ret,
            decls,
            body,
        })
    }

    // ----- generate -----

    fn gen_body(&mut self) -> R<GenBody> {
        self.in_gen += 1;
        let r = self.gen_body_inner();
        self.in_gen -= 1;
        r
    }

    fn gen_body_inner(&mut self) -> R<GenBody> {
        self.attrs()?;
        if self.eat_kw("begin") {
            let mut label = None;
            if self.eat_p(":") {
                label = Some(self.ident()?);
            }
            let mut items = Vec::new();
            while !self.kw("end") {
                if self.peek().k == K::Eof {
                    return self.fail("`end`");
                }
                self.item(&mut items)?;
            }
            self.bump();
            if self.p(":") {
                return self.sv_end_label();
            }
            Ok(GenBody { label, items })
        } else {
            let mut items = Vec::new();
            self.item(&mut items)?;
            Ok(GenBody { label: None, items })
        }
    }

    fn gen_construct(&mut self) -> R<Item> {
        self.enter()?;
        let r = self.gen_construct_inner();
        self.leave();
        r
    }

    fn gen_construct_inner(&mut self) -> R<Item> {
        let t = self.bump();
        match t.s {
            "if" => {
                self.expect_p("(")?;
                let cond = self.expr()?;
                self.expect_p(")")?;
                let then = self.gen_body()?;
                let els = if self.eat_kw("else") {
                    Some(self.gen_body()?)
                } else {
                    None
                };
                Ok(Item::GenIf { cond, then, els })
            }
            "for" => {
                self.expect_p("(")?;
                let i = self.ident()?;
                self.expect_p("=")?;
                let init = self.expr()?;
                self.expect_p(";")?;
                let cond = self.expr()?;
                self.expect_p(";")?;
                let s_tok = self.peek();
                let s = self.ident()?;
                self.expect_p("=")?;
                let step = self.expr()?;
                self.expect_p(")")?;
                let why = if s.name != i.name {
                    Some(
                        "the initialization and the iteration of a generate loop assign the same genvar",
                    )
                } else if self.genvars.contains(&i.name) {
                    Some("nested generate loops do not use the same genvar")
                } else {
                    None
                };
                if let Some(w) = why {
                    self.push(Diag::error(
                        "E4100",
                        format!("{w} (clause 12.4.1)"),
                        self.span(s_tok),
                    ));
                    return Err(());
                }
                self.genvars.push(i.name.clone());
                let body = self.gen_body();
                self.genvars.pop();
                let body = body?;
                Ok(Item::GenFor {
                    init: (i, init),
                    cond,
                    step: (s, step),
                    body,
                })
            }
            _ => {
                self.expect_p("(")?;
                let expr = self.expr()?;
                self.expect_p(")")?;
                let mut items = Vec::new();
                let mut defaults = 0;
                while !self.kw("endcase") {
                    if self.peek().k == K::Eof {
                        return self.fail("`endcase`");
                    }
                    let mut labels = Vec::new();
                    if self.kw("default") {
                        defaults += 1;
                        self.second_default(defaults)?;
                        self.bump();
                        self.eat_p(":");
                    } else {
                        loop {
                            labels.push(self.expr()?);
                            if !self.eat_p(",") {
                                break;
                            }
                        }
                        self.expect_p(":")?;
                    }
                    items.push((labels, self.gen_body()?));
                }
                self.bump();
                Ok(Item::GenCase { expr, items })
            }
        }
    }

    // ----- specify -----

    fn specify(&mut self) -> R<Item> {
        self.bump();
        let mut v = Vec::new();
        // The outputs of the paths so far (clause 14.6.4: a pulse style comes before the paths).
        let mut path_outputs: Vec<String> = Vec::new();
        while !self.kw("endspecify") {
            self.attrs()?;
            let t = self.peek();
            match (t.k, t.s) {
                (K::Eof, _) => return self.fail("`endspecify`"),
                (K::Kw, "specparam") => {
                    let d = self.param_decl(true)?;
                    self.expect_p(";")?;
                    v.push(SpecItem::Param(d));
                }
                (
                    K::Kw,
                    "pulsestyle_onevent"
                    | "pulsestyle_ondetect"
                    | "showcancelled"
                    | "noshowcancelled",
                ) => {
                    self.bump();
                    let at = self.peek();
                    let mut outputs = vec![self.terminal()?];
                    while self.eat_p(",") {
                        outputs.push(self.terminal()?);
                    }
                    self.expect_p(";")?;
                    if outputs
                        .iter()
                        .any(|o| path_outputs.contains(&crate::analysis::show(o)))
                    {
                        self.push(Diag::error(
                            "E4100",
                            format!("`{}` names an output that a path declaration already used (clause 14.6.4)", t.s),
                            self.span(at),
                        ));
                        return Err(());
                    }
                    v.push(SpecItem::Pulse { kind: t.s, outputs });
                }
                (K::Sys, _) => {
                    self.bump();
                    let name = self.text(t);
                    // Clause 15.1: a specify block holds timing checks, no system task.
                    let Some(&(_, lo, hi, controlled)) = TIMING_CHECKS.iter().find(|c| c.0 == name)
                    else {
                        self.push(Diag::error(
                            "E4100",
                            format!("`{name}` is not a timing check, and no system task can appear in a specify block (clause 15.1)"),
                            self.span(t),
                        ));
                        return Err(());
                    };
                    self.expect_p("(")?;
                    let mut args = Vec::new();
                    loop {
                        args.push(self.tc_arg()?);
                        if !self.eat_p(",") {
                            break;
                        }
                    }
                    if !(lo..=hi).contains(&args.len()) || (controlled && args[0].edge.is_none()) {
                        let why = if (lo..=hi).contains(&args.len()) {
                            "starts with a controlled event: posedge, negedge or edge".to_string()
                        } else if lo == hi {
                            format!("takes {lo} arguments, found {}", args.len())
                        } else {
                            format!("takes {lo} to {hi} arguments, found {}", args.len())
                        };
                        self.push(Diag::error(
                            "E4100",
                            format!("`{name}` {why} (clause 15.1)"),
                            self.span(t),
                        ));
                        return Err(());
                    }
                    self.expect_p(")")?;
                    self.expect_p(";")?;
                    v.push(SpecItem::Check { name, args });
                }
                (K::Kw, "if") => {
                    self.bump();
                    self.expect_p("(")?;
                    let c = self.expr()?;
                    self.expect_p(")")?;
                    let mut p = self.path_decl()?;
                    p.cond = Some(c);
                    v.push(SpecItem::Path(p));
                }
                (K::Kw, "ifnone") => {
                    self.bump();
                    let mut p = self.path_decl()?;
                    if p.edge.is_some() || p.data.is_some() {
                        self.push(Diag::error(
                            "E4100",
                            "`ifnone` goes with a simple path, not an edge-sensitive one (clause 14.2.4.4)",
                            self.span(t),
                        ));
                        return Err(());
                    }
                    p.ifnone = true;
                    v.push(SpecItem::Path(p));
                }
                (K::P, "(") => v.push(SpecItem::Path(self.path_decl()?)),
                _ => return self.fail("a specify item"),
            }
            if let Some(SpecItem::Path(p)) = v.last() {
                path_outputs.extend(p.dst.iter().map(crate::analysis::show));
            }
        }
        self.bump();
        Ok(Item::Specify(v))
    }

    /// A terminal: a name with dots and selects.
    fn terminal(&mut self) -> R<Expr> {
        let id = self.ident()?;
        let mut e = Expr::Ident(id);
        loop {
            if self.eat_p(".") {
                e = Expr::Dot(Box::new(e), self.ident()?);
            } else if self.p("[") {
                e = self.selects(e)?;
            } else {
                return Ok(e);
            }
        }
    }

    fn path_decl(&mut self) -> R<PathDecl> {
        self.expect_p("(")?;
        let edge = if self.kw("posedge") || self.kw("negedge") {
            Some(self.bump().s)
        } else {
            None
        };
        let mut src = vec![self.terminal()?];
        while self.eat_p(",") {
            src.push(self.terminal()?);
        }
        // Polarity and the connection: `=>`, `+=>`, `-=>`, `*>`, `+*>`, `-*>` (`+=` and `-=` lex as one token).
        let mut polarity = None;
        let t = self.peek();
        if t.k == K::P {
            match t.s {
                "+" | "-" => {
                    polarity = t.s.chars().next();
                    self.bump();
                }
                "+=" | "-=" => {
                    polarity = t.s.chars().next();
                    self.bump();
                    // the `>` that followed the `=`
                    self.expect_p(">")?;
                    return self.path_tail(edge, src, false, polarity);
                }
                _ => {}
            }
        }
        let t = self.peek();
        let full = match (t.k, t.s) {
            (K::P, "=>") => false,
            (K::P, "*>") => true,
            _ => return self.fail("`=>` or `*>`"),
        };
        self.bump();
        self.path_tail(edge, src, full, polarity)
    }

    fn path_tail(
        &mut self,
        edge: Option<&'static str>,
        src: Vec<Expr>,
        full: bool,
        polarity: Option<char>,
    ) -> R<PathDecl> {
        let mut dst = Vec::new();
        let mut data = None;
        let mut polarity = polarity;
        if self.eat_p("(") {
            dst.push(self.terminal()?);
            while self.eat_p(",") {
                dst.push(self.terminal()?);
            }
            let t = self.peek();
            match (t.k, t.s) {
                (K::P, "+:" | "-:") => {
                    polarity = t.s.chars().next();
                    self.bump();
                }
                (K::P, "+" | "-") => {
                    polarity = t.s.chars().next();
                    self.bump();
                    self.expect_p(":")?;
                }
                (K::P, ":") => {
                    self.bump();
                }
                _ => return self.fail("`:` before the data source"),
            }
            data = Some(self.expr()?);
            self.expect_p(")")?;
        } else {
            dst.push(self.terminal()?);
            while self.eat_p(",") {
                dst.push(self.terminal()?);
            }
        }
        self.expect_p(")")?;
        if !full && (src.len() > 1 || dst.len() > 1) {
            let at = self.peek();
            self.push(Diag::error(
                "E4100",
                "a parallel path (`=>`) has one input and one output, use `*>` for lists (clause 14.2.2)",
                self.span(at),
            ));
            return Err(());
        }
        self.expect_p("=")?;
        let mut delays = Vec::new();
        let open = self.peek();
        if self.eat_p("(") {
            delays.push(self.mintypmax()?);
            while self.eat_p(",") {
                delays.push(self.mintypmax()?);
            }
            self.expect_p(")")?;
            if !matches!(delays.len(), 1 | 2 | 3 | 6 | 12) {
                self.push(Diag::error(
                    "E4100",
                    format!(
                        "a path delay has 1, 2, 3, 6 or 12 values, found {} (clause 14.3)",
                        delays.len()
                    ),
                    self.span(open),
                ));
                return Err(());
            }
        } else {
            delays.push(self.mintypmax()?);
        }
        self.expect_p(";")?;
        Ok(PathDecl {
            cond: None,
            ifnone: false,
            edge,
            src,
            full,
            polarity,
            dst,
            data,
            delays,
        })
    }

    fn tc_arg(&mut self) -> R<TcArg> {
        if self.p(",") || self.p(")") {
            return Ok(TcArg {
                edge: None,
                expr: Expr::Empty,
                cond: None,
            });
        }
        let mut edge = None;
        let t = self.peek();
        if t.k == K::Kw && matches!(t.s, "posedge" | "negedge") {
            self.bump();
            edge = Some(t.s.to_string());
        } else if t.k == K::Kw && t.s == "edge" {
            self.bump();
            self.expect_p("[")?;
            loop {
                let d = self.peek();
                let ok = d.k == K::Num && {
                    let x = self.text(d).to_ascii_lowercase();
                    matches!(
                        x.as_str(),
                        "01" | "10" | "0x" | "1x" | "x0" | "x1" | "0z" | "1z" | "z0" | "z1"
                    )
                };
                if !ok {
                    self.push(Diag::error(
                        "E4100",
                        "an edge descriptor is 01, 10, 0x, 1x, x0, x1 (or z for x) (clause 15.4)",
                        self.span(d),
                    ));
                    return Err(());
                }
                self.bump();
                if !self.eat_p(",") {
                    break;
                }
            }
            if !self.p("]") {
                return self.fail("`]`");
            }
            let close = self.bump();
            let raw = String::from_utf8_lossy(&self.pre.text[t.start as usize..close.end as usize])
                .into_owned();
            edge = Some(raw.split_whitespace().collect());
        }
        let expr = self.mintypmax()?;
        let cond = if self.eat_p("&&&") {
            Some(self.expr()?)
        } else {
            None
        };
        Ok(TcArg { edge, expr, cond })
    }

    // ----- UDP -----

    /// Clauses 4.2.1 and 4.4: charge strength belongs to a trireg without assignment, drive strength to a net with one.
    fn net_strength(
        &mut self,
        kind: NetKind,
        at: Tok,
        strength: &Option<Strength>,
        names: &[Declarator],
    ) -> R<()> {
        let assigned = names.iter().any(|d| d.init.is_some());
        let why = match strength {
            Some(Strength::Charge(_)) if kind != NetKind::Trireg => {
                "a charge strength goes with a trireg net (clause 4.4.1)"
            }
            Some(Strength::Charge(_)) if assigned => {
                "a trireg with a charge strength has no assignment (clause 4.2.1)"
            }
            Some(Strength::Drive(_, "")) => {
                "a net has a drive strength of two strengths (clause 4.4.2)"
            }
            Some(Strength::Drive(..)) if !assigned => {
                "a drive strength goes with a net declaration assignment (clause 4.4.2)"
            }
            _ => return Ok(()),
        };
        self.push(Diag::error("E4100", why, self.span(at)));
        Err(())
    }

    /// Clauses 7.1.2 to 7.8: which gates take a drive strength and how many delays.
    fn gate_spec(
        &mut self,
        t: Tok,
        st: Tok,
        strength: &Option<Strength>,
        dt: Tok,
        delay: &Option<Delay>,
    ) -> R<()> {
        let (clause, drive, max) = match t.s {
            "and" | "nand" | "or" | "nor" | "xor" | "xnor" => ("7.2", true, 2),
            "buf" | "not" => ("7.3", true, 2),
            "bufif0" | "bufif1" | "notif0" | "notif1" => ("7.4", true, 3),
            "cmos" | "rcmos" => ("7.7", false, 3),
            "tranif0" | "tranif1" | "rtranif0" | "rtranif1" => ("7.6", false, 2),
            "tran" | "rtran" => ("7.6", false, 0),
            "pullup" | "pulldown" => ("7.8", true, 0),
            _ => ("7.5", false, 3),
        };
        let pull = matches!(t.s, "pullup" | "pulldown");
        let why = match strength {
            Some(_) if !drive => Some(format!(
                "`{}` takes no drive strength (clause {clause})",
                t.s
            )),
            Some(Strength::Charge(_)) => Some(format!(
                "`{}` takes a drive strength (clause {clause})",
                t.s
            )),
            Some(Strength::Drive(a, "")) if !pull => Some(format!(
                "`{}` takes a drive strength of two strengths, found `({a})` (clause {clause})",
                t.s
            )),
            Some(Strength::Drive(a, ""))
                if !a.ends_with(if t.s == "pullup" { '1' } else { '0' }) =>
            {
                Some(format!(
                    "`{}` takes only a {} strength, found `({a})` (clause {clause})",
                    t.s,
                    if t.s == "pullup" {
                        "strength1"
                    } else {
                        "strength0"
                    }
                ))
            }
            _ => None,
        };
        if let Some(w) = why {
            self.push(Diag::error("E4100", w, self.span(st)));
            return Err(());
        }
        if let Some(d) = delay
            && d.0.len() > max
        {
            let w = match max {
                0 => format!("`{}` takes no delay (clause {clause})", t.s),
                _ => format!(
                    "`{}` takes at most {max} delays, found {} (clause {clause})",
                    t.s,
                    d.0.len()
                ),
            };
            self.push(Diag::error("E4100", w, self.span(dt)));
            return Err(());
        }
        Ok(())
    }

    /// Clause 7.1: the terminals of a gate are positional and their number is fixed by the gate.
    fn gate_terminals(&mut self, t: Tok, insts: &[Instance]) -> R<()> {
        let (lo, hi) = match t.s {
            "and" | "nand" | "or" | "nor" | "xor" | "xnor" | "buf" | "not" => (2, usize::MAX),
            "bufif0" | "bufif1" | "notif0" | "notif1" | "nmos" | "pmos" | "rnmos" | "rpmos"
            | "tranif0" | "tranif1" | "rtranif0" | "rtranif1" => (3, 3),
            "cmos" | "rcmos" => (4, 4),
            "tran" | "rtran" => (2, 2),
            _ => (1, 1), // pullup, pulldown
        };
        for i in insts {
            let ordered = i
                .conns
                .iter()
                .all(|c| matches!(c, Conn::Ordered(e) if !matches!(e, Expr::Empty)));
            let msg = if !ordered {
                format!(
                    "the terminals of a `{}` gate are given by position, none left empty (clause 7.1)",
                    t.s
                )
            } else if !(lo..=hi).contains(&i.conns.len()) {
                let want = if hi == usize::MAX {
                    format!("at least {lo}")
                } else {
                    lo.to_string()
                };
                format!(
                    "a `{}` gate has {want} terminals, found {} (clause 7.1)",
                    t.s,
                    i.conns.len()
                )
            } else {
                continue;
            };
            self.push(Diag::error("E4100", msg, self.span(t)));
            return Err(());
        }
        Ok(())
    }

    fn udp_port_error(&mut self, t: Tok) {
        self.push(Diag::error(
            "E4100",
            "a UDP has one output and it is the first port (clause 8.1)",
            self.span(t),
        ));
    }

    /// The value of a UDP `initial` is 0, 1, 1'b0, 1'b1, 1'bx or 1'bX (clause 8.5).
    fn udp_init(&mut self) -> R<Expr> {
        let t = self.peek();
        let e = self.expr()?;
        let ok = match &e {
            Expr::Num(n) => {
                let n: String = n.chars().filter(|c| !c.is_whitespace()).collect();
                matches!(
                    n.to_ascii_lowercase().as_str(),
                    "0" | "1" | "1'b0" | "1'b1" | "1'bx"
                )
            }
            _ => false,
        };
        if !ok {
            self.push(Diag::error(
                "E4100",
                "the initial value of a UDP is 0, 1, 1'b0, 1'b1 or 1'bx (clause 8.5)",
                self.span(t),
            ));
            return Err(());
        }
        Ok(e)
    }

    fn udp(&mut self) -> R<Udp> {
        self.bump();
        let name = self.ident()?;
        self.expect_p("(")?;
        let mut output: Option<Ident> = None;
        let mut inputs: Vec<Ident> = Vec::new();
        let mut sequential = false;
        let mut init = None;
        let head = self.peek_at(self.attrs_len());
        if head.k == K::Kw && matches!(head.s, "output" | "input") {
            // ANSI style header
            let mut first = true;
            loop {
                self.attrs()?;
                let t = self.peek();
                if (t.k == K::Kw && t.s == "output") != first && (output.is_some() || first) {
                    self.udp_port_error(t);
                    return Err(());
                }
                first = false;
                if t.k == K::Kw && t.s == "output" {
                    self.bump();
                    if self.eat_kw("reg") {
                        sequential = true;
                    }
                    let o = self.ident()?;
                    if self.eat_p("=") {
                        init = Some(self.udp_init()?);
                    }
                    output = Some(o);
                } else if t.k == K::Kw && t.s == "input" {
                    self.bump();
                    inputs.push(self.ident()?);
                } else if self.peek().k == K::Ident {
                    inputs.push(self.ident()?);
                } else {
                    return self.fail("a port");
                }
                if !self.eat_p(",") {
                    break;
                }
            }
            self.expect_p(")")?;
            self.expect_p(";")?;
        } else {
            let mut names = vec![self.ident()?];
            while self.eat_p(",") {
                names.push(self.ident()?);
            }
            self.expect_p(")")?;
            self.expect_p(";")?;
            // declarations
            loop {
                self.attrs()?;
                let t = self.peek();
                match (t.k, t.s) {
                    (K::Kw, "output") => {
                        if output.is_some() {
                            self.udp_port_error(t);
                            return Err(());
                        }
                        self.bump();
                        if self.eat_kw("reg") {
                            sequential = true;
                        }
                        output = Some(self.ident()?);
                        self.expect_p(";")?;
                    }
                    (K::Kw, "input") => {
                        self.bump();
                        inputs.push(self.ident()?);
                        while self.eat_p(",") {
                            inputs.push(self.ident()?);
                        }
                        self.expect_p(";")?;
                    }
                    (K::Kw, "reg") => {
                        self.bump();
                        let r = self.ident()?;
                        self.expect_p(";")?;
                        if output.as_ref().is_some_and(|o| o.name == r.name) || output.is_none() {
                            sequential = true;
                        }
                    }
                    _ => break,
                }
            }
            if output.is_none() {
                return self.fail("`output`");
            }
            // The output is the first name of the header.
            if names[0].name != output.as_ref().map_or("", |o| o.name.as_str()) {
                let t = self.peek();
                self.udp_port_error(t);
                return Err(());
            }
        }
        if self.kw("initial") {
            self.bump();
            let target = self.ident()?;
            if output.as_ref().is_some_and(|o| o.name != target.name) {
                self.push(Diag::error(
                    "E4100",
                    "the initial statement of a UDP assigns its output (clause 8.5)",
                    self.pre
                        .span(target.pos, target.pos + target.name.len() as u32),
                ));
                return Err(());
            }
            self.expect_p("=")?;
            init = Some(self.udp_init()?);
            self.expect_p(";")?;
            sequential = true;
        }
        let Some(output) = output else {
            return self.fail("`output`");
        };
        if inputs.is_empty() {
            self.push(Diag::error(
                "E4100",
                "a UDP has at least one input (clause 8.1.1)",
                self.pre.span(name.pos, name.pos + name.name.len() as u32),
            ));
            return Err(());
        }
        let table = self.expect_kw("table")?;
        // The table body is read as text: its symbols are not Verilog tokens.
        let mut end = None;
        while self.peek().k != K::Eof {
            if self.kw("endtable") {
                end = Some(self.peek());
                break;
            }
            self.bump();
        }
        let Some(end) = end else {
            return self.fail("`endtable`");
        };
        let body = String::from_utf8_lossy(&self.pre.text[table.end as usize..end.start as usize])
            .into_owned();
        let rows = self.udp_rows(&body, table.end, sequential, inputs.len())?;
        self.bump();
        if !self.kw("endprimitive") {
            return self.fail("`endprimitive`");
        }
        self.bump();
        if self.p(":") {
            return self.sv_end_label();
        }
        Ok(Udp {
            name,
            output,
            inputs,
            sequential,
            init,
            rows,
        })
    }

    fn udp_rows(&mut self, body: &str, base: u32, sequential: bool, n_in: usize) -> R<Vec<UdpRow>> {
        let mut rows = Vec::new();
        let mut off = 0usize;
        for row in body.split_inclusive(';') {
            let row_off = off;
            off += row.len();
            let Some(row) = row.strip_suffix(';') else {
                if row.trim().is_empty() {
                    continue;
                }
                let at = base + (row_off + row.len()) as u32;
                self.push(Diag::error(
                    "E4100",
                    "UDP table row is not terminated by `;`",
                    self.pre.span(at, at + 1),
                ));
                return Err(());
            };
            let bad = |s: &mut Self, msg: String| {
                let at = base + row_off as u32;
                s.push(Diag::error(
                    "E4100",
                    msg,
                    s.pre.span(at, at + row.trim().len().max(1) as u32),
                ));
            };
            let fields: Vec<&str> = row.split(':').collect();
            let want = if sequential { 3 } else { 2 };
            if fields.len() != want {
                bad(
                    self,
                    format!(
                        "a UDP table row of a {} primitive has {want} fields separated by `:`",
                        if sequential {
                            "sequential"
                        } else {
                            "combinational"
                        }
                    ),
                );
                return Err(());
            }
            let syms = |s: &str| -> Option<Vec<UdpSym>> {
                let cs: Vec<char> = s.chars().filter(|c| !c.is_whitespace()).collect();
                let mut v = Vec::new();
                let mut k = 0;
                while k < cs.len() {
                    if cs[k] == '(' {
                        if k + 3 < cs.len() && cs[k + 3] == ')' {
                            v.push(UdpSym::Edge(cs[k + 1], cs[k + 2]));
                            k += 4;
                        } else {
                            return None;
                        }
                    } else {
                        v.push(UdpSym::Level(cs[k]));
                        k += 1;
                    }
                }
                Some(v)
            };
            // Table 8-1: edges and `-` only where the table allows them.
            let valid = |s: &UdpSym| match s {
                UdpSym::Level(c) => {
                    "01xXbB?".contains(*c) || (sequential && "rRfFpPnN*".contains(*c))
                }
                UdpSym::Edge(a, b) => {
                    sequential && "01xX?bB".contains(*a) && "01xX?bB".contains(*b)
                }
            };
            let Some(inputs) = syms(fields[0]) else {
                bad(self, "malformed edge `(ab)` in a UDP table row".into());
                return Err(());
            };
            let Some(out) = syms(fields[want - 1]) else {
                bad(self, "malformed UDP table output".into());
                return Err(());
            };
            let state = if sequential {
                match syms(fields[1]) {
                    Some(mut s) if s.len() == 1 => s.pop(),
                    _ => {
                        bad(
                            self,
                            "the current state of a UDP table row is one symbol".into(),
                        );
                        return Err(());
                    }
                }
            } else {
                None
            };
            let edges = inputs
                .iter()
                .filter(|s| {
                    matches!(s, UdpSym::Edge(..))
                        || matches!(s, UdpSym::Level(c) if "rRfFpPnN*".contains(*c))
                })
                .count();
            if edges > 1 {
                bad(
                    self,
                    "a UDP table row has at most one edge (clause 8.4)".into(),
                );
                return Err(());
            }
            if inputs.len() != n_in || out.len() != 1 {
                bad(
                    self,
                    format!("a UDP table row needs {n_in} input symbol(s) and one output"),
                );
                return Err(());
            }
            let out_ok = matches!(&out[0], UdpSym::Level(c) if "01xX".contains(*c) || (sequential && *c == '-'));
            let state_ok = state
                .as_ref()
                .is_none_or(|s| matches!(s, UdpSym::Level(c) if "01xXbB?".contains(*c)));
            if !inputs.iter().all(valid) || !out_ok || !state_ok {
                bad(
                    self,
                    "a symbol of a UDP table row is not allowed in its field: inputs take 0 1 x ? b (and edges in a sequential UDP), the current state 0 1 x ? b, the output 0 1 x (and - in a sequential UDP) (clause 8.1.6)".into(),
                );
                return Err(());
            }
            rows.push(UdpRow {
                inputs,
                state,
                output: out.into_iter().next().unwrap(),
            });
        }
        Ok(rows)
    }
}
