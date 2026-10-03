//! Crates built in a temp directory: one whose manifest cargo rejects (ADR 0010: the facts are still
//! produced, guessed, with cargo's reason recorded), a workspace with a tool bin (ADR 0007), and
//! an actix service with scoped, wrapped routes.

use std::path::Path;

use arch_analyze::{Commits, Options, analyze};
use arch_facts::*;

fn write(root: &Path, files: &[(&str, &str)]) {
    for (path, text) in files {
        let p = root.join(path);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }
}

fn opts() -> Options {
    Options {
        repo_name: Some("t".into()),
        commit: Some("0".repeat(40)),
        commits: Commits::None,
        ..Default::default()
    }
}

#[test]
fn a_crate_cargo_rejects_still_yields_guessed_facts_with_the_reason() {
    let tmp = tempfile::tempdir().unwrap();
    write(
        tmp.path(),
        &[
            (
                "Cargo.toml",
                "[package]\nname = \"broken\"\nversion = \"0.1.0\"\nedition = \"2077\"\n",
            ),
            (
                "src/main.rs",
                "mod store;\nuse store::Store;\nfn main() {\n    let s = Store::open();\n    s.put(1);\n    this does not parse\n}\n",
            ),
            (
                "src/store.rs",
                "pub struct Store { n: u32 }\nimpl Store {\n    pub fn open() -> Self { Store { n: 0 } }\n    pub fn put(&self, _x: u32) {}\n}\n",
            ),
        ],
    );
    let f = analyze(tmp.path(), &opts()).unwrap();
    assert_eq!(f.repo.unit.main_target, "bin:broken");
    assert_eq!(f.analyzer.degraded.len(), 1);
    let reason = &f.analyzer.degraded[0].reason;
    assert!(
        reason.starts_with("syntax-level pass: not type-checked; cargo metadata:"),
        "{reason}"
    );
    assert!(
        f.items
            .iter()
            .any(|i| i.id == "broken[bin:broken]::store::impl Store::put#fn")
    );
    assert!(f.links.iter().all(|l| l.confidence == Confidence::Guessed));
    let calls: Vec<&str> = f
        .links
        .iter()
        .filter(|l| l.from == "broken[bin:broken]::main#fn" && l.kind == LinkKind::Calls)
        .filter_map(|l| match &l.to {
            Target::Item(i) => Some(i.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(
        calls,
        [
            "broken[bin:broken]::store::impl Store::open#fn",
            "broken[bin:broken]::store::impl Store::put#fn"
        ]
    );
}

#[test]
fn a_workspace_tool_bin_and_an_unreached_lib_are_not_analyzed() {
    let tmp = tempfile::tempdir().unwrap();
    let pkg = |name: &str, deps: &str| {
        format!(
            "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\n{deps}"
        )
    };
    write(
        tmp.path(),
        &[
            (
                "Cargo.toml",
                "[workspace]\nresolver = \"2\"\nmembers = [\"app\", \"core\", \"other\", \"xtask\"]\ndefault-members = [\"app\"]\n",
            ),
            (
                "app/Cargo.toml",
                &pkg("app", "core = { path = \"../core\" }\n"),
            ),
            ("app/src/main.rs", "fn main() { core::run(); }\n"),
            ("core/Cargo.toml", &pkg("core", "")),
            ("core/src/lib.rs", "pub fn run() {}\n"),
            ("other/Cargo.toml", &pkg("other", "")),
            ("other/src/lib.rs", "pub fn unused() {}\n"),
            ("xtask/Cargo.toml", &pkg("xtask", "")),
            ("xtask/src/main.rs", "fn main() {}\n"),
            ("app/examples/demo.rs", "fn main() {}\n"),
        ],
    );
    let f = analyze(tmp.path(), &opts()).unwrap();
    assert_eq!(f.repo.unit.main_target, "bin:app");
    let status: Vec<(&str, &CrateStatus)> = f
        .crates
        .iter()
        .map(|c| (c.name.as_str(), &c.status))
        .collect();
    assert_eq!(
        status,
        [
            ("app", &CrateStatus::Analyzed),
            ("core", &CrateStatus::Analyzed),
            (
                "other",
                &CrateStatus::NotAnalyzed("outside the unit's closure".into())
            ),
            ("xtask", &CrateStatus::NotAnalyzed("tool bin".into())),
        ]
    );
    assert!(f.crates[0].targets.contains(&"example:demo".to_string()));
    assert!(
        f.items
            .iter()
            .all(|i| i.crate_name == "app" || i.crate_name == "core"),
        "examples and tool bins yield no items"
    );
    // The bin calls into the workspace lib it depends on.
    assert!(f.links.iter().any(|l| l.from == "app[bin:app]::main#fn"
        && l.to == Target::Item("core::run#fn".into())
        && l.kind == LinkKind::Calls));
}

#[test]
fn actix_routes_carry_their_scope_and_wraps() {
    let tmp = tempfile::tempdir().unwrap();
    write(
        tmp.path(),
        &[
            (
                "Cargo.toml",
                "[package]\nname = \"svc\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\nactix-web = \"4\"\n",
            ),
            (
                "src/main.rs",
                "use actix_web::{web, App};\nmod routes;\nuse routes::{health, dashboard};\n\
                 fn app() {\n    App::new()\n        .wrap(TracingLogger::default())\n        .route(\"/health\", web::get().to(health))\n        \
                 .service(web::scope(\"/admin\").wrap(from_fn(reject_anonymous)).route(\"/dashboard\", web::get().to(dashboard)));\n}\nfn main() { app() }\n",
            ),
            // A module and its handler share a name, re-exported by a glob: both must resolve.
            (
                "src/routes.rs",
                "mod health;\npub use health::*;\npub async fn dashboard() {}\n#[post(\"/hook\")]\npub async fn hook() {}\n",
            ),
            ("src/routes/health.rs", "pub async fn health() {}\n"),
        ],
    );
    let f = analyze(tmp.path(), &opts()).unwrap();
    let ports: Vec<&str> = f.ports.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(ports, ["GET /admin/dashboard", "GET /health", "POST /hook"]);
    let dash = f
        .links
        .iter()
        .find(|l| {
            l.kind == LinkKind::Routes
                && l.to == Target::Item("svc[bin:svc]::routes::dashboard#fn".into())
        })
        .unwrap();
    assert_eq!(
        dash.reason.as_deref(),
        Some(
            "route `GET /admin/dashboard`; middleware, outermost first: TracingLogger → reject_anonymous"
        )
    );
    assert_eq!(
        f.entries
            .iter()
            .filter(|e| e.framework == Some(Framework::Actix))
            .count(),
        3
    );
    assert!(
        f.entries
            .iter()
            .any(|e| e.item == "svc[bin:svc]::routes::health::health#fn")
    );
}
