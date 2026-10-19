//! Line, function, and branch coverage per source file.

use std::{
    cmp::Ordering,
    collections::{BTreeMap, BTreeSet},
    ops::ControlFlow,
    path::{Path, PathBuf},
    sync::Arc,
};

use miden_core::mast::MastNode;
use miden_cov_collector::{PackageFile, PackageHits};
use miden_debug_types::Location;
use miden_mast_package::debug_info::{DebugSourceAsmOp, DebugSourceNode, DebugSourceNodeId};

use crate::{
    ReportFilter, Snapshot,
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
    /// Number of times the function ran.
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
    /// Times each arm was taken: `[true arm, false arm]`. For a loop guard
    /// these are the times the condition held and failed.
    pub arms: [u64; 2],
}

/// A source location: the key of its file (see [`Files::key`]) and its byte
/// range.
type LocationKey = (Arc<str>, u32, u32);

/// Applies the coverable rule to a snapshot.
///
/// Every row, function, and branch whose file resolves and is kept by
/// `filter` contributes to the report, including those never hit; paths that
/// do not resolve are listed in [`Report::unresolved`] unless `filter` drops
/// them.
pub fn build_report(
    snapshot: &Snapshot,
    resolver: &mut SourceResolver,
    filter: &ReportFilter,
) -> Report {
    let mut files = Files {
        resolver,
        filter,
        keys: BTreeMap::new(),
        reported: BTreeMap::new(),
        unresolved: BTreeSet::new(),
    };
    // Each row is one copy of an instruction: the assembler inlines a
    // single-block procedure at every call site, and each copy keeps the
    // callee's location. A location's count is therefore the sum over its
    // rows, across all source nodes of all packages.
    let mut locations: BTreeMap<LocationKey, u64> = BTreeMap::new();
    // Functions with the location their count is read from. The count is
    // known only once every package is tallied, because a body may run
    // through another package's copy of it.
    let mut functions: Vec<(Arc<str>, FunctionCoverage, Option<LocationKey>)> = Vec::new();
    let no_hits = PackageHits::default();

    for (commitment, package) in &snapshot.packages {
        let package = PackageView {
            package,
            hits: snapshot.merged.get(commitment).unwrap_or(&no_hits),
        };
        let debug_info = &package.package.debug_info;

        for (node_id, node) in debug_info.nodes().iter().enumerate() {
            let node_id = node_id as u32;
            for (row, count) in package.coverable_rows(node_id, node) {
                if let Some(location) = package.location(row) {
                    *locations.entry(files.location_key(&location)).or_default() += count;
                }
            }

            // The anchor is the first located row: the split's own control
            // row (`if.true`, or `while.true` for a loop guard).
            if let Some(arms) = package.branch_arms(node)
                && let Some(location) = node.asm_ops.iter().find_map(|row| package.location(row))
                && let Some(file) = files.get(location.uri.as_str())
            {
                file.coverage.branches.push(BranchCoverage {
                    line: file.source.line_of(location.start.0),
                    block: node_id,
                    executed: package.starts(node_id) > 0,
                    arms,
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
            let key = files.key(&stored_path);
            if !files.reported.contains_key(&key) {
                continue;
            }
            let name = debug_info.get_string(function.name_idx);
            let linkage_name = function
                .linkage_name_idx
                .into_option()
                .and_then(|idx| debug_info.get_string(idx));
            let is_own = |row: &DebugSourceAsmOp| {
                let context = debug_info.get_string(row.context_name_idx);
                context.is_some() && (context == name || context == linkage_name)
            };
            let root = function.source_node.into_option();
            let entry = root.and_then(|root| match package.entry_location(root, &is_own) {
                ControlFlow::Break(entry) => entry,
                ControlFlow::Continue(()) => None,
            });
            functions.push((
                key,
                FunctionCoverage {
                    name: name.as_deref().unwrap_or("").to_owned(),
                    line: function.line.0 + 1,
                    // Without an entry location, fall back to the root's
                    // starts, which miss inlined copies and bodies entered
                    // from another package.
                    count: root.map_or(0, |root| package.starts(root)),
                },
                entry.map(|location| files.location_key(&location)),
            ));
        }
    }

    let Files { mut reported, unresolved, .. } = files;
    // Control rows such as `if.true` span their whole block; only the line a
    // location starts on is credited, otherwise every line of the block would
    // look hit. A line holding several instructions takes the maximum.
    for ((key, start, _), count) in &locations {
        if let Some(file) = reported.get_mut(key) {
            let hit = file.coverage.lines.entry(file.source.line_of(*start)).or_insert(0);
            *hit = (*hit).max(*count);
        }
    }
    for (key, mut function, entry) in functions {
        let Some(file) = reported.get_mut(&key) else {
            continue;
        };
        if let Some(entry) = entry {
            function.count = locations.get(&entry).copied().unwrap_or(0);
        }
        // The definition line is coverable and hit on every entry. A
        // procedure whose body is one basic block has no row there, and LCOV
        // consumers (genhtml) reject a function line without a line record.
        let hit = file.coverage.lines.entry(function.line).or_insert(0);
        *hit = (*hit).max(function.count);
        file.coverage.functions.push(function);
    }

    for file in reported.values_mut() {
        let file = &mut file.coverage;
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
        files: reported
            .into_iter()
            .map(|(key, file)| (key.to_string(), file.coverage))
            .collect(),
        unresolved,
    }
}

/// One package's forest and debug info with its merged hits.
struct PackageView<'a> {
    package: &'a PackageFile,
    hits: &'a PackageHits,
}

impl<'a> PackageView<'a> {
    /// How many times source node `node` started.
    fn starts(&self, node: impl Into<u32>) -> u64 {
        self.hits.node_starts.get(&node.into()).copied().unwrap_or(0)
    }

    /// The exec node behind a source node.
    fn exec(&self, node: &DebugSourceNode) -> Option<&'a MastNode> {
        self.package.forest.get_node_by_id(node.exec_node)
    }

    /// The source location of `row`, if it has one.
    fn location(&self, row: &DebugSourceAsmOp) -> Option<Location> {
        let debug_info = &self.package.debug_info;
        row.location_idx.into_option().and_then(|idx| debug_info.get_location(idx))
    }

    /// The rows of `node` that count toward coverage, each with its count.
    ///
    /// Rows with operations count on blocks, as often as their first
    /// operation ran. Rows without operations count only on splits, as the
    /// number of times the condition was evaluated. Zero-operation rows on
    /// joins and loops are structural duplicates: every join of a procedure
    /// repeats its `begin`, and a loop repeats the `while.true` of its guard.
    fn coverable_rows(
        &self,
        node_id: u32,
        node: &'a DebugSourceNode,
    ) -> impl Iterator<Item = (&'a DebugSourceAsmOp, u64)> {
        let is_block = matches!(self.exec(node), Some(MastNode::Block(_)));
        let evaluations = self.branch_arms(node).map(|[taken, not_taken]| taken + not_taken);
        let op_hits = &self.hits.op_hits;
        node.asm_ops.iter().filter_map(move |row| match (row.num_cycles, evaluations) {
            (1.., _) if is_block => {
                Some((row, op_hits.get(&(node_id, row.op_idx)).copied().unwrap_or(0)))
            },
            (0, Some(evaluations)) => Some((row, evaluations)),
            _ => None,
        })
    }

    /// Times each arm of a split was taken, or `None` for other nodes.
    ///
    /// The assembler wraps every `while.true` in a split whose true arm is
    /// the loop and whose false arm is the block taken when the condition
    /// fails on entry. For such a guard the arms are the condition's
    /// outcomes: it held once per iteration, and failed once per skip and
    /// once per loop exit.
    fn branch_arms(&self, node: &DebugSourceNode) -> Option<[u64; 2]> {
        let (Some(MastNode::Split(_)), &[then_arm, else_arm]) =
            (self.exec(node), node.children.as_slice())
        else {
            return None;
        };
        let then_node = self.package.debug_info.source_node(then_arm)?;
        Some(match (self.exec(then_node), then_node.children.as_slice()) {
            (Some(MastNode::Loop(_)), &[body]) => {
                [self.starts(body), self.starts(else_arm) + self.starts(then_arm)]
            },
            _ => [self.starts(then_arm), self.starts(else_arm)],
        })
    }

    /// The location whose count is the number of times a function ran: the
    /// first coverable row of its own (`is_own`) that runs on every entry,
    /// searched from `node_id`, the function's root.
    ///
    /// Breaks with the location, or with `None` once the rest of the body
    /// may run a different number of times than the function was entered.
    fn entry_location(
        &self,
        node_id: DebugSourceNodeId,
        is_own: &impl Fn(&DebugSourceAsmOp) -> bool,
    ) -> ControlFlow<Option<Location>> {
        let Some(node) = self.package.debug_info.source_node(node_id) else {
            return ControlFlow::Break(None);
        };
        // Rows of a procedure inlined into the body keep the callee's name
        // and run as often as the call site, not as the callee's own body;
        // the root of an inlined procedure is the block of its call site.
        let mut own_rows = self
            .coverable_rows(node_id.into(), node)
            .filter(|(row, _)| is_own(row))
            .filter_map(|(row, _)| self.location(row));
        match self.exec(node) {
            // Both children of a join run once per entry of the join.
            Some(MastNode::Join(_)) => {
                for child in &node.children {
                    self.entry_location(*child, is_own)?;
                }
                ControlFlow::Continue(())
            },
            Some(MastNode::Block(_)) => match own_rows.next() {
                Some(location) => ControlFlow::Break(Some(location)),
                None => ControlFlow::Continue(()),
            },
            // A split's own row counts its evaluations, but its arms run
            // conditionally; loop bodies run once per iteration.
            Some(MastNode::Split(_)) => ControlFlow::Break(own_rows.next()),
            _ => ControlFlow::Break(None),
        }
    }
}

/// The source files of a report, created on first resolution of a stored
/// path.
struct Files<'a> {
    resolver: &'a mut SourceResolver,
    filter: &'a ReportFilter,
    /// The key of every stored path seen so far.
    keys: BTreeMap<String, Arc<str>>,
    /// Files that resolve and are kept by the filter, by key.
    reported: BTreeMap<Arc<str>, ReportedFile>,
    /// Stored paths that do not resolve and are kept by the filter.
    unresolved: BTreeSet<String>,
}

/// A file of the report with its text.
struct ReportedFile {
    coverage: FileCoverage,
    source: Arc<SourceText>,
}

impl Files<'_> {
    /// The key that `stored_path` is tallied under: the display form of the
    /// resolved path, or the stored path itself when it does not resolve.
    ///
    /// Several stored paths may resolve to one file, for example when two
    /// packages were built on different machines; they share a key.
    fn key(&mut self, stored_path: &str) -> Arc<str> {
        if let Some(key) = self.keys.get(stored_path) {
            return key.clone();
        }
        let key: Arc<str> = match self.resolver.resolve(stored_path) {
            Some(source) => {
                let key: Arc<str> = source.path.display().to_string().into();
                if self.filter.keeps(&source.path) {
                    self.reported.entry(key.clone()).or_insert_with(|| ReportedFile {
                        coverage: FileCoverage {
                            path: source.path.clone(),
                            ..Default::default()
                        },
                        source,
                    });
                }
                key
            },
            None => {
                if self.filter.keeps(Path::new(stored_path)) {
                    self.unresolved.insert(stored_path.to_owned());
                }
                stored_path.into()
            },
        };
        self.keys.insert(stored_path.to_owned(), key.clone());
        key
    }

    /// The key of `location`.
    fn location_key(&mut self, location: &Location) -> LocationKey {
        (self.key(location.uri.as_str()), location.start.0, location.end.0)
    }

    /// The report entry of the file `stored_path` resolves to, if the file
    /// is reported.
    fn get(&mut self, stored_path: &str) -> Option<&mut ReportedFile> {
        let key = self.key(stored_path);
        self.reported.get_mut(&key)
    }
}

/// Sorts `items` by `order` and merges the items it considers equal.
///
/// Duplicates arise when one source file is part of several packages, for
/// example a library linked into two programs. They are merged by maximum so
/// that LCOV consumers see each record once.
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
