//! Las órdenes que hacen posibles los flujos: `schema` escribe lo que admite cada orden, para que
//! un constructor de flujos pueda dibujar sus opciones, y `run` y `model-validate` trabajan sobre
//! un flujo guardado.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use clap::CommandFactory;
use hipso_core::model::{ArgSpec, Model, Run, Schema, State, ToolSpec};
use hipso_core::{Error, Result};

/// Órdenes que tratan de los propios flujos, no herramientas que meter en uno.
const SKIP: [&str; 5] = ["schema", "mcp", "run", "model-validate", "help"];

/// Las opciones de cada orden, leídas de la definición que usa el analizador.
pub fn schema() -> Schema {
    let cmd = crate::Cli::command();
    let mut tools = Vec::new();
    for sub in cmd.get_subcommands() {
        if SKIP.contains(&sub.get_name()) {
            continue;
        }
        let mut spec = ToolSpec {
            name: sub.get_name().to_string(),
            about: sub.get_about().map(|a| a.to_string()).unwrap_or_default(),
            args: Vec::new(),
        };
        for a in sub.get_arguments() {
            let id = a.get_id().as_str();
            if id == "help" || id == "version" || id == "force" {
                continue;
            }
            let action = a.get_action();
            let flag = !action.takes_values();
            spec.args.push(ArgSpec {
                id: id.to_string(),
                long: a.get_long().map(str::to_string),
                short: a.get_short(),
                help: a.get_help().map(|h| h.to_string()).unwrap_or_default(),
                positional: a.is_positional(),
                flag,
                multiple: a.get_num_args().is_some_and(|n| n.max_values() > 1)
                    || matches!(action, clap::ArgAction::Append),
                required: a.is_required_set(),
                // los rásteres de la calculadora se dan como `NOMBRE=archivo`, que es un texto
                // y el diseño de `cutfill` es un ráster o un texto como `level:500`
                path: a.get_value_parser().type_id() == std::any::TypeId::of::<PathBuf>()
                    || (sub.get_name() == "calc" && id == "inputs")
                    || (sub.get_name() == "cutfill" && id == "design"),
                values: a
                    .get_possible_values()
                    .iter()
                    .filter(|v| !v.is_hide_set())
                    .map(|v| v.get_name().to_string())
                    .collect(),
                default: a
                    .get_default_values()
                    .first()
                    .map(|d| d.to_string_lossy().into_owned()),
            });
        }
        tools.push(spec);
    }
    Schema { tools }
}

fn load(path: &Path) -> Result<Model> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| Error::Param(format!("no se lee el flujo «{}»: {e}", path.display())))?;
    Model::from_json(&text)
}

/// `hipso model-validate`: imprime los problemas de un flujo; el código de salida indica si se
/// puede ejecutar.
pub fn validate(path: &Path) -> Result<()> {
    let m = load(path)?;
    let problems = m.problems(&schema());
    for w in m.warnings() {
        println!("Aviso: {}", w.text);
    }
    for n in &m.nodes {
        if let hipso_core::model::NodeKind::Input { path } = &n.kind {
            println!("Entrada «{}» (--input): {path}", m.name(n));
        }
    }
    for name in m.parameter_names() {
        let targets: Vec<String> = m
            .parameter_targets(name)
            .map(|(n, a)| format!("{}.{a}", m.name(n)))
            .collect();
        let value = m
            .parameter_targets(name)
            .next()
            .and_then(|(n, a)| m.value(&n.id, a))
            .unwrap_or("(por defecto)");
        println!("Parámetro «{name}» (--set): {value}; en {}", targets.join(", "));
    }
    if problems.is_empty() {
        println!("El flujo está bien: {} nodos y {} hilos.", m.nodes.len(), m.edges.len());
        return Ok(());
    }
    for p in &problems {
        println!("- {}", p.text);
    }
    Err(Error::Param(format!("{} problemas", problems.len())))
}

/// Cómo ejecutar un flujo.
pub struct Options {
    /// Sustituciones de entradas, `nombre=ruta`.
    pub inputs: Vec<String>,
    /// Valores de parámetros, `nombre=valor`.
    pub set: Vec<String>,
    /// El nodo hasta el que ejecutar.
    pub until: Option<String>,
    pub resume: bool,
    pub jobs: usize,
}

fn pair<'a>(option: &str, text: &'a str) -> Result<(&'a str, &'a str)> {
    text.split_once('=')
        .map(|(k, v)| (k.trim(), v.trim()))
        .ok_or_else(|| Error::Param(format!("{option} pide nombre=valor, no «{text}»")))
}

/// `hipso run`: ejecuta un flujo con este mismo ejecutable.
pub fn run(path: &Path, options: &Options, results: Option<PathBuf>) -> Result<()> {
    let mut m = load(path)?;
    let schema = schema();
    for s in &options.set {
        let (k, v) = pair("--set", s)?;
        m.set_parameter(&schema, k, v)?;
    }
    if let Some(target) = &options.until {
        m = m.upto(target)?;
    }
    let mut replace = HashMap::new();
    for i in &options.inputs {
        let (k, v) = pair("--input", i)?;
        replace.insert(k.to_string(), v.to_string());
    }
    let results = results
        .or_else(|| (!m.results.is_empty()).then(|| PathBuf::from(&m.results)))
        .unwrap_or_else(|| {
            path.parent()
                .unwrap_or_else(|| Path::new("."))
                .join(format!(
                    "{}_resultados",
                    path.file_stem().map_or("flujo".into(), |s| s.to_string_lossy().into_owned())
                ))
        });
    let exe = std::env::current_exe()?;
    let cancel = AtomicBool::new(false);
    let run = Run {
        exe: &exe,
        schema: &schema,
        results: &results,
        inputs: &replace,
        cancel: &cancel,
        resume: options.resume,
        jobs: options.jobs,
    };
    let names: HashMap<String, String> = m.nodes.iter().map(|n| (n.id.clone(), m.name(n))).collect();
    let report = m.run(
        &run,
        &mut |id, st| {
            let word = match st {
                State::Waiting => return,
                State::Running => "en curso",
                State::Done => "hecho",
                State::Failed => "falló",
            };
            eprintln!("[{}] {word}", names.get(id).map_or(id, |s| s.as_str()));
        },
        &mut |l| eprintln!("  {l}"),
    )?;
    for (id, files) in &report.outputs {
        println!("{}: {} archivos", names.get(id).map_or(id.as_str(), |s| s.as_str()), files.len());
        for f in files.iter().take(20) {
            println!("  {}", f.display());
        }
    }
    println!("resultados en {}", report.folder.display());
    if report.failed > 0 {
        return Err(Error::Param(format!(
            "el flujo ha terminado con {} fallos (véase {}); corregidos, se completa con --resume. El primero: {}",
            report.failed,
            report.folder.join(hipso_core::flowrun::ERRORS).display(),
            report.first_error
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use hipso_core::model::{CATALOG, entry};

    /// Como `super::schema`, en un hilo con la pila grande que usa `main`: la definición de las
    /// órdenes es tan grande que desborda la pila de 2 MB de un hilo de prueba.
    fn schema() -> Schema {
        std::thread::Builder::new()
            .stack_size(64 << 20)
            .spawn(super::schema)
            .and_then(|h| h.join().map_err(|_| std::io::Error::other("el hilo falló")))
            .expect("schema")
    }

    #[test]
    fn every_command_is_in_the_catalog_and_every_entry_is_a_command() {
        let s = schema();
        for t in &s.tools {
            assert!(entry(&t.name).is_some(), "falta «{}» en el catálogo de flujos", t.name);
        }
        for e in CATALOG {
            assert!(s.tool(e.tool).is_some(), "«{}» está en el catálogo y no es una orden", e.tool);
        }
    }

    #[test]
    fn the_schema_reads_back_and_knows_files_and_flags() -> Result<()> {
        let s = schema();
        assert_eq!(Schema::from_json(&s.to_json())?, s);
        let chm = s.tool("chm").ok_or_else(|| Error::Param("sin chm".into()))?;
        assert!(chm.arg("inputs").is_some_and(|a| a.positional && a.path && a.multiple));
        assert!(chm.arg("cell").is_some_and(|a| !a.path && !a.flag && a.default.is_some()));
        assert!(chm.output_arg().is_some());
        // una opción de tipo bandera no lleva valor
        let calc = s.tool("calc").ok_or_else(|| Error::Param("sin calc".into()))?;
        assert!(calc.arg("nearest").is_some_and(|a| a.flag));
        assert!(calc.arg("inputs").is_some_and(|a| a.path));
        Ok(())
    }

    /// Toda opción de archivo de toda orden que el catálogo declara como entrada tiene un tipo que
    /// se distingue del resto, y las opciones nombradas en la tabla existen.
    #[test]
    fn the_kinds_of_the_options_name_options_that_exist() {
        let s = schema();
        for (tool, arg) in hipso_core::model::named_args() {
            let t = s.tool(tool);
            assert!(t.is_some_and(|t| t.arg(arg).is_some()), "{tool} no tiene la opción {arg}");
        }
    }
}
