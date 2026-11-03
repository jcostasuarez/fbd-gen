//! Generación de secciones FBD para EcoStruxure Control Expert.
//!
//! La API principal es [`Generator`]: recibe [`ProgramSpec`] o [`SectionSpec`] y
//! devuelve datos propios más XML/SVG, o un [`GenerationError`] con diagnósticos.
//! Es Rust puro: no necesita Node ni otros procesos, y con el feature
//! `embedded-library` (activo por defecto) tampoco el catálogo de bloques en disco.
//!
//! [`sugiyama`] es el motor de layout, utilizable por separado: grafos dirigidos sin
//! ciclos con puertos en filas fijas, todo en coordenadas enteras.
//!
//! ```no_run
//! use fbd_gen::{Generator, ProgramSpec};
//!
//! let generator = Generator::embedded()?;
//! let json = std::fs::read_to_string("examples/planta_bombas.json")?;
//! let spec: ProgramSpec = serde_json::from_str(&json)?;
//! let program = generator.generate_program(&spec)?;
//! for section in program.sections {
//!     println!("{}: {} bloques", section.name, section.layout.blocks.len());
//! }
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

// Etapas internas: su orden e invariantes no son parte del contrato. Lo público son
// `Generator`, los tipos que se re-exportan abajo y el motor `sugiyama`.
mod consistency;
mod function_call;
mod generator;
mod graph;
mod iec;
mod layout;
mod library;
mod model;
mod normalize;
mod output;
mod partition;
mod resolve;
pub mod sugiyama;
mod types;
mod xef;
mod xml;

#[cfg(test)]
mod tests;

// Los ejemplos de `docs/API.md` se compilan como doctests: la guía no puede quedar
// desactualizada respecto de la API.
#[cfg(doctest)]
#[doc = include_str!("../docs/API.md")]
struct ApiGuide;

pub use consistency::ConsistencyConfig;
pub use function_call::FunctionCallSpec;
pub use generator::{
    GeneratedProgram, GeneratedSection, GenerationError, Generator, GeneratorConfig, SectionSpec,
};
pub use layout::LayoutConfig;
pub use library::{
    BlockDef as BlockDefinition, Extensible as ExtensiblePin, Kind as BlockKind, Library,
    LibraryError, PinDef as PinDefinition,
};
pub use model::{
    BlockSpec, CommentSpec, Diagnostic, DiagnosticCode, DiagnosticLocation, Dir as PinDirection,
    GeneratedVar as GeneratedVariable, Level, LinkSpec, Param as Parameter, PartitionSpec,
    ProgramSpec, Stage, VarSpec,
};
pub use normalize::NormalizeConfig;
pub use output::{
    OutBlock as PlacedBlock, OutLink as RoutedLink, OutPin as PlacedPin,
    OutSection as SectionLayout, OutText as PlacedText,
};
