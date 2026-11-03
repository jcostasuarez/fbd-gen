//! La CLI: entrada por archivo o stdin, `--check`, `--format json` y códigos de salida.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn run(args: &[&str], stdin: Option<&str>) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_fbd-gen"))
        .args(args)
        .current_dir(root())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("el binario fbd-gen se compila para los tests de CLI");
    let mut pipe = child.stdin.take().expect("stdin se pidió con Stdio::piped");
    pipe.write_all(stdin.unwrap_or("").as_bytes())
        .expect("no se pudo escribir en stdin de la CLI");
    drop(pipe);
    child
        .wait_with_output()
        .expect("no se pudo esperar a la CLI")
}

fn json(output: &Output) -> serde_json::Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|e| {
        panic!(
            "stdout no es JSON ({e}): {}",
            String::from_utf8_lossy(&output.stdout)
        )
    })
}

fn scratch(name: &str) -> PathBuf {
    let dir = root().join("out").join(name);
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

#[test]
fn json_output_reports_generated_sections_and_files() {
    let out = scratch("cli-json");
    let output = run(
        &[
            "examples/planta_bombas.json",
            "--format",
            "json",
            "--out-dir",
            out.to_str().unwrap(),
        ],
        None,
    );
    assert_eq!(output.status.code(), Some(0));
    let v = json(&output);
    assert_eq!(v["status"], "ok");
    assert_eq!(v["program"], "Planta_Bombas");
    let sections = v["sections"].as_array().unwrap();
    assert_eq!(sections.len(), 6);
    let manifest = PathBuf::from(v["manifest"].as_str().unwrap());
    assert!(manifest.is_file());
    for s in sections {
        assert!(
            manifest
                .parent()
                .unwrap()
                .join(s["xml"].as_str().unwrap())
                .is_file()
        );
    }
    // Los diagnósticos van completos, con código y ubicación.
    assert!(
        v["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["code"] == "typical_deviation" && d["location"]["group"] == "P103")
    );
}

#[test]
fn check_reads_stdin_and_writes_nothing() {
    let out = scratch("cli-check");
    let graph = std::fs::read_to_string(root().join("examples/p101_motor.json")).unwrap();
    let output = run(
        &[
            "-",
            "--check",
            "--format",
            "json",
            "--out-dir",
            out.to_str().unwrap(),
        ],
        Some(&graph),
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let v = json(&output);
    assert_eq!(v["status"], "ok");
    assert!(v["manifest"].is_null() && v["outDir"].is_null());
    assert!(!out.exists(), "--check no escribe archivos");

    // En texto, sin --format: los nombres de las secciones en stdout.
    let output = run(&["-", "--check"], Some(&graph));
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "P101_Motor");
}

#[test]
fn invalid_input_exits_with_1_and_located_diagnostics() {
    let output = run(
        &["-", "--format", "json"],
        Some(r#"{"name":"P","blocks":[{"id":"n","type":"NOT"}],"links":[["TRUE","n.FOO"]]}"#),
    );
    assert_eq!(output.status.code(), Some(1));
    let v = json(&output);
    assert_eq!(v["status"], "invalid");
    let d = &v["diagnostics"][0];
    assert_eq!(d["code"], "unknown_pin");
    assert_eq!(
        d["location"],
        serde_json::json!({"block": "n", "pin": "FOO", "link": 0})
    );

    // Un JSON mal formado también es una entrada inválida, con su código.
    let output = run(
        &["-", "--format", "json"],
        Some(r#"{"name":"P","blocks":[],"partiton":{}}"#),
    );
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(json(&output)["diagnostics"][0]["code"], "invalid_json");

    // En texto, los diagnósticos van a stderr con etapa y código.
    let output = run(&["-"], Some(r#"{"name":"P","blocks":[]}"#));
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("[error] P / validate/empty_graph:"),
        "{stderr}"
    );
}

#[test]
fn operational_errors_exit_with_3_and_usage_errors_with_2() {
    let output = run(&["no_existe.json", "--format", "json"], None);
    assert_eq!(output.status.code(), Some(3));
    let v = json(&output);
    assert_eq!(v["status"], "error");
    let message = v["message"].as_str().unwrap();
    assert!(message.contains("no_existe.json"), "{message}");
    assert_eq!(message.matches("no se pudo leer").count(), 1, "{message}");

    let output = run(
        &[
            "examples/p101_motor.json",
            "--check",
            "--lib",
            "no_existe.json",
        ],
        None,
    );
    assert_eq!(output.status.code(), Some(3));

    let output = run(&["--format", "yaml", "x.json"], None);
    assert_eq!(output.status.code(), Some(2));
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
