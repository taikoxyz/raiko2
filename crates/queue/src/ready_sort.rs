//! Ordering for Low/Medium ready queues (High stays FIFO).

use crate::types::TaskId;
use std::cmp::Ordering;
use std::collections::VecDeque;

/// Total order used for **Low** and **Medium** ready queues (smaller runs first).
///
/// **High** priority is unchanged (FIFO list / separate path).
pub trait ReadyQueueSort: Clone + Eq + std::hash::Hash + Send + Sync + 'static {
    /// 16-byte key that sorts lexicographically the same as
    /// [`ReadyQueueSort::cmp_for_ready_queue`].
    fn ready_queue_sort_prefix(&self) -> [u8; 16];

    fn cmp_for_ready_queue(&self, other: &Self) -> Ordering {
        self.ready_queue_sort_prefix()
            .cmp(&other.ready_queue_sort_prefix())
    }
}

#[must_use]
pub fn sort_prefix_hex(prefix: &[u8; 16]) -> String {
    let mut s = String::with_capacity(32);
    for b in prefix {
        use core::fmt::Write;
        write!(&mut s, "{b:02x}").unwrap();
    }
    s
}

/// Hex length for [`sort_prefix_hex`].
pub const READY_SORT_PREFIX_HEX_LEN: usize = 32;

#[must_use]
pub fn zset_member_from_encoded<Id: ReadyQueueSort>(
    id: &TaskId<Id>,
    encoded_task_id: &str,
) -> String {
    format!(
        "{}{}",
        sort_prefix_hex(&id.0.ready_queue_sort_prefix()),
        encoded_task_id
    )
}

/// Strip [`sort_prefix_hex`] prefix and return the encoded task id substring.
pub fn encoded_from_zset_member(member: &str) -> Option<&str> {
    member.get(READY_SORT_PREFIX_HEX_LEN..)
}

/// Insert `id` into `deque` sorted ascending by [`ReadyQueueSort`], stable for equal keys
/// (new id is placed after existing equal keys — FIFO among equals).
pub fn insert_ready_sorted<Id: ReadyQueueSort>(deque: &mut VecDeque<TaskId<Id>>, id: TaskId<Id>) {
    let pos = deque.partition_point(|existing| {
        matches!(
            existing.0.cmp_for_ready_queue(&id.0),
            Ordering::Less | Ordering::Equal
        )
    });
    deque.insert(pos, id);
}

impl ReadyQueueSort for u64 {
    fn ready_queue_sort_prefix(&self) -> [u8; 16] {
        let mut b = [0u8; 16];
        b[0..8].copy_from_slice(&self.to_be_bytes());
        b
    }
}

impl ReadyQueueSort for () {
    fn ready_queue_sort_prefix(&self) -> [u8; 16] {
        [0u8; 16]
    }
}
