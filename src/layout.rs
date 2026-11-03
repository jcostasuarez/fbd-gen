//! Etapas 8 y 9: Sugiyama por red (`sugiyama`, en celdas enteras), permutación de
//! entradas conmutativas (`commutative_swaps` y `apply_swaps`, entre dos pasadas), comentarios de red y apilado de redes en la hoja.
//!
//! Antes de exportar se valida la grilla: aunque el motor ya la garantiza, un XML con
//! bloques superpuestos o cables que se tocan no debe salir nunca.

use std::collections::HashMap;
use std::error::Error;
use std::fmt;

use crate::model::{Block, Diagnostics, Level, Param, Pin, PinRef, Section, Stage};
pub(crate) use crate::sugiyama::Pos;
use crate::sugiyama::{self, MAX_EXTENT};

const STAGE: Stage = Stage::Layout;

/// Por qué no se pudo dibujar una sección: el motor rechazó una red o el dibujo no pasó
/// la validación de la grilla.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum LayoutError {
    /// El motor Sugiyama no pudo dibujar una red (ciclo, puertos inválidos, tamaño).
    Engine(sugiyama::LayoutError),
    /// La sección excede [`MAX_EXTENT`] o el dibujo viola un invariante de la grilla
    /// (bloques superpuestos, cables que se tocan). Esto último indicaría un bug del motor.
    Grid(String),
}

impl fmt::Display for LayoutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Engine(e) => write!(f, "{e}"),
            Self::Grid(message) => write!(f, "{message}"),
        }
    }
}

impl Error for LayoutError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Engine(e) => Some(e),
            Self::Grid(_) => None,
        }
    }
}

impl From<sugiyama::LayoutError> for LayoutError {
    fn from(e: sugiyama::LayoutError) -> Self {
        Self::Engine(e)
    }
}

type Result<T> = std::result::Result<T, LayoutError>;

macro_rules! bail {
    ($($arg:tt)*) => {
        return Err(LayoutError::Grid(format!($($arg)*)))
    };
}

/// Medidas del dibujo, en celdas de grilla.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayoutConfig {
    /// Celdas de ancho cada 10 caracteres de un parámetro actual (variable o literal).
    /// Calibrar contra Control Expert.
    pub label_cells_per_10_chars: u32,
    /// Filas libres entre dos bloques de una misma capa.
    pub node_gap: u32,
    /// Ancho mínimo del canal de cables entre dos capas.
    pub layer_gap: u32,
    /// Filas libres entre dos redes.
    pub network_gap: u32,
    /// Margen libre contra el marco de la hoja.
    pub frame_margin: u32,
    /// Caracteres cada 10 celdas de ancho en un textBox. Los demos de Schneider dan
    /// ~12-14; se usa el valor bajo para no quedarse cortos de alto.
    pub text_chars_per_10_cells: u32,
}

impl Default for LayoutConfig {
    fn default() -> Self {
        Self {
            label_cells_per_10_chars: 8,
            node_gap: 2,
            layer_gap: 3,
            network_gap: 3,
            frame_margin: 2,
            text_chars_per_10_cells: 12,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Layout {
    /// Esquina (posX, posY) de cada bloque, en celdas, indexado como `section.blocks`.
    pub blocks: Vec<Pos>,
    /// Polilínea ortogonal de cada cable, indexada como `section.wires`.
    pub wires: Vec<Vec<Pos>>,
    /// Rango vertical [y, y + height) de cada red, incluido su comentario.
    pub networks: Vec<(i32, i32)>,
    /// Comentarios de red, en celdas (como los guarda Control Expert).
    pub texts: Vec<TextBox>,
    pub width: i32,
    pub height: i32,
}

#[derive(Debug, Clone)]
pub(crate) struct TextBox {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub text: String,
}

/// Margen horizontal que ocupan los parámetros actuales a cada lado del bloque.
struct Margins {
    left: i32,
    right: i32,
}

impl Margins {
    fn new(b: &Block, cfg: &LayoutConfig) -> Self {
        let side = |f: fn(&Pin) -> &Option<Param>| {
            b.pins
                .iter()
                .filter_map(|p| f(p).as_ref())
                .map(|p| label_cells(p.text(), cfg))
                .max()
                .unwrap_or(0)
        };
        Self {
            left: side(|p| &p.param_in),
            right: side(|p| &p.param_out),
        }
    }

    /// Ancho de la caja del bloque en el layout: el bloque (pines de entrada en la columna 0 y de salida en la
    /// `width - 1`) más los parámetros escritos a cada lado.
    fn node_width(&self, b: &Block) -> i32 {
        self.left + coord(b.width) - 1 + self.right
    }
}

/// Medida de un bloque (ancho, alto, fila de pin) o de `LayoutConfig` en coordenadas de
/// grilla. Ambas llegan acotadas: la librería limita los bloques a `MAX_BLOCK_CELLS` y el
/// generador valida la configuración antes de dibujar.
pub(crate) fn coord(n: u32) -> i32 {
    i32::try_from(n).expect("medida acotada antes del layout")
}

/// Cuenta en `u64` llevada a `i32`, topeada justo arriba de `MAX_EXTENT`: un texto
/// gigante da una medida que el chequeo de extensión rechaza, nunca un desborde.
fn cells(n: u64) -> i32 {
    i32::try_from(n).unwrap_or(i32::MAX).min(MAX_EXTENT + 1)
}

/// Caracteres de un texto, en `u64` para las cuentas de celdas.
fn char_count(text: &str) -> u64 {
    u64::try_from(text.chars().count()).unwrap_or(u64::MAX)
}

fn label_cells(text: &str, cfg: &LayoutConfig) -> i32 {
    let scaled = char_count(text).saturating_mul(u64::from(cfg.label_cells_per_10_chars));
    cells(scaled.div_ceil(10) + 1)
}

/// Filas que ocupa un comentario de `width` celdas.
fn text_lines(text: &str, width: i32, cfg: &LayoutConfig) -> i32 {
    let per_10 = u64::from(width.max(1).unsigned_abs()) * u64::from(cfg.text_chars_per_10_cells);
    cells(char_count(text).saturating_mul(10).div_ceil(per_10).max(1))
}

/// Corta el apilado en cuanto la hoja pasa `MAX_EXTENT`. Cada red ya viene acotada por el
/// motor, así que las sumas hasta el próximo chequeo entran en `i32`.
fn check_extent(section: &Section, extent: i32) -> Result<()> {
    if extent > MAX_EXTENT {
        bail!(
            "{}: la sección excede {MAX_EXTENT} celdas por eje",
            section.name
        );
    }
    Ok(())
}

/// Borde del bloque donde se conecta un cable.
#[derive(Clone, Copy)]
enum Side {
    /// Columna de entradas: donde llega el cable.
    West,
    /// Columna de salidas: de donde sale el cable.
    East,
}

/// Celda del pin sobre el borde `side` del bloque.
fn pin_cell(section: &Section, layout: &Layout, r: PinRef, side: Side) -> Pos {
    let (b, pos) = (&section.blocks[r.block], layout.blocks[r.block]);
    Pos {
        x: match side {
            Side::West => pos.x,
            Side::East => pos.x + coord(b.width) - 1,
        },
        y: pos.y + coord(section.pin(r).row),
    }
}

fn run_once(section: &Section, nets: &[Vec<usize>], cfg: &LayoutConfig) -> Result<Layout> {
    let margins: Vec<Margins> = section
        .blocks
        .iter()
        .map(|b| Margins::new(b, cfg))
        .collect();
    let spacing = sugiyama::Spacing {
        node: coord(cfg.node_gap),
        layer: coord(cfg.layer_gap),
    };

    let mut layout = Layout {
        blocks: vec![Pos::default(); section.blocks.len()],
        wires: vec![Vec::new(); section.wires.len()],
        networks: Vec::new(),
        texts: Vec::new(),
        width: 0,
        height: 0,
    };
    let margin = coord(cfg.frame_margin);
    let mut cursor_y = margin;
    for members in nets {
        // Caja de cada bloque (con sus parámetros) y cables de la red, en índices locales.
        let local: HashMap<usize, usize> =
            members.iter().enumerate().map(|(i, &b)| (b, i)).collect();
        let boxes: Vec<sugiyama::Node> = members
            .iter()
            .map(|&bi| sugiyama::Node {
                width: margins[bi].node_width(&section.blocks[bi]),
                height: coord(section.blocks[bi].height),
            })
            .collect();
        let wires: Vec<usize> = (0..section.wires.len())
            .filter(|&wi| local.contains_key(&section.wires[wi].from.block))
            .collect();
        let edges: Vec<sugiyama::Edge> = wires
            .iter()
            .map(|&wi| {
                let w = section.wires[wi];
                sugiyama::Edge {
                    from: local[&w.from.block],
                    from_row: coord(section.pin(w.from).row),
                    to: local[&w.to.block],
                    to_row: coord(section.pin(w.to).row),
                }
            })
            .collect();
        let placed = sugiyama::layout(&boxes, &edges, &spacing)?;
        let nodes: HashMap<usize, Pos> = members
            .iter()
            .zip(&placed.nodes)
            .map(|(&bi, &p)| (bi, p))
            .collect();
        let bends: HashMap<usize, Vec<Pos>> = wires.into_iter().zip(placed.bends).collect();

        let (min_x, min_y) = members
            .iter()
            .map(|bi| nodes[bi])
            .chain(bends.values().flatten().copied())
            .fold((i32::MAX, i32::MAX), |(ax, ay), p| {
                (ax.min(p.x), ay.min(p.y))
            });
        let net_width = members
            .iter()
            .map(|bi| nodes[bi].x - min_x + margins[*bi].node_width(&section.blocks[*bi]))
            .chain(bends.values().flatten().map(|p| p.x - min_x + 1))
            .max()
            .unwrap_or(0);

        // Comentarios de la red: un encabezado por comentario, del ancho de la red y con el
        // alto que pide el texto, apilados en el orden del modelo.
        let top = cursor_y;
        for (_, text) in section.comments.iter().filter(|(b, _)| members.contains(b)) {
            let width = net_width.max(10);
            let lines = text_lines(text, width, cfg);
            layout.texts.push(TextBox {
                x: margin,
                y: cursor_y,
                width,
                height: lines + 1,
                text: text.clone(),
            });
            layout.width = layout.width.max(margin + width);
            cursor_y += lines + 2;
            check_extent(section, cursor_y)?;
        }
        let (dx, dy) = (margin - min_x, cursor_y - min_y);

        let mut bottom = cursor_y;
        for &bi in members {
            let (n, b) = (nodes[&bi], &section.blocks[bi]);
            layout.blocks[bi] = Pos {
                x: n.x + dx + margins[bi].left,
                y: n.y + dy,
            };
            bottom = bottom.max(n.y + dy + coord(b.height));
            layout.width = layout.width.max(n.x + dx + margins[bi].node_width(b));
        }
        for (wi, pts) in bends {
            let w = section.wires[wi];
            let mut line = vec![pin_cell(section, &layout, w.from, Side::East)];
            line.extend(pts.into_iter().map(|p| Pos {
                x: p.x + dx,
                y: p.y + dy,
            }));
            line.push(pin_cell(section, &layout, w.to, Side::West));
            bottom = bottom.max(line.iter().map(|p| p.y + 1).max().unwrap_or(0));
            layout.wires[wi] = line;
        }
        layout.networks.push((top, bottom - top));
        cursor_y = bottom + coord(cfg.network_gap);
        check_extent(section, cursor_y.max(layout.width))?;
    }
    layout.width += margin;
    layout.height = cursor_y - coord(cfg.network_gap) + margin;
    check_extent(section, layout.width.max(layout.height))?;
    check(section, &layout, &margins)?;
    Ok(layout)
}

/// Rectángulo de celdas [x, x + w) × [y, y + h).
struct Rect {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
}

impl Rect {
    fn overlaps(&self, o: &Rect) -> bool {
        self.x < o.x + o.w && o.x < self.x + self.w && self.y < o.y + o.h && o.y < self.y + self.h
    }
}

/// Invariantes de la grilla antes de exportar: ningún bloque (con sus parámetros) pisa a
/// otro ni a un comentario; cada cable existe, es ortogonal y no sale de la hoja; y dos
/// cables de salidas distintas no comparten tramos ni se tocan en un quiebre.
fn check(section: &Section, layout: &Layout, margins: &[Margins]) -> Result<()> {
    let mut boxes: Vec<(&str, Rect)> = section
        .blocks
        .iter()
        .zip(&layout.blocks)
        .zip(margins)
        .map(|((b, p), m)| {
            let r = Rect {
                x: p.x - m.left,
                y: p.y,
                w: m.node_width(b),
                h: coord(b.height),
            };
            (b.id.as_str(), r)
        })
        .collect();
    boxes.extend(layout.texts.iter().map(|t| {
        let r = Rect {
            x: t.x,
            y: t.y,
            w: t.width,
            h: t.height,
        };
        ("comentario", r)
    }));
    if let Some((name, r)) = boxes.iter().find(|(_, r)| r.x < 0 || r.y < 0) {
        bail!("{name} sale de la hoja en ({}, {})", r.x, r.y);
    }
    for (i, (a, ra)) in boxes.iter().enumerate() {
        if let Some((b, _)) = boxes[i + 1..].iter().find(|(_, rb)| ra.overlaps(rb)) {
            bail!("{a} se superpone con {b}");
        }
    }
    for (w, pts) in section.wires.iter().zip(&layout.wires) {
        let name = || format!("{} -> {}", section.pin_path(w.from), section.pin_path(w.to));
        if pts.len() < 2 {
            bail!("el cable {} no tiene trazado", name());
        }
        if pts.windows(2).any(|s| s[0].x != s[1].x && s[0].y != s[1].y) {
            bail!("el cable {} no es ortogonal: {pts:?}", name());
        }
        if pts.iter().any(|p| p.x < 0 || p.y < 0) {
            bail!("el cable {} sale de la hoja: {pts:?}", name());
        }
    }
    // Un tramo superpuesto con otro deja algún extremo sobre el otro tramo; un cruce
    // perpendicular, no.
    let on = |q: Pos, a: Pos, b: Pos| {
        (a.x.min(b.x)..=a.x.max(b.x)).contains(&q.x) && (a.y.min(b.y)..=a.y.max(b.y)).contains(&q.y)
    };
    for (i, (wi, pi)) in section.wires.iter().zip(&layout.wires).enumerate() {
        for (wj, pj) in section.wires.iter().zip(&layout.wires).skip(i + 1) {
            if wi.from == wj.from {
                continue;
            }
            for s in pi.windows(2) {
                for t in pj.windows(2) {
                    if on(t[0], s[0], s[1])
                        || on(t[1], s[0], s[1])
                        || on(s[0], t[0], t[1])
                        || on(s[1], t[0], t[1])
                    {
                        bail!(
                            "los cables de {} y {} se tocan",
                            section.pin_path(wi.from),
                            section.pin_path(wj.from)
                        );
                    }
                }
            }
        }
    }
    Ok(())
}

/// Reasignación de las entradas conmutativas de un bloque: la conexión del pin `from`
/// pasa al pin `slot`.
pub(crate) struct InputSwap {
    block: usize,
    mapping: HashMap<usize, usize>,
}

/// Entradas de los bloques conmutativos a reasignar según la altura de lo que las
/// alimenta en `layout`, para que los cables lleguen sin cruzarse. Vacío si ya están bien.
pub(crate) fn commutative_swaps(section: &Section, layout: &Layout) -> Vec<InputSwap> {
    let mut swaps = Vec::new();
    for bi in 0..section.blocks.len() {
        let group: Vec<usize> = (0..section.blocks[bi].pins.len())
            .filter(|&pi| section.blocks[bi].pins[pi].permutable)
            .collect();
        if group.len() < 2 {
            continue;
        }
        let by = layout.blocks[bi].y;
        // Clave = altura de la fuente si viene por cable; si no, la fila actual del pin.
        let mut keys: Vec<(i32, usize)> = group
            .iter()
            .map(|&pi| {
                let r = PinRef { block: bi, pin: pi };
                let key = section
                    .wires
                    .iter()
                    .find(|w| w.to == r)
                    .map_or(by + coord(section.blocks[bi].pins[pi].row), |w| {
                        layout.blocks[w.from.block].y + coord(section.pin(w.from).row)
                    });
                (key, pi)
            })
            .collect();
        keys.sort_unstable();
        // El slot k (en orden de fila) recibe la conexión con la k-ésima clave más chica.
        let mapping: HashMap<usize, usize> = keys
            .iter()
            .zip(&group)
            .map(|(&(_, from_pin), &slot)| (from_pin, slot))
            .collect();
        if mapping.iter().any(|(a, b)| a != b) {
            swaps.push(InputSwap { block: bi, mapping });
        }
    }
    swaps
}

/// Aplica las reasignaciones: mueve parámetros y destinos de cables. Es la única etapa
/// del layout que cambia la sección, y no cambia su significado (entradas conmutativas).
pub(crate) fn apply_swaps(section: &mut Section, swaps: &[InputSwap], diags: &mut Diagnostics) {
    for swap in swaps {
        let block = &mut section.blocks[swap.block];
        let params: HashMap<usize, Option<Param>> = swap
            .mapping
            .keys()
            .map(|&pi| (pi, block.pins[pi].param_in.clone()))
            .collect();
        for (&from_pin, &slot) in &swap.mapping {
            block.pins[slot].param_in.clone_from(&params[&from_pin]);
        }
        for w in section
            .wires
            .iter_mut()
            .filter(|w| w.to.block == swap.block)
        {
            if let Some(&slot) = swap.mapping.get(&w.to.pin) {
                w.to.pin = slot;
            }
        }
        diags.push(
            Level::Info,
            STAGE,
            format!(
                "{}: entradas conmutativas reordenadas",
                section.blocks[swap.block].id
            ),
        );
    }
}

/// Dibuja las redes en la grilla y valida el resultado. No modifica la sección.
pub(crate) fn layout(section: &Section, nets: &[Vec<usize>], cfg: &LayoutConfig) -> Result<Layout> {
    run_once(section, nets, cfg)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::Kind;
    use crate::model::{Dir, Wire};

    fn block(id: &str) -> Block {
        let pin = |name: &str, dir, row| Pin {
            name: name.into(),
            dir,
            ty: "BOOL".into(),
            row,
            permutable: false,
            param_in: None,
            param_out: None,
            inferred: None,
        };
        Block {
            id: id.into(),
            ty: "NOT".into(),
            kind: Kind::Ef,
            instance: None,
            comment: None,
            group: None,
            stage: None,
            extra_pins: 0,
            width: 6,
            height: 5,
            pins: vec![pin("IN", Dir::In, 4), pin("OUT", Dir::Out, 4)],
        }
    }

    /// Dos NOT que alimentan a un tercero, ubicados a mano.
    fn section() -> (Section, Layout, Vec<Margins>) {
        let wire = |from, to| Wire {
            from: PinRef {
                block: from,
                pin: 1,
            },
            to: PinRef { block: to, pin: 0 },
        };
        let section = Section {
            name: "T".into(),
            blocks: vec![block("a"), block("b"), block("c")],
            wires: vec![wire(0, 2), wire(1, 2)],
            execution_dependencies: Vec::new(),
            generated_vars: Vec::new(),
            comments: Vec::new(),
        };
        let layout = Layout {
            blocks: vec![Pos { x: 0, y: 0 }, Pos { x: 0, y: 8 }, Pos { x: 12, y: 0 }],
            wires: vec![Vec::new(), Vec::new()],
            networks: vec![(0, 13)],
            texts: Vec::new(),
            width: 20,
            height: 13,
        };
        let margins = section
            .blocks
            .iter()
            .map(|b| Margins::new(b, &LayoutConfig::default()))
            .collect();
        (section, layout, margins)
    }

    fn line(points: &[(i32, i32)]) -> Vec<Pos> {
        points.iter().map(|&(x, y)| Pos { x, y }).collect()
    }

    #[test]
    fn grid_check_rejects_what_control_expert_would_draw_wrong() {
        let (section, mut layout, margins) = section();
        layout.wires = vec![
            line(&[(5, 4), (12, 4)]),
            line(&[(5, 12), (8, 12), (8, 4), (12, 4)]),
        ];
        // El segundo cable llega a la fila del primero: comparten tramo.
        let error = check(&section, &layout, &margins).unwrap_err();
        assert!(error.to_string().contains("se tocan"), "{error}");

        layout.wires[1] = line(&[(5, 12), (12, 4)]);
        let error = check(&section, &layout, &margins).unwrap_err();
        assert!(error.to_string().contains("no es ortogonal"), "{error}");

        layout.wires[1] = line(&[(5, 12), (8, 12), (8, 5), (12, 5)]);
        layout.blocks[1].y = 3;
        let error = check(&section, &layout, &margins).unwrap_err();
        assert!(error.to_string().contains("se superpone"), "{error}");
    }
}
