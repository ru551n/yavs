//! VHDL-2019 reflection (sem.md 2.11, LRM 2019 16.2 and 16.11).
//!
//! `x'reflect` is a `VALUE_MIRROR`, `T'reflect` a `SUBTYPE_MIRROR`, both from
//! `std.reflection`, which is loaded on demand: no use clause is needed. The
//! HIR is an ordinary `ExprKind::Attr { attr: Reflect, .. }` whose prefix is
//! `AttrPrefix::Expr` (value mirror) or `AttrPrefix::Subtype` (subtype mirror);
//! generic instantiation remaps that prefix like any other attribute, so in
//! `to_dict generic (type record_t) parameter (value : record_t)` each instance
//! reflects its actual type.
//!
//! What the runtime (M3) builds a mirror from, all already in the HIR: the
//! prefix subtype (`Expr::subtype` or the `Subtype`), its class
//! (`TypeKind::class`), names for `simple_name` (`SubtypeDef::name`, else the
//! type's declaration), record element names (`RecordElem::name`, original
//! spelling) and index subtypes. The mirror methods are natives (NATIVE.md,
//! reflection rows); they are called through access values to protected types
//! (`m.get_value_class`), resolved by the protected method calls of
//! expr/names.rs (`Via::Method`).

use yavs_syntax::Symbol;

use crate::db::Db;
use crate::decl::DeclKind;
use crate::ids::SubtypeId;

impl Db {
    /// The subtype of `'reflect`: `std.reflection.SUBTYPE_MIRROR` for a type
    /// mark prefix, `VALUE_MIRROR` for an object. `None` when `std.reflection`
    /// cannot be loaded (reported by the loader).
    pub(crate) fn mirror_subtype(&mut self, type_mark: bool) -> Option<SubtypeId> {
        let u = self.load_primary("std", "reflection").ok()?;
        self.note_dep(u);
        let key = Symbol::intern(if type_mark {
            "subtype_mirror"
        } else {
            "value_mirror"
        });
        let d = *self.scopes[self.units[u].scope].names.get(&key)?.first()?;
        match self.decls[d].kind {
            DeclKind::Type(t) => Some(self.types[t].first_subtype),
            _ => None,
        }
    }
}
