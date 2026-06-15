#![forbid(unsafe_code)]
//! rapt — High-level package manager for RacOS
//!
//! Phase E MVP in this crate provides:
//! - repository index data model
//! - dependency token parsing
//! - topological install plan generation

#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(not(feature = "std"))]
extern crate alloc;

#[cfg(not(feature = "std"))]
use alloc::collections::{BTreeMap as HashMap, BTreeSet as HashSet, VecDeque};
#[cfg(not(feature = "std"))]
use alloc::string::String;
#[cfg(not(feature = "std"))]
use alloc::vec::Vec;
#[cfg(feature = "std")]
use std::collections::{HashMap, HashSet, VecDeque};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoPackage {
    pub name: String,
    pub version: String,
    pub arch: String,
    pub filename: String,
    /// Dependencies in format examples: "libc-lite >= 0.1.0", "zlib"
    pub depends: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DepSpec {
    pub name: String,
    pub constraint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallStep {
    pub name: String,
    pub version: String,
    pub filename: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    EmptyName,
    MissingPackage(String),
    CyclicDependency,
}

pub fn parse_dep(dep: &str) -> Result<DepSpec, Error> {
    let d = dep.trim();
    if d.is_empty() {
        return Err(Error::EmptyName);
    }

    if let Some(idx) = d.find(char::is_whitespace) {
        let name = d[..idx].trim();
        if name.is_empty() {
            return Err(Error::EmptyName);
        }
        let rest = d[idx..].trim();
        let constraint = if rest.is_empty() {
            None
        } else {
            Some(rest.to_string())
        };
        Ok(DepSpec {
            name: name.to_string(),
            constraint,
        })
    } else {
        Ok(DepSpec {
            name: d.to_string(),
            constraint: None,
        })
    }
}

/// Build install order for requested package names.
///
/// Strategy:
/// - choose one package record per name from `index` (first match)
/// - build transitive dependency graph by package name
/// - topological sort (dependencies first)
pub fn plan_install(index: &[RepoPackage], requested: &[&str]) -> Result<Vec<InstallStep>, Error> {
    let mut by_name: HashMap<&str, &RepoPackage> = HashMap::new();
    for p in index {
        by_name.entry(p.name.as_str()).or_insert(p);
    }

    let mut needed: HashSet<String> = HashSet::new();
    let mut queue: VecDeque<String> = requested.iter().map(|s| s.to_string()).collect();

    while let Some(name) = queue.pop_front() {
        if needed.contains(&name) {
            continue;
        }
        let pkg = by_name
            .get(name.as_str())
            .copied()
            .ok_or_else(|| Error::MissingPackage(name.clone()))?;
        needed.insert(name.clone());

        for dep in &pkg.depends {
            let ds = parse_dep(dep)?;
            queue.push_back(ds.name);
        }
    }

    // Build graph dep -> user (so indegree counts unresolved dependencies).
    let mut indegree: HashMap<String, usize> = HashMap::new();
    let mut edges: HashMap<String, Vec<String>> = HashMap::new();
    for n in &needed {
        indegree.insert(n.clone(), 0);
        edges.insert(n.clone(), Vec::new());
    }

    for n in &needed {
        let pkg = by_name
            .get(n.as_str())
            .copied()
            .ok_or_else(|| Error::MissingPackage(n.clone()))?;
        for dep in &pkg.depends {
            let ds = parse_dep(dep)?;
            if needed.contains(&ds.name) {
                edges.get_mut(&ds.name).unwrap().push(n.clone());
                *indegree.get_mut(n).unwrap() += 1;
            }
        }
    }

    let mut ready: VecDeque<String> = indegree
        .iter()
        .filter(|(_, d)| **d == 0)
        .map(|(n, _)| n.clone())
        .collect();

    let mut out: Vec<InstallStep> = Vec::new();
    while let Some(name) = ready.pop_front() {
        let pkg = by_name
            .get(name.as_str())
            .copied()
            .ok_or_else(|| Error::MissingPackage(name.clone()))?;
        out.push(InstallStep {
            name: pkg.name.clone(),
            version: pkg.version.clone(),
            filename: pkg.filename.clone(),
        });

        if let Some(users) = edges.get(&name) {
            for u in users {
                let e = indegree.get_mut(u).unwrap();
                *e -= 1;
                if *e == 0 {
                    ready.push_back(u.clone());
                }
            }
        }
    }

    if out.len() != needed.len() {
        return Err(Error::CyclicDependency);
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_dep_name_only() {
        let d = parse_dep("zlib").unwrap();
        assert_eq!(d.name, "zlib");
        assert!(d.constraint.is_none());
    }

    #[test]
    fn parse_dep_with_constraint() {
        let d = parse_dep("libc-lite >= 0.1.0, < 1.0.0").unwrap();
        assert_eq!(d.name, "libc-lite");
        assert!(d.constraint.is_some());
    }

    #[test]
    fn plan_topo_order() {
        let index = vec![
            RepoPackage {
                name: "libc-lite".into(),
                version: "0.1.0".into(),
                arch: "x86_64".into(),
                filename: "libc-lite.rpk".into(),
                depends: vec![],
            },
            RepoPackage {
                name: "demo".into(),
                version: "1.0.0".into(),
                arch: "x86_64".into(),
                filename: "demo.rpk".into(),
                depends: vec!["libc-lite >= 0.1.0".into()],
            },
        ];

        let plan = plan_install(&index, &["demo"]).unwrap();
        assert_eq!(plan.len(), 2);
        assert_eq!(plan[0].name, "libc-lite");
        assert_eq!(plan[1].name, "demo");
    }
}
