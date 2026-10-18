//! File objects, `line` values and the std.textio natives.
//!
//! Representation (ir.md 9):
//! - a file object is a 4-byte cell holding an index + 1 into `Runtime::files` (0: never
//!   opened); `std.textio.INPUT` and `OUTPUT` are the pre-opened [`FILE_INPUT`] and
//!   [`FILE_OUTPUT`]. File parameters of natives are the address of the cell.
//! - a `line` is an 8-byte access handle; its object is `[descriptor][bytes]` with the
//!   descriptor `1 to len`. Natives take the address of the line variable, replace the
//!   object they change and deallocate the old one. A line stored in shared memory (a shared
//!   variable or a protected object's state) is allocated in the shared heap.
//! - BIT and BOOLEAN elements of logic arrays (`bit_vector`) are the std_ulogic codes of
//!   '0' and '1' in the bit-planes; element 0 of the planes is the rightmost element.
//!
//! Disk files are read and written directly through `yavs_std::file::TextFile`, not through
//! kernel streams, so that a process that closes a file and opens it again for reading in
//! the same delta sees what it wrote. A file object is used by one process, or by serial
//! processes in elaboration order, so the bytes written do not depend on `--threads`.
//! `STD_OUTPUT` goes through the kernel's OUTPUT stream, ordered with `report`.

use std::io::Write;

use yavs_ir::{TyId, Width};
use yavs_kernel::{Ctx, StreamId};
use yavs_std::file::{
    FileError, FileOpenKind, FileOpenState, FileOpenStatus, FileOrigin, STD_OUTPUT, TextFile,
};
use yavs_std::textio::{self as tio, Side};
use yavs_std::types::{Bounds, RealBounds, time_type};

use crate::exec::{Fault, Machine, R, SHARED, SHARED_HEAP, add_addr, ice, rt_err, split};
use crate::heap::{self, Heap, HeapError, Obj};

/// The file cell value of `std.textio.INPUT` (STD_INPUT, read mode).
pub const FILE_INPUT: u32 = 1;
/// The file cell value of `std.textio.OUTPUT` (STD_OUTPUT, write mode).
pub const FILE_OUTPUT: u32 = 2;

pub(crate) enum FileSlot {
    Text(TextFile),
    /// STD_OUTPUT: lines go to the kernel's OUTPUT stream.
    Output {
        open: bool,
    },
}

impl FileSlot {
    pub(crate) fn preopened() -> Vec<FileSlot> {
        vec![
            FileSlot::Text(
                TextFile::from_reader(Box::new(std::io::stdin())).named(yavs_std::file::STD_INPUT),
            ),
            FileSlot::Output { open: true },
        ]
    }

    fn is_open(&self) -> bool {
        match self {
            FileSlot::Text(f) => f.state() == FileOpenState::Open,
            FileSlot::Output { open } => *open,
        }
    }
}

/// External name and FILE_OPEN_KIND position of the open file in a file cell (`value` is the
/// cell's content), for `std.reflection` file mirrors.
pub(crate) fn file_info(m: &Machine, value: u64) -> R<(Vec<u8>, u64)> {
    let files =
        m.rt.files
            .lock()
            .map_err(|_| ice("file table lock poisoned"))?;
    let slot = value
        .checked_sub(1)
        .and_then(|i| files.get(i as usize))
        .ok_or_else(|| Fault::Runtime(FileError::NotOpen.to_string()))?;
    Ok(match slot {
        FileSlot::Text(f) => {
            let kind = f.mode().map_err(file_err)?;
            (f.logical_name().to_vec(), kind as u64)
        }
        FileSlot::Output { open: true } => (STD_OUTPUT.to_vec(), FileOpenKind::Write as u64),
        FileSlot::Output { open: false } => return Err(file_err(FileError::NotOpen)),
    })
}

type Args<'a> = &'a [u64];

pub(crate) fn arg(a: Args<'_>, i: usize) -> R<u64> {
    a.get(i)
        .copied()
        .ok_or_else(|| ice(format!("native argument {i} missing")))
}

fn file_err(e: FileError) -> Fault {
    Fault::Runtime(e.to_string())
}

fn side(v: u64) -> R<Side> {
    match v {
        0 => Ok(Side::Right),
        1 => Ok(Side::Left),
        _ => Err(ice(format!("SIDE position {v}"))),
    }
}

fn width(v: u64) -> R<usize> {
    usize::try_from(v as i64).map_err(|_| ice(format!("negative WIDTH {}", v as i64)))
}

fn origin(v: u64) -> R<FileOrigin> {
    match v {
        0 => Ok(FileOrigin::Begin),
        1 => Ok(FileOrigin::Current),
        2 => Ok(FileOrigin::End),
        _ => Err(ice(format!("FILE_ORIGIN_KIND position {v}"))),
    }
}

/// `OUTPUT` text: through the kernel during simulation, straight to stdout during
/// elaboration (before any simulation output).
fn emit(ctx: &mut Option<&mut Ctx<'_>>, bytes: &[u8]) -> R<()> {
    match ctx.as_deref_mut() {
        Some(c) => c.output(StreamId::OUTPUT, bytes),
        None => std::io::stdout()
            .write_all(bytes)
            .map_err(|e| Fault::Runtime(format!("write to STD_OUTPUT: {e}")))?,
    }
    Ok(())
}

impl Machine {
    // ---- fat arrays ----

    /// Data address and length of the one-dimensional fat pair at `fat`.
    pub(crate) fn fat1(&self, fat: u64) -> R<(u64, u64)> {
        let data = self.load(fat, Width::B8)?;
        let dope = self.load(add_addr(fat, 8)?, Width::B8)?;
        let l = self.load(dope, Width::B8)? as i64;
        let r = self.load(add_addr(dope, 8)?, Width::B8)? as i64;
        let dir = self.load(add_addr(dope, 16)?, Width::B8)?;
        let n = yavs_ir::dim_len(l, r, dir == 1)
            .ok_or_else(|| Fault::Runtime("array too long".into()))?;
        Ok((data, n))
    }

    pub(crate) fn fat_bytes(&self, fat: u64) -> R<Vec<u8>> {
        let (data, n) = self.fat1(fat)?;
        Ok(self.rd(data, n)?.to_vec())
    }

    /// A `bit_vector` (or boolean array) left to right.
    pub(crate) fn bits_at(&self, data: u64, n: u64) -> R<Vec<bool>> {
        let v = self.planes(data, n)?;
        // ponytail: also accepts positions 0/1 (yavs-elab's encoding of BIT leaves) until
        // one encoding is decided; written values are always '0'/'1' codes.
        (0..n as usize)
            .map(|i| match v.get(n as usize - 1 - i) as u8 {
                0 | 2 => Ok(false),
                1 | 3 => Ok(true),
                c => Err(ice(format!("bit array element with std_ulogic code {c}"))),
            })
            .collect()
    }

    fn put_bits(&mut self, data: u64, bits: &[bool]) -> R<()> {
        let n = bits.len();
        let v = yavs_kernel::LogicVec::from_fn(n, |i| {
            yavs_logic::StdULogic::from_code_pub(if bits[n - 1 - i] { 3 } else { 2 })
        });
        self.put_planes(data, &v)
    }

    // ---- lines ----

    fn heap_for(&mut self, cell: u64) -> R<&mut Heap> {
        if matches!(split(cell).0, SHARED | SHARED_HEAP) {
            let sh = self.shared_mut()?;
            Ok(sh.heap.get_or_insert_with(|| Heap::new(heap::SHARED)))
        } else {
            Ok(&mut self.heap)
        }
    }

    fn alloc_in(&mut self, cell: u64, kind: Obj, data: Vec<u8>) -> R<u64> {
        self.heap_for(cell)?
            .alloc(kind, data)
            .map_err(|_| Fault::Runtime("heap object limit (16M objects per heap) exceeded".into()))
    }

    /// A string object `[1 to len][bytes]` in the heap that suits `cell`.
    pub(crate) fn new_line(&mut self, cell: u64, s: &[u8]) -> R<u64> {
        let mut obj: Vec<u8> = [1i64, s.len() as i64, 0]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        obj.extend_from_slice(s);
        self.alloc_in(cell, Obj::Bytes, obj)
    }

    pub(crate) fn alloc_line_vector(&mut self, cell: u64, obj: Vec<u8>) -> R<u64> {
        self.alloc_in(cell, Obj::Handles, obj)
    }

    /// The handles in a `line_vector` object.
    pub(crate) fn line_vector_handles(&mut self, h: u64) -> R<Vec<u64>> {
        let (hp, _) = self.heap_of(h)?;
        let i = hp
            .resolve(h)
            .map_err(|_| Fault::Runtime("directory items were already deallocated".into()))?;
        let d = hp.data(i).ok_or_else(|| ice("heap object vanished"))?;
        Ok(d.get(24..)
            .unwrap_or(&[])
            .as_chunks::<8>()
            .0
            .iter()
            .map(|c| u64::from_le_bytes(*c))
            .collect())
    }

    pub(crate) fn free_handle(&mut self, h: u64) -> R<()> {
        if h == 0 {
            return Ok(());
        }
        let (hp, _) = self.heap_of(h)?;
        match hp.free(h) {
            Ok(()) => Ok(()),
            Err(HeapError::Stale) => {
                rt_err("deallocate of an access value whose object was already deallocated")
            }
            Err(e) => Err(ice(format!("deallocate: {e:?}"))),
        }
    }

    /// The string designated by the line variable at `cell`; `None` for null.
    pub(crate) fn line_get(&mut self, cell: u64) -> R<Option<Vec<u8>>> {
        let h = self.load(cell, Width::B8)?;
        if h == 0 {
            return Ok(None);
        }
        let (hp, _) = self.heap_of(h)?;
        let i = hp
            .resolve(h)
            .map_err(|_| Fault::Runtime("line designates an object that was deallocated".into()))?;
        let d = hp.data(i).ok_or_else(|| ice("heap object vanished"))?;
        let w = |k: usize| {
            let mut a = [0; 8];
            a.copy_from_slice(&d[k * 8..k * 8 + 8]);
            i64::from_le_bytes(a)
        };
        if d.len() < 24 {
            return Err(ice("line object without descriptor"));
        }
        let n = yavs_ir::dim_len(w(0), w(1), w(2) == 1).ok_or_else(|| ice("line length"))?;
        d.get(24..24 + n as usize)
            .map(|b| Some(b.to_vec()))
            .ok_or_else(|| ice("line object shorter than its descriptor"))
    }

    /// Replace the line at `cell` by a new object holding `s`, deallocating the old one.
    pub(crate) fn line_set(&mut self, cell: u64, s: &[u8]) -> R<()> {
        let old = self.load(cell, Width::B8)?;
        self.free_handle(old)?;
        let h = self.new_line(cell, s)?;
        self.store(cell, Width::B8, h)
    }

    // ---- files ----

    fn with_file<T>(&mut self, cell: u64, f: impl FnOnce(&mut FileSlot) -> R<T>) -> R<T> {
        let c = self.load(cell, Width::B4)?;
        let rt = self.rt.clone();
        let mut files = rt
            .files
            .lock()
            .map_err(|_| ice("file table lock poisoned"))?;
        let slot = c
            .checked_sub(1)
            .and_then(|i| files.get_mut(i as usize))
            .ok_or_else(|| Fault::Runtime(FileError::NotOpen.to_string()))?;
        f(slot)
    }

    fn file_state_of(&mut self, cell: u64) -> R<bool> {
        if self.load(cell, Width::B4)? == 0 {
            return Ok(false);
        }
        self.with_file(cell, |s| Ok(s.is_open()))
    }

    fn file_open(&mut self, cell: u64, name: &[u8], kind: u64) -> R<FileOpenStatus> {
        let kind = match kind {
            0 => FileOpenKind::Read,
            1 => FileOpenKind::Write,
            2 => FileOpenKind::Append,
            3 => FileOpenKind::ReadWrite,
            _ => return Err(ice(format!("FILE_OPEN_KIND position {kind}"))),
        };
        if self.file_state_of(cell)? {
            return Ok(FileOpenStatus::StatusError);
        }
        let slot =
            if name == STD_OUTPUT && matches!(kind, FileOpenKind::Write | FileOpenKind::Append) {
                FileSlot::Output { open: true }
            } else {
                match TextFile::open(name, kind) {
                    Ok(f) => FileSlot::Text(f),
                    Err(s) => return Ok(s),
                }
            };
        let c = self.load(cell, Width::B4)?;
        let rt = self.rt.clone();
        let mut files = rt
            .files
            .lock()
            .map_err(|_| ice("file table lock poisoned"))?;
        let index = match c.checked_sub(1) {
            Some(i) if (i as usize) < files.len() => {
                files[i as usize] = slot;
                i
            }
            _ => {
                if files.len() >= u32::MAX as usize {
                    return rt_err("too many file objects");
                }
                files.push(slot);
                files.len() as u64 - 1
            }
        };
        drop(files);
        self.store(cell, Width::B4, index + 1)?;
        Ok(FileOpenStatus::Ok)
    }

    fn writeline_to(&mut self, ctx: &mut Option<&mut Ctx<'_>>, f: u64, line: &[u8]) -> R<()> {
        let mut text = line.to_vec();
        let out = self.with_file(f, |s| match s {
            FileSlot::Text(t) => t.writeline(&mut text).map(|()| false).map_err(file_err),
            FileSlot::Output { open: true } => Ok(true),
            FileSlot::Output { open: false } => Err(file_err(FileError::NotOpen)),
        })?;
        if out {
            text.push(b'\n');
            emit(ctx, &text)?;
        }
        Ok(())
    }
}

// ---- natives ----

use crate::native::NativeFn as Native;

/// The implementation of a textio or file native, `None` when the symbol is not one.
pub(crate) fn lookup(symbol: &str, family: Option<&str>) -> Option<Native> {
    let f: Native = match (symbol, family) {
        (_, Some("file_open")) => file_open,
        (_, Some("file_close")) => file_close,
        (_, Some("endfile")) => endfile,
        (_, Some("flush")) => flush,
        (_, Some("file_state")) => file_state,
        (_, Some("file_mode")) => file_mode,
        (_, Some("file_position")) => file_position,
        (_, Some("file_size")) => file_size,
        (_, Some("file_seek")) => file_seek,
        (_, Some("file_truncate")) => file_truncate,
        (_, Some("file_rewind")) => file_rewind,
        (_, Some("file_canseek")) => file_canseek,
        (_, Some("deallocate")) => deallocate,
        ("yavs.textio.readline.text_line", _) => readline,
        ("yavs.textio.writeline.text_line", _) => writeline,
        ("yavs.textio.tee.text_line", _) => tee,
        ("yavs.textio.justify.string_side_width.r_string", _) => justify,
        ("yavs.textio.sread.line_string_natural", _) => sread,
        ("yavs.textio.write.line_real_string", _) => write_real_format,
        ("yavs.textio.write.line_real_side_width_natural", _) => write_real,
        ("yavs.textio.write.line_time_side_width_time", _) => write_time,
        ("yavs.textio.write.line_bit_side_width", _) => write_bit,
        ("yavs.textio.write.line_bit_vector_side_width", _) => write_bit_vector,
        ("yavs.textio.write.line_boolean_side_width", _) => write_boolean,
        ("yavs.textio.write.line_character_side_width", _) => write_character,
        ("yavs.textio.write.line_integer_side_width", _) => write_integer,
        ("yavs.textio.write.line_string_side_width", _) => write_string,
        ("yavs.textio.hwrite.line_bit_vector_side_width", _) => hwrite,
        ("yavs.textio.owrite.line_bit_vector_side_width", _) => owrite,
        (s, _) => {
            // READ forms with and without GOOD share one implementation.
            let r = s.strip_prefix("yavs.textio.")?;
            let r = r
                .strip_suffix("_boolean")
                .filter(|_| r != "read.line_boolean")
                .unwrap_or(r);
            match r {
                "read.line_bit" => read_bit,
                "read.line_bit_vector" => read_bit_vector,
                "read.line_boolean" => read_boolean,
                "read.line_character" => read_character,
                "read.line_integer" => read_integer,
                "read.line_real" => read_real,
                "read.line_string" => read_string,
                "read.line_time" => read_time,
                "hread.line_bit_vector" => hread,
                "oread.line_bit_vector" => oread,
                _ => return None,
            }
        }
    };
    Some(f)
}

fn file_open(
    m: &mut Machine,
    _: &mut Option<&mut Ctx<'_>>,
    _: Option<TyId>,
    a: Args<'_>,
) -> R<Option<u64>> {
    // FILE_OPEN (F, name, kind) or FILE_OPEN (Status, F, name, kind).
    let (status, k) = if a.len() == 4 {
        (Some(arg(a, 0)?), 1)
    } else {
        (None, 0)
    };
    let cell = arg(a, k)?;
    let name = m.fat_bytes(arg(a, k + 1)?)?;
    let s = m.file_open(cell, &name, arg(a, k + 2)?)?;
    match status {
        Some(st) => m.store(st, Width::B1, s as u64)?,
        None if s != FileOpenStatus::Ok => {
            let what = match s {
                FileOpenStatus::StatusError => "the file object is already open",
                FileOpenStatus::NameError => "the file does not exist or cannot be created",
                _ => "the file cannot be opened in this mode",
            };
            return rt_err(format!(
                "file_open of \"{}\" failed: {what}",
                yavs_std::error::latin1(&name)
            ));
        }
        None => {}
    }
    Ok(None)
}

fn file_close(
    m: &mut Machine,
    _: &mut Option<&mut Ctx<'_>>,
    _: Option<TyId>,
    a: Args<'_>,
) -> R<Option<u64>> {
    let cell = arg(a, 0)?;
    if m.load(cell, Width::B4)? != 0 {
        m.with_file(cell, |s| match s {
            FileSlot::Text(f) => f.close().map_err(file_err),
            FileSlot::Output { open } => {
                *open = false;
                Ok(())
            }
        })?;
    }
    Ok(None)
}

fn text<T>(
    m: &mut Machine,
    cell: u64,
    f: impl FnOnce(&mut TextFile) -> Result<T, FileError>,
) -> R<T> {
    m.with_file(cell, |s| match s {
        FileSlot::Text(t) => f(t).map_err(file_err),
        FileSlot::Output { open: true } => rt_err("operation not supported on STD_OUTPUT"),
        FileSlot::Output { open: false } => Err(file_err(FileError::NotOpen)),
    })
}

fn endfile(
    m: &mut Machine,
    _: &mut Option<&mut Ctx<'_>>,
    _: Option<TyId>,
    a: Args<'_>,
) -> R<Option<u64>> {
    text(m, arg(a, 0)?, |f| f.endfile()).map(|b| Some(b.into()))
}

fn flush(
    m: &mut Machine,
    _: &mut Option<&mut Ctx<'_>>,
    _: Option<TyId>,
    a: Args<'_>,
) -> R<Option<u64>> {
    // STD_OUTPUT is flushed by the kernel at the end of simulation.
    m.with_file(arg(a, 0)?, |s| match s {
        FileSlot::Text(t) => t.flush().map_err(file_err),
        FileSlot::Output { .. } => Ok(()),
    })?;
    Ok(None)
}

fn file_state(
    m: &mut Machine,
    _: &mut Option<&mut Ctx<'_>>,
    _: Option<TyId>,
    a: Args<'_>,
) -> R<Option<u64>> {
    let open = m.file_state_of(arg(a, 0)?)?;
    Ok(Some(if open { 0 } else { 1 }))
}

fn file_mode(
    m: &mut Machine,
    _: &mut Option<&mut Ctx<'_>>,
    _: Option<TyId>,
    a: Args<'_>,
) -> R<Option<u64>> {
    let k = m.with_file(arg(a, 0)?, |s| match s {
        FileSlot::Text(t) => t.mode().map_err(file_err),
        FileSlot::Output { open: true } => Ok(FileOpenKind::Write),
        FileSlot::Output { open: false } => Err(file_err(FileError::NotOpen)),
    })?;
    Ok(Some(k as u64))
}

fn natural(v: impl TryInto<i64>, what: &str) -> R<Option<u64>> {
    match v.try_into() {
        Ok(v) if v >= 0 => Ok(Some(v as u64)),
        _ => rt_err(format!("{what} is not a NATURAL")),
    }
}

fn file_position(
    m: &mut Machine,
    _: &mut Option<&mut Ctx<'_>>,
    _: Option<TyId>,
    a: Args<'_>,
) -> R<Option<u64>> {
    let o = origin(arg(a, 1)?)?;
    let p = text(m, arg(a, 0)?, |f| f.position(o))?;
    natural(p, "the file position relative to this origin")
}

fn file_size(
    m: &mut Machine,
    _: &mut Option<&mut Ctx<'_>>,
    _: Option<TyId>,
    a: Args<'_>,
) -> R<Option<u64>> {
    let n = text(m, arg(a, 0)?, |f| f.size())?;
    natural(n, "the file size")
}

fn file_seek(
    m: &mut Machine,
    _: &mut Option<&mut Ctx<'_>>,
    _: Option<TyId>,
    a: Args<'_>,
) -> R<Option<u64>> {
    let (off, o) = (arg(a, 1)? as i64, origin(arg(a, 2)?)?);
    text(m, arg(a, 0)?, |f| f.seek(off, o))?;
    Ok(None)
}

fn file_truncate(
    m: &mut Machine,
    _: &mut Option<&mut Ctx<'_>>,
    _: Option<TyId>,
    a: Args<'_>,
) -> R<Option<u64>> {
    let (n, o) = (arg(a, 1)? as i64, origin(arg(a, 2)?)?);
    text(m, arg(a, 0)?, |f| f.truncate(n, o))?;
    Ok(None)
}

fn file_rewind(
    m: &mut Machine,
    _: &mut Option<&mut Ctx<'_>>,
    _: Option<TyId>,
    a: Args<'_>,
) -> R<Option<u64>> {
    text(m, arg(a, 0)?, |f| f.rewind())?;
    Ok(None)
}

fn file_canseek(
    m: &mut Machine,
    _: &mut Option<&mut Ctx<'_>>,
    _: Option<TyId>,
    a: Args<'_>,
) -> R<Option<u64>> {
    let cell = arg(a, 0)?;
    // A closed file cannot seek.
    let b = m.file_state_of(cell)?
        && m.with_file(cell, |s| Ok(matches!(s, FileSlot::Text(t) if t.can_seek())))?;
    Ok(Some(u64::from(b)))
}

fn deallocate(
    m: &mut Machine,
    _: &mut Option<&mut Ctx<'_>>,
    _: Option<TyId>,
    a: Args<'_>,
) -> R<Option<u64>> {
    let cell = arg(a, 0)?;
    let h = m.load(cell, Width::B8)?;
    m.free_handle(h)?;
    m.store(cell, Width::B8, 0)?;
    Ok(None)
}

fn readline(
    m: &mut Machine,
    _: &mut Option<&mut Ctx<'_>>,
    _: Option<TyId>,
    a: Args<'_>,
) -> R<Option<u64>> {
    let l = text(m, arg(a, 0)?, |f| f.readline())?;
    m.line_set(arg(a, 1)?, &l)?;
    Ok(None)
}

fn writeline(
    m: &mut Machine,
    ctx: &mut Option<&mut Ctx<'_>>,
    _: Option<TyId>,
    a: Args<'_>,
) -> R<Option<u64>> {
    let cell = arg(a, 1)?;
    let l = m.line_get(cell)?.unwrap_or_default();
    m.writeline_to(ctx, arg(a, 0)?, &l)?;
    m.line_set(cell, b"")?;
    Ok(None)
}

fn tee(
    m: &mut Machine,
    ctx: &mut Option<&mut Ctx<'_>>,
    _: Option<TyId>,
    a: Args<'_>,
) -> R<Option<u64>> {
    let cell = arg(a, 1)?;
    let mut l = m.line_get(cell)?.unwrap_or_default();
    m.writeline_to(ctx, arg(a, 0)?, &l)?;
    l.push(b'\n');
    emit(ctx, &l)?;
    m.line_set(cell, b"")?;
    Ok(None)
}

fn justify(
    m: &mut Machine,
    _: &mut Option<&mut Ctx<'_>>,
    _: Option<TyId>,
    a: Args<'_>,
) -> R<Option<u64>> {
    let v = m.fat_bytes(arg(a, 0)?)?;
    let s = tio::justify(&v, side(arg(a, 1)?)?, width(arg(a, 2)?)?);
    m.ret_string(&s).map(Some)
}

/// READ: parse from a copy of the line; on success store the value and the rest of the line.
/// The GOOD form (one more argument) reports failure in GOOD, the other form is an error.
fn read_with(
    m: &mut Machine,
    a: Args<'_>,
    f: impl FnOnce(&mut Machine, &mut Vec<u8>, u64) -> R<Result<(), yavs_std::error::ConvError>>,
) -> R<Option<u64>> {
    let (cell, dst) = (arg(a, 0)?, arg(a, 1)?);
    let good = a.get(2).copied();
    let mut line = m.line_get(cell)?.unwrap_or_default();
    let ok = match f(m, &mut line, dst)? {
        Ok(()) => {
            m.line_set(cell, &line)?;
            true
        }
        Err(e) if good.is_some() => {
            let _ = e;
            false
        }
        Err(e) => return rt_err(format!("textio read: {e}")),
    };
    if let Some(g) = good {
        m.store(g, Width::B1, ok.into())?;
    }
    Ok(None)
}

fn read_scalar(
    m: &mut Machine,
    a: Args<'_>,
    w: Width,
    f: impl FnOnce(&mut Vec<u8>, yavs_std::types::Revision) -> Result<u64, yavs_std::error::ConvError>,
) -> R<Option<u64>> {
    let rev = m.rt.revision;
    read_with(m, a, |m, l, dst| match f(l, rev) {
        Ok(v) => m.store(dst, w, v).map(Ok),
        Err(e) => Ok(Err(e)),
    })
}

fn read_bits(
    m: &mut Machine,
    a: Args<'_>,
    f: fn(&mut Vec<u8>, usize) -> Result<Vec<bool>, yavs_std::error::ConvError>,
) -> R<Option<u64>> {
    read_with(m, a, |m, l, fat| {
        let (data, n) = m.fat1(fat)?;
        match f(l, n as usize) {
            Ok(b) => m.put_bits(data, &b).map(Ok),
            Err(e) => Ok(Err(e)),
        }
    })
}

macro_rules! natives {
    ($($name:ident($m:ident, $ctx:ident, $a:ident) $body:block)*) => {$(
        fn $name($m: &mut Machine, $ctx: &mut Option<&mut Ctx<'_>>, _: Option<TyId>, $a: Args<'_>) -> R<Option<u64>> {
            let _ = &$ctx;
            $body
        }
    )*};
}

/// WRITE: append to the line (null is empty) and replace it.
fn write_with(
    m: &mut Machine,
    a: Args<'_>,
    f: impl FnOnce(&mut Machine, &mut Vec<u8>) -> R<()>,
) -> R<Option<u64>> {
    let cell = arg(a, 0)?;
    let mut line = m.line_get(cell)?.unwrap_or_default();
    f(m, &mut line)?;
    m.line_set(cell, &line)?;
    Ok(None)
}

fn conv<T>(r: Result<T, yavs_std::error::ConvError>) -> R<T> {
    r.map_err(|e| Fault::Runtime(format!("textio write: {e}")))
}

natives! {
    read_bit(m, ctx, a) { read_scalar(m, a, Width::B1, |l, _| tio::read_bit(l).map(u64::from)) }
    read_boolean(m, ctx, a) { read_scalar(m, a, Width::B1, |l, _| tio::read_boolean(l).map(u64::from)) }
    read_character(m, ctx, a) { read_scalar(m, a, Width::B1, |l, _| tio::read_character(l).map(u64::from)) }
    read_integer(m, ctx, a) {
        read_scalar(m, a, Width::B8, |l, rev| tio::read_integer(l, rev.integer_bounds()).map(|v| v as u64))
    }
    read_real(m, ctx, a) { read_scalar(m, a, Width::B8, |l, _| tio::read_real(l, RealBounds::REAL).map(f64::to_bits)) }
    read_time(m, ctx, a) {
        read_scalar(m, a, Width::B8, |l, _| tio::read_physical(l, &time_type(), Bounds::I64).map(|v| v as u64))
    }
    read_string(m, ctx, a) {
        read_with(m, a, |m, l, fat| {
            let (data, n) = m.fat1(fat)?;
            match tio::read_string(l, n as usize) {
                Ok(b) => m.put(data, &b).map(Ok),
                Err(e) => Ok(Err(e)),
            }
        })
    }
    read_bit_vector(m, ctx, a) { read_bits(m, a, tio::read_bit_vector) }
    hread(m, ctx, a) { read_bits(m, a, tio::hread) }
    oread(m, ctx, a) { read_bits(m, a, tio::oread) }
    sread(m, ctx, a) {
        let cell = arg(a, 0)?;
        let (data, n) = m.fat1(arg(a, 1)?)?;
        let mut line = m.line_get(cell)?.unwrap_or_default();
        let s = tio::sread(&mut line, n as usize);
        m.put(data, &s)?;
        m.store(arg(a, 2)?, Width::B8, s.len() as u64)?;
        m.line_set(cell, &line)?;
        Ok(None)
    }
    write_bit(m, ctx, a) {
        let (v, sd, w) = (arg(a, 1)? != 0, side(arg(a, 2)?)?, width(arg(a, 3)?)?);
        write_with(m, a, |_, l| { tio::write_bit(l, v, sd, w); Ok(()) })
    }
    write_boolean(m, ctx, a) {
        let (v, sd, w) = (arg(a, 1)? != 0, side(arg(a, 2)?)?, width(arg(a, 3)?)?);
        write_with(m, a, |_, l| { tio::write_boolean(l, v, sd, w); Ok(()) })
    }
    write_character(m, ctx, a) {
        let (v, sd, w) = (arg(a, 1)? as u8, side(arg(a, 2)?)?, width(arg(a, 3)?)?);
        write_with(m, a, |_, l| { tio::write_character(l, v, sd, w); Ok(()) })
    }
    write_integer(m, ctx, a) {
        let (v, sd, w) = (arg(a, 1)? as i64, side(arg(a, 2)?)?, width(arg(a, 3)?)?);
        write_with(m, a, |_, l| { tio::write_integer(l, v, sd, w); Ok(()) })
    }
    write_string(m, ctx, a) {
        let (v, sd, w) = (m.fat_bytes(arg(a, 1)?)?, side(arg(a, 2)?)?, width(arg(a, 3)?)?);
        write_with(m, a, |_, l| { tio::write_string(l, &v, sd, w); Ok(()) })
    }
    write_bit_vector(m, ctx, a) { write_bits(m, a, tio::write_bit_vector) }
    hwrite(m, ctx, a) { write_bits(m, a, tio::hwrite) }
    owrite(m, ctx, a) { write_bits(m, a, tio::owrite) }
    write_real(m, ctx, a) {
        let x = f64::from_bits(arg(a, 1)?);
        let (sd, w, d) = (side(arg(a, 2)?)?, width(arg(a, 3)?)?, width(arg(a, 4)?)?);
        write_with(m, a, |_, l| conv(tio::write_real(l, x, sd, w, d)))
    }
    write_real_format(m, ctx, a) {
        let x = f64::from_bits(arg(a, 1)?);
        let f = m.fat_bytes(arg(a, 2)?)?;
        write_with(m, a, |_, l| conv(tio::write_real_format(l, x, &f)))
    }
    write_time(m, ctx, a) {
        let v = arg(a, 1)? as i64;
        let (sd, w, unit) = (side(arg(a, 2)?)?, width(arg(a, 3)?)?, arg(a, 4)? as i64);
        write_with(m, a, |_, l| conv(tio::write_physical(l, v, &time_type(), sd, w, unit)))
    }
}

fn write_bits(
    m: &mut Machine,
    a: Args<'_>,
    f: fn(&mut Vec<u8>, &[bool], Side, usize),
) -> R<Option<u64>> {
    let (data, n) = m.fat1(arg(a, 1)?)?;
    let bits = m.bits_at(data, n)?;
    let (sd, w) = (side(arg(a, 2)?)?, width(arg(a, 3)?)?);
    write_with(m, a, |_, l| {
        f(l, &bits, sd, w);
        Ok(())
    })
}
