//! `yavs lint`: what can be proved about a design from its analysed HIR, before it is simulated.
//!
//! A rule reports what it can derive and nothing it would have to guess: when the evidence is
//! missing (a generic decides a width, a call may hide a `wait`), it is silent. [`RULES`] is the
//! one list: reporting, `--list-rules`, `--explain`, the configuration and the rule reference page
//! all read it. See `AGENTS.md` in this crate.

mod config;
mod report;
mod rules;

pub use config::{Config, Waivers, find_up};
pub use report::{Entry, Reports};

use yavs_diag::{Severity, Span};
use yavs_sem::{Db, UnitId};

/// How sure a rule is that what it reports is wrong.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Certainty {
    /// The program cannot do what it says. On by default.
    Definite,
    /// An exact fact whose significance depends on intent. Off by default.
    Advisory,
    /// Inferred design intent. Off by default.
    Experimental,
    /// A house convention. Off by default.
    Policy,
}

impl Certainty {
    /// The name of the class in `rule.group` and in reports.
    pub fn name(self) -> &'static str {
        match self {
            Certainty::Definite => "definite",
            Certainty::Advisory => "advisory",
            Certainty::Experimental => "experimental",
            Certainty::Policy => "policy",
        }
    }
}

/// One rule: what it is called, how sure it is, what it reports and what it needs to report it.
#[derive(Debug)]
pub struct Rule {
    pub id: &'static str,
    pub certainty: Certainty,
    pub summary: &'static str,
    pub explain: &'static str,
}

pub use rules::RULES;

/// The rule with this id.
pub fn rule(id: &str) -> Option<&'static Rule> {
    RULES.iter().find(|r| r.id == id)
}

/// What one rule found.
#[derive(Debug)]
pub struct Finding {
    pub rule: &'static Rule,
    pub severity: Severity,
    pub span: Span,
    pub message: String,
}

/// The findings of the enabled rules in `units`, in source order. Instances of generic units carry
/// the spans of the generic, so a finding is reported once per place.
pub fn lint(db: &Db, units: &[UnitId], cfg: &Config) -> Vec<Finding> {
    let mut found = rules::check(db, units);
    found.retain(|(id, _, _)| rule(id).is_some_and(|r| cfg.enabled(r)));
    found.sort_by_key(|(id, span, _)| (span.file.0, span.start, span.end, *id));
    found.dedup_by(|a, b| a.0 == b.0 && a.1 == b.1);
    found
        .into_iter()
        .map(|(id, span, message)| {
            let rule = rule(id).expect("kept above");
            Finding {
                rule,
                severity: cfg.severity(rule),
                span,
                message,
            }
        })
        .collect()
}
