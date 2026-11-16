#![allow(dead_code)]

use std::fs;
use std::path::Path;

pub fn read_tauri_json(file: &str) -> serde_json::Value {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let path = match file {
        "cap" => root.join("capabilities").join("default.json"),
        _ => root.join(file),
    };
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

pub fn assert_tauri_csp_and_security(conf: &serde_json::Value) {
    let csp = conf["app"]["security"]["csp"].as_str().unwrap();
    assert!(csp.contains("default-src 'self'"), "csp: {csp}");
    assert!(!csp.contains("unsafe-eval"), "csp: {csp}");
    assert!(!csp.contains("unsafe-inline"), "csp: {csp}");
}

pub fn assert_tauri_no_fs_shell(conf: &serde_json::Value, cap: &serde_json::Value) {
    let conf_s = conf.to_string().to_lowercase();
    let cap_s = cap.to_string().to_lowercase();
    for (hay, needle, label) in [
        (&conf_s, "plugin-fs", "config plugins"),
        (&conf_s, "plugin-shell", "config plugins"),
        (&cap_s, "fs:", "capability plugins"),
        (&cap_s, "shell:", "capability plugins"),
    ] {
        assert!(!hay.contains(needle), "{label}");
    }
}

pub fn assert_tauri_bundle_snapshot(conf: &serde_json::Value) {
    assert_eq!(conf["bundle"]["active"], serde_json::json!(true));
    let targets = conf["bundle"]["targets"].as_array().unwrap();
    let has_nsis = targets.iter().any(|t| t.as_str() == Some("nsis"));
    assert!(has_nsis, "{targets:?}");
    let icons = conf["bundle"]["icon"].as_array().unwrap();
    assert!(!icons.is_empty(), "bundle icons must not be empty");
}
