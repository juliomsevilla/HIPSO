# Plan: clasificador Random Forest para nubes de puntos (CLI + visor), versión mínima

Documento de trabajo para quien vaya a construirlo, en este u otro equipo. **Estado: sin empezar** (solo análisis y diseño). El análisis de TerraScan y de las referencias ya está hecho y resumido más abajo; no hace falta repetirlo.

## Cómo retomarlo

- **Rama:** `version-0.8.2`. Compilar con `cargo build --release -p hipso-cli -p hipso-gui` (en Windows, con `~/.cargo/bin` en el `PATH`); cerrar antes cualquier `hipso-gui.exe` abierto, o la compilación falla con «Acceso denegado».
- **Primer paso:** el paso 1 de la tabla de fases es una **puerta de decisión**: medir tiempo y memoria de las características en una hoja real antes de escribir nada más. Si el coste por escala es excesivo, rehacer con la búsqueda anidada (ver «Riesgos»). Se recomienda además un experimento previo de unos 2 días, sin CLI ni ventanas: características + el bosque que ya existe, entrenado y probado dentro de una nube ya clasificada, y compararlo con `classify`. Si no supera de forma clara a `classify` en alguna clase que importe, descartar el resto.
- **Convenciones del repositorio** (las mismas de `PLAN_EDITOR.md`): commits sin atribución de ninguna herramienta, a nombre del autor; no ejecutar `cargo fmt --all` (el repositorio no está formateado con rustfmt); los finales de línea están mezclados (CRLF y LF), las herramientas de edición por patrón de varias líneas pueden fallar con CRLF; no crear carpetas ni binarios duplicados.
- **Pruebas que obligan a registrar lo nuevo:** `apps/cli/src/flow.rs` exige que toda orden esté en el catálogo de flujos (`crates/core/src/model.rs`: `CATALOG` y `ARG_KINDS`) y `apps/gui/src/tips.rs` exige un texto de ayuda para todo botón de cinta o de panel (también los de `ribbon::small_button` de las ventanas).
- **Patrón de ventanas LiDAR a copiar:** *Propagar clase* (`ToolId::Propagate`, `propagate_form` y su rama en `build_lidar`, en `apps/gui/src/tools.rs` y `apps/gui/src/tools/lidar_ui.rs`) y *Editar con capas*. El resultado se abre solo en el mapa si el informe termina en ` -> ruta`.
- **Licencias:** TerraScan es comercial y 3DMASC/CloudCompare y `lidar_platform` son GPL: se usa solo el método, nunca su código, nombres de ficheros ni la sintaxis exacta de sus configuraciones. HIPSO es freeware propietario.

## Resumen del análisis previo

- **TerraScan** (guía de usuario, v. 29/06/2026, apartados 4.3.6, 5.1.2 y 5.2) no clasifica puntos con ML, sino **grupos** de puntos (planos, copas, densidad, polígonos), con *Decision forest* o *Histogram gradient boosting*, validación, tope por clase y *Add best parameters*. Solo hay modelos de biblioteca para tres especies; no hay deep learning; los propios autores dicen que lo de grupos «sigue en desarrollo».
- **3DMASC** (plugin `q3DMASC` de CloudCompare; Letard et al., arXiv 2401.09481, <https://lidar.univ-rennes.fr/en/3dmasc>) clasifica por punto con atributos de vecindario multiescala y un bosque aleatorio, calculados en **puntos núcleo**. Vecindarios esféricos (la escala es el diámetro); se busca una vez a la escala mayor y las menores recortan la lista. Confianza = fracción de votos del bosque. Sin balance de clases, sin tratar valores faltantes y con validación aleatoria sin semilla: de ahí vienen las mejoras de este plan.
- **lidar_platform** (CNRS, Rennes) es sobre todo un envoltorio de herramientas externas; lo aprovechable es la **selección automática de atributos y escalas** (información mutua, filtro de correlación ≥ 0,85, poda iterativa por importancia del bosque) y el **informe de exactitud frente a cobertura por umbral de confianza**. Ambas quedan fuera de este plan (ver «Fuera por ahora»).
- **Por grupos o por punto:** un punto aislado no tiene información; hace falta su vecindario. Se descartó empezar por grupos (exige segmentar bien, coste mayor de desarrollo) y se eligió el esquema de núcleos y propagación, que aprovecha piezas que HIPSO ya tiene.

## Contexto del diseño

Se pidió estimar el coste de clasificar con machine learning, partiendo del manual de TerraScan, y se acotó: **solo el Random Forest, lo más simple y barato posible, pero con CLI y visor**; las demás mejoras de HIPSO quedan fuera por ahora.

Lo aprendido de las referencias (solo método, sin copiar código GPL de 3DMASC ni de lidar_platform): el bosque se alimenta con características del vecindario del punto a varias escalas, se calculan solo en una submuestra de **puntos núcleo** y se propagan al resto; el modelo se entrena con una nube ya clasificada y se aplica a otras con una **confianza** (fracción de votos). TerraScan hace lo mismo por grupos de puntos; aquí se evita segmentar.

**Hallazgo que lo abarata mucho:** HIPSO ya tiene un bosque aleatorio completo en `crates/core/src/obia/classify.rs` (`Forest::train` sobre una `AttrTable` de columnas `f32` con NaN → valor de relleno, bootstrap, `predict` que devuelve clase y confianza, matriz de confusión out-of-bag, kappa e importancia de atributos, ya paralelo en la predicción). Solo le faltan: guardarse en un archivo y alimentarse de puntos en vez de objetos ráster. **No hace falta ninguna dependencia nueva** (se descarta `smartcore`).

Piezas reutilizables ya existentes: `features::compute` (`features.rs:41`: recuento, linealidad, planaridad, esfericidad, pendiente, rugosidad con rejilla hash y rayon), `extras::ground_surface_pub` (altura sobre el suelo), `Tiled::process` y `Tile {points, records, own}` (`tile.rs`) para recorrer nubes grandes por teselas con margen, el patrón de `pipeline::classify_with_context` (`pipeline.rs:249`) para leer–clasificar–escribir con `las::set_class`, `propagate::Grid` (`propagate.rs`, cuadrícula de puntos con búsqueda del más cercano), `json.rs` (`Json::parse`, `quote`) para el archivo del modelo, y el patrón de órdenes y ventanas LiDAR (`model.rs`, `tools/lidar_ui.rs`, como *Propagar clase*).

## Diseño mínimo

Un módulo nuevo `crates/core/src/mlclass.rs` y dos órdenes:

1. **Matriz de una tesela** `tile_matrix(points, own, spec) -> (índices de núcleo, AttrTable)`: núcleo = un punto propio no ruido por celda de `core_cell` (0,5 m por defecto). Columnas: para cada escala de `radii` (por defecto 0,5 · 1 · 2 m) las 6 características de `features::compute` (NaN si el vecindario es pequeño) + **atributos de altura del vecindario** (z relativa al núcleo: mínima, máxima, media, desviación, rango y percentiles 10 y 90; se calculan con los mismos vecinos que ya recorre `features::compute`, ampliando su `Features`) + altura sobre el suelo (opcional; NaN si la tesela no tiene suelo clasificado) + intensidad + razón de retorno (número de retorno / nº de retornos). ≈ 40 columnas con nombre (3 escalas × 13 + altura sobre el suelo + intensidad + razón de retorno); el modelo guarda la `FeatureSpec`.
2. **Entrenar** (`train`): recorre la nube etiquetada **por teselas** con `Tiled::process`, saca las filas de núcleos (etiqueta = clase del punto núcleo, ignorando ruido y retenidos) y las acumula con un **tope por clase** (muestreo determinista por semilla) para equilibrar clases; entrena con `Forest::train`; **reserva un porcentaje de teselas (20 % por defecto, elegidas con la semilla) que no se usan para entrenar** y evalúa el modelo sobre ellas; devuelve un informe de texto con acierto, exhaustividad y matriz de confusión por clase y kappa sobre esas teselas (el out-of-bag de los núcleos sale demasiado optimista porque los núcleos vecinos se parecen) e importancia de atributos.
3. **Modelo en archivo**: `Forest::to_json/from_json` (nodos como matrices planas por árbol; se añaden en `obia/classify.rs`, que ya tiene los campos privados) y `Model {spec, classes ASPRS ↔ códigos del bosque, forest}` en un único JSON versionado (rechaza versiones nuevas, como `edit/file.rs`).
4. **Clasificar** (`classify_model`): igual que `classify_with_context`, pero por tesela calcula la matriz de núcleos, predice con confianza y **propaga** la clase de cada núcleo a los puntos propios cercanos con `propagate::Grid` (radio = 1,5 × `core_cell`; los puntos sin núcleo cercano conservan su clase). Opciones: clases de origen que se pueden cambiar y **marcado de dudosos**: los puntos con confianza menor que `min_confidence` no reciben la clase predicha sino la clase de dudosos (`--doubtful-class`, 1 «sin clasificar» por defecto; `none` los deja con su clase); el informe cuenta cuántos hay por clase predicha. En el visor se revisan con el filtro de clases y las herramientas de edición ya existentes (no hace falta un modo nuevo).

Órdenes (todas en español en la ayuda): `hipso train-classifier INPUT -o modelo.json [--trees 100] [--max-per-class 20000] [--radii 0.5,1,2] [--core-cell 0.5] [--no-hag] [--seed 1]` y `hipso classify-model INPUTS --model modelo.json [--min-confidence 0.6] [--doubtful-class 1|none] [--from-classes …] [-o salida]`, más las opciones de teselas habituales (`--tile`, `--buffer`) si se pueden reutilizar de `classify`.

Visor: dos ventanas LiDAR con el mismo estilo que *Propagar clase* / *Comparar nubes* (en LiDAR → Preparar, `ToolId::TrainModel` y `ToolId::ApplyModel`, formularios en `tools/lidar_ui.rs`): *Entrenar clasificador* (nube etiquetada cargada, árboles, tope por clase, escalas, usar altura sobre el suelo, «Guardar modelo como…»; el informe sale en el registro) y *Clasificar con modelo* (selector de modelo con «…», confianza mínima, clase de dudosos, clases de origen; el resultado se abre solo en el mapa con ` -> ruta`).

## Fases y coste

| Paso | Qué | Días |
|---|---|---|
| 1 | `tile_matrix` + `FeatureSpec` + estadísticos de z del vecindario + prueba con nube sintética; **medida de tiempo y memoria** por tesela en `20200207.laz` (puerta: si cada escala cuesta demasiado se rehace con la búsqueda anidada) | 2–2,5 |
| 2 | `Forest::to_json/from_json` + `Model` + pruebas de ida y vuelta | 1 |
| 3 | `train` por teselas con tope por clase, **teselas de validación reservadas** e informe | 2 |
| 4 | `classify_model` por teselas + propagación + confianza mínima + marcado de dudosos | 2 |
| 5 | CLI `train-classifier` y `classify-model`, `CATALOG`/`ARG_KINDS`, `docs/CLI.md` y manual | 1 |
| 6 | Visor: dos ventanas, iconos y textos de ayuda, prueba de interfaz | 1,5–2 |
| 7 | Prueba con datos reales (entrenar con una zona corregida, aplicar al resto, comparar con `classify`) y ajustes | 2 |
| **Total** | | **≈ 12–13 días** (≈ 10 sin la ventana) |

Se incluyen la validación sobre teselas reservadas, los atributos de altura del vecindario y el marcado de dudosos. Fuera por ahora: dependencias nuevas (`smartcore`), selección automática de atributos, grupos al estilo TerraScan, atributos de contexto, gradient boosting, ONNX, gráficas y matriz de confusión en el visor, color por confianza, lectura de modelos de 3DMASC y todas las demás mejoras que se vieron en las referencias.

## Riesgos

- **Calidad:** con unas 40 columnas de geometría, altura e intensidad el acierto aún puede quedarse corto en algunas clases (cables, postes, vegetación baja); se añaden columnas si hace falta (no cambia el diseño).
- **Coste de las escalas:** `features::compute` construye su rejilla en cada llamada (una por escala). Con 3 escalas se asume ×3; si la medida del paso 1 lo desaconseja, se calculan las tres en una pasada con el radio mayor.
- **Memoria y tiempo del entrenamiento:** `Forest::train` es secuencial y las filas son `f32` en fila; el tope por clase (20 000 × clases) lo mantiene en cientos de miles de filas.
- **Desequilibrio y objetos finos:** el tope por clase equilibra; la propagación desde núcleos puede perder cables y postes, así que `core_cell` ajustable.
- **Generalización:** el modelo guarda la densidad con que se entrenó y avisa si la nube nueva difiere mucho; la intensidad cruda no es comparable entre vuelos (opción `--no-intensity` si hace falta).
- **Reproducibilidad:** semilla fija y orden de teselas determinista.

## Archivos

- Nuevo: `crates/core/src/mlclass.rs` (registrar en `lib.rs`).
- Modificar: `crates/core/src/obia/classify.rs` (serializar `Forest`), `crates/core/src/propagate.rs` (exponer `Grid`), `crates/core/src/model.rs` (`CATALOG`, `ARG_KINDS`), `apps/cli/src/main.rs` (dos órdenes), `apps/gui/src/tools.rs` y `apps/gui/src/tools/lidar_ui.rs` (dos ventanas), `apps/gui/src/main.rs` (botones, menú y buscador), `apps/gui/src/ribbon.rs` (iconos), `apps/gui/src/tips.rs`, `docs/CLI.md`, `docs/MANUAL.md`, `scripts/gen_cli_docs.py`, `docs/PLAN_EDITOR.md`.

## Verificación

1. Unitarias: matriz de una nube sintética (suelo plano + caja) con valores esperados (planaridad alta en el suelo, altura sobre el suelo de la caja); ida y vuelta del modelo JSON con la misma predicción; rechazo de versión nueva; determinismo con semilla; propagación núcleo → puntos.
2. Integración (`crates/core/tests`): LAS sintético con dos clases separables entrenado y aplicado → acierto > 95 %; con `min_confidence = 1.0` todos los puntos pasan a la clase de dudosos (y con `--doubtful-class none` no cambia nada); el original no se modifica.
3. CLI: `train-classifier` + `classify-model` extremo a extremo sobre una nube sintética y que `apps/cli/src/flow.rs` acepte las órdenes nuevas.
4. Visor: prueba de interfaz de las dos ventanas (como las de *Propagar clase*) y, a mano con `20200207.laz`, entrenar con una zona ya corregida con el editor, aplicar al resto y comparar con `classify`.
5. Validación: el informe de `train-classifier` usa teselas reservadas (comprobar que no se mezclan con las de entrenamiento y que el resultado es reproducible con la misma semilla).
6. Rendimiento: puntos por segundo y memoria por tesela, antes de pasar del paso 1.
