use std::collections::HashMap;

use crate::client::client_id::ClientId;

/// Assigns each client to a worker and pins it there.
#[derive(Debug)]
pub struct ClientDispatcher {
    worker_of: HashMap<ClientId, usize>,
    assigned: Vec<usize>,
    cursor: usize,
}

impl ClientDispatcher {
    pub fn new(workers: usize) -> Self {
        assert!(workers > 0, "at least one worker is required");
        Self {
            worker_of: HashMap::new(),
            assigned: vec![0; workers],
            cursor: 0,
        }
    }

    /// Returns the worker for `client`, assigning a least-loaded worker on first
    /// sight. `capacities` is the remaining queue capacity of each worker.
    pub fn worker_for(&mut self, client: ClientId, capacities: &[usize]) -> usize {
        if let Some(&worker) = self.worker_of.get(&client) {
            return worker;
        }

        let workers = self.assigned.len();
        debug_assert_eq!(capacities.len(), workers);
        let start = self.cursor % workers;
        let mut best = start;
        for offset in 1..workers {
            let worker = (start + offset) % workers;
            let more_capacity = capacities[worker] > capacities[best];
            let fewer_clients = self.assigned[worker] < self.assigned[best];
            let same_load_more_capacity =
                self.assigned[worker] == self.assigned[best] && more_capacity;
            if fewer_clients || same_load_more_capacity {
                best = worker;
            }
        }

        self.assigned[best] += 1;
        self.worker_of.insert(client, best);
        self.cursor = (best + 1) % workers;
        best
    }

    pub fn assigned(&self) -> &[usize] {
        &self.assigned
    }

    pub fn worker_of(&self, client: ClientId) -> Option<usize> {
        self.worker_of.get(&client).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pins_a_client_to_its_first_worker() {
        let mut dispatcher = ClientDispatcher::new(2);
        let first = dispatcher.worker_for(7, &[10, 10]);
        let again = dispatcher.worker_for(7, &[0, 0]);
        assert_eq!(first, again);
    }

    #[test]
    fn spreads_new_clients_across_workers() {
        let mut dispatcher = ClientDispatcher::new(2);
        let a = dispatcher.worker_for(1, &[10, 10]);
        let b = dispatcher.worker_for(2, &[10, 10]);
        assert_ne!(a, b);
        assert_eq!(dispatcher.assigned(), &[1, 1]);
    }

    #[test]
    fn prefers_the_worker_with_fewer_clients() {
        let mut dispatcher = ClientDispatcher::new(2);
        dispatcher.worker_for(1, &[10, 10]);
        // Worker 0 now has one client; worker 1 has none.
        let next = dispatcher.worker_for(2, &[10, 10]);
        assert_eq!(next, 1);
    }

    #[test]
    fn breaks_load_ties_by_remaining_capacity() {
        let mut dispatcher = ClientDispatcher::new(2);
        let next = dispatcher.worker_for(1, &[0, 5]);
        assert_eq!(next, 1);
    }

    #[test]
    fn breaks_full_ties_in_round_robin_order() {
        let mut dispatcher = ClientDispatcher::new(3);
        let order: Vec<usize> = (1..=3)
            .map(|client| dispatcher.worker_for(client, &[10, 10, 10]))
            .collect();
        assert_eq!(order, vec![0, 1, 2]);
        let next = dispatcher.worker_for(4, &[10, 10, 10]);
        assert_eq!(next, 0);
    }
}
