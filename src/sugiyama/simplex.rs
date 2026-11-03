//! Network simplex (Gansner, Koutsofios, North y Vo, 1993) para el problema de rangos:
//! minimizar Σ ω(a)·(λ(head) − λ(tail)) sujeto a λ(head) − λ(tail) ≥ δ(a).
//!
//! Lo usa el layout para las capas y para la posición vertical. Todo es entero: con
//! datos enteros, el óptimo que encuentra el método también lo es.

/// Restricción `λ(head) − λ(tail) ≥ delta`, con costo `weight` por unidad de largo.
#[derive(Debug, Clone, Copy)]
pub struct Arc {
    pub tail: usize,
    pub head: usize,
    pub delta: i32,
    pub weight: i32,
}

/// Por qué no hay solución.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Failure {
    /// Las restricciones tienen un ciclo dirigido.
    Cycle,
    /// Se rompió un invariante del método: es un bug, no un problema de la entrada.
    Internal(&'static str),
}

/// Rangos óptimos de `n` nodos, con el mínimo de cada componente conexa en 0.
pub fn solve(n: usize, arcs: &[Arc]) -> Result<Vec<i32>, Failure> {
    let mut rank = initial_ranks(n, arcs).ok_or(Failure::Cycle)?;
    let mut adj = vec![Vec::new(); n];
    for (i, a) in arcs.iter().enumerate() {
        adj[a.tail].push(i);
        adj[a.head].push(i);
    }
    let mut done = vec![false; n];
    for root in 0..n {
        if done[root] {
            continue;
        }
        let nodes = component(root, arcs, &adj);
        for &v in &nodes {
            done[v] = true;
        }
        let mut tree = Tree::feasible(root, &nodes, arcs, &adj, &mut rank)?;
        tree.optimize(arcs, &adj, &mut rank)?;
        let min = nodes.iter().map(|&v| rank[v]).min().unwrap_or(0);
        for &v in &nodes {
            rank[v] -= min;
        }
    }
    Ok(rank)
}

fn slack(a: &Arc, rank: &[i32]) -> i32 {
    rank[a.head] - rank[a.tail] - a.delta
}

/// Rangos factibles: camino más largo en orden topológico.
fn initial_ranks(n: usize, arcs: &[Arc]) -> Option<Vec<i32>> {
    let mut indeg = vec![0; n];
    let mut out = vec![Vec::new(); n];
    for a in arcs {
        indeg[a.head] += 1;
        out[a.tail].push(a);
    }
    let mut rank = vec![0; n];
    let mut ready: Vec<usize> = (0..n).filter(|&v| indeg[v] == 0).collect();
    let mut seen = 0;
    while let Some(v) = ready.pop() {
        seen += 1;
        for a in &out[v] {
            rank[a.head] = rank[a.head].max(rank[v] + a.delta);
            indeg[a.head] -= 1;
            if indeg[a.head] == 0 {
                ready.push(a.head);
            }
        }
    }
    // Las fuentes empiezan en 0; un δ negativo puede dejar rangos negativos, que se
    // normalizan al final.
    (seen == n).then_some(rank)
}

fn component(root: usize, arcs: &[Arc], adj: &[Vec<usize>]) -> Vec<usize> {
    let mut seen = vec![false; adj.len()];
    seen[root] = true;
    let mut stack = vec![root];
    let mut nodes = Vec::new();
    while let Some(v) = stack.pop() {
        nodes.push(v);
        for &i in &adj[v] {
            let u = if arcs[i].tail == v {
                arcs[i].head
            } else {
                arcs[i].tail
            };
            if !seen[u] {
                seen[u] = true;
                stack.push(u);
            }
        }
    }
    nodes.sort_unstable();
    nodes
}

/// Árbol de expansión de arcos ajustados (slack 0) de una componente.
struct Tree {
    root: usize,
    nodes: Vec<usize>,
    /// Arcos de la componente (los demás no se miran).
    arcs: Vec<usize>,
    in_tree: Vec<bool>,
    /// Arco al padre en el árbol enraizado (`usize::MAX` en la raíz).
    parent: Vec<usize>,
    depth: Vec<usize>,
    /// Numeración en postorden: el subárbol de `v` es `low[v] ..= lim[v]`.
    low: Vec<usize>,
    lim: Vec<usize>,
}

impl Tree {
    fn feasible(
        root: usize,
        nodes: &[usize],
        arcs: &[Arc],
        adj: &[Vec<usize>],
        rank: &mut [i32],
    ) -> Result<Self, Failure> {
        let n = adj.len();
        let mut in_tree = vec![false; arcs.len()];
        let mut member = vec![false; n];
        let mut own: Vec<usize> = nodes.iter().flat_map(|&v| adj[v].iter().copied()).collect();
        own.sort_unstable();
        own.dedup();
        member[root] = true;
        let mut size = 1;
        loop {
            // Crecer por arcos ajustados.
            let mut stack: Vec<usize> = nodes.iter().copied().filter(|&v| member[v]).collect();
            while let Some(v) = stack.pop() {
                for &i in &adj[v] {
                    let a = &arcs[i];
                    let u = if a.tail == v { a.head } else { a.tail };
                    if !member[u] && slack(a, rank) == 0 {
                        member[u] = true;
                        in_tree[i] = true;
                        size += 1;
                        stack.push(u);
                    }
                }
            }
            if size == nodes.len() {
                break;
            }
            // El arco incidente de menor slack se ajusta moviendo todo el árbol.
            let (i, s) = own
                .iter()
                .copied()
                .filter(|&i| member[arcs[i].tail] != member[arcs[i].head])
                .map(|i| (i, slack(&arcs[i], rank)))
                .min_by_key(|&(i, s)| (s, i))
                .ok_or(Failure::Internal(
                    "árbol factible: la componente no es conexa",
                ))?;
            let shift = if member[arcs[i].tail] { s } else { -s };
            for &v in nodes.iter().filter(|&&v| member[v]) {
                rank[v] += shift;
            }
        }
        let mut tree = Self {
            root,
            nodes: nodes.to_vec(),
            arcs: own,
            in_tree,
            parent: vec![usize::MAX; n],
            depth: vec![0; n],
            low: vec![0; n],
            lim: vec![0; n],
        };
        tree.index(arcs, adj);
        Ok(tree)
    }

    /// Recalcula padre, profundidad y numeración en postorden desde la raíz.
    fn index(&mut self, arcs: &[Arc], adj: &[Vec<usize>]) {
        self.parent[self.root] = usize::MAX;
        self.depth[self.root] = 0;
        let mut counter = 0;
        // (nodo, próximo índice de adyacencia, low acumulado)
        let mut stack = vec![(self.root, 0usize, usize::MAX)];
        while let Some(&mut (v, ref mut next, ref mut low)) = stack.last_mut() {
            if let Some(&i) = adj[v].get(*next) {
                *next += 1;
                if !self.in_tree[i] || i == self.parent[v] {
                    continue;
                }
                let u = if arcs[i].tail == v {
                    arcs[i].head
                } else {
                    arcs[i].tail
                };
                self.parent[u] = i;
                self.depth[u] = self.depth[v] + 1;
                stack.push((u, 0, usize::MAX));
                continue;
            }
            let low_v = (*low).min(counter);
            self.low[v] = low_v;
            self.lim[v] = counter;
            counter += 1;
            stack.pop();
            if let Some(top) = stack.last_mut() {
                top.2 = top.2.min(low_v);
            }
        }
    }

    fn in_subtree(&self, v: usize, of: usize) -> bool {
        self.low[of] <= self.lim[v] && self.lim[v] <= self.lim[of]
    }

    /// Nodo hijo del arco `i` del árbol: la raíz de su subárbol.
    fn child(&self, i: usize, arcs: &[Arc]) -> usize {
        let a = &arcs[i];
        if self.parent[a.head] == i {
            a.head
        } else {
            a.tail
        }
    }

    /// Valor de corte de cada arco del árbol: peso que cruza de la componente de su
    /// cola a la de su cabeza, menos el que cruza al revés. Cada arco contribuye a los
    /// arcos del camino del árbol entre sus extremos.
    fn cut_values(&self, arcs: &[Arc]) -> Vec<i64> {
        let mut cut = vec![0i64; arcs.len()];
        for f in self.arcs.iter().map(|&i| &arcs[i]) {
            let w = f.weight as i64;
            let (mut t, mut h) = (f.tail, f.head);
            while t != h {
                // Subir por el lado más profundo. Desde la cola, el subárbol del hijo
                // contiene la cola de f: f sale de él; desde la cabeza, entra.
                let from_tail = self.depth[t] >= self.depth[h];
                let v = if from_tail { t } else { h };
                let i = self.parent[v];
                let a = &arcs[i];
                let leaves_subtree = from_tail;
                // Si el subárbol es la componente de la cabeza de `a`, salir de él es ir
                // de cabeza a cola (negativo).
                let subtree_is_head = a.head == v;
                cut[i] += if leaves_subtree != subtree_is_head {
                    w
                } else {
                    -w
                };
                let up = if a.tail == v { a.head } else { a.tail };
                if from_tail {
                    t = up;
                } else {
                    h = up;
                }
            }
        }
        cut
    }

    fn optimize(
        &mut self,
        arcs: &[Arc],
        adj: &[Vec<usize>],
        rank: &mut [i32],
    ) -> Result<(), Failure> {
        // Tope de iteraciones contra ciclos degenerados: lo que haya es factible.
        let cap = 100 * self.nodes.len() + 1000;
        for _ in 0..cap {
            let cut = self.cut_values(arcs);
            let Some(leave) = self
                .arcs
                .iter()
                .copied()
                .filter(|&i| self.in_tree[i] && cut[i] < 0)
                .min_by_key(|&i| (cut[i], i))
            else {
                return Ok(());
            };
            let child = self.child(leave, arcs);
            let child_is_head = arcs[leave].head == child;
            // Entra el arco de menor slack que va de la componente de la cabeza a la de
            // la cola del arco que sale.
            let enter = self
                .arcs
                .iter()
                .copied()
                .filter(|&i| !self.in_tree[i])
                .filter(|&i| {
                    let (t, h) = (
                        self.in_subtree(arcs[i].tail, child),
                        self.in_subtree(arcs[i].head, child),
                    );
                    if child_is_head { t && !h } else { !t && h }
                })
                .min_by_key(|&i| (slack(&arcs[i], rank), i))
                .ok_or(Failure::Internal(
                    "un valor de corte negativo no tiene arco de reemplazo",
                ))?;
            let s = slack(&arcs[enter], rank);
            let shift = if child_is_head { s } else { -s };
            for &v in &self.nodes {
                if self.in_subtree(v, child) {
                    rank[v] += shift;
                }
            }
            self.in_tree[leave] = false;
            self.in_tree[enter] = true;
            self.index(arcs, adj);
        }
        // Se alcanzó el tope: la solución actual es factible aunque no sea óptima.
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arc(tail: usize, head: usize, delta: i32, weight: i32) -> Arc {
        Arc {
            tail,
            head,
            delta,
            weight,
        }
    }

    fn cost(arcs: &[Arc], rank: &[i32]) -> i64 {
        arcs.iter()
            .map(|a| a.weight as i64 * (rank[a.head] - rank[a.tail]) as i64)
            .sum()
    }

    #[test]
    fn layering_pulls_sources_next_to_their_consumer() {
        // 0 -> 1 -> 2 -> 3 y 4 -> 3: el camino más largo deja 4 en el rango 0; el óptimo
        // lo pone pegado a 3.
        let arcs = [
            arc(0, 1, 1, 1),
            arc(1, 2, 1, 1),
            arc(2, 3, 1, 1),
            arc(4, 3, 1, 1),
        ];
        let rank = solve(5, &arcs).unwrap();
        assert_eq!(rank, [0, 1, 2, 3, 2]);
    }

    #[test]
    fn negative_deltas_align_ports() {
        // Minimizar |(y0 + 3) - (y1 + 1)| con y1 >= y0 + 5 (separación): queda y1 = y0 + 5.
        // Nodo auxiliar 2 debajo de ambos puertos.
        let arcs = [arc(0, 1, 5, 0), arc(2, 0, -3, 1), arc(2, 1, -1, 1)];
        let rank = solve(3, &arcs).unwrap();
        assert_eq!(rank[1] - rank[0], 5);
        // Sin la separación los puertos quedan alineados.
        let rank = solve(3, &arcs[1..]).unwrap();
        assert_eq!(rank[0] + 3, rank[1] + 1);
    }

    #[test]
    fn cycles_are_rejected_and_components_are_solved_apart() {
        assert_eq!(
            solve(2, &[arc(0, 1, 1, 1), arc(1, 0, 1, 1)]),
            Err(Failure::Cycle)
        );
        let rank = solve(4, &[arc(0, 1, 2, 1), arc(3, 2, 1, 1)]).unwrap();
        assert_eq!(rank, [0, 2, 1, 0]);
    }

    #[test]
    fn result_is_feasible_and_no_worse_than_longest_path() {
        // Grafos pseudoaleatorios deterministas: factibilidad y costo.
        let mut seed = 7u64;
        let mut next = |m: u64| {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (seed >> 33) % m
        };
        for _ in 0..200 {
            let n = 2 + next(20) as usize;
            let mut arcs = Vec::new();
            for _ in 0..n + next(2 * n as u64) as usize {
                let (a, b) = (next(n as u64) as usize, next(n as u64) as usize);
                if a < b {
                    arcs.push(arc(a, b, next(4) as i32 - 1, next(4) as i32));
                }
            }
            let rank = solve(n, &arcs).unwrap();
            assert!(
                arcs.iter().all(|a| slack(a, &rank) >= 0),
                "{arcs:?} {rank:?}"
            );
            let start = initial_ranks(n, &arcs).unwrap();
            assert!(cost(&arcs, &rank) <= cost(&arcs, &start));
        }
    }
}
