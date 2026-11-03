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

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use crate::graph::{OnCycle, topo_order as topo_order_of};
use crate::iec::{sanitize, unique_name};
use crate::model::{
    Diagnostics, Level, Param, PartitionSpec, PinRef, Section, Stage, VarSpec, Wire,
};
use crate::types::TypedVar;

const STAGE: Stage = Stage::Partition;

/// Orden topológico de `blocks` según los cables internos; empates por orden del modelo.
/// Los bloques en ciclos se agregan al final en orden del modelo.
fn topo_order(section: &Section, blocks: &[usize]) -> Vec<usize> {
    // `blocks` está en orden del modelo: los índices locales conservan ese orden.
    let local: HashMap<usize, usize> = blocks.iter().enumerate().map(|(i, &b)| (b, i)).collect();
    let edges = section.wires.iter().filter_map(|w| {
        let (from, to) = (*local.get(&w.from.block)?, *local.get(&w.to.block)?);
        (from != to).then_some((from, to))
    });
    let mut order = topo_order_of(blocks.len(), edges, OnCycle::Stop);
    let mut done = vec![false; blocks.len()];
    for &i in &order {
        done[i] = true;
    }
    order.extend((0..blocks.len()).filter(|&i| !done[i]));
    order.into_iter().map(|i| blocks[i]).collect()
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
        chunk.sort_unstable();
        chunks.push(chunk);
        start = c;
    }
    chunks
}

pub(crate) struct Partition {
    /// Secciones en orden de ejecución: subgrafos ya resueltos y tipados, con los cables
    /// que cruzan secciones reemplazados por variables.
    pub sections: Vec<Section>,
    /// Variables creadas para esos cables, con su tipo y un comentario de enlace.
    pub link_vars: Vec<TypedVar>,
}

/// Divide el programa resuelto y tipado (`types::infer` ya corrió: los pines tienen su
/// tipo concreto). `vars` son las variables del programa, para no repetir sus nombres.
pub(crate) fn partition(
    section: &Section,
    cfg: &PartitionSpec,
    vars: &[TypedVar],
    diags: &mut Diagnostics,
) -> Partition {
    // Etapa 4: asignación de bloques a secciones.
    let assignment = Assignment::new(section, cfg, diags);
    check_cohesion(section, &assignment, diags);
    let mut parts = assignment.sections(section);
    let link_vars = link_across(section, &assignment, &mut parts, vars, diags);
    // Etapa 5: orden de las secciones.
    let order = execution_order(&parts, &assignment.names, diags);
    let mut parts: Vec<Option<Section>> = parts.into_iter().map(Some).collect();
    Partition {
        sections: order
            .iter()
            .map(|&s| {
                parts[s]
                    .take()
                    .expect("cada sección aparece una vez en el orden")
            })
            .collect(),
        link_vars,
    }
}

/// A qué sección va cada bloque, y con qué índice dentro de ella.
struct Assignment {
    /// Nombre de cada sección, ya sin repetidos.
    names: Vec<String>,
    /// Bloques de cada sección, en orden del modelo.
    groups: Vec<Vec<usize>>,
    /// Sección de cada bloque del programa.
    sec_of: Vec<usize>,
    /// Índice de cada bloque dentro de su sección.
    local: Vec<usize>,
}

impl Assignment {
    fn new(section: &Section, cfg: &PartitionSpec, diags: &mut Diagnostics) -> Self {
        let (mut names, groups) = subdivide(section, cfg, diags);
        dedupe_names(&mut names, diags);
        let mut sec_of = vec![0; section.blocks.len()];
        let mut local = vec![0; section.blocks.len()];
        for (s, blocks) in groups.iter().enumerate() {
            for (i, &b) in blocks.iter().enumerate() {
                sec_of[b] = s;
                local[b] = i;
            }
        }
        Self {
            names,
            groups,
            sec_of,
            local,
        }
    }

    /// El pin `r` del programa, con el bloque numerado dentro de su sección.
    fn to_local(&self, r: PinRef) -> PinRef {
        PinRef {
            block: self.local[r.block],
            pin: r.pin,
        }
    }

    /// Secciones con sus bloques y comentarios, todavía sin cables.
    fn sections(&self, section: &Section) -> Vec<Section> {
        self.groups
            .iter()
            .enumerate()
            .map(|(s, blocks)| Section {
                name: self.names[s].clone(),
                blocks: blocks.iter().map(|&b| section.blocks[b].clone()).collect(),
                wires: Vec::new(),
                execution_dependencies: Vec::new(),
                generated_vars: Vec::new(),
                comments: section
                    .comments
                    .iter()
                    .filter(|(b, _)| self.sec_of[*b] == s)
                    .map(|(b, text)| (self.local[*b], text.clone()))
                    .collect(),
            })
            .collect()
    }
}

/// Agrupa por equipo (y etapa, si se pide) y subdivide los grupos que pasan el tope.
/// Devuelve el nombre y los bloques de cada sección; los nombres pueden repetirse.
fn subdivide(
    section: &Section,
    cfg: &PartitionSpec,
    diags: &mut Diagnostics,
) -> (Vec<String>, Vec<Vec<usize>>) {
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
    let max = cfg.max_blocks.max(1);
    let mut names = Vec::new();
    let mut groups = Vec::new();
    for ((group, stage), blocks) in keys.into_iter().zip(members) {
        let key = sanitize(&match stage {
            Some(stage) => format!("{group}_{stage}"),
            None => group,
        });
        if blocks.len() <= max {
            names.push(key);
            groups.push(blocks);
            continue;
        }
        let chunks = split(section, &topo_order(section, &blocks), max);
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
    }
    (names, groups)
}

/// Los sufijos de una subdivisión pueden coincidir con otro grupo. Reservar todos los
/// candidatos evita además que un renombre ocupe el nombre natural de otro.
fn dedupe_names(names: &mut [String], diags: &mut Diagnostics) {
    let reserved: HashSet<String> = names.iter().map(|n| n.to_ascii_uppercase()).collect();
    let mut used = HashSet::new();
    for name in names {
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
}

/// Cohesión: avisa de un bloque más conectado con otra sección que con la suya.
fn check_cohesion(section: &Section, assignment: &Assignment, diags: &mut Diagnostics) {
    let Assignment { names, sec_of, .. } = assignment;
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
}

/// Reparte los cables entre las secciones: los internos quedan como cables y cada uno que
/// cruza pasa a una variable que escribe la sección de origen y lee la de destino.
/// Devuelve las variables creadas.
fn link_across(
    section: &Section,
    assignment: &Assignment,
    parts: &mut [Section],
    vars: &[TypedVar],
    diags: &mut Diagnostics,
) -> Vec<TypedVar> {
    let Assignment { names, sec_of, .. } = assignment;
    let mut used_names: HashSet<String> =
        vars.iter().map(|v| v.name.to_ascii_uppercase()).collect();
    used_names.extend(
        section
            .blocks
            .iter()
            .filter_map(|b| b.instance.as_ref().map(|i| i.to_ascii_uppercase())),
    );
    let mut cross_var: HashMap<PinRef, String> = HashMap::new();
    let mut link_vars: Vec<TypedVar> = Vec::new();
    for w in &section.wires {
        let (from_sec, to_sec) = (sec_of[w.from.block], sec_of[w.to.block]);
        if from_sec == to_sec {
            parts[from_sec].wires.push(Wire {
                from: assignment.to_local(w.from),
                to: assignment.to_local(w.to),
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
                    unique_name(&base, &mut used_names)
                });
                if existing.is_none() {
                    parts[from_sec]
                        .pin_mut(assignment.to_local(w.from))
                        .param_out = Some(Param::Var(name.clone()));
                    let comment = format!("Enlace {} -> {}", names[from_sec], names[to_sec]);
                    match link_var(section, *w, &name, comment) {
                        Some(var) => link_vars.push(var),
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
        parts[to_sec].pin_mut(assignment.to_local(w.to)).param_in = Some(Param::Var(var));
    }
    link_vars
}

/// Declaración de la variable `name` que reemplaza al cable `w`, con el tipo inferido de
/// la salida (o, si falta, de la entrada). `None` si ninguno de los dos se pudo deducir.
fn link_var(section: &Section, w: Wire, name: &str, comment: String) -> Option<TypedVar> {
    let ty = section
        .pin(w.from)
        .inferred
        .clone()
        .or_else(|| section.pin(w.to).inferred.clone())?;
    Some(TypedVar {
        name: name.to_string(),
        ty: ty.clone(),
        decl: Some(VarSpec {
            name: name.to_string(),
            ty,
            address: None,
            comment: Some(comment),
            init: None,
        }),
    })
}

/// Orden de ejecución: quien escribe una variable va antes que quien la lee. Un ciclo
/// se rompe con la primera sección pendiente en orden del modelo y se avisa.
fn execution_order(parts: &[Section], names: &[String], diags: &mut Diagnostics) -> Vec<usize> {
    // Lecturas y escrituras de variables por sección.
    let mut reads: Vec<BTreeSet<String>> = Vec::new();
    let mut writes: Vec<BTreeSet<String>> = Vec::new();
    for part in parts {
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
    // `BTreeMap`: los avisos salen en orden alfabético, igual en cada ejecución.
    let mut writers: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
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
    let order = topo_order_of(parts.len(), deps.keys().copied(), OnCycle::Break);
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
    order
}
