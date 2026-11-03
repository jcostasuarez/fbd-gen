//! Algoritmos de grafos compartidos por las etapas. Todos son deterministas: los empates
//! se resuelven por índice, que es el orden del modelo.

use std::cmp::Reverse;
use std::collections::BinaryHeap;

/// Union-find con compresión de caminos. La raíz de cada clase es su menor elemento.
pub struct UnionFind(Vec<usize>);

impl UnionFind {
    pub fn new(n: usize) -> Self {
        Self((0..n).collect())
    }

    pub fn find(&mut self, x: usize) -> usize {
        let mut r = x;
        while self.0[r] != r {
            r = self.0[r];
        }
        let mut c = x;
        while self.0[c] != r {
            let next = self.0[c];
            self.0[c] = r;
            c = next;
        }
        r
    }

    pub fn union(&mut self, a: usize, b: usize) {
        let (a, b) = (self.find(a), self.find(b));
        if a != b {
            self.0[a.max(b)] = a.min(b);
        }
    }
}

/// Qué hace [`topo_order`] cuando un ciclo deja nodos sin ordenar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnCycle {
    /// Devolver el orden parcial: los nodos del ciclo (y lo que depende de ellos) faltan.
    Stop,
    /// Tomar el menor nodo pendiente como si estuviera listo y seguir.
    Break,
}

/// Orden topológico de `0..n` (Kahn). Entre los nodos listos sale primero el de menor
/// índice. Las aristas repetidas cuentan tantas veces como aparecen.
pub fn topo_order(
    n: usize,
    edges: impl IntoIterator<Item = (usize, usize)>,
    on_cycle: OnCycle,
) -> Vec<usize> {
    let mut indeg = vec![0usize; n];
    let mut successors: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (from, to) in edges {
        indeg[to] += 1;
        successors[from].push(to);
    }
    let mut ready: BinaryHeap<Reverse<usize>> =
        (0..n).filter(|&v| indeg[v] == 0).map(Reverse).collect();
    let mut placed = vec![false; n];
    let mut order = Vec::with_capacity(n);
    while order.len() < n {
        let v = match ready.pop() {
            Some(Reverse(v)) => v,
            None if on_cycle == OnCycle::Break => match (0..n).find(|&v| !placed[v]) {
                Some(v) => v,
                None => break,
            },
            None => break,
        };
        if placed[v] {
            continue;
        }
        placed[v] = true;
        order.push(v);
        for &to in &successors[v] {
            indeg[to] -= 1;
            if indeg[to] == 0 && !placed[to] {
                ready.push(Reverse(to));
            }
        }
    }
    order
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ties_follow_index_order_and_cycles_follow_the_policy() {
        assert_eq!(topo_order(4, [(3, 0), (1, 2)], OnCycle::Stop), [1, 2, 3, 0]);
        // 1 -> 2 -> 1 es un ciclo: Stop deja afuera el ciclo y lo que depende de él.
        let edges = [(0, 1), (1, 2), (2, 1), (2, 3)];
        assert_eq!(topo_order(4, edges, OnCycle::Stop), [0]);
        assert_eq!(topo_order(4, edges, OnCycle::Break), [0, 1, 2, 3]);
    }

    #[test]
    fn union_find_roots_are_the_smallest_member() {
        let mut uf = UnionFind::new(5);
        uf.union(4, 2);
        uf.union(2, 3);
        assert_eq!(uf.find(4), 2);
        assert_eq!(uf.find(3), 2);
        assert_eq!(uf.find(0), 0);
    }
}
