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
//! the same delta sees what it wrote. Every caller of a file native is serial (yavs-ir
//! `TOUCHES_SHARED`), so the bytes on disk do not depend on `--threads` even when file objects
//! of different processes name the same external file.
//! `STD_OUTPUT` goes through the kernel's OUTPUT stream, ordered with `report`.

use std::io::Write;

use yavs_ir::{TyId, Width};
use yavs_kernel::Ctx;
use yavs_std::file::{
    FileError, FileOpenKind, FileOpenState, FileOpenStatus, FileOrigin, STD_OUTPUT, TextFile,
};
use yavs_std::textio::{self as tio, Side};
use yavs_std::types::{Bounds, RealBounds, time_type};

use crate::exec::{Fault, Machine, R, SHARED, SHARED_HEAP, add_addr, ice, rt_err, split};
use crate::heap::{Heap, HeapError, Obj};

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
            (f.logical_name().to_vec(), kind.pos(m.rt.revision))
        }
        FileSlot::Output { open: true } => (STD_OUTPUT.to_vec(), FileOpenKind::Write.pos(m.rt.revision)),
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
        Some(c) => c.output(bytes),
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
        self.alloc_heap(matches!(split(cell).0, SHARED | SHARED_HEAP))
    }

    fn alloc_in(&mut self, cell: u64, kind: Obj, data: Vec<u8>) -> R<u64> {
        self.heap_for(cell)?
            .alloc(kind, data)
            .map_err(|_| heap_full())
    }

    /// A string object `[1 to len][bytes]` in the heap that suits `cell`.
    pub(crate) fn new_line(&mut self, cell: u64, s: &[u8]) -> R<u64> {
        let mut obj = line_desc(s.len()).to_vec();
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
        Ok(self.line_ref(cell)?.map(<[u8]>::to_vec))
    }

    /// [`Machine::line_get`] without the copy.
    fn line_ref(&mut self, cell: u64) -> R<Option<&[u8]>> {
        let h = self.load(cell, Width::B8)?;
        if h == 0 {
            return Ok(None);
        }
        let (hp, _) = self.heap_of(h)?;
        let i = hp.resolve(h).map_err(|_| deallocated_line())?;
        let d = hp.data(i).ok_or_else(|| ice("heap object vanished"))?;
        let n = line_len(d)?;
        d.get(24..24 + n).map(Some).ok_or_else(line_short)
    }

    /// Replace the line at `cell` (null is empty) by a new object `1 to len` holding its
    /// bytes without the first `drop` and with `tail` appended, deallocating the old one: as
    /// [`Machine::line_get`] then [`Machine::line_set`], but the bytes move to the new object
    /// instead of being copied, so N READs or WRITEs on one line are O(N) (#196).
    fn line_edit(&mut self, cell: u64, drop: usize, tail: &[u8]) -> R<()> {
        let h = self.load(cell, Width::B8)?;
        let (mut v, mut start) = if h == 0 {
            (line_desc(0).to_vec(), 0)
        } else {
            self.heap_of(h)?.0.take(h).map_err(|e| match e {
                HeapError::Stale => deallocated_line(),
                e => ice(format!("line: {e:?}")),
            })?
        };
        let n = line_len(v.get(start..).unwrap_or_default())?;
        if drop > n || v.len() < start + 24 + n {
            return Err(line_short());
        }
        v.truncate(start + 24 + n);
        start += drop;
        v.extend_from_slice(tail);
        // Compact once the dropped prefix is the larger half: amortized O(1) per byte.
        if start > v.len() - start {
            v.drain(..start);
            start = 0;
        }
        let n = v.len() - start - 24;
        v[start..start + 24].copy_from_slice(&line_desc(n));
        let nh = self
            .heap_for(cell)?
            .alloc_from(Obj::Bytes, v, start)
            .map_err(|_| heap_full())?;
        self.store(cell, Width::B8, nh)
    }

    /// Replace the line at `cell` by a new object holding `s`, deallocating the old one.
    pub(crate) fn line_set(&mut self, cell: u64, s: &[u8]) -> R<()> {
        let old = self.load(cell, Width::B8)?;
        self.free_handle(old)?;
        let h = self.new_line(cell, s)?;
        self.store(cell, Width::B8, h)
    }

    // ---- files ----

    pub(crate) fn with_file<T>(
        &mut self,
        cell: u64,
        f: impl FnOnce(&mut FileSlot) -> R<T>,
    ) -> R<T> {
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
        let kind = FileOpenKind::from_pos(kind, self.rt.revision)
            .ok_or_else(|| ice(format!("FILE_OPEN_KIND position {kind}")))?;
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
        (_, Some("read")) => crate::binfile::read,
        (_, Some("write")) => crate::binfile::write,
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

pub(crate) fn text<T>(
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
    Ok(Some(k.pos(m.rt.revision)))
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

/// Parse from the front of the line at `cell` (null is empty) without copying it; the parsed
/// value and the number of bytes consumed.
fn parse_line<T>(m: &mut Machine, cell: u64, parse: impl FnOnce(&mut &[u8]) -> T) -> R<(T, usize)> {
    let line = m.line_ref(cell)?.unwrap_or_default();
    let mut rest = line;
    let v = parse(&mut rest);
    Ok((v, line.len() - rest.len()))
}

/// READ: parse from the line; on success store the value and drop what was read from the line.
/// The GOOD form (one more argument) reports failure in GOOD, the other form is an error.
fn read_with<T>(
    m: &mut Machine,
    a: Args<'_>,
    parse: impl FnOnce(&mut &[u8]) -> Result<T, yavs_std::error::ConvError>,
    store: impl FnOnce(&mut Machine, T) -> R<()>,
) -> R<Option<u64>> {
    let cell = arg(a, 0)?;
    let good = a.get(2).copied();
    let (v, k) = parse_line(m, cell, parse)?;
    let ok = match v {
        Ok(v) => {
            store(m, v)?;
            m.line_edit(cell, k, b"")?;
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
    f: impl FnOnce(&mut &[u8], yavs_std::types::Revision) -> Result<u64, yavs_std::error::ConvError>,
) -> R<Option<u64>> {
    let (rev, dst) = (m.rt.revision, arg(a, 1)?);
    read_with(m, a, |l| f(l, rev), |m, v| m.store(dst, w, v))
}

fn read_bits(
    m: &mut Machine,
    a: Args<'_>,
    f: fn(&mut &[u8], usize) -> Result<Vec<bool>, yavs_std::error::ConvError>,
) -> R<Option<u64>> {
    let (data, n) = m.fat1(arg(a, 1)?)?;
    read_with(m, a, |l| f(l, n as usize), |m, b| m.put_bits(data, &b))
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
    // A deallocated line is an error before anything is formatted.
    m.line_ref(cell)?;
    let mut tail = Vec::new();
    f(m, &mut tail)?;
    m.line_edit(cell, 0, &tail)?;
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
        let (data, n) = m.fat1(arg(a, 1)?)?;
        read_with(m, a, |l| tio::read_string(l, n as usize), |m, b| m.put(data, &b))
    }
    read_bit_vector(m, ctx, a) { read_bits(m, a, tio::read_bit_vector) }
    hread(m, ctx, a) { read_bits(m, a, tio::hread) }
    oread(m, ctx, a) { read_bits(m, a, tio::oread) }
    sread(m, ctx, a) {
        let cell = arg(a, 0)?;
        let (data, n) = m.fat1(arg(a, 1)?)?;
        let (s, k) = parse_line(m, cell, |l| tio::sread(l, n as usize))?;
        m.put(data, &s)?;
        m.store(arg(a, 2)?, Width::B8, s.len() as u64)?;
        m.line_edit(cell, k, b"")?;
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

fn deallocated_line() -> Fault {
    Fault::Runtime("line designates an object that was deallocated".into())
}

fn line_short() -> Fault {
    ice("line object shorter than its descriptor")
}

fn heap_full() -> Fault {
    Fault::Runtime("heap object limit (16M objects per heap) exceeded".into())
}

/// The descriptor `1 to n` of a line object.
fn line_desc(n: usize) -> [u8; 24] {
    let mut d = [0; 24];
    d[..8].copy_from_slice(&1i64.to_le_bytes());
    d[8..16].copy_from_slice(&(n as i64).to_le_bytes());
    d
}

/// The length of a line object `[descriptor][bytes]` from its descriptor.
fn line_len(d: &[u8]) -> R<usize> {
    let Some(w) = d.get(..24).map(|h| h.as_chunks::<8>().0) else {
        return Err(ice("line object without descriptor"));
    };
    let w = |k: usize| i64::from_le_bytes(w[k]);
    yavs_ir::dim_len(w(0), w(1), w(2) == 1)
        .and_then(|n| usize::try_from(n).ok())
        .ok_or_else(|| ice("line length"))
}
