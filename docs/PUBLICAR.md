# Publicar una versión de EasyLiDAR

Compilamos en local (GitHub Actions es solo manual). Cada sistema sube su paquete a la misma release `vX.Y.Z`:

- **Mac:** `EasyLiDAR-macos-arm64.tar.gz` y `EasyLiDAR-macos-x64.tar.gz`
- **Windows:** `EasyLiDAR-windows-x64.zip`

Da igual el orden. El primero que publica sube la versión y crea la release; el segundo hace `git pull` y añade su paquete a esa release sin tocar la versión.

Cada prompt se pega en tu asistente de código abierto en la carpeta del proyecto. Cambia solo `VERSIÓN` (ejemplo: `0.1.4`) si este es el primer equipo que publica. Si eres el segundo, déjalo en blanco.

---

## Prompt para Mac

```text
Publica EasyLiDAR desde este Mac. VERSIÓN: <X.Y.Z, o vacío si el otro equipo ya publicó>

1. Haz `git pull` y comprueba que el árbol está limpio. Carga Rust con `. ~/.cargo/env`.
2. Mira la versión de Cargo.toml y si existe la release `v<esa versión>` en GitHub (`gh release view`):
   - Si existe y NO tiene aún los paquetes de macOS (`EasyLiDAR-macos-arm64.tar.gz` y `EasyLiDAR-macos-x64.tar.gz`): el otro equipo ya abrió esta versión. NO cambies la versión; ve al paso 4.
   - Si no existe, o ya tiene todos los paquetes de macOS: es una versión nueva. Usa la VERSIÓN que te indico arriba (si falta, pregúntamela).
3. Versión nueva: súbela en Cargo.toml (workspace) y deja Cargo.lock actualizado, ejecuta `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` y `cargo test`, y si todo pasa haz commit "Versión X.Y.Z" y push a main. Si algo falla, párate y dímelo.
4. Ejecuta `./scripts/package.sh --publish`. Comprueba con `gh release view v<versión>` que están subidos los dos paquetes de macOS y dime qué paquetes tiene ya la release y cuáles faltan (Windows).
No lances nada en GitHub Actions.
```

---

## Prompt para Windows

```text
Publica EasyLiDAR desde este PC con Windows. VERSIÓN: <X.Y.Z, o vacío si el otro equipo ya publicó>

1. Haz `git pull` y comprueba que el árbol está limpio. Rust está en `%USERPROFILE%\.cargo\bin`.
2. Mira la versión de Cargo.toml y si existe la release `v<esa versión>` en GitHub (`gh release view`):
   - Si existe y NO tiene aún `EasyLiDAR-windows-x64.zip`: el otro equipo ya abrió esta versión. NO cambies la versión; ve al paso 4.
   - Si no existe, o ya tiene el zip de Windows: es una versión nueva. Usa la VERSIÓN que te indico arriba (si falta, pregúntamela).
3. Versión nueva: súbela en Cargo.toml (workspace) y deja Cargo.lock actualizado, ejecuta `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` y `cargo test`, y si todo pasa haz commit "Versión X.Y.Z" y push a main. Si algo falla, párate y dímelo.
4. Ejecuta `.\scripts\package.ps1 -Publish`. Comprueba con `gh release view v<versión>` que está subido `EasyLiDAR-windows-x64.zip` y dime qué paquetes tiene ya la release y cuáles faltan (macOS).
No lances nada en GitHub Actions.
```

---

## Notas

- Requisitos en cada equipo: Rust (`rustup`), `gh` con la sesión iniciada (`gh auth login`) y el repo sincronizado.
- En el Mac, el paquete Intel necesita el destino `x86_64-apple-darwin`; ya está instalado.
- Cuando la release tenga los tres paquetes, está lista. Hasta entonces el enlace de descarga del sistema que falte apunta a la release anterior o falla.
