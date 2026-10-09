use std::{path::Path, process::Command};

#[test]
fn cargo_dev_command_defaults_to_desktop_entry_point() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let output = Command::new(env!("CARGO"))
        .args([
            "metadata",
            "--no-deps",
            "--offline",
            "--format-version",
            "1",
        ])
        .current_dir(root)
        .output()
        .expect("read Cargo's executable selection metadata");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let metadata: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let package = metadata["packages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["manifest_path"].as_str() == root.join("Cargo.toml").to_str())
        .expect("desktop package");
    let desktop = package["targets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["src_path"].as_str() == root.join("src/main.rs").to_str())
        .expect("desktop entry point");
    assert!(desktop["kind"]
        .as_array()
        .unwrap()
        .iter()
        .any(|k| k == "bin"));
    assert_eq!(package["default_run"], desktop["name"],
        "Tauri's cargo run must select the desktop app even with connector binaries in the workspace");
}
