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
/// One collector serves one execution, from its first cycle to its end.
/// Forests are identified by the address of their shared allocation, which is
/// stable because the processor keeps the forest it loaded for a procedure for
/// the rest of the execution and hands out clones of that `Arc`. The collector
/// keeps a clone of each forest with debug info so the address cannot be
/// reused while it is recording; a forest without debug info cannot be
/// attributed and is not kept.
#[derive(Default)]
pub struct Collector {
    /// The forests with debug info seen so far, keyed by the address of their
    /// allocation.
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
    /// The debug info of the forest's package.
    debug_info: Arc<PackageDebugInfo>,
    /// The hits recorded against the package's debug info.
    hits: PackageHits,
}

/// Everything an execution produced, ready to be written to the coverage
/// directory.
pub struct CollectorOutput {
    /// One entry per forest with debug info the execution passed through, hit
    /// or not; a forest without debug info cannot be attributed to source and
    /// is not kept.
    pub packages: Vec<PackageFile>,
    /// The run with hits for the same packages.
    pub run: RunFile,
}

impl Collector {
    /// Creates a collector with nothing recorded.
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of cycles recorded so far: a cycle that fails is recorded before
    /// it runs, so it counts, and so does a cycle that hit nothing.
    pub fn cycles(&self) -> u64 {
        self.cycles
    }

    /// Counts one cycle.
    ///
    /// Must be called once per step with the record the engine produced for
    /// it. A node start counts for the node and for the edge from its parent;
    /// an operation counts for its row key, the source node and padded index.
    /// A record whose source node the engine inferred, because the processor
    /// tracked none, counts as a cycle and marks its package as touched but
    /// hits nothing, except on the first cycle of an execution. A package
    /// whose root's first cycle fails inside the step that enters it is never
    /// recorded, since no record of its forest is ever produced.
    pub fn on_cycle(&mut self, cycle: &CycleInfo<'_>) {
        self.cycles += 1;
        let first = self.cycles == 1;
        // A forest without debug info cannot be attributed and is not kept;
        // the cycle still counts.
        let Some(debug_info) = cycle.debug_info else { return };
        // A package is reported from the first record in its forest, hit or
        // not, so that what ran without being attributed shows as uncovered
        // rather than disappearing.
        let hits = &mut self.entry(cycle.forest, debug_info).hits;
        // The processor tracks no source node for an invocation whose entry
        // root it could not pick (and, when that invocation `call`s a
        // procedure, none for the callee either), and the engine then guesses
        // the unique root of each node: a block that is also another
        // procedure's root would be credited to that procedure although only
        // the arm ran. A guessed source node is never a hit: coverage may be
        // lost there, never invented. The first record of an execution is
        // exempt: it is inferred only for an execution started from a bare
        // program, whose debug info the processor installs on that step,
        // adopting the same root; a package-started first record is tracked.
        if cycle.source_node_inferred && !first {
            return;
        }
        if let Some(start) = cycle.node_start {
            hits.record_node_start(start.node.into());
            // The parent of a same-forest dynamic callee's root may itself be
            // a guess, which the record does not say; the reporter reads no
            // edge from a dynamic node, so the count stands (0021).
            if let Some(parent) = start.parent {
                hits.record_edge_start(parent.into(), start.node.into());
            }
        }
        // A `RESPAN` cycle carries the index of the next batch's first
        // operation but executes none; only a cycle with an operation is a hit.
        if cycle.op.is_some()
            && let (Some(node), Some(op_idx)) = (cycle.source_node, cycle.op_idx)
        {
            hits.record_op(node.into(), op_idx);
        }
    }

    /// Finishes the run and returns the files to write.
    pub fn finish(self, name: String, outcome: RunOutcome) -> CollectorOutput {
        let mut packages = Vec::new();
        let mut run_packages = Vec::new();
        for entry in self.forests.into_values() {
            packages.push(PackageFile {
                commitment: entry.commitment,
                forest: entry.forest,
                debug_info: entry.debug_info,
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
        debug_info: &Arc<PackageDebugInfo>,
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
