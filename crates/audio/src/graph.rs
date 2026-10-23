//! The graph and its rendering (Web Audio 1.1 section 2.4, "Rendering an
//! Audio Graph"): its nodes ordered so that each is processed after what
//! feeds it; a cycle allowed only through a `DelayNode`, which then gives
//! what it delayed before taking this quantum's input (and delays a
//! quantum at least); any other cycle muted.

use crate::mix::{Bus, Interpretation, Mode, computed_channels, mix_into, silence};
use crate::nodes::Kind;
use crate::param::Param;
use crate::{MAX_NODES, QUANTUM};

/// A node's index in its graph.
pub type NodeId = u32;

/// The quantum being rendered: its first frame and the rate.
#[derive(Debug, Clone, Copy)]
pub struct Quantum {
    pub start: u64,
    pub rate: f32,
}

impl Quantum {
    /// The time of frame `i` of the quantum, in seconds.
    pub fn time(&self, i: usize) -> f64 {
        (self.start + i as u64) as f64 / f64::from(self.rate)
    }
}

/// A node: what it does, its channel rules, its params and what is
/// connected to it.
pub struct Node {
    pub kind: Kind,
    pub inputs: usize,
    pub outputs: usize,
    pub channel_count: usize,
    pub mode: Mode,
    pub interpretation: Interpretation,
    pub params: Vec<Param>,
    /// What feeds each input: a node and its output.
    sources: Vec<Vec<(NodeId, usize)>>,
    /// What feeds each param.
    param_sources: Vec<Vec<(NodeId, usize)>>,
    /// Its outputs as last rendered.
    out: Vec<Bus>,
    /// Whether it is in a cycle with no delay: silent.
    muted: bool,
    /// Whether it is a delay in a cycle: it reads before it writes.
    breaks_cycle: bool,
}

impl Node {
    pub fn new(
        kind: Kind,
        inputs: usize,
        outputs: usize,
        channel_count: usize,
        mode: Mode,
        interpretation: Interpretation,
        params: Vec<Param>,
    ) -> Node {
        let param_count = params.len();
        Node {
            kind,
            inputs,
            outputs,
            channel_count,
            mode,
            interpretation,
            params,
            sources: vec![Vec::new(); inputs],
            param_sources: vec![Vec::new(); param_count],
            out: vec![silence(1); outputs],
            muted: false,
            breaks_cycle: false,
        }
    }

    /// What it last rendered at `output`.
    pub fn output(&self, output: usize) -> Option<&Bus> {
        self.out.get(output)
    }
}

/// A context's graph. Node 0 is its destination.
pub struct Graph {
    pub rate: f32,
    nodes: Vec<Option<Node>>,
    frame: u64,
    order: Option<Vec<NodeId>>,
    /// The listener's params (`AudioListener`): position, forward and up.
    pub listener: Vec<Param>,
    /// Sources that ended in the last quantum, for their `ended` events.
    ended: Vec<NodeId>,
}

impl Graph {
    /// A graph at `rate` whose destination has `channels` channels.
    pub fn new(rate: f32, channels: usize) -> Graph {
        let destination = crate::nodes::destination(channels);
        Graph {
            rate,
            nodes: vec![Some(destination)],
            frame: 0,
            order: None,
            listener: crate::nodes::panner::listener_params(),
            ended: Vec::new(),
        }
    }

    /// The frame the next quantum starts at.
    pub fn frame(&self) -> u64 {
        self.frame
    }

    /// The time of the next quantum, in seconds: `currentTime`.
    pub fn current_time(&self) -> f64 {
        self.frame as f64 / f64::from(self.rate)
    }

    pub fn add(&mut self, node: Node) -> Option<NodeId> {
        let live = self.nodes.iter().filter(|n| n.is_some()).count();
        if live >= MAX_NODES {
            return None;
        }
        self.nodes.push(Some(node));
        self.order = None;
        NodeId::try_from(self.nodes.len() - 1).ok()
    }

    pub fn node(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(id as usize)?.as_ref()
    }

    pub fn node_mut(&mut self, id: NodeId) -> Option<&mut Node> {
        self.nodes.get_mut(id as usize)?.as_mut()
    }

    /// Connects `from`'s output to `to`'s input; false if either is not
    /// there. Connecting twice is once.
    pub fn connect(&mut self, from: NodeId, output: usize, to: NodeId, input: usize) -> bool {
        let ok = self.node(from).is_some_and(|n| output < n.outputs)
            && self.node(to).is_some_and(|n| input < n.inputs);
        if !ok {
            return false;
        }
        let Some(node) = self.node_mut(to) else {
            return false;
        };
        if !node.sources[input].contains(&(from, output)) {
            node.sources[input].push((from, output));
        }
        self.order = None;
        true
    }

    /// Connects `from`'s output to `to`'s param.
    pub fn connect_param(&mut self, from: NodeId, output: usize, to: NodeId, param: usize) -> bool {
        let ok = self.node(from).is_some_and(|n| output < n.outputs)
            && self.node(to).is_some_and(|n| param < n.params.len());
        if !ok {
            return false;
        }
        let Some(node) = self.node_mut(to) else {
            return false;
        };
        if !node.param_sources[param].contains(&(from, output)) {
            node.param_sources[param].push((from, output));
        }
        self.order = None;
        true
    }

    /// Disconnects `from`'s connections that match: its `output` if
    /// given, to `to` if given, at its `input` (or param) if given. How
    /// many there were.
    pub fn disconnect(
        &mut self,
        from: NodeId,
        output: Option<usize>,
        to: Option<NodeId>,
        input: Option<usize>,
        param: bool,
    ) -> usize {
        let mut removed = 0;
        for (id, slot) in self.nodes.iter_mut().enumerate() {
            let Some(node) = slot else { continue };
            if to.is_some_and(|t| t as usize != id) {
                continue;
            }
            let lists = if param {
                &mut node.param_sources
            } else {
                &mut node.sources
            };
            for (index, list) in lists.iter_mut().enumerate() {
                if input.is_some_and(|i| i != index) {
                    continue;
                }
                let before = list.len();
                list.retain(|(f, o)| !(*f == from && output.is_none_or(|out| out == *o)));
                removed += before - list.len();
            }
        }
        if removed > 0 {
            self.order = None;
        }
        removed
    }

    /// Takes `id` out of the graph, with every connection to and from it.
    pub fn remove(&mut self, id: NodeId) {
        if id == 0 {
            return;
        }
        self.disconnect(id, None, None, None, false);
        self.disconnect(id, None, None, None, true);
        if let Some(slot) = self.nodes.get_mut(id as usize) {
            *slot = None;
        }
        self.order = None;
    }

    /// The sources that ended since this was last asked.
    pub fn take_ended(&mut self) -> Vec<NodeId> {
        std::mem::take(&mut self.ended)
    }

    /// Orders the nodes, marks cycles: those through a delay broken there,
    /// the others muted.
    fn plan(&mut self) -> Vec<NodeId> {
        let n = self.nodes.len();
        // Edges: from a node to the nodes it feeds.
        let mut feeds: Vec<Vec<usize>> = vec![Vec::new(); n];
        for (id, slot) in self.nodes.iter().enumerate() {
            let Some(node) = slot else { continue };
            for &(from, _) in node
                .sources
                .iter()
                .flatten()
                .chain(node.param_sources.iter().flatten())
            {
                if self.nodes.get(from as usize).is_some_and(Option::is_some) {
                    feeds[from as usize].push(id);
                }
            }
        }
        let components = strongly_connected(&feeds, |i| self.nodes[i].is_some());
        for slot in self.nodes.iter_mut().flatten() {
            slot.muted = false;
            slot.breaks_cycle = false;
        }
        let mut cut: Vec<bool> = vec![false; n];
        // Each node's component, and whether that component is muted:
        // its own edges are then dropped from the order, so that its
        // members are processed (silent) and what they feed is not held
        // back by them.
        let mut component_of = vec![usize::MAX; n];
        let mut muted_component = vec![false; components.len()];
        for (c, component) in components.iter().enumerate() {
            for &i in component {
                component_of[i] = c;
            }
        }
        for (c, component) in components.iter().enumerate() {
            let cyclic = component.len() > 1 || feeds[component[0]].contains(&component[0]);
            if !cyclic {
                continue;
            }
            let delays: Vec<usize> = component
                .iter()
                .copied()
                .filter(|&i| {
                    matches!(
                        self.nodes[i].as_ref().map(|n| &n.kind),
                        Some(Kind::Delay(_))
                    )
                })
                .collect();
            if delays.is_empty() {
                muted_component[c] = true;
                for &i in component {
                    if let Some(node) = self.nodes[i].as_mut() {
                        node.muted = true;
                    }
                }
            } else {
                for i in delays {
                    cut[i] = true;
                    if let Some(node) = self.nodes[i].as_mut() {
                        node.breaks_cycle = true;
                    }
                }
            }
        }
        // Order with the edges into cycle-breaking delays dropped, and
        // those inside a muted cycle.
        let inside_muted = |from: usize, to: usize| {
            component_of[from] == component_of[to]
                && muted_component
                    .get(component_of[to])
                    .copied()
                    .unwrap_or(false)
        };
        let mut indegree = vec![0usize; n];
        for (from, targets) in feeds.iter().enumerate() {
            for &to in targets {
                if !cut[to] && from != to && !inside_muted(from, to) {
                    indegree[to] += 1;
                }
            }
        }
        let mut ready: Vec<usize> = (0..n)
            .filter(|&i| self.nodes[i].is_some() && indegree[i] == 0)
            .collect();
        let mut order = Vec::with_capacity(n);
        while let Some(i) = ready.pop() {
            order.push(i as NodeId);
            for &to in &feeds[i] {
                if cut[to] || to == i || inside_muted(i, to) {
                    continue;
                }
                indegree[to] -= 1;
                if indegree[to] == 0 {
                    ready.push(to);
                }
            }
        }
        // What is left is in a cycle a delay did not break: muted, last.
        for i in 0..n {
            if self.nodes[i].is_some() && !order.contains(&(i as NodeId)) {
                if let Some(node) = self.nodes[i].as_mut() {
                    node.muted = true;
                }
                order.push(i as NodeId);
            }
        }
        order
    }

    /// The mix of what feeds `list`, at `channels` channels.
    fn gather(
        &self,
        list: &[(NodeId, usize)],
        channels: usize,
        interpretation: Interpretation,
    ) -> Bus {
        let mut bus = silence(channels);
        for &(from, output) in list {
            if let Some(source) = self.node(from).and_then(|n| n.out.get(output)) {
                mix_into(&mut bus, source, interpretation);
            }
        }
        bus
    }

    /// Renders one quantum and gives the destination's sound.
    pub fn render(&mut self) -> Bus {
        if self.order.is_none() {
            self.order = Some(self.plan());
        }
        let order = self.order.clone().unwrap_or_default();
        let quantum = Quantum {
            start: self.frame,
            rate: self.rate,
        };
        let mut late_writes = Vec::new();
        for &id in &order {
            let Some(node) = self.node(id) else { continue };
            // Its inputs, each mixed to its computed channels.
            let inputs: Vec<Bus> = node
                .sources
                .iter()
                .map(|list| {
                    let counts = list.iter().filter_map(|&(f, o)| {
                        self.node(f).and_then(|n| n.out.get(o)).map(Vec::len)
                    });
                    let channels = if list.is_empty() {
                        1
                    } else {
                        computed_channels(counts, node.channel_count, node.mode)
                    };
                    self.gather(list, channels, node.interpretation)
                })
                .collect();
            // Its params' inputs, each mixed to mono.
            let param_inputs: Vec<Option<[f32; QUANTUM]>> = node
                .param_sources
                .iter()
                .map(|list| {
                    (!list.is_empty()).then(|| self.gather(list, 1, Interpretation::Speakers)[0])
                })
                .collect();
            let breaks = node.breaks_cycle;
            let muted = node.muted;
            let listener = self.listener.clone();
            let Some(node) = self.node_mut(id) else {
                continue;
            };
            let mut params = Vec::with_capacity(node.params.len());
            for (param, input) in node.params.iter().zip(&param_inputs) {
                let mut values = [0.0; QUANTUM];
                param.computed(
                    quantum.start,
                    quantum.rate,
                    input.as_ref().map(|v| &v[..]),
                    &mut values,
                );
                params.push(values);
            }
            let context = crate::nodes::Context {
                quantum,
                listener: &listener,
                channel_count: node.channel_count,
            };
            let (outputs, ended) = if breaks {
                // Read now; the input is taken once every node has run.
                late_writes.push(id);
                node.kind.read_delayed(&context, &params)
            } else {
                node.kind.process(&context, &inputs, &params)
            };
            node.out = if muted {
                (0..node.outputs).map(|_| silence(1)).collect()
            } else {
                outputs
            };
            if ended {
                self.ended.push(id);
            }
        }
        for id in late_writes {
            let Some(node) = self.node(id) else { continue };
            let list = &node.sources[0];
            let counts = list
                .iter()
                .filter_map(|&(f, o)| self.node(f).and_then(|n| n.out.get(o)).map(Vec::len));
            let channels = if list.is_empty() {
                1
            } else {
                computed_channels(counts, node.channel_count, node.mode)
            };
            let input = self.gather(list, channels, node.interpretation);
            if let Some(node) = self.node_mut(id)
                && let Kind::Delay(delay) = &mut node.kind
            {
                delay.write(&input);
            }
        }
        // Timelines kept short: what is wholly past forgotten.
        let now = quantum.time(0);
        for node in self.nodes.iter_mut().flatten() {
            for param in &mut node.params {
                if param.events().len() > 64 {
                    param.forget_before(now);
                }
            }
        }
        self.frame += QUANTUM as u64;
        // The destination's channels, whatever it rendered: muted in a
        // cycle, it gave one channel of silence.
        let Some(destination) = self.node(0) else {
            return silence(2);
        };
        let mut bus = silence(destination.channel_count);
        mix_into(&mut bus, &destination.out[0], Interpretation::Discrete);
        bus
    }
}

/// The strongly connected components of a graph by Tarjan's algorithm,
/// iteratively; nodes `live` excludes are left out.
fn strongly_connected(feeds: &[Vec<usize>], live: impl Fn(usize) -> bool) -> Vec<Vec<usize>> {
    let n = feeds.len();
    let mut index = vec![usize::MAX; n];
    let mut low = vec![0; n];
    let mut on_stack = vec![false; n];
    let mut stack = Vec::new();
    let mut components = Vec::new();
    let mut next = 0;
    for root in 0..n {
        if !live(root) || index[root] != usize::MAX {
            continue;
        }
        // (node, the next edge of it to look at)
        let mut work = vec![(root, 0usize)];
        while let Some(top) = work.len().checked_sub(1) {
            let (v, edge) = work[top];
            if index[v] == usize::MAX {
                index[v] = next;
                low[v] = next;
                next += 1;
                stack.push(v);
                on_stack[v] = true;
            }
            if let Some(&w) = feeds[v].get(edge) {
                work[top].1 += 1;
                if !live(w) {
                    continue;
                }
                if index[w] == usize::MAX {
                    work.push((w, 0));
                } else if on_stack[w] {
                    low[v] = low[v].min(index[w]);
                }
                continue;
            }
            work.pop();
            if let Some(&(parent, _)) = work.last() {
                low[parent] = low[parent].min(low[v]);
            }
            if low[v] == index[v] {
                let mut component = Vec::new();
                while let Some(w) = stack.pop() {
                    on_stack[w] = false;
                    component.push(w);
                    if w == v {
                        break;
                    }
                }
                components.push(component);
            }
        }
    }
    components
}
