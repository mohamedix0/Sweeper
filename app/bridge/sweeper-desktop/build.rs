use std::path::{Path, PathBuf};

fn main() {
    println!("cargo::rustc-check-cfg=cfg(has_frontend_dist)");
    let manifest_dir =
        PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let frontend_dist = manifest_dir.join("../../frontend/dist");
    println!("cargo:rerun-if-changed=tauri.conf.json");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=capabilities");
    println!("cargo:rerun-if-changed=isolation");
    println!("cargo:rerun-if-changed={}", frontend_dist.display());

    if frontend_dist.exists() {
        println!("cargo:rustc-cfg=has_frontend_dist");
        tauri_build::build();
        return;
    }

    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    let stub_root = out_dir.join("stub-tauri");
    let stub_dist = stub_root.join("frontend-dist");
    let stub_capabilities = stub_root.join("capabilities");
    let stub_isolation = stub_root.join("isolation");
    std::fs::create_dir_all(&stub_dist).expect("create stub frontend dist");
    std::fs::create_dir_all(&stub_root).expect("create stub tauri root");
    copy_dir_all(&manifest_dir.join("capabilities"), &stub_capabilities)
        .expect("copy capabilities for stub tauri config");
    copy_dir_all(&manifest_dir.join("isolation"), &stub_isolation)
        .expect("copy isolation for stub tauri config");
    std::fs::write(
        stub_dist.join("index.html"),
        "<!doctype html><html><head><meta charset=\"utf-8\"></head><body></body></html>",
    )
    .expect("write stub index.html");

    let stub_config_path = stub_root.join("tauri.conf.json");
    let stub_dist_json = json_path(&stub_dist);
    let isolation_dir_json = json_path(&stub_isolation);
    let stub_config = format!(
        r#"{{
  "$schema": "https://schema.tauri.app/config/2",
  "productName": "Sweeper",
  "version": "0.1.0",
  "identifier": "com.sweeper.desktop",
  "build": {{
    "beforeDevCommand": "",
    "beforeBuildCommand": "",
    "frontendDist": "{stub_dist_json}"
  }},
  "app": {{
    "withGlobalTauri": false,
    "security": {{
      "csp": "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:; connect-src ipc: http://ipc.localhost; object-src 'none'; base-uri 'self'; frame-ancestors 'none'",
      "freezePrototype": true,
      "capabilities": ["default"],
      "pattern": {{
        "use": "isolation",
        "options": {{
          "dir": "{isolation_dir_json}"
        }}
      }}
    }},
    "windows": [
      {{
        "label": "main",
        "title": "Sweeper",
        "width": 1120,
        "height": 760,
        "minWidth": 880,
        "minHeight": 620,
        "center": true,
        "resizable": true
      }}
    ]
  }},
  "bundle": {{
    "active": false
  }}
}}"#
    );
    std::fs::write(&stub_config_path, stub_config).expect("write stub tauri config");

    let context_rs_path = out_dir.join("tauri_context.rs");
    let stub_config_rs = json_path(&stub_config_path);
    std::fs::write(
        &context_rs_path,
        format!("tauri::generate_context!(\"{stub_config_rs}\")\n"),
    )
    .expect("write stub tauri context include");

    // Point tauri-build at the stub config so clean checkouts without frontend/dist can compile.
    // `TAURI_CONFIG` is interpreted as a JSON config override string by Tauri.
    // SAFETY: build scripts are single-process for this crate; this env var is process-local.
    let stub_config_json =
        std::fs::read_to_string(&stub_config_path).expect("read stub tauri config");
    std::env::set_var("TAURI_CONFIG", stub_config_json);
    tauri_build::build();
}

fn copy_dir_all(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let to = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_all(&entry.path(), &to)?;
        } else {
            std::fs::copy(entry.path(), to)?;
        }
    }
    Ok(())
}

fn json_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}
