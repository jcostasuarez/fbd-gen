//! Registra la interfaz de una DFB propia desde Rust y genera una sección que la usa.
//!
//! `cargo run --example api_custom_dfb`: el XML va a stdout y el resumen a stderr.
#![expect(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "el ejemplo muestra su resultado en la terminal"
)]

use fbd_gen::{BlockDefinition, Generator, ProgramSpec};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut generator = Generator::from_library_file(root.join("efb_dimensions.json"))?;
    generator.register_block(
        BlockDefinition::dfb("MotorPropio", 12)
            .input("Enable", "BOOL", 1)
            .input("Reset", "BOOL", 2)
            .output("Running", "BOOL", 1)
            .in_out("State", "INT", 3),
    )?;
    let spec: ProgramSpec = serde_json::from_str(include_str!("custom_dfb.json"))?;
    let program = generator.generate_program(&spec)?;
    for section in program.sections {
        eprintln!(
            "{}: {} bloques, {} variables",
            section.name,
            section.layout.blocks.len(),
            section.variables.len()
        );
        println!("{}", section.xml);
    }
    Ok(())
}
