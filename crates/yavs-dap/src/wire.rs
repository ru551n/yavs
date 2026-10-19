//! DAP base protocol framing: `Content-Length: N\r\n\r\n` then N bytes of JSON.

use std::io::{self, BufRead, Write};

use serde_json::Value;

/// The next message; `None` at the end of input.
pub fn read_message(r: &mut impl BufRead) -> io::Result<Option<Value>> {
    let mut len = None;
    loop {
        let mut line = String::new();
        if r.read_line(&mut line)? == 0 {
            return Ok(None);
        }
        let line = line.trim_end();
        if line.is_empty() {
            if len.is_some() {
                break;
            }
            continue;
        }
        if let Some((k, v)) = line.split_once(':')
            && k.trim().eq_ignore_ascii_case("content-length")
        {
            len = Some(v.trim().parse::<usize>().map_err(|e| {
                io::Error::new(io::ErrorKind::InvalidData, format!("Content-Length: {e}"))
            })?);
        }
    }
    let mut body = vec![0; len.unwrap_or(0)];
    r.read_exact(&mut body)?;
    serde_json::from_slice(&body)
        .map(Some)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

/// Write one message and flush.
pub fn write_message(w: &mut (impl Write + ?Sized), m: &Value) -> io::Result<()> {
    let body = m.to_string();
    write!(w, "Content-Length: {}\r\n\r\n{body}", body.len())?;
    w.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let m = serde_json::json!({"seq": 1, "type": "request", "command": "threads"});
        let mut buf = Vec::new();
        write_message(&mut buf, &m).unwrap();
        write_message(&mut buf, &m).unwrap();
        let mut r = io::Cursor::new(buf);
        assert_eq!(read_message(&mut r).unwrap(), Some(m.clone()));
        assert_eq!(read_message(&mut r).unwrap(), Some(m));
        assert_eq!(read_message(&mut r).unwrap(), None);
    }
}
