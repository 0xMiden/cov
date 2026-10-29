//! A report set against a baseline report and a change, as a pull request
//! shows it.

use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

use crate::{FileCoverage, Report, diff::Diff, source::absolute_path};

/// The head report of a change compared with the report of its base.
#[derive(Debug, Clone)]
pub struct Comparison<'a> {
    /// The report of the head.
    pub head: &'a Report,
    /// Totals of the head.
    pub totals: Totals,
    /// Totals of the baseline, if one was given.
    pub base_totals: Option<Totals>,
    /// Every file of the head: the changed ones, then the moved ones, then
    /// the rest, each group in path order.
    pub files: Vec<FileComparison<'a>>,
    /// Files of the baseline that the head does not report, in path order,
    /// with their baseline totals.
    pub removed: Vec<(String, Totals)>,
    /// Functions entered in the baseline that the head reports as never
    /// entered, as `(file path, function name)`, in file order.
    pub no_longer_entered: Vec<(String, String)>,
}

/// One file of the head with its baseline and its changed lines.
#[derive(Debug, Clone)]
pub struct FileComparison<'a> {
    /// The path relative to the source root, or the full path for a file
    /// outside it.
    pub path: String,
    /// The head coverage of the file.
    pub coverage: &'a FileCoverage,
    /// Which group the file is listed in.
    pub group: Group,
    /// Totals of the head.
    pub totals: Totals,
    /// Totals of the baseline, if the baseline reports the file.
    pub base: Option<Totals>,
    /// Lines the change adds or modifies.
    pub changed_lines: BTreeSet<u32>,
}

impl FileComparison<'_> {
    /// How many of the changed lines that are coverable were hit, or `None`
    /// when the change does not touch the file.
    pub fn patch(&self) -> Option<Ratio> {
        (self.group == Group::Changed).then(|| {
            Ratio::of(
                self.changed_lines
                    .iter()
                    .filter_map(|line| self.coverage.lines.get(line).copied()),
            )
        })
    }
}

/// Why a file is listed where it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Group {
    /// The change touches the file.
    Changed,
    /// The change does not touch the file, but its coverage differs from
    /// the baseline, or the baseline does not report it.
    Moved,
    /// Neither.
    Unchanged,
}

/// Line, function, and branch arm counts of a file or a report.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Totals {
    /// Coverable lines.
    pub lines: Ratio,
    /// Functions.
    pub functions: Ratio,
    /// Branch arms, two per branch.
    pub branches: Ratio,
}

impl Totals {
    /// The totals of one file.
    pub fn of(file: &FileCoverage) -> Self {
        Self {
            lines: Ratio::of(file.lines.values().copied()),
            functions: Ratio::of(file.functions.iter().map(|function| function.count)),
            branches: Ratio::of(file.branches.iter().flat_map(|branch| branch.arms)),
        }
    }

    /// The totals of every file of `report`.
    pub fn of_report(report: &Report) -> Self {
        report.files.values().map(Self::of).fold(Self::default(), Self::add)
    }

    /// Both totals summed.
    pub fn add(self, other: Self) -> Self {
        Self {
            lines: self.lines.add(other.lines),
            functions: self.functions.add(other.functions),
            branches: self.branches.add(other.branches),
        }
    }
}

/// How many of `total` items were hit.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Ratio {
    /// Items hit at least once.
    pub hit: usize,
    /// All items.
    pub total: usize,
}

impl Ratio {
    /// Counts the non-zero entries of `counts`.
    pub fn of(counts: impl Iterator<Item = u64>) -> Self {
        counts.fold(Self::default(), |ratio, count| Self {
            hit: ratio.hit + usize::from(count > 0),
            total: ratio.total + 1,
        })
    }

    /// Both ratios summed.
    pub fn add(self, other: Self) -> Self {
        Self {
            hit: self.hit + other.hit,
            total: self.total + other.total,
        }
    }

    /// The hit share in percent, or `None` when there is nothing to cover.
    pub fn percent(&self) -> Option<f64> {
        (self.total > 0).then(|| 100.0 * self.hit as f64 / self.total as f64)
    }
}

impl core::fmt::Display for Ratio {
    /// `hit/total  pct%`, or `-` when there is nothing to cover.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self.percent() {
            Some(percent) => write!(f, "{}/{} {percent:5.1}%", self.hit, self.total),
            None => write!(f, "-"),
        }
    }
}

/// How the paths of the head, the baseline, and the change are matched:
/// all of them are taken relative to the source root.
#[derive(Debug, Clone)]
pub struct PathMatcher {
    root: PathBuf,
    prefix_maps: Vec<(String, String)>,
}

impl PathMatcher {
    /// Matches paths relative to `root`, the repository root the change's
    /// paths are relative to. Baseline paths are rewritten first with the
    /// first matching `(old, new)` prefix of `prefix_maps`.
    pub fn new(root: &Path, prefix_maps: Vec<(String, String)>) -> Self {
        Self { root: absolute_path(root), prefix_maps }
    }

    /// The path of a head file, whose report key is its canonical path.
    fn head(&self, key: &str) -> String {
        self.relative(key)
    }

    /// The path of a baseline file, given as its `SF` path.
    fn base(&self, path: &str) -> String {
        let mapped = self.prefix_maps.iter().find_map(|(old, new)| {
            path.strip_prefix(old.as_str()).map(|rest| format!("{new}{rest}"))
        });
        self.relative(mapped.as_deref().unwrap_or(path))
    }

    /// `path` relative to the root when it lies under it, otherwise as given.
    fn relative(&self, path: &str) -> String {
        Path::new(path)
            .strip_prefix(&self.root)
            .map_or_else(|_| path.to_owned(), |relative| relative.display().to_string())
    }
}

/// Compares `head` with `base` and the change `diff`.
///
/// Without a baseline nothing moves and no totals differ; without a change
/// nothing is changed.
pub fn compare<'a>(
    head: &'a Report,
    base: Option<&Report>,
    diff: Option<&Diff>,
    paths: &PathMatcher,
) -> Comparison<'a> {
    let base_files: Option<BTreeMap<String, &FileCoverage>> =
        base.map(|base| base.files.iter().map(|(path, file)| (paths.base(path), file)).collect());
    let mut files: Vec<FileComparison<'a>> = head
        .files
        .iter()
        .map(|(key, coverage)| {
            let path = paths.head(key);
            let totals = Totals::of(coverage);
            let base =
                base_files.as_ref().and_then(|files| files.get(&path)).map(|f| Totals::of(f));
            let changed_lines = diff.and_then(|diff| diff.files.get(&path));
            let group = match (changed_lines, &base_files) {
                (Some(_), _) => Group::Changed,
                (None, Some(_)) if base != Some(totals) => Group::Moved,
                _ => Group::Unchanged,
            };
            FileComparison {
                path,
                coverage,
                group,
                totals,
                base,
                changed_lines: changed_lines.cloned().unwrap_or_default(),
            }
        })
        .collect();
    files.sort_by(|a, b| (a.group, &a.path).cmp(&(b.group, &b.path)));

    let mut no_longer_entered = Vec::new();
    let mut removed = Vec::new();
    if let Some(base_files) = &base_files {
        let head_paths: BTreeSet<&str> = files.iter().map(|file| file.path.as_str()).collect();
        removed = base_files
            .iter()
            .filter(|(path, _)| !head_paths.contains(path.as_str()))
            .map(|(path, file)| (path.clone(), Totals::of(file)))
            .collect();
        for file in &files {
            let Some(base) = base_files.get(&file.path) else {
                continue;
            };
            for function in file.coverage.functions.iter().filter(|function| function.count == 0) {
                if base.functions.iter().any(|old| old.name == function.name && old.count > 0) {
                    no_longer_entered.push((file.path.clone(), function.name.clone()));
                }
            }
        }
    }

    Comparison {
        head,
        totals: Totals::of_report(head),
        base_totals: base.map(Totals::of_report),
        files,
        removed,
        no_longer_entered,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FunctionCoverage;

    fn file(path: &str, lines: &[(u32, u64)], functions: &[(&str, u64)]) -> FileCoverage {
        FileCoverage {
            path: path.into(),
            lines: lines.iter().copied().collect(),
            functions: functions
                .iter()
                .map(|(name, count)| FunctionCoverage {
                    name: (*name).into(),
                    line: 1,
                    count: *count,
                })
                .collect(),
            branches: Vec::new(),
        }
    }

    fn report(files: Vec<FileCoverage>) -> Report {
        Report {
            files: files.into_iter().map(|file| (file.path.display().to_string(), file)).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn files_are_grouped_and_matched_relative_to_the_root() {
        let head = report(vec![
            file("/work/a.masm", &[(1, 1), (2, 0), (3, 4)], &[("a", 1)]),
            file("/work/b.masm", &[(1, 1), (2, 0)], &[("b1", 0), ("b2", 1)]),
            file("/work/c.masm", &[(1, 1)], &[]),
            file("/work/new.masm", &[(1, 0)], &[]),
            file("/elsewhere/lib.masm", &[(1, 1)], &[]),
        ]);
        // The baseline was built in another checkout.
        let base = report(vec![
            file("/ci/a.masm", &[(1, 1), (2, 0)], &[("a", 1)]),
            file("/ci/b.masm", &[(1, 1), (2, 1)], &[("b1", 2), ("b2", 1)]),
            file("/ci/c.masm", &[(1, 1)], &[]),
            file("/ci/gone.masm", &[(1, 1)], &[]),
            file("/elsewhere/lib.masm", &[(1, 1)], &[]),
        ]);
        let diff = Diff::parse("+++ b/a.masm\n@@ -1,0 +2,2 @@\n+x\n+y\n");
        let paths = PathMatcher::new(Path::new("/work"), vec![("/ci/".into(), "/work/".into())]);
        let comparison = compare(&head, Some(&base), Some(&diff), &paths);

        let groups: Vec<(&str, Group)> =
            comparison.files.iter().map(|file| (file.path.as_str(), file.group)).collect();
        assert_eq!(
            groups,
            [
                ("a.masm", Group::Changed),
                ("b.masm", Group::Moved),
                ("new.masm", Group::Moved),
                ("/elsewhere/lib.masm", Group::Unchanged),
                ("c.masm", Group::Unchanged),
            ]
        );
        let a = &comparison.files[0];
        assert_eq!(a.patch(), Some(Ratio { hit: 1, total: 2 }));
        assert_eq!(a.base.unwrap().lines, Ratio { hit: 1, total: 2 });
        assert_eq!(comparison.files[1].patch(), None);
        assert_eq!(comparison.files[2].base, None);
        assert_eq!(
            comparison.removed,
            [(
                "gone.masm".to_owned(),
                Totals {
                    lines: Ratio { hit: 1, total: 1 },
                    ..Default::default()
                }
            )]
        );
        assert_eq!(comparison.no_longer_entered, [("b.masm".to_owned(), "b1".to_owned())]);
        assert_eq!(comparison.totals.lines, Ratio { hit: 5, total: 8 });
        assert_eq!(comparison.base_totals.unwrap().lines, Ratio { hit: 6, total: 7 });
    }

    #[test]
    fn head_only() {
        let head = report(vec![file("/work/a.masm", &[(1, 1)], &[])]);
        let comparison = compare(&head, None, None, &PathMatcher::new(Path::new("/work"), vec![]));
        assert_eq!(comparison.files[0].group, Group::Unchanged);
        assert!(comparison.base_totals.is_none() && comparison.removed.is_empty());
    }
}
