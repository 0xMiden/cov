//! Line, function, and branch coverage per source file.

use std::{
    cmp::Ordering,
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::Arc,
};

use miden_core::mast::MastNode;
use miden_cov_collector::PackageHits;

use crate::{
    Snapshot,
    source::{SourceResolver, SourceText},
};

/// Coverage of every resolved source file of a snapshot.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    /// Files keyed by the display form of their resolved path.
    pub files: BTreeMap<String, FileCoverage>,
    /// Stored debug-info paths that no file was found for.
    pub unresolved: BTreeSet<String>,
}

/// Coverage of one source file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileCoverage {
    /// Where the file was found.
    pub path: PathBuf,
    /// Hit count per coverable 1-based line.
    pub lines: BTreeMap<u32, u64>,
    /// Functions defined in the file, ordered by line and name.
    pub functions: Vec<FunctionCoverage>,
    /// Branches in the file, ordered by line and block.
    pub branches: Vec<BranchCoverage>,
}

/// How often one function was entered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionCoverage {
    /// The function name as recorded in debug info.
    pub name: String,
    /// 1-based line of the definition.
    pub line: u32,
    /// Number of times the function started.
    pub count: u64,
}

/// How often each arm of a two-way branch was taken.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchCoverage {
    /// 1-based line of the instruction introducing the branch.
    pub line: u32,
    /// Source node id of the branch, unique per branch within a package.
    pub block: u32,
    /// Whether the branch was reached at all.
    pub executed: bool,
    /// Times each arm was taken: `[true arm, false arm]`.
    pub arms: [u64; 2],
}

/// Applies the coverable rule to a snapshot.
///
/// Every row, function, and branch whose file resolves contributes to the
/// report, including those never hit; paths that do not resolve are listed in
/// [`Report::unresolved`].
pub fn build_report(snapshot: &Snapshot, resolver: &mut SourceResolver) -> Report {
    let mut files = Files { resolver, files: BTreeMap::new() };
    let no_hits = PackageHits::default();

    for (commitment, package) in &snapshot.packages {
        let hits = snapshot.merged.get(commitment).unwrap_or(&no_hits);
        let debug_info = &package.debug_info;
        let node_starts = |node: u32| hits.node_starts.get(&node).copied().unwrap_or(0);

        for (node_id, node) in debug_info.nodes().iter().enumerate() {
            let node_id = node_id as u32;
            for row in &node.asm_ops {
                let Some(location) =
                    row.location_idx.into_option().and_then(|idx| debug_info.get_location(idx))
                else {
                    continue;
                };
                let Some((file, source)) = files.get(location.uri.as_str()) else {
                    continue;
                };
                // Control-flow rows (`if.true`, `while.true`, `begin`, ...)
                // span their whole block; only the line they start on is
                // credited, otherwise every line of the block would look hit.
                let line = source.line_of(location.start.0);
                let count = if row.num_cycles > 0 {
                    hits.op_hits.get(&(node_id, row.op_idx)).copied().unwrap_or(0)
                } else {
                    node_starts(node_id)
                };
                // Several rows share a line (`dup neq.0`) and each counts
                // every execution of it, so summing would multiply the count.
                let hit = file.lines.entry(line).or_insert(0);
                *hit = (*hit).max(count);
            }

            // Only splits are branches. The assembler wraps every
            // `while.true` in a split whose false arm skips the loop, so the
            // guard split already records entered/skipped and the loop node
            // itself must not add a second branch.
            let is_split = package
                .forest
                .get_node_by_id(node.exec_node)
                .is_some_and(|exec| matches!(exec, MastNode::Split(_)));
            // The anchor is the first located row: the split's own control
            // row (`if.true`, or `while.true` for a loop guard).
            if let (true, [then_arm, else_arm]) = (is_split, node.children.as_slice())
                && let Some(location) = node
                    .asm_ops
                    .iter()
                    .find_map(|row| row.location_idx.into_option())
                    .and_then(|idx| debug_info.get_location(idx))
                && let Some((file, source)) = files.get(location.uri.as_str())
            {
                file.branches.push(BranchCoverage {
                    line: source.line_of(location.start.0),
                    block: node_id,
                    executed: node_starts(node_id) > 0,
                    arms: [node_starts((*then_arm).into()), node_starts((*else_arm).into())],
                });
            }
        }

        for function in debug_info.functions() {
            let Some(stored_path) = debug_info
                .get_file(function.file_idx)
                .and_then(|file| debug_info.get_string(file.path_idx))
            else {
                continue;
            };
            let Some((file, _)) = files.get(&stored_path) else {
                continue;
            };
            let line = function.line.0 + 1;
            let count =
                function.source_node.into_option().map_or(0, |node| node_starts(node.into()));
            // The definition line is coverable and hit on every entry. A
            // procedure whose body is one basic block has no row there, and
            // LCOV consumers (genhtml) reject a function line without a line
            // record.
            let hit = file.lines.entry(line).or_insert(0);
            *hit = (*hit).max(count);
            file.functions.push(FunctionCoverage {
                name: debug_info.get_string(function.name_idx).as_deref().unwrap_or("").to_owned(),
                line,
                count,
            });
        }
    }

    let Files { resolver, mut files } = files;
    for file in files.values_mut() {
        dedup(
            &mut file.functions,
            |a, b| (a.line, &a.name).cmp(&(b.line, &b.name)),
            |kept, next| kept.count = kept.count.max(next.count),
        );
        dedup(
            &mut file.branches,
            |a, b| (a.line, a.block).cmp(&(b.line, b.block)),
            |kept, next| {
                kept.executed |= next.executed;
                for (kept, next) in kept.arms.iter_mut().zip(next.arms) {
                    *kept = (*kept).max(next);
                }
            },
        );
    }
    Report {
        files,
        unresolved: resolver.unresolved().map(str::to_owned).collect(),
    }
}

/// The report's files, created on first resolution of a stored path.
struct Files<'a> {
    resolver: &'a mut SourceResolver,
    files: BTreeMap<String, FileCoverage>,
}

impl Files<'_> {
    /// The coverage entry and text of the file `stored_path` resolves to.
    fn get(&mut self, stored_path: &str) -> Option<(&mut FileCoverage, Arc<SourceText>)> {
        let source = self.resolver.resolve(stored_path)?;
        let file =
            self.files
                .entry(source.path.display().to_string())
                .or_insert_with(|| FileCoverage {
                    path: source.path.clone(),
                    ..Default::default()
                });
        Some((file, source))
    }
}

/// Sorts `items` by `order` and merges the items it considers equal.
///
/// Duplicates arise when one source file is part of several packages, for
/// example a library linked into two programs. They are merged by maximum,
/// like rows on one line, so that LCOV consumers see each record once.
fn dedup<T>(items: &mut Vec<T>, order: impl Fn(&T, &T) -> Ordering, merge: impl Fn(&mut T, &T)) {
    items.sort_by(&order);
    items.dedup_by(|next, kept| {
        let same = order(kept, next).is_eq();
        if same {
            merge(kept, next);
        }
        same
    });
}
