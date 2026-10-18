//! Design units, context clauses and configurations (sem.md 1.2, 1.4, 6.5).

use rsim_diag::FileId;
use rsim_syntax::ast;

use crate::LangRev;
use crate::db::{Db, Reported};
use crate::ids::{LibId, UnitId};
use crate::loader::UnitKey;

impl Db {
    /// Analyze one design unit whose AST was taken from the file cache.
    pub(crate) fn analyze_unit(
        &mut self,
        _lib: LibId,
        _key: UnitKey,
        ast: ast::DesignUnit,
        _file: FileId,
        _rev: LangRev,
    ) -> Result<UnitId, Reported> {
        Err(self.unsupported(ast.span, "design unit analysis", "S1"))
    }
}
