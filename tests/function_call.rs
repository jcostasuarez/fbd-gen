use std::path::Path;

use fbd_gen::{FunctionCallSpec, GenerationError, Generator, VarSpec};

fn generator() -> Generator {
    Generator::from_library_file(Path::new(env!("CARGO_MANIFEST_DIR")).join("efb_dimensions.json"))
        .unwrap()
}

#[test]
fn function_call_binds_timer_inputs_and_outputs_and_declares_its_instance() {
    let call: FunctionCallSpec =
        serde_json::from_str(include_str!("../examples/function_call.json")).unwrap();
    let result = generator().generate_function(&call).unwrap();
    assert_eq!(result.name, "P101_Delay");
    assert_eq!(result.layout.blocks.len(), 1);
    assert_eq!(result.layout.blocks[0].ty, "TON");
    assert_eq!(
        result.layout.blocks[0].instance.as_deref(),
        Some("P101_Timer")
    );
    for (name, ty) in [
        ("P101_Timer", "TON"),
        ("P101_Start", "BOOL"),
        ("P101_Run", "BOOL"),
    ] {
        assert!(
            result
                .variables
                .iter()
                .any(|v| v.name == name && v.ty == ty)
        );
    }
    assert!(
        result
            .xml
            .contains(r#"formalParameter="IN" effectiveParameter="P101_Start""#)
    );
    assert!(
        result
            .xml
            .contains(r#"formalParameter="PT" effectiveParameter="T#3s""#)
    );
    assert!(
        result
            .xml
            .contains(r#"formalParameter="Q" effectiveParameter="P101_Run""#)
    );
    assert!(
        result
            .xml
            .contains(r#"<identProgram name="P101_Delay" type="section" task="MAST""#)
    );
}

#[test]
fn extensible_ef_calls_have_no_instance_and_infer_their_pin_count() {
    let call = FunctionCallSpec::new("Permissive", "AND")
        .input("IN1", "TRUE")
        .input("IN2", "Ready")
        .input("IN3", "TRUE")
        .output("OUT", "Permitted");
    let result = generator().generate_function(&call).unwrap();
    assert_eq!(result.layout.blocks.len(), 1);
    assert!(result.layout.blocks[0].instance.is_none());
    assert!(result.variables.iter().all(|v| v.ty == "BOOL"));
    assert!(result.xml.contains(r#"additionnalPinNumber="1""#));
    assert!(result.xml.contains(r#"instanceName=".1" typeName="AND""#));
}

#[test]
fn registered_dfb_calls_bind_in_out_variables_on_the_same_pin() {
    let mut generator = generator();
    generator
        .register_blocks_from_file(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/custom_blocks.json"),
        )
        .unwrap();
    let call = FunctionCallSpec::new("CustomCall", "MotorPropio")
        .with_instance("MyMotor")
        .input("Enable", "EnableSignal")
        .input("Reset", "FALSE")
        .output("Running", "IsRunning")
        .in_out("State", "MotorState");
    let result = generator.generate_function(&call).unwrap();
    assert!(
        result
            .variables
            .iter()
            .any(|v| v.name == "MyMotor" && v.ty == "MotorPropio")
    );
    assert!(
        result
            .variables
            .iter()
            .any(|v| v.name == "MotorState" && v.ty == "INT")
    );
    assert!(result.xml.contains(r#"<inputVariable invertedPin="false" formalParameter="State" effectiveParameter="MotorState""#));
    assert!(
        result
            .xml
            .contains(r#"<outputVariable invertedPin="false" formalParameter="State""#)
    );
}

#[test]
fn function_call_specs_round_trip_and_support_explicit_variable_declarations() {
    let mut call = FunctionCallSpec::new("Copy", "MOVE")
        .input("IN", "X")
        .output("OUT", "Y");
    call.variables.push(VarSpec::new("X", "REAL"));
    let call: FunctionCallSpec =
        serde_json::from_str(&serde_json::to_string(&call).unwrap()).unwrap();
    let result = generator().generate_function(&call).unwrap();
    assert!(
        result
            .variables
            .iter()
            .any(|v| v.name == "Y" && v.ty == "REAL")
    );

    let call = FunctionCallSpec::new("Delay", "TON")
        .input("IN", "TRUE")
        .input("PT", "T#1s");
    assert_eq!(
        generator().generate_function(&call).unwrap().layout.blocks[0]
            .instance
            .as_deref(),
        Some("Delay_call")
    );
}

#[test]
fn invalid_call_bindings_are_rejected() {
    let generator = generator();
    let mut too_many = FunctionCallSpec::new("Bad", "AND").input("IN1", "TRUE");
    too_many.input_count = Some(u32::MAX);
    for call in [
        FunctionCallSpec::new("Bad", "NotRegistered"),
        FunctionCallSpec::new("Bad", "TON").input("Q", "TRUE"),
        FunctionCallSpec::new("Bad", "TON").output("IN", "Y"),
        FunctionCallSpec::new("Bad", "TON").in_out("IN", "Y"),
        FunctionCallSpec::new("Bad", "TON").input("IN", ""),
        FunctionCallSpec::new("Bad", "TON").output("Q", "TRUE"),
        FunctionCallSpec::new("Bad", "INC_INT").in_out("INOUT", "3"),
        FunctionCallSpec::new("Bad", "AND").input("IN4294967295", "TRUE"),
        FunctionCallSpec::new("Bad", "NOT").with_instance("InvalidEfInstance"),
        FunctionCallSpec::new("Bad", "TON").with_instance(""),
        FunctionCallSpec::new("Bad", "TON").input("IN", "Bad_call.Q"),
        FunctionCallSpec::new("Bad", "TON")
            .input("IN", "TRUE")
            .input("in", "FALSE"),
        too_many,
    ] {
        assert!(
            matches!(
                generator.generate_function(&call),
                Err(GenerationError::InvalidInput { .. })
            ),
            "{}: {:?}",
            call.function,
            call.inputs
        );
    }
}

#[test]
fn function_calls_use_the_same_type_validation_as_sections() {
    let call = FunctionCallSpec::new("BadType", "NOT")
        .input("IN", "REAL#1.5")
        .output("OUT", "Y");
    let error = generator().generate_function(&call).unwrap_err();
    assert!(
        error
            .diagnostics()
            .iter()
            .any(|d| d.message.contains("no pertenece a ANY_BIT"))
    );
}
