use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize};

use crate::generator::generate_section_with;
use crate::library::PinDef;
use crate::model::Stage;
use crate::model::default_task;
use crate::{
    BlockSpec, Diagnostic, DiagnosticCode as Code, GeneratedSection, GenerationError, Generator,
    Level, LinkSpec, ProgramSpec, VarSpec,
};

/// Una llamada a una EF/EFB/DFB existente, exportada como una sección de un bloque.
/// Los mapas relacionan nombres de pines con variables o literales IEC.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[non_exhaustive]
pub struct FunctionCallSpec {
    /// Nombre de la sección generada.
    pub name: String,
    /// Tipo de bloque del catálogo, por ejemplo `TON`, `AND` o una DFB registrada.
    pub function: String,
    /// Tarea dueña de la sección (MAST por defecto).
    #[serde(default = "default_task")]
    pub task: String,
    /// Solo EFB/DFB. Si falta, se usa `<nombre saneado de sección>_call`.
    pub instance: Option<String>,
    /// Variables o literales IEC para pines de entrada comunes.
    #[serde(default)]
    pub inputs: BTreeMap<String, String>,
    /// Variables destino de los pines de salida comunes.
    #[serde(default)]
    pub outputs: BTreeMap<String, String>,
    /// Una variable por pin IN_OUT; no admite literales.
    #[serde(default)]
    pub in_outs: BTreeMap<String, String>,
    /// Cantidad explícita de entradas extensibles. Si falta, se deduce de los pines.
    pub input_count: Option<u32>,
    /// Declaraciones explícitas. Las variables no declaradas se tipan por inferencia.
    #[serde(default)]
    pub variables: Vec<VarSpec>,
    /// Comentario del bloque generado.
    pub comment: Option<String>,
}

impl FunctionCallSpec {
    /// Como [`ProgramSpec::from_json`], para una llamada.
    ///
    /// # Errors
    ///
    /// [`GenerationError::InvalidInput`] con el código `invalid_json` si el texto no es
    /// JSON válido, le falta un campo obligatorio o tiene uno desconocido.
    pub fn from_json(json: &str) -> Result<Self, GenerationError> {
        crate::generator::from_json(json)
    }

    /// Llamada `name` a la función `function`, sin pines asignados.
    pub fn new(name: impl Into<String>, function: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            function: function.into(),
            task: default_task(),
            instance: None,
            inputs: BTreeMap::new(),
            outputs: BTreeMap::new(),
            in_outs: BTreeMap::new(),
            input_count: None,
            variables: Vec::new(),
            comment: None,
        }
    }

    /// Nombre de la instancia EFB/DFB; si no se da, se deriva del nombre de la sección.
    #[must_use]
    pub fn with_instance(mut self, instance: impl Into<String>) -> Self {
        self.instance = Some(instance.into());
        self
    }

    /// Asigna una variable o un literal IEC al pin de entrada `pin`.
    #[must_use]
    pub fn input(mut self, pin: impl Into<String>, value: impl Into<String>) -> Self {
        self.inputs.insert(pin.into(), value.into());
        self
    }

    /// Conecta el pin de salida `pin` con la variable `variable`.
    #[must_use]
    pub fn output(mut self, pin: impl Into<String>, variable: impl Into<String>) -> Self {
        self.outputs.insert(pin.into(), variable.into());
        self
    }

    /// Conecta el pin IN_OUT `pin` con la variable `variable`.
    #[must_use]
    pub fn in_out(mut self, pin: impl Into<String>, variable: impl Into<String>) -> Self {
        self.in_outs.insert(pin.into(), variable.into());
        self
    }
}

fn matches_pin(pin: &PinDef, name: &str) -> bool {
    if pin.position.is_none() {
        return false;
    }
    if pin.name.eq_ignore_ascii_case(name) {
        return true;
    }
    pin.extensible
        .as_ref()
        .is_some_and(|ext| ext.index_of(name).is_some_and(|k| k < ext.max))
}

impl Generator {
    /// Genera una llamada a un bloque ya conocido. No crea su implementación ni lo
    /// registra: las DFB propias deben incorporarse antes con `register_block`.
    ///
    /// ```
    /// use fbd_gen::{FunctionCallSpec, Generator};
    ///
    /// let generator = Generator::from_library_file("efb_dimensions.json")?;
    /// let call = FunctionCallSpec::new("P101_Retardo", "TON")
    ///     .with_instance("P101_Timer")
    ///     .input("IN", "P101_Start")
    ///     .input("PT", "T#3s")
    ///     .output("Q", "P101_Run");
    /// let section = generator.generate_function(&call)?;
    /// assert!(section.variables.iter().any(|v| v.name == "P101_Timer" && v.ty == "TON"));
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    ///
    /// # Errors
    ///
    /// [`GenerationError::InvalidInput`] si `function` no está en el catálogo, si un pin
    /// no existe o no corresponde a su lado, o si la sección resultante es inválida;
    /// [`GenerationError::Layout`] si no se puede dibujar.
    pub fn generate_function(
        &self,
        spec: &FunctionCallSpec,
    ) -> Result<GeneratedSection, GenerationError> {
        // Los errores señalan el pin de la llamada (sus claves en `inputs`, `outputs`...).
        let invalid_at = |code: Code, pin: Option<&str>, message: String| {
            let mut d = Diagnostic::new(Level::Error, Stage::Validate, code, message);
            if let Some(pin) = pin {
                d.at_pin_name(pin);
            }
            GenerationError::InvalidInput {
                scope: spec.name.clone(),
                diagnostics: vec![d],
            }
        };
        let invalid = |code: Code, message: String| invalid_at(code, None, message);
        let Some(def) = self.library().get(&spec.function) else {
            return Err(invalid(
                Code::UnknownBlockType,
                format!("tipo de bloque desconocido {}", spec.function),
            ));
        };
        if !def.kind.has_instance() && spec.instance.is_some() {
            return Err(invalid(
                Code::InstanceNotAllowed,
                format!("{} es una EF y no admite instancia", def.name),
            ));
        }
        if spec.instance.as_ref().is_some_and(|s| s.trim().is_empty()) {
            return Err(invalid(
                Code::InvalidInstanceName,
                "el nombre de instancia no puede estar vacío".into(),
            ));
        }
        if let Some(count) = spec.input_count {
            let ext = def.inputs.iter().find_map(|p| p.extensible.as_ref());
            let Some(ext) = ext else {
                return Err(invalid(
                    Code::NotExtensible,
                    format!("{} no tiene entradas extensibles", def.name),
                ));
            };
            if count < ext.min || count > ext.max {
                return Err(invalid(
                    Code::TooManyInputs,
                    format!(
                        "{}: inputCount {count} fuera de rango {}..={}",
                        def.name, ext.min, ext.max
                    ),
                ));
            }
        }
        let id = format!("{}_call", crate::iec::sanitize(&spec.name));
        let ids = HashMap::from([(id.to_ascii_uppercase(), 0)]);
        let mut program = ProgramSpec::new(&spec.name);
        program.task.clone_from(&spec.task);
        program.variables.clone_from(&spec.variables);
        let mut block = BlockSpec::new(&id, &def.name);
        if def.kind.has_instance() {
            block.instance = Some(spec.instance.clone().unwrap_or_else(|| id.clone()));
        }
        block.inputs = spec.input_count;
        block.comment.clone_from(&spec.comment);
        program.blocks.push(block);

        for (bindings, pins, role, is_output) in [
            (&spec.inputs, &def.inputs, "entrada", false),
            (&spec.outputs, &def.outputs, "salida", true),
            (&spec.in_outs, &def.in_outs, "IN_OUT", false),
        ] {
            for (pin, value) in bindings {
                if !pins.iter().any(|p| matches_pin(p, pin)) {
                    return Err(invalid_at(
                        Code::UnknownPin,
                        Some(pin),
                        format!("{}.{pin}: no es un pin de {role} gráfico válido", def.name),
                    ));
                }
                if value.trim().is_empty() {
                    return Err(invalid_at(
                        Code::InvalidEndpoint,
                        Some(pin),
                        format!(
                            "{}.{pin}: el parámetro actual no puede estar vacío",
                            def.name
                        ),
                    ));
                }
                if (is_output || role == "IN_OUT") && crate::iec::is_literal(value) {
                    return Err(invalid_at(
                        Code::OutputToLiteral,
                        Some(pin),
                        format!(
                            "{}.{pin}: {role} requiere una variable, no un literal",
                            def.name
                        ),
                    ));
                }
                if matches!(
                    crate::resolve::parse_endpoint(value, &ids),
                    crate::resolve::Endpoint::Pin { .. }
                ) {
                    return Err(invalid_at(
                        Code::InvalidEndpoint,
                        Some(pin),
                        format!(
                            "{}.{pin}: el parámetro actual debe ser una variable o literal, no un pin",
                            def.name
                        ),
                    ));
                }
                let endpoint = format!("{id}.{pin}");
                program.links.push(if is_output {
                    LinkSpec::Pair(endpoint, value.clone())
                } else {
                    LinkSpec::Pair(value.clone(), endpoint)
                });
            }
        }
        generate_section_with(&program, self.library(), self.config())
    }
}
