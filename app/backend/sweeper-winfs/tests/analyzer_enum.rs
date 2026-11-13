#![cfg(windows)]

use std::fs;

use sweeper_testkit::{create_junction, to_extended_length_path, CanaryTree, Sandbox};
use sweeper_winfs::{
    classify_reparse_tag, is_cloud_placeholder_attributes, read_dir_entries,
    read_dir_entries_with_options, ReparseTagClass, FILE_ATTRIBUTE_OFFLINE_FLAG,
    FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS_FLAG, FILE_ATTRIBUTE_RECALL_ON_OPEN_FLAG,
};

const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;

fn find_entry<'a>(
    entries: &'a [sweeper_winfs::DirEntry],
    name: &str,
) -> Option<&'a sweeper_winfs::DirEntry> {
    entries
        .iter()
        .find(|e| e.name.to_string_lossy().eq_ignore_ascii_case(name))
}

#[test]
fn find_first_file_ex_enumeration_accuracy_preserves_attrs_and_sizes() {
    let sandbox = Sandbox::new().expect("sandbox");
    sandbox
        .write_file("alpha.txt", b"hello")
        .expect("write alpha");
    sandbox.create_dir("nested").expect("create nested dir");
    sandbox
        .write_file("nested/beta.bin", [1u8, 2, 3, 4, 5, 6])
        .expect("write beta");

    let entries = read_dir_entries(sandbox.root()).expect("enumerate sandbox root");
    assert!(entries
        .iter()
        .any(|e| e.name.to_string_lossy() == ".sweeper-sandbox-sentinel"));

    let alpha = find_entry(&entries, "alpha.txt").expect("alpha entry");
    assert!(!alpha.is_dir, "alpha should be file");
    assert_eq!(alpha.size, 5, "alpha size should match content length");
    assert_eq!(alpha.reparse, ReparseTagClass::NotReparse);
    assert_eq!(alpha.reparse_tag, 0);
    assert_eq!(
        alpha.attributes & FILE_ATTRIBUTE_REPARSE_POINT,
        0,
        "regular file must not have reparse attribute"
    );

    let nested = find_entry(&entries, "nested").expect("nested dir entry");
    assert!(nested.is_dir, "nested should be directory");
    assert_eq!(nested.reparse, ReparseTagClass::NotReparse);

    // Optional non-large-fetch path must also enumerate accurately.
    let entries_no_large = read_dir_entries_with_options(sandbox.root(), false)
        .expect("enumerate without large fetch");
    assert!(find_entry(&entries_no_large, "alpha.txt").is_some());
    assert!(find_entry(&entries_no_large, "nested").is_some());
}

#[test]
fn link_points_are_reported_but_not_traversed_and_canary_excluded() {
    let sandbox = Sandbox::new().expect("sandbox");
    let outside = CanaryTree::seed_external("winfs-analyzer-enum").expect("outside canary");

    let inside_dir = sandbox.create_dir("inside").expect("inside dir");
    sandbox
        .write_file("inside/local.txt", b"local")
        .expect("local file");
    let junction_path = sandbox.root().join("junction_to_outside");
    create_junction(&junction_path, outside.root()).expect("create junction");

    let root_entries = read_dir_entries(sandbox.root()).expect("enumerate root");
    let junction = find_entry(&root_entries, "junction_to_outside").expect("junction entry");
    assert_eq!(
        junction.reparse,
        ReparseTagClass::LinkPoint,
        "junction must be classified as Link Point from find data"
    );
    assert_ne!(
        junction.attributes & FILE_ATTRIBUTE_REPARSE_POINT,
        0,
        "junction must carry reparse attribute"
    );
    assert_ne!(
        junction.reparse_tag, 0,
        "junction reparse tag must be preserved"
    );

    let names: Vec<String> = root_entries
        .iter()
        .map(|e| e.name.to_string_lossy().to_string())
        .collect();
    assert!(
        !names.iter().any(|n| n.eq_ignore_ascii_case("root.txt")),
        "enumeration must not include files from junction target"
    );
    assert!(
        !names.iter().any(|n| n.eq_ignore_ascii_case("nested")),
        "enumeration must not include canary subdirs"
    );

    let inside_entries = read_dir_entries(&inside_dir).expect("enumerate inside");
    assert!(find_entry(&inside_entries, "local.txt").is_some());
    outside
        .verify()
        .expect("outside canary must remain unchanged");
}

#[test]
fn cloud_placeholder_attribute_helpers_and_reparse_classification() {
    assert!(is_cloud_placeholder_attributes(FILE_ATTRIBUTE_OFFLINE_FLAG));
    assert!(is_cloud_placeholder_attributes(
        FILE_ATTRIBUTE_RECALL_ON_OPEN_FLAG
    ));
    assert!(is_cloud_placeholder_attributes(
        FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS_FLAG
    ));
    assert!(!is_cloud_placeholder_attributes(0));

    assert_eq!(
        classify_reparse_tag(sweeper_winfs::IO_REPARSE_TAG_CLOUD),
        ReparseTagClass::CloudPlaceholder
    );
    assert_eq!(
        classify_reparse_tag(0x9000_301A),
        ReparseTagClass::CloudPlaceholder
    );
}

#[test]
fn long_paths_over_260_are_enumerated() {
    let sandbox = Sandbox::new().expect("sandbox");
    let deep_rel = format!(
        "deep\\{}\\{}\\{}\\file.txt",
        "a".repeat(80),
        "b".repeat(80),
        "c".repeat(80)
    );
    let deep_path = sandbox.guarded_path(&deep_rel);
    if let Some(parent) = deep_path.parent() {
        fs::create_dir_all(parent).expect("create deep dirs");
    }
    fs::write(&deep_path, b"long-path").expect("write deep file");
    assert!(
        deep_path.to_string_lossy().len() > 240,
        "test path should exercise long-path handling"
    );

    let parent = deep_path.parent().expect("parent");
    let entries = read_dir_entries(parent).expect("enumerate long path dir");
    let file = find_entry(&entries, "file.txt").expect("file in long path");
    assert_eq!(file.size, 9);

    // Extended-length path form should also enumerate.
    let ext_parent = to_extended_length_path(parent);
    let ext_entries = read_dir_entries(&ext_parent).expect("enumerate extended-length path");
    assert!(find_entry(&ext_entries, "file.txt").is_some());
}

#[test]
fn arabic_and_emoji_filenames_are_enumerated_correctly() {
    let sandbox = Sandbox::new().expect("sandbox");
    let arabic_dir = "مجلد عربي";
    let emoji_file = "rocket-🚀.txt";
    let mixed_file = "بيانات-😀.bin";

    sandbox.create_dir(arabic_dir).expect("create arabic dir");
    sandbox
        .write_file(format!("{arabic_dir}/داخل.txt"), "مرحبا".as_bytes())
        .expect("write arabic file");
    sandbox
        .write_file(emoji_file, b"emoji")
        .expect("write emoji file");
    sandbox
        .write_file(mixed_file, b"mixed")
        .expect("write mixed file");

    let root_entries = read_dir_entries(sandbox.root()).expect("enumerate root");
    assert!(
        find_entry(&root_entries, arabic_dir).is_some(),
        "arabic directory missing"
    );
    assert!(
        find_entry(&root_entries, emoji_file).is_some(),
        "emoji filename missing"
    );
    assert!(
        find_entry(&root_entries, mixed_file).is_some(),
        "mixed unicode filename missing"
    );

    let arabic_entries =
        read_dir_entries(&sandbox.root().join(arabic_dir)).expect("enumerate arabic dir");
    assert!(
        find_entry(&arabic_entries, "داخل.txt").is_some(),
        "arabic child filename missing"
    );
}
