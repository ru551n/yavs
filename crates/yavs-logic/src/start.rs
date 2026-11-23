//! Start values the user chooses (`--init-unset`, `--init-x`): `'0'`, `'1'` or bits drawn from a
//! seed, for the elements VHDL leaves at `'U'` and for metavalue literals. A drawn bit is a pure
//! function of the seed, the object or literal (`salt`) and the element, so every tier, thread
//! count and run of the same design gets the same bits.

use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

/// What a start value becomes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fill {
    Zero,
    One,
    /// A bit per element, drawn from this seed.
    Random(u64),
}

impl Fill {
    /// The value of element `i` (0 is the rightmost) of the object or literal `salt`: true for
    /// `'1'`.
    pub fn bit(self, salt: u64, i: u64) -> bool {
        match self {
            Fill::Zero => false,
            Fill::One => true,
            Fill::Random(seed) => mix(mix(seed ^ salt).wrapping_add(i)) & 1 == 1,
        }
    }

    /// The `std_ulogic` code (`'0'` 2, `'1'` 3) of [`Fill::bit`].
    pub fn code(self, salt: u64, i: u64) -> u8 {
        2 + u8::from(self.bit(salt, i))
    }

    /// The option's text: `0`, `1`, `random:SEED`.
    pub fn text(self) -> String {
        match self {
            Fill::Zero => "0".into(),
            Fill::One => "1".into(),
            Fill::Random(s) => format!("random:{s}"),
        }
    }
}

/// A 64-bit mixing function (splitmix64's finaliser).
fn mix(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// The salt of an object or literal from what names it (a path, a source position): FNV-1a.
pub fn salt(parts: &[&[u8]]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for p in parts {
        for &b in *p {
            h = (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3);
        }
        h = (h ^ 0xff).wrapping_mul(0x100_0000_01b3);
    }
    h
}

/// The salt of the metavalue literal at a source site (`yavs_diag::SourceFile::site`): file name,
/// line and byte column, so every mode, tier, checkout and system draws the same bits for it.
pub fn lit_salt(file: &str, line: u32, col: u32) -> u64 {
    salt(&[
        b"lit",
        file.as_bytes(),
        &line.to_le_bytes(),
        &col.to_le_bytes(),
    ])
}

/// The salt of a variable declaration: its name and source site, as [`lit_salt`]'s
/// ([`inst_salt`] adds the process instance). Cycle mode and the normal mode give a vector or
/// scalar variable the same bits.
pub fn var_salt(file: &str, name: &str, line: u32, col: u32) -> u64 {
    salt(&[
        b"var",
        file.as_bytes(),
        name.to_lowercase().as_bytes(),
        &line.to_le_bytes(),
        &col.to_le_bytes(),
    ])
}

/// The salt of a variable in one process instance: [`var_salt`] and the process's name as its
/// reports print it,
/// so that the instances of one process get bits of their own.
pub fn inst_salt(var: u64, process: &str) -> u64 {
    salt(&[&var.to_le_bytes(), process.as_bytes()])
}

// The run's choices: 0 none, else 1 + kind (0, 1, random) and the seed apart.
static UNSET: AtomicU64 = AtomicU64::new(0);
static UNSET_SEED: AtomicU64 = AtomicU64::new(0);
static X: AtomicU64 = AtomicU64::new(0);
static X_SEED: AtomicU64 = AtomicU64::new(0);

fn store(kind: &AtomicU64, seed: &AtomicU64, f: Option<Fill>) {
    let (k, s) = match f {
        None => (0, 0),
        Some(Fill::Zero) => (1, 0),
        Some(Fill::One) => (2, 0),
        Some(Fill::Random(s)) => (3, s),
    };
    seed.store(s, Relaxed);
    kind.store(k, Relaxed);
}

fn load(kind: &AtomicU64, seed: &AtomicU64) -> Option<Fill> {
    match kind.load(Relaxed) {
        1 => Some(Fill::Zero),
        2 => Some(Fill::One),
        3 => Some(Fill::Random(seed.load(Relaxed))),
        _ => None,
    }
}

/// Set the run's `--init-unset` and `--init-x` (once, before elaboration).
pub fn set(unset: Option<Fill>, x: Option<Fill>) {
    store(&UNSET, &UNSET_SEED, unset);
    store(&X, &X_SEED, x);
}

/// `--init-unset`: the start value of `std_ulogic` elements without an initial value.
pub fn unset() -> Option<Fill> {
    load(&UNSET, &UNSET_SEED)
}

/// `--init-x`: the value of the metavalue literals it covers.
pub fn x() -> Option<Fill> {
    load(&X, &X_SEED)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bits_are_a_function_of_seed_salt_and_element() {
        let r = Fill::Random(7);
        let a: Vec<bool> = (0..128).map(|i| r.bit(5, i)).collect();
        assert_eq!(a, (0..128).map(|i| r.bit(5, i)).collect::<Vec<_>>());
        // Both values occur, and another salt or seed gives other bits.
        assert!(a.iter().any(|&b| b) && a.iter().any(|&b| !b));
        assert_ne!(a, (0..128).map(|i| r.bit(6, i)).collect::<Vec<_>>());
        assert_ne!(
            a,
            (0..128)
                .map(|i| Fill::Random(8).bit(5, i))
                .collect::<Vec<_>>()
        );
        assert_eq!((Fill::Zero.code(1, 2), Fill::One.code(1, 2)), (2, 3));
        assert_ne!(salt(&[b"ab", b"c"]), salt(&[b"a", b"bc"]));
    }
}
