//! Raw hit counts for one package.

use alloc::collections::BTreeMap;

use miden_core::serde::{
    ByteReader, ByteWriter, Deserializable, DeserializationError, Serializable,
};

/// Hit counts recorded against one package's debug-info tree.
///
/// Keys are debug-info identifiers: source node ids, and for operations the
/// padded operation index within the source node's basic block, which is the
/// index the debug-info rows are keyed by. Nothing here is resolved to source;
/// the reporter applies the coverable rule.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PackageHits {
    /// How many times each source node started, keyed by source node id.
    pub node_starts: BTreeMap<u32, u64>,
    /// How many times each operation executed, keyed by (source node id,
    /// padded operation index within the node's basic block).
    pub op_hits: BTreeMap<(u32, u32), u64>,
}

impl PackageHits {
    /// Returns `true` when nothing was recorded.
    pub fn is_empty(&self) -> bool {
        self.node_starts.is_empty() && self.op_hits.is_empty()
    }

    /// Adds every count of `other` into `self`.
    pub fn merge_from(&mut self, other: &Self) {
        for (node, count) in &other.node_starts {
            *self.node_starts.entry(*node).or_default() += count;
        }
        for (key, count) in &other.op_hits {
            *self.op_hits.entry(*key).or_default() += count;
        }
    }

    /// Records one start of `node`.
    #[inline]
    pub fn record_node_start(&mut self, node: u32) {
        *self.node_starts.entry(node).or_default() += 1;
    }

    /// Records one execution of operation `op_idx` of `node`.
    #[inline]
    pub fn record_op(&mut self, node: u32, op_idx: u32) {
        *self.op_hits.entry((node, op_idx)).or_default() += 1;
    }
}

impl Serializable for PackageHits {
    fn write_into<W: ByteWriter>(&self, target: &mut W) {
        target.write_usize(self.node_starts.len());
        for (node, count) in &self.node_starts {
            target.write_u32(*node);
            target.write_u64(*count);
        }
        target.write_usize(self.op_hits.len());
        for ((node, op_idx), count) in &self.op_hits {
            target.write_u32(*node);
            target.write_u32(*op_idx);
            target.write_u64(*count);
        }
    }
}

impl Deserializable for PackageHits {
    fn read_from<R: ByteReader>(source: &mut R) -> Result<Self, DeserializationError> {
        let mut hits = Self::default();
        let num_nodes = source.read_usize()?;
        for _ in 0..num_nodes {
            let node = source.read_u32()?;
            let count = source.read_u64()?;
            hits.node_starts.insert(node, count);
        }
        let num_ops = source.read_usize()?;
        for _ in 0..num_ops {
            let node = source.read_u32()?;
            let op_idx = source.read_u32()?;
            let count = source.read_u64()?;
            hits.op_hits.insert((node, op_idx), count);
        }
        Ok(hits)
    }
}
