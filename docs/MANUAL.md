# HIPSO · Manual de uso

**HIPSO: procesamiento y edición de nubes de puntos.**
Autor: Julio M. Sevilla · julio@jsevilla.es · https://es.linkedin.com/in/juliomsevilla

---

## 1. Qué es HIPSO

HIPSO es un programa para abrir nubes de puntos LiDAR (LAS y LAZ), clasificarlas y editarlas, generar modelos del terreno y de alturas de la vegetación, calcular métricas e inventarios de la vegetación, medir troncos y estimar el volumen de los individuos en nubes de escáner terrestre y trabajar, además, con las imágenes y las capas propias de un sistema de información geográfica.

Su diseño responde a tres premisas, y las tres se deducen de un mismo problema: una nube LiDAR de un territorio tiene cientos de millones de puntos, y un programa que no esté pensado para ese tamaño se queda sin memoria o sin paciencia del usuario. La primera premisa es la rapidez, que no se declara, se mide, y por eso este manual da el tiempo de cada herramienta allí donde se ha medido. La segunda es la eficiencia con nubes enormes: los procesos leen la nube como un flujo, de modo que la memoria depende del área y del tamaño de celda, no del número de puntos, y los pasos locales trabajan por teselas, en paralelo, una por núcleo. La tercera es la edición cómoda: seleccionar, reclasificar y borrar con pocos clics, deshacer y rehacer, y ver el resultado al instante en planta, perfil y 3D.

Funciona en Windows, macOS y Linux, y es gratuito (las condiciones están en el archivo `LICENSE`).

## 2. Instalación

El programa no necesita instalador, y por lo tanto basta con tres pasos. Primero se descomprime el ZIP del sistema en una carpeta con permisos de escritura. Después se ejecuta `hipso-gui` (`hipso-gui.exe` en Windows); en macOS y Linux, si no arranca, hay que darle permiso de ejecución con `chmod +x hipso-gui hipso`. Por último, `hipso` es la versión de línea de comandos, descrita en el capítulo 14.

La primera vez en macOS se abre `HIPSO.app`, el sistema lo bloquea y hay que ir a Ajustes del Sistema, Privacidad y seguridad, y pulsar Abrir igualmente. En Windows, si aparece el aviso azul de SmartScreen, se pulsa Más información y después Ejecutar de todas formas. Ambos avisos tienen el mismo origen (el programa no está firmado por un editor reconocido por el sistema) y ninguno indica un problema del archivo.

Los requisitos son Windows 10/11, macOS 12 o superior, o un Linux con X11 o Wayland y OpenGL 2 o superior. En Linux hacen falta además las bibliotecas GTK 3, porque de ellas dependen los diálogos de archivo.

## 3. Primeros pasos

El flujo de trabajo normal tiene cuatro momentos, y conviene conocerlos en orden porque cada uno condiciona el siguiente.

El primero es abrir los datos. *Archivo → Abrir* (Ctrl+O, o arrastrando los archivos a la ventana) admite nubes de puntos, rásteres e imágenes, shapefiles, GeoJSON, GeoPackage y KML/KMZ, varios a la vez, y los añade a lo que ya está abierto (*Cerrar todo* empieza de nuevo). Cada nube es una hoja. Junto a *Abrir*, *Recientes* guarda los diez últimos archivos, y todos los diálogos de archivo empiezan en la carpeta del último abierto; el programa lo recuerda entre sesiones en `preferencias.txt`, dentro de la carpeta de configuración del sistema.

El segundo es mirar. La nube se dibuja en la ventana, la rueda del ratón acerca y el botón central vuelve al encuadre anterior.

El tercero es procesar. La pestaña *LiDAR* reúne las herramientas en cinco grupos (*Preparar*, *Calidad*, *Superficies*, *Inventario de la vegetación* e *Infraestructuras*), y cada una abre una ventana con sus parámetros y el botón *▶ Ejecutar*.

El cuarto es recoger el resultado. Los resultados se guardan junto a los datos, en la carpeta de la nube de origen y con otro nombre, y se cargan solos en el visor al terminar. El primer resultado queda el primero de la lista, encima y seleccionado; una nube resultado pasa a ser la primera nube, y la original se queda cargada (si no hace falta, se quita a mano). Si un proceso produce más de 8 nubes, se cargan las 8 primeras, un aviso lo indica y el resto queda en disco.

Todo trabajo se puede detener con *■ Detener*, y lo que se hubiera escrito a medias se borra. Mientras corre, la barra de estado y el Registro dicen cuánto falta (apartado 7).

## 4. La interfaz

Este capítulo recorre la ventana de HIPSO de arriba abajo y de izquierda a derecha, y explica uno a uno los botones y las opciones que contiene. Cada botón tiene su propio título, y bajo él se dice qué hace, cuándo conviene usarlo y dónde está explicado con detalle el método que lleva detrás. El orden es el de la propia pantalla: primero la ventana entera (apartado 4.1), luego la cabecera (4.2) y las pestañas de la cinta de izquierda a derecha (4.3 a 4.8), después el panel lateral (4.9) y, por último, las vistas (4.10) y los ajustes (4.11).

### 4.1 Las piezas de la ventana

La ventana se compone de seis piezas, y cada una responde a una pregunta distinta. La **cabecera** (arriba) responde a qué se abre y se busca a un clic. La **cinta de opciones** (debajo) responde a qué se puede hacer, y se divide en pestañas y, dentro de cada una, en grupos. El **panel lateral** (a la izquierda) responde a qué hay cargado y cómo se dibuja. El **área de vistas** (en el centro) muestra la planta, el perfil y la vista 3D. La **barra de estado** (abajo) dice dónde está el cursor y qué proceso corre. El **Registro** (un cuadro de texto bajo las vistas, que se muestra o se oculta) conserva lo ocurrido.

La cinta tiene a la izquierda el botón naranja *Inicio* y seis pestañas fijas: *Archivo*, *Vista*, *LiDAR*, *Vectorial*, *Raster* y *Flujos*. El criterio de reparto es la tarea. *Archivo* se ocupa del trabajo en sí (proyectos y archivos), *Vista* de cómo se ve, *LiDAR* de todo lo que parte de una nube de puntos, *Vectorial* y *Raster* de las capas de esos tipos y *Flujos* de encadenar herramientas. Cuando la ventana se estrecha, los grupos de la derecha se pliegan en un botón con menú, de manera que todo sigue a un clic.

En la cinta hay tres clases de botón. El **botón grande** (un icono sobre la etiqueta) se reserva para lo más usado de cada grupo. El **botón pequeño** (un icono junto a la etiqueta, en columnas de hasta tres) es el resto de las herramientas. El **botón con menú** (marcado con ▾) abre una lista de herramientas emparentadas. Pasar el ratón sobre un botón muestra un texto de ayuda con lo que hace y su atajo, y el botón de una herramienta con ventana propia queda marcado mientras la ventana está abierta.

Las herramientas de las pestañas *LiDAR*, *Vectorial* y *Raster* abren una ventana flotante con sus opciones y un botón para calcular. Muchas de esas ventanas se generan a partir de la orden equivalente de la línea de comandos (capítulo 14), con idénticas opciones y la misma ayuda, y cada herramienta que tiene orden es también un nodo del constructor de flujos (capítulo 13). Por eso lo que hace el botón lo hace la orden `hipso` correspondiente, con los mismos resultados. Todo trabajo largo se puede detener con *■ Detener*, y lo que se hubiera escrito a medias se borra. Mientras corre, la barra de estado y el Registro dicen cuánto falta.

### 4.2 La cabecera y el botón Inicio

#### Abrir (Ctrl+O)

Es el primer icono de la cabecera (una carpeta). Abre nubes de puntos (LAS y LAZ, también COPC, PLY y E57), rásteres, imágenes y capas vectoriales, y los añade a lo que ya está abierto. También se pueden arrastrar a la ventana, y un proyecto `.hipso` arrastrado se abre como proyecto.

#### Guardar vista PNG

El segundo icono (una imagen) guarda en PNG lo que se ve en el área de vistas, con la disposición de ventanas que esté activa.

#### Zoom completo y Zoom anterior

El tercer icono encuadra todo lo cargado, y el cuarto vuelve al encuadre anterior de la planta. Son los mismos botones que *Extensión completa* y *Zoom anterior* de la pestaña *Vista*, repetidos aquí porque se usan constantemente.

#### Buscar comando (Ctrl+K)

El campo de la derecha localiza cualquier herramienta o acción por su nombre, desde *Clasificar puntos* hasta *Curvas de nivel*, y la ejecuta. Es la forma más rápida de llegar a una herramienta cuando no se recuerda su pestaña. Incluye las herramientas de la cinta, los colores de los puntos, el cambio de tema, el constructor de flujos y las páginas de *Inicio*.

#### Inicio

El botón naranja abre una vista a pantalla completa con cinco páginas. La página principal lista los proyectos y archivos recientes y permite abrir nubes, capas o un proyecto. *Ajustes* gobierna la caché (apartado 4.11). *Manual* abre este manual dentro del programa (también con F1). *Atajos de teclado* lista todos los atajos (el capítulo 15 los resume). *Acerca de* muestra la versión y las licencias. El botón *Volver* regresa a la vista de trabajo.

### 4.3 Pestaña Archivo

La pestaña *Archivo* tiene dos grupos: *Archivo*, que gestiona qué está abierto, e *Interfaz*, que gobierna el aspecto.

#### Archivo → Archivo

##### Guardar proyecto

Escribe un archivo `.hipso` con lo que hay abierto. Un proyecto recuerda qué nubes y capas están cargadas, su orden de dibujo, su visibilidad y opacidad, el encuadre de la planta, la disposición de ventanas y la orientación de la vista 3D, de manera que una sesión se retoma donde se dejó. El archivo contiene rutas, no datos: los archivos se quedan donde están, y si alguno se ha movido, solo ese deja de cargarse y el Registro lo dice. De una capa de GeoPackage recuerda también la tabla.

##### Abrir proyecto

Cierra lo que hubiera abierto y carga lo del proyecto elegido. Los proyectos `.elp` de versiones anteriores se siguen abriendo, pero se guardan siempre como `.hipso`.

##### Añadir nubes y capas

Añade archivos a lo abierto, igual que *Abrir* de la cabecera. Al cargar varios a la vez entran todos juntos, y lo que esté en otro sistema de coordenadas se pasa al de la primera nube (apartado 4.10).

##### Recientes

Menú con los diez últimos archivos abiertos. Pulsar uno lo vuelve a abrir, y *Vaciar la lista* los olvida. Está desactivado mientras no se haya abierto ninguno.

##### Cerrar todo

Cierra todas las nubes y capas y deja la ventana vacía.

#### Archivo → Interfaz

##### Registro

Muestra u oculta el Registro, que conserva el historial de lo hecho, los errores y el avance de los procesos largos. Es de solo lectura, de modo que el texto se puede seleccionar a través de varias líneas y copiar.

##### Tema claro y Tema oscuro

Alterna entre el tema claro (el predeterminado) y el oscuro. El botón muestra siempre el tema al que se pasaría.

### 4.4 Pestaña Vista

La pestaña *Vista* tiene cinco grupos y sigue el orden del trabajo: primero se decide qué se ve (*Visualización*), luego se navega (*Navegación*), después se usan las herramientas que actúan sobre lo que la navegación deja a la vista (*Herramientas*) y, al final, se ajustan la vista 3D y el perfil.

#### Vista → Visualización

##### Ventanas

Botón grande con menú que reparte la pantalla entre la planta, el perfil y la vista 3D. Las cuatro disposiciones son *2D* (solo la planta), *2D + perfil* (la planta y, debajo, el perfil de una franja de la nube), *3D* (la vista 3D a pantalla completa, independiente de la planta) y *2D + perfil + 3D* (las tres a la vez). El botón muestra la disposición activa.

##### Color

Menú que elige el atributo con el que se colorean los puntos: *Elevación (azul-rojo)*, *Elevación (tierra)*, *Clasificación*, *Intensidad*, *Retorno*, *ID de fuente*, *Archivo*, *RGB* e *ID de árbol (treeID)*. Este último pinta cada copa de un color distinto a partir del atributo `treeID` que escribe *Segmentar individuos* y solo está disponible si la nube lo trae; los puntos sin árbol quedan en gris. Al cargar una nube con color, el programa la muestra en RGB.

##### Rango Z

Menú con el mínimo y el máximo de elevación entre los que se reparte la rampa de color, y el botón *Ajustar a los datos*, que los devuelve al rango real de la nube.

##### Filtros

Menú con cuatro casillas que filtran los retornos que se dibujan (*Primeros retornos*, *Retornos intermedios*, *Últimos retornos* y *Retornos únicos*) y, al final, *Clases y colores…*, que abre la tabla de clases ASPRS: qué clases se ven y de qué color (apartado 6.2).

##### Puntos (− y +)

Los dos botones agrandan o encogen el punto en la planta y en la vista 3D. Los atajos son las teclas + y −, que cambian el tamaño del perfil si el cursor está sobre él.

##### Máx.

Fija el máximo de millones de puntos que se cargan en pantalla, entre 0,5 y 200. Se escribe el valor y con Intro se recargan las nubes. Una nube mayor se decima de forma uniforme, y la decimación en uso (1 de cada *n*) aparece en la barra de estado. Al acercar, el programa lee a más detalle lo que cabe en la ventana (apartado 4.10).

#### Vista → Navegación

##### Desplazar

Botón grande (atajo X). Con él, arrastrar con el botón izquierdo mueve la planta. El botón central del ratón hace lo mismo con cualquier herramienta.

##### Zoom

Atajo Z. La rueda acerca y aleja. Un clic acerca ×2, un clic derecho aleja y arrastrar dibuja la ventana a la que se acerca.

##### Consultar

Atajo Q. Un clic sobre un punto de la nube da sus atributos, y sobre una capa vectorial, la fila de su tabla (apartado 5).

##### Limpiar marcas

Quita de la vista las mediciones y la caja del perfil.

##### Extensión completa

Encuadra todo lo cargado.

##### Zoom a la capa

Encuadra la capa elegida en *Contenido*. Está desactivado si no hay ninguna.

##### Zoom a la selección

Encuadra los puntos seleccionados o, si la capa activa es vectorial, los elementos seleccionados.

##### Zoom anterior y Zoom adelante

Recorren el historial de encuadres de la planta, como las flechas de un navegador. Cada uno se desactiva cuando no hay a dónde ir. El botón central del ratón hace también *Zoom anterior*.

##### Refrescar 3D

Vuelve a encuadrar la vista 3D en lo que se ve en la planta. Es útil cuando hay capas muy separadas entre sí.

#### Vista → Herramientas

##### Densidad

Calcula los puntos por metro cuadrado de un recuadro dibujado en la planta (apartado 5).

##### Perfil

Dibuja una caja sobre la planta (un clic en el extremo izquierdo, otro en el derecho y un tercero en el borde de la caja) y abre su perfil, una vista lateral con todos los puntos de la franja (apartado 5).

##### Medir

Atajo W. Da distancias y áreas sobre la planta (apartado 5). Pulsarlo de nuevo devuelve la herramienta *Zoom*.

##### TIN

Atajo T. Dibuja un TIN sombreado de las nubes en la planta; mientras está activo, los puntos se ocultan. El engranaje contiguo abre dos opciones: la *Equidistancia* de las curvas de nivel y las clases con las que se construye el TIN (*suelo* o *todas las clases*).

##### Curvas de nivel

Dibuja curvas de nivel sobre el TIN, con la equidistancia fijada en el engranaje.

#### Vista → Vista 3D

##### Exageración

Multiplica la escala vertical de la vista 3D, de × 0,1 a × 100, tanto para nubes como para superficies ráster. Se arrastra el valor o se escribe.

##### Enlazar con la planta

Con *2D + perfil + 3D* activo, hace que la vista 3D siga lo que se ve en la planta. Apagada, la vista 3D se mueve por su cuenta.

#### Vista → Perfil

##### Ancho

Ancho de la caja del perfil, de 0,5 a 1000 m.

##### Cuadrícula

Dibuja líneas horizontales con las cotas en el lado izquierdo del perfil.

##### Puntos (− y +)

Tamaño de los puntos del perfil, independiente del de la planta y el 3D.

##### Sin decimar

El perfil lee de los archivos todos los puntos de su caja, hasta 6 millones, en segundo plano. Es lo que garantiza que el perfil nunca decime: lo que se ve es lo que hay. Desmarcada, el perfil usa los puntos de la pantalla.

### 4.5 Pestaña LiDAR

La pestaña *LiDAR* reúne lo que parte de una nube de puntos. Sus seis grupos siguen el orden natural de un proyecto: se *prepara* la nube (clasificarla, proyectarla, limpiarla), se comprueba su *calidad*, se derivan las *superficies*, se mide la vegetación en *ALS* (escáner aéreo) o en *TLS* (escáner terrestre) y se estudian las *infraestructuras*. Cada herramienta se explica con detalle en el apartado que se cita.

#### LiDAR → Preparar

##### Clasificar

Botón grande. Clasifica ruido, suelo, edificios, vegetación por alturas, cables y postes, carreteras y agua, con varios métodos de suelo, reglas propias y recetas guardadas en un archivo. Es la herramienta central de la preparación, porque casi todo lo demás (MDT, CHM, métricas, árboles) parte de la clase de suelo. Véase el apartado 7.1, *Clasificar*.


La ventana *Clasificar puntos (ruido, suelo, edificios)* tiene tres pestañas (*Clasificación*, *Reglas* y *Teselas y rendimiento*), una sección *Guardar en* (carpeta de resultados; vacía, junto a cada nube), el nombre de salida, el formato de salida y los botones *Guardar receta…* y *Cargar receta…*. Cada hoja se clasifica por separado y se escribe en su propio archivo. Las clases se aplican en un orden fijo (solape, ruido, suelo, edificios, carreteras y agua, vegetación, cables y postes, reglas), y cada paso solo ve lo que dejaron los anteriores.

##### Pestaña Clasificación

| Control | Qué hace |
|---|---|
| Apartar el solape (clase 12) | Donde varias pasadas cubren una celda, la de menor ángulo de barrido medio se queda con ella y los puntos de las otras pasan a la clase 12, que los pasos siguientes no usan. Requiere nubes con ángulo de escaneo. |
| Solape: *Celda (m)* | Lado de la celda con la que se decide qué pasada se queda cada zona (5 m por defecto). |
| Ruido: *Sin filtro*, *Puntos aislados*, *Estadístico (SOR)*, *Puntos bajos (ELM)* | Método con el que se marca el ruido (clase 7). *Puntos aislados* marca los puntos con pocos vecinos en celdas de 3×3×3; *Estadístico* los que están a más desviaciones típicas de lo normal de la distancia media a sus vecinos; *Puntos bajos* los puntos más bajos de una celda que quedan por debajo de todo lo que tienen alrededor. |
| Ruido aislado: *Celda xy*, *Celda z* | Tamaño horizontal y vertical de las celdas en las que se cuentan los vecinos (4 m por defecto). |
| Ruido estadístico: *Vecinos (k)* y *Desviaciones (m)* | Con cuántos vecinos se calcula la distancia media de cada punto (10) y cuántas desviaciones típicas por encima de la media hacen falta para llamarlo ruido (3). |
| Puntos bajos: *Celda* y *Bajada mínima (m)* | Lado de la celda (10 m) y cuántos metros por debajo del resto de su celda y de las vecinas debe quedar el punto más bajo para ser ruido (3 m). |
| Suelo: método | *Kraus y Pfeifer (iterativo)*, *MCC (curvatura multiescala)*, *CSF (simulación de tela)*, *PMF (morfológico progresivo)*, *SMRF (morfológico simple)*, *Axelsson (TIN progresivo)* o *Clase 2 existente* (no busca suelo: usa el que trae la nube). Véase *Suelo (clase 2)*. |
| Kraus y Pfeifer | *Celda del filtro* (tamaño de las superficies intermedias, 5 m), *Iteraciones* (5). Los parámetros de la función de peso se dan por la línea de comandos. |
| MCC | *Espaciado (s)* (celda del dominio de escala, aproximadamente el espaciado de los puntos, 1,5 m) y *Umbral de curvatura (t)* (0,3 m). |
| CSF | *Resolución de la tela* (distancia entre partículas, 0,5 m), *Rigidez* (de 1, terreno abrupto, a 3, llano) y *Umbral* (altura sobre la tela hasta la que un punto es suelo, 0,5 m). |
| PMF | *Ventana máxima* (m) y *Pendiente* del terreno que tolera. |
| SMRF | *Celda*, *Ventana máxima (radio)* (mayor que el edificio más grande), *Pendiente máxima del terreno*, *Umbral de altura* y *Factor por pendiente* (lo que se suma al umbral por unidad de pendiente). |
| Axelsson | *Celda de las semillas* (mayor que el edificio más grande), *Ángulo de iteración (°)*, *Distancia de iteración (m)* y *Tolerancia llana (m)* (cerca del plano de un triángulo es suelo sea cual sea el ángulo). |
| Edificios: *Clasificar edificios (6) y vegetación alta (5)* | Quita los tejados del suelo y clasifica lo que está sobre él: edificios (6) y vegetación alta (5). |
| Edificios: *Altura mínima* | Altura sobre el terreno a partir de la cual un punto es edificio o árbol (2,5 m). |
| Edificios: *Edificio más ancho* | El mayor edificio esperado (60 m); fija la ventana con la que se separa el suelo de los tejados. |
| Edificios: *Área mínima* | La menor planta, en unidades cuadradas del mapa, que se acepta como edificio (40). |
| Carreteras (11) | Suelo liso, llano y de intensidad baja. *Intensidad máxima* (0: relativa al suelo), *Rugosidad máxima (m)* y *Pendiente máxima (°)*. |
| Agua (9) | Superficies llanas extensas de intensidad muy baja y retorno único. *Intensidad máxima* (0: relativa al suelo), *Superficie mínima (m²)* y *Desviación máxima de la cota (m)*. |
| Vegetación por alturas | Baja (3), media (4) y alta (5) según la altura sobre el suelo. *Baja hasta (m)* y *Media hasta (m)* fijan los dos cortes. |
| Cables (14) | Líneas casi horizontales altas. *Altura mínima (m)*, *Radio del entorno (m, 0 = según la densidad)* y *Linealidad mínima (0–1)*. |
| Postes y torres (15) | Estructuras verticales estrechas y aisladas. *Altura mínima (m)*, *Distancia de unión (m)*, *Anchura máxima (m)*, *Altura mínima de la estructura (m)* y *Cabeza de apoyo: longitud máxima (m, 0 = no)*. |

##### Pestaña Reglas

Cada regla cambia la clase de los puntos que cumplen todas sus condiciones, al final y en orden, de manera que una regla ve lo que hizo la anterior. Sirve para los estratos de vegetación (una regla por altura) y para casos que los métodos no cubren. El ruido (7 y 18) no se toca salvo que se pida en *De clases*.

| Control | Qué hace |
|---|---|
| De clases | Clases de partida: una lista (1,2) o, con «~», todas menos esas (~7). Vacío: todas menos ruido. |
| Eco: *Único*, *Primero*, *Intermedio*, *Último* | Número de retorno de los puntos afectados. |
| Ángulo \|a\| de | Rango del ángulo de escaneo absoluto. |
| Altura sobre el suelo | Rango de altura de los puntos afectados (m). |
| Intensidad | Rango de intensidad. |
| Forma del entorno | Rangos de *Pendiente*, *Planaridad*, *Linealidad*, *Esfericidad*, *Rugosidad* y *Vecinos*, calculados con los vecinos que caen dentro del *Radio*. |
| A la clase | Clase que reciben los puntos que cumplen todo. |
| Añadir regla, Quitar la última | Añade o quita una regla; las reglas se ejecutan en el orden de la lista. |

##### Pestaña Teselas y rendimiento

| Control | Qué hace |
|---|---|
| Tamaño de tesela | Lado (m) de las teselas en que se divide cada hoja para clasificarla en paralelo (250 por defecto). |
| Búfer | Margen (m) que se lee alrededor de cada tesela para que sus bordes salgan como saldrían de toda la hoja (30 m). |
| Hilos | Núcleos que se usan; 0 es todos. |

##### Receta y formato

*Guardar receta…* escribe esta clasificación en un archivo de texto (se aplica otra vez con *Cargar receta…* o con `hipso classify --recipe`); si la ventana tiene algún parámetro sin opción equivalente en la línea de comandos, el programa lo avisa y ese parámetro vuelve a su valor por defecto al cargarla. *Formato de salida* ofrece LAS o LAZ, y la variante «más rápido y ligero en procesos muy grandes».

Todas las opciones equivalentes de la línea de comandos, con sus valores por defecto, están en *Referencia de herramientas*, orden `hipso classify`.

##### Asignar proyección

Da a una nube sin sistema de coordenadas el de otra nube o un código EPSG, sin tocar los puntos, solo la cabecera. Se usa cuando el archivo no declara el suyo (el aviso aparece en *Información*). Véase el apartado 7.1.


| Control | Qué hace |
|---|---|
| Lista de nubes | Indica con ✔ las nubes cargadas que ya tienen sistema de coordenadas y con ✖ las que no. |
| Copiar de otra nube | Toma el sistema de coordenadas de una nube que lo tiene. |
| Código EPSG | Escribe el código de un sistema, por ejemplo 25830. |
| Reemplazar también el de las nubes que ya lo tienen | Por defecto solo se asigna a las nubes sin sistema; marcada, sustituye también el existente. |

Se escribe una copia nueva (`…_crs`) y los puntos no cambian: solo cambia la cabecera. La orden es `hipso crs`.

##### Exportar puntos

Saca los puntos a otro formato: LAZ (por defecto), LAS, texto con las columnas que se elijan o shapefile 3D, con filtro por clases, uno de cada *n* puntos y, si se marca *Normalizar*, alturas sobre el suelo. Véase el apartado 7.1, *Exportar puntos*. El panel *Edición* tiene otro *Exportar puntos…* que exporta solo la selección.


| Control | Qué hace |
|---|---|
| Formato | *LAZ*, *LAS*, *Texto (ASCII)*, *Shapefile 3D* o *COPC*. |
| Salida | Nombre del archivo (o carpeta, con varias nubes); vacío, junto a cada nube. |
| Normalizar | Exporta alturas sobre el suelo: a la cota de cada punto se le resta la del MDT de su nube. Sin MDT elegido, se usa el último calculado para cada nube, y los puntos fuera del MDT no se exportan. |
| Columnas | En el texto, qué columnas se escriben y en qué orden (x, y, z, intensidad, retorno, número de retornos, clase, fuente, rojo, verde y azul). |
| Separador | Coma, tabulador, espacio o punto y coma. |
| Línea de cabecera | Escribe los nombres de las columnas en la primera línea. |
| Decimales | Decimales de las coordenadas del texto. |
| Un punto de cada | Conserva un punto de cada tantos. |
| Clases | Qué clases se exportan (2,5 o ~7,18). |
| Retornos, elevaciones mínima y máxima | Filtran por número de retorno y por rango de cota. |

La orden es `hipso export` (y `hipso copc` para el formato COPC).

##### Recortar con polígonos

Recorta las nubes con una capa de polígonos: una nube por polígono, o una sola ajustada al perímetro de todos, con búfer opcional. Véase el apartado 9, *Recortar con polígonos*.


| Control | Qué hace |
|---|---|
| Capa de polígonos | Un archivo, o una de las capas ya cargadas. |
| Una nube por cada polígono | Escribe un archivo por polígono. |
| Una sola nube ajustada al perímetro de todos | Escribe un archivo con los puntos de todos los polígonos. |
| Campo con el nombre de cada archivo | Campo cuyo valor nombra el archivo de cada polígono; vacío, el número del polígono. |
| Búfer sobre el perímetro (m) | Zona de seguridad alrededor del borde; con un valor negativo, estrecha el recorte. |
| Un punto va solo al primer polígono que lo contiene | Evita que un punto se repita cuando los polígonos se solapan. |

Se escribe una copia nueva (`…_recorte…`) y la original no cambia. La orden es `hipso clip`.

##### Adelgazar nube

Reduce la densidad sin cambiar la extensión, para igualar nubes de coberturas distintas o aligerar las demasiado densas. Véase el apartado 7.1.


| Control | Qué hace |
|---|---|
| Método | *Densidad objetivo* (al azar hasta unos puntos por m²), *Un punto por celda* (el más cercano al centro, el más bajo, el más alto o el de un percentil de la elevación), *Celdas 3D* (conserva paredes y pendientes), *Conservar un punto de cada n*, *Eliminar un punto de cada n* o *Muestreo al azar (fracción)*. |
| Puntos por m² que quedan | Con la densidad objetivo. Una nube que ya es menos densa se deja como está. |
| Celda (m) | Lado de la celda en los métodos por celda. |
| Percentil | Del 0 al 100 (50 es la mediana) en el método por percentil. |
| Celda horizontal y vertical (m) | Tamaño del vóxel en las celdas 3D. |
| n | Cada cuántos puntos se conserva o se quita uno. |
| Fracción que se conserva (0–1) | En el muestreo al azar. |
| Semilla | Misma semilla, mismo resultado, en los métodos al azar. |

Se escribe una copia nueva por hoja (`…_delgada_…`). La orden es `hipso thin`.

##### Alturas sobre suelo

Calcula la altura de cada punto sobre el suelo. Véase el apartado 6.3.


| Control | Qué hace |
|---|---|
| Guardar la altura como atributo «HeightAboveGround» y conservar las cotas | La nube sigue con sus cotas y gana un atributo con la altura sobre el suelo. |
| Poner la altura en lugar de la cota (nube normalizada) | Sustituye la cota por la altura sobre el suelo. |
| Celda del suelo (m) | Tamaño de la superficie de suelo (0,5 m por defecto). |
| Clase del suelo | Clase de los puntos que forman el suelo (2). |

Un TIN de los puntos de suelo da el suelo bajo cada punto, de modo que no hace falta un MDT aparte. La orden es `hipso hag`.

##### Quitar duplicados

Elimina los puntos repetidos y escribe una nube nueva. Véase el apartado 6.4.


Quita los puntos que están en el mismo sitio (X, Y y Z iguales). Las casillas añaden atributos que también deben coincidir para que dos puntos cuenten como repetidos.

| Control | Qué hace |
|---|---|
| Misma clase, Mismo tiempo GPS, Misma intensidad | Exigen que coincidan además esos atributos. |
| Mismo número de retorno, Mismo número de retornos | Exigen que coincida el retorno y el total de retornos. |
| Mismo ID de fuente (pasada) | Exige que sean de la misma pasada. |
| Quitar todas las copias | Sin marcar, se deja un punto de cada grupo; marcada, se quitan todos. |

La orden es `hipso dedup`.

##### Colorear con imagen

Da a cada punto el color del píxel de una imagen del mismo sitio, normalmente una ortofoto. Véase el apartado 7.1.


| Control | Qué hace |
|---|---|
| Imagen | Ortofoto (`.tif` o `.jp2`), en el mismo sistema de coordenadas que la nube. |
| Bandas: rojo, verde y azul | Qué banda de la imagen da cada color (1, 2 y 3 por defecto). |
| Los puntos fuera de la imagen conservan su color | Si no se marca, quedan en negro. |

Una nube sin color (formatos 0, 1 y 6) se guarda en formato 7. Si la nube es de formato 8 y la imagen tiene banda de infrarrojo cercano, también se asigna. La orden es `hipso colorize`.

##### Normalizar intensidad

Corrige la intensidad por la distancia al escáner (Höfle y Pfeifer, 2007), de manera que la misma superficie tenga el mismo valor en el borde de una pasada que en el centro. Véase el apartado 7.1.


| Control | Qué hace |
|---|---|
| Trayectoria del vuelo | Archivo de texto («tiempo x y z», con el tiempo GPS de la nube): es lo más exacto, porque da la distancia real al escáner de cada punto. |
| Altura de vuelo constante y ángulo de escaneo | Alternativa cuando no hay trayectoria: se estima la distancia con la altitud del vuelo y el ángulo de cada punto. |
| Altitud del vuelo (m) | Con la alternativa anterior, en el mismo sistema de alturas que la nube. |
| Exponente a | 2 para suelo y copas, en torno a 3 para cables y bordes. |
| Distancia de referencia R_ref (m) | Distancia a la que se normaliza; 0 es la media de la nube. |

Cada intensidad se multiplica por (R / R_ref)^a (Höfle y Pfeifer, 2007). La orden es `hipso intensity`.

##### Imagen de intensidad

Genera, por cada nube, un GeoTIFF en tonos de gris con la intensidad media por celda, útil como fondo para digitalizar. Véase el apartado 7.1.


| Control | Qué hace |
|---|---|
| Celda de la imagen (m) | Tamaño del píxel; el botón *Celda automática* pone tres veces la separación media de los puntos. Entre 2 y 4 veces deja pocas celdas vacías y conserva el detalle. |
| Rellenar huecos de hasta (celdas) | Las celdas sin puntos se rellenan con la media de sus vecinas hasta ese número de celdas; los huecos mayores quedan vacíos y transparentes. |
| Retornos | *Primeros*, *Últimos*, *Todos*, o una lista (F, L, A o números como 1,2). |
| Clases | *Todas*, *Suelo*, *Vegetación*, *Edificios* o una lista (2,5 o ~7,18); vacío deja fuera el ruido. |
| Unir las hojas en un mosaico | Escribe además un mosaico con todas las hojas. |

Se escribe un GeoTIFF por nube con la intensidad media por celda.

#### LiDAR → Calidad

##### Comparar nubes

Escribe los puntos de una nube que están a más de una distancia de todos los de otra de referencia, y da la distancia media, el RMS y la compleción. Detecta cortas, cambios de obra o crecimiento entre dos vuelos. Véase el apartado 7.1.


| Control | Qué hace |
|---|---|
| Nube de referencia | La nube con la que se compara la cargada. |
| Distancia mínima de cambio (m) | Un punto cuenta como cambiado si está a más de esta distancia de todos los de la referencia. |
| Vecinos cambiados que necesita un punto | Los puntos de cambio aislados se descartan: cada uno necesita este número de vecinos también cambiados. |

Escribe los puntos que han cambiado y da la distancia media, el RMS y la compleción. La orden es `hipso compare`.

##### Ajustar nubes

Mueve una nube para que encaje con otra de referencia mediante el punto más cercano iterativo (ICP), solo desplazando o desplazando y girando. Resuelve vuelos del mismo sitio con un desfase sistemático. Véase el apartado 7.1.


| Control | Qué hace |
|---|---|
| Nube de referencia | La nube sobre la que se ajusta la cargada. |
| Adelgazar para calcular (m) | Separación de los puntos que se usan para hallar el movimiento (1 m por defecto); más grande es más rápido. |
| Distancia horizontal máxima (m) y vertical máxima (m) | Un punto solo se empareja con otro de la referencia dentro de estas distancias. |
| Iteraciones máximas | Tope de iteraciones del ajuste. |
| Parar con distancia media menor de (m) | El ajuste termina cuando el movimiento baja de este valor. |
| Solo desplazar (sin girar) | El ajuste solo traslada. |
| Punto a plano | Mide la distancia al plano tangente del punto de referencia: mejor en superficies y converge antes. |
| Pares que se usan, los más cercanos (%) | Descarta los pares más lejanos para que los valores atípicos no desvíen el ajuste. |

Usa los últimos retornos de ambas nubes y escribe una copia (`…_ajustada`). La orden es `hipso fit`.

##### Entre pasadas

Produce un ráster con la diferencia de altura entre pasadas de vuelo y el porcentaje de celdas por encima de 8 y 16 cm. Es un control de calidad que debe hacerse antes de modelar. Véase el apartado 7.1.


| Control | Qué hace |
|---|---|
| Celda (m) | Tamaño de la celda en la que se comparan las pasadas. |
| Retornos | *Últimos y únicos* (por defecto), *Todos* o *Solo únicos*. |
| Cada nube cargada es una pasada | Usa cada archivo como una pasada, en lugar del ID de fuente. |
| Las pasadas salen de los huecos en el tiempo GPS, mayores de (s) | Útil cuando el ID de fuente no distingue las pasadas (todos los puntos con el mismo valor). |

Guarda un ráster (`…_pasadas`) con la mayor diferencia de elevación media entre las pasadas que cubren cada celda, y da el porcentaje de celdas por encima de 8 y 16 cm. La orden es `hipso swath`.

##### Líneas de vuelo

Encuentra las líneas de vuelo de una nube aérea a partir de los huecos del tiempo GPS, y escribe una tabla y los contornos de cada una. Véase el apartado 7.1.


| Control | Qué hace |
|---|---|
| Hueco que separa dos líneas (s) | El tiempo GPS sin puntos a partir del cual se corta una línea (30 s por defecto). |
| Puntos mínimos de una línea | Un tramo con menos puntos no es una línea (1 000). |
| Escribir el número de línea en | *ninguna parte*, *ID de fuente* (hasta 65 535) o *dato de usuario* (hasta 255). |
| Capa de polígonos con el contorno de cada línea | Escribe un shapefile con la huella de cada línea. |
| Tabla de las líneas (CSV) | Escribe la tabla con inicio, fin, duración, puntos, caja y ángulo medio de cada línea. |

La orden es `hipso flightlines`.

##### Densidad de Voronoi

Densidad de puntos por celda de Voronoi en lugar de por cuadrícula. Véase el apartado 7.1.


| Control | Qué hace |
|---|---|
| Celda del ráster (m) | Tamaño del ráster de salida (5 m por defecto). |
| Umbral (puntos/m²) | Densidad mínima que debe tener una celda para cumplir. |
| Hueco de datos: celda de Voronoi mayor que (m²) | Una celda de Voronoi mayor que esta superficie se considera un hueco de datos. |
| Retornos | F primeros y únicos (los pulsos), L últimos, A todos, o los números de retorno. |
| Clases | Una lista como 2,5 o, con «~», todas menos esas; vacío deja fuera el ruido (7 y 18) y los puntos retenidos. |
| Ráster de cumple / no cumple | Escribe el ráster que marca qué celdas alcanzan el umbral. |
| Tabla (CSV) | Escribe la tabla de resultados. |

La orden es `hipso voronoi-density`.

#### LiDAR → Superficies

##### MDT

Botón grande. Calcula el modelo digital del terreno a partir de los puntos de suelo, por promedio de las celdas, TIN o ponderación por el inverso de la distancia. Véase el apartado 7.2.


| Control | Qué hace |
|---|---|
| Resultado | Nombre del archivo que se escribe; por defecto lleva la celda, el método y el método de suelo. |
| Celda | Tamaño de celda del MDT (m; 1 por defecto). |
| Método | *Triangulación (TIN)*: superficie continua, el mejor equilibrio; *Ponderado por distancia (IDW)*: suave y rápido; *Media por celda*: la más rápida, en una sola pasada. |
| Suelo | *Clase 2 que trae la nube* (por defecto), o un método que vuelve a buscar el suelo solo para este MDT (*Kraus y Pfeifer*, *MCC*, *CSF*, *PMF*, *SMRF* o *Axelsson*), con los mismos parámetros que *Clasificar*. Sirve para probar varios modelos con distinta clasificación; no modifica la nube. |
| Suavizado (m, 0 = sin suavizar) | Ventana con la que se suaviza el MDT, con *Media*, *Mediana* o *Gauss*. |
| Líneas de rotura | Capa de líneas (bordes de camino, taludes, cunetas, orillas) que el MDT sigue en lugar de redondear. Con cotas (PolylineZ) fijan la altura; sin cotas, toman el suelo más bajo junto a ellas. Solo con TIN o IDW. |
| Hojas vecinas hasta (m, 0 = cada hoja sola) | Calcula cada hoja con las que tiene a esa distancia y se queda solo con lo suyo: sin costura en los bordes, a costa de leer también las vecinas. |
| Hacer además un mosaico con todas las hojas | Escribe también un mosaico de los MDT de las hojas. |

La orden es `hipso dtm`.

##### MDS / CHM

Botón grande. Calcula el modelo digital de superficie y el modelo de alturas de la vegetación (el CHM, la altura de lo más alto sobre el terreno en cada celda, tanto de árboles como de arbustos o cultivos), con el método del punto más alto o la pila de TIN sin picos. Véase el apartado 7.2.


| Control | Qué hace |
|---|---|
| Restar el MDT (altura sobre el suelo, CHM) | Marcada, resta el MDT y sale la altura sobre el suelo (CHM); sin marcar, la elevación absoluta (MDS). Cada hoja usa el último MDT hecho con ella, y si falta, se hace el MDS y el programa avisa de que hay que calcular antes el MDT. |
| Celda | Tamaño de celda (m; 1 por defecto). |
| Método | *Triangulación (TIN) de los puntos más altos*: superficie continua; *Máximo por celda*: el más rápido, en una sola pasada; *Ponderado por distancia (IDW) de los puntos más altos*; *Sin huecos (pit-free, Khosravipour et al., 2014)*, que trabaja con alturas sobre el suelo y por eso necesita el MDT. |
| Arista máxima | Con el método sin huecos, el mayor lado de triángulo que se admite. |
| Rellenar celdas vacías | Rellena las celdas sin puntos con las vecinas. |
| Suavizado (m) | Ventana de suavizado, con *Media*, *Mediana* o *Gauss*. |
| Hacer además un mosaico con todas las hojas | Escribe también un mosaico de las hojas. |

Las órdenes son `hipso chm` y `hipso dsm`.

##### Cambio de la cubierta vegetal

Botón grande. Compara los CHM de dos fechas y mide la pérdida y la ganancia de altura y de cubierta vegetal. Véase el apartado 7.3.


| Control | Qué hace |
|---|---|
| CHM de la primera y de la segunda fecha | Los dos modelos de alturas, que deben coincidir celda a celda (*Raster → Igualar rásteres*). |
| Años entre fechas | Opcional, para las tasas anuales. |
| Cambio mínimo | Un cambio de altura menor que este valor es ruido (1 m). |
| Cubierta desde | Altura a partir de la cual una celda cuenta como cubierta vegetal (2 m). |
| Mancha mínima de pérdida (celdas) | Las manchas de cubierta perdida menores no se comunican (10). |
| Unidades de gestión | Opcional: capa para resumir por unidad, con *Campo del nombre*. |
| Emparejar los individuos de las dos fechas y medir su crecimiento | Detecta los árboles de las dos fechas y los empareja; con *Altura mínima*, *Ventana a + b·h*, *Suavizar* y *Mismo individuo si dista menos de*. |
| Salida | Manchas (shp), manchas (csv) y base del nombre. |

La orden es `hipso change`.

#### LiDAR → ALS

##### Extraer muestras

Botón grande. Corta las nubes por unidades de muestreo (generadas, dibujadas o de un archivo) y calcula sus métricas. Véase el apartado 7.3.


La ventana se divide en dos partes, *1 · De dónde salen las unidades de muestreo* y *2 · Qué se obtiene*.

##### De dónde salen las unidades de muestreo

| Control | Qué hace |
|---|---|
| Archivo vectorial | Usa un archivo: polígonos (se usan tal cual) o puntos con el centro de cada unidad, cuya forma y tamaño se dan abajo. |
| Generarlas (muestreo) | Las crea el programa, al azar o en malla, y guarda su ubicación como capa vectorial. |
| AOI dibujadas | Usa los polígonos dibujados con la herramienta AOI. |
| Forma | *Circular* o *Cuadrada / rectangular*. |
| Campo con el número de unidad de muestreo | Campo que nombra cada unidad. |
| Campo con el lado X, lado Y, o lado fijo (m) | Cómo se da el tamaño de las unidades cuadradas o rectangulares cuando las regiones son puntos. |
| Campo con el radio, o radio fijo (m) | Ídem para las circulares. |
| Solo las unidades de muestreo con CAMPO=VALOR | Filtra las regiones por el valor de un campo. |
| Partir las multipartes | Una unidad por cada parte, nombradas nombre_p1, nombre_p2… |
| Lado (m), Radio (m) | Tamaño de las unidades generadas. |
| Aleatorio | Número de unidades, *Distancia mínima entre centros* (0: el doble del tamaño) y *Semilla* (la misma semilla da las mismas unidades). |
| Sistemático (malla regular) | *Separación en X* y *en Y* (m) entre centros. |
| Limitar a los polígonos de | Las unidades generadas se quedan dentro de estos polígonos (un rodal, por ejemplo). |

##### Qué se obtiene

| Control | Qué hace |
|---|---|
| Capa vectorial con las unidades de muestreo y sus métricas | Escribe la capa de unidades con sus métricas. |
| Nubes recortadas | *ninguna*, *una por unidad de muestreo* o *una con todas*. |
| Clases, Retornos | Qué puntos se usan (2,5 o ~7,18; F, L, A o números). |
| Alturas sobre el suelo | Cada nube menos su MDT (recomendado). Marcada, las métricas salen con alturas normalizadas y hace falta el MDT de cada nube. |
| Puntos en el solape de dos unidades | Se cuentan en todas las que los contienen, o se asignan solo a la primera. |
| Métricas | Todas, o las que se marquen en el selector (con buscador, *Marcar las visibles* y *Quitar todas*). La tabla lleva solo las columnas marcadas en el orden en que se marcaron; el nombre, el área y la fecha de captura van siempre. |

La orden es `hipso clip`.

##### Métricas por celda

Botón grande. Métricas de altura y densidad de la vegetación por celda. Véase el apartado 7.3.


| Control | Qué hace |
|---|---|
| Celda | Lado de las celdas en las que se calculan las métricas (15 m por defecto). |
| Altura de corte (cobertura) | Altura a partir de la cual un retorno cuenta para las métricas de cobertura (2 m). |
| Métricas de intensidad (_intensity) | Escribe además un archivo con las métricas de la intensidad. |
| Métricas por estratos (_strata) | Escribe además un archivo con las métricas por estratos de altura. |
| Métricas de estructura (_estructura) | Escribe además la fracción de huecos, el índice de área foliar, la diversidad de alturas del follaje, la complejidad vertical y la altura de la base de la copa. |
| Capa del perfil, Altura del perfil, Hueco bajo, Extinción k, Base de copa sobre, proporción | Parámetros de las métricas de estructura: el grosor de las capas del perfil, la altura bajo la que se cuenta como hueco (1 m), el coeficiente de extinción del índice foliar (0,5) y el suelo (2 m) y la proporción (10 %) con los que se busca la base de la copa. |
| Memoria máxima (MB) | Memoria que se dedica a las alturas de los puntos (512 MB por defecto); las nubes mayores requieren más pasadas. |

Sin MDT para las hojas, el programa avisa de que hay que calcularlo antes. La orden es `hipso metrics`.

##### Modelos lineales

Botón grande. Relaciona una variable medida en campo (volumen, biomasa, área basimétrica…) con las métricas LiDAR de las mismas unidades de muestreo, para estimarla en toda la superficie, y valida el modelo por validación cruzada. Véase el apartado 7.3.


| Control | Qué hace |
|---|---|
| Capa o tabla de unidades de muestreo | La capa que da *Extraer muestras* (modo solo métricas, shapefile): un polígono por unidad con sus métricas. Puede estar ya cargada. |
| Datos de campo | Excel o CSV con una fila por unidad de muestreo; su nombre debe coincidir con el de la unidad. |
| Muestra en métricas, en campo | Columnas que nombran la unidad en cada tabla. |
| Variable a estimar | Columna de la tabla de campo que se estima. |
| Transformación | Ninguna, logaritmo de la respuesta, o logaritmo de la respuesta y de las métricas. |
| Variables del modelo | *Automático* (el programa elige las métricas) o *Elegir variables* (se marcan en una tabla). |
| Criterio | AICc o BIC para la elección automática. |
| Variables como mucho | Máximo de métricas del modelo (0: una por cada diez unidades, entre dos y cinco). |
| Colinealidad máxima \|r\| | Correlación máxima entre dos métricas; si se supera, se deja la más relacionada con la variable. |
| VIF máximo | Factor de inflación de la varianza máximo que se admite. |
| Probar también las métricas de intensidad | Las incluye entre las candidatas; la intensidad de casi todos los vuelos no está calibrada entre pasadas. |
| Elegir variables… | Analiza la normalidad de la variable y de las métricas y las muestra para marcarlas (*Sugeridas*, *Ninguna*). |
| Hacer el mapa de la variable | Estima la variable en todas las celdas, con las tablas de *Métricas por celda* (deben calcularse igual que las de las unidades de muestreo). |
| Guardar también el intervalo de predicción del 95 % | Escribe la semianchura del intervalo. |
| Estimar también las celdas fuera del rango | Por defecto esas celdas quedan vacías. |
| Ajustar y validar, Gráficas | Ajusta el modelo y lo valida dejando fuera una unidad cada vez; las gráficas de diagnóstico muestran lo ajustado y los residuos. |

La orden es `hipso model`.

##### Segmentar individuos

Detecta los individuos y sus copas en un modelo de alturas. Véase el apartado 7.3.


La ventana se organiza en cinco pasos.

| Control | Qué hace |
|---|---|
| Ráster | El CHM que se segmenta; con varias nubes cargadas, se segmenta el de cada una y cada hoja da su resultado. |
| Individuos (base del nombre) | Nombre de los archivos de salida. |
| 1 · Ventana del suavizado (m, 0 = sin suavizar) | Suaviza el CHM antes de buscar cimas, con *Media*, *Mediana* o *Gauss*. |
| 2 · Origen | *Buscar en el CHM* (máximos locales) o *Archivo de ápices (x,y)…* (cimas dadas a mano). |
| Altura mínima | Altura mínima de una cima para contarla como árbol (2 m). |
| Ventana (diámetro) = a + b·h + c·h² | Diámetro de la ventana de búsqueda de cimas como función de la altura (b = c = 0: fija); *Circular* o *Cuadrada*. |
| 3 · Copas: método | *Dalponte y Coomes 2016* (crecimiento de regiones), *Silva et al. 2016* (Voronoi acotado por altura) o *Cuencas (watershed)*. |
| th_seed, th_cr, max_cr | Dalponte: proporción de la altura de la cima y de la altura media de la copa por encima de la cual un píxel se une, y alcance de la copa en píxeles. |
| max_cr_factor, exclusión | Silva: radio de la copa como proporción de la altura, y proporción de la altura por debajo de la cual se descartan los píxeles. |
| tolerancia (m), ext (píxeles) | Cuenca: escalón mínimo de altura entre copas y radio de búsqueda de picos. |
| 4 · Resultados | Marca qué se escribe además de las cimas: *Ráster de copas*, *Polígonos de copa*, *Ápices (shapefile PointZ)*, *GeoPackage* (ápices y copas en un archivo), *Métricas de la nube en cada copa* (más lento, con alturas normalizadas) y *Nube LAS/LAZ con atributo treeID por punto*. |
| Margen hacia las vecinas (m) | Con varias hojas, cada árbol una sola vez: se busca con lo que tienen las vecinas y se deja a la hoja de su cima. |
| 5 · Inventario de campo | Archivo con X_arbol, Y_arbol, Ht, Muestra, X_Muestra e Y_Muestra, para comprobar la detección; con *Radio de la unidad de muestreo (m)* y *Distancia de emparejado (m)* (una detección coincide con un árbol de campo a esa distancia como máximo). |

La orden es `hipso trees`.

##### Análisis por objetos

Agrupa el modelo de alturas en objetos y los clasifica (OBIA). Véase el apartado 7.3.


| Control | Qué hace |
|---|---|
| Ráster (CHM) | El CHM, de un archivo o de las capas cargadas. Todas las capas deben coincidir celda a celda. |
| Otras capas ráster | Capas adicionales (intensidad, retornos, rugosidad…) con las que describir los objetos. |
| 1 · Rellenar celdas sin puntos hasta (celdas) | Evita huecos entre objetos (0: no). |
| 1 · Suavizar con radio (celdas) | Reduce el ruido de sal y pimienta (0: no). |
| 1 · Rugosidad del CHM (radio, celdas) | Añade la rugosidad local del CHM como capa (0: no). |
| 2 · Fusión de regiones | Crea los objetos fusionando manchas mientras el coste sea menor que la escala al cuadrado. |
| Escala | Mayor, objetos más grandes (conviene probar entre 5 y 15). |
| Peso de la forma, compacidad | Peso de la forma frente al valor (0 a 0,9) y de la compacidad frente a la suavidad dentro de la forma (0 a 1). |
| Peso de la rugosidad | 1 pesa como las demás capas; 0 la deja fuera. |
| 2 · Rango de valores del CHM | Alternativa a la fusión: un objeto es el conjunto de celdas contiguas del CHM entre dos valores (*Entre*), con *unir en diagonal*. |
| Objeto mínimo (celdas) | Los objetos menores se unen a un vecino. |
| 3 · Clasificación | *Solo describir*, *Con reglas* (archivo de texto), *Con muestras (bosque aleatorio)* (shapefile con el campo de la clase) o *Agrupar por similitud* (con *Número de estratos*; el estrato 1 es el más bajo). |
| Campo de la clase | Campo del shapefile de muestras que nombra la clase. Un objeto cuenta como muestra si los polígonos cubren al menos la mitad de sus celdas. |
| 4 · Unir los objetos vecinos de la misma clase en estratos | Estratos homogéneos. *Estrato mínimo (celdas)*: los menores toman la clase de sus vecinos. |
| Unidades de muestreo a repartir | Reparto de unidades por estrato (asignación de Neyman); 0 no reparte. |
| Cobertura desde | Altura a partir de la cual una celda cuenta como cubierta vegetal. |
| Salida | Polígonos (shp), tabla (csv) o GeoPackage (gpkg), con *Simplificar* y la base del nombre. |

La orden es `hipso obia`.

#### LiDAR → TLS

##### Medición de troncos

Botón grande. Mide los troncos en nubes de escáner terrestre, móvil o fotogramétrico, estima su volumen y clasifica troncos y ramas. Véase el apartado 7.3.


| Control | Qué hace |
|---|---|
| Equipo | *Terrestre estático (vóxel 3 cm)*: escáner sobre trípode, nubes densas y de poco ruido. *Móvil o SLAM (vóxel 5 cm)*: escáner de mano o de mochila, con ruido de varios centímetros. |
| Altura de la base sobre el suelo (cm) | Donde empieza la primera troza (10 cm). |
| Diámetro en punta delgada (cm) | Donde termina el fuste comercial (7 cm). |
| Longitud de la troza (m) | Distancia entre los diámetros medidos (0,5 m). |
| La nube ya tiene alturas sobre el suelo | No hay que calcular el terreno. |
| Franja: desde, hasta (m sobre el suelo) | Franja donde se buscan los troncos (0,7 a 3,5 m). |
| Hueco máximo sin medir (m) | Tras él se da el tronco por perdido (2 m). |
| Altura libre de ramas esperada (%) | Proporción de la altura del árbol que se espera que sea su altura comercial; sirve para estimarla en un tronco que se perdió cuando aún era grueso y muy por debajo de la cima (por ejemplo, 65 % en eucalipto). 0: sin estimación, y el tronco queda truncado. |
| Vóxel (cm) | La nube se reduce a un punto por vóxel. |
| Diámetro máximo esperado (m) | El mayor diámetro de tronco que se acepta (1 m). |
| Pasadas de poda de ramas | Rondas de filtrado vertical y de agrupación con las que se quitan las ramas de los puntos de tronco (2). |
| Verticalidad mínima (0–1) | Menor verticalidad que debe tener un punto para contarse como tronco (0,7), medida en un entorno de 10 cm. |
| Vóxeles mínimos de un tronco en la franja | Menor número de vóxeles de un grupo de puntos de tronco dentro de la franja (300); con menos no se cuenta como tronco. |
| Semigrosor de la sección (cm) | Semigrosor de la rebanada horizontal a la que se ajusta cada círculo (5 cm). |
| Puntos mínimos por sección | Menor número de vóxeles de una sección (25); con menos no vale. |
| Sectores ocupados (de 16) | Cuántos de los 16 sectores alrededor del círculo deben tener puntos (9), para que la sección esté repartida y no sea un arco. |
| Malla del terreno simulado (m) | Distancia entre las partículas de la tela con la que se simula el suelo cuando la nube no trae clase 2 (0,5 m). |
| Altura máxima medida (m) | La sección más alta que se mide (40 m). |
| Altura del diámetro (m) | Los troncos separados a esta altura son brotes de una cepa, cada uno un árbol; los que son uno a esa altura y se bifurcan por encima son un solo árbol bifurcado (1,30 m). |
| Distancia entre las bases de una cepa (m) | Distancia entre las bases de los troncos que crecen en una misma cepa (brotes de monte bajo o ramas de una bifurcación), 0,6 m. |
| Guardar la nube con troncos (clase 64) y ramas (clase 65) | Para editar: los puntos dentro del círculo medido pasan a la clase 64 y los demás próximos a un tronco a la 65, hasta la *Distancia máxima al eje (m)*. |
| Guardar también la nube de puntos de cada individuo | Escribe una nube por árbol. |

El resultado son puntos en la base de cada tronco con diámetro a 1,30 m, alturas, volumen, número de trozas, inclinación y calidad (`.shp` o `.csv`), y `<nombre>_trozas.csv` con el detalle de cada troza. La orden es `hipso stem-volume`.

#### LiDAR → Infraestructuras

##### Corredores eléctricos

Botón grande. Mide el peligro de la vegetación bajo una línea eléctrica cuyos cables y postes ya están clasificados. Véase el apartado 7.3.


| Control | Qué hace |
|---|---|
| MDT | Modelo del suelo (archivo o capa cargada); por defecto, el último MDT hecho con la primera nube. |
| Distancia de seguridad | Dentro de esta distancia de un conductor hay invasión (3 m). Depende de la tensión de la línea y de la normativa aplicable. |
| Margen de caída | Un árbol cuya altura más este margen alcanza la posición del conductor podría golpearlo (1 m). |
| Franja a cada lado | La vegetación a más de esta distancia de todo conductor no se mira (30 m). |
| Altura mínima de la vegetación | La vegetación más baja sobre el suelo se ignora (2 m). |
| Unir puntos de cable a menos de | Los puntos de cable más cercanos que esto son un solo conductor (2,5 m). |
| Rellenar huecos de hasta | Mayor hueco de puntos de cable que se rellena (30 m). |
| Clases de vegetación | 3, 4 y 5 por defecto. |
| Salida | Capas (shp) y base del nombre. |

Las opciones que la ventana no muestra (clases de cable y de poste, longitud y número de puntos mínimos de un conductor, altura mínima y alcance de un poste, celda de agrupación) están en la orden `hipso corridor`.

### 4.6 Pestaña Vectorial

La pestaña *Vectorial* reúne lo que opera sobre capas de puntos, líneas y polígonos (shapefile, GeoPackage, GeoJSON y KML). Solo los botones imprescindibles son grandes. La edición de elementos (dibujar, mover vértices, seleccionar, copiar) no está en la cinta, porque depende de la capa elegida: está en el panel lateral *Edición* (apartado 4.9). Todas las herramientas de los grupos *Geometría*, *Geoprocesos* y *Capas y tablas* abren su ventana, trabajan sobre capas ya cargadas o sobre archivos, y escriben una capa nueva sin modificar las originales.

#### Vectorial → Capa

##### Nueva capa

Botón grande. Crea una capa vacía de puntos, líneas o polígonos, con los campos y los tipos que se le den (texto, entero, decimal, sí/no, fecha o hipervínculo). Se rellena después con *Digitalizar* en el panel *Edición*. Véase el apartado 9, *Tipos de campo*.


| Control | Qué hace |
|---|---|
| Nombre | Nombre de la capa y del archivo. |
| Tipo | *Puntos*, *Líneas* o *Polígonos*. |
| Campos de la tabla | Lista de campos con su nombre y su tipo (texto, entero, decimal, sí/no, fecha o hipervínculo); se añaden y se quitan antes de crear la capa. |

La capa se crea vacía y se rellena con *Digitalizar* en el panel *Edición*. Los tipos de campo se explican en el apartado 9, *Tipos de campo*.

##### Tabla de atributos

Botón grande. Abre la tabla de la capa vectorial elegida. Sus celdas se editan con una comprobación del tipo de cada campo, y desde ella se seleccionan filas (*Seleccionar todo*, *Invertir*, *Limpiar* y *Seleccionar por atributo*), se hace *Zoom a la selección*, se *Elimina*, se *Añade campo* y se *Guarda*; la casilla *Solo seleccionados* limita la vista a la selección. Véase el apartado 9.


| Control | Qué hace |
|---|---|
| Seleccionar todo, Invertir, Limpiar | Operan sobre la selección de filas, que es la misma de elementos del mapa. |
| Zoom a la selección | Encuadra los elementos seleccionados. |
| Eliminar | Elimina los elementos seleccionados. |
| Deshacer, Rehacer | Recorren las ediciones de la capa. |
| Guardar | Guarda la capa en su archivo. |
| Seleccionar por atributo | Selecciona las filas cuyo valor cumple una condición sobre un campo (según su tipo), como *Nueva selección*, ampliando la anterior o *Dentro de la selección*. |
| Añadir campo | Añade un campo con nombre y tipo. |
| Solo seleccionados | Muestra solo las filas seleccionadas. |
| Celdas | Un clic abre el valor (o el hipervínculo); un doble clic lo edita, y la tabla comprueba lo escrito contra el tipo del campo (por ejemplo, las fechas se escriben AAAA-MM-DD). Las columnas se ordenan pulsando su cabecera. |

#### Vectorial → Geometría

##### Comprobar validez

Busca polígonos con anillos sin cerrar, autointersecciones y otros defectos que hacen fallar las operaciones posteriores, y lista dónde están. Véase el apartado 9, *Geoprocesos con polígonos*.


Elige la capa de polígonos y busca anillos sin cerrar, autointersecciones, huecos fuera del polígono y partes solapadas, y lista cada uno. No modifica la capa. La orden es `hipso is-valid`.

##### Reparar geometrías

Repara los polígonos no válidos y escribe una capa nueva, conservando los atributos. Véase el apartado 9.


Elige la capa de polígonos y repara los no válidos con la regla par-impar: un lazo se convierte en dos partes y los bordes repetidos se cancelan. Los válidos no se tocan y la tabla de atributos se conserva. La orden es `hipso make-valid`.

#### Vectorial → Geoprocesos

##### Estadística zonal

Para cada polígono resume las celdas de un ráster cuyo centro cae dentro de él (recuento, mínimo, máximo, media, suma, desviación típica y mediana), como el CHM medio de cada rodal. Véase el apartado 9, *Estadística zonal*.


| Control | Qué hace |
|---|---|
| Ráster | El ráster cuyas celdas se resumen, de un archivo o de las capas cargadas. |
| Capa | La capa de polígonos. |
| Prefijo de los campos nuevos | Por ejemplo, `chm_` da `chm_mean`; un campo de shapefile tiene 10 caracteres como máximo. |
| Reparar antes los polígonos no válidos | Evita que un polígono inválido falle. |

El resultado es una capa nueva con los campos originales y un campo más por estadístico (`count`, `min`, `max`, `mean`, `sum`, `stddev` y `median`). La orden es `hipso zonal`.

##### Búfer

Amplía cada elemento la distancia indicada (negativa, lo reduce), también sobre puntos y líneas. Véase el apartado 9.


| Control | Qué hace |
|---|---|
| Capa | La capa que se amplía. Con puntos, da un círculo de ese radio alrededor de cada punto (la distancia debe ser mayor que 0). |
| Distancia | En las unidades de la capa: metros en un sistema proyectado, grados en uno geográfico. Negativa, reduce el polígono. |
| Reparar antes los polígonos no válidos | Corrige los polígonos inválidos antes de operar. |

Las esquinas quedan redondeadas. La orden es `hipso buffer`.

##### Disolver

Une los polígonos que comparten el valor de un campo, o todos, en uno solo. Véase el apartado 9.


| Control | Qué hace |
|---|---|
| Capa | La capa de polígonos. |
| Campo (vacío: todos en uno) | Une los polígonos que comparten el valor de este campo; sin campo, une todos. |

El campo `n` del resultado cuenta cuántos polígonos se unieron. La orden es `hipso dissolve`.

##### Unión

Superpone dos capas y da las piezas que están en las dos y las que están solo en una. Véase el apartado 9.


| Control | Qué hace |
|---|---|
| Primera capa, Segunda capa | Las dos capas de polígonos. |
| Reparar antes los polígonos no válidos | Corrige los polígonos inválidos antes de operar. |

Da las piezas que están en las dos capas (con los campos de ambas, los de la segunda precedidos por `b_`), solo en la primera y solo en la segunda. La orden es `hipso union`.

##### Intersección

Se queda solo con las piezas que están en las dos capas, con los atributos de ambas. Véase el apartado 9.


| Control | Qué hace |
|---|---|
| Primera capa, Segunda capa | Las dos capas de polígonos. |
| Reparar antes los polígonos no válidos | Corrige los polígonos inválidos antes de operar. |

Se queda solo con las piezas que están en las dos capas, con los campos de ambas (los de la segunda con el prefijo `b_`). Lo que solo está en una se descarta. La orden es `hipso intersect`.

##### Diferencia

A la primera capa le quita lo que cubre la segunda. Véase el apartado 9.


| Control | Qué hace |
|---|---|
| Primera capa, Segunda capa | Las dos capas de polígonos. |
| Reparar antes los polígonos no válidos | Corrige los polígonos inválidos antes de operar. |

A la primera capa le quita lo que cubre la segunda; cada pieza que queda conserva los campos de la primera, y los polígonos cubiertos del todo desaparecen. Esta operación no tiene orden de la línea de comandos.

##### Diferencia simétrica

Se queda con lo que está solo en una de las dos capas. Véase el apartado 9.


| Control | Qué hace |
|---|---|
| Primera capa, Segunda capa | Las dos capas de polígonos. |
| Reparar antes los polígonos no válidos | Corrige los polígonos inválidos antes de operar. |

Se queda con lo que está solo en una de las dos capas: lo de la primera con sus campos y lo de la segunda con los suyos precedidos por `b_`. Lo que está en las dos se descarta. No tiene orden de la línea de comandos.

##### Centroides

Una capa de puntos con el centroide de cada elemento. Véase el apartado 9.


Elige la capa (de polígonos o de líneas) y escribe una capa de puntos con el centroide de cada elemento y sus mismos atributos. En un polígono cóncavo o con huecos el centroide puede caer fuera de él: para eso están los *Puntos en superficie*. No tiene orden de la línea de comandos.

##### Puntos en superficie

Una capa de puntos con un punto que cae siempre dentro de cada polígono, a diferencia del centroide, que puede caer fuera en formas cóncavas. Véase el apartado 9.


Elige la capa y escribe una capa de puntos con un punto que cae siempre dentro de cada polígono (o sobre cada línea), aunque sea cóncavo o tenga huecos, con los mismos atributos. Sirve para etiquetar o para extraer valores de un ráster. No tiene orden de la línea de comandos.

##### Unir por localización

Copia a cada elemento de la primera capa los atributos del elemento de la segunda con el que se relaciona espacialmente. Véase el apartado 9.


| Control | Qué hace |
|---|---|
| Primera capa, Segunda capa | La capa que recibe los campos y la que los da. |
| Un elemento de la primera capa cumple si… | La relación espacial con la segunda: que intersecte, esté dentro de, contenga, toque, cruce o no toque («…de la segunda»). |
| Campos | *Los del primer coincidente* o *Suma y media de los numéricos* de todos los que coinciden. |
| Prefijo de los campos nuevos | Prefijo con el que se copian los campos de la segunda capa. |

Añade siempre `n_coinc` con cuántos elementos coinciden. No tiene orden de la línea de comandos.

##### Voronoi

Un polígono de Voronoi por cada punto de una capa (la zona más cercana a él), con los atributos del punto. Sirve para repartir el territorio entre estaciones o medir el área que le toca a cada árbol. Véase el apartado 9.


| Control | Qué hace |
|---|---|
| Capa | Una capa de puntos. |
| Recorte | *Sin recorte* (las celdas de los puntos del borde, que no tienen fin, se dejan fuera), *Una caja algo mayor que los puntos* o *Una capa de polígonos* (*Polígonos de recorte*). |
| Guardar también las aristas de Delaunay | Escribe además una capa con las líneas entre los puntos vecinos. |

Da la celda de Voronoi de cada punto (el área que está más cerca de él que de ningún otro), con los atributos del punto. La orden es `hipso voronoi`.

#### Vectorial → Capas y tablas

##### Extraer valores

Añade a una capa un campo por banda de uno o varios rásteres, con el valor bajo cada elemento (el CHM en cada parcela o árbol). Véase el apartado 9, *Capas y tablas*.


La ventana se genera a partir de la orden `hipso extract`: cada control es una de sus opciones, con la misma ayuda, y las que son archivos tienen botón para elegirlos de las capas cargadas (*Del mapa…*). Todas las opciones son estas.

| Opción | Valor | Por defecto | Qué hace |
|---|---|---|---|
| `LAYER` | archivo o carpeta | ninguno | La capa (puntos, líneas o polígonos), en el mismo sistema que los rásteres. **Obligatoria.** |
| `--raster` | archivo o carpeta | ninguno | Un ráster (se repite para varios) **Obligatoria.** Se puede repetir. |
| `--bands` | texto o número | ninguno | Bandas que extraer, contando desde 1 (por defecto, todas) Se puede repetir. |
| `--bilinear` | casilla | desactivada | Interpola entre los cuatro centros de celda más cercanos en lugar de tomar la celda que contiene el punto. |
| `--prefix` | texto | ninguno | Prefijo de los campos nuevos. |
| `-o`, `--output` | archivo o carpeta | ninguno | Capa de salida; por defecto: <resultados>/<capa>_valores.<ext>. |

##### Unir tabla

Une a una capa las columnas de una tabla (csv, texto o Excel) por un campo clave, como el inventario de campo a las parcelas. Véase el apartado 9.


La ventana se genera a partir de la orden `hipso join-table`: cada control es una de sus opciones, con la misma ayuda, y las que son archivos tienen botón para elegirlos de las capas cargadas (*Del mapa…*). Todas las opciones son estas.

| Opción | Valor | Por defecto | Qué hace |
|---|---|---|---|
| `LAYER` | archivo o carpeta | ninguno | La capa a la que se añaden las columnas (.shp, .gpkg o .geojson) **Obligatoria.** |
| `TABLE` | archivo o carpeta | ninguno | La tabla (.csv, texto o .xlsx) de la que salen las columnas. **Obligatoria.** |
| `--layer-key` | texto o número | ninguno | Campo clave de la capa. **Obligatoria.** |
| `--table-key` | texto o número | ninguno | Columna clave de la tabla (por defecto, el mismo nombre) |
| `--prefix` | texto | ninguno | Prefijo de los campos nuevos. |
| `-o`, `--output` | archivo o carpeta | ninguno | Capa de salida; por defecto: <resultados>/<capa>_union.<ext>. |

##### Combinar capas

Junta varias capas del mismo tipo de geometría en una; los campos son la unión de los de todas. Véase el apartado 9.


La ventana se genera a partir de la orden `hipso merge`: cada control es una de sus opciones, con la misma ayuda, y las que son archivos tienen botón para elegirlos de las capas cargadas (*Del mapa…*). Todas las opciones son estas.

| Opción | Valor | Por defecto | Qué hace |
|---|---|---|---|
| `INPUTS` | archivo o carpeta | ninguno | Las capas que combinar. **Obligatoria.** Se puede repetir. |
| `--origin` | texto o número | ninguno | Añade un campo con el nombre del archivo de origen de cada elemento. |
| `-o`, `--output` | archivo o carpeta | ninguno | Capa de salida; por defecto: <resultados>/<primera>_combinada.<ext>. |

##### Dividir por campo

Escribe una capa por cada valor distinto de un campo (un archivo por rodal, por especie…). Véase el apartado 9.


La ventana se genera a partir de la orden `hipso split`: cada control es una de sus opciones, con la misma ayuda, y las que son archivos tienen botón para elegirlos de las capas cargadas (*Del mapa…*). Todas las opciones son estas.

| Opción | Valor | Por defecto | Qué hace |
|---|---|---|---|
| `INPUT` | archivo o carpeta | ninguno | La capa que se divide (.shp, .gpkg o .geojson) **Obligatoria.** |
| `--field` | texto o número | ninguno | El campo por el que dividir. **Obligatoria.** |
| `--dir` | archivo o carpeta | ninguno | Carpeta de salida; por defecto: <resultados>/<capa>_partes. |

##### Recortar capa

Recorta una capa de puntos, líneas o polígonos con los polígonos de otra, o borra de ella lo que cubren. Véase el apartado 9.


La ventana se genera a partir de la orden `hipso clip-vector`: cada control es una de sus opciones, con la misma ayuda, y las que son archivos tienen botón para elegirlos de las capas cargadas (*Del mapa…*). Todas las opciones son estas.

| Opción | Valor | Por defecto | Qué hace |
|---|---|---|---|
| `INPUT` | archivo o carpeta | ninguno | La capa que recortar. **Obligatoria.** |
| `--mask` | archivo o carpeta | ninguno | Los polígonos con los que recortar. **Obligatoria.** |
| `--erase` | casilla | desactivada | Borra lo que cubren los polígonos en lugar de conservarlo. |
| `-o`, `--output` | archivo o carpeta | ninguno | Capa de salida; por defecto: <resultados>/<capa>_recortada.<ext> (o _borrada) |

##### Cercanía

Añade a cada elemento la distancia al elemento más cercano de otra capa, su número de orden y, si se pide, uno de sus campos. Véase el apartado 9.


La ventana se genera a partir de la orden `hipso near`: cada control es una de sus opciones, con la misma ayuda, y las que son archivos tienen botón para elegirlos de las capas cargadas (*Del mapa…*). Todas las opciones son estas.

| Opción | Valor | Por defecto | Qué hace |
|---|---|---|---|
| `INPUT` | archivo o carpeta | ninguno | La capa. **Obligatoria.** |
| `--to` | archivo o carpeta | ninguno | La capa de la que se busca el elemento más cercano. **Obligatoria.** |
| `--carry` | texto o número | ninguno | Campo de la otra capa que se copia del elemento más cercano. |
| `--max` | texto o número | ninguno | Distancia máxima: los más lejanos quedan sin valor. |
| `-o`, `--output` | archivo o carpeta | ninguno | Capa de salida; por defecto: <resultados>/<capa>_cercania.<ext>. |

##### Resumen estadístico

Recuento, suma, media, mínimo, máximo y desviación de campos numéricos de una capa o tabla, por grupos de un campo. Véase el apartado 9.


La ventana se genera a partir de la orden `hipso summary`: cada control es una de sus opciones, con la misma ayuda, y las que son archivos tienen botón para elegirlos de las capas cargadas (*Del mapa…*). Todas las opciones son estas.

| Opción | Valor | Por defecto | Qué hace |
|---|---|---|---|
| `INPUT` | archivo o carpeta | ninguno | La capa o la tabla (csv, texto, xlsx) **Obligatoria.** |
| `--by` | texto o número | ninguno | Campo por el que agrupar (sin él, un solo grupo) |
| `--fields` | texto o número | ninguno | Campos numéricos que resumir, separados por comas (por defecto, todos los numéricos) Se puede repetir. |
| `-o`, `--output` | archivo o carpeta | ninguno | Tabla CSV de salida; por defecto: <resultados>/<entrada>_resumen.csv. |

### 4.7 Pestaña Raster

La pestaña *Raster* tiene tres grupos. *Imagen* trabaja con bandas, *Geoprocesos* con los valores y la geometría de los rásteres, y *Análisis del terreno* con los modelos de elevación. Los botones grandes de cada grupo son los más comunes, y el resto va en botones pequeños o en menús. Todas las operaciones aceptan rásteres en otros sistemas de coordenadas y los pasan al de destino (apartado 4.10).

#### Raster → Imagen

##### Composición de bandas

Botón grande. Elige qué bandas de una imagen multibanda se muestran como rojo, verde y azul (color natural, falso color infrarrojo y otras), con estirado del contraste, brillo y gamma. Véase el apartado 8.


La ventana *Bandas e índices…* de una imagen multibanda (y la pestaña *Bandas* de *Simbología*) elige qué banda se pinta como rojo, verde y azul. Con *Estirado* se reparte el contraste (*Sin estirar*, *Mínimo–máximo*, *Percentiles 2–98 %*, *±2 desviaciones* o *Cortes manuales*, con el mínimo y el máximo que se pintan, iguales para las tres bandas), y *Brillo*, *Contraste* y *Gamma* los ajustan; *Restablecer* los devuelve a sus valores iniciales. Véase el apartado 4.9, *Simbología de una imagen*.

##### Índices espectrales

Botón grande. Calcula índices como NDVI, GNDVI, NDWI, EVI, SAVI, NDMI, NBR, VARI o ExG a partir de las bandas, o una expresión propia, y los guarda como GeoTIFF. Véase el apartado 8.


| Control | Qué hace |
|---|---|
| Imagen | La imagen multibanda; si es muy grande, el índice se calcula con 1 de cada n píxeles. |
| ¿Qué banda es cada una? | Asigna a cada banda su papel (rojo, verde, azul, infrarrojo cercano, SWIR1, SWIR2). |
| Índice | NDVI, GNDVI, NDWI, EVI, SAVI, NDMI, NBR, VARI, ExG o una *Expresión* propia con RED, GREEN, BLUE, NIR, SWIR1, SWIR2 o B1…Bn, con + − × ÷ y paréntesis. |
| Calcular índice | Calcula el índice y lo añade como capa. |
| Guardar como GeoTIFF… | Guarda el último índice calculado. |

##### Mosaico

Botón grande. Une varios rásteres en uno, eligiendo qué hacer donde se solapan (primero, último, media, mínimo, máximo, mediana, suma o número de capas). Véase el apartado 8.


| Control | Qué hace |
|---|---|
| Rásteres | Lista de rásteres con el mismo número de bandas (se pueden subir, bajar o quitar). |
| Donde se solapan | Qué valor queda donde hay píxeles de varios rásteres: *primero*, *último*, *media*, *mínimo*, *máximo*, *mediana*, *suma* o *número de capas*. |
| Zona que cubre | *Unión de todos*, *Solo donde coinciden todos* o *La del primero*. |
| Remuestreo | *Vecino más cercano*, *Bilineal*, *Media (para reducir)*, *Cúbico* o *Moda (clases, para reducir)*. |
| Tipo de dato | *Como el primero*, *8 bits*, *16 bits* o *Decimal de 32 bits*. |
| Tamaño de píxel | El menor de los rásteres, o uno propio. |
| Sin datos en la entrada, en la salida | Valor de las entradas que significa «sin datos» y valor que se escribe donde no hay datos. |
| Salida | GeoTIFF. |

La salida se escribe por franjas comprimidas, sin cargar las imágenes enteras. La orden es `hipso mosaic`.

##### Crear multibanda

Une varias imágenes de una banda en una sola multibanda, en el orden de la lista. Véase el apartado 8.


| Control | Qué hace |
|---|---|
| Rásteres | Lista de rásteres (archivos o capas cargadas) que se pueden subir, bajar o quitar. El orden de la lista es el de las bandas. |
| Bandas | Qué bandas de cada ráster pasan a la imagen nueva (por ejemplo 1,3); vacío, todas. Admite hasta 64 bandas. |
| Salida | GeoTIFF; por defecto, junto a los datos. |

Cada banda de cada ráster de la lista pasa a ser una banda de la imagen nueva, como al apilar canales sueltos R, G, B y NIR en un solo archivo. La salida se escribe por franjas comprimidas (deflate), sin cargar las imágenes enteras. La orden es `hipso stack`.

##### Igualar rásteres

Reproyecta, recorta a la extensión y remuestrea varios rásteres sobre las celdas de uno de referencia, de modo que coincidan celda a celda y se puedan apilar. Véase el apartado 8.


| Control | Qué hace |
|---|---|
| Referencia | El ráster cuyas celdas se copian (origen, tamaño de celda y dimensiones), de un archivo o de las capas cargadas. |
| Rásteres a igualar | Lista de rásteres (se añaden archivos o capas, y se vacía o se quita uno). Debajo de cada uno la ventana avisa de si ya coincide con la referencia o si está en otro sistema, en cuyo caso se reproyecta. |
| Remuestreo | *Automático* (vecino más próximo para clases y bilineal para el resto), *Vecino más próximo*, *Bilineal*, *Media*, *Cúbico* o *Moda*. |
| Valor de «sin datos» en la salida | Lo que contienen las celdas que quedan fuera de un ráster (-9999 por defecto). |

Cada resultado se guarda junto al original como `<nombre>_alineado.tif`, sin pisar nunca uno anterior. La orden es `hipso align`.

#### Raster → Geoprocesos

##### Reproyectar

Botón grande. Pasa nubes y rásteres a otro sistema de coordenadas y, si se pide, cambia también la referencia vertical de las alturas. Véase el apartado 8.


| Control | Qué hace |
|---|---|
| Archivos | Nubes y rásteres, de archivos o de las capas cargadas. Junto a cada uno se ve su sistema y su referencia vertical. |
| Destino: *Código EPSG* | Escribe el código del sistema de destino. |
| Destino: *Sistema de un ráster* | Toma solo el sistema de un ráster de referencia. |
| Destino: *Celdas de un ráster* | El resultado coincide celda a celda con el ráster de referencia: los rásteres toman su sistema, extensión, origen y celda, y las nubes solo su sistema. |
| Origen, si el archivo no lo declara | Sistema (EPSG) de los archivos sin sistema declarado. |
| Celda del resultado | Tamaño de celda de los rásteres (por defecto, el de cada entrada o su equivalente en el sistema nuevo). |
| Remuestreo | Método con el que se calculan los valores de los rásteres. |
| Alturas de entrada | *Las del archivo*, *Elipsoidal* o un *Código EPSG vertical*. |
| Alturas de salida | *No cambiar las alturas*, *Elipsoidal* o un *Código EPSG vertical*. Pasar de elipsoidales (las que da el GNSS) a ortométricas exige un modelo de geoide. |
| Aceptar aproximaciones | Sin ajuste de datum, o con la altura sin corregir donde no hay geoide. |
| Aceptar una transformación menos precisa si falta la mejor | Por ejemplo, si falta una rejilla. Sin marcar, una transformación imprecisa se rechaza con un error. |

Las rejillas de geoide y de datum se buscan en `HIPSO_GRID_DIR`, `PROJ_DATA` y `PROJ_LIB`; las de España vienen incluidas. Cada resultado se guarda junto al original como `<nombre>_epsg<código>`. La orden es `hipso reproject`.

##### Calculadora

Botón grande. Calcula una expresión celda a celda con varios rásteres (`A`, `B`, `C`… o las bandas `B1`…`Bn`). Véase el apartado 8, *Calculadora ráster*.


| Control | Qué hace |
|---|---|
| Rásteres | Lista de rásteres, que se llaman A, B, C… en el orden de la lista; una banda concreta es `A.2`. |
| Expresión | Operaciones, comparaciones, funciones y condiciones, por ejemplo `if(A > 0, A - B, 0)`. Funciones: abs, sqrt, ln, log10, exp, pow, min, max, floor, ceil, round, sin, cos, tan, atan2, clamp, if e isnan; comparaciones, and, or, not, % y ^. Una celda vacía da vacío, salvo dentro de isnan(). |
| Ejemplos | Carga una expresión de muestra: diferencia, máscara, quitar negativos, reescalar a 0–255, rellenar vacíos o NDVI de una imagen de cuatro bandas. |
| Resultado | *Decimales*, *Enteros 0-255* o *Enteros 0-65535*. |
| Es un ráster de clases | Las vistas reducidas no promedian. |
| Si las celdas de los rásteres difieren, poner los demás sobre las del primero | También reproyecta los que estén en otro sistema. |

La orden es `hipso calc`.

##### Cortar por máscara

Botón grande. Vacía lo que queda fuera de una capa de polígonos o de otro ráster y, si se quiere, corta el ráster a su caja. Véase el apartado 8.


| Control | Qué hace |
|---|---|
| Ráster, Banda | Ráster que se recorta. |
| Máscara | Una capa de polígonos o un ráster (se conservan las celdas donde tiene datos distintos de cero), de un archivo o de las capas cargadas. |
| Cortar el ráster a la caja de la máscara | Si no, conserva su tamaño. |
| Conservar lo que queda fuera de la máscara | Invierte la máscara: vacía lo de dentro. |
| Valor de las celdas vacías | El del archivo, o 0 / -9999 según el tipo. |

Las celdas son las del ráster original: no se remuestrea nada. La orden es `hipso clip-raster`.

##### Reclasificar

Cambia los valores de un ráster con una tabla de reglas (intervalos, comparaciones o valores). Véase el apartado 8.


| Control | Qué hace |
|---|---|
| Ráster, Banda | Ráster y banda que se reclasifican. |
| Reglas | Una regla por línea: «valor = nuevo», «a..b = nuevo» (ambos extremos incluidos), «< b», «<= b», «> a», «>= a» o «nodata»; como resultado, un número o «nodata». «else = …» da el valor de las demás celdas. Manda la primera regla que vale para una celda. |
| Ejemplos | Unir clases, intervalos a clases, valores negativos a «sin datos», máscara 1/0 o rellenar vacíos con 0. |
| Una regla por clase | Rellena una regla «valor = valor» por cada clase del ráster, para cambiar solo las que se quieran. |
| Las celdas que ninguna regla recoge | *Conservan su valor*, *Quedan vacías* o *Pasan a* un valor. |

El resultado usa el tipo entero más pequeño que cabe. La orden es `hipso reclass`.

##### Tamizar

Quita los parches pequeños de un ráster de clases: toman el valor de su vecino mayor. Véase el apartado 8.


| Control | Qué hace |
|---|---|
| Ráster | Ráster de clases, de un archivo o de las capas cargadas. |
| Umbral (celdas) | Los parches conexos del mismo valor con menos celdas que el umbral toman el valor del parche vecino más grande. |
| Las celdas se tocan también por las esquinas (8 vecinas) | Cuenta también las conexiones diagonales. |

Las celdas vacías no se tocan. La orden es `hipso sieve`.

##### Proximidad

Calcula para cada celda la distancia a la celda más cercana de un valor dado. Véase el apartado 8.


| Control | Qué hace |
|---|---|
| Ráster | Ráster de entrada. |
| Valores objetivo | Valores de los que se mide la distancia (2, 5…); vacío, toda celda distinta de 0. Las celdas objetivo valen 0. |
| Distancia máxima | Hasta dónde se calcula; sin límite por defecto. |
| Distancias en celdas | Si no, las distancias salen en unidades del mapa. |

La distancia es euclídea exacta. La orden es `hipso proximity`.

##### Rellenar huecos

Rellena las celdas vacías de un ráster con las de su entorno. Véase el apartado 8.


| Control | Qué hace |
|---|---|
| Ráster | Ráster con celdas vacías. |
| Alcance (celdas) | Hasta qué distancia se buscan celdas con valor a su alrededor. |
| Pasadas de suavizado | Pasadas de una media 3×3 sobre las celdas rellenadas (0: ninguna). |

Rellena ponderando por el inverso del cuadrado de la distancia en ocho direcciones; lo que no tiene ninguna celda a su alcance sigue vacío. La orden es `hipso fill-nodata`.

##### Vectorizar

Convierte un ráster en polígonos con el valor como atributo. Véase el apartado 8.


| Control | Qué hace |
|---|---|
| Agrupar | *Por clase (mismo valor)*: un polígono por conjunto de celdas contiguas con el mismo valor. *Rango de valores*: las celdas entre *Desde* y *hasta*. *Intervalos entre cortes*: un polígono por intervalo definido por los *Cortes*. |
| Celdas mínimas por polígono | Los polígonos menores se descartan. |
| Simplificar | En unidades del mapa; 0 sigue las celdas. |
| Las celdas se tocan también por las esquinas (8 vecinas) | Cuenta también las conexiones diagonales. |
| Guardar como GeoPackage | Si no, shapefile. |

Los polígonos llevan el valor, el área y las celdas como atributos. La orden es `hipso vectorize`.

##### Rasterizar

Convierte una capa vectorial en ráster: cada celda recibe el valor de un campo del último elemento que la cubre. Véase el apartado 8, *Análisis ráster*.


La ventana se genera a partir de la orden `hipso rasterize`: cada control es una de sus opciones, con la misma ayuda, y las que son archivos tienen botón para elegirlos de las capas cargadas (*Del mapa…*). Todas las opciones son estas.

| Opción | Valor | Por defecto | Qué hace |
|---|---|---|---|
| `LAYER` | archivo o carpeta | ninguno | La capa. **Obligatoria.** |
| `--reference` | archivo o carpeta | ninguno | Ráster de referencia: toma sus celdas. |
| `--cell` | texto o número | ninguno | Tamaño de celda cuando no hay referencia. |
| `--field` | texto o número | ninguno | Campo con el valor que se pinta (sin él, 1) |
| `-o`, `--output` | archivo o carpeta | ninguno | GeoTIFF de salida; por defecto: <resultados>/<capa>_raster.tif. |

##### Filtro focal

Media, mediana, mínimo, máximo, suma, rango, desviación, mayoría o suavizado gaussiano de cada celda sobre una ventana a su alrededor. Véase el apartado 8.


La ventana se genera a partir de la orden `hipso focal`: cada control es una de sus opciones, con la misma ayuda, y las que son archivos tienen botón para elegirlos de las capas cargadas (*Del mapa…*). Todas las opciones son estas.

| Opción | Valor | Por defecto | Qué hace |
|---|---|---|---|
| `INPUT` | archivo o carpeta | ninguno | El ráster (GeoTIFF) **Obligatoria.** |
| `--stat` | texto | `mean` | mean, median, min, max, sum, range, std, majority o gaussian. |
| `--radius` | número | `1` | Radio de la ventana en celdas. |
| `--circle` | casilla | desactivada | Ventana circular en lugar de cuadrada. |
| `--sigma` | texto o número | ninguno | Desviación típica del filtro gaussiano, en celdas (por defecto, la mitad del radio) |
| `--min-valid` | número | `1` | Menos celdas con valor que esto dejan la celda vacía. |
| `-o`, `--output` | archivo o carpeta | ninguno | GeoTIFF de salida; por defecto: <resultados>/<entrada>_focal.tif. |

##### Estadísticas ráster

Recuento, mínimo, máximo, media, desviación, mediana, percentiles e histograma de un ráster. Véase el apartado 8.


La ventana se genera a partir de la orden `hipso rstats`: cada control es una de sus opciones, con la misma ayuda, y las que son archivos tienen botón para elegirlos de las capas cargadas (*Del mapa…*). Todas las opciones son estas.

| Opción | Valor | Por defecto | Qué hace |
|---|---|---|---|
| `INPUT` | archivo o carpeta | ninguno | El ráster (GeoTIFF) **Obligatoria.** |
| `--bins` | número | `20` | Intervalos del histograma. |
| `-o`, `--output` | archivo o carpeta | ninguno | CSV del histograma; por defecto: <resultados>/<entrada>_histograma.csv. |

##### Tabular áreas

Área de cada clase de un ráster dentro de cada zona de otro, como tabla CSV. Véase el apartado 8.


La ventana se genera a partir de la orden `hipso tabulate`: cada control es una de sus opciones, con la misma ayuda, y las que son archivos tienen botón para elegirlos de las capas cargadas (*Del mapa…*). Todas las opciones son estas.

| Opción | Valor | Por defecto | Qué hace |
|---|---|---|---|
| `ZONES` | archivo o carpeta | ninguno | El ráster de zonas (clases enteras) **Obligatoria.** |
| `--values` | archivo o carpeta | ninguno | El ráster de clases que se mide dentro de cada zona. **Obligatoria.** |
| `-o`, `--output` | archivo o carpeta | ninguno | CSV de salida; por defecto: <resultados>/<zonas>_tabulado.csv. |

#### Raster → Análisis del terreno

Cinco menús reparten los productos que se derivan de un MDT (o de cualquier ráster de alturas en metros). Todos funcionan igual: se elige el MDT (un archivo o una capa cargada) y la salida, un GeoTIFF del mismo tamaño, y las celdas sin dato siguen sin dato. Véase el apartado 7.2, *Análisis de terreno*.


Los menús de *Análisis del terreno* abren una ventana con los mismos controles de fondo. Cada ventana pide un MDT (archivo o capa cargada), la carpeta de resultados y los siguientes ajustes, según el producto.

| Control | Productos | Qué hace |
|---|---|---|
| Grados, Porcentaje | Pendiente | Unidad de la pendiente. |
| Dirección de la luz, Altura de la luz | Sombreado | Azimut en grados desde el norte en sentido horario (315 por defecto) y altura sobre el horizonte (45). |
| Radio | Posición topográfica (TPI) | Radio del entorno en unidades del mapa (10). |
| Superficie mínima que vierte | Red de drenaje, Subcuencas | Superficie drenada (m²) que forma un cauce (10 000). |
| Observador X / Y, Altura de los ojos, Altura a ver, Alcance | Cuenca visual | Posición del observador en coordenadas del mapa, altura del ojo (1,7 m), altura sobre el suelo que debe verse (0 m) y hasta dónde mira (1000). |
| Bordes y huecos | Productos de ventana 3×3 | *Sin dato* (por defecto), *Repetir el borde* (da valor a las del borde) o *Rellenar con la celda central* (también a las vecinas de un hueco, aplanando la superficie ahí). |
| Factor Z | Todos | Multiplica las alturas cuando no están en las unidades de la celda; 1 si están en metros. |

La orden es `hipso terrain`, con `--product` para elegir el producto.

##### Relieve

Menú con *Pendiente* (inclinación en grados o porcentaje), *Orientación* (hacia dónde mira la ladera, en grados desde el norte) y *Sombreado* (iluminación con la dirección y la altura de luz que se elijan).

##### Forma del terreno

Menú con *Curvatura en planta*, *Curvatura de perfil* y *Curvatura total* (positivas en cumbres y lomas, negativas en vaguadas), *Rugosidad* (diferencia entre la celda más alta y la más baja de la ventana de 3×3), *Índice de rugosidad (TRI)* (media de las diferencias con las ocho vecinas), *Posición topográfica (TPI)* (altura menos la media del círculo de radio dado) y *Crestas y drenaje* (traza las líneas de cresta o de drenaje).

###### Crestas y drenaje

| Control | Qué hace |
|---|---|
| Líneas de | *Cresta* o *Drenaje*. |
| Superficie mínima que recoge | Una línea nace donde se recoge esta superficie. |
| Longitud mínima | Los tramos más cortos se descartan. |
| Suavizado | Pasadas de Chaikin; 0 sigue las celdas. |

Traza líneas 3D con el orden de Strahler (las cabeceras son 1; dos tramos del mismo orden que se juntan dan el siguiente), la longitud y la pendiente media de cada tramo. La orden es `hipso ridges`.

##### Hidrología

Menú con *Profundidad de depresiones* (cuánto hay que rellenar cada depresión para que el agua salga), *Acumulación de flujo* (superficie que vierte a cada celda), *Índice de humedad (TWI)*, *Red de drenaje* (celdas por las que vierte al menos la superficie mínima indicada), *Subcuencas*, *Zonas encharcadas* (cada charco con su área, profundidad, volumen y cota de desborde) e *Inundación* (simula el agua de uno o varios niveles sobre el modelo).

###### Zonas encharcadas

| Control | Qué hace |
|---|---|
| MDT | El modelo del terreno (archivo o capa cargada), en metros y en un sistema métrico (UTM). |
| Terreno | *Nivelado (cultivo)*: 5 cm de profundidad y 1 m² de área mínimos. *Irregular (monte)*: pide 15 cm, 4 m² y que el suelo bajo el charco sea casi llano, porque un MDT de monte tiene hoyos que no son charcos. *A medida*: valores propios. Son puntos de partida que conviene ajustar al MDT. |
| Profundidad mínima | Una mancha de agua menos honda que esto no es un charco. |
| Área mínima | Una mancha menor que esto no es un charco. |
| Suelo casi llano: pendiente media máxima | El suelo bajo el charco debe ser casi llano. |
| Descartar balsas y zanjas: área máxima, profundidad máxima | Quita las masas de agua mayores o más hondas, que son balsas o zanjas. |
| Suavizar el MDT antes | 0: no. En campos con caballones no conviene suavizar, porque borra las calles. |
| Resultados | *Ráster de charcos*, *Profundidad del agua*, *Polígonos (shp)* y *GeoPackage*. |

Se rellenan las depresiones hasta su punto de desborde, y cada mancha de agua más honda que el mínimo es un charco, con su área, profundidad, volumen y cota de desborde. La orden es `hipso ponds`.

###### Inundación

| Control | Qué hace |
|---|---|
| Origen del agua | *Uno o varios puntos* (un río, una rotura, el mar; un punto por línea, «x y» con coordenadas del mapa), *El borde del modelo* o *Bañera: sin conectividad* (se inunda todo lo que está bajo el nivel, llegue o no el agua). |
| Niveles | *Nivel* único o *Una serie* (*Desde*, *hasta*, *cada*; de menor a mayor, con paso positivo y 1000 niveles como máximo). |
| Medidos como | *Cota*, *Altura sobre la celda más baja* o *Altura sobre el origen*. |
| El agua pasa también por las esquinas de las celdas (8 vecinas) | Conectividad de 8 vecinos en lugar de 4. |
| Resultados | *Profundidad por nivel*, *Polígonos de la extensión* y *Tabla (CSV)*. Se guarda también el ráster de alcance, la cota mínima del agua que llega a cada celda, útil como mapa de peligro. |

El agua solo llena lo que puede alcanzar pasando por celdas que no superan el nivel: un hoyo detrás de una loma queda seco. La orden es `hipso flood`.

##### Visibilidad y rutas

Menú con *Cuenca visual* (qué celdas ve un observador a una altura y con un alcance dados) y *Ruta de menor coste* (el camino más barato entre puntos por pendiente, tiempo de marcha o un ráster de costes).

###### Ruta de menor coste

| Control | Qué hace |
|---|---|
| Origen, «x y» | Punto de partida, en coordenadas del mapa. |
| Paradas intermedias | Una por línea, opcional, en orden. |
| Varios destinos: ir al más barato | Con varios destinos, la ruta va al más barato. |
| Destino(s) | Uno por línea. |
| Qué cuesta | *La pendiente* (1 + (pendiente / referencia) ^ potencia por metro), *Tiempo de marcha a pie* (función de Tobler, que distingue subida y bajada), *Nada: la ruta más corta* o *Un ráster de costes por metro*. |
| Potencia, Pendiente de referencia | Con el modelo de la pendiente: exponente, y pendiente (%) a la que el coste por metro se duplica. |
| No cruzar pendientes mayores de | Las celdas más empinadas que esto (%) no se pueden cruzar. |
| Barreras | Polígonos que no se pueden cruzar. |
| Zonas preferidas | Polígonos donde el coste se multiplica por el factor indicado. |
| Guardar también el ráster de coste acumulado desde el origen | Escribe el coste de llegar a cada celda. |

Se mueve por las ocho celdas vecinas, de modo que la ruta puede ser hasta un 8 % más larga que la recta. Da una línea 3D con su longitud, pendientes, subida y bajada. La orden es `hipso leastcost`.

##### Volúmenes

Menú con *Volumen sobre MDT* (volumen entre una superficie y el terreno, celda a celda), *Cubicación de pilas* (volumen de cada pila de material) y *Desmonte y terraplén* (la tierra a quitar y a añadir para llegar a una cota o a una superficie de proyecto).

###### Volumen sobre MDT

| Control | Qué hace |
|---|---|
| Fuente | *CHM de cada nube* (alturas sobre el suelo), *MDS menos MDT (de cada nube)* o *MDS sobre una cota horizontal* (*Cota de referencia*). Con un CHM las alturas ya son sobre el suelo y no hace falta MDT. |
| Altura mínima que cuenta (m) | Las celdas más cerca del MDT que esto no suman: quita el ruido del terreno. |
| Zonas | *Todo el ráster*, *AOI dibujadas* (un volumen por AOI) o *Polígonos de un archivo vectorial* (el resultado es la capa con los volúmenes en su tabla). |

El volumen es V = Σ (altura de la superficie − MDT) · celda². Las celdas por encima del MDT suman al volumen sobre él y las que quedan por debajo, al volumen bajo él (vaciados). Todas las hojas dan una sola tabla (`.csv`); con polígonos de un archivo, una copia de la capa con `Area_m2`, `Vol_sobre`, `Vol_bajo`, `Vol_neto`, `H_media`, `H_max` y `Sin_MDT`.

###### Cubicación de pilas

| Control | Qué hace |
|---|---|
| Modelo de superficie | El MDS, en un sistema métrico. |
| Contornos de las pilas | Capa de polígonos, uno por pila. El contorno debe rodear el pie de la pila: lo que mida a lo largo de él se toma como suelo. |
| Superficie base | *Plano ajustado al contorno (mínimos cuadrados)*, *Cota más baja del contorno*, *Cota mediana del contorno*, *Superficie interpolada desde el contorno*, *Cota fija* o *MDT del terreno de partida*. |
| Altura mínima | Las celdas más bajas que esto sobre la base no cuentan. |
| Densidad del material | Con ella, la tabla da también la masa. |
| Resultados | *Tabla (CSV)*, *Superficie base* y *Altura sobre la base*. |

Mide por pila el volumen, el área en planta y sobre la pendiente, y la altura media y máxima. El volumen se decide por el centro de cada celda: el error es del orden del perímetro por media celda por la altura del borde, y baja con celdas más finas. La orden es `hipso piles`.

###### Desmonte y terraplén

| Control | Qué hace |
|---|---|
| Terreno existente | El MDT de partida. |
| Diseño | *Otro modelo (un MDT de diseño)*, *Plano horizontal a una cota* o *Plano inclinado*. |
| Cota | Cota del plano horizontal. |
| Cota en el origen, Pendiente, Hacia, Origen X / Y | Plano inclinado: cota en un punto, pendiente y orientación hacia la que baja (grados desde el norte, en sentido horario, como correría el agua), con las coordenadas del mapa del punto de origen. |
| Polígonos | Opcional: un resultado por polígono. |
| Tolerancia | Las diferencias de hasta esta cantidad cuentan como sin cambio. |
| Resultados | *Tabla (CSV)* y *Ráster de clases*. |

La diferencia es diseño menos terreno: positiva es terraplén y negativa es desmonte. La orden es `hipso cutfill`.

### 4.8 Pestaña Flujos

La pestaña *Flujos* abre el constructor de flujos, un lienzo donde cada herramienta es un nodo y cada archivo que una produce y otra consume es un hilo. Sustituye el área de vistas mientras está activa, y los trabajos en marcha siguen al cambiar de pestaña. El funcionamiento del lienzo (paleta, puertos, estrellas, problemas y ejecución) está en el apartado 13, *El constructor de flujos*; aquí se explican sus botones.

#### Flujos → Flujo

##### Nuevo flujo

Botón grande. Vacía el lienzo para empezar otro flujo.

##### Abrir flujo

Botón grande. Abre un archivo `.model.json` guardado.

##### Guardar flujo

Botón grande. Guarda el flujo en su archivo, o pide uno si aún no lo tiene.

##### Ejemplos

Botón grande con menú. Carga uno de los seis flujos que trae el programa: *MDT, CHM y árboles*, *MDT y derivados del terreno*, *Volumen de acopios*, *Inventario por área*, *Cambio entre dos fechas* y *Estadística por rodales*. Son el mejor punto de partida porque se ven funcionar enseguida; se explican en *Flujos de ejemplo*.

##### Guardar flujo como…

Guarda el flujo con otro nombre.

#### Flujos → Ejecución

##### Ejecutar

Botón grande. Ejecuta el flujo en segundo plano. No empieza mientras la lista de problemas bajo el lienzo no esté vacía. Cada nodo muestra su estado (en curso, hecho o con error), y si un paso falla se borra todo lo que esa ejecución había escrito.

##### Cancelar

Botón grande. Detiene el proceso en marcha y borra lo escrito a medias.

#### Flujos → Edición

##### Deshacer y Rehacer

Deshacen y rehacen los cambios en el lienzo (Ctrl+Z y Ctrl+Y).

##### Duplicar

Duplica los nodos seleccionados (Ctrl+D).

##### Quitar nodos

Quita los nodos seleccionados (Supr).

#### Flujos → Vista

##### Ordenar

Botón grande. Coloca los nodos en columnas según su distancia a las entradas.

##### Encuadrar

Botón grande. Pone todos los nodos a la vista.

### 4.9 El panel lateral

El panel lateral tiene cuatro pestañas (*Contenido*, *Información*, *Edición* y *Simbología*) que se pueden usar a la vez que la cinta, de manera que *Vista* y las herramientas de edición están abiertas simultáneamente. Lo que muestran *Edición* y *Simbología* depende de la capa elegida en *Contenido*.

#### Panel → Contenido

*Contenido* funciona como la tabla de contenidos de un SIG y su árbol es el orden de dibujo: lo de arriba se dibuja encima.

##### Buscar en el contenido

Filtra el árbol por nombre.

##### Abrir y Mostrar todo

Los dos botones de la cabecera del panel. *Abrir* añade archivos, y *Mostrar todo* enciende todas las capas.

##### El árbol

El árbol tiene tres raíces: *Mapa* (su casilla muestra u oculta todo), las capas (vectoriales, rásteres e imágenes) y el grupo *Nubes de puntos*. El grupo de nubes se comporta como un elemento más y se sube o se baja entero por encima o por debajo de las capas, arrastrándolo o con el botón derecho. Los rásteres e imágenes nuevos se colocan bajo las nubes, y las capas vectoriales sobre ellas. La última fila es *Mapa base* (apartado 4.10). Cada fila tiene su flecha de despliegue (las capas con simbología por valores muestran sus clases debajo), su casilla de visibilidad, su símbolo y su nombre. Una casilla a medias en un grupo indica que solo algunos de sus elementos se ven.

##### Acciones de una fila

Un clic selecciona, el doble clic hace zoom a la capa y el botón derecho abre el menú. Con el ratón encima de una fila aparecen sus acciones: en una capa, *Tabla de atributos* (si es vectorial), *Zoom a la capa* y *Quitar la capa*; en una nube, *Zoom a esta tesela* y *Cerrar esta nube*; en el grupo, *Zoom a todas las nubes* y *Cerrar todas las nubes*. El menú contextual añade *Ver solo esta nube*, *Ver todas*, *Subir* y *Bajar en el orden de dibujo*, *Editar esta capa*, *Guardar* y *Guardar como…* y, en una imagen, *Bandas e índices…*.

##### Selección de varias capas

Con Ctrl (o Cmd) y clic se añade o se quita una capa de la selección, y con Mayús y clic se marca el rango entre la última seleccionada y la pulsada. Con dos o más aparece sobre el árbol una barra con los botones para mostrarlas, ocultarlas, hacer zoom a todas, subirlas, bajarlas y quitarlas, y el botón derecho ofrece lo mismo. La casilla de visibilidad de una capa de la selección enciende o apaga toda la selección, y arrastrar una de ellas mueve el bloque entero conservando el orden entre sí. Las nubes de puntos no entran en esta selección: se manejan una a una o con los botones del grupo.

##### Propiedades al pie

Las propiedades de la capa seleccionada (opacidad, colores) salen al pie del panel.

#### Panel → Información

Muestra el informe de los datos: puntos, extensión, densidad, clases, retornos, fechas, sistema de coordenadas y avisos, y el de los rásteres y capas cargados. Tiene dos botones.

##### Analizar de nuevo

Vuelve a calcular el informe, útil tras editar o añadir datos.

##### Copiar todo

Lleva el informe entero al portapapeles. El botón derecho sobre el texto copia lo seleccionado.

#### Panel → Edición

*Edición* reúne todas las ediciones y cambia con lo que esté elegido en *Contenido*: con una o varias nubes de puntos (o nada) muestra la edición de puntos, y con una capa vectorial, su edición. Con un ráster o una imagen avisa de que no tienen herramientas de edición.

##### Edición de nubes de puntos

El panel se divide en *Selección*, *Editar* e *Historial*. Las ediciones no tocan el archivo original: se guardan al escribir una nube nueva.

La **Selección** tiene ocho herramientas, que se eligen por su icono: *Seleccionar con rectángulo*, *con polígono*, *con lazo* y *con círculo*, *Pincel redondo* y *Pincel cuadrado* (arrastrar para pintar, los puntos tocados se seleccionan, y las teclas [ y ] cambian el tamaño, en planta y en perfil), y *Seleccionar todo lo que queda sobre* o *bajo una línea del perfil*. Cuando una herramienta de selección está activa, el botón izquierdo dibuja la selección y el giro de la vista 3D pasa al botón central. *Todo* (Ctrl+A) selecciona todos los puntos, *Invertir* (Ctrl+I) invierte la selección y *Quitar* (Esc) la limpia. *Combinar selección* decide qué pasa con una selección nueva: *Nueva* sustituye a la anterior, *Añadir* (Mayús) la suma, *Restar* (Alt) la quita e *Intersecar* (Ctrl) deja solo lo común. *Rebanada en 3D ±* limita la selección en profundidad. *Aislar selección* saca los puntos seleccionados a un archivo `…_sel` y deja solo ese en pantalla, con toda su densidad. *Exportar puntos…* exporta la nube, o solo la selección, a LAZ, ASCII o shapefile. *Filtro de atributos de las selecciones* acota la selección por atributos. Debajo se indica cuántos puntos están seleccionados en pantalla y cuántos equivalen en el archivo.

**Editar** tiene *Eliminar* (Supr) para quitar los puntos seleccionados, *Cambiar clase* con un selector de clase y atajos para las más frecuentes (*Suelo*, *Veg. baja*, *Veg. media*, *Veg. alta*, *Edificio*, *Ruido* y *Sin clasificar*), *Quitar duplicados* (apartado 6.4) e *Histogramas* (apartado 6.1). *Guardar nube editada…* escribe una nube nueva con las ediciones, y *Descartar todo* vuelve a la nube original. *Deshacer* (Ctrl+Z) y *Rehacer* (Ctrl+Y) recorren las ediciones, y el *Historial* lista las hechas. Todo el detalle de la edición está en el capítulo 6.

###### Histogramas de atributos

La ventana *Histogramas de atributos* se abre desde el botón *Histogramas* del panel *Edición*.

| Control | Qué hace |
|---|---|
| Atributo | Atributo que se cuenta: elevación, intensidad, retorno, clase, ángulo de escaneo, etc. |
| Barras | Número de intervalos del histograma. |
| Escala logarítmica | Deja ver las barras con pocos puntos, que son las de los valores raros. |
| Solo los puntos seleccionados | Cuenta únicamente la selección. |
| Marcar barras | Un clic o arrastrar marca barras. |
| Valores raros según un estadístico | Marca automáticamente las barras de los valores raros con *Tukey* (fuera de Q1 − k·RIC … Q3 + k·RIC), *Robusto* (lejos de la mediana, con MAD), *Desviaciones típicas* (media ± k·σ) o *Percentiles* (las colas más extremas), con su parámetro (k, z robusto, % en cada cola). |
| Marcar los raros de este atributo, Quitar las marcas de este atributo | Aplican el estadístico elegido o quitan las marcas. |
| Seleccionar puntos | Selecciona los puntos que caen en las barras marcadas de todos los atributos. |
| Seleccionar de cualquier atributo | Selecciona los puntos que caen en las barras marcadas de cualquiera de los atributos. |
| Eliminar, Asignar clase | Elimina los puntos seleccionados o les cambia la clase. |

Debajo del gráfico se ven el número de puntos, el mínimo, el máximo, la media, la desviación típica, la mediana, los cuartiles y el MAD, y cuántas barras y puntos hay marcados. Véase el apartado 6.1.

##### Edición de capas vectoriales

Con una capa vectorial el panel tiene cuatro grupos.

*Capa* tiene *Nueva capa*, *Guardar* (con un asterisco si hay cambios sin guardar) y *Guardar como…*.

*Edición* tiene tres herramientas, *Digitalizar* (dibuja elementos nuevos; sobre una capa de polígonos, un trazo además los adosa o los corta), *Vértices* (se arrastra un vértice, el doble clic sobre un lado crea uno nuevo y el botón derecho sobre un vértice lo quita) y *Mover* (se arrastran los elementos seleccionados, o el que está bajo el cursor). Dos casillas controlan el ajuste del cursor: *Ajustar a vértices* y *Ajustar a bordes* (el punto salta al borde más cercano de cualquier capa visible, aunque no haya un vértice allí). *Reformar*, con *Digitalizar* sobre polígonos, sustituye el tramo más corto del contorno por un trazo que lo cruza dos o más veces (corta una esquina o añade un saliente). *Deshacer* y *Rehacer* recorren las ediciones de la capa.

*Selección* tiene *Seleccionar* (por rectángulo), *Seleccionar todo*, *Invertir selección*, *Limpiar selección*, *Seleccionar por localización* (los elementos que intersectan, están dentro de, contienen, tocan o cruzan a los de otra capa) y *Seleccionar por expresión* (una expresión sobre los campos).

*Geometría y atributos* tiene *Unir seleccionados* (junta los polígonos seleccionados en uno solo, con los atributos del primero), *Separar partes* (convierte cada parte de un elemento de varias partes en un elemento propio), *Agrupar en multiparte* (junta los seleccionados en uno sin unir la geometría), *Transformar selección* (mueve, gira y escala alrededor del centro de la selección), *Copiar*, *Pegar* (en la capa activa, emparejando los campos por nombre) y *Pegar como capa*, *Comprobar topología* (polígonos que se solapan, huecos entre ellos, contornos que se cruzan y, en líneas, cabos sueltos), *Eliminar seleccionados*, *Tabla de atributos* y *Calculadora de campos* (rellena un campo con una expresión). Véase el apartado 9.

###### Calculadora de campos

| Control | Qué hace |
|---|---|
| Campo | El campo que se rellena; si no existe, se crea. *Existentes* ofrece los de la capa. |
| Expresión | Números, textos entre comillas simples, campos (`"especie"`), variables y funciones. |
| Variables | `$area` (superficie del polígono, en unidades de la capa), `$perimeter`, `$length`, `$x` y `$y` (centroide), `$id` (número de fila) y `$npoints` (número de vértices). |
| Funciones | `round(x, 2)`, `floor`, `ceil`, `sqrt`, `upper`, `lower`, `trim`, `concat`, `left`, `right`, `substr`, `replace`, `if(cond, si, no)` y `coalesce`; operaciones `+ - * / % ^`, `\|\|` para unir textos, comparaciones, `and`, `or` y `not`. |
| Solo los seleccionados | Calcula solo los elementos seleccionados. |
| Probar | Evalúa la expresión con algunas filas sin escribir nada. |
| Calcular | Escribe el resultado. |

`$area` está en las unidades de la capa (m² en un sistema proyectado; 1 ha = 10 000 m²).

###### Seleccionar por expresión

Se selecciona los elementos para los que la expresión es verdadera. Usa los campos de la capa y las variables `$area`, `$perimeter`, `$length` y `$npoints`, por ejemplo `"especie" = 'Pino'` o `"n" >= 3 and $area < 5000`. Con *Selección nueva*, *Añadir a la selección* o *Quitar de la selección* se decide cómo se combina con la selección previa.

###### Seleccionar por localización

| Control | Qué hace |
|---|---|
| Elementos de la capa activa que… | La relación: intersectan, están dentro de, contienen, tocan (solo por el borde), cruzan o no tocan. |
| …algún elemento de la capa | La otra capa cargada con la que se compara. |
| Selección nueva, Añadir a la selección, Quitar de la selección | Cómo se combina con la selección previa. |

###### Transformar selección

| Control | Qué hace |
|---|---|
| Desplazamiento en X, en Y | Traslada la selección (unidades de la capa). |
| Giro (grados, antihorario) | Gira la selección alrededor de su centro. |
| Escala | Escala la selección alrededor de su centro (1 la deja igual). |
| Aplicar | Aplica los tres a la vez. |

Se puede deshacer.

###### Comprobar topología

| Control | Qué hace |
|---|---|
| Tolerancia | En polígonos, superficie mínima de un solape o un hueco; en líneas, la distancia a la que dos extremos se tienen por unidos. |
| Comprobar | Lista los incumplimientos (hasta 500) y permite exportarlos; si no hay, dice «Sin incumplimientos». |

En polígonos busca solapes, huecos entre ellos y contornos que se cruzan consigo mismos; en líneas, extremos que no tocan ninguna otra línea (cabos sueltos). La orden es `hipso topology`.

#### Panel → Simbología

Muestra cómo se dibuja la capa elegida en *Contenido*, con una pestaña distinta según su tipo.

##### Simbología de una capa vectorial

*Símbolo* elige el modo de dibujo: *Símbolo único*, *Valores únicos* (un color por valor de un campo), *Colores graduados* (clases por rampa, con *Clases* y *Cuantiles*) o *Tamaño proporcional*. *Colores* fija el color, el trazo y el relleno, la rampa y su inversión y el radio de los puntos. *Aspecto* ajusta la opacidad, el tamaño (puntos), el grosor del trazo (cero en un polígono es sin borde) y el relleno. *Etiquetas* escribe un campo sobre cada elemento, con su tamaño, su color y un *Halo*.

##### Simbología de una imagen

*Bandas* elige las bandas de rojo, verde y azul y abre *Índices espectrales*. *Estirado* reparte el contraste: *Sin estirar*, *Mínimo–máximo*, percentiles 2–98 %, ±2 desviaciones o cortes manuales (*Mín* y *Máx*), con *Brillo*, *Contraste*, *Gamma* y *Restablecer*. *Capa* fija la opacidad.

##### Simbología de un ráster de una banda

*Representación* elige el modo: *Estirar* (rampa continua), *Clasificar* (clases) o *Relieve sombreado*, con la rampa e *Invertir*. *Valores* fija el *Mín* y el *Máx* de la rampa, con *Percentiles 2–98 %* y *Mín–máx* para ajustarlos de un clic, y el número de *Clases*. *Relieve* da el *Azimut* y la *Altura* de la luz, la *Exageración* y el *Color* del sombreado. *Capa* fija la opacidad.

#### La barra de estado

Muestra, de izquierda a derecha, las coordenadas del cursor, el sistema de coordenadas de la vista (el de la primera nube, o «sin declarar»), cuántos puntos hay en pantalla de los del archivo (con la decimación en uso), el rango de elevación, la herramienta activa, la escala y, cuando hay un proceso en marcha, su avance y la estimación de tiempo.

### 4.10 Las vistas

Esta parte explica cómo se navega por el área de vistas y qué hace el programa por debajo para que la navegación sea fluida con nubes grandes. Los botones que gobiernan lo mostrado están en la pestaña *Vista* (apartado 4.4).

#### Navegar en la planta

| Acción | Ratón / tecla |
|---|---|
| Acercar | Rueda, o clic con la herramienta Zoom (×2) |
| Alejar | Clic derecho con Zoom |
| Zoom a una ventana | Arrastrar con Zoom |
| Desplazar | Herramienta Desplazar (X) o botón central |
| Zoom anterior | Botón central, o *Zoom anterior* |
| Extensión completa | *Extensión completa* |

La ampliación a la selección (*Vista → Navegación*, *Zoom a la selección*) encuadra los puntos seleccionados o, si la capa activa es vectorial, los elementos seleccionados. En la vista 3D, *Refrescar 3D* la vuelve a encuadrar en lo que se ve en la planta, algo útil cuando hay capas muy separadas entre sí. Al abrir varios archivos a la vez se cargan todas las capas juntas, y quitar una nube ya no recarga las demás.

##### Detalle al acercar

La vista dibuja una muestra uniforme de la nube, y esa muestra se queda corta cuando la ventana es pequeña. Por ese motivo el programa lee en segundo plano los puntos de la ventana a más detalle y los dibuja mientras cubran la vista, tanto en la planta como en la vista 3D (en 3D, el suelo que abarca la cámara alrededor de su objetivo, con la nube base dibujada fuera de esa zona). Funciona con cualquier LAS/LAZ, no solo con COPC: la primera vez que se abre una nube que ha habido que muestrear, el programa crea en segundo plano una copia COPC en la carpeta de caché del sistema (`~/Library/Caches/HIPSO/copc` en macOS), sin tocar el original; el registro avisa con «Preparando el detalle al acercar» y «Detalle al acercar listo». La copia se reconoce por la ruta, el tamaño y la fecha del original. Se lee de ella el nivel del octree cuyo espaciado es de un punto por píxel, hasta unos 3 millones de puntos, y se vuelve a leer cuando el zoom entra al menos 1,8 veces más. Solo ocurre cuando la ventana ocupa menos de la mitad del área de la nube. La caché guarda hasta 20 GB: pasado ese tope se borran las copias que llevan más tiempo sin usarse, nunca las de las nubes abiertas. La carpeta y el tope se cambian en *Inicio → Ajustes* (véase *Ajustes*, en el capítulo 4).

##### Rásteres en 3D

Un MDT, un CHM o cualquier ráster de una banda (también los grandes, con su copia reducida) se dibuja en la vista 3D como una superficie, con los colores de su simbología y, si esta no trae relieve sombreado, con una luz sencilla. Se combina con una nube de puntos en la misma vista. Se dibuja en el procesador, sin depender de la tarjeta gráfica: con una malla de 4 millones de celdas tarda unos 9 ms por imagen (1400 × 900, Mac M4). La exageración vertical se cambia en *Vista → Vista 3D*.

##### Abrir desde la copia

Cuando una nube ya tiene su copia COPC, el programa saca la muestra de pantalla de los primeros niveles del octree en vez de leer el archivo entero: QULTIVA Z2 (51 millones de puntos) pasa de 14 s a 0,4 s, y el TLS de 94 millones, de 4 s a 0,2 s. La primera apertura de una nube grande sigue leyéndola entera. Con nubes de más de unos 30 millones de puntos la copia se escribe en dos pasadas para gastar menos memoria (QULTIVA Z3 de 90 millones: unos 4,5 GB y 28 s), y por encima de 100 millones por celdas: el cubo de la nube se parte en las celdas de un nivel de su octree, los registros de cada celda esperan en un archivo temporal junto a la copia, y cada celda se construye y se escribe por separado, de modo que la memoria es la de una celda (una décima parte de la RAM) y no depende del tamaño de la nube. Una de las nubes de escáner terrestre de 226 millones de puntos (7,8 GB) se convierte en 5,7 minutos (casi todo es lectura del disco externo) con 1,3 GB de memoria, y da una copia de 1,9 GB que después se abre en 0,27 s y da detalle al acercar en 0,1 s. Hace falta disco libre para el archivo temporal (el registro avisa si no hay); si no hay memoria o disco suficientes, la copia no se crea y el registro lo dice.

##### Rellenar huecos

Las líneas de vuelo dejan puntos muy juntos a lo largo de la pasada y muy separados entre pasadas, y la imagen se vería a rayas. Por eso el programa da a cada píxel vacío el color del punto más cercano, hasta dos veces el espaciado de los puntos en pantalla (de 2 a 8 píxeles), en la planta y en el 3D. Solo rellena entre puntos que están a la misma profundidad (un hueco entre un tronco y el follaje de detrás se respeta), solo en nubes aéreas (las que traen ángulo de escaneo: en una nube de escáner terrestre, con puntos dispersos en todas direcciones, rellenar solo forma bloques) y solo cuando la vista está quieta, y su coste es lineal en los píxeles (unos milisegundos). No tiene control en la interfaz: actúa siempre. La planta pinta siempre lo más alto encima.

#### Mapa base

Se enciende y se apaga en el panel *Contenido*, con la casilla de la última fila (*Mapa base*). Es un mapa de fondo que queda siempre bajo todas las capas, colocado en el sistema de coordenadas de la nube (por lo que la primera nube debe declararlo). Las fuentes son OpenStreetMap (mundial), la ortofoto PNOA y el mapa base del IGN (ambos de España) o bien otro servidor de teselas con una dirección del tipo `https://…/{z}/{x}/{y}.png`.

Las teselas se descargan en segundo plano de los servidores del proveedor (con el mapa apagado no se descarga nada), con el nombre del programa en la petición y como mucho dos descargas a la vez. Se guardan una semana en la carpeta de caché del sistema (en macOS, `~/Library/Caches/HIPSO/tiles`), por lo que se reutilizan sin conexión; *Inicio → Ajustes → Borrar caché* las elimina. El crédito del proveedor se dibuja en la esquina del mapa. Conviene respetar los términos de cada proveedor, y en particular OpenStreetMap no admite descargas masivas.

#### Capas en otros sistemas de coordenadas

Al cargar, el visor pasa al sistema de la primera nube todo lo que esté en otro, y lo hace de tres maneras según el tipo de dato. Las demás nubes se convierten (se guarda una copia `<nombre>_epsg<código>.laz` junto a la original, que la próxima vez se carga sin convertir). Los rásteres y las capas vectoriales se transforman en memoria. Las imágenes de varias bandas se colocan convertidas, a la resolución de su vista previa.

Solo cuando hay capas en sistemas distintos, el registro avisa antes de la carga de que tardará más, y después dice qué se ha convertido y cuánto ha tardado. Las capas abiertas antes de cargar la nube se convierten cuando la nube fija el sistema. Una nube sin sistema declarado se carga tal cual (con aviso), y una nube en coordenadas geográficas (grados) no se convierte.

El coste está medido con hojas reales de 8 y 9 millones de puntos. Cargar las dos juntas en el mismo sistema tarda 1,1 s. Si una está en otro huso o en ED50, la primera vez tarda 2,1 s (unos 1,0 s más por cada 8 millones de puntos convertidos) y las siguientes 1,1 s, porque la copia convertida ya existe.

#### Navegar en 3D

| Acción | Ratón / tecla |
|---|---|
| Girar (sin tope, también por debajo) | Botón izquierdo |
| Mover en el plano de la pantalla | Botón derecho o central (o Mayús + izquierdo) |
| Acercar y alejar | Rueda, o Ctrl + izquierdo |
| Girar con el teclado | Flechas |
| Acercar con el teclado | Re Pág / Av Pág |
| Encuadrar todo | Doble clic o tecla Inicio |
| Norte arriba | Doble clic en la brújula de la esquina de la vista 3D, o la tecla N |
| Exageración vertical | Campo *Exageración* de *Vista → Vista 3D* (de × 0,1 a × 100) |

El acercamiento llega hasta 2 cm del centro de giro, lo suficiente para ver un tronco o una rama punto a punto. Con una herramienta de selección activa el botón izquierdo deja de girar, porque dibuja la selección (véase el capítulo 6), y el giro pasa al botón central; el derecho sigue moviendo y la rueda sigue acercando.

### 4.11 Ajustes

*Inicio → Ajustes* reúne lo que el programa guarda en el disco para ir más deprisa. Hoy es la **caché**: las copias COPC con el detalle de las nubes grandes (la primera apertura las crea en segundo plano y las siguientes las encuentran) y las teselas descargadas del mapa base. La página muestra la *carpeta de la caché* (que se escribe, se elige con *Elegir carpeta…* o se devuelve a la de siempre con *Por defecto*), el *tamaño máximo de las copias* en gigabytes (20 GB por defecto; al superarlo se borran las copias que más tiempo llevan sin usarse, y las de las nubes abiertas se respetan) y lo que ocupan ahora las copias y las teselas.

*Borrar caché…* abre un aviso que explica lo que implica antes de borrar nada: los archivos originales, los proyectos y los resultados no se tocan, y lo único que cambia es que la copia de una nube grande tarda de nuevo en prepararse la próxima vez que se abra, y que el mapa base vuelve a descargarse. Las copias de las nubes abiertas en ese momento se conservan, y no se puede deshacer. Cambiar de carpeta no mueve las copias que ya existen (se rehacen en la nueva al abrir las nubes), y en una carpeta elegida por el usuario el programa solo borra sus propios archivos (`*.copc.laz` y las copias a medio escribir), nunca otros que haya en ella. Los ajustes se guardan en el archivo de preferencias; la variable `HIPSO_CACHE_DIR` sigue fijando la carpeta por defecto, y lo elegido en la página tiene preferencia sobre ella.

## 5. Herramientas de consulta

Las herramientas de consulta no modifican la nube, solo la interrogan, y se agrupan según lo que miden.

### Consultar (Q)

Muestra los atributos del punto sobre el que se hace clic.

### Medir (W)

Da distancias y áreas sobre la planta.

### Densidad

Calcula los puntos por metro cuadrado de un recuadro.

### TIN sombreado (T) y curvas de nivel

Dibujan el relieve de la nube cargada.

### Perfil

Se crea con tres clics (extremo izquierdo, extremo derecho y borde de la caja).

Desde la 0.7.5, los ráster visibles (MDT, MDS…) se dibujan como una línea sobre el perfil, muestreados a lo largo del eje de la caja, con su nombre en la esquina inferior derecha y el eje vertical ampliado para abarcarlos. Si mezclas alturas absolutas con una nube o un ráster normalizado, la escala sale desproporcionada: no son comparables. La casilla *Cuadrícula* del grupo *Perfil* (*Vista → Perfil*) dibuja líneas horizontales con las cotas en el lado izquierdo; se puede quitar.

El perfil merece una explicación más detenida, porque no se comporta como el resto de la vista. Las flechas lo desplazan a lo largo de la nube y su ancho se ajusta en *Vista → Perfil*. Lo decisivo es que **no usa los puntos decimados de la planta y el 3D**: lee de los archivos, en segundo plano, **todos** los de la franja de su caja (y un margen lateral de tres anchos, para poder moverla sin releer; el margen se encoge en nubes muy densas, hasta cero, antes de renunciar a un solo punto). El tope de memoria es un octavo de la RAM del equipo (unos 89 millones de puntos en uno de 16 GB); solo si la caja por sí sola lo supera se dibuja 1 de cada 2, 4… y el registro lo dice. En el TLS, una caja de 300 × 10 m tiene 9,2 millones de puntos y se lee entera en 0,7 s. Mientras llegan se dibuja con los de pantalla, y arriba a la derecha se indica cuántos puntos hay ('leyendo todos…' mientras tanto). La caja del perfil se marca en rojo sobre la planta, pero solo mientras la ventana del perfil está a la vista (diseños *2D + perfil* y *2D + perfil + 3D*): en cualquier otro diseño la marca desaparece, y reaparece al volver a uno que lo incluya.

El tiempo de esa lectura depende del formato. Con la copia COPC en caché (o en una nube COPC) solo se leen y descomprimen los nodos que tocan la zona, con todos los núcleos; sin ella, en un LAS/LAZ normal hay que recorrer el archivo (unos segundos en nubes grandes), saltando los archivos que no tocan la zona. El tamaño de los puntos del perfil es propio (2 px por defecto, con los botones − y + de *Vista → Perfil*) y no depende del de la planta y el 3D. La casilla *Sin decimar* de *Vista → Perfil* controla esta lectura completa: desmarcada, el perfil usa los puntos de la pantalla.

## 6. Edición de nubes de puntos

La edición es por selecciones: primero se eligen puntos, y después se les aplica una acción. Todas las herramientas están en el panel lateral, pestaña *Edición* (*Selección*, *Editar* e *Historial*), mientras que la navegación (zoom, desplazar, extensión, zoom anterior y *Limpiar marcas*) está en la cinta *Vista → Navegación*. El proceso completo tiene seis pasos, y cada uno se apoya en el anterior.

La región es un rectángulo, un círculo, un polígono o un lazo dibujado en la planta, en el perfil o en la vista 3D. En el perfil se seleccionan puntos por su altura (copas, ruido por encima o por debajo del suelo), y en la planta por su posición. La combinación con la selección previa puede ser *Nueva*, *Añadir*, *Restar* o *Intersecar* (en el panel *Edición*, sección *Selección*, donde además se puede invertir la selección), y con el teclado Mayús añade, Alt resta y Ctrl interseca (*Inicio → Atajos de teclado*). El filtro de atributos (clases, rango de Z, rango de intensidad y tipos de retorno) hace que solo se seleccionen los puntos que lo cumplen.

La acción es *Eliminar* o *Asignar clase*, y la lista incluye todas las clases, también las nuevas. *Deshacer* y *Rehacer* son los últimos botones de *Editar* (Ctrl+Z y Ctrl+Y), y el historial de ediciones añade una posibilidad que no suele existir en otros programas: cada edición tiene una casilla, y al quitarla se deshace esa edición sola, sin tocar las demás; al volver a marcarla se recupera. Una edición posterior que elige por clase ve los puntos tal como están en ese momento. Las ediciones desactivadas no se guardan, y *Descartar todo* vuelve a la nube original.

Al guardar la nube editada se escribe un archivo nuevo, y **el original nunca se modifica**. Las ediciones se aplican a todos los puntos del archivo, no solo a los que se ven en pantalla. Cuando hay puntos seleccionados, las herramientas de *LiDAR* trabajan únicamente sobre ellos.

### Pincel y selección sobre o bajo una línea

Además de las formas, hay dos pinceles (*Pincel redondo* y *Pincel cuadrado*) y dos herramientas de línea para el perfil. Con el pincel se arrastra sobre la planta o el perfil y se selecciona todo lo que el trazo toca; el tamaño se da en píxeles de pantalla (deslizador del panel *Edición*, o las teclas `[` y `]`), de modo que se acerca el zoom para retoques finos y se aleja para pasadas anchas. El trazo se convierte en metros al soltar el botón, y la edición guardada es la misma que se vio. Sirve para quitar un pájaro, un cable suelto o ruido pegado a un tronco sin dibujar un polígono cada vez. *Sobre una línea* y *Bajo una línea* se usan en el perfil: se arrastra para trazar una línea y se selecciona todo lo que queda por encima (o por debajo) a lo largo de ella, dentro de la caja del perfil; es la forma más rápida de limpiar puntos bajos bajo el suelo o ruido alto sobre las copas. Los pinceles y las líneas no funcionan en la vista 3D, donde sigue el lazo con rebanada. Como las demás selecciones, se combinan con Mayús, Alt y Ctrl.

### Seleccionar en la vista 3D

Mientras haya una herramienta de selección activa (rectángulo, círculo, polígono o lazo), la vista 3D también selecciona, y el trazo se dibuja directamente sobre la pantalla. Aparece aquí una complicación que la planta no tiene: el trazo es un cono de visión que atraviesa toda la escena, de modo que recoge los puntos que caen dentro del dibujo, tanto los de delante como los de detrás. En un tronco rodeado de ramas por todos los lados, eso significa seleccionar también las del lado contrario. La casilla *Rebanada en 3D ±* lo resuelve: limita la selección a una lámina, paralela a la pantalla, de los metros indicados a cada lado del centro de giro (el punto que queda en el medio de la vista al girar). Basta llevar el tronco al centro, fijar 0,5 m y dibujar el lazo para que solo cuenten las ramas próximas al plano del tronco.

La selección es geométrica, no depende de lo que se ve, y por eso vale igual para los puntos de la pantalla (una parte de la nube) que para todos los del archivo al guardar. La comprobación sobre una nube de 94 millones de puntos lo confirma: una selección en 3D sobre 7,85 millones de puntos tarda unos 12 ms, y el recuento sobre el archivo completo coincide con el estimado con una diferencia inferior al 0,03 %.

### Aislar selección

Una escena completa se muestra decimada (en esa nube, un punto de cada doce), y a esa densidad una rama fina apenas existe. El botón *Aislar selección* (panel *Edición*) escribe los puntos seleccionados en un archivo `…_sel` y deja en pantalla solo ese, con toda su densidad (aislar 1,6 millones de puntos de los 94 tarda unos 4,5 s). Podar un árbol de escáner terrestre pasa a ser corto: un círculo en la planta alrededor del árbol, *Aislar selección*, giro hasta ver las ramas de perfil, lazo en 3D (con rebanada si hace falta), *Eliminar* y guardar. Si hay cambios sin guardar, el botón se niega, para no perderlos. Cuando la nube viene de la herramienta *Medición de troncos*, con los puntos del tronco y de las ramas en clases distintas (capítulo 7.3), el filtro de clase evita incluso el trazo: se seleccionan las ramas por su clase.

### 6.1 Histogramas: detectar puntos raros

*Edición → Editar → Histogramas* abre una ventana con el histograma de un atributo de los puntos en pantalla: elevación, intensidad, número de retorno, número de retornos, clase, pasada (ID de fuente) y ángulo de barrido. Cada barra cuenta los puntos de un intervalo.

Las barras se marcan con un clic o arrastrando sobre varias (si el arrastre empieza en una barra marcada, se desmarcan). Se pueden marcar barras en varios atributos a la vez, y un punto se selecciona si cae en las barras marcadas de todos ellos; los atributos con marcas llevan un punto (●) en la lista. El botón de selección convierte las marcas en una selección, que se combina con la anterior según el modo *Combinar selección* de la cinta y respeta el *Filtro de atributos*, y después se puede *Eliminar* o *Asignar clase* desde botones de la propia ventana.

La escala logarítmica está activa por defecto, porque deja ver las barras con pocos puntos, que son precisamente las de los valores raros. *Solo los puntos seleccionados* hace el histograma únicamente de la selección actual, para afinarla. Bajo el gráfico se muestran el número de puntos, el mínimo, el máximo, la media, la desviación típica, la mediana, los cuartiles y la MAD, y la línea blanca de la base marca la mediana.

Buscar los valores raros a ojo es poco reproducible, por lo que la ventana ofrece cuatro criterios estadísticos cuyos límites se dibujan como líneas naranjas.

| Criterio | Límites | Cuándo usarlo |
|---|---|---|
| Tukey | Q1 − k·RIC y Q3 + k·RIC (k = 1,5; con 3, solo extremos) | General; no le afectan los propios valores raros |
| Robusto (MAD) | mediana ± z·1,4826·MAD (z = 3,5) | Distribuciones muy asimétricas |
| Desviaciones típicas | media ± k·σ (k = 3) | Datos de forma similar a la normal |
| Percentiles | el p % más bajo y más alto | Cuando se quiere una cantidad fija de puntos |

*Marcar los raros de este atributo* los marca como barras, y *Seleccionar los raros de cualquier atributo* aplica el criterio a todos los atributos y selecciona los puntos raros en alguno de ellos. Si un atributo casi no varía (casi todos los valores son iguales), el criterio no puede decidir y lo avisa. Los estadísticos se calculan sobre los puntos que se ven en pantalla, pero al guardar, la regla se aplica a todos los puntos del archivo. El ángulo de barrido se compara en el visor con una décima de grado de resolución.

### 6.2 Clases ASPRS y clases nuevas

HIPSO usa la tabla vigente de ASPRS (LAS 1.4): 0 nunca clasificado, 1 sin clasificar, 2 suelo, 3 a 5 vegetación baja, media y alta, 6 edificio, 7 punto bajo (ruido), 9 agua, 10 ferrocarril, 11 carretera, 13 y 14 cables, 15 torre, 16 conector, 17 tablero de puente, 18 ruido alto, 19 estructura elevada, 20 suelo ignorado, 21 nieve y 22 exclusión temporal. Los códigos 23 a 63 están reservados, y 64 a 255 son libres para el usuario.

Para añadir una clase se abre *Vista → Visualización → Filtros → Clases y colores…*, se escribe el código y el nombre, y se pulsa *Añadir*; el color se cambia con el selector. Las clases propias se guardan para la próxima vez. Un detalle de formato condiciona su uso: los LAS anteriores a la versión 1.4 solo guardan clases hasta la 31, y por eso, si se guarda una edición con una clase mayor, el archivo se escribe como LAS 1.4 automáticamente.

### 6.3 Alturas sobre el suelo

*Preparar → Alturas sobre suelo* (`hipso hag`) calcula la altura de cada punto sobre el suelo a partir de los puntos de suelo (clase 2) de la propia nube. Una TIN de ellos da el suelo bajo cada punto, y no hace falta, por tanto, un MDT aparte. Si la nube no tiene suelo clasificado, la herramienta se niega y lo dice (hay que clasificarla antes).

El resultado es una copia nueva (`…_alturas`) con la altura como atributo extra HeightAboveGround (decimal, legible por otros programas) y las cotas intactas. Con la opción de normalizar (`--replace-z`), la altura sustituye a la cota (`…_normalizada`) y se descartan los puntos sin suelo debajo. La celda del suelo (0,5 m) fija el detalle, y en una hoja de 9 millones de puntos el cálculo tarda unos 3 s. En el borde de la nube el suelo se rellena, y la altura puede variar unos centímetros.

### 6.4 Quitar duplicados

*Edición → Editar → Quitar duplicados* elimina los puntos con las mismas x, y, z (y, si se marca, la misma clase, tiempo GPS, intensidad, retorno o pasada), dejando uno de cada grupo o ninguno. Es útil en nubes unidas de hojas que se solapan, o en entregas con pulsos repetidos, porque los duplicados exactos distorsionan la densidad y las métricas. Su límite es que solo quita coincidencias exactas: los puntos casi coincidentes (ruido de milímetros) no se detectan. Escribe un archivo nuevo.

La edición, en resumen, nunca toca el original y siempre es reversible hasta el momento de guardar. Esa es la propiedad que permite trabajar con pocos clics sin miedo a equivocarse, y de ella dependen todos los procesos del capítulo siguiente.


## 7. LiDAR

La pestaña LiDAR reúne las herramientas en cinco grupos, ordenados como suelen usarse en un trabajo real. *Preparar* contiene *Clasificar*; *Asignar proyección*, *Exportar puntos* (también en el panel *Edición*, junto a la selección que exporta, y con el formato COPC) y *Recortar con polígonos*; *Adelgazar nube*, *Alturas sobre suelo* y *Quitar duplicados*; *Colorear con imagen*, *Normalizar intensidad* y *Imagen de intensidad*. La información de las nubes y de las demás capas está en la pestaña *Información* del panel lateral. *Calidad* reúne *Comparar nubes*, *Ajustar nubes*, *Entre pasadas*, *Líneas de vuelo* y *Densidad de Voronoi*. *Superficies* contiene *MDT*, *MDS / CHM* y *Cambio de la cubierta vegetal*. El relieve, la hidrología, las rutas y los volúmenes están en la pestaña *Raster*, grupo *Análisis del terreno*, porque trabajan sobre cualquier MDT y no solo sobre una nube, y se describen en el apartado 7.2. El inventario de la vegetación se divide según el tipo de nube. El grupo ALS (nubes aéreas) tiene tres botones grandes (*Extraer muestras*, *Métricas por celda* y *Modelos lineales*) y dos pequeños (*Segmentar individuos* y *Análisis por objetos*, OBIA). El grupo TLS (escáner terrestre) tiene el botón *Medición de troncos*. Por último, *Infraestructuras* contiene *Corredores eléctricos*.

La cinta se adapta al ancho de la ventana. Si no caben todos los grupos, los de la derecha se pliegan en un botón grande con su nombre y una flecha (▾) que despliega sus herramientas, así que todas quedan siempre a un clic.

Las herramientas procesan cada hoja por separado y dan un resultado por hoja, sin unirlas nunca (salvo el mosaico, si se pide). Los nombres de los resultados se calculan a partir de la entrada y de los ajustes (`datos_1_mdt_1m.tif`), y no pisan resultados anteriores (se añaden `_2`, `_3`…).

### Cuánto falta

Mientras un proceso corre, la barra de estado muestra el tiempo transcurrido y, si el proceso sabe cuánto trabajo tiene, el porcentaje y una estimación de lo que queda (`42 % · quedan ~ 3 min 20 s`). El *Registro* repite esa línea cada 30 segundos, también al cargar nubes grandes, para poder distinguir un trabajo que avanza de uno colgado. La estimación es aproximada, porque se calcula con la velocidad media hasta ese momento, y mejora a medida que avanza. Los procesos que no declaran su tamaño solo muestran el tiempo.

### 7.1 Preparar

Esta primera etapa existe porque casi todo lo que sigue (modelos, métricas, segmentación) depende de que la nube tenga sistema de coordenadas, una densidad homogénea y el suelo bien clasificado. Un error en este paso se arrastra, sin avisar, a todos los productos posteriores.

#### Información de los datos

Resume los puntos, la extensión, la densidad, las clases, los retornos, las fechas, el sistema de coordenadas y los avisos.

#### Asignar proyección

Da a una nube sin sistema de coordenadas el de otra nube o un código EPSG. Escribe una copia (`…_crs`), y los puntos no se tocan, solo la cabecera.

#### Adelgazar nube

Reduce la densidad sin cambiar la extensión. Se usa para llevar a la misma densidad nubes de coberturas distintas (por ejemplo, la primera y la tercera cobertura del PNOA) antes de compararlas o de calcular métricas, puesto que casi todas las métricas dependen de la densidad de puntos, y también para aligerar nubes demasiado densas. Admite varios criterios. La *densidad objetivo* es el número de puntos por metro cuadrado que deben quedar, elegidos al azar (una nube menos densa se deja igual). *Un punto por celda* conserva el más cercano al centro, el más bajo, el más alto o el del percentil elegido (50 es la mediana); el más bajo conserva el suelo, y el más alto, las copas. Las *celdas 3D* conservan paredes y pendientes. Quedan, por último, *uno de cada n*, *eliminar uno de cada n* y *muestreo al azar*. Su ventaja es que constituye la forma más simple de homogeneizar, y su límite, que descarta información de forma irreversible, por lo que trabaja siempre sobre una copia.

#### Clasificar

Aplica los pasos que se elijan, por teselas y en este orden: solape entre pasadas, ruido, suelo, edificios, vegetación por alturas, cables y postes, carreteras y agua, y por último las reglas. La ventana de clasificación tiene tres pestañas: *Clasificación* (los pasos, uno por apartado y en el mismo orden en que se ejecutan), *Reglas* y *Teselas y rendimiento*. En cada paso con varios métodos (ruido, suelo) se elige uno con un botón de selección. Las edificaciones van antes que la vegetación porque la vegetación solo toma los puntos que no son suelo ni edificio, y las reglas van al final porque ven y pueden cambiar lo que hizo cualquier paso anterior.

##### Una clasificación para distintos sensores

Cada paso es independiente: se activan solo los que hacen falta y se escribe el orden y los valores en una receta (véase *Recetas*), de modo que la misma herramienta sirve para una hoja del PNOA, para un vuelo de dron y para una línea eléctrica. Lo que depende de la densidad de puntos o de la escala del sensor se ajusta solo:

| Paso | Qué se ajusta solo |
|---|---|
| Cables | El radio del entorno es seis veces la separación de los puntos de la tesela (de 0,6 a 3 m) y, en una nube dispersa, también una vez y media y dos veces ese, y los trozos de conductor que un vuelo con pocas pasadas ve separados se unen si van en fila |
| Postes y torres | Los puntos que puede tener una torre crecen con la densidad (en estructuras de 15 m o más), y los cultivos bajos al pie no cuentan como copa |
| Carreteras y agua | El límite de intensidad es una parte de la intensidad mediana del suelo de la tesela, de modo que vale con 16 bits o con valores de 0 a 255 |
| Solape | No aparta las celdas a las que llegan más de dos pasadas (un vuelo con pasadas repetidas las suma) |

Lo que no se ajusta solo es lo propio de cada terreno y de cada uso: el método de suelo y su umbral, la altura de los estratos de vegetación, el margen de los edificios y las reglas. Se eligen en la ventana, se guardan en una receta y se aplican igual a todas las hojas de un bloque. Los pasos se han medido con hojas del PNOA (clasificación del proveedor, que no es una verdad) y con las nubes etiquetadas a mano de DALES (aéreas, de 9 y 45 puntos/m²). Se han probado, sin etiquetas a mano, con un vuelo de dron de unos 100 puntos/m² y con tiras sobre líneas eléctricas de 290 y 471 puntos/m², y no se han probado con escáneres móviles ni terrestres. Lo medido está en cada paso y en el apartado de recetas.

##### Solape entre pasadas (clase 12)

Donde dos pasadas de vuelo cubren el mismo terreno, sus puntos lo describen dos veces. Este paso aparta los puntos de las pasadas que sobran y los pasa a la clase 12 (la clase ASPRS de puntos de solape), de modo que ninguno de los pasos siguientes (ruido, suelo, edificios...) los usa, ni tampoco las reglas ni el MDT.

El terreno se divide en celdas cuadradas (5 m por defecto). En una celda a la que llegan varias pasadas, la que tiene el menor ángulo de barrido medio, en valor absoluto, es la que mejor la ve (la más cercana al nadir) y se queda con la celda; los puntos de las demás son solape. Una celda con una sola pasada no tiene solape, y un archivo sin identificador de pasada ni ángulo de barrido tampoco. Tampoco la tiene una celda a la que llegan más de dos pasadas: el solape lateral entre tiras pone dos pasadas sobre una celda (tres en las esquinas), mientras que un vuelo con varias pasadas sobre el mismo terreno (un dron, vuelos cruzados) las suma, y apartarlas quitaría puntos a los cables y a las estructuras finas. En una tira de cuatro pasadas sobre la misma zona (290 puntos/m²) el paso aparta ahora el 1,3 % de los puntos, y antes apartaba el 73 %. Se activa con *Apartar el solape* en la ventana de clasificación o con `--overlap` (`--overlap-cell`) en la línea de comandos.

##### Ruido

Hay tres métodos, que se eligen en la ventana de clasificación y marcan como ruido (clase 7) los puntos que descartan.

###### Puntos aislados

Son los puntos que apenas tienen vecinos en un bloque de 3 × 3 × 3 celdas. Es rápido y basta para los puntos sueltos muy por encima o por debajo del grueso de la nube (pájaros, reflejos, errores del sensor), pero no detecta ruido en nubes dispersas.

###### SOR (filtro estadístico)

Descarta el punto cuya distancia media a sus *k* vecinos supera la media general más *m* desviaciones típicas (Rusu et al., 2008); se adapta mejor a densidades que cambian de una zona a otra, aunque cuesta más y puede borrar puntos legítimos en bordes y estructuras finas.

###### ELM (mínimo local extendido)

Busca el ruido por debajo del suelo: agrupa los puntos en celdas cuadradas (10 m por defecto) y toma el punto más bajo de cada una. Ese punto es ruido bajo cuando queda al menos un umbral (3 m por defecto) por debajo del cuartil inferior de las alturas de su propia celda y, a la vez, por debajo de la mediana de los puntos más bajos de las ocho celdas que la rodean; la segunda condición evita marcar el suelo legítimo de una vaguada. Se repite sin los puntos ya marcados, de modo que una ráfaga de puntos bajos en una celda se retira capa a capa. Es una variante propia del mínimo local extendido y no equivale al filtro del mismo nombre de otros programas. Contrastado con una hoja PNOA de 1 km² (12,9 millones de puntos) no marca ningún punto legítimo, y con 901 puntos de ruido bajo añadidos a 8 m por debajo del suelo en una zona de 400 × 400 m detecta el 96 % con celdas de 10 m (con celdas de 20 m, en terreno con relieve, baja al 13 %: en celdas grandes el relieve esconde el ruido). Se elige en la ventana de clasificación (*Ruido → Puntos bajos (ELM)*, con la celda y la bajada mínima) o en la línea de comandos con `--noise elm` (`--elm-cell`, `--elm-threshold`).

##### Suelo (clase 2)

No existe un método mejor en todos los casos, porque la elección depende del relieve, de la densidad y de la vegetación. Lo razonable es probar dos o tres en una hoja representativa y comparar los MDT resultantes, mirando el sombreado y no solo las cifras.

| Método | Cuándo es más útil | Ventajas | Problemas |
|---|---|---|---|
| Iterativo de Kraus y Pfeifer | Bosque con suelo visible entre los árboles y relieve suave o medio | Pocos parámetros; robusto frente a la vegetación baja porque va dando menos peso a los puntos altos | Terreno muy abrupto y edificios grandes; en bosque muy denso puede dejar el suelo algo alto |
| Curvatura multiescala (MCC) | Bosque cerrado con sotobosque y relieve ondulado | Pensado para bosque; resiste la vegetación baja | Sensible al umbral de curvatura y al número de escalas; más lento |
| Simulación de tela (CSF) | Terreno de pendiente suave a media, con poca preparación | Pocos parámetros intuitivos (rigidez, resolución de la tela); rápido | En laderas muy pronunciadas o con escarpes la tela «cuelga»; hay que bajar la rigidez en relieve (1) y subirla en llano o zona urbana (3). Con nubes poco densas (1 a 2 puntos/m²) conviene subir la resolución de la tela a 1 o 2 m, o se cuela vegetación en el suelo, a costa de perder algo de suelo en las laderas |
| Morfológico progresivo (PMF) | Zona urbana o mixta, también con pendiente | Muy rápido; fácil de entender; de los mejores en la prueba de la ISPRS | La ventana máxima debe superar al edificio más grande; en bosque denso con pocos puntos de suelo toma vegetación baja por suelo |
| Morfológico simple (SMRF) | Terreno con pendiente y objetos de tamaños distintos; nubes grandes | El más rápido (de 3 a 5 veces más que PMF y de 15 a 30 más que CSF en recortes de 1,3 a 5,5 millones de puntos); el mejor en la prueba de la ISPRS; la pendiente máxima del terreno es un parámetro con sentido físico | Con pocos retornos de suelo bajo el bosque denso rellena con el suelo más cercano; ventana cuadrada |
| Axelsson (TIN progresivo) | Nubes densas y de poco ruido, con edificios y terreno con pendiente | Densifica el suelo respetando el relieve; método clásico | Si hay estructuras mayores que la celda de las semillas hay que subirla; más parámetros; es el más lento y, con los valores por defecto, el que más vegetación baja toma por suelo en bosque denso (véase *Ajuste frente al PNOA*) |
| Clase 2 existente | La entrega ya viene clasificada (por ejemplo, PNOA) | Es lo más rápido y no cambia nada | La calidad es la del proveedor; no se puede mejorar sin reclasificar |

###### Iterativo de Kraus y Pfeifer

Ajusta una superficie por mínimos cuadrados con pesos; los puntos muy por encima de ella pesan menos en cada vuelta, así que la superficie 'baja' hasta el suelo, y después se aceptan los puntos que están a menos de una distancia de ella (Kraus y Pfeifer, 1998).

###### Curvatura multiescala (MCC)

Compara cada punto con una superficie curva ajustada a varias escalas, y el que se desvía menos que el umbral es suelo (Evans y Hudak, 2007). Contrastado con la implementación de lidR sobre cinco escenas sintéticas (llano, ladera de 25°, colinas, una ciudad y un bosque), coincide con ella en el 92 al 100 % de los puntos y comparte sus límites: en ladera fuerte y en bosque denso pierde del 3 al 7 % del suelo, y toma por suelo el centro de los tejados anchos.

###### Simulación de tela (CSF)

Da la vuelta a la nube y deja caer una tela con rigidez ajustable; los puntos que quedan a menos de un umbral de la tela son suelo (Zhang et al., 2016). La caída por paso y la corrección de los muelles siguen las constantes publicadas por los autores, y los muelles unen cada partícula con sus ocho vecinos (los cuatro contiguos y los cuatro diagonales), lo que mantiene la tela tendida sobre un tejado ancho en lugar de dejarla hundirse en su centro. Contrastado con la implementación de lidR sobre las mismas cinco escenas sintéticas, coincide con ella en el 99 al 100 % de los puntos.

###### Morfológico progresivo (PMF)

Aplica aperturas del mínimo con ventanas cada vez mayores, y el punto que sube más que un umbral que crece con la ventana no es suelo (Zhang et al., 2003).

###### Morfológico simple (SMRF)

Trabaja sobre la rejilla del punto más bajo de cada celda. Unas aperturas con ventanas que crecen de una en una celda marcan como objeto lo que cae más de lo que permite la pendiente máxima (*pendiente*, 0,15); lo que no es objeto, rellenado donde falta, es una superficie provisional, y un punto es suelo si está a menos de un *umbral* (0,5 m) más un *factor* (1,25) por la pendiente de esa superficie (Pingel, Clarke y McBride, 2013). Detecta además los puntos bajos aislados. La *ventana máxima* (radio, 18 m) debe superar al edificio más grande. En la línea de comandos se invoca con `--ground smrf`.

###### Axelsson (TIN progresivo)

Parte de una red triangular de semillas, que son el punto más bajo de cada celda grande (mayor que el edificio más grande, 20 m por defecto). Después añade los puntos que están cerca del plano de su triángulo (*distancia de iteración*, 1,4 m) y cuyas líneas a los tres vértices forman con el plano un ángulo pequeño (*ángulo de iteración*, 8°); se rehace la red y se repite hasta que no entra ningún punto. Los puntos a menos de la *tolerancia llana* (10 cm) del plano son suelo sea cual sea el ángulo, porque cerca de un vértice el ángulo sale grande incluso para un punto que está sobre la superficie (Axelsson, 2000). Los puntos que quedan fuera del casco de las semillas no los examina ningún triángulo, y las semillas, interiores a sus celdas, dejan sin cubrir una franja a lo largo del borde de los datos; por eso se añaden como semillas el punto más bajo de cada tramo de media celda en una banda junto al borde y cuatro vértices virtuales en las esquinas.

###### Prueba de filtrado de la ISPRS

Se han repetido los seis métodos con las 15 muestras de la prueba de filtrado del grupo de trabajo III/3 de la ISPRS (Sithole y Vosselman, 2004), que tienen referencia manual del suelo: ocho sitios urbanos y de bosque con 0,2 a 0,7 puntos/m², con pendientes fuertes y discontinuidades, un caso mucho más duro que el PNOA actual. Con los parámetros por defecto, promedio de las 15 muestras (el error total es 100 menos la exactitud global):

| Método | Error total | Suelo perdido (tipo I) | No suelo tomado por suelo (tipo II) | Kappa |
|---|---|---|---|---|
| SMRF | 5,3 % | 5,1 % | 8,3 % | 0,82 |
| PMF | 5,8 % | 4,0 % | 12,0 % | 0,81 |
| Kraus y Pfeifer | 11,2 % | 11,0 % | 10,0 % | 0,69 |
| MCC | 14,8 % | 21,4 % | 4,4 % | 0,63 |
| Axelsson | 14,8 % | 19,4 % | 6,2 % | 0,63 |
| CSF | 15,8 % | 17,2 % | 7,2 % | 0,62 |

Los dos morfológicos (SMRF y PMF) se ajustan bien incluso con tan pocos puntos. Los demás dejan sin clasificar mucho suelo en las laderas empinadas, porque sus parámetros por defecto están pensados para nubes densas y de poco ruido: Axelsson, por ejemplo, tolera 10 cm de ruido del sensor, y con 50 cm la exactitud de sus resultados sube del 85 al 89 %. Ajustar las escalas a la separación de los puntos con una sola regla (celdas iguales a la separación, de 1,5 o 2,5 m, y rigidez 2 en CSF) no mejoró ninguno de los seis, y con 2,5 m empeoraron todos. Quitar antes el ruido bajo con ELM mejora poco (hasta 1,4 puntos de exactitud, en Axelsson y CSF). Las muestras son de 2001 y pequeñas (7 000 a 52 000 puntos), por lo que el resultado dice poco sobre el comportamiento en una hoja entera y mucho sobre la robustez ante el relieve.

La elección depende más del terreno que del método. Con relieve fuerte o zona urbana y pocos puntos, SMRF o PMF (ISPRS); en bosque denso con pocos puntos de suelo, Kraus y Pfeifer o MCC, donde los morfológicos pueden tomar vegetación baja por suelo.

###### Prueba con OpenGF

OpenGF (Qin et al., 2023) es un conjunto de nubes ALS abiertas de varios países con el suelo etiquetado con detalle. Se han usado dos de sus tres hojas de prueba (T1 y T3), de 5 a 8,5 puntos/m², la densidad del PNOA, en una ventana de 1 000 × 1 000 m en el centro de cada una (7,6 y 6,8 millones de puntos; el suelo de la referencia es el 35 % y el 29 %). Con los parámetros por defecto, sin búfer ni limpieza previa del ruido, promedio de las dos hojas:

| Método | Exactitud | Suelo perdido (tipo I) | No suelo tomado por suelo (tipo II) | Kappa | Error del MDT (RMSE) | Tiempo por ventana |
|---|---|---|---|---|---|---|
| PMF | 96,8 % | 1,7 % | 3,9 % | 0,93 | 0,59 m | 0,8 s |
| MCC | 96,3 % | 9,6 % | 1,1 % | 0,91 | 0,52 m | de 21 a 26 s |
| Kraus y Pfeifer | 96,2 % | 8,4 % | 1,8 % | 0,91 | 0,71 m | 0,3 s |
| SMRF | 94,7 % | 1,1 % | 7,2 % | 0,88 | 1,58 m | 0,2 s |
| Axelsson | 92,9 % | 7,5 % | 7,1 % | 0,84 | 2,63 m | de 18 a 25 s |
| CSF | 92,3 % | 0,7 % | 10,9 % | 0,83 | 1,74 m | de 22 a 30 s |

Con esta densidad PMF, MCC y Kraus y Pfeifer se parecen mucho a la referencia (más del 96 %). SMRF y CSF pierden muy poco suelo, pero toman por suelo vegetación y estructuras bajas (el 7 % y el 11 % del no suelo), y por eso el MDT sale 1 o 2 m más alto en la hoja con más relieve (T3). Axelsson queda por detrás, a un coste de decenas de segundos. Es solo una referencia de dos hojas: la referencia es una clasificación hecha con detalle, no una medida de campo, y la tercera hoja de prueba (T2) no se ha usado: es una zona urbana con una plataforma plana de unos 200 × 200 m a 13 m sobre el suelo, que la referencia no da por suelo y que ningún filtro con una ventana de hasta 80 m distingue del terreno (la precisión del suelo no pasa del 76 % ni quitando antes el ruido bajo). La prueba se repite con `cargo run --release -p hipso-core --example ground_bench -- hoja.laz --size 1000`.

###### Límites y comprobaciones

Los seis métodos se han probado además sobre escenas sintéticas de suelo conocido (llano, ladera de 25°, colinas, ciudad con tejados de 15 × 12 m a 7 m de altura y bosque con copas de 10 a 14 m), con tres comprobaciones que un método no debe fallar. Mover la nube a coordenadas UTM no cambia la clase de ningún punto en ninguno. Barajar el orden de los puntos tampoco, salvo en MCC, que cambia hasta el 0,9 % de los puntos en el bosque, porque su ajuste de splines toma el primer punto como origen. Y en el interior de la nube el suelo conocido se clasifica sin pérdidas, salvo MCC (3 a 7 % en ladera fuerte y bosque), que es un límite del propio método.

En el borde de los datos faltan los vecinos de un lado, y los métodos de Kraus y Pfeifer, PMF y SMRF lo notan en las laderas: en la ladera de 25°, dentro de la franja de 10 m del borde pierden el 17 %, el 11 % y el 7 % del suelo (en el interior, el 0,1 % o menos). CSF y MCC no muestran esa diferencia, y Axelsson solo la muestra en el bosque (1,4 % en la franja frente a 0,02 % en el interior). Por eso se trabaja por teselas con un búfer de 30 m y, en un bloque de hojas, con `--neighbors`: solo el contorno exterior de todo el bloque queda sin vecinos.

Una construcción mayor que lo que el método sabe retirar se toma por suelo: en la escena urbana, Kraus y Pfeifer toma el 9 % de los puntos de los tejados y MCC el 12 %; PMF, SMRF, CSF y Axelsson, ninguno. Los métodos con semillas o ventanas (Axelsson, PMF, SMRF) necesitan que la celda o la ventana máxima supere la construcción más grande.

##### Edificios (clase 6)

Primero se estima el terreno con una apertura morfológica mayor que cualquier edificio, y se buscan los puntos que lo superan en más de la altura mínima (2,5 m por defecto). Entre ellos, un punto es semilla de tejado cuando sus vecinos dentro de un radio (1,5 m) están sobre un plano (planaridad de 0,6 o más y rugosidad de 7 cm o menos) y su eco no atravesó nada: un tejado detiene el pulso, y una copa lo deja pasar, de modo que los primeros ecos de pulsos con varios retornos no cuentan como tejado.

Las celdas de 1 m con al menos tres semillas son el núcleo de un tejado. Los núcleos se unen a través de la cumbrera, que no es plana y parte en dos un tejado a dos aguas, y forman regiones. Una región es edificio si es lo bastante grande (40 m² por defecto, contando los aleros) y si al menos el 90 % de sus lados bajan bruscamente al terreno de alrededor: un tejado acaba en una pared, y una ladera, no. Al edificio se le suman las celdas sólidas de su alrededor, dos celdas hacia fuera, que son los aleros y las paredes.

Los puntos del edificio hasta 1,5 m por encima de su tejado son clase 6; lo que queda más arriba (una copa sobre el tejado) y el resto de lo que supera la altura mínima son vegetación alta (5). Un tejado que el filtro de suelo había tomado por suelo se encuentra igual y se reclasifica; para no confundirlo con una ladera, a esos puntos se les exige además que la región tenga algún lado que baje bruscamente al terreno.

Funciona con edificios de tejado plano o a dos aguas, y con tejados con algo de arbolado encima; los tejados muy irregulares o más pequeños que la superficie mínima quedan como vegetación. Necesita densidad: pide al menos tres semillas por celda de 1 m, y con una nube de 0,7 puntos/m² no encuentra ningún edificio.

###### Prueba de edificios con DALES

DALES (Varney et al., 2020) es un conjunto de nubes ALS urbanas y rurales etiquetadas a mano por técnicos, y su versión 2, con las etiquetas revisadas, está publicada con licencia MIT. Se han usado seis de sus hojas de prueba de 500 × 500 m (de la 5080_54470 a la 5175_54395), con unos 45 puntos/m², y la misma nube reducida a una de cada cinco puntos (unos 9 puntos/m², la densidad del PNOA). Los puntos se clasifican sin tener en cuenta la clase que traen (ruido de puntos aislados, suelo SMRF, edificios, cables y postes con sus valores por defecto) y se compara punto a punto, en el interior de 400 × 400 m de cada hoja, con las clases 12 a 14 de la referencia (los tres tipos de edificio), con el suelo (clase 0) y con los cables y los postes. Las clases de vehículos y las vallas de la referencia se cuentan como «otros».

| Clase | 45 puntos/m², precisión / exhaustividad | 9 puntos/m², precisión / exhaustividad |
|---|---|---|
| Edificios | 95 % / 91 % | 96 % / 87 % |
| Suelo | 96 % / 100 % | 95 % / 100 % |
| Cables | 66 % / 41 % | 56 % / 33 % |
| Postes | 68 % / 27 % | 48 % / 33 % |

Los edificios se encuentran bien con las dos densidades (la exhaustividad baja cuatro puntos al reducir los puntos). El suelo no deja casi nada sin marcar, pero toma por suelo parte del resto (un 4 o 5 % de los puntos marcados no lo son). Los cables se encuentran mal y los postes, regular. Con 45 puntos/m² se recoge el 41 % de los puntos de cable con un 66 % de precisión, y con 9 puntos/m² el 33 % con un 56 % (con el radio de entorno fijado en 3 m en lugar de ajustarse a varias escalas se llegó al 63 % con un 85 %, pero en las hojas del PNOA ese radio fijo da peor resultado, de modo que se prefirió la búsqueda a varias escalas). Los postes de DALES (farolas, postes de servicio y algunas torres, entre árboles y edificios) se recogen en algo más de una cuarta parte de sus puntos, con una precisión del 68 % con 45 puntos/m² y del 48 % con 9. Sobre las hojas NPC03 con cables (dos hojas completas), el paso de cables da una precisión del 86 % y una exhaustividad del 60 %, en las que el proveedor ya los clasifica (con el radio de una sola escala era del 71 % y el 29 %). Esos pasos no deben darse por buenos sin revisarlos: sirven para proponer una clasificación que se corrige a mano. La prueba se repite con `cargo run --release -p hipso-core --example pnoa_bench -- --dales --recipe receta.txt hoja.laz`, con `--thin 5` para reducir los puntos.

Cables y postes se probaron además en dos líneas eléctricas reales sin etiquetas a mano. En QULTIVA (un dron con 99 puntos/m², clasificado con otro programa) la torre y los cables de una línea de alta tensión estaban marcados como edificio: de los 4 013 puntos de la primera torre, 3 952 salen como poste, y la segunda torre (814 puntos, entre cultivos bajos) sale como poste entera. Los cables salen con el 86 % de los puntos que el otro programa había marcado como edificio sobre la traza de la línea, y todos los puntos de cable quedan a menos de 1,3 m de ella; esa referencia no es una verdad limpia, porque es la clasificación errónea de un programa. En las tiras de Powerlines (290 puntos/m², cuatro pasadas) salen de cinco a seis conductores con su catenaria y un poste en cada apoyo, comprobado a ojo en la vista de perfil, y con 471 puntos/m² (RAILDATA, una vía con catenaria, 42,9 millones de puntos) salen 384 337 puntos de cable y 105 estructuras de poste, de entre 290 y 4 300 puntos, con una altura mediana de 5,9 m y una separación mediana de 12 m entre vecinas, que encaja con los mástiles de una catenaria; sin etiquetas a mano no se sabe cuántos son falsos ni cuántos faltan, y con esa densidad los postes de mayor tamaño hay que revisarlos. Con las hojas del PNOA de la zona de QULTIVA (NPC02, 12 puntos/m², de las que se tienen los nueve apeos de la línea) la receta de ejemplo con el paso de postes marca torre en ocho de los nueve apeos con 150 a 1 480 puntos (el centro de los puntos queda de 1,4 a 4,3 m del apeo; en el noveno solo hay 7 puntos); fuera de los apeos marca otras estructuras de 20 puntos o más, de las que no se sabe cuántas son torres de otras líneas. El proveedor marca los nueve apeos, con unos 130 a 440 puntos cada uno. Los cables suman 5 177 puntos frente a 8 228 del proveedor. El corredor sobre la clasificación de HIPSO reconoce cinco de los nueve apeos y marca dos postes más, y sobre la del proveedor reconoce ocho: un poste solo cuenta si hay un conductor a menos de 8 m, y los cables de la clasificación de HIPSO no llegan a todos los apoyos.

##### Vegetación por alturas

Se mide la altura del punto sobre el suelo de su entorno: baja (3) por debajo del primer límite, media (4) hasta el segundo y alta (5) por encima. Es una clasificación geométrica y no botánica, de modo que la vegetación arbustiva alta cuenta como vegetación media o alta.

##### Cables (clase 14) y postes o torres (clase 15)

Un punto es cable si está a más de una altura mínima sobre el suelo (5 m) y tiene vecinos alineados (linealidad = (λ₁ − λ₂)/λ₁ de la matriz de covarianza del entorno, mínimo 0,7), con poca anchura a lo ancho de esa línea y de dirección casi horizontal. Los puntos de un conductor llegan en racimos con huecos entre ellos (las líneas de barrido del vuelo), de modo que el radio del entorno se toma de la densidad de la nube: seis veces la separación de los puntos, entre 0,6 y 3 m (1 m con 45 puntos/m² y unos 2 m con 9). Con 0,25 m entre puntos o más (menos de unos 16 puntos/m²), donde un conductor tiene un punto cada metro o dos y las copas de al lado lo hacen una línea con unos radios y con otros no, un punto cuenta como candidato si parece de un cable con alguno de los radios (una vez, una vez y media y dos veces ese, hasta 4,5 m). Las cadenas de puntos de cable se aceptan si miden al menos 10 m y son rectas (su largo es diez veces su ancho), porque las líneas de barrido hacen que un trozo de copa parezca recto durante uno o dos metros. Los postes y torres son estructuras verticales estrechas (poco anchas para su altura, de al menos 6 m), sin puntos alrededor que las unan a las copas y que arrancan del suelo; además se buscan postes finos (farolas y postes de servicio) como líneas verticales de al menos 3 m. Con pocas pasadas sobre una línea alta, el vuelo solo ve el conductor donde lo cruzan las líneas de barrido: trozos de uno o dos metros con metros sin ningún punto entre ellos, que no llegan a 10 m seguidos. Esos trozos (a más de 10 m sobre el suelo, finos, planos y sin más puntos a su alrededor) se unen cuando tres o más van en fila, a menos de 15 m uno del siguiente y sin girar más de 8° en el del medio, y la fila mide al menos 20 m. Una estructura se mide por su núcleo: la anchura y el recuento de puntos no incluyen lo que le cuelga (los conductores que llegan a una torre, una rama que toca un poste), que solo ensancharía la caja. En una torre de transmisión hay de 30 a 40 puntos por metro de altura con 9 puntos/m² y miles con 100 puntos/m², así que en las estructuras de 15 m o más el límite de puntos es de 40 por metro de altura y crece con la densidad, y las filas de puntos de la celosía, separadas hasta 8 m en vertical, se unen en una sola estructura; los cultivos bajos al pie no cuentan como copa (la copa que descalifica es la que sube a más de un tercio de la altura de la torre). Con más de unos 180 puntos/m² los troncos se ven como líneas verticales, y un poste fino con una copa a su alrededor, que sube hasta su remate, se descarta.

Con la densidad del PNOA suele volver solo la cabeza del apoyo (una cruceta de unos metros, a más de 8 m, sin nada debajo). Una cadena corta, recta y aislada a esa altura se toma por cabeza de apoyo y se clasifica como 15 (la longitud máxima es ajustable, y 0 la desactiva). El método se ha contrastado con tres apoyos marcados sobre ortofoto en una hoja PNOA: en uno volvió la cabeza y se clasifica, y en los otros dos no volvió ni un solo punto, de manera que no hay nada que clasificar. Es útil para limpiar el CHM de líneas eléctricas, que de otro modo salen como árboles estrechos.

##### Carreteras (clase 11)

Es suelo liso, llano y oscuro para el láser: el asfalto devuelve menos intensidad que la tierra, y una calzada es un plano. Un punto de suelo es carretera si su intensidad no pasa de un máximo, que por defecto es el 60 % de la intensidad mediana del suelo de la tesela (así vale con cualquier escala: 16 bits o de 0 a 255) y que también puede darse como valor absoluto, su entorno de 1,5 m tiene una rugosidad pequeña (hasta 3 cm, la desviación de los puntos respecto del plano local) y una pendiente local de hasta 5°. Se activa con *Carreteras* en la ventana de clasificación (apartado *Carreteras y agua*) o con `--roads`.

Solo mira la forma y la intensidad del suelo: no sabe si los puntos forman una red, de modo que también marca como carretera cualquier otro suelo liso y oscuro (campos llanos y aparcamientos). El límite relativo se ajusta al sensor y al vuelo, pero en una tesela sin calzadas marca como carretera el suelo liso más oscuro que ese 60 %; con la intensidad de 0 a 255 de un dron, el límite absoluto de 10 000 que se usaba antes marcaba el 27 % de una ventana de cultivos como carretera, y el relativo, el 1,1 %. No se ha medido con una referencia hecha a mano; solo se ha comparado con la clase 11 de hojas NPC03, donde coincide en algo más de la mitad de los puntos (precisión del 56 % y exhaustividad del 53 %), y esa clase la hace el propio proveedor.

##### Agua (clase 9)

El agua es una lámina llana que devuelve casi nada al láser. Se toman los puntos de suelo de un solo retorno y de intensidad baja (por defecto hasta el 37,5 % de la intensidad mediana del suelo de la tesela; también puede darse un valor absoluto) cuyo entorno es llano (pendiente local de hasta 3°), se unen las celdas de 2 m que los contienen en regiones, y una región es agua si ocupa al menos 30 m² y la desviación típica de las cotas de sus puntos no pasa de 20 cm. Se activa con *Agua* en la ventana de clasificación (apartado *Carreteras y agua*) o con `--water`.

Es una herramienta sin validar y de partida. Contra la clase 9 de las hojas NPC03, que son charcas pequeñas y con pocos puntos, coincide muy poco (precisión del 5 % y exhaustividad del 28 %): encuentra algo de agua, pero marca también como agua otro suelo llano y oscuro, y por eso no está en la receta de ejemplo. Conviene revisar el resultado antes de usarlo, y probar con el máximo de intensidad y la superficie mínima del lago que se quiere recoger.

##### Reglas

Una regla cambia la clase de los puntos que cumplen unas condiciones: «a los puntos que cumplan esto, ponles la clase X». Se aplican después de todos los pasos automáticos (ruido, suelo, edificios, vegetación, cables y postes), así que sirven para retocar su resultado o para clasificar lo que esos pasos no tocan. Una regla no sustituye al suelo ni a los edificios: los completa.

###### Cómo funciona una regla

- Una regla actúa solo sobre los puntos que cumplen todas las condiciones que se han marcado. Una condición sin marcar no se mira.
- Cada regla tiene una clase de destino, que es obligatoria. Las clases de partida son opcionales: si no se indica ninguna, la regla mira todas menos el ruido (7 y 18).
- Las reglas se aplican en el orden en que están. Una regla ve lo que hizo la anterior, de modo que el orden importa.
- Si una regla quita la clase 2 a un punto, ese punto deja de ser suelo también para el MDT.

###### Condiciones de una regla

| Condición | Qué mira | Para qué sirve |
|---|---|---|
| Clases de partida | La clase que tiene ahora el punto | Retocar solo una clase (por ejemplo, solo los puntos sin clasificar) |
| Eco | Si el punto es único, primero, intermedio o último de su pulso | Los primeros ecos de pulsos con varios retornos son copa y bordes; los últimos llegaron al suelo |
| Ángulo de barrido | El ángulo del pulso, en valor absoluto | Los puntos con ángulo grande son menos exactos |
| Altura sobre el suelo | La distancia vertical al suelo, medida sobre la media de los puntos de suelo en celdas de 1 m | Hacer estratos de vegetación; un punto sin suelo conocido a su alrededor no cumple esta condición |
| Intensidad | La intensidad del retorno | Separar materiales o superficies por su reflectancia |
| Forma del entorno | Cómo se reparten los puntos vecinos dentro de un radio (1 m por defecto): pendiente, planaridad, linealidad, esfericidad, rugosidad y número de puntos | Separar tejados y paredes (superficies planas), cables (líneas) y copas (volúmenes) |

La forma del entorno se calcula con los valores propios de la covarianza de los vecinos (Demantké et al., 2011; Weinmann et al., 2015). La pendiente es el ángulo del plano local respecto de la horizontal (0° es llano y 90° una pared). La planaridad, la linealidad y la esfericidad van de 0 a 1: valen cerca de 1 en una superficie lisa, en una línea y en un volumen, respectivamente. La rugosidad es la desviación típica de la distancia al plano local, en metros.

###### Ejemplo: estratos de vegetación por altura

Tres reglas, en este orden, reparten en tres clases los puntos aún sin clasificar:

| Regla | Clases de partida | Altura sobre el suelo | Clase de destino |
|---|---|---|---|
| 1 | 1 | hasta 0,5 m | 3 (vegetación baja) |
| 2 | 1 | de 0,5 a 2 m | 4 (vegetación media) |
| 3 | 1 | desde 2 m | 5 (vegetación alta) |

El resultado es exactamente el del paso de vegetación por alturas. Otros usos habituales: pasar a una clase aparte (la 12, por ejemplo) los puntos de ángulo mayor de 25°, que conviene dejar fuera del suelo y de las superficies; separar estructuras por su forma, como las superficies planas y empinadas a más de 2 m del suelo (pendiente de 60° o más y planaridad de 0,7 o más); y distinguir el ruido alto del bajo, como hace el ASPRS (7 es ruido bajo y 18 ruido alto): `--rule "from=7;height=5..;to=18"` pasa a 18 el ruido que está a más de 5 m sobre el suelo. Con reglas también se pueden intentar clases como agua, carreteras o puentes (por ejemplo, suelo plano y de baja intensidad para el agua), pero no se han contrastado con una clasificación de referencia y no hay un ajuste recomendado.

###### Dónde se definen las reglas

- **En la ventana de clasificación**, la pestaña *Reglas* tiene una fila por regla. Se marca la casilla de la regla; se eligen las clases de partida, el eco y las condiciones que se quieran (cada una con su casilla y su intervalo), y la clase de destino. *Añadir regla* y *Quitar la última* cambian el número de reglas.
- **En la línea de comandos**, cada regla es un `--rule` con partes separadas por `;`: `to=` (clase de destino), `from=1,2` (o `from=~7`), `returns=SFIL` o números del 1 al 9, `angle=25..90`, `height=0.5..2`, `intensity=0..100`, `slope=60..`, `planarity=0.8..`, `linearity`, `sphericity`, `roughness`, `neighbours` y `radius=1`. Un intervalo `a..b` incluye los dos extremos, `a..` es «a o más» y `..b` es «hasta b». Por ejemplo: `--rule "from=1;height=0.5..2;to=4"`.

Las características de forma se calculan solo para los puntos que pasan las demás condiciones de la regla, y no cuestan nada si ninguna regla las pide. En una hoja PNOA de 8 millones de puntos, las reglas por altura añaden 0,04 s (el 3 %) y una regla de forma sobre los puntos a más de 2 m del suelo, 0,7 s.

##### Recetas

La ventana de clasificación ya funciona como una receta: se marcan los pasos que se quieren (solape, ruido, suelo, edificios, vegetación, cables y postes, carreteras y agua, reglas) con sus parámetros, y se ejecutan uno detrás de otro, siempre en ese orden. Una receta es esa misma configuración escrita en un archivo de texto, para guardarla y aplicarla otra vez, a otra hoja o a un bloque entero, con el mismo criterio, y para dejar escrito cómo se clasificó.

###### Guardar y cargar una receta

En lo alto de la ventana de clasificación hay dos botones:

- *Guardar receta…* escribe en un archivo de texto la clasificación que hay marcada en la ventana.
- *Cargar receta…* pone en la ventana la clasificación de un archivo de receta, sustituyendo lo que hubiera marcado. Si el archivo trae opciones que la clasificación no usa (por ejemplo, `--results-dir`), la ventana las ignora y lo dice.

Debajo de los botones se lee lo que ha pasado. Solo se guardan los parámetros que tienen opción en la línea de comandos; si se ha cambiado uno avanzado que no la tiene (las alturas de los estratos de vegetación, por ejemplo), la ventana avisa de que se guardará con su valor por defecto.

###### Receta de ejemplo: clasificar como el PNOA

El PNOA entrega sus hojas LiDAR con la clase de cada punto, y su producto más fino es el NPC03. Esta receta recibe una nube sin clasificar y devuelve una clasificación similar a la de ese producto. Está en `docs/recetas/clasificacion_pnoa.txt`, se carga con *Cargar receta…* en la ventana de clasificación, o se aplica con `hipso classify hoja.laz --recipe clasificacion_pnoa.txt`:

```
# Clasificación similar a la del PNOA producto NPC03, el más fino
# (hojas NPC03 del PNOA): solape entre pasadas, ruido, suelo, edificios, cables, carreteras
# y estratos de vegetación por altura sobre el suelo
--overlap
--overlap-cell 5
--noise isolated
--ground smrf
--smrf-threshold 0.2
--smrf-scalar 0.5
--buildings
--building-area 25
--building-offset 2
--wires
# carreteras: suelo liso, llano y de intensidad baja (asfalto)
--roads
# lo que queda al ras del suelo es suelo
--rule "from=1;height=..0.12;to=2"
# estratos: baja de 0,12 a 0,5 m, media de 0,5 a 3 m, alta desde 3 m
--rule "from=1,5;height=0.12..0.5;to=3"
--rule "from=1,3,5;height=0.5..3;to=4"
--rule "from=1,3,4;height=3..;to=5"
```

Qué busca. Reproduce el aspecto de la clasificación del NPC03: solape entre pasadas como clase 12, suelo generoso (SMRF con umbral 0,2 m y escala 0,5), edificios con un margen de 2 m sobre el suelo, cables, vegetación en tres estratos por altura (baja hasta 0,5 m, media hasta 3 m y alta desde 3 m) y carreteras como suelo liso, llano y de intensidad baja (el asfalto devuelve menos intensidad que la tierra; el límite de intensidad es relativo al suelo de cada tesela, y puede darse un valor absoluto).

Qué no es. Los parámetros se eligieron para que el resultado sea similar al de las hojas NPC03, pero esa clasificación es la del proveedor y no una verdad del terreno, de modo que la receta no garantiza un acierto medido. No clasifica torres de transmisión (el paso de postes existe, pero la receta no lo activa), tableros de puente ni agua, ni el ruido bajo; las carreteras salen como suelo liso sin saber si forman una red, y se mezclan con campos llanos oscuros y aparcamientos. La receta se probó también con otros vuelos (una tira de cuatro pasadas con 290 puntos/m² y un dron con intensidad de 0 a 255): el solape aparta el 1,3 % de los puntos de la tira y las carreteras el 1,1 % de la ventana de cultivos, porque el solape ignora las celdas con más de dos pasadas y el límite de intensidad de las carreteras es relativo al suelo de la tesela. Para medir cuánto acierta un método de suelo con una referencia más fiable, véanse las pruebas de la ISPRS y de OpenGF.

###### Receta de ejemplo: una línea eléctrica en un vuelo denso

Para un vuelo de dron o de avión a baja altura sobre una línea eléctrica (de 100 a 500 puntos/m²), `docs/recetas/clasificacion_lineas_electricas.txt` activa el ruido, el suelo SMRF, los edificios, los cables, los postes y torres y los estratos de vegetación. No lleva solape ni carreteras, que en esos vuelos no hacen falta.

```
# Clasificación de una línea eléctrica en un vuelo denso (dron o avión a baja altura, de 100 a 500 puntos/m²):
# ruido, suelo, edificios, cables, postes y torres, y estratos de vegetación por altura sobre el suelo
--noise isolated
--ground smrf
--smrf-threshold 0.2
--smrf-scalar 0.5
--buildings
--building-area 25
--building-offset 2
--wires
--poles
# lo que queda al ras del suelo es suelo
--rule "from=1;height=..0.12;to=2"
# estratos: baja de 0,12 a 0,5 m, media de 0,5 a 3 m, alta desde 3 m
--rule "from=1,5;height=0.12..0.5;to=3"
--rule "from=1,3,5;height=0.5..3;to=4"
--rule "from=1,3,4;height=3..;to=5"
```

Se probó en un vuelo de dron de unos 100 puntos/m² y en tiras de 290 y 471 puntos/m² (resultados en *Cables y postes*), sin etiquetas hechas a mano. El suelo SMRF y los márgenes son los de la receta del PNOA, y no se han ajustado a estos vuelos. Después, *Corredores eléctricos* mide la vegetación que amenaza la línea sobre la nube clasificada con esta receta.

###### Reglas y recetas no son lo mismo

Una regla es un solo cambio de clase («los puntos de la clase 1 a más de 2 m del suelo pasan a la 5»). Una receta es una clasificación entera guardada en un archivo, y puede contener reglas, además del método de suelo y de los demás pasos.

###### Cómo es una receta

Cada línea del archivo lleva una o más opciones de `hipso classify`, las mismas que se escribirían a mano. Las líneas vacías y las que empiezan por `#` (comentarios) se ignoran, y un valor con espacios va entre comillas dobles. El archivo se puede escribir a mano o guardar desde la ventana:

```
# bosque: suelo con SMRF y estratos por altura
--ground smrf
--rule "from=1;height=..0.5;to=3"
--rule "from=1;height=0.5..2;to=4"
--rule "from=1;height=2;to=5"
```

Lo que el archivo no diga toma el valor por defecto de la línea de comandos (por ejemplo, el suelo es Kraus y Pfeifer si no hay `--ground`).

###### Aplicar una receta desde la línea de comandos

Se pasa con `--recipe`:

```
hipso classify hoja1.laz --recipe bosque.txt
hipso classify hoja2.laz --recipe bosque.txt
```

Las opciones del archivo ocupan el lugar de `--recipe`, de modo que dar la receta equivale a escribir esas opciones en la línea de comandos. Por eso una opción del archivo no se puede repetir fuera de él (salvo `--rule`, que se puede repetir), y con varias hojas o con `--neighbors` el resultado es el mismo que si se hubieran dado las opciones a mano.

##### Teselas y rendimiento

Se ajustan el tamaño de tesela, el búfer (la franja alrededor de cada tesela, que permite decidir en los bordes con los vecinos a la vista) y los hilos.

#### Comparar nubes (cambios)

Escribe los puntos de la nube cargada que están a más de una distancia de todos los de otra nube de referencia, y da la distancia media, el RMS y la compleción. Los puntos de cambio aislados se descartan. Sirve para detectar cortas, cambios de obra o crecimiento entre dos vuelos. Al ser una comparación punto a punto, con densidades muy distintas aparecen falsos cambios, y por ese motivo conviene adelgazar antes a la misma densidad.

#### Ajustar nubes

Mueve una nube para que encaje con la de referencia mediante el punto más cercano iterativo (ICP; Besl y McKay, 1992), solo desplazando o desplazando y girando, y muestra el desplazamiento y el giro. Resuelve vuelos del mismo sitio con un desfase sistemático. Necesita que ambas nubes se solapen y que tengan estructuras fijas (edificios, terreno), y sobre un bosque que ha cambiado puede ajustar mal. Dos opciones mejoran esos casos. *Punto a plano* (`--plane`) minimiza la distancia al plano tangente del punto de referencia (Chen y Medioni, 1992) en lugar de la distancia al punto: converge en muchas menos iteraciones sobre superficies (terreno, tejados) y no se queda enganchada a la malla de la nube de referencia, aunque en una superficie completamente plana deja sin fijar el deslizamiento a lo largo de ella. *Pares que se usan* (`--trim`, en %) se queda con los pares más cercanos y deja fuera los demás (Chetverikov et al., 2002), de modo que un tramo cambiado (un desprendimiento, una corta, un edificio nuevo) no arrastra el ajuste; con el 100 % se usan todos.

#### Entre pasadas

Produce un ráster con la diferencia de altura entre pasadas de vuelo (ID de fuente) y el porcentaje de celdas por encima de 8 y 16 cm. Es un control de calidad del vuelo que debe hacerse antes de modelar, puesto que las pasadas desajustadas generan escalones en el MDT y en el CHM.

Cuando el ID de fuente no distingue las pasadas (todos los puntos llevan el mismo valor), la casilla *Las pasadas salen de los huecos en el tiempo GPS, mayores de* (`--gps-gap` en la línea de comandos) las encuentra por los silencios del tiempo GPS, con el método de *Líneas de vuelo*. La casilla *Cada nube cargada es una pasada* (`--by-cloud`) toma cada archivo como una pasada.

#### Líneas de vuelo

Encuentra las líneas de vuelo de una nube aérea a partir de los huecos del tiempo GPS. El avión vuela una línea, gira durante decenas de segundos sin registrar y vuela la siguiente, de modo que, al ordenar los tiempos de todos los puntos y cortar donde el hueco supera un límite (30 s por defecto), queda un tramo de tiempo por cada línea. Con varias nubes, el tiempo cuenta para todas juntas.

La herramienta sirve cuando el ID de fuente no separa las pasadas, y también para comprobar cuántas hay. Se ajustan el hueco que separa dos líneas y los puntos mínimos de una línea (1 000 por defecto): un tramo con menos puntos no es una línea, y el informe dice cuántos puntos han quedado fuera por ese motivo. En la línea de comandos, `--min-duration` añade una duración mínima.

La nube necesita tiempo GPS (formatos de punto 1 y 3 en adelante), y los puntos con tiempo 0 no pertenecen a ninguna línea. Si el tiempo es el de la semana (no ajustado) y el vuelo cruza el domingo a medianoche, el tiempo salta hacia atrás. El informe lo avisa y no lo corrige, de modo que las líneas posteriores pueden salir mal cortadas.

El resultado es una tabla CSV (`…_lineas_de_vuelo.csv`) con, por línea, `Linea`, `T_inicio`, `T_fin`, `Duracion_s`, `Puntos`, la caja (`X_min`, `Y_min`, `X_max`, `Y_max`) y `Angulo_medio` de barrido, y una capa de polígonos con el contorno de cada línea (`…_lineas_de_vuelo.shp`, con los campos `linea`, `t_inicio`, `t_fin`, `duracion`, `puntos` y `angulo`). Si se pide, se escriben además copias de las nubes (`…_lineas.laz`) con el número de línea en el ID de fuente (hasta 65 535) o en el dato de usuario (hasta 255).

La nube se lee dos veces, una para encontrar los tramos y otra para saber a cuál pertenece cada punto, y la memoria sigue al tiempo registrado (un número por décima de segundo) y no al número de puntos. En la línea de comandos: `hipso flightlines nube.laz --gap 30 --assign source-id`.

En resumen, el tiempo GPS basta para recuperar las pasadas cuando el ID de fuente no las distingue.

#### Densidad de Voronoi

Mide la densidad de una nube para controlar su calidad sin depender de por dónde caiga el borde de la celda. La celda de Voronoi de un punto es la zona que está más cerca de él que de ningún otro, y la densidad de una celda del ráster no es el número de puntos dividido entre su área, sino `n / Σ área` de las celdas de Voronoi de sus `n` puntos. Mover la rejilla no cambia, por tanto, el resultado.

Las celdas de Voronoi mayores que un límite (100 m² por defecto) son huecos de datos (agua, un tejado sin retorno) y no cuentan para la densidad. Su área se da aparte.

Se ajustan la celda del ráster (5 m por defecto), el umbral en puntos/m² (8), el hueco de datos, los retornos (por defecto F, los primeros y únicos, que son los pulsos) y las clases (vacío: todas menos el ruido y los puntos retenidos). Se guardan el ráster de densidad (`…_vdensidad.tif`), el de cumple / no cumple (`…_vdensidad_cumple.tif`, con 1 donde se alcanza el umbral) y una tabla CSV con `Celdas`, `Puntos`, `Media`, `Mediana`, `P5`, `P95`, `Pct_cumple`, `Umbral` y `Area_sin_datos_m2`.

La nube se corta en teselas de 250 m con un margen de 20 m, para que las celdas de los puntos del borde de una tesela salgan bien, y la memoria sigue a la tesela y a los rásteres, no a la nube. En la línea de comandos: `hipso voronoi-density nube.laz --cell 5 --threshold 8`.

El porcentaje de celdas que cumple dice, con un solo número, qué parte de la nube alcanza la densidad exigida, y el ráster de cumple / no cumple enseña dónde falla.

#### Colorear con imagen (RGB)

A cada punto el color del píxel de una imagen del mismo sitio, normalmente una ortofoto (PNOA, por ejemplo), y el infrarrojo cercano si la nube es de formato 8 y la imagen tiene esa banda. Se eligen las bandas de rojo, verde y azul (1, 2 y 3 por defecto). Las nubes sin color (formatos 0, 1 y 6) se guardan en formato 7 (LAS 1.4: se conserva todo y la clase pasa a ocupar un byte entero), y las de forma de onda (4 y 9) no se convierten.

La imagen se lee una vez por nube y solo en su extensión; si esa ventana no cabe en memoria, se usa una vista reducida de la propia imagen y el informe lo dice. Los puntos que la imagen no cubre quedan en negro, o con su color si se marca la casilla. Escribe `…_rgb`. Sirve para ver la nube con su aspecto real, para distinguir copas de suelo o de tejados por el color, o para calcular métricas espectrales por árbol. La imagen debe estar en el sistema de la nube, y una ortofoto de otra fecha, o con las copas inclinadas por la perspectiva, da a algunos puntos el color de lo que tienen al lado. Una hoja PNOA de 13 millones de puntos se colorea en 1,5 s. En la línea de comandos: `hipso colorize nube.laz --image orto.tif`.

#### Normalizar intensidad

Parte de un hecho físico: la energía que vuelve al escáner cae con el cuadrado de la distancia, y la misma superficie sale por ello más oscura en el borde de una pasada o desde una pasada más alta. Cada intensidad se multiplica por (R / R_ref)^a (Höfle y Pfeifer, 2007), con *a* = 2 para superficies extensas (suelo, copas) y en torno a 3 para cables y bordes; R_ref es por defecto la distancia media de la nube, para que los valores conserven su tamaño habitual.

La distancia R de cada punto sale de la trayectoria del vuelo (un archivo de texto con líneas 'tiempo x y z', en el tiempo GPS y el sistema de la nube, que es lo más exacto) o, en su defecto, de la altitud de vuelo constante y el ángulo de escaneo: R = (altitud − z) / cos(ángulo). Los puntos sin distancia calculable (fuera de la trayectoria) conservan su intensidad, y el informe dice cuántos son. Escribe `…_intensidad`. Conviene aplicarla antes de usar la intensidad en clasificaciones, métricas o comparaciones entre pasadas y vuelos. No corrige el ángulo de incidencia sobre la superficie, ni la atmósfera, ni las diferencias entre sensores. En la línea de comandos: `hipso intensity nube.laz --trajectory tray.txt` o `--altitude 3200`.

#### Imagen de intensidad

Hace, por cada nube cargada, una imagen en tonos de gris: en cada celda, la intensidad media de sus puntos, guardada como GeoTIFF en el sistema de la nube. Con una celda de 0,5 a 1 m se parece a una ortofoto en blanco y negro y sirve de fondo para digitalizar o de base para la ortofoto; para el color de los puntos está *Colorear con imagen*.

Se ajusta la celda (1 m por defecto; el botón *Celda automática* pone 3 veces la separación media de los puntos, y entre 2 y 4 veces deja pocas celdas vacías y conserva el detalle), los retornos (F primeros y únicos, L últimos, A todos, o los números de retorno; los primeros dan la imagen más nítida) y las clases (una lista como 2,5 o, con «~», todas menos esas; con las clases vacías se dejan fuera el ruido y los puntos retenidos). Las celdas sin puntos se rellenan con la media de sus vecinas hasta el número de celdas indicado en *Rellenar huecos de hasta* (2 por defecto); los huecos mayores, como el agua o una falta de pasada, quedan vacíos y transparentes, no a 0. La imagen se recorta a las celdas que tienen puntos y no a la caja de la cabecera, que en pasadas oblicuas o con una cabecera mal escrita contiene mucho espacio vacío.

La intensidad se corrige antes por el ángulo de escaneo, sin pedir nada: la energía que vuelve al escáner cae con el cuadrado de la distancia, y sobre terreno llano esa distancia crece como 1 / cos(ángulo), así que cada intensidad se multiplica por 1 / cos² del ángulo (hasta 45°) y la altura de vuelo no hace falta. Es una corrección sencilla que quita el oscurecimiento de los bordes de pasada; no tiene en cuenta el relieve, el ángulo de incidencia ni la atmósfera. Con la trayectoria o la altura de vuelo, *Normalizar intensidad* es más exacta y se puede aplicar antes. Después se estira del percentil 2 al 98 de todas las hojas juntas y se guarda como imagen de 8 bits (los tonos van de 1 a 255; el 0 es «sin dato»), con la misma escala en todas las hojas para que se unan sin saltos de tono. El informe da el rango de intensidades que corresponde a esa escala. Como la imagen es de 8 bits, ya no sirve para análisis de intensidad: para eso, usa la nube.

Escribe `…_intensidad_1m.tif`, y con la casilla *Unir las hojas en un mosaico* un `…_intensidad_1m_mosaico.tif` con todas las hojas; donde se solapan se toma la media de las hojas. Los rásteres de hoja también se pueden unir después con *Mosaico*, en la pestaña *Raster*. Se hace en una sola pasada y la memoria sigue al tamaño de la imagen, no al de la nube. En la línea de comandos, `hipso raster nube.laz --stat mean-intensity --cell 1` da la intensidad media cruda, en 32 bits y sin corregir ni recortar.

#### Exportar puntos

Saca los puntos a otro formato. Por defecto escribe una nube LAZ sin normalizar, y también se puede elegir LAS, texto (ASCII) con las columnas que se quieran, o shapefile 3D (PointZ). Se puede filtrar por clases y tomar uno de cada *n* puntos. Si se marca *Normalizar*, la herramienta pide el MDT y exporta alturas sobre el suelo: a la cota de cada punto se le resta la del MDT de su propia nube en ese lugar. Con varias nubes, cada una usa su MDT (el último que se calculó para ella, salvo que se indique uno a mano). Los puntos que quedan fuera del MDT no se exportan, y el mensaje final dice cuántos fueron.

La normalización resulta útil para trabajar con otros programas que esperan alturas normalizadas, o para compartir un conjunto ya listo para métricas o segmentación. Es tan buena como el MDT: uno con errores deja puntos de suelo con alturas de unos decímetros. En la línea de comandos: `hipso export nube.laz --normalize -o altura.laz` (con `--dtm mdt.tif` si no es el último MDT de esa nube; `.las`, `.laz`, `.shp` o texto según la extensión).

##### Exportar a COPC

Elegir el formato COPC en *Exportar puntos* reescribe la nube como COPC (Cloud Optimized Point Cloud), un LAZ 1.4 normal cuyos bloques son los nodos de un octree. Cada nodo guarda un punto por celda de una malla de 128 × 128 × 128 sobre su cubo y pasa el resto a sus ocho hijos, hasta nodos de 100 000 puntos como mucho. Cualquier programa que lea LAZ lo lee entero, y los que conocen COPC (QGIS, visores web, este mismo programa) leen solo la zona y el detalle que necesitan: recortar 4 ha de una hoja de 13 millones de puntos lee menos de un millón.

Se conservan todos los atributos (comprobado punto a punto con lidR). Los formatos 0 a 3 pasan a 6 o 7, que son los que admite COPC, y el sistema de coordenadas se escribe como WKT, que COPC exige (si la nube no lo declara, el informe avisa). La nube se ordena en memoria: hasta 30 millones de puntos se sostiene entera (unos 70 bytes por punto) y, por encima, se lee dos veces, una para las posiciones y otra, por lotes de nodos, para los registros de cada bloque, con lo que la memoria baja a unos 35-50 bytes por punto (el fichero resultante es idéntico). Por encima de 100 millones de puntos se escribe por celdas (ver el apartado 4.2), sin límite práctico de tamaño. Una hoja PNOA de 13 millones de puntos tarda 1,3 s. Escribe `<nombre>.copc.laz`. En la línea de comandos: `hipso copc nube.laz`.

### 7.2 Superficies

Todas las superficies se escriben como GeoTIFF y se calculan hoja por hoja.

#### Contorno de los rásteres

El MDT, el MDS y el CHM se calculan sobre la caja que envuelve la nube, pero **no se rellenan las zonas donde no hay puntos**: los rásteres siguen el contorno real de la nube, de modo que una nube con forma de L, un vuelo girado o un hueco entre pasadas no se convierte en un rectángulo con valores inventados. Para conseguirlo se anota en una malla gruesa (la de unas 6 celdas, de 3 m como mínimo) dónde cae algún punto, se cierran los huecos pequeños (hasta unos 4 cuadros de esa malla, para que un claro o un lago no rompan el contorno) y lo que queda fuera se deja sin dato. Así los mosaicos de hojas contiguas encajan sin que una hoja pise a la vecina con extrapolaciones.

#### MDT (modelo digital del terreno)

Se calcula a partir de los puntos de suelo (la clase 2 de la nube, o el método que se elija al clasificar) y hay tres formas de dar valor a cada celda, con la triangulación por defecto.

La triangulación (TIN) es la opción de referencia y casi siempre la mejor. Por teselas, se queda con el punto de suelo más cercano al centro de cada celda, se triangula (Delaunay) y se interpola linealmente dentro de cada triángulo, lo que da una superficie continua que respeta los puntos. No se interpola a través de triángulos con aristas de más de 50 m, y esos huecos se rellenan al final con la media de las celdas vecinas. Su problema aparece cuando hay muy pocos puntos de suelo bajo copas densas: los triángulos son grandes y la superficie se alisa.

El ponderado por distancia (IDW) toma en cada celda la media de los 8 puntos de suelo más cercanos ponderada por 1/d² (Shepard, 1968). Antes de calcular, el suelo se reduce a un punto (la media) por cada media celda, así que el tiempo depende de la malla y no de la densidad. Da una superficie suave y muy rápida, con un alcance de 30 celdas (entre 10 y 50 m), y conviene en nubes muy densas o con ruido donde se quiere un suelo alisado. Suaviza las rupturas de pendiente (taludes, cunetas) y no pasa por los puntos.

La media por celda calcula la media de las elevaciones de los puntos de suelo de cada celda, en una sola pasada y sin cargar la nube. Es la más rápida y la que menos memoria usa, pero deja huecos donde no hay puntos (se rellenan al final), de modo que solo conviene para comprobaciones rápidas o nubes inmensas en un equipo justo de memoria.

Las líneas de rotura (con TIN o IDW) son un shapefile de líneas o polígonos que el MDT debe seguir en lugar de redondear: bordes de camino, taludes, cunetas, orillas. Cada línea se densifica a vértices separados media celda, que entran en la triangulación con los puntos de suelo, y los puntos de suelo pegados a la línea se apartan, así que los triángulos siguen la línea en vez de cruzarla. Las líneas con cotas (PolylineZ, PolygonZ) fijan la altura y son líneas duras, como las de un levantamiento topográfico. Las líneas sin cotas (un cauce o una cuneta dibujados sobre la ortofoto) toman en cada vértice el punto de suelo más bajo a menos de una celda, lo que hunde el cauce en el modelo; donde bajo las copas no hay suelo tan cerca, ese punto puede quedar algo más alto que la superficie que daba la triangulación sola. Contrastado en una hoja PNOA, una línea 3D deja el MDT exactamente a su altura y no cambia ninguna celda a más de 3 m de ella. En la línea de comandos: `hipso dtm nube.laz --breaklines lineas.shp`.

#### MDS (modelo digital de superficie)

Representa lo más alto que ve el sensor (copas, tejados) y se calcula con los puntos que no son ruido. Por defecto se usa la triangulación (TIN), que toma el punto más alto de cada celda, lo triangula y lo interpola, sin espigas ni huecos pequeños. El máximo por celda da la cota del punto más alto de cada celda en una sola pasada y rellena las celdas vacías (casilla *Rellenar*); es lo más rápido, pero deja 'pozos' en copas con huecos. El ponderado por distancia (IDW) trabaja sobre los puntos más altos de cada media celda.

#### CHM (modelo de alturas de la vegetación)

Es, por celda, la altura del punto más alto sobre el terreno, `z − MDT` (las alturas negativas valen 0). Es la entrada de la segmentación de individuos y de muchos inventarios, y usa el MDT de esa misma nube. Con pit-free se apilan varias triangulaciones de los puntos por encima de umbrales de altura (0, 2, 5, 10, 15 m…) y se toma el máximo de capas (Khosravipour et al., 2014). Su ventaja es que quita los 'pozos' que deja el máximo por celda en copas con huecos, y eso reduce las cimas falsas al segmentar; su problema, que es más lento y exige MDT.

El botón *MDS / CHM* reúne ambos con los mismos métodos (TIN, máximo por celda, IDW y, con MDT, pit-free). Si se marca *Restar el MDT* y todas las nubes tienen MDT, sale el CHM (`chm_…`), y si no, el MDS (`mds_…`). El pit-free exige MDT porque trabaja con alturas sobre el suelo.

#### Suavizado (MDT, MDS y CHM)

Es una ventana circular de media, mediana o gauss, con el tamaño en metros. La mediana conserva mejor los bordes de copa, el gauss da una superficie más suave y la media es la más simple. Suavizar el CHM antes de segmentar reduce las cimas falsas, a costa de fundir copas pequeñas vecinas.

Como recomendación, una celda de 1 m para el MDT en terreno con vegetación con 1 a 10 puntos/m² (PNOA), y de 0,5 m para el MDS y el CHM. Si hay pocos puntos de suelo bajo copas densas, una celda mayor da un terreno más fiable.

#### Análisis de terreno

En la pestaña *Raster*, grupo *Análisis del terreno*, cada producto tiene su botón y su ventana, con sus propios valores y su ayuda. Los botones se reparten en cinco menús: *Relieve* (pendiente, orientación y sombreado), *Forma del terreno* (curvaturas, rugosidad, TRI, TPI y *Crestas y drenaje*), *Hidrología* (profundidad de depresiones, acumulación de flujo, TWI, red de drenaje, subcuencas, *Zonas encharcadas* e *Inundación*), *Visibilidad y rutas* (cuenca visual y *Ruta de menor coste*) y *Volúmenes* (*Volumen sobre MDT*, *Cubicación de pilas* y *Desmonte y terraplén*).

Los productos derivados de un MDT (o de cualquier ráster de alturas) funcionan todos igual: se elige el MDT (un archivo o una capa ya cargada) y la salida, que es un GeoTIFF del mismo tamaño y con el mismo sistema de coordenadas. El ráster debe estar en un sistema métrico (UTM), y en grados el programa se niega a calcular. Las celdas sin dato siguen sin dato. Una celda del borde, o vecina de un hueco, no tiene la ventana de 3×3 completa, y por defecto queda sin dato: no se calcula con alturas que no existen, que es lo que hace `gdaldem` si no se le pide lo contrario. Dos opciones, explícitas, dan valor a esas celdas. *Repetir el borde* (`--edges repeat`) repite la fila y la columna extremas fuera del ráster, y deja sin dato las vecinas de un hueco. *Rellenar con la celda central* (`--edges center`) sustituye cualquier vecina que falte por la altura de la propia celda, con lo que aplana la superficie junto al borde y los huecos y subestima la pendiente allí. En el interior del ráster, lejos de bordes y huecos, las tres opciones dan exactamente el mismo resultado. Contrastado con `gdaldem slope` (GDAL 3.13) sobre un MDT de 501 × 501 celdas, la opción por defecto da las mismas celdas con valor y las mismas vacías que `gdaldem` sin opciones, con una diferencia máxima de 0,0014° (redondeo en coma flotante). Ninguna de las dos opciones de borde equivale a `-compute_edges` de `gdaldem`, que completa las celdas que faltan fuera del ráster extrapolando linealmente las alturas del borde (y trata de otro modo las cuatro esquinas); quien necesite exactamente eso debe usar `gdaldem`. Si las alturas no están en las unidades de la celda, el *factor Z* las corrige.

| Grupo | Producto | Qué es y cuándo sirve |
|---|---|---|
| Relieve | Pendiente | Inclinación en grados o porcentaje (Horn, 1981; ventana de 3×3). Base de casi todo lo demás: accesos, erosión, tipos de estación |
| | Orientación | Hacia dónde mira la ladera, en grados desde el norte; las celdas llanas quedan vacías. Solana y umbría |
| | Sombreado | Iluminación 0–255 con la dirección y la altura de la luz que se elijan (315° y 45° por defecto). Sirve para ver el MDT y detectar defectos |
| Forma | Curvatura en planta, de perfil y total | Zevenbergen y Thorne (1987), en 1/m; positiva en cumbres, lomas y espolones, negativa en vaguadas. La de planta indica si el agua converge o diverge; la de perfil, si acelera o frena |
| | Rugosidad | Diferencia entre la celda más alta y la más baja de la ventana de 3×3 |
| | Índice de rugosidad (TRI) | Media de las diferencias con las ocho vecinas (Wilson et al., 2007) |
| | Posición topográfica (TPI) | Altura menos la media del círculo de radio dado (Weiss, 2001): positivo en lomas, negativo en vaguadas. Se calcula con sumas acumuladas por fila, así que un radio grande apenas cuesta más. El resultado depende del radio: conviene probarlo a varias escalas |
| Hidrología | Profundidad de depresiones | Cuánto hay que rellenar cada depresión para que el agua salga del modelo (Priority-Flood, Barnes et al., 2014); sirve también para detectar defectos del MDT |
| | Acumulación de flujo | Superficie (m²) que vierte a cada celda, con direcciones D8 (O'Callaghan y Mark, 1984) sobre el modelo ya rellenado |
| | Índice de humedad (TWI) | `ln(a / tan β)` (Beven y Kirkby, 1979): alto donde el agua se acumula y la pendiente es baja |
| | Red de drenaje | Celdas por las que vierte al menos la superficie mínima indicada (en hectáreas) |
| | Subcuencas | Cuenca de cada tramo de la red (de una cabecera o confluencia a la siguiente confluencia), numeradas desde 1 |
| Visibilidad | Cuenca visual | Qué celdas ve un observador (X, Y del mapa, altura de los ojos, altura que debe verse y alcance): 1 las ve, 0 no, vacío fuera del alcance. No aplica la curvatura terrestre ni la refracción (despreciables a pocos kilómetros) |

Todos los productos heredan los defectos del MDT: uno con terrazas, escalones entre hojas o ruido produce curvaturas y pendientes falsas, y la red de drenaje sale entrecortada. D8 reparte todo el flujo de una celda a una sola vecina, lo que da redes limpias pero no representa bien la dispersión en laderas llanas. Además, la mayoría de estos índices dependen de la resolución, y por eso hay que comparar siempre con la misma celda.

Todo se calcula con el ráster completo en memoria (unos 20 bytes por celda las derivadas y 35 la hidrología; un MDT de 25 millones de celdas tarda de 2 a 7 s), por filas en paralelo. Por la línea de comandos: `hipso terrain mdt.tif --product slope`, y `--help` lista los productos y sus parámetros (entre ellos `--edges nodata|repeat|center`).

#### Zonas encharcadas

Delimita dónde se queda el agua en un MDT. El modelo se rellena hasta que todas las depresiones drenan (Priority-Flood; Barnes et al., 2014), una celda está bajo el agua cuando la superficie rellena queda por encima del terreno más que la profundidad mínima, y las celdas que comparten un lado forman un charco. El MDT debe estar en un sistema métrico.

De cada charco se miden el área (`area_m2`), la profundidad máxima (`prof_max`) y la media (`prof_med`), el volumen (`vol_m3`), la cota a la que desborda (`cota_desb`), la superficie que vierte a él (`cuenca_m2`) y la pendiente media del terreno bajo él, en grados (`pend_med`).

Se elige el MDT y el tipo de terreno, que fija los valores de partida. *Nivelado (cultivo)* pide una profundidad mínima de 5 cm y un área mínima de 1 m². *Irregular (monte)* pide 15 cm, 4 m² y una pendiente media bajo el charco de 8° como máximo, porque un MDT de monte tiene hoyos que no son charcos. *A medida* deja todo libre.

Además se pueden descartar las balsas y las zanjas con un área máxima y una profundidad máxima. *Suavizar el MDT antes* (gauss, en metros; 0 es no suavizar) alisa el ruido de un MDT fotogramétrico o de pocos retornos, pero en un campo con caballones no conviene, porque borra las calles. Si el MDT no declara el valor de 'sin datos' y trae ceros, los toma por tal solo cuando cubren buena parte del modelo y los demás valores están bien por encima de 0 (`--nodata` lo fija).

Los resultados son el ráster de charcos (`…_charcos.tif`, con el número de cada charco), la profundidad del agua (`…_charcos_profundidad.tif`), los polígonos (`…_charcos.shp`) y un GeoPackage (tabla `charcos`). En la línea de comandos: `hipso ponds mdt.tif --min-depth 0.05 --min-area 1`. Allí, `--method local` encuentra también los huecos que no son depresiones cerradas (el fondo de un surco bajo la cresta del modelo suavizado), con la pendiente y la superficie que vierte comprobadas celda a celda, y `--min-volume`, `--min-catchment` y `--max-slope` descartan charcos por volumen, por superficie que vierte y por pendiente.

En resumen, el resultado depende de los mínimos elegidos, y por eso conviene fijarlos con el tipo de terreno antes de medir.

#### Inundación

Simula el agua de uno o varios niveles sobre un MDT y da, por nivel, el área inundada, el volumen, la profundidad y el número de masas de agua. No inunda nivel a nivel: calcula una sola vez el nivel de agua más bajo con el que se alcanza cada celda desde el origen, con un relleno por prioridad que avanza por las celdas más bajas, y la celda toma el mayor entre el nivel que le llega y su propia cota. Una celda está inundada en un nivel cuando ese alcance no lo supera, y la profundidad es el nivel menos la cota. Una serie de niveles cuesta, por tanto, casi lo mismo que uno solo.

Con conectividad (lo normal) el agua solo llena lo que puede alcanzar desde el origen pasando por celdas que no superan el nivel, y un hoyo detrás de una loma queda seco. Con *Bañera: sin conectividad* se inunda todo lo que está por debajo del nivel, llegue el agua o no. Ese modelo enseña las depresiones del terreno, pero exagera una inundación real.

El origen del agua es uno o varios puntos (un río, una rotura, el mar; un `x y` por línea, con las coordenadas del mapa) o el borde del modelo (y las celdas vecinas a las vacías). Los niveles son uno solo o una serie `desde:hasta:paso` (hasta 1 000 niveles), medidos como cota, como altura sobre la celda más baja del modelo o como altura sobre el origen. El agua pasa por las ocho celdas vecinas, o por las cuatro que comparten un lado si se desmarca la casilla.

Se guardan el ráster de alcance (`…_inundacion_alcance.tif`, con el nivel mínimo del agua que llega a cada celda, útil como mapa de peligro), la profundidad de cada nivel, los polígonos de la extensión de cada nivel y la tabla `…_inundacion.csv` con `Nivel_m`, `Area_m2`, `Volumen_m3`, `Prof_media_m`, `Prof_max_m` y `Masas_de_agua`. En la línea de comandos: `hipso flood mdt.tif --seed 500000,4500000 --levels 100:110:1`.

#### Crestas y drenaje

Traza las líneas de cresta o de drenaje de un MDT como líneas 3D. La red de drenaje son las celdas que recogen al menos una superficie mínima (1 000 m² por defecto), seguidas aguas abajo con las direcciones de flujo D8 del análisis hidrológico. Las crestas son el drenaje de la superficie invertida: lo que un cauce es para el terreno, una cresta lo es para su imagen invertida.

La red se corta en tramos, de una cabecera o una confluencia a la siguiente, y cada tramo es una línea con las cotas del MDT y los campos `ID`, `ORDEN`, `LONG` (m) y `PEND_MED` (pendiente media, en %). El orden es el de Strahler: las cabeceras son 1 y dos tramos del mismo orden que se juntan dan el siguiente.

Se ajustan la superficie mínima que recoge, la longitud mínima de un tramo y el suavizado (pasadas de Chaikin; 0 sigue las celdas). En una cresta o un cauce perfectamente llano, con cotas iguales a lo largo de la línea, el flujo se reparte por el centro y la línea sale en dos mitades que se tocan allí. El resultado es `…_crestas.shp` o `…_drenaje.shp`. En la línea de comandos: `hipso ridges mdt.tif --drainage --min-area 1000`.

#### Ruta de menor coste

Busca el camino más barato sobre un MDT entre un origen y un destino, con paradas intermedias si se quiere. Se mueve por las ocho celdas vecinas, de modo que sigue algo la retícula y puede resultar hasta un 8 % más larga que la recta. El coste de pasar de una celda a la vecina es la distancia por el coste medio de las dos celdas, y la ruta es la de menor suma total (Dijkstra). No se cruzan las celdas vacías, las más pendientes que el límite ni las de dentro de una barrera.

El coste puede ser la pendiente, el tiempo de marcha a pie, nada o un ráster propio. Con la pendiente, el coste por metro es `1 + (pendiente / referencia) ^ potencia`, con la pendiente en %, potencia 2 y referencia 10 % por defecto, de modo que a la pendiente de referencia el coste por metro es el doble. El tiempo de marcha es el de la función de Tobler, que distingue subida y bajada y da segundos. Con *Nada* la ruta es la más corta, y con un ráster de costes por metro (con las celdas del MDT) el coste es el de ese ráster.

Se puede fijar una pendiente máxima que no se cruza, una capa de polígonos de barreras y una capa de zonas preferidas, donde el coste se multiplica por un factor (0,5 por defecto). Con varios destinos y *Varios destinos: ir al más barato*, la ruta va al que cuesta menos. Las coordenadas son las del mapa.

La ruta es una línea 3D (`…_ruta.shp`) con `ID`, `LONG_2D`, `LONG_3D` (sobre el terreno), `COSTE`, `PEND_MED`, `PEND_MAX`, `ASCENSO` y `DESCENSO`, y con Tobler el informe da el tiempo en horas y minutos. *Guardar también el ráster de coste acumulado desde el origen* escribe `…_ruta_coste.tif`. En la línea de comandos: `hipso leastcost mdt.tif --from 500000,4500000 --to 501000,4501000 --model tobler`.

#### Volumen sobre MDT

Calcula el volumen entre una superficie y el terreno, celda a celda: V = Σ (altura de la superficie − MDT) · celda². Las celdas por encima del terreno suman al volumen sobre él, y las que quedan por debajo, al volumen bajo él (vaciados). Cada nube es una hoja y busca sus rásteres en su carpeta de resultados (o se eligen a mano). La superficie puede ser de tres tipos. El CHM de cada nube ya contiene alturas sobre el suelo y no necesita MDT; da el volumen de la vegetación o de lo que sobresale del terreno. El MDS menos MDT toma la superficie y el terreno de la nube, y el MDT se muestrea en el centro de cada celda del MDS; las celdas que el MDT no cubre no se miden (se cuentan aparte). El MDS sobre una cota horizontal mide el volumen por encima de un plano a la elevación indicada (acopios, por ejemplo).

La altura mínima que cuenta descarta las celdas más cerca del terreno que ese valor, y quita así el ruido de la superficie. Las zonas son todo el ráster, las AOI dibujadas (un volumen por AOI) o los polígonos de un archivo vectorial; si se solapan, se miden una a una.

Para el volumen de copas segmentadas se eligen como zonas el shapefile de copas que da *Segmentar individuos* (`…_individuos.shp`) y como superficie el CHM (no el ráster de copas, que guarda los números de árbol y no alturas). La columna `Region` de la tabla lleva el `TREEID` de cada copa, el mismo número que la columna `id` de la tabla de árboles y que el campo `TREEID` del shapefile de copas, y por él se une el volumen de cada árbol a su altura, su área de copa y su posición.

El resultado es un único `…_volumen.csv` (con una columna `Hoja` si hay varias nubes) con, por zona, `Area_m2`, `Vol_sobre_m3`, `Vol_bajo_m3`, `Vol_neto_m3`, `H_media_m` y `H_max_m` (de las celdas sobre el terreno) y `Celdas_sin_MDT`. La precisión es la del ráster, y cuando es posible conviene usar la misma celda para el MDS y el MDT. El volumen de una copa por este método es el volumen envolvente bajo la superficie del CHM, no el de madera.


#### Cubicación de pilas

Mide el material que hay sobre una superficie base dentro del contorno de cada pila: volumen, área en planta y sobre la pendiente, altura media y máxima y, con la densidad, la masa. Hacen falta un modelo de superficie (MDS) en un sistema métrico y una capa de polígonos con el contorno de cada pila.

El contorno debe rodear el pie de la pila, porque lo que valga la superficie a lo largo de él se toma como suelo. Un contorno dibujado por dentro de la pila sube la base y resta volumen.

La base se construye para cada pila de una de seis formas: un plano ajustado al contorno por mínimos cuadrados (el valor por defecto), la cota más baja del contorno, la cota mediana del contorno, una superficie interpolada desde el contorno, una cota fija, o un MDT del terreno de partida, el que había antes de la pila. *Altura mínima* descarta las celdas más cerca de la base que ese valor, y con *Densidad del material* (t/m³) la capa y la tabla llevan también la masa.

El volumen se decide por el centro de cada celda. Su error es del orden del perímetro por media celda por la altura del borde, y baja con celdas más finas.

Se guardan una capa de polígonos con el contorno de cada pila (`…_pilas.shp`), con los campos `area_2d`, `area_3d` (sobre la pendiente), `base_z`, `vol_sobre`, `vol_bajo`, `h_media`, `h_max`, `perimetro` y `masa_t`, y la tabla `…_pilas.csv` (`Id`, `Nombre`, `Area_2D_m2`, `Area_3D_m2`, `Cota_base_m`, `Volumen_m3`, `Vol_bajo_m3`, `H_media_m`, `H_max_m`, `Perimetro_m`, `X`, `Y` y `Masa_t`). Si se marca, se escriben también la superficie base y la altura sobre la base como rásteres. En la línea de comandos: `hipso piles mds.tif --piles pilas.shp --base plane --density 1.6`.

En resumen, el volumen de una pila depende de la base y del contorno, y el contorno debe rodear su pie.

#### Desmonte y terraplén

Calcula la tierra que hay que quitar (desmonte) y la que hay que añadir (terraplén) para llevar un terreno existente a un diseño. La diferencia es `diseño − terreno`, celda a celda: positiva es terraplén (falta tierra) y negativa es desmonte (sobra). El volumen de una celda es el valor absoluto de la diferencia por el área de la celda, y las diferencias de hasta la tolerancia (5 cm por defecto) cuentan como sin cambio.

El terreno existente es un ráster (un MDT). El diseño puede ser otro ráster (un MDT de diseño), un plano horizontal a una cota o un plano inclinado, que se define con la cota en un punto de origen, la pendiente en % y la orientación hacia la que baja (grados desde el norte, en sentido horario, como correría el agua).

Con una capa de polígonos opcional se mide además cada polígono por separado, y el total del ráster entero se da siempre. El resultado es el ráster de la diferencia (`…_cutfill.tif`), una tabla (`…_cutfill.csv`) con una fila `Total` y una por polígono (`Desmonte_m3`, `Terraplen_m3`, `Neto_m3`, `Area_desmonte_m2`, `Area_terraplen_m2` y `Area_sin_cambio_m2`) y, si se pide, un ráster de clases (`…_cutfill_clases.tif`, con 1 para desmonte, 2 para terraplén y 0 para sin cambio). En la línea de comandos: `hipso cutfill existente.tif --design level:152.5 --regions parcelas.shp`.

El volumen neto indica si la obra necesita aportar tierra (positivo) o retirarla (negativo), y las áreas dicen dónde.

### 7.3 Inventario de la vegetación

El inventario de la vegetación se divide según el tipo de nube, porque lo que se puede medir depende de cómo se haya captado. Con una nube aérea (ALS) se trabaja a escala de unidad de gestión y de individuo; con una nube de escáner terrestre (TLS), tronco a tronco. De ahí que la pestaña *LiDAR* tenga dos grupos separados: ALS, con los botones *Extraer muestras*, *Métricas por celda* y *Modelos lineales* (los tres de escala de unidad de gestión) y *Segmentar individuos* y *Análisis por objetos* (de escala de individuo), y *Cambio de la cubierta vegetal* en el grupo *Superficies*; y TLS, con el botón *Medición de troncos*. Las herramientas ALS convierten la nube en variables que un gestor del territorio reconoce (alturas, coberturas, volúmenes, árboles) y se ordenan de lo más agregado a lo más detallado: primero las métricas por celda, que sirven para modelar a escala de unidad de gestión, y después las que detectan y siguen árboles individuales.

#### Métricas por celda

La herramienta cuadricula la nube en celdas (15 m por defecto; la rejilla está alineada a múltiplos del tamaño de celda, de modo que las celdas de hojas vecinas encajan). En cada celda calcula estadísticos de las alturas sobre el suelo de todos los puntos que no son ruido, con `altura = z − MDT` (el MDT de esa misma nube). Las celdas con menos puntos que el mínimo (4) no dan métricas. Se añade la fecha de captura (`Date` en las tablas de texto, `captura` en las capas).

Estas métricas son las variables explicativas del enfoque por área (*area-based approach*, Næsset, 2002): percentiles de altura, cobertura de la vegetación y momentos, con los que se modelan volumen, área basimétrica, biomasa o altura dominante a partir de unidades de muestreo de campo (White et al., 2013). Su ventaja es la robustez, porque no dependen de detectar árboles, y la rapidez, que permite cubrir territorios enormes. Son las mismas métricas que da FUSION (McGaughey) y coinciden con sus columnas, y los momentos L siguen a Hosking (1990). Sus límites son tres: dependen de la densidad de puntos y del tamaño de la celda (que debe ser el de la unidad de muestreo de campo), de la fecha del vuelo y de un buen MDT, y en celdas con agua, edificios o pocos puntos no son fiables.

Las opciones son el *límite de altura* de cobertura (2 m), la *intensidad* (los mismos estadísticos aplicados a la intensidad), los *estratos* (estadísticos por franjas de altura: por defecto hasta 0,15, 1,37, 5, 10, 20 y 30 m) y la *memoria* máxima, porque se procesa por bandas de filas que caben en ella y la nube se lee una vez por banda.

##### Qué es cada métrica

En la tabla, `h` son las alturas de los puntos de la celda y `n` su número.

| Métrica | Cómo se calcula |
|---|---|
| Total return count | n, número de puntos de la celda |
| Elev minimum, maximum, mean | Mínimo, máximo y media de h |
| Elev mode | Moda de h, estimada con un histograma |
| Elev stddev, variance, CV | Desviación típica, varianza y coeficiente de variación (desviación / media) |
| Elev IQ | Rango intercuartílico: P75 − P25 |
| Elev skewness, kurtosis | Asimetría y curtosis (momentos de tercer y cuarto orden, como en FUSION: sobre la desviación típica muestral; la curtosis no resta 3). En celdas casi sin vegetación las alturas se concentran a ras de suelo y estas dos métricas dependen mucho de unos pocos puntos y de cómo se normalizó: un par de centímetros de diferencia en el MDT las cambia mucho. Conviene usarlas con cautela en modelos de zonas abiertas |
| Elev AAD | Desviación media absoluta respecto a la media |
| Elev MAD median, MAD mode | Mediana de las desviaciones absolutas respecto a la mediana, y respecto a la moda |
| Elev L1 a L4 | Momentos L (L1 = media; L2 mide la dispersión; L3 y L4, la forma) |
| Elev L CV, L skewness, L kurtosis | L2 / L1, L3 / L2 y L4 / L2 |
| Elev P01 … P99 | Percentiles 1, 5, 10, 20, 25, 30, 40, 50, 60, 70, 75, 80, 90, 95 y 99 de h (interpolación lineal entre valores ordenados) |
| Return 1 … 9 count, Other return count | Puntos de cada número de retorno (los de número 10 o superior, en «otros»). «Primer retorno» es el retorno número 1 |
| Percentage first returns above *hb* | 100 · primeros retornos con h > *hb* / total de primeros retornos. Es la cobertura de la vegetación con el límite *hb* |
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

La metodología es la de los estadísticos de altura habituales en inventario con LiDAR (percentiles, cobertura, momentos), que son las variables que se usan en los modelos de volumen, biomasa o altura dominante.

##### Nombres de los campos del shapefile

El formato dBASE de un shapefile limita los nombres a 10 caracteres, y por eso se abrevian. El CSV conserva los nombres largos, y junto al shapefile se escribe `<nombre>_campos.txt` con la correspondencia exacta de esa ejecución. Las abreviaturas son las de la tabla siguiente, con límite de cobertura de 2 m (el número cambia con el límite elegido).

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

Con *intensidad*, los campos de `TRN` a `ECmMn` salen con `I` en lugar de `E` (por ejemplo `IMn`, `IP95`, `ISd`), y la columna del CSV empieza por `Int` en lugar de `Elev`.

Las columnas de estratos se llaman `ES<n><estadístico>` (altura, *Elev*) e `IS<n><estadístico>` (intensidad), donde `<n>` es el número del estrato (0 es el primero, el de menor altura) y el estadístico va abreviado (`TRN` recuento de puntos, `Rpro` proporción sobre el total de la celda, `Min`, `Max`, `Mn` media, `Mo` moda, `Med` mediana, `Sd` desviación, `Cv` coeficiente de variación, `Skw` asimetría, `Kur` curtosis; por ejemplo `ES0Mn` es la media de las alturas del primer estrato). El `_campos.txt` dice qué franja es cada número.

##### Métricas de estructura

Con la casilla *Métricas de estructura* (o `--structure`) se escribe, junto a las demás, un archivo `…_estructura` con cinco magnitudes por celda. La fracción de huecos es la proporción de retornos por debajo de una altura (1 m por defecto). El índice de área foliar (LAI) vale −ln(fracción de huecos) / k, con k = 0,5 por defecto (ley de Beer–Lambert; Monsi y Saeki, 1953). La diversidad de alturas del follaje (FHD) es la entropía de Shannon de la proporción de retornos en cada capa de 1 m (MacArthur y MacArthur, 1961). El índice de complejidad vertical (VCI) es esa entropía dividida por su máximo, el ln del número de capas (van Ewijk et al., 2011). La altura de la base de la copa (CBH) es el borde inferior de la primera capa, por encima de 2 m, que tiene al menos el 10 % de los retornos de la capa más llena.

Se calculan con las mismas alturas que ya se guardan para las demás métricas, de manera que no hay pasada extra y, sin la casilla, no cuestan nada. Los límites son claros. El LAI sale de la proporción de retornos y no de una medida directa de hojas, así que es un índice relativo, útil para comparar pero no un valor absoluto. Con pocos retornos por celda las métricas de perfil son ruidosas, y conviene usar celdas de 15 a 20 m o más. La CBH depende del suelo de 2 m elegido y del 10 % (ambos cambian en la ventana) y no sustituye una medida de campo. La densidad aparente de copa (CBD) y la rugosidad de superficie (*rumple*) no están, porque necesitan coeficientes del usuario o un modelo de superficie aparte. Las filas de las métricas se calculan en paralelo por filas de celdas, con resultado idéntico al anterior (comprobado byte a byte) y menos tiempo con varios núcleos.

#### Segmentar individuos

Detecta árboles individuales sobre el CHM: sus cimas, su copa y su altura. Con varias nubes cargadas segmenta el CHM de cada una y da un resultado por hoja (si se elige un CHM a mano, solo ese). Un árbol en el borde entre dos hojas sale en las dos, cada vez cortado, y la hoja que solo tiene su flanco inventa una cima en el borde; con *Cada árbol una sola vez* (`--around` en la línea de comandos) cada hoja busca los árboles sobre su CHM ampliado con lo que tienen las vecinas en un margen (30 m por defecto) y se queda solo con los árboles cuya cima cae en su extensión (el borde oeste y el sur le pertenecen, el este y el norte son de la hoja siguiente), de modo que ninguno se cuenta dos veces ni se corta. Los números `id` son de cada hoja, y un punto de la nube que cae en una copa que es de la vecina queda con `treeID` 0. Las hojas deben tener la misma celda. El proceso tiene cinco pasos. El primero es un suavizado opcional del CHM (media, mediana o gauss). El segundo localiza las cimas, que son máximos locales en una ventana (fija, o variable según la altura, `a + b · h + c · h²`; circular o cuadrada) por encima de una altura mínima (Popescu y Wynne, 2004). El tercero delimita las copas con uno de tres métodos. El cuarto escribe las salidas: tabla de árboles (`id`, posición, altura, área y diámetro de copa), puntos de las cimas, polígonos de copa, ráster de identificadores y, si se pide, el `treeID` de cada punto de la nube. También se puede pedir un GeoPackage con los ápices y las copas en un solo archivo (tablas `apices` y `copas`) y las métricas de la nube dentro de cada copa. Estas son las de *Extraer muestras*, calculadas con los puntos que caen dentro de cada polígono de copa (con alturas normalizadas, lo que hace el proceso más lento), y se añaden como campos de los polígonos de copa. El quinto, opcional, es la evaluación con inventario de campo: empareja cada árbol medido con la cima más cercana (dentro de una distancia dada) y da aciertos, omisiones, comisiones, exhaustividad, precisión, la medida F y el error de altura (Eysn et al., 2015, plantean este tipo de contraste).

| Método | Cómo funciona | Cuándo es más útil | Ventajas | Problemas |
|---|---|---|---|---|
| Dalponte y Coomes | Crece una región desde cada cima añadiendo celdas vecinas que no sean demasiado más bajas que la cima ni que la media de la copa (umbrales *semilla* y *copa*) y hasta un alcance máximo | Masas con copas bien diferenciadas (coníferas, plantaciones, encinar abierto) | Copas de forma natural y compactas; se controla bien con el alcance máximo | Dos parámetros de umbral que dependen de la especie; en copas muy irregulares deja celdas sin asignar |
| Silva | Cada celda pasa a la cima más cercana si está dentro de un radio proporcional a la altura de la cima y por encima de una fracción de ella | Dehesas y masas abiertas, o cuando se quieren copas casi circulares | Muy simple y estable; sin crecimiento iterativo | Copas más geométricas que reales; recorta copas grandes o alargadas |
| Cuencas (watershed) | Inunda el CHM invertido desde las cimas y traza la línea de reparto entre copas vecinas (Vincent y Soille, 1991; aplicado a copas por Chen et al., 2006) | Copas que se tocan en masas cerradas, donde la línea de unión importa | Reparte bien el espacio entre copas vecinas | Sensible al ruido del CHM: sin suavizar produce muchas copas pequeñas (sobresegmentación) |

Dalponte y Silva se han contrastado con el paquete lidR 4.3 (Roussel et al., 2020), usado como caja negra: las cimas coinciden exactamente, y las copas coinciden en el 97 % (Dalponte) y el 99 % (Silva) de las celdas, con diferencias solo en píxeles disputados entre dos árboles. No se usó código de lidR. Las referencias de los métodos son Dalponte y Coomes (2016) y Silva et al. (2016).

Los buenos resultados exigen un CHM limpio (mejor *pit-free* que el máximo por celda), una celda de 0,5 m y una ventana de cimas ajustada al tamaño de las copas. Con inventario de campo se puede calibrar la ventana: en una zona de prueba, una ventana variable `3 + 0,1·h` acertó el 86 % de los árboles con una precisión del 76 %.

Conviene conocer los límites, y todos se deducen de que la segmentación trabaja sobre el CHM y no sobre los puntos. Solo ve lo que asoma por arriba, de modo que los árboles dominados bajo otros no aparecen, y en masas cerradas de frondosas varias copas se funden en una. Las copas con varias cimas se parten en dos si la ventana es pequeña, y dos árboles pequeños se funden en uno si es grande, por lo que no hay una ventana buena para todo y se prueba en una unidad de muestreo con árboles contados. La vegetación arbustiva alta se detecta como árboles bajos, y se corrige subiendo la altura mínima (`hmin`) o filtrando después por altura y por área de copa. El CHM debe proceder de un MDT correcto, ya que un error de suelo de decímetros desplaza todas las alturas. Y las alturas son las del CHM (máximo de la celda), algo menores, por tanto, que la altura real del árbol, sobre todo con celdas gruesas.

##### Enlace con los volúmenes

Los árboles, las cimas y las copas comparten el número `id` (campo `TREEID` en los shapefiles). Para el volumen de cada copa se usa el shapefile de copas como zonas de *Volumen sobre MDT* (apartado 7.2), y la tabla de volúmenes lleva ese mismo `TREEID`, por el que se une a la de árboles. Un árbol sin polígono de copa (por tener menos de tres celdas) no aparece en el shapefile de copas, pero sí en la tabla de árboles.

#### OBIA: análisis por objetos

Agrupa las celdas de varias capas en objetos (unidades de gestión, copas de árboles concretos, tejados, manchas de vegetación arbustiva), los describe con atributos y, si se desea, les asigna una clase. Es el análisis de imagen orientado a objetos (OBIA) aplicado a productos LiDAR: en lugar de clasificar cada celda por separado, se clasifican objetos homogéneos, con su tamaño, su forma y las estadísticas de cada capa. El proceso se compone de seis fases.

Las capas de entrada son un CHM (obligatorio, el primero) y, opcionalmente, más rásteres que coincidan con él celda a celda (mismo origen, tamaño de celda y dimensiones; si no coinciden, la herramienta lo dice y se igualan antes con *Ráster → Igualar rásteres*). De la nube cargada pueden salir además, en una sola lectura, la intensidad media, los retornos por pulso (alto bajo la copa, 1 en tejados y suelo desnudo), la proporción de suelo y la densidad de puntos. La rugosidad del CHM (desviación típica en una ventana) distingue una copa, cuya altura varía de celda a celda, de un tejado o un prado.

Los objetos se obtienen con dos métodos. La fusión de regiones (Baatz y Schäpe, 2000) parte de manchas de celdas casi iguales y fusiona, ronda a ronda, cada par de vecinas que son la mejor opción una de la otra, mientras el coste de fusionarlas (cuánto crece la heterogeneidad de valores y, si se pide, de forma) sea menor que la *escala* al cuadrado. Las capas se estandarizan, así que la escala no depende de las unidades. A mayor escala, objetos mayores (conviene probar entre 5 y 15). El peso de la forma (0 a 0,9) y la compacidad hacen, con valores altos, los objetos más redondeados y menos fieles al valor. Cada capa puede tener su peso (0 la deja fuera), y el resultado no depende del número de hilos. El método de rango de valores toma las celdas conexas cuyo CHM está entre dos valores (por ejemplo, todo lo que supera 2 m forma masas de vegetación arbórea). El objeto mínimo (en celdas) se fusiona con su vecino más similar (fusión) o se descarta (rango).

Los atributos de cada objeto son `area`, `perimetro`, `compacidad` (4π·área/perímetro²: un cuadrado vale 0,79), `alargamiento` (0 redondo, cerca de 1 una línea), el centroide `x`/`y` y, para cada capa, su `_media`, `_desv`, `_min` y `_max`.

La clasificación admite tres vías. Las reglas se escriben en un archivo de texto, una por línea (`vegetacion_arborea: chm_media >= 5 and area < 400`); vale la primera regla que cumple un objeto, y un objeto con un valor vacío no cumple condiciones sobre él. Agrupar por similitud no necesita reglas ni muestras: se indica cuántos estratos se quieren y los objetos se agrupan con k-means (Lloyd, 1982; arranque de Arthur y Vassilvitskii, 2007) a partir de la media de cada capa, el percentil 95 de la altura y la cobertura de la vegetación, estandarizados y con más peso para los objetos grandes; el estrato 1 es el más bajo, y con una semilla fija el resultado se repite. Las muestras son un shapefile de polígonos con un campo que nombra la clase (se crea con *Vectorial → Nueva capa* de polígonos), y un bosque aleatorio (Breiman, 2001; 100 árboles por defecto) aprende de los objetos que las muestras cubren (al menos la mitad de sus celdas). La exactitud se mide con los objetos que cada árbol no vio durante el entrenamiento, y se da la exactitud global, el índice kappa (Congalton, 1991), la precisión del usuario y del productor de cada clase y los atributos que más pesan. También aquí, con semilla fija, el resultado se repite.

Los estratos homogéneos son opcionales y necesitan una clasificación. Los objetos vecinos de la misma clase se unen en un solo polígono, y las áreas menores que el *estrato mínimo* toman la clase del vecino con el que más lindan (las que no tienen vecino grande se quedan como están). Se obtiene el polígono de cada estrato, un ráster de estratos y una tabla resumen con, por clase, el número de áreas, la superficie en hectáreas, la media y la desviación del CHM y el coeficiente de variación (cuanto menor, más homogéneo). Si se indica un número de unidades de muestreo, se reparten entre los estratos por asignación de Neyman (1934): más unidades de muestreo a los estratos grandes y variables, con un mínimo de dos por estrato cuando alcanzan. La cobertura (`cobertura`, en %) es la proporción de celdas del objeto cuyo CHM llega a la altura indicada (2 m por defecto), y también se calculan `<capa>_p50` y `<capa>_p95` del CHM.

Las salidas son el ráster de números de objeto (`…_objetos.tif`, entero de 32 bits), los polígonos con sus atributos y, si hay clase, la clase y la confianza (la proporción de árboles que votan por ella), el ráster de clases (`…_clases.tif`) y la tabla CSV. Los polígonos son el contorno exacto de las celdas, con huecos y sin superposiciones, y el área del polígono es la del objeto. *Simplificar* aligera los contornos con Douglas-Peucker, a costa de pequeños huecos entre vecinos. La casilla *GeoPackage* guarda además los polígonos de los objetos y de los estratos en un solo archivo (`….gpkg`, con las tablas `objetos` y `estratos`).

##### Rendimiento

Todas las fases son lineales en el número de celdas. En un equipo de pruebas, una hoja PNOA de 1 km² (CHM de 1 m, 11 millones de puntos, cuatro capas, clasificada por reglas) se procesó en menos de un segundo, y la segmentación de 20 millones de celdas sintéticas con ruido, en unos 2 s. La memoria crece con las celdas y con el número de manchas iniciales (un CHM ruidoso da más), y en rejillas de cientos de millones de celdas conviene empezar con una zona.

Los límites que conviene conocer son seis. La escala no se puede dar 'a ojo', porque depende del tamaño de celda y de la variabilidad de las capas; se prueba en una zona pequeña y se comprueba que el límite de los objetos sigue lo que se ve. Un CHM ruidoso (máximo por celda con pocos puntos) da muchas manchas pequeñas y objetos con bordes irregulares, y se corrige suavizando o usando el CHM *pit-free*. El coeficiente de variación no sirve con medias cercanas a cero (un estrato sin vegetación arbórea sale con un valor enorme), y en esos estratos hay que mirar la desviación; además, la asignación de Neyman usa la variabilidad del CHM, y si la variable que se va a estimar es otra (volumen, área basimétrica), conviene repartir con la variabilidad de esa variable en una prueba piloto (Cochran, 1977). La segmentación se hace con toda la rejilla en memoria, y todavía no hay proceso por teselas. Las muestras desbalanceadas (pocos objetos de una clase) bajan la exactitud de esa clase, y se recomienda dibujar un número similar de muestras por clase y de sitios distintos. Por último, las unidades: el área va en unidades de mapa al cuadrado y la escala en celdas por desviaciones típicas, no en metros.

#### Cambio de la cubierta vegetal

Compara los modelos de alturas (CHM) de dos fechas que coinciden celda a celda (si no coinciden, se igualan antes con *Ráster → Igualar rásteres*) y produce cuatro resultados. El primero es la diferencia de alturas (GeoTIFF) y un ráster de clases por celda: 1 estable, 2 crece, 3 baja, 4 pierde la cubierta (era cubierta vegetal y deja de serlo) y 5 gana cubierta. Un cambio de altura menor que el cambio mínimo (1 m por defecto) se considera ruido, y la cubierta empieza a una altura que se elige (2 m por defecto). El segundo son las perturbaciones: manchas conexas de pérdida de cubierta de al menos un tamaño mínimo (10 celdas por defecto), como polígonos (con sus alturas antes y después) y como tabla. El tercero es el resumen por unidad de gestión, si se da un archivo de polígonos (shapefile o GeoJSON): superficie, cobertura de la vegetación en cada fecha, cambio medio de altura y hectáreas que crecen, bajan, pierden o ganan cubierta. El cuarto, opcional, es el crecimiento de cada individuo: se detectan las cimas en las dos fechas con la ventana variable de la segmentación de árboles y se emparejan por distancia (el par más próximo primero, hasta una distancia máxima), y se obtiene la tabla de crecimiento (por año si se indican los años) y el recuento de árboles perdidos y nuevos.

Si se indican los años entre fechas, los resultados incluyen la tasa anual. Los límites son estos. La diferencia de dos modelos solo vale lo que valga su acuerdo vertical, y un MDT distinto, o con error de decímetros entre fechas, desplaza todas las celdas; el informe mide ese acuerdo (la mediana de la diferencia donde las dos fechas son suelo) y avisa si es comparable al cambio mínimo. Con solo LiDAR no se distingue una corta de un incendio o un viento, porque se mide la pérdida de altura y de cubierta vegetal, no su causa. Y el emparejado de árboles falla donde las copas se juntan (varias cimas por copa) o donde cambia la ventana de cimas entre fechas.

#### Modelos lineales

Relaciona una variable medida en campo (volumen, área basimétrica, biomasa, número de individuos…) con las métricas LiDAR de las mismas unidades de muestreo, y con ese modelo dibuja el mapa de la variable. Es el paso 4 y el 6 del enfoque por área (capítulo 13), hechos dentro del programa.

##### Qué necesita

Las métricas de las unidades de muestreo son la capa de unidades de muestreo que da *Extraer muestras* (modo solo métricas, formato shapefile), con un polígono por unidad de muestreo y sus métricas; puede ser una capa ya cargada (*De las capas cargadas*) o un archivo, y también vale la tabla de texto (.csv) de esa misma herramienta. Los datos de campo son un Excel (.xlsx), .csv o .txt con una fila por unidad de muestreo y una columna con el mismo nombre de unidad de muestreo que la tabla de métricas (`1`, `1.0` y `001` cuentan como la misma). Si en lugar de eso se dispone de la lista de árboles, hay que sumarla o promediarla antes por unidad de muestreo, porque la herramienta se niega si una unidad de muestreo sale en varias filas. La variable a estimar es una columna de la tabla de campo. El mapa es opcional y se hace con las tablas de texto de *Métricas por celda* (una por hoja), calculadas con las mismas opciones que las unidades de muestreo.

##### Análisis previo y variables del modelo

Las variables se dan de dos maneras, que se eligen en la ventana. En el modo automático, el programa deja de lado las métricas que repiten a otra (correlación mayor que un límite, 0,8 por defecto; por ejemplo, los percentiles altos de altura entre sí, de los que se queda la más relacionada con la variable) y elige entre las demás por BIC o AICc, con una variable por cada diez unidades de muestreo como mucho. Esa poda y la elección se repiten en cada vuelta de la validación cruzada, y no es un detalle menor, porque meter todas las métricas sin podar da modelos inestables. En el modo elegir variables, el botón abre una tabla con todas las métricas (correlación de Pearson y de Spearman con la variable, normalidad y asimetría de cada una, y cuáles repiten a otra) donde se marcan las del modelo. *Sugeridas* marca las mejor relacionadas que no se repiten. El modelo usa exactamente las marcadas, y si dos están muy correlacionadas, la herramienta lo dice y se niega a ajustar.

En los dos casos el informe empieza con el análisis previo: normalidad de la variable a estimar (Shapiro–Wilk, con la asimetría y la curtosis), y también de su logaritmo, con la recomendación de la transformación logarítmica si ésta la corrige. Una regresión lineal exige residuos normales, no variables normales, de modo que la normalidad de la variable es solo el primer aviso, y la de los residuos se comprueba tras el ajuste. En la línea de comandos, `--analyze` da solo este análisis y `--candidates` restringe las candidatas del modo automático.

##### El modelo

Es una regresión lineal múltiple por mínimos cuadrados. Las métricas pueden darse a mano (la opción con más criterio: por ejemplo, un percentil alto de altura y una medida de cobertura) o elegirse automáticamente por selección progresiva con BIC (el criterio más estricto contra el sobreajuste) o AICc, con un máximo de variables (por defecto una por cada diez unidades de muestreo, entre dos y cinco) y un límite de colinealidad (VIF ≤ 5). Las métricas absolutas (recuentos de puntos y de retornos) no se ofrecen como candidatas, porque dependen del tamaño de la unidad de muestreo y un modelo ajustado con unidades de muestreo no valdría para celdas de otro tamaño. Las de intensidad tampoco, porque en casi todos los vuelos no está calibrada entre pasadas (se pueden pedir expresamente). Si se pide el mapa, solo se ofrecen las métricas que están también en la tabla de las celdas. La transformación puede ser ninguna, el logaritmo de la variable (cuando el error crece con el tamaño de la unidad de muestreo) o el logaritmo de la variable y de las métricas (relación potencial); con logaritmos se deshace la transformación con la corrección de sesgo exp(s²/2).

##### Cómo se juzga

Un modelo siempre se ve bien con las unidades de muestreo con las que se ajustó, sobre todo si sus variables se eligieron entre decenas de candidatas con pocas unidades de muestreo. Por eso el informe pone en primer plano siete elementos.

La validación cruzada dejando una unidad de muestreo fuera, repitiendo la elección de métricas en cada prueba: cada unidad de muestreo se estima con un modelo que ni la vio ni la usó para escoger sus variables, y el RMSE, el R², el sesgo y el error absoluto medio de esa validación son lo que cabe esperar en unidades de muestreo nuevas. Se compara con el ajuste (optimista) y, si la elección es automática, con la validación del modelo final con sus variables ya fijadas; la diferencia entre las dos es el precio de haber elegido las variables con los mismos datos. La estabilidad de la elección indica qué porcentaje de las pruebas eligió cada métrica y en cuántas salió exactamente el modelo final, y si el modelo cambia a cada unidad de muestreo que se quita, la elección no es fiable aunque el R² sea alto.

Los coeficientes se dan con su error típico, t, p y VIF (cuánto se infla la varianza por la relación con las demás métricas; más de 5 es señal de colinealidad) y el test F del modelo; en una elección automática los p son optimistas. Las unidades de muestreo se examinan con la palanca (cuánto se aparta su combinación de métricas del conjunto), el residuo estudentizado y la distancia de Cook (cuánto cambiaría el modelo sin ella). El informe nombra las que mandan sobre el modelo, y la tabla `…_muestras.csv` da, para cada una, el observado, el ajustado, el estimado por validación cruzada y un aviso (`atipica`, `palanca`, `influyente`); antes de quitar una, hay que revisar su dato de campo y su posición.

Los tres supuestos de la regresión lineal se contrastan sobre los residuos y se resumen en una línea ('se cumplen los tres' o cuáles no): la normalidad de los residuos (Shapiro–Wilk), la linealidad (RESET de Ramsey: si añadir al modelo los cuadrados y cubos de sus valores ajustados lo mejora, la relación no es lineal) y la varianza constante (Breusch–Pagan). Si alguno falla, los errores típicos, los valores p y los intervalos de predicción no son fiables, y el informe sugiere el logaritmo cuando puede ayudar. Por último, los avisos en lenguaje llano señalan cuándo algo no cuadra: pocas unidades de muestreo, menos de diez por variable, validación muy por debajo del ajuste, sesgo, colinealidad o unidades de muestreo influyentes.

##### El mapa

Aplica el modelo a cada celda. Las celdas con alguna métrica fuera del rango de las unidades de muestreo quedan vacías (el modelo no ha visto nada similar; se puede desactivar), y el informe dice qué porcentaje y por qué métrica. Una variable que no puede ser negativa se deja en 0 si el modelo da un valor negativo. Opcionalmente se guarda el intervalo de predicción del 95 % (semiancho, en las unidades de la variable) en otro ráster. La suma que da el informe es una estimación con el error indicado, no un inventario.

Se usa después de *Extraer muestras* y *Métricas por celda*, con unas 30 o más unidades de muestreo medidas en campo. Con 20 unidades de muestreo o menos, la validación cruzada es muy variable y la elección automática casi siempre sobreajusta, y da mejor resultado elegir pocas métricas con sentido físico. El modelo solo vale para bosques como los de las unidades de muestreo, con el mismo tamaño de unidad de muestreo y celda, la misma densidad de puntos y fechas próximas entre el campo y el vuelo. Una posición de unidad de muestreo con errores de GPS de varios metros limita lo que cualquier modelo puede explicar.

#### Corredores eléctricos

Esta herramienta no clasifica: mide el peligro de la vegetación sobre una línea eléctrica que ya tiene los cables (clase 14) y los postes (clase 15) clasificados. Quién es cable y quién es poste lo decide la clasificación (Clasificar → *Cables y postes*), y la herramienta lee esas clases sin repetir esa búsqueda. Reconstruye cada conductor con los puntos de cable que se tocan (a menos de 2,5 m). Los conductores que corren en paralelo a menos de esa distancia (las fases de una línea, a uno o tres metros entre sí) quedan en el mismo grupo de puntos, y se separan siguiéndolos de tramo en tramo por su posición lateral y su altura (a más de 1 m uno de otro son distintos); cada uno se traza con la altura y el desplazamiento medianos cada 2 m a lo largo de su dirección. Los huecos de hasta 30 m sin puntos se rellenan con la parábola ajustada a los puntos de los lados, que sigue la catenaria en lugar de cortarla en línea recta. Los postes son los grupos de puntos de clase 15 de al menos 6 m de alto, y los vanos de un conductor son los tramos entre los postes que están a su lado (hasta 8 m).

Después recorre la vegetación (clases 3, 4 y 5) a menos de 30 m de un conductor, mide su altura sobre el MDT y agrupa los puntos en celdas de 3 m. Una celda es un riesgo por invasión si algún punto está a menos de la distancia de seguridad (3 m por defecto) del conductor, y un riesgo por caída si su altura más un margen (1 m) alcanza la posición horizontal del conductor. Salen los conductores (polilíneas 3D), los postes, los puntos de riesgo y dos tablas: el informe por vano (longitud, flecha, celdas en riesgo, distancia mínima y altura máxima de lo que está en riesgo) y la lista de riesgos. **La distancia de seguridad depende de la tensión y de la normativa que se aplique, y debe ajustarse a cada caso.**

El resultado vale lo que valga la clasificación de cables y postes. Los clasificadores automáticos confunden las copas de los árboles con cables, y en nubes de baja densidad (PNOA) los cables apenas tienen puntos. Por ese motivo se descartan los conductores de menos de 25 m y los postes de menos de 6 m o más anchos de 8 m, y la herramienta se niega (y lo dice) si todo lo que hay es ruido de ese tipo. Se ha comprobado con una línea sintética (conductores, huecos, postes y árboles de riesgo conocidos) y se ha ejecutado sobre una tira real de 290 puntos/m² (una línea con cinco o seis conductores): saca nueve trazos de conductor, con flechas de 0,6 a 1,7 m en vanos de 50 m, y la distancia a la vegetación de cada vano; no se ha comprobado cuánto se ajusta cada trazo a los puntos. Con el inventario de apoyos de una línea real (nueve apeos en QULTIVA) solo se pudo comprobar dos con la nube del dron, porque los otros siete quedan fuera de ella; con las hojas del PNOA de la zona (12 puntos/m²) sí caen los nueve. La clasificación de postes encuentra los dos (el centro de los puntos de poste queda a 1,8 y 2,0 m de cada apeo) y no marca ninguna torre más; el corredor reconoce uno, porque el otro no tiene ningún cable clasificado cerca y un poste solo cuenta si hay un conductor a menos de 8 m, y los conductores salen en trozos de unos 100 m de los 400 m entre apoyos, porque la nube solo trae cables en algunos tramos. Sobre las hojas del PNOA, con la clasificación del proveedor, el corredor reconoce ocho de los nueve apeos (a 1,1 a 4,3 m) sin ninguna torre de más, con 33 trazos de conductor para una línea de unos tres kilómetros; con la clasificación de HIPSO reconoce cinco. **No se ha contrastado con medidas de campo de las distancias ni de la flecha**, así que los números de peligro son los de la geometría de la nube y no los de una inspección. Los cortes por huecos de más de 30 m se avisan, puesto que ahí no se ha medido nada.

#### Fecha de captura en los polígonos

Todo polígono que sale de un proceso (celdas de métricas, unidades de muestreo, objetos, cambios…) lleva una sola columna de fecha, `captura`: el día en que se capturaron los datos LiDAR, sacado de la información de los archivos (el tiempo GPS de los puntos). Si los datos son de varios días salen todos (`2017-09-08|2017-10-07`), y si el archivo no la trae, el campo queda en blanco. Nunca es la fecha en que se creó la capa. Las tablas de texto la llevan como `Date`, que vale `NA` cuando no se conoce. Las capas dibujadas por el usuario no la llevan.

#### Borde de los polígonos

En el panel *Simbología* de una capa de polígonos, un *Grosor* de 0 quita el borde y deja solo el relleno.

#### Extraer muestras

Obtiene unidades de muestreo desde un archivo vectorial, generadas por muestreo (aleatorio o sistemático) o dibujadas, y da una nube por unidad de muestreo, una nube con todas y/o una capa de unidades de muestreo con sus métricas. Tres opciones condicionan el resultado.

La normalización de alturas sobre el suelo está por defecto marcada: las nubes recortadas y las métricas de cada unidad de muestreo salen con alturas sobre el suelo, restando a cada punto el MDT de su nube. Por eso hace falta el MDT de cada hoja, y si falta, la herramienta lo dice antes de empezar. Para trabajar con las cotas originales se desmarca en *Filtros y opciones*, pero las métricas por unidad de muestreo (percentiles, cobertura…) solo tienen sentido sobre alturas normalizadas, así que desmarcarla solo conviene para recortes que se van a seguir procesando.

Cuando dos unidades de muestreo se solapan, los puntos de la zona común se pueden tratar de dos maneras. *Se cuentan en todas las unidades de muestreo que los contienen* (lo predeterminado): cada unidad de muestreo es independiente y las métricas de cada una son completas. *Se asignan solo a la primera unidad de muestreo que los contiene* (la primera del archivo): un punto no se repite nunca, de modo que la suma de puntos de todas las unidades de muestreo no supera la de la nube, pero la segunda unidad de muestreo queda sin los puntos que comparte con la primera y sus métricas salen incompletas. La primera opción es la adecuada para inventario, y la segunda, para cuando cada punto debe aparecer una sola vez. Los filtros son clases, retornos y límites de Z (de altura, si se normaliza).

Cuando las unidades vienen de un archivo de polígonos, dos opciones cambian la forma de las regiones antes de recortar. En la ventana, la casilla *Partir las multipartes* hace lo mismo que `--split-parts`; el búfer solo está en la línea de comandos. `--buffer` las amplía una distancia (o las reduce, si es negativa), con las esquinas redondeadas, en las unidades de la capa: metros en un sistema proyectado y grados en uno geográfico, caso en el que el programa avisa porque casi nunca es lo que se pretende. `--split-parts` parte cada entidad multiparte en una región por parte (`<nombre>_p1`, `<nombre>_p2`…), cada una con los huecos que le corresponden. La tabla de métricas lleva por defecto todas las columnas; en la ventana, el apartado *Métricas* de «Qué se obtiene» deja marcar solo las que interesan (con un buscador), y en la línea de comandos `--metrics-columns` hace lo mismo. Con `--mode each` se obtiene una nube por entidad, con el nombre tomado del campo que indique `--name-field`.

Es el primer paso del enfoque por área (capítulo 13) y sirve para preparar nubes de unidades de muestreo de campo. Como las métricas dependen del tamaño de la unidad de muestreo y de la densidad de puntos, solo se comparan unidades de muestreo del mismo tamaño y de nubes con densidad similar.

#### Medición de troncos

Es la herramienta que antes se llamaba *Cubicar árboles* (en la línea de comandos, `stem-volume`; el nombre anterior, `cubicar`, sigue funcionando). Está pensada para nubes de escáner terrestre, móvil o fotogramétrico de una unidad de muestreo, y cada nube es una hoja. El procedimiento tiene siete pasos encadenados.

Primero, la nube se reduce a un punto por vóxel (3 cm por defecto). Segundo, se calcula el terreno (con la clase 2 si la nube la tiene, o simulando una tela) y las alturas sobre el suelo; si la nube ya viene normalizada, se marca la casilla. Tercero, se buscan troncos en una franja (de 0,7 a 3,5 m): puntos de entorno vertical, agrupados y limpiados de ramas. Cuarto, se sigue cada tronco hacia arriba y hacia abajo, ajustando un círculo a una rebanada horizontal cada pocos centímetros. Una sección vale si tiene puntos suficientes, repartidos alrededor y sin puntos dentro, y el centro de la siguiente se predice, así que el seguimiento sigue troncos inclinados o curvos. Si una zona del tronco no se ve (una oclusión, follaje, una parte con pocos puntos), el seguimiento sigue más allá, ensanchando la búsqueda, hasta un hueco máximo sin medir (2 m por defecto); a partir de ahí se da el tronco por perdido, y las secciones del hueco se rellenan por interpolación (`INTERPOLADO`).

Quinto, se estima el volumen con la fórmula de Smalian: para cada troza, V = (A₁ + A₂) / 2 · L, con A el área del círculo de cada extremo y L la longitud de la troza. Se empieza en la altura de la base (10 cm sobre el suelo por defecto) y se termina donde el diámetro baja al diámetro en punta delgada (7 cm por defecto), y la última troza se corta en ese punto. Sexto, la longitud de la troza (0,5 m por defecto) es la separación entre los diámetros medidos a lo largo del tronco.

Séptimo, los varios troncos en un mismo individuo (masas de rebrote, como el eucalipto). En cada nivel de la franja se buscan todos los círculos del grupo, no uno solo, y se encadenan de un nivel a otro, así dos troncos pegados son dos troncos y no un círculo ancho que los envuelva. La regla para tratarlos es la altura del diámetro (1,30 m). Si los troncos están separados a 1,30 m (se bifurcan por debajo), son brotes de una cepa: cada uno es un árbol, con su diámetro a 1,30 m y su volumen, y comparten el número de cepa (`CEPA`) y el aviso `BROTES`, que cuenta cuántos tiene la cepa. Si a 1,30 m es un solo tronco y se bifurca por encima, es un árbol bifurcado, un solo árbol: el diámetro a 1,30 m es el del tronco, y el volumen es el del tronco hasta la bifurcación más el de cada rama desde ella (`RAMA` en las trozas; `H_BIF` es la altura de la bifurcación; aviso `BIFURCADO`). El mismo tronco encontrado dos veces se cuenta una vez, y los troncos cuyas bases están a menos de la distancia entre bases (0,6 m) son de la misma cepa.

##### Troncos perdidos y altura libre de ramas

El seguimiento del tronco se detiene cuando pasa el hueco máximo sin medir, y en una nube de escáner terrestre eso ocurre antes de lo esperable: a partir de unos 5 m la densidad de puntos baja, y el follaje o los verticilos de ramas ocultan el tronco. El resultado es una altura libre de ramas (`ALT_LIBRE_RAMAS`) corta, marcada con el aviso `TRUNCADO`. Lo que delata el fallo es una incoherencia sencilla: si un árbol mide 20 m y su tronco se pierde a 5 m con un diámetro todavía mayor que el de punta delgada (7 cm), el tronco no ha terminado ahí, algo lo ha ocultado. En la nube de prueba (94 millones de puntos) el hueco máximo pesa tremendamente: con 2 m salían 329 árboles truncados de 471 y 45,7 m³; con 4 m, 281 de 479 y 57,6 m³; y con 6 m, 270 de 486 y 66,0 m³.

Hay dos remedios, y se pueden combinar. El primero es subir el hueco máximo (4 m es un buen punto de partida cuando hay poco follaje bajo la copa), a costa de más tiempo y del riesgo de enganchar una rama. El segundo es indicar la altura libre de ramas esperada de la especie, como porcentaje de la altura total (65 para el eucalipto, 0 para no estimar). Los troncos perdidos con un diámetro superior al de punta, y a más de un metro de la copa, se completan entonces hasta esa altura, con el diámetro bajando en línea recta desde la última sección medida hasta el diámetro en punta delgada. Salen con el aviso `ESTIMADO` en lugar de `TRUNCADO`, y el informe cuenta cuántos son y qué volumen incluye el tramo estimado. Es una estimación y no una medida, y conviene decirlo con claridad: la bajada lineal del diámetro probablemente da un volumen algo alto, y el resultado debe contrastarse con datos de campo (con 4 m de hueco y un 65 %, la nube de prueba pasó de 57,6 a 102,5 m³). Los árboles bifurcados no se estiman. En la línea de comandos son `--max-gap` y `--lost-top-share`.

##### Troncos y ramas clasificados

Editar a mano las ramas de un árbol exige distinguirlas del tronco, y el análisis ya conoce el tronco (su eje y su círculo a cada altura). Con la casilla *Guardar la nube con troncos (clase 64) y ramas (clase 65)* (o `--stem-classes` en la consola) se aprovecha para escribir `…_troncos.laz`: la nube completa, con los puntos del tronco en la clase 64 (naranja) y los demás puntos junto a un tronco, hasta la distancia máxima al eje, en la 65 (verde). El resto conserva su clase. Un punto es de tronco cuando queda dentro del círculo medido a su altura, más un margen de un vóxel; entre secciones el círculo se interpola, y por encima de la última se prolonga siguiendo la inclinación del tronco hasta la altura del árbol, con un adelgazamiento de entre 0,5 y 2 cm de radio por metro.

En un árbol bien medido de la nube de prueba el resultado es limpio, con el tronco entero en la clase 64 y la copa en la 65. Pero la clasificación es tan buena como la medición: donde el tronco no se midió (los pinos más altos de esa misma nube, por ejemplo) el tronco sale como rama, y un tronco que el análisis no detectó, si está cerca de otro árbol, también. El análisis completo de esa nube, con la clasificación, tarda unos 30 s. Por su tamaño, `…_troncos.laz` no se abre solo al terminar (se abre con *Abrir*). Una vez abierto, editar un árbol es ocultar la clase 64, rodear las ramas con el lazo en 3D y eliminarlas.

El resultado consta de tres archivos. Un shapefile (y/o CSV) con un punto en la base de cada árbol lleva `DIAM_130_CM`, `ALT_TOT`, `ALT_LIBRE_RAMAS` (altura hasta la punta delgada), `VOL_M3`, `NTROZAS`, `D_BASE_CM`, `D_TOP_CM`, `AB_M2` (área basimétrica), `INCLIN`, `PUNTOS`, `CALIDAD`, `AVISOS`, `Date`, `CEPA` (número de cepa), `BROTES` (número de árboles de esa cepa; 1 si va solo) y `H_BIF` (altura de la bifurcación, solo en árboles bifurcados). El archivo `…_trozas.csv` recoge cada troza de cada árbol, con alturas, diámetros, longitud y volumen, donde `RAMA` es 0 para el tronco y 1, 2… para cada rama de un árbol bifurcado. Si se marca *Guardar también la nube de cada árbol*, se crea una carpeta `…_arboles` con un archivo por árbol, y cada punto va al eje de tronco más cercano dentro de la distancia indicada. Los árboles se escriben por tandas de 150, con una lectura de la nube por tanda, porque el sistema limita los archivos abiertos a la vez (256 por defecto en macOS) y los 471 de la nube de prueba lo superaban.

Con esto se cierra el capítulo de procesos. El hilo común de todas las herramientas es el mismo: leer la nube como un flujo, producir un archivo nuevo junto al original y dejar en el informe la cifra que permite juzgar si el resultado es fiable.


## 8. Ráster

Las herramientas de ráster tratan imágenes y modelos que ya están en celdas: se pueden componer, medir con índices, igualar entre sí y unir en mosaicos. Su punto común es que leen por ventana y escriben por franjas comprimidas, de manera que el tamaño de la imagen apenas condiciona la memoria necesaria.

### Composición de bandas

Permite color natural, falso color infrarrojo y otras combinaciones, con estirado del contraste, brillo y gamma. El estirado puede ser sin estirar, mínimo–máximo, percentiles 2–98 %, ±2 desviaciones o cortes manuales: se escriben el valor mínimo y el máximo que se pintan (los mismos para las tres bandas; por ejemplo, de 0 a 3000 en una reflectancia de Sentinel-2) y lo que queda fuera se satura. Al elegirlos parten de los límites que se estaban usando.

### Índices espectrales

Están disponibles NDVI (Rouse et al., 1974), GNDVI (Gitelson et al., 1996), NDWI (McFeeters, 1996), EVI (Huete et al., 2002), SAVI (Huete, 1988), NDMI (Gao, 1996), NBR (Key y Benson, 2006), VARI (Gitelson et al., 2002), ExG (Woebbecke et al., 1995) o una expresión propia con `B1…Bn` o `RED`, `GREEN`, `BLUE`, `NIR`, `SWIR1`, `SWIR2`. El índice sale como capa ráster y se guarda en GeoTIFF. La elección depende de lo que se quiera medir. NDVI y GNDVI sirven para vigor y cobertura verde (el NDVI se satura con vegetación muy densa, y el GNDVI es más sensible a la clorofila). SAVI es preferible cuando el suelo desnudo pesa, y EVI en vegetación densa. NDWI y NDMI se usan para agua y humedad de la vegetación, y NBR para zonas quemadas. VARI y ExG funcionan con imágenes solo RGB (ortofotos), sin infrarrojo, aunque con menos fiabilidad.

### Igualar rásteres

Pone varios rásteres sobre las celdas de uno de referencia (mismo sistema de coordenadas, origen, tamaño de celda y dimensiones) para que coincidan celda a celda y se puedan apilar o usar juntos, por ejemplo en el análisis por objetos. Es una sola operación que **reproyecta, ajusta la extensión y remuestrea**: si un ráster está en otro sistema de coordenadas se reproyecta al de la referencia, se recorta o se amplía (con celdas sin datos) a la extensión de la referencia, y sus valores se calculan en las celdas de la referencia. Cada ráster da un archivo nuevo `<nombre>_alineado.tif`, y el original no se toca. La lista muestra la celda, el tamaño y el sistema de coordenadas de cada ráster y avisa de qué se hará con él. Por la línea de comandos es `hipso align referencia.tif ráster.tif…`, y `hipso reproject ráster.tif -o salida.tif --reference referencia.tif` hace lo mismo con un solo ráster.

El **remuestreo** es *automático* por defecto, y la ventana explica cada método al elegirlo:

- *Automático*: si el ráster son números enteros (clases, identificadores) conserva los valores (vecino más cercano, o moda si se reduce la resolución); si no, usa la media cuando las celdas de salida son mayores que las de entrada y bilineal en los demás casos.
- *Vecino más cercano*: copia el valor de la celda más cercana, sin inventar valores. Es el que hay que usar con clases, identificadores y máscaras, y el más rápido; en imágenes continuas deja bordes escalonados.
- *Bilineal*: pondera las cuatro celdas vecinas. Es suave y no se pasa de los valores originales; es lo habitual para modelos de elevaciones, temperaturas o índices cuando la celda de salida es igual o mayor que la de entrada.
- *Cúbico*: ajusta una curva (convolución cúbica de Keys, con a = −0,5) por las 16 celdas vecinas. Conserva mejor el detalle al ampliar o girar una imagen, pero puede rebasar ligeramente los valores máximos y mínimos. Es para fotografías e imágenes, no para clases.
- *Media*: promedia todas las celdas de entrada bajo la de salida. Es el que hay que usar al reducir la resolución de datos continuos, porque no pierde información ni crea aliasing; con celdas más pequeñas equivale al vecino más cercano.
- *Moda*: el valor más frecuente bajo la celda de salida; es la «media» de las clases y sirve para reducir la resolución de un ráster de clases.

Cuando los sistemas son distintos, la transformación se calcula exacta en una malla de puntos y se interpola entre ellos, y la malla se afina hasta que el error es menor que una décima de celda de la entrada. Las celdas que el ráster no cubre quedan sin datos, y el valor de «sin datos» de la salida se elige en la ventana. El trabajo se hace por franjas de filas, sin cargar el ráster entero, con todos los núcleos.

### Reproyectar

Pasa nubes (LAS y LAZ) y rásteres a otro sistema de coordenadas y, si se pide, cambia también la referencia vertical de las alturas. Se añaden los archivos (o se toman de las capas cargadas), se escribe el EPSG de destino y, cuando el archivo no declara el suyo, el de origen. Las alturas se tratan aparte porque son una operación distinta de la horizontal: pasar de alturas elipsoidales (las que da el GNSS) a ortométricas (las que miden las cotas) exige un modelo de geoide, y mezclar unas con otras produce errores de decenas de metros (en Madrid, la diferencia es de unos 51 m). Por eso las alturas solo cambian cuando se elige una *altura de salida*, que puede ser elipsoidal o un código EPSG vertical (5782 para las alturas ortométricas de la Península, 9398 para las de Canarias, 3855 para EGM2008). La *altura de entrada* se toma del propio archivo si la declara; si no la declara y se pide un cambio, la herramienta se niega y pide indicarla, porque no puede adivinarse. Las nubes y los rásteres de salida llevan escrita la referencia vertical (clave 4096 de GeoTIFF), de modo que el camino de vuelta no necesita preguntar nada. Las cotas de un ráster se transforman celda a celda.

El **destino** puede darse de tres maneras: un *código EPSG*; el *sistema de un ráster* de referencia (se toma su EPSG, y los archivos conservan su extensión); o las *celdas de un ráster* de referencia, en cuyo caso los rásteres toman también su extensión, su origen y su celda y coinciden con él celda a celda (las nubes solo toman su sistema). Para los rásteres hay además el mismo *remuestreo* que en *Igualar rásteres*, con su ayuda, y la *celda del resultado*, que por defecto es la del original (o su equivalente en el sistema nuevo si las unidades cambian). La reproyección de rásteres es una sola pasada por franjas, que no depende del tamaño del ráster.

La herramienta es estricta por defecto. Una transformación puede no hacerse con la mejor operación por tres motivos: no existe ajuste de datum entre los dos sistemas (solo se mueven las proyecciones), no hay modelo de geoide que cubra el punto (la altura se queda sin corregir) o la mejor operación necesita una rejilla que falta o que no cubre el punto, y se usa una menos precisa. Cada uno puede suponer metros de error, y por eso se rechaza con un error explícito, que dice cuál ha sido, en lugar de entregar un valor que parezca bueno. Las dos casillas finales de la ventana (en la línea de comandos, `--allow-ballpark` y `--allow-fallback`, o las variables `HIPSO_ALLOW_BALLPARK=YES` y `HIPSO_ONLY_BEST=NO`) aceptan las aproximaciones, y entonces el informe cuenta cuántos puntos pasaron por cada una. Las rejillas de España (ED50 a ETRS89 y los geoides EGM08-REDNAP de la Península y Canarias) van dentro del programa; las demás se buscan en la carpeta `HIPSO_GRID_DIR`, en `PROJ_DATA` y en `PROJ_LIB`, y con `HIPSO_NETWORK=ON` se descargan bajo demanda a la caché del usuario.

### Crear multibanda

Apila en una sola imagen las bandas de varios rásteres, en el orden de la lista (por ejemplo, R, G, B y NIR sueltos). En *Bandas* se eligen las de cada ráster (por ejemplo `1,3`), y vacío significa todas. Admite hasta 64 bandas.

### Mosaico

Une rásteres con el mismo número de bandas. Donde se solapan se elige qué hacer: *primero*, *último*, *media*, *mínimo*, *máximo*, *mediana*, *suma* o *número de capas*. También se fijan la zona que cubre (unión, intersección o la del primero), el remuestreo (vecino, bilineal, media, cúbico o moda), el tipo de dato, el tamaño de píxel y el valor de 'sin datos'.

Estas herramientas leen las imágenes por ventana y escriben un Cloud Optimized GeoTIFF (teselas de 512 × 512, vistas reducidas hasta caber en una tesela y todos los directorios al principio), por lo que se pueden unir imágenes de varios gigabytes con poca memoria y el resultado se abre al instante en QGIS o en un visor web. La compresión es JPEG (calidad 90, YCbCr, como las ortofotos del PNOA) para las imágenes de 8 bits y tres bandas, y deflate (sin pérdida) para todo lo demás (bandas de satélite, clases, alturas); en la línea de comandos se elige con `--compress deflate|jpeg` y `--quality`. Como referencia, el mosaico de tres hojas de ortofoto PNOA de 0,25 m (1,7 GB) ocupa 2,4 GB en JPEG frente a 14,7 GB sin pérdida, y sus colores difieren de los originales en menos de medio nivel sobre 255. Todos los rásteres deben estar en el mismo sistema de coordenadas, porque no se reproyecta.

### Tamizar, proximidad y relleno de huecos

Tres operaciones sobre un ráster, en la ventana *Operaciones ráster* (pestaña *Raster*, grupo *Geoprocesos*, que reúne también *Vectorizar* y *Calculadora*; cada operación abre su propia ventana) y por la línea de comandos. `hipso sieve` elimina las manchas pequeñas de un ráster de clases: cada conjunto de celdas conexas del mismo valor y con menos celdas que `--threshold` toma el valor de su vecino mayor (con preferencia por los vecinos que no son pequeños, y con `--eight` se cuentan también las conexiones por esquina). `hipso proximity` calcula la distancia euclídea exacta de cada celda a la más próxima de las clases pedidas (`--values`, o toda celda distinta de cero), en unidades del mapa o en celdas, con un alcance máximo opcional. `hipso fill-nodata` rellena las celdas sin dato con la media ponderada por el inverso del cuadrado de la distancia de la celda con valor más cercana en cada una de las ocho direcciones, hasta `--max-distance` celdas, con `--smoothing` pasadas de suavizado sobre lo rellenado. Ninguna de las tres se ha contrastado con las herramientas equivalentes de GDAL, que no estaban disponibles; se han comprobado con casos de resultado conocido. Contrastadas con GDAL 3.13 sobre un MDT real de 501 × 501 celdas: *proximidad* es exacta y coincide con la transformada de distancia euclídea exacta de SciPy con una diferencia máxima de 8·10⁻⁶ (la de `gdal_proximity.py` es aproximada y se aparta hasta 0,39 m con celdas de 2 m); *tamizar* deja sin manchas menores que el umbral, como `gdal_sieve.py`, y da el mismo resultado salvo cuando una mancha pequeña toca vecinos de igual tamaño (el desempate es distinto) y en cadenas de manchas pequeñas, un 2,1 % de las celdas con 4 vecinos y un 0,24 % con 8; el *relleno de huecos* implementa exactamente su fórmula (media ponderada por el inverso del cuadrado de la distancia en ocho direcciones; coincide con una referencia independiente a 2·10⁻⁵) y no cambia ninguna celda con dato, pero no es el algoritmo de `gdal_fillnodata.py`, que rellena menos celdas y da valores distintos (media de 0,37 m y máximo de 6 m en huecos de un MDT de relieve variado).

### Reclasificar

Cambia los valores de una banda de un ráster según una tabla de reglas, y escribe un ráster nuevo `<nombre>_reclas.tif` (por la línea de comandos, `hipso reclass ráster.tif --rules "0..10 = 1; 10..25 = 2; >= 25 = 3"`). Cada regla es una línea (o se separan con `;`) con la forma *condición = resultado*. La condición es un valor (`3`), un intervalo con los dos extremos incluidos (`0..10` o `0 - 10`), una comparación (`< 0`, `<= 5`, `> 100`, `>= 100`) o `nodata` para las celdas vacías; el resultado es un número o `nodata`. **Si varias reglas valen para una celda, manda la primera.** `else = …` da el valor de las celdas que ninguna regla recoge; si no se escribe, la ventana pregunta qué hacer con ellas (conservan su valor, quedan vacías o pasan a un valor) y `--others` hace lo mismo por la línea de comandos. El botón *Ejemplos* trae las reglas más habituales (unir clases, pasar intervalos a clases, quitar negativos, una máscara 1/0, rellenar vacíos) y *Una regla por clase* escribe una regla `valor = valor` por cada clase que tiene el ráster, para cambiar solo las que se quiera.

El resultado usa el **tipo entero más pequeño** que cabe (8 bits si todos los valores son enteros entre 0 y 254, 16 bits hasta 65534, decimal en los demás casos), con 255, 65535 o −9999 como valor de «sin datos» según el tipo, y las vistas reducidas no promedian las clases. Si un valor de clase coincide con el de «sin datos» la herramienta se detiene y pide elegir otro. Se trabaja por franjas de filas.

### Cortar por máscara

Deja sin datos lo que queda fuera de una máscara y, si se quiere, corta el ráster a la caja de la máscara (`hipso clip-raster ráster.tif --mask máscara`). La máscara es una capa de polígonos (shapefile, GeoPackage, GeoJSON o KML), que se pasa al sistema del ráster si declara otro, o **un ráster**, en cuyo caso se conservan las celdas donde tiene datos distintos de cero (si está en otro sistema se transforma celda a celda, más despacio). Una celda se conserva si su centro cae dentro de un polígono, y las celdas son siempre las del ráster original: no se remuestrea nada. *Cortar el ráster a la caja de la máscara* ajusta el tamaño a celdas enteras del original, y *Conservar lo que queda fuera* invierte la máscara (no se corta en ese caso). El valor de las celdas vaciadas es el que declara el archivo o, si no declara ninguno, 0 en los rásteres enteros y −9999 en los decimales; se puede fijar a mano. Si la máscara no toca el ráster, la herramienta lo dice y recuerda comprobar que estén en el mismo sistema. Los rásteres de varias bandas se cortan completos.

### Calculadora ráster

Calcula una expresión celda a celda sobre uno o varios rásteres, que se llaman `A`, `B`, `C`… en el orden de la lista (en la línea de comandos, con el nombre que se les dé: `NOMBRE=archivo.tif`). Una banda concreta es `A.2`, y un ráster solitario sin nombre tiene las bandas `B1`, `B2`… (por ejemplo, `(B4 - B3) / (B4 + B3)` es el NDVI de una imagen de cuatro bandas).

La expresión admite números, las operaciones `+ - * / % ^`, las comparaciones (`< <= > >= == !=`), `and`, `or`, `not` y las funciones `abs`, `sqrt`, `ln`, `log10`, `exp`, `pow`, `min`, `max`, `floor`, `ceil`, `round`, `sin`, `cos`, `tan`, `atan2`, `clamp`, `if` e `isnan`. Verdadero vale 1, falso vale 0, y cualquier valor distinto de 0 cuenta como verdadero. La precedencia es la habitual: `-2^2` es `-(2^2)`, `2^3^2` es `2^(3^2)`, y `not` se une como el signo menos, de modo que `not A > 0` es `(not A) > 0`.

Una celda vacía da un resultado vacío, salvo dentro de `isnan()`, y una división por cero también. La ventana trae un menú de *Ejemplos* (diferencia, máscara, quitar los valores negativos, reescalar a 0–255, rellenar los vacíos de A con B y NDVI).

Admite **cualquier número de rásteres a la vez** (se han probado cuatro, con huecos y con celdas distintas, contra numpy: una celda vacía en cualquiera de ellos vacía el resultado de las ramas que la leen). Los rásteres deben tener las mismas celdas. Si no las tienen, se rechaza, salvo que se marque ponerlos sobre las del primero (`--align`, con el remuestreo que se elija); si están en otro sistema de coordenadas, se reproyectan al del primero en el mismo paso (véase *Igualar rásteres*). El resultado es decimal de 32 bits o entero de 8 o de 16 bits (`--dtype f32|u8|u16`), y con un ráster de clases conviene marcar la casilla que evita que las vistas reducidas promedien (`--nearest`).

La expresión se compila una vez y se evalúa por franjas de filas en paralelo, de modo que la memoria sigue a una franja y no al ráster, y un error de la expresión dice en qué columna está. Se escribe `…_calc.tif`. En la línea de comandos: `hipso calc "if(A > 0, A - B, 0)" A=chm_2020.tif B=chm_2015.tif`.

### Análisis ráster

Cuatro herramientas del grupo *Geoprocesos* de la pestaña *Raster* abren la misma ventana genérica (la que se genera a partir de la orden de la línea de comandos, con las mismas opciones y la ayuda de cada una).

*Rasterizar* convierte una capa vectorial en un GeoTIFF: cada celda recibe el valor del campo elegido del último elemento que la cubre, o 1 si no se indica campo. Las celdas se toman de un ráster de referencia (`--reference`) o de un tamaño de celda (`--cell`). Sirve para llevar rodales o parcelas a la calculadora ráster. *Filtro focal* sustituye cada celda por la media, la mediana, el mínimo, el máximo, la suma, el rango, la desviación típica, la mayoría o un suavizado gaussiano de las celdas de una ventana a su alrededor (cuadrada o circular, de radio `--radius` en celdas); con `--min-valid` se deja vacía la celda que tiene menos vecinas con valor de las indicadas. *Estadísticas ráster* da el recuento, el mínimo, el máximo, la media, la desviación, la mediana y los percentiles de las celdas con valor, y escribe el histograma en un CSV (`--bins` intervalos). *Tabular áreas* mide el área de cada clase de un ráster (`--values`) dentro de cada zona de otro, como tabla CSV. En la línea de comandos son `hipso rasterize`, `focal`, `rstats` y `tabulate`.

### Vectorizar un ráster

Convierte un ráster en polígonos. Forman un polígono las celdas contiguas que comparten una clase (el mismo valor entero, que es lo normal), que caen en un rango de valores (`--range desde,hasta`) o que están entre dos cortes (`--breaks 0,5,10`, con un polígono por intervalo). El contorno sigue los bordes de las celdas, o se simplifica con *Simplificar* (`--simplify`, en unidades del mapa; 0 conserva los escalones).

Cada polígono lleva `valor`, `area_m2` y `celdas`, y además `lim_inf` y `lim_sup` si se agrupa por rango o por cortes. *Celdas mínimas por polígono* descarta las manchas pequeñas, y *Las celdas se tocan también por las esquinas* (`--diagonal`) une las que solo se tocan en un vértice. El resultado es un shapefile o, si se marca, un GeoPackage (`…_poligonos.shp`).

Sirve para pasar a polígonos una clasificación, una máscara de agua o el resultado de la calculadora. En la línea de comandos: `hipso vectorize clases.tif --classes --min-cells 4`.

## 9. Vectorial

El tratamiento vectorial cubre lo necesario para acompañar a la nube: abrir capas, crearlas, editarlas y simbolizarlas.

Las capas se abren con *Abrir* de la cabecera (o *Archivo → Añadir nubes y capas*), y admiten imágenes y rásteres (GeoTIFF, JPEG 2000, PNG y JPEG con archivo de coordenadas, ENVI, SRTM), shapefiles, GeoJSON, GeoPackage y KML/KMZ; también se pueden arrastrar a la ventana. Una capa nueva puede ser de puntos, líneas o polígonos, con los campos de su tabla y el **tipo de cada uno** (véase *Tipos de campo*): se digitaliza en la capa activa (con ajuste a vértices), se edita su tabla de atributos y se guarda como shapefile, GeoJSON o GeoPackage (según la extensión que se elija) con la proyección de la nube. La simbología puede ser de color único, por categorías (cada valor tiene su botón de color, que se cambia uno a uno) o por rampa, con clasificación y etiquetas, y la edición vectorial incluye seleccionar, mover, editar vértices, deshacer y rehacer. Las capas de polígonos visibles alimentan la herramienta de unidades de muestreo.

### Estadística zonal

Para cada polígono, resume las celdas de un ráster cuyo centro cae dentro de él (el CHM medio de cada rodal, por ejemplo): número de celdas, mínimo, máximo, media, suma, desviación típica (de población, dividida entre el número de celdas) y mediana. El resultado es una capa de polígonos, no una tabla aparte: conserva la geometría y todos los campos de la original, y añade un campo por estadístico (`count`, `min`, `max`, `mean`, `sum`, `stddev` y `median`), con un prefijo opcional (`chm_` da `chm_mean`) que además evita pisar campos ya existentes, como exige el límite de 10 caracteres de un shapefile. Un polígono sin celdas queda con recuento 0 y los demás valores vacíos. Ráster y polígonos deben estar en el mismo sistema, y si no, el programa lo avisa. Está en la pestaña *Vectorial* (grupo *Geoprocesos*) y en la línea de comandos como `hipso zonal chm.tif --regions rodales.shp --prefix chm_`.

### Geoprocesos con polígonos

Cada una de las demás operaciones tiene su propia ventana (*Comprobar validez* y *Reparar geometrías* en el grupo *Geometría* de la pestaña *Vectorial*, el resto en *Geoprocesos*), todas con resultado en una capa nueva y con la tabla de atributos conservada, salvo donde se dice. *Validar* comprueba las reglas habituales de validez (anillos cerrados, sin autointersecciones, huecos dentro de su polígono, partes que no se solapan) y lista las entidades que fallan con el motivo. *Reparar* corrige las inválidas con la regla par-impar (Ledoux et al., 2014): un lazo se convierte en dos partes y los bordes repetidos se cancelan; las válidas no se tocan. El *búfer* amplía cada polígono la distancia indicada, o lo reduce si es negativa, con esquinas redondeadas; las entidades que desaparecen al reducirse no se escriben, porque el formato shapefile no admite una forma vacía. Sobre una capa de puntos, el búfer da un círculo de esa distancia alrededor de cada punto, y una distancia de 0 o negativa se rechaza con un mensaje. *Disolver* une las entidades que comparten el valor de un campo (o todas, si no se indica) y añade el campo `n` con cuántas se unieron. *Unión de capas* superpone dos capas y entrega las piezas que están en las dos (con los campos de ambas, los de la segunda precedidos por `b_`), solo en la primera y solo en la segunda. Búfer, disolver y unión se niegan con entradas inválidas y piden repararlas antes, porque una operación booleana sobre una geometría rota da resultados que parecen correctos y no lo son; la casilla *Reparar antes* (`--make-valid`) lo hace en el mismo paso. Por la línea de comandos son `hipso is-valid`, `make-valid`, `buffer --distance`, `dissolve --field` y `union`. Las capas deben estar en el mismo sistema de coordenadas, y la distancia del búfer se da en sus unidades (grados, si es geográfico).

#### Intersección

*Intersección de dos capas* (en *Vectorial → Geoprocesos*, y `hipso intersect`) se queda solo con las piezas que están en las dos capas, con los campos de ambas (los de la segunda, con el prefijo `b_`).

#### Otros geoprocesos con capas

En la misma ventana: *Diferencia* quita de la primera capa lo que cubre la segunda (las piezas conservan los campos de la primera), *Diferencia simétrica* deja lo que está solo en una de las dos (lo de la segunda con sus campos precedidos por `b_`), *Centroides* y *Puntos en la superficie* dan una capa de puntos con los atributos de cada elemento (el punto en la superficie cae siempre dentro del polígono, aunque sea cóncavo o tenga huecos, y el centroide no), y *Unir atributos por localización* copia a cada elemento de la primera capa los campos de los elementos de la segunda que intersectan, contiene, tocan o cruzan (el primer coincidente, o la suma y la media de los campos numéricos), con el campo `n_coinc` que cuenta cuántos coinciden. Diferencia y diferencia simétrica exigen polígonos válidos, como las demás operaciones booleanas.

#### Capas y tablas

El grupo *Capas y tablas* de la pestaña *Vectorial* reúne siete herramientas que, como las de análisis ráster, abren una ventana generada a partir de la orden de la línea de comandos, y que también son nodos del constructor de flujos. Las capas deben estar en el mismo sistema de coordenadas que los rásteres y capas con que se combinan, y si no, el programa lo avisa en lugar de mezclarlas.

*Extraer valores* añade a una capa un campo por banda de uno o varios rásteres, con el valor bajo cada elemento (en el punto, o en un punto interior de la línea o del polígono; con `--bilinear` se interpola entre las cuatro celdas más cercanas). *Unir tabla* añade a una capa las columnas de una tabla CSV, de texto o de Excel por un campo clave (`--layer-key` y `--table-key`), y reconoce la coma decimal. *Combinar capas* junta capas del mismo tipo de geometría en una, con la unión de sus campos y, si se pide, un campo con el archivo de origen de cada elemento. *Dividir por campo* escribe una capa por cada valor distinto de un campo, por ejemplo un archivo por rodal. *Recortar capa* conserva de una capa de puntos, líneas o polígonos lo que cae dentro de los polígonos de otra, o con `--erase` lo que cae fuera; cada pieza conserva los atributos de su elemento y el que no deja nada se descarta. *Cercanía* añade la distancia al elemento más cercano de otra capa, su número de orden y, si se pide, uno de sus campos. *Resumen estadístico* calcula el recuento, la suma, la media, el mínimo, el máximo y la desviación de campos numéricos de una capa o tabla, por grupos de un campo. En la línea de comandos son `hipso extract`, `join-table`, `merge`, `split`, `clip-vector`, `near` y `summary`.

#### Voronoi

*Voronoi* (en *Vectorial → Geoprocesos*, y `hipso voronoi` en la línea de comandos) toma una capa de puntos y da una capa de polígonos con la celda de Voronoi de cada punto, es decir, la zona que está más cerca de él que de ningún otro, con los atributos del punto. Sirve, por ejemplo, para repartir el territorio entre estaciones o para medir el área que le toca a cada árbol.

Las celdas de los puntos del borde del conjunto no tienen fin. Sin recorte se dejan fuera, y con *Una caja algo mayor que los puntos* o con una capa de polígonos se cortan por ella. Los puntos repetidos (mismas x e y) cuentan como uno, de modo que el primero conserva la celda y los demás no tienen.

Con *Guardar también las aristas de Delaunay* (`--delaunay`) se escribe otra capa (`…_voronoi_delaunay.shp`) con las líneas entre los puntos vecinos de la triangulación. El resultado es un shapefile, un GeoPackage o un GeoJSON, según la extensión.

### Adosar y cortar polígonos

Con una capa de polígonos en edición y *Digitalizar* activo, un trazo que empieza y termina dentro de polígonos (del mismo o de varios distintos) añade, como elemento nuevo y pegado a todos ellos, lo que cubre fuera de los polígonos. Cualquier otro trazo que atraviese polígonos corta todos los que parte de una vez (cada parte conserva los atributos del original), empiece y termine donde empiece y termine.

### Unir, separar y agrupar

En el panel *Edición* con una capa vectorial elegida, *Unir seleccionados* une los polígonos seleccionados en uno solo (el resultado ocupa el lugar y conserva los atributos del primero; si no se tocan, queda un elemento de varias partes). *Separar partes* convierte cada parte de un elemento de varias partes en un elemento propio, con los mismos atributos, y *Agrupar en multiparte* hace lo contrario sin unir la geometría. Todo se puede deshacer.

### Transformar, reformar, copiar y comprobar

Con una capa vectorial en edición, el panel *Edición* ofrece además las herramientas siguientes.

*Ajustar a bordes* es una casilla junto a *Ajustar a vértices*: al digitalizar o mover vértices, el punto salta a la posición más cercana de un borde de cualquier capa visible, aunque en ese lugar no haya un vértice. Si hay un vértice a tiro, manda sobre el borde.

*Transformar selección* abre una ventana para desplazar, girar y escalar los elementos seleccionados. El giro (en grados, en sentido antihorario) y la escala se aplican alrededor del centro de la selección entera, de modo que los elementos conservan su disposición, y el desplazamiento va en las unidades de la capa. Se puede deshacer.

*Reformar* vale para capas de polígonos y actúa con *Digitalizar*. Un trazo que cruza el contorno de un polígono al menos dos veces sustituye el tramo de borde más corto entre el primer y el último cruce por el trazo: si pasa por dentro, corta una esquina, y si sale y vuelve a entrar, añade un saliente. Se reforman los polígonos seleccionados o, si no hay selección, los que toca el trazo. Mientras el botón está activo, el trazo nunca crea un polígono nuevo.

*Copiar* guarda los elementos seleccionados con sus atributos, *Pegar* los añade a la capa activa y *Pegar como capa* crea con ellos una capa nueva. Solo se pega en una capa del mismo tipo de geometría, y los campos se emparejan por nombre sin distinguir mayúsculas: los que la capa de destino no tiene se descartan y los que la copia no trae quedan vacíos.

*Comprobar topología* abre una ventana que revisa la capa activa. En polígonos busca los **solapes** (superficie común a dos polígonos), los **huecos** (zonas rodeadas de polígonos que ninguno cubre) y las **autointersecciones** (contornos que se cruzan consigo mismos); en líneas, los **cabos sueltos**, es decir, extremos que no tocan ningún otro extremo ni línea. La tolerancia fija la superficie mínima que cuenta como solape o hueco y, en líneas, la distancia a la que dos extremos se tienen por unidos. Pulsar un incumplimiento de la lista selecciona los elementos implicados y acerca el mapa. En la línea de comandos, `hipso topology capa.shp` escribe el informe en un CSV (regla, elementos, coordenadas y superficie).

### Seleccionar por localización

*Seleccionar por localización…* selecciona los elementos de la capa activa que intersectan, están dentro de, contienen, tocan (solo por el borde), cruzan o no tocan a algún elemento de otra capa cargada, y la selección puede ser nueva, ampliarse o reducirse.

### Tipos de campo

Cada campo de una capa vectorial tiene un tipo, que se elige al crear la capa (*Nueva capa*) o el campo (*Añadir campo* en la tabla de atributos) y se muestra al pasar el ratón por el encabezado de la columna. Los tipos son:

- *Texto*: cualquier texto.
- *Número entero*: números sin decimales, como `12` o `-3`.
- *Número decimal*: números con decimales; se admite la coma o el punto.
- *Sí / No*: se guarda como `Sí` o `No`, y se entiende también sí, no, s, n, true, false, 1 y 0.
- *Fecha*: se guarda como `AAAA-MM-DD`, y se entiende también `31/03/2025`; no se admiten fechas que no existen (el 31 de febrero).
- *Hipervínculo*: una dirección web (`https://…`) o la ruta de un archivo. En la tabla se muestra subrayado y un clic lo abre (el doble clic lo edita; el número de fila sigue seleccionando la fila).

Al escribir en una celda, el valor se comprueba contra el tipo de su campo cuando se sale de ella: si no encaja, la celda queda como estaba y debajo de la tabla se explica qué se esperaba. El botón derecho sobre un encabezado permite **cambiar el tipo** de un campo; el cambio solo se hace si todos los valores valen para el tipo nuevo, y si no, se dice cuál falla. Los números y las fechas se alinean y se ordenan por su valor (las fechas por día), y el texto por orden alfabético sin distinguir mayúsculas. *Seleccionar por atributo* lee el valor escrito según el tipo del campo (un desplegable *Sí / No* en los campos sí/no, una fecha en los de fecha) y avisa si no es del tipo. La simbología graduada solo ofrece los campos numéricos y de fecha, aunque un campo de texto tenga valores que parezcan números.

Los tipos se conservan al guardar. En un *shapefile* los campos son numéricos (enteros o con decimales), lógicos, de fecha o de carácter, de modo que otros programas los leen con su tipo; el shapefile no distingue un hipervínculo de un texto, así que los hipervínculos se anotan en `<nombre>_tipos.txt` junto a él. En un *GeoPackage* las columnas se declaran `INTEGER`, `REAL`, `BOOLEAN`, `DATE` y `TEXT`, y los hipervínculos como `TEXT_HYPERLINK` (otros programas pueden leerlos como texto). En *GeoJSON* los números y los sí/no se escriben como números y como `true`/`false`, y las fechas y los enlaces como texto; al leerlo, una columna cuyos valores son todos fechas `AAAA-MM-DD` se toma como de fechas. Cuando un archivo no declara tipos (KML, o una capa de otro programa) cada campo se tipifica por sus valores: entero o decimal si todos son números, y texto si no.

### Calculadora de campos

*Calculadora de campos…* rellena un campo (que se crea si no existe) con una expresión, en todos los elementos o solo en los seleccionados. La expresión admite números, textos entre comillas simples, campos (`"especie"`, o el nombre a secas), las variables `$area`, `$perimeter`, `$length`, `$x`, `$y`, `$id` y `$npoints`, las operaciones `+ - * / % ^`, `||` para unir textos, comparaciones, `and`, `or` y `not`, los valores `true` y `false`, y las funciones `round`, `floor`, `ceil`, `abs`, `sqrt`, `ln`, `log10`, `pow`, `min`, `max`, `upper`, `lower`, `trim`, `len`, `concat`, `left`, `right`, `substr`, `replace`, `if` y `coalesce`, y para las fechas `date('2025-03-31')`, `today()`, `year`, `month` y `day`. Cada campo se lee **según su tipo**: los números como números, las fechas como fechas (se comparan por día, también con un texto que se lea como fecha, `fecha > '2025-01-01'`; una fecha más o menos días es una fecha, y la resta de dos fechas da días), los campos sí/no como verdadero o falso, y los de texto y los hipervínculos como texto (`"codigo" = 7` no selecciona un código `007`, y `'10'` va antes que `'7'`). Un valor vacío sigue vacío sea cual sea el tipo. Si el campo ya existe se respeta su tipo y cada valor se comprueba contra él (un texto en un campo numérico, o un decimal en un campo entero, da un error con el número de fila y no cambia nada; `round()` lo resuelve); si el campo se crea, toma el tipo de los valores (fecha, sí/no, entero, decimal o texto). Por ejemplo, `$area / 10000` da la superficie en hectáreas en un sistema proyectado. Las superficies y longitudes están en las unidades de la capa. *Probar* muestra el resultado en las tres primeras filas sin cambiar nada, y si la expresión falla en alguna fila no se modifica ninguna.

### Guardar la edición

Una capa editada se guarda sobre sí misma (*Guardar*, o el botón *Guardar* al terminar de editar) o como capa nueva (*Guardar como…* / *Guardar como capa nueva…*). En un GeoPackage se escribe sobre la tabla de la que se leyó la capa y se conservan las demás tablas de objetos del archivo; si el archivo tiene datos que no se conservan (estilos guardados por QGIS, tablas solo de atributos, teselas) el programa lo avisa y deja elegir entre guardar igualmente o guardar como capa nueva. La capa vuelve a escribirse en el sistema de coordenadas en que estaba la tabla. Un KML o KMZ no se sobrescribe: se guarda siempre como archivo nuevo, para no perder sus estilos. Las herramientas de edición (*Digitalizar*, *Vértices*, *Mover*) se activan y desactivan con el mismo botón; al desactivarlas tras hacer cambios, el programa pregunta si se guardan, se descartan o se dejan sin guardar.

### Recortar con polígonos

*Recortar con polígonos* (en *LiDAR → Preparar*) recorta las nubes con una capa de polígonos: una nube por polígono, o una sola ajustada al perímetro de todos, con un búfer opcional que deja una zona de seguridad. El búfer no construye un polígono mayor: cada punto se compara con su distancia al borde del polígono (exacta, con esquinas redondeadas), lo que prepara el recorte en milisegundos; construir el polígono ampliado con la operación de búfer de capas cuesta segundos con un polígono de 50 000 vértices y solo compensa con decenas de millones de puntos por polígono, por lo que se mantienen las dos formas.

### Volumen sobre MDT con polígonos

Cuando las zonas son los polígonos de un archivo, el resultado es una copia de esa capa con los volúmenes en su tabla de atributos (`Area_m2`, `Vol_sobre`, `Vol_bajo`, `Vol_neto`, `H_media`, `H_max`, `Sin_MDT`); con las AOI dibujadas o todo el ráster sigue siendo una tabla CSV.

### Capas grandes

Una capa con muchos objetos (más de 4 000, o más de 150 000 vértices) se dibuja como una sola imagen en lugar de un dibujo por objeto, y por ello el coste depende del tamaño de la pantalla y no del número de vértices. Mientras se mueve la vista se desplaza y escala la última imagen, y se redibuja cuando la vista lleva un instante quieta. Un shapefile de 41 mil objetos pasó de 4,5 GB de memoria a 233 MB y dejó de colgar el equipo. Para seleccionar y consultar, las capas usan un índice espacial R-tree en memoria.

## 10. Rutas y formatos

Este capítulo reúne las reglas de dónde se escribe cada resultado y qué formatos se leen y se escriben, porque de ambas cosas depende que un flujo de trabajo sea repetible.

Los resultados van a la misma carpeta que la entrada, con otro nombre, salvo que se elija otra: todas las ventanas de herramientas tienen la fila *Guardar en*, con una carpeta de resultados que es la misma para todas (vacía, significa junto al original), y en la línea de comandos se indica con `--results-dir`. Una ejecución nunca hereda la ruta de otra ni sobrescribe sin avisar, y si se elige a mano una ruta que ya existe, se pide confirmación. Las tablas (métricas, árboles, unidades de muestreo) se escriben como capa vectorial (shapefile, con el `.prj` de la nube), CSV o ambos. Los nombres de campo de un shapefile tienen como mucho 10 caracteres, y `<nombre>_campos.txt` explica cada uno. Toda salida de métricas lleva la fecha de captura (`Date`), sacada del tiempo GPS de los puntos (o de la hora Unix, si el escáner la usa, como GeoSLAM: una fecha futura no puede ser la de captura).

### Formatos que se leen

Las nubes admitidas son LAS/LAZ (incluido COPC, que además se lee solo en la parte que hace falta), PLY (ASCII o binario) y E57, el formato de intercambio de los escáneres terrestres y móviles. En E57 cada escaneo se coloca con su pose, las coordenadas esféricas se pasan a cartesianas, el color y la intensidad se guardan en 16 bits y cada punto lleva el número de su escaneo como ID de fuente. PLY y E57 se convierten a LAZ, con *Abrir* en el programa o con `hipso import`. También se leen ASCII `x y z` y, entre los vectores, shapefile y GeoJSON.

Entre los rásteres, el GeoTIFF se lee también en sus variantes Cloud Optimized y BigTIFF, y el ASCII grid. El JPEG 2000 (`.jp2`, `.j2k`, `.j2c`) es la forma en que vienen las bandas de Sentinel-2, y se lee con código propio en Rust, sin librerías externas: se admiten la georreferencia GMLJP2 (la de Sentinel-2) y GeoJP2, las bandas de 8 a 16 bits y el color de las imágenes TCI. Solo se decodifican las teselas de la ventana pedida, y se usan las vistas reducidas que el propio archivo lleva (cada nivel divide el tamaño por dos), de modo que abrir una banda de 10 m es inmediato. Los valores salen tal cual vienen (en Sentinel-2, el número digital; el 0 es 'sin dato' pero no se enmascara).

El PNG y el JPEG se leen con su archivo de coordenadas (`.pgw`/`.pngw`/`.wld` y `.jgw`/`.jpgw`/`.wld`; el `.prj` da el sistema). ENVI admite `.bil`, `.bsq`, `.bip`, `.img` y `.dat` con su `.hdr`, y los `.flt` de Esri, y las teselas SRTM `.hgt` se reconocen por su nombre (tipo `N37W006`). Estos formatos se leen enteros en la memoria, por lo que son para imágenes de tamaño normal, y una ortofoto enorme conviene en GeoTIFF o JPEG 2000.

No se leen ECW (formato propietario: solo se lee con un SDK que no es de código abierto, y un programa en Rust puro no puede llevarlo; se convierte con QGIS, GeoLibre o RustGis, por ejemplo en QGIS con *Ráster → Conversión → Traducir*), MrSID, Imagine `.img`, NetCDF, HDF o GRIB. Un JPEG 2000 con cambios de progresión (POC) o cabeceras de paquetes empaquetadas tampoco se lee (no los usa Sentinel-2), y una imagen girada o con píxeles no cuadrados exige remuestrearla antes.

### GeoPackage

La lectura de un `.gpkg` con varias tablas de objetos abre la ventana *Tablas del GeoPackage*, donde se elige cuáles se cargan. Cada una es una capa (`archivo · tabla`) con sus atributos, y cada tabla se reproyecta desde su propio sistema de coordenadas, aunque las tablas del mismo archivo tengan sistemas distintos. Sirven como unidades de muestreo o de gestión en el recorte, las métricas y el resto de herramientas. El archivo se lee entero en memoria (límite de 2 GB).

La escritura se produce al guardar una capa vectorial (o una capa nueva) con la extensión `.gpkg`. Se escribe un GeoPackage con la tabla de objetos, sus atributos, su geometría y su sistema de coordenadas (EPSG), y un índice espacial R-tree (`rtree_*`). El escritor es propio, en Rust puro, sin SQLite ni GDAL, y el resultado se ha validado abriéndolo con SQLite y con GDAL. Un archivo nuevo lleva una tabla, con el nombre del archivo; una capa leída de un GeoPackage se guarda sobre su propia tabla (véase *Guardar la edición*).

### KML y KMZ

Se leen y se escriben `.kml` y `.kmz` (un zip con un `doc.kml`). KML va siempre en longitud y latitud (WGS 84, EPSG:4326), así que al abrirlo se pasa al sistema de la nube como cualquier otra capa, y al guardar se pasan los vértices a longitud y latitud. Se leen los *Placemark* con puntos, líneas y polígonos (con huecos), también dentro de *MultiGeometry*; `name` y `description` pasan a campos con esos nombres, y los datos de `ExtendedData` (`Data` y `SimpleData`) a un campo cada uno. Como una capa tiene un solo tipo de geometría, se queda con el del primer elemento y cuenta los demás como omitidos. Las altitudes, los estilos, las carpetas, las superposiciones y los modelos 3D no se leen ni se escriben. Una capa sin sistema de coordenadas conocido y con coordenadas proyectadas no se puede guardar como KML: se guarda como shapefile o se le asigna antes un sistema.

### Formatos que se escriben

Se escriben LAS/LAZ, COPC (*Exportar puntos* en formato COPC, o `hipso copc`), GeoTIFF y shapefile, GeoPackage (guardando una capa con extensión `.gpkg`), GeoJSON, KML/KMZ, CSV y PNG. Para el GeoTIFF, *Mosaico*, *Crear multibanda*, *Igualar rásteres* y `hipso cog` escriben Cloud Optimized GeoTIFF, con teselas de 512 y vistas reducidas con todos los directorios al principio del archivo, y `cog` acepta también imágenes de varias bandas, de cualquier tamaño. La compresión del COG es JPEG (imágenes de 8 bits y tres bandas) o deflate (todo lo demás). **No se admiten zstd ni LERC**: ambas exigirían una dependencia escrita en C, y HIPSO está escrito en Rust sin ellas para que se compile y se distribuya en cualquier sistema sin bibliotecas externas. Un COG con zstd o LERC se puede producir fuera de HIPSO a partir del GeoTIFF que escribe.

El GeoJSON se obtiene guardando una capa con esa extensión, o con `hipso geojson` para convertir entre shapefile y GeoJSON, y lleva el sistema de coordenadas en el miembro `crs`.

Los archivos que están solo en la nube (OneDrive, iCloud) y no se han descargado no se pueden leer, y el programa lo indica; hay que descargarlos antes.

### Sistemas de coordenadas

Si lo que se combina está en sistemas distintos (nubes entre sí, con el MDT, con rásteres o con unidades de muestreo y muestras en shapefile o GeoJSON), HIPSO lo pasa al sistema de la primera nube y lo dice en el informe: '"mdt.tif" estaba en ETRS89 / UTM 29N (EPSG:25829) y se ha pasado a ETRS89 / UTM 30N (EPSG:25830)'. Las nubes y los rásteres se convierten en copias temporales (que se borran al acabar) y las unidades de muestreo en memoria, y si todo está ya en un sistema no se hace nada. Las transformaciones son las de PROJ, con las mallas del IGN y del ICGC para ED50 a ETRS89 (Península, Baleares y Cataluña) y el geoide EGM08-REDNAP (Península y Canarias) incluidos en el programa. El motor es global, y fuera de España se comprueba contra PROJ con `scripts/check_world_crs.py` (Estados Unidos, Canadá, Portugal, Francia, Alemania, Reino Unido, Japón, Chile y Australia, con diferencias de 0,001 m como máximo en los casos que no necesitan rejillas adicionales). Cuando una transformación no puede hacerse con la mejor operación (falta una rejilla, el punto cae fuera de su cobertura, no hay ajuste de datum), no entrega un valor aproximado en silencio: se detiene con un error, salvo que se acepten las aproximaciones.

**Por defecto solo cambian X e Y**, y las alturas se conservan; para cambiarlas se pide expresamente una referencia vertical (véase *Reproyectar*, en el capítulo 8, con `--vertical-in` y `--vertical-out`). Entre husos, un ráster mantiene su celda (por ejemplo 1 m) y se remuestrea con el método elegido (por defecto, bilineal, o vecino más cercano si sus valores son enteros: clases), por franjas y con todas las bandas. Una nube convertida cuesta una pasada sobre ella, de modo que para trabajar mucho con ella conviene convertirla una vez con `hipso reproject nube.laz -o nube_25830.laz --to 25830` (también vale para rásteres; con `--from` se indica el sistema de un archivo que no lo declara). En las nubes LAS 1.4 con formato de punto 6 o superior, cuando se pide una referencia vertical, el sistema se escribe como WKT compuesto (horizontal y vertical, con el bit de WKT de la cabecera), que es lo que ese formato prevé; en los formatos anteriores se declara en las claves GeoTIFF, como hasta ahora.

#### Rejillas de otros países

HIPSO lleva incorporadas solo las rejillas de España (ED50 a ETRS89 y el geoide EGM08-REDNAP). Los ajustes de datum y los geoides de otros países (por ejemplo, NAD27 a NAD83 en Estados Unidos con NADCON, o OSGB36 a ETRS89 en Reino Unido con OSTN15) necesitan sus rejillas, y no se prueban dentro del programa. Quien las necesite en su distribución las añade así:

1. Descargar la rejilla del repositorio PROJ-data (<https://cdn.proj.org>), que ofrece GeoTIFF (`.tif`), NTv2 (`.gsb`) y GTX (`.gtx`); por ejemplo, `us_noaa_conus.tif` para NAD27 a NAD83 en Estados Unidos continentales, y `uk_os_OSTN15_NTv2_OSGBtoETRS.tif` para OSGB36 a ETRS89 en Reino Unido.
2. Guardarla en una carpeta y apuntar a ella con la variable de entorno `HIPSO_GRID_DIR` (varias carpetas, separadas como en `PATH`) antes de abrir HIPSO o de lanzar `hipso`. También se leen las carpetas de `PROJ_DATA` y `PROJ_LIB`, y el directorio de usuario de PROJ donde escribe `projsync`.
3. Alternativamente, con `HIPSO_NETWORK=ON` el programa descarga la rejilla que le falta, leyendo solo los bloques necesarios; con `HIPSO_GRID_DOWNLOAD=YES` la descarga entera a una caché (`HIPSO_GRID_CACHE`) y las siguientes veces la encuentra allí.
4. Comprobar el resultado: sin la rejilla, la herramienta rechaza la transformación o, si se aceptan las aproximaciones, cuenta cuántos puntos pasaron por una operación menos precisa (véase *Reproyectar*). Con la rejilla instalada no debe aparecer ese aviso.

Los límites son los siguientes. *Igualar rásteres*, *Reproyectar*, *Apilar*, *Mosaico* y la *Calculadora* reproyectan al sistema de la referencia (o del primer ráster) los rásteres que estén en otro, siempre que ambos lo declaren; si uno no lo declara, no se puede comprobar y el resultado lo advierte. No se convierten nubes desde o hacia grados. ED50 en Canarias no está definido y no debe usarse. Si un archivo no declara su sistema, no se puede comprobar y el resultado lleva un aviso, y ETRS89 y WGS 84 del mismo huso se consideran equivalentes y solo avisan (difieren en torno a un metro). En la lista de capas, un ⚠ marca la nube o la capa cuyo sistema no coincide con el de la primera nube.

## 11. Rendimiento y memoria

El rendimiento de HIPSO descansa en una decisión de diseño, que consiste en no cargar nunca la nube entera, y todo lo que sigue es una consecuencia de ella.

Las nubes grandes no se cargan enteras: la vista dibuja una muestra (*Máx.* en *Vista → Visualización*) y los procesos leen en flujo. Un LAZ que necesita varias pasadas se descomprime una vez a un archivo temporal que se borra al terminar, y por eso hay que dejar espacio libre en el disco temporal (la nube sin comprimir). El tamaño de tesela y el búfer se ajustan en *Teselas y rendimiento*, y la memoria es aproximadamente (núcleos × tesela). Con teselas de 250 m y 30 m de búfer, un equipo de 16 GB trabaja sin problema con nubes de cualquier tamaño.

### Tamaño máximo de un ráster

Un ráster en memoria admite hasta 2^29 celdas (unos 537 millones, 2 GiB de datos). Con una celda de 1 m son unos 23 000 × 23 000 m, mucho más que una hoja o que el área de un inventario por masas. Si una zona o una celda más fina lo supera, HIPSO lo dice y pide usar una celda mayor o dividir la zona en teselas (capítulo 12); no está previsto trabajar con rásteres mayores.

Los pasos se reparten en dos grupos según su consumo. Son en flujo (memoria casi constante) la información, el MDT por *media por celda*, el CHM por máximo, el MDS por máximo, las métricas, el recorte, la exportación (también a LAZ y normalizada), el adelgazado y los duplicados. Son por teselas (memoria = núcleos × tesela) la clasificación, el MDT por TIN o IDW, el MDS por TIN o IDW y el CHM *pit-free*. Para trabajar solo con lo que está en pantalla, se seleccionan los puntos y el proceso se aplica solo a ellos.

Las capas vectoriales grandes se dibujan como una imagen y se seleccionan con un índice R-tree (capítulo 9). En la vista en planta, la nube se dibuja con un lienzo atómico compartido, y mover la vista pasó de 49,5 ms a 18,4 ms. En OBIA, las claves de borde se cuentan por bloques, sin acumular un gigabyte de memoria.

Un archivo COPC es un LAZ cuyos bloques son los nodos de un octree. Al recortar con unidades de muestreo se leen solo los nodos que tocan las unidades de muestreo (en una nube de 50 millones de puntos, una ventana de 100 × 100 m lee el 4 % de los puntos), y al mostrarla con decimación se leen solo los primeros niveles del árbol (89,9 millones de puntos en 0,25 s en lugar de recorrerlos todos). Al acercar la vista, la ventana se vuelve a leer a más detalle en segundo plano (apartado 4.2), y ese mecanismo sirve también para cualquier LAS/LAZ gracias a la copia COPC que el programa guarda en su caché. El resto de las herramientas lo leen como un LAZ normal.

## 12. Trabajar con miles de teselas

Un vuelo es una colección de cientos o miles de hojas (por ejemplo, las de 2 × 2 km del PNOA), y el programa las trata con ocho reglas que conviene conocer antes de lanzar un lote.

Cada hoja se procesa por separado y da su propio resultado. Si una hoja falla, el proceso se detiene con el mensaje del error y se borra lo parcial de esa hoja (las hojas ya terminadas se conservan), y no hay límite de hojas porque nunca se mantienen varias en memoria a la vez. Solo se cargan como hojas las que se quieran ver, ya que para procesar no hace falta abrirlas: en la línea de comandos se pasa la lista de archivos y no se dibuja nada, y la interfaz sirve para probar los parámetros en una o dos hojas. Antes de lanzar el lote se prueba en una hoja representativa (con relieve, bosque cerrado y claros), donde se elige el método de suelo, el tamaño de celda y las alturas. Como entrada conviene usar LAZ, que pesa unas 5 veces menos y se descomprime en paralelo.

Las salidas llevan el nombre de la hoja (`hoja_mdt_1m_tin.tif`) y van a la carpeta de cada hoja. Una ejecución nueva no sobrescribe, sino que crea `_2`, `_3`…, y para repetir un lote se usa otra carpeta de resultados o se borra lo anterior. Respecto a la alineación entre hojas, los centros de las celdas de las métricas están en múltiplos del tamaño de celda, de modo que el borde de la hoja cae casi siempre a mitad de una celda: esa celda sale en las tablas de las dos hojas, cada una con la mitad de sus puntos, y sus recuentos, percentiles y cobertura no valen. Se resuelve con `--neighbors` (véase *Sin costura en los bordes*), que da a cada celda todos sus puntos y la escribe una sola vez.

El MDT y el CHM pueden dar además un mosaico de todas las hojas (casilla o `--mosaic`), y para el resto de rásteres se usa *Mosaico* en Ráster; los mosaicos de miles de hojas se escriben por franjas comprimidas, sin cargar las imágenes enteras. En cuanto a los bordes, la clasificación por teselas usa un búfer alrededor de cada tesela *interna* de la hoja, pero en el borde de la hoja no hay vecinos al otro lado. Para que los bordes salgan como los daría todo el bloque está la opción de las hojas vecinas, que se explica más abajo.

### Antes de lanzar el lote: `hipso catalog`

Lee solo las cabeceras de las hojas (sin leer ningún punto, así que mil hojas tardan un segundo) y dice si forman un teselado que se pueda procesar: sistema de coordenadas distinto o sin declarar, referencia vertical distinta, hojas idénticas que contarían los puntos dos veces, solapes (y si son tan grandes que parecen pasadas de vuelo y no un teselado), huecos rodeados de hojas, formatos de punto distintos que pierden hora GPS o color, y LAZ sin índice espacial. Acepta archivos o una carpeta, y acaba con código 1 si hay errores (sistemas distintos, hojas repetidas, archivos ilegibles). `--tolerance` (1 m) es lo que pueden solaparse o separarse las hojas sin que cuente.

### Sin costura en los bordes

Con `--neighbors M` (en la interfaz, *Hojas vecinas hasta (m)*; 0 apaga la opción) en `classify`, `dtm`, `dsm`, `chm` y `metrics`, cada hoja se calcula con las hojas que están a menos de M metros y se queda solo con lo suyo: los bordes salen como los habría dado todo el bloque. Es opcional y cuesta leer también las vecinas (a lo sumo ocho por hoja; la memoria sube lo de una hoja y sus vecinas, no la del vuelo). Qué hace en cada herramienta:

- **Clasificar:** el filtro de suelo, el de ruido y el de edificios ven lo que hay al otro lado del borde (un edificio cortado por el borde no se toma por suelo, ni un pájaro partido en dos por ruido aislado). Solo se clasifican y se escriben los puntos de la hoja. Con M igual al búfer de las teselas (30 m) basta.
- **MDT, MDS y CHM por triangulación (TIN, IDW, sin huecos):** la triangulación llega más allá del último punto de la hoja, y el resultado se recorta a su extensión. El CHM usa también los MDT de las vecinas para el suelo de sus puntos (si una vecina no tiene MDT, se sigue sin ella).
- **MDS y CHM por máximo:** el máximo de una celda no depende de las vecinas, así que solo se usa la opción si se rellenan las celdas vacías o se suaviza; sin ellas el resultado sería idéntico y no se lee nada de más.
- **Métricas:** cada celda es de una sola hoja (la que contiene su centro; en la franja entre dos extensiones, la más cercana) y recibe todos sus puntos, también los de la vecina; el MDT se amplía con los de las vecinas. Se usa al menos una celda de distancia.
- **Coste medido** (cuatro hojas PNOA de 1 km, 51 millones de puntos, 10 núcleos): sin la opción no cambia nada, y con `--neighbors 30` la memoria es la misma y el tiempo sube una lectura de cada hoja (en un bloque grande es una sola, no una por vecina, porque antes de procesar se cortan de cada hoja las franjas que sus vecinas necesitan): clasificar de 11 s a 14 s, MDT de 6 s a 9 s, MDS por TIN de 6 s a 9,5 s, CHM sin huecos de 6,5 s a 10 s y métricas de 4,5 s a 7 s.
- **Límite:** en el borde exterior del bloque no hay vecinas, y los huecos rellenados por interpolación allí siguen dependiendo de lo que haya dentro. Las hojas deben estar en el mismo sistema de coordenadas (`hipso catalog` lo comprueba) y no solaparse (un solape cuenta dos veces los puntos).

El ejemplo siguiente usa la línea de comandos en PowerShell (en Linux y macOS es igual, con comodines).

```
$hojas = Get-ChildItem hojas\*.laz
hipso info $hojas
hipso catalog hojas\
hipso classify $hojas --ground csf --noise isolated
hipso dtm $hojas --surface tin --cell 1
hipso dsm $hojas --method tin --cell 0.5
hipso chm $hojas --cell 0.5 --method pitfree
hipso metrics $hojas --cell 20 --strata
```

Los comandos que reciben varias hojas dan un resultado por hoja. Para paralelizar entre máquinas, se reparten las hojas en lotes y se lanza un proceso por lote y por carpeta de resultados.

## 13. Encadenar un flujo de inventario por área (ABA)

El enfoque por área (*area-based approach*, ABA) modela una variable de inventario (volumen, área basimétrica, biomasa, altura dominante…) con las métricas LiDAR de unidades de muestreo de campo y aplica el modelo a toda la superficie, celda a celda. El orden de los pasos no es arbitrario, porque cada uno fija una condición que el siguiente necesita, y con HIPSO se encadena en siete.

En el primer paso se preparan las hojas: se abre una hoja de prueba y se revisa *Información* (proyección, densidad, clases, fecha de captura). Si falta la proyección, se usa *Asignar proyección*, y si las coberturas son de densidades distintas, *Adelgazar nube* las lleva a una densidad común. El segundo paso es clasificar (ruido y suelo), o usar la clase 2 de la entrega, y generar el MDT de cada hoja. El tercero obtiene las métricas de las unidades de muestreo de campo con *Extraer muestras*, con el archivo de unidades de muestreo (o con centros y radio), en modo solo métricas. Da una capa con un polígono por unidad de muestreo y todas las métricas de *Métricas por celda* (7.3), con el número de unidad de muestreo, y opcionalmente una nube por unidad de muestreo para revisarlas. Debe dejarse marcado *Alturas sobre el suelo*, porque las métricas de las unidades de muestreo y las de toda la superficie tienen que calcularse igual.

El cuarto paso es el modelo. *Modelos lineales* une (por el nombre de la unidad de muestreo) la tabla de métricas con la tabla del inventario de campo (Excel, CSV o texto), ajusta la regresión, la valida dejando una unidad de muestreo fuera cada vez y enseña si la elección de variables es estable y qué unidades de muestreo mandan. Lo que debe mirarse es la validación cruzada, no el ajuste. Si se prefiere otro programa estadístico, la tabla de métricas y la de campo se pueden unir igualmente por el nombre. Hay que usar los mismos parámetros (límite de cobertura, tamaño de unidad de muestreo, MDT) en las unidades de muestreo y en el resto del territorio.

El quinto paso calcula las métricas de toda la superficie: *Métricas por celda* en todas las hojas, con el tamaño de celda igual al de la unidad de muestreo (por ejemplo, 400 m² son celdas de 20 m, o de 15 m para unidades de muestreo de radio 7,98 m). Cada hoja da su capa de celdas con las mismas columnas que las unidades de muestreo. El sexto aplica el modelo: en la misma herramienta, marcando *Hacer el mapa* y dando las tablas de texto de las celdas del paso anterior, sale el ráster de la variable y, opcionalmente, el del intervalo de predicción. Las celdas fuera del rango de las unidades de muestreo quedan vacías. El séptimo es el contraste, que compara con las unidades de muestreo retenidas para validar y con el CHM o la segmentación de árboles si se quiere un segundo criterio.

Hay tres cautelas finales. Las unidades de muestreo y la cobertura LiDAR deben ser de fechas próximas (la columna `Date` ayuda a comprobarlo). La unidad de muestreo debe tener el tamaño de la celda. Y conviene excluir las celdas con menos de unos cientos de puntos, las que tienen agua o edificios y las que caen en el borde de las hojas.

### El constructor de flujos

Encadenar a mano los pasos de un inventario (clasificar, MDT, CHM, árboles, métricas) obliga a repetirlos con cada zona. La pestaña *Flujos* los dibuja como un grafo: cada herramienta es un **nodo**, cada archivo que una produce y otra consume es un **hilo**, y el flujo entero se guarda en un archivo `.model.json` que se vuelve a ejecutar sobre otros datos.

A la izquierda está la paleta, con las herramientas agrupadas como en la cinta y un buscador, y las entradas (*Archivo…*, *Carpeta de hojas…* y *Del mapa…*, que toma una capa o nube ya cargada). Un clic añade un nodo en el centro del lienzo. Cada herramienta muestra como **puertos** sus opciones que reciben archivos (la nube, el MDT, el polígono…), con un asterisco los obligatorios; se arrastra desde el puerto de salida de un nodo hasta uno de entrada para conectarlos, y solo se iluminan los puertos que admiten lo que el nodo produce (una herramienta ráster no recibe lo que produce una vectorial). Una entrada que recibe varios archivos, como una carpeta de hojas, ejecuta la herramienta una vez por archivo cuando la opción solo admite uno. A la derecha, el nodo seleccionado muestra sus opciones (las mismas de la línea de comandos, con su ayuda, y las poco usadas plegadas en *Más opciones*) y, sin ningún nodo seleccionado, la carpeta de resultados y si se abren en el mapa al terminar. En el lienzo se desplaza arrastrando el fondo, se acerca con Ctrl y la rueda o con el gesto de pellizco, y *Ordenar* coloca los nodos en columnas según su distancia a las entradas y *Encuadrar* los pone todos a la vista. Supr quita lo seleccionado, Ctrl+D lo duplica y Ctrl+Z y Ctrl+Y deshacen y rehacen.

La **estrella** de la cabecera de cada nodo marca los resultados que se conservan; los de los demás nodos son intermedios y se borran en cuanto nadie los necesita. Bajo el lienzo se listan los **problemas** a medida que se construye (un bucle, una entrada sin archivo, una opción obligatoria vacía, un hilo entre tipos que no encajan), y *Ejecutar* no empieza hasta que no queda ninguno. La ejecución es en segundo plano: cada nodo muestra su estado (naranja en curso, verde hecho, rojo con error), *Cancelar* detiene el proceso en marcha y, si un paso falla o se cancela, se borra todo lo que esa ejecución había escrito, sin dejar medio resultado. Los resultados conservados van a la carpeta elegida, cada nodo en una carpeta con su nombre y sin sobrescribir nunca una existente (si el nombre ya está, se usa `_2`), y se abren en el mapa si así se pidió. El flujo en que se trabaja se guarda solo y se recupera al volver a abrir el programa.

Cada nodo ejecuta el programa `hipso` con las opciones del nodo, de modo que un flujo hace exactamente lo que harían los mismos comandos escritos uno tras otro. Por eso un flujo también se ejecuta sin interfaz: `hipso run flujo.model.json` (con `--input nombre=ruta` para cambiar un nodo de entrada por nombre, y `--results-dir` para la carpeta de resultados) y `hipso model-validate flujo.model.json`, que lista lo que impide ejecutarlo. Un archivo de flujo solo puede nombrar órdenes de `hipso` y sus opciones (nunca una orden de shell ni otro programa), así que ejecutar el flujo de otra persona no ejecuta nada distinto de lo que `hipso` sabe hacer. Las ayudas de las opciones salen del propio programa, de modo que una orden nueva de `hipso` aparece sola en la paleta.

## 14. Línea de comandos

`hipso` hace lo mismo que la interfaz, por lotes, y con `--help` se ve la lista completa de comandos y de opciones.

```
hipso info nube.laz
hipso thin nube.laz --method density --density 1
hipso classify nube.laz --ground csf --noise isolated --buildings --vegetation
hipso dtm nube.laz --cell 1 --surface tin
hipso dsm nube.laz --cell 0.5 --method tin
hipso chm nube.laz --cell 0.5 --method pitfree
hipso run flujo.model.json --input Nube=hoja.laz --results-dir resultados
hipso model-validate flujo.model.json
hipso terrain mdt.tif --product slope --percent
hipso terrain mdt.tif --product streams --threshold 10000
hipso terrain mdt.tif --product viewshed --observer 504000 4405000 --max-distance 2000
hipso metrics nube.laz
hipso trees chm.tif --method silva --crowns
hipso obia chm.tif --cloud nube.laz --roughness 2 --scale 8 --rules reglas.txt
hipso obia chm.tif --cloud nube.laz --samples muestras.shp --class-field clase
hipso obia chm.tif --cloud nube.laz --scale 12 --min-cells 200 --kmeans 4 --strata --plots 30
hipso clip nube.laz --regions muestras.geojson --mode each
hipso metrics nube.laz --cell 20 --structure
hipso change chm_2019.tif chm_2024.tif --years 5 --stands unidades.shp --trees
hipso corridor nube_clasificada.laz --dtm mdt.tif --min-distance 4
hipso reproject nube.laz --to 25830 --vertical-in ellipsoidal --vertical-out 5782 -o nube_orto.laz
hipso zonal chm.tif --regions rodales.shp --prefix chm_
hipso buffer rodales.shp --distance 25
hipso dissolve rodales.shp --field especie
hipso union rodales.shp zonas.shp
hipso intersect rodales.shp zonas.shp
hipso is-valid rodales.shp
hipso sieve clases.tif --threshold 5 --eight
hipso proximity clases.tif --values 3 --max-distance 200
hipso fill-nodata mdt.tif --max-distance 50
hipso clip nube.laz --regions rodales.shp --buffer 10 --mode each --name-field nombre
hipso classify nube.laz --noise elm
hipso import escaneo.ply --epsg 25830 -o escaneo.laz
hipso catalog hojas/ --tolerance 1
hipso dtm hojas/*.laz --surface tin --neighbors 30
hipso classify hojas/*.laz --ground csf --neighbors 30
hipso chm hojas/*.laz --method pitfree --neighbors 30
hipso metrics hojas/*.laz --cell 20 --neighbors 20
hipso trees chm_a.tif --around chm_b.tif --around chm_c.tif --margin 30
hipso fit nube_2.laz --reference nube_1.laz --plane --trim 0.8
hipso import escaneo_tls.e57 --epsg 25830
hipso colorize nube.laz --image ortofoto.tif
hipso intensity nube.laz --trajectory trayectoria.txt
hipso intensity nube.laz --altitude 3200 --exponent 2
hipso copc nube.laz
hipso dtm nube.laz --cell 1 --breaklines bordes_camino.shp
hipso geojson capa.shp
hipso cog chm.tif
hipso --results-dir D:/resultados chm nube.laz --dtm mdt.tif
hipso stem-volume muestra_tls.laz --csv --tree-clouds --log-length 0.5 --top-diameter 0.07
hipso stem-volume muestra_tls.laz --csv --stem-classes --max-gap 4 --lost-top-share 0.65
hipso dedup nube.laz --also class,gps
hipso compare nube2.laz --reference nube1.laz --distance 0.5
hipso fit nube2.laz --reference nube1.laz
hipso swath vuelo.laz --cell 1
hipso stack r.tif g.tif b.tif nir.tif -o multibanda.tif
hipso mosaic a.tif b.tif -o mosaico.tif --overlap mean
hipso mosaic orto1.tif orto2.tif -o orto.tif --compress jpeg --quality 85
hipso cog ortofoto.tif
hipso align chm.tif mdt.tif ortofoto.tif
hipso render nube.laz --view 3d -o vista.png --color class
hipso export nube.laz -o puntos.csv --items x,y,z,class --header
hipso export nube.laz --normalize --dtm mdt.tif -o altura.laz
```

## 15. Atajos

Los atajos están también en *Inicio → Atajos de teclado*.

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

Los problemas más frecuentes tienen causas identificables, y cada uno se describe con su síntoma y su solución.

Si el volumen de las copas sale absurdo (cientos de miles de m³) o las alturas son enormes, se ha usado como superficie el ráster de copas, que guarda números de árbol, en vez del CHM. En *Volumen sobre MDT* hay que elegir el CHM (el programa ya no confunde ambos al buscarlos solo). Si un MDT o un CHM antiguo es un rectángulo con valores inventados fuera de la nube, se calculó con una versión anterior; los rásteres nuevos siguen el contorno de la nube, y basta regenerarlos. El mensaje 'No hay MDT de esta nube' se resuelve generando antes el MDT con la herramienta MDT.

Si el informe de *Medición de troncos* avisa de muchos árboles `TRUNCADO`, el tronco se perdió antes de llegar al diámetro en punta, casi siempre por el hueco máximo sin medir (véase el capítulo 7.3, donde se explica cómo subirlo y cómo estimar la altura libre de ramas). Si la nube no se ve completa, se sube *Máx.* en *Vista → Visualización* y se pulsa Intro. Si los diámetros de los árboles salen vacíos, hay que comprobar que la nube tenga suelo (o marcar que ya está normalizada) y que la franja de búsqueda esté dentro de la altura de los troncos. Si el mosaico de imágenes dice que los sistemas son distintos, se pasan los rásteres al mismo sistema con `hipso reproject` antes de unirlos. Un resultado a medias tras detener un proceso se borra solo, y si queda algún archivo, se borra a mano.

Cuando la interfaz va lenta o a saltos, lo primero es saber qué usa para dibujar. Con una tarjeta gráfica moderna usa wgpu (DirectX 12, Vulkan o Metal). Con un controlador antiguo (por ejemplo una Intel HD de 2015, que no llega a DirectX 12), wgpu solo ofrece un adaptador por programa, que dibuja con el procesador y va muy lento, y el programa elige entonces OpenGL. Para ver lo que detecta se ejecuta `hipso-gui --gpu-info`. Para medir los fotogramas por segundo con una nube, `hipso-gui nube.laz --bench` (cuenta 8 s y sale). Para forzar uno, se usa la variable de entorno `HIPSO_RENDERER` con `glow` (OpenGL) o `wgpu`.

## 17. Bibliografía

Referencias de los métodos descritos en este manual.

### Clasificación y modelos del terreno

- Axelsson, P. (2000). DEM generation from laser scanner data using adaptive TIN models. *International Archives of Photogrammetry and Remote Sensing*, 33(B4), 110–117.
- Evans, J. S. y Hudak, A. T. (2007). A multiscale curvature algorithm for classifying discrete return LiDAR in forested environments. *IEEE Transactions on Geoscience and Remote Sensing*, 45(4), 1029–1038.
- Kraus, K. y Pfeifer, N. (1998). Determination of terrain models in wooded areas with airborne laser scanner data. *ISPRS Journal of Photogrammetry and Remote Sensing*, 53(4), 193–203.
- Rusu, R. B., Marton, Z. C., Blodow, N., Dolha, M. y Beetz, M. (2008). Towards 3D point cloud based object maps for household environments. *Robotics and Autonomous Systems*, 56(11), 927–941.
- Shepard, D. (1968). A two-dimensional interpolation function for irregularly-spaced data. *Proceedings of the 23rd ACM National Conference*, 517–524.
- Pingel, T. J., Clarke, K. C. y McBride, W. A. (2013). An improved simple morphological filter for the terrain classification of airborne LIDAR data. *ISPRS Journal of Photogrammetry and Remote Sensing*, 77, 21–30.
- Qin, N., Tan, W., Ma, L., Zhang, D., Guan, H. y Li, J. (2023). Deep learning for filtering the ground from ALS point clouds: A dataset, evaluations and issues. *ISPRS Journal of Photogrammetry and Remote Sensing*, 202, 246–261.
- Sithole, G. y Vosselman, G. (2004). Experimental comparison of filter algorithms for bare-Earth extraction from airborne laser scanning point clouds. *ISPRS Journal of Photogrammetry and Remote Sensing*, 59(1–2), 85–101.
- Demantké, J., Mallet, C., David, N. y Vallet, B. (2011). Dimensionality based scale selection in 3D lidar point clouds. *The International Archives of the Photogrammetry, Remote Sensing and Spatial Information Sciences*, XXXVIII-5/W12, 97–102.
- Varney, N., Asari, V. K. y Graehling, Q. (2020). DALES: A large-scale aerial LiDAR data set for semantic segmentation. *2020 IEEE/CVF Conference on Computer Vision and Pattern Recognition Workshops (CVPRW)*, 717–726.
- Weinmann, M., Jutzi, B., Hinz, S. y Mallet, C. (2015). Semantic point cloud interpretation based on optimal neighborhoods, relevant features and efficient classifiers. *ISPRS Journal of Photogrammetry and Remote Sensing*, 105, 286–304.
- Zhang, K., Chen, S.-C., Whitman, D., Shyu, M.-L., Yan, J. y Zhang, C. (2003). A progressive morphological filter for removing nonground measurements from airborne LIDAR data. *IEEE Transactions on Geoscience and Remote Sensing*, 41(4), 872–882.
- Zhang, W., Qi, J., Wan, P., Wang, H., Xie, D., Wang, X. y Yan, G. (2016). An easy-to-use airborne LiDAR data filtering method based on cloth simulation. *Remote Sensing*, 8(6), 501.

### Análisis por objetos

- Baatz, M. y Schäpe, A. (2000). Multiresolution segmentation: an optimization approach for high quality multi-scale image segmentation. En J. Strobl, T. Blaschke y G. Griesebner (eds.), *Angewandte Geographische Informationsverarbeitung XII*, 12–23. Wichmann.
- Blaschke, T. (2010). Object based image analysis for remote sensing. *ISPRS Journal of Photogrammetry and Remote Sensing*, 65(1), 2–16.
- Arthur, D. y Vassilvitskii, S. (2007). k-means++: the advantages of careful seeding. *Proceedings of the 18th Annual ACM-SIAM Symposium on Discrete Algorithms*, 1027–1035.
- Breiman, L. (2001). Random forests. *Machine Learning*, 45(1), 5–32.
- Cochran, W. G. (1977). *Sampling techniques* (3.ª ed.). Wiley, Nueva York.
- Congalton, R. G. (1991). A review of assessing the accuracy of classifications of remotely sensed data. *Remote Sensing of Environment*, 37(1), 35–46.
- Lloyd, S. P. (1982). Least squares quantization in PCM. *IEEE Transactions on Information Theory*, 28(2), 129–137.
- Neyman, J. (1934). On the two different aspects of the representative method: the method of stratified sampling and the method of purposive selection. *Journal of the Royal Statistical Society*, 97(4), 558–625.

### Modelos lineales

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

### Estructura del dosel

- MacArthur, R. H. y MacArthur, J. W. (1961). On bird species diversity. *Ecology*, 42(3), 594–598.
- Monsi, M. y Saeki, T. (1953). Über den Lichtfaktor in den Pflanzengesellschaften und seine Bedeutung für die Stoffproduktion. *Japanese Journal of Botany*, 14, 22–52.
- Van Ewijk, K. Y., Treitz, P. M. y Scott, N. A. (2011). Characterizing forest succession in central Ontario using lidar-derived indices. *Photogrammetric Engineering & Remote Sensing*, 77(3), 261–269.

### Modelo de copas y árboles individuales

- Chen, Q., Baldocchi, D., Gong, P. y Kelly, M. (2006). Isolating individual trees in a savanna woodland using small footprint lidar data. *Photogrammetric Engineering & Remote Sensing*, 72(8), 923–932.
- Dalponte, M. y Coomes, D. A. (2016). Tree-centric mapping of forest carbon density from airborne laser scanning and hyperspectral data. *Methods in Ecology and Evolution*, 7(10), 1236–1245.
- Eysn, L. et al. (2015). A benchmark of lidar-based single tree detection methods using heterogeneous forest data from the Alpine Space. *Forests*, 6(5), 1721–1747.
- Khosravipour, A., Skidmore, A. K., Isenburg, M., Wang, T. y Hussin, Y. A. (2014). Generating pit-free canopy height models from airborne lidar. *Photogrammetric Engineering & Remote Sensing*, 80(9), 863–872.
- Popescu, S. C. y Wynne, R. H. (2004). Seeing the trees in the forest: using lidar and multispectral data fusion with local filtering and variable window size for estimating tree height. *Photogrammetric Engineering & Remote Sensing*, 70(5), 589–604.
- Silva, C. A., Hudak, A. T., Vierling, L. A., Loudermilk, E. L., O'Brien, J. J., Hiers, J. K., et al. (2016). Imputation of individual longleaf pine (*Pinus palustris* Mill.) tree attributes from field and LiDAR data. *Canadian Journal of Remote Sensing*, 42(5), 554–573.
- Vincent, L. y Soille, P. (1991). Watersheds in digital spaces: an efficient algorithm based on immersion simulations. *IEEE Transactions on Pattern Analysis and Machine Intelligence*, 13(6), 583–598.

### Inventario por área y métricas

- Hosking, J. R. M. (1990). L-moments: analysis and estimation of distributions using linear combinations of order statistics. *Journal of the Royal Statistical Society B*, 52(1), 105–124.
- McGaughey, R. J. *FUSION/LDV: Software for LIDAR Data Analysis and Visualization*. USDA Forest Service, Pacific Northwest Research Station.
- Næsset, E. (2002). Predicting forest stand characteristics with airborne scanning laser using a practical two-stage procedure and field data. *Remote Sensing of Environment*, 80(1), 88–99.
- Roussel, J.-R., Auty, D., Coops, N. C., Tompalski, P., Goodbody, T. R. H., Meador, A. S., Bourdon, J.-F., de Boissieu, F. y Achim, A. (2020). lidR: an R package for analysis of airborne laser scanning (ALS) data. *Remote Sensing of Environment*, 251, 112061.
- White, J. C., Wulder, M. A., Varhola, A., Vastaranta, M., Coops, N. C., Cook, B. D., Pitt, D. y Woods, M. (2013). A best practices guide for generating forest inventory attributes from airborne laser scanning data using an area-based approach. *The Forestry Chronicle*, 89(6), 722–723.

### Intensidad

- Höfle, B. y Pfeifer, N. (2007). Correction of laser scanning intensity data: data and model-driven approaches. *ISPRS Journal of Photogrammetry and Remote Sensing*, 62(6), 415–433.

### Estadística de valores raros

- Leys, C., Ley, C., Klein, O., Bernard, P. y Licata, L. (2013). Detecting outliers: do not use standard deviation around the mean, use absolute deviation around the median. *Journal of Experimental Social Psychology*, 49(4), 764–766.
- Rousseeuw, P. J. y Croux, C. (1993). Alternatives to the median absolute deviation. *Journal of the American Statistical Association*, 88(424), 1273–1283.
- Tukey, J. W. (1977). *Exploratory Data Analysis*. Addison-Wesley.

### Análisis de terreno

- Barnes, R., Lehman, C. y Mulla, D. (2014). Priority-flood: an optimal depression-filling and watershed-labeling algorithm for digital elevation models. *Computers & Geosciences*, 62, 117–127.
- Beven, K. J. y Kirkby, M. J. (1979). A physically based, variable contributing area model of basin hydrology. *Hydrological Sciences Bulletin*, 24(1), 43–69.
- Horn, B. K. P. (1981). Hill shading and the reflectance map. *Proceedings of the IEEE*, 69(1), 14–47.
- O'Callaghan, J. F. y Mark, D. M. (1984). The extraction of drainage networks from digital elevation data. *Computer Vision, Graphics, and Image Processing*, 28(3), 323–344.
- Weiss, A. D. (2001). Topographic position and landforms analysis. Póster, *ESRI International User Conference*, San Diego.
- Wilson, M. F. J., O'Connell, B., Brown, C., Guinan, J. C. y Grehan, A. J. (2007). Multiscale terrain analysis of multibeam bathymetry data for habitat mapping on the continental slope. *Marine Geodesy*, 30(1–2), 3–35.
- Zevenbergen, L. W. y Thorne, C. R. (1987). Quantitative analysis of land surface topography. *Earth Surface Processes and Landforms*, 12(1), 47–56.

### Ráster y vectores

- Douglas, D. H. y Peucker, T. K. (1973). Algorithms for the reduction of the number of points required to represent a digitized line or its caricature. *The Canadian Cartographer*, 10(2), 112–122.
- Felzenszwalb, P. F. y Huttenlocher, D. P. (2012). Distance transforms of sampled functions. *Theory of Computing*, 8, art. 19, 415–428.
- Keys, R. G. (1981). Cubic convolution interpolation for digital image processing. *IEEE Transactions on Acoustics, Speech, and Signal Processing*, 29(6), 1153–1160.
- Ledoux, H., Arroyo Ohori, K. y Meijers, M. (2014). A triangulation-based approach to automatically repair GIS polygons. *Computers & Geosciences*, 66, 121–131.
- Zhang, T. Y. y Suen, C. Y. (1984). A fast parallel algorithm for thinning digital patterns. *Communications of the ACM*, 27(3), 236–239.

### Registro de nubes, estimación de volumen y formatos

- ASPRS (2019). *LAS Specification 1.4 – R15*. American Society for Photogrammetry and Remote Sensing.
- Besl, P. J. y McKay, N. D. (1992). A method for registration of 3-D shapes. *IEEE Transactions on Pattern Analysis and Machine Intelligence*, 14(2), 239–256.
- Chen, Y. y Medioni, G. (1992). Object modelling by registration of multiple range images. *Image and Vision Computing*, 10(3), 145–155.
- Chetverikov, D., Svirko, D., Stepanov, D. y Krsek, P. (2002). The trimmed iterative closest point algorithm. *Proceedings of the 16th International Conference on Pattern Recognition*, 3, 545–548.
- Husch, B., Beers, T. W. y Kershaw, J. A. (2003). *Forest Mensuration* (4.ª ed.). John Wiley & Sons. (Fórmula de Smalian.)
- Isenburg, M. (2013). LASzip: lossless compression of LiDAR data. *Photogrammetric Engineering & Remote Sensing*, 79(2), 209–217.
- Liang, X., Kankare, V., Hyyppä, J., Wang, Y., Kukko, A., Haggrén, H., et al. (2016). Terrestrial laser scanning in forest inventories. *ISPRS Journal of Photogrammetry and Remote Sensing*, 115, 63–77.

### Índices espectrales

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

**HIPSO** · Procesamiento y edición de nubes de puntos.
Autor: Julio M. Sevilla.
Contacto: julio@jsevilla.es · https://es.linkedin.com/in/juliomsevilla

Se agradecen todas las aportaciones y la detección de errores; los errores se irán corrigiendo en la medida de lo posible.

Uso gratuito. Consulta `LICENSE` para las condiciones y `THIRD_PARTY_LICENSES.md` y `NOTICE` para los componentes de terceros.

