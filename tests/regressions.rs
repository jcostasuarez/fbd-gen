use std::collections::HashSet;
use std::path::Path;
use std::sync::OnceLock;

use fbd_gen::layout::LayoutConfig;
use fbd_gen::library::Library;
use fbd_gen::model::*;
use fbd_gen::normalize::{self, NormalizeConfig};
use fbd_gen::{Generator, GeneratorConfig, resolve, types};

fn lib() -> &'static Library {
    static LIB: OnceLock<Library> = OnceLock::new();
    LIB.get_or_init(|| {
        Library::load(Path::new(env!("CARGO_MANIFEST_DIR")).join("efb_dimensions.json")).unwrap()
    })
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

fn spec(json: &str) -> ProgramSpec {
    serde_json::from_str(json).unwrap()
}

#[test]
fn fanout_source_is_placed_before_consumers_declared_first() {
    let s = spec(include_str!("../examples/probes/fanout_order.json"));
    let result = generator().generate_program(&s).unwrap();
    let out = &result.sections[0];
    let json: serde_json::Value = serde_json::from_str(&out.layout_json().unwrap()).unwrap();
    assert_eq!(json["generatedVariables"][0]["name"], "src_OUT");
    let blocks = json["blocks"].as_array().unwrap();
    let source = blocks.iter().find(|b| b["id"] == "src").unwrap();
    for consumer in blocks.iter().filter(|b| b["id"] != "src") {
        assert!(source["network"].as_u64() < consumer["network"].as_u64());
        assert!(source["y"].as_i64() < consumer["y"].as_i64());
    }
    assert!(
        out.xml
            .contains(r#"formalParameter="OUT" effectiveParameter="src_OUT""#)
    );
}

#[test]
fn fanout_order_does_not_reintroduce_feedback_dependencies() {
    let s = spec(
        r#"{"name":"Loop","blocks":[
        {"id":"a","type":"NOT"},{"id":"b","type":"NOT"}
    ],"links":[["a.OUT","b.IN"],["b.OUT","a.IN"]]}"#,
    );
    let mut d = Diagnostics::default();
    let mut section = resolve::resolve(&s, lib(), &mut d).unwrap();
    normalize::normalize(&mut section, &NormalizeConfig { max_fanout: 0 }, &mut d);
    assert!(section.wires.is_empty());
    assert_eq!(normalize::networks(&section), [vec![0], vec![1]]);
    assert_eq!(section.generated_vars.len(), 2);
}

#[test]
fn reconverging_fanout_keeps_wires_when_network_order_is_not_enough() {
    // Al quitar Q -> b, a y b seguirían en la misma red a través de sel.
    // El layout no vería esa dependencia y podría ejecutar b antes de a.
    let s = spec(
        r#"{"name":"Reconvergence","blocks":[
        {"id":"a","type":"TON"},{"id":"b","type":"NOT"},
        {"id":"sel","type":"SEL"},{"id":"d","type":"NOT"}
    ],"links":[["TRUE","a.IN"],["T#1s","a.PT"],
        ["a.Q","b.IN"],["a.Q","d.IN"],["a.ET","sel.IN0"],
        ["b.OUT","sel.G"],["T#2s","sel.IN1"]]}"#,
    );
    let mut d = Diagnostics::default();
    let mut section = resolve::resolve(&s, lib(), &mut d).unwrap();
    normalize::normalize(&mut section, &NormalizeConfig { max_fanout: 1 }, &mut d);
    assert_eq!(section.wires.len(), 4);
    assert!(section.generated_vars.is_empty());
    assert_eq!(normalize::networks(&section), [vec![0, 1, 2, 3]]);
    assert!(
        d.0.iter()
            .any(|d| d.message.contains("se conserva fan-out"))
    );
}

fn partitioned(json: &str) -> fbd_gen::partition::Partition {
    let s = spec(json);
    let mut d = Diagnostics::default();
    let mut section = resolve::resolve(&s, lib(), &mut d).unwrap();
    let (vars, pins) = types::infer_all(&mut section, &s.variables, &mut d);
    let result = fbd_gen::partition::partition(&s, &section, &vars, &pins, &mut d);
    assert!(!d.has_errors(), "{:?}", d.0);
    let unique: HashSet<String> = result
        .sections
        .iter()
        .map(|s| s.name.to_ascii_uppercase())
        .collect();
    assert_eq!(unique.len(), result.sections.len());
    result
}

#[test]
fn split_section_names_cannot_overwrite_existing_groups() {
    let result = partitioned(
        r#"{"name":"P","partition":{"maxBlocks":1},"blocks":[
        {"id":"a","type":"NOT","group":"A"},{"id":"b","type":"NOT","group":"A"},
        {"id":"c","type":"NOT","group":"A_1"},{"id":"d","type":"NOT","group":"a_1"},
        {"id":"e","type":"NOT","group":"A_1_2"}
    ]}"#,
    );
    assert_eq!(result.sections.len(), 5);
    let ids: HashSet<&str> = result
        .sections
        .iter()
        .flat_map(|s| &s.blocks)
        .map(|b| b.id.as_str())
        .collect();
    assert_eq!(ids, HashSet::from(["a", "b", "c", "d", "e"]));
    assert_eq!(result.sections.last().unwrap().name, "A_1_2");
}

#[test]
fn sanitizing_and_concatenating_names_does_not_merge_distinct_groups() {
    let result = partitioned(
        r#"{"name":"P","partition":{"byStage":true},"blocks":[
        {"id":"a","type":"NOT","group":"P-A"},{"id":"b","type":"NOT","group":"P_A"},
        {"id":"c","type":"NOT","group":"A_B","stage":"C"},
        {"id":"d","type":"NOT","group":"A","stage":"B_C"}
    ]}"#,
    );
    assert_eq!(result.sections.len(), 4);
    assert!(result.sections.iter().all(|s| s.blocks.len() == 1));
}

#[test]
fn scalar_generic_families_reject_incompatible_types() {
    for (block, pin, ty, family) in [
        ("NOT", "IN", "REAL", "ANY_BIT"),
        ("NOT", "IN", "DINT", "ANY_BIT"),
        ("ADD", "IN1", "BOOL", "ANY_NUM"),
        ("MUX", "K", "REAL", "ANY_INT"),
        ("SQRT", "IN", "BOOL", "ANY_REAL"),
        ("MAX", "IN1", "Para_PIDFF", "ANY_ELEMENTARY"),
    ] {
        let s = spec(&format!(
            r#"{{"name":"P","variables":[{{"name":"X","type":"{ty}"}}],
            "blocks":[{{"id":"b","type":"{block}"}}],"links":[["X","b.{pin}"]]}}"#
        ));
        let error = generator()
            .generate_program(&s)
            .expect_err(&format!("{block} aceptó {ty}"));
        assert!(
            error
                .diagnostics()
                .iter()
                .any(|d| d.level == Level::Error && d.message.contains(family)),
            "{:?}",
            error.diagnostics()
        );
    }
}

#[test]
fn scalar_generic_families_accept_valid_types_and_any_accepts_ddts() {
    for (block, pin, ty) in [
        ("NOT", "IN", "BOOL"),
        ("NOT", "IN", "EBOOL"),
        ("NOT", "IN", "BYTE"),
        ("NOT", "IN", "WORD"),
        ("NOT", "IN", "DWORD"),
        ("ADD", "IN1", "INT"),
        ("ADD", "IN1", "REAL"),
        ("MUX", "K", "INT"),
        ("SQRT", "IN", "REAL"),
        ("MAX", "IN1", "DATE"),
        ("MOVE", "IN", "Para_PIDFF"),
    ] {
        let s = spec(&format!(
            r#"{{"name":"P","variables":[{{"name":"X","type":"{ty}"}}],
            "blocks":[{{"id":"b","type":"{block}"}}],"links":[["X","b.{pin}"]]}}"#
        ));
        let mut d = Diagnostics::default();
        let mut section = resolve::resolve(&s, lib(), &mut d).unwrap();
        types::infer(&mut section, &s.variables, &mut d);
        assert!(!d.has_errors(), "{block} / {ty}: {:?}", d.0);
    }
}

#[test]
fn generic_constraints_check_literals_without_variables_and_all_strong_types() {
    for json in [
        r#"{"name":"P","blocks":[{"id":"n","type":"NOT"}],"links":[["REAL#1.5","n.IN"]]}"#,
        r#"{"name":"P","variables":[{"name":"X","type":"BOOL"},{"name":"Y","type":"REAL"}],
            "blocks":[{"id":"n","type":"NOT"}],"links":[["X","n.IN"],["n.OUT","Y"]]}"#,
    ] {
        let s = spec(json);
        let mut d = Diagnostics::default();
        let mut section = resolve::resolve(&s, lib(), &mut d).unwrap();
        types::infer(&mut section, &s.variables, &mut d);
        assert!(
            d.0.iter()
                .any(|d| d.level == Level::Error
                    && d.message.contains("REAL no pertenece a ANY_BIT")),
            "{:?}",
            d.0
        );
    }
}

#[test]
fn typed_literals_infer_output_variables_through_generic_blocks() {
    for (literal, ty) in [
        ("REAL#1.5", "REAL"),
        ("lreal#1.5", "LREAL"),
        ("INT#3", "INT"),
        ("DINT#3", "DINT"),
        ("UINT#3", "UINT"),
        ("UDINT#3", "UDINT"),
        ("BOOL#TRUE", "BOOL"),
        ("BYTE#16#FF", "BYTE"),
        ("WORD#16#FFFF", "WORD"),
        ("DWORD#16#FFFFFFFF", "DWORD"),
        ("STRING#'abc'", "STRING"),
        ("T#5s", "TIME"),
        ("TIME#5s", "TIME"),
        ("D#2026-10-17", "DATE"),
        ("DATE#2026-10-17", "DATE"),
        ("TOD#12:30:00", "TOD"),
        ("TIME_OF_DAY#12:30:00", "TOD"),
        ("DT#2026-10-17-12:30:00", "DT"),
        ("DATE_AND_TIME#2026-10-17-12:30:00", "DT"),
    ] {
        let s = spec(&format!(
            r#"{{"name":"P","blocks":[{{"id":"a","type":"MOVE"}},{{"id":"b","type":"MOVE"}}],
            "links":[["{literal}","a.IN"],["a.OUT","b.IN"],["b.OUT","Y"]]}}"#
        ));
        let mut d = Diagnostics::default();
        let mut section = resolve::resolve(&s, lib(), &mut d).unwrap();
        let vars = types::infer(&mut section, &s.variables, &mut d);
        assert!(!d.has_errors(), "{literal}: {:?}", d.0);
        assert_eq!(
            vars.iter().find(|v| v.name == "Y").unwrap().ty,
            ty,
            "{literal}"
        );
    }
    // El caso original también debe completar partición, layout y exportación.
    let s = spec(
        r#"{"name":"P","blocks":[{"id":"m","type":"MOVE"}],
        "links":[["REAL#1.5","m.IN"],["m.OUT","Y"]]}"#,
    );
    let result = generator().generate_program(&s).unwrap();
    assert!(
        result.sections[0]
            .xml
            .contains(r#"<variables name="Y" typeName="REAL">"#)
    );
}

#[test]
fn numeric_base_prefix_is_not_mistaken_for_a_type() {
    let s = spec(
        r#"{"name":"P","variables":[{"name":"Y","type":"WORD"}],
        "blocks":[{"id":"m","type":"MOVE"}],"links":[["16#FF","m.IN"],["m.OUT","Y"]]}"#,
    );
    let mut d = Diagnostics::default();
    let mut section = resolve::resolve(&s, lib(), &mut d).unwrap();
    let vars = types::infer(&mut section, &s.variables, &mut d);
    assert!(!d.has_errors(), "{:?}", d.0);
    assert_eq!(vars[0].ty, "WORD");
}

#[test]
fn hostile_inputs_are_rejected_before_layout() {
    for (json, expected) in [
        // El nombre termina en archivos: no puede salir de la carpeta de salida.
        (
            r#"{"name":"../fuera","blocks":[{"id":"a","type":"NOT"}]}"#,
            "identificador IEC",
        ),
        (
            r#"{"name":"","blocks":[{"id":"a","type":"NOT"}]}"#,
            "identificador IEC",
        ),
        (r#"{"name":"Vacio","blocks":[]}"#, "no tiene bloques"),
        (
            r#"{"name":"T","blocks":[{"id":"a.b","type":"NOT"}]}"#,
            "id de bloque inválido",
        ),
        (
            r#"{"name":"T","blocks":[{"id":"t","type":"TON","instance":"T 1"}]}"#,
            "nombre de instancia inválido",
        ),
        (
            r#"{"name":"T","blocks":[{"id":"n","type":"NOT"}],"links":[["mi var","n.IN"]]}"#,
            "variable válida",
        ),
        (
            r#"{"name":"T","blocks":[{"id":"n","type":"NOT"}],"links":[["X","n.IN"],["n.OUT","Y<1>"]]}"#,
            "variable válida",
        ),
        // Antes reservaba miles de millones de pines y el proceso abortaba.
        (
            r#"{"name":"T","blocks":[{"id":"a","type":"AND","inputs":4000000000}]}"#,
            "máximo",
        ),
    ] {
        let error = generator().generate_program(&spec(json)).expect_err(json);
        assert!(
            error
                .diagnostics()
                .iter()
                .any(|d| d.level == Level::Error && d.message.contains(expected)),
            "{json}: {:?}",
            error.diagnostics()
        );
    }
}

#[test]
fn structured_and_topological_variables_are_accepted() {
    for name in [
        "Motor.Run",
        "Tab[3]",
        "Planta.Bomba[2].Run",
        "%MW100",
        "%I0.1.2",
    ] {
        assert!(is_variable_ref(name), "{name}");
    }
    for name in ["mi var", "Y<1>", "a..b", "Tab[]", "%", "1abc", ""] {
        assert!(!is_variable_ref(name), "{name}");
    }
}

#[test]
fn unknown_fields_in_the_input_are_rejected() {
    // Un typo del LLM no debe ignorarse: `instace` dejaría la instancia con el id.
    for json in [
        r#"{"name":"T","blocks":[{"id":"t","type":"TON","instace":"Mi_Timer"}]}"#,
        r#"{"name":"T","blocks":[{"id":"t","type":"TON"}],"partiton":{"maxBlocks":1}}"#,
        r#"{"name":"T","blocks":[{"id":"t","type":"TON"}],"partition":{"maxBlock":1}}"#,
        r#"{"name":"T","blocks":[{"id":"t","type":"TON"}],"variables":[{"name":"V","type":"BOOL","adress":"%M1"}]}"#,
        r#"{"name":"T","blocks":[{"id":"t","type":"TON"}],"comments":[{"at":"t","txt":"x"}]}"#,
        r#"{"name":"T","blocks":[{"id":"t","type":"TON"}],"links":[{"from":"A","to":"t.IN","via":"x"}]}"#,
    ] {
        let error = serde_json::from_str::<ProgramSpec>(json).unwrap_err();
        assert!(
            error.to_string().contains("unknown field")
                || error.to_string().contains("did not match any variant"),
            "{json}: {error}"
        );
    }
    let ok: ProgramSpec = serde_json::from_str(
        r#"{"name":"T","blocks":[{"id":"t","type":"TON"}],"links":[{"from":"A","to":"t.IN"}]}"#,
    )
    .unwrap();
    assert_eq!(ok.links[0].ends(), ("A", "t.IN"));
}

#[test]
fn incompatible_elementary_types_are_errors_before_generating_xml() {
    // BOOL -> variable -> TIME: Control Expert no compila esa sección.
    let through_variable = spec(
        r#"{"name":"T","blocks":[{"id":"a","type":"TON"},{"id":"b","type":"TON"}],
        "links":[["Start","a.IN"],["T#5s","a.PT"],["a.Q","X"],["Start","b.IN"],["X","b.PT"]]}"#,
    );
    let error = generator().generate_program(&through_variable).unwrap_err();
    let errors: Vec<&str> = error
        .diagnostics()
        .iter()
        .filter(|d| d.level == Level::Error)
        .map(|d| d.message.as_str())
        .collect();
    assert!(
        errors
            .iter()
            .any(|m| m.starts_with("X: tipos incompatibles") && m.contains("a.Q")),
        "{errors:?}"
    );

    // Lo mismo con un cable directo y con un literal tipado.
    for json in [
        r#"{"name":"T","blocks":[{"id":"a","type":"TON"},{"id":"b","type":"TON"}],
        "links":[["Start","a.IN"],["T#5s","a.PT"],["a.Q","b.PT"],["Start","b.IN"]]}"#,
        r#"{"name":"T","blocks":[{"id":"a","type":"TON"}],
        "links":[["Start","a.IN"],["INT#5","a.PT"]]}"#,
    ] {
        let error = generator().generate_program(&spec(json)).unwrap_err();
        assert!(
            error
                .diagnostics()
                .iter()
                .any(|d| d.level == Level::Error && d.message.contains("tipos incompatibles")),
            "{json}: {:?}",
            error.diagnostics()
        );
    }
}

#[test]
fn ebool_and_bool_are_compatible() {
    let s = spec(
        r#"{"name":"T","variables":[{"name":"In1","type":"EBOOL"}],
        "blocks":[{"id":"a","type":"TON"}],
        "links":[["In1","a.IN"],["T#5s","a.PT"],["a.Q","Out1"]]}"#,
    );
    let result = generator().generate_program(&s).unwrap();
    let all = result
        .diagnostics
        .iter()
        .chain(result.sections.iter().flat_map(|s| &s.diagnostics));
    assert!(
        all.into_iter()
            .all(|d| !d.message.contains("incompatibles"))
    );
}

#[test]
fn derived_type_mismatch_is_only_a_warning() {
    // Los alias de arrays/DDT no se pueden decidir sin el diccionario del proyecto.
    let s = spec(
        r#"{"name":"T","variables":[{"name":"V","type":"ARRUINT2"}],
        "blocks":[{"id":"a","type":"TON"}],
        "links":[["Start","a.IN"],["V","a.PT"],["a.Q","Out1"]]}"#,
    );
    // TIME (pin) contra ARRUINT2 (declaración): uno es derivado, así que se genera
    // igual y queda un aviso.
    let result = generator().generate_program(&s).unwrap();
    assert!(
        result
            .diagnostics
            .iter()
            .any(|d| d.level == Level::Warning && d.message.contains("tipos incompatibles"))
    );
}
