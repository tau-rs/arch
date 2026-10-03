//! The stream-json parser over output recorded from the real CLI
//! (`experiments/record-stream.sh`, Claude Code 2.1.272).

use arch_driver::{StreamParser, TurnEvent, TurnResult};
use pretty_assertions::assert_eq;

fn parse(name: &str) -> (Vec<TurnEvent>, arch_driver::StreamStats) {
    let path = format!("{}/tests/fixtures/{name}.jsonl", env!("CARGO_MANIFEST_DIR"));
    let text = std::fs::read_to_string(&path).unwrap();
    let mut parser = StreamParser::default();
    let events = text.lines().flat_map(|l| parser.line(l)).collect();
    (events, parser.stats())
}

fn result(events: &[TurnEvent]) -> &TurnResult {
    match events.last() {
        Some(TurnEvent::Result(r)) => r,
        other => panic!("last event is not a result: {other:?}"),
    }
}

#[test]
fn text_turn_starts_says_and_ends() {
    let (events, stats) = parse("text");
    assert!(matches!(
        &events[0],
        TurnEvent::Started { model, tools, .. } if model.starts_with("claude-haiku") && tools.contains(&"Edit".to_string())
    ));
    let texts: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            TurnEvent::Text { text, subagent } => Some((text.as_str(), subagent.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(texts, vec![("ready", None)]);
    let r = result(&events);
    assert!(r.ok);
    assert_eq!(r.subtype, "success");
    assert_eq!(r.text.as_deref(), Some("ready"));
    assert_eq!(r.num_turns, 1);
    assert!(r.cost_usd.unwrap() > 0.0);
    assert_eq!(stats.skipped, 0, "every line is JSON");
    assert_eq!(stats.results, 1);
}

#[test]
fn tool_calls_pair_with_results_and_hooks_are_seen() {
    let (events, _) = parse("tools");
    let calls: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            TurnEvent::ToolCall {
                id, name, input, ..
            } => Some((id.clone(), name.clone(), input.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(
        calls.iter().map(|c| c.1.as_str()).collect::<Vec<_>>(),
        ["Read", "Edit"]
    );
    assert_eq!(calls[1].2["file_path"], "/work/src/lib.rs");
    for (id, _, _) in &calls {
        assert!(
            events
                .iter()
                .any(|e| matches!(e, TurnEvent::ToolResult { id: rid, ok: true, .. } if rid == id)),
            "call {id} has a successful result"
        );
    }
    let hooks: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            TurnEvent::Hook {
                name, exit_code, ..
            } => Some((name.as_str(), *exit_code)),
            _ => None,
        })
        .collect();
    assert_eq!(
        hooks,
        [
            ("PostToolUse:Read", Some(0)),
            ("PreToolUse:Edit", Some(0)),
            ("PostToolUse:Edit", Some(0))
        ]
    );
}

#[test]
fn a_hook_denial_reaches_the_model_and_the_result() {
    let (events, _) = parse("denied");
    assert!(events.iter().any(|e| matches!(
        e,
        TurnEvent::Hook { name, exit_code: Some(2), output, .. }
            if name == "PreToolUse:Edit" && output.contains("outside element a3f9c2e1")
    )));
    let denied = events
        .iter()
        .find_map(|e| match e {
            TurnEvent::ToolResult {
                ok: false, content, ..
            } => Some(content.clone()),
            _ => None,
        })
        .expect("the Edit's result is an error");
    assert!(denied.contains("outside element a3f9c2e1"));
    let r = result(&events);
    assert!(r.ok, "a denied tool does not fail the turn");
    assert_eq!(r.denials, vec!["Edit".to_string()]);
}

#[test]
fn sub_agent_events_carry_their_parent() {
    let (events, _) = parse("subagent");
    let (id, kind) = events
        .iter()
        .find_map(|e| match e {
            TurnEvent::Subagent { id, kind, .. } => Some((id.clone(), kind.clone())),
            _ => None,
        })
        .expect("a sub-agent started");
    assert_eq!(kind, "general-purpose");
    assert!(events.iter().any(|e| matches!(
        e,
        TurnEvent::ToolCall { name, subagent: Some(p), .. } if name == "Glob" && *p == id
    )));
    assert!(events.iter().any(|e| matches!(
        e,
        TurnEvent::ToolCall { name, subagent: None, .. } if name == "Agent"
    )));
    assert!(result(&events).ok);
}

#[test]
fn resume_keeps_the_session_id() {
    let (text, _) = parse("text");
    let (resume, _) = parse("resume");
    let id = |events: &[TurnEvent]| match &events[0] {
        TurnEvent::Started { session_id, .. } => session_id.clone(),
        other => panic!("{other:?}"),
    };
    assert_eq!(id(&resume), result(&resume).session_id);
    assert_eq!(
        id(&text),
        id(&resume),
        "both files map the same recorded id"
    );
    assert_eq!(result(&resume).text.as_deref(), Some("again"));
}

#[test]
fn structured_output_is_in_the_result() {
    let (events, _) = parse("structured");
    let r = result(&events);
    let s = r.structured.as_ref().expect("structured output");
    assert_eq!(s["verdict"], "pass");
    assert!(s["reason"].as_str().unwrap().contains("greet"));
}

#[test]
fn max_turns_is_an_error_result() {
    let (events, _) = parse("max-turns");
    let r = result(&events);
    assert!(!r.ok);
    assert_eq!(r.subtype, "error_max_turns");
    assert_eq!(r.text, None);
}

#[test]
fn garbage_and_unknown_lines_are_counted_never_fatal() {
    let mut parser = StreamParser::default();
    assert!(parser.line("not json").is_empty());
    assert!(parser.line("").is_empty());
    assert!(
        parser
            .line(r#"{"type":"from_the_future","x":1}"#)
            .is_empty()
    );
    assert!(
        parser
            .line(r#"{"type":"assistant","message":{"content":"odd"}}"#)
            .is_empty()
    );
    let stats = parser.stats();
    assert_eq!(stats.lines, 3, "blank lines are not counted");
    assert_eq!(stats.skipped, 1);
    assert_eq!(stats.unknown, 1);
}
