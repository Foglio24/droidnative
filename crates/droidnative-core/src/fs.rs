//! Read-only access to the host filesystem, rooted so probes can run against fixture trees in tests.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct HostFs {
    root: PathBuf,
}

impl Default for HostFs {
    fn default() -> Self {
        Self::real()
    }
}

impl HostFs {
    pub fn real() -> Self {
        Self { root: PathBuf::from("/") }
    }

    /// Treat `root` as `/` (used by tests with a fake `/sys`, `/proc`, `/etc`).
    pub fn rooted(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn is_real(&self) -> bool {
        self.root == Path::new("/")
    }

    /// Map an absolute host path into this filesystem view.
    pub fn path(&self, host_path: impl AsRef<Path>) -> PathBuf {
        let p = host_path.as_ref();
        self.root.join(p.strip_prefix("/").unwrap_or(p))
    }

    pub fn exists(&self, host_path: impl AsRef<Path>) -> bool {
        self.path(host_path).exists()
    }

    pub fn read(&self, host_path: impl AsRef<Path>) -> Option<String> {
        std::fs::read_to_string(self.path(host_path)).ok()
    }

    pub fn read_trimmed(&self, host_path: impl AsRef<Path>) -> Option<String> {
        self.read(host_path).map(|s| s.trim().to_string())
    }

    pub fn read_bytes(&self, host_path: impl AsRef<Path>) -> Option<Vec<u8>> {
        std::fs::read(self.path(host_path)).ok()
    }

    /// Sorted entry names of a directory (empty when missing).
    pub fn list(&self, host_path: impl AsRef<Path>) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(self.path(host_path))
            .map(|rd| rd.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned()).collect())
            .unwrap_or_default();
        names.sort();
        names
    }

    /// Resolve a symlink and return the last path component of its target (e.g. a driver name).
    pub fn link_basename(&self, host_path: impl AsRef<Path>) -> Option<String> {
        std::fs::read_link(self.path(host_path))
            .ok()
            .and_then(|t| t.file_name().map(|n| n.to_string_lossy().into_owned()))
    }

    /// Read `KEY=VALUE` lines (uevent, os-release); values have surrounding quotes removed.
    pub fn key_values(&self, host_path: impl AsRef<Path>) -> Vec<(String, String)> {
        self.read(host_path).map(|s| parse_key_values(&s)).unwrap_or_default()
    }
}

pub fn parse_key_values(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                return None;
            }
            let (k, v) = line.split_once('=')?;
            let v = v.trim();
            let v = v.strip_prefix('"').and_then(|v| v.strip_suffix('"')).unwrap_or(v);
            let v = v.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')).unwrap_or(v);
            Some((k.trim().to_string(), v.to_string()))
        })
        .collect()
}

pub fn lookup<'a>(pairs: &'a [(String, String)], key: &str) -> Option<&'a str> {
    pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_quoted_values_and_skips_comments() {
        let pairs = parse_key_values("# c\nNAME=\"Ubuntu\"\nID=ubuntu\nX='y z'\n\n");
        assert_eq!(lookup(&pairs, "NAME"), Some("Ubuntu"));
        assert_eq!(lookup(&pairs, "ID"), Some("ubuntu"));
        assert_eq!(lookup(&pairs, "X"), Some("y z"));
        assert_eq!(pairs.len(), 3);
    }

    #[test]
    fn rooted_paths_map_absolute_paths() {
        let fs = HostFs::rooted("/tmp/fixture");
        assert_eq!(fs.path("/sys/class/drm"), PathBuf::from("/tmp/fixture/sys/class/drm"));
        assert!(!fs.is_real());
    }
}
