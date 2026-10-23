//! IR-independent descriptions of the scalar types the conversions work on,
//! and the types of `std.standard`.
//!
//! All VHDL text is Latin-1 bytes (`[u8]`): CHARACTER has 256 values and a
//! STRING element is one byte. Identifiers keep their original spelling
//! (plan rule 8); comparisons fold case only for basic identifiers.

/// One enumeration literal as written in its type declaration.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum EnumLit {
    /// An identifier, basic (`Foo`) or extended (`\Bar\`, with any inner
    /// backslash doubled as in the source).
    Ident(Vec<u8>),
    /// A character literal; the byte is the character between the quotes.
    Char(u8),
}

impl EnumLit {
    pub fn ident(s: &str) -> EnumLit {
        EnumLit::Ident(to_latin1(s))
    }
}

/// An enumeration type: its literals in position order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnumType {
    pub lits: Vec<EnumLit>,
}

impl EnumType {
    pub fn from_idents(names: &[&str]) -> EnumType {
        EnumType {
            lits: names.iter().map(|n| EnumLit::ident(n)).collect(),
        }
    }
}

/// A unit of a physical type and its value in primary units.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unit {
    pub name: Vec<u8>,
    pub value: i64,
}

/// A physical type. `units[0]` is the primary unit (value 1); the others
/// follow in declaration order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhysicalType {
    pub units: Vec<Unit>,
}

impl PhysicalType {
    pub fn new(units: &[(&str, i64)]) -> PhysicalType {
        PhysicalType {
            units: units
                .iter()
                .map(|&(n, v)| Unit {
                    name: to_latin1(n),
                    value: v,
                })
                .collect(),
        }
    }

    pub fn primary(&self) -> &Unit {
        &self.units[0]
    }

    /// The unit whose value is exactly `value` (the first if several).
    pub fn unit_with_value(&self, value: i64) -> Option<&Unit> {
        self.units.iter().find(|u| u.value == value)
    }
}

/// Inclusive bounds of an integer or physical subtype.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bounds {
    pub low: i64,
    pub high: i64,
}

impl Bounds {
    pub const I64: Bounds = Bounds {
        low: i64::MIN,
        high: i64::MAX,
    };
    /// INTEGER in VHDL-2008 (sim1 and sim2 use 32 bits).
    pub const I32: Bounds = Bounds {
        low: i32::MIN as i64,
        high: i32::MAX as i64,
    };

    pub fn contains(self, v: i64) -> bool {
        self.low <= v && v <= self.high
    }
}

/// Inclusive bounds of a floating-point subtype.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RealBounds {
    pub low: f64,
    pub high: f64,
}

impl RealBounds {
    pub const REAL: RealBounds = RealBounds {
        low: -f64::MAX,
        high: f64::MAX,
    };
}

/// Which revision of the standard the design is analysed as.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Revision {
    /// 1076-1993: the 2008 numbers (a 32-bit INTEGER is within the 1993 minimum).
    Vhdl1993,
    Vhdl2008,
    Vhdl2019,
}

impl Revision {
    /// The range of INTEGER: 32 bits in 2008, 64 bits in 2019 (LRM 2019
    /// 5.2.3.2 requires at least 64 bits).
    pub fn integer_bounds(self) -> Bounds {
        match self {
            Revision::Vhdl1993 | Revision::Vhdl2008 => Bounds::I32,
            Revision::Vhdl2019 => Bounds::I64,
        }
    }

    /// `std.env.vhdl_version`.
    pub fn version_string(self) -> &'static str {
        match self {
            Revision::Vhdl1993 => "1993",
            Revision::Vhdl2008 => "2008",
            Revision::Vhdl2019 => "2019",
        }
    }
}

/// Encode a Rust string as Latin-1. Characters above U+00FF cannot occur in
/// VHDL text; they become `?` so the caller sees them in the output.
pub fn to_latin1(s: &str) -> Vec<u8> {
    s.chars()
        .map(|c| u8::try_from(u32::from(c)).unwrap_or(b'?'))
        .collect()
}

/// Lower-case one Latin-1 byte (ASCII and the Latin-1 letters).
pub fn lower(c: u8) -> u8 {
    match c {
        b'A'..=b'Z' => c + 32,
        0xC0..=0xDE if c != 0xD7 => c + 32,
        _ => c,
    }
}

/// Upper-case one Latin-1 byte. 0xDF and 0xFF have no Latin-1 capital.
pub fn upper(c: u8) -> u8 {
    match c {
        b'a'..=b'z' => c - 32,
        0xE0..=0xFE if c != 0xF7 => c - 32,
        _ => c,
    }
}

/// Is this byte a Latin-1 letter (LRM 15.2 letters)?
pub fn is_letter(c: u8) -> bool {
    c.is_ascii_alphabetic() || (c >= 0xC0 && c != 0xD7 && c != 0xF7)
}

/// Names of the CHARACTER literals that are identifiers (control
/// characters), indexed by position; `None` for graphic characters.
pub fn character_name(c: u8) -> Option<String> {
    const LOW: [&str; 32] = [
        "NUL", "SOH", "STX", "ETX", "EOT", "ENQ", "ACK", "BEL", "BS", "HT", "LF", "VT", "FF", "CR",
        "SO", "SI", "DLE", "DC1", "DC2", "DC3", "DC4", "NAK", "SYN", "ETB", "CAN", "EM", "SUB",
        "ESC", "FSP", "GSP", "RSP", "USP",
    ];
    match c {
        0..=31 => Some(LOW[c as usize].to_string()),
        127 => Some("DEL".to_string()),
        128..=159 => Some(format!("C{c}")),
        _ => None,
    }
}

/// The literals of std.standard.CHARACTER.
pub fn character_type() -> EnumType {
    EnumType {
        lits: (0..=255u8)
            .map(|c| match character_name(c) {
                Some(n) => EnumLit::Ident(n.into_bytes()),
                None => EnumLit::Char(c),
            })
            .collect(),
    }
}

pub fn boolean_type() -> EnumType {
    EnumType::from_idents(&["FALSE", "TRUE"])
}

pub fn bit_type() -> EnumType {
    EnumType {
        lits: vec![EnumLit::Char(b'0'), EnumLit::Char(b'1')],
    }
}

pub fn severity_level_type() -> EnumType {
    EnumType::from_idents(&["NOTE", "WARNING", "ERROR", "FAILURE"])
}

pub fn file_open_kind_type(rev: Revision) -> EnumType {
    match rev {
        Revision::Vhdl1993 | Revision::Vhdl2008 => {
            EnumType::from_idents(&["READ_MODE", "WRITE_MODE", "APPEND_MODE"])
        }
        Revision::Vhdl2019 => {
            EnumType::from_idents(&["READ_MODE", "READ_WRITE_MODE", "WRITE_MODE", "APPEND_MODE"])
        }
    }
}

pub fn file_open_status_type() -> EnumType {
    EnumType::from_idents(&["OPEN_OK", "STATUS_ERROR", "NAME_ERROR", "MODE_ERROR"])
}

/// TIME and its units, values in femtoseconds.
pub const TIME_UNITS: [(&str, i64); 8] = [
    ("fs", 1),
    ("ps", 1_000),
    ("ns", 1_000_000),
    ("us", 1_000_000_000),
    ("ms", 1_000_000_000_000),
    ("sec", 1_000_000_000_000_000),
    ("min", 60_000_000_000_000_000),
    ("hr", 3_600_000_000_000_000_000),
];

pub fn time_type() -> PhysicalType {
    PhysicalType::new(&TIME_UNITS)
}
