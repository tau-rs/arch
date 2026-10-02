//! The committed `.arch/` files (spec §7; ADR 0003, 0004, 0021) and the notes archive.

use std::process::Command;

use arch_facts::*;
use pretty_assertions::assert_eq;

fn arch_in_tmp() -> (tempfile::TempDir, ArchDir) {
    let tmp = tempfile::tempdir().unwrap();
    let arch = ArchDir::of_repo(tmp.path());
    arch.create().unwrap();
    (tmp, arch)
}

#[test]
fn layout_is_created_and_absent_files_read_as_defaults() {
    let (_tmp, arch) = arch_in_tmp();
    assert!(arch.exists());
    assert!(arch.board_path().is_file());
    assert_eq!(arch.read_areas().unwrap(), Areas::default());
    assert_eq!(arch.read_rules().unwrap(), Rules::default());
    assert_eq!(arch.read_allows().unwrap(), Allows::default());
    assert_eq!(arch.sessions().unwrap(), vec![]);
    assert_eq!(arch.area_description("ship").unwrap(), None);
}

#[test]
fn areas_rules_allows_round_trip_as_toml() {
    let (_tmp, arch) = arch_in_tmp();
    let mut areas = Areas {
        rule: Some(ColumnRule::Hexagon),
        main_bin: Some("smallsvc".into()),
        areas: vec![],
    };
    areas.set(AreaOverride {
        name: "ship".into(),
        paths: vec!["src/ship/**".into()],
        side: Some(Column::Domain),
        order: Some(1),
    });
    arch.write_areas(&areas).unwrap();
    assert_eq!(arch.read_areas().unwrap(), areas);
    assert!(
        std::fs::read_to_string(arch.areas_path())
            .unwrap()
            .contains("[[area]]")
    );

    let rules = Rules::v1_template();
    arch.write_rules(&rules).unwrap();
    assert_eq!(arch.read_rules().unwrap(), rules);

    let mut allows = Allows::default();
    allows.add(Allow {
        site: "src/pg.rs::dequeue".into(),
        rule: "cycle".into(),
        target: None,
        reason: "known".into(),
        by: "t".into(),
        at: Some(now()),
    });
    arch.write_allows(&allows).unwrap();
    assert_eq!(arch.read_allows().unwrap(), allows);

    arch.write_area_description("ship", "# ship\nShipping.")
        .unwrap();
    assert_eq!(
        arch.area_description("ship").unwrap().as_deref(),
        Some("# ship\nShipping.")
    );
}

#[test]
fn a_bad_file_names_itself() {
    let (_tmp, arch) = arch_in_tmp();
    std::fs::write(arch.rules_path(), "this is not toml = = =").unwrap();
    let err = arch.read_rules().unwrap_err();
    assert!(
        matches!(err, Error::Format { ref path, .. } if path.ends_with("rules")),
        "{err}"
    );
}

fn sample_plan(id: &str) -> Plan {
    let mut plan = Plan::new(SessionId::new(id), "ship, pay, notify");
    let e1 = plan
        .add_element("add the ship port", "src/ship.rs")
        .id
        .clone();
    let e2 = plan
        .add_element("wire the notify adapter", "src/notify.rs")
        .id
        .clone();
    plan.groups.push(Group {
        name: "g1".into(),
        elements: vec![e1, e2],
        gate: Gate {
            commands: vec!["cargo test".into()],
            ..Gate::default()
        },
    });
    plan
}

#[test]
fn session_folder_holds_plan_thread_and_records() {
    let (_tmp, arch) = arch_in_tmp();
    let id = SessionId::new("s-2026-10-02-a");
    let dir = arch.session(&id);
    assert!(!dir.exists());
    assert_eq!(
        dir.read_plan().unwrap(),
        None,
        "plan · none before Accept (ADR 0022)"
    );

    let plan = sample_plan(id.as_str());
    dir.write_plan(&plan).unwrap();
    assert_eq!(dir.read_plan().unwrap(), Some(plan.clone()));
    let e1 = plan.elements[0].id.clone();

    dir.append_thread(&ThreadEntry::new(
        ThreadAuthor::Planner,
        ThreadEvent::Text {
            text: "drafting".into(),
        },
    ))
    .unwrap();
    dir.append_thread(&ThreadEntry::new(
        ThreadAuthor::Planner,
        ThreadEvent::Changed { what: None },
    ))
    .unwrap();
    let mut from_driver = ThreadEntry::new(
        ThreadAuthor::Agent {
            element: Some(e1.clone()),
        },
        ThreadEvent::ToolCall {
            tool: "read".into(),
            summary: "src/ship.rs".into(),
        },
    );
    from_driver.driver = Some(DriverPointer {
        driver: "claude-code".into(),
        session_id: "abc".into(),
        transcript_path: None,
    });
    dir.append_thread(&from_driver).unwrap();
    let thread = dir.read_thread().unwrap();
    assert_eq!(thread.len(), 3);
    assert_eq!(thread[2], from_driver);
    assert_eq!(
        std::fs::read_to_string(dir.thread_path())
            .unwrap()
            .lines()
            .count(),
        3,
        "one JSON object per line"
    );

    let r1 = dir
        .write_record(
            RecordKind::GateOutput {
                group: "g1".into(),
                command: "cargo test".into(),
                exit_code: 0,
                output: "ok".into(),
            },
            vec![Witness::Tool {
                tool: "cargo test".into(),
                output_sha256: ContentHash::of_str("test result: ok").0,
            }],
        )
        .unwrap();
    let r2 = dir
        .write_record(
            RecordKind::JudgeVerdict {
                element: e1,
                verdict: Verdict::Pass,
                reason: "realized".into(),
            },
            vec![],
        )
        .unwrap();
    assert_eq!((r1.seq, r2.seq), (1, 2));
    assert_eq!(dir.read_records().unwrap(), vec![r1, r2]);
    assert!(dir.records_dir().join("0001-gate-output.toml").is_file());
    assert!(dir.records_dir().join("0002-judge-verdict.toml").is_file());

    assert_eq!(arch.sessions().unwrap(), vec![id]);
    let files: Vec<_> = dir
        .files()
        .unwrap()
        .into_iter()
        .map(|(p, _)| p.to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        files,
        vec![
            "plan.toml",
            "records/0001-gate-output.toml",
            "records/0002-judge-verdict.toml",
            "thread.jsonl"
        ]
    );
}

fn git(repo: &std::path::Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

#[test]
fn archive_moves_the_session_folder_to_refs_notes_arch_and_restores_it() {
    let (tmp, arch) = arch_in_tmp();
    let repo = tmp.path();
    git(repo, &["init", "-q", "-b", "main"]);
    // The notes archive commits as whoever runs arch; CI runners have no global identity.
    git(repo, &["config", "user.name", "t"]);
    git(repo, &["config", "user.email", "t@x"]);
    git(
        repo,
        &[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@x",
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "merge",
        ],
    );
    let merge = git(repo, &["rev-parse", "HEAD"]);

    let id = SessionId::new("s1");
    let dir = arch.session(&id);
    let plan = sample_plan("s1");
    dir.write_plan(&plan).unwrap();
    dir.append_thread(&ThreadEntry::new(
        ThreadAuthor::You,
        ThreadEvent::Text { text: "go".into() },
    ))
    .unwrap();
    dir.write_record(
        RecordKind::Override {
            what: "gate g1".into(),
            reason: "flaky".into(),
            by: "t".into(),
        },
        vec![],
    )
    .unwrap();
    let before = dir.files().unwrap();

    assert_eq!(Archive::read_note(repo, &merge).unwrap(), None);
    let archive = Archive::move_to_notes(&dir, repo, &merge).unwrap();
    assert!(
        !dir.exists(),
        "main's tree never holds session folders (ADR 0003)"
    );
    assert_eq!(archive.files.len(), 3);
    assert!(git(repo, &["notes", "--ref", NOTES_REF, "list"]).contains(&merge));

    let read = Archive::read_note(repo, &merge).unwrap().unwrap();
    assert_eq!(read, archive);
    read.restore_to(&dir).unwrap();
    assert_eq!(dir.files().unwrap(), before);
    assert_eq!(dir.read_plan().unwrap(), Some(plan));
}
