use crate::{Result, error::err};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

pub const MANIFEST_FILE: &str = "rsim-lib.toml";
pub const LOCK_FILE: &str = "_RSIM_LIB";
pub(crate) const FORMAT: u32 = 1;

/// Case-fold an identifier: basic identifiers are case-insensitive,
/// extended identifiers (`\Foo\`) are not.
pub fn fold(name: &str) -> String {
    if name.starts_with('\\') {
        name.to_string()
    } else {
        name.to_ascii_lowercase()
    }
}

/// `sha256:<hex>` of file contents.
pub fn hash_content(bytes: &[u8]) -> String {
    let d = Sha256::digest(bytes);
    let mut s = String::from("sha256:");
    for b in d.iter() {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnitKind {
    Entity,
    Architecture,
    Package,
    PackageBody,
    PackageInstance,
    Configuration,
    Context,
}

impl UnitKind {
    pub fn describe(self) -> &'static str {
        match self {
            UnitKind::Entity => "entity",
            UnitKind::Architecture => "architecture",
            UnitKind::Package => "package",
            UnitKind::PackageBody => "package body",
            UnitKind::PackageInstance => "package instance",
            UnitKind::Configuration => "configuration",
            UnitKind::Context => "context",
        }
    }
}

/// A reference to a unit in some library (library name folded).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct UnitRef {
    pub library: String,
    pub kind: UnitKind,
    pub key: String,
}

impl UnitRef {
    pub fn new(library: &str, kind: UnitKind, name: &str) -> Self {
        UnitRef {
            library: fold(library),
            kind,
            key: fold(name),
        }
    }
}

/// One design unit defined by an analyzed file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnitRecord {
    pub kind: UnitKind,
    /// Original spelling.
    pub name: String,
    /// Folded lookup key. Architectures use `entity/arch`.
    pub key: String,
    /// Folded key of the entity (architecture) or package (body) this unit belongs to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub primary: Option<String>,
    pub std: String,
    #[serde(default)]
    pub depends_on: Vec<UnitRef>,
    /// Filled in by the library on load and update; not stored per unit.
    #[serde(skip)]
    pub file: PathBuf,
}

impl UnitRecord {
    pub fn new(kind: UnitKind, name: &str, std: &str) -> Self {
        UnitRecord {
            kind,
            name: name.into(),
            key: fold(name),
            primary: None,
            std: std.into(),
            depends_on: vec![],
            file: PathBuf::new(),
        }
    }
    /// Architecture `arch` of entity `entity`.
    pub fn architecture(entity: &str, arch: &str, std: &str) -> Self {
        UnitRecord {
            name: arch.into(),
            key: format!("{}/{}", fold(entity), fold(arch)),
            primary: Some(fold(entity)),
            ..Self::new(UnitKind::Architecture, arch, std)
        }
    }
    /// Body of package `pkg`.
    pub fn package_body(pkg: &str, std: &str) -> Self {
        UnitRecord {
            primary: Some(fold(pkg)),
            ..Self::new(UnitKind::PackageBody, pkg, std)
        }
    }
    pub fn depends(mut self, library: &str, kind: UnitKind, name: &str) -> Self {
        self.depends_on.push(UnitRef::new(library, kind, name));
        self
    }
    /// Name for messages: `kind name`.
    pub fn display(&self) -> String {
        match (self.kind, &self.primary) {
            (UnitKind::Architecture, Some(e)) => format!("architecture {} of {e}", self.name),
            _ => format!("{} {}", self.kind.describe(), self.name),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileEntry {
    /// Absolute path as analyzed.
    pub path: PathBuf,
    pub hash: String,
    #[serde(default, rename = "unit")]
    pub units: Vec<UnitRecord>,
}

/// Contents of `rsim-lib.toml`. Files are in compile order.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub format: u32,
    pub name: String,
    pub standard: String,
    #[serde(default, rename = "file")]
    pub files: Vec<FileEntry>,
}

impl Manifest {
    pub(crate) fn new(name: &str, standard: &str) -> Self {
        Manifest {
            format: FORMAT,
            name: name.into(),
            standard: standard.into(),
            files: vec![],
        }
    }

    pub(crate) fn parse(text: &str, origin: &std::path::Path) -> Result<Self> {
        let mut m: Manifest = toml::from_str(text).map_err(|e| {
            Box::new(
                err(
                    "L0003",
                    format!("library manifest `{}` is corrupt", origin.display()),
                )
                .with_note(e.to_string()),
            )
        })?;
        if m.format != FORMAT {
            return Err(Box::new(err(
                "L0004",
                format!(
                    "library manifest `{}` has format version {}, this rsim reads version {FORMAT}",
                    origin.display(),
                    m.format
                ),
            )));
        }
        for f in &mut m.files {
            for u in &mut f.units {
                u.file = f.path.clone();
            }
        }
        Ok(m)
    }

    pub(crate) fn to_text(&self) -> String {
        toml::to_string(self).expect("manifest serializes")
    }

    /// Replace everything `path` defined with `units`; the file moves to the end.
    pub(crate) fn replace_file(&mut self, path: PathBuf, hash: String, mut units: Vec<UnitRecord>) {
        self.files.retain(|f| f.path != path);
        for u in &mut units {
            u.file = path.clone();
        }
        for f in &mut self.files {
            f.units
                .retain(|o| !units.iter().any(|u| u.kind == o.kind && u.key == o.key));
        }
        self.files.retain(|f| !f.units.is_empty());
        self.files.push(FileEntry { path, hash, units });
    }

    pub fn units(&self) -> impl Iterator<Item = &UnitRecord> {
        self.files.iter().flat_map(|f| f.units.iter())
    }

    pub fn find(&self, kind: UnitKind, key: &str) -> Option<&UnitRecord> {
        self.units().find(|u| u.kind == kind && u.key == key)
    }

    pub fn file(&self, path: &std::path::Path) -> Option<&FileEntry> {
        self.files.iter().find(|f| f.path == path)
    }
}
