//! The context pack for one element's sub-agent, or for the judge (ADR 0005, 0013).
//!
//! Reads the repository's `.arch/` files (areas, rules, allows, area descriptions) and hands
//! them with the facts and the plan to [`arch_views::context_pack`], which renders the text.
//! Writes nothing: the caller puts the text where the driver reads it.

use std::collections::BTreeMap;

use arch_facts::{ArchDir, ElementId, Facts, Plan};
use arch_views::{PackInput, context_pack};

/// The pack for `element`'s sub-agent, or for the judge when `element` is `None` (the judge
/// reads the whole plan instead of one element's position).
pub fn build(
    facts: &Facts,
    arch: &ArchDir,
    plan: &Plan,
    element: Option<&ElementId>,
) -> Result<String, crate::Error> {
    let areas = arch.read_areas()?;
    let rules = arch.read_rules()?;
    let allows = arch.read_allows()?;
    let mut descriptions = BTreeMap::new();
    for area in &areas.areas {
        if let Some(text) = arch.area_description(&area.name)? {
            descriptions.insert(area.name.clone(), text);
        }
    }
    Ok(context_pack(&PackInput {
        facts,
        areas: &areas,
        rules: &rules,
        allows: &allows,
        descriptions: &descriptions,
        plan,
        element,
    })?)
}
