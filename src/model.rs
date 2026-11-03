//! Modelos del pipeline: el grafo semántico de entrada (lo que genera el LLM) y la
//! sección resuelta contra la librería, que es lo que consumen las etapas siguientes.

use serde::{Deserialize, Serialize};

use crate::library::Kind;

// ---------------------------------------------------------------------------
// Entrada
// ---------------------------------------------------------------------------

/// Grafo semántico de un programa o de una sección. No lleva coordenadas: eso es trabajo
/// del layout. Si los bloques llevan `group` (equipo) o el grafo es grande, la etapa de
/// partición lo divide en varias secciones (ver `partition`).
///
/// ```json
/// {
///   "name": "P101_Motor",
///   "blocks": [
///     { "id": "perm", "type": "AND", "inputs": 3 },
///     { "id": "tmr",  "type": "TON", "instance": "P101_TON_Arr" }
///   ],
///   "links": [
///     ["P101_Auto", "perm.IN1"],
///     ["perm.OUT",  "tmr.IN"],
///     ["T#5s",      "tmr.PT"],
///     ["tmr.Q",     "P101_Run"]
///   ]
/// }
/// ```
///
/// Cada extremo de un link es `bloque.PIN`, un literal IEC (`TRUE`, `5`, `T#5s`,
/// `16#FF`, `'texto'`) o, si no es ninguna de las dos cosas, una variable.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProgramSpec {
    /// Nombre del programa: identificador IEC.
    pub name: String,
    /// Tarea dueña de la sección (MAST por defecto).
    #[serde(default = "default_task")]
    pub task: String,
    /// Declaraciones explícitas. Las variables no declaradas se tipan por inferencia.
    #[serde(default)]
    pub variables: Vec<VarSpec>,
    /// Bloques del programa.
    pub blocks: Vec<BlockSpec>,
    /// Conexiones: cada extremo es `bloque.PIN`, un literal IEC o una variable.
    #[serde(default)]
    pub links: Vec<LinkSpec>,
    /// Criterios para dividir el programa en secciones.
    #[serde(default)]
    pub partition: PartitionSpec,
    /// Comentarios de red: cada uno se dibuja como encabezado (textBox) de la red que
    /// contiene al bloque `at`.
    #[serde(default)]
    pub comments: Vec<CommentSpec>,
}

impl ProgramSpec {
    /// Programa `name` vacío, en la tarea MAST y con la partición por defecto.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            task: default_task(),
            variables: Vec::new(),
            blocks: Vec::new(),
            links: Vec::new(),
            partition: PartitionSpec::default(),
            comments: Vec::new(),
        }
    }
}

/// Comentario de red.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CommentSpec {
    /// Id de un bloque de la red comentada.
    pub at: String,
    /// Texto del comentario.
    pub text: String,
}

/// Criterios de partición en secciones (etapa 4).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct PartitionSpec {
    /// Además del equipo (`group`), separar por etapa del flujo de señal (`stage`).
    pub by_stage: bool,
    /// Tope de bloques por sección; por encima se subdivide.
    pub max_blocks: usize,
}

impl Default for PartitionSpec {
    fn default() -> Self {
        Self {
            by_stage: false,
            max_blocks: 25,
        }
    }
}

pub(crate) fn default_task() -> String {
    "MAST".into()
}

/// Declaración de una variable.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct VarSpec {
    /// Nombre: identificador IEC.
    pub name: String,
    /// Tipo IEC (`BOOL`, `INT`, `TIME`...) o DDT.
    #[serde(rename = "type")]
    pub ty: String,
    /// Dirección topológica (`%MW100`, `%I0.1.2`...).
    pub address: Option<String>,
    /// Comentario de la declaración.
    pub comment: Option<String>,
    /// Valor inicial, como literal IEC.
    pub init: Option<String>,
}

impl VarSpec {
    /// Variable `name` de tipo `ty`, sin dirección, comentario ni valor inicial.
    pub fn new(name: impl Into<String>, ty: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ty: ty.into(),
            address: None,
            comment: None,
            init: None,
        }
    }
}

/// Instancia o llamada a un bloque del catálogo.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BlockSpec {
    /// Identificador del bloque dentro del grafo; los links lo usan como `id.PIN`.
    pub id: String,
    /// Tipo de bloque del catálogo (`TON`, `AND`, una DFB registrada...).
    #[serde(rename = "type")]
    pub ty: String,
    /// Nombre de instancia (EFB/DFB). Si falta se usa el `id`.
    pub instance: Option<String>,
    /// Cantidad de entradas extensibles. Si falta se deduce de los links.
    pub inputs: Option<u32>,
    /// Comentario del bloque (se dibuja con el bloque en Control Expert).
    pub comment: Option<String>,
    /// Equipo o unidad funcional (`P101`). Criterio principal de partición en secciones y
    /// unidad de comparación del control de consistencia.
    pub group: Option<String>,
    /// Etapa del flujo de señal (`entradas`, `permisivos`, `control`, `salidas`...).
    pub stage: Option<String>,
}

impl BlockSpec {
    /// Bloque `id` de tipo `ty`, sin opciones.
    pub fn new(id: impl Into<String>, ty: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            ty: ty.into(),
            instance: None,
            inputs: None,
            comment: None,
            group: None,
            stage: None,
        }
    }
}

/// Conexión entre dos extremos: `bloque.PIN`, literal IEC o variable.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(untagged, deny_unknown_fields)]
#[non_exhaustive]
pub enum LinkSpec {
    /// `[origen, destino]`, la forma compacta.
    Pair(String, String),
    /// `{ "from": origen, "to": destino }`.
    Object {
        /// Origen.
        from: String,
        /// Destino.
        to: String,
    },
}

impl LinkSpec {
    /// Extremos `(origen, destino)`.
    pub fn ends(&self) -> (&str, &str) {
        match self {
            LinkSpec::Pair(a, b) => (a, b),
            LinkSpec::Object { from, to } => (from, to),
        }
    }
}

// ---------------------------------------------------------------------------
// Sección resuelta
// ---------------------------------------------------------------------------

/// Dirección de un pin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[expect(
    clippy::exhaustive_enums,
    reason = "las tres direcciones de pin de la IEC 61131-3"
)]
pub enum Dir {
    /// Entrada.
    In,
    /// Salida.
    Out,
    /// Entrada y salida a la vez (IN_OUT).
    InOut,
}

impl Dir {
    /// Si el pin se dibuja del lado izquierdo (entradas e IN_OUT).
    pub fn has_left(self) -> bool {
        matches!(self, Dir::In | Dir::InOut)
    }
    /// Si el pin se dibuja del lado derecho (salidas e IN_OUT).
    pub fn has_right(self) -> bool {
        matches!(self, Dir::Out | Dir::InOut)
    }
}

/// Parámetro actual escrito junto a un pin (Control Expert no dibuja variables como
/// objetos en FBD: son texto pegado al pin).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(tag = "kind", content = "value", rename_all = "lowercase")]
#[non_exhaustive]
pub enum Param {
    /// Variable.
    Var(String),
    /// Literal IEC.
    Lit(String),
}

impl Param {
    /// Texto tal como se escribe junto al pin.
    pub fn text(&self) -> &str {
        match self {
            Param::Var(s) | Param::Lit(s) => s,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Pin {
    pub name: String,
    pub dir: Dir,
    pub ty: String,
    /// Fila relativa a `posY` del bloque.
    pub row: u32,
    /// Pertenece al grupo extensible de un bloque conmutativo (se puede permutar).
    pub permutable: bool,
    /// Parámetro del lado izquierdo (entradas, IN_OUT).
    pub param_in: Option<Param>,
    /// Parámetro del lado derecho (salidas, IN_OUT).
    pub param_out: Option<Param>,
    /// Tipo concreto deducido por la inferencia (`types`), si se pudo deducir. Viaja con
    /// el pin cuando la partición copia el bloque a su sección.
    pub inferred: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct Block {
    pub id: String,
    pub ty: String,
    pub kind: Kind,
    pub instance: Option<String>,
    pub comment: Option<String>,
    pub group: Option<String>,
    pub stage: Option<String>,
    /// Entradas extensibles por encima del mínimo de la librería (`additionnalPinNumber`).
    pub extra_pins: u32,
    pub width: u32,
    pub height: u32,
    pub pins: Vec<Pin>,
}

impl Block {
    pub(crate) fn pin_index(&self, name: &str) -> Option<usize> {
        self.pins
            .iter()
            .position(|p| p.name.eq_ignore_ascii_case(name))
    }

    /// Nombre que identifica al bloque en la sección (instancia o id).
    pub(crate) fn label(&self) -> &str {
        self.instance.as_deref().unwrap_or(&self.id)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct PinRef {
    pub block: usize,
    pub pin: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Wire {
    pub from: PinRef,
    pub to: PinRef,
}

/// Variable que el generador declaró para reemplazar un cable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GeneratedVar {
    /// Nombre de la variable.
    pub name: String,
    /// Tipo IEC inferido.
    #[serde(rename = "type")]
    pub ty: String,
    /// Por qué se generó: fan-out, realimentación o cruce entre secciones.
    pub reason: String,
}

#[derive(Debug, Clone)]
pub(crate) struct Section {
    pub name: String,
    pub blocks: Vec<Block>,
    pub wires: Vec<Wire>,
    /// Dependencias de ejecución conservadas al convertir fan-out a variables.
    /// No incluye los cortes intencionales de realimentación.
    pub execution_dependencies: Vec<(usize, usize)>,
    pub generated_vars: Vec<GeneratedVar>,
    /// Comentarios de red: (bloque ancla, texto).
    pub comments: Vec<(usize, String)>,
}

impl Section {
    pub(crate) fn pin(&self, r: PinRef) -> &Pin {
        &self.blocks[r.block].pins[r.pin]
    }

    pub(crate) fn pin_mut(&mut self, r: PinRef) -> &mut Pin {
        &mut self.blocks[r.block].pins[r.pin]
    }

    pub(crate) fn pin_path(&self, r: PinRef) -> String {
        let b = &self.blocks[r.block];
        format!("{}.{}", b.id, b.pins[r.pin].name)
    }
}

// ---------------------------------------------------------------------------
// Diagnósticos
// ---------------------------------------------------------------------------

/// Gravedad de un diagnóstico.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
#[expect(
    clippy::exhaustive_enums,
    reason = "gravedad estable: error, aviso, información"
)]
pub enum Level {
    /// La entrada es inválida: no se genera nada.
    Error,
    /// Se generó, pero conviene revisarlo.
    Warning,
    /// Decisión informativa del generador.
    Info,
}

impl Level {
    /// Nombre en minúsculas, igual que al serializar.
    pub fn as_str(self) -> &'static str {
        match self {
            Level::Error => "error",
            Level::Warning => "warning",
            Level::Info => "info",
        }
    }
}

impl std::fmt::Display for Level {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Etapa del pipeline que emitió un diagnóstico. Se serializa en minúsculas.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum Stage {
    /// Validación del grafo contra la librería.
    Validate,
    /// Inferencia y chequeo de tipos.
    Types,
    /// Consistencia entre equipos.
    Consistency,
    /// Partición en secciones y su orden.
    Partition,
    /// Normalización: fan-out y realimentaciones.
    Normalize,
    /// Layout y validación de la grilla.
    Layout,
}

impl Stage {
    /// Nombre en minúsculas, igual que al serializar.
    pub fn as_str(self) -> &'static str {
        match self {
            Stage::Validate => "validate",
            Stage::Types => "types",
            Stage::Consistency => "consistency",
            Stage::Partition => "partition",
            Stage::Normalize => "normalize",
            Stage::Layout => "layout",
        }
    }
}

impl std::fmt::Display for Stage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Mensaje del generador sobre la entrada o sobre una decisión tomada.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Diagnostic {
    /// Gravedad.
    pub level: Level,
    /// Etapa que lo emitió.
    pub stage: Stage,
    /// Texto para el usuario.
    pub message: String,
}

/// `[warning] normalize: ...`: gravedad, etapa y mensaje.
impl std::fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {}: {}", self.level, self.stage, self.message)
    }
}

#[derive(Debug, Default)]
pub(crate) struct Diagnostics(pub Vec<Diagnostic>);

impl Diagnostics {
    pub(crate) fn push(&mut self, level: Level, stage: Stage, message: impl Into<String>) {
        self.0.push(Diagnostic {
            level,
            stage,
            message: message.into(),
        });
    }

    pub(crate) fn has_errors(&self) -> bool {
        self.0.iter().any(|d| d.level == Level::Error)
    }
}
