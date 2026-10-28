//! The tooltip of every button of the ribbon and of the side panels, in one place. A button
//! finds its text by its label, so a new button only needs a line here. In debug builds a
//! button with no text says so once in the terminal.

/// What the button called `label` does (and its shortcut), if it has a text.
pub fn tip(label: &str) -> Option<&'static str> {
    let key = label.replace('\n', " ");
    let t = match key.as_str() {
        // Inicio
        "Abrir" => {
            "Abre nubes de puntos (LAS/LAZ), rásteres, imágenes y capas vectoriales; se añaden a lo que ya está abierto (Ctrl+O)."
        }
        "Información" => {
            "Muestra las características de las nubes abiertas: puntos, densidad, clases, coordenadas y avisos."
        }
        "Alturas sobre suelo" => {
            "Calcula la altura de cada punto sobre el suelo a partir de los puntos de suelo de la propia nube y la guarda como atributo (conservando las cotas) o en lugar de la cota."
        }
        "Guardar vista" => "Guarda la vista actual como imagen PNG.",
        "Cerrar todo" => "Cierra las nubes y las capas abiertas.",
        "Nueva capa" => {
            "Crea una capa vectorial vacía (puntos, líneas o áreas) para digitalizar sobre la planta."
        }
        "Tabla de atributos" => "Abre la tabla de atributos de la capa vectorial.",
        "Digitalizar" => "Dibuja elementos nuevos en la capa vectorial activa.",
        "Guardar capa" => "Guarda la capa vectorial activa en su archivo.",
        "Composición de bandas" => {
            "Elige qué bandas de una imagen multibanda dan el rojo, el verde y el azul."
        }
        "Índices espectrales" => {
            "Calcula índices como el NDVI a partir de las bandas de una imagen."
        }
        "Crear multibanda" => "Une varias imágenes de una banda en una sola multibanda.",
        "Mosaico" => "Une varios rásteres en uno.",
        "Igualar rásteres" => {
            "Pone varios rásteres con la misma rejilla, proyección y tamaño de celda."
        }
        "Tema" => "Cambia entre el tema claro y el oscuro.",
        "Registro" => "Muestra lo que se ha hecho, los errores y el avance de los procesos largos.",
        // Vista
        "Zoom" => "Clic acerca, clic derecho aleja y arrastrar dibuja una ventana de zoom (Z).",
        "Desplazar" => "Mueve la vista arrastrando (X).",
        "Extensión completa" => "Encuadra todo lo cargado.",
        "Zoom anterior" => "Vuelve al encuadre anterior.",
        "Zoom a la selección" => {
            "Encuadra los puntos seleccionados o los elementos seleccionados de la capa vectorial activa."
        }
        "Refrescar 3D" => {
            "Vuelve a encuadrar la vista 3D en lo que se ve en la planta (útil con capas muy separadas)."
        }
        "Guardar proyecto" => {
            "Guarda qué nubes y capas están abiertas y cómo se ve (archivo .elp)."
        }
        "Abrir proyecto" => "Abre un proyecto .elp: cierra lo abierto y carga sus nubes y capas.",
        "Limpiar marcas" => "Quita las mediciones, la consulta, la densidad y el perfil dibujados.",
        "Consultar" => "Muestra los atributos del punto sobre el que haces clic (Q).",
        "Densidad" => "Dibuja un rectángulo y da los puntos por metro cuadrado.",
        "Perfil" => {
            "Dibuja una franja de la nube vista de lado: clic en los dos extremos y en el borde de la caja."
        }
        "Medir" => "Mide distancias y áreas sobre la planta (W).",
        "TIN" => {
            "TIN sombreado del terreno (T). Oculta los puntos de la planta mientras está activo."
        }
        "Curvas de nivel" => {
            "Curvas de nivel del terreno. Ocultan los puntos de la planta mientras están activas."
        }
        // Procesar
        "Información de los datos" => {
            "Analiza las nubes: puntos, densidad, clases, coordenadas y avisos."
        }
        "Clasificar" => "Clasifica ruido, suelo, edificios, vegetación, cables y postes.",
        "Asignar proyección" => "Asigna a una nube el sistema de coordenadas que le falta.",
        "Adelgazar nube" => "Reduce la densidad de la nube conservando su forma.",
        "Comparar nubes" => "Mide las diferencias entre dos nubes del mismo lugar.",
        "Ajustar nubes" => "Alinea una nube a otra de referencia.",
        "Entre pasadas" => "Mide cuánto se separan entre sí las pasadas de vuelo.",
        "MDT" => "Modelo digital del terreno a partir de los puntos de suelo.",
        "MDS / CHM" => "Modelo digital de superficie y de alturas de la vegetación (CHM).",
        "Análisis de terreno" => {
            "Pendiente, orientación, curvatura, sombreado e hidrología a partir de un MDT."
        }
        "Volumen sobre MDT" => "Volumen entre una superficie y el terreno, celda a celda.",
        "Métricas de rodal" => "Métricas de altura y densidad de la vegetación por celda.",
        "Segmentar árboles" => "Detecta los árboles y sus copas en un modelo de alturas.",
        "OBIA" => "Agrupa el modelo de alturas en objetos y los clasifica.",
        "Modelos de transición" => {
            "Relaciona una variable de campo con las métricas LiDAR de las parcelas y la valida."
        }
        "Cambio forestal" => {
            "Compara dos fechas y mide la pérdida y la ganancia de altura y de copa."
        }
        "Corredores eléctricos" => {
            "Conductores, apoyos y vegetación en riesgo de las líneas eléctricas."
        }
        "Recortar parcelas" => "Corta las nubes por parcelas y calcula sus métricas.",
        "Métodos de masa" => {
            "Inventario por área con nubes aéreas (ALS): métricas de rodal, recorte de parcelas y modelos de transición."
        }
        "Dendrometría" => {
            "Inventario con nubes de escáner terrestre (TLS): mide los fustes, los cubica y clasifica fustes y ramas."
        }
        "Dendrometría TLS" => {
            "Mide los fustes en nubes de escáner terrestre (diámetros a lo largo del tronco, altura y volumen con Smalian) y clasifica fustes y ramas."
        }
        // Ayuda
        "Manual" => "Abre el manual de uso (F1).",
        "Atajos de teclado" => "Lista de los atajos de teclado.",
        "Acerca de" => "Versión y licencias.",
        "Buscar comando" => "Busca cualquier comando por su nombre (Ctrl+K).",
        // Panel de edición
        "Todo" => "Selecciona todos los puntos (Ctrl+A).",
        "Invertir" => "Invierte la selección (Ctrl+I).",
        "Quitar" => "Quita la selección (Esc).",
        "Exportar puntos…" => "Exporta la nube, o solo la selección, a LAZ, ASCII o shapefile.",
        "Aislar selección" => {
            "Saca los puntos seleccionados a un archivo «…_sel» y deja solo ese en pantalla, con toda su densidad."
        }
        "Eliminar" => "Elimina los puntos seleccionados (Supr).",
        "Cambiar clase" => "Asigna la clase elegida a los puntos seleccionados.",
        "Quitar duplicados" => "Elimina los puntos repetidos y escribe una nube nueva.",
        "Histogramas" => {
            "Histograma de un atributo de los puntos, para detectar valores raros y seleccionarlos."
        }
        "Guardar nube editada…" => {
            "Escribe una nube nueva con las ediciones; el original no se modifica."
        }
        "Descartar todo" => "Descarta todas las ediciones y vuelve a la nube original.",
        "Deshacer" => "Deshace la última edición (Ctrl+Z).",
        "Rehacer" => "Rehace la edición deshecha (Ctrl+Y).",
        // Pestañas de capa
        "Zoom a la capa" => "Encuadra la capa.",
        "Guardar" | "Guardar *" => "Guarda la capa en su archivo.",
        "Guardar como…" => "Guarda la capa con otro nombre.",
        "Seleccionar todo" => "Selecciona todos los elementos de la capa.",
        "Invertir selección" => "Invierte la selección de elementos.",
        "Limpiar selección" => "Quita la selección de elementos.",
        _ => {
            #[cfg(debug_assertions)]
            {
                use std::sync::Mutex;
                static SEEN: Mutex<Vec<String>> = Mutex::new(Vec::new());
                if let Ok(mut s) = SEEN.lock()
                    && !s.contains(&key)
                {
                    eprintln!("botón sin texto emergente: {key}");
                    s.push(key);
                }
            }
            return None;
        }
    };
    Some(t)
}

#[cfg(test)]
mod tests {
    use super::tip;

    /// The string literals of the arguments of the call whose `(` is at `open`.
    fn literals(src: &str, open: usize) -> Vec<String> {
        let b = src.as_bytes();
        let (mut depth, mut i, mut out) = (0usize, open, Vec::new());
        while i < b.len() {
            match b[i] {
                b'(' => depth += 1,
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                b'"' => {
                    let start = i + 1;
                    i += 1;
                    while i < b.len() && b[i] != b'"' {
                        if b[i] == b'\\' {
                            i += 1;
                        }
                        i += 1;
                    }
                    out.push(src[start..i].replace("\\n", " "));
                }
                _ => {}
            }
            i += 1;
        }
        out
    }

    #[test]
    fn every_ribbon_button_has_a_tooltip() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut files = vec![dir.clone()];
        files.extend(
            std::fs::read_dir(dir.join("tools"))
                .into_iter()
                .flatten()
                .flatten()
                .map(|e| e.path()),
        );
        let mut missing = Vec::new();
        let mut stack = files;
        while let Some(p) = stack.pop() {
            if p.is_dir() {
                stack.extend(
                    std::fs::read_dir(&p)
                        .into_iter()
                        .flatten()
                        .flatten()
                        .map(|e| e.path()),
                );
                continue;
            }
            if p.extension().is_none_or(|e| e != "rs")
                || p.ends_with("tips.rs")
                || p.ends_with("ribbon.rs")
            {
                continue;
            }
            let src = std::fs::read_to_string(&p).unwrap_or_default();
            for name in ["ribbon::big_button(", "ribbon::small_button("] {
                for (at, _) in src.match_indices(name) {
                    let lits = literals(&src, at + name.len() - 1);
                    if !lits.is_empty() && !lits.iter().any(|l| tip(l).is_some()) {
                        missing.push(format!("{}: {:?}", p.display(), lits));
                    }
                }
            }
        }
        assert!(
            missing.is_empty(),
            "buttons with no tooltip in tips.rs:\n{}",
            missing.join("\n")
        );
    }
}
