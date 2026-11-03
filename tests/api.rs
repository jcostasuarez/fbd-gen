//! API pública: entradas, errores tipados y registro de bloques propios.

use std::path::Path;

use fbd_gen::{
    BlockDefinition, BlockSpec, GenerationError, Generator, GeneratorConfig, Library, LibraryError,
    LinkSpec, ProgramSpec, SectionSpec,
};

fn generator() -> Generator {
    Generator::from_library_file(Path::new(env!("CARGO_MANIFEST_DIR")).join("efb_dimensions.json"))
        .expect("efb_dimensions.json es la librería del repositorio")
}

#[test]
fn owned_results_expose_layout_and_all_variables_without_json_parsing() {
    let result = {
        let generator = generator();
        let program: ProgramSpec =
            serde_json::from_str(include_str!("../examples/p101_motor.json")).unwrap();
        generator.generate_program(&program).unwrap()
    };
    assert_eq!(result.name, "P101_Motor");
    assert_eq!(result.sections.len(), 1);
    let section = &result.sections[0];
    assert!(!section.layout.blocks.is_empty());
    assert!(!section.layout.links.is_empty());
    assert!(section.variables.iter().any(|v| v.ty == "TON"));
    for var in &section.variables {
        assert!(section.xml.contains(&format!(
            "<variables name=\"{}\" typeName=\"{}\"",
            var.name, var.ty
        )));
    }
    let json: serde_json::Value = serde_json::from_str(&section.layout_json().unwrap()).unwrap();
    assert_eq!(json["blocks"][0]["id"], section.layout.blocks[0].id);
    assert_eq!(
        serde_json::to_value(&result).unwrap()["sections"][0]["name"],
        section.name
    );
    assert!(section.svg.starts_with("<svg"));
}

#[test]
fn program_and_section_have_distinct_partition_contracts() {
    let json = r#"{"name":"P","partition":{"maxBlocks":1},"blocks":[
        {"id":"a","type":"NOT"},{"id":"b","type":"NOT"}
    ],"links":[["BOOL#TRUE","a.IN"],["a.OUT","b.IN"]]}"#;
    assert!(serde_json::from_str::<SectionSpec>(json).is_err());
    let spec: ProgramSpec = serde_json::from_str(json).unwrap();
    let result = generator().generate_program(&spec).unwrap();
    assert_eq!(result.sections.len(), 2);

    let mut spec = SectionSpec::new("Single");
    let mut a = BlockSpec::new("a", "NOT");
    a.group = Some("A".into());
    let mut b = BlockSpec::new("b", "NOT");
    b.group = Some("B".into());
    spec.blocks = vec![a, b];
    spec.links = vec![
        LinkSpec::Pair("BOOL#TRUE".into(), "a.IN".into()),
        LinkSpec::Pair("a.OUT".into(), "b.IN".into()),
    ];
    let result = generator().generate_section(&spec).unwrap();
    assert_eq!(result.name, "Single");
    assert_eq!(result.layout.blocks.len(), 2);
}

#[test]
fn input_specifications_can_be_serialized_for_clients() {
    let mut spec = ProgramSpec::new("P");
    spec.blocks.push(BlockSpec::new("n", "NOT"));
    let json = serde_json::to_string(&spec).unwrap();
    let decoded: ProgramSpec = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded.blocks[0].ty, "NOT");
    assert_eq!(decoded.task, "MAST");
}

#[test]
fn invalid_graphs_return_typed_errors() {
    let generator = generator();
    let mut spec = ProgramSpec::new("Bad");
    spec.blocks.push(BlockSpec::new("n", "MissingBlock"));
    let error = generator.generate_program(&spec).unwrap_err();
    assert!(!error.diagnostics().is_empty());
    assert!(matches!(error, GenerationError::InvalidInput { ref scope, .. } if scope == "Bad"));

    let spec: SectionSpec = serde_json::from_str(
        r#"{"name":"BadType","blocks":[{"id":"n","type":"NOT"}],"links":[["REAL#1.5","n.IN"]]}"#,
    )
    .unwrap();
    assert!(matches!(
        generator.generate_section(&spec),
        Err(GenerationError::InvalidInput { .. })
    ));
}

#[test]
fn invalid_configuration_is_rejected() {
    let mut config = GeneratorConfig::default();
    config.layout.text_chars_per_10_cells = 0;
    assert!(matches!(
        generator().with_config(config),
        Err(GenerationError::InvalidConfig { .. })
    ));
    let mut config = GeneratorConfig::default();
    config.consistency.similarity = 2.0;
    assert!(matches!(
        generator().with_config(config),
        Err(GenerationError::InvalidConfig { .. })
    ));
}

#[test]
fn catalog_read_errors_are_typed_and_keep_the_source() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("no_such_catalog_api_test.json");
    let error = Generator::from_library_file(path).unwrap_err();
    assert!(std::error::Error::source(&error).is_some());
    assert!(matches!(
        error,
        GenerationError::Library(LibraryError::Io { .. })
    ));
}

#[test]
fn base_catalog_is_validated_like_registered_blocks() {
    let block = |name: &str, kind: &str, pin: &str| {
        format!(
            r#"{{"name":"{name}","kind":"{kind}","geometry":{{"inputs":[{{"name":"{pin}","type":"BOOL","position":1}}]}},"size":{{"width":7}}}}"#
        )
    };
    let catalog = |blocks: &[String]| format!(r#"{{"blocks":[{}]}}"#, blocks.join(","));
    let invalid = |json: String| match Library::from_json_str(&json) {
        Err(LibraryError::InvalidDefinition { block, message }) => format!("{block}: {message}"),
        Err(LibraryError::DuplicateBlock { name }) => format!("duplicado {name}"),
        other => panic!("{other:?}"),
    };

    assert!(Library::from_json_str(&catalog(&[block("MI_EF", "EF", "IN")])).is_ok());
    assert_eq!(
        invalid(catalog(&[block("MI_FB", "FB", "IN")])),
        r#"MI_FB: tipo de bloque desconocido "FB" (EF, EFB o DFB)"#
    );
    assert_eq!(
        invalid(catalog(&[
            block("MI_EF", "EF", "IN"),
            block("mi_ef", "EF", "IN")
        ])),
        "duplicado mi_ef"
    );
    assert!(invalid(catalog(&[block("MI_EF", "EF", "EN")])).starts_with("MI_EF: pin EN"));
}

#[test]
fn registered_dfb_exports_instance_wires_variables_and_both_in_out_sides() {
    let mut generator = generator();
    generator
        .register_block(
            BlockDefinition::dfb("MotorPropio", 12)
                .input("Enable", "BOOL", 1)
                .input("Reset", "BOOL", 2)
                .output("Running", "BOOL", 1)
                .in_out("State", "INT", 3),
        )
        .unwrap();
    let program: ProgramSpec =
        serde_json::from_str(include_str!("../examples/custom_dfb.json")).unwrap();
    let result = generator.generate_program(&program).unwrap();
    let section = &result.sections[0];
    let motor = section
        .layout
        .blocks
        .iter()
        .find(|b| b.id == "motor")
        .unwrap();
    assert_eq!(motor.instance.as_deref(), Some("P201_Motor"));
    assert_eq!(motor.width, 12);
    assert!(
        section
            .variables
            .iter()
            .any(|v| v.name == "P201_Motor" && v.ty == "MotorPropio")
    );
    assert!(
        section
            .variables
            .iter()
            .any(|v| v.name == "Estado" && v.ty == "INT" && v.init.as_deref() == Some("0"))
    );
    assert!(
        section
            .variables
            .iter()
            .any(|v| v.name == "Habilitar" && v.ty == "BOOL")
    );
    assert!(section.xml.contains(
        r#"<inputVariable invertedPin="false" formalParameter="State" effectiveParameter="Estado""#
    ));
    assert!(
        section
            .xml
            .contains(r#"<outputVariable invertedPin="false" formalParameter="State""#)
    );
    assert!(
        section
            .xml
            .contains(r#"<linkSource parentObjectName="P201_Motor" pinName="Running""#)
    );
    assert!(
        !section.xml.contains("DFBSource"),
        "se exporta la instancia, no la lógica de la DFB"
    );
}

#[test]
fn external_catalog_json_and_file_support_the_same_definition() {
    let mut generator = Generator::new(Library::default());
    generator
        .register_blocks_from_file(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/custom_blocks.json"),
        )
        .unwrap();
    let mut library = Library::default();
    library
        .extend_from_json(include_str!("../examples/custom_blocks.json"))
        .unwrap();
    assert_eq!(
        generator.library().get("motorpropio").unwrap().width,
        library.get("MOTORPROPIO").unwrap().width
    );
    let json = serde_json::to_string(library.get("MotorPropio").unwrap()).unwrap();
    let definition: BlockDefinition = serde_json::from_str(&json).unwrap();
    assert_eq!(definition.in_outs[0].name, "State");
}

#[test]
fn duplicate_registration_is_case_insensitive_and_atomic() {
    let mut library = Library::default();
    library
        .register(BlockDefinition::dfb("Custom", 10).input("IN", "BOOL", 1))
        .unwrap();
    let error = library
        .register_all(vec![
            BlockDefinition::dfb("New", 10),
            BlockDefinition::dfb("custom", 20),
        ])
        .unwrap_err();
    assert!(matches!(error, LibraryError::DuplicateBlock { .. }));
    assert_eq!(library.len(), 1);
    assert!(library.get("New").is_none());
    assert_eq!(library.get("Custom").unwrap().width, 10);
    let mut generator = generator();
    assert!(matches!(
        generator.register_block(BlockDefinition::dfb("ton", 10)),
        Err(LibraryError::DuplicateBlock { .. })
    ));
}

#[test]
fn invalid_dfb_interfaces_do_not_enter_the_catalog() {
    for definition in [
        BlockDefinition::dfb("Bad Name", 10),
        BlockDefinition::dfb("Bad", 0),
        BlockDefinition::dfb("Bad", 10).input("X", "", 1),
        BlockDefinition::dfb("Bad", 10).input("X", "BO\u{1}OL", 1),
        BlockDefinition::dfb("Bad", 10).input("X", "BOOL", 0),
        BlockDefinition::dfb("Bad", 10).input("X", "BOOL", u32::MAX),
        // Tope de 1000 celdas: un ancho o una fila absurda desbordaría el layout.
        BlockDefinition::dfb("Bad", 1001),
        BlockDefinition::dfb("Bad", i32::MAX.unsigned_abs()),
        BlockDefinition::dfb("Bad", 10).input("X", "BOOL", 997),
        BlockDefinition::dfb("Bad", 10).input("EN", "BOOL", 1),
        BlockDefinition::dfb("Bad", 10)
            .input("X", "BOOL", 1)
            .output("x", "BOOL", 2),
        BlockDefinition::dfb("Bad", 10)
            .input("X", "BOOL", 1)
            .in_out("Y", "INT", 1),
    ] {
        let mut library = Library::default();
        assert!(matches!(
            library.register(definition),
            Err(LibraryError::InvalidDefinition { .. })
        ));
        assert!(library.is_empty());
    }
    let mut library = Library::default();
    assert!(matches!(
        library.extend_from_json(
            r#"{"blocks":[{"name":"Typo","kind":"DFB","width":10,"inputz":[] }]}"#
        ),
        Err(LibraryError::Json(_))
    ));
    assert!(library.is_empty());
}

#[test]
fn block_dimensions_up_to_the_cap_are_accepted() {
    let mut library = Library::default();
    library
        .register(BlockDefinition::dfb("Wide", 1000).input("X", "BOOL", 996))
        .unwrap();
    assert_eq!(library.len(), 1);
}

#[test]
fn custom_ef_definitions_can_be_registered_without_instances() {
    let mut generator = Generator::new(Library::default());
    let mut definition = BlockDefinition::dfb("CustomEf", 10)
        .input("IN", "BOOL", 1)
        .output("OUT", "BOOL", 1);
    definition.kind = fbd_gen::BlockKind::Ef;
    generator.register_block(definition).unwrap();
    let spec: SectionSpec = serde_json::from_str(r#"{"name":"EF","blocks":[{"id":"n","type":"CustomEf"}],"links":[["TRUE","n.IN"],["n.OUT","Y"]]}"#).unwrap();
    let result = generator.generate_section(&spec).unwrap();
    assert!(result.layout.blocks[0].instance.is_none());
    assert!(
        result
            .xml
            .contains(r#"instanceName=".1" typeName="CustomEf""#)
    );
}

#[test]
fn cli_accepts_custom_dfb_catalog_and_preserves_the_manifest_format() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let out = root.join("out").join("api-cli-test");
    let status = std::process::Command::new(env!("CARGO_BIN_EXE_fbd-gen"))
        .arg(root.join("examples/custom_dfb.json"))
        .arg("--lib")
        .arg(root.join("efb_dimensions.json"))
        .arg("--extra-lib")
        .arg(root.join("examples/custom_blocks.json"))
        .arg("--out-dir")
        .arg(&out)
        .current_dir(std::env::temp_dir())
        .output()
        .unwrap();
    assert!(
        status.status.success(),
        "{}",
        String::from_utf8_lossy(&status.stderr)
    );
    let manifest: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(out.join("Motor_DFB/manifest.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(manifest["program"], "Motor_DFB");
    assert_eq!(manifest["task"], "MAST");
    assert_eq!(manifest["sections"][0]["xml"], "Motor_DFB.xbd");
    assert!(
        std::fs::read_to_string(out.join("Motor_DFB/Motor_DFB.xbd"))
            .unwrap()
            .contains("P201_Motor")
    );
}

#[test]
fn programs_write_the_same_files_as_the_cli() {
    let json = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/planta_bombas.json"),
    )
    .unwrap();
    let spec: ProgramSpec = serde_json::from_str(&json).unwrap();
    let program = Generator::embedded()
        .unwrap()
        .generate_program(&spec)
        .unwrap();
    let dir = std::env::temp_dir().join(format!("fbd-gen-api-write-{}", std::process::id()));
    let manifest_path = program.write_to(&dir).unwrap();
    assert_eq!(manifest_path, dir.join("manifest.json"));

    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&manifest_path).unwrap()).unwrap();
    assert_eq!(manifest["program"], "Planta_Bombas");
    let sections = manifest["sections"].as_array().unwrap();
    assert_eq!(sections.len(), program.sections.len());
    for (entry, section) in sections.iter().zip(&program.sections) {
        assert_eq!(entry["name"], section.name.as_str());
        for (key, ext) in [("xml", "xbd"), ("svg", "svg"), ("layout", "layout.json")] {
            assert_eq!(entry[key], section.file_name(ext).as_str());
            assert!(dir.join(section.file_name(ext)).is_file());
        }
        assert_eq!(
            std::fs::read_to_string(dir.join(section.file_name("xbd"))).unwrap(),
            section.xml
        );
    }
    std::fs::remove_dir_all(&dir).unwrap();
}
