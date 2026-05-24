//! Build script.
//!
//! Captures the current git branch and short commit hash at build time and
//! exposes them to the binary through `cargo:rustc-env`. The running app
//! prints these in its startup banner so a given build can always be traced
//! back to the exact source it came from.

use std::process::Command;

fn main() {
    // Re-run when HEAD moves so the embedded hash stays accurate.
    println!("cargo:rerun-if-changed=.git/HEAD");

    let branch = git(&["rev-parse", "--abbrev-ref", "HEAD"]).unwrap_or_else(|| "unknown".into());
    let commit = git(&["rev-parse", "--short", "HEAD"]).unwrap_or_else(|| "uncommitted".into());

    // A trailing "+" marks a working tree with uncommitted changes.
    let dirty = match git(&["status", "--porcelain"]) {
        Some(s) if !s.is_empty() => "+",
        _ => "",
    };

    println!("cargo:rustc-env=AV_GIT_BRANCH={branch}");
    println!("cargo:rustc-env=AV_GIT_COMMIT={commit}{dirty}");
}

/// Run a git command and return its trimmed stdout, or `None` if git is
/// unavailable or the command fails (e.g. before the first commit).
fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8(out.stdout).ok()?.trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}
