//! Client-to-worker routing policy.

use std::collections::HashMap;

use super::id::ClientId;

/// Tracks the worker assigned to each client and balances new clients.
#[derive(Debug)]
pub(crate) struct ClientDispatcher {
    assignments: HashMap<ClientId, usize>,
    assigned_counts: Vec<usize>,
    cursor: usize,
}

impl ClientDispatcher {
    /// Create a dispatcher for a fixed number of workers.
    pub(crate) fn new(worker_count: usize) -> Self {
        debug_assert!(worker_count > 0);
        Self {
            assignments: HashMap::new(),
            assigned_counts: vec![0; worker_count],
            cursor: 0,
        }
    }

    /// Return the worker already assigned to `client`, if any.
    pub(crate) fn assigned_worker(&self, client: ClientId) -> Option<usize> {
        self.assignments.get(&client).copied()
    }

    /// Assign `client` to the least-loaded worker and return that worker.
    ///
    /// Existing assignments are returned unchanged.
    pub(crate) fn assign(&mut self, client: ClientId, capacities: &[usize]) -> usize {
        if let Some(&worker) = self.assignments.get(&client) {
            return worker;
        }

        let worker = choose_worker(capacities, &self.assigned_counts, self.cursor);
        self.assignments.insert(client, worker);
        self.assigned_counts[worker] += 1;
        self.cursor = (worker + 1) % capacities.len();
        worker
    }
}

/// Choose the worker with the fewest assigned clients, then the largest
/// remaining capacity, then the first worker in round-robin order.
pub(crate) fn choose_worker(
    capacities: &[usize],
    assigned_counts: &[usize],
    cursor: usize,
) -> usize {
    debug_assert!(!capacities.is_empty());
    debug_assert_eq!(capacities.len(), assigned_counts.len());

    let worker_count = capacities.len();
    let mut best = cursor % worker_count;
    let mut best_count = assigned_counts[best];
    let mut best_capacity = capacities[best];

    for offset in 1..worker_count {
        let index = (cursor + offset) % worker_count;
        let count = assigned_counts[index];
        let capacity = capacities[index];

        if count < best_count || (count == best_count && capacity > best_capacity) {
            best = index;
            best_count = count;
            best_capacity = capacity;
        }
    }

    best
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assigned_count_wins_over_capacity() {
        assert_eq!(choose_worker(&[256, 0, 256], &[2, 0, 1], 0), 1);
    }

    #[test]
    fn capacity_breaks_assigned_count_ties() {
        assert_eq!(choose_worker(&[0, 5, 5], &[1, 1, 2], 0), 1);
        assert_eq!(choose_worker(&[5, 0, 5], &[1, 1, 1], 0), 0);
    }

    #[test]
    fn cursor_breaks_remaining_ties() {
        assert_eq!(choose_worker(&[5, 5, 5], &[1, 1, 1], 1), 1);
        assert_eq!(choose_worker(&[5, 5, 5], &[1, 1, 1], 2), 2);
    }

    #[test]
    fn new_clients_spread_across_equal_workers() {
        let mut dispatcher = ClientDispatcher::new(3);

        let workers: Vec<_> = (0..3)
            .map(|client| dispatcher.assign(client, &[256, 256, 256]))
            .collect();

        assert_eq!(workers, vec![0, 1, 2]);
    }

    #[test]
    fn existing_client_stays_pinned() {
        let mut dispatcher = ClientDispatcher::new(3);

        let worker = dispatcher.assign(7, &[256, 256, 256]);
        assert_eq!(dispatcher.assigned_worker(7), Some(worker));
        assert_eq!(dispatcher.assign(7, &[0, 0, 0]), worker);
    }
}
