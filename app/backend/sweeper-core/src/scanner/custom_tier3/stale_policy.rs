use super::*;
use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

pub(super) fn resolve_stale_search_roots(opts: &ScanSettingsOptions) -> Vec<String> {
    if !opts.tier3_project_search_roots.is_empty() {
        return opts.tier3_project_search_roots.clone();
    }
    default_tier3_project_search_roots()
}

fn push_unique_search_root(out: &mut Vec<String>, canon: &CanonicalPath) {
    if is_disallowed_stale_search_root(canon) {
        return;
    }
    let s = canon.to_string();
    if !out.iter().any(|e| e.eq_ignore_ascii_case(&s)) {
        out.push(s);
    }
}

fn is_ignored_profile_name(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "public" | "default" | "default user" | "all users"
    )
}

fn is_eligible_profile_child(name: &str, path: &Path) -> bool {
    if name.starts_with('.') || is_ignored_child_name(name) {
        return false;
    }
    !is_policy_protected_path(path)
}

fn is_ignored_child_name(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "appdata" | "ntuser" | "application data" | "local settings"
    )
}

fn is_policy_protected_path(path: &Path) -> bool {
    let s = path.to_string_lossy();
    let Ok(cp) = CanonicalPath::parse_lexical(&s) else {
        return true;
    };
    policy::is_subtree_protected(&cp)
        || policy::is_ancestor_of_subtree_protected(&cp)
        || policy::is_protected_container_root(&cp)
}

fn is_plain_child_dir(entry: &std::fs::DirEntry) -> bool {
    let Ok(ft) = entry.file_type() else {
        return false;
    };
    if !ft.is_dir() || ft.is_symlink() {
        return false;
    }
    let Ok(meta) = std::fs::symlink_metadata(entry.path()) else {
        return false;
    };
    (meta.file_attributes() & 0x0400) == 0
}

fn append_profile_children(profile_dir: &Path, roots: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(profile_dir) else {
        return;
    };
    for entry in entries.flatten() {
        if !is_plain_child_dir(&entry) {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        let path = entry.path();
        if !is_eligible_profile_child(&name, &path) {
            continue;
        }
        if let Ok(canon) = CanonicalPath::from_existing_path(&path.to_string_lossy()) {
            push_unique_search_root(roots, &canon.into_canonical());
        }
    }
}

fn append_user_profile_roots(users_dir: &Path, roots: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(users_dir) else {
        return;
    };
    for entry in entries.flatten() {
        if !is_plain_child_dir(&entry) {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if is_ignored_profile_name(&name) {
            continue;
        }
        append_profile_children(&entry.path(), roots);
    }
}

pub(super) fn default_tier3_project_search_roots() -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    if let Some(canon) = user_profile_search_root() {
        append_profile_children(Path::new(canon.as_str()), &mut out);
    }
    for letter in ['C', 'D', 'E', 'F', 'G'] {
        let base = format!("{letter}:\\Users");
        if Path::new(&base).is_dir() {
            append_user_profile_roots(Path::new(&base), &mut out);
        }
    }
    out
}

pub(super) fn user_profile_search_root() -> Option<CanonicalPath> {
    let p = sweeper_winfs::resolve_known_folder(KnownFolderId::UserProfile).ok()?;
    let v = CanonicalPath::from_existing_path(&p.to_string_lossy()).ok()?;
    Some(v.into_canonical())
}

pub(super) fn normalize_stale_search_root(search_root: &str) -> Option<PathBuf> {
    let root = CanonicalPath::from_existing_path(search_root)
        .ok()?
        .into_canonical();
    if is_disallowed_stale_search_root(&root) {
        return None;
    }
    Some(PathBuf::from(root.as_str()))
}

pub(super) fn is_disallowed_stale_search_root(root: &CanonicalPath) -> bool {
    if policy::is_subtree_protected(root) {
        return true;
    }
    if policy::is_ancestor_of_subtree_protected(root) {
        return true;
    }
    policy::is_protected_container_root(root)
}

pub(super) fn is_plain_directory_entry(ent: &sweeper_winfs::DirEntry) -> bool {
    ent.is_dir && matches!(ent.reparse, ReparseTagClass::NotReparse)
}

pub(super) fn should_traverse_stale_child(ent: &sweeper_winfs::DirEntry) -> bool {
    if !is_plain_directory_entry(ent) {
        return false;
    }
    let name = ent.name.to_string_lossy();
    if is_stale_traversal_skip_name(&name, &ent.path) {
        return false;
    }
    !is_disallowed_tier3_scan_path(&ent.path)
}

pub(super) fn is_stale_traversal_skip_name(name: &str, path: &Path) -> bool {
    if is_known_stale_skip_name(name) {
        return true;
    }
    if is_rust_target_artifact(name, path) {
        return true;
    }
    name.eq_ignore_ascii_case("node_modules")
}

pub(super) fn is_rust_target_artifact(name: &str, path: &Path) -> bool {
    name.eq_ignore_ascii_case("target") && path.join("..\\Cargo.toml").exists()
}

pub(super) fn is_known_stale_skip_name(name: &str) -> bool {
    matches_known_skip_name(name)
}

pub(super) fn matches_known_skip_name(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "$recycle.bin"
            | "system volume information"
            | "windows"
            | "program files"
            | "program files (x86)"
    )
}

pub(super) fn is_disallowed_tier3_scan_path(path: &Path) -> bool {
    let Ok(canon) = CanonicalPath::parse_lexical(&path.to_string_lossy()) else {
        return true;
    };
    if is_policy_disallowed_tier3_path(&canon) {
        return true;
    }
    has_disallowed_tier3_anchor(&canon)
}

pub(super) fn is_policy_disallowed_tier3_path(canon: &CanonicalPath) -> bool {
    if policy::is_subtree_protected(canon) {
        return true;
    }
    if policy::is_ancestor_of_subtree_protected(canon) {
        return true;
    }
    policy::is_protected_container_root(canon)
}

pub(super) fn has_disallowed_tier3_anchor(canon: &CanonicalPath) -> bool {
    let s = canon.as_str().to_ascii_uppercase();
    is_disallowed_system_anchor(&s) || s.contains(r"\SYSTEM VOLUME INFORMATION")
}

pub(super) fn is_disallowed_system_anchor(s: &str) -> bool {
    if s.starts_with(r"C:\WINDOWS") {
        return true;
    }
    if s.starts_with(r"C:\PROGRAM FILES") {
        return true;
    }
    s.starts_with(r"C:\PROGRAM FILES (X86)")
}

pub(super) fn is_stale_rust_target_dir(
    target_dir: &Path,
    parent: &Path,
    stale_threshold: Duration,
    now: SystemTime,
) -> bool {
    let cargo_toml = parent.join("Cargo.toml");
    if !cargo_toml.is_file() {
        return false;
    }
    let _ = target_dir;
    is_path_stale_enough(&cargo_toml, stale_threshold, now)
}

pub(super) fn is_stale_node_modules_dir(
    parent: &Path,
    stale_threshold: Duration,
    now: SystemTime,
) -> bool {
    let mut found = false;
    for lf in node_lockfiles() {
        let p = parent.join(lf);
        if !p.is_file() {
            continue;
        }
        found = true;
        if !is_path_stale_enough(&p, stale_threshold, now) {
            return false;
        }
    }
    found
}

pub(super) fn node_lockfiles() -> [&'static str; 5] {
    [
        "package-lock.json",
        "pnpm-lock.yaml",
        "yarn.lock",
        "bun.lock",
        "bun.lockb",
    ]
}

pub(super) fn is_path_stale_enough(
    path: &Path,
    stale_threshold: Duration,
    now: SystemTime,
) -> bool {
    let mtime = match std::fs::metadata(path).and_then(|m| m.modified()) {
        Ok(t) => t,
        Err(_) => return false,
    };
    if mtime > now {
        return false;
    }
    now.duration_since(mtime)
        .map(|age| age >= stale_threshold)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_children_include_dev_and_skip_protected() {
        // Eligibility is decided from the absolute, production-shaped path
        // `C:\Users\<name>\<child>`. Use literal paths so the verdict does not
        // depend on the machine's temp directory: a sandbox nested under a real
        // user profile skews the segment-based policy checks (and canonical
        // handle resolution can fail on locked-down runners), making a
        // filesystem-based test non-deterministic across environments.
        assert!(is_eligible_profile_child(
            "dev",
            Path::new(r"C:\Users\Test\dev")
        ));
        // Known user-data subtrees are never project search roots.
        assert!(!is_eligible_profile_child(
            "Documents",
            Path::new(r"C:\Users\Test\Documents")
        ));
        // AppData is an ignored child name regardless of the absolute path.
        assert!(!is_eligible_profile_child(
            "AppData",
            Path::new(r"C:\Users\Test\AppData")
        ));
        // Dot-directories (e.g. a nested .git) are never search roots.
        assert!(!is_eligible_profile_child(
            ".git",
            Path::new(r"C:\Users\Test\.git")
        ));
    }

    #[test]
    fn ignored_profile_names_are_filtered() {
        for name in ["Public", "Default", "Default User", "All Users"] {
            assert!(
                is_ignored_profile_name(name),
                "expected {name} to be ignored"
            );
        }
        assert!(!is_ignored_profile_name("Example"));
    }
}
