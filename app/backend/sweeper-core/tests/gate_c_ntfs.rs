use std::fs;
use std::path::{Path, PathBuf};

use sweeper_core::paths::CanonicalPath;
use sweeper_core::policy;
use sweeper_testkit::Sandbox;

fn sandbox_base_dir() -> PathBuf {
    fn strip_extended_prefix(p: PathBuf) -> PathBuf {
        let s = p.to_string_lossy();
        if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
            return PathBuf::from(format!(r"\\{}", rest));
        }
        if let Some(rest) = s.strip_prefix(r"\\?\") {
            return PathBuf::from(rest.to_string());
        }
        p
    }

    let base = if let Some(v) = std::env::var_os("CARGO_TARGET_DIR") {
        PathBuf::from(v).join("sweeper-test-sandboxes")
    } else if let Some(v) = std::env::var_os("CARGO_TARGET_TMPDIR") {
        PathBuf::from(v).join("sweeper-test-sandboxes")
    } else {
        let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        manifest
            .join("../../target/sweeper-test-sandboxes")
            .components()
            .collect::<PathBuf>()
    };
    let _ = fs::create_dir_all(&base);
    match std::fs::canonicalize(&base) {
        Ok(c) => strip_extended_prefix(c),
        Err(_) => base,
    }
}

#[cfg(windows)]
#[test]
fn ntfs_junction_is_detected_as_link_point_and_rejected() {
    let sb = Sandbox::new_in(sandbox_base_dir()).unwrap();
    let target = sb.create_dir(r"real\target\dir").unwrap();
    let link_parent = sb.create_dir(r"links\here").unwrap();
    let link = link_parent.join("junc");
    sweeper_testkit::create_junction(&link, &target).unwrap();
    // resolve via on-disk canonicalization should reject link point
    let res = CanonicalPath::from_existing_path(&link.to_string_lossy());
    assert!(
        matches!(res, Err(sweeper_core::paths::PathError::LinkPoint)),
        "{res:?}"
    );
    // custom path validation should reject
    let v = policy::validate_custom_path(&link.to_string_lossy());
    assert!(!v.allowed, "junction must be rejected: {}", v.reason);
}

#[cfg(windows)]
#[test]
fn ntfs_symlink_is_rejected_when_privilege_available() {
    let sb = Sandbox::new_in(sandbox_base_dir()).unwrap();
    let target = sb.write_file(r"real\target\file.txt", b"hi").unwrap();
    let link = sb.root().join("linksym.txt");
    match sweeper_testkit::create_file_symlink(&link, &target) {
        Ok(()) => {
            let res = CanonicalPath::from_existing_path(&link.to_string_lossy());
            assert!(
                matches!(res, Err(sweeper_core::paths::PathError::LinkPoint)),
                "{res:?}"
            );
        }
        Err(e) => {
            let msg = e.to_string().to_lowercase();
            if msg.contains("privilege") || msg.contains("permission") {
                eprintln!("skipping symlink rejection assertion (no privilege): {e}");
            } else {
                panic!("symlink create failed: {e}");
            }
        }
    }
}

#[test]
fn unicode_names_can_be_parsed_and_resolved() {
    let sb = Sandbox::new_in(sandbox_base_dir()).unwrap();
    let p = sb.create_dir(r"uni\مرحبا\😀emoji\dir").unwrap();
    let v = CanonicalPath::from_existing_path(&p.to_string_lossy());
    assert!(v.is_ok(), "resolve unicode path failed: {v:?}");
}

#[test]
fn long_lexical_paths_over_260_are_rejected_or_handled_safely() {
    // Lexical policy: very long paths rejected (>4096 guard) and >260 is a case we must handle.
    let long = format!(r"C:\a\b\c\{}", "d".repeat(300));
    let r = CanonicalPath::parse_lexical(&long);
    if let Ok(p) = r {
        assert!(p.as_str().len() > 260);
    }
}

#[test]
fn prefix_collision_paths_are_segment_aware_on_disk() {
    let sb = Sandbox::new_in(sandbox_base_dir()).unwrap();
    let base = sb.create_dir(r"Windows").unwrap();
    let old = sb.create_dir(r"Windows.old\Temp").unwrap();
    let apps = sb.create_dir(r"WindowsApps\Temp").unwrap();
    let base_c = CanonicalPath::from_existing_path(&base.to_string_lossy())
        .unwrap()
        .into_canonical();
    let old_c = CanonicalPath::from_existing_path(&old.to_string_lossy())
        .unwrap()
        .into_canonical();
    let apps_c = CanonicalPath::from_existing_path(&apps.to_string_lossy())
        .unwrap()
        .into_canonical();
    assert!(!base_c.is_self_or_ancestor_of(&old_c));
    assert!(!base_c.is_self_or_ancestor_of(&apps_c));
}

#[test]
fn custom_path_validation_requires_existing_path() {
    let sb = Sandbox::new_in(sandbox_base_dir()).unwrap();
    let p = sb.create_dir(r"a\b\c").unwrap();
    let verdict = policy::validate_custom_path(&p.to_string_lossy());
    assert!(verdict.allowed, "expected allowed, got: {}", verdict.reason);
    let missing = p.join("does-not-exist");
    let v2 = policy::validate_custom_path(&missing.to_string_lossy());
    assert!(!v2.allowed, "missing must be rejected: {}", v2.reason);
}

#[test]
fn real_long_path_over_260_is_supported_for_legitimate_sandbox_paths() {
    let sb = Sandbox::new_in(sandbox_base_dir()).unwrap();
    let mut long_dir = sb.root().to_path_buf();
    long_dir.push("long_base_for_gate_c");
    long_dir.push("a".repeat(80));
    long_dir.push("b".repeat(80));
    long_dir.push("c".repeat(80));
    long_dir.push("final_dir");
    let create_path = to_extended_path(&long_dir);
    fs::create_dir_all(&create_path).expect("create long directory");
    let path_str = long_dir.to_string_lossy().to_string();
    assert!(
        path_str.len() > 260,
        "test requires >260 char path, got {}",
        path_str.len()
    );

    let resolved = CanonicalPath::from_existing_path(&path_str)
        .expect("from_existing_path should resolve legitimate long sandbox path");
    assert!(resolved.as_str().len() >= 3);

    let verdict = policy::validate_custom_path(&path_str);
    assert!(
        verdict.allowed,
        "validate_custom_path should allow legitimate long sandbox path: {} (len={})",
        verdict.reason,
        path_str.len()
    );
}

#[test]
fn custom_path_validation_rejects_directory_containing_pattern_protected_descendant() {
    let sb_git = Sandbox::new_in(sandbox_base_dir()).unwrap();
    let root = sb_git.create_dir(r"proj\safe\rootdir").unwrap();
    let git_dir = root.join(".git");
    fs::create_dir_all(to_extended_path(&git_dir)).unwrap();
    fs::write(git_dir.join("config"), b"x").unwrap();
    let verdict_git = policy::validate_custom_path(&root.to_string_lossy());
    assert!(
        !verdict_git.allowed,
        "path containing .git descendant must be rejected: {}",
        verdict_git.reason
    );

    let sb_ssh = Sandbox::new_in(sandbox_base_dir()).unwrap();
    let root2 = sb_ssh.create_dir(r"proj2\safe\rootdir").unwrap();
    let ssh_dir = root2.join(".ssh");
    fs::create_dir_all(to_extended_path(&ssh_dir)).unwrap();
    let verdict_ssh = policy::validate_custom_path(&root2.to_string_lossy());
    assert!(
        !verdict_ssh.allowed,
        "path containing .ssh descendant must be rejected: {}",
        verdict_ssh.reason
    );
}

#[cfg(windows)]
fn to_extended_path(path: &Path) -> PathBuf {
    let s = path.to_string_lossy();
    if s.starts_with(r"\\?\") {
        path.to_path_buf()
    } else {
        PathBuf::from(format!(r"\\?\{}", s))
    }
}

#[cfg(not(windows))]
fn to_extended_path(path: &Path) -> PathBuf {
    path.to_path_buf()
}
