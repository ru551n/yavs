//! Inlining of thin subprograms: a call of a function whose body is straight-line code that
//! reads its parameters, computes and calls one or two more (the package function in front of
//! a protected method, an accessor) becomes that code in the caller.
//!
//! A call costs about as much as ten operations (frame, argument slots, return), and a library
//! testbench is a chain of such calls. The pass runs last, on the finished code of a
//! subprogram (after the other passes and the renumbering), and takes the callee from its own
//! optimised code, made apart from the callee's published copy (`forwarder`), so the result
//! does not depend on which function was optimised first.
//!
//! What stays as it was: the callee's checks and calls, in their order, and what a trace shows.
//! An inlined instruction keeps the statement of the callee and `Func::via` names the call it
//! came from, so a runtime error lists the callee as a frame of its own (`Machine::trace`).
//! Callees that allocate temporaries, have statements of their own to free (their `Stmt`
//! markers go: a new frame has no temporaries to free), touch their frame beyond reading the
//! parameters, branch, suspend or are nested are not inlined. `YAVS_NO_INLINE=1` turns it off.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use crate::{AddrSpace, FuncId, FuncKind, Inst, Reg, RegTy, Role, StmtId, Width};

/// The most instructions of an inlined body.
const MAX_BODY: usize = 24;

/// The part of a callee that is inlined: its straight-line code with the parameter loads taken
/// out.
pub(crate) struct Forwarder {
    /// The instructions, without frame addresses, parameter loads and statement markers; the
    /// last is the `Ret`.
    body: Vec<(Inst, StmtId)>,
    /// The callee's operand lists.
    operands: Vec<Reg>,
    reg_ty: Vec<RegTy>,
    /// For each parameter the registers its loads defined.
    params: Vec<Vec<Reg>>,
    /// How many instructions of the body define each register.
    defs: Vec<u8>,
}

/// Forwarders by callee, made once per program (a copy of the program starts without them).
#[derive(Default, Debug)]
pub struct FwdCache(Mutex<HashMap<u32, Option<Arc<Forwarder>>>>);

impl Clone for FwdCache {
    fn clone(&self) -> Self {
        Self::default()
    }
}

impl std::fmt::Debug for Forwarder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Forwarder({} instructions)", self.body.len())
    }
}

/// Is inlining on? `YAVS_NO_INLINE` set to anything but `0` or the empty string turns it off.
pub(super) fn enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| {
        std::env::var("YAVS_NO_INLINE")
            .map(|v| v.is_empty() || v == "0")
            .unwrap_or(true)
    })
}

/// The inlinable body of `callee`, if it has one.
fn forwarder(prog: &crate::Program, callee: FuncId) -> Option<Arc<Forwarder>> {
    if let Some(r) = prog
        .fwd
        .0
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(&callee.0)
    {
        return r.clone();
    }
    // Not under the lock: the optimiser takes the program's other locks.
    let made = make(prog, callee).map(Arc::new);
    prog.fwd
        .0
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(callee.0, made.clone());
    made
}

fn make(prog: &crate::Program, callee: FuncId) -> Option<Forwarder> {
    let raw = prog.funcs.get(callee)?;
    // Cheap exits before optimising a function: not a small subprogram of the plain kind.
    if !matches!(raw.kind, FuncKind::Function | FuncKind::Procedure)
        || raw.nested
        || raw.parent.is_some()
        || raw.body.is_some()
        || raw.flags.contains(crate::FuncFlags::MAY_SUSPEND)
        || raw.flags.contains(crate::FuncFlags::HAS_COVER)
        || raw
            .roots
            .iter()
            .any(|&(off, _)| !raw.params.iter().any(|p| p.slot == off))
        || raw
            .params
            .iter()
            .any(|p| p.width != Width::B8 || p.width == Width::Sig)
        || raw.full().code.len() > 4 * MAX_BODY
    {
        return None;
    }
    let c = super::optimize_body(prog, callee)?;
    if c.reg_ty.contains(&RegTy::Sig) {
        return None;
    }
    // One path: the first block ends in the `Ret`, no branches, nothing else reachable.
    let first = c.blocks.get(crate::BlockId(0))?;
    let code = c.code.get(first.start as usize..first.end as usize)?;
    if !matches!(code.last(), Some(Inst::Ret { .. }))
        || code
            .iter()
            .any(|i| matches!(i, Inst::Br { .. } | Inst::CondBr { .. } | Inst::Case { .. }))
        || first.start != 0
    {
        return None;
    }
    // The registers holding the address of a parameter's slot.
    let mut frame: HashMap<u32, u32> = HashMap::new();
    for i in code {
        if let Inst::AddrOf {
            dst,
            space: AddrSpace::Frame,
            off,
        } = *i
        {
            frame.insert(dst.0, off);
        }
    }
    let slot = |off: u32| c.params.iter().position(|p| p.slot == off);
    let mut params: Vec<Vec<Reg>> = vec![Vec::new(); c.params.len()];
    let mut body: Vec<(Inst, StmtId)> = Vec::new();
    for (k, i) in code.iter().enumerate() {
        let loc = *c.loc.get(first.start as usize + k)?;
        match *i {
            Inst::Stmt => continue,
            Inst::AddrOf {
                space: AddrSpace::Frame,
                ..
            } => continue,
            Inst::Load { dst, addr, w } if frame.contains_key(&addr.0) => {
                let p = slot(*frame.get(&addr.0)?)?;
                if w != Width::B8 {
                    return None;
                }
                params[p].push(dst);
                continue;
            }
            _ => {}
        }
        if !allowed(prog, i) {
            return None;
        }
        // The frame is touched by the parameter loads only.
        let mut touches = false;
        i.visit(&mut |r, role, _| touches |= role == Role::Use && frame.contains_key(&r.0));
        if let Some(l) = i.list() {
            touches |= c
                .operands
                .get(l.start as usize..(l.start + l.len) as usize)?
                .iter()
                .any(|r| frame.contains_key(&r.0));
        }
        if touches {
            return None;
        }
        body.push((*i, loc));
    }
    if body.len() > MAX_BODY || !matches!(body.last(), Some((Inst::Ret { .. }, _))) {
        return None;
    }
    let mut defs = vec![0u8; c.reg_ty.len()];
    for (i, _) in &body {
        i.visit(&mut |r, role, _| {
            if role == Role::Def
                && let Some(d) = defs.get_mut(r.0 as usize)
            {
                *d = d.saturating_add(1);
            }
        });
    }
    Some(Forwarder {
        body,
        operands: c.operands.clone(),
        reg_ty: c.reg_ty.clone(),
        params,
        defs,
    })
}

/// Instructions an inlined body may have: reads, computations, checks, calls.
fn allowed(prog: &crate::Program, i: &Inst) -> bool {
    match *i {
        Inst::Const { .. }
        | Inst::ConstF { .. }
        | Inst::Move { .. }
        | Inst::Select { .. }
        | Inst::Index { .. }
        | Inst::Field { .. }
        | Inst::Bin { .. }
        | Inst::BinRange { .. }
        | Inst::Un { .. }
        | Inst::Cmp { .. }
        | Inst::Bool { .. }
        | Inst::Check { .. }
        | Inst::Pos { .. }
        | Inst::DopeLen { .. }
        | Inst::Deref { .. }
        | Inst::Ret { .. } => true,
        Inst::AddrOf { space, .. } => !matches!(space, AddrSpace::Frame | AddrSpace::Up(_)),
        Inst::Load { w, .. } | Inst::Store { w, .. } => w != Width::Sig,
        Inst::Call { func, .. } | Inst::CallProt { func, .. } => prog
            .funcs
            .get(func)
            .is_some_and(|c| !c.flags.contains(crate::FuncFlags::MAY_SUSPEND)),
        _ => false,
    }
}

/// Inline the calls of `f` (function `id`) whose callee has a forwarder. `true`: the code
/// changed.
pub(super) fn run(prog: &crate::Program, id: FuncId, f: &mut crate::Func) -> bool {
    if !enabled()
        || super::keep_variables_on()
        || !matches!(f.kind, FuncKind::Function | FuncKind::Procedure)
    {
        return false;
    }
    // The calls to inline and their callees.
    let mut sites: HashMap<usize, Arc<Forwarder>> = HashMap::new();
    for (pc, i) in f.code.iter().enumerate() {
        let Inst::Call { func, args, dst } = *i else {
            continue;
        };
        if func == id {
            continue;
        }
        let Some(fw) = forwarder(prog, func) else {
            continue;
        };
        let Some(callee) = prog.funcs.get(func) else {
            continue;
        };
        // One argument for each parameter, the result in a register of the type it is returned in.
        let ret = match fw.body.last() {
            Some((Inst::Ret { v }, _)) => *v,
            _ => continue,
        };
        let arg_ty = |k: usize| {
            f.operands
                .get(args.start as usize + k)
                .and_then(|r| f.reg_ty.get(r.0 as usize))
        };
        // The arguments are registers of the types the parameter loads define.
        let typed = fw.params.iter().enumerate().all(|(k, regs)| {
            regs.iter()
                .all(|r| arg_ty(k).is_some() && arg_ty(k) == fw.reg_ty.get(r.0 as usize))
        });
        let fits = typed
            && args.len as usize == callee.params.len()
            && match (dst, ret) {
                (Some(d), Some(r)) => {
                    f.reg_ty.get(d.0 as usize).is_some()
                        && f.reg_ty.get(d.0 as usize) == fw.reg_ty.get(r.0 as usize)
                }
                (None, _) => true,
                (Some(_), None) => false,
            };
        if fits {
            sites.insert(pc, fw);
        }
    }
    if sites.is_empty() {
        return false;
    }
    let mut code: Vec<Inst> = Vec::with_capacity(f.code.len() + 8 * sites.len());
    let mut loc: Vec<StmtId> = Vec::with_capacity(code.capacity());
    let mut via: Vec<Option<(StmtId, FuncId)>> = Vec::with_capacity(code.capacity());
    let mut blocks = Vec::with_capacity(f.blocks.len());
    let mut reg_ty = f.reg_ty.clone();
    let mut operands = f.operands.clone();
    let old_via = |pc: usize| f.via.get(pc).copied().flatten();
    for (_, b) in f.blocks.iter() {
        let start = code.len() as u32;
        for pc in b.start as usize..b.end as usize {
            let Some(fw) = sites.get(&pc) else {
                code.push(f.code[pc]);
                loc.push(f.loc[pc]);
                via.push(old_via(pc));
                continue;
            };
            let Inst::Call { func, args, dst } = f.code[pc] else {
                // Not reachable: the sites are calls. Leave the function as it was.
                return false;
            };
            let site = f.loc[pc];
            let arg = |k: usize| f.operands[args.start as usize + k];
            let args_of = |k: usize| arg(k);
            // Where each register of the body lives in this function: a parameter the body
            // never writes is the argument itself, the returned value is made in the result
            // register when nothing else is, any other register is a new one.
            let mut at: HashMap<u32, Reg> = HashMap::new();
            for (k, regs) in fw.params.iter().enumerate() {
                for &r in regs {
                    if fw.defs[r.0 as usize] == 0 {
                        at.insert(r.0, args_of(k));
                    }
                }
            }
            let ret = match fw.body.last() {
                Some((Inst::Ret { v }, _)) => *v,
                _ => None,
            };
            let mut ret_moved = None;
            if let (Some(d), Some(r)) = (dst, ret) {
                let is_arg = (0..args.len as usize).any(|k| arg(k) == d);
                let is_param = fw.params.iter().any(|p| p.contains(&r));
                if fw.defs[r.0 as usize] == 1 && !is_param && !is_arg {
                    at.insert(r.0, d);
                } else {
                    ret_moved = Some((d, r));
                }
            }
            let mut used: Vec<u32> = Vec::new();
            for (inst, _) in &fw.body {
                inst.visit(&mut |r, _, _| used.push(r.0));
                if let Some(l) = inst.list() {
                    used.extend(
                        fw.operands[l.start as usize..(l.start + l.len) as usize]
                            .iter()
                            .map(|r| r.0),
                    );
                }
            }
            // The registers the parameter loads defined and the body reads.
            used.sort_unstable();
            used.dedup();
            for r in used {
                if let std::collections::hash_map::Entry::Vacant(e) = at.entry(r) {
                    e.insert(Reg(reg_ty.len() as u32));
                    reg_ty.push(fw.reg_ty[r as usize]);
                }
            }
            let map = |r: Reg| at.get(&r.0).copied().unwrap_or(r);
            // The other parameters arrive in the registers the loads defined.
            for (k, regs) in fw.params.iter().enumerate() {
                for &r in regs {
                    if at.get(&r.0) != Some(&arg(k)) && at.contains_key(&r.0) {
                        code.push(Inst::Move {
                            dst: map(r),
                            src: arg(k),
                        });
                        loc.push(site);
                        via.push(None);
                    }
                }
            }
            for &(inst, l) in &fw.body {
                let mut inst = inst;
                if let Inst::Ret { .. } = inst {
                    if let Some((d, r)) = ret_moved {
                        code.push(Inst::Move {
                            dst: d,
                            src: map(r),
                        });
                        loc.push(site);
                        via.push(None);
                    }
                    continue;
                }
                inst.visit_mut(&mut |r, _, _| *r = map(*r));
                match &mut inst {
                    Inst::Call { args, .. } | Inst::CallProt { args, .. } => {
                        let start = operands.len() as u32;
                        let from = args.start as usize..(args.start + args.len) as usize;
                        operands.extend(fw.operands[from].iter().map(|&r| map(r)));
                        args.start = start;
                    }
                    _ => {}
                }
                code.push(inst);
                loc.push(l);
                via.push(Some((site, func)));
            }
        }
        blocks.push((start, code.len() as u32));
    }
    for (k, (s, e)) in blocks.into_iter().enumerate() {
        if let Some(b) = f.blocks.get_mut(crate::BlockId(k as u32)) {
            b.start = s;
            b.end = e;
        }
    }
    f.code = code;
    f.loc = loc;
    f.via = via;
    f.reg_ty = reg_ty;
    f.operands = operands;
    true
}
