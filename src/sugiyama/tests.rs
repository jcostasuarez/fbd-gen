use super::*;

/// Pseudoaleatorio determinista (LCG de Knuth) para los tests con grafos al azar.
pub(super) struct Lcg(u64);

impl Lcg {
    const MULTIPLIER: u64 = 6_364_136_223_846_793_005;
    const INCREMENT: u64 = 1_442_695_040_888_963_407;

    pub(super) fn new(seed: u64) -> Self {
        Self(seed)
    }

    /// Entero uniforme en `0..m`.
    pub(super) fn below(&mut self, m: usize) -> usize {
        self.0 = self
            .0
            .wrapping_mul(Self::MULTIPLIER)
            .wrapping_add(Self::INCREMENT);
        let m = u64::try_from(m).expect("cota de prueba en u64");
        usize::try_from((self.0 >> 33) % m).expect("menor que la cota")
    }

    /// Entero uniforme en `0..m`, para filas y medidas de la grilla.
    pub(super) fn below_i32(&mut self, m: i32) -> i32 {
        let m = usize::try_from(m).expect("cota de prueba positiva");
        i32::try_from(self.below(m)).expect("menor que la cota")
    }
}

const SPACING: Spacing = Spacing { node: 2, layer: 3 };

/// Polilínea completa de cada cable: puerto de salida, quiebres, puerto de entrada.
fn wires(nodes: &[Node], edges: &[Edge], p: &Placement) -> Vec<Vec<Pos>> {
    edges
        .iter()
        .zip(&p.bends)
        .map(|(e, bends)| {
            let (a, b) = (p.nodes[e.from], p.nodes[e.to]);
            let mut line = vec![Pos {
                x: a.x + nodes[e.from].width,
                y: a.y + e.from_row,
            }];
            line.extend(bends);
            line.push(Pos {
                x: b.x,
                y: b.y + e.to_row,
            });
            line
        })
        .collect()
}

/// Lo que tiene que cumplir cualquier resultado. Devuelve el problema encontrado.
fn violation(nodes: &[Node], edges: &[Edge], p: &Placement) -> Option<String> {
    for (i, a) in nodes.iter().enumerate() {
        let pa = p.nodes[i];
        for (j, b) in nodes.iter().enumerate().skip(i + 1) {
            let pb = p.nodes[j];
            let apart = pa.x + a.width < pb.x
                || pb.x + b.width < pa.x
                || pa.y + a.height <= pb.y
                || pb.y + b.height <= pa.y;
            if !apart {
                return Some(format!("nodos {i} y {j} superpuestos"));
            }
        }
    }
    let lines = wires(nodes, edges, p);
    let segs: Vec<Vec<(Pos, Pos)>> = lines
        .iter()
        .map(|l| l.windows(2).map(|w| (w[0], w[1])).collect())
        .collect();
    for (i, s) in segs.iter().enumerate() {
        for &(a, b) in s {
            if a.x != b.x && a.y != b.y {
                return Some(format!("cable {i} diagonal: {:?}", lines[i]));
            }
            // Ningún tramo atraviesa una caja (los puertos están en su borde).
            for (k, n) in nodes.iter().enumerate() {
                let o = p.nodes[k];
                let rows = o.y..o.y + n.height;
                let hits = if a.y == b.y {
                    rows.contains(&a.y) && a.x.min(b.x) < o.x + n.width && a.x.max(b.x) > o.x
                } else {
                    (o.x..=o.x + n.width).contains(&a.x)
                        && a.y.min(b.y) < o.y + n.height
                        && a.y.max(b.y) >= o.y
                };
                if hits {
                    return Some(format!("cable {i} atraviesa el nodo {k}: {:?}", lines[i]));
                }
            }
        }
    }
    // Cables de salidas distintas: ni tramos superpuestos ni un quiebre sobre otro cable.
    let on = |q: Pos, (a, b): (Pos, Pos)| {
        (a.x.min(b.x)..=a.x.max(b.x)).contains(&q.x) && (a.y.min(b.y)..=a.y.max(b.y)).contains(&q.y)
    };
    for i in 0..edges.len() {
        for j in i + 1..edges.len() {
            let (ei, ej) = (&edges[i], &edges[j]);
            if (ei.from, ei.from_row) == (ej.from, ej.from_row) {
                continue;
            }
            for &s in &segs[i] {
                for &t in &segs[j] {
                    // Un tramo superpuesto deja un extremo sobre el otro tramo.
                    if on(t.0, s) || on(t.1, s) || on(s.0, t) || on(s.1, t) {
                        return Some(format!(
                            "cables {i} y {j} se tocan: {:?} / {:?}",
                            lines[i], lines[j]
                        ));
                    }
                }
            }
        }
    }
    None
}

fn node(width: i32, height: i32) -> Node {
    Node { width, height }
}

fn edge(from: usize, from_row: i32, to: usize, to_row: i32) -> Edge {
    Edge {
        from,
        from_row,
        to,
        to_row,
    }
}

#[test]
fn chain_is_straight() {
    let nodes = [node(8, 6), node(8, 6), node(8, 6)];
    let edges = [edge(0, 4, 1, 4), edge(1, 4, 2, 4)];
    let p = layout(&nodes, &edges, &SPACING).unwrap();
    assert!(p.bends.iter().all(Vec::is_empty), "{p:?}");
    assert_eq!(p.nodes[1].x, 8 + 3);
    assert_eq!(violation(&nodes, &edges, &p), None);
}

#[test]
fn long_edges_and_fanout_respect_the_invariants() {
    // 0 alimenta a 1, 2 y 3; 3 está dos capas más allá.
    let nodes = [node(7, 6), node(9, 7), node(7, 5), node(10, 8)];
    let edges = [
        edge(0, 4, 1, 4),
        edge(0, 4, 2, 4),
        edge(1, 5, 3, 4),
        edge(0, 4, 3, 6),
    ];
    let p = layout(&nodes, &edges, &SPACING).unwrap();
    assert_eq!(violation(&nodes, &edges, &p), None, "{p:?}");
}

#[test]
fn cycles_are_rejected() {
    let nodes = [node(7, 6), node(7, 6)];
    assert!(matches!(
        layout(&nodes, &[edge(0, 4, 1, 4), edge(1, 4, 0, 5)], &SPACING),
        Err(LayoutError::Cycle)
    ));
    assert!(matches!(
        layout(&nodes, &[edge(0, 9, 1, 4)], &SPACING),
        Err(LayoutError::PortOutOfNode { edge: 0 })
    ));
}

#[test]
fn random_networks_respect_the_invariants() {
    let mut rng = Lcg::new(99);
    for case in 0..1000 {
        let n = 1 + rng.below(30);
        let mut nodes = Vec::new();
        let mut ports = Vec::new();
        for _ in 0..n {
            let (ins, outs) = (1 + rng.below_i32(5), 1 + rng.below_i32(3));
            nodes.push(node(
                5 + rng.below_i32(14),
                4 + ins.max(outs) + rng.below_i32(2),
            ));
            ports.push((ins, outs));
        }
        let mut edges = Vec::new();
        let mut driven = std::collections::HashSet::new();
        for _ in 0..n + rng.below(2 * n) {
            let (a, b) = (rng.below(n), rng.below(n));
            let to_row = 4 + rng.below_i32(ports[b].0);
            if a < b && driven.insert((b, to_row)) {
                edges.push(edge(a, 4 + rng.below_i32(ports[a].1), b, to_row));
            }
        }
        let p = layout(&nodes, &edges, &SPACING).unwrap();
        if let Some(v) = violation(&nodes, &edges, &p) {
            panic!("caso {case}: {v}\n{nodes:?}\n{edges:?}\n{p:?}");
        }
    }
}

#[test]
fn oversized_or_negative_nodes_are_rejected_instead_of_overflowing() {
    let wide = [
        Node {
            width: MAX_EXTENT,
            height: 4,
        },
        Node {
            width: 5,
            height: 4,
        },
    ];
    let edge = [Edge {
        from: 0,
        from_row: 1,
        to: 1,
        to_row: 1,
    }];
    assert_eq!(
        layout(&wide, &edge, &SPACING).unwrap_err(),
        LayoutError::TooLarge
    );
    let tall = [Node {
        width: 5,
        height: i32::MAX,
    }];
    assert_eq!(
        layout(&tall, &[], &SPACING).unwrap_err(),
        LayoutError::TooLarge
    );
    let negative = [Node {
        width: -1,
        height: 4,
    }];
    assert_eq!(
        layout(&negative, &[], &SPACING).unwrap_err(),
        LayoutError::TooLarge
    );
}
