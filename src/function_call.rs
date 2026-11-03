use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize};

use crate::generator::generate_section_with;
use crate::library::PinDef;
use crate::model::default_task;
use crate::{
    BlockSpec, Diagnostic, GeneratedSection, GenerationError, Generator, Level, LinkSpec,
    ProgramSpec, VarSpec,
};

/// Una llamada a una EF/EFB/DFB existente, exportada como una sección de un bloque.
/// Los mapas relacionan nombres de pines con variables o literales IEC.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FunctionCallSpec {
    /// Nombre de la sección generada.
    pub name: String,
    /// Tipo de bloque del catálogo, por ejemplo `TON`, `AND` o una DFB registrada.
    pub function: String,
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
    #[serde(default)]
    pub variables: Vec<VarSpec>,
    pub comment: Option<String>,
}

impl FunctionCallSpec {
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

    pub fn with_instance(mut self, instance: impl Into<String>) -> Self {
        self.instance = Some(instance.into());
        self
    }

    pub fn input(mut self, pin: impl Into<String>, value: impl Into<String>) -> Self {
        self.inputs.insert(pin.into(), value.into());
        self
    }

    pub fn output(mut self, pin: impl Into<String>, variable: impl Into<String>) -> Self {
        self.outputs.insert(pin.into(), variable.into());
        self
    }

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
    pub fn generate_function(
        &self,
        spec: &FunctionCallSpec,
    ) -> Result<GeneratedSection, GenerationError> {
        let invalid = |message: String| GenerationError::InvalidInput {
            scope: spec.name.clone(),
            diagnostics: vec![Diagnostic {
                level: Level::Error,
                stage: crate::model::Stage::Validate,
                message,
            }],
        };
        let Some(def) = self.library().get(&spec.function) else {
            return Err(invalid(format!(
                "tipo de bloque desconocido {}",
                spec.function
            )));
        };
        if !def.kind.has_instance() && spec.instance.is_some() {
            return Err(invalid(format!(
                "{} es una EF y no admite instancia",
                def.name
            )));
        }
        if spec.instance.as_ref().is_some_and(|s| s.trim().is_empty()) {
            return Err(invalid(
                "el nombre de instancia no puede estar vacío".into(),
            ));
        }
        if let Some(count) = spec.input_count {
            let ext = def.inputs.iter().find_map(|p| p.extensible.as_ref());
            let Some(ext) = ext else {
                return Err(invalid(format!(
                    "{} no tiene entradas extensibles",
                    def.name
                )));
            };
            if count < ext.min || count > ext.max {
                return Err(invalid(format!(
                    "{}: inputCount {count} fuera de rango {}..={}",
                    def.name, ext.min, ext.max
                )));
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
                    return Err(invalid(format!(
                        "{}.{pin}: no es un pin de {role} gráfico válido",
                        def.name
                    )));
                }
                if value.trim().is_empty() {
                    return Err(invalid(format!(
                        "{}.{pin}: el parámetro actual no puede estar vacío",
                        def.name
                    )));
                }
                if (is_output || role == "IN_OUT") && crate::iec::is_literal(value) {
                    return Err(invalid(format!(
                        "{}.{pin}: {role} requiere una variable, no un literal",
                        def.name
                    )));
                }
                if matches!(
                    crate::resolve::parse_endpoint(value, &ids),
                    crate::resolve::Endpoint::Pin { .. }
                ) {
                    return Err(invalid(format!(
                        "{}.{pin}: el parámetro actual debe ser una variable o literal, no un pin",
                        def.name
                    )));
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
