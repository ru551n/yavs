use std::borrow::Cow;
use std::collections::HashMap;
use std::fs::File;
use std::io::BufWriter;
use std::path::Path;
use std::time::{Duration, Instant};

use wavefst::{
    ChainCompression, FileType, FstWriter, GeomEntry, Header, MiscAttributeType, ScopeType,
    SignalValue, SupplementalDataType as Dt, SupplementalVarType, SupplementalVariableMetadata,
    VarDir, VarType,
};

use crate::model::*;

#[derive(Debug)]
pub enum Error {
    Fst(wavefst::Error),
    /// A value with the wrong byte length for its declared type, or an unsupported declaration.
    Value(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Fst(e) => write!(f, "fst: {e}"),
            Error::Value(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for Error {}

impl From<wavefst::Error> for Error {
    fn from(e: wavefst::Error) -> Self {
        Error::Fst(e)
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Fst(e.into())
    }
}

#[derive(Clone, Debug)]
pub struct WaveConfig {
    /// Time unit of the `time` argument of [`WaveWriter::change`]: 10^exponent seconds.
    /// Default -15 (femtoseconds, yavs's resolution).
    pub timescale_exponent: i8,
    /// `Some(d)`: a block is also flushed on the next time advance once `d` of wall-clock time has
    /// passed, so a killed run leaves a readable file (the bytes then depend on the run's speed).
    /// `None` (default): blocks end only at `block_change_limit`, so the file's bytes depend on
    /// the changes alone (#666).
    pub flush_interval: Option<Duration>,
    /// Also flush after this many buffered changes (at least twice the number of signals).
    pub block_change_limit: usize,
    pub version: String,
}

impl Default for WaveConfig {
    fn default() -> Self {
        Self {
            timescale_exponent: -15,
            flush_interval: None,
            block_change_limit: 250_000,
            version: format!("yavs {}", env!("CARGO_PKG_VERSION")),
        }
    }
}

#[derive(Clone)]
enum Codec {
    /// Position -> FST character; scalar if `width == 1` and not a vector.
    Logic {
        /// Position -> character, 0 for a position the type does not have.
        chars: Box<[u8; 256]>,
        /// An even position `z` whose character is `0` and `z + 1` is `1`: vectors of only those
        /// are mapped eight at a time (`ZERO_NONE`: no such pair).
        zero: u8,
        width: usize,
        vector: bool,
    },
    Enum {
        width: u32,
    },
    Int {
        bits: u32,
    },
    Real,
}

/// The FST handle of a signal id (0: not declared) and its type's index in the hierarchy.
#[derive(Clone, Copy, Default)]
struct Slot {
    handle: u32,
    ty: u32,
}

/// Records value changes into an FST file. Finishes the file on [`Self::finish`] or drop.
pub struct WaveWriter {
    fst: Option<FstWriter<BufWriter<File>>>,
    slots: Vec<Slot>,
    /// How each type of the hierarchy is declared and its values written.
    plans: Vec<Option<Plan>>,
    enum_tables: HashMap<(String, Vec<String>), u32>,
    now: Option<u64>,
    scratch: Vec<u8>,
    cfg: WaveConfig,
    last_flush: Instant,
    flush_due: bool,
    since_check: u32,
}

impl WaveWriter {
    pub fn create(path: impl AsRef<Path>, h: &Hierarchy, cfg: WaveConfig) -> Result<Self, Error> {
        let file = BufWriter::new(File::create(path)?);
        // A block starts with the value of every variable: with many variables, hold at least
        // twice as many changes per block, or the blocks are mostly those values.
        let block_change_limit = cfg.block_change_limit.max(2 * h.signals());
        let mut fst = FstWriter::builder(file)
            .timescale_exponent(cfg.timescale_exponent)
            .block_change_limit(block_change_limit)
            .chain_compression(ChainCompression::Lz4)
            .build()?;
        let mut w = Self {
            fst: None,
            slots: Vec::new(),
            plans: vec![None; h.types().len()],
            enum_tables: HashMap::new(),
            now: None,
            scratch: Vec::new(),
            last_flush: Instant::now(),
            flush_due: false,
            since_check: 0,
            cfg,
        };
        for it in h.items() {
            match it {
                HierItem::Scope {
                    name,
                    kind,
                    component,
                } => {
                    let comp = (!component.is_empty()).then(|| component.to_string());
                    fst.begin_scope(scope_type(kind), name, comp)?;
                }
                HierItem::Up => fst.end_scope()?,
                HierItem::Signal {
                    name,
                    id,
                    ty,
                    ty_index,
                    port,
                    class,
                } => w.declare_signal(&mut fst, name, id, (ty, ty_index), port, class)?,
            }
        }
        let header = Header {
            version: w.cfg.version.clone(),
            file_type: FileType::Vhdl,
            ..Header::default()
        };
        fst.write_header(header)?;
        w.fst = Some(fst);
        Ok(w)
    }

    fn declare_signal(
        &mut self,
        fst: &mut FstWriter<BufWriter<File>>,
        name: &str,
        id: SignalId,
        (ty, ty_index): (&SigType, usize),
        port: PortDir,
        class: ObjClass,
    ) -> Result<(), Error> {
        let dir = match port {
            PortDir::None => VarDir::Implicit,
            PortDir::In => VarDir::Input,
            PortDir::Out => VarDir::Output,
            PortDir::InOut => VarDir::Inout,
            PortDir::Buffer => VarDir::Buffer,
            PortDir::Linkage => VarDir::Linkage,
        };
        let idx = id.0 as usize;
        if self.slots.len() <= idx {
            self.slots.resize(idx + 1, Slot::default());
        }
        if self.plans[ty_index].is_none() {
            self.plans[ty_index] = Some(self.plan(fst, ty)?);
        }
        let p = self.plans[ty_index].as_ref().expect("planned");
        let name = match ty {
            SigType::Vector { bounds, .. } => {
                format!("{name}[{}:{}]", bounds.left, bounds.right)
            }
            _ => name.to_string(),
        };
        let slot = self.slots[idx];
        if let Some(t) = p.table {
            fst.add_enum_table_ref(t)?;
        }
        let meta = SupplementalVariableMetadata::new(
            p.desc.clone(),
            match class {
                ObjClass::Signal => SupplementalVarType::VhdlSignal,
                ObjClass::Variable => SupplementalVarType::VhdlVariable,
                ObjClass::Constant => SupplementalVarType::VhdlConstant,
            },
            p.data,
        );
        if slot.handle != 0 {
            // What `add_supplemental_variable` writes before a variable, then the alias.
            let argument = (u64::from(meta.variable_type as u8) << 10)
                | (u64::from(meta.data_type as u16) & 0x3ff);
            fst.add_misc_attribute(
                MiscAttributeType::SupplementalVariable,
                meta.type_description,
                argument,
            )?;
            fst.add_alias(p.var_type, dir, name, slot.handle)?;
            return Ok(());
        }
        let handle = fst.add_supplemental_variable(p.var_type, dir, name, p.geom.clone(), meta)?;
        self.slots[idx] = Slot {
            handle,
            ty: ty_index as u32,
        };
        Ok(())
    }

    fn plan(&mut self, fst: &mut FstWriter<BufWriter<File>>, ty: &SigType) -> Result<Plan, Error> {
        let (elem, vec) = match ty {
            SigType::Scalar(ScalarType::Enum(e)) => (Some(e), None),
            SigType::Vector {
                elem,
                bounds,
                type_name,
            } => (Some(elem), Some((bounds, type_name))),
            _ => (None, None),
        };
        if let Some(e) = elem {
            let width = vec.map_or(1, |(b, _)| b.len() as usize);
            if width == 0 {
                return Err(Error::Value("null range vector cannot be recorded".into()));
            }
            if let Some(chars) = e.logic_chars() {
                let vector = vec.is_some();
                let bit = chars.iter().all(|c| matches!(c, b'0' | b'1'));
                let (data, desc) = logic_meta(e, vec.and_then(|(_, t)| t.as_deref()), bit, vector);
                return Ok(Plan {
                    var_type: if bit {
                        VarType::SvBit
                    } else {
                        VarType::SvLogic
                    },
                    geom: GeomEntry::Fixed(width as u32),
                    codec: Codec::Logic {
                        zero: (0..chars.len().saturating_sub(1))
                            .step_by(2)
                            .find(|&z| chars[z] == b'0' && chars[z + 1] == b'1')
                            .map_or(ZERO_NONE, |z| z as u8),
                        chars: Box::new(std::array::from_fn(|i| {
                            chars.get(i).copied().unwrap_or(0)
                        })),
                        width,
                        vector,
                    },
                    table: None,
                    data,
                    desc,
                });
            }
            if vec.is_some() {
                return Err(Error::Value(format!(
                    "vectors of {} are not supported; declare an array scope",
                    e.name
                )));
            }
            let w = (usize::BITS - e.literals.len().saturating_sub(1).leading_zeros()).max(1);
            let key = (e.name.clone(), e.literals.clone());
            let table = match self.enum_tables.get(&key) {
                Some(t) => *t,
                None => {
                    let vals: Vec<String> = (0..e.literals.len())
                        .map(|i| format!("{i:0w$b}", w = w as usize))
                        .collect();
                    let entries: Vec<(&str, &str)> = e
                        .literals
                        .iter()
                        .map(String::as_str)
                        .zip(vals.iter().map(String::as_str))
                        .collect();
                    let tname = e.name.replace(char::is_whitespace, "_");
                    let t = fst.create_enum_table(&tname, &entries, w as usize)?;
                    self.enum_tables.insert(key, t);
                    t
                }
            };
            let data = match e.name.to_ascii_lowercase().as_str() {
                "boolean" => Dt::VhdlBoolean,
                "character" => Dt::VhdlCharacter,
                _ => Dt::None,
            };
            return Ok(Plan {
                var_type: VarType::SvEnum,
                geom: GeomEntry::Fixed(w),
                codec: Codec::Enum { width: w },
                table: Some(table),
                data,
                desc: e.name.clone(),
            });
        }
        let SigType::Scalar(sc) = ty else {
            unreachable!()
        };
        Ok(match sc {
            ScalarType::Integer { name, bits } => {
                if !(1..=64).contains(bits) {
                    return Err(Error::Value(format!("integer width {bits} out of range")));
                }
                let data = match name.to_ascii_lowercase().as_str() {
                    "natural" => Dt::VhdlNatural,
                    "positive" => Dt::VhdlPositive,
                    _ => Dt::VhdlInteger,
                };
                Plan {
                    var_type: VarType::VcdInteger,
                    geom: GeomEntry::Fixed(*bits),
                    codec: Codec::Int { bits: *bits },
                    table: None,
                    data,
                    desc: name.clone(),
                }
            }
            ScalarType::Time => Plan {
                var_type: VarType::VcdTime,
                geom: GeomEntry::Fixed(64),
                codec: Codec::Int { bits: 64 },
                table: None,
                data: Dt::VhdlTime,
                desc: "time".into(),
            },
            ScalarType::Real => Plan {
                var_type: VarType::VcdReal,
                geom: GeomEntry::Real,
                codec: Codec::Real,
                table: None,
                data: Dt::VhdlReal,
                desc: "real".into(),
            },
            ScalarType::Enum(_) => unreachable!(),
        })
    }

    /// Record a value change. Times must not decrease. Ids that were filtered out by the
    /// selection (or never declared) are ignored. Emit every signal's initial value at the start
    /// time; undeclared initial values read as unknown.
    pub fn change(&mut self, time: u64, id: SignalId, value: &[u8]) -> Result<(), Error> {
        let Some(slot) = self.slots.get(id.0 as usize).filter(|s| s.handle != 0) else {
            return Ok(());
        };
        let codec = &self.plans[slot.ty as usize]
            .as_ref()
            .expect("a declared signal has a plan")
            .codec;
        let fst = self.fst.as_mut().expect("writer finished");
        if self.now.is_some_and(|t| time > t) {
            self.since_check += 1;
            if self.since_check >= 256 {
                self.since_check = 0;
                self.flush_due |= self
                    .cfg
                    .flush_interval
                    .is_some_and(|i| self.last_flush.elapsed() >= i);
            }
            if self.flush_due {
                fst.flush()?;
                self.flush_due = false;
                self.last_flush = Instant::now();
            }
        }
        self.now = Some(time);
        let bad = |what: &str| Error::Value(format!("signal {}: {what}", id.0));
        let packed_buf;
        let v = match codec {
            Codec::Logic {
                chars,
                zero,
                width,
                vector,
            } => {
                if value.len() != *width {
                    return Err(bad("wrong element count"));
                }
                if !*vector {
                    let c = chars[value[0] as usize];
                    if c == 0 {
                        return Err(bad("enum position out of range"));
                    }
                    if c == b'0' || c == b'1' {
                        fst.emit_binary_change(time, slot.handle, c == b'1')?;
                    } else {
                        fst.emit_change(time, slot.handle, SignalValue::Bit(c as char))?;
                    }
                    return Ok(());
                }
                self.scratch.clear();
                if *zero == ZERO_NONE || !ascii_binary(value, *zero, &mut self.scratch) {
                    self.scratch.clear();
                    let mut missing = false;
                    self.scratch.extend(value.iter().map(|&p| {
                        let c = chars[p as usize];
                        missing |= c == 0;
                        c
                    }));
                    if missing {
                        return Err(bad("enum position out of range"));
                    }
                }
                SignalValue::Bytes(Cow::Borrowed(&self.scratch))
            }
            Codec::Enum { width } => {
                let b: [u8; 4] = value.try_into().map_err(|_| bad("expected u32"))?;
                packed_buf = packed(u32::from_le_bytes(b) as u64, *width);
                packed_value(&packed_buf, *width)
            }
            Codec::Int { bits } => {
                let b: [u8; 8] = value.try_into().map_err(|_| bad("expected i64"))?;
                packed_buf = packed(i64::from_le_bytes(b) as u64, *bits);
                packed_value(&packed_buf, *bits)
            }
            Codec::Real => {
                let b: [u8; 8] = value.try_into().map_err(|_| bad("expected f64"))?;
                SignalValue::Real(f64::from_le_bytes(b))
            }
        };
        fst.emit_change(time, slot.handle, v)?;
        Ok(())
    }

    /// Force a block to disk now (only call between time steps).
    pub fn flush(&mut self) -> Result<(), Error> {
        if let Some(f) = self.fst.as_mut() {
            f.flush()?;
            self.last_flush = Instant::now();
        }
        Ok(())
    }

    /// Write the final block and close the file.
    pub fn finish(mut self) -> Result<(), Error> {
        self.close()
    }

    fn close(&mut self) -> Result<(), Error> {
        if let Some(f) = self.fst.take() {
            f.finish()?;
        }
        Ok(())
    }
}

impl Drop for WaveWriter {
    fn drop(&mut self) {
        let _ = self.close();
    }
}

#[derive(Clone)]
struct Plan {
    var_type: VarType,
    geom: GeomEntry,
    codec: Codec,
    table: Option<u32>,
    data: Dt,
    desc: String,
}

const ZERO_NONE: u8 = u8::MAX;

/// Appends the characters of `value` (positions) if they are all `zero` or `zero + 1` (an even
/// `zero`, whose characters are `0` and `1`), eight at a time; false (and nothing
/// appended) otherwise.
fn ascii_binary(value: &[u8], zero: u8, out: &mut Vec<u8>) -> bool {
    const ONES: u64 = 0x0101_0101_0101_0101;
    let z = u64::from(zero) * ONES;
    let (chunks, rest) = value.as_chunks::<8>();
    for c in chunks {
        let y = u64::from_le_bytes(*c) ^ z;
        if y & !ONES != 0 {
            return false;
        }
        out.extend_from_slice(&(y + 0x3030_3030_3030_3030).to_le_bytes());
    }
    rest.iter().all(|&p| {
        let y = p ^ zero;
        out.push(b'0' + y);
        y < 2
    })
}

/// Low `bits` bits of `v`, MSB first, and the number of bytes they take.
fn packed(v: u64, bits: u32) -> ([u8; 8], usize) {
    let shifted = if bits >= 64 { v } else { v << (64 - bits) };
    (shifted.to_be_bytes(), bits.div_ceil(8) as usize)
}

fn packed_value(p: &([u8; 8], usize), width: u32) -> SignalValue<'_> {
    SignalValue::PackedBits {
        width,
        bits: Cow::Borrowed(&p.0[..p.1]),
    }
}

fn logic_meta(e: &EnumType, type_name: Option<&str>, bit: bool, vector: bool) -> (Dt, String) {
    let tn = type_name.map(str::to_ascii_lowercase);
    let data = match (
        tn.as_deref(),
        bit,
        e.name.to_ascii_lowercase().as_str(),
        vector,
    ) {
        (Some("unsigned"), ..) => Dt::VhdlUnsigned,
        (Some("signed"), ..) => Dt::VhdlSigned,
        (_, true, _, false) => Dt::VhdlBit,
        (_, true, _, true) => Dt::VhdlBitVector,
        (_, _, "std_ulogic", false) => Dt::VhdlStdUlogic,
        (_, _, "std_ulogic", true) => Dt::VhdlStdUlogicVector,
        (_, _, _, false) => Dt::VhdlStdLogic,
        (_, _, _, true) => Dt::VhdlStdLogicVector,
    };
    let desc = match type_name {
        Some(t) => t.to_string(),
        None if vector => format!("{}_vector", e.name),
        None => e.name.clone(),
    };
    (data, desc)
}

fn scope_type(k: ScopeKind) -> ScopeType {
    match k {
        ScopeKind::Instance => ScopeType::VhdlArchitecture,
        ScopeKind::Block => ScopeType::VhdlBlock,
        ScopeKind::Process => ScopeType::VhdlProcess,
        ScopeKind::Procedure => ScopeType::VhdlProcedure,
        ScopeKind::Function => ScopeType::VhdlFunction,
        ScopeKind::ForGenerate => ScopeType::VhdlForGenerate,
        ScopeKind::IfGenerate => ScopeType::VhdlIfGenerate,
        ScopeKind::Generate => ScopeType::VhdlGenerate,
        ScopeKind::Package => ScopeType::VhdlPackage,
        ScopeKind::Record => ScopeType::VhdlRecord,
        ScopeKind::Array => ScopeType::SvArray,
    }
}

#[cfg(test)]
mod tests {
    use super::ascii_binary;

    #[test]
    fn binary_vectors_map_eight_at_a_time() {
        for len in 0..21usize {
            let v: Vec<u8> = (0..len).map(|i| 2 + ((i * 7 % 3) & 1) as u8).collect();
            let mut out = Vec::new();
            assert!(ascii_binary(&v, 2, &mut out));
            let want: Vec<u8> = v.iter().map(|p| b'0' + (p - 2)).collect();
            assert_eq!(out, want, "len {len}");
        }
        for at in [0, 7, 8, 15, 19] {
            let mut v = vec![2u8; 20];
            v[at] = 4;
            assert!(!ascii_binary(&v, 2, &mut Vec::new()), "position {at}");
        }
    }
}
