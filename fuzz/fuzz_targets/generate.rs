//! La entrada viene de un LLM: ningún JSON, por raro que sea, debe hacer entrar en pánico
//! al generador. Un error de validación es un resultado válido; un pánico, un bug.

#![no_main]

use std::sync::OnceLock;

use fbd_gen::serde_json;
use fbd_gen::{FunctionCallSpec, Generator, Library, ProgramSpec, SectionSpec};
use libfuzzer_sys::fuzz_target;

fn generator() -> &'static Generator {
    static GENERATOR: OnceLock<Generator> = OnceLock::new();
    GENERATOR.get_or_init(|| {
        let mut library = Library::embedded().expect("la librería incluida es válida");
        library
            .extend_from_json(include_str!("../../examples/custom_blocks.json"))
            .expect("custom_blocks.json es un catálogo válido");
        Generator::new(library)
    })
}

fuzz_target!(|data: &[u8]| {
    let generator = generator();
    if let Ok(spec) = serde_json::from_slice::<ProgramSpec>(data) {
        let _ = generator.generate_program(&spec);
    }
    if let Ok(spec) = serde_json::from_slice::<SectionSpec>(data) {
        let _ = generator.generate_section(&spec);
    }
    if let Ok(spec) = serde_json::from_slice::<FunctionCallSpec>(data) {
        let _ = generator.generate_function(&spec);
    }
});
