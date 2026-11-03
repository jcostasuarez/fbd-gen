use std::path::Path;
use std::sync::OnceLock;

use crate::iec::is_literal;
use crate::layout::{self, LayoutConfig};
use crate::library::Library;
use crate::model::*;
use crate::normalize::{self, NormalizeConfig};
use crate::resolve::resolve;
use crate::{
    GeneratedProgram, GeneratedSection, GenerationError, Generator, GeneratorConfig, SectionSpec,
};

/// Layout con la reasignación de entradas conmutativas, como en `render_section`.
fn place(sec: &mut Section, nets: &[Vec<usize>], d: &mut Diagnostics) -> layout::Layout {
    let cfg = LayoutConfig::default();
    let first = layout::layout(sec, nets, &cfg).unwrap();
    let swaps = layout::commutative_swaps(sec, &first);
    if swaps.is_empty() {
        return first;
    }
    layout::apply_swaps(sec, &swaps, d);
    layout::layout(sec, nets, &cfg).unwrap()
}

fn lib() -> &'static Library {
    static LIB: OnceLock<Library> = OnceLock::new();
    LIB.get_or_init(|| {
        Library::load(Path::new(env!("CARGO_MANIFEST_DIR")).join("efb_dimensions.json")).unwrap()
    })
}

fn spec(json: &str) -> ProgramSpec {
    serde_json::from_str(json).unwrap()
}

fn generator() -> &'static Generator {
    static GENERATOR: OnceLock<Generator> = OnceLock::new();
    GENERATOR.get_or_init(|| {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("efb_dimensions.json");
        Generator::from_library_file(path)
            .unwrap()
            .with_config(GeneratorConfig {
                layout: LayoutConfig::default(),
                ..GeneratorConfig::default()
            })
            .unwrap()
    })
}

/// Genera el grafo como una única sección, sin partición.
fn section(s: &ProgramSpec) -> Result<GeneratedSection, GenerationError> {
    generator().generate_section(&SectionSpec {
        name: s.name.clone(),
        task: s.task.clone(),
        variables: s.variables.clone(),
        blocks: s.blocks.clone(),
        links: s.links.clone(),
        comments: s.comments.clone(),
    })
}

#[test]
fn literals() {
    for s in [
        "TRUE", "false", "5", "-3.5", "1e3", "T#5s", "16#FF", "INT#3", "'abc'",
    ] {
        assert!(is_literal(s), "{s}");
    }
    for s in ["P101_Run", "%MW100", "Motor.Run", "TT101"] {
        assert!(!is_literal(s), "{s}");
    }
}

#[test]
fn extensible_geometry() {
    let s = spec(r#"{"name":"t","blocks":[{"id":"m","type":"MUX"}],"links":[["x","m.IN3"]]}"#);
    let mut d = Diagnostics::default();
    let sec = resolve(&s, lib(), &mut d).unwrap();
    let b = &sec.blocks[0];
    let rows: Vec<(&str, u32)> = b.pins.iter().map(|p| (p.name.as_str(), p.row)).collect();
    // K en posición 1, IN0 extensible desde la posición 3, 4 instancias por la referencia a IN3.
    assert_eq!(
        &rows[..5],
        &[("K", 4), ("IN0", 6), ("IN1", 7), ("IN2", 8), ("IN3", 9)]
    );
    assert_eq!(b.height, 10);
    assert!(b.instance.is_none(), "las EF no llevan instancia");
}

#[test]
fn validation_errors() {
    let s = spec(
        r#"{"name":"t","blocks":[
            {"id":"a","type":"AND"},{"id":"b","type":"NOT"},{"id":"c","type":"NOPE"}
        ],"links":[]}"#,
    );
    let mut d = Diagnostics::default();
    assert!(resolve(&s, lib(), &mut d).is_none());
    assert!(d.0.iter().any(|x| x.message.contains("desconocido NOPE")));

    let s = spec(
        r#"{"name":"t","blocks":[{"id":"a","type":"AND"},{"id":"b","type":"NOT"}],"links":[
            ["x","b.IN"], ["a.OUT","b.IN"], ["b.IN","a.IN1"], ["a.OUT","5"], ["a.FOO","b.IN"]
        ]}"#,
    );
    let mut d = Diagnostics::default();
    assert!(resolve(&s, lib(), &mut d).is_none());
    let msgs: Vec<&str> = d.0.iter().map(|x| x.message.as_str()).collect();
    assert!(
        msgs.iter().any(|m| m.contains("ya está conectada")),
        "{msgs:?}"
    );
    assert!(
        msgs.iter().any(|m| m.contains("no es una salida")),
        "{msgs:?}"
    );
    assert!(msgs.iter().any(|m| m.contains("literal")), "{msgs:?}");
    assert!(
        msgs.iter().any(|m| m.contains("no tiene el pin FOO")),
        "{msgs:?}"
    );
}

#[test]
fn feedback_is_cut_against_model_order() {
    // Enclavamiento: run.OUT vuelve a seal.IN1, y seal está declarado antes que run.
    let s = spec(
        r#"{"name":"t","blocks":[
            {"id":"start","type":"AND"},{"id":"seal","type":"AND"},{"id":"run","type":"OR"}
        ],"links":[
            ["start.OUT","run.IN1"],["seal.OUT","run.IN2"],["run.OUT","seal.IN1"]
        ]}"#,
    );
    let mut d = Diagnostics::default();
    let mut sec = resolve(&s, lib(), &mut d).unwrap();
    normalize::normalize(&mut sec, &NormalizeConfig::default(), &mut d);
    assert_eq!(sec.wires.len(), 2);
    assert_eq!(sec.generated_vars[0].name, "run_OUT");
    assert_eq!(
        sec.blocks[1].pins[0].param_in,
        Some(Param::Var("run_OUT".into()))
    );
}

#[test]
fn fanout_becomes_variable() {
    let s = spec(
        r#"{"name":"t","blocks":[
            {"id":"src","type":"NOT"},{"id":"a","type":"NOT"},{"id":"b","type":"NOT"},{"id":"c","type":"NOT"}
        ],"links":[["src.OUT","a.IN"],["src.OUT","b.IN"],["src.OUT","c.IN"]]}"#,
    );
    let mut d = Diagnostics::default();
    let mut sec = resolve(&s, lib(), &mut d).unwrap();
    normalize::normalize(&mut sec, &NormalizeConfig { max_fanout: 2 }, &mut d);
    assert!(sec.wires.is_empty());
    assert_eq!(normalize::networks(&sec).len(), 4);
}

#[test]
fn commutative_inputs_follow_sources() {
    // a (arriba) alimenta IN3 y c (abajo) alimenta IN1: los cables se cruzan hasta permutar.
    let s = spec(
        r#"{"name":"t","blocks":[
            {"id":"a","type":"NOT"},{"id":"b","type":"NOT"},{"id":"c","type":"NOT"},{"id":"sum","type":"OR","inputs":3}
        ],"links":[["a.OUT","sum.IN3"],["b.OUT","sum.IN2"],["c.OUT","sum.IN1"]]}"#,
    );
    let mut d = Diagnostics::default();
    let mut sec = resolve(&s, lib(), &mut d).unwrap();
    normalize::normalize(&mut sec, &NormalizeConfig::default(), &mut d);
    let nets = normalize::networks(&sec);
    let lay = place(&mut sec, &nets, &mut d);

    // Sin cruces: el orden vertical de las fuentes coincide con el de los pines destino.
    let mut ends: Vec<(i32, i32)> = lay
        .wires
        .iter()
        .map(|w| (w[0].y, w.last().unwrap().y))
        .collect();
    ends.sort();
    assert!(ends.windows(2).all(|p| p[0].1 < p[1].1), "{ends:?}");
    for w in &lay.wires {
        assert!(
            w.windows(2).all(|s| s[0].x == s[1].x || s[0].y == s[1].y),
            "no ortogonal: {w:?}"
        );
    }
}

#[test]
fn types_are_inferred_through_generic_blocks() {
    // TT1 llega a MAX, MAX alimenta GT y GT compara contra 85.0: todo REAL; la salida BOOL.
    let s = spec(
        r#"{"name":"t","variables":[{"name":"Alarm","type":"EBOOL"}],"blocks":[
            {"id":"m","type":"MAX"},{"id":"g","type":"GT"}
        ],"links":[["TT1","m.IN1"],["TT2","m.IN2"],["m.OUT","g.IN1"],["85.0","g.IN2"],["g.OUT","Alarm"]]}"#,
    );
    let mut d = Diagnostics::default();
    let mut sec = resolve(&s, lib(), &mut d).unwrap();
    let vars = crate::types::infer(&mut sec, &s.variables, &mut d);
    let ty = |n: &str| vars.iter().find(|v| v.name == n).unwrap().ty.clone();
    assert_eq!(ty("TT1"), "REAL");
    assert_eq!(ty("TT2"), "REAL");
    assert_eq!(ty("Alarm"), "EBOOL");
    assert!(!d.has_errors(), "{:?}", d.0);
}

#[test]
fn untyped_variable_is_an_error() {
    let s = spec(
        r#"{"name":"t","blocks":[{"id":"m","type":"MOVE"}],"links":[["A","m.IN"],["m.OUT","B"]]}"#,
    );
    let mut d = Diagnostics::default();
    let mut sec = resolve(&s, lib(), &mut d).unwrap();
    crate::types::infer(&mut sec, &s.variables, &mut d);
    assert!(d.has_errors());
}

#[test]
fn exchange_xml_links_land_on_pins() {
    let s = spec(
        r#"{"name":"t","blocks":[{"id":"a","type":"AND"},{"id":"tmr","type":"TON","instance":"T1"}],
            "links":[["X","a.IN1"],["Y","a.IN2"],["a.OUT","tmr.IN"],["T#3s","tmr.PT"],["tmr.Q","Z"]]}"#,
    );
    let xml = section(&s).unwrap().xml;
    assert!(
        xml.contains(r#"<FFBBlock instanceName=".1" typeName="AND""#),
        "{xml}"
    );
    assert!(xml.contains(r#"<variables name="T1" typeName="TON">"#));
    assert!(xml.contains(r#"formalParameter="PT" effectiveParameter="T#3s""#));

    // Extremos del link = columna de salida (posX + width - 1) y fila posY + 4.
    let pos = |inst: &str| {
        let at = xml.find(&format!("instanceName=\"{inst}\"")).unwrap();
        let tail = &xml[at..];
        let num = |key: &str| -> i32 {
            let i = tail.find(key).unwrap() + key.len();
            tail[i..].split('"').next().unwrap().parse().unwrap()
        };
        (num("posX=\""), num("posY=\""), num("width=\""))
    };
    let ((ax, ay, aw), (tx, ty, _)) = (pos(".1"), pos("T1"));
    assert!(xml.contains(&format!(
        "<linkSource parentObjectName=\".1\" pinName=\"OUT\">\n\t\t\t\t\t\t<objPosition posX=\"{}\" posY=\"{}\">",
        ax + aw - 1,
        ay + 4
    )));
    assert!(xml.contains(&format!(
        "<linkDestination parentObjectName=\"T1\" pinName=\"IN\">\n\t\t\t\t\t\t<objPosition posX=\"{tx}\" posY=\"{}\">",
        ty + 4
    )));
}

// ---------------------------------------------------------------------------
// Etapas de programa: consistencia, partición y orden
// ---------------------------------------------------------------------------

fn run_program(json: &str) -> GeneratedProgram {
    generator().generate_program(&spec(json)).unwrap()
}

/// Diagnósticos de programa, se haya podido generar o no.
fn program_diagnostics(json: &str) -> Vec<Diagnostic> {
    match generator().generate_program(&spec(json)) {
        Ok(program) => program.diagnostics,
        Err(error) => error.diagnostics().to_vec(),
    }
}

fn messages(d: &[Diagnostic], level: Level, stage: &str) -> Vec<String> {
    d.iter()
        .filter(|x| x.level == level && x.stage == stage)
        .map(|x| x.message.clone())
        .collect()
}

/// Un equipo "motor": permisivo, temporizador y alarma. `variant` introduce desvíos.
fn motor(g: &str, variant: &str) -> (String, String) {
    let l = g.to_lowercase();
    let pt = if variant == "pt" { "T#9s" } else { "T#5s" };
    let blocks = format!(
        r#"{{"id":"{l}_a","type":"AND","group":"{g}"}},{{"id":"{l}_t","type":"TON","group":"{g}","instance":"{g}_T"}},
           {{"id":"{l}_n","type":"NOT","group":"{g}"}}{extra}"#,
        extra = if variant == "extra" {
            format!(r#",{{"id":"{l}_x","type":"NOT","group":"{g}"}}"#)
        } else {
            String::new()
        }
    );
    let links = format!(
        r#"["{g}_Auto","{l}_a.IN1"],["{g}_Ok","{l}_a.IN2"],["{l}_a.OUT","{l}_t.IN"],["{pt}","{l}_t.PT"],
           ["{l}_t.Q","{l}_n.IN"],["{l}_n.OUT","{g}_Run"]{extra}"#,
        extra = if variant == "extra" {
            format!(r#",["{g}_Aux","{l}_x.IN"],["{l}_x.OUT","{g}_AuxN"]"#)
        } else {
            String::new()
        }
    );
    (blocks, links)
}

fn plant(variants: &[(&str, &str)]) -> String {
    let (b, l): (Vec<String>, Vec<String>) = variants.iter().map(|(g, v)| motor(g, v)).unzip();
    format!(
        r#"{{"name":"Planta","blocks":[{}],"links":[{}]}}"#,
        b.join(","),
        l.join(",")
    )
}

#[test]
fn consistency_flags_deviating_equipment_and_parameters() {
    let d = program_diagnostics(&plant(&[
        ("M1", ""),
        ("M2", ""),
        ("M3", "pt"),
        ("M4", "extra"),
    ]));
    let info = messages(&d, Level::Info, "consistency");
    let warn = messages(&d, Level::Warning, "consistency");
    assert!(
        info.iter()
            .any(|m| m.contains("típico compartido por M1, M2, M3")),
        "{info:?}"
    );
    assert!(
        info.iter()
            .any(|m| m.contains("M3: m3_t.PT = T#9s") && m.contains("T#5s")),
        "{info:?}"
    );
    assert!(
        warn.iter().any(|m| m.starts_with("M4 se parece")
            && m.contains("1 NOT de más")
            && m.contains("M4_Aux")),
        "{warn:?}"
    );
}

#[test]
fn partition_by_group_cuts_wires_into_variables_in_dependency_order() {
    // M2 aparece primero en el modelo pero lee lo que escribe M1: M1 tiene que ir antes.
    let r = run_program(
        r#"{"name":"P","variables":[{"name":"X","type":"BOOL"}],"blocks":[
            {"id":"b","type":"NOT","group":"M2"},
            {"id":"a","type":"NOT","group":"M1"}
        ],"links":[["X","a.IN"],["a.OUT","b.IN"],["b.OUT","Y"]]}"#,
    );
    let names: Vec<&str> = r.sections.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["M1", "M2"]);
    let xml = |i: usize| r.sections[i].xml.clone();
    assert!(
        xml(0).contains(r#"formalParameter="OUT" effectiveParameter="a_OUT""#),
        "{}",
        xml(0)
    );
    assert!(xml(1).contains(r#"formalParameter="IN" effectiveParameter="a_OUT""#));
    // El enlace se declara (tipado) en ambas secciones.
    assert!(xml(0).contains(r#"<variables name="a_OUT" typeName="BOOL">"#));
    assert!(xml(1).contains(r#"<variables name="a_OUT" typeName="BOOL">"#));
}

#[test]
fn circular_section_dependency_is_reported() {
    let d = program_diagnostics(
        r#"{"name":"P","blocks":[
            {"id":"a","type":"NOT","group":"A"},{"id":"b","type":"NOT","group":"B"}
        ],"links":[["a.OUT","b.IN"],["b.OUT","a.IN"]]}"#,
    );
    let warn = messages(&d, Level::Warning, "partition");
    assert!(
        warn.iter().any(|m| m.contains("dependencia circular")),
        "{warn:?}"
    );
}

#[test]
fn oversized_section_is_split_along_the_flow() {
    // Cadena de 7 NOT con tope de 3 bloques: 3 secciones, cada corte cruza un solo cable.
    let blocks: Vec<String> = (0..7)
        .map(|i| format!(r#"{{"id":"n{i}","type":"NOT"}}"#))
        .collect();
    let mut links = vec![r#"["X","n0.IN"]"#.to_string()];
    links.extend((0..6).map(|i| format!(r#"["n{i}.OUT","n{}.IN"]"#, i + 1)));
    links.push(r#"["n6.OUT","Y"]"#.into());
    let r = run_program(&format!(
        r#"{{"name":"Cadena","partition":{{"maxBlocks":3}},"variables":[{{"name":"X","type":"BOOL"}}],"blocks":[{}],"links":[{}]}}"#,
        blocks.join(","),
        links.join(",")
    ));
    let names: Vec<&str> = r.sections.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["Cadena_1", "Cadena_2", "Cadena_3"]);
    let cut = r
        .diagnostics
        .iter()
        .filter(|d| d.message.contains("cruza de"))
        .count();
    assert_eq!(cut, 2);
}

#[test]
fn block_more_connected_to_another_section_is_flagged() {
    let d = program_diagnostics(
        r#"{"name":"P","variables":[{"name":"X","type":"BOOL"},{"name":"Y","type":"BOOL"}],"blocks":[
            {"id":"a","type":"AND","group":"A"},{"id":"b1","type":"NOT","group":"B"},
            {"id":"b2","type":"NOT","group":"B"},{"id":"b3","type":"NOT","group":"B"}
        ],"links":[["b1.OUT","a.IN1"],["b2.OUT","a.IN2"],["a.OUT","b3.IN"],
                   ["X","b1.IN"],["Y","b2.IN"],["b3.OUT","Z"]]}"#,
    );
    let warn = messages(&d, Level::Warning, "partition");
    assert!(
        warn.iter()
            .any(|m| m.starts_with("a está en A pero tiene 3 conexiones con B")),
        "{warn:?}"
    );
}

// ---------------------------------------------------------------------------
// Comentarios de red e IN_OUT (verificados contra reexports de Control Expert)
// ---------------------------------------------------------------------------

#[test]
fn network_comment_is_a_header_text_box_in_cells() {
    let s = spec(
        r#"{"name":"t","variables":[{"name":"A","type":"BOOL"},{"name":"B","type":"BOOL"}],
            "blocks":[{"id":"n1","type":"NOT"},{"id":"n2","type":"NOT"}],
            "links":[["A","n1.IN"],["n1.OUT","X"],["B","n2.IN"],["n2.OUT","Y"]],
            "comments":[{"at":"n2","text":"Segunda red: inversión de B"}]}"#,
    );
    let mut d = Diagnostics::default();
    let mut sec = resolve(&s, lib(), &mut d).unwrap();
    let nets = normalize::networks(&sec);
    let lay = place(&mut sec, &nets, &mut d);
    assert_eq!(lay.texts.len(), 1);
    let t = &lay.texts[0];
    // Debajo de la primera red, encima del bloque comentado y sin pisarlo.
    assert!(
        t.y >= lay.blocks[0].y + sec.blocks[0].height as i32,
        "{t:?}"
    );
    assert!(
        t.y + t.height < lay.blocks[1].y,
        "{t:?} {:?}",
        lay.blocks[1]
    );
    let xml = section(&s).unwrap().xml;
    assert!(xml.contains(&format!(
        r#"<textBox width="{}" height="{}">Segunda red: inversión de B<objPosition posX="{}" posY="{}">"#,
        t.width, t.height, t.x, t.y
    )), "{xml}");
}

#[test]
fn comment_on_unknown_block_is_an_error() {
    let s = spec(
        r#"{"name":"t","blocks":[{"id":"n","type":"NOT"}],"comments":[{"at":"nope","text":"x"}]}"#,
    );
    let mut d = Diagnostics::default();
    assert!(resolve(&s, lib(), &mut d).is_none());
}

#[test]
fn in_out_pins_are_exported_on_both_sides_in_control_expert_order() {
    // PIDFF: OUT es IN_OUT. Control Expert lo lista como entrada (con su variable) y
    // como salida, antes que las salidas comunes (OUTD...).
    let s = spec(
        r#"{"name":"t","variables":[{"name":"P","type":"Para_PIDFF"}],
            "blocks":[{"id":"pid","type":"PIDFF","instance":"TIC1"},{"id":"lim","type":"LIMIT"}],
            "links":[["PV","pid.PV"],["SP","pid.SP"],["P","pid.PARA"],["TV_Mem","pid.OUT"],
                     ["pid.OUT","lim.IN"],["0.0","lim.MN"],["100.0","lim.MX"],["lim.OUT","TV"],["pid.OUTD","D"]]}"#,
    );
    let xml = section(&s).unwrap().xml;
    let at = |needle: &str| {
        xml.find(needle)
            .unwrap_or_else(|| panic!("falta {needle}\n{xml}"))
    };
    at(r#"<inputVariable invertedPin="false" formalParameter="OUT" effectiveParameter="TV_Mem">"#);
    assert!(
        at(r#"<outputVariable invertedPin="false" formalParameter="OUT">"#)
            < at(r#"formalParameter="OUTD""#)
    );
    assert!(xml.contains(r#"<linkSource parentObjectName="TIC1" pinName="OUT">"#));
    assert!(xml.contains(r#"<variables name="TV_Mem" typeName="REAL">"#));
}

#[test]
fn feedback_cut_keeps_the_other_wires_of_the_output() {
    // run.OUT cierra el lazo hacia seal y además alimenta cmd: solo se corta el lazo.
    let s = spec(
        r#"{"name":"t","variables":[{"name":"A","type":"BOOL"}],"blocks":[
            {"id":"start","type":"AND"},{"id":"seal","type":"AND"},{"id":"run","type":"OR"},{"id":"cmd","type":"AND"}
        ],"links":[
            ["A","start.IN1"],["start.OUT","run.IN1"],["seal.OUT","run.IN2"],
            ["run.OUT","seal.IN1"],["run.OUT","cmd.IN1"],["cmd.OUT","Q"]
        ]}"#,
    );
    let mut d = Diagnostics::default();
    let mut sec = resolve(&s, lib(), &mut d).unwrap();
    normalize::normalize(&mut sec, &NormalizeConfig::default(), &mut d);
    let paths: Vec<(String, String)> = sec
        .wires
        .iter()
        .map(|w| (sec.pin_path(w.from), sec.pin_path(w.to)))
        .collect();
    assert!(
        paths.contains(&("run.OUT".into(), "cmd.IN1".into())),
        "{paths:?}"
    );
    assert!(!paths.iter().any(|(_, to)| to == "seal.IN1"), "{paths:?}");
    let run_out = sec.blocks[2].pin_index("OUT").unwrap();
    assert_eq!(
        sec.blocks[2].pins[run_out].param_out,
        Some(Param::Var("run_OUT".into()))
    );
    assert_eq!(
        sec.blocks[1].pins[0].param_in,
        Some(Param::Var("run_OUT".into()))
    );

    // El XML lleva la variable en la salida y el cable restante desde el mismo pin.
    let xml = section(&s).unwrap().xml;
    assert!(xml.contains(r#"formalParameter="OUT" effectiveParameter="run_OUT""#));
    assert!(
        xml.contains(r#"<linkSource parentObjectName=".3" pinName="OUT">"#),
        "{xml}"
    );
}

#[test]
fn each_network_comment_gets_its_own_box() {
    let s = spec(
        r#"{"name":"t","variables":[{"name":"A","type":"BOOL"}],
            "blocks":[{"id":"n1","type":"NOT"},{"id":"n2","type":"NOT"}],
            "links":[["A","n1.IN"],["n1.OUT","n2.IN"],["n2.OUT","Y"]],
            "comments":[{"at":"n1","text":"Primero"},{"at":"n2","text":"Segundo"}]}"#,
    );
    let mut d = Diagnostics::default();
    let mut sec = resolve(&s, lib(), &mut d).unwrap();
    let nets = normalize::networks(&sec);
    let lay = place(&mut sec, &nets, &mut d);
    let t: Vec<(&str, i32, i32)> = lay
        .texts
        .iter()
        .map(|t| (t.text.as_str(), t.y, t.height))
        .collect();
    assert_eq!(t.len(), 2, "{t:?}");
    assert_eq!((t[0].0, t[1].0), ("Primero", "Segundo"));
    assert!(t[0].1 + t[0].2 < t[1].1, "las cajas no se pisan: {t:?}");
    assert!(t[1].1 + t[1].2 < lay.blocks[0].y, "{t:?}");
}
