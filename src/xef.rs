//! Export de la sección posicionada al XML de intercambio FBD de Control Expert
//! (`FBDExchangeFile`, lo que importa `import_xml(kind="section")`).
//!
//! Estructura según `SrcXmlSchema/FBDSource.xsd` de Control Expert 15.1 y sus exports:
//! - todas las coordenadas van en celdas de grilla;
//! - `descriptionFFB` lista EN, entradas, ENO, salidas (aunque `enEnO="false"`);
//! - un link lleva sus extremos sobre los pines y después los quiebres (`gridObjPosition`);
//! - `nbRows`/`nbColumns` se dejan en 24/36: el contenido puede excederlos, pero otros
//!   valores hacen fallar la importación.

use std::fmt::Write as _;

use crate::layout::Layout;
use crate::model::*;
use crate::types::TypedVar;

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// Nombre con el que el XML identifica cada bloque: la instancia de EFB/DFB o `.N` para EF.
pub fn object_names(section: &Section) -> Vec<String> {
    let mut ef = 0;
    section
        .blocks
        .iter()
        .map(|b| match &b.instance {
            Some(i) => i.clone(),
            None => {
                ef += 1;
                format!(".{ef}")
            }
        })
        .collect()
}

pub fn to_exchange_xml(
    section: &Section,
    layout: &Layout,
    task: &str,
    vars: &[TypedVar],
) -> String {
    let names = object_names(section);
    let mut x = String::new();
    x.push_str(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<FBDExchangeFile>\n",
    );
    x.push_str("\t<fileHeader company=\"Schneider Automation\" product=\"Control Expert V15.1\" dateTime=\"date_and_time#2026-1-1-0:0:0\" content=\"Derived Function Block source file\" DTDVersion=\"41\"></fileHeader>\n");
    x.push_str("\t<contentHeader name=\"Project\" version=\"0.0.000\"></contentHeader>\n");
    x.push_str("\t<program>\n");
    let _ = writeln!(
        x,
        "\t\t<identProgram name=\"{}\" type=\"section\" task=\"{}\"></identProgram>",
        esc(&section.name),
        esc(task)
    );
    x.push_str("\t\t<FBDSource nbRows=\"24\" nbColumns=\"36\">\n\t\t\t<networkFBD>\n");
    // Igual que los exports de Control Expert: textBox en celdas, el texto se ajusta solo.
    for t in &layout.texts {
        let _ = writeln!(
            x,
            "\t\t\t\t<textBox width=\"{}\" height=\"{}\">{}<objPosition posX=\"{}\" posY=\"{}\"></objPosition></textBox>",
            t.width,
            t.height,
            esc(&t.text),
            t.x,
            t.y
        );
    }

    for (bi, b) in section.blocks.iter().enumerate() {
        let pos = layout.blocks[bi];
        let _ = writeln!(
            x,
            "\t\t\t\t<FFBBlock instanceName=\"{}\" typeName=\"{}\" additionnalPinNumber=\"{}\" enEnO=\"false\" width=\"{}\" height=\"{}\">",
            esc(&names[bi]),
            esc(&b.ty),
            b.extra_pins,
            b.width,
            b.height
        );
        let _ = writeln!(
            x,
            "\t\t\t\t\t<objPosition posX=\"{}\" posY=\"{}\"></objPosition>",
            pos.x, pos.y
        );
        x.push_str("\t\t\t\t\t<descriptionFFB execAfter=\"\">\n");
        let pin = |x: &mut String, tag: &str, name: &str, param: Option<&Param>| {
            let eff = param
                .map(|p| format!(" effectiveParameter=\"{}\"", esc(p.text())))
                .unwrap_or_default();
            let _ = writeln!(
                x,
                "\t\t\t\t\t\t<{tag} invertedPin=\"false\" formalParameter=\"{}\"{eff}></{tag}>",
                esc(name)
            );
        };
        let mut by_row: Vec<&Pin> = b.pins.iter().collect();
        by_row.sort_by_key(|p| p.row);
        pin(&mut x, "inputVariable", "EN", None);
        for p in by_row.iter().filter(|p| p.dir.has_left()) {
            pin(&mut x, "inputVariable", &p.name, p.param_in.as_ref());
        }
        // Control Expert reexporta los IN_OUT antes que las salidas (la importación acepta
        // cualquier orden, pero así el round-trip queda idéntico).
        pin(&mut x, "outputVariable", "ENO", None);
        for dir in [Dir::InOut, Dir::Out] {
            for p in by_row.iter().filter(|p| p.dir == dir) {
                pin(&mut x, "outputVariable", &p.name, p.param_out.as_ref());
            }
        }
        x.push_str("\t\t\t\t\t</descriptionFFB>\n");
        if let Some(c) = &b.comment {
            let _ = writeln!(x, "\t\t\t\t\t<comment>{}</comment>", esc(c));
        }
        x.push_str("\t\t\t\t</FFBBlock>\n");
    }

    for (w, pts) in section.wires.iter().zip(&layout.wires) {
        let (start, end) = (pts[0], pts[pts.len() - 1]);
        x.push_str("\t\t\t\t<linkFB>\n");
        for (tag, r, p) in [
            ("linkSource", w.from, start),
            ("linkDestination", w.to, end),
        ] {
            let _ = writeln!(
                x,
                "\t\t\t\t\t<{tag} parentObjectName=\"{}\" pinName=\"{}\">\n\t\t\t\t\t\t<objPosition posX=\"{}\" posY=\"{}\"></objPosition>\n\t\t\t\t\t</{tag}>",
                esc(&names[r.block]),
                esc(&section.pin(r).name),
                p.x,
                p.y
            );
        }
        for p in &pts[1..pts.len() - 1] {
            let _ = writeln!(
                x,
                "\t\t\t\t\t<gridObjPosition posX=\"{}\" posY=\"{}\"></gridObjPosition>",
                p.x, p.y
            );
        }
        x.push_str("\t\t\t\t</linkFB>\n");
    }
    x.push_str("\t\t\t</networkFBD>\n\t\t</FBDSource>\n\t</program>\n\t<dataBlock>\n");

    for v in vars {
        let addr = v
            .decl
            .as_ref()
            .and_then(|d| d.address.as_ref())
            .map(|a| format!(" topologicalAddress=\"{}\"", esc(a)))
            .unwrap_or_default();
        let _ = write!(
            x,
            "\t\t<variables name=\"{}\" typeName=\"{}\"{addr}>",
            esc(&v.name),
            esc(&v.ty)
        );
        if let Some(d) = &v.decl {
            if let Some(c) = &d.comment {
                let _ = write!(x, "<comment>{}</comment>", esc(c));
            }
            if let Some(i) = &d.init {
                let _ = write!(x, "<variableInit value=\"{}\"></variableInit>", esc(i));
            }
        }
        x.push_str("</variables>\n");
    }
    for b in &section.blocks {
        if let Some(inst) = &b.instance {
            let _ = writeln!(
                x,
                "\t\t<variables name=\"{}\" typeName=\"{}\"></variables>",
                esc(inst),
                esc(&b.ty)
            );
        }
    }
    x.push_str("\t</dataBlock>\n</FBDExchangeFile>\n");
    x
}
