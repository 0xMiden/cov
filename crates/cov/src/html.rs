//! A self-contained HTML page of a [`Comparison`]: totals, an index of the
//! files, and the source of every file with its line, function, and branch
//! coverage.
//!
//! The page is a single file with an inline style and no script, and refers
//! to nothing outside itself, so a browser shows it straight from a workflow
//! artifact.

use std::{
    collections::BTreeMap,
    fs,
    io::{self, Write},
};

use crate::{
    BranchCoverage, BranchKind, FunctionCoverage,
    compare::{Comparison, FileComparison, Group},
    markdown::{METRICS, delta, label},
};

/// Default of [`HtmlOptions::source_limit`].
pub const SOURCE_LIMIT: usize = 16 * 1024 * 1024;

/// How the page is drawn.
#[derive(Debug, Clone)]
pub struct HtmlOptions {
    /// Title of the page.
    pub title: String,
    /// Most bytes of source text drawn on the page. The files past it are
    /// listed without their source.
    pub source_limit: usize,
}

impl Default for HtmlOptions {
    fn default() -> Self {
        Self {
            title: "Coverage".into(),
            source_limit: SOURCE_LIMIT,
        }
    }
}

/// The style sheet of the page. Source rows have the columns line number,
/// hit count, changed marker, source text, and notes.
const STYLE: &str = "\
body { font: 14px/1.4 system-ui, sans-serif; margin: 1.5em; color: #1f2328; }
table { border-collapse: collapse; }
th, td { padding: 2px 8px; text-align: left; }
.totals td, .totals th, .index td, .index th { border-bottom: 1px solid #d0d7de; }
.delta { color: #57606a; }
.source { font: 12px/1.5 ui-monospace, monospace; width: 100%; margin-bottom: 2em; }
.source td { padding: 0 6px; vertical-align: top; }
.source td:nth-child(1), .source td:nth-child(2) { text-align: right; color: #6e7781; }
.source td:nth-child(1) a { color: inherit; text-decoration: none; }
.source td:nth-child(3) { color: #0969da; font-weight: bold; }
.source td:nth-child(4) { white-space: pre; width: 100%; }
.source td:nth-child(5) { white-space: nowrap; }
.covered td:nth-child(2), .covered td:nth-child(4) { background: #dafbe1; }
.uncovered td:nth-child(2), .uncovered td:nth-child(4) { background: #ffebe9; }
.partial td:nth-child(2), .partial td:nth-child(4) { background: #fff8c5; }
.tag { border-radius: 3px; padding: 0 4px; margin-left: 4px; background: #eaeef2; }
.miss { background: #ffcecb; }
.none { color: #6e7781; font-style: italic; }
.note { color: #57606a; }
:target { outline: 2px solid #0969da; }
";

/// Writes the page for `comparison`.
pub fn write(
    comparison: &Comparison<'_>,
    options: &HtmlOptions,
    out: &mut dyn Write,
) -> io::Result<()> {
    let title = escape(&options.title);
    write!(
        out,
        "<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n\
         <title>{title}</title>\n<style>\n{STYLE}</style>\n</head>\n<body>\n<h1>{title}</h1>\n"
    )?;
    write_totals(comparison, out)?;
    let unresolved = comparison.head.unresolved_summary();
    if !unresolved.is_empty() {
        writeln!(
            out,
            "<details class=\"note\"><summary>Unresolved sources ({})</summary><ul>",
            unresolved.len()
        )?;
        for line in &unresolved {
            writeln!(out, "<li>{}</li>", escape(line))?;
        }
        writeln!(out, "</ul></details>")?;
    }
    write_index(comparison, out)?;
    write_sources(&comparison.files, options.source_limit, out)?;
    writeln!(out, "</body>\n</html>")
}

/// The anchor of the section of the file at `path`, the path of a
/// [`FileComparison`]: `file-` and the path with every byte other than
/// `[A-Za-z0-9./-]` written as `_xx` in lowercase hex.
///
/// Distinct paths have distinct anchors, and an anchor is safe in an HTML
/// attribute and a URL fragment as it is.
pub fn anchor(path: &str) -> String {
    let mut anchor = String::from("file-");
    for byte in path.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'/' | b'-') {
            anchor.push(char::from(byte));
        } else {
            anchor.push_str(&format!("_{byte:02x}"));
        }
    }
    anchor
}

/// The totals table, against the baseline when there is one.
fn write_totals(comparison: &Comparison<'_>, out: &mut dyn Write) -> io::Result<()> {
    let base = comparison.base_totals;
    write!(out, "<table class=\"totals\">\n<tr><th></th>")?;
    if base.is_some() {
        write!(out, "<th>Base</th>")?;
    }
    write!(out, "<th>Head</th>")?;
    if base.is_some() {
        write!(out, "<th>Δ</th>")?;
    }
    writeln!(out, "</tr>")?;
    for (name, pick) in METRICS {
        let head = pick(&comparison.totals);
        write!(out, "<tr><th>{name}</th>")?;
        match &base {
            Some(base) => write!(
                out,
                "<td>{}</td><td>{}</td><td>{}</td>",
                label(pick(base)),
                label(head),
                delta(head, pick(base))
            )?,
            None => write!(out, "<td>{}</td>", label(head))?,
        }
        writeln!(out, "</tr>")?;
    }
    writeln!(out, "</table>")
}

/// The index of every file with its totals, then the files of the baseline
/// that the head no longer reports.
fn write_index(comparison: &Comparison<'_>, out: &mut dyn Write) -> io::Result<()> {
    writeln!(
        out,
        "<h2>Files</h2>\n<table class=\"index\">\n<tr><th></th><th>File</th><th>Lines</th>\
         <th>Functions</th><th>Branches</th><th>Patch</th></tr>"
    )?;
    for file in &comparison.files {
        let group = match file.group {
            Group::Changed => "changed",
            Group::Moved => "moved",
            Group::Unchanged => "unchanged",
        };
        write!(
            out,
            "<tr><td>{group}</td><td><a href=\"#{}\">{}</a></td>",
            anchor(&file.path),
            escape(&file.path)
        )?;
        for (_, pick) in METRICS {
            let head = pick(&file.totals);
            write!(out, "<td>{}", label(head))?;
            if let Some(base) = &file.base {
                write!(out, " <span class=\"delta\">{}</span>", delta(head, pick(base)))?;
            }
            write!(out, "</td>")?;
        }
        let patch = file.patch().map_or_else(|| "-".into(), label);
        writeln!(out, "<td>{patch}</td></tr>")?;
    }
    writeln!(out, "</table>")?;

    if !comparison.removed.is_empty() {
        writeln!(
            out,
            "<h2>Removed files</h2>\n<table class=\"index\">\n<tr><th>File</th>\
             <th>Base lines</th><th>Base functions</th><th>Base branches</th></tr>"
        )?;
        for (path, base) in &comparison.removed {
            write!(out, "<tr><td>{}</td>", escape(path))?;
            for (_, pick) in METRICS {
                write!(out, "<td>{}</td>", label(pick(base)))?;
            }
            writeln!(out, "</tr>")?;
        }
        writeln!(out, "</table>")?;
    }
    Ok(())
}

/// One section per file with its source, until drawing the next file would
/// take the source text past `limit` bytes; the files from there on are
/// only listed.
fn write_sources(
    files: &[FileComparison<'_>],
    limit: usize,
    out: &mut dyn Write,
) -> io::Result<()> {
    let mut drawn = 0;
    for (idx, file) in files.iter().enumerate() {
        let source = fs::read_to_string(&file.coverage.path);
        let size = source.as_ref().map_or(0, String::len);
        if drawn + size > limit {
            writeln!(out, "<h2>Not drawn (page size limit)</h2>\n<ul>")?;
            for file in &files[idx..] {
                // The index links here, so the anchor of the file's section
                // still lands on its name.
                writeln!(out, "<li id=\"{}\">{}</li>", anchor(&file.path), escape(&file.path))?;
            }
            return writeln!(out, "</ul>");
        }
        drawn += size;
        write_file(file, source, out)?;
    }
    Ok(())
}

/// The section of one file: its totals and every line of `source`, or a note
/// when the source cannot be read.
fn write_file(
    file: &FileComparison<'_>,
    source: io::Result<String>,
    out: &mut dyn Write,
) -> io::Result<()> {
    let anchor = anchor(&file.path);
    writeln!(
        out,
        "<section id=\"{anchor}\">\n<h2><a href=\"#{anchor}\">{}</a></h2>",
        escape(&file.path)
    )?;
    let text = match source {
        Ok(text) => text,
        Err(error) => {
            return writeln!(
                out,
                "<p class=\"note\">The source of {} cannot be read: {}</p>\n</section>",
                escape(&file.coverage.path.display().to_string()),
                escape(&error.to_string())
            );
        },
    };

    let mut functions: BTreeMap<u32, Vec<&FunctionCoverage>> = BTreeMap::new();
    for function in &file.coverage.functions {
        functions.entry(function.line).or_default().push(function);
    }
    let mut branches: BTreeMap<u32, Vec<&BranchCoverage>> = BTreeMap::new();
    for branch in &file.coverage.branches {
        branches.entry(branch.line).or_default().push(branch);
    }

    writeln!(out, "<table class=\"source\">")?;
    for (n, line) in (1u32..).zip(text.lines()) {
        let count = file.coverage.lines.get(&n);
        let branches = branches.get(&n).map_or(&[][..], Vec::as_slice);
        let partial = branches.iter().any(|branch| branch.executed && branch.arms.contains(&0));
        let class = match count {
            _ if partial => " class=\"partial\"",
            Some(0) => " class=\"uncovered\"",
            Some(_) => " class=\"covered\"",
            None => "",
        };
        let count = count.map(u64::to_string).unwrap_or_default();
        let changed = if file.changed_lines.contains(&n) { "+" } else { "" };
        write!(
            out,
            "<tr id=\"{anchor}-L{n}\"{class}><td><a href=\"#{anchor}-L{n}\">{n}</a></td>\
             <td>{count}</td><td>{changed}</td><td>{}</td><td>",
            escape(line)
        )?;
        for function in functions.get(&n).into_iter().flatten() {
            let miss = if function.count == 0 { " miss" } else { "" };
            write!(
                out,
                "<span class=\"tag{miss}\">fn {} ×{}</span>",
                escape(&function.name),
                function.count
            )?;
        }
        for branch in branches {
            write_branch(branch, out)?;
        }
        writeln!(out, "</td></tr>")?;
    }
    writeln!(out, "</table>\n</section>")
}

/// The marks of one branch: each arm with how often it was taken, or
/// `not evaluated` when the branch was never reached.
fn write_branch(branch: &BranchCoverage, out: &mut dyn Write) -> io::Result<()> {
    if !branch.executed {
        return write!(out, "<span class=\"tag none\">not evaluated</span>");
    }
    let arms = match branch.kind {
        Some(BranchKind::If) => ["true", "false"],
        Some(BranchKind::While) => ["iterations", "exits"],
        None => ["arm 1", "arm 2"],
    };
    for (arm, count) in arms.into_iter().zip(branch.arms) {
        let miss = if count == 0 { " miss" } else { "" };
        write!(out, "<span class=\"tag{miss}\">{arm} ×{count}</span>")?;
    }
    Ok(())
}

/// `text` safe as element content and as a quoted attribute value.
///
/// Invariant: every piece of text on the page goes through here. The page
/// embeds file contents and paths from the checkout of a pull request, which
/// an untrusted author controls, and must not let them add markup.
fn escape(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
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
        FileCoverage, Report,
        compare::{PathMatcher, compare},
        diff::Diff,
    };

    const SOURCE: &str = "\
proc f
    if.true
        push.1
    end
    while.true
        x
    end
end
<script>alert(\"&'\")</script>
";

    /// A report of one file, written to `dir` as `name`, with lines 2 and 3
    /// hit, line 6 not hit, one function per kind of branch line, and one
    /// branch of every kind.
    fn report(dir: &Path, name: &str) -> Report {
        // Canonical, as the report keys of `build_report` are.
        let path = dir.canonicalize().unwrap().join(name);
        fs::write(&path, SOURCE).unwrap();
        let branch = |line, block, executed, arms, kind| BranchCoverage {
            line,
            block,
            executed,
            arms,
            kind,
        };
        let file = FileCoverage {
            path: path.clone(),
            lines: [(2, 2), (3, 1), (5, 3), (6, 0)].into_iter().collect(),
            functions: vec![FunctionCoverage {
                name: "f<\"x\">".into(),
                line: 1,
                count: 2,
            }],
            branches: vec![
                branch(2, 10, true, [2, 0], Some(BranchKind::If)),
                branch(5, 40, true, [0, 3], Some(BranchKind::While)),
                branch(6, 50, true, [1, 1], None),
                branch(6, 60, false, [0, 0], Some(BranchKind::If)),
            ],
        };
        Report {
            files: [(path.display().to_string(), file)].into_iter().collect(),
            ..Default::default()
        }
    }

    fn render(comparison: &Comparison<'_>, source_limit: usize) -> String {
        let options = HtmlOptions { title: "T<i>".into(), source_limit };
        let mut out = Vec::new();
        write(comparison, &options, &mut out).unwrap();
        String::from_utf8(out).unwrap()
    }

    /// The row of line `n` of the file at `path`.
    fn row<'a>(page: &'a str, path: &str, n: u32) -> &'a str {
        let id = format!("<tr id=\"{}-L{n}\"", anchor(path));
        page.lines().find(|line| line.starts_with(&id)).unwrap()
    }

    #[test]
    fn source_lines_carry_their_coverage() {
        let dir = tempfile::tempdir().unwrap();
        let head = report(dir.path(), "a.masm");
        let diff = Diff::parse("+++ b/a.masm\n@@ -2,0 +3,2 @@\n+x\n+y\n");
        let paths = PathMatcher::new(dir.path(), vec![]);
        let page = render(&compare(&head, Some(&head), Some(&diff), &paths), SOURCE_LIMIT);

        assert!(page.contains("<section id=\"file-a.masm\">"), "{page}");
        assert!(page.contains("<td><a href=\"#file-a.masm\">a.masm</a></td>"), "{page}");
        assert_eq!(
            row(&page, "a.masm", 1),
            "<tr id=\"file-a.masm-L1\"><td><a href=\"#file-a.masm-L1\">1</a></td><td></td>\
             <td></td><td>proc f</td><td><span class=\"tag\">fn f&lt;&quot;x&quot;&gt; ×2</span>\
             </td></tr>"
        );
        assert_eq!(
            row(&page, "a.masm", 2),
            "<tr id=\"file-a.masm-L2\" class=\"partial\"><td><a href=\"#file-a.masm-L2\">2</a>\
             </td><td>2</td><td></td><td>    if.true</td><td><span class=\"tag\">true ×2</span>\
             <span class=\"tag miss\">false ×0</span></td></tr>"
        );
        assert_eq!(
            row(&page, "a.masm", 3),
            "<tr id=\"file-a.masm-L3\" class=\"covered\"><td><a href=\"#file-a.masm-L3\">3</a>\
             </td><td>1</td><td>+</td><td>        push.1</td><td></td></tr>"
        );
        assert!(row(&page, "a.masm", 4).contains("<td>+</td>"));
        assert!(row(&page, "a.masm", 5).contains(
            "<span class=\"tag miss\">iterations ×0</span><span class=\"tag\">exits ×3</span>"
        ));
        assert!(
            row(&page, "a.masm", 6).starts_with("<tr id=\"file-a.masm-L6\" class=\"uncovered\">")
        );
        assert!(row(&page, "a.masm", 6).ends_with(
            "<span class=\"tag\">arm 1 ×1</span><span class=\"tag\">arm 2 ×1</span>\
             <span class=\"tag none\">not evaluated</span></td></tr>"
        ));
        assert!(
            row(&page, "a.masm", 9)
                .contains("<td>&lt;script&gt;alert(&quot;&amp;&#39;&quot;)&lt;/script&gt;</td>")
        );
        assert!(!page.contains("L10"));
    }

    #[test]
    fn page_is_self_contained_and_escaped() {
        let dir = tempfile::tempdir().unwrap();
        let name = "x<y\"z'.masm";
        let mut head = report(dir.path(), name);
        head.unresolved.insert("pkg".into(), ["/a/<b>.masm".to_owned()].into());
        let paths = PathMatcher::new(dir.path(), vec![]);
        let page = render(&compare(&head, None, None, &paths), SOURCE_LIMIT);

        for forbidden in ["<script", "<link", "src=", "url(", "<i>", "<b>", "x<y", "f<\""] {
            assert!(!page.contains(forbidden), "{forbidden} in {page}");
        }
        for (idx, _) in page.match_indices("href=\"") {
            assert!(page[idx..].starts_with("href=\"#"), "{}", &page[idx..idx + 40]);
        }
        let anchor = anchor(name);
        assert_eq!(anchor, "file-x_3cy_22z_27.masm");
        assert!(page.contains(&format!("<a href=\"#{anchor}\">x&lt;y&quot;z&#39;.masm</a>")));
        assert!(page.contains("<title>T&lt;i&gt;</title>"));
        assert!(page.contains("<li>pkg: source file not found: /a/&lt;b&gt;.masm</li>"));
    }

    #[test]
    fn anchors_are_injective() {
        assert_eq!(anchor("a_b/c d.masm"), "file-a_5fb/c_20d.masm");
        assert_ne!(anchor("a_20"), anchor("a "));
        assert_eq!(anchor("é"), "file-_c3_a9");
    }

    #[test]
    fn files_past_the_size_limit_are_listed() {
        let dir = tempfile::tempdir().unwrap();
        let mut head = report(dir.path(), "a.masm");
        for name in ["b.masm", "c.masm"] {
            let more = report(dir.path(), name);
            head.files.extend(more.files);
        }
        let paths = PathMatcher::new(dir.path(), vec![]);
        let page = render(&compare(&head, None, None, &paths), 2 * SOURCE.len());

        assert!(page.contains("<section id=\"file-a.masm\">"));
        assert!(page.contains("<section id=\"file-b.masm\">"));
        assert!(!page.contains("<section id=\"file-c.masm\">"));
        assert!(page.contains(
            "<h2>Not drawn (page size limit)</h2>\n<ul>\n<li id=\"file-c.masm\">c.masm</li>\n</ul>"
        ));
        // The index still has every file.
        assert!(page.contains("<a href=\"#file-c.masm\">c.masm</a>"));
    }

    #[test]
    fn an_unreadable_file_is_noted() {
        let dir = tempfile::tempdir().unwrap();
        let head = report(dir.path(), "a.masm");
        fs::remove_file(dir.path().join("a.masm")).unwrap();
        let paths = PathMatcher::new(dir.path(), vec![]);
        let page = render(&compare(&head, None, None, &paths), SOURCE_LIMIT);

        assert!(page.contains("<section id=\"file-a.masm\">"));
        assert!(page.contains("cannot be read"), "{page}");
        assert!(!page.contains("file-a.masm-L1"));
    }
}
