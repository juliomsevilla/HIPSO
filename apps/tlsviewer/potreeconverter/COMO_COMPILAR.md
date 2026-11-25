# Compilar PotreeConverter para el visor

`hipso-tls` convierte una nube LAS o LAZ a un octree que lee el visor, y para eso llama a
**PotreeConverter** (Markus Schütz, licencia BSD de 2 cláusulas), que no forma parte de HIPSO. Hay
que compilarlo aparte y dejar el ejecutable `PotreeConverter` junto a `hipso-tls`.

## Versión

Se usa la 2.x del repositorio <https://github.com/potree/PotreeConverter>, en la confirmación
`db42c63` (la última cuando se hizo; con ella la conversión de las nubes de prueba salió bien). Los
cambios de este directorio (`macos-portabilidad.patch`) se aplican encima.

## macOS (Apple Silicon e Intel)

Hacen falta las herramientas de línea de comandos de Xcode, CMake y TBB (`brew install cmake tbb`).

```sh
git clone https://github.com/potree/PotreeConverter.git
cd PotreeConverter
git checkout db42c63
git apply /ruta/a/HIPSO/apps/tlsviewer/potreeconverter/macos-portabilidad.patch
cmake -S . -B build -DCMAKE_BUILD_TYPE=Release \
  -DCMAKE_CXX_FLAGS="-fexperimental-library -D__debugbreak=__builtin_trap -I$(brew --prefix tbb)/include" \
  -DTBB_DIR="$(brew --prefix)/lib/cmake/TBB"
cmake --build build -j
```

El resultado es `build/PotreeConverter`, junto con `build/liblaszip.dylib`. En macOS enlaza con la
TBB de Homebrew (`/opt/homebrew/opt/tbb/lib/libtbb.12.dylib`) y con `liblaszip.dylib` por `@rpath`:
para que funcione en otro equipo hay que copiar las dos bibliotecas junto al ejecutable y corregir
sus rutas con `install_name_tool`, o compilar TBB y LASzip estáticos.

Qué corrige el parche (cinco cambios de portabilidad, ninguno cambia el resultado):
- Los algoritmos paralelos de la biblioteca estándar de libc++ (`std::execution::par`) se pasan por
  referencia constante y se activan con `-fexperimental-library`.
- Una conversión explícita a `int32_t` al construir un punto del muestreo de Poisson.
- `fseek` de 64 bits y `__debugbreak` (con `-D__debugbreak=__builtin_trap`).
- Reserva de memoria virtual con `mmap` y lectura de la memoria y la CPU en macOS
  (`unsuck_platform_specific.cpp`), que solo existían para Windows y Linux.

## Windows y Linux

No se han compilado todavía. PotreeConverter trae su propio `CMakeLists.txt` para los dos y no
necesita el parche. En Linux necesita TBB (`libtbb-dev`) y un compilador con C++23.

## Dónde lo busca el visor

`hipso-tls` y el botón de «Ver nubes TLS» (hoy solo en la orden «Ver nubes TLS» del buscador de
comandos) buscan `PotreeConverter` en la misma carpeta que su propio ejecutable
(`hipso_core::tls_octree::junto_al_programa`). Sin él, el visor abre las nubes que ya estén
convertidas y avisa con «Falta PotreeConverter junto al programa» al pedir una conversión.
