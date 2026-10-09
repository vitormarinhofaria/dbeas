//! Persistência do estado de sessão por workspace (spec §10.1).
//!
//! Guarda apenas estado serializável: abas, ordem, aba ativa, SQL, modo do
//! explorador e layout. Resultados de consultas e conexões nunca são salvos.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::fsutil;
use crate::workspace::WorkspaceId;

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Session {
    pub version: u32,
    #[serde(default)]
    pub active_workspace: Option<WorkspaceId>,
    #[serde(default)]
    pub layout: Layout,
    #[serde(default)]
    pub workspaces: BTreeMap<WorkspaceId, WorkspaceSession>,
}

impl Default for Session {
    fn default() -> Self {
        Self {
            version: SCHEMA_VERSION,
            active_workspace: None,
            layout: Layout::default(),
            workspaces: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Layout {
    pub sidebar_width: f32,
}

impl Default for Layout {
    fn default() -> Self {
        Self {
            sidebar_width: 280.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExplorerMode {
    #[default]
    Tables,
    Objects,
    Files,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct WorkspaceSession {
    #[serde(default)]
    pub explorer_mode: ExplorerMode,
    #[serde(default)]
    pub active_tab: usize,
    /// O primeiro item é sempre o Scratch.
    #[serde(default)]
    pub tabs: Vec<TabSession>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TabSession {
    pub kind: TabKind,
    /// SQL do editor. Para arquivos, só é usado se `dirty`; caso contrário o
    /// conteúdo é relido do disco ao restaurar.
    pub sql: String,
    /// Fração da altura ocupada pela grade de resultados.
    #[serde(default = "default_split")]
    pub split: f32,
}

fn default_split() -> f32 {
    0.55
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TabKind {
    Scratch,
    Table { schema: String, name: String },
    File { path: PathBuf, dirty: bool },
}

/// `~/.local/state/dbeas/session.json` (XDG_STATE_HOME), separado da
/// configuração global.
pub fn default_path() -> PathBuf {
    dirs::state_dir()
        .or_else(dirs::config_dir)
        .unwrap_or_else(|| {
            dirs::home_dir()
                .unwrap_or_default()
                .join(".local")
                .join("state")
        })
        .join("dbeas")
        .join("session.json")
}

impl Session {
    /// Sessão ausente ou ilegível não impede a abertura: começa vazia.
    pub fn load(path: &Path) -> (Self, Option<String>) {
        match std::fs::read_to_string(path) {
            Ok(text) => match serde_json::from_str::<Session>(&text) {
                Ok(s) if s.version <= SCHEMA_VERSION => (s, None),
                Ok(s) => (
                    Self::default(),
                    Some(format!(
                        "sessão salva usa versão {} não suportada; ignorada",
                        s.version
                    )),
                ),
                Err(e) => (
                    Self::default(),
                    Some(format!("sessão salva inválida, ignorada: {e}")),
                ),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (Self::default(), None),
            Err(e) => (
                Self::default(),
                Some(format!("não foi possível ler a sessão: {e}")),
            ),
        }
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        let json = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        fsutil::atomic_write(path, &json)
            .map_err(|e| format!("não foi possível gravar a sessão: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.json");
        let mut s = Session::default();
        s.active_workspace = Some(WorkspaceId("a#b".into()));
        s.workspaces.insert(
            WorkspaceId("a#b".into()),
            WorkspaceSession {
                explorer_mode: ExplorerMode::Files,
                active_tab: 1,
                tabs: vec![
                    TabSession {
                        kind: TabKind::Scratch,
                        sql: "select 1".into(),
                        split: 0.5,
                    },
                    TabSession {
                        kind: TabKind::Table {
                            schema: "public".into(),
                            name: "t".into(),
                        },
                        sql: "select a from t".into(),
                        split: 0.6,
                    },
                ],
            },
        );
        s.save(&path).unwrap();
        let (loaded, warning) = Session::load(&path);
        assert!(warning.is_none());
        assert_eq!(loaded, s);
    }

    #[test]
    fn invalid_session_starts_empty_with_warning() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.json");
        std::fs::write(&path, "nope").unwrap();
        let (loaded, warning) = Session::load(&path);
        assert_eq!(loaded, Session::default());
        assert!(warning.is_some());
    }
}
