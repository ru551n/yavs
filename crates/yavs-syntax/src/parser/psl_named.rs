//! Named PSL sequences and properties (#445): `sequence s (boolean x; const n) is {x[*n]; b};`,
//! `property p (boolean x) is always (x -> next b);`. A declaration is remembered as the range
//! of its body tokens; a use parses that range again with the formals bound, so nothing is
//! copied and the tree needs no clone:
//!
//! - a `boolean` formal becomes a process variable set from the actual at every clock tick
//!   (`Parser::psl_args`), and the name in the body is renamed to it;
//! - a `const` formal is a number (a count, or a literal in an expression);
//! - a `sequence` formal is the token range of the actual, parsed where the formal is used,
//!   with the environment of the use that bound it.
//!
//! Anything else (a `property` formal, a typed formal such as `bit` or `numeric`, a name used
//! before its declaration) leaves the declaration, or the use, to the skip with `W0004`.

use super::psl::Gen;
use super::psl_seq::{Leaves, Sere};
use super::{PResult, Parser, Recover};
use crate::Symbol;
use crate::ast::*;
use crate::token::{Kw, Tok};

/// Most expansions (uses of named sequences and properties) in one directive.
const MAX_EXPANSIONS: u32 = 200;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum DeclKind {
    Sequence,
    Property,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ParamKind {
    Boolean,
    Const,
    Sequence,
    Property,
}

/// A declared sequence or property: the tokens `body.0..body.1` are its body (up to, not
/// including, the `;`).
pub(super) struct PslDecl {
    pub kind: DeclKind,
    name: Symbol,
    params: Vec<(Symbol, ParamKind)>,
    body: (usize, usize),
}

impl PslDecl {
    pub(super) fn is(&self, name: Symbol) -> bool {
        self.name == name
    }
}

/// What a formal stands for during an expansion.
#[derive(Clone)]
pub(super) enum Binding {
    /// The process variable holding the actual.
    Boolean(String),
    Const(u32),
    /// The tokens of the actual and the environment of the use that wrote them.
    Sequence(usize, usize, Env),
    /// The same for a `property` formal.
    Property(usize, usize, Env),
}

pub(super) type Env = Vec<(Symbol, Binding)>;

/// The boolean actuals of the expansions of one directive: variable name and expression.
pub(super) type Args = Vec<(String, Expr)>;

impl Parser<'_> {
    /// The `property` or `sequence` declaration at the current keyword. `true` when it was
    /// recorded (and consumed through its `;`); `false`, with nothing consumed, when it is
    /// outside the supported forms.
    pub(super) fn psl_decl(&mut self) -> bool {
        let kind = match self.peek() {
            Tok::Kw(Kw::Property) => DeclKind::Property,
            Tok::Kw(Kw::Sequence) => DeclKind::Sequence,
            _ => return false,
        };
        let from = self.pos;
        let ok = self.psl_decl_inner(kind);
        if !ok {
            self.pos = from;
        }
        ok
    }

    fn psl_decl_inner(&mut self, kind: DeclKind) -> bool {
        self.bump();
        if self.peek() != Tok::Ident {
            return false;
        }
        let name = self.make_ident(self.toks[self.pos]).key;
        self.bump();
        let mut params = Vec::new();
        if self.eat(Tok::LParen) {
            loop {
                let Some(group) = self.psl_param_kind() else {
                    return false;
                };
                loop {
                    if self.peek() != Tok::Ident {
                        return false;
                    }
                    params.push((self.make_ident(self.toks[self.pos]).key, group));
                    self.bump();
                    if !self.eat(Tok::Comma) {
                        break;
                    }
                    // Tolerated: `(boolean x, sequence s)` with a comma between groups.
                    if self.peek() != Tok::Ident || self.psl_param_kind_at(0).is_some() {
                        break;
                    }
                }
                if !(self.eat(Tok::Semi) || self.toks[self.pos - 1].kind == Tok::Comma) {
                    break;
                }
            }
            if !self.eat(Tok::RParen) {
                return false;
            }
        }
        if !self.eat_kw(Kw::Is) {
            return false;
        }
        let start = self.pos;
        let mut depth = 0u32;
        loop {
            match self.toks[self.pos].kind {
                Tok::Eof | Tok::Kw(Kw::End | Kw::Begin) => return false,
                Tok::LParen | Tok::LBracket | Tok::LBrace => depth += 1,
                Tok::RParen | Tok::RBracket | Tok::RBrace => depth = depth.saturating_sub(1),
                Tok::Semi if depth == 0 => break,
                _ => {}
            }
            self.pos += 1;
        }
        let end = self.pos;
        if end == start
            || self.psl_foreign_range(start, end, true, self.psl_has_temporal(start, end))
        {
            return false;
        }
        self.bump();
        self.psl_decls.push(PslDecl {
            kind,
            name,
            params,
            body: (start, end),
        });
        true
    }

    /// `boolean`, `const` or `sequence` before a group of formals.
    fn psl_param_kind(&mut self) -> Option<ParamKind> {
        let kind = self.psl_param_kind_at(0)?;
        self.bump();
        Some(kind)
    }

    fn psl_param_kind_at(&self, i: usize) -> Option<ParamKind> {
        match self.psl_word(i).as_deref() {
            Some("boolean") => Some(ParamKind::Boolean),
            Some("const") => Some(ParamKind::Const),
            None if self.nth(i) == Tok::Kw(Kw::Sequence) => Some(ParamKind::Sequence),
            None if self.nth(i) == Tok::Kw(Kw::Property) => Some(ParamKind::Property),
            _ => None,
        }
    }

    /// The declaration of `kind` that the identifier `i` tokens ahead names, unless a
    /// formal of an expansion in progress has that name.
    pub(super) fn psl_named_at(&self, i: usize, kind: DeclKind) -> Option<usize> {
        if self.nth(i) != Tok::Ident {
            return None;
        }
        let key = self
            .make_ident(self.toks[(self.pos + i).min(self.eof_index())])
            .key;
        if self.psl_env.iter().any(|(n, _)| *n == key) {
            return None;
        }
        self.psl_decls
            .iter()
            .rposition(|d| d.name == key && d.kind == kind)
    }

    pub(super) fn psl_named(&self, kind: DeclKind) -> Option<usize> {
        self.psl_named_at(0, kind)
    }

    /// The formal of an expansion in progress that the identifier `i` tokens ahead names.
    fn psl_formal_at(&self, i: usize) -> Option<&Binding> {
        if self.nth(i) != Tok::Ident || self.psl_env.is_empty() {
            return None;
        }
        let key = self
            .make_ident(self.toks[(self.pos + i).min(self.eof_index())])
            .key;
        self.psl_env.iter().rfind(|(n, _)| *n == key).map(|b| &b.1)
    }

    pub(super) fn psl_formal_is_sequence(&self, i: usize) -> bool {
        matches!(self.psl_formal_at(i), Some(Binding::Sequence(..)))
    }

    /// What the formals stand for, to tell expressions with the same text apart.
    pub(super) fn psl_env_key(&self) -> String {
        let mut k = String::new();
        for (n, b) in &self.psl_env {
            match b {
                Binding::Boolean(v) => k.push_str(&format!("{}={v};", n.as_str())),
                Binding::Const(c) => k.push_str(&format!("{}={c};", n.as_str())),
                Binding::Sequence(..) | Binding::Property(..) => {}
            }
        }
        k
    }

    /// The actuals `(a, b)` after a name, as token ranges; none without parentheses.
    fn psl_actuals(&mut self) -> PResult<Vec<(usize, usize)>> {
        let mut v = Vec::new();
        if !self.eat(Tok::LParen) {
            return Ok(v);
        }
        loop {
            let start = self.pos;
            let mut depth = 0u32;
            loop {
                match self.toks[self.pos].kind {
                    Tok::Eof => return Err(Recover),
                    Tok::Semi if depth == 0 => return Err(Recover),
                    Tok::LParen | Tok::LBracket | Tok::LBrace => depth += 1,
                    Tok::RParen | Tok::RBracket | Tok::RBrace if depth > 0 => depth -= 1,
                    Tok::RParen | Tok::Comma if depth == 0 => break,
                    _ => {}
                }
                self.pos += 1;
            }
            v.push((start, self.pos));
            if !self.eat(Tok::Comma) {
                break;
            }
        }
        self.expect(Tok::RParen)?;
        Ok(v)
    }

    /// Use the declaration `d` named at the current token: bind its formals to the actuals,
    /// run `parse` on its body and come back after the use.
    pub(super) fn psl_expand<T>(
        &mut self,
        d: usize,
        parse: impl FnOnce(&mut Self) -> PResult<T>,
    ) -> PResult<T> {
        self.psl_expansions += 1;
        if self.psl_expansions > MAX_EXPANSIONS {
            return Err(Recover);
        }
        self.bump();
        let actuals = self.psl_actuals()?;
        let after = self.pos;
        let (params, body) = {
            let decl = &self.psl_decls[d];
            (decl.params.clone(), decl.body)
        };
        if actuals.len() != params.len() {
            return Err(Recover);
        }
        let mut env: Env = Vec::new();
        self.psl_scope += 1;
        let scope = self.psl_scope;
        for ((name, kind), (start, end)) in params.into_iter().zip(actuals) {
            self.pos = start;
            let bound = match kind {
                ParamKind::Boolean => {
                    let e = self.psl_expr()?;
                    let var = format!("\\psl arg {scope} {}\\", name.as_str());
                    self.psl_args.push((var.clone(), e));
                    Binding::Boolean(var)
                }
                ParamKind::Const => Binding::Const(self.psl_count()?),
                ParamKind::Sequence => Binding::Sequence(start, end, self.psl_env.clone()),
                ParamKind::Property => Binding::Property(start, end, self.psl_env.clone()),
            };
            if !matches!(kind, ParamKind::Sequence | ParamKind::Property) && self.pos != end {
                return Err(Recover);
            }
            env.push((name, bound));
        }
        let outer = std::mem::replace(&mut self.psl_env, env);
        let outer_end = std::mem::replace(&mut self.psl_end, body.1);
        self.pos = body.0;
        let r = self.nested(parse).and_then(|v| {
            if self.pos == body.1 {
                Ok(v)
            } else {
                Err(Recover)
            }
        });
        self.psl_env = outer;
        self.psl_end = outer_end;
        self.pos = after;
        r
    }

    /// The sequence a `sequence` formal stands for, parsed where it is used (current token).
    pub(super) fn psl_formal_sequence(&mut self, leaves: &mut Leaves) -> PResult<Option<Sere>> {
        let Some(Binding::Sequence(start, end, env)) = self.psl_formal_at(0).cloned() else {
            return Ok(None);
        };
        self.bump();
        let after = self.pos;
        let outer = std::mem::replace(&mut self.psl_env, env);
        let outer_end = std::mem::replace(&mut self.psl_end, end);
        self.pos = start;
        let r = self
            .nested(|p| p.sere_concat(leaves))
            .and_then(|s| if self.pos == end { Ok(s) } else { Err(Recover) });
        self.psl_env = outer;
        self.psl_end = outer_end;
        self.pos = after;
        r.map(Some)
    }

    pub(super) fn psl_formal_is_property(&self, i: usize) -> bool {
        matches!(self.psl_formal_at(i), Some(Binding::Property(..)))
    }

    /// The property a `property` formal stands for, parsed by `parse` where it is used
    /// (current token), with the environment of the use that bound it.
    pub(super) fn psl_formal_property<T>(
        &mut self,
        parse: impl FnOnce(&mut Self) -> PResult<T>,
    ) -> PResult<Option<T>> {
        let Some(Binding::Property(start, end, env)) = self.psl_formal_at(0).cloned() else {
            return Ok(None);
        };
        self.bump();
        let after = self.pos;
        let outer = std::mem::replace(&mut self.psl_env, env);
        let outer_end = std::mem::replace(&mut self.psl_end, end);
        self.pos = start;
        let r = self
            .nested(parse)
            .and_then(|p| if self.pos == end { Ok(p) } else { Err(Recover) });
        self.psl_env = outer;
        self.psl_end = outer_end;
        self.pos = after;
        r.map(Some)
    }

    /// A boolean expression of a PSL directive, with the formals of the expansion in
    /// progress replaced.
    pub(super) fn psl_expr(&mut self) -> PResult<Expr> {
        let mut e = self.expr()?;
        if !self.psl_env.is_empty() || !self.psl_decls.is_empty() {
            self.psl_subst(&mut e)?;
        }
        Ok(e)
    }

    /// A count: a number, or a `const` formal.
    pub(super) fn psl_count(&mut self) -> PResult<u32> {
        if let Some(Binding::Const(n)) = self.psl_formal_at(0) {
            let n = *n;
            self.bump();
            return Ok(n);
        }
        if self.peek() != Tok::Abstract {
            return Err(Recover);
        }
        let t = self.bump();
        self.text(t).parse::<u32>().map_err(|_| Recover)
    }

    /// Rename the boolean formals in `e` to their variables and replace the `const` ones by
    /// their value. An expression form this does not walk (an aggregate, a range argument)
    /// skips the directive.
    fn psl_subst(&self, e: &mut Expr) -> PResult<()> {
        match &mut e.kind {
            ExprKind::Literal(_) => Ok(()),
            ExprKind::Paren(x)
            | ExprKind::Unary { operand: x, .. }
            | ExprKind::Qualified { operand: x, .. } => self.psl_subst(x),
            ExprKind::Binary { left, right, .. } => {
                self.psl_subst(left)?;
                self.psl_subst(right)
            }
            ExprKind::Conditional(alts) => {
                for (v, c) in alts {
                    self.psl_subst(v)?;
                    if let Some(c) = c {
                        self.psl_subst(c)?;
                    }
                }
                Ok(())
            }
            ExprKind::Name(n) => {
                if let NameKind::Simple(id) = &n.kind
                    && let Some((_, b)) = self.psl_env.iter().rfind(|(f, _)| *f == id.key)
                {
                    let g = Gen::new(e.span);
                    *e = match b {
                        Binding::Boolean(var) => g.var(var),
                        Binding::Const(v) => g.int(&v.to_string()),
                        Binding::Sequence(..) | Binding::Property(..) => return Err(Recover),
                    };
                    return Ok(());
                }
                self.psl_subst_name(n)
            }
            // ponytail: other expression forms (aggregates, allocators) are not walked, so a
            // formal inside one would stay a plain name; inside an expansion that skips the
            // directive.
            _ if self.psl_env.is_empty() => Ok(()),
            _ => Err(Recover),
        }
    }

    fn psl_subst_name(&self, n: &mut Name) -> PResult<()> {
        match &mut n.kind {
            // A declared sequence or property is no boolean: skip.
            NameKind::Simple(id) if self.psl_decls.iter().any(|d| d.is(id.key)) => Err(Recover),
            NameKind::Simple(_) | NameKind::Character(_) | NameKind::Operator(_) => Ok(()),
            NameKind::Selected { prefix, .. } | NameKind::Attribute { prefix, .. } => {
                self.psl_subst_name(prefix)
            }
            NameKind::Apply { prefix, args } => {
                self.psl_subst_name(prefix)?;
                for a in args {
                    match &mut a.actual {
                        Actual::Expr(x) => self.psl_subst(x)?,
                        _ if self.psl_env.is_empty() => {}
                        _ => return Err(Recover),
                    }
                }
                Ok(())
            }
            _ if self.psl_env.is_empty() => Ok(()),
            _ => Err(Recover),
        }
    }

    /// `var := false; if actual then var := true;` for every boolean actual of the
    /// expansions, first in the statements run after the clock event.
    pub(super) fn psl_arg_stmts(&mut self, g: &mut Gen) {
        let args = std::mem::take(&mut self.psl_args);
        let mut stmts = Vec::new();
        for (var, e) in args {
            let t = g.simple_subtype("boolean");
            let no = g.var("false");
            g.decl(std::slice::from_ref(&var), t, Some(no));
            stmts.push(g.assign(&var, g.var("false")));
            stmts.push(g.when(e, vec![g.assign(&var, g.var("true"))]));
        }
        g.pre.splice(0..0, stmts);
    }
}
