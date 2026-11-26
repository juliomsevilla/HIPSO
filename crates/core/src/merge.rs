//! Unir dos o más nubes en una, con una decisión sobre los puntos duplicados que quedan al unir.

use std::path::{Path, PathBuf};

use crate::cloudtools::{self, DupFields};
use crate::{Error, Result};

/// Qué hacer con los puntos duplicados de la nube unida.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DupAction {
    /// No buscarlos: la nube unida los conserva.
    Keep,
    /// Marcar como retenidos (withheld) todos menos uno de cada conjunto.
    Mark,
    /// Quitarlos, dejando un punto de cada conjunto.
    Delete,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MergeReport {
    pub points: u64,
    /// Duplicados marcados o eliminados (0 con `DupAction::Keep`).
    pub duplicates: u64,
    pub notes: Vec<String>,
}

/// Une `inputs` (dos o más) en `output`. Las nubes pueden tener otra escala, otro desplazamiento u
/// otro sistema de coordenadas; el formato de punto debe ser el mismo.
/// ponytail: duplicados solo exactos (mismo XYZ del registro), sin tolerancia.
pub fn merge(
    inputs: &[PathBuf],
    output: &Path,
    dups: DupAction,
    fields: DupFields,
) -> Result<MergeReport> {
    if inputs.len() < 2 {
        return Err(Error::Param("hacen falta dos o más nubes para unir".into()));
    }
    if inputs.iter().any(|p| crate::paths::same_file(p, output)) {
        return Err(Error::Param(
            "el archivo nuevo no debe sobrescribir una de las nubes".into(),
        ));
    }
    let ext = output
        .extension()
        .map_or("laz".into(), |e| e.to_string_lossy().into_owned());
    let union = if dups == DupAction::Keep {
        output.to_path_buf()
    } else {
        output.with_extension(format!("union-tmp.{ext}"))
    };
    let job = crate::ops::whole_cloud_job(inputs.to_vec(), union.clone())?;
    let (moved, unified) = crate::clip::unified(&job)?;
    let run = crate::clip::run(&moved);
    let report = match run {
        Ok(r) => r,
        Err(e) => {
            let _ = std::fs::remove_file(&union);
            return Err(e);
        }
    };
    let mut out = MergeReport {
        points: report.points_written,
        duplicates: 0,
        notes: unified.warnings.clone(),
    };
    if dups != DupAction::Keep {
        let r = if dups == DupAction::Mark {
            cloudtools::mark_duplicates(&union, output, fields)
        } else {
            cloudtools::remove_duplicates(&union, output, fields, false)
        };
        let _ = std::fs::remove_file(&union);
        let (read, n) = r?;
        out.duplicates = n;
        if dups == DupAction::Delete {
            out.points = read - n;
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn needs_two_or_more_clouds() {
        let one = [PathBuf::from("a.laz")];
        let out = Path::new("o.laz");
        assert!(merge(&[], out, DupAction::Keep, DupFields::default()).is_err());
        assert!(merge(&one, out, DupAction::Keep, DupFields::default()).is_err());
    }
}
