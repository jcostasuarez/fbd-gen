//! CLI de fbd-gen: lee un grafo semántico, escribe las secciones en `<out-dir>/<programa>/`
//! e informa los diagnósticos por stderr (o todo como JSON por stdout con `--format json`).
#![expect(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "la salida de la CLI es su interfaz: rutas por stdout, diagnósticos por stderr"
)]

use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::{Parser, ValueEnum};
use serde_json::json;

use fbd_gen::{
    Diagnostic, GeneratedProgram, GenerationError, Generator, GeneratorConfig, Level, ProgramSpec,
};

/// Genera las secciones FBD de Control Expert a partir del grafo semántico de un programa.
///
/// Código de salida: 0 si genera, 1 si la entrada es inválida (ver los diagnósticos),
/// 2 si los argumentos son incorrectos y 3 ante un error de lectura, escritura,
/// librería o un error interno.
#[derive(Parser)]
#[command(version)]
struct Args {
    /// Grafo semántico del programa o de una sección (JSON). `-` lee de la entrada
    /// estándar.
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
    /// Valida y genera en memoria, sin escribir archivos.
    #[arg(long)]
    check: bool,
    /// `text`: diagnósticos legibles en stderr y rutas en stdout. `json`: un solo objeto
    /// JSON en stdout con `status` (`ok`, `invalid` o `error`) y los diagnósticos
    /// completos (código y ubicación).
    #[arg(long, value_enum, default_value_t = Format::Text)]
    format: Format,
}

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
enum Format {
    Text,
    Json,
}

/// Resultado de una corrida, antes de mostrarlo en el formato pedido.
enum Outcome {
    Generated {
        program: GeneratedProgram,
        /// Carpeta y manifest escritos; `None` con `--check`.
        written: Option<(PathBuf, PathBuf)>,
    },
    Invalid {
        scope: String,
        diagnostics: Vec<Diagnostic>,
    },
}

const EXIT_INVALID: u8 = 1;
const EXIT_ERROR: u8 = 3;

fn main() -> ExitCode {
    let args = Args::parse();
    let format = args.format;
    match run(&args) {
        Ok(Outcome::Generated { program, written }) => {
            report_generated(format, &program, written.as_ref());
            ExitCode::SUCCESS
        }
        Ok(Outcome::Invalid { scope, diagnostics }) => {
            match format {
                Format::Text => print_diagnostics(&format!("{scope} / "), &diagnostics),
                Format::Json => println!(
                    "{}",
                    json!({"status": "invalid", "scope": scope, "diagnostics": diagnostics})
                ),
            }
            ExitCode::from(EXIT_INVALID)
        }
        Err(e) => {
            match format {
                Format::Text => eprintln!("error: {e:#}"),
                Format::Json => {
                    println!(
                        "{}",
                        json!({"status": "error", "message": format!("{e:#}")})
                    );
                }
            }
            ExitCode::from(EXIT_ERROR)
        }
    }
}

fn print_diagnostics(scope: &str, diags: &[Diagnostic]) {
    for d in diags {
        let tag = match d.level {
            Level::Error => "error",
            Level::Warning => "aviso",
            Level::Info => "info",
        };
        eprintln!("[{tag}] {scope}{}/{}: {}", d.stage, d.code, d.message);
    }
}

fn report_generated(
    format: Format,
    program: &GeneratedProgram,
    written: Option<&(PathBuf, PathBuf)>,
) {
    match format {
        Format::Text => {
            print_diagnostics("", &program.diagnostics);
            for s in &program.sections {
                print_diagnostics(&format!("{} / ", s.name), &s.diagnostics);
            }
            match written {
                Some((dir, manifest)) => {
                    println!("{}", manifest.display());
                    for s in &program.sections {
                        println!("  {}", dir.join(s.file_name("xbd")).display());
                    }
                }
                None => {
                    for s in &program.sections {
                        println!("{}", s.name);
                    }
                }
            }
        }
        Format::Json => {
            let sections: Vec<_> = program
                .sections
                .iter()
                .map(|s| {
                    json!({
                        "name": s.name,
                        "xml": s.file_name("xbd"),
                        "svg": s.file_name("svg"),
                        "layout": s.file_name("layout.json"),
                        "diagnostics": s.diagnostics,
                    })
                })
                .collect();
            let path = |p: &Path| p.display().to_string();
            println!(
                "{}",
                json!({
                    "status": "ok",
                    "program": program.name,
                    "task": program.task,
                    "outDir": written.map(|(dir, _)| path(dir)),
                    "manifest": written.map(|(_, manifest)| path(manifest)),
                    "diagnostics": program.diagnostics,
                    "sections": sections,
                })
            );
        }
    }
}

fn read_input(path: &Path) -> Result<String> {
    if path == Path::new("-") {
        let mut text = String::new();
        std::io::stdin()
            .read_to_string(&mut text)
            .context("no se pudo leer la entrada estándar")?;
        Ok(text)
    } else {
        std::fs::read_to_string(path).with_context(|| format!("no se pudo leer {}", path.display()))
    }
}

fn run(args: &Args) -> Result<Outcome> {
    let mut generator = match &args.lib {
        Some(path) => Generator::from_library_file(path)?,
        None => Generator::embedded()?,
    };
    for path in &args.extra_lib {
        generator.register_blocks_from_file(path)?;
    }
    let mut config = GeneratorConfig::default();
    config.normalization.max_fanout = args.max_fanout;
    let generator = generator.with_config(config)?;

    let text = read_input(&args.input)?;
    let result = ProgramSpec::from_json(&text).and_then(|spec| generator.generate_program(&spec));
    let program = match result {
        Ok(program) => program,
        Err(GenerationError::InvalidInput { scope, diagnostics }) => {
            return Ok(Outcome::Invalid { scope, diagnostics });
        }
        Err(error) => return Err(error.into()),
    };
    if args.check {
        return Ok(Outcome::Generated {
            program,
            written: None,
        });
    }
    let dir = args.out_dir.join(&program.name);
    let manifest = program
        .write_to(&dir)
        .with_context(|| format!("no se pudo escribir en {}", dir.display()))?;
    Ok(Outcome::Generated {
        program,
        written: Some((dir, manifest)),
    })
}
