//! Finds the source files that debug-info locations refer to.

use std::{
    collections::BTreeMap,
    fs,
    path::{Component, Path, PathBuf},
    sync::Arc,
};

use miden_debug_types::Uri;

/// Resolves file paths stored in debug info to source text on disk.
///
/// Stored paths are recorded at build time and may be absolute paths from
/// another machine, relative paths, or `file://` URIs. Lookups are cached, and
/// paths that cannot be resolved are remembered for reporting rather than
/// treated as errors.
#[derive(Debug, Default)]
pub struct SourceResolver {
    source_root: Option<PathBuf>,
    prefix_maps: Vec<(String, String)>,
    cache: BTreeMap<String, Option<Arc<SourceText>>>,
}

impl SourceResolver {
    /// Creates a resolver that, besides the current directory, looks under
    /// `source_root` and rewrites stored paths with `prefix_maps`, given as
    /// `(old, new)` prefix pairs.
    pub fn new(source_root: Option<PathBuf>, prefix_maps: Vec<(String, String)>) -> Self {
        Self {
            source_root,
            prefix_maps,
            cache: BTreeMap::new(),
        }
    }

    /// Returns the source text a stored debug-info path refers to, or `None`
    /// if no file is found.
    ///
    /// Candidates, in order: the path after applying the first matching prefix
    /// map (and stripping a `file://` scheme) as given; then, with a source
    /// root, the path joined to the root, and for an absolute path each of its
    /// suffixes joined to the root, longest first.
    pub fn resolve(&mut self, stored_path: &str) -> Option<Arc<SourceText>> {
        if let Some(cached) = self.cache.get(stored_path) {
            return cached.clone();
        }
        let resolved = self.find(stored_path).and_then(|path| {
            let text = fs::read_to_string(&path).ok()?;
            Some(Arc::new(SourceText::new(path, text)))
        });
        self.cache.insert(stored_path.to_owned(), resolved.clone());
        resolved
    }

    /// Stored paths that could not be resolved so far, in sorted order.
    pub fn unresolved(&self) -> impl Iterator<Item = &str> {
        self.cache
            .iter()
            .filter(|(_, resolved)| resolved.is_none())
            .map(|(stored, _)| stored.as_str())
    }

    /// Returns the first existing file for `stored_path`.
    fn find(&self, stored_path: &str) -> Option<PathBuf> {
        let mapped = self
            .prefix_maps
            .iter()
            .find_map(|(old, new)| {
                stored_path.strip_prefix(old.as_str()).map(|rest| format!("{new}{rest}"))
            })
            .unwrap_or_else(|| stored_path.to_owned());
        // `None` means a URI with a scheme other than `file`, which has no
        // file on disk.
        let path = Uri::new(&mapped).to_path()?;
        if path.is_file() {
            return Some(path);
        }

        let root = self.source_root.as_deref()?;
        if path.is_relative() {
            return Some(root.join(&path)).filter(|candidate| candidate.is_file());
        }
        // An absolute path is usually from the build machine. Its longest
        // suffix that exists under the root is taken to be the same file, which
        // lets a checkout stand in for the build tree wherever it lives.
        let components: Vec<Component<'_>> = path
            .components()
            .filter(|c| !matches!(c, Component::Prefix(_) | Component::RootDir))
            .collect();
        (0..components.len())
            .map(|start| root.join(components[start..].iter().collect::<PathBuf>()))
            .find(|candidate| candidate.is_file())
    }
}

/// The text of a resolved source file with a line index.
#[derive(Debug, Clone)]
pub struct SourceText {
    /// Where the file was found, as constructed during resolution.
    pub path: PathBuf,
    /// The file contents.
    pub text: String,
    /// Byte offset of the start of each line.
    line_starts: Vec<u32>,
}

impl SourceText {
    /// Indexes `text`, found at `path`.
    pub fn new(path: PathBuf, text: String) -> Self {
        let line_starts = std::iter::once(0)
            .chain(text.match_indices('\n').map(|(i, _)| i as u32 + 1))
            .collect();
        Self { path, text, line_starts }
    }

    /// The 1-based line containing byte offset `byte`. Offsets past the end
    /// fall on the last line.
    pub fn line_of(&self, byte: u32) -> u32 {
        // `line_starts[0]` is 0, so at least one start is `<= byte`.
        self.line_starts.partition_point(|&start| start <= byte) as u32
    }

    /// Number of lines; a trailing newline does not start a new line.
    pub fn line_count(&self) -> u32 {
        let starts = self.line_starts.len() as u32;
        if self.text.is_empty() || self.text.ends_with('\n') {
            starts - 1
        } else {
            starts
        }
    }
}

/// Returns `path` made absolute, or unchanged if it cannot be canonicalized.
pub(crate) fn absolute_path(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Returns `path` relative to the current directory when it lies below it.
pub(crate) fn display_relative(path: &Path) -> String {
    let relative = std::env::current_dir().ok().and_then(|cwd| {
        absolute_path(path)
            .strip_prefix(absolute_path(&cwd))
            .ok()
            .map(Path::to_path_buf)
    });
    relative.unwrap_or_else(|| path.to_path_buf()).display().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_of_maps_bytes_to_lines() {
        let text = SourceText::new("x".into(), "ab\ncd\n\nef".into());
        assert_eq!(text.line_of(0), 1);
        assert_eq!(text.line_of(2), 1, "the newline belongs to its line");
        assert_eq!(text.line_of(3), 2);
        assert_eq!(text.line_of(6), 3);
        assert_eq!(text.line_of(7), 4);
        assert_eq!(text.line_of(100), 4);
        assert_eq!(text.line_count(), 4);
        assert_eq!(SourceText::new("x".into(), "a\n".into()).line_count(), 1);
        assert_eq!(SourceText::new("x".into(), String::new()).line_count(), 0);
    }

    #[test]
    fn prefix_map_rewrites_stored_paths() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("src/lib.masm");
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(&file, "begin\nend\n").unwrap();

        let new = tmp.path().display().to_string();
        let mut resolver = SourceResolver::new(
            None,
            vec![
                ("/nowhere".into(), "/still/nowhere".into()),
                ("/build/machine".into(), new.clone()),
                ("/build".into(), "/wrong".into()),
            ],
        );
        let found = resolver.resolve("/build/machine/src/lib.masm").unwrap();
        assert_eq!(found.path, file);
        assert_eq!(found.text, "begin\nend\n");

        // The scheme of a `file://` URI is stripped after mapping.
        let found = resolver.resolve(&format!("file://{new}/src/lib.masm")).unwrap();
        assert_eq!(found.path, file);

        assert!(resolver.resolve("/build/other/src/lib.masm").is_none());
        assert_eq!(resolver.unresolved().collect::<Vec<_>>(), ["/build/other/src/lib.masm"]);
    }

    #[test]
    fn source_root_finds_the_longest_existing_suffix() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("checkout");
        let deep = root.join("crates/a/src/lib.masm");
        let shallow = root.join("src/lib.masm");
        for file in [&deep, &shallow] {
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(file, file.display().to_string()).unwrap();
        }
        let relative = root.join("rel/x.masm");
        fs::create_dir_all(relative.parent().unwrap()).unwrap();
        fs::write(&relative, "x").unwrap();

        let mut resolver = SourceResolver::new(Some(root), vec![]);
        assert_eq!(resolver.resolve("/home/ci/work/crates/a/src/lib.masm").unwrap().path, deep);
        assert_eq!(resolver.resolve("/home/ci/work/x/src/lib.masm").unwrap().path, shallow);
        assert_eq!(resolver.resolve("rel/x.masm").unwrap().path, relative);
        // Relative paths are not walked by suffix.
        assert!(resolver.resolve("other/rel/x.masm").is_none());
        assert!(resolver.resolve("/home/ci/work/missing.masm").is_none());
        assert_eq!(
            resolver.unresolved().collect::<Vec<_>>(),
            ["/home/ci/work/missing.masm", "other/rel/x.masm"]
        );
    }
}
