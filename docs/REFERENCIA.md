# HIPSO · Referencia de la línea de comandos y de los formatos

Texto de referencia con todos los comandos de `hipso`, las rutas de resultados y los formatos de salida. La guía de uso de la interfaz es `MANUAL.md`.

Herramientas LiDAR (LAS/LAZ) para vegetación, terreno y cubierta en Rust puro para Windows, Linux y macOS. Línea de comandos e interfaz gráfica.

Ofrece un flujo de trabajo para el análisis de la vegetación con salidas compatibles con el formato de FUSION y métodos publicados (Kraus y Pfeifer, MCC, CSF, PMF, Dalponte, Silva…), implementados desde sus artículos. La premisa es procesar nubes muy grandes con poca memoria y en poco tiempo.

- **Streaming:** los pasos globales (MDT FUSION, CHM, métricas) leen la nube como flujo. La memoria depende del área, no del número de puntos.
- **Teselas en paralelo:** los pasos locales (ruido, suelo MCC/CSF/PMF/SMRF, edificios, TIN, CHM sin huecos) trabajan por teselas con búfer, una por núcleo. La memoria es núcleos × tesela.
- **LAZ:** la lectura es paralela. Si un paso necesita varias pasadas, el LAZ se descomprime una sola vez a un temporal en disco, que se borra al terminar.

## Uso

```sh
hipso-gui                                # interfaz gráfica (acepta un .tif/.asc o nubes como argumento)
hipso info nube.laz
hipso classify nube.laz -o clasif.laz --noise isolated --ground csf --buildings
hipso dtm nube.laz -o mdt.tif                                    # en streaming
hipso dtm clasif.laz -o mdt.tif --ground classes --surface tin   # TIN de la clase 2, por teselas
hipso chm nube.laz --dtm mdt.tif -o chm.tif --method pitfree --cell 0.5
hipso metrics nube.laz --dtm mdt.tif -o metricas.csv --intensity --strata
hipso trees chm.tif --method silva --crowns --crowns-shp --field inventario.txt
hipso clip nube.laz --regions muestras.shp --mode each          # PolyClipData
hipso render nube.laz --view 3d -o vista.png --color class      # imagen del visor
hipso export nube.laz -o puntos.csv --items x,y,z,class --header
hipso run flujo.model.json --input Nube=hoja.laz   # un flujo guardado por el constructor de flujos
```

**Ráster y vectorial.** Además de las nubes se cargan imágenes multibanda (GeoTIFF, JPEG 2000 como las bandas de Sentinel-2, PNG/JPEG con archivo de coordenadas, ENVI) como fondo, con composiciones de bandas (color natural, falso color infrarrojo…) y índices espectrales (NDVI, GNDVI, NDWI, EVI, SAVI, NDMI, NBR, VARI, ExG o una expresión propia sobre las bandas); el índice sale como capa ráster y se guarda en GeoTIFF. *Igualar rásteres* (`align`) reproyecta, ajusta la extensión y remuestrea varios rásteres a las celdas de uno de referencia, para que coincidan celda a celda. *Reclasificar* (`reclass`) y *Cortar por máscara* (`clip-raster`) completan las operaciones ráster. Se pueden crear capas vectoriales de puntos, líneas y polígonos (*Nueva capa*), digitalizar en la capa activa, editar su tabla de atributos y guardarlas como shapefile con la proyección de la nube; las tablas de atributos de cualquier capa vectorial cargada se pueden ver y editar. Cada campo tiene un tipo (texto, entero, decimal, sí/no, fecha o hipervínculo) que se comprueba al editar, se conserva al guardar y lo respetan las consultas y la calculadora de campos. Los CSV, TSV y XLSX se abren como tablas sin geometría. El panel **EDA** da histograma, dispersión, cajas y estadísticas (cuartiles por el método 7 de Hyndman y Fan, 1996) de capas, tablas, rásteres y nubes, con los gráficos enlazados a la selección; sustituye a las ventanas de resumen estadístico y de estadísticas ráster (las órdenes `summary` y `rstats` siguen). El mapa base incluye Sentinel-2 reciente en color natural e infrarrojo color (datos abiertos de Copernicus, mosaico de los últimos 60 días).

**Flujos.** `run` ejecuta un flujo guardado por el constructor de flujos de la interfaz (un `.model.json`: nodos que son órdenes de `hipso` y entradas, e hilos que llevan los archivos de unos a otras), con `--input nombre=ruta` para sustituir un nodo de entrada y `--results-dir` para la carpeta de resultados; solo se conservan los nodos marcados, y si un paso falla se borra todo lo que la ejecución escribió. `model-validate` lista lo que impide ejecutar un flujo y `schema` escribe en JSON lo que admite cada orden (es lo que lee la interfaz para dibujar las opciones de cada nodo).

**Hojas.** Cada nube es una hoja: `classify`, `dtm`, `chm` y `metrics` procesan las que les des una a una y escriben un resultado por hoja (nunca las unen); `--mosaic` añade además un mosaico del MDT o del CHM. El MDT parte de la clase 2 que trae la nube (`--ground` prueba otro método) y MDT y CHM aceptan `--smooth` (media, mediana o gauss). Un trabajo se puede parar y se borra lo parcial. Si una hoja falla (un LAZ truncado, un disco lleno), las demás siguen: al final se listan las que fallaron, con su archivo y el motivo, se borra lo que dejaron a medias y el comando acaba con código 1.

**Tamaños grandes.** La memoria no depende del tamaño del ráster ni de la nube. Los mosaicos, la pendiente, la orientación, el sombreado, las curvaturas, la rugosidad, el TRI, el TPI, la reproyección de rásteres y el COG de un ráster de más de 32 millones de celdas se hacen por franjas de filas, con el margen que cada producto necesita, y el resultado es idéntico celda a celda al de leerlo entero. El tamaño de la franja sale de la memoria del equipo (unos 33 millones de celdas en uno de 16 GB, unos 134 millones en uno de 64 GB). `HIPSO_STREAM=1` fuerza las franjas en cualquier tamaño y `HIPSO_STREAM=0` las prohíbe (para comparar). `dedup` reparte las claves en tantas pasadas como pida la memoria del equipo (`HIPSO_DEDUP_PARTS=n` fija el número). Las depresiones, la acumulación de flujo, el TWI, la red de drenaje y las subcuencas siguen el agua por todo el ráster a la vez: antes de leerlo dicen cuánta memoria necesitan y, si no cabe, piden una celda mayor o dividir la zona.

**Catálogo y bordes entre hojas.** `catalog` lee solo las cabeceras de las hojas (o de una carpeta) y comprueba el teselado antes de procesar: sistema de coordenadas distinto o sin declarar, referencia vertical, hojas repetidas, solapes (y pasadas de vuelo, que no son un teselado), huecos, formatos de punto mezclados y LAZ sin índice. Acaba con código 1 si hay errores; `--tolerance` (1 m) fija lo que pueden solaparse o separarse las hojas. `--neighbors M` en `classify`, `dtm`, `dsm`, `chm` y `metrics` calcula cada hoja con las hojas a menos de M metros y se queda con lo suyo (sin costura en los bordes; en `metrics` cada celda sale una sola vez, con todos sus puntos; en `dsm` y `chm` por máximo solo actúa con relleno de huecos o suavizado). `trees --around CHM --margin M` busca los árboles sobre el CHM ampliado con las hojas vecinas y deja a cada hoja solo los de su cima: un árbol en el borde se cuenta una vez. `fit --plane` ajusta con la distancia al plano tangente (punto a plano) y `fit --trim 0.8` usa solo el 80 % de pares más cercanos.

**Unidades de muestreo.** `clip` lee las unidades de muestreo de un archivo (campo con el número, forma circular o cuadrada, radio o lados en metros) o las genera con `--sample random|systematic`; `--mode metrics` da solo la capa vectorial de unidades de muestreo con sus métricas, `--mode each` una nube por unidad de muestreo. `crs` asigna a una nube sin proyección la de otra (`--from`) o un EPSG.

**Formatos de las tablas.** Las métricas por celda, las métricas por unidad de muestreo y los árboles se guardan por defecto como capa vectorial (shapefile: un polígono por celda o unidad de muestreo, un punto por árbol, con el sistema de coordenadas de la nube). Con `--format csv` (o `-o resultado.csv`) se escribe solo la tabla de texto, que es lo más ligero en procesos enormes, y con `--format both` las dos. En `clip` el mismo ajuste es `--metrics-format`. Los nombres de campo del DBF tienen 10 caracteres como máximo: `<nombre>_campos.txt` dice qué significa cada uno. Todos los rásteres y shapefiles que se crean se pueden cargar como capas en la interfaz (*Abrir*, o arrastrándolos), colorear por un campo y consultar con la herramienta Consultar. Con `--metrics-columns` se eligen las métricas por unidad de muestreo que se escriben, por el nombre de su columna y en el orden dado (`--metrics-columns "Elev P90,Elev mean,Int mean"`); sin él se escriben todas, y un nombre que no existe da un error que lista los válidos.

**Rutas de resultados.** Si no das `-o`, cada resultado se nombra a partir de la entrada y de los ajustes (`datos_1_mdt_1m_tin.tif`, `datos_1_chm_0.5m_pitfree.tif`…) y se guarda en la misma carpeta que la entrada (no se crea ninguna carpeta `resultados`), salvo que indiques otra con `--results-dir`. Una ejecución nunca hereda la ruta de otra ni pisa un resultado anterior: si el nombre existe se usa `_2`, `_3`… Un `-o` que nombra un archivo ya existente detiene la orden, salvo que se añada `--force`, y nunca puede ser uno de los archivos de entrada. `chm` y `metrics` usan, si no pasas `--dtm`, el MDT más reciente **de esa misma nube**; si no existe, avisan en vez de coger el de otra.

| Comando | Qué hace | Comparable a (solo referencia) |
|---|---|---|
| `classify` | Solape entre pasadas (12), ruido (7), suelo (2), edificios (6), vegetación por estratos (3, 4, 5), cables (14), postes y torres (15), carreteras (11), agua (9) y reglas, en una receta o por opciones. Escribe LAS o LAZ. | MCC-LIDAR, lidR `classify_*` |
| `dtm` | MDT en GeoTIFF por media por celda o TIN | GroundFilter + GridSurfaceCreate, lidR `rasterize_terrain` |
| `chm` | Modelo de alturas de la vegetación: punto más alto por celda, o sin huecos | CanopyModel, lidR `pitfree` |
| `metrics` | CSV de elevación, intensidad (`--intensity`) y estratos (`--strata`) | GridMetrics |
| `trees` | Segmentación de individuos: suavizado, cimas, copas, polígonos, `treeID` por punto y comprobación con inventario | lidR `smooth_height`, `locate_trees`, `dalponte2016`, `silva2016`, `watershed`; CanopyMaxima |
| `obia` | Análisis por objetos: capas (CHM, rásteres, intensidad, retornos, suelo, densidad, rugosidad) → objetos (fusión de regiones o rango de valores) → atributos → clase (reglas o bosque aleatorio con muestras) → polígonos con huecos, ráster de objetos y de clases, tabla | análisis de imagen orientado a objetos (OBIA) |
| `change` | Cambio de la cubierta vegetal entre dos CHM: diferencia, clases (crece, baja, pierde/gana cubierta), manchas de perturbación, resumen por unidad de gestión y crecimiento por individuo | detección de cambios con LiDAR multitemporal |
| `model` | Modelos lineales: regresión de una variable de campo (CSV o Excel) sobre las métricas de las unidades de muestreo, con validación cruzada que repite la elección de métricas, diagnósticos y, con las métricas de las celdas, el mapa de la variable (y su intervalo de predicción) | enfoque por área (ABA) |
| `corridor` | Conductores, postes y vegetación en riesgo (invasión y caída) de líneas eléctricas, con informe por vano | análisis de corredores eléctricos con LiDAR |
| `reproject` | Nube o ráster a otro sistema de coordenadas (`--to`) o a las celdas de un ráster de referencia (`--reference`), con remuestreo a elegir (`--resample`, `--cell`) y cambio opcional de la referencia vertical (`--vertical-in`, `--vertical-out`) y política estricta ante aproximaciones; también capas vectoriales (shapefile, GeoPackage, GeoJSON, KML), sin cambio vertical | PROJ, `gdalwarp`, `ogr2ogr -t_srs` |
| `reclass` | Reclasifica un ráster con una tabla de reglas (valores, intervalos, comparaciones, `nodata`) | `gdal_calc.py`, `terra::classify` |
| `clip-raster` | Corta un ráster con una capa de polígonos u otro ráster como máscara, con o sin recorte a su caja | `gdalwarp -cutline`, `terra::mask` |
| `zonal` | Estadística de un ráster por polígono (recuento, mínimo, máximo, media, suma, desviación y mediana) como capa de polígonos con campos nuevos | estadística zonal de QGIS |
| `extract`, `join-table`, `merge`, `split`, `clip-vector`, `near`, `summary`, `fields` | Valores de rásteres a una capa, unión de una tabla por un campo, combinar y dividir capas, recortar o borrar con polígonos (puntos, líneas y polígonos), distancia al elemento más cercano, estadísticas de resumen por grupos (en la interfaz, en el EDA) y quitar o renombrar campos | *Sample raster values*, *Join attributes by field value*, *Merge*, *Split vector layer*, *Clip*, *Join by nearest*, *Basic statistics* de QGIS |
| `rasterize`, `focal`, `rstats`, `tabulate` | Vector a ráster, filtros focales (media, mediana, mínimo, máximo, suma, rango, desviación, mayoría, gaussiano), estadísticas con histograma y área de clases por zona | `gdal_rasterize`, `terra::focal`, `terra::global`, `terra::crosstab` |
| `topology` | Reglas de topología: solapes, huecos y autointersecciones de polígonos; cabos sueltos de líneas; informe CSV | *Topology checker* de QGIS |
| `fix-topology` | Repara la topología: junta vértices cercanos y los apoya en los bordes vecinos, quita solapes (se queda el mayor, el menor o el primero) y rellena huecos pequeños con el vecino de más borde; en líneas, une extremos cortos | *Topology checker* y *v.clean* de QGIS y GRASS |
| `simplify`, `parallel` | Simplificación de Douglas y Peucker con el borde común de dos polígonos tratado una sola vez, y línea paralela a una distancia con inglete limitada | `ST_SimplifyPreserveTopology`, `ST_OffsetCurve` de PostGIS |
| `is-valid`, `make-valid` | Comprobar y reparar la validez de los polígonos de una capa | `ST_IsValid`, `ST_MakeValid` |
| `buffer`, `dissolve`, `union`, `intersect` | Búfer, disolución por campo, unión (superposición) e intersección de capas de polígonos | geoprocesos de QGIS |
| `sieve`, `proximity`, `fill-nodata` | Quitar manchas pequeñas de un ráster de clases, distancia a unas clases y relleno de huecos por distancia inversa | `gdal_sieve`, `gdal_proximity`, `gdal_fillnodata` |
| `import`, `geojson`, `cog` | PLY a LAZ; shapefile ↔ GeoJSON; ráster a Cloud Optimized GeoTIFF | conversión de formatos |
| `clip` | Recorte con polígonos, unidades de muestreo circulares o rectángulos; una nube, el exterior o un archivo por región; métricas por región | ClipData, PolyClipData |
| `edit` | Elimina o reclasifica los puntos que están dentro (o fuera) de polígonos y cumplen unas condiciones; escribe un archivo nuevo | herramientas de edición de QGIS |
| `filter` | Une, filtra por clase/retorno/elevación, normaliza con un MDT y convierte LAS↔LAZ en una pasada | lidR `filter_poi`, `normalize_height` |
| `thin` | Un punto por celda (el más bajo, el más alto, el más cercano al centro o el primero) | lidR `decimate_points` |
| `raster` | Malla de densidad, cuenta, elevación (media, máxima, mínima, rango) o intensidad | lidR `rasterize_density` |
| `terrain` | Análisis de terreno de un MDT: pendiente, orientación, sombreado, curvaturas, rugosidad, TRI, TPI, depresiones, acumulación de flujo, TWI, red de drenaje, subcuencas y cuenca visual (GeoTIFF) | `gdaldem`, GRASS `r.slope.aspect`, `r.watershed`, `r.viewshed` |
| `ponds` | Zonas encharcadas de un MDT: charcos con área, profundidad, volumen, cota de desborde, superficie que vierte y pendiente | GRASS `r.fill.dir` |
| `flood` | Simulación de inundación sobre un MDT a uno o varios niveles, con conectividad desde unas semillas o en modo bañera | `terra::patches` sobre `z <= nivel` |
| `ridges` | Líneas de cresta o de drenaje de un MDT como líneas 3D con orden de Strahler, longitud y pendiente | `terra::terrain` (D8) |
| `leastcost` | Ruta de menor coste sobre un MDT por pendiente, tiempo de marcha de Tobler o un ráster de costes, con barreras, zonas preferidas y paradas | `terra::costDist`, GRASS `r.cost` |
| `piles` | Cubicación de pilas: volumen, áreas, alturas y masa sobre una base ajustada al contorno de cada pila | QGIS *Raster surface volume* |
| `cutfill` | Desmonte y terraplén entre un terreno y un diseño (ráster, cota o plano inclinado), con totales por polígono | aritmética de rásteres (`terra`) |
| `calc` | Calculadora ráster: una expresión celda a celda sobre rásteres con nombre, evaluada por franjas | `gdal_calc.py`, `terra::app` |
| `vectorize` | Ráster a polígonos por clase, rango o intervalos, con valor, área y celdas | `gdal_polygonize.py`, `terra::patches` |
| `flightlines` | Líneas de vuelo de una nube aérea por los huecos en el tiempo GPS: tabla, contornos y copias con el número de línea | lidR `retrieve_flightlines` |
| `voronoi`, `voronoi-density` | Polígonos de Voronoi (y aristas de Delaunay) de una capa de puntos, y densidad de una nube por celdas de Voronoi con el porcentaje que llega a un umbral | GEOS `ST_VoronoiPolygons` |
| `dtm-validate` | Exactitud vertical de un MDT con puntos de control (nube, capa o tabla): sesgo, MAE, RMSE, desviación y los robustos NMAD y LE95 de Höhle y Höhle (2009), con la tabla de errores punto a punto | estadísticos de error de elevaciones |
| `canopy-gaps` | Huecos del dosel: manchas conectadas del CHM por debajo de una altura, con superficie, perímetro, forma y alturas | `terra::patches` sobre `chm < altura` |
| `voxel-metrics` | Estructura vertical por columnas y capas: vóxeles ocupados, volumen, relleno, alturas, entropía de las alturas, fracción de huecos y LAI, un ráster por métrica | rejillas de conteo por capas |
| `trees3d`, `tree-metrics` | Segmentar con puntos, de arriba abajo en la línea de Li et al. (2012), con la nube escrita con `treeID`; y métricas por árbol (altura, base y proporción de copa, superficie, volumen por envolvente convexa y por forma alfa 3D) | segmentación de nubes de puntos |
| `contour`, `hillshade` | Curvas de nivel (shapefile 3D) y sombreado (PNG) de un MDT o CHM | `gdal_contour`, `gdaldem hillshade` |
| `tile` | Divide la nube en teselas cuadradas, con búfer opcional | lidR `catalog_retile` |
| `render` | Dibuja la nube (planta, 3D, perfil; TIN sombreado y curvas) a PNG | visores de nubes de puntos |
| `export` | Puntos a LAS/LAZ (por defecto), a texto con columnas a elegir o a shapefile PointZ; con `--normalize`, alturas sobre el suelo con el MDT de cada nube | lidR `normalize_height` |

Métodos disponibles:

- **Suelo (`--ground`):**
  - `fusion`: Kraus y Pfeifer (1998).
  - `mcc`: Evans y Hudak (2007).
  - `csf`: simulación de tela, Zhang 2016.
  - `pmf`: morfológico progresivo, Zhang 2003.
  - `smrf`: morfológico simple, Pingel et al. 2013.
  - `axelsson`: TIN progresivo, Axelsson 2000.
  - `classes`: usa la clase 2 ya existente.
- **Ruido (`--noise`):**
  - `isolated`: celdas 3×3×3.
  - `sor`: estadístico sobre los k vecinos más cercanos.
  - `elm`: mínimo local extendido, para el ruido bajo.
- **Solape (`--overlap`, `--overlap-cell`):** aparta a la clase 12 las pasadas que sobran en las celdas con dos pasadas; con más de dos no aparta ninguna.
- **Vegetación (`--vegetation`), cables (`--wires`), postes y torres (`--poles`), carreteras (`--roads`) y agua (`--water`):** pasos con valores por defecto que se ajustan a la densidad de la tesela y a la escala de la intensidad; carreteras y agua se comparan con la intensidad mediana del suelo.
- **Reglas (`--rule`, repetible):** `to=`, `from=`, `returns=`, `angle=`, `height=` (m sobre el suelo), `intensity=`, `slope=`, `planarity=`, `linearity=`, `sphericity=`, `roughness=`, `neighbours=` y `radius=`; se aplican en orden, al final.
- **Recetas (`--recipe ARCHIVO`):** las opciones del archivo (una o más por línea; `#` comenta) ocupan el lugar de la opción. La ventana de clasificación del GUI las guarda y las carga (*Guardar receta…*, *Cargar receta…*).

- **Estructura (`metrics --structure`):** fracción de huecos, LAI (Beer–Lambert), FHD (Shannon), VCI y CBH a partir de las alturas de los retornos de cada celda, en un archivo `_estructura`; `--structure-layer`, `--structure-top`, `--gap-height`, `--extinction`, `--cbh-floor`, `--cbh-share`.
- **Cambio (`change`):** `--min-change`, `--cover-height`, `--min-patch`, `--years`, `--stands`, `--trees` (cimas de las dos fechas, emparejadas por distancia, `--match-dist`); el informe da el acuerdo vertical de los dos modelos.
- **Modelo (`model`):** `--plots` (capa `.shp` o tabla de texto de `clip --metrics`), `--field` (.csv, .txt o .xlsx, una fila por unidad de muestreo), `--response`, `--plot-key` y `--field-key`; `--analyze` solo hace el análisis previo (normalidad de la variable y cribado de candidatas); `--candidates a,b` restringe las candidatas y `--max-corr` (0,8) quita las que repiten a otra; `--predictors a,b` fija las métricas, por su nombre largo (`"Elev P90"`) o por el corto del shapefile (`EP90`) (si no, se eligen solas con `--criterion bic|aicc`, `--max-vars`, `--max-vif`; la intensidad solo con `--with-intensity`); `--transform none|log-response|log-both`; `--cells` (tablas de texto de `metrics`, una por hoja) escribe el mapa en `-o`, `--interval` añade el intervalo de predicción del 95 %, `--extrapolate` estima también fuera del rango de las unidades de muestreo. Escribe `<salida>_modelo.txt` y `<salida>_parcelas.csv`.
- **Zonas encharcadas (`ponds`):** el MDT se rellena por prioridad (Barnes et al., 2014), y un charco es una mancha de celdas con agua más honda que `--min-depth` (0,05). Se descartan charcos con `--min-area`, `--max-area`, `--max-depth`, `--min-volume`, `--max-slope` (grados) y `--min-catchment`. El método `--method depression` (por defecto) mide el agua que cada depresión retiene hasta su desborde, y `local` busca huecos frente al modelo suavizado, con la pendiente y la superficie vertida comprobadas celda a celda. `--smooth` (gauss, en unidades del mapa) suaviza antes, `--nodata` fija el valor de 'sin datos', y `--depth-raster` y `--gpkg` añaden la profundidad y un GeoPackage. Los campos son `id`, `area_m2`, `prof_max`, `prof_med`, `vol_m3`, `cota_desb`, `cuenca_m2` y `pend_med`, y los resultados se nombran `_charcos`.
- **Inundación (`flood`):** el nivel mínimo de agua que alcanza cada celda desde las semillas (`--seed x,y`, repetible, o `--border`) se calcula una sola vez para todos los niveles (`--level` repetible, o `--levels desde:hasta:paso`, con `--relative min|seed` para medirlos como alturas). `--bathtub` inunda todo lo que está bajo el nivel, sin conectividad, y `--connectivity 4|8` decide por cuántas vecinas pasa el agua. `--extent` escribe los polígonos de cada nivel, `--no-depth` y `--no-csv` quitan la profundidad y la tabla, y el ráster de alcance se escribe siempre. Los resultados se nombran `_inundacion`.
- **Crestas y drenaje (`ridges`):** la red D8 de las celdas que recogen al menos `--min-area` (1 000 m²) se traza en el MDT con `--drainage`, y en su imagen invertida para las crestas, y se corta en tramos. `--min-length` descarta tramos cortos y `--smooth` aplica pasadas de Chaikin. Los campos son `ID`, `ORDEN` (de Strahler), `LONG` y `PEND_MED` (en %), y los resultados se nombran `_crestas` o `_drenaje`.
- **Ruta de menor coste (`leastcost`):** `--from` y `--to` fijan el origen y el destino (este, repetible con `--nearest`), y `--via` añade paradas en orden. El coste sale de `--model slope|tobler|uniform|raster:<tif>`, con `--power` (2) y `--reference-slope` (10 %) para la pendiente. `--max-slope` y `--barriers` impiden cruzar, `--prefer` y `--prefer-factor` (0,5) abaratan zonas, y `--cost-distance` escribe el coste acumulado. Los campos son `ID`, `LONG_2D`, `LONG_3D`, `COSTE`, `PEND_MED`, `PEND_MAX`, `ASCENSO` y `DESCENSO`, y los resultados se nombran `_ruta` y `_ruta_coste`.
- **Pilas (`piles`):** `--piles` da los contornos y `--base plane|lowest|median|interpolated|level:<z>|dtm:<archivo>` decide la base de cada pila. `--min-height` descarta alturas pequeñas, `--density` (t/m³) añade la masa, y `--base-raster` y `--height-raster` escriben la base y la altura sobre ella. El volumen se decide por el centro de cada celda. Los campos son `area_2d`, `area_3d`, `base_z`, `vol_sobre`, `vol_bajo`, `h_media`, `h_max`, `perimetro` y `masa_t`, y la tabla se llama `_pilas.csv`.
- **Desmonte y terraplén (`cutfill`):** `--design` admite un ráster, `level:<z>` o `plane:<z0>,<pendiente %>,<orientación>,<x>,<y>`, y la diferencia es `diseño − existente` (positiva es terraplén). `--regions` mide cada polígono aparte, `--tolerance` (0,05 m) fija lo que cuenta como sin cambio y `--classified` escribe el ráster de clases. Los campos son `desmonte`, `terraplen`, `neto`, `a_desm`, `a_terr` y `a_igual`, y los resultados se nombran `_cutfill`.
- **Calculadora (`calc`):** los rásteres se pasan como `NOMBRE=archivo.tif` (con uno solo basta el archivo, y sus bandas son `B1`, `B2`…, donde `A.2` es la banda 2 de A). La expresión admite `+ - * / % ^`, comparaciones, `and`, `or`, `not` y las funciones `abs sqrt ln log10 exp pow min max floor ceil round sin cos tan atan2 clamp if isnan`. `--dtype f32|u8|u16` fija el tipo del resultado, `--nearest` conserva las clases en las vistas reducidas y `--align` pone los rásteres sobre las celdas del primero (reproyectándolos si están en otro sistema). Los resultados se nombran `_calc`.
- **Vectorizar (`vectorize`):** `--classes` (por defecto) agrupa por valor, `--range desde,hasta` por un rango y `--breaks a,b,c` por intervalos. `--min-cells` descarta manchas pequeñas, `--simplify` aligera los contornos y `--diagonal` une las celdas que se tocan por una esquina. La extensión de `-o` decide entre shapefile y GeoPackage. Los campos son `valor`, `area_m2` y `celdas` (y `lim_inf` y `lim_sup` en rangos e intervalos), y los resultados se nombran `_poligonos`.
- **Líneas de vuelo (`flightlines`):** `--gap` (30 s) es el hueco del tiempo GPS que separa dos líneas y `--min-points` (1 000) y `--min-duration` descartan tramos pequeños. `--assign source-id|user-data` escribe copias de las nubes con el número de línea, y `--no-csv` y `--no-vector` quitan la tabla y los contornos. Con `swath --gps-gap` las pasadas se toman de esos mismos huecos, y los resultados se nombran `_lineas_de_vuelo`.
- **Voronoi (`voronoi`, `voronoi-density`):** `voronoi` da el polígono de cada punto con sus atributos, y `--clip bbox` o una capa de polígonos recorta las celdas del borde, que sin recorte se omiten, mientras que `--delaunay` añade las aristas. `voronoi-density` calcula `n / Σ área` por celda del ráster (`--cell`, 5 m), con `--threshold` (8 puntos/m²), `--returns` (F), `--class`, `--max-cell-area` (100 m², a partir de lo cual una celda es un hueco de datos), `--tile` (250 m) y `--buffer` (20 m). Escribe `_vdensidad`, `_vdensidad_cumple` y una tabla.
- **Validar MDT (`dtm-validate`):** `--reference` es una nube (clase `--class`, 2), una capa de puntos con `--z-field` o una tabla con `--x-field`, `--y-field` y `--z-field`. `--sampling` elige entre bilinear y nearest, y `--max-error` descarta puntos. El error es MDT − referencia, y NMAD = 1,4826 · mediana(|e − mediana(e)|) y LE95 = percentil 95 de |e|, según Höhle y Höhle (2009).
- **Huecos del dosel (`canopy-gaps`):** `--height` (5 m), `--min-area` (1 m²) y `--max-area` (10 000 m²); los huecos que tocan el borde o una celda sin dato se descartan salvo con `--include-edge`. Las celdas sin dato nunca son hueco, y la conectividad es por aristas.
- **Métricas de vóxeles (`voxel-metrics`):** `--cell` (2 m), `--dz` (1 m), `--max-height` (60 m), `--min-points`, `--ground-height` (1 m) y `--k` (0,5), y `--metrics` elige entre `puntos`, `voxeles`, `volumen`, `relleno`, `hmax`, `hmedia`, `entropia`, `hueco` y `lai`. Entropía, fracción de huecos y LAI siguen las definiciones de las métricas de estructura de `metrics --structure`.
- **Segmentar con puntos (`trees3d`):** recorrido de arriba abajo con separaciones `--dt1` (1,5 m) y `--dt2` (2 m) por debajo y por encima de `--zu` (15 m), radio de unión `--radius` (3 m) y `--min-points`. Es una reconstrucción a partir de la descripción general del método de Li et al. (2012), sin sus reglas de borde (véase su contraste con lidR en la validación). Cada árbol se numera por la posición de su cima en una retícula de 1 m, de modo que las teselas coinciden.
- **Copas en nubes terrestres (`stem-volume --crowns`):** cada tronco siembra los vóxeles de su eje medido y el camino más corto (Dijkstra 1959) por los vóxeles ocupados de 10 cm, con 26 vecinos y coste igual a la longitud, reparte los demás puntos hasta 30 m de camino; lo que no alcanza vuelve al eje más cercano. Contra los 167 individuos que trae la propia nube de 5193 en un atributo (`PredInstance`, de un modelo externo, no verdad de campo), la pureza de los árboles pasa de 0,853 (eje) a 0,875 (vóxel de 10 cm) y 0,881 (5 cm), y el IoU medio de 0,769 a 0,798 y 0,808; con 20 cm es 0,781. Solo el 5,4 % de los puntos cambia de árbol. La altura y el volumen convexo por árbol salen casi iguales con los dos métodos, y el área de copa sale algo mayor (mediana 1,10 frente a 1,03 respecto a la referencia). En la nube de 414 M de puntos de 11684, `stem-volume --tree-ids --crowns` tarda 623 s y llega a 11,6 GB de memoria máxima (2 560 árboles, el 99,9 % de los puntos con árbol); no se midió aparte lo que cuesta `--crowns` ahí. En 5193 (73 M) suma unos 15 s y tiene un pico de unos 2 GB.
- **Métricas por árbol (`tree-metrics`):** la base de la copa es el primer salto vertical mayor que `--gap` (2 m) al bajar desde la cima, por encima de `--crown-min` (1,3 m); la forma alfa suma los tetraedros de la triangulación de Delaunay 3D (Bowyer 1981; Watson 1981) con radio circunscrito menor que `--alpha` (2 m), y una copa de más de 2 500 puntos se reduce a un punto por vóxel antes de triangularla. El DAP es el diámetro de un círculo ajustado por mínimos cuadrados algebraicos a los puntos entre 1,2 y 1,4 m de altura, con tres pasadas que quitan los puntos a más de tres veces el residuo; se rechaza si hay menos de 12 puntos, si el radio sale fuera de 2–60 cm, si el residuo pasa del 20 % del radio o si queda un hueco de más de 200°. Contra el diámetro a 1,30 m de `stem-volume` en la nube TLS 5193 (133 árboles), 96 árboles tienen DAP y la mediana de la diferencia es 0,15 cm; el 94 % queda a menos de 2 cm, y cuatro árboles fallan por mucho (hasta 93 cm), por follaje o troncos vecinos asignados al mismo número.
- **Corredores (`corridor`):** no clasifica: lee los cables (14) y los postes (15) ya clasificados con `classify`. Conductores por proximidad de puntos de cable (`--link`), los paralelos separados por posición y altura, mediana por tramos y parábola en los huecos (`--gap-max`), postes (`--min-pole-height`), distancia de seguridad (`--min-distance`), margen de caída (`--fall-margin`), longitud mínima de conductor (`--min-length`).
- **COPC:** el recorte (`clip`) y la vista leen solo los nodos que hacen falta; los demás procesos lo leen como LAZ. `copc` escribe una nube como COPC 1.0 (octree de 128³ celdas por nodo, nodos de hasta 100 000 puntos, formatos 0–3 pasados a 6/7, WKT obligatorio); `<nombre>.copc.laz`.
- **Color (`colorize`):** `--image` (ortofoto en el sistema de la nube), `--bands 1,2,3`, `--nir` (formatos 8 y 10), `--keep-outside`. Los formatos 0, 1 y 6 se escriben como 7; 4 y 9 no se admiten. `_rgb`.
- **Intensidad (`intensity`):** I · (R / R_ref)^a (Höfle y Pfeifer, 2007). `--trajectory` (texto «tiempo x y z») o `--altitude` (altitud constante; R = (altitud − z) / cos(ángulo)); `--exponent` (2), `--reference` (por defecto, la distancia media). `_intensidad`.
- **Importar (`import`):** PLY y E57 (cada escaneo con su pose; su número en el ID de fuente); `--epsg`.
- **Líneas de rotura (`dtm --breaklines`):** shapefile de líneas o polígonos; con cotas fijan la altura, sin cotas toman el suelo más bajo a menos de una celda. Solo con `--surface tin|idw`.
- **Reclasificar (`reclass`):** `--rules` (o `--rules-file`) lleva una regla por línea o separadas por `;`: `10 = 1`, `0..5 = 2`, `< 0 = nodata`, `>= 100 = 4`, `nodata = 0`, `else = …`; manda la primera que vale. `--others keep|nodata|<valor>` dice qué hacer con lo que ninguna recoge, `--band` elige la banda y `--nodata` el valor de vacío. El resultado usa el tipo entero más pequeño que cabe y se nombra `_reclass`.
- **Cortar por máscara (`clip-raster`):** `--mask` es una capa de polígonos o un ráster (se conservan las celdas con dato distinto de cero); `--no-crop` conserva el tamaño del ráster, `--invert` conserva lo de fuera y `--nodata` fija el valor de las celdas vaciadas. Se nombra `_recorte`.
- **Reproyectar y remuestrear rásteres (`reproject`, `align`):** `--resample auto|nearest|bilinear|average|cubic|mode`; `auto` conserva los valores de las clases y, si no, usa media al reducir y bilineal en los demás casos. `reproject --reference ráster.tif` toma el sistema, la extensión y la celda de otro ráster. Con 114 millones de celdas, UTM 30N a 29N y bilineal, tarda 2,0 s en un modelo de elevaciones, frente a 4,9 s de GDAL 3.9 (mismo producto, COG con vistas reducidas); en una ortofoto de 8 bits y tres bandas, 2,5 s frente a 2,1 s.
- **Rásteres en teselas (`mosaic`, `stack`, `align`, `cog`):** Cloud Optimized GeoTIFF; `--compress auto|deflate|jpeg` (auto: JPEG YCbCr para 8 bits y 3 bandas, deflate en lo demás) y `--quality` (90).
- **Objetos (`obia`):**
  - Fusión de regiones (`--method merge`): manchas iniciales por intervalos de valor (`--init`), grafo de vecindad y fusión de parejas mutuamente mejores mientras el coste (valor y forma, Baatz y Schäpe 2000) sea menor que `--scale` al cuadrado; capas estandarizadas con peso (`--weight capa=peso`); `--min-cells` fusiona el objeto mínimo con su vecino más similar.
  - Rango de valores (`--method mask`): componentes conexas de una capa entre `--min` y `--max`.
  - Capas de nube (`--cloud`, `--cloud-layers`): intensidad, retornos, suelo y densidad en una sola lectura (las celdas sin puntos se rellenan con la más cercana hasta `--cloud-fill` celdas, 3 por defecto, y se suavizan con `--cloud-smooth`, 1 por defecto, para evitar huecos y ruido de sal y pimienta); `--roughness` añade la desviación típica local del CHM.
  - Estratos homogéneos (`--kmeans N` agrupa por similitud; `--strata` une los objetos vecinos de una clase; `--strata-min-cells`, `--plots` reparte unidades de muestreo con asignación de Neyman; `--cover-height` fija la altura de la cobertura): polígonos, ráster y tabla resumen con superficie, media, desviación y coeficiente de variación.
  - Clasificación: reglas de texto (`--rules`) o bosque aleatorio (`--samples`, `--class-field`, `--trees`, `--seed`) con exactitud sobre los objetos fuera de bolsa, kappa y precisión por clase.
  - Resultados: polígonos con huecos y atributos, ráster de objetos (entero de 32 bits), ráster de clases y CSV. La interfaz puede escribir además un GeoPackage con los objetos y los estratos (tablas `objetos` y `estratos`).

- **Árboles (`trees`):**
  - Cimas: máximo local en ventana circular o cuadrada, fija o variable `a + b·h + c·h²`. La ventana es un diámetro con el borde incluido, y en una meseta gana la primera celda.
  - Copas: `dalponte` (alcance en píxeles, regla del 5 %, 4-vecinos), `silva` (vecino más cercano con radio `factor × altura`) y `watershed` (cuencas con tolerancia).
  - Suavizado previo del CHM (`--smooth`, media, mediana o gauss) y cimas a mano (`--tops-file`).
  - Resultados: CSV, ráster de copas, polígonos de copa y ápices en shapefile, y una copia de la nube con `treeID` por punto (`--cloud`). La interfaz escribe además un GeoPackage con los ápices y las copas (tablas `apices` y `copas`), y pone las métricas de *Extraer muestras* de cada copa como campos de sus polígonos.
  - Comprobación con inventario de campo (`--field`): acertados, falsos positivos, omitidos, exhaustividad, precisión y error de altura.

`hipso <comando> --help` lista todos los parámetros.

## Validación

- **FUSION 4.61:**
  - Clasifica exactamente los mismos puntos de suelo (189 113 en `fusa.laz` y 18 371 741 en 27,8 millones de puntos).
  - El CHM es idéntico celda a celda.
  - Coinciden las 71 columnas de métricas de elevación y las 34 de intensidad. En las de estratos solo quedan diferencias de 1e-4 por redondeo float32.
- **MCC-LIDAR 2.1:** el 94,8 % de los puntos coincide. En el MDT resultante la mediana de diferencias es 0 m y el p90 2,5 cm.
- **lidR 4.3 (caja negra):** los ápices coinciden exactamente (11 007, 4 352 y 8 528 árboles en tres configuraciones sobre un CHM de 2,1 M de celdas). Las copas cubren casi las mismas celdas (Dalponte 99,99 %, Silva 99,7 %) y asignan el mismo árbol en el 97 % (Dalponte) y el 99 % (Silva) de ellas; la diferencia está en píxeles disputados entre dos árboles.
- **Inventario de campo** (`datos_1`, 16 unidades de muestreo, 125 árboles): con ventana variable 3 + 0,1·h se acierta el 86 % de los árboles (precisión 76 %, F = 0,81) con un sesgo de altura de +0,55 m.
- **Herramientas de la versión 0.8:** se contrastaron con R (terra 1.9.11 sobre GDAL 3.8.5, sf 1.1.0 sobre GEOS, lidR 4.2.3 y deldir 2.0.4), usado como caja negra. Cada comparación se explica a continuación, con su método y sus datos.
  - **`flightlines`:** se comparó con `lidR::retrieve_flightlines(dt = 30)` sobre la hoja Z4 de QULTIVA, de 55 millones de puntos. Salen las mismas seis líneas, con el mismo número de puntos en cada una y con tiempos de inicio y fin que difieren menos de 0,5 ms.
  - **`voronoi`:** se comparó con `st_voronoi` de GEOS. Con 400 celdas recortadas, la diferencia de área máxima es de 1,1e-5 m², y con 30 000 puntos es de 1,8e-3 m² por punto, en celdas de hasta 11 800 m². `deldir` pierde área en el borde de la nube de puntos, de modo que allí no sirve de referencia.
  - **`voronoi-density`:** se comparó con las áreas de GEOS sobre 64 celdas interiores, con una diferencia máxima de 1,2e-6 puntos/m². Un punto exactamente sobre el borde de una celda del ráster pertenece a la celda de arriba, porque el lado sur es de la celda superior.
  - **`calc`:** se comparó con `terra` en cuatro expresiones sobre 49 millones de celdas, con una diferencia máxima de 3,6e-6 (la precisión de la coma flotante de 32 bits).
  - **`vectorize`:** se comparó con `terra::patches` (con 4 y con 8 vecinas) y sale el mismo número de polígonos y la misma área por clase, con hasta 1 865 polígonos en una clase.
  - **`cutfill`:** se comparó con `terra` con tres diseños (un ráster, una cota y un plano inclinado), y la diferencia relativa es de 5e-6 o menor.
  - **`piles`:** se comparó con `terra`, enmascarando con `mask(touches = FALSE)`. Las áreas son idénticas y los volúmenes coinciden a 6e-5. Con el valor por defecto de `terra` (`touches = TRUE`) las áreas salen hasta un 1 % mayores, y un contorno que pasa por los centros de las celdas cambia el resultado por la regla de empate.
  - **`flood`:** se comparó con la componente conexa de `z <= nivel` (`terra::patches`). Las áreas son idénticas y los volúmenes coinciden a 1e-3 en el nivel más bajo.
  - **`ridges`:** se comparó con el flujo D8 de `terra` sobre un MDT sin sumideros, y comparten el 99,4 % de las celdas en el drenaje y el 100 % en las crestas.
  - **`leastcost`:** el ráster de coste acumulado es igual al de `terra::costDist` en las 6 000 celdas, con una diferencia máxima de 4,3e-6, y el tiempo de Tobler coincide con la fórmula cerrada (76,450 s cuesta arriba al 10 % y 53,87 s cuesta abajo).
  - **`trees3d`:** se comparó con `lidR::segment_trees(li2012(dt1, dt2, R = 0, Zu, hmin))` de lidR 4.2.3, usado como caja negra, sobre la nube de CapitolForest sin los puntos de suelo y normalizada con el mismo MDT, con `--radius 0` y `--min-points 1` (que lidR no tiene). Con los parámetros por defecto (1,5, 2, 15 m y altura mínima 2 m), sobre 2,87 M de puntos (22 579 árboles frente a 22 875 de lidR): el 100 % de las cimas de HIPSO son cimas de lidR y el 98,7 % de las de lidR lo son de HIPSO; los 296 árboles que solo tiene lidR son sobre todo pequeños (de unas decenas de puntos); el 99,54 % de los puntos de cada árbol de HIPSO está en un solo árbol de lidR y el 99,74 % a la inversa, con el 100 % de los puntos de al menos 2 m asignados por los dos. Con otros parámetros (1, 1,5, 20 m y altura mínima 5 m) sobre un recorte de 300 × 300 m: 4 192 árboles frente a 4 199, con el 99,83 % de las cimas de lidR y el 99,94 % de coincidencia de puntos. Tarda 1,0 s frente a los 124 s de lidR (compilado sin OpenMP, así que no es una comparación de velocidad justa). La diferencia que queda es que lidR crea algunos árboles más pequeños donde aquí un punto se une al árbol vecino, y que lidR también asigna a un árbol los puntos por debajo de la altura mínima (aquí quedan sin árbol).
  - **`dtm-validate`, `canopy-gaps`, `voxel-metrics`, `tree-metrics`, `alpha3d`:** solo tienen pruebas con datos sintéticos de resultado conocido (un desplazamiento del MDT, un hueco rectangular, columnas con puntos a alturas dadas, conos de copa, una retícula y una esfera para la forma alfa, donde la envolvente es el 97 % del volumen de la esfera con 600 puntos) y una ejecución sobre la nube de CapitolForest (4,9 M de puntos, 120 ha): `trees3d` encuentra 11 106 árboles frente a 9 975 del CHM suavizado, y las alturas de las cimas coinciden con las del ráster de `voxel-metrics`. No se han comparado con otro software ni con un inventario de campo.
  - **Sin contraste externo:** el orden de Strahler, la pendiente máxima, las barreras y las zonas preferidas de `leastcost`, y las zonas encharcadas solo tienen pruebas unitarias. Tampoco se ha comparado `voronoi` con scipy, `calc` ni `vectorize` con las herramientas de GDAL, ni `flood` con un paquete de hidrología.
- **Edificios:** el 83 % de los puntos de edificio que clasificó el proveedor de `fusa`, con un 98 % de precisión, medido con el método anterior; el método actual usa puntos de tejado, no un ráster.

## Rendimiento

| Nube de 6,9 M de puntos, 4 núcleos | HIPSO | lidR 4.3 (R) |
|---|---|---|
| Suelo CSF (*) | 26 s · 176 MB | 620 s |
| MDT por TIN | 5 s · 142 MB | 46 s |
| CHM sin huecos | 8 s · 269 MB | 72 s |
| Métricas | 4 s · 48 MB | 10 s |
| Árboles | 0,4 s · 57 MB | 25 s |

(*) Medido antes de que la tela de CSF uniera cada partícula con sus ocho vecinos (antes, con los cuatro contiguos); ahora tarda entre 1,0 y 2,4 veces más, según la nube.

| Nube de 27,8 M de puntos | FUSION | HIPSO (LAS) | HIPSO (LAZ) |
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

Ver [PUBLICAR.md](PUBLICAR.md): cómo se generan y publican los paquetes de Windows, Linux y macOS.

## Compilar

```sh
cargo build --release      # target/release/hipso y hipso-gui
cargo test
```

Requiere Rust 1.90 o superior. No hay dependencias de C.

| Crate | Función |
|---|---|
| `crates/core` | Motor: lectura y escritura LAS/LAZ, lector JPEG 2000 propio (`jp2/`), teselado, filtros, rejillas y GeoTIFF, métricas, árboles, recorte, visor (`view`) y jobs |
| `apps/cli` | `hipso` (línea de comandos) |
| `apps/gui` | `hipso-gui` (eframe/egui) |

## Interfaz gráfica

`hipso-gui` es el visor y editor de nubes de puntos, rásteres y capas vectoriales, con las herramientas de proceso a un clic. Su ventana tiene una cabecera (abrir, guardar la vista en PNG, extensión completa y encuadre anterior, y el buscador de comandos con Ctrl+K), una cinta de opciones con seis pestañas (*Archivo*, *Vista*, *LiDAR*, *Vectorial*, *Raster* y *Flujos*), un panel lateral con cuatro pestañas (*Contenido*, *Información*, *Edición* y *Simbología*), el área de vistas (planta, perfil y 3D), el Registro y la barra de estado, en tema claro u oscuro. El capítulo 4 del manual explica cada botón; aquí solo se resumen los hechos técnicos.

- **Abrir:** LAS, LAZ y COPC, PLY y E57 (convertidos a LAZ), ASCII XYZ, rásteres e imágenes (GeoTIFF, JP2, ASC y otros) y capas vectoriales (shapefile, GeoJSON, GeoPackage, KML/KMZ). *Recientes* guarda los diez últimos archivos y los diálogos empiezan en la última carpeta usada. Se puede arrastrar y soltar. Una decimación (*Máx.* de *Vista → Visualización*) mantiene la nube en memoria, y se carga en segundo plano.
- **Ventanas:** planta, perfil y 3D, solas o combinadas; el 3D puede seguir la extensión de la planta.
- **Color de los puntos:** elevación (azul-rojo, tierra o viridis), clasificación con colores editables, intensidad, retorno, ID de fuente, archivo, RGB e ID de árbol (`treeID`), con filtros por clase y por tipo de retorno y un rango de elevación propio.
- **TIN sombreado** (con las clases que se elijan) y **curvas de nivel** con equidistancia.
- **Herramientas del visor:** zoom (clic, ventana, rueda, encuadre anterior y completo), desplazar, consultar punto (Q; en planta, perfil y 3D, con todos los atributos del punto), medir (W, con distancia 3D, área y perímetro), densidad de puntos y perfil (tres clics; flechas y rueda mueven la caja).
- **LiDAR, Raster y Vectorial:** cada herramienta abre una ventana con sus opciones, y las mismas opciones existen como orden de `hipso`. Las rutas de salida se calculan a partir de la entrada y los ajustes, se pueden editar a mano y piden confirmación antes de sobrescribir. Al terminar, los rásteres y capas nuevos aparecen en el mapa.
- **Flujos:** el constructor de nodos encadena las herramientas y guarda el flujo como `.model.json`.
- **Atajos:** Ctrl+O abrir, Ctrl+K comandos, F1 manual, Q consultar, W medir, Z zoom, X desplazar, T TIN, +/− tamaño de punto, N norte arriba.

**Capas.** El panel *Contenido* es como la tabla de contenidos de un SIG: un árbol con el mapa, las capas en orden de dibujo (que se arrastran para reordenarlas) y las nubes de puntos; cada capa tiene su casilla de visibilidad, su símbolo y su nombre; el botón derecho abre zoom, tabla, guardar, subir, bajar y quitar, y la leyenda de las capas con símbolos por valores cuelga de su fila. Lo que cambia con la capa elegida está en el panel lateral, no en la cinta:

- Pestaña *Simbología* (vectorial): símbolo único, valores únicos (con un color por valor), colores graduados (intervalos iguales o cuantiles, rampa y número de clases) y tamaño proporcional; colores de trazo y relleno, opacidad, grosor, tamaño de punto, y etiquetas con campo, tamaño, color y halo.
- Pestaña *Edición*, con una capa vectorial elegida: digitalizar (con ajuste a vértices y a bordes), reformar, seleccionar (clic, rectángulo, por localización o por expresión), mover vértices (doble clic en un lado inserta uno, botón derecho en un vértice lo quita), mover y transformar elementos, unir, separar y agrupar, copiar y pegar, comprobar topología, eliminar, deshacer y rehacer (Ctrl+Z, Ctrl+Y), guardar en su sitio y guardar como. La tabla de atributos edita celdas, añade campos, ordena la selección con la del mapa y selecciona por atributo.
- Pestaña *Simbología*, ráster e imágenes: bandas de rojo, verde y azul con ajustes preestablecidos (color natural, infrarrojo), estirado (sin estirar, mínimo–máximo, percentiles, desviaciones, cortes manuales), brillo, contraste y gamma. Los rásteres de una banda se estiran con una rampa, se clasifican o se dibujan con relieve sombreado.

Las imágenes se leen **por ventana y nivel de pirámide**: se usan las vistas reducidas que el GeoTIFF lleva dentro y solo se decodifican los mosaicos que se ven, de modo que una ortofoto de miles de millones de píxeles abre al instante y se afina al acercar. Los JPEG con color YCbCr se convierten a RGB. En JPEG 2000 (`crates/core/src/jp2/`, escrito a partir de la norma T.800: cajas JP2, marcadores del codestream, paquetes con árboles de etiquetas, los cinco órdenes de progresión, codificador MQ/EBCOT con sus opciones, wavelets 5/3 y 9/7 y transformadas de color RCT/ICT) las vistas reducidas son los niveles de la wavelet y solo se decodifican las teselas de la ventana. Medido con las bandas de Sentinel-2: una banda de 10 m (10 980 × 10 980) entera en 2,3 s en 10 núcleos, y una ventana de 480 × 440 celdas en unas centésimas de segundo; las bandas de 20 y 60 m y la imagen TCI en color coinciden valor a valor con OpenJPEG (los archivos con wavelet 9/7 difieren como mucho en 1–2 niveles, por el redondeo).

**Edición de puntos.** La pestaña *Edición* del panel lateral, con una nube elegida, trabaja como las herramientas vectoriales de un SIG:

- Selecciona con rectángulo (un clic elige los puntos cercanos), polígono (clic derecho o doble clic cierra), lazo o círculo, en la planta o en el perfil.
- Selecciona también con la varita mágica (en la planta): un clic elige los puntos conectados con el pulsado y parecidos a él, siguiendo una superficie lisa (por el plano local de cada punto) o un volumen, con todos los puntos del archivo y no con la muestra que dibuja la vista.
- Combina selecciones: nueva, añadir (Mayús), restar (Alt), intersecar (Ctrl), invertir (Ctrl+I) y todo (Ctrl+A). Un filtro de atributos (clases, elevación, intensidad, tipo de retorno) restringe lo que se selecciona, o selecciona por atributos sin región.
- Elimina (Supr) o cambia la clase de la selección. Deshacer y rehacer (Ctrl+Z, Ctrl+Y), con un historial visible. Los puntos seleccionados se pintan en cian.
- Guardar crea un archivo nuevo (nunca sobrescribe el original). Los cambios son reglas: en pantalla se aplican a los puntos que se ven, y al guardar se aplican a **todos** los puntos del archivo en una sola pasada por todos los núcleos, así que el resultado es exacto aunque la nube esté decimada y la memoria no crece. En el borde de un polígono, unos pocos puntos pueden contarse distinto en pantalla (coordenadas decimales en f32) que en el archivo; el archivo es el exacto.
- Por línea de comandos: `hipso edit nube.laz --regions zonas.shp --class 5 --set-class 7`.

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

Ver [THIRD_PARTY_LICENSES.md](../THIRD_PARTY_LICENSES.md) y `NOTICE`.


## Manual y licencia

El manual de uso está en `docs/MANUAL.md` y dentro del programa (Inicio → Manual, o F1). HIPSO es de uso gratuito: ver `LICENSE`; los componentes de terceros están en `NOTICE` y `THIRD_PARTY_LICENSES.md`.
