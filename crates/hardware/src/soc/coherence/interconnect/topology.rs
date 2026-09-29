//! Network topologies: ring, 2-D mesh and hypercube routing.

/// The shape of a routed network: which nodes exist, who neighbours whom,
/// and the next hop from any node towards any other.
pub trait NetworkTopology: Send + Sync + std::fmt::Debug {
    /// Human-readable kind.
    fn kind(&self) -> &'static str;
    /// Routers in the network (at least the number of endpoints).
    fn node_count(&self) -> usize;
    /// Neighbours of `node`, in a fixed order.
    fn neighbours(&self, node: usize) -> Vec<usize>;
    /// Next node on the route from `from` to `to` (`from != to`).
    fn next_hop(&self, from: usize, to: usize) -> usize;
    /// Longest route between two nodes, in hops.
    fn diameter(&self) -> usize;
}

/// A bidirectional ring; routes take the shorter direction.
#[derive(Clone, Copy, Debug)]
pub struct Ring {
    nodes: usize,
}

impl Ring {
    /// A ring of `nodes` stops (at least two).
    #[must_use]
    pub fn new(nodes: usize) -> Self {
        Self { nodes: nodes.max(2) }
    }
}

impl NetworkTopology for Ring {
    fn kind(&self) -> &'static str {
        "ring"
    }

    fn node_count(&self) -> usize {
        self.nodes
    }

    fn neighbours(&self, node: usize) -> Vec<usize> {
        vec![(node + self.nodes - 1) % self.nodes, (node + 1) % self.nodes]
    }

    fn next_hop(&self, from: usize, to: usize) -> usize {
        let forward = (to + self.nodes - from) % self.nodes;
        if forward <= self.nodes - forward {
            (from + 1) % self.nodes
        } else {
            (from + self.nodes - 1) % self.nodes
        }
    }

    fn diameter(&self) -> usize {
        self.nodes / 2
    }
}

/// A `k × k` grid with XY routing, optionally with wraparound (a torus).
#[derive(Clone, Copy, Debug)]
pub struct Mesh2D {
    k: usize,
    wraparound: bool,
}

impl Mesh2D {
    /// The smallest square grid with room for `endpoints`.
    #[must_use]
    pub fn for_endpoints(endpoints: usize, wraparound: bool) -> Self {
        let mut k = 1;
        while k * k < endpoints.max(1) {
            k += 1;
        }
        Self { k: k.max(2), wraparound }
    }

    const fn coords(&self, node: usize) -> (usize, usize) {
        (node % self.k, node / self.k)
    }

    const fn node(&self, x: usize, y: usize) -> usize {
        y * self.k + x
    }

    /// Next coordinate towards `to` along one axis.
    const fn step(&self, from: usize, to: usize) -> usize {
        if from == to {
            return from;
        }
        if !self.wraparound {
            return if to > from { from + 1 } else { from - 1 };
        }
        let forward = (to + self.k - from) % self.k;
        if forward <= self.k - forward { (from + 1) % self.k } else { (from + self.k - 1) % self.k }
    }
}

impl NetworkTopology for Mesh2D {
    fn kind(&self) -> &'static str {
        if self.wraparound { "torus" } else { "mesh" }
    }

    fn node_count(&self) -> usize {
        self.k * self.k
    }

    fn neighbours(&self, node: usize) -> Vec<usize> {
        let (x, y) = self.coords(node);
        let mut out = Vec::with_capacity(4);
        if self.wraparound {
            out.push(self.node((x + self.k - 1) % self.k, y));
            out.push(self.node((x + 1) % self.k, y));
            out.push(self.node(x, (y + self.k - 1) % self.k));
            out.push(self.node(x, (y + 1) % self.k));
        } else {
            if x > 0 {
                out.push(self.node(x - 1, y));
            }
            if x + 1 < self.k {
                out.push(self.node(x + 1, y));
            }
            if y > 0 {
                out.push(self.node(x, y - 1));
            }
            if y + 1 < self.k {
                out.push(self.node(x, y + 1));
            }
        }
        out.sort_unstable();
        out.dedup();
        out
    }

    fn next_hop(&self, from: usize, to: usize) -> usize {
        let (fx, fy) = self.coords(from);
        let (tx, ty) = self.coords(to);
        if fx == tx { self.node(fx, self.step(fy, ty)) } else { self.node(self.step(fx, tx), fy) }
    }

    fn diameter(&self) -> usize {
        if self.wraparound { 2 * (self.k / 2) } else { 2 * (self.k - 1) }
    }
}

/// A hypercube with dimension-order routing.
#[derive(Clone, Copy, Debug)]
pub struct Hypercube {
    dims: u32,
}

impl Hypercube {
    /// The smallest hypercube with room for `endpoints`.
    #[must_use]
    pub fn for_endpoints(endpoints: usize) -> Self {
        let mut dims = 1;
        while (1usize << dims) < endpoints.max(2) {
            dims += 1;
        }
        Self { dims }
    }
}

impl NetworkTopology for Hypercube {
    fn kind(&self) -> &'static str {
        "hypercube"
    }

    fn node_count(&self) -> usize {
        1 << self.dims
    }

    fn neighbours(&self, node: usize) -> Vec<usize> {
        (0..self.dims).map(|d| node ^ (1 << d)).collect()
    }

    fn next_hop(&self, from: usize, to: usize) -> usize {
        let differing = from ^ to;
        from ^ (1 << differing.trailing_zeros())
    }

    fn diameter(&self) -> usize {
        self.dims as usize
    }
}
