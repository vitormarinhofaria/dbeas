// SPDX-FileCopyrightText: 2026 Vitor Marinho Faria
// SPDX-License-Identifier: GPL-3.0-or-later

//! Configuração global: registro dos arquivos `dbeasy.json` (spec §3.4).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::fsutil;

pub const SCHEMA_VERSION: u32 = 1;

/// Escolha de tema: acompanhar o sistema (padrão) ou um tema fixo.
#[derive(Debug, Clone, PartialEq)]
pub enum ThemeChoice {
    System,
    Named(iced::Theme),
}

impl ThemeChoice {
    /// Todas as opções, com a do sistema primeiro.
    pub fn all() -> Vec<Self> {
        std::iter::once(Self::System)
            .chain(iced::Theme::ALL.iter().cloned().map(Self::Named))
            .collect()
    }

    /// Tema efetivo; `System` vira Light ou Dark conforme o modo do sistema.
    pub fn resolve(&self, mode: iced::theme::Mode) -> iced::Theme {
        match self {
            Self::System => <iced::Theme as iced::theme::Base>::default(mode),
            Self::Named(theme) => theme.clone(),
        }
    }
}

impl std::fmt::Display for ThemeChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::System => f.write_str("Padrão (sistema)"),
            Self::Named(theme) => theme.fmt(f),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    pub version: u32,
    #[serde(default)]
    pub workspaces: Vec<PathBuf>,
    /// Nome (`Display`) do tema aplicado; ausente acompanha o sistema.
    #[serde(default)]
    pub theme: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            version: SCHEMA_VERSION,
            workspaces: Vec::new(),
            theme: None,
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

    /// Tema aplicado; ausente ou desconhecido (ex.: tema removido) acompanha o sistema.
    pub fn theme_choice(&self) -> ThemeChoice {
        self.theme
            .as_deref()
            .and_then(|name| iced::Theme::ALL.iter().find(|t| t.to_string() == name))
            .cloned()
            .map_or(ThemeChoice::System, ThemeChoice::Named)
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
    fn theme_persists_and_falls_back() {
        use iced::theme::Mode;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config");
        let mut s = Settings::default();
        assert_eq!(s.theme_choice(), ThemeChoice::System);
        s.theme = Some(iced::Theme::Nord.to_string());
        s.save(&path).unwrap();
        assert_eq!(
            Settings::load(&path).unwrap().theme_choice(),
            ThemeChoice::Named(iced::Theme::Nord)
        );
        s.theme = Some("inexistente".into());
        assert_eq!(s.theme_choice(), ThemeChoice::System);
        std::fs::write(&path, r#"{"version": 1, "workspaces": []}"#).unwrap();
        assert_eq!(Settings::load(&path).unwrap().theme_choice(), ThemeChoice::System);
        assert_eq!(ThemeChoice::System.resolve(Mode::Light), iced::Theme::Light);
        assert_eq!(ThemeChoice::System.resolve(Mode::Dark), iced::Theme::Dark);
    }

    #[test]
    fn rejects_newer_schema() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config");
        std::fs::write(&path, r#"{"version": 99, "workspaces": []}"#).unwrap();
        assert!(Settings::load(&path).is_err());
    }
}
