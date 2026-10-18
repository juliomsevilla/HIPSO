//! Which points a tool or export takes: by class, by return, by elevation, thinned.

use crate::classes::ClassSet;
use crate::las::Point;
use crate::{Error, Result};

/// A set of classes. `parse` reads FUSION's form: `2,3,5` or `~7,18` (all but those).
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
                Error::Param(format!("class '{part}' is not a number from 0 to 255"))
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

    /// Sets a class in or out.
    pub fn set(&mut self, class: u8, on: bool) {
        self.set.set(class, on);
    }
}

/// The kind of a return within its pulse.
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

/// A set of returns, by kind and by number. `parse` reads FUSION's form: `A`, `F`, `L`, `1235`.
#[derive(Clone, Debug, PartialEq)]
pub struct ReturnSel {
    pub kinds: [bool; 4],
    /// Bit `n`: return number `n`.
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

/// The points a tool takes.
#[derive(Clone, Debug, PartialEq)]
pub struct Filter {
    pub classes: ClassSel,
    pub returns: ReturnSel,
    /// Elevation limits (heights above ground when the tool normalises).
    pub zmin: Option<f64>,
    pub zmax: Option<f64>,
    /// Keep one point in this many (1 keeps all).
    pub decimate: u64,
    /// Points marked withheld, and noise (7, 18), are left out.
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
    /// Whether a point passes class, return and noise tests (not elevation or thinning, which
    /// the caller applies to the value it wants limited).
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
    fn class_lists_follow_fusion() -> Result<()> {
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
