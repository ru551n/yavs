//! Path elements of `'path_name` / `'instance_name` (LRM 16.2.5) that do not depend on the
//! elaborated hierarchy: the text of an item of a library-unit package is the same in every
//! instance, so a subprogram that reads it needs no instance-data slot (#460).

use yavs_sem::Db;
use yavs_sem::decl::DeclKind;

/// `name[type,type return type]` (LRM 16.2.5, lower case): a subprogram's path element. The type
/// marks are the declared subtypes (`std_logic`, `natural`), not their base types, separated by a
/// comma and a space as in sim1 (sim2 writes no space; user decision, #784).
pub fn subprogram_element(db: &Db, d: yavs_sem::DeclId) -> String {
    let decl = &db.decls[d];
    let sub = decl.kind.subprogram().expect("a subprogram declaration");
    let ty = |st| db.subtype_name(st).to_ascii_lowercase();
    let params: Vec<String> = sub.params.iter().map(|p| ty(p.subtype)).collect();
    let mut sig = params.join(", ");
    if let Some(r) = sub.ret {
        if !sig.is_empty() {
            sig.push(' ');
        }
        sig += &format!("return {}", ty(r));
    }
    format!("{}[{sig}]", decl.name.to_string().to_ascii_lowercase())
}

/// The path elements of the subprograms and local package (instance) an item declared in
/// `scope` sits in, outermost first, each with its colon, when it sits in a package; the
/// second value is `:lib:pkg:` when that package is a library unit (then the elements start
/// below it). Empty and `None` for an item outside any package.
pub fn package_prefix(db: &Db, scope: yavs_sem::ScopeId) -> (String, Option<String>) {
    use yavs_sem::scope::ScopeKind as S;
    let mut parts = Vec::new();
    let mut sc = Some(scope);
    while let Some(s) = sc {
        let sd = &db.scopes[s];
        match (sd.kind, sd.owner) {
            (S::Subprogram, Some(o)) => parts.push(format!("{}:", subprogram_element(db, o))),
            (S::Loop | S::SeqBlock, _) => {}
            // A body continues in its package (a library unit's body hangs off its context region).
            (S::PackageBody, _) => {
                sc = db.body_package(s);
                continue;
            }
            (S::Package, _) => {
                if let Some(u) = db.units.iter().map(|(_, u)| u).find(|u| u.scope == s) {
                    let lib = db.libs[u.lib].name.key.to_string();
                    let lib = format!(":{lib}:{}:", u.name.key);
                    return (parts.iter().rev().map(String::as_str).collect(), Some(lib));
                }
                parts.push(format!(
                    "{}:",
                    sd.owner.map_or(String::new(), |o| db.decls[o]
                        .name
                        .to_string()
                        .to_ascii_lowercase())
                ));
                return (parts.iter().rev().map(String::as_str).collect(), None);
            }
            (S::Instance, Some(o))
                if matches!(db.decls[o].kind, yavs_sem::decl::DeclKind::PackageInst(_)) =>
            {
                parts.push(format!(
                    "{}:",
                    db.decls[o].name.to_string().to_ascii_lowercase()
                ));
                return (parts.iter().rev().map(String::as_str).collect(), None);
            }
            _ => break,
        }
        sc = sd.parent;
    }
    (String::new(), None)
}

/// The whole `'path_name` / `'instance_name` of `d` when it is an item of a library-unit package
/// (`:lib:pkg:` then the subprogram elements and the item), the same text in every instance.
pub fn library_package_item_name(db: &Db, d: yavs_sem::DeclId) -> Option<String> {
    let decl = &db.decls[d];
    let own = match &decl.kind {
        DeclKind::Unit(_) | DeclKind::Label(_) => return None,
        DeclKind::Subprogram(_) => format!("{}:", subprogram_element(db, d)),
        _ => decl.name.to_string().to_ascii_lowercase(),
    };
    let (pkg, lib) = package_prefix(db, decl.scope);
    Some(format!("{}{pkg}{own}", lib?))
}
