//! Sequential and concurrent statements (sem.md 7.2). Sequential: slice S2;
//! concurrent, processes, instances: slice S5.

use rsim_syntax::ast;

use crate::db::Db;
use crate::ids::{ScopeId, StmtId};

impl Db {
    /// Analyze a sequence of statements in `scope` (a subprogram, process or
    /// block region). Hook for decl.rs (M1 lead): the owner of this module
    /// replaces the body.
    pub(crate) fn seq_stmts(&mut self, _scope: ScopeId, stmts: &[ast::SeqStmt]) -> Box<[StmtId]> {
        if let Some(s) = stmts.first() {
            self.unsupported(s.span, "sequential statements", "S2");
        }
        Box::new([])
    }
}
