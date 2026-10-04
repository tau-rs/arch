//! Cargo reader: which packages and targets exist, which one is the unit, and its closure
//! (ADR 0007). Reads `cargo metadata --no-deps`; when cargo cannot answer, reads the manifests
//! by hand so the syntax-level pass still has a unit (ADR 0010).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, anyhow, bail};
use arch_facts::{Crate, CrateStatus};
use serde::Deserialize;

/// What a cargo target is built as.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum TargetKind {
    /// The package's library (including proc-macro libraries).
    Lib,
    /// A binary.
    Bin,
    /// An example.
    Example,
    /// An integration test.
    Test,
    /// A benchmark.
    Bench,
}

/// One cargo target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    /// Kind.
    pub kind: TargetKind,
    /// Target name as cargo knows it.
    pub name: String,
    /// Root source file, repository-relative.
    pub root: PathBuf,
}

impl Target {
    /// `lib`, `bin:<name>`, `example:<name>`…, as listed in [`Crate::targets`].
    pub fn label(&self) -> String {
        match self.kind {
            TargetKind::Lib => "lib".into(),
            TargetKind::Bin => format!("bin:{}", self.name),
            TargetKind::Example => format!("example:{}", self.name),
            TargetKind::Test => format!("test:{}", self.name),
            TargetKind::Bench => format!("bench:{}", self.name),
        }
    }
}

/// A declared dependency of a package.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dep {
    /// Package name on the registry (`actix-web`).
    pub package: String,
    /// The name the code spells (`actix_web`, or the rename).
    pub ident: String,
    /// 1-based line of the declaration in the manifest, or 1 when not found.
    pub line: u32,
    /// The workspace package it points at, for path dependencies inside the repository.
    pub local: bool,
    /// Enabled features, as declared.
    pub features: Vec<String>,
}

/// A workspace package.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Package {
    /// Package name.
    pub name: String,
    /// `Cargo.toml`, repository-relative.
    pub manifest: PathBuf,
    /// Its targets.
    pub targets: Vec<Target>,
    /// Its normal dependencies (dev and build dependencies are not part of the closure).
    pub deps: Vec<Dep>,
}

impl Package {
    /// The name the code spells for this package's library.
    pub fn ident(&self) -> String {
        self.name.replace('-', "_")
    }

    /// The library target, when the package has one.
    pub fn lib(&self) -> Option<&Target> {
        self.targets.iter().find(|t| t.kind == TargetKind::Lib)
    }
}

/// A target of the unit: where the syntax pass starts walking.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnitTarget {
    /// Index into [`UnitPlan::packages`].
    pub package: usize,
    /// The target.
    pub target: Target,
}

/// The cargo reader's answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnitPlan {
    /// Workspace packages, by name.
    pub packages: Vec<Package>,
    /// `bin:<name>` or `lib`: the unit's main target.
    pub main_target: String,
    /// The package that owns the main target.
    pub main_package: usize,
    /// Targets analyzed: the main target, its package's lib, and the libs of the workspace
    /// packages it reaches through path dependencies.
    pub unit: Vec<UnitTarget>,
    /// Set when `cargo metadata` failed and the manifests were read by hand: cargo's error.
    pub cargo_error: Option<String>,
}

impl UnitPlan {
    /// `package` and the workspace packages it depends on, transitively: the crates its code can
    /// name. The syntax pass looks only into these, as rustc does.
    pub fn closure(&self, package: usize) -> BTreeSet<usize> {
        let mut seen = BTreeSet::new();
        let mut queue = vec![package];
        while let Some(p) = queue.pop() {
            if !seen.insert(p) {
                continue;
            }
            for d in &self.packages[p].deps {
                if let Some(q) = self.packages.iter().position(|q| q.name == d.package) {
                    queue.push(q);
                }
            }
        }
        seen
    }

    /// The packages whose [`UnitPlan::closure`] holds one of `changed`: what a declaration
    /// change in them can reach. Every other package's facts stay what they were.
    pub fn dependents(&self, changed: &BTreeSet<usize>) -> BTreeSet<usize> {
        (0..self.packages.len())
            .filter(|p| !self.closure(*p).is_disjoint(changed))
            .collect()
    }

    /// The crates as facts: analyzed, or not analyzed with the reason (ADR 0007).
    pub fn crates(&self) -> Vec<Crate> {
        let analyzed: BTreeSet<usize> = self.unit.iter().map(|u| u.package).collect();
        let mut out: Vec<Crate> = self
            .packages
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let mut targets: Vec<String> = p.targets.iter().map(Target::label).collect();
                targets.sort();
                let status = if analyzed.contains(&i) {
                    CrateStatus::Analyzed
                } else if p.lib().is_none() {
                    CrateStatus::NotAnalyzed("tool bin".into())
                } else {
                    CrateStatus::NotAnalyzed("outside the unit's closure".into())
                };
                Crate {
                    name: p.name.clone(),
                    status,
                    targets,
                }
            })
            .collect();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }
}

#[derive(Deserialize)]
struct Metadata {
    packages: Vec<MetaPackage>,
    workspace_root: PathBuf,
    #[serde(default)]
    workspace_default_members: Vec<String>,
}

#[derive(Deserialize)]
struct MetaPackage {
    id: String,
    name: String,
    manifest_path: PathBuf,
    targets: Vec<MetaTarget>,
    dependencies: Vec<MetaDep>,
}

#[derive(Deserialize)]
struct MetaTarget {
    kind: Vec<String>,
    name: String,
    src_path: PathBuf,
}

#[derive(Deserialize)]
struct MetaDep {
    name: String,
    rename: Option<String>,
    kind: Option<String>,
    path: Option<PathBuf>,
    #[serde(default)]
    features: Vec<String>,
}

/// Read the unit of the repository at `root`. `main_bin` is the `areas.toml` override.
pub fn read(root: &Path, main_bin: Option<&str>) -> Result<UnitPlan> {
    if !root.join("Cargo.toml").is_file() {
        bail!("no Cargo.toml in {}", root.display());
    }
    let (packages, cargo_error) = match metadata(root) {
        Ok(p) => (p, None),
        Err(e) => {
            let reason = format!("{e:#}");
            (
                manifests(root).with_context(|| reason.clone())?,
                Some(reason),
            )
        }
    };
    plan(packages, main_bin, cargo_error)
}

fn rel(root: &Path, p: &Path) -> PathBuf {
    p.strip_prefix(root).unwrap_or(p).to_path_buf()
}

fn metadata(root: &Path) -> Result<Vec<Package>> {
    let out = Command::new("cargo")
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .current_dir(root)
        .output()
        .context("running cargo metadata")?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let line = err
            .lines()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("cargo metadata failed");
        bail!("cargo metadata: {}", line.trim());
    }
    let meta: Metadata = serde_json::from_slice(&out.stdout).context("cargo metadata output")?;
    // cargo canonicalizes; do the same so paths under the root strip cleanly.
    let ws = meta
        .workspace_root
        .canonicalize()
        .unwrap_or(meta.workspace_root.clone());
    let root = root.canonicalize().unwrap_or(root.to_path_buf());
    let base = if ws.starts_with(&root) {
        root.clone()
    } else {
        ws
    };
    let dirs: BTreeMap<PathBuf, String> = meta
        .packages
        .iter()
        .filter_map(|p| Some((p.manifest_path.parent()?.to_path_buf(), p.name.clone())))
        .collect();
    let default: Vec<&str> = meta
        .workspace_default_members
        .iter()
        .map(String::as_str)
        .collect();
    let mut metas = meta.packages.iter().collect::<Vec<_>>();
    // Default members first, so "first bin" follows what `cargo run` would pick.
    metas.sort_by_key(|p| (!default.contains(&p.id.as_str()), p.name.clone()));
    let mut packages = Vec::new();
    for p in metas {
        let text = std::fs::read_to_string(&p.manifest_path).unwrap_or_default();
        let mut targets = Vec::new();
        for t in &p.targets {
            let kind = match t.kind.first().map(String::as_str) {
                Some("bin") => TargetKind::Bin,
                Some("example") => TargetKind::Example,
                Some("test") => TargetKind::Test,
                Some("bench") => TargetKind::Bench,
                Some("custom-build") | None => continue,
                Some(_) => TargetKind::Lib,
            };
            targets.push(Target {
                kind,
                name: t.name.clone(),
                root: rel(&base, &t.src_path),
            });
        }
        let deps = p
            .dependencies
            .iter()
            .filter(|d| d.kind.is_none())
            .map(|d| Dep {
                package: d.name.clone(),
                ident: d
                    .rename
                    .clone()
                    .unwrap_or_else(|| d.name.clone())
                    .replace('-', "_"),
                line: dep_line(&text, d.rename.as_deref().unwrap_or(&d.name)),
                local: d.path.as_ref().is_some_and(|p| dirs.contains_key(p)),
                features: d.features.clone(),
            })
            .collect();
        packages.push(Package {
            name: p.name.clone(),
            manifest: rel(&base, &p.manifest_path),
            targets,
            deps,
        });
    }
    Ok(packages)
}

/// The 1-based line where `name` is declared as a dependency in a manifest.
fn dep_line(manifest: &str, name: &str) -> u32 {
    let mut in_deps = false;
    for (i, line) in manifest.lines().enumerate() {
        let l = line.trim();
        if l.starts_with('[') {
            if l.trim_matches(['[', ']'])
                .ends_with(&format!("dependencies.{name}"))
            {
                return i as u32 + 1;
            }
            in_deps = l.trim_matches(['[', ']']).ends_with("dependencies");
            continue;
        }
        if in_deps {
            let key = l
                .split(['=', '.'])
                .next()
                .unwrap_or("")
                .trim()
                .trim_matches('"');
            if key == name {
                return i as u32 + 1;
            }
        }
    }
    1
}

/// Manifest-only reading, for when cargo cannot resolve the workspace.
fn manifests(root: &Path) -> Result<Vec<Package>> {
    let top: toml::Table = read_toml(&root.join("Cargo.toml"))?;
    let mut dirs: Vec<PathBuf> = Vec::new();
    if top.contains_key("package") {
        dirs.push(PathBuf::new());
    }
    if let Some(members) = top
        .get("workspace")
        .and_then(|w| w.get("members"))
        .and_then(|m| m.as_array())
    {
        for m in members.iter().filter_map(|m| m.as_str()) {
            if let Some(prefix) = m.strip_suffix("/*") {
                let mut found: Vec<PathBuf> = std::fs::read_dir(root.join(prefix))
                    .into_iter()
                    .flatten()
                    .flatten()
                    .map(|e| e.path())
                    .filter(|p| p.join("Cargo.toml").is_file())
                    .map(|p| rel(root, &p))
                    .collect();
                found.sort();
                dirs.extend(found);
            } else if root.join(m).join("Cargo.toml").is_file() {
                dirs.push(PathBuf::from(m));
            }
        }
    }
    let mut packages = Vec::new();
    for dir in &dirs {
        let manifest = dir.join("Cargo.toml");
        let text = std::fs::read_to_string(root.join(&manifest)).unwrap_or_default();
        let t: toml::Table = read_toml(&root.join(&manifest))?;
        let Some(name) = t
            .get("package")
            .and_then(|p| p.get("name"))
            .and_then(|n| n.as_str())
        else {
            continue;
        };
        let mut targets = Vec::new();
        let lib = t
            .get("lib")
            .and_then(|l| l.get("path"))
            .and_then(|p| p.as_str())
            .unwrap_or("src/lib.rs");
        if root.join(dir).join(lib).is_file() {
            targets.push(Target {
                kind: TargetKind::Lib,
                name: name.replace('-', "_"),
                root: dir.join(lib),
            });
        }
        let bins = t
            .get("bin")
            .and_then(|b| b.as_array())
            .cloned()
            .unwrap_or_default();
        for b in &bins {
            let bname = b.get("name").and_then(|n| n.as_str()).unwrap_or(name);
            let path = b
                .get("path")
                .and_then(|p| p.as_str())
                .map(str::to_string)
                .unwrap_or_else(|| {
                    if root
                        .join(dir)
                        .join("src/bin")
                        .join(format!("{bname}.rs"))
                        .is_file()
                    {
                        format!("src/bin/{bname}.rs")
                    } else {
                        "src/main.rs".into()
                    }
                });
            targets.push(Target {
                kind: TargetKind::Bin,
                name: bname.into(),
                root: dir.join(path),
            });
        }
        if bins.is_empty() && root.join(dir).join("src/main.rs").is_file() {
            targets.push(Target {
                kind: TargetKind::Bin,
                name: name.into(),
                root: dir.join("src/main.rs"),
            });
        }
        let mut deps = Vec::new();
        if let Some(d) = t.get("dependencies").and_then(|d| d.as_table()) {
            for (key, v) in d {
                let package = v
                    .get("package")
                    .and_then(|p| p.as_str())
                    .unwrap_or(key)
                    .to_string();
                let features = v
                    .get("features")
                    .and_then(|f| f.as_array())
                    .map(|f| {
                        f.iter()
                            .filter_map(|x| x.as_str().map(str::to_string))
                            .collect()
                    })
                    .unwrap_or_default();
                let local = v
                    .get("path")
                    .and_then(|p| p.as_str())
                    .is_some_and(|p| root.join(dir).join(p).join("Cargo.toml").is_file());
                deps.push(Dep {
                    package,
                    ident: key.replace('-', "_"),
                    line: dep_line(&text, key),
                    local,
                    features,
                });
            }
        }
        packages.push(Package {
            name: name.into(),
            manifest,
            targets,
            deps,
        });
    }
    if packages.is_empty() {
        bail!("no package found in {}", root.display());
    }
    Ok(packages)
}

fn read_toml(path: &Path) -> Result<toml::Table> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    text.parse()
        .with_context(|| format!("parsing {}", path.display()))
}

fn plan(
    packages: Vec<Package>,
    main_bin: Option<&str>,
    cargo_error: Option<String>,
) -> Result<UnitPlan> {
    let bin_of = |p: &Package, name: Option<&str>| {
        p.targets
            .iter()
            .find(|t| t.kind == TargetKind::Bin && name.is_none_or(|n| n == t.name))
            .cloned()
    };
    let main = match main_bin {
        Some(name) => packages
            .iter()
            .enumerate()
            .find_map(|(i, p)| bin_of(p, Some(name)).map(|t| (i, t)))
            .ok_or_else(|| {
                anyhow!("areas.toml names main_bin `{name}`, which no package declares")
            })?,
        None => packages
            .iter()
            .enumerate()
            .find_map(|(i, p)| bin_of(p, None).map(|t| (i, t)))
            .or_else(|| {
                packages
                    .iter()
                    .enumerate()
                    .find_map(|(i, p)| p.lib().cloned().map(|t| (i, t)))
            })
            .ok_or_else(|| anyhow!("no bin or lib target in the workspace"))?,
    };
    let (main_package, main_target) = main;
    let mut unit = vec![UnitTarget {
        package: main_package,
        target: main_target.clone(),
    }];
    // Closure: the main package's own lib, then libs of workspace packages reached by path deps.
    let mut seen = BTreeSet::new();
    let mut queue = vec![main_package];
    while let Some(i) = queue.pop() {
        if !seen.insert(i) {
            continue;
        }
        if let Some(lib) = packages[i].lib()
            && *lib != main_target
        {
            unit.push(UnitTarget {
                package: i,
                target: lib.clone(),
            });
        }
        for d in packages[i].deps.iter().filter(|d| d.local) {
            if let Some(j) = packages.iter().position(|p| p.name == d.package) {
                queue.push(j);
            }
        }
    }
    unit.sort_by(|a, b| {
        (a.package, a.target.kind, &a.target.name).cmp(&(b.package, b.target.kind, &b.target.name))
    });
    Ok(UnitPlan {
        main_target: main_target.label(),
        main_package,
        packages,
        unit,
        cargo_error,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pkg(name: &str, targets: &[(TargetKind, &str)], local_deps: &[&str]) -> Package {
        Package {
            name: name.into(),
            manifest: PathBuf::from(format!("{name}/Cargo.toml")),
            targets: targets
                .iter()
                .map(|(k, n)| Target {
                    kind: *k,
                    name: (*n).into(),
                    root: PathBuf::from(format!("{name}/src/{n}.rs")),
                })
                .collect(),
            deps: local_deps
                .iter()
                .map(|d| Dep {
                    package: (*d).into(),
                    ident: (*d).into(),
                    line: 1,
                    local: true,
                    features: vec![],
                })
                .collect(),
        }
    }

    #[test]
    fn a_change_reaches_the_package_and_those_depending_on_it() {
        let lib = |n: &'static str| [(TargetKind::Lib, n)];
        let packages = vec![
            pkg("app", &[(TargetKind::Bin, "app")], &["leaf", "side"]),
            pkg("base", &lib("base"), &[]),
            pkg("leaf", &lib("leaf"), &["base"]),
            pkg("side", &lib("side"), &["base"]),
        ];
        let plan = plan(packages, None, None).unwrap();
        let set = |v: &[usize]| v.iter().copied().collect::<BTreeSet<usize>>();
        assert_eq!(plan.closure(0), set(&[0, 1, 2, 3]));
        assert_eq!(plan.closure(2), set(&[1, 2]));
        assert_eq!(plan.dependents(&set(&[2])), set(&[0, 2]));
        assert_eq!(plan.dependents(&set(&[1])), set(&[0, 1, 2, 3]));
        assert_eq!(plan.dependents(&set(&[0])), set(&[0]));
    }

    #[test]
    fn the_unit_is_the_first_bin_its_lib_and_the_path_dependencies_it_reaches() {
        let packages = vec![
            pkg(
                "app",
                &[
                    (TargetKind::Lib, "app"),
                    (TargetKind::Bin, "app"),
                    (TargetKind::Example, "demo"),
                ],
                &["core"],
            ),
            pkg("core", &[(TargetKind::Lib, "core")], &[]),
            pkg("other", &[(TargetKind::Lib, "other")], &[]),
            pkg("xtask", &[(TargetKind::Bin, "xtask")], &[]),
        ];
        let plan = plan(packages, None, None).unwrap();
        assert_eq!(plan.main_target, "bin:app");
        let labels: Vec<String> = plan
            .unit
            .iter()
            .map(|u| format!("{}/{}", plan.packages[u.package].name, u.target.label()))
            .collect();
        assert_eq!(labels, ["app/lib", "app/bin:app", "core/lib"]);
        let crates = plan.crates();
        assert_eq!(crates[0].status, CrateStatus::Analyzed);
        assert_eq!(crates[0].targets, ["bin:app", "example:demo", "lib"]);
        assert_eq!(
            crates[2].status,
            CrateStatus::NotAnalyzed("outside the unit's closure".into())
        );
        assert_eq!(
            crates[3].status,
            CrateStatus::NotAnalyzed("tool bin".into())
        );
    }

    #[test]
    fn areas_toml_can_name_the_bin_and_a_lib_only_workspace_falls_back_to_the_lib() {
        let packages = vec![
            pkg("a", &[(TargetKind::Bin, "a")], &[]),
            pkg("b", &[(TargetKind::Bin, "tool")], &[]),
        ];
        assert_eq!(
            plan(packages.clone(), Some("tool"), None)
                .unwrap()
                .main_package,
            1
        );
        assert!(plan(packages, Some("nope"), None).is_err());
        let lib_only = vec![pkg("l", &[(TargetKind::Lib, "l")], &[])];
        assert_eq!(plan(lib_only, None, None).unwrap().main_target, "lib");
    }

    #[test]
    fn dependency_lines_are_found_in_both_manifest_spellings() {
        let m = "[package]\nname = \"x\"\n\n[dependencies]\nanyhow = \"1\"\nsqlx = { version = \"0.8\" }\n\n[dependencies.serde]\nversion = \"1\"\n";
        assert_eq!(dep_line(m, "anyhow"), 5);
        assert_eq!(dep_line(m, "sqlx"), 6);
        assert_eq!(dep_line(m, "serde"), 8);
        assert_eq!(dep_line(m, "missing"), 1);
    }
}
