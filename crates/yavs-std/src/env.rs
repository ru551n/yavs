//! std.env: STOP/FINISH as data, RESOLUTION_LIMIT, and the VHDL-2019
//! additions (date and time, GETENV, VHDL_VERSION, TOOL_*, DIR_*).

use std::fmt;
use std::io;
use std::path::Path;

use crate::arith::TimeResolution;
use crate::file::path_of;
use crate::types::Revision;

// --------------------------------------------------------- stop / finish

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HaltKind {
    /// `stop`: the tool may stay in interactive mode.
    Stop,
    /// `finish`: the simulation ends.
    Finish,
}

/// A call of `std.env.stop` or `std.env.finish`, for the kernel to act on
/// after the current process commits its side effects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Halt {
    pub kind: HaltKind,
    /// The STATUS argument, if given.
    pub status: Option<i64>,
}

impl Halt {
    /// Process exit code: the status (0 when absent), or 1 when it is
    /// outside 0..=255 since exit codes cannot carry it.
    pub fn exit_code(self) -> i32 {
        match self.status {
            None => 0,
            Some(s) => u8::try_from(s).map(i32::from).unwrap_or(1),
        }
    }
}

/// `std.env.resolution_limit`, in femtoseconds.
pub fn resolution_limit(r: TimeResolution) -> i64 {
    r.resolution_limit()
}

// ------------------------------------------------------- tool constants

pub fn vhdl_version(rev: Revision) -> &'static str {
    rev.version_string()
}
pub const TOOL_TYPE: &str = "SIMULATION";
pub const TOOL_VENDOR: &str = "yavs";
pub const TOOL_NAME: &str = "yavs";
pub const TOOL_EDITION: &str = "MIT OR Apache-2.0";
pub const TOOL_VERSION: &str = env!("CARGO_PKG_VERSION");

/// `DIR_SEPARATOR`.
pub const DIR_SEPARATOR: &str = if cfg!(windows) { "\\" } else { "/" };

/// `getenv(name)`: `None` when the variable is not set (the STRING form
/// then returns `""`, the LINE form `null`). Values are returned as raw
/// bytes on Unix and Latin-1 encoded elsewhere.
pub fn getenv(name: &[u8]) -> Option<Vec<u8>> {
    let v = std::env::var_os(crate::error::latin1(name))?;
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        Some(v.as_bytes().to_vec())
    }
    #[cfg(not(unix))]
    {
        Some(crate::types::to_latin1(&v.to_string_lossy()))
    }
}

// --------------------------------------------------------- date and time

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DayOfWeek {
    Sunday,
    Monday,
    Tuesday,
    Wednesday,
    Thursday,
    Friday,
    Saturday,
}

/// `std.env.TIME_RECORD`. `month` is 0..11 and `dayofyear` 0..365, as the
/// package declares them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeRecord {
    pub microsecond: i64,
    pub second: i64,
    pub minute: i64,
    pub hour: i64,
    pub day: i64,
    pub month: i64,
    pub year: i64,
    pub weekday: DayOfWeek,
    pub dayofyear: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TimeError {
    /// The epoch value is not finite or too large.
    BadEpoch(f64),
    /// The date's year is outside TIME_RECORD's year range 1 to 4095.
    YearOutOfRange(i64),
    /// A field of the record is outside its range.
    BadField { field: &'static str, value: i64 },
    /// The local time zone could not be determined.
    TimeZone(String),
}

impl fmt::Display for TimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TimeError::BadEpoch(x) => write!(f, "{x} seconds is not a representable time"),
            TimeError::YearOutOfRange(y) => {
                write!(f, "year {y} is outside the TIME_RECORD range 1 to 4095")
            }
            TimeError::BadField { field, value } => {
                write!(f, "TIME_RECORD field {field} = {value} is out of range")
            }
            TimeError::TimeZone(e) => write!(f, "cannot determine the local time zone: {e}"),
        }
    }
}

impl std::error::Error for TimeError {}

const US: i128 = 1_000_000;

/// Days since 1970-01-01 of a proleptic Gregorian date (month 1..12).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The date of a day number: (year, month 1..12, day).
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// Microseconds since the epoch of an epoch value in seconds: floored, so
/// the microsecond field never rounds up into the next second.
fn epoch_us(x: f64) -> Result<i128, TimeError> {
    let us = (x * 1e6).floor();
    if !us.is_finite() || us.abs() > 1e30 {
        return Err(TimeError::BadEpoch(x));
    }
    Ok(us as i128)
}

fn record_from_us(us: i128) -> Result<TimeRecord, TimeError> {
    let secs = us.div_euclid(US);
    let micro = us.rem_euclid(US) as i64;
    let days =
        i64::try_from(secs.div_euclid(86_400)).map_err(|_| TimeError::YearOutOfRange(i64::MAX))?;
    let sod = secs.rem_euclid(86_400) as i64;
    let (y, m, d) = civil_from_days(days);
    if !(1..=4095).contains(&y) {
        return Err(TimeError::YearOutOfRange(y));
    }
    const DAYS: [DayOfWeek; 7] = [
        DayOfWeek::Thursday,
        DayOfWeek::Friday,
        DayOfWeek::Saturday,
        DayOfWeek::Sunday,
        DayOfWeek::Monday,
        DayOfWeek::Tuesday,
        DayOfWeek::Wednesday,
    ];
    Ok(TimeRecord {
        microsecond: micro,
        second: sod % 60,
        minute: sod / 60 % 60,
        hour: sod / 3600,
        day: d,
        month: m - 1,
        year: y,
        weekday: DAYS[days.rem_euclid(7) as usize],
        dayofyear: days - days_from_civil(y, 1, 1),
    })
}

fn record_us(t: &TimeRecord) -> Result<i128, TimeError> {
    let check = |field, value: i64, lo, hi| {
        if (lo..=hi).contains(&value) {
            Ok(())
        } else {
            Err(TimeError::BadField { field, value })
        }
    };
    check("microsecond", t.microsecond, 0, 999_999)?;
    check("second", t.second, 0, 61)?;
    check("minute", t.minute, 0, 59)?;
    check("hour", t.hour, 0, 23)?;
    check("day", t.day, 1, 31)?;
    check("month", t.month, 0, 11)?;
    check("year", t.year, 1, 4095)?;
    let days = days_from_civil(t.year, t.month + 1, t.day) as i128;
    let secs = days * 86_400 + (t.hour * 3600 + t.minute * 60 + t.second) as i128;
    Ok(secs * US + t.microsecond as i128)
}

/// `gmtime(timer)`: the UTC record of an epoch time in seconds.
pub fn gmtime(timer: f64) -> Result<TimeRecord, TimeError> {
    record_from_us(epoch_us(timer)?)
}

/// UTC offset of the system time zone at the given epoch second.
pub fn local_offset_seconds(epoch_secs: i64) -> Result<i64, TimeError> {
    let ts =
        jiff::Timestamp::from_second(epoch_secs).map_err(|e| TimeError::TimeZone(e.to_string()))?;
    let tz = jiff::tz::TimeZone::try_system().map_err(|e| TimeError::TimeZone(e.to_string()))?;
    Ok(tz.to_offset(ts).seconds() as i64)
}

/// `localtime(timer)`: the record in the system time zone.
pub fn localtime(timer: f64) -> Result<TimeRecord, TimeError> {
    let us = epoch_us(timer)?;
    let secs = i64::try_from(us.div_euclid(US)).map_err(|_| TimeError::BadEpoch(timer))?;
    let off = local_offset_seconds(secs)? as i128;
    record_from_us(us + off * US)
}

/// `epoch(trec)`: seconds since the epoch of a record read as UTC.
pub fn epoch(t: &TimeRecord) -> Result<f64, TimeError> {
    Ok(record_us(t)? as f64 / 1e6)
}

/// `localtime(trec)`: a UTC record converted to local time.
pub fn localtime_of(t: &TimeRecord) -> Result<TimeRecord, TimeError> {
    let us = record_us(t)?;
    let off = local_offset_seconds(us.div_euclid(US) as i64)? as i128;
    record_from_us(us + off * US)
}

/// `gmtime(trec)`: a local-time record converted to UTC.
pub fn gmtime_of(t: &TimeRecord) -> Result<TimeRecord, TimeError> {
    let us = record_us(t)?;
    // The offset is that of the local wall time, so look it up at the
    // approximate UTC instant.
    let off = local_offset_seconds(us.div_euclid(US) as i64)? as i128;
    record_from_us(us - off * US)
}

fn delta_us(delta: f64) -> Result<i128, TimeError> {
    let us = (delta * 1e6).round();
    if !us.is_finite() || us.abs() > 1e30 {
        return Err(TimeError::BadEpoch(delta));
    }
    Ok(us as i128)
}

/// `trec + delta` (and `delta + trec`), delta in seconds.
pub fn add_seconds(t: &TimeRecord, delta: f64) -> Result<TimeRecord, TimeError> {
    record_from_us(record_us(t)? + delta_us(delta)?)
}

/// `trec - delta`.
pub fn sub_seconds(t: &TimeRecord, delta: f64) -> Result<TimeRecord, TimeError> {
    record_from_us(record_us(t)? - delta_us(delta)?)
}

/// `tr1 - tr2` in seconds.
pub fn diff_seconds(a: &TimeRecord, b: &TimeRecord) -> Result<f64, TimeError> {
    Ok((record_us(a)? - record_us(b)?) as f64 / 1e6)
}

/// `to_string(trec, frac_digits)`: `2023-11-14T22:13:20.123`, with 0 to 6
/// fraction digits, truncated.
pub fn time_record_to_string(t: &TimeRecord, frac_digits: u32) -> Result<Vec<u8>, TimeError> {
    if frac_digits > 6 {
        return Err(TimeError::BadField {
            field: "frac_digits",
            value: frac_digits as i64,
        });
    }
    record_us(t)?;
    let mut s = format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
        t.year,
        t.month + 1,
        t.day,
        t.hour,
        t.minute,
        t.second
    );
    if frac_digits > 0 {
        let f = format!("{:06}", t.microsecond);
        s.push('.');
        s.push_str(&f[..frac_digits as usize]);
    }
    Ok(s.into_bytes())
}

/// `time_to_seconds(t)`, `t` in femtoseconds.
pub fn time_to_seconds(fs: i64) -> f64 {
    fs as f64 / 1e15
}

/// `seconds_to_time(r)`: truncated toward zero to femtoseconds (sim1's
/// rounding); a value outside TIME is an error.
pub fn seconds_to_time(r: f64) -> Result<i64, crate::arith::ArithError> {
    let v = (r * 1e15).trunc();
    if !v.is_finite() || v >= 9_223_372_036_854_775_808.0 || v < -9_223_372_036_854_775_808.0 {
        return Err(crate::arith::ArithError::RealOverflow {
            op: crate::arith::Op::Mul,
            value: r,
        });
    }
    Ok(v as i64)
}

// ------------------------------------------------------------ directories

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirOpenStatus {
    Ok,
    NotFound,
    NoDirectory,
    AccessDenied,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirCreateStatus {
    Ok,
    ItemExists,
    AccessDenied,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirDeleteStatus {
    Ok,
    NoDirectory,
    NotEmpty,
    AccessDenied,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileDeleteStatus {
    Ok,
    NoFile,
    AccessDenied,
    Error,
}

/// `std.env.DIRECTORY`: the canonical name and the item names, sorted
/// byte-wise, without `.` and `..`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Directory {
    pub name: Vec<u8>,
    pub items: Vec<Vec<u8>>,
}

fn bytes_of(p: &Path) -> Vec<u8> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        p.as_os_str().as_bytes().to_vec()
    }
    #[cfg(not(unix))]
    {
        crate::types::to_latin1(&p.to_string_lossy())
    }
}

fn open_status(e: &io::Error) -> DirOpenStatus {
    match e.kind() {
        io::ErrorKind::NotFound => DirOpenStatus::NotFound,
        io::ErrorKind::PermissionDenied => DirOpenStatus::AccessDenied,
        io::ErrorKind::NotADirectory => DirOpenStatus::NoDirectory,
        _ => DirOpenStatus::Error,
    }
}

/// `dir_open(dir, path, status)`.
pub fn dir_open(path: &[u8]) -> Result<Directory, DirOpenStatus> {
    let p = path_of(path);
    let canon = p.canonicalize().map_err(|e| open_status(&e))?;
    if !canon.is_dir() {
        return Err(DirOpenStatus::NoDirectory);
    }
    let mut items = Vec::new();
    for e in std::fs::read_dir(&canon).map_err(|e| open_status(&e))? {
        let e = e.map_err(|e| open_status(&e))?;
        items.push(bytes_of(Path::new(&e.file_name())));
    }
    items.sort();
    Ok(Directory {
        name: bytes_of(&canon),
        items,
    })
}

pub fn dir_itemexists(path: &[u8]) -> bool {
    path_of(path).symlink_metadata().is_ok()
}

pub fn dir_itemisdir(path: &[u8]) -> bool {
    path_of(path).is_dir()
}

pub fn dir_itemisfile(path: &[u8]) -> bool {
    path_of(path).is_file()
}

/// `dir_workingdir`: the current directory.
pub fn dir_workingdir() -> io::Result<Vec<u8>> {
    Ok(bytes_of(&std::env::current_dir()?))
}

/// `dir_workingdir(path, status)`: change the current directory. This is
/// process-wide state; the kernel must serialise it.
pub fn dir_set_workingdir(path: &[u8]) -> DirOpenStatus {
    let p = path_of(path);
    if !p.exists() {
        return DirOpenStatus::NotFound;
    }
    if !p.is_dir() {
        return DirOpenStatus::NoDirectory;
    }
    match std::env::set_current_dir(&p) {
        Ok(()) => DirOpenStatus::Ok,
        Err(e) => open_status(&e),
    }
}

/// `dir_createdir(path, parents, status)`.
pub fn dir_createdir(path: &[u8], parents: bool) -> DirCreateStatus {
    let p = path_of(path);
    if p.exists() {
        return DirCreateStatus::ItemExists;
    }
    let r = if parents {
        std::fs::create_dir_all(&p)
    } else {
        std::fs::create_dir(&p)
    };
    match r {
        Ok(()) => DirCreateStatus::Ok,
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => DirCreateStatus::ItemExists,
        Err(e) if e.kind() == io::ErrorKind::PermissionDenied => DirCreateStatus::AccessDenied,
        Err(_) => DirCreateStatus::Error,
    }
}

/// `dir_deletedir(path, recursive, status)`.
pub fn dir_deletedir(path: &[u8], recursive: bool) -> DirDeleteStatus {
    let p = path_of(path);
    if !p.is_dir() {
        return DirDeleteStatus::NoDirectory;
    }
    let r = if recursive {
        std::fs::remove_dir_all(&p)
    } else {
        std::fs::remove_dir(&p)
    };
    match r {
        Ok(()) => DirDeleteStatus::Ok,
        Err(e) if e.kind() == io::ErrorKind::DirectoryNotEmpty => DirDeleteStatus::NotEmpty,
        Err(e) if e.kind() == io::ErrorKind::PermissionDenied => DirDeleteStatus::AccessDenied,
        Err(_) => DirDeleteStatus::Error,
    }
}

/// `dir_deletefile(path, status)`.
pub fn dir_deletefile(path: &[u8]) -> FileDeleteStatus {
    let p = path_of(path);
    if !p.is_file() {
        return FileDeleteStatus::NoFile;
    }
    match std::fs::remove_file(&p) {
        Ok(()) => FileDeleteStatus::Ok,
        Err(e) if e.kind() == io::ErrorKind::PermissionDenied => FileDeleteStatus::AccessDenied,
        Err(_) => FileDeleteStatus::Error,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calendar() {
        let t = gmtime(951_782_400.0).unwrap();
        assert_eq!((t.year, t.month, t.day, t.dayofyear), (2000, 1, 29, 59));
        assert_eq!(t.weekday, DayOfWeek::Tuesday);
        assert_eq!(epoch(&t).unwrap(), 951_782_400.0);
        assert!(gmtime(253_402_300_800.0 * 2.0).is_err());
        let t = gmtime(-86_400.5).unwrap();
        assert_eq!(
            time_record_to_string(&t, 6).unwrap(),
            b"1969-12-30T23:59:59.500000"
        );
        assert!(time_record_to_string(&t, 7).is_err());
        let t = sub_seconds(&gmtime(0.0).unwrap(), 90_061.5).unwrap();
        assert_eq!(
            time_record_to_string(&t, 6).unwrap(),
            b"1969-12-30T22:58:58.500000"
        );
        assert_eq!(
            Halt {
                kind: HaltKind::Finish,
                status: Some(300)
            }
            .exit_code(),
            1
        );
        assert!(seconds_to_time(9_223.372_036_854_776).is_err());
    }

    #[test]
    fn directories() {
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path().to_path_buf();
        let n = bytes_of(&d);
        let sub = bytes_of(&d.join("a/b"));
        assert_eq!(dir_createdir(&sub, false), DirCreateStatus::Error);
        assert_eq!(dir_createdir(&sub, true), DirCreateStatus::Ok);
        assert_eq!(dir_createdir(&sub, true), DirCreateStatus::ItemExists);
        std::fs::write(d.join("f.txt"), b"x").unwrap();
        let dir = dir_open(&n).unwrap();
        assert_eq!(dir.items, vec![b"a".to_vec(), b"f.txt".to_vec()]);
        assert!(dir_itemisfile(&bytes_of(&d.join("f.txt"))));
        assert_eq!(
            dir_open(&bytes_of(&d.join("f.txt"))).unwrap_err(),
            DirOpenStatus::NoDirectory
        );
        assert_eq!(
            dir_open(&bytes_of(&d.join("nope"))).unwrap_err(),
            DirOpenStatus::NotFound
        );
        assert_eq!(
            dir_deletefile(&bytes_of(&d.join("f.txt"))),
            FileDeleteStatus::Ok
        );
        assert_eq!(
            dir_deletefile(&bytes_of(&d.join("f.txt"))),
            FileDeleteStatus::NoFile
        );
        assert_eq!(dir_deletedir(&n, false), DirDeleteStatus::NotEmpty);
        assert_eq!(dir_deletedir(&n, true), DirDeleteStatus::Ok);
        assert!(!dir_itemexists(&n));
        assert_eq!(getenv(b"YAVS_STD_SURELY_UNSET_VARIABLE"), None);
    }
}
