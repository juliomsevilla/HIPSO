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
3. `easylidar` es la versión de línea de comandos (capítulo 12).

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

### 6.1 Clases ASPRS y clases nuevas

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
- **Clasificar**: elige lo que quieras.
  - *Ruido*: puntos aislados o filtro estadístico (SOR).
  - *Suelo*: iterativo de Kraus y Pfeifer, curvatura multiescala (MCC), simulación de tela (CSF), morfológico progresivo (PMF) o la clase 2 que ya trae la nube.
  - *Edificios* y vegetación alta.
  - *Vegetación por alturas*: baja (3), media (4) y alta (5).
  - *Cables* (14): líneas casi horizontales altas. *Postes y torres* (15): líneas finas casi verticales.
  - *Teselas y rendimiento*: tamaño de tesela, búfer e hilos.

### 7.2 Superficies

- **MDT**: modelo digital del terreno en GeoTIFF, por media de las celdas o por TIN. Parte de la clase 2 de la nube.
- **CHM**: modelo de altura de copas, con opción *pit-free* y suavizado. Usa el MDT **de esa misma nube**; si no existe, avisa.

### 7.3 Inventario

- **Métricas de rodal**: métricas de altura, cobertura e intensidad por celda, opcionalmente por estratos. Salen como capa vectorial y/o CSV, con la fecha de captura.
- **Segmentar árboles**: detecta cimas y copas sobre el CHM (ventana fija o variable circular o cuadrada, Dalponte, Silva, cuencas), con suavizado, polígonos de copa y evaluación frente a un inventario de campo.

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
4. Sigue cada tronco hacia arriba y hacia abajo, ajustando un círculo a una rebanada horizontal cada pocos centímetros. Una sección vale si tiene puntos suficientes, repartidos alrededor y sin puntos dentro. El centro de la siguiente se predice, así que sigue troncos inclinados o curvos.
5. **Cubica con la fórmula de Smalian**: para cada troza, V = (A₁ + A₂) / 2 · L, con A el área del círculo de cada extremo y L la longitud de la troza. Se empieza en la **altura de la base** (10 cm sobre el suelo por defecto) y se termina donde el diámetro baja al **diámetro en punta delgada** (7 cm por defecto); la última troza se corta en ese punto.
6. La **longitud de la troza** (0,5 m por defecto) es la separación entre los diámetros medidos a lo largo del tronco.

**Resultado:**

- Un shapefile (y/o CSV) con un punto en la base de cada árbol: `DAP_CM`, `ALT_TOT`, `ALT_COM` (altura hasta la punta delgada), `VOL_M3`, `NTROZAS`, `D_BASE_CM`, `D_TOP_CM`, `AB_M2` (área basimétrica), `INCLIN`, `PUNTOS`, `CALIDAD`, `AVISOS` y `Date`.
- `…_trozas.csv`: cada troza de cada árbol, con alturas, diámetros, longitud y volumen.
- Si se marca *Guardar también la nube de cada árbol*: una carpeta `…_arboles` con un archivo por árbol. Cada punto va al eje de tronco más cercano dentro de la distancia indicada.

Avisos: `TRUNCADO` (el tronco se pierde antes de llegar a la punta delgada), `SIN_DAP` (no hay sección válida a 1,3 m), `INTERPOLADO` (se rellenaron huecos), `INCLINADO` (más de 25°) y `SIN_VOLUMEN`.

Consejos: con nubes muy densas, un vóxel de 2 a 3 cm es suficiente; si faltan troncos, baja los vóxeles mínimos de un tronco o la verticalidad mínima; si se cuelan arbustos, súbelos. Una sola posición de escáner solo ve una parte de cada tronco: baja los *sectores ocupados* con cuidado.

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
- Un LAZ que necesita varias pasadas se descomprime una vez a un archivo temporal que se borra al terminar.
- Teselas: tamaño y búfer se ajustan en *Teselas y rendimiento*. La memoria es aproximadamente (núcleos × tesela).
- Para trabajar con lo que está en pantalla, selecciona los puntos: el proceso se aplica solo a ellos.

## 12. Línea de comandos

`easylidar` hace lo mismo que la interfaz, por lotes. Con `--help` se ve la lista completa de comandos y de opciones.

```
easylidar info nube.laz
easylidar thin nube.laz --method density --density 1
easylidar classify nube.laz --ground csf --noise isolated --buildings --vegetation
easylidar dtm nube.laz --cell 1
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

## 13. Atajos

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

## 14. Solución de problemas

- **«No hay MDT de esta nube»**: genera antes el MDT con la herramienta MDT.
- **La nube no se ve completa**: sube *Puntos en pantalla* en el panel *Capas* y pulsa *Aplicar*.
- **Los diámetros de los árboles salen vacíos**: comprueba que la nube tenga suelo (o marca que ya está normalizada) y que la franja de búsqueda esté dentro de la altura de los troncos.
- **El mosaico dice que los sistemas son distintos**: reproyecta los rásteres al mismo sistema antes de unirlos.
- **Un resultado a medias tras detener un proceso**: se borra solo; si queda algún archivo, bórralo a mano.

## 15. Acerca de

**EasyLiDAR** · Procesamiento y edición de nubes LiDAR para forestales.
Autor: Julio M. Sevilla.
Contacto: julio@jsevilla.es · https://es.linkedin.com/in/juliomsevilla

Se agradecen todas las aportaciones y la detección de errores; los errores se irán corrigiendo en la medida de lo posible.

Uso gratuito. Consulta `LICENSE` para las condiciones y `THIRD_PARTY_LICENSES.md` y `NOTICE` para los componentes de terceros.
