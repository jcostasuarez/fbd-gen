//! Etapa 1: resolución del grafo semántico contra la librería y validación
//! (tipos de bloque, pines, sentidos, conductores múltiples, tipos de dato).

use std::collections::{HashMap, HashSet};

use crate::iec::{is_address, is_identifier, is_literal, is_variable_ref, task_name, var_root};
use crate::library::{BlockDef, Extensible, Library};
use crate::model::{
    Block, BlockSpec, Diagnostic, DiagnosticCode as Code, Diagnostics, Dir, Level, Param, Pin,
    PinRef, ProgramSpec, Section, Stage, Wire,
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
fn extensible_count<'d>(
    def: &'d BlockDef,
    explicit: Option<u32>,
    referenced: &[String],
) -> Option<(usize, &'d Extensible, u32)> {
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
    Some((idx, ext, explicit.unwrap_or(0).max(max_ref).max(ext.min)))
}

fn build_block(
    spec: &BlockSpec,
    def: &BlockDef,
    referenced: &[String],
    diags: &mut Diagnostics,
) -> Block {
    let mut ext = extensible_count(def, spec.inputs, referenced);
    let mut extra_pins = 0;
    if let Some((_, e, n)) = &mut ext {
        if *n > e.max {
            diags
                .push(
                    Level::Error,
                    STAGE,
                    Code::TooManyInputs,
                    format!("{}: {} entradas extensibles, máximo {}", spec.id, n, e.max),
                )
                .at_block(&spec.id);
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
            (Some(e), Some((ei, _, n))) if ei == i => {
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
        diags
            .push(
                Level::Warning,
                STAGE,
                Code::EstimatedWidth,
                format!(
                    "{}: ancho de {} estimado ({} celdas)",
                    spec.id, def.name, def.width
                ),
            )
            .at_block(&spec.id);
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
    let mut check = |what: &dyn Fn() -> String, at: &dyn Fn(&mut Diagnostic), text: &str| {
        if let Some(c) = xml::invalid_char(text) {
            at(diags.push(
                Level::Error,
                STAGE,
                Code::InvalidXmlChar,
                format!(
                    "{}: caracter U+{:04X} no admitido en XML",
                    what(),
                    u32::from(c)
                ),
            ));
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
                check(
                    &|| format!("variable {}: {field}", v.name),
                    &|d| {
                        d.at_variable(&v.name);
                    },
                    text,
                );
            }
        }
    }
    for b in &spec.blocks {
        if let Some(text) = &b.comment {
            check(
                &|| format!("{}: comentario", b.id),
                &|d| {
                    d.at_block(&b.id);
                },
                text,
            );
        }
    }
    for (i, c) in spec.comments.iter().enumerate() {
        check(
            &|| format!("comentario de la red de {}", c.at),
            &|d| {
                d.at_comment(i);
            },
            &c.text,
        );
    }
    for (i, l) in spec.links.iter().enumerate() {
        let (a, b) = l.ends();
        for end in [a, b] {
            check(
                &|| format!("link {i}"),
                &|d| {
                    d.at_link(i);
                },
                end,
            );
        }
    }
}

/// Un link ya parseado: sus dos extremos y el texto original de cada uno.
type ParsedLink<'a> = (Endpoint, Endpoint, &'a str, &'a str);

pub(crate) fn resolve(
    spec: &ProgramSpec,
    lib: &Library,
    diags: &mut Diagnostics,
) -> Option<Section> {
    check_program(spec, diags);
    check_xml_texts(spec, diags);
    let ids = index_blocks(spec, diags);
    let links: Vec<ParsedLink<'_>> = spec
        .links
        .iter()
        .map(|l| {
            let (a, b) = l.ends();
            (parse_endpoint(a, &ids), parse_endpoint(b, &ids), a, b)
        })
        .collect();
    let blocks = build_blocks(
        spec,
        lib,
        &referenced_pins(spec.blocks.len(), &links),
        diags,
    );
    if diags.has_errors() {
        return None;
    }
    check_names(spec, &ids, &blocks, &links, diags);
    let mut section = Section {
        name: spec.name.clone(),
        blocks,
        wires: Vec::new(),
        execution_dependencies: Vec::new(),
        generated_vars: Vec::new(),
        comments: resolve_comments(spec, &ids, diags),
    };
    let mut drivers = HashMap::new();
    for (li, link) in links.into_iter().enumerate() {
        connect(&mut section, &mut drivers, li, link, diags);
    }
    warn_unconnected(&section, &drivers, diags);
    (!diags.has_errors()).then_some(section)
}

/// Nombre del programa, tarea y declaraciones de variables.
fn check_program(spec: &ProgramSpec, diags: &mut Diagnostics) {
    // El nombre termina en la sección de Control Expert y en los archivos de salida.
    if !is_identifier(&spec.name) {
        diags.push(
            Level::Error,
            STAGE,
            Code::InvalidProgramName,
            format!(
                "nombre \"{}\" inválido: tiene que ser un identificador IEC",
                spec.name
            ),
        );
    }
    if spec.blocks.is_empty() {
        diags.push(
            Level::Error,
            STAGE,
            Code::EmptyGraph,
            "el grafo no tiene bloques",
        );
    }
    if task_name(&spec.task).is_none() {
        diags.push(
            Level::Error,
            STAGE,
            Code::InvalidTask,
            format!(
                "tarea \"{}\" inválida: MAST, FAST, AUX0 a AUX3 o SAFE",
                spec.task
            ),
        );
    }
    for v in &spec.variables {
        if !is_variable_ref(&v.name) {
            diags
                .push(
                    Level::Error,
                    STAGE,
                    Code::InvalidVariableName,
                    format!("variable declarada con nombre inválido: \"{}\"", v.name),
                )
                .at_variable(&v.name);
        }
        if let Some(address) = v.address.as_ref().filter(|a| !is_address(a)) {
            diags.push(
                Level::Error,
                STAGE, Code::InvalidAddress,
                format!(
                    "variable {}: \"{address}\" no es una dirección topológica (%MW100, %I0.1.2...)",
                    v.name
                ),
            ).at_variable(&v.name);
        }
    }
}

/// Índice de cada bloque por id (sin distinguir mayúsculas). Valida ids e instancias.
fn index_blocks(spec: &ProgramSpec, diags: &mut Diagnostics) -> HashMap<String, usize> {
    let mut ids = HashMap::new();
    for (i, b) in spec.blocks.iter().enumerate() {
        // Un punto en el id lo confundiría con `bloque.PIN` en los links.
        if !is_identifier(&b.id) {
            diags
                .push(
                    Level::Error,
                    STAGE,
                    Code::InvalidBlockId,
                    format!("id de bloque inválido: \"{}\"", b.id),
                )
                .at_block(&b.id);
        }
        if let Some(inst) = b.instance.as_ref().filter(|i| !is_identifier(i)) {
            diags
                .push(
                    Level::Error,
                    STAGE,
                    Code::InvalidInstanceName,
                    format!("{}: nombre de instancia inválido \"{inst}\"", b.id),
                )
                .at_block(&b.id);
        }
        if ids.insert(b.id.to_ascii_uppercase(), i).is_some() {
            diags
                .push(
                    Level::Error,
                    STAGE,
                    Code::DuplicateBlockId,
                    format!("id de bloque duplicado: {}", b.id),
                )
                .at_block(&b.id);
        }
    }
    ids
}

/// Pines referenciados por bloque, para dimensionar las entradas extensibles.
fn referenced_pins(block_count: usize, links: &[ParsedLink<'_>]) -> Vec<Vec<String>> {
    let mut referenced = vec![Vec::new(); block_count];
    for (a, b, ..) in links {
        for e in [a, b] {
            if let Endpoint::Pin { block, pin } = e {
                referenced[*block].push(pin.clone());
            }
        }
    }
    referenced
}

/// Bloques con sus pines según la librería. Un tipo desconocido es un error.
fn build_blocks(
    spec: &ProgramSpec,
    lib: &Library,
    referenced: &[Vec<String>],
    diags: &mut Diagnostics,
) -> Vec<Block> {
    let mut blocks = Vec::new();
    for (b, referenced) in spec.blocks.iter().zip(referenced) {
        match lib.get(&b.ty) {
            Some(def) => blocks.push(build_block(b, def, referenced, diags)),
            None => {
                diags
                    .push(
                        Level::Error,
                        STAGE,
                        Code::UnknownBlockType,
                        format!("{}: tipo de bloque desconocido {}", b.id, b.ty),
                    )
                    .at_block(&b.id);
            }
        }
    }
    blocks
}

/// Instancias únicas y nombres de variables sin ambigüedad.
///
/// Una variable simple no puede llamarse como una instancia (Control Expert declararía
/// las dos) y una declarada no puede llamarse como un id de bloque (`Motor.Run` sería
/// ambiguo entre un pin y un campo). Un acceso a una instancia (`T1.ET`) sí es válido.
fn check_names(
    spec: &ProgramSpec,
    ids: &HashMap<String, usize>,
    blocks: &[Block],
    links: &[ParsedLink<'_>],
    diags: &mut Diagnostics,
) {
    let mut instances = HashMap::new();
    for b in blocks {
        if let Some(inst) = &b.instance
            && let Some(other) = instances.insert(inst.to_ascii_uppercase(), &b.id)
        {
            diags
                .push(
                    Level::Error,
                    STAGE,
                    Code::DuplicateInstance,
                    format!("instancia {} usada por {} y {}", inst, other, b.id),
                )
                .at_block(&b.id);
        }
    }

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
            diags
                .push(Level::Error, STAGE, Code::NameCollision, message)
                .at_variable(name);
        }
    }
}

/// Comentarios de red anclados a su bloque.
fn resolve_comments(
    spec: &ProgramSpec,
    ids: &HashMap<String, usize>,
    diags: &mut Diagnostics,
) -> Vec<(usize, String)> {
    let mut comments = Vec::new();
    for (i, c) in spec.comments.iter().enumerate() {
        match ids.get(&c.at.to_ascii_uppercase()) {
            Some(&b) => comments.push((b, c.text.clone())),
            None => {
                diags
                    .push(
                        Level::Error,
                        STAGE,
                        Code::UnknownCommentTarget,
                        format!("comentario \"{}\": no existe el bloque {}", c.text, c.at),
                    )
                    .at_comment(i);
            }
        }
    }
    comments
}

/// Extremo de un link ya buscado en la sección.
enum End {
    /// Un pin de bloque.
    Pin(PinRef),
    /// Una variable o un literal, escrito junto al pin del otro extremo.
    Value(Param),
}

/// Busca el pin de un extremo `bloque.PIN` del link `li`; los demás extremos son valores.
fn lookup(section: &Section, li: usize, endpoint: Endpoint) -> Result<End, Box<Diagnostic>> {
    match endpoint {
        Endpoint::Pin { block, pin } => section.blocks[block]
            .pin_index(&pin)
            .map(|p| End::Pin(PinRef { block, pin: p }))
            .ok_or_else(|| {
                let b = &section.blocks[block];
                let mut d = Diagnostic::new(
                    Level::Error,
                    STAGE,
                    Code::UnknownPin,
                    format!("{}: el bloque {} no tiene el pin {}", b.id, b.ty, pin),
                );
                d.at_link(li).at_block(&b.id).at_pin_name(&pin);
                Box::new(d)
            }),
        Endpoint::Var(name) => Ok(End::Value(Param::Var(name))),
        Endpoint::Lit(text) => Ok(End::Value(Param::Lit(text))),
    }
}

/// Aplica un link a la sección: un cable entre dos pines, o un parámetro junto a un pin.
/// `drivers` recuerda qué link conecta cada entrada, para rechazar una segunda.
fn connect(
    section: &mut Section,
    drivers: &mut HashMap<PinRef, String>,
    li: usize,
    (a, b, sa, sb): ParsedLink<'_>,
    diags: &mut Diagnostics,
) {
    let link = format!("[{sa} -> {sb}]");
    let (from, to) = match (lookup(section, li, a), lookup(section, li, b)) {
        (Ok(from), Ok(to)) => (from, to),
        (Err(e), _) | (_, Err(e)) => {
            diags.0.push(*e);
            return;
        }
    };
    let invalid_var = [&from, &to].into_iter().find_map(|e| match e {
        End::Value(Param::Var(v)) if !is_variable_ref(v) => Some(v),
        _ => None,
    });
    if let Some(name) = invalid_var {
        diags
            .push(
                Level::Error,
                STAGE,
                Code::InvalidEndpoint,
                format!("{link}: \"{name}\" no es un pin, un literal ni una variable válida"),
            )
            .at_link(li);
        return;
    }
    let is_output = |section: &Section, r: PinRef, diags: &mut Diagnostics| {
        let ok = section.pin(r).dir.has_right();
        if !ok {
            diags
                .push(
                    Level::Error,
                    STAGE,
                    Code::NotAnOutput,
                    format!("{link}: {} no es una salida", section.pin_path(r)),
                )
                .at_link(li)
                .at_pin(section, r);
        }
        ok
    };
    match (from, to) {
        (End::Pin(from), End::Pin(to)) => {
            if is_output(section, from, diags)
                && claim_input(section, drivers, li, to, &link, diags)
            {
                // La compatibilidad de tipos la valida la inferencia (types.rs).
                section.wires.push(Wire { from, to });
            }
        }
        (End::Value(param), End::Pin(to)) => {
            if claim_input(section, drivers, li, to, &link, diags) {
                section.pin_mut(to).param_in = Some(param);
            }
        }
        (End::Pin(from), End::Value(Param::Var(name))) => {
            if !is_output(section, from, diags) {
                return;
            }
            if let Some(prev) = &section.pin(from).param_out {
                diags
                    .push(
                        Level::Error,
                        STAGE,
                        Code::OutputAlreadyAssigned,
                        format!("{link}: la salida ya escribe {}", prev.text()),
                    )
                    .at_link(li)
                    .at_pin(section, from);
                return;
            }
            section.pin_mut(from).param_out = Some(Param::Var(name));
        }
        (End::Pin(from), End::Value(Param::Lit(_))) => {
            diags
                .push(
                    Level::Error,
                    STAGE,
                    Code::OutputToLiteral,
                    format!("{link}: una salida no puede escribir un literal"),
                )
                .at_link(li)
                .at_pin(section, from);
        }
        (End::Value(_), End::Value(_)) => {
            diags
                .push(
                    Level::Error,
                    STAGE,
                    Code::NoBlockEndpoint,
                    format!("{link}: ningún extremo es un pin de bloque"),
                )
                .at_link(li);
        }
    }
}

/// Reserva la entrada `r` para `link`: tiene que ser una entrada y no estar conectada.
fn claim_input(
    section: &Section,
    drivers: &mut HashMap<PinRef, String>,
    li: usize,
    r: PinRef,
    link: &str,
    diags: &mut Diagnostics,
) -> bool {
    if !section.pin(r).dir.has_left() {
        diags
            .push(
                Level::Error,
                STAGE,
                Code::NotAnInput,
                format!("{link}: {} no es una entrada", section.pin_path(r)),
            )
            .at_link(li)
            .at_pin(section, r);
        return false;
    }
    if let Some(prev) = drivers.insert(r, link.to_string()) {
        diags
            .push(
                Level::Error,
                STAGE,
                Code::InputAlreadyDriven,
                format!(
                    "{link}: {} ya está conectada por {prev}",
                    section.pin_path(r)
                ),
            )
            .at_link(li)
            .at_pin(section, r);
        return false;
    }
    true
}

/// Avisos para entradas extensibles e IN_OUT que ningún link alimenta.
fn warn_unconnected(section: &Section, drivers: &HashMap<PinRef, String>, diags: &mut Diagnostics) {
    for (bi, b) in section.blocks.iter().enumerate() {
        for (pi, p) in b.pins.iter().enumerate() {
            let connected = drivers.contains_key(&PinRef { block: bi, pin: pi });
            if p.permutable && !connected {
                diags
                    .push(
                        Level::Warning,
                        STAGE,
                        Code::UnconnectedExtensibleInput,
                        format!("{}.{}: entrada extensible sin conectar", b.id, p.name),
                    )
                    .at_block(&b.id)
                    .at_pin_name(&p.name);
            }
            if p.dir == Dir::InOut && !connected {
                diags
                    .push(
                        Level::Warning,
                        STAGE,
                        Code::UnconnectedInOut,
                        format!(
                            "{}.{}: IN_OUT sin conectar del lado de entrada",
                            b.id, p.name
                        ),
                    )
                    .at_block(&b.id)
                    .at_pin_name(&p.name);
            }
        }
    }
}
