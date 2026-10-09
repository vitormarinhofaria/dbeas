//! Estado de execução de workspaces e abas.
//!
//! Separa o que é serializável (convertido para/desde `session`) do que é
//! transitório: conexão, catálogo, resultados e estado de rolagem.

use std::collections::HashSet;
use std::path::PathBuf;

use iced::widget::Id;
use iced::widget::scrollable::Viewport;
use iced::widget::text_editor::Content;

use crate::connection::{ConnectionHandle, Status, TxState};
use crate::data_edit::{EditTarget, Pending};
use crate::data_grid::ScrollState;
use crate::file_explorer::FileEntry;
use crate::metadata::Catalog;
use crate::query::ResultSet;
use crate::query::transform::Direction;
use crate::session::{ExplorerMode, TabKind, TabSession, WorkspaceSession};
use crate::workspace::Descriptor;

pub struct Workspace {
    pub desc: Descriptor,
    pub status: Status,
    pub connection_error: Option<String>,
    pub connection: Option<ConnectionHandle>,
    pub catalog: Option<Catalog>,
    pub catalog_loading: bool,
    pub catalog_error: Option<String>,
    pub explorer: ExplorerMode,
    pub filter: String,
    pub expanded: HashSet<String>,
    pub files: FilesState,
    pub tabs: Vec<Tab>,
    pub active_tab: usize,
    /// Estado da transação, espelhado da conexão após cada resposta.
    pub tx: TxState,
    /// Um comando de transação está em andamento.
    pub tx_busy: bool,
    /// Alterações feitas pela grade na transação atual (para o resumo).
    pub grid_log: Vec<String>,
}

#[derive(Default)]
pub struct FilesState {
    pub current_dir: Option<PathBuf>,
    pub entries: Option<Result<Vec<FileEntry>, String>>,
}

impl Workspace {
    pub fn new(desc: Descriptor, saved: Option<&WorkspaceSession>, next_id: &mut u64) -> Self {
        let mut tabs: Vec<Tab> = saved
            .map(|s| s.tabs.iter().map(|t| Tab::restore(t, next_id)).collect())
            .unwrap_or_default();
        // O Scratch é sempre a primeira aba (spec §4.3).
        if !matches!(tabs.first().map(|t| &t.kind), Some(TabKind::Scratch)) {
            tabs.retain(|t| !matches!(t.kind, TabKind::Scratch));
            tabs.insert(0, Tab::new(TabKind::Scratch, String::new(), next_id));
        }
        let active_tab = saved.map_or(0, |s| s.active_tab).min(tabs.len() - 1);

        Self {
            desc,
            status: Status::Disconnected,
            connection_error: None,
            connection: None,
            catalog: None,
            catalog_loading: false,
            catalog_error: None,
            explorer: saved.map(|s| s.explorer_mode).unwrap_or_default(),
            filter: String::new(),
            expanded: HashSet::new(),
            files: FilesState::default(),
            tabs,
            active_tab,
            tx: TxState::Idle,
            tx_busy: false,
            grid_log: Vec::new(),
        }
    }

    pub fn snapshot(&self) -> WorkspaceSession {
        WorkspaceSession {
            explorer_mode: self.explorer,
            active_tab: self.active_tab,
            tabs: self.tabs.iter().map(Tab::snapshot).collect(),
        }
    }

    pub fn tab_index(&self, id: u64) -> Option<usize> {
        self.tabs.iter().position(|t| t.id == id)
    }

    pub fn read_only(&self) -> bool {
        self.desc.config.as_ref().is_ok_and(|c| c.read_only)
    }

    /// Libera recursos transitórios. Resultados ficam visíveis, mas os
    /// cursores deixam de existir.
    pub fn require_transaction(&self) -> bool {
        self.desc
            .config
            .as_ref()
            .is_ok_and(|c| c.require_transaction)
    }

    /// Atualiza o estado da transação a partir da conexão.
    pub fn sync_tx(&mut self) {
        if let Some(handle) = &self.connection {
            self.tx = handle.tx_state();
        }
        if self.tx == TxState::Idle {
            self.grid_log.clear();
        }
    }

    pub fn disconnect(&mut self) {
        self.tx = TxState::Idle;
        self.tx_busy = false;
        self.grid_log.clear();
        self.connection = None;
        self.status = Status::Disconnected;
        self.catalog_loading = false;
        for tab in &mut self.tabs {
            tab.run.cursor = None;
            tab.run.running = false;
            tab.run.fetching = false;
            if let Some(result) = &mut tab.run.result {
                result.has_more = false;
            }
        }
    }
}

pub struct Tab {
    pub id: u64,
    pub kind: TabKind,
    pub editor: Content,
    /// Alterações não salvas (só arquivos SQL).
    pub dirty: bool,
    /// Arquivo ainda sendo lido do disco.
    pub loading_file: bool,
    pub split: f32,
    pub run: RunState,
    pub editor_scroll_id: Id,
    pub editor_viewport: Option<Viewport>,
    pub grid_header_id: Id,
    pub grid_body_id: Id,
    pub grid_scroll: ScrollState,
    pub widths: Vec<f32>,
    pub selected: Option<(usize, usize)>,
    pub columns: ColumnLayout,
    /// Instrução que produziu o resultado atual, para transformações.
    pub source: Option<Source>,
    /// Texto da instrução antes da primeira remoção/reordenação de colunas,
    /// usado por "restaurar colunas".
    pub pristine: Option<String>,
    /// O SQL foi gerado pela aplicação e não foi editado pelo usuário.
    pub generated: bool,
    /// Tabela para a qual o resultado é editável, ou o motivo de não ser.
    pub edit_target: Option<Result<EditTarget, String>>,
    /// Alterações pendentes da grade, separadas dos valores originais.
    pub pending: Pending,
    /// Célula em edição.
    pub editing: Option<CellEditor>,
}

#[derive(Debug, Clone)]
pub struct CellEditor {
    pub row: usize,
    pub col: usize,
    pub text: String,
    pub input_id: Id,
}

/// Disposição visual das colunas da grade (spec §5.2–5.4).
#[derive(Debug, Clone, Default)]
pub struct ColumnLayout {
    /// Colunas visíveis do resultado, na ordem exibida.
    pub order: Vec<usize>,
    /// Coluna de ordenação (pelo nome no resultado) e direção.
    pub sort: Option<(String, Direction)>,
    /// Cabeçalho pressionado (início de clique ou arraste).
    pub pressed: Option<usize>,
    /// Para cada coluna do resultado, o índice do item correspondente no
    /// `SELECT` atual, se o mapeamento for conhecido.
    pub item_of: Vec<Option<usize>>,
    /// Nomes ocultados a reaplicar após a próxima execução (sessão restaurada).
    pub pending_hidden: Vec<String>,
}

impl ColumnLayout {
    pub fn reset(&mut self, columns: usize, items: Option<usize>) {
        self.order = (0..columns).collect();
        self.pressed = None;
        self.item_of = if items == Some(columns) {
            (0..columns).map(Some).collect()
        } else {
            vec![None; columns]
        };
    }

    pub fn hidden_count(&self) -> usize {
        self.item_of.len().saturating_sub(self.order.len())
    }
}

/// Instrução de origem de um resultado e sua posição no editor.
#[derive(Debug, Clone)]
pub struct Source {
    pub range: std::ops::Range<usize>,
    pub text: String,
}

#[derive(Default)]
pub struct RunState {
    pub running: bool,
    pub fetching: bool,
    /// Incrementado a cada execução; respostas de execuções antigas são ignoradas.
    pub exec_id: u64,
    /// Deslocamento, no texto do editor, do trecho enviado na execução atual.
    pub base_offset: usize,
    pub result: Option<ResultSet>,
    pub cursor: Option<u64>,
    pub messages: Vec<String>,
    pub error: Option<ErrorInfo>,
    pub elapsed_ms: Option<u128>,
    /// SQL enviado na execução atual.
    pub sent: String,
    /// A execução atual foi disparada pela grade (ordenação), não pelo usuário.
    pub from_grid: bool,
}

#[derive(Debug, Clone)]
pub struct ErrorInfo {
    pub text: String,
    /// Linha e coluna a partir de 1, quando conhecidas.
    pub position: Option<(usize, usize)>,
    /// Linha (a partir de 0) para marcação no gutter.
    pub line: Option<usize>,
}

impl Tab {
    pub fn new(kind: TabKind, sql: String, next_id: &mut u64) -> Self {
        *next_id += 1;
        let id = *next_id;
        Self {
            id,
            kind,
            editor: Content::with_text(&sql),
            dirty: false,
            loading_file: false,
            split: 0.55,
            run: RunState::default(),
            editor_scroll_id: Id::unique(),
            editor_viewport: None,
            grid_header_id: Id::unique(),
            grid_body_id: Id::unique(),
            grid_scroll: ScrollState::default(),
            widths: Vec::new(),
            selected: None,
            columns: ColumnLayout::default(),
            source: None,
            pristine: None,
            generated: false,
            edit_target: None,
            pending: Pending::new(),
            editing: None,
        }
    }

    fn restore(saved: &TabSession, next_id: &mut u64) -> Self {
        let mut tab = Self::new(saved.kind.clone(), saved.sql.clone(), next_id);
        tab.split = saved.split.clamp(0.1, 0.9);
        tab.columns.pending_hidden = saved.hidden_columns.clone();
        tab.columns.sort = saved.sort.as_ref().map(|s| {
            let direction = if s.descending {
                Direction::Desc
            } else {
                Direction::Asc
            };
            (s.column.clone(), direction)
        });
        if let TabKind::File { dirty, .. } = &saved.kind {
            tab.dirty = *dirty;
            // Arquivos sem alterações são relidos do disco pelo app.
            tab.loading_file = !*dirty;
        }
        tab
    }

    pub fn snapshot(&self) -> TabSession {
        let kind = match &self.kind {
            TabKind::File { path, .. } => TabKind::File {
                path: path.clone(),
                dirty: self.dirty,
            },
            other => other.clone(),
        };
        let sql = match &self.kind {
            // Conteúdo salvo é relido do disco; não precisa ir para a sessão.
            TabKind::File { .. } if !self.dirty => String::new(),
            _ => self.editor.text(),
        };
        let hidden_columns = match &self.run.result {
            Some(result) if !self.columns.item_of.is_empty() => (0..result.columns.len())
                .filter(|c| !self.columns.order.contains(c))
                .map(|c| result.columns[c].name.clone())
                .collect(),
            _ => self.columns.pending_hidden.clone(),
        };
        TabSession {
            kind,
            sql,
            split: self.split,
            hidden_columns,
            sort: self
                .columns
                .sort
                .as_ref()
                .map(|(column, d)| crate::session::SavedSort {
                    column: column.clone(),
                    descending: *d == Direction::Desc,
                }),
        }
    }

    pub fn title(&self) -> String {
        match &self.kind {
            TabKind::Scratch => "Scratch".into(),
            TabKind::Table { schema, name } => {
                if schema == "public" {
                    name.clone()
                } else {
                    format!("{schema}.{name}")
                }
            }
            TabKind::File { path, .. } => path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.display().to_string()),
        }
    }

    pub fn is_scratch(&self) -> bool {
        matches!(self.kind, TabKind::Scratch)
    }

    pub fn file_path(&self) -> Option<&PathBuf> {
        match &self.kind {
            TabKind::File { path, .. } => Some(path),
            _ => None,
        }
    }
}
