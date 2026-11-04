//! Propiedades del pipeline completo sobre programas pseudoaleatorios (deterministas, sin
//! dependencias de fuzzing). Cada caso se genera desde la librería real: tipos de bloque,
//! pines extensibles, literales del tipo del pin, cables entre bloques, variables, grupos,
//! etapas y comentarios. Una parte se "ensucia" a propósito con entradas hostiles.
//!
//! Para cualquier entrada, `generate_program`:
//! - no entra en pánico ni devuelve `Layout`/`Internal` (eso sería un bug);
//! - es determinista (dos corridas dan exactamente lo mismo);
//! - si rechaza, trae al menos un error, y cada ubicación apunta a algo de la entrada;
//! - si acepta, no trae errores, cada XML está bien formado y coincide con su layout, y
//!   todos los bloques de la entrada aparecen una sola vez entre las secciones.
//!
//! `FBD_PROPERTY_CASES` cambia la cantidad de casos (por defecto 300 de cada tipo).

use std::collections::HashSet;
use std::path::Path;

use fbd_gen::{
    BlockDefinition, BlockSpec, CommentSpec, Diagnostic, GeneratedProgram, GenerationError,
    Generator, Level, Library, LinkSpec, ProgramSpec, VarSpec,
};

/// xorshift64*: suficiente para variar entradas, y reproducible por semilla.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        let n = u64::try_from(n.max(1)).expect("cota de prueba en u64");
        usize::try_from(self.next() % n).expect("menor que la cota")
    }
    fn chance(&mut self, percent: usize) -> bool {
        self.below(100) < percent
    }
    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len())]
    }
}

fn cases() -> u64 {
    std::env::var("FBD_PROPERTY_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(300)
}

fn generator() -> Generator {
    Generator::from_library_file(Path::new(env!("CARGO_MANIFEST_DIR")).join("efb_dimensions.json"))
        .expect("efb_dimensions.json es la librería del repositorio")
}

/// Bloques frecuentes en programas reales, para que una buena parte de los casos tipe.
const COMMON: &[&str] = &[
    "AND", "OR", "XOR", "NOT", "TON", "TOF", "TP", "RS", "SR", "R_TRIG", "F_TRIG", "CTU", "ADD",
    "MUL", "SUB", "MAX", "MIN", "GT", "LT", "EQ", "SEL", "MOVE", "LIMIT", "ADD_INT", "ADD_REAL",
];

/// Tipo concreto con el que se usa un pin. Los genéricos (`ANY_NUM`...) toman uno de su
/// familia, el mismo para todos los pines genéricos del bloque. `None`: tipo que el
/// generador no sabe alimentar (arrays, DDT, referencias).
fn concrete(ty: &str, generic: &str) -> Option<String> {
    let elementary = [
        "BOOL", "INT", "UINT", "DINT", "UDINT", "REAL", "TIME", "WORD", "DWORD", "BYTE",
    ];
    match ty {
        "EBOOL" => Some("BOOL".into()),
        t if elementary.contains(&t) => Some(t.into()),
        "ANY" | "ANY_ELEMENTARY" | "ANY_MAGNITUDE" | "ANY_NUM" | "ANY_INT" | "ANY_BIT"
        | "ANY_REAL" => Some(generic.into()),
        _ => None,
    }
}

/// Tipo de los pines genéricos de un bloque, compatible con todas sus familias.
fn generic_type(def: &BlockDefinition, rng: &mut Rng) -> &'static str {
    let has = |family: &str| {
        def.inputs
            .iter()
            .chain(&def.outputs)
            .any(|p| p.ty == family)
    };
    let options: &'static [&'static str] = if has("ANY_BIT") {
        &["BOOL", "WORD"]
    } else if has("ANY_REAL") {
        &["REAL"]
    } else if has("ANY_INT") {
        &["INT", "DINT"]
    } else if has("ANY_NUM") || has("ANY_MAGNITUDE") {
        &["INT", "DINT", "REAL"]
    } else {
        &["BOOL", "INT", "REAL", "TIME"]
    };
    options[rng.below(options.len())]
}

fn literal_for(ty: &str, rng: &mut Rng) -> String {
    match ty {
        "BOOL" => rng.pick(&["TRUE", "FALSE", "BOOL#1"]).to_string(),
        "INT" => rng.pick(&["INT#3", "INT#-7"]).to_string(),
        "UINT" => "UINT#2".into(),
        "DINT" => "DINT#70000".into(),
        "UDINT" => "UDINT#5".into(),
        "REAL" => rng.pick(&["REAL#1.5", "REAL#-0.25"]).to_string(),
        "TIME" => rng.pick(&["T#2s", "T#150ms", "TIME#1m"]).to_string(),
        "WORD" => "WORD#16#FF".into(),
        "DWORD" => "DWORD#16#FFFF".into(),
        "BYTE" => "BYTE#2#1010".into(),
        _ => "TRUE".into(),
    }
}

/// Un programa plausible: pines conectados con literales, variables o salidas de su
/// mismo tipo, con realimentaciones, fan-out, grupos y etapas. La mayoría es válida.
fn plausible(rng: &mut Rng, lib: &Library) -> ProgramSpec {
    let usable: Vec<&BlockDefinition> = lib
        .iter()
        .filter(|d| {
            d.inputs
                .iter()
                .chain(&d.outputs)
                .all(|p| p.position.is_some() && concrete(&p.ty, "INT").is_some())
                && d.in_outs.is_empty()
        })
        .collect();
    let mut spec = ProgramSpec::new(format!("P{}", rng.below(1000)));
    let n = 1 + if rng.chance(15) {
        rng.below(40)
    } else {
        rng.below(12)
    };
    let groups = 1 + rng.below(4);
    // Por bloque: definición y tipo de sus pines genéricos.
    let mut blocks: Vec<(&BlockDefinition, &str)> = Vec::new();
    for i in 0..n {
        // Con el tipo explícito, la inferencia es la misma en la MSRV y en stable.
        let common: &&str = rng.pick(COMMON);
        let def = match lib.get(common) {
            Some(def) if rng.chance(70) => def,
            _ => *rng.pick(&usable),
        };
        let mut block = BlockSpec::new(format!("b{i}"), &def.name);
        if rng.chance(50) {
            block.group = Some(format!("G{}", rng.below(groups)));
        }
        if rng.chance(30) {
            block.stage = Some(rng.pick(&["entradas", "control", "salidas"]).to_string());
        }
        if rng.chance(15) {
            block.comment = Some(format!("Bloque {i}"));
        }
        if let Some(ext) = def.inputs.iter().find_map(|p| p.extensible.as_ref())
            && rng.chance(40)
        {
            let span = (ext.max - ext.min).min(6) + 1;
            let extra = rng.below(usize::try_from(span).expect("a lo sumo 7"));
            block.inputs = Some(ext.min + u32::try_from(extra).expect("a lo sumo 6"));
        }
        spec.blocks.push(block);
        blocks.push((def, generic_type(def, rng)));
    }
    for ty in ["BOOL", "INT", "REAL", "TIME"] {
        if rng.chance(60) {
            spec.variables.push(VarSpec::new(format!("v_{ty}"), ty));
        }
    }
    let declared: Vec<(String, String)> = spec
        .variables
        .iter()
        .map(|v| (v.name.clone(), v.ty.clone()))
        .collect();
    let outputs: Vec<(usize, &str, String)> = blocks
        .iter()
        .enumerate()
        .flat_map(|(i, (d, g))| {
            d.outputs
                .iter()
                .filter_map(move |p| concrete(&p.ty, g).map(|t| (i, p.name.as_str(), t)))
        })
        .collect();
    let mut written = 0;
    for (i, (def, generic)) in blocks.iter().enumerate() {
        for pin in &def.inputs {
            let Some(ty) = concrete(&pin.ty, generic) else {
                continue;
            };
            let names: Vec<String> = match &pin.extensible {
                Some(ext) => {
                    let count = spec.blocks[i].inputs.unwrap_or(ext.min);
                    (0..count).map(|k| ext.pin_name(k)).collect()
                }
                None => vec![pin.name.clone()],
            };
            for name in names {
                if rng.chance(10) {
                    continue;
                }
                let same: Vec<String> = outputs
                    .iter()
                    .filter(|(b, _, t)| *t == ty && (*b != i || rng.chance(20)))
                    .map(|(b, p, _)| format!("b{b}.{p}"))
                    .collect();
                let var = declared
                    .iter()
                    .find(|(_, t)| *t == ty)
                    .map(|(n, _)| n.clone());
                let source = match rng.below(10) {
                    0..=4 if !same.is_empty() => rng.pick(&same).clone(),
                    5..=6 if var.is_some() => var.unwrap_or_default(),
                    _ => literal_for(&ty, rng),
                };
                spec.links
                    .push(LinkSpec::Pair(source, format!("b{i}.{name}")));
            }
        }
        for pin in &def.outputs {
            if rng.chance(25) {
                spec.links.push(LinkSpec::Object {
                    from: format!("b{i}.{}", pin.name),
                    to: format!("w{written}"),
                });
                written += 1;
            }
        }
    }
    if rng.chance(30) {
        let at = format!("b{}", rng.below(n));
        spec.comments
            .push(CommentSpec::new(at, "Comentario de la red"));
    }
    spec.partition.by_stage = rng.chance(50);
    spec.partition.max_blocks = 2 + rng.below(25);
    spec
}

/// Textos que suelen romper parsers y validaciones.
const HOSTILE: &[&str] = &[
    "",
    " ",
    ".",
    "a.b.c",
    "b0.",
    ".IN",
    "%MW",
    "%MW100",
    "%I0.1.2",
    "T#",
    "T#-5s",
    "16#",
    "INT#",
    "REAL#1e999",
    "x[",
    "x[3]",
    "Motor.Run",
    "'",
    "'sin cerrar",
    "<&>\"'",
    "a\u{1}b",
    "\u{FFFF}",
    "ñandú",
    "名前",
    "1.5.5",
    "__",
    "TRUE.Q",
    "b0.IN999",
    "b0.in1",
    "B0.OUT",
];

/// Ensucia un programa con mutaciones al azar sobre nombres, links, tipos y opciones.
fn hostile(rng: &mut Rng, lib: &Library) -> ProgramSpec {
    let mut spec = plausible(rng, lib);
    let long = "X".repeat(5000);
    for _ in 0..=rng.below(4) {
        let text = if rng.chance(5) {
            long.clone()
        } else {
            rng.pick(HOSTILE).to_string()
        };
        match rng.below(12) {
            0 if !spec.links.is_empty() => {
                let i = rng.below(spec.links.len());
                let (a, b) = spec.links[i].ends();
                spec.links[i] = if rng.chance(50) {
                    LinkSpec::Pair(text, b.to_string())
                } else {
                    LinkSpec::Pair(a.to_string(), text)
                };
            }
            1 => {
                let i = rng.below(spec.blocks.len());
                spec.blocks[i].ty = if rng.chance(50) {
                    text
                } else {
                    spec.blocks[i].ty.to_ascii_lowercase()
                };
            }
            2 => {
                let i = rng.below(spec.blocks.len());
                spec.blocks[i].id = text;
            }
            3 => {
                let i = rng.below(spec.blocks.len());
                spec.blocks[i].instance = Some(text);
            }
            4 => {
                let i = rng.below(spec.blocks.len());
                spec.blocks[i].inputs = Some(*rng.pick(&[0, 1, 33, 1000, u32::MAX]));
            }
            5 => spec.task = text,
            6 => spec.name = text,
            7 => {
                let mut v = VarSpec::new(
                    text,
                    *rng.pick(&["BOOL", "", "NO_EXISTE", "ARRAY[0..3] OF INT"]),
                );
                v.address = rng.chance(30).then(|| rng.pick(HOSTILE).to_string());
                v.init = rng.chance(30).then(|| rng.pick(HOSTILE).to_string());
                v.comment = rng.chance(30).then(|| rng.pick(HOSTILE).to_string());
                spec.variables.push(v);
            }
            8 => {
                // Duplica un bloque (id repetido) o un link (entrada con dos fuentes).
                if rng.chance(50) {
                    let b = spec.blocks[rng.below(spec.blocks.len())].clone();
                    spec.blocks.push(b);
                } else if !spec.links.is_empty() {
                    let l = spec.links[rng.below(spec.links.len())].clone();
                    spec.links.push(l);
                }
            }
            9 => spec.comments.push(CommentSpec::new(
                rng.pick(&["b0", "zz", ""]).to_string(),
                text,
            )),
            10 => {
                // Un lazo explícito entre dos bloques de salida/entrada booleana.
                spec.links
                    .push(LinkSpec::Pair("b0.OUT".into(), "b0.IN2".into()));
            }
            _ => spec.partition.max_blocks = *rng.pick(&[0, 1, usize::MAX]),
        }
    }
    spec
}

/// Ids, grupos, links y comentarios de la entrada, para validar ubicaciones.
fn check_locations(spec: &ProgramSpec, diags: &[Diagnostic], sections: Option<&HashSet<String>>) {
    let ids: HashSet<String> = spec
        .blocks
        .iter()
        .map(|b| b.id.to_ascii_uppercase())
        .collect();
    let groups: HashSet<&str> = spec
        .blocks
        .iter()
        .filter_map(|b| b.group.as_deref())
        .collect();
    for d in diags {
        let l = &d.location;
        if let Some(block) = &l.block {
            // `generate_function` usa un id propio; acá solo hay programas.
            assert!(
                ids.contains(&block.to_ascii_uppercase()),
                "bloque {block} inexistente: {d:?}"
            );
        }
        if let Some(link) = l.link {
            assert!(link < spec.links.len(), "link {link} fuera de rango: {d:?}");
        }
        if let Some(comment) = l.comment {
            assert!(
                comment < spec.comments.len(),
                "comentario fuera de rango: {d:?}"
            );
        }
        if let Some(group) = &l.group {
            assert!(
                groups.contains(group.as_str()),
                "grupo {group} inexistente: {d:?}"
            );
        }
        if let (Some(section), Some(sections)) = (&l.section, sections) {
            assert!(
                sections.contains(section),
                "sección {section} inexistente: {d:?}"
            );
        }
        if l.pin.is_some() {
            assert!(
                l.block.is_some() || d.stage == fbd_gen::Stage::Consistency,
                "pin sin bloque: {d:?}"
            );
        }
        assert!(!d.message.is_empty(), "diagnóstico sin mensaje: {d:?}");
    }
}

fn check_accepted(spec: &ProgramSpec, program: &GeneratedProgram) {
    let mut all = program.diagnostics.clone();
    for s in &program.sections {
        all.extend(s.diagnostics.iter().cloned());
    }
    assert!(
        all.iter().all(|d| d.level != Level::Error),
        "error en un Ok: {all:#?}"
    );

    let mut names = HashSet::new();
    let mut ids = Vec::new();
    for s in &program.sections {
        assert!(
            names.insert(s.name.to_ascii_uppercase()),
            "sección repetida {}",
            s.name
        );
        let doc = roxmltree::Document::parse(&s.xml)
            .unwrap_or_else(|e| panic!("XML mal formado en {}: {e}", s.name));
        let count = |tag: &str| doc.descendants().filter(|n| n.has_tag_name(tag)).count();
        assert_eq!(count("FFBBlock"), s.layout.blocks.len(), "{}", s.name);
        assert_eq!(count("linkFB"), s.layout.links.len(), "{}", s.name);
        assert_eq!(count("textBox"), s.layout.text_boxes.len(), "{}", s.name);
        assert!(
            s.svg.starts_with("<svg") || s.svg.starts_with("<?xml"),
            "{}",
            s.name
        );
        roxmltree::Document::parse(&s.svg)
            .unwrap_or_else(|e| panic!("SVG mal formado en {}: {e}", s.name));
        ids.extend(s.layout.blocks.iter().map(|b| b.id.to_ascii_uppercase()));
    }
    ids.sort();
    let mut expected: Vec<String> = spec
        .blocks
        .iter()
        .map(|b| b.id.to_ascii_uppercase())
        .collect();
    expected.sort();
    assert_eq!(ids, expected, "cada bloque en una sola sección");
    let sections: HashSet<String> = program.sections.iter().map(|s| s.name.clone()).collect();
    check_locations(spec, &all, Some(&sections));
}

/// Ejecuta un caso y devuelve si fue aceptado.
fn run_case(generator: &Generator, spec: &ProgramSpec, label: &str) -> bool {
    let first = generator.generate_program(spec);
    let second = generator.generate_program(spec);
    let json = serde_json::to_string(spec).expect("un ProgramSpec siempre se serializa");
    match (&first, &second) {
        (Ok(a), Ok(b)) => {
            assert_eq!(
                serde_json::to_string(a).expect("un resultado siempre se serializa"),
                serde_json::to_string(b).expect("un resultado siempre se serializa"),
                "{label}: no determinista\n{json}"
            );
            check_accepted(spec, a);
            true
        }
        (Err(GenerationError::InvalidInput { diagnostics, .. }), Err(_)) => {
            assert_eq!(
                format!("{first:?}"),
                format!("{second:?}"),
                "{label}: no determinista\n{json}"
            );
            assert!(
                diagnostics.iter().any(|d| d.level == Level::Error),
                "{label}: rechazo sin errores\n{json}"
            );
            check_locations(spec, diagnostics, None);
            false
        }
        (Err(e), _) => panic!("{label}: {e} ({e:?})\n{json}"),
        (Ok(_), Err(e)) => panic!("{label}: no determinista: {e}\n{json}"),
    }
}

#[test]
fn plausible_programs_satisfy_the_pipeline_contract() {
    let generator = generator();
    let mut accepted = 0;
    for seed in 0..cases() {
        let spec = plausible(&mut Rng::new(seed), generator.library());
        accepted += u64::from(run_case(&generator, &spec, &format!("plausible {seed}")));
    }
    eprintln!("plausibles aceptados: {accepted} de {}", cases());
    // El generador de casos tiene que producir programas válidos de verdad, no solo
    // errores de validación.
    assert!(
        accepted * 10 >= cases() * 7,
        "solo {accepted} de {} aceptados",
        cases()
    );
}

#[test]
fn hostile_programs_never_break_the_pipeline() {
    let generator = generator();
    for seed in 0..cases() {
        let spec = hostile(&mut Rng::new(seed ^ 0xDEAD_BEEF), generator.library());
        run_case(&generator, &spec, &format!("hostil {seed}"));
    }
}
