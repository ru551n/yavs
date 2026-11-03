//! `YAVS_COUNTERS=kinds` (or `=pc`): how often compiled code calls into the host, by entry point
//! and instruction kind (with `pc`, by function and pc); `=exits`: only the census of what leaves
//! compiled code (`exit`, `ret`, `noentry`, `deopt`, `deopt_frames`, `unwind_*`). `yavs run` prints them with the other
//! work counters as `counters: cb_<entry>.<Kind>[.<function>.pc<N>]=N` (`crates/yavs/src/counters.rs`),
//! next to the totals per callback kind of `yavs-llvm` (`cb_<entry>`). A tuning aid and the check
//! that an instruction takes its fast path: a counter that stays out is a fast path that always
//! held. Off, a call costs one load.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

static MODE: OnceLock<u8> = OnceLock::new();
static COUNTS: Mutex<Option<HashMap<String, u64>>> = Mutex::new(None);

fn mode() -> u8 {
    *MODE.get_or_init(|| match std::env::var("YAVS_COUNTERS").as_deref() {
        Ok("pc") => 2,
        Ok("kinds") => 1,
        // Only what leaves compiled code: cheap enough for a whole run.
        Ok("exits") => 3,
        _ => 0,
    })
}

/// Count one call of host entry `entry` for the instruction at `pc` of `func`.
#[inline(always)]
pub(crate) fn note(entry: &str, func: &str, pc: u32, inst: Option<&yavs_ir::Inst>) {
    if matches!(mode(), 1 | 2) {
        note_slow(entry, func, pc, inst);
    }
}

/// Compiled code of `f` returned `pc` to the interpreter (`same`: where it was entered, no entry
/// there). A `Ret` is its normal end.
#[inline(always)]
pub(crate) fn note_exit(f: &yavs_ir::Func, pc: u32, same: bool) {
    if mode() != 0 {
        let inst = f.code.get(pc as usize);
        let e = match inst {
            Some(yavs_ir::Inst::Ret { .. }) => "ret",
            _ if same => "noentry",
            _ => "exit",
        };
        note_slow(e, &f.name, pc, inst);
    }
}

/// Native frames are materialised: the top one names why, every one counts.
#[inline(always)]
pub(crate) fn note_deopt(prog: &yavs_ir::Program, frames: &[crate::NativeFrame]) {
    if mode() != 0
        && let Some(top) = frames.last()
        && let Some(f) = prog.func(top.func)
    {
        note_slow("deopt", &f.name, top.pc, f.code.get(top.pc as usize));
        for _ in frames {
            note_slow("deopt_frames", "", 0, None);
        }
    }
}

/// An exit of compiled code the interpreter does not see as such (`rt::step_in`).
pub fn note_unwind(why: &str, func: &str, pc: u32) {
    if mode() != 0 {
        note_slow(why, func, pc, None);
    }
}

#[cold]
#[inline(never)]
fn note_slow(entry: &str, func: &str, pc: u32, inst: Option<&yavs_ir::Inst>) {
    let mut key = entry.to_string();
    if let Some(i) = inst {
        let d = format!("{i:?}");
        key.push('.');
        key.push_str(d.split([' ', '{', '(']).next().unwrap_or(""));
    }
    if mode() == 2 {
        key.push_str(&format!(".{func}.pc{pc}"));
    }
    *COUNTS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get_or_insert_with(HashMap::new)
        .entry(key)
        .or_insert(0) += 1;
}

/// The counts, most frequent first (`"entry[.Kind][.func.pcN]"`, calls); empty when off.
pub fn report() -> Vec<(String, u64)> {
    let mut v: Vec<(String, u64)> = COUNTS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take()
        .map(|m| m.into_iter().collect())
        .unwrap_or_default();
    v.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    v
}
