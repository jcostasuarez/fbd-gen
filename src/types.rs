//! Etapa 2: inferencia de tipos de las variables para poder declararlas en el `dataBlock`.
//!
//! Unificación simple: dentro de una EF, los pines con el mismo tipo genérico (`ANY_BIT`,
//! `ANY_NUM`...) comparten tipo; un cable une sus dos pines; una variable une todos los
//! pines donde aparece. Los tipos concretos (pines, declaraciones) son fuertes; los
//! literales sin prefijo solo sugieren (`85.0` -> REAL); los tipados fijan su tipo.

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::graph::UnionFind;
use crate::iec::{is_literal, var_root};
use crate::library::Kind;
use crate::model::{Diagnostics, Level, Param, PinRef, Section, Stage, VarSpec};

const STAGE: Stage = Stage::Types;

fn is_generic(ty: &str) -> bool {
    ty.to_ascii_uppercase().starts_with("ANY")
}

fn typed_literal_type(lit: &str) -> Option<&'static str> {
    let u = lit.trim().to_ascii_uppercase();
    let (prefix, _) = u.split_once('#')?;
    Some(match prefix {
        "BOOL" => "BOOL",
        "EBOOL" => "EBOOL",
        "BYTE" => "BYTE",
        "WORD" => "WORD",
        "DWORD" => "DWORD",
        "LWORD" => "LWORD",
        "SINT" => "SINT",
        "INT" => "INT",
        "DINT" => "DINT",
        "LINT" => "LINT",
        "USINT" => "USINT",
        "UINT" => "UINT",
        "UDINT" => "UDINT",
        "ULINT" => "ULINT",
        "REAL" => "REAL",
        "LREAL" => "LREAL",
        "STRING" => "STRING",
        "WSTRING" => "WSTRING",
        "T" | "TIME" => "TIME",
        "D" | "DATE" => "DATE",
        "TOD" | "TIME_OF_DAY" => "TOD",
        "DT" | "DATE_AND_TIME" => "DT",
        // 2#, 8# y 16# indican la base, no un tipo de dato.
        _ => return None,
    })
}

/// Restricciones de las familias escalares. Las familias específicas de Schneider
/// (arrays, referencias e IODDT) requieren metadatos adicionales y no se restringen aquí.
fn accepts_generic(family: &str, ty: &str) -> bool {
    let ty = ty.to_ascii_uppercase();
    let bit = matches!(
        ty.as_str(),
        "BOOL" | "EBOOL" | "BYTE" | "WORD" | "DWORD" | "LWORD"
    );
    let int = matches!(
        ty.as_str(),
        "SINT" | "INT" | "DINT" | "LINT" | "USINT" | "UINT" | "UDINT" | "ULINT"
    );
    let real = matches!(ty.as_str(), "REAL" | "LREAL");
    match family.to_ascii_uppercase().as_str() {
        "ANY_BIT" => bit,
        "ANY_INT" => int,
        "ANY_REAL" => real,
        "ANY_NUM" => int || real,
        "ANY_ELEMENTARY" => {
            bit || int
                || real
                || matches!(
                    ty.as_str(),
                    "TIME"
                        | "DATE"
                        | "TOD"
                        | "DT"
                        | "TIME_OF_DAY"
                        | "DATE_AND_TIME"
                        | "STRING"
                        | "WSTRING"
                )
        }
        _ => true,
    }
}

fn literal_hint(lit: &str) -> Option<&'static str> {
    let u = lit.trim().to_ascii_uppercase();
    if u == "TRUE" || u == "FALSE" {
        Some("BOOL")
    } else if u.starts_with("T#") || u.starts_with("TIME#") {
        Some("TIME")
    } else if u.starts_with('\'') || u.starts_with('"') {
        Some("STRING")
    } else if u.contains('#') {
        None
    } else if u.contains('.') || u.contains('E') {
        Some("REAL")
    } else {
        Some("INT")
    }
}

/// Forma canónica de un tipo: sin espacios ni distinción de mayúsculas, con los alias
/// IEC unificados. EBOOL se conecta donde se espera BOOL, así que no es un conflicto.
fn canonical(ty: &str) -> String {
    let t: String = ty
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .to_ascii_uppercase();
    match t.as_str() {
        "EBOOL" => "BOOL".into(),
        "TIME_OF_DAY" => "TOD".into(),
        "DATE_AND_TIME" => "DT".into(),
        _ => t,
    }
}

fn compatible(a: &str, b: &str) -> bool {
    canonical(a) == canonical(b)
}

/// Tipo elemental IEC: entre dos elementales distintos no hay conversión implícita, así
/// que Control Expert rechaza la conexión. Los derivados (arrays, DDT) pueden tener alias.
fn is_elementary(ty: &str) -> bool {
    matches!(
        canonical(ty).as_str(),
        "BOOL"
            | "BYTE"
            | "WORD"
            | "DWORD"
            | "LWORD"
            | "SINT"
            | "INT"
            | "DINT"
            | "LINT"
            | "USINT"
            | "UINT"
            | "UDINT"
            | "ULINT"
            | "REAL"
            | "LREAL"
            | "TIME"
            | "DATE"
            | "TOD"
            | "DT"
            | "STRING"
            | "WSTRING"
    )
}

/// Valor inicial de una variable elemental: tiene que ser un literal y, si es tipado
/// (`T#5s`, `INT#3`), del tipo declarado. Un literal sin prefijo solo sugiere un tipo,
/// igual que en un pin, así que no se rechaza. Los derivados (arrays, DDT) se inicializan
/// por elemento y no se validan aquí.
fn check_init(decl: &VarSpec, diags: &mut Diagnostics) {
    let Some(init) = decl.init.as_deref().filter(|_| is_elementary(&decl.ty)) else {
        return;
    };
    let problem = if !is_literal(init) {
        "no es un literal".to_string()
    } else if let Some(ty) = typed_literal_type(init).filter(|ty| !compatible(ty, &decl.ty)) {
        format!("es {ty}")
    } else {
        return;
    };
    diags.push(
        Level::Error,
        STAGE,
        format!(
            "variable {} ({}): el valor inicial \"{init}\" {problem}",
            decl.name, decl.ty
        ),
    );
}

/// Tipo concreto que impone un pin, una declaración o un literal tipado, con su origen
/// para los mensajes.
struct Strong {
    node: usize,
    ty: String,
    origin: String,
}

/// Una variable de la sección con su tipo final, en orden de primera aparición.
#[derive(Debug, Clone)]
pub(crate) struct TypedVar {
    pub name: String,
    pub ty: String,
    pub decl: Option<VarSpec>,
}

/// Tipa las variables de la sección y deja en cada pin su tipo concreto
/// ([`Pin::inferred`]). Corre una sola vez, sobre el grafo completo: la partición y la
/// normalización solo copian esos tipos.
pub(crate) fn infer(
    section: &mut Section,
    declared: &[VarSpec],
    diags: &mut Diagnostics,
) -> Vec<TypedVar> {
    for d in declared {
        check_init(d, diags);
    }
    let decl_by_name: HashMap<String, &VarSpec> = declared
        .iter()
        .map(|d| (d.name.to_ascii_uppercase(), d))
        .collect();
    let graph = TypeGraph::new(section, declared);
    let classes = Classes::new(graph);
    classes.report_conflicts(diags);
    for (&r, &n) in &classes.pin_node {
        section.pin_mut(r).inferred = classes.type_of(classes.class_of[n]);
    }
    check_generic_pins(section, &classes, diags);
    let result = typed_vars(section, &classes, &decl_by_name, diags);
    for g in &mut section.generated_vars {
        if let Some(v) = result.iter().find(|v| v.name.eq_ignore_ascii_case(&g.name)) {
            g.ty = v.ty.clone();
        }
    }
    result
}

/// Grafo de tipos: un nodo por pin y uno por variable, unidos por cables, parámetros y
/// pines genéricos de una misma EF. Cada clase de nodos unidos tiene un único tipo.
struct TypeGraph {
    /// Nodo de cada pin; los primeros `pin_node.len()` nodos son pines.
    pin_node: HashMap<PinRef, usize>,
    /// Variables en orden de primera aparición; la `i` es el nodo `pin_node.len() + i`.
    var_names: Vec<String>,
    /// Pares de nodos con el mismo tipo.
    links: Vec<(usize, usize)>,
    /// Tipos concretos impuestos por pines, declaraciones y literales tipados.
    strong: Vec<Strong>,
    /// Tipos sugeridos por literales sin prefijo (`5`, `1.5`).
    weak: Vec<(usize, &'static str)>,
}

impl TypeGraph {
    fn new(section: &Section, declared: &[VarSpec]) -> Self {
        let mut pin_node: HashMap<PinRef, usize> = HashMap::new();
        for (bi, b) in section.blocks.iter().enumerate() {
            for pi in 0..b.pins.len() {
                let n = pin_node.len();
                pin_node.insert(PinRef { block: bi, pin: pi }, n);
            }
        }
        let mut var_names: Vec<String> = Vec::new();
        let mut var_node: HashMap<String, usize> = HashMap::new();
        let mut var_of = |name: &str, var_names: &mut Vec<String>| -> usize {
            let key = name.to_ascii_uppercase();
            *var_node.entry(key).or_insert_with(|| {
                var_names.push(name.to_string());
                pin_node.len() + var_names.len() - 1
            })
        };

        let mut links: Vec<(usize, usize)> = Vec::new();
        let mut strong: Vec<Strong> = Vec::new();
        let mut weak: Vec<(usize, &'static str)> = Vec::new();
        for (bi, b) in section.blocks.iter().enumerate() {
            let mut generic_groups: HashMap<String, usize> = HashMap::new();
            for (pi, p) in b.pins.iter().enumerate() {
                let r = PinRef { block: bi, pin: pi };
                let n = pin_node[&r];
                if is_generic(&p.ty) {
                    if b.kind == Kind::Ef {
                        let first = *generic_groups.entry(p.ty.to_ascii_uppercase()).or_insert(n);
                        links.push((first, n));
                    }
                } else {
                    strong.push(Strong {
                        node: n,
                        ty: p.ty.clone(),
                        origin: section.pin_path(r),
                    });
                }
                for param in [&p.param_in, &p.param_out].into_iter().flatten() {
                    match param {
                        Param::Var(v) => links.push((n, var_of(v, &mut var_names))),
                        Param::Lit(l) => {
                            if let Some(ty) = typed_literal_type(l) {
                                strong.push(Strong {
                                    node: n,
                                    ty: ty.to_string(),
                                    origin: format!("{l} en {}", section.pin_path(r)),
                                });
                            } else {
                                weak.extend(literal_hint(l).map(|h| (n, h)));
                            }
                        }
                    }
                }
            }
        }
        links.extend(
            section
                .wires
                .iter()
                .map(|w| (pin_node[&w.from], pin_node[&w.to])),
        );
        for d in declared {
            if let Some(&n) = var_node.get(&d.name.to_ascii_uppercase()) {
                strong.push(Strong {
                    node: n,
                    ty: d.ty.clone(),
                    origin: format!("declaración de {}", d.name),
                });
            }
        }
        Self {
            pin_node,
            var_names,
            links,
            strong,
            weak,
        }
    }
}

/// Clases del grafo de tipos con sus tipos fuertes, sugeridos y variables.
struct Classes {
    pin_node: HashMap<PinRef, usize>,
    var_names: Vec<String>,
    /// Clase (representante) de cada nodo.
    class_of: Vec<usize>,
    /// Tipos fuertes por clase, con su origen, en el orden en que se encontraron.
    strong: BTreeMap<usize, Vec<Strong>>,
    weak: BTreeMap<usize, Vec<&'static str>>,
}

impl Classes {
    fn new(graph: TypeGraph) -> Self {
        let nodes = graph.pin_node.len() + graph.var_names.len();
        let mut uf = UnionFind::new(nodes);
        for (a, b) in graph.links {
            uf.union(a, b);
        }
        let class_of: Vec<usize> = (0..nodes).map(|n| uf.find(n)).collect();
        let mut strong: BTreeMap<usize, Vec<Strong>> = BTreeMap::new();
        for s in graph.strong {
            strong.entry(class_of[s.node]).or_default().push(s);
        }
        let mut weak: BTreeMap<usize, Vec<&'static str>> = BTreeMap::new();
        for (n, t) in graph.weak {
            weak.entry(class_of[n]).or_default().push(t);
        }
        Self {
            pin_node: graph.pin_node,
            var_names: graph.var_names,
            class_of,
            strong,
            weak,
        }
    }

    /// Clase de la variable `i` (en orden de aparición).
    fn var_class(&self, i: usize) -> usize {
        self.class_of[self.pin_node.len() + i]
    }

    /// Tipo de la clase: el primer tipo fuerte o, si no hay, el sugerido por los
    /// literales (`REAL` gana sobre los enteros).
    fn type_of(&self, class: usize) -> Option<String> {
        self.strong
            .get(&class)
            .and_then(|s| s.first())
            .map(|s| s.ty.clone())
            .or_else(|| {
                let w = self.weak.get(&class)?;
                Some(if w.contains(&"REAL") { "REAL" } else { w[0] }.to_string())
            })
    }

    /// Una clase (cables, variables y pines genéricos unidos) tiene que tener un solo
    /// tipo concreto. Entre elementales es un error seguro de compilación.
    fn report_conflicts(&self, diags: &mut Diagnostics) {
        let mut class_vars: BTreeMap<usize, Vec<&str>> = BTreeMap::new();
        for (i, name) in self.var_names.iter().enumerate() {
            class_vars.entry(self.var_class(i)).or_default().push(name);
        }
        for (class, strongs) in &self.strong {
            let first = &strongs[0];
            if let Some(other) = strongs.iter().find(|s| !compatible(&s.ty, &first.ty)) {
                let level = if is_elementary(&first.ty) && is_elementary(&other.ty) {
                    Level::Error
                } else {
                    Level::Warning
                };
                let subject = class_vars
                    .get(class)
                    .map(|v| format!("{}: ", v.join(", ")))
                    .unwrap_or_default();
                diags.push(
                    level,
                    STAGE,
                    format!(
                        "{subject}tipos incompatibles {} ({}) y {} ({})",
                        first.ty, first.origin, other.ty, other.origin
                    ),
                );
            }
        }
    }
}

/// Cada pin genérico acepta solo tipos de su familia (`ANY_NUM`, `ANY_BIT`...). Valida
/// también clases sin variables (por ejemplo, un literal en un pin) y todos los tipos
/// fuertes: uno válido no debe ocultar otro inválido.
fn check_generic_pins(section: &Section, classes: &Classes, diags: &mut Diagnostics) {
    for (bi, b) in section.blocks.iter().enumerate() {
        for (pi, p) in b.pins.iter().enumerate().filter(|(_, p)| is_generic(&p.ty)) {
            let r = PinRef { block: bi, pin: pi };
            let class = classes.class_of[classes.pin_node[&r]];
            let candidates: Vec<String> = match classes.strong.get(&class) {
                Some(strongs) => strongs.iter().map(|s| s.ty.clone()).collect(),
                None => classes.type_of(class).into_iter().collect(),
            };
            if let Some(ty) = candidates.iter().find(|ty| !accepts_generic(&p.ty, ty)) {
                diags.push(
                    Level::Error,
                    STAGE,
                    format!(
                        "{}: el tipo {ty} no pertenece a {}",
                        section.pin_path(r),
                        p.ty
                    ),
                );
            }
        }
    }
}

/// Variables a declarar, con su tipo: el declarado o el de su clase. Una dirección
/// topológica no se declara, y un acceso (`Motor.Run`) declara su raíz.
fn typed_vars(
    section: &Section,
    classes: &Classes,
    decl_by_name: &HashMap<String, &VarSpec>,
    diags: &mut Diagnostics,
) -> Vec<TypedVar> {
    let mut result = Vec::new();
    let mut accessed: Vec<(&str, &str)> = Vec::new();
    for (i, name) in classes.var_names.iter().enumerate() {
        match var_root(name) {
            None => continue,
            Some(root) if root.len() != name.len() => {
                accessed.push((name, root));
                continue;
            }
            Some(_) => {}
        }
        let decl = decl_by_name.get(&name.to_ascii_uppercase()).copied();
        let ty = decl
            .map(|d| d.ty.clone())
            .or_else(|| classes.type_of(classes.var_class(i)));
        match ty {
            Some(ty) => result.push(TypedVar {
                name: name.clone(),
                ty,
                decl: decl.cloned(),
            }),
            None => diags.push(
                Level::Error,
                STAGE,
                format!("{name}: no se pudo deducir el tipo; declarala en \"variables\""),
            ),
        }
    }

    // La raíz de un acceso es una variable declarada (su tipo, estructura o array, no se
    // puede deducir de un campo) o una instancia de bloque, que ya se declara sola.
    let instances: HashSet<String> = section
        .blocks
        .iter()
        .filter_map(|b| b.instance.as_ref().map(|i| i.to_ascii_uppercase()))
        .collect();
    for (name, root) in accessed {
        let key = root.to_ascii_uppercase();
        if instances.contains(&key) || result.iter().any(|v| v.name.eq_ignore_ascii_case(root)) {
            continue;
        }
        match decl_by_name.get(&key) {
            Some(d) => result.push(TypedVar {
                name: d.name.clone(),
                ty: d.ty.clone(),
                decl: Some((*d).clone()),
            }),
            None => diags.push(
                Level::Error,
                STAGE,
                format!(
                    "{name}: {root} tiene que estar declarada en \"variables\" o ser una instancia"
                ),
            ),
        }
    }
    result
}

/// Variables que declara una sección ya normalizada, en orden de primera aparición (el
/// orden del `dataBlock`). El tipo y la declaración salen de `known` (las variables del
/// programa y las de enlace entre secciones) o de las variables que creó la
/// normalización; el nombre se escribe como aparece en la sección.
pub(crate) fn section_vars(
    section: &Section,
    known: &HashMap<String, TypedVar>,
    diags: &mut Diagnostics,
) -> Vec<TypedVar> {
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    let params = section
        .blocks
        .iter()
        .flat_map(|b| &b.pins)
        .flat_map(|p| [&p.param_in, &p.param_out])
        .flatten();
    for param in params {
        let Param::Var(name) = param else { continue };
        // Direcciones: no se declaran. Accesos: se declara la raíz, si no es una instancia
        // (la inferencia ya validó que sea una de las dos).
        let Some(root) = var_root(name) else { continue };
        if root.len() != name.len() {
            if let Some(t) = known.get(&root.to_ascii_uppercase())
                && seen.insert(root.to_ascii_uppercase())
            {
                result.push(t.clone());
            }
            continue;
        }
        let key = name.to_ascii_uppercase();
        if !seen.insert(key.clone()) {
            continue;
        }
        let typed = known
            .get(&key)
            .map(|t| (t.ty.clone(), t.decl.clone()))
            .or_else(|| {
                section
                    .generated_vars
                    .iter()
                    .find(|g| g.name.eq_ignore_ascii_case(name) && !is_generic(&g.ty))
                    .map(|g| (g.ty.clone(), None))
            });
        match typed {
            Some((ty, decl)) => result.push(TypedVar {
                name: name.clone(),
                ty,
                decl,
            }),
            None => diags.push(
                Level::Error,
                STAGE,
                format!("{name}: no se pudo deducir el tipo; declarala en \"variables\""),
            ),
        }
    }
    result
}
