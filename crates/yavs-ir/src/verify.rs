//! The IR verifier: one linear walk over every function (ir.md 1.2).
//!
//! Its errors are internal compiler errors: malformed IR is a bug in the producer (lowering,
//! elaboration or a hand-written test), never a user error.

use std::fmt;

use crate::inst::Role;
use crate::{AddrSpace, FuncFlags, FuncId, FuncKind, Inst, Program, RegTy};

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
    for (id, _) in prog.funcs.iter().skip(first) {
        verify_func(prog, id, &mut errs);
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

fn verify_func(prog: &Program, id: FuncId, errs: &mut Vec<VerifyError>) {
    let Some(f) = prog.funcs.get(id) else { return };
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
    for (b, blk) in f.blocks.iter() {
        let mut epoch = 0u32;
        for pc in blk.start as usize..blk.end as usize {
            let inst = &f.code[pc];
            let s = f.loc[pc];
            match prog.stmts.get(s) {
                Some(info) if info.func == id => {}
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
        Inst::LayoutOf { ty, .. } | Inst::New { ty, .. } if prog.types.get(ty).is_none() => {
            err(format!("missing type {ty}"))
        }
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
        Inst::CallNative { native, .. } if prog.natives.get(native).is_none() => {
            err(format!("missing native {native}"))
        }
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
