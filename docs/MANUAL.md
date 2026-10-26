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

1. **Inicio → Abrir** (Ctrl+O, o arrastra los archivos a la ventana). Abre lo que le des —nubes de puntos, rásteres e imágenes, shapefiles y GeoJSON—, varios a la vez, y lo **añade** a lo que ya está abierto; *Cerrar todo* empieza de nuevo. Cada nube es una **hoja**. Junto a *Abrir*, **Recientes** guarda los diez últimos archivos abiertos, y todos los diálogos de archivo empiezan en la carpeta de la que abriste lo último; el programa lo recuerda entre sesiones (en `preferencias.txt`, en la carpeta de configuración del sistema).
2. La nube se dibuja en la ventana. Rueda del ratón para acercar; botón central para volver al zoom anterior.
3. **Procesar** reúne las herramientas en cinco grupos: *Preparar*, *Calidad*, *Superficies*, *Inventario forestal* e *Infraestructuras*. Cada una abre una ventana con sus parámetros y el botón **▶ Ejecutar**.
4. Los resultados se guardan junto a los datos, en la misma carpeta que la nube de origen (con otro nombre) y se cargan solos en el visor al terminar el proceso: el primer resultado queda el primero de la lista, encima y seleccionado (una nube resultado pasa a ser la primera nube y **la original se queda cargada**; si no la quieres, la quitas tú). Si un proceso da más de 8 nubes, se cargan las 8 primeras y un aviso lo dice; el resto queda en disco.

Todo trabajo se puede detener con **■ Detener**: se borra lo que se hubiera escrito a medias. Mientras corre, la barra de estado y el Registro dicen cuánto falta (apartado 7).

## 4. La interfaz

- **Cinta de opciones**, con pestañas: *Inicio*, *Vista*, *Procesar* y *Ayuda*. Al elegir una capa aparecen pestañas propias (simbología y edición vectorial para capas vectoriales, ráster para imágenes y rásteres).
- **Panel lateral**: *Contenido* (el mapa con sus capas y nubes en forma de árbol), *Información* (el informe de los datos; **⧉ Copiar todo** lo lleva entero al portapapeles, también con el botón derecho sobre el texto) y *Edición*. La edición de puntos vive entera en este panel (selección, edición, historial), así que puedes tener abierta la cinta de *Vista* y las herramientas de edición a la vez.
  - El panel *Contenido* es como la tabla de contenidos de un SIG. Arriba, un buscador y los botones *Abrir* y *Mostrar todo*. Debajo, el árbol, que es el **orden de dibujo**: **Mapa** (su casilla muestra u oculta todo), las **capas** y el grupo **Nubes de puntos**, de arriba abajo; lo de arriba se dibuja encima. El grupo de nubes se comporta como un elemento más: se sube y se baja entero por encima o por debajo de las capas (arrastrándolo o con el botón derecho). Los rásteres e imágenes nuevos se colocan bajo las nubes, y las capas vectoriales sobre ellas. Cada fila tiene su flecha (las capas con simbología por valores muestran sus clases debajo), su casilla de visibilidad, su símbolo y su nombre; con el ratón encima aparecen sus acciones: en una capa, la tabla de atributos (si es vectorial), el zoom y quitarla; en una nube, el zoom a esa tesela y cerrarla; en el grupo, el zoom a todas y cerrar todas. *Ver solo esta nube* está en el botón derecho de cada nube. Un clic selecciona, el doble clic hace zoom a la capa y el botón derecho abre el menú. **Arrastra una capa** (o el grupo de nubes) para cambiar su orden de dibujo. Las propiedades de la capa seleccionada (opacidad, colores) salen al pie del panel. Una casilla a medias en un grupo indica que solo algunos de sus elementos se ven.
- **Barra de estado**: coordenadas del cursor, el **sistema de coordenadas de la vista** (el de la primera nube, o «sin declarar»), herramienta activa, escala y, cuando hay un proceso en marcha, su avance y la estimación de tiempo.
- **Registro** (Inicio → Registro): historial de lo que se ha hecho, de los errores y del avance de los procesos largos.
- **Buscar comando** (Ctrl+K): escribe el nombre de cualquier herramienta.
- **Recientes** (Inicio → Archivo): los diez últimos archivos abiertos; *Vaciar la lista* los olvida.
- **Atajos de teclado**: **Ayuda → Atajos de teclado** abre una ventana con todos (el capítulo 15 los resume). Los botones de la cinta no repiten los atajos en su rótulo.
- **Tema claro y oscuro** (Inicio → Tema).
- **Iconos**: son propios, dibujados en SVG; si faltara alguno se ve un cuadro con una interrogación.

### 4.1 Ventanas de trabajo

En **Vista → Ventanas** se elige cómo se reparte la pantalla (*Disposición*), el tamaño de los puntos en la planta y el 3D y las opciones del 3D. Los grupos de la pestaña *Vista* están en este orden: *Navegación*, *Ventanas*, *Perfil* (ancho de la caja, tamaño de sus puntos y *Sin decimar*), *Carga de puntos* (el máximo de millones de puntos en pantalla y *Aplicar*; la decimación en uso sale en la barra de estado), *Visualización* (*Color*, rango de elevación y filtros) y *Herramientas*.

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

### 4.2.1 Mapa base

*Explorador de capas → «Mapa base» (la última fila; su casilla lo enciende y lo apaga).* Es un mapa de fondo, siempre bajo todas las capas, colocado en el sistema de coordenadas de la nube (hace falta que la primera nube lo declare). Fuentes: **OpenStreetMap** (mundial), **ortofoto PNOA** y **mapa base del IGN** (España), u otro servidor de teselas con una dirección del tipo `https://…/{z}/{x}/{y}.png`. Las teselas se descargan en segundo plano de los servidores del proveedor (nada se descarga con el mapa apagado), con el nombre del programa en la petición y como mucho dos descargas a la vez, y se guardan **una semana** en la carpeta de caché del sistema (en macOS, `~/Library/Caches/EasyLiDAR/tiles`), de modo que se reutilizan sin internet. El crédito del proveedor se dibuja en la esquina del mapa. Respeta los términos de cada proveedor: OpenStreetMap no admite descargas masivas.

### 4.2.2 Capas en otros sistemas de coordenadas

Al cargar, el visor **pasa al sistema de la primera nube** todo lo que está en otro: las demás nubes se convierten (se guarda una copia `<nombre>_epsg<código>.laz` junto a la original, que la próxima vez se carga sin convertir), los rásteres y las capas vectoriales se transforman en memoria y las imágenes de varias bandas se colocan convertidas, a la resolución de su vista previa. Solo si hay capas en sistemas distintos, el registro avisa antes de que se cargue que **tardará más**, y después dice qué se ha convertido y cuánto ha tardado. Las capas abiertas antes de cargar la nube se convierten cuando la nube fija el sistema. Una nube sin sistema declarado se carga tal cual (con aviso), y una nube en coordenadas geográficas (grados) no se convierte. Medido con hojas reales de 8 y 9 millones de puntos: cargar las dos juntas en el mismo sistema tarda 1,1 s; si una está en otro huso o en ED50 la primera vez tarda 2,1 s (unos 1,0 s más por cada 8 millones de puntos convertidos) y las siguientes 1,1 s.

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

**Vista → Visualización → Colorear por**: elevación (dos rampas), clasificación, intensidad, número de retorno, ID de fuente, archivo o RGB. Bajo el desplegable están *Rango de elevación…* y *Filtros…*. Se puede fijar el rango de elevación, filtrar retornos y elegir las clases visibles. Los puntos se ocultan o se muestran con la casilla de *Nubes de puntos* del panel *Contenido*; en la planta se ocultan solos mientras el TIN o las curvas de nivel están activos.

**Vista → Ventanas** tiene tres bloques con su etiqueta: *Vista* (distribución, *Tamaño 2D/3D* y *Vista 3D…*), *Perfil* (*Ancho*, *Tamaño* y *Todos los puntos*) y *Carga de puntos* (*Puntos en pantalla (M)* y *Aplicar*, con cuánta decimación hay). El panel *Contenido* tiene solo las capas y las nubes. Los tamaños de punto de 2D/3D y del perfil son independientes (las teclas + y − cambian el del perfil si el cursor está sobre él, y el de la planta y el 3D en cualquier otro caso). La longitud del perfil se ve en el propio perfil.

**Vista → Herramientas**: *Consultar*, *Densidad* y *Perfil* (botones grandes) y, en botones pequeños, *Medir*, *TIN* y *Curvas de nivel* (el engranaje junto a *TIN* abre la equidistancia y las clases del TIN).

## 5. Herramientas de consulta

- **Consultar (Q)**: clic en un punto para ver sus atributos.
- **Medir (W)**: distancias y áreas sobre la planta.
- **Densidad**: puntos por metro cuadrado de un recuadro.
- **Perfil**: tres clics (extremo izquierdo, extremo derecho y borde de la caja) crean un perfil. Las flechas lo desplazan a lo largo de la nube; el ancho se ajusta en *Vista → Ventanas*. **El perfil no usa los puntos decimados de la planta y el 3D:** lee de los archivos, en segundo plano, los de la zona de su caja (y un margen, para poder moverla sin releer) a máxima densidad, con un tope de 6 millones de puntos en memoria. Mientras llegan se dibuja con los de pantalla; arriba a la derecha se indica cuántos puntos hay (y «leyendo todos…» mientras tanto). En nubes COPC la lectura es casi inmediata; en un LAS/LAZ normal hay que recorrer el archivo (unos segundos en nubes grandes), saltando los archivos que no tocan la zona. El tamaño de los puntos del perfil es propio (por defecto 2 px; *Vista → Ventanas*) y no depende del de la planta y el 3D. La casilla *Todos los puntos* de *Vista → Ventanas* lo desactiva.
- **TIN sombreado (T)** y **curvas de nivel**.

## 6. Edición de nubes de puntos

La edición es por **selecciones**: eliges puntos, y luego les haces algo. Todas las herramientas están en el panel lateral, pestaña *Edición*: *Selección*, *Editar* e *Historial*. La navegación (zoom, desplazar, extensión, zoom anterior y *Limpiar marcas*) está en la cinta *Vista → Navegación*.

1. **Región:** rectángulo, círculo o polígono dibujado en la planta o en el perfil. En el perfil se seleccionan puntos por su altura (copas, ruido por encima o por debajo del suelo…) y en la planta por su posición.
2. **Combinar selección:** *Nueva*, *Añadir*, *Restar* o *Intersecar* (en el panel *Edición*, sección *Selección*; la selección se puede además invertir). Con el teclado: Mayús añade, Alt resta y Ctrl interseca (ver *Ayuda → Atajos de teclado*).
3. **Filtro de atributos:** clases, rango de Z, rango de intensidad y tipos de retorno. Solo se seleccionan los puntos que lo cumplen.
4. **Acción:** *Eliminar* o *Asignar clase* (la lista incluye todas las clases, también las nuevas).
5. **Deshacer / Rehacer** (últimos botones de *Editar*; Ctrl+Z / Ctrl+Y) y el **historial** de ediciones. Cada edición del historial tiene una casilla: **quítala para deshacer esa edición sola**, sin tocar las demás, y vuelve a marcarla para recuperarla (una edición posterior que elige por clase ve los puntos como están en ese momento). Las ediciones desactivadas no se guardan. *Descartar todo* vuelve a la nube original.
6. **Guardar la nube editada**: se escribe un archivo nuevo; **el original nunca se modifica**. Las ediciones se aplican a todos los puntos del archivo, no solo a los que se ven en pantalla.

Con puntos seleccionados, las herramientas de *Procesar* trabajan solo sobre ellos.

### 6.1 Histogramas: detectar puntos raros

**Edición → Editar → Histogramas** abre una ventana con el histograma de un atributo de los puntos en pantalla: elevación, intensidad, nº de retorno, nº de retornos, clase, pasada (ID de fuente) y ángulo de barrido. Cada barra cuenta los puntos de un intervalo.

- **Marcar barras:** clic en una barra, o arrastra sobre varias (si empiezas en una marcada, desmarcas). Puedes marcar barras en varios atributos: un punto se selecciona si cae en las barras marcadas de **todos** ellos. Los atributos con marcas llevan un punto (●) en la lista.
- **Seleccionar puntos:** convierte las marcas en una selección, que se combina con la anterior según el modo *Combinar selección* de la cinta (nueva, añadir, restar, intersecar) y respeta el *Filtro de atributos*. Después *Eliminar* o *Asignar clase* (hay botones en la propia ventana).
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


### 6.3 Alturas sobre el suelo

*Preparar → Alturas sobre suelo* (`easylidar hag`). Calcula la altura de cada punto sobre el suelo a partir de los **puntos de suelo (clase 2) de la propia nube**: una TIN de ellos da el suelo bajo cada punto, así que no hace falta un MDT aparte. Si la nube no tiene suelo clasificado se niega y lo dice (clasifícala antes). Se escribe una copia nueva (`…_alturas`) con la altura como atributo extra **HeightAboveGround** (decimal, lo leen otros programas) y las cotas intactas; con la opción de normalizar (`--replace-z`) la altura sustituye a la cota (`…_normalizada`) y se descartan los puntos sin suelo debajo. La **celda** del suelo (0,5 m) fija el detalle; en una hoja de 9 millones de puntos tarda unos 3 s. En el borde de la nube el suelo se rellena y la altura puede variar unos centímetros.

### 6.4 Quitar duplicados

*Edición → Editar → Quitar duplicados.* Elimina los puntos con las mismas x, y, z (y, si se marca, la misma clase, tiempo GPS, intensidad, retorno o pasada). Se deja uno de cada grupo o ninguno. **Cuándo usarlo:** nubes unidas de hojas que se solapan, o entregas con pulsos repetidos; los duplicados exactos distorsionan la densidad y las métricas. **Límite:** solo quita coincidencias exactas; puntos casi coincidentes (ruido de milímetros) no se detectan. Escribe un archivo nuevo.

## 7. Procesar

La pestaña **Procesar** reúne las herramientas en cinco grupos, en el orden en que suelen usarse:

1. **Preparar**: *Clasificar*; *Información*, *Asignar proyección* y *Exportar puntos* (también en el panel *Edición*, junto a la selección que exporta); *Adelgazar nube*, *Alturas sobre suelo* y *Quitar duplicados*; *Colorear con imagen*, *Normalizar intensidad* y *Guardar como COPC*.
2. **Calidad**: *Comparar nubes*, *Ajustar nubes* y *Entre pasadas*.
3. **Superficies**: *MDT*, *MDS / CHM*, *Análisis de terreno* y *Volumen sobre MDT*. Todas trabajan sobre el relieve y la altura de la vegetación.
4. **Inventario forestal**: *Métricas de rodal*, *Recortar parcelas*, *Segmentar árboles*, *Cubicar (TLS)*, *Modelos de transición*, *Cambio forestal* y *Análisis por objetos* (OBIA).
5. **Infraestructuras**: *Corredores eléctricos*.

La cinta se adapta al ancho de la ventana: si no caben todos los grupos, los de la derecha se pliegan en un botón grande con su nombre y una flecha (▾) que despliega sus herramientas, así que todas quedan siempre a un clic.

Las herramientas procesan **cada hoja por separado** y dan un resultado por hoja; nunca las unen (salvo el mosaico, si lo pides). Los nombres de los resultados se calculan a partir de la entrada y de los ajustes (`datos_1_mdt_1m.tif`) y no pisan resultados anteriores (`_2`, `_3`…).

**Cuánto falta.** Mientras un proceso corre, la barra de estado muestra el tiempo transcurrido y, si el proceso sabe cuánto trabajo tiene, el porcentaje y una estimación de lo que queda (`42 % · quedan ~ 3 min 20 s`). El **Registro** repite esa línea cada 30 segundos, también al cargar nubes grandes, para saber si el trabajo avanza o está colgado. La estimación es aproximada: se calcula con la velocidad media hasta ese momento y mejora a medida que avanza. Los procesos que no declaran su tamaño solo muestran el tiempo.

### 7.1 Preparar

- **Información de los datos**: puntos, extensión, densidad, clases, retornos, fechas, sistema de coordenadas y avisos.
- **Asignar proyección**: da a una nube sin sistema de coordenadas el de otra nube o un código EPSG. Escribe una copia (`…_crs`). Los puntos no se tocan, solo la cabecera.
- **Adelgazar nube**: reduce la densidad sin cambiar la extensión. **Cuándo usarla:** para llevar a la misma densidad nubes de coberturas distintas (por ejemplo, la primera y la tercera cobertura del PNOA) antes de compararlas o de calcular métricas, porque casi todas las métricas dependen de la densidad de puntos; y para aligerar nubes demasiado densas.
  - *Densidad objetivo*: puntos por metro cuadrado que deben quedar, al azar (una nube menos densa se deja igual).
  - *Un punto por celda*: el más cercano al centro, el más bajo, el más alto o el del percentil elegido (50 = mediana). El más bajo conserva el suelo; el más alto, las copas.
  - *Celdas 3D*: conserva paredes y pendientes.
  - *Uno de cada n*, *eliminar uno de cada n* y *muestreo al azar*.
  - **Ventaja:** es la forma más simple de homogeneizar. **Límite:** descarta información de forma irreversible; trabaja siempre sobre una copia.
- **Clasificar**: elige lo que quieras. Los pasos se aplican por teselas, en este orden: ruido, suelo, edificios, vegetación por alturas, cables y postes, y por último las reglas por eco y ángulo.

  **Ruido.**
  - **Puntos aislados**: un punto sin apenas vecinos en un bloque de 3 × 3 × 3 celdas es ruido (clase 7). Es rápido y basta para los puntos sueltos muy por encima o por debajo del grueso de la nube (pájaros, reflejos, errores del sensor). No detecta ruido en nubes dispersas.
  - **SOR (filtro estadístico)**: se descarta el punto cuya distancia media a sus *k* vecinos supera la media general más *m* desviaciones típicas (Rusu et al., 2008). Se adapta mejor a densidades que cambian de una zona a otra, pero cuesta más y puede borrar puntos legítimos en bordes y estructuras finas.

  **Suelo (clase 2).** No hay un método mejor en todos los casos: depende del relieve, de la densidad y de la vegetación. Prueba dos o tres en una hoja representativa y compara los MDT resultantes (hay que mirar el sombreado, no solo las cifras).

  | Método | Cuándo es más útil | Ventajas | Problemas |
  |---|---|---|---|
  | **Iterativo de Kraus y Pfeifer** | Bosque con suelo visible entre los árboles y relieve suave o medio | Pocos parámetros; robusto frente a la vegetación baja porque va dando menos peso a los puntos altos | Terreno muy abrupto y edificios grandes; en bosque muy denso puede dejar el suelo algo alto |
  | **Curvatura multiescala (MCC)** | Bosque cerrado con sotobosque y relieve ondulado | Pensado para bosque; resiste la vegetación baja | Sensible al umbral de curvatura y al número de escalas; más lento |
  | **Simulación de tela (CSF)** | Terreno de pendiente suave a media, con poca preparación | Pocos parámetros intuitivos (rigidez, resolución de la tela); rápido | En laderas muy pronunciadas o con escarpes la tela «cuelga»; hay que subir la rigidez en relieve y bajarla en llano. Con nubes poco densas (1 a 2 puntos/m²) conviene subir la resolución de la tela a 1 o 2 m, o se cuela vegetación en el suelo |
  | **Morfológico progresivo (PMF)** | Zona urbana o mixta, relieve moderado | Muy rápido; fácil de entender | Con pendientes fuertes confunde ladera con objeto; en bosque denso pierde suelo |
  | **Morfológico simple (SMRF)** | Terreno con pendiente y objetos de tamaños distintos; nubes grandes | Muy rápido (unas 3 veces más que PMF y 8 más que CSF en una hoja de 9 millones de puntos); la pendiente máxima del terreno es un parámetro con sentido físico | Con pocos retornos de suelo bajo el bosque denso rellena con el suelo más cercano; ventana cuadrada |
  | **Axelsson (TIN progresivo)** | Uso general; terrenos con pendiente y edificios | Muy contrastado; densifica el suelo respetando el relieve | Si hay estructuras mayores que la celda de las semillas hay que subirla; más parámetros |
  | **Clase 2 existente** | La entrega ya viene clasificada (por ejemplo, PNOA) | Es lo más rápido y no cambia nada | La calidad es la del proveedor; no se puede mejorar sin reclasificar |

  Cómo trabaja cada uno:
  - **Iterativo de Kraus y Pfeifer**: ajusta una superficie por mínimos cuadrados con pesos; los puntos muy por encima de ella pesan menos en cada vuelta, así que la superficie «baja» hasta el suelo. Después se aceptan los puntos que están a menos de una distancia de ella (Kraus y Pfeifer, 1998).
  - **Curvatura multiescala (MCC)**: compara cada punto con una superficie curva ajustada a varias escalas; el que se desvía menos que el umbral es suelo (Evans y Hudak, 2007).
  - **Simulación de tela (CSF)**: se da la vuelta a la nube y se deja caer una tela con rigidez ajustable; los puntos que quedan a menos de un umbral de la tela son suelo (Zhang et al., 2016). La caída por paso y la corrección de los muelles siguen las constantes publicadas por los autores. Contrastado en una hoja PNOA de 2023 y otra de 2010, con la clasificación del PNOA como referencia: recupera el 99,4 % del suelo y deja como suelo el 0,8 % (2023) y el 3,5 % (2010) de la vegetación alta.
  - **Morfológico progresivo (PMF)**: aperturas del mínimo con ventanas cada vez mayores; el punto que sube más que un umbral que crece con la ventana no es suelo (Zhang et al., 2003).
  - **Morfológico simple (SMRF)**: sobre la rejilla del punto más bajo de cada celda, aperturas con ventanas que crecen de una en una celda marcan como objeto lo que cae más de lo que permite la pendiente máxima (**pendiente**, 0,15); lo que no es objeto, rellenado donde falta, es una superficie provisional, y un punto es suelo si está a menos de un **umbral** (0,5 m) más un **factor** (1,25) por la pendiente de esa superficie (Pingel, Clarke y McBride, 2013). Detecta además los puntos bajos aislados. La **ventana máxima** (radio, 18 m) debe superar al edificio más grande. `--ground smrf` en la línea de comandos.
  - **Axelsson (TIN progresivo)**: se parte de una red triangular de semillas, el punto más bajo de cada celda grande (mayor que el edificio más grande, 20 m por defecto), y se van añadiendo los puntos que están cerca del plano de su triángulo (**distancia de iteración**, 1,4 m) y cuyas líneas a los tres vértices forman con el plano un ángulo pequeño (**ángulo de iteración**, 8°). Se rehace la red y se repite hasta que no entra ningún punto. Los puntos a menos de la **tolerancia llana** (10 cm) del plano son suelo sea cual sea el ángulo: cerca de un vértice el ángulo sale grande incluso para un punto sobre la superficie (Axelsson, 2000).

  **Edificios.** Se obtiene un terreno de referencia con una apertura morfológica mayor que cualquier edificio; las celdas que lo superan más que la altura mínima forman regiones, y una región es edificio si es lo bastante grande, plana (los tejados son planos, las copas rugosas) y está limitada por saltos bruscos. Los puntos de tejado son clase 6 y el resto de lo que supera esa altura, vegetación alta (5). Funciona bien con edificios de tejado plano o a dos aguas; los tejados con árboles encima o cubiertas muy irregulares pueden quedar como vegetación.

  **Vegetación por alturas.** Altura del punto sobre el suelo de su entorno: baja (3) por debajo del primer límite, media (4) hasta el segundo y alta (5) por encima. Es una clasificación geométrica, no botánica: un matorral alto cuenta como vegetación media o alta.

  **Cables (14) y postes o torres (15).** Un punto es cable si está a más de una altura mínima sobre el suelo y tiene vecinos alineados (linealidad = (λ₁ − λ₂)/λ₁ de la matriz de covarianza del entorno, mínimo 0,9) y de dirección casi horizontal. Además un cable está en el aire (pocos puntos a su alrededor) y es continuo: las cadenas de puntos de cable de menos de 4 m se descartan, porque las líneas de barrido hacen que un trozo de copa parezca recto durante uno o dos metros. Los postes y torres son estructuras verticales estrechas (poco anchas para su altura, de al menos 6 m), sin puntos alrededor que las unan a las copas, y que arrancan del suelo. Con la densidad del PNOA suele volver solo la **cabeza** del apoyo (una cruceta de unos metros, a más de 8 m, sin nada debajo): una cadena corta, recta y aislada a esa altura se toma por cabeza de apoyo y se clasifica como 15 (longitud máxima ajustable; 0 la desactiva). Se ha contrastado con tres apoyos marcados sobre ortofoto en una hoja PNOA: en uno volvió la cabeza y se clasifica; en los otros dos no volvió ni un solo punto, así que no hay nada que clasificar. Útil para limpiar el CHM de líneas eléctricas, que si no salen como árboles estrechos.

  **Reglas por eco y ángulo.** Cambian la clase de los puntos según el **eco** del pulso y el **ángulo de barrido**, al final y en orden (una regla ve lo que hizo la anterior). Cada regla tiene: las clases de partida (por defecto, todas menos el ruido 7 y 18), los ecos (único, primero de varios, intermedio, último), un intervalo de ángulo absoluto y la clase de destino. Dos usos habituales: pasar a una clase aparte (12, por ejemplo) los puntos de ángulo grande, que son menos exactos y conviene dejar fuera del suelo y de las superficies; y pasar a vegetación alta (5) los primeros ecos de pulsos con varios retornos que aún no tienen clase. Si una regla quita la clase 2 a un punto, ese punto deja de ser suelo también para el MDT. En la línea de comandos: `--rule "angle=25;to=12"` (se repite para varias; partes `to=`, `from=1,2` o `from=~7`, `returns=SFIL` o números, `angle=25..90`).

  **Teselas y rendimiento**: tamaño de tesela, búfer (franja alrededor de cada tesela, para que las decisiones en los bordes se tomen con los vecinos) e hilos.
- **Comparar nubes (cambios)**: escribe los puntos de la nube cargada que están a más de una distancia de todos los de otra nube de referencia. Da la distancia media, el RMS y la compleción. Los puntos de cambio aislados se descartan. **Cuándo usarla:** detectar cortas, cambios de obra o crecimiento entre dos vuelos. **Límite:** es una comparación punto a punto; con densidades muy distintas aparecen falsos cambios, por eso conviene adelgazar antes a la misma densidad.
- **Ajustar nubes**: mueve una nube para que encaje con la de referencia (punto más cercano iterativo, ICP; Besl y McKay, 1992), solo desplazando o desplazando y girando. Muestra el desplazamiento y el giro. **Cuándo usarla:** vuelos del mismo sitio con un desfase sistemático. **Límite:** necesita que ambas nubes se solapen y tengan estructuras fijas (edificios, terreno); sobre un bosque que ha cambiado puede ajustar mal.
- **Entre pasadas**: ráster con la diferencia de altura entre pasadas de vuelo (ID de fuente) y el porcentaje de celdas por encima de 8 y 16 cm. **Cuándo usarla:** como control de calidad de un vuelo, antes de modelar: pasadas desajustadas generan escalones en el MDT y en el CHM.
- **Colorear con imagen** (RGB): da a cada punto el color del píxel de una imagen del mismo sitio, normalmente una ortofoto (PNOA, por ejemplo), y el infrarrojo cercano si la nube es de formato 8 y la imagen tiene esa banda. Se eligen las bandas de rojo, verde y azul (1, 2 y 3 por defecto). Las nubes sin color (formatos 0, 1 y 6) se guardan en formato 7 (LAS 1.4: se conserva todo y la clase pasa a ocupar un byte entero); las de forma de onda (4 y 9) no se convierten. La imagen se lee una vez por nube y solo en su extensión; si esa ventana no cabe en memoria se usa una vista reducida de la propia imagen y el informe lo dice. Los puntos que la imagen no cubre quedan en negro, o con su color si marcas la casilla. Escribe `…_rgb`. **Cuándo usarla:** para ver la nube con su aspecto real, para distinguir copas de suelo o de tejados por el color, o para métricas espectrales por árbol. **Límite:** la imagen debe estar en el sistema de la nube; una ortofoto de otra fecha, o con las copas inclinadas por la perspectiva, da a algunos puntos el color de lo que tienen al lado. Una hoja PNOA de 13 millones de puntos se colorea en 1,5 s. En la línea de comandos: `easylidar colorize nube.laz --image orto.tif`.
- **Normalizar intensidad**: la energía que vuelve al escáner cae con el cuadrado de la distancia, así que la misma superficie sale más oscura en el borde de una pasada o desde una pasada más alta. Cada intensidad se multiplica por (R / R_ref)^a (Höfle y Pfeifer, 2007): *a* = 2 para superficies extensas (suelo, copas) y en torno a 3 para cables y bordes; R_ref es por defecto la distancia media de la nube, para que los valores conserven su tamaño habitual. La distancia R de cada punto sale de la **trayectoria** del vuelo (un archivo de texto con líneas «tiempo x y z», en el tiempo GPS y el sistema de la nube; es lo más exacto) o, sin ella, de la **altitud de vuelo** constante y el ángulo de escaneo: R = (altitud − z) / cos(ángulo). Los puntos sin distancia calculable (fuera de la trayectoria) conservan su intensidad, y el informe dice cuántos son. Escribe `…_intensidad`. **Cuándo usarla:** antes de usar la intensidad en clasificaciones, métricas o comparaciones entre pasadas y vuelos. **Límite:** no corrige el ángulo de incidencia sobre la superficie, la atmósfera ni las diferencias entre sensores. En la línea de comandos: `easylidar intensity nube.laz --trajectory tray.txt` o `--altitude 3200`.
- **Guardar como COPC**: reescribe la nube como **COPC** (Cloud Optimized Point Cloud), un LAZ 1.4 normal cuyos bloques son los nodos de un octree: cada nodo guarda un punto por celda de una malla de 128 × 128 × 128 sobre su cubo y pasa el resto a sus ocho hijos, hasta nodos de 100 000 puntos como mucho. Cualquier programa que lea LAZ lo lee entero; los que conocen COPC (QGIS, visores web, este mismo programa) leen solo la zona y el detalle que necesitan: recortar 4 ha de una hoja de 13 millones de puntos lee menos de un millón. Se conservan todos los atributos (comprobado punto a punto con lidR); los formatos 0 a 3 pasan a 6 o 7, que son los que admite COPC, y el sistema de coordenadas se escribe como WKT, que COPC exige (si la nube no lo declara, el informe avisa). La nube se ordena en memoria (unos 40 bytes por punto): las nubes de cientos de millones de puntos se dividen antes en teselas. Una hoja PNOA de 13 millones de puntos tarda 1,3 s. Escribe `<nombre>.copc.laz`. En la línea de comandos: `easylidar copc nube.laz`.
- **Exportar puntos**: saca los puntos a otro formato. Por defecto se escribe una **nube LAZ** sin normalizar; también se puede elegir LAS, **texto (ASCII)** con las columnas que se quieran, o **shapefile 3D** (PointZ). Se puede filtrar por clases y tomar uno de cada *n* puntos.
  - **Normalizar:** si se marca *Normalizar*, la herramienta pide el MDT y exporta **alturas sobre el suelo**: a la cota de cada punto se le resta la del MDT de **su propia nube** en ese lugar. Con varias nubes cada una usa su MDT (el último que se calculó para ella, salvo que se indique uno a mano). Los puntos que quedan fuera del MDT no se exportan y el mensaje final dice cuántos fueron.
  - **Cuándo es útil:** para trabajar con otros programas que esperan alturas normalizadas, o para compartir un conjunto ya listo para métricas o segmentación.
  - **Límite:** la normalización es tan buena como el MDT; un MDT con errores deja puntos de suelo con alturas de unos decímetros.
  - En la línea de comandos: `easylidar export nube.laz --normalize -o altura.laz` (con `--dtm mdt.tif` si no es el último MDT de esa nube; `.las`, `.laz`, `.shp` o texto según la extensión).

### 7.2 Superficies

Todas se escriben como GeoTIFF y se calculan **hoja por hoja**.

**Contorno de los rásteres.** El MDT, el MDS y el CHM se calculan sobre la caja que envuelve la nube, pero **no se rellenan las zonas donde no hay puntos**: los rásteres siguen el contorno real de la nube (una nube con forma de L, un vuelo girado o un hueco entre pasadas no se convierte en un rectángulo con valores inventados). Para ello se anota en una malla gruesa (la de unas 6 celdas, de 3 m como mínimo) dónde cae algún punto, se cierran los huecos pequeños (hasta unos 4 cuadros de esa malla, para que un claro o un lago no rompan el contorno) y lo que queda fuera se deja sin dato. Así los mosaicos de hojas contiguas encajan sin que una hoja pise a la vecina con extrapolaciones.

- **MDT (modelo digital del terreno)**, a partir de los puntos de suelo (clase 2 de la nube, o el método que elijas al clasificar). Hay tres formas de dar valor a cada celda; **por defecto, la triangulación**:
  - **Triangulación (TIN)** *(predeterminado)*: por teselas, se queda con el punto de suelo más cercano al centro de cada celda, se triangula (Delaunay) y se interpola linealmente dentro de cada triángulo. Es una superficie continua que respeta los puntos. No se interpola a través de triángulos con aristas de más de 50 m; esos huecos se rellenan al final con la media de las celdas vecinas. **Cuándo es mejor:** casi siempre; es la opción de referencia. **Problema:** cuando hay muy pocos puntos de suelo bajo copas densas, los triángulos son grandes y la superficie se alisa.
  - **Ponderado por distancia (IDW)**: cada celda toma la media de los 8 puntos de suelo más cercanos ponderada por 1/d² (Shepard, 1968). Antes de calcular, el suelo se reduce a un punto (la media) por cada media celda, así que el tiempo depende de la malla y no de la densidad. Da una superficie suave; es muy rápida. El alcance es de 30 celdas (entre 10 y 50 m). **Cuándo es mejor:** nubes muy densas o con ruido, donde se quiere un suelo alisado. **Problema:** suaviza las rupturas de pendiente (taludes, cunetas) y no pasa por los puntos.
  - **Media por celda**: la media de las elevaciones de los puntos de suelo de cada celda, en una sola pasada y sin cargar la nube. Es la más rápida y la que menos memoria usa, pero deja huecos donde no hay puntos (se rellenan al final). **Cuándo es mejor:** comprobaciones rápidas o nubes inmensas en un equipo justo de memoria.
  - **Líneas de rotura** (con TIN o IDW): un shapefile de líneas o polígonos que el MDT debe seguir en lugar de redondear: bordes de camino, taludes, cunetas, orillas. Cada línea se densifica a vértices separados media celda que entran en la triangulación con los puntos de suelo, y los puntos de suelo pegados a la línea se apartan, así que los triángulos siguen la línea en vez de cruzarla. Las líneas **con cotas** (PolylineZ, PolygonZ) fijan la altura: son líneas duras, como las de un levantamiento topográfico. Las líneas **sin cotas** (un cauce o una cuneta dibujados sobre la ortofoto) toman en cada vértice el punto de suelo más bajo a menos de una celda, lo que hunde el cauce en el modelo; donde bajo las copas no hay suelo tan cerca, ese punto puede quedar algo más alto que la superficie que daba la triangulación sola. Contrastado en una hoja PNOA: una línea 3D deja el MDT exactamente a su altura y no cambia ninguna celda a más de 3 m de ella. En la línea de comandos: `easylidar dtm nube.laz --breaklines lineas.shp`.
- **MDS (modelo digital de superficie)**: lo más alto que ve el sensor (copas, tejados). Se calcula con los puntos que no son ruido, por defecto con la **triangulación**:
  - **Triangulación (TIN)** *(predeterminado)*: se toma el punto más alto de cada celda, se triangula y se interpola; sin espigas ni huecos pequeños.
  - **Máximo por celda**: la cota del punto más alto de cada celda, en una sola pasada; las celdas vacías se rellenan (casilla *Rellenar*). Es lo más rápido; deja «pozos» en copas con huecos.
  - **Ponderado por distancia (IDW)**: sobre los puntos más altos de cada media celda.
- **CHM (altura de copas)**: por celda, la altura del punto más alto sobre el terreno, `z − MDT` (las alturas negativas valen 0). **Cuándo usarlo:** es la entrada de la segmentación de árboles y de muchos inventarios. Usa el MDT **de esa misma nube**. Con **pit-free** se apilan varias triangulaciones de los puntos por encima de umbrales de altura (0, 2, 5, 10, 15 m…) y se toma el máximo de capas (Khosravipour et al., 2014). **Ventaja:** quita los «pozos» que deja el máximo por celda en copas con huecos, lo que reduce las cimas falsas al segmentar. **Problema:** más lento y exige MDT.
- **MDS / CHM en una sola herramienta:** el botón *MDS / CHM* ofrece los mismos métodos (TIN, máximo por celda, IDW y, con MDT, pit-free). Si marcas *Restar el MDT* y todas las nubes tienen MDT, sale el CHM (`chm_…`); si no, sale el MDS (`mds_…`). El pit-free exige MDT porque trabaja con alturas sobre el suelo.
- **Suavizado** (MDT, MDS y CHM): ventana circular de media, mediana o gauss, con el tamaño en metros. La mediana conserva mejor los bordes de copa; el gauss da una superficie más suave; la media es la más simple. Suavizar el CHM antes de segmentar reduce las cimas falsas, a costa de fundir copas pequeñas vecinas.

Recomendación: celda de 1 m para el MDT en terreno forestal con 1 a 10 puntos/m² (PNOA); 0,5 m para el MDS y el CHM. Si hay pocos puntos de suelo bajo copas densas, una celda mayor da un terreno más fiable.

#### Análisis de terreno

El botón *Análisis de terreno* calcula productos derivados de un MDT (o de cualquier ráster de alturas): se elige el MDT (un archivo o una capa ya cargada), el producto y la salida, que es un GeoTIFF del mismo tamaño y con el mismo sistema de coordenadas. El ráster debe estar en un sistema métrico (UTM); en grados el programa se niega a calcular. Las celdas sin dato siguen sin dato, y en los bordes y junto a huecos la ventana toma el valor de la propia celda (como `gdaldem`). Si las alturas no están en las unidades de la celda, el *factor Z* las corrige.

| Grupo | Producto | Qué es y cuándo sirve |
|---|---|---|
| Relieve | **Pendiente** | Inclinación en grados o porcentaje (Horn, 1981; ventana de 3×3). Base de casi todo lo demás: accesos, erosión, tipos de estación |
| | **Orientación** | Hacia dónde mira la ladera, en grados desde el norte; las celdas llanas quedan vacías. Solana y umbría |
| | **Sombreado** | Iluminación 0–255 con la dirección y la altura de la luz que se elijan (315° y 45° por defecto). Sirve para **ver** el MDT y detectar defectos |
| Forma | **Curvatura en planta, de perfil y total** | Zevenbergen y Thorne (1987), en 1/m; positiva en cumbres, lomas y espolones, negativa en vaguadas. La de planta indica si el agua converge o diverge; la de perfil, si acelera o frena |
| | **Rugosidad** | Diferencia entre la celda más alta y la más baja de la ventana de 3×3 |
| | **Índice de rugosidad (TRI)** | Media de las diferencias con las ocho vecinas (Wilson et al., 2007) |
| | **Posición topográfica (TPI)** | Altura menos la media del círculo de radio dado (Weiss, 2001): positivo en lomas, negativo en vaguadas. Se calcula con sumas acumuladas por fila, así que un radio grande apenas cuesta más. El resultado depende del radio: pruébalo a varias escalas |
| Hidrología | **Profundidad de depresiones** | Cuánto hay que rellenar cada depresión para que el agua salga del modelo (Priority-Flood, Barnes et al., 2014); sirve también para detectar defectos del MDT |
| | **Acumulación de flujo** | Superficie (m²) que vierte a cada celda, con direcciones D8 (O'Callaghan y Mark, 1984) sobre el modelo ya rellenado |
| | **Índice de humedad (TWI)** | `ln(a / tan β)` (Beven y Kirkby, 1979): alto donde el agua se acumula y la pendiente es baja |
| | **Red de drenaje** | Celdas por las que vierte al menos la superficie mínima indicada (en hectáreas) |
| | **Subcuencas** | Cuenca de cada tramo de la red (de una cabecera o confluencia a la siguiente confluencia), numeradas desde 1 |
| Visibilidad | **Cuenca visual** | Qué celdas ve un observador (X, Y del mapa, altura de los ojos, altura que debe verse y alcance): 1 las ve, 0 no, vacío fuera del alcance. No aplica la curvatura terrestre ni la refracción (despreciables a pocos kilómetros) |

**Ventajas y límites.** Todos los productos heredan los defectos del MDT: un MDT con terrazas, escalones entre hojas o ruido produce curvaturas y pendientes falsas, y la red de drenaje sale entrecortada. D8 reparte todo el flujo de una celda a una sola vecina, lo que da redes limpias pero no representa bien la dispersión en laderas llanas. La mayoría de estos índices dependen de la resolución: compara siempre con la misma celda.

Todo se calcula con el ráster completo en memoria (unos 20 bytes por celda las derivadas y 35 la hidrología; un MDT de 25 millones de celdas tarda de 2 a 7 s), por filas en paralelo. Por la línea de comandos: `easylidar terrain mdt.tif --product slope`, y `--help` lista los productos y sus parámetros.

#### Volumen sobre MDT

Calcula el volumen entre una superficie y el terreno, celda a celda: V = Σ (altura de la superficie − MDT) · celda². Las celdas por encima del terreno suman al **volumen sobre él** y las que quedan por debajo, al **volumen bajo él** (vaciados). Cada nube es una hoja y busca sus rásteres en su carpeta de resultados (o los eliges a mano).

- **CHM de cada nube:** las alturas ya son sobre el suelo, así que no hace falta MDT. Es el volumen de la vegetación o de lo que sobresale del terreno.
- **MDS menos MDT:** superficie y terreno de la nube. El MDT se muestrea en el centro de cada celda del MDS, y las celdas que el MDT no cubre no se miden (se cuentan aparte).
- **MDS sobre una cota horizontal:** volumen por encima de un plano a la elevación indicada (acopios, por ejemplo).
- **Altura mínima que cuenta:** las celdas más cerca del terreno que esta altura no suman. Quita el ruido de la superficie.
- **Zonas:** todo el ráster, las AOI dibujadas (un volumen por AOI) o los polígonos de un archivo vectorial. Si las zonas se solapan, se miden una a una.

**Volumen de copas segmentadas.** Elige como zonas el shapefile de copas que da *Segmentar árboles* (`…_copas.shp`) y como superficie el **CHM** (no el ráster de copas, que guarda los números de árbol y no alturas). La columna `Region` de la tabla lleva el **`TREEID`** de cada copa, el mismo número que la columna `id` de la tabla de árboles y que el campo `TREEID` del shapefile de copas, así que el volumen de cada árbol se une a su altura, su área de copa y su posición por ese número.

**Resultado:** un único `…_volumen.csv` (con una columna `Hoja` si hay varias nubes) con, por zona, `Area_m2`, `Vol_sobre_m3`, `Vol_bajo_m3`, `Vol_neto_m3`, `H_media_m` y `H_max_m` (de las celdas sobre el terreno) y `Celdas_sin_MDT`. La precisión es la del ráster: usa la misma celda para el MDS y el MDT cuando puedas. El volumen de una copa por este método es el volumen **envolvente** bajo la superficie del CHM, no el de madera.

### 7.3 Inventario forestal

#### Métricas de rodal

Cuadricula la nube en celdas (15 m por defecto; la rejilla está alineada a múltiplos del tamaño de celda, de modo que las celdas de hojas vecinas encajan). En cada celda calcula estadísticos de las **alturas sobre el suelo** de todos los puntos que no son ruido: `altura = z − MDT` (el MDT de esa misma nube). Las celdas con menos puntos que el mínimo (4) no dan métricas. Se añade la fecha de captura (`Date` en las tablas de texto, `captura` en las capas).

**Cuándo usarlo, ventajas y límites.** Son las variables explicativas del enfoque por área (*area-based approach*, Næsset, 2002): percentiles de altura, cobertura de copas y momentos, con las que se modelan volumen, área basimétrica, biomasa o altura dominante a partir de parcelas de campo (White et al., 2013). **Ventajas:** son robustas porque no dependen de detectar árboles, y se calculan rápido para territorios enormes. Son las mismas métricas que da FUSION (McGaughey) y coinciden con sus columnas, y los momentos L siguen a Hosking (1990). **Límites:** dependen de la densidad de puntos y del tamaño de la celda (la celda debe ser del tamaño de la parcela de campo), de la fecha del vuelo y de un buen MDT; en celdas con agua, edificios o pocos puntos no son fiables.

Opciones: *límite de altura* de cobertura (2 m), *intensidad* (los mismos estadísticos de la intensidad), *estratos* (estadísticos por franjas de altura: por defecto hasta 0,15, 1,37, 5, 10, 20 y 30 m) y la *memoria* máxima (se procesa por bandas de filas que caben en ella: la nube se lee una vez por banda).

**Qué es cada métrica.** `h` son las alturas de los puntos de la celda; `n`, su número.

| Métrica | Cómo se calcula |
|---|---|
| Total return count | n, número de puntos de la celda |
| Elev minimum, maximum, mean | Mínimo, máximo y media de h |
| Elev mode | Moda de h, estimada con un histograma |
| Elev stddev, variance, CV | Desviación típica, varianza y coeficiente de variación (desviación / media) |
| Elev IQ | Rango intercuartílico: P75 − P25 |
| Elev skewness, kurtosis | Asimetría y curtosis (momentos de tercer y cuarto orden, como en FUSION: sobre la desviación típica muestral; la curtosis no resta 3). En celdas casi sin vegetación las alturas se concentran a ras de suelo y estas dos métricas dependen mucho de unos pocos puntos y de cómo se normalizó: un par de centímetros de diferencia en el MDT las cambia mucho. Úsalas con cautela en modelos de zonas abiertas |
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

**Métricas de estructura.** Con la casilla *Métricas de estructura* (o `--structure`) se escribe, junto a las demás, un archivo `…_estructura` con, por celda: la **fracción de huecos** (proporción de retornos por debajo de una altura, 1 m por defecto), el **índice de área foliar** (LAI = −ln(fracción de huecos) / k, con k = 0,5 por defecto: ley de Beer–Lambert, Monsi y Saeki, 1953), la **diversidad de alturas del follaje** (FHD: entropía de Shannon de la proporción de retornos en cada capa de 1 m; MacArthur y MacArthur, 1961), el **índice de complejidad vertical** (VCI: esa entropía dividida por su máximo, ln del número de capas; van Ewijk et al., 2011) y la **altura de la base de la copa** (CBH: el borde inferior de la primera capa, por encima de 2 m, que tiene al menos el 10 % de los retornos de la capa más llena). Se calculan con las mismas alturas que ya se guardan para las demás métricas: no hay pasada extra y, sin la casilla, no cuestan nada. **Límites:** el LAI sale de la proporción de retornos y no de una medida directa de hojas, así que es un índice relativo (útil para comparar, no un valor absoluto); con pocos retornos por celda las métricas de perfil son ruidosas (usa celdas de 15–20 m o más); la CBH depende del suelo de 2 m elegido y del 10 % (cambian en la ventana) y no sustituye una medida de campo. La densidad aparente de copa (CBD) y la rugosidad de superficie (*rumple*) no están: necesitan coeficientes del usuario o un modelo de superficie aparte. Las filas de las métricas ya se calculan **en paralelo** por filas de celdas: el resultado es idéntico al de antes (se ha comprobado byte a byte) y tarda menos con varios núcleos.

#### Segmentar árboles

Detecta árboles individuales sobre el **CHM**: sus cimas, su copa y su altura. Con varias nubes cargadas segmenta el CHM de cada una y da un resultado por hoja (si eliges un CHM a mano, solo ese). Los pasos son:

1. **Suavizado** opcional del CHM (media, mediana o gauss).
2. **Cimas**: máximos locales en una ventana (fija, o variable según la altura: `a + b · h + c · h²`; circular o cuadrada) por encima de una altura mínima (Popescu y Wynne, 2004).
3. **Copas**, a elegir entre tres métodos.
4. **Salidas**: tabla de árboles (`id`, posición, altura, área y diámetro de copa), puntos de las cimas, polígonos de copa, ráster de identificadores y, si se pide, el `treeID` de cada punto de la nube.
5. **Evaluación con inventario de campo**: empareja cada árbol medido con la cima más cercana (dentro de una distancia dada) y da aciertos, omisiones, comisiones, exhaustividad, precisión, la medida F y el error de altura (Eysn et al., 2015, plantean este tipo de contraste).

**Los tres métodos de copa**

| Método | Cómo funciona | Cuándo es más útil | Ventajas | Problemas |
|---|---|---|---|---|
| **Dalponte y Coomes** | Crece una región desde cada cima añadiendo celdas vecinas que no sean demasiado más bajas que la cima ni que la media de la copa (umbrales *semilla* y *copa*) y hasta un alcance máximo | Masas con copas bien diferenciadas (coníferas, plantaciones, encinar abierto) | Copas de forma natural y compactas; se controla bien con el alcance máximo | Dos parámetros de umbral que dependen de la especie; en copas muy irregulares deja celdas sin asignar |
| **Silva** | Cada celda pasa a la cima más cercana si está dentro de un radio proporcional a la altura de la cima y por encima de una fracción de ella | Dehesas y masas abiertas, o cuando se quieren copas casi circulares | Muy simple y estable; sin crecimiento iterativo | Copas más geométricas que reales; recorta copas grandes o alargadas |
| **Cuencas (watershed)** | Inunda el CHM invertido desde las cimas y traza la línea de reparto entre copas vecinas (Vincent y Soille, 1991; aplicado a copas por Chen et al., 2006) | Copas que se tocan en masas cerradas, donde la línea de unión importa | Reparte bien el espacio entre copas vecinas | Sensible al ruido del CHM: sin suavizar produce muchas copas pequeñas (sobresegmentación) |

**Dalponte y Silva se han contrastado con el paquete lidR 4.3** (Roussel et al., 2020), usado como caja negra: las cimas coinciden exactamente, y las copas coinciden en el 97 % (Dalponte) y el 99 % (Silva) de las celdas; las diferencias están en píxeles disputados entre dos árboles. No se usó código de lidR. Referencias de los métodos: Dalponte y Coomes (2016) y Silva et al. (2016).

**Cuándo da buenos resultados.** Con un CHM limpio (mejor *pit-free* que el máximo por celda), una celda de 0,5 m y una ventana de cimas ajustada al tamaño de las copas. Con inventario de campo se puede calibrar la ventana: en una zona de prueba, una ventana variable `3 + 0,1·h` acertó el 86 % de los árboles con una precisión del 76 %.

**Problemas y límites que conviene conocer**
- **Es una segmentación del CHM**, no de los puntos: ve solo lo que asoma por arriba. Los árboles dominados bajo otros no aparecen, y en masas cerradas de frondosas varias copas se funden en una.
- **Las copas con varias cimas se parten en dos** si la ventana es pequeña y **dos árboles pequeños se funden en uno** si es grande. No hay una ventana buena para todo: se prueba en una parcela con árboles contados.
- **Matorral alto y vegetación arbustiva** se detectan como árboles bajos: sube la altura mínima (`hmin`) o filtra después por altura y por área de copa.
- El CHM ha de proceder de un MDT correcto: un error de suelo de decímetros desplaza todas las alturas.
- Las alturas son las del CHM (máximo de la celda) y por tanto algo menores que la altura real del árbol, sobre todo con celdas gruesas.

**Enlazar con los volúmenes.** Los árboles, las cimas y las copas comparten el número `id` (campo `TREEID` en los shapefiles). Para el volumen de cada copa usa el shapefile de copas como zonas de *Volumen sobre MDT* (ver 7.2): la tabla de volúmenes lleva ese mismo `TREEID`, y se une a la de árboles por él. Un árbol sin polígono de copa (por tener menos de tres celdas) no aparece en el shapefile de copas pero sí en la tabla de árboles.

#### OBIA: análisis por objetos

Agrupa las celdas de varias capas en **objetos** (rodales, copas, tejados, manchas de matorral), los describe con atributos y, si quieres, les da una clase. Es el análisis de imagen orientado a objetos (OBIA) aplicado a productos LiDAR: en lugar de clasificar cada celda por separado, se clasifican objetos homogéneos, con su tamaño, su forma y las estadísticas de cada capa.

1. **Capas.** Un CHM (obligatorio, el primero) y, si quieres, más rásteres que coincidan con él celda a celda (mismo origen, tamaño de celda y dimensiones; si no coinciden, la herramienta lo dice y se igualan antes con *Ráster → Igualar rásteres*). De la nube cargada pueden salir además, en una sola lectura, la **intensidad media**, los **retornos por pulso** (alto bajo la copa, 1 en tejados y suelo desnudo), la **proporción de suelo** y la **densidad de puntos**. La **rugosidad** del CHM (desviación típica en una ventana) distingue una copa, cuya altura varía de celda a celda, de un tejado o un prado.
2. **Objetos**, con dos métodos:
   - **Fusión de regiones** (Baatz y Schäpe, 2000). Parte de manchas de celdas casi iguales y fusiona, ronda a ronda, cada par de vecinas que son la mejor opción una de la otra mientras el coste de fusionarlas (cuánto crece la heterogeneidad de valores y, si se pide, de forma) sea menor que la *escala* al cuadrado. Las capas se estandarizan, así que la escala no depende de las unidades. **Escala**: mayor, objetos mayores (prueba entre 5 y 15). **Peso de la forma** (0 a 0,9) y **compacidad**: con valores altos los objetos son más redondeados y menos fieles al valor. Cada capa puede tener su peso (0 la deja fuera). El resultado no depende del número de hilos.
   - **Rango de valores**: las celdas conexas cuyo CHM está entre dos valores (por ejemplo, todo lo que supera 2 m forma masas de arbolado).
   - El **objeto mínimo** (en celdas) se fusiona con su vecino más parecido (fusión) o se descarta (rango).
3. **Atributos** de cada objeto: `area`, `perimetro`, `compacidad` (4π·área/perímetro²: un cuadrado vale 0,79), `alargamiento` (0 redondo, cerca de 1 una línea), el centroide `x`/`y`, y para cada capa su `_media`, `_desv`, `_min` y `_max`.
4. **Clasificación**, a elegir:
   - **Reglas** en un archivo de texto, una por línea: `arbolado: chm_media >= 5 and area < 400`. Vale la primera regla que cumple un objeto; un objeto con un valor vacío no cumple condiciones sobre él.
   - **Agrupar por parecido** (sin reglas ni muestras): indica cuántos estratos quieres y los objetos se agrupan con k-means (Lloyd, 1982; arranque de Arthur y Vassilvitskii, 2007) a partir de la media de cada capa, el percentil 95 de la altura y la cobertura de copas, estandarizados y con más peso para los objetos grandes. El estrato 1 es el más bajo. Con una semilla fija el resultado se repite.
   - **Muestras**: un shapefile de polígonos con un campo que nombra la clase (créalo con *Vectorial → Nueva capa* (de áreas)). Un **bosque aleatorio** (Breiman, 2001; 100 árboles por defecto) aprende de los objetos que las muestras cubren (al menos la mitad de sus celdas). La exactitud se mide con los objetos que cada árbol no vio durante el entrenamiento: se da la exactitud global, el índice kappa (Congalton, 1991), la precisión del usuario y del productor de cada clase y los atributos que más pesan. Con una semilla fija el resultado se repite.
5. **Estratos homogéneos** (opcional, necesita una clasificación): los objetos vecinos de la misma clase se unen en un solo polígono, y las áreas menores que el *estrato mínimo* toman la clase del vecino con el que más lindan (las que no tienen vecino grande se quedan como están). Se obtiene el polígono de cada estrato, un ráster de estratos y una tabla resumen con, por clase, el número de áreas, la superficie en hectáreas, la media y la desviación del CHM y el **coeficiente de variación** (cuanto menor, más homogéneo). Si indicas un número de **parcelas**, se reparten entre los estratos por asignación de Neyman (1934): más parcelas a los estratos grandes y variables, con un mínimo de dos por estrato cuando alcanzan. La **cobertura** (`cobertura`, en %) es la proporción de celdas del objeto cuyo CHM llega a la altura indicada (2 m por defecto); también se calculan `<capa>_p50` y `<capa>_p95` del CHM.
6. **Salidas**: ráster de números de objeto (`…_objetos.tif`, entero de 32 bits), polígonos con sus atributos y, si hay clase, la clase y la confianza (la proporción de árboles que votan por ella), ráster de clases (`…_clases.tif`) y/o tabla CSV. Los polígonos son el contorno exacto de las celdas, con huecos y sin superposiciones; el área del polígono es la del objeto. *Simplificar* aligera los contornos con Douglas-Peucker, a costa de pequeños huecos entre vecinos.

**Rendimiento.** Todas las fases son lineales en el número de celdas. En un equipo de pruebas, una hoja PNOA de 1 km² (CHM de 1 m, 11 millones de puntos, cuatro capas, clasificada por reglas) se procesó en menos de un segundo, y la segmentación de 20 millones de celdas sintéticas con ruido, en unos 2 s. La memoria crece con las celdas y con el número de manchas iniciales (un CHM ruidoso da más); en rejillas de cientos de millones de celdas conviene empezar con una zona.

**Problemas y límites que conviene conocer**
- **La escala no se puede dar «a ojo»**: depende del tamaño de celda y de la variabilidad de las capas. Prueba en una zona pequeña y comprueba que el límite de los objetos sigue lo que ves.
- **Un CHM ruidoso** (máximo por celda con pocos puntos) da muchas manchas pequeñas y objetos con bordes irregulares; suaviza o usa el CHM *pit-free* antes.
- **El coeficiente de variación** no sirve con medias cercanas a cero (un estrato sin arbolado sale con un valor enorme); en esos estratos mira la desviación. La asignación de Neyman usa la variabilidad del CHM; si lo que vas a estimar es otra variable (volumen, área basimétrica), conviene repartir con la variabilidad de esa variable en una prueba piloto (Cochran, 1977).
- **La segmentación se hace con toda la rejilla en memoria**; no hay aún proceso por teselas.
- **Las muestras desbalanceadas** (pocos objetos de una clase) bajan la exactitud de esa clase; dibuja un número parecido de muestras por clase y de sitios distintos.
- **Las unidades**: el área va en unidades de mapa al cuadrado y la escala en celdas por desviaciones típicas, no en metros.

#### Cambio forestal

Compara los modelos de alturas (CHM) de **dos fechas** que coinciden celda a celda (si no, iguálalos antes con *Ráster → Igualar rásteres*). Da:

- La **diferencia de alturas** (GeoTIFF) y un ráster de **clases** por celda: 1 estable, 2 crece, 3 baja, 4 pierde la copa (era copa y deja de serlo) y 5 gana copa. Un cambio de altura menor que el **cambio mínimo** (1 m por defecto) se considera ruido; la copa empieza a una altura que se elige (2 m por defecto).
- Las **perturbaciones**: manchas conexas de pérdida de copa de al menos un tamaño mínimo (10 celdas por defecto), como polígonos (con sus alturas antes y después) y/o tabla.
- El **resumen por rodal** (si das un archivo de polígonos, shapefile o GeoJSON): superficie, cobertura de copa en cada fecha, cambio medio de altura y hectáreas que crecen, bajan, pierden o ganan copa.
- El **crecimiento de cada árbol** (opción): se detectan las cimas en las dos fechas con la ventana variable de la segmentación de árboles y se emparejan por distancia (el par más próximo primero, hasta una distancia máxima); da la tabla de crecimiento (por año si indicas los años) y cuenta los árboles perdidos y los nuevos.

Si indicas los **años** entre fechas, los resultados incluyen la tasa anual. **Límites que conviene conocer:** la diferencia de dos modelos solo vale lo que valga su acuerdo vertical, y un MDT distinto o con error de decímetros entre fechas desplaza todas las celdas; el informe mide ese acuerdo (la mediana de la diferencia donde las dos fechas son suelo) y avisa si es comparable al cambio mínimo. Con solo LiDAR **no se distingue** una corta de un incendio o un viento: se mide la pérdida de altura y de copa, no su causa. El emparejado de árboles falla donde las copas se juntan (varias cimas por copa) o donde cambia la ventana de cimas entre fechas.

#### Modelos de transición

Relaciona una variable medida en campo (volumen, área basimétrica, biomasa, número de pies…) con las métricas LiDAR de las mismas parcelas, y con ese modelo dibuja el mapa de la variable. Es el paso 4 y 6 del enfoque por área (capítulo 13) hecho dentro del programa.

**Qué hace, y qué necesita.**

- **Métricas de las parcelas:** la capa de parcelas que da *Recortar parcelas* (modo solo métricas, formato shapefile): un polígono por parcela con sus métricas. Puede ser una capa ya cargada en el programa (*De las capas cargadas*) o un archivo; también vale la tabla de texto (.csv) de esa misma herramienta.
- **Datos de campo:** un Excel (.xlsx), .csv o .txt con **una fila por parcela** y una columna con el mismo nombre de parcela que la tabla de métricas (`1`, `1.0` y `001` cuentan como la misma). Si en lugar de eso tienes la lista de árboles, súmala o promedia antes por parcela: la herramienta se niega si una parcela sale en varias filas.
- **Variable a estimar:** una columna de la tabla de campo.
- **Mapa (opcional):** las tablas de texto de *Métricas de rodal* (una por hoja), calculadas con las mismas opciones que las parcelas.

**Análisis previo y variables del modelo.** Hay dos maneras de dar las variables, que se eligen en la ventana:

- **Automático.** El programa deja de lado las métricas que repiten a otra (correlación mayor que un límite, 0,8 por defecto; p. ej. los percentiles altos de altura entre sí: se queda la más relacionada con la variable) y elige entre las demás por BIC o AICc, con una variable por cada diez parcelas como mucho. Esa poda y la elección se repiten en cada vuelta de la validación cruzada. Meter todas las métricas sin podar da modelos inestables.
- **Elegir variables.** El botón abre una tabla con todas las métricas (correlación de Pearson y de Spearman con la variable, normalidad y asimetría de cada una, y cuáles repiten a otra) donde marcas las del modelo. *Sugeridas* marca las mejor relacionadas que no se repiten. El modelo usa exactamente las marcadas; si dos están muy correlacionadas, la herramienta lo dice y se niega a ajustar.

En los dos casos el informe empieza con el **análisis previo**: normalidad de la variable a estimar (Shapiro–Wilk, con la asimetría y la curtosis), también de su logaritmo, con la recomendación de la transformación logarítmica si ésta la arregla. Una regresión lineal exige residuos normales, no variables normales: la normalidad de la variable es el primer aviso y la de los residuos se comprueba tras el ajuste. En la línea de comandos, `--analyze` da solo este análisis y `--candidates` restringe las candidatas del modo automático.

**El modelo.** Una regresión lineal múltiple por mínimos cuadrados. Puedes dar tú las métricas (la opción con más criterio forestal: p. ej. un percentil alto de altura y una medida de cobertura) o dejar que se **elijan automáticamente** por selección progresiva con BIC (el criterio más estricto contra el sobreajuste) o AICc, con un máximo de variables (por defecto una por cada diez parcelas, entre dos y cinco) y un límite de colinealidad (VIF ≤ 5). Las métricas absolutas (recuentos de puntos y de retornos) no se ofrecen como candidatas porque dependen del tamaño de la parcela y un modelo ajustado con parcelas no valdría para celdas de otro tamaño; las de **intensidad** tampoco, porque en casi todos los vuelos no está calibrada entre pasadas (se pueden pedir expresamente). Si pides el mapa, solo se ofrecen las métricas que están también en la tabla de las celdas. La **transformación** puede ser ninguna, el logaritmo de la variable (cuando el error crece con el tamaño del rodal) o el logaritmo de la variable y de las métricas (relación potencial); con logaritmos se deshace la transformación con la corrección de sesgo exp(s²/2).

**Cómo se juzga: lo importante.** Un modelo siempre se ve bien con las parcelas con las que se ajustó, sobre todo si sus variables se eligieron entre decenas de candidatas con pocas parcelas. Por eso el informe pone en primer plano:

- **La validación cruzada dejando una parcela fuera, repitiendo la elección de métricas en cada prueba.** Cada parcela se estima con un modelo que ni la vio ni la usó para escoger sus variables; el RMSE, el R², el sesgo y el error absoluto medio de esa validación son lo que cabe esperar en parcelas nuevas. Se compara con el ajuste (optimista) y, si la elección es automática, con la validación del modelo final con sus variables ya fijadas: la diferencia entre las dos es el precio de haber elegido las variables con los mismos datos.
- **La estabilidad de la elección.** Qué porcentaje de las pruebas eligió cada métrica y en cuántas salió exactamente el modelo final. Si el modelo cambia a cada parcela que se quita, la elección no es fiable aunque el R² sea alto.
- **Los coeficientes** con su error típico, t, p y VIF (cuánto se infla la varianza por la relación con las demás métricas; más de 5 es señal de colinealidad), y el test F del modelo. En una elección automática los p son optimistas.
- **Las parcelas**: palanca (cuánto se aparta su combinación de métricas del conjunto), residuo estudentizado y distancia de Cook (cuánto cambiaría el modelo sin ella). El informe nombra las que mandan sobre el modelo y la tabla `…_parcelas.csv` da, para cada una, el observado, el ajustado, el estimado por validación cruzada y un aviso (`atipica`, `palanca`, `influyente`): revisa su dato de campo y su posición antes de quitarla.
- **Los tres supuestos de la regresión lineal**, contrastados sobre los residuos y resumidos en una línea ("se cumplen los tres" o cuáles no): **normalidad de los residuos** (Shapiro–Wilk), **linealidad** (RESET de Ramsey: si añadir al modelo los cuadrados y cubos de sus valores ajustados lo mejora, la relación no es lineal) y **varianza constante** (Breusch–Pagan). Si alguno falla, los errores típicos, los valores p y los intervalos de predicción no son fiables; el informe sugiere el logaritmo cuando puede ayudar.
- **Avisos** en lenguaje llano cuando algo no cuadra: pocas parcelas, menos de diez por variable, validación muy por debajo del ajuste, sesgo, colinealidad o parcelas influyentes.

**El mapa.** Aplica el modelo a cada celda. Las celdas con alguna métrica **fuera del rango de las parcelas** quedan vacías (el modelo no ha visto nada parecido; se puede desactivar), y el informe dice qué porcentaje y por qué métrica. Una variable que no puede ser negativa se deja en 0 si el modelo da un valor negativo. Opcionalmente se guarda el **intervalo de predicción del 95 %** (semiancho, en las unidades de la variable) en otro ráster. La suma que da el informe es una estimación con el error de arriba, no un inventario.

**Cuándo usarlo:** después de *Recortar parcelas* y *Métricas de rodal*, con unas 30 o más parcelas medidas en campo. **Límites:** con 20 parcelas o menos la validación cruzada es muy variable y la elección automática casi siempre sobreajusta; da mejor resultado elegir pocas métricas con sentido forestal. El modelo solo vale para bosques como los de las parcelas, con el mismo tamaño de parcela y celda, la misma densidad de puntos, y fechas próximas entre el campo y el vuelo. Una posición de parcela con errores de GPS de varios metros limita lo que cualquier modelo puede explicar.

#### Corredores eléctricos

Para líneas eléctricas en nubes con los **cables (clase 14) y los postes (clase 15) ya clasificados** (Clasificar → *Cables y postes*). Reconstruye cada conductor con los puntos de cable que se tocan (a menos de 2,5 m), a partir de la altura y el desplazamiento medianos cada 2 m a lo largo de su dirección; los huecos de hasta 30 m sin puntos se rellenan con la parábola ajustada a los puntos de los lados, que sigue la catenaria en lugar de cortarla en línea recta. Los postes son los grupos de puntos de clase 15 de al menos 6 m de alto. Los **vanos** de un conductor son los tramos entre los postes que están a su lado (hasta 8 m).

Después recorre la vegetación (clases 3, 4 y 5) a menos de 30 m de un conductor, mide su altura sobre el MDT y agrupa los puntos en celdas de 3 m. Una celda es un **riesgo por invasión** si algún punto está a menos de la distancia de seguridad (3 m por defecto) del conductor, y un **riesgo por caída** si su altura más un margen (1 m) alcanza la posición horizontal del conductor. Salen los conductores (polilíneas 3D), los postes, los puntos de riesgo y dos tablas: el **informe por vano** (longitud, flecha, celdas en riesgo, distancia mínima y altura máxima de lo que está en riesgo) y la lista de riesgos. **La distancia de seguridad depende de la tensión y de la normativa que se aplique: ajústala a tu caso.**

**Fecha de captura en los polígonos.** Todo polígono que sale de un proceso (celdas de métricas, parcelas, objetos, cambios…) lleva una sola columna de fecha, `captura`: el día en que se capturaron los datos LiDAR, sacado de la información de los archivos (el tiempo GPS de los puntos). Si los datos son de varios días salen todos (`2017-09-08|2017-10-07`); si el archivo no la trae, el campo queda **en blanco**. Nunca es la fecha en que se creó la capa. Las tablas de texto la llevan como `Date`, que vale `NA` cuando no se conoce. Las capas que dibujas tú no la llevan.

**Borde de los polígonos.** En la pestaña de simbología de una capa de polígonos, *Grosor* 0 quita el borde y deja solo el relleno.

**Límites y cautelas.** El resultado vale lo que valga la clasificación de cables y postes: los clasificadores automáticos confunden las copas de los árboles con cables, y en nubes de baja densidad (PNOA) los cables apenas tienen puntos. Por eso se descartan los conductores de menos de 25 m y los postes de menos de 6 m o más anchos de 8 m, y la herramienta se niega (y lo dice) si todo lo que hay es ruido de ese tipo. Se ha comprobado con una línea sintética (conductores, huecos, postes y árboles de riesgo conocidos); **no se ha podido contrastar con una línea eléctrica real** porque los datos de prueba disponibles no tenían ninguna. Los cortes por huecos de más de 30 m se avisan: ahí no se ha medido nada.

#### Recortar parcelas

Parcelas desde un archivo vectorial, generadas por muestreo (aleatorio o sistemático) o dibujadas. Da una nube por parcela, una nube con todas y/o una capa de parcelas con sus métricas.

- **Alturas sobre el suelo (normalizar).** Por defecto está **marcado**: las nubes recortadas y las métricas de cada parcela salen con alturas sobre el suelo, restando a cada punto el MDT de **su nube**. Por eso hace falta el MDT de cada hoja; si falta, la herramienta lo dice antes de empezar. Si quieres trabajar con las cotas originales, desmárcalo en *Filtros y opciones*. Las métricas por parcela (percentiles, cobertura…) solo tienen sentido sobre alturas normalizadas, así que desmarcarlo solo conviene para recortes que se van a seguir procesando.
- **Parcelas que se solapan.** Cuando dos parcelas comparten superficie, los puntos de la zona común se pueden tratar de dos maneras. *Se cuentan en todas las parcelas que los contienen* (lo predeterminado): cada parcela es independiente y las métricas de cada una son completas. *Se asignan solo a la primera parcela que los contiene* (la primera del archivo): un punto no se repite nunca, así que la suma de puntos de todas las parcelas no supera la de la nube, pero la segunda parcela queda sin los puntos que comparte con la primera y sus métricas salen incompletas. Usa la primera opción para inventario y la segunda cuando necesites que cada punto aparezca una sola vez.
- **Filtros:** clases, retornos y límites de Z (de altura, si se normaliza).

**Cuándo usarlo:** el primer paso del enfoque por área (capítulo 13) y para preparar nubes de parcelas de campo. **Límite:** las métricas dependen del tamaño de la parcela y de la densidad de puntos; compara parcelas del mismo tamaño y de nubes con densidad parecida.

#### Cubicar árboles

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

Avisos: `TRUNCADO` (el tronco se pierde antes de llegar a la punta delgada: su volumen es solo el del tramo medido, y el resumen dice cuánto suman esos árboles), `SIN_DAP` (no hay sección válida a 1,3 m), `INTERPOLADO` (se rellenaron huecos), `INCLINADO` (más de 25°), `ENSANCHA` (por encima de 1,3 m alguna sección salía más ancha que las de debajo, por una rama, un fuste vecino o una horquilla, y se descartó: un tronco solo se estrecha hacia arriba), `SIN_VOLUMEN`, `BROTES` (pertenece a una cepa con varios brotes) y `BIFURCADO`.

Consejos: **el vóxel depende del equipo**. Con escáner terrestre estático (nubes densas y de poco ruido), 2 a 3 cm. Con escáner **móvil o SLAM** (GeoSLAM, mochila o de mano), 5 cm: el ruido es de varios centímetros y con vóxeles finos los troncos se pierden a pocos metros (hay un selector de equipo en el formulario, y `--voxel 0.05` en la línea de comandos). Un círculo se rechaza si hay vóxeles dentro de su mitad interior (*puntos dentro del círculo*, 5 por defecto) o si su residuo es mayor que el admitido: así no se miden como un solo tronco dos troncos pegados ni una rama con follaje. Además, si faltan troncos, baja los vóxeles mínimos de un tronco o la verticalidad mínima; si se cuelan arbustos, súbelos. Una sola posición de escáner solo ve una parte de cada tronco: baja los *sectores ocupados* con cuidado.


## 8. Ráster

- **Composición de bandas**: color natural, falso color infrarrojo y otras, con estirado del contraste, brillo y gamma. El estirado puede ser sin estirar, mínimo–máximo, percentiles 2–98 %, ±2 desviaciones o **cortes manuales**: se escriben el valor mínimo y el máximo que se pintan (los mismos para las tres bandas; por ejemplo, de 0 a 3000 en una reflectancia de Sentinel-2) y lo que queda fuera se satura. Al elegirlos parten de los límites que se estaban usando.
- **Índices espectrales**: NDVI (Rouse et al., 1974), GNDVI (Gitelson et al., 1996), NDWI (McFeeters, 1996), EVI (Huete et al., 2002), SAVI (Huete, 1988), NDMI (Gao, 1996), NBR (Key y Benson, 2006), VARI (Gitelson et al., 2002), ExG (Woebbecke et al., 1995) o una expresión propia con `B1…Bn` o `RED`, `GREEN`, `BLUE`, `NIR`, `SWIR1`, `SWIR2`. El índice sale como capa ráster y se guarda en GeoTIFF. **Cuándo usar cada uno:** NDVI y GNDVI para vigor y cobertura verde (el NDVI se satura con vegetación muy densa; el GNDVI es más sensible a la clorofila); SAVI cuando el suelo desnudo pesa; EVI en vegetación densa; NDWI y NDMI para agua y humedad de la vegetación; NBR para zonas quemadas; VARI y ExG con imágenes solo RGB (ortofotos), sin infrarrojo, con menos fiabilidad.
- **Igualar rásteres**: pone varios rásteres sobre las celdas de uno de **referencia** (mismo origen, tamaño de celda y dimensiones) para que coincidan celda a celda y se puedan apilar o usar juntos, por ejemplo en el análisis por objetos. Cada ráster da un archivo nuevo `<nombre>_alineado.tif` y el original no se toca. El remuestreo es *automático* por defecto (media si el ráster es más fino que la referencia, bilineal en los demás casos); con clases o identificadores usa *vecino más cercano*, porque promediarlos los estropea. Las celdas que el ráster no cubre quedan sin datos. La lista muestra la celda, el tamaño y el sistema de coordenadas de cada ráster y avisa si alguno no coincide con la referencia. **Límite:** todos deben estar en el mismo sistema de coordenadas; si no, la herramienta se niega (no reproyecta).
- **Crear multibanda**: apila en una sola imagen las bandas de varios rásteres, en el orden de la lista (por ejemplo, R, G, B y NIR sueltos). En *Bandas* se eligen las de cada ráster (por ejemplo `1,3`); vacío = todas. Hasta 64 bandas.
- **Mosaico**: une rásteres con el mismo número de bandas. **Donde se solapan** se elige qué hacer: *primero*, *último*, *media*, *mínimo*, *máximo*, *mediana*, *suma* o *número de capas*. También: zona que cubre (unión, intersección o la del primero), remuestreo (vecino, bilineal o media), tipo de dato, tamaño de píxel y valor de «sin datos».

Estas herramientas leen las imágenes por ventana y escriben un **Cloud Optimized GeoTIFF** (teselas de 512 × 512, vistas reducidas hasta caber en una tesela y todos los directorios al principio): se pueden unir imágenes de varios gigabytes con poca memoria y el resultado se abre al instante en QGIS o en un visor web. La compresión es **JPEG** (calidad 90, YCbCr, como las ortofotos del PNOA) para las imágenes de 8 bits y tres bandas, y **deflate** (sin pérdida) para todo lo demás (bandas de satélite, clases, alturas); en la línea de comandos se elige con `--compress deflate|jpeg` y `--quality`. Como referencia, el mosaico de tres hojas de ortofoto PNOA de 0,25 m (1,7 GB) ocupa 2,4 GB en JPEG frente a 14,7 GB sin pérdida, y sus colores difieren de los originales en menos de medio nivel sobre 255. Todos los rásteres deben estar en el mismo sistema de coordenadas (no se reproyecta).

## 9. Vectorial

- **Capas** (Inicio → Abrir): abre imágenes y rásteres (GeoTIFF, JPEG 2000, PNG y JPEG con archivo de coordenadas, ENVI, SRTM), shapefiles y GeoJSON. También se pueden arrastrar a la ventana.
- **Nueva capa**: puntos, líneas o áreas. Se digitaliza en la capa activa (con ajuste a vértices), se edita su **tabla de atributos** y se guarda como shapefile con la proyección de la nube.
- **Simbología**: color único, por categorías o por rampa, con clasificación y etiquetas.
- **Edición vectorial**: seleccionar, mover, editar vértices, deshacer y rehacer.
- Las capas de áreas visibles alimentan la herramienta de parcelas.

## 10. Rutas y formatos

- Los resultados van a la misma carpeta que la entrada, con otro nombre.
- Una ejecución nunca hereda la ruta de otra ni sobrescribe sin avisar. Si eliges una ruta a mano que ya existe, se pide confirmación.
- Tablas (métricas, árboles, parcelas): capa vectorial (shapefile, con el `.prj` de la nube), CSV o ambos. Los nombres de campo de un shapefile tienen como mucho 10 caracteres; `<nombre>_campos.txt` explica cada uno.
- Toda salida de métricas lleva la fecha de captura (`Date`), sacada del tiempo GPS de los puntos (o de la hora Unix, si el escáner la usa, como GeoSLAM: una fecha futura no puede ser la de captura).
- **Formatos que se leen:** nubes LAS/LAZ (incluido **COPC**, que además se lee solo en la parte que hace falta), **PLY** (ASCII o binario) y **E57** (el formato de intercambio de los escáneres terrestres y móviles: cada escaneo se coloca con su pose, las coordenadas esféricas se pasan a cartesianas, color e intensidad se guardan en 16 bits y cada punto lleva el número de su escaneo como ID de fuente); los dos se convierten a LAZ, con *Abrir* en el programa o `easylidar import`. También ASCII `x y z`; vectores shapefile y **GeoJSON**; rásteres:
  - **GeoTIFF** (también Cloud Optimized y BigTIFF) y ASCII grid.
  - **JPEG 2000** (`.jp2`, `.j2k`, `.j2c`), que es como vienen las bandas de **Sentinel-2**: lectura propia en Rust, sin librerías externas. Se leen la georreferencia GMLJP2 (la de Sentinel-2) y GeoJP2, las bandas de 8 a 16 bits y el color de las imágenes TCI. Se decodifican **solo las teselas de la ventana pedida**, y se usan las vistas reducidas que el propio archivo lleva (cada nivel divide el tamaño por dos), así que abrir una banda de 10 m es inmediato. Los valores salen tal cual vienen (en Sentinel-2, el número digital; el 0 es «sin dato» pero no se enmascara).
  - **PNG** y **JPEG** con su archivo de coordenadas (`.pgw`/`.pngw`/`.wld` y `.jgw`/`.jpgw`/`.wld`; el `.prj` da el sistema), **ENVI** (`.bil`, `.bsq`, `.bip`, `.img`, `.dat` con su `.hdr`, y los `.flt` de Esri), y teselas **SRTM** `.hgt` (con el nombre tipo `N37W006`). Se leen enteros en la memoria, así que son para imágenes de tamaño normal; una ortofoto enorme conviene en GeoTIFF o JPEG 2000.

  **No se leen:** **ECW** (formato propietario: solo se lee con un SDK que no es de código abierto, y un programa en Rust puro no puede llevarlo; conviértelo con QGIS, GeoLibre o RustGis, por ejemplo en QGIS con *Ráster → Conversión → Traducir*), MrSID, Imagine `.img`, NetCDF, HDF o GRIB, ni KML. Un JPEG 2000 con cambios de progresión (POC) o cabeceras de paquetes empaquetadas no se lee (no los usa Sentinel-2) y una imagen girada o con píxeles no cuadrados exige remuestrearla antes.
- **GeoPackage** (`.gpkg`): se lee la primera capa de objetos (puntos, líneas o polígonos, con su tabla de atributos y su sistema de coordenadas) para mostrarla, editarla y usarla como parcelas o rodales en el recorte, las métricas y el resto de herramientas. Si el archivo tiene varias capas se carga la primera; para otra, expórtala antes con QGIS, GeoLibre o RustGis. Se lee entero en memoria (límite de 2 GB). No se escribe.
- **Formatos que se escriben:** LAS/LAZ, **COPC** (*Guardar como COPC* o `easylidar copc`), GeoTIFF (*Mosaico*, *Crear multibanda*, *Igualar rásteres* y `easylidar cog` escriben **Cloud Optimized GeoTIFF**: teselas de 512 y vistas reducidas con todos los directorios al principio del archivo; `cog` acepta también imágenes de varias bandas, de cualquier tamaño), shapefile, **GeoJSON** (guardando una capa con esa extensión, o `easylidar geojson` para convertir entre shapefile y GeoJSON; lleva el sistema de coordenadas en el miembro `crs`), CSV y PNG.
- Los archivos que están **solo en la nube** (OneDrive, iCloud) y no se han descargado no se pueden leer: el programa lo indica; descárgalos antes.
- **Sistemas de coordenadas.** Si lo que combinas está en sistemas distintos (nubes entre sí, con el MDT, con rásteres o con parcelas y muestras en shapefile o GeoJSON), EasyLiDAR **lo pasa al sistema de la primera nube** y lo dice en el informe: «"mdt.tif" estaba en ETRS89 / UTM 29N (EPSG:25829) y se ha pasado a ETRS89 / UTM 30N (EPSG:25830)». Las nubes y los rásteres se convierten en copias temporales (que se borran al acabar) y las parcelas en memoria; si todo está ya en un sistema no se hace nada. Las transformaciones son las de PROJ, con las mallas del IGN y del ICGC para ED50 a ETRS89 (Península, Baleares y Cataluña) y el geoide EGM08-REDNAP incluidos en el programa. **Solo cambian X e Y**: las alturas se conservan. Entre husos, un ráster mantiene su celda (por ejemplo 1 m) y se interpola; si sus valores son enteros (clases) se toma el vecino más cercano. Una nube convertida cuesta una pasada sobre ella; para trabajar mucho con ella, conviértela una vez con `easylidar reproject nube.laz -o nube_25830.laz --to 25830` (también vale para rásteres; con `--from` se indica el sistema de un archivo que no lo declara). **Límites:** las imágenes de varias bandas y las herramientas de imágenes grandes (*Apilar*, *Mosaico*, *Igualar rásteres*) exigen el mismo sistema y se niegan si no; no se convierten nubes desde o hacia grados; ED50 en Canarias no está definido y no debe usarse. Si un archivo no declara su sistema, no se puede comprobar y el resultado lleva un aviso. ETRS89 y WGS 84 del mismo huso se consideran equivalentes y solo avisan (difieren en torno a un metro). En la lista de capas, un ⚠ marca la nube o la capa cuyo sistema no coincide con el de la primera nube.

## 11. Rendimiento y memoria

- Las nubes grandes no se cargan enteras: la vista dibuja una muestra (*Puntos en pantalla*) y los procesos leen en flujo.
- Un LAZ que necesita varias pasadas se descomprime una vez a un archivo temporal que se borra al terminar (deja espacio libre en el disco temporal: la nube sin comprimir).
- Teselas: tamaño y búfer se ajustan en *Teselas y rendimiento*. La memoria es aproximadamente (núcleos × tesela). Con teselas de 250 m y 30 m de búfer, un equipo de 16 GB trabaja sin problema con nubes de cualquier tamaño.
- Qué pasos son en flujo (memoria casi constante): información, MDT por *media por celda*, CHM por máximo, MDS por máximo, métricas, recorte, exportar (también a LAZ y normalizado), adelgazar, duplicados. Qué pasos son por teselas (memoria = núcleos × tesela): clasificar, MDT por TIN o IDW, MDS por TIN o IDW, CHM *pit-free*.
- Para trabajar solo con lo que está en pantalla, selecciona los puntos: el proceso se aplica solo a ellos.
- **COPC.** Un archivo COPC es un LAZ cuyos bloques son los nodos de un octree. Al **recortar** con parcelas se leen solo los nodos que tocan las parcelas (en una nube de 50 millones de puntos, una ventana de 100 × 100 m lee el 4 % de los puntos), y al **mostrarla** con decimación se leen solo los primeros niveles del árbol (89,9 millones de puntos en 0,25 s en lugar de recorrerlos todos). El resto de las herramientas lo leen como un LAZ normal.

## 12. Trabajar con miles de teselas

Un vuelo es una colección de cientos o miles de hojas (por ejemplo, las de 2 × 2 km del PNOA). EasyLiDAR las trata así:

- **Cada hoja se procesa por separado y da su propio resultado.** Si una hoja falla, el proceso se detiene con el mensaje del error y se borra lo parcial de esa hoja (las hojas ya terminadas se conservan). No hay límite de hojas porque nunca se mantienen varias en memoria a la vez.
- **Se cargan como hojas solo las que quieras ver.** Para procesar, no hace falta abrirlas: en la **línea de comandos** se pasa la lista de archivos y no se dibuja nada. La interfaz sirve para probar los parámetros en una o dos hojas.
- **Prueba en una hoja representativa** (con relieve, bosque cerrado y claros) antes de lanzar el lote: elige el método de suelo, el tamaño de celda y las alturas.
- **Usa LAZ** como entrada: pesa unas 5 veces menos y se descomprime en paralelo.
- **Las salidas llevan el nombre de la hoja** (`hoja_mdt_1m_tin.tif`) y van a la carpeta de cada hoja. Una ejecución nueva no sobrescribe: crea `_2`, `_3`… Para repetir un lote, usa otra carpeta de resultados o borra lo anterior.
- **Alineación entre hojas.** Las celdas de las métricas están alineadas a múltiplos del tamaño de celda: si el lado de la hoja es múltiplo de la celda (2000 m con celdas de 20 m), las celdas de hojas vecinas encajan sin solaparse ni dejar huecos. Si no lo es, las celdas del borde quedan con una parte de los puntos: ponles un mínimo de puntos más alto o usa un tamaño de celda divisor del lado de la hoja.
- **Mosaicos.** MDT y CHM pueden dar además un mosaico de todas las hojas (casilla o `--mosaic`); para el resto de rásteres usa *Mosaico* en Ráster. Los mosaicos de miles de hojas se escriben por franjas comprimidas, sin cargar las imágenes enteras.
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
3. **Métricas de las parcelas de campo.** *Recortar parcelas* con el archivo de parcelas (o con centros y radio): modo **solo métricas**. Da una capa con un polígono por parcela y todas las métricas de *Métricas de rodal* (7.3), con el número de parcela. Opcionalmente, una nube por parcela para revisarlas. Deja marcado *Alturas sobre el suelo*: las métricas de las parcelas y las de toda la superficie deben calcularse igual.
4. **Modelo.** *Modelos de transición* une (por el nombre de la parcela) la tabla de métricas con la tabla del inventario de campo (Excel, CSV o texto), ajusta la regresión, la valida dejando una parcela fuera cada vez y enseña si la elección de variables es estable y qué parcelas mandan. Mira la validación cruzada, no el ajuste. Si prefieres otro programa estadístico, la tabla de métricas y la de campo se pueden unir igualmente por el nombre. **Usa los mismos parámetros** (límite de cobertura, tamaño de parcela, MDT) en las parcelas y en el resto del territorio.
5. **Métricas de toda la superficie.** *Métricas de rodal* en todas las hojas, con el tamaño de celda **igual al de la parcela** (por ejemplo, 400 m² → celdas de 20 m, o 15 m para parcelas de radio 7,98 m). Cada hoja da su capa de celdas con las mismas columnas que las parcelas.
6. **Aplicar el modelo.** En la misma herramienta, marcando *Hacer el mapa* y dando las tablas de texto de las celdas del paso 5: sale el ráster de la variable, y opcionalmente el del intervalo de predicción. Las celdas fuera del rango de las parcelas quedan vacías.
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
easylidar terrain mdt.tif --product slope --percent
easylidar terrain mdt.tif --product streams --threshold 10000
easylidar terrain mdt.tif --product viewshed --observer 504000 4405000 --max-distance 2000
easylidar metrics nube.laz
easylidar trees chm.tif --method silva --crowns
easylidar obia chm.tif --cloud nube.laz --roughness 2 --scale 8 --rules reglas.txt
easylidar obia chm.tif --cloud nube.laz --samples muestras.shp --class-field clase
easylidar obia chm.tif --cloud nube.laz --scale 12 --min-cells 200 --kmeans 4 --strata --plots 30
easylidar clip nube.laz --regions parcelas.geojson --mode each
easylidar metrics nube.laz --cell 20 --structure
easylidar change chm_2019.tif chm_2024.tif --years 5 --stands rodales.shp --trees
easylidar corridor nube_clasificada.laz --dtm mdt.tif --min-distance 4
easylidar import escaneo.ply --epsg 25830 -o escaneo.laz
easylidar import escaneo_tls.e57 --epsg 25830
easylidar colorize nube.laz --image ortofoto.tif
easylidar intensity nube.laz --trajectory trayectoria.txt
easylidar intensity nube.laz --altitude 3200 --exponent 2
easylidar copc nube.laz
easylidar dtm nube.laz --cell 1 --breaklines bordes_camino.shp
easylidar geojson capa.shp
easylidar cog chm.tif
easylidar --results-dir D:/resultados chm nube.laz --dtm mdt.tif
easylidar cubicar parcela_tls.laz --csv --tree-clouds --log-length 0.5 --top-diameter 0.07
easylidar dedup nube.laz --also class,gps
easylidar compare nube2.laz --reference nube1.laz --distance 0.5
easylidar fit nube2.laz --reference nube1.laz
easylidar swath vuelo.laz --cell 1
easylidar stack r.tif g.tif b.tif nir.tif -o multibanda.tif
easylidar mosaic a.tif b.tif -o mosaico.tif --overlap mean
easylidar mosaic orto1.tif orto2.tif -o orto.tif --compress jpeg --quality 85
easylidar cog ortofoto.tif
easylidar align chm.tif mdt.tif ortofoto.tif
easylidar render nube.laz --view 3d -o vista.png --color class
easylidar export nube.laz -o puntos.csv --items x,y,z,class --header
easylidar export nube.laz --normalize --dtm mdt.tif -o altura.laz
```

## 15. Atajos

Están también en **Ayuda → Atajos de teclado**.

| Atajo | Acción |
|---|---|
| Ctrl+O | Abrir (nubes, rásteres y vectores) |
| Ctrl+K | Buscar comando |
| F1 | Manual |
| Q / W / Z / X | Consultar / Medir / Zoom / Desplazar |
| T | TIN sombreado |
| + / − | Tamaño del punto |
| Ctrl+Z / Ctrl+Y | Deshacer / Rehacer |
| Flechas | Mover el perfil (planta) o girar (3D) |
| Esc | Cancelar la operación en curso |
| Ctrl+A / Ctrl+I | Seleccionar todo / invertir la selección |
| Mayús / Alt / Ctrl (al seleccionar) | Añadir / restar / intersecar |
| Supr | Eliminar los puntos seleccionados |

## 16. Solución de problemas

- **El volumen de las copas sale absurdo (cientos de miles de m³) o las alturas son enormes:** se ha usado como superficie el ráster de copas, que guarda números de árbol, en vez del CHM. En *Volumen sobre MDT* elige el CHM (el programa ya no confunde ambos al buscarlos solo).
- **Un MDT o un CHM antiguo es un rectángulo con valores inventados fuera de la nube:** se calculó con una versión anterior. Los rásteres nuevos siguen el contorno de la nube; regénéralos.
- **«No hay MDT de esta nube»**: genera antes el MDT con la herramienta MDT.
- **La nube no se ve completa**: sube *Puntos en pantalla* en *Vista → Ventanas* (grupo *Carga de puntos*) y pulsa *Aplicar*.
- **Los diámetros de los árboles salen vacíos**: comprueba que la nube tenga suelo (o marca que ya está normalizada) y que la franja de búsqueda esté dentro de la altura de los troncos.
- **El mosaico de imágenes dice que los sistemas son distintos**: pasa los rásteres al mismo sistema con `easylidar reproject` antes de unirlos.
- **Un resultado a medias tras detener un proceso**: se borra solo; si queda algún archivo, bórralo a mano.
- **La interfaz va lenta o a saltos**: mira qué usa para dibujar. Con una tarjeta gráfica moderna usa **wgpu** (DirectX 12, Vulkan o Metal). Con un driver antiguo (por ejemplo una Intel HD de 2015, que no llega a DirectX 12) wgpu solo ofrece un adaptador **por software**, que dibuja con el procesador y va muy lento, así que el programa elige entonces **OpenGL**. Para ver lo que detecta: `easylidar-gui --gpu-info`. Para medir los fotogramas por segundo con una nube: `easylidar-gui nube.laz --bench` (cuenta 8 s y sale). Para forzar uno: variable de entorno `EASYLIDAR_RENDERER` con `glow` (OpenGL) o `wgpu`.

## 17. Bibliografía

Referencias de los métodos descritos en este manual.

**Clasificación y modelos del terreno**

- Axelsson, P. (2000). DEM generation from laser scanner data using adaptive TIN models. *International Archives of Photogrammetry and Remote Sensing*, 33(B4), 110–117.
- Evans, J. S. y Hudak, A. T. (2007). A multiscale curvature algorithm for classifying discrete return LiDAR in forested environments. *IEEE Transactions on Geoscience and Remote Sensing*, 45(4), 1029–1038.
- Kraus, K. y Pfeifer, N. (1998). Determination of terrain models in wooded areas with airborne laser scanner data. *ISPRS Journal of Photogrammetry and Remote Sensing*, 53(4), 193–203.
- Rusu, R. B., Marton, Z. C., Blodow, N., Dolha, M. y Beetz, M. (2008). Towards 3D point cloud based object maps for household environments. *Robotics and Autonomous Systems*, 56(11), 927–941.
- Shepard, D. (1968). A two-dimensional interpolation function for irregularly-spaced data. *Proceedings of the 23rd ACM National Conference*, 517–524.
- Pingel, T. J., Clarke, K. C. y McBride, W. A. (2013). An improved simple morphological filter for the terrain classification of airborne LIDAR data. *ISPRS Journal of Photogrammetry and Remote Sensing*, 77, 21–30.
- Zhang, K., Chen, S.-C., Whitman, D., Shyu, M.-L., Yan, J. y Zhang, C. (2003). A progressive morphological filter for removing nonground measurements from airborne LIDAR data. *IEEE Transactions on Geoscience and Remote Sensing*, 41(4), 872–882.
- Zhang, W., Qi, J., Wan, P., Wang, H., Xie, D., Wang, X. y Yan, G. (2016). An easy-to-use airborne LiDAR data filtering method based on cloth simulation. *Remote Sensing*, 8(6), 501.

**Análisis por objetos**

- Baatz, M. y Schäpe, A. (2000). Multiresolution segmentation: an optimization approach for high quality multi-scale image segmentation. En J. Strobl, T. Blaschke y G. Griesebner (eds.), *Angewandte Geographische Informationsverarbeitung XII*, 12–23. Wichmann, Heidelberg.
- Blaschke, T. (2010). Object based image analysis for remote sensing. *ISPRS Journal of Photogrammetry and Remote Sensing*, 65(1), 2–16.
- Arthur, D. y Vassilvitskii, S. (2007). k-means++: the advantages of careful seeding. *Proceedings of the 18th Annual ACM-SIAM Symposium on Discrete Algorithms*, 1027–1035.
- Breiman, L. (2001). Random forests. *Machine Learning*, 45(1), 5–32.
- Cochran, W. G. (1977). *Sampling techniques* (3.ª ed.). Wiley, Nueva York.
- Congalton, R. G. (1991). A review of assessing the accuracy of classifications of remotely sensed data. *Remote Sensing of Environment*, 37(1), 35–46.
- Lloyd, S. P. (1982). Least squares quantization in PCM. *IEEE Transactions on Information Theory*, 28(2), 129–137.
- Neyman, J. (1934). On the two different aspects of the representative method: the method of stratified sampling and the method of purposive selection. *Journal of the Royal Statistical Society*, 97(4), 558–625.

**Modelos de transición**

- Akaike, H. (1974). A new look at the statistical model identification. *IEEE Transactions on Automatic Control*, 19(6), 716–723.
- Allen, D. M. (1974). The relationship between variable selection and data augmentation and a method for prediction. *Technometrics*, 16(1), 125–127.
- Ambroise, C. y McLachlan, G. J. (2002). Selection bias in gene extraction on the basis of microarray gene-expression data. *Proceedings of the National Academy of Sciences*, 99(10), 6562–6566.
- Baskerville, G. L. (1972). Use of logarithmic regression in the estimation of plant biomass. *Canadian Journal of Forest Research*, 2(1), 49–53.
- Breusch, T. S. y Pagan, A. R. (1979). A simple test for heteroscedasticity and random coefficient variation. *Econometrica*, 47(5), 1287–1294.
- Cook, R. D. (1977). Detection of influential observation in linear regression. *Technometrics*, 19(1), 15–18.
- Hurvich, C. M. y Tsai, C.-L. (1989). Regression and time series model selection in small samples. *Biometrika*, 76(2), 297–307.
- Koenker, R. (1981). A note on studentizing a test for heteroscedasticity. *Journal of Econometrics*, 17(1), 107–112.
- Næsset, E. (2002). Predicting forest stand characteristics with airborne scanning laser using a practical two-stage procedure and field data. *Remote Sensing of Environment*, 80(1), 88–99.
- Ramsey, J. B. (1969). Tests for specification errors in classical linear least-squares regression analysis. *Journal of the Royal Statistical Society B*, 31(2), 350–371.
- Royston, P. (1995). Remark AS R94: a remark on algorithm AS 181: the W-test for normality. *Applied Statistics*, 44(4), 547–551.
- Schwarz, G. (1978). Estimating the dimension of a model. *Annals of Statistics*, 6(2), 461–464.
- Shapiro, S. S. y Wilk, M. B. (1965). An analysis of variance test for normality (complete samples). *Biometrika*, 52(3–4), 591–611.

**Estructura del dosel**

- MacArthur, R. H. y MacArthur, J. W. (1961). On bird species diversity. *Ecology*, 42(3), 594–598.
- Monsi, M. y Saeki, T. (1953). Über den Lichtfaktor in den Pflanzengesellschaften und seine Bedeutung für die Stoffproduktion. *Japanese Journal of Botany*, 14, 22–52.
- Van Ewijk, K. Y., Treitz, P. M. y Scott, N. A. (2011). Characterizing forest succession in central Ontario using lidar-derived indices. *Photogrammetric Engineering & Remote Sensing*, 77(3), 261–269.

**Modelo de copas y árboles individuales**

- Chen, Q., Baldocchi, D., Gong, P. y Kelly, M. (2006). Isolating individual trees in a savanna woodland using small footprint lidar data. *Photogrammetric Engineering & Remote Sensing*, 72(8), 923–932.
- Dalponte, M. y Coomes, D. A. (2016). Tree-centric mapping of forest carbon density from airborne laser scanning and hyperspectral data. *Methods in Ecology and Evolution*, 7(10), 1236–1245.
- Eysn, L. et al. (2015). A benchmark of lidar-based single tree detection methods using heterogeneous forest data from the Alpine Space. *Forests*, 6(5), 1721–1747.
- Khosravipour, A., Skidmore, A. K., Isenburg, M., Wang, T. y Hussin, Y. A. (2014). Generating pit-free canopy height models from airborne lidar. *Photogrammetric Engineering & Remote Sensing*, 80(9), 863–872.
- Popescu, S. C. y Wynne, R. H. (2004). Seeing the trees in the forest: using lidar and multispectral data fusion with local filtering and variable window size for estimating tree height. *Photogrammetric Engineering & Remote Sensing*, 70(5), 589–604.
- Silva, C. A., Hudak, A. T., Vierling, L. A., Loudermilk, E. L., O'Brien, J. J., Hiers, J. K., et al. (2016). Imputation of individual longleaf pine (*Pinus palustris* Mill.) tree attributes from field and LiDAR data. *Canadian Journal of Remote Sensing*, 42(5), 554–573.
- Vincent, L. y Soille, P. (1991). Watersheds in digital spaces: an efficient algorithm based on immersion simulations. *IEEE Transactions on Pattern Analysis and Machine Intelligence*, 13(6), 583–598.

**Inventario por área y métricas**

- Hosking, J. R. M. (1990). L-moments: analysis and estimation of distributions using linear combinations of order statistics. *Journal of the Royal Statistical Society B*, 52(1), 105–124.
- McGaughey, R. J. *FUSION/LDV: Software for LIDAR Data Analysis and Visualization*. USDA Forest Service, Pacific Northwest Research Station.
- Næsset, E. (2002). Predicting forest stand characteristics with airborne scanning laser using a practical two-stage procedure and field data. *Remote Sensing of Environment*, 80(1), 88–99.
- Roussel, J.-R., Auty, D., Coops, N. C., Tompalski, P., Goodbody, T. R. H., Meador, A. S., Bourdon, J.-F., de Boissieu, F. y Achim, A. (2020). lidR: an R package for analysis of airborne laser scanning (ALS) data. *Remote Sensing of Environment*, 251, 112061.
- White, J. C., Wulder, M. A., Varhola, A., Vastaranta, M., Coops, N. C., Cook, B. D., Pitt, D. y Woods, M. (2013). A best practices guide for generating forest inventory attributes from airborne laser scanning data using an area-based approach. *The Forestry Chronicle*, 89(6), 722–723.

**Estadística de valores raros**

- Leys, C., Ley, C., Klein, O., Bernard, P. y Licata, L. (2013). Detecting outliers: do not use standard deviation around the mean, use absolute deviation around the median. *Journal of Experimental Social Psychology*, 49(4), 764–766.
- Rousseeuw, P. J. y Croux, C. (1993). Alternatives to the median absolute deviation. *Journal of the American Statistical Association*, 88(424), 1273–1283.
- Tukey, J. W. (1977). *Exploratory Data Analysis*. Addison-Wesley.

**Análisis de terreno**

- Barnes, R., Lehman, C. y Mulla, D. (2014). Priority-flood: an optimal depression-filling and watershed-labeling algorithm for digital elevation models. *Computers & Geosciences*, 62, 159–168.
- Beven, K. J. y Kirkby, M. J. (1979). A physically based, variable contributing area model of basin hydrology. *Hydrological Sciences Bulletin*, 24(1), 43–69.
- Horn, B. K. P. (1981). Hill shading and the reflectance map. *Proceedings of the IEEE*, 69(1), 14–47.
- O'Callaghan, J. F. y Mark, D. M. (1984). The extraction of drainage networks from digital elevation data. *Computer Vision, Graphics, and Image Processing*, 28(3), 323–344.
- Weiss, A. D. (2001). Topographic position and landforms analysis. Póster, *ESRI International User Conference*, San Diego.
- Wilson, M. F. J., O'Connell, B., Brown, C., Guinan, J. C. y Grehan, A. J. (2007). Multiscale terrain analysis of multibeam bathymetry data for habitat mapping on the continental slope. *Marine Geodesy*, 30(1–2), 3–35.
- Zevenbergen, L. W. y Thorne, C. R. (1987). Quantitative analysis of land surface topography. *Earth Surface Processes and Landforms*, 12(1), 47–56.

**Registro de nubes, cubicación y formatos**

- ASPRS (2019). *LAS Specification 1.4 – R15*. American Society for Photogrammetry and Remote Sensing.
- Besl, P. J. y McKay, N. D. (1992). A method for registration of 3-D shapes. *IEEE Transactions on Pattern Analysis and Machine Intelligence*, 14(2), 239–256.
- Husch, B., Beers, T. W. y Kershaw, J. A. (2003). *Forest Mensuration* (4.ª ed.). John Wiley & Sons. (Fórmula de Smalian.)
- Isenburg, M. (2013). LASzip: lossless compression of LiDAR data. *Photogrammetric Engineering & Remote Sensing*, 79(2), 209–217.
- Liang, X., Kankare, V., Hyyppä, J., Wang, Y., Kukko, A., Haggrén, H., et al. (2016). Terrestrial laser scanning in forest inventories. *ISPRS Journal of Photogrammetry and Remote Sensing*, 115, 63–77.

**Índices espectrales**

- Gao, B.-C. (1996). NDWI: a normalized difference water index for remote sensing of vegetation liquid water from space. *Remote Sensing of Environment*, 58(3), 257–266.
- Gitelson, A. A., Kaufman, Y. J. y Merzlyak, M. N. (1996). Use of a green channel in remote sensing of global vegetation from EOS-MODIS. *Remote Sensing of Environment*, 58(3), 289–298.
- Gitelson, A. A., Kaufman, Y. J., Stark, R. y Rundquist, D. (2002). Novel algorithms for remote estimation of vegetation fraction. *Remote Sensing of Environment*, 80(1), 76–87.
- Huete, A. R. (1988). A soil-adjusted vegetation index (SAVI). *Remote Sensing of Environment*, 25(3), 295–309.
- Huete, A., Didan, K., Miura, T., Rodriguez, E. P., Gao, X. y Ferreira, L. G. (2002). Overview of the radiometric and biophysical performance of the MODIS vegetation indices. *Remote Sensing of Environment*, 83(1–2), 195–213.
- Key, C. H. y Benson, N. C. (2006). Landscape assessment: ground measure of severity, the Composite Burn Index, and remote sensing of severity, the Normalized Burn Ratio. En *FIREMON: Fire Effects Monitoring and Inventory System*, USDA Forest Service, RMRS-GTR-164-CD.
- McFeeters, S. K. (1996). The use of the Normalized Difference Water Index (NDWI) in the delineation of open water features. *International Journal of Remote Sensing*, 17(7), 1425–1432.
- ITU-T (2019). *Recommendation T.800 | ISO/IEC 15444-1: JPEG 2000 image coding system, Part 1: Core coding system*.
- Taubman, D. S. y Marcellin, M. W. (2002). *JPEG2000: Image Compression Fundamentals, Standards and Practice*. Kluwer.
- Rouse, J. W., Haas, R. H., Schell, J. A. y Deering, D. W. (1974). Monitoring vegetation systems in the Great Plains with ERTS. *NASA SP-351*, 309–317.
- Woebbecke, D. M., Meyer, G. E., Von Bargen, K. y Mortensen, D. A. (1995). Color indices for weed identification under various soil, residue, and lighting conditions. *Transactions of the ASAE*, 38(1), 259–269.

Las comparaciones con otros programas (FUSION, MCC-LIDAR, lidR) están en `docs/REFERENCIA.md`.


## 18. Acerca de

**EasyLiDAR** · Procesamiento y edición de nubes LiDAR para forestales.
Autor: Julio M. Sevilla.
Contacto: julio@jsevilla.es · https://es.linkedin.com/in/juliomsevilla

Se agradecen todas las aportaciones y la detección de errores; los errores se irán corrigiendo en la medida de lo posible.

Uso gratuito. Consulta `LICENSE` para las condiciones y `THIRD_PARTY_LICENSES.md` y `NOTICE` para los componentes de terceros.
