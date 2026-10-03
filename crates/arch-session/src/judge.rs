//! The judge (ADR 0013): the same provider in a fresh invocation, a fixed prompt
//! (`prompts/judge.md`), read-only tools, and a structured per-element verdict. Its output schema
//! has no field for code, and the prompt forbids proposing any.

use std::collections::HashMap;

use arch_driver::{Context, Driver, Task, TurnEvent};
use arch_facts::{ElementId, Plan, Verdict};
use serde_json::{Value, json};

use crate::gate::{CommandRun, tail};
use crate::{Error, git};

/// The fixed prompt.
pub const PROMPT: &str = include_str!("../prompts/judge.md");

/// The judge's tools: it reads, it never writes.
pub const TOOLS: &[&str] = &["Read", "Glob", "Grep"];

/// One element's verdict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ElementVerdict {
    /// The element.
    pub element: ElementId,
    /// Pass or fail.
    pub verdict: Verdict,
    /// Why.
    pub reason: String,
}

/// The `--json-schema` of the answer: `{verdicts: [{element, verdict, reason}]}`, nothing else.
pub fn schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "verdicts": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "element": { "type": "string", "description": "the element's id" },
                        "verdict": { "type": "string", "enum": ["pass", "fail"] },
                        "reason": { "type": "string" }
                    },
                    "required": ["element", "verdict", "reason"],
                    "additionalProperties": false
                }
            }
        },
        "required": ["verdicts"],
        "additionalProperties": false
    })
}

/// What the judge is shown.
pub struct Judging<'a> {
    /// The plan.
    pub plan: &'a Plan,
    /// The group's elements.
    pub elements: &'a [ElementId],
    /// The gate's runs.
    pub runs: &'a [CommandRun],
    /// The worktree.
    pub worktree: &'a std::path::Path,
    /// The commit the session started from.
    pub base: &'a str,
    /// The model, when not the CLI's default.
    pub model: Option<String>,
}

/// The judge's prompt for this group.
pub fn prompt(j: &Judging<'_>) -> String {
    let elements = j
        .elements
        .iter()
        .filter_map(|id| j.plan.element(id))
        .map(|e| {
            let files: Vec<_> = e.files.iter().map(|f| f.display().to_string()).collect();
            format!(
                "- {} · id {} · {}\n  site: {} · files: {}",
                e.label,
                e.id,
                e.intention,
                e.site,
                files.join(", ")
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let gate = j
        .runs
        .iter()
        .map(|r| {
            format!(
                "### {} · exit {}\n\n```\n{}\n```",
                r.command,
                r.exit_code,
                tail(&r.output, 8 * 1024)
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    let mut diff = git(j.worktree, &["diff", j.base, "--", ".", ":(exclude).arch"])
        .unwrap_or_else(|e| format!("(no diff: {e})"));
    let untracked = git(
        j.worktree,
        &[
            "ls-files",
            "--others",
            "--exclude-standard",
            "--",
            ".",
            ":(exclude).arch",
        ],
    )
    .unwrap_or_default();
    if !untracked.is_empty() {
        diff.push_str("\n\nNew files not yet committed:\n");
        diff.push_str(&untracked);
    }
    PROMPT
        .replace("{elements}", &elements)
        .replace(
            "{gate}",
            if gate.is_empty() {
                "(no command)"
            } else {
                &gate
            },
        )
        .replace(
            "{diff}",
            &format!("```diff\n{}\n```", tail(&diff, 96 * 1024)),
        )
}

/// Run the judge: one fresh driver session, one verdict per element. An element the answer does
/// not name fails, so a silent judge never passes a gate. Returns the verdicts and the driver
/// session id.
pub fn run(
    driver: &mut dyn Driver,
    j: &Judging<'_>,
    context: &Context,
) -> Result<(Vec<ElementVerdict>, String), Error> {
    let task = Task {
        prompt: prompt(j),
        cwd: j.worktree.to_path_buf(),
        tools: TOOLS.iter().map(|t| t.to_string()).collect(),
        allowed_tools: TOOLS.iter().map(|t| t.to_string()).collect(),
        output_schema: Some(schema()),
        model: j.model.clone(),
        max_turns: None,
    };
    let turn = driver.start(&task, context)?;
    let session_id = turn.session_id().to_string();
    let mut structured = None;
    let mut subtype = String::new();
    for event in turn {
        if let TurnEvent::Result(r) = event? {
            subtype = r.subtype;
            structured = r.structured;
        }
    }
    let answered = parse(j.plan, structured.as_ref());
    let verdicts = j
        .elements
        .iter()
        .map(|id| {
            answered.get(id).cloned().unwrap_or_else(|| ElementVerdict {
                element: id.clone(),
                verdict: Verdict::Fail,
                reason: format!("the judge gave no verdict for this element ({subtype})"),
            })
        })
        .collect();
    Ok((verdicts, session_id))
}

/// The verdicts in a structured answer, keyed by element; an element named by label is found too.
fn parse(plan: &Plan, answer: Option<&Value>) -> HashMap<ElementId, ElementVerdict> {
    let mut out = HashMap::new();
    let Some(items) = answer.and_then(|a| a["verdicts"].as_array()) else {
        return out;
    };
    for item in items {
        let named = item["element"].as_str().unwrap_or("");
        let Some(e) = plan
            .elements
            .iter()
            .find(|e| e.id.as_str() == named || e.label == named)
        else {
            continue;
        };
        let verdict = match item["verdict"].as_str() {
            Some("pass") => Verdict::Pass,
            _ => Verdict::Fail,
        };
        out.insert(
            e.id.clone(),
            ElementVerdict {
                element: e.id.clone(),
                verdict,
                reason: item["reason"].as_str().unwrap_or("").to_string(),
            },
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use arch_facts::SessionId;

    #[test]
    fn the_schema_has_no_room_for_code() {
        let s = schema();
        let item = &s["properties"]["verdicts"]["items"];
        let fields: Vec<_> = item["properties"].as_object().unwrap().keys().collect();
        assert_eq!(fields, ["element", "reason", "verdict"]);
        assert_eq!(item["additionalProperties"], false);
        assert_eq!(s["additionalProperties"], false);
    }

    #[test]
    fn the_prompt_forbids_code() {
        assert!(PROMPT.contains("You do not write, fix or propose code"));
    }

    #[test]
    fn verdicts_are_read_by_id_or_label_and_anything_but_pass_fails() {
        let mut plan = Plan::new(SessionId::new("s"), "x");
        let e1 = plan.add_element("a", "x").id.clone();
        let e2 = plan.add_element("b", "y").id.clone();
        let answer = json!({ "verdicts": [
            { "element": e1.as_str(), "verdict": "pass", "reason": "done" },
            { "element": "E2", "verdict": "maybe", "reason": "?" },
            { "element": "E9", "verdict": "pass", "reason": "unknown element" },
        ]});
        let v = parse(&plan, Some(&answer));
        assert_eq!(v.len(), 2);
        assert_eq!(v[&e1].verdict, Verdict::Pass);
        assert_eq!(v[&e2].verdict, Verdict::Fail);
        assert!(parse(&plan, None).is_empty());
    }
}
