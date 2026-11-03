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

// Etapas internas. Son públicas para las pruebas, pero su orden e invariantes no son
// parte del contrato: usar `Generator`.
#[doc(hidden)]
pub mod consistency;
mod function_call;
mod generator;
#[doc(hidden)]
pub mod layout;
#[doc(hidden)]
pub mod library;
#[doc(hidden)]
pub mod model;
#[doc(hidden)]
pub mod normalize;
#[doc(hidden)]
pub mod output;
#[doc(hidden)]
pub mod partition;
#[doc(hidden)]
pub mod resolve;
pub mod sugiyama;
#[doc(hidden)]
pub mod types;
#[doc(hidden)]
pub mod xef;

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
    BlockSpec, CommentSpec, Diagnostic, Dir as PinDirection, GeneratedVar as GeneratedVariable,
    Level, LinkSpec, Param as Parameter, PartitionSpec, ProgramSpec, VarSpec,
};
pub use normalize::NormalizeConfig;
pub use output::{
    OutBlock as PlacedBlock, OutLink as RoutedLink, OutPin as PlacedPin,
    OutSection as SectionLayout, OutText as PlacedText,
};
