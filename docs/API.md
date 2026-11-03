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

`ProgramSpec` contiene `name`, `task`, `variables`, `blocks`, `links`, `comments` y
`partition`. Verifica consistencia entre equipos, divide por grupos/etapas/tamaño y
ordena las secciones por dependencias.

`SectionSpec` contiene los mismos campos salvo `partition`: siempre genera una sola
sección. Deserializar un JSON con `partition` como `SectionSpec` es un error; `group`
y `stage` de sus bloques se conservan como metadatos. No se aplica el chequeo entre equipos.

Los dos formatos implementan `Serialize` y `Deserialize`. Para construirlos desde Rust
existen `ProgramSpec::new`, `SectionSpec::new`, `BlockSpec::new` y `VarSpec::new`.

`GeneratorConfig` agrupa `normalization`, `layout` y `consistency`; cada configuración
tiene `Default`. `LayoutConfig` son medidas enteras en celdas: separación entre
bloques, entre capas y entre redes, margen de la hoja y escalas de texto.

## Resultados y errores

`GeneratedProgram` contiene nombre, tarea, diagnósticos globales y secciones en orden de
ejecución. Cada `GeneratedSection` contiene:

- `layout`: datos propios de bloques, pines, redes, coordenadas, recorridos y textos.
- `variables`: declaraciones emitidas, incluidas variables inferidas e instancias EFB/DFB.
- `diagnostics`: avisos e información propios de la sección (normalización y layout). Los
  de validación, tipos, consistencia y partición están una sola vez, en el programa.
  Cada `Diagnostic` tiene `level`, `stage` (`Stage::Validate`, `Types`, `Consistency`,
  `Partition`, `Normalize` o `Layout`, en minúsculas al serializar) y `message`.
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

`GenerationError::diagnostics()` da acceso a los diagnósticos de una entrada inválida.
Los fallos operativos conservan su causa mediante `std::error::Error::source()`.
Si falla una sección, `generate_program` devuelve `Err`; no publica un programa parcial.

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

El campo `kind` admite `EF`, `EFB` y `DFB`. Para EF/EFB se pueden construir o deserializar
`BlockDefinition`, `PinDefinition` y, para una EF extensible, `ExtensiblePin`.
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
```

## Motor de layout

`fbd_gen::sugiyama::layout` se puede usar por separado: recibe cajas (`Node`), cables
entre puertos en filas fijas (`Edge`, sin ciclos) y separaciones (`Spacing`), y devuelve la
esquina de cada caja y los quiebres de cada cable (`Placement`), todo en enteros.
Si la entrada no se puede dibujar devuelve `sugiyama::LayoutError`: un puerto fuera de su
caja (`PortOutOfNode`), cables con un ciclo (`Cycle`), restricciones sin solución
(`Infeasible`), o una caja con medidas negativas o un dibujo que podría pasar de
`sugiyama::MAX_EXTENT` celdas por eje (`TooLarge`). Ese tope garantiza que ninguna cuenta
en `i32` desborde.

Los demás módulos (validación, partición, normalización, export) son internos: su orden e
invariantes no forman parte del contrato. La API pública es `Generator`, los tipos que
re-exporta la raíz del crate y `sugiyama`.
