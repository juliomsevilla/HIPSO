//! Symbology of the layers, in the manner of a desktop GIS: colour ramps, and for vectors the four
//! renderers (single symbol, unique values, graduated colours, proportional symbols) and labels.

use rustlidar_core::view::VectorData;

/// Named colour ramps of five stops, low to high.
pub const RAMPS: [(&str, [[u8; 3]; 5]); 8] = [
    (
        "Azul–rojo",
        [
            [40, 60, 220],
            [30, 190, 230],
            [70, 210, 70],
            [250, 220, 40],
            [225, 40, 30],
        ],
    ),
    (
        "Viridis",
        [
            [68, 1, 84],
            [59, 82, 139],
            [33, 145, 140],
            [94, 201, 98],
            [253, 231, 37],
        ],
    ),
    (
        "Rojo–amarillo–verde",
        [
            [215, 25, 28],
            [253, 174, 97],
            [255, 255, 191],
            [166, 217, 106],
            [26, 150, 65],
        ],
    ),
    (
        "Amarillo–marrón",
        [
            [255, 255, 204],
            [254, 217, 118],
            [254, 153, 41],
            [217, 95, 14],
            [102, 37, 6],
        ],
    ),
    (
        "Blancos–azules",
        [
            [239, 243, 255],
            [189, 215, 231],
            [107, 174, 214],
            [49, 130, 189],
            [8, 81, 156],
        ],
    ),
    (
        "Tierra",
        [
            [40, 90, 60],
            [120, 150, 70],
            [200, 180, 100],
            [160, 110, 70],
            [250, 250, 250],
        ],
    ),
    (
        "Espectral",
        [
            [43, 131, 186],
            [171, 221, 164],
            [255, 255, 191],
            [253, 174, 97],
            [215, 25, 28],
        ],
    ),
    (
        "Grises",
        [
            [0, 0, 0],
            [64, 64, 64],
            [128, 128, 128],
            [192, 192, 192],
            [255, 255, 255],
        ],
    ),
];

/// Distinct colours for categories.
pub const PALETTE: [[u8; 3]; 12] = [
    [141, 211, 199],
    [255, 255, 179],
    [190, 186, 218],
    [251, 128, 114],
    [128, 177, 211],
    [253, 180, 98],
    [179, 222, 105],
    [252, 205, 229],
    [188, 128, 189],
    [204, 235, 197],
    [255, 237, 111],
    [217, 217, 217],
];

/// The colour of ramp `ramp` at `t` in 0…1 (reversed when `invert`).
pub fn ramp_rgb(ramp: usize, t: f64, invert: bool) -> [u8; 3] {
    let stops = &RAMPS[ramp.min(RAMPS.len() - 1)].1;
    let t = if invert { 1.0 - t } else { t };
    let t = t.clamp(0.0, 1.0) * 4.0;
    let i = (t.floor() as usize).min(3);
    let f = t - i as f64;
    let c = |k: usize| {
        (f64::from(stops[i][k]) + (f64::from(stops[i + 1][k]) - f64::from(stops[i][k])) * f) as u8
    };
    [c(0), c(1), c(2)]
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum SymKind {
    #[default]
    Single,
    Unique,
    Graduated,
    Proportional,
}

impl SymKind {
    pub const ALL: [SymKind; 4] = [
        SymKind::Single,
        SymKind::Unique,
        SymKind::Graduated,
        SymKind::Proportional,
    ];

    pub fn name(self) -> &'static str {
        match self {
            SymKind::Single => "Símbolo único",
            SymKind::Unique => "Valores únicos",
            SymKind::Graduated => "Colores graduados",
            SymKind::Proportional => "Tamaño proporcional",
        }
    }
}

/// How the features of a layer are told apart.
#[derive(Clone, Debug)]
pub struct Symbology {
    pub kind: SymKind,
    pub field: Option<usize>,
    pub ramp: usize,
    pub invert: bool,
    pub classes: usize,
    pub quantile: bool,
    /// Graduated: `classes + 1` class limits.
    pub breaks: Vec<f64>,
    /// Graduated: one colour per class. Unique: one per category.
    pub colors: Vec<[u8; 3]>,
    /// Unique: the distinct values, sorted.
    pub categories: Vec<String>,
    /// Proportional: radius in pixels at the smallest and the largest value, and those values.
    pub size: (f32, f32),
    pub range: (f64, f64),
}

impl Default for Symbology {
    fn default() -> Self {
        Self {
            kind: SymKind::Single,
            field: None,
            ramp: 0,
            invert: false,
            classes: 5,
            quantile: false,
            breaks: Vec::new(),
            colors: Vec::new(),
            categories: Vec::new(),
            size: (3.0, 14.0),
            range: (0.0, 1.0),
        }
    }
}

const GREY: [u8; 3] = [150, 150, 150];

impl Symbology {
    /// The fields that hold numbers.
    pub fn numeric_fields(data: &VectorData) -> Vec<usize> {
        (0..data.fields.len())
            .filter(|&c| {
                data.num
                    .iter()
                    .any(|row| row.get(c).is_some_and(|x| x.is_finite()))
            })
            .collect()
    }

    fn values(&self, data: &VectorData) -> Vec<f64> {
        let Some(c) = self.field else {
            return Vec::new();
        };
        data.num
            .iter()
            .filter_map(|row| row.get(c))
            .copied()
            .filter(|x| x.is_finite())
            .collect()
    }

    /// Works out the classes (limits, categories and their colours) from the data.
    pub fn classify(&mut self, data: &VectorData) {
        self.breaks.clear();
        self.colors.clear();
        self.categories.clear();
        let Some(c) = self.field else {
            return;
        };
        match self.kind {
            SymKind::Single => {}
            SymKind::Graduated => {
                let mut v = self.values(data);
                if v.is_empty() {
                    return;
                }
                v.sort_by(f64::total_cmp);
                let n = self.classes.clamp(2, 12);
                let (lo, hi) = (v[0], v[v.len() - 1]);
                self.breaks = (0..=n)
                    .map(|k| {
                        if self.quantile {
                            let i = (k * (v.len() - 1)) / n;
                            v[i]
                        } else {
                            lo + (hi - lo) * k as f64 / n as f64
                        }
                    })
                    .collect();
                self.colors = (0..n)
                    .map(|k| ramp_rgb(self.ramp, (k as f64 + 0.5) / n as f64, self.invert))
                    .collect();
            }
            SymKind::Unique => {
                let mut cats: Vec<String> = data
                    .text
                    .iter()
                    .filter_map(|row| row.get(c))
                    .cloned()
                    .collect();
                cats.sort();
                cats.dedup();
                cats.truncate(300);
                let n = cats.len().max(1);
                self.colors = (0..cats.len())
                    .map(|k| {
                        if cats.len() <= PALETTE.len() {
                            PALETTE[k]
                        } else {
                            ramp_rgb(self.ramp, k as f64 / n as f64, self.invert)
                        }
                    })
                    .collect();
                self.categories = cats;
            }
            SymKind::Proportional => {
                let v = self.values(data);
                self.range = v.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |a, x| {
                    (a.0.min(*x), a.1.max(*x))
                });
                if !self.range.0.is_finite() {
                    self.range = (0.0, 1.0);
                }
            }
        }
    }

    /// The colour of feature `i`, or `base` when the symbology does not colour.
    pub fn color_of(&self, data: &VectorData, i: usize, base: [u8; 3]) -> [u8; 3] {
        let Some(c) = self.field else {
            return base;
        };
        match self.kind {
            SymKind::Graduated if self.breaks.len() > 2 => {
                let Some(v) = data
                    .num
                    .get(i)
                    .and_then(|r| r.get(c))
                    .copied()
                    .filter(|x| x.is_finite())
                else {
                    return GREY;
                };
                let inner = &self.breaks[1..self.breaks.len() - 1];
                let k = inner.partition_point(|b| *b <= v);
                self.colors.get(k).copied().unwrap_or(GREY)
            }
            SymKind::Unique => data
                .text
                .get(i)
                .and_then(|r| r.get(c))
                .and_then(|t| self.categories.binary_search(t).ok())
                .and_then(|k| self.colors.get(k).copied())
                .unwrap_or(GREY),
            _ => base,
        }
    }

    /// The radius of the point of feature `i` (the layer's own unless proportional).
    pub fn size_of(&self, data: &VectorData, i: usize, base: f32) -> f32 {
        let (Some(c), SymKind::Proportional) = (self.field, self.kind) else {
            return base;
        };
        let Some(v) = data
            .num
            .get(i)
            .and_then(|r| r.get(c))
            .copied()
            .filter(|x| x.is_finite())
        else {
            return self.size.0;
        };
        let t = ((v - self.range.0) / (self.range.1 - self.range.0).max(1e-12)).clamp(0.0, 1.0);
        self.size.0 + (self.size.1 - self.size.0) * t.sqrt() as f32
    }

    /// The legend: a label and a colour per entry.
    pub fn legend(&self, base: [u8; 3]) -> Vec<(String, [u8; 3])> {
        match self.kind {
            SymKind::Graduated if self.breaks.len() > 1 => self
                .colors
                .iter()
                .enumerate()
                .map(|(k, c)| {
                    (
                        format!("{} – {}", fmt(self.breaks[k]), fmt(self.breaks[k + 1])),
                        *c,
                    )
                })
                .collect(),
            SymKind::Unique => self
                .categories
                .iter()
                .zip(&self.colors)
                .map(|(t, c)| {
                    (
                        if t.is_empty() {
                            "(vacío)".to_string()
                        } else {
                            t.clone()
                        },
                        *c,
                    )
                })
                .collect(),
            _ => vec![(String::new(), base)],
        }
    }
}

fn fmt(v: f64) -> String {
    if v.abs() >= 1000.0 || v == v.trunc() {
        format!("{v:.0}")
    } else {
        format!("{v:.2}")
    }
}

/// How the features are labelled.
#[derive(Clone, Debug)]
pub struct Labels {
    pub field: Option<usize>,
    pub size: f32,
    pub color: [u8; 3],
    pub halo: bool,
}

impl Default for Labels {
    fn default() -> Self {
        Self {
            field: None,
            size: 12.0,
            color: [255, 255, 255],
            halo: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data() -> VectorData {
        let vals = [1.0, 2.0, 3.0, 4.0, 10.0];
        VectorData {
            fields: vec!["v".into()],
            text: vals.iter().map(|v| vec![v.to_string()]).collect(),
            num: vals.iter().map(|v| vec![*v]).collect(),
            ..VectorData::default()
        }
    }

    #[test]
    fn graduated_puts_values_in_their_class() {
        let d = data();
        let mut s = Symbology {
            kind: SymKind::Graduated,
            field: Some(0),
            classes: 3,
            ..Symbology::default()
        };
        s.classify(&d);
        assert_eq!(s.breaks.len(), 4);
        let low = s.color_of(&d, 0, [0; 3]);
        let high = s.color_of(&d, 4, [0; 3]);
        assert_eq!(low, s.colors[0]);
        assert_eq!(high, s.colors[2]);
    }

    #[test]
    fn unique_gives_each_value_its_colour() {
        let d = data();
        let mut s = Symbology {
            kind: SymKind::Unique,
            field: Some(0),
            ..Symbology::default()
        };
        s.classify(&d);
        assert_eq!(s.categories.len(), 5);
        assert_ne!(s.color_of(&d, 0, [0; 3]), s.color_of(&d, 1, [0; 3]));
    }
}
