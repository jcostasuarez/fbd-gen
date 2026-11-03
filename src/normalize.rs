//! Etapa 6: normalizaciones que bajan la complejidad del dibujo antes del layout.
//! Decide cable o variable: realimentaciones y fan-out alto.
//!
//! Una salida puede escribir una variable y a la vez tener cables (verificado en Control
//! Expert 15.1): por eso una realimentación corta solo el cable que cierra el lazo y deja
//! los demás cables de esa salida como están.

use std::collections::{BTreeSet, HashMap, HashSet};

use crate::graph::{OnCycle, UnionFind, topo_order};
use crate::iec::{sanitize, unique_name};
use crate::model::*;

const STAGE: Stage = Stage::Normalize;

#[derive(Debug, Clone)]
pub struct NormalizeConfig {
    /// Umbral para reemplazar fan-out por variables cuando se conserva la ejecución.
    pub max_fanout: usize,
}

impl Default for NormalizeConfig {
    fn default() -> Self {
        Self { max_fanout: 3 }
    }
}

pub fn normalize(section: &mut Section, cfg: &NormalizeConfig, diags: &mut Diagnostics) {
    let mut names = existing_names(section);

    // Realimentaciones: un lazo no se puede cerrar con links. Se corta solo el cable que
    // cierra el lazo; el resto de los cables de esa salida se mantiene.
    for w in back_edges(section) {
        to_variable(
            section,
            w.from,
            Some(w.to),
            "realimentación",
            &mut names,
            diags,
        );
    }

    section
        .execution_dependencies
        .extend(section.wires.iter().map(|w| (w.from.block, w.to.block)));
    section.execution_dependencies.sort_unstable();
    section.execution_dependencies.dedup();

    let mut outputs: Vec<PinRef> = section.wires.iter().map(|w| w.from).collect();
    outputs.sort_by_key(|r| (r.block, r.pin));
    outputs.dedup();
    for out in outputs {
        let fanout = section.wires.iter().filter(|w| w.from == out).count();
        if fanout > cfg.max_fanout {
            // El corte solo es seguro si las dependencias restantes se pueden
            // expresar mediante cables dentro de una red y orden entre redes.
            let wires = section.wires.clone();
            section.wires.retain(|w| w.from != out);
            let can_order = ordered_networks(section).is_some();
            section.wires = wires;
            if !can_order {
                diags.push(
                    Level::Info,
                    STAGE,
                    format!(
                        "{}: se conserva fan-out {fanout} para respetar el orden de ejecución",
                        section.pin_path(out)
                    ),
                );
                continue;
            }
            let reason = format!("fan-out {fanout} > {}", cfg.max_fanout);
            to_variable(section, out, None, &reason, &mut names, diags);
        }
    }
}

fn existing_names(section: &Section) -> HashSet<String> {
    let mut names = HashSet::new();
    for b in &section.blocks {
        if let Some(i) = &b.instance {
            names.insert(i.to_ascii_uppercase());
        }
        for p in &b.pins {
            for param in [&p.param_in, &p.param_out].into_iter().flatten() {
                if let Param::Var(v) = param {
                    names.insert(v.to_ascii_uppercase());
                }
            }
        }
    }
    names
}

/// Reemplaza por una variable el cable de `out` a `only`, o todos los cables de `out`.
/// La salida pasa a escribir la variable (si no escribía ya una) y los destinos la leen.
fn to_variable(
    section: &mut Section,
    out: PinRef,
    only: Option<PinRef>,
    reason: &str,
    names: &mut HashSet<String>,
    diags: &mut Diagnostics,
) {
    let cut = |w: &Wire| w.from == out && only.is_none_or(|t| w.to == t);
    let targets: Vec<PinRef> = section
        .wires
        .iter()
        .filter(|w| cut(w))
        .map(|w| w.to)
        .collect();
    if targets.is_empty() {
        return;
    }
    let name = match &section.pin(out).param_out {
        Some(p) => p.text().to_string(),
        None => {
            let base = sanitize(&format!(
                "{}_{}",
                section.blocks[out.block].label(),
                section.pin(out).name
            ));
            let name = unique_name(&base, names);
            // La inferencia (types.rs) corre antes y deja el tipo concreto en el pin. Si
            // no lo pudo deducir, queda el genérico y `types::section_vars` lo reporta.
            let pin = section.pin(out);
            let ty = pin.inferred.clone().unwrap_or_else(|| pin.ty.clone());
            section.generated_vars.push(GeneratedVar {
                name: name.clone(),
                ty,
                reason: reason.to_string(),
            });
            section.pin_mut(out).param_out = Some(Param::Var(name.clone()));
            name
        }
    };
    for t in &targets {
        section.pin_mut(*t).param_in = Some(Param::Var(name.clone()));
    }
    section.wires.retain(|w| !cut(w));
    diags.push(
        Level::Info,
        STAGE,
        format!(
            "{} -> variable {} ({reason}, {} destino/s)",
            section.pin_path(out),
            name,
            targets.len()
        ),
    );
}

/// Cables que cierran un ciclo. Dentro de cada componente fuertemente conexo se cortan
/// los cables que van hacia atrás en el orden del modelo (de un bloque a otro declarado
/// antes): es el corte que respeta la intención del autor y deja el resto acíclico.
fn back_edges(section: &Section) -> Vec<Wire> {
    let scc = strongly_connected(section);
    section
        .wires
        .iter()
        .filter(|w| scc[w.from.block] == scc[w.to.block] && w.from.block >= w.to.block)
        .copied()
        .collect()
}

/// Tarjan iterativo: id de componente fuertemente conexo por bloque.
fn strongly_connected(section: &Section) -> Vec<usize> {
    let n = section.blocks.len();
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];
    for w in &section.wires {
        adj[w.from.block].push(w.to.block);
    }
    let mut index = vec![usize::MAX; n];
    let mut low = vec![0; n];
    let mut on_stack = vec![false; n];
    let mut comp = vec![usize::MAX; n];
    let (mut counter, mut ncomp) = (0, 0);
    let mut stack = Vec::new();
    for root in 0..n {
        if index[root] != usize::MAX {
            continue;
        }
        let mut call = vec![(root, 0usize)];
        while let Some(&(v, i)) = call.last() {
            if i == 0 && index[v] == usize::MAX {
                index[v] = counter;
                low[v] = counter;
                counter += 1;
                stack.push(v);
                on_stack[v] = true;
            }
            if let Some(&u) = adj[v].get(i) {
                call.last_mut().unwrap().1 += 1;
                if index[u] == usize::MAX {
                    call.push((u, 0));
                } else if on_stack[u] {
                    low[v] = low[v].min(index[u]);
                }
                continue;
            }
            call.pop();
            if let Some(&(parent, _)) = call.last() {
                low[parent] = low[parent].min(low[v]);
            }
            if low[v] == index[v] {
                while let Some(u) = stack.pop() {
                    on_stack[u] = false;
                    comp[u] = ncomp;
                    if u == v {
                        break;
                    }
                }
                ncomp += 1;
            }
        }
    }
    comp
}

/// Etapa 7: redes = componentes conexos por cables, ordenadas por las dependencias
/// anteriores al corte de fan-out. Los empates respetan el orden del modelo.
pub fn networks(section: &Section) -> Vec<Vec<usize>> {
    ordered_networks(section).expect("la normalización conserva las dependencias de ejecución")
}

fn ordered_networks(section: &Section) -> Option<Vec<Vec<usize>>> {
    let n = section.blocks.len();
    let mut uf = UnionFind::new(n);
    for w in &section.wires {
        uf.union(w.from.block, w.to.block);
    }
    let mut groups: Vec<Vec<usize>> = Vec::new();
    let mut index: HashMap<usize, usize> = HashMap::new();
    let mut network_of = vec![0; n];
    for (b, net) in network_of.iter_mut().enumerate() {
        let r = uf.find(b);
        let g = *index.entry(r).or_insert_with(|| {
            groups.push(Vec::new());
            groups.len() - 1
        });
        groups[g].push(b);
        *net = g;
    }
    // El layout solo ve los cables. Si un corte deja emisor y receptor en la misma red,
    // se pierde su orden relativo salvo que otro cable aún una ese par de bloques.
    let wired: BTreeSet<(usize, usize)> = section
        .wires
        .iter()
        .map(|w| (w.from.block, w.to.block))
        .collect();
    if section
        .execution_dependencies
        .iter()
        .any(|&(a, b)| network_of[a] == network_of[b] && !wired.contains(&(a, b)))
    {
        return None;
    }
    let deps: BTreeSet<(usize, usize)> = section
        .execution_dependencies
        .iter()
        .map(|&(a, b)| (network_of[a], network_of[b]))
        .filter(|(a, b)| a != b)
        .collect();
    let order = topo_order(groups.len(), deps, OnCycle::Stop);
    (order.len() == groups.len()).then(|| {
        order
            .into_iter()
            .map(|g| std::mem::take(&mut groups[g]))
            .collect()
    })
}
