//! Reglas de XML 1.0 compartidas por el export (`xef`) y la vista previa (`output`).

/// Caracter que XML 1.0 admite en un documento (`Char` de la especificación): tabulación,
/// saltos de línea y todo lo imprimible salvo U+FFFE y U+FFFF. Los demás caracteres de
/// control no se pueden escribir ni escapados, así que un texto que los tenga se rechaza
/// al validar la entrada.
pub(crate) fn is_char(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\r') || (c >= ' ' && c != '\u{FFFE}' && c != '\u{FFFF}')
}

/// Primer caracter que no puede ir en un documento XML, si hay alguno.
pub(crate) fn invalid_char(s: &str) -> Option<char> {
    s.chars().find(|&c| !is_char(c))
}

/// Escapa un texto para un atributo o un contenido de elemento.
pub(crate) fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_characters_are_not_xml_text() {
        assert_eq!(invalid_char("Arranque\tmotor\r\nP101 ñ °C"), None);
        for bad in [
            "a\u{0}b",
            "a\u{1}b",
            "a\u{7}b",
            "a\u{1B}b",
            "a\u{FFFE}b",
            "a\u{FFFF}b",
        ] {
            assert!(invalid_char(bad).is_some(), "{bad:?}");
        }
    }

    #[test]
    fn escape_covers_markup_and_quotes() {
        assert_eq!(
            escape(r#"<a & 'b' "c">"#),
            "&lt;a &amp; &apos;b&apos; &quot;c&quot;&gt;"
        );
    }
}
