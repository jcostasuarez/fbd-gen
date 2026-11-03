use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::Parser;

use fbd_gen::{
    Diagnostic, GenerationError, Generator, GeneratorConfig, Level, NormalizeConfig, ProgramSpec,
};

/// Genera las secciones FBD de Control Expert a partir del grafo semántico de un programa.
#[derive(Parser)]
struct Args {
    /// Grafo semántico del programa o de una sección (JSON).
    input: PathBuf,
    /// Librería de bloques exportada de Control Expert. Por defecto, la de Control
    /// Expert 15.1 incluida en el ejecutable.
    #[arg(long)]
    lib: Option<PathBuf>,
    /// Catálogo JSON de interfaces EF/EFB/DFB propias. Puede repetirse.
    #[arg(long = "extra-lib", value_name = "JSON")]
    extra_lib: Vec<PathBuf>,
    /// Carpeta base. Se escribe en <out-dir>/<programa>/: por sección .xbd (XML de
    /// intercambio FBD), .svg y .layout.json, más manifest.json con el orden de ejecución.
    #[arg(long, default_value = "out")]
    out_dir: PathBuf,
    /// Umbral de destinos por salida para usar una variable. Se conservan los cables
    /// si el corte impide respetar las dependencias de ejecución.
    #[arg(long, default_value_t = 3)]
    max_fanout: usize,
}

fn main() -> ExitCode {
    match run() {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn print(scope: &str, diags: &[Diagnostic]) {
    for d in diags {
        let tag = match d.level {
            Level::Error => "error",
            Level::Warning => "aviso",
            Level::Info => "info",
        };
        eprintln!("[{tag}] {scope}{}: {}", d.stage, d.message);
    }
}

fn run() -> Result<bool> {
    let args = Args::parse();
    let mut generator = match &args.lib {
        Some(path) => Generator::from_library_file(path)?,
        None => Generator::embedded()?,
    };
    for path in &args.extra_lib {
        generator.register_blocks_from_file(path)?;
    }
    let text = std::fs::read_to_string(&args.input)
        .with_context(|| format!("no se pudo leer {}", args.input.display()))?;
    let spec: ProgramSpec = serde_json::from_str(&text).context("grafo de programa inválido")?;

    let generator = generator.with_config(GeneratorConfig {
        normalization: NormalizeConfig {
            max_fanout: args.max_fanout,
        },
        ..GeneratorConfig::default()
    })?;
    let result = match generator.generate_program(&spec) {
        Ok(result) => result,
        Err(GenerationError::InvalidInput { scope, diagnostics }) => {
            print(&format!("{scope} / "), &diagnostics);
            return Ok(false);
        }
        Err(error) => return Err(error.into()),
    };

    print("", &result.diagnostics);
    for s in &result.sections {
        print(&format!("{} / ", s.name), &s.diagnostics);
    }

    let dir = args.out_dir.join(&spec.name);
    let manifest = result
        .write_to(&dir)
        .with_context(|| format!("no se pudo escribir en {}", dir.display()))?;
    println!("{}", manifest.display());
    for s in &result.sections {
        println!("  {}", dir.join(s.file_name("xbd")).display());
    }
    Ok(true)
}
