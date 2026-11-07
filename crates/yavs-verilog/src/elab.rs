//! Elaboration of a Verilog module instance that VHDL instantiates (#776 step C): parameters from
//! VHDL generics, nets and variables as domain nets, continuous assignments, `always` and
//! `initial` blocks compiled to process code (`exec.rs`), and the ports bound to VHDL signals.
//! Everything of the design goes into one [`Design`], which makes the single serial [`Domain`] of a
//! run (`domain`, once per build).

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use yavs_kernel::{DriverId, LogicVec, SigRef};

use crate::ast::{
    Ctrl, Decl, Declarator, Delay, EventCtrl, Expr, Ident, Item, Module, NetKind, ParamDecl, Ports,
    Range, RangeKind, Stmt, VarKind,
};
use crate::exec::{Arg, CaseKind, Code, E, Inst, Lhs, Op, Start, reads};
use crate::host::Tables;
use crate::lower::{LowerError, Lowerer, MAX_WIDTH};
use crate::runtime::{Domain, Edge, NetId};
use crate::sem::{Const, R, Sx, Sym, Ty, Var, real_to_i64, string_value, unsupported};
use crate::value::V4;
use crate::{Diag, Dir, Source, Span};

/// The value of a VHDL generic that sets a parameter.
#[derive(Clone, Debug)]
pub enum ParamValue {
    /// `integer`: a 32-bit signed value.
    Int(i64),
    Real(f64),
    /// `string`: 8 bits per character.
    Str(Vec<u8>),
    /// `std_logic_vector`, `bit_vector`, `std_logic`, `bit`, `boolean`: element 0 the rightmost.
    Logic(LogicVec),
}

/// A VHDL generic of the instance.
#[derive(Clone, Debug)]
pub struct Generic {
    pub name: String,
    pub value: ParamValue,
}

/// A VHDL port of the instance: matched by name, `key` is the caller's number for it.
#[derive(Clone, Debug)]
pub struct PortSpec {
    pub name: String,
    pub input: bool,
    pub width: u32,
    /// A `bit` or `bit_vector` port: x and z go out as '0'.
    pub bit: bool,
    pub key: u32,
}

/// A module instance as VHDL instantiates it.
pub struct Instance<'a> {
    /// The VHDL path of the instance (`%m`, messages).
    pub path: &'a str,
    pub source: &'a Source,
    pub module: &'a Module,
    pub generics: &'a [Generic],
    pub ports: &'a [PortSpec],
}

/// Every Verilog instance of a design.
#[derive(Default)]
pub struct Design {
    nets: Vec<LogicVec>,
    /// The processes, lowered to IR as they are elaborated (`lower.rs`).
    low: Lowerer,
    /// The program, its runtime and the tables of the processes, made by the first `domain`.
    tabs: OnceLock<Result<Arc<Tables>, String>>,
    inputs: Vec<(u32, u32)>,
    outputs: Vec<(u32, u32, bool)>,
    /// The smallest precision of the instances, for `%t` (17.3.2).
    tf_fs: u64,
}

impl Design {
    pub fn is_empty(&self) -> bool {
        self.low.procs.is_empty() && self.inputs.is_empty() && self.outputs.is_empty()
    }

    /// The keys of the VHDL ports the design drives, in the order `domain` takes their drivers.
    pub fn outputs(&self) -> Vec<u32> {
        self.outputs.iter().map(|o| o.0).collect()
    }

    /// The keys of the VHDL ports the design reads.
    pub fn inputs(&self) -> Vec<u32> {
        self.inputs.iter().map(|i| i.0).collect()
    }

    /// The domain of one build: `input` is the signal of an input key, `drivers` the kernel
    /// drivers of `outputs()` in order.
    pub fn domain(&self, input: &dyn Fn(u32) -> SigRef, drivers: &[DriverId]) -> Domain {
        let mut d = Domain::new();
        for init in &self.nets {
            d.net(init.clone());
        }
        for &(k, n) in &self.inputs {
            d.input(input(k), NetId(n));
        }
        for (&(_, n, bit), &drv) in self.outputs.iter().zip(drivers) {
            match bit {
                true => d.output_bit(NetId(n), drv),
                false => d.output(NetId(n), drv),
            }
        }
        match self.tables() {
            Ok(t) => {
                for (p, info) in t.procs.iter().enumerate() {
                    d.process(&info.name, crate::host::body(t.clone(), p));
                }
            }
            // `prepare` reports it; a caller that did not call it gets the error at the start.
            Err(m) => {
                let m = m.clone();
                d.process("verilog", move |x| Err(x.error(m.clone())));
            }
        }
        d
    }

    /// Makes the program of the design's processes and its runtime: an error is a bug in yavs.
    /// `domain` calls it if the caller did not.
    pub fn prepare(&self) -> Result<(), String> {
        self.tables().map(|_| ())
    }

    /// The runtime the design's processes run on (a compiled tier is set on it, #776).
    pub fn runtime(&self) -> Result<Arc<yavs_interp::Runtime>, String> {
        Ok(self.tables()?.rt.clone())
    }

    /// The number of process functions: processes with the same body share one (tests).
    pub fn functions(&self) -> usize {
        self.low.functions()
    }

    #[cfg(test)]
    pub(crate) fn nets_mut(&mut self) -> &mut Vec<LogicVec> {
        &mut self.nets
    }

    /// A process made by hand (tests).
    #[cfg(test)]
    pub(crate) fn add_code(&mut self, code: &Code) -> Result<(), String> {
        let widths: Vec<u32> = self.nets.iter().map(|n| n.len() as u32).collect();
        self.low.add(code, &widths).map_err(|e| match e {
            LowerError::Wide(w) => format!("{w} bits"),
            LowerError::Bug(m) => m,
        })
    }

    fn tables(&self) -> Result<&Arc<Tables>, String> {
        let t = self.tabs.get_or_init(|| {
            let prog = self.low.pb.clone().finish().map_err(|es| {
                let e: Vec<String> = es
                    .iter()
                    .map(|e| format!("{}: {}", e.name, e.msg))
                    .collect();
                format!(
                    "the IR of the Verilog processes does not verify: {}",
                    e.join("; ")
                )
            })?;
            let rt = yavs_interp::Runtime::new(Arc::new(prog), yavs_interp::Bindings::default())
                .map_err(|e| e.to_string())?;
            Ok(Arc::new(Tables {
                rt,
                tasks: self.low.tasks.clone(),
                procs: self.low.procs.clone(),
                tf_fs: self.tf_fs.max(1),
            }))
        });
        t.as_ref()
            .map_err(|m| format!("{m} (this is a bug in yavs)"))
    }

    /// Elaborates one instance into the design.
    pub fn add(&mut self, inst: &Instance<'_>) -> Result<(), Vec<Diag>> {
        let mut el = Elab::new(self, inst);
        match el.module() {
            Ok(()) if el.errors.is_empty() => Ok(()),
            Ok(()) => Err(el.errors),
            Err(d) => {
                el.errors.push(*d);
                Err(el.errors)
            }
        }
    }
}

/// Elements `.0 .. .0 + .1` of a net and what drives them.
type Driver = (u32, u32, &'static str);

/// How a name is declared before it gets its net.
struct Decl0 {
    name: Ident,
    dir: Option<Dir>,
    range: Option<(i64, i64)>,
    signed: bool,
    reg: bool,
    init: Option<Expr>,
}

struct Elab<'d, 'a> {
    d: &'d mut Design,
    inst: &'a Instance<'a>,
    syms: HashMap<String, Sym>,
    vars: Vec<Var>,
    errors: Vec<Diag>,
    info: Arc<Inst>,
    spanf: Box<dyn Fn(u32, u32) -> Span + 'a>,
    at: Span,
    /// Per variable: the elements continuous drivers cover, and whether a process writes it.
    driven: Vec<(Vec<Driver>, bool)>,
    counts: HashMap<&'static str, u32>,
}

fn find<'n>(
    names: impl Iterator<Item = &'n str>,
    want: &str,
) -> Result<Option<&'n str>, Vec<&'n str>> {
    let all: Vec<&str> = names.collect();
    if let Some(n) = all.iter().find(|n| **n == want) {
        return Ok(Some(n));
    }
    let near: Vec<&str> = all
        .iter()
        .copied()
        .filter(|n| n.eq_ignore_ascii_case(want))
        .collect();
    match near.as_slice() {
        [] => Ok(None),
        [n] => Ok(Some(n)),
        _ => Err(near),
    }
}

impl<'d, 'a> Elab<'d, 'a> {
    fn new(d: &'d mut Design, inst: &'a Instance<'a>) -> Elab<'d, 'a> {
        let ts = inst
            .module
            .timescale
            .unwrap_or(inst.source.default_timescale);
        let src = inst.source;
        let spanf: Box<dyn Fn(u32, u32) -> Span + 'a> = Box::new(move |a, b| src.span(a, b - a));
        let m = &inst.module.name;
        let at = spanf(m.pos, m.pos + m.name.len() as u32);
        d.tf_fs = match d.tf_fs {
            0 => ts.precision_fs,
            t => t.min(ts.precision_fs),
        };
        Elab {
            d,
            inst,
            syms: HashMap::new(),
            vars: Vec::new(),
            errors: Vec::new(),
            info: Arc::new(Inst {
                path: inst.path.to_string(),
                unit_fs: ts.unit_fs,
                precision_fs: ts.precision_fs,
            }),
            spanf,
            at,
            driven: Vec::new(),
            counts: HashMap::new(),
        }
    }

    fn sx(&self) -> Sx<'_> {
        Sx {
            syms: &self.syms,
            vars: &self.vars,
            at: self.at,
            span: &*self.spanf,
            konst: false,
            unit_fs: self.info.unit_fs,
        }
    }

    fn span_id(&self, i: &Ident) -> Span {
        (self.spanf)(i.pos, i.pos + i.name.len().max(1) as u32)
    }

    fn module(&mut self) -> R<()> {
        let m = self.inst.module;
        if m.encrypted.is_some() {
            return Err(Box::new(
                Diag::error(
                    "E4001",
                    format!(
                        "the module `{}` of the instance {} is encrypted and cannot be simulated",
                        m.name.name, self.inst.path
                    ),
                    self.at,
                )
                .help("yavs never decrypts protected IP; use the vendor's unencrypted simulation model"),
            ));
        }
        self.params()?;
        let decls = self.declarations()?;
        self.nets(decls)?;
        self.bind_ports()?;
        self.items()?;
        self.check_drivers();
        Ok(())
    }

    // ---- parameters ----

    fn params(&mut self) -> R<()> {
        let m = self.inst.module;
        let header = !m.params.is_empty();
        let mut list: Vec<(&ParamDecl, bool)> = m.params.iter().map(|p| (p, p.local)).collect();
        for it in &m.items {
            if let Item::Param(p) = it {
                // With a parameter port list, a body parameter is local (12.2).
                list.push((p, p.local || header || p.spec));
            }
        }
        let names: Vec<(&str, bool)> = list
            .iter()
            .flat_map(|(p, local)| {
                p.assigns
                    .iter()
                    .map(move |(i, _)| (i.name.as_str(), *local))
            })
            .collect();
        let mut over: HashMap<&str, &ParamValue> = HashMap::new();
        for g in self.inst.generics {
            match find(names.iter().map(|n| n.0), &g.name) {
                Ok(Some(n)) if names.iter().any(|x| x.0 == n && x.1) => {
                    self.errors.push(
                        Diag::error(
                            "E4701",
                            format!("the generic `{}` names the local parameter `{n}` of module `{}`", g.name, m.name.name),
                            self.at,
                        )
                        .note("IEEE 1364-2005 12.2: a local parameter cannot be overridden")
                        .note(format!("instance {}", self.inst.path)),
                    );
                }
                Ok(Some(n)) => {
                    over.insert(n, &g.value);
                }
                Ok(None) => {
                    let mut d = Diag::error(
                        "E4701",
                        format!("the generic `{}` has no parameter in module `{}`", g.name, m.name.name),
                        self.at,
                    )
                    .note(format!("instance {}", self.inst.path));
                    let cands: Vec<&str> = names.iter().filter(|n| !n.1).map(|n| n.0).collect();
                    if let Some(h) = yavs_diag::suggest::suggest(&g.name, "parameter", cands.iter().map(|n| (*n, *n))) {
                        d = d.help(h);
                    } else if !cands.is_empty() {
                        d = d.note(format!("its parameters: {}", cands.join(", ")));
                    }
                    self.errors.push(d);
                }
                Err(many) => self.errors.push(
                    Diag::error(
                        "E4701",
                        format!("the generic `{}` matches several parameters of module `{}` that differ only in case: {}", g.name, m.name.name, many.join(", ")),
                        self.at,
                    )
                    .note(format!("instance {}", self.inst.path)),
                ),
            }
        }
        for (p, _) in list {
            let declared = self.param_type(p)?;
            for (name, e) in &p.assigns {
                let v = match over.get(name.name.as_str()) {
                    Some(v) => from_generic(v),
                    None => self.sx().konst(e)?,
                };
                let v = match declared {
                    Some(t) => convert(v, t),
                    None => v,
                };
                self.syms.insert(name.name.clone(), Sym::Param(v));
            }
        }
        Ok(())
    }

    /// The declared type of a parameter, `None` when it takes its value's (12.2).
    fn param_type(&self, p: &ParamDecl) -> R<Option<Ty>> {
        Ok(match (p.kind, &p.range) {
            (Some(VarKind::Integer), _) => Some(Ty::vec(32, true)),
            (Some(VarKind::Time), _) => Some(Ty::vec(64, false)),
            (Some(VarKind::Real | VarKind::Realtime), _) => Some(Ty::REAL),
            (_, Some(r)) => {
                let (msb, lsb) = self.range(r)?;
                Some(Ty::vec((msb - lsb).unsigned_abs() as u32 + 1, p.signed))
            }
            // `parameter signed P = v`: signed, the width of the value.
            (_, None) if p.signed => Some(Ty::vec(0, true)),
            _ => None,
        })
    }

    fn range(&self, r: &Range) -> R<(i64, i64)> {
        let sx = self.sx();
        Ok((sx.konst_int(&r.msb)?, sx.konst_int(&r.lsb)?))
    }

    // ---- declarations ----

    fn declarations(&mut self) -> R<Vec<Decl0>> {
        let m = self.inst.module;
        let mut out: Vec<Decl0> = Vec::new();
        let add = |el: &Self,
                   out: &mut Vec<Decl0>,
                   d: &Declarator,
                   dir: Option<Dir>,
                   range: Option<(i64, i64)>,
                   signed: bool,
                   reg: bool|
         -> R<()> {
            if !d.dims.is_empty() {
                return Err(unsupported(
                    "a memory (an array of variables)",
                    el.span_id(&d.name),
                ));
            }
            match out.iter_mut().find(|x| x.name.name == d.name.name) {
                // A non-ANSI port's direction and its type declaration (12.3.3) meet here.
                Some(x) => {
                    x.dir = x.dir.or(dir);
                    x.range = x.range.or(range);
                    x.signed |= signed;
                    x.reg |= reg;
                    x.init = x.init.clone().or(d.init.clone());
                }
                None => out.push(Decl0 {
                    name: d.name.clone(),
                    dir,
                    range,
                    signed,
                    reg,
                    init: d.init.clone(),
                }),
            }
            Ok(())
        };
        let port_items: Vec<&crate::ast::PortDecl> = match &m.ports {
            Ports::Ansi(v) => v.iter().collect(),
            _ => Vec::new(),
        };
        let body_ports = m.items.iter().filter_map(|i| match i {
            Item::Port(p) => Some(p),
            _ => None,
        });
        for p in port_items.into_iter().chain(body_ports) {
            if p.dir == Dir::Inout {
                let n = p.names.first().map(|d| &d.name);
                return Err(unsupported(
                    "an inout port",
                    n.map_or(self.at, |n| self.span_id(n)),
                ));
            }
            let (range, signed, reg) = match p.var {
                Some(VarKind::Integer) => (Some((31, 0)), true, true),
                Some(VarKind::Time) => (Some((63, 0)), false, true),
                Some(VarKind::Real | VarKind::Realtime) => {
                    return Err(unsupported("a real port", self.at));
                }
                Some(VarKind::Reg) => (self.opt_range(&p.range)?, p.signed, true),
                None => {
                    self.net_kind(p.net.unwrap_or(NetKind::Wire), &p.names)?;
                    (self.opt_range(&p.range)?, p.signed, false)
                }
            };
            for d in &p.names {
                add(self, &mut out, d, Some(p.dir), range, signed, reg)?;
            }
        }
        for it in &m.items {
            match it {
                Item::Net(n) => {
                    self.net_kind(n.kind, &n.names)?;
                    if n.strength.is_some() {
                        return Err(unsupported("a drive strength", self.decl_at(&n.names)));
                    }
                    if n.delay.is_some() {
                        return Err(unsupported("a net delay", self.decl_at(&n.names)));
                    }
                    let r = self.opt_range(&n.range)?;
                    for d in &n.names {
                        add(self, &mut out, d, None, r, n.signed, false)?;
                    }
                }
                Item::Var(v) => {
                    let (r, s) = match v.kind {
                        VarKind::Reg => (self.opt_range(&v.range)?, v.signed),
                        VarKind::Integer => (Some((31, 0)), true),
                        VarKind::Time => (Some((63, 0)), false),
                        VarKind::Real | VarKind::Realtime => {
                            return Err(unsupported("a real variable", self.decl_at(&v.names)));
                        }
                    };
                    for d in &v.names {
                        add(self, &mut out, d, None, r, s, true)?;
                    }
                }
                Item::Event(names) => {
                    return Err(unsupported("a named event", self.decl_at(names)));
                }
                _ => {}
            }
        }
        // A name a continuous assignment drives without declaring it is an implicit scalar net
        // (6.1.2), unless `default_nettype none`.
        for it in &m.items {
            if let Item::Assign(a) = it {
                for (l, _) in &a.assigns {
                    if let Expr::Ident(i) = l
                        && !out.iter().any(|d| d.name.name == i.name)
                        && !self.syms.contains_key(&i.name)
                        && m.nettype != crate::Nettype::None
                    {
                        out.push(Decl0 {
                            name: i.clone(),
                            dir: None,
                            range: None,
                            signed: false,
                            reg: false,
                            init: None,
                        });
                    }
                }
            }
        }
        Ok(out)
    }

    fn decl_at(&self, names: &[Declarator]) -> Span {
        names.first().map_or(self.at, |d| self.span_id(&d.name))
    }

    fn net_kind(&self, k: NetKind, names: &[Declarator]) -> R<()> {
        match k {
            NetKind::Wire | NetKind::Tri => Ok(()),
            k => Err(unsupported(
                &format!("the net type `{}`", format!("{k:?}").to_lowercase()),
                self.decl_at(names),
            )),
        }
    }

    fn opt_range(&self, r: &Option<Range>) -> R<Option<(i64, i64)>> {
        r.as_ref().map(|r| self.range(r)).transpose()
    }

    /// Gives every declaration its net; a pure copy `assign a = b;` onto an internal net with no
    /// other driver makes `a` the net of `b` (plan 3.6).
    fn nets(&mut self, decls: Vec<Decl0>) -> R<()> {
        let m = self.inst.module;
        let index: HashMap<&str, usize> = decls
            .iter()
            .enumerate()
            .map(|(i, d)| (d.name.name.as_str(), i))
            .collect();
        let width = |d: &Decl0| {
            d.range
                .map_or(1, |(a, b)| (a - b).unsigned_abs() as u32 + 1)
        };
        // How many continuous assignments name each declaration on their left.
        let mut targets: HashMap<&str, u32> = HashMap::new();
        let mut copies = Vec::new();
        for it in &m.items {
            match it {
                Item::Assign(a) => {
                    for (l, r) in &a.assigns {
                        let mut names = Vec::new();
                        lhs_names(l, &mut names);
                        for n in names {
                            *targets.entry(n).or_default() += 1;
                        }
                        if let (Expr::Ident(a), Expr::Ident(b)) = (l, r) {
                            copies.push((a.name.as_str(), b.name.as_str()));
                        }
                    }
                }
                Item::Net(n) => {
                    for d in &n.names {
                        if let Some(r) = &d.init {
                            *targets.entry(d.name.name.as_str()).or_default() += 1;
                            if let Expr::Ident(b) = r {
                                copies.push((d.name.name.as_str(), b.name.as_str()));
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        // Union of aliases: `of[i]` is the declaration whose net `i` uses.
        let mut of: Vec<usize> = (0..decls.len()).collect();
        fn root(of: &[usize], mut i: usize) -> usize {
            while of[i] != i {
                i = of[i];
            }
            i
        }
        for (a, b) in copies {
            let (Some(&ia), Some(&ib)) = (index.get(a), index.get(b)) else {
                continue;
            };
            let da = &decls[ia];
            // Only an internal net whose one driver is this copy, of the copy's width.
            if da.dir.is_some()
                || da.reg
                || targets.get(a) != Some(&1)
                || width(da) != width(&decls[ib])
            {
                continue;
            }
            let rb = root(&of, ib);
            if rb != ia {
                of[ia] = rb;
            }
        }
        let mut net_of: Vec<Option<u32>> = vec![None; decls.len()];
        for i in 0..decls.len() {
            let r = root(&of, i);
            if net_of[r].is_none() {
                let d = &decls[r];
                let w = width(d);
                let init = match &d.init {
                    // `reg r = v;` starts at `v` (6.2.1, read as the initial value).
                    Some(e) if d.reg => match self.sx().konst(e)? {
                        Const::V(v, s) => v.ext(w, s).to_logic(),
                        Const::R(r) => V4::from_i64(64, real_to_i64(r)).ext(w, true).to_logic(),
                    },
                    _ if d.reg => V4::xs(w).to_logic(),
                    _ => V4::zs(w).to_logic(),
                };
                self.d.nets.push(init);
                net_of[r] = Some(self.d.nets.len() as u32 - 1);
            }
            net_of[i] = net_of[r];
        }
        for (i, d) in decls.into_iter().enumerate() {
            let (msb, lsb) = d.range.unwrap_or((0, 0));
            if self.syms.contains_key(&d.name.name) {
                return Err(Box::new(Diag::error(
                    "E4203",
                    format!(
                        "`{}` is declared twice in module `{}`",
                        d.name.name, m.name.name
                    ),
                    self.span_id(&d.name),
                )));
            }
            self.syms
                .insert(d.name.name.clone(), Sym::Var(self.vars.len()));
            self.vars.push(Var {
                name: d.name.name.clone(),
                net: net_of[i].expect("every declaration has a net"),
                msb,
                lsb,
                signed: d.signed,
                reg: d.reg,
                pos: d.name.pos,
            });
            self.driven.push((Vec::new(), false));
        }
        Ok(())
    }

    // ---- ports ----

    fn port_names(&self) -> R<Vec<Ident>> {
        let m = self.inst.module;
        Ok(match &m.ports {
            Ports::None => Vec::new(),
            Ports::Ansi(v) => v
                .iter()
                .flat_map(|p| p.names.iter().map(|d| d.name.clone()))
                .collect(),
            Ports::NonAnsi(v) => {
                let mut out = Vec::new();
                for p in v {
                    match (&p.name, &p.expr) {
                        (Some(n), None) => out.push(n.clone()),
                        (Some(n), Some(Expr::Ident(i))) if n.name == i.name => out.push(n.clone()),
                        _ => {
                            return Err(unsupported(
                                "a port expression in a module header",
                                p.name.as_ref().map_or(self.at, |n| self.span_id(n)),
                            ));
                        }
                    }
                }
                out
            }
        })
    }

    fn dir_of(&self, n: &str) -> Option<Dir> {
        let m = self.inst.module;
        let ansi = match &m.ports {
            Ports::Ansi(v) => v.as_slice(),
            _ => &[],
        };
        let body = m.items.iter().filter_map(|i| match i {
            Item::Port(p) => Some(p),
            _ => None,
        });
        ansi.iter()
            .chain(body)
            .find(|p| p.names.iter().any(|d| d.name.name == n))
            .map(|p| p.dir)
    }

    fn bind_ports(&mut self) -> R<()> {
        let names = self.port_names()?;
        let m = self.inst.module;
        for p in self.inst.ports {
            let found = find(names.iter().map(|n| n.name.as_str()), &p.name);
            let n = match found {
                Ok(Some(n)) => n.to_string(),
                Ok(None) => {
                    let mut d = Diag::error(
                        "E4700",
                        format!(
                            "the port `{}` of the VHDL component has no port in module `{}`",
                            p.name, m.name.name
                        ),
                        self.at,
                    )
                    .note(format!("instance {}", self.inst.path));
                    if let Some(h) = yavs_diag::suggest::suggest(
                        &p.name,
                        "port",
                        names.iter().map(|n| (n.name.as_str(), n.name.as_str())),
                    ) {
                        d = d.help(h);
                    }
                    self.errors.push(d);
                    continue;
                }
                Err(many) => {
                    self.errors.push(Diag::error(
                        "E4700",
                        format!("the port `{}` matches several ports of module `{}` that differ only in case: {}", p.name, m.name.name, many.join(", ")),
                        self.at,
                    ));
                    continue;
                }
            };
            let Some(Sym::Var(k)) = self.syms.get(&n) else {
                return Err(Box::new(Diag::error(
                    "E4202",
                    format!(
                        "the port `{n}` of module `{}` has no declaration",
                        m.name.name
                    ),
                    self.at,
                )));
            };
            let k = *k;
            let v = &self.vars[k];
            let span = (self.spanf)(v.pos, v.pos + v.name.len() as u32);
            let dir = self.dir_of(&n);
            let ok_dir = match dir {
                Some(Dir::Input) => p.input,
                Some(Dir::Output) => !p.input,
                _ => false,
            };
            if !ok_dir {
                self.errors.push(
                    Diag::error(
                        "E4700",
                        format!(
                            "the port `{}` is {} in the VHDL component and {} in module `{}`",
                            p.name,
                            if p.input { "in" } else { "out" },
                            match dir {
                                Some(Dir::Input) => "an input",
                                Some(Dir::Output) => "an output",
                                _ => "no input or output",
                            },
                            m.name.name
                        ),
                        span,
                    )
                    .note(format!("instance {}", self.inst.path)),
                );
                continue;
            }
            if v.w() != p.width {
                self.errors.push(
                    Diag::error(
                        "E4700",
                        format!(
                            "the port `{}` has {} element(s) in the VHDL component and {} bit(s) in module `{}`",
                            p.name,
                            p.width,
                            v.w(),
                            m.name.name
                        ),
                        span,
                    )
                    .label(format!("[{}:{}]", v.msb, v.lsb))
                    .note(format!("instance {}", self.inst.path))
                    .note("a VHDL vector maps onto the Verilog vector left to left, so the widths must be equal"),
                );
                continue;
            }
            if p.input {
                self.d.inputs.push((p.key, v.net));
                self.driven[k].0.push((0, v.w(), "the VHDL port"));
            } else {
                self.d.outputs.push((p.key, v.net, p.bit));
            }
        }
        Ok(())
    }

    // ---- items ----

    fn items(&mut self) -> R<()> {
        let m = self.inst.module;
        let (mut assigns, mut always, mut initial) = (Vec::new(), Vec::new(), Vec::new());
        for it in &m.items {
            match it {
                Item::Port(_) | Item::Var(_) | Item::Param(_) | Item::Event(_) => {}
                // Declared but not called: a call is reported where it is.
                Item::Task(_) | Item::Function(_) => {}
                // ponytail: specify blocks are not evaluated (path delays are later work, D9).
                Item::Specify(_) => {}
                Item::Genvar(_) => {}
                Item::Net(n) => {
                    for d in &n.names {
                        if let Some(init) = &d.init {
                            assigns.push((Expr::Ident(d.name.clone()), init.clone()));
                        }
                    }
                }
                Item::Assign(a) => {
                    let at = a
                        .assigns
                        .first()
                        .map_or(self.at, |(l, _)| self.sx().span_of(l));
                    if a.strength.is_some() {
                        return Err(unsupported(
                            "a drive strength on a continuous assignment",
                            at,
                        ));
                    }
                    if a.delay.is_some() {
                        return Err(unsupported("a delay on a continuous assignment", at));
                    }
                    assigns.extend(a.assigns.iter().cloned());
                }
                Item::Always(s) => always.push(s),
                Item::Initial(s) => initial.push(s),
                Item::Gate(g) => {
                    return Err(unsupported(
                        &format!("the gate primitive `{}`", g.gate),
                        self.at,
                    ));
                }
                Item::Inst(i) => {
                    return Err(unsupported(
                        &format!("an instance of `{}` inside a Verilog module", i.module.name),
                        self.span_id(&i.module),
                    ));
                }
                Item::Defparam(_) => return Err(unsupported("`defparam`", self.at)),
                Item::Generate(_)
                | Item::GenBlock(_)
                | Item::GenIf { .. }
                | Item::GenFor { .. }
                | Item::GenCase { .. } => return Err(unsupported("a generate construct", self.at)),
                Item::ElabTask { name, .. } => {
                    return Err(unsupported(
                        &format!("the elaboration task `{name}`"),
                        self.at,
                    ));
                }
            }
        }
        // Continuous assignments first, then `always`, then `initial`: all of them start in
        // the Active region at time 0 in this order, so an `initial` write at time 0 is seen by
        // the `always` blocks waiting for it (the LRM leaves the order open, 9.9).
        for (l, r) in &assigns {
            self.assign(l, r)?;
        }
        for s in always {
            self.process(s, true)?;
        }
        for s in initial {
            self.process(s, false)?;
        }
        Ok(())
    }

    fn name(&mut self, kind: &'static str) -> String {
        let c = self.counts.entry(kind).or_default();
        *c += 1;
        format!("{}.{kind}{}", self.inst.path, *c)
    }

    fn assign(&mut self, l: &Expr, r: &Expr) -> R<()> {
        let lhs = self.lhs(l, false)?;
        let mut parts = Vec::new();
        lhs.nets(&mut parts);
        // An alias made in `nets` needs no process.
        if let (Lhs::Part { net, .. }, Expr::Ident(b)) = (&lhs, r)
            && let Some(Sym::Var(k)) = self.syms.get(&b.name)
            && self.vars[*k].net == *net
        {
            return Ok(());
        }
        let sx = self.sx();
        let rt = sx.ty(r)?;
        let ctx = Ty::vec(lhs.width().max(rt.w), rt.signed);
        let rhs = E::Ext(Box::new(sx.build(r, ctx)?), lhs.width(), rt.signed);
        let mut deps = Vec::new();
        reads(&rhs, &mut deps);
        lhs.reads(&mut deps);
        let name = self.name("assign");
        let ops = vec![
            Op::Set(lhs, rhs),
            Op::Wait(deps.into_iter().map(|n| (n, Edge::Any)).collect()),
            Op::Jmp(0),
        ];
        self.push(name, ops)?;
        Ok(())
    }

    fn push(&mut self, name: String, ops: Vec<Op>) -> R<()> {
        let code = Code {
            name,
            ops,
            inst: self.info.clone(),
        };
        let widths: Vec<u32> = self.d.nets.iter().map(|n| n.len() as u32).collect();
        match self.d.low.add(&code, &widths) {
            Ok(()) => Ok(()),
            Err(LowerError::Wide(w)) => Err(unsupported(
                &format!("a vector of {w} bits (more than {MAX_WIDTH}) in a process"),
                self.at,
            )),
            Err(LowerError::Bug(m)) => Err(Box::new(Diag::error(
                "E9999",
                format!(
                    "the process `{}` cannot be lowered: {m} (this is a bug in yavs)",
                    code.name
                ),
                self.at,
            ))),
        }
    }

    /// An assignment target: nets for a continuous assignment, variables in a process.
    fn lhs(&mut self, e: &Expr, procedural: bool) -> R<Lhs> {
        let sx = self.sx();
        let var = |i: &Ident| -> R<usize> {
            match self.syms.get(&i.name) {
                Some(Sym::Var(k)) => Ok(*k),
                Some(Sym::Param(_)) => Err(Box::new(Diag::error(
                    "E4507",
                    format!("the parameter `{}` cannot be assigned", i.name),
                    self.span_id(i),
                ))),
                None => Err(Box::new(
                    Diag::error(
                        "E4504",
                        format!("`{}` is not declared", i.name),
                        self.span_id(i),
                    )
                    .note(
                        "IEEE 1364-2005 12.7: a name is declared in the module before it is used",
                    ),
                )),
            }
        };
        let (k, start, n, i) = match e {
            Expr::Concat(parts) => {
                let mut v = Vec::new();
                for p in parts {
                    v.push(self.lhs(p, procedural)?);
                }
                return Ok(Lhs::Cat(v));
            }
            Expr::Ident(i) => {
                let k = var(i)?;
                (k, Start::Fixed(0), self.vars[k].w(), i)
            }
            Expr::Index(b, x) | Expr::Range(b, _, x, _) => {
                let Expr::Ident(i) = &**b else {
                    return Err(unsupported("this assignment target", sx.span_of(e)));
                };
                let k = var(i)?;
                let E::Sel {
                    idx,
                    idx_signed,
                    desc,
                    adj,
                    n,
                    ..
                } = sx.build(e, Ty::vec(sx.ty(e)?.w, false))?
                else {
                    unreachable!("a select builds a Sel")
                };
                let _ = x;
                let start = match &*idx {
                    E::K(v) => match v.to_i64(idx_signed) {
                        Some(kv) => {
                            let s = if desc { kv - adj } else { adj - kv };
                            let w = self.vars[k].w() as i64;
                            if s < 0 || s + n as i64 > w {
                                return Err(Box::new(Diag::error(
                                    "E4506",
                                    format!(
                                        "the select is outside `{}` [{}:{}]",
                                        i.name, self.vars[k].msb, self.vars[k].lsb
                                    ),
                                    sx.span_of(e),
                                )));
                            }
                            Start::Fixed(s as u32)
                        }
                        None => Start::Dyn {
                            idx: *idx,
                            signed: idx_signed,
                            desc,
                            adj,
                        },
                    },
                    _ => Start::Dyn {
                        idx: *idx,
                        signed: idx_signed,
                        desc,
                        adj,
                    },
                };
                (k, start, n, i)
            }
            _ => return Err(unsupported("this assignment target", sx.span_of(e))),
        };
        let v = &self.vars[k];
        let span = self.span_id(i);
        if procedural && !v.reg {
            return Err(Box::new(
                Diag::error("E4507", format!("the net `{}` is assigned in a procedural block", v.name), span)
                    .note("IEEE 1364-2005 9.2: a procedural assignment writes a variable (reg, integer, time)")
                    .help(format!("declare `{}` as a reg", v.name)),
            ));
        }
        if !procedural && v.reg {
            return Err(Box::new(
                Diag::error(
                    "E4507",
                    format!(
                        "the variable `{}` is the target of a continuous assignment",
                        v.name
                    ),
                    span,
                )
                .note("IEEE 1364-2005 6.1: a continuous assignment drives a net")
                .help(format!("declare `{}` as a wire", v.name)),
            ));
        }
        if procedural {
            self.driven[k].1 = true;
        } else {
            match &start {
                Start::Fixed(s) => self.driven[k].0.push((*s, n, "a continuous assignment")),
                Start::Dyn { .. } => {
                    return Err(Box::new(Diag::error(
                        "E4506",
                        format!("the select of `{}` in a continuous assignment is not constant", v.name),
                        span,
                    ).note("IEEE 1364-2005 6.1.2: the target of a continuous assignment has constant selects")));
                }
            }
        }
        Ok(Lhs::Part {
            net: v.net,
            start,
            n,
        })
    }

    /// Bits of a net driven twice: the domain does not resolve drivers yet.
    fn check_drivers(&mut self) {
        for (k, (cont, _)) in self.driven.iter().enumerate() {
            for (i, a) in cont.iter().enumerate() {
                if let Some(b) = cont[i + 1..]
                    .iter()
                    .find(|b| a.0 < b.0 + b.1 && b.0 < a.0 + a.1)
                {
                    let v = &self.vars[k];
                    self.errors.push(
                        Diag::error(
                            "E4502",
                            format!("the net `{}` has more than one driver: {} and {}", v.name, a.2, b.2),
                            (self.spanf)(v.pos, v.pos + v.name.len() as u32),
                        )
                        .note("a net with several drivers needs resolution, which Verilog elaboration does not implement"),
                    );
                    break;
                }
            }
        }
    }

    fn process(&mut self, s: &Stmt, always: bool) -> R<()> {
        let kind = if always { "always" } else { "initial" };
        let name = self.name(kind);
        let mut c = Cc {
            ops: Vec::new(),
            slots: 0,
            timed: false,
        };
        self.stmt(&mut c, s)?;
        if always {
            if !c.timed {
                return Err(Box::new(
                    Diag::error("E4503", format!("the `always` block `{name}` has no timing control"), self.stmt_at(s))
                        .note("IEEE 1364-2005 9.9.2: an always block without a delay or event control loops at one time for ever"),
                ));
            }
            c.ops.push(Op::Jmp(0));
        } else {
            c.ops.push(Op::Done);
        }
        self.push(name, c.ops)?;
        Ok(())
    }

    fn stmt_at(&self, s: &Stmt) -> Span {
        let sx = self.sx();
        match s {
            Stmt::Assign { lhs, .. } => sx.span_of(lhs),
            Stmt::If { cond, .. } => sx.span_of(cond),
            Stmt::Case { expr, .. } => sx.span_of(expr),
            Stmt::Block { label: Some(l), .. } => self.span_id(l),
            Stmt::Block { stmts, .. } => stmts.first().map_or(self.at, |s| self.stmt_at(s)),
            Stmt::Event { body, .. } | Stmt::Delay { body, .. } => self.stmt_at(body),
            _ => self.at,
        }
    }

    fn stmt(&mut self, c: &mut Cc, s: &Stmt) -> R<()> {
        match s {
            Stmt::Null => {}
            Stmt::Block {
                par, decls, stmts, ..
            } => {
                if *par {
                    return Err(unsupported("`fork ... join`", self.stmt_at(s)));
                }
                if !decls.is_empty() {
                    return Err(unsupported(
                        "declarations in a named block",
                        self.stmt_at(s),
                    ));
                }
                for s in stmts {
                    self.stmt(c, s)?;
                }
            }
            Stmt::Assign {
                lhs,
                rhs,
                nonblocking,
                ctrl,
            } => {
                let target = self.lhs(lhs, true)?;
                let sx = self.sx();
                let rt = sx.ty(rhs)?;
                let w = target.width();
                let value = if rt.real {
                    sx.build(rhs, Ty::vec(w, true))?
                } else {
                    let ctx = Ty::vec(w.max(rt.w), rt.signed);
                    E::Ext(Box::new(sx.build(rhs, ctx)?), w, rt.signed)
                };
                let delay = match ctrl {
                    None => None,
                    Some(Ctrl::Delay(d)) => Some(self.delay(d)?),
                    Some(_) => {
                        return Err(unsupported(
                            "an event control inside an assignment",
                            sx.span_of(lhs),
                        ));
                    }
                };
                match (nonblocking, delay) {
                    (true, d) => c.ops.push(Op::Nba(target, value, d.map(|d| d.0))),
                    (false, None) => c.ops.push(Op::Set(target, value)),
                    (false, Some((d, r))) => {
                        // `a = #d b`: b now, a after the delay (9.7.7).
                        let k = c.slots;
                        c.slots += 1;
                        c.timed = true;
                        c.ops.push(Op::Hold(k, value));
                        c.ops.push(Op::Delay(d, r));
                        c.ops.push(Op::SetHeld(target, k));
                    }
                }
            }
            Stmt::If { cond, then, els } => {
                let e = self.sx().cond(cond)?;
                let j = c.ops.len();
                c.ops.push(Op::Jmp(0));
                self.stmt(c, then)?;
                match els {
                    None => {
                        let end = c.here();
                        c.ops[j] = Op::JmpIfNot(e, end);
                    }
                    Some(els) => {
                        let skip = c.ops.len();
                        c.ops.push(Op::Jmp(0));
                        let at = c.here();
                        c.ops[j] = Op::JmpIfNot(e, at);
                        self.stmt(c, els)?;
                        let end = c.here();
                        c.ops[skip] = Op::Jmp(end);
                    }
                }
            }
            Stmt::Case { kind, expr, items } => self.case(c, kind, expr, items)?,
            Stmt::For {
                init,
                cond,
                step,
                body,
            } => {
                self.stmt(c, &assign_stmt(&init.0, &init.1))?;
                let top = c.here();
                let e = self.sx().cond(cond)?;
                let j = c.ops.len();
                c.ops.push(Op::Jmp(0));
                self.stmt(c, body)?;
                self.stmt(c, &assign_stmt(&step.0, &step.1))?;
                c.ops.push(Op::Jmp(top));
                let end = c.here();
                c.ops[j] = Op::JmpIfNot(e, end);
            }
            Stmt::While { cond, body } => {
                let top = c.here();
                let e = self.sx().cond(cond)?;
                let j = c.ops.len();
                c.ops.push(Op::Jmp(0));
                self.stmt(c, body)?;
                c.ops.push(Op::Jmp(top));
                let end = c.here();
                c.ops[j] = Op::JmpIfNot(e, end);
            }
            Stmt::Repeat { count, body } => {
                let (e, t) = self.sx().self_det(count)?;
                let k = c.slots;
                c.slots += 1;
                c.ops.push(Op::RepeatInit(k, e, t.signed));
                let top = c.here();
                c.ops.push(Op::RepeatTest(k, 0));
                self.stmt(c, body)?;
                c.ops.push(Op::Jmp(top));
                let end = c.here();
                c.ops[top as usize] = Op::RepeatTest(k, end);
            }
            Stmt::Forever(body) => {
                let top = c.here();
                self.stmt(c, body)?;
                c.ops.push(Op::Jmp(top));
            }
            Stmt::Wait { cond, body } => {
                // `wait (c)`: on at once when c holds, else wait for a change of what it reads.
                let top = c.here();
                let e = self.sx().cond(cond)?;
                let mut deps = Vec::new();
                reads(&e, &mut deps);
                c.ops.push(Op::JmpIfNot(e, top + 2));
                c.ops.push(Op::Jmp(top + 4));
                c.ops
                    .push(Op::Wait(deps.into_iter().map(|n| (n, Edge::Any)).collect()));
                c.ops.push(Op::Jmp(top));
                c.timed = true;
                self.stmt(c, body)?;
            }
            Stmt::Delay { delay, body } => {
                let (d, r) = self.delay(delay)?;
                c.ops.push(Op::Delay(d, r));
                c.timed = true;
                self.stmt(c, body)?;
            }
            Stmt::Event { ctrl, body } => {
                let items = match ctrl {
                    EventCtrl::Star => {
                        let mut sub = Cc {
                            ops: Vec::new(),
                            slots: c.slots,
                            timed: false,
                        };
                        self.stmt(&mut sub, body)?;
                        let mut deps = Vec::new();
                        for op in &sub.ops {
                            op_reads(op, &mut deps);
                        }
                        deps.into_iter().map(|n| (n, Edge::Any)).collect()
                    }
                    EventCtrl::List(list) => {
                        let mut v = Vec::new();
                        for ev in list {
                            let Expr::Ident(i) = &ev.expr else {
                                return Err(unsupported(
                                    "an event control on something other than a name",
                                    self.sx().span_of(&ev.expr),
                                ));
                            };
                            let net = match self.syms.get(&i.name) {
                                Some(Sym::Var(k)) => self.vars[*k].net,
                                _ => {
                                    return Err(Box::new(Diag::error(
                                        "E4504",
                                        format!("`{}` is not a net or variable", i.name),
                                        self.span_id(i),
                                    )));
                                }
                            };
                            let edge = match ev.edge {
                                Some("posedge") => Edge::Pos,
                                Some("negedge") => Edge::Neg,
                                _ => Edge::Any,
                            };
                            v.push((net, edge));
                        }
                        v
                    }
                };
                c.ops.push(Op::Wait(items));
                c.timed = true;
                self.stmt(c, body)?;
            }
            Stmt::SysCall { name, args } => match name.as_str() {
                "$display" | "$write" => {
                    let sx = self.sx();
                    let mut out = Vec::new();
                    for a in args {
                        out.push(match a {
                            Expr::Str(s) => Arg::Fmt(crate::sem::string_bytes(s)),
                            Expr::Empty => Arg::Fmt(b" ".to_vec()),
                            _ => {
                                let (e, t) = sx.self_det(a)?;
                                if t.real {
                                    Arg::Real(e)
                                } else {
                                    Arg::Val(e, t.signed)
                                }
                            }
                        });
                    }
                    c.ops.push(Op::Display(out, name == "$display"));
                }
                "$finish" => c.ops.push(Op::Finish),
                _ => return Err(unsupported(&format!("the system task `{name}`"), self.at)),
            },
            Stmt::ProcAssign { kind, lhs, .. } => {
                return Err(unsupported(
                    &format!("procedural `{kind}`"),
                    self.sx().span_of(lhs),
                ));
            }
            Stmt::Trigger(e) => return Err(unsupported("an event trigger", self.sx().span_of(e))),
            Stmt::Disable(e) => return Err(unsupported("`disable`", self.sx().span_of(e))),
            Stmt::TaskCall { name, .. } => {
                return Err(unsupported("a task call", self.sx().span_of(name)));
            }
        }
        Ok(())
    }

    fn case(
        &mut self,
        c: &mut Cc,
        kind: &str,
        expr: &Expr,
        items: &[crate::ast::CaseItem],
    ) -> R<()> {
        let kind = match kind {
            "casez" => CaseKind::Casez,
            "casex" => CaseKind::Casex,
            _ => CaseKind::Case,
        };
        let sx = self.sx();
        // All expressions at the largest width, signed only if all are (9.5).
        let mut t = sx.ty(expr)?;
        for it in items {
            for e in &it.exprs {
                let u = sx.ty(e)?;
                t = Ty::vec(t.w.max(u.w), t.signed && u.signed);
            }
        }
        if t.real {
            return Err(unsupported("a real case expression", sx.span_of(expr)));
        }
        let sel = sx.build(expr, t)?;
        let mut arms: Vec<(Vec<E>, &Stmt)> = Vec::new();
        let mut default = None;
        for it in items {
            if it.exprs.is_empty() {
                default = Some(&it.stmt);
            } else {
                let es = it
                    .exprs
                    .iter()
                    .map(|e| sx.build(e, t))
                    .collect::<R<Vec<_>>>()?;
                arms.push((es, &it.stmt));
            }
        }
        let at = c.ops.len();
        c.ops.push(Op::Jmp(0));
        let mut targets = Vec::new();
        let mut ends = Vec::new();
        for (es, s) in arms {
            let here = c.here();
            for e in es {
                targets.push((e, here));
            }
            self.stmt(c, s)?;
            ends.push(c.ops.len());
            c.ops.push(Op::Jmp(0));
        }
        let dflt = c.here();
        if let Some(s) = default {
            self.stmt(c, s)?;
        }
        let end = c.here();
        for e in ends {
            c.ops[e] = Op::Jmp(end);
        }
        c.ops[at] = Op::Case {
            sel,
            kind,
            items: targets,
            default: dflt,
        };
        Ok(())
    }

    /// A delay value in the module's unit; the flag: real.
    fn delay(&self, d: &Delay) -> R<(E, bool)> {
        let Some(e) = d.0.first().filter(|_| d.0.len() == 1) else {
            return Err(unsupported(
                "a delay with several values in a procedural statement",
                self.at,
            ));
        };
        let (n, t) = self.sx().self_det(e)?;
        Ok((n, t.real))
    }
}

/// The process being compiled.
struct Cc {
    ops: Vec<Op>,
    slots: u32,
    /// It has a delay or event control (an `always` block without one never yields).
    timed: bool,
}

impl Cc {
    fn here(&self) -> u32 {
        self.ops.len() as u32
    }
}

fn assign_stmt(l: &Expr, r: &Expr) -> Stmt {
    Stmt::Assign {
        lhs: l.clone(),
        rhs: r.clone(),
        nonblocking: false,
        ctrl: None,
    }
}

fn lhs_names<'e>(e: &'e Expr, out: &mut Vec<&'e str>) {
    match e {
        Expr::Ident(i) => out.push(&i.name),
        Expr::Index(b, _)
        | Expr::Range(b, RangeKind::Colon | RangeKind::PlusColon | RangeKind::MinusColon, _, _) => {
            lhs_names(b, out)
        }
        Expr::Concat(v) => v.iter().for_each(|p| lhs_names(p, out)),
        _ => {}
    }
}

/// The nets an `@*` block reads (9.7.5): every expression of its code, and the indices of its
/// targets, but not the targets themselves.
fn op_reads(op: &Op, out: &mut Vec<u32>) {
    match op {
        Op::Set(l, e) | Op::Nba(l, e, None) => {
            reads(e, out);
            l.reads(out);
        }
        Op::Nba(l, e, Some(d)) => {
            reads(e, out);
            reads(d, out);
            l.reads(out);
        }
        Op::Hold(_, e) | Op::JmpIfNot(e, _) | Op::RepeatInit(_, e, _) | Op::Delay(e, _) => {
            reads(e, out)
        }
        Op::SetHeld(l, _) => l.reads(out),
        Op::Case { sel, items, .. } => {
            reads(sel, out);
            items.iter().for_each(|(e, _)| reads(e, out));
        }
        Op::Display(args, _) => {
            for a in args {
                if let Arg::Val(e, _) | Arg::Real(e) = a {
                    reads(e, out);
                }
            }
        }
        Op::Jmp(_) | Op::RepeatTest(..) | Op::Wait(_) | Op::Finish | Op::Done => {}
    }
}

/// A generic's value at its own type: an integer is 32 bits signed, a string 8 bits a
/// character, a logic value as wide as it is.
fn from_generic(v: &ParamValue) -> Const {
    match v {
        ParamValue::Int(i) => Const::V(V4::from_i64(32, *i), true),
        ParamValue::Real(r) => Const::R(*r),
        ParamValue::Str(s) => Const::V(string_value(s), false),
        ParamValue::Logic(l) => Const::V(V4::from_logic(l), false),
    }
}

/// A value converted to a parameter's declared type (12.2). A type of width 0 is `signed`
/// without a range: the value's width.
fn convert(v: Const, t: Ty) -> Const {
    match (v, t.real) {
        (Const::R(r), true) => Const::R(r),
        (Const::V(v, s), true) => Const::R(crate::exec::to_f64(&v, s)),
        (Const::R(r), false) => {
            let w = if t.w == 0 { 32 } else { t.w };
            Const::V(V4::from_i64(64, real_to_i64(r)).ext(w, true), t.signed)
        }
        (Const::V(v, s), false) => {
            let w = if t.w == 0 { v.width() } else { t.w };
            Const::V(v.ext(w, s), t.signed)
        }
    }
}

impl Source {
    /// The module or primitive declared under `name`.
    pub fn decl(&self, name: &str) -> Option<&Decl> {
        self.decls.iter().find(|d| match d {
            Decl::Module(m) => m.name.name == name,
            Decl::Udp(u) => u.name.name == name,
            Decl::Envelope(_) => false,
        })
    }
}
