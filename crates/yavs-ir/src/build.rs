//! Building programs: lowering (and hand-written tests) use these.

use yavs_diag::Span;

use crate::verify::{VerifyError, verify};
use crate::{
    AddrSpace, Block, BlockId, CaseTable, CaseTableId, CheckMsg, CheckMsgId, ConstData, ConstId,
    CovInfo, CovItem, ForeignId, ForeignRef, Func, FuncFlags, FuncId, FuncKind, IdVec, Inst, Local,
    Mode, NativeId, NativeRef, Param, ParamClass, Program, Refl, ReflId, Reg, RegTy, Regs,
    RootKind, StmtId, StmtInfo, StmtKind, Ty, TyId, Width,
};

#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct ProgramBuilder {
    pub(crate) prog: Program,
    /// Empty in a serialised builder (nothing is serialised while a lowering has failed).
    #[serde(skip)]
    errors: Vec<VerifyError>,
    /// Functions below this id passed the verifier in a [`ProgramBuilder::snapshot`].
    verified: usize,
    /// Functions below this id are propagated and numbered in the program held: set by
    /// [`ProgramBuilder::snapshot_take`], whose program comes back with
    /// [`ProgramBuilder::restore`].
    processed: usize,
    /// Functions below this index are propagated in the program held (see `snapshot`).
    propagated: usize,
}

impl ProgramBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn ty(&mut self, t: Ty) -> TyId {
        self.prog.types.push(t)
    }

    /// Replace type `id` (recursive types: an access type is pushed before the record that
    /// designates it and completed afterwards).
    pub fn set_ty(&mut self, id: TyId, t: Ty) {
        if let Some(x) = self.prog.types.get_mut(id) {
            *x = t;
        }
    }

    /// A GC root at `off` in the shared variables area (`Program::shared_roots`).
    pub fn shared_root(&mut self, off: u32, kind: RootKind) {
        self.prog.shared_roots.push((off, kind));
    }

    pub fn konst(&mut self, c: ConstData) -> ConstId {
        self.prog.consts.push(c)
    }

    pub fn native(&mut self, symbol: &str, ty: Option<TyId>) -> NativeId {
        self.prog.natives.push(NativeRef {
            symbol: symbol.to_string(),
            ty,
        })
    }

    pub fn foreign(&mut self, f: ForeignRef) -> ForeignId {
        self.prog.foreign.push(f)
    }

    pub fn check_msg(&mut self, what: &str, dir: crate::Dir) -> CheckMsgId {
        self.check_msg_for(what, dir, "")
    }

    /// A check message naming the subtype: `value 101 outside of PCT range 0 to 100 for
    /// variable P` (`what` = "variable P", `subtype` = "PCT").
    pub fn check_msg_for(&mut self, what: &str, dir: crate::Dir, subtype: &str) -> CheckMsgId {
        self.prog.checks.push(CheckMsg {
            what: what.to_string(),
            dir,
            subtype: subtype.to_string(),
        })
    }

    pub fn case_table(&mut self, t: CaseTable) -> CaseTableId {
        self.prog.cases.push(t)
    }

    pub fn cov(&mut self, c: CovInfo) -> CovItem {
        self.prog.cov.push(c)
    }

    /// A condition tree for `Inst::CoverCond`: its index.
    pub fn cov_cond(&mut self, c: crate::CovCond) -> u32 {
        self.prog.cov_conds.push(c);
        self.prog.cov_conds.len() as u32 - 1
    }

    pub fn reflect(&mut self, r: Refl) -> ReflId {
        self.prog.reflect.push(r)
    }

    /// Replace a reflection entry (a placeholder pushed before its element subtypes, so
    /// recursive types through access values terminate).
    pub fn reflect_set(&mut self, id: ReflId, r: Refl) {
        if let Some(e) = self.prog.reflect.get_mut(id) {
            *e = r;
        }
    }

    /// Bytes held by the constant pool's byte strings.
    pub fn const_bytes(&self) -> usize {
        self.prog
            .consts
            .values()
            .iter()
            .map(|c| match c {
                ConstData::Bytes(b) => b.len(),
                _ => 0,
            })
            .sum()
    }

    /// The functions that call one of `seed`, directly or through other functions, found by
    /// reading the code lowered so far.
    pub fn callers_of(&self, seed: &[FuncId]) -> Vec<FuncId> {
        let n = self.prog.funcs.len();
        let mut callers: Vec<Vec<u32>> = vec![Vec::new(); n];
        for (id, f) in self.prog.funcs.iter() {
            for inst in &f.code {
                if let Some(c) = callee(inst)
                    && let Some(v) = callers.get_mut(c.0 as usize)
                {
                    v.push(id.0);
                }
            }
        }
        let mut seen = vec![false; n];
        let mut work: Vec<u32> = seed.iter().map(|f| f.0).collect();
        let mut out = Vec::new();
        while let Some(f) = work.pop() {
            for &c in callers.get(f as usize).into_iter().flatten() {
                if !seen[c as usize] {
                    seen[c as usize] = true;
                    out.push(FuncId(c));
                    work.push(c);
                }
            }
        }
        out
    }

    /// A digest of the id space built so far: every function's identity in id order and the size
    /// of every table. Two lowerings that hand out the same ids have the same digest whatever
    /// order their maps were written in (`DefaultHasher::new` has fixed keys, so it agrees
    /// between processes of one build).
    pub fn fingerprint(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let p = &self.prog;
        let mut h = std::hash::DefaultHasher::new();
        for f in p.funcs.values() {
            (
                &f.name,
                &f.mangled,
                f.kind,
                f.parent,
                f.params.len(),
                f.nested,
                f.defined,
            )
                .hash(&mut h);
        }
        (
            p.funcs.len(),
            p.types.len(),
            p.consts.len(),
            p.natives.len(),
            p.foreign.len(),
            p.checks.len(),
            p.cases.len(),
            p.stmts.len(),
            p.reflect.len(),
        )
            .hash(&mut h);
        h.finish()
    }

    /// The functions declared and not defined (their lowering failed or has not run).
    pub fn undefined_funcs(&self) -> Vec<FuncId> {
        self.prog
            .funcs
            .iter()
            .filter(|(_, f)| !f.defined)
            .map(|(id, _)| id)
            .collect()
    }

    /// The number of functions declared so far.
    pub fn func_count(&self) -> usize {
        self.prog.funcs.len()
    }

    pub fn types(&self) -> &IdVec<TyId, Ty> {
        &self.prog.types
    }

    pub fn consts(&self) -> &IdVec<ConstId, ConstData> {
        &self.prog.consts
    }

    /// [`Ty::layout`] against the types and constants built so far (static layouts of
    /// constants).
    pub fn layout(
        &self,
        ty: TyId,
        desc: &[u8],
    ) -> Result<(crate::Layout, usize), crate::LayoutError> {
        Ty::layout(&self.prog.types, &self.prog.consts, ty, desc)
    }

    /// Declare a function so it can be called before it is defined.
    pub fn declare(&mut self, name: &str, kind: FuncKind, parent: Option<FuncId>) -> FuncId {
        let id = self.prog.funcs.push(Func {
            name: name.to_string(),
            kind,
            mangled: name.to_string(),
            parent,
            params: Vec::new(),
            locals: Vec::new(),
            frame: 0,
            roots: Vec::new(),
            reg_ty: Vec::new(),
            blocks: IdVec::new(),
            code: Vec::new(),
            loc: Vec::new(),
            operands: Vec::new(),
            via: Vec::new(),
            via_nodes: Vec::new(),
            resume: Vec::new(),
            flags: FuncFlags::default(),
            body: None,
            size: 0,
            defined: false,
            nested: false,
            spare: false,
            ret_ty: None,
            opt: Default::default(),
            memo: Default::default(),
            lazy: None,
            fetches: Default::default(),
        });
        if let Some(p) = parent.and_then(|p| self.prog.funcs.get_mut(p)) {
            p.nested = true;
        }
        id
    }

    /// Start the body of a declared function.
    pub fn define(&mut self, f: FuncId) -> FuncBuilder<'_> {
        FuncBuilder {
            pb: self,
            id: f,
            params: Vec::new(),
            locals: Vec::new(),
            frame: 0,
            roots: Vec::new(),
            reg_ty: Vec::new(),
            blocks: vec![Vec::new()],
            cur: 0,
            stmt: None,
            operands: Vec::new(),
            flags: FuncFlags::default(),
            body: None,
            ret_ty: None,
        }
    }

    /// Propagate flags, number resume points and verify. Errors are internal compiler
    /// errors of the producer.
    pub fn finish(self) -> Result<Program, Vec<VerifyError>> {
        // Functions a snapshot verified already are not verified again (see `snapshot`).
        let (verified, processed) = (self.verified, self.processed);
        self.finish_from(verified, processed)
    }

    /// [`ProgramBuilder::finish`] on a copy, building goes on. Only the functions added since
    /// the last snapshot are verified again: a function never changes once defined, and what
    /// is added later cannot make an earlier one invalid.
    pub fn snapshot(&mut self) -> Result<Program, Vec<VerifyError>> {
        let n = self.prog.funcs.len();
        // Propagate in place: the next snapshot starts from this builder, not from the copy.
        propagate(&mut self.prog, self.propagated);
        let p = self.clone().finish_from(self.verified, 0)?;
        if eager_verify() {
            self.verified = n;
        }
        self.propagated = n;
        Ok(p)
    }

    /// [`ProgramBuilder::snapshot`] without the copy: the program leaves the builder, which
    /// holds an empty one until the caller hands the same program back with
    /// [`ProgramBuilder::restore`] to go on building. Only the functions added since the last
    /// call are propagated, numbered and verified.
    pub fn snapshot_take(&mut self) -> Result<Program, Vec<VerifyError>> {
        let n = self.prog.funcs.len();
        let b = ProgramBuilder {
            prog: std::mem::take(&mut self.prog),
            errors: self.errors.clone(),
            verified: self.verified,
            processed: self.processed,
            propagated: self.propagated,
        };
        let p = b.finish_from(self.verified, self.processed)?;
        if eager_verify() {
            self.verified = n;
        }
        (self.processed, self.propagated) = (n, n);
        Ok(p)
    }

    /// Verify what is not verified yet, so that the program can be stored and read back without
    /// the verification of first use (a stored program is verified when it is made, #479).
    pub fn verify_now(&mut self) -> Result<(), Vec<VerifyError>> {
        self.prog.verify_pending()?;
        self.verified = self.prog.funcs.len();
        Ok(())
    }

    /// Put back the program [`ProgramBuilder::snapshot_take`] returned.
    pub fn restore(&mut self, prog: Program) {
        self.prog = prog;
    }

    fn finish_from(
        mut self,
        verified: usize,
        processed: usize,
    ) -> Result<Program, Vec<VerifyError>> {
        let prog = &mut self.prog;
        for (id, f) in prog.funcs.iter() {
            if !f.defined {
                self.errors.push(VerifyError::new(
                    id,
                    &f.name,
                    None,
                    "declared but never defined",
                ));
            }
        }
        if !self.errors.is_empty() {
            return Err(self.errors);
        }
        propagate(prog, processed);
        number_resumes(prog, processed);
        let errors = if eager_verify() {
            verify(prog, verified)
        } else {
            // Verified when `Program::func` first hands each out (#482).
            let v = verified as u32;
            prog.lazy_verify = Some(prog.lazy_verify.map_or(v, |l| l.min(v)));
            Vec::new()
        };
        if errors.is_empty() {
            Ok(self.prog)
        } else {
            Err(errors)
        }
    }
}

/// Whether [`ProgramBuilder::finish`] verifies every function: debug builds (and so the
/// tests) and runs with `YAVS_VERIFY_IR` set. Other builds verify a function when
/// [`Program::func`] first hands it out (`Program::optimized`), so the functions a run never
/// calls cost nothing (#479).
pub(crate) fn eager_verify() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| cfg!(debug_assertions) || std::env::var_os("YAVS_VERIFY_IR").is_some())
}

/// Callees of `inst` that propagate flags to the caller.
fn callee(inst: &Inst) -> Option<FuncId> {
    match *inst {
        Inst::Call { func, .. } | Inst::CallProt { func, .. } => Some(func),
        _ => None,
    }
}

/// Natives that read or write disk files or directories. File objects of different processes
/// can name the same external file, so their callers are serial: the bytes on disk and what a
/// process reads back do not depend on `--threads` (#82, #107). Includes textio `OUTPUT`, which
/// the kernel orders anyway.
// ponytail: also serialises processes that only write to OUTPUT; tell them apart by the file
// argument if serial testbench processes ever dominate.
fn touches_files(symbol: &str) -> bool {
    const FILE_OPS: [&str; 13] = [
        "file_open",
        "file_close",
        "read",
        "write",
        "flush",
        "endfile",
        "file_state",
        "file_mode",
        "file_position",
        "file_size",
        "file_seek",
        "file_truncate",
        "file_rewind",
    ];
    if let Some(rest) = symbol.strip_prefix("yavs.implicit.") {
        return FILE_OPS.contains(&rest.split('.').next().unwrap_or(rest));
    }
    [
        "yavs.textio.readline.",
        "yavs.textio.writeline.",
        "yavs.textio.tee.",
        "yavs.env.dir_",
    ]
    .iter()
    .any(|p| symbol.starts_with(p))
}

/// MAY_SUSPEND and TOUCHES_SHARED over the reverse call graph; each edge is visited once. Only
/// functions from `first` on are scanned and changed: the earlier ones are final (they
/// cannot call a later function), so their flags only seed the later callers'.
fn propagate(prog: &mut Program, first: usize) {
    let n = prog.funcs.len();
    let mut callers: Vec<Vec<u32>> = vec![Vec::new(); n];
    let mut flags: Vec<FuncFlags> = prog.funcs.values().iter().map(|f| f.flags).collect();
    // Functions that may leave temporaries in the caller's memory (`FuncFlags::NO_TEMPS` is the
    // rest): the local cause, then the callers of those.
    let mut leaves: Vec<bool> = flags
        .iter()
        .enumerate()
        .map(|(i, f)| i < first && !f.contains(FuncFlags::NO_TEMPS))
        .collect();
    for (id, f) in prog.funcs.iter().skip(first) {
        let fl = &mut flags[id.0 as usize];
        // (A function declared and never defined has no code to judge.)
        let mut lt = !f.defined;
        for inst in &f.code {
            lt |= crate::opt::leaves_temps(inst, &f.reg_ty, &prog.natives, &|_| true);
            match *inst {
                Inst::Suspend { .. } => fl.insert(FuncFlags::MAY_SUSPEND),
                Inst::AddrOf {
                    space: AddrSpace::Shared,
                    ..
                }
                | Inst::CallProt { .. }
                | Inst::New { shared: true, .. }
                | Inst::NewFill { shared: true, .. } => fl.insert(FuncFlags::TOUCHES_SHARED),
                Inst::CallNative { native, .. }
                    if prog
                        .natives
                        .get(native)
                        .is_some_and(|n| touches_files(&n.symbol)) =>
                {
                    fl.insert(FuncFlags::TOUCHES_FILES)
                }
                Inst::Cover { .. } | Inst::CoverIn { .. } | Inst::CoverCond { .. } => {
                    fl.insert(FuncFlags::HAS_COVER)
                }
                _ => {}
            }
            if let Some(c) = callee(inst)
                && let Some(v) = callers.get_mut(c.0 as usize)
            {
                v.push(id.0);
            }
        }
        leaves[id.0 as usize] = lt;
    }
    // (A call whose result is not a scalar counts above for every callee; the callee's own
    // `leaves` reaches its callers here.)
    let mut work: Vec<u32> = (0..n as u32).filter(|&i| leaves[i as usize]).collect();
    while let Some(f) = work.pop() {
        for &c in &callers[f as usize] {
            if !leaves[c as usize] {
                leaves[c as usize] = true;
                work.push(c);
            }
        }
    }
    for (i, l) in leaves.iter().enumerate().skip(first) {
        if !l {
            flags[i].insert(FuncFlags::NO_TEMPS);
        }
    }
    for bit in [
        FuncFlags::MAY_SUSPEND,
        FuncFlags::TOUCHES_SHARED,
        FuncFlags::TOUCHES_FILES,
    ] {
        let mut work: Vec<u32> = (0..n as u32)
            .filter(|&i| flags[i as usize].contains(bit))
            .collect();
        while let Some(f) = work.pop() {
            for &c in &callers[f as usize] {
                // A function never suspends (H11); the verifier reports such a call.
                let is_fn = prog
                    .funcs
                    .get(FuncId(c))
                    .is_some_and(|f| f.kind == FuncKind::Function);
                if bit == FuncFlags::MAY_SUSPEND && is_fn {
                    continue;
                }
                if !flags[c as usize].contains(bit) {
                    flags[c as usize].insert(bit);
                    work.push(c);
                }
            }
        }
    }
    for (i, fl) in flags.into_iter().enumerate().skip(first) {
        if let Some(f) = prog.funcs.get_mut(FuncId(i as u32)) {
            f.flags = fl;
        }
    }
}

fn number_resumes(prog: &mut Program, first: usize) {
    let suspending: Vec<bool> = prog
        .funcs
        .values()
        .iter()
        .map(|f| f.flags.contains(FuncFlags::MAY_SUSPEND))
        .collect();
    for i in first..prog.funcs.len() {
        if let Some(f) = prog.funcs.get_mut(FuncId(i as u32)) {
            number_func(f, |c| suspending.get(c.0 as usize) == Some(&true));
        }
    }
}

/// Number the resume points of one function and set its size.
pub(crate) fn number_func(f: &mut Func, suspending: impl Fn(FuncId) -> bool) {
    f.resume.clear();
    for pc in 0..f.code.len() {
        let n = f.resume.len() as u32;
        match &mut f.code[pc] {
            Inst::Suspend { resume } => {
                *resume = n;
                f.resume.push(pc as u32 + 1);
            }
            Inst::Call { func, .. } if suspending(*func) => {
                f.resume.push(pc as u32 + 1);
            }
            _ => {}
        }
    }
    f.size = f.code.len() as u32;
}

/// Builds one function body. Instructions go into the current block and carry the current
/// statement.
pub struct FuncBuilder<'a> {
    pb: &'a mut ProgramBuilder,
    id: FuncId,
    params: Vec<Param>,
    locals: Vec<Local>,
    frame: u32,
    roots: Vec<(u32, RootKind)>,
    reg_ty: Vec<RegTy>,
    blocks: Vec<Vec<(Inst, Option<StmtId>)>>,
    cur: usize,
    stmt: Option<StmtId>,
    operands: Vec<Reg>,
    flags: FuncFlags,
    body: Option<BlockId>,
    ret_ty: Option<TyId>,
}

impl FuncBuilder<'_> {
    pub fn id(&self) -> FuncId {
        self.id
    }

    pub fn program(&mut self) -> &mut ProgramBuilder {
        self.pb
    }

    pub fn reg(&mut self, t: RegTy) -> Reg {
        let r = Reg(self.reg_ty.len() as u32);
        self.reg_ty.push(t);
        if t == RegTy::Sig {
            self.reg_ty.push(RegTy::I64); // the length half
        }
        r
    }

    /// A frame slot of `size` bytes aligned to `align`; returns its offset.
    pub fn slot(&mut self, size: u32, align: u32) -> u32 {
        let off = self.frame.next_multiple_of(align.max(1));
        self.frame = off + size;
        off
    }

    /// A named variable of `size` bytes (visible to debuggers); returns its slot.
    pub fn local(&mut self, name: &str, ty: TyId, size: u32, align: u32) -> u32 {
        let slot = self.slot(size, align);
        self.locals.push(Local {
            name: name.to_string(),
            slot,
            size,
            ty,
        });
        slot
    }

    pub fn root(&mut self, off: u32, kind: RootKind) {
        self.roots.push((off, kind));
    }

    /// A parameter; with a type it is also visible to debuggers as a local.
    pub fn param(
        &mut self,
        name: &str,
        width: Width,
        class: ParamClass,
        mode: Mode,
        ty: Option<TyId>,
    ) -> u32 {
        let slot = self.slot(width.bytes(), 8);
        self.params.push(Param {
            name: name.to_string(),
            slot,
            width,
            class,
            mode,
            ty,
            arr: None,
        });
        if let Some(ty) = ty {
            self.locals.push(Local {
                name: name.to_string(),
                slot,
                size: width.bytes(),
                ty,
            });
        }
        slot
    }

    /// Marks `b` as where a process's statements start (its declarative part ends).
    pub fn body(&mut self, b: BlockId) {
        self.body = Some(b);
    }

    pub fn flag(&mut self, f: FuncFlags) {
        self.flags.insert(f);
    }

    /// The parameter at `slot` is an array passed by value of type `ty` (its descriptor is the
    /// next parameter): what the memo of pure calls reads its bytes by (`Param::arr`).
    pub fn param_array(&mut self, slot: u32, ty: TyId) {
        if let Some(p) = self.params.iter_mut().find(|p| p.slot == slot) {
            p.arr = Some(ty);
        }
    }

    /// The function's result type (`Func::ret_ty`).
    pub fn ret_ty(&mut self, ty: TyId) {
        self.ret_ty = Some(ty);
    }

    /// The entry block is `BlockId(0)`.
    pub fn block(&mut self) -> BlockId {
        self.blocks.push(Vec::new());
        BlockId(self.blocks.len() as u32 - 1)
    }

    pub fn at(&mut self, b: BlockId) {
        self.cur = b.0 as usize;
    }

    /// The block instructions go to now.
    pub fn current(&self) -> BlockId {
        BlockId(self.cur as u32)
    }

    /// Instructions emitted so far into `b`.
    pub fn block_len(&self, b: BlockId) -> usize {
        self.blocks.get(b.0 as usize).map_or(0, Vec::len)
    }

    /// Take the instructions out of `b`, with their statements (for `splice` into another block).
    pub fn take(&mut self, b: BlockId) -> Vec<(Inst, Option<StmtId>)> {
        self.blocks
            .get_mut(b.0 as usize)
            .map(std::mem::take)
            .unwrap_or_default()
    }

    /// Put `code` into block `b` before its instruction number `at`.
    pub fn splice(&mut self, b: BlockId, at: usize, code: Vec<(Inst, Option<StmtId>)>) {
        if let Some(blk) = self.blocks.get_mut(b.0 as usize) {
            blk.splice(at..at, code);
        }
    }

    /// The statement new instructions belong to; returns the previous one.
    pub fn set_stmt(&mut self, s: Option<StmtId>) -> Option<StmtId> {
        std::mem::replace(&mut self.stmt, s)
    }

    /// Start a statement: records it in the program's statement table and emits `Stmt`.
    pub fn stmt(&mut self, kind: StmtKind, span: Span) -> StmtId {
        let id = self.pb.prog.stmts.push(StmtInfo {
            span,
            func: self.id,
            kind,
            parent: None,
        });
        self.stmt = Some(id);
        self.emit(Inst::Stmt);
        id
    }

    pub fn regs(&mut self, rs: &[Reg]) -> Regs {
        let start = self.operands.len() as u32;
        self.operands.extend_from_slice(rs);
        Regs {
            start,
            len: rs.len() as u32,
        }
    }

    /// The registers of an operand list made by [`FuncBuilder::regs`].
    pub fn list(&self, r: Regs) -> Option<&[Reg]> {
        self.operands
            .get(r.start as usize..r.start as usize + r.len as usize)
    }

    pub fn emit(&mut self, i: Inst) {
        let s = self.stmt;
        if let Some(b) = self.blocks.get_mut(self.cur) {
            b.push((i, s));
        }
    }

    /// `Const` into a new I64 register.
    pub fn int(&mut self, v: i64) -> Reg {
        let dst = self.reg(RegTy::I64);
        self.emit(Inst::Const { dst, v });
        dst
    }

    pub fn addr(&mut self, space: AddrSpace, off: u32) -> Reg {
        let dst = self.reg(RegTy::Ptr);
        self.emit(Inst::AddrOf { dst, space, off });
        dst
    }

    pub fn finish(self) {
        let mut code = Vec::new();
        let mut loc = Vec::new();
        let mut blocks = IdVec::new();
        let mut missing = false;
        for b in self.blocks {
            let start = code.len() as u32;
            for (i, s) in b {
                code.push(i);
                loc.push(s.unwrap_or_else(|| {
                    missing = true;
                    StmtId(u32::MAX)
                }));
            }
            blocks.push(Block {
                start,
                end: code.len() as u32,
            });
        }
        let Some(f) = self.pb.prog.funcs.get_mut(self.id) else {
            return;
        };
        if missing {
            let e = VerifyError::new(self.id, &f.name, None, "instruction before any statement");
            self.pb.errors.push(e);
        }
        f.params = self.params;
        f.locals = self.locals;
        f.frame = self.frame;
        f.roots = self.roots;
        f.reg_ty = self.reg_ty;
        f.blocks = blocks;
        f.code = code;
        f.loc = loc;
        f.operands = self.operands;
        f.flags = self.flags;
        f.body = self.body;
        f.ret_ty = self.ret_ty;
        f.defined = true;
        crate::casetab::tabulate(self.pb, self.id);
    }
}
