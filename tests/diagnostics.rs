//! Contrato de los diagnósticos para programas: código estable y ubicación en términos de
//! la entrada (ids de bloque, índices de link y comentario, variables, equipos).

use std::collections::HashSet;
use std::path::Path;

use fbd_gen::{
    Diagnostic, DiagnosticCode, DiagnosticLocation, FunctionCallSpec, Generator, Level, ProgramSpec,
};

fn generator() -> Generator {
    Generator::from_library_file(Path::new(env!("CARGO_MANIFEST_DIR")).join("efb_dimensions.json"))
        .expect("efb_dimensions.json es la librería del repositorio")
}

fn spec(json: &str) -> ProgramSpec {
    serde_json::from_str(json).unwrap_or_else(|e| panic!("JSON de prueba inválido: {e}\n{json}"))
}

/// Diagnósticos de una entrada inválida.
fn rejected(json: &str) -> Vec<Diagnostic> {
    generator()
        .generate_program(&spec(json))
        .expect_err("la entrada tenía que ser inválida")
        .diagnostics()
        .to_vec()
}

/// Diagnósticos de un programa válido: los del programa y los de cada sección.
fn accepted(json: &str) -> Vec<Diagnostic> {
    let program = generator()
        .generate_program(&spec(json))
        .unwrap_or_else(|e| panic!("la entrada tenía que ser válida: {e}"));
    let mut all = program.diagnostics;
    for s in program.sections {
        all.extend(s.diagnostics);
    }
    all
}

fn find(diags: &[Diagnostic], code: DiagnosticCode) -> &Diagnostic {
    diags
        .iter()
        .find(|d| d.code == code)
        .unwrap_or_else(|| panic!("falta {code}: {diags:#?}"))
}

fn location(
    block: Option<&str>,
    pin: Option<&str>,
    link: Option<usize>,
    variable: Option<&str>,
) -> DiagnosticLocation {
    let mut l = DiagnosticLocation::default();
    l.block = block.map(Into::into);
    l.pin = pin.map(Into::into);
    l.link = link;
    l.variable = variable.map(Into::into);
    l
}

#[test]
fn link_errors_point_to_the_link_block_and_pin() {
    let diags = rejected(
        r#"{"name":"P","blocks":[{"id":"n","type":"NOT"}],
            "links":[["TRUE","n.FOO"],["A","n.IN"],["B","n.IN"],["TRUE","Y"]],
            "comments":[{"at":"zz","text":"c"}]}"#,
    );
    assert!(diags.iter().all(|d| d.level == Level::Error));

    let unknown = find(&diags, DiagnosticCode::UnknownPin);
    assert_eq!(
        unknown.location,
        location(Some("n"), Some("FOO"), Some(0), None)
    );

    let driven = find(&diags, DiagnosticCode::InputAlreadyDriven);
    assert_eq!(
        driven.location,
        location(Some("n"), Some("IN"), Some(2), None)
    );

    let no_block = find(&diags, DiagnosticCode::NoBlockEndpoint);
    assert_eq!(no_block.location, location(None, None, Some(3), None));

    let comment = find(&diags, DiagnosticCode::UnknownCommentTarget);
    assert_eq!(comment.location.comment, Some(0));
}

#[test]
fn block_and_variable_errors_are_located() {
    let diags = rejected(
        r#"{"name":"P","variables":[{"name":"V","type":"WORD","address":"MW100"}],
            "blocks":[{"id":"x","type":"NoExiste"},{"id":"X","type":"NOT"}]}"#,
    );
    assert_eq!(
        find(&diags, DiagnosticCode::UnknownBlockType).location,
        location(Some("x"), None, None, None)
    );
    assert_eq!(
        find(&diags, DiagnosticCode::DuplicateBlockId).location,
        location(Some("X"), None, None, None)
    );
    assert_eq!(
        find(&diags, DiagnosticCode::InvalidAddress).location,
        location(None, None, None, Some("V"))
    );
}

#[test]
fn type_errors_point_to_the_variable_or_the_pin() {
    let diags = rejected(
        r#"{"name":"P","blocks":[{"id":"t","type":"TON"},{"id":"m","type":"MOVE"}],
            "links":[["INT#3","t.PT"],["X","m.IN"],["m.OUT","Y"]]}"#,
    );
    // Sin variables en la clase, el conflicto se ubica en el pin que impuso el tipo.
    let mismatch = find(&diags, DiagnosticCode::TypeMismatch);
    assert_eq!(mismatch.location.block.as_deref(), Some("t"));
    assert_eq!(mismatch.location.pin.as_deref(), Some("PT"));

    let undeducible: Vec<_> = diags
        .iter()
        .filter(|d| d.code == DiagnosticCode::UndeducibleType)
        .map(|d| d.location.variable.as_deref().unwrap())
        .collect();
    assert_eq!(undeducible, ["X", "Y"]);
}

#[test]
fn successful_programs_carry_located_warnings_and_info() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let json = std::fs::read_to_string(root.join("examples/planta_bombas.json")).unwrap();
    let diags = accepted(&json);

    // P103 tiene un desvío del típico a propósito y P104 otro tiempo.
    let deviation = find(&diags, DiagnosticCode::TypicalDeviation);
    assert_eq!(deviation.level, Level::Warning);
    assert_eq!(deviation.location.group.as_deref(), Some("P103"));
    let outlier = find(&diags, DiagnosticCode::LiteralOutlier);
    assert_eq!(outlier.location.group.as_deref(), Some("P104"));
    assert_eq!(outlier.location.pin.as_deref(), Some("PT"));

    let cut = find(&diags, DiagnosticCode::FeedbackCut);
    assert!(cut.location.block.is_some() && cut.location.variable.is_some());
    assert!(
        find(&diags, DiagnosticCode::SectionOrder)
            .location
            .is_empty()
    );
    find(&diags, DiagnosticCode::CrossSectionLink);
}

#[test]
fn function_call_errors_point_to_the_pin() {
    let call = FunctionCallSpec::new("Call", "TON").input("NOPE", "A");
    let error = generator().generate_function(&call).unwrap_err();
    let d = find(error.diagnostics(), DiagnosticCode::UnknownPin);
    assert_eq!(d.location.pin.as_deref(), Some("NOPE"));

    let call = FunctionCallSpec::new("Call", "AND").with_instance("I");
    let error = generator().generate_function(&call).unwrap_err();
    find(error.diagnostics(), DiagnosticCode::InstanceNotAllowed);
}

#[test]
fn codes_have_unique_stable_snake_case_names() {
    let mut seen = HashSet::new();
    for &code in DiagnosticCode::ALL {
        let name = code.as_str();
        assert!(seen.insert(name), "{name} repetido");
        assert!(
            name.chars().all(|c| c.is_ascii_lowercase() || c == '_'),
            "{name} no es snake_case"
        );
        assert_eq!(code.to_string(), name);
        assert_eq!(serde_json::to_value(code).unwrap(), name);
    }
}

#[test]
fn serialized_diagnostics_omit_empty_location_fields() {
    let diags =
        rejected(r#"{"name":"P","blocks":[{"id":"n","type":"NOT"}],"links":[["TRUE","n.FOO"]]}"#);
    let json = serde_json::to_value(find(&diags, DiagnosticCode::UnknownPin)).unwrap();
    assert_eq!(
        json,
        serde_json::json!({
            "level": "error",
            "stage": "validate",
            "code": "unknown_pin",
            "location": {"block": "n", "pin": "FOO", "link": 0},
            "message": "n: el bloque NOT no tiene el pin FOO",
        })
    );

    let diags = rejected(r#"{"name":"P","blocks":[]}"#);
    let json = serde_json::to_value(find(&diags, DiagnosticCode::EmptyGraph)).unwrap();
    assert!(json.get("location").is_none(), "{json}");
}

#[test]
fn malformed_json_is_an_invalid_input_with_its_own_code() {
    for (json, hint) in [
        (
            r#"{"name":"P","blocks":[{"id":"n","type":"NOT","instace":"x"}]}"#,
            "instace",
        ),
        (r#"{"name":"P","blocks":[{"id":"n"}]}"#, "type"),
        (r#"{"name":"P","blocks":["#, "EOF"),
    ] {
        let error = ProgramSpec::from_json(json).unwrap_err();
        let d = find(error.diagnostics(), DiagnosticCode::InvalidJson);
        assert_eq!(d.level, Level::Error);
        assert!(d.message.contains(hint), "{}", d.message);
        assert!(d.location.is_empty());
    }
    let spec =
        ProgramSpec::from_json(r#"{"name":"P","blocks":[{"id":"n","type":"NOT"}]}"#).unwrap();
    assert_eq!(spec.blocks.len(), 1);
}

/// Cada nivel de la cadena `source()` aporta su propio mensaje: ninguno repite el de su
/// causa, así un formateador de cadenas (`anyhow` con `{:#}`) no lo muestra dos veces.
#[test]
fn error_chains_do_not_repeat_their_causes() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let errors = [
        Generator::from_library_file(root.join("no_existe.json")).unwrap_err(),
        Generator::from_library_file(root.join("README.md")).unwrap_err(),
    ];
    for error in errors {
        let mut chain = vec![error.to_string()];
        let mut source = std::error::Error::source(&error);
        while let Some(s) = source {
            chain.push(s.to_string());
            source = s.source();
        }
        assert!(chain.len() >= 2, "sin causa: {chain:?}");
        for pair in chain.windows(2) {
            assert!(!pair[0].contains(&pair[1]), "causa repetida: {chain:?}");
        }
    }
}
