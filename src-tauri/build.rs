use std::path::Path;
use std::process::Command;

fn main() {
    emit_git_sha();
    tauri_build::build()
}

/// Capture the short commit SHA at build time and expose it as `CIMON_GIT_SHA` so the panel footer
/// can show which commit the running binary was built from. Falls back to an empty string when git
/// is unavailable (e.g. a source-tarball build), which `app_info` treats as "no commit".
fn emit_git_sha() {
    let sha = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
        .unwrap_or_default();
    println!("cargo:rustc-env=CIMON_GIT_SHA={sha}");

    // Rebuild when HEAD moves so the embedded SHA stays in sync with the working tree on
    // incremental builds. Watch HEAD itself, the branch ref it points at, and packed-refs (a
    // commit may only update the packed file rather than a loose ref).
    let git_dir = Path::new("../.git");
    let head = git_dir.join("HEAD");
    if head.exists() {
        println!("cargo:rerun-if-changed=../.git/HEAD");
        if let Ok(contents) = std::fs::read_to_string(&head) {
            if let Some(reference) = contents.strip_prefix("ref:").map(str::trim) {
                println!("cargo:rerun-if-changed=../.git/{reference}");
            }
        }
        if git_dir.join("packed-refs").exists() {
            println!("cargo:rerun-if-changed=../.git/packed-refs");
        }
    }
}
