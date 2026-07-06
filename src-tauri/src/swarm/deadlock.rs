use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use parking_lot::RwLock;
use serde::Serialize;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::error;

use super::events::SwarmEvent;
use super::bus::AgentBus;

pub struct WaitForGraph {
    edges: HashMap<String, HashSet<String>>,
}

impl WaitForGraph {
    pub fn new() -> Self {
        Self {
            edges: HashMap::new(),
        }
    }

    pub fn add_wait(&mut self, waiter: &str, wait_for: &str) {
        self.edges
            .entry(waiter.to_string())
            .or_default()
            .insert(wait_for.to_string());
    }

    pub fn remove_wait(&mut self, waiter: &str, wait_for: &str) {
        if let Some(set) = self.edges.get_mut(waiter) {
            set.remove(wait_for);
            if set.is_empty() {
                self.edges.remove(waiter);
            }
        }
    }

    pub fn remove_all(&mut self, waiter: &str) {
        self.edges.remove(waiter);
    }

    pub fn find_cycle(&self) -> Option<Vec<String>> {
        let mut color: HashMap<&str, u8> = HashMap::new();
        let mut parent: HashMap<&str, Option<&str>> = HashMap::new();

        for node in self.edges.keys() {
            color.insert(node.as_str(), 0);
            parent.insert(node.as_str(), None);
        }

        for node in self.edges.keys() {
            if color.get(node.as_str()) == Some(&0) {
                if let Some(cycle) = self.dfs(node.as_str(), &mut color, &mut parent) {
                    return Some(cycle);
                }
            }
        }

        None
    }

    fn dfs<'a>(
        &'a self,
        node: &'a str,
        color: &mut HashMap<&'a str, u8>,
        parent: &mut HashMap<&'a str, Option<&'a str>>,
    ) -> Option<Vec<String>> {
        color.insert(node, 1);

        if let Some(neighbors) = self.edges.get(node) {
            for neighbor in neighbors {
                let nb = neighbor.as_str();
                match color.get(nb) {
                    Some(&1) => {
                        let mut cycle = vec![nb.to_string()];
                        let mut cur = node;
                        while cur != nb {
                            cycle.push(cur.to_string());
                            cur = parent.get(cur).unwrap().unwrap();
                        }
                        cycle.reverse();
                        return Some(cycle);
                    }
                    Some(&0) => {
                        parent.insert(nb, Some(node));
                        if let Some(cycle) = self.dfs(nb, color, parent) {
                            return Some(cycle);
                        }
                    }
                    _ => {}
                }
            }
        }

        color.insert(node, 2);
        None
    }

    pub fn total_edges(&self) -> usize {
        self.edges.values().map(|s| s.len()).sum()
    }

    pub fn active_waiters(&self) -> usize {
        self.edges.len()
    }

    pub fn all_edges(&self) -> Vec<(String, String)> {
        let mut edges = Vec::new();
        for (waiter, wait_fors) in &self.edges {
            for wait_for in wait_fors {
                edges.push((waiter.clone(), wait_for.clone()));
            }
        }
        edges
    }
}

impl Default for WaitForGraph {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Serialize, Clone)]
pub struct DeadlockStatus {
    pub total_edges: usize,
    pub active_waiters: usize,
    pub has_deadlock: bool,
    pub current_cycle: Option<Vec<String>>,
    pub detected_at: Option<i64>,
}

pub struct DeadlockDetector {
    graph: Arc<RwLock<WaitForGraph>>,
    cancel: CancellationToken,
    handle: Option<JoinHandle<()>>,
    last_cycle: Arc<RwLock<Option<(Vec<String>, i64)>>>,
    event_sender: Option<tokio::sync::broadcast::Sender<SwarmEvent>>,
}

impl DeadlockDetector {
    pub fn new() -> Self {
        Self {
            graph: Arc::new(RwLock::new(WaitForGraph::new())),
            cancel: CancellationToken::new(),
            handle: None,
            last_cycle: Arc::new(RwLock::new(None)),
            event_sender: None,
        }
    }

    pub fn with_bus(bus: &AgentBus) -> Self {
        Self {
            graph: bus.wait_for_graph(),
            cancel: CancellationToken::new(),
            handle: None,
            last_cycle: Arc::new(RwLock::new(None)),
            event_sender: Some(bus.event_sender()),
        }
    }

    pub fn start(&mut self) {
        if self.handle.is_some() {
            return;
        }

        let graph = Arc::clone(&self.graph);
        let cancel = self.cancel.clone();
        let last_cycle = Arc::clone(&self.last_cycle);
        let event_sender = self.event_sender.clone();

        let handle = tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(1));
            loop {
                tokio::select! {
                    _ = cancel.cancelled() => {
                        break;
                    }
                    _ = interval.tick() => {
                        let cycle_opt = graph.read().find_cycle();
                        match cycle_opt {
                            Some(cycle) => {
                                let now = chrono::Utc::now().timestamp_millis();
                                let mut last = last_cycle.write();
                                let is_new = match &*last {
                                    None => true,
                                    Some((existing_cycle, _)) => existing_cycle != &cycle,
                                };
                                *last = Some((cycle.clone(), now));
                                drop(last);

                                if is_new {
                                    error!(
                                        target: "nebula.deadlock",
                                        cycle = ?cycle,
                                        "deadlock detected in WaitForGraph"
                                    );
                                    if let Some(ref sender) = event_sender {
                                        let _ = sender.send(SwarmEvent::deadlock_detected(cycle.clone(), now));
                                    }
                                }
                            }
                            None => {
                                let mut last = last_cycle.write();
                                if last.is_some() {
                                    *last = None;
                                }
                            }
                        }
                    }
                }
            }
        });

        self.handle = Some(handle);
    }

    pub fn stop(&mut self) {
        self.cancel.cancel();
        if let Some(handle) = self.handle.take() {
            handle.abort();
        }
    }

    pub fn graph(&self) -> Arc<RwLock<WaitForGraph>> {
        Arc::clone(&self.graph)
    }

    pub fn status(&self) -> DeadlockStatus {
        let graph = self.graph.read();
        let last = self.last_cycle.read();
        let (current_cycle, detected_at) = match &*last {
            Some((cycle, ts)) => (Some(cycle.clone()), Some(*ts)),
            None => (None, None),
        };
        DeadlockStatus {
            total_edges: graph.total_edges(),
            active_waiters: graph.active_waiters(),
            has_deadlock: current_cycle.is_some(),
            current_cycle,
            detected_at,
        }
    }
}

impl Default for DeadlockDetector {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for DeadlockDetector {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_add_remove_basic() {
        let mut g = WaitForGraph::new();
        assert_eq!(g.total_edges(), 0);
        assert_eq!(g.active_waiters(), 0);

        g.add_wait("A", "B");
        assert_eq!(g.total_edges(), 1);
        assert_eq!(g.active_waiters(), 1);

        g.add_wait("A", "C");
        assert_eq!(g.total_edges(), 2);
        assert_eq!(g.active_waiters(), 1);

        g.add_wait("B", "C");
        assert_eq!(g.total_edges(), 3);
        assert_eq!(g.active_waiters(), 2);

        g.remove_wait("A", "B");
        assert_eq!(g.total_edges(), 2);
        assert_eq!(g.active_waiters(), 2);

        g.remove_wait("A", "C");
        assert_eq!(g.total_edges(), 1);
        assert_eq!(g.active_waiters(), 1);
    }

    #[test]
    fn test_find_cycle_2_node() {
        let mut g = WaitForGraph::new();
        g.add_wait("A", "B");
        g.add_wait("B", "A");

        let cycle = g.find_cycle();
        assert!(cycle.is_some());
        let cycle = cycle.unwrap();
        assert_eq!(cycle.len(), 2);
        assert!(cycle.contains(&"A".to_string()));
        assert!(cycle.contains(&"B".to_string()));
    }

    #[test]
    fn test_find_cycle_3_node() {
        let mut g = WaitForGraph::new();
        g.add_wait("A", "B");
        g.add_wait("B", "C");
        g.add_wait("C", "A");

        let cycle = g.find_cycle();
        assert!(cycle.is_some());
        let cycle = cycle.unwrap();
        assert_eq!(cycle.len(), 3);
        assert!(cycle.contains(&"A".to_string()));
        assert!(cycle.contains(&"B".to_string()));
        assert!(cycle.contains(&"C".to_string()));
    }

    #[test]
    fn test_find_cycle_self_loop() {
        let mut g = WaitForGraph::new();
        g.add_wait("A", "A");

        let cycle = g.find_cycle();
        assert!(cycle.is_some());
        let cycle = cycle.unwrap();
        assert_eq!(cycle.len(), 1);
        assert_eq!(cycle[0], "A");
    }

    #[test]
    fn test_find_cycle_no_cycle() {
        let mut g = WaitForGraph::new();
        g.add_wait("A", "B");
        g.add_wait("B", "C");
        g.add_wait("C", "D");

        assert!(g.find_cycle().is_none());
    }

    #[test]
    fn test_find_cycle_multiple_rings() {
        let mut g = WaitForGraph::new();
        g.add_wait("A", "B");
        g.add_wait("B", "A");
        g.add_wait("C", "D");
        g.add_wait("D", "E");
        g.add_wait("E", "C");

        let cycle = g.find_cycle();
        assert!(cycle.is_some());
    }

    #[test]
    fn test_remove_all() {
        let mut g = WaitForGraph::new();
        g.add_wait("A", "B");
        g.add_wait("A", "C");
        g.add_wait("A", "D");
        assert_eq!(g.total_edges(), 3);
        assert_eq!(g.active_waiters(), 1);

        g.remove_all("A");
        assert_eq!(g.total_edges(), 0);
        assert_eq!(g.active_waiters(), 0);
    }

    #[tokio::test]
    async fn test_deadlock_status_accurate() {
        let detector = DeadlockDetector::new();
        let status = detector.status();
        assert_eq!(status.total_edges, 0);
        assert_eq!(status.active_waiters, 0);
        assert!(!status.has_deadlock);
        assert!(status.current_cycle.is_none());
        assert!(status.detected_at.is_none());

        detector.graph().write().add_wait("A", "B");
        detector.graph().write().add_wait("B", "A");

        let status = detector.status();
        assert_eq!(status.total_edges, 2);
        assert_eq!(status.active_waiters, 2);
    }

    #[test]
    fn test_concurrent_access() {
        use std::sync::Arc;
        use std::thread;

        let graph = Arc::new(RwLock::new(WaitForGraph::new()));
        let mut handles = Vec::new();

        for i in 0..10 {
            let g = Arc::clone(&graph);
            handles.push(thread::spawn(move || {
                let waiter = format!("agent-{}", i);
                for j in 0..5 {
                    let target = format!("target-{}", j);
                    g.write().add_wait(&waiter, &target);
                    g.read().find_cycle();
                    g.write().remove_wait(&waiter, &target);
                }
            }));
        }

        for h in handles {
            h.join().unwrap();
        }

        assert_eq!(graph.read().total_edges(), 0);
        assert_eq!(graph.read().active_waiters(), 0);
    }
}
