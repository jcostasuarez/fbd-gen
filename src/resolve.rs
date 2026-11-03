//! Etapa 1: resolución del grafo semántico contra la librería y validación
//! (tipos de bloque, pines, sentidos, conductores múltiples, tipos de dato).

use std::collections::HashMap;

use crate::library::{BlockDef, Library};
use crate::model::*;

const STAGE: &str = "validate";

pub enum Endpoint {
    Pin { block: usize, pin: String },
    Var(String),
    Lit(String),
}

/// Literales IEC 61131-3: booleanos, numéricos, tipados (`T#5s`, `16#FF`, `INT#3`) y strings.
pub fn is_literal(s: &str) -> bool {
    let s = s.trim();
    if s.eq_ignore_ascii_case("TRUE") || s.eq_ignore_ascii_case("FALSE") {
        return true;
    }
    if s.starts_with('\'') || s.starts_with('"') {
        return true;
    }
    if let Some((prefix, _)) = s.split_once('#') {
        return !prefix.is_empty()
            && prefix
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_');
    }
    let body = s.strip_prefix(['-', '+']).unwrap_or(s);
    body.starts_with(|c: char| c.is_ascii_digit())
        && body
            .chars()
            .all(|c| c.is_ascii_digit() || matches!(c, '_' | '.' | 'e' | 'E' | '-' | '+'))
}

pub fn parse_endpoint(s: &str, ids: &HashMap<&str, usize>) -> Endpoint {
    let s = s.trim();
    if let Some((id, pin)) = s.split_once('.')
        && let Some(&block) = ids.get(id)
    {
        return Endpoint::Pin {
            block,
            pin: pin.to_string(),
        };
    }
    if is_literal(s) {
        Endpoint::Lit(s.to_string())
    } else {
        Endpoint::Var(s.to_string())
    }
}

/// Cantidad de instancias del pin extensible: la explícita, la mayor referenciada o el mínimo.
fn extensible_count(
    def: &BlockDef,
    explicit: Option<u32>,
    referenced: &[String],
) -> Option<(usize, u32)> {
    let (idx, ext) = def
        .inputs
        .iter()
        .enumerate()
        .find_map(|(i, p)| p.extensible.as_ref().map(|e| (i, e)))?;
    let max_ref = referenced
        .iter()
        .filter_map(|name| {
            let rest = name.get(ext.prefix.len()..)?;
            if !name[..ext.prefix.len()].eq_ignore_ascii_case(&ext.prefix) {
                return None;
            }
            let n: u32 = rest.parse().ok()?;
            n.checked_sub(ext.first_index).map(|k| k + 1)
        })
        .max()
        .unwrap_or(0);
    Some((idx, explicit.unwrap_or(0).max(max_ref).max(ext.min)))
}

fn build_block(
    spec: &BlockSpec,
    def: &BlockDef,
    referenced: &[String],
    diags: &mut Diagnostics,
) -> Block {
    let mut ext = extensible_count(def, spec.inputs, referenced);
    let mut extra_pins = 0;
    if let Some((i, n)) = &mut ext {
        let e = def.inputs[*i].extensible.as_ref().unwrap();
        if *n > e.max {
            diags.push(
                Level::Error,
                STAGE,
                format!("{}: {} entradas extensibles, máximo {}", spec.id, n, e.max),
            );
            // El error ya invalida la sección; no crear más pines que los que admite.
            *n = e.max;
        }
        extra_pins = *n - e.min;
    }
    let commutative = def.is_commutative();
    let mut pins = Vec::new();
    let mut push = |name: String, dir, ty: &str, row, permutable| {
        pins.push(Pin {
            name,
            dir,
            ty: ty.to_string(),
            row,
            permutable,
            param_in: None,
            param_out: None,
            inferred: None,
        });
    };
    for (i, p) in def.inputs.iter().enumerate() {
        let Some(pos) = p.position else { continue };
        match (&p.extensible, ext) {
            (Some(e), Some((ei, n))) if ei == i => {
                for k in 0..n {
                    push(e.pin_name(k), Dir::In, &p.ty, 3 + pos + k, commutative);
                }
            }
            _ => push(p.name.clone(), Dir::In, &p.ty, 3 + pos, false),
        }
    }
    for p in &def.outputs {
        if let Some(pos) = p.position {
            push(p.name.clone(), Dir::Out, &p.ty, 3 + pos, false);
        }
    }
    for p in &def.in_outs {
        if let Some(pos) = p.position {
            push(p.name.clone(), Dir::InOut, &p.ty, 3 + pos, false);
        }
    }
    let height = pins.iter().map(|p| p.row + 1).max().unwrap_or(4).max(4);
    if !def.width_measured {
        diags.push(
            Level::Warning,
            STAGE,
            format!(
                "{}: ancho de {} estimado ({} celdas)",
                spec.id, def.name, def.width
            ),
        );
    }
    Block {
        id: spec.id.clone(),
        ty: def.name.clone(),
        kind: def.kind,
        instance: def
            .kind
            .has_instance()
            .then(|| spec.instance.clone().unwrap_or_else(|| spec.id.clone())),
        comment: spec.comment.clone(),
        group: spec.group.clone(),
        stage: spec.stage.clone(),
        extra_pins,
        width: def.width,
        height,
        pins,
    }
}

pub fn resolve(spec: &ProgramSpec, lib: &Library, diags: &mut Diagnostics) -> Option<Section> {
    // El nombre termina en la sección de Control Expert y en los archivos de salida.
    if !is_identifier(&spec.name) {
        diags.push(
            Level::Error,
            STAGE,
            format!(
                "nombre \"{}\" inválido: tiene que ser un identificador IEC",
                spec.name
            ),
        );
    }
    if spec.blocks.is_empty() {
        diags.push(Level::Error, STAGE, "el grafo no tiene bloques");
    }
    for v in &spec.variables {
        if !is_variable_ref(&v.name) {
            diags.push(
                Level::Error,
                STAGE,
                format!("variable declarada con nombre inválido: \"{}\"", v.name),
            );
        }
    }
    let mut ids = HashMap::new();
    for (i, b) in spec.blocks.iter().enumerate() {
        // Un punto en el id lo confundiría con `bloque.PIN` en los links.
        if !is_identifier(&b.id) {
            diags.push(
                Level::Error,
                STAGE,
                format!("id de bloque inválido: \"{}\"", b.id),
            );
        }
        if let Some(inst) = b.instance.as_ref().filter(|i| !is_identifier(i)) {
            diags.push(
                Level::Error,
                STAGE,
                format!("{}: nombre de instancia inválido \"{inst}\"", b.id),
            );
        }
        if ids.insert(b.id.as_str(), i).is_some() {
            diags.push(
                Level::Error,
                STAGE,
                format!("id de bloque duplicado: {}", b.id),
            );
        }
    }

    let links: Vec<(Endpoint, Endpoint, &str, &str)> = spec
        .links
        .iter()
        .map(|l| {
            let (a, b) = l.ends();
            (parse_endpoint(a, &ids), parse_endpoint(b, &ids), a, b)
        })
        .collect();

    // Pines referenciados por bloque, para dimensionar las entradas extensibles.
    let mut referenced: Vec<Vec<String>> = vec![Vec::new(); spec.blocks.len()];
    for (a, b, ..) in &links {
        for e in [a, b] {
            if let Endpoint::Pin { block, pin } = e {
                referenced[*block].push(pin.clone());
            }
        }
    }

    let mut blocks = Vec::new();
    for (i, b) in spec.blocks.iter().enumerate() {
        match lib.get(&b.ty) {
            Some(def) => blocks.push(build_block(b, def, &referenced[i], diags)),
            None => diags.push(
                Level::Error,
                STAGE,
                format!("{}: tipo de bloque desconocido {}", b.id, b.ty),
            ),
        }
    }
    if diags.has_errors() {
        return None;
    }

    let mut instances = HashMap::new();
    for b in &blocks {
        if let Some(inst) = &b.instance
            && let Some(other) = instances.insert(inst.to_ascii_uppercase(), &b.id)
        {
            diags.push(
                Level::Error,
                STAGE,
                format!("instancia {} usada por {} y {}", inst, other, b.id),
            );
        }
    }

    let mut comments = Vec::new();
    for c in &spec.comments {
        match ids.get(c.at.as_str()) {
            Some(&b) => comments.push((b, c.text.clone())),
            None => diags.push(
                Level::Error,
                STAGE,
                format!("comentario \"{}\": no existe el bloque {}", c.text, c.at),
            ),
        }
    }
    let mut section = Section {
        name: spec.name.clone(),
        blocks,
        wires: Vec::new(),
        execution_dependencies: Vec::new(),
        generated_vars: Vec::new(),
        comments,
    };
    let mut drivers: HashMap<PinRef, String> = HashMap::new();

    for (a, b, sa, sb) in links {
        let lookup = |section: &Section, e: &Endpoint| -> Result<Option<PinRef>, String> {
            match e {
                Endpoint::Pin { block, pin } => section.blocks[*block]
                    .pin_index(pin)
                    .map(|p| {
                        Some(PinRef {
                            block: *block,
                            pin: p,
                        })
                    })
                    .ok_or_else(|| {
                        format!(
                            "{}: el bloque {} no tiene el pin {}",
                            section.blocks[*block].id, section.blocks[*block].ty, pin
                        )
                    }),
                _ => Ok(None),
            }
        };
        let (ra, rb) = match (lookup(&section, &a), lookup(&section, &b)) {
            (Ok(ra), Ok(rb)) => (ra, rb),
            (Err(e), _) | (_, Err(e)) => {
                diags.push(Level::Error, STAGE, e);
                continue;
            }
        };
        let link = format!("[{sa} -> {sb}]");
        if let Some(name) = [&a, &b].into_iter().find_map(|e| match e {
            Endpoint::Var(v) if !is_variable_ref(v) => Some(v),
            _ => None,
        }) {
            diags.push(
                Level::Error,
                STAGE,
                format!("{link}: \"{name}\" no es un pin, un literal ni una variable válida"),
            );
            continue;
        }

        // Destino de un valor: entrada (cable o parámetro).
        let mut claim_input = |section: &Section, r: PinRef, diags: &mut Diagnostics| -> bool {
            if !section.pin(r).dir.has_left() {
                diags.push(
                    Level::Error,
                    STAGE,
                    format!("{link}: {} no es una entrada", section.pin_path(r)),
                );
                return false;
            }
            if let Some(prev) = drivers.insert(r, link.clone()) {
                diags.push(
                    Level::Error,
                    STAGE,
                    format!(
                        "{link}: {} ya está conectada por {prev}",
                        section.pin_path(r)
                    ),
                );
                return false;
            }
            true
        };

        match (ra, rb) {
            (Some(from), Some(to)) => {
                if !section.pin(from).dir.has_right() {
                    diags.push(
                        Level::Error,
                        STAGE,
                        format!("{link}: {} no es una salida", section.pin_path(from)),
                    );
                    continue;
                }
                if !claim_input(&section, to, diags) {
                    continue;
                }
                // La compatibilidad de tipos la valida la inferencia (types.rs).
                section.wires.push(Wire { from, to });
            }
            (None, Some(to)) => {
                if !claim_input(&section, to, diags) {
                    continue;
                }
                let param = match a {
                    Endpoint::Lit(s) => Param::Lit(s),
                    Endpoint::Var(s) => Param::Var(s),
                    Endpoint::Pin { .. } => unreachable!(),
                };
                section.pin_mut(to).param_in = Some(param);
            }
            (Some(from), None) => {
                let Endpoint::Var(name) = b else {
                    diags.push(
                        Level::Error,
                        STAGE,
                        format!("{link}: una salida no puede escribir un literal"),
                    );
                    continue;
                };
                if !section.pin(from).dir.has_right() {
                    diags.push(
                        Level::Error,
                        STAGE,
                        format!("{link}: {} no es una salida", section.pin_path(from)),
                    );
                    continue;
                }
                if let Some(prev) = &section.pin(from).param_out {
                    diags.push(
                        Level::Error,
                        STAGE,
                        format!("{link}: la salida ya escribe {}", prev.text()),
                    );
                    continue;
                }
                section.pin_mut(from).param_out = Some(Param::Var(name));
            }
            (None, None) => diags.push(
                Level::Error,
                STAGE,
                format!("{link}: ningún extremo es un pin de bloque"),
            ),
        }
    }

    for (bi, b) in section.blocks.iter().enumerate() {
        for (pi, p) in b.pins.iter().enumerate() {
            let connected = drivers.contains_key(&PinRef { block: bi, pin: pi });
            if p.permutable && !connected {
                diags.push(
                    Level::Warning,
                    STAGE,
                    format!("{}.{}: entrada extensible sin conectar", b.id, p.name),
                );
            }
            if p.dir == Dir::InOut && !connected {
                diags.push(
                    Level::Warning,
                    STAGE,
                    format!(
                        "{}.{}: IN_OUT sin conectar del lado de entrada",
                        b.id, p.name
                    ),
                );
            }
        }
    }

    (!diags.has_errors()).then_some(section)
}
