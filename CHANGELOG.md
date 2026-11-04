# Cambios

Formato de [Keep a Changelog](https://keepachangelog.com/es-ES/1.1.0/); las versiones
siguen [SemVer](https://semver.org/lang/es/). Mientras la versión sea `0.x`, un cambio
incompatible en la API o en el formato de entrada sube el número del medio.

## [Sin publicar]

## [0.1.0] - 2026-11-04

Primera versión publicada.

### Generación

- Grafo semántico sin coordenadas (`ProgramSpec`, `SectionSpec`, `FunctionCallSpec`) a
  secciones FBD de EcoStruxure Control Expert 15.1: XML de intercambio (`.xbd`), layout en
  celdas (`.layout.json`), vista previa (`.svg`) y `manifest.json` con el orden de ejecución.
- Validación contra la librería de bloques de Control Expert 15.1, incluida en el
  ejecutable, y catálogos JSON adicionales de EF/EFB/DFB propias (`--extra-lib`,
  `Generator::register_block`).
- Inferencia de tipos sobre todo el programa, con literales tipados y familias escalares
  (`ANY_BIT`, `ANY_INT`, `ANY_REAL`, `ANY_NUM`, `ANY_ELEMENTARY`).
- Partición por equipo, etapa y tamaño, con chequeo de consistencia entre equipos del mismo
  típico y orden de secciones por dependencias.
- Realimentaciones y fan-out resueltos con variables cuando hace falta; comentarios de red.
- Layout Sugiyama propio en enteros: capas y posición vertical por network simplex, orden
  por barycenter y ruteo ortogonal sin cables superpuestos entre salidas distintas.

### CLI

- `fbd-gen <entrada>` con `--lib`, `--extra-lib`, `--out-dir`, `--max-fanout`, `--check`,
  `--format json`, entrada por stdin (`-`) y códigos de salida estables (0 a 3).

### API Rust

- `Generator` con resultados estructurados y `GenerationError` tipado; diagnósticos con
  código estable, etapa y ubicación.
- Features `cli` y `embedded-library` para usar el crate solo como librería.

### Verificado en Control Expert 15.1

- Los ejemplos `p101_motor` y `planta_bombas` importan y compilan con 0 errores y 0
  advertencias, y el round-trip conserva posiciones, pines, cables, quiebres y textBox.

### Limitaciones conocidas

- 472 bloques de la librería no tienen ancho medido y se usa una estimación conservadora
  (se avisa en los diagnósticos).
- Las familias de arrays, referencias e IODDT todavía requieren validación en Control Expert.

[Sin publicar]: https://github.com/jcostasuarez/fbd-gen/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/jcostasuarez/fbd-gen/releases/tag/v0.1.0
