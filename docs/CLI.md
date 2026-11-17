# Referencia de herramientas

Cada herramienta del programa es una orden de la línea de comandos, y la interfaz gráfica y el constructor de flujos ejecutan estas mismas órdenes. Por eso esta página sirve para las tres cosas: para cada herramienta dice qué hace, dónde está en la cinta y qué significa cada una de sus opciones, con su valor por defecto y los valores que admite. Las ventanas de la interfaz muestran estas mismas opciones, con la misma ayuda, y el constructor de flujos las ofrece como puertos y parámetros de cada nodo.

La página se genera con `scripts/gen_cli_docs.py` a partir de la definición de las órdenes del propio programa, de modo que lista exactamente lo que admite la versión publicada. Todas las órdenes aceptan además `--help`, que imprime lo mismo en la terminal. Cuando una opción no lleva valor por defecto, la herramienta decide por sí misma o no actúa hasta que se le dé un valor.

## Revisar y preparar nubes

### hipso info

Informe completo de archivos LAS/LAZ: cabecera, clases, retornos, densidad, avisos

**En la interfaz:** panel lateral → Información. El método se explica en el apartado 4.9.

| Opción | Valor | Por defecto | Qué hace |
|---|---|---|---|
| `INPUTS` | archivo o carpeta | ninguno | Archivos LAS o LAZ de los que se saca el informe. **Obligatoria.** Se puede repetir. |

### hipso catalog

Revisa una entrega de hojas solo con sus cabeceras (no se lee ningún punto): mismo sistema de coordenadas, hojas repetidas, solapes, huecos dentro del bloque. Una carpeta equivale a sus LAS/LAZ

**En la interfaz:** sin botón propio (la línea de comandos y el constructor de flujos). El método se explica en el apartado 12.

| Opción | Valor | Por defecto | Qué hace |
|---|---|---|---|
| `INPUTS` | archivo o carpeta | ninguno | Archivos LAS/LAZ (o carpetas con ellos) de la entrega que se revisa. **Obligatoria.** Se puede repetir. |
| `--tolerance` | número | `1` | Cuánto (m) pueden solaparse las hojas o quedarse cortas unas de otras sin que cuente. |

### hipso crs

Da a las nubes sin sistema de coordenadas el de otra nube o un código EPSG

**En la interfaz:** LiDAR → Preparar → Asignar proyección. El método se explica en el apartado 7.1.

| Opción | Valor | Por defecto | Qué hace |
|---|---|---|---|
| `INPUTS` | archivo o carpeta | ninguno | Nubes LAS o LAZ a las que se asigna el sistema de coordenadas. **Obligatoria.** Se puede repetir. |
| `--from` | archivo o carpeta | ninguno | Una nube que tiene el sistema de coordenadas que copiar. |
| `--epsg` | texto o número | ninguno | Código EPSG del sistema de coordenadas, p. ej. 25830. |
| `--replace` | casilla | desactivada | Sustituye el sistema de coordenadas de nubes que ya tienen uno. |
| `-o`, `--output` | archivo o carpeta | ninguno | Salida (solo una entrada); por defecto: <resultados>/<entrada>_crs.<ext>. |

### hipso import

Importa una nube de puntos de otro formato como LAS/LAZ: PLY (ASCII o binario) o E57 (escáneres terrestres y móviles; cada escaneo se coloca con su pose y su número queda como identificador de fuente del punto)

**En la interfaz:** Abrir (un PLY o un E57 se convierte al abrirlo). El método se explica en el apartado 10.

| Opción | Valor | Por defecto | Qué hace |
|---|---|---|---|
| `INPUT` | archivo o carpeta | ninguno | El archivo que convertir. **Obligatoria.** |
| `-o`, `--output` | archivo o carpeta | ninguno | .las o .laz de salida; por defecto: <resultados>/<entrada>_ply.laz o _e57.laz (nunca un archivo existente) |
| `--epsg` | texto o número | ninguno | Código EPSG del sistema de coordenadas de los puntos (PLY y E57 rara vez lo dicen) |

### hipso merge-clouds

Une dos o más nubes en una y decide qué hacer con los puntos duplicados

**En la interfaz:** LiDAR → Calidad → Unir nubes. El método se explica en el apartado 6.5.

| Opción | Valor | Por defecto | Qué hace |
|---|---|---|---|
| `INPUTS` | archivo o carpeta | ninguno | Nubes LAS o LAZ que se unen (dos o más). **Obligatoria.** Se puede repetir. |
| `-o`, `--output` | archivo o carpeta | ninguno | Nube unida; por defecto, en la carpeta de resultados. |
| `--dups` | una de: `keep`, `mark`, `delete` | `keep` | Qué hacer con los duplicados: keep (no buscarlos), mark (marcarlos como retenidos) o delete (quitarlos). |
| `--also` | texto o número | ninguno | Atributos que también deben coincidir: class, gps, intensity, return, count, source. Se puede repetir. |

### hipso dedup

Quita los puntos duplicados (mismas x, y, z y los atributos pedidos)

**En la interfaz:** LiDAR → Preparar → Quitar duplicados. El método se explica en el apartado 6.4.

| Opción | Valor | Por defecto | Qué hace |
|---|---|---|---|
| `INPUTS` | archivo o carpeta | ninguno | Nubes LAS o LAZ de las que se quitan los puntos repetidos. **Obligatoria.** Se puede repetir. |
| `-o`, `--output` | archivo o carpeta | ninguno | Archivo de salida (solo una entrada); por defecto, en la carpeta de resultados. |
| `--all-copies` | casilla | desactivada | Borra todas las copias, no solo las sobrantes. |
| `--also` | texto o número | ninguno | Atributos que también deben coincidir: class, gps, intensity, return, count, source. Se puede repetir. |

### hipso tile

Divide las nubes en teselas cuadradas (con un margen opcional), un archivo por tesela

**En la interfaz:** sin botón propio (la línea de comandos y el constructor de flujos). El método se explica en el apartado 12.

| Opción | Valor | Por defecto | Qué hace |
|---|---|---|---|
| `INPUTS` | archivo o carpeta | ninguno | Nubes LAS o LAZ que dividir en teselas. **Obligatoria.** Se puede repetir. |
| `-o`, `--output` | archivo o carpeta | ninguno | Nombre base de las teselas; por defecto: <resultados>/<entrada>_tile.<ext>. |
| `--size` | número | `1000` | Lado de cada tesela (m) |
| `--buffer` | número | `0` | Margen (m) que cada tesela toma de sus vecinas. |

### hipso clip

Recorta nubes con polígonos, parcelas o rectángulos

**En la interfaz:** LiDAR → ALS → Extraer muestras. El método se explica en el apartado 7.3.

| Opción | Valor | Por defecto | Qué hace |
|---|---|---|---|
| `INPUTS` | archivo o carpeta | ninguno | Nubes LAS o LAZ que recortar. **Obligatoria.** Se puede repetir. |
| `--regions` | archivo o carpeta | ninguno | Polígonos (.shp), centros de parcelas (.shp de puntos, con --radius) o una lista (.csv/.txt) |
| `--rect` | texto o número | ninguno | Un rectángulo: minx,miny,maxx,maxy (se repite para varios) Se puede repetir. |
| `--circle` | texto o número | ninguno | Una parcela circular: x,y,radio (se repite para varias) Se puede repetir. |
| `--name-field` | texto o número | ninguno | El atributo que da nombre a cada región: el nombre de un campo o su posición contando desde 1. |
| `--buffer` | texto o número | ninguno | Amplía cada región esta distancia en unidades del mapa (metros en un sistema proyectado); si es negativa, la reduce. Las esquinas quedan redondeadas. |
| `--split-parts` | casilla | desactivada | Divide los elementos de varias partes en una región por parte (llamadas <nombre>_p1, <nombre>_p2…) |
| `--radius` | texto o número | ninguno | Radio de las parcelas dadas como puntos. |
| `--radius-field` | texto o número | ninguno | Atributo que guarda el radio propio de cada punto (metros) |
| `--square` | casilla | desactivada | Las parcelas dadas como puntos son cuadradas en lugar de circulares. |
| `--side` | texto o número | ninguno | Lado de las parcelas cuadradas, en metros (o los campos que guardan los lados) |
| `--side-x-field` | texto o número | ninguno | Campo de las regiones con el lado X de la parcela cuadrada (m) |
| `--side-y-field` | texto o número | ninguno | Campo de las regiones con el lado Y de la parcela cuadrada (m) |
| `--sample` | una de las opciones | ninguno | Genera las parcelas en lugar de leerlas: al azar o en una cuadrícula regular. Valores: `random`: Parcelas al azar (--count), con una distancia mínima entre centros; `systematic`: Parcelas en una cuadrícula regular (--spacing). |
| `--count` | número | `10` | Muestreo aleatorio: número de parcelas. |
| `--spacing` | texto o número | ninguno | Muestreo sistemático: separación entre centros en metros, X o X,Y. |
| `--min-dist` | texto o número | ninguno | Muestreo aleatorio: distancia mínima entre centros (por defecto: el doble del tamaño de la parcela) |
| `--seed` | número | `1` | Semilla del muestreo aleatorio: la misma semilla da las mismas parcelas. |
| `--limit` | archivo o carpeta | ninguno | Las parcelas generadas se quedan dentro de estos polígonos (un rodal, por ejemplo) |
| `--layout` | archivo o carpeta | ninguno | Dónde se guarda la capa con las parcelas; por defecto: junto a los resultados. |
| `--where` | texto o número | ninguno | Conserva solo las regiones cuyo atributo tiene un valor: CAMPO=VALOR. |
| `--mode` | una de las opciones | `merge` | Qué se escribe: una nube con todo (merge), una nube por región (each) o solo la capa con las métricas (metrics) Valores: `merge`: Una nube con los puntos de todas las unidades de muestreo; `each`: Una nube por unidad de muestreo; `metrics`: Sin nubes: solo la capa de unidades de muestreo con sus métricas. |
| `-o`, `--output` | archivo o carpeta | ninguno | Archivo de salida (un nombre base con --mode each); por defecto: junto a los resultados. |
| `--class` | texto o número | ninguno | Clases que se conservan: 2,5 o ~7,18 para todas menos esas. |
| `--returns` | texto o número | ninguno | Retornos que se conservan: A, F, L o números (p. ej. 1,2) |
| `--zmin` | texto o número | ninguno | Elevación mínima (m) de los puntos que se conservan. |
| `--zmax` | texto o número | ninguno | Elevación máxima (m) de los puntos que se conservan. |
| `--dtm` | archivo o carpeta | ninguno | Modelo del suelo: escribe alturas sobre el suelo en lugar de elevaciones. |
| `--z-offset` | número | `0` | Se suma a cada elevación escrita (m) |
| `--decimate` | número | `1` | Conserva un punto de cada tantos. |
| `--exclusive` | casilla | desactivada | Regiones que se solapan: un punto va solo a la primera. |
| `--keep-empty` | casilla | desactivada | Escribe archivos para las regiones sin puntos. |
| `--max-open` | número | `64` | Archivos abiertos a la vez con --mode each. |
| `--metrics` | archivo o carpeta | ninguno | Escribe además las CloudMetrics de cada región en este CSV. |
| `--above` | número | `2` | Métricas: altura de corte para la cobertura. |
| `--metrics-format` | una de las opciones | ninguno | Formato de la tabla de métricas: shapefile de las regiones, tabla de texto o ambos. Valores: `vector`: Solo shapefile; `csv`: Solo tabla de texto (lo mejor para ejecuciones muy grandes); `both`: Both. |
| `--minht` | número | `0` | Métricas: solo los retornos por encima de esta altura. |
| `--metrics-columns` | texto o número | ninguno | Métricas: las columnas que se escriben, por nombre (`Elev P90,Elev mean,Int mean`); por defecto: todas. Se puede repetir. |

### hipso filter

Une, filtra, normaliza o convierte nubes en una pasada (LAS o LAZ según la extensión de la salida)

**En la interfaz:** LiDAR → Preparar → Exportar puntos (con normalizar). El método se explica en el apartado 7.1.

| Opción | Valor | Por defecto | Qué hace |
|---|---|---|---|
| `INPUTS` | archivo o carpeta | ninguno | Nubes LAS o LAZ que unir, filtrar o convertir. **Obligatoria.** Se puede repetir. |
| `-o`, `--output` | archivo o carpeta | ninguno | Salida; por defecto: <resultados>/<entrada>_filtrada.<ext>. |
| `--class` | texto o número | ninguno | Clases que se conservan: 2,5 o ~7,18 para todas menos esas. |
| `--returns` | texto o número | ninguno | Retornos que se conservan: A, F, L o números (p. ej. 1,2) |
| `--zmin` | texto o número | ninguno | Elevación mínima (m) de los puntos que se usan. |
| `--zmax` | texto o número | ninguno | Elevación máxima (m) de los puntos que se usan. |
| `--decimate` | número | `1` | Conserva un punto de cada tantos. |
| `--dtm` | archivo o carpeta | ninguno | Escribe alturas sobre este modelo del suelo en lugar de elevaciones. |
| `--z-offset` | número | `0` | Se suma a cada elevación escrita (m) |

### hipso thin

Adelgaza (diezma) una nube: por celda, por celda 3D, un punto de cada n, al azar o hasta una densidad. Cada entrada tiene su propio archivo adelgazado salvo que se dé -o (entonces se unen)

**En la interfaz:** LiDAR → Preparar → Adelgazar nube. El método se explica en el apartado 7.1.

| Opción | Valor | Por defecto | Qué hace |
|---|---|---|---|
| `INPUTS` | archivo o carpeta | ninguno | Nubes LAS o LAZ que adelgazar. **Obligatoria.** Se puede repetir. |
| `-o`, `--output` | archivo o carpeta | ninguno | Archivo de salida (las entradas se unen); por defecto: un archivo adelgazado por cada entrada. |
| `--method` | una de las opciones | `lowest` | Cómo se adelgaza la nube: un punto por celda (más bajo, más alto, central o percentil), por celda 3D, uno de cada n, quitando uno de cada n, al azar o hasta una densidad. Valores: `lowest`: Punto más bajo de cada celda; `highest`: Punto más alto de cada celda; `nearest`: Punto más cercano al centro de cada celda; `percentile`: Punto en el percentil --percentile de las elevaciones de cada celda; `voxel`: Más cercano al centro de cada celda 3D (--vcell high); `every`: Uno de cada n puntos (--n); `remove`: Quita uno de cada n puntos (--n); `random`: Random sample (--fraction); `density`: Muestra aleatoria hasta --density puntos por m². |
| `--cell` | número | `1` | Tamaño de celda en metros (métodos por celda) |
| `--vcell` | número | `1` | Altura de la celda 3D en metros (vóxel) |
| `--percentile` | número | `50` | Percentil, de 0 a 100, que se conserva en cada celda (50 es la mediana) |
| `--n` | número | `2` | Con `every` y `remove`: cada cuántos puntos se conserva (o se quita) uno. |
| `--fraction` | número | `0.5` | Proporción de puntos que se conservan, de 0 a 1 (aleatorio) |
| `--density` | número | `1` | Puntos por m² que se dejan (densidad) |
| `--seed` | número | `1` | Semilla de los métodos aleatorios. |
| `--class` | texto o número | ninguno | Clases que se conservan: 2,5 o ~7,18 para todas menos esas. |
| `--returns` | texto o número | ninguno | Retornos que se conservan: A, F, L o números (p. ej. 1,2) |
| `--zmin` | texto o número | ninguno | Elevación mínima (m) de los puntos que se usan. |
| `--zmax` | texto o número | ninguno | Elevación máxima (m) de los puntos que se usan. |
| `--decimate` | número | `1` | Conserva un punto de cada tantos. |

### hipso edit

Edita puntos como si fueran elementos: borra o reclasifica los puntos dentro de polígonos que cumplen las pruebas de atributos; escribe un archivo nuevo, nunca el original

**En la interfaz:** panel lateral → Edición (nubes de puntos). El método se explica en el apartado 6.

| Opción | Valor | Por defecto | Qué hace |
|---|---|---|---|
| `INPUT` | archivo o carpeta | ninguno | Nube LAS o LAZ que editar. **Obligatoria.** |
| `-o`, `--output` | archivo o carpeta | ninguno | Nube de salida; por defecto, en la carpeta de resultados con el nombre de la entrada. |
| `--regions` | archivo o carpeta | ninguno | Polígonos, centros de parcelas (con --radius) o una lista; por defecto: toda la nube. |
| `--radius` | texto o número | ninguno | Radio (m) de las parcelas cuando las regiones son puntos. |
| `--outside` | casilla | desactivada | Selecciona en cambio los puntos fuera de las regiones. |
| `--class` | texto o número | ninguno | Solo se seleccionan los puntos de estas clases (2,5 o ~7,18) |
| `--zmin` | texto o número | ninguno | Elevación mínima (m) de los puntos que se seleccionan. |
| `--zmax` | texto o número | ninguno | Elevación máxima (m) de los puntos que se seleccionan. |
| `--delete` | casilla | desactivada | Borra los puntos seleccionados. |
| `--set-class` | texto o número | ninguno | Cambia los puntos seleccionados a esta clase. |

### hipso apply-edits

Aplica a una o más nubes las ediciones guardadas desde el visor (un archivo de ediciones JSON) y escribe archivos nuevos, nunca el original

**En la interfaz:** panel lateral → Edición → Guardar ediciones… y Cargar ediciones…. El método se explica en el apartado 6.

| Opción | Valor | Por defecto | Qué hace |
|---|---|---|---|
| `INPUTS` | archivo o carpeta | ninguno | Nubes LAS o LAZ a las que se aplican las ediciones. **Obligatoria.** Se puede repetir. |
| `--edits` | archivo o carpeta | ninguno | Archivo de ediciones, el que escribe el visor con «Guardar ediciones…». **Obligatoria.** |
| `-o`, `--output` | archivo o carpeta | ninguno | Nube de salida (solo una entrada); por defecto, en la carpeta de resultados. |

### hipso colorize

Colorea los puntos a partir de una imagen del mismo lugar (una ortofoto): cada punto toma el rojo, el verde y el azul de su píxel. Los formatos sin color se escriben como formato 7

**En la interfaz:** LiDAR → Preparar → Colorear con imagen. El método se explica en el apartado 7.1.

| Opción | Valor | Por defecto | Qué hace |
|---|---|---|---|
| `INPUTS` | archivo o carpeta | ninguno | Nubes LAS o LAZ a las que se da el color de la imagen. **Obligatoria.** Se puede repetir. |
| `--image` | archivo o carpeta | ninguno | La imagen (GeoTIFF, JPEG 2000…), en el sistema de coordenadas de las nubes. **Obligatoria.** |
| `--bands` | número | `1` | Bandas del rojo, el verde y el azul, desde 1. Se puede repetir. |
| `--nir` | texto o número | ninguno | Banda del infrarrojo cercano, que se escribe en los formatos 8 y 10. |
| `--keep-outside` | casilla | desactivada | Los puntos fuera de la imagen conservan su color (por defecto: negro) |
| `-o`, `--output` | archivo o carpeta | ninguno | Salida (solo una entrada); por defecto: <resultados>/<entrada>_rgb.<ext>. |

### hipso intensity

Normaliza la intensidad por la distancia al escáner, I · (R / R_ref)^a, a partir de la trayectoria (tiempo GPS, x, y, z) o de la altitud de vuelo y el ángulo de barrido

**En la interfaz:** LiDAR → Preparar → Normalizar intensidad. El método se explica en el apartado 7.1.

| Opción | Valor | Por defecto | Qué hace |
|---|---|---|---|
| `INPUTS` | archivo o carpeta | ninguno | Nubes LAS o LAZ cuya intensidad se normaliza. **Obligatoria.** Se puede repetir. |
| `--trajectory` | archivo o carpeta | ninguno | Trayectoria del escáner: un archivo de texto con líneas «tiempo x y z». |
| `--altitude` | texto o número | ninguno | Altitud del vuelo (m, mismo datum vertical que los puntos), cuando no hay trayectoria. |
| `--exponent` | número | `2` | Exponente a de la distancia: 2 para superficies (suelo, dosel), alrededor de 3 para cables. |
| `--reference` | texto o número | ninguno | Distancia de referencia R_ref (m); por defecto: la distancia media de cada nube. |
| `-o`, `--output` | archivo o carpeta | ninguno | Salida (solo una entrada); por defecto: <resultados>/<entrada>_intensidad.<ext>. |

### hipso swath

Diferencia de altura entre pasadas como GeoTIFF, con las cifras del solape

**En la interfaz:** LiDAR → Calidad → Entre pasadas. El método se explica en el apartado 7.1.

| Opción | Valor | Por defecto | Qué hace |
|---|---|---|---|
| `INPUTS` | archivo o carpeta | ninguno | Nubes LAS o LAZ aéreas con ID de fuente o tiempo GPS. **Obligatoria.** Se puede repetir. |
| `-o`, `--output` | archivo o carpeta | ninguno | Ráster de salida; por defecto, en la carpeta de resultados. |
| `--cell` | número | `1` | Tamaño de celda del ráster de diferencias (m) |
| `--returns` | texto | `last` | Retornos que se usan: all, last, single. |
| `--by-cloud` | casilla | desactivada | Toma cada nube como una pasada en lugar del identificador de fuente del punto. |
| `--gps-gap` | texto o número | ninguno | Halla las pasadas a partir de los saltos del tiempo GPS mayores que esto (s), cuando el identificador de fuente del punto no las guarda. |

### hipso flightlines

Pasadas de una nube aérea halladas a partir de los saltos del tiempo GPS: cuántas hay, cuándo, cuántos puntos tienen y dónde. Con --assign, copias de las nubes que llevan el número de pasada

**En la interfaz:** LiDAR → Calidad → Líneas de vuelo. El método se explica en el apartado 7.1.

| Opción | Valor | Por defecto | Qué hace |
|---|---|---|---|
| `INPUTS` | archivo o carpeta | ninguno | Nubes LAS o LAZ aéreas con tiempo GPS. **Obligatoria.** Se puede repetir. |
| `--gap` | número | `30` | Un silencio del tiempo GPS mayor que esto (s) separa dos pasadas. |
| `--min-points` | número | `1000` | Los tramos con menos puntos no son pasadas. |
| `--min-duration` | número | `0` | Los tramos más cortos que esto (s) no son pasadas. |
| `--assign` | texto o número | ninguno | Escribe el número de pasada en copias de las nubes: `source-id` o `user-data`. |
| `--no-csv` | casilla | desactivada | No escribe la tabla de resultados en CSV. |
| `--no-vector` | casilla | desactivada | Sin capa de polígonos con el contorno de cada pasada. |
| `-o`, `--output` | archivo o carpeta | ninguno | Carpeta de la tabla y de las copias; por defecto: la carpeta de resultados de la primera nube. |

### hipso compare

Puntos de una nube que han cambiado frente a una nube de referencia (detección de cambios)

**En la interfaz:** LiDAR → Calidad → Comparar nubes. El método se explica en el apartado 7.1.

| Opción | Valor | Por defecto | Qué hace |
|---|---|---|---|
| `INPUT` | archivo o carpeta | ninguno | Nube en la que se buscan cambios. **Obligatoria.** |
| `--reference` | archivo o carpeta | ninguno | Nube(s) de referencia con las que se compara. **Obligatoria.** Se puede repetir. |
| `-o`, `--output` | archivo o carpeta | ninguno | Nube de salida con los puntos que han cambiado; por defecto, en la carpeta de resultados. |
| `--distance` | número | `0.5` | Distancia (m) a partir de la cual un punto ha cambiado. |
| `--neighbors` | número | `3` | Otros puntos cambiados que necesita un punto cambiado dentro del doble de la distancia. |

### hipso fit

Lleva una nube sobre una nube de referencia (punto más cercano iterativo) y escribe el resultado

**En la interfaz:** LiDAR → Calidad → Ajustar nubes. El método se explica en el apartado 7.1.

| Opción | Valor | Por defecto | Qué hace |
|---|---|---|---|
| `INPUT` | archivo o carpeta | ninguno | Nube que se mueve para que encaje con la de referencia. **Obligatoria.** |
| `--reference` | archivo o carpeta | ninguno | Nube(s) de referencia sobre las que se ajusta. **Obligatoria.** Se puede repetir. |
| `-o`, `--output` | archivo o carpeta | ninguno | Nube ajustada; por defecto, en la carpeta de resultados. |
| `--thin` | número | `1` | Adelgazamiento (m) de los puntos usados para hallar el movimiento. |
| `--max-xy` | número | `5` | Máximo desplazamiento horizontal (m) que se admite entre un punto y su pareja. |
| `--max-z` | número | `2` | Máxima diferencia de altura (m) que se admite entre un punto y su pareja. |
| `--iterations` | número | `30` | Máximo de iteraciones del ajuste. |
| `--stop` | número | `0.01` | Se detiene cuando el movimiento de una iteración baja de este valor (m) |
| `--shift-only` | casilla | desactivada | Solo traslada, no gira. |
| `--plane` | casilla | desactivada | Punto a plano: distancia al plano tangente del punto de referencia. |
| `--trim` | número | `1` | Proporción de los pares más cercanos que se usan para hallar el movimiento (1 los conserva todos) |

### hipso hag

Altura de cada punto sobre el suelo, a partir de los puntos de suelo que ya tiene la nube (un TIN con ellos): se escribe como el atributo adicional HeightAboveGround o, con --replace-z, en lugar de la elevación

**En la interfaz:** LiDAR → Preparar → Alturas sobre suelo. El método se explica en el apartado 6.3.

| Opción | Valor | Por defecto | Qué hace |
|---|---|---|---|
| `INPUTS` | archivo o carpeta | ninguno | Nubes LAS o LAZ con el suelo ya clasificado (clase 2) **Obligatoria.** Se puede repetir. |
| `-o`, `--output` | archivo o carpeta | ninguno | Salida (solo una entrada); por defecto: <resultados>/<entrada>_alturas.<ext>. |
| `--cell` | número | `0.5` | Celda de la superficie del suelo. |
| `--ground-class` | número | `2` | Clase de los puntos de suelo. |
| `--replace-z` | casilla | desactivada | Escribe la altura como elevación (una nube normalizada) en lugar de como atributo. |

## Clasificar y modelar superficies

### hipso classify

Clasifica ruido, suelo y edificios, tesela a tesela en paralelo, y escribe los puntos

**En la interfaz:** LiDAR → Preparar → Clasificar. El método se explica en el apartado 7.1.

| Opción | Valor | Por defecto | Qué hace |
|---|---|---|---|
| `INPUTS` | archivo o carpeta | ninguno | Nubes LAS o LAZ que clasificar; cada una es una hoja. **Obligatoria.** Se puede repetir. |
| `-o`, `--output` | archivo o carpeta | ninguno | LAS de salida, o LAZ si termina en .laz; por defecto: <resultados>/<entrada>_clasif<ajustes>. |
| `--neighbors` | número | `0` | Lee las hojas que estén a menos de tantos metros de cada hoja como contexto, de modo que sus bordes salgan como saldrían de todo el bloque, y escribe solo los puntos de la propia hoja (0: desactivado) |
| `--ground` | una de las opciones | ninguno | Cómo se encuentran los puntos de suelo (classify: kraus; dtm: classes, la clase 2 que ya trae la nube) Valores: `kraus`: Filtro iterativo de Kraus y Pfeifer (por defecto); `mcc`: Multiscale Curvature Classification (Evans & Hudak 2007); `csf`: Cloth Simulation Filter (Zhang et al. 2016); `pmf`: Progressive Morphological Filter (Zhang et al. 2003); `smrf`: Simple Morphological Filter (Pingel et al. 2013); `axelsson`: Progressive TIN densification (Axelsson 2000); `classes`: Puntos ya clasificados como suelo (clase 2). |
| `--filter-cell` | número | `5` | Kraus-Pfeifer: tamaño de celda de las superficies intermedias del filtro. |
| `--iterations` | número | `5` | Kraus-Pfeifer: número de iteraciones del filtro. |
| `--g` | número | `-2` | Kraus-Pfeifer: desplazamiento g de la función de peso (los residuos por debajo pesan 1); también --kp-g. |
| `--w` | número | `2.5` | Kraus-Pfeifer: anchura w de la función de peso por encima de g (los residuos por encima de g + w pesan 0); también --kp-w. |
| `--a` | número | `1` | Kraus-Pfeifer: pendiente a de la función de peso; también --kp-a. |
| `--b` | número | `4` | Kraus-Pfeifer: pendiente b de la función de peso; también --kp-b. |
| `--tolerance` | texto o número | ninguno | Kraus-Pfeifer: suelo = a esta distancia de la superficie final (por defecto: residuo <= g + w) |
| `--mcc-spacing` | número | `1.5` | MCC: tamaño de celda del dominio de escala 2 (aproximadamente el espaciado de los puntos) |
| `--mcc-threshold` | número | `0.3` | MCC: umbral de curvatura (m) por encima del cual un punto deja de ser suelo. |
| `--csf-resolution` | número | `0.5` | CSF: distancia entre las partículas de la tela. |
| `--csf-rigidness` | número | `1` | CSF: rigidez, de 1 (terreno escarpado) a 3 (llano) |
| `--csf-threshold` | número | `0.5` | CSF: altura sobre la tela hasta la que los puntos son suelo. |
| `--pmf-window` | número | `20` | PMF: mayor tamaño de ventana (m) de la apertura morfológica progresiva. |
| `--pmf-slope` | número | `1` | PMF: pendiente del terreno (subida sobre recorrido) que tolera el filtro. |
| `--smrf-cell` | número | `1` | SMRF: lado de celda (m) de las superficies que se comparan. |
| `--smrf-window` | número | `18` | SMRF: mayor radio de ventana (mayor que el edificio más grande) |
| `--smrf-slope` | número | `0.15` | SMRF: pendiente más empinada del terreno (subida sobre recorrido) |
| `--smrf-threshold` | número | `0.5` | SMRF: umbral de altura entre un punto y la superficie. |
| `--smrf-scalar` | número | `1.25` | SMRF: se suma al umbral por unidad de pendiente. |
| `--axelsson-seed-cell` | número | `20` | Axelsson: lado de las celdas cuyos puntos más bajos siembran el TIN (mayor que cualquier edificio) |
| `--axelsson-angle` | número | `8` | Axelsson: ángulo máximo (grados) entre un punto y el plano de la triangulación para aceptarlo como suelo. |
| `--axelsson-dist` | número | `1.4` | Axelsson: distancia máxima (m) de un punto al plano de la triangulación para aceptarlo como suelo. |
| `--axelsson-flat` | número | `0.1` | Axelsson: los puntos a esta distancia (m) de un triángulo son suelo sea cual sea el ángulo. |
| `--overlap` | casilla | desactivada | Aparta antes que nada, como clase 12 (solape), los puntos de las pasadas que solapan con otra: donde varias pasadas cubren una celda, la de menor ángulo de barrido medio se la queda. |
| `--overlap-cell` | número | `5` | Solape: lado de celda en metros. |
| `--noise` | una de las opciones | ninguno | Clasifica primero el ruido (clase 7) con este método. Valores: `isolated`: Puntos aislados en celdas de 3 x 3 x 3; `sor`: Eliminación estadística de valores atípicos sobre los k vecinos más próximos; `elm`: Ruido bajo: puntos muy por debajo de las alturas de su celda (mínimo local extendido). |
| `--sor-k` | número | `10` | Ruido (sor): número de vecinos con los que se calcula la distancia media de cada punto. |
| `--sor-m` | número | `3` | Ruido (sor): desviaciones típicas por encima de la distancia media. |
| `--noise-step` | número | `4` | Ruido (isolated): tamaño de celda en x e y. |
| `--noise-step-z` | número | `4` | Ruido (isolated): tamaño de celda en z. |
| `--elm-cell` | número | `10` | Ruido (elm): lado de celda en metros. |
| `--elm-threshold` | número | `3` | Ruido (elm): metros por debajo del siguiente punto de su celda y de los puntos más bajos de las celdas vecinas que debe estar el punto más bajo de una celda para ser ruido bajo. |
| `--isolated` | número | `5` | Ruido: el mayor número de otros puntos alrededor de un punto que aún se llama aislado. |
| `--vegetation` | casilla | desactivada | Clasifica la vegetación por altura sobre el suelo: baja (3), media (4) y alta (5) |
| `--wires` | casilla | desactivada | Clasifica los cables (14): líneas casi horizontales muy por encima del suelo. |
| `--poles` | casilla | desactivada | Clasifica los postes y torres (15): estructuras estrechas y aisladas que se elevan del suelo. |
| `--wire-min-height` | número | `5` | Cables: altura mínima sobre el suelo (m). |
| `--wire-radius` | número | `0` | Cables: radio del entorno con el que se mira la forma de cada punto (m; 0: según el espaciado de los puntos). |
| `--wire-linearity` | número | `0.7` | Cables: linealidad mínima del entorno (0 a 1). |
| `--wire-max-slope` | número | `0.5` | Cables: mayor pendiente de la línea (0 horizontal, 1 vertical). |
| `--wire-max-neighbours` | número | `1000` | Cables: mayor número de puntos alrededor de un punto de cable. |
| `--wire-max-width` | número | `0` | Cables: mayor anchura del entorno a través de la línea (m; 0: según el radio). |
| `--wire-min-length` | número | `10` | Cables: longitud mínima de una cadena de puntos de cable (m). |
| `--wire-min-straightness` | número | `10` | Cables: lo recta que debe ser una cadena (longitud sobre anchura). |
| `--wire-voxel` | número | `0.05` | Cables: lado de los vóxeles a los que se reducen los puntos antes de mirar su forma (m). |
| `--wire-link-gap` | número | `15` | Cables: mayor hueco (m) que se salva al unir trozos de conductor en fila (0: no se unen). |
| `--wire-piece-voxel` | número | `0.5` | Cables: lado (m) de los vóxeles con que se buscan trozos finos y sueltos de conductor (0: no se buscan). |
| `--wire-piece-min-length` | número | `1.2` | Cables: longitud mínima de un trozo fino de conductor (m). |
| `--wire-piece-max-width` | número | `0.12` | Cables: mayor desviación típica (m) de un trozo fino a través de su dirección. |
| `--wire-piece-min-points` | número | `6` | Cables: mínimo de puntos de un trozo fino de conductor. |
| `--wire-piece-isolation` | número | `1` | Cables: un trozo fino de conductor cuelga solo: a menos de esta distancia (m) no hay más que unos pocos vóxeles de otros grupos (0: no se comprueba). |
| `--wire-piece-max-others` | número | `12` | Cables: cuántos vóxeles de otros grupos tolera un trozo fino a esa distancia. |
| `--pole-min-height` | número | `2` | Postes: altura mínima sobre el suelo de los puntos de una estructura (m). |
| `--pole-radius` | número | `1.5` | Postes: los puntos más cercanos que esto pertenecen a la misma estructura (m). |
| `--pole-max-width` | número | `8` | Postes: mayor anchura de la estructura (m). |
| `--pole-min-extent` | número | `6` | Postes: lo mínimo que debe elevarse la estructura (m). |
| `--pole-head-length` | número | `12` | Postes: longitud (m) de la cabeza de una torre vista como una cruceta aislada (0: no se busca). |
| `--pole-thin-extent` | número | `3` | Postes: lo que debe elevarse un poste fino (m; 0: no se buscan). |
| `--pole-wire-reach` | número | `10` | Postes: con cables en la tesela, solo cuentan las estructuras a menos de esta distancia (m) de un cable, y se buscan las torres de la línea (0: no se usan los cables). |
| `--roads` | casilla | desactivada | Clasifica las carreteras (11): suelo liso, llano y oscuro (intensidad baja) |
| `--water` | casilla | desactivada | Clasifica el agua (9): regiones extensas y niveladas de suelo oscuro de retorno único. |
| `--rule` | texto o número | ninguno | Clasifica por reglas, al final y en orden. Se repite para varias reglas. Partes, separadas por `;`: to=CLASE (obligatoria), from=1,2 (o ~7), returns=F\|L\|A\|1..9, angle=25..90 (grados absolutos; `angle=25` es 25 o más), height=0.5..2 (m sobre el suelo), intensity=0..100 y la forma del entorno dentro de radius=1 (m): slope=60.. (grados), planarity, linearity, sphericity (0 a 1), roughness (m), neighbours (número). Por ejemplo: `angle=25;to=12` o `from=1;height=0.5..2;to=4`. Se puede repetir. |
| `--buildings` | casilla | desactivada | Clasifica edificios (6) y vegetación alta (5), y quita los tejados del suelo. |
| `--recipe` | archivo o carpeta | ninguno | Lee más de estas opciones de un archivo de texto, una o varias por línea (`#` inicia una línea de comentario): una clasificación escrita una vez y ejecutada en cualquier hoja o bloque. Las opciones del archivo van donde está `--recipe`, de modo que no pueden repetirse en la línea de comandos, salvo `--rule`. |
| `--building-offset` | número | `2.5` | Edificios: altura sobre el terreno a partir de la cual los puntos son edificios o árboles. |
| `--building-window` | número | `60` | Edificios: el mayor edificio esperado (m); fija el tamaño de la ventana con la que se separa el suelo de los tejados. |
| `--building-area` | número | `40` | Edificios: la menor planta (en unidades cuadradas del mapa) que se acepta como edificio. |
| `--tile` | número | `250` | Lado (m) de las teselas en que se divide cada hoja para clasificarla en paralelo. |
| `--buffer` | número | `30` | Margen que se lee alrededor de cada tesela (elevado a la mitad de la ventana de edificios al clasificar edificios) |
| `--threads` | número | `0` | Hilos (0: uno por núcleo) |

### hipso dtm

MDT como GeoTIFF, uno por hoja. Por defecto: la clase de suelo (2) que ya trae la nube; --ground prueba otro método

**En la interfaz:** LiDAR → Superficies → MDT. El método se explica en el apartado 7.2.

| Opción | Valor | Por defecto | Qué hace |
|---|---|---|---|
| `INPUTS` | archivo o carpeta | ninguno | Nubes LAS o LAZ de las que se calcula el MDT; cada una da un GeoTIFF. **Obligatoria.** Se puede repetir. |
| `-o`, `--output` | archivo o carpeta | ninguno | GeoTIFF de salida; por defecto: <resultados>/<entrada>_mdt_<celda>m<ajustes>.tif. |
| `--cell` | número | `1` | Tamaño de celda del MDT. |
| `--surface` | una de las opciones | `tin` | Cómo reciben su elevación las celdas. Valores: `mean`: Elevación media de los puntos de suelo por celda (en flujo, una pasada); `tin`: Triangulación de Delaunay (TIN) de los puntos de suelo (por defecto); `idw`: Ponderación por el inverso de la distancia de los puntos de suelo más próximos. |
| `--smooth` | número | `0` | Suaviza el MDT con una ventana de este tamaño en unidades del mapa (0: nada) |
| `--smoothing` | una de las opciones | `mean` | Suavizado del modelo: media, mediana o gaussiano. Valores: `mean`: Media de la ventana; `median`: Mediana de la ventana; `gaussian`: Suavizado gaussiano. |
| `--mosaic` | casilla | desactivada | Une además los MDT de las hojas en un mosaico (por defecto cada hoja conserva el suyo) |
| `--breaklines` | archivo o carpeta | ninguno | Líneas de ruptura que debe seguir el MDT (shapefile de líneas o polígonos): con elevaciones (PolylineZ) fijan la altura; sin ellas, toman el suelo más bajo a su lado. |
| `--neighbors` | número | `0` | Hace el MDT de cada hoja con las hojas que estén a menos de tantos metros de ella, recortado a su propia extensión: sin costura en los bordes de las hojas (0: cada hoja por separado) |
| `--ground` | una de las opciones | ninguno | Cómo se encuentran los puntos de suelo (classify: kraus; dtm: classes, la clase 2 que ya trae la nube) Valores: `kraus`: Filtro iterativo de Kraus y Pfeifer (por defecto); `mcc`: Multiscale Curvature Classification (Evans & Hudak 2007); `csf`: Cloth Simulation Filter (Zhang et al. 2016); `pmf`: Progressive Morphological Filter (Zhang et al. 2003); `smrf`: Simple Morphological Filter (Pingel et al. 2013); `axelsson`: Progressive TIN densification (Axelsson 2000); `classes`: Puntos ya clasificados como suelo (clase 2). |
| `--filter-cell` | número | `5` | Kraus-Pfeifer: tamaño de celda de las superficies intermedias del filtro. |
| `--iterations` | número | `5` | Kraus-Pfeifer: número de iteraciones del filtro. |
| `--g` | número | `-2` | Kraus-Pfeifer: desplazamiento g de la función de peso (los residuos por debajo pesan 1); también --kp-g. |
| `--w` | número | `2.5` | Kraus-Pfeifer: anchura w de la función de peso por encima de g (los residuos por encima de g + w pesan 0); también --kp-w. |
| `--a` | número | `1` | Kraus-Pfeifer: pendiente a de la función de peso; también --kp-a. |
| `--b` | número | `4` | Kraus-Pfeifer: pendiente b de la función de peso; también --kp-b. |
| `--tolerance` | texto o número | ninguno | Kraus-Pfeifer: suelo = a esta distancia de la superficie final (por defecto: residuo <= g + w) |
| `--mcc-spacing` | número | `1.5` | MCC: tamaño de celda del dominio de escala 2 (aproximadamente el espaciado de los puntos) |
| `--mcc-threshold` | número | `0.3` | MCC: umbral de curvatura (m) por encima del cual un punto deja de ser suelo. |
| `--csf-resolution` | número | `0.5` | CSF: distancia entre las partículas de la tela. |
| `--csf-rigidness` | número | `1` | CSF: rigidez, de 1 (terreno escarpado) a 3 (llano) |
| `--csf-threshold` | número | `0.5` | CSF: altura sobre la tela hasta la que los puntos son suelo. |
| `--pmf-window` | número | `20` | PMF: mayor tamaño de ventana (m) de la apertura morfológica progresiva. |
| `--pmf-slope` | número | `1` | PMF: pendiente del terreno (subida sobre recorrido) que tolera el filtro. |
| `--smrf-cell` | número | `1` | SMRF: lado de celda (m) de las superficies que se comparan. |
| `--smrf-window` | número | `18` | SMRF: mayor radio de ventana (mayor que el edificio más grande) |
| `--smrf-slope` | número | `0.15` | SMRF: pendiente más empinada del terreno (subida sobre recorrido) |
| `--smrf-threshold` | número | `0.5` | SMRF: umbral de altura entre un punto y la superficie. |
| `--smrf-scalar` | número | `1.25` | SMRF: se suma al umbral por unidad de pendiente. |
| `--axelsson-seed-cell` | número | `20` | Axelsson: lado de las celdas cuyos puntos más bajos siembran el TIN (mayor que cualquier edificio) |
| `--axelsson-angle` | número | `8` | Axelsson: ángulo máximo (grados) entre un punto y el plano de la triangulación para aceptarlo como suelo. |
| `--axelsson-dist` | número | `1.4` | Axelsson: distancia máxima (m) de un punto al plano de la triangulación para aceptarlo como suelo. |
| `--axelsson-flat` | número | `0.1` | Axelsson: los puntos a esta distancia (m) de un triángulo son suelo sea cual sea el ángulo. |
| `--overlap` | casilla | desactivada | Aparta antes que nada, como clase 12 (solape), los puntos de las pasadas que solapan con otra: donde varias pasadas cubren una celda, la de menor ángulo de barrido medio se la queda. |
| `--overlap-cell` | número | `5` | Solape: lado de celda en metros. |
| `--noise` | una de las opciones | ninguno | Clasifica primero el ruido (clase 7) con este método. Valores: `isolated`: Puntos aislados en celdas de 3 x 3 x 3; `sor`: Eliminación estadística de valores atípicos sobre los k vecinos más próximos; `elm`: Ruido bajo: puntos muy por debajo de las alturas de su celda (mínimo local extendido). |
| `--sor-k` | número | `10` | Ruido (sor): número de vecinos con los que se calcula la distancia media de cada punto. |
| `--sor-m` | número | `3` | Ruido (sor): desviaciones típicas por encima de la distancia media. |
| `--noise-step` | número | `4` | Ruido (isolated): tamaño de celda en x e y. |
| `--noise-step-z` | número | `4` | Ruido (isolated): tamaño de celda en z. |
| `--elm-cell` | número | `10` | Ruido (elm): lado de celda en metros. |
| `--elm-threshold` | número | `3` | Ruido (elm): metros por debajo del siguiente punto de su celda y de los puntos más bajos de las celdas vecinas que debe estar el punto más bajo de una celda para ser ruido bajo. |
| `--isolated` | número | `5` | Ruido: el mayor número de otros puntos alrededor de un punto que aún se llama aislado. |
| `--vegetation` | casilla | desactivada | Clasifica la vegetación por altura sobre el suelo: baja (3), media (4) y alta (5) |
| `--wires` | casilla | desactivada | Clasifica los cables (14): líneas casi horizontales muy por encima del suelo. |
| `--poles` | casilla | desactivada | Clasifica los postes y torres (15): estructuras estrechas y aisladas que se elevan del suelo. |
| `--wire-min-height` | número | `5` | Cables: altura mínima sobre el suelo (m). |
| `--wire-radius` | número | `0` | Cables: radio del entorno con el que se mira la forma de cada punto (m; 0: según el espaciado de los puntos). |
| `--wire-linearity` | número | `0.7` | Cables: linealidad mínima del entorno (0 a 1). |
| `--wire-max-slope` | número | `0.5` | Cables: mayor pendiente de la línea (0 horizontal, 1 vertical). |
| `--wire-max-neighbours` | número | `1000` | Cables: mayor número de puntos alrededor de un punto de cable. |
| `--wire-max-width` | número | `0` | Cables: mayor anchura del entorno a través de la línea (m; 0: según el radio). |
| `--wire-min-length` | número | `10` | Cables: longitud mínima de una cadena de puntos de cable (m). |
| `--wire-min-straightness` | número | `10` | Cables: lo recta que debe ser una cadena (longitud sobre anchura). |
| `--wire-voxel` | número | `0.05` | Cables: lado de los vóxeles a los que se reducen los puntos antes de mirar su forma (m). |
| `--wire-link-gap` | número | `15` | Cables: mayor hueco (m) que se salva al unir trozos de conductor en fila (0: no se unen). |
| `--wire-piece-voxel` | número | `0.5` | Cables: lado (m) de los vóxeles con que se buscan trozos finos y sueltos de conductor (0: no se buscan). |
| `--wire-piece-min-length` | número | `1.2` | Cables: longitud mínima de un trozo fino de conductor (m). |
| `--wire-piece-max-width` | número | `0.12` | Cables: mayor desviación típica (m) de un trozo fino a través de su dirección. |
| `--wire-piece-min-points` | número | `6` | Cables: mínimo de puntos de un trozo fino de conductor. |
| `--wire-piece-isolation` | número | `1` | Cables: un trozo fino de conductor cuelga solo: a menos de esta distancia (m) no hay más que unos pocos vóxeles de otros grupos (0: no se comprueba). |
| `--wire-piece-max-others` | número | `12` | Cables: cuántos vóxeles de otros grupos tolera un trozo fino a esa distancia. |
| `--pole-min-height` | número | `2` | Postes: altura mínima sobre el suelo de los puntos de una estructura (m). |
| `--pole-radius` | número | `1.5` | Postes: los puntos más cercanos que esto pertenecen a la misma estructura (m). |
| `--pole-max-width` | número | `8` | Postes: mayor anchura de la estructura (m). |
| `--pole-min-extent` | número | `6` | Postes: lo mínimo que debe elevarse la estructura (m). |
| `--pole-head-length` | número | `12` | Postes: longitud (m) de la cabeza de una torre vista como una cruceta aislada (0: no se busca). |
| `--pole-thin-extent` | número | `3` | Postes: lo que debe elevarse un poste fino (m; 0: no se buscan). |
| `--pole-wire-reach` | número | `10` | Postes: con cables en la tesela, solo cuentan las estructuras a menos de esta distancia (m) de un cable, y se buscan las torres de la línea (0: no se usan los cables). |
| `--roads` | casilla | desactivada | Clasifica las carreteras (11): suelo liso, llano y oscuro (intensidad baja) |
| `--water` | casilla | desactivada | Clasifica el agua (9): regiones extensas y niveladas de suelo oscuro de retorno único. |
| `--rule` | texto o número | ninguno | Clasifica por reglas, al final y en orden. Se repite para varias reglas. Partes, separadas por `;`: to=CLASE (obligatoria), from=1,2 (o ~7), returns=F\|L\|A\|1..9, angle=25..90 (grados absolutos; `angle=25` es 25 o más), height=0.5..2 (m sobre el suelo), intensity=0..100 y la forma del entorno dentro de radius=1 (m): slope=60.. (grados), planarity, linearity, sphericity (0 a 1), roughness (m), neighbours (número). Por ejemplo: `angle=25;to=12` o `from=1;height=0.5..2;to=4`. Se puede repetir. |
| `--buildings` | casilla | desactivada | Clasifica edificios (6) y vegetación alta (5), y quita los tejados del suelo. |
| `--recipe` | archivo o carpeta | ninguno | Lee más de estas opciones de un archivo de texto, una o varias por línea (`#` inicia una línea de comentario): una clasificación escrita una vez y ejecutada en cualquier hoja o bloque. Las opciones del archivo van donde está `--recipe`, de modo que no pueden repetirse en la línea de comandos, salvo `--rule`. |
| `--building-offset` | número | `2.5` | Edificios: altura sobre el terreno a partir de la cual los puntos son edificios o árboles. |
| `--building-window` | número | `60` | Edificios: el mayor edificio esperado (m); fija el tamaño de la ventana con la que se separa el suelo de los tejados. |
| `--building-area` | número | `40` | Edificios: la menor planta (en unidades cuadradas del mapa) que se acepta como edificio. |
| `--tile` | número | `250` | Lado (m) de las teselas en que se divide cada hoja para clasificarla en paralelo. |
| `--buffer` | número | `30` | Margen que se lee alrededor de cada tesela (elevado a la mitad de la ventana de edificios al clasificar edificios) |
| `--threads` | número | `0` | Hilos (0: uno por núcleo) |

### hipso dsm

Modelo digital de superficie (retornos más altos) como GeoTIFF, uno por hoja

**En la interfaz:** LiDAR → Superficies → MDS / CHM. El método se explica en el apartado 7.2.

| Opción | Valor | Por defecto | Qué hace |
|---|---|---|---|
| `INPUTS` | archivo o carpeta | ninguno | Nubes LAS o LAZ de las que se calcula el MDS; cada una da un GeoTIFF. **Obligatoria.** Se puede repetir. |
| `-o`, `--output` | archivo o carpeta | ninguno | GeoTIFF de salida (solo una entrada); por defecto: <resultados>/<entrada>_mds_<celda>m_<método>.tif. |
| `--cell` | número | `1` | Tamaño de celda del MDS (m) |
| `--method` | una de las opciones | `tin` | Cómo se calcula la superficie: punto más alto por celda o TIN de los primeros retornos. Valores: `max`: Punto más alto de cada celda (en flujo, una pasada); `tin`: TIN por el punto más alto de cada celda (por defecto); `idw`: Ponderación por el inverso de la distancia de los puntos más altos de las celdas. |
| `--no-fill` | casilla | desactivada | Deja vacías las celdas sin puntos (método max) |
| `--tile` | número | `250` | Tamaño de tesela (métodos tin e idw) |
| `--threads` | número | `0` | Hilos (0: uno por núcleo) |
| `--smooth` | número | `0` | Suaviza con una ventana de este tamaño en unidades del mapa (0: nada) |
| `--smoothing` | una de las opciones | `mean` | Suavizado del modelo: media, mediana o gaussiano. Valores: `mean`: Media de la ventana; `median`: Mediana de la ventana; `gaussian`: Suavizado gaussiano. |
| `--neighbors` | número | `0` | Lee las hojas que estén a menos de tantos metros de cada hoja como contexto, de modo que sus bordes salgan como saldrían de todo el bloque, y escribe solo los puntos de la propia hoja (0: desactivado) |

### hipso chm

Modelo de altura de copas como GeoTIFF, uno por hoja

**En la interfaz:** LiDAR → Superficies → MDS / CHM. El método se explica en el apartado 7.2.

| Opción | Valor | Por defecto | Qué hace |
|---|---|---|---|
| `INPUTS` | archivo o carpeta | ninguno | Nubes LAS o LAZ de las que se calcula el CHM; cada una da un GeoTIFF. **Obligatoria.** Se puede repetir. |
| `--dtm` | archivo o carpeta | ninguno | Modelo del suelo (GeoTIFF o rejilla ASCII); por defecto: el MDT más reciente hecho con esta misma nube. |
| `-o`, `--output` | archivo o carpeta | ninguno | GeoTIFF de salida; por defecto: <resultados>/<entrada>_chm_<celda>m.tif. |
| `--cell` | número | `1` | Tamaño de celda del CHM (m) |
| `--no-fill` | casilla | desactivada | Deja vacías las celdas sin puntos (método max) |
| `--method` | una de las opciones | `max` | Punto más alto por celda, o pila de TIN sin picos (Khosravipour et al., 2014) Valores: `max`: Punto más alto de cada celda; `pitfree`: Pila de TIN sin picos (Khosravipour et al., 2014); `tin`: TIN por el punto más alto de cada celda; `idw`: Ponderación por el inverso de la distancia del punto más alto de cada celda. |
| `--max-edge` | número | `1` | Sin picos: arista de triángulo más larga en las capas por encima de 0. |
| `--tile` | número | `250` | Sin picos: tamaño de tesela. |
| `--threads` | número | `0` | Sin picos: hilos (0: uno por núcleo) |
| `--smooth` | número | `0` | Suaviza el CHM con una ventana de este tamaño en unidades del mapa (0: nada) |
| `--smoothing` | una de las opciones | `mean` | Suavizado del modelo: media, mediana o gaussiano. Valores: `mean`: Media de la ventana; `median`: Mediana de la ventana; `gaussian`: Suavizado gaussiano. |
| `--mosaic` | casilla | desactivada | Une además los CHM de las hojas en un mosaico (por defecto cada hoja conserva el suyo) |
| `--neighbors` | número | `0` | Lee las hojas que estén a menos de tantos metros de cada hoja como contexto, de modo que sus bordes salgan como saldrían de todo el bloque, y escribe solo los puntos de la propia hoja (0: desactivado) |

### hipso hillshade

PNG de sombreado de un MDT o un CHM

**En la interfaz:** sin botón propio (la línea de comandos y el constructor de flujos). El método se explica en el apartado 7.2.

| Opción | Valor | Por defecto | Qué hace |
|---|---|---|---|
| `INPUT` | archivo o carpeta | ninguno | El MDT o el CHM (GeoTIFF) **Obligatoria.** |
| `-o`, `--output` | archivo o carpeta | ninguno | PNG de salida; por defecto: <resultados>/<entrada>_sombreado.png. |
| `--azimuth` | número | `315` | Dirección de la luz, en grados en el sentido de las agujas del reloj desde el norte. |
| `--altitude` | número | `45` | Altura de la luz sobre el horizonte, en grados. |

### hipso contour

Curvas de nivel de un MDT o un CHM como shapefile de polilíneas 3D

**En la interfaz:** Vista → Herramientas → Curvas de nivel (en pantalla). El método se explica en el apartado 5.

| Opción | Valor | Por defecto | Qué hace |
|---|---|---|---|
| `INPUT` | archivo o carpeta | ninguno | El MDT o el CHM (GeoTIFF) **Obligatoria.** |
| `-o`, `--output` | archivo o carpeta | ninguno | Shapefile de salida; por defecto: <resultados>/<entrada>_curvas_<equidistancia>m.shp. |
| `--interval` | número | `5` | Equidistancia de las curvas de nivel (m) |

### hipso raster

Rejilla de una estadística de los puntos (densidad, recuento, elevación, intensidad) como GeoTIFF

**En la interfaz:** LiDAR → Preparar → Imagen de intensidad (la rejilla de intensidad media). El método se explica en el apartado 7.1.

| Opción | Valor | Por defecto | Qué hace |
|---|---|---|---|
| `INPUTS` | archivo o carpeta | ninguno | Nubes LAS o LAZ de las que se calcula la rejilla. **Obligatoria.** Se puede repetir. |
| `-o`, `--output` | archivo o carpeta | ninguno | GeoTIFF de salida; por defecto, en la carpeta de resultados con el nombre de la entrada y el estadístico. |
| `--cell` | número | `1` | Tamaño de celda de la rejilla (m) |
| `--stat` | una de las opciones | `density` | Estadístico de cada celda: densidad, recuento, elevación media, máxima, mínima o rango, o intensidad media o máxima. Valores: `density`: Puntos por m²; `count`: Número de puntos; `mean-z`: Elevación media; `max-z`: Elevación máxima; `min-z`: Elevación mínima; `range-z`: Diferencia entre la elevación máxima y la mínima; `mean-intensity`: Intensidad media; `max-intensity`: Intensidad máxima. |
| `--class` | texto o número | ninguno | Clases que se conservan: 2,5 o ~7,18 para todas menos esas. |
| `--returns` | texto o número | ninguno | Retornos que se conservan: A, F, L o números (p. ej. 1,2) |
| `--zmin` | texto o número | ninguno | Elevación mínima (m) de los puntos que se usan. |
| `--zmax` | texto o número | ninguno | Elevación máxima (m) de los puntos que se usan. |
| `--decimate` | número | `1` | Conserva un punto de cada tantos. |

### hipso metrics

Métricas por celda de la rejilla: elevación y, opcionalmente, intensidad y estratos

**En la interfaz:** LiDAR → ALS → Métricas por celda. El método se explica en el apartado 7.3.

| Opción | Valor | Por defecto | Qué hace |
|---|---|---|---|
| `INPUTS` | archivo o carpeta | ninguno | Nubes LAS o LAZ de las que se calculan las métricas. **Obligatoria.** Se puede repetir. |
| `--dtm` | archivo o carpeta | ninguno | Modelo del suelo (GeoTIFF o rejilla ASCII); por defecto: el MDT más reciente hecho con esta misma nube. |
| `-o`, `--output` | archivo o carpeta | ninguno | Shapefile de celdas de salida (o .csv si se da uno); por defecto: <resultados>/<entrada>_metricas_<celda>m.shp. |
| `--cell` | número | `15` | Lado de las celdas en las que se calculan las métricas (m) |
| `--heightbreak` | número | `2` | Altura de corte para las métricas de cobertura. |
| `--neighbors` | número | `0` | Con varias hojas, da a las celdas del borde de cada hoja los puntos de las vecinas a menos de tantos metros (al menos una celda) y escribe cada celda una sola vez. |
| `--min-points` | número | `4` | Menor número de puntos con que una celda tiene métricas; con menos queda vacía. |
| `--memory-mb` | número | `512` | Máximo de memoria, en MiB, para las alturas de los puntos (las nubes mayores requieren más pasadas) |
| `--intensity` | casilla | desactivada | Escribe también métricas de intensidad en <nombre de salida>_intensity (mismo formato que la salida) |
| `--strata` | casilla | desactivada | Escribe también métricas de estratos en <nombre de salida>_strata (mismo formato que la salida) |
| `--strata-limits` | número | `0.15` | Límites superiores de los estratos de altura. Se puede repetir. |
| `--int-strata-limits` | número | `0.15` | Límites superiores de los estratos de intensidad. Se puede repetir. |
| `--structure` | casilla | desactivada | Escribe también métricas de estructura en <nombre de salida>_estructura: fracción de huecos, índice de área foliar, diversidad de la altura del follaje, índice de complejidad vertical y altura de la base de la copa. |
| `--structure-layer` | número | `1` | Estructura: grosor de las capas del perfil vertical. |
| `--structure-top` | número | `30` | Estructura: altura del perfil (los retornos por encima caen en su última capa) |
| `--gap-height` | número | `1` | Estructura: los retornos por debajo de esta altura cuentan como luz que llegó al suelo. |
| `--extinction` | número | `0.5` | Estructura: coeficiente de extinción del índice de área foliar. |
| `--cbh-floor` | número | `2` | Estructura: la base de la copa se busca por encima de esta altura. |
| `--cbh-share` | número | `0.1` | Estructura: una capa es copa cuando guarda esta proporción de los retornos de la capa más llena. |
| `--format` | una de las opciones | ninguno | Formato de salida: shapefile de celdas, tabla de texto o ambos (por defecto: según el nombre de la salida) Valores: `vector`: Solo shapefile; `csv`: Solo tabla de texto (lo mejor para ejecuciones muy grandes); `both`: Both. |

## Vegetación e inventario

### hipso trees

Árboles individuales a partir de un modelo de altura de copas: cimas (máximos locales), copas (Dalponte, Silva o cuenca), polígonos, treeID por punto y comprobación con datos de campo

**En la interfaz:** LiDAR → ALS → Segmentar individuos. El método se explica en el apartado 7.3.

| Opción | Valor | Por defecto | Qué hace |
|---|---|---|---|
| `CHM` | archivo o carpeta | ninguno | Modelo de altura de copas (GeoTIFF o rejilla ASCII) **Obligatoria.** |
| `--around` | archivo o carpeta | ninguno | Modelos de copas de las hojas vecinas de esta (se repite para cada una): los árboles se buscan en esta hoja ampliada con --margin con lo que tienen allí, y solo se conservan los árboles con su cima en esta hoja, de modo que un árbol en el borde de una hoja se encuentra una sola vez y entero. Se puede repetir. |
| `--margin` | número | `30` | Hasta dónde (m) se mira en las hojas vecinas. |
| `-o`, `--output` | archivo o carpeta | ninguno | Cimas de los árboles (.shp, o .csv si se da uno); por defecto: <resultados>/<chm>_arboles_<método>.shp (nunca un archivo existente) |
| `--crowns` | casilla | desactivada | Escribe además el ráster de copas (identificadores de árbol) |
| `--crowns-shp` | casilla | desactivada | Escribe además los polígonos de copa (shapefile) |
| `--tops-shp` | casilla | desactivada | Escribe además las cimas de los árboles (shapefile PointZ) |
| `--cloud` | archivo o carpeta | ninguno | Copia esta nube añadiendo un atributo treeID (LAS/LAZ), en <resultados>/<nube>_treeid. |
| `--tops-file` | archivo o carpeta | ninguno | Cimas dadas a mano: CSV con líneas x,y (en lugar de buscarlas) |
| `--hmin` | número | `2` | Altura mínima (m) de una cima para contarla como árbol. |
| `--ws-a` | número | `3` | Diámetro de ventana = a + b h + c h² (unidades del mapa) |
| `--ws-b` | número | `0.1` | Coeficiente de la altura (b) del diámetro de la ventana de búsqueda de cimas. |
| `--ws-c` | número | `0` | Coeficiente del cuadrado de la altura (c) del diámetro de la ventana de búsqueda de cimas. |
| `--shape` | una de las opciones | `circle` | Forma de la ventana de búsqueda de cimas. Valores: `circle`: Ventana circular; `square`: Ventana cuadrada. |
| `--method` | una de las opciones | `dalponte` | Método con el que se reparten las copas entre las cimas: Dalponte, Silva o cuenca. Valores: `dalponte`: Crecimiento de regiones desde las cimas (Dalponte y Coomes, 2016); `silva`: Polígonos de Voronoi acotados por la altura de cada árbol (Silva et al., 2016); `watershed`: Cuencas (watershed) sobre el CHM invertido. |
| `--th-seed` | número | `0.45` | Dalponte: un píxel se une por encima de esta proporción de la altura de su cima... |
| `--th-cr` | número | `0.55` | ...y de la altura media de la copa. |
| `--max-cr` | número | `10` | Dalponte: alcance de la copa en píxeles. |
| `--max-cr-factor` | número | `0.6` | Silva: radio de la copa como proporción de la altura del árbol. |
| `--exclusion` | número | `0.3` | Silva: descarta los píxeles más bajos que esta proporción de la altura del árbol. |
| `--tolerance` | número | `1` | Cuenca: escalón mínimo de altura entre copas. |
| `--ext` | número | `1` | Cuenca: radio de búsqueda de picos en píxeles. |
| `--smooth` | número | `0` | Suaviza antes el CHM con una ventana de este tamaño (unidades del mapa; 0: nada) |
| `--smoothing` | una de las opciones | `mean` | Suavizado del CHM antes de buscar cimas: media, mediana o gaussiano. Valores: `mean`: Media de la ventana; `median`: Mediana de la ventana; `gaussian`: Suavizado gaussiano. |
| `--field` | archivo o carpeta | ninguno | Inventario de campo (delimitado por tabuladores, ; o ,, se admiten comas decimales) con el que comprobar. |
| `--field-x` | texto | `X_arbol` | Columna del inventario de campo con la coordenada X del árbol. |
| `--field-y` | texto | `Y_arbol` | Columna del inventario de campo con la coordenada Y del árbol. |
| `--field-h` | texto | `Ht` | Columna del inventario de campo con la altura del árbol (m) |
| `--field-plot` | texto | `Muestra` | Columna del inventario de campo con el nombre de la parcela. |
| `--plot-x` | texto | `X_Muestra` | Columna del inventario de campo con la coordenada X del centro de la parcela. |
| `--plot-y` | texto | `Y_Muestra` | Columna del inventario de campo con la coordenada Y del centro de la parcela. |
| `--plot-radius` | número | `10` | Radio de la parcela para la comprobación. |
| `--format` | una de las opciones | ninguno | Formato de salida: shapefile de cimas, tabla de texto o ambos. Valores: `vector`: Solo shapefile; `csv`: Solo tabla de texto (lo mejor para ejecuciones muy grandes); `both`: Both. |
| `--match-dist` | número | `3` | Una detección coincide con un árbol de campo a esta distancia como máximo. |

### hipso obia

Análisis basado en objetos: agrupa las celdas de varias capas (altura de copas, intensidad, retornos, rugosidad…) en objetos, los describe y clasifica, y escribe polígonos

**En la interfaz:** LiDAR → ALS → Análisis por objetos. El método se explica en el apartado 7.3.

| Opción | Valor | Por defecto | Qué hace |
|---|---|---|---|
| `RASTERS` | archivo o carpeta | ninguno | Rásteres sobre las mismas celdas (GeoTIFF), el modelo de altura de copas el primero. **Obligatoria.** Se puede repetir. |
| `--cloud` | archivo o carpeta | ninguno | Nubes de puntos de las que derivar capas (véase --cloud-layers) Se puede repetir. |
| `--cloud-layers` | texto | `intensidad,retornos` | Capas que se hacen con las nubes: intensidad, retornos, suelo, densidad. Se puede repetir. |
| `--cloud-fill` | número | `3` | Las celdas vacías de las capas hechas con las nubes toman el valor de la celda más cercana que lo tenga, hasta tantas celdas de distancia (0: quedan vacías y dejan huecos) |
| `--cloud-smooth` | número | `1` | Promedia esas capas en una ventana de este radio en celdas (0: no) |
| `--roughness` | número | `0` | Añade la rugosidad local del primer ráster en una ventana de tantas celdas (0: no) |
| `--method` | una de las opciones | `merge` | Cómo se hacen los objetos. Valores: `merge`: Fusión multirresolución de manchas; `mask`: Celdas conectadas con un valor dentro de un rango. |
| `--scale` | número | `8` | Fusión: las manchas se fusionan mientras el coste sea menor que la escala al cuadrado (mayor: objetos mayores) |
| `--shape` | número | `0.1` | Fusión: peso de la forma frente al valor, de 0 a 0,9. |
| `--compact` | número | `0.5` | Fusión: peso de la compacidad frente a la suavidad dentro de la forma, de 0 a 1. |
| `--weight` | texto o número | ninguno | Fusión: peso de una capa, como capa=peso (0 la deja fuera); se repite para varias. Se puede repetir. |
| `--init` | número | `0.25` | Fusión: anchura, en desviaciones típicas, de los intervalos de las manchas iniciales. |
| `--mask-layer` | texto o número | ninguno | Máscara: la capa y el rango de valores que forman un objeto. |
| `--min` | número | `-inf` | Máscara: valor mínimo de la capa elegida que forma un objeto. |
| `--max` | número | `inf` | Máscara: valor máximo de la capa elegida que forma un objeto. |
| `--diagonal` | casilla | desactivada | Máscara: las celdas que se tocan por una esquina van juntas. |
| `--min-cells` | número | `10` | Objeto más pequeño, en celdas. |
| `--rules` | archivo o carpeta | ninguno | Clasifica con reglas de un archivo de texto (clase: atributo >= valor y ...) |
| `--samples` | archivo o carpeta | ninguno | Clasifica con un bosque aleatorio entrenado con polígonos de muestra (.shp) |
| `--kmeans` | texto o número | ninguno | Agrupa los objetos en tantos estratos según su parecido (sin reglas ni muestras) |
| `--features` | texto o número | ninguno | Atributos con los que se hacen los estratos (por defecto: las medias y los percentiles 95 de las capas, y la cobertura del dosel) Se puede repetir. |
| `--strata` | casilla | desactivada | Une las clases en estratos: las superficies de una clase que se tocan pasan a ser un polígono. |
| `--strata-min-cells` | número | `500` | Los estratos más pequeños que tantas celdas toman la clase de sus vecinos. |
| `--plots` | número | `0` | Reparte tantas parcelas de muestra entre los estratos (asignación de Neyman) |
| `--cover-height` | número | `2` | Altura a partir de la cual una celda cuenta como cubierta por el dosel (atributo `cobertura`) |
| `--class-field` | texto | `clase` | Campo de las muestras que da nombre a la clase. |
| `--trees` | número | `100` | Número de árboles del bosque aleatorio. |
| `--seed` | número | `1` | Semilla del bosque aleatorio: la misma semilla da el mismo resultado. |
| `-o`, `--output` | archivo o carpeta | ninguno | Ruta base de los resultados (.shp o .csv); por defecto: <resultados>/<ráster>_objetos.shp. |
| `--format` | una de las opciones | ninguno | Formato de salida: shapefile de objetos, tabla de texto o ambos. Valores: `vector`: Solo shapefile; `csv`: Solo tabla de texto (lo mejor para ejecuciones muy grandes); `both`: Both. |
| `--simplify` | número | `0` | Simplifica los polígonos en tantas unidades del mapa (0: contornos exactos) |

### hipso stem-volume

Mide los troncos de una nube terrestre o móvil a lo largo del tronco y estima el volumen de cada individuo con la fórmula de Smalian (un shapefile o CSV de árboles, un CSV de trozas y, opcionalmente, nubes por árbol)

**En la interfaz:** LiDAR → TLS → Medición de troncos. El método se explica en el apartado 7.3.

| Opción | Valor | Por defecto | Qué hace |
|---|---|---|---|
| `INPUTS` | archivo o carpeta | ninguno | Nubes LAS o LAZ de escáner terrestre; cada una es una unidad de muestreo. **Obligatoria.** Se puede repetir. |
| `-o`, `--output` | archivo o carpeta | ninguno | Tabla de árboles (.shp o .csv); por defecto: <resultados>/<entrada>_volumen.shp. |
| `--csv` | casilla | desactivada | Escribe una tabla de texto además del shapefile. |
| `--normalized` | casilla | desactivada | Las z de la nube ya son alturas sobre el suelo. |
| `--base-height` | número | `0.1` | Altura sobre el suelo (m) donde empieza la primera troza. |
| `--top-diameter` | número | `0.07` | Diámetro (m) en el que termina el fuste comercial. |
| `--log-length` | número | `0.5` | Distancia (m) entre los diámetros medidos: la longitud de una troza. |
| `--voxel` | número | `0.03` | Vóxel (m) al que se reduce la nube. |
| `--stripe-low` | número | `0.7` | Franja (m sobre el suelo) donde se buscan los troncos. |
| `--stripe-high` | número | `3.5` | Límite superior (m sobre el suelo) de la franja donde se buscan los troncos. |
| `--max-diameter` | número | `1` | Mayor diámetro de tronco esperado (m) |
| `--tree-clouds` | casilla | desactivada | Escribe además una nube por árbol. |
| `--stem-classes` | casilla | desactivada | Escribe además la nube con los troncos en la clase 64 y las ramas en la clase 65. |
| `--assign-dist` | número | `8` | Distancia (m) hasta la que un punto va al árbol más cercano en las nubes por árbol. |
| `--stool-dist` | número | `0.6` | Distancia (m) entre las bases de los troncos de una cepa (brotes de monte bajo) |
| `--fork-height` | número | `1.3` | Altura (m) del diámetro: los troncos separados a esa altura son brotes, cada uno un árbol; los troncos que son uno a esa altura y se bifurcan por encima son un solo árbol bifurcado. |
| `--max-gap` | número | `2` | Mayor tramo (m) de un tronco que puede quedar sin medir antes de darlo por perdido. |
| `--lost-top-share` | número | `0` | Altura comercial como proporción (0-1) de la altura del árbol, para estimarla en los troncos perdidos cuando aún eran gruesos y muy por debajo de la cima (por ejemplo 0,65 para el eucalipto); 0: sin estimación. |

### hipso model

Modelo de transición (por área): regresión de una variable de campo (volumen, área basimétrica…) sobre las métricas LiDAR de las parcelas, con validación cruzada dejando una fuera que repite la elección de métricas, diagnósticos y, dadas las métricas de las celdas, el mapa de la variable

**En la interfaz:** LiDAR → ALS → Modelos lineales. El método se explica en el apartado 7.3.

| Opción | Valor | Por defecto | Qué hace |
|---|---|---|---|
| `--plots` | archivo o carpeta | ninguno | Métricas de las parcelas: la capa (.shp) o la tabla de texto de `clip --metrics`. **Obligatoria.** |
| `--field` | archivo o carpeta | ninguno | Tabla de campo, una fila por parcela (.csv, .txt o .xlsx) **Obligatoria.** |
| `--response` | texto o número | ninguno | Columna de la tabla de campo que se estima. **Obligatoria.** |
| `--plot-key` | texto | `Muestra` | Columna que nombra la parcela en la tabla de métricas. |
| `--field-key` | texto o número | ninguno | Columna que nombra la parcela en la tabla de campo; por defecto: el mismo nombre. |
| `--predictors` | texto o número | ninguno | Métricas que se usan, separadas por comas; por defecto: elegidas automáticamente. Se puede repetir. |
| `--transform` | una de las opciones | `none` | Transformación antes del ajuste: ninguna, logaritmo de la respuesta, o logaritmo de la respuesta y de las métricas. Valores: `none`: Sin transformación; `log-response`: Logaritmo de la respuesta; `log-both`: Logaritmo de la respuesta y de las métricas (una ley de potencias). |
| `--criterion` | una de las opciones | `bic` | Criterio de la elección automática. Valores: `bic`: Criterio de información bayesiano (BIC): castiga más los modelos con muchas métricas; `aicc`: Criterio de Akaike corregido para muestras pequeñas (AICc). |
| `--max-vars` | número | `0` | Máximo de métricas que conserva la elección automática (0: una por cada cinco parcelas, como mucho cinco) |
| `--max-vif` | número | `5` | Mayor factor de inflación de la varianza que acepta la elección automática. |
| `--candidates` | texto o número | ninguno | Métricas que puede usar la elección automática, separadas por comas; por defecto: todas las de la tabla. Se puede repetir. |
| `--max-corr` | número | `0.8` | Los candidatos correlacionados más que esto dicen lo mismo: solo se conserva el más relacionado con la variable (1: se conservan todos) |
| `--analyze` | casilla | desactivada | Solo el análisis previo al ajuste: normalidad de la variable y cribado de los candidatos. |
| `--with-intensity` | casilla | desactivada | Ofrece también las métricas de intensidad a la elección automática (por defecto se dejan fuera: la intensidad de la mayoría de los vuelos no está calibrada entre pasadas) |
| `--cells` | archivo o carpeta | ninguno | Métricas de las celdas (la tabla de texto de `metrics`; una por hoja): escribe el mapa. Se puede repetir. |
| `-o`, `--output` | archivo o carpeta | ninguno | El mapa (GeoTIFF); por defecto: junto a la tabla de parcelas. |
| `--interval` | casilla | desactivada | Escribe también la semianchura del intervalo de predicción del 95 %. |
| `--extrapolate` | casilla | desactivada | Estima también las celdas fuera del rango de las parcelas. |
| `--epsg` | texto o número | ninguno | Código EPSG del mapa; por defecto: el de la capa de la tabla de celdas. |

### hipso change

Cambio forestal entre dos fechas a partir de dos modelos de altura de copas sobre las mismas celdas: diferencia de altura, crecimiento, pérdida y cambio de cobertura, manchas de dosel perdido, resumen por rodal y crecimiento de cada árbol

**En la interfaz:** LiDAR → Superficies → Cambio de la cubierta vegetal. El método se explica en el apartado 7.3.

| Opción | Valor | Por defecto | Qué hace |
|---|---|---|---|
| `BEFORE` | archivo o carpeta | ninguno | Modelo de altura de copas de la primera fecha. **Obligatoria.** |
| `AFTER` | archivo o carpeta | ninguno | Modelo de altura de copas de la segunda fecha (mismas celdas; véase `align`) **Obligatoria.** |
| `-o`, `--output` | archivo o carpeta | ninguno | Ruta base de los resultados; por defecto: <resultados>/<después>_cambio.shp (nunca un archivo existente) |
| `--min-change` | número | `1` | Un cambio de altura menor que este es ruido. |
| `--cover-height` | número | `2` | Altura a partir de la cual una celda cuenta como dosel. |
| `--min-patch` | número | `10` | Las manchas de dosel perdido menores que tantas celdas no se comunican. |
| `--years` | texto o número | ninguno | Años entre las fechas, para las tasas anuales. |
| `--stands` | archivo o carpeta | ninguno | Rodales que resumir (.shp o .geojson) |
| `--stand-field` | texto o número | ninguno | Campo de los rodales que los nombra. |
| `--trees` | casilla | desactivada | Empareja además los árboles de las dos fechas y mide su crecimiento. |
| `--hmin` | número | `2` | Árboles: altura mínima (m) de una cima para contarla como árbol. |
| `--ws-a` | número | `3` | Árboles: diámetro de ventana = a + b h (unidades del mapa) |
| `--ws-b` | número | `0.1` | Coeficiente de la altura (b) del diámetro de la ventana de búsqueda de cimas. |
| `--smooth` | número | `0` | Árboles: suaviza antes los dos modelos con una ventana de este tamaño (0: nada) |
| `--match-dist` | número | `3` | Árboles: es el mismo árbol cuando están más cerca que esto. |
| `--format` | una de las opciones | ninguno | Formato de salida: shapefile, tabla de texto o ambos. Valores: `vector`: Solo shapefile; `csv`: Solo tabla de texto (lo mejor para ejecuciones muy grandes); `both`: Both. |

### hipso corridor

Corredores de líneas eléctricas: conductores reconstruidos a partir de los puntos de cable (clase 14), postes (clase 15) y la vegetación que se acerca demasiado o que podría alcanzar la línea si cayera, con un informe por vano. Se clasifican antes los cables y los postes

**En la interfaz:** LiDAR → Infraestructuras → Corredores eléctricos. El método se explica en el apartado 7.3.

| Opción | Valor | Por defecto | Qué hace |
|---|---|---|---|
| `INPUTS` | archivo o carpeta | ninguno | Nubes LAS o LAZ de la línea eléctrica, con los cables (clase 14) y los postes (clase 15) ya clasificados. **Obligatoria.** Se puede repetir. |
| `--dtm` | archivo o carpeta | ninguno | Modelo del suelo (GeoTIFF o rejilla ASCII); por defecto: el MDT más reciente hecho con la primera nube. |
| `-o`, `--output` | archivo o carpeta | ninguno | Ruta base de los resultados; por defecto: <resultados>/<entrada>_corredor.shp (nunca un archivo existente) |
| `--wire-class` | número | `14` | Clase de los puntos de cable. |
| `--pole-class` | número | `15` | Clase de los puntos de poste. |
| `--min-distance` | número | `3` | A menos de esta distancia de un conductor (en el espacio) hay invasión. |
| `--fall-margin` | número | `1` | Un árbol cuya altura más este margen alcanza la posición del conductor podría golpearlo. |
| `--half-width` | número | `30` | La vegetación a más de esta distancia de todo conductor no se mira. |
| `--link` | número | `2.5` | Los puntos de cable más cercanos que esto son un solo conductor. |
| `--gap-max` | número | `30` | Mayor hueco de puntos de cable que se rellena (los mayores cortan el conductor) |
| `--min-points` | número | `30` | Menor número de puntos de un conductor. |
| `--min-length` | número | `25` | Conductor más corto (los más cortos son ruido de la clasificación, p. ej. en copas de árboles) |
| `--min-pole-height` | número | `6` | Altura mínima (m) de un poste o una torre para aceptarlo. |
| `--cell` | número | `3` | Lado de las celdas en las que se agrupa la vegetación. |
| `--min-height` | número | `2` | La vegetación más baja que esto sobre el suelo se ignora. |
| `--vegetation` | texto | `3,4,5` | Clases de vegetación. |
| `--pole-reach` | número | `8` | Un poste a más de esta distancia de un conductor no es uno de sus apoyos. |
| `--format` | una de las opciones | ninguno | Formato de salida: shapefile, tabla de texto o ambos. Valores: `vector`: Solo shapefile; `csv`: Solo tabla de texto (lo mejor para ejecuciones muy grandes); `both`: Both. |

## Ráster

### hipso stack

Imagen multibanda a partir de rásteres: cada banda de cada entrada pasa a ser una banda de la salida

**En la interfaz:** Raster → Imagen → Crear multibanda. El método se explica en el apartado 8.

| Opción | Valor | Por defecto | Qué hace |
|---|---|---|---|
| `INPUTS` | archivo o carpeta | ninguno | Rásteres que apilar; sus bandas se colocan en el orden de la lista. **Obligatoria.** Se puede repetir. |
| `-o`, `--output` | archivo o carpeta | ninguno | Ráster multibanda de salida. **Obligatoria.** |
| `--cell` | texto o número | ninguno | Tamaño de píxel de la salida; por defecto: el de la entrada más fina. |
| `--src-nodata` | texto o número | ninguno | Valor de las entradas que significa sin datos. |
| `--nodata` | número | `0` | Valor que se escribe donde no hay datos. |
| `--compress` | una de las opciones | `auto` | Compresión de las teselas: auto (JPEG para imágenes de 8 bits y tres bandas, las ortofotos habituales; deflate en los demás casos), deflate (sin pérdidas) o jpeg (imágenes de 8 bits de 1 o 3 bandas) Valores: `auto`: JPEG en imágenes de 8 bits y tres bandas, deflate en los demás casos; `deflate`: Sin pérdidas; `jpeg`: Con pérdidas, para imágenes de 8 bits de una o tres bandas. |
| `--quality` | número | `90` | Calidad JPEG, de 1 a 100. |

### hipso mosaic

Mosaico de rásteres con las mismas bandas; --overlap dice qué hacer donde se solapan

**En la interfaz:** Raster → Imagen → Mosaico. El método se explica en el apartado 8.

| Opción | Valor | Por defecto | Qué hace |
|---|---|---|---|
| `INPUTS` | archivo o carpeta | ninguno | Rásteres que unir. **Obligatoria.** Se puede repetir. |
| `-o`, `--output` | archivo o carpeta | ninguno | Ráster de salida. **Obligatoria.** |
| `--overlap` | una de las opciones | `first` | Qué valor queda donde los rásteres se solapan: el primero, el último, la media, el mínimo, el máximo…. Valores: `first`: El valor del primer ráster de la lista; `last`: El valor del último ráster de la lista; `mean`: La media de los rásteres; `min`: El mínimo; `max`: El máximo; `median`: La mediana; `sum`: La suma; `count`: Cuántos rásteres cubren la celda. |
| `--cell` | texto o número | ninguno | Tamaño de píxel de la salida; por defecto: el de la entrada más fina. |
| `--src-nodata` | texto o número | ninguno | Valor de las entradas que significa sin datos. |
| `--nodata` | número | `0` | Valor que se escribe donde no hay datos. |
| `--intersection` | casilla | desactivada | La salida cubre solo donde coinciden todas las entradas. |
| `--average` | casilla | desactivada | Promedia los píxeles de origen al reducir (por defecto: el más cercano) |
| `--compress` | una de las opciones | `auto` | Compresión de las teselas: auto (JPEG para imágenes de 8 bits y tres bandas, las ortofotos habituales; deflate en los demás casos), deflate (sin pérdidas) o jpeg (imágenes de 8 bits de 1 o 3 bandas) Valores: `auto`: JPEG en imágenes de 8 bits y tres bandas, deflate en los demás casos; `deflate`: Sin pérdidas; `jpeg`: Con pérdidas, para imágenes de 8 bits de una o tres bandas. |
| `--quality` | número | `90` | Calidad JPEG, de 1 a 100. |

### hipso align

Pone rásteres sobre las celdas de un ráster de referencia (mismo origen, celda y tamaño), de modo que coincidan celda a celda. Escribe <resultados>/<nombre>_alineado.tif para cada uno

**En la interfaz:** Raster → Imagen → Igualar rásteres. El método se explica en el apartado 8.

| Opción | Valor | Por defecto | Qué hace |
|---|---|---|---|
| `REFERENCE` | archivo o carpeta | ninguno | El ráster cuyas celdas toman todos los demás. **Obligatoria.** |
| `INPUTS` | archivo o carpeta | ninguno | Rásteres que alinear. **Obligatoria.** Se puede repetir. |
| `--resample` | una de las opciones | `auto` | Cómo se trasladan los valores (auto: la media si el ráster es más fino; si no, bilineal) Valores: `auto`: Según los datos: vecino más cercano (o moda al reducir) en rásteres de enteros; media si las celdas de salida son mayores que las de entrada; bilineal en los demás casos; `nearest`: Para clases e identificadores; `bilinear`: Interpolación bilineal (superficies continuas); `average`: Media de las celdas que cubre cada celda nueva; `cubic`: Cubic convolution (smooth surfaces); `mode`: Valor más frecuente (clases, al reducir). |
| `--nodata` | número | `-9999` | Lo que contienen las celdas fuera de un ráster. |
| `--compress` | una de las opciones | `auto` | Compresión de las teselas: auto (JPEG para imágenes de 8 bits y tres bandas, las ortofotos habituales; deflate en los demás casos), deflate (sin pérdidas) o jpeg (imágenes de 8 bits de 1 o 3 bandas) Valores: `auto`: JPEG en imágenes de 8 bits y tres bandas, deflate en los demás casos; `deflate`: Sin pérdidas; `jpeg`: Con pérdidas, para imágenes de 8 bits de una o tres bandas. |
| `--quality` | número | `90` | Calidad JPEG, de 1 a 100. |

### hipso reproject

Lleva una nube (LAS/LAZ) o un ráster a otro sistema de coordenadas, p. ej. de ED50 a ETRS89 con la rejilla del IGN español. Las alturas se conservan salvo que --vertical-out pida otra referencia vertical (p. ej. de elipsoidal a ortométrica con el modelo de geoide)

**En la interfaz:** Raster → Geoprocesos → Reproyectar. El método se explica en el apartado 8.

| Opción | Valor | Por defecto | Qué hace |
|---|---|---|---|
| `INPUT` | archivo o carpeta | ninguno | La nube (LAS/LAZ) o el ráster que reproyectar. **Obligatoria.** |
| `-o`, `--output` | archivo o carpeta | ninguno | Archivo de salida (LAS/LAZ, o GeoTIFF para rásteres) **Obligatoria.** |
| `--to` | texto o número | ninguno | Código EPSG del sistema que se escribe, p. ej. 25830 (no hace falta con --reference) |
| `--from` | texto o número | ninguno | Código EPSG en el que está la entrada, cuando no lo declara. |
| `--reference` | archivo o carpeta | ninguno | Rásteres: toma el sistema, la extensión, el origen y la celda de este ráster, de modo que el resultado coincida celda a celda con él. |
| `--resample` | una de las opciones | `auto` | Rásteres: cómo se trasladan los valores (auto: las clases conservan sus valores; si no, la media cuando las celdas crecen y bilineal en caso contrario) Valores: `auto`: Según los datos: vecino más cercano (o moda al reducir) en rásteres de enteros; media si las celdas de salida son mayores que las de entrada; bilineal en los demás casos; `nearest`: Para clases e identificadores; `bilinear`: Interpolación bilineal (superficies continuas); `average`: Media de las celdas que cubre cada celda nueva; `cubic`: Cubic convolution (smooth surfaces); `mode`: Valor más frecuente (clases, al reducir). |
| `--cell` | texto o número | ninguno | Rásteres: celda del resultado (por defecto: la de la entrada o la equivalente en el sistema nuevo) |
| `--nodata` | número | `-9999` | Lo que contienen las celdas vacías de un ráster cuando se lleva sobre una referencia. |
| `--compress` | una de las opciones | `auto` | Compresión de las teselas: auto (JPEG para imágenes de 8 bits y tres bandas, las ortofotos habituales; deflate en los demás casos), deflate (sin pérdidas) o jpeg (imágenes de 8 bits de 1 o 3 bandas) Valores: `auto`: JPEG en imágenes de 8 bits y tres bandas, deflate en los demás casos; `deflate`: Sin pérdidas; `jpeg`: Con pérdidas, para imágenes de 8 bits de una o tres bandas. |
| `--quality` | número | `90` | Calidad JPEG, de 1 a 100. |
| `--vertical-in` | texto o número | ninguno | Referencia vertical de las alturas de la entrada: `ellipsoidal` o un código EPSG (p. ej. 5782), cuando el archivo no la declara. Hace falta para cambiar las alturas con --vertical-out. |
| `--vertical-out` | texto o número | ninguno | Referencia vertical que dar a las alturas (`ellipsoidal` o un código EPSG). Sin ella, las alturas no se tocan. |
| `--allow-ballpark` | casilla | desactivada | Acepta aproximaciones: sin ajuste de datum, alturas conservadas donde ningún geoide cubre el punto (por defecto: las rechaza; también HIPSO_ALLOW_BALLPARK=YES) |
| `--allow-fallback` | casilla | desactivada | Acepta una operación inferior cuando la mejor no puede ejecutarse, p. ej. por falta de una rejilla (por defecto: la rechaza; también HIPSO_ONLY_BEST=NO) |

### hipso cog

Reescribe un ráster como Cloud Optimized GeoTIFF (teselas y vistas reducidas, con todos los directorios al principio del archivo); las imágenes de varias bandas se escriben en flujo, sea cual sea su tamaño

**En la interfaz:** sin botón propio (la línea de comandos y el constructor de flujos). El método se explica en el apartado 10.

| Opción | Valor | Por defecto | Qué hace |
|---|---|---|---|
| `INPUT` | archivo o carpeta | ninguno | GeoTIFF, JPEG 2000 o rejilla ASCII, de una banda o de varias (ortofotos, imágenes de satélite) **Obligatoria.** |
| `-o`, `--output` | archivo o carpeta | ninguno | Salida; por defecto: <resultados>/<entrada>_cog.tif (nunca un archivo existente) |
| `--nearest` | casilla | desactivada | Reduce las vistas reducidas tomando una celda en lugar de promediar (clases, identificadores) |
| `--compress` | una de las opciones | `auto` | Compresión de las teselas: auto (JPEG para imágenes de 8 bits y tres bandas, las ortofotos habituales; deflate en los demás casos), deflate (sin pérdidas) o jpeg (imágenes de 8 bits de 1 o 3 bandas) Valores: `auto`: JPEG en imágenes de 8 bits y tres bandas, deflate en los demás casos; `deflate`: Sin pérdidas; `jpeg`: Con pérdidas, para imágenes de 8 bits de una o tres bandas. |
| `--quality` | número | `90` | Calidad JPEG, de 1 a 100. |

### hipso reclass

Reclasifica un ráster: cada valor o rango de valores pasa a ser otro. Las reglas son texto, una por línea o separadas por `;`: `10 = 1`, `0..5 = 2`, `< 0 = nodata`, `>= 100 = 4`, `nodata = 0`; decide la primera regla que coincide con una celda

**En la interfaz:** Raster → Geoprocesos → Reclasificar. El método se explica en el apartado 8.

| Opción | Valor | Por defecto | Qué hace |
|---|---|---|---|
| `INPUT` | archivo o carpeta | ninguno | Ráster que reclasificar (GeoTIFF y otros) **Obligatoria.** |
| `--rules` | texto o número | ninguno | Las reglas, como texto (p. ej. "0..10 = 1; 10..50 = 2; else = nodata") |
| `--rules-file` | archivo o carpeta | ninguno | Un archivo de texto con las reglas, una por línea. |
| `--others` | texto | `keep` | En qué se convierten las celdas a las que no se aplica ninguna regla: keep (su valor), nodata o un valor. |
| `--band` | número | `1` | Banda que reclasificar. |
| `--src-nodata` | texto o número | ninguno | Valor de la entrada que significa «sin datos», además de lo que declare su archivo. |
| `--nodata` | texto o número | ninguno | Lo que contienen las celdas vacías del resultado (por defecto: 255, 65535 o -9999 según el tipo elegido) |
| `-o`, `--output` | archivo o carpeta | ninguno | GeoTIFF; por defecto: <resultados>/<entrada>_reclass.tif. |
| `--compress` | una de las opciones | `auto` | Compresión de las teselas: auto (JPEG para imágenes de 8 bits y tres bandas, las ortofotos habituales; deflate en los demás casos), deflate (sin pérdidas) o jpeg (imágenes de 8 bits de 1 o 3 bandas) Valores: `auto`: JPEG en imágenes de 8 bits y tres bandas, deflate en los demás casos; `deflate`: Sin pérdidas; `jpeg`: Con pérdidas, para imágenes de 8 bits de una o tres bandas. |
| `--quality` | número | `90` | Calidad JPEG, de 1 a 100. |

### hipso clip-raster

Recorta un ráster con una capa de polígonos u otro ráster: lo que queda fuera de la máscara se vacía y, salvo con --no-crop, el resultado se corta a la caja de la máscara

**En la interfaz:** Raster → Geoprocesos → Cortar por máscara. El método se explica en el apartado 8.

| Opción | Valor | Por defecto | Qué hace |
|---|---|---|---|
| `INPUT` | archivo o carpeta | ninguno | Ráster que recortar. **Obligatoria.** |
| `--mask` | archivo o carpeta | ninguno | Máscara: una capa de polígonos (shapefile, GeoPackage, GeoJSON, KML) o un ráster (se conservan las celdas con datos distintos de cero) **Obligatoria.** |
| `--no-crop` | casilla | desactivada | No corta a la caja de la máscara: conserva el tamaño de la entrada. |
| `--invert` | casilla | desactivada | Conserva lo que queda fuera de la máscara en lugar de lo que queda dentro. |
| `--nodata` | texto o número | ninguno | Lo que contienen las celdas descartadas (por defecto: lo del archivo; si no, 0 o -9999 según el tipo) |
| `-o`, `--output` | archivo o carpeta | ninguno | GeoTIFF; por defecto: <resultados>/<entrada>_recorte.tif. |
| `--compress` | una de las opciones | `auto` | Compresión de las teselas: auto (JPEG para imágenes de 8 bits y tres bandas, las ortofotos habituales; deflate en los demás casos), deflate (sin pérdidas) o jpeg (imágenes de 8 bits de 1 o 3 bandas) Valores: `auto`: JPEG en imágenes de 8 bits y tres bandas, deflate en los demás casos; `deflate`: Sin pérdidas; `jpeg`: Con pérdidas, para imágenes de 8 bits de una o tres bandas. |
| `--quality` | número | `90` | Calidad JPEG, de 1 a 100. |

### hipso sieve

Quita las manchas pequeñas de un ráster de clases: las celdas conectadas de un valor con menos celdas que --threshold toman el valor de su mancha vecina mayor

**En la interfaz:** Raster → Geoprocesos → Tamizar. El método se explica en el apartado 8.

| Opción | Valor | Por defecto | Qué hace |
|---|---|---|---|
| `INPUT` | archivo o carpeta | ninguno | Ráster de clases (GeoTIFF) **Obligatoria.** |
| `--threshold` | número | `2` | Las manchas con menos celdas que esto se fusionan. |
| `--eight` | casilla | desactivada | Las celdas se tocan por aristas y esquinas (8) en lugar de solo por aristas (4) |
| `-o`, `--output` | archivo o carpeta | ninguno | GeoTIFF; por defecto: <resultados>/<entrada>_sieve.tif. |

### hipso proximity

Distancia de cada celda a la celda más cercana de algunas clases (euclídea, exacta)

**En la interfaz:** Raster → Geoprocesos → Proximidad. El método se explica en el apartado 8.

| Opción | Valor | Por defecto | Qué hace |
|---|---|---|---|
| `INPUT` | archivo o carpeta | ninguno | Ráster (GeoTIFF) del que se calcula la distancia. **Obligatoria.** |
| `--values` | texto o número | ninguno | Valores que cuentan como objetivos (por defecto: toda celda distinta de cero) Se puede repetir. |
| `--max-distance` | texto o número | ninguno | Las distancias mayores que esto se dejan vacías. |
| `--cells` | casilla | desactivada | Distancias en celdas en lugar de unidades del mapa. |
| `-o`, `--output` | archivo o carpeta | ninguno | GeoTIFF; por defecto: <resultados>/<entrada>_proximity.tif. |

### hipso fill-nodata

Rellena las celdas vacías a partir de las celdas con valor que las rodean (pesos por el inverso de la distancia en ocho direcciones)

**En la interfaz:** Raster → Geoprocesos → Rellenar huecos. El método se explica en el apartado 8.

| Opción | Valor | Por defecto | Qué hace |
|---|---|---|---|
| `INPUT` | archivo o carpeta | ninguno | Ráster (GeoTIFF) cuyos huecos se rellenan. **Obligatoria.** |
| `--max-distance` | número | `100` | Hasta dónde se buscan celdas con valor, en celdas. |
| `--smoothing` | número | `0` | Pasadas de una media 3x3 sobre las celdas rellenadas. |
| `-o`, `--output` | archivo o carpeta | ninguno | GeoTIFF; por defecto: <resultados>/<entrada>_relleno.tif. |

### hipso calc

Calculadora ráster: una expresión sobre rásteres con nombres, celda a celda. Funciones: abs sqrt ln log10 exp pow min max floor ceil round sin cos tan atan2 clamp if isnan; comparaciones, `and`, `or`, `not`, `%`, `^`. `A.2` es la banda 2 de A. Ejemplo: `hipso calc "if(A>0, A, 0)" A=chm.tif -o out.tif`

**En la interfaz:** Raster → Geoprocesos → Calculadora. El método se explica en el apartado 8.

| Opción | Valor | Por defecto | Qué hace |
|---|---|---|---|
| `EXPR` | texto o número | ninguno | La expresión, entre comillas. **Obligatoria.** |
| `INPUTS` | archivo o carpeta | ninguno | Los rásteres, como `NOMBRE=archivo.tif` (`A=chm.tif B=otro.tif`); con uno solo, también vale un archivo sin nombre y sus bandas son B1, B2…. **Obligatoria.** Se puede repetir. |
| `-o`, `--output` | archivo o carpeta | ninguno | El resultado; por defecto: <resultados>/<primer ráster>_calc.tif. |
| `--dtype` | una de las opciones | `f32` | Tipo de muestra del resultado. Valores: `f32`: Decimal de 32 bits; `u8`: Entero sin signo de 8 bits; `u16`: Entero sin signo de 16 bits. |
| `--nearest` | casilla | desactivada | Las vistas reducidas conservan el primer píxel válido en lugar de la media (clases) |
| `--align` | una de las opciones | ninguno | Los rásteres cuyas celdas difieren de las del primero se ponen sobre ellas con este remuestreo; sin él se rechazan. Valores: `auto`: Según los datos: vecino más cercano (o moda al reducir) en rásteres de enteros; media si las celdas de salida son mayores que las de entrada; bilineal en los demás casos; `nearest`: Para clases e identificadores; `bilinear`: Interpolación bilineal (superficies continuas); `average`: Media de las celdas que cubre cada celda nueva; `cubic`: Cubic convolution (smooth surfaces); `mode`: Valor más frecuente (clases, al reducir). |

### hipso vectorize

Vectoriza un ráster: las celdas que comparten una clase, caen en un rango o están entre dos cortes pasan a ser polígonos con el valor, el área y el número de celdas como atributos

**En la interfaz:** Raster → Geoprocesos → Vectorizar. El método se explica en el apartado 8.

| Opción | Valor | Por defecto | Qué hace |
|---|---|---|---|
| `INPUT` | archivo o carpeta | ninguno | El ráster (un GeoTIFF) **Obligatoria.** |
| `-o`, `--output` | archivo o carpeta | ninguno | Los polígonos: `.shp` o `.gpkg`; por defecto: <resultados>/<entrada>_poligonos.shp. |
| `--range` | texto o número | ninguno | Agrupa por rango: `bajo,alto`, ambos incluidos; el resto se deja fuera. Se puede repetir. |
| `--breaks` | texto o número | ninguno | Agrupa por los intervalos entre estos cortes, de menor a mayor: `0,5,10`. Se puede repetir. |
| `--classes` | casilla | desactivada | Agrupa por clase (valores enteros iguales); es lo habitual. |
| `--min-cells` | número | `1` | Descarta las partes con menos celdas que esto. |
| `--simplify` | número | `0` | Simplifica los límites (unidades del mapa); 0 conserva los escalones de las celdas. |
| `--diagonal` | casilla | desactivada | Las celdas que se tocan por una esquina son una sola parte. |

### hipso rasterize

Rasteriza una capa: cada celda recibe el valor de un campo del último elemento que la cubre (o 1)

**En la interfaz:** Raster → Geoprocesos → Rasterizar. El método se explica en el apartado 8.

| Opción | Valor | Por defecto | Qué hace |
|---|---|---|---|
| `LAYER` | archivo o carpeta | ninguno | La capa. **Obligatoria.** |
| `--reference` | archivo o carpeta | ninguno | Ráster de referencia: toma sus celdas. |
| `--cell` | texto o número | ninguno | Tamaño de celda cuando no hay referencia. |
| `--field` | texto o número | ninguno | Campo con el valor que se pinta (sin él, 1) |
| `-o`, `--output` | archivo o carpeta | ninguno | GeoTIFF de salida; por defecto: <resultados>/<capa>_raster.tif. |

### hipso focal

Filtro focal de un ráster: media, mediana, mínimo, máximo, suma, rango, desviación o mayoría sobre una ventana

**En la interfaz:** Raster → Geoprocesos → Filtro focal. El método se explica en el apartado 8.

| Opción | Valor | Por defecto | Qué hace |
|---|---|---|---|
| `INPUT` | archivo o carpeta | ninguno | El ráster (GeoTIFF) **Obligatoria.** |
| `--stat` | texto | `mean` | mean, median, min, max, sum, range, std, majority o gaussian. |
| `--radius` | número | `1` | Radio de la ventana en celdas. |
| `--circle` | casilla | desactivada | Ventana circular en lugar de cuadrada. |
| `--sigma` | texto o número | ninguno | Desviación típica del filtro gaussiano, en celdas (por defecto, la mitad del radio) |
| `--min-valid` | número | `1` | Menos celdas con valor que esto dejan la celda vacía. |
| `-o`, `--output` | archivo o carpeta | ninguno | GeoTIFF de salida; por defecto: <resultados>/<entrada>_focal.tif. |

### hipso rstats

Estadísticas de un ráster (recuento, mínimo, máximo, media, desviación, mediana, percentiles) y su histograma

**En la interfaz:** Raster → Geoprocesos → Estadísticas ráster. El método se explica en el apartado 8.

| Opción | Valor | Por defecto | Qué hace |
|---|---|---|---|
| `INPUT` | archivo o carpeta | ninguno | El ráster (GeoTIFF) **Obligatoria.** |
| `--bins` | número | `20` | Intervalos del histograma. |
| `-o`, `--output` | archivo o carpeta | ninguno | CSV del histograma; por defecto: <resultados>/<entrada>_histograma.csv. |

### hipso tabulate

Área de cada clase de un ráster dentro de cada zona de otro (tabular área), como CSV

**En la interfaz:** Raster → Geoprocesos → Tabular áreas. El método se explica en el apartado 8.

| Opción | Valor | Por defecto | Qué hace |
|---|---|---|---|
| `ZONES` | archivo o carpeta | ninguno | El ráster de zonas (clases enteras) **Obligatoria.** |
| `--values` | archivo o carpeta | ninguno | El ráster de clases que se mide dentro de cada zona. **Obligatoria.** |
| `-o`, `--output` | archivo o carpeta | ninguno | CSV de salida; por defecto: <resultados>/<zonas>_tabulado.csv. |

### hipso zonal

Estadísticas de un ráster (un CHM, un índice, un MDT) dentro de cada polígono. El resultado es una capa de polígonos (shapefile) con los atributos propios de los polígonos más un campo nuevo por estadístico: count, min, max, mean, sum, stddev, median. Una celda cuenta cuando su centro está en el polígono

**En la interfaz:** Vectorial → Geoprocesos → Estadística zonal. El método se explica en el apartado 9.

| Opción | Valor | Por defecto | Qué hace |
|---|---|---|---|
| `RASTER` | archivo o carpeta | ninguno | El ráster (GeoTIFF) **Obligatoria.** |
| `--regions` | archivo o carpeta | ninguno | Los polígonos (.shp, .geojson, .gpkg), en el sistema del ráster. **Obligatoria.** |
| `--prefix` | texto | ninguno | Prefijo de los campos nuevos (un campo de shapefile tiene como mucho 10 caracteres: `chm_` deja 6 para el estadístico, así que caben `stddev` y `median`) |
| `--split-parts` | casilla | desactivada | Divide los elementos de varias partes en un polígono por parte. |
| `-o`, `--output` | archivo o carpeta | ninguno | Shapefile; por defecto: <resultados>/<ráster>_zonal.shp. |

## Terreno e hidrología

### hipso terrain

Análisis de terreno de un MDT (GeoTIFF): pendiente, orientación, sombreado, curvaturas, rugosidad, TRI, TPI, depresiones, flujo acumulado, índice de humedad, cauces, subcuencas, cuenca visual

**En la interfaz:** Raster → Análisis del terreno (menús Relieve, Forma del terreno, Hidrología y Visibilidad y rutas). El método se explica en el apartado 7.2.

| Opción | Valor | Por defecto | Qué hace |
|---|---|---|---|
| `INPUT` | archivo o carpeta | ninguno | El MDT (un GeoTIFF en un sistema proyectado, celda en metros) **Obligatoria.** |
| `--product` | una de las opciones | ninguno | Producto que se calcula: pendiente, orientación, sombreado, curvaturas, rugosidad, TRI, TPI, depresiones, flujo, TWI, cauces, subcuencas o cuenca visual. **Obligatoria.** Valores: `slope`: Pendiente, en grados o en porcentaje (--percent); `aspect`: Orientación de la ladera, en grados desde el norte; `hillshade`: Sombreado de 0 a 255 con la luz de --azimuth y --altitude; `plan-curvature`: Curvatura en planta (1/m); `profile-curvature`: Curvatura de perfil (1/m); `curvature`: Curvatura total (1/m); `roughness`: Rugosidad: la celda más alta menos la más baja de la ventana de 3×3; `tri`: Índice de rugosidad del terreno: media de las diferencias con las ocho vecinas; `tpi`: Posición topográfica: altura menos la media del entorno de radio --radius; `fill-depth`: Profundidad de depresiones: lo que hay que rellenar para que el agua salga; `flow-accumulation`: Acumulación de flujo (D8): superficie que vierte a cada celda, en m²; `twi`: Índice topográfico de humedad ln(a / tan β); `streams`: Red de drenaje: las celdas que drenan al menos --threshold m²; `subbasins`: Subcuencas: la cuenca de cada tramo de la red de drenaje; `viewshed`: Cuenca visual de un observador (--observer). |
| `-o`, `--output` | archivo o carpeta | ninguno | GeoTIFF; por defecto: <resultados>/<entrada>_<producto>.tif. |
| `--percent` | casilla | desactivada | Pendiente en porcentaje en lugar de grados. |
| `--edges` | texto | `nodata` | Ventanas sobre el borde del ráster o una celda vacía (productos 3x3): `nodata` (por defecto, sin valor), `repeat` (repite las celdas del borde; sin valor alrededor de los huecos) o `center` (los vecinos que faltan toman la altura del centro) |
| `--z-factor` | número | `1` | Multiplica las alturas cuando no están en las unidades de la celda. |
| `--azimuth` | número | `315` | Sombreado: dirección de la luz en grados en el sentido de las agujas del reloj desde el norte. |
| `--altitude` | número | `45` | Sombreado: altura de la luz sobre el horizonte en grados. |
| `--radius` | número | `10` | TPI: radio del entorno (unidades del mapa) |
| `--threshold` | número | `10000` | Cauces y subcuencas: superficie drenada (m²) que forma un cauce. |
| `--observer` | texto o número | ninguno | Cuenca visual: la X y la Y del observador. Se puede repetir. |
| `--eye` | número | `1.7` | Cuenca visual: altura del ojo sobre el suelo (m) |
| `--target` | número | `0` | Cuenca visual: altura sobre el suelo que debe verse (m) |
| `--max-distance` | número | `1000` | Cuenca visual: hasta dónde mira el observador (unidades del mapa) |

### hipso ponds

Charcas: donde queda agua en un MDT. El modelo se rellena hasta que drena, y cada mancha conectada de agua más profunda que el mínimo es una charca, medida por área, profundidad, volumen, nivel de desborde y superficie drenada

**En la interfaz:** Raster → Análisis del terreno → Hidrología → Zonas encharcadas. El método se explica en el apartado 7.2.

| Opción | Valor | Por defecto | Qué hace |
|---|---|---|---|
| `INPUT` | archivo o carpeta | ninguno | El MDT (un GeoTIFF en un sistema proyectado, celda en metros) **Obligatoria.** |
| `-o`, `--output` | archivo o carpeta | ninguno | Ruta base de los resultados; por defecto: <resultados>/<entrada>_charcos.shp. |
| `--method` | texto | `depression` | `depression` (agua retenida hasta el nivel de desborde de cada depresión) o `local` (puntos bajos frente al modelo suavizado; la pendiente y la superficie drenada se comprueban celda a celda) |
| `--smooth` | número | `0` | Suaviza antes el MDT (gaussiano, desviación típica en unidades del mapa; 0: no) |
| `--nodata` | texto o número | ninguno | El valor que significa sin datos cuando el MDT no declara ninguno. |
| `--min-depth` | número | `0.05` | El agua menos profunda no es una charca (unidades del MDT) |
| `--min-area` | número | `0` | Descarta las charcas menores que esto (m²) |
| `--max-area` | texto o número | ninguno | Descarta las charcas mayores que esto (m²): los embalses no son charcos. |
| `--max-depth` | texto o número | ninguno | Descarta las charcas más profundas que esto (unidades del MDT): los hoyos y las zanjas no son charcos. |
| `--min-volume` | número | `0` | Descarta las charcas que retienen menos que esto (m³) |
| `--max-slope` | texto o número | ninguno | Descarta las charcas en suelo con una pendiente media mayor que esta (grados) |
| `--min-catchment` | texto o número | ninguno | Descarta las charcas que drenan menos superficie que esta (m²) |
| `--depth-raster` | casilla | desactivada | Escribe también la profundidad del agua como ráster. |
| `--gpkg` | casilla | desactivada | Escribe además un GeoPackage. |

### hipso flood

Simulación de inundación sobre un MDT: el área, el volumen y la profundidad del agua a uno o varios niveles, desde unas semillas. Con conectividad, un hoyo detrás de una cresta queda seco

**En la interfaz:** Raster → Análisis del terreno → Hidrología → Inundación. El método se explica en el apartado 7.2.

| Opción | Valor | Por defecto | Qué hace |
|---|---|---|---|
| `INPUT` | archivo o carpeta | ninguno | El MDT (un GeoTIFF en un sistema proyectado) **Obligatoria.** |
| `--seed` | texto o número | ninguno | Una semilla del agua, `x,y`; se repite para varias. Sin semillas, `--border`. Se puede repetir. |
| `--border` | casilla | desactivada | El agua viene del borde del modelo (y de junto a las celdas vacías) |
| `--bathtub` | casilla | desactivada | El modelo de bañera: todo lo que queda bajo el nivel se inunda, esté conectado o no. |
| `--level` | texto o número | ninguno | Un nivel del agua, se repite para varios. Se puede repetir. |
| `--levels` | texto o número | ninguno | Una serie de niveles `desde:hasta:paso`. |
| `--relative` | una de las opciones | ninguno | Los niveles son alturas sobre `min` (la celda más baja) o sobre la `seed`; por defecto: elevaciones. Valores: `min`: Sobre la celda más baja del modelo; `seed`: Sobre la elevación de la primera semilla. |
| `--connectivity` | número | `8` | Conectividad del agua: con 4 vecinos pasa solo por los lados; con 8, también por las diagonales. |
| `--extent` | casilla | desactivada | Una capa de polígonos con la superficie inundada de cada nivel. |
| `--no-depth` | casilla | desactivada | Sin ráster de profundidad por nivel. |
| `--no-csv` | casilla | desactivada | No escribe la tabla de resultados en CSV. |
| `-o`, `--output` | archivo o carpeta | ninguno | Ruta base de los resultados; por defecto: <resultados>/<entrada>_inundacion. |

### hipso ridges

Líneas de cresta o de drenaje trazadas a partir de un MDT, como líneas 3D con orden de Strahler, longitud y pendiente media

**En la interfaz:** Raster → Análisis del terreno → Forma del terreno → Crestas y drenaje. El método se explica en el apartado 7.2.

| Opción | Valor | Por defecto | Qué hace |
|---|---|---|---|
| `INPUT` | archivo o carpeta | ninguno | El MDT (un GeoTIFF en un sistema proyectado) **Obligatoria.** |
| `--drainage` | casilla | desactivada | Traza las líneas de drenaje en lugar de las crestas. |
| `--min-area` | número | `1000` | Superficie drenada que forma una línea (m2) |
| `--min-length` | número | `0` | Descarta las líneas más cortas que esto (m) |
| `--smooth` | número | `0` | Pasadas de suavizado (Chaikin); 0 conserva las celdas. |
| `-o`, `--output` | archivo o carpeta | ninguno | Las líneas; por defecto: <resultados>/<entrada>_crestas.shp (o _drenaje) |

### hipso leastcost

Ruta de menor coste sobre un MDT, desde un origen hasta un destino con paradas opcionales

**En la interfaz:** Raster → Análisis del terreno → Visibilidad y rutas → Ruta de menor coste. El método se explica en el apartado 7.2.

| Opción | Valor | Por defecto | Qué hace |
|---|---|---|---|
| `INPUT` | archivo o carpeta | ninguno | El MDT (un GeoTIFF en un sistema proyectado) **Obligatoria.** |
| `--from` | texto o número | ninguno | El punto de origen `x,y`, en las coordenadas del MDT. **Obligatoria.** |
| `--to` | texto o número | ninguno | El destino `x,y`; se repite con --nearest para ir al más barato de varios. **Obligatoria.** Se puede repetir. |
| `--via` | texto o número | ninguno | Una parada en el camino `x,y`, en orden. Se puede repetir. |
| `--model` | texto | `slope` | `slope`, `tobler`, `uniform` o `raster:<tif>` (coste por metro en las celdas del MDT) |
| `--power` | número | `2` | Modelo de pendiente: exponente y pendiente (%) donde el coste por metro se duplica. |
| `--reference-slope` | número | `10` | Modelo de pendiente: pendiente (%) a la que el coste por metro se duplica. |
| `--max-slope` | texto o número | ninguno | Las celdas más empinadas que esto (%) no se pueden cruzar. |
| `--barriers` | archivo o carpeta | ninguno | Polígonos que no se pueden cruzar. |
| `--prefer` | archivo o carpeta | ninguno | Polígonos donde el coste se multiplica por --prefer-factor. |
| `--prefer-factor` | número | `0.5` | Factor por el que se multiplica el coste dentro de los polígonos preferidos. |
| `--nearest` | casilla | desactivada | Va al más barato de los destinos. |
| `--cost-distance` | casilla | desactivada | Escribe también un ráster con el coste de llegar a cada celda desde el origen. |
| `-o`, `--output` | archivo o carpeta | ninguno | La ruta; por defecto: <resultados>/<entrada>_ruta.shp. |

### hipso piles

Volúmenes de acopios: el material sobre una superficie base dentro del contorno de cada pila. La base se hace para cada pila (un nivel, la altura más baja o mediana de su contorno, un plano por el contorno, una superficie interpolada a partir de él o un modelo del terreno del sitio antes)

**En la interfaz:** Raster → Análisis del terreno → Volúmenes → Cubicación de pilas. El método se explica en el apartado 7.2.

| Opción | Valor | Por defecto | Qué hace |
|---|---|---|---|
| `SURFACE` | archivo o carpeta | ninguno | El modelo de superficie (un GeoTIFF, MDS) **Obligatoria.** |
| `--piles` | archivo o carpeta | ninguno | Los contornos de las pilas (una capa de polígonos) **Obligatoria.** |
| `--base` | texto | `plane` | `plane`, `lowest`, `median`, `interpolated`, `level:<z>` o `dtm:<archivo>`. |
| `--min-height` | número | `0` | Las alturas más cercanas que esto a la base no cuentan (m) |
| `--density` | texto o número | ninguno | Densidad del material (t/m3): la capa y la tabla reciben una masa. |
| `-o`, `--output` | archivo o carpeta | ninguno | Ruta base de los resultados; por defecto: <resultados>/<superficie>_pilas. |
| `--no-csv` | casilla | desactivada | No escribe la tabla de resultados en CSV. |
| `--base-raster` | casilla | desactivada | Escribe también un ráster con la superficie base. |
| `--height-raster` | casilla | desactivada | Escribe también un ráster con la altura sobre la base. |

### hipso cutfill

Desmonte y terraplén entre un terreno existente y un diseño: dz = diseño - existente; positivo es terraplén (tierra que añadir), negativo es desmonte (tierra que quitar)

**En la interfaz:** Raster → Análisis del terreno → Volúmenes → Desmonte y terraplén. El método se explica en el apartado 7.2.

| Opción | Valor | Por defecto | Qué hace |
|---|---|---|---|
| `EXISTING` | archivo o carpeta | ninguno | El terreno existente (un GeoTIFF) **Obligatoria.** |
| `--design` | texto o número | ninguno | El diseño: un archivo ráster, `level:<z>` o `plane:<z0>,<pendiente %>,<orientación grados>,<x>,<y>` (elevación z0 en x,y, bajando hacia la orientación) **Obligatoria.** |
| `--regions` | archivo o carpeta | ninguno | Polígonos que medir uno a uno (el ráster entero se mide siempre también) |
| `--tolerance` | número | `0.05` | Las diferencias de hasta este tamaño cuentan como sin cambio (m) |
| `-o`, `--output` | archivo o carpeta | ninguno | Ruta base de los resultados; por defecto: <resultados>/<existente>_cutfill. |
| `--no-csv` | casilla | desactivada | No escribe la tabla de resultados en CSV. |
| `--classified` | casilla | desactivada | Escribe también un ráster de números enteros: 1 desmonte, 2 terraplén, 0 sin cambio. |

## Vectorial

### hipso is-valid

Comprueba que los polígonos de una capa son válidos (anillos cerrados, sin autointersecciones, huecos dentro, sin solapes entre partes) y lista los que no lo son

**En la interfaz:** Vectorial → Geometría → Comprobar validez. El método se explica en el apartado 9.

| Opción | Valor | Por defecto | Qué hace |
|---|---|---|---|
| `INPUT` | archivo o carpeta | ninguno | Capa de polígonos (.shp, .geojson, .gpkg) **Obligatoria.** |

### hipso make-valid

Repara polígonos inválidos (regla par-impar: un lazo pasa a ser dos partes y los bordes duplicados se cancelan). Los válidos no se tocan; los atributos se conservan

**En la interfaz:** Vectorial → Geometría → Reparar geometrías. El método se explica en el apartado 9.

| Opción | Valor | Por defecto | Qué hace |
|---|---|---|---|
| `INPUT` | archivo o carpeta | ninguno | Capa de polígonos (.shp, .geojson, .gpkg) que reparar. **Obligatoria.** |
| `-o`, `--output` | archivo o carpeta | ninguno | Shapefile; por defecto: <resultados>/<entrada>_valido.shp. |

### hipso buffer

Amplía (o, si es negativa, reduce) cada polígono una distancia en unidades del mapa, con esquinas redondeadas

**En la interfaz:** Vectorial → Geoprocesos → Búfer. El método se explica en el apartado 9.

| Opción | Valor | Por defecto | Qué hace |
|---|---|---|---|
| `INPUT` | archivo o carpeta | ninguno | Capa que ampliar o reducir (.shp, .geojson, .gpkg) **Obligatoria.** |
| `--distance` | texto o número | ninguno | Distancia en las unidades de la capa (metros en un sistema proyectado; grados si es geográfico) **Obligatoria.** |
| `--make-valid` | casilla | desactivada | Repara antes los polígonos inválidos. |
| `-o`, `--output` | archivo o carpeta | ninguno | Shapefile; por defecto: <resultados>/<entrada>_buffer.shp. |

### hipso dissolve

Une polígonos: los que comparten el valor de un campo pasan a ser un solo elemento (con el número de unidos en `n`); sin campo, toda la capa pasa a ser uno

**En la interfaz:** Vectorial → Geoprocesos → Disolver. El método se explica en el apartado 9.

| Opción | Valor | Por defecto | Qué hace |
|---|---|---|---|
| `INPUT` | archivo o carpeta | ninguno | Capa de polígonos que disolver (.shp, .geojson, .gpkg) **Obligatoria.** |
| `--field` | texto o número | ninguno | Campo por el que disolver. |
| `--make-valid` | casilla | desactivada | Repara antes los polígonos inválidos. |
| `-o`, `--output` | archivo o carpeta | ninguno | Shapefile; por defecto: <resultados>/<entrada>_disuelto.shp. |

### hipso union

Unión de dos capas de polígonos como superposición: las piezas que están en las dos (atributos de ambas, los de la segunda con el prefijo `b_`), solo en la primera y solo en la segunda

**En la interfaz:** Vectorial → Geoprocesos → Unión. El método se explica en el apartado 9.

| Opción | Valor | Por defecto | Qué hace |
|---|---|---|---|
| `FIRST` | archivo o carpeta | ninguno | La primera capa de polígonos. **Obligatoria.** |
| `SECOND` | archivo o carpeta | ninguno | La segunda capa de polígonos. **Obligatoria.** |
| `--make-valid` | casilla | desactivada | Repara antes los polígonos inválidos. |
| `-o`, `--output` | archivo o carpeta | ninguno | Shapefile; por defecto: <resultados>/<primera>_union.shp. |

### hipso intersect

Intersección de dos capas de polígonos: solo las piezas que están en las dos (atributos de ambas, los de la segunda con el prefijo `b_`)

**En la interfaz:** Vectorial → Geoprocesos → Intersección. El método se explica en el apartado 9.

| Opción | Valor | Por defecto | Qué hace |
|---|---|---|---|
| `FIRST` | archivo o carpeta | ninguno | La primera capa de polígonos. **Obligatoria.** |
| `SECOND` | archivo o carpeta | ninguno | La segunda capa de polígonos. **Obligatoria.** |
| `--make-valid` | casilla | desactivada | Repara antes los polígonos inválidos. |
| `-o`, `--output` | archivo o carpeta | ninguno | Shapefile; por defecto: <resultados>/<primera>_interseccion.shp. |

### hipso voronoi

Polígonos de Voronoi (y, con --delaunay, las aristas de Delaunay) de una capa de puntos, con los atributos de cada punto

**En la interfaz:** Vectorial → Geoprocesos → Voronoi. El método se explica en el apartado 9.

| Opción | Valor | Por defecto | Qué hace |
|---|---|---|---|
| `INPUT` | archivo o carpeta | ninguno | Una capa de puntos. **Obligatoria.** |
| `--clip` | texto o número | ninguno | Las celdas se cortan por `bbox` (la caja de los puntos, algo mayor) o por una capa de polígonos; sin ello, se dejan fuera las celdas de los puntos del borde. |
| `--delaunay` | casilla | desactivada | Escribe también las aristas de la triangulación de Delaunay. |
| `-o`, `--output` | archivo o carpeta | ninguno | Los polígonos: `.shp`, `.gpkg` o `.geojson`; por defecto: <resultados>/<entrada>_voronoi.shp. |

### hipso voronoi-density

Densidad de puntos por celdas de Voronoi, para comprobar una nube: n / suma de las áreas de las celdas de los puntos de cada celda ráster, con la proporción de celdas que alcanzan un umbral

**En la interfaz:** LiDAR → Calidad → Densidad de Voronoi. El método se explica en el apartado 7.1.

| Opción | Valor | Por defecto | Qué hace |
|---|---|---|---|
| `INPUTS` | archivo o carpeta | ninguno | Nubes LAS o LAZ de las que se mide la densidad. **Obligatoria.** Se puede repetir. |
| `--cell` | número | `5` | Celda del ráster de densidad (m) |
| `--threshold` | número | `8` | Puntos por m2 que debe alcanzar la densidad. |
| `--returns` | texto | `F` | Retornos que se usan: A, F, L o números; por defecto F (primeros retornos: pulsos) |
| `--class` | texto o número | ninguno | Clases que se usan: 2,5 o ~7,18. |
| `--max-cell-area` | número | `100` | Una celda de Voronoi mayor que esto (m2) es un hueco en los datos. |
| `--no-pass-fail` | casilla | desactivada | Sin ráster de 1 donde se alcanza el umbral y 0 donde no. |
| `--no-csv` | casilla | desactivada | No escribe la tabla de resultados en CSV. |
| `--tile` | número | `250` | Lado de las teselas y su margen (m) |
| `--buffer` | número | `20` | Margen (m) que cada tesela toma de sus vecinas. |
| `-o`, `--output` | archivo o carpeta | ninguno | Ruta base de los resultados; por defecto: <resultados>/<primera nube>_vdensidad. |

### hipso extract

Extrae los valores de uno o varios rásteres a una capa: un campo nuevo por banda con el valor bajo cada elemento (en el punto o en un punto interior de la línea o del polígono)

**En la interfaz:** Vectorial → Capas y tablas → Extraer valores. El método se explica en el apartado 9.

| Opción | Valor | Por defecto | Qué hace |
|---|---|---|---|
| `LAYER` | archivo o carpeta | ninguno | La capa (puntos, líneas o polígonos), en el mismo sistema que los rásteres. **Obligatoria.** |
| `--raster` | archivo o carpeta | ninguno | Un ráster (se repite para varios) **Obligatoria.** Se puede repetir. |
| `--bands` | texto o número | ninguno | Bandas que extraer, contando desde 1 (por defecto, todas) Se puede repetir. |
| `--bilinear` | casilla | desactivada | Interpola entre los cuatro centros de celda más cercanos en lugar de tomar la celda que contiene el punto. |
| `--prefix` | texto | ninguno | Prefijo de los campos nuevos. |
| `-o`, `--output` | archivo o carpeta | ninguno | Capa de salida; por defecto: <resultados>/<capa>_valores.<ext>. |

### hipso join-table

Une a una capa las columnas de una tabla (csv, texto o xlsx) por un campo clave

**En la interfaz:** Vectorial → Capas y tablas → Unir tabla. El método se explica en el apartado 9.

| Opción | Valor | Por defecto | Qué hace |
|---|---|---|---|
| `LAYER` | archivo o carpeta | ninguno | La capa a la que se añaden las columnas (.shp, .gpkg o .geojson) **Obligatoria.** |
| `TABLE` | archivo o carpeta | ninguno | La tabla (.csv, texto o .xlsx) de la que salen las columnas. **Obligatoria.** |
| `--layer-key` | texto o número | ninguno | Campo clave de la capa. **Obligatoria.** |
| `--table-key` | texto o número | ninguno | Columna clave de la tabla (por defecto, el mismo nombre) |
| `--prefix` | texto | ninguno | Prefijo de los campos nuevos. |
| `-o`, `--output` | archivo o carpeta | ninguno | Capa de salida; por defecto: <resultados>/<capa>_union.<ext>. |

### hipso merge

Combina capas del mismo tipo de geometría en una (los campos son la unión de los de todas)

**En la interfaz:** Vectorial → Capas y tablas → Combinar capas. El método se explica en el apartado 9.

| Opción | Valor | Por defecto | Qué hace |
|---|---|---|---|
| `INPUTS` | archivo o carpeta | ninguno | Las capas que combinar. **Obligatoria.** Se puede repetir. |
| `--origin` | texto o número | ninguno | Añade un campo con el nombre del archivo de origen de cada elemento. |
| `-o`, `--output` | archivo o carpeta | ninguno | Capa de salida; por defecto: <resultados>/<primera>_combinada.<ext>. |

### hipso split

Divide una capa en una por cada valor distinto de un campo (un archivo por rodal, por especie...)

**En la interfaz:** Vectorial → Capas y tablas → Dividir por campo. El método se explica en el apartado 9.

| Opción | Valor | Por defecto | Qué hace |
|---|---|---|---|
| `INPUT` | archivo o carpeta | ninguno | La capa que se divide (.shp, .gpkg o .geojson) **Obligatoria.** |
| `--field` | texto o número | ninguno | El campo por el que dividir. **Obligatoria.** |
| `--dir` | archivo o carpeta | ninguno | Carpeta de salida; por defecto: <resultados>/<capa>_partes. |

### hipso clip-vector

Recorta una capa de puntos, líneas o polígonos con los polígonos de otra (con --erase, borra lo que cubren)

**En la interfaz:** Vectorial → Capas y tablas → Recortar capa. El método se explica en el apartado 9.

| Opción | Valor | Por defecto | Qué hace |
|---|---|---|---|
| `INPUT` | archivo o carpeta | ninguno | La capa que recortar. **Obligatoria.** |
| `--mask` | archivo o carpeta | ninguno | Los polígonos con los que recortar. **Obligatoria.** |
| `--erase` | casilla | desactivada | Borra lo que cubren los polígonos en lugar de conservarlo. |
| `-o`, `--output` | archivo o carpeta | ninguno | Capa de salida; por defecto: <resultados>/<capa>_recortada.<ext> (o _borrada) |

### hipso near

Distancia de cada elemento al elemento más cercano de otra capa, con su número de orden y, si se pide, uno de sus campos

**En la interfaz:** Vectorial → Capas y tablas → Cercanía. El método se explica en el apartado 9.

| Opción | Valor | Por defecto | Qué hace |
|---|---|---|---|
| `INPUT` | archivo o carpeta | ninguno | La capa. **Obligatoria.** |
| `--to` | archivo o carpeta | ninguno | La capa de la que se busca el elemento más cercano. **Obligatoria.** |
| `--carry` | texto o número | ninguno | Campo de la otra capa que se copia del elemento más cercano. |
| `--max` | texto o número | ninguno | Distancia máxima: los más lejanos quedan sin valor. |
| `-o`, `--output` | archivo o carpeta | ninguno | Capa de salida; por defecto: <resultados>/<capa>_cercania.<ext>. |

### hipso summary

Estadísticas de resumen de campos numéricos de una capa o tabla, por grupos de un campo (recuento, suma, media, mínimo, máximo, desviación)

**En la interfaz:** Vectorial → Capas y tablas → Resumen estadístico. El método se explica en el apartado 9.

| Opción | Valor | Por defecto | Qué hace |
|---|---|---|---|
| `INPUT` | archivo o carpeta | ninguno | La capa o la tabla (csv, texto, xlsx) **Obligatoria.** |
| `--by` | texto o número | ninguno | Campo por el que agrupar (sin él, un solo grupo) |
| `--fields` | texto o número | ninguno | Campos numéricos que resumir, separados por comas (por defecto, todos los numéricos) Se puede repetir. |
| `-o`, `--output` | archivo o carpeta | ninguno | Tabla CSV de salida; por defecto: <resultados>/<entrada>_resumen.csv. |

### hipso topology

Comprueba la topología de una capa: polígonos que se solapan, huecos entre polígonos y contornos que se cruzan consigo mismos; en líneas, cabos sueltos

**En la interfaz:** panel lateral → Edición (capas vectoriales) → Comprobar topología. El método se explica en el apartado 9.

| Opción | Valor | Por defecto | Qué hace |
|---|---|---|---|
| `INPUT` | archivo o carpeta | ninguno | La capa de polígonos o de líneas. **Obligatoria.** |
| `--tolerance` | número | `0.000001` | Tolerancia: superficie mínima de un solape o un hueco, o distancia a la que dos extremos de línea se tienen por unidos. |
| `-o`, `--output` | archivo o carpeta | ninguno | Tabla CSV del informe; por defecto: <resultados>/<capa>_topologia.csv. |

### hipso geojson

Convierte una capa vectorial entre shapefile y GeoJSON (según la extensión de la entrada). El GeoJSON recibe el sistema de coordenadas del shapefile como miembro `crs`

**En la interfaz:** sin botón propio (la línea de comandos y el constructor de flujos). El método se explica en el apartado 10.

| Opción | Valor | Por defecto | Qué hace |
|---|---|---|---|
| `INPUT` | archivo o carpeta | ninguno | Un .shp (da un .geojson) o un .geojson (da un .shp) **Obligatoria.** |
| `-o`, `--output` | archivo o carpeta | ninguno | Salida; por defecto: <resultados>/<entrada>.geojson o .shp (nunca un archivo existente) |

## Salida y comprobación de nubes

### hipso export

Extrae puntos de LAS/LAZ a LAS/LAZ, a columnas ASCII o a un shapefile de puntos 3D

**En la interfaz:** LiDAR → Preparar → Exportar puntos. El método se explica en el apartado 7.1.

| Opción | Valor | Por defecto | Qué hace |
|---|---|---|---|
| `INPUTS` | archivo o carpeta | ninguno | Nubes LAS o LAZ que exportar. **Obligatoria.** Se puede repetir. |
| `-o`, `--output` | archivo o carpeta | ninguno | Archivo (o, con varias entradas, una carpeta). .laz y .las escriben una nube, .shp un shapefile PointZ y cualquier otro, texto. **Obligatoria.** |
| `--normalize` | casilla | desactivada | Exporta alturas sobre el suelo: cada nube menos su MDT. |
| `--dtm` | archivo o carpeta | ninguno | MDT con el que normalizar (una entrada); por defecto: el MDT más reciente hecho con cada nube. |
| `--items` | texto | `x` | Columnas en orden: x,y,z,intensity,return,returns,class,source,red,green,blue. Se puede repetir. |
| `--delimiter` | texto | `,` | Separador de columnas del texto. |
| `--header` | casilla | desactivada | Escribe una primera línea con los nombres de las columnas. |
| `--decimals` | número | `2` | Decimales de las coordenadas. |
| `--class` | texto o número | ninguno | Clases que se exportan: 2,5 o ~7,18. |
| `--returns` | texto o número | ninguno | Retornos que se exportan: A, F, L o números. |
| `--zmin` | texto o número | ninguno | Elevación mínima (m) de los puntos que se exportan. |
| `--zmax` | texto o número | ninguno | Elevación máxima (m) de los puntos que se exportan. |
| `--decimate` | número | `1` | Conserva un punto de cada tantos. |

### hipso copc

Escribe las nubes como COPC (Cloud Optimized Point Cloud): un LAZ cuyos bloques son los nodos de un octree, para visores y servidores que leen solo la parte y el detalle que necesitan

**En la interfaz:** se crea sola al abrir una nube grande (véase Ajustes). El método se explica en el apartado 4.10.

| Opción | Valor | Por defecto | Qué hace |
|---|---|---|---|
| `INPUTS` | archivo o carpeta | ninguno | Nubes LAS o LAZ que convertir a COPC. **Obligatoria.** Se puede repetir. |
| `-o`, `--output` | archivo o carpeta | ninguno | Salida (solo una entrada); por defecto: <resultados>/<entrada>.copc.laz. |

### hipso render

Dibuja una nube (planta, 3D o perfil) en un PNG, como hace el visor

**En la interfaz:** Archivo → Guardar vista PNG (en la interfaz, lo que se ve). El método se explica en el apartado 4.2.

| Opción | Valor | Por defecto | Qué hace |
|---|---|---|---|
| `INPUTS` | archivo o carpeta | ninguno | Nubes LAS o LAZ que dibujar. **Obligatoria.** Se puede repetir. |
| `-o`, `--output` | archivo o carpeta | ninguno | PNG que escribir; por defecto: junto a la primera entrada. |
| `--view` | una de las opciones | `plan` | Vista: planta, 3D o perfil. Valores: `plan`: Planta, vista desde arriba; `3d`: Vista 3D, orientada con --yaw y --pitch; `profile`: Perfil de una caja (--profile). |
| `--color` | una de las opciones | `elevation` | Atributo con el que se colorean los puntos. Valores: `elevation`: Elevación, con la rampa azul-rojo; `earth`: Elevación, con la rampa de tierra; `class`: Clase de cada punto; `intensity`: Intensidad; `return`: Número de retorno; `source`: ID de fuente (la pasada); `file`: Archivo de origen; `rgb`: Color RGB del punto; `tree`: Crown number (treeID extra attribute). |
| `--size` | texto | `1600x1000` | Tamaño de la imagen: ANCHOxALTO. |
| `--max-points` | número | `8000000` | Puntos que se guardan en memoria (se carga uno de cada N) |
| `--point-size` | número | `1` | Tamaño del punto, en píxeles. |
| `--yaw` | número | `30` | 3D: rotación y elevación en grados. |
| `--pitch` | número | `35` | 3D: elevación de la cámara, en grados. |
| `--profile` | texto o número | ninguno | Perfil: x1,y1,x2,y2,ancho de la caja del perfil. |
| `--shade` | texto o número | ninguno | Planta: TIN sombreado bajo los puntos, a partir de las clases dadas (p. ej. 2; vacío: todas) |
| `--contours` | texto o número | ninguno | Planta: curvas de nivel cada tanto. |
| `--class` | texto o número | ninguno | Clases que se dibujan: 2,5 o ~7,18. |

## Flujos de trabajo

Tres órdenes más no procesan datos, sino flujos del constructor (capítulo 13). `hipso schema` imprime en JSON la definición de todas las órdenes y opciones de esta página, que es lo que lee el constructor para ofrecer cada herramienta. `hipso run flujo.model.json` ejecuta un flujo guardado sin interfaz (con `--input nombre=ruta` para cambiar una entrada y `--results-dir` para la carpeta de resultados), y `hipso model-validate flujo.model.json` lista lo que impide ejecutarlo.
