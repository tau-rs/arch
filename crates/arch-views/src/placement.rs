//! Which area and side an item sits in, from `.arch/areas.toml`.
//!
//! `arch init` writes every area with its computed side (ADR 0027, decision 5), so the file is
//! the whole answer: the first area whose path patterns match the item's file wins. An item whose
//! file matches no area is unplaced: it has no column, so no rule applies to it (ADR 0025,
//! decision 2). That covers the crate root (`main.rs`, `lib.rs`) and integration tests.

use arch_facts::{Areas, Column, Item};
use globset::{Glob, GlobSet, GlobSetBuilder};

use crate::Error;

/// Where an item sits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placement {
    /// Area name.
    pub area: String,
    /// The area's side, when it has one.
    pub side: Option<Column>,
}

/// The areas of a unit, compiled for lookup by file path.
#[derive(Debug)]
pub struct Placements {
    areas: Vec<(Placement, GlobSet)>,
}

impl Placements {
    /// Compile the path patterns of `areas.toml`.
    pub fn new(areas: &Areas) -> Result<Self, Error> {
        let mut out = Vec::with_capacity(areas.areas.len());
        for area in &areas.areas {
            let mut set = GlobSetBuilder::new();
            for pattern in &area.paths {
                let glob = Glob::new(pattern).map_err(|e| Error::BadPattern {
                    area: area.name.clone(),
                    pattern: pattern.clone(),
                    reason: e.kind().to_string(),
                })?;
                set.add(glob);
            }
            let set = set.build().map_err(|e| Error::BadPattern {
                area: area.name.clone(),
                pattern: area.paths.join(", "),
                reason: e.kind().to_string(),
            })?;
            let placement = Placement {
                area: area.name.clone(),
                side: area.side,
            };
            out.push((placement, set));
        }
        Ok(Self { areas: out })
    }

    /// The placement of a repository-relative file path; `None` when no area claims it.
    pub fn of_file(&self, file: &str) -> Option<&Placement> {
        self.areas
            .iter()
            .find(|(_, set)| set.is_match(file))
            .map(|(p, _)| p)
    }

    /// The placement of an item.
    pub fn of(&self, item: &Item) -> Option<&Placement> {
        self.of_file(&item.file)
    }
}

/// The name a side goes by in `.arch/rules` (`driving`, `domain`, `public-api`, …).
pub fn side_name(side: Column) -> &'static str {
    match side {
        Column::Driving => "driving",
        Column::Domain => "domain",
        Column::Driven => "driven",
        Column::Externals => "externals",
        Column::PublicApi => "public-api",
        Column::Internals => "internals",
        Column::Leaves => "leaves",
    }
}

#[cfg(test)]
mod tests {
    use arch_facts::AreaOverride;

    use super::*;

    fn area(name: &str, side: Option<Column>, paths: &[&str]) -> AreaOverride {
        AreaOverride {
            name: name.into(),
            paths: paths.iter().map(|p| p.to_string()).collect(),
            side,
            order: None,
        }
    }

    #[test]
    fn first_matching_area_wins_and_unmatched_files_are_unplaced() {
        let areas = Areas {
            areas: vec![
                area("http", Some(Column::Driving), &["src/adapters/http/**"]),
                area("adapters", Some(Column::Driven), &["src/adapters/**"]),
                area(
                    "app",
                    Some(Column::Domain),
                    &["src/app/**", "src/worker.rs"],
                ),
            ],
            ..Areas::default()
        };
        let p = Placements::new(&areas).unwrap();
        assert_eq!(
            p.of_file("src/adapters/http/handlers.rs").unwrap().area,
            "http"
        );
        assert_eq!(
            p.of_file("src/adapters/stripe/mod.rs").unwrap().area,
            "adapters"
        );
        assert_eq!(
            p.of_file("src/worker.rs").unwrap().side,
            Some(Column::Domain)
        );
        assert!(p.of_file("src/main.rs").is_none());
        assert!(p.of_file("tests/flows.rs").is_none());
    }

    #[test]
    fn a_bad_pattern_names_its_area() {
        let areas = Areas {
            areas: vec![area("http", None, &["src/[oops"])],
            ..Areas::default()
        };
        let err = Placements::new(&areas).unwrap_err().to_string();
        assert!(
            err.contains("area `http`") && err.contains("src/[oops"),
            "{err}"
        );
    }
}
