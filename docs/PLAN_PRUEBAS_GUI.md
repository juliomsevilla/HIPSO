# Plan de pruebas de la interfaz de HIPSO

Este plan sirve para recorrer la interfaz entera y comprobar dos cosas: que nada falla (ni se cierra, ni se queda colgado, ni pierde datos) y que usarla resulta cómodo (se entiende qué hacer, se ve que algo ocurre y todo se puede deshacer). Cada prueba dice qué hacer y qué debe pasar; lo que no coincida se anota como hallazgo (véase el apartado 9).

## 1. Qué se prueba

- La versión en desarrollo (`0.8.x`) compilada en release, en macOS con Apple Silicon. Windows y Linux no se pueden probar desde este equipo: quedan fuera de este plan y conviene repetir los bloques 1, 2 y 12 en cada uno antes de la release.
- Todas las pestañas de la cinta (*Archivo*, *Vista*, *LiDAR*, *Vectorial*, *Raster*, *Flujos*), el panel lateral, la barra de estado, las vistas (planta, perfil y 3D), los dos docks inferiores (tabla de atributos y EDA) y las ventanas de cada herramienta.
- Las herramientas añadidas en esta versión: reproyectar capas vectoriales, reparar topología, quitar y renombrar campos, simplificar, paralela, capturar coordenadas, ir a coordenadas, mapas base de Sentinel-2, EDA y tablas CSV.

## 2. Preparación

1. Compilar y arrancar. El programa `hipso` (línea de comandos) debe estar en la misma carpeta que `hipso-gui`, porque los flujos y las herramientas genéricas lo ejecutan:

   ```
   cargo build --release -p hipso-gui -p hipso-cli
   target/release/hipso-gui
   ```

2. Crear los datos sintéticos (no hacen falta los discos externos):

   ```
   python3 scripts/datos_prueba_gui.py target/datos_prueba
   ```

   Se escriben `escena.las` (532 000 puntos, EPSG:25830: terreno ondulado, 62 árboles, un edificio de 40 × 16 m a 8 m, dos postes con un cable y ruido), `escena_desplazada.las` (la misma, movida 1,5 m en x, 0,8 m en y y 0,3 m en z), `rodales.geojson` (12 polígonos), `parcelas.geojson` y `parcelas.csv` (30 parcelas con coma decimal), `trazo.geojson` (una línea) y `topologia_rota.geojson` (una ranura, un solape y un hueco). La carpeta `target/datos_prueba/derivados` trae además la escena clasificada, su MDT y su CHM.

3. Datos reales, si el disco externo está montado (`/Volumes/Ext1Tb/EasyLiDAR_data`): una hoja del PNOA (`PNOA/LiDAR`), una ortofoto (`PNOA/Raster`), la parcela de escáner terrestre (`TLS`), los siete COPC de línea eléctrica (`Powerlines`) y los vuelos de dron (`QULTIVA`). Se usan en los bloques que lo piden.

4. Apuntar el equipo (modelo, memoria, resolución y escala de la pantalla) y empezar cada sesión con la configuración de fábrica: apartar la carpeta `HIPSO` de las preferencias del sistema (en macOS y Linux, `~/.config/HIPSO`, que guarda `preferencias.txt`) y restaurarla al terminar.

## 3. Cómo se anota

Cada prueba termina en una de tres marcas: **✔** (cumple), **~** (cumple, pero con algo que mejorar) o **✖** (falla). Los fallos se clasifican así:

| Gravedad | Qué es |
|---|---|
| Crítica | Cierre inesperado, bloqueo sin salida, datos perdidos o corrompidos, resultado numérico equivocado. |
| Alta | Una función no hace lo que dice, un mensaje engaña, o no se puede deshacer algo que debería. |
| Media | Funciona, pero hay que adivinar cómo, tarda sin avisar o el aspecto se descompone. |
| Baja | Texto, alineación, atajo o detalle estético. |

Con cada hallazgo se guarda cómo reproducirlo, el contenido del *Registro* en ese momento y, si es visual, una captura.

## 4. Criterios de experiencia de uso

En todas las pruebas se mira, además del resultado:

- **Respuesta.** Una acción que tarda más de un segundo enseña que está trabajando (progreso, rueda, texto en la barra de estado), y la ventana sigue contestando al ratón.
- **Claridad.** Cada botón tiene un nombre que se entiende y un texto de ayuda al pasar el ratón; cada ventana de herramienta explica qué hace y avisa en naranja de lo que falta.
- **Reversibilidad.** Lo que cambia datos se puede deshacer y rehacer, y lo que escribe archivos nunca pisa uno existente sin decirlo.
- **Mensajes.** Un error dice qué falló y qué hacer; no enseña códigos internos ni se queda en el registro sin avisar.
- **Coherencia.** La misma acción se hace igual en todas partes (selección con Mayús y Alt, cierre con la cruz, guardado en la carpeta de resultados).
- **Esfuerzo.** Se cuentan los clics hasta el primer resultado en cada escenario del apartado 8.

## 5. Bloques de prueba

### Bloque 1 · Marco y arranque

1. Arrancar sin argumentos: la ventana abre en menos de 3 s, con la cinta en *Inicio*, el panel lateral vacío y la barra de estado.
2. Redimensionar de 800 × 500 a pantalla completa y volver: la cinta pliega sus grupos en menús al estrechar y ninguno queda cortado; el panel lateral se estrecha y se ensancha arrastrando su borde.
3. Cambiar entre tema claro y oscuro (*Archivo → Interfaz*): todo cambia a la vez, incluidos los gráficos, la tabla y las ventanas abiertas.
4. *Buscar comando* (Ctrl+K): escribir «histograma», «EDA», «coordenadas», «reproyectar»; cada orden aparece y se ejecuta.
5. F1 abre el manual dentro del programa; la búsqueda y los títulos funcionan.
6. *Inicio → Ajustes*: cambiar el máximo de puntos, la carpeta de la caché y vaciarla; el cambio se conserva al reabrir.
7. Pasar el ratón por todos los botones de las seis pestañas: ninguno queda sin texto de ayuda.
8. Cerrar la ventana con datos sin guardar (una capa editada): debe avisar, no perder.

### Bloque 2 · Abrir, cerrar y guardar

1. Abrir `escena.las` con el botón, con Ctrl+O y arrastrándolo a la ventana: carga con progreso y aparece en la planta, ajustada a la vista.
2. Abrir a la vez `escena.las` y `escena_desplazada.las`: ambas aparecen, y la de arriba prevalece donde se solapan.
3. Abrir `rodales.geojson`, `parcelas.geojson`, `trazo.geojson`, `parcelas.csv` y un GeoTIFF de `derivados`: cada uno se carga como lo que es (la tabla CSV no se dibuja pero abre su tabla).
4. Abrir un archivo que no se lee (un `.txt` cualquiera, un LAS truncado a mitad con `head -c 100000`, un GeoJSON con la llave final borrada): mensaje claro, sin cierre, el resto de capas intactas.
5. *Recientes*: reabrir uno, vaciar la lista.
6. Guardar proyecto, cerrar todo, abrir proyecto: vuelven las capas, el orden, la visibilidad, la vista y la orientación 3D.
7. Mover un archivo del proyecto de sitio y abrir el proyecto: avisa de lo que falta y abre el resto.
8. Rutas con espacios y tildes (`~/Pruebas HIPSO/ñandú.las`): abre y guarda sin problema.
9. *Guardar vista como PNG*: el archivo se abre y coincide con la planta.
10. *Cerrar todo* con un proceso en marcha: no debe dejar procesos huérfanos.

### Bloque 3 · Vistas y navegación

1. Planta: rueda para acercar (hacia el cursor), arrastrar para desplazar, doble clic para encuadrar, *Extensión completa*, *Zoom a la capa* y *Zoom a la selección*.
2. *Zoom anterior* y *Zoom adelante* recorren el historial; el botón se apaga cuando no hay más.
3. Alternar las cuatro ventanas (2D, 2D + perfil, 3D, las tres) y arrastrar las barras de división: se conservan al cambiar.
4. Vista 3D: girar sin tope, desplazar, rueda, doble clic, *Refrescar 3D*; sin saltos ni parpadeos.
5. Perfil: trazar la caja (tres clics), moverla con las flechas y la rueda, cambiar el ancho, ver una nube coloreada por altura; la planta y el perfil comparten el cursor.
6. Colorear por elevación, clase, intensidad, retorno, pasada, archivo, RGB e ID de árbol (este último solo con árboles segmentados): el cambio es inmediato y la leyenda coincide.
7. Filtro por cota («Dibujar solo entre dos cotas»): oculta en las tres vistas.
8. *Consultar* (Q) un punto de suelo, uno de copa y uno del cable: la ficha dice la clase y la cota correctas; en el perfil y en el 3D también.
9. *Medir* (W): distancia, desnivel y superficie de un polígono; botón derecho borra la medición.
10. *Densidad*: un rectángulo da puntos/m² coherentes con *Información de los datos* (13,3 puntos/m² en la escena sintética).
11. TIN sombreado (T) y curvas de nivel: aparecen sobre la planta y se pueden quitar.
12. **Capturar coordenadas**: con la herramienta de la cinta, cada clic anota un punto numerado y copia «X Y» al portapapeles; en la ventana, cambiar a EPSG 4326 convierte lo anotado; *Copiar todo* pega una tabla en una hoja de cálculo; *Vaciar* borra las marcas.
13. **Ir a coordenadas** (barra de estado): escribir `440100 4470100`; centrar sin cambiar la ampliación y marcar el punto. Repetir con `-3,70 40,41` y EPSG 4326 (debe caer en Madrid, fuera de la escena, y avisar o centrar allí). Con X vacía, con letras y con un EPSG inexistente: mensaje claro.
14. Mapa base: encender y probar OpenStreetMap, PNOA, IGN base, **Sentinel-2 color natural reciente** y **Sentinel-2 infrarrojo color reciente**. Deben mostrar «preparando el mosaico…» y después teselas; con la red cortada, un mensaje que no bloquee. El crédito aparece abajo a la derecha y lo que se dibuja encima sigue visible.
15. Una capa en otro sistema que la nube (por ejemplo `rodales` reproyectado a 4326): se convierte al cargar y el registro lo cuenta.

### Bloque 4 · Edición de nubes de puntos

Con `escena.las`:

1. Selección con rectángulo, círculo, polígono, lazo, pincel redondo y cuadrado, en planta, en perfil y en 3D (con rebanada de profundidad). El contador de seleccionados coincide con lo que se ve.
2. Modos de la cinta: nueva, añadir, quitar, intersectar e invertir, y con Mayús/Alt en las herramientas que los admiten.
3. Seleccionar sobre, bajo y cerca de una línea del perfil; clasificar a lo largo de una línea de una vez.
4. **Varita mágica**: pulsar el tejado del edificio selecciona el tejado y no el suelo; probar en un árbol (copa conectada) y en el cable.
5. Seleccionar con plano local sobre una zona de suelo.
6. Histogramas de atributos: marcar barras con clic y arrastre, *Marcar los raros* con cada estadístico, *Seleccionar puntos*; los 300 puntos altos y 200 bajos de ruido deben salir como atípicos de Z.
7. Reclasificar la selección a la clase 7 y comprobar el color; borrar; *Retener* (withheld) en lugar de borrar.
8. Deshacer y rehacer cada una de las acciones anteriores varias veces seguidas; el contador y el dibujo vuelven al estado exacto.
9. *Aislar selección* de un árbol: se ve con toda su densidad y se puede volver.
10. Clases ASPRS: añadir una clase propia (64–255) con nombre y color; aparece en la leyenda y en los histogramas.
11. Guardar las ediciones en un archivo nuevo y aplicarlas a otra nube (`escena_desplazada.las`): el resultado se abre y la clase cambiada está donde se esperaba.
12. *Propagar clase* y *Editar con capas* con `rodales.geojson`.
13. Salir con ediciones sin guardar: avisa.

### Bloque 5 · LiDAR

Con `escena.las` salvo que se diga otra cosa. En cada herramienta: abrir la ventana, leer el párrafo de ayuda, ejecutar con los valores por defecto, comprobar que el resultado se abre solo en el mapa, que el *Registro* cuenta qué hizo y que *Guardar en* lleva el producto a la carpeta elegida.

1. **Preparar.** *Información de los datos* (comparar con la salida de `hipso info`); *Asignar proyección* a un LAS sin sistema (copiar uno con la cabecera sin VLR); *Adelgazar nube*; **Clasificar** con suelo CSF, con otro método de suelo, con edificios, con vegetación, con ruido, con cables y postes, y con una receta guardada y cargada; *Comparar nubes*, *Ajustar nubes* (ICP) con `escena_desplazada.las` sobre `escena.las` (el desplazamiento recuperado debe rondar 1,5 / 0,8 / 0,3 m); *Entre pasadas*, *Líneas de vuelo*, *Densidad de Voronoi*, *Colorear con imagen*, *Normalizar intensidad*, *Imagen de intensidad*, *Exportar puntos* (LAS, LAZ y COPC), *Propagar clase* y *Editar con capas*.
2. **Superficies.** MDT (TIN, ponderado, media por celda, con líneas de rotura), MDS, CHM con y sin depresiones, suavizado, contorno de los rásteres, curvas de nivel.
3. **Terreno e hidrología.** Análisis de terreno (relieve, formas, crestas y drenaje con Strahler), zonas encharcadas, inundación, ruta de menor coste sobre el MDT, volumen sobre MDT con un polígono, cubicación de pilas, desmonte y terraplén entre `escena` y `escena_desplazada`.
4. **ALS e inventario.** Métricas por celda, extraer muestras de `parcelas.geojson`, segmentar individuos (comparar el recuento con los 62 árboles sintéticos), OBIA, cambio de la cubierta vegetal, modelos lineales sobre las parcelas (diagnóstico de residuos), fecha de captura y borde en los polígonos.
5. **TLS** (solo con la parcela real): medición de troncos.
6. **Infraestructuras.** Corredores eléctricos con el cable sintético clasificado y con los COPC reales.
7. Revisar que ninguna ventana de proceso se desborda con la ventana pequeña y que las que se pueden cancelar se paran de verdad (*Detener* borra lo que haya empezado).
8. **Para investigar.** En la escena sintética, el edificio de 40 × 16 m con tejado plano a 8 m no sale como clase 6 con `--ground csf --buildings` (queda como vegetación alta). Hay que repetirlo con una hoja real con edificios antes de darlo por fallo o por límite de la escena de prueba.

### Bloque 6 · Capas vectoriales

1. **Crear y digitalizar.** *Nueva capa* de puntos, líneas y polígonos con campos de cada tipo; digitalizar con ajuste a vértices y a bordes; mover vértices (arrastrar, doble clic en un lado para insertar, botón derecho para quitar); mover elementos; adosar y cortar polígonos con un trazo; *Reformar*.
2. **Selección.** Rectángulo, por localización, por expresión (`"altura" > 12 and $area > 5000`), por atributos desde la tabla; invertir, limpiar, zoom a la selección.
3. **Geometría y atributos.** Unir seleccionados, separar partes, agrupar en multiparte, transformar (desplazar, girar, escalar), copiar, pegar y pegar como capa, eliminar.
4. **Tabla.** Abrir; ordenar; filtrar; solo seleccionados; editar celdas con valores válidos y no válidos (fecha mal escrita, texto en un entero); cambiar el tipo de un campo; **renombrar** un campo (repetido y vacío deben rechazarse); **eliminar** un campo usado por la simbología o las etiquetas y comprobar que la capa sigue dibujándose; deshacer cada cambio.
5. **Calculadora de campos.** Probar, calcular y calcular solo los seleccionados con `round("altura" * 1.1, 1)`, `$area / 10000` y texto con `concat`.
6. **Topología.** *Comprobar topología* sobre `topologia_rota.geojson`: debe listar el solape, la ranura y el hueco. **Reparar topología** con ajuste 0,5 y hueco hasta 10: las ranuras y los huecos desaparecen, el solape se queda con el polígono mayor, los atributos se conservan y *Deshacer* devuelve lo anterior. Repetir con `trazo.geojson` y un extremo suelto.
7. **Geoprocesos.** Comprobar y reparar validez, búfer, disolver, unión, intersección, diferencia, diferencia simétrica, centroides, puntos en superficie, unir por localización, Voronoi, estadística zonal, extraer valores, unir tabla, combinar, dividir por campo, recortar, cercanía.
8. **Nuevos.** *Simplificar* `rodales.geojson` con tolerancia 5 (los polígonos vecinos siguen compartiendo borde y no aparecen huecos); *Paralela* de `trazo.geojson` a 10 y a −10 m (con una esquina cerrada y otra abierta); *Quitar y renombrar campos*; *Reproyectar* `rodales.geojson` a 4326, a 25829 y a Web Mercator (3857), y volver a 25830 (diferencia menor de 1 mm); un KML solo admite 4326 y debe decirlo.
9. **Guardar.** En shapefile, GeoPackage y GeoJSON: reabrir y comparar geometría, campos y tipos. Un nombre de campo de más de 10 caracteres en shapefile avisa.
10. **Campos en listas.** En cada ventana de herramienta que pide un campo o una columna (*Dividir por campo*, *Unir tabla*, *Quitar y renombrar campos*, *Cercanía*, *Rasterizar*, *Disolver*, el campo de nombre de *Cambio de la cubierta* y el de clase de *OBIA*), el campo se elige de una lista con los de la tabla, sin escribirlo: la ventana se abre con la capa activa ya elegida; al cambiar de capa la lista cambia; con la capa sin elegir, la lista está apagada y dice qué falta; con un archivo ilegible vuelve el cuadro de texto con un aviso. *Quitar y renombrar campos* admite varios campos con casillas y cada renombrado como una fila (campo → nombre nuevo); quitar y renombrar el mismo campo se rechaza con un mensaje que lo dice. Las listas de las herramientas que piden solo números (*Rasterizar*, el modelo lineal) no ofrecen los campos de texto. Lo mismo en el inspector de los nodos del constructor de flujos cuando el archivo es un nodo de entrada.
11. **Cinta Vectorial.** Los grupos van en este orden: *Capa*, *Geometría* (validez, reparar geometrías y topología, simplificar, paralela y recortar capa), *Tabla* (extraer valores, unir tabla, dividir por campo y quitar o renombrar campos) y *Geoprocesos* (búfer, disolver, unión, intersección, diferencia, centroides, unir por localización, Voronoi, estadística zonal, combinar capas y cercanía). Ninguna herramienta aparece en dos grupos y las ventanas se abren desde su botón.
12. Capas grandes: abrir un shapefile de decenas de miles de elementos (el de rodales real) y comprobar que se dibuja como imagen y se consulta con fluidez.

### Bloque 7 · Ráster

Con el MDT y el CHM de `derivados` y, si hay, la ortofoto real:

1. Abrir; simbología (tipo de estiramiento, rampa de color, sombreado); superficie en 3D con exageración vertical; perfil sobre el ráster.
2. Imagen multibanda: composición de bandas y los índices espectrales (con la ortofoto o el Sentinel-2 real).
3. Reproyectar a otro EPSG, igualar rásteres, crear multibanda, mosaico con cada regla de solape, escribir COG.
4. Calculadora ráster con `if(A > 0, A - B, 0)` y una expresión mal escrita (mensaje claro).
5. Reclasificar, cortar por máscara, tamizar, proximidad, rellenar huecos, filtro focal, tabular áreas, rasterizar una capa, vectorizar un ráster.
6. Ráster grande: la ortofoto real de 550 MB abre sin congelar la ventana.

### Bloque 8 · EDA

1. Abrir `rodales.geojson` y pulsar **EDA** (*Archivo → Análisis*): el dock aparece encima de la tabla si está abierta, con un histograma, una dispersión y unas cajas; las columnas por defecto no son el identificador.
2. Arrastrar sobre el histograma: se seleccionan los rodales en el mapa y en la tabla; con Mayús se añaden y con Alt se quitan; un clic en el fondo de la dispersión limpia.
3. Rectángulo en la dispersión; clic en una caja (agrupar por especie); arrastre vertical dentro de una caja.
4. Seleccionar en el mapa o en la tabla: los tres gráficos pintan en naranja lo seleccionado, y las cajas muestran la caja de lo seleccionado junto a la de todos.
5. Cambiar de columna X e Y, de agrupación, el número de barras y el recuento logarítmico.
6. Pestaña *Estadísticas*: sin grupos, agrupadas por especie y con *Solo los seleccionados*; *Copiar* pega en una hoja de cálculo; *Exportar CSV* escribe un archivo que se abre bien con punto decimal.
7. **Guardar imagen**: cada gráfico y los tres juntos, desde el botón y desde el botón derecho; el PNG es nítido, tiene el tamaño del gráfico y lleva la selección en naranja. Probar con el tema oscuro.
8. Abrir `parcelas.csv`: se carga como tabla (sin dibujarse), las columnas con coma decimal son numéricas, y el EDA funciona igual.
9. Fuente *Nube de puntos*: histograma de Z, dispersión Z frente a intensidad con rectángulo (debe encolar una regla que se ve en la selección de la nube y vale para guardar las ediciones) y cajas de Z por clase.
10. Fuente ráster (el CHM): histograma y caja; sin selección, con el aviso.
11. Con 8 millones de puntos (`Ajustes`: subir el máximo a 8 M con una hoja real): abrir el EDA y notar cuánto tarda; debe estar por debajo de un segundo.
12. Cerrar y reabrir el EDA, cambiar de capa a mitad, quitar la capa que se está graficando: sin fallos.

### Bloque 9 · Flujos

1. Crear un flujo nuevo, añadir nodos desde la paleta, conectar con hilos, ver los problemas en vivo (un hilo entre tipos incompatibles se rechaza con un motivo).
2. Un inventario completo (clasificar, MDT, CHM, métricas): ejecutar entero y por nodo, parar a mitad (debe limpiar lo que dejó), guardar, cerrar y abrir.
3. Deshacer y rehacer en el lienzo; copiar nodos; borrar un nodo con hilos.
4. Los nodos de las herramientas nuevas (`fix-topology`, `simplify`, `parallel`, `fields`) aparecen y se pueden conectar a una capa.
5. Un flujo guardado antes de esta versión (con `summary` o `rstats`) sigue abriendo y ejecutando.

### Bloque 10 · Panel lateral, registro y barra de estado

1. *Contenido*: selección múltiple de capas (Ctrl/Cmd y Mayús), mostrar y ocultar, subir y bajar arrastrando, quitar, zoom a la capa, buscar.
2. *Información*: analizar de nuevo y *Copiar todo*.
3. *Edición*: cambia según la capa elegida (nube, vector, ráster sin herramientas con su aviso).
4. *Simbología*: color único, por categorías, graduado, proporcional; etiquetas; cambiar un campo y que se actualice.
5. *Registro*: se abre y se cierra, el texto se puede seleccionar y copiar, no crece sin límite.
6. Barra de estado: coordenadas, sistema, número de puntos, herramienta, escala, progreso y *Detener* aparecen y desaparecen con sentido.

### Bloque 11 · Robustez

1. Cancelar cada tipo de proceso largo (clasificar, MDT, exportar) a los 2 s: el programa vuelve a su estado y no deja archivos a medias.
2. Lanzar una herramienta con otra en marcha: o se encola o avisa; nunca se corrompe.
3. Cerrar la ventana con un proceso en marcha y con un diálogo de archivo abierto.
4. Una entrada con valores imposibles: tolerancias negativas, tamaños de celda 0, distancia 0, texto en un campo numérico; la ventana explica el problema antes de ejecutar.
2 bis. **Procesos a la vez.** Las herramientas de las ventanas genéricas (cinta Vectorial y Raster: *Unir tabla*, *Dividir por campo*, *Reparar topología*, *Simplificar*, *Filtro focal*…) pueden estar varias abiertas a la vez y ejecutándose a la vez, cada una con su ventana: abrir otra no cierra la primera, la ventana de la que corre enseña «Ejecutando… N s» con la última línea que dijo, la barra de estado enseña cada una con su tiempo y un botón ■ que detiene solo esa, y al terminar cada ventana dice cómo acabó. Las herramientas con ventana propia (clasificar, MDT, terreno…) corren de una en una: mientras una corre, el botón de las demás está apagado y la barra de estado enseña cuál es; sí pueden correr junto a una de las genéricas.
5. Un archivo de salida que ya existe: no se pisa sin decirlo.
6. Disco de salida de solo lectura, o carpeta de resultados borrada a mitad: mensaje claro.
7. Dos instancias del programa a la vez sobre los mismos datos.
8. Una sesión larga (una hora de uso variado): la memoria no crece sin parar (Monitor de Actividad) y el rendimiento no se degrada.
9. Reducir la ventana mientras se calcula, minimizarla y recuperarla, cambiar de monitor o de escala de pantalla.
10. Teclado con distribución española: atajos con tildes y teclas muertas no se pierden; Ctrl y Cmd según el sistema.

### Bloque 12 · Rendimiento

Anotar los tiempos con la escena sintética y, si se puede, con una hoja real del PNOA y con una parcela de escáner terrestre:

| Acción | Escena sintética | Hoja PNOA |
|---|---|---|
| Abrir la nube hasta verla | | |
| Imágenes por segundo al girar en 3D (barra de estado) | | |
| Clasificar con suelo y edificios | | |
| MDT y CHM | | |
| Seleccionar con rectángulo en planta | | |
| Abrir el EDA y pasar a *Estadísticas* | | |
| Memoria máxima | | |

## 6. Datos que cubren cada bloque

| Bloque | Sintéticos | Reales (si hay disco) |
|---|---|---|
| 2 a 4 | `escena.las`, `escena_desplazada.las` | hoja PNOA |
| 5 | `escena.las`, `parcelas.geojson` | PNOA, TLS, COPC de líneas |
| 6 | `rodales`, `parcelas`, `trazo`, `topologia_rota` | `Vectorial/rodales.shp` |
| 7 | MDT y CHM de `derivados` | ortofotos, Sentinel-2 |
| 8 | `rodales`, `parcelas.csv`, `escena.las` | hoja PNOA |
| 9 a 12 | los anteriores | los anteriores |

## 7. Pruebas automáticas que acompañan

Antes de cada sesión manual y después de arreglar cualquier hallazgo: `cargo test -p hipso-core -p hipso-cli -p hipso-gui` debe pasar entero (a fecha de este plan, más de quinientas pruebas del núcleo y cincuenta de la interfaz, además de las de la línea de comandos). Lo que se descubra a mano y se pueda automatizar (una selección que no cuadra, un cálculo, un diálogo) se añade a `apps/gui/src/ui_tests.rs` siguiendo el patrón de las existentes (eventos de puntero sin ventana).

Los interruptores ocultos sirven para capturas repetibles de regresión: `--window-png ruta` guarda la ventana entera como PNG pasados unos segundos (sin que salga nada más que HIPSO), `--eda-png ruta` guarda los tres gráficos del EDA, `--action nombre` abre una ventana de herramienta (por ejemplo, `--action clitool-fields`) y `--charts`, `--table`, `--histogram` y `--vsel-all` abren cada pieza en el arranque.

## 7 bis. Batería automática con datos reales

`python3 scripts/bateria_gui.py` (con el disco de datos montado) lanza la interfaz una vez por escenario (nubes de hasta 6 GB, COPC, capas de medio millón de elementos, GeoPackage de 431 MB, ortofotos, las seis pestañas de la cinta y todas las ventanas de herramientas), guarda la ventana entera como PNG con `--window-png`, vigila que no se cierre ni entre en pánico y anota tiempo y memoria; después ejecuta con `hipso` las mismas herramientas que lanzan las ventanas. El informe queda en `target/bateria/informe.md` y las capturas en `target/bateria`. Con `--solo gui`, `--solo cli` o `--filtro texto,texto` se repite solo una parte. Las capturas se revisan a ojo: que un escenario salga «OK» solo dice que no falló, no que se vea bien.

## 8. Escenarios de extremo a extremo

Cada escenario se hace seguido, como lo haría una persona, y se anotan los clics, los tiempos y cualquier momento de duda.

1. **Inventario de una hoja.** Abrir la nube, comprobar su información, clasificar, hacer MDT y CHM, métricas por celda y segmentar individuos; abrir las parcelas, extraer muestras y ajustar un modelo lineal; mirar los residuos en el EDA. Debe poder hacerse sin leer el manual.
2. **Revisar y limpiar una nube.** Detectar ruido con el histograma, seleccionarlo, borrarlo, reclasificar un tejado con la varita, deshacer dos pasos, guardar las ediciones y aplicarlas a otra nube.
3. **Reparar cartografía de rodales.** Abrir `topologia_rota.geojson`, reparar la topología, simplificar, reproyectar a 4326 y guardar en GeoPackage; abrir el resultado y comparar.
4. **EDA de parcelas.** Cargar `parcelas.csv`, ver la relación entre volumen y área basal, seleccionar las parcelas atípicas con un rectángulo, guardar la imagen y exportar sus estadísticas.
5. **Línea eléctrica.** Clasificar los COPC reales, medir la vegetación que invade con el corredor y revisar un vano en el perfil.
6. **Proceso repetible.** Construir un flujo con el inventario, guardarlo y ejecutarlo sobre otra hoja.

## 9. Registro de hallazgos

| N.º | Bloque | Qué pasó | Cómo reproducirlo | Gravedad | Estado |
|---|---|---|---|---|---|
| 1 | 5.8 | En `escena.las` el edificio de 40 × 16 m a 8 m sale como vegetación alta con `--ground csf --buildings` | `hipso classify escena.las --ground csf --buildings` | por determinar | por investigar con datos reales |
| | | | | | |

## 10. Cierre

La interfaz se da por probada cuando los doce bloques y los seis escenarios están recorridos, no queda ningún hallazgo crítico ni alto abierto, y los medios y bajos están anotados con su decisión (arreglar o aceptar). Conviene repetir los bloques 1, 2 y 12 en Windows y en Linux antes de publicar.
