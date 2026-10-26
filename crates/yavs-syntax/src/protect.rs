//! Detection of IEEE 1735 / IEEE 1364 protected envelopes (`` `protect begin_protected ``).
//!
//! Language-neutral on purpose: the VHDL lexer calls it today, a Verilog front end can reuse it.
//! yavs never decrypts an envelope; callers report it and stop reading the file.

/// The words of the directive line that starts at `at` (just after the backtick), with a
/// leading `pragma` dropped: `` `pragma protect key_block `` gives `["protect", "key_block"]`.
fn words(bytes: &[u8], at: usize) -> Vec<&[u8]> {
    let line = bytes.get(at..).unwrap_or_default();
    let line = &line[..line.iter().position(|&b| b == b'\n').unwrap_or(line.len())];
    let mut w: Vec<&[u8]> = line
        .split(|b| matches!(b, b' ' | b'\t' | b'\r' | b'='))
        .filter(|w| !w.is_empty())
        .collect();
    if w.first().is_some_and(|f| f.eq_ignore_ascii_case(b"pragma")) {
        w.remove(0);
    }
    w
}

/// Does the directive whose name starts at `at` (just after the backtick) open an encrypted
/// envelope: `protect begin_protected` or `pragma protect begin_protected`, case-insensitive?
pub fn begins_envelope(bytes: &[u8], at: usize) -> bool {
    let w = words(bytes, at);
    w.len() >= 2
        && w[0].eq_ignore_ascii_case(b"protect")
        && w[1].eq_ignore_ascii_case(b"begin_protected")
}

/// What the envelope header says: the `encrypt_agent` string and how many `key_block`s it
/// carries (one per tool vendor), read from the file after `at`, up to `end_protected`.
pub fn envelope_info(bytes: &[u8], at: usize) -> (Option<String>, usize) {
    let (mut agent, mut keys) = (None, 0);
    for line in bytes
        .get(at..)
        .unwrap_or_default()
        .split(|&b| b == b'\n')
        .skip(1)
    {
        let Some(tick) = line.iter().position(|&b| b == b'`') else {
            continue;
        };
        let w = words(line, tick + 1);
        if w.len() < 2 || !w[0].eq_ignore_ascii_case(b"protect") {
            continue;
        }
        if w[1].eq_ignore_ascii_case(b"end_protected") {
            break;
        } else if w[1].eq_ignore_ascii_case(b"key_block") {
            keys += 1;
        } else if agent.is_none() && w[1].eq_ignore_ascii_case(b"encrypt_agent") {
            agent = line
                .split(|&b| b == b'"')
                .nth(1)
                .map(|a| String::from_utf8_lossy(a).into_owned());
        }
    }
    (agent, keys)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_both_spellings() {
        for s in [
            "`protect begin_protected",
            "`PRAGMA Protect  Begin_Protected\r\n",
            "`protect\tbegin_protected x",
        ] {
            assert!(begins_envelope(s.as_bytes(), 1), "{s:?}");
        }
        for s in [
            "`protect begin",
            "`protect\nbegin_protected",
            "`pragma protect data_block",
            "`protected",
        ] {
            assert!(!begins_envelope(s.as_bytes(), 1), "{s:?}");
        }
    }

    #[test]
    fn header_info() {
        let t = b"`protect begin_protected\n`protect encrypt_agent = \"ACME\"\n`protect key_block\nAAAA\n`pragma protect key_block\nBB\n`protect end_protected\n`protect key_block\n";
        assert_eq!(envelope_info(t, 1), (Some("ACME".into()), 2));
    }
}
