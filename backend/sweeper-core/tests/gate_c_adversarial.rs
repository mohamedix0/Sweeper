use sweeper_core::paths::{CanonicalPath, PathError};
use sweeper_core::policy;

#[test]
fn rejects_traversal_and_dots() {
    for s in [
        r"C:\Temp\..\Windows",
        r"C:\Temp\.\foo",
        r"C:\.\Temp",
        r"C:\..\Temp",
        r"C:\a\..\b",
    ] {
        assert!(
            CanonicalPath::parse_lexical(s).is_err(),
            "should reject {s}"
        );
        assert!(!policy::validate_custom_path_lexical(s).allowed, "{s}");
    }
}

#[test]
fn rejects_mixed_separators_normalizes_safely() {
    // Mixed separators are allowed as input form but normalized; must not enable traversal.
    let ok = CanonicalPath::parse_lexical("C:/Users/Test/AppData/Local/TempDir").unwrap();
    assert_eq!(ok.as_str(), r"C:\Users\Test\AppData\Local\TempDir");
    assert!(CanonicalPath::parse_lexical("C:/Users/Test/../etc").is_err());
}

#[test]
fn rejects_device_and_verbatim_prefixes() {
    for s in [
        r"\\?\C:\Windows",
        r"\\.\C:\Windows",
        r"\??\C:\Windows",
        r"\\?\GLOBALROOT\Device\HarddiskVolume1\Windows",
        r"\\?\UNC\server\share\folder",
    ] {
        let r = CanonicalPath::parse_lexical(s);
        assert!(r.is_err(), "should reject {s}: {r:?}");
    }
}

#[test]
fn rejects_unc_paths() {
    for s in [r"\\server\share", r"\\server\share\folder"] {
        let r = CanonicalPath::parse_lexical(s);
        assert!(matches!(r, Err(PathError::Unc)), "{s}: {r:?}");
    }
}

#[test]
fn rejects_drive_relative() {
    for s in [r"C:foo", r"c:Windows\Temp"] {
        assert!(CanonicalPath::parse_lexical(s).is_err(), "{s}");
    }
}

#[test]
fn rejects_trailing_spaces_and_dots_in_segments() {
    for s in [
        r"C:\Temp\folder ",
        r"C:\Temp\folder.",
        r"C:\Temp \x",
        r"C:\Temp.",
        r"C:\a\b \c",
        r"C:\a\b.\c",
    ] {
        let s = s.replace("\\x", "");
        if s.is_empty() {
            continue;
        }
        assert!(CanonicalPath::parse_lexical(&s).is_err(), "reject: {s}");
    }
}

#[test]
fn rejects_ads_and_reserved_device_names() {
    for s in [
        r"C:\Temp\file.txt:stream",
        r"C:\Temp\CON",
        r"C:\Temp\con.txt",
        r"C:\Temp\PRN",
        r"C:\Temp\AUX",
        r"C:\Temp\NUL",
        r"C:\Temp\COM1",
        r"C:\Temp\COM9",
        r"C:\Temp\LPT1",
        r"C:\Temp\LPT9",
        r"C:\Temp\NUL\foo",
    ] {
        assert!(CanonicalPath::parse_lexical(s).is_err(), "reject {s}");
    }
}

#[test]
fn rejects_empty_root_nul_and_bad_unicode() {
    assert!(CanonicalPath::parse_lexical("").is_err());
    assert!(CanonicalPath::parse_lexical("C:\\\0foo").is_err());
    // bidi/control/fullwidth
    assert!(CanonicalPath::parse_lexical("C:\\Temp\u{202E}\\foo").is_err());
    assert!(CanonicalPath::parse_lexical("C:\\Temp\u{200E}\\foo").is_err());
    assert!(CanonicalPath::parse_lexical("C:\\Temp\u{0001}\\foo").is_err());
    assert!(CanonicalPath::parse_lexical("C：\\Temp\\foo").is_err()); // fullwidth colon
    assert!(CanonicalPath::parse_lexical("C:\\Temp／foo").is_err()); // fullwidth slash
}

#[test]
fn rejects_long_paths_over_limit() {
    let long = format!(r"C:\{}\a\b\c", "a".repeat(5000));
    assert!(CanonicalPath::parse_lexical(&long).is_err());
}

#[test]
fn case_insensitive_segment_aware_prefix_no_collision() {
    let win = CanonicalPath::parse_lexical(r"c:\windows").unwrap();
    let old = CanonicalPath::parse_lexical(r"C:\Windows.old\foo\bar\baz").unwrap();
    let apps = CanonicalPath::parse_lexical(r"C:\WindowsApps\foo\bar\baz").unwrap();
    let child = CanonicalPath::parse_lexical(r"C:\WINDOWS\Temp").unwrap();
    assert!(!win.is_self_or_ancestor_of(&old));
    assert!(!win.is_self_or_ancestor_of(&apps));
    assert!(win.is_self_or_ancestor_of(&child));
}

#[test]
fn protected_container_and_subtree_rules() {
    use sweeper_core::paths::parse_drive_root;
    for s in [r"C:\", r"C:\Windows", r"C:\Users", r"C:\ProgramData"] {
        let p = if s == r"C:\" {
            parse_drive_root(s).unwrap()
        } else {
            CanonicalPath::parse_lexical(s).unwrap()
        };
        assert!(policy::is_protected_for_selection(&p), "{s}");
    }
    let p = CanonicalPath::parse_lexical(r"C:\Windows\System32").unwrap();
    assert!(policy::is_subtree_protected(&p));
    // Arbitrary descendants under protected container roots (e.g. C:\Windows, C:\Users, ProgramData, User Data) are blocked.
    let under_win = CanonicalPath::parse_lexical(r"C:\Windows\SomeRandomDir\Cache").unwrap();
    assert!(policy::is_protected_for_selection(&under_win));
    let under_users = CanonicalPath::parse_lexical(r"C:\Users\Alice\AppData\Local\TempX").unwrap();
    assert!(policy::is_protected_for_selection(&under_users));
    let under_profile = CanonicalPath::parse_lexical(r"C:\Users\Alice\Random\Deep\Path").unwrap();
    assert!(policy::is_protected_for_selection(&under_profile));
    let safeish = CanonicalPath::parse_lexical(r"C:\Windows.old\Temp\Cache").unwrap();
    assert!(!policy::is_protected_for_selection(&safeish));
    let safeish2 = CanonicalPath::parse_lexical(r"C:\Tools\Cache\Temp").unwrap();
    assert!(!policy::is_protected_for_selection(&safeish2));
}

#[test]
fn custom_path_lexical_depth_and_protection() {
    assert!(
        !policy::validate_custom_path_lexical(r"C:\a\b").allowed,
        "depth < 3"
    );
    assert!(!policy::validate_custom_path_lexical(r"C:\Windows\System32\foo").allowed);
    assert!(!policy::validate_custom_path_lexical(r"C:\Users\Alice").allowed);
    assert!(policy::validate_custom_path_lexical(r"C:\SomeApp\Cache\Temp").allowed);
}

#[test]
fn fuzz_entrypoint_never_panics_on_adversarial_bytes() {
    let samples: &[&[u8]] = &[
        b"",
        b"\0",
        b"C:\\..\\..\\Windows\\System32",
        b"\\\\?\\C:\\Windows",
        b"\\\\server\\share",
        b"C:foo",
        b"C:\\CON\\x",
        b"C:\\a\\b.\x00c",
        "C:\\Users\\Alice\\\u{202E}evil".as_bytes(),
        &[0xff, 0xfe, 0xfd, 0x00, 0x41],
    ];
    for s in samples {
        sweeper_core::fuzz_path_policy_bytes(s);
    }
}
