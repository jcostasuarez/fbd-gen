//! Layout jerárquico (Sugiyama) en celdas enteras, a la medida de una red FBD: flujo de
//! izquierda a derecha, puertos fijos en filas enteras y cables ortogonales.
//!
//! 1. Capas: network simplex sobre los cables (largo total mínimo, cada cable avanza al
//!    menos una capa).
//! 2. Nodos ficticios de una fila para los cables que saltan capas.
//! 3. Orden en cada capa: barycenter por fila de puerto con barridos alternados; se
//!    conserva el orden con menos cruces y los empates siguen el orden del modelo.
//! 4. Posición vertical: network simplex sobre el grafo auxiliar de Gansner, que minimiza
//!    el desvío de cada cable respetando la separación entre nodos de una capa.
//! 5. Ruteo: en cada canal entre capas, un tronco vertical por salida (sus cables lo
//!    comparten) en columnas que no superponen cables distintos (ver `routing`).
//!
//! No usa flotantes: entradas, cuentas intermedias y salida son enteros.

mod routing;
mod simplex;

use std::error::Error;
use std::fmt;

use routing::{Route, Trunk};
use simplex::Arc;

/// Extensión máxima de un dibujo por eje, en celdas. Con este tope las cuentas en `i32`
/// del motor (y de quien escale las coordenadas, como una vista previa en píxeles) no
/// desbordan; una red más grande es [`LayoutError::TooLarge`].
pub const MAX_EXTENT: i32 = 1 << 24;

/// Celda de la grilla: columna `x`, fila `y`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Pos {
    pub x: i32,
    pub y: i32,
}

/// Caja de un nodo. Los puertos de entrada están en la columna 0 y los de salida en la
/// columna `width`, cada uno en su fila (relativa al borde superior).
#[derive(Debug, Clone, Copy)]
pub struct Node {
    pub width: i32,
    pub height: i32,
}

/// Cable de la salida `from_row` de `from` a la entrada `to_row` de `to`.
#[derive(Debug, Clone, Copy)]
pub struct Edge {
    pub from: usize,
    pub from_row: i32,
    pub to: usize,
    pub to_row: i32,
}

#[derive(Debug, Clone, Copy)]
pub struct Spacing {
    /// Filas libres entre dos nodos de una misma capa.
    pub node: i32,
    /// Ancho mínimo, en columnas, del canal entre dos capas.
    pub layer: i32,
}

#[derive(Debug, Clone)]
pub struct Placement {
    /// Esquina superior izquierda de cada nodo.
    pub nodes: Vec<Pos>,
    /// Quiebres de cada cable, entre su puerto de salida y su puerto de entrada.
    pub bends: Vec<Vec<Pos>>,
}

/// Entrada que el motor no puede dibujar.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum LayoutError {
    /// El cable `edge` apunta a un nodo inexistente o a una fila fuera de su nodo.
    PortOutOfNode { edge: usize },
    /// Los cables forman un ciclo dirigido (hay que cortarlo antes).
    Cycle,
    /// Las restricciones de posición vertical no tienen solución.
    Infeasible,
    /// Un nodo tiene medidas negativas, o el dibujo podría exceder [`MAX_EXTENT`].
    TooLarge,
}

impl fmt::Display for LayoutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PortOutOfNode { edge } => write!(f, "cable {edge}: puerto fuera de su nodo"),
            Self::Cycle => write!(f, "los cables forman un ciclo"),
            Self::Infeasible => write!(f, "restricciones de posición vertical inconsistentes"),
            Self::TooLarge => write!(f, "la red excede {MAX_EXTENT} celdas por eje"),
        }
    }
}

impl Error for LayoutError {}

type Result<T> = std::result::Result<T, LayoutError>;

/// Nodo real o ficticio (un tramo de un cable largo dentro de una capa).
struct Vertex {
    layer: usize,
    height: i32,
    width: i32,
    real: bool,
    /// Clave de desempate: orden del modelo.
    model: (usize, usize),
}

/// Tramo de un cable entre dos capas consecutivas.
struct Segment {
    edge: usize,
    from: usize,
    from_row: i32,
    to: usize,
    to_row: i32,
}

/// Los cables deben formar un grafo sin ciclos (la normalización corta los lazos).
pub fn layout(nodes: &[Node], edges: &[Edge], spacing: &Spacing) -> Result<Placement> {
    if nodes.iter().any(|n| n.width < 0 || n.height < 1) {
        return Err(LayoutError::TooLarge);
    }
    for (i, e) in edges.iter().enumerate() {
        let ok = |v: usize, row: i32| v < nodes.len() && (0..nodes[v].height).contains(&row);
        if !ok(e.from, e.from_row) || !ok(e.to, e.to_row) {
            return Err(LayoutError::PortOutOfNode { edge: i });
        }
    }
    let (vertices, segments) = split_long_edges(nodes, edges)?;
    check_extent(&vertices, &segments, spacing)?;
    let layers = order_layers(&vertices, &segments);
    let ys = place_vertically(&vertices, &segments, &layers, spacing)?;
    Ok(route(
        nodes, edges, &vertices, &segments, &layers, &ys, spacing,
    ))
}

/// Cota superior del dibujo, antes de posicionar: si entra en [`MAX_EXTENT`], ninguna
/// cuenta posterior desborda.
///
/// - Vertical: el network simplex deja ajustado un árbol de expansión del grafo auxiliar
///   (vértices más un nodo por tramo), así que dos rangos difieren a lo sumo en
///   `nodos × δ máximo`, y δ está acotado por el alto de un vértice más la separación. Las
///   filas de unión de un tronco partido pueden salir del rango, una fila por extremo
///   ocupado como mucho.
/// - Horizontal: el ancho de cada capa más su canal, que usa a lo sumo dos columnas por
///   tramo.
fn check_extent(vertices: &[Vertex], segments: &[Segment], spacing: &Spacing) -> Result<()> {
    let (v, s) = (vertices.len() as i64, segments.len() as i64);
    let node_gap = i64::from(spacing.node).abs().max(1);
    let layer_gap = i64::from(spacing.layer).abs();
    let delta = vertices
        .iter()
        .map(|x| i64::from(x.height))
        .max()
        .unwrap_or(0)
        + node_gap
        + 1;
    let layers = vertices
        .iter()
        .map(|x| x.layer as i64 + 1)
        .max()
        .unwrap_or(0);
    let widths: i64 = vertices.iter().map(|x| i64::from(x.width)).sum();
    let height = (v + s)
        .saturating_mul(delta)
        .saturating_mul(2)
        .saturating_add(4 * s + 2);
    let width = widths
        .saturating_add(layers.saturating_mul(layer_gap + 2))
        .saturating_add(2 * s);
    if height.max(width) > i64::from(MAX_EXTENT) {
        return Err(LayoutError::TooLarge);
    }
    Ok(())
}

/// Etapas 1 y 2: capas por network simplex y nodos ficticios.
fn split_long_edges(nodes: &[Node], edges: &[Edge]) -> Result<(Vec<Vertex>, Vec<Segment>)> {
    let arcs: Vec<Arc> = edges
        .iter()
        .map(|e| Arc {
            tail: e.from,
            head: e.to,
            delta: 1,
            weight: 1,
        })
        .collect();
    let Some(rank) = simplex::solve(nodes.len(), &arcs) else {
        return Err(LayoutError::Cycle);
    };
    let mut vertices: Vec<Vertex> = nodes
        .iter()
        .enumerate()
        .map(|(i, n)| Vertex {
            layer: rank[i] as usize,
            height: n.height,
            width: n.width,
            real: true,
            model: (i, 0),
        })
        .collect();
    let mut segments = Vec::new();
    for (i, e) in edges.iter().enumerate() {
        let (mut from, mut from_row) = (e.from, e.from_row);
        for layer in rank[e.from] as usize + 1..rank[e.to] as usize {
            vertices.push(Vertex {
                layer,
                height: 1,
                width: 0,
                real: false,
                model: (e.from, 1 + i),
            });
            let dummy = vertices.len() - 1;
            segments.push(Segment {
                edge: i,
                from,
                from_row,
                to: dummy,
                to_row: 0,
            });
            (from, from_row) = (dummy, 0);
        }
        segments.push(Segment {
            edge: i,
            from,
            from_row,
            to: e.to,
            to_row: e.to_row,
        });
    }
    Ok((vertices, segments))
}

fn gap(a: &Vertex, b: &Vertex, spacing: &Spacing) -> i32 {
    if a.real && b.real { spacing.node } else { 1 }
}

/// Borde superior de cada vértice si la capa se apila en el orden dado.
fn stacked(order: &[usize], vertices: &[Vertex], spacing: &Spacing, top: &mut [i32]) {
    let mut y = 0;
    for (k, &v) in order.iter().enumerate() {
        if k > 0 {
            y += gap(&vertices[order[k - 1]], &vertices[v], spacing);
        }
        top[v] = y;
        y += vertices[v].height;
    }
}

/// Etapa 3: orden de cada capa.
fn order_layers(vertices: &[Vertex], segments: &[Segment]) -> Vec<Vec<usize>> {
    // La separación exacta no cambia el orden relativo; alcanza con una estimación.
    let spacing = Spacing { node: 2, layer: 0 };
    let count = vertices.iter().map(|v| v.layer + 1).max().unwrap_or(0);
    let mut layers: Vec<Vec<usize>> = vec![Vec::new(); count];
    for (i, v) in vertices.iter().enumerate() {
        layers[v.layer].push(i);
    }
    for layer in &mut layers {
        layer.sort_by_key(|&v| vertices[v].model);
    }
    let mut best = layers.clone();
    let mut best_crossings = crossings(&layers, vertices, segments);
    let mut top = vec![0; vertices.len()];
    for sweep in 0..24 {
        let forward = sweep % 2 == 0;
        let range: Vec<usize> = if forward {
            (1..count).collect()
        } else {
            (0..count.saturating_sub(1)).rev().collect()
        };
        for l in range {
            let fixed = if forward { l - 1 } else { l + 1 };
            stacked(&layers[fixed], vertices, &spacing, &mut top);
            stacked(&layers[l], vertices, &spacing, &mut top);
            // Borde superior deseado: el que alinea cada puerto con su vecino fijo.
            let mut sum = vec![(0i64, 0i64); vertices.len()];
            for s in segments {
                let (free, own_row, other, other_row) = if forward {
                    (s.to, s.to_row, s.from, s.from_row)
                } else {
                    (s.from, s.from_row, s.to, s.to_row)
                };
                if vertices[free].layer == l && vertices[other].layer == fixed {
                    sum[free].0 += (top[other] + other_row - own_row) as i64;
                    sum[free].1 += 1;
                }
            }
            for &v in &layers[l] {
                if sum[v].1 == 0 {
                    sum[v] = (top[v] as i64, 1);
                }
            }
            // Orden estable: los empates conservan el orden actual.
            layers[l].sort_by(|&a, &b| (sum[a].0 * sum[b].1).cmp(&(sum[b].0 * sum[a].1)));
        }
        let c = crossings(&layers, vertices, segments);
        if c < best_crossings {
            best_crossings = c;
            best = layers.clone();
        }
    }
    best
}

/// Cruces entre capas consecutivas: pares de tramos cuyos extremos (posición en la
/// capa, fila del puerto) quedan en orden opuesto. Una misma salida no cruza consigo.
fn crossings(layers: &[Vec<usize>], vertices: &[Vertex], segments: &[Segment]) -> usize {
    let mut pos = vec![0; vertices.len()];
    for layer in layers {
        for (k, &v) in layer.iter().enumerate() {
            pos[v] = k;
        }
    }
    // Por capa, los extremos (posición, fila) de cada tramo que sale de ella.
    type Ends = ((usize, i32), (usize, i32));
    let mut by_layer: Vec<Vec<Ends>> = vec![Vec::new(); layers.len()];
    for s in segments {
        by_layer[vertices[s.from].layer].push(((pos[s.from], s.from_row), (pos[s.to], s.to_row)));
    }
    let mut total = 0;
    for segs in &by_layer {
        for (i, a) in segs.iter().enumerate() {
            for b in &segs[i + 1..] {
                if (a.0 < b.0 && a.1 > b.1) || (a.0 > b.0 && a.1 < b.1) {
                    total += 1;
                }
            }
        }
    }
    total
}

/// Etapa 4: borde superior de cada vértice.
fn place_vertically(
    vertices: &[Vertex],
    segments: &[Segment],
    layers: &[Vec<usize>],
    spacing: &Spacing,
) -> Result<Vec<i32>> {
    let mut arcs = Vec::new();
    for layer in layers {
        for w in layer.windows(2) {
            let (a, b) = (&vertices[w[0]], &vertices[w[1]]);
            arcs.push(Arc {
                tail: w[0],
                head: w[1],
                delta: a.height + gap(a, b, spacing),
                weight: 0,
            });
        }
    }
    // Un nodo auxiliar por tramo, debajo de sus dos puertos: el costo es el desvío entre
    // ellos. Los tramos de cables largos pesan más, para que corran derechos.
    for (k, s) in segments.iter().enumerate() {
        let aux = vertices.len() + k;
        let weight = match (vertices[s.from].real, vertices[s.to].real) {
            (true, true) => 1,
            (false, false) => 8,
            _ => 2,
        };
        for (head, row) in [(s.from, s.from_row), (s.to, s.to_row)] {
            arcs.push(Arc {
                tail: aux,
                head,
                delta: -row,
                weight,
            });
        }
    }
    let Some(rank) = simplex::solve(vertices.len() + segments.len(), &arcs) else {
        return Err(LayoutError::Infeasible);
    };
    // Cada componente queda normalizada por separado; el auxiliar de un tramo puede
    // quedar arriba de sus nodos, así que se vuelve a normalizar sobre los vértices.
    let ys = &rank[..vertices.len()];
    let min = ys.iter().copied().min().unwrap_or(0);
    Ok(ys.iter().map(|y| y - min).collect())
}

/// Etapa 5: columnas de las capas, ruteo por canal y quiebres de cada cable.
fn route(
    nodes: &[Node],
    edges: &[Edge],
    vertices: &[Vertex],
    segments: &[Segment],
    layers: &[Vec<usize>],
    ys: &[i32],
    spacing: &Spacing,
) -> Placement {
    let widths: Vec<i32> = layers
        .iter()
        .map(|l| l.iter().map(|&v| vertices[v].width).max().unwrap_or(0))
        .collect();
    // Troncos de cada canal: los tramos que salen de un mismo puerto comparten uno.
    let mut trunks: Vec<Vec<Trunk>> = vec![Vec::new(); layers.len()];
    let mut trunk_of = vec![(0, 0); segments.len()];
    let mut rows: Vec<Vec<i32>> = vec![Vec::new(); layers.len()];
    for (k, s) in segments.iter().enumerate() {
        let channel = vertices[s.from].layer;
        let (src, dst) = (ys[s.from] + s.from_row, ys[s.to] + s.to_row);
        rows[channel].extend([src, dst]);
        let list = &mut trunks[channel];
        let t = match list.iter().position(|t| t.port == (s.from, s.from_row)) {
            Some(t) => t,
            None => {
                list.push(Trunk {
                    port: (s.from, s.from_row),
                    src,
                    targets: Vec::new(),
                });
                list.len() - 1
            }
        };
        list[t].targets.push(dst);
        trunk_of[k] = (channel, t);
    }
    let routes: Vec<routing::Channel> = trunks
        .iter()
        .zip(&rows)
        .map(|(t, r)| routing::assign(t, r))
        .collect();

    // Columna de cada capa: su ancho más el canal que la sigue.
    let mut xs = Vec::with_capacity(layers.len());
    let mut x = 0;
    let mut channel_start = Vec::with_capacity(layers.len());
    for (l, w) in widths.iter().enumerate() {
        xs.push(x);
        let columns = routes[l].columns;
        let width = spacing.layer.max(columns + 1);
        channel_start.push(x + w + (width - columns + 1) / 2);
        x += w + width;
    }

    let mut bends = vec![Vec::new(); edges.len()];
    for (k, s) in segments.iter().enumerate() {
        let (channel, t) = trunk_of[k];
        let (src, dst) = (ys[s.from] + s.from_row, ys[s.to] + s.to_row);
        let col = |c: i32| channel_start[channel] + c;
        let pts = &mut bends[s.edge];
        match routes[channel].routes[t] {
            _ if src == dst => {}
            Route::Trunk(c) => pts.extend([Pos { x: col(c), y: src }, Pos { x: col(c), y: dst }]),
            Route::Dogleg { first, row, second } => pts.extend([
                Pos {
                    x: col(first),
                    y: src,
                },
                Pos {
                    x: col(first),
                    y: row,
                },
                Pos {
                    x: col(second),
                    y: row,
                },
                Pos {
                    x: col(second),
                    y: dst,
                },
            ]),
        }
    }
    Placement {
        nodes: (0..nodes.len())
            .map(|i| Pos {
                x: xs[vertices[i].layer],
                y: ys[i],
            })
            .collect(),
        bends,
    }
}

#[cfg(test)]
mod tests;
