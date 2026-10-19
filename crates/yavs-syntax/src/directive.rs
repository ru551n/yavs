//! Conditional analysis (LRM 2019 clause 24) and other tool directives,
//! evaluated on the token stream before parsing.
//!
//! `` `if ``, `` `elsif ``, `` `else `` and `` `end [if] `` keep or drop the
//! tokens they enclose, `` `warning `` and `` `error `` report a message, and
//! any other directive is ignored with a warning up to the end of its line.

use yavs_diag::{Diagnostic, FileId, Span};

use crate::Std;
use crate::codes;
use crate::source::Src;
use crate::token::{Kw, Tok, Token};

/// Values of the standard conditional analysis identifiers, plus user
/// definitions (which win).
fn lookup(name: &str, std: Std, defines: &[(String, String)]) -> Option<String> {
    if let Some((_, v)) = defines.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)) {
        return Some(v.clone());
    }
    let v = match name.to_ascii_uppercase().as_str() {
        "VHDL_VERSION" => std.as_str(),
        "TOOL_TYPE" => "SIMULATION",
        "TOOL_VENDOR" => "yavs",
        "TOOL_NAME" => "yavs",
        "TOOL_EDITION" => "",
        "TOOL_VERSION" => env!("CARGO_PKG_VERSION"),
        _ => return None,
    };
    Some(v.to_string())
}

struct Frame {
    open: Span,
    /// Tokens in the current branch are kept.
    active: bool,
    /// The enclosing region is active.
    parent: bool,
    /// Some branch has been taken already.
    taken: bool,
    seen_else: bool,
}

pub(crate) fn preprocess(
    file: FileId,
    src: Src,
    toks: Vec<Token>,
    std: Std,
    defines: &[(String, String)],
    diags: &mut Vec<Diagnostic>,
) -> Vec<Token> {
    if !toks.iter().any(|t| t.kind == Tok::Directive) {
        return toks;
    }
    let mut p = Pre {
        file,
        src,
        toks: &toks,
        std,
        defines,
        diags,
        i: 0,
    };
    let mut out = Vec::with_capacity(toks.len());
    let mut stack: Vec<Frame> = Vec::new();
    loop {
        let t = toks[p.i];
        let active = stack.last().is_none_or(|f| f.active);
        if t.kind == Tok::Eof {
            if let Some(f) = stack.last() {
                p.diags.push(
                    Diagnostic::error(codes::DIRECTIVE, "unterminated `if directive")
                        .with_primary(Span::new(file, t.start, t.end), "expected `end if")
                        .with_secondary(f.open, "this `if is never closed"),
                );
            }
            out.push(t);
            return out;
        }
        if t.kind != Tok::Directive {
            if active {
                out.push(t);
            }
            p.i += 1;
            continue;
        }
        let span = Span::new(file, t.start, t.end);
        let name = src.text(t.start + 1, t.end).to_ascii_lowercase();
        p.i += 1;
        match name.as_str() {
            "if" => {
                let v = p.condition(active);
                stack.push(Frame {
                    open: span,
                    active: active && v,
                    parent: active,
                    taken: v,
                    seen_else: false,
                });
            }
            "elsif" | "else" => {
                let parent = stack.last().is_none_or(|f| f.parent);
                let v = name == "else" || p.condition(parent);
                match stack.last_mut() {
                    Some(f) if !f.seen_else => {
                        f.active = f.parent && !f.taken && v;
                        f.taken |= v;
                        f.seen_else = name == "else";
                    }
                    Some(_) => p.error(span, &format!("`{name} after `else")),
                    None => p.error(span, &format!("`{name} without `if")),
                }
            }
            "end" => {
                if p.same_line() && toks[p.i].kind == Tok::Kw(Kw::If) {
                    p.i += 1;
                }
                if stack.pop().is_none() {
                    p.error(span, "`end without `if");
                }
            }
            "warning" | "error" => {
                let msg = if p.same_line() && toks[p.i].kind == Tok::Str {
                    let s = toks[p.i];
                    p.i += 1;
                    unquote(&src.text(s.start, s.end))
                } else {
                    String::new()
                };
                if active {
                    let d = if name == "error" {
                        Diagnostic::error(codes::USER_ERROR, msg)
                    } else {
                        Diagnostic::warning(codes::USER_WARNING, msg)
                    };
                    p.diags.push(d.with_primary(span, ""));
                }
            }
            _ => {
                if active {
                    p.diags.push(
                        Diagnostic::warning(
                            codes::UNKNOWN_DIRECTIVE,
                            format!("unknown tool directive `{name}"),
                        )
                        .with_primary(span, "ignored up to the end of the line"),
                    );
                }
                while p.same_line() {
                    p.i += 1;
                }
            }
        }
    }
}

/// Text of a string literal without its quotes, with doubled quotes undone.
pub(crate) fn unquote(s: &str) -> String {
    let q = &s[..1];
    let inner = s.get(1..s.len().saturating_sub(1)).unwrap_or("");
    inner.replace(&format!("{q}{q}"), q)
}

enum Val {
    Bool(bool),
    Str(String),
}

struct Pre<'a> {
    file: FileId,
    src: Src<'a>,
    toks: &'a [Token],
    std: Std,
    defines: &'a [(String, String)],
    diags: &'a mut Vec<Diagnostic>,
    i: usize,
}

impl Pre<'_> {
    fn error(&mut self, span: Span, msg: &str) {
        self.diags
            .push(Diagnostic::error(codes::DIRECTIVE, msg).with_primary(span, ""));
    }

    fn span(&self) -> Span {
        let t = self.toks[self.i];
        Span::new(self.file, t.start, t.end)
    }

    /// The current token is on the same line as the previous one.
    fn same_line(&self) -> bool {
        let t = self.toks[self.i];
        let prev = self.toks[self.i - 1];
        t.kind != Tok::Eof && !self.src.has_newline(prev.end, t.start)
    }

    /// Parse `expr then`, returning its value; on error report (when `report`)
    /// and skip to `then`.
    fn condition(&mut self, report: bool) -> bool {
        let start = self.i;
        let mut msgs = Vec::new();
        let v = match self.expr(&mut msgs) {
            Some(Val::Bool(b)) if self.toks[self.i].kind == Tok::Kw(Kw::Then) => Some(b),
            Some(Val::Bool(_)) => {
                msgs.push((self.span(), "expected `then`".to_string()));
                None
            }
            Some(Val::Str(_)) => {
                msgs.push((
                    self.span(),
                    "condition is a string, not a boolean".to_string(),
                ));
                None
            }
            None => None,
        };
        if v.is_none() {
            self.i = start;
            while !matches!(
                self.toks[self.i].kind,
                Tok::Kw(Kw::Then) | Tok::Eof | Tok::Directive
            ) {
                self.i += 1;
            }
            if report && let Some((span, msg)) = msgs.into_iter().next() {
                self.diags.push(
                    Diagnostic::error(codes::DIRECTIVE, "invalid conditional analysis expression")
                        .with_primary(span, msg),
                );
            }
        }
        if self.toks[self.i].kind == Tok::Kw(Kw::Then) {
            self.i += 1;
        }
        v.unwrap_or(false)
    }

    fn expr(&mut self, msgs: &mut Vec<(Span, String)>) -> Option<Val> {
        let mut l = self.relation(msgs)?;
        loop {
            let op = match self.toks[self.i].kind {
                Tok::Kw(k @ (Kw::And | Kw::Or | Kw::Xor | Kw::Xnor | Kw::Nand | Kw::Nor)) => k,
                _ => return Some(l),
            };
            let span = self.span();
            self.i += 1;
            let r = self.relation(msgs)?;
            let (Val::Bool(a), Val::Bool(b)) = (l, r) else {
                msgs.push((
                    span,
                    "operands of a logical operator must be boolean".into(),
                ));
                return None;
            };
            l = Val::Bool(match op {
                Kw::And => a && b,
                Kw::Or => a || b,
                Kw::Xor => a != b,
                Kw::Xnor => a == b,
                Kw::Nand => !(a && b),
                _ => !(a || b),
            });
        }
    }

    fn relation(&mut self, msgs: &mut Vec<(Span, String)>) -> Option<Val> {
        let l = self.primary(msgs)?;
        let k = self.toks[self.i].kind;
        if !matches!(k, Tok::Eq | Tok::Ne | Tok::Lt | Tok::Le | Tok::Gt | Tok::Ge) {
            return Some(l);
        }
        let span = self.span();
        self.i += 1;
        let r = self.primary(msgs)?;
        let (Val::Str(a), Val::Str(b)) = (l, r) else {
            msgs.push((span, "relational operators compare strings".into()));
            return None;
        };
        Some(Val::Bool(match k {
            Tok::Eq => a == b,
            Tok::Ne => a != b,
            Tok::Lt => a < b,
            Tok::Le => a <= b,
            Tok::Gt => a > b,
            _ => a >= b,
        }))
    }

    fn primary(&mut self, msgs: &mut Vec<(Span, String)>) -> Option<Val> {
        let t = self.toks[self.i];
        let span = self.span();
        let text = &*self.src.text(t.start, t.end);
        self.i += 1;
        match t.kind {
            Tok::Kw(Kw::Not) => match self.primary(msgs)? {
                Val::Bool(b) => Some(Val::Bool(!b)),
                Val::Str(_) => {
                    msgs.push((span, "operand of `not` must be boolean".into()));
                    None
                }
            },
            Tok::LParen => {
                let v = self.expr(msgs)?;
                if self.toks[self.i].kind != Tok::RParen {
                    msgs.push((self.span(), "expected `)`".into()));
                    return None;
                }
                self.i += 1;
                Some(v)
            }
            Tok::Str => Some(Val::Str(unquote(text))),
            Tok::Ident => match lookup(text, self.std, self.defines) {
                Some(v) => Some(Val::Str(v)),
                None => {
                    msgs.push((
                        span,
                        format!("unknown conditional analysis identifier `{text}`"),
                    ));
                    None
                }
            },
            _ => {
                self.i -= 1;
                msgs.push((
                    span,
                    format!("expected identifier or string, found {}", t.kind.describe()),
                ));
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::lex;

    fn run(src: &str, std: Std) -> (String, Vec<Diagnostic>) {
        let mut d = Vec::new();
        let s = Src {
            bytes: src.as_bytes(),
            latin1: false,
        };
        let toks = lex(FileId(0), s, std, false, &mut d);
        let defs = vec![("USER".to_string(), "on".to_string())];
        let toks = preprocess(FileId(0), s, toks, std, &defs, &mut d);
        let text = toks
            .iter()
            .filter(|t| t.kind != Tok::Eof)
            .map(|t| &src[t.start as usize..t.end as usize])
            .collect::<Vec<_>>()
            .join(" ");
        (text, d)
    }

    #[test]
    fn branches() {
        let src = "a `if VHDL_VERSION >= \"2019\" then b `elsif TOOL_NAME = \"x\" then c `else d `end if e";
        assert_eq!(run(src, Std::Vhdl2019).0, "a b e");
        assert_eq!(run(src, Std::Vhdl2008).0, "a d e");
    }

    #[test]
    fn nested_and_logic() {
        let src = "`if not (USER = \"on\" and TOOL_TYPE /= \"SIMULATION\") then `if USER = \"off\" then x `else y `end z `end";
        assert_eq!(run(src, Std::Vhdl2019).0, "y z");
    }

    #[test]
    fn diagnostics() {
        let (text, d) = run(
            "`warning \"hi\"\n`foo bar baz\nq `if NOPE = \"1\" then r `end",
            Std::Vhdl2019,
        );
        assert_eq!(text, "q");
        let msgs: Vec<_> = d.iter().map(|d| d.message.as_str()).collect();
        assert_eq!(
            msgs,
            [
                "hi",
                "unknown tool directive `foo",
                "invalid conditional analysis expression"
            ]
        );
        let (_, d) = run("`if TOOL_NAME = \"yavs\" then x", Std::Vhdl2019);
        assert_eq!(d[0].message, "unterminated `if directive");
    }
}
