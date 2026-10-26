# EasyLiDAR · Referencia de la línea de comandos y de los formatos

Texto de referencia con todos los comandos de `easylidar`, las rutas de resultados y los formatos de salida. La guía de uso de la interfaz es `MANUAL.md`.

Herramientas forestales LiDAR (LAS/LAZ) en Rust puro para Windows, Linux y macOS. Línea de comandos e interfaz gráfica.

Ofrece un flujo de trabajo forestal con salidas compatibles con el formato de FUSION y métodos publicados (Kraus y Pfeifer, MCC, CSF, PMF, Dalponte, Silva…), implementados desde sus artículos. La premisa es procesar nubes muy grandes con poca memoria y en poco tiempo.

- **Streaming:** los pasos globales (MDT FUSION, CHM, métricas) leen la nube como flujo. La memoria depende del área, no del número de puntos.
- **Teselas en paralelo:** los pasos locales (ruido, suelo MCC/CSF/PMF/SMRF, edificios, TIN, CHM sin huecos) trabajan por teselas con búfer, una por núcleo. La memoria es núcleos × tesela.
- **LAZ:** la lectura es paralela. Si un paso necesita varias pasadas, el LAZ se descomprime una sola vez a un temporal en disco, que se borra al terminar.

## Uso

```sh
easylidar-gui                                # interfaz gráfica (acepta un .tif/.asc o nubes como argumento)
easylidar info nube.laz
easylidar classify nube.laz -o clasif.laz --noise isolated --ground csf --buildings
easylidar dtm nube.laz -o mdt.tif                                    # en streaming
easylidar dtm clasif.laz -o mdt.tif --ground classes --surface tin   # TIN de la clase 2, por teselas
easylidar chm nube.laz --dtm mdt.tif -o chm.tif --method pitfree --cell 0.5
easylidar metrics nube.laz --dtm mdt.tif -o metricas.csv --intensity --strata
easylidar trees chm.tif --method silva --crowns --crowns-shp --field inventario.txt
easylidar clip nube.laz --regions parcelas.shp --mode each          # PolyClipData
easylidar render nube.laz --view 3d -o vista.png --color class      # imagen del visor
easylidar export nube.laz -o puntos.csv --items x,y,z,class --header
```

**Ráster y vectorial.** Además de las nubes se cargan imágenes multibanda (GeoTIFF, JPEG 2000 como las bandas de Sentinel-2, PNG/JPEG con archivo de coordenadas, ENVI) como fondo, con composiciones de bandas (color natural, falso color infrarrojo…) y índices espectrales (NDVI, GNDVI, NDWI, EVI, SAVI, NDMI, NBR, VARI, ExG o una expresión propia sobre las bandas); el índice sale como capa ráster y se guarda en GeoTIFF. *Igualar rásteres* (`align`) pone varios rásteres sobre las celdas de uno de referencia, para que coincidan celda a celda; exige el mismo sistema de coordenadas. Se pueden crear capas vectoriales de puntos, líneas y áreas (*Nueva capa*), digitalizar en la capa activa, editar su tabla de atributos y guardarlas como shapefile con la proyección de la nube; las tablas de atributos de cualquier capa vectorial cargada se pueden ver y editar.

**Hojas.** Cada nube es una hoja: `classify`, `dtm`, `chm` y `metrics` procesan las que les des una a una y escriben un resultado por hoja (nunca las unen); `--mosaic` añade además un mosaico del MDT o del CHM. El MDT parte de la clase 2 que trae la nube (`--ground` prueba otro método) y MDT y CHM aceptan `--smooth` (media, mediana o gauss). Un trabajo se puede parar y se borra lo parcial.

**Parcelas.** `clip` lee las parcelas de un archivo (campo con el número, forma circular o cuadrada, radio o lados en metros) o las genera con `--sample random|systematic`; `--mode metrics` da solo la capa vectorial de parcelas con sus métricas, `--mode each` una nube por parcela. `crs` asigna a una nube sin proyección la de otra (`--from`) o un EPSG.

**Formatos de las tablas.** Las métricas por celda, las métricas por parcela y los árboles se guardan por defecto como capa vectorial (shapefile: un polígono por celda o parcela, un punto por árbol, con el sistema de coordenadas de la nube). Con `--format csv` (o `-o resultado.csv`) se escribe solo la tabla de texto, que es lo más ligero en procesos enormes, y con `--format both` las dos. En `clip` el mismo ajuste es `--metrics-format`. Los nombres de campo del DBF tienen 10 caracteres como máximo: `<nombre>_campos.txt` dice qué significa cada uno. Todos los rásteres y shapefiles que se crean se pueden cargar como capas en la interfaz (*Abrir*, o arrastrándolos), colorear por un campo y consultar con la herramienta Consultar.

**Rutas de resultados.** Si no das `-o`, cada resultado se nombra a partir de la entrada y de los ajustes (`datos_1_mdt_1m_tin.tif`, `datos_1_chm_0.5m_pitfree.tif`…) y se guarda en la misma carpeta que la entrada (no se crea ninguna carpeta `resultados`), salvo que indiques otra con `--results-dir`. Una ejecución nunca hereda la ruta de otra ni pisa un resultado anterior: si el nombre existe se usa `_2`, `_3`… Solo se sobrescribe con un `-o` explícito. `chm` y `metrics` usan, si no pasas `--dtm`, el MDT más reciente **de esa misma nube**; si no existe, avisan en vez de coger el de otra.

| Comando | Qué hace | Comparable a (solo referencia) |
|---|---|---|
| `classify` | Ruido (7), suelo (2), edificios (6) y vegetación alta (5). Escribe LAS o LAZ. | MCC-LIDAR, lidR `classify_*` |
| `dtm` | MDT en GeoTIFF por media por celda o TIN | GroundFilter + GridSurfaceCreate, lidR `rasterize_terrain` |
| `chm` | Modelo de copas: punto más alto por celda, o sin huecos | CanopyModel, lidR `pitfree` |
| `metrics` | CSV de elevación, intensidad (`--intensity`) y estratos (`--strata`) | GridMetrics |
| `trees` | Segmentación de árboles: suavizado, cimas, copas, polígonos, `treeID` por punto y comprobación con inventario | lidR `smooth_height`, `locate_trees`, `dalponte2016`, `silva2016`, `watershed`; CanopyMaxima |
| `obia` | Análisis por objetos: capas (CHM, rásteres, intensidad, retornos, suelo, densidad, rugosidad) → objetos (fusión de regiones o rango de valores) → atributos → clase (reglas o bosque aleatorio con muestras) → polígonos con huecos, ráster de objetos y de clases, tabla | análisis de imagen orientado a objetos (OBIA) |
| `change` | Cambio forestal entre dos CHM: diferencia, clases (crece, baja, pierde/gana copa), manchas de perturbación, resumen por rodal y crecimiento por árbol | detección de cambios con LiDAR multitemporal |
| `model` | Modelos de transición: regresión de una variable de campo (CSV o Excel) sobre las métricas de las parcelas, con validación cruzada que repite la elección de métricas, diagnósticos y, con las métricas de las celdas, el mapa de la variable (y su intervalo de predicción) | enfoque por área (ABA) |
| `corridor` | Conductores, postes y vegetación en riesgo (invasión y caída) de líneas eléctricas, con informe por vano | análisis de corredores eléctricos con LiDAR |
| `import`, `geojson`, `cog` | PLY a LAZ; shapefile ↔ GeoJSON; ráster a Cloud Optimized GeoTIFF | conversión de formatos |
| `clip` | Recorte con polígonos, parcelas circulares o rectángulos; una nube, el exterior o un archivo por región; métricas por región | ClipData, PolyClipData |
| `edit` | Elimina o reclasifica los puntos que están dentro (o fuera) de polígonos y cumplen unas condiciones; escribe un archivo nuevo | herramientas de edición de QGIS |
| `filter` | Une, filtra por clase/retorno/elevación, normaliza con un MDT y convierte LAS↔LAZ en una pasada | WhiteboxTools `lidar_join`, `filter_lidar_classes`, `normalize_lidar` |
| `thin` | Un punto por celda (el más bajo, el más alto, el más cercano al centro o el primero) | `lidar_thin`, `lidar_block_minimum/maximum` |
| `raster` | Malla de densidad, cuenta, elevación (media, máxima, mínima, rango) o intensidad | `lidar_point_density` |
| `terrain` | Análisis de terreno de un MDT: pendiente, orientación, sombreado, curvaturas, rugosidad, TRI, TPI, depresiones, acumulación de flujo, TWI, red de drenaje, subcuencas y cuenca visual (GeoTIFF) | `gdaldem`, WhiteboxTools, GRASS `r.slope.aspect`, `r.watershed`, `r.viewshed` |
| `contour`, `hillshade` | Curvas de nivel (shapefile 3D) y sombreado (PNG) de un MDT o CHM | `contour`, `hillshade` |
| `tile` | Divide la nube en teselas cuadradas, con búfer opcional | lastile, `lidar_tile` |
| `render` | Dibuja la nube (planta, 3D, perfil; TIN sombreado y curvas) a PNG | visores de nubes de puntos |
| `export` | Puntos a LAS/LAZ (por defecto), a texto con columnas a elegir o a shapefile PointZ; con `--normalize`, alturas sobre el suelo con el MDT de cada nube | LAS to ASCII del visor, `normalize_lidar` |

Métodos disponibles:

- **Suelo (`--ground`):**
  - `fusion`: Kraus y Pfeifer (1998).
  - `mcc`: Evans y Hudak (2007).
  - `csf`: simulación de tela, Zhang 2016.
  - `pmf`: morfológico progresivo, Zhang 2003.
  - `classes`: usa la clase 2 ya existente.
- **Ruido (`--noise`):**
  - `isolated`: celdas 3×3×3.
  - `sor`: estadístico sobre los k vecinos más cercanos.

- **Estructura (`metrics --structure`):** fracción de huecos, LAI (Beer–Lambert), FHD (Shannon), VCI y CBH a partir de las alturas de los retornos de cada celda, en un archivo `_estructura`; `--structure-layer`, `--structure-top`, `--gap-height`, `--extinction`, `--cbh-floor`, `--cbh-share`.
- **Cambio (`change`):** `--min-change`, `--cover-height`, `--min-patch`, `--years`, `--stands`, `--trees` (cimas de las dos fechas, emparejadas por distancia, `--match-dist`); el informe da el acuerdo vertical de los dos modelos.
- **Modelo (`model`):** `--plots` (capa `.shp` o tabla de texto de `clip --metrics`), `--field` (.csv, .txt o .xlsx, una fila por parcela), `--response`, `--plot-key` y `--field-key`; `--analyze` solo hace el análisis previo (normalidad de la variable y cribado de candidatas); `--candidates a,b` restringe las candidatas y `--max-corr` (0,8) quita las que repiten a otra; `--predictors a,b` fija las métricas, por su nombre largo (`"Elev P90"`) o por el corto del shapefile (`EP90`) (si no, se eligen solas con `--criterion bic|aicc`, `--max-vars`, `--max-vif`; la intensidad solo con `--with-intensity`); `--transform none|log-response|log-both`; `--cells` (tablas de texto de `metrics`, una por hoja) escribe el mapa en `-o`, `--interval` añade el intervalo de predicción del 95 %, `--extrapolate` estima también fuera del rango de las parcelas. Escribe `<salida>_modelo.txt` y `<salida>_parcelas.csv`.
- **Corredores (`corridor`):** conductores por proximidad de puntos de cable (`--link`), mediana por tramos y parábola en los huecos (`--gap-max`), postes (`--min-pole-height`), distancia de seguridad (`--min-distance`), margen de caída (`--fall-margin`), longitud mínima de conductor (`--min-length`).
- **COPC:** el recorte (`clip`) y la vista leen solo los nodos que hacen falta; los demás procesos lo leen como LAZ. `copc` escribe una nube como COPC 1.0 (octree de 128³ celdas por nodo, nodos de hasta 100 000 puntos, formatos 0–3 pasados a 6/7, WKT obligatorio); `<nombre>.copc.laz`.
- **Color (`colorize`):** `--image` (ortofoto en el sistema de la nube), `--bands 1,2,3`, `--nir` (formatos 8 y 10), `--keep-outside`. Los formatos 0, 1 y 6 se escriben como 7; 4 y 9 no se admiten. `_rgb`.
- **Intensidad (`intensity`):** I · (R / R_ref)^a (Höfle y Pfeifer, 2007). `--trajectory` (texto «tiempo x y z») o `--altitude` (altitud constante; R = (altitud − z) / cos(ángulo)); `--exponent` (2), `--reference` (por defecto, la distancia media). `_intensidad`.
- **Importar (`import`):** PLY y E57 (cada escaneo con su pose; su número en el ID de fuente); `--epsg`.
- **Líneas de rotura (`dtm --breaklines`):** shapefile de líneas o polígonos; con cotas fijan la altura, sin cotas toman el suelo más bajo a menos de una celda. Solo con `--surface tin|idw`.
- **Rásteres en teselas (`mosaic`, `stack`, `align`, `cog`):** Cloud Optimized GeoTIFF; `--compress auto|deflate|jpeg` (auto: JPEG YCbCr para 8 bits y 3 bandas, deflate en lo demás) y `--quality` (90).
- **Objetos (`obia`):**
  - Fusión de regiones (`--method merge`): manchas iniciales por intervalos de valor (`--init`), grafo de vecindad y fusión de parejas mutuamente mejores mientras el coste (valor y forma, Baatz y Schäpe 2000) sea menor que `--scale` al cuadrado; capas estandarizadas con peso (`--weight capa=peso`); `--min-cells` fusiona el objeto mínimo con su vecino más parecido.
  - Rango de valores (`--method mask`): componentes conexas de una capa entre `--min` y `--max`.
  - Capas de nube (`--cloud`, `--cloud-layers`): intensidad, retornos, suelo y densidad en una sola lectura; `--roughness` añade la desviación típica local del CHM.
  - Estratos homogéneos (`--kmeans N` agrupa por parecido; `--strata` une los objetos vecinos de una clase; `--strata-min-cells`, `--plots` reparte parcelas con asignación de Neyman; `--cover-height` fija la altura de la cobertura): polígonos, ráster y tabla resumen con superficie, media, desviación y coeficiente de variación.
  - Clasificación: reglas de texto (`--rules`) o bosque aleatorio (`--samples`, `--class-field`, `--trees`, `--seed`) con exactitud sobre los objetos fuera de bolsa, kappa y precisión por clase.
  - Resultados: polígonos con huecos y atributos, ráster de objetos (entero de 32 bits), ráster de clases y CSV.

- **Árboles (`trees`):**
  - Cimas: máximo local en ventana circular o cuadrada, fija o variable `a + b·h + c·h²`. La ventana es un diámetro con el borde incluido, y en una meseta gana la primera celda.
  - Copas: `dalponte` (alcance en píxeles, regla del 5 %, 4-vecinos), `silva` (vecino más cercano con radio `factor × altura`) y `watershed` (cuencas con tolerancia).
  - Suavizado previo del CHM (`--smooth`, media, mediana o gauss) y cimas a mano (`--tops-file`).
  - Resultados: CSV, ráster de copas, polígonos de copa y ápices en shapefile, y una copia de la nube con `treeID` por punto (`--cloud`).
  - Comprobación con inventario de campo (`--field`): acertados, falsos positivos, omitidos, exhaustividad, precisión y error de altura.

`easylidar <comando> --help` lista todos los parámetros.

## Validación

- **FUSION 4.61:**
  - Clasifica exactamente los mismos puntos de suelo (189 113 en `fusa.laz` y 18 371 741 en 27,8 millones de puntos).
  - El CHM es idéntico celda a celda.
  - Coinciden las 71 columnas de métricas de elevación y las 34 de intensidad. En las de estratos solo quedan diferencias de 1e-4 por redondeo float32.
- **MCC-LIDAR 2.1:** el 94,8 % de los puntos coincide. En el MDT resultante la mediana de diferencias es 0 m y el p90 2,5 cm.
- **lidR 4.3 (caja negra):** los ápices coinciden exactamente (11 007, 4 352 y 8 528 árboles en tres configuraciones sobre un CHM de 2,1 M de celdas). Las copas cubren casi las mismas celdas (Dalponte 99,99 %, Silva 99,7 %) y asignan el mismo árbol en el 97 % (Dalponte) y el 99 % (Silva) de ellas; la diferencia está en píxeles disputados entre dos árboles.
- **Inventario de campo** (`datos_1`, 16 parcelas, 125 árboles): con ventana variable 3 + 0,1·h se acierta el 86 % de los árboles (precisión 76 %, F = 0,81) con un sesgo de altura de +0,55 m.
- **Edificios:** detecta el 83 % de los puntos de edificio que clasificó el proveedor de `fusa`, con un 98 % de precisión.

## Rendimiento

| Nube de 6,9 M de puntos, 4 núcleos | EasyLiDAR | lidR 4.3 (R) |
|---|---|---|
| Suelo CSF | 26 s · 176 MB | 620 s |
| MDT por TIN | 5 s · 142 MB | 46 s |
| CHM sin huecos | 8 s · 269 MB | 72 s |
| Métricas | 4 s · 48 MB | 10 s |
| Árboles | 0,4 s · 57 MB | 25 s |

| Nube de 27,8 M de puntos | FUSION | EasyLiDAR (LAS) | EasyLiDAR (LAZ) |
|---|---|---|---|
| MDT | 125 s · 649 MB | 17 s · 96 MB | 25 s · 98 MB |
| CHM | 31 s · 83 MB | 11 s · 76 MB | 12 s · 77 MB |
| Métricas | 104 s · 458 MB | 15 s · 163 MB | 37 s · 163 MB |

## Diferencias conocidas

- Las rejillas se alinean por la esquina en múltiplos del tamaño de celda, para que las teselas encajen. El CSV de métricas usa la alineación del formato compatible con FUSION.
- Los huecos del MDT se rellenan desde fuera hacia dentro.
- Si un punto no tiene suelo debajo, se descarta. FUSION, en cambio, usa su cota absoluta como si fuera altura.
- CSF hunde la tela en las naves muy grandes. En zonas urbanas usa `--buildings`, o FUSION/PMF.

## Distribuir

Ver [docs/DISTRIBUCION.md](docs/DISTRIBUCION.md): ZIP portable, compilación para Windows, Linux y macOS con GitHub Actions, instaladores y firma.

## Compilar

```sh
cargo build --release      # target/release/easylidar y easylidar-gui
cargo test
```

Requiere Rust 1.90 o superior. No hay dependencias de C.

| Crate | Función |
|---|---|
| `crates/core` | Motor: lectura y escritura LAS/LAZ, lector JPEG 2000 propio (`jp2/`), teselado, filtros, rejillas y GeoTIFF, métricas, árboles, recorte, visor (`view`) y jobs |
| `apps/cli` | `easylidar` (línea de comandos) |
| `apps/gui` | `easylidar-gui` (eframe/egui) |

## Interfaz gráfica

`easylidar-gui` es un visor de nubes con el aspecto de RustGIS (cinta de opciones, panel de capas y de información, paleta de comandos con Ctrl+K, tema oscuro y claro) y las herramientas de proceso a un clic.

- **Abrir:** LAS, LAZ, PLY y E57 (convertidos a LAZ) y ASCII XYZ; *Recientes* con los diez últimos archivos y los diálogos en la última carpeta usada; ráster GeoTIFF/JP2/ASC y shapefile como referencia. Arrastrar y soltar. Una decimación (puntos máximos) mantiene la nube en memoria; se carga en segundo plano.
- **Ventanas:** planta, perfil y 3D, solas o combinadas. El 3D sigue la extensión de la planta.
- **Color de los puntos:** elevación (azul-rojo o tierra), clasificación con colores editables, intensidad, retorno, ID de fuente, archivo y RGB. Filtros por clase y por tipo de retorno. Tamaño de punto.
- **TIN sombreado** (con las clases que elijas) y **curvas de nivel** con equidistancia.
- **Herramientas del visor:** zoom (clic, ventana, rueda, extensión anterior y completa), desplazar, consultar punto (Q), medir (W, con distancia 3D), densidad de puntos, perfil (tres clics; flechas y rueda mueven la caja), marcadores POI y áreas AOI con nombre, color y comentario, guardados y cargados como shapefile.
- **Procesar:** información automática de los datos al abrir, clasificar, MDT, CHM, métricas, segmentación de árboles, recorte con polígonos y parcelas (archivo, AOI dibujadas, rectángulo o círculo) y exportar. Las rutas de salida se calculan a partir de la entrada y los ajustes, se pueden editar a mano y piden confirmación antes de sobrescribir. Al terminar, los rásteres y shapefiles nuevos aparecen como capas.
- **Atajos:** Ctrl+O abrir, Ctrl+K comandos, Q consultar, W medir (distancia, y área y perímetro con tres puntos o más), Z zoom, X desplazar, T TIN, +/− tamaño de punto, N norte arriba.

**Capas.** El panel *Contenido* es como la tabla de contenidos de un SIG: un árbol con el mapa, las capas en orden de dibujo (que se arrastran para reordenarlas) y las nubes de puntos; cada capa tiene su casilla de visibilidad, su símbolo y su nombre (recortado si es largo); el botón derecho abre zoom, tabla, guardar, subir, bajar y quitar, y la leyenda de las capas con símbolos por valores cuelga de su fila. Al elegir una capa aparecen en la cinta pestañas propias (en el color de acento):

- *Simbología* (vectorial): símbolo único, valores únicos, colores graduados (intervalos iguales o cuantiles, rampa y número de clases) y tamaño proporcional; colores de trazo y relleno, opacidad, grosor, tamaño de punto; etiquetas sin solapes con campo, tamaño, color y halo. Los colores de cada clase se editan en la leyenda.
- *Edición vectorial*: digitalizar (con ajuste a vértices), seleccionar (clic, rectángulo, Mayús/Ctrl), mover vértices (doble clic en un lado inserta uno, botón derecho en un vértice lo quita), mover elementos, eliminar, deshacer y rehacer (Ctrl+Z, Ctrl+Y), guardar en su sitio y guardar como. La tabla de atributos edita celdas, añade campos, ordena la selección con la del mapa y selecciona por atributo (=, ≠, <, ≤, >, ≥, contiene).
- *Ráster* (imágenes): bandas de rojo, verde y azul con ajustes preestablecidos (color natural, infrarrojo), estirado (sin estirar, mínimo–máximo, percentiles, desviaciones), brillo, contraste y gamma. (Rásteres de una banda: estirar con rampa, clasificar y relieve sombreado.)

Las imágenes se leen **por ventana y nivel de pirámide**: se usan las vistas reducidas que el GeoTIFF lleva dentro y solo se decodifican los mosaicos que se ven, de modo que una ortofoto de miles de millones de píxeles abre al instante y se afina al acercar. Los JPEG con color YCbCr se convierten a RGB. En JPEG 2000 (`crates/core/src/jp2/`, escrito a partir de la norma T.800: cajas JP2, marcadores del codestream, paquetes con árboles de etiquetas, los cinco órdenes de progresión, codificador MQ/EBCOT con sus opciones, wavelets 5/3 y 9/7 y transformadas de color RCT/ICT) las vistas reducidas son los niveles de la wavelet y solo se decodifican las teselas de la ventana. Medido con las bandas de Sentinel-2: una banda de 10 m (10 980 × 10 980) entera en 2,3 s en 10 núcleos, y una ventana de 480 × 440 celdas en unas centésimas de segundo; las bandas de 20 y 60 m y la imagen TCI en color coinciden valor a valor con OpenJPEG (los archivos con wavelet 9/7 difieren como mucho en 1–2 niveles, por el redondeo).

**Edición de puntos.** La pestaña *Edición* trabaja como las herramientas vectoriales de un SIG:

- Selecciona con rectángulo (un clic elige los puntos cercanos), polígono (clic derecho o doble clic cierra), lazo o círculo, en la planta o en el perfil.
- Combina selecciones: nueva, añadir (Mayús), restar (Alt), intersecar (Ctrl), invertir (Ctrl+I) y todo (Ctrl+A). Un filtro de atributos (clases, elevación, intensidad, tipo de retorno) restringe lo que se selecciona, o selecciona por atributos sin región.
- Elimina (Supr) o cambia la clase de la selección. Deshacer y rehacer (Ctrl+Z, Ctrl+Y), con un historial visible. Los puntos seleccionados se pintan en cian.
- Guardar crea un archivo nuevo (nunca sobrescribe el original). Los cambios son reglas: en pantalla se aplican a los puntos que se ven, y al guardar se aplican a **todos** los puntos del archivo en una sola pasada por todos los núcleos, así que el resultado es exacto aunque la nube esté decimada y la memoria no crece. En el borde de un polígono, unos pocos puntos pueden contarse distinto en pantalla (coordenadas decimales en f32) que en el archivo; el archivo es el exacto.
- Por línea de comandos: `easylidar edit nube.laz --regions zonas.shp --class 5 --set-class 7`.

No se leen los formatos Imagine ni GeoSAR.

Cada herramienta es un *job* del motor; la CLI y la interfaz solo rellenan sus parámetros. No hay `unsafe`, ni `unwrap`/`panic!` fuera de los tests.

## Créditos

- **RustGIS** (Apache-2.0): planteamiento del lector LAS y de la arquitectura por capas.
- **FUSION/LDV** (R. J. McGaughey, USDA Forest Service): formato de salida de las tablas documentado en su manual, para que los resultados sean compatibles.
- **MCC-LIDAR** (Apache-2.0) y la biblioteca CSF (Apache-2.0): algoritmos y constantes.
- **lidR** (GPL-3): referencia para comparar resultados; los algoritmos se implementaron desde sus artículos originales.
- **CSF** (Apache-2.0): método publicado (Zhang et al., 2016).
- **COPC 1.0** (copc.io): especificación pública del formato. **E57** (ASTM E2807), leído con el crate `e57` (MIT). **jpeg-encoder** (MIT/Apache-2.0, IJG) para las teselas JPEG. Intensidad normalizada según Höfle y Pfeifer (2007).
- **terrio** (MIT, Qiusheng Wu) y **PROJ** (MIT): el motor de coordenadas de `crates/crs` (proyecciones, tablas EPSG, transformaciones de datum) se adapta del módulo `crs` de terrio, que a su vez lo porta de PROJ. Datos: EPSG (IOGP), proyecciones de Esri (Apache-2.0) y las mallas del IGN y del ICGC (CC-BY 4.0) para pasar de ED50 a ETRS89 y del geoide EGM08-REDNAP.
- **Análisis de terreno:** métodos de Horn (1981), Zevenbergen y Thorne (1987), Wilson et al. (2007), Weiss (2001), O'Callaghan y Mark (1984), Beven y Kirkby (1979) y Barnes, Lehman y Mulla (2014), implementados desde esas publicaciones.

Ver [docs/LICENCIAS.md](docs/LICENCIAS.md), `NOTICE` y `THIRD_PARTY_LICENSES.md`.


## Manual y licencia

El manual de uso está en `docs/MANUAL.md` y dentro del programa (Ayuda → Manual, o F1). EasyLiDAR es de uso gratuito: ver `LICENSE`; los componentes de terceros están en `NOTICE` y `THIRD_PARTY_LICENSES.md`.
