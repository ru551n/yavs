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
}

impl<'a> Parser<'a> {
    pub(crate) fn new(pre: &'a Pre, toks: &'a [Tok]) -> Parser<'a> {
        Parser {
            pre,
            toks,
            i: 0,
            diags: Vec::new(),
            depth: 0,
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
    pub(crate) fn attrs(&mut self) -> R<()> {
        while self.p("(*") {
            // `@(* )` style star lists are handled by the event control parser, never reach here
            let open = self.bump();
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
                (K::Kw, "module") => {
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
                (K::Kw, "macromodule" | "config") => {
                    self.push(
                        Diag::error("E4101", format!("`{}` is not supported", t.s), self.span(t))
                            .help(if t.s == "config" {
                                "configurations and library maps are outside what yavs reads"
                            } else {
                                "write `module`"
                            }),
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
                    self.push(
                        Diag::error(
                            "E4003",
                            "nested module declaration (SystemVerilog)",
                            self.span(t),
                        )
                        .help("declare the module outside; IEEE 1364-2005 has no nested modules"),
                    );
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
                });
            } else if self.p("{") {
                let e = self.expr()?;
                v.push(NonAnsiPort {
                    name: None,
                    expr: Some(e),
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
                    });
                } else {
                    v.push(NonAnsiPort {
                        name: Some(name),
                        expr: None,
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
        let range = self.opt_range()?;
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
                let range = self.opt_range()?;
                let names = self.declarators(true)?;
                self.expect_p(";")?;
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
                let strength = self.strength()?;
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
                let strength = self.strength()?;
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
                self.bump();
                let mut v = Vec::new();
                while !self.kw("endgenerate") {
                    if self.peek().k == K::Eof {
                        return self.fail("`endgenerate`");
                    }
                    self.item(&mut v)?;
                }
                self.bump();
                out.push(Item::Generate(v));
            }
            (K::Kw, "if" | "for" | "case" | "casex" | "casez") => out.push(self.gen_construct()?),
            // Accepted by the other tools: a generate block that stands alone.
            (K::Kw, "begin") => {
                let b = self.gen_body()?;
                out.push(Item::Generate(b.items));
            }
            (K::Kw, "specify") => out.push(self.specify()?),
            (
                K::Kw,
                "and" | "nand" | "or" | "nor" | "xor" | "xnor" | "buf" | "not" | "bufif0"
                | "bufif1" | "notif0" | "notif1" | "pullup" | "pulldown" | "nmos" | "pmos"
                | "rnmos" | "rpmos" | "cmos" | "rcmos" | "tran" | "rtran" | "tranif0" | "tranif1"
                | "rtranif0" | "rtranif1",
            ) => {
                self.bump();
                let strength = self.strength()?;
                let delay = self.delay()?;
                let mut insts = Vec::new();
                loop {
                    insts.push(self.instance(false)?);
                    if !self.eat_p(",") {
                        break;
                    }
                }
                self.expect_p(";")?;
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
                let mut params = Vec::new();
                if self.eat_p("#") {
                    if self.eat_p("(") {
                        if !self.p(")") {
                            params = self.conns()?;
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
            self.conns()?
        };
        self.expect_p(")")?;
        Ok(Instance { name, range, conns })
    }

    /// Ordered or named connections up to the closing `)`.
    fn conns(&mut self) -> R<Vec<Conn>> {
        let mut v = Vec::new();
        loop {
            self.attrs()?;
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
    pub(crate) fn block_decl(&mut self, out: &mut Vec<Item>) -> R<bool> {
        let t = self.peek();
        if t.k == K::Kw
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
            self.item(out)?;
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

    fn task(&mut self) -> R<Task> {
        self.bump();
        let automatic = self.eat_kw("automatic");
        let name = self.ident()?;
        let mut decls = Vec::new();
        if self.p("(") {
            self.tf_ports(&mut decls)?;
        }
        self.expect_p(";")?;
        while self.block_decl(&mut decls)? {}
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
        if self.p("(") {
            self.tf_ports(&mut decls)?;
        }
        self.expect_p(";")?;
        while self.block_decl(&mut decls)? {}
        let mut body = Vec::new();
        while !self.kw("endfunction") {
            if self.peek().k == K::Eof {
                return self.fail("`endfunction`");
            }
            body.push(self.stmt()?);
        }
        self.bump();
        if self.p(":") {
            return self.sv_end_label();
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
                let s = self.ident()?;
                self.expect_p("=")?;
                let step = self.expr()?;
                self.expect_p(")")?;
                let body = self.gen_body()?;
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
                while !self.kw("endcase") {
                    if self.peek().k == K::Eof {
                        return self.fail("`endcase`");
                    }
                    let mut labels = Vec::new();
                    if self.eat_kw("default") {
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
                    let mut outputs = vec![self.terminal()?];
                    while self.eat_p(",") {
                        outputs.push(self.terminal()?);
                    }
                    self.expect_p(";")?;
                    v.push(SpecItem::Pulse { kind: t.s, outputs });
                }
                (K::Sys, _) => {
                    self.bump();
                    let name = self.text(t);
                    self.expect_p("(")?;
                    let mut args = Vec::new();
                    loop {
                        args.push(self.tc_arg()?);
                        if !self.eat_p(",") {
                            break;
                        }
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
                    p.ifnone = true;
                    v.push(SpecItem::Path(p));
                }
                (K::P, "(") => v.push(SpecItem::Path(self.path_decl()?)),
                _ => return self.fail("a specify item"),
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
        self.expect_p("=")?;
        let mut delays = Vec::new();
        if self.eat_p("(") {
            delays.push(self.mintypmax()?);
            while self.eat_p(",") {
                delays.push(self.mintypmax()?);
            }
            self.expect_p(")")?;
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
            while !self.p("]") {
                if self.peek().k == K::Eof {
                    return self.fail("`]`");
                }
                self.bump();
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

    fn udp(&mut self) -> R<Udp> {
        self.bump();
        let name = self.ident()?;
        self.expect_p("(")?;
        let mut output: Option<Ident> = None;
        let mut inputs: Vec<Ident> = Vec::new();
        let mut sequential = false;
        let mut init = None;
        if self.kw("output") || self.kw("input") {
            // ANSI style header
            loop {
                let t = self.peek();
                if t.k == K::Kw && t.s == "output" {
                    self.bump();
                    if self.eat_kw("reg") {
                        sequential = true;
                    }
                    let o = self.ident()?;
                    if self.eat_p("=") {
                        init = Some(self.expr()?);
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
            // header order: output first
            let _ = names;
        }
        if self.kw("initial") {
            self.bump();
            self.ident()?;
            self.expect_p("=")?;
            init = Some(self.expr()?);
            self.expect_p(";")?;
            sequential = true;
        }
        let Some(output) = output else {
            return self.fail("`output`");
        };
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
            let valid = |s: &UdpSym| match s {
                UdpSym::Level(c) => "01xXbB?-rRfFpPnN*".contains(*c),
                UdpSym::Edge(a, b) => "01xX?bB".contains(*a) && "01xX?bB".contains(*b),
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
            if inputs.len() != n_in || out.len() != 1 || !inputs.iter().all(valid) {
                bad(
                    self,
                    format!("a UDP table row needs {n_in} input symbol(s) and one valid output"),
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
