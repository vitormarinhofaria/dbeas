// SPDX-FileCopyrightText: 2026 Vitor Marinho Faria
// SPDX-License-Identifier: GPL-3.0-or-later

//! Configuração global: registro dos arquivos `dbeasy.json` (spec §3.4).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::fsutil;

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    pub version: u32,
    #[serde(default)]
    pub workspaces: Vec<PathBuf>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            version: SCHEMA_VERSION,
            workspaces: Vec::new(),
        }
    }
}

/// `~/.config/dbeasy/config`
pub fn default_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join(".config"))
        .join("dbeasy")
        .join("config")
}

impl Settings {
    /// Carrega o arquivo; ausente equivale a uma configuração vazia.
    pub fn load(path: &Path) -> Result<Self, String> {
        match std::fs::read_to_string(path) {
            Ok(text) => {
                let settings: Settings = serde_json::from_str(&text).map_err(|e| {
                    format!("configuração global inválida ({}): {e}", path.display())
                })?;
                if settings.version > SCHEMA_VERSION {
                    return Err(format!(
                        "configuração global usa a versão de esquema {}, mais nova que a suportada ({SCHEMA_VERSION})",
                        settings.version
                    ));
                }
                Ok(settings)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(format!("não foi possível ler {}: {e}", path.display())),
        }
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        let json = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        fsutil::atomic_write(path, &json)
            .map_err(|e| format!("não foi possível gravar {}: {e}", path.display()))
    }

    /// Registra um arquivo pelo caminho absoluto normalizado. Retorna `false`
    /// se ele já estava registrado.
    pub fn register(&mut self, file: &Path) -> bool {
        let normalized = fsutil::canonical_or_normalized(file);
        if self
            .workspaces
            .iter()
            .any(|p| fsutil::canonical_or_normalized(p) == normalized)
        {
            return false;
        }
        self.workspaces.push(normalized);
        true
    }

    /// Remove o registro sem tocar no arquivo do projeto.
    pub fn unregister(&mut self, file: &Path) -> bool {
        let normalized = fsutil::canonical_or_normalized(file);
        let before = self.workspaces.len();
        self.workspaces
            .retain(|p| fsutil::canonical_or_normalized(p) != normalized);
        before != self.workspaces.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_dedupes_after_normalization() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("dbeasy.json");
        std::fs::write(&file, "[]").unwrap();
        let mut s = Settings::default();
        assert!(s.register(&file));
        assert!(!s.register(&dir.path().join(".").join("dbeasy.json")));
        assert_eq!(s.workspaces.len(), 1);
        assert!(s.unregister(&file));
        assert!(s.workspaces.is_empty());
        assert!(file.exists());
    }

    #[test]
    fn round_trip_and_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dbeasy").join("config");
        assert_eq!(Settings::load(&path).unwrap(), Settings::default());
        let mut s = Settings::default();
        s.workspaces.push(PathBuf::from("/a/dbeasy.json"));
        s.save(&path).unwrap();
        assert_eq!(Settings::load(&path).unwrap(), s);
    }

    #[test]
    fn rejects_newer_schema() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config");
        std::fs::write(&path, r#"{"version": 99, "workspaces": []}"#).unwrap();
        assert!(Settings::load(&path).is_err());
    }
}
