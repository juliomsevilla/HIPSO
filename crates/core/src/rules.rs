//! Classification by rules on the echo of a point and the scan angle: the cheap criteria that say
//! a lot. The first returns of pulses that gave several are canopy and edges, the last ones are
//! what reached the ground; points scanned at a steep angle are less accurate and are best kept
//! out of the ground and the surfaces.
//!
//! A rule changes the class of the points that are of certain classes, are certain echoes (`S`
//! single, `F` first of several, `I` intermediate, `L` last of several; or return numbers `1` to
//! `9`) and lie in a range of scan angles; the rules are applied in order, so a later one sees
//! what an earlier one made.

use crate::classes::ClassSet;
use crate::filter::{ClassSel, ReturnKind, ReturnSel};
use crate::las::Point;
use crate::{Error, Result};

/// The class of ground, as the ground filters label it.
const GROUND: u8 = 2;

#[derive(Clone, Debug, PartialEq)]
pub struct ClassRule {
    /// Only points of these classes are looked at. Noise (7, 18) is left alone by default.
    pub from: ClassSet,
    /// Which echoes of the pulse.
    pub returns: ReturnSel,
    /// Absolute scan angle (degrees) from and to, both included; None: any angle.
    pub angle: Option<(f64, f64)>,
    pub to: u8,
}

impl ClassRule {
    /// A rule that sends every point that is not noise to class `to`.
    pub fn new(to: u8) -> Self {
        let mut from = ClassSet::all();
        from.set(7, false);
        from.set(18, false);
        Self {
            from,
            returns: ReturnSel::all(),
            angle: None,
            to,
        }
    }

    /// Whether the rule takes a point that is now of class `class`.
    pub fn takes(&self, p: &Point, class: u8) -> bool {
        self.from.contains(class)
            && self.returns.allows(p)
            && self.angle.is_none_or(|(lo, hi)| {
                let a = f64::from(p.scan_angle.abs());
                a >= lo && a <= hi
            })
    }

    /// Reads a rule from text: `to=12` and any of `from=1,2` (or `~7`), `returns=F` (`S`, `F`,
    /// `I`, `L`, `A`, or numbers: `FL` is first or last of several), `angle=25..90` (or
    /// `angle=25` for 25 and over), separated by `;`.
    pub fn parse(text: &str) -> Result<Self> {
        let mut rule = Self::new(0);
        let mut has_to = false;
        for part in text.split(';').map(str::trim).filter(|p| !p.is_empty()) {
            let (key, value) = part.split_once('=').ok_or_else(|| {
                Error::Param(format!(
                    "la parte de la regla '{part}' debe ser nombre=valor"
                ))
            })?;
            let value = value.trim();
            match key.trim() {
                "to" => {
                    rule.to = value.parse().map_err(|_| {
                        Error::Param(format!("la clase '{value}' no es un número de 0 a 255"))
                    })?;
                    has_to = true;
                }
                "from" => rule.from = ClassSel::parse(value)?.set,
                "returns" | "echo" => rule.returns = parse_returns(value)?,
                "angle" => {
                    let num = |t: &str| {
                        t.trim().replace(',', ".").parse::<f64>().map_err(|_| {
                            Error::Param(format!("el ángulo '{t}' no es un número de grados"))
                        })
                    };
                    rule.angle = Some(match value.split_once("..") {
                        Some((lo, hi)) => (num(lo)?, num(hi)?),
                        None => (num(value)?, 180.0),
                    });
                }
                other => {
                    return Err(Error::Param(format!(
                        "parte de regla desconocida '{other}': usa to, from, returns o angle"
                    )));
                }
            }
        }
        if !has_to {
            return Err(Error::Param(
                "una regla necesita la clase de destino: to=12".into(),
            ));
        }
        Ok(rule)
    }

    /// The rule as `parse` reads it, to show it.
    pub fn text(&self) -> String {
        let mut parts = Vec::new();
        if let Some((lo, hi)) = self.angle {
            parts.push(if hi >= 180.0 {
                format!("angle ≥ {lo}°")
            } else {
                format!("angle {lo}° a {hi}°")
            });
        }
        parts.push(format!("→ clase {}", self.to));
        parts.join(" ")
    }
}

/// The echoes of a rule from text: `S`, `F`, `I`, `L`, `A` or return numbers `1` to `9`.
fn parse_returns(text: &str) -> Result<ReturnSel> {
    let mut sel = ReturnSel {
        kinds: [false; 4],
        numbers: 0,
    };
    for c in text.trim().chars() {
        match c.to_ascii_uppercase() {
            'A' => sel.kinds = [true; 4],
            'S' => sel.kinds[ReturnKind::Single.index()] = true,
            'F' => sel.kinds[ReturnKind::First.index()] = true,
            'I' => sel.kinds[ReturnKind::Intermediate.index()] = true,
            'L' => sel.kinds[ReturnKind::Last.index()] = true,
            d @ '1'..='9' => sel.numbers |= 1 << (d as u8 - b'0'),
            ',' | ' ' => {}
            other => {
                return Err(Error::Param(format!(
                    "'{other}' no es un eco: usa S, F, I, L, A o de 1 a 9"
                )));
            }
        }
    }
    Ok(sel)
}

/// Applies the rules, in order, to the points of a tile. `class` holds the class a step has
/// already decided (None: the point keeps its own); `ground` follows it. Returns, for each point,
/// whether a rule changed its class.
pub fn apply(
    pts: &[Point],
    class: &mut [Option<u8>],
    ground: &mut [bool],
    rules: &[ClassRule],
) -> Vec<bool> {
    let mut changed = vec![false; pts.len()];
    if rules.is_empty() {
        return changed;
    }
    for (k, q) in pts.iter().enumerate() {
        let mut now = class.get(k).copied().flatten().unwrap_or(q.class);
        for r in rules {
            if now != r.to && r.takes(q, now) {
                now = r.to;
                changed[k] = true;
            }
        }
        if changed[k] {
            if let Some(c) = class.get_mut(k) {
                *c = Some(now);
            }
            if let Some(g) = ground.get_mut(k) {
                *g = now == GROUND;
            }
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pt(class: u8, r: u8, n: u8, angle: f32) -> Point {
        Point {
            class,
            return_number: r,
            number_of_returns: n,
            scan_angle: angle,
            ..Default::default()
        }
    }

    #[test]
    fn a_rule_takes_by_class_echo_and_absolute_angle() -> Result<()> {
        // first returns of several, at 25° or more to either side, to class 12
        let rule = ClassRule::parse("from=1,2;returns=F;angle=25..90;to=12")?;
        assert!(rule.takes(&pt(1, 1, 3, 30.0), 1));
        assert!(rule.takes(&pt(2, 1, 3, -30.0), 2));
        // the last of several, a single return, class 5, or a gentle angle: not taken
        assert!(!rule.takes(&pt(1, 3, 3, 30.0), 1));
        assert!(
            !rule.takes(&pt(1, 1, 1, 30.0), 1),
            "F is the first of several only"
        );
        assert!(ClassRule::parse("returns=SL;to=5")?.takes(&pt(1, 1, 1, 0.0), 1));
        assert!(!rule.takes(&pt(5, 1, 3, 30.0), 5));
        assert!(!rule.takes(&pt(1, 1, 3, 10.0), 1));
        // an angle with no upper limit
        assert!(ClassRule::parse("angle=25;to=1")?.takes(&pt(2, 1, 1, -80.0), 2));
        Ok(())
    }

    #[test]
    fn noise_is_left_alone_unless_the_rule_says_otherwise() -> Result<()> {
        let rule = ClassRule::new(1);
        assert!(!rule.takes(&pt(7, 1, 1, 0.0), 7) && !rule.takes(&pt(18, 1, 1, 0.0), 18));
        assert!(ClassRule::parse("from=7;to=1")?.takes(&pt(7, 1, 1, 0.0), 7));
        Ok(())
    }

    #[test]
    fn bad_rules_say_what_is_wrong() {
        for bad in ["from=1", "to=300", "angle=x;to=1", "colour=3;to=1", "to"] {
            assert!(
                matches!(ClassRule::parse(bad), Err(Error::Param(_))),
                "{bad}"
            );
        }
    }

    #[test]
    fn rules_apply_in_order_and_ground_follows() -> Result<()> {
        let pts = [
            pt(2, 1, 1, 40.0), // ground at a steep angle
            pt(2, 1, 1, 5.0),  // ground at a gentle angle
            pt(1, 1, 3, 5.0),  // first of several
            pt(7, 1, 1, 40.0), // noise
        ];
        let rules = [
            ClassRule::parse("angle=30;to=12")?,
            // sees what the rule before made: class 12 points that are first returns go to 5
            ClassRule::parse("from=12;returns=S;to=5")?,
            ClassRule::parse("from=1;returns=F;to=5")?,
        ];
        let mut class = vec![None; 4];
        let mut ground = vec![true, true, false, false];
        let changed = apply(&pts, &mut class, &mut ground, &rules);
        assert_eq!(class, [Some(5), None, Some(5), None]);
        assert_eq!(changed, [true, false, true, false]);
        // the steep ground point is no longer ground
        assert_eq!(ground, [false, true, false, false]);
        Ok(())
    }
}
