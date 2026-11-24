//! The scopes of `--two-state[=PATH]` and `--cycle[=PATH]` (A64): instance paths resolved on the
//! elaborated hierarchy, or the design under test found by [`crate::ElabConfig::detect`].

use yavs_diag::Diagnostic;

use crate::{Case, Hierarchy, ScopeId, ScopeKind, ScopeReq, codes};

/// The scopes a path can name: those of the design hierarchy (a labelled process too), the
/// package roots left out.
fn named(h: &Hierarchy, s: ScopeId) -> bool {
    let mut cur = Some(s);
    while let Some(c) = cur {
        let sc = h.scope(c);
        match sc.kind {
            ScopeKind::Library | ScopeKind::Package { .. } => return false,
            ScopeKind::Component { .. } if c == s => return false,
            _ => {}
        }
        cur = sc.parent;
    }
    true
}

/// Whether `path` (lower case, dotted) names scope text `have`: segment by segment, a segment
/// without an index matching every iteration of a generate.
fn matches(path: &str, have: &str) -> bool {
    let (mut p, mut h) = (path.split('.'), have.split('.'));
    loop {
        match (p.next(), h.next()) {
            (None, None) => return true,
            (Some(a), Some(b)) => {
                if a != b && (a.contains('(') || b.split_once('(').is_none_or(|(n, _)| n != a)) {
                    return false;
                }
            }
            _ => return false,
        }
    }
}

/// The roots `paths` name, a component instance from its component (its ports are in).
pub(crate) fn resolve(
    h: &Hierarchy,
    flag: &str,
    paths: &[String],
) -> Result<Vec<ScopeId>, Box<Diagnostic>> {
    let texts: Vec<(ScopeId, String)> = (0..h.scopes.len())
        .map(|i| ScopeId(i as u32))
        .filter(|&s| named(h, s))
        .map(|s| (s, h.dotted_in(s, Case::Lrm)))
        .collect();
    let mut roots = Vec::new();
    for p in paths {
        let key = p.trim().to_lowercase();
        let found: Vec<ScopeId> = (texts.iter())
            .filter(|(_, t)| matches(&key, t))
            .map(|&(s, _)| match h.scope(s).parent {
                Some(c) if matches!(h.scope(c).kind, ScopeKind::Component { .. }) => c,
                _ => s,
            })
            .collect();
        if found.is_empty() {
            return Err(Box::new(unknown(h, flag, p, &key, &texts)));
        }
        roots.extend(found);
    }
    roots.sort();
    roots.dedup();
    Ok(roots)
}

fn unknown(
    h: &Hierarchy,
    flag: &str,
    p: &str,
    key: &str,
    texts: &[(ScopeId, String)],
) -> Diagnostic {
    let top = h.dotted_in(ScopeId(0), Case::Lrm);
    let mut d = Diagnostic::error(
        codes::SCOPE_PATH,
        format!("`{flag}={p}` names no instance, block, generate or process of the design"),
    );
    let full = format!("{top}.{key}");
    let help = if texts.iter().any(|(_, t)| matches(&full, t)) {
        Some(format!(
            "a path starts at the top-level unit: `{flag}={top}.{p}`"
        ))
    } else {
        // Every iteration's text and the generate's own name are candidates.
        let mut cands: Vec<String> = texts.iter().map(|(_, t)| t.clone()).collect();
        cands.extend(texts.iter().filter_map(|(_, t)| {
            let stripped: Vec<&str> = t
                .split('.')
                .map(|s| s.split_once('(').map_or(s, |x| x.0))
                .collect();
            Some(stripped.join(".")).filter(|x| x != t)
        }));
        cands.sort();
        cands.dedup();
        yavs_diag::suggest::suggest(key, "path", cands.iter().map(|c| (c.as_str(), c.as_str())))
    };
    d = d.with_note(format!(
        "a path names a part of the hierarchy from the top-level unit down, such as `{top}.dut`; \
         `{top}.g` names every iteration of generate `g`, `{top}.g(1)` one"
    ));
    match help {
        Some(help) => d.with_help(help),
        None => d.with_help("`yavs elab --print-hierarchy TOP` lists the paths of the design"),
    }
}

/// Per scope, whether it is at or below one of `roots`.
pub(crate) fn mark(h: &Hierarchy, roots: &[ScopeId]) -> Vec<bool> {
    let mut v = vec![false; h.scopes.len()];
    for (i, sc) in h.scopes.iter().enumerate() {
        // A parent is made before its children.
        v[i] = roots.contains(&ScopeId(i as u32)) || sc.parent.is_some_and(|p| v[p.index()]);
    }
    v
}

/// `--cycle` and `--two-state` scopes that overlap in part: the region would hold processes
/// of both value semantics. Only the scopes holding processes count.
pub(crate) fn overlap(
    h: &Hierarchy,
    procs: &[crate::ProcInst],
    cycle: &[bool],
    two: &[bool],
) -> Option<Diagnostic> {
    if cycle.is_empty() || two.is_empty() {
        return None;
    }
    let at = |want: bool| {
        (procs.iter())
            .map(|p| p.scope)
            .find(|s| cycle[s.index()] && two[s.index()] == want)
    };
    let (Some(out), Some(both)) = (at(false), at(true)) else {
        return None;
    };
    let name = |s: ScopeId| h.dotted(h.instance_of(s));
    Some(
        Diagnostic::error(
            codes::SCOPE_OVERLAP,
            "the cycle scope and the `--two-state` scope overlap in part",
        )
        .with_note(format!(
            "`{}` is in both, `{}` only in the cycle scope: the cycle region computes with one kind \
             of values, so its scope must lie inside the `--two-state` scope or outside it",
            name(both),
            name(out)
        ))
        .with_help(
            "give `--two-state` the cycle scope's paths (or more), or keep the two scopes apart",
        ),
    )
}

/// The scope a request names.
pub(crate) fn roots_of(
    h: &Hierarchy,
    flag: &str,
    req: &ScopeReq,
    auto: &Option<Vec<ScopeId>>,
) -> Result<Vec<ScopeId>, Box<Diagnostic>> {
    match req {
        ScopeReq::Auto => Ok(auto.clone().unwrap_or_default()),
        ScopeReq::Paths(p) => resolve(h, flag, p),
        ScopeReq::Whole => Ok(Vec::new()),
    }
}

impl crate::Elaborated {
    /// The paths of scope roots, as a path names them (a component by its instance).
    pub fn scope_paths(&self, roots: &[ScopeId]) -> Vec<String> {
        let h = &self.hier;
        (roots.iter())
            .map(|&s| match h.scope(s).kind {
                ScopeKind::Component { .. } => h.scope(s).children.first().copied().unwrap_or(s),
                _ => s,
            })
            .map(|s| h.dotted(s))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::matches;

    #[test]
    fn a_path_without_an_index_names_every_iteration() {
        assert!(matches("tb.g", "tb.g(1)"));
        assert!(matches("tb.g.u", "tb.g(3).u"));
        assert!(matches("tb.g(1)", "tb.g(1)"));
        assert!(!matches("tb.g(1)", "tb.g(2)"));
        assert!(!matches("tb.g", "tb.gx(1)"));
        assert!(!matches("tb.g", "tb.g(1).u"));
        assert!(!matches("tb", "tb.dut"));
        assert!(matches("tb.dut", "tb.dut"));
    }
}
