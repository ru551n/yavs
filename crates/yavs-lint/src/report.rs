//! The report files: JSON, SARIF 2.1.0, JUnit, Code Climate (quality report) and SonarQube generic
//! issues. Console output is the ordinary diagnostic rendering of the command line.

use crate::Certainty;
use serde_json::{Value, json};
use std::fmt::Write as _;

/// One finding as a report needs it.
pub struct Entry {
    pub file: String,
    pub rule: &'static crate::Rule,
    pub line: usize,
    pub column: usize,
    pub error: bool,
    pub message: String,
}

/// What a run found: every file looked at, and the findings in order.
pub struct Reports<'a> {
    pub files: &'a [String],
    pub entries: &'a [Entry],
}

fn xml(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// A file name as a SARIF URI: relative to the working directory with `/`, or `file://`.
fn uri(name: &str) -> String {
    let path = std::path::Path::new(name);
    let relative = std::env::current_dir()
        .ok()
        .and_then(|cwd| path.strip_prefix(cwd).ok().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| path.to_path_buf());
    let text = relative.to_string_lossy().replace('\\', "/");
    let text = text.trim_start_matches("./");
    if relative.is_absolute() {
        format!(
            "file://{}{text}",
            if text.starts_with('/') { "" } else { "/" }
        )
    } else {
        text.to_owned()
    }
}

impl Reports<'_> {
    fn of<'b>(&'b self, file: &'b str) -> impl Iterator<Item = &'b Entry> {
        self.entries.iter().filter(move |e| e.file == file)
    }

    /// `{"files": [{"file_path", "violations": [{"rule", "linenumber", "severity", "solution"}]}]}`.
    pub fn json(&self) -> String {
        let files: Vec<Value> = self
            .files
            .iter()
            .map(|f| {
                let violations: Vec<Value> = self
                    .of(f)
                    .map(|e| {
                        json!({
                            "rule": e.rule.id,
                            "linenumber": e.line,
                            "severity": if e.error { "Error" } else { "Warning" },
                            "solution": e.message,
                        })
                    })
                    .collect();
                json!({ "file_path": f, "violations": violations })
            })
            .collect();
        pretty(&json!({ "files": files }))
    }

    /// One testcase per file, failing when the file has a finding.
    pub fn junit(&self) -> String {
        let failures = self
            .files
            .iter()
            .filter(|f| self.of(f).next().is_some())
            .count();
        let mut out = String::from("<?xml version=\"1.0\" ?>\n");
        let _ = writeln!(
            out,
            "<testsuite errors=\"0\" failures=\"{failures}\" tests=\"{}\" time=\"0\" name=\"yavs-lint\">",
            self.files.len()
        );
        for f in self.files {
            let _ = writeln!(out, "  <testcase name=\"{}\" time=\"0\">", xml(f));
            if self.of(f).next().is_some() {
                out.push_str("    <failure type=\"Failure\">\n");
                for e in self.of(f) {
                    let _ = writeln!(out, "      {}: {} : {}", e.rule.id, e.line, xml(&e.message));
                }
                out.push_str("    </failure>\n");
            }
            out.push_str("  </testcase>\n");
        }
        out.push_str("</testsuite>\n");
        out
    }

    /// Code Climate issues, as GitLab reads them.
    pub fn quality(&self) -> String {
        let issues: Vec<Value> = self
            .entries
            .iter()
            .map(|e| {
                let key = format!("{}:{}:{}:{}", e.file, e.rule.id, e.line, e.message);
                let fingerprint = format!(
                    "{:016x}{:016x}",
                    yavs_diag::digest(key.as_bytes()),
                    yavs_diag::digest(format!("{key}#").as_bytes())
                );
                let category = match e.rule.certainty {
                    Certainty::Definite => "Bug Risk",
                    Certainty::Policy => "Style",
                    _ => "Clarity",
                };
                json!({
                    "description": format!("{} :: {}", e.rule.id, e.message),
                    "fingerprint": fingerprint,
                    "severity": if e.error { "critical" } else { "minor" },
                    "categories": [category],
                    "location": { "path": e.file, "lines": { "begin": e.line } },
                })
            })
            .collect();
        pretty(&json!(issues))
    }

    /// SonarQube generic issues. Only a definite rule's finding is a bug; columns count from zero.
    pub fn sonarqube(&self) -> String {
        let issues: Vec<Value> = self
            .entries
            .iter()
            .map(|e| {
                json!({
                    "engineId": "yavs",
                    "ruleId": e.rule.id,
                    "type": if e.rule.certainty == Certainty::Definite { "BUG" } else { "CODE_SMELL" },
                    "severity": if e.error { "CRITICAL" } else { "MINOR" },
                    "primaryLocation": {
                        "message": e.message,
                        "filePath": e.file,
                        "textRange": {
                            "startLine": e.line,
                            "startColumn": e.column.saturating_sub(1),
                        },
                    },
                })
            })
            .collect();
        pretty(&json!({ "issues": issues }))
    }

    /// SARIF 2.1.0, one result per finding; the class of a rule is its tag.
    pub fn sarif(&self) -> String {
        let mut seen: Vec<&crate::Rule> = self.entries.iter().map(|e| e.rule).collect();
        seen.sort_by_key(|r| r.id);
        seen.dedup_by_key(|r| r.id);
        let rules: Vec<Value> = seen
            .iter()
            .map(|r| {
                json!({
                    "id": r.id,
                    "name": r.id,
                    "shortDescription": { "text": r.summary },
                    "properties": { "tags": [r.certainty.name()] },
                })
            })
            .collect();
        let results: Vec<Value> = self
            .entries
            .iter()
            .map(|e| {
                json!({
                    "ruleId": e.rule.id,
                    "level": if e.error { "error" } else { "warning" },
                    "message": { "text": e.message },
                    "locations": [{
                        "physicalLocation": {
                            "artifactLocation": { "uri": uri(&e.file) },
                            "region": { "startLine": e.line, "startColumn": e.column },
                        },
                    }],
                })
            })
            .collect();
        pretty(&json!({
            "$schema": "https://json.schemastore.org/sarif-2.1.0.json",
            "version": "2.1.0",
            "runs": [{
                "tool": { "driver": { "name": "yavs", "rules": rules } },
                "results": results,
            }],
        }))
    }
}

fn pretty(v: &Value) -> String {
    serde_json::to_string_pretty(v).expect("a JSON value") + "\n"
}
