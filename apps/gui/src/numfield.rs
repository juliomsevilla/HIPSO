//! A number box you type in. It takes the place of egui's drag value: no dragging, no click to
//! start editing. The value is applied when editing ends (Enter or leaving the box), so a
//! half-typed number never reaches the program.

use std::ops::RangeInclusive;

use eframe::egui::{self, Response, TextEdit, Ui, Widget, emath::Numeric};

type Formatter<'a> = Box<dyn Fn(f64, RangeInclusive<usize>) -> String + 'a>;
type Parser<'a> = Box<dyn Fn(&str) -> Option<f64> + 'a>;

pub struct NumField<'a, N: Numeric> {
    value: &'a mut N,
    range: RangeInclusive<f64>,
    prefix: String,
    suffix: String,
    decimals: Option<usize>,
    formatter: Option<Formatter<'a>>,
    parser: Option<Parser<'a>>,
}

impl<'a, N: Numeric> NumField<'a, N> {
    pub fn new(value: &'a mut N) -> Self {
        Self {
            value,
            range: f64::NEG_INFINITY..=f64::INFINITY,
            prefix: String::new(),
            suffix: String::new(),
            decimals: None,
            formatter: None,
            parser: None,
        }
    }

    pub fn range<R: Into<f64> + Copy>(mut self, r: RangeInclusive<R>) -> Self {
        self.range = (*r.start()).into()..=(*r.end()).into();
        self
    }

    /// Kept so the calls written for a drag value still read the same; typing has no speed.
    pub fn speed(self, _speed: impl Into<f64>) -> Self {
        self
    }

    pub fn prefix(mut self, p: impl ToString) -> Self {
        self.prefix = p.to_string();
        self
    }

    pub fn suffix(mut self, s: impl ToString) -> Self {
        self.suffix = s.to_string();
        self
    }

    pub fn fixed_decimals(mut self, n: usize) -> Self {
        self.decimals = Some(n);
        self
    }

    pub fn custom_formatter(
        mut self,
        f: impl Fn(f64, RangeInclusive<usize>) -> String + 'a,
    ) -> Self {
        self.formatter = Some(Box::new(f));
        self
    }

    pub fn custom_parser(mut self, f: impl Fn(&str) -> Option<f64> + 'a) -> Self {
        self.parser = Some(Box::new(f));
        self
    }

    fn show(&self, v: f64) -> String {
        if let Some(f) = &self.formatter {
            return f(v, 0..=6);
        }
        let body = if N::INTEGRAL {
            format!("{}", v.round() as i64)
        } else if let Some(d) = self.decimals {
            format!("{v:.d$}")
        } else {
            let s = format!("{v:.4}");
            s.trim_end_matches('0').trim_end_matches('.').to_string()
        };
        format!("{}{body}{}", self.prefix, self.suffix)
    }

    fn parse(&self, text: &str) -> Option<f64> {
        if let Some(p) = &self.parser {
            return p(text);
        }
        text.trim()
            .trim_start_matches(self.prefix.as_str())
            .trim_end_matches(self.suffix.as_str())
            .trim()
            .replace(',', ".")
            .parse::<f64>()
            .ok()
    }
}

impl<N: Numeric> Widget for NumField<'_, N> {
    fn ui(self, ui: &mut Ui) -> Response {
        let id = ui.next_auto_id();
        let key = egui::Id::new((id, "numfield"));
        let current = self.value.to_f64();
        let mut text = ui
            .data(|d| d.get_temp::<String>(key))
            .unwrap_or_else(|| self.show(current));
        let mut r = ui.add(
            TextEdit::singleline(&mut text)
                .id(id)
                .desired_width(60.0)
                .horizontal_align(egui::Align::RIGHT),
        );
        if r.has_focus() {
            ui.data_mut(|d| d.insert_temp(key, text.clone()));
        } else {
            ui.data_mut(|d| d.remove_temp::<String>(key));
        }
        if r.lost_focus()
            && let Some(v) = self.parse(&text).filter(|v| v.is_finite())
        {
            let v = v.clamp(*self.range.start(), *self.range.end());
            if (v - current).abs() > f64::EPSILON {
                *self.value = N::from_f64(v);
                r.mark_changed();
            }
        }
        r
    }
}
