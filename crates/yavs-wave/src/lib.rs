//! Waveform recording sink for yavs.
//!
//! The kernel side is a seam of two things: a [`Hierarchy`] description built with
//! [`HierarchyBuilder`] and a stream of `(time, SignalId, value bytes)` calls to
//! [`WaveWriter::change`]. Nothing here depends on the kernel.
//!
//! Value byte layouts, per declared [`SigType`]:
//! - [`SigType::Vector`] (bit/std_logic family): one byte per element, the enumeration position.
//! - [`ScalarType::Enum`]: a `u32` little endian, the enumeration position.
//! - [`ScalarType::Integer`] and [`ScalarType::Time`]: an `i64` little endian.
//! - [`ScalarType::Real`]: an `f64` little endian.
//!
//! Composites are declared by the caller as scopes ([`ScopeKind::Record`], [`ScopeKind::Array`])
//! whose leaves are ordinary signals, so every sub-element has its own [`SignalId`].

mod model;
mod select;
mod writer;

pub mod interrupt;

pub use model::{
    Bounds, Dir, EnumType, Hierarchy, HierarchyBuilder, Node, ObjClass, PortDir, ScalarType, Scope,
    ScopeKind, SigType, Signal, SignalId, Skipped,
};
pub use select::Selection;
pub use writer::{Error, WaveConfig, WaveWriter};
