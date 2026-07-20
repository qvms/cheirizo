//! Build revision metadata used by diagnostics and packaging.
//!
//! Release builds set `WRDP_BUILD_REVISION`; local builds use the checked-out
//! Git revision. Source archives without Git metadata fall back to `unknown`.

use std::process::Command;

fn command_stdout(program: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(program).args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    (!value.is_empty()).then_some(value)
}

fn revision() -> String {
    std::env::var("WRDP_BUILD_REVISION")
        .ok()
        .filter(|value| !value.is_empty())
        .or_else(|| command_stdout("git", &["rev-parse", "--short", "HEAD"]))
        .unwrap_or_else(|| "unknown".to_owned())
}

fn main() {
    println!("cargo:rustc-env=GIT_HASH={}", revision());
    println!("cargo:rerun-if-env-changed=WRDP_BUILD_REVISION");
    println!("cargo:rerun-if-changed=.git/HEAD");
    if let Ok(head) = std::fs::read_to_string(".git/HEAD")
        && let Some(reference) = head.trim().strip_prefix("ref: ")
    {
        println!("cargo:rerun-if-changed=.git/{reference}");
    }
}
