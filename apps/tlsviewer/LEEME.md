# El visor de nubes densas (`hipso-tls`)

Una ventana propia que enseña una nube terrestre densa con todo el detalle al acercarse. Dibuja con
el visor de código abierto Potree dentro de la vista web del sistema (no abre navegador ni servidor)
y lee de disco, por trozos, un octree que prepara PotreeConverter.

No está en la cinta del programa: se abre con la orden **Ver nubes TLS** del buscador de comandos
(Ctrl+K), que convierte la primera nube cargada si hace falta, o desde la consola:

```sh
hipso-tls nube.laz                 # convierte (si no está ya) y abre
hipso-tls carpeta_octree           # abre una conversión hecha
hipso-tls                          # ventana vacía: «Abrir nube…» en la cinta del visor
```

La conversión queda en la caché (`…/HIPSO/octree/`) y ocupa algo menos que la nube original (unos
560 MB para 740 MB). `hipso-tls` y PotreeConverter van junto a `hipso`; véase
`potreeconverter/COMO_COMPILAR.md`.

`hipso-tls` es un miembro del espacio de trabajo pero no de los de por defecto: `cargo build` no lo
compila (en Linux necesita las bibliotecas de WebKitGTK). Se compila con `cargo build --release -p
hipso-tls`.

## Publicar una nube en la web

Es el uso que queda preparado para cuando haya que pasar nubes LiDAR a clientes. La página del visor
(`web/`) funciona también sin la ventana: lee `data/metadata.json` y `data/octree.bin` por rangos de
bytes, de modo que vale cualquier servidor de archivos estáticos **que admita peticiones con
`Range`** (nginx, Apache, GitHub Pages, S3…).

```sh
apps/tlsviewer/publicar_web.sh carpeta_octree carpeta_destino
```

crea `carpeta_destino/` con la página, Potree y los datos. Para verla en local hace falta un
servidor con soporte de `Range` (el de `python -m http.server` no lo tiene). **Aviso:** este camino
(la carpeta publicada, el servidor y el navegador) no se ha probado de punta a punta; sí se ha
probado la ventana. Antes de ofrecerlo a clientes hay que probarlo con el servidor real, y ver qué
datos se publican: la nube convertida es una copia de los puntos.

Pendiente antes de repartir el visor con el programa: compilar PotreeConverter para cada sistema
y revisar la licencia del relieve de Potree (EDL), que viene de código GPL de CloudCompare
(`THIRD_PARTY.md`).
