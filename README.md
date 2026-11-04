# fbd-gen

Generador de secciones FBD para EcoStruxure Control Expert 15.1, en Rust puro.
El LLM (u otra herramienta) produce un **grafo semántico** sin coordenadas; este crate lo
valida contra la librería de bloques y lo dibuja con un Sugiyama propio en celdas enteras.

## Uso

Cada [release](https://github.com/jcostasuarez/fbd-gen/releases) trae el ejecutable para
Windows y Linux, sin dependencias: descomprimir y correr.

```bash
fbd-gen examples/planta_bombas.json
```

Desde el código fuente, con Rust 1.88 o posterior:

```bash
cargo run -- examples/planta_bombas.json
```

No necesita Node, internet ni archivos externos: la librería de bloques de Control
Expert 15.1 va incluida en el ejecutable (feature `embedded-library`, activo por
defecto). `--lib` permite usar otra exportación de la librería; cada bloque se valida
con las mismas reglas que un bloque propio (nombres IEC, pines sin repetir ni superponer,
tipo EF/EFB/DFB) y un nombre repetido es un error. Para usar solo la
librería desde otro crate:

```toml
fbd-gen = { path = "...", default-features = false, features = ["embedded-library"] }
```

### API Rust y bloques propios

La API recomendada es `Generator`: recibe un `ProgramSpec` o un `SectionSpec` y devuelve
resultados estructurados más XML/SVG, o un `GenerationError` explícito. El generador
conserva su catálogo y opciones entre llamadas. `Generator::embedded()` lo crea con la librería incluida. Ver [entradas y
salidas](docs/API.md).

Para generar una única llamada a un bloque existente, `generate_function` recibe un
`FunctionCallSpec` con el tipo, instancia y asignaciones de pines. Ver el ejemplo
`cargo run --example api_function` y su equivalente `examples/function_call.json`.

Las interfaces de EF/EFB/DFB propias se pueden registrar desde Rust con
`register_block`, o desde un catálogo JSON adicional:

```bash
cargo run -- examples/custom_dfb.json --extra-lib examples/custom_blocks.json
```

El catálogo adicional no modifica el catálogo base y rechaza nombres duplicados.
Registrar la interfaz permite generar instancias y conexiones: la implementación de
la DFB y sus tipos DDT deben existir en Control Expert antes de importar la sección.

Escribe en `out/<programa>/`, por cada sección:

- `<sección>.xbd`: XML de intercambio FBD, listo para `import_xml(kind="section")`.
- `<sección>.layout.json`: bloques en celdas de grilla, cables como polilíneas ortogonales,
  variables generadas y diagnósticos.
- `<sección>.svg`: vista previa.

y `manifest.json` con las secciones en orden de ejecución y los diagnósticos de programa.

Otras opciones de la CLI:

- `-` como entrada lee el grafo de la entrada estándar.
- `--check` valida y genera en memoria, sin escribir archivos.
- `--format json` escribe en stdout un solo objeto JSON: `status` (`ok`, `invalid` o
  `error`), las secciones con sus archivos y los diagnósticos completos. Pensado para
  que otra herramienta o un LLM lea el resultado sin interpretar texto.

```bash
cat grafo.json | fbd-gen - --check --format json
```

El código de salida es 0 si genera, 1 si la entrada es inválida (incluido un JSON mal
formado o con un campo desconocido), 2 si los argumentos son incorrectos y 3 ante un
error de lectura, escritura, librería o un error interno.

Cada diagnóstico trae un `code` estable (`unknown_pin`, `type_mismatch`...) y una
`location` con el bloque, pin, índice de link o variable de la entrada que lo causó, para
que un programa (o el LLM que generó el grafo) pueda corregirlo sin interpretar el
mensaje. Ver [diagnósticos](docs/API.md#diagnósticos).

### Verificación en Control Expert

`scripts/ce_check.py` automatiza Control Expert mediante el servidor `control-expert-mcp`
(proyecto `sce_mcp`); indicar su ejecutable en `CE_MCP_EXE` si no está en `../sce_mcp`.

```bash
python scripts/ce_check.py out/Planta_Bombas/manifest.json
```

Crea un proyecto M580 nuevo, importa las secciones en orden, compila con rebuild completo y
lo guarda en `out/ce_test/fbd_check_<fecha>.stu`. Sale con código 0 solo si el build termina sin errores.

Con `--dump` guarda además cómo reexporta Control Expert cada sección, y
`scripts/roundtrip_diff.py` lo compara contra lo generado (bloques, orden de pines, cables con
sus quiebres, textBox):

```bash
python scripts/ce_check.py --dump out/Planta_Bombas/manifest.json
python scripts/roundtrip_diff.py out/Planta_Bombas/manifest.json
```

## Formato de entrada

Ver `examples/p101_motor.json` y la documentación de `SectionSpec` en `src/model.rs`.
Cada link es `[origen, destino]`; un extremo es `bloque.PIN`, un literal IEC o una variable.
El nombre del programa, los ids de bloque y las instancias son identificadores IEC
(letra o `_`, después letras, dígitos o `_`; sin caracteres extendidos). Las variables
admiten además campos (`Motor.Run`), índices (`Tab[3]`) y direcciones (`%MW100`). Un
campo o un índice declara su raíz, que tiene que estar en `variables` (su tipo no se
puede deducir) o ser una instancia de bloque (`T1.ET`); una dirección no se declara. Los
ids de bloque no distinguen mayúsculas, y una variable no puede llamarse como una instancia
ni, si está declarada, como un id de bloque (`Motor.Run` sería ambiguo). Un
nombre inválido, un grafo sin bloques o más entradas extensibles que el máximo del
bloque son errores de validación, antes de ejecutar el layout.
Un campo desconocido (`instace`, `partiton`...) es un error al leer el JSON: un
typo nunca se ignora en silencio.
Los textos libres que van al XML (comentarios, `type`, `address`, `comment` e `init` de
las variables, literales de los links) no pueden tener caracteres de control salvo
tabulación y saltos de línea: XML 1.0 no los admite ni escapados.

`variables` es opcional: lo que no se declara se tipa por inferencia (`src/types.rs`), y
una variable sin tipo deducible es un error.

`task` es `MAST` por defecto; también se admiten `FAST`, `AUX0` a `AUX3` y `SAFE`, sin
distinguir mayúsculas, y el XML lleva el nombre en mayúsculas. En una variable declarada,
`address` tiene que ser una dirección topológica (`%MW100`) e `init`, si el tipo es
elemental, un literal: uno tipado tiene que ser del tipo declarado (`INT#3` no inicializa
un `WORD`), y uno sin prefijo (`16#FF`, `1.5`) solo sugiere el tipo, igual que en un pin.

Para un programa con varios equipos, cada bloque lleva `group` (equipo: `P101`) y
opcionalmente `stage` (etapa: `entradas`, `salidas`...). `partition` controla la división:

```json
"partition": { "byStage": true, "maxBlocks": 25 }
```

`comments` agrega un encabezado (textBox) a la red que contiene al bloque `at`:

```json
"comments": [{ "at": "perm", "text": "Permisivos de arranque" }]
```

Ver `examples/planta_bombas.json`: 4 bombas del mismo típico (P103 con un desvío y P104 con
otro tiempo a propósito) y una parte general partida en entradas y salidas.

Los nombres de sección se hacen únicos sin distinguir mayúsculas, incluso cuando un
grupo coincide con el nombre de una subdivisión (`A_1`) o dos grupos se sanean igual
(`P-A`, `P_A`). Los renombres se informan en los diagnósticos.

Los literales tipados (`REAL#1.5`, `DINT#3`, `WORD#16#FF`, `T#5s`...) aportan su tipo a
la inferencia. Las familias escalares `ANY_BIT`, `ANY_INT`, `ANY_REAL`, `ANY_NUM` y
`ANY_ELEMENTARY` rechazan tipos incompatibles antes de generar XML. Las familias de
arrays, referencias e IODDT todavía requieren validación en Control Expert.
Dos tipos elementales distintos unidos por un cable o una variable (`BOOL` y `TIME`,
`INT` y `DINT`) son un error; `EBOOL` y `BOOL` son compatibles. Entre tipos derivados
(arrays, DDT) la diferencia es un aviso, porque puede tratarse de un alias.

`--max-fanout` es un umbral de simplificación. Las redes se ordenan para que la fuente
de una variable creada por fan-out se ejecute antes que sus destinos, aunque estos se
declaren primero. Si el corte no permite conservar las dependencias mediante cables y
orden entre redes, se mantienen los cables y se informa en los diagnósticos. Los cortes
intencionales de realimentación conservan su lectura del scan anterior.

## Pipeline

| Etapa | Módulo |
|---|---|
| 1. Validación contra la librería | `resolve.rs` |
| 2. Inferencia de tipos, una sola vez sobre todo el programa (cada pin queda con su tipo) | `types.rs` |
| 3. Consistencia entre equipos (hash Weisfeiler-Lehman) | `consistency.rs` |
| 4. Partición: equipo, etapa, tamaño; chequeo de cohesión. Produce subgrafos ya resueltos y tipados, con los cables entre secciones convertidos en variables | `partition.rs` |
| 5. Orden de secciones por dependencias; ciclos reportados | `partition.rs` |
| 6. Normalización: cable o variable (realimentación, fan-out) | `normalize.rs` |
| 7. Redes (componentes conexos, dependencias de fan-out, desempate por orden del modelo) con comentario de red | `normalize.rs`, `layout.rs` |
| 8. Sugiyama por red y permutación de entradas conmutativas | `sugiyama/`, `layout.rs` |
| 9. Grilla, apilado de redes y validación de la geometría | `layout.rs` |
| Export a XML de intercambio FBD | `xef.rs` |

Las etapas 6 a 9 y el export son comunes a `generate_program`, `generate_section` y
`generate_function`; una sección nunca se vuelve a validar ni a tipar.

## Motor de layout

`sugiyama` es un layout jerárquico a la medida de FBD, sin flotantes en ninguna etapa:

1. **Capas** por network simplex (Gansner et al.): largo total mínimo de cables.
2. **Nodos ficticios** de una fila para los cables que saltan capas.
3. **Orden** en cada capa: barycenter por fila de puerto con barridos alternados; se
   conserva el orden con menos cruces y los empates siguen el orden del modelo.
4. **Posición vertical** por network simplex sobre el grafo auxiliar de Gansner:
   minimiza el desvío de cada cable con la separación mínima entre bloques. Los tramos
   de cables largos pesan más, para que corran derechos.
5. **Ruteo ortogonal** por canal: un tronco vertical por salida (su fan-out lo comparte),
   columnas ordenadas para que cables distintos nunca compartan un tramo. Si dos salidas
   intercambian filas, un tronco se parte por una fila libre.

Cada bloque va alineado a la izquierda de su capa. Antes de exportar, `layout.rs`
valida la grilla igual (bloques y comentarios sin superponerse, cables
ortogonales que no se tocan entre salidas distintas): si algo falla, la generación da
error en vez de producir un XML roto.
Un bloque propio no puede pasar de 1000 celdas de ancho ni de alto, y una sección no
puede pasar de `sugiyama::MAX_EXTENT` (2²⁴) celdas por eje: son topes propios de fbd-gen
para que las cuentas enteras no desborden, y pasarlos es un error.

## Hechos verificados contra Control Expert 15.1

- `p101_motor` (1 sección) y `planta_bombas` (6 secciones) importan y compilan con 0 errores
  y 0 advertencias; las secciones quedan en la tarea en el orden de importación.
- Las EF genéricas (`AND`, `MAX`, `GT`) se aceptan sin la variante tipada.
- Round-trip idéntico (`roundtrip_diff.py`) en las 9 secciones de los ejemplos: posiciones,
  pines, cables, quiebres y textBox se conservan tal cual.
- `textBox`: posición y tamaño en **celdas** (igual que los demos de Schneider
  `demo_ControlExpert_*.XEF`); el texto se ajusta solo, ~1.2-1.4 caracteres por celda.
- IN_OUT: va como `inputVariable` (con la variable) y como `outputVariable` con el mismo nombre.
  El lado derecho puede cablearse a una entrada común (PIDFF.OUT -> LIMIT.IN compila).
  La importación acepta cualquier orden de pines; Control Expert reexporta los IN_OUT antes que
  las salidas y así se generan (`examples/probes/io_probe.json`).
- Un lazo cerrado con un link falla con `E1153 Bucle sin fin`
  (probado con `examples/probes/loop_probe.json` y una variante armada a mano con el lazo
  cableado). Por eso se corta solo el cable
  que cierra el lazo: la salida escribe una variable (`<bloque>_<pin>`, o la que ya escribía)
  y el destino la lee; los demás cables de esa salida se mantienen.
- Una salida puede escribir una variable y tener cables a la vez: compila y el reexport la
  conserva igual. En el editor la variable queda superpuesta al cable que sale del pin; es la
  convención habitual y se deja así a propósito.
- Todas las coordenadas y tamaños son celdas enteras.
- `nbRows`/`nbColumns` se dejan en 24/36 aunque el contenido los exceda: otros valores
  hacen fallar la importación.

## Supuestos a verificar en Control Expert

- `label_cells_per_10_chars` (8 celdas cada 10 caracteres) estima el ancho de los parámetros actuales.
- 472 bloques no tienen ancho medido; se usa una estimación conservadora (aviso en diagnósticos).

## Pruebas locales

```bash
cargo test --all-targets --all-features
```

El motor se prueba con invariantes sobre 1000 redes pseudoaleatorias (sin superposiciones,
cables ortogonales que no atraviesan bloques ni se tocan entre salidas distintas) y el
network simplex contra su solución inicial. Las regresiones cubren además la grilla
entera de los ejemplos, el determinismo, entradas hostiles, el orden de fan-out, la
conservación de realimentaciones y cables necesarios, colisiones de nombres, familias
escalares y literales tipados.

`tests/properties.rs` genera programas pseudoaleatorios desde la librería real
(plausibles y hostiles) y verifica el contrato del pipeline completo: sin pánicos ni
errores internos, determinismo, rechazos con errores y ubicaciones que apuntan a la
entrada, XML y SVG bien formados y coherentes con el layout. `FBD_PROPERTY_CASES` cambia
la cantidad de casos (300 por defecto; el CI corre 5000 en release).

La entrada la escribe un LLM, así que el generador no debe entrar en pánico con ningún
JSON: un error de validación es un resultado válido, un pánico es un bug. `fuzz/` tiene
un target de [cargo-fuzz](https://github.com/rust-fuzz/cargo-fuzz) que prueba
`ProgramSpec`, `SectionSpec` y `FunctionCallSpec` arbitrarios y verifica que no aparezcan
errores internos (requiere nightly). Conviene empezar con los ejemplos como corpus:

```bash
mkdir -p fuzz/corpus/generate && cp examples/*.json examples/probes/*.json fuzz/corpus/generate/
cd fuzz && cargo +nightly fuzz run generate -- -max_total_time=300
```

`.github/workflows/fuzz.yml` lo corre un minuto en cada PR y media hora cada noche.

`examples/probes/fanout_order.json` permite inspeccionar también el layout generado
cuando los consumidores están declarados antes que la fuente. Estas pruebas no
reemplazan la compilación y el round-trip en Control Expert descritos arriba.

## Estilo y chequeos

El crate sigue las recomendaciones de [BeautifulRust](https://github.com/morganlinton/BeautifulRust),
que resume las [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/),
[*Effective Rust*](https://www.lurklurk.org/effective-rust/),
[Rust Design Patterns](https://rust-unofficial.github.io/patterns/) y las guías de
[Google](https://google.github.io/styleguide/rustguide.html) y
[Microsoft](https://microsoft.github.io/rust-guidelines/). Los lints están en `[lints]` de
`Cargo.toml` y en `clippy.toml`: sin `unwrap` fuera de tests, sin `as` que pierda datos,
sin `bool` como parámetro, toda la API pública documentada (con `# Errors`) y cada
supresión con `#[expect(..., reason = "...")]`. El código de producción tampoco usa
`expect`, `panic!` ni `unreachable!` (`src/lib.rs`): un invariante roto es
`GenerationError::Internal`. Antes de subir cambios:

```bash
cargo fmt
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
cargo test --no-default-features
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features
```

CI (`.github/workflows/ci.yml`) corre lo mismo para cada combinación de features, en
Linux y Windows, más las propiedades extendidas, el paquete y la versión mínima de Rust
declarada en `rust-version` (1.88).

## Publicar una versión

1. En un PR, subir `version` en `Cargo.toml` y pasar lo de `[Sin publicar]` de
   `CHANGELOG.md` a la nueva versión, con la fecha.
2. Mergearlo en `main` con el CI en verde.

Al llegar a `main` una versión sin tag, `.github/workflows/release.yml` verifica que
`CHANGELOG.md` tenga su sección, corre los tests, compila los ejecutables de Windows y
Linux y crea el tag `vX.Y.Z` y el release de GitHub con las notas de esa versión. Los
merges que no cambian la versión no publican nada. También se puede publicar subiendo
el tag a mano (`git push origin v0.2.0`), que tiene que coincidir con `Cargo.toml`.

## Licencia

MIT (ver `LICENSE`). `efb_dimensions.json` contiene solo la interfaz de los bloques de
la librería de Control Expert 15.1 (nombres, pines, tipos, posiciones y anchos en celdas),
necesaria para generar secciones compatibles. EcoStruxure Control Expert es una marca de
Schneider Electric; este proyecto no está afiliado a Schneider Electric.
