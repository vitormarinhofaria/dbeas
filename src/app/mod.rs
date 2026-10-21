// SPDX-FileCopyrightText: 2026 Vitor Marinho Faria
// SPDX-License-Identifier: GPL-3.0-or-later

//! Estado global, mensagens e ciclo de vida da janela.

mod focus;
mod quick_open;
mod right_click;
mod state;
mod view;

use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;
use std::time::Duration;

use iced::keyboard::{Key, Modifiers, key};
use iced::widget::scrollable::{AbsoluteOffset, Viewport};
use iced::widget::{operation, text_editor};
use iced::{Event, Point, Size, Subscription, Task, Theme, event, mouse, window};

use crate::commands::{self, Command};
use crate::connection::{
    self, ConnectionHandle, ExecOptions, Page, Status, TxCommand, TxState, TxSummary,
};
use crate::data_edit;
use crate::data_grid;
use crate::file_explorer::{self, FileEntry};
use crate::metadata::{self, Catalog};
use crate::query::transform::{self, Direction};
use crate::query::{Outcome, QueryError, split};
use crate::session::{self, ExplorerMode, Session, TabKind};
use crate::settings::{self, Settings, ThemeChoice};
use crate::sql_editor;
use crate::workspace::editor::{self as config_editor, SslChoice};
use crate::workspace::{self, WorkspaceId};

use quick_open::QuickOpen;
use state::{ErrorInfo, Tab, Workspace};

/// Identificador da aplicação no desktop: casa a janela com o arquivo
/// `.desktop` e o ícone instalados pelo pacote (ver `packaging/`).
pub const APP_ID: &str = "io.github.vitormarinhofaria.DBeasy";

pub fn run() -> iced::Result {
    iced::application(App::boot, App::update, App::view)
        .window(window_settings())
        .title(App::title)
        .theme(App::theme)
        .subscription(App::subscription)
        .exit_on_close_request(false)
        .window_size(Size::new(1400.0, 860.0))
        .run()
}

fn window_settings() -> iced::window::Settings {
    iced::window::Settings {
        #[cfg(target_os = "linux")]
        platform_specific: iced::window::settings::PlatformSpecific {
            application_id: APP_ID.to_string(),
            ..Default::default()
        },
        ..Default::default()
    }
}

pub struct App {
    settings_path: PathBuf,
    settings: Settings,
    /// Seção de configurações da barra lateral aberta.
    app_settings_open: bool,
    /// Lista de workspaces da barra lateral expandida (recolhida mostra só o ativo).
    workspaces_open: bool,
    /// Tema em pré-visualização, ainda não aplicado.
    theme_preview: Option<ThemeChoice>,
    /// Modo claro/escuro do sistema, para a opção de tema padrão.
    system_mode: iced::theme::Mode,
    session_path: PathBuf,
    /// Última sessão conhecida, inclusive de workspaces que não puderam ser
    /// carregados agora (arquivo ausente), para não perder seu estado.
    session: Session,
    workspaces: Vec<Workspace>,
    active: Option<usize>,
    sidebar_width: f32,
    window_width: f32,
    window_height: f32,
    /// Posição do último clique direito, para abrir menus de contexto no cursor.
    last_right_click: Point,
    /// Menu de contexto aberto sobre uma aba do workspace ativo.
    tab_menu: Option<TabMenu>,
    drag: Option<Drag>,
    modal: Option<Modal>,
    quick_open: Option<QuickOpen>,
    sql_files: HashMap<WorkspaceId, Vec<PathBuf>>,
    notice: Option<Notice>,
    session_dirty: bool,
    next_tab_id: u64,
    /// Identifica o teste de conexão do editor de configurações em andamento.
    next_test_id: u64,
    after_tx: Option<AfterTx>,
    pending_execution: Option<PendingExecution>,
    /// Janela cujo fechamento foi pedido, enquanto há transação a resolver.
    closing_window: Option<window::Id>,
    /// Modificadores pressionados agora (Ctrl/Shift no clique de linha).
    modifiers: Modifiers,
}

/// Menu de contexto de uma aba (clique direito).
pub struct TabMenu {
    pub tab_id: u64,
    /// Canto superior esquerdo do menu, em coordenadas da janela.
    pub at: Point,
}

/// Ações do menu de contexto das abas.
#[derive(Debug, Clone, Copy)]
pub enum TabMenuAction {
    Close,
    CloseLeft,
    CloseRight,
    CloseAll,
}

#[derive(Debug, Clone)]
pub struct Notice {
    pub text: String,
    pub is_error: bool,
}

#[derive(Debug, Clone, Copy)]
pub enum DragTarget {
    Sidebar,
    Split,
}

#[derive(Debug, Clone, Copy)]
enum Drag {
    Sidebar,
    Split { start: Option<(f32, f32)> },
}

pub enum Modal {
    /// Resumo da transação antes do COMMIT.
    Commit {
        ws: WorkspaceId,
        summary: Option<Result<TxSummary, String>>,
        grid: Vec<String>,
    },
    /// Fechar a aplicação ou desconectar com transação aberta.
    PendingTransaction {
        ws: WorkspaceId,
        name: String,
        failed: bool,
        closing: bool,
    },
    /// Executar um arquivo salvo dentro de uma transação nova.
    RunFileInTransaction {
        title: String,
    },
    /// Novo nome de um arquivo ou diretório.
    RenameItem {
        entry: FileEntry,
        name: String,
        error: Option<String>,
        input_id: iced::widget::Id,
        busy: bool,
    },
    /// Confirmação de exclusão (move para a lixeira).
    DeleteItem {
        entry: FileEntry,
        /// Arquivos `.sql` dentro do diretório.
        sql_files: usize,
        /// Abas abertas que serão fechadas, e se alguma tem alterações.
        open_tabs: Vec<String>,
        unsaved: bool,
        error: Option<String>,
        busy: bool,
    },
    /// Nome do novo arquivo SQL. `dir` é o diretório exibido, relativo ao
    /// `sqlFilesDir` (para exibição).
    NewFile {
        dir: String,
        name: String,
        error: Option<String>,
        input_id: iced::widget::Id,
        creating: bool,
    },
    /// Confirmação da exclusão das linhas selecionadas na grade (Del).
    DeleteRows {
        tab: u64,
        /// `schema.tabela`.
        table: String,
        /// Chave de cada linha a excluir, para exibição.
        keys: Vec<String>,
        sql: String,
        /// Alterações pendentes nessas linhas, descartadas com a exclusão.
        dropped_edits: usize,
        error: Option<String>,
        running: bool,
        needs_transaction: bool,
        in_transaction: bool,
    },
    /// SQL das alterações da grade, antes de executar (Ctrl+S).
    GridSave {
        tab: u64,
        sql: String,
        error: Option<String>,
        running: bool,
        needs_transaction: bool,
    },
    AddWorkspace {
        path: String,
        error: Option<String>,
        /// Interpretação de `path`; decide entre "Adicionar" e "Criar".
        target: workspace::add::Target,
        /// Pedindo confirmação para criar o arquivo de `target`.
        confirm: bool,
    },
    RemoveWorkspace {
        file: PathBuf,
        names: Vec<String>,
    },
    /// Edição gráfica das entradas de um `dbeasy.json`, uma por aba.
    EditWorkspace {
        doc: Box<config_editor::Document>,
        error: Option<String>,
        show_password: bool,
        /// Teste de conexão da aba exibida; descartado ao editar ou trocar de aba.
        test: Option<ConnTest>,
    },
    /// Fechamento de abas com alterações não salvas. `tab_ids` são todas as
    /// abas a fechar (inclusive as sem alterações), `dirty` os títulos das
    /// que têm alterações.
    CloseTab {
        tab_ids: Vec<u64>,
        dirty: Vec<String>,
    },
    Shortcuts,
    /// Versão, copyright e licenças. `license` mostra o texto da GPL;
    /// `error` é a falha ao abrir algo externo (navegador, avisos de terceiros).
    About {
        license: bool,
        error: Option<String>,
    },
}

/// Teste de conexão do editor de configurações.
pub struct ConnTest {
    id: u64,
    /// `None` enquanto conecta.
    result: Option<Result<connection::Probe, String>>,
}

/// O que fazer depois que a transação pendente for encerrada.
#[derive(Debug, Clone)]
enum AfterTx {
    Close(window::Id),
    Disconnect(WorkspaceId),
}

/// Execução aguardando o BEGIN pedido pelo usuário.
struct PendingExecution {
    ws: WorkspaceId,
    tab: u64,
    sql: String,
    base: usize,
    statements: Vec<std::ops::Range<usize>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseChoice {
    Save,
    Discard,
}

#[derive(Debug, Clone)]
pub enum Message {
    // Workspaces
    SelectWorkspace(usize),
    Connect(usize),
    Disconnect(usize),
    Connected(WorkspaceId, Result<ConnectionHandle, String>),
    CatalogLoaded(WorkspaceId, Result<Catalog, String>),
    OpenAddWorkspace,
    AddPathChanged(String),
    BrowseWorkspaceFile,
    BrowseWorkspaceDir,
    BrowsedWorkspacePath(Option<PathBuf>),
    SubmitAddWorkspace,
    /// Volta da confirmação de criação para a edição do caminho.
    CancelCreateWorkspace,
    AskRemoveWorkspace(usize),
    ConfirmRemoveWorkspace,
    OpenWorkspaceSettings(usize),
    SettingsTab(usize),
    SettingsAddEntry,
    SettingsRemoveEntry(usize),
    SettingsField(config_editor::Field, String),
    SettingsSslMode(SslChoice),
    SettingsEnvKey(usize, String),
    SettingsEnvValue(usize, String),
    SettingsEnvAdd,
    SettingsEnvRemove(usize),
    SettingsReadOnly(bool),
    SettingsRequireTransaction(bool),
    SettingsTogglePassword,
    /// Abre o seletor de diretório do `sqlFilesDir`, no diretório do json.
    SettingsBrowseSqlDir,
    /// Diretório escolhido para a aba indicada.
    SettingsPickedSqlDir(usize, Option<PathBuf>),
    SettingsSave,
    SettingsTestConnection,
    SettingsTested(u64, Result<connection::Probe, String>),

    // Transações
    TxBegin(WorkspaceId),
    TxRequestCommit(WorkspaceId),
    TxSummaryLoaded(WorkspaceId, Result<TxSummary, String>),
    TxCommit(WorkspaceId),
    TxRollback(WorkspaceId),
    TxFinished(WorkspaceId, TxCommand, Result<(), String>),
    PendingTxCommit,
    PendingTxRollback,
    ConfirmRunFile,

    // Exploradores
    SetExplorer(ExplorerMode),
    FilterChanged(String),
    ToggleExpanded(String),
    OpenRelation {
        schema: String,
        name: String,
    },
    NavigateDir(PathBuf),
    FileMenu(FileEntry),
    CloseFileMenu,
    RenameDialog,
    RenameName(String),
    RenameSubmit,
    Renamed(WorkspaceId, PathBuf, Result<PathBuf, String>),
    DeleteDialog,
    DeleteConfirm,
    Deleted(WorkspaceId, Result<PathBuf, String>),
    NewFileDialog,
    NewFileName(String),
    NewFileSubmit,
    FileCreated(WorkspaceId, Result<PathBuf, String>),
    FilesListed(WorkspaceId, PathBuf, Result<Vec<FileEntry>, String>),
    OpenFile(PathBuf),
    FileRead {
        ws: WorkspaceId,
        tab: u64,
        result: Result<String, String>,
    },
    SqlFilesScanned(WorkspaceId, Vec<PathBuf>),

    // Abas
    SelectTab(usize),
    CloseTab(usize),
    ConfirmCloseTab(CloseChoice),
    RightClickAt(Point),
    TabMenu(usize),
    TabMenuAction(TabMenuAction),
    CloseTabMenu,
    Editor(text_editor::Action),
    EditorScrolled(Viewport),
    GridScrolled(Viewport),
    SelectCell(usize, usize),
    /// Clique no número da linha (Ctrl/Shift pelos modificadores atuais).
    SelectRow(usize),
    /// Del ou botão Excluir: abre a confirmação da exclusão.
    DeleteRowsDialog,
    DeleteRowsConfirm,
    RowsDeleted {
        ws: WorkspaceId,
        tab: u64,
        rows: BTreeSet<usize>,
        log: Vec<String>,
        result: Result<Vec<Vec<crate::query::Value>>, QueryError>,
    },
    ModifiersChanged(Modifiers),
    CompletionMove(i32),
    CompletionAccept,
    CompletionClose,
    CompletionPick(usize),
    DiagnoseDue {
        ws: WorkspaceId,
        tab: u64,
        rev: u64,
    },
    Diagnosed {
        ws: WorkspaceId,
        tab: u64,
        rev: u64,
        diagnostics: Vec<crate::diagnostics::Diagnostic>,
    },
    StartEdit(usize, usize),
    EditInput(String),
    CommitEdit,
    SetNull,
    RevertCell,
    RevertAll,
    GridSaveCopy,
    GridSaveExecute,
    GridSaveDiscard,
    EditsApplied {
        ws: WorkspaceId,
        tab: u64,
        cells: Vec<(usize, Vec<usize>)>,
        log: Vec<String>,
        result: Result<Vec<Vec<crate::query::Value>>, QueryError>,
    },
    HeaderPressed(usize),
    HeaderReleased(usize),
    HideColumn(usize),
    RestoreColumns,
    FetchMore,
    Executed {
        ws: WorkspaceId,
        tab: u64,
        exec_id: u64,
        result: Result<Outcome, QueryError>,
    },
    Fetched {
        ws: WorkspaceId,
        tab: u64,
        exec_id: u64,
        result: Result<Page, QueryError>,
    },
    Saved {
        ws: WorkspaceId,
        tab: u64,
        result: Result<(), String>,
        then_close: bool,
    },
    Cancelled(Result<(), String>),

    // Comandos e seletor rápido
    Command(Command),
    KeyPressed {
        key: Key,
        physical: key::Physical,
        modifiers: Modifiers,
        captured: bool,
    },
    QuickOpenQuery(String),
    QuickOpenSubmit,
    QuickOpenPick(usize),
    ShowShortcuts,
    ShowAbout,
    ToggleAppSettings,
    ToggleWorkspaces,
    PreviewTheme(ThemeChoice),
    SystemTheme(iced::theme::Mode),
    ApplyTheme,
    RevertTheme,
    AboutToggleLicense,
    AboutOpenSource,
    AboutOpenThirdParty,
    CloseModal,

    // Layout
    DragStart(DragTarget),
    DragMove(Point),
    DragEnd,
    WindowResized(Size),

    // Ciclo de vida
    CloseRequested(window::Id),
    Autosave,
    SessionSaved(Result<(), String>),
    DismissNotice,
}

impl App {
    /// Registra na tela "Sobre" a falha (ou o sucesso) ao abrir algo externo.
    fn about_open(&mut self, result: Result<(), String>) {
        if let Some(Modal::About { error, .. }) = &mut self.modal {
            *error = result.err();
        }
    }

    fn boot() -> (Self, Task<Message>) {
        let settings_path = settings::default_path();
        let session_path = session::default_path();

        let mut notices = Vec::new();
        let settings = Settings::load(&settings_path).unwrap_or_else(|e| {
            notices.push(e);
            Settings::default()
        });
        let (session, warning) = Session::load(&session_path);
        notices.extend(warning);

        let mut app = Self {
            settings_path,
            settings,
            app_settings_open: false,
            workspaces_open: false,
            theme_preview: None,
            system_mode: iced::theme::Mode::None,
            session_path,
            sidebar_width: session.layout.sidebar_width.clamp(180.0, 700.0),
            session,
            workspaces: Vec::new(),
            active: None,
            window_width: 1400.0,
            window_height: 860.0,
            last_right_click: Point::ORIGIN,
            tab_menu: None,
            drag: None,
            modal: None,
            quick_open: None,
            sql_files: HashMap::new(),
            notice: notices.pop().map(|text| Notice {
                text,
                is_error: true,
            }),
            session_dirty: false,
            next_tab_id: 0,
            next_test_id: 0,
            after_tx: None,
            pending_execution: None,
            closing_window: None,
            modifiers: Modifiers::empty(),
        };

        let mut tasks = vec![
            app.reload_workspaces(),
            iced::system::theme().map(Message::SystemTheme),
        ];
        let active = app
            .session
            .active_workspace
            .as_ref()
            .and_then(|id| app.workspaces.iter().position(|w| &w.desc.id == id))
            .or(if app.workspaces.is_empty() {
                None
            } else {
                Some(0)
            });
        if let Some(index) = active {
            tasks.push(app.select_workspace(index));
        }
        (app, Task::batch(tasks))
    }

    fn title(&self) -> String {
        match self.active_ws() {
            Some(ws) => format!("DBeasy — {}", ws.desc.display_name),
            None => "DBeasy".into(),
        }
    }

    fn theme(&self) -> Theme {
        self.theme_preview
            .clone()
            .unwrap_or_else(|| self.settings.theme_choice())
            .resolve(self.system_mode)
    }

    fn subscription(&self) -> Subscription<Message> {
        let mut subs = vec![
            event::listen_with(|event, status, _window| match event {
                Event::Keyboard(iced::keyboard::Event::KeyPressed {
                    key,
                    physical_key,
                    modifiers,
                    ..
                }) => Some(Message::KeyPressed {
                    key,
                    physical: physical_key,
                    modifiers,
                    captured: status == event::Status::Captured,
                }),
                Event::Keyboard(iced::keyboard::Event::ModifiersChanged(modifiers)) => {
                    Some(Message::ModifiersChanged(modifiers))
                }
                Event::Window(window::Event::Resized(size)) => Some(Message::WindowResized(size)),
                // Clique fora de qualquer widget fecha o menu de contexto.
                Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))
                    if status == event::Status::Ignored =>
                {
                    Some(Message::CloseFileMenu)
                }
                _ => None,
            }),
            window::close_requests().map(Message::CloseRequested),
        ];
        if self.session_dirty {
            subs.push(iced::time::every(Duration::from_secs(3)).map(|_| Message::Autosave));
        }
        if self.drag.is_some() {
            subs.push(event::listen_with(|event, _status, _window| match event {
                Event::Mouse(mouse::Event::CursorMoved { position }) => {
                    Some(Message::DragMove(position))
                }
                Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                    Some(Message::DragEnd)
                }
                _ => None,
            }));
        }
        subs.push(iced::system::theme_changes().map(Message::SystemTheme));
        Subscription::batch(subs)
    }

    // ------------------------------------------------------------------
    // Acesso ao estado

    fn active_ws(&self) -> Option<&Workspace> {
        self.active.and_then(|i| self.workspaces.get(i))
    }

    fn active_ws_mut(&mut self) -> Option<&mut Workspace> {
        self.active.and_then(|i| self.workspaces.get_mut(i))
    }

    fn active_tab_mut(&mut self) -> Option<&mut Tab> {
        let ws = self.active_ws_mut()?;
        let index = ws.active_tab;
        ws.tabs.get_mut(index)
    }

    fn ws_by_id(&mut self, id: &WorkspaceId) -> Option<&mut Workspace> {
        self.workspaces.iter_mut().find(|w| &w.desc.id == id)
    }

    fn tab_by_id(&mut self, ws: &WorkspaceId, tab: u64) -> Option<&mut Tab> {
        self.ws_by_id(ws)?.tabs.iter_mut().find(|t| t.id == tab)
    }

    fn notify(&mut self, text: impl Into<String>) {
        self.notice = Some(Notice {
            text: text.into(),
            is_error: false,
        });
    }

    fn notify_error(&mut self, text: impl Into<String>) {
        self.notice = Some(Notice {
            text: text.into(),
            is_error: true,
        });
    }

    // ------------------------------------------------------------------
    // Atualização

    fn update(&mut self, message: Message) -> Task<Message> {
        if !matches!(
            message,
            Message::Autosave
                | Message::SessionSaved(_)
                | Message::DragMove(_)
                | Message::WindowResized(_)
                | Message::EditorScrolled(_)
                | Message::GridScrolled(_)
                | Message::KeyPressed { .. }
                | Message::CloseFileMenu
                | Message::FileMenu(_)
                | Message::RightClickAt(_)
                | Message::TabMenu(_)
                | Message::CloseTabMenu
        ) {
            self.session_dirty = true;
        }

        if closes_file_menu(&message) {
            if let Some(ws) = self.active_ws_mut() {
                ws.files.menu = None;
            }
        }

        if let Message::Executed { ws, .. } | Message::Fetched { ws, .. } = &message {
            let id = ws.clone();
            if let Some(w) = self.ws_by_id(&id) {
                w.sync_tx();
            }
        }

        match message {
            Message::SelectWorkspace(index) => self.select_workspace(index),
            Message::Connect(index) => self.connect(index),
            Message::Disconnect(index) => {
                let Some(ws) = self.workspaces.get_mut(index) else {
                    return Task::none();
                };
                if ws.tx != TxState::Idle && ws.connection.is_some() {
                    self.modal = Some(Modal::PendingTransaction {
                        ws: ws.desc.id.clone(),
                        name: ws.desc.display_name.clone(),
                        failed: ws.tx == TxState::Failed,
                        closing: false,
                    });
                    return Task::none();
                }
                ws.disconnect();
                Task::none()
            }
            Message::Connected(id, result) => {
                let Some(ws) = self.ws_by_id(&id) else {
                    return Task::none();
                };
                if ws.status != Status::Connecting {
                    return Task::none();
                }
                match result {
                    Ok(handle) => {
                        ws.status = Status::Connected;
                        ws.connection_error = None;
                        ws.connection = Some(handle);
                        let catalog = self.load_catalog(&id);
                        let active = self.active_ws().is_some_and(|w| w.desc.id == id);
                        let rerun = active
                            && self
                                .active_tab_mut()
                                .is_some_and(|t| std::mem::take(&mut t.rerun_on_connect));
                        if rerun {
                            Task::batch([catalog, self.restore_blank_tab()])
                        } else {
                            catalog
                        }
                    }
                    Err(error) => {
                        ws.status = Status::Error;
                        ws.connection_error = Some(error);
                        Task::none()
                    }
                }
            }
            Message::CatalogLoaded(id, result) => {
                if let Some(ws) = self.ws_by_id(&id) {
                    ws.catalog_loading = false;
                    match result {
                        Ok(catalog) => {
                            ws.catalog = Some(catalog);
                            ws.catalog_error = None;
                            refresh_edit_targets(ws);
                        }
                        Err(e) => ws.catalog_error = Some(e),
                    }
                }
                Task::none()
            }
            Message::OpenAddWorkspace => {
                self.modal = Some(Modal::AddWorkspace {
                    path: String::new(),
                    error: None,
                    target: workspace::add::resolve(""),
                    confirm: false,
                });
                Task::none()
            }
            Message::AddPathChanged(value) => {
                self.set_add_path(value);
                Task::none()
            }
            Message::BrowseWorkspaceFile => Task::perform(
                async {
                    rfd::AsyncFileDialog::new()
                        .set_title("Selecione um dbeasy.json")
                        .add_filter("dbeasy.json", &["json"])
                        .pick_file()
                        .await
                        .map(|f| f.path().to_path_buf())
                },
                Message::BrowsedWorkspacePath,
            ),
            Message::BrowseWorkspaceDir => Task::perform(
                async {
                    rfd::AsyncFileDialog::new()
                        .set_title("Selecione o diretório do dbeasy.json")
                        .pick_folder()
                        .await
                        .map(|f| f.path().to_path_buf())
                },
                Message::BrowsedWorkspacePath,
            ),
            Message::BrowsedWorkspacePath(picked) => {
                if let Some(p) = picked {
                    self.set_add_path(p.display().to_string());
                }
                Task::none()
            }
            Message::CancelCreateWorkspace => {
                if let Some(Modal::AddWorkspace { confirm, .. }) = &mut self.modal {
                    *confirm = false;
                }
                Task::none()
            }
            Message::SubmitAddWorkspace => self.add_workspace(),
            Message::AskRemoveWorkspace(index) => {
                if let Some(ws) = self.workspaces.get(index) {
                    let file = ws.desc.file.clone();
                    let names = self
                        .workspaces
                        .iter()
                        .filter(|w| w.desc.file == file)
                        .map(|w| w.desc.display_name.clone())
                        .collect();
                    self.modal = Some(Modal::RemoveWorkspace { file, names });
                }
                Task::none()
            }
            Message::ConfirmRemoveWorkspace => {
                let Some(Modal::RemoveWorkspace { file, .. }) = self.modal.take() else {
                    return Task::none();
                };
                self.settings.unregister(&file);
                if let Err(e) = self.settings.save(&self.settings_path) {
                    self.notify_error(e);
                }
                let prefix = format!("{}#", file.display());
                self.session
                    .workspaces
                    .retain(|id, _| !id.0.starts_with(&prefix));
                self.reload_workspaces()
            }
            Message::OpenWorkspaceSettings(index) => {
                let Some(ws) = self.workspaces.get(index) else {
                    return Task::none();
                };
                let Some(entry) = ws.desc.entry else {
                    let error = ws.desc.config.as_ref().err().map(ToString::to_string);
                    self.notify_error(format!(
                        "O arquivo não pôde ser lido como dbeasy.json; corrija-o num editor de texto{}",
                        error.map(|e| format!(": {e}")).unwrap_or_default()
                    ));
                    return Task::none();
                };
                match config_editor::Document::open(&ws.desc.file, entry) {
                    Ok(doc) => {
                        self.modal = Some(Modal::EditWorkspace {
                            doc: Box::new(doc),
                            error: None,
                            show_password: false,
                            test: None,
                        });
                        // Tira o foco do editor SQL sob o modal (Tab/Enter iriam para ele).
                        focus::first_in(view::SETTINGS_FIELDS)
                    }
                    Err(e) => {
                        self.notify_error(e);
                        Task::none()
                    }
                }
            }
            Message::SettingsField(..)
            | Message::SettingsSslMode(_)
            | Message::SettingsEnvKey(..)
            | Message::SettingsEnvValue(..)
            | Message::SettingsEnvAdd
            | Message::SettingsEnvRemove(_)
            | Message::SettingsReadOnly(_)
            | Message::SettingsRequireTransaction(_)
            | Message::SettingsTogglePassword
            | Message::SettingsTab(_)
            | Message::SettingsAddEntry
            | Message::SettingsRemoveEntry(_) => {
                if let Some(Modal::EditWorkspace {
                    doc,
                    error,
                    show_password,
                    test,
                }) = &mut self.modal
                {
                    *error = None;
                    if !matches!(message, Message::SettingsTogglePassword) {
                        // O resultado valia para a configuração anterior.
                        *test = None;
                    }
                    if matches!(message, Message::SettingsTab(_) | Message::SettingsAddEntry) {
                        // A senha de outra entrada começa oculta.
                        *show_password = false;
                    }
                    let form = doc.form_mut();
                    match message {
                        Message::SettingsField(field, value) => form.set(field, value),
                        Message::SettingsSslMode(choice) => {
                            form.set(config_editor::Field::SslMode, choice.0)
                        }
                        Message::SettingsEnvKey(row, key) => form.set_env_key(row, key),
                        Message::SettingsEnvValue(row, value) => form.set_env_value(row, value),
                        Message::SettingsEnvAdd => form.add_env(),
                        Message::SettingsEnvRemove(row) => form.remove_env(row),
                        Message::SettingsReadOnly(v) => form.set_read_only(v),
                        Message::SettingsRequireTransaction(v) => form.set_require_transaction(v),
                        Message::SettingsTogglePassword => *show_password = !*show_password,
                        Message::SettingsTab(tab) => doc.select(tab),
                        Message::SettingsAddEntry => doc.add_entry(),
                        Message::SettingsRemoveEntry(tab) => doc.remove_new_entry(tab),
                        _ => {}
                    }
                }
                Task::none()
            }
            Message::SettingsBrowseSqlDir => {
                let Some(Modal::EditWorkspace { doc, .. }) = &self.modal else {
                    return Task::none();
                };
                let tab = doc.active;
                let dir = doc.base_dir().to_path_buf();
                Task::perform(
                    async move {
                        rfd::AsyncFileDialog::new()
                            .set_title("Selecione o diretório dos arquivos SQL")
                            .set_directory(dir)
                            .pick_folder()
                            .await
                            .map(|f| f.path().to_path_buf())
                    },
                    move |picked| Message::SettingsPickedSqlDir(tab, picked),
                )
            }
            Message::SettingsPickedSqlDir(tab, picked) => {
                if let (
                    Some(Modal::EditWorkspace {
                        doc, error, test, ..
                    }),
                    Some(picked),
                ) = (&mut self.modal, picked)
                {
                    let value = doc.dir_value(&picked);
                    // A aba pode ter mudado enquanto o seletor estava aberto.
                    if let Some(form) = doc.forms.get_mut(tab) {
                        form.set(config_editor::Field::SqlFilesDir, value);
                        *error = None;
                        *test = None;
                    }
                }
                Task::none()
            }
            Message::SettingsSave => self.save_workspace_settings(),
            Message::SettingsTestConnection => self.test_settings_connection(),
            Message::SettingsTested(id, result) => {
                if let Some(Modal::EditWorkspace {
                    test: Some(test), ..
                }) = &mut self.modal
                    && test.id == id
                {
                    test.result = Some(result);
                }
                Task::none()
            }

            Message::TxBegin(id) => self.tx_command(&id, TxCommand::Begin),
            Message::TxRollback(id) => {
                self.modal = None;
                self.tx_command(&id, TxCommand::Rollback)
            }
            Message::TxCommit(id) => {
                self.modal = None;
                self.tx_command(&id, TxCommand::Commit)
            }
            Message::TxRequestCommit(id) => {
                let Some(ws) = self.ws_by_id(&id) else {
                    return Task::none();
                };
                let Some(handle) = ws.connection.clone() else {
                    return Task::none();
                };
                self.modal = Some(Modal::Commit {
                    ws: id.clone(),
                    summary: None,
                    grid: ws.grid_log.clone(),
                });
                Task::perform(async move { handle.summary().await }, move |r| {
                    Message::TxSummaryLoaded(id.clone(), r)
                })
            }
            Message::TxSummaryLoaded(id, result) => {
                if let Some(Modal::Commit { ws, summary, .. }) = &mut self.modal {
                    if *ws == id {
                        *summary = Some(result);
                    }
                }
                if let Some(ws) = self.ws_by_id(&id) {
                    ws.sync_tx();
                }
                Task::none()
            }
            Message::TxFinished(id, command, result) => {
                let Some(ws) = self.ws_by_id(&id) else {
                    return Task::none();
                };
                ws.tx_busy = false;
                ws.sync_tx();
                match result {
                    Ok(()) => {
                        if let Some(Modal::GridSave {
                            needs_transaction, ..
                        }) = &mut self.modal
                        {
                            *needs_transaction = command != TxCommand::Begin;
                        }
                        self.notify(match command {
                            TxCommand::Begin => "Transação iniciada",
                            TxCommand::Commit => "Transação confirmada (COMMIT)",
                            TxCommand::Rollback => "Transação revertida (ROLLBACK)",
                        });
                        let mut tasks = Vec::new();
                        if command == TxCommand::Begin {
                            if let Some(p) = self.pending_execution.take().filter(|p| p.ws == id) {
                                tasks.push(self.run_pending(p));
                            }
                        } else {
                            tasks.push(self.continue_after_tx());
                        }
                        Task::batch(tasks)
                    }
                    Err(e) => {
                        self.pending_execution = None;
                        self.after_tx = None;
                        self.closing_window = None;
                        self.notify_error(format!("Falha na transação: {e}"));
                        Task::none()
                    }
                }
            }
            Message::PendingTxCommit => {
                let Some(Modal::PendingTransaction { ws, closing, .. }) = self.modal.take() else {
                    return Task::none();
                };
                self.after_tx = Some(self.after_for(&ws, closing));
                self.update(Message::TxRequestCommit(ws))
            }
            Message::PendingTxRollback => {
                let Some(Modal::PendingTransaction { ws, closing, .. }) = self.modal.take() else {
                    return Task::none();
                };
                self.after_tx = Some(self.after_for(&ws, closing));
                self.tx_command(&ws, TxCommand::Rollback)
            }
            Message::ConfirmRunFile => {
                self.modal = None;
                let Some(id) = self.pending_execution.as_ref().map(|p| p.ws.clone()) else {
                    return Task::none();
                };
                self.tx_command(&id, TxCommand::Begin)
            }
            Message::SetExplorer(mode) => {
                let Some(ws) = self.active_ws_mut() else {
                    return Task::none();
                };
                ws.explorer = mode;
                if mode == ExplorerMode::Files && ws.files.entries.is_none() {
                    return self.list_files(None);
                }
                Task::none()
            }
            Message::FilterChanged(value) => {
                if let Some(ws) = self.active_ws_mut() {
                    ws.filter = value;
                }
                Task::none()
            }
            Message::ToggleExpanded(key) => {
                if let Some(ws) = self.active_ws_mut() {
                    if !ws.expanded.remove(&key) {
                        ws.expanded.insert(key);
                    }
                }
                Task::none()
            }
            Message::OpenRelation { schema, name } => self.open_relation(&schema, &name),
            Message::NavigateDir(dir) => self.list_files(Some(dir)),
            Message::FileMenu(entry) => {
                if let Some(ws) = self.active_ws_mut() {
                    ws.files.menu = Some(entry);
                }
                Task::none()
            }
            Message::CloseFileMenu => {
                if let Some(ws) = self.active_ws_mut() {
                    ws.files.menu = None;
                }
                Task::none()
            }
            Message::RenameDialog => {
                let Some(entry) = self.active_ws_mut().and_then(|w| w.files.menu.take()) else {
                    return Task::none();
                };
                let input_id = iced::widget::Id::unique();
                // O campo recebe o caminho relativo ao sqlFilesDir: editar só o
                // nome renomeia; mudar os diretórios move o item.
                let name = match self.files_location() {
                    Some((root, _)) => file_explorer::relative_to_root(&root, &entry.path),
                    None => entry.name.clone(),
                };
                self.modal = Some(Modal::RenameItem {
                    name,
                    entry,
                    error: None,
                    input_id: input_id.clone(),
                    busy: false,
                });
                Task::batch([
                    operation::focus(input_id.clone()),
                    operation::select_all(input_id),
                ])
            }
            Message::RenameName(value) => {
                if let Some(Modal::RenameItem { name, error, .. }) = &mut self.modal {
                    *name = value;
                    *error = None;
                }
                Task::none()
            }
            Message::RenameSubmit => {
                let Some((root, _)) = self.files_location() else {
                    return Task::none();
                };
                let id = self
                    .active_ws()
                    .map(|w| w.desc.id.clone())
                    .expect("workspace ativo");
                let Some(Modal::RenameItem {
                    entry,
                    name,
                    busy,
                    error,
                    ..
                }) = &mut self.modal
                else {
                    return Task::none();
                };
                if *busy {
                    return Task::none();
                }
                *busy = true;
                *error = None;
                let old = entry.path.clone();
                Task::perform(
                    file_explorer::rename(root, old.clone(), name.clone()),
                    move |r| Message::Renamed(id.clone(), old.clone(), r),
                )
            }
            Message::Renamed(id, old, result) => match result {
                Ok(new) => {
                    if matches!(self.modal, Some(Modal::RenameItem { .. })) {
                        self.modal = None;
                    }
                    let Some(ws) = self.ws_by_id(&id) else {
                        return Task::none();
                    };
                    // Abas e diretório atual acompanham o novo caminho.
                    let moved = |p: &PathBuf| p.strip_prefix(&old).ok().map(|rest| new.join(rest));
                    for tab in &mut ws.tabs {
                        if let TabKind::File { path, .. } = &mut tab.kind {
                            if let Some(updated) = moved(path) {
                                *path = updated;
                            }
                        }
                    }
                    if let Some(dir) = ws.files.current_dir.as_mut() {
                        if let Some(updated) =
                            std::fs::canonicalize(&*dir).ok().as_ref().and_then(moved)
                        {
                            *dir = updated;
                        }
                    }
                    let shown = display_path(ws, &new);
                    self.notify(format!("Renomeado/movido para {shown}"));
                    self.list_files(None)
                }
                Err(e) => {
                    if let Some(Modal::RenameItem { error, busy, .. }) = &mut self.modal {
                        *error = Some(e);
                        *busy = false;
                    } else {
                        self.notify_error(e);
                    }
                    Task::none()
                }
            },
            Message::DeleteDialog => {
                let Some(ws) = self.active_ws_mut() else {
                    return Task::none();
                };
                let Some(entry) = ws.files.menu.take() else {
                    return Task::none();
                };
                let affected: Vec<&Tab> = ws
                    .tabs
                    .iter()
                    .filter(|t| t.file_path().is_some_and(|p| p.starts_with(&entry.path)))
                    .collect();
                let open_tabs = affected.iter().map(|t| t.title()).collect();
                let unsaved = affected.iter().any(|t| t.dirty);
                let sql_files = if entry.is_dir {
                    file_explorer::count_sql_files(&entry.path)
                } else {
                    1
                };
                self.modal = Some(Modal::DeleteItem {
                    entry,
                    sql_files,
                    open_tabs,
                    unsaved,
                    error: None,
                    busy: false,
                });
                Task::none()
            }
            Message::DeleteConfirm => {
                let Some((root, _)) = self.files_location() else {
                    return Task::none();
                };
                let id = self
                    .active_ws()
                    .map(|w| w.desc.id.clone())
                    .expect("workspace ativo");
                let Some(Modal::DeleteItem {
                    entry, busy, error, ..
                }) = &mut self.modal
                else {
                    return Task::none();
                };
                if *busy {
                    return Task::none();
                }
                *busy = true;
                *error = None;
                Task::perform(
                    file_explorer::move_to_trash(root, entry.path.clone()),
                    move |r| Message::Deleted(id.clone(), r),
                )
            }
            Message::Deleted(id, result) => match result {
                Ok(removed) => {
                    if matches!(self.modal, Some(Modal::DeleteItem { .. })) {
                        self.modal = None;
                    }
                    let Some(ws) = self.ws_by_id(&id) else {
                        return Task::none();
                    };
                    // Fecha as abas dos arquivos removidos (o Scratch nunca é afetado).
                    let active_id = ws.tabs.get(ws.active_tab).map(|t| t.id);
                    ws.tabs
                        .retain(|t| !t.file_path().is_some_and(|p| p.starts_with(&removed)));
                    ws.active_tab = active_id
                        .and_then(|id| ws.tabs.iter().position(|t| t.id == id))
                        .unwrap_or(0);
                    let current_removed = ws
                        .files
                        .current_dir
                        .as_ref()
                        .and_then(|d| std::fs::canonicalize(d).ok())
                        .is_none_or(|d| d.starts_with(&removed));
                    if current_removed {
                        ws.files.current_dir = None;
                    }
                    let shown = display_path(ws, &removed);
                    self.notify(format!("{shown} movido para a lixeira"));
                    self.list_files(None)
                }
                Err(e) => {
                    if let Some(Modal::DeleteItem { error, busy, .. }) = &mut self.modal {
                        *error = Some(e);
                        *busy = false;
                    } else {
                        self.notify_error(e);
                    }
                    Task::none()
                }
            },
            Message::NewFileDialog => {
                let Some((root, dir)) = self.files_location() else {
                    self.notify_error("Nenhum diretório de arquivos SQL configurado (sqlFilesDir)");
                    return Task::none();
                };
                let root = std::fs::canonicalize(&root).unwrap_or(root);
                let dir = std::fs::canonicalize(&dir).unwrap_or(dir);
                let relative = dir
                    .strip_prefix(&root)
                    .map(|p| p.display().to_string())
                    .unwrap_or_default();
                let input_id = iced::widget::Id::unique();
                self.modal = Some(Modal::NewFile {
                    dir: if relative.is_empty() {
                        ".".into()
                    } else {
                        format!("./{relative}")
                    },
                    name: String::new(),
                    error: None,
                    input_id: input_id.clone(),
                    creating: false,
                });
                operation::focus(input_id)
            }
            Message::NewFileName(value) => {
                if let Some(Modal::NewFile { name, error, .. }) = &mut self.modal {
                    *name = value;
                    *error = None;
                }
                Task::none()
            }
            Message::NewFileSubmit => {
                let Some((root, dir)) = self.files_location() else {
                    return Task::none();
                };
                let Some(Modal::NewFile {
                    name,
                    creating,
                    error,
                    ..
                }) = &mut self.modal
                else {
                    return Task::none();
                };
                if *creating {
                    return Task::none();
                }
                *creating = true;
                *error = None;
                let name = name.clone();
                let id = self
                    .active_ws()
                    .map(|w| w.desc.id.clone())
                    .expect("workspace ativo");
                Task::perform(file_explorer::create_file(root, dir, name), move |r| {
                    Message::FileCreated(id.clone(), r)
                })
            }
            Message::FileCreated(id, result) => match result {
                Ok(path) => {
                    if matches!(self.modal, Some(Modal::NewFile { .. })) {
                        self.modal = None;
                    }
                    if self.active_ws().is_none_or(|w| w.desc.id != id) {
                        return Task::none();
                    }
                    let shown = self
                        .active_ws()
                        .map(|w| display_path(w, &path))
                        .unwrap_or_default();
                    self.notify(format!("Arquivo {shown} criado"));
                    // Atualiza a listagem e abre o arquivo; nada é executado.
                    Task::batch([self.list_files(None), self.open_file(path)])
                }
                Err(e) => {
                    if let Some(Modal::NewFile {
                        error, creating, ..
                    }) = &mut self.modal
                    {
                        *error = Some(e);
                        *creating = false;
                    } else {
                        self.notify_error(e);
                    }
                    Task::none()
                }
            },
            Message::FilesListed(id, dir, result) => {
                if let Some(ws) = self.ws_by_id(&id) {
                    if result.is_ok() || ws.files.current_dir.is_none() {
                        ws.files.current_dir = Some(dir);
                        ws.files.entries = Some(result);
                    } else if let Err(e) = result {
                        self.notify_error(e);
                    }
                }
                Task::none()
            }
            Message::OpenFile(path) => self.open_file(path),
            Message::FileRead { ws, tab, result } => {
                let Some(t) = self.tab_by_id(&ws, tab) else {
                    return Task::none();
                };
                t.loading_file = false;
                match result {
                    Ok(content) => {
                        t.editor = text_editor::Content::with_text(&content);
                        t.dirty = false;
                        t.text_rev += 1;
                        return schedule_diagnostics(ws, tab, t.text_rev);
                    }
                    Err(e) => {
                        t.run.error = Some(ErrorInfo {
                            text: e.clone(),
                            position: None,
                            line: None,
                            offset: None,
                            rev: t.text_rev,
                        });
                        self.notify_error(e);
                    }
                }
                Task::none()
            }
            Message::SqlFilesScanned(id, files) => {
                let is_active = self.active_ws().is_some_and(|w| w.desc.id == id);
                if let (true, Some(ws), Some(q)) = (
                    is_active,
                    self.active.map(|i| &self.workspaces[i]),
                    &mut self.quick_open,
                ) {
                    let root = ws
                        .desc
                        .config
                        .as_ref()
                        .ok()
                        .and_then(|c| c.sql_files_dir.clone());
                    let root = root.and_then(|r| std::fs::canonicalize(r).ok());
                    let mut fresh = QuickOpen::new(ws.catalog.as_ref(), &files, root.as_deref());
                    fresh.input_id = q.input_id.clone();
                    fresh.set_query(std::mem::take(&mut q.query));
                    *q = fresh;
                }
                self.sql_files.insert(id, files);
                Task::none()
            }

            Message::SelectTab(index) => {
                if let Some(ws) = self.active_ws_mut() {
                    if index < ws.tabs.len() {
                        ws.active_tab = index;
                        let tab = &ws.tabs[index];
                        let diagnose =
                            schedule_diagnostics(ws.desc.id.clone(), tab.id, tab.text_rev);
                        return Task::batch([diagnose, self.restore_blank_tab()]);
                    }
                }
                Task::none()
            }
            Message::CloseTab(index) => self.request_close_tab(index),
            Message::ConfirmCloseTab(choice) => {
                let Some(Modal::CloseTab { tab_ids, .. }) = self.modal.take() else {
                    return Task::none();
                };
                let mut tasks = Vec::new();
                for id in tab_ids {
                    let dirty = self
                        .active_ws()
                        .and_then(|w| w.tab_index(id).map(|i| w.tabs[i].dirty))
                        .unwrap_or(false);
                    match choice {
                        CloseChoice::Save if dirty => tasks.push(self.save_tab(id, true)),
                        _ => self.close_tab_by_id(id),
                    }
                }
                Task::batch(tasks)
            }
            Message::RightClickAt(position) => {
                self.last_right_click = position;
                Task::none()
            }
            Message::TabMenu(index) => {
                let Some(ws) = self.active_ws() else {
                    return Task::none();
                };
                let Some(tab) = ws.tabs.get(index) else {
                    return Task::none();
                };
                // Mantém o menu inteiro dentro da janela.
                let at = Point::new(
                    self.last_right_click
                        .x
                        .min(self.window_width - view::TAB_MENU_WIDTH - 4.0)
                        .max(0.0),
                    self.last_right_click.y,
                );
                self.tab_menu = Some(TabMenu { tab_id: tab.id, at });
                Task::none()
            }
            Message::CloseTabMenu => {
                self.tab_menu = None;
                Task::none()
            }
            Message::TabMenuAction(action) => {
                let Some(menu) = self.tab_menu.take() else {
                    return Task::none();
                };
                let Some(ws) = self.active_ws() else {
                    return Task::none();
                };
                let Some(index) = ws.tab_index(menu.tab_id) else {
                    return Task::none();
                };
                let ids = ws
                    .tabs
                    .iter()
                    .enumerate()
                    .filter(|(i, t)| !t.is_scratch() && action.includes(index, *i))
                    .map(|(_, t)| t.id)
                    .collect();
                self.request_close_tabs(ids)
            }
            Message::Editor(action) => {
                // Clicar ou digitar no editor tira o foco da grade; rolar, não.
                if !matches!(action, text_editor::Action::Scroll { .. }) {
                    if let Some(tab) = self.active_tab_mut() {
                        tab.grid_focused = false;
                    }
                }
                self.editor_action(action)
            }
            Message::EditorScrolled(viewport) => {
                if let Some(tab) = self.active_tab_mut() {
                    tab.editor_viewport = Some(viewport);
                }
                Task::none()
            }
            Message::GridScrolled(viewport) => {
                let Some(tab) = self.active_tab_mut() else {
                    return Task::none();
                };
                tab.grid_scroll.offset_x = viewport.absolute_offset().x;
                tab.grid_scroll.offset_y = viewport.absolute_offset().y;
                tab.grid_scroll.viewport_width = viewport.bounds().width;
                tab.grid_scroll.viewport_height = viewport.bounds().height;
                let header = operation::scroll_to(
                    tab.grid_header_id.clone(),
                    AbsoluteOffset {
                        x: Some(viewport.absolute_offset().x),
                        y: None,
                    },
                );
                let wants_more = data_grid::near_end(&viewport)
                    && tab.run.result.as_ref().is_some_and(|r| r.has_more)
                    && !tab.run.fetching;
                if wants_more {
                    Task::batch([header, self.fetch_more()])
                } else {
                    header
                }
            }
            Message::HeaderPressed(col) => {
                if let Some(tab) = self.active_tab_mut() {
                    tab.columns.pressed = Some(col);
                }
                Task::none()
            }
            Message::HeaderReleased(col) => {
                let pressed = self.active_tab_mut().and_then(|t| t.columns.pressed.take());
                match pressed {
                    Some(p) if p == col => self.toggle_sort(col),
                    Some(p) => {
                        self.move_column(p, col);
                        Task::none()
                    }
                    None => Task::none(),
                }
            }
            Message::HideColumn(col) => {
                self.hide_column(col);
                Task::none()
            }
            Message::RestoreColumns => self.restore_columns(),
            Message::SelectCell(row, col) => {
                self.commit_edit();
                if let Some(tab) = self.active_tab_mut() {
                    tab.selected = Some((row, col));
                    tab.row_selection.clear();
                    tab.columns.pressed = None;
                }
                self.focus_grid()
            }
            Message::SelectRow(row) => {
                self.commit_edit();
                let (ctrl, shift) = (self.modifiers.command(), self.modifiers.shift());
                if let Some(tab) = self.active_tab_mut() {
                    tab.row_selection.click(row, ctrl, shift);
                    tab.selected = None;
                    tab.columns.pressed = None;
                }
                self.focus_grid()
            }
            Message::ModifiersChanged(modifiers) => {
                self.modifiers = modifiers;
                Task::none()
            }
            Message::DeleteRowsDialog => {
                self.open_delete_rows();
                Task::none()
            }
            Message::DeleteRowsConfirm => self.delete_rows(),
            Message::RowsDeleted {
                ws,
                tab,
                rows,
                log,
                result,
            } => self.on_rows_deleted(ws, tab, rows, log, result),
            Message::CompletionMove(delta) => {
                if let Some(c) = self.active_tab_mut().and_then(|t| t.completion.as_mut()) {
                    let len = c.data.items.len().max(1) as i32;
                    if c.active {
                        c.selected = (c.selected as i32 + delta).rem_euclid(len) as usize;
                    } else {
                        // Primeira seta ativa a lista: ↓ vai ao primeiro item, ↑ ao último.
                        c.active = true;
                        c.selected = if delta > 0 { 0 } else { (len - 1) as usize };
                    }
                }
                Task::none()
            }
            Message::CompletionClose => {
                if let Some(tab) = self.active_tab_mut() {
                    tab.completion = None;
                }
                Task::none()
            }
            Message::CompletionPick(index) => {
                if let Some(c) = self.active_tab_mut().and_then(|t| t.completion.as_mut()) {
                    c.selected = index;
                }
                self.accept_completion()
            }
            Message::CompletionAccept => self.accept_completion(),
            Message::DiagnoseDue { ws, tab, rev } => {
                let Some(t) = self.tab_by_id(&ws, tab) else {
                    return Task::none();
                };
                if t.text_rev != rev {
                    return Task::none();
                }
                let text = t.editor.text();
                Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || crate::diagnostics::check(&text))
                            .await
                            .unwrap_or_default()
                    },
                    move |diagnostics| Message::Diagnosed {
                        ws: ws.clone(),
                        tab,
                        rev,
                        diagnostics,
                    },
                )
            }
            Message::Diagnosed {
                ws,
                tab,
                rev,
                diagnostics,
            } => {
                if let Some(t) = self.tab_by_id(&ws, tab) {
                    if t.text_rev == rev {
                        t.diagnostics = diagnostics;
                        t.diagnostics_rev = rev;
                    }
                }
                Task::none()
            }
            Message::StartEdit(row, col) => self.start_edit(row, col),
            Message::EditInput(value) => {
                if let Some(editor) = self.active_tab_mut().and_then(|t| t.editing.as_mut()) {
                    editor.text = value;
                }
                Task::none()
            }
            Message::CommitEdit => {
                self.commit_edit();
                Task::none()
            }
            Message::SetNull => {
                if let Some((row, col)) = self.editable_selection() {
                    let tab = self.active_tab_mut().expect("aba ativa");
                    tab.editing = None;
                    tab.pending.insert((row, col), None);
                }
                Task::none()
            }
            Message::RevertCell => {
                if let Some(tab) = self.active_tab_mut() {
                    if let Some(cell) = tab.selected {
                        tab.pending.remove(&cell);
                    }
                    tab.editing = None;
                }
                Task::none()
            }
            Message::RevertAll => {
                if let Some(tab) = self.active_tab_mut() {
                    tab.pending.clear();
                    tab.editing = None;
                }
                Task::none()
            }
            Message::GridSaveCopy => {
                let Some(Modal::GridSave { sql, .. }) = self.modal.take() else {
                    return Task::none();
                };
                self.notify("SQL copiado; as alterações continuam pendentes");
                iced::clipboard::write(sql)
            }
            Message::GridSaveExecute => {
                if let Some(Modal::GridSave { running, error, .. }) = &mut self.modal {
                    *running = true;
                    *error = None;
                }
                self.apply_pending()
            }
            Message::GridSaveDiscard => {
                let Some(Modal::GridSave { tab, .. }) = self.modal.take() else {
                    return Task::none();
                };
                if let Some(t) = self
                    .active_ws_mut()
                    .and_then(|w| w.tabs.iter_mut().find(|t| t.id == tab))
                {
                    t.pending.clear();
                    t.editing = None;
                }
                self.notify("Alterações pendentes descartadas");
                Task::none()
            }
            Message::EditsApplied {
                ws,
                tab,
                cells,
                log,
                result,
            } => self.on_edits_applied(ws, tab, cells, log, result),
            Message::FetchMore => self.fetch_more(),
            Message::Executed {
                ws,
                tab,
                exec_id,
                result,
            } => {
                let task = self.on_executed(ws.clone(), tab, exec_id, result);
                if let Some(w) = self.ws_by_id(&ws) {
                    refresh_edit_targets(w);
                }
                task
            }
            Message::Fetched {
                ws,
                tab,
                exec_id,
                result,
            } => {
                let Some(t) = self.tab_by_id(&ws, tab) else {
                    return Task::none();
                };
                if t.run.exec_id != exec_id {
                    return Task::none();
                }
                t.run.fetching = false;
                match result {
                    Ok(page) => {
                        if let Some(rs) = &mut t.run.result {
                            rs.rows.extend(page.rows);
                            rs.has_more = page.has_more;
                            rs.truncated = page.truncated;
                        }
                        if !page.has_more {
                            t.run.cursor = None;
                        }
                    }
                    Err(e) => {
                        if let Some(rs) = &mut t.run.result {
                            rs.has_more = false;
                        }
                        t.run.cursor = None;
                        self.notify_error(e.message);
                    }
                }
                Task::none()
            }
            Message::Saved {
                ws,
                tab,
                result,
                then_close,
            } => {
                let Some(t) = self.tab_by_id(&ws, tab) else {
                    return Task::none();
                };
                match result {
                    Ok(()) => {
                        t.dirty = false;
                        let title = t.title();
                        self.notify(format!("{title} salvo"));
                        if then_close {
                            self.close_tab_by_id(tab);
                        }
                    }
                    Err(e) => self.notify_error(e),
                }
                Task::none()
            }
            Message::Cancelled(result) => {
                match result {
                    Ok(()) => self.notify("Cancelamento solicitado ao servidor"),
                    Err(e) => self.notify_error(format!("Falha ao cancelar: {e}")),
                }
                Task::none()
            }

            Message::Command(command) => self.run_command(command),
            Message::KeyPressed {
                key,
                physical,
                modifiers,
                captured,
            } => self.on_key(key, physical, modifiers, captured),
            Message::QuickOpenQuery(query) => {
                if let Some(q) = &mut self.quick_open {
                    q.set_query(query);
                }
                Task::none()
            }
            Message::QuickOpenSubmit => {
                let target = self
                    .quick_open
                    .as_ref()
                    .and_then(|q| q.current())
                    .map(|i| i.target.clone());
                self.quick_open = None;
                match target {
                    Some(quick_open::Target::Relation { schema, name, .. }) => {
                        self.open_relation(&schema, &name)
                    }
                    Some(quick_open::Target::File(path)) => self.open_file(path),
                    None => Task::none(),
                }
            }
            Message::QuickOpenPick(index) => {
                if let Some(q) = &mut self.quick_open {
                    q.selected = index;
                }
                self.update(Message::QuickOpenSubmit)
            }
            Message::ShowShortcuts => {
                self.modal = Some(Modal::Shortcuts);
                Task::none()
            }
            Message::ToggleAppSettings => {
                self.app_settings_open = !self.app_settings_open;
                if !self.app_settings_open {
                    self.theme_preview = None;
                }
                Task::none()
            }
            Message::ToggleWorkspaces => {
                self.workspaces_open = !self.workspaces_open;
                Task::none()
            }
            Message::PreviewTheme(theme) => {
                self.theme_preview = (theme != self.settings.theme_choice()).then_some(theme);
                Task::none()
            }
            Message::SystemTheme(mode) => {
                self.system_mode = mode;
                Task::none()
            }
            Message::RevertTheme => {
                self.theme_preview = None;
                Task::none()
            }
            Message::ApplyTheme => {
                if let Some(theme) = self.theme_preview.take() {
                    self.settings.theme = match theme {
                        ThemeChoice::System => None,
                        ThemeChoice::Named(theme) => Some(theme.to_string()),
                    };
                    if let Err(e) = self.settings.save(&self.settings_path) {
                        self.notice = Some(Notice {
                            text: e,
                            is_error: true,
                        });
                    }
                }
                Task::none()
            }
            Message::ShowAbout => {
                self.modal = Some(Modal::About {
                    license: false,
                    error: None,
                });
                Task::none()
            }
            Message::AboutToggleLicense => {
                if let Some(Modal::About { license, .. }) = &mut self.modal {
                    *license = !*license;
                }
                Task::none()
            }
            Message::AboutOpenSource => {
                self.about_open(crate::about::open_external(crate::about::SOURCE_URL));
                Task::none()
            }
            Message::AboutOpenThirdParty => {
                self.about_open(crate::about::open_third_party());
                Task::none()
            }
            Message::CloseModal => {
                self.modal = None;
                self.after_tx = None;
                self.closing_window = None;
                self.pending_execution = None;
                self.quick_open = None;
                Task::none()
            }

            Message::DragStart(target) => {
                self.drag = Some(match target {
                    DragTarget::Sidebar => Drag::Sidebar,
                    DragTarget::Split => Drag::Split { start: None },
                });
                Task::none()
            }
            Message::DragMove(position) => {
                match self.drag {
                    Some(Drag::Sidebar) => {
                        self.sidebar_width =
                            (position.x - view::WORKSPACE_STRIP_WIDTH).clamp(180.0, 700.0);
                    }
                    Some(Drag::Split { start: None }) => {
                        let ratio = self.active_tab_mut().map_or(0.55, |t| t.split);
                        self.drag = Some(Drag::Split {
                            start: Some((position.y, ratio)),
                        });
                    }
                    Some(Drag::Split {
                        start: Some((y0, ratio0)),
                    }) => {
                        let height = (self.window_height - view::CHROME_HEIGHT).max(200.0);
                        if let Some(tab) = self.active_tab_mut() {
                            tab.split = (ratio0 + (position.y - y0) / height).clamp(0.1, 0.9);
                        }
                    }
                    None => {}
                }
                Task::none()
            }
            Message::DragEnd => {
                self.drag = None;
                self.session_dirty = true;
                Task::none()
            }
            Message::WindowResized(size) => {
                self.window_width = size.width;
                self.window_height = size.height;
                Task::none()
            }

            Message::CloseRequested(id) => {
                if let Some(ws) = self
                    .workspaces
                    .iter()
                    .find(|w| w.tx != TxState::Idle && w.connection.is_some())
                {
                    self.closing_window = Some(id);
                    self.modal = Some(Modal::PendingTransaction {
                        ws: ws.desc.id.clone(),
                        name: ws.desc.display_name.clone(),
                        failed: ws.tx == TxState::Failed,
                        closing: true,
                    });
                    return Task::none();
                }
                // Gravação síncrona: a janela fecha logo em seguida.
                if let Err(e) = self.snapshot().save(&self.session_path) {
                    eprintln!("dbeasy: {e}");
                }
                window::close(id).chain(iced::exit())
            }
            Message::Autosave => {
                if !self.session_dirty {
                    return Task::none();
                }
                self.session_dirty = false;
                let session = self.snapshot();
                let path = self.session_path.clone();
                Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || session.save(&path))
                            .await
                            .map_err(|e| e.to_string())?
                    },
                    Message::SessionSaved,
                )
            }
            Message::SessionSaved(result) => {
                if let Err(e) = result {
                    self.notify_error(e);
                }
                Task::none()
            }
            Message::DismissNotice => {
                self.notice = None;
                Task::none()
            }
        }
    }

    // ------------------------------------------------------------------
    // Workspaces

    /// Recarrega todos os `dbeasy.json` registrados, preservando o estado em
    /// execução dos workspaces que continuam existindo.
    fn reload_workspaces(&mut self) -> Task<Message> {
        self.reload_workspaces_keeping(None)
    }

    /// Como `reload_workspaces`; `edited` é o arquivo gravado pelo editor de
    /// configurações: as entradas dele são casadas pela posição no array, e
    /// mantêm abas, conexão e sessão mesmo que o `name` (parte do id) tenha
    /// mudado. O editor nunca remove nem reordena entradas.
    fn reload_workspaces_keeping(&mut self, edited: Option<&std::path::Path>) -> Task<Message> {
        let mut active_id = self.active_ws().map(|w| w.desc.id.clone());
        for ws in &self.workspaces {
            self.session
                .workspaces
                .insert(ws.desc.id.clone(), ws.snapshot());
        }

        let mut previous = std::mem::take(&mut self.workspaces);
        let mut tasks = Vec::new();
        for desc in workspace::load_all(&self.settings.workspaces) {
            let found = if edited == Some(desc.file.as_path()) && desc.entry.is_some() {
                previous
                    .iter()
                    .position(|w| w.desc.file == desc.file && w.desc.entry == desc.entry)
            } else {
                previous.iter().position(|w| w.desc.id == desc.id)
            };
            match found {
                Some(i) => {
                    let mut ws = previous.swap_remove(i);
                    if ws.desc.id != desc.id {
                        let old = ws.desc.id.clone();
                        self.session.workspaces.remove(&old);
                        if let Some(files) = self.sql_files.remove(&old) {
                            self.sql_files.insert(desc.id.clone(), files);
                        }
                        if active_id.as_ref() == Some(&old) {
                            active_id = Some(desc.id.clone());
                        }
                    }
                    ws.desc = desc;
                    self.workspaces.push(ws);
                }
                None => {
                    let saved = self.session.workspaces.get(&desc.id).cloned();
                    let ws = Workspace::new(desc, saved.as_ref(), &mut self.next_tab_id);
                    // Arquivos SQL sem alterações são relidos do disco.
                    for tab in ws.tabs.iter().filter(|t| t.loading_file) {
                        if let Some(path) = tab.file_path() {
                            tasks.push(read_file_task(ws.desc.id.clone(), tab.id, path.clone()));
                        }
                    }
                    self.workspaces.push(ws);
                }
            }
        }
        // Workspaces descartados liberam conexão e resultados aqui (drop).
        drop(previous);

        self.active = active_id
            .and_then(|id| self.workspaces.iter().position(|w| w.desc.id == id))
            .or(if self.workspaces.is_empty() {
                None
            } else {
                Some(0)
            });
        Task::batch(tasks)
    }

    /// Grava as entradas do editor de configurações e recarrega os
    /// workspaces. Uma conexão aberta continua com a configuração antiga até
    /// ser refeita (pode haver transação em andamento).
    fn save_workspace_settings(&mut self) -> Task<Message> {
        let Some(Modal::EditWorkspace { doc, error, .. }) = &mut self.modal else {
            return Task::none();
        };
        if let Err(e) = doc.save() {
            *error = Some(e);
            return Task::none();
        }
        let file = doc.file.clone();
        let added = doc.forms.iter().filter(|f| f.index.is_none()).count();
        self.modal = None;

        let before: HashMap<usize, _> = self
            .workspaces
            .iter()
            .filter(|w| w.desc.file == file)
            .filter_map(|w| Some((w.desc.entry?, w.desc.config.clone())))
            .collect();
        let reload = self.reload_workspaces_keeping(Some(&file));

        let mut tasks = vec![reload];
        let mut relist_files = false;
        let mut reconnect = Vec::new();
        let active = self.active;
        for (i, ws) in self.workspaces.iter_mut().enumerate() {
            if ws.desc.file != file {
                continue;
            }
            let Some(before) = ws.desc.entry.and_then(|e| before.get(&e)) else {
                continue;
            };
            let changed = match (before, &ws.desc.config) {
                (Ok(a), Ok(b)) => a != b,
                (a, b) => a.is_ok() != b.is_ok(),
            };
            let old_dir = before.as_ref().ok().and_then(|c| c.sql_files_dir.clone());
            let new_dir = ws
                .desc
                .config
                .as_ref()
                .ok()
                .and_then(|c| c.sql_files_dir.clone());
            if old_dir != new_dir {
                ws.files = state::FilesState::default();
                self.sql_files.remove(&ws.desc.id);
                relist_files |= active == Some(i) && ws.explorer == ExplorerMode::Files;
            }
            if changed && ws.connection.is_some() {
                reconnect.push(ws.desc.display_name.clone());
            }
        }
        if relist_files {
            tasks.push(self.list_files(None));
        }
        let mut message = match added {
            0 => format!("Configurações de {} salvas.", file.display()),
            1 => format!(
                "Configurações salvas; 1 workspace adicionado a {}.",
                file.display()
            ),
            n => format!(
                "Configurações salvas; {n} workspaces adicionados a {}.",
                file.display()
            ),
        };
        if !reconnect.is_empty() {
            message.push_str(&format!(
                " Desconecte e conecte de novo para aplicá-las à conexão aberta de {}.",
                reconnect.join(", ")
            ));
        }
        self.notify(message);
        Task::batch(tasks)
    }

    /// Testa a configuração da aba exibida (inclusive alterações ainda não
    /// salvas) numa conexão própria, fechada ao terminar.
    fn test_settings_connection(&mut self) -> Task<Message> {
        let Some(Modal::EditWorkspace { doc, test, .. }) = &mut self.modal else {
            return Task::none();
        };
        let config = match doc.form().resolved() {
            Ok(config) => config,
            Err(e) => {
                *test = Some(ConnTest {
                    id: 0,
                    result: Some(Err(e)),
                });
                return Task::none();
            }
        };
        self.next_test_id += 1;
        let id = self.next_test_id;
        *test = Some(ConnTest { id, result: None });
        Task::perform(connection::probe(config), move |result| {
            Message::SettingsTested(id, result)
        })
    }

    fn select_workspace(&mut self, index: usize) -> Task<Message> {
        if index >= self.workspaces.len() {
            return Task::none();
        }
        self.active = Some(index);
        self.quick_open = None;
        let ws = &self.workspaces[index];
        let tab = &ws.tabs[ws.active_tab];
        let mut tasks = vec![schedule_diagnostics(
            ws.desc.id.clone(),
            tab.id,
            tab.text_rev,
        )];
        let ws = &self.workspaces[index];
        if ws.status == Status::Disconnected && ws.desc.config.is_ok() {
            tasks.push(self.connect(index));
        }
        let ws = &self.workspaces[index];
        if ws.explorer == ExplorerMode::Files && ws.files.entries.is_none() {
            tasks.push(self.list_files(None));
        }
        Task::batch(tasks)
    }

    fn cycle_workspace(&mut self, delta: isize) -> Task<Message> {
        let len = self.workspaces.len();
        if len == 0 {
            return Task::none();
        }
        let current = self.active.unwrap_or(0) as isize;
        let next = (current + delta).rem_euclid(len as isize) as usize;
        self.select_workspace(next)
    }

    fn connect(&mut self, index: usize) -> Task<Message> {
        let Some(ws) = self.workspaces.get_mut(index) else {
            return Task::none();
        };
        let config = match &ws.desc.config {
            Ok(config) => config.clone(),
            Err(e) => {
                ws.status = Status::Error;
                ws.connection_error = Some(format!("configuração inválida: {e}"));
                return Task::none();
            }
        };
        ws.status = Status::Connecting;
        ws.connection_error = None;
        ws.catalog = None;
        let id = ws.desc.id.clone();
        Task::perform(connection::connect(config), move |result| {
            Message::Connected(id.clone(), result)
        })
    }

    fn load_catalog(&mut self, id: &WorkspaceId) -> Task<Message> {
        let Some(ws) = self.ws_by_id(id) else {
            return Task::none();
        };
        let Some(handle) = ws.connection.clone() else {
            return Task::none();
        };
        ws.catalog_loading = true;
        let id = id.clone();
        Task::perform(async move { handle.catalog().await }, move |r| {
            Message::CatalogLoaded(id.clone(), r)
        })
    }

    fn set_add_path(&mut self, value: String) {
        if let Some(Modal::AddWorkspace {
            path,
            error,
            target,
            confirm,
        }) = &mut self.modal
        {
            *target = workspace::add::resolve(&value);
            *path = value;
            *error = None;
            *confirm = false;
        }
    }

    /// Registra o `dbeasy.json` informado. Se ele ainda não existe, pede
    /// confirmação, cria o arquivo e abre o editor gráfico da entrada criada.
    fn add_workspace(&mut self) -> Task<Message> {
        let Some(Modal::AddWorkspace {
            error,
            target,
            confirm,
            ..
        }) = &mut self.modal
        else {
            return Task::none();
        };
        let (file, created) = match target {
            workspace::add::Target::Invalid(message) => {
                *error = Some(message.clone());
                return Task::none();
            }
            workspace::add::Target::Create(_) if !*confirm => {
                *confirm = true;
                return Task::none();
            }
            workspace::add::Target::Create(file) => {
                if let Err(e) = workspace::add::create(file) {
                    *error = Some(e);
                    *confirm = false;
                    return Task::none();
                }
                (file.clone(), true)
            }
            workspace::add::Target::Existing(file) => (file.clone(), false),
        };
        // Um arquivo recém-criado pode já estar registrado (foi apagado do
        // disco depois de registrado): basta recarregar.
        if !self.settings.register(&file) && !created {
            *error = Some("Este arquivo já está registrado.".into());
            return Task::none();
        }
        self.modal = None;
        if let Err(e) = self.settings.save(&self.settings_path) {
            self.notify_error(e);
        }
        let before = self.workspaces.len();
        let reload = self.reload_workspaces();
        let canonical = crate::fsutil::canonical_or_normalized(&file);
        let select = if self.workspaces.len() > before {
            match self
                .workspaces
                .iter()
                .position(|w| w.desc.file == canonical)
            {
                Some(i) => self.select_workspace(i),
                None => Task::none(),
            }
        } else {
            Task::none()
        };
        let mut focus = Task::none();
        if created {
            match config_editor::Document::open(&canonical, 0) {
                Ok(doc) => {
                    self.modal = Some(Modal::EditWorkspace {
                        doc: Box::new(doc),
                        error: None,
                        show_password: false,
                        test: None,
                    });
                    focus = focus::first_in(view::SETTINGS_FIELDS);
                }
                Err(e) => self.notify_error(e),
            }
        }
        Task::batch([reload, select, focus])
    }

    // ------------------------------------------------------------------
    // Exploradores e abertura de abas

    /// `sqlFilesDir` do workspace ativo e o diretório exibido no explorador.
    fn files_location(&self) -> Option<(PathBuf, PathBuf)> {
        let ws = self.active_ws()?;
        let root = ws.desc.config.as_ref().ok()?.sql_files_dir.clone()?;
        let dir = ws.files.current_dir.clone().unwrap_or_else(|| root.clone());
        Some((root, dir))
    }

    fn list_files(&mut self, dir: Option<PathBuf>) -> Task<Message> {
        let Some(ws) = self.active_ws_mut() else {
            return Task::none();
        };
        let Some(root) = ws
            .desc
            .config
            .as_ref()
            .ok()
            .and_then(|c| c.sql_files_dir.clone())
        else {
            return Task::none();
        };
        let dir = dir
            .or_else(|| ws.files.current_dir.clone())
            .unwrap_or_else(|| root.clone());
        let id = ws.desc.id.clone();
        Task::perform(file_explorer::list_dir(root, dir.clone()), move |r| {
            Message::FilesListed(id.clone(), dir.clone(), r)
        })
    }

    fn open_relation(&mut self, schema: &str, name: &str) -> Task<Message> {
        let mut next_id = self.next_tab_id;
        let Some(ws) = self.active_ws_mut() else {
            return Task::none();
        };
        let existing = ws.tabs.iter().position(
            |t| matches!(&t.kind, TabKind::Table { schema: s, name: n } if s == schema && n == name),
        );
        if let Some(index) = existing {
            ws.active_tab = index;
            return Task::none();
        }
        let Some(relation) = ws.catalog.as_ref().and_then(|c| c.find(schema, name)) else {
            self.notify_error(format!("{schema}.{name} não está nos metadados carregados"));
            return Task::none();
        };
        let sql = metadata::select_for(relation);
        let mut tab = Tab::new(
            TabKind::Table {
                schema: schema.into(),
                name: name.into(),
            },
            sql,
            &mut next_id,
        );
        tab.generated = true;
        ws.tabs.push(tab);
        ws.active_tab = ws.tabs.len() - 1;
        self.next_tab_id = next_id;
        // Decisão registrada: o SELECT gerado pela aplicação é executado ao
        // abrir a tabela; SQL do usuário nunca é executado automaticamente.
        self.execute(true)
    }

    fn open_file(&mut self, path: PathBuf) -> Task<Message> {
        let mut next_id = self.next_tab_id;
        let Some(ws) = self.active_ws_mut() else {
            return Task::none();
        };
        let root = ws
            .desc
            .config
            .as_ref()
            .ok()
            .and_then(|c| c.sql_files_dir.clone());
        let Some(path) = root
            .as_deref()
            .and_then(|r| file_explorer::resolve_within(r, &path))
        else {
            self.notify_error("o arquivo está fora do sqlFilesDir do workspace");
            return Task::none();
        };
        if let Some(index) = ws.tabs.iter().position(|t| t.file_path() == Some(&path)) {
            ws.active_tab = index;
            return Task::none();
        }
        let mut tab = Tab::new(
            TabKind::File {
                path: path.clone(),
                dirty: false,
            },
            String::new(),
            &mut next_id,
        );
        tab.loading_file = true;
        let task = read_file_task(ws.desc.id.clone(), tab.id, path);
        ws.tabs.push(tab);
        ws.active_tab = ws.tabs.len() - 1;
        self.next_tab_id = next_id;
        task
    }

    fn request_close_tab(&mut self, index: usize) -> Task<Message> {
        let Some(ws) = self.active_ws() else {
            return Task::none();
        };
        let Some(tab) = ws.tabs.get(index) else {
            return Task::none();
        };
        if tab.is_scratch() {
            self.notify("O Scratch não pode ser fechado");
            return Task::none();
        }
        let id = tab.id;
        self.request_close_tabs(vec![id])
    }

    /// Fecha as abas indicadas do workspace ativo. Se alguma tiver alterações
    /// não salvas, pergunta antes uma única vez por todas; cancelar não fecha
    /// nenhuma.
    fn request_close_tabs(&mut self, tab_ids: Vec<u64>) -> Task<Message> {
        let Some(ws) = self.active_ws() else {
            return Task::none();
        };
        let dirty: Vec<String> = ws
            .tabs
            .iter()
            .filter(|t| t.dirty && tab_ids.contains(&t.id))
            .map(|t| t.title())
            .collect();
        if !dirty.is_empty() {
            self.modal = Some(Modal::CloseTab { tab_ids, dirty });
            return Task::none();
        }
        for id in tab_ids {
            self.close_tab_by_id(id);
        }
        Task::none()
    }

    fn close_tab_by_id(&mut self, tab_id: u64) {
        let Some(ws) = self.active_ws_mut() else {
            return;
        };
        let Some(index) = ws.tab_index(tab_id) else {
            return;
        };
        if index == 0 {
            return;
        }
        ws.tabs.remove(index);
        if ws.active_tab >= index {
            ws.active_tab = ws.active_tab.saturating_sub(1);
        }
    }

    // ------------------------------------------------------------------
    // Editor e execução

    fn editor_action(&mut self, action: text_editor::Action) -> Task<Message> {
        let Some(tab) = self.active_tab_mut() else {
            return Task::none();
        };
        if tab.loading_file {
            return Task::none();
        }
        if let text_editor::Action::Scroll { lines } = action {
            return operation::scroll_by(
                tab.editor_scroll_id.clone(),
                AbsoluteOffset {
                    x: 0.0,
                    y: lines as f32 * sql_editor::LINE_HEIGHT,
                },
            );
        }
        let is_edit = action.is_edit();
        let inserted = match &action {
            text_editor::Action::Edit(text_editor::Edit::Insert(c)) => Some(*c),
            _ => None,
        };
        let deleting = matches!(
            action,
            text_editor::Action::Edit(text_editor::Edit::Backspace | text_editor::Edit::Delete)
        );
        tab.editor.perform(action);
        let line = tab.editor.cursor().position.line;
        let scroll = match sql_editor::scroll_to_show(line, tab.editor_viewport.as_ref()) {
            Some(y) => operation::scroll_to(
                tab.editor_scroll_id.clone(),
                AbsoluteOffset {
                    x: None,
                    y: Some(y),
                },
            ),
            None => Task::none(),
        };
        if !is_edit {
            tab.completion = None;
            return scroll;
        }

        // Autocomplete automático: ao digitar parte de um nome ou `.`.
        let open = tab.completion.is_some();
        let diagnose = self.text_changed();
        match inserted {
            Some(c) if c.is_alphanumeric() || c == '_' || c == '.' => {
                self.refresh_completion(false)
            }
            _ if deleting && open => self.refresh_completion(false),
            _ => {
                if let Some(tab) = self.active_tab_mut() {
                    tab.completion = None;
                }
            }
        }
        Task::batch([scroll, diagnose])
    }

    /// Marca o texto da aba ativa como alterado e agenda o diagnóstico.
    fn text_changed(&mut self) -> Task<Message> {
        let Some(ws) = self.active_ws_mut() else {
            return Task::none();
        };
        let ws_id = ws.desc.id.clone();
        let index = ws.active_tab;
        let tab = &mut ws.tabs[index];
        if tab.file_path().is_some() {
            tab.dirty = true;
        }
        tab.generated = false;
        tab.pristine = None;
        tab.text_rev += 1;
        schedule_diagnostics(ws_id, tab.id, tab.text_rev)
    }

    /// Recalcula a lista de sugestões para o cursor da aba ativa.
    fn refresh_completion(&mut self, forced: bool) {
        let Some(ws) = self.active_ws_mut() else {
            return;
        };
        let empty = Catalog::default();
        let catalog = ws.catalog.as_ref().unwrap_or(&empty);
        let index = ws.active_tab;
        let tab = &mut ws.tabs[index];
        let text = tab.editor.text();
        let cursor = tab.editor.cursor();
        let offset = split::byte_offset(&text, cursor.position.line, cursor.position.column);
        let previous = tab
            .completion
            .as_ref()
            .and_then(|c| c.data.items.get(c.selected).map(|i| i.label.clone()));
        // Continua ativa enquanto o usuário digita; abrir com Ctrl+Espaço ativa.
        let active = forced || tab.completion.as_ref().is_some_and(|c| c.active);
        tab.completion = crate::completion::complete(&text, offset, catalog, forced).map(|data| {
            let selected = previous
                .and_then(|label| data.items.iter().position(|i| i.label == label))
                .unwrap_or(0);
            state::CompletionState {
                data,
                selected,
                active,
            }
        });
    }

    fn accept_completion(&mut self) -> Task<Message> {
        let Some(tab) = self.active_tab_mut() else {
            return Task::none();
        };
        let Some(state) = tab.completion.take() else {
            return Task::none();
        };
        let Some(item) = state.data.items.get(state.selected) else {
            return Task::none();
        };
        let text = tab.editor.text();
        let cursor = tab.editor.cursor();
        let offset = split::byte_offset(&text, cursor.position.line, cursor.position.column);
        let typed = text
            .get(state.data.start..offset)
            .map_or(0, |t| t.chars().count());
        for _ in 0..typed {
            tab.editor
                .perform(text_editor::Action::Select(text_editor::Motion::Left));
        }
        tab.editor
            .perform(text_editor::Action::Edit(text_editor::Edit::Paste(
                std::sync::Arc::new(item.insert.clone()),
            )));
        let focus = operation::focus(tab.editor_id.clone());
        let diagnose = self.text_changed();
        Task::batch([focus, diagnose])
    }

    /// Executa a seleção; sem seleção, o script inteiro (`whole`) ou a
    /// instrução sob o cursor.
    fn execute(&mut self, whole: bool) -> Task<Message> {
        let Some(ws_index) = self.active else {
            return Task::none();
        };
        let ws = &mut self.workspaces[ws_index];
        if ws.connection.is_none() {
            let connecting = ws.status == Status::Connecting;
            self.notify_error(if connecting {
                "Aguarde a conexão terminar para executar"
            } else {
                "Workspace desconectado: conecte antes de executar"
            });
            return Task::none();
        }
        let tab_index = ws.active_tab;
        let tab = &mut ws.tabs[tab_index];
        if tab.run.running {
            self.notify("Já existe uma execução em andamento nesta aba (Ctrl+. cancela)");
            return Task::none();
        }

        let text = tab.editor.text();
        let cursor = tab.editor.cursor();
        let cursor_offset = split::byte_offset(&text, cursor.position.line, cursor.position.column);
        let (sql, base, statements) = match cursor.selection {
            Some(other) => {
                let other = split::byte_offset(&text, other.line, other.column);
                let (a, b) = (cursor_offset.min(other), cursor_offset.max(other));
                let sql = text[a..b].to_string();
                let statements = split::statements(&sql);
                (sql, a, statements)
            }
            None if whole => {
                let statements = split::statements(&text);
                (text, 0, statements)
            }
            None => match split::statement_at(&text, cursor_offset) {
                Some(range) => {
                    let sql = text[range.clone()].to_string();
                    let len = sql.len();
                    (sql, range.start, vec![0..len])
                }
                None => (String::new(), 0, Vec::new()),
            },
        };
        if statements.is_empty() {
            self.notify("Nada para executar");
            return Task::none();
        }

        // Arquivo salvo em workspace com transação obrigatória: confirma e
        // executa dentro de uma transação nova (decisão em usabilidade/transacoes.md).
        let ws = &self.workspaces[ws_index];
        let tab = &ws.tabs[ws.active_tab];
        let saved_file = tab.file_path().is_some() && !tab.dirty;
        let writes = statements
            .iter()
            .any(|r| crate::query::classify::check_read_only(&sql[r.clone()]).is_err());
        if saved_file && writes && ws.require_transaction() && ws.tx == TxState::Idle {
            self.pending_execution = Some(PendingExecution {
                ws: ws.desc.id.clone(),
                tab: tab.id,
                sql,
                base,
                statements,
            });
            self.modal = Some(Modal::RunFileInTransaction { title: tab.title() });
            return Task::none();
        }
        self.start_execution(sql, base, statements, false)
    }

    /// Refaz o resultado da aba ativa restaurada em branco, se for uma aba de
    /// tabela cujo último resultado veio de um `SELECT`. Sem conexão pronta,
    /// fica para quando ela chegar.
    fn restore_blank_tab(&mut self) -> Task<Message> {
        let Some(ws) = self.active_ws_mut() else {
            return Task::none();
        };
        let connected = ws.connection.is_some();
        let Some(tab) = ws.tabs.get_mut(ws.active_tab) else {
            return Task::none();
        };
        if tab.run.running {
            return Task::none();
        }
        let Some(range) = tab.restorable_query() else {
            return Task::none();
        };
        if !connected {
            tab.rerun_on_connect = true;
            return Task::none();
        }
        self.execute_range(range, false)
    }

    /// Executa a instrução que está em `range` no editor da aba ativa.
    fn execute_range(&mut self, range: std::ops::Range<usize>, from_grid: bool) -> Task<Message> {
        let Some(tab) = self.active_tab_mut() else {
            return Task::none();
        };
        let sql = tab.editor.text()[range.clone()].to_string();
        let len = sql.len();
        self.start_execution(sql, range.start, vec![0..len], from_grid)
    }

    fn start_execution(
        &mut self,
        sql: String,
        base: usize,
        statements: Vec<std::ops::Range<usize>>,
        from_grid: bool,
    ) -> Task<Message> {
        let Some(ws) = self.active_ws_mut() else {
            return Task::none();
        };
        let ws_id = ws.desc.id.clone();
        let Some(handle) = ws.connection.clone() else {
            return Task::none();
        };
        let index = ws.active_tab;
        let tab = &mut ws.tabs[index];
        if !tab.pending.is_empty() {
            self.notify_error(
                "Há alterações pendentes na grade: revise com Ctrl+S ou reverta antes de executar de novo",
            );
            return Task::none();
        }
        tab.blank = None;
        tab.rerun_on_connect = false;
        tab.run.running = true;
        tab.run.exec_id += 1;
        tab.run.base_offset = base;
        tab.run.error = None;
        tab.run.cursor = None;
        tab.run.sent = sql.clone();
        tab.run.from_grid = from_grid;
        // Arquivos salvos podem conter BEGIN/COMMIT; SQL manual não.
        let options = ExecOptions {
            allow_tx_control: tab.file_path().is_some() && !tab.dirty,
        };
        let exec_id = tab.run.exec_id;
        let tab_id = tab.id;
        Task::perform(
            async move { handle.execute_with(sql, statements, options).await },
            move |result| Message::Executed {
                ws: ws_id.clone(),
                tab: tab_id,
                exec_id,
                result,
            },
        )
    }

    fn on_executed(
        &mut self,
        ws: WorkspaceId,
        tab: u64,
        exec_id: u64,
        result: Result<Outcome, QueryError>,
    ) -> Task<Message> {
        let Some(t) = self.tab_by_id(&ws, tab) else {
            return Task::none();
        };
        if t.run.exec_id != exec_id {
            return Task::none();
        }
        t.run.running = false;
        match result {
            Ok(outcome) => {
                t.run.messages = outcome.messages;
                t.run.elapsed_ms = Some(outcome.elapsed_ms);
                t.run.cursor = outcome.cursor;
                if let Some(result) = outcome.result {
                    t.widths = data_grid::column_widths(&result);
                    layout_after_execution(t, &result, outcome.result_statement);
                    t.run.result = Some(result);
                    t.selected = None;
                    t.row_selection.clear();
                    t.grid_scroll.offset_y = 0.0;
                    return operation::scroll_to(
                        t.grid_body_id.clone(),
                        AbsoluteOffset {
                            x: Some(0.0),
                            y: Some(0.0),
                        },
                    );
                }
                Task::none()
            }
            Err(error) => {
                // O último resultado válido é mantido (spec §5.1).
                let text = t.editor.text();
                let offset = error
                    .offset
                    .or(error.statement.as_ref().map(|r| r.start))
                    .map(|o| t.run.base_offset + o);
                let mut info = ErrorInfo {
                    text: describe_error(&error),
                    position: None,
                    line: None,
                    offset: error
                        .offset
                        .map(|o| t.run.base_offset + o)
                        .filter(|o| *o <= text.len()),
                    rev: t.text_rev,
                };
                if let Some(offset) = offset.filter(|o| *o <= text.len()) {
                    info.position = Some(split::line_col(&text, offset));
                    let (line, column) = split::line_and_byte_column(&text, offset);
                    info.line = Some(line);
                    t.editor.move_to(text_editor::Cursor {
                        position: text_editor::Position { line, column },
                        selection: None,
                    });
                }
                t.run.error = Some(info);
                Task::none()
            }
        }
    }

    fn fetch_more(&mut self) -> Task<Message> {
        let Some(ws) = self.active_ws_mut() else {
            return Task::none();
        };
        let ws_id = ws.desc.id.clone();
        let Some(handle) = ws.connection.clone() else {
            return Task::none();
        };
        let index = ws.active_tab;
        let tab = &mut ws.tabs[index];
        let Some(cursor) = tab.run.cursor else {
            return Task::none();
        };
        if tab.run.fetching || tab.run.running {
            return Task::none();
        }
        tab.run.fetching = true;
        let (tab_id, exec_id) = (tab.id, tab.run.exec_id);
        Task::perform(
            async move { handle.fetch_more(cursor).await },
            move |result| Message::Fetched {
                ws: ws_id.clone(),
                tab: tab_id,
                exec_id,
                result,
            },
        )
    }

    fn save_tab(&mut self, tab_id: u64, then_close: bool) -> Task<Message> {
        let Some(ws) = self.active_ws_mut() else {
            return Task::none();
        };
        let ws_id = ws.desc.id.clone();
        let Some(tab) = ws.tabs.iter().find(|t| t.id == tab_id) else {
            return Task::none();
        };
        let Some(path) = tab.file_path().cloned() else {
            return Task::none();
        };
        let contents = tab.editor.text();
        Task::perform(file_explorer::write_file(path, contents), move |result| {
            Message::Saved {
                ws: ws_id.clone(),
                tab: tab_id,
                result,
                then_close,
            }
        })
    }

    // ------------------------------------------------------------------
    // Comandos

    fn run_command(&mut self, command: Command) -> Task<Message> {
        if self.modal.is_some() {
            return Task::none();
        }
        match command {
            Command::ExecuteCurrent => self.execute(false),
            Command::ExecuteScript => self.execute(true),
            Command::Save if self.has_pending() => self.open_grid_save(),
            Command::SaveDirect => {
                if self.has_pending() {
                    self.commit_edit();
                    self.apply_pending()
                } else {
                    self.notify("Nenhuma alteração pendente na grade");
                    Task::none()
                }
            }
            Command::Save => {
                let Some(ws) = self.active_ws() else {
                    return Task::none();
                };
                let tab = &ws.tabs[ws.active_tab];
                if tab.file_path().is_some() {
                    let id = tab.id;
                    self.save_tab(id, false)
                } else {
                    self.notify(
                        "Nada para salvar: o SQL desta aba é guardado automaticamente na sessão",
                    );
                    Task::none()
                }
            }
            Command::CancelQuery => {
                let Some(ws) = self.active_ws() else {
                    return Task::none();
                };
                let running = ws.tabs[ws.active_tab].run.running;
                match (running, ws.connection.clone()) {
                    (true, Some(handle)) => {
                        Task::perform(async move { handle.cancel().await }, Message::Cancelled)
                    }
                    _ => Task::none(),
                }
            }
            Command::QuickOpen => self.open_quick_open(),
            Command::Complete => {
                self.refresh_completion(true);
                if self
                    .active_ws()
                    .is_some_and(|w| w.tabs[w.active_tab].completion.is_none())
                {
                    self.notify("Nenhuma sugestão para esta posição");
                }
                Task::none()
            }
            Command::NextWorkspace => self.cycle_workspace(1),
            Command::PreviousWorkspace => self.cycle_workspace(-1),
            Command::CloseTab => {
                let index = self.active_ws().map(|w| w.active_tab);
                match index {
                    Some(i) => self.request_close_tab(i),
                    None => Task::none(),
                }
            }
            Command::RefreshMetadata => {
                let Some(ws) = self.active_ws() else {
                    return Task::none();
                };
                let id = ws.desc.id.clone();
                let files = if ws.explorer == ExplorerMode::Files {
                    self.list_files(None)
                } else {
                    Task::none()
                };
                Task::batch([self.load_catalog(&id), files])
            }
        }
    }

    fn open_quick_open(&mut self) -> Task<Message> {
        let Some(ws) = self.active_ws() else {
            return Task::none();
        };
        let id = ws.desc.id.clone();
        let root = ws
            .desc
            .config
            .as_ref()
            .ok()
            .and_then(|c| c.sql_files_dir.clone());
        let canonical_root = root.as_ref().and_then(|r| std::fs::canonicalize(r).ok());
        let files = self.sql_files.get(&id).cloned().unwrap_or_default();
        let q = QuickOpen::new(ws.catalog.as_ref(), &files, canonical_root.as_deref());
        let focus = operation::focus(q.input_id.clone());
        self.quick_open = Some(q);
        // Atualiza a lista de arquivos em segundo plano.
        let scan = match root {
            Some(root) => Task::perform(file_explorer::scan_sql_files(root), move |files| {
                Message::SqlFilesScanned(id.clone(), files)
            }),
            None => Task::none(),
        };
        Task::batch([focus, scan])
    }

    fn on_key(
        &mut self,
        key: Key,
        physical: key::Physical,
        modifiers: Modifiers,
        captured: bool,
    ) -> Task<Message> {
        if let Some(q) = &mut self.quick_open {
            match key.as_ref() {
                Key::Named(key::Named::ArrowDown) => q.move_selection(1),
                Key::Named(key::Named::ArrowUp) => q.move_selection(-1),
                Key::Named(key::Named::Escape) => self.quick_open = None,
                _ => {}
            }
            return Task::none();
        }
        if let Some(Modal::EditWorkspace { doc, .. }) = &self.modal {
            match key.as_ref() {
                // Tab/Shift+Tab percorrem só os campos do diálogo.
                Key::Named(key::Named::Tab) if !modifiers.control() => {
                    return focus::cycle(view::SETTINGS_FIELDS, !modifiers.shift());
                }
                Key::Named(key::Named::Enter) if modifiers.is_empty() && doc.can_save() => {
                    return self.save_workspace_settings();
                }
                _ => {}
            }
        }
        // Qualquer tecla fecha o menu de contexto das abas; Esc só fecha.
        if self.tab_menu.take().is_some() && matches!(key.as_ref(), Key::Named(key::Named::Escape))
        {
            return Task::none();
        }
        if let Some(modal) = &self.modal {
            // Esc não descarta alterações feitas no editor de configurações.
            let keeps_edits = matches!(modal, Modal::EditWorkspace { doc, .. } if doc.is_dirty());
            if matches!(key.as_ref(), Key::Named(key::Named::Escape)) && !keeps_edits {
                self.modal = None;
            }
            return Task::none();
        }
        if matches!(key.as_ref(), Key::Named(key::Named::Escape)) {
            if let Some(ws) = self.active_ws_mut() {
                if ws.files.menu.take().is_some() {
                    return Task::none();
                }
            }
            if let Some(tab) = self.active_tab_mut() {
                if tab.editing.take().is_some() {
                    return Task::none();
                }
            }
        }
        // Com a grade focada, setas navegam e Del exclui as linhas
        // selecionadas; nunca quando um campo (editor SQL, editor de célula)
        // capturou a tecla.
        let grid_focused = self
            .active_ws()
            .is_some_and(|w| w.tabs[w.active_tab].grid_focused);
        if !captured && grid_focused && !modifiers.command() && !modifiers.alt() {
            let arrow = match key.as_ref() {
                Key::Named(key::Named::ArrowUp) => Some(data_grid::Arrow::Up),
                Key::Named(key::Named::ArrowDown) => Some(data_grid::Arrow::Down),
                Key::Named(key::Named::ArrowLeft) => Some(data_grid::Arrow::Left),
                Key::Named(key::Named::ArrowRight) => Some(data_grid::Arrow::Right),
                _ => None,
            };
            if let Some(arrow) = arrow {
                return self.navigate_grid(arrow, modifiers.shift());
            }
            if matches!(key.as_ref(), Key::Named(key::Named::Delete)) && !modifiers.shift() {
                let has_rows = self
                    .active_ws()
                    .is_some_and(|w| !w.tabs[w.active_tab].row_selection.rows.is_empty());
                if has_rows {
                    self.open_delete_rows();
                } else {
                    self.notify(
                        "Selecione linhas pelo número (Shift+↑/↓ estende) para excluí-las com Del",
                    );
                }
                return Task::none();
            }
        }
        if captured {
            // Atalhos da aplicação continuam valendo dentro do editor de célula.
            let editing = self
                .active_ws()
                .is_some_and(|w| w.tabs[w.active_tab].editing.is_some());
            if !editing {
                return Task::none();
            }
        }
        match commands::lookup(&key, physical, modifiers) {
            Some(command) => self.run_command(command),
            None => Task::none(),
        }
    }

    // ------------------------------------------------------------------
    // Sessão

    fn snapshot(&self) -> Session {
        let mut session = self.session.clone();
        for ws in &self.workspaces {
            session.workspaces.insert(ws.desc.id.clone(), ws.snapshot());
        }
        session.active_workspace = self.active_ws().map(|w| w.desc.id.clone());
        session.layout.sidebar_width = self.sidebar_width;
        session
    }
}

/// Interações que fecham o menu de contexto do explorador de arquivos.
fn closes_file_menu(message: &Message) -> bool {
    matches!(
        message,
        Message::Editor(_)
            | Message::SelectTab(_)
            | Message::CloseTab(_)
            | Message::TabMenu(_)
            | Message::SelectWorkspace(_)
            | Message::SetExplorer(_)
            | Message::NavigateDir(_)
            | Message::OpenFile(_)
            | Message::OpenRelation { .. }
            | Message::Command(_)
            | Message::DragStart(_)
            | Message::StartEdit(..)
            | Message::SelectCell(..)
            | Message::SelectRow(_)
            | Message::HeaderPressed(_)
            | Message::NewFileDialog
            | Message::FilesListed(..)
    )
}

/// Caminho de um arquivo para exibição: relativo ao `sqlFilesDir` do
/// workspace quando estiver dentro dele, senão o caminho completo.
pub(crate) fn display_path(ws: &Workspace, path: &std::path::Path) -> String {
    let root = ws
        .desc
        .config
        .as_ref()
        .ok()
        .and_then(|c| c.sql_files_dir.as_ref())
        .and_then(|r| std::fs::canonicalize(r).ok());
    match root.as_ref().and_then(|r| path.strip_prefix(r).ok()) {
        Some(relative) => relative.display().to_string(),
        None => path.display().to_string(),
    }
}

fn read_file_task(ws: WorkspaceId, tab: u64, path: PathBuf) -> Task<Message> {
    Task::perform(file_explorer::read_file(path), move |result| {
        Message::FileRead {
            ws: ws.clone(),
            tab,
            result,
        }
    })
}

fn describe_error(error: &QueryError) -> String {
    let mut text = error.message.clone();
    if let Some(code) = &error.code {
        text = format!("[{code}] {text}");
    }
    if let Some(detail) = &error.detail {
        text.push_str(&format!("\nDetalhe: {detail}"));
    }
    if let Some(hint) = &error.hint {
        text.push_str(&format!("\nDica: {hint}"));
    }
    text
}

// ----------------------------------------------------------------------
// Transformações pela grade (spec §5.2–5.5)

impl App {
    fn toggle_sort(&mut self, col: usize) -> Task<Message> {
        let Some(tab) = self.active_tab_mut() else {
            return Task::none();
        };
        if tab.run.running {
            return Task::none();
        }
        let Some(result) = &tab.run.result else {
            return Task::none();
        };
        let name = result.columns[col].name.clone();
        let unique = result.columns.iter().filter(|c| c.name == name).count() == 1;
        let next = match &tab.columns.sort {
            Some((n, Direction::Asc)) if *n == name => Some(Direction::Desc),
            Some((n, Direction::Desc)) if *n == name => None,
            _ => Some(Direction::Asc),
        };
        let expr = if unique {
            metadata::quote_ident(&name)
        } else if let Some(item) = tab.columns.item_of.get(col).copied().flatten() {
            (item + 1).to_string()
        } else {
            self.notify_error(format!(
                "Ordenação não aplicada: há mais de uma coluna chamada {name}"
            ));
            return Task::none();
        };

        let outcome = current_source(tab).and_then(|src| {
            transform::set_order(&src.text, &expr, next).map(|sql| (src.range.clone(), sql))
        });
        match outcome {
            Ok((_, sql)) => {
                let range = replace_source(tab, sql);
                tab.columns.sort = next.map(|d| (name, d));
                // Colunas ocultadas só na grade continuam ocultas após reexecutar.
                tab.columns.pending_hidden = hidden_names(tab);
                self.execute_range(range, true)
            }
            Err(reason) => {
                let origin = origin_label(tab);
                self.notify_error(format!("Ordenação não aplicada ({origin}): {reason}"));
                Task::none()
            }
        }
    }

    fn hide_column(&mut self, col: usize) {
        let Some(tab) = self.active_tab_mut() else {
            return;
        };
        let Some(result) = &tab.run.result else {
            return;
        };
        let name = result.columns[col].name.clone();
        if tab.columns.order.len() <= 1 {
            self.notify("A grade precisa manter ao menos uma coluna visível");
            return;
        }
        tab.columns.order.retain(|c| *c != col);

        let outcome = current_source(tab).and_then(|src| {
            let item =
                tab.columns.item_of.get(col).copied().flatten().ok_or(
                    "não foi possível relacionar a coluna a um item do SELECT".to_string(),
                )?;
            transform::remove_item(&src.text, item).map(|sql| (item, src.text.clone(), sql))
        });
        match outcome {
            Ok((item, before, sql)) => {
                tab.pristine.get_or_insert(before);
                replace_source(tab, sql);
                for slot in tab.columns.item_of.iter_mut() {
                    *slot = match *slot {
                        Some(i) if i == item => None,
                        Some(i) if i > item => Some(i - 1),
                        other => other,
                    };
                }
                self.notify(format!("Coluna {name} ocultada e removida do SQL"));
            }
            Err(reason) => {
                let origin = origin_label(tab);
                self.notify_error(format!(
                    "Coluna {name} ocultada só na grade; SQL não alterado ({origin}): {reason}"
                ));
            }
        }
    }

    fn move_column(&mut self, from: usize, to: usize) {
        let Some(tab) = self.active_tab_mut() else {
            return;
        };
        let (Some(a), Some(b)) = (
            tab.columns.order.iter().position(|c| *c == from),
            tab.columns.order.iter().position(|c| *c == to),
        ) else {
            return;
        };
        let moved = tab.columns.order.remove(a);
        tab.columns.order.insert(b, moved);

        let outcome = current_source(tab).and_then(|src| {
            let shape = transform::analyze(&src.text)?;
            let items: Option<Vec<usize>> = tab
                .columns
                .order
                .iter()
                .map(|c| tab.columns.item_of.get(*c).copied().flatten())
                .collect();
            let items = items
                .filter(|items| items.len() == shape.items.len())
                .ok_or("há colunas ocultadas só na grade que continuam no SELECT".to_string())?;
            transform::reorder_items(&src.text, &items).map(|sql| (src.text.clone(), sql))
        });
        match outcome {
            Ok((before, sql)) => {
                tab.pristine.get_or_insert(before);
                replace_source(tab, sql);
                let order = tab.columns.order.clone();
                for (position, c) in order.into_iter().enumerate() {
                    tab.columns.item_of[c] = Some(position);
                }
            }
            Err(reason) => {
                let origin = origin_label(tab);
                self.notify_error(format!(
                    "Colunas reordenadas só na grade; SQL não alterado ({origin}): {reason}"
                ));
            }
        }
    }

    fn restore_columns(&mut self) -> Task<Message> {
        let Some(tab) = self.active_tab_mut() else {
            return Task::none();
        };
        let Some(result) = &tab.run.result else {
            return Task::none();
        };
        let columns = result.columns.len();
        tab.columns.order = (0..columns).collect();
        tab.columns.pending_hidden.clear();

        let Some(pristine) = tab.pristine.take() else {
            return Task::none();
        };
        if let Err(reason) = current_source(tab) {
            self.notify_error(format!("Colunas restauradas só na grade: {reason}"));
            return Task::none();
        }
        // Reaplica a ordenação atual sobre o SQL original, se possível.
        let sql = match &tab.columns.sort {
            Some((name, d)) => {
                transform::set_order(&pristine, &metadata::quote_ident(name), Some(*d))
                    .unwrap_or(pristine)
            }
            None => pristine,
        };
        let items = transform::analyze(&sql).map(|s| s.items.len()).ok();
        let range = replace_source(tab, sql);
        if items == Some(columns) {
            tab.columns.item_of = (0..columns).map(Some).collect();
            Task::none()
        } else {
            // O resultado atual não tem as colunas restauradas: reexecuta.
            self.execute_range(range, true)
        }
    }
}

/// Instrução de origem do resultado, se o editor ainda tiver o mesmo texto
/// naquela posição.
fn current_source(tab: &Tab) -> Result<state::Source, String> {
    let source = tab
        .source
        .clone()
        .ok_or("o resultado não veio de uma única instrução identificável")?;
    let text = tab.editor.text();
    if text.get(source.range.clone()) != Some(source.text.as_str()) {
        return Err("o SQL foi alterado desde a execução; execute novamente".into());
    }
    Ok(source)
}

/// Troca a instrução de origem no editor por `sql`, preservando o restante do
/// texto e a posição do cursor. Retorna a nova faixa da instrução.
fn replace_source(tab: &mut Tab, sql: String) -> std::ops::Range<usize> {
    let Some(source) = tab.source.as_mut() else {
        return 0..0;
    };
    let text = tab.editor.text();
    let updated = format!(
        "{}{sql}{}",
        &text[..source.range.start],
        &text[source.range.end..]
    );
    let cursor = tab.editor.cursor();
    tab.editor = text_editor::Content::with_text(&updated);
    let line = cursor
        .position
        .line
        .min(tab.editor.line_count().saturating_sub(1));
    let column = tab
        .editor
        .line(line)
        .map_or(0, |l| cursor.position.column.min(l.text.len()));
    tab.editor.move_to(text_editor::Cursor {
        position: text_editor::Position { line, column },
        selection: None,
    });
    source.range = source.range.start..source.range.start + sql.len();
    source.text = sql;
    let range = source.range.clone();
    if tab.file_path().is_some() {
        tab.dirty = true;
    }
    range
}

fn hidden_names(tab: &Tab) -> Vec<String> {
    let Some(result) = &tab.run.result else {
        return Vec::new();
    };
    (0..result.columns.len())
        .filter(|c| !tab.columns.order.contains(c))
        .map(|c| result.columns[c].name.clone())
        .collect()
}

fn origin_label(tab: &Tab) -> &'static str {
    if tab.generated {
        "consulta gerada pela aplicação"
    } else {
        "consulta editada pelo usuário"
    }
}

/// Atualiza origem e disposição das colunas quando chega um novo resultado.
fn layout_after_execution(
    tab: &mut Tab,
    result: &crate::query::ResultSet,
    statement: Option<std::ops::Range<usize>>,
) {
    tab.source = statement.and_then(|r| {
        let text = tab.run.sent.get(r.clone())?.to_string();
        let base = tab.run.base_offset;
        Some(state::Source {
            range: base + r.start..base + r.end,
            text,
        })
    });
    let shape = tab
        .source
        .as_ref()
        .and_then(|s| transform::analyze(&s.text).ok());
    let items = shape
        .as_ref()
        .filter(|s| !s.has_wildcard && !s.set_operation)
        .map(|s| s.items.len());
    tab.columns.reset(result.columns.len(), items);

    // Ordenação continua indicada se a consulta ainda tem ORDER BY e a coluna existe.
    let keep_sort = tab.run.from_grid
        || (shape.as_ref().is_some_and(|s| s.order_by.is_some())
            && tab
                .columns
                .sort
                .as_ref()
                .is_some_and(|(name, _)| result.columns.iter().any(|c| &c.name == name)));
    if !keep_sort {
        tab.columns.sort = None;
    }

    let hidden = std::mem::take(&mut tab.columns.pending_hidden);
    if !hidden.is_empty() {
        let keep: Vec<usize> = tab
            .columns
            .order
            .iter()
            .copied()
            .filter(|c| !hidden.contains(&result.columns[*c].name))
            .collect();
        if !keep.is_empty() {
            tab.columns.order = keep;
        }
    }
    if !tab.run.from_grid {
        tab.pristine = None;
    }
}

// ----------------------------------------------------------------------
// Transações

impl App {
    fn tx_command(&mut self, id: &WorkspaceId, command: TxCommand) -> Task<Message> {
        let Some(ws) = self.ws_by_id(id) else {
            return Task::none();
        };
        let Some(handle) = ws.connection.clone() else {
            return Task::none();
        };
        if ws.tx_busy {
            return Task::none();
        }
        ws.tx_busy = true;
        let id = id.clone();
        Task::perform(async move { handle.transaction(command).await }, move |r| {
            Message::TxFinished(id.clone(), command, r)
        })
    }

    fn after_for(&self, ws: &WorkspaceId, closing: bool) -> AfterTx {
        match (closing, self.closing_window) {
            (true, Some(window)) => AfterTx::Close(window),
            _ => AfterTx::Disconnect(ws.clone()),
        }
    }

    /// Retoma o fechamento ou a desconexão depois que a transação foi
    /// encerrada.
    fn continue_after_tx(&mut self) -> Task<Message> {
        match self.after_tx.take() {
            Some(AfterTx::Close(window)) => self.update(Message::CloseRequested(window)),
            Some(AfterTx::Disconnect(id)) => {
                if let Some(index) = self.workspaces.iter().position(|w| w.desc.id == id) {
                    return self.update(Message::Disconnect(index));
                }
                Task::none()
            }
            None => Task::none(),
        }
    }

    fn run_pending(&mut self, pending: PendingExecution) -> Task<Message> {
        let still_there = self
            .active_ws()
            .is_some_and(|w| w.desc.id == pending.ws && w.tabs[w.active_tab].id == pending.tab);
        if !still_there {
            self.notify("Transação iniciada; execute o arquivo novamente");
            return Task::none();
        }
        self.start_execution(pending.sql, pending.base, pending.statements, false)
    }
}

// ----------------------------------------------------------------------
// Edição de dados pela grade (spec §6)

/// Recalcula, para cada aba com resultado, se ele é editável.
fn refresh_edit_targets(ws: &mut Workspace) {
    let read_only = ws.read_only();
    for tab in &mut ws.tabs {
        tab.edit_target = tab.run.result.as_ref().map(|result| {
            if read_only {
                return Err("workspace somente leitura".to_string());
            }
            match &ws.catalog {
                Some(catalog) => data_edit::edit_target(result, catalog),
                None => Err("metadados ainda não carregados".to_string()),
            }
        });
        if !matches!(tab.edit_target, Some(Ok(_))) {
            tab.editing = None;
        }
    }
}

impl App {
    fn has_pending(&self) -> bool {
        self.active_ws().is_some_and(|w| {
            !w.tabs[w.active_tab].pending.is_empty() || w.tabs[w.active_tab].editing.is_some()
        })
    }

    /// Célula selecionada, se ela puder ser editada.
    fn editable_selection(&self) -> Option<(usize, usize)> {
        let ws = self.active_ws()?;
        let tab = &ws.tabs[ws.active_tab];
        let (row, col) = tab.selected?;
        let Some(Ok(target)) = &tab.edit_target else {
            return None;
        };
        target.columns.get(col)?.as_ref()?;
        Some((row, col))
    }

    fn start_edit(&mut self, row: usize, col: usize) -> Task<Message> {
        self.commit_edit();
        let Some(tab) = self.active_tab_mut() else {
            return Task::none();
        };
        tab.selected = Some((row, col));
        let reason = match &tab.edit_target {
            None => Some("execute a consulta primeiro".to_string()),
            Some(Err(reason)) => Some(reason.clone()),
            Some(Ok(target)) if target.columns.get(col).is_none_or(|c| c.is_none()) => {
                Some("a coluna não corresponde a uma coluna da tabela".to_string())
            }
            Some(Ok(_)) => None,
        };
        if let Some(reason) = reason {
            self.notify_error(format!("Resultado não editável: {reason}"));
            return Task::none();
        }
        let Some(result) = &tab.run.result else {
            return Task::none();
        };
        let text = match tab.pending.get(&(row, col)) {
            Some(Some(v)) => v.clone(),
            Some(None) => String::new(),
            None => data_edit::edit_text(&result.rows[row][col]),
        };
        let input_id = iced::widget::Id::unique();
        tab.editing = Some(state::CellEditor {
            row,
            col,
            text,
            input_id: input_id.clone(),
        });
        Task::batch([
            operation::focus(input_id.clone()),
            operation::select_all(input_id),
        ])
    }

    /// Confirma a célula em edição como alteração pendente. Um texto igual ao
    /// valor original desfaz a pendência.
    fn commit_edit(&mut self) {
        let Some(tab) = self.active_tab_mut() else {
            return;
        };
        let Some(editor) = tab.editing.take() else {
            return;
        };
        let Some(result) = &tab.run.result else {
            return;
        };
        let original = &result.rows[editor.row][editor.col];
        if !original.is_null() && data_edit::edit_text(original) == editor.text {
            tab.pending.remove(&(editor.row, editor.col));
        } else {
            tab.pending
                .insert((editor.row, editor.col), Some(editor.text));
        }
    }

    fn build_pending_updates(
        &mut self,
    ) -> Result<(Vec<data_edit::RowUpdate>, data_edit::EditTarget), String> {
        self.commit_edit();
        let ws = self.active_ws().ok_or("nenhum workspace ativo")?;
        let tab = &ws.tabs[ws.active_tab];
        let result = tab.run.result.as_ref().ok_or("sem resultado")?;
        let target = match &tab.edit_target {
            Some(Ok(target)) => target.clone(),
            Some(Err(reason)) => return Err(reason.clone()),
            None => return Err("resultado não editável".into()),
        };
        Ok((
            data_edit::build_updates(&target, result, &tab.pending)?,
            target,
        ))
    }

    fn open_grid_save(&mut self) -> Task<Message> {
        match self.build_pending_updates() {
            Ok((updates, _)) => {
                let ws = self.active_ws().expect("workspace ativo");
                let sql = updates
                    .iter()
                    .map(|u| u.display.as_str())
                    .collect::<Vec<_>>()
                    .join("\n\n");
                self.modal = Some(Modal::GridSave {
                    tab: ws.tabs[ws.active_tab].id,
                    sql,
                    error: None,
                    running: false,
                    needs_transaction: ws.require_transaction() && ws.tx != TxState::Active,
                });
            }
            Err(e) => self.notify_error(format!("Não foi possível preparar as alterações: {e}")),
        }
        Task::none()
    }

    fn apply_pending(&mut self) -> Task<Message> {
        let (updates, target) = match self.build_pending_updates() {
            Ok(built) => built,
            Err(e) => {
                self.edit_failed(format!("Não foi possível preparar as alterações: {e}"));
                return Task::none();
            }
        };
        let Some(ws) = self.active_ws() else {
            return Task::none();
        };
        let Some(handle) = ws.connection.clone() else {
            self.edit_failed("Workspace desconectado".into());
            return Task::none();
        };
        let tab = &ws.tabs[ws.active_tab];
        let result = tab.run.result.as_ref().expect("resultado");
        let table = format!("{}.{}", target.schema, target.table);

        let mut log = Vec::new();
        let mut statements = Vec::new();
        let mut cells = Vec::new();
        for u in &updates {
            let key: Vec<String> = target
                .key
                .iter()
                .map(|k| {
                    format!(
                        "{}={}",
                        result.columns[*k].name,
                        result.rows[u.row][*k].display()
                    )
                })
                .collect();
            let changes: Vec<String> = u
                .columns
                .iter()
                .map(|c| {
                    let before = &result.rows[u.row][*c];
                    let before = if before.is_null() {
                        "NULL".into()
                    } else {
                        before.display()
                    };
                    let after = tab.pending[&(u.row, *c)]
                        .clone()
                        .unwrap_or_else(|| "NULL".into());
                    format!("{}: {before} → {after}", result.columns[*c].name)
                })
                .collect();
            log.push(format!(
                "{table} [{}] {}",
                key.join(", "),
                changes.join("; ")
            ));
            statements.push(connection::EditStatement {
                sql: u.sql.clone(),
                params: u.params.clone(),
                label: format!("UPDATE {table} [{}]", key.join(", ")),
            });
            cells.push((u.row, u.columns.clone()));
        }

        let ws_id = ws.desc.id.clone();
        let tab_id = tab.id;
        Task::perform(
            async move { handle.apply_edits(statements).await },
            move |result| Message::EditsApplied {
                ws: ws_id.clone(),
                tab: tab_id,
                cells: cells.clone(),
                log: log.clone(),
                result,
            },
        )
    }

    /// Erro ao gravar: aparece no popup, se aberto; senão na barra de status.
    /// As alterações pendentes são mantidas.
    fn edit_failed(&mut self, message: String) {
        if let Some(Modal::GridSave { error, running, .. }) = &mut self.modal {
            *error = Some(message);
            *running = false;
        } else {
            self.notify_error(message);
        }
    }

    fn on_edits_applied(
        &mut self,
        ws: WorkspaceId,
        tab: u64,
        cells: Vec<(usize, Vec<usize>)>,
        log: Vec<String>,
        result: Result<Vec<Vec<crate::query::Value>>, QueryError>,
    ) -> Task<Message> {
        let Some(w) = self.ws_by_id(&ws) else {
            return Task::none();
        };
        w.sync_tx();
        let in_transaction = w.tx == TxState::Active;
        let written = match result {
            Ok(written) => written,
            Err(e) => {
                self.edit_failed(format!("Alterações não gravadas: {}", describe_error(&e)));
                return Task::none();
            }
        };
        if in_transaction {
            w.grid_log.extend(log);
        }
        let Some(t) = w.tabs.iter_mut().find(|t| t.id == tab) else {
            return Task::none();
        };
        let rows = cells.len();
        if let Some(rs) = &mut t.run.result {
            for ((row, cols), values) in cells.into_iter().zip(written) {
                for (col, value) in cols.into_iter().zip(values) {
                    rs.rows[row][col] = value;
                    t.pending.remove(&(row, col));
                }
            }
        }
        if matches!(self.modal, Some(Modal::GridSave { .. })) {
            self.modal = None;
        }
        self.notify(if in_transaction {
            format!("{rows} linha(s) alterada(s) na transação; use Confirmar… para gravar")
        } else {
            format!("{rows} linha(s) gravada(s)")
        });
        Task::none()
    }

    /// Dá o foco do teclado à grade da aba ativa. O editor SQL precisa perder
    /// o foco explicitamente: o clique na grade é capturado por ela e não
    /// chega ao editor (o `stack` que o envolve para em eventos capturados).
    fn focus_grid(&mut self) -> Task<Message> {
        let Some(tab) = self.active_tab_mut() else {
            return Task::none();
        };
        tab.grid_focused = true;
        if tab.editing.is_some() {
            return Task::none();
        }
        iced::advanced::widget::operate(iced::advanced::widget::operation::focusable::unfocus())
    }

    /// Setas na grade focada: move a seleção e rola até ela.
    fn navigate_grid(&mut self, arrow: data_grid::Arrow, shift: bool) -> Task<Message> {
        let Some(tab) = self.active_tab_mut() else {
            return Task::none();
        };
        let Some(rows) = tab.run.result.as_ref().map(|r| r.rows.len()) else {
            return Task::none();
        };
        let Some(cell) = data_grid::navigate(
            &mut tab.selected,
            &mut tab.row_selection,
            &tab.columns.order,
            rows,
            arrow,
            shift,
        ) else {
            return Task::none();
        };
        match data_grid::reveal_cell(&tab.grid_scroll, &tab.widths, &tab.columns.order, cell) {
            (None, None) => Task::none(),
            (x, y) => operation::scroll_to(tab.grid_body_id.clone(), AbsoluteOffset { x, y }),
        }
    }

    /// Linhas selecionadas da aba ativa e os `DELETE` correspondentes.
    fn build_row_deletes(
        &self,
    ) -> Result<(Vec<data_edit::RowDelete>, data_edit::EditTarget), String> {
        let ws = self.active_ws().ok_or("nenhum workspace ativo")?;
        let tab = &ws.tabs[ws.active_tab];
        let result = tab.run.result.as_ref().ok_or("sem resultado")?;
        let target = match &tab.edit_target {
            Some(Ok(target)) => target.clone(),
            Some(Err(reason)) => return Err(format!("resultado não editável: {reason}")),
            None => return Err("resultado não editável".into()),
        };
        Ok((
            data_edit::build_deletes(&target, result, &tab.row_selection.rows)?,
            target,
        ))
    }

    fn open_delete_rows(&mut self) {
        if self.modal.is_some() {
            return;
        }
        self.commit_edit();
        let (deletes, target) = match self.build_row_deletes() {
            Ok(built) => built,
            Err(e) => {
                self.notify_error(format!("Não é possível excluir as linhas: {e}"));
                return;
            }
        };
        if deletes.is_empty() {
            return;
        }
        let ws = self.active_ws().expect("workspace ativo");
        let tab = &ws.tabs[ws.active_tab];
        let dropped_edits = tab
            .pending
            .keys()
            .filter(|(row, _)| tab.row_selection.rows.contains(row))
            .count();
        self.modal = Some(Modal::DeleteRows {
            tab: tab.id,
            table: format!("{}.{}", target.schema, target.table),
            keys: deletes.iter().map(|d| d.key.clone()).collect(),
            sql: deletes
                .iter()
                .map(|d| d.display.as_str())
                .collect::<Vec<_>>()
                .join("\n\n"),
            dropped_edits,
            error: None,
            running: false,
            needs_transaction: ws.require_transaction() && ws.tx != TxState::Active,
            in_transaction: ws.tx == TxState::Active,
        });
    }

    fn delete_rows(&mut self) -> Task<Message> {
        let fail = |app: &mut Self, message: String| {
            if let Some(Modal::DeleteRows { error, running, .. }) = &mut app.modal {
                *error = Some(message);
                *running = false;
            }
            Task::none()
        };
        let (deletes, target) = match self.build_row_deletes() {
            Ok(built) => built,
            Err(e) => return fail(self, format!("Não foi possível preparar a exclusão: {e}")),
        };
        let Some(ws) = self.active_ws() else {
            return Task::none();
        };
        let Some(handle) = ws.connection.clone() else {
            return fail(self, "Workspace desconectado".into());
        };
        let tab = &ws.tabs[ws.active_tab];
        let table = format!("{}.{}", target.schema, target.table);
        let rows = tab.row_selection.rows.clone();
        let log: Vec<String> = deletes
            .iter()
            .map(|d| format!("{table} [{}] excluída", d.key))
            .collect();
        let statements = deletes
            .into_iter()
            .map(|d| connection::EditStatement {
                sql: d.sql,
                params: d.params,
                label: format!("DELETE {table} [{}]", d.key),
            })
            .collect();
        let ws_id = ws.desc.id.clone();
        let tab_id = tab.id;
        if let Some(Modal::DeleteRows { running, error, .. }) = &mut self.modal {
            *running = true;
            *error = None;
        }
        Task::perform(
            async move { handle.apply_edits(statements).await },
            move |result| Message::RowsDeleted {
                ws: ws_id.clone(),
                tab: tab_id,
                rows: rows.clone(),
                log: log.clone(),
                result,
            },
        )
    }

    fn on_rows_deleted(
        &mut self,
        ws: WorkspaceId,
        tab: u64,
        rows: BTreeSet<usize>,
        log: Vec<String>,
        result: Result<Vec<Vec<crate::query::Value>>, QueryError>,
    ) -> Task<Message> {
        let Some(w) = self.ws_by_id(&ws) else {
            return Task::none();
        };
        w.sync_tx();
        let in_transaction = w.tx == TxState::Active;
        if let Err(e) = result {
            let message = format!("Linhas não excluídas: {}", describe_error(&e));
            if let Some(Modal::DeleteRows { error, running, .. }) = &mut self.modal {
                *error = Some(message);
                *running = false;
            } else {
                self.notify_error(message);
            }
            return Task::none();
        }
        if in_transaction {
            w.grid_log.extend(log);
        }
        if let Some(t) = w.tabs.iter_mut().find(|t| t.id == tab) {
            if let Some(rs) = &mut t.run.result {
                for &row in rows.iter().rev() {
                    if row < rs.rows.len() {
                        rs.rows.remove(row);
                    }
                }
            }
            t.pending = data_edit::pending_after_delete(&t.pending, &rows);
            t.selected = t
                .selected
                .and_then(|(row, col)| data_edit::index_after_delete(row, &rows).map(|r| (r, col)));
            t.editing = None;
            t.row_selection.clear();
        }
        if matches!(self.modal, Some(Modal::DeleteRows { .. })) {
            self.modal = None;
        }
        let count = rows.len();
        self.notify(if in_transaction {
            format!("{count} linha(s) excluída(s) na transação; use Confirmar… para gravar")
        } else {
            format!("{count} linha(s) excluída(s)")
        });
        Task::none()
    }
}

/// Agenda o diagnóstico de sintaxe para daqui a pouco; se o texto mudar
/// antes, a revisão não confere e o pedido é ignorado.
fn schedule_diagnostics(ws: WorkspaceId, tab: u64, rev: u64) -> Task<Message> {
    Task::perform(tokio::time::sleep(Duration::from_millis(400)), move |_| {
        Message::DiagnoseDue {
            ws: ws.clone(),
            tab,
            rev,
        }
    })
}

impl TabMenuAction {
    /// Se a aba na posição `index` é fechada pela ação disparada na aba `clicked`.
    fn includes(self, clicked: usize, index: usize) -> bool {
        match self {
            TabMenuAction::Close => index == clicked,
            TabMenuAction::CloseLeft => index < clicked,
            TabMenuAction::CloseRight => index > clicked,
            TabMenuAction::CloseAll => true,
        }
    }
}
