//! The classes the user has added to the ASPRS table (codes 23–255): name and colour, kept in
//! a small text file so they are there the next time.

use std::collections::BTreeMap;
use std::path::PathBuf;

use rustlidar_core::classes;

#[derive(Default)]
pub struct ClassTable {
    /// code → (name, colour)
    pub custom: BTreeMap<u8, (String, [u8; 3])>,
    /// The new class being typed in the window.
    pub draft: (String, String),
}

fn file() -> Option<PathBuf> {
    let base = std::env::var_os("APPDATA")
        .or_else(|| std::env::var_os("XDG_CONFIG_HOME"))
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("EasyLiDAR").join("clases.txt"))
}

impl ClassTable {
    pub fn load() -> Self {
        let mut t = Self::default();
        let text = file()
            .and_then(|f| std::fs::read_to_string(f).ok())
            .unwrap_or_default();
        for line in text.lines() {
            let mut it = line.split(';');
            let (Some(code), Some(name)) = (it.next(), it.next()) else {
                continue;
            };
            let rgb: Vec<u8> = it.filter_map(|v| v.trim().parse().ok()).collect();
            if let (Ok(code), [r, g, b]) = (code.trim().parse::<u8>(), rgb.as_slice()) {
                t.custom
                    .insert(code, (name.trim().to_string(), [*r, *g, *b]));
            }
        }
        t
    }

    pub fn save(&self) {
        let Some(f) = file() else { return };
        let text: String = self
            .custom
            .iter()
            .map(|(c, (n, k))| format!("{c};{};{};{};{}\n", n.replace(';', ","), k[0], k[1], k[2]))
            .collect();
        if let Some(d) = f.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let _ = std::fs::write(f, text);
    }

    pub fn name(&self, c: u8) -> String {
        self.custom
            .get(&c)
            .map_or_else(|| classes::name(c).to_string(), |(n, _)| n.clone())
    }

    /// Adds or renames a class; its colour goes to `colors`. Err: why it was refused.
    pub fn add(
        &mut self,
        code: &str,
        name: &str,
        colors: &mut [[u8; 3]; 256],
    ) -> Result<u8, String> {
        let c: u8 = code
            .trim()
            .parse()
            .map_err(|_| "El código debe ser un número de 0 a 255.".to_string())?;
        if classes::DEFINED.contains(&c) {
            return Err(format!(
                "La clase {c} ya está definida por ASPRS ({}).",
                classes::name(c)
            ));
        }
        let name = name.trim();
        if name.is_empty() {
            return Err("Escribe un nombre para la clase.".into());
        }
        let color = colors[usize::from(c)];
        self.custom.insert(c, (name.to_string(), color));
        self.save();
        Ok(c)
    }

    pub fn remove(&mut self, c: u8, colors: &mut [[u8; 3]; 256]) {
        self.custom.remove(&c);
        colors[usize::from(c)] = classes::default_color(c);
        self.save();
    }

    /// Applies the saved colours.
    pub fn paint(&self, colors: &mut [[u8; 3]; 256]) {
        for (c, (_, k)) in &self.custom {
            colors[usize::from(*c)] = *k;
        }
    }

    /// Codes to offer: the ASPRS ones, the user's, and those in `present`.
    pub fn known(&self, present: &[u64; 256]) -> Vec<u8> {
        (0..=255u8)
            .filter(|c| {
                classes::DEFINED.contains(c)
                    || self.custom.contains_key(c)
                    || present[usize::from(*c)] > 0
            })
            .collect()
    }
}
