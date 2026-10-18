//! "Did you mean" suggestions (sem.md 8.3).

/// Optimal string alignment distance (Damerau-Levenshtein with adjacent transpositions).
pub fn distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut d = vec![vec![0usize; b.len() + 1]; a.len() + 1];
    for (i, row) in d.iter_mut().enumerate() {
        row[0] = i;
    }
    d[0] = (0..=b.len()).collect();
    for i in 1..=a.len() {
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            let mut v = (d[i - 1][j] + 1)
                .min(d[i][j - 1] + 1)
                .min(d[i - 1][j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                v = v.min(d[i - 2][j - 2] + 1);
            }
            d[i][j] = v;
        }
    }
    d[a.len()][b.len()]
}

/// The closest candidate within `max(1, len/3)`, if any.
pub fn closest<'a>(key: &str, candidates: impl Iterator<Item = &'a str>) -> Option<&'a str> {
    let limit = (key.chars().count() / 3).max(1);
    candidates
        .filter(|c| *c != key)
        .map(|c| (distance(key, c), c))
        .filter(|(d, _)| *d <= limit)
        .min()
        .map(|(_, c)| c)
}

#[cfg(test)]
mod tests {
    #[test]
    fn closest_finds_typos() {
        assert_eq!(super::distance("ab", "ba"), 1);
        let c = ["numeric_std", "std_logic_1164"];
        assert_eq!(
            super::closest("numeric_sdt", c.into_iter()),
            Some("numeric_std")
        );
        assert_eq!(super::closest("xyz", c.into_iter()), None);
    }
}
