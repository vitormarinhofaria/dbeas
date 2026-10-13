// SPDX-FileCopyrightText: 2026 Vitor Marinho Faria
// SPDX-License-Identifier: GPL-3.0-or-later

//! Workspaces: carregamento dos `dbeas.json` registrados, identificação e
//! nomes exibidos (spec §3, §4.1).

pub mod config;
pub mod env;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub use config::{ConfigError, ResolvedConfig, SslMode};

/// Identificador estável de um workspace entre execuções: caminho do arquivo +
/// `name` (ou índice, quando a entrada não tem nome). Usado como chave da sessão.
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub struct WorkspaceId(pub String);

/// Uma entrada de workspace como aparece na barra lateral, válida ou com erro.
#[derive(Debug, Clone)]
pub struct Descriptor {
    pub id: WorkspaceId,
    pub file: PathBuf,
    pub display_name: String,
    pub config: Result<ResolvedConfig, ConfigError>,
}

/// Carrega todos os arquivos registrados. Um arquivo ausente ou inválido gera
/// um único descritor com erro, sem impedir os demais (spec §3.4).
pub fn load_all(files: &[PathBuf]) -> Vec<Descriptor> {
    let mut out = Vec::new();
    let mut base_names = Vec::new();

    for file in files {
        match config::load_file(file) {
            Ok(entries) => {
                let mut seen: HashMap<String, usize> = HashMap::new();
                for entry in entries {
                    let label = entry
                        .name
                        .clone()
                        .unwrap_or_else(|| format!("entrada #{}", entry.index + 1));
                    let count = seen.entry(label.clone()).or_insert(0);
                    *count += 1;
                    let key = if *count == 1 {
                        label.clone()
                    } else {
                        format!("{label}#{count}")
                    };
                    out.push(Descriptor {
                        id: WorkspaceId(format!("{}#{}", file.display(), key)),
                        file: file.clone(),
                        display_name: label.clone(),
                        config: entry.result,
                    });
                    base_names.push(label);
                }
            }
            Err(error) => {
                let label = file
                    .parent()
                    .and_then(Path::file_name)
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| file.display().to_string());
                out.push(Descriptor {
                    id: WorkspaceId(format!("{}#", file.display())),
                    file: file.clone(),
                    display_name: label.clone(),
                    config: Err(error),
                });
                base_names.push(label);
            }
        }
    }

    disambiguate(&mut out, &base_names);
    out
}

/// Nomes repetidos recebem o nome do repositório Git (ou do diretório pai)
/// entre parênteses (spec §4.1).
fn disambiguate(descriptors: &mut [Descriptor], base_names: &[String]) {
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for name in base_names {
        *counts.entry(name.as_str()).or_insert(0) += 1;
    }
    for (desc, base) in descriptors.iter_mut().zip(base_names) {
        if counts[base.as_str()] > 1 {
            desc.display_name = format!("{base} ({})", origin_label(&desc.file));
        }
    }
}

/// Nome do repositório Git que contém o arquivo, ou o nome do diretório pai.
pub fn origin_label(file: &Path) -> String {
    let parent = file.parent().unwrap_or(file);
    let mut dir = Some(parent);
    while let Some(d) = dir {
        if d.join(".git").exists() {
            if let Some(name) = d.file_name() {
                return name.to_string_lossy().into_owned();
            }
        }
        dir = d.parent();
    }
    parent
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| parent.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn duplicate_names_get_repository_suffix() {
        let root = tempfile::tempdir().unwrap();
        let a = root.path().join("repo-a");
        let b = root.path().join("repo-b").join("conf");
        fs::create_dir_all(a.join(".git")).unwrap();
        fs::create_dir_all(&b).unwrap();
        let json = r#"[{"name":"Dev","dbName":"x","dbUser":"u"},{"name":"Prod","dbName":"x","dbUser":"u"}]"#;
        fs::write(a.join("dbeas.json"), json).unwrap();
        fs::write(
            b.join("dbeas.json"),
            r#"[{"name":"Dev","dbName":"y","dbUser":"u"}]"#,
        )
        .unwrap();

        let all = load_all(&[a.join("dbeas.json"), b.join("dbeas.json")]);
        let names: Vec<_> = all.iter().map(|d| d.display_name.as_str()).collect();
        assert_eq!(names, ["Dev (repo-a)", "Prod", "Dev (conf)"]);
    }

    #[test]
    fn missing_file_yields_error_descriptor() {
        let all = load_all(&[PathBuf::from("/nao/existe/dbeas.json")]);
        assert_eq!(all.len(), 1);
        assert!(matches!(all[0].config, Err(ConfigError::Read { .. })));
    }

    #[test]
    fn ids_are_unique_for_duplicate_names_in_same_file() {
        let root = tempfile::tempdir().unwrap();
        let f = root.path().join("dbeas.json");
        fs::write(
            &f,
            r#"[{"name":"A","dbName":"x","dbUser":"u"},{"name":"A","dbName":"y","dbUser":"u"}]"#,
        )
        .unwrap();
        let all = load_all(&[f]);
        assert_ne!(all[0].id, all[1].id);
    }
}
