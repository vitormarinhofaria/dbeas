//! Estado global, mensagens e ciclo de vida da janela.

mod quick_open;
mod state;
mod view;

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use iced::keyboard::{Key, Modifiers, key};
use iced::widget::scrollable::{AbsoluteOffset, Viewport};
use iced::widget::{operation, text_editor};
use iced::{Event, Point, Size, Subscription, Task, Theme, event, mouse, window};

use crate::commands::{self, Command};
use crate::connection::{self, ConnectionHandle, Page, Status};
use crate::data_grid;
use crate::file_explorer::{self, FileEntry};
use crate::metadata::{self, Catalog};
use crate::query::transform::{self, Direction};
use crate::query::{Outcome, QueryError, split};
use crate::session::{self, ExplorerMode, Session, TabKind};
use crate::settings::{self, Settings};
use crate::sql_editor;
use crate::workspace::{self, WorkspaceId};

use quick_open::QuickOpen;
use state::{ErrorInfo, Tab, Workspace};

pub fn run() -> iced::Result {
    iced::application(App::boot, App::update, App::view)
        .title(App::title)
        .theme(App::theme)
        .subscription(App::subscription)
        .exit_on_close_request(false)
        .window_size(Size::new(1400.0, 860.0))
        .run()
}

pub struct App {
    settings_path: PathBuf,
    settings: Settings,
    session_path: PathBuf,
    /// Última sessão conhecida, inclusive de workspaces que não puderam ser
    /// carregados agora (arquivo ausente), para não perder seu estado.
    session: Session,
    workspaces: Vec<Workspace>,
    active: Option<usize>,
    sidebar_width: f32,
    window_height: f32,
    drag: Option<Drag>,
    modal: Option<Modal>,
    quick_open: Option<QuickOpen>,
    sql_files: HashMap<WorkspaceId, Vec<PathBuf>>,
    notice: Option<Notice>,
    session_dirty: bool,
    next_tab_id: u64,
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
    AddWorkspace { path: String, error: Option<String> },
    RemoveWorkspace { file: PathBuf, names: Vec<String> },
    CloseTab { tab_id: u64, title: String },
    Shortcuts,
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
    BrowsedWorkspaceFile(Option<PathBuf>),
    SubmitAddWorkspace,
    AskRemoveWorkspace(usize),
    ConfirmRemoveWorkspace,

    // Exploradores
    SetExplorer(ExplorerMode),
    FilterChanged(String),
    ToggleExpanded(String),
    OpenRelation {
        schema: String,
        name: String,
    },
    NavigateDir(PathBuf),
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
    Editor(text_editor::Action),
    EditorScrolled(Viewport),
    GridScrolled(Viewport),
    SelectCell(usize, usize),
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
            session_path,
            sidebar_width: session.layout.sidebar_width.clamp(180.0, 700.0),
            session,
            workspaces: Vec::new(),
            active: None,
            window_height: 860.0,
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
        };

        let mut tasks = vec![app.reload_workspaces()];
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
            Some(ws) => format!("DBeas — {}", ws.desc.display_name),
            None => "DBeas".into(),
        }
    }

    fn theme(&self) -> Theme {
        Theme::CatppuccinMocha
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
                Event::Window(window::Event::Resized(size)) => Some(Message::WindowResized(size)),
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
        ) {
            self.session_dirty = true;
        }

        match message {
            Message::SelectWorkspace(index) => self.select_workspace(index),
            Message::Connect(index) => self.connect(index),
            Message::Disconnect(index) => {
                if let Some(ws) = self.workspaces.get_mut(index) {
                    ws.disconnect();
                }
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
                        self.load_catalog(&id)
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
                });
                Task::none()
            }
            Message::AddPathChanged(value) => {
                if let Some(Modal::AddWorkspace { path, error }) = &mut self.modal {
                    *path = value;
                    *error = None;
                }
                Task::none()
            }
            Message::BrowseWorkspaceFile => Task::perform(
                async {
                    rfd::AsyncFileDialog::new()
                        .set_title("Selecione um dbeas.json")
                        .add_filter("dbeas.json", &["json"])
                        .pick_file()
                        .await
                        .map(|f| f.path().to_path_buf())
                },
                Message::BrowsedWorkspaceFile,
            ),
            Message::BrowsedWorkspaceFile(picked) => {
                if let (Some(p), Some(Modal::AddWorkspace { path, error })) =
                    (picked, &mut self.modal)
                {
                    *path = p.display().to_string();
                    *error = None;
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
                    }
                    Err(e) => {
                        t.run.error = Some(ErrorInfo {
                            text: e.clone(),
                            position: None,
                            line: None,
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
                    }
                }
                Task::none()
            }
            Message::CloseTab(index) => self.request_close_tab(index),
            Message::ConfirmCloseTab(choice) => {
                let Some(Modal::CloseTab { tab_id, .. }) = self.modal.take() else {
                    return Task::none();
                };
                match choice {
                    CloseChoice::Discard => {
                        self.close_tab_by_id(tab_id);
                        Task::none()
                    }
                    CloseChoice::Save => self.save_tab(tab_id, true),
                }
            }
            Message::Editor(action) => self.editor_action(action),
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
                tab.grid_scroll.offset_y = viewport.absolute_offset().y;
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
                if let Some(tab) = self.active_tab_mut() {
                    tab.selected = Some((row, col));
                }
                Task::none()
            }
            Message::FetchMore => self.fetch_more(),
            Message::Executed {
                ws,
                tab,
                exec_id,
                result,
            } => self.on_executed(ws, tab, exec_id, result),
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
            Message::CloseModal => {
                self.modal = None;
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
                self.window_height = size.height;
                Task::none()
            }

            Message::CloseRequested(id) => {
                // Gravação síncrona: a janela fecha logo em seguida.
                if let Err(e) = self.snapshot().save(&self.session_path) {
                    eprintln!("dbeas: {e}");
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

    /// Recarrega todos os `dbeas.json` registrados, preservando o estado em
    /// execução dos workspaces que continuam existindo.
    fn reload_workspaces(&mut self) -> Task<Message> {
        let active_id = self.active_ws().map(|w| w.desc.id.clone());
        for ws in &self.workspaces {
            self.session
                .workspaces
                .insert(ws.desc.id.clone(), ws.snapshot());
        }

        let mut previous = std::mem::take(&mut self.workspaces);
        let mut tasks = Vec::new();
        for desc in workspace::load_all(&self.settings.workspaces) {
            match previous.iter().position(|w| w.desc.id == desc.id) {
                Some(i) => {
                    let mut ws = previous.swap_remove(i);
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

    fn select_workspace(&mut self, index: usize) -> Task<Message> {
        if index >= self.workspaces.len() {
            return Task::none();
        }
        self.active = Some(index);
        self.quick_open = None;
        let mut tasks = Vec::new();
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

    fn add_workspace(&mut self) -> Task<Message> {
        let Some(Modal::AddWorkspace { path, error }) = &mut self.modal else {
            return Task::none();
        };
        let raw = path.trim();
        let expanded = match raw.strip_prefix("~/") {
            Some(rest) => dirs::home_dir().unwrap_or_default().join(rest),
            None => PathBuf::from(raw),
        };
        if raw.is_empty() || !expanded.is_file() {
            *error = Some("Informe o caminho de um arquivo dbeas.json existente.".into());
            return Task::none();
        }
        if !self.settings.register(&expanded) {
            *error = Some("Este arquivo já está registrado.".into());
            return Task::none();
        }
        self.modal = None;
        if let Err(e) = self.settings.save(&self.settings_path) {
            self.notify_error(e);
        }
        let before = self.workspaces.len();
        let reload = self.reload_workspaces();
        let select = if self.workspaces.len() > before {
            let canonical = crate::fsutil::canonical_or_normalized(&expanded);
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
        Task::batch([reload, select])
    }

    // ------------------------------------------------------------------
    // Exploradores e abertura de abas

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
        if tab.dirty {
            self.modal = Some(Modal::CloseTab {
                tab_id: tab.id,
                title: tab.title(),
            });
            return Task::none();
        }
        let id = tab.id;
        self.close_tab_by_id(id);
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
        tab.editor.perform(action);
        if is_edit {
            if tab.file_path().is_some() {
                tab.dirty = true;
            }
            tab.generated = false;
            tab.pristine = None;
        }
        let line = tab.editor.cursor().position.line;
        match sql_editor::scroll_to_show(line, tab.editor_viewport.as_ref()) {
            Some(y) => operation::scroll_to(
                tab.editor_scroll_id.clone(),
                AbsoluteOffset {
                    x: None,
                    y: Some(y),
                },
            ),
            None => Task::none(),
        }
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
        self.start_execution(sql, base, statements, false)
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
        tab.run.running = true;
        tab.run.exec_id += 1;
        tab.run.base_offset = base;
        tab.run.error = None;
        tab.run.cursor = None;
        tab.run.sent = sql.clone();
        tab.run.from_grid = from_grid;
        let exec_id = tab.run.exec_id;
        let tab_id = tab.id;
        Task::perform(
            async move { handle.execute(sql, statements).await },
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
        if self.modal.is_some() {
            if matches!(key.as_ref(), Key::Named(key::Named::Escape)) {
                self.modal = None;
            }
            return Task::none();
        }
        if captured {
            return Task::none();
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
