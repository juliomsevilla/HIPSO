//! Más clases para la tubería de clasificación, a partir de la geometría de los puntos que rodean a
//! cada uno: vegetación en tres estratos de altura (baja 3, media 4, alta 5), cables (14) y postes
//! o torres (15). Se ejecutan después del suelo y los edificios sobre las mismas teselas, así que
//! saben qué puntos son suelo.
//!
//! Los cables y los postes miran solo los puntos por encima del suelo. En el entorno de un punto,
//! los tres autovalores l1 ≥ l2 ≥ l3 de la covarianza dicen cómo están dispuestos los puntos: una
//! línea tiene linealidad (l1 − l2)/l1 cercana a 1. Un cable es una línea casi horizontal muy por
//! encima del suelo, en el aire y larga: las líneas de barrido de un vuelo hacen rectos trozos de
//! copa durante uno o dos metros, así que las cadenas cortas y los entornos concurridos no son
//! cables. Un poste o torre es una estructura estrecha, aislada de las copas, que se eleva del
//! suelo.

use std::collections::HashMap;

use rayon::prelude::*;

use crate::grid::Grid;
use crate::las::Point;

pub const LOW_VEGETATION: u8 = 3;
pub const MEDIUM_VEGETATION: u8 = 4;
pub const HIGH_VEGETATION: u8 = 5;
pub const WIRE: u8 = 14;
pub const POLE: u8 = 15;
pub const WATER: u8 = 9;
pub const ROAD: u8 = 11;

/// Espaciado de los puntos (m) a partir del cual una torre tiene 12 puntos por metro de altura o
/// menos.
const POLE_REFERENCE_SPACING: f64 = 0.33;

/// Radio (m) en el que un trozo de conductor no tiene más puntos que los suyos.
const ALONE: f64 = 2.5;

/// Las distancias verticales cuentan tanto al enlazar los puntos de una torre: sus piezas son filas
/// de puntos separadas unos metros (un entramado escaneado desde arriba), y un hueco vertical de 8
/// m enlaza.
const POLE_Z_SCALE: f64 = 0.19;

/// Cuántas veces sus propios puntos puede tener un poste de copa a su alrededor (por encima de un
/// tercio de su altura).
const CROWD_TOLERANCE: f64 = 1.6;

/// Cuántas veces sus propios puntos puede tener un poste fino a su alrededor en la punta (un tronco
/// tiene su copa).
const CAP_TOLERANCE: f64 = 3.0;

/// Puntos por metro de altura que puede tener una torre a 9 puntos/m² (una torre de alta tensión
/// con varias crucetas tiene más que un poste).
const TOWER_POINTS_PER_METRE: f64 = 40.0;

/// Una estructura al menos así de alta es una torre, cuyos puntos crecen con la densidad del vuelo;
/// un árbol estrecho de esa altura a 45 puntos/m² tiene tantos, así que las más bajas conservan el
/// recuento de 9.
const TOWER_HEIGHT: f64 = 15.0;

/// Factor de densidad (véase `POLE_REFERENCE_SPACING`) por encima del cual los troncos de los
/// árboles se resuelven como líneas.
const TRUNK_DENSITY: f64 = 20.0;

/// Los trozos de conductor más bajos enlazados a través de huecos están por encima del suelo (m):
/// un vuelo los ve en trozos solo cuando la línea está muy por encima de todo.
const LINKED_MIN_HEIGHT: f64 = 10.0;

/// Espaciado (m) a partir del cual se miran los cables con más de un radio.
const WIRE_SPARSE_SPACING: f64 = 0.25;

/// Altura mínima de la cabeza de una torre sobre el suelo (m).
const HEAD_MIN_HEIGHT: f64 = 8.0;

#[derive(Clone, Debug, PartialEq)]
pub struct VegParams {
    /// Altura sobre el suelo hasta la que la vegetación es baja y hasta la que es media.
    pub low: f64,
    pub medium: f64,
    /// Celda de la superficie del suelo de la que salen las alturas.
    pub cell: f64,
}

impl Default for VegParams {
    fn default() -> Self {
        Self {
            low: 0.5,
            medium: 2.0,
            cell: 1.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct WireParams {
    /// Altura mínima de un cable sobre el suelo.
    pub min_height: f64,
    /// Radio del entorno (m; 0: seis veces el espaciado de los puntos, entre 0,6 y 3 m, y con un
    /// espaciado de 0,25 m o más también 1,5 y 2 veces eso, hasta 4,5 m: el mismo cable es una
    /// línea a 1 m con 45 puntos/m² y solo a 2 a 4 m con 9, donde un conductor tiene un punto cada
    /// uno o dos metros y las copas a su lado lo hacen una línea a unos radios y no a otros).
    pub radius: f64,
    /// Smallest linearity (0–1).
    pub linearity: f64,
    /// Mayor pendiente de la línea (|dz| de su dirección, 0 horizontal – 1 vertical).
    pub max_slope: f64,
    /// Un cable cuelga en el aire: como mucho este número de puntos por encima de `min_height` a
    /// menos de `2 × radius` de él. Una rama dentro de una copa tiene decenas.
    pub max_neighbours: usize,
    /// Mayor anchura que puede tener el entorno a través de su dirección principal (desviación
    /// típica, m): un conductor es una línea, una rama en una copa tiene anchura. A diferencia de
    /// un recuento de vecinos, no depende de la densidad de los puntos.
    pub max_width: f64,
    /// Un cable es continuo: la cadena de puntos de cable (cada uno a menos de `2 × radius` del
    /// siguiente) debe llegar tan lejos. Las líneas de barrido de un vuelo hacen parecer rectos
    /// trozos de copa durante uno o dos metros, nunca durante varios.
    pub min_length: f64,
    /// Una cadena de puntos de cable es una línea: su longitud sobre su anchura (las raíces
    /// cuadradas de los dos mayores autovalores de sus puntos). Un seto o un trozo de copa que la
    /// linealidad de sus partes deja pasar es una mancha a esa escala.
    pub min_straightness: f64,
    /// Lado de los vóxeles a los que se reducen los puntos antes de mirar su forma (m).
    pub voxel: f64,
    /// Un vuelo con pocas pasadas sobre una línea ve un conductor solo donde sus líneas de barrido
    /// lo cruzan: trozos rectos cortos (uno o dos metros) con metros de nada entre ellos. Los
    /// trozos que apuntan unos a otros, como mucho a esa distancia (m, 0 para desactivar), forman
    /// un cable cuando tres o más se alinean a lo largo de al menos el doble de `min_length`.
    pub link_gap: f64,
}

impl Default for WireParams {
    fn default() -> Self {
        Self {
            min_height: 5.0,
            radius: 0.0,
            linearity: 0.7,
            max_slope: 0.5,
            max_neighbours: 1000,
            max_width: 0.0,
            min_length: 10.0,
            min_straightness: 10.0,
            voxel: 0.05,
            link_gap: 15.0,
        }
    }
}

/// Los postes y las torres se hallan como estructuras aisladas y estrechas que se elevan del suelo:
/// a la densidad de un vuelo, una torre de celosía es una columna suelta de puntos, no una línea
/// limpia, así que lo que la marca es su forma y que nada la toca.
#[derive(Clone, Debug, PartialEq)]
pub struct PoleParams {
    /// Los puntos más bajos que esto sobre el suelo no forman parte de una estructura.
    pub min_height: f64,
    /// Los puntos más cercanos que esto pertenecen a la misma estructura.
    pub radius: f64,
    /// Mayor anchura que puede tener la estructura (m, a cada lado de la huella).
    pub max_width: f64,
    /// Lo mínimo que debe elevarse (m).
    pub min_extent: f64,
    /// A la densidad de un vuelo solo puede volver la cabeza de una torre: una cruceta de unos
    /// metros de largo, en lo alto, sin nada debajo. Una cadena corta y aislada de puntos así de
    /// alta y así de larga (m, 0 para desactivar) se toma por la cabeza de una torre. Un conductor
    /// es más largo, y una copa tiene copa debajo.
    pub head_length: f64,
    /// Postes finos (farolas, postes de servicios): una línea vertical de puntos, estrecha, que se
    /// eleva al menos tanto (m; 0 para desactivar).
    pub thin_extent: f64,
}

impl Default for PoleParams {
    fn default() -> Self {
        Self {
            min_height: 2.0,
            radius: 1.5,
            max_width: 8.0,
            min_extent: 6.0,
            head_length: 12.0,
            thin_extent: 3.0,
        }
    }
}

/// Las carreteras son suelo liso, llano y oscuro para el láser: el asfalto devuelve menos
/// intensidad que la tierra, y la superficie de una carretera es un plano (rugosidad y pendiente
/// pequeñas). Lo que se mide es la forma de los puntos de suelo dentro de `radius`.
#[derive(Clone, Debug, PartialEq)]
pub struct RoadParams {
    /// Mayor intensidad de un punto de carretera, en la escala propia del sensor (el asfalto es
    /// oscuro); 0: una proporción (`relative_intensity`) de la mediana de intensidad del suelo de
    /// la tesela, que es la misma sea cual sea el sensor, su escala (16 bits, 0 a 255) o el vuelo.
    pub max_intensity: f64,
    /// Con `max_intensity` 0, la proporción de la mediana de intensidad del suelo que puede
    /// alcanzar una carretera.
    pub relative_intensity: f64,
    /// Mayor rugosidad que puede tener la superficie: desviación típica respecto al plano local
    /// (m).
    pub max_roughness: f64,
    /// Mayor pendiente que puede tener el plano local (grados).
    pub max_slope: f64,
    /// Radio del entorno (m).
    pub radius: f64,
}

impl Default for RoadParams {
    fn default() -> Self {
        Self {
            max_intensity: 0.0,
            relative_intensity: 0.6,
            max_roughness: 0.03,
            max_slope: 5.0,
            radius: 1.5,
        }
    }
}

/// El agua es una lámina plana que devuelve casi nada al láser: puntos de suelo de un solo retorno,
/// de baja intensidad, nivelados, que forman una región extensa cuyas alturas apenas varían. Se
/// unen las celdas de `cell` metros que contienen esos puntos; una región es agua cuando tiene al
/// menos `min_area` metros cuadrados y sus alturas se desvían menos de `max_deviation`.
#[derive(Clone, Debug, PartialEq)]
pub struct WaterParams {
    /// Mayor intensidad de un punto de agua, en la escala propia del sensor; 0: una proporción
    /// (`relative_intensity`) de la mediana de intensidad del suelo de la tesela.
    pub max_intensity: f64,
    /// Con `max_intensity` 0, la proporción de la mediana de intensidad del suelo que puede
    /// alcanzar el agua.
    pub relative_intensity: f64,
    /// Mayor pendiente que puede tener el plano local (grados).
    pub max_slope: f64,
    /// Lado de las celdas que se unen (m).
    pub cell: f64,
    /// Smallest region (m²).
    pub min_area: f64,
    /// Mayor desviación típica de las alturas de una región (m).
    pub max_deviation: f64,
}

impl Default for WaterParams {
    fn default() -> Self {
        Self {
            max_intensity: 0.0,
            relative_intensity: 0.375,
            max_slope: 3.0,
            cell: 2.0,
            min_area: 30.0,
            max_deviation: 0.2,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Extras {
    pub roads: Option<RoadParams>,
    pub water: Option<WaterParams>,
    pub vegetation: Option<VegParams>,
    pub wires: Option<WireParams>,
    pub poles: Option<PoleParams>,
}

impl Extras {
    pub fn any(&self) -> bool {
        self.vegetation.is_some()
            || self.wires.is_some()
            || self.poles.is_some()
            || self.roads.is_some()
            || self.water.is_some()
    }

    /// Una etiqueta para nombres de archivo.
    pub fn tag(&self) -> String {
        let mut t = String::new();
        if self.vegetation.is_some() {
            t.push_str("_veg");
        }
        if self.wires.is_some() {
            t.push_str("_cables");
        }
        if self.poles.is_some() {
            t.push_str("_postes");
        }
        if self.roads.is_some() {
            t.push_str("_carreteras");
        }
        if self.water.is_some() {
            t.push_str("_agua");
        }
        t
    }
}

/// Lo mismo, para los ejemplos que miran alturas.
#[doc(hidden)]
pub fn ground_surface_pub(pts: &[Point], ground: &[bool], cell: f64) -> Option<Grid> {
    ground_surface(pts, ground, cell)
}

/// Superficie por los puntos de suelo, para alturas sobre el suelo.
pub(crate) fn ground_surface(pts: &[Point], ground: &[bool], cell: f64) -> Option<Grid> {
    let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for q in pts.iter().filter(|q| !q.is_noise()) {
        (x0, y0, x1, y1) = (x0.min(q.x), y0.min(q.y), x1.max(q.x), y1.max(q.y));
    }
    if x1 < x0 {
        return None;
    }
    let mut g = Grid::covering([x0, y0, x1, y1], cell).ok()?;
    let mut sum = vec![0f64; g.len()];
    let mut n = vec![0u32; g.len()];
    for (q, _) in pts.iter().zip(ground).filter(|(_, g)| **g) {
        if let Some(i) = g.index(q.x, q.y) {
            sum[i] += q.z;
            n[i] += 1;
        }
    }
    if n.iter().all(|c| *c == 0) {
        return None;
    }
    for i in 0..g.len() {
        g.data[i] = if n[i] > 0 {
            (sum[i] / f64::from(n[i])) as f32
        } else {
            f32::NAN
        };
    }
    g.fill_holes();
    Some(g)
}

type Cell = (i32, i32, i32);

/// Para cada punto de `set`, (linealidad, |dz| de la dirección principal, anchura a través de ella)
/// de sus vecinos de `set` a menos de `radius`; `None` con menos de cuatro.
fn line_features(pts: &[Point], set: &[usize], radius: f64) -> Vec<Option<(f64, f64, f64)>> {
    let key = |q: &Point| -> Cell {
        (
            (q.x / radius).floor() as i32,
            (q.y / radius).floor() as i32,
            (q.z / radius).floor() as i32,
        )
    };
    let mut cells: HashMap<Cell, Vec<u32>> = HashMap::new();
    for (j, &i) in set.iter().enumerate() {
        cells.entry(key(&pts[i])).or_default().push(j as u32);
    }
    let r2 = radius * radius;
    (0..set.len())
        .into_par_iter()
        .map(|j| {
            let q = &pts[set[j]];
            let (cx, cy, cz) = key(q);
            let mut nb: Vec<[f64; 3]> = Vec::new();
            for dx in -1..=1 {
                for dy in -1..=1 {
                    for dz in -1..=1 {
                        let Some(list) = cells.get(&(cx + dx, cy + dy, cz + dz)) else {
                            continue;
                        };
                        for &o in list {
                            let w = &pts[set[o as usize]];
                            if (w.x - q.x).powi(2) + (w.y - q.y).powi(2) + (w.z - q.z).powi(2) <= r2
                            {
                                nb.push([w.x, w.y, w.z]);
                            }
                        }
                    }
                }
            }
            if nb.len() < 4 {
                return None;
            }
            let (_, cov, _) = crate::tls::covariance(nb.iter().copied());
            let (vals, vec) = crate::tls::eigen(cov);
            if vals[0] <= 1e-12 {
                return None;
            }
            Some((
                (vals[0] - vals[1]) / vals[0],
                vec[2].abs(),
                vals[1].max(0.0).sqrt(),
            ))
        })
        .collect()
}

/// Para cada punto de `set`, cuántos otros puntos de `set` hay a menos de `radius`.
fn neighbour_counts(pts: &[Point], set: &[usize], radius: f64) -> Vec<usize> {
    let key = |q: &Point| -> Cell {
        (
            (q.x / radius).floor() as i32,
            (q.y / radius).floor() as i32,
            (q.z / radius).floor() as i32,
        )
    };
    let mut cells: HashMap<Cell, Vec<u32>> = HashMap::new();
    for (j, &i) in set.iter().enumerate() {
        cells.entry(key(&pts[i])).or_default().push(j as u32);
    }
    let r2 = radius * radius;
    (0..set.len())
        .into_par_iter()
        .map(|j| {
            let q = &pts[set[j]];
            let (cx, cy, cz) = key(q);
            let mut n = 0;
            for dx in -1..=1 {
                for dy in -1..=1 {
                    for dz in -1..=1 {
                        for &o in cells
                            .get(&(cx + dx, cy + dy, cz + dz))
                            .into_iter()
                            .flatten()
                        {
                            let w = &pts[set[o as usize]];
                            if (w.x - q.x).powi(2) + (w.y - q.y).powi(2) + (w.z - q.z).powi(2) <= r2
                            {
                                n += 1;
                            }
                        }
                    }
                }
            }
            n - 1
        })
        .collect()
}

/// La caja envolvente de cada cadena `[x0, y0, z0, x1, y1, z1]` y su tamaño, por cadena.
type ChainBoxes = HashMap<usize, ([f64; 6], usize)>;

/// Agrupa los puntos `cand` (índices en `pts`) en cadenas: dos puntos a menos de `link` uno del
/// otro están en la misma. Devuelve el grupo de cada punto y la caja envolvente `[x0, y0, z0, x1,
/// y1, z1]` y el tamaño de cada grupo.
fn chains(pts: &[Point], cand: &[usize], link: f64) -> (Vec<usize>, ChainBoxes) {
    chains_z(pts, cand, link, 1.0)
}

/// [`chains`] donde las alturas cuentan `zscale` veces lo que las distancias en planta (por debajo
/// de 1, dos puntos tan separados en vertical siguen enlazados: las piezas de una torre de celosía
/// son filas de puntos con metros entre ellas).
fn chains_z(
    pts: &[Point],
    cand: &[usize],
    link: f64,
    zscale: f64,
) -> (Vec<usize>, ChainBoxes) {
    let group = crate::cluster::link_labels(
        cand.len(),
        |j| {
            let q = &pts[cand[j]];
            [q.x, q.y, q.z]
        },
        link,
        zscale,
    );
    let mut boxes: HashMap<usize, ([f64; 6], usize)> = HashMap::new();
    for (&g, &i) in group.iter().zip(cand) {
        let q = &pts[i];
        let (b, n) = boxes.entry(g).or_insert((
            [f64::MAX, f64::MAX, f64::MAX, f64::MIN, f64::MIN, f64::MIN],
            0,
        ));
        for (k, v) in [q.x, q.y, q.z].into_iter().enumerate() {
            b[k] = b[k].min(v);
            b[k + 3] = b[k + 3].max(v);
        }
        *n += 1;
    }
    (group, boxes)
}

/// Suelo que el paso aún puede nombrar: no tomado por otra clase.
fn plain_ground(pts: &[Point], ground: &[bool], class: &[Option<u8>], k: usize) -> bool {
    ground[k] && !pts[k].is_noise() && class[k].is_none_or(|c| c == 2)
}

/// Los puntos con los que no son suelo normal apartados (retenidos), para que la forma de un
/// entorno sea la forma del suelo.
fn ground_only(pts: &[Point], ground: &[bool], class: &[Option<u8>]) -> Vec<Point> {
    pts.iter()
        .enumerate()
        .map(|(k, q)| {
            let mut q = *q;
            if !plain_ground(pts, ground, class, k) {
                q.withheld = true;
            }
            q
        })
        .collect()
}

/// Puntos de carretera: suelo liso, llano y oscuro.
fn roads(pts: &[Point], ground: &[bool], class: &mut [Option<u8>], p: &RoadParams) {
    let limit = intensity_limit(pts, ground, class, p.max_intensity, p.relative_intensity);
    let wanted: Vec<bool> = (0..pts.len())
        .map(|k| plain_ground(pts, ground, class, k) && f64::from(pts[k].intensity) <= limit)
        .collect();
    let only = ground_only(pts, ground, class);
    let f = crate::features::compute(&only, &wanted, p.radius);
    for (k, f) in f.iter().enumerate() {
        if let Some(f) = f
            && f64::from(f.roughness) <= p.max_roughness
            && f64::from(f.slope) <= p.max_slope
        {
            class[k] = Some(ROAD);
        }
    }
}

/// La mayor intensidad que cuenta como oscura: `absolute` cuando está fijada o, si no, `relative`
/// veces la mediana de intensidad del suelo normal de la tesela (0 cuando no hay).
fn intensity_limit(
    pts: &[Point],
    ground: &[bool],
    class: &[Option<u8>],
    absolute: f64,
    relative: f64,
) -> f64 {
    if absolute > 0.0 {
        return absolute;
    }
    let mut v: Vec<f64> = (0..pts.len())
        .filter(|&k| plain_ground(pts, ground, class, k))
        .map(|k| f64::from(pts[k].intensity))
        .collect();
    if v.is_empty() {
        return 0.0;
    }
    relative * crate::cluster::median(&mut v)
}

/// Puntos de agua: regiones extensas y niveladas de suelo oscuro de retorno único.
fn water(pts: &[Point], ground: &[bool], class: &mut [Option<u8>], p: &WaterParams) {
    let limit = intensity_limit(pts, ground, class, p.max_intensity, p.relative_intensity);
    let wanted: Vec<bool> = (0..pts.len())
        .map(|k| {
            plain_ground(pts, ground, class, k)
                && pts[k].number_of_returns <= 1
                && f64::from(pts[k].intensity) <= limit
        })
        .collect();
    if !wanted.iter().any(|w| *w) {
        return;
    }
    let only = ground_only(pts, ground, class);
    let f = crate::features::compute(&only, &wanted, 2.0 * p.cell);
    let mut cells: HashMap<(i32, i32), Vec<usize>> = HashMap::new();
    for (k, f) in f.iter().enumerate() {
        if let Some(f) = f
            && f64::from(f.slope) <= p.max_slope
        {
            cells
                .entry((
                    (pts[k].x / p.cell).floor() as i32,
                    (pts[k].y / p.cell).floor() as i32,
                ))
                .or_default()
                .push(k);
        }
    }
    // regiones de celdas conectadas en 8 direcciones
    let mut seen: std::collections::HashSet<(i32, i32)> = std::collections::HashSet::new();
    let keys: Vec<(i32, i32)> = cells.keys().copied().collect();
    for start in keys {
        if !seen.insert(start) {
            continue;
        }
        let mut stack = vec![start];
        let mut members: Vec<(i32, i32)> = vec![start];
        while let Some((cx, cy)) = stack.pop() {
            for dx in -1..=1 {
                for dy in -1..=1 {
                    let n = (cx + dx, cy + dy);
                    if cells.contains_key(&n) && seen.insert(n) {
                        stack.push(n);
                        members.push(n);
                    }
                }
            }
        }
        if (members.len() as f64) * p.cell * p.cell < p.min_area {
            continue;
        }
        let zs: Vec<f64> = members
            .iter()
            .flat_map(|c| cells[c].iter().map(|&k| pts[k].z))
            .collect();
        let n = zs.len() as f64;
        let mean = zs.iter().sum::<f64>() / n;
        let sd = (zs.iter().map(|z| (z - mean).powi(2)).sum::<f64>() / n).sqrt();
        if sd <= p.max_deviation {
            for c in &members {
                for &k in &cells[c] {
                    class[k] = Some(WATER);
                }
            }
        }
    }
}

/// Distancia media entre los puntos de una tesela (raíz cuadrada del área por punto, la caja
/// envolvente entre los puntos que no son ruido); `None` con menos de 100 puntos.
fn spacing(pts: &[Point]) -> Option<f64> {
    let (mut x0, mut y0, mut x1, mut y1, mut n) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN, 0u64);
    for q in pts.iter().filter(|q| !q.is_noise()) {
        (x0, y0, x1, y1, n) = (x0.min(q.x), y0.min(q.y), x1.max(q.x), y1.max(q.y), n + 1);
    }
    let area = (x1 - x0) * (y1 - y0);
    (n >= 100 && area > 0.0).then(|| (area / n as f64).sqrt())
}

/// Los radios con los que se mira un punto para ver si es parte de un cable: el de la tesela (o el
/// que dan los parámetros) y, en un vuelo disperso (0,25 m entre puntos o más), 1,5 y 2 veces ese.
/// Un cable es una línea al radio que alcanza su siguiente punto y no tiene copa a su lado, y ese
/// radio no es el mismo a lo largo de la línea.
fn wire_radii(pts: &[Point], w: &WireParams) -> Vec<f64> {
    let (base, _) = wire_scale(pts, w);
    let sparse = w.radius <= 0.0 && spacing(pts).is_some_and(|s| s >= WIRE_SPARSE_SPACING);
    if sparse {
        vec![base, (1.5 * base).min(4.5), (2.0 * base).min(4.5)]
    } else {
        vec![base]
    }
}

/// Radio del entorno y mayor anchura que puede tener a través, para los cables de una tesela: los
/// parámetros o (cuando 0) a partir del espaciado de sus puntos.
fn wire_scale(pts: &[Point], w: &WireParams) -> (f64, f64) {
    let radius = if w.radius > 0.0 {
        w.radius
    } else {
        spacing(pts).map_or(1.5, |s| (6.0 * s).clamp(0.6, 3.0))
    };
    let width = if w.max_width > 0.0 {
        w.max_width
    } else {
        0.35 * radius
    };
    (radius, width)
}

/// Cuáles de los puntos `cand` pertenecen a una cadena larga (diagonal de la caja envolvente de al
/// menos `min_len`) y recta (la raíz cuadrada de su primer autovalor es al menos `straightness`
/// veces la del segundo).
fn straight_chains(
    pts: &[Point],
    cand: &[usize],
    link: f64,
    min_len: f64,
    straightness: f64,
) -> Vec<bool> {
    let (group, boxes) = chains(pts, cand, link);
    let mut members: HashMap<usize, Vec<[f64; 3]>> = HashMap::new();
    for (&g, &i) in group.iter().zip(cand) {
        let (b, _) = boxes[&g];
        let diag = ((b[3] - b[0]).powi(2) + (b[4] - b[1]).powi(2) + (b[5] - b[2]).powi(2)).sqrt();
        if diag >= min_len {
            members
                .entry(g)
                .or_default()
                .push([pts[i].x, pts[i].y, pts[i].z]);
        }
    }
    let straight: HashMap<usize, bool> = members
        .iter()
        .map(|(g, m)| {
            let (_, cov, n) = crate::tls::covariance(m.iter().copied());
            let (vals, _) = crate::tls::eigen(cov);
            let ok = n >= 4 && vals[1].max(1e-9).sqrt() * straightness <= vals[0].max(0.0).sqrt();
            (*g, ok)
        })
        .collect();
    group
        .iter()
        .map(|g| straight.get(g).copied().unwrap_or(false))
        .collect()
}

/// Cuáles de los puntos `set` son trozos de un conductor enlazados a través de huecos: trozos
/// cortos y compactos (0,4 a 4 m, puntos a menos de `link` unos de otros, reducidos a vóxeles;
/// finos y nivelados) que están en fila: tres de ellos, cada uno como mucho a `link_gap` del
/// siguiente, con el giro en el del medio por debajo de 8° y la pendiente de ambos tramos dentro de
/// `max_slope`, forman una cadena; las cadenas que abarcan al menos el doble de `min_length` son
/// cable. Los trozos son lo que ve un vuelo con pocas pasadas de un conductor: las líneas de
/// barrido lo cruzan aquí y allá, y entre ellas hay metros sin un punto. Tres trozos en fila por
/// casualidad, tan separados, apenas ocurre en una copa.
fn linked_wires(pts: &[Point], set: &[usize], w: &WireParams, link: f64) -> Vec<bool> {
    let vs = w.voxel.max(0.05);
    let mut voxel_of: HashMap<(i32, i32, i32), usize> = HashMap::new();
    let mut centres: Vec<Point> = Vec::new();
    let in_voxel: Vec<usize> = set
        .iter()
        .map(|&k| {
            let q = &pts[k];
            let key = (
                (q.x / vs).floor() as i32,
                (q.y / vs).floor() as i32,
                (q.z / vs).floor() as i32,
            );
            *voxel_of.entry(key).or_insert_with(|| {
                centres.push(Point {
                    x: (f64::from(key.0) + 0.5) * vs,
                    y: (f64::from(key.1) + 0.5) * vs,
                    z: (f64::from(key.2) + 0.5) * vs,
                    ..Point::default()
                });
                centres.len() - 1
            })
        })
        .collect();
    let all: Vec<usize> = (0..centres.len()).collect();
    let (group, boxes) = chains(&centres, &all, link);
    let mut members: HashMap<usize, Vec<[f64; 3]>> = HashMap::new();
    for (&g, &i) in group.iter().zip(&all) {
        let (b, n) = boxes[&g];
        let diag = ((b[3] - b[0]).powi(2) + (b[4] - b[1]).powi(2) + (b[5] - b[2]).powi(2)).sqrt();
        if n >= 8 && (0.4..=4.0).contains(&diag) && b[5] - b[2] <= 1.0 {
            members
                .entry(g)
                .or_default()
                .push([centres[i].x, centres[i].y, centres[i].z]);
        }
    }
    // lo que hay alrededor de un trozo: un conductor cuelga solo, un trozo de copa tiene copa a su
    // alrededor
    let mut around: HashMap<Cell, Vec<usize>> = HashMap::new();
    for (i, c) in centres.iter().enumerate() {
        around
            .entry((
                (c.x / ALONE).floor() as i32,
                (c.y / ALONE).floor() as i32,
                (c.z / ALONE).floor() as i32,
            ))
            .or_default()
            .push(i);
    }
    let alone = |g: usize, c: &[f64; 3]| -> bool {
        let (cx, cy, cz) = (
            (c[0] / ALONE).floor() as i32,
            (c[1] / ALONE).floor() as i32,
            (c[2] / ALONE).floor() as i32,
        );
        let mut others = 0;
        for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in -1..=1 {
                    for &i in around.get(&(cx + dx, cy + dy, cz + dz)).into_iter().flatten() {
                        let q = &centres[i];
                        if group[i] != g
                            && (q.x - c[0]).powi(2) + (q.y - c[1]).powi(2) + (q.z - c[2]).powi(2)
                                <= ALONE * ALONE
                        {
                            others += 1;
                        }
                    }
                }
            }
        }
        others <= 2
    };
    // (grupo, centro) de los trozos que son finos y están solos
    let mut pieces: Vec<(usize, [f64; 3])> = Vec::new();
    for (g, m) in &members {
        let (c, cov, _) = crate::tls::covariance(m.iter().copied());
        let (vals, _) = crate::tls::eigen(cov);
        if vals[1].max(0.0).sqrt() <= 0.3 && alone(*g, &c) {
            pieces.push((*g, c));
        }
    }
    pieces.sort_by_key(|p| p.0);
    let mut out = vec![false; set.len()];
    if pieces.len() < 3 {
        return out;
    }
    let centres_of: Vec<[f64; 3]> = pieces.iter().map(|p| p.1).collect();
    let mut dsu = crate::cluster::in_a_row(
        &centres_of,
        2.0 * link,
        w.link_gap,
        w.max_slope,
        crate::cluster::STRAIGHT,
    );
    let mut rows: HashMap<usize, (usize, [f64; 6])> = HashMap::new();
    for (j, p) in pieces.iter().enumerate() {
        let r = dsu.find(j);
        let (n, b) = rows.entry(r).or_insert((
            0,
            [f64::MAX, f64::MAX, f64::MAX, f64::MIN, f64::MIN, f64::MIN],
        ));
        *n += 1;
        for k in 0..3 {
            b[k] = b[k].min(p.1[k]);
            b[k + 3] = b[k + 3].max(p.1[k]);
        }
    }
    let wire_group: std::collections::HashSet<usize> = pieces
        .iter()
        .enumerate()
        .filter(|(j, _)| {
            let (n, b) = rows[&dsu.find(*j)];
            let span =
                ((b[3] - b[0]).powi(2) + (b[4] - b[1]).powi(2) + (b[5] - b[2]).powi(2)).sqrt();
            n >= 3 && span >= 2.0 * w.min_length
        })
        .map(|(_, p)| p.0)
        .collect();
    for (o, &v) in out.iter_mut().zip(&in_voxel) {
        *o = wire_group.contains(&group[v]);
    }
    out
}

/// Da las clases adicionales a los puntos de una tesela. `class` guarda la clase que decidió cada
/// paso (`None`: la propia del punto); `ground` dice qué puntos son suelo.
pub fn apply(pts: &[Point], ground: &[bool], class: &mut [Option<u8>], e: &Extras) {
    if let Some(w) = &e.water {
        water(pts, ground, class, w);
    }
    if let Some(r) = &e.roads {
        roads(pts, ground, class, r);
    }
    if e.vegetation.is_none() && e.wires.is_none() && e.poles.is_none() {
        return;
    }
    let now = |class: &[Option<u8>], k: usize| class[k].unwrap_or(pts[k].class);
    let surface_cell = e.vegetation.as_ref().map_or(1.0, |v| v.cell.max(0.25));
    let needs_heights = e.any();
    let surface = if needs_heights {
        ground_surface(pts, ground, surface_cell)
    } else {
        None
    };
    let Some(surface) = surface else { return };
    let height = |k: usize| -> f64 {
        surface
            .sample(pts[k].x, pts[k].y)
            .map_or(0.0, |g| pts[k].z - f64::from(g))
    };
    let above: Vec<usize> = (0..pts.len())
        .filter(|&k| !ground[k] && !pts[k].is_noise() && now(class, k) != 2 && now(class, k) != 6)
        .collect();
    if let Some(v) = &e.vegetation {
        for &k in &above {
            let h = height(k);
            class[k] = Some(if h < v.low {
                LOW_VEGETATION
            } else if h < v.medium {
                MEDIUM_VEGETATION
            } else {
                HIGH_VEGETATION
            });
        }
    }
    if let Some(w) = &e.wires {
        let set: Vec<usize> = above
            .iter()
            .copied()
            .filter(|&k| height(k) >= w.min_height)
            .collect();
        // los puntos de un conductor vienen en grupos con huecos entre ellos (las líneas de barrido
        // de un vuelo): en un entorno de puntos un grupo es una mancha, no una línea. Los puntos se
        // reducen a los centros de los vóxeles que llenan, de modo que la forma que se ve es la de
        // la estructura, sea cual sea la densidad
        let (radius, _) = wire_scale(pts, w);
        let vs = w.voxel.max(0.05);
        let mut voxel_of: HashMap<(i32, i32, i32), usize> = HashMap::new();
        let mut centres: Vec<Point> = Vec::new();
        let in_voxel: Vec<usize> = set
            .iter()
            .map(|&k| {
                let q = &pts[k];
                let key = (
                    (q.x / vs).floor() as i32,
                    (q.y / vs).floor() as i32,
                    (q.z / vs).floor() as i32,
                );
                *voxel_of.entry(key).or_insert_with(|| {
                    centres.push(Point {
                        x: (f64::from(key.0) + 0.5) * vs,
                        y: (f64::from(key.1) + 0.5) * vs,
                        z: (f64::from(key.2) + 0.5) * vs,
                        ..Point::default()
                    });
                    centres.len() - 1
                })
            })
            .collect();
        let all: Vec<usize> = (0..centres.len()).collect();
        // un vóxel es candidato si parece un cable a algún radio
        let mut is_cand = vec![false; centres.len()];
        let mut reach = radius;
        for r in wire_radii(pts, w) {
            reach = reach.max(r);
            let width_limit = if w.max_width > 0.0 { w.max_width } else { 0.35 * r };
            let crowd = neighbour_counts(&centres, &all, 2.0 * r);
            for ((&v, f), n) in all.iter().zip(line_features(&centres, &all, r)).zip(crowd) {
                if let Some((lin, dz, width)) = f
                    && lin >= w.linearity
                    && dz <= w.max_slope
                    && width <= width_limit
                    && n <= w.max_neighbours
                {
                    is_cand[v] = true;
                }
            }
        }
        let cand: Vec<usize> = all.iter().copied().filter(|&v| is_cand[v]).collect();
        let keep = straight_chains(
            &centres,
            &cand,
            reach,
            w.min_length,
            w.min_straightness,
        );
        let mut wire = vec![false; centres.len()];
        for (&v, k) in cand.iter().zip(keep) {
            wire[v] = k;
        }
        for (&k, &v) in set.iter().zip(&in_voxel) {
            if wire[v] {
                class[k] = Some(WIRE);
            }
        }
        if w.link_gap > 0.0 {
            let rest: Vec<usize> = set
                .iter()
                .copied()
                .filter(|&k| class[k] != Some(WIRE) && height(k) >= LINKED_MIN_HEIGHT)
                .collect();
            for (&k, hit) in rest.iter().zip(linked_wires(pts, &rest, w, radius)) {
                if hit {
                    class[k] = Some(WIRE);
                }
            }
        }
    }
    if let Some(p) = &e.poles {
        let set: Vec<usize> = above
            .iter()
            .copied()
            .filter(|&k| height(k) >= p.min_height && now(class, k) != WIRE)
            .collect();
        // lo que hay alrededor de una estructura: puntos a más de 3 m sobre el suelo, por celda de
        // 3 m
        let mut under: HashMap<(i32, i32), Vec<[f64; 4]>> = HashMap::new();
        for &k in &above {
            // los conductores llegan a la torre: no son copa a su alrededor
            if height(k) >= 3.0 && class[k] != Some(WIRE) {
                under
                    .entry((
                        (pts[k].x / 3.0).floor() as i32,
                        (pts[k].y / 3.0).floor() as i32,
                    ))
                    .or_default()
                    .push([pts[k].x, pts[k].y, pts[k].z, height(k)]);
            }
        }
        // cuántos puntos puede tener una estructura y cuántos sueltos tolera una torre crecen con
        // la densidad del vuelo: 12 por metro de altura es el recuento de una torre a 9 puntos/m²
        let density = spacing(pts).map_or(1.0, |s| (POLE_REFERENCE_SPACING / s).powi(2).max(1.0));
        let (group, boxes) = chains_z(pts, &set, p.radius, POLE_Z_SCALE);
        let mut members: HashMap<usize, Vec<f64>> = HashMap::new();
        for (&k, g) in set.iter().zip(&group) {
            if height(k) >= 3.0 {
                members.entry(*g).or_default().push(height(k));
            }
        }
        // puntos muy por encima del suelo a menos de 3 m de la cadena en planta, los de la cadena
        // aparte: una torre en un corredor no tiene ninguno, un árbol en el borde de un rodal sí.
        // «Alto» es 3 m o un tercio de la altura de la estructura, lo que sea más: un cultivo de
        // árboles bajos al pie de una torre no es copa a su alrededor
        let crowd = |b: &[f64; 6], own: &[f64], tolerance: f64| -> bool {
            let high = (0.3 * (b[5] - b[2])).max(3.0);
            let (cx0, cx1) = (
                (b[0] / 3.0).floor() as i32 - 1,
                (b[3] / 3.0).floor() as i32 + 1,
            );
            let (cy0, cy1) = (
                (b[1] / 3.0).floor() as i32 - 1,
                (b[4] / 3.0).floor() as i32 + 1,
            );
            let near: usize = (cx0..=cx1)
                .flat_map(|cx| (cy0..=cy1).map(move |cy| (cx, cy)))
                .filter_map(|c| under.get(&c))
                .flatten()
                .filter(|q| {
                    q[3] >= high
                        && q[0] > b[0] - 3.0
                        && q[0] < b[3] + 3.0
                        && q[1] > b[1] - 3.0
                        && q[1] < b[4] + 3.0
                })
                .count();
            let own = own.iter().filter(|h| **h >= high).count() as f64;
            near as f64 > own * tolerance + 2.0 * density
        };
        // un tronco sube hasta su copa: alrededor de su punta, desde 2 m por debajo, hay muchos más
        // puntos que los suyos. Una farola, un poste, tiene una lámpara y poco más allí arriba.
        let capped = |b: &[f64; 6], own: usize| -> bool {
            let (cx0, cx1) = (
                ((b[0] - 2.0) / 3.0).floor() as i32,
                ((b[3] + 2.0) / 3.0).floor() as i32,
            );
            let (cy0, cy1) = (
                ((b[1] - 2.0) / 3.0).floor() as i32,
                ((b[4] + 2.0) / 3.0).floor() as i32,
            );
            let near: usize = (cx0..=cx1)
                .flat_map(|cx| (cy0..=cy1).map(move |cy| (cx, cy)))
                .filter_map(|c| under.get(&c))
                .flatten()
                .filter(|q| {
                    q[2] >= b[5] - 2.0
                        && q[0] > b[0] - 2.0
                        && q[0] < b[3] + 2.0
                        && q[1] > b[1] - 2.0
                        && q[1] < b[4] + 2.0
                })
                .count();
            near as f64 > own as f64 * CAP_TOLERANCE + 2.0 * density
        };
        let crowded = |g: &usize| -> bool {
            let (b, _) = boxes[g];
            crowd(&b, members.get(g).map_or(&[][..], |v| v.as_slice()), CROWD_TOLERANCE)
        };
        // una estructura se apoya en el suelo, es estrecha para su altura y es dispersa; una copa
        // es ancha y densa, un bosque es una cadena mucho más ancha que cualquier torre; la planta
        // de cada cadena, para medir su anchura sin lo que tiene unido: los conductores que llegan
        // a una torre, o una rama que toca un poste, estiran su caja
        let mut plan: HashMap<usize, (Vec<f64>, Vec<f64>)> = HashMap::new();
        for (&k, g) in set.iter().zip(&group) {
            let e = plan.entry(*g).or_default();
            e.0.push(pts[k].x);
            e.1.push(pts[k].y);
        }
        let core_width = |g: &usize| -> f64 {
            plan.get(g).map_or(0.0, |(x, y)| {
                crate::cluster::core_spread(x).max(crate::cluster::core_spread(y))
            })
        };
        // los puntos de una cadena dentro de su núcleo: los de una torre, sin los conductores
        // unidos
        let core_points = |g: &usize, width: f64| -> usize {
            let Some((x, y)) = plan.get(g) else { return 0 };
            let mid = |v: &[f64]| {
                let mut v = v.to_vec();
                v.sort_by(f64::total_cmp);
                v[v.len() / 2]
            };
            let (mx, my) = (mid(x), mid(y));
            let half = width / 2.0 + 0.5;
            x.iter()
                .zip(y)
                .filter(|(a, b)| (**a - mx).abs() <= half && (**b - my).abs() <= half)
                .count()
        };
        let is_pole = |g: &usize| -> bool {
            let (b, _) = boxes[g];
            let width = core_width(g);
            let n = core_points(g, width);
            let extent = b[5] - b[2];
            n >= 5
                && width <= p.max_width
                && extent >= p.min_extent
                && width <= 0.35 * extent
                && (n as f64)
                    <= if extent >= TOWER_HEIGHT {
                        TOWER_POINTS_PER_METRE * extent * density
                    } else {
                        12.0 * extent
                    }
        };
        let low = |g: &usize| -> bool {
            // llega al suelo: algún punto de la cadena está a menos de 3 m de él
            let (b, _) = boxes[g];
            let (x, y) = ((b[0] + b[3]) / 2.0, (b[1] + b[4]) / 2.0);
            surface
                .sample(x, y)
                .is_some_and(|gz| b[2] - f64::from(gz) <= p.min_height + 3.0)
        };
        let poles: std::collections::HashSet<usize> = boxes
            .keys()
            .filter(|g| is_pole(g) && low(g) && !crowded(g))
            .copied()
            .collect();
        for (&k, g) in set.iter().zip(&group) {
            if poles.contains(g) {
                class[k] = Some(POLE);
            }
        }
        if p.head_length > 0.0 {
            let high: Vec<usize> = above
                .iter()
                .copied()
                .filter(|&k| height(k) >= HEAD_MIN_HEIGHT && class[k] != Some(POLE))
                .collect();
            let (group, boxes) = chains(pts, &high, p.radius);
            // una cruceta es una barra recta: la dispersión de sus puntos en planta es larga en un
            // sentido y fina en el otro (un trozo de copa es una mancha)
            let mut plan: HashMap<usize, Vec<[f64; 2]>> = HashMap::new();
            for (&k, g) in high.iter().zip(&group) {
                plan.entry(*g).or_default().push([pts[k].x, pts[k].y]);
            }
            let straight = |g: &usize| -> bool {
                let q = &plan[g];
                let n = q.len() as f64;
                let (mx, my) = (
                    q.iter().map(|v| v[0]).sum::<f64>() / n,
                    q.iter().map(|v| v[1]).sum::<f64>() / n,
                );
                let (mut sxx, mut syy, mut sxy) = (0.0, 0.0, 0.0);
                for v in q {
                    sxx += (v[0] - mx).powi(2);
                    syy += (v[1] - my).powi(2);
                    sxy += (v[0] - mx) * (v[1] - my);
                }
                let (sxx, syy, sxy) = (sxx / n, syy / n, sxy / n);
                let mid = (sxx + syy) / 2.0;
                let d = ((sxx - syy) / 2.0).hypot(sxy);
                let (l1, l2) = (mid + d, (mid - d).max(0.0));
                l2.sqrt() <= 0.6 && l1.sqrt() >= 3.0 * l2.sqrt()
            };
            let is_head = |g: &usize| -> bool {
                let (b, n) = boxes[g];
                if !straight(g) {
                    return false;
                }
                let diag =
                    ((b[3] - b[0]).powi(2) + (b[4] - b[1]).powi(2) + (b[5] - b[2]).powi(2)).sqrt();
                if n < 8 || diag < 3.0 || diag > p.head_length {
                    return false;
                }
                // nada a menos de 3 m de la cadena en planta y por debajo, hasta el suelo
                let (cx0, cx1) = (
                    (b[0] / 3.0).floor() as i32 - 1,
                    (b[3] / 3.0).floor() as i32 + 1,
                );
                let (cy0, cy1) = (
                    (b[1] / 3.0).floor() as i32 - 1,
                    (b[4] / 3.0).floor() as i32 + 1,
                );
                !(cx0..=cx1).any(|cx| {
                    (cy0..=cy1).any(|cy| {
                        under.get(&(cx, cy)).is_some_and(|ps| {
                            ps.iter().any(|q| {
                                q[2] < b[2] - 1.0
                                    && q[0] > b[0] - 3.0
                                    && q[0] < b[3] + 3.0
                                    && q[1] > b[1] - 3.0
                                    && q[1] < b[4] + 3.0
                            })
                        })
                    })
                })
            };
            let heads: std::collections::HashSet<usize> =
                boxes.keys().filter(|g| is_head(g)).copied().collect();
            for (&k, g) in high.iter().zip(&group) {
                if heads.contains(g) {
                    class[k] = Some(POLE);
                }
            }
        }
        if p.thin_extent > 0.0 {
            // un poste fino es una línea vertical: el mismo aspecto que un cable, puesto de pie
            let set: Vec<usize> = above
                .iter()
                .copied()
                .filter(|&k| height(k) >= 1.0 && class[k].is_none_or(|c| c != POLE && c != WIRE))
                .collect();
            let w = WireParams::default();
            let (radius, max_width) = wire_scale(pts, &w);
            let vs = w.voxel.max(0.05);
            let mut voxel_of: HashMap<(i32, i32, i32), usize> = HashMap::new();
            let mut centres: Vec<Point> = Vec::new();
            let in_voxel: Vec<usize> = set
                .iter()
                .map(|&k| {
                    let q = &pts[k];
                    let key = (
                        (q.x / vs).floor() as i32,
                        (q.y / vs).floor() as i32,
                        (q.z / vs).floor() as i32,
                    );
                    *voxel_of.entry(key).or_insert_with(|| {
                        centres.push(Point {
                            x: (f64::from(key.0) + 0.5) * vs,
                            y: (f64::from(key.1) + 0.5) * vs,
                            z: (f64::from(key.2) + 0.5) * vs,
                            ..Point::default()
                        });
                        centres.len() - 1
                    })
                })
                .collect();
            let all: Vec<usize> = (0..centres.len()).collect();
            let cand: Vec<usize> = all
                .iter()
                .zip(line_features(&centres, &all, radius))
                .filter_map(|(&v, f)| {
                    f.filter(|&(lin, dz, width)| lin >= 0.7 && dz >= 0.8 && width <= max_width)
                        .map(|_| v)
                })
                .collect();
            // los troncos se ven como líneas verticales solo cuando el vuelo es denso (por encima
            // de unos 180 puntos/m²)
            let trunks_resolved = density >= TRUNK_DENSITY;
            let (group, boxes) = chains(&centres, &cand, radius);
            // las alturas de los puntos de cada cadena, para distinguir un poste de un tronco: un
            // tronco tiene su copa alrededor
            let group_of: HashMap<usize, usize> =
                cand.iter().copied().zip(group.iter().copied()).collect();
            let mut own: HashMap<usize, Vec<f64>> = HashMap::new();
            for (&k, v) in set.iter().zip(&in_voxel) {
                if let Some(g) = group_of.get(v) {
                    own.entry(*g).or_default().push(height(k));
                }
            }
            let good: std::collections::HashSet<usize> = boxes
                .iter()
                .filter(|(g, (b, n))| {
                    let plan = (b[3] - b[0]).hypot(b[4] - b[1]);
                    *n >= 4
                        && b[5] - b[2] >= p.thin_extent
                        && plan <= 0.5 * (b[5] - b[2]).max(1.0)
                        && !(trunks_resolved && capped(b, own.get(g).map_or(0, Vec::len)))
                })
                .map(|(g, _)| *g)
                .collect();
            let mut keep = vec![false; centres.len()];
            for (&v, g) in cand.iter().zip(&group) {
                keep[v] = good.contains(g);
            }
            for (&k, &v) in set.iter().zip(&in_voxel) {
                if keep[v] {
                    class[k] = Some(POLE);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pt(x: f64, y: f64, z: f64) -> Point {
        Point {
            x,
            y,
            z,
            class: 1,
            return_number: 1,
            number_of_returns: 1,
            ..Point::default()
        }
    }

    #[test]
    fn strata_wire_and_pole() {
        let mut pts = Vec::new();
        // suelo llano
        for i in 0..30 {
            for j in 0..30 {
                pts.push(pt(f64::from(i), f64::from(j), 0.0));
            }
        }
        let ground = vec![true; pts.len()];
        let n_ground = pts.len();
        // vegetación a 0,2, 1,0 y 6,0 m
        for (k, z) in [0.2, 1.0, 6.0].into_iter().enumerate() {
            pts.push(pt(5.0 + k as f64 * 0.3, 5.0, z));
        }
        // un cable a 8 m a lo largo de x, un poste a lo largo de z en x = 20
        for i in 0..40 {
            pts.push(pt(2.0 + f64::from(i) * 0.5, 25.0, 8.0));
        }
        for i in 0..30 {
            pts.push(pt(20.0, 10.0, 0.5 + f64::from(i) * 0.3));
        }
        // una torre de celosía lejos del resto: una columna suelta de 2 m de ancho y 18 m de alto
        let tower_from = pts.len();
        for i in 0..24 {
            let z = 1.0 + f64::from(i) * 0.75;
            let w = 1.0 - z / 40.0;
            pts.push(pt(10.0 - w, 3.0, z));
            pts.push(pt(10.0 + w, 3.0 + w, z + 0.3));
        }
        let tower_to = pts.len();
        // una copa sola, ancha para su altura, no debe contar
        let crown_from = pts.len();
        for i in 0..8 {
            for j in 0..8 {
                pts.push(pt(
                    24.0 + f64::from(i) * 0.4,
                    6.0 + f64::from(j) * 0.4,
                    5.0 + f64::from(i % 3),
                ));
            }
        }
        // la cabeza de una torre lejos de todo: una cruceta de 6 m a 12 m de altura sin nada debajo
        let head_from = pts.len();
        for i in 0..24 {
            pts.push(pt(27.0, 12.0 + f64::from(i) * 0.25, 12.0));
        }
        let head_to = pts.len();
        let mut ground_all = ground;
        ground_all.resize(pts.len(), false);
        let mut class = vec![None; pts.len()];
        let e = Extras {
            vegetation: Some(VegParams::default()),
            wires: Some(WireParams::default()),
            poles: Some(PoleParams::default()),
            ..Extras::default()
        };
        apply(&pts, &ground_all, &mut class, &e);
        assert_eq!(class[n_ground], Some(LOW_VEGETATION));
        assert_eq!(class[n_ground + 1], Some(MEDIUM_VEGETATION));
        assert_eq!(class[n_ground + 2], Some(HIGH_VEGETATION));
        let wires = class.iter().filter(|c| **c == Some(WIRE)).count();
        let poles = class.iter().filter(|c| **c == Some(POLE)).count();
        assert!(wires >= 35, "{wires}");
        assert!(poles >= 25, "{poles}");
        let tower = class[tower_from..tower_to]
            .iter()
            .filter(|c| **c == Some(POLE))
            .count();
        assert!(tower >= 40, "{tower}");
        assert!(
            class[crown_from..head_from]
                .iter()
                .all(|c| *c != Some(POLE))
        );
        assert!(
            class[head_from..head_to].iter().all(|c| *c == Some(POLE)),
            "{:?}",
            &class[head_from..head_to]
        );
    }

    fn flat_ground(w: i32, h: i32, step: f64) -> Vec<Point> {
        let mut pts = Vec::new();
        for i in 0..(f64::from(w) / step) as i32 {
            for j in 0..(f64::from(h) / step) as i32 {
                pts.push(pt(f64::from(i) * step, f64::from(j) * step, 0.0));
            }
        }
        pts
    }

    /// Un vuelo con pocas pasadas ve un conductor en trozos de un metro con metros de nada entre
    /// ellos: tres o más en fila, lejos de todo lo demás, son un cable; dos no lo son, y los trozos
    /// con copa a su alrededor tampoco. Un vuelo disperso mira un cable con tres radios, uno denso
    /// con uno.
    #[test]
    fn wires_are_looked_at_from_more_radii_in_a_sparse_survey() {
        let w = WireParams::default();
        let dense = flat_ground(60, 60, 0.1);
        let sparse = flat_ground(60, 60, 0.4);
        assert_eq!(wire_radii(&dense, &w).len(), 1);
        let r = wire_radii(&sparse, &w);
        assert_eq!(r.len(), 3);
        assert!(r[0] < r[1] && r[1] < r[2] && r[2] <= 4.5, "{r:?}");
        let fixed = WireParams {
            radius: 2.0,
            ..w
        };
        assert_eq!(wire_radii(&sparse, &fixed), vec![2.0]);
    }

    #[test]
    fn pieces_of_conductor_in_a_row_are_wire() {
        let mut pts = flat_ground(120, 40, 0.5);
        let n_ground = pts.len();
        let piece = |pts: &mut Vec<Point>, x: f64, y: f64, z: f64| {
            for i in 0..20 {
                pts.push(pt(x + f64::from(i) * 0.05, y, z));
            }
        };
        // seis trozos a 10 m uno de otro a 14 m, y otros dos aparte de ellos
        let wire_from = pts.len();
        for k in 0..6 {
            piece(&mut pts, 5.0 + 10.0 * f64::from(k), 25.0, 14.0);
        }
        let wire_to = pts.len();
        piece(&mut pts, 5.0, 5.0, 14.0);
        piece(&mut pts, 15.0, 5.0, 14.0);
        let pair_to = pts.len();
        // tres trozos en fila, cada uno en medio de una mancha de puntos de copa
        for k in 0..3 {
            let x = 70.0 + 10.0 * f64::from(k);
            piece(&mut pts, x, 25.0, 14.0);
            for a in 0..6 {
                for b in 0..6 {
                    for c in 0..6 {
                        pts.push(pt(
                            x - 0.6 + f64::from(a) * 0.25,
                            24.0 + f64::from(b) * 0.25,
                            13.0 + f64::from(c) * 0.25,
                        ));
                    }
                }
            }
        }
        let mut ground = vec![false; pts.len()];
        ground[..n_ground].fill(true);
        let mut class = vec![None; pts.len()];
        let e = Extras {
            wires: Some(WireParams::default()),
            ..Extras::default()
        };
        apply(&pts, &ground, &mut class, &e);
        let wires = |a: usize, b: usize| class[a..b].iter().filter(|c| **c == Some(WIRE)).count();
        assert_eq!(wires(wire_from, wire_to), wire_to - wire_from);
        assert_eq!(wires(wire_to, pair_to), 0, "two pieces are not a line");
        assert_eq!(wires(pair_to, pts.len()), 0, "pieces inside crown are not wire");
    }

    /// A la densidad de un vuelo, una torre de celosía tiene miles de puntos, una columna suelta
    /// con metros entre sus filas; los cultivos bajos a su pie no son copa a su alrededor.
    #[test]
    fn a_dense_survey_tower_among_low_crops_is_a_pole() {
        let mut pts = flat_ground(40, 40, 0.1);
        let n_ground = pts.len();
        let tower_from = pts.len();
        for k in 0..13 {
            let z = 3.0 + 3.0 * f64::from(k);
            for i in 0..50 {
                let d = -1.5 + f64::from(i) * 0.06;
                pts.push(pt(20.0 + d, 20.0, z));
                pts.push(pt(20.0, 20.0 + d, z));
            }
        }
        let tower_to = pts.len();
        // cultivos de 3,5 m de alto en un anillo a 4 a 6 m de la torre
        for i in 0..400 {
            let a = f64::from(i) * 0.37;
            let r = 4.0 + f64::from(i % 20) * 0.1;
            pts.push(pt(20.0 + r * a.cos(), 20.0 + r * a.sin(), 3.5));
        }
        let mut ground = vec![false; pts.len()];
        ground[..n_ground].fill(true);
        let mut class = vec![None; pts.len()];
        let e = Extras {
            poles: Some(PoleParams::default()),
            ..Extras::default()
        };
        apply(&pts, &ground, &mut class, &e);
        let poles = class[tower_from..tower_to]
            .iter()
            .filter(|c| **c == Some(POLE))
            .count();
        assert!(poles >= (tower_to - tower_from) * 9 / 10, "{poles}");
        assert!(class[tower_to..].iter().all(|c| *c != Some(POLE)));
    }

    /// Una franja plana y oscura de suelo es una carretera; el suelo brillante a su lado y el suelo
    /// rugoso no lo son.
    #[test]
    fn dark_smooth_ground_is_road() {
        let mut pts = Vec::new();
        let mut rough_seed = 7u64;
        for i in 0..40 {
            for j in 0..40 {
                let mut q = pt(f64::from(i) * 0.5, f64::from(j) * 0.5, 0.0);
                if i < 20 {
                    // road: dark, flat
                    q.intensity = 5000;
                } else if j < 20 {
                    // bright soil, flat
                    q.intensity = 30000;
                } else {
                    // suelo oscuro pero rugoso
                    q.intensity = 5000;
                    rough_seed = rough_seed
                        .wrapping_mul(6364136223846793005)
                        .wrapping_add(1442695040888963407);
                    q.z = ((rough_seed >> 40) as f64 / f64::from(1u32 << 24) - 0.5) * 0.4;
                }
                pts.push(q);
            }
        }
        let ground = vec![true; pts.len()];
        let mut class = vec![None; pts.len()];
        let e = Extras {
            roads: Some(RoadParams {
                max_intensity: 10_000.0,
                ..RoadParams::default()
            }),
            ..Extras::default()
        };
        apply(&pts, &ground, &mut class, &e);
        let road = |i: usize, j: usize| class[i * 40 + j] == Some(ROAD);
        assert!(road(5, 5) && road(10, 30), "the dark flat strip is road");
        assert!(!road(30, 5), "bright soil is not");
        assert!(!road(30, 30), "rough ground is not");
    }

    /// Sin un límite fijado, oscuro es relativo al suelo de la tesela: la misma escena se encuentra
    /// con intensidades de 16 bits y con intensidades de 0 a 255.
    #[test]
    fn the_road_limit_follows_the_scale_of_the_sensor() {
        for scale in [1.0, 65535.0 / 255.0] {
            let mut pts = Vec::new();
            for i in 0..40 {
                for j in 0..40 {
                    let mut q = pt(f64::from(i) * 0.5, f64::from(j) * 0.5, 0.0);
                    // una franja oscura de una quinta parte del suelo, tierra brillante en el resto
                    q.intensity = if i < 8 { 20.0 * scale } else { 120.0 * scale } as u16;
                    pts.push(q);
                }
            }
            let ground = vec![true; pts.len()];
            let mut class = vec![None; pts.len()];
            let e = Extras {
                roads: Some(RoadParams::default()),
                ..Extras::default()
            };
            apply(&pts, &ground, &mut class, &e);
            assert!(class[4 * 40 + 20] == Some(ROAD), "scale {scale}");
            assert!(class[30 * 40 + 20] != Some(ROAD), "scale {scale}");
        }
    }

    /// Una lámina grande y nivelada de retornos únicos oscuros es agua; una pequeña no lo es.
    #[test]
    fn a_dark_level_sheet_is_water() {
        let mut pts = Vec::new();
        for i in 0..60 {
            for j in 0..60 {
                let mut q = pt(f64::from(i) * 0.5, f64::from(j) * 0.5, 10.0);
                q.intensity = 800;
                pts.push(q);
            }
        }
        // una pequeña mancha oscura lejos
        let lake = pts.len();
        for i in 0..6 {
            for j in 0..6 {
                let mut q = pt(200.0 + f64::from(i) * 0.5, f64::from(j) * 0.5, 10.0);
                q.intensity = 800;
                pts.push(q);
            }
        }
        let ground = vec![true; pts.len()];
        let mut class = vec![None; pts.len()];
        let e = Extras {
            water: Some(WaterParams {
                max_intensity: 6_000.0,
                ..WaterParams::default()
            }),
            ..Extras::default()
        };
        apply(&pts, &ground, &mut class, &e);
        assert_eq!(class[30 * 60 + 30], Some(WATER));
        assert_eq!(class[lake + 10], None);
    }
}
