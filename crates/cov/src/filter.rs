//! Selects the source files a report covers.

use std::path::{Path, PathBuf};

use globset::{Glob, GlobSet, GlobSetBuilder};

use crate::{Error, source::absolute_path};

/// Include and exclude globs over source file paths, and the directory a
/// report is scoped to.
///
/// A file is kept when it matches at least one include glob, or there are
/// none and it lies in the scope, and matches no exclude glob. Each glob is
/// tried against the path as given and against the same path relative to a
/// base directory (absolute when the given path is relative), so both
/// `src/**` and absolute globs work. The default keeps every file.
#[derive(Debug, Clone, Default)]
pub struct ReportFilter {
    include: Option<GlobSet>,
    exclude: Option<GlobSet>,
    base: PathBuf,
    /// The canonical directory that files must lie under when there are no
    /// include globs; `None` keeps files wherever they are.
    scope: Option<PathBuf>,
}

impl ReportFilter {
    /// Creates a filter from `include` and `exclude` globs, with relative
    /// paths taken relative to the current directory.
    pub fn new<S: AsRef<str>>(include: &[S], exclude: &[S]) -> Result<Self, Error> {
        Ok(Self {
            include: glob_set(include)?,
            exclude: glob_set(exclude)?,
            // Without a current directory only the path as given is matched.
            base: std::env::current_dir().unwrap_or_default(),
            scope: None,
        })
    }

    /// Takes relative paths relative to `base` instead of the current
    /// directory.
    pub fn relative_to(mut self, base: impl Into<PathBuf>) -> Self {
        self.base = base.into();
        self
    }

    /// Without include globs, keeps only the files that lie under `root`,
    /// so that dependencies resolved elsewhere, such as a library from the
    /// cargo registry, drop out of the report. Include globs replace the
    /// scope and may name files anywhere.
    pub fn scoped_to(mut self, root: &Path) -> Self {
        self.scope = Some(absolute_path(root));
        self
    }

    /// Returns `true` when the source file found at `path` belongs in the
    /// report.
    pub fn keeps(&self, path: &Path) -> bool {
        let in_scope = || {
            self.include.is_some()
                || self.scope.as_ref().is_none_or(|scope| absolute_path(path).starts_with(scope))
        };
        self.matches_globs(path) && in_scope()
    }

    /// Returns `true` when a stored path that no file was found for belongs
    /// in the list of unresolved sources.
    ///
    /// The scope does not apply: where the file would be found is unknown.
    pub fn keeps_unresolved(&self, path: &Path) -> bool {
        self.matches_globs(path)
    }

    /// Returns `true` when `path` matches an include glob, or there are
    /// none, and no exclude glob.
    fn matches_globs(&self, path: &Path) -> bool {
        let other = if path.is_absolute() {
            path.strip_prefix(&self.base).ok().map(Path::to_path_buf)
        } else {
            Some(self.base.join(path))
        };
        let matches = |set: &GlobSet| {
            set.is_match(path) || other.as_deref().is_some_and(|other| set.is_match(other))
        };
        self.include.as_ref().is_none_or(matches) && !self.exclude.as_ref().is_some_and(matches)
    }
}

/// Compiles `globs`, or returns `None` when there are none.
fn glob_set<S: AsRef<str>>(globs: &[S]) -> Result<Option<GlobSet>, Error> {
    if globs.is_empty() {
        return Ok(None);
    }
    let mut builder = GlobSetBuilder::new();
    for glob in globs {
        let glob = glob.as_ref();
        builder.add(
            Glob::new(glob)
                .map_err(|source| Error::InvalidGlob { glob: glob.to_owned(), source })?,
        );
    }
    builder.build().map(Some).map_err(|source| Error::InvalidGlob {
        glob: globs.iter().map(AsRef::as_ref).collect::<Vec<_>>().join(", "),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filter(include: &[&str], exclude: &[&str]) -> ReportFilter {
        ReportFilter::new(include, exclude).unwrap().relative_to("/work")
    }

    #[test]
    fn default_keeps_everything() {
        assert!(ReportFilter::default().keeps(Path::new("/any/file.masm")));
        assert!(filter(&[], &[]).keeps(Path::new("rel/file.masm")));
    }

    #[test]
    fn include_only() {
        let filter = filter(&["src/**"], &[]);
        assert!(filter.keeps(Path::new("/work/src/a.masm")));
        assert!(filter.keeps(Path::new("src/nested/b.masm")));
        assert!(!filter.keeps(Path::new("/work/lib/c.masm")));
        assert!(!filter.keeps(Path::new("/elsewhere/src/a.masm")));
    }

    #[test]
    fn exclude_only() {
        let filter = filter(&[], &["**/tests/**"]);
        assert!(filter.keeps(Path::new("/work/src/a.masm")));
        assert!(!filter.keeps(Path::new("/work/tests/a.masm")));
        assert!(!filter.keeps(Path::new("tests/a.masm")));
    }

    #[test]
    fn exclude_wins_over_include() {
        let filter = filter(&["src/**"], &["src/gen/**"]);
        assert!(filter.keeps(Path::new("/work/src/a.masm")));
        assert!(!filter.keeps(Path::new("/work/src/gen/a.masm")));
        assert!(!filter.keeps(Path::new("/work/other.masm")));
    }

    #[test]
    fn absolute_globs_match_relative_paths() {
        let filter = filter(&["/work/src/*.masm"], &[]);
        assert!(filter.keeps(Path::new("src/a.masm")));
        assert!(filter.keeps(Path::new("/work/src/a.masm")));
        assert!(!filter.keeps(Path::new("lib/a.masm")));
    }

    #[test]
    fn scope_keeps_files_under_the_root_unless_included() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("checkout");
        let inside = root.join("src/a.masm");
        let outside = tmp.path().join("registry/lib/b.masm");
        for file in [&inside, &outside] {
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, "").unwrap();
        }

        let scoped = |include: &[&str], exclude: &[&str]| {
            ReportFilter::new(include, exclude).unwrap().relative_to(&root).scoped_to(&root)
        };
        let filter = scoped(&[], &[]);
        assert!(filter.keeps(&inside));
        // A path through `..` is canonicalized before the scope check.
        assert!(filter.keeps(&root.join("src/../src/a.masm")));
        assert!(!filter.keeps(&outside));
        // Unresolved paths are not scoped.
        assert!(filter.keeps_unresolved(&outside));
        assert!(!scoped(&[], &["src/**"]).keeps(&inside));

        // An include glob brings outside files back in.
        let registry = format!("{}/registry/**", tmp.path().display());
        let filter = scoped(&[&registry], &[]);
        assert!(filter.keeps(&outside));
        assert!(!filter.keeps(&inside));
    }

    #[test]
    fn invalid_glob_is_named() {
        let error = ReportFilter::new(&["src/[".to_owned()], &[]).unwrap_err();
        assert!(error.to_string().contains("src/["), "{error}");
    }
}
