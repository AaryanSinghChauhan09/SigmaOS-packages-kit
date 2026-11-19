// SPDX-License-Identifier: MIT OR Apache-2.0

//! # SigmaOS Packages Kit
//!
//! An apt/dpkg-style transaction kit for the SigmaOS platform, inspired by
//! the Omarchy desktop philosophy, Hyprland's compositing elegance, and the
//! Catppuccin Mocha colour palette — clean, composable, and correct.
//!
//! This crate provides:
//! - Package metadata (`Package`, `Version`, `Architecture`, `Section`)
//! - Transaction lifecycle management (`Transaction`, `TransactionState`)
//! - A dependency graph model (`DepGraph`) with cycle detection
//! - A package cache with coherency tracking (`PackageCache`)
//! - Builder-pattern constructors with invariant validation

#![forbid(unsafe_code)]

use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;

// ──────────────────────────────────────────────────────────────────────────────
// Architecture
// ──────────────────────────────────────────────────────────────────────────────

/// CPU architecture a package targets.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Architecture {
    Amd64,
    Arm64,
    Armhf,
    I386,
    All,
    Custom(String),
}

impl fmt::Display for Architecture {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Architecture::Amd64 => write!(f, "amd64"),
            Architecture::Arm64 => write!(f, "arm64"),
            Architecture::Armhf => write!(f, "armhf"),
            Architecture::I386 => write!(f, "i386"),
            Architecture::All => write!(f, "all"),
            Architecture::Custom(s) => write!(f, "{s}"),
        }
    }
}

impl Architecture {
    /// Parse an architecture string (case-insensitive).
    pub fn parse(s: &str) -> Self {
        match s.to_ascii_lowercase().as_str() {
            "amd64" => Architecture::Amd64,
            "arm64" => Architecture::Arm64,
            "armhf" => Architecture::Armhf,
            "i386" => Architecture::I386,
            "all" => Architecture::All,
            other => Architecture::Custom(other.to_string()),
        }
    }

    /// Returns `true` when the architecture is architecture-independent.
    pub fn is_arch_all(&self) -> bool {
        matches!(self, Architecture::All)
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Section
// ──────────────────────────────────────────────────────────────────────────────

/// Debian-style package section.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Section {
    Admin,
    Devel,
    Games,
    Graphics,
    Libs,
    Net,
    Utils,
    X11,
    Other(String),
}

impl fmt::Display for Section {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Section::Admin => write!(f, "admin"),
            Section::Devel => write!(f, "devel"),
            Section::Games => write!(f, "games"),
            Section::Graphics => write!(f, "graphics"),
            Section::Libs => write!(f, "libs"),
            Section::Net => write!(f, "net"),
            Section::Utils => write!(f, "utils"),
            Section::X11 => write!(f, "x11"),
            Section::Other(s) => write!(f, "{s}"),
        }
    }
}

impl Section {
    pub fn parse(s: &str) -> Self {
        match s.to_ascii_lowercase().as_str() {
            "admin" => Section::Admin,
            "devel" => Section::Devel,
            "games" => Section::Games,
            "graphics" => Section::Graphics,
            "libs" => Section::Libs,
            "net" => Section::Net,
            "utils" => Section::Utils,
            "x11" => Section::X11,
            other => Section::Other(other.to_string()),
        }
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Version
// ──────────────────────────────────────────────────────────────────────────────

/// A simplified Debian-style version (epoch:upstream-revision).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    pub epoch: u32,
    pub upstream: String,
    pub revision: Option<String>,
}

impl Version {
    /// Create a version without epoch or revision.
    pub fn new(upstream: impl Into<String>) -> Self {
        Version { epoch: 0, upstream: upstream.into(), revision: None }
    }

    /// Parse `[epoch:]upstream[-revision]`.
    pub fn parse(s: &str) -> Result<Self, PkgError> {
        let s = s.trim();
        if s.is_empty() {
            return Err(PkgError::InvalidVersion("empty version string".into()));
        }
        // Extract epoch
        let (epoch, rest) = if let Some(colon) = s.find(':') {
            let ep: u32 = s[..colon]
                .parse()
                .map_err(|_| PkgError::InvalidVersion(format!("bad epoch in '{s}'")))?;
            (ep, &s[colon + 1..])
        } else {
            (0, s)
        };
        // Extract revision
        let (upstream, revision) = if let Some(dash) = rest.rfind('-') {
            (&rest[..dash], Some(rest[dash + 1..].to_string()))
        } else {
            (rest, None)
        };
        if upstream.is_empty() {
            return Err(PkgError::InvalidVersion(format!("empty upstream in '{s}'")));
        }
        Ok(Version { epoch, upstream: upstream.to_string(), revision })
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.epoch != 0 {
            write!(f, "{}:", self.epoch)?;
        }
        write!(f, "{}", self.upstream)?;
        if let Some(rev) = &self.revision {
            write!(f, "-{rev}")?;
        }
        Ok(())
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.epoch
            .cmp(&other.epoch)
            .then_with(|| self.upstream.cmp(&other.upstream))
            .then_with(|| self.revision.cmp(&other.revision))
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Error types
// ──────────────────────────────────────────────────────────────────────────────

/// All errors that can arise within the packages-kit domain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PkgError {
    InvalidVersion(String),
    InvalidPackageName(String),
    DependencyCycle(Vec<String>),
    PackageNotFound(String),
    TransactionConflict(String),
    CacheCoherencyError(String),
    ValidationError(String),
}

impl fmt::Display for PkgError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PkgError::InvalidVersion(m) => write!(f, "invalid version: {m}"),
            PkgError::InvalidPackageName(m) => write!(f, "invalid package name: {m}"),
            PkgError::DependencyCycle(cycle) => {
                write!(f, "dependency cycle detected: {}", cycle.join(" → "))
            }
            PkgError::PackageNotFound(n) => write!(f, "package not found: {n}"),
            PkgError::TransactionConflict(m) => write!(f, "transaction conflict: {m}"),
            PkgError::CacheCoherencyError(m) => write!(f, "cache coherency error: {m}"),
            PkgError::ValidationError(m) => write!(f, "validation error: {m}"),
        }
    }
}

impl std::error::Error for PkgError {}

// ──────────────────────────────────────────────────────────────────────────────
// Package metadata
// ──────────────────────────────────────────────────────────────────────────────

/// Full package descriptor (mirrors a dpkg `control` stanza).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Package {
    pub name: String,
    pub version: Version,
    pub arch: Architecture,
    pub section: Section,
    pub description: String,
    pub installed_size_kb: u64,
    pub maintainer: String,
    pub depends: Vec<String>,
    pub recommends: Vec<String>,
    pub conflicts: Vec<String>,
}

impl Package {
    /// Validate package name per Debian policy (lowercase, alphanumeric + `+-.`).
    pub fn validate_name(name: &str) -> Result<(), PkgError> {
        if name.is_empty() {
            return Err(PkgError::InvalidPackageName("name must not be empty".into()));
        }
        if !name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "+-_.".contains(c)) {
            return Err(PkgError::InvalidPackageName(format!(
                "'{name}' contains characters not allowed by Debian policy"
            )));
        }
        Ok(())
    }

    /// Canonical identifier `name:arch`.
    pub fn id(&self) -> String {
        format!("{}:{}", self.name, self.arch)
    }
}

impl fmt::Display for Package {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} ({}) [{}] — {}",
            self.name, self.version, self.arch, self.description
        )
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Package builder (builder pattern with validation)
// ──────────────────────────────────────────────────────────────────────────────

/// Builder for `Package`; validates invariants before construction.
#[derive(Debug, Default)]
pub struct PackageBuilder {
    name: Option<String>,
    version: Option<Version>,
    arch: Option<Architecture>,
    section: Option<Section>,
    description: Option<String>,
    installed_size_kb: u64,
    maintainer: String,
    depends: Vec<String>,
    recommends: Vec<String>,
    conflicts: Vec<String>,
}

impl PackageBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    pub fn version(mut self, v: Version) -> Self {
        self.version = Some(v);
        self
    }

    pub fn arch(mut self, a: Architecture) -> Self {
        self.arch = Some(a);
        self
    }

    pub fn section(mut self, s: Section) -> Self {
        self.section = Some(s);
        self
    }

    pub fn description(mut self, d: impl Into<String>) -> Self {
        self.description = Some(d.into());
        self
    }

    pub fn installed_size_kb(mut self, kb: u64) -> Self {
        self.installed_size_kb = kb;
        self
    }

    pub fn maintainer(mut self, m: impl Into<String>) -> Self {
        self.maintainer = m.into();
        self
    }

    pub fn depends(mut self, deps: Vec<String>) -> Self {
        self.depends = deps;
        self
    }

    pub fn conflicts(mut self, c: Vec<String>) -> Self {
        self.conflicts = c;
        self
    }

    /// Validate all fields and produce a `Package`.
    pub fn build(self) -> Result<Package, PkgError> {
        let name = self
            .name
            .ok_or_else(|| PkgError::ValidationError("package name is required".into()))?;
        Package::validate_name(&name)?;

        let version = self
            .version
            .ok_or_else(|| PkgError::ValidationError("version is required".into()))?;

        Ok(Package {
            name,
            version,
            arch: self.arch.unwrap_or(Architecture::All),
            section: self.section.unwrap_or(Section::Utils),
            description: self.description.unwrap_or_default(),
            installed_size_kb: self.installed_size_kb,
            maintainer: self.maintainer,
            depends: self.depends,
            recommends: self.recommends,
            conflicts: self.conflicts,
        })
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Transaction state machine
// ──────────────────────────────────────────────────────────────────────────────

/// Lifecycle states of a package transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransactionState {
    Pending,
    Downloading { progress_pct: u8 },
    Installing,
    Configuring,
    Done,
    Failed(String),
    RolledBack,
}

impl fmt::Display for TransactionState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TransactionState::Pending => write!(f, "pending"),
            TransactionState::Downloading { progress_pct } => {
                write!(f, "downloading ({progress_pct}%)")
            }
            TransactionState::Installing => write!(f, "installing"),
            TransactionState::Configuring => write!(f, "configuring"),
            TransactionState::Done => write!(f, "done"),
            TransactionState::Failed(reason) => write!(f, "failed: {reason}"),
            TransactionState::RolledBack => write!(f, "rolled-back"),
        }
    }
}

/// A single package-install/remove transaction.
#[derive(Debug, Clone)]
pub struct Transaction {
    pub id: u64,
    pub package_name: String,
    pub state: TransactionState,
    pub log: Vec<String>,
}

impl Transaction {
    pub fn new(id: u64, package_name: impl Into<String>) -> Self {
        Transaction {
            id,
            package_name: package_name.into(),
            state: TransactionState::Pending,
            log: Vec::new(),
        }
    }

    /// Advance state machine; returns `Err` on illegal transitions.
    pub fn advance(&mut self, next: TransactionState) -> Result<(), PkgError> {
        let allowed = match &self.state {
            TransactionState::Pending => matches!(
                next,
                TransactionState::Downloading { .. } | TransactionState::Failed(_)
            ),
            TransactionState::Downloading { .. } => matches!(
                next,
                TransactionState::Downloading { .. }
                    | TransactionState::Installing
                    | TransactionState::Failed(_)
            ),
            TransactionState::Installing => matches!(
                next,
                TransactionState::Configuring
                    | TransactionState::Failed(_)
                    | TransactionState::RolledBack
            ),
            TransactionState::Configuring => matches!(
                next,
                TransactionState::Done | TransactionState::Failed(_)
            ),
            TransactionState::Done | TransactionState::Failed(_) | TransactionState::RolledBack => {
                false
            }
        };
        if !allowed {
            return Err(PkgError::TransactionConflict(format!(
                "cannot transition from '{}' to '{next}'",
                self.state
            )));
        }
        self.log.push(format!("{} → {next}", self.state));
        self.state = next;
        Ok(())
    }

    pub fn is_terminal(&self) -> bool {
        matches!(
            self.state,
            TransactionState::Done | TransactionState::Failed(_) | TransactionState::RolledBack
        )
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Dependency graph
// ──────────────────────────────────────────────────────────────────────────────

/// A directed dependency graph over package names.
/// Edges represent "A depends on B".
#[derive(Debug, Default)]
pub struct DepGraph {
    edges: HashMap<String, Vec<String>>,
}

impl DepGraph {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register that `pkg` depends on each name in `deps`.
    pub fn add_deps(&mut self, pkg: impl Into<String>, deps: Vec<String>) {
        self.edges.insert(pkg.into(), deps);
    }

    /// Topological install order (dependencies installed before packages that need them).
    /// Returns `Err` if a cycle exists.
    pub fn install_order(&self) -> Result<Vec<String>, PkgError> {
        let mut in_degree: HashMap<&str, usize> = HashMap::new();
        let mut dependents: HashMap<&str, Vec<&str>> = HashMap::new();
        let mut all_nodes: HashSet<&str> = HashSet::new();

        for (node, deps) in &self.edges {
            all_nodes.insert(node.as_str());
            in_degree.entry(node.as_str()).or_insert(0);
            for dep in deps {
                all_nodes.insert(dep.as_str());
                dependents.entry(dep.as_str()).or_default().push(node.as_str());
                *in_degree.entry(node.as_str()).or_insert(0) += 1;
            }
        }
        for node in &all_nodes {
            in_degree.entry(node).or_insert(0);
        }

        let mut queue: VecDeque<&str> = in_degree
            .iter()
            .filter(|(_, &d)| d == 0)
            .map(|(n, _)| *n)
            .collect();

        let mut order = Vec::with_capacity(all_nodes.len());
        while let Some(node) = queue.pop_front() {
            order.push(node.to_string());
            if let Some(deps) = dependents.get(node) {
                for dep in deps {
                    let d = in_degree.get_mut(dep).unwrap();
                    *d -= 1;
                    if *d == 0 {
                        queue.push_back(dep);
                    }
                }
            }
        }

        if order.len() != all_nodes.len() {
            let installed: HashSet<&str> = order.iter().map(|s| s.as_str()).collect();
            let cycle: Vec<String> = all_nodes
                .iter()
                .filter(|n| !installed.contains(**n))
                .map(|s| s.to_string())
                .collect();
            return Err(PkgError::DependencyCycle(cycle));
        }

        Ok(order)
    }

    /// Returns all transitive dependencies of `pkg` (BFS).
    pub fn transitive_deps(&self, pkg: &str) -> Vec<String> {
        let mut visited = HashSet::new();
        let mut queue = VecDeque::new();
        queue.push_back(pkg.to_string());
        while let Some(node) = queue.pop_front() {
            if visited.contains(&node) {
                continue;
            }
            visited.insert(node.clone());
            if let Some(deps) = self.edges.get(&node) {
                for d in deps {
                    if !visited.contains(d) {
                        queue.push_back(d.clone());
                    }
                }
            }
        }
        visited.remove(pkg);
        visited.into_iter().collect()
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Package cache
// ──────────────────────────────────────────────────────────────────────────────

/// Cache state for bookkeeping.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CacheState {
    Clean,
    Dirty,
    Rebuilding,
}

impl fmt::Display for CacheState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CacheState::Clean => write!(f, "clean"),
            CacheState::Dirty => write!(f, "dirty"),
            CacheState::Rebuilding => write!(f, "rebuilding"),
        }
    }
}

/// In-memory package cache with coherency tracking.
#[derive(Debug)]
pub struct PackageCache {
    packages: HashMap<String, Package>,
    state: CacheState,
    generation: u64,
}

impl PackageCache {
    pub fn new() -> Self {
        PackageCache {
            packages: HashMap::new(),
            state: CacheState::Clean,
            generation: 0,
        }
    }

    /// Insert or update a package entry; marks cache dirty.
    pub fn insert(&mut self, pkg: Package) {
        let key = pkg.name.clone();
        self.packages.insert(key, pkg);
        self.state = CacheState::Dirty;
    }

    /// Retrieve a package by name.
    pub fn get(&self, name: &str) -> Option<&Package> {
        self.packages.get(name)
    }

    /// Remove a package; returns `Err` if it was not cached.
    pub fn remove(&mut self, name: &str) -> Result<Package, PkgError> {
        self.packages
            .remove(name)
            .ok_or_else(|| PkgError::PackageNotFound(name.to_string()))
            .map(|p| {
                self.state = CacheState::Dirty;
                p
            })
    }

    /// Mark cache as clean and bump generation counter.
    pub fn commit(&mut self) {
        self.state = CacheState::Clean;
        self.generation += 1;
    }

    pub fn state(&self) -> &CacheState {
        &self.state
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn len(&self) -> usize {
        self.packages.len()
    }

    pub fn is_empty(&self) -> bool {
        self.packages.is_empty()
    }

    /// Coherency check: returns `Err` if any package's declared deps are
    /// missing from the cache.
    pub fn check_coherency(&self) -> Result<(), PkgError> {
        for pkg in self.packages.values() {
            for dep in &pkg.depends {
                // Strip version constraints like `libc6 (>= 2.17)` → `libc6`
                let dep_name = dep.split_whitespace().next().unwrap_or(dep.as_str());
                if !self.packages.contains_key(dep_name) {
                    return Err(PkgError::CacheCoherencyError(format!(
                        "'{}' depends on '{}' which is not cached",
                        pkg.name, dep_name
                    )));
                }
            }
        }
        Ok(())
    }

    /// All cached packages sorted by name.
    pub fn sorted_names(&self) -> Vec<&str> {
        let mut names: Vec<&str> = self.packages.keys().map(|s| s.as_str()).collect();
        names.sort_unstable();
        names
    }
}

impl Default for PackageCache {
    fn default() -> Self {
        Self::new()
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Tests
// ──────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── helpers ───────────────────────────────────────────────────────────────

    fn sample_package(name: &str) -> Package {
        PackageBuilder::new()
            .name(name)
            .version(Version::new("1.0.0"))
            .arch(Architecture::Amd64)
            .section(Section::Utils)
            .description("test package")
            .build()
            .expect("valid package")
    }

    // ── Version ───────────────────────────────────────────────────────────────

    #[test]
    fn version_parse_simple() {
        // Arrange
        let raw = "2.5.3";
        // Act
        let v = Version::parse(raw).unwrap();
        // Assert
        assert_eq!(v.epoch, 0);
        assert_eq!(v.upstream, "2.5.3");
        assert!(v.revision.is_none());
        assert_eq!(v.to_string(), "2.5.3");
    }

    #[test]
    fn version_parse_full() {
        // Arrange / Act
        let v = Version::parse("2:1.9.3p484-2+deb8u1").unwrap();
        // Assert
        assert_eq!(v.epoch, 2);
        assert_eq!(v.upstream, "1.9.3p484");
        assert_eq!(v.revision.as_deref(), Some("2+deb8u1"));
        assert_eq!(v.to_string(), "2:1.9.3p484-2+deb8u1");
    }

    #[test]
    fn version_ordering() {
        // Arrange
        let old = Version::parse("1:1.0-1").unwrap();
        let new = Version::parse("1:2.0-1").unwrap();
        // Act / Assert
        assert!(new > old);
    }

    #[test]
    fn version_parse_empty_is_err() {
        // Act
        let result = Version::parse("");
        // Assert
        assert!(matches!(result, Err(PkgError::InvalidVersion(_))));
    }

    // ── Architecture ──────────────────────────────────────────────────────────

    #[test]
    fn architecture_parse_and_display() {
        // Arrange / Act
        let arch = Architecture::parse("ARM64");
        // Assert
        assert_eq!(arch, Architecture::Arm64);
        assert_eq!(arch.to_string(), "arm64");
        assert!(!arch.is_arch_all());
        assert!(Architecture::All.is_arch_all());
    }

    // ── PackageBuilder ────────────────────────────────────────────────────────

    #[test]
    fn builder_produces_valid_package() {
        // Arrange
        let v = Version::new("3.14.1");
        // Act
        let pkg = PackageBuilder::new()
            .name("sigma-core")
            .version(v.clone())
            .arch(Architecture::Amd64)
            .section(Section::Libs)
            .description("Core SigmaOS library")
            .installed_size_kb(4096)
            .maintainer("Aaryan <sigma@example.com>")
            .depends(vec!["libc6".into(), "libssl3".into()])
            .build()
            .unwrap();
        // Assert
        assert_eq!(pkg.name, "sigma-core");
        assert_eq!(pkg.version, v);
        assert_eq!(pkg.arch, Architecture::Amd64);
        assert_eq!(pkg.depends.len(), 2);
        assert_eq!(pkg.id(), "sigma-core:amd64");
    }

    #[test]
    fn builder_rejects_invalid_name() {
        // Arrange / Act
        let result = PackageBuilder::new()
            .name("INVALID NAME!")
            .version(Version::new("1.0"))
            .build();
        // Assert
        assert!(matches!(result, Err(PkgError::InvalidPackageName(_))));
    }

    #[test]
    fn builder_rejects_missing_version() {
        // Arrange / Act
        let result = PackageBuilder::new().name("good-pkg").build();
        // Assert
        assert!(matches!(result, Err(PkgError::ValidationError(_))));
    }

    // ── Transaction ───────────────────────────────────────────────────────────

    #[test]
    fn transaction_state_machine_happy_path() {
        // Arrange
        let mut tx = Transaction::new(1, "sigma-core");
        // Act & Assert
        assert_eq!(tx.state, TransactionState::Pending);

        tx.advance(TransactionState::Downloading { progress_pct: 50 }).unwrap();
        tx.advance(TransactionState::Downloading { progress_pct: 100 }).unwrap();
        tx.advance(TransactionState::Installing).unwrap();
        tx.advance(TransactionState::Configuring).unwrap();
        tx.advance(TransactionState::Done).unwrap();

        assert!(tx.is_terminal());
        assert_eq!(tx.log.len(), 5);
    }

    #[test]
    fn transaction_illegal_transition_is_err() {
        // Arrange
        let mut tx = Transaction::new(2, "bad-pkg");
        // Act
        let result = tx.advance(TransactionState::Done); // skip states
        // Assert
        assert!(matches!(result, Err(PkgError::TransactionConflict(_))));
    }

    #[test]
    fn transaction_failure_path() {
        // Arrange
        let mut tx = Transaction::new(3, "broken-pkg");
        tx.advance(TransactionState::Downloading { progress_pct: 10 }).unwrap();
        // Act
        tx.advance(TransactionState::Failed("checksum mismatch".into())).unwrap();
        // Assert
        assert!(tx.is_terminal());
        assert!(matches!(tx.state, TransactionState::Failed(_)));
    }

    // ── DepGraph ──────────────────────────────────────────────────────────────

    #[test]
    fn dep_graph_install_order() {
        // Arrange: A → B → C (A depends on B, B depends on C)
        let mut g = DepGraph::new();
        g.add_deps("app", vec!["libfoo".into()]);
        g.add_deps("libfoo", vec!["libc6".into()]);
        // Act
        let order = g.install_order().unwrap();
        // Assert: libc6 must appear before libfoo, libfoo before app
        let pos = |n: &str| order.iter().position(|s| s == n).unwrap();
        assert!(pos("libc6") < pos("libfoo"));
        assert!(pos("libfoo") < pos("app"));
    }

    #[test]
    fn dep_graph_detects_cycle() {
        // Arrange: A → B → A
        let mut g = DepGraph::new();
        g.add_deps("alpha", vec!["beta".into()]);
        g.add_deps("beta", vec!["alpha".into()]);
        // Act
        let result = g.install_order();
        // Assert
        assert!(matches!(result, Err(PkgError::DependencyCycle(_))));
    }

    #[test]
    fn dep_graph_transitive_deps() {
        // Arrange
        let mut g = DepGraph::new();
        g.add_deps("app", vec!["libfoo".into()]);
        g.add_deps("libfoo", vec!["libc6".into()]);
        // Act
        let mut deps = g.transitive_deps("app");
        deps.sort();
        // Assert
        assert!(deps.contains(&"libc6".to_string()));
        assert!(deps.contains(&"libfoo".to_string()));
    }

    // ── PackageCache ──────────────────────────────────────────────────────────

    #[test]
    fn cache_insert_get_remove() {
        // Arrange
        let mut cache = PackageCache::new();
        let pkg = sample_package("neovim");
        // Act
        cache.insert(pkg.clone());
        // Assert — dirty after insert
        assert_eq!(*cache.state(), CacheState::Dirty);
        assert_eq!(cache.len(), 1);
        let fetched = cache.get("neovim").unwrap();
        assert_eq!(fetched.name, "neovim");

        // Remove
        let removed = cache.remove("neovim").unwrap();
        assert_eq!(removed.name, "neovim");
        assert!(cache.is_empty());
    }

    #[test]
    fn cache_commit_bumps_generation() {
        // Arrange
        let mut cache = PackageCache::new();
        cache.insert(sample_package("sigma-panel"));
        assert_eq!(cache.generation(), 0);
        // Act
        cache.commit();
        // Assert
        assert_eq!(*cache.state(), CacheState::Clean);
        assert_eq!(cache.generation(), 1);
    }

    #[test]
    fn cache_coherency_missing_dep() {
        // Arrange
        let mut cache = PackageCache::new();
        let pkg = PackageBuilder::new()
            .name("sigma-wm")
            .version(Version::new("1.0"))
            .depends(vec!["libwayland-client0".into()])
            .build()
            .unwrap();
        cache.insert(pkg);
        // Act
        let result = cache.check_coherency();
        // Assert
        assert!(matches!(result, Err(PkgError::CacheCoherencyError(_))));
    }

    #[test]
    fn cache_coherency_satisfied() {
        // Arrange
        let mut cache = PackageCache::new();
        let lib = sample_package("libwayland-client0");
        let wm = PackageBuilder::new()
            .name("sigma-wm")
            .version(Version::new("1.0"))
            .depends(vec!["libwayland-client0".into()])
            .build()
            .unwrap();
        cache.insert(lib);
        cache.insert(wm);
        // Act / Assert
        assert!(cache.check_coherency().is_ok());
    }

    #[test]
    fn cache_sorted_names() {
        // Arrange
        let mut cache = PackageCache::new();
        for name in ["zsh", "bash", "fish", "dash"] {
            cache.insert(sample_package(name));
        }
        // Act
        let names = cache.sorted_names();
        // Assert
        assert_eq!(names, vec!["bash", "dash", "fish", "zsh"]);
    }

    // ── Error display ─────────────────────────────────────────────────────────

    #[test]
    fn error_display_messages() {
        // Arrange / Act / Assert
        assert!(PkgError::InvalidVersion("bad".into())
            .to_string()
            .contains("invalid version"));
        assert!(PkgError::DependencyCycle(vec!["a".into(), "b".into()])
            .to_string()
            .contains("→"));
        assert!(PkgError::PackageNotFound("ghost".into())
            .to_string()
            .contains("ghost"));
    }
}
