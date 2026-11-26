# Auditoría integral de HIPSO

**Fecha:** 26-11-2026
**Rama:** `version-0.8.2` (commit `be8209d`, versión 0.8.5)
**Alcance:** `crates/core`, `apps/gui` (incluido el asistente de IA), `apps/cli` (incluido el servidor MCP), configuración de compilación y CI.

---

## Método

- **No se modificó nada del repositorio.**
- `cargo clippy` se ejecutó con la carpeta de compilación en un directorio temporal y con `--locked`, así que no tocó ni `target/` ni `Cargo.lock`.
- Se revisó el código a mano y con cuatro revisores especializados en paralelo:
  - lectores de formatos de archivo;
  - seguridad de datos al escribir;
  - corrección numérica de ráster, terreno e hidrología;
  - revisión general.
- **Todos los hallazgos de este informe se comprobaron leyendo el código.** Los de los revisores también se verificaron antes de incluirlos.
- `cargo test --workspace`: 724 tests pasan, 0 fallan y 9 están ignorados.

---

## 1. Veredicto

El código es de buena calidad:

- El manejo de errores es disciplinado y casi todo devuelve `Result`.
- Hay muchos tests.
- Los algoritmos de terreno, hidrología y estadística son correctos.

Aun así hay problemas importantes, de más a menos grave:

1. **Un fallo crítico que puede borrar la carpeta de datos del usuario** al ejecutar un flujo.
2. **Dos caminos que destruyen o recortan archivos originales**, aunque el producto promete que "el original nunca se modifica".
3. **Errores de georreferenciación y de alturas:**
   - muestreo fuera del ráster;
   - GeoTIFF con PixelIsPoint.
4. **Lectores sin límites:** un archivo dañado puede cerrar o colgar el programa.
5. **La comprobación de clippy de la CI está rota.**

---

## 2. Revisión de la auditoría anterior

Este documento reúne las dos auditorías: la anterior y esta. Los hallazgos válidos de la anterior están incorporados en las secciones 3 y 4. Los que no se sostuvieron se explican en la tabla de abajo.

La auditoría anterior (`sequential-herding-yeti.md`) se hizo solo leyendo código, sin compilar. Acertó a medias:

| # | Lo que decía | Veredicto | Motivo |
|---|---|---|---|
| 1 | Los `.expect()` de `vector_ui.rs:424,444,453` cuelgan la interfaz | **Exagerado** | El botón "Ejecutar" no se activa sin la segunda capa ni sin el ráster (`vector_ui.rs:326-329`). Además la tarea corre dentro de `catch_unwind` (`main.rs:2269`). Lo que sí pasa es que rompen el lint del proyecto (ver 3.5). |
| 2 | El asistente de IA puede leer rutas arbitrarias | **Correcto** | `resolve()` acepta cualquier ruta y el resumen se envía al servicio externo. Riesgo medio-bajo. |
| 3 | `quantile()` falla con una lista vacía | **Correcto pero inofensivo** | El único sitio que la llama ya descarta las listas vacías. |
| 4 | El servidor MCP no limita la concurrencia ni tiene lista de herramientas permitidas | **Correcto** | Es una mejora, no un fallo. |
| 5 | `aba.rs` da NaN o infinito con cero grados de libertad | **Falso** | Todo está protegido: `moments` sale antes si n<2, AICc comprueba `n-p-1>0`, hay `if m.df() < 1.0 → Err` y `chosen.is_empty() → Err`. |
| 6 | Colisiones de `pack()` en la varita mágica | **Falso como fallo** | Si dos celdas colisionan, `around()` vuelve a filtrar por distancia real: solo se pierde velocidad. Además la varita trabaja en un radio de 100 m. |
| 7 | `len()-1` sin protección en `charts.rs` | **Falso** | Los cinco sitios tienen un `is_empty()` explícito (`charts.rs:928` y `charts.rs:1303`). |
| — | "Cero `unwrap`/`panic` en producción" | **Falso** | Clippy encuentra 5 en código de producción (ver 3.5). |

**Conclusión:** la auditoría anterior se centró en pánicos, que este código casi no tiene, y no vio los problemas importantes: pérdida de datos, georreferenciación, lectores sin límites y el lint roto.

---

## 3. Hallazgos, de más a menos grave

### 3.1 CRÍTICO: un flujo puede borrar la carpeta de datos del usuario

**Dónde:** `crates/core/src/model.rs:1215` y `crates/core/src/model.rs:1264`

**Qué pasa:**

- Al ejecutar un flujo, cada paso trabaja en `results/.flujo_<pid>/<id del paso>`.
- Cuando ningún otro paso necesita ya ese resultado, se borra con:

  ```rust
  let _ = std::fs::remove_dir_all(work.join(&src.id));
  ```

- **El id del paso no se sanea.** Solo se comprueba que no esté vacío ni repetido (`model.rs:742-746`).

**Escenario:**

- Un paso tiene el id `".."`.
- `work.join("..")` apunta entonces a la carpeta de resultados.
- Por defecto, esa carpeta es la de los propios datos de entrada (`paths.rs:33`).
- Al terminar el paso que lo consume, **se borra la carpeta completa con las nubes del usuario**.
- En Windows, un id absoluto (`C:\Users\x`) es aún peor, porque `join` sustituye la ruta entera.

**Por dónde puede llegar un id así:**

- un `.model.json` compartido por otra persona;
- `hipso run`;
- la herramienta `run` del servidor MCP;
- el asistente de IA (`ai/tools.rs:584-588`). Este caso encaja justo con la condición de borrado, porque el asistente marca como "no conservar" los pasos que consume otro paso. Bastaría una inyección de instrucciones o un id raro generado por el modelo.

**Arreglo:**

- Usar `folder_name(&n.id)`, que ya existe en `model.rs:914`, en todos los `work.join`.
- Rechazar en `problems()` y en `build_model` los ids que no cumplan `[A-Za-z0-9_-]+`.

---

### 3.2 ALTO: una salida igual a la entrada destruye el original

**Dónde:**

- `thin.rs:197`
- `reproject.rs:197` y `reproject.rs:756`
- `pipeline.rs:265` (`classify_with_context`)
- `ops::whole_cloud_job` (`apps/cli/src/main.rs:4708`)
- herramientas ráster como reclass (`main.rs:5929`)

**Qué pasa:**

- `hipso thin a.las -o a.las` crea el archivo de salida, lo que lo vacía, y después vuelve a leer la entrada, que ya está vacía.
- En Windows, Rust abre los archivos permitiendo que otro los escriba a la vez, así que el truncado no da ningún error.

**Las protecciones que existen son incompletas.** Otros comandos sí comprueban `input == output`: `edit::save`, `extract_selected`, `merge`, `propagate`, `cloudtools` y `assign_projection`. Pero esa comparación de `Path` no detecta:

- `a.las` frente a `./a.las`;
- una ruta relativa frente a una absoluta;
- mayúsculas distintas en Windows.

**Arreglo:** un único helper en `paths` que canonicalice las dos rutas, las compare sin distinguir mayúsculas en Windows, y se use desde `out_path` y `Writer::create`.

---

### 3.3 ALTO: guardar una capa shapefile o GeoJSON sobrescribe el original sin red de seguridad y recorta textos

**Dónde:** `apps/gui/src/layers.rs:1852-1878`, que llama a `crates/core/src/vector.rs:2155-2180`

**Qué pasa:**

- Ctrl-S sobre una capa `.shp` o `.geojson` escribe directamente encima del archivo original, sin archivo temporal.
- `.shp`, `.shx` y `.dbf` se vacían primero. Un fallo a mitad de escritura destruye la capa.
- **Además se pierden datos en silencio:** los campos de texto se reescriben con un ancho de 80 caracteres (`add_character_field(n, 80)` y `truncate(t, 80)`), aunque el formato DBF admite 254. Cualquier valor más largo se corta.

**Comparación:** el GeoPackage sí se guarda bien. Usa temporal y renombrado (`gpkg_write.rs:1046-1050`) y avisa antes de perder tablas.

**Arreglo:**

- Escribir en un nombre temporal hermano y renombrar cada archivo componente.
- Ampliar el campo a 254 caracteres o al ancho original.

---

### 3.4 MEDIO: `Grid::sample` devuelve valores fuera del ráster

**Dónde:** `crates/core/src/grid.rs:207-211`

```rust
c.clamp(0.0, (self.cols - 1) as f64) as usize,
r.clamp(0.0, (self.rows - 1) as f64) as usize,
```

**Qué pasa:**

- Para un punto fuera de la rejilla, `sample` debería devolver `None`.
- En cambio limita el índice al borde y devuelve el valor de la celda del borde.
- El propio código sabe que esto ocurre: `reproject.rs:703` lo comenta y lo protege, igual que `hag.rs`, `volume.rs` y `resampled_onto`.

**Sitios afectados que no lo protegen:**

- **Validar MDT** en modo bilineal (`dtmcheck.rs:280`):
  - los puntos de control fuera del MDT se cuentan con la altura del borde;
  - eso ensucia el RMSE, el NMAD y el LE95;
  - con un sistema de coordenadas que no coincide, el aviso "ningún punto de control cae sobre el MDT" nunca salta.
- **Alturas medidas contra una prolongación plana del borde:**
  - `chm.rs:45`
  - `metrics.rs:798`
  - `voxelmetrics.rs:236` (su rama `lost += 1` nunca se ejecuta)
  - `treemetrics.rs:196`
  - `trees3d.rs:329`
  - `jobs.rs:288`, `pipeline.rs:525`, `clip.rs:346`, `export.rs:156`, `corridor.rs:862` y `rules.rs:413`

**Arreglo:** una sola línea al principio de `sample`, que corrige todos los sitios que la llaman:

```rust
if self.index(x, y).is_none() { return None; }
```

---

### 3.5 MEDIO: la comprobación de clippy de la CI está rota

**Contexto:** el workspace prohíbe `unwrap`, `expect`, `panic` y `unreachable` (`Cargo.toml`, `[workspace.lints.clippy]`), y la CI ejecuta `cargo clippy --all-targets -- -D warnings`.

**Resultado de clippy (toolchain 1.99.1):** falla.

- **Código de producción que incumple la regla:**
  - `crates/core/src/cluster.rs:19`: `expect` en `Dsu::push`. Es un error duro que detiene la compilación de `hipso-core`.
  - `apps/gui/src/vector_ui.rs:369`: `unreachable!`.
  - `apps/gui/src/vector_ui.rs:424`, `:444` y `:453`: `expect`.
- **Tests:** unos 200 `unwrap` y `expect` en módulos de test (`ui_tests.rs`, `geomops.rs`, `view.rs`, `charts.rs`, `topofix.rs`, `geomedit.rs`…). No hay `clippy.toml` con `allow-unwrap-in-tests`.
- **Estilo:** 18 avisos (tipos complejos, `is_multiple_of`, `as_chunks`, `if` que se pueden unir…). Algunos son lints nuevos de esta toolchain.

**Arreglo:**

- Sustituir los 5 sitios de producción por `ok_or_else(...)?`, el patrón que ya usa `vector_ui.rs:462`.
- Añadir un `clippy.toml` con `allow-unwrap-in-tests = true` y `allow-expect-in-tests = true`.
- Corregir o permitir los 18 avisos de estilo.

---

### 3.6 MEDIO: los GeoTIFF con PixelIsPoint se leen desplazados media celda

**Dónde:** `crates/core/src/grid.rs:526-556` (`read_tiff`) y `grid.rs:488-505` (`read_extent`)

**Qué pasa:**

- El lector trata siempre el tiepoint como la esquina del píxel.
- No consulta nunca la GeoKey 1025 (GTRasterTypeGeoKey); solo la escribe, en `grid.rs:363`.
- Con PixelIsPoint (valor 2), que usan muchos MDT, el tiepoint es el centro del primer píxel.
- **Ejemplo:** tiepoint (500000, 4000000) con celdas de 1 m. La esquina real es (499999,5, 4000000,5), así que toda la rejilla queda desplazada 0,5 m al este y 0,5 m al sur.

**Comparación:** el lector JP2 sí lo tiene en cuenta (`jp2/mod.rs:190`).

**Arreglo:** leer la clave 1025 en `read_tiff` y `read_extent`. Si vale 2, usar `x0 = tx - sx/2` y `top = ty + sy/2`.

---

### 3.7 MEDIO: lectores sin límites ante archivos dañados o manipulados

Un archivo corrupto o fabricado a propósito puede **cerrar de golpe o colgar** HIPSO o la CLI.

- Un fallo de reserva de memoria aborta el proceso entero; el `catch_unwind` de la GUI no lo frena.
- El perfil release no activa `overflow-checks`, así que los desbordes de enteros no dan error: el número da la vuelta en silencio y el cálculo sigue con un valor erróneo.

| Formato | Dónde | Qué pasa con un archivo malo | Arreglo |
|---|---|---|---|
| **GeoTIFF** | `Limits::unlimited()` en `grid.rs:528`, `raster.rs:53`, `pyramid.rs:70`, `cog.rs:319` y `tiled.rs:679` | Un TIFF de unos 200 bytes que dice medir 100000×100000 en float64 intenta reservar terabytes y aborta. **Es el más importante: afecta a todo GeoTIFF que se abre.** | Mantener los límites del crate, o fijarlos según `MAX_CELLS` |
| **PNG** | `rasterfmt.rs:168` (`png::Limits { bytes: usize::MAX }`) | Una cabecera que declara una imagen gigante provoca la reserva y el aborto | Quitar `set_limits` o poner un tope |
| **JP2 (cajas)** | `jp2/mod.rs:91` (`end: (at + len).min(limit)`) | Con una longitud XLBox enorme, la suma da la vuelta a 0 y el bucle relee las mismas cajas para siempre (cuelgue) | `at.checked_add(len)` y rechazar `end <= at` |
| **JP2 (precintos)** | `jp2/tier2.rs:241`, `:313`, `:390-414` | Un número de precintos o paquetes desorbitado provoca una reserva enorme | Limitar los totales antes de `build_comp` |
| **ENVI** | `rasterfmt.rs:482-489` | `h.offset + n * nb * size` se desborda y el corte `&data[h.offset..need]` entra en pánico | `checked_mul` / `checked_add` y tope `MAX_CELLS` |
| **GeoPackage (SQLite)** | `sqlite.rs:163-192` (`leaves`) | Páginas internas que apuntan a la misma página causan una explosión exponencial (cuelgue y luego memoria agotada) | Conjunto de páginas visitadas |
| **GeoPackage (SQLite)** | `sqlite.rs:238-243` | Una cadena de overflow que se apunta a sí misma, con un tamaño declarado enorme, crece sin fin | Rechazar `size > data.len()` y limitar las iteraciones |
| **XLSX** | `xlsx.rs:158-163` y `:232-233` | La referencia `ZZZZZZZZ1` intenta crear una columna de unas 2·10¹¹ celdas | Rechazar `col >= 16384` |
| **XLSX / KMZ** | `xlsx.rs:40-81` y `kml.rs:33` | Cada parte tiene un tope de 256 MB, pero no hay tope total: una bomba zip pasa | Tope total de bytes descomprimidos |
| **COPC** | `copc.rs:297` y `:289` | Un nodo que declara 2³¹ puntos intenta reservar unos 70 GB | Rechazar `points > BATCH_POINTS` o `bytes > file_len` |
| **COPC (zoom)** | `copc_detail.rs` | Un nivel absurdo en el archivo cuelga el hilo del zoom | Limitar los niveles a ≤ 30 |
| **LAS** | `las.rs:1059` | El búfer se reserva antes de mirar el número de puntos: un registro de 65535 bytes con 0 puntos ya reserva 4,3 GB | `batch.min(count)` y tope a `record_len` |

**Arreglo de conjunto:** añadir `overflow-checks = true` en `[profile.release]`. Convierte los desbordes silenciosos en un pánico controlado, con un coste mínimo.

---

### 3.8 MEDIO: la marca de "guardado" se pone antes de guardar la nube

**Dónde:** `apps/gui/src/editing.rs:1452-1453`

**Qué pasa:**

- `self.ed.saved` y `self.ed.toggled` se actualizan antes de que la tarea de fondo ejecute `edit::save` (línea 1462).
- Si el guardado falla, la interfaz indica que no hay cambios pendientes, y el usuario puede cerrar el programa.
- **Atenuante:** el archivo de sesión solo se borra si todo se guardó bien (líneas 1466-1468), así que las ediciones se pueden recuperar.

**Arreglo:** actualizar esas marcas solo cuando la tarea termine con éxito.

---

### 3.9 MEDIO: los procesos siguen vivos al cerrar HIPSO

**Qué pasa:**

- La GUI no tiene ningún manejador de cierre (no hay `on_exit` ni `close_requested`).
- Las herramientas que se lanzan como subprocesos `hipso`, desde la ventana de herramientas (`cli_tool_ui.rs:665`) o desde Flujos, siguen gastando CPU y escribiendo archivos después de cerrar el programa.
- Cancelar una herramienta CLI mata el proceso (`model.rs:1062`) pero deja los archivos a medio escribir. Las tareas internas, en cambio, sí borran lo que hayan creado.

**Arreglo:** guardar los `Child` en marcha, matarlos al cerrar la app y limpiar las salidas parciales al cancelar.

---

### 3.10 BAJO–MEDIO: otros

- **La CLI sobrescribe sin avisar con `-o`.**
  - Dónde: `apps/cli/src/main.rs:3434-3436`.
  - Qué pasa: no existe la opción `--force`.
  - Arreglo: fallar si la salida existe, salvo con `--force`.
- **Escrituras no atómicas.** Un fallo a mitad de escritura corrompe la versión buena anterior.
  - Dónde:
    - el proyecto (`project.rs:72`);
    - las preferencias (`prefs.rs:163`, con el error ignorado);
    - los `model.json` (`flow_ui.rs:371` y `:1086`);
    - el historial del asistente (`ai/store.rs:228`);
    - `classtable.rs:55` y `gpkg_write.rs:776`.
  - Arreglo: un helper `write_atomic`, reutilizando el patrón de `edit/file.rs:273-279`.
- **La hidrología se traga los errores, incluida la cancelación.**
  - Dónde: `hydro.rs:143` (`fill_depth`), `:191` (`flow_accumulation`) y `:207` (`wetness`).
  - Qué pasa: `if let Ok(d) = drainage(...)` convierte un `Err` en una rejilla vacía, y se escribe un GeoTIFF vacío en lugar de parar.
  - Arreglo: devolver `Result` y propagar el error con `?`.
- **La reproyección trata los MDT enteros como rásteres de clases.**
  - Dónde: `reproject.rs:507-513`.
  - Qué pasa: si todos los valores muestreados son enteros, se usa vecino más próximo. Un SRTM Int16 sale con escalones.
  - Arreglo: decidir por el tipo de dato o por una opción explícita, no por los valores.
- **El asistente de IA puede leer cualquier ruta.**
  - Dónde: `ai/tools.rs:326`.
  - Qué pasa: el modelo puede pedir el resumen de archivos `.csv` o `.txt` arbitrarios del equipo, que se envía al servicio externo.
  - Arreglo: limitarlo a las capas abiertas.
- **La clave de la API de IA se guarda en texto plano en Windows.**
  - Dónde: `ai/config.rs:191-193`.
  - Qué pasa: el permiso 0600 solo se aplica en Unix. La clave se envía también a una URL personalizada `http://` sin cifrar.
  - Arreglo: usar DPAPI o el llavero del sistema, o al menos avisar si la URL no es https.
- **El servidor MCP no limita los procesos concurrentes.**
  - Dónde: `mcp.rs:297-308`.
  - Qué pasa: cada `tools/call` lanza un proceso `hipso` sin tope.
- **Desviación típica inconsistente.**
  - Qué pasa: `rasterproc.rs:308` usa la poblacional (`/n`); el resto del programa usa la muestral (`/n-1`).
- **Escritor JSON del asistente.**
  - Dónde: `ai/jsonw.rs`.
  - Qué pasa: un número `NaN` o infinito se escribiría como `NaN`/`inf`, que no es JSON válido. Hoy no ocurre en la práctica.
- **`quantile()` sin protección propia ante una lista vacía** (de la auditoría anterior).
  - Dónde: `ai/tools.rs:420`.
  - Qué pasa: `sorted.len() - 1` sobre una lista vacía daría la vuelta y fallaría. Hoy no ocurre, porque el único sitio que la llama descarta las listas vacías.
  - Arreglo: añadir al principio `if sorted.is_empty() { return f64::NAN; }`.
- **Clave de celda de la varita mágica con 21 bits por eje** (de la auditoría anterior, solo informativo).
  - Dónde: `wand.rs:113-116`.
  - Qué pasa: a partir de ±10⁶ celdas dos celdas distintas comparten clave. No da resultados erróneos, porque `around()` vuelve a filtrar por distancia; solo se pierde velocidad.
  - Arreglo: basta un comentario que documente el límite.

---

## 4. Comprobado y correcto

- **Georreferenciación:** `col_row`, `index`, `center` y `sample` son coherentes (fila 0 al norte, centros en +0,5). El tiepoint de escritura coincide con el de lectura. `.asc` con `xllcenter` resta bien media celda.
- **Terreno:**
  - Horn, pendiente en grados y porcentaje, orientación y sombreado correctos.
  - Zevenbergen-Thorne con signos correctos.
  - TPI con sumas acumuladas.
- **Hidrología:** Priority-Flood sin ciclos, TWI con área específica `acc*cell`, numeración de subcuencas correcta.
- **Estadística:**
  - percentiles de tipo 7 (Hyndman-Fan) en todo el programa;
  - NMAD con 1,4826;
  - R² correctos según el contexto;
  - Shapiro-Wilk y Royston;
  - Breusch-Pagan con un solo regresor;
  - guardas de grados de libertad en `aba.rs`.
- **NaN:** todas las ordenaciones usan `total_cmp`; NaN se filtra antes de min, max y media.
- **JSON:** pares sustitutos bien manejados y profundidad limitada (`MAX_DEPTH`).
- **Lectores robustos:** KML, WKB de GeoPackage, PLY, E57, cabeceras LAS y VLR, jerarquía COPC, codestream JP2, shapefile y `.asc`.
- **Escrituras seguras:** archivo de ediciones y sesión, configuración de IA, caché COPC, copias CRS temporales, carpeta de trabajo de los flujos (salvo 3.1), nombres por defecto con `paths::unique` y guardado de GeoPackage.
- **Concurrencia:** las herramientas CLI tienen cada una su propio `cancel` y no chocan con el `CANCEL` global. Las tareas internas son de una en una (`running: Option`).
- **Pánicos en tareas de fondo:** se capturan con `catch_unwind`, y el perfil release usa unwind, no abort.
- **Comprobaciones de la auditoría anterior, confirmadas:**
  - `tls::eigen` ordena los valores propios de mayor a menor (`tls.rs:230`), así que la planaridad `(l2−l3)/l1` de la varita es correcta;
  - el `unreachable!()` de `vector_ui.rs:369` es realmente inalcanzable, aunque incumple el lint;
  - `dtmcheck::accuracy` y `geomedit::simplify_ring/line` tienen protección ante listas vacías;
  - `rastercalc.rs:559-560`: dividir entre cero o calcular el módulo de cero da NaN, no un pánico;
  - `tls.rs:1361`: el `last.remove(0)` está protegido por `len() > 3`;
  - el escritor `quote` escapa bien los caracteres de control.

---

## 5. Prioridad recomendada

1. **3.1:** sanear los ids de los nodos del flujo. Es la única pérdida de datos masiva.
2. **3.2 y 3.3:** proteger los originales (salida = entrada; guardar shapefile).
3. **3.4 y 3.6:** una o dos líneas cada uno; corrigen alturas y desplazamientos en muchos productos.
4. **3.7:** quitar `Limits::unlimited()` en los 6 sitios de TIFF y PNG, y activar `overflow-checks`.
5. **3.5:** dejar la CI de clippy en verde.
6. **3.8–3.10:** según disponibilidad.

---

## 6. Pendiente

- **`cargo test --workspace`:** terminado. **724 tests pasan, 0 fallan y 9 están ignorados** (marcados `#[ignore]`; uno de ellos, por ejemplo, necesita una clave real del servicio de IA). Los fallos de este informe no los detecta ningún test actual.
- **No auditado en profundidad:**
  - la interfaz de edición de nubes en 3D;
  - los diálogos de guardado de cada herramienta de la GUI. Los diálogos nativos piden confirmación antes de sobrescribir, pero no impiden elegir el archivo de entrada como salida.
