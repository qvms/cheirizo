//! Cleanup helpers for sesman-managed runtime artifacts.
//!
//! These helpers remove stale socket/lock/log paths from earlier per-user
//! wrdp sessions so fresh session startup does not inherit broken runtime state.

use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};

/// Remove configured runtime artifacts.
///
/// Explicit cleanup paths are required lifecycle operations: leaving one behind
/// can make the next compositor bind or readiness check use stale state. Glob
/// expansion and removal errors are likewise returned instead of allowing
/// startup/teardown to continue with a partially cleaned runtime.
pub(super) fn cleanup_runtime_paths(
    cleanup_paths: &[PathBuf],
    cleanup_globs: &[String],
) -> Result<()> {
    for path in cleanup_paths {
        remove_stale_path(path)?;
    }
    for pattern in cleanup_globs {
        cleanup_simple_glob(pattern)?;
    }
    Ok(())
}

fn remove_stale_path(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e)
            .with_context(|| format!("failed to remove stale runtime path {}", path.display())),
    }
}

fn cleanup_simple_glob(pattern: &str) -> Result<()> {
    let Some(star) = pattern.find('*') else {
        return remove_stale_path(Path::new(pattern));
    };
    let (prefix, suffix_with_star) = pattern.split_at(star);
    let suffix = &suffix_with_star[1..];
    let prefix_path = Path::new(prefix);
    let parent = prefix_path.parent().unwrap_or_else(|| Path::new("."));
    let Some(name_prefix) = prefix_path.file_name().and_then(|name| name.to_str()) else {
        return Ok(());
    };
    let entries = match fs::read_dir(parent) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => {
            return Err(e)
                .with_context(|| format!("failed to scan cleanup path {}", parent.display()));
        }
    };
    for entry in entries {
        let entry = entry
            .with_context(|| format!("failed to read cleanup entry in {}", parent.display()))?;
        let file_name = entry.file_name();
        let Some(file_name) = file_name.to_str() else {
            continue;
        };
        if file_name.starts_with(name_prefix) && file_name.ends_with(suffix) {
            remove_stale_path(&entry.path())?;
        }
    }
    Ok(())
}
