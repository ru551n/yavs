//! `docs/environment.rst` lists every `YAVS_` environment variable the sources name: a variable
//! that is added without a row, or a row whose variable is gone, fails here.

use std::collections::BTreeSet;
use std::path::Path;

/// Names that only test that a variable is unset: nothing reads them for a purpose.
const NOT_VARIABLES: [&str; 2] = [
    "YAVS_STD_SURELY_UNSET_VARIABLE",
    "YAVS_TEXTIO_TEST_UNSET_VARIABLE",
];

/// The string literals `"YAVS_..."` and `"VUNIT_YAVS_..."` of one source text.
fn quoted(text: &str, found: &mut BTreeSet<String>) {
    for (i, _) in text.match_indices('"') {
        let name: String = text[i + 1..]
            .chars()
            .take_while(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || *c == '_')
            .collect();
        if (name.starts_with("YAVS_") || name.starts_with("VUNIT_YAVS_"))
            && !name.ends_with('_')
            && text[i + 1 + name.len()..].starts_with('"')
        {
            found.insert(name);
        }
    }
}

fn scan(dir: &Path, found: &mut BTreeSet<String>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            scan(&path, found);
        } else if path.extension().is_some_and(|x| x == "rs" || x == "py") {
            // A file that is not text (none today) cannot name a variable.
            if let Ok(text) = std::fs::read_to_string(&path) {
                quoted(&text, found);
            }
        }
    }
}

/// The first cell of each table row of the page that names such a variable.
fn listed(page: &str) -> BTreeSet<String> {
    page.lines()
        .filter_map(|l| l.trim_start().strip_prefix("* - ``"))
        .filter_map(|l| l.split("``").next())
        .filter(|n| n.starts_with("YAVS_") || n.starts_with("VUNIT_YAVS_"))
        .map(str::to_string)
        .collect()
}

#[test]
fn every_variable_is_on_the_page_and_every_row_is_read() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut found = BTreeSet::new();
    scan(&root.join("crates"), &mut found);
    scan(&root.join("python/vunit-yavs"), &mut found);
    assert!(found.len() > 50, "the scan found only {}", found.len());
    for name in NOT_VARIABLES {
        found.remove(name);
    }
    let page = std::fs::read_to_string(root.join("docs/environment.rst")).unwrap();
    let rows = listed(&page);
    let missing: Vec<_> = found.difference(&rows).collect();
    assert!(
        missing.is_empty(),
        "these environment variables are read in crates/ or python/vunit-yavs but have no row in \
         docs/environment.rst: {missing:?}\nAdd a row: under \"For users\" when a user needs it, \
         else under \"For development and diagnostics\"."
    );
    let stale: Vec<_> = rows.difference(&found).collect();
    assert!(
        stale.is_empty(),
        "docs/environment.rst lists variables nothing reads any more: {stale:?}\nRemove their rows."
    );
}
