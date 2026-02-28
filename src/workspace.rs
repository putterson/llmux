use crate::error::{Error, Result};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Create a workspace directory with symlinks to the given source directories.
/// Returns the path to the workspace directory.
pub fn create_workspace(base_dir: &Path, name: &str, sources: &[String]) -> Result<PathBuf> {
    let workspace_dir = base_dir.join(name);
    std::fs::create_dir_all(&workspace_dir)?;

    // Track basename usage to handle collisions
    let mut basename_counts: HashMap<String, usize> = HashMap::new();

    for source in sources {
        let source_path = PathBuf::from(source);
        let source_path = source_path.canonicalize().map_err(|_| {
            Error::PathNotFound(source_path.clone())
        })?;

        let basename = source_path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| Error::Workspace(format!("invalid source path: {}", source)))?
            .to_string();

        let count = basename_counts.entry(basename.clone()).or_insert(0);
        *count += 1;

        let link_name = if *count > 1 {
            format!("{}-{}", basename, count)
        } else {
            basename
        };

        let link_path = workspace_dir.join(&link_name);
        std::os::unix::fs::symlink(&source_path, &link_path).map_err(|e| {
            Error::Workspace(format!(
                "failed to symlink {} -> {}: {}",
                link_path.display(),
                source_path.display(),
                e
            ))
        })?;
    }

    Ok(workspace_dir)
}

/// Remove a workspace directory
pub fn remove_workspace(workspace_dir: &Path) -> Result<()> {
    if workspace_dir.exists() {
        // Only remove symlinks and the directory itself, not the targets
        for entry in std::fs::read_dir(workspace_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_symlink() {
                std::fs::remove_file(&path)?;
            }
        }
        std::fs::remove_dir(workspace_dir)?;
    }
    Ok(())
}

/// Clean up all workspace directories that don't correspond to active sessions
pub fn clean_workspaces(base_dir: &Path, active_names: &[String], dry_run: bool) -> Result<Vec<PathBuf>> {
    let mut cleaned = Vec::new();
    if !base_dir.exists() {
        return Ok(cleaned);
    }

    for entry in std::fs::read_dir(base_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string();
            if !active_names.contains(&name) {
                if !dry_run {
                    remove_workspace(&path)?;
                }
                cleaned.push(path);
            }
        }
    }

    Ok(cleaned)
}
