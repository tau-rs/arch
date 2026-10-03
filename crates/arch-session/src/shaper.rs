//! The core shaper (spec §8): elements grouped by dependency, one gate per group.
//!
//! Groups are the topological layers over `depends_on`: group 1 holds the elements that depend
//! on nothing, group n those whose dependencies all sit in earlier groups. Inside a group the
//! plan's order is kept. Every group gets the same gate: the project's test command, `arch
//! check` and the judge, with a fix-round budget of 2 ([`Gate::default`]).
//!
//! ```text
//! chain    E1 → E2 → E3      group 1: E1 · group 2: E2 · group 3: E3
//! diamond  E1 → E2, E3 → E4  group 1: E1 · group 2: E2 E3 · group 3: E4
//! none     E1 E2 E3 E4       group 1: E1 E2 E3 E4
//! ```

use std::collections::HashSet;

use arch_facts::{ElementId, Gate, Group, Plan};

use crate::Error;

/// The plan's groups, one gate each; `test_command` is the gate's command (none when empty).
pub fn shape(plan: &Plan, test_command: &str) -> Result<Vec<Group>, Error> {
    let known: HashSet<&ElementId> = plan.elements.iter().map(|e| &e.id).collect();
    for e in &plan.elements {
        if let Some(missing) = e.depends_on.iter().find(|d| !known.contains(d)) {
            return Err(Error::UnknownDependency {
                element: e.label.clone(),
                depends_on: missing.to_string(),
            });
        }
    }
    let gate = Gate {
        commands: if test_command.trim().is_empty() {
            vec![]
        } else {
            vec![test_command.to_string()]
        },
        ..Gate::default()
    };
    let mut placed: HashSet<&ElementId> = HashSet::new();
    let mut left: Vec<_> = plan.elements.iter().collect();
    let mut groups = vec![];
    while !left.is_empty() {
        let (ready, rest): (Vec<_>, Vec<_>) = left
            .into_iter()
            .partition(|e| e.depends_on.iter().all(|d| placed.contains(d)));
        if ready.is_empty() {
            return Err(Error::Cycle {
                elements: rest.iter().map(|e| e.label.clone()).collect(),
            });
        }
        placed.extend(ready.iter().map(|e| &e.id));
        groups.push(Group {
            name: format!("group {}", groups.len() + 1),
            elements: ready.iter().map(|e| e.id.clone()).collect(),
            gate: gate.clone(),
        });
        left = rest;
    }
    Ok(groups)
}

#[cfg(test)]
mod tests {
    use super::*;
    use arch_facts::SessionId;
    use pretty_assertions::assert_eq;

    /// A plan of `n` elements E1…En with `deps` as (element, depends on) label numbers.
    fn plan(n: usize, deps: &[(usize, usize)]) -> Plan {
        let mut p = Plan::new(SessionId::new("s1"), "ship it");
        for i in 1..=n {
            p.add_element(format!("do {i}"), format!("src/e{i}.rs"));
        }
        for &(e, d) in deps {
            let id = p.elements[d - 1].id.clone();
            p.elements[e - 1].depends_on.push(id);
        }
        p
    }

    fn labels(plan: &Plan, groups: &[Group]) -> Vec<Vec<String>> {
        groups
            .iter()
            .map(|g| {
                g.elements
                    .iter()
                    .map(|id| plan.element(id).unwrap().label.clone())
                    .collect()
            })
            .collect()
    }

    #[test]
    fn a_chain_is_one_group_per_link() {
        let p = plan(3, &[(2, 1), (3, 2)]);
        let g = shape(&p, "cargo test").unwrap();
        assert_eq!(labels(&p, &g), [vec!["E1"], vec!["E2"], vec!["E3"]]);
        assert_eq!(
            g.iter().map(|g| g.name.as_str()).collect::<Vec<_>>(),
            ["group 1", "group 2", "group 3"]
        );
    }

    #[test]
    fn a_diamond_puts_the_two_middles_together() {
        let p = plan(4, &[(2, 1), (3, 1), (4, 2), (4, 3)]);
        let g = shape(&p, "cargo test").unwrap();
        assert_eq!(labels(&p, &g), [vec!["E1"], vec!["E2", "E3"], vec!["E4"]]);
    }

    #[test]
    fn independent_elements_share_one_group_in_plan_order() {
        let p = plan(4, &[]);
        let g = shape(&p, "cargo test").unwrap();
        assert_eq!(labels(&p, &g), [vec!["E1", "E2", "E3", "E4"]]);
    }

    #[test]
    fn a_layer_keeps_plan_order_whatever_the_dependency_order() {
        // E1 depends on E3: E3 and E2 go first, in plan order.
        let p = plan(3, &[(1, 3)]);
        let g = shape(&p, "cargo test").unwrap();
        assert_eq!(labels(&p, &g), [vec!["E2", "E3"], vec!["E1"]]);
    }

    #[test]
    fn every_group_gets_the_core_gate() {
        let p = plan(2, &[(2, 1)]);
        let g = shape(&p, "cargo test --workspace").unwrap();
        for group in &g {
            assert_eq!(
                group.gate,
                Gate {
                    commands: vec!["cargo test --workspace".into()],
                    check: true,
                    judge: true,
                    fix_rounds: 2,
                }
            );
        }
        let g = shape(&p, "  ").unwrap();
        assert!(g[0].gate.commands.is_empty(), "no test command, no command");
    }

    #[test]
    fn an_empty_plan_has_no_group() {
        assert_eq!(shape(&plan(0, &[]), "cargo test").unwrap(), vec![]);
    }

    #[test]
    fn a_cycle_is_an_error_naming_its_elements() {
        let p = plan(3, &[(1, 2), (2, 1)]);
        let e = shape(&p, "cargo test").unwrap_err();
        assert!(matches!(&e, Error::Cycle { elements } if elements == &["E1", "E2"]));
        assert_eq!(e.to_string(), "plan: dependency cycle among E1, E2");
        let selfish = plan(1, &[(1, 1)]);
        assert!(matches!(
            shape(&selfish, "").unwrap_err(),
            Error::Cycle { .. }
        ));
    }

    #[test]
    fn an_unknown_dependency_is_an_error() {
        let mut p = plan(1, &[]);
        p.elements[0]
            .depends_on
            .push(ElementId::from_str_unchecked("deadbeef"));
        let e = shape(&p, "").unwrap_err();
        assert_eq!(
            e.to_string(),
            "plan: E1 depends on deadbeef, which is not in the plan"
        );
    }
}
