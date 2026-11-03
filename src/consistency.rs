//! Etapa 3: control de consistencia entre equipos (`group`) por hash de subgrafos.
//!
//! Cada equipo se reduce a un grafo etiquetado (tipo de bloque, pines, cables internos,
//! variables con el nombre del equipo reemplazado por `{G}`) y se le calcula un hash
//! Weisfeiler-Lehman. Los equipos con el mismo hash comparten un típico. Un equipo único
//! que se parece mucho a un típico compartido casi siempre es un error de generación, no
//! una variante intencional: se reporta con la diferencia concreta. Dentro de un típico,
//! los literales que difieren de la mayoría (`T#5s` contra `T#50s`) se reportan aparte.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::hash::{Hash, Hasher};

use crate::model::{Diagnostics, Level, Param, PinRef, Section, Stage};

const STAGE: Stage = Stage::Consistency;

/// Opciones del control de consistencia entre equipos (`group`).
#[derive(Debug, Clone, PartialEq)]
pub struct ConsistencyConfig {
    /// Similitud mínima (Jaccard sobre rasgos WL) para considerar que un equipo debería
    /// seguir a un típico.
    pub similarity: f64,
    /// Rondas de refinamiento Weisfeiler-Lehman: cuántos vecinos de distancia mira el hash.
    pub wl_rounds: usize,
}

impl Default for ConsistencyConfig {
    fn default() -> Self {
        Self {
            similarity: 0.6,
            wl_rounds: 3,
        }
    }
}

/// FNV-1a de 64 bits. A diferencia de `DefaultHasher`, su algoritmo no cambia entre
/// versiones de Rust: los clusters, y el orden de sus diagnósticos, son reproducibles.
struct Fnv(u64);

impl Hasher for Fnv {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 ^= u64::from(b);
            self.0 = self.0.wrapping_mul(0x0100_0000_01b3);
        }
    }
}

fn h<T: Hash>(v: &T) -> u64 {
    let mut s = Fnv(0xcbf2_9ce4_8422_2325);
    v.hash(&mut s);
    s.finish()
}

/// Reemplaza el nombre del equipo por `{G}` (sin distinguir mayúsculas).
fn normalize_name(name: &str, group: &str) -> String {
    let (lower, g) = (name.to_ascii_lowercase(), group.to_ascii_lowercase());
    match lower.find(&g) {
        Some(i) if !g.is_empty() => format!("{}{{G}}{}", &name[..i], &name[i + g.len()..]),
        _ => name.to_string(),
    }
}

struct GroupGraph {
    name: String,
    blocks: Vec<usize>,
    /// Etiqueta WL final de cada bloque (alineada con `blocks`).
    labels: Vec<u64>,
    /// Multiconjunto ordenado de etiquetas por ronda (0 = solo tipo de bloque).
    features: Vec<Vec<u64>>,
    hash: u64,
    block_types: BTreeMap<String, usize>,
    var_roles: BTreeSet<String>,
}

fn build(section: &Section, name: &str, blocks: &[usize], rounds: usize) -> GroupGraph {
    // Contexto de cada pin: cable interno (vecino), externo, variable, literal o nada.
    enum Ctx {
        Wire(usize, String),
        Ext,
        Var(String),
        Lit,
        Free,
    }
    let local: HashMap<usize, usize> = blocks.iter().enumerate().map(|(i, &b)| (b, i)).collect();
    let mut ctx: Vec<Vec<(String, Ctx)>> = Vec::new();
    let mut var_roles = BTreeSet::new();
    let mut block_types = BTreeMap::new();
    for &b in blocks {
        let blk = &section.blocks[b];
        *block_types.entry(blk.ty.clone()).or_insert(0) += 1;
        let mut pins = Vec::new();
        for (pi, p) in blk.pins.iter().enumerate() {
            let r = PinRef { block: b, pin: pi };
            let mut push_param = |side: &str, param: &Option<Param>, wired: Option<PinRef>| {
                let c = match (wired, param) {
                    (Some(o), _) => match local.get(&o.block) {
                        Some(&lo) => Ctx::Wire(lo, section.pin(o).name.clone()),
                        None => Ctx::Ext,
                    },
                    (None, Some(Param::Var(v))) => {
                        let role = normalize_name(v, name);
                        var_roles.insert(role.clone());
                        Ctx::Var(role)
                    }
                    (None, Some(Param::Lit(_))) => Ctx::Lit,
                    (None, None) => Ctx::Free,
                };
                pins.push((format!("{side}{}", p.name), c));
            };
            if p.dir.has_left() {
                let w = section.wires.iter().find(|w| w.to == r).map(|w| w.from);
                push_param("<", &p.param_in, w);
            }
            if p.dir.has_right() {
                // Una salida puede tener varios cables: uno por destino.
                // y además puede escribir una variable (corte de realimentación).
                let outs: Vec<PinRef> = section
                    .wires
                    .iter()
                    .filter(|w| w.from == r)
                    .map(|w| w.to)
                    .collect();
                if outs.is_empty() || p.param_out.is_some() {
                    push_param(">", &p.param_out, None);
                }
                for o in outs {
                    push_param(">", &None, Some(o));
                }
            }
        }
        ctx.push(pins);
    }

    let mut labels: Vec<u64> = blocks
        .iter()
        .map(|&b| h(&(&section.blocks[b].ty, section.blocks[b].extra_pins)))
        .collect();
    let sorted = |v: &[u64]| {
        let mut v = v.to_vec();
        v.sort_unstable();
        v
    };
    let mut features = vec![sorted(&labels)];
    for _ in 0..rounds {
        let next: Vec<u64> = ctx
            .iter()
            .enumerate()
            .map(|(i, pins)| {
                let mut parts: Vec<(String, u64)> = pins
                    .iter()
                    .map(|(pin, c)| {
                        let v = match c {
                            Ctx::Wire(n, other) => h(&("w", labels[*n], other)),
                            Ctx::Ext => h(&"ext"),
                            Ctx::Var(role) => h(&("v", role)),
                            Ctx::Lit => h(&"lit"),
                            Ctx::Free => h(&"-"),
                        };
                        (pin.clone(), v)
                    })
                    .collect();
                parts.sort();
                h(&(labels[i], parts))
            })
            .collect();
        labels = next;
        features.push(sorted(&labels));
    }
    GroupGraph {
        name: name.to_string(),
        blocks: blocks.to_vec(),
        hash: h(features
            .last()
            .expect("features empieza con las etiquetas iniciales")),
        labels,
        features,
        block_types,
        var_roles,
    }
}

/// Similitud entre dos equipos: Jaccard por ronda WL, ponderado para que pese más la
/// estructura local (un cambio puntual contamina todas las etiquetas de rondas altas).
fn similarity(a: &[Vec<u64>], b: &[Vec<u64>]) -> f64 {
    let (mut num, mut den, mut w) = (0.0, 0.0, 1.0);
    for (ra, rb) in a.iter().zip(b) {
        num += w * jaccard(ra, rb);
        den += w;
        w /= 2.0;
    }
    num / den
}

/// Jaccard de multiconjuntos ordenados.
fn jaccard(a: &[u64], b: &[u64]) -> f64 {
    let (mut i, mut j, mut inter) = (0, 0, 0);
    while i < a.len() && j < b.len() {
        match a[i].cmp(&b[j]) {
            std::cmp::Ordering::Equal => {
                inter += 1;
                i += 1;
                j += 1;
            }
            std::cmp::Ordering::Less => i += 1,
            std::cmp::Ordering::Greater => j += 1,
        }
    }
    let union = a.len() + b.len() - inter;
    if union == 0 {
        return 1.0;
    }
    #[expect(
        clippy::as_conversions,
        clippy::cast_precision_loss,
        reason = "conteos de etiquetas de un grupo, muy por debajo de 2^52"
    )]
    let ratio = inter as f64 / union as f64;
    ratio
}

fn describe_diff(g: &GroupGraph, typical: &GroupGraph) -> String {
    let mut parts = Vec::new();
    let types: BTreeSet<&String> = g
        .block_types
        .keys()
        .chain(typical.block_types.keys())
        .collect();
    for t in types {
        let (a, b) = (
            g.block_types.get(t).copied().unwrap_or(0),
            typical.block_types.get(t).copied().unwrap_or(0),
        );
        if a < b {
            parts.push(format!("le falta {} {t}", b - a));
        } else if a > b {
            parts.push(format!("tiene {} {t} de más", a - b));
        }
    }
    let show = |role: &str| role.replace("{G}", &g.name);
    let missing: Vec<String> = typical
        .var_roles
        .difference(&g.var_roles)
        .map(|r| show(r))
        .collect();
    let extra: Vec<String> = g
        .var_roles
        .difference(&typical.var_roles)
        .map(|r| show(r))
        .collect();
    if !missing.is_empty() {
        parts.push(format!("no usa {}", missing.join(", ")));
    }
    if !extra.is_empty() {
        parts.push(format!("usa {} (no está en el típico)", extra.join(", ")));
    }
    if parts.is_empty() {
        parts.push("mismos bloques y variables, conexiones distintas".into());
    }
    parts.join("; ")
}

/// Por equipo, los literales escritos en un pin: (valor, id de bloque).
type LiteralsByGroup = BTreeMap<usize, Vec<(String, String)>>;

/// Literales que difieren de la mayoría dentro de un típico.
fn literal_outliers(section: &Section, members: &[&GroupGraph], diags: &mut Diagnostics) {
    // (etiqueta WL del bloque, pin) -> por equipo, lista de (valor, id de bloque).
    let mut table: BTreeMap<(u64, String), LiteralsByGroup> = BTreeMap::new();
    for (gi, g) in members.iter().enumerate() {
        for (&b, &label) in g.blocks.iter().zip(&g.labels) {
            for p in &section.blocks[b].pins {
                if let Some(Param::Lit(v)) = &p.param_in {
                    table
                        .entry((label, p.name.clone()))
                        .or_default()
                        .entry(gi)
                        .or_default()
                        .push((v.clone(), section.blocks[b].id.clone()));
                }
            }
        }
    }
    // Los literales IEC no distinguen mayúsculas (`T#5s` == `t#5S`).
    let key = |vals: &[(String, String)]| {
        let mut v: Vec<String> = vals.iter().map(|x| x.0.to_ascii_uppercase()).collect();
        v.sort();
        v
    };
    for ((_, pin), per_group) in table {
        let mut counts: HashMap<Vec<String>, usize> = HashMap::new();
        for vals in per_group.values() {
            *counts.entry(key(vals)).or_default() += 1;
        }
        if counts.len() < 2 {
            continue;
        }
        let (majority, n) = counts
            .iter()
            .max_by_key(|(_, n)| **n)
            .map(|(v, n)| (v.clone(), *n))
            .expect("hay al menos dos valores distintos");
        // Para mostrar, los valores tal como los escribió un equipo de la mayoría.
        let shown = per_group
            .values()
            .find(|v| key(v) == majority)
            .expect("la mayoría sale de los valores de los grupos");
        let shown: Vec<&str> = shown.iter().map(|x| x.0.as_str()).collect();
        if n * 2 <= per_group.len() {
            let names: Vec<&str> = per_group
                .keys()
                .map(|&gi| members[gi].name.as_str())
                .collect();
            diags.push(
                Level::Info,
                STAGE,
                format!(
                    "{}: el pin {pin} no tiene un valor mayoritario",
                    names.join(", ")
                ),
            );
            continue;
        }
        for (gi, vals) in &per_group {
            let v = key(vals);
            if v != majority {
                diags.push(
                    Level::Info,
                    STAGE,
                    format!(
                        "{}: {}.{pin} = {} (el resto del típico usa {})",
                        members[*gi].name,
                        vals[0].1,
                        vals.iter()
                            .map(|x| x.0.as_str())
                            .collect::<Vec<_>>()
                            .join(", "),
                        shown.join(", ")
                    ),
                );
            }
        }
    }
}

pub(crate) fn check(section: &Section, cfg: &ConsistencyConfig, diags: &mut Diagnostics) {
    let mut groups: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (i, b) in section.blocks.iter().enumerate() {
        if let Some(g) = &b.group {
            groups.entry(g).or_default().push(i);
        }
    }
    if groups.len() < 2 {
        return;
    }
    let graphs: Vec<GroupGraph> = groups
        .iter()
        .map(|(n, b)| build(section, n, b, cfg.wl_rounds))
        .collect();

    let mut clusters: BTreeMap<u64, Vec<usize>> = BTreeMap::new();
    for (i, g) in graphs.iter().enumerate() {
        clusters.entry(g.hash).or_default().push(i);
    }
    for members in clusters.values().filter(|m| m.len() > 1) {
        let names: Vec<&str> = members.iter().map(|&i| graphs[i].name.as_str()).collect();
        diags.push(
            Level::Info,
            STAGE,
            format!("típico compartido por {}", names.join(", ")),
        );
        let refs: Vec<&GroupGraph> = members.iter().map(|&i| &graphs[i]).collect();
        literal_outliers(section, &refs, diags);
    }

    for (i, g) in graphs.iter().enumerate() {
        if clusters[&g.hash].len() > 1 {
            continue;
        }
        let best = clusters
            .values()
            .filter(|m| m.len() > 1)
            .map(|m| (similarity(&g.features, &graphs[m[0]].features), m))
            .max_by(|a, b| a.0.total_cmp(&b.0));
        if let Some((sim, members)) = best.filter(|(s, _)| *s >= cfg.similarity) {
            let names: Vec<&str> = members.iter().map(|&m| graphs[m].name.as_str()).collect();
            diags.push(
                Level::Warning,
                STAGE,
                format!(
                    "{} se parece al típico de {} (similitud {sim:.2}) pero difiere: {}",
                    g.name,
                    names.join(", "),
                    describe_diff(g, &graphs[members[0]])
                ),
            );
        } else if let Some((j, sim)) = graphs
            .iter()
            .enumerate()
            .filter(|&(j, o)| j > i && clusters[&o.hash].len() == 1)
            .map(|(j, o)| (j, similarity(&g.features, &o.features)))
            .filter(|(_, s)| *s >= cfg.similarity)
            .max_by(|a, b| a.1.total_cmp(&b.1))
        {
            diags.push(
                Level::Info,
                STAGE,
                format!(
                    "{} y {} son casi iguales (similitud {sim:.2}): {}",
                    g.name,
                    graphs[j].name,
                    describe_diff(g, &graphs[j])
                ),
            );
        }
    }
}
