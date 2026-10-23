//! Layouts kept in the frame of a process (#532).
//!
//! `LayoutOf` of a record, or of an array whose element layout the descriptor decides, is a walk
//! over the descriptor: tens of instructions in the interpreter (which keeps the last layout of
//! each type, a memo it compares the descriptor with) and, in compiled code, a call out of it
//! for every access of a field. A process asks for the layout of the same few types with the
//! same descriptors at every activation, so each such `LayoutOf` of a process names bytes of the
//! process's frame (`Inst::LayoutOf::cache`, one slot per type) where a tier may keep the last
//! layout it made together with the descriptor it was made for. Whether the kept layout is the
//! right one is a comparison of descriptor bytes each time: nothing is assumed about the
//! descriptor, which is memory like any other. The frame bytes behind the function's own start
//! zero, so a slot is empty (its first word 0) until a tier keeps something in it.

use super::*;

/// The most bytes of slots one process takes.
const MAX_BYTES: u32 = 4 * 1024;
/// The longest descriptor and layout kept.
const MAX_DESC: u64 = 512;
const MAX_WORDS: u64 = 64;

/// The descriptor bytes and layout words of `ty` when its layout is worth keeping: a record, or
/// an array of several dimensions or of elements that have descriptors of their own. (The
/// layout of an array of scalars is its lengths: compiled code computes it in place.)
fn shape(prog: &Program, ty: TyId) -> Option<(u32, u32)> {
    let types = &prog.types;
    let words = match types.get(ty)? {
        Ty::Record { fields } => 2 + fields.len() as u64,
        Ty::Array {
            elem,
            elem_desc,
            dims,
        } if *dims > 1 || elem_desc.is_none() && Ty::needs_desc(types, *elem) => 4,
        _ => return None,
    };
    let dsz = Ty::desc_size(types, ty)?;
    (dsz > 0 && dsz <= MAX_DESC && dsz % 8 == 0 && words <= MAX_WORDS)
        .then_some((dsz as u32, words as u32))
}

/// Give the `LayoutOf` of process `f` that qualify a slot of its frame.
pub(super) fn run(prog: &Program, f: &mut crate::Func) {
    if f.kind != FuncKind::Process {
        return;
    }
    let base = f.frame.next_multiple_of(8);
    let mut next = base;
    let mut slots: Vec<(TyId, u32)> = Vec::new();
    for i in &mut f.code {
        let Inst::LayoutOf {
            ty,
            cache: c @ None,
            ..
        } = i
        else {
            continue;
        };
        let Some((dsz, words)) = shape(prog, *ty) else {
            continue;
        };
        // A flag word, the descriptor, the layout words.
        let at = match slots.iter().find(|s| s.0 == *ty) {
            Some(s) => s.1,
            None => {
                let size = 8 + dsz + 8 * words;
                if next - base + size > MAX_BYTES {
                    continue;
                }
                slots.push((*ty, next));
                next += size;
                next - size
            }
        };
        *c = Some(at);
    }
    f.frame = next;
}
