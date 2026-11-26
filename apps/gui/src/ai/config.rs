//! Los ajustes del asistente: el servicio activo con la clave y el modelo de cada uno, el perfil,
//! el estilo y el límite de las respuestas, y dónde se guardan las conversaciones. Se guardan en
//! `asistente.json` en la carpeta de configuración del usuario, solo legible por él.

use std::collections::BTreeMap;
use std::path::PathBuf;

use hipso_core::json::Json;

use super::jsonw::{arr, dump, n, num, obj, s, text};
use super::roles;

/// Un servicio de IA. Todos hablan la API de OpenAI (`/chat/completions` y `/models`); el de
/// Claude usa la capa de compatibilidad de Anthropic.
pub struct Provider {
    pub id: &'static str,
    pub name: &'static str,
    pub base: &'static str,
    pub model: &'static str,
    /// Dónde se saca la clave.
    pub keys_at: &'static str,
    /// Los modelos con límite de respuesta `max_completion_tokens` y sin temperatura.
    pub openai_style: bool,
}

pub const PROVIDERS: [Provider; 5] = [
    Provider {
        id: "deepseek",
        name: "DeepSeek",
        base: "https://api.deepseek.com",
        model: "deepseek-chat",
        keys_at: "platform.deepseek.com/api_keys",
        openai_style: false,
    },
    Provider {
        id: "openrouter",
        name: "OpenRouter",
        base: "https://openrouter.ai/api/v1",
        model: "openai/gpt-4o-mini",
        keys_at: "openrouter.ai/keys",
        openai_style: false,
    },
    Provider {
        id: "anthropic",
        name: "Claude (Anthropic)",
        base: "https://api.anthropic.com/v1",
        model: "claude-haiku-5-5",
        keys_at: "console.anthropic.com/settings/keys",
        openai_style: false,
    },
    Provider {
        id: "openai",
        name: "ChatGPT (OpenAI)",
        base: "https://api.openai.com/v1",
        model: "gpt-4o-mini",
        keys_at: "platform.openai.com/api-keys",
        openai_style: true,
    },
    Provider {
        id: "custom",
        name: "Otro servicio compatible con OpenAI",
        base: "",
        model: "",
        keys_at: "el panel de tu servicio",
        openai_style: false,
    },
];

pub fn provider(id: &str) -> &'static Provider {
    PROVIDERS.iter().find(|p| p.id == id).unwrap_or(&PROVIDERS[0])
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProviderCfg {
    pub key: String,
    pub model: String,
    /// La dirección base, solo para «otro servicio».
    pub base: String,
}

/// Cuánto debe extenderse el asistente.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Style {
    Brief,
    Normal,
    Detailed,
}

impl Style {
    pub const ALL: [Style; 3] = [Style::Brief, Style::Normal, Style::Detailed];

    pub fn id(self) -> &'static str {
        match self {
            Style::Brief => "breve",
            Style::Normal => "normal",
            Style::Detailed => "detallado",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Style::Brief => "Breve",
            Style::Normal => "Normal",
            Style::Detailed => "En profundidad",
        }
    }

    fn parse(t: &str) -> Style {
        Style::ALL.into_iter().find(|s| s.id() == t).unwrap_or(Style::Normal)
    }

    /// La instrucción de extensión que va en el prompt.
    pub fn prompt(self) -> &'static str {
        match self {
            Style::Brief => "Extensión: breve y directa (unas 150 palabras como mucho) salvo que se pida más; primero la respuesta y luego solo lo imprescindible.",
            Style::Normal => "Extensión: la necesaria; primero la respuesta y luego el detalle, con listas o tablas cortas.",
            Style::Detailed => "Extensión: en profundidad cuando se pida explicar: qué hace, cómo funciona, parámetros, cuándo usarlo y límites, con ejemplos; sin relleno.",
        }
    }
}

/// Dónde se guardan las conversaciones.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Storage {
    /// En la carpeta de configuración de HIPSO.
    App,
    /// En una carpeta que elige la persona (un disco externo, una carpeta sincronizada…).
    Folder,
    /// En ninguna parte: solo viven mientras el programa está abierto.
    Off,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub provider: String,
    pub providers: BTreeMap<String, ProviderCfg>,
    /// `none`, el identificador de un perfil o `custom`.
    pub role: String,
    pub custom_name: String,
    pub custom_text: String,
    pub style: Style,
    /// Tokens como mucho en cada respuesta.
    pub max_output: u32,
    /// Mensajes de la conversación que se reenvían en cada pregunta.
    pub history: usize,
    pub storage: Storage,
    pub folder: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            provider: PROVIDERS[0].id.to_string(),
            providers: BTreeMap::new(),
            role: roles::NONE.to_string(),
            custom_name: "Mi perfil".into(),
            custom_text: String::new(),
            style: Style::Normal,
            max_output: 1500,
            history: 8,
            storage: Storage::App,
            folder: String::new(),
        }
    }
}

/// Deja solo la clave: quita espacios, saltos de línea, comillas y prefijos pegados por error
/// («Bearer …», «Authorization: Bearer …»).
pub fn clean_key(k: &str) -> String {
    let mut t = k.trim().to_string();
    loop {
        let before = t.clone();
        t = t
            .trim_matches(|c: char| c.is_whitespace() || "\"'`“”‘’«»".contains(c))
            .to_string();
        let low = t.to_lowercase();
        for prefix in ["authorization:", "bearer:", "bearer "] {
            if low.starts_with(prefix) {
                t = t[prefix.len()..].to_string();
                break;
            }
        }
        if t == before {
            break;
        }
    }
    t.chars().filter(|c| !c.is_whitespace()).collect()
}

/// Si una dirección envía la clave sin cifrar fuera de este equipo: `http://` a otra máquina.
pub fn unencrypted(base: &str) -> bool {
    let Some(rest) = base.trim().to_ascii_lowercase().strip_prefix("http://").map(str::to_string) else {
        return false;
    };
    let host = rest.split('/').next().unwrap_or("");
    let host = host.rsplit_once('@').map_or(host, |h| h.1);
    let name = match host.strip_prefix('[') {
        Some(v6) => v6.split(']').next().unwrap_or(""),
        None => host.split(':').next().unwrap_or(""),
    };
    !(name == "localhost" || name == "::1" || name.parse::<std::net::Ipv4Addr>().is_ok_and(|a| a.is_loopback()))
}

impl Config {
    pub fn file() -> Option<PathBuf> {
        Some(crate::prefs::config_dir()?.join("asistente.json"))
    }

    pub fn load() -> Config {
        Self::file()
            .and_then(|f| std::fs::read_to_string(f).ok())
            .map(|t| Self::from_json(&t))
            .unwrap_or_default()
    }

    pub fn from_json(text_in: &str) -> Config {
        let mut c = Config::default();
        let Ok(j) = Json::parse(text_in) else { return c };
        let t = |k: &str| text(&j, k);
        if PROVIDERS.iter().any(|p| p.id == t("provider")) {
            c.provider = t("provider");
        }
        if let Some(Json::Obj(members)) = j.get("providers") {
            for (id, v) in members {
                c.providers.insert(
                    id.clone(),
                    ProviderCfg { key: clean_key(&text(v, "key")), model: text(v, "model"), base: text(v, "base") },
                );
            }
        }
        if !t("role").is_empty() {
            c.role = t("role");
        }
        if !t("custom_name").is_empty() {
            c.custom_name = t("custom_name");
        }
        c.custom_text = t("custom_text");
        c.style = Style::parse(&t("style"));
        if let Some(v) = num(&j, "max_output") {
            c.max_output = (v as u32).clamp(200, 16000);
        }
        if let Some(v) = num(&j, "history") {
            c.history = (v as usize).clamp(2, 40);
        }
        c.storage = match t("storage").as_str() {
            "folder" => Storage::Folder,
            "off" => Storage::Off,
            _ => Storage::App,
        };
        c.folder = t("folder");
        c
    }

    pub fn to_json(&self) -> String {
        let providers = Json::Obj(
            self.providers
                .iter()
                .map(|(id, p)| {
                    (id.clone(), obj(vec![("key", s(&p.key)), ("model", s(&p.model)), ("base", s(&p.base))]))
                })
                .collect(),
        );
        dump(&obj(vec![
            ("provider", s(&self.provider)),
            ("providers", providers),
            ("role", s(&self.role)),
            ("custom_name", s(&self.custom_name)),
            ("custom_text", s(&self.custom_text)),
            ("style", s(self.style.id())),
            ("max_output", n(f64::from(self.max_output))),
            ("history", n(self.history as f64)),
            (
                "storage",
                s(match self.storage {
                    Storage::App => "app",
                    Storage::Folder => "folder",
                    Storage::Off => "off",
                }),
            ),
            ("folder", s(&self.folder)),
            ("version", arr(vec![n(1.0)])),
        ]))
    }

    pub fn save(&self) -> std::io::Result<()> {
        let Some(f) = Self::file() else { return Ok(()) };
        if let Some(d) = f.parent() {
            std::fs::create_dir_all(d)?;
        }
        // se escribe en un archivo aparte y se renombra, de modo que un corte no deja la
        // configuración a medias
        let tmp = f.with_extension("json.tmp");
        std::fs::write(&tmp, self.to_json())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
        }
        std::fs::rename(tmp, f)
    }

    pub fn current(&self) -> ProviderCfg {
        self.providers.get(&self.provider).cloned().unwrap_or_default()
    }

    pub fn entry(&mut self, id: &str) -> &mut ProviderCfg {
        self.providers.entry(id.to_string()).or_default()
    }

    /// El modelo del servicio activo: el elegido o el de por defecto.
    pub fn model(&self) -> String {
        let m = self.current().model;
        if m.trim().is_empty() { provider(&self.provider).model.to_string() } else { m.trim().to_string() }
    }

    /// La dirección base del servicio activo.
    pub fn base(&self) -> String {
        let p = provider(&self.provider);
        if p.id == "custom" {
            self.current().base.trim().trim_end_matches('/').to_string()
        } else {
            p.base.to_string()
        }
    }

    pub fn has_key(&self) -> bool {
        !self.current().key.trim().is_empty()
    }

    /// El nombre del perfil elegido.
    pub fn role_name(&self) -> String {
        match self.role.as_str() {
            roles::NONE => "Sin rol".into(),
            roles::CUSTOM => self.custom_name.clone(),
            id => roles::find(id).map_or("Sin rol".into(), |r| r.name.to_string()),
        }
    }

    /// El texto del perfil elegido (vacío si no hay).
    pub fn role_text(&self) -> String {
        match self.role.as_str() {
            roles::NONE => String::new(),
            roles::CUSTOM => self.custom_text.trim().to_string(),
            id => roles::find(id).map_or(String::new(), |r| r.text.to_string()),
        }
    }

    /// La carpeta de las conversaciones, si se guardan.
    pub fn conversations_dir(&self) -> Option<PathBuf> {
        match self.storage {
            Storage::Off => None,
            Storage::App => Some(crate::prefs::config_dir()?.join("conversaciones")),
            Storage::Folder => {
                let f = self.folder.trim();
                (!f.is_empty()).then(|| PathBuf::from(f))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_lose_quotes_and_prefixes() {
        for dirty in ["sk-1", " sk-1\n", "\"sk-1\"", "Bearer sk-1", "bearer Bearer sk-1", "Authorization: Bearer sk-1", "“sk-1”", "sk-\n1"] {
            assert_eq!(clean_key(dirty), "sk-1", "{dirty:?}");
        }
    }

    #[test]
    fn settings_round_trip() {
        let mut c = Config::default();
        c.provider = "openai".into();
        c.entry("openai").key = "sk-x".into();
        c.entry("openai").model = "gpt-4o".into();
        c.role = "lidar_analyst".into();
        c.style = Style::Brief;
        c.max_output = 900;
        c.storage = Storage::Folder;
        c.folder = "/tmp/conv".into();
        c.custom_text = "texto \"propio\"\ncon salto".into();
        assert_eq!(Config::from_json(&c.to_json()), c);
        assert_eq!(c.model(), "gpt-4o");
        assert!(c.role_text().contains("LiDAR") || c.role_text().contains("LiDAR".to_lowercase().as_str()) || !c.role_text().is_empty());
    }

    #[test]
    fn defaults_have_no_role_and_the_app_folder() {
        let c = Config::default();
        assert_eq!(c.role_text(), "");
        assert_eq!(c.role_name(), "Sin rol");
        assert_eq!(c.model(), "deepseek-chat");
        assert_eq!(c.base(), "https://api.deepseek.com");
        assert!(!c.has_key());
        // un archivo roto da los valores por defecto
        assert_eq!(Config::from_json("{no"), Config::default());
    }
}
#[cfg(test)]
mod unencrypted_tests {
    use super::unencrypted;

    #[test]
    fn only_plain_http_to_another_machine_is_unencrypted() {
        for u in ["http://api.ejemplo.com/v1", "HTTP://192.168.1.4:8080", "http://localhost.ejemplo.com"] {
            assert!(unencrypted(u), "{u}");
        }
        for u in ["https://api.ejemplo.com", "http://localhost:11434/v1", "http://127.0.0.1/v1", "http://[::1]:80", ""] {
            assert!(!unencrypted(u), "{u}");
        }
    }
}
