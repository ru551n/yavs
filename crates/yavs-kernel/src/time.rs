//! Simulation time: femtoseconds in a `u64`, capped at VHDL `TIME'HIGH`.

use std::fmt;

/// Physical time units of `std.standard.TIME`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Unit {
    Fs,
    Ps,
    Ns,
    Us,
    Ms,
    Sec,
    Min,
    Hr,
}

impl Unit {
    /// Femtoseconds in one unit.
    pub const fn fs(self) -> u64 {
        match self {
            Unit::Fs => 1,
            Unit::Ps => 1_000,
            Unit::Ns => 1_000_000,
            Unit::Us => 1_000_000_000,
            Unit::Ms => 1_000_000_000_000,
            Unit::Sec => 1_000_000_000_000_000,
            Unit::Min => 60_000_000_000_000_000,
            Unit::Hr => 3_600_000_000_000_000_000,
        }
    }

    const fn name(self) -> &'static str {
        match self {
            Unit::Fs => "fs",
            Unit::Ps => "ps",
            Unit::Ns => "ns",
            Unit::Us => "us",
            Unit::Ms => "ms",
            Unit::Sec => "sec",
            Unit::Min => "min",
            Unit::Hr => "hr",
        }
    }
}

/// Why a time computation failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimeError {
    /// The result is beyond `TIME'HIGH`.
    Overflow,
    /// A delay or time value was negative.
    Negative,
}

impl fmt::Display for TimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            TimeError::Overflow => "time overflow: result exceeds TIME'HIGH",
            TimeError::Negative => "negative time value",
        })
    }
}

impl std::error::Error for TimeError {}

/// A point in simulation time or a delay, in femtoseconds.
///
/// Always in `0..=TIME'HIGH` (`i64::MAX` fs), so it converts losslessly to a
/// VHDL `TIME` value in every tier.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Time(u64);

impl Time {
    pub const ZERO: Time = Time(0);
    /// `TIME'HIGH`.
    pub const MAX: Time = Time(i64::MAX as u64);

    /// `value` units, checked.
    pub fn new(value: u64, unit: Unit) -> Result<Time, TimeError> {
        value
            .checked_mul(unit.fs())
            .and_then(Time::from_fs)
            .ok_or(TimeError::Overflow)
    }

    pub const fn fs(fs: u64) -> Option<Time> {
        Self::from_fs(fs)
    }

    /// The femtoseconds, for compact per-element storage (`from_raw` is the inverse).
    pub(crate) const fn raw(self) -> u64 {
        self.0
    }

    pub(crate) const fn from_raw(fs: u64) -> Time {
        Time(fs)
    }

    const fn from_fs(fs: u64) -> Option<Time> {
        if fs <= Time::MAX.0 {
            Some(Time(fs))
        } else {
            None
        }
    }

    /// From a VHDL `TIME` value (signed femtoseconds).
    pub fn from_vhdl(fs: i64) -> Result<Time, TimeError> {
        u64::try_from(fs).map(Time).map_err(|_| TimeError::Negative)
    }

    /// Shorthand for tests and builders: panics on overflow.
    pub fn ns(ns: u64) -> Time {
        Time::new(ns, Unit::Ns).expect("time overflow")
    }

    pub const fn as_fs(self) -> u64 {
        self.0
    }

    pub fn checked_add(self, d: Time) -> Result<Time, TimeError> {
        self.0
            .checked_add(d.0)
            .and_then(Time::from_fs)
            .ok_or(TimeError::Overflow)
    }

    /// `self - earlier`, saturating at zero.
    pub fn since(self, earlier: Time) -> Time {
        Time(self.0.saturating_sub(earlier.0))
    }

    /// Truncate to a multiple of the resolution limit (LRM 5.2.4.2).
    pub fn quantize(self, resolution: Unit) -> Time {
        let r = resolution.fs();
        Time(self.0 / r * r)
    }
}

impl fmt::Display for Time {
    /// The largest unit that represents the value exactly, as sim1 and sim2
    /// print `now`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let units = [
            Unit::Hr,
            Unit::Min,
            Unit::Sec,
            Unit::Ms,
            Unit::Us,
            Unit::Ns,
            Unit::Ps,
        ];
        let unit = units
            .into_iter()
            .find(|u| self.0.is_multiple_of(u.fs()) && self.0 != 0)
            .unwrap_or(Unit::Fs);
        write!(f, "{} {}", self.0 / unit.fs(), unit.name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checked_arithmetic() {
        assert_eq!(Time::new(3, Unit::Ns).unwrap().as_fs(), 3_000_000);
        assert_eq!(Time::new(u64::MAX, Unit::Ns), Err(TimeError::Overflow));
        assert_eq!(
            Time::MAX.checked_add(Time::fs(1).unwrap()),
            Err(TimeError::Overflow)
        );
        assert_eq!(Time::from_vhdl(-1), Err(TimeError::Negative));
        assert!(Time::fs(u64::MAX).is_none());
    }

    #[test]
    fn quantize_and_display() {
        let t = Time::new(1500, Unit::Ps).unwrap();
        assert_eq!(t.quantize(Unit::Ns), Time::ns(1));
        assert_eq!(t.to_string(), "1500 ps");
        assert_eq!(Time::ns(2000).to_string(), "2 us");
        assert_eq!(Time::ZERO.to_string(), "0 fs");
    }
}
