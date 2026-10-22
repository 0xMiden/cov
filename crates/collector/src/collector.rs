//! The coverage instrument: counts the cycle records the debugger engine
//! produces.

use alloc::{collections::BTreeMap, string::String, sync::Arc, vec::Vec};

use miden_core::{Word, mast::MastForest};
use miden_debug_engine::CycleInfo;
use miden_mast_package::debug_info::PackageDebugInfo;

use crate::{
    format::{PackageFile, RunFile, RunOutcome},
    hits::PackageHits,
};

/// Records hits for every forest an execution passes through.
///
/// Forests are identified by the address of their shared allocation, which is
/// stable because the processor and the host hand out clones of the same
/// `Arc`. The collector keeps a clone of each forest so the address cannot be
/// reused while it is recording.
#[derive(Default)]
pub struct Collector {
    /// The forests seen so far, keyed by the address of their allocation.
    forests: BTreeMap<usize, ForestEntry>,
    /// Number of records received: one per attempted cycle, the failing one
    /// included.
    cycles: u64,
}

/// What the collector knows about one forest.
struct ForestEntry {
    /// The forest itself, kept so its address stays taken.
    forest: Arc<MastForest>,
    /// The forest's commitment, which names the package in the run file.
    commitment: Word,
    /// The debug info of the forest's package, if it carried any.
    debug_info: Option<Arc<PackageDebugInfo>>,
    /// The hits recorded against the package's debug info.
    hits: PackageHits,
}

/// Everything an execution produced, ready to be written to the coverage
/// directory.
pub struct CollectorOutput {
    /// One entry per forest that carried debug info; forests without debug
    /// info cannot be attributed to source and are dropped.
    pub packages: Vec<PackageFile>,
    /// The run with hits for the same packages.
    pub run: RunFile,
}

impl Collector {
    /// Creates a collector with nothing recorded.
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of cycles recorded so far; a cycle that fails is recorded
    /// before it runs, so it counts.
    pub fn cycles(&self) -> u64 {
        self.cycles
    }

    /// Counts one cycle.
    ///
    /// Must be called once per step with the record the engine produced for
    /// it. A node start counts for the node and for the edge from its parent;
    /// an operation counts for its row key, the source node and padded index.
    /// A record whose source node the engine inferred, because the processor
    /// tracked none, counts as a cycle but hits nothing, except on the first
    /// cycle of an execution.
    pub fn on_cycle(&mut self, cycle: &CycleInfo<'_>) {
        // The processor tracks no source node for an invocation whose entry
        // root it could not pick, and the engine then guesses the unique root
        // of each node: a block that is also another procedure's root would
        // be credited to that procedure although only the arm ran. Coverage
        // may be lost there, never invented. The first cycle of an execution
        // started from a bare program is inferred too, and exactly: the
        // processor adopts that root when it installs the debug info.
        if cycle.source_node_inferred && self.cycles > 0 {
            self.cycles += 1;
            return;
        }
        if let Some(start) = cycle.node_start {
            let hits = &mut self.entry(cycle.forest, cycle.debug_info).hits;
            hits.record_node_start(start.node.into());
            if let Some(parent) = start.parent {
                hits.record_edge_start(parent.into(), start.node.into());
            }
        }
        // A `RESPAN` cycle carries the index of the next batch's first
        // operation but executes none; only a cycle with an operation is a hit.
        if cycle.op.is_some()
            && let (Some(node), Some(op_idx)) = (cycle.source_node, cycle.op_idx)
        {
            self.entry(cycle.forest, cycle.debug_info).hits.record_op(node.into(), op_idx);
        }
        self.cycles += 1;
    }

    /// Finishes the run and returns the files to write.
    pub fn finish(self, name: String, outcome: RunOutcome) -> CollectorOutput {
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
        CollectorOutput {
            packages,
            run: RunFile {
                name,
                outcome,
                cycles: self.cycles,
                packages: run_packages,
            },
        }
    }

    /// The entry of `forest`, created with `debug_info` on first sight.
    fn entry(
        &mut self,
        forest: &Arc<MastForest>,
        debug_info: Option<&Arc<PackageDebugInfo>>,
    ) -> &mut ForestEntry {
        let key = Arc::as_ptr(forest) as usize;
        self.forests.entry(key).or_insert_with(|| ForestEntry {
            forest: forest.clone(),
            commitment: forest.commitment(),
            debug_info: debug_info.cloned(),
            hits: PackageHits::default(),
        })
    }
}
