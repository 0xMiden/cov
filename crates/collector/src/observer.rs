//! Per-cycle observation of a VM execution.

use alloc::{collections::BTreeMap, string::String, sync::Arc, vec::Vec};

use miden_core::{
    Word,
    mast::{MastForest, MastNode, MastNodeId},
};
use miden_mast_package::debug_info::PackageDebugInfo;
use miden_processor::{Continuation, ResumeContext};

use crate::{
    format::{PackageFile, RunFile, RunOutcome},
    hits::PackageHits,
};

/// Records hits for every forest an execution passes through.
///
/// Forests are identified by the address of their shared allocation, which is
/// stable because the processor and the host hand out clones of the same
/// `Arc`. The observer keeps a clone of each forest so the address cannot be
/// reused while it is recording.
#[derive(Default)]
pub struct Observer {
    forests: BTreeMap<usize, ForestEntry>,
    /// Nodes that have started and not yet finished, innermost last, as
    /// (forest key, source node id). Gives a starting node its parent.
    open: Vec<(usize, u32)>,
    cycles: u64,
}

/// What the observer knows about one forest.
struct ForestEntry {
    forest: Arc<MastForest>,
    commitment: Word,
    debug_info: Option<Arc<PackageDebugInfo>>,
    hits: PackageHits,
}

/// Everything an execution produced, ready to be written to the coverage
/// directory.
pub struct ObserverOutput {
    /// One entry per forest that carried debug info; forests without debug
    /// info cannot be attributed to source and are dropped.
    pub packages: Vec<PackageFile>,
    /// The run with hits for the same packages.
    pub run: RunFile,
}

impl Observer {
    /// Creates an observer with nothing recorded.
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of cycles observed so far.
    pub fn cycles(&self) -> u64 {
        self.cycles
    }

    /// Records the continuations the next step will execute.
    ///
    /// Must be called once before every step with the context that step will
    /// consume. The continuations scheduled for the next clock cycle are
    /// exactly the ones the step consumes, so nothing is counted twice.
    pub fn observe(&mut self, ctx: &ResumeContext) {
        // A step may cross a forest boundary mid-cycle: an `EnterForest`
        // continuation switches the forest every later continuation refers to.
        let mut forest = ctx.current_forest();
        let mut debug_info = ctx.debug_info();
        for (continuation, source_node) in ctx
            .continuation_stack()
            .iter_continuations_for_next_clock_with_source_node_ids()
        {
            match continuation {
                Continuation::EnterForest { forest: next, package_debug_info, .. } => {
                    forest = next;
                    debug_info = package_debug_info.clone();
                    self.return_to(forest_key(forest));
                },
                Continuation::StartNode(_) => {
                    if let Some(source_node) = source_node {
                        let child = u32::from(source_node);
                        let key = forest_key(forest);
                        let parent = self.parent_of(key, &debug_info, child);
                        let hits = &mut self.entry(forest, &debug_info).hits;
                        hits.record_node_start(child);
                        if let Some(parent) = parent {
                            hits.record_edge_start(parent, child);
                        }
                        // An external node resolves to another forest's root
                        // and never finishes in this one; it is left open and
                        // dropped when execution returns to this forest.
                        self.open.push((key, child));
                    }
                },
                Continuation::FinishJoin(_)
                | Continuation::FinishSplit(_)
                | Continuation::FinishCall(_)
                | Continuation::FinishDyn(_)
                | Continuation::FinishBasicBlock(_) => {
                    if let Some(source_node) = source_node {
                        self.close(forest_key(forest), source_node.into());
                    }
                },
                // A loop's finish continuation runs once per iteration and
                // rechecks the condition, so the loop stays open; it is
                // dropped when its parent finishes.
                Continuation::FinishLoop(_) => {},
                Continuation::ResumeBasicBlock { node_id, batch_index, op_idx_in_batch } => {
                    if let Some(source_node) = source_node {
                        let op_idx =
                            padded_op_index(forest, *node_id, *batch_index, *op_idx_in_batch);
                        self.entry(forest, &debug_info).hits.record_op(source_node.into(), op_idx);
                    }
                },
                _ => {},
            }
        }
        self.cycles += 1;
    }

    /// Finishes the run and returns the files to write.
    pub fn finish(self, name: String, outcome: RunOutcome) -> ObserverOutput {
        let mut packages = Vec::new();
        let mut run_packages = Vec::new();
        for entry in self.forests.into_values() {
            let Some(debug_info) = entry.debug_info else { continue };
            if entry.hits.is_empty() {
                continue;
            }
            packages.push(PackageFile {
                commitment: entry.commitment,
                forest: entry.forest,
                debug_info,
            });
            run_packages.push((entry.commitment, entry.hits));
        }
        ObserverOutput {
            packages,
            run: RunFile {
                name,
                outcome,
                cycles: self.cycles,
                packages: run_packages,
            },
        }
    }

    /// The innermost open node of forest `key` that lists `child` among its
    /// children, or the innermost open node of that forest when none does.
    ///
    /// The second case covers a node whose parent is not tracked, such as
    /// the children of a root entered from another forest. A stale loop
    /// that exited but is still open never lists its sibling as a child, so
    /// the first rule skips it.
    fn parent_of(
        &self,
        key: usize,
        debug_info: &Option<Arc<PackageDebugInfo>>,
        child: u32,
    ) -> Option<u32> {
        let debug_info = debug_info.as_ref()?;
        let same_forest = self.open.iter().rev().filter(|(k, _)| *k == key).map(|(_, n)| *n);
        let lists_child = |node: &u32| {
            debug_info
                .source_node((*node).into())
                .is_some_and(|n| n.children.iter().any(|c| u32::from(*c) == child))
        };
        same_forest.clone().find(lists_child).or_else(|| same_forest.clone().next())
    }

    /// Pops the open nodes above and including `node` of forest `key`.
    /// Leaves the stack alone when the node is not open.
    fn close(&mut self, key: usize, node: u32) {
        if let Some(pos) = self.open.iter().rposition(|entry| *entry == (key, node)) {
            self.open.truncate(pos);
        }
    }

    /// Pops everything that belongs to forests left when execution returns
    /// to forest `key`, including the external node that led out of it.
    fn return_to(&mut self, key: usize) {
        while let Some((k, node)) = self.open.last().copied() {
            if k != key {
                self.open.pop();
                continue;
            }
            let is_external = self
                .forests
                .get(&k)
                .zip(self.forests[&k].debug_info.as_ref())
                .and_then(|(entry, debug_info)| {
                    let source = debug_info.source_node(node.into())?;
                    entry.forest.get_node_by_id(source.exec_node)
                })
                .is_some_and(|exec| matches!(exec, MastNode::External(_)));
            if is_external {
                self.open.pop();
            }
            break;
        }
    }

    fn entry(
        &mut self,
        forest: &Arc<MastForest>,
        debug_info: &Option<Arc<PackageDebugInfo>>,
    ) -> &mut ForestEntry {
        let key = forest_key(forest);
        self.forests.entry(key).or_insert_with(|| ForestEntry {
            forest: forest.clone(),
            commitment: forest.commitment(),
            debug_info: debug_info.clone(),
            hits: PackageHits::default(),
        })
    }
}

/// The key a forest is tracked under: the address of its shared allocation.
fn forest_key(forest: &Arc<MastForest>) -> usize {
    Arc::as_ptr(forest) as usize
}

/// Converts a batch-relative operation position into the padded index within
/// the basic block, which is how debug-info rows are keyed.
fn padded_op_index(
    forest: &MastForest,
    node_id: MastNodeId,
    batch_index: usize,
    op_idx_in_batch: usize,
) -> u32 {
    let MastNode::Block(block) = &forest[node_id] else {
        return op_idx_in_batch as u32;
    };
    let offset: usize =
        block.op_batches()[..batch_index].iter().map(|batch| batch.ops().len()).sum();
    (offset + op_idx_in_batch) as u32
}
