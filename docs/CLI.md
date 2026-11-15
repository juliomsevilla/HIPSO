# Órdenes de `hipso`

Cada herramienta del programa es una orden de la línea de comandos, y la interfaz gráfica y el constructor de flujos ejecutan estas mismas órdenes. Esta página se genera con `scripts/gen_cli_docs.py` a partir de la ayuda del propio programa, de modo que lista exactamente lo que admite la versión publicada.

## Revisar y preparar nubes

### hipso info

Informe completo de archivos LAS/LAZ: cabecera, clases, retornos, densidad, avisos

```
Informe completo de archivos LAS/LAZ: cabecera, clases, retornos, densidad, avisos

Uso: hipso info [OPTIONS] <INPUTS>...

Argumentos:
  <INPUTS>...  

Opciones:
      --results-dir <RESULTS_DIR>  Envía a esta carpeta todo producto al que no se dé una salida explícita, en lugar de la carpeta de cada entrada
```

### hipso catalog

Revisa una entrega de hojas solo con sus cabeceras (no se lee ningún punto): mismo sistema de coordenadas, hojas repetidas, solapes, huecos dentro del bloque. Una carpeta equivale a sus LAS/LAZ

```
Revisa una entrega de hojas solo con sus cabeceras (no se lee ningún punto): mismo sistema de coordenadas, hojas repetidas, solapes, huecos dentro del bloque. Una carpeta equivale a sus LAS/LAZ

Uso: hipso catalog [OPTIONS] <INPUTS>...

Argumentos:
  <INPUTS>...  

Opciones:
      --results-dir <RESULTS_DIR>  Envía a esta carpeta todo producto al que no se dé una salida explícita, en lugar de la carpeta de cada entrada
      --tolerance <TOLERANCE>      Cuánto (m) pueden solaparse las hojas o quedarse cortas unas de otras sin que cuente [default: 1]
```

### hipso crs

Da a las nubes sin sistema de coordenadas el de otra nube o un código EPSG

```
Da a las nubes sin sistema de coordenadas el de otra nube o un código EPSG

Uso: hipso crs [OPTIONS] <INPUTS>...

Argumentos:
  <INPUTS>...  

Opciones:
      --from <FROM>                Una nube que tiene el sistema de coordenadas que copiar
      --results-dir <RESULTS_DIR>  Envía a esta carpeta todo producto al que no se dé una salida explícita, en lugar de la carpeta de cada entrada
      --epsg <EPSG>                EPSG code, e.g. 25830
      --replace                    Sustituye el sistema de coordenadas de nubes que ya tienen uno
  -o, --output <OUTPUT>            Salida (solo una entrada); por defecto: <resultados>/<entrada>_crs.<ext>
```

### hipso import

Importa una nube de puntos de otro formato como LAS/LAZ: PLY (ASCII o binario) o E57 (escáneres terrestres y móviles; cada escaneo se coloca con su pose y su número queda como identificador de fuente del punto)

```
Importa una nube de puntos de otro formato como LAS/LAZ: PLY (ASCII o binario) o E57 (escáneres terrestres y móviles; cada escaneo se coloca con su pose y su número queda como identificador de fuente del punto)

Uso: hipso import [OPTIONS] <INPUT>

Argumentos:
  <INPUT>  El archivo que convertir

Opciones:
  -o, --output <OUTPUT>            .las o .laz de salida; por defecto: <resultados>/<entrada>_ply.laz o _e57.laz (nunca un archivo existente)
      --results-dir <RESULTS_DIR>  Envía a esta carpeta todo producto al que no se dé una salida explícita, en lugar de la carpeta de cada entrada
      --epsg <EPSG>                Código EPSG del sistema de coordenadas de los puntos (PLY y E57 rara vez lo dicen)
```

### hipso dedup

Quita los puntos duplicados (mismas x, y, z y los atributos pedidos)

```
Quita los puntos duplicados (mismas x, y, z y los atributos pedidos)

Uso: hipso dedup [OPTIONS] <INPUTS>...

Argumentos:
  <INPUTS>...  

Opciones:
  -o, --output <OUTPUT>            
      --results-dir <RESULTS_DIR>  Envía a esta carpeta todo producto al que no se dé una salida explícita, en lugar de la carpeta de cada entrada
      --all-copies                 Borra todas las copias, no solo las sobrantes
      --also <ALSO>                Atributos que también deben coincidir: class, gps, intensity, return, count, source
```

### hipso tile

Divide las nubes en teselas cuadradas (con un margen opcional), un archivo por tesela

```
Divide las nubes en teselas cuadradas (con un margen opcional), un archivo por tesela

Uso: hipso tile [OPTIONS] <INPUTS>...

Argumentos:
  <INPUTS>...  

Opciones:
  -o, --output <OUTPUT>            Nombre base de las teselas; por defecto: <resultados>/<entrada>_tile.<ext>
      --results-dir <RESULTS_DIR>  Envía a esta carpeta todo producto al que no se dé una salida explícita, en lugar de la carpeta de cada entrada
      --size <SIZE>                [default: 1000]
      --buffer <BUFFER>            [default: 0]
```

### hipso clip

Recorta nubes con polígonos, parcelas o rectángulos

```
Recorta nubes con polígonos, parcelas o rectángulos

Uso: hipso clip [OPTIONS] <INPUTS>...

Argumentos:
  <INPUTS>...
          

Opciones:
      --regions <REGIONS>
          Polígonos (.shp), centros de parcelas (.shp de puntos, con --radius) o una lista (.csv/.txt)

      --results-dir <RESULTS_DIR>
          Envía a esta carpeta todo producto al que no se dé una salida explícita, en lugar de la carpeta de cada entrada

      --rect <RECT>
          Un rectángulo: minx,miny,maxx,maxy (se repite para varios)

      --circle <CIRCLE>
          Una parcela circular: x,y,radio (se repite para varias)

      --name-field <NAME_FIELD>
          El atributo que da nombre a cada región: el nombre de un campo o su posición contando desde 1

      --buffer <BUFFER>
          Amplía cada región esta distancia en unidades del mapa (metros en un sistema proyectado); si es negativa, la reduce. Las esquinas quedan redondeadas

      --split-parts
          Divide los elementos de varias partes en una región por parte (llamadas <nombre>_p1, <nombre>_p2…)

      --radius <RADIUS>
          Radio de las parcelas dadas como puntos

      --radius-field <RADIUS_FIELD>
          Atributo que guarda el radio propio de cada punto (metros)

      --square
          Las parcelas dadas como puntos son cuadradas en lugar de circulares

      --side <SIDE>
          Lado de las parcelas cuadradas, en metros (o los campos que guardan los lados)

      --side-x-field <SIDE_X_FIELD>
          

      --side-y-field <SIDE_Y_FIELD>
          

      --sample <SAMPLE>
          Genera las parcelas en lugar de leerlas: al azar o en una cuadrícula regular
          
          [possible values: random, systematic]

      --count <COUNT>
          Muestreo aleatorio: número de parcelas
          
          [default: 10]

      --spacing <SPACING>
          Muestreo sistemático: separación entre centros en metros, X o X,Y

      --min-dist <MIN_DIST>
          Muestreo aleatorio: distancia mínima entre centros (por defecto: el doble del tamaño de la parcela)

      --seed <SEED>
          Semilla del muestreo aleatorio: la misma semilla da las mismas parcelas
          
          [default: 1]

      --limit <LIMIT>
          Generated plots stay inside these polygons (a stand)

      --layout <LAYOUT>
          Dónde se guarda la capa con las parcelas; por defecto: junto a los resultados

      --where <WHERE>
          Conserva solo las regiones cuyo atributo tiene un valor: CAMPO=VALOR

      --mode <MODE>
          Possible values:
          - merge:   Una nube con los puntos de todas las unidades de muestreo
          - each:    Una nube por unidad de muestreo
          - metrics: Sin nubes: solo la capa de unidades de muestreo con sus métricas
          
          [default: merge]

  -o, --output <OUTPUT>
          Archivo de salida (un nombre base con --mode each); por defecto: junto a los resultados

      --class <CLASS>
          Clases que se conservan: 2,5 o ~7,18 para todas menos esas

      --returns <RETURNS>
          Retornos que se conservan: A, F, L o números (p. ej. 1,2)

      --zmin <ZMIN>
          

      --zmax <ZMAX>
          

      --dtm <DTM>
          Modelo del suelo: escribe alturas sobre el suelo en lugar de elevaciones

      --z-offset <Z_OFFSET>
          Se suma a cada elevación escrita
          
          [default: 0]

      --decimate <DECIMATE>
          Conserva un punto de cada tantos
          
          [default: 1]

      --exclusive
          Regiones que se solapan: un punto va solo a la primera

      --keep-empty
          Escribe archivos para las regiones sin puntos

      --max-open <MAX_OPEN>
          Archivos abiertos a la vez con --mode each
          
          [default: 64]

      --metrics <METRICS>
          Escribe además las CloudMetrics de cada región en este CSV

      --above <ABOVE>
          Métricas: altura de corte para la cobertura
          
          [default: 2]

      --metrics-format <METRICS_FORMAT>
          Formato de la tabla de métricas: shapefile de las regiones, tabla de texto o ambos

          Possible values:
          - vector: Solo shapefile
          - csv:    Solo tabla de texto (lo mejor para ejecuciones muy grandes)
          - both:   Both

      --minht <MINHT>
          Métricas: solo los retornos por encima de esta altura
          
          [default: 0]

      --metrics-columns <METRICS_COLUMNS>
          Métricas: las columnas que se escriben, por nombre (`Elev P90,Elev mean,Int mean`); por defecto: todas

  -h, --help
```

### hipso filter

Une, filtra, normaliza o convierte nubes en una pasada (LAS o LAZ según la extensión de la salida)

```
Une, filtra, normaliza o convierte nubes en una pasada (LAS o LAZ según la extensión de la salida)

Uso: hipso filter [OPTIONS] <INPUTS>...

Argumentos:
  <INPUTS>...  

Opciones:
  -o, --output <OUTPUT>            Output; default: <results>/<input>_filtrada.<ext>
      --results-dir <RESULTS_DIR>  Envía a esta carpeta todo producto al que no se dé una salida explícita, en lugar de la carpeta de cada entrada
      --class <CLASS>              Clases que se conservan: 2,5 o ~7,18 para todas menos esas
      --returns <RETURNS>          Retornos que se conservan: A, F, L o números (p. ej. 1,2)
      --zmin <ZMIN>                
      --zmax <ZMAX>                
      --decimate <DECIMATE>        Conserva un punto de cada tantos [default: 1]
      --dtm <DTM>                  Escribe alturas sobre este modelo del suelo en lugar de elevaciones
      --z-offset <Z_OFFSET>        Se suma a cada elevación escrita [default: 0]
```

### hipso thin

Adelgaza (diezma) una nube: por celda, por celda 3D, un punto de cada n, al azar o hasta una densidad. Cada entrada tiene su propio archivo adelgazado salvo que se dé -o (entonces se unen)

```
Adelgaza (diezma) una nube: por celda, por celda 3D, un punto de cada n, al azar o hasta una densidad. Cada entrada tiene su propio archivo adelgazado salvo que se dé -o (entonces se unen)

Uso: hipso thin [OPTIONS] <INPUTS>...

Argumentos:
  <INPUTS>...
          

Opciones:
  -o, --output <OUTPUT>
          

      --results-dir <RESULTS_DIR>
          Envía a esta carpeta todo producto al que no se dé una salida explícita, en lugar de la carpeta de cada entrada

      --method <METHOD>
          Possible values:
          - lowest:     Punto más bajo de cada celda
          - highest:    Punto más alto de cada celda
          - nearest:    Punto más cercano al centro de cada celda
          - percentile: Punto en el percentil --percentile de las elevaciones de cada celda
          - voxel:      Más cercano al centro de cada celda 3D (--vcell high)
          - every:      Uno de cada n puntos (--n)
          - remove:     Quita uno de cada n puntos (--n)
          - random:     Random sample (--fraction)
          - density:    Muestra aleatoria hasta --density puntos por m²
          
          [default: lowest]

      --cell <CELL>
          Tamaño de celda en metros (métodos por celda)
          
          [default: 1]

      --vcell <VCELL>
          Altura de la celda 3D en metros (vóxel)
          
          [default: 1]

      --percentile <PERCENTILE>
          Percentile 0-100 (percentile; 50 = median)
          
          [default: 50]

      --n <N>
          n de cada / quitar
          
          [default: 2]

      --fraction <FRACTION>
          Proporción de puntos que se conservan, de 0 a 1 (aleatorio)
          
          [default: 0.5]

      --density <DENSITY>
          Puntos por m² que se dejan (densidad)
          
          [default: 1]

      --seed <SEED>
          Semilla de los métodos aleatorios
          
          [default: 1]

      --class <CLASS>
          Clases que se conservan: 2,5 o ~7,18 para todas menos esas

      --returns <RETURNS>
          Retornos que se conservan: A, F, L o números (p. ej. 1,2)

      --zmin <ZMIN>
          

      --zmax <ZMAX>
          

      --decimate <DECIMATE>
          Conserva un punto de cada tantos
          
          [default: 1]

  -h, --help
```

### hipso edit

Edita puntos como si fueran elementos: borra o reclasifica los puntos dentro de polígonos que cumplen las pruebas de atributos; escribe un archivo nuevo, nunca el original

```
Edita puntos como si fueran elementos: borra o reclasifica los puntos dentro de polígonos que cumplen las pruebas de atributos; escribe un archivo nuevo, nunca el original

Uso: hipso edit [OPTIONS] <INPUT>

Argumentos:
  <INPUT>  

Opciones:
  -o, --output <OUTPUT>            
      --results-dir <RESULTS_DIR>  Envía a esta carpeta todo producto al que no se dé una salida explícita, en lugar de la carpeta de cada entrada
      --regions <REGIONS>          Polígonos, centros de parcelas (con --radius) o una lista; por defecto: toda la nube
      --radius <RADIUS>            
      --outside                    Selecciona en cambio los puntos fuera de las regiones
      --class <CLASS>              Solo se seleccionan los puntos de estas clases (2,5 o ~7,18)
      --zmin <ZMIN>                
      --zmax <ZMAX>                
      --delete                     Borra los puntos seleccionados
      --set-class <SET_CLASS>      Cambia los puntos seleccionados a esta clase
```

### hipso colorize

Colorea los puntos a partir de una imagen del mismo lugar (una ortofoto): cada punto toma el rojo, el verde y el azul de su píxel. Los formatos sin color se escriben como formato 7

```
Colorea los puntos a partir de una imagen del mismo lugar (una ortofoto): cada punto toma el rojo, el verde y el azul de su píxel. Los formatos sin color se escriben como formato 7

Uso: hipso colorize [OPTIONS] --image <IMAGE> <INPUTS>...

Argumentos:
  <INPUTS>...  

Opciones:
      --image <IMAGE>                  La imagen (GeoTIFF, JPEG 2000…), en el sistema de coordenadas de las nubes
      --results-dir <RESULTS_DIR>      Envía a esta carpeta todo producto al que no se dé una salida explícita, en lugar de la carpeta de cada entrada
      --bands <BANDS> <BANDS> <BANDS>  Bandas del rojo, el verde y el azul, desde 1 [default: 1 2 3]
      --nir <NIR>                      Banda del infrarrojo cercano, que se escribe en los formatos 8 y 10
      --keep-outside                   Los puntos fuera de la imagen conservan su color (por defecto: negro)
  -o, --output <OUTPUT>                Salida (solo una entrada); por defecto: <resultados>/<entrada>_rgb.<ext>
```

### hipso intensity

Normaliza la intensidad por la distancia al escáner, I · (R / R_ref)^a, a partir de la trayectoria (tiempo GPS, x, y, z) o de la altitud de vuelo y el ángulo de barrido

```
Normaliza la intensidad por la distancia al escáner, I · (R / R_ref)^a, a partir de la trayectoria (tiempo GPS, x, y, z) o de la altitud de vuelo y el ángulo de barrido

Uso: hipso intensity [OPTIONS] <INPUTS>...

Argumentos:
  <INPUTS>...  

Opciones:
      --results-dir <RESULTS_DIR>  Envía a esta carpeta todo producto al que no se dé una salida explícita, en lugar de la carpeta de cada entrada
      --trajectory <TRAJECTORY>    Trayectoria del escáner: un archivo de texto con líneas «tiempo x y z»
      --altitude <ALTITUDE>        Altitud del vuelo (m, mismo datum vertical que los puntos), cuando no hay trayectoria
      --exponent <EXPONENT>        Exponente a de la distancia: 2 para superficies (suelo, dosel), alrededor de 3 para cables [default: 2]
      --reference <REFERENCE>      Distancia de referencia R_ref (m); por defecto: la distancia media de cada nube
  -o, --output <OUTPUT>            Salida (solo una entrada); por defecto: <resultados>/<entrada>_intensidad.<ext>
```

### hipso swath

Diferencia de altura entre pasadas como GeoTIFF, con las cifras del solape

```
Diferencia de altura entre pasadas como GeoTIFF, con las cifras del solape

Uso: hipso swath [OPTIONS] <INPUTS>...

Argumentos:
  <INPUTS>...  

Opciones:
  -o, --output <OUTPUT>            
      --results-dir <RESULTS_DIR>  Envía a esta carpeta todo producto al que no se dé una salida explícita, en lugar de la carpeta de cada entrada
      --cell <CELL>                [default: 1]
      --returns <RETURNS>          Retornos que se usan: all, last, single [default: last]
      --by-cloud                   Toma cada nube como una pasada en lugar del identificador de fuente del punto
      --gps-gap <GPS_GAP>          Halla las pasadas a partir de los saltos del tiempo GPS mayores que esto (s), cuando el identificador de fuente del punto no las guarda
```

### hipso flightlines

Pasadas de una nube aérea halladas a partir de los saltos del tiempo GPS: cuántas hay, cuándo, cuántos puntos tienen y dónde. Con --assign, copias de las nubes que llevan el número de pasada

```
Pasadas de una nube aérea halladas a partir de los saltos del tiempo GPS: cuántas hay, cuándo, cuántos puntos tienen y dónde. Con --assign, copias de las nubes que llevan el número de pasada

Uso: hipso flightlines [OPTIONS] <INPUTS>...

Argumentos:
  <INPUTS>...  

Opciones:
      --gap <GAP>                    Un silencio del tiempo GPS mayor que esto (s) separa dos pasadas [default: 30]
      --results-dir <RESULTS_DIR>    Envía a esta carpeta todo producto al que no se dé una salida explícita, en lugar de la carpeta de cada entrada
      --min-points <MIN_POINTS>      Los tramos con menos puntos no son pasadas [default: 1000]
      --min-duration <MIN_DURATION>  Los tramos más cortos que esto (s) no son pasadas [default: 0]
      --assign <ASSIGN>              Escribe el número de pasada en copias de las nubes: `source-id` o `user-data`
      --no-csv                       Sin tabla CSV
      --no-vector                    Sin capa de polígonos con el contorno de cada pasada
  -o, --output <OUTPUT>              Carpeta de la tabla y de las copias; por defecto: la carpeta de resultados de la primera nube
```

### hipso compare

Puntos de una nube que han cambiado frente a una nube de referencia (detección de cambios)

```
Puntos de una nube que han cambiado frente a una nube de referencia (detección de cambios)

Uso: hipso compare [OPTIONS] --reference <REFERENCE> <INPUT>

Argumentos:
  <INPUT>  Nube en la que buscar cambios

Opciones:
      --reference <REFERENCE>      Nube(s) de referencia
      --results-dir <RESULTS_DIR>  Envía a esta carpeta todo producto al que no se dé una salida explícita, en lugar de la carpeta de cada entrada
  -o, --output <OUTPUT>            
      --distance <DISTANCE>        Distancia (m) a partir de la cual un punto ha cambiado [default: 0.5]
      --neighbors <NEIGHBORS>      Otros puntos cambiados que necesita un punto cambiado dentro del doble de la distancia [default: 3]
```

### hipso fit

Lleva una nube sobre una nube de referencia (punto más cercano iterativo) y escribe el resultado

```
Lleva una nube sobre una nube de referencia (punto más cercano iterativo) y escribe el resultado

Uso: hipso fit [OPTIONS] --reference <REFERENCE> <INPUT>

Argumentos:
  <INPUT>  

Opciones:
      --reference <REFERENCE>      
      --results-dir <RESULTS_DIR>  Envía a esta carpeta todo producto al que no se dé una salida explícita, en lugar de la carpeta de cada entrada
  -o, --output <OUTPUT>            
      --thin <THIN>                Adelgazamiento (m) de los puntos usados para hallar el movimiento [default: 1]
      --max-xy <MAX_XY>            [default: 5]
      --max-z <MAX_Z>              [default: 2]
      --iterations <ITERATIONS>    [default: 30]
      --stop <STOP>                [default: 0.01]
      --shift-only                 Solo traslada, no gira
      --plane                      Punto a plano: distancia al plano tangente del punto de referencia
      --trim <TRIM>                Proporción de los pares más cercanos que se usan para hallar el movimiento (1 los conserva todos) [default: 1]
```

### hipso hag

Altura de cada punto sobre el suelo, a partir de los puntos de suelo que ya tiene la nube (un TIN con ellos): se escribe como el atributo adicional HeightAboveGround o, con --replace-z, en lugar de la elevación

```
Altura de cada punto sobre el suelo, a partir de los puntos de suelo que ya tiene la nube (un TIN con ellos): se escribe como el atributo adicional HeightAboveGround o, con --replace-z, en lugar de la elevación

Uso: hipso hag [OPTIONS] <INPUTS>...

Argumentos:
  <INPUTS>...  

Opciones:
  -o, --output <OUTPUT>              Salida (solo una entrada); por defecto: <resultados>/<entrada>_alturas.<ext>
      --results-dir <RESULTS_DIR>    Envía a esta carpeta todo producto al que no se dé una salida explícita, en lugar de la carpeta de cada entrada
      --cell <CELL>                  Celda de la superficie del suelo [default: 0.5]
      --ground-class <GROUND_CLASS>  Clase de los puntos de suelo [default: 2]
      --replace-z                    Escribe la altura como elevación (una nube normalizada) en lugar de como atributo
```

## Clasificar y modelar superficies

### hipso classify

Clasifica ruido, suelo y edificios, tesela a tesela en paralelo, y escribe los puntos

```
Clasifica ruido, suelo y edificios, tesela a tesela en paralelo, y escribe los puntos

Uso: hipso classify [OPTIONS] <INPUTS>...

Argumentos:
  <INPUTS>...
          

Opciones:
  -o, --output <OUTPUT>
          LAS de salida, o LAZ si termina en .laz; por defecto: <resultados>/<entrada>_clasif<ajustes>

      --results-dir <RESULTS_DIR>
          Envía a esta carpeta todo producto al que no se dé una salida explícita, en lugar de la carpeta de cada entrada

      --neighbors <NEIGHBORS>
          Lee las hojas que estén a menos de tantos metros de cada hoja como contexto, de modo que sus bordes salgan como saldrían de todo el bloque, y escribe solo los puntos de la propia hoja (0: desactivado)
          
          [default: 0]

      --ground <GROUND>
          Cómo se encuentran los puntos de suelo (classify: kraus; dtm: classes, la clase 2 que ya trae la nube)

          Possible values:
          - kraus:    Filtro iterativo de Kraus y Pfeifer (por defecto)
          - mcc:      Multiscale Curvature Classification (Evans & Hudak 2007)
          - csf:      Cloth Simulation Filter (Zhang et al. 2016)
          - pmf:      Progressive Morphological Filter (Zhang et al. 2003)
          - smrf:     Simple Morphological Filter (Pingel et al. 2013)
          - axelsson: Progressive TIN densification (Axelsson 2000)
          - classes:  Puntos ya clasificados como suelo (clase 2)

      --filter-cell <FILTER_CELL>
          Kraus-Pfeifer: tamaño de celda de las superficies intermedias del filtro
          
          [default: 5]

      --iterations <ITERATIONS>
          Kraus-Pfeifer: filter iterations
          
          [default: 5]

      --g <G>
          Kraus-Pfeifer: desplazamiento g de la función de peso (los residuos por debajo pesan 1); también --kp-g
          
          [default: -2]
          [alias: --kp-g]

      --w <W>
          Kraus-Pfeifer: anchura w de la función de peso por encima de g (los residuos por encima de g + w pesan 0); también --kp-w
          
          [default: 2.5]
          [alias: --kp-w]

      --a <A>
          Kraus-Pfeifer: pendiente a de la función de peso; también --kp-a
          
          [default: 1]
          [alias: --kp-a]

      --b <B>
          Kraus-Pfeifer: pendiente b de la función de peso; también --kp-b
          
          [default: 4]
          [alias: --kp-b]

      --tolerance <TOLERANCE>
          Kraus-Pfeifer: suelo = a esta distancia de la superficie final (por defecto: residuo <= g + w)

      --mcc-spacing <MCC_SPACING>
          MCC: tamaño de celda del dominio de escala 2 (aproximadamente el espaciado de los puntos)
          
          [default: 1.5]

      --mcc-threshold <MCC_THRESHOLD>
          MCC: curvature threshold
          
          [default: 0.3]

      --csf-resolution <CSF_RESOLUTION>
          CSF: distancia entre las partículas de la tela
          
          [default: 0.5]

      --csf-rigidness <CSF_RIGIDNESS>
          CSF: rigidez, de 1 (terreno escarpado) a 3 (llano)
          
          [default: 1]

      --csf-threshold <CSF_THRESHOLD>
          CSF: altura sobre la tela hasta la que los puntos son suelo
          
          [default: 0.5]

      --pmf-window <PMF_WINDOW>
          PMF: mayor ventana
          
          [default: 20]

      --pmf-slope <PMF_SLOPE>
          PMF: pendiente del terreno (subida sobre recorrido)
          
          [default: 1]

      --smrf-cell <SMRF_CELL>
          SMRF: celda de las superficies
          
          [default: 1]

      --smrf-window <SMRF_WINDOW>
          SMRF: mayor radio de ventana (mayor que el edificio más grande)
          
          [default: 18]

      --smrf-slope <SMRF_SLOPE>
          SMRF: pendiente más empinada del terreno (subida sobre recorrido)
          
          [default: 0.15]

      --smrf-threshold <SMRF_THRESHOLD>
          SMRF: umbral de altura entre un punto y la superficie
          
          [default: 0.5]

      --smrf-scalar <SMRF_SCALAR>
          SMRF: se suma al umbral por unidad de pendiente
          
          [default: 1.25]

      --axelsson-seed-cell <AXELSSON_SEED_CELL>
          Axelsson: lado de las celdas cuyos puntos más bajos siembran el TIN (mayor que cualquier edificio)
          
          [default: 20]

      --axelsson-angle <AXELSSON_ANGLE>
          Axelsson: iteration angle (degrees)
          
          [default: 8]

      --axelsson-dist <AXELSSON_DIST>
          Axelsson: iteration distance (m)
          
          [default: 1.4]

      --axelsson-flat <AXELSSON_FLAT>
          Axelsson: los puntos a esta distancia (m) de un triángulo son suelo sea cual sea el ángulo
          
          [default: 0.1]

      --overlap
          Aparta antes que nada, como clase 12 (solape), los puntos de las pasadas que solapan con otra: donde varias pasadas cubren una celda, la de menor ángulo de barrido medio se la queda

      --overlap-cell <OVERLAP_CELL>
          Solape: lado de celda en metros
          
          [default: 5]

      --noise <NOISE>
          Clasifica primero el ruido (clase 7) con este método

          Possible values:
          - isolated: Puntos aislados en celdas de 3 x 3 x 3
          - sor:      Eliminación estadística de valores atípicos sobre los k vecinos más próximos
          - elm:      Ruido bajo: puntos muy por debajo de las alturas de su celda (mínimo local extendido)

      --sor-k <SOR_K>
          Ruido (sor): vecinos por punto
          
          [default: 10]

      --sor-m <SOR_M>
          Ruido (sor): desviaciones típicas por encima de la distancia media
          
          [default: 3]

      --noise-step <NOISE_STEP>
          Ruido (isolated): tamaño de celda en x e y
          
          [default: 4]

      --noise-step-z <NOISE_STEP_Z>
          Ruido: tamaño de celda en z
          
          [default: 4]

      --elm-cell <ELM_CELL>
          Ruido (elm): lado de celda en metros
          
          [default: 10]

      --elm-threshold <ELM_THRESHOLD>
          Ruido (elm): metros por debajo del siguiente punto de su celda y de los puntos más bajos de las celdas vecinas que debe estar el punto más bajo de una celda para ser ruido bajo
          
          [default: 3]

      --isolated <ISOLATED>
          Ruido: el mayor número de otros puntos alrededor de un punto que aún se llama aislado
          
          [default: 5]

      --vegetation
          Clasifica la vegetación por altura sobre el suelo: baja (3), media (4) y alta (5)

      --wires
          Clasifica los cables (14): líneas casi horizontales muy por encima del suelo

      --poles
          Clasifica los postes y torres (15): estructuras estrechas y aisladas que se elevan del suelo

      --roads
          Clasifica las carreteras (11): suelo liso, llano y oscuro (intensidad baja)

      --water
          Clasifica el agua (9): regiones extensas y niveladas de suelo oscuro de retorno único

      --rule <RULES>
          Clasifica por reglas, al final y en orden. Se repite para varias reglas. Partes, separadas por `;`: to=CLASE (obligatoria), from=1,2 (o ~7), returns=F|L|A|1..9, angle=25..90 (grados absolutos; `angle=25` es 25 o más), height=0.5..2 (m sobre el suelo), intensity=0..100 y la forma del entorno dentro de radius=1 (m): slope=60.. (grados), planarity, linearity, sphericity (0 a 1), roughness (m), neighbours (número). Por ejemplo: `angle=25;to=12` o `from=1;height=0.5..2;to=4`

      --buildings
          Clasifica edificios (6) y vegetación alta (5), y quita los tejados del suelo

      --recipe <FILE>
          Lee más de estas opciones de un archivo de texto, una o varias por línea (`#` inicia una línea de comentario): una clasificación escrita una vez y ejecutada en cualquier hoja o bloque. Las opciones del archivo van donde está `--recipe`, de modo que no pueden repetirse en la línea de comandos, salvo `--rule`

      --building-offset <BUILDING_OFFSET>
          Edificios: altura sobre el terreno a partir de la cual los puntos son edificios o árboles
          
          [default: 2.5]

      --building-window <BUILDING_WINDOW>
          Buildings: widest building expected
          
          [default: 60]

      --building-area <BUILDING_AREA>
          Buildings: smallest footprint (square units)
          
          [default: 40]

      --tile <TILE>
          Tamaño de tesela
          
          [default: 250]

      --buffer <BUFFER>
          Margen que se lee alrededor de cada tesela (elevado a la mitad de la ventana de edificios al clasificar edificios)
          
          [default: 30]

      --threads <THREADS>
          Hilos (0: uno por núcleo)
          
          [default: 0]

  -h, --help
```

### hipso dtm

MDT como GeoTIFF, uno por hoja. Por defecto: la clase de suelo (2) que ya trae la nube; --ground prueba otro método

```
MDT como GeoTIFF, uno por hoja. Por defecto: la clase de suelo (2) que ya trae la nube; --ground prueba otro método

Uso: hipso dtm [OPTIONS] <INPUTS>...

Argumentos:
  <INPUTS>...
          

Opciones:
  -o, --output <OUTPUT>
          GeoTIFF de salida; por defecto: <resultados>/<entrada>_mdt_<celda>m<ajustes>.tif

      --results-dir <RESULTS_DIR>
          Envía a esta carpeta todo producto al que no se dé una salida explícita, en lugar de la carpeta de cada entrada

      --cell <CELL>
          Tamaño de celda del MDT
          
          [default: 1]

      --surface <SURFACE>
          Cómo reciben su elevación las celdas

          Possible values:
          - mean: Elevación media de los puntos de suelo por celda (en flujo, una pasada)
          - tin:  Triangulación de Delaunay (TIN) de los puntos de suelo (por defecto)
          - idw:  Ponderación por el inverso de la distancia de los puntos de suelo más próximos
          
          [default: tin]

      --smooth <SMOOTH>
          Suaviza el MDT con una ventana de este tamaño en unidades del mapa (0: nada)
          
          [default: 0]

      --smoothing <SMOOTHING>
          [default: mean]
          [possible values: mean, median, gaussian]

      --mosaic
          Une además los MDT de las hojas en un mosaico (por defecto cada hoja conserva el suyo)

      --breaklines <BREAKLINES>
          Líneas de ruptura que debe seguir el MDT (shapefile de líneas o polígonos): con elevaciones (PolylineZ) fijan la altura; sin ellas, toman el suelo más bajo a su lado

      --neighbors <NEIGHBORS>
          Hace el MDT de cada hoja con las hojas que estén a menos de tantos metros de ella, recortado a su propia extensión: sin costura en los bordes de las hojas (0: cada hoja por separado)
          
          [default: 0]

      --ground <GROUND>
          Cómo se encuentran los puntos de suelo (classify: kraus; dtm: classes, la clase 2 que ya trae la nube)

          Possible values:
          - kraus:    Filtro iterativo de Kraus y Pfeifer (por defecto)
          - mcc:      Multiscale Curvature Classification (Evans & Hudak 2007)
          - csf:      Cloth Simulation Filter (Zhang et al. 2016)
          - pmf:      Progressive Morphological Filter (Zhang et al. 2003)
          - smrf:     Simple Morphological Filter (Pingel et al. 2013)
          - axelsson: Progressive TIN densification (Axelsson 2000)
          - classes:  Puntos ya clasificados como suelo (clase 2)

      --filter-cell <FILTER_CELL>
          Kraus-Pfeifer: tamaño de celda de las superficies intermedias del filtro
          
          [default: 5]

      --iterations <ITERATIONS>
          Kraus-Pfeifer: filter iterations
          
          [default: 5]

      --g <G>
          Kraus-Pfeifer: desplazamiento g de la función de peso (los residuos por debajo pesan 1); también --kp-g
          
          [default: -2]
          [alias: --kp-g]

      --w <W>
          Kraus-Pfeifer: anchura w de la función de peso por encima de g (los residuos por encima de g + w pesan 0); también --kp-w
          
          [default: 2.5]
          [alias: --kp-w]

      --a <A>
          Kraus-Pfeifer: pendiente a de la función de peso; también --kp-a
          
          [default: 1]
          [alias: --kp-a]

      --b <B>
          Kraus-Pfeifer: pendiente b de la función de peso; también --kp-b
          
          [default: 4]
          [alias: --kp-b]

      --tolerance <TOLERANCE>
          Kraus-Pfeifer: suelo = a esta distancia de la superficie final (por defecto: residuo <= g + w)

      --mcc-spacing <MCC_SPACING>
          MCC: tamaño de celda del dominio de escala 2 (aproximadamente el espaciado de los puntos)
          
          [default: 1.5]

      --mcc-threshold <MCC_THRESHOLD>
          MCC: curvature threshold
          
          [default: 0.3]

      --csf-resolution <CSF_RESOLUTION>
          CSF: distancia entre las partículas de la tela
          
          [default: 0.5]

      --csf-rigidness <CSF_RIGIDNESS>
          CSF: rigidez, de 1 (terreno escarpado) a 3 (llano)
          
          [default: 1]

      --csf-threshold <CSF_THRESHOLD>
          CSF: altura sobre la tela hasta la que los puntos son suelo
          
          [default: 0.5]

      --pmf-window <PMF_WINDOW>
          PMF: mayor ventana
          
          [default: 20]

      --pmf-slope <PMF_SLOPE>
          PMF: pendiente del terreno (subida sobre recorrido)
          
          [default: 1]

      --smrf-cell <SMRF_CELL>
          SMRF: celda de las superficies
          
          [default: 1]

      --smrf-window <SMRF_WINDOW>
          SMRF: mayor radio de ventana (mayor que el edificio más grande)
          
          [default: 18]

      --smrf-slope <SMRF_SLOPE>
          SMRF: pendiente más empinada del terreno (subida sobre recorrido)
          
          [default: 0.15]

      --smrf-threshold <SMRF_THRESHOLD>
          SMRF: umbral de altura entre un punto y la superficie
          
          [default: 0.5]

      --smrf-scalar <SMRF_SCALAR>
          SMRF: se suma al umbral por unidad de pendiente
          
          [default: 1.25]

      --axelsson-seed-cell <AXELSSON_SEED_CELL>
          Axelsson: lado de las celdas cuyos puntos más bajos siembran el TIN (mayor que cualquier edificio)
          
          [default: 20]

      --axelsson-angle <AXELSSON_ANGLE>
          Axelsson: iteration angle (degrees)
          
          [default: 8]

      --axelsson-dist <AXELSSON_DIST>
          Axelsson: iteration distance (m)
          
          [default: 1.4]

      --axelsson-flat <AXELSSON_FLAT>
          Axelsson: los puntos a esta distancia (m) de un triángulo son suelo sea cual sea el ángulo
          
          [default: 0.1]

      --overlap
          Aparta antes que nada, como clase 12 (solape), los puntos de las pasadas que solapan con otra: donde varias pasadas cubren una celda, la de menor ángulo de barrido medio se la queda

      --overlap-cell <OVERLAP_CELL>
          Solape: lado de celda en metros
          
          [default: 5]

      --noise <NOISE>
          Clasifica primero el ruido (clase 7) con este método

          Possible values:
          - isolated: Puntos aislados en celdas de 3 x 3 x 3
          - sor:      Eliminación estadística de valores atípicos sobre los k vecinos más próximos
          - elm:      Ruido bajo: puntos muy por debajo de las alturas de su celda (mínimo local extendido)

      --sor-k <SOR_K>
          Ruido (sor): vecinos por punto
          
          [default: 10]

      --sor-m <SOR_M>
          Ruido (sor): desviaciones típicas por encima de la distancia media
          
          [default: 3]

      --noise-step <NOISE_STEP>
          Ruido (isolated): tamaño de celda en x e y
          
          [default: 4]

      --noise-step-z <NOISE_STEP_Z>
          Ruido: tamaño de celda en z
          
          [default: 4]

      --elm-cell <ELM_CELL>
          Ruido (elm): lado de celda en metros
          
          [default: 10]

      --elm-threshold <ELM_THRESHOLD>
          Ruido (elm): metros por debajo del siguiente punto de su celda y de los puntos más bajos de las celdas vecinas que debe estar el punto más bajo de una celda para ser ruido bajo
          
          [default: 3]

      --isolated <ISOLATED>
          Ruido: el mayor número de otros puntos alrededor de un punto que aún se llama aislado
          
          [default: 5]

      --vegetation
          Clasifica la vegetación por altura sobre el suelo: baja (3), media (4) y alta (5)

      --wires
          Clasifica los cables (14): líneas casi horizontales muy por encima del suelo

      --poles
          Clasifica los postes y torres (15): estructuras estrechas y aisladas que se elevan del suelo

      --roads
          Clasifica las carreteras (11): suelo liso, llano y oscuro (intensidad baja)

      --water
          Clasifica el agua (9): regiones extensas y niveladas de suelo oscuro de retorno único

      --rule <RULES>
          Clasifica por reglas, al final y en orden. Se repite para varias reglas. Partes, separadas por `;`: to=CLASE (obligatoria), from=1,2 (o ~7), returns=F|L|A|1..9, angle=25..90 (grados absolutos; `angle=25` es 25 o más), height=0.5..2 (m sobre el suelo), intensity=0..100 y la forma del entorno dentro de radius=1 (m): slope=60.. (grados), planarity, linearity, sphericity (0 a 1), roughness (m), neighbours (número). Por ejemplo: `angle=25;to=12` o `from=1;height=0.5..2;to=4`

      --buildings
          Clasifica edificios (6) y vegetación alta (5), y quita los tejados del suelo

      --recipe <FILE>
          Lee más de estas opciones de un archivo de texto, una o varias por línea (`#` inicia una línea de comentario): una clasificación escrita una vez y ejecutada en cualquier hoja o bloque. Las opciones del archivo van donde está `--recipe`, de modo que no pueden repetirse en la línea de comandos, salvo `--rule`

      --building-offset <BUILDING_OFFSET>
          Edificios: altura sobre el terreno a partir de la cual los puntos son edificios o árboles
          
          [default: 2.5]

      --building-window <BUILDING_WINDOW>
          Buildings: widest building expected
          
          [default: 60]

      --building-area <BUILDING_AREA>
          Buildings: smallest footprint (square units)
          
          [default: 40]

      --tile <TILE>
          Tamaño de tesela
          
          [default: 250]

      --buffer <BUFFER>
          Margen que se lee alrededor de cada tesela (elevado a la mitad de la ventana de edificios al clasificar edificios)
          
          [default: 30]

      --threads <THREADS>
          Hilos (0: uno por núcleo)
          
          [default: 0]

  -h, --help
```

### hipso dsm

Modelo digital de superficie (retornos más altos) como GeoTIFF, uno por hoja

```
Modelo digital de superficie (retornos más altos) como GeoTIFF, uno por hoja

Uso: hipso dsm [OPTIONS] <INPUTS>...

Argumentos:
  <INPUTS>...
          

Opciones:
  -o, --output <OUTPUT>
          GeoTIFF de salida (solo una entrada); por defecto: <resultados>/<entrada>_mds_<celda>m_<método>.tif

      --results-dir <RESULTS_DIR>
          Envía a esta carpeta todo producto al que no se dé una salida explícita, en lugar de la carpeta de cada entrada

      --cell <CELL>
          [default: 1]

      --method <METHOD>
          Possible values:
          - max: Punto más alto de cada celda (en flujo, una pasada)
          - tin: TIN por el punto más alto de cada celda (por defecto)
          - idw: Ponderación por el inverso de la distancia de los puntos más altos de las celdas
          
          [default: tin]

      --no-fill
          Deja vacías las celdas sin puntos (método max)

      --tile <TILE>
          Tamaño de tesela (métodos tin e idw)
          
          [default: 250]

      --threads <THREADS>
          Hilos (0: uno por núcleo)
          
          [default: 0]

      --smooth <SMOOTH>
          Suaviza con una ventana de este tamaño en unidades del mapa (0: nada)
          
          [default: 0]

      --smoothing <SMOOTHING>
          [default: mean]
          [possible values: mean, median, gaussian]

      --neighbors <NEIGHBORS>
          Lee las hojas que estén a menos de tantos metros de cada hoja como contexto, de modo que sus bordes salgan como saldrían de todo el bloque, y escribe solo los puntos de la propia hoja (0: desactivado)
          
          [default: 0]

  -h, --help
```

### hipso chm

Modelo de altura de copas como GeoTIFF, uno por hoja

```
Modelo de altura de copas como GeoTIFF, uno por hoja

Uso: hipso chm [OPTIONS] <INPUTS>...

Argumentos:
  <INPUTS>...
          

Opciones:
      --dtm <DTM>
          Modelo del suelo (GeoTIFF o rejilla ASCII); por defecto: el MDT más reciente hecho con esta misma nube

      --results-dir <RESULTS_DIR>
          Envía a esta carpeta todo producto al que no se dé una salida explícita, en lugar de la carpeta de cada entrada

  -o, --output <OUTPUT>
          GeoTIFF de salida; por defecto: <resultados>/<entrada>_chm_<celda>m.tif

      --cell <CELL>
          [default: 1]

      --no-fill
          Deja vacías las celdas sin puntos (método max)

      --method <METHOD>
          Punto más alto por celda, o pila de TIN sin picos (Khosravipour et al., 2014)

          Possible values:
          - max:     Punto más alto de cada celda
          - pitfree: Pila de TIN sin picos (Khosravipour et al., 2014)
          - tin:     TIN por el punto más alto de cada celda
          - idw:     Ponderación por el inverso de la distancia del punto más alto de cada celda
          
          [default: max]

      --max-edge <MAX_EDGE>
          Sin picos: arista de triángulo más larga en las capas por encima de 0
          
          [default: 1]

      --tile <TILE>
          Sin picos: tamaño de tesela
          
          [default: 250]

      --threads <THREADS>
          Sin picos: hilos (0: uno por núcleo)
          
          [default: 0]

      --smooth <SMOOTH>
          Suaviza el CHM con una ventana de este tamaño en unidades del mapa (0: nada)
          
          [default: 0]

      --smoothing <SMOOTHING>
          [default: mean]
          [possible values: mean, median, gaussian]

      --mosaic
          Une además los CHM de las hojas en un mosaico (por defecto cada hoja conserva el suyo)

      --neighbors <NEIGHBORS>
          Lee las hojas que estén a menos de tantos metros de cada hoja como contexto, de modo que sus bordes salgan como saldrían de todo el bloque, y escribe solo los puntos de la propia hoja (0: desactivado)
          
          [default: 0]

  -h, --help
```

### hipso hillshade

PNG de sombreado de un MDT o un CHM

```
PNG de sombreado de un MDT o un CHM

Uso: hipso hillshade [OPTIONS] <INPUT>

Argumentos:
  <INPUT>  

Opciones:
  -o, --output <OUTPUT>            
      --results-dir <RESULTS_DIR>  Envía a esta carpeta todo producto al que no se dé una salida explícita, en lugar de la carpeta de cada entrada
      --azimuth <AZIMUTH>          [default: 315]
      --altitude <ALTITUDE>        [default: 45]
```

### hipso contour

Curvas de nivel de un MDT o un CHM como shapefile de polilíneas 3D

```
Curvas de nivel de un MDT o un CHM como shapefile de polilíneas 3D

Uso: hipso contour [OPTIONS] <INPUT>

Argumentos:
  <INPUT>  

Opciones:
  -o, --output <OUTPUT>            
      --results-dir <RESULTS_DIR>  Envía a esta carpeta todo producto al que no se dé una salida explícita, en lugar de la carpeta de cada entrada
      --interval <INTERVAL>        [default: 5]
```

### hipso raster

Rejilla de una estadística de los puntos (densidad, recuento, elevación, intensidad) como GeoTIFF

```
Rejilla de una estadística de los puntos (densidad, recuento, elevación, intensidad) como GeoTIFF

Uso: hipso raster [OPTIONS] <INPUTS>...

Argumentos:
  <INPUTS>...  

Opciones:
  -o, --output <OUTPUT>            
      --results-dir <RESULTS_DIR>  Envía a esta carpeta todo producto al que no se dé una salida explícita, en lugar de la carpeta de cada entrada
      --cell <CELL>                [default: 1]
      --stat <STAT>                [default: density] [possible values: density, count, mean-z, max-z, min-z, range-z, mean-intensity, max-intensity]
      --class <CLASS>              Clases que se conservan: 2,5 o ~7,18 para todas menos esas
      --returns <RETURNS>          Retornos que se conservan: A, F, L o números (p. ej. 1,2)
      --zmin <ZMIN>                
      --zmax <ZMAX>                
      --decimate <DECIMATE>        Conserva un punto de cada tantos [default: 1]
```

### hipso metrics

Métricas por celda de la rejilla: elevación y, opcionalmente, intensidad y estratos

```
Métricas por celda de la rejilla: elevación y, opcionalmente, intensidad y estratos

Uso: hipso metrics [OPTIONS] <INPUTS>...

Argumentos:
  <INPUTS>...
          

Opciones:
      --dtm <DTM>
          Modelo del suelo (GeoTIFF o rejilla ASCII); por defecto: el MDT más reciente hecho con esta misma nube

      --results-dir <RESULTS_DIR>
          Envía a esta carpeta todo producto al que no se dé una salida explícita, en lugar de la carpeta de cada entrada

  -o, --output <OUTPUT>
          Shapefile de celdas de salida (o .csv si se da uno); por defecto: <resultados>/<entrada>_metricas_<celda>m.shp

      --cell <CELL>
          [default: 15]

      --heightbreak <HEIGHTBREAK>
          Altura de corte para las métricas de cobertura
          
          [default: 2]

      --neighbors <NEIGHBORS>
          Con varias hojas, da a las celdas del borde de cada hoja los puntos de las vecinas a menos de tantos metros (al menos una celda) y escribe cada celda una sola vez
          
          [default: 0]

      --min-points <MIN_POINTS>
          [default: 4]

      --memory-mb <MEMORY_MB>
          Máximo de memoria, en MiB, para las alturas de los puntos (las nubes mayores requieren más pasadas)
          
          [default: 512]

      --intensity
          Escribe también métricas de intensidad en <nombre de salida>_intensity (mismo formato que la salida)

      --strata
          Escribe también métricas de estratos en <nombre de salida>_strata (mismo formato que la salida)

      --strata-limits <STRATA_LIMITS>
          Límites superiores de los estratos de altura
          
          [default: 0.15 1.37 5 10 20 30]

      --int-strata-limits <INT_STRATA_LIMITS>
          Límites superiores de los estratos de intensidad
          
          [default: 0.15 1.37]

      --structure
          Escribe también métricas de estructura en <nombre de salida>_estructura: fracción de huecos, índice de área foliar, diversidad de la altura del follaje, índice de complejidad vertical y altura de la base de la copa

      --structure-layer <STRUCTURE_LAYER>
          Estructura: grosor de las capas del perfil vertical
          
          [default: 1]

      --structure-top <STRUCTURE_TOP>
          Estructura: altura del perfil (los retornos por encima caen en su última capa)
          
          [default: 30]

      --gap-height <GAP_HEIGHT>
          Estructura: los retornos por debajo de esta altura cuentan como luz que llegó al suelo
          
          [default: 1]

      --extinction <EXTINCTION>
          Estructura: coeficiente de extinción del índice de área foliar
          
          [default: 0.5]

      --cbh-floor <CBH_FLOOR>
          Estructura: la base de la copa se busca por encima de esta altura
          
          [default: 2]

      --cbh-share <CBH_SHARE>
          Estructura: una capa es copa cuando guarda esta proporción de los retornos de la capa más llena
          
          [default: 0.1]

      --format <FORMAT>
          Formato de salida: shapefile de celdas, tabla de texto o ambos (por defecto: según el nombre de la salida)

          Possible values:
          - vector: Solo shapefile
          - csv:    Solo tabla de texto (lo mejor para ejecuciones muy grandes)
          - both:   Both

  -h, --help
```

## Vegetación e inventario

### hipso trees

Árboles individuales a partir de un modelo de altura de copas: cimas (máximos locales), copas (Dalponte, Silva o cuenca), polígonos, treeID por punto y comprobación con datos de campo

```
Árboles individuales a partir de un modelo de altura de copas: cimas (máximos locales), copas (Dalponte, Silva o cuenca), polígonos, treeID por punto y comprobación con datos de campo

Uso: hipso trees [OPTIONS] <CHM>

Argumentos:
  <CHM>
          Modelo de altura de copas (GeoTIFF o rejilla ASCII)

Opciones:
      --around <AROUND>
          Modelos de copas de las hojas vecinas de esta (se repite para cada una): los árboles se buscan en esta hoja ampliada con --margin con lo que tienen allí, y solo se conservan los árboles con su cima en esta hoja, de modo que un árbol en el borde de una hoja se encuentra una sola vez y entero

      --results-dir <RESULTS_DIR>
          Envía a esta carpeta todo producto al que no se dé una salida explícita, en lugar de la carpeta de cada entrada

      --margin <MARGIN>
          Hasta dónde (m) se mira en las hojas vecinas
          
          [default: 30]

  -o, --output <OUTPUT>
          Cimas de los árboles (.shp, o .csv si se da uno); por defecto: <resultados>/<chm>_arboles_<método>.shp (nunca un archivo existente)

      --crowns
          Escribe además el ráster de copas (identificadores de árbol)

      --crowns-shp
          Escribe además los polígonos de copa (shapefile)

      --tops-shp
          Escribe además las cimas de los árboles (shapefile PointZ)

      --cloud <CLOUD>
          Copia esta nube añadiendo un atributo treeID (LAS/LAZ), en <resultados>/<nube>_treeid

      --tops-file <TOPS_FILE>
          Cimas dadas a mano: CSV con líneas x,y (en lugar de buscarlas)

      --hmin <HMIN>
          Cima de árbol más baja
          
          [default: 2]

      --ws-a <WS_A>
          Diámetro de ventana = a + b h + c h² (unidades del mapa)
          
          [default: 3]

      --ws-b <WS_B>
          [default: 0.1]

      --ws-c <WS_C>
          [default: 0]

      --shape <SHAPE>
          Forma de la ventana de búsqueda de cimas
          
          [default: circle]
          [possible values: circle, square]

      --method <METHOD>
          Crown method
          
          [default: dalponte]
          [possible values: dalponte, silva, watershed]

      --th-seed <TH_SEED>
          Dalponte: un píxel se une por encima de esta proporción de la altura de su cima...
          
          [default: 0.45]

      --th-cr <TH_CR>
          ...y de la altura media de la copa
          
          [default: 0.55]

      --max-cr <MAX_CR>
          Dalponte: alcance de la copa en píxeles
          
          [default: 10]

      --max-cr-factor <MAX_CR_FACTOR>
          Silva: radio de la copa como proporción de la altura del árbol
          
          [default: 0.6]

      --exclusion <EXCLUSION>
          Silva: descarta los píxeles más bajos que esta proporción de la altura del árbol
          
          [default: 0.3]

      --tolerance <TOLERANCE>
          Cuenca: escalón mínimo de altura entre copas
          
          [default: 1]

      --ext <EXT>
          Cuenca: radio de búsqueda de picos en píxeles
          
          [default: 1]

      --smooth <SMOOTH>
          Suaviza antes el CHM con una ventana de este tamaño (unidades del mapa; 0: nada)
          
          [default: 0]

      --smoothing <SMOOTHING>
          [default: mean]
          [possible values: mean, median, gaussian]

      --field <FIELD>
          Inventario de campo (delimitado por tabuladores, ; o ,, se admiten comas decimales) con el que comprobar

      --field-x <FIELD_X>
          [default: X_arbol]

      --field-y <FIELD_Y>
          [default: Y_arbol]

      --field-h <FIELD_H>
          [default: Ht]

      --field-plot <FIELD_PLOT>
          [default: Muestra]

      --plot-x <PLOT_X>
          [default: X_Muestra]

      --plot-y <PLOT_Y>
          [default: Y_Muestra]

      --plot-radius <PLOT_RADIUS>
          Radio de la parcela para la comprobación
          
          [default: 10]

      --format <FORMAT>
          Formato de salida: shapefile de cimas, tabla de texto o ambos

          Possible values:
          - vector: Solo shapefile
          - csv:    Solo tabla de texto (lo mejor para ejecuciones muy grandes)
          - both:   Both

      --match-dist <MATCH_DIST>
          Una detección coincide con un árbol de campo a esta distancia como máximo
          
          [default: 3]

  -h, --help
```

### hipso obia

Análisis basado en objetos: agrupa las celdas de varias capas (altura de copas, intensidad, retornos, rugosidad…) en objetos, los describe y clasifica, y escribe polígonos

```
Análisis basado en objetos: agrupa las celdas de varias capas (altura de copas, intensidad, retornos, rugosidad…) en objetos, los describe y clasifica, y escribe polígonos

Uso: hipso obia [OPTIONS] <RASTERS>...

Argumentos:
  <RASTERS>...
          Rásteres sobre las mismas celdas (GeoTIFF), el modelo de altura de copas el primero

Opciones:
      --cloud <CLOUD>...
          Nubes de puntos de las que derivar capas (véase --cloud-layers)

      --results-dir <RESULTS_DIR>
          Envía a esta carpeta todo producto al que no se dé una salida explícita, en lugar de la carpeta de cada entrada

      --cloud-layers <CLOUD_LAYERS>
          Capas que se hacen con las nubes: intensidad, retornos, suelo, densidad
          
          [default: intensidad,retornos]

      --cloud-fill <CLOUD_FILL>
          Las celdas vacías de las capas hechas con las nubes toman el valor de la celda más cercana que lo tenga, hasta tantas celdas de distancia (0: quedan vacías y dejan huecos)
          
          [default: 3]

      --cloud-smooth <CLOUD_SMOOTH>
          Promedia esas capas en una ventana de este radio en celdas (0: no)
          
          [default: 1]

      --roughness <ROUGHNESS>
          Añade la rugosidad local del primer ráster en una ventana de tantas celdas (0: no)
          
          [default: 0]

      --method <METHOD>
          Cómo se hacen los objetos

          Possible values:
          - merge: Fusión multirresolución de manchas
          - mask:  Celdas conectadas con un valor dentro de un rango
          
          [default: merge]

      --scale <SCALE>
          Fusión: las manchas se fusionan mientras el coste sea menor que la escala al cuadrado (mayor: objetos mayores)
          
          [default: 8]

      --shape <SHAPE>
          Fusión: peso de la forma frente al valor, de 0 a 0,9
          
          [default: 0.1]

      --compact <COMPACT>
          Fusión: peso de la compacidad frente a la suavidad dentro de la forma, de 0 a 1
          
          [default: 0.5]

      --weight <WEIGHTS>
          Fusión: peso de una capa, como capa=peso (0 la deja fuera); se repite para varias

      --init <INIT>
          Fusión: anchura, en desviaciones típicas, de los intervalos de las manchas iniciales
          
          [default: 0.25]

      --mask-layer <MASK_LAYER>
          Máscara: la capa y el rango de valores que forman un objeto

      --min <MIN>
          [default: -inf]

      --max <MAX>
          [default: inf]

      --diagonal
          Máscara: las celdas que se tocan por una esquina van juntas

      --min-cells <MIN_CELLS>
          Objeto más pequeño, en celdas
          
          [default: 10]

      --rules <RULES>
          Clasifica con reglas de un archivo de texto (clase: atributo >= valor y ...)

      --samples <SAMPLES>
          Clasifica con un bosque aleatorio entrenado con polígonos de muestra (.shp)

      --kmeans <KMEANS>
          Agrupa los objetos en tantos estratos según su parecido (sin reglas ni muestras)

      --features <FEATURES>
          Atributos con los que se hacen los estratos (por defecto: las medias y los percentiles 95 de las capas, y la cobertura del dosel)

      --strata
          Une las clases en estratos: las superficies de una clase que se tocan pasan a ser un polígono

      --strata-min-cells <STRATA_MIN_CELLS>
          Los estratos más pequeños que tantas celdas toman la clase de sus vecinos
          
          [default: 500]

      --plots <PLOTS>
          Reparte tantas parcelas de muestra entre los estratos (asignación de Neyman)
          
          [default: 0]

      --cover-height <COVER_HEIGHT>
          Altura a partir de la cual una celda cuenta como cubierta por el dosel (atributo `cobertura`)
          
          [default: 2]

      --class-field <CLASS_FIELD>
          Campo de las muestras que da nombre a la clase
          
          [default: clase]

      --trees <TREES>
          Árboles del bosque
          
          [default: 100]

      --seed <SEED>
          [default: 1]

  -o, --output <OUTPUT>
          Ruta base de los resultados (.shp o .csv); por defecto: <resultados>/<ráster>_objetos.shp

      --format <FORMAT>
          Possible values:
          - vector: Solo shapefile
          - csv:    Solo tabla de texto (lo mejor para ejecuciones muy grandes)
          - both:   Both

      --simplify <SIMPLIFY>
          Simplifica los polígonos en tantas unidades del mapa (0: contornos exactos)
          
          [default: 0]

  -h, --help
```

### hipso stem-volume

Mide los troncos de una nube terrestre o móvil a lo largo del tronco y estima el volumen de cada individuo con la fórmula de Smalian (un shapefile o CSV de árboles, un CSV de trozas y, opcionalmente, nubes por árbol)

```
Mide los troncos de una nube terrestre o móvil a lo largo del tronco y estima el volumen de cada individuo con la fórmula de Smalian (un shapefile o CSV de árboles, un CSV de trozas y, opcionalmente, nubes por árbol)

Uso: hipso stem-volume [OPTIONS] <INPUTS>...

Argumentos:
  <INPUTS>...  

Opciones:
  -o, --output <OUTPUT>
          Tabla de árboles (.shp o .csv); por defecto: <resultados>/<entrada>_volumen.shp
      --results-dir <RESULTS_DIR>
          Envía a esta carpeta todo producto al que no se dé una salida explícita, en lugar de la carpeta de cada entrada
      --csv
          Escribe una tabla de texto además del shapefile
      --normalized
          Las z de la nube ya son alturas sobre el suelo
      --base-height <BASE_HEIGHT>
          Altura sobre el suelo (m) donde empieza la primera troza [default: 0.1]
      --top-diameter <TOP_DIAMETER>
          Diámetro (m) en el que termina el fuste comercial [default: 0.07]
      --log-length <LOG_LENGTH>
          Distancia (m) entre los diámetros medidos: la longitud de una troza [default: 0.5]
      --voxel <VOXEL>
          Vóxel (m) al que se reduce la nube [default: 0.03]
      --stripe-low <STRIPE_LOW>
          Franja (m sobre el suelo) donde se buscan los troncos [default: 0.7]
      --stripe-high <STRIPE_HIGH>
          [default: 3.5]
      --max-diameter <MAX_DIAMETER>
          Largest stem diameter expected (m) [default: 1]
      --tree-clouds
          Escribe además una nube por árbol
      --stem-classes
          Escribe además la nube con los troncos en la clase 64 y las ramas en la clase 65
      --assign-dist <ASSIGN_DIST>
          Distancia (m) hasta la que un punto va al árbol más cercano en las nubes por árbol [default: 8]
      --stool-dist <STOOL_DIST>
          Distancia (m) entre las bases de los troncos de una cepa (brotes de monte bajo) [default: 0.6]
      --fork-height <FORK_HEIGHT>
          Altura (m) del diámetro: los troncos separados a esa altura son brotes, cada uno un árbol; los troncos que son uno a esa altura y se bifurcan por encima son un solo árbol bifurcado [default: 1.3]
      --max-gap <MAX_GAP>
          Mayor tramo (m) de un tronco que puede quedar sin medir antes de darlo por perdido [default: 2]
      --lost-top-share <LOST_TOP_SHARE>
          Altura comercial como proporción (0-1) de la altura del árbol, para estimarla en los troncos perdidos cuando aún eran gruesos y muy por debajo de la cima (por ejemplo 0,65 para el eucalipto); 0: sin estimación [default: 0]
  -h, --help
```

### hipso model

Modelo de transición (por área): regresión de una variable de campo (volumen, área basimétrica…) sobre las métricas LiDAR de las parcelas, con validación cruzada dejando una fuera que repite la elección de métricas, diagnósticos y, dadas las métricas de las celdas, el mapa de la variable

```
Modelo de transición (por área): regresión de una variable de campo (volumen, área basimétrica…) sobre las métricas LiDAR de las parcelas, con validación cruzada dejando una fuera que repite la elección de métricas, diagnósticos y, dadas las métricas de las celdas, el mapa de la variable

Uso: hipso model [OPTIONS] --plots <PLOTS> --field <FIELD> --response <RESPONSE>

Opciones:
      --plots <PLOTS>
          Métricas de las parcelas: la capa (.shp) o la tabla de texto de `clip --metrics`

      --results-dir <RESULTS_DIR>
          Envía a esta carpeta todo producto al que no se dé una salida explícita, en lugar de la carpeta de cada entrada

      --field <FIELD>
          Tabla de campo, una fila por parcela (.csv, .txt o .xlsx)

      --response <RESPONSE>
          Columna de la tabla de campo que se estima

      --plot-key <PLOT_KEY>
          Columna que nombra la parcela en la tabla de métricas
          
          [default: Muestra]

      --field-key <FIELD_KEY>
          Columna que nombra la parcela en la tabla de campo; por defecto: el mismo nombre

      --predictors <PREDICTORS>
          Métricas que se usan, separadas por comas; por defecto: elegidas automáticamente

      --transform <TRANSFORM>
          Possible values:
          - none:         Sin transformación
          - log-response: Logaritmo de la respuesta
          - log-both:     Logaritmo de la respuesta y de las métricas (una ley de potencias)
          
          [default: none]

      --criterion <CRITERION>
          Criterio de la elección automática
          
          [default: bic]
          [possible values: bic, aicc]

      --max-vars <MAX_VARS>
          Máximo de métricas que conserva la elección automática (0: una por cada cinco parcelas, como mucho cinco)
          
          [default: 0]

      --max-vif <MAX_VIF>
          Mayor factor de inflación de la varianza que acepta la elección automática
          
          [default: 5]

      --candidates <CANDIDATES>
          Métricas que puede usar la elección automática, separadas por comas; por defecto: todas las de la tabla

      --max-corr <MAX_CORR>
          Los candidatos correlacionados más que esto dicen lo mismo: solo se conserva el más relacionado con la variable (1: se conservan todos)
          
          [default: 0.8]

      --analyze
          Solo el análisis previo al ajuste: normalidad de la variable y cribado de los candidatos

      --with-intensity
          Ofrece también las métricas de intensidad a la elección automática (por defecto se dejan fuera: la intensidad de la mayoría de los vuelos no está calibrada entre pasadas)

      --cells <CELLS>...
          Métricas de las celdas (la tabla de texto de `metrics`; una por hoja): escribe el mapa

  -o, --output <OUTPUT>
          El mapa (GeoTIFF); por defecto: junto a la tabla de parcelas

      --interval
          Escribe también la semianchura del intervalo de predicción del 95 %

      --extrapolate
          Estima también las celdas fuera del rango de las parcelas

      --epsg <EPSG>
          Código EPSG del mapa; por defecto: el de la capa de la tabla de celdas

  -h, --help
```

### hipso change

Cambio forestal entre dos fechas a partir de dos modelos de altura de copas sobre las mismas celdas: diferencia de altura, crecimiento, pérdida y cambio de cobertura, manchas de dosel perdido, resumen por rodal y crecimiento de cada árbol

```
Cambio forestal entre dos fechas a partir de dos modelos de altura de copas sobre las mismas celdas: diferencia de altura, crecimiento, pérdida y cambio de cobertura, manchas de dosel perdido, resumen por rodal y crecimiento de cada árbol

Uso: hipso change [OPTIONS] <BEFORE> <AFTER>

Argumentos:
  <BEFORE>
          Modelo de altura de copas de la primera fecha

  <AFTER>
          Modelo de altura de copas de la segunda fecha (mismas celdas; véase `align`)

Opciones:
  -o, --output <OUTPUT>
          Ruta base de los resultados; por defecto: <resultados>/<después>_cambio.shp (nunca un archivo existente)

      --results-dir <RESULTS_DIR>
          Envía a esta carpeta todo producto al que no se dé una salida explícita, en lugar de la carpeta de cada entrada

      --min-change <MIN_CHANGE>
          Un cambio de altura menor que este es ruido
          
          [default: 1]

      --cover-height <COVER_HEIGHT>
          Altura a partir de la cual una celda cuenta como dosel
          
          [default: 2]

      --min-patch <MIN_PATCH>
          Las manchas de dosel perdido menores que tantas celdas no se comunican
          
          [default: 10]

      --years <YEARS>
          Años entre las fechas, para las tasas anuales

      --stands <STANDS>
          Rodales que resumir (.shp o .geojson)

      --stand-field <STAND_FIELD>
          Campo de los rodales que los nombra

      --trees
          Empareja además los árboles de las dos fechas y mide su crecimiento

      --hmin <HMIN>
          Árboles: cima de árbol más baja
          
          [default: 2]

      --ws-a <WS_A>
          Árboles: diámetro de ventana = a + b h (unidades del mapa)
          
          [default: 3]

      --ws-b <WS_B>
          [default: 0.1]

      --smooth <SMOOTH>
          Árboles: suaviza antes los dos modelos con una ventana de este tamaño (0: nada)
          
          [default: 0]

      --match-dist <MATCH_DIST>
          Árboles: es el mismo árbol cuando están más cerca que esto
          
          [default: 3]

      --format <FORMAT>
          Possible values:
          - vector: Solo shapefile
          - csv:    Solo tabla de texto (lo mejor para ejecuciones muy grandes)
          - both:   Both

  -h, --help
```

### hipso corridor

Corredores de líneas eléctricas: conductores reconstruidos a partir de los puntos de cable (clase 14), postes (clase 15) y la vegetación que se acerca demasiado o que podría alcanzar la línea si cayera, con un informe por vano. Se clasifican antes los cables y los postes

```
Corredores de líneas eléctricas: conductores reconstruidos a partir de los puntos de cable (clase 14), postes (clase 15) y la vegetación que se acerca demasiado o que podría alcanzar la línea si cayera, con un informe por vano. Se clasifican antes los cables y los postes

Uso: hipso corridor [OPTIONS] <INPUTS>...

Argumentos:
  <INPUTS>...
          

Opciones:
      --dtm <DTM>
          Modelo del suelo (GeoTIFF o rejilla ASCII); por defecto: el MDT más reciente hecho con la primera nube

      --results-dir <RESULTS_DIR>
          Envía a esta carpeta todo producto al que no se dé una salida explícita, en lugar de la carpeta de cada entrada

  -o, --output <OUTPUT>
          Ruta base de los resultados; por defecto: <resultados>/<entrada>_corredor.shp (nunca un archivo existente)

      --wire-class <WIRE_CLASS>
          Clase de los puntos de cable
          
          [default: 14]

      --pole-class <POLE_CLASS>
          Clase de los puntos de poste
          
          [default: 15]

      --min-distance <MIN_DISTANCE>
          A menos de esta distancia de un conductor (en el espacio) hay invasión
          
          [default: 3]

      --fall-margin <FALL_MARGIN>
          Un árbol cuya altura más este margen alcanza la posición del conductor podría golpearlo
          
          [default: 1]

      --half-width <HALF_WIDTH>
          La vegetación a más de esta distancia de todo conductor no se mira
          
          [default: 30]

      --link <LINK>
          Los puntos de cable más cercanos que esto son un solo conductor
          
          [default: 2.5]

      --gap-max <GAP_MAX>
          Mayor hueco de puntos de cable que se rellena (los mayores cortan el conductor)
          
          [default: 30]

      --min-points <MIN_POINTS>
          Menor número de puntos de un conductor
          
          [default: 30]

      --min-length <MIN_LENGTH>
          Conductor más corto (los más cortos son ruido de la clasificación, p. ej. en copas de árboles)
          
          [default: 25]

      --min-pole-height <MIN_POLE_HEIGHT>
          Lowest pole
          
          [default: 6]

      --cell <CELL>
          Lado de las celdas en las que se agrupa la vegetación
          
          [default: 3]

      --min-height <MIN_HEIGHT>
          La vegetación más baja que esto sobre el suelo se ignora
          
          [default: 2]

      --vegetation <VEGETATION>
          Clases de vegetación
          
          [default: 3,4,5]

      --pole-reach <POLE_REACH>
          Un poste a más de esta distancia de un conductor no es uno de sus apoyos
          
          [default: 8]

      --format <FORMAT>
          Possible values:
          - vector: Solo shapefile
          - csv:    Solo tabla de texto (lo mejor para ejecuciones muy grandes)
          - both:   Both

  -h, --help
```

## Ráster

### hipso stack

Imagen multibanda a partir de rásteres: cada banda de cada entrada pasa a ser una banda de la salida

```
Imagen multibanda a partir de rásteres: cada banda de cada entrada pasa a ser una banda de la salida

Uso: hipso stack [OPTIONS] --output <OUTPUT> <INPUTS>...

Argumentos:
  <INPUTS>...  

Opciones:
  -o, --output <OUTPUT>            
      --results-dir <RESULTS_DIR>  Envía a esta carpeta todo producto al que no se dé una salida explícita, en lugar de la carpeta de cada entrada
      --cell <CELL>                Tamaño de píxel de la salida; por defecto: el de la entrada más fina
      --src-nodata <SRC_NODATA>    Valor de las entradas que significa sin datos
      --nodata <NODATA>            Valor que se escribe donde no hay datos [default: 0]
      --compress <COMPRESS>        Compresión de las teselas: auto (JPEG para imágenes de 8 bits y tres bandas, las ortofotos habituales; deflate en los demás casos), deflate (sin pérdidas) o jpeg (imágenes de 8 bits de 1 o 3 bandas) [default: auto] [possible values: auto, deflate, jpeg]
      --quality <QUALITY>          Calidad JPEG, de 1 a 100 [default: 90]
```

### hipso mosaic

Mosaico de rásteres con las mismas bandas; --overlap dice qué hacer donde se solapan

```
Mosaico de rásteres con las mismas bandas; --overlap dice qué hacer donde se solapan

Uso: hipso mosaic [OPTIONS] --output <OUTPUT> <INPUTS>...

Argumentos:
  <INPUTS>...  

Opciones:
  -o, --output <OUTPUT>            
      --results-dir <RESULTS_DIR>  Envía a esta carpeta todo producto al que no se dé una salida explícita, en lugar de la carpeta de cada entrada
      --overlap <OVERLAP>          [default: first] [possible values: first, last, mean, min, max, median, sum, count]
      --cell <CELL>                
      --src-nodata <SRC_NODATA>    
      --nodata <NODATA>            [default: 0]
      --intersection               La salida cubre solo donde coinciden todas las entradas
      --average                    Promedia los píxeles de origen al reducir (por defecto: el más cercano)
      --compress <COMPRESS>        Compresión de las teselas: auto (JPEG para imágenes de 8 bits y tres bandas, las ortofotos habituales; deflate en los demás casos), deflate (sin pérdidas) o jpeg (imágenes de 8 bits de 1 o 3 bandas) [default: auto] [possible values: auto, deflate, jpeg]
      --quality <QUALITY>          Calidad JPEG, de 1 a 100 [default: 90]
```

### hipso align

Pone rásteres sobre las celdas de un ráster de referencia (mismo origen, celda y tamaño), de modo que coincidan celda a celda. Escribe <resultados>/<nombre>_alineado.tif para cada uno

```
Pone rásteres sobre las celdas de un ráster de referencia (mismo origen, celda y tamaño), de modo que coincidan celda a celda. Escribe <resultados>/<nombre>_alineado.tif para cada uno

Uso: hipso align [OPTIONS] <REFERENCE> <INPUTS>...

Argumentos:
  <REFERENCE>
          El ráster cuyas celdas toman todos los demás

  <INPUTS>...
          Rásteres que alinear

Opciones:
      --resample <RESAMPLE>
          Cómo se trasladan los valores (auto: la media si el ráster es más fino; si no, bilineal)

          Possible values:
          - auto
          - nearest:  Para clases e identificadores
          - bilinear
          - average
          - cubic:    Cubic convolution (smooth surfaces)
          - mode:     Valor más frecuente (clases, al reducir)
          
          [default: auto]

      --results-dir <RESULTS_DIR>
          Envía a esta carpeta todo producto al que no se dé una salida explícita, en lugar de la carpeta de cada entrada

      --nodata <NODATA>
          Lo que contienen las celdas fuera de un ráster
          
          [default: -9999]

      --compress <COMPRESS>
          Compresión de las teselas: auto (JPEG para imágenes de 8 bits y tres bandas, las ortofotos habituales; deflate en los demás casos), deflate (sin pérdidas) o jpeg (imágenes de 8 bits de 1 o 3 bandas)
          
          [default: auto]
          [possible values: auto, deflate, jpeg]

      --quality <QUALITY>
          Calidad JPEG, de 1 a 100
          
          [default: 90]

  -h, --help
```

### hipso reproject

Lleva una nube (LAS/LAZ) o un ráster a otro sistema de coordenadas, p. ej. de ED50 a ETRS89 con la rejilla del IGN español. Las alturas se conservan salvo que --vertical-out pida otra referencia vertical (p. ej. de elipsoidal a ortométrica con el modelo de geoide)

```
Lleva una nube (LAS/LAZ) o un ráster a otro sistema de coordenadas, p. ej. de ED50 a ETRS89 con la rejilla del IGN español. Las alturas se conservan salvo que --vertical-out pida otra referencia vertical (p. ej. de elipsoidal a ortométrica con el modelo de geoide)

Uso: hipso reproject [OPTIONS] --output <OUTPUT> <INPUT>

Argumentos:
  <INPUT>
          

Opciones:
  -o, --output <OUTPUT>
          Archivo de salida (LAS/LAZ, o GeoTIFF para rásteres)

      --results-dir <RESULTS_DIR>
          Envía a esta carpeta todo producto al que no se dé una salida explícita, en lugar de la carpeta de cada entrada

      --to <TO>
          Código EPSG del sistema que se escribe, p. ej. 25830 (no hace falta con --reference)

      --from <FROM>
          Código EPSG en el que está la entrada, cuando no lo declara

      --reference <REFERENCE>
          Rásteres: toma el sistema, la extensión, el origen y la celda de este ráster, de modo que el resultado coincida celda a celda con él

      --resample <RESAMPLE>
          Rásteres: cómo se trasladan los valores (auto: las clases conservan sus valores; si no, la media cuando las celdas crecen y bilineal en caso contrario)

          Possible values:
          - auto
          - nearest:  Para clases e identificadores
          - bilinear
          - average
          - cubic:    Cubic convolution (smooth surfaces)
          - mode:     Valor más frecuente (clases, al reducir)
          
          [default: auto]

      --cell <CELL>
          Rásteres: celda del resultado (por defecto: la de la entrada o la equivalente en el sistema nuevo)

      --nodata <NODATA>
          Lo que contienen las celdas vacías de un ráster cuando se lleva sobre una referencia
          
          [default: -9999]

      --compress <COMPRESS>
          Compresión de las teselas: auto (JPEG para imágenes de 8 bits y tres bandas, las ortofotos habituales; deflate en los demás casos), deflate (sin pérdidas) o jpeg (imágenes de 8 bits de 1 o 3 bandas)
          
          [default: auto]
          [possible values: auto, deflate, jpeg]

      --quality <QUALITY>
          Calidad JPEG, de 1 a 100
          
          [default: 90]

      --vertical-in <VERTICAL_IN>
          Referencia vertical de las alturas de la entrada: `ellipsoidal` o un código EPSG (p. ej. 5782), cuando el archivo no la declara. Hace falta para cambiar las alturas con --vertical-out

      --vertical-out <VERTICAL_OUT>
          Referencia vertical que dar a las alturas (`ellipsoidal` o un código EPSG). Sin ella, las alturas no se tocan

      --allow-ballpark
          Acepta aproximaciones: sin ajuste de datum, alturas conservadas donde ningún geoide cubre el punto (por defecto: las rechaza; también HIPSO_ALLOW_BALLPARK=YES)

      --allow-fallback
          Acepta una operación inferior cuando la mejor no puede ejecutarse, p. ej. por falta de una rejilla (por defecto: la rechaza; también HIPSO_ONLY_BEST=NO)

  -h, --help
```

### hipso cog

Reescribe un ráster como Cloud Optimized GeoTIFF (teselas y vistas reducidas, con todos los directorios al principio del archivo); las imágenes de varias bandas se escriben en flujo, sea cual sea su tamaño

```
Reescribe un ráster como Cloud Optimized GeoTIFF (teselas y vistas reducidas, con todos los directorios al principio del archivo); las imágenes de varias bandas se escriben en flujo, sea cual sea su tamaño

Uso: hipso cog [OPTIONS] <INPUT>

Argumentos:
  <INPUT>  GeoTIFF, JPEG 2000 o rejilla ASCII, de una banda o de varias (ortofotos, imágenes de satélite)

Opciones:
  -o, --output <OUTPUT>            Salida; por defecto: <resultados>/<entrada>_cog.tif (nunca un archivo existente)
      --results-dir <RESULTS_DIR>  Envía a esta carpeta todo producto al que no se dé una salida explícita, en lugar de la carpeta de cada entrada
      --nearest                    Reduce las vistas reducidas tomando una celda en lugar de promediar (clases, identificadores)
      --compress <COMPRESS>        Compresión de las teselas: auto (JPEG para imágenes de 8 bits y tres bandas, las ortofotos habituales; deflate en los demás casos), deflate (sin pérdidas) o jpeg (imágenes de 8 bits de 1 o 3 bandas) [default: auto] [possible values: auto, deflate, jpeg]
      --quality <QUALITY>          Calidad JPEG, de 1 a 100 [default: 90]
```

### hipso reclass

Reclasifica un ráster: cada valor o rango de valores pasa a ser otro. Las reglas son texto, una por línea o separadas por `;`: `10 = 1`, `0..5 = 2`, `< 0 = nodata`, `>= 100 = 4`, `nodata = 0`; decide la primera regla que coincide con una celda

```
Reclasifica un ráster: cada valor o rango de valores pasa a ser otro. Las reglas son texto, una por línea o separadas por `;`: `10 = 1`, `0..5 = 2`, `< 0 = nodata`, `>= 100 = 4`, `nodata = 0`; decide la primera regla que coincide con una celda

Uso: hipso reclass [OPTIONS] <INPUT>

Argumentos:
  <INPUT>  Ráster (GeoTIFF y otros)

Opciones:
      --results-dir <RESULTS_DIR>  Envía a esta carpeta todo producto al que no se dé una salida explícita, en lugar de la carpeta de cada entrada
      --rules <RULES>              Las reglas, como texto (p. ej. "0..10 = 1; 10..50 = 2; else = nodata")
      --rules-file <RULES_FILE>    Un archivo de texto con las reglas, una por línea
      --others <OTHERS>            En qué se convierten las celdas a las que no se aplica ninguna regla: keep (su valor), nodata o un valor [default: keep]
      --band <BAND>                Banda que reclasificar [default: 1]
      --src-nodata <SRC_NODATA>    Valor de la entrada que significa «sin datos», además de lo que declare su archivo
      --nodata <NODATA>            Lo que contienen las celdas vacías del resultado (por defecto: 255, 65535 o -9999 según el tipo elegido)
  -o, --output <OUTPUT>            GeoTIFF; default: <results>/<input>_reclass.tif
      --compress <COMPRESS>        Compresión de las teselas: auto (JPEG para imágenes de 8 bits y tres bandas, las ortofotos habituales; deflate en los demás casos), deflate (sin pérdidas) o jpeg (imágenes de 8 bits de 1 o 3 bandas) [default: auto] [possible values: auto, deflate, jpeg]
      --quality <QUALITY>          Calidad JPEG, de 1 a 100 [default: 90]
```

### hipso clip-raster

Recorta un ráster con una capa de polígonos u otro ráster: lo que queda fuera de la máscara se vacía y, salvo con --no-crop, el resultado se corta a la caja de la máscara

```
Recorta un ráster con una capa de polígonos u otro ráster: lo que queda fuera de la máscara se vacía y, salvo con --no-crop, el resultado se corta a la caja de la máscara

Uso: hipso clip-raster [OPTIONS] --mask <MASK> <INPUT>

Argumentos:
  <INPUT>  Ráster que recortar

Opciones:
      --mask <MASK>                Máscara: una capa de polígonos (shapefile, GeoPackage, GeoJSON, KML) o un ráster (se conservan las celdas con datos distintos de cero)
      --results-dir <RESULTS_DIR>  Envía a esta carpeta todo producto al que no se dé una salida explícita, en lugar de la carpeta de cada entrada
      --no-crop                    No corta a la caja de la máscara: conserva el tamaño de la entrada
      --invert                     Conserva lo que queda fuera de la máscara en lugar de lo que queda dentro
      --nodata <NODATA>            Lo que contienen las celdas descartadas (por defecto: lo del archivo; si no, 0 o -9999 según el tipo)
  -o, --output <OUTPUT>            GeoTIFF; default: <results>/<input>_recorte.tif
      --compress <COMPRESS>        Compresión de las teselas: auto (JPEG para imágenes de 8 bits y tres bandas, las ortofotos habituales; deflate en los demás casos), deflate (sin pérdidas) o jpeg (imágenes de 8 bits de 1 o 3 bandas) [default: auto] [possible values: auto, deflate, jpeg]
      --quality <QUALITY>          Calidad JPEG, de 1 a 100 [default: 90]
```

### hipso sieve

Quita las manchas pequeñas de un ráster de clases: las celdas conectadas de un valor con menos celdas que --threshold toman el valor de su mancha vecina mayor

```
Quita las manchas pequeñas de un ráster de clases: las celdas conectadas de un valor con menos celdas que --threshold toman el valor de su mancha vecina mayor

Uso: hipso sieve [OPTIONS] <INPUT>

Argumentos:
  <INPUT>  Ráster de clases (GeoTIFF)

Opciones:
      --results-dir <RESULTS_DIR>  Envía a esta carpeta todo producto al que no se dé una salida explícita, en lugar de la carpeta de cada entrada
      --threshold <THRESHOLD>      Las manchas con menos celdas que esto se fusionan [default: 2]
      --eight                      Las celdas se tocan por aristas y esquinas (8) en lugar de solo por aristas (4)
  -o, --output <OUTPUT>            GeoTIFF; default: <results>/<input>_sieve.tif
```

### hipso proximity

Distancia de cada celda a la celda más cercana de algunas clases (euclídea, exacta)

```
Distancia de cada celda a la celda más cercana de algunas clases (euclídea, exacta)

Uso: hipso proximity [OPTIONS] <INPUT>

Argumentos:
  <INPUT>  Ráster (GeoTIFF)

Opciones:
      --results-dir <RESULTS_DIR>    Envía a esta carpeta todo producto al que no se dé una salida explícita, en lugar de la carpeta de cada entrada
      --values <VALUES>...           Valores que cuentan como objetivos (por defecto: toda celda distinta de cero)
      --max-distance <MAX_DISTANCE>  Las distancias mayores que esto se dejan vacías
      --cells                        Distancias en celdas en lugar de unidades del mapa
  -o, --output <OUTPUT>              GeoTIFF; default: <results>/<input>_proximity.tif
```

### hipso fill-nodata

Rellena las celdas vacías a partir de las celdas con valor que las rodean (pesos por el inverso de la distancia en ocho direcciones)

```
Rellena las celdas vacías a partir de las celdas con valor que las rodean (pesos por el inverso de la distancia en ocho direcciones)

Uso: hipso fill-nodata [OPTIONS] <INPUT>

Argumentos:
  <INPUT>  Ráster (GeoTIFF)

Opciones:
      --max-distance <MAX_DISTANCE>  Hasta dónde se buscan celdas con valor, en celdas [default: 100]
      --results-dir <RESULTS_DIR>    Envía a esta carpeta todo producto al que no se dé una salida explícita, en lugar de la carpeta de cada entrada
      --smoothing <SMOOTHING>        Pasadas de una media 3x3 sobre las celdas rellenadas [default: 0]
  -o, --output <OUTPUT>              GeoTIFF; default: <results>/<input>_relleno.tif
```

### hipso calc

Calculadora ráster: una expresión sobre rásteres con nombres, celda a celda. Funciones: abs sqrt ln log10 exp pow min max floor ceil round sin cos tan atan2 clamp if isnan; comparaciones, `and`, `or`, `not`, `%`, `^`. `A.2` es la banda 2 de A. Ejemplo: `hipso calc "if(A>0, A, 0)" A=chm.tif -o out.tif`

```
Calculadora ráster: una expresión sobre rásteres con nombres, celda a celda. Funciones: abs sqrt ln log10 exp pow min max floor ceil round sin cos tan atan2 clamp if isnan; comparaciones, `and`, `or`, `not`, `%`, `^`. `A.2` es la banda 2 de A. Ejemplo: `hipso calc "if(A>0, A, 0)" A=chm.tif -o out.tif`

Uso: hipso calc [OPTIONS] <EXPR> <INPUTS>...

Argumentos:
  <EXPR>
          La expresión, entre comillas

  <INPUTS>...
          Los rásteres, como `NOMBRE=archivo.tif` (`A=chm.tif B=otro.tif`); con uno solo, también vale un archivo sin nombre y sus bandas son B1, B2…

Opciones:
  -o, --output <OUTPUT>
          El resultado; por defecto: <resultados>/<primer ráster>_calc.tif

      --results-dir <RESULTS_DIR>
          Envía a esta carpeta todo producto al que no se dé una salida explícita, en lugar de la carpeta de cada entrada

      --dtype <DTYPE>
          Tipo de muestra del resultado
          
          [default: f32]
          [possible values: f32, u8, u16]

      --nearest
          Las vistas reducidas conservan el primer píxel válido en lugar de la media (clases)

      --align <ALIGN>
          Los rásteres cuyas celdas difieren de las del primero se ponen sobre ellas con este remuestreo; sin él se rechazan

          Possible values:
          - auto
          - nearest:  Para clases e identificadores
          - bilinear
          - average
          - cubic:    Cubic convolution (smooth surfaces)
          - mode:     Valor más frecuente (clases, al reducir)

  -h, --help
```

### hipso vectorize

Vectoriza un ráster: las celdas que comparten una clase, caen en un rango o están entre dos cortes pasan a ser polígonos con el valor, el área y el número de celdas como atributos

```
Vectoriza un ráster: las celdas que comparten una clase, caen en un rango o están entre dos cortes pasan a ser polígonos con el valor, el área y el número de celdas como atributos

Uso: hipso vectorize [OPTIONS] <INPUT>

Argumentos:
  <INPUT>  El ráster (un GeoTIFF)

Opciones:
  -o, --output <OUTPUT>            Los polígonos: `.shp` o `.gpkg`; por defecto: <resultados>/<entrada>_poligonos.shp
      --results-dir <RESULTS_DIR>  Envía a esta carpeta todo producto al que no se dé una salida explícita, en lugar de la carpeta de cada entrada
      --range <RANGE> <RANGE>      Agrupa por rango: `bajo,alto`, ambos incluidos; el resto se deja fuera
      --breaks <BREAKS>            Agrupa por los intervalos entre estos cortes, de menor a mayor: `0,5,10`
      --classes                    Agrupa por clase (valores enteros iguales); es lo habitual
      --min-cells <MIN_CELLS>      Descarta las partes con menos celdas que esto [default: 1]
      --simplify <SIMPLIFY>        Simplifica los límites (unidades del mapa); 0 conserva los escalones de las celdas [default: 0]
      --diagonal                   Las celdas que se tocan por una esquina son una sola parte
```

### hipso rasterize

Rasteriza una capa: cada celda recibe el valor de un campo del último elemento que la cubre (o 1)

```
Rasteriza una capa: cada celda recibe el valor de un campo del último elemento que la cubre (o 1)

Uso: hipso rasterize [OPTIONS] <LAYER>

Argumentos:
  <LAYER>  La capa

Opciones:
      --reference <REFERENCE>      Ráster de referencia: toma sus celdas
      --results-dir <RESULTS_DIR>  Envía a esta carpeta todo producto al que no se dé una salida explícita, en lugar de la carpeta de cada entrada
      --cell <CELL>                Tamaño de celda cuando no hay referencia
      --field <FIELD>              Campo con el valor que se pinta (sin él, 1)
  -o, --output <OUTPUT>            GeoTIFF de salida; por defecto: <resultados>/<capa>_raster.tif
```

### hipso focal

Filtro focal de un ráster: media, mediana, mínimo, máximo, suma, rango, desviación o mayoría sobre una ventana

```
Filtro focal de un ráster: media, mediana, mínimo, máximo, suma, rango, desviación o mayoría sobre una ventana

Uso: hipso focal [OPTIONS] <INPUT>

Argumentos:
  <INPUT>  El ráster (GeoTIFF)

Opciones:
      --results-dir <RESULTS_DIR>  Envía a esta carpeta todo producto al que no se dé una salida explícita, en lugar de la carpeta de cada entrada
      --stat <STAT>                mean, median, min, max, sum, range, std, majority o gaussian [default: mean]
      --radius <RADIUS>            Radio de la ventana en celdas [default: 1]
      --circle                     Ventana circular en lugar de cuadrada
      --sigma <SIGMA>              Desviación típica del filtro gaussiano, en celdas (por defecto, la mitad del radio)
      --min-valid <MIN_VALID>      Menos celdas con valor que esto dejan la celda vacía [default: 1]
  -o, --output <OUTPUT>            GeoTIFF de salida; por defecto: <resultados>/<entrada>_focal.tif
```

### hipso rstats

Estadísticas de un ráster (recuento, mínimo, máximo, media, desviación, mediana, percentiles) y su histograma

```
Estadísticas de un ráster (recuento, mínimo, máximo, media, desviación, mediana, percentiles) y su histograma

Uso: hipso rstats [OPTIONS] <INPUT>

Argumentos:
  <INPUT>  El ráster (GeoTIFF)

Opciones:
      --bins <BINS>                Intervalos del histograma [default: 20]
      --results-dir <RESULTS_DIR>  Envía a esta carpeta todo producto al que no se dé una salida explícita, en lugar de la carpeta de cada entrada
  -o, --output <OUTPUT>            CSV del histograma; por defecto: <resultados>/<entrada>_histograma.csv
```

### hipso tabulate

Área de cada clase de un ráster dentro de cada zona de otro (tabular área), como CSV

```
Área de cada clase de un ráster dentro de cada zona de otro (tabular área), como CSV

Uso: hipso tabulate [OPTIONS] --values <VALUES> <ZONES>

Argumentos:
  <ZONES>  El ráster de zonas (clases enteras)

Opciones:
      --results-dir <RESULTS_DIR>  Envía a esta carpeta todo producto al que no se dé una salida explícita, en lugar de la carpeta de cada entrada
      --values <VALUES>            El ráster de clases que se mide dentro de cada zona
  -o, --output <OUTPUT>            CSV de salida; por defecto: <resultados>/<zonas>_tabulado.csv
```

### hipso zonal

Estadísticas de un ráster (un CHM, un índice, un MDT) dentro de cada polígono. El resultado es una capa de polígonos (shapefile) con los atributos propios de los polígonos más un campo nuevo por estadístico: count, min, max, mean, sum, stddev, median. Una celda cuenta cuando su centro está en el polígono

```
Estadísticas de un ráster (un CHM, un índice, un MDT) dentro de cada polígono. El resultado es una capa de polígonos (shapefile) con los atributos propios de los polígonos más un campo nuevo por estadístico: count, min, max, mean, sum, stddev, median. Una celda cuenta cuando su centro está en el polígono

Uso: hipso zonal [OPTIONS] --regions <REGIONS> <RASTER>

Argumentos:
  <RASTER>  El ráster (GeoTIFF)

Opciones:
      --regions <REGIONS>          Los polígonos (.shp, .geojson, .gpkg), en el sistema del ráster
      --results-dir <RESULTS_DIR>  Envía a esta carpeta todo producto al que no se dé una salida explícita, en lugar de la carpeta de cada entrada
      --prefix <PREFIX>            Prefijo de los campos nuevos (un campo de shapefile tiene como mucho 10 caracteres: `chm_` deja 6 para el estadístico, así que caben `stddev` y `median`) [default: ""]
      --split-parts                Divide los elementos de varias partes en un polígono por parte
  -o, --output <OUTPUT>            Shapefile; por defecto: <resultados>/<ráster>_zonal.shp
```

## Terreno e hidrología

### hipso terrain

Análisis de terreno de un MDT (GeoTIFF): pendiente, orientación, sombreado, curvaturas, rugosidad, TRI, TPI, depresiones, flujo acumulado, índice de humedad, cauces, subcuencas, cuenca visual

```
Análisis de terreno de un MDT (GeoTIFF): pendiente, orientación, sombreado, curvaturas, rugosidad, TRI, TPI, depresiones, flujo acumulado, índice de humedad, cauces, subcuencas, cuenca visual

Uso: hipso terrain [OPTIONS] --product <PRODUCT> <INPUT>

Argumentos:
  <INPUT>  El MDT (un GeoTIFF en un sistema proyectado, celda en metros)

Opciones:
      --product <PRODUCT>            [possible values: slope, aspect, hillshade, plan-curvature, profile-curvature, curvature, roughness, tri, tpi, fill-depth, flow-accumulation, twi, streams, subbasins, viewshed]
      --results-dir <RESULTS_DIR>    Envía a esta carpeta todo producto al que no se dé una salida explícita, en lugar de la carpeta de cada entrada
  -o, --output <OUTPUT>              GeoTIFF; default: <results>/<input>_<product>.tif
      --percent                      Pendiente en porcentaje en lugar de grados
      --edges <EDGES>                Ventanas sobre el borde del ráster o una celda vacía (productos 3x3): `nodata` (por defecto, sin valor), `repeat` (repite las celdas del borde; sin valor alrededor de los huecos) o `center` (los vecinos que faltan toman la altura del centro) [default: nodata]
      --z-factor <Z_FACTOR>          Multiplica las alturas cuando no están en las unidades de la celda [default: 1]
      --azimuth <AZIMUTH>            Sombreado: dirección de la luz en grados en el sentido de las agujas del reloj desde el norte [default: 315]
      --altitude <ALTITUDE>          Sombreado: altura de la luz sobre el horizonte en grados [default: 45]
      --radius <RADIUS>              TPI: radio del entorno (unidades del mapa) [default: 10]
      --threshold <THRESHOLD>        Cauces y subcuencas: superficie drenada (m²) que forma un cauce [default: 10000]
      --observer <X> <Y>             Cuenca visual: la X y la Y del observador
      --eye <EYE>                    Cuenca visual: altura del ojo sobre el suelo (m) [default: 1.7]
      --target <TARGET>              Cuenca visual: altura sobre el suelo que debe verse (m) [default: 0]
      --max-distance <MAX_DISTANCE>  Cuenca visual: hasta dónde mira el observador (unidades del mapa) [default: 1000]
```

### hipso ponds

Charcas: donde queda agua en un MDT. El modelo se rellena hasta que drena, y cada mancha conectada de agua más profunda que el mínimo es una charca, medida por área, profundidad, volumen, nivel de desborde y superficie drenada

```
Charcas: donde queda agua en un MDT. El modelo se rellena hasta que drena, y cada mancha conectada de agua más profunda que el mínimo es una charca, medida por área, profundidad, volumen, nivel de desborde y superficie drenada

Uso: hipso ponds [OPTIONS] <INPUT>

Argumentos:
  <INPUT>  El MDT (un GeoTIFF en un sistema proyectado, celda en metros)

Opciones:
  -o, --output <OUTPUT>                Ruta base de los resultados; por defecto: <resultados>/<entrada>_charcos.shp
      --results-dir <RESULTS_DIR>      Envía a esta carpeta todo producto al que no se dé una salida explícita, en lugar de la carpeta de cada entrada
      --method <METHOD>                `depression` (agua retenida hasta el nivel de desborde de cada depresión) o `local` (puntos bajos frente al modelo suavizado; la pendiente y la superficie drenada se comprueban celda a celda) [default: depression]
      --smooth <SMOOTH>                Suaviza antes el MDT (gaussiano, desviación típica en unidades del mapa; 0: no) [default: 0]
      --nodata <NODATA>                El valor que significa sin datos cuando el MDT no declara ninguno
      --min-depth <MIN_DEPTH>          El agua menos profunda no es una charca (unidades del MDT) [default: 0.05]
      --min-area <MIN_AREA>            Descarta las charcas menores que esto (m²) [default: 0]
      --max-area <MAX_AREA>            Descarta las charcas mayores que esto (m²): los embalses no son charcos
      --max-depth <MAX_DEPTH>          Descarta las charcas más profundas que esto (unidades del MDT): los hoyos y las zanjas no son charcos
      --min-volume <MIN_VOLUME>        Descarta las charcas que retienen menos que esto (m³) [default: 0]
      --max-slope <MAX_SLOPE>          Descarta las charcas en suelo con una pendiente media mayor que esta (grados)
      --min-catchment <MIN_CATCHMENT>  Descarta las charcas que drenan menos superficie que esta (m²)
      --depth-raster                   Escribe también la profundidad del agua como ráster
      --gpkg                           Escribe además un GeoPackage
```

### hipso flood

Simulación de inundación sobre un MDT: el área, el volumen y la profundidad del agua a uno o varios niveles, desde unas semillas. Con conectividad, un hoyo detrás de una cresta queda seco

```
Simulación de inundación sobre un MDT: el área, el volumen y la profundidad del agua a uno o varios niveles, desde unas semillas. Con conectividad, un hoyo detrás de una cresta queda seco

Uso: hipso flood [OPTIONS] <INPUT>

Argumentos:
  <INPUT>
          El MDT (un GeoTIFF en un sistema proyectado)

Opciones:
      --results-dir <RESULTS_DIR>
          Envía a esta carpeta todo producto al que no se dé una salida explícita, en lugar de la carpeta de cada entrada

      --seed <X,Y>
          Una semilla del agua, `x,y`; se repite para varias. Sin semillas, `--border`

      --border
          El agua viene del borde del modelo (y de junto a las celdas vacías)

      --bathtub
          El modelo de bañera: todo lo que queda bajo el nivel se inunda, esté conectado o no

      --level <Z>
          Un nivel del agua, se repite para varios

      --levels <A:B:STEP>
          Una serie de niveles `desde:hasta:paso`

      --relative <RELATIVE>
          Los niveles son alturas sobre `min` (la celda más baja) o sobre la `seed`; por defecto: elevaciones

          Possible values:
          - min:  Sobre la celda más baja del modelo
          - seed: Sobre la elevación de la primera semilla

      --connectivity <CONNECTIVITY>
          4 u 8 vecinos dejan pasar el agua
          
          [default: 8]

      --extent
          Una capa de polígonos con la superficie inundada de cada nivel

      --no-depth
          Sin ráster de profundidad por nivel

      --no-csv
          Sin tabla CSV

  -o, --output <OUTPUT>
          Ruta base de los resultados; por defecto: <resultados>/<entrada>_inundacion

  -h, --help
```

### hipso ridges

Líneas de cresta o de drenaje trazadas a partir de un MDT, como líneas 3D con orden de Strahler, longitud y pendiente media

```
Líneas de cresta o de drenaje trazadas a partir de un MDT, como líneas 3D con orden de Strahler, longitud y pendiente media

Uso: hipso ridges [OPTIONS] <INPUT>

Argumentos:
  <INPUT>  El MDT (un GeoTIFF en un sistema proyectado)

Opciones:
      --drainage                   Traza las líneas de drenaje en lugar de las crestas
      --results-dir <RESULTS_DIR>  Envía a esta carpeta todo producto al que no se dé una salida explícita, en lugar de la carpeta de cada entrada
      --min-area <MIN_AREA>        Superficie drenada que forma una línea (m2) [default: 1000]
      --min-length <MIN_LENGTH>    Descarta las líneas más cortas que esto (m) [default: 0]
      --smooth <SMOOTH>            Pasadas de suavizado (Chaikin); 0 conserva las celdas [default: 0]
  -o, --output <OUTPUT>            Las líneas; por defecto: <resultados>/<entrada>_crestas.shp (o _drenaje)
```

### hipso leastcost

Ruta de menor coste sobre un MDT, desde un origen hasta un destino con paradas opcionales

```
Ruta de menor coste sobre un MDT, desde un origen hasta un destino con paradas opcionales

Uso: hipso leastcost [OPTIONS] --from <X,Y> --to <X,Y> <INPUT>

Argumentos:
  <INPUT>  El MDT (un GeoTIFF en un sistema proyectado)

Opciones:
      --from <X,Y>
          El origen `x,y`
      --results-dir <RESULTS_DIR>
          Envía a esta carpeta todo producto al que no se dé una salida explícita, en lugar de la carpeta de cada entrada
      --to <X,Y>
          El destino `x,y`; se repite con --nearest para ir al más barato de varios
      --via <X,Y>
          Una parada en el camino `x,y`, en orden
      --model <MODEL>
          `slope`, `tobler`, `uniform` o `raster:<tif>` (coste por metro en las celdas del MDT) [default: slope]
      --power <POWER>
          Modelo de pendiente: exponente y pendiente (%) donde el coste por metro se duplica [default: 2]
      --reference-slope <REFERENCE_SLOPE>
          [default: 10]
      --max-slope <MAX_SLOPE>
          Las celdas más empinadas que esto (%) no se pueden cruzar
      --barriers <BARRIERS>
          Polígonos que no se pueden cruzar
      --prefer <PREFER>
          Polígonos donde el coste se multiplica por --prefer-factor
      --prefer-factor <PREFER_FACTOR>
          [default: 0.5]
      --nearest
          Va al más barato de los destinos
      --cost-distance
          Escribe también un ráster con el coste de llegar a cada celda desde el origen
  -o, --output <OUTPUT>
          La ruta; por defecto: <resultados>/<entrada>_ruta.shp
  -h, --help
```

### hipso piles

Volúmenes de acopios: el material sobre una superficie base dentro del contorno de cada pila. La base se hace para cada pila (un nivel, la altura más baja o mediana de su contorno, un plano por el contorno, una superficie interpolada a partir de él o un modelo del terreno del sitio antes)

```
Volúmenes de acopios: el material sobre una superficie base dentro del contorno de cada pila. La base se hace para cada pila (un nivel, la altura más baja o mediana de su contorno, un plano por el contorno, una superficie interpolada a partir de él o un modelo del terreno del sitio antes)

Uso: hipso piles [OPTIONS] --piles <PILES> <SURFACE>

Argumentos:
  <SURFACE>  El modelo de superficie (un GeoTIFF, MDS)

Opciones:
      --piles <PILES>              Los contornos de las pilas (una capa de polígonos)
      --results-dir <RESULTS_DIR>  Envía a esta carpeta todo producto al que no se dé una salida explícita, en lugar de la carpeta de cada entrada
      --base <BASE>                `plane`, `lowest`, `median`, `interpolated`, `level:<z>` o `dtm:<archivo>` [default: plane]
      --min-height <MIN_HEIGHT>    Las alturas más cercanas que esto a la base no cuentan (m) [default: 0]
      --density <DENSITY>          Densidad del material (t/m3): la capa y la tabla reciben una masa
  -o, --output <OUTPUT>            Ruta base de los resultados; por defecto: <resultados>/<superficie>_pilas
      --no-csv                     Sin tabla CSV
      --base-raster                Escribe también un ráster con la superficie base
      --height-raster              Escribe también un ráster con la altura sobre la base
```

### hipso cutfill

Desmonte y terraplén entre un terreno existente y un diseño: dz = diseño - existente; positivo es terraplén (tierra que añadir), negativo es desmonte (tierra que quitar)

```
Desmonte y terraplén entre un terreno existente y un diseño: dz = diseño - existente; positivo es terraplén (tierra que añadir), negativo es desmonte (tierra que quitar)

Uso: hipso cutfill [OPTIONS] --design <DESIGN> <EXISTING>

Argumentos:
  <EXISTING>  El terreno existente (un GeoTIFF)

Opciones:
      --design <DESIGN>            El diseño: un archivo ráster, `level:<z>` o `plane:<z0>,<pendiente %>,<orientación grados>,<x>,<y>` (elevación z0 en x,y, bajando hacia la orientación)
      --results-dir <RESULTS_DIR>  Envía a esta carpeta todo producto al que no se dé una salida explícita, en lugar de la carpeta de cada entrada
      --regions <REGIONS>          Polígonos que medir uno a uno (el ráster entero se mide siempre también)
      --tolerance <TOLERANCE>      Las diferencias de hasta este tamaño cuentan como sin cambio (m) [default: 0.05]
  -o, --output <OUTPUT>            Ruta base de los resultados; por defecto: <resultados>/<existente>_cutfill
      --no-csv                     Sin tabla CSV
      --classified                 Escribe también un ráster de números enteros: 1 desmonte, 2 terraplén, 0 sin cambio
```

## Vectorial

### hipso is-valid

Comprueba que los polígonos de una capa son válidos (anillos cerrados, sin autointersecciones, huecos dentro, sin solapes entre partes) y lista los que no lo son

```
Comprueba que los polígonos de una capa son válidos (anillos cerrados, sin autointersecciones, huecos dentro, sin solapes entre partes) y lista los que no lo son

Uso: hipso is-valid [OPTIONS] <INPUT>

Argumentos:
  <INPUT>  Capa de polígonos (.shp, .geojson, .gpkg)

Opciones:
      --results-dir <RESULTS_DIR>  Envía a esta carpeta todo producto al que no se dé una salida explícita, en lugar de la carpeta de cada entrada
```

### hipso make-valid

Repara polígonos inválidos (regla par-impar: un lazo pasa a ser dos partes y los bordes duplicados se cancelan). Los válidos no se tocan; los atributos se conservan

```
Repara polígonos inválidos (regla par-impar: un lazo pasa a ser dos partes y los bordes duplicados se cancelan). Los válidos no se tocan; los atributos se conservan

Uso: hipso make-valid [OPTIONS] <INPUT>

Argumentos:
  <INPUT>  

Opciones:
  -o, --output <OUTPUT>            Shapefile; default: <results>/<input>_valido.shp
      --results-dir <RESULTS_DIR>  Envía a esta carpeta todo producto al que no se dé una salida explícita, en lugar de la carpeta de cada entrada
```

### hipso buffer

Amplía (o, si es negativa, reduce) cada polígono una distancia en unidades del mapa, con esquinas redondeadas

```
Amplía (o, si es negativa, reduce) cada polígono una distancia en unidades del mapa, con esquinas redondeadas

Uso: hipso buffer [OPTIONS] --distance <DISTANCE> <INPUT>

Argumentos:
  <INPUT>  

Opciones:
      --distance <DISTANCE>        Distancia en las unidades de la capa (metros en un sistema proyectado; grados si es geográfico)
      --results-dir <RESULTS_DIR>  Envía a esta carpeta todo producto al que no se dé una salida explícita, en lugar de la carpeta de cada entrada
      --make-valid                 Repara antes los polígonos inválidos
  -o, --output <OUTPUT>            Shapefile; default: <results>/<input>_buffer.shp
```

### hipso dissolve

Une polígonos: los que comparten el valor de un campo pasan a ser un solo elemento (con el número de unidos en `n`); sin campo, toda la capa pasa a ser uno

```
Une polígonos: los que comparten el valor de un campo pasan a ser un solo elemento (con el número de unidos en `n`); sin campo, toda la capa pasa a ser uno

Uso: hipso dissolve [OPTIONS] <INPUT>

Argumentos:
  <INPUT>  

Opciones:
      --field <FIELD>              Campo por el que disolver
      --results-dir <RESULTS_DIR>  Envía a esta carpeta todo producto al que no se dé una salida explícita, en lugar de la carpeta de cada entrada
      --make-valid                 Repara antes los polígonos inválidos
  -o, --output <OUTPUT>            Shapefile; default: <results>/<input>_disuelto.shp
```

### hipso union

Unión de dos capas de polígonos como superposición: las piezas que están en las dos (atributos de ambas, los de la segunda con el prefijo `b_`), solo en la primera y solo en la segunda

```
Unión de dos capas de polígonos como superposición: las piezas que están en las dos (atributos de ambas, los de la segunda con el prefijo `b_`), solo en la primera y solo en la segunda

Uso: hipso union [OPTIONS] <FIRST> <SECOND>

Argumentos:
  <FIRST>   
  <SECOND>  

Opciones:
      --make-valid                 Repara antes los polígonos inválidos
      --results-dir <RESULTS_DIR>  Envía a esta carpeta todo producto al que no se dé una salida explícita, en lugar de la carpeta de cada entrada
  -o, --output <OUTPUT>            Shapefile; por defecto: <resultados>/<primera>_union.shp
```

### hipso intersect

Intersección de dos capas de polígonos: solo las piezas que están en las dos (atributos de ambas, los de la segunda con el prefijo `b_`)

```
Intersección de dos capas de polígonos: solo las piezas que están en las dos (atributos de ambas, los de la segunda con el prefijo `b_`)

Uso: hipso intersect [OPTIONS] <FIRST> <SECOND>

Argumentos:
  <FIRST>   
  <SECOND>  

Opciones:
      --make-valid                 Repara antes los polígonos inválidos
      --results-dir <RESULTS_DIR>  Envía a esta carpeta todo producto al que no se dé una salida explícita, en lugar de la carpeta de cada entrada
  -o, --output <OUTPUT>            Shapefile; por defecto: <resultados>/<primera>_interseccion.shp
```

### hipso voronoi

Polígonos de Voronoi (y, con --delaunay, las aristas de Delaunay) de una capa de puntos, con los atributos de cada punto

```
Polígonos de Voronoi (y, con --delaunay, las aristas de Delaunay) de una capa de puntos, con los atributos de cada punto

Uso: hipso voronoi [OPTIONS] <INPUT>

Argumentos:
  <INPUT>  Una capa de puntos

Opciones:
      --clip <CLIP>                Las celdas se cortan por `bbox` (la caja de los puntos, algo mayor) o por una capa de polígonos; sin ello, se dejan fuera las celdas de los puntos del borde
      --results-dir <RESULTS_DIR>  Envía a esta carpeta todo producto al que no se dé una salida explícita, en lugar de la carpeta de cada entrada
      --delaunay                   Escribe también las aristas de la triangulación de Delaunay
  -o, --output <OUTPUT>            Los polígonos: `.shp`, `.gpkg` o `.geojson`; por defecto: <resultados>/<entrada>_voronoi.shp
```

### hipso voronoi-density

Densidad de puntos por celdas de Voronoi, para comprobar una nube: n / suma de las áreas de las celdas de los puntos de cada celda ráster, con la proporción de celdas que alcanzan un umbral

```
Densidad de puntos por celdas de Voronoi, para comprobar una nube: n / suma de las áreas de las celdas de los puntos de cada celda ráster, con la proporción de celdas que alcanzan un umbral

Uso: hipso voronoi-density [OPTIONS] <INPUTS>...

Argumentos:
  <INPUTS>...  

Opciones:
      --cell <CELL>                    Celda del ráster de densidad (m) [default: 5]
      --results-dir <RESULTS_DIR>      Envía a esta carpeta todo producto al que no se dé una salida explícita, en lugar de la carpeta de cada entrada
      --threshold <THRESHOLD>          Puntos por m2 que debe alcanzar la densidad [default: 8]
      --returns <RETURNS>              Retornos que se usan: A, F, L o números; por defecto F (primeros retornos: pulsos) [default: F]
      --class <CLASS>                  Clases que se usan: 2,5 o ~7,18
      --max-cell-area <MAX_CELL_AREA>  Una celda de Voronoi mayor que esto (m2) es un hueco en los datos [default: 100]
      --no-pass-fail                   Sin ráster de 1 donde se alcanza el umbral y 0 donde no
      --no-csv                         Sin tabla CSV
      --tile <TILE>                    Lado de las teselas y su margen (m) [default: 250]
      --buffer <BUFFER>                [default: 20]
  -o, --output <OUTPUT>                Ruta base de los resultados; por defecto: <resultados>/<primera nube>_vdensidad
```

### hipso extract

Extrae los valores de uno o varios rásteres a una capa: un campo nuevo por banda con el valor bajo cada elemento (en el punto o en un punto interior de la línea o del polígono)

```
Extrae los valores de uno o varios rásteres a una capa: un campo nuevo por banda con el valor bajo cada elemento (en el punto o en un punto interior de la línea o del polígono)

Uso: hipso extract [OPTIONS] --raster <RASTER> <LAYER>

Argumentos:
  <LAYER>  La capa (puntos, líneas o polígonos), en el mismo sistema que los rásteres

Opciones:
      --raster <RASTER>            Un ráster (se repite para varios)
      --results-dir <RESULTS_DIR>  Envía a esta carpeta todo producto al que no se dé una salida explícita, en lugar de la carpeta de cada entrada
      --bands <BANDS>              Bandas que extraer, contando desde 1 (por defecto, todas)
      --bilinear                   Interpola entre los cuatro centros de celda más cercanos en lugar de tomar la celda que contiene el punto
      --prefix <PREFIX>            Prefijo de los campos nuevos [default: ""]
  -o, --output <OUTPUT>            Capa de salida; por defecto: <resultados>/<capa>_valores.<ext>
```

### hipso join-table

Une a una capa las columnas de una tabla (csv, texto o xlsx) por un campo clave

```
Une a una capa las columnas de una tabla (csv, texto o xlsx) por un campo clave

Uso: hipso join-table [OPTIONS] --layer-key <LAYER_KEY> <LAYER> <TABLE>

Argumentos:
  <LAYER>  La capa
  <TABLE>  La tabla

Opciones:
      --layer-key <LAYER_KEY>      Campo clave de la capa
      --results-dir <RESULTS_DIR>  Envía a esta carpeta todo producto al que no se dé una salida explícita, en lugar de la carpeta de cada entrada
      --table-key <TABLE_KEY>      Columna clave de la tabla (por defecto, el mismo nombre)
      --prefix <PREFIX>            Prefijo de los campos nuevos [default: ""]
  -o, --output <OUTPUT>            Capa de salida; por defecto: <resultados>/<capa>_union.<ext>
```

### hipso merge

Combina capas del mismo tipo de geometría en una (los campos son la unión de los de todas)

```
Combina capas del mismo tipo de geometría en una (los campos son la unión de los de todas)

Uso: hipso merge [OPTIONS] <INPUTS>...

Argumentos:
  <INPUTS>...  Las capas que combinar

Opciones:
      --origin <ORIGIN>            Añade un campo con el nombre del archivo de origen de cada elemento
      --results-dir <RESULTS_DIR>  Envía a esta carpeta todo producto al que no se dé una salida explícita, en lugar de la carpeta de cada entrada
  -o, --output <OUTPUT>            Capa de salida; por defecto: <resultados>/<primera>_combinada.<ext>
```

### hipso split

Divide una capa en una por cada valor distinto de un campo (un archivo por rodal, por especie...)

```
Divide una capa en una por cada valor distinto de un campo (un archivo por rodal, por especie...)

Uso: hipso split [OPTIONS] --field <FIELD> <INPUT>

Argumentos:
  <INPUT>  La capa

Opciones:
      --field <FIELD>              El campo por el que dividir
      --results-dir <RESULTS_DIR>  Envía a esta carpeta todo producto al que no se dé una salida explícita, en lugar de la carpeta de cada entrada
      --dir <DIR>                  Carpeta de salida; por defecto: <resultados>/<capa>_partes
```

### hipso clip-vector

Recorta una capa de puntos, líneas o polígonos con los polígonos de otra (con --erase, borra lo que cubren)

```
Recorta una capa de puntos, líneas o polígonos con los polígonos de otra (con --erase, borra lo que cubren)

Uso: hipso clip-vector [OPTIONS] --mask <MASK> <INPUT>

Argumentos:
  <INPUT>  La capa que recortar

Opciones:
      --mask <MASK>                Los polígonos con los que recortar
      --results-dir <RESULTS_DIR>  Envía a esta carpeta todo producto al que no se dé una salida explícita, en lugar de la carpeta de cada entrada
      --erase                      Borra lo que cubren los polígonos en lugar de conservarlo
  -o, --output <OUTPUT>            Capa de salida; por defecto: <resultados>/<capa>_recortada.<ext> (o _borrada)
```

### hipso near

Distancia de cada elemento al elemento más cercano de otra capa, con su número de orden y, si se pide, uno de sus campos

```
Distancia de cada elemento al elemento más cercano de otra capa, con su número de orden y, si se pide, uno de sus campos

Uso: hipso near [OPTIONS] --to <TO> <INPUT>

Argumentos:
  <INPUT>  La capa

Opciones:
      --results-dir <RESULTS_DIR>  Envía a esta carpeta todo producto al que no se dé una salida explícita, en lugar de la carpeta de cada entrada
      --to <TO>                    La capa de la que se busca el elemento más cercano
      --carry <CARRY>              Campo de la otra capa que se copia del elemento más cercano
      --max <MAX>                  Distancia máxima: los más lejanos quedan sin valor
  -o, --output <OUTPUT>            Capa de salida; por defecto: <resultados>/<capa>_cercania.<ext>
```

### hipso summary

Estadísticas de resumen de campos numéricos de una capa o tabla, por grupos de un campo (recuento, suma, media, mínimo, máximo, desviación)

```
Estadísticas de resumen de campos numéricos de una capa o tabla, por grupos de un campo (recuento, suma, media, mínimo, máximo, desviación)

Uso: hipso summary [OPTIONS] <INPUT>

Argumentos:
  <INPUT>  La capa o la tabla (csv, texto, xlsx)

Opciones:
      --by <BY>                    Campo por el que agrupar (sin él, un solo grupo)
      --results-dir <RESULTS_DIR>  Envía a esta carpeta todo producto al que no se dé una salida explícita, en lugar de la carpeta de cada entrada
      --fields <FIELDS>            Campos numéricos que resumir, separados por comas (por defecto, todos los numéricos)
  -o, --output <OUTPUT>            Tabla CSV de salida; por defecto: <resultados>/<entrada>_resumen.csv
```

### hipso topology

Comprueba la topología de una capa: polígonos que se solapan, huecos entre polígonos y contornos que se cruzan consigo mismos; en líneas, cabos sueltos

```
Comprueba la topología de una capa: polígonos que se solapan, huecos entre polígonos y contornos que se cruzan consigo mismos; en líneas, cabos sueltos

Uso: hipso topology [OPTIONS] <INPUT>

Argumentos:
  <INPUT>  La capa de polígonos o de líneas

Opciones:
      --results-dir <RESULTS_DIR>  Envía a esta carpeta todo producto al que no se dé una salida explícita, en lugar de la carpeta de cada entrada
      --tolerance <TOLERANCE>      Tolerancia: superficie mínima de un solape o un hueco, o distancia a la que dos extremos de línea se tienen por unidos [default: 0.000001]
  -o, --output <OUTPUT>            Tabla CSV del informe; por defecto: <resultados>/<capa>_topologia.csv
```

### hipso geojson

Convierte una capa vectorial entre shapefile y GeoJSON (según la extensión de la entrada). El GeoJSON recibe el sistema de coordenadas del shapefile como miembro `crs`

```
Convierte una capa vectorial entre shapefile y GeoJSON (según la extensión de la entrada). El GeoJSON recibe el sistema de coordenadas del shapefile como miembro `crs`

Uso: hipso geojson [OPTIONS] <INPUT>

Argumentos:
  <INPUT>  Un .shp (da un .geojson) o un .geojson (da un .shp)

Opciones:
  -o, --output <OUTPUT>            Salida; por defecto: <resultados>/<entrada>.geojson o .shp (nunca un archivo existente)
      --results-dir <RESULTS_DIR>  Envía a esta carpeta todo producto al que no se dé una salida explícita, en lugar de la carpeta de cada entrada
```

## Salida y comprobación de nubes

### hipso export

Extrae puntos de LAS/LAZ a LAS/LAZ, a columnas ASCII o a un shapefile de puntos 3D

```
Extrae puntos de LAS/LAZ a LAS/LAZ, a columnas ASCII o a un shapefile de puntos 3D

Uso: hipso export [OPTIONS] --output <OUTPUT> <INPUTS>...

Argumentos:
  <INPUTS>...  

Opciones:
  -o, --output <OUTPUT>            Archivo (o, con varias entradas, una carpeta). .laz/.las escriben una nube, .shp un shapefile PointZ y cualquier otro, texto
      --results-dir <RESULTS_DIR>  Envía a esta carpeta todo producto al que no se dé una salida explícita, en lugar de la carpeta de cada entrada
      --normalize                  Exporta alturas sobre el suelo: cada nube menos su MDT
      --dtm <DTM>                  MDT con el que normalizar (una entrada); por defecto: el MDT más reciente hecho con cada nube
      --items <ITEMS>              Columnas en orden: x,y,z,intensity,return,returns,class,source,red,green,blue [default: x y z]
      --delimiter <DELIMITER>      [default: ,]
      --header                     
      --decimals <DECIMALS>        [default: 2]
      --class <CLASS>              
      --returns <RETURNS>          
      --zmin <ZMIN>                
      --zmax <ZMAX>                
      --decimate <DECIMATE>        Conserva un punto de cada tantos [default: 1]
```

### hipso copc

Escribe las nubes como COPC (Cloud Optimized Point Cloud): un LAZ cuyos bloques son los nodos de un octree, para visores y servidores que leen solo la parte y el detalle que necesitan

```
Escribe las nubes como COPC (Cloud Optimized Point Cloud): un LAZ cuyos bloques son los nodos de un octree, para visores y servidores que leen solo la parte y el detalle que necesitan

Uso: hipso copc [OPTIONS] <INPUTS>...

Argumentos:
  <INPUTS>...  

Opciones:
  -o, --output <OUTPUT>            Salida (solo una entrada); por defecto: <resultados>/<entrada>.copc.laz
      --results-dir <RESULTS_DIR>  Envía a esta carpeta todo producto al que no se dé una salida explícita, en lugar de la carpeta de cada entrada
```

### hipso render

Dibuja una nube (planta, 3D o perfil) en un PNG, como hace el visor

```
Dibuja una nube (planta, 3D o perfil) en un PNG, como hace el visor

Uso: hipso render [OPTIONS] <INPUTS>...

Argumentos:
  <INPUTS>...
          

Opciones:
  -o, --output <OUTPUT>
          PNG que escribir; por defecto: junto a la primera entrada

      --results-dir <RESULTS_DIR>
          Envía a esta carpeta todo producto al que no se dé una salida explícita, en lugar de la carpeta de cada entrada

      --view <VIEW>
          [default: plan]
          [possible values: plan, 3d, profile]

      --color <COLOR>
          Possible values:
          - elevation
          - earth
          - class
          - intensity
          - return
          - source
          - file
          - rgb
          - tree:      Crown number (treeID extra attribute)
          
          [default: elevation]

      --size <SIZE>
          Tamaño de la imagen: ANCHOxALTO
          
          [default: 1600x1000]

      --max-points <MAX_POINTS>
          Puntos que se guardan en memoria (se carga uno de cada N)
          
          [default: 8000000]

      --point-size <POINT_SIZE>
          [default: 1]

      --yaw <YAW>
          3D: rotación y elevación en grados
          
          [default: 30]

      --pitch <PITCH>
          [default: 35]

      --profile <PROFILE>
          Profile: x1,y1,x2,y2,width

      --shade <SHADE>
          Planta: TIN sombreado bajo los puntos, a partir de las clases dadas (p. ej. 2; vacío: todas)

      --contours <CONTOURS>
          Planta: curvas de nivel cada tanto

      --class <CLASS>
          Clases que se dibujan: 2,5 o ~7,18

  -h, --help
```

## Flujos de trabajo

### hipso schema

Lo que admite cada orden, como JSON: las opciones que dibuja un constructor de flujos para cada herramienta

```
Lo que admite cada orden, como JSON: las opciones que dibuja un constructor de flujos para cada herramienta

Uso: hipso schema [OPTIONS]

Opciones:
      --results-dir <RESULTS_DIR>  Envía a esta carpeta todo producto al que no se dé una salida explícita, en lugar de la carpeta de cada entrada
```

### hipso run

Ejecuta un flujo guardado por el constructor de flujos (un `.model.json`): cada nodo es una orden de este programa, ejecutada en orden; los productos conservados van a la carpeta de resultados

```
Ejecuta un flujo guardado por el constructor de flujos (un `.model.json`): cada nodo es una orden de este programa, ejecutada en orden; los productos conservados van a la carpeta de resultados

Uso: hipso run [OPTIONS] <MODEL>

Argumentos:
  <MODEL>  El flujo

Opciones:
      --input <INPUTS>             Sustituye un nodo de entrada (por su nombre o identificador): `nombre=ruta`
      --results-dir <RESULTS_DIR>  Envía a esta carpeta todo producto al que no se dé una salida explícita, en lugar de la carpeta de cada entrada
```

### hipso model-validate

Comprueba un flujo: lista lo que impide ejecutarlo (un bucle, un archivo u opción que falta)

```
Comprueba un flujo: lista lo que impide ejecutarlo (un bucle, un archivo u opción que falta)

Uso: hipso model-validate [OPTIONS] <MODEL>

Argumentos:
  <MODEL>  El flujo

Opciones:
      --results-dir <RESULTS_DIR>  Envía a esta carpeta todo producto al que no se dé una salida explícita, en lugar de la carpeta de cada entrada
```
