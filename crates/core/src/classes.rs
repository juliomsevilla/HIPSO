//! Point classes. LAS 1.4 stores a class in a byte; the ASPRS table (R15, 2019) defines 0–22,
//! reserves 23–63 and leaves 64–255 to the user. A [`ClassSet`] holds any subset of the 256.

/// A set of classes, one bit per code 0–255.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ClassSet([u64; 4]);

impl ClassSet {
    pub const fn all() -> Self {
        Self([u64::MAX; 4])
    }

    pub const fn none() -> Self {
        Self([0; 4])
    }

    /// The classes whose bit is set in `bits` (codes 0–31).
    pub const fn from_bits(bits: u32) -> Self {
        Self([bits as u64, 0, 0, 0])
    }

    #[inline]
    pub const fn contains(&self, class: u8) -> bool {
        self.0[(class >> 6) as usize] >> (class & 63) & 1 == 1
    }

    pub fn set(&mut self, class: u8, on: bool) {
        let (w, b) = ((class >> 6) as usize, class & 63);
        if on {
            self.0[w] |= 1 << b;
        } else {
            self.0[w] &= !(1 << b);
        }
    }

    pub fn toggle(&mut self, class: u8) {
        self.0[(class >> 6) as usize] ^= 1 << (class & 63);
    }

    /// The only class `c`.
    pub fn only(c: u8) -> Self {
        let mut s = Self::none();
        s.set(c, true);
        s
    }

    /// Every class but those in `self`.
    pub fn inverted(&self) -> Self {
        Self(self.0.map(|w| !w))
    }
}

impl Default for ClassSet {
    fn default() -> Self {
        Self::all()
    }
}

/// The ASPRS name of a class code (LAS 1.4 R15).
pub fn name(class: u8) -> &'static str {
    match class {
        0 => "Creado, nunca clasificado",
        1 => "Sin clasificar",
        2 => "Suelo",
        3 => "Vegetación baja",
        4 => "Vegetación media",
        5 => "Vegetación alta",
        6 => "Edificio",
        7 => "Punto bajo (ruido)",
        8 => "Reservado",
        9 => "Agua",
        10 => "Ferrocarril",
        11 => "Superficie de carretera",
        12 => "Reservado (solape en el PNOA)",
        13 => "Protección de cable",
        14 => "Conductor de cable",
        15 => "Torre de transmisión",
        16 => "Conector de cable",
        17 => "Tablero de puente",
        18 => "Ruido alto",
        19 => "Estructura elevada",
        20 => "Suelo ignorado",
        21 => "Nieve",
        22 => "Exclusión temporal",
        23..=63 => "Reservado",
        _ => "Definido por el usuario",
    }
}

/// Codes the ASPRS table gives a meaning to.
pub const DEFINED: std::ops::RangeInclusive<u8> = 0..=22;

/// First code left to the user.
pub const USER_FIRST: u8 = 64;

/// The colour a class is painted in unless the user changes it.
pub fn default_color(class: u8) -> [u8; 3] {
    match class {
        0 => [200, 200, 200],
        1 => [150, 150, 150],
        2 => [170, 110, 50],
        3 => [150, 215, 100],
        4 => [60, 175, 60],
        5 => [0, 110, 20],
        6 => [220, 50, 50],
        7 => [230, 0, 230],
        8 => [255, 150, 0],
        9 => [40, 100, 230],
        10 => [120, 120, 160],
        11 => [90, 90, 90],
        12 => [240, 220, 40],
        13 => [255, 255, 130],
        14 => [255, 200, 130],
        15 => [160, 110, 200],
        16 => [110, 110, 70],
        17 => [60, 60, 150],
        18 => [255, 80, 200],
        19 => [0, 170, 170],
        20 => [120, 80, 40],
        21 => [235, 245, 255],
        22 => [200, 120, 120],
        c => hue(c),
    }
}

/// Distinct colours for the codes without one: the golden angle spreads neighbours apart.
fn hue(c: u8) -> [u8; 3] {
    let h = (f32::from(c) * 137.508) % 360.0 / 60.0;
    let (s, v) = (0.65f32, 0.9f32);
    let f = h - h.floor();
    let (p, q, t) = (v * (1.0 - s), v * (1.0 - s * f), v * (1.0 - s * (1.0 - f)));
    let (r, g, b) = match h as u8 {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    };
    [(r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8]
}

/// The colours of all 256 classes.
pub fn default_colors() -> [[u8; 3]; 256] {
    let mut c = [[0u8; 3]; 256];
    for (k, e) in c.iter_mut().enumerate() {
        *e = default_color(k as u8);
    }
    c
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_holds_any_of_256() {
        let mut s = ClassSet::none();
        s.set(200, true);
        s.set(2, true);
        assert!(s.contains(200) && s.contains(2) && !s.contains(201));
        s.toggle(200);
        assert!(!s.contains(200));
        assert!(ClassSet::from_bits(1 << 5).contains(5));
        assert!(!ClassSet::only(7).inverted().contains(7));
    }
}
