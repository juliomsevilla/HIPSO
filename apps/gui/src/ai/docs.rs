//! La documentación que el asistente consulta en lugar de llevarla en la cabeza: el manual y la
//! referencia de las órdenes, troceados por apartados, con una búsqueda por palabras. El asistente
//! pide solo el apartado que necesita, de modo que cada pregunta cuesta unos cientos de tokens y no
//! decenas de miles.

use std::collections::HashMap;
use std::sync::OnceLock;

const CLI: &str = include_str!("../../../../docs/CLI.md");

/// Un apartado: desde su encabezado hasta el siguiente (de cualquier nivel).
pub struct Section {
    pub id: String,
    pub level: u8,
    pub title: String,
    /// Los títulos de los apartados que lo contienen, y el suyo.
    pub trail: String,
    pub body: String,
    tokens_title: Vec<String>,
    tokens_trail: Vec<String>,
    tf: HashMap<String, u16>,
}

pub struct Docs {
    pub manual: Vec<Section>,
    pub cli: Vec<Section>,
    idf: HashMap<String, f32>,
}

const STOP: [&str; 38] = [
    "que", "para", "con", "los", "las", "una", "uno", "por", "del", "como", "cual", "cuales", "esta",
    "este", "esto", "son", "hay", "sus", "mas", "pero", "sin", "sobre", "entre", "desde", "hace",
    "hacer", "puedo", "puede", "quiero", "dime", "explica", "explicame", "sirve", "usar", "uso",
    "herramienta", "hipso", "programa",
];

/// Minúsculas y sin acentos.
pub fn fold(s: &str) -> String {
    s.chars()
        .flat_map(|c| c.to_lowercase())
        .map(|c| match c {
            'á' | 'à' | 'ä' | 'â' => 'a',
            'é' | 'è' | 'ë' | 'ê' => 'e',
            'í' | 'ì' | 'ï' | 'î' => 'i',
            'ó' | 'ò' | 'ö' | 'ô' => 'o',
            'ú' | 'ù' | 'ü' | 'û' => 'u',
            c => c,
        })
        .collect()
}

/// Las palabras de un texto, sin acentos y con el plural quitado.
pub fn tokens(s: &str) -> Vec<String> {
    fold(s)
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.chars().count() >= 3 && !STOP.contains(w))
        .map(|w| {
            let w = w.strip_suffix("es").filter(|r| r.len() >= 4).unwrap_or(w);
            w.strip_suffix('s').filter(|r| r.len() >= 3).unwrap_or(w).to_string()
        })
        .collect()
}

type Pending = Option<(u8, String, String, String)>;

fn flush(cur: &mut Pending, out: &mut Vec<Section>, prefix: &str) {
    if let Some((level, title, trail, body)) = cur.take() {
        let mut tf: HashMap<String, u16> = HashMap::new();
        for t in tokens(&body) {
            *tf.entry(t).or_insert(0) += 1;
        }
        out.push(Section {
            id: format!("{prefix}{}", out.len()),
            level,
            tokens_title: tokens(&title),
            tokens_trail: tokens(&trail),
            title,
            trail,
            body: body.trim().to_string(),
            tf,
        });
    }
}

fn parse(text: &str, prefix: &str) -> Vec<Section> {
    let mut out: Vec<Section> = Vec::new();
    let mut stack: Vec<(u8, String)> = Vec::new();
    let mut in_code = false;
    let mut cur: Pending = None;
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            in_code = !in_code;
        }
        let level = if in_code { 0 } else { line.chars().take_while(|c| *c == '#').count() };
        if (1..=5).contains(&level) && line[level..].starts_with(' ') {
            flush(&mut cur, &mut out, prefix);
            let title = line[level..].trim().to_string();
            while stack.last().is_some_and(|(l, _)| usize::from(*l) >= level) {
                stack.pop();
            }
            stack.push((level as u8, title.clone()));
            let trail = stack.iter().map(|(_, t)| t.as_str()).collect::<Vec<_>>().join(" › ");
            cur = Some((level as u8, title, trail, String::new()));
        } else if let Some(c) = cur.as_mut() {
            c.3.push_str(line);
            c.3.push('\n');
        }
    }
    flush(&mut cur, &mut out, prefix);
    out
}

pub fn docs() -> &'static Docs {
    static DOCS: OnceLock<Docs> = OnceLock::new();
    DOCS.get_or_init(|| {
        let manual = parse(crate::manual::TEXT, "m");
        let cli = parse(CLI, "c");
        let mut df: HashMap<String, u32> = HashMap::new();
        for s in manual.iter().chain(&cli) {
            for t in s.tf.keys() {
                *df.entry(t.clone()).or_insert(0) += 1;
            }
        }
        let n = (manual.len() + cli.len()) as f32;
        let idf = df.into_iter().map(|(t, d)| (t, (1.0 + n / (1.0 + d as f32)).ln())).collect();
        Docs { manual, cli, idf }
    })
}

impl Docs {
    pub fn section(&self, id: &str) -> Option<&Section> {
        let (kind, k) = id.split_at_checked(1)?;
        let list = match kind {
            "m" => &self.manual,
            "c" => &self.cli,
            _ => return None,
        };
        list.get(k.parse::<usize>().ok()?)
    }

    fn score(&self, s: &Section, q: &[String]) -> f32 {
        let mut total = 0.0;
        for t in q {
            let idf = self.idf.get(t).copied().unwrap_or(2.0);
            let in_title = s.tokens_title.iter().filter(|x| *x == t).count() as f32;
            let in_trail = s.tokens_trail.iter().filter(|x| *x == t).count() as f32;
            let in_body = f32::from(s.tf.get(t).copied().unwrap_or(0)).min(6.0);
            total += idf * (4.0 * in_title + in_trail + in_body);
        }
        // un apartado larguísimo no gana por tener de todo
        total / (1.0 + (1.0 + s.body.len() as f32 / 2500.0).ln())
    }

    /// Los `n` apartados del manual que más se parecen a la consulta.
    pub fn search(&self, query: &str, n: usize) -> Vec<&Section> {
        let q = tokens(query);
        if q.is_empty() {
            return Vec::new();
        }
        let mut hits: Vec<(f32, &Section)> = self
            .manual
            .iter()
            .filter(|s| s.level >= 2 && s.body.len() > 40)
            .map(|s| (self.score(s, &q), s))
            .filter(|(sc, _)| *sc > 0.0)
            .collect();
        hits.sort_by(|a, b| b.0.total_cmp(&a.0));
        hits.into_iter().take(n).map(|(_, s)| s).collect()
    }

    /// Los subapartados directos de un apartado.
    pub fn children<'a>(&'a self, s: &Section) -> Vec<&'a Section> {
        let list = if s.id.starts_with('m') { &self.manual } else { &self.cli };
        let Some(pos) = list.iter().position(|x| x.id == s.id) else { return Vec::new() };
        list[pos + 1..].iter().take_while(|x| x.level > s.level).filter(|x| x.level == s.level + 1).collect()
    }

    /// La sección de la referencia de una orden.
    pub fn tool(&self, name: &str) -> Option<&Section> {
        let want = format!("hipso {}", name.trim().trim_start_matches("hipso ").trim());
        self.cli.iter().find(|s| s.level == 3 && s.title == want)
    }
}

/// El extracto de un apartado más cercano a la consulta: el párrafo con más palabras suyas.
pub fn excerpt(body: &str, query: &str, max: usize) -> String {
    let q = tokens(query);
    let best = body
        .split("\n\n")
        .filter(|p| !p.trim().is_empty() && !p.trim_start().starts_with('|'))
        .max_by_key(|p| {
            let t = tokens(p);
            q.iter().filter(|w| t.contains(w)).count()
        })
        .unwrap_or(body);
    cut(best.trim(), max)
}

/// Como mucho `max` caracteres, cortados en un límite de carácter.
pub fn cut(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max).collect();
    out.push('…');
    out
}

/// Una tabla de opciones de la referencia de órdenes en líneas cortas: `--cell (número, por
/// defecto 1): qué hace`.
pub fn compact_options(body: &str) -> String {
    let mut out = String::new();
    for line in body.lines() {
        let t = line.trim();
        if !t.starts_with('|') {
            out.push_str(line);
            out.push('\n');
            continue;
        }
        let cells: Vec<&str> = t.trim_matches('|').split(" | ").map(str::trim).collect();
        if cells.len() < 4 || cells[0].starts_with("Opción") || cells[0].starts_with("---") {
            continue;
        }
        let def = cells[2].trim_matches('`');
        out.push_str(&format!(
            "- {} ({}{}): {}\n",
            cells[0].replace('`', ""),
            cells[1],
            if def == "ninguno" || def == "desactivada" { String::new() } else { format!(", por defecto {def}") },
            cells[3..].join(" | ")
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_manual_and_the_reference_are_split_into_sections() {
        let d = docs();
        assert!(d.manual.len() > 300, "{}", d.manual.len());
        assert!(d.cli.len() > 80, "{}", d.cli.len());
        assert!(d.tool("chm").is_some());
        assert!(d.tool("hipso dtm-validate").is_some());
        assert!(d.tool("nada").is_none());
    }

    #[test]
    fn a_search_finds_the_section_that_explains_the_tool() {
        let d = docs();
        let t = d.search("validar un MDT con puntos de control", 3);
        assert!(t.iter().any(|s| s.title.contains("Validar")), "{:?}", t.iter().map(|s| &s.trail).collect::<Vec<_>>());
        let v = d.search("densidad de Voronoi", 2);
        assert!(v.iter().any(|s| s.title.contains("Voronoi")));
        let tr = d.search("segmentar árboles con puntos Li", 3);
        assert!(tr.iter().any(|s| s.title.contains("Segmentar con puntos")), "{:?}", tr.iter().map(|s| &s.trail).collect::<Vec<_>>());
    }

    #[test]
    fn sections_know_their_ids() {
        let d = docs();
        let hit = d.search("flujos constructor nodos", 1);
        let id = hit.first().map(|s| s.id.clone()).unwrap_or_default();
        assert!(!id.is_empty());
        assert_eq!(d.section(&id).map(|x| x.id.clone()), Some(id));
        assert!(d.section("x9").is_none() && d.section("m99999").is_none());
    }

    #[test]
    fn options_become_short_lines() {
        let body = "Texto\n\n| Opción | Valor | Por defecto | Qué hace |\n|---|---|---|---|\n| `--cell` | número | `1` | Celda. |\n| `-o`, `--output` | archivo | ninguno | Salida. |\n";
        let c = compact_options(body);
        assert!(c.contains("- --cell (número, por defecto 1): Celda."), "{c}");
        assert!(c.contains("- -o, --output (archivo): Salida."), "{c}");
    }

    #[test]
    fn folding_and_stemming() {
        assert_eq!(tokens("Clasificación de los árboles"), vec!["clasificacion", "arbol"]);
        assert_eq!(fold("ÁÉÍÓÚ ü"), "aeiou u");
    }
}
