//! Reglas léxicas de IEC 61131-3 con la configuración por defecto de Control Expert:
//! identificadores, referencias a variables, literales y nombres generados.

use std::collections::HashSet;

/// Identificador IEC sin caracteres extendidos: letra o `_` seguida de letras, dígitos o
/// `_`.
pub fn is_identifier(s: &str) -> bool {
    let mut chars = s.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Variable que se puede escribir como parámetro actual: un identificador con campos
/// (`Motor.Run`) o índices (`Tab[3]`) opcionales, o una dirección topológica (`%MW100`).
pub fn is_variable_ref(s: &str) -> bool {
    if let Some(address) = s.strip_prefix('%') {
        return address.starts_with(|c: char| c.is_ascii_alphabetic())
            && address
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '.');
    }
    s.split('.').all(|part| {
        let (head, index) = part.split_at(part.find('[').unwrap_or(part.len()));
        is_identifier(head)
            && (index.is_empty()
                || index.starts_with('[')
                    && index.ends_with(']')
                    && index.len() > 2
                    && index
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || "_,[]".contains(c)))
    })
}

/// Variable que hay que declarar para un parámetro actual: el propio nombre si es simple,
/// la raíz de un acceso (`Motor` en `Motor.Run`, `Tab` en `Tab[3]`) o `None` para una
/// dirección topológica (`%MW100`), que no se declara.
pub fn var_root(name: &str) -> Option<&str> {
    if name.starts_with('%') {
        return None;
    }
    Some(&name[..name.find(['.', '[']).unwrap_or(name.len())])
}

/// Literales: booleanos, numéricos, tipados (`T#5s`, `16#FF`, `INT#3`) y strings.
pub fn is_literal(s: &str) -> bool {
    let s = s.trim();
    if s.eq_ignore_ascii_case("TRUE") || s.eq_ignore_ascii_case("FALSE") {
        return true;
    }
    if s.starts_with('\'') || s.starts_with('"') {
        return true;
    }
    if let Some((prefix, _)) = s.split_once('#') {
        return !prefix.is_empty()
            && prefix
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_');
    }
    let body = s.strip_prefix(['-', '+']).unwrap_or(s);
    body.starts_with(|c: char| c.is_ascii_digit())
        && body
            .chars()
            .all(|c| c.is_ascii_digit() || matches!(c, '_' | '.' | 'e' | 'E' | '-' | '+'))
}

/// Tareas de Control Expert que admiten secciones FBD: la maestra, la rápida, las
/// auxiliares y la de seguridad (M580 Safety). Las secciones de evento no se importan
/// con una tarea.
const TASKS: [&str; 7] = ["MAST", "FAST", "AUX0", "AUX1", "AUX2", "AUX3", "SAFE"];

/// Nombre canónico (en mayúsculas) de una tarea, sin distinguir mayúsculas en la entrada.
pub fn task_name(task: &str) -> Option<&'static str> {
    TASKS.into_iter().find(|t| t.eq_ignore_ascii_case(task))
}

/// Dirección topológica (`%MW100`, `%I0.1.2`).
pub fn is_address(s: &str) -> bool {
    s.starts_with('%') && is_variable_ref(s)
}

/// Identificador válido a partir de un texto arbitrario.
pub fn sanitize(s: &str) -> String {
    let mut out: String = s
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    if out.is_empty() || out.starts_with(|c: char| c.is_ascii_digit()) {
        out.insert(0, 'V');
    }
    out
}

/// `base`, o `base_2`, `base_3`..., el primero que no esté en `used` (que guarda nombres
/// en mayúsculas, porque IEC no las distingue). Lo reserva en `used`.
pub fn unique_name(base: &str, used: &mut HashSet<String>) -> String {
    let mut name = base.to_string();
    let mut n = 2;
    while used.contains(&name.to_ascii_uppercase()) {
        name = format!("{base}_{n}");
        n += 1;
    }
    used.insert(name.to_ascii_uppercase());
    name
}
