//! API principal de generación y registro de bloques externos.

use std::collections::HashMap;
use std::error::Error;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::layout::LayoutError as SectionLayoutError;
use crate::library::{BlockDef, Library, LibraryError};
use crate::model::{
    self, BlockSpec, CommentSpec, Diagnostic, Diagnostics, Level, PartitionSpec, ProgramSpec,
    Section, VarSpec,
};
use crate::types::TypedVar;
use crate::{ConsistencyConfig, LayoutConfig, NormalizeConfig};
use crate::{consistency, layout, normalize, output, partition, resolve, types, xef};

/// Entrada de una sola sección: no acepta opciones de partición.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SectionSpec {
    /// Nombre de la sección: identificador IEC.
    pub name: String,
    /// Tarea dueña de la sección (MAST por defecto).
    #[serde(default = "model::default_task")]
    pub task: String,
    /// Declaraciones explícitas. Las variables no declaradas se tipan por inferencia.
    #[serde(default)]
    pub variables: Vec<VarSpec>,
    /// Bloques de la sección.
    pub blocks: Vec<BlockSpec>,
    /// Conexiones: cada extremo es `bloque.PIN`, un literal IEC o una variable.
    #[serde(default)]
    pub links: Vec<model::LinkSpec>,
    /// Comentarios de red, dibujados como encabezado de la red de su bloque.
    #[serde(default)]
    pub comments: Vec<CommentSpec>,
}

impl SectionSpec {
    /// Sección `name` vacía, en la tarea MAST.
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
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GeneratorConfig {
    /// Fan-out y realimentaciones.
    pub normalization: NormalizeConfig,
    /// Medidas del dibujo en celdas.
    pub layout: LayoutConfig,
    /// Control de consistencia entre equipos.
    pub consistency: ConsistencyConfig,
}

/// Una generación exitosa siempre contiene todas sus secciones, ordenadas.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GeneratedProgram {
    /// Nombre del programa.
    pub name: String,
    /// Tarea en su forma canónica (`MAST`, `FAST`...).
    pub task: String,
    /// Diagnósticos de validación, tipos, consistencia y partición.
    pub diagnostics: Vec<Diagnostic>,
    /// Secciones en orden de ejecución.
    pub sections: Vec<GeneratedSection>,
}

/// Resultado propio, sin referencias al generador ni necesidad de parsear JSON.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GeneratedSection {
    /// Nombre de la sección.
    pub name: String,
    /// Tarea en su forma canónica (`MAST`, `FAST`...).
    pub task: String,
    /// Diagnósticos propios de la sección: normalización y layout.
    pub diagnostics: Vec<Diagnostic>,
    /// Bloques, pines y cables posicionados en la grilla.
    pub layout: output::OutSection,
    /// Todas las declaraciones emitidas, incluidas instancias EFB/DFB.
    pub variables: Vec<VarSpec>,
    /// XML de intercambio FBD (`.xbd`), listo para importar en Control Expert.
    pub xml: String,
    /// Vista previa SVG.
    pub svg: String,
}

impl GeneratedProgram {
    /// Contenido de `manifest.json`: las secciones en orden de ejecución, con sus archivos,
    /// y los diagnósticos del programa. Es el formato que leen `scripts/ce_check.py` y
    /// `scripts/roundtrip_diff.py`.
    ///
    /// # Errors
    ///
    /// Si falla la serialización a JSON.
    pub fn manifest_json(&self) -> Result<String, serde_json::Error> {
        // Campos en orden alfabético: es el orden histórico del archivo.
        #[derive(Serialize)]
        struct Manifest<'a> {
            diagnostics: &'a [Diagnostic],
            program: &'a str,
            sections: Vec<ManifestSection>,
            task: &'a str,
        }
        #[derive(Serialize)]
        struct ManifestSection {
            layout: String,
            name: String,
            svg: String,
            xml: String,
        }
        serde_json::to_string_pretty(&Manifest {
            diagnostics: &self.diagnostics,
            program: &self.name,
            sections: self
                .sections
                .iter()
                .map(|s| ManifestSection {
                    layout: s.file_name("layout.json"),
                    name: s.name.clone(),
                    svg: s.file_name("svg"),
                    xml: s.file_name("xbd"),
                })
                .collect(),
            task: &self.task,
        })
    }

    /// Escribe en `dir` lo mismo que la CLI: por sección `.xbd`, `.svg` y `.layout.json`,
    /// más `manifest.json`. Crea la carpeta si no existe y devuelve la ruta del manifest.
    ///
    /// # Errors
    ///
    /// Si no se puede crear `dir` o escribir alguno de los archivos.
    pub fn write_to(&self, dir: impl AsRef<Path>) -> io::Result<PathBuf> {
        let dir = dir.as_ref();
        std::fs::create_dir_all(dir)?;
        for s in &self.sections {
            std::fs::write(dir.join(s.file_name("xbd")), &s.xml)?;
            std::fs::write(dir.join(s.file_name("svg")), &s.svg)?;
            std::fs::write(dir.join(s.file_name("layout.json")), s.layout_json()?)?;
        }
        let path = dir.join("manifest.json");
        std::fs::write(&path, self.manifest_json()?)?;
        Ok(path)
    }
}

impl GeneratedSection {
    /// Archivo de la sección con la extensión dada (`xbd`, `svg`, `layout.json`).
    pub fn file_name(&self, extension: &str) -> String {
        format!("{}.{extension}", self.name)
    }

    /// Formato `.layout.json`: el layout más los diagnósticos de la sección, sin XML/SVG.
    ///
    /// # Errors
    ///
    /// Si falla la serialización a JSON.
    pub fn layout_json(&self) -> Result<String, serde_json::Error> {
        #[derive(Serialize)]
        struct LayoutFile<'a> {
            #[serde(flatten)]
            layout: &'a output::OutSection,
            diagnostics: &'a [Diagnostic],
        }
        serde_json::to_string_pretty(&LayoutFile {
            layout: &self.layout,
            diagnostics: &self.diagnostics,
        })
    }
}

/// Una entrada inválida es `Err`, igual que un fallo de configuración o de ejecución.
#[derive(Debug)]
#[non_exhaustive]
pub enum GenerationError {
    /// El grafo de entrada es inválido; los diagnósticos dicen por qué.
    InvalidInput {
        /// Programa o sección donde se detectó el problema.
        scope: String,
        /// Todos los diagnósticos acumulados, con al menos un error.
        diagnostics: Vec<Diagnostic>,
    },
    /// Opciones de [`GeneratorConfig`] fuera de rango.
    InvalidConfig {
        /// Qué opción falló y su rango válido.
        message: String,
    },
    /// No se pudo leer o registrar el catálogo de bloques.
    Library(LibraryError),
    /// El motor de layout no pudo dibujar la sección dentro de la grilla.
    Layout {
        /// Sección que no se pudo dibujar.
        section: String,
        /// Causa: un error del motor Sugiyama o una validación de la grilla.
        source: SectionLayoutError,
    },
}

impl GenerationError {
    /// Diagnósticos de una entrada inválida; vacío para los demás errores.
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
            Self::Layout { source, .. } => Some(source),
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
    /// Generador sobre `library`, con las opciones por defecto.
    pub fn new(library: Library) -> Self {
        Self {
            library,
            config: GeneratorConfig::default(),
        }
    }

    /// Generador con la librería exportada de Control Expert en `path`.
    ///
    /// # Errors
    ///
    /// [`GenerationError::Library`] si el archivo no se puede leer o tiene bloques inválidos.
    pub fn from_library_file(path: impl AsRef<Path>) -> Result<Self, GenerationError> {
        Ok(Self::new(Library::load(path)?))
    }

    /// Generador con la librería de Control Expert 15.1 incluida en el binario.
    ///
    /// # Errors
    ///
    /// [`GenerationError::Library`] solo si la librería incluida no pasa la validación:
    /// un `efb_dimensions.json` corrupto al compilar el crate.
    #[cfg(feature = "embedded-library")]
    pub fn embedded() -> Result<Self, GenerationError> {
        Ok(Self::new(Library::embedded()?))
    }

    /// Reemplaza las opciones del generador.
    ///
    /// # Errors
    ///
    /// [`GenerationError::InvalidConfig`] si alguna medida o umbral está fuera de rango.
    pub fn with_config(mut self, config: GeneratorConfig) -> Result<Self, GenerationError> {
        validate_config(&config)?;
        self.config = config;
        Ok(self)
    }

    /// Catálogo de bloques del generador.
    pub fn library(&self) -> &Library {
        &self.library
    }
    /// Opciones vigentes.
    pub fn config(&self) -> &GeneratorConfig {
        &self.config
    }

    /// Registra la interfaz de una EF/EFB/DFB externa sin sustituir tipos existentes.
    ///
    /// # Errors
    ///
    /// [`LibraryError::InvalidDefinition`] si la interfaz es inválida y
    /// [`LibraryError::DuplicateBlock`] si el nombre ya existe.
    pub fn register_block(&mut self, definition: BlockDef) -> Result<(), LibraryError> {
        self.library.register(definition)
    }

    /// Registra las interfaces de un catálogo JSON adicional (`{ "blocks": [...] }`).
    ///
    /// # Errors
    ///
    /// Si el archivo no se puede leer o parsear, o alguna definición no se puede
    /// registrar; en ese caso no se registra ninguna.
    pub fn register_blocks_from_file(
        &mut self,
        path: impl AsRef<Path>,
    ) -> Result<(), LibraryError> {
        self.library.extend_from_file(path)
    }

    /// Genera un programa completo: valida, controla la consistencia entre equipos,
    /// particiona en secciones y dibuja cada una.
    ///
    /// # Errors
    ///
    /// [`GenerationError::InvalidInput`] si el grafo es inválido y
    /// [`GenerationError::Layout`] si alguna sección no se puede dibujar. No hay
    /// resultados parciales.
    pub fn generate_program(
        &self,
        spec: &ProgramSpec,
    ) -> Result<GeneratedProgram, GenerationError> {
        generate_program_with(spec, &self.library, &self.config)
    }

    /// Genera una sola sección. `group` y `stage` de los bloques son solo metadatos.
    ///
    /// ```
    /// use fbd_gen::{BlockSpec, Generator, LinkSpec, SectionSpec, VarSpec};
    ///
    /// let generator = Generator::from_library_file("efb_dimensions.json")?;
    /// let mut spec = SectionSpec::new("Arranque");
    /// // `AND` es genérico (ANY_BIT): las variables declaradas fijan el tipo.
    /// for name in ["P101_Auto", "P101_Listo", "P101_Run"] {
    ///     spec.variables.push(VarSpec::new(name, "BOOL"));
    /// }
    /// spec.blocks.push(BlockSpec::new("perm", "AND"));
    /// spec.links = vec![
    ///     LinkSpec::Pair("P101_Auto".into(), "perm.IN1".into()),
    ///     LinkSpec::Pair("P101_Listo".into(), "perm.IN2".into()),
    ///     LinkSpec::Pair("perm.OUT".into(), "P101_Run".into()),
    /// ];
    /// let section = generator.generate_section(&spec)?;
    /// assert_eq!(section.layout.blocks.len(), 1);
    /// assert!(section.xml.contains("AND"));
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    ///
    /// # Errors
    ///
    /// [`GenerationError::InvalidInput`] si el grafo es inválido y
    /// [`GenerationError::Layout`] si no se puede dibujar.
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

/// Tarea en su forma canónica (`mast` -> `MAST`). Una tarea desconocida ya es un error
/// de `resolve`, que corta antes de exportar.
fn task(spec: &ProgramSpec) -> &str {
    crate::iec::task_name(&spec.task).unwrap_or(&spec.task)
}

/// Variables conocidas por nombre (sin distinguir mayúsculas).
type VarTable = HashMap<String, TypedVar>;

fn var_table(vars: impl IntoIterator<Item = TypedVar>) -> VarTable {
    vars.into_iter()
        .map(|v| (v.name.to_ascii_uppercase(), v))
        .collect()
}

/// Etapas comunes de entrada: resolución contra la librería e inferencia de tipos. Los
/// errores de tipos quedan en `diags` para que el llamador corte cuando corresponda.
fn resolve_and_type(
    spec: &ProgramSpec,
    lib: &Library,
    diags: &mut Diagnostics,
) -> Option<(Section, Vec<TypedVar>)> {
    let mut section = resolve::resolve(spec, lib, diags)?;
    let vars = types::infer(&mut section, &spec.variables, diags);
    Some((section, vars))
}

/// Etapas 5 a 8 sobre una sección resuelta y tipada: normalización, redes, layout y
/// exportación. `diags` trae lo que ya se reportó para esta sección.
fn render_section(
    mut section: Section,
    task: &str,
    known: &VarTable,
    cfg: &GeneratorConfig,
    mut diags: Diagnostics,
) -> Result<GeneratedSection, GenerationError> {
    normalize::normalize(&mut section, &cfg.normalization, &mut diags);
    let nets = normalize::networks(&section);
    let vars = types::section_vars(&section, known, &mut diags);
    if diags.has_errors() {
        return Err(GenerationError::InvalidInput {
            scope: section.name.clone(),
            diagnostics: diags.0,
        });
    }
    let place = |section: &Section| {
        layout::layout(section, &nets, &cfg.layout).map_err(|source| GenerationError::Layout {
            section: section.name.clone(),
            source,
        })
    };
    // Primera pasada; si las entradas conmutativas quedan cruzadas se reasignan según
    // ese dibujo y se dibuja de nuevo.
    let mut placed = place(&section)?;
    let swaps = layout::commutative_swaps(&section, &placed);
    if !swaps.is_empty() {
        layout::apply_swaps(&mut section, &swaps, &mut diags);
        placed = place(&section)?;
    }
    let data = output::to_output(&section, &nets, &placed);
    let xml = xef::to_exchange_xml(&section, &placed, task, &vars);
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
        name: section.name,
        task: task.to_string(),
        diagnostics: diags.0,
        layout: data,
        variables,
        xml,
        svg,
    })
}

pub(crate) fn generate_section_with(
    spec: &ProgramSpec,
    lib: &Library,
    cfg: &GeneratorConfig,
) -> Result<GeneratedSection, GenerationError> {
    validate_config(cfg)?;
    let mut diags = Diagnostics::default();
    let Some((section, vars)) =
        resolve_and_type(spec, lib, &mut diags).filter(|_| !diags.has_errors())
    else {
        return Err(invalid(spec, diags));
    };
    render_section(section, task(spec), &var_table(vars), cfg, diags)
}

pub(crate) fn generate_program_with(
    spec: &ProgramSpec,
    lib: &Library,
    cfg: &GeneratorConfig,
) -> Result<GeneratedProgram, GenerationError> {
    validate_config(cfg)?;
    let mut diags = Diagnostics::default();
    let Some((section, vars)) = resolve_and_type(spec, lib, &mut diags) else {
        return Err(invalid(spec, diags));
    };
    // La consistencia se informa aunque haya errores de tipos: suele explicarlos.
    consistency::check(&section, &cfg.consistency, &mut diags);
    if diags.has_errors() {
        return Err(invalid(spec, diags));
    }
    let part = partition::partition(&section, &spec.partition, &vars, &mut diags);
    if diags.has_errors() {
        return Err(invalid(spec, diags));
    }
    let known = var_table(vars.into_iter().chain(part.link_vars));
    // El error de una sección se informa con el contexto del programa.
    let with_program_context = |error| match error {
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
    };
    let sections = part
        .sections
        .into_iter()
        .map(|section| {
            render_section(section, task(spec), &known, cfg, Diagnostics::default())
                .map_err(with_program_context)
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(GeneratedProgram {
        name: spec.name.clone(),
        task: task(spec).to_string(),
        diagnostics: diags.0,
        sections,
    })
}
