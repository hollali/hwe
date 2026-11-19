use std::fmt;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VPath(PathBuf);

impl VPath {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self(path.into())
    }

    pub fn as_path(&self) -> &Path {
        &self.0
    }

    pub fn into_path_buf(self) -> PathBuf {
        self.0
    }

    pub fn join(&self, name: impl AsRef<Path>) -> Self {
        Self(self.0.join(name))
    }

    pub fn parent(&self) -> Option<Self> {
        self.0.parent().map(|parent| Self(parent.to_path_buf()))
    }

    pub fn file_name(&self) -> Option<&std::ffi::OsStr> {
        self.0.file_name()
    }

    pub fn display(&self) -> std::path::Display<'_> {
        self.0.display()
    }
}

impl Default for VPath {
    fn default() -> Self {
        Self(PathBuf::new())
    }
}

impl fmt::Display for VPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0.display(), f)
    }
}

impl AsRef<Path> for VPath {
    fn as_ref(&self) -> &Path {
        self.as_path()
    }
}

impl From<PathBuf> for VPath {
    fn from(path: PathBuf) -> Self {
        Self(path)
    }
}

impl From<&Path> for VPath {
    fn from(path: &Path) -> Self {
        Self(path.to_path_buf())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parent_and_name_round_trip() {
        let path = VPath::new("/var/log/syslog");
        let parent = path.parent().expect("parent");
        assert_eq!(parent.as_path(), Path::new("/var/log"));
        assert_eq!(path.file_name().unwrap(), "syslog");
    }

    #[test]
    fn root_has_no_parent() {
        assert_eq!(VPath::new("/").parent(), None);
    }

    #[test]
    fn join_appends_segment() {
        let path = VPath::new("/tmp").join("a.txt");
        assert_eq!(path.as_path(), Path::new("/tmp/a.txt"));
    }
}
