//! The rules, and the walk of a unit's own regions that feeds them. Each rule states its evidence
//! and reports nothing when it is missing: a value is known only when analysis folded it, so a
//! width or index a generic decides is never judged, and code under a condition that folds to
//! false is not walked. Instances of generics are skipped: their source is the generic's.

use crate::{Certainty, Config, Rule};
use serde_json::{Value, json};
use yavs_diag::Span;
use yavs_sem::decl::{Body, Decl, DeclKind, Subprogram, SubprogramKind};
use yavs_sem::hir::{
    Actual, Arg, BlockConfig, Choice, ChoiceSet, ConfigItem, ConstValue, Direction, EntityAspect,
    ExprKind, GenAlt, InstTarget, LoopScheme, RangeC, Rhs, Sensitivity, StmtKind, UnitHir,
    Waveform,
};
use yavs_sem::implicit::Family;
use yavs_sem::loader::UnitKey;
use yavs_sem::types::{Constraint, TypeKind};
use yavs_sem::{Db, DeclId, ExprId, LibId, ScopeId, StmtId, SubtypeId, UnitId};
use yavs_syntax::ast::Ident;

const fn rule(
    id: &'static str,
    certainty: Certainty,
    summary: &'static str,
    explain: &'static str,
) -> Rule {
    Rule {
        id,
        certainty,
        summary,
        explain,
    }
}

const D: Certainty = Certainty::Definite;
const A: Certainty = Certainty::Advisory;
const X: Certainty = Certainty::Experimental;
const P: Certainty = Certainty::Policy;

/// Every rule, in id order.
pub static RULES: &[Rule] = &[
    rule(
        "lint_001",
        A,
        "A signal read by a combinational process is missing from its sensitivity list.",
        "The process does not run when that signal changes, so what it computes goes stale and simulation differs from the logic the text describes. A process that tests a clock edge or suspends on `wait` is not judged.",
    ),
    rule(
        "lint_002",
        A,
        "A signal in a sensitivity list is not read by the process.",
        "The process runs for nothing when the signal changes, or something it was meant to read is missing. A process that calls a subprogram is not judged, as the subprogram may read the signal by name.",
    ),
    rule(
        "lint_004",
        A,
        "A declaration is never used.",
        "A signal, constant or variable of an architecture that nothing names. A declaration of a package can be used anywhere, and types and subprograms are not judged.",
    ),
    rule(
        "lint_005",
        A,
        "`library work` is implicit and does not need declaring.",
        "Every design unit has it. A clause that repeats it says nothing.",
    ),
    rule(
        "lint_006",
        A,
        "A context clause is not attached to any design unit.",
        "A context clause belongs to the unit after it. Analysis already rejects one with no unit after it, so a lint run shows that error and this rule never adds a finding of its own; it stays so that a configuration can name it.",
    ),
    rule(
        "lint_600",
        X,
        "A combinational process does not assign a signal on every path, inferring a latch.",
        "The signal keeps its value on the path that does not assign it, which a synthesis tool builds as a latch. A process is taken as combinational when it tests no edge and has no `wait` or procedure call. A signal assigned first has a default and is not reported. A signal assigned in part or inside a loop is not judged.",
    ),
    rule(
        "lint_601",
        A,
        "A signal is assigned by more than one concurrent statement.",
        "Legal on a resolved type, where resolution decides the value rather than the design. Deliberate tri-state buses are reported too: waive those. Parts are compared by value, so `q(3)` and `q(4)` are two signals; parts a generic decides do not count, and neither do statements under different generate alternatives, as a generic decides which exist. An output of an instance is a driver, an `inout` too.",
    ),
    rule(
        "lint_602",
        P,
        "A signal assigned by a clocked process does not have a register suffix (set `suffixes`).",
        "A naming convention. `suffixes` lists what is accepted, `*` and `?` allowed; the default is `_q`, `_r`, `_reg`.",
    ),
    rule(
        "lint_603",
        P,
        "A signal assigned by a clocked process does not have a register prefix (set `prefixes`).",
        "A naming convention. `prefixes` lists what is accepted, `*` and `?` allowed; the default is `r_`.",
    ),
    rule(
        "lint_700",
        X,
        "A signal registered on one clock is used in logic on another, without a synchroniser.",
        "The receiving flop can sample the signal while it changes and go metastable. The clock of a register is the edge its process tests; an architecture needs two clocks for this to be asked. A capture into a flop and nothing else counts as the first stage of a synchroniser, so a single stage is accepted, and a signal on two clocks is skipped.",
    ),
    rule(
        "lint_701",
        X,
        "A register reset by one signal is used in logic reset by another.",
        "The resets can release at different moments, so the destination captures from a register that is mid-release. Only an asynchronous reset counts: the branch a clock edge is the `elsif` of. A capture into a flop and nothing else is accepted; a register cleared by two resets is skipped.",
    ),
    rule(
        "lint_702",
        A,
        "A register a clocked process assigns but its reset branch does not.",
        "The reset branch says what the design starts from, and this register starts from what it holds. Often an oversight, sometimes a saving on a wide data path. Only an asynchronous reset is judged: the branch a clock edge is the `elsif` of.",
    ),
    rule(
        "lint_703",
        A,
        "A clock used on both its rising and its falling edge.",
        "It halves the time between the two sets of registers. Sometimes that is the design: double data rate logic, or an interface that samples on one edge and shifts on the other. An edge in a `wait` is a process suspending, and is not counted.",
    ),
    rule(
        "lint_710",
        A,
        "A state of an enumerated state machine is never entered.",
        "No assignment gives this value to a signal of an enumeration the architecture declares and a `case` selects on, and no initial value does. The machine has to be readable: a value that does not fold, or a signal driven through a port or a parameter, leaves the type alone.",
    ),
    rule(
        "lint_711",
        A,
        "A state of an enumerated state machine has no exit.",
        "The `case` alternative of the state never assigns another state, so the machine cannot leave it unless it is a deliberate terminal state: waive that. A `case` that assigns no state at all is a multiplexer and is not checked, and neither is one after a default assignment, as the default is then the exit.",
    ),
    rule(
        "lint_712",
        A,
        "A `when others` alternative that no value can reach.",
        "The other alternatives already name every value of the selector. Add a value to the enumeration and the new value falls into `others`; without it, the same edit is an error naming the missing choice. The selector must be an object of an enumeration type with every choice folded.",
    ),
    rule(
        "lint_713",
        P,
        "A `when others` alternative on an enumeration, instead of naming every value.",
        "While `others` is there, a new value of the enumeration quietly takes that branch. A project that wants every value spelled out enables this. It takes the `others` alternatives that do stand in for values, and lint_712 those that do not.",
    ),
    rule(
        "lint_720",
        A,
        "A signal depends on itself through combinational logic, with no register in the loop.",
        "Hardware oscillates or settles somewhere nobody chose. A clocked process gives no dependency. Left out so that a cycle is a real one: assignments to and reads of part of an object, anything through an instance, attributes, and assignments with an `after`.",
    ),
    rule(
        "lint_730",
        A,
        "A signal is read but nothing drives it: no assignment, and no instance output.",
        "It holds its initial value for ever: usually a wire someone forgot to connect. A signal with an initial value is taken as tied on purpose; a subprogram that names the signal may drive it, so that counts as driven.",
    ),
    rule(
        "lint_740",
        D,
        "A vector is assigned to one of a different width.",
        "Both sides are of one array type, so analysis has nothing to say; the lengths are compared when the assignment runs, and it fails. Reported when the lengths of the target and of the value are both known: an object or a slice with a range that folds, or a literal. A width that depends on a generic, a function result or an operator is not judged.",
    ),
    rule(
        "lint_741",
        D,
        "A vector is compared with one of a different width, which is never equal.",
        "The predefined `=` is true only for equal lengths with matching elements, so comparing a four-bit vector with a three-bit literal is legal and always false (`/=` always true). Reported for the predefined operators only: an overloaded `=` such as the one of an arithmetic package may resize its operands. Both lengths must be known, as for lint_740.",
    ),
    rule(
        "lint_750",
        A,
        "A component declaration does not match the entity it stands for.",
        "A component is a hand-written copy of an entity's interface, and copies drift. Port names, modes and order are compared, not types. Only an entity of that name in the same library, analyzed in this run, is compared, so a component standing for a primitive is left alone.",
    ),
    rule(
        "lint_751",
        D,
        "A binding indication or an instance names an architecture that is not declared.",
        "`use entity work.e(a)` in a configuration and `i : entity work.e(a)` in an architecture name architecture `a` of `e`, and elaboration fails when the library has no such architecture. Analysis accepts them, as the architecture may come later. The library's own list is the evidence, so analyze the whole library first. A block configuration `for a` with no such architecture is an analysis error, E1002.",
    ),
    rule(
        "lint_760",
        A,
        "A subprogram whose body calls itself, which no synthesis tool accepts.",
        "Recursion is legal and simulates, and code only ever simulated may be right to use it. For code that has to synthesise, the call has no bottom to inline from. The call is resolved, so another subprogram of the same name does not count. Direct recursion only.",
    ),
    rule(
        "lint_770",
        D,
        "A process with no sensitivity list and no wait statement, which can never suspend.",
        "A process repeats for ever; it lets time advance only by suspending, at a `wait` or at the end of its body when it has a sensitivity list. With neither, simulation time never advances. A process that calls a procedure is left alone, because the procedure may hold the `wait`. A `wait` that looks unreachable still counts.",
    ),
    rule(
        "lint_771",
        D,
        "A function that can reach the end of its body without returning a value.",
        "Reaching the end of a function is an error, reported only when a run takes that path with those arguments. Anything unclear counts as an ending, so correct code is not accused: a loop is assumed to return, and so is an assertion or report that always fails, and a procedure call that ends the body, which may stop the run. A `case` covers its subtype, so it needs no `others`.",
    ),
    rule(
        "lint_772",
        D,
        "A statement that nothing can reach.",
        "The statement follows a `return`, or an `exit` or `next` with no condition, or an `if` or `case` all of whose paths do one of those. An assertion is no transfer of control, even at severity failure. A `translate_off` or `synthesis_off` pragma between the two statements makes this no finding: code unreachable in simulation and reachable in synthesis is how a design asks which tool reads it.",
    ),
    rule(
        "lint_780",
        D,
        "An index that is outside the declared range of the array it selects from.",
        "Evaluating the index raises an error. Reported when the index and the range of the array both fold; `yavs analyze` warns about some of the same (W1404), and this rule makes it a finding of the lint run.",
    ),
    rule(
        "lint_781",
        D,
        "A division, `mod` or `rem` whose divisor is zero.",
        "The right operand of the predefined `/`, `mod` and `rem` may not be zero: evaluating it raises an error. Reported when the divisor folds to zero. An overloaded operator defines its own behaviour and is not judged.",
    ),
    rule(
        "lint_782",
        D,
        "An assignment of a value outside the declared range of its target.",
        "The value has to belong to the target's subtype, so the assignment raises an error when it runs. Analysis already rejects this when the value and the range both fold, as E1403, so a lint run shows that error and this rule never adds a finding of its own; it stays so that a configuration can name it.",
    ),
    rule(
        "lint_790",
        A,
        "A signal declared in a wider region than the one that uses it.",
        "Where a signal is declared says who it belongs to. Declared in the architecture and used only inside one block or generate body, it reads as shared state when it is not; in a `for ... generate` it is one signal for every iteration, where one per iteration was probably meant. The guard and maps of a block and the header of a generate are read in the region around them. An unused signal, and one with an attribute specification, are not reported.",
    ),
];

pub(crate) type Raw = (&'static str, Span, String);

/// What every rule finds in `units`, with no configuration applied.
pub(crate) fn check(db: &Db, units: &[UnitId], cfg: &Config, facts: bool) -> (Vec<Raw>, Value) {
    let mut w = Walk {
        db,
        out: Vec::new(),
        seen: Default::default(),
        within: None,
        lib: None,
    };
    for &u in units {
        w.unit(u);
    }
    let mut out = w.out;
    let mut arches = Vec::new();
    if facts || cfg.beyond_definite() {
        for &u in units {
            let f = crate::design::architecture(db, u, cfg, &mut out);
            if !f.is_null() {
                arches.push(f);
            }
        }
    }
    let files: std::collections::HashSet<_> = units.iter().map(|&u| db.units[u].file).collect();
    if crate::rule("lint_005").is_some_and(|r| cfg.enabled(r)) {
        for &file in &files {
            library_work(db, file, &mut out);
        }
    }
    out.retain(|(_, span, _)| files.contains(&span.file));
    let facts = if facts {
        json!({ "schema_version": 1, "architectures": arches })
    } else {
        Value::Null
    };
    (out, facts)
}

/// `lint_005`: the syntax of the file is the only place that says what its context clauses wrote.
fn library_work(db: &Db, file: yavs_diag::FileId, out: &mut Vec<Raw>) {
    let rev = db
        .units
        .iter()
        .find(|(_, u)| u.file == file)
        .map_or(db.rev, |(_, u)| u.rev);
    let opts = yavs_syntax::Options {
        std: rev,
        defines: Vec::new(),
        psl: false,
    };
    let source = db.sources.get(file);
    if source.try_text().is_err() {
        return;
    }
    let Some(design) = yavs_syntax::parse_file(file, source, &opts).design else {
        return;
    };
    for unit in &design.units {
        for item in &unit.context {
            if let yavs_syntax::ast::ContextItem::Library { names, .. } = item {
                for n in names.iter().filter(|n| n.key.as_str() == "work") {
                    out.push((
                        "lint_005",
                        n.span,
                        "`library work` is implicit and does not need declaring".to_string(),
                    ));
                }
            }
        }
    }
}

struct Walk<'a> {
    db: &'a Db,
    out: Vec<Raw>,
    /// Regions walked: a protected type and a subprogram with a spec are reached twice.
    seen: std::collections::HashSet<ScopeId>,
    /// The subprogram whose body is being walked: its spec's declaration and its body's.
    within: Option<(DeclId, Option<DeclId>, String)>,
    /// The library of the unit being walked.
    lib: Option<LibId>,
}

impl Walk<'_> {
    fn unit(&mut self, u: UnitId) {
        let unit = &self.db.units[u];
        self.lib = Some(unit.lib);
        self.region(unit.scope);
        // A subprogram with a spec in the primary unit is declared there, with the body found in
        // this one; findings in other files are dropped by the caller.
        match &unit.hir {
            UnitHir::Entity { stmts, .. } => self.list(stmts),
            UnitHir::Architecture { entity, stmts } => {
                self.region(self.db.units[*entity].scope);
                self.list(stmts);
            }
            UnitHir::PackageBody { package } => self.region(self.db.units[*package].scope),
            UnitHir::Configuration(block) => self.configuration(block),
            _ => {}
        }
    }

    /// The declarations of a region. Instances are skipped: their source is the generic's.
    fn region(&mut self, scope: ScopeId) {
        if !self.seen.insert(scope) {
            return;
        }
        for &d in &self.db.scopes[scope].order {
            let decl = &self.db.decls[d];
            match &decl.kind {
                DeclKind::Object(o) => {
                    if let Some(e) = o.default {
                        self.expr(e);
                    }
                }
                DeclKind::Subprogram(s) => self.subprogram(d, decl, s),
                DeclKind::Component(_) => {
                    if let Some(lib) = self.lib {
                        crate::design::component(self.db, d, lib, &mut self.out);
                    }
                }
                DeclKind::Type(t) => {
                    if let TypeKind::Protected {
                        decl_scope, body, ..
                    } = &self.db.types[*t].kind
                    {
                        self.region(*decl_scope);
                        if let Some(b) = body {
                            self.region(*b);
                        }
                    }
                }
                DeclKind::Package { scope, body, .. } => {
                    self.region(*scope);
                    if let Some(b) = body {
                        self.region(*b);
                    }
                }
                _ => {}
            }
        }
    }

    fn subprogram(&mut self, d: DeclId, decl: &Decl, s: &Subprogram) {
        let Body::Hir { scope, stmts } = &s.body else {
            return;
        };
        // A subprogram with a spec is declared twice with one body: walk it once.
        if self.seen.contains(scope) {
            return;
        }
        self.region(*scope);
        if matches!(s.kind, SubprogramKind::Function { .. }) && !self.ends(stmts, true) {
            self.out.push((
                "lint_771",
                self.body_place(decl, *scope, stmts),
                format!(
                    "`{}` can reach the end of its body without returning a value, which is an error when it does",
                    decl.name
                ),
            ));
        }
        let outer = self
            .within
            .replace((d, self.db.scopes[*scope].owner, decl.name.to_string()));
        self.list(stmts);
        self.within = outer;
    }

    /// `lint_751`: an architecture named for `unit` that its library does not have.
    fn named_architecture(&mut self, unit: UnitId, arch: &Ident, what: &str) {
        let db = self.db;
        let UnitKey::Primary(entity) = db.units[unit].key else {
            return;
        };
        let lib = db.units[unit].lib;
        let key = UnitKey::Secondary(entity, arch.key);
        if !db.libs[lib].units.contains_key(&key) && db.locate(lib, key).is_none() {
            self.out.push((
                "lint_751",
                arch.span,
                format!(
                    "{what} architecture `{}` of `{}`, which is not declared",
                    arch.text, db.units[unit].name.text
                ),
            ));
        }
    }

    /// `lint_751`: the architecture a binding indication names, in this block and those below it.
    fn configuration(&mut self, block: &BlockConfig) {
        for item in &block.items {
            match item {
                ConfigItem::Block(b) => self.configuration(b),
                ConfigItem::Component(c) => {
                    if let Some(binding) = &c.binding
                        && let Some(EntityAspect::Entity {
                            unit,
                            arch: Some(arch),
                        }) = &binding.aspect
                    {
                        self.named_architecture(*unit, arch, "Configuration binds to");
                    }
                    if let Some(b) = &c.block {
                        self.configuration(b);
                    }
                }
            }
        }
    }

    /// `lint_760`: a call of the subprogram whose body holds it.
    fn call(&mut self, callee: DeclId, span: Span) {
        if let Some((d, owner, name)) = &self.within
            && (callee == *d || Some(callee) == *owner)
        {
            let message = format!("Subprogram `{name}` calls itself");
            self.out.push(("lint_760", span, message));
        }
    }

    /// `lint_712` and `lint_713`: `when others` on a selector whose values are all known.
    fn others(&mut self, s: StmtId, expr: ExprId, alts: &[(ChoiceSet, Box<[StmtId]>)]) {
        let db = self.db;
        let x = &db.exprs[expr];
        if !matches!(x.kind, ExprKind::Obj(_)) {
            return;
        }
        let TypeKind::Enum { lits } = &db.types[db.subtypes[x.subtype].base].kind else {
            return;
        };
        let (low, high) = match &db.subtypes[x.subtype].constraint {
            Constraint::None => (0, lits.len() as i64 - 1),
            Constraint::Range(r) => match self.bounds(r) {
                Some((l, Direction::Downto, r)) => (r, l),
                Some((l, _, r)) => (l, r),
                None => return,
            },
            _ => return,
        };
        let mut place = None;
        let mut named = vec![false; lits.len()];
        for (set, body) in alts {
            if set.choices.iter().any(|c| matches!(c, Choice::Others)) {
                place = Some(body.first().map_or(db.stmts[s].span, |&b| db.stmts[b].span));
                continue;
            }
            let Some(intervals) = &set.intervals else {
                return;
            };
            for &(a, b) in intervals.iter() {
                for p in a..=b {
                    if let Some(n) = named.get_mut(p as usize) {
                        *n = true;
                    }
                }
            }
        }
        let Some(place) = place else { return };
        let missing: Vec<String> = (low..=high)
            .filter(|&p| !named.get(p as usize).copied().unwrap_or(true))
            .filter_map(|p| lits.get(p as usize))
            .map(|&l| db.decls[l].name.to_string())
            .collect();
        let selector = self.shown(expr);
        if missing.is_empty() {
            self.out.push((
                "lint_712",
                place,
                format!(
                    "`when others` can never be taken: the other alternatives already name all {} values of `{selector}`",
                    high - low + 1
                ),
            ));
        } else {
            self.out.push((
                "lint_713",
                place,
                format!(
                    "`when others` covers {} of the values of `{selector}` ({})",
                    missing.len(),
                    missing.join(", ")
                ),
            ));
        }
    }

    /// Where to name a function: the body, if a parameter or statement of it says where it is (the
    /// declaration in hand may be the spec, in another file).
    fn body_place(&self, decl: &Decl, scope: ScopeId, stmts: &[StmtId]) -> Span {
        let first = self.db.scopes[scope]
            .order
            .first()
            .map(|&d| self.db.decls[d].span);
        first
            .or_else(|| stmts.first().map(|&s| self.db.stmts[s].span))
            .unwrap_or_else(|| decl.name_span())
    }

    /// The value of `e` as a position, when analysis folded it to an integer, physical or
    /// enumeration value.
    fn pos(&self, e: ExprId) -> Option<i64> {
        match self.db.value(e)? {
            ConstValue::Int(v) | ConstValue::Phys(v) => Some(*v),
            ConstValue::Enum(p) => Some(*p as i64),
            _ => None,
        }
    }

    /// Whether `e` folded to the boolean (or bit) of this position.
    fn folded(&self, e: ExprId, pos: u32) -> bool {
        matches!(self.db.value(e), Some(ConstValue::Enum(p)) if *p == pos)
    }

    /// Whether control always leaves this list before its end. `lenient`: is asked of a function
    /// that may fall through, where what is unclear counts as an ending so correct code is not
    /// accused; otherwise only what certainly transfers counts.
    fn ends(&self, list: &[StmtId], lenient: bool) -> bool {
        list.iter().any(|&s| self.transfers(s, lenient))
            || (lenient
                && list
                    .last()
                    .is_some_and(|&s| matches!(self.db.stmts[s].kind, StmtKind::ProcCall(_))))
    }

    fn transfers(&self, s: StmtId, lenient: bool) -> bool {
        let fails = |sev: &Option<ExprId>| {
            // `failure` is the last position of `severity_level`.
            sev.is_some_and(|e| self.folded(e, 3))
        };
        match &self.db.stmts[s].kind {
            StmtKind::Return(_) => true,
            StmtKind::Exit { when, .. } | StmtKind::Next { when, .. } => when.is_none(),
            StmtKind::Assert { cond, severity, .. } => {
                lenient && (self.folded(*cond, 0) || fails(severity))
            }
            StmtKind::Report { severity, .. } => lenient && fails(severity),
            StmtKind::If { branches, else_ } => else_.as_ref().is_some_and(|e| {
                self.ends(e, lenient) && branches.iter().all(|(_, b)| self.ends(b, lenient))
            }),
            StmtKind::Case { alts, .. } => {
                !alts.is_empty() && alts.iter().all(|(_, b)| self.ends(b, lenient))
            }
            StmtKind::Loop { .. } => lenient,
            StmtKind::SeqBlock { body, .. } => self.ends(body, lenient),
            _ => false,
        }
    }

    /// A statement list: what follows a transfer of control, then each statement.
    fn list(&mut self, list: &[StmtId]) {
        for pair in list.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            if self.transfers(a, false) && !self.pragma_between(a, b) {
                self.out.push((
                    "lint_772",
                    self.db.stmts[b].span,
                    "Nothing reaches this statement: the statement before it always transfers control"
                        .to_string(),
                ));
            }
        }
        for &s in list {
            self.stmt(s);
        }
    }

    /// A pragma in the text between two statements: what follows is reachable for another tool.
    /// An unreadable source counts as one, as the rule then cannot tell.
    fn pragma_between(&self, a: StmtId, b: StmtId) -> bool {
        let (a, b) = (self.db.stmts[a].span, self.db.stmts[b].span);
        let Some(between) = self.text(Span::new(a.file, a.end, b.start)) else {
            return true;
        };
        let between = between.to_ascii_lowercase();
        between.contains("translate_o") || between.contains("synthesis_o")
    }

    fn text(&self, span: Span) -> Option<&str> {
        let file = self.db.sources.get(span.file);
        file.try_text()
            .ok()?
            .get(file.text_offset(span.start)..file.text_offset(span.end))
    }

    /// `e` as written, for a message.
    fn shown(&self, e: ExprId) -> String {
        self.text(self.db.exprs[e].span)
            .map_or_else(|| "the value".to_string(), str::to_string)
    }

    fn stmt(&mut self, s: StmtId) {
        let db = self.db;
        let stmt = &db.stmts[s];
        match &stmt.kind {
            StmtKind::Wait { until, timeout, .. } => {
                self.opt(*until);
                self.opt(*timeout);
            }
            StmtKind::Assert {
                cond,
                report,
                severity,
            }
            | StmtKind::ConcAssert {
                cond,
                report,
                severity,
                ..
            } => {
                self.expr(*cond);
                self.opt(*report);
                self.opt(*severity);
            }
            StmtKind::Report { report, severity } => {
                self.expr(*report);
                self.opt(*severity);
            }
            StmtKind::SignalAssign { target, rhs, .. }
            | StmtKind::ConcSignalAssign { target, rhs, .. } => {
                self.expr(target.expr);
                self.waveforms(target.expr, rhs);
            }
            StmtKind::Force { target, rhs, .. } | StmtKind::VarAssign { target, rhs, .. } => {
                self.expr(target.expr);
                self.values(target.expr, rhs);
            }
            StmtKind::Release { target, .. } => self.expr(target.expr),
            StmtKind::ProcCall(c) | StmtKind::ConcProcCall { call: c, .. } => {
                self.call(c.callee, c.span);
                self.args(&c.args);
            }
            StmtKind::If { branches, else_ } => {
                let mut open = true;
                for (c, b) in branches {
                    self.expr(*c);
                    // Code a constant turned off is not judged.
                    if !self.folded(*c, 0) {
                        self.list(b);
                    }
                    open &= !self.folded(*c, 1);
                    if !open {
                        return;
                    }
                }
                if let Some(e) = else_ {
                    self.list(e);
                }
            }
            StmtKind::Case { expr, alts, .. } => {
                self.expr(*expr);
                self.others(s, *expr, alts);
                for (_, b) in alts {
                    self.list(b);
                }
            }
            StmtKind::Loop {
                scheme,
                scope,
                body,
            } => {
                match scheme {
                    LoopScheme::While(c) => self.expr(*c),
                    LoopScheme::For { range, .. } => self.range(range),
                    LoopScheme::Forever => {}
                }
                self.region(*scope);
                self.list(body);
            }
            StmtKind::Next { when, .. } | StmtKind::Exit { when, .. } => self.opt(*when),
            StmtKind::Return(e) => self.opt(*e),
            StmtKind::Null => {}
            StmtKind::SeqBlock { scope, body } => {
                self.region(*scope);
                self.list(body);
            }
            StmtKind::Process {
                sensitivity,
                scope,
                body,
                ..
            } => {
                if matches!(sensitivity, Sensitivity::None) && !self.may_suspend(body) {
                    let named = stmt
                        .label
                        .map_or(String::new(), |l| format!(" `{}`", l.text));
                    self.out.push((
                        "lint_770",
                        stmt.span,
                        format!(
                            "Process{named} can never suspend: it has no sensitivity list and no wait statement, so simulation time cannot advance"
                        ),
                    ));
                }
                self.region(*scope);
                self.list(body);
            }
            StmtKind::Block {
                scope,
                generic_map,
                port_map,
                body,
                ..
            } => {
                self.args(generic_map);
                self.args(port_map);
                self.region(*scope);
                self.list(body);
            }
            StmtKind::Instance {
                target,
                generic_map,
                port_map,
                ..
            } => {
                if let InstTarget::Entity {
                    unit,
                    arch: Some(arch),
                } = target
                {
                    self.named_architecture(*unit, arch, "Instance names");
                }
                self.args(generic_map);
                self.args(port_map);
            }
            StmtKind::ForGenerate {
                range, scope, body, ..
            } => {
                self.range(range);
                self.region(*scope);
                self.list(body);
            }
            StmtKind::IfGenerate { alts } => {
                for alt in alts {
                    if let Some(c) = alt.cond {
                        self.expr(c);
                    }
                    if !alt.cond.is_some_and(|c| self.folded(c, 0)) {
                        self.alternative(alt);
                    }
                }
            }
            StmtKind::CaseGenerate { expr, alts } => {
                self.expr(*expr);
                for (_, alt) in alts {
                    self.alternative(alt);
                }
            }
        }
    }

    fn alternative(&mut self, alt: &GenAlt) {
        self.region(alt.scope);
        self.list(&alt.body);
    }

    /// Whether any statement below may suspend: a `wait`, or a call that may hold one.
    fn may_suspend(&self, list: &[StmtId]) -> bool {
        list.iter().any(|&s| match &self.db.stmts[s].kind {
            StmtKind::Wait { .. } | StmtKind::ProcCall(_) => true,
            StmtKind::If { branches, else_ } => {
                branches.iter().any(|(_, b)| self.may_suspend(b))
                    || else_.as_ref().is_some_and(|e| self.may_suspend(e))
            }
            StmtKind::Case { alts, .. } => alts.iter().any(|(_, b)| self.may_suspend(b)),
            StmtKind::Loop { body, .. } | StmtKind::SeqBlock { body, .. } => self.may_suspend(body),
            _ => false,
        })
    }

    fn opt(&mut self, e: Option<ExprId>) {
        if let Some(e) = e {
            self.expr(e);
        }
    }

    fn range(&mut self, r: &RangeC) {
        if let RangeC::Explicit { left, right, .. } = r {
            self.expr(*left);
            self.expr(*right);
        }
    }

    fn args(&mut self, args: &[Arg]) {
        for a in args {
            match &a.actual {
                Actual::Expr(e) | Actual::Converted { actual: e, .. } => self.expr(*e),
                Actual::Individual(parts) => {
                    for (_, e, ..) in parts.iter() {
                        self.expr(*e);
                    }
                }
                _ => {}
            }
        }
    }

    fn waveforms(&mut self, target: ExprId, rhs: &Rhs<Waveform>) {
        let one = |w: &mut Self, f: &Waveform| {
            if let Waveform::Elements(els) = f {
                for el in els {
                    if let Some(v) = el.value {
                        w.assigned(target, v);
                    }
                    w.opt(el.after);
                }
            }
        };
        match rhs {
            Rhs::Simple(f) => one(self, f),
            Rhs::Conditional(alts) => {
                for (f, c) in alts {
                    one(self, f);
                    self.opt(*c);
                }
            }
            Rhs::Selected { expr, alts, .. } => {
                self.expr(*expr);
                for (f, _) in alts {
                    one(self, f);
                }
            }
        }
    }

    fn values(&mut self, target: ExprId, rhs: &Rhs<ExprId>) {
        match rhs {
            Rhs::Simple(v) => self.assigned(target, *v),
            Rhs::Conditional(alts) => {
                for (v, c) in alts {
                    self.assigned(target, *v);
                    self.opt(*c);
                }
            }
            Rhs::Selected { expr, alts, .. } => {
                self.expr(*expr);
                for (v, _) in alts {
                    self.assigned(target, *v);
                }
            }
        }
    }

    /// The bounds of a range, when both fold.
    fn bounds(&self, r: &RangeC) -> Option<(i64, Direction, i64)> {
        match r {
            RangeC::Explicit { left, dir, right } => {
                Some((self.pos(*left)?, *dir, self.pos(*right)?))
            }
            RangeC::Attr { .. } => None,
        }
    }

    /// The range of each dimension of an array subtype, where it folds.
    fn dims(&self, st: SubtypeId) -> Vec<Option<(i64, Direction, i64)>> {
        match &self.db.subtypes[st].constraint {
            Constraint::Array {
                dims: Some(dims), ..
            } => dims.iter().map(|r| self.bounds(r)).collect(),
            _ => Vec::new(),
        }
    }

    /// The length of `e` when it is certain: a one-dimensional object or slice whose range folds,
    /// or a literal. A call or an operator can give any length at run time.
    fn width(&self, e: ExprId) -> Option<i64> {
        let x = &self.db.exprs[e];
        let len = |(l, d, r): (i64, Direction, i64)| {
            let n = if d == Direction::Downto { l - r } else { r - l };
            (n + 1).max(0)
        };
        match &x.kind {
            ExprKind::Obj(_) => match self.dims(x.subtype).as_slice() {
                [Some(b)] => Some(len(*b)),
                _ => None,
            },
            ExprKind::Slice { range, .. } => self.bounds(range).map(len),
            ExprKind::Const(_) => match self.db.value(e)? {
                ConstValue::Array(a) => match &*a.bounds {
                    [b] => Some(len(*b)),
                    _ => None,
                },
                _ => None,
            },
            _ => None,
        }
    }

    /// An assignment of `value` to `target`: its width, then the value's own expressions.
    fn assigned(&mut self, target: ExprId, value: ExprId) {
        if let (Some(t), Some(v)) = (self.width(target), self.width(value))
            && t != v
        {
            self.out.push((
                "lint_740",
                self.db.exprs[value].span,
                format!(
                    "`{}` is {v} bits wide and is assigned to `{}`, which is {t}",
                    self.shown(value),
                    self.shown(target)
                ),
            ));
        }
        self.expr(value);
    }

    /// `e` and every expression below it.
    fn expr(&mut self, e: ExprId) {
        let mut all = Vec::new();
        self.db.walk_expr(e, &mut |x| all.push(x));
        for x in all {
            self.one(x);
        }
    }

    fn one(&mut self, e: ExprId) {
        let db = self.db;
        match &db.exprs[e].kind {
            ExprKind::Call { callee, args, .. } => {
                self.call(*callee, db.exprs[e].span);
                let DeclKind::Subprogram(Subprogram {
                    body: Body::Implicit(op),
                    ..
                }) = &db.decls[*callee].kind
                else {
                    return;
                };
                let actual = |i: usize| match args.get(i).map(|a| &a.actual) {
                    Some(Actual::Expr(x)) => Some(*x),
                    _ => None,
                };
                let (Some(l), Some(r)) = (actual(0), actual(1)) else {
                    return;
                };
                match op.family {
                    Family::Eq | Family::Neq => {
                        if let (Some(a), Some(b)) = (self.width(l), self.width(r))
                            && a != b
                        {
                            let always = op.family == Family::Neq;
                            self.out.push((
                                "lint_741",
                                db.exprs[e].span,
                                format!(
                                    "`{}` is {a} bits and is compared with `{}`, which is {b}, so the comparison is always {always}",
                                    self.shown(l),
                                    self.shown(r)
                                ),
                            ));
                        }
                    }
                    Family::Div | Family::Mod | Family::Rem => {
                        let zero = match db.value(r) {
                            Some(ConstValue::Int(0) | ConstValue::Phys(0)) => true,
                            Some(ConstValue::Real(v)) => *v == 0.0,
                            _ => false,
                        };
                        if zero {
                            self.out.push((
                                "lint_781",
                                db.exprs[r].span,
                                "Division by zero: evaluating this raises an error".to_string(),
                            ));
                        }
                    }
                    _ => {}
                }
            }
            ExprKind::Index { prefix, indices } => {
                let p = &db.exprs[*prefix];
                if !matches!(
                    p.kind,
                    ExprKind::Obj(_) | ExprKind::Field { .. } | ExprKind::Index { .. }
                ) {
                    return;
                }
                for (i, b) in indices.iter().zip(self.dims(p.subtype)) {
                    if let (Some(at), Some((l, d, r))) = (self.pos(*i), b) {
                        let (low, high) = if d == Direction::Downto {
                            (r, l)
                        } else {
                            (l, r)
                        };
                        if at < low || at > high {
                            self.out.push((
                                "lint_780",
                                db.exprs[*i].span,
                                format!(
                                    "Index `{}` is outside the range of `{}`: evaluating this raises an error",
                                    self.shown(*i),
                                    self.shown(*prefix)
                                ),
                            ));
                        }
                    }
                }
            }
            _ => {}
        }
    }
}
