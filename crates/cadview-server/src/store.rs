//! Document store abstraction.
//!
//! Flat S3-like key space. Keys are paths (e.g. "sub/floor-plan.dwg").
//! Path prefixes act as virtual folders for UI grouping.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// Flat document store. Keys are slash-separated paths.
pub trait DocumentStore: Send + Sync {
    /// List all document keys.
    fn list(&self) -> Result<Vec<String>>;
    /// Load raw file bytes by key.
    fn load(&self, key: &str) -> Result<Vec<u8>>;
    /// Check if a key exists.
    fn exists(&self, key: &str) -> bool;
    /// Resolve a key to an absolute filesystem path (for overlay DWG saves).
    fn resolve_path(&self, key: &str) -> Option<String> {
        let _ = key;
        None
    }
}

// ── SingleFileStore ─────────────────────────────────────────────────

/// Store backed by a single DWG file. One key only.
pub struct SingleFileStore {
    path: PathBuf,
    key: String,
}

impl SingleFileStore {
    pub fn new(path: PathBuf) -> Self {
        let key = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        Self { path, key }
    }

    pub fn key(&self) -> &str {
        &self.key
    }
}

impl DocumentStore for SingleFileStore {
    fn list(&self) -> Result<Vec<String>> {
        Ok(vec![self.key.clone()])
    }

    fn load(&self, key: &str) -> Result<Vec<u8>> {
        if key != self.key {
            anyhow::bail!("key '{}' not found (only '{}' available)", key, self.key);
        }
        std::fs::read(&self.path).with_context(|| format!("reading {}", self.path.display()))
    }

    fn exists(&self, key: &str) -> bool {
        key == self.key
    }

    fn resolve_path(&self, key: &str) -> Option<String> {
        if key == self.key {
            self.path
                .canonicalize()
                .ok()
                .map(|p| p.to_string_lossy().to_string())
        } else {
            None
        }
    }
}

// ── FolderStore ─────────────────────────────────────────────────────

/// Store backed by a filesystem directory (recursive).
/// Keys are relative paths with "/" separators.
pub struct FolderStore {
    root: PathBuf,
}

impl FolderStore {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }
}

impl DocumentStore for FolderStore {
    fn list(&self) -> Result<Vec<String>> {
        let mut keys = Vec::new();
        collect_dwg_files(&self.root, &self.root, &mut keys)?;
        keys.sort();
        Ok(keys)
    }

    fn load(&self, key: &str) -> Result<Vec<u8>> {
        let file_path = self
            .root
            .join(key.replace('/', std::path::MAIN_SEPARATOR_STR));
        if !file_path.starts_with(&self.root) {
            anyhow::bail!("path traversal rejected: '{}'", key);
        }
        std::fs::read(&file_path).with_context(|| format!("reading {}", file_path.display()))
    }

    fn exists(&self, key: &str) -> bool {
        let file_path = self
            .root
            .join(key.replace('/', std::path::MAIN_SEPARATOR_STR));
        if !file_path.starts_with(&self.root) || !file_path.is_file() {
            return false;
        }
        file_path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("dwg"))
    }

    fn resolve_path(&self, key: &str) -> Option<String> {
        let file_path = self
            .root
            .join(key.replace('/', std::path::MAIN_SEPARATOR_STR));
        if file_path.starts_with(&self.root) && file_path.is_file() {
            file_path
                .canonicalize()
                .ok()
                .map(|p| p.to_string_lossy().to_string())
        } else {
            None
        }
    }
}

fn collect_dwg_files(root: &Path, dir: &Path, out: &mut Vec<String>) -> Result<()> {
    let entries = std::fs::read_dir(dir).with_context(|| format!("listing {}", dir.display()))?;
    for entry in entries {
        let entry = entry?;
        let ft = entry.file_type()?;
        let path = entry.path();
        if ft.is_dir() {
            collect_dwg_files(root, &path, out)?;
        } else if ft.is_file() {
            if let Some(ext) = path.extension() {
                if ext.eq_ignore_ascii_case("dwg") {
                    let rel = path
                        .strip_prefix(root)
                        .unwrap_or(&path)
                        .to_string_lossy()
                        .replace('\\', "/");
                    out.push(rel);
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn single_file_store() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("test.dwg");
        fs::write(&file, b"fake dwg").unwrap();

        let store = SingleFileStore::new(file);
        assert_eq!(store.list().unwrap(), vec!["test.dwg"]);
        assert!(store.exists("test.dwg"));
        assert!(!store.exists("other.dwg"));
        assert_eq!(store.load("test.dwg").unwrap(), b"fake dwg");
        assert!(store.load("nope.dwg").is_err());
    }

    #[test]
    fn folder_store_recursive() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.dwg"), b"a").unwrap();
        fs::create_dir(dir.path().join("sub")).unwrap();
        fs::write(dir.path().join("sub/b.dwg"), b"b").unwrap();
        fs::write(dir.path().join("readme.txt"), b"ignore me").unwrap();

        let store = FolderStore::new(dir.path().to_path_buf());
        let mut keys = store.list().unwrap();
        keys.sort();
        assert_eq!(keys, vec!["a.dwg", "sub/b.dwg"]);
        assert!(store.exists("a.dwg"));
        assert!(store.exists("sub/b.dwg"));
        assert!(!store.exists("readme.txt"));
        assert_eq!(store.load("sub/b.dwg").unwrap(), b"b");
    }

    #[test]
    fn folder_store_path_traversal_blocked() {
        let dir = tempfile::tempdir().unwrap();
        let store = FolderStore::new(dir.path().to_path_buf());
        assert!(!store.exists("../../etc/passwd"));
    }
}
