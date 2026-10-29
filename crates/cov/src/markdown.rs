//! A pull-request comment of a [`Comparison`] in GitHub-flavored markdown:
//! totals against the baseline, the files whose coverage changed, and the
//! functions no longer entered.

use std::io::{self, Write};

use crate::{
    compare::{Comparison, FileComparison, Group, Ratio, Totals},
    html::anchor,
};

/// Size the comment stays strictly below, in bytes: the CI comment pipeline
/// rejects a larger one.
pub const MARKDOWN_LIMIT: usize = 60 * 1024;

/// Rows each list shows before the size guard starts shrinking it.
const ROWS: usize = 200;

/// How the comment is written.
#[derive(Debug, Clone)]
pub struct MarkdownOptions {
    /// Heading of the comment.
    pub title: String,
    /// Where the HTML page is published, linked from the comment and its file
    /// rows.
    pub page_url: Option<String>,
}

impl Default for MarkdownOptions {
    fn default() -> Self {
        Self { title: "Coverage".into(), page_url: None }
    }
}

/// Reads one measure of [`Totals`].
pub(crate) type Measure = fn(&Totals) -> Ratio;

/// The three measures of [`Totals`] with their names, in display order.
pub(crate) const METRICS: [(&str, Measure); 3] = [
    ("Lines", |totals| totals.lines),
    ("Functions", |totals| totals.functions),
    ("Branches", |totals| totals.branches),
];

/// `90.0%`, or `-` when there is nothing to cover.
pub(crate) fn percent(ratio: Ratio) -> String {
    ratio.percent().map_or_else(|| "-".into(), |percent| format!("{percent:.1}%"))
}

/// `90.0% (9/10)`, or `-` when there is nothing to cover.
pub(crate) fn label(ratio: Ratio) -> String {
    match ratio.percent() {
        Some(percent) => format!("{percent:.1}% ({}/{})", ratio.hit, ratio.total),
        None => "-".into(),
    }
}

/// The change from `base` to `head` in percentage points: `+1.2%`, `-0.5%`,
/// `0.0%`, or `-` when either has nothing to cover.
pub(crate) fn delta(head: Ratio, base: Ratio) -> String {
    match (head.percent(), base.percent()) {
        (Some(head), Some(base)) => {
            // Rounded first so that a change too small to show is `0.0%`,
            // never `+0.0%` or `-0.0%`.
            let delta = ((head - base) * 10.0).round() / 10.0;
            if delta == 0.0 {
                "0.0%".into()
            } else {
                format!("{delta:+.1}%")
            }
        },
        _ => "-".into(),
    }
}

/// Writes the comment for `comparison`, always strictly smaller than
/// [`MARKDOWN_LIMIT`].
///
/// Long lists are cut to fit, pointing at the page for the rest. Fails with
/// [`io::ErrorKind::InvalidInput`] when even the comment without any list
/// row does not fit, which only an oversized title or page URL can cause.
pub fn write(
    comparison: &Comparison<'_>,
    options: &MarkdownOptions,
    out: &mut dyn Write,
) -> io::Result<()> {
    let mut rows = ROWS;
    loop {
        let mut text = Vec::new();
        render(comparison, options, rows, &mut text)?;
        if text.len() < MARKDOWN_LIMIT {
            return out.write_all(&text);
        }
        if rows == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("the coverage comment does not fit in {MARKDOWN_LIMIT} bytes"),
            ));
        }
        rows /= 2;
    }
}

/// Writes the comment with at most `rows` rows per list.
fn render(
    comparison: &Comparison<'_>,
    options: &MarkdownOptions,
    rows: usize,
    out: &mut Vec<u8>,
) -> io::Result<()> {
    let page = options.page_url.as_deref();
    writeln!(out, "## {}\n", escape(&options.title))?;
    if let Some(url) = page {
        writeln!(out, "[Coverage page]({url})\n")?;
    }
    write_totals(comparison, out)?;
    write_files(comparison, page, rows, out)?;

    let functions = &comparison.no_longer_entered;
    if !functions.is_empty() {
        writeln!(
            out,
            "<details><summary>Functions no longer entered ({})</summary>\n",
            functions.len()
        )?;
        for (file, name) in functions.iter().take(rows) {
            writeln!(out, "- {}: {}", code(file), code(name))?;
        }
        write_more(functions.len(), rows, page, out)?;
        writeln!(out, "\n</details>\n")?;
    }

    let unresolved = comparison.head.unresolved_summary();
    if !unresolved.is_empty() {
        writeln!(out, "<details><summary>Unresolved sources ({})</summary>\n", unresolved.len())?;
        for line in unresolved.iter().take(rows) {
            writeln!(out, "- {}", escape(line))?;
        }
        write_more(unresolved.len(), rows, page, out)?;
        writeln!(out, "\n</details>")?;
    }
    Ok(())
}

/// The totals table, against the baseline when there is one.
fn write_totals(comparison: &Comparison<'_>, out: &mut Vec<u8>) -> io::Result<()> {
    let head = &comparison.totals;
    match &comparison.base_totals {
        Some(base) => {
            writeln!(out, "| | Base | Head | Δ |\n|---|--:|--:|--:|")?;
            for (name, pick) in METRICS {
                let (head, base) = (pick(head), pick(base));
                writeln!(
                    out,
                    "| {name} | {} | {} | {} |",
                    label(base),
                    label(head),
                    delta(head, base)
                )?;
            }
            writeln!(out)
        },
        None => {
            writeln!(out, "| | Head |\n|---|--:|")?;
            for (name, pick) in METRICS {
                writeln!(out, "| {name} | {} |", label(pick(head)))?;
            }
            writeln!(out, "\nNo baseline was found, so only the head is shown.\n")
        },
    }
}

/// The table of changed, moved, and removed files, at most `rows` of them.
fn write_files(
    comparison: &Comparison<'_>,
    page: Option<&str>,
    rows: usize,
    out: &mut Vec<u8>,
) -> io::Result<()> {
    let listed: Vec<&FileComparison<'_>> =
        comparison.files.iter().filter(|file| file.group != Group::Unchanged).collect();
    let removed = &comparison.removed;
    if listed.is_empty() && removed.is_empty() {
        return writeln!(out, "No file was changed or changed its coverage.\n");
    }

    writeln!(out, "| File | Lines | Functions | Branches | Patch |\n|---|--:|--:|--:|--:|")?;
    for file in listed.iter().take(rows) {
        let name = match page {
            Some(url) => format!("[{}]({url}#{})", code(&file.path), anchor(&file.path)),
            None => code(&file.path),
        };
        write!(out, "| {name} |")?;
        for (_, pick) in METRICS {
            let head = pick(&file.totals);
            write!(out, " {}", percent(head))?;
            match file.base.map(|base| delta(head, pick(&base))) {
                Some(delta) if delta != "-" => write!(out, " ({delta}) |")?,
                _ => write!(out, " |")?,
            }
        }
        let patch = file
            .patch()
            .and_then(|patch| {
                let percent = patch.percent()?;
                Some(format!("{}/{} ({percent:.1}%)", patch.hit, patch.total))
            })
            .unwrap_or_else(|| "-".into());
        writeln!(out, " {patch} |")?;
    }
    for (path, base) in removed.iter().take(rows.saturating_sub(listed.len())) {
        write!(out, "| {} (removed) |", code(path))?;
        for (_, pick) in METRICS {
            match pick(base).percent() {
                Some(percent) => write!(out, " was {percent:.1}% |")?,
                None => write!(out, " - |")?,
            }
        }
        writeln!(out, " - |")?;
    }
    write_more(listed.len() + removed.len(), rows, page, out)?;
    writeln!(out)
}

/// The line standing in for the `total - rows` items a list leaves out, if
/// any.
fn write_more(total: usize, rows: usize, page: Option<&str>, out: &mut Vec<u8>) -> io::Result<()> {
    if total <= rows {
        return Ok(());
    }
    let more = total - rows;
    // The blank line ends a table, which would take the line as a row.
    match page {
        Some(_) => writeln!(out, "\n… and {more} more, see the page"),
        None => writeln!(out, "\n… and {more} more"),
    }
}

/// `text` as inline code that is safe anywhere, table cells included.
fn code(text: &str) -> String {
    format!("<code>{}</code>", escape(text))
}

/// `text` as literal inline text: every ASCII punctuation character that
/// markdown or HTML could read as syntax becomes a character reference, and
/// control characters become spaces.
///
/// File paths and function names come from the checkout of a pull request,
/// which may name a file to break the table or inject markup. Character
/// references, unlike backslash escapes or code spans, also keep `|` from
/// splitting a table cell.
fn escape(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            c if c.is_alphanumeric() || matches!(c, ' ' | '.' | '/' | '-' | ',' | ':') => {
                escaped.push(c)
            },
            c if c.is_control() => escaped.push(' '),
            c if c.is_ascii() => escaped.push_str(&format!("&#{};", u32::from(c))),
            c => escaped.push(c),
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::{
        FileCoverage, FunctionCoverage, Report,
        compare::{PathMatcher, compare},
        diff::Diff,
    };

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

    fn render(comparison: &Comparison<'_>, page_url: Option<&str>) -> String {
        let options = MarkdownOptions {
            title: "MASM coverage".into(),
            page_url: page_url.map(str::to_owned),
        };
        let mut out = Vec::new();
        write(comparison, &options, &mut out).unwrap();
        String::from_utf8(out).unwrap()
    }

    fn paths() -> PathMatcher {
        PathMatcher::new(Path::new("/work"), vec![])
    }

    #[test]
    fn with_baseline_and_page() {
        let head = report(vec![
            file("/work/a.masm", &[(1, 1), (2, 1), (3, 0), (4, 1)], &[("a", 1)]),
            file("/work/b.masm", &[(1, 0)], &[("b", 0)]),
            file("/work/c.masm", &[(1, 1)], &[]),
        ]);
        let base = report(vec![
            file("/work/a.masm", &[(1, 1), (2, 0), (3, 0)], &[("a", 1)]),
            file("/work/b.masm", &[(1, 1)], &[("b", 1)]),
            file("/work/c.masm", &[(1, 1)], &[]),
            file("/work/gone.masm", &[(1, 1)], &[]),
        ]);
        let diff = Diff::parse("+++ b/a.masm\n@@ -1,0 +2,3 @@\n+x\n+y\n+z\n");
        let comparison = compare(&head, Some(&base), Some(&diff), &paths());
        let url = "https://example.com/p";
        assert_eq!(
            render(&comparison, Some(url)),
            format!(
                "\
## MASM coverage

[Coverage page]({url})

| | Base | Head | Δ |
|---|--:|--:|--:|
| Lines | 66.7% (4/6) | 66.7% (4/6) | 0.0% |
| Functions | 100.0% (2/2) | 50.0% (1/2) | -50.0% |
| Branches | - | - | - |

| File | Lines | Functions | Branches | Patch |
|---|--:|--:|--:|--:|
| [<code>a.masm</code>]({url}#file-a.masm) | 75.0% (+41.7%) | 100.0% (0.0%) | - | 2/3 (66.7%) |
| [<code>b.masm</code>]({url}#file-b.masm) | 0.0% (-100.0%) | 0.0% (-100.0%) | - | - |
| <code>gone.masm</code> (removed) | was 100.0% | - | - | - |

<details><summary>Functions no longer entered (1)</summary>

- <code>b.masm</code>: <code>b</code>

</details>

"
            )
        );
    }

    #[test]
    fn without_baseline_or_page() {
        let head = report(vec![file("/work/a.masm", &[(1, 1), (2, 0)], &[])]);
        let comparison = compare(&head, None, None, &paths());
        assert_eq!(
            render(&comparison, None),
            "\
## MASM coverage

| | Head |
|---|--:|
| Lines | 50.0% (1/2) |
| Functions | - |
| Branches | - |

No baseline was found, so only the head is shown.

No file was changed or changed its coverage.

"
        );
    }

    #[test]
    fn names_cannot_break_the_table() {
        let head = report(vec![file("/work/a|b`<i>_*.masm", &[(1, 1)], &[])]);
        let diff = Diff::parse("+++ b/a|b`<i>_*.masm\n@@ -1,0 +1 @@\n+x\n");
        let comparison = compare(&head, None, Some(&diff), &paths());
        let text = render(&comparison, None);
        let row = text.lines().find(|line| line.contains("a&#124;b")).unwrap();
        assert_eq!(
            row,
            "| <code>a&#124;b&#96;&#60;i&#62;&#95;&#42;.masm</code> | 100.0% | - | - | 1/1 (100.0%) |"
        );
        assert_eq!(escape("line\nbreak\r"), "line break ");
    }

    #[test]
    fn long_lists_are_cut_to_the_size_limit() {
        let long = "x".repeat(200);
        let head = report(
            (0..5000)
                .map(|i| file(&format!("/work/{long}/{i}.masm"), &[(1, 0)], &[("f", 0)]))
                .collect(),
        );
        let base = report(
            (0..5000)
                .map(|i| file(&format!("/work/{long}/{i}.masm"), &[(1, 1)], &[("f", 1)]))
                .collect(),
        );
        let comparison = compare(&head, Some(&base), None, &paths());
        for page in [None, Some("https://example.com/p")] {
            let text = render(&comparison, page);
            assert!(text.len() < MARKDOWN_LIMIT, "{}", text.len());
            let rows = text.lines().filter(|line| line.starts_with("| [<code>")).count();
            if page.is_some() {
                assert!(rows > 0);
                assert!(text.contains(&format!("… and {} more, see the page", 5000 - rows)));
            } else {
                assert!(text.contains("more\n"));
            }
            assert!(text.contains("Functions no longer entered (5000)"));
        }
    }

    #[test]
    fn deltas_are_rounded_before_the_sign() {
        let ratio = |hit, total| Ratio { hit, total };
        assert_eq!(delta(ratio(1, 3), ratio(1, 3)), "0.0%");
        assert_eq!(delta(ratio(10000, 30001), ratio(1, 3)), "0.0%");
        assert_eq!(delta(ratio(2, 3), ratio(1, 3)), "+33.3%");
        assert_eq!(delta(ratio(0, 0), ratio(1, 3)), "-");
    }
}
