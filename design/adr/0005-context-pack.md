# ADR 0005 · Context pack: the standard input to planner, sub-agents, judge and Ask

- Date: 2026-10-02
- Status: accepted
- Source: `spec/arch-v1-spec.md` §13.5 (decisions of 2 Oct 2026, taken in the product chat)

## Context

Planner, sub-agents, judge and Ask each need the same picture of the unit to act within its architecture; without one definition each would read the repo differently and the judge could not be fair to the sub-agent.

## Decision

The **context pack** is the standard input to the planner, the sub-agents and the judge, and what Ask reads. It holds:
- the unit's map as text: areas, sides, ports, entries, cross-area links;
- the rules;
- the open findings;
- the area descriptions (`.arch/areas/<name>.md`);
- the element's position and what it may call.

## Consequences

- `arch-views` produces the pack; `arch-server` hands it to every driver call and to the MCP `ask` path, so an agent and a person see one view (spec §9).
- The judge ([ADR 0013](0013-judge.md)) verdicts against the same pack the sub-agent received.
- Area descriptions are the one free-text input a team has into every agent turn.
