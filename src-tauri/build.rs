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
    // commit may only update the packed file rather than a loose ref). Resolve each path via
    // `git rev-parse --git-path`, which works in a LINKED WORKTREE too: there `.git` is a FILE
    // pointing into the main repo's `worktrees/` dir, so the old `../.git/HEAD` path join silently
    // missed and the SHA went stale. Only emit `rerun-if-changed` for paths that actually exist, so
    // a missing (e.g. packed) ref does not force a rebuild on every invocation.
    let head_path = git_path("HEAD");
    let mut watch: Vec<String> = Vec::new();
    if let Some(head) = &head_path {
        watch.push(head.clone());
        if let Ok(contents) = std::fs::read_to_string(head) {
            if let Some(reference) = contents.strip_prefix("ref:").map(str::trim) {
                watch.extend(git_path(reference));
            }
        }
    }
    watch.extend(git_path("packed-refs"));
    for path in watch {
        if Path::new(&path).exists() {
            println!("cargo:rerun-if-changed={path}");
        }
    }
}

/// Resolve a path inside the git directory via `git rev-parse --git-path <name>`. Unlike joining
/// onto `../.git`, this correctly handles a linked worktree (where `.git` is a file) and the shared
/// common dir for refs. Returns `None` when git is unavailable (e.g. a source-tarball build).
fn git_path(name: &str) -> Option<String> {
    let out = Command::new("git")
        .args(["rev-parse", "--git-path", name])
        .output()
        .ok()
        .filter(|o| o.status.success())?;
    let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!path.is_empty()).then_some(path)
}
