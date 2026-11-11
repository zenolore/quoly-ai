fn main() {
    let attributes = tauri_build::Attributes::new().windows_attributes(
        tauri_build::WindowsAttributes::new_without_app_manifest(),
    );
    tauri_build::try_build(attributes).expect("failed to run tauri-build");
    embed_windows_test_manifest();
}

// tauri-build links its Common-Controls v6 manifest via a .res that only
// reaches lib and bin builds, so `cargo test` executables start with
// Common-Controls v5 and die with STATUS_ENTRYPOINT_NOT_FOUND
// (tauri-apps/tauri#11028). Providing the manifest once for every target
// via /MANIFEST:EMBED replaces it — the .res and /MANIFEST:EMBED cannot
// coexist (CVT1100 duplicate resource), so the winresource one is disabled
// above. Same workaround as tauri's own workspace build script.
fn embed_windows_test_manifest() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let manifest = std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap())
        .join("windows-app-manifest.xml");
    println!("cargo:rerun-if-changed={}", manifest.display());
    match std::env::var("CARGO_CFG_TARGET_ENV").as_deref() {
        Ok("msvc") => {
            println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
            println!("cargo:rustc-link-arg=/MANIFESTINPUT:{}", manifest.display());
        }
        // GNU toolchain: windres compiles the manifest into a COFF object
        // (resource ID 1 / RT_MANIFEST) that is linked into the exe. Without
        // it the process starts with Common-Controls v5 and dies with
        // STATUS_ENTRYPOINT_NOT_FOUND, same as the MSVC test binaries above.
        _ => {
            let out = std::env::var("OUT_DIR").unwrap();
            let obj = std::path::Path::new(&out).join("app_manifest.res.o");
            let rc = std::path::Path::new(&out).join("app_manifest.rc");
            std::fs::write(
                &rc,
                format!(
                    "1 24 \"{}\"",
                    manifest.display().to_string().replace('\\', "/")
                ),
            )
            .expect("write manifest rc");
            let windres = std::path::PathBuf::from(
                std::env::var("WINDRES").unwrap_or_else(|_| "windres".into()),
            );
            let status = std::process::Command::new(&windres)
                .args(["--input", rc.to_str().unwrap(), "--output", obj.to_str().unwrap()])
                .status()
                .unwrap_or_else(|e| panic!("run windres at {windres:?}: {e}"));
            if !status.success() {
                panic!("windres failed to compile {}", manifest.display());
            }
            println!("cargo:rerun-if-changed={}", obj.display());
            println!("cargo:rustc-link-arg={}", obj.display());
        }
    }
}
