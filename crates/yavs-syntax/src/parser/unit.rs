//! Design files, context clauses and library units.

use super::{PResult, Parser, Recover};
use crate::ast::*;
use crate::token::{Kw, Tok};

fn is_unit_start(t: Tok) -> bool {
    matches!(
        t,
        Tok::Kw(
            Kw::Entity
                | Kw::Architecture
                | Kw::Package
                | Kw::Configuration
                | Kw::Context
                | Kw::Library
                | Kw::Use
        )
    )
}

impl Parser<'_> {
    pub(super) fn design_file(&mut self) -> DesignFile {
        let mut units = Vec::new();
        while !self.at(Tok::Eof) {
            let before = self.pos;
            match self.design_unit() {
                Ok(Some(u)) => units.push(u),
                Ok(None) => {}
                Err(Recover) => self.sync_unit(),
            }
            if self.pos == before {
                self.bump();
            }
        }
        DesignFile { units }
    }

    /// Skip to the start of the next design unit: a unit keyword after `;`.
    fn sync_unit(&mut self) {
        loop {
            let t = self.peek();
            if t == Tok::Eof {
                return;
            }
            let prev_semi = self.pos > 0 && self.toks[self.pos - 1].kind == Tok::Semi;
            if prev_semi && is_unit_start(t) {
                return;
            }
            self.bump();
        }
    }

    fn design_unit(&mut self) -> PResult<Option<DesignUnit>> {
        let start = self.span();
        self.psl_warned = false;
        self.psl_clock = None;
        self.psl_decls.clear();
        let mut context = Vec::new();
        loop {
            match self.peek() {
                Tok::Kw(Kw::Library) => {
                    let s = self.bump_span();
                    let names = self.ident_list()?;
                    self.semi()?;
                    context.push(ContextItem::Library {
                        names,
                        span: self.span_from(s),
                    });
                }
                Tok::Kw(Kw::Use) => context.push(ContextItem::Use(self.use_clause()?)),
                Tok::Kw(Kw::Context) if self.nth(2) != Tok::Kw(Kw::Is) => {
                    context.push(self.context_reference()?);
                }
                _ => break,
            }
        }
        let unit = match self.peek() {
            Tok::Kw(Kw::Entity) => LibraryUnit::Entity(self.entity()?),
            Tok::Kw(Kw::Architecture) => LibraryUnit::Architecture(self.architecture()?),
            Tok::Kw(Kw::Package) if self.nth(1) == Tok::Kw(Kw::Body) => {
                LibraryUnit::PackageBody(self.package_body()?)
            }
            Tok::Kw(Kw::Package) => self.package_unit()?,
            Tok::Kw(Kw::Configuration) => LibraryUnit::Configuration(self.configuration()?),
            Tok::Kw(Kw::Context) => LibraryUnit::Context(self.context_decl()?),
            Tok::Kw(Kw::Vunit | Kw::Vmode | Kw::Vprop) => {
                self.psl_skip_unit();
                return Ok(None);
            }
            Tok::Eof if context.is_empty() => return Ok(None),
            _ => return Err(self.expected("design unit")),
        };
        Ok(Some(DesignUnit {
            context,
            unit,
            span: self.span_from(start),
        }))
    }

    /// Skip a PSL verification unit up to its closing `}`.
    fn psl_skip_unit(&mut self) {
        self.psl_warn();
        let mut depth = 0u32;
        loop {
            match self.peek() {
                Tok::Eof => return,
                Tok::LBrace => depth += 1,
                Tok::RBrace => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        self.bump();
                        return;
                    }
                }
                _ => {}
            }
            self.bump();
        }
    }

    fn context_reference(&mut self) -> PResult<ContextItem> {
        let start = self.expect_kw(Kw::Context)?;
        let mut names = vec![self.name()?];
        while self.eat(Tok::Comma) {
            names.push(self.name()?);
        }
        self.semi()?;
        Ok(ContextItem::Context {
            names,
            span: self.span_from(start),
        })
    }

    fn context_decl(&mut self) -> PResult<ContextDecl> {
        let start = self.expect_kw(Kw::Context)?;
        let name = self.ident()?;
        self.expect_kw(Kw::Is)?;
        let mut items = Vec::new();
        loop {
            match self.peek() {
                Tok::Kw(Kw::Library) => {
                    let s = self.bump_span();
                    let names = self.ident_list()?;
                    self.semi()?;
                    items.push(ContextItem::Library {
                        names,
                        span: self.span_from(s),
                    });
                }
                Tok::Kw(Kw::Use) => items.push(ContextItem::Use(self.use_clause()?)),
                Tok::Kw(Kw::Context) => items.push(self.context_reference()?),
                _ => break,
            }
        }
        let end_name = self.end(start, "context", &[Kw::Context], true, Some(&name))?;
        Ok(ContextDecl {
            name,
            items,
            end_name,
            span: self.span_from(start),
        })
    }

    fn entity(&mut self) -> PResult<EntityDecl> {
        let start = self.expect_kw(Kw::Entity)?;
        let name = self.ident()?;
        self.expect_kw(Kw::Is)?;
        let generics = self.opt_interface_clause(Kw::Generic)?;
        let ports = self.opt_interface_clause(Kw::Port)?;
        let decls = self.decls(false);
        let stmts = if self.eat_kw(Kw::Begin) {
            self.conc_stmts()
        } else {
            Vec::new()
        };
        let end_name = self.end(start, "entity", &[Kw::Entity], true, Some(&name))?;
        Ok(EntityDecl {
            name,
            generics,
            ports,
            decls,
            stmts,
            end_name,
            span: self.span_from(start),
        })
    }

    fn architecture(&mut self) -> PResult<ArchitectureBody> {
        let start = self.expect_kw(Kw::Architecture)?;
        let name = self.ident()?;
        self.expect_kw(Kw::Of)?;
        let mut entity = self.ident()?;
        let mut entity_lib = None;
        if self.eat(Tok::Dot) {
            entity_lib = Some(entity);
            entity = self.ident()?;
        }
        self.expect_kw(Kw::Is)?;
        let decls = self.decls(false);
        self.expect_kw(Kw::Begin)?;
        let stmts = self.conc_stmts();
        let end_name = self.end(
            start,
            "architecture",
            &[Kw::Architecture],
            true,
            Some(&name),
        )?;
        Ok(ArchitectureBody {
            name,
            entity_lib,
            entity,
            decls,
            stmts,
            end_name,
            span: self.span_from(start),
        })
    }

    fn configuration(&mut self) -> PResult<ConfigurationDecl> {
        let start = self.expect_kw(Kw::Configuration)?;
        let name = self.ident()?;
        self.expect_kw(Kw::Of)?;
        let entity = self.type_mark()?;
        self.expect_kw(Kw::Is)?;
        let mut decls = Vec::new();
        while matches!(self.peek(), Tok::Kw(Kw::Use | Kw::Attribute | Kw::Group)) {
            let before = self.pos;
            match self.decl() {
                Ok(d) => decls.extend(d),
                Err(Recover) => {
                    self.sync();
                    if self.pos == before {
                        self.bump();
                    }
                }
            }
        }
        let block = if self.at_kw(Kw::For) {
            Some(self.block_config()?)
        } else {
            None
        };
        let end_name = self.end(
            start,
            "configuration",
            &[Kw::Configuration],
            true,
            Some(&name),
        )?;
        Ok(ConfigurationDecl {
            name,
            entity,
            decls,
            block,
            end_name,
            span: self.span_from(start),
        })
    }

    fn block_config(&mut self) -> PResult<BlockConfig> {
        let start = self.expect_kw(Kw::For)?;
        let spec = self.name()?;
        let mut uses = Vec::new();
        while self.at_kw(Kw::Use) {
            uses.push(self.use_clause()?);
        }
        let mut items = Vec::new();
        while self.at_kw(Kw::For) {
            if self.at_component_spec() {
                items.push(ConfigItem::Component(self.component_config()?));
            } else {
                items.push(ConfigItem::Block(self.block_config()?));
            }
        }
        self.end(start, "block configuration", &[Kw::For], false, None)?;
        Ok(BlockConfig {
            spec,
            uses,
            items,
            span: self.span_from(start),
        })
    }

    /// After `for`: `a, b :`, `all :` or `others :`.
    fn at_component_spec(&self) -> bool {
        let mut i = self.pos + 1;
        loop {
            match self.toks[i].kind {
                Tok::Kw(Kw::All | Kw::Others) | Tok::Ident | Tok::ExtIdent => i += 1,
                _ => return false,
            }
            match self.toks[i].kind {
                Tok::Colon => return true,
                Tok::Comma => i += 1,
                _ => return false,
            }
        }
    }

    fn component_config(&mut self) -> PResult<ComponentConfig> {
        let start = self.expect_kw(Kw::For)?;
        let spec = self.component_spec()?;
        let binding = if matches!(self.peek(), Tok::Kw(Kw::Use | Kw::Generic | Kw::Port)) {
            let b = self.binding()?;
            self.semi()?;
            Some(b)
        } else {
            None
        };
        let block = if self.at_kw(Kw::For) {
            Some(self.block_config()?)
        } else {
            None
        };
        self.end(start, "component configuration", &[Kw::For], false, None)?;
        Ok(ComponentConfig {
            spec,
            binding,
            block,
            span: self.span_from(start),
        })
    }
}
