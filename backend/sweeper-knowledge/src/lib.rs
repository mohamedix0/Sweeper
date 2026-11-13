//! Allowlist, Protected Paths, and Knowledge Base types/data.
//! Zero filesystem I/O in this crate.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod lint;

use std::fmt;

use serde::{Deserialize, Serialize};

/// Embedded Tier 1 & Tier 2 allowlist TOML (build-time linted).
pub const EMBEDDED_ALLOWLIST_TOML: &str = include_str!("../allowlist.toml");

/// Risk classification for targets and knowledge entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Risk {
    /// Known target whose contents are regenerated automatically and safe to remove.
    Safe,
    /// Recoverable but potentially costly to regenerate or may disturb tools.
    Caution,
    /// Not recognized; never auto-cleaned without explicit opt-in/confirmation flows.
    Unknown,
    /// Protected; displayed for context only and never selectable for Clean.
    Protected,
}

/// Target tier grouping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Tier {
    /// System temp and caches.
    Tier1,
    /// Developer caches.
    Tier2,
    /// Heavy or risky items (deferred/disabled by default in v1 phases).
    Tier3,
}

/// Closed enum of supported Windows Known Folders used by embedded targets.
/// Mapping to `FOLDERID_*` constants/resolution lives in `sweeper-winfs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum KnownFolder {
    /// `FOLDERID_LocalAppData`
    LocalAppData,
    /// `FOLDERID_RoamingAppData`
    RoamingAppData,
    /// `FOLDERID_Profile` (user profile root)
    UserProfile,
    /// `FOLDERID_Windows`
    Windows,
    /// `FOLDERID_ProgramData`
    ProgramData,
}

/// Cleanup scope for a target. Sweeper cleans contents only (never the target root folder).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    /// Remove contents inside the target root; keep the root directory itself.
    Contents,
}

/// Validated allowlist target definition (embedded, data-only).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TargetDefinition {
    /// Stable identifier.
    pub id: String,
    /// Human-readable name.
    pub name: String,
    /// Tier grouping.
    pub tier: Tier,
    /// Risk classification.
    pub risk: Risk,
    /// Known folder root anchor.
    pub known_folder: KnownFolder,
    /// Relative subpath under the known folder (validated, `\` separators).
    /// At most one intermediate wildcard segment (`*`) allowed; never in the final segment.
    pub relative_subpath: String,
    /// Scope invariant (contents only).
    pub scope: Scope,
    /// Minimum age in hours before files are eligible (default 24).
    pub min_age_hours: u32,
    /// Knowledge Base description: what it is and what cleaning affects.
    pub description: String,
    /// Whether elevation is required to clean this target (false for Tier 1 & 2).
    pub requires_elevation: bool,
}

/// Knowledge Base entry for Analyzer/UI annotations (including display-only/protected/unknown notes).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnowledgeEntry {
    /// Stable identifier.
    pub id: String,
    /// Display title.
    pub title: String,
    /// Description of what the location is and what cleaning/selection affects.
    pub description: String,
    /// Risk classification shown in Analyzer/UI.
    pub risk: Risk,
    /// Known folder anchor (when applicable).
    pub known_folder: Option<KnownFolder>,
    /// Relative path/pattern hint (display/annotation; may be empty).
    pub relative_pattern: String,
    /// Additional notes (e.g. display-only, protected container, `.vhdx` never deleted).
    pub notes: String,
}

/// Kind of protected path rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ProtectedPathKind {
    /// Subtree-protected: path or any descendant is unconditionally blocked.
    SubtreeProtected,
    /// Protected container root: exact identity protected; non-allowlist descendants blocked from selection.
    ProtectedContainerRoot,
}

/// Canonical protected path/catalog entry metadata (data-only).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProtectedPathEntry {
    /// Stable identifier for the rule/entry.
    pub id: String,
    /// Protection kind.
    pub kind: ProtectedPathKind,
    /// Human-readable display path or pattern.
    pub display: String,
    /// Rationale/notes aligned with implementation plan.
    pub notes: String,
}

/// Catalog of protected path definitions/metadata aligned with `docs/IMPLEMENTATION_PLAN.md` §3.4.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ProtectedPathCatalog {
    /// Protected path entries.
    pub entries: Vec<ProtectedPathEntry>,
}

impl ProtectedPathCatalog {
    /// Returns the canonical built-in protected path catalog metadata.
    pub fn builtin() -> Self {
        builtin_protected_path_catalog()
    }
}

/// Returns the embedded, build-time-linted Tier 1 & Tier 2 allowlist targets.
pub fn embedded_targets() -> Vec<TargetDefinition> {
    // Safe to expect: build.rs enforces lint at compile time and this parses the same embedded text.
    let lint_targets = lint::lint_allowlist_toml(EMBEDDED_ALLOWLIST_TOML)
        .expect("embedded allowlist.toml must be valid (enforced at build time)");
    lint_targets
        .into_iter()
        .map(|t| {
            TargetDefinition::try_from(t)
                .expect("embedded allowlist.toml must map to valid TargetDefinition")
        })
        .collect()
}

/// Returns built-in Knowledge Base annotations for Analyzer (including display-only/protected/unknown notes).
pub fn builtin_knowledge_entries() -> Vec<KnowledgeEntry> {
    builtin_knowledge_entries_impl()
}

/// Returns the canonical built-in protected path catalog metadata.
pub fn builtin_protected_path_catalog() -> ProtectedPathCatalog {
    ProtectedPathCatalog {
        entries: vec![
            // Subtree-protected paths
            p("prot-win-system32", ProtectedPathKind::SubtreeProtected, r"C:\Windows\System32", "Windows system binaries; unconditionally protected subtree."),
            p("prot-win-winsxs", ProtectedPathKind::SubtreeProtected, r"C:\Windows\WinSxS", "Windows component store; unconditionally protected subtree."),
            p("prot-win-syswow64", ProtectedPathKind::SubtreeProtected, r"C:\Windows\SysWOW64", "Windows WOW64 system binaries; unconditionally protected subtree."),
            p("prot-win-boot", ProtectedPathKind::SubtreeProtected, r"C:\Windows\Boot", "Boot files; unconditionally protected subtree."),
            p("prot-win-fonts", ProtectedPathKind::SubtreeProtected, r"C:\Windows\Fonts", "System fonts; unconditionally protected subtree."),
            p("prot-win-servicing", ProtectedPathKind::SubtreeProtected, r"C:\Windows\servicing", "Windows servicing internals; unconditionally protected subtree."),
            p("prot-program-files", ProtectedPathKind::SubtreeProtected, r"C:\Program Files", "Installed applications; unconditionally protected subtree."),
            p("prot-program-files-x86", ProtectedPathKind::SubtreeProtected, r"C:\Program Files (x86)", "Installed x86 applications; unconditionally protected subtree."),
            p("prot-user-desktop", ProtectedPathKind::SubtreeProtected, r"C:\Users\<name>\Desktop", "User data folder; never cleaned by Sweeper."),
            p("prot-user-documents", ProtectedPathKind::SubtreeProtected, r"C:\Users\<name>\Documents", "User data folder; never cleaned by Sweeper."),
            p("prot-user-downloads", ProtectedPathKind::SubtreeProtected, r"C:\Users\<name>\Downloads", "User data folder; never cleaned by Sweeper."),
            p("prot-user-pictures", ProtectedPathKind::SubtreeProtected, r"C:\Users\<name>\Pictures", "User data folder; never cleaned by Sweeper."),
            p("prot-user-videos", ProtectedPathKind::SubtreeProtected, r"C:\Users\<name>\Videos", "User data folder; never cleaned by Sweeper."),
            p("prot-user-music", ProtectedPathKind::SubtreeProtected, r"C:\Users\<name>\Music", "User data folder; never cleaned by Sweeper."),
            p("prot-user-onedrive", ProtectedPathKind::SubtreeProtected, r"C:\Users\<name>\OneDrive*", "OneDrive sync roots; protected to avoid cloud-destructive operations."),
            p("prot-secrets-ssh", ProtectedPathKind::SubtreeProtected, r"**\.ssh", "Secrets/credentials; any directory named .ssh is subtree-protected."),
            p("prot-secrets-gnupg", ProtectedPathKind::SubtreeProtected, r"**\.gnupg", "Secrets/keys; any directory named .gnupg is subtree-protected."),
            p("prot-secrets-aws", ProtectedPathKind::SubtreeProtected, r"**\.aws", "Credentials/config; any directory named .aws is subtree-protected."),
            p("prot-secrets-kube", ProtectedPathKind::SubtreeProtected, r"**\.kube", "Kubernetes credentials/config; any directory named .kube is subtree-protected."),
            p("prot-secrets-docker", ProtectedPathKind::SubtreeProtected, r"**\.docker", "Docker config/credentials; any directory named .docker is subtree-protected."),
            p("prot-vcs-git", ProtectedPathKind::SubtreeProtected, r"**\.git", "Version control metadata; any directory named .git is subtree-protected."),
            p("prot-os-svi", ProtectedPathKind::SubtreeProtected, r"C:\System Volume Information", "OS internal volume metadata; unconditionally protected."),
            p("prot-os-pagefile", ProtectedPathKind::SubtreeProtected, r"C:\pagefile.sys", "OS paging file; unconditionally protected."),
            p("prot-os-hiberfil", ProtectedPathKind::SubtreeProtected, r"C:\hiberfil.sys", "OS hibernation file; unconditionally protected."),
            p("prot-os-swapfile", ProtectedPathKind::SubtreeProtected, r"C:\swapfile.sys", "OS swap file; unconditionally protected."),
            p("prot-os-ntuser", ProtectedPathKind::SubtreeProtected, r"C:\Users\<name>\NTUSER.DAT*", "User registry hive files; unconditionally protected."),
            p("prot-os-recovery", ProtectedPathKind::SubtreeProtected, r"C:\Recovery", "Windows recovery data; unconditionally protected."),
            // Protected container roots
            p("prot-root-drive", ProtectedPathKind::ProtectedContainerRoot, r"C:\ (and any drive root)", "Drive roots cannot be deleted/selected; only explicit allowlisted subfolders may be cleaned."),
            p("prot-root-windows", ProtectedPathKind::ProtectedContainerRoot, r"C:\Windows", "Windows root is container-protected; descendants blocked from selection except explicit allowlisted subfolders."),
            p("prot-root-users", ProtectedPathKind::ProtectedContainerRoot, r"C:\Users", "Users root is container-protected; only explicit allowlisted subfolders may be cleaned."),
            p("prot-root-user-profile", ProtectedPathKind::ProtectedContainerRoot, r"C:\Users\<name>", "User profile roots are container-protected; descendants blocked from selection except explicit allowlisted subfolders."),
            p("prot-root-programdata", ProtectedPathKind::ProtectedContainerRoot, r"C:\ProgramData", "ProgramData root is container-protected; descendants blocked from selection except explicit allowlisted subfolders."),
            p("prot-root-browser-user-data", ProtectedPathKind::ProtectedContainerRoot, r"...\User Data", "Browser User Data roots are container-protected; only explicit leaf cache subfolders may be allowlisted."),
            p("prot-root-browser-profile", ProtectedPathKind::ProtectedContainerRoot, r"...\User Data\Default or Profile *", "Browser profile roots are container-protected; only explicit leaf cache subfolders may be allowlisted."),
        ],
    }
}

fn p(id: &str, kind: ProtectedPathKind, display: &str, notes: &str) -> ProtectedPathEntry {
    ProtectedPathEntry {
        id: id.to_string(),
        kind,
        display: display.to_string(),
        notes: notes.to_string(),
    }
}

fn builtin_knowledge_entries_impl() -> Vec<KnowledgeEntry> {
    vec![
        KnowledgeEntry {
            id: "kb-local-programs".to_string(),
            title: "Local Programs".to_string(),
            description: "Per-user installed applications under LocalAppData\\Programs.".to_string(),
            risk: Risk::Protected,
            known_folder: Some(KnownFolder::LocalAppData),
            relative_pattern: "Programs".to_string(),
            notes: "Display-only in Analyzer. Protected/Unknown-installed application content must not be cleaned by Sweeper.".to_string(),
        },
        KnowledgeEntry {
            id: "kb-rustup".to_string(),
            title: "Rustup Toolchains".to_string(),
            description: "Rust toolchains, targets, and components managed by rustup.".to_string(),
            risk: Risk::Unknown,
            known_folder: Some(KnownFolder::UserProfile),
            relative_pattern: ".rustup".to_string(),
            notes: "Display-only in Analyzer. Removing toolchains/components can break development environments; not an allowlisted clean target.".to_string(),
        },
        KnowledgeEntry {
            id: "kb-msix-packages".to_string(),
            title: "Microsoft Store / MSIX Packages".to_string(),
            description: "Packaged app data and installation state under LocalAppData\\Packages.".to_string(),
            risk: Risk::Unknown,
            known_folder: Some(KnownFolder::LocalAppData),
            relative_pattern: "Packages".to_string(),
            notes: "Display-only in Analyzer. Package contents may contain app data/state; not safe for generic cleaning.".to_string(),
        },
        KnowledgeEntry {
            id: "kb-wsl-docker-vhdx".to_string(),
            title: "WSL / Docker Virtual Disks (.vhdx)".to_string(),
            description: "Virtual disk images used by WSL distributions or Docker backends.".to_string(),
            risk: Risk::Caution,
            known_folder: None,
            relative_pattern: "*.vhdx".to_string(),
            notes: "Display-only in Analyzer. Sweeper v1 must never delete or compact .vhdx files; compaction is out of scope.".to_string(),
        },
    ]
}

/// Error converting lint-parsed target strings into typed domain enums.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetParseError {
    /// Target id.
    pub id: String,
    /// Error message.
    pub message: String,
}

impl fmt::Display for TargetParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "target `{}`: {}", self.id, self.message)
    }
}

impl std::error::Error for TargetParseError {}

impl TryFrom<LintTarget> for TargetDefinition {
    type Error = TargetParseError;

    fn try_from(t: LintTarget) -> Result<Self, Self::Error> {
        let tier = match t.tier.as_str() {
            "Tier1" => Tier::Tier1,
            "Tier2" => Tier::Tier2,
            "Tier3" => Tier::Tier3,
            other => {
                return Err(TargetParseError {
                    id: t.id,
                    message: format!("invalid tier `{other}`"),
                })
            }
        };
        let risk = match t.risk.as_str() {
            "Safe" => Risk::Safe,
            "Caution" => Risk::Caution,
            "Unknown" => Risk::Unknown,
            "Protected" => Risk::Protected,
            other => {
                return Err(TargetParseError {
                    id: t.id,
                    message: format!("invalid risk `{other}`"),
                })
            }
        };
        let known_folder = match t.known_folder.as_str() {
            "LocalAppData" => KnownFolder::LocalAppData,
            "RoamingAppData" => KnownFolder::RoamingAppData,
            "UserProfile" => KnownFolder::UserProfile,
            "Windows" => KnownFolder::Windows,
            "ProgramData" => KnownFolder::ProgramData,
            other => {
                return Err(TargetParseError {
                    id: t.id,
                    message: format!("invalid known_folder `{other}`"),
                })
            }
        };
        let scope = match t.scope.as_str() {
            "contents" => Scope::Contents,
            other => {
                return Err(TargetParseError {
                    id: t.id,
                    message: format!("invalid scope `{other}`"),
                })
            }
        };
        Ok(TargetDefinition {
            id: t.id,
            name: t.name,
            tier,
            risk,
            known_folder,
            relative_subpath: t.relative_subpath,
            scope,
            min_age_hours: t.min_age_hours,
            description: t.description,
            requires_elevation: t.requires_elevation,
        })
    }
}

pub use lint::{lint_allowlist_toml, lint_targets, LintError, LintTarget};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_allowlist_is_valid_and_tier1_tier2_only() {
        let targets = embedded_targets();
        assert!(!targets.is_empty());
        assert!(targets
            .iter()
            .all(|t| matches!(t.tier, Tier::Tier1 | Tier::Tier2)));
        assert!(targets.iter().all(|t| !t.requires_elevation));
        assert!(targets.iter().all(|t| matches!(t.scope, Scope::Contents)));
    }

    #[test]
    fn lint_rejects_globstar() {
        let toml = r#"
[[targets]]
id="x"
name="X"
tier="Tier1"
risk="Safe"
known_folder="LocalAppData"
relative_subpath="Foo\\**\\Bar"
scope="contents"
min_age_hours=24
requires_elevation=false
description="x"
"#;
        let err = lint_allowlist_toml(toml).unwrap_err();
        assert!(err.to_string().contains("**"), "{err}");
    }

    #[test]
    fn lint_rejects_wildcard_in_final_segment() {
        let toml = r#"
[[targets]]
id="x"
name="X"
tier="Tier1"
risk="Safe"
known_folder="LocalAppData"
relative_subpath="Foo\\*"
scope="contents"
min_age_hours=24
requires_elevation=false
description="x"
"#;
        let err = lint_allowlist_toml(toml).unwrap_err();
        assert!(err.to_string().contains("final path segment"), "{err}");
    }
}
