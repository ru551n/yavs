/// Include/exclude selection by hierarchical path (`top.u1.sig`) and the array size cap.
///
/// Patterns are matched case-insensitively (VHDL identifiers fold case; the recorded spelling is
/// untouched). `*` matches within one path level, `**` across levels, `?` one character. A pattern
/// that matches a scope selects or excludes everything below it. An empty include list selects all.
#[derive(Clone, Debug)]
pub struct Selection {
    pub include: Vec<String>,
    pub exclude: Vec<String>,
    /// Arrays (and memories) with more elements than this are not recorded.
    pub max_array_elems: usize,
}

impl Default for Selection {
    fn default() -> Self {
        Self {
            include: Vec::new(),
            exclude: Vec::new(),
            max_array_elems: 65_536,
        }
    }
}

impl Selection {
    pub(crate) fn included(&self, path: &str) -> bool {
        self.include.is_empty() || any_match(&self.include, path)
    }

    pub(crate) fn excluded(&self, path: &str) -> bool {
        any_match(&self.exclude, path)
    }
}

/// True if a pattern matches `path` or any ancestor of it.
fn any_match(patterns: &[String], path: &str) -> bool {
    patterns.iter().any(|p| {
        let mut end = 0;
        loop {
            end = path[end..].find('.').map_or(path.len(), |i| end + i);
            if glob(p.as_bytes(), &path.as_bytes()[..end]) {
                return true;
            }
            if end == path.len() {
                return false;
            }
            end += 1;
        }
    })
}

fn glob(p: &[u8], s: &[u8]) -> bool {
    match p.split_first() {
        None => s.is_empty(),
        Some((b'*', rest)) if rest.first() == Some(&b'*') => {
            (0..=s.len()).any(|i| glob(&rest[1..], &s[i..]))
        }
        Some((b'*', rest)) => {
            for i in 0..=s.len() {
                if glob(rest, &s[i..]) {
                    return true;
                }
                if s.get(i) == Some(&b'.') || i == s.len() {
                    break;
                }
            }
            false
        }
        Some((b'?', rest)) => s.first().is_some_and(|c| *c != b'.') && glob(rest, &s[1..]),
        Some((c, rest)) => {
            s.first().is_some_and(|x| x.eq_ignore_ascii_case(c)) && glob(rest, &s[1..])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sel(inc: &[&str], exc: &[&str]) -> Selection {
        Selection {
            include: inc.iter().map(|s| s.to_string()).collect(),
            exclude: exc.iter().map(|s| s.to_string()).collect(),
            ..Selection::default()
        }
    }

    #[test]
    fn globs() {
        let s = sel(&["top.u1"], &["top.*.dbg*", "**.clk"]);
        assert!(s.included("top.u1.data"));
        assert!(!s.included("top.u2.data"));
        assert!(s.excluded("top.u1.dbg_x"));
        assert!(s.excluded("TOP.u9.deep.clk"));
        assert!(!s.excluded("top.u1.data"));
        assert!(sel(&[], &[]).included("anything"));
        assert!(
            sel(&[], &["top.*"]).excluded("top.a.b"),
            "scope match excludes below"
        );
    }
}
