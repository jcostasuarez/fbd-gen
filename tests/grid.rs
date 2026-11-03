//! Control Expert guarda todo en celdas enteras: lo que sale del layout tiene que ser
//! entero, ortogonal y sin cables de salidas distintas superpuestos.

use std::path::Path;

use fbd_gen::{GeneratedProgram, Generator, ProgramSpec};

fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn generator() -> Generator {
    let mut generator = Generator::from_library_file(root().join("efb_dimensions.json"))
        .expect("efb_dimensions.json es la librería del repositorio");
    generator
        .register_blocks_from_file(root().join("examples/custom_blocks.json"))
        .expect("custom_blocks.json es un catálogo válido");
    generator
}

fn example(name: &str) -> ProgramSpec {
    let json = std::fs::read_to_string(root().join("examples").join(name))
        .unwrap_or_else(|e| panic!("no se pudo leer examples/{name}: {e}"));
    serde_json::from_str(&json).unwrap_or_else(|e| panic!("examples/{name} no es válido: {e}"))
}

fn assert_integer_grid(program: &GeneratedProgram) {
    for s in &program.sections {
        for link in &s.layout.links {
            let pts = &link.points;
            assert!(pts.len() >= 2, "{}: {pts:?}", link.from);
            for w in pts.windows(2) {
                assert!(
                    w[0][0] == w[1][0] || w[0][1] == w[1][1],
                    "{} -> {} no es ortogonal: {pts:?}",
                    link.from,
                    link.to
                );
            }
            assert!(pts.iter().flatten().all(|&c| c >= 0), "{pts:?}");
        }
        // Lo que lee Control Expert: cada posX/posY/width/height es un entero.
        for key in ["posX=\"", "posY=\"", "width=\"", "height=\""] {
            for (at, _) in s.xml.match_indices(key) {
                let value = s.xml[at + key.len()..]
                    .split('"')
                    .next()
                    .unwrap_or_default();
                assert!(
                    value.parse::<i32>().is_ok_and(|v| v >= 0),
                    "{}: {key}{value}",
                    s.name
                );
            }
        }
    }
}

#[test]
fn examples_land_on_integer_cells() {
    let generator = generator();
    for name in [
        "p101_motor.json",
        "planta_bombas.json",
        "custom_dfb.json",
        "probes/fanout_order.json",
        "probes/io_probe.json",
        "probes/loop_probe.json",
    ] {
        let program = generator.generate_program(&example(name)).unwrap();
        assert_integer_grid(&program);
    }
}

#[test]
fn generation_is_deterministic() {
    let generator = generator();
    let spec = example("planta_bombas.json");
    let first = generator.generate_program(&spec).unwrap();
    let second = generator.generate_program(&spec).unwrap();
    for (a, b) in first.sections.iter().zip(&second.sections) {
        assert_eq!(a.xml, b.xml, "{}", a.name);
    }
}
