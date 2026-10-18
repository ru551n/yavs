//! Sequential statements (DESIGN.md "Sequential statements") and the statements of implicit
//! processes.

use yavs_diag::Span;
use yavs_ir::CheckKind;
use yavs_ir::{
    BlockId, CaseTable, CmpOp, Dir, DriveMode, Inst, IntOp, Reg, RegTy, StmtKind, Width,
};
use yavs_sem::hir::{
    Choice, ChoiceSet, Delay, ExprKind, LoopScheme, Lsp, LspStep, ObjRoot, Rhs, StmtKind as H,
    Target, Waveform,
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
            H::ConcSignalAssign {
                target,
                delay,
                rhs,
                guarded: true,
                ..
            } => self.guarded_assign(target, delay, rhs, span),
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
        let array = ty::is_array(db, db.base(db.exprs[expr].subtype));
        if matching
            || array
            || sets.iter().any(|s| {
                s.intervals.is_none() && !s.choices.iter().all(|c| matches!(c, Choice::Others))
            })
        {
            return self.case_chain(expr, matching, sets, span);
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

    /// `case` by comparing with each choice in order: arrays (LRM 10.9, choices of the
    /// expression's length), matching `case?` (`?=` on BIT and STD_ULOGIC elements) and
    /// choices that are not locally static.
    fn case_chain(
        &mut self,
        expr: ExprId,
        matching: bool,
        sets: &[&ChoiceSet],
        span: Span,
    ) -> R<Vec<BlockId>> {
        let db = self.db;
        let t = db.base(db.exprs[expr].subtype);
        let array = ty::is_array(db, t);
        let v = self.expr(expr)?;
        let blocks: Vec<BlockId> = sets.iter().map(|_| self.block()).collect();
        let mut default = None;
        for (set, &b) in sets.iter().zip(&blocks) {
            for c in set.choices.iter() {
                let hit = match c {
                    Choice::Others => {
                        default = Some(b);
                        continue;
                    }
                    Choice::Expr(e) => {
                        let cv = self.expr(*e)?;
                        match (array, matching) {
                            (true, false) => self.array_cmp(CmpOp::Eq, v, cv, t, span)?,
                            (true, true) => self.array_match(v, cv, t, span)?,
                            (false, m) => {
                                let a = self.scal(v, span)?;
                                let b = self.scal(cv, span)?;
                                if m {
                                    self.match_scalar(a, b, t, span)?
                                } else {
                                    self.cmp(CmpOp::Eq, a, b)
                                }
                            }
                        }
                    }
                    Choice::Range(rc) if !array => {
                        let a = self.scal(v, span)?;
                        let r = self.range(rc, span)?;
                        let (lo, hi) = self.low_high(r);
                        let ge = self.cmp(CmpOp::Ge, a, lo);
                        let le = self.cmp(CmpOp::Le, a, hi);
                        let dst = self.reg(RegTy::I64);
                        self.emit(Inst::Bool {
                            op: yavs_ir::BoolOp::And,
                            dst,
                            a: ge,
                            b: le,
                        });
                        dst
                    }
                    _ => return Err(self.cx.ice(span, "choice kind in a case statement")),
                };
                let next = self.block();
                self.branch(hit, b, next);
                self.at(next);
            }
        }
        match default {
            Some(b) => self.jump(b),
            None => {
                let what = self.msg("case expression value not covered by any choice", Dir::To);
                self.term(Inst::Unreachable { what });
            }
        }
        Ok(blocks)
    }

    /// The `?=` class of a plane code: 0 for '0'/'L', 1 for '1'/'H', 2 for '-', 3 otherwise.
    fn match_class(&mut self, code: Reg) -> Reg {
        let tab = self
            .fb
            .program()
            .konst(yavs_ir::ConstData::Bytes(vec![3, 3, 0, 1, 3, 3, 0, 1, 2]));
        let base = self.reg(RegTy::Ptr);
        self.emit(Inst::AddrOf {
            dst: base,
            space: yavs_ir::AddrSpace::Const(tab),
            off: 0,
        });
        let a = self.reg(RegTy::Ptr);
        self.emit(Inst::Index {
            dst: a,
            base,
            pos: code,
            stride: 1,
        });
        self.load(a, Width::B1, RegTy::I64)
    }

    /// Whether two plane codes match under `?=` (a '-' matches anything, metavalues nothing).
    fn match_codes(&mut self, a: Reg, b: Reg) -> Reg {
        let ca = self.match_class(a);
        let cb = self.match_class(b);
        let two = self.int(2);
        let wa = self.cmp(CmpOp::Eq, ca, two);
        let wb = self.cmp(CmpOp::Eq, cb, two);
        let eq = self.cmp(CmpOp::Eq, ca, cb);
        let meta = self.cmp(CmpOp::Lt, ca, two);
        let same = self.reg(RegTy::I64);
        self.emit(Inst::Bool {
            op: yavs_ir::BoolOp::And,
            dst: same,
            a: eq,
            b: meta,
        });
        let w = self.reg(RegTy::I64);
        self.emit(Inst::Bool {
            op: yavs_ir::BoolOp::Or,
            dst: w,
            a: wa,
            b: wb,
        });
        let dst = self.reg(RegTy::I64);
        self.emit(Inst::Bool {
            op: yavs_ir::BoolOp::Or,
            dst,
            a: w,
            b: same,
        });
        dst
    }

    /// `?=` on scalars of type `t` (BIT, STD_ULOGIC; positions in, plane codes compared).
    fn match_scalar(&mut self, a: Reg, b: Reg, t: yavs_sem::TypeId, span: Span) -> R<Reg> {
        let tid = self.ty(t, span)?;
        if yavs_ir::Ty::logic_leaf(self.fb.program().types(), tid).is_none() {
            return Err(self.cx.unsupported(
                span,
                "matching case over types other than BIT and STD_ULOGIC",
            ));
        }
        let bias = self.code_bias(tid);
        let a = self.enc(a, bias);
        let b = self.enc(b, bias);
        Ok(self.match_codes(a, b))
    }

    /// `?=` on two arrays of BIT or STD_ULOGIC elements: equal lengths and every element matches.
    fn array_match(
        &mut self,
        a: crate::expr::V,
        b: crate::expr::V,
        t: yavs_sem::TypeId,
        span: Span,
    ) -> R<Reg> {
        let (ad, ap) = self.arr(a, span)?;
        let (bd, bp) = self.arr(b, span)?;
        let tid = self.ty(t, span)?;
        if yavs_ir::Ty::logic_leaf(self.fb.program().types(), tid).is_none() {
            return Err(self.cx.unsupported(
                span,
                "matching case over arrays other than BIT and STD_ULOGIC vectors",
            ));
        }
        let la = self.dope_len(ap);
        let lb = self.dope_len(bp);
        let r = self.cmp(CmpOp::Eq, la, lb);
        let (body, done) = (self.block(), self.block());
        self.branch(r, body, done);
        self.at(body);
        self.for_n(la, |fx, i| {
            let ea = fx.reg(RegTy::I64);
            fx.emit(Inst::LGet {
                dst: ea,
                base: ad,
                idx: i,
            });
            let eb = fx.reg(RegTy::I64);
            fx.emit(Inst::LGet {
                dst: eb,
                base: bd,
                idx: i,
            });
            let m = fx.match_codes(ea, eb);
            let both = fx.reg(RegTy::I64);
            fx.emit(Inst::Bool {
                op: yavs_ir::BoolOp::And,
                dst: both,
                a: r,
                b: m,
            });
            fx.emit(Inst::Move { dst: r, src: both });
            Ok(())
        })?;
        self.jump(done);
        self.at(done);
        Ok(r)
    }

    /// A guarded concurrent assignment (LRM 11.6): assign while GUARD is true; otherwise
    /// disconnect the drivers of a guarded (`bus` or `register`) target.
    fn guarded_assign(
        &mut self,
        target: &Target,
        delay: &Delay,
        rhs: &Rhs<Waveform>,
        span: Span,
    ) -> R<()> {
        let db = self.db;
        let Some(g) = self.guard else {
            return Err(self
                .cx
                .ice(span, "guarded assignment outside a guarded block"));
        };
        let c = self.sig_of(g, span)?;
        let raw = self.reg(RegTy::I64);
        self.emit(Inst::SigReadScalar { dst: raw, sig: c });
        let tid = self.decl_ty(g, span)?;
        let bias = self.code_bias(tid);
        let c = self.dec(raw, bias);
        let (t, f, j) = (self.block(), self.block(), self.block());
        self.branch(c, t, f);
        self.at(t);
        self.sig_assign(target, delay, rhs, span)?;
        if self.live {
            self.jump(j);
        }
        self.at(f);
        let guarded_target = target.lsps.iter().all(|l| match l.root {
            ObjRoot::Decl(d) => matches!(&db.decls[d].kind,
                yavs_sem::decl::DeclKind::Object(o) if o.signal_kind.is_some()),
            _ => false,
        });
        if guarded_target {
            let [lsp] = &target.lsps[..] else {
                return Err(self.cx.unsupported(span, "aggregate signal targets"));
            };
            let slot = self.driver_of(lsp)?;
            let (_, sref, _) = self.sig_name(target.expr)?;
            let mode = match delay {
                Delay::Transport => DriveMode::Transport,
                _ => DriveMode::Inertial,
            };
            // ponytail: disconnection after 0 fs until disconnection specifications reach
            // the HIR (yavs-elab uses 0 for the kernel's guarded signals too).
            self.emit(Inst::Disconnect {
                driver: slot,
                target: Some(sref),
                after: None,
                mode,
            });
        }
        self.jump(j);
        self.at(j);
        Ok(())
    }

    /// `GUARD <= condition` (zero delay) in a block's guard process; GUARD is driver slot 0.
    pub fn drive_guard(&mut self, g: yavs_sem::DeclId, cond: ExprId, span: Span) -> R<()> {
        let v = self.scalar_expr(cond)?;
        let tid = self.decl_ty(g, span)?;
        let bias = self.code_bias(tid);
        let code = self.enc(v, bias);
        let wave = self.alloc_n(8 + 32);
        let z = self.int(0);
        self.store(wave, z, Width::B8);
        let va = self.offset(wave, 8);
        self.emit(Inst::LSet {
            base: va,
            idx: z,
            v: code,
        });
        let n = self.int(1);
        let s = self.sig_of(g, span)?;
        self.emit(Inst::Drive {
            driver: 0,
            target: Some(s),
            wave,
            n,
            mode: DriveMode::Inertial,
            reject: None,
        });
        Ok(())
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
        if let ExprKind::Aggregate { assocs, .. } = &self.db.exprs[target].kind {
            return self.assign_aggregate(assocs, v, e, span);
        }
        let p: Place = self.place(target)?;
        self.store_place(p, v, self.db.exprs[e].subtype, what, span)
    }

    /// `(a, b) := v` (LRM 10.6.2.1): each element of the aggregate target gets the matching
    /// element of the value, positionally or by index choice. Scalar elements only.
    fn assign_aggregate(
        &mut self,
        assocs: &[yavs_sem::hir::AggAssoc],
        v: crate::expr::V,
        e: ExprId,
        span: Span,
    ) -> R<()> {
        let db = self.db;
        let t = db.base(db.exprs[e].subtype);
        let Some((elem, 1)) = ty::array_of(db, t) else {
            return Err(self
                .cx
                .unsupported(span, "aggregate targets of records and matrices"));
        };
        let (data, dope) = self.arr(v, span)?;
        let tid = self.ty(t, span)?;
        let logic = yavs_ir::Ty::logic_leaf(self.fb.program().types(), tid).is_some();
        let bias = self.code_bias(tid);
        let (_, rt, w) = self.scalar(elem, span)?;
        let count = self.dope_len(dope);
        let m = self.msg("aggregate target", Dir::To);
        for (k, a) in assocs.iter().enumerate() {
            if a.slice_piece {
                return Err(self.cx.unsupported(span, "slices in aggregate targets"));
            }
            let pos = match a.choices.first() {
                None => self.int(k as i64),
                Some(Choice::Expr(c)) if a.choices.len() == 1 => {
                    let i = self.scalar_expr(*c)?;
                    let p = self.reg(RegTy::I64);
                    self.emit(Inst::Pos {
                        dst: p,
                        dope,
                        dim: 0,
                        idx: i,
                        what: m,
                    });
                    p
                }
                _ => return Err(self.cx.unsupported(span, "this aggregate target choice")),
            };
            let x = if logic {
                let one = self.int(1);
                let last = self.bin(IntOp::Sub, count, one);
                let idx = self.bin(IntOp::Sub, last, pos);
                let c = self.reg(RegTy::I64);
                self.emit(Inst::LGet {
                    dst: c,
                    base: data,
                    idx,
                });
                self.dec(c, bias)
            } else {
                let a = self.reg(RegTy::Ptr);
                self.emit(Inst::Index {
                    dst: a,
                    base: data,
                    pos,
                    stride: w.bytes(),
                });
                self.load(a, w, rt)
            };
            let what = self.target_name(a.value);
            let p = self.place(a.value)?;
            self.store_place(p, crate::expr::V::S(x), elem, &what, span)?;
        }
        Ok(())
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
    /// The driver slot of a target's longest static prefix: the `ProcessInfo::drivers` entry
    /// with the same root and path (slots are 1:1 with those entries, as yavs-elab expects).
    fn driver_of(&mut self, lsp: &Lsp) -> R<u32> {
        let key = format!("{:?}{:?}", lsp.root, lsp.path);
        match self
            .drivers
            .iter()
            .position(|l| format!("{:?}{:?}", l.root, l.path) == key)
        {
            Some(k) => Ok(self.driver_base.get(k).copied().unwrap_or(k as u32)),
            None => Err(self
                .cx
                .ice(lsp.span, "signal target without a driver of the process")),
        }
    }

    /// A record signal (or record part of one) as a target: each leaf is driven with its
    /// field of every waveform value, through its own driver slot.
    #[allow(clippy::too_many_arguments)]
    fn drive_record(
        &mut self,
        d: yavs_sem::DeclId,
        path: &[u32],
        base: u32,
        covered: &[crate::RecLeaf],
        target: &Target,
        delay: &Delay,
        elems: &[yavs_sem::hir::WaveElem],
        span: Span,
    ) -> R<()> {
        let db = self.db;
        let st = db.exprs[target.expr].subtype;
        let t = db.base(st);
        let tid = self.ty(t, span)?;
        let mut vals = Vec::new();
        for el in elems {
            let Some(v) = el.value else {
                return Err(self
                    .cx
                    .unsupported(span, "null waveform elements of record signals"));
            };
            let at = match el.after {
                Some(a) => self.scalar_expr(a)?,
                None => self.int(0),
            };
            let x = self.expr(v)?;
            vals.push((at, self.arr(x, span)?));
        }
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
        for leaf in self.leaves_under(d, path) {
            let slot = base
                + covered
                    .iter()
                    .position(|l| l.path == leaf.path)
                    .unwrap_or(0) as u32;
            let (sref, ldope) = self.leaf_sig(&leaf, span)?;
            let ltid = self.ty(db.base(leaf.st), span)?;
            let logic = yavs_ir::Ty::logic_leaf(self.fb.program().types(), ltid).is_some();
            let rel = &leaf.path[path.len()..];
            let size = ldope.map(|dp| self.layout(ltid, dp).0);
            let stride = match size {
                Some(sz) => {
                    let seven = self.int(7);
                    let r = self.bin(IntOp::Add, sz, seven);
                    let eight = self.int(8);
                    let q = self.bin(IntOp::Div, r, eight);
                    let r8 = self.bin(IntOp::Mul, q, eight);
                    self.bin(IntOp::Add, r8, eight)
                }
                None => self.int(if logic { 8 + 32 } else { 16 }),
            };
            let n = self.int(vals.len() as i64);
            let total = self.bin(IntOp::Mul, stride, n);
            let wave = self.alloc(total);
            for (j, &(at, (data, desc))) in vals.iter().enumerate() {
                let (faddr, _) = self.leaf_addr(t, tid, data, desc, rel, span)?;
                let jj = self.int(j as i64);
                let o = self.bin(IntOp::Mul, jj, stride);
                let e = self.reg(RegTy::Ptr);
                self.emit(Inst::Index {
                    dst: e,
                    base: wave,
                    pos: o,
                    stride: 1,
                });
                self.store(e, at, Width::B8);
                let va = self.offset(e, 8);
                match size {
                    Some(sz) => self.emit(Inst::Copy {
                        dst: va,
                        src: faddr,
                        bytes: sz,
                    }),
                    None => {
                        let (_, rt, w) = self.scalar(leaf.st, span)?;
                        let x = self.load(faddr, w, rt);
                        if logic {
                            let bias = self.code_bias(ltid);
                            let c = self.enc(x, bias);
                            let z = self.int(0);
                            self.emit(Inst::LSet {
                                base: va,
                                idx: z,
                                v: c,
                            });
                        } else {
                            self.store(va, x, w);
                        }
                    }
                }
            }
            self.emit(Inst::Drive {
                driver: slot,
                target: Some(sref),
                wave,
                n,
                mode,
                reject,
            });
        }
        Ok(())
    }

    /// `(a, b) <= v after t, ...` (LRM 10.5.2.1): each scalar element of the aggregate target is
    /// driven with the matching element of every waveform value.
    fn drive_aggregate(
        &mut self,
        target: &Target,
        assocs: &[yavs_sem::hir::AggAssoc],
        delay: &Delay,
        elems: &[yavs_sem::hir::WaveElem],
        span: Span,
    ) -> R<()> {
        let db = self.db;
        if assocs.len() != target.lsps.len() {
            return Err(self
                .cx
                .unsupported(span, "aggregate targets with choice lists"));
        }
        let t = db.base(db.exprs[target.expr].subtype);
        let Some((esub, 1)) = ty::array_of(db, t) else {
            return Err(self
                .cx
                .unsupported(span, "aggregate signal targets of records and matrices"));
        };
        let vtid = self.ty(t, span)?;
        let vlogic = yavs_ir::Ty::logic_leaf(self.fb.program().types(), vtid).is_some();
        let vbias = self.code_bias(vtid);
        let (_, rt, vw) = self.scalar(esub, span)?;
        let mut vals = Vec::new();
        for el in elems {
            let Some(v) = el.value else {
                return Err(self
                    .cx
                    .unsupported(span, "null waveform elements in aggregate targets"));
            };
            let at = match el.after {
                Some(a) => self.scalar_expr(a)?,
                None => self.int(0),
            };
            let x = self.expr(v)?;
            vals.push((at, self.arr(x, span)?));
        }
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
        let m = self.msg("aggregate target", Dir::To);
        for (k, (a, lsp)) in assocs.iter().zip(target.lsps.iter()).enumerate() {
            if a.slice_piece {
                return Err(self
                    .cx
                    .unsupported(span, "slices in aggregate signal targets"));
            }
            let slot = self.driver_of(lsp)?;
            let (_, sref, tdope) = self.sig_name(a.value)?;
            if tdope.is_some() {
                return Err(self
                    .cx
                    .unsupported(span, "array elements of aggregate signal targets"));
            }
            let st = db.exprs[a.value].subtype;
            let ttid = self.ty(db.base(st), span)?;
            let tlogic = yavs_ir::Ty::logic_leaf(self.fb.program().types(), ttid).is_some();
            let tbias = self.code_bias(ttid);
            let (_, _, tw) = self.scalar(st, span)?;
            let stride = if tlogic { 8 + 32 } else { 16 };
            let wave = self.alloc_n(stride * vals.len() as u32);
            for (j, &(at, (data, dope))) in vals.iter().enumerate() {
                let pos = match a.choices.first() {
                    None => self.int(k as i64),
                    Some(Choice::Expr(c)) if a.choices.len() == 1 => {
                        let i = self.scalar_expr(*c)?;
                        let p = self.reg(RegTy::I64);
                        self.emit(Inst::Pos {
                            dst: p,
                            dope,
                            dim: 0,
                            idx: i,
                            what: m,
                        });
                        p
                    }
                    _ => return Err(self.cx.unsupported(span, "this aggregate target choice")),
                };
                let x = if vlogic {
                    let count = self.dope_len(dope);
                    let one = self.int(1);
                    let last = self.bin(IntOp::Sub, count, one);
                    let idx = self.bin(IntOp::Sub, last, pos);
                    let c = self.reg(RegTy::I64);
                    self.emit(Inst::LGet {
                        dst: c,
                        base: data,
                        idx,
                    });
                    self.dec(c, vbias)
                } else {
                    let p = self.reg(RegTy::Ptr);
                    self.emit(Inst::Index {
                        dst: p,
                        base: data,
                        pos,
                        stride: vw.bytes(),
                    });
                    self.load(p, vw, rt)
                };
                let what = self.target_name(a.value);
                self.check_scalar(x, Some(esub), st, &what, span)?;
                let e = self.offset(wave, stride * j as u32);
                self.store(e, at, Width::B8);
                let va = self.offset(e, 8);
                if tlogic {
                    let z = self.int(0);
                    let c = self.enc(x, tbias);
                    self.emit(Inst::LSet {
                        base: va,
                        idx: z,
                        v: c,
                    });
                } else {
                    self.store(va, x, tw);
                }
            }
            let n = self.int(vals.len() as i64);
            self.emit(Inst::Drive {
                driver: slot,
                target: Some(sref),
                wave,
                n,
                mode,
                reject,
            });
        }
        Ok(())
    }

    fn drive(&mut self, target: &Target, delay: &Delay, w: &Waveform, span: Span) -> R<()> {
        let db = self.db;
        let Waveform::Elements(elems) = w else {
            return Ok(()); // `unaffected`
        };
        if let ExprKind::Aggregate { assocs, .. } = &db.exprs[target.expr].kind {
            return self.drive_aggregate(target, assocs, delay, elems, span);
        }
        let [lsp] = &target.lsps[..] else {
            return Err(self.cx.unsupported(span, "aggregate signal targets"));
        };
        if !matches!(lsp.root, ObjRoot::Decl(_)) {
            return Err(self.cx.unsupported(
                lsp.span,
                "assignments to external names and implicit signals",
            ));
        }
        let param_slot = match lsp.root {
            ObjRoot::Decl(d) => match self.cx.objs.get(&d).copied() {
                Some(Obj::SignalRef { out: true, .. }) => Some(self.driver_slot_of(d, span)?),
                Some(Obj::SignalRef { out: false, .. }) => {
                    return Err(self.cx.ice(span, "assignment to an `in` signal parameter"));
                }
                _ => None,
            },
            _ => None,
        };
        let mut slot = match param_slot {
            Some(_) => 0,
            None => self.driver_of(lsp)?,
        };
        if let Some((d, path)) = self.field_path(target.expr) {
            let leaves = self.leaves_under(d, &path);
            let covered = self.leaves_under(d, &crate::func::lsp_fields(lsp));
            if !matches!(leaves.as_slice(), [l] if l.path == path) {
                return self.drive_record(d, &path, slot, &covered, target, delay, elems, span);
            }
            slot += covered.iter().position(|l| l.path == path).unwrap_or(0) as u32;
        }
        let (_, sref, tdope) = self.sig_name(target.expr)?;
        let st = db.exprs[target.expr].subtype;
        let tid = self.ty(db.base(st), span)?;
        let logic = yavs_ir::Ty::logic_leaf(self.fb.program().types(), tid).is_some();
        // Bytes per waveform element: the time, then the value rounded up to 8 bytes.
        let (stride, size) = match tdope {
            Some(dope) => {
                let (size, _) = self.layout(tid, dope);
                let seven = self.int(7);
                let r = self.bin(IntOp::Add, size, seven);
                let eight = self.int(8);
                let q = self.bin(IntOp::Div, r, eight);
                let r8 = self.bin(IntOp::Mul, q, eight);
                (self.bin(IntOp::Add, r8, eight), Some(size))
            }
            None => (self.int(if logic { 8 + 32 } else { 16 }), None),
        };
        let n = self.int(elems.len() as i64);
        let total = self.bin(IntOp::Mul, stride, n);
        let wave = self.alloc(total);
        let what = self.target_name(target.expr);
        for (k, el) in elems.iter().enumerate() {
            let Some(v) = el.value else {
                return Err(self
                    .cx
                    .unsupported(span, "null waveform elements (guarded signals)"));
            };
            let kk = self.int(k as i64);
            let o = self.bin(IntOp::Mul, kk, stride);
            let at = self.reg(RegTy::Ptr);
            self.emit(Inst::Index {
                dst: at,
                base: wave,
                pos: o,
                stride: 1,
            });
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
            self.store(at, t, Width::B8);
            let va = self.offset(at, 8);
            match (tdope, size) {
                (Some(dope), Some(size)) => {
                    let val = self.expr(v)?;
                    let (data, vd) = self.arr(val, span)?;
                    let dims = ty::array_of(db, db.base(st)).map_or(1, |x| x.1);
                    let m = self.msg(&what, Dir::To);
                    self.emit(Inst::SubtypeCheck {
                        dope: vd,
                        want: dope,
                        dims,
                        what: m,
                    });
                    self.emit(Inst::Copy {
                        dst: va,
                        src: data,
                        bytes: size,
                    });
                }
                _ => {
                    let (_, _, vw) = self.scalar(st, span)?;
                    let r = self.scalar_expr(v)?;
                    self.check_scalar(r, Some(db.exprs[v].subtype), st, &what, span)?;
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
            }
        }
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
        match param_slot {
            Some(slot) => self.emit(Inst::DriveSlot {
                slot,
                target: sref,
                wave,
                n,
                mode,
                reject,
            }),
            None => self.emit(Inst::Drive {
                driver: slot,
                target: Some(sref),
                wave,
                n,
                mode,
                reject,
            }),
        }
        Ok(())
    }

    /// `WaitOn` for each signal of a sensitivity list.
    pub fn wait_on(&mut self, sens: &[Lsp]) -> R<()> {
        for l in sens {
            let (ObjRoot::Decl(d) | ObjRoot::ImplicitSignal(d)) = l.root else {
                return Err(self.cx.unsupported(l.span, "sensitivity to external names"));
            };
            if matches!(self.cx.objs.get(&d), Some(Obj::RecSignal)) {
                // A record signal: every leaf below the prefix's fields.
                for leaf in self.leaves_under(d, &crate::func::lsp_fields(l)) {
                    let (s, _) = self.leaf_sig(&leaf, l.span)?;
                    self.emit(Inst::WaitOn { sig: s });
                }
                continue;
            }
            let s = match &l.path[..] {
                [] => self.sig_of(d, l.span)?,
                [LspStep::Index(ix)] if ix.len() == 1 => self.sig_select(d, Ok(ix[0]), l.span)?.0,
                [LspStep::Slice(r)] => self.sig_select(d, Err(r), l.span)?.0,
                path => self.lsp_sig(d, path, l.span)?,
            };
            self.emit(Inst::WaitOn { sig: s });
        }
        Ok(())
    }

    /// The signal at path `path` of array signal `d`: slices and elements, each indexed in
    /// the index space of the alias subtype a `Rebase` step gives (LRM 6.6.2).
    fn lsp_sig(&mut self, d: yavs_sem::DeclId, path: &[LspStep], span: Span) -> R<Reg> {
        let nested = |fx: &mut Self| {
            Err(fx
                .cx
                .unsupported(span, "sensitivity to record and nested sub-elements"))
        };
        let mut sig = match self.cx.objs.get(&d).copied() {
            Some(Obj::Signal { sig }) => Ok(sig),
            _ => Err(self.sig_of(d, span)?),
        };
        let mut dope = self.sig_dope(d, span)?;
        let tid = self.decl_ty(d, span)?;
        let logic = self.is_logic(tid);
        let what = self.db.decls[d].name.to_string();
        for step in path {
            let Some(dp) = dope else {
                return nested(self);
            };
            let sel = match step {
                LspStep::Rebase(st) => {
                    dope = Some(self.dope_as(dp, Some(*st), &what, span)?);
                    continue;
                }
                LspStep::Index(ix) if ix.len() == 1 => Ok(ix[0]),
                LspStep::Slice(r) => Err(r),
                _ => return nested(self),
            };
            let (r, nd) = self.select_in(sig, dp, logic, &what, sel, span)?;
            (sig, dope) = (Err(r), nd);
        }
        match sig {
            Ok(_) => self.sig_of(d, span),
            Err(r) => Ok(r),
        }
    }

    /// Signals named in `e` (the implicit sensitivity of `wait until`, LRM 10.2).
    pub fn signals_in(&self, e: ExprId, out: &mut Vec<Lsp>) {
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
