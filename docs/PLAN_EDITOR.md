# Plan: llevar la edición de nubes de HIPSO al nivel de TerraScan

Documento de trabajo para quien vaya a construir las funciones. Resume qué tiene TerraScan que merece la pena traer a HIPSO, qué no, en qué orden y cómo hacerlo dentro de la arquitectura actual. La fuente de TerraScan es su guía de usuario (versión 29/06/2026); las referencias entre paréntesis son a sus apartados.

## Estado actual (rama `version-0.8.2`)

**Hecho: E01, E02 y E03** (Fase 0) y **E07, E08 y E09** (Fase 1, sin comprobar a mano). Todo lo demás está sin empezar. Las fichas de E01–E03 de la sección 4 llevan una nota **Implementado** con lo que se hizo y en qué se apartó del diseño.

| ID | Estado | Resumen |
|---|---|---|
| E01 | Hecho, **aislar comprobado a mano** (guardar con foco y planta+3D, pendiente) | *Aislar selección* fija un foco y no crea archivos; *Volver a la nube* lo quita |
| E02 | Hecho (ya existía) | *Exportar puntos…* ya exportaba la selección; se retiró el camino viejo, se corrigieron textos y se quitó el botón duplicado del panel *Edición* (queda el de la cinta) |
| E03 | Hecho, **recuperación comprobada a mano** | Ediciones a JSON, `hipso apply-edits`, *Guardar/Cargar ediciones…*, sesión recuperable |
| E07 | Hecho, **sin comprobar a mano** | *Seleccionar con plano local*: `Region::PlaneBand` con los coeficientes del ajuste |
| E08 | Hecho, **sin comprobar a mano** | Color *Altura sobre el suelo* con `PointStore::compute_hag` |
| E09 | Hecho, **sin comprobar a mano** | Colores *Ángulo de barrido*, *Clase + intensidad* y *Elevación + intensidad*; el color pasa a un botón grande con un icono por modo |

### Comprobado a mano

- **Aislar selección** y **Volver a la nube** funcionan.
- **Recuperar** las ediciones tras cerrar funciona. La sesión se autoguarda sola tras cada edición; *Guardar ediciones…* solo exporta ese mismo JSON a un archivo propio.
- **Velocidad:** da buen resultado en `--release`.

### Qué falta comprobar a mano

1. **Foco en una nube COPC grande:** aislar un árbol, editar, *Volver a la nube* y guardar. Mirar **planta y 3D a la vez**: comparten `step_detail` con `mem::swap`.
2. **Que guardar con foco da lo esperado:** el archivo guardado debe tener lo editado dentro del foco y todo lo demás intacto (la prueba de núcleo `a_focused_edit_deletes_only_inside_the_focus_and_survives_a_file` lo cubre a nivel de archivo, no de interfaz).
3. ~~Recuperación tras un cierre inesperado~~ (comprobada, véase arriba).
4. **E07–E09:** (a) en una hoja con suelo clasificado, *Color → Altura sobre el suelo*: la copa de un árbol debe salir a su altura y la leyenda mostrar la rampa en metros; con una nube sin clase 2, el registro debe avisar; (b) con una nube PNOA, *Ángulo de barrido* debe dar azul y rojo a los dos lados de cada pasada; (c) *Seleccionar con plano local*, clic sobre suelo con ruido bajo: *Bajo* selecciona los puntos bajos y *Eliminar* los quita; con *Curva* en una vaguada sigue el terreno; (d) *Guardar ediciones…* y `hipso apply-edits` dan lo mismo que la pantalla; (e) los 13 iconos del menú de color se leen en tema claro y oscuro.
5. **Velocidad:** ya da buen resultado; se compara siempre en `--release`. Una compilación `dev` va mucho más lenta y no vale. Se quitaron dos copias de operaciones por fotograma (`profile_data.rs`, `copcdetail.rs`); los costes que quedan son la lectura de hasta ~3 millones de puntos por ventana de foco (dos si hay planta y 3D) y el guardado síncrono de la sesión tras cada edición.

### Cómo continuar en otro equipo

```
git fetch origin
git checkout version-0.8.2
cargo build --release -p hipso-gui -p hipso-cli      # el LTO tarda varios minutos
cargo test --workspace                                 # debe pasar entero
cargo run --release -p hipso-gui -- ruta/a/una/nube.laz
```

- En Windows con Git Bash puede que `cargo` no esté en el `PATH`: `export PATH="$PATH:/c/Users/<usuario>/.cargo/bin"`.
- **No hay `python` en el `PATH` de todas las máquinas.** `scripts/gen_cli_docs.py` necesita Python 3 y, en Windows, `-X utf8`; además lee `target/release/hipso`, de modo que hay que compilar antes la versión `release`.
- **No regenerar `docs/CLI.md` entero:** el archivo versionado ya difiere de lo que genera el script (orden de `dedup` y `merge-clouds`, formato de la columna de valores). Se añadió la sección de `apply-edits` a mano. Si se regenera, revisar el `git diff` y quedarse solo con lo que corresponda.
- `site/contenido/ordenes.html` dice «setenta y cinco herramientas» y ya estaba desfasado antes de este trabajo; no se ha tocado.
- El repositorio **no está formateado con `rustfmt`** (`cargo fmt --check` da cientos de diferencias ajenas). No ejecutar `cargo fmt --all`: ensuciaría el historial.
- Los finales de línea están mezclados (algunos archivos en CRLF, otros en LF; `.gitattributes` tiene `* text=auto`). Git lo normaliza; no es un problema, pero las herramientas de edición por patrón de varias líneas pueden fallar con CRLF.
- Dos pruebas obligan a registrar lo nuevo: `apps/cli/src/flow.rs` exige que **toda orden esté en el catálogo de flujos** (`crates/core/src/model.rs`, `CATALOG` y `ARG_KINDS`), y `apps/gui/src/tips.rs` exige que **todo botón de cinta o de panel tenga su texto de ayuda**.
- Commits sin atribución de ninguna herramienta, a nombre de `juliomsevilla` (véase 1.3).

### Siguiente paso recomendado

Comprobar a mano E07–E09 (véase abajo) y seguir con la Fase 1: **E04** (paso del perfil), **E05** (*close to line*), **E06** (línea quebrada), **E10** (inspeccionar punto) y **E11** (propagar la clase). Los tres primeros son cambios pequeños sobre `ProfileBox` y `Region::Side`.

## 0. Cómo usar este documento

- Cada función tiene un identificador (`E01`…). Se construyen **en el orden de la sección 5**, no en el orden numérico.
- Cada ficha dice qué se quiere, por qué merece (o no) la pena para HIPSO, el diseño propuesto, los archivos que se tocan, los criterios de aceptación y las pruebas.
- Lo marcado **(verificado)** se comprobó leyendo el código. Lo marcado **(verificar)** es una suposición: se comprueba antes de escribir código y, si resulta falsa, se corrige la ficha en este mismo documento.
- Esfuerzo: **S** 1–3 días, **M** 4–10 días, **L** 2–4 semanas, **XL** más de un mes, para una persona que conoce el código.
- Antes de empezar, `git status`: si hay cambios sin confirmar que no son del trabajo en curso, no se mezclan en sus commits.

## 1. Qué es HIPSO y qué hay que respetar

### 1.1 El foco (README, «Premisas»)

HIPSO persigue tres cosas por este orden:

1. **Rapidez.** Cada novedad se mide frente a otros programas libres.
2. **Eficiencia con nubes enormes.** Los procesos leen la nube como flujo; la memoria depende del área y de la celda, no del número de puntos; los pasos locales van por teselas en paralelo.
3. **Edición cómoda y asequible.** Seleccionar, reclasificar y borrar con pocos clics; deshacer y rehacer; resultado al instante en planta, perfil y 3D.

Y es gratuito, multiplataforma (Windows, macOS, Linux) y de Rust puro. Su terreno es la nube aérea (PNOA, dron), el inventario forestal, el terreno y las líneas eléctricas. El propio README dice que **no se ha probado con escáneres móviles ni terrestres**.

**Consecuencia para este plan:** TerraScan es un editor que carga bloques enteros en memoria y los modifica en el sitio, pensado también para CAD (Bentley, Spatix), escáner móvil, edificios y torres. HIPSO no debe copiar eso. Debe copiar **lo que hace cómodo y preciso el trabajo manual** y hacerlo con su modelo: ediciones como reglas que se aplican en flujo al guardar.

### 1.2 Arquitectura de edición actual (verificado)

- `crates/core/src/edit.rs` define `Region` (zona geométrica), `SelOp` (región + atributos + modo), `Action` (solo `Delete` y `Reclassify(u8)`), `Edit` (selección + acción + etiqueta + activada), `Session` (historial con deshacer, rehacer y activar o desactivar cada paso) y `save` / `extract_selected`.
- **Las ediciones son reglas geométricas, no listas de índices.** `edit::save(input, output, edits)` recorre el archivo en flujo, en paralelo por lotes, y decide por cada punto. El original nunca se toca: `save` rechaza `input == output`.
- Los puntos que ve el usuario son una **muestra** (`PointStore`, `stride > 1`) del archivo. Las ediciones se hacen sobre la muestra y se aplican a todos los puntos al guardar. Por eso una región tiene que ser decidible **punto a punto** sin mirar a los vecinos.
- `PointStore` (`crates/core/src/view.rs`) guarda: `x, y, z`, `class`, `intensity`, `ret`, `src`, `scan`, `file`, `rgb`, `tree`, `selected`, `deleted`. **No guarda** tiempo GPS, normales, grupo ni altura sobre el suelo.
- `apps/gui/src/copcdetail.rs` ya relee a toda densidad la ventana visible desde una copia COPC en la caché (hasta `BUDGET` = 3 millones de puntos y `MAX_SHARE` = 0,5 del área), le aplica las ediciones de la sesión (`Session::apply_to`) y le marca la selección. Las ventanas de planta y de 3D usan almacenes de detalle distintos (`detail`, `detail3`).
- El perfil es una caja recta: `ProfileBox { a, b, width }` (`view.rs:1622`). Las regiones `Profile`, `ProfileBrush` y `Side` usan `ProfileBox::local()`.
- *Aislar selección* hoy escribe un `…_sel.laz` (`editing.rs:679`), vacía las nubes (`main.rs:1901`) y abre una `Session` nueva sobre ese archivo. Ahí está el fallo que motiva `E01`.
- Las recetas (`recipe.rs`) y los flujos (`.model.json`) automatizan la **clasificación**, no la edición manual.

### 1.3 Reglas del repositorio

- Lints del workspace: `unsafe_code = forbid`; `unwrap_used`, `expect_used`, `panic`, `todo`, `unreachable` en `deny`. Errores con `Result` y `hipso_core::Error`.
- **Cada herramienta tiene orden en la línea de comandos** (`apps/cli/src/main.rs`) y la ventana es un reflejo de ella. Si algo se puede hacer con la interfaz, `hipso` debe poder hacerlo sin ella. Las pruebas del CLI construyen el esquema en un hilo de pila grande.
- Pruebas: unitarias en el propio módulo (`#[cfg(test)]` de `edit.rs`) y de integración en `crates/core/tests/`. Las rutas se comparan como `Path`, no como texto, para que pasen en Windows.
- Documentación que se actualiza con cada función visible: `docs/MANUAL.md` (apartado que corresponda), `docs/CLI.md` (se regenera con `scripts/gen_cli_docs.py`), `apps/gui/src/tips.rs` (ayuda emergente), y `README.md` si cambia lo que HIPSO dice de sí mismo. `scripts/comprobar_manual_cinta.py` comprueba el manual contra la cinta.
- Textos de la interfaz, comentarios y mensajes de commit **en español**, en el estilo del repositorio.
- **Commits y PR sin atribución.** Nada de `Co-Authored-By`, ni «Generated with…», ni mención a herramientas de IA. Todo figura como hecho por `juliomsevilla` (config local `user.name = juliomsevilla`, `user.email = juliomsevilla@users.noreply.github.com`). No se escribe ningún correo personal.
- Un commit por paso lógico: núcleo, CLI, GUI, documentación, como en el historial reciente.

### 1.4 Definición de «hecho» para cualquier ficha

1. Compila en las tres plataformas objetivo (al menos `cargo build` y `cargo clippy --workspace --all-targets` limpios aquí).
2. Pruebas del núcleo que fallan sin el cambio y pasan con él.
3. Si es una acción de edición: se comprueba que **`save` sobre el archivo original da lo mismo que la vista previa** (la propiedad de la que depende todo el modelo).
4. Orden de `hipso` equivalente, con prueba.
5. Manual, `CLI.md` y tip actualizados.
6. Medida de rendimiento si toca el camino de lectura o de guardado (premisa 1 y 2): tiempo y memoria antes y después sobre una nube grande de `crates/core/tests/data` o una propia.

## 2. Criterio de decisión

Una función de TerraScan **se hace** si cumple las tres:

- Sirve a lo que HIPSO ya hace (nube aérea, terreno, bosque, líneas eléctricas).
- Cabe en el modelo de reglas en flujo (o lo amplía de forma limpia).
- Reduce clics o errores en la edición, o aprovecha una capacidad que HIPSO ya tiene y TerraScan no.

**Se descarta** si exige cargar la nube entera, depende de CAD, de trayectorias, de escáner móvil o terrestre, o repite algo que HIPSO ya tiene.

## 3. Resumen y veredicto

Prioridad: **P0** cimiento, **P1** alto retorno, **P2** merece la pena, **P3** solo con demanda concreta.

| ID | Función | Esfuerzo | Prioridad | Veredicto |
|---|---|---|---|---|
| E01 | Foco de vista (aislar sin archivo) y *Volver a la nube* | M | P0 | Hacer |
| E02 | *Exportar selección* como botón aparte | S | P0 | Hacer (con E01) |
| E03 | Sesión de edición guardable, recuperable y reaplicable | S–M | P0 | Hacer |
| E04 | Paso de avance del perfil configurable y atajos | S | P1 | Hacer |
| E05 | *Close to line*: sobre, cerca y bajo una línea de una vez | S | P1 | Hacer |
| E06 | Línea de varios vértices en *sobre/bajo una línea* | S | P1 | Hacer |
| E07 | Reclasificar por plano local ajustado (ruido, vegetación sobre suelo, añadir al suelo) | M | P1 | Hacer |
| E08 | Colorear por altura sobre el suelo | M | P1 | Hacer |
| E09 | Colorear por ángulo de barrido y por combinaciones (clase+intensidad, elevación+intensidad) | S | P1 | Hacer |
| E10 | *Inspect Point*: atributos del punto bajo el cursor | S–M | P1 | Hacer |
| E11 | Propagar la clase a toda la nube desde una nube adelgazada (*Closeby points*) | M | P1 | Hacer |
| E12 | Marcar *withheld* en vez de borrar | S–M | P2 | Hacer |
| E13 | Perfil a lo largo de una polilínea (eje curvo) | L | P2 | Hacer |
| E14 | Cursor sincronizado entre planta, perfil y 3D | M | P2 | Hacer |
| E15 | Perfil perpendicular con un clic (*Cut Section*) | S | P2 | Hacer |
| E16 | Rebanada horizontal (*Draw Horizontal Section*) | S | P2 | Hacer si falta (verificar) |
| E17 | Clasificar con una capa de polígonos o de líneas | S–M | P2 | Hacer |
| E18 | Revisar individuos uno a uno (*Inspect Groups* sobre `treeID`) | M | P2 | Hacer |
| E19 | Atajos de teclado configurables y paleta de comandos | M | P2 | Hacer |
| E20 | Acciones nuevas en el modelo: `SetZ`, `Add` | L | P3 | Solo si se hace E21 o E22 |
| E21 | Fijar cota (*Fix Elevation*, agua plana) | M | P3 | Hacer después de E20 |
| E22 | Puntos sintéticos | M | P3 | Solo con demanda |
| E23 | Informe de control con puntos de control (GCP) | M | P3 | Hacer, es QA clásico |
| E24 | *Model/Contour keypoints* | M | P3 | Solo con demanda |
| E25 | Sección de plano (*Draw Plane Section*) | M | P3 | Solo con demanda |
| E26 | Suavizar puntos (*Smoothen*) | M | P3 | Solo con demanda |

Descartes razonados en la sección 6.

## 4. Fichas

### E01. Foco de vista (aislar sin crear archivo) — P0, M

**Implementado.** Diferencias con el diseño de abajo:
- **La selección del foco se guarda como `Region::Within(Vec<SelOp>)`**, una región nueva que anida una selección entera (`edit.rs`). Toda edición hecha con foco lleva al final una intersección con esa región (`edit::focus_op`, `editing::with_focus`), de modo que `save` y la vista previa limitan igual la edición al foco, lo vea el usuario o no. Así un lazo sobre un árbol aislado no borra a los vecinos ocultos.
- **El foco guarda la forma de la selección, no sus filtros de atributos** (`edit::geometry_of`). Si dependiera de la clase, un punto reclasificado dentro del foco saldría de él. Se documenta en el manual.
- **Ocultar lo de fuera** usa un campo nuevo `PointStore::outside: Vec<bool>` que consulta `Style::visible`; `deleted` no se toca porque `Session::replay` lo borra.
- **La ventana de detalle del foco** se pide con `px = 0.0` (el nivel más denso que cabe en `BUDGET`) y se reconoce por eso al llegar el resultado (`copcdetail.rs`, variable `stale`). Se dibuja aunque la vista sea mayor que ella. Sin copia COPC o con `stride == 1` el foco solo oculta.
- *Aislar* con foco ya puesto acota más (intersección de los dos). Ya no se niega a trabajar con cambios sin guardar.
- `histo.rs` y `profile_data.rs` también respetan el foco. `drop_cloud_in_memory` y la recarga lo reinician.
- Pruebas de núcleo en `edit.rs`: `a_focus_limits_what_an_edit_can_reach`, `a_focus_keeps_the_shape_of_a_selection_and_drops_its_filters`, `a_focused_edit_deletes_only_inside_the_focus_and_survives_a_file`.

**Qué.** *Aislar selección* deja de escribir `…_sel.laz` y de abrir otra nube. Fija un **foco**: la vista muestra solo los puntos de la selección, a toda su densidad, y todas las ediciones siguen en la sesión de la nube original. *Volver a la nube* quita el foco. *Guardar* escribe la nube entera editada en un archivo nuevo.

**Por qué merece la pena.** Hoy la edición hecha sobre lo aislado no llega a la nube completa y el usuario acaba con un archivo huérfano. TerraScan no extrae trozos a archivos: usa el *Displayset* (4.2.6) y la valla, y *Save points* escribe el bloque origen. Y es coherente con HIPSO: las ediciones ya son reglas geométricas que valen para todo el archivo, y la lectura a detalle ya existe.

**Diseño.**
- `Focus { ops: Vec<SelOp>, bbox: [f64; 4] }` en `Editing` o en `App`.
- *Aislar*: calcular la caja de los puntos seleccionados, guardar `ops` y `bbox`, **no** llamar a `extract_selected`, **no** llamar a `drop_cloud`, **no** crear `Session`.
- Detalle forzado: en `step_detail` (`copcdetail.rs:105`) si hay foco, usar la caja del foco como ventana y saltarse `MAX_SHARE` y el criterio de zoom. Debe valer para `detail` y `detail3`. Si el recuento supera `BUDGET`, avisar con una línea en el registro y dibujar lo que quepa (no fallar).
- Ocultar lo de fuera: con foco, no dibujar el almacén base y dibujar del almacén de detalle solo los puntos que cumplen `Focus::ops` (`edit::selection(st, &ops)`), recalculado cuando cambie `edit_rev`.
- Las ediciones nuevas siguen entrando por `Session::apply`. `Session::apply_to` ya las aplica al almacén de detalle.
- *Volver a la nube*: `focus = None`, vaciar el detalle forzado. Botón visible mientras haya foco, junto al aviso «Foco activo: N puntos».
- Con foco, *Guardar* no cambia.
- Sin copia COPC o con `stride == 1` (nube ya entera en memoria): el foco se aplica solo como filtro de visibilidad, sin detalle. Con nubes que no son LAS/LAZ, `detail_src` queda vacío: el foco sigue siendo solo un filtro.

**Archivos.** `apps/gui/src/editing.rs` (`isolate_selection`, `selection_inputs`), `apps/gui/src/main.rs` (retirar el camino `isolate` de la línea 1901 y el campo `isolate`), `apps/gui/src/copcdetail.rs`, `apps/gui/src/tips.rs`, `docs/MANUAL.md` (apartado «Aislar selección»).

**Aceptación.**
1. Aislar un árbol de una nube TLS grande no crea archivos nuevos y no cambia la lista de nubes.
2. Eliminar ramas en el foco, *Volver a la nube* y guardar da un LAZ de la nube entera sin esas ramas y con el resto intacto.
3. Deshacer y el historial siguen funcionando igual dentro y fuera del foco.
4. Con una nube sin COPC el botón funciona como filtro y lo dice.

**Pruebas.** Núcleo: sobre una nube de prueba, una `Edit` con región `Plan` aplicada a la muestra y a `edit::save` selecciona los mismos puntos. GUI: se comprueba a mano (anotar los pasos en el commit).

**Riesgo.** Los dos almacenes de detalle comparten `step_detail` con `mem::swap`; hay que probar planta y 3D a la vez.

### E02. *Exportar selección* aparte — P0, S

**Implementado: casi nada que construir.** *Exportar puntos…* ya trabaja sobre la selección: con puntos seleccionados, las herramientas de LiDAR escriben los `…_sel` y ejecutan el proceso sobre ellos (`apps/gui/src/main.rs`, bloque de `selection_inputs`). Solo se retiró el camino viejo de *Aislar* (campo `App::isolate` y su bloque) y se corrigieron el manual, el README y las ayudas. `edit::extract_selected` se conserva porque esa ruta lo usa. Después se quitó el botón *Exportar puntos…* del panel *Edición*, que repetía el de la cinta y ocupaba espacio.

**Qué (diseño original).** El comportamiento actual de *Aislar* pasa a un botón *Exportar selección…* que pide el archivo de destino (hoy lo decide el programa: `results_dir(c)/…_sel.ext`). Reutiliza `edit::extract_selected` sin cambios.

**Por qué.** La exportación es una necesidad legítima (entregar un árbol, un objeto) pero no debe confundirse con el foco.

**CLI.** Comprobar si ya hay orden equivalente (`hipso export` con recorte); si no, `hipso extract-selection` o similar sobre `SelOp` serializadas (ver E03).

**Archivos.** `apps/gui/src/editing.rs`, `apps/gui/src/tips.rs`, `docs/MANUAL.md`.

### E03. Sesión de edición guardable, recuperable y reaplicable — P0, S–M

**Implementado.**
- `crates/core/src/edit/file.rs` (módulo hijo de `edit`, porque necesita campos privados de `Brush` y `ViewCut`): `to_json`, `from_json`, `write` (temporal y cambio de nombre), `read` y `session_path`. Escritor propio con el lector `json.rs`, **sin añadir dependencias**. Formato `{"hipso":"edits","version":1,"edits":[…]}`; los límites infinitos se guardan como `f64::MAX`.
- Piezas de apoyo: `ClassSet::words/from_words`, `Projector::to_array/from_array`, `Brush::from_stamps/stamps`.
- **La sesión se guarda en la carpeta de la caché**, no junto a las nubes (`<nombre>_<hash>.edits.json`, clave: las nubes LAS/LAZ abiertas, en orden). Se reescribe tras cada edición, se borra al guardar la nube editada, al descartar o al quedar sin ediciones. Al abrir las mismas nubes el panel ofrece *Recuperar* o *Descartar*.
- `hipso apply-edits --edits sesion.json entrada.laz [-o salida]`. Registrada en el catálogo de flujos. Probada de punta a punta con un LAS sintético.
- Panel *Edición*: *Guardar ediciones…* y *Cargar ediciones…* (`Cmd::Apply`). Con ediciones de una sesión anterior sin guardar, el panel muestra *Recuperar* y *Descartar*.
- **Límite conocido:** el guardado de la sesión es síncrono en el hilo de la interfaz. Con pinceladas muy largas (miles de estampas) cada edición reescribe un JSON grande. Si se nota, pasarlo a un hilo con una sola cola de escritura.

**Qué (diseño original).** Serializar la lista de `Edit` (`SelOp` + `Action` + etiqueta + activada) a un archivo JSON junto a la nube. Tres usos:
1. **Recuperación:** autoguardado tras cada edición. Si el programa se cierra, al reabrir la nube se ofrece recuperar.
2. **Reaplicar:** cargar la misma sesión sobre otra hoja contigua.
3. **Sin interfaz:** `hipso apply-edits sesion.json entrada.laz salida.laz` ejecuta `edit::save`.

**Por qué merece la pena.** Es casi gratis por cómo está hecho el modelo (las ediciones son reglas y no índices) y encaja con las premisas: reproducible, sin interfaz y con nubes enormes. TerraScan lo cubre con macros y con copia de seguridad (4.1.37). En TerraScan las ediciones manuales no se reaplican a otra hoja; aquí sí.

**Diseño.** `serde` en los tipos de `edit.rs` (comprobar si el workspace ya usa `serde` o un JSON propio: hay `crates/core/src/json.rs`; usar lo que haya, **no añadir dependencias nuevas sin motivo**). Versión del formato en el archivo. Las coordenadas se guardan **absolutas** y no relativas al `origin` del almacén, porque el origen cambia entre nubes.

**Aceptación.** Aplicar la sesión de A sobre A por CLI da el mismo archivo que *Guardar* en la interfaz (comparar recuentos del `SaveReport` y los hashes de clase).

### E04. Paso de avance del perfil — P1, S

**Qué.** TerraScan (*Move Section*, 4.2.2.4) ofrece medio ancho, casi el ancho completo, ancho completo o paso fijo, con clic o tecla y hacia atrás.

**Estado HIPSO (verificado).** La rueda y las flechas arriba y abajo mueven la caja un ancho por paso (`main.rs:4208`).

**Diseño.** Una opción `profile_step: StepMode { Half, AlmostFull, Full, Fixed(f64) }` en *Vista → Perfil*, usada por `ProfileBox::shifted`. «Casi completo» es 0,95 del ancho: evita perder puntos por el redondeo al hacer zoom (TerraScan lo recomienda por esa razón). Atajos configurables cuando exista E19.

**Aceptación.** Con paso fijo de 0,5 m en una caja de 2 m, tres pulsaciones avanzan 1,5 m.

### E05. *Close to line* — P1, S

**Qué.** (4.2.5.7) En el perfil, un solo trazo clasifica **por encima, cerca y por debajo** de la línea, cada tramo con su clase origen y destino, y con tolerancias arriba y abajo.

**Estado HIPSO (verificado).** `Region::Side { bx, a, b, above, height }` ya da «sobre» y «bajo» con altura máxima opcional (`beside_line`, `edit.rs:133`).

**Diseño.** Región nueva `Region::Near { bx, a, b, above_tol, below_tol }` que cumple si `|z - z_línea(u)|` cae entre las tolerancias. Una orden de interfaz crea hasta tres `Edit` con una sola etiqueta compuesta. Sin cambios en `save`.

**Aceptación.** Un trazo sobre un cable clasifica el cable (cerca), la vegetación (sobre) y el suelo (bajo) en una operación y se deshace de una vez.

### E06. Línea de varios vértices en *sobre/bajo una línea* — P1, S

**Qué.** TerraScan usa para cables una curva de tres puntos (*Above/Below curve*, 4.2.8.3.7). HIPSO usa un segmento recto. Una **polilínea** con N vértices cubre ambos casos y la parábola de TerraScan es un caso particular.

**Diseño.** `Region::Side` pasa de `a, b: [f64; 2]` a `path: Vec<[f64; 2]>` en coordenadas de perfil `(u, z)`, y `beside_line` interpola el tramo que contiene `u`. Mantener lectura de sesiones antiguas con el formato de dos puntos (ver E03).

**Aceptación.** Prueba de `beside_line` con una línea quebrada.

### E07. Reclasificar por plano local — P1, M

**Implementado.** Diferencias con el diseño:
- **No es una herramienta de reclasificar sino de seleccionar** (*Seleccionar con plano local*, `Nav::SelPlane`): el clic añade a la selección y después se usa *Cambiar clase* o *Eliminar*, que ya tienen deshacer, historial y archivo de ediciones. Sirve igual para ruido, vegetación y suelo, y el usuario ve la selección antes de aplicar.
- `Region::PlaneBand { cx, cy, r, coef: [f64; 6], lo, hi }`: seis coeficientes para el plano y la curva de segundo grado (`edit::fit_surface`, mínimos cuadrados en coordenadas locales divididas por el radio). La banda es `lo <= z - superficie <= hi`, con extremos infinitos. Va en el archivo de ediciones como `plane_band`.
- El ajuste usa la ventana de detalle si el clic cae dentro de ella y el almacén de la vista si no.
- La tolerancia se da en metros o en desviaciones típicas del ajuste. **Queda sin hacer «hasta N puntos»** (el percentil que lo traduce a tolerancia); se añade si se echa en falta.
- Pruebas en `edit.rs`: plano recuperado y banda, curva frente a plano, ajuste sin puntos suficientes y banda idéntica en el archivo guardado (`a_plane_band_gives_the_same_answer_in_the_file_as_on_screen`); y la ida y vuelta por JSON en `file.rs`.

**Qué.** Fusiona tres herramientas de TerraScan: *Classify Noise* (4.2.5.9), *Remove Vegetation* (4.2.5.10) y *Add point to ground* (4.3.5.9). El usuario hace clic en un círculo; el programa ajusta un plano o una superficie curva a los puntos de ciertas clases dentro del círculo y reclasifica los puntos que quedan por encima o por debajo de él (hasta una tolerancia, hasta N puntos, o hasta k desviaciones típicas).

**Por qué merece la pena.** Es la herramienta de limpieza fina más usada en TerraScan, y HIPSO tiene filtros automáticos de ruido pero ningún ajuste local interactivo. Cabe en el modelo si el **ajuste se hace al crear la edición** y lo que se guarda es una región decidible punto a punto.

**Diseño.** `Region::PlaneBand { cx, cy, r, plane: [a, b, c], below: f64, above: f64 }`: cumple si el punto está dentro del círculo y su cota respecto a `plane` cae fuera o dentro de la banda. El ajuste (mínimos cuadrados, planar o de segundo grado) se calcula en la interfaz sobre el almacén visible (o el detalle). La sesión guarda los **coeficientes** y no los puntos, así `save` sigue siendo en flujo. Vista previa: marcar la selección antes de aplicar, como en TerraScan.
- *Hasta N puntos*: se traduce al crear la edición en una tolerancia (el percentil correspondiente) para que siga siendo determinista.

**Archivos.** `crates/core/src/edit.rs` (región y prueba), `apps/gui/src/editing.rs` (herramienta), `apps/cli` (`apply-edits` ya lo cubre vía E03).

**Aceptación.** En un suelo con ruido bajo, un clic clasifica los puntos bajos; la misma sesión por CLI da el mismo resultado.

### E08. Colorear por altura sobre el suelo — P1, M

**Implementado.** `ColorBy::HeightAboveGround` y `PointStore::compute_hag`: la rejilla (1 m, o mayor si pasaría de unos 4 millones de celdas) toma el mínimo de los puntos de clase 2 no borrados, rellena los huecos con la media de los vecinos de fuera adentro y cada punto toma el suelo por interpolación bilineal. Se guarda en `PointStore::hag` y se recalcula con `edit_rev` (`App::ensure_hag`); con otro color se suelta. El extremo de la rampa es el percentil 99 (`hag_range`, mínimo 2 m). Sin suelo, el registro lo dice y los puntos quedan en gris. CLI: `hipso render --color hag` (error si no hay suelo). Pruebas en `view.rs`.

**Qué.** Modo de color `HeightAboveGround`. Es lo primero que hace TerraScan antes de clasificar por distancia (4.3.7.18) y lo que más ayuda a editar vegetación y ruido.

**Estado HIPSO (verificado).** `ColorBy` (`view.rs:601`) no lo tiene. `hag.rs` calcula la altura con un TIN del suelo, pero como proceso a archivo.

**Diseño.** Calcular la altura a partir de los puntos de clase 2 del **propio almacén visible** (o del detalle): rejilla del mínimo local más interpolación, no un TIN completo. Es una aproximación de visualización y se dice en el tip. No se guarda en el archivo. Se recalcula cuando cambian las clases (`edit_rev`). Leyenda con rampa propia.

**Aceptación.** En una hoja con suelo clasificado, la copa de un árbol sale a su altura real con error menor que la celda de la rejilla.

**Riesgo.** Sin clase 2 no hay referencia: mostrar «sin suelo clasificado» y no inventar.

### E09. Más modos de color — P1, S

**Implementado.** Se añadieron `ScanAngle` (con signo, ±30° en los extremos), `ClassIntensity` y `ElevationIntensity` a `ColorBy`, a `--color` del CLI (`scan-angle`, `class-intensity`, `elevation-intensity`) y a la leyenda (rampas con valores). Además, el color deja el pequeño menú de *Visualización* y pasa a un **botón grande desplegable** junto a *Ventanas* (`colormodes.rs`), con un icono por modo (`assets/icons/c_*.svg`) y una ayuda por modo. **Queda fuera:** el ángulo de barrido solo existe si el archivo lo trae (`PointStore::scan`).

**Qué.** TerraScan colorea por ángulo de barrido, por clase+intensidad, elevación+intensidad, clase+profundidad, línea+intensidad, densidad y eco. Se añaden los que usan lo que `PointStore` ya guarda: `ScanAngle` (`scan`), `ClassIntensity`, `ElevationIntensity`. Se añade a `ColorBy` en `view.rs`, a la orden `--color-by` del CLI (`Plan`, `Three`, `Profile` ya lo usan) y a la leyenda.

**Fuera de este paso:** tiempo GPS, densidad local, normales (el almacén no los tiene; ver sección 6).

### E10. Inspeccionar punto — P1, S–M

**Qué.** (4.2.2.14) Al pasar el cursor, etiqueta con los atributos del punto más cercano: clase, cota, intensidad, retorno, pasada, ángulo, altura sobre el suelo si existe, `treeID`.

**Diseño.** Búsqueda del punto más cercano al cursor en la vista activa (planta, perfil o 3D). Atención al coste: en 3D hay que buscar en el espacio de pantalla. Tooltip y una línea fija en la barra de estado. Ruta: `apps/gui/src/main.rs` (dibujo de planta y perfil) y `editing.rs`.

**Aceptación.** El tooltip aparece en menos de un fotograma con 10 millones de puntos en pantalla (premisa 1).

### E11. Propagar la clase a toda la nube — P1, M

**Qué.** TerraScan lo describe como el flujo para nubes muy densas (*Closeby points*, 5.1.1.28 y «Copy result to inactive points», 4.2.1.11): adelgazar, clasificar lo adelgazado y copiar la clase de cada punto clasificado a los puntos retirados.

**Por qué es la función más alineada con el foco.** HIPSO ya tiene *Adelgazar nube* y clasifica por teselas. Esto permite clasificar con costosos algoritmos sobre un 10 % de los puntos y obtener la clasificación de todos con memoria constante.

**Diseño.** Orden `hipso propagate-class --reference clasificada.laz --target completa.laz --radius R --out salida.laz`. Por teselas: leer los puntos de referencia de la tesela y su margen, índice espacial (rejilla), y para cada punto de `target` asignar la clase del más cercano dentro de `R` o la clase por defecto. Paralelo por teselas con el mismo patrón que `classify`. Opción para elegir qué clases se propagan y la 2D/3D.

**Archivos.** Módulo nuevo en `crates/core/src` (por ejemplo `propagate.rs`), orden en `apps/cli/src/main.rs`, ventana en `apps/gui` (grupo *Preparar*).

**Aceptación.** Adelgazar a 1 de cada 10, clasificar suelo, propagar: más del 99 % de coincidencia con clasificar la nube completa en una hoja de prueba, en menos tiempo.

### E12. Marcar *withheld* en vez de borrar — P2, S–M

**Qué.** (4.3.3.7) En LAS 1.4, *Delete* puede poner el bit *withheld* en vez de quitar el punto. Eso hace reversible el borrado en el archivo entregado.

**Diseño.** `Action::Withhold`. En los formatos 0–5 los bits 5–7 del byte de clase son sintético, punto clave y *withheld*; en 6–10 está en un campo aparte. `las.rs` ya tiene `set_class`: añadir `set_flags`. En `save`, `Withhold` no descarta el registro; lo marca. Opción en el diálogo de guardado: «Borrar» o «Marcar como retenido».

**Aceptación.** `hipso` lee el archivo guardado y descarta (o conserva) los retenidos según una casilla. Prueba de ida y vuelta.

### E13. Perfil a lo largo de una polilínea — P2, L

**Qué.** (*Travel Path*, 4.2.2.7) Recorrer un eje curvo (carretera, río, línea eléctrica) con secciones de ancho y orientación constantes, sin recolocar la caja a mano.

**Por qué merece la pena.** Es la parte de TerraScan más cercana a lo que HIPSO hace con *Corredores eléctricos*. Y era la pregunta original del usuario.

**Diseño.** Generalizar `ProfileBox` a un **marco de perfil** con dos variantes: recta (la actual) y polilínea. Los dos exponen `local(x, y) -> (u, t)` y `length()`. Las regiones `Profile`, `ProfileBrush` y `Side` pasan a usar el marco. Para la polilínea, `u` es la distancia acumulada por el eje y `t` el desplazamiento lateral con signo (proyección al tramo más cercano; en un codo cóncavo hay que decidir el empate: usar el tramo de menor `|t|`).
- Avance: el «paso» del perfil (E04) recorre el eje en lugar de desplazar la caja.
- Origen del eje: una capa vectorial de líneas (el repositorio ya digitaliza y carga líneas) o un trazo con clics.
- Lectura a toda densidad de la franja curva: ampliar la lectura del perfil (`profile_data.rs`) a la envolvente del tramo visible.

**Archivos.** `crates/core/src/view.rs` (marco), `crates/core/src/edit.rs` (regiones), `apps/gui/src/profile_data.rs`, `apps/gui/src/main.rs`, `docs/MANUAL.md`.

**Riesgo principal.** Toca el perfil, el dibujo y varias regiones de selección a la vez. Hacerlo **después** de E04 y E05, con pruebas de regresión del perfil recto.

### E14. Cursor sincronizado — P2, M

**Qué.** (*Synchronize Views* / cursor, 4.2.2.9) Al mover el ratón sobre el perfil, un marcador indica el mismo punto en la planta y en 3D, y al revés.

**Diseño.** Estado `hover_world: Option<[f64; 3]>` compartido por las vistas; cada vista dibuja su marcador. En el perfil el punto se obtiene de `(u, z)` y la caja. Con E13, de `(u, z)` y el eje.

### E15. Perfil perpendicular con un clic — P2, S

**Qué.** (*Cut Section*, 4.2.2.6) En el perfil, un clic crea un nuevo perfil perpendicular en ese punto, con el mismo ancho.

**Diseño.** Nueva `ProfileBox` con `a, b` ortogonales a la actual en el punto de `u` del clic. Reutiliza la caja actual como origen para volver.

### E16. Rebanada horizontal — P2, S (verificar)

**Qué.** (*Draw Horizontal Section*, 4.2.2.2) Una planta que muestra solo una franja de cota, útil para ver troncos, postes y fachadas.

**Verificar.** HIPSO tiene filtros de vista por cota (`z_lo`, `z_hi` en `Editing`). Si el filtro de cota ya afecta al dibujado de la planta, esta función es un control deslizante con atajo y un centro/ancho, y se acaba ahí. Si no, es M.

### E17. Clasificar con capas — P2, S–M

**Qué.** (*By polygons*, 5.1.1.20; *By centerline*, 5.1.1.7) Reclasificar los puntos dentro de una capa de polígonos o a cierta distancia de una capa de líneas (con lado y rango de cotas).

**Por qué.** Es donde HIPSO tiene ventaja sobre un editor de nubes puro: ya integra capas vectoriales. *Corredores eléctricos* demuestra el patrón.

**Diseño.** Regla de clasificación por capa, en `rules.rs` (comprobar cómo se declaran hoy las reglas de `classify`) y paso nuevo de receta. Orden `hipso classify` con regla `--in-polygons capa.shp --to-class N`. El índice espacial de polígonos ya existe (`rtree.rs`).

### E18. Revisar individuos — P2, M

**Qué.** (*Inspect Groups*, 4.3.6.9) Lista de grupos; al elegir uno, las vistas se centran en él; botones para reclasificarlo o quitarlo de la lista.

**Diseño.** Sobre el atributo `tree` (`treeID`) que ya escribe *Segmentar individuos*. Lista ordenada (por número, alturas, número de puntos); al elegir uno se activa un **foco** (E01) con una `SelOp` por `treeID`. Botones configurables hasta cuatro clases, como TerraScan.

**Depende de E01.** Requiere que `SelOp`/`Attr` pueda filtrar por `treeID` (comprobar `AttrKind` en `edit.rs:184`).

### E19. Atajos configurables y paleta de comandos — P2, M

**Qué.** TerraScan permite asignar comandos a teclas y escribirlos (*key-in*, 4.7). HIPSO tiene atajos fijos (`Z`, `Y`, `A`, `I`, `Escape`, `[`, `]`).

**Diseño.** Tabla `Command → tecla` en `prefs.rs`, editable y persistente. Paleta (`Ctrl+Mayús+P`): búsqueda difusa por nombre sobre los `Action` de la interfaz. Es lo que hace cómodo recorrer un corredor largo sin ratón.

### E20. Acciones nuevas en el modelo (`SetZ`, `Add`) — P3, L

**Cuándo.** Solo si se va a construir E21 o E22. Hoy `Action` solo tiene `Delete` y `Reclassify`, y `save` solo reescribe registros.

**Diseño.** `Action::SetZ(Z)` modifica la coordenada Z del registro (reescribir con la escala y el desplazamiento de la cabecera; cuidado con el redondeo). `Action::Add(Vec<NewPoint>)` agrega puntos en el orden del archivo (al final). `Session::replay` y `apply_to` deben tratar los puntos nuevos en el almacén visible. `save` escribe primero los originales y luego los nuevos. Reescribir las cabeceras (recuento, extensiones). Ampliar las pruebas de ida y vuelta.

### E21. Fijar cota (*Fix Elevation*) — P3, M

**Qué.** (4.2.5.11) En polígonos, poner todos los puntos de ciertas clases a una cota constante o a un percentil de ellos. Típico para superficies de agua.

**Por qué.** Encaja con las herramientas de hidrología, charcas e inundación de HIPSO, pero hoy esas herramientas parten de nubes con el agua mal plana. Depende de E20.

### E22. Puntos sintéticos — P3, M

**Qué.** (4.2.5.8) Añadir puntos a mano, a lo largo de un vector o en una rejilla dentro de un polígono, con cota de la clase vecina o fija. Para rellenar huecos de suelo antes de un MDT. Depende de E20. Solo con demanda concreta: el MDT de HIPSO ya rellena huecos.

### E23. Informe de control con puntos de control — P3, M

**Qué.** (4.3.7.15) Dada una tabla de puntos de control (`id, x, y, z`), calcular la diferencia entre su cota y la de la nube (o de un MDT) con triángulo de los tres puntos de suelo más cercanos, con estadísticas (media, desviación, RMS) y tabla por punto.

**Por qué.** Es el control de calidad más pedido por quien entrega nubes a un cliente. Coherente con el grupo *Calidad*. Orden `hipso control-report`.

### E24, E25, E26 — P3

- **E24 (*Model/Contour keypoints*, 5.1.1.36 y 5.1.1.29):** reducir el suelo a los puntos necesarios para un MDT con una tolerancia vertical. HIPSO ya adelgaza; solo vale si los usuarios necesitan MDT ligeros.
- **E25 (*Draw Plane Section*, 4.2.2.3):** sección de un plano ajustado a unos puntos. Útil para señales y fachadas; poco para el terreno de HIPSO.
- **E26 (*Smoothen points*, 4.3.7.9):** suavizar cotas del suelo. Puede alterar datos medidos; solo con demanda.

## 5. Orden de construcción y dependencias

```
Fase 0  (cimiento)      E01 → E02, E03
Fase 1  (edición fina)  E04, E05, E06 → E07 → E08, E09, E10 → E11
Fase 2  (navegación)    E12, E15, E16 → E13 → E14, E17, E18, E19
Fase 3  (modelo/QA)     E23 → (E20 → E21, E22) → E24..E26
```

Hitos que se pueden entregar solos:
- **v1 (Fase 0):** foco, exportar aparte, sesión recuperable.
- **v2 (Fase 1):** con esto HIPSO iguala a TerraScan en edición manual quirúrgica sobre nubes aéreas. Es el objetivo principal.
- **v3 (Fase 2):** recorridos curvos y revisión de individuos.

Orden dentro de la fase 1: E04–E06 primero (cambios pequeños en perfil y regiones, buenos para validar el flujo de trabajo del agente), E07 después (introduce una región nueva con coeficientes), E08–E10 en paralelo (lectura y dibujo), E11 cierra (es independiente y de núcleo).

Estimación total: fase 0 de 1–2 semanas; fase 1 de 6–8; fase 2 de 6–9; fase 3 de 6–10 si se hace todo.

## 6. Qué no se hace y por qué

| TerraScan | Motivo |
|---|---|
| Torres, cruces, atachments, tipos de torre, informes de línea (4.2.8) | Vectorización de torres: otro producto. `Corredores eléctricos` y la detección de cables y postes ya cubren la parte útil. |
| Edificios LOD2, parches, aristas, base de datos 3DCityDB (4.2.10) | Vectorización de edificios: otro producto, XL por cada herramienta. |
| Carriles, vías, pintura vial, curvatura de alineaciones, distancias de visibilidad (4.2.9) | Ligado al escáner móvil, que el README dice no haber probado. |
| Forma de onda (*waveform*) (4.2.11) | Pide datos que casi nadie entrega y un perfil por escáner. XL. |
| *Gaussian splats*, *Copy/Cut/Paste points* (4.2.4) | Pensado para escenas de visualización, no para edición de nubes de medición. |
| Trayectorias (4.5), `Cut overlap` por rango y por calidad (4.3.8.10) | Exige formatos propietarios (SBET, POF…) y es XL. HIPSO resuelve el solape por pasadas con ID de fuente o huecos de tiempo GPS. Revisar `overlap.rs` antes de decidir: si falta *cut by offset/scan angle*, añadirlo sin trayectorias es S. |
| Proyecto por bloques y bloqueo `.LCK` (4.4) | HIPSO ya tiene hojas, recetas, flujos y proyectos `.hipso`. No se replica la gestión de bloques. |
| Macros (5.3) | Las recetas y los flujos las sustituyen. La parte nueva y barata es E03 (reaplicar ediciones manuales). |
| Agrupación y modelos entrenables (4.3.6, 5.2) | HIPSO tiene segmentación de individuos y OBIA. Un `Train model` de aprendizaje automático es XL y sin demanda. |
| Paredes, túneles, interiores, `Window reflections`, batimetría (5.1.1.2, .40, .41, .42, .43) | Fuera de las nubes aéreas y forestales de HIPSO. |
| Atributos nuevos en el almacén: tiempo GPS, normales, grupo (4.3.7.20) | Ampliar `PointStore` encarece la memoria por punto, que es justo lo que la premisa 2 protege. Se calculan bajo demanda en la ventana de detalle si algún día hacen falta. |
| Ajuste a referencia (4.3.7.24–26), extraer color, geoide, comparar con referencia | HIPSO ya tiene ICP, *Colorear con una imagen*, cambio de referencia vertical y *Comparar nubes*. |
| Entorno CAD: niveles, celdas, vallas, `Spaccels`, `key-in` de MicroStation | Específico de CAD. La paleta de comandos de E19 cubre el equivalente útil. |

## 7. Cómo medir si valió la pena

- **Edición manual** (v2): tomar tres tareas que hoy cuestan varios clics (limpiar ruido bajo suelo, separar un cable de la vegetación, quitar ramas de un árbol TLS) y contar clics y tiempo antes y después.
- **Nubes enormes:** memoria máxima y tiempo de `apply-edits` y de `propagate-class` sobre una nube de cientos de millones de puntos; no deben crecer con el número de puntos.
- **Regresión del perfil recto** tras E13: el mismo perfil, el mismo resultado de selección en una nube de referencia.
