//! Building programs: lowering (and hand-written tests) use these.

use yavs_diag::Span;

use crate::verify::{VerifyError, verify};
use crate::{
    AddrSpace, Block, BlockId, CaseTable, CaseTableId, CheckMsg, CheckMsgId, ConstData, ConstId,
    CovInfo, CovItem, ForeignId, ForeignRef, Func, FuncFlags, FuncId, FuncKind, IdVec, Inst, Local,
    Mode, NativeId, NativeRef, Param, ParamClass, Program, Refl, ReflId, Reg, RegTy, Regs,
    RootKind, StmtId, StmtInfo, StmtKind, Ty, TyId, Width,
};

#[derive(Default)]
pub struct ProgramBuilder {
    prog: Program,
    errors: Vec<VerifyError>,
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

    pub fn types(&self) -> &IdVec<TyId, Ty> {
        &self.prog.types
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
        self.prog.funcs.push(Func {
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
            resume: Vec::new(),
            flags: FuncFlags::default(),
            size: 0,
            defined: false,
        })
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
        }
    }

    /// Propagate flags, number resume points and verify. Errors are internal compiler
    /// errors of the producer.
    pub fn finish(mut self) -> Result<Program, Vec<VerifyError>> {
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
        propagate(prog);
        number_resumes(prog);
        let errors = verify(prog);
        if errors.is_empty() {
            Ok(self.prog)
        } else {
            Err(errors)
        }
    }
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

/// MAY_SUSPEND and TOUCHES_SHARED over the reverse call graph; each edge is visited once.
fn propagate(prog: &mut Program) {
    let n = prog.funcs.len();
    let mut callers: Vec<Vec<u32>> = vec![Vec::new(); n];
    let mut flags: Vec<FuncFlags> = Vec::with_capacity(n);
    for (id, f) in prog.funcs.iter() {
        let mut fl = f.flags;
        for inst in &f.code {
            match *inst {
                Inst::Suspend { .. } => fl.insert(FuncFlags::MAY_SUSPEND),
                Inst::AddrOf {
                    space: AddrSpace::Shared,
                    ..
                }
                | Inst::CallProt { .. }
                | Inst::New { shared: true, .. } => fl.insert(FuncFlags::TOUCHES_SHARED),
                Inst::CallNative { native, .. }
                    if prog
                        .natives
                        .get(native)
                        .is_some_and(|n| touches_files(&n.symbol)) =>
                {
                    fl.insert(FuncFlags::TOUCHES_SHARED)
                }
                Inst::Cover { .. } => fl.insert(FuncFlags::HAS_COVER),
                _ => {}
            }
            if let Some(c) = callee(inst)
                && let Some(v) = callers.get_mut(c.0 as usize)
            {
                v.push(id.0);
            }
        }
        flags.push(fl);
    }
    for bit in [FuncFlags::MAY_SUSPEND, FuncFlags::TOUCHES_SHARED] {
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
    for (i, fl) in flags.into_iter().enumerate() {
        if let Some(f) = prog.funcs.get_mut(FuncId(i as u32)) {
            f.flags = fl;
        }
    }
}

fn number_resumes(prog: &mut Program) {
    let suspending: Vec<bool> = prog
        .funcs
        .values()
        .iter()
        .map(|f| f.flags.contains(FuncFlags::MAY_SUSPEND))
        .collect();
    for i in 0..prog.funcs.len() {
        let Some(f) = prog.funcs.get_mut(FuncId(i as u32)) else {
            continue;
        };
        f.resume.clear();
        for pc in 0..f.code.len() {
            let n = f.resume.len() as u32;
            match &mut f.code[pc] {
                Inst::Suspend { resume } => {
                    *resume = n;
                    f.resume.push(pc as u32 + 1);
                }
                Inst::Call { func, .. } if suspending.get(func.0 as usize) == Some(&true) => {
                    f.resume.push(pc as u32 + 1);
                }
                _ => {}
            }
        }
        f.size = f.code.len() as u32;
    }
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

    pub fn flag(&mut self, f: FuncFlags) {
        self.flags.insert(f);
    }

    /// The entry block is `BlockId(0)`.
    pub fn block(&mut self) -> BlockId {
        self.blocks.push(Vec::new());
        BlockId(self.blocks.len() as u32 - 1)
    }

    pub fn at(&mut self, b: BlockId) {
        self.cur = b.0 as usize;
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
        f.defined = true;
    }
}
