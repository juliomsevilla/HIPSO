# Cómo distribuir EasyLiDAR

EasyLiDAR son dos ejecutables sin dependencias de C ni instalador: `easylidar` (línea de comandos) y `easylidar-gui` (interfaz). En Windows el runtime de C va enlazado dentro (`.cargo/config.toml`), así que corren en un equipo limpio. Un binario solo sirve para el sistema en el que se compiló.

## Paquetes

| Sistema | Cómo se genera | Resultado |
|---|---|---|
| Windows x64 | `powershell -File scripts\package.ps1` | `dist\EasyLiDAR-<versión>-windows-x64.zip` |
| Linux, macOS | `scripts/package.sh` en esa máquina | `dist/EasyLiDAR-<versión>-<sistema>-<arquitectura>.tar.gz` (conserva el permiso de ejecución) |
| Los tres, a la vez | Etiqueta `vX.Y.Z` en GitHub (`.github/workflows/release.yml`) | Paquetes de Windows, Linux, macOS Intel y macOS Apple Silicon como artefactos |
| Código fuente | `EasyLiDAR-<versión>-fuente.zip` | Permite compilar en cualquier sistema con `cargo build --release` |

Cada paquete lleva: los dos ejecutables, `README.md`, `LICENSE`, `NOTICE`, `THIRD_PARTY_LICENSES.md`, `MANUAL.md` y la carpeta `licenses`. El manual también está dentro del programa (Ayuda → Manual, o F1). **No incluyas archivos de desarrollo (`target`, `.git`, notas internas) ni datos de terceros.**

Linux compila con: `libxkbcommon-dev libwayland-dev libgtk-3-dev libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev`.

## Qué verá el usuario

- **Windows:** un `.exe` sin firmar muestra el aviso azul de SmartScreen (Más información → Ejecutar de todas formas). Un certificado de firma de código lo elimina.
- **macOS:** sin firma ni notarización, Gatekeeper bloquea la app: clic derecho → Abrir. Para quitar el aviso hace falta una cuenta de Apple Developer (`codesign` y `notarytool`).
- **Linux:** no hace falta firma. Si el archivo pierde el permiso: `chmod +x easylidar easylidar-gui`.

Requisitos: Windows 10/11, macOS 12 o superior, o un Linux con X11/Wayland y OpenGL 2 o superior (Mesa sin GPU dedicada), con GTK 3 para los diálogos de archivo.

## Instaladores (opcional)

| Sistema | Herramienta | Resultado |
|---|---|---|
| Windows | Inno Setup o `cargo-wix` | `.exe` / `.msi` con acceso directo y asociación de `.las` y `.laz` |
| macOS | `cargo-bundle` | `.app`, que se empaqueta en `.dmg` |
| Linux | `cargo-deb`, o AppImage | `.deb` o un único archivo ejecutable |

El logotipo está en `assets/` (`icon.svg`, `easylidar.ico`, PNG). `apps/gui/build.rs` lo incrusta en el `.exe` de Windows.

## Versiones

Sube la versión en `[workspace.package]` del `Cargo.toml` raíz, etiqueta (`vX.Y.Z`) y genera los paquetes. No hay actualización automática: avisa de la nueva versión.

## Licencia

Uso gratuito (`LICENSE`). Los componentes de terceros conservan las suyas (`NOTICE`, `THIRD_PARTY_LICENSES.md`, `licenses/`). Mientras el programa sea gratuito, nada impide cobrar en el futuro: la licencia ya lo prevé (cláusula 5).
