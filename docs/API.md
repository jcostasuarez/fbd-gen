# API Rust

La entrada recomendada es `fbd_gen::Generator`. Mantiene el catálogo y la configuración
para reutilizarlos entre generaciones. Ninguna operación escribe archivos de salida:
el llamador decide qué guardar y dónde.

```rust,no_run
use fbd_gen::{Generator, ProgramSpec};

let generator = Generator::from_library_file("efb_dimensions.json")?;
let spec: ProgramSpec = serde_json::from_str(&std::fs::read_to_string("examples/planta_bombas.json")?)?;
let program = generator.generate_program(&spec)?;
for section in program.sections {
    println!("{}: {} bloques", section.name, section.layout.blocks.len());
    // section.xml, section.svg y section.layout_json()? están listos para guardar.
    // section.variables contiene también las declaraciones de instancias EFB/DFB.
}
# Ok::<(), Box<dyn std::error::Error>>(())
```

## Operaciones principales

| Operación | Entrada | Salida |
|---|---|---|
| `Generator::new` | `Library` | Generador con opciones por defecto |
| `Generator::from_library_file` | Ruta del catálogo exportado de Control Expert | `Result<Generator, GenerationError>` |
| `with_config` | Generador y `GeneratorConfig` | `Result<Generator, GenerationError>`; valida las opciones |
| `library` / `config` | Generador | Referencias de solo lectura al catálogo y las opciones |
| `generate_program` | `&ProgramSpec` | `Result<GeneratedProgram, GenerationError>` |
| `generate_section` | `&SectionSpec` | `Result<GeneratedSection, GenerationError>` |
| `generate_function` | `&FunctionCallSpec` | `Result<GeneratedSection, GenerationError>` con una única llamada |
| `register_block` | `BlockDefinition` | `Result<(), LibraryError>` |
| `register_blocks_from_file` | Ruta de un catálogo adicional | `Result<(), LibraryError>` |

`ProgramSpec::from_json`, `SectionSpec::from_json` y `FunctionCallSpec::from_json` leen
la entrada desde JSON. A diferencia de `serde_json::from_str`, un error de formato
(sintaxis, campo faltante o desconocido como `instace`) llega como
`GenerationError::InvalidInput` con el código `invalid_json`, igual que los errores de
validación, así un cliente los trata a todos del mismo modo.

`ProgramSpec` contiene `name`, `task`, `variables`, `blocks`, `links`, `comments` y
`partition`. Verifica consistencia entre equipos, divide por grupos/etapas/tamaño y
ordena las secciones por dependencias.

`SectionSpec` contiene los mismos campos salvo `partition`: siempre genera una sola
sección. Deserializar un JSON con `partition` como `SectionSpec` es un error; `group`
y `stage` de sus bloques se conservan como metadatos. No se aplica el chequeo entre equipos.

Los dos formatos implementan `Serialize` y `Deserialize`. Para construirlos desde Rust
existen `ProgramSpec::new`, `SectionSpec::new`, `BlockSpec::new`, `VarSpec::new` y
`CommentSpec::new`; los campos opcionales se completan después, uno por uno.

`GeneratorConfig` agrupa `normalization`, `layout` y `consistency`; cada configuración
tiene `Default` y se ajusta campo por campo:

```rust,no_run
use fbd_gen::{Generator, GeneratorConfig};

let mut config = GeneratorConfig::default();
config.normalization.max_fanout = 5;
config.layout.node_gap = 3;
let generator = Generator::embedded()?.with_config(config)?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

### Compatibilidad entre versiones

Los structs públicos (entradas, configuraciones, resultados, `Diagnostic` y los tipos de
`sugiyama`) son `#[non_exhaustive]`: una versión nueva puede agregarles campos sin romper
a quien los use. Por eso, desde otro crate no se arman con un literal `Tipo { .. }`, ni
siquiera con `..Default::default()`: se usan `new`/`Default` y después se asignan los
campos públicos. Los enums de errores, `Stage` y `DiagnosticCode` también pueden sumar
variantes, así que un `match` sobre ellos necesita un brazo `_`. `Level`, `PinDirection`,
`BlockKind`, `Parameter` y `LinkSpec` son conjuntos cerrados. `LayoutConfig` son medidas enteras en celdas: separación entre
bloques, entre capas y entre redes, margen de la hoja y escalas de texto.

## Resultados y errores

`GeneratedProgram` contiene nombre, tarea, diagnósticos globales y secciones en orden de
ejecución. Cada `GeneratedSection` contiene:

- `layout`: datos propios de bloques, pines, redes, coordenadas, recorridos y textos.
- `variables`: declaraciones emitidas, incluidas variables inferidas e instancias EFB/DFB.
- `diagnostics`: avisos e información propios de la sección (normalización y layout). Los
  de validación, tipos, consistencia y partición están una sola vez, en el programa.
  Cada `Diagnostic` tiene `level`, `stage` (`Stage::Validate`, `Types`, `Consistency`,
  `Partition`, `Normalize` o `Layout`, en minúsculas al serializar), `code`, `location`
  y `message` (ver [Diagnósticos](#diagnósticos)).
- `xml` y `svg`: representaciones listas para guardar.
- `layout_json()`: serialización del layout, compatible con el archivo `.layout.json` de la CLI.
- `file_name(ext)`: nombre de archivo que usa la CLI (`<sección>.xbd`, `.svg`, `.layout.json`).

`GeneratedProgram::write_to(dir)` escribe los mismos archivos que la CLI más
`manifest.json` (secciones en orden de ejecución y diagnósticos del programa), y
`manifest_json()` devuelve solo el manifest.

Los resultados no toman referencias a la entrada ni al generador: pueden conservarse
después de liberar ambos. También implementan `Serialize` para intercambiarlos completos.

Un resultado `Ok` no contiene secciones inválidas ni salidas opcionales. Los errores son:

| Variante de `GenerationError` | Significado |
|---|---|
| `InvalidInput { scope, diagnostics }` | Grafo inválido; incluye el ámbito y los diagnósticos |
| `InvalidConfig { message }` | Opciones fuera de rango o inválidas |
| `Library(LibraryError)` | Lectura o deserialización del catálogo base |
| `Layout { section, source }` | El motor de layout no pudo dibujar la sección sin violar la grilla |
| `Internal { section, message }` | Se rompió un invariante del pipeline: es un bug de fbd-gen, no un problema de la entrada |

La generación no entra en pánico: el código de producción no tiene `unwrap`, `expect`,
`panic!` ni `unreachable!` (lo impone un lint de clippy). Un invariante roto llega como
`Internal` o, dentro del motor, como `Layout` con causa `sugiyama::LayoutError::Internal`;
conviene reportarlo con la entrada que lo produce.

`GenerationError::diagnostics()` da acceso a los diagnósticos de una entrada inválida.
Los fallos operativos conservan su causa mediante `std::error::Error::source()`, y cada
nivel de la cadena tiene su propio mensaje sin repetir el de su causa: para mostrarlos
todos se recorre la cadena (por ejemplo, `anyhow` con `{:#}`).
Si falla una sección, `generate_program` devuelve `Err`; no publica un programa parcial.

## Diagnósticos

Un diagnóstico sirve a dos lectores. `message` es para personas y puede cambiar de
redacción. `code` (`DiagnosticCode`) y `location` (`DiagnosticLocation`) son para
programas, por ejemplo el LLM que corrige su propio grafo: con ellos se puede decidir
qué arreglar sin interpretar el texto.

- `code` dice qué se detectó: `unknown_pin`, `input_already_driven`, `type_mismatch`,
  `undeducible_type`, `feedback_cut`... Su nombre serializado (`DiagnosticCode::as_str`)
  es estable; `DiagnosticCode::ALL` lista todos y la documentación de cada variante dice
  cuándo aparece. Pueden sumarse códigos en versiones futuras.
- `location` dice dónde, en términos de la entrada: `block` (el `id` del bloque), `pin`,
  `link` (índice en `links`, desde 0), `variable`, `comment` (índice en `comments`),
  `group` (equipo) y `section`. Solo se completan los campos que aplican; al serializar
  se omiten los vacíos, y `location` entera se omite si no hay ninguno.

```json
{
  "level": "error",
  "stage": "validate",
  "code": "unknown_pin",
  "location": { "block": "n", "pin": "FOO", "link": 0 },
  "message": "n: el bloque NOT no tiene el pin FOO"
}
```

```rust,no_run
use fbd_gen::{DiagnosticCode, Generator, ProgramSpec};

let spec: ProgramSpec = serde_json::from_str(r#"{"name":"P","blocks":[{"id":"n","type":"NOT"}],
    "links":[["TRUE","n.FOO"]]}"#)?;
let error = Generator::embedded()?.generate_program(&spec).unwrap_err();
for d in error.diagnostics() {
    if d.code == DiagnosticCode::UnknownPin {
        println!("link {:?}: {:?} no tiene {:?}", d.location.link, d.location.block, d.location.pin);
    }
}
# Ok::<(), Box<dyn std::error::Error>>(())
```

La CLI los imprime como `[error] validate/unknown_pin: ...`, y `manifest.json` y cada
`.layout.json` los incluyen completos.

## DFB y funciones externas

Una DFB creada en Control Expert puede registrarse por su interfaz y dimensiones:

```rust,no_run
use fbd_gen::{BlockDefinition, Generator};

let mut generator = Generator::from_library_file("efb_dimensions.json")?;
generator.register_block(
    BlockDefinition::dfb("MotorPropio", 12)
        .input("Enable", "BOOL", 1)
        .input("Reset", "BOOL", 2)
        .output("Running", "BOOL", 1)
        .in_out("State", "INT", 3)
)?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

El ancho se expresa en celdas, de 2 a 1000. Las posiciones de pines empiezan en 1 y se
expresan en su lado, sin contar las filas de cabecera que agrega el generador; la última
fila de un pin (contando sus extensiones) no puede pasar de 996. Los topes solo descartan
definiciones absurdas: la librería de Control Expert 15.1 llega a 20 de ancho y a la
posición 34. IN_OUT ocupa ambos lados:
su posición debe quedar libre tanto entre las entradas como entre las salidas.

El registro valida nombres, tipos no vacíos, dimensiones, pines duplicados y posiciones
superpuestas. `EN` y `ENO` están reservados: el exportador los agrega implícitamente.
No comprueba la existencia de tipos DDT propios en Control Expert.

`Library::register_all` y `extend_from_json` son atómicos: ante un error no incorporan
ninguno de los bloques de ese lote. Los duplicados se rechazan sin distinguir mayúsculas;
el catálogo base no se sobrescribe.

También se puede describir la interfaz en JSON. El formato adicional es
`{ "blocks": [BlockDefinition, ...] }`, como en `examples/custom_blocks.json`.
Es un formato de interfaz simple; el catálogo base exportado de Control Expert conserva
su formato con `geometry` y `size`.

```bash
cargo run -- examples/custom_dfb.json --extra-lib examples/custom_blocks.json
```

`--extra-lib` puede repetirse para cargar varios archivos. Desde Rust se usa
`register_blocks_from_file`, o se construye un `Library` y se llama a
`register`, `register_all`, `extend_from_json` o `extend_from_file` antes de crear el generador.
`Library::default()` permite empezar con un catálogo vacío.

`Library::iter()` (o `for def in &library`) recorre todas las definiciones, base y
propias, ordenadas por nombre sin distinguir mayúsculas; `get` busca una por nombre. Por
ejemplo, para describirle al LLM los bloques disponibles:

```rust,no_run
use fbd_gen::{BlockKind, Generator};

let generator = Generator::embedded()?;
for def in generator.library().iter().filter(|d| d.kind == BlockKind::Efb) {
    let inputs: Vec<&str> = def.inputs.iter().map(|p| p.name.as_str()).collect();
    println!("{}({})", def.name, inputs.join(", "));
}
# Ok::<(), Box<dyn std::error::Error>>(())
```

El campo `kind` admite `EF`, `EFB` y `DFB`. Para EF/EFB se puede deserializar la
definición o construirla con `BlockDefinition::new(nombre, kind, ancho)`. Una entrada
extensible se arma con `PinDefinition` y `ExtensiblePin` y se agrega con `pin`:

```rust,no_run
use fbd_gen::{BlockDefinition, BlockKind, ExtensiblePin, Generator, PinDefinition, PinDirection};

let mut generator = Generator::embedded()?;
generator.register_block(
    // IN1..IN8, mínimo 2: `"inputs": 4` en el bloque crea IN1..IN4.
    BlockDefinition::new("MiAnd", BlockKind::Ef, 8)
        .pin(
            PinDirection::In,
            PinDefinition::new("IN1", "BOOL", 1).with_extensible(ExtensiblePin::new("IN", 1, 2, 8)),
        )
        .output("OUT", "BOOL", 1),
)?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

Las EFB/DFB generan instancias; una EF genera un objeto sin instancia persistente.

**Registrar una interfaz no crea ni importa la lógica de la DFB.** Antes de importar la
sección generada, el proyecto de Control Expert debe tener su definición y los DDT que
utilice. `examples/custom_dfb.json` sirve para demostrar la generación; requiere definir
`MotorPropio` en Control Expert para compilar allí.

El ejemplo Rust ejecutable equivalente es:

```bash
cargo run --example api_custom_dfb
```

## Llamada a una función existente

`generate_function` permite generar una sección que llama a una EF/EFB/DFB del catálogo,
sin armar manualmente `blocks` y `links`. No crea una nueva definición de función o DFB.

```rust,no_run
use fbd_gen::{FunctionCallSpec, Generator};

let generator = Generator::from_library_file("efb_dimensions.json")?;
let call = FunctionCallSpec::new("P101_Delay", "TON")
    .with_instance("P101_Timer")
    .input("IN", "P101_Start")
    .input("PT", "T#3s")
    .output("Q", "P101_Run");
let section = generator.generate_function(&call)?;
// section.xml, section.svg, section.layout y section.variables.
# Ok::<(), Box<dyn std::error::Error>>(())
```

`name` es el nombre de la sección y `function` es el tipo de bloque existente.
`inputs` acepta variables o literales IEC; `outputs` acepta variables destino.
`inOuts` asigna una variable a cada parámetro IN_OUT mediante `.in_out("Pin", "Variable")`:
el mismo pin se exporta de ambos lados y Control Expert actualiza esa variable por referencia.
No se debe repetir un IN_OUT en `inputs` u `outputs`.

`instance` solo se admite para EFB/DFB; si falta, se usa el nombre saneado de la sección
con sufijo `_call`. Las EF no tienen instancia. `inputCount` permite indicar la cantidad
de entradas extensibles dentro del rango del bloque; los pines referenciados también
pueden ampliar esa cantidad. `variables` permite declarar tipos y metadatos explícitos
cuando no alcanza la inferencia. `task` toma `MAST` por defecto (ver las tareas admitidas en el README) y `comment` comenta
el bloque.

Se comprueban el tipo de bloque, la dirección y existencia de los pines, los parámetros
vacíos y el uso de variables donde corresponde. Las llamadas usan la misma validación
de tipos y exportación que `generate_section`. Para una DFB propia, registrarla antes
con `register_block` o `register_blocks_from_file`.

El formato JSON equivalente está en `examples/function_call.json`. El ejemplo ejecutable:

```bash
cargo run --example api_function
```

## Uso desde otro ejecutable

El crate es Rust puro y no lanza procesos. Con el feature `embedded-library`,
`Generator::embedded()` (o `Library::embedded()`) usa la librería de Control Expert 15.1
incluida en el binario, sin depender de `efb_dimensions.json` en disco;
`Library::from_json_str` carga cualquier catálogo ya leído.

```toml
fbd-gen = { path = "...", default-features = false, features = ["embedded-library"] }
```

```rust
let generator = fbd_gen::Generator::embedded()?;
# Ok::<(), fbd_gen::GenerationError>(())
```

## Motor de layout

`fbd_gen::sugiyama::layout` se puede usar por separado: recibe cajas (`Node::new`), cables
entre puertos en filas fijas (`Edge::new`, sin ciclos) y separaciones (`Spacing::new`), y
devuelve la esquina de cada caja y los quiebres de cada cable (`Placement`), todo en enteros.

```rust
use fbd_gen::sugiyama::{Edge, Node, Spacing, layout};

// Dos cajas de 4x3; la salida de la fila 1 de la primera va a la entrada de la fila 1
// de la segunda.
let placement = layout(
    &[Node::new(4, 3), Node::new(4, 3)],
    &[Edge::new(0, 1, 1, 1)],
    &Spacing::new(2, 3),
)?;
assert!(placement.nodes[1].x > placement.nodes[0].x);
# Ok::<(), fbd_gen::sugiyama::LayoutError>(())
```
Si la entrada no se puede dibujar devuelve `sugiyama::LayoutError`: un puerto fuera de su
caja (`PortOutOfNode`), cables con un ciclo (`Cycle`), restricciones sin solución
(`Infeasible`), una caja con medidas negativas o un dibujo que podría pasar de
`sugiyama::MAX_EXTENT` celdas por eje (`TooLarge`), o un invariante interno roto
(`Internal`, un bug). Ese tope garantiza que ninguna cuenta
en `i32` desborde.

Los demás módulos (validación, partición, normalización, export) son internos: su orden e
invariantes no forman parte del contrato. La API pública es `Generator`, los tipos que
re-exporta la raíz del crate y `sugiyama`.
