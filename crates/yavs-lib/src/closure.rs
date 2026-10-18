use crate::Result;
use crate::error::err;
use crate::library::Library;
use crate::manifest::{Manifest, UnitKind, UnitRecord, UnitRef, fold, hash_content};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Library name to directory mapping (`--work NAME:PATH`, `-P`, ...). Names are folded.
#[derive(Clone, Debug, Default)]
pub struct Libraries {
    libs: Vec<Library>,
}

impl Libraries {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, name: &str, dir: impl Into<PathBuf>) {
        let name = fold(name);
        self.libs.retain(|l| l.name != name);
        self.libs.push(Library::new(&name, dir));
    }

    /// Parse `NAME:PATH` (split at the first colon, so `work:C:\x` works).
    pub fn add_spec(&mut self, spec: &str) -> Result<()> {
        match spec.split_once(':') {
            Some((n, p)) if !n.is_empty() && !p.is_empty() => {
                self.add(n, p);
                Ok(())
            }
            _ => Err(Box::new(
                err("L0020", format!("invalid library mapping `{spec}`"))
                    .with_help("expected NAME:PATH, e.g. `ieee:/opt/yavs/lib/ieee`"),
            )),
        }
    }

    pub fn get(&self, name: &str) -> Option<&Library> {
        let n = fold(name);
        self.libs.iter().find(|l| l.name == n)
    }

    /// Folded names of the mapped libraries, in mapping order.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.libs.iter().map(|l| l.name.as_str())
    }
}

/// One source file in the closure, in compile order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClosureFile {
    pub library: String,
    pub path: PathBuf,
    pub hash: String,
    /// Standard to re-parse this file with.
    pub std: String,
}

type Node = (String, UnitKind, String); // library, kind, key

struct Ctx<'a> {
    libs: &'a Libraries,
    manifests: HashMap<String, Manifest>,
}

impl Ctx<'_> {
    fn manifest(&mut self, lib: &str) -> Result<&Manifest> {
        if !self.manifests.contains_key(lib) {
            let Some(l) = self.libs.get(lib) else {
                let known: Vec<_> = self.libs.libs.iter().map(|l| l.name.as_str()).collect();
                return Err(Box::new(
                    err("L0030", format!("unknown library `{lib}`"))
                        .with_note(format!(
                            "mapped libraries: {}",
                            if known.is_empty() {
                                "none".into()
                            } else {
                                known.join(", ")
                            }
                        ))
                        .with_help(format!("map it with `{lib}:PATH`")),
                ));
            };
            let m = l.read()?;
            self.manifests.insert(lib.to_string(), m);
        }
        Ok(&self.manifests[lib])
    }

    fn unit(&mut self, r: &UnitRef, needed_by: Option<&str>) -> Result<UnitRecord> {
        let m = self.manifest(&r.library)?;
        if let Some(u) = m.find(r.kind, &r.key) {
            return Ok(u.clone());
        }
        let want = format!("{} {}", r.kind.describe(), r.key);
        let mut d = err(
            "L0031",
            format!(
                "unknown {} `{}` in library `{}`",
                r.kind.describe(),
                r.key,
                r.library
            ),
        );
        if let Some(by) = needed_by {
            d = d.with_note(format!("needed by {by}"));
        }
        let best = m
            .units()
            .map(|u| (edit_distance(&u.key, &r.key), u))
            .filter(|(dist, _)| *dist <= (r.key.len() / 3).max(2))
            .min_by_key(|(dist, u)| (*dist, u.kind != r.kind));
        d = match best {
            Some((_, u)) => {
                d.with_help(format!("did you mean {} `{}`?", u.kind.describe(), u.name))
            }
            None => d.with_help(format!(
                "library `{}` has no {want}; analyze the file that defines it",
                r.library
            )),
        };
        Err(Box::new(d))
    }

    /// Latest analyzed architecture of an entity.
    fn latest_arch(&mut self, lib: &str, entity: &str) -> Result<Option<UnitRecord>> {
        Ok(self
            .manifest(lib)?
            .units()
            .filter(|u| u.kind == UnitKind::Architecture && u.primary.as_deref() == Some(entity))
            .last()
            .cloned())
    }

    fn node(lib: &str, u: &UnitRecord) -> Node {
        (lib.to_string(), u.kind, u.key.clone())
    }
}

/// Levenshtein distance.
fn edit_distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut cur = vec![i + 1];
        for (j, cb) in b.iter().enumerate() {
            cur.push(
                (prev[j] + usize::from(ca != *cb))
                    .min(prev[j + 1] + 1)
                    .min(cur[j] + 1),
            );
        }
        prev = cur;
    }
    prev[b.len()]
}

/// Ordered list of files (dependencies first, across libraries) needed to elaborate `top`.
/// Verifies every file still has the hash recorded at analysis time.
pub fn closure(libs: &Libraries, top: &UnitRef) -> Result<Vec<ClosureFile>> {
    let mut cx = Ctx {
        libs,
        manifests: HashMap::new(),
    };
    let top_unit = cx.unit(top, None)?;

    // 1. Reachable units. `order_edges` are "A must be compiled after B" pairs.
    let mut units: HashMap<Node, UnitRecord> = HashMap::new();
    let mut discovered: Vec<Node> = Vec::new();
    let mut order_edges: Vec<(Node, Node)> = Vec::new();
    let mut stack = vec![(top.library.clone(), top_unit)];
    while let Some((lib, u)) = stack.pop() {
        let n = Ctx::node(&lib, &u);
        if units.contains_key(&n) {
            continue;
        }
        let by = format!("{} `{}.{}`", u.kind.describe(), lib, u.name);
        let mut deps: Vec<(String, UnitRecord)> = Vec::new();
        for d in &u.depends_on {
            deps.push((d.library.clone(), cx.unit(d, Some(&by))?));
        }
        // Implicit: secondary units need their primary.
        if let (Some(p), UnitKind::Architecture | UnitKind::PackageBody) = (&u.primary, u.kind) {
            let pk = if u.kind == UnitKind::Architecture {
                UnitKind::Entity
            } else {
                UnitKind::Package
            };
            deps.push((
                lib.clone(),
                cx.unit(
                    &UnitRef {
                        library: lib.clone(),
                        kind: pk,
                        key: p.clone(),
                    },
                    Some(&by),
                )?,
            ));
        }
        for (dl, du) in &deps {
            order_edges.push((n.clone(), Ctx::node(dl, du)));
        }
        let mut pulls: Vec<(String, UnitRecord)> = deps;
        // Pull-only: an entity brings its latest architecture, a package its body.
        match u.kind {
            UnitKind::Entity => {
                pulls.extend(cx.latest_arch(&lib, &u.key)?.map(|a| (lib.clone(), a)))
            }
            UnitKind::Package => pulls.extend(
                cx.manifest(&lib)?
                    .find(UnitKind::PackageBody, &u.key)
                    .cloned()
                    .map(|b| (lib.clone(), b)),
            ),
            _ => {}
        }
        discovered.push(n.clone());
        units.insert(n, u);
        stack.extend(pulls.into_iter().rev());
    }

    // 2. File graph (file = library + path), edges only between reachable units.
    type FileKey = (String, PathBuf);
    let fkey = |n: &Node| -> FileKey { (n.0.clone(), units[n].file.clone()) };
    let mut files: Vec<FileKey> = Vec::new();
    for n in &discovered {
        let f = fkey(n);
        if !files.contains(&f) {
            files.push(f);
        }
    }
    let mut edges: HashMap<FileKey, Vec<(Node, Node)>> = HashMap::new();
    for (a, b) in order_edges {
        if fkey(&a) != fkey(&b) {
            edges.entry(fkey(&a)).or_default().push((a, b));
        }
    }

    // 3. DFS post-order with cycle detection.
    let label = |n: &Node| {
        format!(
            "{} `{}.{}` in {}",
            units[n].kind.describe(),
            n.0,
            units[n].name,
            units[n].file.display()
        )
    };
    let mut done: Vec<FileKey> = Vec::new();
    let mut visiting: Vec<(FileKey, Option<(Node, Node)>)> = Vec::new();
    // Pull-only units (bodies, architectures) have no edge from their puller, so every
    // reachable file is a DFS root (top unit's file first). Only dependencies-first
    // order matters, so a body may follow the units that use its package.
    let roots = files.clone();
    fn visit(
        f: FileKey,
        via: Option<(Node, Node)>,
        edges: &HashMap<FileKey, Vec<(Node, Node)>>,
        done: &mut Vec<FileKey>,
        visiting: &mut Vec<(FileKey, Option<(Node, Node)>)>,
        fkey: &dyn Fn(&Node) -> FileKey,
        label: &dyn Fn(&Node) -> String,
    ) -> Result<()> {
        if done.contains(&f) {
            return Ok(());
        }
        if let Some(pos) = visiting.iter().position(|(v, _)| *v == f) {
            let mut d = err("L0032", "dependency cycle between design units");
            for (_, e) in visiting[pos + 1..]
                .iter()
                .map(|(v, e)| (v, e.clone()))
                .chain(std::iter::once((&f, via)))
            {
                let (a, b) = e.expect("non-root frames have an edge");
                d = d.with_note(format!("{} needs {}", label(&a), label(&b)));
            }
            return Err(Box::new(d.with_help("break the cycle, for example by moving shared declarations into a separate package")));
        }
        visiting.push((f.clone(), via));
        for (a, b) in edges.get(&f).into_iter().flatten() {
            visit(
                fkey(b),
                Some((a.clone(), b.clone())),
                edges,
                done,
                visiting,
                fkey,
                label,
            )?;
        }
        visiting.pop();
        done.push(f);
        Ok(())
    }
    for r in roots {
        visit(r, None, &edges, &mut done, &mut visiting, &fkey, &label)?;
    }

    // 4. Verify content hashes.
    let mut out = Vec::new();
    for (lib, path) in done {
        let fe = cx.manifests[&lib]
            .file(&path)
            .expect("file of a reachable unit");
        check_hash(&lib, &path, &fe.hash)?;
        out.push(ClosureFile {
            library: lib,
            path,
            hash: fe.hash.clone(),
            std: fe.std.clone(),
        });
    }
    Ok(out)
}

fn check_hash(lib: &str, path: &Path, recorded: &str) -> Result<()> {
    let bytes = std::fs::read(path).map_err(|e| {
        Box::new(
            err(
                "L0033",
                format!(
                    "source file `{}` of library `{lib}` cannot be read",
                    path.display()
                ),
            )
            .with_note(e.to_string())
            .with_help("restore the file or re-analyze the design"),
        )
    })?;
    if hash_content(&bytes) != recorded {
        return Err(Box::new(
            err(
                "L0034",
                format!(
                    "`{}` changed on disk since it was analyzed into library `{lib}`",
                    path.display()
                ),
            )
            .with_help(format!(
                "re-analyze it: `yavs analyze --work {lib} {}`",
                path.display()
            )),
        ));
    }
    Ok(())
}
