//! Inferencia de tipos de las variables para poder declararlas en el `dataBlock`.
//!
//! Unificación simple: dentro de una EF, los pines con el mismo tipo genérico (`ANY_BIT`,
//! `ANY_NUM`...) comparten tipo; un cable une sus dos pines; una variable une todos los
//! pines donde aparece. Los tipos concretos (pines, declaraciones) son fuertes; los
//! literales sin prefijo solo sugieren (`85.0` -> REAL); los tipados fijan su tipo.

use std::collections::{BTreeMap, HashMap};

use crate::library::Kind;
use crate::model::*;

const STAGE: &str = "types";

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

/// EBOOL se conecta donde se espera BOOL; no es un conflicto real.
fn compatible(a: &str, b: &str) -> bool {
    let norm = |t: &str| {
        if t.eq_ignore_ascii_case("EBOOL") {
            "BOOL".to_string()
        } else {
            t.to_ascii_uppercase()
        }
    };
    norm(a) == norm(b)
}

struct UnionFind(Vec<usize>);

impl UnionFind {
    fn find(&mut self, x: usize) -> usize {
        let mut r = x;
        while self.0[r] != r {
            r = self.0[r];
        }
        let mut c = x;
        while self.0[c] != r {
            let n = self.0[c];
            self.0[c] = r;
            c = n;
        }
        r
    }
    fn union(&mut self, a: usize, b: usize) {
        let (a, b) = (self.find(a), self.find(b));
        if a != b {
            self.0[a.max(b)] = a.min(b);
        }
    }
}

/// Una variable de la sección con su tipo final, en orden de primera aparición.
#[derive(Debug, Clone)]
pub struct TypedVar {
    pub name: String,
    pub ty: String,
    pub decl: Option<VarSpec>,
}

pub fn infer(
    section: &mut Section,
    declared: &[VarSpec],
    diags: &mut Diagnostics,
) -> Vec<TypedVar> {
    infer_all(section, declared, diags).0
}

/// Como [`infer`], y además el tipo resuelto de cada pin (si se pudo deducir).
pub fn infer_all(
    section: &mut Section,
    declared: &[VarSpec],
    diags: &mut Diagnostics,
) -> (Vec<TypedVar>, HashMap<PinRef, String>) {
    // Nodos: un nodo por pin, después uno por variable.
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
    let mut strong: Vec<(usize, String)> = Vec::new();
    let mut weak: Vec<(usize, &'static str)> = Vec::new();

    for (bi, b) in section.blocks.iter().enumerate() {
        let mut generic_groups: HashMap<String, usize> = HashMap::new();
        for (pi, p) in b.pins.iter().enumerate() {
            let n = pin_node[&PinRef { block: bi, pin: pi }];
            if is_generic(&p.ty) {
                if b.kind == Kind::Ef {
                    let first = *generic_groups.entry(p.ty.to_ascii_uppercase()).or_insert(n);
                    links.push((first, n));
                }
            } else {
                strong.push((n, p.ty.clone()));
            }
            for param in [&p.param_in, &p.param_out].into_iter().flatten() {
                match param {
                    Param::Var(v) => links.push((n, var_of(v, &mut var_names))),
                    Param::Lit(l) => {
                        if let Some(ty) = typed_literal_type(l) {
                            strong.push((n, ty.to_string()));
                        } else {
                            weak.extend(literal_hint(l).map(|h| (n, h)));
                        }
                    }
                }
            }
        }
    }
    for w in &section.wires {
        links.push((pin_node[&w.from], pin_node[&w.to]));
    }
    let mut decl_by_name: HashMap<String, &VarSpec> = HashMap::new();
    for d in declared {
        decl_by_name.insert(d.name.to_ascii_uppercase(), d);
        if let Some(&n) = var_node.get(&d.name.to_ascii_uppercase()) {
            strong.push((n, d.ty.clone()));
        }
    }

    let mut uf = UnionFind((0..pin_node.len() + var_names.len()).collect());
    for (a, b) in links {
        uf.union(a, b);
    }
    let mut class_strong: BTreeMap<usize, Vec<String>> = BTreeMap::new();
    for (n, t) in strong {
        class_strong.entry(uf.find(n)).or_default().push(t);
    }
    let mut class_weak: BTreeMap<usize, Vec<&str>> = BTreeMap::new();
    for (n, t) in weak {
        class_weak.entry(uf.find(n)).or_default().push(t);
    }

    let class_type = |class: usize| -> Option<String> {
        class_strong
            .get(&class)
            .and_then(|s| s.first().cloned())
            .or_else(|| {
                let w = class_weak.get(&class)?;
                Some(if w.contains(&"REAL") { "REAL" } else { w[0] }.to_string())
            })
    };
    let pin_types: HashMap<PinRef, String> = pin_node
        .iter()
        .filter_map(|(&r, &n)| class_type(uf.find(n)).map(|t| (r, t)))
        .collect();

    // Validar también clases sin variables (por ejemplo, un literal en un pin).
    // Se comprueban todos los tipos fuertes: uno válido no debe ocultar otro inválido.
    for (bi, b) in section.blocks.iter().enumerate() {
        for (pi, p) in b.pins.iter().enumerate().filter(|(_, p)| is_generic(&p.ty)) {
            let r = PinRef { block: bi, pin: pi };
            let class = uf.find(pin_node[&r]);
            let candidates = class_strong
                .get(&class)
                .cloned()
                .unwrap_or_else(|| class_type(class).into_iter().collect());
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

    let mut result = Vec::new();
    for (i, name) in var_names.iter().enumerate() {
        let class = uf.find(pin_node.len() + i);
        let decl = decl_by_name.get(&name.to_ascii_uppercase()).copied();
        let strongs = class_strong.get(&class).map(Vec::as_slice).unwrap_or(&[]);
        if let Some(conflict) = strongs.iter().find(|t| !compatible(t, &strongs[0])) {
            diags.push(
                Level::Warning,
                STAGE,
                format!("{name}: tipos en conflicto {} y {conflict}", strongs[0]),
            );
        }
        let ty = decl.map(|d| d.ty.clone()).or_else(|| class_type(class));
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

    for g in &mut section.generated_vars {
        if let Some(v) = result.iter().find(|v| v.name.eq_ignore_ascii_case(&g.name)) {
            g.ty = v.ty.clone();
        }
    }
    (result, pin_types)
}
