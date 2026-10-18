//! rsim simulation kernel: the VHDL LRM 14.7 simulation cycle with parallel
//! process evaluation, driven only through [`SimControl`].
//!
//! The kernel knows nothing about the IR. Processes implement [`Process`];
//! values are opaque bytes described by [`SignalType`]. See `DESIGN.md`.

mod builder;
mod error;
mod kernel;
mod process;
mod signal;
mod time;

pub use builder::{Builder, ProcFlags};
pub use error::{ErrorKind, SimError, StmtId};
pub use kernel::{
    Config, EndReason, Hook, Interrupt, Observer, Phase, Report, RunStatus, SimControl, Stats,
};
pub use process::{Ctx, ProcId, Process, Severity};
pub use signal::{DelayMode, DriverId, ForceMode, Range, Resolver, SigRef, SignalId, SignalType};
pub use time::{Time, TimeError, Unit};
