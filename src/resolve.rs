//! Etapa 1: resolución del grafo semántico contra la librería y validación
//! (tipos de bloque, pines, sentidos, conductores múltiples, tipos de dato).

use std::collections::{HashMap, HashSet};

use crate::iec::{is_address, is_identifier, is_literal, is_variable_ref, task_name, var_root};
use crate::library::{BlockDef, Library};
use crate::model::{
    Block, BlockSpec, Diagnostics, Dir, Level, Param, Pin, PinRef, ProgramSpec, Section, Stage,
    Wire,
};
use crate::xml;

const STAGE: Stage = Stage::Validate;

pub(crate) enum Endpoint {
    Pin { block: usize, pin: String },
    Var(String),
    Lit(String),
}

pub(crate) fn parse_endpoint(s: &str, ids: &HashMap<String, usize>) -> Endpoint {
    let s = s.trim();
    if let Some((id, pin)) = s.split_once('.')
        && let Some(&block) = ids.get(&id.to_ascii_uppercase())
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
        .filter_map(|name| ext.index_of(name).map(|k| k + 1))
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
        let e = def.inputs[*i]
            .extensible
            .as_ref()
            .expect("`extensible_count` devuelve el índice del pin extensible");
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

/// Textos libres que terminan en el XML. Un caracter que XML 1.0 no admite (los de
/// control, salvo tabulación y saltos de línea) produciría un `.xbd` mal formado, que
/// Control Expert no puede importar. Los nombres ya se validan como identificadores.
fn check_xml_texts(spec: &ProgramSpec, diags: &mut Diagnostics) {
    let mut check = |what: &dyn Fn() -> String, text: &str| {
        if let Some(c) = xml::invalid_char(text) {
            diags.push(
                Level::Error,
                STAGE,
                format!(
                    "{}: caracter U+{:04X} no admitido en XML",
                    what(),
                    u32::from(c)
                ),
            );
        }
    };
    for v in &spec.variables {
        let fields = [
            ("tipo", Some(&v.ty)),
            ("dirección", v.address.as_ref()),
            ("comentario", v.comment.as_ref()),
            ("valor inicial", v.init.as_ref()),
        ];
        for (field, text) in fields {
            if let Some(text) = text {
                check(&|| format!("variable {}: {field}", v.name), text);
            }
        }
    }
    for b in &spec.blocks {
        if let Some(text) = &b.comment {
            check(&|| format!("{}: comentario", b.id), text);
        }
    }
    for c in &spec.comments {
        check(&|| format!("comentario de la red de {}", c.at), &c.text);
    }
    for (i, l) in spec.links.iter().enumerate() {
        let (a, b) = l.ends();
        for end in [a, b] {
            check(&|| format!("link {i}"), end);
        }
    }
}

pub(crate) fn resolve(
    spec: &ProgramSpec,
    lib: &Library,
    diags: &mut Diagnostics,
) -> Option<Section> {
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
    if task_name(&spec.task).is_none() {
        diags.push(
            Level::Error,
            STAGE,
            format!(
                "tarea \"{}\" inválida: MAST, FAST, AUX0 a AUX3 o SAFE",
                spec.task
            ),
        );
    }
    for v in &spec.variables {
        if !is_variable_ref(&v.name) {
            diags.push(
                Level::Error,
                STAGE,
                format!("variable declarada con nombre inválido: \"{}\"", v.name),
            );
        }
        if let Some(address) = v.address.as_ref().filter(|a| !is_address(a)) {
            diags.push(
                Level::Error,
                STAGE,
                format!(
                    "variable {}: \"{address}\" no es una dirección topológica (%MW100, %I0.1.2...)",
                    v.name
                ),
            );
        }
    }
    check_xml_texts(spec, diags);
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
        if ids.insert(b.id.to_ascii_uppercase(), i).is_some() {
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

    // Nombres de variables: una variable simple no puede llamarse como una instancia
    // (Control Expert declararía las dos) y una declarada no puede llamarse como un id
    // de bloque (`Motor.Run` sería ambiguo entre un pin y un campo). Un acceso a una
    // instancia (`T1.ET`) sí es válido.
    let mut reported = HashSet::new();
    let declared = spec.variables.iter().map(|v| (v.name.as_str(), true));
    let used = links
        .iter()
        .flat_map(|(a, b, ..)| [a, b])
        .filter_map(|e| match e {
            Endpoint::Var(v) => Some((v.as_str(), false)),
            _ => None,
        });
    for (name, is_declared) in declared.chain(used) {
        let Some(root) = var_root(name) else { continue };
        let key = root.to_ascii_uppercase();
        let message = if let Some(owner) = instances.get(&key).filter(|_| root == name) {
            format!("variable {name}: ya es el nombre de la instancia de {owner}")
        } else if let Some(&b) = ids.get(&key).filter(|_| is_declared) {
            format!(
                "variable {name}: hay un bloque con id {}; `{}.X` sería ambiguo",
                spec.blocks[b].id, spec.blocks[b].id
            )
        } else {
            continue;
        };
        if reported.insert(key) {
            diags.push(Level::Error, STAGE, message);
        }
    }

    let mut comments = Vec::new();
    for c in &spec.comments {
        match ids.get(&c.at.to_ascii_uppercase()) {
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
