//! Ancestor-chain cache and link-point inspection for `remover`.
//!
//! Only immutable path strings are cached; safety verdicts are never stored.
//! Every call still opens and inspects each ancestor handle at deletion time,
//! so ADR 0003 handle-based revalidation and TOCTOU protection are unchanged.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use sweeper_winfs::{self, ReparseTagClass, WinfsError};

use crate::paths::CanonicalPath;

/// Maximum number of directory-level ancestor chains kept in the cache.
/// Bounding it keeps memory flat even for a Clean run over a very large tree.
pub(super) const ANCESTOR_CHAIN_CACHE_CAPACITY: usize = 512;

/// Retained-cost budget, in segment slots across all cached chains. A chain of
/// depth `d` retains `1 + 2 + ... + d` segment slots because every canonical
/// prefix owns its segment strings. Bounding the *complete* cost (not just the
/// entry count) stops a handful of very deep directories from exhausting memory.
pub(super) const ANCESTOR_CHAIN_RETAINED_COST_BUDGET: usize = 16 * 1024;

/// Precomputed canonical ancestor paths for one directory level.
#[derive(Debug, Clone)]
pub(super) struct AncestorChain {
    /// Canonical paths from the drive root down to and including this level.
    pub(super) paths: Vec<CanonicalPath>,
}

/// Lazily yield canonical prefixes from the drive root down to and including
/// `directory_level`, skipping segments that fail lexical parsing.
pub(super) fn ancestor_prefixes(
    directory_level: &CanonicalPath,
) -> impl Iterator<Item = CanonicalPath> + '_ {
    let drive_root = PathBuf::from(format!(r"{}\", directory_level.drive_prefix()));
    directory_level
        .segments()
        .scan(drive_root, |current, segment| {
            current.push(segment);
            Some(current.clone())
        })
        .filter_map(|prefix| CanonicalPath::parse_lexical(&prefix.to_string_lossy()).ok())
}

impl AncestorChain {
    fn build(directory_level: &CanonicalPath) -> Self {
        Self {
            paths: ancestor_prefixes(directory_level).collect(),
        }
    }
}

/// Canonical prefixes from the drive root down to `directory_level`, generated
/// lazily. `None` means the caller should stop inspecting: either the prefix
/// did not parse or it does not exist on disk.
pub(super) fn inspect_ancestor_prefix(prefix: &CanonicalPath) -> Result<Option<bool>, String> {
    #[cfg(test)]
    ANCESTOR_PREFIX_CHECKS.with(|checks| checks.set(checks.get() + 1));
    match sweeper_winfs::open_for_inspection(Path::new(prefix.as_str()))
        .and_then(|h| sweeper_winfs::inspect_reparse_point(&h))
    {
        Ok(ReparseTagClass::LinkPoint) => Ok(Some(true)),
        Ok(_) => Ok(Some(false)),
        Err(WinfsError::Win32 { code, .. }) if sweeper_winfs::is_not_found(code) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

/// Inspect one prefix at a time for an over-budget chain. Each canonical path
/// is dropped before the next is generated, so transient memory stays O(depth)
/// rather than O(depth²) even though retained cache cost stays zero.
pub(super) fn inspect_deep_ancestors(directory_level: &CanonicalPath) -> Result<bool, String> {
    for prefix in ancestor_prefixes(directory_level) {
        match inspect_ancestor_prefix(&prefix)? {
            Some(true) => return Ok(true),
            Some(false) => {}
            None => return Ok(false),
        }
    }
    Ok(false)
}

/// Segment slots retained by one chain of the given depth: every canonical
/// prefix owns its segment strings, so the cost grows quadratically.
pub(super) fn ancestor_chain_cost(segment_count: usize) -> usize {
    segment_count.saturating_mul(segment_count.saturating_add(1)) / 2
}

thread_local! {
    pub(super) static ANCESTOR_CHAIN_CACHE: RefCell<HashMap<String, AncestorChain>> =
        RefCell::new(HashMap::new());
    pub(super) static ANCESTOR_CHAIN_RETAINED_COST: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    #[cfg(test)]
    pub(super) static ANCESTOR_CHAIN_BUILDS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    #[cfg(test)]
    pub(super) static ANCESTOR_PREFIX_CHECKS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Run `f` with the ancestor chain for `directory_level`, building it at most
/// once per level. `directory_level` is always a directory: for a per-item
/// check it is the item's parent directory, so siblings share one entry.
/// Entries are immutable path strings only, never safety verdicts. Expensive
/// deep chains are built for this call but not retained; ordinary shallow
/// directories stay cached within both the entry cap and the cost budget.
pub(super) fn with_ancestor_chain<R, F>(directory_level: &CanonicalPath, f: F) -> R
where
    F: FnOnce(&AncestorChain) -> R,
{
    let cost = ancestor_chain_cost(directory_level.segment_count());
    if cost > ANCESTOR_CHAIN_RETAINED_COST_BUDGET {
        // A deep chain would dominate the budget on its own. Still build and
        // inspect every prefix for this item, but never retain it in the cache.
        let chain = AncestorChain::build(directory_level);
        #[cfg(test)]
        ANCESTOR_CHAIN_BUILDS.with(|builds| builds.set(builds.get() + 1));
        return f(&chain);
    }
    ANCESTOR_CHAIN_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if !cache.contains_key(directory_level.as_str()) {
            let retained = ANCESTOR_CHAIN_RETAINED_COST.get();
            if cache.len() >= ANCESTOR_CHAIN_CACHE_CAPACITY
                || retained.saturating_add(cost) > ANCESTOR_CHAIN_RETAINED_COST_BUDGET
            {
                cache.clear();
                ANCESTOR_CHAIN_RETAINED_COST.set(0);
            }
            let chain = AncestorChain::build(directory_level);
            cache.insert(directory_level.as_str().to_string(), chain);
            ANCESTOR_CHAIN_RETAINED_COST.set(ANCESTOR_CHAIN_RETAINED_COST.get() + cost);
            #[cfg(test)]
            ANCESTOR_CHAIN_BUILDS.with(|builds| builds.set(builds.get() + 1));
        }
        let chain = cache
            .get(directory_level.as_str())
            .expect("ancestor chain was just ensured to exist");
        f(chain)
    })
}

#[cfg(test)]
pub(super) fn ancestor_chain_cache_len() -> usize {
    ANCESTOR_CHAIN_CACHE.with(|cache| cache.borrow().len())
}

#[cfg(test)]
pub(super) fn ancestor_chain_retained_cost() -> usize {
    ANCESTOR_CHAIN_RETAINED_COST.with(|cost| cost.get())
}

#[cfg(test)]
pub(super) fn ancestor_chain_build_count() -> usize {
    ANCESTOR_CHAIN_BUILDS.with(|builds| builds.get())
}
