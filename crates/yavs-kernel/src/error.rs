use std::fmt;

use crate::process::ProcId;
use crate::time::Time;

/// An IR statement id, resolved to a source span by the IR's tables.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct StmtId(pub u32);

/// A kernel or runtime error. It stops the simulation at once and carries
/// what yavs-diag needs for a runtime diagnostic (see [`ErrorInfo`]). Boxed,
/// so `Result<_, SimError>` stays one word on the hot paths.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SimError(Box<ErrorInfo>);

impl std::ops::Deref for SimError {
    type Target = ErrorInfo;
    fn deref(&self) -> &ErrorInfo {
        &self.0
    }
}

impl std::ops::DerefMut for SimError {
    fn deref_mut(&mut self) -> &mut ErrorInfo {
        &mut self.0
    }
}

/// The kind, time and delta, the running process (id and instance path) and
/// the VHDL call stack of an error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ErrorInfo {
    pub kind: ErrorKind,
    pub time: Time,
    pub delta: u32,
    /// The process that was running, with its hierarchical instance path.
    pub process: Option<(ProcId, String)>,
    /// Statement ids of the VHDL call stack, innermost first. Filled by the
    /// interpreter or the LLVM tier; empty for kernel-level errors.
    pub trace: Vec<StmtId>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    /// `base + delay` is beyond `TIME'HIGH`.
    TimeOverflow { base: Time, delay: Time },
    /// More than `limit` delta cycles at one time; `processes` (instance
    /// paths) ran in the last delta, `cause` is the loop behind it if one was found.
    DeltaLimit {
        limit: u32,
        processes: Vec<String>,
        cause: crate::LoopCause,
    },
    /// A postponed process scheduled a delta cycle (LRM 14.7.5.3).
    PostponedDelta,
    /// A malformed signal assignment.
    InvalidAssign(String),
    /// A wait or observer wake-up deadline earlier than the current time.
    DeadlineInPast { deadline: Time },
    /// An id or element range that does not exist in this model.
    BadRef(String),
    /// A resolution function failed.
    Resolution { signal: String, message: String },
    /// A port conversion function failed or returned a value of the wrong size.
    Conversion { formal: String, message: String },
    /// Raised by process code: bound check, overflow, null access, ...
    Runtime(String),
    /// A call of a foreign subprogram whose C symbol no `--load` library or the executable
    /// defines (`searched` lists the places looked in).
    Foreign { message: String, searched: String },
    /// The model, the configuration or a `SimControl` request is invalid.
    Usage(String),
    /// A broken kernel invariant: a bug in yavs.
    Internal(String),
}

impl SimError {
    /// An error without process context, at time zero (builder errors).
    pub fn new(kind: ErrorKind) -> SimError {
        SimError(Box::new(ErrorInfo {
            kind,
            time: Time::ZERO,
            delta: 0,
            process: None,
            trace: Vec::new(),
        }))
    }

    pub(crate) fn at(kind: ErrorKind, time: Time, delta: u32) -> SimError {
        let mut e = SimError::new(kind);
        e.time = time;
        e.delta = delta;
        e
    }

    /// Exit status for the CLI: 101 for internal errors (as rustc ICEs), 1
    /// for every other error.
    pub fn exit_status(&self) -> i32 {
        match self.kind {
            ErrorKind::Internal(_) => 101,
            _ => 1,
        }
    }
}

impl ErrorKind {
    /// The diagnostic code (yavs-diag registry: `E3000`-`E3099` belong to the kernel, an
    /// internal error is `E9999` as in every stage).
    pub fn code(&self) -> &'static str {
        match self {
            ErrorKind::TimeOverflow { .. } => "E3001",
            ErrorKind::DeltaLimit { .. } => "E3002",
            ErrorKind::PostponedDelta => "E3003",
            ErrorKind::InvalidAssign(_) => "E3004",
            ErrorKind::DeadlineInPast { .. } => "E3005",
            ErrorKind::BadRef(_) => "E3006",
            ErrorKind::Resolution { .. } => "E3007",
            ErrorKind::Conversion { .. } => "E3008",
            ErrorKind::Runtime(_) => "E3009",
            ErrorKind::Foreign { .. } => "E0107",
            ErrorKind::Usage(_) => "E3010",
            ErrorKind::Internal(_) => "E9999",
        }
    }
}

impl fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ErrorKind::TimeOverflow { base, delay } => {
                write!(f, "time overflow: {base} + {delay} exceeds TIME'HIGH")
            }
            ErrorKind::DeltaLimit {
                limit, processes, ..
            } => {
                write!(
                    f,
                    "delta cycle limit {limit} exceeded; active processes: {}",
                    processes.join(", ")
                )
            }
            ErrorKind::PostponedDelta => f.write_str("postponed process caused a delta cycle"),
            ErrorKind::InvalidAssign(reason) => write!(f, "invalid signal assignment: {reason}"),
            ErrorKind::DeadlineInPast { deadline } => {
                write!(f, "wait deadline {deadline} is in the past")
            }
            ErrorKind::BadRef(what) => write!(f, "invalid reference: {what}"),
            ErrorKind::Resolution { signal, message } => {
                write!(f, "resolution of {signal} failed: {message}")
            }
            ErrorKind::Conversion { formal, message } => {
                write!(f, "conversion for port {formal} failed: {message}")
            }
            ErrorKind::Runtime(msg) | ErrorKind::Usage(msg) => f.write_str(msg),
            ErrorKind::Foreign { message, .. } => f.write_str(message),
            ErrorKind::Internal(msg) => write!(
                f,
                "internal error: {msg}; this is a bug in yavs, please report it"
            ),
        }
    }
}

impl fmt::Display for SimError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} +{}", self.time, self.delta)?;
        if let Some((_, path)) = &self.process {
            write!(f, " in {path}")?;
        }
        write!(f, ": {}", self.kind)
    }
}

impl std::error::Error for SimError {}
