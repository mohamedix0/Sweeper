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

/// Cleanup scope for a target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    /// Remove contents inside the target root; keep the root directory itself.
    /// Each file is age-eligible individually (Minimum Age applies).
    Contents,
    /// Remove the entire, explicitly scoped cache directory as one unit: no
    /// per-file Minimum Age filtering and no per-item selection. Reserved for
    /// caches that are 100% regenerable (issue #41). The root directory is
    /// removed after its contents; there is no partial/per-file deletion model.
    WholeCache,
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
    let mut entries = Vec::new();
    entries.extend(protected_system_knowledge_entries());
    entries.extend(user_and_temp_knowledge_entries());
    entries.extend(browser_and_app_knowledge_entries());
    entries.extend(developer_and_elevation_knowledge_entries());
    entries
}

fn protected_system_knowledge_entries() -> Vec<KnowledgeEntry> {
    vec![
        KnowledgeEntry {
            id: "kb-windows-root".to_string(),
            title: "Windows Directory".to_string(),
            description: "Windows operating system directory (system binaries, servicing, configuration, and OS-managed components).".to_string(),
            risk: Risk::Protected,
            known_folder: Some(KnownFolder::Windows),
            relative_pattern: "".to_string(),
            notes: "Protected container root in Analyzer. Only explicitly allowlisted subfolders (e.g. Windows Temp when enabled by policy/phase) may be cleanable; the Windows root and non-allowlisted descendants are not selectable.".to_string(),
        },
        KnowledgeEntry {
            id: "kb-users-root".to_string(),
            title: "Users Directory".to_string(),
            description: "Root directory containing Windows user profiles and shared per-user locations.".to_string(),
            risk: Risk::Protected,
            known_folder: None,
            relative_pattern: r"C:\Users".to_string(),
            notes: "Protected container root in Analyzer. Non-allowlist descendants are not selectable; only explicitly allowlisted subfolders under profiles may be cleanable.".to_string(),
        },
        KnowledgeEntry {
            id: "kb-user-profile-root".to_string(),
            title: "User Profile".to_string(),
            description: "A user profile root (home directory) containing personal folders, application data, and profile state.".to_string(),
            risk: Risk::Protected,
            known_folder: Some(KnownFolder::UserProfile),
            relative_pattern: "".to_string(),
            notes: "Protected container root in Analyzer. Personal folders and non-allowlisted descendants are not selectable; only explicitly allowlisted subfolders may be cleanable.".to_string(),
        },
        KnowledgeEntry {
            id: "kb-program-files".to_string(),
            title: "Program Files".to_string(),
            description: "Installed 64-bit (and system-native) applications managed by installers and Windows.".to_string(),
            risk: Risk::Protected,
            known_folder: None,
            relative_pattern: r"C:\Program Files".to_string(),
            notes: "Subtree-protected. Application binaries and shared components must not be removed by Sweeper.".to_string(),
        },
        KnowledgeEntry {
            id: "kb-program-files-x86".to_string(),
            title: "Program Files (x86)".to_string(),
            description: "Installed 32-bit applications on 64-bit Windows systems.".to_string(),
            risk: Risk::Protected,
            known_folder: None,
            relative_pattern: r"C:\Program Files (x86)".to_string(),
            notes: "Subtree-protected. Application binaries and shared components must not be removed by Sweeper.".to_string(),
        },
        KnowledgeEntry {
            id: "kb-local-programs".to_string(),
            title: "Local Programs".to_string(),
            description: "Per-user installed applications under LocalAppData\\Programs. Removing installed program files can break applications and their updaters.".to_string(),
            risk: Risk::Protected,
            known_folder: Some(KnownFolder::LocalAppData),
            relative_pattern: "Programs".to_string(),
            notes: "Display-only in Analyzer. Protected/Unknown-installed application content must not be cleaned by Sweeper unless explicitly allowlisted in a future, audited scope.".to_string(),
        },
    ]
}

fn user_and_temp_knowledge_entries() -> Vec<KnowledgeEntry> {
    vec![
        KnowledgeEntry {
            id: "kb-user-desktop".to_string(),
            title: "Desktop".to_string(),
            description: "User Desktop folder containing files and shortcuts the user intentionally keeps visible.".to_string(),
            risk: Risk::Protected,
            known_folder: Some(KnownFolder::UserProfile),
            relative_pattern: "Desktop".to_string(),
            notes: "Personal user data. Subtree-protected and never selectable for Clean.".to_string(),
        },
        KnowledgeEntry {
            id: "kb-user-documents".to_string(),
            title: "Documents".to_string(),
            description: "User Documents folder for personal documents and application-saved files.".to_string(),
            risk: Risk::Protected,
            known_folder: Some(KnownFolder::UserProfile),
            relative_pattern: "Documents".to_string(),
            notes: "Personal user data. Subtree-protected and never selectable for Clean.".to_string(),
        },
        KnowledgeEntry {
            id: "kb-user-downloads".to_string(),
            title: "Downloads".to_string(),
            description: "User Downloads folder containing files downloaded by browsers and other apps.".to_string(),
            risk: Risk::Protected,
            known_folder: Some(KnownFolder::UserProfile),
            relative_pattern: "Downloads".to_string(),
            notes: "Personal user data. Subtree-protected and never selectable for Clean.".to_string(),
        },
        KnowledgeEntry {
            id: "kb-user-pictures".to_string(),
            title: "Pictures".to_string(),
            description: "User Pictures library containing photos and images.".to_string(),
            risk: Risk::Protected,
            known_folder: Some(KnownFolder::UserProfile),
            relative_pattern: "Pictures".to_string(),
            notes: "Personal user data. Subtree-protected and never selectable for Clean.".to_string(),
        },
        KnowledgeEntry {
            id: "kb-user-videos".to_string(),
            title: "Videos".to_string(),
            description: "User Videos library containing video files.".to_string(),
            risk: Risk::Protected,
            known_folder: Some(KnownFolder::UserProfile),
            relative_pattern: "Videos".to_string(),
            notes: "Personal user data. Subtree-protected and never selectable for Clean.".to_string(),
        },
        KnowledgeEntry {
            id: "kb-user-music".to_string(),
            title: "Music".to_string(),
            description: "User Music library containing audio files.".to_string(),
            risk: Risk::Protected,
            known_folder: Some(KnownFolder::UserProfile),
            relative_pattern: "Music".to_string(),
            notes: "Personal user data. Subtree-protected and never selectable for Clean.".to_string(),
        },
        KnowledgeEntry {
            id: "kb-wsl-docker-vhdx".to_string(),
            title: "WSL / Docker Virtual Disks (.vhdx)".to_string(),
            description: "Virtual disk image files used by WSL distributions and Docker Desktop backends to store Linux/container filesystems.".to_string(),
            risk: Risk::Caution,
            known_folder: None,
            relative_pattern: "*.vhdx".to_string(),
            notes: "Display-only in Analyzer. Sweeper v1 must never delete or compact .vhdx files (compaction is out of scope); selecting them for cleanup must be rejected by safety policy.".to_string(),
        },
    ]
}

fn browser_and_app_knowledge_entries() -> Vec<KnowledgeEntry> {
    vec![
        KnowledgeEntry {
            id: "kb-gpu-shader-caches".to_string(),
            title: "GPU and DirectX Shader Caches".to_string(),
            description: "Compiled shader and pipeline caches created by DirectX, NVIDIA, and AMD graphics drivers/components (e.g. D3DSCache, DXCache, GLCache, VkCache).".to_string(),
            risk: Risk::Safe,
            known_folder: Some(KnownFolder::LocalAppData),
            relative_pattern: r"D3DSCache; NVIDIA\DXCache; NVIDIA\GLCache; AMD\DxCache; AMD\VkCache".to_string(),
            notes: "Safe, regenerable caches. Clearing may cause temporary stutter or longer load times while shaders are rebuilt.".to_string(),
        },
        KnowledgeEntry {
            id: "kb-explorer-thumbcache-staging".to_string(),
            title: "Windows Explorer Thumbnail Cache Staging".to_string(),
            description: "Staged thumbnail cache cleanup data used by Windows Explorer (ThumbCacheToDelete).".to_string(),
            risk: Risk::Safe,
            known_folder: Some(KnownFolder::LocalAppData),
            relative_pattern: r"Microsoft\Windows\Explorer\ThumbCacheToDelete".to_string(),
            notes: "Safe. Windows Explorer regenerates thumbnail cache data as needed.".to_string(),
        },
        KnowledgeEntry {
            id: "kb-explorer-icon-web-cache-leaves".to_string(),
            title: "Explorer Icon and Thumbnail Cache Databases (out of scope)".to_string(),
            description: "Per-file Explorer cache databases (iconcache_*.db and thumbcache_*.db) stored directly inside the Explorer folder alongside non-cache data.".to_string(),
            risk: Risk::Unknown,
            known_folder: Some(KnownFolder::LocalAppData),
            relative_pattern: r"Microsoft\Windows\Explorer\iconcache_*.db; Microsoft\Windows\Explorer\thumbcache_*.db".to_string(),
            notes: "Out of scope for Sweeper v1. No safe leaf-scoped cleanup exists: these are individual database files interleaved with non-cache Explorer data, and the allowlist can only express whole-directory contents scope (no per-file patterns). Explorer holds the databases open while it is running, so safe removal would require stopping Explorer or a tool-driven API, which is outside the explicit user-action model. The only cleanable Explorer cache is the ThumbCacheToDelete staging folder, which Windows itself stages for deletion.".to_string(),
        },
        KnowledgeEntry {
            id: "kb-browser-extra-caches".to_string(),
            title: "Additional Browser Caches (Firefox, Brave)".to_string(),
            description: "Regenerable browser cache locations for Firefox profiles (cache2, startupCache) and Brave Chromium cache leaf folders.".to_string(),
            risk: Risk::Safe,
            known_folder: Some(KnownFolder::LocalAppData),
            relative_pattern: r"Mozilla\Firefox\Profiles\*\cache2; BraveSoftware\Brave-Browser\User Data\...\Cache".to_string(),
            notes: "Safe. Only explicit cache leaf folders/profile cache locations are targeted; browser profile data, cookies, history, and settings are not targeted.".to_string(),
        },
        KnowledgeEntry {
            id: "kb-vivaldi-opera-browser-caches".to_string(),
            title: "Vivaldi and Opera Browser Caches".to_string(),
            description: "Regenerable cache locations for the Vivaldi and Opera browsers: HTTP Cache, Code Cache, and GPUCache leaves under Vivaldi User Data profiles and Opera channel profiles.".to_string(),
            risk: Risk::Safe,
            // Anchors the HTTP-cache leaves listed in `relative_pattern` (all LocalAppData).
            // Opera's Code Cache and GPUCache leaves are separate targets anchored to
            // RoamingAppData (the Roaming Opera profile tree).
            known_folder: Some(KnownFolder::LocalAppData),
            relative_pattern: r"Vivaldi\User Data\Default\Cache; Vivaldi\User Data\Profile *\Cache; Opera Software\Opera *\Cache; Opera Software\Opera *\Default\Cache".to_string(),
            notes: "Safe. Only explicit cache leaf folders are targeted; browser profile data (history, cookies, bookmarks, settings) is not targeted. Opera HTTP Cache lives under LocalAppData; Opera Code Cache and GPUCache live under the RoamingAppData Opera Default profile tree (Opera One 104+ / Opera GX 119+).".to_string(),
        },
        KnowledgeEntry {
            id: "kb-app-runtime-caches".to_string(),
            title: "Application Runtime Caches (Discord, Slack, Teams, Spotify, Steam)".to_string(),
            description: "Regenerable cache data stored by desktop apps such as Discord, Slack, classic Teams, Spotify, and Steam's embedded browser.".to_string(),
            risk: Risk::Safe,
            known_folder: Some(KnownFolder::RoamingAppData),
            relative_pattern: r"discord\Cache; Slack\Cache; Microsoft\Teams\Cache; Spotify\Data; Steam\htmlcache\Cache".to_string(),
            notes: "Safe. Clearing app caches can free space and may require apps to rebuild cached assets on next launch.".to_string(),
        },
    ]
}

fn developer_and_elevation_knowledge_entries() -> Vec<KnowledgeEntry> {
    vec![
        KnowledgeEntry {
            id: "kb-windows-temp".to_string(),
            title: "Windows Temp".to_string(),
            description: "Windows system temporary files under C:\\Windows\\Temp. Contents can be recreated, but active installers or system tasks may still be using files.".to_string(),
            risk: Risk::Caution,
            known_folder: Some(KnownFolder::Windows),
            relative_pattern: "Temp".to_string(),
            notes: "Caution. Requires administrator elevation. Only this explicit audited subfolder is cleanable; C:\\Windows itself remains a Protected container root.".to_string(),
        },
        KnowledgeEntry {
            id: "kb-windows-update-download-cache".to_string(),
            title: "Windows Update Download Cache".to_string(),
            description: "Downloaded Windows Update payloads in C:\\Windows\\SoftwareDistribution\\Download. Clearing can reclaim space but may require updates to be downloaded again.".to_string(),
            risk: Risk::Caution,
            known_folder: Some(KnownFolder::Windows),
            relative_pattern: "SoftwareDistribution\\Download".to_string(),
            notes: "Caution. Requires administrator elevation. Explicitly audited Phase 6 target only; C:\\Windows remains Protected.".to_string(),
        },
        KnowledgeEntry {
            id: "kb-windows-prefetch".to_string(),
            title: "Windows Prefetch".to_string(),
            description: "Windows Prefetch data in C:\\Windows\\Prefetch used to optimize application startup. Removing contents is recoverable but may temporarily slow launches.".to_string(),
            risk: Risk::Caution,
            known_folder: Some(KnownFolder::Windows),
            relative_pattern: "Prefetch".to_string(),
            notes: "Caution. Requires administrator elevation. Explicitly audited Phase 6 target only; C:\\Windows remains Protected.".to_string(),
        },
        KnowledgeEntry {
            id: "kb-programdata-package-cache".to_string(),
            title: "ProgramData Package Cache".to_string(),
            description: "Installer package cache in C:\\ProgramData\\Package Cache used by some installers for repair, modify, or uninstall operations.".to_string(),
            risk: Risk::Caution,
            known_folder: Some(KnownFolder::ProgramData),
            relative_pattern: "Package Cache".to_string(),
            notes: "Caution. Requires administrator elevation. Explicitly audited Phase 6 target only; C:\\ProgramData remains a Protected container root.".to_string(),
        },
        KnowledgeEntry {
            id: "kb-windows-delivery-optimization-cache".to_string(),
            title: "Windows Delivery Optimization Cache".to_string(),
            description: "Windows Delivery Optimization cache used to store update/content download fragments that may be shared with other devices on the network.".to_string(),
            risk: Risk::Caution,
            known_folder: Some(KnownFolder::Windows),
            relative_pattern: r"ServiceProfiles\NetworkService\AppData\Local\Microsoft\Windows\DeliveryOptimization\Cache".to_string(),
            notes: "Caution. Requires administrator elevation. Explicitly audited Phase 6 target only; clearing may force Windows to re-download update content.".to_string(),
        },
        KnowledgeEntry {
            id: "kb-programdata-wer-report-archive".to_string(),
            title: "Windows Error Reporting ReportArchive (System)".to_string(),
            description: "Archived Windows Error Reporting (WER) diagnostic reports stored system-wide under ProgramData.".to_string(),
            risk: Risk::Caution,
            known_folder: Some(KnownFolder::ProgramData),
            relative_pattern: r"Microsoft\Windows\WER\ReportArchive".to_string(),
            notes: "Caution. Requires administrator elevation. Explicitly audited Phase 6 target only; removes historical diagnostic archives, not active user data.".to_string(),
        },
        KnowledgeEntry {
            id: "kb-programdata-wer-report-queue".to_string(),
            title: "Windows Error Reporting ReportQueue (System)".to_string(),
            description: "Queued Windows Error Reporting (WER) diagnostic reports stored system-wide under ProgramData awaiting upload/processing.".to_string(),
            risk: Risk::Caution,
            known_folder: Some(KnownFolder::ProgramData),
            relative_pattern: r"Microsoft\Windows\WER\ReportQueue".to_string(),
            notes: "Caution. Requires administrator elevation. Explicitly audited Phase 6 target only; removes queued diagnostic reports, not active user data.".to_string(),
        },
        KnowledgeEntry {
            id: "kb-windows-defender-scan-history".to_string(),
            title: "Windows Defender Scan History (Quick)".to_string(),
            description: "Microsoft Defender quick-scan history/result data stored under ProgramData. Cleaning removes historical scan records, not malware definitions or active protection.".to_string(),
            risk: Risk::Caution,
            known_folder: Some(KnownFolder::ProgramData),
            relative_pattern: r"Microsoft\Windows Defender\Scans\History\Results\Quick".to_string(),
            notes: "Caution. Requires administrator elevation. Explicitly audited Phase 6 target only; removes Defender quick-scan history results only.".to_string(),
        },
        KnowledgeEntry {
            id: "kb-rustup".to_string(),
            title: "Rustup Toolchains".to_string(),
            description: "Rust toolchains, targets, and components managed by rustup (compilers, stdlibs, and installed targets).".to_string(),
            risk: Risk::Unknown,
            known_folder: Some(KnownFolder::UserProfile),
            relative_pattern: ".rustup".to_string(),
            notes: "Display-only in Analyzer. Removing toolchains/components can break development environments and require re-download/reinstall; not an allowlisted clean target.".to_string(),
        },
        KnowledgeEntry {
            id: "kb-msix-packages".to_string(),
            title: "Microsoft Store / MSIX Packages".to_string(),
            description: "Packaged app installation state and data under LocalAppData\\Packages used by Microsoft Store and MSIX/UWP apps.".to_string(),
            risk: Risk::Unknown,
            known_folder: Some(KnownFolder::LocalAppData),
            relative_pattern: "Packages".to_string(),
            notes: "Display-only in Analyzer. Package contents may contain app data/state and app registrations; not safe for generic cleaning.".to_string(),
        },
        KnowledgeEntry {
            id: "kb-developer-package-caches".to_string(),
            title: "Developer Package Manager Caches (pnpm, Yarn, Bun)".to_string(),
            description: "Package manager download caches for pnpm, Yarn, and Bun stored in user-local locations.".to_string(),
            risk: Risk::Caution,
            known_folder: Some(KnownFolder::UserProfile),
            relative_pattern: r"pnpm-cache; Yarn\Cache; .bun\install\cache".to_string(),
            notes: "Caution. Contents are generally re-downloadable/rebuildable, but clearing can slow installs/builds and require network access to restore dependencies.".to_string(),
        },
        KnowledgeEntry {
            id: "kb-uv-cache".to_string(),
            title: "uv Cache".to_string(),
            description: "uv package download cache directory under LocalAppData.".to_string(),
            risk: Risk::Caution,
            known_folder: Some(KnownFolder::LocalAppData),
            relative_pattern: r"uv\cache".to_string(),
            notes: "Cleanable via the `uv-cache` Target using whole-cache scope: the entire explicitly scoped `uv\\cache` directory is removed as one unit, never per-file and never by age. The cache is fully regenerated by uv on the next run. Per-file age-based deletion is prohibited because it can corrupt uv's hardlink/archive index and leave a partially cleared cache when files are still in use by a running uv process; `uv cache clean` would be an equally safe tool-driven alternative.".to_string(),
        },
        KnowledgeEntry {
            id: "kb-maven-repository-cache".to_string(),
            title: "Maven Caches (~/.m2)".to_string(),
            description: "Maven user-local caches. The Maven Wrapper distribution cache (.m2\\wrapper\\dists) is a cleanable whole-cache Target; the local repository (.m2\\repository) is shown for context only.".to_string(),
            risk: Risk::Caution,
            known_folder: Some(KnownFolder::UserProfile),
            relative_pattern: r".m2\wrapper\dists (cleanable); .m2\repository (display only)".to_string(),
            notes: "Cleanable via the `maven-wrapper-dists` Target using whole-cache scope: the entire explicitly scoped, regenerable wrapper-download directory is removed as one unit, never per-file and never by age. The `.m2\\repository` local repository is deliberately NOT targeted: it mixes downloaded dependencies with locally installed (`mvn install`) artifacts that cannot be re-downloaded, so per-file or whole-directory deletion would irreversibly delete locally installed artifacts. Excluding the local repository entirely is the safe mechanism; a tool-driven clean that reliably excludes locally installed artifacts would also be acceptable.".to_string(),
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
            "whole_cache" => Scope::WholeCache,
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

/// Deterministic fuzz entrypoint for allowlist parsing/linting.
/// Accepts arbitrary bytes (valid UTF-8 slices are linted) and must never panic.
pub fn fuzz_allowlist_bytes(data: &[u8]) {
    if let Ok(text) = std::str::from_utf8(data) {
        let _ = lint::lint_allowlist_toml(text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_allowlist_is_valid_and_includes_elevation_targets() {
        let targets = embedded_targets();
        assert!(!targets.is_empty());
        let audited_elevated = [
            "windows-temp",
            "windows-update-download-cache",
            "windows-prefetch",
            "programdata-package-cache",
            "windows-delivery-optimization-cache",
            "programdata-wer-report-archive",
            "programdata-wer-report-queue",
            "windows-defender-scan-history",
        ];
        for id in audited_elevated {
            let t = targets
                .iter()
                .find(|t| t.id == id)
                .unwrap_or_else(|| panic!("missing Phase 6 elevation target `{id}`"));
            assert_eq!(t.tier, Tier::Tier3, "{id}");
            assert_eq!(t.risk, Risk::Caution, "{id}");
            assert!(t.requires_elevation, "{id}");
            assert!(matches!(t.scope, Scope::Contents), "{id}");
            assert!(t.min_age_hours >= 24, "{id}");
        }
        assert!(targets.iter().all(|t| {
            if audited_elevated.contains(&t.id.as_str()) {
                t.requires_elevation
            } else {
                matches!(t.tier, Tier::Tier1 | Tier::Tier2) && !t.requires_elevation
            }
        }));
    }

    fn single_target_allowlist_toml(relative_subpath: &str) -> String {
        format!(
            r#"
[[targets]]
id="x"
name="X"
tier="Tier1"
risk="Safe"
known_folder="LocalAppData"
relative_subpath="{relative_subpath}"
scope="contents"
min_age_hours=24
requires_elevation=false
description="x"
"#
        )
    }

    #[test]
    fn lint_rejects_globstar() {
        let err = lint_allowlist_toml(&single_target_allowlist_toml(r"Foo\\**\\Bar")).unwrap_err();
        assert!(err.to_string().contains("**"), "{err}");
    }

    #[test]
    fn lint_rejects_wildcard_in_final_segment() {
        let err = lint_allowlist_toml(&single_target_allowlist_toml(r"Foo\\*")).unwrap_err();
        assert!(err.to_string().contains("final path segment"), "{err}");
    }

    #[test]
    fn fuzz_allowlist_bytes_handles_binary_malformed_and_valid() {
        fuzz_allowlist_bytes(&[0xff, 0xfe, 0x00, 0x80]);
        fuzz_allowlist_bytes(b"not = [valid toml");
        let valid = r#"
[[targets]]
id="x"
name="X"
tier="Tier1"
risk="Safe"
known_folder="LocalAppData"
relative_subpath="Temp"
scope="contents"
min_age_hours=24
requires_elevation=false
description="x"
"#;
        fuzz_allowlist_bytes(valid.as_bytes());
    }

    fn utf8_overlap_toml(subpath_a: &str, subpath_b: &str) -> String {
        format!(
            r#"
[[targets]]
id="a"
name="A"
tier="Tier2"
risk="Safe"
known_folder="LocalAppData"
relative_subpath='{subpath_a}'
scope="contents"
min_age_hours=24
requires_elevation=false
description="x"

[[targets]]
id="b"
name="B"
tier="Tier2"
risk="Safe"
known_folder="LocalAppData"
relative_subpath='{subpath_b}'
scope="contents"
min_age_hours=24
requires_elevation=false
description="y"
"#
        )
    }

    #[test]
    fn lint_non_overlapping_utf8_wildcard_and_literal() {
        let toml = utf8_overlap_toml("\u{00E9}*\\Cache", "a\u{00E9}\\Cache");
        assert!(lint_allowlist_toml(&toml).is_ok());
    }

    #[test]
    fn lint_non_overlapping_utf8_wildcards() {
        let toml = utf8_overlap_toml("\u{00E9}*\\Cache", "a\u{00E9}*\\Cache");
        assert!(lint_allowlist_toml(&toml).is_ok());
    }

    #[test]
    fn lint_overlapping_utf8_wildcard_and_literal() {
        let toml = utf8_overlap_toml("\u{00E9}*\\Cache", "\u{00E9}x\\Cache");
        let err = lint_allowlist_toml(&toml).unwrap_err();
        assert!(matches!(err, LintError::Overlap { .. }));
    }

    #[test]
    fn fuzz_allowlist_bytes_handles_utf8_overlap_pairs_without_panic() {
        fuzz_allowlist_bytes(utf8_overlap_toml("\u{00E9}*\\Cache", "a\u{00E9}\\Cache").as_bytes());
        fuzz_allowlist_bytes(utf8_overlap_toml("\u{00E9}*\\Cache", "a\u{00E9}*\\Cache").as_bytes());
        fuzz_allowlist_bytes(utf8_overlap_toml("\u{00E9}*\\Cache", "\u{00E9}x\\Cache").as_bytes());
    }
}
