//! IEEE 1164 `std_ulogic`: the nine values and their tables.
//!
//! The tables are transcribed from the IEEE std_logic_1164 package body (the spec) and checked
//! against it by `tests/ieee_tables.rs`. Rows and columns are in the order `U X 0 1 Z W L H -`.

use core::fmt;

/// One `std_ulogic` value. The discriminant is the layout-independent 4-bit code.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
#[repr(u8)]
pub enum StdULogic {
    U,
    X,
    Zero,
    One,
    Z,
    W,
    L,
    H,
    DontCare,
}

use StdULogic::*;

pub type Tab = [[u8; 16]; 16];

const CHARS: &[u8; 9] = b"UX01ZWLH-";

/// All values in code order.
pub const ALL: [StdULogic; 9] = [U, X, Zero, One, Z, W, L, H, DontCare];

const fn code(c: u8) -> u8 {
    let mut i = 0;
    while i < 9 {
        if CHARS[i] == c {
            return i as u8;
        }
        i += 1;
    }
    panic!("bad table char")
}

const fn tab2(rows: [&[u8; 9]; 9]) -> Tab {
    let mut t = [[0u8; 16]; 16];
    let mut r = 0;
    while r < 9 {
        let mut c = 0;
        while c < 9 {
            t[r][c] = code(rows[r][c]);
            c += 1;
        }
        r += 1;
    }
    t
}

const fn tab1(row: &[u8; 9]) -> [u8; 16] {
    let mut t = [0u8; 16];
    let mut c = 0;
    while c < 9 {
        t[c] = code(row[c]);
        c += 1;
    }
    t
}

pub const RESOLVE: Tab = tab2([
    b"UUUUUUUUU",
    b"UXXXXXXXX",
    b"UX0X0000X",
    b"UXX11111X",
    b"UX01ZWLHX",
    b"UX01WWWWX",
    b"UX01LWLWX",
    b"UX01HWWHX",
    b"UXXXXXXXX",
]);
pub const AND: Tab = tab2([
    b"UU0UUU0UU",
    b"UX0XXX0XX",
    b"000000000",
    b"UX01XX01X",
    b"UX0XXX0XX",
    b"UX0XXX0XX",
    b"000000000",
    b"UX01XX01X",
    b"UX0XXX0XX",
]);
pub const OR: Tab = tab2([
    b"UUU1UUU1U",
    b"UXX1XXX1X",
    b"UX01XX01X",
    b"111111111",
    b"UXX1XXX1X",
    b"UXX1XXX1X",
    b"UX01XX01X",
    b"111111111",
    b"UXX1XXX1X",
]);
pub const XOR: Tab = tab2([
    b"UUUUUUUUU",
    b"UXXXXXXXX",
    b"UX01XX01X",
    b"UX10XX10X",
    b"UXXXXXXXX",
    b"UXXXXXXXX",
    b"UX01XX01X",
    b"UX10XX10X",
    b"UXXXXXXXX",
]);
/// `?=` on `std_ulogic` (1164-2008 predefined match table).
pub const MATCH: Tab = tab2([
    b"UUUUUUUU1",
    b"UXXXXXXX1",
    b"UX10XX101",
    b"UX01XX011",
    b"UXXXXXXX1",
    b"UXXXXXXX1",
    b"UX10XX101",
    b"UX01XX011",
    b"111111111",
]);
pub const NOT: [u8; 16] = tab1(b"UX10XX10X");
pub const TO_X01: [u8; 16] = tab1(b"XX01XX01X");
pub const TO_X01Z: [u8; 16] = tab1(b"XX01ZX01X");
pub const TO_UX01: [u8; 16] = tab1(b"UX01XX01X");

impl StdULogic {
    #[inline]
    pub(crate) fn from_code(c: u8) -> Self {
        ALL[c as usize]
    }
    #[inline]
    fn bin(self, o: Self, t: &Tab) -> Self {
        Self::from_code(t[self as usize][o as usize])
    }
    #[inline]
    fn un(self, t: &[u8; 16]) -> Self {
        Self::from_code(t[self as usize])
    }

    #[doc(hidden)]
    pub fn from_code_pub(c: u8) -> Self {
        Self::from_code(c)
    }

    pub fn from_char(c: char) -> Option<Self> {
        let i = CHARS.iter().position(|&b| b as char == c)?;
        Some(ALL[i])
    }
    pub fn to_char(self) -> char {
        CHARS[self as usize] as char
    }
    pub fn from_bool(b: bool) -> Self {
        if b { One } else { Zero }
    }
    /// `to_bit(x, xmap)`: `0`/`L` -> false, `1`/`H` -> true, anything else -> `xmap`.
    pub fn to_bit(self, xmap: bool) -> bool {
        match self {
            Zero | L => false,
            One | H => true,
            _ => xmap,
        }
    }
    /// The `??` condition operator: true for `1` and `H`.
    pub fn is_true(self) -> bool {
        matches!(self, One | H)
    }
    /// `is_x`: not one of `0 1 L H`.
    pub fn is_x(self) -> bool {
        !matches!(self, Zero | One | L | H)
    }

    pub fn and(self, o: Self) -> Self {
        self.bin(o, &AND)
    }
    pub fn or(self, o: Self) -> Self {
        self.bin(o, &OR)
    }
    pub fn xor(self, o: Self) -> Self {
        self.bin(o, &XOR)
    }
    pub fn nand(self, o: Self) -> Self {
        self.and(o).not()
    }
    pub fn nor(self, o: Self) -> Self {
        self.or(o).not()
    }
    pub fn xnor(self, o: Self) -> Self {
        self.xor(o).not()
    }
    #[allow(clippy::should_implement_trait)]
    pub fn not(self) -> Self {
        self.un(&NOT)
    }
    /// Two-driver resolution (`resolved` for one pair).
    pub fn resolve(self, o: Self) -> Self {
        self.bin(o, &RESOLVE)
    }
    /// `?=`: matching equality as a `std_ulogic`.
    pub fn match_eq(self, o: Self) -> Self {
        self.bin(o, &MATCH)
    }
    pub fn to_x01(self) -> Self {
        self.un(&TO_X01)
    }
    pub fn to_x01z(self) -> Self {
        self.un(&TO_X01Z)
    }
    pub fn to_ux01(self) -> Self {
        self.un(&TO_UX01)
    }
    /// `rising_edge`: `self` is the new value, `prev` the previous one.
    pub fn rising_edge(prev: Self, new: Self) -> bool {
        new.to_x01() == One && prev.to_x01() == Zero
    }
    pub fn falling_edge(prev: Self, new: Self) -> bool {
        new.to_x01() == Zero && prev.to_x01() == One
    }
}

impl fmt::Display for StdULogic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_char())
    }
}
