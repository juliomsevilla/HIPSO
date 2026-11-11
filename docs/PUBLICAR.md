# Publicar una versión de HIPSO

Compilamos el Mac en local; Windows y Linux los compila GitHub Actions. Cada sistema sube su paquete a la misma release `vX.Y.Z`:

- **Mac:** `HIPSO-macos-arm64.tar.gz` y `HIPSO-macos-x64.tar.gz`
- **Windows:** `HIPSO-windows-x64.zip` — lo compila GitHub Actions (`.github/workflows/windows.yml`) solo, en cuanto el Mac crea la release. Si hace falta, también se puede compilar en un PC con Windows (`scripts/package.ps1 -Publish`) con el prompt de más abajo.
- **Linux:** `HIPSO-linux-x64.tar.gz` — lo compila GitHub Actions (`.github/workflows/linux.yml`) solo, en cuanto el Mac crea la release. No hay que hacer nada a mano.

Da igual el orden entre Mac y Windows. El primero que publica sube la versión y crea la release; el segundo hace `git pull` y añade su paquete a esa release sin tocar la versión. Si Windows publica primero, es su `gh release create` el que dispara la compilación de Linux, igual que haría el Mac.

Cada prompt se pega en tu asistente de código abierto en la carpeta del proyecto. Cambia solo `VERSIÓN` (ejemplo: `0.1.4`) si este es el primer equipo que publica. Si eres el segundo, déjalo en blanco.

---

## Prompt para Mac

```text
Publica HIPSO desde este Mac. VERSIÓN: <X.Y.Z, o vacío si el otro equipo ya publicó>

1. Haz `git pull` y comprueba que el árbol está limpio. Carga Rust con `. ~/.cargo/env`.
2. Mira la versión de Cargo.toml y si existe la release `v<esa versión>` en GitHub (`gh release view`):
   - Si existe y NO tiene aún los paquetes de macOS (`HIPSO-macos-arm64.tar.gz` y `HIPSO-macos-x64.tar.gz`): el otro equipo ya abrió esta versión. NO cambies la versión; ve al paso 4.
   - Si no existe, o ya tiene todos los paquetes de macOS: es una versión nueva. Usa la VERSIÓN que te indico arriba (si falta, pregúntamela).
3. Versión nueva: súbela en Cargo.toml (workspace) y deja Cargo.lock actualizado, ejecuta `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` y `cargo test`, y si todo pasa haz commit "Versión X.Y.Z" y push a main. Si algo falla, párate y dímelo.
4. Ejecuta `./scripts/package.sh --publish`. Comprueba con `gh release view v<versión>` que están subidos los dos paquetes de macOS y dime qué paquetes tiene ya la release y cuáles faltan (Windows).
4b. Si la release acaba de crearse, GitHub Actions compila Linux solo (workflow `linux`): mira con `gh run list --workflow linux.yml --limit 1` que ha arrancado y dímelo. Si no arrancó, lánzalo con `gh workflow run linux.yml -f tag=v<versión>`. No esperes a que termine, solo dime el estado.
No lances ningún otro flujo de Actions (`release.yml` es solo de emergencia).
```

---

## Prompt para Windows

```text
Publica HIPSO desde este PC con Windows. VERSIÓN: <X.Y.Z, o vacío si el otro equipo ya publicó>

1. Haz `git pull` y comprueba que el árbol está limpio. Rust está en `%USERPROFILE%\.cargo\bin`.
2. Mira la versión de Cargo.toml y si existe la release `v<esa versión>` en GitHub (`gh release view`):
   - Si existe y NO tiene aún `HIPSO-windows-x64.zip`: el otro equipo ya abrió esta versión. NO cambies la versión; ve al paso 4.
   - Si no existe, o ya tiene el zip de Windows: es una versión nueva. Usa la VERSIÓN que te indico arriba (si falta, pregúntamela).
3. Versión nueva: súbela en Cargo.toml (workspace) y deja Cargo.lock actualizado, ejecuta `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` y `cargo test`, y si todo pasa haz commit "Versión X.Y.Z" y push a main. Si algo falla, párate y dímelo.
4. Ejecuta `.\scripts\package.ps1 -Publish`. Comprueba con `gh release view v<versión>` que está subido `HIPSO-windows-x64.zip` y dime qué paquetes tiene ya la release y cuáles faltan (macOS y Linux; Linux lo sube GitHub Actions solo y puede tardar unos minutos: mira `gh run list --workflow linux.yml --limit 1`).
No lances nada en GitHub Actions.
```

---

## Notas

- Requisitos en cada equipo: Rust (`rustup`), `gh` con la sesión iniciada (`gh auth login`) y el repo sincronizado.
- En el Mac, el paquete Intel necesita el destino `x86_64-apple-darwin`; ya está instalado.
- El flujo de Linux necesita que `linux.yml` esté ya en `main` (lo está tras hacer push). Tarda unos minutos y sube el paquete a la misma release; si falla, revisa `gh run list --workflow linux.yml` o lánzalo a mano con `gh workflow run linux.yml -f tag=vX.Y.Z`.
- Cuando la release tenga los cuatro paquetes (macOS arm64, macOS x64, Windows y Linux), está lista. Hasta entonces el enlace de descarga del sistema que falte apunta a la release anterior o falla.

## Sin `gh`: que GitHub lo compile todo

Si no hay `gh` a mano, basta con subir la versión y la etiqueta: `git tag vX.Y.Z && git push origin main vX.Y.Z`. El flujo `release.yml` arranca solo con la etiqueta, compila Windows, Linux, macOS Apple Silicon y macOS Intel, y publica la release con los cuatro paquetes (`generate_release_notes`). Las releases creadas así no disparan `linux.yml`, que solo sirve para el camino con `gh`. Seguimiento: pestaña Actions del repositorio.

Tanto `release.yml` como `scripts/package.sh` crean `HIPSO.app` con el mismo script (`scripts/macos-app.sh`), de modo que los paquetes de macOS son equivalentes salgan de donde salgan. Una release creada con `gh` dispara `release.yml` (por la etiqueta) y `linux.yml` a la vez, y el primero sobrescribe los paquetes que haya subido el Mac.

Si GitHub no compilara macOS (cuota o runners), usa el prompt de Mac de arriba en un Mac: `./scripts/package.sh --publish` sube los dos paquetes de macOS a la release `vX.Y.Z`.
