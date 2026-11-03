//! Etapas 4 y 5: partición del programa en secciones y orden de las secciones.
//!
//! Criterios, en orden: equipo (`group`), etapa del flujo de señal (`stage`, opcional) y
//! tamaño máximo (se subdivide siguiendo el orden topológico, así los cortes van hacia
//! adelante). Cada cable que cruza secciones se reemplaza por una variable. Un chequeo de
//! cohesión avisa cuando un bloque está más conectado con otra sección que con la suya.
//!
//! El orden de las secciones respeta las dependencias (quién escribe lo que otro lee).
//! Lo que no se puede respetar (ciclos entre secciones) se reporta: esa lectura toma el
//! valor del ciclo de scan anterior.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, BinaryHeap, HashMap, HashSet};

use crate::model::*;
use crate::normalize::sanitize;
use crate::types::TypedVar;

const STAGE: &str = "partition";

/// Orden topológico de `blocks` según los cables internos; empates por orden del modelo.
/// Los bloques en ciclos se agregan al final en orden del modelo.
fn topo_order(section: &Section, blocks: &[usize]) -> Vec<usize> {
    let set: HashSet<usize> = blocks.iter().copied().collect();
    let mut indeg: HashMap<usize, usize> = blocks.iter().map(|&b| (b, 0)).collect();
    let mut adj: HashMap<usize, Vec<usize>> = HashMap::new();
    for w in &section.wires {
        if set.contains(&w.from.block) && set.contains(&w.to.block) && w.from.block != w.to.block {
            adj.entry(w.from.block).or_default().push(w.to.block);
            *indeg.get_mut(&w.to.block).unwrap() += 1;
        }
    }
    let mut ready: BinaryHeap<Reverse<usize>> = blocks
        .iter()
        .filter(|b| indeg[b] == 0)
        .map(|&b| Reverse(b))
        .collect();
    let mut order = Vec::new();
    while let Some(Reverse(b)) = ready.pop() {
        order.push(b);
        for &n in adj.get(&b).into_iter().flatten() {
            let d = indeg.get_mut(&n).unwrap();
            *d -= 1;
            if *d == 0 {
                ready.push(Reverse(n));
            }
        }
    }
    let done: HashSet<usize> = order.iter().copied().collect();
    let mut rest: Vec<usize> = blocks
        .iter()
        .copied()
        .filter(|b| !done.contains(b))
        .collect();
    rest.sort();
    order.extend(rest);
    order
}

/// Divide una secuencia topológica en tramos de a lo sumo `max`, eligiendo cada corte
/// (cerca del punto ideal) donde menos cables lo atraviesan.
fn split(section: &Section, order: &[usize], max: usize) -> Vec<Vec<usize>> {
    let n = order.len();
    let k = n.div_ceil(max);
    let pos: HashMap<usize, usize> = order.iter().enumerate().map(|(i, &b)| (b, i)).collect();
    let crossing = |p: usize| {
        section
            .wires
            .iter()
            .filter(|w| match (pos.get(&w.from.block), pos.get(&w.to.block)) {
                (Some(&a), Some(&b)) => (a < p) != (b < p),
                _ => false,
            })
            .count()
    };
    let mut cuts = Vec::new();
    let mut prev = 0;
    for i in 1..k {
        let ideal = i * n / k;
        let lo = ideal
            .saturating_sub(max / 4)
            .max(prev + 1)
            .max(n.saturating_sub(max * (k - i)));
        let hi = ideal.min(prev + max);
        let best = (lo..=hi.max(lo))
            .min_by_key(|&p| (crossing(p), p.abs_diff(ideal)))
            .unwrap_or(ideal);
        cuts.push(best);
        prev = best;
    }
    let mut chunks = Vec::new();
    let mut start = 0;
    for c in cuts.into_iter().chain([n]) {
        let mut chunk = order[start..c].to_vec();
        chunk.sort();
        chunks.push(chunk);
        start = c;
    }
    chunks
}

pub struct Partition {
    /// Secciones en orden de ejecución: subgrafos ya resueltos y tipados, con los cables
    /// que cruzan secciones reemplazados por variables.
    pub sections: Vec<Section>,
    /// Variables creadas para esos cables, con su tipo y un comentario de enlace.
    pub link_vars: Vec<TypedVar>,
}

/// Divide el programa resuelto y tipado (`types::infer` ya corrió: los pines tienen su
/// tipo concreto). `vars` son las variables del programa, para no repetir sus nombres.
pub fn partition(
    section: &Section,
    cfg: &PartitionSpec,
    vars: &[TypedVar],
    diags: &mut Diagnostics,
) -> Partition {
    // --- Etapa 4: asignación de bloques a secciones -------------------------------
    let mut keys: Vec<(String, Option<String>)> = Vec::new();
    let mut members: Vec<Vec<usize>> = Vec::new();
    for (i, b) in section.blocks.iter().enumerate() {
        // La identidad se conserva antes de concatenar y sanear el nombre de archivo.
        let key = (
            b.group.clone().unwrap_or_else(|| section.name.clone()),
            b.stage.clone().filter(|_| cfg.by_stage),
        );
        match keys.iter().position(|k| *k == key) {
            Some(k) => members[k].push(i),
            None => {
                keys.push(key);
                members.push(vec![i]);
            }
        }
    }
    let mut names = Vec::new();
    let mut groups: Vec<Vec<usize>> = Vec::new();
    for ((group, stage), blocks) in keys.into_iter().zip(members) {
        let key = sanitize(&match stage {
            Some(stage) => format!("{group}_{stage}"),
            None => group,
        });
        if blocks.len() > cfg.max_blocks.max(1) {
            let chunks = split(
                section,
                &topo_order(section, &blocks),
                cfg.max_blocks.max(1),
            );
            diags.push(
                Level::Info,
                STAGE,
                format!(
                    "{key}: {} bloques > {}, dividida en {} secciones",
                    blocks.len(),
                    cfg.max_blocks,
                    chunks.len()
                ),
            );
            for (i, c) in chunks.into_iter().enumerate() {
                names.push(format!("{key}_{}", i + 1));
                groups.push(c);
            }
        } else {
            names.push(key);
            groups.push(blocks);
        }
    }
    // Los sufijos de una subdivisión pueden coincidir con otro grupo. Reservar todos
    // los candidatos evita además que un renombre ocupe el nombre natural de otro.
    let reserved: HashSet<String> = names.iter().map(|n| n.to_ascii_uppercase()).collect();
    let mut used = HashSet::new();
    for name in &mut names {
        if used.insert(name.to_ascii_uppercase()) {
            continue;
        }
        let base = name.clone();
        let mut suffix = 2;
        loop {
            let candidate = format!("{base}_{suffix}");
            let key = candidate.to_ascii_uppercase();
            if !reserved.contains(&key) && used.insert(key) {
                *name = candidate;
                break;
            }
            suffix += 1;
        }
        diags.push(
            Level::Info,
            STAGE,
            format!("nombre de sección {base} repetido; se usa {name}"),
        );
    }
    let mut sec_of = vec![0; section.blocks.len()];
    for (s, blocks) in groups.iter().enumerate() {
        for &b in blocks {
            sec_of[b] = s;
        }
    }

    // Cohesión: un bloque más conectado con otra sección que con la suya.
    for (b, blk) in section.blocks.iter().enumerate() {
        let mut count: BTreeMap<usize, usize> = BTreeMap::new();
        for w in section
            .wires
            .iter()
            .filter(|w| w.from.block == b || w.to.block == b)
        {
            let other = if w.from.block == b {
                w.to.block
            } else {
                w.from.block
            };
            if other != b {
                *count.entry(sec_of[other]).or_default() += 1;
            }
        }
        let own = count.get(&sec_of[b]).copied().unwrap_or(0);
        if let Some((&s, &n)) = count
            .iter()
            .filter(|e| *e.0 != sec_of[b])
            .max_by_key(|e| *e.1)
            && n >= 2
            && n > own
        {
            diags.push(
                    Level::Warning,
                    STAGE,
                    format!(
                        "{} está en {} pero tiene {n} conexiones con {} y {own} con la suya; revisar su `group`",
                        blk.id, names[sec_of[b]], names[s]
                    ),
                );
        }
    }

    // --- Armado de las secciones ---------------------------------------------------
    // Los bloques conservan el orden del modelo dentro de su sección.
    let mut local = vec![0; section.blocks.len()];
    for blocks in &groups {
        for (i, &b) in blocks.iter().enumerate() {
            local[b] = i;
        }
    }
    let to_local = |r: PinRef| PinRef {
        block: local[r.block],
        pin: r.pin,
    };
    let mut parts: Vec<Section> = groups
        .iter()
        .enumerate()
        .map(|(s, blocks)| Section {
            name: names[s].clone(),
            blocks: blocks.iter().map(|&b| section.blocks[b].clone()).collect(),
            wires: Vec::new(),
            execution_dependencies: Vec::new(),
            generated_vars: Vec::new(),
            comments: section
                .comments
                .iter()
                .filter(|(b, _)| sec_of[*b] == s)
                .map(|(b, text)| (local[*b], text.clone()))
                .collect(),
        })
        .collect();

    // --- Cables entre secciones -> variables --------------------------------------
    let mut used_names: HashSet<String> =
        vars.iter().map(|v| v.name.to_ascii_uppercase()).collect();
    for b in &section.blocks {
        if let Some(i) = &b.instance {
            used_names.insert(i.to_ascii_uppercase());
        }
    }
    let mut cross_var: HashMap<PinRef, String> = HashMap::new();
    let mut link_vars: Vec<TypedVar> = Vec::new();
    for w in &section.wires {
        let (from_sec, to_sec) = (sec_of[w.from.block], sec_of[w.to.block]);
        if from_sec == to_sec {
            parts[from_sec].wires.push(Wire {
                from: to_local(w.from),
                to: to_local(w.to),
            });
            continue;
        }
        let var = match cross_var.get(&w.from) {
            Some(v) => v.clone(),
            None => {
                // Si la salida ya escribe una variable, se reutiliza.
                let existing = section
                    .pin(w.from)
                    .param_out
                    .as_ref()
                    .map(|p| p.text().to_string());
                let name = existing.clone().unwrap_or_else(|| {
                    let base = sanitize(&format!(
                        "{}_{}",
                        section.blocks[w.from.block].label(),
                        section.pin(w.from).name
                    ));
                    let mut name = base.clone();
                    let mut n = 2;
                    while used_names.contains(&name.to_ascii_uppercase()) {
                        name = format!("{base}_{n}");
                        n += 1;
                    }
                    name
                });
                if existing.is_none() {
                    used_names.insert(name.to_ascii_uppercase());
                    parts[from_sec].pin_mut(to_local(w.from)).param_out =
                        Some(Param::Var(name.clone()));
                    let ty = section
                        .pin(w.from)
                        .inferred
                        .clone()
                        .or_else(|| section.pin(w.to).inferred.clone());
                    match ty {
                        Some(ty) => link_vars.push(TypedVar {
                            name: name.clone(),
                            ty: ty.clone(),
                            decl: Some(VarSpec {
                                name: name.clone(),
                                ty,
                                address: None,
                                comment: Some(format!(
                                    "Enlace {} -> {}",
                                    names[from_sec], names[to_sec]
                                )),
                                init: None,
                            }),
                        }),
                        None => diags.push(
                            Level::Error,
                            STAGE,
                            format!(
                                "{name}: no se pudo deducir el tipo del enlace entre secciones"
                            ),
                        ),
                    }
                }
                cross_var.insert(w.from, name.clone());
                name
            }
        };
        diags.push(
            Level::Info,
            STAGE,
            format!(
                "{} -> {} cruza de {} a {}: variable {var}",
                section.pin_path(w.from),
                section.pin_path(w.to),
                names[from_sec],
                names[to_sec]
            ),
        );
        parts[to_sec].pin_mut(to_local(w.to)).param_in = Some(Param::Var(var));
    }

    // Lecturas y escrituras de variables por sección, para ordenarlas.
    let mut reads: Vec<BTreeSet<String>> = Vec::new();
    let mut writes: Vec<BTreeSet<String>> = Vec::new();
    for part in &parts {
        let (mut r, mut w) = (BTreeSet::new(), BTreeSet::new());
        for p in part.blocks.iter().flat_map(|b| &b.pins) {
            if let Some(Param::Var(v)) = &p.param_in {
                r.insert(v.to_ascii_uppercase());
            }
            if let Some(Param::Var(v)) = &p.param_out {
                w.insert(v.to_ascii_uppercase());
            }
        }
        reads.push(r);
        writes.push(w);
    }

    // --- Etapa 5: orden de las secciones -------------------------------------------
    let n = parts.len();
    let mut writers: HashMap<&str, Vec<usize>> = HashMap::new();
    for (s, w) in writes.iter().enumerate() {
        for v in w {
            writers.entry(v).or_default().push(s);
        }
    }
    for (v, ws) in &writers {
        if ws.len() > 1 {
            let list: Vec<&str> = ws.iter().map(|&s| names[s].as_str()).collect();
            diags.push(
                Level::Warning,
                STAGE,
                format!("{v} se escribe en varias secciones: {}", list.join(", ")),
            );
        }
    }
    let mut deps: BTreeMap<(usize, usize), Vec<String>> = BTreeMap::new();
    for (s, r) in reads.iter().enumerate() {
        for v in r {
            for &w in writers.get(v.as_str()).into_iter().flatten() {
                if w != s {
                    deps.entry((w, s)).or_default().push(v.clone());
                }
            }
        }
    }
    let mut indeg = vec![0; n];
    for &(_, to) in deps.keys() {
        indeg[to] += 1;
    }
    let mut ready: BinaryHeap<Reverse<usize>> =
        (0..n).filter(|&s| indeg[s] == 0).map(Reverse).collect();
    let mut order = Vec::new();
    let mut placed = vec![false; n];
    while order.len() < n {
        let s = match ready.pop() {
            Some(Reverse(s)) => s,
            // Ciclo: se rompe con la primera sección pendiente en orden del modelo.
            None => (0..n).find(|&s| !placed[s]).unwrap(),
        };
        if placed[s] {
            continue;
        }
        placed[s] = true;
        order.push(s);
        for (&(from, to), _) in deps.range((s, 0)..(s + 1, 0)) {
            debug_assert_eq!(from, s);
            indeg[to] -= 1;
            if indeg[to] == 0 && !placed[to] {
                ready.push(Reverse(to));
            }
        }
    }
    let pos: HashMap<usize, usize> = order.iter().enumerate().map(|(i, &s)| (s, i)).collect();
    for ((from, to), vs) in &deps {
        if pos[from] > pos[to] {
            diags.push(
                Level::Warning,
                STAGE,
                format!(
                    "dependencia circular: {} lee {} de {}, que se ejecuta después (valor del scan anterior)",
                    names[*to],
                    vs.join(", "),
                    names[*from]
                ),
            );
        }
    }
    let ordered: Vec<&str> = order.iter().map(|&s| names[s].as_str()).collect();
    diags.push(
        Level::Info,
        STAGE,
        format!("orden de secciones: {}", ordered.join(" -> ")),
    );

    let mut parts: Vec<Option<Section>> = parts.into_iter().map(Some).collect();
    Partition {
        sections: order.iter().map(|&s| parts[s].take().unwrap()).collect(),
        link_vars,
    }
}
