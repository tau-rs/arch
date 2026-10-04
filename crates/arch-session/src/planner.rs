//! The planner (spec §6; ADR 0020): one read-only driver call in the repository that drafts the
//! elements of a change, with `--json-schema` for the answer. `--plan plan.toml` gives the same
//! elements by hand, for deterministic runs.
//!
//! ```toml
//! [[element]]
//! intention = "add Refund to the domain"
//! site = "src/domain/refund.rs"
//! files = ["src/domain/refund.rs"]
//!
//! [[element]]
//! intention = "RefundRepo port"
//! site = "src/domain/ports.rs"
//! files = ["src/domain/ports.rs"]
//! depends_on = ["E1"]
//! ```
//!
//! Either way the elements become a [`Plan`] draft: ids derived (ADR 0021), labels by position,
//! `depends_on` labels turned into ids. Groups are left to Accept's shaper.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use arch_driver::{Context, Driver, Task, TurnEvent};
use arch_facts::{Plan, SessionId, ThreadAuthor, ThreadEntry};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::Error;
use crate::engine::thread_entry;

/// The fixed prompt.
pub const PROMPT: &str = include_str!("../prompts/planner.md");

/// The planner's tools: it reads, it never writes.
pub const TOOLS: &[&str] = &["Read", "Glob", "Grep"];

/// One element as the planner (or `plan.toml`) gives it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Planned {
    /// What to do.
    pub intention: String,
    /// Where.
    pub site: String,
    /// The files it may write.
    #[serde(default)]
    pub files: Vec<PathBuf>,
    /// The labels (`E1`) of the elements it depends on.
    #[serde(default)]
    pub depends_on: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PlanFile {
    #[serde(rename = "element", default)]
    elements: Vec<Planned>,
}

/// The `--json-schema` of the answer: `{elements: [{intention, site, files, depends_on}]}`.
pub fn schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "elements": {
                "type": "array",
                "minItems": 1,
                "items": {
                    "type": "object",
                    "properties": {
                        "intention": { "type": "string" },
                        "site": { "type": "string" },
                        "files": { "type": "array", "items": { "type": "string" } },
                        "depends_on": {
                            "type": "array",
                            "items": { "type": "string", "description": "an earlier element's label, E1, E2, …" }
                        }
                    },
                    "required": ["intention", "site", "files", "depends_on"],
                    "additionalProperties": false
                }
            }
        },
        "required": ["elements"],
        "additionalProperties": false
    })
}

/// The elements of a hand-written `plan.toml`.
pub fn read_plan_file(path: &Path) -> Result<Vec<Planned>, Error> {
    let text = std::fs::read_to_string(path).map_err(|source| Error::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let file: PlanFile = toml::from_str(&text)
        .map_err(|e| Error::Plan(format!("{}: {}", path.display(), e.message())))?;
    if file.elements.is_empty() {
        return Err(Error::Plan(format!("{}: no [[element]]", path.display())));
    }
    Ok(file.elements)
}

/// The draft for `session`: ids derived, labels by position, dependencies by label.
pub fn draft(session: &SessionId, intention: &str, elements: &[Planned]) -> Result<Plan, Error> {
    let mut plan = Plan::new(session.clone(), intention);
    for p in elements {
        plan.add_element(&p.intention, &p.site);
        plan.elements.last_mut().expect("just added").files = p.files.clone();
    }
    let ids: HashMap<String, _> = plan
        .elements
        .iter()
        .map(|e| (e.label.clone(), e.id.clone()))
        .collect();
    for (i, p) in elements.iter().enumerate() {
        let label = plan.elements[i].label.clone();
        for dep in &p.depends_on {
            let id = ids.get(dep).ok_or_else(|| Error::UnknownDependency {
                element: label.clone(),
                depends_on: dep.clone(),
            })?;
            plan.elements[i].depends_on.push(id.clone());
        }
    }
    Ok(plan)
}

/// What the planner is given.
pub struct Planning<'a> {
    /// The repository.
    pub repo: &'a Path,
    /// The person's sentence.
    pub intention: &'a str,
    /// The model, when not the CLI's default.
    pub model: Option<String>,
}

/// Run the planner: one fresh read-only driver session. Returns the elements and the planner's
/// thread (the filtered stream, ADR 0003).
pub fn run(
    driver: &mut dyn Driver,
    p: &Planning<'_>,
    context: &Context,
) -> Result<(Vec<Planned>, Vec<ThreadEntry>), Error> {
    let task = Task {
        prompt: PROMPT.replace("{intention}", p.intention),
        cwd: p.repo.to_path_buf(),
        tools: TOOLS.iter().map(|t| t.to_string()).collect(),
        allowed_tools: TOOLS.iter().map(|t| t.to_string()).collect(),
        output_schema: Some(schema()),
        model: p.model.clone(),
        max_turns: None,
    };
    let turn = driver.start(&task, context)?;
    let mut thread = vec![];
    let mut names = HashMap::new();
    let mut structured = None;
    let mut subtype = String::new();
    for event in turn {
        let event = event?;
        if let Some(entry) = thread_entry(ThreadAuthor::Planner, &event, &mut names) {
            thread.push(entry);
        }
        if let TurnEvent::Result(r) = event {
            subtype = r.subtype;
            structured = r.structured;
        }
    }
    let elements = structured
        .and_then(|s| serde_json::from_value::<Vec<Planned>>(s["elements"].clone()).ok())
        .filter(|e| !e.is_empty())
        .ok_or_else(|| Error::Plan(format!("the planner gave no elements ({subtype})")))?;
    Ok((elements, thread))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn planned(intention: &str, deps: &[&str]) -> Planned {
        Planned {
            intention: intention.into(),
            site: format!("src/{intention}.rs"),
            files: vec![format!("src/{intention}.rs").into()],
            depends_on: deps.iter().map(|d| d.to_string()).collect(),
        }
    }

    #[test]
    fn labels_become_ids_and_an_unknown_label_is_an_error() {
        let s = SessionId::new("3c9e1f0a");
        let plan = draft(&s, "refunds", &[planned("a", &[]), planned("b", &["E1"])]).unwrap();
        assert_eq!(plan.elements[1].label, "E2");
        assert_eq!(
            plan.elements[1].depends_on,
            vec![plan.elements[0].id.clone()]
        );
        assert_eq!(plan.elements[0].files, vec![PathBuf::from("src/a.rs")]);
        assert!(plan.groups.is_empty(), "Accept shapes");

        let e = draft(&s, "refunds", &[planned("a", &["E7"])]).unwrap_err();
        assert_eq!(
            e.to_string(),
            "plan: E1 depends on E7, which is not in the plan"
        );
    }

    #[test]
    fn the_schema_is_closed_and_asks_for_every_field() {
        let s = schema();
        let item = &s["properties"]["elements"]["items"];
        assert_eq!(item["additionalProperties"], false);
        assert_eq!(item["required"].as_array().unwrap().len(), 4);
        assert!(PROMPT.contains("You do not write or change any file"));
    }

    #[test]
    fn a_plan_file_reads_and_refuses_unknown_keys() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("plan.toml");
        std::fs::write(
            &p,
            "[[element]]\nintention = \"a\"\nsite = \"src/a.rs\"\nfiles = [\"src/a.rs\"]\n\n\
             [[element]]\nintention = \"b\"\nsite = \"src/b.rs\"\ndepends_on = [\"E1\"]\n",
        )
        .unwrap();
        let elements = read_plan_file(&p).unwrap();
        assert_eq!(elements.len(), 2);
        assert_eq!(elements[1].depends_on, ["E1"]);

        std::fs::write(
            &p,
            "[[element]]\nintention = \"a\"\nsite = \"x\"\nid = \"z\"\n",
        )
        .unwrap();
        assert!(
            read_plan_file(&p)
                .unwrap_err()
                .to_string()
                .contains("unknown field")
        );
        std::fs::write(&p, "").unwrap();
        assert!(
            read_plan_file(&p)
                .unwrap_err()
                .to_string()
                .contains("no [[element]]")
        );
    }
}
