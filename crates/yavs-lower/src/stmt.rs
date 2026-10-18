//! Sequential statements (DESIGN.md "Sequential statements") and the statements of implicit
//! processes.

use yavs_diag::Span;
use yavs_ir::CheckKind;
use yavs_ir::{BlockId, CaseTable, CmpOp, Dir, DriveMode, Inst, IntOp, RegTy, StmtKind, Width};
use yavs_sem::hir::{
    Choice, ChoiceSet, Delay, ExprKind, LoopScheme, Lsp, ObjRoot, Rhs, StmtKind as H, Target,
    Waveform,
};
use yavs_sem::types::{Direction, TypeKind};
use yavs_sem::{ExprId, StmtId};

use crate::expr::{Dr, Place};
use crate::func::Fx;
use crate::{Failed, Obj, R, Space, ty};

fn ir_kind(k: &H) -> StmtKind {
    match k {
        H::Wait { .. } => StmtKind::Wait,
        H::Assert { .. } | H::ConcAssert { .. } => StmtKind::Assert,
        H::Report { .. } => StmtKind::Report,
        H::SignalAssign { .. } | H::ConcSignalAssign { .. } => StmtKind::SigAssign,
        H::VarAssign { .. } => StmtKind::VarAssign,
        H::ProcCall(_) | H::ConcProcCall { .. } => StmtKind::Call,
        H::If { .. } => StmtKind::If,
        H::Case { .. } => StmtKind::Case,
        H::Loop { .. } => StmtKind::Loop,
        H::Next { .. } => StmtKind::Next,
        H::Exit { .. } => StmtKind::Exit,
        H::Return(_) => StmtKind::Return,
        _ => StmtKind::Null,
    }
}

impl Fx<'_, '_> {
    /// Lower `stmts` in order; every failing statement reports, the rest still lower.
    pub fn stmts(&mut self, stmts: &[StmtId]) -> R<()> {
        let mut ok = true;
        for &s in stmts {
            ok &= self.stmt(s).is_ok();
        }
        if ok { Ok(()) } else { Err(Failed) }
    }

    pub fn stmt(&mut self, id: StmtId) -> R<()> {
        let s = &self.db.stmts[id];
        self.stmt_start(ir_kind(&s.kind), s.span);
        self.stmt_kind(id)
    }

    /// The statement's code, after its `Stmt` marker.
    pub fn stmt_kind(&mut self, id: StmtId) -> R<()> {
        let db = self.db;
        let s = &db.stmts[id];
        let span = s.span;
        match &s.kind {
            H::Null => Ok(()),
            H::VarAssign { target, rhs } => self.var_assign(target, rhs, span),
            H::SignalAssign { target, delay, rhs }
            | H::ConcSignalAssign {
                target, delay, rhs, ..
            } => self.sig_assign(target, delay, rhs, span),
            H::Assert {
                cond,
                report,
                severity,
            }
            | H::ConcAssert {
                cond,
                report,
                severity,
                ..
            } => {
                let c = self.scalar_expr(*cond)?;
                let (fail, ok) = (self.block(), self.block());
                self.branch(c, ok, fail);
                self.at(fail);
                self.report(*report, *severity, 2, span)?;
                self.jump(ok);
                self.at(ok);
                Ok(())
            }
            H::Report { report, severity } => self.report(Some(*report), *severity, 0, span),
            H::ProcCall(c) | H::ConcProcCall { call: c, .. } => {
                self.call(c.callee, &c.args, c.method_of, c.span)?;
                Ok(())
            }
            H::If { branches, else_ } => {
                let j = self.block();
                for (c, body) in branches.iter() {
                    let c = self.scalar_expr(*c)?;
                    let (t, f) = (self.block(), self.block());
                    self.branch(c, t, f);
                    self.at(t);
                    self.cover_branch(span);
                    let r = self.stmts(body);
                    if self.live {
                        self.jump(j);
                    }
                    self.at(f);
                    r?;
                }
                if let Some(body) = else_ {
                    self.cover_branch(span);
                    self.stmts(body)?;
                }
                if self.live {
                    self.jump(j);
                }
                self.at(j);
                Ok(())
            }
            H::Case {
                expr,
                matching,
                alts,
            } => {
                let sets: Vec<&ChoiceSet> = alts.iter().map(|a| &a.0).collect();
                let blocks = self.case_blocks(*expr, *matching, &sets, span)?;
                let j = self.block();
                let mut ok = true;
                for ((_, body), b) in alts.iter().zip(blocks) {
                    self.at(b);
                    self.cover_branch(span);
                    ok &= self.stmts(body).is_ok();
                    if self.live {
                        self.jump(j);
                    }
                }
                self.at(j);
                if ok { Ok(()) } else { Err(Failed) }
            }
            H::Loop { scheme, body, .. } => self.loop_(id, scheme, body, span),
            H::Next { loop_, when } | H::Exit { loop_, when } => {
                let Some(&(_, next, exit)) = self.loops.iter().rev().find(|l| l.0 == *loop_) else {
                    return Err(self.cx.ice(span, "next/exit outside its loop"));
                };
                let to = if matches!(s.kind, H::Next { .. }) {
                    next
                } else {
                    exit
                };
                match when {
                    Some(c) => {
                        let c = self.scalar_expr(*c)?;
                        let cont = self.block();
                        self.branch(c, to, cont);
                        self.at(cont);
                    }
                    None => self.jump(to),
                }
                Ok(())
            }
            H::Return(v) => self.return_(*v, span),
            H::Wait { on, until, timeout } => self.wait(on, *until, *timeout, span),
            H::SeqBlock { scope, body } => {
                self.decls(*scope)?;
                self.stmts(body)
            }
            H::Force { target, mode, rhs } => self.force(target, *mode, rhs, span),
            H::Release { target, mode } => self.release(target, *mode, span),
            _ => Err(self
                .cx
                .ice(span, "concurrent statement in a sequential region")),
        }
    }

    /// Branch on `expr` to one new block per choice set (LRM 10.9); the current block is
    /// closed. A value no choice covers is a runtime error (sem checks coverage, so only an
    /// out-of-subtype value can get there).
    fn case_blocks(
        &mut self,
        expr: ExprId,
        matching: bool,
        sets: &[&ChoiceSet],
        span: Span,
    ) -> R<Vec<BlockId>> {
        let db = self.db;
        if matching {
            return Err(self
                .cx
                .unsupported(span, "matching case and selected statements"));
        }
        if ty::is_array(db, db.base(db.exprs[expr].subtype)) {
            return Err(self
                .cx
                .unsupported(span, "case and selected statements over arrays"));
        }
        let v = self.scalar_expr(expr)?;
        let mut arms = Vec::new();
        let mut default = None;
        let mut blocks = Vec::new();
        for set in sets {
            let b = self.block();
            blocks.push(b);
            let others = set.choices.iter().any(|c| matches!(c, Choice::Others));
            if others {
                default = Some(b);
            }
            match &set.intervals {
                Some(iv) => arms.extend(iv.iter().map(|&(lo, hi)| (lo, hi, b))),
                None if others => {}
                None => {
                    return Err(self
                        .cx
                        .unsupported(span, "choices that are not locally static"));
                }
            }
        }
        let missing = default.is_none();
        let default = default.unwrap_or_else(|| self.block());
        arms.sort_by_key(|a| a.0);
        let table = self.fb.program().case_table(CaseTable { arms, default });
        self.term(Inst::Case { v, table });
        if missing {
            self.at(default);
            let what = self.msg("case expression value not covered by any choice", Dir::To);
            self.term(Inst::Unreachable { what });
        }
        Ok(blocks)
    }

    fn report(
        &mut self,
        msg: Option<ExprId>,
        sev: Option<ExprId>,
        default: i64,
        span: Span,
    ) -> R<()> {
        let (data, dope) = match msg {
            Some(m) => {
                let v = self.expr(m)?;
                self.arr(v, span)?
            }
            None => {
                let st = match self.db.well_known.string {
                    Some(t) => self.db.types[t].first_subtype,
                    None => return Err(self.cx.ice(span, "STRING is not known")),
                };
                let text = b"Assertion violation.";
                let v = yavs_sem::stat::ConstValue::Array(yavs_sem::stat::ArrayConst {
                    bounds: Box::new([(1, Direction::To, text.len() as i64)]),
                    elems: yavs_sem::stat::Elems::Bytes(text.to_vec().into_boxed_slice()),
                });
                let v = self.konst(&v, st, span)?;
                self.arr(v, span)?
            }
        };
        let len = self.dope_len(dope);
        let severity = match sev {
            Some(e) => self.scalar_expr(e)?,
            None => self.int(default),
        };
        self.emit(Inst::Report {
            msg: data,
            len,
            severity,
        });
        Ok(())
    }

    fn var_assign(&mut self, target: &Target, rhs: &Rhs<ExprId>, span: Span) -> R<()> {
        let what = self.target_name(target.expr);
        match rhs {
            Rhs::Simple(e) => self.assign(target.expr, *e, &what, span),
            Rhs::Conditional(alts) => {
                let j = self.block();
                for &(e, c) in alts.iter() {
                    let next = self.block();
                    if let Some(c) = c {
                        let c = self.scalar_expr(c)?;
                        let t = self.block();
                        self.branch(c, t, next);
                        self.at(t);
                    }
                    self.cover_branch(span);
                    self.assign(target.expr, e, &what, span)?;
                    self.jump(j);
                    self.at(next);
                }
                self.jump(j);
                self.at(j);
                Ok(())
            }
            Rhs::Selected {
                expr,
                matching,
                alts,
            } => {
                let sets: Vec<&ChoiceSet> = alts.iter().map(|a| &a.1).collect();
                let blocks = self.case_blocks(*expr, *matching, &sets, span)?;
                let j = self.block();
                for (&(e, _), b) in alts.iter().zip(blocks) {
                    self.at(b);
                    self.cover_branch(span);
                    self.assign(target.expr, e, &what, span)?;
                    self.jump(j);
                }
                self.at(j);
                Ok(())
            }
        }
    }

    fn assign(&mut self, target: ExprId, e: ExprId, what: &str, span: Span) -> R<()> {
        let v = self.expr(e)?;
        let p: Place = self.place(target)?;
        self.store_place(p, v, self.db.exprs[e].subtype, what, span)
    }

    fn target_name(&self, e: ExprId) -> String {
        let mut e = e;
        loop {
            match &self.db.exprs[e].kind {
                ExprKind::Obj(d) => return ty::obj_desc(self.db, *d),
                ExprKind::Index { prefix, .. } | ExprKind::Slice { prefix, .. } => e = *prefix,
                _ => return "target".into(),
            }
        }
    }

    fn sig_assign(
        &mut self,
        target: &Target,
        delay: &Delay,
        rhs: &Rhs<Waveform>,
        span: Span,
    ) -> R<()> {
        match rhs {
            Rhs::Simple(w) => self.drive(target, delay, w, span),
            Rhs::Conditional(alts) => {
                let j = self.block();
                for (w, c) in alts.iter() {
                    let next = self.block();
                    if let Some(c) = c {
                        let c = self.scalar_expr(*c)?;
                        let t = self.block();
                        self.branch(c, t, next);
                        self.at(t);
                    }
                    self.cover_branch(span);
                    self.drive(target, delay, w, span)?;
                    self.jump(j);
                    self.at(next);
                }
                self.jump(j);
                self.at(j);
                Ok(())
            }
            Rhs::Selected {
                expr,
                matching,
                alts,
            } => {
                let sets: Vec<&ChoiceSet> = alts.iter().map(|a| &a.1).collect();
                let blocks = self.case_blocks(*expr, *matching, &sets, span)?;
                let j = self.block();
                for ((w, _), b) in alts.iter().zip(blocks) {
                    self.at(b);
                    self.cover_branch(span);
                    self.drive(target, delay, w, span)?;
                    self.jump(j);
                }
                self.at(j);
                Ok(())
            }
        }
    }

    /// The instance signal and driver slot of a whole-signal target.
    fn driver_of(&mut self, lsp: &Lsp) -> R<(u32, u32)> {
        let ObjRoot::Decl(d) = lsp.root else {
            return Err(self.cx.unsupported(
                lsp.span,
                "assignments to external names and implicit signals",
            ));
        };
        if !lsp.path.is_empty() {
            return Err(self
                .cx
                .unsupported(lsp.span, "assignments to signal sub-elements (M2 slice 2)"));
        }
        let Some(Obj::Signal { sig }) = self.cx.objs.get(&d).copied() else {
            return Err(self
                .cx
                .unsupported(lsp.span, "assignments to ports and signal parameters"));
        };
        match self.drivers.iter().position(|&x| x == d) {
            Some(slot) => Ok((sig, slot as u32)),
            None => Err(self
                .cx
                .ice(lsp.span, "signal target without a driver of the process")),
        }
    }

    fn drive(&mut self, target: &Target, delay: &Delay, w: &Waveform, span: Span) -> R<()> {
        let db = self.db;
        let Waveform::Elements(elems) = w else {
            return Ok(()); // `unaffected`
        };
        let [lsp] = &target.lsps[..] else {
            return Err(self.cx.unsupported(span, "aggregate signal targets"));
        };
        let (sig, slot) = self.driver_of(lsp)?;
        let st = db.exprs[target.expr].subtype;
        if ty::is_array(db, db.base(st)) {
            return Err(self.cx.ice(span, "array signal target"));
        }
        let (tid, _, vw) = self.scalar(st, span)?;
        let logic = yavs_ir::Ty::logic_leaf(self.fb.program().types(), tid).is_some();
        let stride = if logic { 8 + 32 } else { 16 };
        let n = elems.len() as u32;
        let wave = self.alloc_n(stride * n);
        let what = self.target_name(target.expr);
        let mut first = None;
        for (k, el) in elems.iter().enumerate() {
            let Some(v) = el.value else {
                return Err(self
                    .cx
                    .unsupported(span, "null waveform elements (guarded signals)"));
            };
            let at = self.offset(wave, stride * k as u32);
            let t = match el.after {
                Some(a) => {
                    let t = self.scalar_expr(a)?;
                    let z = self.int(0);
                    let max = self.int(i64::MAX);
                    let m = self.msg("delay of a waveform element", Dir::To);
                    self.emit(Inst::Check {
                        kind: CheckKind::Range,
                        v: t,
                        lo: z,
                        hi: max,
                        what: m,
                    });
                    t
                }
                None => self.int(0),
            };
            first.get_or_insert(t);
            self.store(at, t, Width::B8);
            let r = self.scalar_expr(v)?;
            self.check_scalar(r, Some(db.exprs[v].subtype), st, &what, span)?;
            let va = self.offset(at, 8);
            if logic {
                let z = self.int(0);
                let bias = self.code_bias(tid);
                let r = self.enc(r, bias);
                self.emit(Inst::LSet {
                    base: va,
                    idx: z,
                    v: r,
                });
            } else {
                self.store(va, r, vw);
            }
        }
        let n = self.int(i64::from(n));
        let (mode, reject) = match delay {
            Delay::Transport => (DriveMode::Transport, None),
            Delay::Default => (DriveMode::Inertial, None),
            Delay::Inertial { reject } => {
                let r = match reject {
                    Some(e) => Some(self.scalar_expr(*e)?),
                    None => None,
                };
                (DriveMode::Inertial, r)
            }
        };
        let _ = sig;
        self.emit(Inst::Drive {
            driver: slot,
            target: None,
            wave,
            n,
            mode,
            reject,
        });
        Ok(())
    }

    /// `WaitOn` for each signal of a sensitivity list.
    pub fn wait_on(&mut self, sens: &[Lsp]) -> R<()> {
        for l in sens {
            let ObjRoot::Decl(d) = l.root else {
                return Err(self
                    .cx
                    .unsupported(l.span, "sensitivity to external names and implicit signals"));
            };
            if !l.path.is_empty() {
                return Err(self
                    .cx
                    .unsupported(l.span, "sensitivity to signal sub-elements"));
            }
            let s = self.sig_of(d, l.span)?;
            self.emit(Inst::WaitOn { sig: s });
        }
        Ok(())
    }

    /// Signals named in `e` (the implicit sensitivity of `wait until`, LRM 10.2).
    fn signals_in(&self, e: ExprId, out: &mut Vec<Lsp>) {
        let db = self.db;
        let x = &db.exprs[e];
        match &x.kind {
            ExprKind::Obj(d) => {
                if matches!(self.cx.objs.get(d), Some(Obj::Signal { .. }))
                    && !out.iter().any(|l| l.root == ObjRoot::Decl(*d))
                {
                    out.push(Lsp {
                        root: ObjRoot::Decl(*d),
                        path: Box::new([]),
                        stat: x.stat,
                        span: x.span,
                    });
                }
            }
            ExprKind::Call { args, .. } => {
                for a in args.iter() {
                    if let yavs_sem::hir::Actual::Expr(e) = a.actual {
                        self.signals_in(e, out);
                    }
                }
            }
            ExprKind::Index { prefix, indices } => {
                self.signals_in(*prefix, out);
                for &i in indices.iter() {
                    self.signals_in(i, out);
                }
            }
            ExprKind::Slice { prefix, .. } | ExprKind::Field { prefix, .. } => {
                self.signals_in(*prefix, out)
            }
            ExprKind::Qualified { operand, .. }
            | ExprKind::Conv { operand, .. }
            | ExprKind::ImplicitConv { operand } => self.signals_in(*operand, out),
            ExprKind::Attr { args, .. } => {
                for &a in args.iter() {
                    self.signals_in(a, out);
                }
            }
            _ => {}
        }
    }

    fn wait(
        &mut self,
        on: &[Lsp],
        until: Option<ExprId>,
        timeout: Option<ExprId>,
        _span: Span,
    ) -> R<()> {
        let mut sens = on.to_vec();
        if sens.is_empty()
            && let Some(u) = until
        {
            self.signals_in(u, &mut sens);
        }
        let t = match timeout {
            Some(e) => {
                let t = self.scalar_expr(e)?;
                let z = self.int(0);
                let max = self.int(i64::MAX);
                let m = self.msg("timeout of a wait statement", Dir::To);
                self.emit(Inst::Check {
                    kind: CheckKind::Range,
                    v: t,
                    lo: z,
                    hi: max,
                    what: m,
                });
                Some(t)
            }
            None => None,
        };
        let Some(until) = until else {
            self.wait_on(&sens)?;
            if let Some(t) = t {
                self.emit(Inst::WaitFor { t });
            }
            self.emit(Inst::Suspend { resume: 0 });
            return Ok(());
        };
        // The deadline outlives the suspension, so it lives in the frame (ir.md 1.2).
        let deadline = t.map(|t| {
            let off = self.fb.slot(8, 8);
            let now = self.reg(RegTy::I64);
            self.emit(Inst::Now { dst: now });
            let d = self.bin(IntOp::Add, now, t);
            let a = self.addr(Space::Frame(self.depth), off);
            self.store(a, d, Width::B8);
            off
        });
        let (top, exit) = (self.block(), self.block());
        self.jump(top);
        self.at(top);
        self.wait_on(&sens)?;
        if let Some(off) = deadline {
            let a = self.addr(Space::Frame(self.depth), off);
            let d = self.load(a, Width::B8, RegTy::I64);
            self.emit(Inst::WaitAt { t: d });
        }
        self.emit(Inst::Suspend { resume: 0 });
        if deadline.is_some() {
            let to = self.reg(RegTy::I64);
            self.emit(Inst::TimedOut { dst: to });
            let check = self.block();
            self.branch(to, exit, check);
            self.at(check);
        }
        let c = self.scalar_expr(until)?;
        self.branch(c, exit, top);
        self.at(exit);
        Ok(())
    }

    fn return_(&mut self, v: Option<ExprId>, span: Span) -> R<()> {
        let db = self.db;
        let (Some(e), Some(ret)) = (v, self.ret) else {
            self.term(Inst::Ret { v: None });
            return Ok(());
        };
        let val = self.expr(e)?;
        match val {
            crate::expr::V::S(r) => {
                self.check_scalar(
                    r,
                    Some(db.exprs[e].subtype),
                    ret,
                    &format!("the result of {}", self.name),
                    span,
                )?;
                self.term(Inst::Ret { v: Some(r) });
            }
            crate::expr::V::A(data, dope) if self.is_general(db.base(ret), span)? => {
                let t = db.base(ret);
                let tid = self.ty(t, span)?;
                let mut dope = dope;
                if let Some(want) = self.desc_of(ret, Some(dope), span)? {
                    self.check_desc(
                        t,
                        tid,
                        dope,
                        want,
                        &format!("the result of {}", self.name),
                        span,
                    )?;
                    dope = want;
                }
                // The descriptor may live in this frame: copy it out with the result.
                let nd = self.copy_desc(tid, dope);
                let fat = self.alloc_n(16);
                self.store(fat, data, Width::B8);
                let d = self.offset(fat, 8);
                self.store(d, nd, Width::B8);
                self.term(Inst::Ret { v: Some(fat) });
            }
            crate::expr::V::A(data, mut dope) => {
                let dims = ty::array_of(db, db.base(ret)).map_or(1, |x| x.1);
                if let Some(want) = self.subtype_dope(ret, span)? {
                    let m = self.msg(&format!("the result of {}", self.name), Dir::To);
                    self.emit(Inst::SubtypeCheck {
                        dope,
                        want,
                        dims,
                        what: m,
                    });
                    dope = want;
                }
                // The descriptor may live in this frame: copy it out with the result.
                let bytes = 24 * u32::from(dims);
                let nd = self.alloc_n(bytes);
                let b = self.int(i64::from(bytes));
                self.emit(Inst::Copy {
                    dst: nd,
                    src: dope,
                    bytes: b,
                });
                let fat = self.alloc_n(16);
                self.store(fat, data, Width::B8);
                let d = self.offset(fat, 8);
                self.store(d, nd, Width::B8);
                self.term(Inst::Ret { v: Some(fat) });
            }
        }
        Ok(())
    }

    fn loop_(&mut self, id: StmtId, scheme: &LoopScheme, body: &[StmtId], span: Span) -> R<()> {
        let db = self.db;
        match scheme {
            LoopScheme::Forever => {
                let (head, exit) = (self.block(), self.block());
                self.jump(head);
                self.at(head);
                self.loops.push((id, head, exit));
                let r = self.stmts(body);
                self.loops.pop();
                if self.live {
                    self.jump(head);
                }
                self.at(exit);
                r
            }
            LoopScheme::While(c) => {
                let (head, b, exit) = (self.block(), self.block(), self.block());
                self.jump(head);
                self.at(head);
                let c = self.scalar_expr(*c)?;
                self.branch(c, b, exit);
                self.at(b);
                self.cover_branch(span);
                self.loops.push((id, head, exit));
                let r = self.stmts(body);
                self.loops.pop();
                if self.live {
                    self.jump(head);
                }
                self.at(exit);
                r
            }
            LoopScheme::For { param, range } => {
                let r = self.range(range, span)?;
                let pst = match &db.decls[*param].kind {
                    yavs_sem::decl::DeclKind::Object(o) => o.subtype,
                    _ => return Err(self.cx.ice(span, "loop parameter is not an object")),
                };
                if !matches!(
                    db.types[db.base(pst)].kind,
                    TypeKind::Integer
                        | TypeKind::Enum { .. }
                        | TypeKind::UniversalInteger
                        | TypeKind::Physical { .. }
                ) {
                    return Err(self.cx.ice(span, "loop over a non-discrete range"));
                }
                let tid = self.ty(db.base(pst), span)?;
                let name = db.decls[*param].name.to_string();
                let off = self.fb.local(&name, tid, 8, 8);
                let roff = self.fb.slot(8, 8);
                let doff = self.fb.slot(8, 8);
                let p = self.addr(Space::Frame(self.depth), off);
                self.store(p, r.left, Width::B8);
                let ra = self.addr(Space::Frame(self.depth), roff);
                self.store(ra, r.right, Width::B8);
                let dir = self.dir_reg(r.dir);
                let da = self.addr(Space::Frame(self.depth), doff);
                self.store(da, dir, Width::B8);
                self.cx.objs.insert(
                    *param,
                    Obj::Scalar {
                        space: Space::Frame(self.depth),
                        off,
                        w: Width::B8,
                    },
                );
                let null = match r.dir {
                    Dr::Static(Direction::To) => self.cmp(CmpOp::Gt, r.left, r.right),
                    Dr::Static(Direction::Downto) => self.cmp(CmpOp::Lt, r.left, r.right),
                    Dr::Dyn(d) => {
                        let gt = self.cmp(CmpOp::Gt, r.left, r.right);
                        let lt = self.cmp(CmpOp::Lt, r.left, r.right);
                        let z = self.int(0);
                        let up = self.cmp(CmpOp::Eq, d, z);
                        self.select(up, gt, lt, RegTy::I64)
                    }
                };
                let (b, next, inc, exit) = (self.block(), self.block(), self.block(), self.block());
                self.branch(null, exit, b);
                self.at(b);
                self.cover_branch(span);
                self.loops.push((id, next, exit));
                let res = self.stmts(body);
                self.loops.pop();
                if self.live {
                    self.jump(next);
                }
                // `next`: stop at the right bound before stepping, so the parameter never
                // steps past it (no overflow at the type's bounds).
                self.at(next);
                let p = self.addr(Space::Frame(self.depth), off);
                let i = self.load(p, Width::B8, RegTy::I64);
                let ra = self.addr(Space::Frame(self.depth), roff);
                let right = self.load(ra, Width::B8, RegTy::I64);
                let last = self.cmp(CmpOp::Eq, i, right);
                self.branch(last, exit, inc);
                self.at(inc);
                let da = self.addr(Space::Frame(self.depth), doff);
                let d = self.load(da, Width::B8, RegTy::I64);
                let z = self.int(0);
                let up = self.cmp(CmpOp::Eq, d, z);
                let one = self.int(1);
                let m1 = self.int(-1);
                let step = self.select(up, one, m1, RegTy::I64);
                let n = self.bin(IntOp::Add, i, step);
                let p = self.addr(Space::Frame(self.depth), off);
                self.store(p, n, Width::B8);
                self.jump(b);
                self.at(exit);
                res
            }
        }
    }
}
