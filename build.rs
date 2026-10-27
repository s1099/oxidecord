//! Bakes the JSON theme files in `themes/` into the binary, and on Windows the
//! app icon into the exe's resources.

use std::{env, fs, path::PathBuf};

fn main() {
    println!("cargo::rerun-if-changed=themes");

    let themes_dir =
        PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR")).join("themes");

    let mut files: Vec<PathBuf> = fs::read_dir(&themes_dir)
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
                .collect()
        })
        .unwrap_or_default();
    // Keeps preset parse order stable.
    files.sort();

    let mut generated = String::from(
        "/// Every JSON file in `themes/`, as `(file name, contents)`, in name order.\n",
    );
    generated.push_str("pub static PRESET_THEME_FILES: &[(&str, &str)] = &[\n");
    for path in &files {
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        // Backslashes would be escape sequences in the generated string.
        let path = path.display().to_string().replace('\\', "/");
        generated.push_str(&format!("    (\"{name}\", include_str!(\"{path}\")),\n"));
    }
    generated.push_str("];\n");

    let out = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR")).join("preset_themes.rs");
    fs::write(&out, generated).expect("failed to write generated preset theme list");

    // `cfg!` would describe the host, not the target.
    if env::var("CARGO_CFG_TARGET_OS").is_ok_and(|os| os == "windows") {
        println!("cargo::rerun-if-changed=assets/icon.ico");
        embed_resource::compile("assets/icon.rc", embed_resource::NONE)
            .manifest_optional()
            .expect("failed to embed the app icon");
    }
}
