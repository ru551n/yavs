//! Textual dump for debugging and tests.

use std::fmt;

use crate::{Func, FuncId, Program};

impl Program {
    /// The dump of one function.
    pub fn dump_func(&self, id: FuncId) -> String {
        let mut s = String::new();
        if let Some(f) = self.func(id) {
            let _ = write_func(&mut s, id, f);
        }
        s
    }
}

fn write_func(out: &mut impl fmt::Write, id: FuncId, f: &Func) -> fmt::Result {
    writeln!(
        out,
        "{id} {:?} `{}` frame {} regs {} flags {:#x}",
        f.kind,
        f.name,
        f.frame,
        f.reg_ty.len(),
        f.flags.0
    )?;
    for p in &f.params {
        writeln!(
            out,
            "  param {} @{} {:?} {:?} {:?}",
            p.name, p.slot, p.width, p.class, p.mode
        )?;
    }
    for (b, blk) in f.blocks.iter() {
        writeln!(out, "{b}:")?;
        for pc in blk.start..blk.end {
            let i = &f.code[pc as usize];
            write!(out, "  {pc:4} s{:<4} {i:?}", f.loc[pc as usize].0)?;
            if let Some(rs) = i.list().and_then(|l| f.regs(l)) {
                write!(out, " {rs:?}")?;
            }
            writeln!(out)?;
        }
    }
    Ok(())
}

impl fmt::Display for Program {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (id, t) in self.types.iter() {
            writeln!(f, "{id} = {t:?}")?;
        }
        for (id, c) in self.consts.iter() {
            writeln!(f, "{id} = {c:?}")?;
        }
        for (id, n) in self.natives.iter() {
            writeln!(f, "{id} = native {} {:?}", n.symbol, n.ty)?;
        }
        for (id, x) in self.foreign.iter() {
            writeln!(
                f,
                "{id} = foreign `{}` {} {:?}",
                x.name,
                x.symbol,
                x.signature()
            )?;
        }
        for (id, _) in self.funcs.iter().filter(|(_, f)| !f.spare) {
            if let Some(func) = self.func(id) {
                write_func(f, id, func)?;
            }
        }
        Ok(())
    }
}
