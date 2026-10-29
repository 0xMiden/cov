//! Line, function, and branch coverage per source file.

use std::{
    collections::{BTreeMap, BTreeSet},
    ops::ControlFlow,
    path::{Path, PathBuf},
    sync::Arc,
};

use miden_core::{
    Word,
    mast::{MastNode, MastNodeExt},
};
use miden_cov_collector::{PackageFile, PackageHits};
use miden_debug_types::Location;
use miden_mast_package::debug_info::{
    DebugSourceAsmOp, DebugSourceInlineCall, DebugSourceNode, DebugSourceNodeId,
};

use crate::{
    ReportFilter, Snapshot,
    source::{SourceResolver, SourceText, absolute_path},
};

/// Coverage of every resolved source file of a snapshot.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    /// Files keyed by the display form of their canonical resolved path.
    pub files: BTreeMap<String, FileCoverage>,
    /// Stored debug-info paths that no file was found for, by the label of
    /// each package that refers to them (see [`package_label`]).
    pub unresolved: BTreeMap<String, BTreeSet<String>>,
}

impl Report {
    /// One line per package with unresolved paths: how many source files
    /// were not found and the directory they have in common, which is the
    /// `OLD` prefix a `--path-prefix-map` needs.
    pub fn unresolved_summary(&self) -> Vec<String> {
        self.unresolved
            .iter()
            .map(|(package, paths)| match (paths.len(), common_dir(paths)) {
                (1, _) => format!("{package}: source file not found: {}", paths.first().unwrap()),
                (n, Some(dir)) => format!("{package}: {n} source files not found under {dir}"),
                (n, None) => {
                    format!(
                        "{package}: {n} source files not found, e.g. {}",
                        paths.first().unwrap()
                    )
                },
            })
            .collect()
    }
}

/// The longest directory, with a trailing `/`, that every one of `paths`
/// lies under, or `None` when they share none.
fn common_dir(paths: &BTreeSet<String>) -> Option<String> {
    // In sorted order the first and the last path differ the most.
    let (first, last) = (paths.first()?, paths.last()?);
    let shared = first.bytes().zip(last.bytes()).take_while(|(a, b)| a == b).count();
    // `/` is one byte in UTF-8, so the slice ends on a character boundary.
    let end = first.as_bytes()[..shared].iter().rposition(|byte| *byte == b'/')?;
    let dir = &first[..=end];
    (dir != "/" && !dir.is_empty()).then(|| dir.to_owned())
}

/// The label a package is listed under: the namespace its function names
/// share, such as `miden::core`, or the start of its commitment when they
/// share none.
///
/// Coverage directories do not record package names.
pub fn package_label(package: &PackageFile) -> String {
    let debug_info = &package.debug_info;
    let mut names = debug_info
        .functions()
        .iter()
        .filter_map(|function| debug_info.get_string(function.name_idx));
    let namespace = names.next().map(|first| {
        let mut shared: Vec<String> = segments(&first).map(str::to_owned).collect();
        // The last segment is the procedure's own name.
        shared.pop();
        for name in names {
            let common = shared.iter().zip(segments(&name)).take_while(|(a, b)| a == b).count();
            shared.truncate(common);
        }
        shared.join("::")
    });
    match namespace {
        // `$exec` is the namespace of every program.
        Some(namespace) if !namespace.is_empty() && !namespace.starts_with('$') => namespace,
        _ => format!("package {}", &package.commitment.to_hex()[..10]),
    }
}

/// The `::`-separated segments of a procedure path, without a leading `::`.
fn segments(name: &str) -> impl Iterator<Item = &str> {
    name.trim_start_matches("::").split("::")
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
    /// The instruction introducing the branch, or `None` when unknown, as
    /// for a branch read back from LCOV.
    pub kind: Option<BranchKind>,
}

/// The instruction a branch comes from, which gives its arms their meaning.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BranchKind {
    /// An `if.true`: the arms are the true and the false block.
    If,
    /// A `while.true`: the arms are the iterations and the exits.
    While,
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
        package: Arc::from(""),
        unresolved: BTreeMap::new(),
    };
    // Each row is one copy of an instruction: the assembler copies a
    // procedure's tree at every place it is inlined, and each copy keeps the
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
    let mut functions: BTreeMap<FunctionKey, FunctionTally> = BTreeMap::new();
    let no_hits = PackageHits::default();
    let views: Vec<(Word, PackageView<'_>)> = snapshot
        .packages
        .iter()
        .map(|(commitment, package)| {
            (
                *commitment,
                PackageView::new(package, snapshot.merged.get(commitment).unwrap_or(&no_hits)),
            )
        })
        .collect();

    // A `call` names its callee by the digest of the child's root, and the
    // same procedure is recorded in many packages of the directory: a library
    // linked into two programs, a library's table imported into a program
    // that `exec`s one of its procedures. Records that agree on the
    // function's file as the report resolves it, its line, and its name are
    // one function; a root that two different functions share, in one
    // package or across packages, cannot be credited.
    let mut owners: BTreeMap<Word, BTreeMap<FunctionKey, (Word, u32)>> = BTreeMap::new();
    for (commitment, package) in &views {
        files.package = package.label.clone();
        for (idx, function) in package.package.debug_info.functions().iter().enumerate() {
            if package.canonical[idx] != idx as u32 {
                continue;
            }
            let Some(key) = package.function_key(idx as u32, &mut files) else {
                continue;
            };
            owners
                .entry(function.mast_root)
                .or_default()
                .entry(key)
                .or_insert((*commitment, idx as u32));
        }
    }
    // Entries into a function through the `call` nodes of any package, keyed
    // by the callee's package and record.
    let mut call_entries: BTreeMap<(Word, u32), u64> = BTreeMap::new();

    for (commitment, package) in &views {
        files.package = package.label.clone();
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
                branch.kind = Some(split.kind);
                for (total, arm) in branch.arms.iter_mut().zip(split.arms) {
                    *total += arm;
                }
            }
        }

        for (child, digest, edges) in package.call_edges() {
            // Only visible records can make a root ambiguous: a library
            // without debug info is never written, and a record without a
            // file or a name has no identity, so a digest they share with a
            // reported function is credited to that function.
            let Some(callee) = owners.get(&digest) else { continue };
            let (1, Some((key, owner))) = (callee.len(), callee.first_key_value()) else {
                continue;
            };
            // The caller's own records of the function, when it has any, so
            // that its own root is recognized; otherwise the first package's.
            let mut own = package.records_with(key, digest, &mut files).peekable();
            let (callee_package, callee) = own.peek().map_or(*owner, |idx| (*commitment, *idx));
            // The function's own root, the child of a `call` outside any copy,
            // is counted by its starts. Any of the caller's records may carry
            // it: an imported record of the same function has none.
            let own_root = own.any(|idx| {
                debug_info.functions()[idx as usize].source_node.into_option() == Some(child)
            });
            // Crediting another package's record relies on the collector never
            // seeing the root start of a procedure entered through an external
            // node: the processor runs that start inside the step that resolves
            // the node. Were it described, the library's root starts and these
            // edges would both count the entry.
            if !own_root {
                *call_entries.entry((callee_package, callee)).or_default() += edges;
            }
        }
    }

    for (commitment, package) in &views {
        files.package = package.label.clone();
        let debug_info = &package.package.debug_info;
        // Entries into a function through the inlined copies of its own package.
        let inline_entries = package.inline_entries();
        for (function_idx, function) in debug_info.functions().iter().enumerate() {
            if package.canonical[function_idx] != function_idx as u32 {
                continue;
            }
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
            let names = &package.names[function_idx];
            let is_own = |row: &DebugSourceAsmOp| {
                debug_info
                    .get_string(row.context_name_idx)
                    .is_some_and(|context| names.contains(&context))
            };
            let root = function.source_node.into_option();
            let entry = root.and_then(|root| match package.entry_location(root, &is_own) {
                ControlFlow::Break(entry) => entry,
                ControlFlow::Continue(()) => None,
            });
            let name = debug_info.get_string(function.name_idx);
            let name = name.as_deref().unwrap_or("").to_owned();
            let tally = functions.entry((key, function.line.0 + 1, name)).or_default();
            tally.starts += root.map_or(0, |root| package.starts(root))
                + inline_entries.get(&(function_idx as u32)).copied().unwrap_or(0)
                + call_entries.get(&(*commitment, function_idx as u32)).copied().unwrap_or(0);
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
    for ((key, start, _), BranchTally { executed, arms, kind }) in branches {
        if let Some(file) = reported.get_mut(&key) {
            file.coverage.branches.push(BranchCoverage {
                line: file.source.line_of(start),
                block: start,
                executed,
                arms,
                kind,
            });
        }
    }
    for ((key, line, name), FunctionTally { entry, starts }) in functions {
        let Some(file) = reported.get_mut(&key) else {
            continue;
        };
        // Without an entry location, fall back to the starts of the root, the
        // entries of every copy, and the edges of every `call` to it. A body
        // of another package entered through `exec` is counted where the copy
        // or the external node that enters it begins, in the calling package.
        let count = entry.map_or(starts, |entry| entries.get(&entry).copied().unwrap_or(0));
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

/// The identity of a function across packages: its file as the report
/// resolves it, its line, and its name, the key the report tallies functions
/// under.
type FunctionKey = (Arc<str>, u32, String);

/// One package's forest and debug info with its merged hits.
struct PackageView<'a> {
    package: &'a PackageFile,
    hits: &'a PackageHits,
    /// For each function record, the index of the record that stands for it.
    ///
    /// The assembler records a re-exported procedure twice, differing only
    /// in the linkage name; the first record stands for both.
    canonical: Vec<u32>,
    /// For each canonical record, every name it and its re-exports go by;
    /// empty for the other records.
    names: Vec<Vec<Arc<str>>>,
    /// What the package is listed under, see [`package_label`].
    label: Arc<str>,
}

impl<'a> PackageView<'a> {
    /// Pairs a package with its hits and folds its duplicate records.
    fn new(package: &'a PackageFile, hits: &'a PackageHits) -> Self {
        let debug_info = &package.debug_info;
        let functions = debug_info.functions();
        let mut first = BTreeMap::new();
        let canonical: Vec<u32> = functions
            .iter()
            .enumerate()
            .map(|(idx, function)| {
                let record = (
                    function.mast_root,
                    function.name_idx,
                    function.file_idx,
                    function.line,
                    function.source_node.into_option(),
                );
                *first.entry(record).or_insert(idx as u32)
            })
            .collect();
        let mut names = vec![Vec::new(); functions.len()];
        for (function, canonical) in functions.iter().zip(&canonical) {
            names[*canonical as usize].extend(
                [Some(function.name_idx), function.linkage_name_idx.into_option()]
                    .into_iter()
                    .flatten()
                    .filter_map(|idx| debug_info.get_string(idx)),
            );
        }
        Self {
            package,
            hits,
            canonical,
            names,
            label: package_label(package).into(),
        }
    }

    /// The identity of the record `function`, if it names a file: the key the
    /// report tallies it under.
    fn function_key(&self, function: u32, files: &mut Files) -> Option<FunctionKey> {
        let debug_info = &self.package.debug_info;
        let record = &debug_info.functions()[function as usize];
        let path = debug_info
            .get_file(record.file_idx)
            .and_then(|file| debug_info.get_string(file.path_idx))?;
        let name = debug_info.get_string(record.name_idx)?;
        Some((files.key(&path), record.line.0 + 1, name.to_string()))
    }

    /// The canonical records of this package with the identity `key` and the
    /// MAST root `digest`, in table order.
    fn records_with<'f>(
        &'f self,
        key: &'f FunctionKey,
        digest: Word,
        files: &'f mut Files<'_>,
    ) -> impl Iterator<Item = u32> + 'f {
        let debug_info = &self.package.debug_info;
        (0..debug_info.functions().len() as u32).filter(move |idx| {
            self.canonical[*idx as usize] == *idx
                && debug_info.functions()[*idx as usize].mast_root == digest
                && self.function_key(*idx, files).as_ref() == Some(key)
        })
    }

    /// How many times source node `node` started.
    fn starts(&self, node: impl Into<u32>) -> u64 {
        self.hits.node_starts.get(&node.into()).copied().unwrap_or(0)
    }

    /// How many times `child` started as the child of `parent`.
    ///
    /// A procedure entered dynamically or from another package has one
    /// source tree however many places call it, and the `call`s of one
    /// procedure within one inlined copy of their caller share one tree, so
    /// a node's own starts cannot tell those sites apart; the edge recorded
    /// by the collector can.
    fn starts_from(&self, parent: u32, child: impl Into<u32>) -> u64 {
        self.hits.edge_starts.get(&(parent, child.into())).copied().unwrap_or(0)
    }

    /// How many times each function was entered through an inlined copy of
    /// its body, keyed by the index of the record standing for the function.
    ///
    /// The assembler emits a copy of a procedure's source tree at every
    /// `exec` of it and marks the copy's rows with an inline call naming the
    /// procedure, so the procedure's own root runs only through a `call`, as
    /// the entry point, or as a dynamic target. A copy is entered where its
    /// inline call begins: at a node whose parent does not carry the same
    /// call, or in the middle of a block. On a block it counts once per
    /// execution of the first operation of the call's range; on a control
    /// node, which has no operations, once per start of the node.
    fn inline_entries(&self) -> BTreeMap<u32, u64> {
        let debug_info = &self.package.debug_info;
        let nodes = debug_info.nodes();
        // A clone shared by several `call` nodes keeps the last of them as
        // its parent; they carry the same inherited inline calls, so the
        // choice does not matter for the entries.
        let mut parent = vec![None; nodes.len()];
        for (node_id, node) in nodes.iter().enumerate() {
            for child in &node.children {
                if let Some(slot) = parent.get_mut(u32::from(*child) as usize) {
                    *slot = Some(node_id as u32);
                }
            }
        }
        let mut entries = BTreeMap::new();
        for (node_id, node) in nodes.iter().enumerate() {
            if node.inline_calls.is_empty() {
                continue;
            }
            let is_block = matches!(self.exec(node), Some(MastNode::Block(_)));
            // A parent has children, so it is a control node and runs no
            // operations of its own. The assembler gives every inline call it
            // adds to a copied node the node's operation range, and the range
            // of a control node starts at 0, so the calls containing operation
            // 0 are all the calls the parent's copy carries: its frame.
            let inherited = |call: &DebugSourceInlineCall| {
                parent[node_id].is_some_and(|parent| {
                    debug_info.inline_calls_for_operation(parent.into(), 0).any(|above| {
                        (above.callee_idx, above.loc_idx) == (call.callee_idx, call.loc_idx)
                    })
                })
            };
            for call in &node.inline_calls {
                if call.op_idx == 0 && inherited(call) {
                    continue;
                }
                let count = if is_block {
                    self.hits.op_hits.get(&(node_id as u32, call.op_idx)).copied().unwrap_or(0)
                } else {
                    self.starts(node_id as u32)
                };
                // A package file is read with the wire-format checks only, not
                // the reference checks of a package, so the callee may lie
                // outside the function table; its entries are then keyed by an
                // index no record has and never read.
                let callee = self
                    .canonical
                    .get(u32::from(call.callee_idx) as usize)
                    .copied()
                    .unwrap_or(u32::from(call.callee_idx));
                *entries.entry(callee).or_default() += count;
            }
        }
        entries
    }

    /// The `call` nodes of the package: for each, its child, the digest of
    /// the child's exec node, and the edges from the node to the child.
    ///
    /// A `call` inside an inlined copy clones the callee's tree without an
    /// inline call naming the callee; the clone's root is the `call` node's
    /// child, shared by every `call` of the callee within the copy, so the
    /// edges, not the child's starts, are what one `call` contributes. The
    /// child of a `call` into a linked library is that library's root,
    /// whose start the collector never sees. The caller credits the edges to
    /// the one function whose MAST root the child is, in whichever package of
    /// the directory records it.
    fn call_edges(&self) -> impl Iterator<Item = (DebugSourceNodeId, Word, u64)> + '_ {
        let debug_info = &self.package.debug_info;
        debug_info.nodes().iter().enumerate().filter_map(move |(node_id, node)| {
            let (Some(MastNode::Call(_)), &[child]) = (self.exec(node), node.children.as_slice())
            else {
                return None;
            };
            let callee = self.exec(debug_info.source_node(child)?)?;
            Some((child, callee.digest(), self.starts_from(node_id as u32, child)))
        })
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
            let iterations = self.starts_from(then_arm.into(), body);
            return Some(Split {
                kind: BranchKind::While,
                arms: [iterations, entered],
                evaluations: iterations + entered,
            });
        }
        let arms = [self.starts_from(node_id, then_arm), self.starts_from(node_id, else_arm)];
        Some(Split {
            kind: BranchKind::If,
            arms,
            evaluations: entered,
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
        // and run as often as the call site, not as the callee's own body.
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
    /// The key of every stored path seen so far, and whether the path is
    /// unresolved and kept by the filter.
    keys: BTreeMap<String, (Arc<str>, bool)>,
    /// Files that resolve and are kept by the filter, by key.
    reported: BTreeMap<Arc<str>, ReportedFile>,
    /// The label of the package whose records are being read, which an
    /// unresolved path is listed under.
    package: Arc<str>,
    /// Stored paths that do not resolve and are kept by the filter, by
    /// package label.
    unresolved: BTreeMap<String, BTreeSet<String>>,
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
        let (key, unresolved) = match self.keys.get(stored_path) {
            Some(known) => known.clone(),
            None => {
                let known = self.resolve(stored_path);
                self.keys.insert(stored_path.to_owned(), known.clone());
                known
            },
        };
        // Every package that refers to the path lists it, not only the first.
        if unresolved
            && !self
                .unresolved
                .get(&*self.package)
                .is_some_and(|paths| paths.contains(stored_path))
        {
            self.unresolved
                .entry(self.package.to_string())
                .or_default()
                .insert(stored_path.to_owned());
        }
        key
    }

    /// The key of `stored_path` and whether it is an unresolved path the
    /// filter keeps, adding the file to the report when it resolves and the
    /// filter keeps it.
    fn resolve(&mut self, stored_path: &str) -> (Arc<str>, bool) {
        match self.resolver.resolve(stored_path) {
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
                (key, false)
            },
            None => (stored_path.into(), self.filter.keeps_unresolved(Path::new(stored_path))),
        }
    }

    /// The key of `location`.
    fn location_key(&mut self, location: &Location) -> LocationKey {
        (self.key(location.uri.as_str()), location.start.0, location.end.0)
    }
}

/// The condition counts of one split node.
struct Split {
    /// Whether the split is an `if.true` or guards a loop.
    kind: BranchKind,
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
    kind: Option<BranchKind>,
}

/// The copies of one function seen so far.
#[derive(Default)]
struct FunctionTally {
    /// The location whose entries count the function's runs, if any copy
    /// has one; it is the same for every copy.
    entry: Option<LocationKey>,
    /// Starts of the function's own root, entries of its inlined copies, and
    /// edges of the `call`s to it, summed over the packages of the snapshot.
    starts: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unresolved_paths_are_summarized_per_package() {
        let mut report = Report::default();
        let mut add = |package: &str, paths: &[&str]| {
            report
                .unresolved
                .insert(package.into(), paths.iter().map(|path| path.to_string()).collect());
        };
        add("miden::core", &["/ci/core/asm/sys/b.masm", "/ci/core/asm/a.masm"]);
        add("one", &["/ci/one.masm"]);
        // `/ci/core/asm/s` is a common string prefix but not a directory.
        add("siblings", &["/ci/core/asm/sa.masm", "/ci/core/asm/sb.masm"]);
        add("scattered", &["/a.masm", "/b/c.masm"]);
        add("relative", &["w.masm", "x/w.masm"]);
        assert_eq!(
            report.unresolved_summary(),
            [
                "miden::core: 2 source files not found under /ci/core/asm/",
                "one: source file not found: /ci/one.masm",
                "relative: 2 source files not found, e.g. w.masm",
                "scattered: 2 source files not found, e.g. /a.masm",
                "siblings: 2 source files not found under /ci/core/asm/",
            ]
        );
    }
}
