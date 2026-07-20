//! Cleanup helpers for sesman-managed runtime artifacts.
//!
//! These helpers remove stale socket/lock/log paths from earlier per-user
//! wrdp sessions so fresh session startup does not inherit broken runtime state.

use std::{
    fs,
    path::{Path, PathBuf},
};

use tracing::warn;

pub(super) fn cleanup_runtime_paths(cleanup_paths: &[PathBuf], cleanup_globs: &[String]) {
    for path in cleanup_paths {
        remove_stale_path(path);
    }
    for pattern in cleanup_globs {
        cleanup_simple_glob(pattern);
    }
}

fn remove_stale_path(path: &Path) {
    if let Err(e) = fs::remove_file(path)
        && e.kind() != std::io::ErrorKind::NotFound
    {
        warn!(
            "failed to remove stale runtime path {}: {e}",
            path.display()
        );
    }
}

fn cleanup_simple_glob(pattern: &str) {
    let Some(star) = pattern.find('*') else {
        remove_stale_path(Path::new(pattern));
        return;
    };
    let (prefix, suffix_with_star) = pattern.split_at(star);
    let suffix = &suffix_with_star[1..];
    let prefix_path = Path::new(prefix);
    let parent = prefix_path.parent().unwrap_or_else(|| Path::new("."));
    let Some(name_prefix) = prefix_path.file_name().and_then(|name| name.to_str()) else {
        return;
    };
    let Ok(entries) = fs::read_dir(parent) else {
        return;
    };
    for entry in entries.flatten() {
        let file_name = entry.file_name();
        let Some(file_name) = file_name.to_str() else {
            continue;
        };
        if file_name.starts_with(name_prefix) && file_name.ends_with(suffix) {
            remove_stale_path(&entry.path());
        }
    }
}
