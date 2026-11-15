//! Qué puntos toma una herramienta o una exportación: por clase, por retorno, por elevación,
//! adelgazados.

use crate::classes::ClassSet;
use crate::las::Point;
use crate::{Error, Result};

/// Un conjunto de clases. `parse` lee la forma compatible con FUSION: `2,3,5` o `~7,18` (todas
/// menos esas).
#[derive(Clone, Debug, PartialEq)]
pub struct ClassSel {
    pub set: ClassSet,
}

impl ClassSel {
    pub fn all() -> Self {
        Self {
            set: ClassSet::all(),
        }
    }

    pub fn parse(text: &str) -> Result<Self> {
        let text = text.trim();
        let (invert, list) = match text.strip_prefix('~') {
            Some(rest) => (true, rest),
            None => (false, text),
        };
        let mut set = ClassSet::none();
        for part in list.split([',', ' ', ';']).filter(|p| !p.is_empty()) {
            let c: u8 = part.parse().map_err(|_| {
                Error::Param(format!("la clase '{part}' no es un número de 0 a 255"))
            })?;
            set.set(c, true);
        }
        Ok(Self {
            set: if invert { set.inverted() } else { set },
        })
    }

    #[inline]
    pub fn allows(&self, class: u8) -> bool {
        self.set.contains(class)
    }

    /// Pone una clase dentro o fuera.
    pub fn set(&mut self, class: u8, on: bool) {
        self.set.set(class, on);
    }
}

/// El tipo de un retorno dentro de su pulso.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ReturnKind {
    Single,
    First,
    Intermediate,
    Last,
}

impl ReturnKind {
    pub fn of(p: &Point) -> Self {
        let (r, n) = (p.return_number.max(1), p.number_of_returns.max(1));
        if n == 1 {
            Self::Single
        } else if r == 1 {
            Self::First
        } else if r >= n {
            Self::Last
        } else {
            Self::Intermediate
        }
    }

    pub fn index(self) -> usize {
        self as usize
    }
}

/// Un conjunto de retornos, por tipo y por número. `parse` lee la forma compatible con FUSION: `A`,
/// `F`, `L`, `1235`.
#[derive(Clone, Debug, PartialEq)]
pub struct ReturnSel {
    pub kinds: [bool; 4],
    /// Bit `n`: número de retorno `n`.
    pub numbers: u16,
}

impl ReturnSel {
    pub fn all() -> Self {
        Self {
            kinds: [true; 4],
            numbers: 0,
        }
    }

    pub fn parse(text: &str) -> Result<Self> {
        let mut sel = Self {
            kinds: [false; 4],
            numbers: 0,
        };
        for c in text.trim().chars() {
            match c.to_ascii_uppercase() {
                'A' => sel.kinds = [true; 4],
                'F' => {
                    sel.kinds[ReturnKind::Single.index()] = true;
                    sel.kinds[ReturnKind::First.index()] = true;
                }
                'L' => sel.kinds[ReturnKind::Last.index()] = true,
                d @ '1'..='9' => sel.numbers |= 1 << (d as u8 - b'0'),
                ',' | ' ' | ';' => {}
                other => {
                    return Err(Error::Param(format!(
                        "'{other}' is not a return: use A, F, L or 1 to 9"
                    )));
                }
            }
        }
        Ok(sel)
    }

    #[inline]
    pub fn allows(&self, p: &Point) -> bool {
        self.kinds[ReturnKind::of(p).index()] || (self.numbers >> p.return_number.min(15)) & 1 == 1
    }
}

/// Los puntos que toma una herramienta.
#[derive(Clone, Debug, PartialEq)]
pub struct Filter {
    pub classes: ClassSel,
    pub returns: ReturnSel,
    /// Límites de elevación (alturas sobre el suelo cuando la herramienta normaliza).
    pub zmin: Option<f64>,
    pub zmax: Option<f64>,
    /// Conserva un punto de cada tantos (1 los conserva todos).
    pub decimate: u64,
    /// Los puntos marcados como retenidos y el ruido (7, 18) se dejan fuera.
    pub skip_noise: bool,
}

impl Default for Filter {
    fn default() -> Self {
        Self {
            classes: ClassSel::all(),
            returns: ReturnSel::all(),
            zmin: None,
            zmax: None,
            decimate: 1,
            skip_noise: false,
        }
    }
}

impl Filter {
    /// Si un punto pasa las pruebas de clase, retorno y ruido (no la de elevación ni la de
    /// adelgazamiento, que aplica quien llama al valor que quiere limitar).
    #[inline]
    pub fn keeps(&self, p: &Point) -> bool {
        (!self.skip_noise || !p.is_noise())
            && !p.withheld
            && self.classes.allows(p.class)
            && self.returns.allows(p)
    }

    #[inline]
    pub fn z_ok(&self, z: f64) -> bool {
        self.zmin.is_none_or(|m| z >= m) && self.zmax.is_none_or(|m| z <= m)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pt(class: u8, r: u8, n: u8) -> Point {
        Point {
            class,
            return_number: r,
            number_of_returns: n,
            ..Default::default()
        }
    }

    #[test]
    fn class_lists_accept_the_compatible_form() -> Result<()> {
        let s = ClassSel::parse("2,5")?;
        assert!(s.allows(2) && s.allows(5) && !s.allows(3) && !s.allows(70));
        let s = ClassSel::parse("~7,18")?;
        assert!(s.allows(2) && !s.allows(7) && !s.allows(18) && s.allows(70));
        assert!(ClassSel::parse("x").is_err());
        Ok(())
    }

    #[test]
    fn returns_by_kind_and_number() -> Result<()> {
        let first = ReturnSel::parse("F")?;
        assert!(
            first.allows(&pt(1, 1, 1)) && first.allows(&pt(1, 1, 3)) && !first.allows(&pt(1, 2, 3))
        );
        let last = ReturnSel::parse("L")?;
        assert!(last.allows(&pt(1, 3, 3)) && !last.allows(&pt(1, 1, 1)));
        let two = ReturnSel::parse("2")?;
        assert!(two.allows(&pt(1, 2, 3)) && !two.allows(&pt(1, 3, 3)));
        assert!(ReturnSel::parse("Q").is_err());
        Ok(())
    }
}
