//! Line, function, and branch coverage per source file.

use std::{
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
    source::{SourceResolver, SourceText, absolute_path},
};

/// Coverage of every resolved source file of a snapshot.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    /// Files keyed by the display form of their canonical resolved path.
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
    /// Byte offset of the instruction introducing the branch, which tells
    /// apart the branches of one line.
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
    // How often the node holding each location was entered, summed the same
    // way. It differs from the location's count only for a loop guard's
    // control row, which counts every evaluation of the condition.
    let mut entries: BTreeMap<LocationKey, u64> = BTreeMap::new();
    // Branches by the location of the instruction introducing them. Copies
    // of one branch, from an unrolled `repeat` or from every package a source
    // file is linked into, add up.
    let mut branches: BTreeMap<LocationKey, BranchTally> = BTreeMap::new();
    // Functions by file, line, and name, each listed once per package that
    // contains it. The count is known only once every package is tallied,
    // because a body may run through another package's copy of it.
    let mut functions: BTreeMap<(Arc<str>, u32, String), FunctionTally> = BTreeMap::new();
    let no_hits = PackageHits::default();

    for (commitment, package) in &snapshot.packages {
        let package =
            PackageView::new(package, snapshot.merged.get(commitment).unwrap_or(&no_hits));
        let debug_info = &package.package.debug_info;

        for (node_id, node) in debug_info.nodes().iter().enumerate() {
            let node_id = node_id as u32;
            for (row, count, entered) in package.coverable_rows(node_id, node) {
                if let Some(location) = package.location(row) {
                    let key = files.location_key(&location);
                    *locations.entry(key.clone()).or_default() += count;
                    *entries.entry(key).or_default() += entered;
                }
            }

            // The anchor is the first located row: the split's own control
            // row (`if.true`, or `while.true` for a loop guard).
            if let Some(split) = package.split(node_id, node)
                && let Some(location) = node.asm_ops.iter().find_map(|row| package.location(row))
            {
                let branch = branches.entry(files.location_key(&location)).or_default();
                branch.executed |= package.starts(node_id) > 0;
                for (total, arm) in branch.arms.iter_mut().zip(split.arms) {
                    *total += arm;
                }
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
            let name = name.as_deref().unwrap_or("").to_owned();
            let tally = functions.entry((key, function.line.0 + 1, name)).or_default();
            tally.root_starts += root.map_or(0, |root| package.starts(root));
            if let Some(entry) = entry {
                tally.entry = Some(files.location_key(&entry));
            }
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
    for ((key, start, _), BranchTally { executed, arms }) in branches {
        if let Some(file) = reported.get_mut(&key) {
            file.coverage.branches.push(BranchCoverage {
                line: file.source.line_of(start),
                block: start,
                executed,
                arms,
            });
        }
    }
    for ((key, line, name), FunctionTally { entry, root_starts }) in functions {
        let Some(file) = reported.get_mut(&key) else {
            continue;
        };
        // Without an entry location, fall back to the root's starts, which
        // miss inlined copies and bodies entered from another package.
        let count = entry.map_or(root_starts, |entry| entries.get(&entry).copied().unwrap_or(0));
        // The definition line is coverable and hit on every entry. A
        // procedure whose body is one basic block has no row there, and LCOV
        // consumers (genhtml) reject a function line without a line record.
        let hit = file.coverage.lines.entry(line).or_insert(0);
        *hit = (*hit).max(count);
        file.coverage.functions.push(FunctionCoverage { name, line, count });
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
    /// The parents of every source node, once per edge.
    parents: Vec<Vec<u32>>,
}

impl<'a> PackageView<'a> {
    fn new(package: &'a PackageFile, hits: &'a PackageHits) -> Self {
        let nodes = package.debug_info.nodes();
        let mut parents = vec![Vec::new(); nodes.len()];
        for (parent, node) in nodes.iter().enumerate() {
            for &child in &node.children {
                if let Some(parents) = parents.get_mut(u32::from(child) as usize) {
                    parents.push(parent as u32);
                }
            }
        }
        Self { package, hits, parents }
    }

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

    /// The rows of `node` that count toward coverage, each with its count
    /// and the number of times `node` was entered.
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
    ) -> impl Iterator<Item = (&'a DebugSourceAsmOp, u64, u64)> {
        let is_block = matches!(self.exec(node), Some(MastNode::Block(_)));
        let entered = self.starts(node_id);
        let evaluations = self.split(node_id, node).map(|split| split.evaluations);
        let op_hits = &self.hits.op_hits;
        node.asm_ops.iter().filter_map(move |row| match (row.num_cycles, evaluations) {
            (1.., _) if is_block => {
                let hits = op_hits.get(&(node_id, row.op_idx)).copied().unwrap_or(0);
                Some((row, hits, hits))
            },
            (0, Some(evaluations)) => Some((row, evaluations, entered)),
            _ => None,
        })
    }

    /// The condition counts of split `node_id`, or `None` for other nodes.
    ///
    /// The assembler wraps every `while.true` in a split whose true arm is
    /// the loop and whose false arm is the block taken when the condition
    /// fails on entry. For such a guard the arms are the condition's
    /// outcomes: it held once per iteration, and failed once per entry,
    /// either skipping the loop or ending it.
    fn split(&self, node_id: u32, node: &DebugSourceNode) -> Option<Split> {
        let (Some(MastNode::Split(_)), &[then_arm, else_arm]) =
            (self.exec(node), node.children.as_slice())
        else {
            return None;
        };
        let entered = self.starts(node_id);
        let then_node = self.package.debug_info.source_node(then_arm)?;
        if let (Some(MastNode::Loop(_)), &[body]) =
            (self.exec(then_node), then_node.children.as_slice())
        {
            let iterations =
                self.starts_from(then_arm.into(), body).unwrap_or_else(|| self.starts(body));
            return Some(Split {
                arms: [iterations, entered],
                evaluations: iterations + entered,
            });
        }
        // One arm determines the other: every evaluation takes exactly one.
        let arms = match (self.starts_from(node_id, then_arm), self.starts_from(node_id, else_arm))
        {
            (Some(taken), Some(not_taken)) => [taken, not_taken],
            (Some(taken), None) => [taken, entered.saturating_sub(taken)],
            (None, Some(not_taken)) => [entered.saturating_sub(not_taken), not_taken],
            // Both arms are procedures also reached from other branches or
            // loops; their starts are an upper bound.
            (None, None) => {
                [self.starts(then_arm).min(entered), self.starts(else_arm).min(entered)]
            },
        };
        Some(Split { arms, evaluations: entered })
    }

    /// How many times `child` started as the child of `parent`, if the hits
    /// tell.
    ///
    /// A procedure has one source tree however many places call it, so the
    /// root of a procedure called from several places has several parents
    /// and its starts add up over them. A join or a call starts its children
    /// once per start of its own; subtracting those leaves the starts from
    /// `parent` when no other parent is a branch or a loop.
    fn starts_from(&self, parent: u32, child: DebugSourceNodeId) -> Option<u64> {
        let mut edges = 0;
        let mut from_others = 0;
        for &other in self.parents.get(u32::from(child) as usize)? {
            if other == parent {
                edges += 1;
                continue;
            }
            let other_node = self.package.debug_info.source_node(other.into())?;
            match self.exec(other_node)? {
                MastNode::Join(_) | MastNode::Call(_) => from_others += self.starts(other),
                _ => return None,
            }
        }
        // A split whose arms are one node cannot tell them apart.
        (edges == 1).then(|| self.starts(child).saturating_sub(from_others))
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
            .filter(|(row, ..)| is_own(row))
            .filter_map(|(row, ..)| self.location(row));
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
            // A split is entered once per entry of the function, but its
            // arms run conditionally; loop bodies run once per iteration.
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
    /// canonical resolved path, or the stored path itself when it does not
    /// resolve.
    ///
    /// Several stored paths may resolve to one file, for example when two
    /// packages were built on different machines or from different
    /// directories; they share a key.
    fn key(&mut self, stored_path: &str) -> Arc<str> {
        if let Some(key) = self.keys.get(stored_path) {
            return key.clone();
        }
        let key: Arc<str> = match self.resolver.resolve(stored_path) {
            Some(source) => {
                let key: Arc<str> = absolute_path(&source.path).display().to_string().into();
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
}

/// The condition counts of one split node.
struct Split {
    /// Times each arm was taken: `[true arm, false arm]`.
    arms: [u64; 2],
    /// Times the condition was evaluated.
    evaluations: u64,
}

/// The copies of one branch seen so far.
#[derive(Default)]
struct BranchTally {
    executed: bool,
    arms: [u64; 2],
}

/// The copies of one function seen so far.
#[derive(Default)]
struct FunctionTally {
    /// The location whose entries count the function's runs, if any copy
    /// has one; it is the same for every copy.
    entry: Option<LocationKey>,
    /// Starts of the function's root, summed over its copies.
    root_starts: u64,
}
