//! TEXT files: FILE_OPEN, READLINE, WRITELINE, TEE, ENDFILE, FLUSH and
//! the VHDL-2019 file operations (FILE_STATE, FILE_MODE, FILE_POSITION,
//! FILE_SIZE, FILE_REWIND, FILE_SEEK, FILE_TRUNCATE, FILE_CANSEEK).
//!
//! Line ends: READLINE splits at LF and drops the CR characters directly
//! before it (CRLF and stray CRs at the end of a line), and the last line
//! may lack an LF. WRITELINE always writes LF, on every platform.

use crate::types::Revision;
use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::{self, BufRead, BufReader, BufWriter, Read, Seek, SeekFrom, Write};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileOpenKind {
    Read,
    Write,
    Append,
    /// VHDL-2019.
    ReadWrite,
}

impl FileOpenKind {
    /// The kind at position `pos` of FILE_OPEN_KIND: READ, WRITE, APPEND in 2008; READ,
    /// READ_WRITE, WRITE, APPEND in 2019 (LRM 2019 5.5.2).
    pub fn from_pos(pos: u64, rev: Revision) -> Option<Self> {
        use FileOpenKind::*;
        let table = match rev {
            Revision::Vhdl1993 | Revision::Vhdl2008 => &[Read, Write, Append][..],
            Revision::Vhdl2019 => &[Read, ReadWrite, Write, Append][..],
        };
        table.get(pos as usize).copied()
    }

    /// The inverse of `from_pos` (READ_WRITE exists only in 2019).
    pub fn pos(self, rev: Revision) -> u64 {
        use FileOpenKind::*;
        match (rev, self) {
            (_, Read) => 0,
            (Revision::Vhdl1993 | Revision::Vhdl2008, Write) => 1,
            (Revision::Vhdl1993 | Revision::Vhdl2008, Append | ReadWrite) => 2,
            (Revision::Vhdl2019, ReadWrite) => 1,
            (Revision::Vhdl2019, Write) => 2,
            (Revision::Vhdl2019, Append) => 3,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileOpenStatus {
    Ok,
    StatusError,
    NameError,
    ModeError,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileOpenState {
    Open,
    Closed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileOrigin {
    Begin,
    Current,
    End,
}

#[derive(Debug)]
pub enum FileError {
    /// The file object is not open.
    NotOpen,
    /// The operation needs another open kind, for example READLINE on a
    /// file opened for writing.
    WrongMode {
        op: &'static str,
        kind: FileOpenKind,
    },
    /// READLINE at end of file.
    EndOfFile,
    /// Seek or size on a stream such as STD_INPUT.
    NotSeekable {
        op: &'static str,
    },
    /// A seek before the beginning of the file.
    BadPosition {
        position: i128,
    },
    Io(io::Error),
}

impl fmt::Display for FileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FileError::NotOpen => write!(f, "file is not open"),
            FileError::WrongMode { op, kind } => {
                write!(f, "{op} is not allowed on a file opened with {kind:?} mode")
            }
            FileError::EndOfFile => write!(f, "read past the end of the file"),
            FileError::NotSeekable { op } => write!(f, "{op} needs a seekable file"),
            FileError::BadPosition { position } => {
                write!(
                    f,
                    "file position {position} is before the start of the file"
                )
            }
            FileError::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for FileError {}

impl From<io::Error> for FileError {
    fn from(e: io::Error) -> Self {
        FileError::Io(e)
    }
}

type FResult<T> = Result<T, FileError>;

enum Inner {
    /// READ_MODE and READ_WRITE_MODE on a disk file. Writes in
    /// READ_WRITE_MODE go to the underlying file at the logical position.
    Disk(BufReader<File>),
    /// WRITE_MODE and APPEND_MODE on a disk file.
    DiskOut(BufWriter<File>),
    /// A read stream (STD_INPUT, or one the runtime provides).
    Source(BufReader<Box<dyn Read + Send>>),
    /// A write stream (STD_OUTPUT, or one the runtime provides).
    Sink(Box<dyn Write + Send>),
}

/// An open (or closed) VHDL file object of type TEXT.
pub struct TextFile {
    kind: FileOpenKind,
    inner: Option<Inner>,
    /// The external name it was opened with (`file_value_mirror.get_file_logical_name`).
    name: Vec<u8>,
}

impl fmt::Debug for TextFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TextFile")
            .field("kind", &self.kind)
            .field("open", &self.inner.is_some())
            .finish()
    }
}

/// The external names that denote the standard streams (LRM 16.4).
pub const STD_INPUT: &[u8] = b"STD_INPUT";
pub const STD_OUTPUT: &[u8] = b"STD_OUTPUT";

impl TextFile {
    /// FILE_OPEN. `name` is the external name (Latin-1). `STD_INPUT` and
    /// `STD_OUTPUT` denote the process streams. The error is the
    /// FILE_OPEN_STATUS value; FILE_OPEN without a status reports it.
    pub fn open(name: &[u8], kind: FileOpenKind) -> Result<TextFile, FileOpenStatus> {
        if name == STD_INPUT {
            return match kind {
                FileOpenKind::Read => Ok(Self::from_reader(Box::new(io::stdin())).named(name)),
                _ => Err(FileOpenStatus::ModeError),
            };
        }
        if name == STD_OUTPUT {
            return match kind {
                FileOpenKind::Write | FileOpenKind::Append => {
                    Ok(Self::from_writer(Box::new(io::stdout()), kind).named(name))
                }
                _ => Err(FileOpenStatus::ModeError),
            };
        }
        if name.is_empty() {
            return Err(FileOpenStatus::NameError);
        }
        let path = path_of(name);
        let mut o = OpenOptions::new();
        match kind {
            FileOpenKind::Read => o.read(true),
            FileOpenKind::Write => o.write(true).create(true).truncate(true),
            FileOpenKind::Append => o.append(true).create(true),
            FileOpenKind::ReadWrite => o.read(true).write(true),
        };
        let f = o.open(path).map_err(|e| match e.kind() {
            io::ErrorKind::PermissionDenied => FileOpenStatus::ModeError,
            _ => FileOpenStatus::NameError,
        })?;
        let inner = match kind {
            FileOpenKind::Read | FileOpenKind::ReadWrite => Inner::Disk(BufReader::new(f)),
            _ => Inner::DiskOut(BufWriter::new(f)),
        };
        Ok(TextFile {
            kind,
            inner: Some(inner),
            name: name.to_vec(),
        })
    }

    /// A read-mode file over any stream.
    pub fn from_reader(r: Box<dyn Read + Send>) -> TextFile {
        TextFile {
            kind: FileOpenKind::Read,
            inner: Some(Inner::Source(BufReader::new(r))),
            name: Vec::new(),
        }
    }

    /// A write-mode (or append-mode) file over any stream.
    pub fn from_writer(w: Box<dyn Write + Send>, kind: FileOpenKind) -> TextFile {
        TextFile {
            kind,
            inner: Some(Inner::Sink(w)),
            name: Vec::new(),
        }
    }

    /// The same file with external name `name`.
    pub fn named(mut self, name: &[u8]) -> TextFile {
        self.name = name.to_vec();
        self
    }

    /// The external name given to FILE_OPEN (empty for a stream made by the runtime).
    pub fn logical_name(&self) -> &[u8] {
        &self.name
    }

    /// FILE_CLOSE: flushes and closes. Closing a closed file does nothing.
    pub fn close(&mut self) -> FResult<()> {
        let r = self.flush();
        self.inner = None;
        match r {
            Err(FileError::NotOpen) => Ok(()),
            r => r,
        }
    }

    /// FILE_STATE (2019).
    pub fn state(&self) -> FileOpenState {
        if self.inner.is_some() {
            FileOpenState::Open
        } else {
            FileOpenState::Closed
        }
    }

    /// FILE_MODE (2019). Errors when the file is closed.
    pub fn mode(&self) -> FResult<FileOpenKind> {
        self.inner
            .as_ref()
            .map(|_| self.kind)
            .ok_or(FileError::NotOpen)
    }

    fn reader(&mut self, op: &'static str) -> FResult<&mut dyn BufRead> {
        let kind = self.kind;
        match self.inner.as_mut().ok_or(FileError::NotOpen)? {
            Inner::Disk(r) => Ok(r),
            Inner::Source(r) => Ok(r),
            _ => Err(FileError::WrongMode { op, kind }),
        }
    }

    fn writer(&mut self, op: &'static str) -> FResult<&mut dyn Write> {
        let kind = self.kind;
        match self.inner.as_mut().ok_or(FileError::NotOpen)? {
            Inner::DiskOut(w) => Ok(w),
            Inner::Sink(w) => Ok(w),
            Inner::Disk(r) if kind == FileOpenKind::ReadWrite => {
                // Drop the read buffer so the file position is the logical one.
                let p = r.stream_position()?;
                r.seek(SeekFrom::Start(p))?;
                Ok(r.get_mut())
            }
            _ => Err(FileError::WrongMode { op, kind }),
        }
    }

    /// ENDFILE.
    pub fn endfile(&mut self) -> FResult<bool> {
        Ok(self.reader("ENDFILE")?.fill_buf()?.is_empty())
    }

    /// READLINE: the next line without its end-of-line characters. Reading
    /// at end of file is an error.
    pub fn readline(&mut self) -> FResult<Vec<u8>> {
        let r = self.reader("READLINE")?;
        let mut line = Vec::new();
        if r.read_until(b'\n', &mut line)? == 0 {
            return Err(FileError::EndOfFile);
        }
        if line.last() == Some(&b'\n') {
            line.pop();
        }
        while line.last() == Some(&b'\r') {
            line.pop();
        }
        Ok(line)
    }

    /// WRITELINE: write the line and an LF; the line becomes empty.
    pub fn writeline(&mut self, line: &mut Vec<u8>) -> FResult<()> {
        let w = self.writer("WRITELINE")?;
        w.write_all(line)?;
        w.write_all(b"\n")?;
        line.clear();
        Ok(())
    }

    /// TEE: write the line to this file and to `output` (the runtime's
    /// OUTPUT); the line becomes empty.
    pub fn tee(&mut self, output: &mut TextFile, line: &mut Vec<u8>) -> FResult<()> {
        let mut copy = line.clone();
        self.writeline(line)?;
        output.writeline(&mut copy)
    }

    /// WRITE on a file of a non-text type: raw bytes.
    pub fn write_bytes(&mut self, b: &[u8]) -> FResult<()> {
        self.writer("WRITE")?.write_all(b)?;
        Ok(())
    }

    /// READ on a file of a non-text type: exactly `n` bytes; fewer left is an error.
    pub fn read_bytes(&mut self, n: usize) -> FResult<Vec<u8>> {
        let r = self.reader("READ")?;
        let mut v = Vec::new();
        (&mut *r).take(n as u64).read_to_end(&mut v)?;
        if v.len() < n {
            return Err(FileError::EndOfFile);
        }
        Ok(v)
    }

    /// Skip `n` bytes of a file of a non-text type; fewer left is an error.
    pub fn skip_bytes(&mut self, n: u64) -> FResult<()> {
        let r = self.reader("READ")?;
        if io::copy(&mut (&mut *r).take(n), &mut io::sink())? < n {
            return Err(FileError::EndOfFile);
        }
        Ok(())
    }

    /// FLUSH.
    pub fn flush(&mut self) -> FResult<()> {
        match self.inner.as_mut().ok_or(FileError::NotOpen)? {
            Inner::DiskOut(w) => w.flush()?,
            Inner::Sink(w) => w.flush()?,
            Inner::Disk(r) => r.get_mut().flush()?,
            Inner::Source(_) => {}
        }
        Ok(())
    }

    /// FILE_CANSEEK (2019).
    pub fn can_seek(&self) -> bool {
        matches!(self.inner, Some(Inner::Disk(_) | Inner::DiskOut(_)))
    }

    fn seekable(&mut self, op: &'static str) -> FResult<&mut dyn SeekWrite> {
        match self.inner.as_mut().ok_or(FileError::NotOpen)? {
            Inner::Disk(r) => Ok(r),
            Inner::DiskOut(w) => Ok(w),
            _ => Err(FileError::NotSeekable { op }),
        }
    }

    /// FILE_POSITION (2019), in bytes from `origin`.
    pub fn position(&mut self, origin: FileOrigin) -> FResult<i128> {
        let s = self.seekable("FILE_POSITION")?;
        let pos = s.stream_position()? as i128;
        Ok(match origin {
            FileOrigin::Begin => pos,
            FileOrigin::Current => 0,
            // The result is a NATURAL: the distance from the position to the end.
            FileOrigin::End => s.len()? as i128 - pos,
        })
    }

    /// FILE_SIZE (2019), in bytes.
    pub fn size(&mut self) -> FResult<u64> {
        self.seekable("FILE_SIZE")?.len()
    }

    /// FILE_REWIND (2019).
    pub fn rewind(&mut self) -> FResult<()> {
        self.seek(0, FileOrigin::Begin)
    }

    /// FILE_SEEK (2019). Seeking before the start is an error.
    pub fn seek(&mut self, offset: i64, origin: FileOrigin) -> FResult<()> {
        let s = self.seekable("FILE_SEEK")?;
        let base = match origin {
            FileOrigin::Begin => 0,
            FileOrigin::Current => s.stream_position()? as i128,
            FileOrigin::End => s.len()? as i128,
        };
        let target = base + offset as i128;
        let t = u64::try_from(target).map_err(|_| FileError::BadPosition { position: target })?;
        s.seek(SeekFrom::Start(t))?;
        Ok(())
    }

    /// FILE_TRUNCATE (2019): set the size relative to `origin`.
    pub fn truncate(&mut self, size: i64, origin: FileOrigin) -> FResult<()> {
        let s = self.seekable("FILE_TRUNCATE")?;
        let base = match origin {
            FileOrigin::Begin => 0,
            FileOrigin::Current => s.stream_position()? as i128,
            FileOrigin::End => s.len()? as i128,
        };
        let target = base + size as i128;
        let t = u64::try_from(target).map_err(|_| FileError::BadPosition { position: target })?;
        s.set_len(t)
    }
}

/// Seek plus access to the underlying file, for the 2019 operations.
trait SeekWrite: Seek {
    fn file(&mut self) -> FResult<&mut File>;
    fn len(&mut self) -> FResult<u64> {
        self.flush_out()?;
        Ok(self.file()?.metadata()?.len())
    }
    fn set_len(&mut self, n: u64) -> FResult<()> {
        self.flush_out()?;
        self.file()?.set_len(n)?;
        Ok(())
    }
    fn flush_out(&mut self) -> FResult<()> {
        Ok(())
    }
}

impl SeekWrite for BufReader<File> {
    fn file(&mut self) -> FResult<&mut File> {
        // Drop the read buffer so later reads see the change.
        let p = self.stream_position()?;
        self.seek(SeekFrom::Start(p))?;
        Ok(self.get_mut())
    }
}

impl SeekWrite for BufWriter<File> {
    fn file(&mut self) -> FResult<&mut File> {
        Ok(self.get_mut())
    }
    fn flush_out(&mut self) -> FResult<()> {
        self.flush()?;
        Ok(())
    }
}

/// External file names are Latin-1 bytes; on Unix they are used as raw
/// bytes, elsewhere decoded as Latin-1.
pub fn path_of(name: &[u8]) -> std::path::PathBuf {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        std::path::PathBuf::from(std::ffi::OsStr::from_bytes(name))
    }
    #[cfg(not(unix))]
    {
        std::path::PathBuf::from(crate::error::latin1(name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_kind_positions_follow_the_revision() {
        use FileOpenKind::*;
        for (rev, kinds) in [
            (Revision::Vhdl2008, &[Read, Write, Append][..]),
            (Revision::Vhdl2019, &[Read, ReadWrite, Write, Append][..]),
        ] {
            for (i, k) in kinds.iter().enumerate() {
                assert_eq!(FileOpenKind::from_pos(i as u64, rev), Some(*k));
                assert_eq!(k.pos(rev), i as u64);
            }
            assert_eq!(FileOpenKind::from_pos(kinds.len() as u64, rev), None);
        }
    }

    #[test]
    fn lines_and_seek() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let p = dir.join("t.txt");
        let name = p.to_str().unwrap().as_bytes();
        let mut f = TextFile::open(name, FileOpenKind::Write).unwrap();
        let mut l = b"ab".to_vec();
        f.writeline(&mut l).unwrap();
        assert!(l.is_empty());
        assert!(matches!(f.readline(), Err(FileError::WrongMode { .. })));
        f.close().unwrap();
        assert_eq!(f.state(), FileOpenState::Closed);
        assert!(matches!(f.writeline(&mut l), Err(FileError::NotOpen)));
        std::fs::write(&p, b"x\r\ny").unwrap();
        let mut f = TextFile::open(name, FileOpenKind::ReadWrite).unwrap();
        assert_eq!(f.readline().unwrap(), b"x");
        assert_eq!(f.position(FileOrigin::Begin).unwrap(), 3);
        assert_eq!(f.size().unwrap(), 4);
        assert_eq!(f.readline().unwrap(), b"y");
        assert!(f.endfile().unwrap());
        assert!(matches!(f.readline(), Err(FileError::EndOfFile)));
        f.rewind().unwrap();
        assert!(f.seek(-1, FileOrigin::Begin).is_err());
        f.truncate(1, FileOrigin::Begin).unwrap();
        assert_eq!(f.readline().unwrap(), b"x");
        assert!(f.endfile().unwrap());
        assert_eq!(
            TextFile::open(b"", FileOpenKind::Read).unwrap_err(),
            FileOpenStatus::NameError
        );
        assert_eq!(
            TextFile::open(
                dir.join("missing").to_str().unwrap().as_bytes(),
                FileOpenKind::Read
            )
            .unwrap_err(),
            FileOpenStatus::NameError
        );
        assert_eq!(
            TextFile::open(STD_INPUT, FileOpenKind::Write).unwrap_err(),
            FileOpenStatus::ModeError
        );
    }
}
