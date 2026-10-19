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
                },
                Continuation::StartNode(_) => {
                    if let Some(source_node) = source_node {
                        self.entry(forest, &debug_info).hits.record_node_start(source_node.into());
                    }
                },
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

    fn entry(
        &mut self,
        forest: &Arc<MastForest>,
        debug_info: &Option<Arc<PackageDebugInfo>>,
    ) -> &mut ForestEntry {
        let key = Arc::as_ptr(forest) as usize;
        self.forests.entry(key).or_insert_with(|| ForestEntry {
            forest: forest.clone(),
            commitment: forest.commitment(),
            debug_info: debug_info.clone(),
            hits: PackageHits::default(),
        })
    }
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
