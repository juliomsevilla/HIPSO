//! La ayuda emergente de cada botón de la cinta y de los paneles laterales, en un solo sitio. Un
//! botón encuentra su texto por su etiqueta, así que un botón nuevo solo necesita una línea aquí.
//! En las compilaciones de depuración un botón sin texto lo dice una vez en el terminal.

/// Lo que hace el botón llamado `label` (y su atajo), si tiene un texto.
pub fn tip(label: &str) -> Option<&'static str> {
    let key = label.replace('\n', " ");
    let t = match key.as_str() {
        "Extraer valores" => "Añade a una capa un campo por banda de uno o varios rásteres, con el valor bajo cada elemento (por ejemplo el CHM en cada parcela o árbol).",
        "Unir tabla" => "Une a una capa las columnas de una tabla (csv, texto o Excel) por un campo clave, como el inventario de campo a las parcelas.",
        "Combinar capas" => "Junta varias capas del mismo tipo de geometría en una; los campos son la unión de los de todas.",
        "Dividir por campo" => "Escribe una capa por cada valor distinto de un campo (un archivo por rodal, por especie…).",
        "Recortar capa" => "Recorta una capa de puntos, líneas o polígonos con los polígonos de otra, o borra de ella lo que cubren.",
        "Cercanía" => "Añade a cada elemento la distancia al elemento más cercano de otra capa, su número de orden y, si se pide, uno de sus campos.",
        "Resumen estadístico" => "Recuento, suma, media, mínimo, máximo y desviación de campos numéricos de una capa o tabla, por grupos de un campo.",
        "Rasterizar" => "Convierte una capa vectorial en ráster: cada celda recibe el valor de un campo del último elemento que la cubre.",
        "Filtro focal" => "Media, mediana, mínimo, máximo, suma, rango, desviación, mayoría o gaussiano de cada celda sobre una ventana a su alrededor.",
        "Estadísticas ráster" => "Recuento, mínimo, máximo, media, desviación, mediana, percentiles e histograma de un ráster.",
        "Tabular áreas" => "Área de cada clase de un ráster dentro de cada zona de otro, como tabla CSV.",
        // Preparar (LiDAR) y flujos
        "Exportar puntos" => "Saca los puntos a LAZ, LAS, texto o shapefile 3D, con filtro por clases, uno de cada n puntos y alturas sobre el suelo si se pide.",
        "Colorear con imagen" => "Da a cada punto el color del píxel de una imagen (normalmente una ortofoto) del mismo sitio.",
        "Normalizar intensidad" => "Corrige la intensidad por la distancia al escáner, para que la misma superficie tenga el mismo valor en cualquier punto de la pasada.",
        "Análisis por objetos" => "Agrupa el modelo de alturas en objetos y los clasifica (OBIA).",
        "Guardar flujo como…" => "Guarda el flujo con otro nombre.",
        "Quitar nodos" => "Quita los nodos seleccionados del lienzo (Supr).",
        "Nuevo flujo" => "Vacía el lienzo para empezar otro flujo.",
        "Abrir flujo" => "Abre un flujo guardado (.model.json).",
        "Guardar flujo" => "Guarda el flujo en su archivo.",
        "Ejemplos" => "Carga uno de los flujos de ejemplo que trae el programa.",
        "Ejecutar" => "Ejecuta el flujo en segundo plano; no empieza mientras haya problemas listados bajo el lienzo.",
        "Cancelar" => "Detiene el flujo en marcha y borra lo que esa ejecución había escrito.",
        "Duplicar" => "Duplica los nodos seleccionados (Ctrl+D).",
        "Ordenar" => "Coloca los nodos en columnas según su distancia a las entradas.",
        "Encuadrar" => "Pone todos los nodos a la vista.",
        // Análisis del terreno
        "Pendiente" => "Inclinación del terreno en grados o porcentaje.",
        "Orientación" => "Hacia dónde mira la ladera, en grados desde el norte.",
        "Sombreado" => "Iluminación del relieve con la dirección y la altura de luz que se elijan.",
        "Curvatura en planta" => "Curvatura de las curvas de nivel: indica si el agua converge o diverge.",
        "Curvatura de perfil" => "Curvatura en el sentido de la pendiente: indica si el agua acelera o frena.",
        "Curvatura total" => "Curvatura total de la superficie.",
        "Rugosidad" => "Diferencia entre la celda más alta y la más baja de la ventana de 3×3.",
        "Índice de rugosidad (TRI)" => "Media de las diferencias de altura con las ocho celdas vecinas.",
        "Posición topográfica (TPI)" => "Altura de cada celda menos la media del círculo de radio dado: positivo en lomas, negativo en vaguadas.",
        "Profundidad de depresiones" => "Cuánto hay que rellenar cada depresión para que el agua salga del modelo.",
        "Acumulación de flujo" => "Superficie que vierte a cada celda, con direcciones D8.",
        "Índice de humedad (TWI)" => "Índice topográfico de humedad: alto donde el agua se acumula y la pendiente es baja.",
        "Red de drenaje" => "Celdas por las que vierte al menos la superficie mínima indicada.",
        "Subcuencas" => "Cuenca de cada tramo de la red de drenaje.",
        "Cuenca visual" => "Qué celdas ve un observador situado en un punto, a una altura y con un alcance dados.",
        "Cubicación de pilas" => "Mide el volumen de material de cada pila sobre el terreno.",
        "Desmonte y terraplén" => "Calcula la tierra a quitar y a añadir para llegar a una cota o a una superficie de proyecto.",
        // Inicio
        "Abrir" => {
            "Abre nubes de puntos (LAS/LAZ), rásteres, imágenes y capas vectoriales; se añaden a lo que ya está abierto (Ctrl+O)."
        }
        "Añadir nubes y capas" => {
            "Añade nubes de puntos (LAS/LAZ), rásteres, imágenes y capas vectoriales a lo que ya está abierto (Ctrl+O)."
        }
        "Restablecer" => "Vuelve el brillo, el contraste y el gamma a sus valores iniciales.",
        "Percentiles 2–98 %" => "Ajusta el rango de colores a los percentiles 2 y 98 de los valores, sin contar los extremos.",
        "Mín–máx" => "Ajusta el rango de colores al valor mínimo y al máximo.",
        "Ajustar a bordes" => "Al digitalizar o mover vértices, el punto salta al borde más cercano de cualquier capa visible, aunque no haya un vértice allí.",
        "Reformar" => "Con la herramienta de digitalizar sobre polígonos, un trazo que cruza el contorno al menos dos veces sustituye el tramo más corto del borde por el trazo (corta una esquina o añade un saliente).",
        "Transformar selección" => "Mueve, gira y escala los elementos seleccionados alrededor del centro de la selección.",
        "Copiar" => "Copia los elementos seleccionados para pegarlos en otra capa del mismo tipo de geometría.",
        "Pegar" => "Pega en la capa activa los elementos copiados; los campos se emparejan por nombre.",
        "Pegar como capa" => "Crea una capa nueva con los elementos copiados.",
        "Comprobar topología" => "Lista los polígonos que se solapan, los huecos entre ellos y los contornos que se cruzan; en líneas, los cabos sueltos.",
        "Unir seleccionados" => "Une los polígonos seleccionados en uno solo, con los atributos del primero; el límite entre los que se tocan desaparece.",
        "Separar partes" => "Convierte cada parte de un elemento de varias partes en un elemento propio, con los mismos atributos.",
        "Agrupar en multiparte" => "Junta los elementos seleccionados en uno solo con varias partes, sin unir la geometría.",
        "Seleccionar por localización" => "Selecciona los elementos que intersectan, están dentro de, contienen, tocan o cruzan a los de otra capa.",
        "Seleccionar por expresión" => "Selecciona los elementos para los que se cumple una expresión sobre sus campos y su geometría.",
        "Calculadora de campos" => "Rellena un campo con una expresión sobre los otros campos y la geometría ($area, $length…).",
        "Eliminar seleccionados" => "Borra de la capa los elementos seleccionados (se puede deshacer).",
        "Seleccionar" => "Selecciona elementos de la capa con un clic o arrastrando un rectángulo; pulsa de nuevo para dejar de seleccionar.",
        "Geoprocesos" => {
            "Operaciones con capas de polígonos: estadística zonal, validar, reparar, búfer, disolver, unión e intersección."
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
            "Crea una capa vectorial vacía (puntos, líneas o polígonos) para digitalizar sobre la planta."
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
            "Reproyecta, recorta a la extensión y remuestrea varios rásteres a las celdas de uno de referencia (misma rejilla, proyección y tamaño de celda)."
        }
        "Tamizar" => {
            "Quita los parches pequeños de un ráster de clases: toman el valor del vecino mayor."
        }
        "Reclasificar" => {
            "Cambia los valores de un ráster con una tabla de reglas: un valor, un intervalo o una comparación pasan a un valor nuevo o a «sin datos»."
        }
        "Cortar por máscara" => {
            "Vacía lo que queda fuera de una capa de polígonos o de otro ráster y, si quieres, corta el ráster a su caja."
        }
        "Proximidad" => "Calcula para cada celda la distancia a la celda más cercana de unos valores.",
        "Voronoi" => "Un polígono de Voronoi por cada punto de una capa, con sus atributos.",
        "Calculadora" => {
            "Calcula una expresión celda a celda con varios rásteres: operaciones, funciones y condiciones."
        }
        "Vectorizar" => {
            "Convierte un ráster en polígonos con el valor como atributo: por clase, por rango o por intervalos."
        }
        "Rellenar huecos" => {
            "Rellena las celdas vacías de un ráster con las celdas con valor de alrededor."
        }
        "Reproyectar" => {
            "Pasa nubes y rásteres a otro sistema de coordenadas y cambia la referencia vertical de las alturas."
        }
        "Tema claro" | "Tema oscuro" => "Cambia entre el tema claro y el oscuro.",
        "Registro" => "Muestra lo que se ha hecho, los errores y el avance de los procesos largos.",
        // Vista
        "2D" => "Solo la planta.",
        "2D + perfil" => "La planta y el perfil.",
        "3D" => "Solo la vista 3D.",
        "2D + perfil + 3D" => "La planta, el perfil y la vista 3D a la vez.",
        "Zoom adelante" => "Vuelve a la vista que había antes de «Zoom anterior».",
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
            "Guarda qué nubes y capas están abiertas y cómo se ve (archivo .hipso)."
        }
        "Abrir proyecto" => {
            "Abre un proyecto .hipso (o .elp, el formato anterior): cierra lo abierto y carga sus nubes y capas."
        }
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
        // LiDAR
        "Información de los datos" => {
            "Analiza las nubes: puntos, densidad, clases, coordenadas y avisos."
        }
        "Clasificar" => "Clasifica ruido, suelo, edificios, vegetación, cables y postes.",
        "Asignar proyección" => "Asigna a una nube el sistema de coordenadas que le falta.",
        "Adelgazar nube" => "Reduce la densidad de la nube conservando su forma.",
        "Comparar nubes" => "Mide las diferencias entre dos nubes del mismo lugar.",
        "Ajustar nubes" => "Alinea una nube a otra de referencia.",
        "Líneas de vuelo" => "Encuentra las líneas de vuelo de una nube aérea por los huecos en el tiempo GPS.",
        "Imagen de intensidad" => "Ráster GeoTIFF de la intensidad media de los puntos en cada celda, una imagen de la nube en tonos de gris.",
        "Densidad de Voronoi" => "Densidad de puntos por celda con celdas de Voronoi, con el porcentaje que llega a un umbral.",
        "Entre pasadas" => "Mide cuánto se separan entre sí las pasadas de vuelo.",
        "MDT" => "Modelo digital del terreno a partir de los puntos de suelo.",
        "MDS / CHM" => "Modelo digital de superficie y de alturas de la vegetación (CHM).",
        "Cubicación\nde pilas" | "Cubicación de pilas" => {
            "Mide el volumen de material de cada pila sobre una superficie base hecha desde su contorno."
        }
        "Desmonte y\nterraplén" | "Desmonte y terraplén" => {
            "Calcula la tierra a quitar y a añadir para llevar un terreno a un diseño."
        }
        "Inundación" => {
            "Simula el agua de uno o varios niveles sobre un MDT desde un origen: área, volumen y profundidad."
        }
        "Crestas y drenaje" => {
            "Traza las líneas de cresta o de drenaje de un MDT, con el orden de Strahler."
        }
        "Ruta de menor coste" => {
            "Busca el camino más barato sobre un MDT entre puntos, por pendiente, tiempo de marcha o un ráster de costes."
        }
        "Zonas encharcadas" => {
            "Delimita donde se queda el agua en un MDT: cada charco con su área, profundidad, volumen y cota de desborde."
        }
        "Análisis de terreno" => {
            "Pendiente, orientación, curvatura, sombreado e hidrología a partir de un MDT."
        }
        "Volumen sobre MDT" => "Volumen entre una superficie y el terreno, celda a celda.",
        "Métricas por celda" => "Métricas de altura y densidad de la vegetación por celda.",
        "Segmentar individuos" => "Detecta los individuos y sus copas en un modelo de alturas.",
        "OBIA" => "Agrupa el modelo de alturas en objetos y los clasifica.",
        "Modelos lineales" => {
            "Relaciona una variable de campo con las métricas LiDAR de las unidades de muestreo y la valida."
        }
        "Cambio de la cubierta vegetal" | "Cambio de la\ncubierta vegetal" => {
            "Compara dos fechas y mide la pérdida y la ganancia de altura y de cubierta vegetal."
        }
        "Corredores eléctricos" => {
            "Conductores, apoyos y vegetación en riesgo de las líneas eléctricas."
        }
        "Extraer muestras" => "Corta las nubes por unidades de muestreo y calcula sus métricas.",
        "Recortar con polígonos" => {
            "Recorta las nubes con los polígonos de una capa: una nube por polígono o una sola ajustada al perímetro de todos, con búfer opcional."
        }
        "Medición de troncos" => {
            "Inventario con nubes de escáner terrestre (TLS): mide los troncos, estima su volumen y clasifica troncos y ramas."
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
        "Unir nubes" => "Une dos o más nubes en una nueva y decide qué hacer con los puntos repetidos.",
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
        k if crate::vector_ui::Op::ALL.iter().any(|o| o.label() == k) => crate::vector_ui::Op::ALL
            .iter()
            .find(|o| o.label() == k)
            .map_or("", |o| o.help()),
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

    /// Los literales de cadena de los argumentos de la llamada cuyo `(` está en `open`.
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
