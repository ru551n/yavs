//! Declarations, interface lists, types and subprograms.

use super::{PResult, Parser, Recover};
use crate::Symbol;
use crate::ast::*;
use crate::directive::unquote;
use crate::token::{Kw, Tok};

/// Entity classes allowed in attribute specifications and group templates.
fn is_entity_class(k: Kw) -> bool {
    matches!(
        k,
        Kw::Entity
            | Kw::Architecture
            | Kw::Configuration
            | Kw::Procedure
            | Kw::Function
            | Kw::Package
            | Kw::Type
            | Kw::Subtype
            | Kw::Constant
            | Kw::Signal
            | Kw::Variable
            | Kw::Component
            | Kw::Label
            | Kw::Literal
            | Kw::Units
            | Kw::Group
            | Kw::File
            | Kw::Property
            | Kw::Sequence
            | Kw::View
    )
}

/// An optional generic clause and generic map aspect.
type GenericHeader = (Option<Vec<InterfaceDecl>>, Option<Vec<AssocElem>>);

impl Parser<'_> {
    pub(super) fn at_decl_start(&mut self) -> bool {
        matches!(
            self.peek(),
            Tok::Kw(
                Kw::Type
                    | Kw::Subtype
                    | Kw::Constant
                    | Kw::Signal
                    | Kw::Variable
                    | Kw::Shared
                    | Kw::File
                    | Kw::Alias
                    | Kw::Component
                    | Kw::Attribute
                    | Kw::Function
                    | Kw::Procedure
                    | Kw::Pure
                    | Kw::Impure
                    | Kw::Package
                    | Kw::Use
                    | Kw::Group
                    | Kw::Disconnect
                    | Kw::For
                    | Kw::View
                    | Kw::Private
                    | Kw::Property
                    | Kw::Sequence
                    | Kw::Default
            )
        )
    }

    /// A declarative part. Stops at `begin`, `end` or end of file; any other
    /// token that cannot start a declaration is an error, unless `lenient`,
    /// when it also ends the part (generate bodies without `begin`).
    pub(super) fn decls(&mut self, lenient: bool) -> Vec<Decl> {
        let mark = self.sc.decls.len();
        loop {
            if self.at_decl_start() {
                let before = self.pos;
                match self.decl() {
                    Ok(Some(d)) => self.sc.decls.push(d),
                    Ok(None) => {}
                    Err(Recover) => {
                        self.sync();
                        if self.pos == before {
                            self.bump();
                        }
                    }
                }
            } else if lenient || matches!(self.peek(), Tok::Kw(Kw::Begin | Kw::End) | Tok::Eof) {
                return self.sc.decls.drain(mark..).collect();
            } else {
                let _ = self.expected("declaration");
                let before = self.pos;
                self.sync();
                if self.pos == before {
                    self.bump();
                }
            }
        }
    }

    /// One declaration. `Ok(None)` for skipped PSL.
    pub(super) fn decl(&mut self) -> PResult<Option<Decl>> {
        self.nested(|p| p.decl_inner())
    }

    fn decl_inner(&mut self) -> PResult<Option<Decl>> {
        let start = self.span();
        Ok(Some(match self.peek() {
            Tok::Kw(Kw::Type) => Decl::Type(self.type_decl()?),
            Tok::Kw(Kw::Subtype) => {
                self.bump();
                let name = self.ident()?;
                self.expect_kw(Kw::Is)?;
                let subtype = self.subtype_indication()?;
                self.semi()?;
                Decl::Subtype(SubtypeDecl {
                    name,
                    subtype,
                    span: self.span_from(start),
                })
            }
            Tok::Kw(Kw::Constant | Kw::Signal | Kw::Variable | Kw::Shared | Kw::Private) => {
                Decl::Object(self.object_decl()?)
            }
            Tok::Kw(Kw::File) => Decl::File(self.file_decl()?),
            Tok::Kw(Kw::Alias) => Decl::Alias(self.alias_decl()?),
            Tok::Kw(Kw::Component) => Decl::Component(self.component_decl()?),
            Tok::Kw(Kw::Attribute) => self.attribute_decl()?,
            Tok::Kw(Kw::Function | Kw::Procedure | Kw::Pure | Kw::Impure) => self.subprogram()?,
            Tok::Kw(Kw::Package) => {
                if self.nth(1) == Tok::Kw(Kw::Body) {
                    Decl::PackageBody(self.package_body()?)
                } else {
                    self.package()?
                }
            }
            Tok::Kw(Kw::Use) => Decl::Use(self.use_clause()?),
            Tok::Kw(Kw::Group) => self.group()?,
            Tok::Kw(Kw::Disconnect) => {
                self.bump();
                let signals = self.signal_list()?;
                self.expect(Tok::Colon)?;
                let type_mark = self.type_mark()?;
                self.expect_kw(Kw::After)?;
                let after = self.expr()?;
                self.semi()?;
                Decl::Disconnect(DisconnectSpec {
                    signals,
                    type_mark,
                    after,
                    span: self.span_from(start),
                })
            }
            Tok::Kw(Kw::For) => Decl::ConfigSpec(self.config_spec()?),
            Tok::Kw(Kw::View) => Decl::View(self.view_decl()?),
            Tok::Kw(Kw::Default) if self.psl_default_clock() => return Ok(None),
            Tok::Kw(Kw::Property | Kw::Sequence | Kw::Default) => {
                // PSL `property`, `sequence`, other `default` forms.
                if !self.psl_decl() {
                    if self.at_kw(Kw::Property) || self.at_kw(Kw::Sequence) {
                        self.psl_skipped_name();
                    }
                    self.psl_skip();
                }
                return Ok(None);
            }
            _ => return Err(self.expected("a declaration")),
        }))
    }

    fn signal_list(&mut self) -> PResult<SignalList> {
        if self.eat_kw(Kw::Others) {
            return Ok(SignalList::Others);
        }
        if self.eat_kw(Kw::All) {
            return Ok(SignalList::All);
        }
        let mut v = vec![self.name()?];
        while self.eat(Tok::Comma) {
            v.push(self.name()?);
        }
        Ok(SignalList::Names(v))
    }

    pub(super) fn use_clause(&mut self) -> PResult<UseClause> {
        let start = self.expect_kw(Kw::Use)?;
        let mut names = vec![self.name()?];
        while self.eat(Tok::Comma) {
            names.push(self.name()?);
        }
        self.semi()?;
        Ok(UseClause {
            names,
            span: self.span_from(start),
        })
    }

    fn type_decl(&mut self) -> PResult<TypeDecl> {
        let start = self.expect_kw(Kw::Type)?;
        let name = self.ident()?;
        if self.at(Tok::Semi) {
            self.bump();
            return Ok(TypeDecl {
                name,
                def: None,
                span: self.span_from(start),
            });
        }
        self.expect_kw(Kw::Is)?;
        let def_start = self.span();
        let def = match self.peek() {
            Tok::LParen => {
                self.bump();
                let mut lits = Vec::new();
                loop {
                    if self.at(Tok::Char) {
                        lits.push(EnumLit::Char(self.char_lit()?));
                    } else {
                        lits.push(EnumLit::Ident(self.ident()?));
                    }
                    if !self.eat(Tok::Comma) {
                        break;
                    }
                }
                self.expect(Tok::RParen)?;
                TypeDef::Enum(lits)
            }
            Tok::Kw(Kw::Range) => {
                self.bump();
                let range = self.range()?;
                if self.at_kw(Kw::Units) {
                    let units = self.bump_span();
                    let primary = self.ident()?;
                    self.semi()?;
                    let mut secondary = Vec::new();
                    while self.at_ident() {
                        let id = self.ident()?;
                        self.expect(Tok::Eq)?;
                        let value = self.primary()?;
                        self.semi()?;
                        secondary.push((id, value));
                    }
                    let end_name =
                        self.end(units, "units definition", &[Kw::Units], false, Some(&name))?;
                    return Ok(TypeDecl {
                        name,
                        def: Some(TypeDef::Physical(PhysicalTypeDef {
                            range,
                            primary,
                            secondary,
                            end_name,
                        })),
                        span: self.span_from(start),
                    });
                }
                TypeDef::Range(range)
            }
            Tok::Kw(Kw::Array) => {
                self.bump();
                let indexes = self.array_indexes()?;
                self.expect_kw(Kw::Of)?;
                let element = self.subtype_indication()?;
                TypeDef::Array { indexes, element }
            }
            Tok::Kw(Kw::Record) => {
                self.bump();
                let mut elements = Vec::new();
                while self.at_ident() {
                    let s = self.span();
                    let names = self.ident_list()?;
                    self.expect(Tok::Colon)?;
                    let subtype = self.subtype_indication()?;
                    self.semi()?;
                    elements.push(ElementDecl {
                        names,
                        subtype,
                        span: self.span_from(s),
                    });
                }
                if elements.is_empty() {
                    self.require_2019(def_start, "an empty record");
                }
                let end_name = self.end(def_start, "record", &[Kw::Record], false, Some(&name))?;
                return Ok(TypeDecl {
                    name,
                    def: Some(TypeDef::Record { elements, end_name }),
                    span: self.span_from(start),
                });
            }
            Tok::Kw(Kw::Access) => {
                self.bump();
                TypeDef::Access(self.subtype_indication()?)
            }
            Tok::Kw(Kw::File) => {
                self.bump();
                self.expect_kw(Kw::Of)?;
                TypeDef::File(self.type_mark()?)
            }
            Tok::Kw(Kw::Protected) => {
                self.bump();
                let def = if self.eat_kw(Kw::Body) {
                    let decls = self.decls(false);
                    let end_name = self.end(
                        def_start,
                        "protected type body",
                        &[Kw::Protected, Kw::Body],
                        false,
                        Some(&name),
                    )?;
                    TypeDef::ProtectedBody(ProtectedBody { decls, end_name })
                } else {
                    let (generics, generic_map) = self.generic_header()?;
                    let decls = self.decls(false);
                    let end_name = self.end(
                        def_start,
                        "protected type",
                        &[Kw::Protected],
                        false,
                        Some(&name),
                    )?;
                    TypeDef::Protected(ProtectedDecl {
                        generics,
                        generic_map,
                        decls,
                        end_name,
                    })
                };
                return Ok(TypeDecl {
                    name,
                    def: Some(def),
                    span: self.span_from(start),
                });
            }
            Tok::Kw(Kw::New) => {
                self.bump();
                self.require_2019(def_start, "a protected type instantiation");
                let protected = self.type_mark()?;
                let generic_map = self.opt_map(Kw::Generic)?;
                TypeDef::ProtectedInstance {
                    protected,
                    generic_map,
                }
            }
            _ => return Err(self.expected("type definition")),
        };
        self.semi()?;
        Ok(TypeDecl {
            name,
            def: Some(def),
            span: self.span_from(start),
        })
    }

    /// `generic (...); [generic map (...);]` if present.
    pub(super) fn generic_header(&mut self) -> PResult<GenericHeader> {
        if !self.at_kw(Kw::Generic) {
            return Ok((None, None));
        }
        if self.nth(1) == Tok::Kw(Kw::Map) {
            let m = self.map_aspect(Kw::Generic)?;
            self.semi()?;
            return Ok((None, Some(m)));
        }
        self.bump();
        let generics = self.interface_list()?;
        self.semi()?;
        let map = if self.at_kw(Kw::Generic) {
            let m = self.map_aspect(Kw::Generic)?;
            self.semi()?;
            Some(m)
        } else {
            None
        };
        Ok((Some(generics), map))
    }

    /// `generic map (...)` or `port map (...)`.
    pub(super) fn map_aspect(&mut self, kw: Kw) -> PResult<Vec<AssocElem>> {
        self.expect_kw(kw)?;
        self.expect_kw(Kw::Map)?;
        self.assoc_list()
    }

    pub(super) fn opt_map(&mut self, kw: Kw) -> PResult<Option<Vec<AssocElem>>> {
        if self.at_kw(kw) && self.nth(1) == Tok::Kw(Kw::Map) {
            Ok(Some(self.map_aspect(kw)?))
        } else {
            Ok(None)
        }
    }

    pub(super) fn char_lit(&mut self) -> PResult<CharLit> {
        let span = self.expect(Tok::Char)?;
        let value = self.char_value(span)?;
        Ok(CharLit { value, span })
    }

    fn array_indexes(&mut self) -> PResult<ArrayIndexes> {
        self.expect(Tok::LParen)?;
        // `t range <>` (possibly `pkg.t range <>`) before the first `,` or `)`.
        let mut i = self.pos;
        let unconstrained = loop {
            match self.toks[i].kind {
                Tok::Kw(Kw::Range) => break self.toks[i + 1].kind == Tok::Box,
                Tok::Comma | Tok::RParen | Tok::LParen | Tok::Eof => break false,
                _ => i += 1,
            }
        };
        let r = if unconstrained {
            let mut v = Vec::new();
            loop {
                v.push(self.type_mark()?);
                self.expect_kw(Kw::Range)?;
                self.expect(Tok::Box)?;
                if !self.eat(Tok::Comma) {
                    break;
                }
            }
            ArrayIndexes::Unconstrained(v)
        } else {
            let mut v = vec![self.discrete_range()?];
            while self.eat(Tok::Comma) {
                v.push(self.discrete_range()?);
            }
            ArrayIndexes::Constrained(v)
        };
        self.expect(Tok::RParen)?;
        Ok(r)
    }

    fn object_decl(&mut self) -> PResult<ObjectDecl> {
        let start = self.span();
        let private = self.eat_kw(Kw::Private);
        if private {
            self.require_2019(start, "a private variable");
        }
        let class = match self.bump().kind {
            Tok::Kw(Kw::Constant) => ObjectClass::Constant,
            Tok::Kw(Kw::Signal) => ObjectClass::Signal,
            Tok::Kw(Kw::Shared) => {
                self.expect_kw(Kw::Variable)?;
                ObjectClass::SharedVariable
            }
            Tok::Kw(Kw::Variable) => ObjectClass::Variable,
            _ => {
                self.pos -= 1;
                return Err(self.expected("`variable`"));
            }
        };
        let names = self.ident_list()?;
        self.expect(Tok::Colon)?;
        let subtype = self.subtype_indication()?;
        let signal_kind = if self.eat_kw(Kw::Register) {
            Some(SignalKind::Register)
        } else if self.eat_kw(Kw::Bus) {
            Some(SignalKind::Bus)
        } else {
            None
        };
        let default = if self.eat(Tok::Assign) {
            Some(self.cond_expr()?)
        } else {
            None
        };
        self.semi()?;
        Ok(ObjectDecl {
            class,
            names,
            subtype,
            signal_kind,
            default,
            private,
            span: self.span_from(start),
        })
    }

    fn file_decl(&mut self) -> PResult<FileDecl> {
        let start = self.expect_kw(Kw::File)?;
        let names = self.ident_list()?;
        self.expect(Tok::Colon)?;
        let subtype = self.subtype_indication()?;
        let open_kind = if self.eat_kw(Kw::Open) {
            Some(self.expr()?)
        } else {
            None
        };
        let file_name = if self.eat_kw(Kw::Is) {
            Some(self.expr()?)
        } else {
            None
        };
        self.semi()?;
        Ok(FileDecl {
            names,
            subtype,
            open_kind,
            file_name,
            span: self.span_from(start),
        })
    }

    pub(super) fn designator(&mut self) -> PResult<Designator> {
        match self.peek() {
            Tok::Ident | Tok::ExtIdent => Ok(Designator::Ident(self.ident()?)),
            Tok::Str => {
                let t = self.bump();
                let span = self.tok_span(t);
                Ok(Designator::Operator(
                    Symbol::intern(&unquote(&self.text(t))),
                    span,
                ))
            }
            Tok::Char => Ok(Designator::Char(self.char_lit()?)),
            _ => Err(self.expected("designator")),
        }
    }

    fn alias_decl(&mut self) -> PResult<AliasDecl> {
        let start = self.expect_kw(Kw::Alias)?;
        let designator = self.designator()?;
        let subtype = if self.eat(Tok::Colon) {
            Some(self.subtype_indication()?)
        } else {
            None
        };
        self.expect_kw(Kw::Is)?;
        let name = self.name()?;
        self.semi()?;
        Ok(AliasDecl {
            designator,
            subtype,
            name,
            span: self.span_from(start),
        })
    }

    pub(super) fn component_decl(&mut self) -> PResult<ComponentDecl> {
        let start = self.expect_kw(Kw::Component)?;
        let name = self.ident()?;
        self.eat_kw(Kw::Is);
        let generics = self.opt_interface_clause(Kw::Generic)?;
        let ports = self.opt_interface_clause(Kw::Port)?;
        // 2019 (LCS-2016-055a): `end component` may drop its keyword.
        if self.at_kw(Kw::End) && self.nth(1) != Tok::Kw(Kw::Component) {
            let span = self.span();
            self.require_2019(span, "`end` without `component`");
        }
        let end_name = self.end(start, "component", &[Kw::Component], true, Some(&name))?;
        Ok(ComponentDecl {
            name,
            generics,
            ports,
            end_name,
            span: self.span_from(start),
        })
    }

    /// `generic (...);` or `port (...);` if present.
    pub(super) fn opt_interface_clause(&mut self, kw: Kw) -> PResult<Option<Vec<InterfaceDecl>>> {
        if !self.at_kw(kw) || self.nth(1) == Tok::Kw(Kw::Map) {
            return Ok(None);
        }
        self.bump();
        let list = self.interface_list()?;
        self.semi()?;
        Ok(Some(list))
    }

    fn attribute_decl(&mut self) -> PResult<Decl> {
        let start = self.expect_kw(Kw::Attribute)?;
        let attr = self.ident()?;
        if self.eat(Tok::Colon) {
            let type_mark = self.type_mark()?;
            self.semi()?;
            return Ok(Decl::Attribute(AttributeDecl {
                name: attr,
                type_mark,
                span: self.span_from(start),
            }));
        }
        self.expect_kw(Kw::Of)?;
        let entities = if self.eat_kw(Kw::Others) {
            EntityNameList::Others
        } else if self.eat_kw(Kw::All) {
            EntityNameList::All
        } else {
            let mut v = Vec::new();
            loop {
                let d = self.designator()?;
                let sig = if self.at(Tok::LBracket) {
                    Some(self.signature()?)
                } else {
                    None
                };
                v.push((d, sig));
                if !self.eat(Tok::Comma) {
                    break;
                }
            }
            EntityNameList::Names(v)
        };
        self.expect(Tok::Colon)?;
        let class_span = self.span();
        let class = self.entity_class()?;
        self.expect_kw(Kw::Is)?;
        let value = self.expr()?;
        self.semi()?;
        Ok(Decl::AttributeSpec(AttributeSpec {
            attr,
            entities,
            class,
            class_span,
            value,
            span: self.span_from(start),
        }))
    }

    fn entity_class(&mut self) -> PResult<Kw> {
        match self.peek() {
            Tok::Kw(k) if is_entity_class(k) => {
                self.bump();
                Ok(k)
            }
            _ => Err(self.expected("entity class")),
        }
    }

    fn group(&mut self) -> PResult<Decl> {
        let start = self.expect_kw(Kw::Group)?;
        let name = self.ident()?;
        if self.eat_kw(Kw::Is) {
            self.expect(Tok::LParen)?;
            let mut classes = Vec::new();
            loop {
                let k = self.entity_class()?;
                classes.push((k, self.eat(Tok::Box)));
                if !self.eat(Tok::Comma) {
                    break;
                }
            }
            self.expect(Tok::RParen)?;
            self.semi()?;
            return Ok(Decl::GroupTemplate(GroupTemplateDecl {
                name,
                classes,
                span: self.span_from(start),
            }));
        }
        self.expect(Tok::Colon)?;
        let template = self.type_mark()?;
        self.expect(Tok::LParen)?;
        let mut constituents = Vec::new();
        loop {
            if self.at(Tok::Char) {
                let c = self.char_lit()?;
                constituents.push(Name {
                    kind: NameKind::Character(c.value),
                    span: c.span,
                });
            } else {
                constituents.push(self.name()?);
            }
            if !self.eat(Tok::Comma) {
                break;
            }
        }
        self.expect(Tok::RParen)?;
        self.semi()?;
        Ok(Decl::Group(GroupDecl {
            name,
            template,
            constituents,
            span: self.span_from(start),
        }))
    }

    fn view_decl(&mut self) -> PResult<ViewDecl> {
        let start = self.expect_kw(Kw::View)?;
        let name = self.ident()?;
        self.expect_kw(Kw::Of)?;
        let subtype = self.subtype_indication()?;
        self.expect_kw(Kw::Is)?;
        let mut elements = Vec::new();
        while self.at_ident() {
            let s = self.span();
            let names = self.ident_list()?;
            self.expect(Tok::Colon)?;
            let mode = self.mode()?.ok_or_else(|| self.expected("mode"))?;
            self.semi()?;
            elements.push(ViewElement {
                names,
                mode,
                span: self.span_from(s),
            });
        }
        let end_name = self.end(start, "view", &[Kw::View], false, Some(&name))?;
        Ok(ViewDecl {
            name,
            subtype,
            elements,
            end_name,
            span: self.span_from(start),
        })
    }

    fn mode(&mut self) -> PResult<Option<Mode>> {
        let m = match self.peek() {
            Tok::Kw(Kw::In) => Mode::In,
            Tok::Kw(Kw::Out) => Mode::Out,
            Tok::Kw(Kw::Inout) => Mode::Inout,
            Tok::Kw(Kw::Buffer) => Mode::Buffer,
            Tok::Kw(Kw::Linkage) => Mode::Linkage,
            Tok::Kw(Kw::View) => {
                self.bump();
                return Ok(Some(if self.eat(Tok::LParen) {
                    let name = self.name()?;
                    self.expect(Tok::RParen)?;
                    Mode::View { name, array: true }
                } else {
                    Mode::View {
                        name: self.name()?,
                        array: false,
                    }
                }));
            }
            _ => return Ok(None),
        };
        self.bump();
        Ok(Some(m))
    }

    // Interface lists.

    /// `( decl ; decl ... )`
    pub(super) fn interface_list(&mut self) -> PResult<Vec<InterfaceDecl>> {
        let mark = self.sc.iface.len();
        match self.interface_items(mark) {
            Ok(()) => Ok(self.sc.iface.drain(mark..).collect()),
            Err(e) => {
                self.sc.iface.truncate(mark);
                Err(e)
            }
        }
    }

    /// The items of an interface list, pushed on the scratch stack above `mark`.
    fn interface_items(&mut self, mark: usize) -> PResult<()> {
        let open = self.expect(Tok::LParen)?;
        loop {
            if self.at(Tok::RParen) && self.sc.iface.len() > mark {
                let span = self.prev_span();
                self.require_2019(span, "a trailing `;` in an interface list");
                break;
            }
            if matches!(
                self.peek(),
                Tok::Kw(Kw::End | Kw::Begin | Kw::Is) | Tok::Eof
            ) {
                let found = self.peek().describe();
                let span = self.span();
                self.report(
                    yavs_diag::Diagnostic::error(
                        crate::codes::SYNTAX,
                        format!("expected interface declaration or `)`, found {found}"),
                    )
                    .with_primary(span, "expected `)`")
                    .with_secondary(open, "interface list starts here"),
                );
                return Err(Recover);
            }
            let d = self.interface_decl()?;
            self.sc.iface.push(d);
            if !self.eat(Tok::Semi) {
                break;
            }
        }
        if !self.at(Tok::RParen) {
            let found = self.peek().describe();
            let span = self.span();
            self.report(
                yavs_diag::Diagnostic::error(
                    crate::codes::SYNTAX,
                    format!("expected `;` or `)`, found {found}"),
                )
                .with_primary(span, "expected `;` or `)`")
                .with_secondary(open, "interface list starts here"),
            );
            return Err(Recover);
        }
        self.bump();
        Ok(())
    }

    fn interface_decl(&mut self) -> PResult<InterfaceDecl> {
        let start = self.span();
        match self.peek() {
            Tok::Kw(Kw::Type) => {
                self.bump();
                let name = self.ident()?;
                let def = if self.eat_kw(Kw::Is) {
                    self.require_2019(start, "a generic type definition");
                    Some(self.generic_type_def()?)
                } else {
                    None
                };
                Ok(InterfaceDecl::Type(Box::new(InterfaceType {
                    name,
                    def,
                    span: self.span_from(start),
                })))
            }
            Tok::Kw(Kw::Function | Kw::Procedure | Kw::Pure | Kw::Impure) => {
                let spec = self.subprogram_spec()?;
                let default = if self.eat_kw(Kw::Is) {
                    if self.eat(Tok::Box) {
                        Some(SubprogramDefault::Box)
                    } else {
                        Some(SubprogramDefault::Name(self.name()?))
                    }
                } else {
                    None
                };
                Ok(InterfaceDecl::Subprogram(Box::new(InterfaceSubprogram {
                    spec,
                    default,
                })))
            }
            Tok::Kw(Kw::Package) => {
                self.bump();
                let name = self.ident()?;
                self.expect_kw(Kw::Is)?;
                self.expect_kw(Kw::New)?;
                let package = self.type_mark()?;
                self.expect_kw(Kw::Generic)?;
                self.expect_kw(Kw::Map)?;
                let generic_map = if self.at(Tok::LParen)
                    && self.nth(1) == Tok::Box
                    && self.nth(2) == Tok::RParen
                {
                    self.pos += 3;
                    InterfacePackageMap::Box
                } else if self.at(Tok::LParen)
                    && self.nth(1) == Tok::Kw(Kw::Default)
                    && self.nth(2) == Tok::RParen
                {
                    self.pos += 3;
                    InterfacePackageMap::Default
                } else {
                    InterfacePackageMap::Assoc(self.assoc_list()?)
                };
                Ok(InterfaceDecl::Package(Box::new(InterfacePackage {
                    name,
                    package,
                    generic_map,
                    span: self.span_from(start),
                })))
            }
            _ => {
                let class = match self.peek() {
                    Tok::Kw(Kw::Constant) => Some(InterfaceClass::Constant),
                    Tok::Kw(Kw::Signal) => Some(InterfaceClass::Signal),
                    Tok::Kw(Kw::Variable) => Some(InterfaceClass::Variable),
                    Tok::Kw(Kw::File) => Some(InterfaceClass::File),
                    _ => None,
                };
                if class.is_some() {
                    self.bump();
                }
                let names = self.ident_list()?;
                self.expect(Tok::Colon)?;
                let mode = self.mode()?;
                let view = matches!(mode, Some(Mode::View { .. }));
                let subtype = if view {
                    if self.eat_kw(Kw::Of) {
                        Some(self.incomplete_subtype()?)
                    } else {
                        None
                    }
                } else {
                    Some(self.incomplete_subtype()?)
                };
                let bus = self.eat_kw(Kw::Bus);
                let default = if self.eat(Tok::Assign) {
                    Some(self.cond_expr()?)
                } else {
                    None
                };
                Ok(InterfaceDecl::Object(InterfaceObject {
                    class,
                    names,
                    mode,
                    subtype,
                    bus,
                    default,
                    span: self.span_from(start),
                }))
            }
        }
    }

    fn incomplete_subtype(&mut self) -> PResult<IncompleteSubtype> {
        if self.at_kw(Kw::Type) {
            let start = self.bump_span();
            self.require_2019(start, "an anonymous type");
            self.expect_kw(Kw::Is)?;
            let def = self.generic_type_def()?;
            return Ok(IncompleteSubtype::Anonymous(
                Box::new(def),
                self.span_from(start),
            ));
        }
        Ok(IncompleteSubtype::Subtype(self.subtype_indication()?))
    }

    fn generic_type_def(&mut self) -> PResult<GenericTypeDef> {
        self.nested(|p| {
            // `private` is only reserved in VHDL-2019. In 2008 the error is
            // already reported, so take the identifier rather than add another.
            let private = p.at_kw(Kw::Private)
                || p.at(Tok::Ident) && p.text(p.toks[p.pos]).eq_ignore_ascii_case("private");
            Ok(match p.peek() {
                _ if private => {
                    p.bump();
                    GenericTypeDef::Private
                }
                Tok::Box => {
                    p.bump();
                    GenericTypeDef::Scalar
                }
                Tok::LParen => {
                    p.bump();
                    p.expect(Tok::Box)?;
                    p.expect(Tok::RParen)?;
                    GenericTypeDef::Discrete
                }
                Tok::Kw(Kw::Range) => {
                    p.bump();
                    p.expect(Tok::Box)?;
                    if p.eat(Tok::Dot) {
                        p.expect(Tok::Box)?;
                        GenericTypeDef::Floating
                    } else {
                        GenericTypeDef::Integer
                    }
                }
                Tok::Kw(Kw::Units) => {
                    p.bump();
                    p.expect(Tok::Box)?;
                    GenericTypeDef::Physical
                }
                Tok::Kw(Kw::Array) => {
                    p.bump();
                    let indexes = if p.at(Tok::LParen) && p.nth(1) == Tok::Kw(Kw::Type) {
                        p.bump();
                        let mut v = Vec::new();
                        loop {
                            p.expect_kw(Kw::Type)?;
                            p.expect_kw(Kw::Is)?;
                            v.push(p.generic_type_def()?);
                            if !p.eat(Tok::Comma) {
                                break;
                            }
                        }
                        p.expect(Tok::RParen)?;
                        ArrayIndexes::Anonymous(v)
                    } else {
                        p.array_indexes()?
                    };
                    p.expect_kw(Kw::Of)?;
                    let element = p.incomplete_subtype()?;
                    GenericTypeDef::Array { indexes, element }
                }
                Tok::Kw(Kw::Access) => {
                    p.bump();
                    GenericTypeDef::Access(p.incomplete_subtype()?)
                }
                Tok::Kw(Kw::File) => {
                    p.bump();
                    p.expect_kw(Kw::Of)?;
                    GenericTypeDef::File(p.incomplete_subtype()?)
                }
                _ => return Err(p.expected("generic type definition")),
            })
        })
    }

    // Subprograms.

    fn subprogram_spec(&mut self) -> PResult<SubprogramSpec> {
        let start = self.span();
        // Parts LRM 4.4 does not allow before `is new`, reported once we know it follows.
        let mut illegal: Vec<(yavs_diag::Span, &str)> = Vec::new();
        if self.at_kw(Kw::Pure) || self.at_kw(Kw::Impure) {
            illegal.push((self.span(), "`pure` or `impure`"));
        }
        let purity = if self.eat_kw(Kw::Pure) {
            Some(Purity::Pure)
        } else if self.eat_kw(Kw::Impure) {
            Some(Purity::Impure)
        } else {
            None
        };
        let kind = if self.eat_kw(Kw::Function) {
            SubprogramKind::Function(purity)
        } else if purity.is_none() && self.eat_kw(Kw::Procedure) {
            SubprogramKind::Procedure
        } else {
            return Err(self.expected("`function`"));
        };
        let designator = self.designator()?;
        let (generics, generic_map) = if self.at_kw(Kw::Generic) && self.nth(1) == Tok::LParen {
            illegal.push((self.span(), "generic clause"));
            self.bump();
            let g = self.interface_list()?;
            (Some(g), self.opt_map(Kw::Generic)?)
        } else {
            (None, None)
        };
        if self.at_kw(Kw::Parameter) || self.at(Tok::LParen) {
            illegal.push((self.span(), "parameter list"));
        }
        let has_param_kw = self.eat_kw(Kw::Parameter);
        let params = if self.at(Tok::LParen) || has_param_kw {
            Some(self.interface_list()?)
        } else {
            None
        };
        let (mut return_ident, mut return_type) = (None, None);
        let instance = self.at_kw(Kw::Is) && self.nth(1) == Tok::Kw(Kw::New);
        if let SubprogramKind::Function(_) = kind
            && !instance
        {
            illegal.push((self.span(), "`return`"));
            self.expect_kw(Kw::Return)?;
            if self.at_ident() && self.nth(1) == Tok::Kw(Kw::Of) {
                let id = self.ident()?;
                self.require_2019(id.span, "a named return value");
                self.bump();
                return_ident = Some(id);
            }
            return_type = Some(self.type_mark()?);
        }
        if self.at_kw(Kw::Is) && self.nth(1) == Tok::Kw(Kw::New) {
            for (span, what) in illegal {
                self.report(
                    yavs_diag::Diagnostic::error(
                        crate::codes::SYNTAX,
                        format!("unexpected {what} in subprogram instantiation"),
                    )
                    .with_primary(span, "not allowed before `is new`")
                    .with_help(
                        "a subprogram instantiation is `function|procedure designator is new \
                         name [signature] [generic map (...)];`; the instance takes purity, \
                         parameters and result from the uninstantiated subprogram",
                    ),
                );
            }
        }
        Ok(SubprogramSpec {
            kind,
            designator,
            generics,
            generic_map,
            params,
            return_ident,
            return_type,
            span: self.span_from(start),
        })
    }

    fn subprogram(&mut self) -> PResult<Decl> {
        let start = self.span();
        let spec = self.subprogram_spec()?;
        if self.at(Tok::Semi) {
            self.bump();
            return Ok(Decl::Subprogram(spec));
        }
        self.expect_kw(Kw::Is)?;
        if self.eat_kw(Kw::New) {
            let subprogram = self.name()?;
            let generic_map = self.opt_map(Kw::Generic)?;
            self.semi()?;
            return Ok(Decl::SubprogramInstance(SubprogramInstance {
                kind: spec.kind,
                designator: spec.designator,
                subprogram,
                generic_map,
                span: self.span_from(start),
            }));
        }
        let decls = self.decls(false);
        self.expect_kw(Kw::Begin)?;
        let stmts = self.seq_stmts();
        let what = match spec.kind {
            SubprogramKind::Procedure => "procedure",
            SubprogramKind::Function(_) => "function",
        };
        if !self.at_kw(Kw::End) {
            let found = self.peek().describe();
            let span = self.span();
            self.report(
                yavs_diag::Diagnostic::error(
                    crate::codes::SYNTAX,
                    format!("expected `end`, found {found}"),
                )
                .with_primary(span, "expected `end`")
                .with_secondary(start, format!("this {what} is not closed")),
            );
            return Err(Recover);
        }
        self.bump();
        let kw = match spec.kind {
            SubprogramKind::Procedure => Kw::Procedure,
            SubprogramKind::Function(_) => Kw::Function,
        };
        self.eat_kw(kw);
        let end_name = if matches!(
            self.peek(),
            Tok::Ident | Tok::ExtIdent | Tok::Str | Tok::Char
        ) {
            Some(self.designator()?)
        } else {
            None
        };
        if let Some(e) = &end_name {
            self.check_end_designator(e, &spec.designator);
        }
        self.semi()?;
        Ok(Decl::SubprogramBody(SubprogramBody {
            spec,
            decls,
            stmts,
            end_name,
            span: self.span_from(start),
        }))
    }

    // Packages.

    /// A package declaration or instantiation, as a declaration.
    fn package(&mut self) -> PResult<Decl> {
        Ok(match self.package_unit()? {
            LibraryUnit::Package(p) => Decl::Package(p),
            LibraryUnit::PackageInstance(p) => Decl::PackageInstance(p),
            _ => return Err(Recover),
        })
    }

    pub(super) fn package_unit(&mut self) -> PResult<LibraryUnit> {
        let start = self.expect_kw(Kw::Package)?;
        let name = self.ident()?;
        self.expect_kw(Kw::Is)?;
        if self.at_kw(Kw::New) {
            let at = self.span();
            self.require_2008(at, "a package instantiation");
        }
        if self.eat_kw(Kw::New) {
            let package = self.type_mark()?;
            let generic_map = self.opt_map(Kw::Generic)?;
            self.semi()?;
            return Ok(LibraryUnit::PackageInstance(PackageInstance {
                name,
                package,
                generic_map,
                span: self.span_from(start),
            }));
        }
        if self.at_kw(Kw::Generic) {
            let at = self.span();
            self.require_2008(at, "a package with generics");
        }
        let (generics, generic_map) = self.generic_header()?;
        let decls = self.decls(false);
        let end_name = self.end(start, "package", &[Kw::Package], true, Some(&name))?;
        Ok(LibraryUnit::Package(PackageDecl {
            name,
            generics,
            generic_map,
            decls,
            end_name,
            span: self.span_from(start),
        }))
    }

    pub(super) fn package_body(&mut self) -> PResult<PackageBody> {
        let start = self.expect_kw(Kw::Package)?;
        self.expect_kw(Kw::Body)?;
        let name = self.ident()?;
        self.expect_kw(Kw::Is)?;
        let decls = self.decls(false);
        let end_name = self.end(
            start,
            "package body",
            &[Kw::Package, Kw::Body],
            true,
            Some(&name),
        )?;
        Ok(PackageBody {
            name,
            decls,
            end_name,
            span: self.span_from(start),
        })
    }

    // Configuration.

    /// `inst, inst : comp` / `all : comp` / `others : comp`
    pub(super) fn component_spec(&mut self) -> PResult<ComponentSpec> {
        let start = self.span();
        let instances = if self.eat_kw(Kw::All) {
            InstanceList::All
        } else if self.eat_kw(Kw::Others) {
            InstanceList::Others
        } else {
            InstanceList::Labels(self.ident_list()?)
        };
        self.expect(Tok::Colon)?;
        let component = self.type_mark()?;
        Ok(ComponentSpec {
            instances,
            component,
            span: self.span_from(start),
        })
    }

    /// The binding indication after a component specification, possibly empty.
    pub(super) fn binding(&mut self) -> PResult<BindingIndication> {
        let start = self.span();
        let entity_aspect = if self.eat_kw(Kw::Use) {
            Some(if self.eat_kw(Kw::Entity) {
                let name = self.type_mark()?;
                let arch = if self.eat(Tok::LParen) {
                    let a = self.ident()?;
                    self.expect(Tok::RParen)?;
                    Some(a)
                } else {
                    None
                };
                EntityAspect::Entity { name, arch }
            } else if self.eat_kw(Kw::Configuration) {
                EntityAspect::Configuration(self.type_mark()?)
            } else if self.eat_kw(Kw::Open) {
                EntityAspect::Open
            } else if self.at_kw(Kw::Vunit) {
                // `use vunit a, b`: a PSL binding, skipped.
                self.psl_warn();
                self.bump();
                self.name()?;
                while self.eat(Tok::Comma) {
                    self.name()?;
                }
                return Ok(BindingIndication {
                    entity_aspect: None,
                    generic_map: None,
                    port_map: None,
                    span: self.span_from(start),
                });
            } else {
                return Err(self.expected("`entity`, `configuration` or `open`"));
            })
        } else {
            None
        };
        let generic_map = self.opt_map(Kw::Generic)?;
        let port_map = self.opt_map(Kw::Port)?;
        Ok(BindingIndication {
            entity_aspect,
            generic_map,
            port_map,
            span: self.span_from(start),
        })
    }

    fn config_spec(&mut self) -> PResult<ConfigSpec> {
        let start = self.expect_kw(Kw::For)?;
        let spec = self.component_spec()?;
        let binding = self.binding()?;
        self.semi()?;
        if self.at_kw(Kw::End) && self.nth(1) == Tok::Kw(Kw::For) {
            self.pos += 2;
            self.semi()?;
        }
        Ok(ConfigSpec {
            spec,
            binding,
            span: self.span_from(start),
        })
    }
}
