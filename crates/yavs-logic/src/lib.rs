//! IEEE 1164 logic for yavs: the scalar `StdULogic` and packed vectors.
//!
//! `LogicVec` is the one runtime vector type. Two layouts implement it (`Nibbles`, `Planes`);
//! see LAYOUT.md for the benchmark and the choice of default.

#![forbid(unsafe_code)]

mod bitvec;
mod nibbles;
mod planes;
mod repr;
mod scalar;
mod vec;
pub mod wide;

pub use bitvec::BitVec;
pub use nibbles::Nibbles;
pub use planes::{Block, Planes};
pub use repr::{BinOp, Repr, UnOp};
pub use scalar::{ALL, StdULogic};
pub use vec::Logic;

#[cfg(feature = "nibbles")]
pub type DefaultRepr = Nibbles;
#[cfg(not(feature = "nibbles"))]
pub type DefaultRepr = Planes;

/// The packed `std_ulogic_vector` used by the simulator.
pub type LogicVec = Logic<DefaultRepr>;

/// Truth tables as `[row][col]` codes, for tests against the IEEE package body.
#[doc(hidden)]
pub mod tables {
    pub use crate::scalar::{AND, MATCH, NOT, OR, RESOLVE, TO_UX01, TO_X01, TO_X01Z, XOR};
}
