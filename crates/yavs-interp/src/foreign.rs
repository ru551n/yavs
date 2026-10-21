//! VHPIDIRECT calls (ir.md 1.7): marshalling VHDL values to C arguments and back.

use yavs_ffi::{Arg, CType, Ret};
use yavs_ir::{
    ForeignArg, ForeignId, ForeignMode, Func, LogicKind, Program, Regs, Ty, TyId, VhpiIndex,
    VhpiModel, Width,
};

use yavs_kernel::{Ctx, DelayMode, Elem, Time, Value};

use crate::exec::{Fault, Machine, R, add_addr, ice, rt_err};
use crate::heap::Obj;

/// The scalar type of a model call's value or array element.
#[derive(Clone, Debug, PartialEq)]
pub enum ScalarKind {
    Int,
    Real,
    /// A physical type; `time` for the predefined `time` (primary unit `fs`).
    Phys {
        time: bool,
    },
    /// Literals in original spelling; `logic` for std_ulogic and BIT types.
    Enum {
        lits: Vec<String>,
        logic: Option<LogicKind>,
    },
    /// An access value: the one cell is the address of the designated object's data, 0 for
    /// null.
    Access,
    /// A record: its scalar fields are the argument's `fields`.
    Record,
}

/// One actual of a model call (or its function result). `cells` are the VHDL cells of the
/// scalar or of the array elements, row-major, leftmost first: integers and physical values
/// as `i64`, reals as `f64` bits, enumerations as literal positions.
#[derive(Clone, Debug, PartialEq)]
pub struct ModelArg {
    pub name: String,
    pub mode: ForeignMode,
    pub kind: ScalarKind,
    pub array: bool,
    pub cells: Vec<u64>,
    /// The number of elements the value has; `None` for an unconstrained array result
    /// before the model sizes it (`vhpiSizeConstraint`).
    pub len: Option<usize>,
    /// False for a function result the model has not set yet.
    pub set: bool,
    /// The fields of a record, in declaration order (each a scalar).
    pub fields: Vec<ModelArg>,
    /// A signal parameter: its value, and what the model drove (`cells` unused).
    pub signal: Option<ModelSignal>,
}

/// A signal parameter of a model call.
#[derive(Clone, Debug, PartialEq)]
pub struct ModelSignal {
    /// The signal's value at the call.
    pub value: Value,
    /// `out`/`inout`: a deposit (`vhpi_put_value`), applied before the transactions.
    pub deposit: Option<Value>,
    /// `out`/`inout`: transactions scheduled on the caller's driver, in order.
    pub txns: Vec<ModelTxn>,
}

/// A transaction a model scheduled on a driver (`vhpi_schedule_transaction`).
#[derive(Clone, Debug, PartialEq)]
pub struct ModelTxn {
    pub value: Value,
    pub delay: Time,
    pub mode: DelayMode,
}

/// A call of a `'FOREIGN "VHPI library model"` subprogram.
#[derive(Clone, Debug, PartialEq)]
pub struct ModelCall {
    pub subprogram: String,
    pub args: Vec<ModelArg>,
    /// The function result, `None` for a procedure.
    pub ret: Option<ModelArg>,
}

/// A foreign model registered by a plugin; bound by the elaborator (`Bindings::models`).
pub trait ForeignModel: Send + Sync {
    /// Runs the model. `out` and `inout` arguments and the result are read back afterwards.
    fn call(&self, call: &mut ModelCall) -> Result<(), String>;
}

fn scalar_kind(t: &Ty) -> Option<ScalarKind> {
    Some(match t {
        Ty::Int { .. } => ScalarKind::Int,
        Ty::Real => ScalarKind::Real,
        Ty::Physical { units, .. } => ScalarKind::Phys {
            time: units
                .first()
                .is_some_and(|u| u.0.eq_ignore_ascii_case("fs")),
        },
        Ty::Enum { lits, logic } => ScalarKind::Enum {
            lits: lits.clone(),
            logic: *logic,
        },
        _ => return None,
    })
}

fn csize(ct: CType) -> usize {
    match ct {
        CType::I8 | CType::U8 => 1,
        CType::I16 | CType::U16 => 2,
        CType::I32 | CType::U32 | CType::F32 => 4,
        CType::I64 | CType::F64 | CType::Ptr => 8,
    }
}

/// The C type of a scalar VHDL type (the sim1 mapping of `yavs_ffi::vhdl`).
fn ctype_of(t: &Ty) -> Option<CType> {
    use yavs_ffi::vhdl::{Mode, VhdlType, lower_param};
    let v = match t {
        Ty::Int { lo, hi } => VhdlType::Integer {
            low: *lo,
            high: *hi,
        },
        Ty::Enum { lits, .. } => VhdlType::Enum {
            literals: lits.len() as u64,
        },
        Ty::Real => VhdlType::Real,
        Ty::Physical { .. } => VhdlType::Physical,
        _ => return None,
    };
    lower_param(v, Mode::In).ok()?.first().copied()
}

pub(crate) fn arg_bits(a: Arg) -> u64 {
    match a {
        Arg::I8(v) => v as u8 as u64,
        Arg::I16(v) => v as u16 as u64,
        Arg::I32(v) => v as u32 as u64,
        Arg::I64(v) => v as u64,
        Arg::U8(v) => v.into(),
        Arg::U16(v) => v.into(),
        Arg::U32(v) => v.into(),
        Arg::F32(v) => v.to_bits().into(),
        Arg::F64(v) => v.to_bits(),
        Arg::Ptr(p) => p as usize as u64,
    }
}

/// A C value's bits back to a VHDL cell (sign-extended integers, f64 reals).
fn from_c_bits(ct: CType, b: u64) -> u64 {
    match ct {
        CType::I8 => b as i8 as i64 as u64,
        CType::I16 => b as i16 as i64 as u64,
        CType::I32 => b as i32 as i64 as u64,
        CType::I64 | CType::Ptr => b,
        CType::U8 => b & 0xff,
        CType::U16 => b & 0xffff,
        CType::U32 => b & 0xffff_ffff,
        CType::F32 => f64::from(f32::from_bits(b as u32)).to_bits(),
        CType::F64 => b,
    }
}

/// A VHDL cell as a C argument, range-checked against the C type.
fn to_c(ct: CType, v: u64, name: &str, k: usize) -> R<Arg> {
    let i = v as i64;
    let range = || {
        Fault::Runtime(format!(
            "argument {} of foreign `{name}`: {i} does not fit C type {ct}",
            k + 1
        ))
    };
    Ok(match ct {
        CType::I8 => Arg::I8(i8::try_from(i).map_err(|_| range())?),
        CType::I16 => Arg::I16(i16::try_from(i).map_err(|_| range())?),
        CType::I32 => Arg::I32(i32::try_from(i).map_err(|_| range())?),
        CType::I64 => Arg::I64(i),
        CType::U8 => Arg::U8(u8::try_from(i).map_err(|_| range())?),
        CType::U16 => Arg::U16(u16::try_from(i).map_err(|_| range())?),
        CType::U32 => Arg::U32(u32::try_from(i).map_err(|_| range())?),
        CType::F32 => Arg::F32(f64::from_bits(v) as f32),
        CType::F64 => Arg::F64(f64::from_bits(v)),
        CType::Ptr => return Err(ice("a scalar foreign argument of C type void *")),
    })
}

fn put_c(buf: &mut [u8], at: usize, ct: CType, v: u64, name: &str, k: usize) -> R<()> {
    let n = csize(ct);
    let b = arg_bits(to_c(ct, v, name, k)?).to_le_bytes();
    buf[at..at + n].copy_from_slice(&b[..n]);
    Ok(())
}

fn get_c(buf: &[u8], at: usize, ct: CType) -> u64 {
    let n = csize(ct);
    let mut b = [0u8; 8];
    b[..n].copy_from_slice(&buf[at..at + n]);
    from_c_bits(ct, u64::from_le_bytes(b))
}

/// Range check of a value C wrote into an `out`/`inout` parameter.
fn check_back(t: &Ty, v: u64, name: &str, k: usize) -> R<()> {
    let bad = match t {
        Ty::Int { lo, hi } | Ty::Physical { lo, hi, .. } => {
            let i = v as i64;
            (i < *lo || i > *hi).then(|| format!("{i} is outside {lo} to {hi}"))
        }
        Ty::Enum { lits, .. } => {
            (v >= lits.len() as u64).then(|| format!("{v} is not a position of the type"))
        }
        _ => None,
    };
    match bad {
        Some(m) => rt_err(format!(
            "foreign `{name}` wrote an invalid value to argument {}: {m}",
            k + 1
        )),
        None => Ok(()),
    }
}

fn width_of(size: u64) -> R<Width> {
    match size {
        1 => Ok(Width::B1),
        4 => Ok(Width::B4),
        8 => Ok(Width::B8),
        _ => Err(ice(format!("scalar of {size} bytes"))),
    }
}

/// How one argument was marshalled, for copy-back.
enum Temp {
    None,
    Scalar(Box<[u64; 1]>, CType, Width),
    Array {
        buf: Vec<u8>,
        data: u64,
        ty: TyId,
        n: u64,
    },
    Record {
        buf: Vec<u8>,
        addr: u64,
        ty: TyId,
    },
}

impl Machine {
    pub(crate) fn ty(&self, t: TyId) -> R<Ty> {
        self.rt
            .prog
            .types
            .get(t)
            .cloned()
            .ok_or_else(|| ice(format!("no type {t}")))
    }

    /// Data address and per-dimension lengths of the fat pair at `fat`.
    pub(crate) fn fat(&self, fat: u64, dims: u8) -> R<(u64, Vec<u64>)> {
        let data = self.load(fat, Width::B8)?;
        let dope = self.load(add_addr(fat, 8)?, Width::B8)?;
        let mut lens = Vec::with_capacity(dims.into());
        for d in 0..u64::from(dims) {
            let at = add_addr(dope, d * 24)?;
            let l = self.load(at, Width::B8)? as i64;
            let r = self.load(add_addr(at, 8)?, Width::B8)? as i64;
            let dir = self.load(add_addr(at, 16)?, Width::B8)?;
            lens.push(
                yavs_ir::dim_len(l, r, dir == 1)
                    .ok_or_else(|| Fault::Runtime("array too long for a foreign call".into()))?,
            );
        }
        Ok((data, lens))
    }

    /// The element type, its C type and whether it is a logic leaf (`Some(is_bit)`).
    fn array_elem(&self, ty: TyId, name: &str) -> R<(Ty, CType, Option<bool>, u8)> {
        let Ty::Array { elem, dims, .. } = self.ty(ty)? else {
            return Err(ice(format!(
                "foreign array argument of non-array type {ty}"
            )));
        };
        let et = self.ty(elem)?;
        let logic = Ty::logic_leaf(&self.rt.prog.types, ty);
        let nested = matches!(et, Ty::Array { .. });
        match ctype_of(&et) {
            Some(ct) if !nested => {
                let bit = logic.map(|k| k != yavs_ir::LogicKind::StdULogic);
                Ok((et, ct, bit, dims))
            }
            _ => rt_err(format!(
                "foreign `{name}`: arrays of composite elements are not supported (ir.md 1.7)"
            )),
        }
    }

    fn marshal_array(&mut self, ty: TyId, fat: u64, name: &str) -> R<(Vec<u8>, u64, Vec<u64>)> {
        let (et, ct, logic, dims) = self.array_elem(ty, name)?;
        let (data, lens) = self.fat(fat, dims)?;
        let n = lens.iter().try_fold(1u64, |a, &l| a.checked_mul(l));
        let n = n
            .filter(|&n| n <= u64::from(u32::MAX))
            .ok_or_else(|| Fault::Runtime(format!("foreign `{name}`: array argument too large")))?;
        let buf = self.marshal_data(&et, ct, logic, data, n, name)?;
        Ok((buf, data, lens))
    }

    /// The `n` elements at `data` as a C array (`marshal_array`).
    fn marshal_data(
        &mut self,
        et: &Ty,
        ct: CType,
        logic: Option<bool>,
        data: u64,
        n: u64,
        name: &str,
    ) -> R<Vec<u8>> {
        let cs = csize(ct);
        let mut buf = vec![0u8; n as usize * cs];
        match logic {
            Some(bit) => {
                let v = self.planes(data, n)?;
                for i in 0..n as usize {
                    let mut code = v.get(n as usize - 1 - i) as u64;
                    if bit {
                        code = code
                            .checked_sub(2)
                            .filter(|&c| c <= 1)
                            .ok_or_else(|| ice("bit array element that is not '0' or '1'"))?;
                    }
                    buf[i * cs] = code as u8;
                }
            }
            None => {
                let es = et.elem_size().ok_or_else(|| ice("array element size"))?;
                let w = width_of(es)?;
                for i in 0..n {
                    let v = self.load(add_addr(data, i * es)?, w)?;
                    put_c(&mut buf, i as usize * cs, ct, v, name, 0)?;
                }
            }
        }
        Ok(buf)
    }

    /// The fields of a record of type `ty` in VHDL layout, for a VHPI model: each field's
    /// type and byte offset, and the element count of an array field (scalars or constrained
    /// arrays of scalars).
    #[allow(clippy::type_complexity)]
    fn model_fields(&self, ty: TyId, name: &str) -> R<Vec<(String, TyId, u64, Option<u64>)>> {
        let prog = &self.rt.prog;
        let Some(Ty::Record { fields }) = prog.types.get(ty) else {
            return Err(ice("record argument of a non-record type"));
        };
        let (l, _) = Ty::layout(&prog.types, &prog.consts, ty, &[])
            .map_err(|e| ice(format!("record layout: {e}")))?;
        let mut out = Vec::with_capacity(fields.len());
        for (f, &off) in fields.iter().zip(&l.offsets) {
            let n = match (prog.types.get(f.ty), f.desc) {
                (Some(Ty::Array { .. }), Some(c)) => {
                    let Some(yavs_ir::ConstData::Bytes(d)) = prog.consts.get(c) else {
                        return Err(ice("record field descriptor"));
                    };
                    let (fl, _) = Ty::layout(&prog.types, &prog.consts, f.ty, d)
                        .map_err(|e| ice(format!("field layout: {e}")))?;
                    Some(fl.count)
                }
                (Some(Ty::Array { .. }), None) => {
                    return rt_err(format!(
                        "foreign `{name}`: record field `{}` without a static constraint",
                        f.name
                    ));
                }
                _ => None,
            };
            out.push((f.name.clone(), f.ty, off, n));
        }
        Ok(out)
    }

    fn unmarshal_array(
        &mut self,
        ty: TyId,
        buf: &[u8],
        data: u64,
        n: u64,
        name: &str,
        k: usize,
    ) -> R<()> {
        let (et, ct, logic, _) = self.array_elem(ty, name)?;
        let cs = csize(ct);
        match logic {
            Some(bit) => {
                let mut v = self.planes(data, n)?;
                for i in 0..n as usize {
                    let c = buf[i * cs];
                    let code = if bit {
                        c.checked_add(2).filter(|_| c <= 1)
                    } else {
                        Some(c)
                    };
                    let s = code
                        .filter(|&c| c <= 8)
                        .map(yavs_logic::StdULogic::from_code_pub)
                        .ok_or_else(|| {
                            Fault::Runtime(format!(
                                "foreign `{name}` wrote an invalid value to argument {}: \
                                 element code {c}",
                                k + 1
                            ))
                        })?;
                    v.set(n as usize - 1 - i, s);
                }
                self.put_planes(data, &v)
            }
            None => {
                let es = et.elem_size().ok_or_else(|| ice("array element size"))?;
                let w = width_of(es)?;
                for i in 0..n {
                    let v = get_c(buf, i as usize * cs, ct);
                    check_back(&et, v, name, k)?;
                    self.store(add_addr(data, i * es)?, w, v)?;
                }
                Ok(())
            }
        }
    }

    /// Field types, VHDL offsets, C offsets and the C struct size of a record.
    #[allow(clippy::type_complexity)]
    fn record_layout(&self, ty: TyId, name: &str) -> R<(Vec<(Ty, CType, u64)>, Vec<usize>, usize)> {
        let Ty::Record { fields } = self.ty(ty)? else {
            return Err(ice(format!(
                "foreign record argument of non-record type {ty}"
            )));
        };
        let prog = &self.rt.prog;
        let (l, _) = Ty::layout(&prog.types, &prog.consts, ty, &[])
            .map_err(|e| ice(format!("record layout: {e}")))?;
        let mut out = Vec::new();
        let (mut coffs, mut at, mut align) = (Vec::new(), 0usize, 1usize);
        for (f, &off) in fields.iter().zip(&l.offsets) {
            let ft = self.ty(f.ty)?;
            let Some(ct) = ctype_of(&ft) else {
                return rt_err(format!(
                    "foreign `{name}`: record field `{}` is not a scalar (ir.md 1.7)",
                    f.name
                ));
            };
            let cs = csize(ct);
            at = at.next_multiple_of(cs);
            coffs.push(at);
            at += cs;
            align = align.max(cs);
            out.push((ft, ct, off));
        }
        Ok((out, coffs, at.next_multiple_of(align)))
    }

    fn access_ptr(&mut self, h: u64) -> R<*mut std::ffi::c_void> {
        if h == 0 {
            return Ok(std::ptr::null_mut());
        }
        let prog = self.rt.prog.clone();
        let (heap, _) = self.heap_of(h)?;
        let i = heap.resolve(h).map_err(|_| {
            Fault::Runtime("access value passed to C designates a deallocated object".into())
        })?;
        let skip = match heap.kind(i) {
            Some(Obj::Typed(t)) => match prog.types.get(t) {
                Some(Ty::Array { .. }) => Ty::desc_size(&prog.types, t).unwrap_or(0) as usize,
                _ => 0,
            },
            Some(Obj::Bytes | Obj::Handles) => 24,
            Some(Obj::Mirror) => {
                return rt_err("a std.reflection mirror cannot be passed to a foreign subprogram");
            }
            None => 0,
        };
        let d = heap
            .data_mut(i)
            .ok_or_else(|| ice("heap object vanished"))?;
        let skip = skip.min(d.len());
        Ok(d[skip..].as_mut_ptr().cast())
    }

    /// The scalar kind of a type or of an array type's element, and that scalar `Ty`.
    fn kind_of(&self, t: TyId) -> R<(ScalarKind, Ty)> {
        let mut t = self.ty(t)?;
        if let Ty::Array { elem, .. } = t {
            t = self.ty(elem)?;
        }
        Ok((
            scalar_kind(&t).ok_or_else(|| ice("VHPI model argument type"))?,
            t,
        ))
    }

    /// A call of a VHPI model: arguments as neutral cells, the model reads and writes them,
    /// `out`/`inout` arguments and the result come back.
    #[allow(clippy::too_many_arguments)]
    fn model_call(
        &mut self,
        ctx: &mut Option<&mut Ctx<'_>>,
        name: &str,
        id: ForeignId,
        m: &VhpiModel,
        fargs: &[ForeignArg],
        regs: &[yavs_ir::Reg],
    ) -> R<u64> {
        let rt = self.rt.clone();
        let Some(h) = rt.models.get(id.0 as usize).and_then(|h| h.as_ref()) else {
            return Err(Fault::Unbound(Box::new((
                format!(
                    "VHPI model `{} {}` of foreign subprogram `{name}` is not registered",
                    m.library, m.model
                ),
                "the plugins loaded with `--vhpi`".into(),
            ))));
        };
        let mut call = ModelCall {
            subprogram: name.to_string(),
            args: Vec::new(),
            ret: None,
        };
        // Argument `k` starts at register `starts[k]`; arrays keep their buffer layout for
        // the copy back.
        let mut starts = Vec::with_capacity(fargs.len());
        let mut arrays: Vec<Option<(u64, u64)>> = Vec::with_capacity(fargs.len());
        let mut at = 0;
        for (a, p) in fargs.iter().zip(&m.params) {
            let r = *regs
                .get(at)
                .ok_or_else(|| ice("model argument registers"))?;
            starts.push(at);
            at += a.cells();
            let v = self.r(r);
            let mut arg = ModelArg {
                name: p.name.clone(),
                mode: p.mode,
                kind: ScalarKind::Record,
                array: false,
                cells: Vec::new(),
                len: Some(1),
                set: true,
                fields: Vec::new(),
                signal: None,
            };
            arrays.push(None);
            match *a {
                ForeignArg::Value(_) => arg.cells = vec![v],
                ForeignArg::ScalarRef(_, w) => arg.cells = vec![self.load(v, w)?],
                ForeignArg::Array { ty, .. } => {
                    let (buf, data, lens) = self.marshal_array(ty, v, name)?;
                    let n: u64 = lens.iter().product();
                    let (_, ct, _, _) = self.array_elem(ty, name)?;
                    let cs = csize(ct);
                    arrays[starts.len() - 1] = Some((data, n));
                    arg.cells = (0..n as usize).map(|i| get_c(&buf, i * cs, ct)).collect();
                    arg.array = true;
                }
                ForeignArg::Record { ty, .. } => {
                    for (fname, fty, off, n) in self.model_fields(ty, name)? {
                        let at = add_addr(v, off)?;
                        let (kind, et) = self.kind_of(fty)?;
                        let cells = match n {
                            None => {
                                let w = width_of(et.scalar_size().ok_or_else(|| ice("field"))?)?;
                                vec![self.load(at, w)?]
                            }
                            Some(n) => {
                                let (et, ct, logic, _) = self.array_elem(fty, name)?;
                                let buf = self.marshal_data(&et, ct, logic, at, n, name)?;
                                let cs = csize(ct);
                                (0..n as usize).map(|i| get_c(&buf, i * cs, ct)).collect()
                            }
                        };
                        arg.fields.push(ModelArg {
                            name: format!("{}.{fname}", p.name),
                            kind,
                            array: n.is_some(),
                            len: Some(cells.len()),
                            cells,
                            ..arg.clone()
                        });
                    }
                    arg.len = Some(arg.fields.len());
                    call.args.push(arg);
                    continue;
                }
                ForeignArg::Access => {
                    arg.kind = ScalarKind::Access;
                    arg.cells = vec![self.access_ptr(v)? as usize as u64];
                    call.args.push(arg);
                    continue;
                }
                ForeignArg::Signal { .. } => {
                    if self.is_multi(r) {
                        return rt_err(format!(
                            "foreign `{name}`: a signal parameter associated in parts is not \
                             supported"
                        ));
                    }
                    let (sr, ty) = self.sigref(r)?;
                    let Some(c) = ctx.as_deref_mut() else {
                        return rt_err("signal parameter of a foreign call during elaboration");
                    };
                    let value = match ty.elem {
                        Elem::Bytes(_) => Value::Bytes(c.value(sr)?.to_vec()),
                        Elem::Logic => Value::Logic(c.logic(sr)?),
                    };
                    arg.array = matches!(self.ty(p.ty)?, Ty::Array { .. });
                    arg.len = Some(sr.range.map_or(0, |r| r.len) as usize);
                    arg.signal = Some(ModelSignal {
                        value,
                        deposit: None,
                        txns: Vec::new(),
                    });
                }
            }
            arg.kind = self.kind_of(p.ty)?.0;
            if arg.signal.is_none() {
                arg.len = Some(arg.cells.len());
            }
            call.args.push(arg);
        }
        if let Some(rt_ty) = m.ret {
            let (kind, _) = self.kind_of(rt_ty)?;
            call.ret = Some(ModelArg {
                name: String::new(),
                mode: ForeignMode::Out,
                kind,
                array: m.ret_index.is_some(),
                cells: Vec::new(),
                len: match m.ret_index {
                    Some(ix) => ix.len.map(|n| n as usize),
                    None => Some(1),
                },
                set: false,
                fields: Vec::new(),
                signal: None,
            });
        }
        h.call(&mut call).map_err(|e| {
            Fault::Runtime(format!(
                "foreign call of `{name}` (VHPI {} {}): {e}",
                m.library, m.model
            ))
        })?;
        for (k, (a, p)) in fargs.iter().zip(&m.params).enumerate() {
            if p.mode == ForeignMode::In {
                continue;
            }
            let r = regs[starts[k]];
            let v = self.r(r);
            let arg = &call.args[k];
            let cells = &arg.cells;
            match *a {
                ForeignArg::ScalarRef(_, w) => {
                    let (_, et) = self.kind_of(p.ty)?;
                    let c = *cells.first().ok_or_else(|| ice("model argument cells"))?;
                    check_back(&et, c, name, k)?;
                    self.store(v, w, c)?;
                }
                ForeignArg::Array { ty, .. } => {
                    let Some((data, n)) = arrays[k] else {
                        return Err(ice("array argument"));
                    };
                    let (_, ct, _, _) = self.array_elem(ty, name)?;
                    if cells.len() as u64 != n {
                        return rt_err(format!(
                            "foreign `{name}` changed the length of argument {}",
                            k + 1
                        ));
                    }
                    let cs = csize(ct);
                    let mut buf = vec![0u8; n as usize * cs];
                    for (i, &c) in cells.iter().enumerate() {
                        put_c(&mut buf, i * cs, ct, c, name, k)?;
                    }
                    self.unmarshal_array(ty, &buf, data, n, name, k)?;
                }
                ForeignArg::Record { ty, .. } => {
                    let fields: Vec<Vec<u64>> =
                        arg.fields.iter().map(|f| f.cells.clone()).collect();
                    for ((_, fty, off, n), cells) in
                        self.model_fields(ty, name)?.into_iter().zip(fields)
                    {
                        let at = add_addr(v, off)?;
                        match n {
                            None => {
                                let (_, et) = self.kind_of(fty)?;
                                let c = *cells.first().ok_or_else(|| ice("field cells"))?;
                                check_back(&et, c, name, k)?;
                                let w = width_of(et.scalar_size().ok_or_else(|| ice("field"))?)?;
                                self.store(at, w, c)?;
                            }
                            Some(n) => {
                                let (_, ct, _, _) = self.array_elem(fty, name)?;
                                let cs = csize(ct);
                                let mut buf = vec![0u8; n as usize * cs];
                                for (i, &c) in cells.iter().enumerate() {
                                    put_c(&mut buf, i * cs, ct, c, name, k)?;
                                }
                                self.unmarshal_array(fty, &buf, at, n, name, k)?;
                            }
                        }
                    }
                }
                ForeignArg::Signal { out: true, .. } => {
                    let Some(sig) = &arg.signal else {
                        return Err(ice("signal argument without a value"));
                    };
                    let slot = u32::try_from(self.r(regs[starts[k] + 1]))
                        .map_err(|_| ice("driver slot out of range"))?;
                    let d = self.driver(slot)?;
                    let (sr, _) = self.sigref(r)?;
                    let Some(c) = ctx.as_deref_mut() else {
                        return rt_err("signal parameter of a foreign call during elaboration");
                    };
                    if let Some(dv) = &sig.deposit {
                        c.deposit(sr, dv)?;
                    }
                    for t in &sig.txns {
                        c.assign(d.id, sr.range, &[(t.delay, &t.value)], t.mode)?;
                    }
                }
                _ => return Err(ice("VHPI model argument shape")),
            }
        }
        match (call.ret, m.ret_index, m.ret) {
            (None, ..) => Ok(0),
            (Some(r), Some(ix), Some(ty)) if r.set => self.model_array(ty, ix, &r.cells, name),
            (Some(r), None, _) if r.set && r.cells.len() == 1 => Ok(r.cells[0]),
            _ => rt_err(format!(
                "foreign function `{name}` (VHPI {} {}) did not set its result",
                m.library, m.model
            )),
        }
    }

    /// A model's array result of type `ty` as a fat pair in temporary memory, indexed from
    /// `ix.left` in its direction.
    fn model_array(&mut self, ty: TyId, ix: VhpiIndex, cells: &[u64], name: &str) -> R<u64> {
        let n = cells.len() as u64;
        let (et, ct, logic, _) = self.array_elem(ty, name)?;
        let size = match logic {
            Some(_) => n.div_ceil(64) * 32,
            None => n * et.elem_size().ok_or_else(|| ice("array element size"))?,
        };
        let data = self.temp_alloc(size)?;
        let cs = csize(ct);
        let mut buf = vec![0u8; cells.len() * cs];
        for (i, &c) in cells.iter().enumerate() {
            put_c(&mut buf, i * cs, ct, c, name, 0)?;
        }
        self.unmarshal_array(ty, &buf, data, n, name, 0)?;
        let span = i64::try_from(n).ok().and_then(|n| {
            if ix.downto {
                ix.left.checked_sub(n - 1)
            } else {
                ix.left.checked_add(n - 1)
            }
        });
        let Some(right) = span else {
            return rt_err(format!(
                "foreign function `{name}`: a result of {n} elements does not fit its index"
            ));
        };
        let dope = self.temp_alloc(24)?;
        let d: Vec<u8> = [ix.left, right, i64::from(ix.downto)]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        self.put(dope, &d)?;
        let fat = self.temp_alloc(16)?;
        self.store(fat, Width::B8, data)?;
        self.store(add_addr(fat, 8)?, Width::B8, dope)?;
        Ok(fat)
    }

    pub(crate) fn foreign(
        &mut self,
        prog: &Program,
        f: &Func,
        id: ForeignId,
        args: Regs,
        ctx: &mut Option<&mut Ctx<'_>>,
    ) -> R<u64> {
        let x = prog
            .foreign
            .get(id)
            .ok_or_else(|| ice(format!("no foreign subprogram {id}")))?;
        self.tainted
            .get_or_insert_with(|| format!("it calls the foreign subprogram `{}`", x.name));
        let rt = self.rt.clone();
        if let Some(m) = &x.model {
            let regs = f.regs(args).ok_or_else(|| ice("operand list"))?.to_vec();
            return self.model_call(ctx, x.name.as_str(), id, m, &x.args, &regs);
        }
        let Some(fun) = rt.foreign.get(id.0 as usize).and_then(|f| f.as_ref()) else {
            return Err(Fault::Unbound(Box::new((
                format!(
                    "symbol `{}` of foreign subprogram `{}` is not defined",
                    x.symbol, x.name
                ),
                rt.searched.clone(),
            ))));
        };
        let regs = f.regs(args).ok_or_else(|| ice("operand list"))?.to_vec();
        let name = &x.name;
        let mut temps = Vec::with_capacity(regs.len());
        // Marshal into owned buffers first; pointers are taken once nothing moves.
        for (k, (a, &r)) in x.args.iter().zip(&regs).enumerate() {
            let v = self.r(r);
            temps.push(match *a {
                ForeignArg::Value(_) | ForeignArg::Access => Temp::None,
                ForeignArg::Signal { .. } => {
                    return Err(ice("a signal parameter of a VHPIDIRECT subprogram"));
                }
                ForeignArg::ScalarRef(ct, w) => {
                    let cur = self.load(v, w)?;
                    Temp::Scalar(Box::new([arg_bits(to_c(ct, cur, name, k)?)]), ct, w)
                }
                ForeignArg::Array { ty, .. } => {
                    let (buf, data, lens) = self.marshal_array(ty, v, name)?;
                    let n = lens.iter().product();
                    // Lengths are pushed below from the dope again.
                    Temp::Array { buf, data, ty, n }
                }
                ForeignArg::Record { ty, .. } => {
                    let (fields, coffs, size) = self.record_layout(ty, name)?;
                    let mut buf = vec![0u8; size];
                    for ((ft, ct, off), co) in fields.iter().zip(&coffs) {
                        let w = width_of(ft.scalar_size().unwrap_or(8))?;
                        let fv = self.load(add_addr(v, *off)?, w)?;
                        put_c(&mut buf, *co, *ct, fv, name, k)?;
                    }
                    Temp::Record { buf, addr: v, ty }
                }
            });
        }
        let mut cargs = Vec::with_capacity(regs.len() + 2);
        for (k, ((a, &r), t)) in x.args.iter().zip(&regs).zip(temps.iter_mut()).enumerate() {
            let v = self.r(r);
            match (*a, t) {
                (ForeignArg::Value(ct), _) => cargs.push(to_c(ct, v, name, k)?),
                (ForeignArg::Access, _) => cargs.push(Arg::Ptr(self.access_ptr(v)?)),
                (_, Temp::Scalar(b, ..)) => cargs.push(Arg::Ptr(b.as_mut_ptr().cast())),
                (ForeignArg::Array { lengths, ty, .. }, Temp::Array { buf, .. }) => {
                    cargs.push(Arg::Ptr(buf.as_mut_ptr().cast()));
                    if lengths > 0 {
                        let Ty::Array { dims, .. } = self.ty(ty)? else {
                            return Err(ice("array type"));
                        };
                        let (_, lens) = self.fat(v, dims)?;
                        for l in lens {
                            cargs.push(Arg::I64(l as i64));
                        }
                    }
                }
                (_, Temp::Record { buf, .. }) => cargs.push(Arg::Ptr(buf.as_mut_ptr().cast())),
                _ => return Err(ice("foreign argument marshalling mismatch")),
            }
        }
        let ret = fun.call(&cargs).map_err(|e| {
            Fault::Runtime(format!("foreign call of `{}` ({}): {e}", x.name, x.symbol))
        })?;
        drop(cargs);
        for (k, (a, t)) in x.args.iter().zip(temps).enumerate() {
            let v = self.r(regs[k]);
            match (*a, t) {
                (ForeignArg::ScalarRef(..), Temp::Scalar(b, ct, w)) => {
                    self.store(v, w, from_c_bits(ct, b[0]))?;
                }
                (
                    ForeignArg::Array {
                        copy_back: true, ..
                    },
                    Temp::Array { buf, data, ty, n },
                ) => {
                    self.unmarshal_array(ty, &buf, data, n, name, k)?;
                }
                (
                    ForeignArg::Record {
                        copy_back: true, ..
                    },
                    Temp::Record { buf, addr, ty },
                ) => {
                    let (fields, coffs, _) = self.record_layout(ty, name)?;
                    for ((ft, ct, off), co) in fields.iter().zip(&coffs) {
                        let fv = get_c(&buf, *co, *ct);
                        check_back(ft, fv, name, k)?;
                        let w = width_of(ft.scalar_size().unwrap_or(8))?;
                        self.store(add_addr(addr, *off)?, w, fv)?;
                    }
                }
                _ => {}
            }
        }
        Ok(match ret {
            Ret::Void => 0,
            Ret::I8(v) => v as i64 as u64,
            Ret::I16(v) => v as i64 as u64,
            Ret::I32(v) => v as i64 as u64,
            Ret::I64(v) => v as u64,
            Ret::U8(v) => v.into(),
            Ret::U16(v) => v.into(),
            Ret::U32(v) => v.into(),
            Ret::F32(v) => f64::from(v).to_bits(),
            Ret::F64(v) => v.to_bits(),
            Ret::Ptr(p) => p as usize as u64,
        })
    }
}
