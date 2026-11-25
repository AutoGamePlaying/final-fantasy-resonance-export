//! Build script for `ffr-db`: records the current git commit SHA (when available) into
//! `FFR_DB_GIT_COMMIT` for inclusion in build manifests.

use std::fs;
use std::path::Path;
use std::process::Command;

fn main() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default();
    let repo_root = Path::new(&manifest_dir).join("../..");
    let git_dir = repo_root.join(".git");

    if git_dir.is_dir() {
        let head_path = git_dir.join("HEAD");
        if head_path.is_file() {
            println!("cargo:rerun-if-changed={}", head_path.display());
            if let Ok(head_text) = fs::read_to_string(&head_path)
                && let Some(ref_rel) = head_text.trim().strip_prefix("ref:")
            {
                let ref_path = git_dir.join(ref_rel.trim());
                if ref_path.is_file() {
                    println!("cargo:rerun-if-changed={}", ref_path.display());
                }
            }
        }
        let packed_refs = git_dir.join("packed-refs");
        if packed_refs.is_file() {
            println!("cargo:rerun-if-changed={}", packed_refs.display());
        }
    }

    if let Ok(output) = Command::new("git")
        .args(["rev-parse", "--verify", "HEAD"])
        .current_dir(&repo_root)
        .output()
        && output.status.success()
        && let Ok(raw) = String::from_utf8(output.stdout)
    {
        let commit = raw.trim();
        if !commit.is_empty() && commit.bytes().all(|b| b.is_ascii_hexdigit()) {
            println!("cargo:rustc-env=FFR_DB_GIT_COMMIT={commit}");
        }
    }
}
