// SPDX-FileCopyrightText: 2026 Vitor Marinho Faria
// SPDX-License-Identifier: GPL-3.0-or-later

//! Diálogo "Adicionar workspace": decide se o caminho informado registra um
//! `dbeasy.json` existente ou cria um novo.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Nome do arquivo criado quando o caminho informado é um diretório.
pub const FILE_NAME: &str = "dbeasy.json";

/// O que o botão do diálogo fará com o caminho informado.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// Arquivo existente a registrar.
    Existing(PathBuf),
    /// Arquivo a criar (e registrar); diretórios ausentes também são criados.
    Create(PathBuf),
    Invalid(String),
}

/// Interpreta o texto digitado: um arquivo existente é registrado; um
/// diretório usa o `dbeasy.json` dele, criando-o se não houver; um caminho
/// inexistente vira um arquivo novo (se terminar em `.json`) ou um diretório
/// novo com `dbeasy.json` dentro.
pub fn resolve(input: &str) -> Target {
    let raw = input.trim();
    if raw.is_empty() {
        return Target::Invalid("Informe o caminho de um dbeasy.json ou de um diretório.".into());
    }
    let path = match raw.strip_prefix("~/") {
        Some(rest) => dirs::home_dir().unwrap_or_default().join(rest),
        None => PathBuf::from(raw),
    };
    if path.is_file() {
        return Target::Existing(path);
    }
    let file = if path.is_dir() {
        let file = path.join(FILE_NAME);
        if file.is_file() {
            return Target::Existing(file);
        }
        file
    } else if path.extension().is_some_and(|e| e == "json") {
        path
    } else {
        path.join(FILE_NAME)
    };
    if !file.is_absolute() {
        return Target::Invalid("Para criar um dbeasy.json, informe um caminho absoluto.".into());
    }
    if file.exists() {
        return Target::Invalid(format!("{} existe e não é um arquivo.", file.display()));
    }
    Target::Create(file)
}

/// Cria `file` com uma única entrada, nomeada pelo diretório, para ser
/// completada no editor gráfico. Nunca sobrescreve um arquivo existente.
pub fn create(file: &Path) -> Result<(), String> {
    let dir = file.parent().unwrap_or(Path::new("/"));
    let name = dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "workspace".into());
    let entry = serde_json::json!([{ "name": name }]);
    let mut text = serde_json::to_string_pretty(&entry).expect("JSON serializável");
    text.push('\n');

    let fail = |e: std::io::Error| format!("não foi possível criar {}: {e}", file.display());
    fs::create_dir_all(dir).map_err(fail)?;
    let mut out = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(file)
        .map_err(fail)?;
    out.write_all(text.as_bytes()).map_err(fail)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn existing_file_or_directory_with_file_is_added() {
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join(FILE_NAME);
        fs::write(&file, "[]").unwrap();
        assert_eq!(
            resolve(&file.display().to_string()),
            Target::Existing(file.clone())
        );
        assert_eq!(
            resolve(&format!(" {} ", root.path().display())),
            Target::Existing(file)
        );
    }

    #[test]
    fn directory_without_file_or_new_path_is_created() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().display().to_string();
        assert_eq!(resolve(&dir), Target::Create(root.path().join(FILE_NAME)));

        let new_dir = root.path().join("novo").join("projeto");
        assert_eq!(
            resolve(&new_dir.display().to_string()),
            Target::Create(new_dir.join(FILE_NAME))
        );

        let new_file = root.path().join("outro").join("bancos.json");
        assert_eq!(
            resolve(&new_file.display().to_string()),
            Target::Create(new_file)
        );
    }

    #[test]
    fn empty_or_relative_new_path_is_invalid() {
        assert!(matches!(resolve("  "), Target::Invalid(_)));
        assert!(matches!(
            resolve("caminho/relativo/que/nao/existe"),
            Target::Invalid(_)
        ));
    }

    #[test]
    fn create_writes_named_entry_and_never_overwrites() {
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("meu-projeto").join(FILE_NAME);
        create(&file).unwrap();
        let entries = crate::workspace::config::load_file(&file).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name.as_deref(), Some("meu-projeto"));

        fs::write(&file, "[]").unwrap();
        assert!(create(&file).is_err());
        assert_eq!(fs::read_to_string(&file).unwrap(), "[]");
    }
}
