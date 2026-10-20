use alloy_primitives::{B256, Bytes, keccak256, map::B256Map};
use alloy_rlp::Decodable;
use alloy_trie::{EMPTY_ROOT_HASH, Nibbles, nodes::TrieNode};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};

const MAX_KEY_NIBBLES: usize = 64;
const MAX_SNAPSHOT_NODES: usize = 1_000_000;
const KECCAK_RATE_BYTES: u64 = 136;

/// Counts MPT nodes by their structural kind.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MptNodeKindCounts {
    pub null: u64,
    pub branch: u64,
    pub extension: u64,
    pub leaf: u64,
}

impl MptNodeKindCounts {
    const fn increment(&mut self, kind: MptNodeKind) {
        match kind {
            MptNodeKind::Branch => self.branch += 1,
            MptNodeKind::Extension => self.extension += 1,
            MptNodeKind::Leaf => self.leaf += 1,
        }
    }

    pub(crate) const fn add_assign(&mut self, other: Self) {
        self.null += other.null;
        self.branch += other.branch;
        self.extension += other.extension;
        self.leaf += other.leaf;
    }
}

/// Aggregate path cost for trie reads.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MptReadMetrics {
    pub operations: u64,
    pub node_visits: u64,
    pub visited_nodes: MptNodeKindCounts,
}

/// Aggregate structural cost for one mutation class.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MptMutationMetrics {
    pub operations: u64,
    pub successful_operations: u64,
    pub pre_mutation_node_visits: u64,
    pub pre_mutation_visited_nodes: MptNodeKindCounts,
    pub created_nodes: MptNodeKindCounts,
    pub removed_nodes: MptNodeKindCounts,
    pub created_encoded_bytes: u64,
    pub removed_encoded_bytes: u64,
    /// Sum of post-mutation nodes whose encoding changed after each operation.
    /// A node changed by multiple operations is deliberately counted multiple times.
    pub mutation_dirty_encoded_nodes: u64,
    pub mutation_dirty_encoded_bytes: u64,
}

/// Actual dirty encoding and Keccak work observed when a trie hash is requested.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MptHashMetrics {
    pub operations: u64,
    pub dirty_encoded_nodes: u64,
    pub dirty_encoded_bytes: u64,
    pub dirty_hashed_nodes: u64,
    pub hash_input_bytes: u64,
    pub hash_input_blocks: u64,
}

/// Keccak work required to hash witness RLP nodes into the hydration map.
/// This does not include the later trie decode or digest-resolution work.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MptHydrationMetrics {
    pub hashed_rlp_nodes: u64,
    pub hash_input_bytes: u64,
    pub hash_input_blocks: u64,
}

/// Decode and digest-resolution work performed while materializing a sparse trie.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MptMaterializationMetrics {
    pub operations: u64,
    pub resolved_hashed_rlp_nodes: u64,
    pub resolved_hashed_rlp_bytes: u64,
    pub decoded_nodes: MptNodeKindCounts,
    pub resolved_digest_lookups: u64,
    pub unresolved_digest_lookups: u64,
}

/// Host-side MPT workload counters used by the zkGas calibration path.
///
/// This calibration implementation retains one decoded snapshot per materialized trie. Reads walk
/// that snapshot in `O(path depth)`; each successful mutation exports and rebuilds it once, so the
/// observer overhead is `O(mutations * materialized_nodes)`. The node bound is checked before a
/// mutation exports the trie, initial materialization clones only reachable witness RLPs, and path
/// walks fail closed above the 64-nibble Ethereum key bound. This is still host calibration code;
/// later proposer integration should collect the same counters directly from trie cursors.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MptWorkloadMetrics {
    pub witness_hydration: MptHydrationMetrics,
    pub state_materializations: MptMaterializationMetrics,
    pub storage_materializations: MptMaterializationMetrics,
    pub account_reads: MptReadMetrics,
    pub storage_reads: MptReadMetrics,
    pub account_inserts: MptMutationMetrics,
    pub account_updates: MptMutationMetrics,
    pub account_deletes: MptMutationMetrics,
    pub storage_inserts: MptMutationMetrics,
    pub storage_updates: MptMutationMetrics,
    pub storage_deletes: MptMutationMetrics,
    pub account_hashes: MptHashMetrics,
    pub storage_hashes: MptHashMetrics,
    pub storage_wipe_operations: u64,
    pub storage_wipe_dropped_nodes: MptNodeKindCounts,
    pub storage_wipe_dropped_encoded_bytes: u64,
    pub materialized_storage_drop_operations: u64,
    pub materialized_storage_drop_nodes: MptNodeKindCounts,
    pub materialized_storage_drop_encoded_bytes: u64,
}

impl MptHydrationMetrics {
    pub(crate) const fn record_rlp(&mut self, len: usize) {
        let len = len as u64;
        self.hashed_rlp_nodes += 1;
        self.hash_input_bytes += len;
        self.hash_input_blocks += len / KECCAK_RATE_BYTES + 1;
    }
}

impl MptMaterializationMetrics {
    pub(crate) const fn add_assign(&mut self, other: Self) {
        self.operations += other.operations;
        self.resolved_hashed_rlp_nodes += other.resolved_hashed_rlp_nodes;
        self.resolved_hashed_rlp_bytes += other.resolved_hashed_rlp_bytes;
        self.decoded_nodes.add_assign(other.decoded_nodes);
        self.resolved_digest_lookups += other.resolved_digest_lookups;
        self.unresolved_digest_lookups += other.unresolved_digest_lookups;
    }

    const fn record_resolved_rlp(&mut self, len: usize) {
        self.resolved_hashed_rlp_nodes += 1;
        self.resolved_hashed_rlp_bytes += len as u64;
        self.resolved_digest_lookups += 1;
    }
}

impl MptReadMetrics {
    pub(crate) const fn record(&mut self, observation: &PathObservation) {
        self.operations += 1;
        self.node_visits += observation.node_visits;
        self.visited_nodes.add_assign(observation.visited_nodes);
    }
}

impl MptMutationMetrics {
    pub(crate) fn record(&mut self, observation: &MutationObservation) {
        self.operations += 1;
        self.successful_operations += u64::from(observation.succeeded);
        self.pre_mutation_node_visits += observation.path.node_visits;
        self.pre_mutation_visited_nodes
            .add_assign(observation.path.visited_nodes);
        self.created_nodes
            .add_assign(observation.delta.created_nodes);
        self.removed_nodes
            .add_assign(observation.delta.removed_nodes);
        self.created_encoded_bytes += observation.delta.created_encoded_bytes;
        self.removed_encoded_bytes += observation.delta.removed_encoded_bytes;
        self.mutation_dirty_encoded_nodes += observation.delta.dirty_encoded_nodes;
        self.mutation_dirty_encoded_bytes += observation.delta.dirty_encoded_bytes;
    }
}

impl MptHashMetrics {
    pub(crate) const fn record(&mut self, observation: HashObservation) {
        self.operations += 1;
        self.dirty_encoded_nodes += observation.dirty_encoded_nodes;
        self.dirty_encoded_bytes += observation.dirty_encoded_bytes;
        self.dirty_hashed_nodes += observation.dirty_hashed_nodes;
        self.hash_input_bytes += observation.hash_input_bytes;
        self.hash_input_blocks += observation.hash_input_blocks;
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MptObservationError(String);

impl MptObservationError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for MptObservationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for MptObservationError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MptNodeKind {
    Branch,
    Extension,
    Leaf,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct PathObservation {
    pub(crate) node_visits: u64,
    pub(crate) visited_nodes: MptNodeKindCounts,
    pub(crate) found: bool,
    visited_paths: Vec<Vec<u8>>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct StructuralDelta {
    pub(crate) created_nodes: MptNodeKindCounts,
    pub(crate) removed_nodes: MptNodeKindCounts,
    pub(crate) created_encoded_bytes: u64,
    pub(crate) removed_encoded_bytes: u64,
    pub(crate) dirty_encoded_nodes: u64,
    pub(crate) dirty_encoded_bytes: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct MutationObservation {
    pub(crate) path: PathObservation,
    pub(crate) succeeded: bool,
    pub(crate) delta: StructuralDelta,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct HashObservation {
    pub(crate) dirty_encoded_nodes: u64,
    pub(crate) dirty_encoded_bytes: u64,
    pub(crate) dirty_hashed_nodes: u64,
    pub(crate) hash_input_bytes: u64,
    pub(crate) hash_input_blocks: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SnapshotNode {
    kind: MptNodeKind,
    rlp: Bytes,
    child_paths: Vec<Vec<u8>>,
    cached_on_prehashed_load: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct TrieSnapshot {
    nodes: BTreeMap<Vec<u8>, SnapshotNode>,
}

trait RlpLookup {
    fn rlp(&self, digest: &B256) -> Option<Bytes>;
}

impl<T: AsRef<[u8]>> RlpLookup for B256Map<T> {
    fn rlp(&self, digest: &B256) -> Option<Bytes> {
        self.get(digest)
            .map(|rlp| Bytes::copy_from_slice(rlp.as_ref()))
    }
}

impl RlpLookup for BTreeMap<B256, Bytes> {
    fn rlp(&self, digest: &B256) -> Option<Bytes> {
        self.get(digest).cloned()
    }
}

impl TrieSnapshot {
    pub(crate) fn ensure_node_bound(size: usize) -> Result<(), MptObservationError> {
        if size > MAX_SNAPSHOT_NODES {
            return Err(MptObservationError::new("MPT snapshot node bound exceeded"));
        }
        Ok(())
    }

    pub(crate) fn from_rlp_nodes(nodes: &[Bytes]) -> Result<Self, MptObservationError> {
        let Some(root) = nodes.first() else {
            return Ok(Self::default());
        };
        let rlp_by_digest = nodes
            .iter()
            .skip(1)
            .map(|rlp| (keccak256(rlp), rlp.clone()))
            .collect::<BTreeMap<_, _>>();
        let mut snapshot = Self::default();
        let mut materialization = MptMaterializationMetrics::default();
        materialization.record_resolved_rlp(root.len());
        snapshot.walk_resolved(
            root.clone(),
            &[],
            &rlp_by_digest,
            true,
            &mut materialization,
        )?;
        Ok(snapshot)
    }

    pub(crate) fn from_prehashed<T: AsRef<[u8]>>(
        root: B256,
        source: &B256Map<T>,
    ) -> Result<(Self, MptMaterializationMetrics), MptObservationError> {
        let mut materialization = MptMaterializationMetrics {
            operations: 1,
            ..Default::default()
        };
        if root == EMPTY_ROOT_HASH {
            return Ok((Self::default(), materialization));
        }

        let Some(root_rlp) = source.rlp(&root) else {
            materialization.unresolved_digest_lookups += 1;
            return Ok((Self::default(), materialization));
        };
        materialization.record_resolved_rlp(root_rlp.len());

        let mut snapshot = Self::default();
        snapshot.walk_resolved(root_rlp, &[], source, true, &mut materialization)?;
        Ok((snapshot, materialization))
    }

    fn walk_resolved<L: RlpLookup>(
        &mut self,
        rlp: Bytes,
        path: &[u8],
        rlp_by_digest: &L,
        cached_on_prehashed_load: bool,
        materialization: &mut MptMaterializationMetrics,
    ) -> Result<(), MptObservationError> {
        if path.len() > MAX_KEY_NIBBLES {
            return Err(MptObservationError::new(
                "MPT snapshot path exceeds 64 nibbles",
            ));
        }
        if self.nodes.len() >= MAX_SNAPSHOT_NODES {
            return Err(MptObservationError::new("MPT snapshot node bound exceeded"));
        }

        let node = decode_node(&rlp)?;
        let kind = node_kind(&node).ok_or_else(|| {
            MptObservationError::new("unexpected empty node inside non-empty MPT snapshot")
        })?;
        materialization.decoded_nodes.increment(kind);
        self.nodes.insert(
            path.to_vec(),
            SnapshotNode {
                kind,
                rlp,
                child_paths: Vec::new(),
                cached_on_prehashed_load,
            },
        );

        let mut child_paths = Vec::new();

        match node {
            TrieNode::EmptyRoot | TrieNode::Leaf(_) => {}
            TrieNode::Extension(extension) => {
                let mut child_path = path.to_vec();
                child_path.extend(extension.key.to_vec());
                if let Some(child) = resolve_child_for_materialization(
                    &extension.child,
                    rlp_by_digest,
                    materialization,
                ) {
                    let child_is_hashed = extension.child.as_hash().is_some();
                    child_paths.push(child_path.clone());
                    self.walk_resolved(
                        child,
                        &child_path,
                        rlp_by_digest,
                        child_is_hashed,
                        materialization,
                    )?;
                }
            }
            TrieNode::Branch(branch) => {
                for (nibble, child) in branch.as_ref().children() {
                    let Some(child) = child else { continue };
                    let mut child_path = path.to_vec();
                    child_path.push(nibble);
                    let child_is_hashed = child.as_hash().is_some();
                    if let Some(resolved_child) =
                        resolve_child_for_materialization(child, rlp_by_digest, materialization)
                    {
                        child_paths.push(child_path.clone());
                        self.walk_resolved(
                            resolved_child,
                            &child_path,
                            rlp_by_digest,
                            child_is_hashed,
                            materialization,
                        )?;
                    }
                }
            }
        }
        self.nodes
            .get_mut(path)
            .expect("snapshot node was inserted before walking its children")
            .child_paths = child_paths;
        Ok(())
    }

    pub(crate) fn cached_paths_on_prehashed_load(&self) -> BTreeSet<Vec<u8>> {
        self.nodes
            .iter()
            .filter(|(_, node)| node.cached_on_prehashed_load)
            .map(|(path, _)| path.clone())
            .collect()
    }

    pub(crate) fn observe_path(
        &self,
        key: impl AsRef<[u8]>,
    ) -> Result<PathObservation, MptObservationError> {
        let key = Nibbles::unpack(key);
        let key = key.to_vec();
        if self.nodes.is_empty() {
            let visited_nodes = MptNodeKindCounts {
                null: 1,
                ..Default::default()
            };
            return Ok(PathObservation {
                node_visits: 1,
                visited_nodes,
                found: false,
                visited_paths: vec![Vec::new()],
            });
        }

        let mut consumed = 0usize;
        let mut current_path = Vec::new();
        let mut observation = PathObservation::default();
        loop {
            if observation.node_visits > (MAX_KEY_NIBBLES + 1) as u64 {
                return Err(MptObservationError::new("MPT path node bound exceeded"));
            }
            let current = self.nodes.get(&current_path).ok_or_else(|| {
                MptObservationError::new("MPT target path reaches an unresolved child")
            })?;
            let node = decode_node(&current.rlp)?;
            let kind = node_kind(&node).ok_or_else(|| {
                MptObservationError::new("unexpected empty node inside non-empty MPT path")
            })?;
            observation.node_visits += 1;
            observation.visited_nodes.increment(kind);
            observation.visited_paths.push(current_path.clone());

            match node {
                TrieNode::EmptyRoot => unreachable!("handled by node_kind"),
                TrieNode::Leaf(leaf) => {
                    observation.found = key.get(consumed..) == Some(leaf.key.to_vec().as_slice());
                    return Ok(observation);
                }
                TrieNode::Extension(extension) => {
                    let extension_key = extension.key.to_vec();
                    if !key
                        .get(consumed..)
                        .is_some_and(|tail| tail.starts_with(&extension_key))
                    {
                        return Ok(observation);
                    }
                    consumed += extension_key.len();
                    current_path.extend(extension_key);
                    if !self.nodes.contains_key(&current_path) {
                        return Err(MptObservationError::new(
                            "MPT target path reaches an unresolved extension child",
                        ));
                    }
                }
                TrieNode::Branch(branch) => {
                    let Some(&nibble) = key.get(consumed) else {
                        return Ok(observation);
                    };
                    consumed += 1;
                    current_path.push(nibble);
                    let branch_ref = branch.as_ref();
                    let child = branch_ref
                        .children()
                        .find_map(|(index, child)| (index == nibble).then_some(child).flatten());
                    let Some(_) = child else {
                        return Ok(observation);
                    };
                    if !self.nodes.contains_key(&current_path) {
                        return Err(MptObservationError::new(
                            "MPT target path reaches an unresolved branch child",
                        ));
                    }
                }
            }
        }
    }

    pub(crate) fn node_counts_and_bytes(&self) -> (MptNodeKindCounts, u64) {
        let mut counts = MptNodeKindCounts::default();
        let mut bytes = 0;
        for node in self.nodes.values() {
            counts.increment(node.kind);
            bytes += node.rlp.len() as u64;
        }
        (counts, bytes)
    }

    fn diff(&self, after: &Self) -> StructuralDelta {
        let mut delta = StructuralDelta::default();
        for (path, before_node) in &self.nodes {
            match after.nodes.get(path) {
                None => {
                    delta.removed_nodes.increment(before_node.kind);
                    delta.removed_encoded_bytes += before_node.rlp.len() as u64;
                }
                Some(after_node) if after_node.kind != before_node.kind => {
                    delta.removed_nodes.increment(before_node.kind);
                    delta.created_nodes.increment(after_node.kind);
                    delta.removed_encoded_bytes += before_node.rlp.len() as u64;
                    delta.created_encoded_bytes += after_node.rlp.len() as u64;
                    delta.dirty_encoded_nodes += 1;
                    delta.dirty_encoded_bytes += after_node.rlp.len() as u64;
                }
                Some(after_node) if after_node.rlp != before_node.rlp => {
                    delta.dirty_encoded_nodes += 1;
                    delta.dirty_encoded_bytes += after_node.rlp.len() as u64;
                }
                Some(_) => {}
            }
        }
        for (path, after_node) in &after.nodes {
            if !self.nodes.contains_key(path) {
                // Path-keyed snapshots cannot prove object identity for a subtree that moved.
                // Treat it as newly encoded instead of pairing equal RLP bytes across paths: two
                // distinct subtrees may encode identically, and a moved subtree may already have
                // a dirty cache from an earlier mutation in the same hash batch. This deliberate
                // overcharge prevents either case from suppressing real final hash work.
                delta.created_nodes.increment(after_node.kind);
                delta.created_encoded_bytes += after_node.rlp.len() as u64;
                delta.dirty_encoded_nodes += 1;
                delta.dirty_encoded_bytes += after_node.rlp.len() as u64;
            }
        }
        delta
    }

    pub(crate) fn hash_observation(
        &self,
        cached_paths: &BTreeSet<Vec<u8>>,
    ) -> (HashObservation, BTreeSet<Vec<u8>>) {
        let mut observation = HashObservation::default();
        let mut newly_cached_paths = BTreeSet::new();
        self.observe_uncached_subtree(
            &Vec::new(),
            cached_paths,
            &mut newly_cached_paths,
            &mut observation,
        );
        (observation, newly_cached_paths)
    }

    fn observe_uncached_subtree(
        &self,
        path: &Vec<u8>,
        cached_paths: &BTreeSet<Vec<u8>>,
        newly_cached_paths: &mut BTreeSet<Vec<u8>>,
        observation: &mut HashObservation,
    ) {
        if cached_paths.contains(path) {
            return;
        }
        let Some(node) = self.nodes.get(path) else {
            return;
        };

        for child_path in &node.child_paths {
            self.observe_uncached_subtree(
                child_path,
                cached_paths,
                newly_cached_paths,
                observation,
            );
        }

        newly_cached_paths.insert(path.clone());
        observation.dirty_encoded_nodes += 1;
        observation.dirty_encoded_bytes += node.rlp.len() as u64;
        if path.is_empty() || node.rlp.len() >= 32 {
            let bytes = node.rlp.len() as u64;
            observation.dirty_hashed_nodes += 1;
            observation.hash_input_bytes += bytes;
            observation.hash_input_blocks += bytes / KECCAK_RATE_BYTES + 1;
        }
    }
}

pub(crate) fn observe_mutation(
    before: &TrieSnapshot,
    after: &TrieSnapshot,
    path: PathObservation,
    succeeded: bool,
    cached_paths: &mut BTreeSet<Vec<u8>>,
) -> MutationObservation {
    let delta = before.diff(after);
    if succeeded {
        cached_paths.retain(|node_path| {
            !path.visited_paths.contains(node_path)
                && before
                    .nodes
                    .get(node_path)
                    .zip(after.nodes.get(node_path))
                    .is_some_and(|(before, after)| {
                        before.kind == after.kind && before.rlp == after.rlp
                    })
        });
    }
    MutationObservation {
        path,
        succeeded,
        delta,
    }
}

fn decode_node(rlp: &[u8]) -> Result<TrieNode, MptObservationError> {
    TrieNode::decode(&mut &rlp[..])
        .map_err(|error| MptObservationError::new(format!("invalid resolved MPT node: {error}")))
}

const fn node_kind(node: &TrieNode) -> Option<MptNodeKind> {
    match node {
        TrieNode::EmptyRoot => None,
        TrieNode::Branch(_) => Some(MptNodeKind::Branch),
        TrieNode::Extension(_) => Some(MptNodeKind::Extension),
        TrieNode::Leaf(_) => Some(MptNodeKind::Leaf),
    }
}

fn resolve_child_for_materialization<L: RlpLookup>(
    child: &alloy_trie::nodes::RlpNode,
    rlp_by_digest: &L,
    materialization: &mut MptMaterializationMetrics,
) -> Option<Bytes> {
    match child.as_hash() {
        Some(hash) => {
            if let Some(rlp) = rlp_by_digest.rlp(&hash) {
                materialization.record_resolved_rlp(rlp.len());
                Some(rlp)
            } else {
                materialization.unresolved_digest_lookups += 1;
                None
            }
        }
        None => Some(Bytes::copy_from_slice(child.as_slice())),
    }
}

#[cfg(test)]
mod tests {
    use super::TrieSnapshot;
    use alloy_primitives::{Bytes, U256, keccak256, map::B256Map};
    use risc0_ethereum_trie::CachedTrie;
    use std::{cell::Cell, rc::Rc};

    struct CountedRlp {
        bytes: Bytes,
        accesses: Rc<Cell<u64>>,
    }

    impl AsRef<[u8]> for CountedRlp {
        fn as_ref(&self) -> &[u8] {
            self.accesses.set(self.accesses.get() + 1);
            &self.bytes
        }
    }

    #[test]
    fn prehashed_snapshot_does_not_scan_unreachable_witness_nodes() {
        let mut trie = CachedTrie::default();
        trie.insert([0x10], alloy_rlp::encode(U256::from(1)));
        trie.insert([0x20], alloy_rlp::encode(U256::from(2)));
        let root = trie.hash();
        let root_rlp = trie.rlp_nodes().remove(0);

        let reachable_accesses = Rc::new(Cell::new(0));
        let unreachable_accesses = Rc::new(Cell::new(0));
        let unreachable_rlp = Bytes::from_static(&[0xc2, 0x20, 0x01]);
        let mut source = B256Map::default();
        source.insert(
            root,
            CountedRlp {
                bytes: root_rlp,
                accesses: Rc::clone(&reachable_accesses),
            },
        );
        source.insert(
            keccak256(&unreachable_rlp),
            CountedRlp {
                bytes: unreachable_rlp,
                accesses: Rc::clone(&unreachable_accesses),
            },
        );

        TrieSnapshot::from_prehashed(root, &source).unwrap();

        assert_eq!(reachable_accesses.get(), 1);
        assert_eq!(unreachable_accesses.get(), 0);
    }
}
