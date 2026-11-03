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
#[non_exhaustive]
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
#[non_exhaustive]
pub struct CommentSpec {
    /// Id de un bloque de la red comentada.
    pub at: String,
    /// Texto del comentario.
    pub text: String,
}

impl CommentSpec {
    /// Comentario de la red que contiene al bloque `at`.
    pub fn new(at: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            at: at.into(),
            text: text.into(),
        }
    }
}

/// Criterios de partición en secciones (etapa 4).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
#[non_exhaustive]
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
#[non_exhaustive]
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
#[non_exhaustive]
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
#[non_exhaustive]
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

/// Define [`DiagnosticCode`] con su nombre estable en un solo lugar: el mismo texto sale
/// en `as_str`, en `Display` y al serializar.
macro_rules! diagnostic_codes {
    ($($(#[doc = $doc:literal])* $variant:ident = $name:literal,)*) => {
        /// Qué detectó un diagnóstico, independiente del texto del mensaje.
        ///
        /// El nombre serializado (`unknown_pin`, `type_mismatch`...) es estable: se puede
        /// usar para filtrar, contar o decidir una corrección automática sin interpretar
        /// el mensaje, que es para personas y puede cambiar de redacción. Pueden aparecer
        /// códigos nuevos en versiones futuras.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        #[non_exhaustive]
        pub enum DiagnosticCode {
            $($(#[doc = $doc])* $variant,)*
        }

        impl DiagnosticCode {
            /// Todos los códigos, en orden de declaración.
            pub const ALL: &'static [DiagnosticCode] = &[$(DiagnosticCode::$variant,)*];

            /// Nombre estable en `snake_case`, el mismo que se serializa.
            pub fn as_str(self) -> &'static str {
                match self {
                    $(DiagnosticCode::$variant => $name,)*
                }
            }
        }
    };
}

diagnostic_codes! {
    // --- Validación (Stage::Validate) ---
    /// El texto no es JSON válido, le falta un campo obligatorio o tiene uno desconocido
    /// (`instace`, `partiton`...). El mensaje trae la línea y la columna.
    InvalidJson = "invalid_json",
    /// El nombre del programa o de la sección no es un identificador IEC.
    InvalidProgramName = "invalid_program_name",
    /// El grafo no tiene bloques.
    EmptyGraph = "empty_graph",
    /// La tarea no es MAST, FAST, AUX0 a AUX3 ni SAFE.
    InvalidTask = "invalid_task",
    /// Una variable declarada tiene un nombre inválido.
    InvalidVariableName = "invalid_variable_name",
    /// `address` no es una dirección topológica (`%MW100`).
    InvalidAddress = "invalid_address",
    /// Un texto que va al XML tiene un caracter de control que XML 1.0 no admite.
    InvalidXmlChar = "invalid_xml_char",
    /// El id de un bloque no es un identificador IEC.
    InvalidBlockId = "invalid_block_id",
    /// El nombre de instancia no es un identificador IEC, o está vacío.
    InvalidInstanceName = "invalid_instance_name",
    /// Dos bloques con el mismo id (sin distinguir mayúsculas).
    DuplicateBlockId = "duplicate_block_id",
    /// El tipo de bloque no está en el catálogo.
    UnknownBlockType = "unknown_block_type",
    /// Más entradas extensibles que el máximo del bloque, o fuera de su rango.
    TooManyInputs = "too_many_inputs",
    /// Se pidió una cantidad de entradas extensibles a un bloque que no las tiene.
    NotExtensible = "not_extensible",
    /// Se dio una instancia a una EF, que no tiene.
    InstanceNotAllowed = "instance_not_allowed",
    /// Dos bloques con el mismo nombre de instancia.
    DuplicateInstance = "duplicate_instance",
    /// Una variable se llama como una instancia o como un id de bloque.
    NameCollision = "name_collision",
    /// Un comentario apunta a un bloque inexistente.
    UnknownCommentTarget = "unknown_comment_target",
    /// El bloque no tiene ese pin.
    UnknownPin = "unknown_pin",
    /// Un extremo no es un pin, un literal ni una variable válida (o está vacío).
    InvalidEndpoint = "invalid_endpoint",
    /// El destino de un valor no es una entrada.
    NotAnInput = "not_an_input",
    /// La entrada ya tiene un valor (cable, variable o literal).
    InputAlreadyDriven = "input_already_driven",
    /// El origen de un cable no es una salida.
    NotAnOutput = "not_an_output",
    /// Una salida o un IN_OUT apunta a un literal en vez de una variable.
    OutputToLiteral = "output_to_literal",
    /// La salida ya escribe otra variable.
    OutputAlreadyAssigned = "output_already_assigned",
    /// Ningún extremo del link es un pin de bloque.
    NoBlockEndpoint = "no_block_endpoint",
    /// El ancho del bloque en el catálogo es una estimación (aviso).
    EstimatedWidth = "estimated_width",
    /// Una entrada extensible quedó sin conectar (aviso).
    UnconnectedExtensibleInput = "unconnected_extensible_input",
    /// Un IN_OUT quedó sin conectar del lado de entrada (aviso).
    UnconnectedInOut = "unconnected_in_out",

    // --- Tipos (Stage::Types) ---
    /// El valor inicial no es un literal o es de otro tipo.
    InvalidInitialValue = "invalid_initial_value",
    /// Tipos distintos unidos por un cable o una variable (error entre elementales,
    /// aviso entre derivados).
    TypeMismatch = "type_mismatch",
    /// El tipo no pertenece a la familia genérica del pin (`ANY_INT`...).
    GenericTypeMismatch = "generic_type_mismatch",
    /// No se pudo deducir el tipo de una variable: hay que declararla.
    UndeducibleType = "undeducible_type",
    /// Un campo o índice cuya raíz no está declarada ni es una instancia.
    UndeclaredRoot = "undeclared_root",

    // --- Consistencia entre equipos (Stage::Consistency) ---
    /// Varios equipos comparten un típico (información).
    SharedTypical = "shared_typical",
    /// Un equipo usa en un pin un literal distinto al del resto de su típico.
    LiteralOutlier = "literal_outlier",
    /// Un pin del típico no tiene un valor mayoritario.
    NoMajorityValue = "no_majority_value",
    /// Un equipo se parece a un típico compartido pero difiere (aviso).
    TypicalDeviation = "typical_deviation",
    /// Dos equipos únicos son casi iguales.
    NearDuplicate = "near_duplicate",

    // --- Partición y orden de secciones (Stage::Partition) ---
    /// Un grupo superó `maxBlocks` y se dividió.
    SectionSplit = "section_split",
    /// Un nombre de sección se repetía y se renombró.
    SectionRenamed = "section_renamed",
    /// Un bloque está más conectado con otra sección que con la suya (aviso).
    WeakCohesion = "weak_cohesion",
    /// Un cable entre secciones se reemplazó por una variable.
    CrossSectionLink = "cross_section_link",
    /// Una variable se escribe en varias secciones (aviso).
    MultipleWriters = "multiple_writers",
    /// Una sección lee un valor que otra escribe después (aviso).
    CircularDependency = "circular_dependency",
    /// Orden de ejecución de las secciones.
    SectionOrder = "section_order",

    // --- Normalización (Stage::Normalize) ---
    /// Un lazo se cortó con una variable (lectura del scan anterior).
    FeedbackCut = "feedback_cut",
    /// Un fan-out alto se reemplazó por una variable.
    FanoutToVariable = "fanout_to_variable",
    /// Un fan-out alto se conservó para respetar el orden de ejecución.
    FanoutKept = "fanout_kept",

    // --- Layout (Stage::Layout) ---
    /// Se reordenaron entradas conmutativas para reducir cruces.
    InputsReordered = "inputs_reordered",
}

impl std::fmt::Display for DiagnosticCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Serialize for DiagnosticCode {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

/// Dónde está lo que señala un diagnóstico, con los nombres e índices de la entrada.
/// Solo se completan los campos que aplican; al serializar se omiten los vacíos.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct DiagnosticLocation {
    /// Id del bloque (`BlockSpec::id`), tal como se escribió en la entrada.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block: Option<String>,
    /// Pin del bloque (`IN1`, `Q`...).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pin: Option<String>,
    /// Índice del link en `links`, desde 0.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link: Option<usize>,
    /// Variable (declarada, usada en un link o creada por el generador).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variable: Option<String>,
    /// Índice del comentario en `comments`, desde 0.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<usize>,
    /// Equipo (`BlockSpec::group`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    /// Sección generada.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub section: Option<String>,
}

impl DiagnosticLocation {
    /// Sin ningún dato de ubicación: el diagnóstico es del programa entero.
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// Un hallazgo de una etapa del pipeline.
///
/// `code` y `location` son para programas (un cliente, el LLM que corrige su grafo);
/// `message` es para personas y puede cambiar de redacción entre versiones.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct Diagnostic {
    /// Gravedad.
    pub level: Level,
    /// Etapa que lo emitió.
    pub stage: Stage,
    /// Qué se detectó, con un nombre estable.
    pub code: DiagnosticCode,
    /// Dónde, con los nombres e índices de la entrada.
    #[serde(skip_serializing_if = "DiagnosticLocation::is_empty")]
    pub location: DiagnosticLocation,
    /// Texto para personas; puede cambiar de redacción entre versiones.
    pub message: String,
}

impl Diagnostic {
    pub(crate) fn new(
        level: Level,
        stage: Stage,
        code: DiagnosticCode,
        message: impl Into<String>,
    ) -> Self {
        Self {
            level,
            stage,
            code,
            location: DiagnosticLocation::default(),
            message: message.into(),
        }
    }

    pub(crate) fn at_block(&mut self, id: impl Into<String>) -> &mut Self {
        self.location.block = Some(id.into());
        self
    }

    /// Bloque y pin de una referencia dentro de `section`.
    pub(crate) fn at_pin(&mut self, section: &Section, r: PinRef) -> &mut Self {
        let b = &section.blocks[r.block];
        self.location.block = Some(b.id.clone());
        self.location.pin = Some(b.pins[r.pin].name.clone());
        self
    }

    pub(crate) fn at_pin_name(&mut self, pin: impl Into<String>) -> &mut Self {
        self.location.pin = Some(pin.into());
        self
    }

    pub(crate) fn at_link(&mut self, index: usize) -> &mut Self {
        self.location.link = Some(index);
        self
    }

    pub(crate) fn at_variable(&mut self, name: impl Into<String>) -> &mut Self {
        self.location.variable = Some(name.into());
        self
    }

    pub(crate) fn at_comment(&mut self, index: usize) -> &mut Self {
        self.location.comment = Some(index);
        self
    }

    pub(crate) fn at_group(&mut self, group: impl Into<String>) -> &mut Self {
        self.location.group = Some(group.into());
        self
    }

    pub(crate) fn at_section(&mut self, section: impl Into<String>) -> &mut Self {
        self.location.section = Some(section.into());
        self
    }
}

/// `[warning] normalize/fanout_to_variable: ...`: gravedad, etapa, código y mensaje.
impl std::fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "[{}] {}/{}: {}",
            self.level, self.stage, self.code, self.message
        )
    }
}

#[derive(Debug, Default)]
pub(crate) struct Diagnostics(pub Vec<Diagnostic>);

impl Diagnostics {
    /// Agrega un diagnóstico y lo devuelve para completar su ubicación con `at_*`.
    pub(crate) fn push(
        &mut self,
        level: Level,
        stage: Stage,
        code: DiagnosticCode,
        message: impl Into<String>,
    ) -> &mut Diagnostic {
        self.0.push(Diagnostic::new(level, stage, code, message));
        let last = self.0.len() - 1;
        &mut self.0[last]
    }

    pub(crate) fn has_errors(&self) -> bool {
        self.0.iter().any(|d| d.level == Level::Error)
    }
}
