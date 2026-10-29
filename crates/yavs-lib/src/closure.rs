use crate::Result;
use crate::error::err;
use crate::library::Library;
use crate::manifest::{Manifest, UnitKind, UnitLoc, UnitRecord, UnitRef, fold, hash_content};
use std::collections::{HashMap, HashSet};
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
    manifests: HashMap<String, std::sync::Arc<Manifest>>,
    /// The shared locks of the libraries read, held until the closure is done: a record is read
    /// only while no writer can replace it.
    guards: Vec<std::fs::File>,
    /// The records read, by library and position of the file.
    records: HashMap<(String, usize), Vec<UnitRecord>>,
}

impl Ctx<'_> {
    fn manifest(&mut self, lib: &str) -> Result<std::sync::Arc<Manifest>> {
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
            // A directory without an index is an empty library (cli-contract section 3).
            let (m, guard) = if l.has_manifest() {
                l.read_locked()?
            } else {
                (Default::default(), None)
            };
            self.guards.extend(guard);
            self.manifests.insert(lib.to_string(), m);
        }
        Ok(self.manifests[lib].clone())
    }

    /// The record of the unit `loc` of library `lib`.
    fn record(&mut self, lib: &str, m: &Manifest, loc: UnitLoc<'_>) -> Result<UnitRecord> {
        let at = (lib.to_string(), loc.file_idx);
        if !self.records.contains_key(&at) {
            let recs = m.records(&m.file_at(loc.file_idx))?;
            self.records.insert(at.clone(), recs);
        }
        match self.records[&at]
            .iter()
            .find(|u| u.kind == loc.kind && u.key == loc.key)
        {
            Some(u) => Ok(u.clone()),
            None => Err(Box::new(
                err(
                    "L0003",
                    format!(
                        "the record of `{}` in library `{lib}` lacks {} `{}`",
                        loc.file.display(),
                        loc.kind.describe(),
                        loc.key
                    ),
                )
                .with_help(format!(
                    "the library was changed outside yavs: re-analyze `{}`",
                    loc.file.display()
                )),
            )),
        }
    }

    fn unit(&mut self, r: &UnitRef, needed_by: Option<&str>) -> Result<UnitRecord> {
        let m = self.manifest(&r.library)?;
        if let Some(loc) = m.find(r.kind, &r.key) {
            return self.record(&r.library, &m, loc);
        }
        // An architecture is keyed `entity/architecture`.
        let shown = match (r.kind, r.key.split_once('/')) {
            (UnitKind::Architecture, Some((e, a))) => format!("`{a}` of entity `{e}`"),
            _ => format!("`{}`", r.key),
        };
        let want = format!("{} {shown}", r.kind.describe());
        let mut d = err(
            "L0031",
            format!("unknown {want} in library `{}`", r.library),
        );
        if let Some(by) = needed_by {
            d = d.with_note(format!("needed by {by}"));
        }
        let best = m
            .units()
            .map(|u| (edit_distance(u.key, &r.key), u))
            .filter(|(dist, _)| *dist <= (r.key.len() / 3).max(2))
            .min_by_key(|(dist, u)| (*dist, u.kind != r.kind));
        d = match best {
            Some((_, loc)) => {
                let u = self.record(&r.library, &m, loc)?;
                let kind = u.kind.describe();
                let a = if kind.starts_with(['a', 'e', 'i', 'o', 'u']) {
                    "an"
                } else {
                    "a"
                };
                d.with_help(format!(
                    "{a} {kind} with a similar name exists: `{}`",
                    u.name
                ))
            }
            None if m.units().next().is_none() => {
                let dir = self
                    .libs
                    .get(&r.library)
                    .map(|l| l.dir.display().to_string());
                d.with_help(format!(
                    "library `{}` holds no design units{}; analyze the files into it first or fix its `NAME:PATH` mapping",
                    r.library,
                    match dir {
                        Some(dir) if !Path::new(&dir).exists() => {
                            format!(" (its directory `{dir}` does not exist)")
                        }
                        Some(dir) => format!(" (its directory `{dir}` has no analyzed files)"),
                        None => String::new(),
                    }
                ))
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
        let m = self.manifest(lib)?;
        let loc = m
            .units()
            .filter(|u| u.kind == UnitKind::Architecture && u.primary == Some(entity))
            .last();
        loc.map(|loc| self.record(lib, &m, loc)).transpose()
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
        guards: Vec::new(),
        records: HashMap::new(),
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
            UnitKind::Package => {
                let m = cx.manifest(&lib)?;
                if let Some(loc) = m.find(UnitKind::PackageBody, &u.key) {
                    pulls.push((lib.clone(), cx.record(&lib, &m, loc)?));
                }
            }
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
    let mut seen: HashSet<FileKey> = HashSet::new();
    for n in &discovered {
        let f = fkey(n);
        if seen.insert(f.clone()) {
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
    let mut done = Walk::default();
    // Pull-only units (bodies, architectures) have no edge from their puller, so every
    // reachable file is a DFS root (top unit's file first). Only dependencies-first
    // order matters, so a body may follow the units that use its package.
    let roots = files.clone();
    // The files of the package bodies a package's file is followed by: a body that is not in
    // its package's file (a settings package whose body the user supplies) is pull-only, so
    // the DFS below would put it after everything else. Directly after its package, where its
    // own dependencies allow, it is in the same place for every design that shares the
    // package's files, which is what lets the analysis of those files be cached once (#407).
    let mut bodies_of: HashMap<FileKey, Vec<FileKey>> = HashMap::new();
    for (n, u) in &units {
        if let (UnitKind::PackageBody, Some(p)) = (u.kind, &u.primary) {
            let decl: Node = (n.0.clone(), UnitKind::Package, p.clone());
            if units.contains_key(&decl) && fkey(&decl) != fkey(n) {
                bodies_of.entry(fkey(&decl)).or_default().push(fkey(n));
            }
        }
    }
    for v in bodies_of.values_mut() {
        v.sort();
        v.dedup();
    }
    /// The files in post-order and the same set for membership tests, and the files being
    /// visited (a stack with the edge that led to each, and the same set).
    #[derive(Default)]
    struct Walk {
        order: Vec<FileKey>,
        set: HashSet<FileKey>,
        visiting: Vec<(FileKey, Option<(Node, Node)>)>,
        on_stack: HashSet<FileKey>,
    }
    impl Walk {
        fn contains(&self, f: &FileKey) -> bool {
            self.set.contains(f)
        }
        fn push(&mut self, f: FileKey) {
            if self.set.insert(f.clone()) {
                self.order.push(f);
            }
        }
    }
    /// The order edges of each file and the body files that follow a file.
    type Graph<'a> = (
        &'a HashMap<FileKey, Vec<(Node, Node)>>,
        &'a HashMap<FileKey, Vec<FileKey>>,
    );
    fn visit(
        f: FileKey,
        via: Option<(Node, Node)>,
        graph: Graph,
        done: &mut Walk,
        fkey: &dyn Fn(&Node) -> FileKey,
        label: &dyn Fn(&Node) -> String,
    ) -> Result<()> {
        let (edges, bodies_of) = graph;
        if done.contains(&f) {
            return Ok(());
        }
        if done.on_stack.contains(&f)
            && let Some(pos) = done.visiting.iter().position(|(v, _)| *v == f)
        {
            let mut d = err("L0032", "dependency cycle between design units");
            for (_, e) in done.visiting[pos + 1..]
                .iter()
                .map(|(v, e)| (v, e.clone()))
                .chain(std::iter::once((&f, via)))
            {
                let (a, b) = e.expect("non-root frames have an edge");
                d = d.with_note(format!("{} needs {}", label(&a), label(&b)));
            }
            return Err(Box::new(d.with_help("break the cycle, for example by moving shared declarations into a separate package")));
        }
        done.visiting.push((f.clone(), via));
        done.on_stack.insert(f.clone());
        for (a, b) in edges.get(&f).into_iter().flatten() {
            visit(
                fkey(b),
                Some((a.clone(), b.clone())),
                graph,
                done,
                fkey,
                label,
            )?;
        }
        done.visiting.pop();
        done.on_stack.remove(&f);
        done.push(f.clone());
        for b in bodies_of.get(&f).into_iter().flatten() {
            if !done.contains(b)
                && edges
                    .get(b)
                    .into_iter()
                    .flatten()
                    .all(|(_, t)| done.contains(&fkey(t)))
            {
                done.push(b.clone());
            }
        }
        Ok(())
    }
    for r in roots {
        visit(r, None, (&edges, &bodies_of), &mut done, &fkey, &label)?;
    }

    // 4. Verify content hashes.
    let mut out = Vec::new();
    for (lib, path) in done.order {
        let fe = cx.manifests[&lib]
            .file(&path)
            .expect("file of a reachable unit");
        if !fe.unchanged() {
            let dir = cx.libs.get(&lib).map(|l| l.dir.as_path());
            check_hash(&lib, dir, fe.std, &path, fe.hash)?;
        }
        out.push(ClosureFile {
            library: lib,
            path,
            hash: fe.hash.to_string(),
            std: fe.std.to_string(),
        });
    }
    Ok(out)
}

fn check_hash(lib: &str, dir: Option<&Path>, std: &str, path: &Path, recorded: &str) -> Result<()> {
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
                "re-analyze it: `yavs analyze --std={std} --work={lib}:{} {}`",
                dir.unwrap_or(Path::new("DIR")).display(),
                path.display()
            )),
        ));
    }
    Ok(())
}
