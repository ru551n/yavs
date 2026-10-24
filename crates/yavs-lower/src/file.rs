//! File objects (DESIGN.md "Composites", ir.md 9 "files"): a 4-byte cell holding the
//! runtime's file index plus one (0: not open). Natives take the cell's address; textio's
//! `INPUT` and `OUTPUT` are the pre-opened cells `FILE_INPUT` and `FILE_OUTPUT`.

use yavs_diag::Span;
use yavs_ir::{AddrSpace, ConstData, FuncKind, Reg, RegTy, Width};
use yavs_sem::decl::DeclKind;
use yavs_sem::implicit::{Family, Form, ImplicitOp};
use yavs_sem::{DeclId, ScopeId};

use crate::expr::Conv;
use crate::expr::V;
use crate::func::Fx;
use crate::{Obj, R, Space};

/// yavs-interp's pre-opened cells of textio's `INPUT` and `OUTPUT`.
const FILE_INPUT: u32 = 1;
const FILE_OUTPUT: u32 = 2;

impl Fx<'_, '_> {
    /// A file object declared in a process or subprogram, opened when its declaration says so.
    pub fn local_file(&mut self, d: DeclId) -> R<()> {
        let db = self.db;
        let decl = &db.decls[d];
        let DeclKind::Object(o) = &decl.kind else {
            return Err(self.cx.ice(decl.span, "not an object"));
        };
        let span = decl.span;
        let t = db.base(o.subtype);
        let tid = self.ty(t, span)?;
        let off = self.fb.local(&decl.name.to_string(), tid, 4, 4);
        let a = self.addr(Space::Frame(self.depth), off);
        let z = self.int(0);
        self.store(a, z, Width::B4);
        self.cx.objs.insert(
            d,
            Obj::Scalar {
                space: Space::Frame(self.depth),
                off,
                w: Width::B4,
            },
        );
        if matches!(self.kind, FuncKind::Procedure | FuncKind::Function) {
            let op = ImplicitOp {
                family: Family::FileClose,
                ty: t,
                form: Form::Plain,
            };
            let sym = yavs_sem::mangle::implicit_native(db, &op);
            self.local_files.push((off, sym, tid));
        }
        self.open_file(d, Space::Frame(self.depth), off)
    }

    /// The cell of file object `d` of an architecture: a closed cell in the shared heap,
    /// opened as its declaration says; returns its address.
    pub fn file_region(&mut self, d: DeclId) -> R<Reg> {
        let db = self.db;
        let decl = &db.decls[d];
        let DeclKind::Object(o) = &decl.kind else {
            return Err(self.cx.ice(decl.span, "not an object"));
        };
        let tid = self.ty(db.base(o.subtype), decl.span)?;
        let buf = self.alloc_n(8);
        let z = self.int(0);
        self.store(buf, z, Width::B8);
        let size = self.int(8);
        let h = self.reg(RegTy::Handle);
        self.emit(yavs_ir::Inst::New {
            dst: h,
            ty: tid,
            init: buf,
            size,
            shared: true,
        });
        let what = self.msg(&decl.name.to_string(), yavs_ir::Dir::To);
        let p = self.reg(RegTy::Ptr);
        self.emit(yavs_ir::Inst::Deref {
            dst: p,
            handle: h,
            what,
        });
        self.open_file_at(d, p)?;
        Ok(p)
    }

    /// Open file object `d` (its cell at `off` in `space`) as its declaration says, if it does.
    pub fn open_file(&mut self, d: DeclId, space: Space, off: u32) -> R<()> {
        let a = self.addr(space, off);
        self.open_file_at(d, a)
    }

    /// Open file object `d`, whose cell is at address `a`, as its declaration says, if it does.
    pub fn open_file_at(&mut self, d: DeclId, a: Reg) -> R<()> {
        let db = self.db;
        let decl = &db.decls[d];
        let DeclKind::Object(o) = &decl.kind else {
            return Err(self.cx.ice(decl.span, "not an object"));
        };
        let span = decl.span;
        let t = db.base(o.subtype);
        let tid = self.ty(t, span)?;
        let Some((kind, name)) = o.file_open else {
            return Ok(());
        };
        let v = self.expr(name)?;
        let (data, dope) = self.arr(v, span)?;
        let fat = self.make_fat(data, dope);
        let kind = match kind {
            Some(k) => self.scalar_expr(k)?,
            None => self.int(0), // read_mode
        };
        let op = ImplicitOp {
            family: Family::FileOpen,
            ty: t,
            form: Form::Plain,
        };
        let sym = yavs_sem::mangle::implicit_native(db, &op);
        self.native(&sym, Some(tid), &[a, fat, kind], None, span)?;
        Ok(())
    }

    /// The cell address of a file object that is not a lowered object: textio's `INPUT` and
    /// `OUTPUT`.
    pub fn file_cell(&mut self, d: DeclId, span: Span) -> R<Reg> {
        let db = self.db;
        let decl = &db.decls[d];
        let in_textio = |s: ScopeId| {
            db.scopes[s]
                .owner
                .is_some_and(|o| db.decls[o].name.to_string().eq_ignore_ascii_case("textio"))
        };
        let cell = match decl.name.to_string().to_ascii_lowercase().as_str() {
            "input" if in_textio(decl.scope) => FILE_INPUT,
            "output" if in_textio(decl.scope) => FILE_OUTPUT,
            _ => {
                return Err(self
                    .cx
                    .unsupported(span, "file objects declared in packages (M3)"));
            }
        };
        let c = self
            .fb
            .program()
            .konst(ConstData::Bytes(cell.to_le_bytes().to_vec()));
        let dst = self.reg(RegTy::Ptr);
        self.emit(yavs_ir::Inst::AddrOf {
            dst,
            space: AddrSpace::Const(c),
            off: 0,
        });
        Ok(dst)
    }

    /// An implicit file operation (`file_open`, `endfile`, `read`, ...): a native taking
    /// the file cell's address.
    pub fn file_op(
        &mut self,
        op: &ImplicitOp,
        sp: &yavs_sem::decl::Subprogram,
        args: &[yavs_sem::hir::Arg],
        span: Span,
    ) -> R<Option<V>> {
        let cells = self.cells(sp, args, span, Conv::Fat)?;
        let tid = self.ty(op.ty, span)?;
        let sym = yavs_sem::mangle::implicit_native(self.db, op);
        let r = self.native(&sym, Some(tid), &cells, sp.ret, span)?;
        self.copy_out(span)?;
        Ok(r)
    }
}
