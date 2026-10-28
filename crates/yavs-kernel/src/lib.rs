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
pub mod profile;
mod signal;
mod time;
mod value;

pub use builder::{
    Builder, Converter, EdgeTrigger, IdleLit, IdleRead, PortConv, PortMode, ProcFlags,
};
pub use error::{ErrorInfo, ErrorKind, SimError, StmtId};
pub use kernel::{
    Config, DeadClock, EndReason, Hook, Interrupt, LoopCause, LoopLink, Observer, Phase, ProcWait,
    Report, RunStatus, SigView, SimControl, Stats, StuckProc, StuckReport, StuckSignal, Tokens,
    WakeId, progress,
};
pub use process::{
    Cond, CopyProcess, Ctx, EDGE_LEVEL, Gate, GateProcess, Lit, LiveSink, MuxPart, MuxProcess,
    Opnd, ProcId, Process, ProgArm, ProgNode, ProgOp, ProgPart, ProgProcess, Severity, Src,
    ToggleProcess, edge_key,
};
pub use signal::{
    DelayMode, DriverId, ForceMode, GuardKind, Range, Resolution, Resolver, SigRef, SignalId,
};
pub use time::{Time, TimeError, Unit};
pub use value::{Elem, LogicVec, SignalType, Value, ValueRef};
