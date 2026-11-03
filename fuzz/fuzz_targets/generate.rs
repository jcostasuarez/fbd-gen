//! Bytes arbitrarios -> JSON -> `generate_program` / `generate_function`.
//!
//! Lo que tiene que valer para cualquier entrada que se deserialice:
//! - nada entra en pánico;
//! - `Layout` e `Internal` no aparecen (son bugs, no entradas inválidas);
//! - un rechazo trae al menos un error, y un resultado aceptado ninguno.

#![no_main]

use std::sync::LazyLock;

use fbd_gen::{FunctionCallSpec, GenerationError, Generator, Level, ProgramSpec};
use libfuzzer_sys::fuzz_target;

static GENERATOR: LazyLock<Generator> =
    LazyLock::new(|| Generator::embedded().expect("librería incluida"));

fn check<T>(result: Result<T, GenerationError>, diagnostics: impl Fn(&T) -> Vec<Level>) {
    match result {
        Ok(value) => assert!(diagnostics(&value).iter().all(|l| *l != Level::Error)),
        Err(GenerationError::InvalidInput { diagnostics, .. }) => {
            assert!(diagnostics.iter().any(|d| d.level == Level::Error));
        }
        Err(error @ (GenerationError::Layout { .. } | GenerationError::Internal { .. })) => {
            panic!("bug: {error}")
        }
        Err(_) => {}
    }
}

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    if let Ok(spec) = serde_json::from_str::<ProgramSpec>(text) {
        check(GENERATOR.generate_program(&spec), |p| {
            p.diagnostics
                .iter()
                .chain(p.sections.iter().flat_map(|s| &s.diagnostics))
                .map(|d| d.level)
                .collect()
        });
    }
    if let Ok(call) = serde_json::from_str::<FunctionCallSpec>(text) {
        check(GENERATOR.generate_function(&call), |s| {
            s.diagnostics.iter().map(|d| d.level).collect()
        });
    }
});
