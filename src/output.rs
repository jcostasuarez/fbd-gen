//! Salidas: JSON con la sección posicionada (base para el export a XEF) y una vista
//! previa SVG para revisar el resultado sin abrir Control Expert.

use std::fmt::Write as _;

use serde::Serialize;

use crate::layout::{Layout, coord};
use crate::model::{Dir, GeneratedVar, Param, Section};
use crate::xml::escape as esc;

/// Sección posicionada: lo que contiene `.layout.json`, en celdas de grilla.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OutSection {
    /// Nombre de la sección.
    pub name: String,
    /// Ancho de la hoja, margen incluido.
    pub width: i32,
    /// Alto de la hoja, margen incluido.
    pub height: i32,
    /// Bloques en el orden del modelo.
    pub blocks: Vec<OutBlock>,
    /// Cables con su recorrido.
    pub links: Vec<OutLink>,
    /// Comentarios de red.
    pub text_boxes: Vec<OutText>,
    /// Variables que reemplazan cables.
    pub generated_variables: Vec<GeneratedVar>,
}

/// Bloque posicionado.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OutBlock {
    /// Identificador del bloque en el grafo de entrada.
    pub id: String,
    /// Tipo de bloque.
    #[serde(rename = "type")]
    pub ty: String,
    /// Nombre de la instancia, si es EFB/DFB.
    pub instance: Option<String>,
    /// Índice de la red (componente conexa) a la que pertenece.
    pub network: usize,
    /// Columna de la esquina superior izquierda (`posX` en Control Expert).
    pub x: i32,
    /// Fila de la esquina superior izquierda (`posY` en Control Expert).
    pub y: i32,
    /// Ancho en celdas.
    pub width: u32,
    /// Alto en celdas.
    pub height: u32,
    /// Pines con su fila y los parámetros escritos junto a ellos.
    pub pins: Vec<OutPin>,
}

/// Pin posicionado.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OutPin {
    /// Nombre del pin.
    pub name: String,
    /// Dirección.
    pub dir: Dir,
    /// Fila relativa a la esquina superior del bloque.
    pub row: u32,
    /// Variable o literal escrito del lado izquierdo.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub param_in: Option<Param>,
    /// Variable escrita del lado derecho.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub param_out: Option<Param>,
}

/// Comentario de red (textBox).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OutText {
    /// Texto del comentario.
    pub text: String,
    /// Columna de la esquina superior izquierda.
    pub x: i32,
    /// Fila de la esquina superior izquierda.
    pub y: i32,
    /// Ancho en celdas.
    pub width: i32,
    /// Alto en celdas.
    pub height: i32,
}

/// Cable entre dos pines.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OutLink {
    /// Pin de origen, como `bloque.PIN`.
    pub from: String,
    /// Pin de destino, como `bloque.PIN`.
    pub to: String,
    /// Polilínea ortogonal `[x, y]`, del pin de origen al de destino.
    pub points: Vec<[i32; 2]>,
}

pub(crate) fn to_output(section: &Section, nets: &[Vec<usize>], layout: &Layout) -> OutSection {
    let mut network_of = vec![0; section.blocks.len()];
    for (n, members) in nets.iter().enumerate() {
        for &b in members {
            network_of[b] = n;
        }
    }
    OutSection {
        name: section.name.clone(),
        width: layout.width,
        height: layout.height,
        blocks: section
            .blocks
            .iter()
            .enumerate()
            .map(|(i, b)| OutBlock {
                id: b.id.clone(),
                ty: b.ty.clone(),
                instance: b.instance.clone(),
                network: network_of[i],
                x: layout.blocks[i].x,
                y: layout.blocks[i].y,
                width: b.width,
                height: b.height,
                pins: b
                    .pins
                    .iter()
                    .map(|p| OutPin {
                        name: p.name.clone(),
                        dir: p.dir,
                        row: p.row,
                        param_in: p.param_in.clone(),
                        param_out: p.param_out.clone(),
                    })
                    .collect(),
            })
            .collect(),
        links: section
            .wires
            .iter()
            .zip(&layout.wires)
            .map(|(w, pts)| OutLink {
                from: section.pin_path(w.from),
                to: section.pin_path(w.to),
                points: pts.iter().map(|p| [p.x, p.y]).collect(),
            })
            .collect(),
        text_boxes: layout
            .texts
            .iter()
            .map(|t| OutText {
                text: t.text.clone(),
                x: t.x,
                y: t.y,
                width: t.width,
                height: t.height,
            })
            .collect(),
        generated_variables: section.generated_vars.clone(),
    }
}

const C: i32 = 12; // píxeles por celda en la vista previa

pub(crate) fn to_svg(section: &Section, layout: &Layout) -> String {
    let margin = 2 * C;
    let (w, h) = (
        layout.width * C + 2 * margin,
        layout.height * C + 2 * margin,
    );
    let mut s = String::new();
    let _ = write!(
        s,
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="{vx} {vy} {w} {h}" font-family="Consolas, monospace" font-size="9">
<defs><pattern id="grid" width="{C}" height="{C}" patternUnits="userSpaceOnUse"><circle cx="0" cy="0" r="0.8" fill="#c8ccd2"/></pattern></defs>
<rect x="{vx}" y="{vy}" width="{w}" height="{h}" fill="#ffffff"/>
<rect x="{vx}" y="{vy}" width="{w}" height="{h}" fill="url(#grid)"/>
"##,
        vx = -margin,
        vy = -margin,
    );

    for (i, (y, nh)) in layout.networks.iter().enumerate() {
        let _ = writeln!(
            s,
            r##"<rect x="{x}" y="{yy}" width="{ww}" height="{hh}" fill="none" stroke="#e3e6ea" stroke-dasharray="4 3"/><text x="{x}" y="{ty}" fill="#9aa1aa">red {i}</text>"##,
            x = -C,
            yy = y * C - C / 2,
            ww = (layout.width + 2) * C,
            hh = nh * C + C / 2,
            ty = y * C - C / 2 - 2,
        );
    }

    for t in &layout.texts {
        let _ = writeln!(
            s,
            r##"<rect x="{}" y="{}" width="{}" height="{}" fill="#fff8c5" stroke="#d4a72c"/>"##,
            t.x * C,
            t.y * C,
            t.width * C,
            t.height * C
        );
        // Ajuste de línea aproximado, como lo hace el editor dentro de la caja.
        let per_line = usize::try_from((t.width * C - 8) / 6).map_or(1, |n| n.max(1));
        let chars: Vec<char> = t.text.chars().collect();
        let baselines = (t.y * C + 11..).step_by(11);
        for (line, y) in chars.chunks(per_line).zip(baselines) {
            let line: String = line.iter().collect();
            let _ = writeln!(
                s,
                r##"<text x="{}" y="{y}" fill="#7d4e00">{}</text>"##,
                t.x * C + 4,
                esc(&line)
            );
        }
    }

    for pts in &layout.wires {
        let p: Vec<String> = pts
            .iter()
            .map(|p| format!("{},{}", p.x * C, p.y * C))
            .collect();
        let _ = writeln!(
            s,
            r##"<polyline points="{}" fill="none" stroke="#1f6feb" stroke-width="1.2"/>"##,
            p.join(" ")
        );
    }

    for (b, pos) in section.blocks.iter().zip(&layout.blocks) {
        let (x0, x1) = (pos.x * C, (pos.x + coord(b.width) - 1) * C);
        let (y0, y1) = ((pos.y + 2) * C, (pos.y + coord(b.height)) * C);
        let _ = writeln!(
            s,
            r##"<rect x="{x0}" y="{y0}" width="{}" height="{}" fill="#f6f8fa" stroke="#24292f"/>"##,
            x1 - x0,
            y1 - y0
        );
        let cx = i32::midpoint(x0, x1);
        let _ = writeln!(
            s,
            r#"<text x="{cx}" y="{}" text-anchor="middle" font-weight="bold">{}</text>"#,
            y0 + C - 2,
            esc(&b.ty)
        );
        if let Some(inst) = &b.instance {
            let _ = writeln!(
                s,
                r##"<text x="{cx}" y="{}" text-anchor="middle" fill="#57606a">{}</text>"##,
                y0 - 3,
                esc(inst)
            );
        }
        for p in &b.pins {
            let py = (pos.y + coord(p.row)) * C;
            if p.dir.has_left() {
                let _ = writeln!(
                    s,
                    r#"<text x="{}" y="{}">{}</text>"#,
                    x0 + 3,
                    py + 3,
                    esc(&p.name)
                );
            }
            if p.dir.has_right() {
                let _ = writeln!(
                    s,
                    r#"<text x="{}" y="{}" text-anchor="end">{}</text>"#,
                    x1 - 3,
                    py + 3,
                    esc(&p.name)
                );
            }
            let color = |param: &Param| {
                if matches!(param, Param::Lit(_)) {
                    "#953800"
                } else {
                    "#116329"
                }
            };
            if let Some(param) = &p.param_in {
                let _ = writeln!(
                    s,
                    r##"<line x1="{}" y1="{py}" x2="{x0}" y2="{py}" stroke="#8c959f"/><text x="{}" y="{}" text-anchor="end" fill="{}">{}</text>"##,
                    x0 - C / 2,
                    x0 - C / 2 - 2,
                    py + 3,
                    color(param),
                    esc(param.text())
                );
            }
            if let Some(param) = &p.param_out {
                let _ = writeln!(
                    s,
                    r##"<line x1="{x1}" y1="{py}" x2="{}" y2="{py}" stroke="#8c959f"/><text x="{}" y="{}" fill="{}">{}</text>"##,
                    x1 + C / 2,
                    x1 + C / 2 + 2,
                    py + 3,
                    color(param),
                    esc(param.text())
                );
            }
        }
    }
    s.push_str("</svg>\n");
    s
}
