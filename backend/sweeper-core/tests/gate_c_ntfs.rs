use sweeper_core::paths::CanonicalPath;
use sweeper_core::policy;
use sweeper_testkit::Sandbox;

#[cfg(windows)]
#[test]
fn ntfs_junction_is_detected_as_link_point_and_rejected() {
    let sb = Sandbox::new().unwrap();
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
    let sb = Sandbox::new().unwrap();
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
    let sb = Sandbox::new().unwrap();
    let p = sb.create_dir(r"uni\مرحبا\😀emoji\dir").unwrap();
    let v = CanonicalPath::from_existing_path(&p.to_string_lossy());
    assert!(v.is_ok(), "resolve unicode path failed: {v:?}");
}

#[test]
fn long_lexical_paths_over_260_are_rejected_or_handled_safely() {
    // Lexical policy: very long paths rejected (>4096 guard) and >260 is a case we must handle.
    let long = format!(r"C:\a\b\c\{}", "d".repeat(300));
    let r = CanonicalPath::parse_lexical(&long);
    // Either parsed or rejected, but must not panic and must be consistent; >260 chars string is valid input to consider.
    if let Ok(p) = r {
        assert!(p.as_str().len() > 260);
    }
}

#[test]
fn prefix_collision_paths_are_segment_aware_on_disk() {
    let sb = Sandbox::new().unwrap();
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
    let sb = Sandbox::new().unwrap();
    let p = sb.create_dir(r"a\b\c").unwrap();
    let verdict = policy::validate_custom_path(&p.to_string_lossy());
    // Sandbox temp paths are deep and outside protected locations; should be allowed.
    assert!(verdict.allowed, "expected allowed, got: {}", verdict.reason);
    let missing = p.join("does-not-exist");
    let v2 = policy::validate_custom_path(&missing.to_string_lossy());
    assert!(!v2.allowed, "missing must be rejected: {}", v2.reason);
}
