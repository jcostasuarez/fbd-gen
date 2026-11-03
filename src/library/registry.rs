use std::collections::HashSet;
use std::error::Error;
use std::fmt;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use super::{BlockDef, Kind, Library, PinDef};
use crate::iec::is_identifier as identifier;
use crate::model::Dir;

/// Tope del ancho de un bloque y de la última fila de sus pines, en celdas. La librería
/// de Control Expert 15.1 llega a 20 de ancho y a la posición 34: el tope solo descarta
/// definiciones absurdas, que harían desbordar las cuentas enteras del layout.
pub const MAX_BLOCK_CELLS: u32 = 1000;

/// Errores de lectura y registro del catálogo, sin reemplazos implícitos.
#[derive(Debug)]
#[non_exhaustive]
pub enum LibraryError {
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    Json(serde_json::Error),
    DuplicateBlock {
        name: String,
    },
    InvalidDefinition {
        block: String,
        message: String,
    },
}

impl fmt::Display for LibraryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => write!(f, "no se pudo leer {}: {source}", path.display()),
            Self::Json(source) => write!(f, "JSON de librería inválido: {source}"),
            Self::DuplicateBlock { name } => write!(f, "el bloque {name} ya existe en la librería"),
            Self::InvalidDefinition { block, message } => {
                write!(f, "definición {block} inválida: {message}")
            }
        }
    }
}

impl Error for LibraryError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Json(source) => Some(source),
            _ => None,
        }
    }
}

impl BlockDef {
    /// Define la interfaz y geometría de una DFB; no contiene su implementación IEC.
    pub fn dfb(name: impl Into<String>, width: u32) -> Self {
        Self::new(name, Kind::Dfb, width)
    }

    /// Interfaz vacía de un bloque del tipo `kind`, con `width` celdas de ancho medido.
    /// Los pines se agregan con [`BlockDef::input`], [`BlockDef::output`],
    /// [`BlockDef::in_out`] o [`BlockDef::pin`].
    pub fn new(name: impl Into<String>, kind: Kind, width: u32) -> Self {
        Self {
            name: name.into(),
            kind,
            width,
            width_measured: true,
            typed_variant_of: None,
            inputs: Vec::new(),
            outputs: Vec::new(),
            in_outs: Vec::new(),
        }
    }

    /// `position` empieza en 1. La fila de grilla incluye además la cabecera del bloque.
    pub fn input(mut self, name: impl Into<String>, ty: impl Into<String>, position: u32) -> Self {
        self.inputs.push(PinDef::new(name, ty, position));
        self
    }

    pub fn output(mut self, name: impl Into<String>, ty: impl Into<String>, position: u32) -> Self {
        self.outputs.push(PinDef::new(name, ty, position));
        self
    }

    pub fn in_out(mut self, name: impl Into<String>, ty: impl Into<String>, position: u32) -> Self {
        self.in_outs.push(PinDef::new(name, ty, position));
        self
    }

    /// Agrega un pin ya armado (por ejemplo, una entrada extensible) del lado `dir`.
    pub fn pin(mut self, dir: Dir, pin: PinDef) -> Self {
        match dir {
            Dir::In => self.inputs.push(pin),
            Dir::Out => self.outputs.push(pin),
            Dir::InOut => self.in_outs.push(pin),
        }
        self
    }
}

fn validate(def: &BlockDef) -> Result<(), LibraryError> {
    let bad = |message: String| LibraryError::InvalidDefinition {
        block: def.name.clone(),
        message,
    };
    if !identifier(&def.name) {
        return Err(bad("nombre IEC inválido".into()));
    }
    if !(2..=MAX_BLOCK_CELLS).contains(&def.width) {
        return Err(bad(format!(
            "ancho {} fuera de rango (2 a {MAX_BLOCK_CELLS})",
            def.width
        )));
    }
    if def.kind != Kind::Ef && def.typed_variant_of.is_some() {
        return Err(bad("solo una EF puede ser una variante tipada".into()));
    }
    let mut names = HashSet::from(["EN".to_string(), "ENO".to_string()]);
    let mut rows: [Vec<(u32, u32)>; 2] = [Vec::new(), Vec::new()];
    let mut extensible = None;
    for (side, pins) in [(0, &def.inputs), (1, &def.outputs), (2, &def.in_outs)] {
        for pin in pins {
            if !identifier(&pin.name) || !names.insert(pin.name.to_ascii_uppercase()) {
                return Err(bad(format!(
                    "pin {} inválido, duplicado o reservado",
                    pin.name
                )));
            }
            if pin.ty.trim().is_empty() || pin.ty.chars().any(char::is_control) {
                return Err(bad(format!(
                    "{}: tipo vacío o con caracteres de control",
                    pin.name
                )));
            }
            let mut count = 1;
            if let Some(ext) = &pin.extensible {
                if def.kind != Kind::Ef || side != 0 || extensible.is_some() {
                    return Err(bad(
                        "se admite un único pin extensible, de entrada y en una EF".into(),
                    ));
                }
                if ext.min == 0
                    || ext.max < ext.min
                    || !identifier(&ext.prefix)
                    || ext.first_index.checked_add(ext.max - 1).is_none()
                    || !ext.pin_name(0).eq_ignore_ascii_case(&pin.name)
                {
                    return Err(bad(format!("{}: extensión inválida", pin.name)));
                }
                count = ext.max;
                extensible = Some((ext, &pin.name));
            }
            if let Some(start) = pin.position {
                let end = start
                    .checked_add(count - 1)
                    .filter(|&n| n <= MAX_BLOCK_CELLS - 4);
                let Some(end) = end.filter(|_| start > 0) else {
                    return Err(bad(format!(
                        "{}: posición fuera de rango (1 a {})",
                        pin.name,
                        MAX_BLOCK_CELLS - 4
                    )));
                };
                for (i, occupied) in rows.iter_mut().enumerate() {
                    if side == i || side == 2 {
                        if occupied.iter().any(|&(a, b)| start <= b && a <= end) {
                            return Err(bad(format!(
                                "{}: posición superpuesta con otro pin",
                                pin.name
                            )));
                        }
                        occupied.push((start, end));
                    }
                }
            }
        }
    }
    if let Some((ext, first)) = extensible {
        for name in &names {
            if name.eq_ignore_ascii_case(first) {
                continue;
            }
            if ext.index_of(name).is_some_and(|k| k < ext.max) {
                return Err(bad(format!("el pin {name} colisiona con la extensión")));
            }
        }
    }
    Ok(())
}

impl Library {
    /// Registra una EF/EFB/DFB propia. Rechaza duplicados sin distinguir mayúsculas.
    pub fn register(&mut self, definition: BlockDef) -> Result<(), LibraryError> {
        self.register_all(vec![definition])
    }

    /// Registro atómico: si una definición falla, el catálogo no se modifica.
    pub fn register_all(&mut self, definitions: Vec<BlockDef>) -> Result<(), LibraryError> {
        let mut names = HashSet::new();
        for def in &definitions {
            validate(def)?;
            let key = def.name.to_ascii_uppercase();
            if self.blocks.contains_key(&key) || !names.insert(key) {
                return Err(LibraryError::DuplicateBlock {
                    name: def.name.clone(),
                });
            }
        }
        for def in definitions {
            self.blocks.insert(def.name.to_ascii_uppercase(), def);
        }
        Ok(())
    }

    /// Carga extensiones con formato `{ "blocks": [BlockDefinition, ...] }`.
    pub fn extend_from_json(&mut self, json: &str) -> Result<(), LibraryError> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Extensions {
            blocks: Vec<BlockDef>,
        }
        let extensions: Extensions = serde_json::from_str(json).map_err(LibraryError::Json)?;
        self.register_all(extensions.blocks)
    }

    pub fn extend_from_file(&mut self, path: impl AsRef<Path>) -> Result<(), LibraryError> {
        let path = path.as_ref();
        let json = std::fs::read_to_string(path).map_err(|source| LibraryError::Io {
            path: path.to_owned(),
            source,
        })?;
        self.extend_from_json(&json)
    }
}
