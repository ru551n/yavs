//! `YAVS_TIER_CENSUS=1` (or `=pc`): how often compiled code calls into the host, by entry point
//! and instruction kind (with `pc`, by function and pc), printed by `yavs run` when the
//! simulation ends. A tuning aid and the check that an instruction takes its fast path: a
//! counter that stays at 0 is a fast path that always held. Off, a call costs one load.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

static MODE: OnceLock<u8> = OnceLock::new();
static COUNTS: Mutex<Option<HashMap<String, u64>>> = Mutex::new(None);

fn mode() -> u8 {
    *MODE.get_or_init(|| match std::env::var("YAVS_TIER_CENSUS").as_deref() {
        Ok("pc") => 2,
        Ok("") | Err(_) | Ok("0") => 0,
        Ok(_) => 1,
    })
}

/// Count one call of host entry `entry` for the instruction at `pc` of `func`.
#[inline(always)]
pub(crate) fn note(entry: &str, func: &str, pc: u32, inst: Option<&yavs_ir::Inst>) {
    if mode() != 0 {
        note_slow(entry, func, pc, inst);
    }
}

#[cold]
#[inline(never)]
fn note_slow(entry: &str, func: &str, pc: u32, inst: Option<&yavs_ir::Inst>) {
    let kind = inst.map_or_else(
        || "-".to_string(),
        |i| {
            let d = format!("{i:?}");
            d.split([' ', '{', '(']).next().unwrap_or("").to_string()
        },
    );
    let key = if mode() == 2 {
        format!("{entry} {kind} {func} pc{pc}")
    } else {
        format!("{entry} {kind}")
    };
    *COUNTS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get_or_insert_with(HashMap::new)
        .entry(key)
        .or_insert(0) += 1;
}

/// The counts, most frequent first (`"entry kind [func pcN]"`, calls); empty when off.
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
