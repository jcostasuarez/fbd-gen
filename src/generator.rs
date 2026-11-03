//! API principal de generación y registro de bloques externos.

use std::error::Error;
use std::fmt;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::library::{BlockDef, Library, LibraryError};
use crate::model::{
    self, BlockSpec, CommentSpec, Diagnostic, Diagnostics, Level, PartitionSpec, ProgramSpec,
    VarSpec,
};
use crate::{ConsistencyConfig, LayoutConfig, NormalizeConfig};
use crate::{consistency, layout, normalize, output, partition, resolve, types, xef};

/// Entrada de una sola sección: no acepta opciones de partición.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SectionSpec {
    pub name: String,
    #[serde(default = "model::default_task")]
    pub task: String,
    #[serde(default)]
    pub variables: Vec<VarSpec>,
    pub blocks: Vec<BlockSpec>,
    #[serde(default)]
    pub links: Vec<model::LinkSpec>,
    #[serde(default)]
    pub comments: Vec<CommentSpec>,
}

impl SectionSpec {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            task: model::default_task(),
            variables: Vec::new(),
            blocks: Vec::new(),
            links: Vec::new(),
            comments: Vec::new(),
        }
    }
}

impl From<SectionSpec> for ProgramSpec {
    fn from(s: SectionSpec) -> Self {
        Self {
            name: s.name,
            task: s.task,
            variables: s.variables,
            blocks: s.blocks,
            links: s.links,
            comments: s.comments,
            partition: PartitionSpec::default(),
        }
    }
}

/// Opciones reutilizables para todas las generaciones de una instancia.
#[derive(Debug, Clone, Default)]
pub struct GeneratorConfig {
    pub normalization: NormalizeConfig,
    pub layout: LayoutConfig,
    pub consistency: ConsistencyConfig,
}

/// Una generación exitosa siempre contiene todas sus secciones, ordenadas.
#[derive(Debug, Clone, Serialize)]
pub struct GeneratedProgram {
    pub name: String,
    pub task: String,
    pub diagnostics: Vec<Diagnostic>,
    pub sections: Vec<GeneratedSection>,
}

/// Resultado propio, sin referencias al generador ni necesidad de parsear JSON.
#[derive(Debug, Clone, Serialize)]
pub struct GeneratedSection {
    pub name: String,
    pub task: String,
    pub diagnostics: Vec<Diagnostic>,
    pub layout: output::OutSection,
    /// Todas las declaraciones emitidas, incluidas instancias EFB/DFB.
    pub variables: Vec<VarSpec>,
    pub xml: String,
    pub svg: String,
}

impl GeneratedSection {
    /// Formato `.layout.json`, compatible con la CLI y sin XML/SVG duplicados.
    pub fn layout_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(&self.layout)
    }
}

/// Una entrada inválida es `Err`, igual que un fallo de configuración o de ejecución.
#[derive(Debug)]
#[non_exhaustive]
pub enum GenerationError {
    InvalidInput {
        scope: String,
        diagnostics: Vec<Diagnostic>,
    },
    InvalidConfig {
        message: String,
    },
    Library(LibraryError),
    Layout {
        section: String,
        source: Box<dyn Error + Send + Sync>,
    },
}

impl GenerationError {
    pub fn diagnostics(&self) -> &[Diagnostic] {
        match self {
            Self::InvalidInput { diagnostics, .. } => diagnostics,
            _ => &[],
        }
    }
}

impl fmt::Display for GenerationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidInput { scope, diagnostics } => {
                write!(f, "entrada inválida en {scope}")?;
                if let Some(d) = diagnostics.iter().find(|d| d.level == Level::Error) {
                    write!(f, ": {}", d.message)?;
                }
                Ok(())
            }
            Self::InvalidConfig { message } => write!(f, "configuración inválida: {message}"),
            Self::Library(source) => write!(f, "{source}"),
            Self::Layout { section, source } => write!(f, "layout de {section}: {source}"),
        }
    }
}

impl Error for GenerationError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Library(source) => Some(source),
            Self::Layout { source, .. } => Some(source.as_ref()),
            _ => None,
        }
    }
}

impl From<LibraryError> for GenerationError {
    fn from(error: LibraryError) -> Self {
        Self::Library(error)
    }
}

/// Generador reutilizable: mantiene el catálogo y las opciones entre llamadas.
#[derive(Debug)]
pub struct Generator {
    library: Library,
    config: GeneratorConfig,
}

impl Generator {
    pub fn new(library: Library) -> Self {
        Self {
            library,
            config: GeneratorConfig::default(),
        }
    }

    pub fn from_library_file(path: impl AsRef<Path>) -> Result<Self, GenerationError> {
        Ok(Self::new(Library::load(path)?))
    }

    /// Generador con la librería de Control Expert 15.1 incluida en el binario.
    #[cfg(feature = "embedded-library")]
    pub fn embedded() -> Result<Self, GenerationError> {
        Ok(Self::new(Library::embedded()?))
    }

    pub fn with_config(mut self, config: GeneratorConfig) -> Result<Self, GenerationError> {
        validate_config(&config)?;
        self.config = config;
        Ok(self)
    }

    pub fn library(&self) -> &Library {
        &self.library
    }
    pub fn config(&self) -> &GeneratorConfig {
        &self.config
    }

    /// Registra la interfaz de una EF/EFB/DFB externa sin sustituir tipos existentes.
    pub fn register_block(&mut self, definition: BlockDef) -> Result<(), LibraryError> {
        self.library.register(definition)
    }

    pub fn register_blocks_from_file(
        &mut self,
        path: impl AsRef<Path>,
    ) -> Result<(), LibraryError> {
        self.library.extend_from_file(path)
    }

    pub fn generate_program(
        &self,
        spec: &ProgramSpec,
    ) -> Result<GeneratedProgram, GenerationError> {
        generate_program_with(spec, &self.library, &self.config)
    }

    /// Genera una sola sección. `group` y `stage` de los bloques son solo metadatos.
    pub fn generate_section(
        &self,
        spec: &SectionSpec,
    ) -> Result<GeneratedSection, GenerationError> {
        generate_section_with(&spec.clone().into(), &self.library, &self.config)
    }
}

fn validate_config(config: &GeneratorConfig) -> Result<(), GenerationError> {
    let bad = |message: &str| GenerationError::InvalidConfig {
        message: message.into(),
    };
    let c = &config.layout;
    // Topes holgados: las cuentas de grilla se hacen en i32 sin riesgo de desborde.
    if ![c.label_cells_per_10_chars, c.text_chars_per_10_cells]
        .iter()
        .all(|n| (1..=100).contains(n))
    {
        return Err(bad("las escalas de texto deben estar entre 1 y 100"));
    }
    if [c.node_gap, c.layer_gap, c.network_gap, c.frame_margin]
        .iter()
        .any(|&n| n > 100)
    {
        return Err(bad("las separaciones deben ser de 100 celdas o menos"));
    }
    if !config.consistency.similarity.is_finite()
        || !(0.0..=1.0).contains(&config.consistency.similarity)
    {
        return Err(bad("la similitud debe estar entre 0 y 1"));
    }
    Ok(())
}

fn invalid(spec: &ProgramSpec, diags: Diagnostics) -> GenerationError {
    GenerationError::InvalidInput {
        scope: spec.name.clone(),
        diagnostics: diags.0,
    }
}

pub(crate) fn generate_section_with(
    spec: &ProgramSpec,
    lib: &Library,
    cfg: &GeneratorConfig,
) -> Result<GeneratedSection, GenerationError> {
    validate_config(cfg)?;
    let mut diags = Diagnostics::default();
    let Some(mut section) = resolve::resolve(spec, lib, &mut diags) else {
        return Err(invalid(spec, diags));
    };
    normalize::normalize(&mut section, &cfg.normalization, &mut diags);
    let nets = normalize::networks(&section);
    let vars = types::infer(&mut section, &spec.variables, &mut diags);
    if diags.has_errors() {
        return Err(invalid(spec, diags));
    }
    let placed =
        layout::layout(&mut section, &nets, &cfg.layout, &mut diags).map_err(|source| {
            GenerationError::Layout {
                section: spec.name.clone(),
                source: source.into(),
            }
        })?;
    let data = output::to_output(&section, &nets, &placed, &diags);
    let xml = xef::to_exchange_xml(&section, &placed, &spec.task, &vars);
    let svg = output::to_svg(&section, &placed);
    let mut variables: Vec<VarSpec> = vars
        .into_iter()
        .map(|v| {
            let mut d = v.decl.unwrap_or_else(|| VarSpec::new(&v.name, &v.ty));
            d.name = v.name;
            d.ty = v.ty;
            d
        })
        .collect();
    variables.extend(
        section
            .blocks
            .iter()
            .filter_map(|b| b.instance.as_ref().map(|i| VarSpec::new(i, &b.ty))),
    );
    Ok(GeneratedSection {
        name: spec.name.clone(),
        task: spec.task.clone(),
        diagnostics: diags.0,
        layout: data,
        variables,
        xml,
        svg,
    })
}

pub(crate) fn generate_program_with(
    spec: &ProgramSpec,
    lib: &Library,
    cfg: &GeneratorConfig,
) -> Result<GeneratedProgram, GenerationError> {
    validate_config(cfg)?;
    let mut diags = Diagnostics::default();
    let Some(mut section) = resolve::resolve(spec, lib, &mut diags) else {
        return Err(invalid(spec, diags));
    };
    consistency::check(&section, &cfg.consistency, &mut diags);
    let (vars, pins) = types::infer_all(&mut section, &spec.variables, &mut diags);
    if diags.has_errors() {
        return Err(invalid(spec, diags));
    }
    let part = partition::partition(spec, &section, &vars, &pins, &mut diags);
    if diags.has_errors() {
        return Err(invalid(spec, diags));
    }
    let mut sections = Vec::new();
    for spec in part.sections {
        let mut result = generate_section_with(&spec, lib, cfg).map_err(|error| match error {
            GenerationError::InvalidInput {
                scope,
                mut diagnostics,
            } => {
                let mut all = diags.0.clone();
                all.append(&mut diagnostics);
                GenerationError::InvalidInput {
                    scope,
                    diagnostics: all,
                }
            }
            error => error,
        })?;
        result
            .diagnostics
            .retain(|d| d.stage != "validate" || d.level == Level::Error);
        result.layout.diagnostics.clone_from(&result.diagnostics);
        sections.push(result);
    }
    Ok(GeneratedProgram {
        name: spec.name.clone(),
        task: spec.task.clone(),
        diagnostics: diags.0,
        sections,
    })
}
