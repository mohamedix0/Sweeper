//! Validated-item construction and path-policy checks for `remover`.
//!
//! All raw strings are validated here before a [`ValidatedItem`] can reach a
//! removal API. Keeping the constructors in one focused module lets the parent
//! module stay orchestration-only without changing the public construction
//! paths or any of their validation error strings.

use sweeper_knowledge::Risk;
use sweeper_winfs::FileIdentity;

use crate::paths::CanonicalPath;
use crate::policy;

use super::prechecks::is_protected_root;
use super::{ValidatedItem, ValidatedSource};

/// Named fields for the one place that assembles a `ValidatedItem`.
///
/// Every constructor funnels through [`ItemParts::build`], so the struct-literal
/// assembly exists once instead of being duplicated across constructors.
struct ItemParts {
    id: String,
    target_id: String,
    target_root: String,
    path: String,
    size_bytes: u64,
    mtime_unix_secs: Option<u64>,
    source: ValidatedSource,
    risk: Option<Risk>,
}

impl ItemParts {
    /// The single private construction helper for `ValidatedItem`.
    fn build(self) -> ValidatedItem {
        ValidatedItem {
            id: self.id,
            target_id: self.target_id,
            target_root: self.target_root,
            path: self.path,
            size_bytes: self.size_bytes,
            mtime_unix_secs: self.mtime_unix_secs,
            source: self.source,
            risk: self.risk,
            expected_identity: None,
            identity_required: false,
        }
    }
}

/// Named inputs for a root-style item whose `target_root` and `path` are the
/// same canonical directory.
struct RootItemContext {
    id: String,
    target_id: String,
    canon: String,
    size_bytes: u64,
    mtime_unix_secs: Option<u64>,
    source: ValidatedSource,
}

/// Named inputs for the allowlisted-scan validation pipeline.
struct AllowlistedScanInput {
    id: String,
    target_id: String,
    target_root: String,
    path: String,
    size_bytes: u64,
    mtime_unix_secs: Option<u64>,
}

/// Named inputs for the Tier 3 stale project-directory pipeline.
struct Tier3ProjectDirectoryInput {
    id: String,
    logical_target_id: String,
    path: String,
    size_bytes: u64,
    mtime_unix_secs: Option<u64>,
}

impl ValidatedItem {
    /// Crate-internal constructor for validated construction pipelines and test-support helpers.
    #[cfg(feature = "test-support")]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new_internal(
        id: String,
        target_id: String,
        target_root: String,
        path: String,
        size_bytes: u64,
        mtime_unix_secs: Option<u64>,
        source: ValidatedSource,
        risk: Option<Risk>,
    ) -> Self {
        ItemParts {
            id,
            target_id,
            target_root,
            path,
            size_bytes,
            mtime_unix_secs,
            source,
            risk,
        }
        .build()
    }

    /// Item id.
    pub fn id(&self) -> &str {
        &self.id
    }
    /// Target id.
    pub fn target_id(&self) -> &str {
        &self.target_id
    }
    /// Target root path.
    pub fn target_root(&self) -> &str {
        &self.target_root
    }
    /// Item path.
    pub fn path(&self) -> &str {
        &self.path
    }
    /// Estimated size bytes.
    pub fn size_bytes(&self) -> u64 {
        self.size_bytes
    }
    /// Last modified time (unix seconds), if known.
    pub fn mtime_unix_secs(&self) -> Option<u64> {
        self.mtime_unix_secs
    }

    /// Bind this item to the filesystem identity observed at validation time.
    ///
    /// `None` is recorded as a required-but-missing binding so moment-of-deletion
    /// revalidation fails closed instead of deleting an unverified entry.
    pub fn with_identity_binding(mut self, identity: Option<FileIdentity>) -> Self {
        self.expected_identity = identity;
        self.identity_required = true;
        self
    }

    /// Construct a validated allowlisted scan item for bridge-mediated clean execution.
    ///
    /// Performs basic lexical/path-policy checks (containment within target root, non-protected,
    /// canonical lexical parsing). Moment-of-deletion handle-based revalidation remains mandatory
    /// inside the remover.
    pub fn from_allowlisted_scan(
        id: impl Into<String>,
        target_id: impl Into<String>,
        target_root: impl Into<String>,
        path: impl Into<String>,
        size_bytes: u64,
        mtime_unix_secs: Option<u64>,
    ) -> Result<Self, String> {
        Self::from_allowlisted_scan_input(AllowlistedScanInput {
            id: id.into(),
            target_id: target_id.into(),
            target_root: target_root.into(),
            path: path.into(),
            size_bytes,
            mtime_unix_secs,
        })
    }

    /// Validation pipeline for allowlisted scan items. Takes a single named
    /// context so the logic stays below the argument-count limit while the
    /// public wrapper keeps its established signature.
    fn from_allowlisted_scan_input(input: AllowlistedScanInput) -> Result<Self, String> {
        let AllowlistedScanInput {
            id,
            target_id,
            target_root,
            path,
            size_bytes,
            mtime_unix_secs,
        } = input;

        let root = Self::validated_allowlisted_root(&target_root, "target root")?;
        let item =
            CanonicalPath::parse_lexical(&path).map_err(|e| format!("invalid item path: {e}"))?;

        if !root.is_self_or_ancestor_of(&item) || item.is_same_path(&root) {
            return Err(
                "item path must be inside target root (and not the root itself)".to_string(),
            );
        }
        if is_protected_root(&item) {
            return Err("item path is protected".to_string());
        }

        Ok(ItemParts {
            id,
            target_id: target_id.clone(),
            target_root,
            path,
            size_bytes,
            mtime_unix_secs,
            source: ValidatedSource::AllowlistedTarget { target_id },
            risk: None,
        }
        .build())
    }

    /// Construct a validated allowlisted root item (contents-only; root kept).
    ///
    /// Used when an Analyzer selection resolves to the target root itself: the
    /// root must pass the same depth/protected-path checks as an allowlisted
    /// scan item, but `target_root` and `path` are the same directory so the
    /// remover cleans its contents and keeps the root (Minimum Age applies).
    pub fn from_allowlisted_root(
        id: impl Into<String>,
        target_id: impl Into<String>,
        path: impl Into<String>,
        size_bytes: u64,
    ) -> Result<Self, String> {
        let id = id.into();
        let target_id = target_id.into();
        let path = path.into();
        let canon = Self::validated_allowlisted_root(&path, "allowlisted root")?;
        Ok(Self::new_root_item(RootItemContext {
            id,
            target_id: target_id.clone(),
            canon: canon.to_string(),
            size_bytes,
            mtime_unix_secs: None,
            source: ValidatedSource::AllowlistedTarget { target_id },
        }))
    }

    /// Shared depth/protected-path policy for allowlisted roots. Both the scan
    /// path and the Analyzer-selected root path must use this one definition so
    /// a future policy tightening cannot be applied to only one of them.
    fn validated_allowlisted_root(path: &str, label: &str) -> Result<CanonicalPath, String> {
        let canon =
            CanonicalPath::parse_lexical(path).map_err(|e| format!("invalid {label}: {e}"))?;
        if canon.segment_count() < 3 {
            return Err(format!("{label} depth < 3"));
        }
        if is_protected_root(&canon) {
            return Err(format!("{label} is protected"));
        }
        Ok(canon)
    }

    /// Validate a root path with the strict Tier 3 root policy and return its
    /// canonical string form. Shared by the root-style constructors so the
    /// validation/canonicalisation sequence exists in exactly one place.
    fn validated_tier3_root(path: &str, label: &str) -> Result<String, String> {
        let verdict = policy::validate_tier3_target_root(path);
        if !verdict.allowed {
            return Err(format!("{label} not allowed: {}", verdict.reason));
        }
        CanonicalPath::parse_lexical(path)
            .map(|p| p.to_string())
            .map_err(|e| format!("invalid {label}: {e}"))
    }

    /// Build a root-style item whose `target_root` and `path` are the same
    /// canonical directory. All root-style sources carry `Risk::Caution`.
    fn new_root_item(ctx: RootItemContext) -> Self {
        ItemParts {
            id: ctx.id,
            target_id: ctx.target_id,
            target_root: ctx.canon.clone(),
            path: ctx.canon,
            size_bytes: ctx.size_bytes,
            mtime_unix_secs: ctx.mtime_unix_secs,
            source: ctx.source,
            risk: Some(Risk::Caution),
        }
        .build()
    }

    /// Construct a validated custom-path root item (contents-only clean of the root directory).
    ///
    /// The path is validated with `policy::validate_custom_path` and stored as both `target_root`
    /// and `path` so the remover applies contents-only semantics (root directory kept).
    pub fn from_custom_path_root(
        id: impl Into<String>,
        path: impl Into<String>,
        size_bytes: u64,
    ) -> Result<Self, String> {
        let id = id.into();
        let path = path.into();
        let verdict = policy::validate_custom_path(&path);
        if !verdict.allowed {
            return Err(format!("custom path not allowed: {}", verdict.reason));
        }
        let canon = CanonicalPath::parse_lexical(&path)
            .map_err(|e| format!("invalid custom path: {e}"))?
            .to_string();
        Ok(Self::new_root_item(RootItemContext {
            id,
            target_id: "custom-path".to_string(),
            canon,
            size_bytes,
            mtime_unix_secs: None,
            source: ValidatedSource::CustomPath,
        }))
    }

    /// Construct a validated Tier 3 recycle-bin root item (contents-only; root kept).
    ///
    /// Used for user-scoped recycle-bin cleanup. The root must pass Tier 3 safety validation.
    pub fn from_tier3_recycle_bin_root(
        id: impl Into<String>,
        path: impl Into<String>,
        size_bytes: u64,
    ) -> Result<Self, String> {
        let id = id.into();
        let path = path.into();
        let canon = Self::validated_tier3_root(&path, "recycle bin path")?;
        Ok(Self::new_root_item(RootItemContext {
            id,
            target_id: "recycle-bin".to_string(),
            canon,
            size_bytes,
            mtime_unix_secs: None,
            source: ValidatedSource::Tier3RecycleBin,
        }))
    }

    /// Construct a validated Tier 3 stale project directory item.
    ///
    /// `path` is the stale `target/` or `node_modules/` directory to remove. `target_root` is the
    /// same directory and may be deleted after its contents are safely cleaned (unlike custom paths).
    pub fn from_tier3_project_directory(
        id: impl Into<String>,
        logical_target_id: impl Into<String>,
        path: impl Into<String>,
        size_bytes: u64,
        mtime_unix_secs: Option<u64>,
    ) -> Result<Self, String> {
        Self::from_tier3_project_directory_input(Tier3ProjectDirectoryInput {
            id: id.into(),
            logical_target_id: logical_target_id.into(),
            path: path.into(),
            size_bytes,
            mtime_unix_secs,
        })
    }

    /// Validation pipeline for stale project directories. Takes a single named
    /// context so the logic stays below the argument-count limit while the
    /// public wrapper keeps its established signature.
    fn from_tier3_project_directory_input(
        input: Tier3ProjectDirectoryInput,
    ) -> Result<Self, String> {
        let Tier3ProjectDirectoryInput {
            id,
            logical_target_id,
            path,
            size_bytes,
            mtime_unix_secs,
        } = input;
        let canon = Self::validated_tier3_root(&path, "stale project directory")?;
        let source = ValidatedSource::Tier3ProjectDirectory {
            target_id: logical_target_id.clone(),
        };
        Ok(Self::new_root_item(RootItemContext {
            id,
            target_id: logical_target_id,
            canon,
            size_bytes,
            mtime_unix_secs,
            source,
        }))
    }

    /// Construct a validated whole-cache root item (issue #41).
    ///
    /// `path` is the entire cache directory and is also the `target_root`:
    /// the remover cleans its contents and then removes the root itself. The
    /// root must pass the strict Tier 3 root policy (protected paths, link
    /// points, depth >= 3, must exist). Age filtering is disabled for this
    /// source so the cache is never left partially cleared by Minimum Age.
    pub fn from_whole_cache_root(
        id: impl Into<String>,
        target_id: impl Into<String>,
        path: impl Into<String>,
        size_bytes: u64,
    ) -> Result<Self, String> {
        let id = id.into();
        let target_id = target_id.into();
        let path = path.into();
        let canon = Self::validated_tier3_root(&path, "whole-cache root")?;
        let source = ValidatedSource::WholeCache {
            target_id: target_id.clone(),
        };
        Ok(Self::new_root_item(RootItemContext {
            id,
            target_id,
            canon,
            size_bytes,
            mtime_unix_secs: None,
            source,
        }))
    }
}
