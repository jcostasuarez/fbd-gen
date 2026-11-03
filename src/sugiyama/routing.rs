//! Ruteo ortogonal de un canal entre dos capas.
//!
//! Cada salida tiene un tronco vertical que comparten todos sus cables del canal. El
//! tronco llega desde la izquierda por la fila de la salida y sale hacia la derecha por
//! la fila de cada destino. Dos troncos cuyos tramos verticales se tocan van en columnas
//! distintas, y el orden de las columnas evita superponer cables distintos:
//!
//! - si la fila de llegada de A es una fila de salida de B, A va a la izquierda de B
//!   (si no, los dos horizontales compartirían la fila entre ambas columnas);
//! - el resto de los pares se ordena por la cantidad de cruces.
//!
//! Si esas reglas forman un ciclo (dos salidas que intercambian filas), uno de los troncos
//! se parte: la llegada baja por una columna hasta una fila libre, cruza y los destinos
//! salen de otra columna. La mitad de la llegada no tiene salidas y la otra no tiene
//! llegada, así que ninguna regla obligatoria las vuelve a meter en un ciclo.

/// Tronco de una salida en el canal.
#[derive(Debug, Clone)]
pub struct Trunk {
    /// Vértice y fila de la salida: identifica el tronco.
    pub port: (usize, i32),
    pub src: i32,
    pub targets: Vec<i32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    /// Columna del tronco (relativa al inicio del canal).
    Trunk(i32),
    /// Tronco partido: la llegada baja por `first` hasta `row`, cruza y los destinos
    /// salen de `second`.
    Dogleg { first: i32, row: i32, second: i32 },
}

#[derive(Debug, Clone)]
pub struct Channel {
    pub columns: i32,
    pub routes: Vec<Route>,
}

/// Tramo vertical a ubicar en una columna.
struct Piece {
    owner: usize,
    /// Fila por la que llega desde el borde izquierdo del canal.
    src: Option<i32>,
    /// Filas por las que sale hacia el borde derecho.
    dst: Vec<i32>,
    lo: i32,
    hi: i32,
}

impl Piece {
    fn new(owner: usize, src: Option<i32>, dst: Vec<i32>, also: i32) -> Self {
        let rows = src.iter().chain(&dst).copied().chain([also]);
        let (lo, hi) = rows.fold((i32::MAX, i32::MIN), |(lo, hi), r| (lo.min(r), hi.max(r)));
        Self {
            owner,
            src,
            dst,
            lo,
            hi,
        }
    }

    /// Los tramos verticales se tocan (o quedan pegados) si comparten columna.
    fn conflicts(&self, o: &Piece) -> bool {
        self.lo <= o.hi + 1 && o.lo <= self.hi + 1
    }

    /// `self` tiene que ir a la izquierda de `o`.
    fn must_precede(&self, o: &Piece) -> bool {
        self.owner != o.owner && self.src.is_some_and(|s| o.dst.contains(&s))
    }

    /// Cruces si `self` va a la izquierda de `o`: sus salidas atraviesan el tramo de `o`
    /// y la llegada de `o` atraviesa el de `self`.
    fn crossings_left_of(&self, o: &Piece) -> usize {
        let inside = |r: i32, p: &Piece| p.lo < r && r < p.hi;
        self.dst.iter().filter(|&&t| inside(t, o)).count()
            + o.src.is_some_and(|s| inside(s, self)) as usize
    }
}

fn pieces(trunks: &[Trunk], split: &[Option<i32>]) -> Vec<Piece> {
    let mut out = Vec::new();
    for (i, t) in trunks.iter().enumerate() {
        match split[i] {
            None => out.push(Piece::new(i, Some(t.src), t.targets.clone(), t.src)),
            Some(row) => {
                out.push(Piece::new(i, Some(t.src), Vec::new(), row));
                out.push(Piece::new(i, None, t.targets.clone(), row));
            }
        }
    }
    // Un tronco recto (todos sus destinos en su fila) no ocupa columna.
    out.retain(|p| p.lo < p.hi);
    out
}

fn reaches(adj: &[Vec<usize>], from: usize, to: usize) -> bool {
    let mut seen = vec![false; adj.len()];
    let mut stack = vec![from];
    while let Some(v) = stack.pop() {
        if v == to {
            return true;
        }
        if !std::mem::replace(&mut seen[v], true) {
            stack.extend(&adj[v]);
        }
    }
    false
}

/// Fila libre del canal para unir las dos mitades de un cable partido: entre sus filas
/// si se puede, lo más cerca posible del medio.
fn free_row(src: i32, dst: i32, used: &[i32]) -> i32 {
    let (lo, hi) = (src.min(dst), src.max(dst));
    let mid = lo + (hi - lo) / 2;
    let free = |r: &i32| !used.contains(r);
    let inside = (0..=hi - lo).flat_map(|d| [mid - d, mid + d]);
    if let Some(r) = inside.filter(|r| lo < *r && *r < hi).find(free) {
        return r;
    }
    // Afuera del intervalo siempre hay una fila libre: `used` es finito.
    let mut d = 1;
    loop {
        if let Some(r) = [hi + d, lo - d].into_iter().find(free) {
            return r;
        }
        d += 1;
    }
}

/// Columnas del canal. `rows` son las filas de todos los extremos del canal (también
/// los cables rectos), que una fila de unión no puede usar.
pub fn assign(trunks: &[Trunk], rows: &[i32]) -> Channel {
    let mut used = rows.to_vec();
    let mut split = vec![None; trunks.len()];
    let mut ps = pieces(trunks, &split);
    for _ in 0..=trunks.len() {
        let mut hard = vec![Vec::new(); ps.len()];
        for (i, p) in ps.iter().enumerate() {
            for (j, q) in ps.iter().enumerate() {
                if i != j && p.must_precede(q) {
                    hard[i].push(j);
                }
            }
        }
        // Se parte el primer tronco que esté en un ciclo.
        let in_cycle = |i: usize| hard[i].iter().any(|&j| reaches(&hard, j, i));
        let Some(owner) = (0..ps.len()).find(|&i| in_cycle(i)).map(|i| ps[i].owner) else {
            break;
        };
        let t = &trunks[owner];
        // Un tronco siempre tiene destinos; sin ellos, la fila de partida sirve igual.
        let nearest = t
            .targets
            .iter()
            .copied()
            .min_by_key(|&r| (r.abs_diff(t.src), r))
            .unwrap_or(t.src);
        let row = free_row(t.src, nearest, &used);
        used.push(row);
        split[owner] = Some(row);
        ps = pieces(trunks, &split);
    }

    // Orden de columnas: primero las reglas obligatorias, después los cruces, de la
    // decisión más clara a la más pareja. Nada que cierre un ciclo.
    let mut adj = vec![Vec::new(); ps.len()];
    let add = |adj: &mut Vec<Vec<usize>>, a: usize, b: usize| {
        if !reaches(adj, b, a) {
            adj[a].push(b);
        }
    };
    let mut soft = Vec::new();
    for i in 0..ps.len() {
        for j in i + 1..ps.len() {
            let (p, q) = (&ps[i], &ps[j]);
            if !p.conflicts(q) {
                continue;
            }
            if p.must_precede(q) {
                add(&mut adj, i, j);
            } else if q.must_precede(p) {
                add(&mut adj, j, i);
            } else {
                let (pq, qp) = (p.crossings_left_of(q), q.crossings_left_of(p));
                soft.push((pq.abs_diff(qp), i, j, pq <= qp));
            }
        }
    }
    soft.sort_by_key(|&(d, i, j, _)| (std::cmp::Reverse(d), i, j));
    for (_, i, j, i_first) in soft {
        if reaches(&adj, i, j) || reaches(&adj, j, i) {
            continue;
        }
        if i_first {
            add(&mut adj, i, j);
        } else {
            add(&mut adj, j, i);
        }
    }
    // Pares en conflicto que todavía no tienen orden.
    for i in 0..ps.len() {
        for j in i + 1..ps.len() {
            if ps[i].conflicts(&ps[j]) && !reaches(&adj, i, j) && !reaches(&adj, j, i) {
                add(&mut adj, i, j);
            }
        }
    }

    // Columna = camino más largo desde la izquierda.
    let mut column = vec![0i32; ps.len()];
    let mut indeg = vec![0; ps.len()];
    for next in &adj {
        for &j in next {
            indeg[j] += 1;
        }
    }
    let mut ready: Vec<usize> = (0..ps.len()).filter(|&i| indeg[i] == 0).collect();
    while let Some(i) = ready.pop() {
        for &j in &adj[i] {
            column[j] = column[j].max(column[i] + 1);
            indeg[j] -= 1;
            if indeg[j] == 0 {
                ready.push(j);
            }
        }
    }

    let mut routes = vec![Route::Trunk(0); trunks.len()];
    for (owner, row) in split.iter().enumerate() {
        let cols: Vec<i32> = ps
            .iter()
            .zip(&column)
            .filter(|(p, _)| p.owner == owner)
            .map(|(_, &c)| c)
            .collect();
        routes[owner] = match (row, cols.as_slice()) {
            (Some(row), [first, second]) => Route::Dogleg {
                first: *first,
                row: *row,
                second: *second,
            },
            (_, [c, ..]) => Route::Trunk(*c),
            (_, []) => Route::Trunk(0),
        };
    }
    Channel {
        columns: column.iter().map(|c| c + 1).max().unwrap_or(0),
        routes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trunk(src: i32, targets: &[i32]) -> Trunk {
        Trunk {
            port: (src as usize, 0),
            src,
            targets: targets.to_vec(),
        }
    }

    #[test]
    fn fanout_shares_one_column_and_straight_wires_take_none() {
        let c = assign(&[trunk(5, &[2, 5, 9]), trunk(12, &[12])], &[5, 2, 9, 12]);
        assert_eq!(c.columns, 1);
        assert_eq!(c.routes[0], Route::Trunk(0));
    }

    #[test]
    fn arriving_row_that_another_trunk_leaves_by_goes_left() {
        // A sale por la fila 9 y B llega por la 9: B a la izquierda, o sus horizontales
        // compartirían la fila entre las dos columnas.
        let c = assign(&[trunk(4, &[9]), trunk(9, &[14])], &[4, 9, 14]);
        assert_eq!(c.columns, 2);
        assert_eq!(c.routes, [Route::Trunk(1), Route::Trunk(0)]);
    }

    #[test]
    fn swapped_fanouts_are_resolved_with_a_dogleg() {
        let c = assign(
            &[trunk(16, &[25, 4]), trunk(25, &[16, 27])],
            &[16, 25, 4, 27],
        );
        assert!(
            c.routes.iter().any(|r| matches!(r, Route::Dogleg { .. })),
            "{:?}",
            c.routes
        );
    }

    #[test]
    fn swapped_rows_are_resolved_with_a_dogleg() {
        // A: 5 -> 9 y B: 9 -> 5. Cualquier orden superpone; uno se parte.
        let c = assign(&[trunk(5, &[9]), trunk(9, &[5])], &[5, 9]);
        let doglegs: Vec<&Route> = c
            .routes
            .iter()
            .filter(|r| matches!(r, Route::Dogleg { .. }))
            .collect();
        assert_eq!(doglegs.len(), 1, "{:?}", c.routes);
        if let Route::Dogleg { row, .. } = doglegs[0] {
            assert!(5 < *row && *row < 9);
        }
        assert_eq!(c.columns, 3);
    }
}
