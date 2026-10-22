//! yavs simulation kernel: the VHDL LRM 14.7 simulation cycle with parallel process
//! evaluation, driven only through [`SimControl`].
//!
//! The kernel knows nothing about the IR. Processes implement [`Process`]; values are bytes or
//! bit-plane logic, described by [`SignalType`]. See `DESIGN.md`.

mod builder;
pub mod callout;
mod error;
mod kernel;
mod process;
mod signal;
mod time;
mod value;

pub use builder::{Builder, Converter, EdgeTrigger, PortConv, PortMode, ProcFlags};
pub use error::{ErrorInfo, ErrorKind, SimError, StmtId};
pub use kernel::{
    Config, EndReason, Hook, Interrupt, Observer, Phase, Report, RunStatus, SigView, SimControl,
    Stats, WakeId,
};
pub use process::{CopyProcess, Ctx, LiveSink, ProcId, Process, Severity};
pub use signal::{
    DelayMode, DriverId, ForceMode, GuardKind, Range, Resolution, Resolver, SigRef, SignalId,
};
pub use time::{Time, TimeError, Unit};
pub use value::{Elem, LogicVec, SignalType, Value, ValueRef};
