//! The lines a change adds, read from a unified diff.

use std::collections::{BTreeMap, BTreeSet};

/// The files a unified diff changes, by their path after the change, each
/// with the 1-based lines it adds or modifies.
///
/// Paths are as `git diff` prints them, relative to the repository root,
/// without the `b/` prefix. A file the change deletes is not listed; a file
/// whose hunks only remove lines is listed with no lines.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Diff {
    /// Added lines by new path.
    pub files: BTreeMap<String, BTreeSet<u32>>,
}

impl Diff {
    /// Reads the output of `git diff` or `diff -u`.
    ///
    /// Anything outside file headers and hunks, such as `diff --git` lines,
    /// mode changes, and binary-file notes, is skipped.
    pub fn parse(text: &str) -> Self {
        let mut diff = Self::default();
        // The new path of the file the hunks belong to.
        let mut file: Option<String> = None;
        // The next line number of the new file and the lines of the old and
        // the new file the current hunk still has to show.
        let (mut next, mut old_left, mut new_left) = (0u32, 0u32, 0u32);
        for line in text.lines() {
            // Inside a hunk the counts, not the content, tell where it ends:
            // a removed line may start with `--` and an added one with `++`.
            if old_left > 0 || new_left > 0 {
                match line.as_bytes().first() {
                    Some(b'+') => {
                        if let Some(lines) = file.as_ref().and_then(|path| diff.files.get_mut(path))
                        {
                            lines.insert(next);
                        }
                        next += 1;
                        new_left = new_left.saturating_sub(1);
                    },
                    Some(b'-') => old_left = old_left.saturating_sub(1),
                    // `\ No newline at end of file` belongs to the line above.
                    Some(b'\\') => {},
                    // A context line; some tools strip the space of an
                    // empty one.
                    _ => {
                        next += 1;
                        old_left = old_left.saturating_sub(1);
                        new_left = new_left.saturating_sub(1);
                    },
                }
                continue;
            }
            if let Some(path) = line.strip_prefix("+++ ") {
                file = new_path(path);
                if let Some(path) = &file {
                    diff.files.entry(path.clone()).or_default();
                }
            } else if let Some(hunk) = line.strip_prefix("@@ ")
                && let Some((old, new)) = hunk_ranges(hunk)
            {
                (next, old_left, new_left) = (new.0, old.1, new.1);
            }
        }
        diff
    }
}

/// The path after the change from the value of a `+++` header, or `None`
/// for a deleted file.
fn new_path(header: &str) -> Option<String> {
    // `diff -u` follows the path with a tab and a timestamp.
    let path = header.split('\t').next().unwrap_or(header);
    let path = match path.strip_prefix('"').and_then(|path| path.strip_suffix('"')) {
        Some(quoted) => unquote(quoted),
        None => path.to_owned(),
    };
    if path == "/dev/null" {
        return None;
    }
    Some(path.strip_prefix("b/").map(str::to_owned).unwrap_or(path))
}

/// Undoes the C-style quoting git applies to paths with special characters:
/// `\"`, `\\`, `\t`, `\n`, and octal escapes of the UTF-8 bytes.
fn unquote(quoted: &str) -> String {
    let mut bytes = Vec::with_capacity(quoted.len());
    let mut rest = quoted.bytes().peekable();
    while let Some(byte) = rest.next() {
        if byte != b'\\' {
            bytes.push(byte);
            continue;
        }
        match rest.next() {
            Some(b't') => bytes.push(b'\t'),
            Some(b'n') => bytes.push(b'\n'),
            Some(digit @ b'0'..=b'7') => {
                let mut value = u32::from(digit - b'0');
                for _ in 0..2 {
                    if let Some(digit @ b'0'..=b'7') = rest.peek().copied() {
                        value = value * 8 + u32::from(digit - b'0');
                        rest.next();
                    }
                }
                bytes.push(value as u8);
            },
            Some(other) => bytes.push(other),
            None => bytes.push(b'\\'),
        }
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

/// The `(start, count)` of the old and the new side of a hunk header
/// `-a,b +c,d @@ ...`, after the leading `@@ `.
fn hunk_ranges(hunk: &str) -> Option<((u32, u32), (u32, u32))> {
    let mut fields = hunk.split(' ');
    let old = range(fields.next()?.strip_prefix('-')?)?;
    let new = range(fields.next()?.strip_prefix('+')?)?;
    Some((old, new))
}

/// `start,count`, or `start` alone for a count of one.
fn range(range: &str) -> Option<(u32, u32)> {
    match range.split_once(',') {
        Some((start, count)) => Some((start.parse().ok()?, count.parse().ok()?)),
        None => Some((range.parse().ok()?, 1)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(diff: &Diff, path: &str) -> Vec<u32> {
        diff.files[path].iter().copied().collect()
    }

    #[test]
    fn added_lines_are_numbered_in_the_new_file() {
        let diff = Diff::parse(
            "\
diff --git a/asm/wallet.masm b/asm/wallet.masm
index 1111111..2222222 100644
--- a/asm/wallet.masm
+++ b/asm/wallet.masm
@@ -1,4 +1,5 @@ proc deposit
 a
-b
+B
+++c
 d

@@ -20 +21,2 @@
-x
+--y
+z
\\ No newline at end of file
diff --git a/gone.masm b/gone.masm
deleted file mode 100644
--- a/gone.masm
+++ /dev/null
@@ -1 +0,0 @@
-gone
diff --git a/shrunk.masm b/shrunk.masm
--- a/shrunk.masm
+++ b/shrunk.masm
@@ -3,2 +2,0 @@
-p
-q
diff --git a/img.png b/img.png
Binary files a/img.png and b/img.png differ
",
        );
        assert_eq!(diff.files.keys().collect::<Vec<_>>(), ["asm/wallet.masm", "shrunk.masm"]);
        // The empty context line is line 5; `+--y` and `+++c` are added lines,
        // not headers.
        assert_eq!(lines(&diff, "asm/wallet.masm"), [2, 3, 21, 22]);
        assert!(diff.files["shrunk.masm"].is_empty());
    }

    #[test]
    fn plain_and_quoted_paths() {
        let diff = Diff::parse(
            "\
--- old/a.masm\t2026-10-29 10:00:00
+++ new/a.masm\t2026-10-29 10:01:00
@@ -0,0 +1 @@
+a
+++ \"b/sp ace/\\303\\251\\\"q\\\".masm\"
@@ -0,0 +1 @@
+b
",
        );
        assert_eq!(lines(&diff, "new/a.masm"), [1]);
        assert_eq!(lines(&diff, "sp ace/é\"q\".masm"), [1]);
    }
}
