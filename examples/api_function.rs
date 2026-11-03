//! Genera una única llamada a un TON con `Generator::generate_function`.
//!
//! `cargo run --example api_function`: el XML va a stdout y el resumen a stderr.
#![expect(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "el ejemplo muestra su resultado en la terminal"
)]

use fbd_gen::{FunctionCallSpec, Generator};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let generator = Generator::from_library_file(root.join("efb_dimensions.json"))?;
    let call = FunctionCallSpec::new("P101_Delay", "TON")
        .with_instance("P101_Timer")
        .input("IN", "P101_Start")
        .input("PT", "T#3s")
        .output("Q", "P101_Run");
    let section = generator.generate_function(&call)?;
    eprintln!("{}: {} bloques", section.name, section.layout.blocks.len());
    println!("{}", section.xml);
    Ok(())
}
