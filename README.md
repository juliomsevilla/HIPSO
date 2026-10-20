<p align="center">
  <img src="assets/icon-256.png" alt="EasyLiDAR" width="128">
</p>

<h1 align="center">EasyLiDAR</h1>

<p align="center">
  <strong>Procesamiento y edición de nubes LiDAR para forestales</strong><br>
  Rápido, ligero con nubes enormes y cómodo de editar. Windows, macOS y Linux.
</p>

<p align="center">
  <a href="https://github.com/juliomsevilla/EasyLiDAR/releases/latest"><img alt="Descargar" src="https://img.shields.io/github/v/release/juliomsevilla/EasyLiDAR?label=descargar&color=ff8a1f&style=for-the-badge"></a>
  <img alt="Plataformas" src="https://img.shields.io/badge/Windows%20%C2%B7%20macOS%20%C2%B7%20Linux-555?style=for-the-badge">
  <img alt="Gratuito" src="https://img.shields.io/badge/uso-gratuito-2e7d32?style=for-the-badge">
</p>

---

## Descargar

Descarga el paquete de tu sistema, descomprímelo y ejecuta `easylidar-gui`. No necesita instalador ni permisos de administrador.

| Sistema | Descarga |
|---|---|
| **Windows** 10/11 (x64) | [EasyLiDAR-windows-x64.zip](https://github.com/juliomsevilla/EasyLiDAR/releases/latest/download/EasyLiDAR-windows-x64.zip) |
| **macOS** Apple Silicon (M1 o posterior) | [EasyLiDAR-macos-arm64.tar.gz](https://github.com/juliomsevilla/EasyLiDAR/releases/latest/download/EasyLiDAR-macos-arm64.tar.gz) |
| **macOS** Intel | [EasyLiDAR-macos-x64.tar.gz](https://github.com/juliomsevilla/EasyLiDAR/releases/latest/download/EasyLiDAR-macos-x64.tar.gz) |
| **Linux** x64 | [EasyLiDAR-linux-x64.tar.gz](https://github.com/juliomsevilla/EasyLiDAR/releases/latest/download/EasyLiDAR-linux-x64.tar.gz) |

Todas las versiones están en la página de [Releases](https://github.com/juliomsevilla/EasyLiDAR/releases).

**Primera vez que lo abres**

- **Windows:** si aparece el aviso azul de SmartScreen: *Más información → Ejecutar de todas formas*.
- **macOS:** abre `EasyLiDAR.app` (clic derecho → *Abrir* la primera vez; no abre ventana de Terminal). También puedes usar `easylidar-gui` desde la consola; si no arranca: `chmod +x easylidar-gui easylidar`.
- **Linux:** `chmod +x easylidar-gui easylidar` si el archivo perdió el permiso. Hace falta GTK 3 para los diálogos de archivo y OpenGL 2 o superior (Mesa sirve).

---

## Qué hace

### Premisas

1. **Rapidez.**
2. **Eficiencia con nubes de puntos enormes:** los procesos leen la nube como un flujo y la memoria depende del área y de la celda, no del número de puntos. Los pasos locales trabajan por teselas en paralelo.
3. **Edición cómoda de nubes de puntos:** seleccionar, reclasificar y borrar con pocos clics, deshacer y rehacer, y ver el resultado al instante en planta, perfil y 3D.

### Visor y edición

- Ventanas **2D**, **2D + perfil**, **3D** (independiente de la planta) y **2D + perfil + 3D**.
- Navegación 3D libre: giro sin tope, desplazamiento en el plano de la pantalla, zoom con la rueda, encuadre con doble clic.
- Colorear por elevación, clase, intensidad, retorno, pasada, archivo o RGB.
- **Edición por selecciones** (rectángulo, círculo, polígono, en planta o en perfil) con filtros por clase, altura, intensidad y retorno; eliminar o reclasificar; deshacer, rehacer e historial. El original nunca se modifica.
- **Histogramas de atributos** (elevación, intensidad, retornos, clase, pasada, ángulo de barrido): se marcan barras para seleccionar los puntos, o se detectan los valores raros con un estadístico (Tukey, MAD, desviaciones típicas, percentiles).
- **Clases ASPRS 0–255:** la tabla vigente y clases nuevas propias (64–255) con nombre y color.

### Procesar

| Grupo | Herramientas |
|---|---|
| Preparar | Información de los datos · Asignar proyección · **Adelgazar nube** (densidad objetivo, celdas, percentil, 3D, uno de cada n) · Clasificar: ruido, suelo (Kraus y Pfeifer, MCC, CSF, PMF, **Axelsson**), edificios, vegetación por estratos, cables y postes, **reglas por eco y ángulo de barrido** |
| Superficies | MDT (triangulación por defecto, ponderado por distancia o media por celda) · MDS (triangulación por defecto, ponderado por distancia o máximo por celda) · CHM (con *pit-free* y suavizado) |
| Inventario | Métricas de rodal · Segmentación de árboles (Dalponte, Silva, cuencas, ventana variable) con evaluación frente a campo |
| Recorte y salida | Parcelas (de archivo, muestreo o dibujadas) · Exportar puntos |
| Calidad y comparación | Quitar duplicados · **Comparar nubes** (cambios) · **Ajustar una nube a otra** (ICP) · Separación entre pasadas |
| **TLS** | **Cubicar árboles**: mide diámetros a lo largo del tronco y cubica con Smalian |
| **Volúmenes** | Volumen sobre (y bajo) el MDT, desde el CHM o el MDS, por ráster completo, AOI o polígonos |

### Cubicación de árboles (TLS)

En nubes de escáner terrestre, móvil o fotogramétrico: encuentra los troncos, los sigue de abajo arriba ajustando círculos cada pocos centímetros y cubica cada árbol con la **fórmula de Smalian**, `V = (A₁ + A₂) / 2 · L`, desde la altura de la base (10 cm por defecto) hasta el diámetro de punta delgada (7 cm por defecto), con trozas de 0,5 m (la distancia entre diámetros). Entrega un shapefile con la posición de cada árbol y sus datos dasométricos (DAP, alturas, volumen, nº de trozas, inclinación, calidad), el detalle de cada troza en CSV y, si se quiere, la nube de cada árbol.

### Teledetección

- Composiciones de bandas e índices espectrales (NDVI, GNDVI, NDWI, EVI, SAVI, NDMI, NBR, VARI, ExG o expresión propia).
- **Crear multibanda** a partir de rásteres sueltos.
- **Mosaico** con regla de solape: primero, último, media, mínimo, máximo, mediana, suma o número de capas.
- Lectura por ventana y escritura por franjas comprimidas: se unen imágenes de varios gigabytes con poca memoria.

### Capas y formatos

LAS y LAZ, GeoTIFF, ASCII, shapefile y CSV. Capas vectoriales con digitalización, tabla de atributos editable y simbología.

---

## Manual

El manual completo está en [`docs/MANUAL.md`](docs/MANUAL.md) y dentro del programa: **Ayuda → Manual** o la tecla **F1**. La referencia de la línea de comandos está en [`docs/REFERENCIA.md`](docs/REFERENCIA.md).

## Línea de comandos

`easylidar` hace lo mismo que la interfaz, por lotes:

```sh
easylidar info nube.laz
easylidar thin nube.laz --method density --density 1
easylidar classify nube.laz --ground csf --noise isolated --buildings --vegetation
easylidar dtm nube.laz --cell 1
easylidar chm nube.laz --cell 0.5 --method pitfree
easylidar cubicar parcela_tls.laz --csv --tree-clouds
easylidar mosaic a.tif b.tif -o mosaico.tif --overlap mean
easylidar compare nube2.laz --reference nube1.laz --distance 0.5
```

`easylidar <comando> --help` lista todos los parámetros.

## Compilar desde el código fuente

Hace falta [Rust](https://rustup.rs) 1.90 o superior.

```sh
git clone https://github.com/juliomsevilla/EasyLiDAR.git
cd EasyLiDAR
cargo build --release        # target/release/easylidar y easylidar-gui
cargo test --workspace
```

En Linux, antes: `sudo apt-get install libxkbcommon-dev libwayland-dev libgtk-3-dev libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev`.

Los paquetes de cada sistema se generan en local con `scripts/package.ps1` (Windows) o `scripts/package.sh` (macOS y Linux). Con `-Publish` (Windows) o `--publish` (macOS y Linux) se suben a la release `vX.Y.Z` de Releases, que se crea si no existe (hace falta [`gh`](https://cli.github.com) con sesión iniciada). Los flujos de GitHub Actions son solo manuales.

## Limitaciones conocidas

**EasyLiDAR no reproyecta nubes de puntos, ni al vuelo ni de ningún otro modo.** Trabaja siempre con el sistema de coordenadas que traigan tus archivos LAS/LAZ.

- Si cargas varias teselas a la vez, **se toma el sistema de coordenadas de la primera nube** y se aplica al resultado. El programa no comprueba que las demás coincidan: mezclar teselas en sistemas distintos (por ejemplo, ETRS89 UTM 29N con 30N) colocaría los puntos en coordenadas incompatibles, sin aviso.
- Antes de cargar un lote, asegúrate de que todas las teselas comparten el mismo sistema de referencia. Las de una misma descarga (por ejemplo, el PNOA de una región) suelen venir ya homogéneas.

**¿Por qué no reproyecta al vuelo?** Porque en una nube 3D reproyectar no es solo mover X e Y. Cambiar de proyección deforma ligeramente la geometría, y cambiar de datum vertical (de altitudes elipsoidales a ortométricas, por ejemplo) modifica también la Z y requiere una malla de geoide. Hacerlo de forma invisible haría que lo que ves en pantalla dejase de corresponder a lo que hay en tus archivos, y que cada herramienta (MDT, recortes, ediciones, exportación) tuviera que reproyectar por su cuenta. Preferimos no esconder una operación que cambia tus datos.

Si necesitas reproyectar, hazlo antes con una herramienta pensada para ello, como [PDAL](https://pdal.io), [LAStools](https://lastools.github.io) o [CloudCompare](https://www.cloudcompare.org), que son más eficientes y precisos en esta tarea, y abre después en EasyLiDAR las nubes ya homogéneas.

---

## Licencia

EasyLiDAR es de **uso gratuito**, también para trabajo profesional y comercial. No se puede modificar, vender ni redistribuir salvo en su paquete original y gratis. Las condiciones están en [`LICENSE`](LICENSE). Los componentes de terceros conservan sus licencias ([`NOTICE`](NOTICE), [`THIRD_PARTY_LICENSES.md`](THIRD_PARTY_LICENSES.md), [`licenses/`](licenses)).

## Autoría y contacto

**Julio M. Sevilla**
✉ [julio@jsevilla.es](mailto:julio@jsevilla.es) · [LinkedIn](https://es.linkedin.com/in/juliomsevilla)

Se agradecen todas las aportaciones y la detección de errores (también en [Issues](https://github.com/juliomsevilla/EasyLiDAR/issues)). Los errores se irán corrigiendo en la medida de lo posible.
