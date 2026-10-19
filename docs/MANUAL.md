# EasyLiDAR · Manual de uso

**EasyLiDAR: procesamiento y edición de nubes LiDAR para forestales.**
Autor: Julio M. Sevilla · julio@jsevilla.es · https://es.linkedin.com/in/juliomsevilla

---

## 1. Qué es EasyLiDAR

EasyLiDAR abre nubes de puntos LiDAR (LAS y LAZ), las clasifica, las edita, genera modelos del terreno y de copas, calcula métricas e inventarios forestales, mide y cubica árboles en nubes de escáner terrestre y trabaja con imágenes y capas de un SIG.

Se diseñó con tres premisas:

- **Rapidez.**
- **Eficiencia con nubes enormes:** los procesos leen la nube como un flujo y la memoria depende del área y del tamaño de celda, no del número de puntos. Los pasos locales trabajan por teselas, en paralelo, una por núcleo.
- **Edición cómoda de nubes de puntos:** seleccionar, reclasificar y borrar con pocos clics, deshacer y rehacer, y ver el resultado al instante en planta, perfil y 3D.

Funciona en Windows, macOS y Linux. Es gratuito (ver el archivo `LICENSE`).

## 2. Instalación

No necesita instalador.

1. Descomprime el ZIP de tu sistema en una carpeta con permisos de escritura.
2. Ejecuta `easylidar-gui` (`easylidar-gui.exe` en Windows). En macOS y Linux, si no arranca, da permiso de ejecución: `chmod +x easylidar-gui easylidar`.
3. `easylidar` es la versión de línea de comandos (capítulo 14).

En macOS, la primera vez: clic derecho sobre el programa → Abrir. En Windows, si aparece el aviso azul de SmartScreen: Más información → Ejecutar de todas formas.

Requisitos: Windows 10/11, macOS 12 o superior, o un Linux con X11/Wayland y OpenGL 2 o superior. En Linux hacen falta además las bibliotecas GTK 3 para los diálogos de archivo.

## 3. Primeros pasos

1. **Inicio → Abrir** (o arrastra los archivos a la ventana). Se pueden abrir varias nubes: cada una es una **hoja**.
2. La nube se dibuja en la ventana. Rueda del ratón para acercar; botón central para volver al zoom anterior.
3. **Procesar** reúne las herramientas. Cada una abre una ventana con sus parámetros y el botón **▶ Ejecutar**.
4. Los resultados se guardan junto a los datos, en la carpeta `results` o `resultados`, y se cargan como capas.

Todo trabajo se puede detener con **■ Detener**: se borra lo que se hubiera escrito a medias.

## 4. La interfaz

- **Cinta de opciones**, con pestañas: *Inicio*, *Vista*, *Edición*, *Procesar* y *Ayuda*. Al elegir una capa aparecen pestañas propias (simbología y edición vectorial para capas vectoriales, ráster para imágenes y rásteres).
- **Panel lateral**: *Capas* (nubes, imágenes, rásteres y vectores; casilla de visibilidad, símbolo y menú del botón derecho), *Información* y *Edición*.
- **Barra de estado**: coordenadas del cursor, herramienta activa y escala.
- **Registro** (Inicio → Registro): historial de lo que se ha hecho y de los errores.
- **Buscar comando** (Ctrl+K): escribe el nombre de cualquier herramienta.
- **Tema claro y oscuro** (Inicio → Tema).

### 4.1 Ventanas de trabajo

En **Vista → Ventanas** se elige cómo se reparte la pantalla:

| Disposición | Qué muestra |
|---|---|
| 2D | La planta. |
| 2D + perfil | La planta y, debajo, el perfil de una franja de la nube. |
| 3D | Solo la vista 3D, a pantalla completa y **independiente** de la planta. |
| 2D + perfil + 3D | Las tres a la vez. La vista 3D solo sigue a la planta si se marca *Enlazar con la planta* en el menú *Vista 3D…*. |

### 4.2 Navegar en la planta

| Acción | Ratón / tecla |
|---|---|
| Acercar | Rueda, o clic con la herramienta Zoom (×2) |
| Alejar | Clic derecho con Zoom |
| Zoom a una ventana | Arrastrar con Zoom |
| Desplazar | Herramienta Desplazar (X) o botón central |
| Zoom anterior | Botón central, o *Zoom anterior* |
| Extensión completa | *Extensión completa* |

### 4.3 Navegar en 3D

| Acción | Ratón / tecla |
|---|---|
| Girar (sin tope, también por debajo) | Botón izquierdo |
| Mover en el plano de la pantalla | Botón derecho o central (o Mayús + izquierdo) |
| Acercar y alejar | Rueda, o Ctrl + izquierdo |
| Girar con el teclado | Flechas |
| Acercar con el teclado | Re Pág / Av Pág |
| Encuadrar todo | Doble clic o tecla Inicio |
| Norte arriba, exageración vertical | Menú *Vista 3D…* |

### 4.4 Colorear los puntos

**Vista → Puntos → Colorear por**: elevación (dos rampas), clasificación, intensidad, número de retorno, ID de fuente, archivo o RGB. Se puede fijar el rango de elevación, el tamaño del punto (+ / −), filtrar retornos y elegir las clases visibles.

## 5. Herramientas de consulta

- **Consultar (Q)**: clic en un punto para ver sus atributos.
- **Medir (W)**: distancias y áreas sobre la planta.
- **Densidad**: puntos por metro cuadrado de un recuadro.
- **Perfil**: tres clics (extremo izquierdo, extremo derecho y borde de la caja) crean un perfil. Las flechas lo desplazan a lo largo de la nube; el ancho se ajusta en el panel.
- **TIN sombreado (T)** y **curvas de nivel**.

## 6. Edición de nubes de puntos

La edición es por **selecciones**: eliges puntos, y luego les haces algo.

1. **Región:** rectángulo, círculo o polígono dibujado en la planta o en el perfil. En el perfil se seleccionan puntos por su altura (copas, ruido por encima o por debajo del suelo…) y en la planta por su posición.
2. **Modo:** nueva selección, añadir, restar, intersectar o invertir.
3. **Filtro de atributos:** clases, rango de Z, rango de intensidad y tipos de retorno. Solo se seleccionan los puntos que lo cumplen.
4. **Acción:** *Eliminar* o *Asignar clase* (la lista incluye todas las clases, también las nuevas).
5. **Deshacer / Rehacer** (Ctrl+Z / Ctrl+Y) y el **historial** de ediciones. *Descartar cambios* vuelve a la nube original.
6. **Guardar la nube editada**: se escribe un archivo nuevo; **el original nunca se modifica**. Las ediciones se aplican a todos los puntos del archivo, no solo a los que se ven en pantalla.

Con puntos seleccionados, las herramientas de *Procesar* trabajan solo sobre ellos.

### 6.1 Histogramas: detectar puntos raros

**Edición → Histogramas** abre una ventana con el histograma de un atributo de los puntos en pantalla: elevación, intensidad, nº de retorno, nº de retornos, clase, pasada (ID de fuente) y ángulo de barrido. Cada barra cuenta los puntos de un intervalo.

- **Marcar barras:** clic en una barra, o arrastra sobre varias (si empiezas en una marcada, desmarcas). Puedes marcar barras en varios atributos: un punto se selecciona si cae en las barras marcadas de **todos** ellos. Los atributos con marcas llevan un punto (●) en la lista.
- **Seleccionar puntos:** convierte las marcas en una selección, que se combina con la anterior según el modo de la cinta (nueva, añadir, restar, intersectar) y respeta el *Filtro de atributos*. Después *Eliminar* o *Asignar clase* (hay botones en la propia ventana).
- **Escala logarítmica** (activa por defecto): deja ver las barras con pocos puntos, que son las de los valores raros. *Solo los puntos seleccionados* hace el histograma únicamente de la selección actual, para afinarla.
- Debajo del gráfico: número de puntos, mínimo, máximo, media, desviación típica, mediana, cuartiles y MAD. La línea blanca de la base marca la mediana.

**Valores raros según un estadístico.** Para no buscarlos a ojo, elige un criterio y mira las líneas naranjas, que son sus límites:

| Criterio | Límites | Cuándo usarlo |
|---|---|---|
| Tukey | Q1 − k·RIC y Q3 + k·RIC (k = 1,5; con 3, solo extremos) | General; no le afectan los propios valores raros |
| Robusto (MAD) | mediana ± z·1,4826·MAD (z = 3,5) | Distribuciones muy asimétricas |
| Desviaciones típicas | media ± k·σ (k = 3) | Datos de forma parecida a la normal |
| Percentiles | el p % más bajo y más alto | Cuando se quiere una cantidad fija de puntos |

*Marcar los raros de este atributo* los marca como barras. *Seleccionar los raros de cualquier atributo* aplica el criterio a todos los atributos y selecciona los puntos raros en alguno de ellos. Si un atributo casi no varía (casi todos los valores iguales), el criterio no puede decidir y lo avisa.

Los estadísticos se calculan sobre los puntos que se ven en pantalla; al **guardar**, la regla se aplica a todos los puntos del archivo. El ángulo de barrido se compara en el visor con una décima de grado de resolución.

### 6.2 Clases ASPRS y clases nuevas

EasyLiDAR usa la tabla vigente de ASPRS (LAS 1.4): 0 sin clasificar nunca, 1 sin clasificar, 2 suelo, 3 a 5 vegetación baja, media y alta, 6 edificio, 7 punto bajo (ruido), 9 agua, 10 ferrocarril, 11 carretera, 13 y 14 cables, 15 torre, 16 conector, 17 tablero de puente, 18 ruido alto, 19 estructura elevada, 20 suelo ignorado, 21 nieve, 22 exclusión temporal. Los códigos 23 a 63 están reservados y **64 a 255 son libres para el usuario**.

Para añadir una clase: **Vista → Clases y colores** → escribe el código y el nombre → *Añadir*. Cambia su color con el selector. Las clases propias se guardan para la próxima vez.

Los LAS anteriores a la versión 1.4 solo guardan clases hasta la 31. Si guardas una edición con una clase mayor, el archivo se escribe como LAS 1.4 automáticamente.

## 7. Procesar

Las herramientas procesan **cada hoja por separado** y dan un resultado por hoja; nunca las unen (salvo el mosaico, si lo pides). Los nombres de los resultados se calculan a partir de la entrada y de los ajustes (`datos_1_mdt_1m.tif`) y no pisan resultados anteriores (`_2`, `_3`…).

### 7.1 Preparar

- **Información de los datos**: puntos, extensión, densidad, clases, retornos, fechas, sistema de coordenadas y avisos.
- **Asignar proyección**: da a una nube sin sistema de coordenadas el de otra nube o un código EPSG. Escribe una copia (`…_crs`).
- **Adelgazar nube**: reduce la densidad sin cambiar la extensión. Sirve para llevar a la misma densidad nubes de coberturas distintas (por ejemplo, la primera y la tercera del PNOA) antes de compararlas.
  - *Densidad objetivo*: puntos por metro cuadrado que deben quedar, al azar (una nube menos densa se deja igual).
  - *Un punto por celda*: el más cercano al centro, el más bajo, el más alto o el del percentil elegido (50 = mediana).
  - *Celdas 3D*: conserva paredes y pendientes.
  - *Uno de cada n*, *eliminar uno de cada n* y *muestreo al azar*.
- **Clasificar**: elige lo que quieras. Los pasos se aplican por teselas, en este orden: ruido, suelo, edificios, vegetación por alturas, cables y postes, y por último las reglas por eco y ángulo.
  - *Ruido*: **puntos aislados** (un punto sin apenas vecinos en un bloque de 3 × 3 × 3 celdas, clase 7) o filtro estadístico **SOR** (se descarta el punto cuya distancia media a sus k vecinos supera la media general más m desviaciones).
  - *Suelo* (clase 2), a elegir:
    - **Iterativo de Kraus y Pfeifer**: ajusta una superficie por mínimos cuadrados con pesos; los puntos muy por encima de ella pesan menos en cada vuelta, así que la superficie «baja» hasta el suelo. Después se aceptan los puntos que están a menos de una distancia de ella.
    - **Curvatura multiescala (MCC)**: compara cada punto con una superficie curva ajustada a varias escalas; el que se desvía menos que el umbral es suelo.
    - **Simulación de tela (CSF)**: se da la vuelta a la nube y se deja caer una tela con rigidez ajustable; los puntos que quedan a menos de un umbral de la tela son suelo.
    - **Morfológico progresivo (PMF)**: aperturas del mínimo con ventanas cada vez mayores; el punto que sube más que un umbral que crece con la ventana no es suelo.
    - **Axelsson (TIN progresivo)**: se parte de una red triangular de semillas, el punto más bajo de cada celda grande (mayor que el edificio más grande, 20 m por defecto), y se van añadiendo los puntos que están cerca del plano de su triángulo (**distancia de iteración**, 1,4 m) y cuyas líneas a los tres vértices forman con el plano un ángulo pequeño (**ángulo de iteración**, 8°). Se rehace la red y se repite hasta que no entra ningún punto. Los puntos a menos de la **tolerancia llana** (10 cm) del plano son suelo sea cual sea el ángulo: cerca de un vértice el ángulo sale grande incluso para un punto sobre la superficie. Sirve en terrenos con pendiente y con edificios; si hay estructuras mayores que la celda de las semillas, súbela.
    - **Clase 2 existente**: respeta la clasificación que trae la nube.
  - *Edificios*: se obtiene un terreno de referencia con una apertura morfológica mayor que cualquier edificio; las celdas que lo superan más que la altura mínima forman regiones, y una región es edificio si es lo bastante grande, plana (los tejados son planos, las copas rugosas) y está limitada por saltos bruscos. Los puntos de tejado son clase 6 y el resto de lo que supera esa altura, vegetación alta (5).
  - *Vegetación por alturas*: altura del punto sobre el suelo de su entorno: baja (3) por debajo del primer límite, media (4) hasta el segundo y alta (5) por encima.
  - *Cables* (14): puntos de más de una altura mínima sobre el suelo con vecinos alineados (linealidad = (λ₁ − λ₂)/λ₁ de la matriz de covarianza del entorno, mínimo 0,9) y dirección casi horizontal. *Postes y torres* (15): lo mismo pero casi vertical y de poca anchura.
  - *Reglas por eco y ángulo*: cambian la clase de los puntos según el **eco** del pulso y el **ángulo de barrido**, al final y en orden (una regla ve lo que hizo la anterior). Cada regla tiene: las clases de partida (por defecto, todas menos el ruido 7 y 18), los ecos (único, primero de varios, intermedio, último), un intervalo de ángulo absoluto y la clase de destino. Dos usos habituales: pasar a una clase aparte (12, por ejemplo) los puntos de ángulo grande, que son menos exactos y conviene dejar fuera del suelo y de las superficies; y pasar a vegetación alta (5) los primeros ecos de pulsos con varios retornos que aún no tienen clase. Si una regla quita la clase 2 a un punto, ese punto deja de ser suelo también para el MDT. En la línea de comandos: `--rule "angle=25;to=12"` (se repite para varias; partes `to=`, `from=1,2` o `from=~7`, `returns=SFIL` o números, `angle=25..90`).
  - *Teselas y rendimiento*: tamaño de tesela, búfer (franja alrededor de cada tesela, para que las decisiones en los bordes se tomen con los vecinos) e hilos.

### 7.2 Superficies

Todas se escriben como GeoTIFF y se calculan **hoja por hoja**.

- **MDT (modelo digital del terreno)**, a partir de los puntos de suelo (clase 2 de la nube, o el método que elijas al clasificar). Hay tres formas de dar valor a cada celda; **por defecto, la triangulación**:
  - **Triangulación (TIN)** *(predeterminado)*: por teselas, se queda con el punto de suelo más cercano al centro de cada celda, se triangula (Delaunay) y se interpola linealmente dentro de cada triángulo. Es una superficie continua que respeta los puntos. No se interpola a través de triángulos con aristas de más de 50 m; esos huecos se rellenan al final con la media de las celdas vecinas.
  - **Ponderado por distancia (IDW)**: cada celda toma la media de los 8 puntos de suelo más cercanos ponderada por 1/d². Antes de calcular, el suelo se reduce a un punto (la media) por cada media celda, así que el tiempo depende de la malla y no de la densidad. Da una superficie suave; es muy rápida. El alcance es de 30 celdas (entre 10 y 50 m).
  - **Media por celda**: la media de las elevaciones de los puntos de suelo de cada celda, en una sola pasada y sin cargar la nube. Es la más rápida y la que menos memoria usa, pero deja huecos donde no hay puntos (se rellenan al final).
- **MDS (modelo digital de superficie)**: lo más alto que ve el sensor (copas, tejados). Se calcula con los puntos que no son ruido, por defecto con la **triangulación**:
  - **Triangulación (TIN)** *(predeterminado)*: se toma el punto más alto de cada celda, se triangula y se interpola; sin espigas ni huecos pequeños.
  - **Máximo por celda**: la cota del punto más alto de cada celda, en una sola pasada; las celdas vacías se rellenan (casilla *Rellenar*).
  - **Ponderado por distancia (IDW)**: sobre los puntos más altos de cada media celda.
- **CHM (altura de copas)**: por celda, la altura del punto más alto sobre el terreno, `z − MDT` (las alturas negativas valen 0). Con *pit-free* se apilan varias triangulaciones de los puntos por encima de umbrales de altura (0, 2, 5, 10, 15 m…) y se toma el máximo de capas: quita los «pozos» que deja el máximo por celda en copas con huecos. Usa el MDT **de esa misma nube**.
- **Suavizado** (MDT, MDS y CHM): ventana circular de media, mediana o gauss, con el tamaño en metros.

Recomendación: celda de 1 m para el MDT en terreno forestal con 1 a 10 puntos/m² (PNOA); 0,5 m para el MDS y el CHM. Si hay pocos puntos de suelo bajo copas densas, una celda mayor da un terreno más fiable.

### 7.3 Inventario

#### Métricas de rodal

Cuadricula la nube en celdas (15 m por defecto; la rejilla está alineada a múltiplos del tamaño de celda, de modo que las celdas de hojas vecinas encajan). En cada celda calcula estadísticos de las **alturas sobre el suelo** de todos los puntos que no son ruido: `altura = z − MDT` (el MDT de esa misma nube). Las celdas con menos puntos que el mínimo (4) no dan métricas. Se añaden las columnas de fecha de captura (`Date`).

Opciones: *límite de altura* de cobertura (2 m), *intensidad* (los mismos estadísticos de la intensidad), *estratos* (estadísticos por franjas de altura: por defecto hasta 0,15, 1,37, 5, 10, 20 y 30 m) y la *memoria* máxima (se procesa por bandas de filas que caben en ella: la nube se lee una vez por banda).

**Qué es cada métrica.** `h` son las alturas de los puntos de la celda; `n`, su número.

| Métrica | Cómo se calcula |
|---|---|
| Total return count | n, número de puntos de la celda |
| Elev minimum, maximum, mean | Mínimo, máximo y media de h |
| Elev mode | Moda de h, estimada con un histograma |
| Elev stddev, variance, CV | Desviación típica, varianza y coeficiente de variación (desviación / media) |
| Elev IQ | Rango intercuartílico: P75 − P25 |
| Elev skewness, kurtosis | Asimetría y curtosis (momentos de tercer y cuarto orden) |
| Elev AAD | Desviación media absoluta respecto a la media |
| Elev MAD median, MAD mode | Mediana de las desviaciones absolutas respecto a la mediana, y respecto a la moda |
| Elev L1 a L4 | Momentos L (L1 = media; L2 mide la dispersión; L3 y L4, la forma) |
| Elev L CV, L skewness, L kurtosis | L2 / L1, L3 / L2 y L4 / L2 |
| Elev P01 … P99 | Percentiles 1, 5, 10, 20, 25, 30, 40, 50, 60, 70, 75, 80, 90, 95 y 99 de h (interpolación lineal entre valores ordenados) |
| Return 1 … 9 count, Other return count | Puntos de cada número de retorno (los de número 10 o superior, en «otros»). «Primer retorno» es el retorno número 1 |
| Percentage first returns above *hb* | 100 · primeros retornos con h > *hb* / total de primeros retornos. Es la **cobertura de copas** con el límite *hb* |
| Percentage all returns above *hb* | 100 · todos los retornos con h > *hb* / total de retornos |
| (All returns above *hb*) / (Total first returns) · 100 | Retornos por encima de *hb* sobre primeros retornos, en % (proxy de densidad de follaje) |
| First / All returns above *hb* | Recuentos de los anteriores |
| Percentage first / all returns above mean (mode) | Lo mismo con la media (o la moda) de la celda como límite |
| (All returns above mean / mode) / (Total first returns) · 100 | Ídem |
| First / All returns above mean (mode) | Recuentos |
| Total first returns, Total all returns | Denominadores usados |
| Canopy relief ratio | (media − mínimo) / (máximo − mínimo): cuánto del rango vertical ocupa la masa del follaje |
| Elev quadratic mean, cubic mean | Media cuadrática (√(Σh²/n)) y cúbica (∛(Σh³/n)) |
| Profile area | Área bajo la curva de los percentiles 1 a 99 de h (las alturas negativas valen 0), normalizada por el percentil 99 |
| Int … (con *intensidad*) | Los mismos estadísticos (mínimo, máximo, media, moda, desviación, percentiles…) de la intensidad |
| Elev strata (de a a b) … (con *estratos*) | Para cada franja de altura: número de puntos, proporción sobre el total de la celda, mínimo, máximo, media, moda, mediana, desviación, CV, asimetría y curtosis de las alturas de esa franja |

La metodología de las métricas es la de los estadísticos de altura habituales en inventario con LiDAR (percentiles, cobertura, momentos); son las variables explicativas que se usan en los modelos de volumen, biomasa o altura dominante.

**Nombres de los campos del shapefile.** El formato dBASE de un shapefile limita los nombres a 10 caracteres, así que se abrevian. El **CSV conserva los nombres largos**, y junto al shapefile se escribe `<nombre>_campos.txt` con la correspondencia exacta de esa ejecución. Las abreviaturas son estas (con límite de cobertura 2 m; el número cambia con el límite que elijas):

| Campo del shapefile | Columna del CSV (métrica) |
|---|---|
| `row` | row |
| `col` | col |
| `CX` | center X |
| `CY` | center Y |
| `TRN` | Total return count |
| `EMin` | Elev minimum |
| `EMax` | Elev maximum |
| `EMn` | Elev mean |
| `EMo` | Elev mode |
| `ESd` | Elev stddev |
| `EVar` | Elev variance |
| `ECv` | Elev CV |
| `EIq` | Elev IQ |
| `ESkw` | Elev skewness |
| `EKur` | Elev kurtosis |
| `EAad` | Elev AAD |
| `EL1` | Elev L1 |
| `EL2` | Elev L2 |
| `EL3` | Elev L3 |
| `EL4` | Elev L4 |
| `ELCv` | Elev L CV |
| `ELSkw` | Elev L skewness |
| `ELKur` | Elev L kurtosis |
| `EP01` | Elev P01 |
| `EP05` | Elev P05 |
| `EP10` | Elev P10 |
| `EP20` | Elev P20 |
| `EP25` | Elev P25 |
| `EP30` | Elev P30 |
| `EP40` | Elev P40 |
| `EP50` | Elev P50 |
| `EP60` | Elev P60 |
| `EP70` | Elev P70 |
| `EP75` | Elev P75 |
| `EP80` | Elev P80 |
| `EP90` | Elev P90 |
| `EP95` | Elev P95 |
| `EP99` | Elev P99 |
| `R1N` | Return 1 count |
| `R2N` | Return 2 count |
| `R3N` | Return 3 count |
| `R4N` | Return 4 count |
| `R5N` | Return 5 count |
| `R6N` | Return 6 count |
| `R7N` | Return 7 count |
| `R8N` | Return 8 count |
| `R9N` | Return 9 count |
| `ORN` | Other return count |
| `PcFRAb200` | Percentage first returns above 2.00 |
| `PcARAb200` | Percentage all returns above 2.00 |
| `ARAb200TFR` | (All returns above 2.00) / (Total first returns) * 100 |
| `FRAb200` | First returns above 2.00 |
| `ARAb200` | All returns above 2.00 |
| `PcFRAbMn` | Percentage first returns above mean |
| `PcFRAbMo` | Percentage first returns above mode |
| `PcARAbMn` | Percentage all returns above mean |
| `PcARAbMo` | Percentage all returns above mode |
| `ARAbMnTFR1` | (All returns above mean) / (Total first returns) * 100 |
| `ARAbMoTFR1` | (All returns above mode) / (Total first returns) * 100 |
| `FRAbMn` | First returns above mean |
| `FRAbMo` | First returns above mode |
| `ARAbMn` | All returns above mean |
| `ARAbMo` | All returns above mode |
| `TFR` | Total first returns |
| `TAR` | Total all returns |
| `EMadMed` | Elev MAD median |
| `EMadMo` | Elev MAD mode |
| `CanRelRat` | Canopy relief ratio |
| `EQmMn` | Elev quadratic mean |
| `ECmMn` | Elev cubic mean |
| `PfAr` | Profile area |
| `Date` | Date |

Con *intensidad* los campos de `TRN` a `ECmMn` salen con `I` en lugar de `E` (por ejemplo `IMn`, `IP95`, `ISd`) y la columna del CSV empieza por `Int` en lugar de `Elev`.


Las columnas de estratos se llaman `ES<n><estadístico>` (altura, *Elev*) e `IS<n><estadístico>` (intensidad), donde `<n>` es el número del estrato (0 es el primero, el de menor altura) y el estadístico va abreviado (`TRN` recuento de puntos, `Rpro` proporción sobre el total de la celda, `Min`, `Max`, `Mn` media, `Mo` moda, `Med` mediana, `Sd` desviación, `Cv` coeficiente de variación, `Skw` asimetría, `Kur` curtosis; por ejemplo `ES0Mn` es la media de las alturas del primer estrato). El `_campos.txt` dice qué franja es cada número.

#### Segmentación de árboles

Detecta cimas y copas sobre el CHM:

1. **Suavizado** opcional del CHM.
2. **Cimas**: máximos locales en una ventana (fija, o variable según la altura: `a + b · h`; circular o cuadrada) por encima de una altura mínima.
3. **Copas**, a elegir: *Dalponte y Coomes* (crece una región desde cada cima añadiendo celdas vecinas que no sean demasiado más bajas que la cima ni que la media de la copa), *Silva* (celdas dentro de un radio proporcional a la altura de la cima y por encima de una fracción de ella) o *cuencas* (watershed sobre el CHM invertido).
4. Salidas: puntos de las cimas con altura y área de copa, polígonos de copa, ráster de identificadores y, si se pide, el `treeID` de cada punto de la nube.
5. **Evaluación con inventario de campo**: empareja cada árbol medido con la cima más cercana (dentro de una distancia dada) y da aciertos, omisiones, comisiones y la medida F.

### 7.4 Recorte y salida

- **Recortar parcelas**: parcelas desde un archivo vectorial, generadas por muestreo (aleatorio o sistemático) o dibujadas. Da una nube por parcela, una nube con todas y/o una capa de parcelas con sus métricas.
- **Exportar puntos**: ASCII (CSV, TXT) o shapefile, eligiendo los campos.

### 7.5 Calidad y comparación

- **Quitar duplicados**: puntos con las mismas x, y, z (y, si se marca, la misma clase, tiempo GPS, intensidad, retorno o pasada). Se deja uno de cada grupo o ninguno.
- **Comparar nubes (cambios)**: escribe los puntos de la nube cargada que están a más de una distancia de todos los de otra nube de referencia. Da la distancia media, el RMS y la compleción. Los puntos de cambio aislados se descartan.
- **Ajustar una nube a otra**: la mueve para que encaje con la de referencia (punto más cercano iterativo), solo desplazando o desplazando y girando. Muestra el desplazamiento y el giro.
- **Separación entre pasadas**: ráster con la diferencia de altura entre pasadas de vuelo (ID de fuente) y el porcentaje de celdas por encima de 8 y 16 cm.

### 7.6 TLS: Cubicar árboles

Para nubes de escáner terrestre, móvil o fotogramétrico de una parcela. Cada nube es una hoja.

1. La nube se reduce a un punto por vóxel (3 cm por defecto).
2. Calcula el terreno (con la clase 2 si la nube la tiene, o simulando una tela) y las alturas sobre el suelo. Si la nube ya viene normalizada, marca la casilla.
3. Busca troncos en una franja (de 0,7 a 3,5 m): puntos de entorno vertical, agrupados y limpiados de ramas.
4. Sigue cada tronco hacia arriba y hacia abajo, ajustando un círculo a una rebanada horizontal cada pocos centímetros. Una sección vale si tiene puntos suficientes, repartidos alrededor y sin puntos dentro. El centro de la siguiente se predice, así que sigue troncos inclinados o curvos. Si una zona del tronco no se ve (una oclusión, follaje, una parte con pocos puntos), el seguimiento **sigue más allá**, ensanchando la búsqueda, hasta un **hueco máximo sin medir** (2 m por defecto); a partir de ahí se da el tronco por perdido. Las secciones del hueco se rellenan por interpolación (`INTERPOLADO`).
5. **Cubica con la fórmula de Smalian**: para cada troza, V = (A₁ + A₂) / 2 · L, con A el área del círculo de cada extremo y L la longitud de la troza. Se empieza en la **altura de la base** (10 cm sobre el suelo por defecto) y se termina donde el diámetro baja al **diámetro en punta delgada** (7 cm por defecto); la última troza se corta en ese punto.
6. La **longitud de la troza** (0,5 m por defecto) es la separación entre los diámetros medidos a lo largo del tronco.
7. **Varios troncos en un mismo pie** (masas de rebrote, como el eucalipto): en cada nivel de la franja se buscan **todos** los círculos del grupo, no uno solo, y se encadenan de un nivel a otro; así dos troncos pegados son dos troncos y no un círculo ancho que los envuelva. La regla para tratarlos es la altura del diámetro (1,30 m):
   - **Brotes de una cepa:** si los troncos están separados a 1,30 m (se bifurcan por debajo), cada uno es un árbol, con su DAP y su volumen. Comparten el número de **cepa** (`CEPA`) y el aviso `BROTES`, y `BROTES` cuenta cuántos tiene la cepa.
   - **Árbol bifurcado:** si a 1,30 m es un solo tronco y se bifurca por encima, es un solo árbol: el DAP es el del tronco, y el volumen es el del tronco hasta la bifurcación más el de cada rama desde ella (`RAMA` en las trozas; `H_BIF` es la altura de la bifurcación; aviso `BIFURCADO`).
   - El mismo tronco encontrado dos veces se cuenta una vez. Los troncos cuyas bases están a menos de la **distancia entre bases** (0,6 m) son de la misma cepa.

**Resultado:**

- Un shapefile (y/o CSV) con un punto en la base de cada árbol: `DAP_CM`, `ALT_TOT`, `ALT_COM` (altura hasta la punta delgada), `VOL_M3`, `NTROZAS`, `D_BASE_CM`, `D_TOP_CM`, `AB_M2` (área basimétrica), `INCLIN`, `PUNTOS`, `CALIDAD`, `AVISOS`, `Date`, `CEPA` (número de cepa), `BROTES` (nº de árboles de esa cepa; 1 si va solo) y `H_BIF` (altura de la bifurcación, solo en árboles bifurcados).
- `…_trozas.csv`: cada troza de cada árbol, con alturas, diámetros, longitud y volumen; `RAMA` es 0 para el tronco y 1, 2… para cada rama de un árbol bifurcado.
- Si se marca *Guardar también la nube de cada árbol*: una carpeta `…_arboles` con un archivo por árbol. Cada punto va al eje de tronco más cercano dentro de la distancia indicada.

Avisos: `TRUNCADO` (el tronco se pierde antes de llegar a la punta delgada), `SIN_DAP` (no hay sección válida a 1,3 m), `INTERPOLADO` (se rellenaron huecos), `INCLINADO` (más de 25°), `SIN_VOLUMEN`, `BROTES` (pertenece a una cepa con varios brotes) y `BIFURCADO`.

Consejos: **el vóxel depende del equipo**. Con escáner terrestre estático (nubes densas y de poco ruido), 2 a 3 cm. Con escáner **móvil o SLAM** (GeoSLAM, mochila o de mano), 5 cm: el ruido es de varios centímetros y con vóxeles finos los troncos se pierden a pocos metros (hay un selector de equipo en el formulario, y `--voxel 0.05` en la línea de comandos). Un círculo se rechaza si hay vóxeles dentro de su mitad interior (*puntos dentro del círculo*, 5 por defecto) o si su residuo es mayor que el admitido: así no se miden como un solo tronco dos troncos pegados ni una rama con follaje. Además, si faltan troncos, baja los vóxeles mínimos de un tronco o la verticalidad mínima; si se cuelan arbustos, súbelos. Una sola posición de escáner solo ve una parte de cada tronco: baja los *sectores ocupados* con cuidado.

### 7.7 Volúmenes respecto al MDT

Calcula el volumen entre una superficie y el terreno, celda a celda: V = Σ (altura de la superficie − MDT) · celda². Las celdas por encima del terreno suman al **volumen sobre él** y las que quedan por debajo, al **volumen bajo él** (vaciados). Cada nube es una hoja y busca sus rásteres en su carpeta de resultados (o los eliges a mano).

- **CHM de cada nube:** las alturas ya son sobre el suelo, así que no hace falta MDT. Es el volumen de la vegetación o de lo que sobresale del terreno.
- **MDS menos MDT:** superficie y terreno de la nube. El MDT se muestrea en el centro de cada celda del MDS, y las celdas que el MDT no cubre no se miden (se cuentan aparte).
- **MDS sobre una cota horizontal:** volumen por encima de un plano a la elevación indicada (acopios, por ejemplo).
- **Altura mínima que cuenta:** las celdas más cerca del terreno que esta altura no suman. Quita el ruido de la superficie.
- **Zonas:** todo el ráster, las AOI dibujadas (un volumen por AOI) o los polígonos de un archivo vectorial. Si las zonas se solapan, se miden una a una.

**Resultado:** un `…_volumen.csv` con, por zona, `Area_m2`, `Vol_sobre_m3`, `Vol_bajo_m3`, `Vol_neto_m3`, `H_media_m` y `H_max_m` (de las celdas sobre el terreno) y `Celdas_sin_MDT`. La precisión es la del ráster: usa la misma celda para el MDS y el MDT cuando puedas.

## 8. Teledetección

- **Composición de bandas**: color natural, falso color infrarrojo y otras, con estirado del contraste, brillo y gamma.
- **Índices espectrales**: NDVI, GNDVI, NDWI, EVI, SAVI, NDMI, NBR, VARI, ExG o una expresión propia con `B1…Bn` o `RED`, `GREEN`, `BLUE`, `NIR`, `SWIR1`, `SWIR2`. El índice sale como capa ráster y se guarda en GeoTIFF.
- **Crear multibanda**: apila en una sola imagen las bandas de varios rásteres, en el orden de la lista (por ejemplo, R, G, B y NIR sueltos). En *Bandas* se eligen las de cada ráster (por ejemplo `1,3`); vacío = todas. Hasta 64 bandas.
- **Mosaico**: une rásteres con el mismo número de bandas. **Donde se solapan** se elige qué hacer: *primero*, *último*, *media*, *mínimo*, *máximo*, *mediana*, *suma* o *número de capas*. También: zona que cubre (unión, intersección o la del primero), remuestreo (vecino, bilineal o media), tipo de dato, tamaño de píxel y valor de «sin datos».

Estas herramientas leen las imágenes por ventana y escriben comprimido por franjas: se pueden unir imágenes de varios gigabytes con poca memoria. Todos los rásteres deben estar en el mismo sistema de coordenadas (no se reproyecta).

## 9. Capas

- **Cargar capas**: imágenes y rásteres GeoTIFF, shapefiles. También se pueden arrastrar a la ventana.
- **Nueva capa**: puntos, líneas o áreas. Se digitaliza en la capa activa (con ajuste a vértices), se edita su **tabla de atributos** y se guarda como shapefile con la proyección de la nube.
- **Simbología**: color único, por categorías o por rampa, con clasificación y etiquetas.
- **Edición vectorial**: seleccionar, mover, editar vértices, deshacer y rehacer.
- Las capas de áreas visibles alimentan la herramienta de parcelas.

## 10. Rutas y formatos

- Los resultados van a `results` (si el proyecto tiene `data` y `results`) o a `resultados` junto a la entrada.
- Una ejecución nunca hereda la ruta de otra ni sobrescribe sin avisar. Si eliges una ruta a mano que ya existe, se pide confirmación.
- Tablas (métricas, árboles, parcelas): capa vectorial (shapefile, con el `.prj` de la nube), CSV o ambos. Los nombres de campo de un shapefile tienen como mucho 10 caracteres; `<nombre>_campos.txt` explica cada uno.
- Toda salida de métricas lleva la fecha de captura (`Date`), sacada del tiempo GPS de los puntos.

## 11. Rendimiento y memoria

- Las nubes grandes no se cargan enteras: la vista dibuja una muestra (*Puntos en pantalla*) y los procesos leen en flujo.
- Un LAZ que necesita varias pasadas se descomprime una vez a un archivo temporal que se borra al terminar (deja espacio libre en el disco temporal: la nube sin comprimir).
- Teselas: tamaño y búfer se ajustan en *Teselas y rendimiento*. La memoria es aproximadamente (núcleos × tesela). Con teselas de 250 m y 30 m de búfer, un equipo de 16 GB trabaja sin problema con nubes de cualquier tamaño.
- Qué pasos son en flujo (memoria casi constante): información, MDT por *media por celda*, CHM por máximo, MDS por máximo, métricas, recorte, exportar, adelgazar, duplicados. Qué pasos son por teselas (memoria = núcleos × tesela): clasificar, MDT por TIN o IDW, MDS por TIN o IDW, CHM *pit-free*.
- Para trabajar solo con lo que está en pantalla, selecciona los puntos: el proceso se aplica solo a ellos.

## 12. Trabajar con miles de teselas

Un vuelo es una colección de cientos o miles de hojas (por ejemplo, las de 2 × 2 km del PNOA). EasyLiDAR las trata así:

- **Cada hoja se procesa por separado y da su propio resultado.** Si una hoja falla, el proceso se detiene con el mensaje del error y se borra lo parcial de esa hoja (las hojas ya terminadas se conservan). No hay límite de hojas porque nunca se mantienen varias en memoria a la vez.
- **Se cargan como hojas solo las que quieras ver.** Para procesar, no hace falta abrirlas: en la **línea de comandos** se pasa la lista de archivos y no se dibuja nada. La interfaz sirve para probar los parámetros en una o dos hojas.
- **Prueba en una hoja representativa** (con relieve, bosque cerrado y claros) antes de lanzar el lote: elige el método de suelo, el tamaño de celda y las alturas.
- **Usa LAZ** como entrada: pesa unas 5 veces menos y se descomprime en paralelo.
- **Las salidas llevan el nombre de la hoja** (`hoja_mdt_1m_tin.tif`) y van a la carpeta `results`. Una ejecución nueva no sobrescribe: crea `_2`, `_3`… Para repetir un lote, usa otra carpeta de resultados o borra lo anterior.
- **Alineación entre hojas.** Las celdas de las métricas están alineadas a múltiplos del tamaño de celda: si el lado de la hoja es múltiplo de la celda (2000 m con celdas de 20 m), las celdas de hojas vecinas encajan sin solaparse ni dejar huecos. Si no lo es, las celdas del borde quedan con una parte de los puntos: ponles un mínimo de puntos más alto o usa un tamaño de celda divisor del lado de la hoja.
- **Mosaicos.** MDT y CHM pueden dar además un mosaico de todas las hojas (casilla o `--mosaic`); para el resto de rásteres usa *Mosaico* en Teledetección. Los mosaicos de miles de hojas se escriben por franjas comprimidas, sin cargar las imágenes enteras.
- **Bordes.** La clasificación por teselas usa un búfer alrededor de cada tesela *interna* de la hoja; en el borde de la hoja no hay vecinos al otro lado. Si los bordes importan (MDT continuo entre hojas), genera el MDT sobre varias hojas a la vez (se unen y se trocean de nuevo por teselas internas) o añade un búfer con las hojas vecinas.

Ejemplo con la línea de comandos (PowerShell; en Linux y macOS es igual con comodines):

```
$hojas = Get-ChildItem hojas\*.laz
easylidar info $hojas
easylidar classify $hojas --ground csf --noise isolated
easylidar dtm $hojas --surface tin --cell 1
easylidar dsm $hojas --method tin --cell 0.5
easylidar chm $hojas --cell 0.5 --method pitfree
easylidar metrics $hojas --cell 20 --strata
```

Los comandos que reciben varias hojas dan un resultado por hoja. Para paralelizar entre máquinas, reparte las hojas en lotes y lanza un proceso por lote y por carpeta de resultados.

## 13. Encadenar un flujo de inventario por área (ABA)

El enfoque por área (*area-based approach*, ABA) modela una variable de inventario (volumen, área basimétrica, biomasa, altura dominante…) con las métricas LiDAR de **parcelas de campo** y aplica el modelo a toda la superficie, celda a celda. Con EasyLiDAR se encadena así:

1. **Preparar las hojas.** Abre una hoja de prueba; revisa *Información* (proyección, densidad, clases, fecha de captura). Si falta la proyección: *Asignar proyección*. Si las coberturas son de densidades distintas: *Adelgazar nube* a una densidad común.
2. **Clasificar** (ruido y suelo) o usar la clase 2 de la entrega, y **generar el MDT** de cada hoja.
3. **Métricas de las parcelas de campo.** *Recortar parcelas* con el archivo de parcelas (o con centros y radio): modo **solo métricas**. Da una capa con un polígono por parcela y todas las métricas de la sección 7.3, con el número de parcela. Opcionalmente, una nube por parcela para revisarlas.
4. **Modelo.** Une (por el número de parcela) la tabla de métricas con la tabla del inventario de campo en la hoja de cálculo o en un programa estadístico, y ajusta el modelo (por ejemplo `log(V) ~ log(P95) + log(cobertura) + log(media)`). Selecciona las variables con criterio forestal y compruébalas con validación cruzada. **Usa los mismos parámetros** (límite de cobertura, tamaño de parcela, MDT) en las parcelas y en el resto del territorio.
5. **Métricas de toda la superficie.** *Métricas de rodal* en todas las hojas, con el tamaño de celda **igual al de la parcela** (por ejemplo, 400 m² → celdas de 20 m, o 15 m para parcelas de radio 7,98 m). Cada hoja da su capa de celdas con las mismas columnas que las parcelas.
6. **Aplicar el modelo** a la capa de celdas (en el SIG o en tu hoja de cálculo): el resultado es un mapa del inventario con una celda por cuadrícula. Si necesitas ráster, rasteriza el campo resultante.
7. **Contraste.** Compara con las parcelas retenidas para validar y con el CHM o la segmentación de árboles si quieres un segundo criterio.

Notas: las parcelas y la cobertura LiDAR deben ser de fechas próximas (la columna `Date` ayuda a comprobarlo); la parcela debe ser del tamaño de la celda; y conviene excluir las celdas con menos de unos cientos de puntos, las que tienen agua o edificios y las que caen en el borde de las hojas.

## 14. Línea de comandos

`easylidar` hace lo mismo que la interfaz, por lotes. Con `--help` se ve la lista completa de comandos y de opciones.

```
easylidar info nube.laz
easylidar thin nube.laz --method density --density 1
easylidar classify nube.laz --ground csf --noise isolated --buildings --vegetation
easylidar dtm nube.laz --cell 1 --surface tin
easylidar dsm nube.laz --cell 0.5 --method tin
easylidar chm nube.laz --cell 0.5 --method pitfree
easylidar metrics nube.laz
easylidar trees chm.tif --method silva --crowns
easylidar clip nube.laz --regions parcelas.shp --mode each
easylidar cubicar parcela_tls.laz --csv --tree-clouds --log-length 0.5 --top-diameter 0.07
easylidar dedup nube.laz --also class,gps
easylidar compare nube2.laz --reference nube1.laz --distance 0.5
easylidar fit nube2.laz --reference nube1.laz
easylidar swath vuelo.laz --cell 1
easylidar stack r.tif g.tif b.tif nir.tif -o multibanda.tif
easylidar mosaic a.tif b.tif -o mosaico.tif --overlap mean
easylidar render nube.laz --view 3d -o vista.png --color class
easylidar export nube.laz -o puntos.csv --items x,y,z,class --header
```

## 15. Atajos

| Atajo | Acción |
|---|---|
| Ctrl+O | Abrir nubes |
| Ctrl+K | Buscar comando |
| F1 | Manual |
| Q / W / Z / X | Consultar / Medir / Zoom / Desplazar |
| T | TIN sombreado |
| + / − | Tamaño del punto |
| Ctrl+Z / Ctrl+Y | Deshacer / Rehacer |
| Flechas | Mover el perfil (planta) o girar (3D) |
| Esc | Cancelar la operación en curso |

## 16. Solución de problemas

- **«No hay MDT de esta nube»**: genera antes el MDT con la herramienta MDT.
- **La nube no se ve completa**: sube *Puntos en pantalla* en el panel *Capas* y pulsa *Aplicar*.
- **Los diámetros de los árboles salen vacíos**: comprueba que la nube tenga suelo (o marca que ya está normalizada) y que la franja de búsqueda esté dentro de la altura de los troncos.
- **El mosaico dice que los sistemas son distintos**: reproyecta los rásteres al mismo sistema antes de unirlos.
- **Un resultado a medias tras detener un proceso**: se borra solo; si queda algún archivo, bórralo a mano.
- **La interfaz va lenta o a saltos**: mira qué usa para dibujar. Con una tarjeta gráfica moderna usa **wgpu** (DirectX 12, Vulkan o Metal). Con un driver antiguo (por ejemplo una Intel HD de 2015, que no llega a DirectX 12) wgpu solo ofrece un adaptador **por software**, que dibuja con el procesador y va muy lento, así que el programa elige entonces **OpenGL**. Para ver lo que detecta: `easylidar-gui --gpu-info`. Para medir los fotogramas por segundo con una nube: `easylidar-gui nube.laz --bench` (cuenta 8 s y sale). Para forzar uno: variable de entorno `EASYLIDAR_RENDERER` con `glow` (OpenGL) o `wgpu`.

## 17. Acerca de

**EasyLiDAR** · Procesamiento y edición de nubes LiDAR para forestales.
Autor: Julio M. Sevilla.
Contacto: julio@jsevilla.es · https://es.linkedin.com/in/juliomsevilla

Se agradecen todas las aportaciones y la detección de errores; los errores se irán corrigiendo en la medida de lo posible.

Uso gratuito. Consulta `LICENSE` para las condiciones y `THIRD_PARTY_LICENSES.md` y `NOTICE` para los componentes de terceros.
