//! The IR verifier: one linear walk over every function (ir.md 1.2).
//!
//! Its errors are internal compiler errors: malformed IR is a bug in the producer (lowering,
//! elaboration or a hand-written test), never a user error.

use std::fmt;

use crate::inst::Role;
use crate::{AddrSpace, ConstData, FuncFlags, FuncId, FuncKind, Inst, Program, RegTy};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifyError {
    pub func: FuncId,
    pub name: String,
    pub pc: Option<u32>,
    pub msg: String,
}

impl VerifyError {
    pub(crate) fn new(func: FuncId, name: &str, pc: Option<u32>, msg: impl Into<String>) -> Self {
        VerifyError {
            func,
            name: name.to_string(),
            pc,
            msg: msg.into(),
        }
    }
}

impl fmt::Display for VerifyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "IR verifier: {} `{}`", self.func, self.name)?;
        if let Some(pc) = self.pc {
            write!(f, " at {pc}")?;
        }
        write!(f, ": {}", self.msg)
    }
}

impl std::error::Error for VerifyError {}

/// Verify `prog`, its functions from id `first` on.
pub(crate) fn verify(prog: &Program, first: usize) -> Vec<VerifyError> {
    let mut errs = Vec::new();
    for (id, f) in prog.funcs.iter().skip(first).filter(|(_, f)| !f.spare) {
        verify_func(prog, id, f.full(), &mut errs);
    }
    for (id, t) in prog.cases.iter() {
        let sorted = t.arms.windows(2).all(|w| w[0].1 < w[1].0);
        if !sorted || t.arms.iter().any(|a| a.0 > a.1) {
            errs.push(VerifyError::new(
                FuncId(u32::MAX),
                "<program>",
                None,
                format!("{id}: choices not sorted, overlapping or empty"),
            ));
        }
    }
    errs
}

pub(crate) fn verify_func(
    prog: &Program,
    id: FuncId,
    f: &crate::Func,
    errs: &mut Vec<VerifyError>,
) {
    let mut err = |pc: Option<usize>, m: String| {
        errs.push(VerifyError::new(id, &f.name, pc.map(|p| p as u32), m));
    };
    let n = f.code.len();
    if f.loc.len() != n {
        err(
            None,
            format!("{} locations for {n} instructions", f.loc.len()),
        );
        return;
    }
    if !f.via.is_empty() && f.via.len() != n {
        err(
            None,
            format!("{} inlining records for {n} instructions", f.via.len()),
        );
        return;
    }
    if f.blocks.is_empty() {
        err(None, "no blocks".into());
        return;
    }
    // Blocks: contiguous, non-empty, terminator last and only last.
    let mut next = 0u32;
    for (b, blk) in f.blocks.iter() {
        if blk.start != next || blk.end <= blk.start {
            err(None, format!("block {b} is empty or not contiguous"));
            return;
        }
        next = blk.end;
        for pc in blk.start..blk.end {
            let last = pc + 1 == blk.end;
            let term = f.code[pc as usize].is_terminator();
            if term != last {
                let m = if last {
                    format!("block {b} does not end with a terminator")
                } else {
                    format!("terminator in the middle of block {b}")
                };
                err(Some(pc as usize), m);
            }
        }
    }
    if next as usize != n {
        err(None, "blocks do not cover the code".into());
    }
    let frame = f.frame;
    for p in &f.params {
        if p.slot + p.width.bytes() > frame {
            err(None, format!("parameter {} outside the frame", p.name));
        }
    }
    for &(off, _) in &f.roots {
        if off + 8 > frame {
            err(None, format!("root at {off} outside the frame"));
        }
    }
    if f.kind == FuncKind::Process && !f.params.is_empty() {
        err(None, "a process has no parameters".into());
    }
    let mut depth = 0u8;
    let mut p = f.parent;
    while let Some(q) = p {
        depth = depth.saturating_add(1);
        p = prog.funcs.get(q).and_then(|q| q.parent);
        if depth == u8::MAX {
            err(None, "static link chain is cyclic".into());
            break;
        }
    }
    let nregs = f.reg_ty.len();
    // Per register: (block, suspension epoch) of its last definition.
    let mut def: Vec<(u32, u32)> = vec![(u32::MAX, 0); nregs];
    let is_fn = matches!(
        f.kind,
        FuncKind::Function | FuncKind::Elab | FuncKind::Thunk
    );
    // Debug builds and tests only: the dataflow costs about 30 ms on a large verification
    // library, and nothing depends on it until frames start with stale registers (#430).
    if cfg!(debug_assertions) {
        for (pc, r) in undefined_uses(prog, f) {
            err(
                Some(pc as usize),
                format!("r{r} is read before it is written on some path"),
            );
        }
    }
    for (b, blk) in f.blocks.iter() {
        let mut epoch = 0u32;
        for pc in blk.start as usize..blk.end as usize {
            let inst = &f.code[pc];
            let s = f.loc[pc];
            // An inlined instruction has the statement of the function it came from, and the
            // statement of each call it was inlined through is one of the function that
            // contained the call: the next callee outward, this function for the last.
            let chain: Vec<crate::ViaNode> = f.inlined_chain(pc).collect();
            for (k, n) in chain.iter().enumerate() {
                let outer = chain.get(k + 1).map_or(id, |o| o.callee);
                if prog.stmts.get(n.site).is_none_or(|i| i.func != outer) {
                    err(
                        Some(pc),
                        format!("inlined call statement {} is not of {outer}", n.site.0),
                    );
                }
            }
            let own = chain.first().map_or(id, |n| n.callee);
            match prog.stmts.get(s) {
                Some(info) if info.func == own => {}
                Some(_) => err(
                    Some(pc),
                    format!("statement {} belongs to another function", s.0),
                ),
                None => err(Some(pc), format!("no statement {}", s.0)),
            }
            let mut uses = Vec::new();
            inst.visit(&mut |r, role, want| {
                let ok = match f.reg_ty.get(r.0 as usize) {
                    None => false,
                    Some(&t) => match want {
                        Some(RegTy::Sig) => t == RegTy::Sig && (r.0 as usize + 1) < nregs,
                        Some(RegTy::I64) => t == RegTy::I64 || t == RegTy::Handle,
                        Some(w) => t == w,
                        None => t != RegTy::Sig,
                    },
                };
                if !ok {
                    let have = f.reg_ty.get(r.0 as usize);
                    uses.push(Err(format!("{r:?} is {have:?}, expected {want:?}")));
                } else {
                    uses.push(Ok((r, role)));
                }
            });
            if let Some(l) = inst.list() {
                match f.regs(l) {
                    None => uses.push(Err("operand list out of range".into())),
                    Some(rs) => {
                        for &r in rs {
                            if (r.0 as usize) < nregs {
                                uses.push(Ok((r, Role::Use)));
                            } else {
                                uses.push(Err(format!("{r:?} out of range")));
                            }
                        }
                    }
                }
            }
            for u in uses {
                match u {
                    Err(m) => err(Some(pc), m),
                    Ok((r, Role::Use)) => {
                        let (db, de) = def[r.0 as usize];
                        let live_across = if db == b.0 { de < epoch } else { epoch > 0 };
                        if live_across {
                            err(Some(pc), format!("{r:?} is live across a suspension point"));
                        }
                    }
                    Ok((r, Role::Def)) => def[r.0 as usize] = (b.0, epoch),
                }
            }
            check_inst(prog, f, id, inst, depth, is_fn, &mut |m| err(Some(pc), m));
            let suspends = match *inst {
                Inst::Suspend { .. } => true,
                Inst::Call { func, .. } => prog
                    .funcs
                    .get(func)
                    .is_some_and(|c| c.flags.contains(FuncFlags::MAY_SUSPEND)),
                _ => false,
            };
            if suspends {
                epoch += 1;
            }
        }
    }
}

fn check_inst(
    prog: &Program,
    f: &crate::Func,
    id: FuncId,
    inst: &Inst,
    depth: u8,
    is_fn: bool,
    err: &mut dyn FnMut(String),
) {
    let block_ok = |b: crate::BlockId| f.blocks.get(b).is_some();
    let msg_ok = |m: crate::CheckMsgId| prog.checks.get(m).is_some();
    for t in inst.targets() {
        if !block_ok(t) {
            err(format!("branch to missing block {t}"));
        }
    }
    let call = |func: FuncId, nargs: u32, has_dst: bool, err: &mut dyn FnMut(String)| {
        let Some(c) = prog.funcs.get(func) else {
            err(format!("call of missing function {func}"));
            return;
        };
        match c.kind {
            FuncKind::Process => err(format!("call of process {func}")),
            FuncKind::Elab if f.kind != FuncKind::Elab => err(format!(
                "call of elaboration body {func} outside elaboration"
            )),
            _ => {}
        }
        if c.params.len() != nargs as usize {
            err(format!(
                "{func} `{}` takes {} arguments, {nargs} given",
                c.name,
                c.params.len()
            ));
        }
        if c.kind != FuncKind::Thunk && has_dst != (c.kind == FuncKind::Function) {
            err(format!("result register does not match the kind of {func}"));
        }
        if is_fn && c.flags.contains(FuncFlags::MAY_SUSPEND) {
            err(format!(
                "{:?} calls {func} `{}`, which may suspend",
                f.kind, c.name
            ));
        }
    };
    match *inst {
        Inst::AddrOf { space, off, .. } => match space {
            AddrSpace::Frame if off > f.frame => {
                err(format!("frame offset {off} beyond the frame"))
            }
            AddrSpace::Up(d) if d == 0 || d > depth => err(format!(
                "static link depth {d}, function nesting is {depth}"
            )),
            AddrSpace::Const(c) if prog.consts.get(c).is_none() => {
                err(format!("missing constant {c}"))
            }
            _ => {}
        },
        Inst::CopyConst { c, .. } if prog.consts.get(c).is_none() => {
            err(format!("missing constant {c}"))
        }
        Inst::SigGather { tab, .. } | Inst::SigScatter { tab, .. } => {
            let ok = matches!(
                prog.consts.get(tab),
                Some(ConstData::Bytes(b)) if crate::gather::valid(&prog.types, b)
            );
            if !ok {
                err(format!("constant {tab} is not a leaf table"));
            }
        }
        Inst::Pos { what, .. }
        | Inst::Check { what, .. }
        | Inst::FCheck { what, .. }
        | Inst::BinRange { what, .. }
        | Inst::FToIRange { what, .. }
        | Inst::SubtypeCheck { what, .. }
        | Inst::Unreachable { what }
        | Inst::Deref { what, .. }
            if !msg_ok(what) =>
        {
            err(format!("missing check message {what}"))
        }
        Inst::SlicePos { what, dir_what, .. } if !msg_ok(what) || !msg_ok(dir_what) => {
            err(format!("missing check message {what} or {dir_what}"))
        }
        Inst::LayoutOf { ty, .. } | Inst::New { ty, .. } | Inst::NewFill { ty, .. }
            if prog.types.get(ty).is_none() =>
        {
            err(format!("missing type {ty}"))
        }
        Inst::LayoutOf {
            cache: Some(at), ..
        } if at >= f.frame => err(format!("layout cache {at} beyond the frame")),
        Inst::Case { table, .. } => match prog.cases.get(table) {
            None => err(format!("missing case table {table}")),
            Some(t) => {
                if !block_ok(t.default) || t.arms.iter().any(|a| !block_ok(a.2)) {
                    err(format!("case table {table} names a missing block"));
                }
            }
        },
        Inst::Ret { v } => {
            // A thunk evaluates an expression or initialises an object: either form.
            if f.kind != FuncKind::Thunk && v.is_some() != (f.kind == FuncKind::Function) {
                err(format!("return value does not match {:?}", f.kind));
            }
        }
        Inst::Call { func, args, dst } => call(func, args.len, dst.is_some(), err),
        Inst::CallProt {
            func, args, dst, ..
        } => call(func, args.len + 1, dst.is_some(), err),
        Inst::CallNative { native, args, .. } => match prog.natives.get(native) {
            None => err(format!("missing native {native}")),
            // An `ieee` native takes its array arguments as values (`ieee::ARRAY_REGS`).
            Some(n) => {
                if let Some(key) = crate::ieee::symbol_key(&n.symbol)
                    && crate::ieee::arg_regs(key) != args.len as usize
                {
                    err(format!(
                        "native `{}` takes {} argument registers, {} given",
                        n.symbol,
                        crate::ieee::arg_regs(key),
                        args.len
                    ));
                }
            }
        },
        Inst::CallForeign { foreign, args, .. } => match prog.foreign.get(foreign) {
            None => err(format!("missing foreign subprogram {foreign}")),
            Some(x) if x.args.iter().map(|a| a.cells()).sum::<usize>() != args.len as usize => {
                err(format!(
                    "foreign `{}` takes {} argument registers, {} given",
                    x.name,
                    x.args.iter().map(|a| a.cells()).sum::<usize>(),
                    args.len
                ))
            }
            _ => {}
        },
        Inst::Cover { item } if prog.cov.get(item).is_none() => {
            err(format!("missing coverage item {item}"))
        }
        Inst::CoverCond { cond, .. } if prog.cov_conds.get(cond as usize).is_none() => {
            err(format!("missing coverage condition {cond}"))
        }
        Inst::Suspend { resume } => {
            if is_fn {
                err(format!("{:?} cannot suspend", f.kind));
            }
            if f.resume.get(resume as usize).is_none() {
                err(format!("resume index {resume} not numbered"));
            }
        }
        Inst::Elab { op, .. } if f.kind != FuncKind::Elab => err(format!(
            "elaboration-only instruction {op:?} in {:?} {id}",
            f.kind
        )),
        _ => {}
    }
}

/// Registers read where some path from the entry (or, in a process, from the last `Suspend`)
/// does not define them first. A process that registers no wait condition anywhere (it only
/// has `wait;`) is never woken: no path goes on behind its `Suspend`: `(pc, register)`. Registers defined once by a constant or an
/// address start every frame with their value and are exempt.
pub fn undefined_uses(prog: &Program, f: &crate::Func) -> Vec<(u32, u32)> {
    let n = f.reg_ty.len();
    let w = n.div_ceil(64).max(1);
    let mut defs = vec![0u8; n];
    for inst in &f.code {
        inst.visit(&mut |r, role, _| {
            if role == Role::Def
                && let Some(d) = defs.get_mut(r.0 as usize)
            {
                *d = d.saturating_add(1);
            }
        });
    }
    let mut pre = vec![0u64; w];
    for inst in &f.code {
        let r = match *inst {
            Inst::Const { dst, .. } | Inst::ConstF { dst, .. } => Some(dst),
            Inst::AddrOf { dst, space, .. } if !matches!(space, AddrSpace::Up(_)) => Some(dst),
            _ => None,
        };
        if let Some(r) = r
            && defs.get(r.0 as usize) == Some(&1)
        {
            pre[r.0 as usize / 64] |= 1 << (r.0 % 64);
        }
    }
    let nb = f.blocks.len();
    let mut input: Vec<Option<Vec<u64>>> = vec![None; nb];
    if nb == 0 {
        return Vec::new();
    }
    input[0] = Some(pre.clone());
    let mut work = vec![0usize];
    let mut bad = Vec::new();
    let is_proc = f.kind == FuncKind::Process;
    let forever = is_proc
        && !f.code.iter().any(|i| {
            matches!(
                i,
                Inst::WaitOn { .. } | Inst::WaitFor { .. } | Inst::WaitAt { .. }
            )
        });
    let blocks: Vec<_> = f.blocks.iter().map(|(_, b)| *b).collect();
    'work: while let Some(b) = work.pop() {
        let mut cur = input[b].clone().unwrap_or_else(|| vec![!0; w]);
        let blk = blocks[b];
        for pc in blk.start as usize..blk.end as usize {
            let inst = &f.code[pc];
            let mut uses = Vec::new();
            inst.visit(&mut |r, role, _| uses.push((r, role)));
            if let Some(l) = inst.list()
                && let Some(rs) = f.regs(l)
            {
                uses.extend(rs.iter().map(|&r| (r, Role::Use)));
            }
            for (r, role) in uses {
                let (i, m) = (r.0 as usize / 64, 1u64 << (r.0 % 64));
                if i >= w {
                    continue;
                }
                match role {
                    Role::Use if cur[i] & m == 0 => {
                        bad.push((pc as u32, r.0));
                    }
                    Role::Use => {}
                    Role::Def => cur[i] |= m,
                }
            }
            if is_proc && matches!(inst, Inst::Suspend { .. }) {
                cur.clone_from(&pre);
                if forever {
                    continue 'work;
                }
            }
        }
        let succ: Vec<crate::BlockId> = match f.code[blk.end as usize - 1] {
            Inst::Case { table, .. } => prog.cases.get(table).map_or_else(Vec::new, |t| {
                t.arms
                    .iter()
                    .map(|a| a.2)
                    .chain(std::iter::once(t.default))
                    .collect()
            }),
            ref i => i.targets(),
        };
        for s in succ {
            let s = s.0 as usize;
            if s >= nb {
                continue;
            }
            let new = match &input[s] {
                None => cur.clone(),
                Some(old) => old.iter().zip(&cur).map(|(a, b)| a & b).collect(),
            };
            if input[s].as_ref() != Some(&new) {
                input[s] = Some(new);
                work.push(s);
            }
        }
    }
    bad.sort_unstable();
    bad.dedup();
    bad
}
