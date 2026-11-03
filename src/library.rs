//! Carga de la librería de bloques (`efb_dimensions.json`, exportada de Control Expert 15.1)
//! y cálculo de la geometría por bloque.

use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

mod registry;
pub use registry::LibraryError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Kind {
    Ef,
    Efb,
    Dfb,
}

impl Kind {
    /// EFB y DFB son instancias con nombre; las EF no.
    pub fn has_instance(self) -> bool {
        !matches!(self, Kind::Ef)
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Extensible {
    pub min: u32,
    pub max: u32,
    /// `IN1` -> prefijo `IN`, primer índice 1; `IN0` -> `IN`, 0.
    pub prefix: String,
    pub first_index: u32,
}

impl Extensible {
    pub fn pin_name(&self, k: u32) -> String {
        format!("{}{}", self.prefix, self.first_index + k)
    }

    /// Posición `k` (desde 0) del pin `name` en la extensión: la inversa de
    /// [`Extensible::pin_name`], sin distinguir mayúsculas. No mira el máximo.
    pub fn index_of(&self, name: &str) -> Option<u32> {
        let prefix = name.get(..self.prefix.len())?;
        if !prefix.eq_ignore_ascii_case(&self.prefix) {
            return None;
        }
        name[self.prefix.len()..]
            .parse::<u32>()
            .ok()?
            .checked_sub(self.first_index)
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PinDef {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: String,
    /// Posición en su lado (1 = arriba). `None` = pin sin representación gráfica.
    pub position: Option<u32>,
    pub extensible: Option<Extensible>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BlockDef {
    pub name: String,
    pub kind: Kind,
    pub typed_variant_of: Option<String>,
    #[serde(default)]
    pub inputs: Vec<PinDef>,
    #[serde(default)]
    pub outputs: Vec<PinDef>,
    #[serde(default)]
    pub in_outs: Vec<PinDef>,
    /// Ancho en celdas de grilla (medido o estimado).
    pub width: u32,
    #[serde(default = "measured_width")]
    pub width_measured: bool,
}

fn measured_width() -> bool {
    true
}

impl BlockDef {
    /// Nombre de la función genérica (`ADD_REAL` -> `ADD`).
    pub fn generic_name(&self) -> &str {
        self.typed_variant_of.as_deref().unwrap_or(&self.name)
    }

    /// Bloques cuyas entradas extensibles se pueden permutar sin cambiar el resultado.
    pub fn is_commutative(&self) -> bool {
        matches!(
            self.generic_name(),
            "AND" | "OR" | "XOR" | "ADD" | "MUL" | "MAX" | "MIN"
        )
    }
}

#[derive(Debug, Clone, Default)]
pub struct Library {
    blocks: HashMap<String, BlockDef>,
}

impl Library {
    pub fn load(path: impl AsRef<Path>) -> Result<Self, LibraryError> {
        let path = path.as_ref();
        let text = std::fs::read_to_string(path).map_err(|source| LibraryError::Io {
            path: path.to_owned(),
            source,
        })?;
        Self::from_json_str(&text)
    }

    /// Carga la librería desde el JSON ya leído (mismo formato que `efb_dimensions.json`).
    /// Cada bloque pasa por las mismas validaciones que uno registrado con
    /// [`Library::register`], y un nombre repetido es un error.
    pub fn from_json_str(text: &str) -> Result<Self, LibraryError> {
        let raw: RawLibrary = serde_json::from_str(text).map_err(LibraryError::Json)?;
        let definitions = raw
            .blocks
            .into_iter()
            .map(RawBlock::into_def)
            .collect::<Result<Vec<_>, _>>()?;
        let mut library = Self::default();
        library.register_all(definitions)?;
        Ok(library)
    }

    /// La librería de Control Expert 15.1 incluida en el binario (feature `embedded-library`).
    /// Se lee una sola vez por proceso; cada llamada devuelve una copia propia.
    #[cfg(feature = "embedded-library")]
    pub fn embedded() -> Result<Self, LibraryError> {
        static EMBEDDED: std::sync::OnceLock<Library> = std::sync::OnceLock::new();
        if let Some(library) = EMBEDDED.get() {
            return Ok(library.clone());
        }
        let library = Self::from_json_str(include_str!("../efb_dimensions.json"))?;
        Ok(EMBEDDED.get_or_init(|| library).clone())
    }

    pub fn get(&self, name: &str) -> Option<&BlockDef> {
        self.blocks.get(&name.to_ascii_uppercase())
    }

    pub fn len(&self) -> usize {
        self.blocks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }
}

/// Estimación conservadora del ancho para los bloques que Control Expert no pudo medir.
/// Ajustada contra los 1092 bloques medidos: tiende a sobreestimar, lo que es preferible a
/// que los bloques se pisen.
/// Se calcula en enteros: 5 celdas cada 8 caracteres de nombre de pin.
fn estimate_width(name: &str, inputs: &[RawPin], outputs: &[RawPin], in_outs: &[RawPin]) -> u32 {
    let longest = |pins: &mut dyn Iterator<Item = &RawPin>| {
        pins.map(|p| p.name.chars().count() as u32)
            .max()
            .unwrap_or(0)
    };
    let left = longest(&mut inputs.iter().chain(in_outs));
    let right = longest(&mut outputs.iter().chain(in_outs));
    let by_pins = (left * 5).div_ceil(8) + (right * 5).div_ceil(8) + 2;
    let by_name = name.chars().count() as u32 + 4;
    by_pins.max(by_name).max(7)
}

#[derive(Deserialize)]
struct RawLibrary {
    blocks: Vec<RawBlock>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawBlock {
    name: String,
    kind: String,
    typed_variant_of: Option<String>,
    geometry: RawGeometry,
    size: Option<RawSize>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawGeometry {
    #[serde(default)]
    inputs: Vec<RawPin>,
    #[serde(default)]
    outputs: Vec<RawPin>,
    #[serde(default)]
    in_outs: Vec<RawPin>,
}

#[derive(Deserialize)]
struct RawPin {
    name: String,
    #[serde(rename = "type")]
    ty: String,
    position: Option<u32>,
    extensible: Option<RawExtensible>,
}

#[derive(Deserialize)]
struct RawExtensible {
    min: u32,
    max: u32,
}

#[derive(Deserialize)]
struct RawSize {
    width: Option<u32>,
}

impl RawBlock {
    fn into_def(self) -> Result<BlockDef, LibraryError> {
        let kind = match self.kind.as_str() {
            "EF" => Kind::Ef,
            "EFB" => Kind::Efb,
            "DFB" => Kind::Dfb,
            other => {
                return Err(LibraryError::InvalidDefinition {
                    block: self.name,
                    message: format!("tipo de bloque desconocido \"{other}\" (EF, EFB o DFB)"),
                });
            }
        };
        let measured = self.size.as_ref().and_then(|s| s.width);
        let g = self.geometry;
        let width = measured
            .unwrap_or_else(|| estimate_width(&self.name, &g.inputs, &g.outputs, &g.in_outs));
        let conv = |pins: Vec<RawPin>| pins.into_iter().map(RawPin::into_def).collect();
        Ok(BlockDef {
            name: self.name,
            kind,
            typed_variant_of: self.typed_variant_of,
            inputs: conv(g.inputs),
            outputs: conv(g.outputs),
            in_outs: conv(g.in_outs),
            width,
            width_measured: measured.is_some(),
        })
    }
}

impl RawPin {
    fn into_def(self) -> PinDef {
        let extensible = self.extensible.map(|e| {
            let digits = self
                .name
                .chars()
                .rev()
                .take_while(|c| c.is_ascii_digit())
                .count();
            let (prefix, idx) = self.name.split_at(self.name.len() - digits);
            Extensible {
                min: e.min,
                max: e.max,
                prefix: prefix.to_string(),
                first_index: idx.parse().unwrap_or(1),
            }
        });
        PinDef {
            name: self.name,
            ty: self.ty,
            position: self.position.filter(|&p| p > 0),
            extensible,
        }
    }
}
