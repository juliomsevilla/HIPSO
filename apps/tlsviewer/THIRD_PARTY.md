# Componentes de terceros de «Ver nubes TLS»

La ventana «Ver nubes TLS» (`hipso-tls`) lleva estos componentes de código abierto. Sus avisos de
copyright y licencias se conservan tal cual en `web/libs/` y se citan aquí.

| Componente | Uso | Licencia |
|---|---|---|
| Potree 1.8 (Markus Schütz, 2011-2020), `web/libs/potree/` | Visor de nubes de puntos | BSD de 2 cláusulas |
| PotreeConverter 2.x (Markus Schütz), ejecutable `PotreeConverter` | Prepara la nube para el visor | BSD de 2 cláusulas |
| three.js (incluido en Potree) | Dibujo 3D | MIT |
| jQuery 3.1.1, jQuery UI, Tween.js, proj4js, i18next | Dependencias de Potree | MIT |
| wry y tao | Ventana y vista web del sistema | MIT o Apache-2.0 |

Notas:
- El sombreado de relieve de Potree (EDL) está adaptado por su autor del código de CloudCompare,
  que es GPL. HIPSO distribuye Potree como componente separado, con su propia licencia, sin
  fusionar su código con el de HIPSO. Conviene revisarlo antes de publicar una versión.
- PotreeConverter se compila para macOS con cinco cambios de portabilidad (algoritmos paralelos de
  libc++, `fseek` de 64 bits, `__debugbreak`, memoria virtual por `mmap` y lectura de memoria y
  CPU). El parche y las instrucciones de compilación están en `potreeconverter/`.
- En macOS, PotreeConverter enlaza con TBB de Homebrew; hay que incluirlo o enlazarlo estático al
  empaquetar (`potreeconverter/COMO_COMPILAR.md`).

## Parche de Potree: nodos vacíos

El conversor deja algunos nodos hoja sin puntos. Potree los pasaba al decodificador, que falla con un
búfer vacío y no devuelve su plaza de carga (hay cuatro), de modo que la carga se iba atascando y
quedaban huecos. En `libs/potree/potree.js` (función `load` del cargador de octrees) un nodo de 0 bytes
se da ahora por cargado, sin puntos y sin pasar por el decodificador.
