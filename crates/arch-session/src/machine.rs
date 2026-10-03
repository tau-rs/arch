//! The session state machine (`handoff-arch.md` §2; spec §6): one explicit table of the legal
//! edges over [`SessionState`]. Every state change goes through [`next`]; anything not in the
//! table is an [`Error::IllegalTransition`].
//!
//! ```text
//! Planning ─delegate─▶ Running ─ask─▶ Asks ─answer─▶ Running
//!     └─save plan─▶ Yours      ─denied─▶ Deviation ─typology─▶ Running
//!                              ─group done─▶ Gate ─passed · fix round─▶ Running
//!                                              ├─budget spent─▶ GateFailed ─one more · re-plan · accept as is─▶ Running
//!                                              └─last passed─▶ Done ◀─accept as is (last group)─┘
//! Done ─PR created─▶ InReview ─merged─▶ Merged ─archived─▶ Archived
//! ```

use arch_facts::SessionState::{self, *};

use crate::Error;

/// What moves a session from one state to the next.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Trigger {
    /// Accept · delegate: the scheduler starts.
    Delegate,
    /// Save plan: a locked `you` session (spec §6).
    SavePlan,
    /// An agent called the `ask` tool.
    Ask,
    /// A person answered the ask.
    Answer,
    /// A write outside the element's scope was denied (ADR 0012).
    Denied,
    /// A person chose a deviation typology: back on the plan · update the plan · not this change.
    Typology,
    /// The running group's elements are all done: its gate runs.
    GroupDone,
    /// The gate passed and another group follows.
    GatePassed,
    /// The gate failed with fix rounds left: the failing elements go back.
    FixRound,
    /// The gate failed with the fix-round budget spent: the four-door question.
    BudgetSpent,
    /// The last group's gate passed.
    LastGatePassed,
    /// Door 1: one more round, with a hint.
    OneMore,
    /// Door 4: re-plan (#49).
    Replan,
    /// Door 3: accept as is, with an override record, and another group follows.
    AcceptAsIs,
    /// Door 3 on the last group.
    AcceptAsIsLast,
    /// The PR was created.
    PrCreated,
    /// The forge merged it (a forge fact, ADR 0003).
    Merged,
    /// The folder moved to `refs/notes/arch` (an arch fact, ADR 0003).
    Archived,
}

/// Every legal edge: (from, trigger, to).
pub const TABLE: &[(SessionState, Trigger, SessionState)] = &[
    (Planning, Trigger::Delegate, Running),
    (Planning, Trigger::SavePlan, Yours),
    (Running, Trigger::Ask, Asks),
    (Asks, Trigger::Answer, Running),
    (Running, Trigger::Denied, Deviation),
    (Deviation, Trigger::Typology, Running),
    (Running, Trigger::GroupDone, Gate),
    (Gate, Trigger::GatePassed, Running),
    (Gate, Trigger::FixRound, Running),
    (Gate, Trigger::BudgetSpent, GateFailed),
    (Gate, Trigger::LastGatePassed, Done),
    (GateFailed, Trigger::OneMore, Running),
    (GateFailed, Trigger::Replan, Running),
    (GateFailed, Trigger::AcceptAsIs, Running),
    (GateFailed, Trigger::AcceptAsIsLast, Done),
    (Done, Trigger::PrCreated, InReview),
    (InReview, Trigger::Merged, Merged),
    (Merged, Trigger::Archived, Archived),
];

/// The state `trigger` moves a session in `from` to, or [`Error::IllegalTransition`].
pub fn next(from: SessionState, trigger: Trigger) -> Result<SessionState, Error> {
    TABLE
        .iter()
        .find(|(f, t, _)| *f == from && *t == trigger)
        .map(|(_, _, to)| *to)
        .ok_or(Error::IllegalTransition { from, trigger })
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    const STATES: [SessionState; 11] = [
        Planning, Yours, Running, Asks, Deviation, Gate, GateFailed, Done, InReview, Merged,
        Archived,
    ];
    const TRIGGERS: [Trigger; 18] = [
        Trigger::Delegate,
        Trigger::SavePlan,
        Trigger::Ask,
        Trigger::Answer,
        Trigger::Denied,
        Trigger::Typology,
        Trigger::GroupDone,
        Trigger::GatePassed,
        Trigger::FixRound,
        Trigger::BudgetSpent,
        Trigger::LastGatePassed,
        Trigger::OneMore,
        Trigger::Replan,
        Trigger::AcceptAsIs,
        Trigger::AcceptAsIsLast,
        Trigger::PrCreated,
        Trigger::Merged,
        Trigger::Archived,
    ];

    /// The diagram of issue #47, edge by edge, written out apart from [`TABLE`].
    #[test]
    fn every_edge_of_the_diagram_is_legal() {
        let diagram = [
            (Planning, "Accept · delegate", Trigger::Delegate, Running),
            (Planning, "Save plan", Trigger::SavePlan, Yours),
            (Running, "ask tool", Trigger::Ask, Asks),
            (Asks, "answer", Trigger::Answer, Running),
            (
                Running,
                "out-of-scope write denied",
                Trigger::Denied,
                Deviation,
            ),
            (Deviation, "typology chosen", Trigger::Typology, Running),
            (Running, "group's elements done", Trigger::GroupDone, Gate),
            (Gate, "pass, next group", Trigger::GatePassed, Running),
            (Gate, "fail, fix round ≤ 2", Trigger::FixRound, Running),
            (Gate, "budget spent", Trigger::BudgetSpent, GateFailed),
            (GateFailed, "one more round", Trigger::OneMore, Running),
            (GateFailed, "re-plan", Trigger::Replan, Running),
            (
                GateFailed,
                "accept as is, next group",
                Trigger::AcceptAsIs,
                Running,
            ),
            (
                GateFailed,
                "accept as is (override record)",
                Trigger::AcceptAsIsLast,
                Done,
            ),
            (Gate, "last group passed", Trigger::LastGatePassed, Done),
            (Done, "PR created", Trigger::PrCreated, InReview),
            (InReview, "merged", Trigger::Merged, Merged),
            (Merged, "archived", Trigger::Archived, Archived),
        ];
        for (from, label, trigger, to) in diagram {
            assert_eq!(next(from, trigger).ok(), Some(to), "{from:?} --{label}-->");
        }
        assert_eq!(
            diagram.len(),
            TABLE.len(),
            "the table has no edge the diagram lacks"
        );
    }

    #[test]
    fn every_other_pair_is_an_illegal_transition() {
        let mut legal = 0;
        for from in STATES {
            for trigger in TRIGGERS {
                let in_table = TABLE.iter().any(|(f, t, _)| *f == from && *t == trigger);
                match next(from, trigger) {
                    Ok(_) => {
                        assert!(in_table);
                        legal += 1;
                    }
                    Err(Error::IllegalTransition {
                        from: f,
                        trigger: t,
                    }) => {
                        assert!(!in_table);
                        assert_eq!((f, t), (from, trigger));
                    }
                    Err(e) => panic!("unexpected {e}"),
                }
            }
        }
        assert_eq!(legal, TABLE.len(), "one edge per (state, trigger) pair");
    }

    #[test]
    fn the_error_names_state_and_trigger() {
        let e = next(Archived, Trigger::Delegate).unwrap_err();
        assert_eq!(
            e.to_string(),
            "a session in state Archived cannot take Delegate"
        );
    }
}
