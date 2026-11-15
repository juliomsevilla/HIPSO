//! Métricas de rejilla, con la disposición de tabla del `*_all_returns_elevation_stats.csv`
//! compatible con FUSION: estadísticos descriptivos de las alturas de los puntos en cada celda.
//!
//! La memoria se mantiene por debajo de un presupuesto sea cual sea el tamaño de la nube: una
//! primera pasada cuenta los puntos de cada celda, luego las filas se procesan en bandas que caben
//! en el presupuesto, leyendo cada banda la nube una vez y colocando sus alturas directamente en
//! porciones por celda (una ordenación por conteo).

use std::io::Write;

use rayon::prelude::*;

use crate::grid::Grid;
use crate::las::Reader;
use crate::{Error, Result};

/// Las columnas de salida, en el orden de esa disposición; `{hb}` es la altura de corte.
const COLUMNS: [&str; 71] = [
    "row",
    "col",
    "center X",
    "center Y",
    "Total return count",
    "Elev minimum",
    "Elev maximum",
    "Elev mean",
    "Elev mode",
    "Elev stddev",
    "Elev variance",
    "Elev CV",
    "Elev IQ",
    "Elev skewness",
    "Elev kurtosis",
    "Elev AAD",
    "Elev L1",
    "Elev L2",
    "Elev L3",
    "Elev L4",
    "Elev L CV",
    "Elev L skewness",
    "Elev L kurtosis",
    "Elev P01",
    "Elev P05",
    "Elev P10",
    "Elev P20",
    "Elev P25",
    "Elev P30",
    "Elev P40",
    "Elev P50",
    "Elev P60",
    "Elev P70",
    "Elev P75",
    "Elev P80",
    "Elev P90",
    "Elev P95",
    "Elev P99",
    "Return 1 count",
    "Return 2 count",
    "Return 3 count",
    "Return 4 count",
    "Return 5 count",
    "Return 6 count",
    "Return 7 count",
    "Return 8 count",
    "Return 9 count",
    "Other return count",
    "Percentage first returns above {hb}",
    "Percentage all returns above {hb}",
    "(All returns above {hb}) / (Total first returns) * 100",
    "First returns above {hb}",
    "All returns above {hb}",
    "Percentage first returns above mean",
    "Percentage first returns above mode",
    "Percentage all returns above mean",
    "Percentage all returns above mode",
    "(All returns above mean) / (Total first returns) * 100",
    "(All returns above mode) / (Total first returns) * 100",
    "First returns above mean",
    "First returns above mode",
    "All returns above mean",
    "All returns above mode",
    "Total first returns",
    "Total all returns",
    "Elev MAD median",
    "Elev MAD mode",
    "Canopy relief ratio",
    "Elev quadratic mean",
    "Elev cubic mean",
    "Profile area",
];

const PERCENTILES: [f64; 15] = [
    0.01, 0.05, 0.10, 0.20, 0.25, 0.30, 0.40, 0.50, 0.60, 0.70, 0.75, 0.80, 0.90, 0.95, 0.99,
];

/// Bytes retenidos por punto mientras se procesa su banda: su altura, su número de retorno y su
/// intensidad cuando se piden métricas de intensidad o de estratos.
const BYTES_PER_POINT: u64 = 5;
const BYTES_PER_POINT_INT: u64 = 7;

/// Adónde van las métricas: las de elevación siempre, las demás cuando se piden.
pub struct Outputs<'a> {
    pub elevation: &'a mut dyn Write,
    pub intensity: Option<&'a mut dyn Write>,
    pub strata: Option<&'a mut dyn Write>,
    /// Las métricas de estructura ([`StructureParams`]), cuando se piden.
    pub structure: Option<&'a mut dyn Write>,
}

/// Métricas de estructura de una celda, a partir de las alturas de sus retornos sobre el suelo:
/// cuánta luz pasa, cómo se reparten los retornos en altura y dónde empieza la copa.
#[derive(Clone, Debug, PartialEq)]
pub struct StructureParams {
    /// Grosor de las capas del perfil vertical, en las unidades de altura.
    pub layer: f64,
    /// Altura del perfil: los retornos por encima caen en su última capa.
    pub top: f64,
    /// Los retornos por debajo de esta altura cuentan como luz que llegó al suelo.
    pub gap_height: f64,
    /// Coeficiente de extinción del índice de área foliar (0,5 para una distribución esférica de
    /// las hojas).
    pub k: f64,
    /// La base de la copa se busca por encima de esta altura (los arbustos y los combustibles de
    /// superficie quedan por debajo).
    pub cbh_floor: f64,
    /// Una capa pertenece a la copa cuando contiene al menos esta proporción de la capa más llena.
    pub cbh_share: f64,
}

impl Default for StructureParams {
    fn default() -> Self {
        Self {
            layer: 1.0,
            top: 30.0,
            gap_height: 1.0,
            k: 0.5,
            cbh_floor: 2.0,
            cbh_share: 0.1,
        }
    }
}

impl StructureParams {
    fn layers(&self) -> usize {
        (self.top / self.layer).ceil().max(1.0) as usize
    }

    fn check(&self) -> Result<()> {
        if !(self.layer > 0.0) || !(self.top > self.layer) || self.layers() > MAX_LAYERS {
            return Err(Error::Param(format!(
                "el perfil vertical necesita capas positivas y entre 2 y {MAX_LAYERS} capas (altura / grosor)"
            )));
        }
        if !(self.k > 0.0) || !(self.cbh_share > 0.0 && self.cbh_share <= 1.0) {
            return Err(Error::Param(
                "el coeficiente de extinción debe ser positivo y la proporción de la base de copa estar entre 0 y 1".into(),
            ));
        }
        Ok(())
    }
}

/// Máximo de capas del perfil vertical.
const MAX_LAYERS: usize = 256;

/// La fracción de huecos, el índice de área foliar, la diversidad de la altura del follaje, el
/// índice de complejidad vertical y la altura de la base de la copa de una celda, a partir de las
/// alturas sobre el suelo de sus retornos (NaN donde no se pueden saber).
///
/// - Fracción de huecos: la proporción de retornos por debajo de `gap_height`.
/// - LAI: `-ln(fracción de huecos) / k` (Beer–Lambert), con la fracción mantenida en un retorno
///   para que una celda sin luz que pase no dé infinito.
/// - FHD: la entropía de Shannon de la proporción de retornos en cada capa (MacArthur y MacArthur,
///   1961).
/// - VCI: esa entropía entre su máximo, `ln(capas)` (van Ewijk et al., 2011).
/// - CBH: el borde inferior de la primera capa por encima de `cbh_floor` con al menos `cbh_share`
///   de los retornos de la capa más llena por encima de ella.
fn structure_metrics(h: &[f32], sp: &StructureParams) -> [f64; 5] {
    let n = h.len();
    if n == 0 {
        return [f64::NAN; 5];
    }
    let layers = sp.layers();
    let mut counts = [0u32; MAX_LAYERS];
    let mut ground = 0usize;
    for &x in h {
        let x = f64::from(x);
        ground += usize::from(x < sp.gap_height);
        let l = ((x.max(0.0) / sp.layer) as usize).min(layers - 1);
        counts[l] += 1;
    }
    let nf = n as f64;
    let gap = ground as f64 / nf;
    let lai = -(gap.max(1.0 / nf)).ln() / sp.k;
    let entropy: f64 = counts[..layers]
        .iter()
        .filter(|&&c| c > 0)
        .map(|&c| {
            let p = f64::from(c) / nf;
            -p * p.ln()
        })
        .sum();
    let vci = if layers > 1 {
        entropy / (layers as f64).ln()
    } else {
        f64::NAN
    };
    let first = (sp.cbh_floor / sp.layer).ceil().max(0.0) as usize;
    let fullest = counts[first.min(layers)..layers]
        .iter()
        .copied()
        .max()
        .unwrap_or(0);
    let cbh = if fullest == 0 {
        f64::NAN
    } else {
        let need = (sp.cbh_share * f64::from(fullest)).max(1.0);
        (first..layers)
            .find(|&l| f64::from(counts[l]) >= need)
            .map_or(f64::NAN, |l| l as f64 * sp.layer)
    };
    // `+ 0.0` convierte un cero negativo (una celda sin retornos por encima del suelo) en un cero
    // normal
    [gap + 0.0, lai + 0.0, entropy + 0.0, vci + 0.0, cbh]
}

const STRUCTURE_COLUMNS: [&str; 6] = [
    "Points",
    "Gap fraction",
    "Leaf area index",
    "Foliage height diversity",
    "Vertical complexity index",
    "Crown base height",
];

/// El texto de las métricas de una fila de celdas, listo para escribirse en orden.
#[derive(Default)]
struct RowText {
    elevation: Vec<u8>,
    intensity: Vec<u8>,
    strata: Vec<u8>,
    structure: Vec<u8>,
    with_metrics: usize,
}

/// Métricas de intensidad: los estadísticos de distribución del archivo de elevación (sus columnas
/// 5 a 38), de las intensidades.
const INT_STATS: usize = 34;

fn int_header() -> String {
    COLUMNS[..4 + INT_STATS]
        .iter()
        .map(|c| c.replacen("Elev ", "Int ", 1))
        .collect::<Vec<_>>()
        .join(",")
}

/// Los estratos de `limits`: (nombre, inferior, superior).
fn strata_names(limits: &[f64]) -> Vec<(String, f64, f64)> {
    let mut out = Vec::new();
    let mut low = f64::NEG_INFINITY;
    for &up in limits {
        let name = if low == f64::NEG_INFINITY {
            format!("below {up:.2}")
        } else {
            format!("{low:.2} to {up:.2}")
        };
        out.push((name, low, up));
        low = up;
    }
    if let Some(&last) = limits.last() {
        out.push((format!("above {last:.2}"), last, f64::INFINITY));
    }
    out
}

const STRATA_STATS: [&str; 11] = [
    "total return count",
    "return proportion",
    "min",
    "max",
    "mean",
    "mode",
    "median",
    "stddev",
    "CV",
    "skewness",
    "kurtosis",
];

fn strata_header(p: &MetricParams) -> String {
    let mut cols = vec!["row".to_string(), "col".to_string()];
    for (kind, limits) in [("Elev", &p.strata), ("Int", &p.int_strata)] {
        for (name, _, _) in strata_names(limits) {
            cols.extend(
                STRATA_STATS
                    .iter()
                    .map(|s| format!("{kind} strata ({name}) {s}")),
            );
        }
    }
    cols.join(",")
}

/// Los estadísticos de un estrato, con el formato de esa disposición.
fn stratum(out: &mut dyn Write, v: &mut [f32], total: usize) -> std::io::Result<()> {
    let n = v.len();
    if n == 0 {
        out.write_all(b",0,0.0")?;
        for _ in 0..9 {
            out.write_all(b",-9999.0")?;
        }
        return Ok(());
    }
    let nf = n as f64;
    if n < 3 {
        // Los estratos de menos de 3 puntos reciben solo su media.
        let mean = v.iter().map(|&x| f64::from(x)).sum::<f64>() / nf;
        write!(
            out,
            ",{n},{:.6},-9999.0,-9999.0,{mean:.6}",
            nf / total.max(1) as f64
        )?;
        for _ in 0..6 {
            out.write_all(b",-9999.0")?;
        }
        return Ok(());
    }
    let (mut min, mut max, mut sum) = (f64::INFINITY, f64::NEG_INFINITY, 0.0);
    for &x in v.iter() {
        let x = f64::from(x);
        min = min.min(x);
        max = max.max(x);
        sum += x;
    }
    let mean = sum / nf;
    let (mut m2, mut m3, mut m4) = (0.0, 0.0, 0.0);
    for &x in v.iter() {
        let d = f64::from(x) - mean;
        m2 += d * d;
        m3 += d * d * d;
        m4 += d * d * d * d;
    }
    let var = if n > 1 { m2 / (nf - 1.0) } else { 0.0 };
    let sd = var.sqrt();
    let skew = if sd > 0.0 {
        m3 / ((nf - 1.0) * sd.powi(3))
    } else {
        0.0
    };
    let kurt = if sd > 0.0 {
        m4 / ((nf - 1.0) * sd.powi(4))
    } else {
        0.0
    };
    let mode_v = mode(v, min, max);
    v.sort_unstable_by(f32::total_cmp);
    let median = percentile(v, 0.5);
    let cv = if mean != 0.0 { sd / mean } else { 0.0 };
    write!(out, ",{n},{:.6}", nf / total.max(1) as f64)?;
    for x in [min, max, mean, mode_v, median, sd, cv, skew, kurt] {
        write!(out, ",{x:.6}")?;
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq)]
pub struct MetricParams {
    pub cell: f64,
    /// Altura de corte para las métricas de cobertura.
    pub heightbreak: f64,
    /// Las celdas con menos puntos no reciben métricas.
    pub min_points: u32,
    /// Máximo de bytes de alturas de puntos retenidos a la vez.
    pub memory: u64,
    /// Límites superiores de los estratos de altura (estratos de altura) y de los estratos de
    /// intensidad (estratos de intensidad).
    pub strata: Vec<f64>,
    pub int_strata: Vec<f64>,
    /// Calcula también las métricas de estructura (escritas junto a las demás).
    pub structure: Option<StructureParams>,
}

impl Default for MetricParams {
    fn default() -> Self {
        Self {
            cell: 15.0,
            heightbreak: 2.0,
            min_points: 4,
            memory: 512 << 20,
            strata: vec![0.15, 1.37, 5.0, 10.0, 20.0, 30.0],
            int_strata: vec![0.15, 1.37],
            structure: None,
        }
    }
}

/// El percentil («método 5» de Resampling Stats): lineal entre estadísticos de orden, en el rango
/// `p (n - 1)`. `v` está ordenado y no vacío.
fn percentile(v: &[f32], p: f64) -> f64 {
    // El rango se calcula en precisión simple, lo que elige los mismos estadísticos de orden y la
    // misma fracción, algo que importa donde valores vecinos están lejos unos de otros.
    let idx32 = p as f32 * (v.len() - 1) as f32;
    let i = idx32.floor() as usize;
    let idx = i as f64 + f64::from(idx32 - idx32.floor());
    let lo = f64::from(v.get(i).copied().unwrap_or(0.0));
    let hi = f64::from(
        v.get(i + 1)
            .copied()
            .unwrap_or(v.get(i).copied().unwrap_or(0.0)),
    );
    lo + (idx - i as f64) * (hi - lo)
}

/// La moda: 64 intervalos de anchura `(max - min) / 63` (solo el máximo abre el último) y el borde
/// inferior del más lleno, el más bajo en caso de empate.
fn mode(v: &[f32], min: f64, max: f64) -> f64 {
    const BINS: usize = 64;
    let width = (max - min) / (BINS - 1) as f64;
    if !(width > 0.0) {
        return min;
    }
    let mut hist = [0u32; BINS];
    for &x in v {
        // Multiplicar primero mantiene los datos de números enteros (intensidades) en bordes de
        // intervalo exactos.
        let b = (((f64::from(x) - min) * (BINS - 1) as f64 / (max - min)) as usize).min(BINS - 1);
        if let Some(h) = hist.get_mut(b) {
            *h += 1;
        }
    }
    let mut best = 0;
    for (i, &h) in hist.iter().enumerate() {
        if h > hist.get(best).copied().unwrap_or(0) {
            best = i;
        }
    }
    min + best as f64 * width
}

/// Las métricas de una celda: `h` sus alturas (ordenadas aquí), `ret` sus números de retorno.
fn cell_metrics(h: &mut [f32], ret: &[u8], hb: f64) -> Vec<f64> {
    let n = h.len() as f64;
    let (mut sum, mut sum2, mut sum3) = (0.0, 0.0, 0.0);
    let (mut min, mut max) = (f64::INFINITY, f64::NEG_INFINITY);
    for &x in h.iter() {
        let x = f64::from(x);
        sum += x;
        sum2 += x * x;
        // La media cúbica es de valores absolutos.
        sum3 += x.abs().powi(3);
        min = min.min(x);
        max = max.max(x);
    }
    let mean = sum / n;
    let (mut m2, mut m3, mut m4, mut aad) = (0.0, 0.0, 0.0, 0.0);
    for &x in h.iter() {
        let d = f64::from(x) - mean;
        m2 += d * d;
        m3 += d * d * d;
        m4 += d * d * d * d;
        aad += d.abs();
    }
    let variance = if n > 1.0 { m2 / (n - 1.0) } else { 0.0 };
    let sd = variance.sqrt();
    let skew = if sd > 0.0 {
        m3 / ((n - 1.0) * sd.powi(3))
    } else {
        0.0
    };
    let kurt = if sd > 0.0 {
        m4 / ((n - 1.0) * sd.powi(4))
    } else {
        0.0
    };
    let mode_v = mode(h, min, max);

    // Recuentos que emparejan una altura con su número de retorno, antes de ordenar las alturas.
    let mut by_ret = [0u32; 10];
    let (
        mut first,
        mut first_hb,
        mut all_hb,
        mut first_mean,
        mut first_mode,
        mut all_mean,
        mut all_mode,
    ) = (0u32, 0u32, 0u32, 0u32, 0u32, 0u32, 0u32);
    for (&x, &r) in h.iter().zip(ret) {
        let x = f64::from(x);
        let slot = if (1..=9).contains(&r) {
            usize::from(r) - 1
        } else {
            9
        };
        if let Some(c) = by_ret.get_mut(slot) {
            *c += 1;
        }
        let is_first = r == 1;
        first += u32::from(is_first);
        all_hb += u32::from(x > hb);
        all_mean += u32::from(x > mean);
        all_mode += u32::from(x > mode_v);
        if is_first {
            first_hb += u32::from(x > hb);
            first_mean += u32::from(x > mean);
            first_mode += u32::from(x > mode_v);
        }
    }

    h.sort_unstable_by(f32::total_cmp);
    let pct: Vec<f64> = PERCENTILES.iter().map(|&p| percentile(h, p)).collect();
    let p = |q: f64| percentile(h, q);
    let median = p(0.5);

    // Momentos L a partir de momentos ponderados por probabilidad (insesgados, Wang 1996).
    let (mut b0, mut b1, mut b2, mut b3) = (0.0, 0.0, 0.0, 0.0);
    for (i, &x) in h.iter().enumerate() {
        let (x, i) = (f64::from(x), i as f64);
        b0 += x;
        if n > 1.0 {
            b1 += x * i / (n - 1.0);
        }
        if n > 2.0 {
            b2 += x * i * (i - 1.0) / ((n - 1.0) * (n - 2.0));
        }
        if n > 3.0 {
            b3 += x * i * (i - 1.0) * (i - 2.0) / ((n - 1.0) * (n - 2.0) * (n - 3.0));
        }
    }
    let (b0, b1, b2, b3) = (b0 / n, b1 / n, b2 / n, b3 / n);
    let l1 = b0;
    let l2 = 2.0 * b1 - b0;
    let l3 = 6.0 * b2 - 6.0 * b1 + b0;
    let l4 = 20.0 * b3 - 30.0 * b2 + 12.0 * b1 - b0;
    let ratio = |a: f64, b: f64| if b != 0.0 { a / b } else { 0.0 };

    // Desviaciones absolutas medianas, respecto a la mediana y a la moda.
    let mut dev: Vec<f32> = h
        .iter()
        .map(|&x| (f64::from(x) - median).abs() as f32)
        .collect();
    dev.sort_unstable_by(f32::total_cmp);
    let mad_median = percentile(&dev, 0.5);
    for (d, &x) in dev.iter_mut().zip(h.iter()) {
        *d = (f64::from(x) - mode_v).abs() as f32;
    }
    dev.sort_unstable_by(f32::total_cmp);
    let mad_mode = percentile(&dev, 0.5);

    // Área del perfil: el área bajo la curva de percentiles 1-99 de las alturas (como mínimo 0),
    // normalizada por el percentil 99.
    let mut clamped: Vec<f32> = h.iter().map(|&x| x.max(0.0)).collect();
    clamped.sort_unstable_by(f32::total_cmp);
    let p99 = percentile(&clamped, 0.99);
    let profile = if p99 > 0.0 {
        let v: Vec<f64> = (1..=99)
            .map(|k| percentile(&clamped, f64::from(k) / 100.0) / p99)
            .collect();
        v.windows(2).map(|w| (w[0] + w[1]) / 2.0).sum::<f64>()
    } else {
        -9999.0
    };

    let (f, a) = (f64::from(first), n);
    let pc = |c: u32, of: f64| {
        if of > 0.0 {
            f64::from(c) / of * 100.0
        } else {
            0.0
        }
    };
    let mut out = vec![
        n,
        min,
        max,
        mean,
        mode_v,
        sd,
        variance,
        ratio(sd, mean),
        p(0.75) - p(0.25),
        skew,
        kurt,
        aad / n,
        l1,
        l2,
        l3,
        l4,
        ratio(l2, l1),
        ratio(l3, l2),
        ratio(l4, l2),
    ];
    out.extend(pct);
    out.extend(by_ret.iter().map(|&c| f64::from(c)));
    out.extend([
        pc(first_hb, f),
        pc(all_hb, a),
        pc(all_hb, f),
        f64::from(first_hb),
        f64::from(all_hb),
        pc(first_mean, f),
        pc(first_mode, f),
        pc(all_mean, a),
        pc(all_mode, a),
        pc(all_mean, f),
        pc(all_mode, f),
        f64::from(first_mean),
        f64::from(first_mode),
        f64::from(all_mean),
        f64::from(all_mode),
        f,
        a,
        mad_median,
        mad_mode,
        ratio(mean - min, max - min),
        (sum2 / n).sqrt(),
        (sum3 / n).cbrt(),
        profile,
    ]);
    out
}

/// Índices de columna (en los 67 valores tras fila, columna y centro) escritos como números
/// enteros.
fn is_count(k: usize) -> bool {
    // Total, los 10 recuentos de retorno y los 4 + 4 + 2 recuentos de cobertura.
    k == 0 || (34..44).contains(&k) || (47..49).contains(&k) || (55..61).contains(&k)
}

/// La cabecera de una fila de métricas por muestra (tabla por muestra): las métricas de elevación y
/// luego las de intensidad.
pub fn sample_header(heightbreak: f64) -> Vec<String> {
    // (la columna de la fecha de captura la añade quien llama, que conoce los archivos)
    let mut cols: Vec<String> = COLUMNS[4..]
        .iter()
        .map(|c| c.replace("{hb}", &format!("{heightbreak:.2}")))
        .collect();
    cols.extend(
        COLUMNS[4..4 + INT_STATS]
            .iter()
            .map(|c| c.replacen("Elev ", "Int ", 1)),
    );
    cols
}

/// Las métricas de una muestra: `heights` e `intensity` de sus puntos, `ret` sus números de
/// retorno. Las muestras vacías dan -9999 en todo, como exige la disposición compatible.
pub fn sample_row(
    heights: &mut [f32],
    ret: &[u8],
    intensity: &mut [f32],
    heightbreak: f64,
) -> Vec<String> {
    let total = 67 + INT_STATS;
    if heights.is_empty() {
        return (0..total)
            .map(|k| {
                if is_count(k % 67) {
                    "-9999".to_string()
                } else {
                    "-9999.0000".to_string()
                }
            })
            .collect();
    }
    let fmt = |k: usize, v: f64| {
        if is_count(k) {
            format!("{}", v as i64)
        } else {
            format!("{v:.4}")
        }
    };
    let mut row: Vec<String> = cell_metrics(heights, ret, heightbreak)
        .into_iter()
        .enumerate()
        .map(|(k, v)| fmt(k, v))
        .collect();
    row.extend(
        cell_metrics(intensity, ret, heightbreak)
            .into_iter()
            .take(INT_STATS)
            .enumerate()
            .map(|(k, v)| fmt(k, v)),
    );
    row
}

/// Lo que hizo la ejecución.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MetricReport {
    pub cells: usize,
    pub cells_with_metrics: usize,
    pub bands: usize,
    pub points: u64,
}

/// Lo que necesita una hoja de un bloque además de sus propios puntos para dar a las celdas de su
/// borde lo que tendrían si el bloque fuera un solo archivo.
#[derive(Default)]
pub struct Context<'a> {
    /// Puntos de las hojas vecinas cercanas a esta: cuentan en las celdas en que caen.
    pub halo: &'a [crate::las::Point],
    /// Si esta hoja escribe la celda centrada en `(x, y)`; cada celda de un bloque la escribe una
    /// sola hoja. `None`: todas.
    pub owns: Option<&'a (dyn Fn(f64, f64) -> bool + Sync)>,
}

/// Los puntos propios de la hoja y luego el halo de sus vecinas.
fn scan_with_halo(
    reader: &Reader,
    halo: &[crate::las::Point],
    mut f: impl FnMut(&crate::las::Point),
) -> Result<()> {
    reader.scan(&mut f)?;
    halo.iter().for_each(f);
    Ok(())
}

/// Escribe las métricas de cada celda como CSV, empezando por la fila más al norte.
pub fn grid_metrics(
    reader: &Reader,
    dtm: &Grid,
    p: &MetricParams,
    outs: Outputs,
) -> Result<MetricReport> {
    grid_metrics_in(reader, dtm, p, outs, &Context::default())
}

/// [`grid_metrics`] sobre una hoja de un bloque: con los puntos de sus vecinas cerca de sus bordes
/// y escribiendo solo las celdas que le pertenecen.
pub fn grid_metrics_in(
    reader: &Reader,
    dtm: &Grid,
    p: &MetricParams,
    outs: Outputs,
    ctx: &Context,
) -> Result<MetricReport> {
    let Outputs {
        elevation: out,
        mut intensity,
        mut strata,
        mut structure,
    } = outs;
    let structure_params = match (&structure, &p.structure) {
        (Some(_), Some(sp)) => Some(sp.clone()),
        (Some(_), None) => Some(StructureParams::default()),
        _ => None,
    };
    if let Some(sp) = &structure_params {
        sp.check()?;
    }
    let with_int = intensity.is_some() || strata.is_some();
    let per_point = if with_int {
        BYTES_PER_POINT_INT
    } else {
        BYTES_PER_POINT
    };
    if !(p.cell > 0.0) || p.memory < per_point {
        return Err(Error::Param(
            "las métricas necesitan una celda positiva y un límite de memoria".into(),
        ));
    }
    let h = &reader.header;
    // Alineación: centros de celda en múltiplos del tamaño de celda.
    let (c0, r0) = (
        (h.bounds[0] / p.cell).floor(),
        (h.bounds[1] / p.cell).floor(),
    );
    let cols = ((h.bounds[3] / p.cell).ceil() - c0) as usize + 1;
    let rows = ((h.bounds[4] / p.cell).ceil() - r0) as usize + 1;
    let grid = Grid::new(
        c0 * p.cell - p.cell / 2.0,
        r0 * p.cell - p.cell / 2.0,
        p.cell,
        cols,
        rows,
    )?;

    // las celdas que escribe esta ejecución
    let owned: Option<Vec<bool>> = ctx.owns.map(|owns| {
        (0..grid.len())
            .map(|i| {
                let (cx, cy) = grid.center(i % cols, i / cols);
                owns(cx, cy)
            })
            .collect()
    });
    let is_owned = |i: usize| owned.as_ref().is_none_or(|o| o.get(i).copied().unwrap_or(false));
    let height = |pt: &crate::las::Point| -> Option<(usize, f32)> {
        if pt.is_noise() {
            return None;
        }
        let i = grid.index(pt.x, pt.y)?;
        if !is_owned(i) {
            return None;
        }
        Some((i, pt.z as f32 - dtm.sample(pt.x, pt.y)?))
    };

    // Pasada 1: puntos por celda.
    let mut count = vec![0u32; grid.len()];
    let mut report = MetricReport {
        cells: (0..grid.len()).filter(|&i| is_owned(i)).count(),
        ..Default::default()
    };
    scan_with_halo(reader, ctx.halo, |pt| {
        if let Some((i, _)) = height(pt)
            && let Some(c) = count.get_mut(i)
        {
            *c += 1;
            report.points += 1;
        }
    })?;

    // El día en que se capturaron los datos es una métrica más de cada celda.
    let date = reader.capture_date().unwrap_or_else(|| "NA".into());
    let header: Vec<String> = COLUMNS
        .iter()
        .map(|c| c.replace("{hb}", &format!("{:.2}", p.heightbreak)))
        .collect();
    writeln!(out, "{},Date", header.join(","))?;
    if let Some(o) = intensity.as_mut() {
        writeln!(o, "{},Date", int_header())?;
    }
    if let Some(o) = strata.as_mut() {
        writeln!(o, "{},Date", strata_header(p))?;
    }
    if let Some(o) = structure.as_mut() {
        writeln!(
            o,
            "row,col,center X,center Y,{},Date",
            STRUCTURE_COLUMNS.join(",")
        )?;
    }
    let (elev_strata, int_strata) = (strata_names(&p.strata), strata_names(&p.int_strata));

    let budget = (p.memory / per_point).max(1);
    let mut row = 0usize;
    while row < rows {
        // La banda: tantas filas enteras como quepan en el presupuesto (al menos una).
        let mut end = row;
        let mut pts = 0u64;
        while end < rows {
            let in_row: u64 = count
                .get(end * cols..(end + 1) * cols)
                .map_or(0, |s| s.iter().map(|&c| u64::from(c)).sum());
            if end > row && pts + in_row > budget {
                break;
            }
            pts += in_row;
            end += 1;
        }
        let (first_cell, last_cell) = (row * cols, end * cols);
        let mut start = Vec::with_capacity(last_cell - first_cell + 1);
        let mut acc = 0usize;
        start.push(0);
        for c in count.get(first_cell..last_cell).unwrap_or(&[]) {
            acc += *c as usize;
            start.push(acc);
        }
        let mut heights = vec![0f32; acc];
        let mut rets = vec![0u8; acc];
        let mut ints = vec![0u16; if with_int { acc } else { 0 }];
        let mut fill = vec![0u32; last_cell - first_cell];
        if acc > 0 {
            scan_with_halo(reader, ctx.halo, |pt| {
                let Some((i, ht)) = height(pt) else { return };
                if i < first_cell || i >= last_cell {
                    return;
                }
                let k = i - first_cell;
                if let (Some(s), Some(f)) = (start.get(k), fill.get_mut(k)) {
                    let at = s + *f as usize;
                    if let (Some(hv), Some(rv)) = (heights.get_mut(at), rets.get_mut(at)) {
                        *hv = ht;
                        *rv = pt.return_number;
                        *f += 1;
                    }
                    if let Some(iv) = ints.get_mut(at) {
                        *iv = pt.intensity;
                    }
                }
            })?;
        }
        report.bands += 1;
        crate::check_cancel()?;
        // Cada fila de celdas se formatea en su propio hilo y se escribe en orden después: lo que
        // corre en paralelo es la ordenación de las alturas de una celda (la parte costosa).
        let (want_int, want_strata) = (intensity.is_some(), strata.is_some());
        let mut h_rows: Vec<&mut [f32]> = Vec::with_capacity(end - row);
        {
            let mut rest: &mut [f32] = &mut heights;
            for ri in 0..end - row {
                let (a, b) = (start[ri * cols], start[(ri + 1) * cols]);
                let (cur, tail) = rest.split_at_mut(b - a);
                h_rows.push(cur);
                rest = tail;
            }
        }
        let texts: Vec<std::io::Result<RowText>> = h_rows
            .par_iter_mut()
            .enumerate()
            .map(|(ri, hrow)| -> std::io::Result<RowText> {
                let r = row + ri;
                let base = start[ri * cols];
                let mut t = RowText::default();
                for c in 0..cols {
                    let k = ri * cols + c;
                    // una celda que escribe otra hoja del bloque
                    if !is_owned(row * cols + k) {
                        continue;
                    }
                    let (abs_s, abs_e) = (start[k], start[k + 1]);
                    let (s, e) = (abs_s - base, abs_e - base);
                    let (cx, cy) = grid.center(c, r);
                    let prefix = format!("{},{},{:.4},{:.4}", rows - 1 - r, c, cx, cy);
                    t.elevation.extend_from_slice(prefix.as_bytes());
                    let n = e - s;
                    let enough = n >= p.min_points as usize && n > 0;
                    let rs = &rets[abs_s..abs_e];
                    // La intensidad y los estratos leen las alturas sin ordenar, así que van
                    // primero.
                    if want_int {
                        let o = &mut t.intensity;
                        o.extend_from_slice(prefix.as_bytes());
                        if enough {
                            let mut iv: Vec<f32> =
                                ints[abs_s..abs_e].iter().map(|&v| f32::from(v)).collect();
                            let stats = cell_metrics(&mut iv, rs, p.heightbreak);
                            for (k, v) in stats.into_iter().take(INT_STATS).enumerate() {
                                if k == 0 {
                                    write!(o, ",{}", v as i64)?;
                                } else {
                                    write!(o, ",{v:.4}")?;
                                }
                            }
                        } else {
                            for k in 0..INT_STATS {
                                o.write_all(if k == 0 { b",-9999" } else { b",-9999.0000" })?;
                            }
                        }
                        writeln!(o, ",{date}")?;
                    }
                    if want_strata {
                        let o = &mut t.strata;
                        write!(o, "{},{}", rows - 1 - r, c)?;
                        let (hs, is) = (&hrow[s..e], &ints[abs_s..abs_e]);
                        for (by_height, limits) in [(true, &elev_strata), (false, &int_strata)] {
                            for (_, low, up) in limits {
                                if !enough {
                                    o.write_all(b",-9999,-9999.0")?;
                                    for _ in 0..9 {
                                        o.write_all(b",-9999.0")?;
                                    }
                                    continue;
                                }
                                let mut v: Vec<f32> = hs
                                    .iter()
                                    .zip(is)
                                    .filter(|(h, _)| f64::from(**h) >= *low && f64::from(**h) < *up)
                                    .map(|(h, i)| if by_height { *h } else { f32::from(*i) })
                                    .collect();
                                stratum(&mut *o, &mut v, n)?;
                            }
                        }
                        writeln!(o, ",{date}")?;
                    }
                    if let Some(sp) = &structure_params {
                        let o = &mut t.structure;
                        o.extend_from_slice(prefix.as_bytes());
                        if enough {
                            let m = structure_metrics(&hrow[s..e], sp);
                            write!(o, ",{n}")?;
                            for v in m {
                                if v.is_nan() {
                                    o.write_all(b",-9999.0000")?;
                                } else {
                                    write!(o, ",{v:.4}")?;
                                }
                            }
                        } else {
                            o.write_all(b",-9999")?;
                            for _ in 0..5 {
                                o.write_all(b",-9999.0000")?;
                            }
                        }
                        writeln!(o, ",{date}")?;
                    }
                    if enough {
                        for (k, v) in cell_metrics(&mut hrow[s..e], rs, p.heightbreak)
                            .into_iter()
                            .enumerate()
                        {
                            if is_count(k) {
                                write!(t.elevation, ",{}", v as i64)?;
                            } else {
                                write!(t.elevation, ",{v:.4}")?;
                            }
                        }
                        t.with_metrics += 1;
                    } else {
                        for k in 0..67 {
                            t.elevation.write_all(if is_count(k) {
                                b",-9999"
                            } else {
                                b",-9999.0000"
                            })?;
                        }
                    }
                    writeln!(t.elevation, ",{date}")?;
                }
                Ok(t)
            })
            .collect();
        for t in texts {
            let t = t?;
            out.write_all(&t.elevation)?;
            if let Some(o) = intensity.as_mut() {
                o.write_all(&t.intensity)?;
            }
            if let Some(o) = strata.as_mut() {
                o.write_all(&t.strata)?;
            }
            if let Some(o) = structure.as_mut() {
                o.write_all(&t.structure)?;
            }
            report.cells_with_metrics += t.with_metrics;
        }
        row = end;
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn structure_of_bare_ground_a_uniform_profile_and_a_crown() {
        let sp = StructureParams::default();
        // toda la luz llega al suelo
        let bare = structure_metrics(&[0.0, 0.2, 0.1, 0.4], &sp);
        assert_eq!((bare[0], bare[1], bare[2]), (1.0, 0.0, 0.0));
        assert!(bare[4].is_nan());
        // diez capas de 1 m con el mismo número de retornos cada una: entropía ln(10)
        let heights: Vec<f32> = (0..100).map(|i| (i % 10) as f32 + 0.5).collect();
        let uniform = structure_metrics(&heights, &sp);
        assert!((uniform[0] - 0.1).abs() < 1e-12);
        assert!((uniform[1] - (-(0.1f64).ln() / 0.5)).abs() < 1e-12);
        assert!((uniform[2] - 10f64.ln()).abs() < 1e-9);
        assert!((uniform[3] - 10f64.ln() / 30f64.ln()).abs() < 1e-9);
        assert_eq!(uniform[4], 2.0);
        // retornos de suelo y una copa entre 8 y 12 m: la base está en 8
        let mut crown: Vec<f32> = vec![0.3; 40];
        crown.extend((0..60).map(|i| 8.0 + (i % 4) as f32 + 0.5));
        let c = structure_metrics(&crown, &sp);
        assert_eq!(c[4], 8.0);
        assert!((c[0] - 0.4).abs() < 1e-12);
        // sin retornos, sin respuesta
        assert!(structure_metrics(&[], &sp)[0].is_nan());
        // un perfil que necesitaría demasiadas capas se rechaza
        let bad = StructureParams { layer: 0.01, ..sp };
        assert!(bad.check().is_err());
    }

    #[test]
    fn percentile_is_method_5() {
        let v = [1.0, 2.0, 3.0, 4.0];
        assert_eq!(percentile(&v, 0.0), 1.0);
        assert_eq!(percentile(&v, 0.5), 2.5);
        assert_eq!(percentile(&v, 1.0), 4.0);
    }

    #[test]
    fn counts_and_cover() {
        let mut h = [0.1f32, 0.2, 5.0, 10.0];
        let ret = [1u8, 1, 1, 2];
        let m = cell_metrics(&mut h, &ret, 2.0);
        assert_eq!(m.len(), 67);
        assert_eq!(m[0], 4.0);
        // 3 primeros retornos, 1 por encima de 2 m; 2 de los 4 por encima.
        assert!((m[44] - 100.0 / 3.0).abs() < 1e-9);
        assert_eq!(m[45], 50.0);
        assert_eq!(m[59], 3.0);
    }
}
