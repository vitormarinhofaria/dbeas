// SPDX-FileCopyrightText: 2026 Vitor Marinho Faria
// SPDX-License-Identifier: GPL-3.0-or-later

//! Renderização da janela principal.

use iced::widget::{
    Column, Space, button, center, column, container, mouse_area, opaque, row, rule, scrollable,
    space, stack, text, text_input, tooltip,
};
use iced::{Alignment, Color, Element, Font, Length, Theme, mouse};

use super::quick_open::{QuickOpen, Target};
use super::state::{Tab, Workspace};
use super::{App, CloseChoice, DragTarget, Message, Modal};
use crate::commands::{Command, SHORTCUTS};
use crate::connection::{Status, TxState};
use crate::data_grid::{self, GridView};
use crate::metadata::RelationKind;
use crate::session::{ExplorerMode, TabKind};
use crate::sql_editor::{self, EditorView};

/// Deslocamento horizontal da barra lateral em relação à janela.
pub const WORKSPACE_STRIP_WIDTH: f32 = 0.0;
/// Altura aproximada de cabeçalho, abas, barra de ferramentas e status, usada
/// para converter o arraste do divisor em proporção.
pub const CHROME_HEIGHT: f32 = 170.0;

const BOLD: Font = Font {
    weight: iced::font::Weight::Bold,
    ..Font::DEFAULT
};

impl App {
    pub fn view(&self) -> Element<'_, Message> {
        let sidebar = container(self.sidebar())
            .width(self.sidebar_width)
            .height(Length::Fill)
            .style(panel);
        let divider = mouse_area(
            container(Space::new())
                .width(5)
                .height(Length::Fill)
                .style(divider_style),
        )
        .on_press(Message::DragStart(DragTarget::Sidebar))
        .interaction(mouse::Interaction::ResizingHorizontally);

        let main = column![
            self.header(),
            self.tab_bar(),
            self.tab_content(),
            self.status_bar()
        ]
        .width(Length::Fill)
        .height(Length::Fill);

        let base: Element<'_, Message> = row![sidebar, divider, main].into();

        let overlay = if let Some(q) = &self.quick_open {
            Some(quick_open_view(q))
        } else {
            self.modal.as_ref().map(|m| self.modal_view(m))
        };

        // A raiz é sempre um `stack` com o layout na primeira camada: trocar o
        // tipo do widget raiz faria o Iced recriar todos os widgets, perdendo
        // foco do editor e posições de rolagem.
        let layer: Element<'_, Message> = match overlay {
            Some(content) => opaque(
                mouse_area(center(opaque(content)).style(|_| {
                    container::Style {
                        background: Some(
                            Color {
                                a: 0.55,
                                ..Color::BLACK
                            }
                            .into(),
                        ),
                        ..container::Style::default()
                    }
                }))
                .on_press(Message::CloseModal),
            ),
            None => Space::new().into(),
        };
        stack![base, layer].into()
    }

    // ------------------------------------------------------------------
    // Barra lateral

    fn sidebar(&self) -> Element<'_, Message> {
        let mut list = Column::new().spacing(2);
        for (i, ws) in self.workspaces.iter().enumerate() {
            list = list.push(workspace_item(i, ws, self.active == Some(i)));
        }
        if self.workspaces.is_empty() {
            list = list.push(text("Nenhum workspace registrado.").size(13).style(muted));
        }

        let workspaces = column![
            row![
                text("WORKSPACES").size(11).font(BOLD).style(muted),
                space::horizontal(),
                button(text("+").size(14))
                    .on_press(Message::OpenAddWorkspace)
                    .padding([0, 8])
                    .style(button::text),
            ]
            .align_y(Alignment::Center),
            scrollable(list).height(Length::Shrink),
        ]
        .spacing(4);

        let explorer: Element<'_, Message> = match self.active_ws() {
            Some(ws) => explorer(ws),
            None => text("").into(),
        };

        column![
            container(workspaces).max_height(320).padding(8),
            rule::horizontal(1),
            container(explorer).padding(8).height(Length::Fill),
        ]
        .into()
    }

    // ------------------------------------------------------------------
    // Cabeçalho do workspace ativo

    fn header(&self) -> Element<'_, Message> {
        let Some(ws) = self.active_ws() else {
            return container(
                text("Adicione um arquivo dbeas.json pelo botão + da barra lateral.").size(14),
            )
            .padding(12)
            .into();
        };
        let index = self.active.unwrap_or(0);

        let mut info = row![
            status_dot(ws.status),
            text(&ws.desc.display_name).size(20).font(BOLD)
        ]
        .spacing(8)
        .align_y(Alignment::Center);

        match &ws.desc.config {
            Ok(c) => {
                info = info.push(
                    text(format!("{}@{}:{}/{}", c.user, c.host, c.port, c.db_name))
                        .size(13)
                        .style(muted),
                );
                if let Some(handle) = ws.connection.as_ref() {
                    let (label, explanation) = tls_badge(c, handle.uses_tls());
                    info = info.push(badge(label, explanation));
                }
                if c.read_only {
                    info = info.push(badge(
                        "somente leitura",
                        "Proteção da ferramenta: bloqueia comandos de escrita e abre a sessão com \
                         default_transaction_read_only. Não substitui permissões no PostgreSQL.",
                    ));
                }
                if c.require_transaction {
                    info = info.push(badge(
                        "transação obrigatória",
                        "Escritas (SQL manual, grade e arquivos) só rodam dentro de uma transação \
                         iniciada pelo botão. Consultas de leitura rodam sem transação.",
                    ));
                }
            }
            Err(_) => info = info.push(text("configuração inválida").size(13).style(danger_text)),
        }

        let action: Element<'_, Message> = match ws.status {
            Status::Connected => button(text("Desconectar").size(13))
                .on_press(Message::Disconnect(index))
                .style(button::secondary)
                .into(),
            Status::Connecting => text("conectando…").size(13).into(),
            _ if ws.desc.config.is_ok() => button(text("Conectar").size(13))
                .on_press(Message::Connect(index))
                .into(),
            _ => Space::new().into(),
        };

        let mut col = column![
            row![info, space::horizontal(), transaction_controls(ws), action]
                .spacing(8)
                .align_y(Alignment::Center)
        ]
        .spacing(4);
        if let Some(error) = ws.connection_error.as_ref() {
            col = col.push(text(error).size(13).style(danger_text));
        } else if let Err(error) = &ws.desc.config {
            col = col.push(
                text(format!("{error} ({})", ws.desc.file.display()))
                    .size(13)
                    .style(danger_text),
            );
        }
        container(col).padding([8, 12]).width(Length::Fill).into()
    }

    // ------------------------------------------------------------------
    // Abas

    fn tab_bar(&self) -> Element<'_, Message> {
        let Some(ws) = self.active_ws() else {
            return Space::new().into();
        };
        let mut tabs = row![].spacing(2);
        for (i, tab) in ws.tabs.iter().enumerate() {
            let active = i == ws.active_tab;
            let mut label = tab.title();
            if tab.dirty {
                label.push_str(" ●");
            }
            let icon = match tab.kind {
                TabKind::Scratch => "✎",
                TabKind::Table { .. } => "▦",
                TabKind::File { .. } => "▤",
            };
            let mut content = row![text(icon).size(12), text(label).size(13)]
                .spacing(6)
                .align_y(Alignment::Center);
            if !tab.is_scratch() {
                content = content.push(
                    button(text("×").size(13))
                        .on_press(Message::CloseTab(i))
                        .padding([0, 4])
                        .style(button::text),
                );
            }
            tabs = tabs.push(
                button(content)
                    .on_press(Message::SelectTab(i))
                    .padding([4, 10])
                    .style(move |theme: &Theme, status| tab_style(theme, status, active)),
            );
        }
        container(
            scrollable(tabs).direction(scrollable::Direction::Horizontal(
                scrollable::Scrollbar::new().width(4).scroller_width(4),
            )),
        )
        .padding([0, 8])
        .width(Length::Fill)
        .into()
    }

    fn tab_content(&self) -> Element<'_, Message> {
        let Some(ws) = self.active_ws() else {
            return center(text("")).into();
        };
        let tab = &ws.tabs[ws.active_tab];

        let top = (tab.split * 1000.0) as u16;
        let bottom = 1000 - top;

        let results = container(results_panel(ws, tab))
            .height(Length::FillPortion(top))
            .width(Length::Fill);
        let divider = mouse_area(
            container(Space::new())
                .height(6)
                .width(Length::Fill)
                .style(divider_style),
        )
        .on_press(Message::DragStart(DragTarget::Split))
        .interaction(mouse::Interaction::ResizingVertically);

        let editor: Element<'_, Message> = if tab.loading_file {
            center(text("Carregando arquivo…")).into()
        } else {
            // Marcações só valem para o texto em que foram obtidas.
            let diagnostics: &[crate::diagnostics::Diagnostic] =
                if tab.diagnostics_rev == tab.text_rev {
                    &tab.diagnostics
                } else {
                    &[]
                };
            let fresh_error = tab.run.error.as_ref().filter(|e| e.rev == tab.text_rev);
            let error_offset = fresh_error.and_then(|e| e.offset);
            let mut error_lines: Vec<usize> = diagnostics
                .iter()
                .map(|d| d.line.saturating_sub(1))
                .collect();
            let mut underlines = Vec::new();
            if !diagnostics.is_empty() || error_offset.is_some() {
                let text = tab.editor.text();
                underlines.extend(
                    diagnostics
                        .iter()
                        .map(|d| d.offset)
                        .chain(error_offset)
                        .map(|o| crate::diagnostics::error_span(&text, o)),
                );
            }
            error_lines.extend(fresh_error.and_then(|e| e.line));
            let completion = tab.completion.as_ref().map(|c| {
                let text = tab.editor.text();
                let (line, column) = crate::query::split::line_col(&text, c.data.start);
                sql_editor::CompletionView {
                    items: &c.data.items,
                    selected: c.selected,
                    active: c.active,
                    line: line - 1,
                    column: column - 1,
                }
            });
            sql_editor::view(EditorView {
                content: &tab.editor,
                editor_id: tab.editor_id.clone(),
                scroll_id: tab.editor_scroll_id.clone(),
                error_lines,
                underlines,
                scroll_y: tab.editor_viewport.map_or(0.0, |v| v.absolute_offset().y),
                completion,
                completion_keys: sql_editor::CompletionKeys {
                    up: Message::CompletionMove(-1),
                    down: Message::CompletionMove(1),
                    accept: Message::CompletionAccept,
                    close: Message::CompletionClose,
                },
                on_pick: Message::CompletionPick,
                on_action: Message::Editor,
                on_command: Message::Command,
                on_scroll: Message::EditorScrolled,
                min_height: ((self.window_height - CHROME_HEIGHT) * (1.0 - tab.split) - 4.0).max(120.0),
                read_only_hint: ws.read_only().then_some(
                    "Workspace somente leitura: só consultas de leitura são executadas. Ctrl+Enter executa.",
                ),
            })
        };
        let mut editor_panel = column![editor];
        if let Some(first) = tab.diagnostics.first() {
            let more = tab.diagnostics.len() - 1;
            let suffix = if more > 0 {
                format!(" (+{more})")
            } else {
                String::new()
            };
            editor_panel = editor_panel.push(
                container(
                    text(format!(
                        "Sintaxe — linha {}, coluna {}: {}{suffix}",
                        first.line, first.column, first.message
                    ))
                    .size(12)
                    .style(danger_text),
                )
                .padding([2, 8]),
            );
        }
        let editor = container(editor_panel)
            .height(Length::FillPortion(bottom))
            .width(Length::Fill);

        column![results, divider, editor]
            .height(Length::Fill)
            .into()
    }

    fn status_bar(&self) -> Element<'_, Message> {
        let mut bar = row![].spacing(12).align_y(Alignment::Center);
        if let Some(notice) = &self.notice {
            let style = if notice.is_error {
                danger_text
            } else {
                text::default
            };
            bar = bar.push(
                text(&notice.text)
                    .size(13)
                    .style(style)
                    .wrapping(text::Wrapping::None),
            );
            bar = bar.push(
                button(text("×").size(13))
                    .on_press(Message::DismissNotice)
                    .padding([0, 4])
                    .style(button::text),
            );
        }
        bar = bar.push(space::horizontal());
        if let Some(ws) = self.active_ws() {
            if let Some(path) = ws.tabs[ws.active_tab].file_path() {
                bar = bar.push(
                    text(super::display_path(ws, path))
                        .size(12)
                        .style(muted)
                        .wrapping(text::Wrapping::None),
                );
            }
            let lsp = tooltip(
                text("sintaxe: parser do PostgreSQL · autocomplete: catálogo")
                    .size(12)
                    .style(muted)
                    .wrapping(text::Wrapping::None),
                container(
                    text(
                        "Diagnóstico de sintaxe com o parser do PostgreSQL (libpg_query), sem consultar o \
                         servidor. Nomes de tabelas e colunas só são validados ao executar. Autocomplete \
                         com os metadados do workspace ativo (Ctrl+Espaço). Servidor LSP externo: não \
                         configurado.",
                    )
                    .size(12),
                )
                .padding(6)
                .max_width(420)
                .style(container::rounded_box),
                tooltip::Position::Top,
            );
            bar = bar.push(lsp);
        }
        bar = bar.push(
            button(text("Atalhos").size(12))
                .on_press(Message::ShowShortcuts)
                .padding([2, 8])
                .style(button::text),
        );
        container(bar)
            .padding([4, 12])
            .width(Length::Fill)
            .clip(true)
            .style(panel)
            .into()
    }

    // ------------------------------------------------------------------
    // Modais

    fn modal_view<'a>(&'a self, modal: &'a Modal) -> Element<'a, Message> {
        let body: Element<'a, Message> = match modal {
            Modal::Commit { ws, summary, grid } => commit_view(ws, summary.as_ref(), grid),
            Modal::PendingTransaction {
                name,
                failed,
                closing,
                ..
            } => {
                let action = if *closing { "fechar a aplicação" } else { "desconectar" };
                let mut buttons = row![
                    space::horizontal(),
                    button(text(if *closing { "Cancelar fechamento" } else { "Cancelar" }))
                        .on_press(Message::CloseModal)
                        .style(button::secondary),
                    button(text("Abortar (ROLLBACK)"))
                        .on_press(Message::PendingTxRollback)
                        .style(button::danger),
                ]
                .spacing(8);
                if !failed {
                    buttons = buttons.push(button(text("Confirmar…")).on_press(Message::PendingTxCommit));
                }
                column![
                    text("Transação em aberto").size(18).font(BOLD),
                    text(format!(
                        "O workspace {name} tem uma transação {}. Antes de {action}, confirme ou aborte.",
                        if *failed { "com erro" } else { "ativa" }
                    ))
                    .size(13),
                    buttons,
                ]
                .spacing(10)
                .into()
            }
            Modal::GridSave {
                sql,
                error,
                running,
                needs_transaction,
                ..
            } => {
                let mut col = column![
                    text("Alterações pendentes").size(18).font(BOLD),
                    text("SQL que será executado (valores enviados como parâmetros):")
                        .size(13)
                        .style(muted),
                    container(scrollable(text(sql).font(Font::MONOSPACE).size(12)).height(Length::Shrink))
                        .padding(8)
                        .max_height(320)
                        .width(Length::Fill)
                        .style(container::bordered_box),
                ]
                .spacing(10);
                if *needs_transaction {
                    let begin = self.active_ws().map(|w| Message::TxBegin(w.desc.id.clone()));
                    col = col.push(
                        row![
                            text("Este workspace exige transação para gravar alterações.")
                                .size(13)
                                .style(danger_text),
                            space::horizontal(),
                            button(text("Iniciar transação").size(13)).on_press_maybe(begin),
                        ]
                        .spacing(8)
                        .align_y(Alignment::Center),
                    );
                }
                if let Some(e) = error {
                    col = col.push(text(e).size(13).style(danger_text));
                }
                col.push(
                    row![
                        button(text("Cancelar e reverter"))
                            .on_press(Message::GridSaveDiscard)
                            .style(button::danger),
                        space::horizontal(),
                        button(text("Copiar e fechar"))
                            .on_press(Message::GridSaveCopy)
                            .style(button::secondary),
                        button(text(if *running { "Executando…" } else { "Executar e fechar" }))
                            .on_press_maybe((!*running && !*needs_transaction).then_some(Message::GridSaveExecute))
                            .style(button::success),
                    ]
                    .spacing(8),
                )
                .into()
            }
            Modal::RenameItem {
                entry,
                name,
                error,
                input_id,
                busy,
            } => {
                let kind = if entry.is_dir { "diretório" } else { "arquivo" };
                let mut col = column![
                    text(format!("Renomear ou mover {kind}")).size(18).font(BOLD),
                    text(format!(
                        "Caminho relativo ao sqlFilesDir. Altere o nome para renomear ou os \
                         diretórios para mover (ex.: relatorios/{}). Só é possível mover para \
                         diretórios existentes abaixo do sqlFilesDir; \"..\" não é permitido.{}",
                        entry.name,
                        if entry.is_dir { "" } else { " A extensão .sql é mantida." }
                    ))
                    .size(13)
                    .style(muted),
                    text_input("caminho/novo-nome.sql", name)
                        .id(input_id.clone())
                        .on_input(Message::RenameName)
                        .on_submit(Message::RenameSubmit),
                ]
                .spacing(10);
                if let Some(e) = error {
                    col = col.push(text(e).size(13).style(danger_text));
                }
                col.push(
                    row![
                        space::horizontal(),
                        button(text("Cancelar"))
                            .on_press(Message::CloseModal)
                            .style(button::secondary),
                        button(text(if *busy { "Aplicando…" } else { "Renomear/mover" }))
                            .on_press_maybe((!*busy).then_some(Message::RenameSubmit)),
                    ]
                    .spacing(8),
                )
                .into()
            }
            Modal::DeleteItem {
                entry,
                sql_files,
                open_tabs,
                unsaved,
                error,
                busy,
            } => {
                let what = if entry.is_dir {
                    format!(
                        "O diretório {} e todo o seu conteúdo ({sql_files} arquivo(s) .sql) serão movidos para a lixeira.",
                        entry.name
                    )
                } else {
                    format!("O arquivo {} será movido para a lixeira.", entry.name)
                };
                let mut col = column![
                    text("Excluir").size(18).font(BOLD),
                    text(what).size(13),
                    text("A lixeira do sistema permite recuperá-lo depois.").size(12).style(muted),
                ]
                .spacing(10);
                if !open_tabs.is_empty() {
                    col = col.push(
                        text(format!("Abas que serão fechadas: {}", open_tabs.join(", "))).size(13),
                    );
                }
                if *unsaved {
                    col = col.push(
                        text("Há alterações não salvas nessas abas; elas serão descartadas.")
                            .size(13)
                            .style(danger_text),
                    );
                }
                if let Some(e) = error {
                    col = col.push(text(e).size(13).style(danger_text));
                }
                col.push(
                    row![
                        space::horizontal(),
                        button(text("Cancelar"))
                            .on_press(Message::CloseModal)
                            .style(button::secondary),
                        button(text(if *busy { "Excluindo…" } else { "Mover para a lixeira" }))
                            .on_press_maybe((!*busy).then_some(Message::DeleteConfirm))
                            .style(button::danger),
                    ]
                    .spacing(8),
                )
                .into()
            }
            Modal::NewFile {
                dir,
                name,
                error,
                input_id,
                creating,
            } => {
                let preview = if name.trim().is_empty() {
                    String::new()
                } else {
                    let trimmed = name.trim();
                    let lower = trimmed.to_ascii_lowercase();
                    if lower.ends_with(".sql") {
                        format!("{dir}/{trimmed}")
                    } else {
                        format!("{dir}/{trimmed}.sql")
                    }
                };
                let mut col = column![
                    text("Novo arquivo SQL").size(18).font(BOLD),
                    text(format!("Criado em {dir} (relativo ao sqlFilesDir). A extensão .sql é acrescentada se faltar."))
                        .size(13)
                        .style(muted),
                    text_input("consulta.sql", name)
                        .id(input_id.clone())
                        .on_input(Message::NewFileName)
                        .on_submit(Message::NewFileSubmit),
                ]
                .spacing(10);
                if !preview.is_empty() {
                    col = col.push(text(preview).size(12).font(Font::MONOSPACE).style(muted));
                }
                if let Some(e) = error {
                    col = col.push(text(e).size(13).style(danger_text));
                }
                col.push(
                    row![
                        space::horizontal(),
                        button(text("Cancelar"))
                            .on_press(Message::CloseModal)
                            .style(button::secondary),
                        button(text(if *creating { "Criando…" } else { "Criar" }))
                            .on_press_maybe((!*creating).then_some(Message::NewFileSubmit)),
                    ]
                    .spacing(8),
                )
                .into()
            }
            Modal::RunFileInTransaction { title } => column![
                text("Executar em transação").size(18).font(BOLD),
                text(format!(
                    "Este workspace exige transação. {title} será executado dentro de uma transação \
                     nova, que ficará aberta até você confirmar (COMMIT) ou cancelar (ROLLBACK)."
                ))
                .size(13),
                row![
                    space::horizontal(),
                    button(text("Cancelar")).on_press(Message::CloseModal).style(button::secondary),
                    button(text("Iniciar transação e executar")).on_press(Message::ConfirmRunFile),
                ]
                .spacing(8),
            ]
            .spacing(10)
            .into(),
            Modal::AddWorkspace { path, error } => {
                let mut col = column![
                    text("Adicionar workspace").size(18).font(BOLD),
                    text("Caminho do arquivo dbeas.json. O arquivo é registrado uma única vez em ~/.config/dbeas/config.").size(13).style(muted),
                    row![
                        text_input("/caminho/do/projeto/dbeas.json", path)
                            .on_input(Message::AddPathChanged)
                            .on_submit(Message::SubmitAddWorkspace)
                            .width(Length::Fill),
                        button(text("Procurar…")).on_press(Message::BrowseWorkspaceFile).style(button::secondary),
                    ]
                    .spacing(8),
                ]
                .spacing(10);
                if let Some(e) = error {
                    col = col.push(text(e).size(13).style(danger_text));
                }
                col.push(
                    row![
                        space::horizontal(),
                        button(text("Cancelar"))
                            .on_press(Message::CloseModal)
                            .style(button::secondary),
                        button(text("Adicionar")).on_press(Message::SubmitAddWorkspace),
                    ]
                    .spacing(8),
                )
                .into()
            }
            Modal::RemoveWorkspace { file, names } => column![
                text("Remover workspace").size(18).font(BOLD),
                text(format!(
                    "O registro do arquivo {} será removido da lista.",
                    file.display()
                ))
                .size(13),
                text(format!("Workspaces afetados: {}", names.join(", "))).size(13),
                text("Nenhum arquivo do projeto será apagado.")
                    .size(13)
                    .style(muted),
                row![
                    space::horizontal(),
                    button(text("Cancelar"))
                        .on_press(Message::CloseModal)
                        .style(button::secondary),
                    button(text("Remover"))
                        .on_press(Message::ConfirmRemoveWorkspace)
                        .style(button::danger),
                ]
                .spacing(8),
            ]
            .spacing(10)
            .into(),
            Modal::CloseTab { title, .. } => column![
                text("Alterações não salvas").size(18).font(BOLD),
                text(format!("{title} tem alterações não salvas.")).size(13),
                row![
                    space::horizontal(),
                    button(text("Cancelar"))
                        .on_press(Message::CloseModal)
                        .style(button::secondary),
                    button(text("Descartar"))
                        .on_press(Message::ConfirmCloseTab(CloseChoice::Discard))
                        .style(button::danger),
                    button(text("Salvar")).on_press(Message::ConfirmCloseTab(CloseChoice::Save)),
                ]
                .spacing(8),
            ]
            .spacing(10)
            .into(),
            Modal::Shortcuts => {
                let mut col = column![text("Atalhos").size(18).font(BOLD)].spacing(6);
                for s in SHORTCUTS {
                    col = col.push(row![
                        text(s.keys).font(Font::MONOSPACE).size(13).width(170),
                        text(s.description).size(13)
                    ]);
                }
                col.push(text("Atalhos da aplicação têm prioridade sobre o editor quando ele está focado.").size(12).style(muted))
                    .push(row![space::horizontal(), button(text("Fechar")).on_press(Message::CloseModal)])
                    .into()
            }
        };
        container(body)
            .padding(20)
            .width(640)
            .max_height((self.window_height - 80.0).max(240.0))
            .style(container::rounded_box)
            .into()
    }
}

// ----------------------------------------------------------------------
// Componentes

fn workspace_item(index: usize, ws: &Workspace, active: bool) -> Element<'_, Message> {
    let status = ws.status;
    let label = column![
        text(&ws.desc.display_name)
            .size(14)
            .font(if active { BOLD } else { Font::DEFAULT }),
        text(if ws.desc.config.is_err() {
            "erro de configuração"
        } else {
            status.label()
        })
        .size(11)
        .style(muted),
    ];
    let content = row![status_dot(status), label, space::horizontal()]
        .spacing(8)
        .align_y(Alignment::Center);
    let item = button(content)
        .on_press(Message::SelectWorkspace(index))
        .width(Length::Fill)
        .padding([4, 8])
        .style(move |theme: &Theme, s| tab_style(theme, s, active));

    let remove = button(text("−").size(13))
        .on_press(Message::AskRemoveWorkspace(index))
        .padding([0, 6])
        .style(button::text);
    let tip = match &ws.desc.config {
        Err(e) => format!("{}: {e}", ws.desc.file.display()),
        Ok(_) => format!("{} — {}", status.label(), ws.desc.file.display()),
    };
    tooltip(
        row![item, remove].align_y(Alignment::Center),
        container(text(tip).size(12))
            .padding(6)
            .style(container::rounded_box),
        tooltip::Position::Right,
    )
    .into()
}

fn transaction_controls(ws: &Workspace) -> Element<'_, Message> {
    if ws.connection.is_none() || (!ws.require_transaction() && ws.tx == TxState::Idle) {
        return Space::new().into();
    }
    let id = ws.desc.id.clone();
    let busy = ws.tx_busy;
    let mut controls = row![].spacing(6).align_y(Alignment::Center);
    match ws.tx {
        TxState::Idle => {
            controls = controls.push(
                button(text("Iniciar transação").size(13))
                    .on_press_maybe((!busy).then(|| Message::TxBegin(id.clone()))),
            );
        }
        TxState::Active => {
            controls = controls
                .push(
                    text("● transação ativa")
                        .size(13)
                        .color(Color::from_rgb(0.95, 0.75, 0.3)),
                )
                .push(
                    button(text("Cancelar").size(13))
                        .on_press_maybe((!busy).then(|| Message::TxRollback(id.clone())))
                        .style(button::danger),
                )
                .push(
                    button(text("Confirmar…").size(13))
                        .on_press_maybe((!busy).then(|| Message::TxRequestCommit(id.clone())))
                        .style(button::success),
                );
        }
        TxState::Failed => {
            controls = controls
                .push(tooltip(
                    text("✕ transação com erro").size(13).style(danger_text),
                    container(
                        text(
                            "Um comando falhou dentro da transação. O PostgreSQL rejeita os \
                              próximos comandos até o ROLLBACK.",
                        )
                        .size(12),
                    )
                    .padding(6)
                    .max_width(320)
                    .style(container::rounded_box),
                    tooltip::Position::Bottom,
                ))
                .push(
                    button(text("Cancelar (ROLLBACK)").size(13))
                        .on_press_maybe((!busy).then(|| Message::TxRollback(id.clone())))
                        .style(button::danger),
                );
        }
    }
    controls.into()
}

fn commit_view<'a>(
    ws: &'a crate::workspace::WorkspaceId,
    summary: Option<&'a Result<crate::connection::TxSummary, String>>,
    grid: &'a [String],
) -> Element<'a, Message> {
    let mut body = column![text("Confirmar transação").size(18).font(BOLD)].spacing(10);
    match summary {
        None => body = body.push(text("Carregando resumo…").size(13).style(muted)),
        Some(Err(e)) => body = body.push(text(e).size(13).style(danger_text)),
        Some(Ok(summary)) => {
            let mut tables =
                column![text("Linhas alteradas por tabela").size(14).font(BOLD)].spacing(2);
            if summary.tables.is_empty() {
                tables = tables.push(
                    text("Nenhuma linha inserida, alterada ou removida.")
                        .size(13)
                        .style(muted),
                );
            }
            for t in &summary.tables {
                tables = tables.push(
                    row![
                        text(format!("{}.{}", t.schema, t.table))
                            .size(13)
                            .font(Font::MONOSPACE)
                            .width(Length::Fill),
                        text(format!("+{}  ~{}  -{}", t.inserted, t.updated, t.deleted))
                            .size(13)
                            .font(Font::MONOSPACE),
                    ]
                    .spacing(8),
                );
            }
            body = body.push(tables);

            let mut statements =
                column![text("Instruções executadas").size(14).font(BOLD)].spacing(2);
            if summary.statements.is_empty() {
                statements = statements.push(
                    text("Nenhuma instrução de escrita registrada.")
                        .size(13)
                        .style(muted),
                );
            }
            for st in &summary.statements {
                statements = statements.push(text(st).size(12).font(Font::MONOSPACE));
            }
            body = body.push(statements);

            if !grid.is_empty() {
                let mut edits =
                    column![text("Alterações pela grade").size(14).font(BOLD)].spacing(2);
                for g in grid {
                    edits = edits.push(text(g).size(12).font(Font::MONOSPACE));
                }
                body = body.push(edits);
            }
            body = body.push(
                text("Contagens vêm de pg_stat_xact_user_tables e incluem efeitos de triggers. DDL e TRUNCATE \
                      aparecem apenas na lista de instruções.")
                .size(11)
                .style(muted),
            );
        }
    }
    let ready = matches!(summary, Some(Ok(_)));
    let buttons = row![
        space::horizontal(),
        button(text("Voltar"))
            .on_press(Message::CloseModal)
            .style(button::secondary),
        button(text("Reverter (ROLLBACK)"))
            .on_press(Message::TxRollback(ws.clone()))
            .style(button::danger),
        button(text("Confirmar (COMMIT)"))
            .on_press_maybe(ready.then(|| Message::TxCommit(ws.clone())))
            .style(button::success),
    ]
    .spacing(8);
    column![scrollable(body).height(Length::Shrink), buttons]
        .spacing(12)
        .into()
}

fn status_dot<'a>(status: Status) -> Element<'a, Message> {
    let (symbol, color) = match status {
        Status::Disconnected => ("○", Color::from_rgb(0.55, 0.55, 0.6)),
        Status::Connecting => ("◐", Color::from_rgb(0.95, 0.8, 0.3)),
        Status::Connected => ("●", Color::from_rgb(0.4, 0.85, 0.5)),
        Status::Error => ("✕", Color::from_rgb(0.95, 0.4, 0.4)),
    };
    text(symbol).size(14).color(color).into()
}

/// Rótulo e explicação do estado TLS da conexão ativa.
fn tls_badge(
    config: &crate::workspace::ResolvedConfig,
    uses_tls: bool,
) -> (&'static str, &'static str) {
    use crate::tls::Verification;
    use crate::workspace::SslMode;
    if !uses_tls {
        return match config.ssl_mode {
            SslMode::Disable => (
                "sem TLS",
                "Conexão sem criptografia (sslMode disable). Configure sslMode no dbeas.json para usar TLS.",
            ),
            _ => (
                "sem TLS",
                "O servidor não aceitou TLS e a conexão seguiu sem criptografia (sslMode prefer). \
                 Use sslMode require ou verify-full para exigir TLS.",
            ),
        };
    }
    match crate::tls::verification(config.ssl_mode, config.ssl_root_cert.is_some()) {
        Some(Verification::Full) => (
            "TLS verificado",
            "Conexão cifrada; o certificado do servidor foi emitido por CA confiável e corresponde a \
             dbHost (sslMode verify-full).",
        ),
        Some(Verification::Ca) => (
            "TLS (CA verificada)",
            "Conexão cifrada; o certificado foi emitido por CA confiável, mas o nome do host não foi \
             conferido (sslMode verify-ca, ou require com sslRootCert).",
        ),
        _ => (
            "TLS sem verificação",
            "Conexão cifrada, mas o certificado do servidor não foi verificado (sslMode prefer/require): \
             não protege contra interceptação. Use verify-full com sslRootCert para verificar.",
        ),
    }
}

fn badge<'a>(label: &'a str, explanation: &'a str) -> Element<'a, Message> {
    tooltip(
        container(text(label).size(11))
            .padding([2, 6])
            .style(|theme: &Theme| {
                let p = theme.extended_palette();
                container::Style {
                    background: Some(p.secondary.weak.color.into()),
                    text_color: Some(p.secondary.weak.text),
                    border: iced::Border {
                        radius: 4.0.into(),
                        ..iced::Border::default()
                    },
                    ..container::Style::default()
                }
            }),
        container(text(explanation).size(12))
            .padding(6)
            .max_width(360)
            .style(container::rounded_box),
        tooltip::Position::Bottom,
    )
    .into()
}

fn explorer(ws: &Workspace) -> Element<'_, Message> {
    let mode_button = |label: &'static str, mode: ExplorerMode| {
        let active = ws.explorer == mode;
        button(text(label).size(12))
            .on_press(Message::SetExplorer(mode))
            .padding([3, 8])
            .style(move |theme: &Theme, s| tab_style(theme, s, active))
    };
    let modes = row![
        mode_button("Tabelas", ExplorerMode::Tables),
        mode_button("Objetos", ExplorerMode::Objects),
        mode_button("Arquivos", ExplorerMode::Files),
    ]
    .spacing(4);

    let content: Element<'_, Message> = match ws.explorer {
        ExplorerMode::Tables => tables_explorer(ws),
        ExplorerMode::Objects => objects_explorer(ws),
        ExplorerMode::Files => files_explorer(ws),
    };
    column![modes, content].spacing(8).into()
}

fn catalog_placeholder(ws: &Workspace) -> Option<Element<'_, Message>> {
    if ws.catalog.is_some() {
        return None;
    }
    let msg = if let Some(e) = &ws.catalog_error {
        e.clone()
    } else if ws.catalog_loading {
        "Carregando metadados…".into()
    } else {
        match ws.status {
            Status::Connected => "Metadados não carregados (F5 atualiza).".into(),
            Status::Connecting => "Conectando…".into(),
            _ => "Conecte o workspace para listar objetos.".into(),
        }
    };
    Some(text(msg).size(13).style(muted).into())
}

fn tables_explorer(ws: &Workspace) -> Element<'_, Message> {
    let filter = text_input("Filtrar tabelas e views", &ws.filter)
        .on_input(Message::FilterChanged)
        .size(13);
    if let Some(p) = catalog_placeholder(ws) {
        return column![filter, p].spacing(6).into();
    }
    let needle = ws.filter.to_lowercase();
    let mut list = Column::new().spacing(1);
    let mut count = 0;
    for r in ws.catalog.iter().flat_map(|c| c.relations()) {
        let full = format!("{}.{}", r.schema, r.name);
        if !needle.is_empty() && !full.to_lowercase().contains(&needle) {
            continue;
        }
        count += 1;
        let label = if r.schema == "public" {
            r.name.clone()
        } else {
            full
        };
        list = list.push(object_button(
            relation_icon(r.kind),
            label,
            Some(Message::OpenRelation {
                schema: r.schema.clone(),
                name: r.name.clone(),
            }),
        ));
    }
    if count == 0 {
        list = list.push(text("Nenhuma tabela ou view.").size(13).style(muted));
    }
    column![filter, scrollable(list).height(Length::Fill)]
        .spacing(6)
        .into()
}

fn objects_explorer(ws: &Workspace) -> Element<'_, Message> {
    if let Some(p) = catalog_placeholder(ws) {
        return p;
    }
    let mut tree = Column::new().spacing(1);
    for schema in ws.catalog.iter().flat_map(|c| c.schemas.iter()) {
        let key = format!("s:{}", schema.name);
        let open = ws.expanded.contains(&key);
        tree = tree.push(tree_toggle(
            0,
            open,
            format!("{} (schema)", schema.name),
            key.clone(),
        ));
        if !open {
            continue;
        }
        let groups: [(&str, Vec<(String, Option<Message>)>); 5] = [
            (
                "Tabelas",
                relations_of(schema, |k| !k.is_view() && k != RelationKind::ForeignTable),
            ),
            ("Views", relations_of(schema, |k| k == RelationKind::View)),
            (
                "Views materializadas",
                relations_of(schema, |k| k == RelationKind::MaterializedView),
            ),
            (
                "Funções",
                schema.functions.iter().map(|f| (f.clone(), None)).collect(),
            ),
            (
                "Sequências",
                schema.sequences.iter().map(|s| (s.clone(), None)).collect(),
            ),
        ];
        for (label, items) in groups {
            if items.is_empty() {
                continue;
            }
            let gkey = format!("{key}/{label}");
            let gopen = ws.expanded.contains(&gkey);
            tree = tree.push(tree_toggle(
                1,
                gopen,
                format!("{label} ({})", items.len()),
                gkey,
            ));
            if gopen {
                for (name, action) in items {
                    tree = tree.push(row![
                        Space::new().width(28),
                        object_button("", name, action)
                    ]);
                }
            }
        }
    }
    scrollable(tree).height(Length::Fill).into()
}

fn relations_of(
    schema: &crate::metadata::Schema,
    filter: impl Fn(RelationKind) -> bool,
) -> Vec<(String, Option<Message>)> {
    schema
        .relations
        .iter()
        .filter(|r| filter(r.kind))
        .map(|r| {
            (
                r.name.clone(),
                Some(Message::OpenRelation {
                    schema: r.schema.clone(),
                    name: r.name.clone(),
                }),
            )
        })
        .collect()
}

fn tree_toggle<'a>(depth: u16, open: bool, label: String, key: String) -> Element<'a, Message> {
    row![
        Space::new().width(f32::from(depth) * 14.0),
        button(
            row![
                text(if open { "▾" } else { "▸" }).size(12),
                text(label).size(13)
            ]
            .spacing(6)
        )
        .on_press(Message::ToggleExpanded(key))
        .padding([2, 4])
        .width(Length::Fill)
        .style(button::text),
    ]
    .into()
}

fn files_explorer(ws: &Workspace) -> Element<'_, Message> {
    let root = ws
        .desc
        .config
        .as_ref()
        .ok()
        .and_then(|c| c.sql_files_dir.clone());
    let Some(root) = root else {
        return column![
            new_file_button(Some("Nenhum diretório configurado (sqlFilesDir).")),
            text("Nenhum diretório configurado (sqlFilesDir).")
                .size(13)
                .style(muted),
        ]
        .spacing(6)
        .into();
    };
    let entries = match &ws.files.entries {
        None => return text("Carregando…").size(13).style(muted).into(),
        Some(Err(e)) => {
            return column![
                new_file_button(Some(e.as_str())),
                text(e).size(13).style(danger_text),
            ]
            .spacing(6)
            .into();
        }
        Some(Ok(entries)) => entries,
    };

    let current = ws.files.current_dir.clone().unwrap_or_else(|| root.clone());
    let canonical_root = std::fs::canonicalize(&root).unwrap_or(root.clone());
    let canonical_current = std::fs::canonicalize(&current).unwrap_or(current.clone());
    let relative = canonical_current
        .strip_prefix(&canonical_root)
        .map(|p| p.display().to_string())
        .unwrap_or_default();

    let mut list = Column::new().spacing(1);
    if canonical_current != canonical_root {
        if let Some(parent) = canonical_current.parent() {
            list = list.push(object_button(
                "↰",
                "..".into(),
                Some(Message::NavigateDir(parent.to_path_buf())),
            ));
        }
    }
    for e in entries {
        let (icon, msg) = if e.is_dir {
            ("▸", Message::NavigateDir(e.path.clone()))
        } else {
            ("▤", Message::OpenFile(e.path.clone()))
        };
        let menu_open = ws.files.menu.as_ref().is_some_and(|m| m.path == e.path);
        let item = mouse_area(object_button(icon, e.name.clone(), Some(msg)))
            .on_right_press(Message::FileMenu(e.clone()));
        list = list.push(container(item).style(move |theme: &Theme| {
            if menu_open {
                container::Style {
                    background: Some(theme.extended_palette().background.weak.color.into()),
                    ..container::Style::default()
                }
            } else {
                container::Style::default()
            }
        }));
        if menu_open {
            list = list.push(file_menu());
        }
    }
    if entries.is_empty() {
        list = list.push(text("Nenhum arquivo .sql aqui.").size(13).style(muted));
    }
    column![
        row![
            text(format!("/{relative}"))
                .size(12)
                .font(Font::MONOSPACE)
                .style(muted)
                .width(Length::Fill),
            new_file_button(None),
        ]
        .align_y(Alignment::Center),
        scrollable(list).height(Length::Fill),
    ]
    .spacing(4)
    .into()
}

/// Menu de contexto de um arquivo/diretório, exibido logo abaixo do item.
fn file_menu<'a>() -> Element<'a, Message> {
    let entry = |label: &'static str, message: Message, danger: bool| {
        button(text(label).size(13))
            .on_press(message)
            .width(Length::Fill)
            .padding([3, 10])
            .style(move |theme: &Theme, status| {
                let p = theme.extended_palette();
                button::Style {
                    background: matches!(status, button::Status::Hovered)
                        .then(|| p.background.weak.color.into()),
                    text_color: if danger {
                        p.danger.strong.color
                    } else {
                        p.background.base.text
                    },
                    ..button::Style::default()
                }
            })
    };
    row![
        Space::new().width(18),
        container(
            column![
                entry("Renomear/mover…", Message::RenameDialog, false),
                entry("Excluir…", Message::DeleteDialog, true),
            ]
            .width(160),
        )
        .padding(2)
        .style(|theme: &Theme| {
            let p = theme.extended_palette();
            container::Style {
                background: Some(p.background.weaker.color.into()),
                border: iced::Border {
                    color: p.background.strong.color,
                    width: 1.0,
                    radius: 4.0.into(),
                },
                ..container::Style::default()
            }
        }),
    ]
    .into()
}

/// Botão "Novo arquivo"; com `unavailable`, aparece desabilitado e explica o motivo.
fn new_file_button(unavailable: Option<&str>) -> Element<'_, Message> {
    let button = button(text("+ Novo arquivo").size(12))
        .padding([2, 8])
        .style(button::secondary)
        .on_press_maybe(unavailable.is_none().then_some(Message::NewFileDialog));
    match unavailable {
        None => button.into(),
        Some(reason) => tooltip(
            button,
            container(text(format!("Indisponível: {reason}")).size(12))
                .padding(6)
                .max_width(320)
                .style(container::rounded_box),
            tooltip::Position::Bottom,
        )
        .into(),
    }
}

fn object_button<'a>(
    icon: &'a str,
    label: String,
    action: Option<Message>,
) -> Element<'a, Message> {
    button(row![text(icon).size(12).width(14), text(label).size(13)].spacing(6))
        .on_press_maybe(action)
        .padding([2, 4])
        .width(Length::Fill)
        .style(button::text)
        .into()
}

fn relation_icon(kind: RelationKind) -> &'static str {
    match kind {
        RelationKind::View | RelationKind::MaterializedView => "◇",
        _ => "▦",
    }
}

fn results_panel<'a>(ws: &'a Workspace, tab: &'a Tab) -> Element<'a, Message> {
    let run = &tab.run;
    let can_run = ws.connection.is_some() && !run.running;

    let mut toolbar = row![
        hint(
            button(text("▶ Executar").size(13))
                .on_press_maybe(can_run.then_some(Message::Command(Command::ExecuteCurrent)))
                .padding([3, 10]),
            "Executa o texto selecionado ou, sem seleção, apenas a instrução onde está o \
             cursor (delimitada por ;).",
            "Ctrl+Enter",
        ),
        hint(
            button(text("Script").size(13))
                .on_press_maybe(can_run.then_some(Message::Command(Command::ExecuteScript)))
                .padding([3, 10])
                .style(button::secondary),
            "Executa todas as instruções do editor, em ordem, parando no primeiro erro. A \
             grade mostra o resultado da última instrução que retornar linhas.",
            "Ctrl+Shift+Enter",
        ),
    ]
    .spacing(6)
    .align_y(Alignment::Center);
    if run.running {
        toolbar = toolbar.push(text("executando…").size(13)).push(
            button(text("Cancelar").size(13))
                .on_press(Message::Command(Command::CancelQuery))
                .padding([3, 10])
                .style(button::danger),
        );
    }
    if let Some(result) = &run.result {
        let mut summary = format!("{} linha(s)", result.rows.len());
        if result.has_more {
            summary.push_str(" (há mais)");
        }
        if result.truncated {
            summary.push_str(&format!(
                " — limite de {} linhas em memória atingido",
                crate::query::MAX_ROWS
            ));
        }
        toolbar = toolbar.push(text(summary).size(13).style(muted));
        if result.has_more && run.cursor.is_some() {
            toolbar = toolbar.push(
                button(
                    text(if run.fetching {
                        "carregando…"
                    } else {
                        "Carregar mais"
                    })
                    .size(12),
                )
                .on_press_maybe((!run.fetching).then_some(Message::FetchMore))
                .padding([2, 8])
                .style(button::secondary),
            );
        }
    }
    let hidden = tab.columns.hidden_count();
    if hidden > 0 || tab.pristine.is_some() {
        let label = if hidden > 0 {
            format!("Restaurar colunas ({hidden})")
        } else {
            "Restaurar colunas".to_string()
        };
        toolbar = toolbar.push(
            button(text(label).size(12))
                .on_press(Message::RestoreColumns)
                .padding([2, 8])
                .style(button::secondary),
        );
    }
    if let Some(ms) = run.elapsed_ms {
        toolbar = toolbar.push(text(format!("{ms} ms")).size(12).style(muted));
    }
    if let Some(last) = run.messages.last() {
        toolbar = toolbar
            .push(space::horizontal())
            .push(text(last).size(12).style(muted));
    }

    let mut col = column![container(toolbar).padding([4, 8])].spacing(0);
    if let Some(edit) = edit_bar(tab) {
        col = col.push(container(edit).padding([0, 8]));
    }

    if let Some(error) = &run.error {
        let position = error
            .position
            .map(|(l, c)| format!("Linha {l}, coluna {c}: "))
            .unwrap_or_default();
        col = col.push(
            container(
                text(format!("{position}{}", error.text))
                    .size(13)
                    .font(Font::MONOSPACE),
            )
            .padding(8)
            .width(Length::Fill)
            .style(|theme: &Theme| {
                let p = theme.extended_palette();
                container::Style {
                    background: Some(
                        Color {
                            a: 0.18,
                            ..p.danger.base.color
                        }
                        .into(),
                    ),
                    text_color: Some(p.danger.strong.color),
                    ..container::Style::default()
                }
            }),
        );
    }

    let grid: Element<'a, Message> = match &run.result {
        Some(result) if !result.columns.is_empty() => data_grid::view(GridView {
            result,
            widths: &tab.widths,
            scroll: &tab.grid_scroll,
            header_id: tab.grid_header_id.clone(),
            body_id: tab.grid_body_id.clone(),
            selected: tab.selected,
            order: &tab.columns.order,
            sort: tab.columns.sort.as_ref().map(|(name, d)| {
                (
                    name.as_str(),
                    *d == crate::query::transform::Direction::Desc,
                )
            }),
            pressed: tab.columns.pressed,
            on_scroll: Message::GridScrolled,
            on_select: Message::SelectCell,
            on_header_press: Message::HeaderPressed,
            on_header_release: Message::HeaderReleased,
            on_header_middle: Message::HideColumn,
            pending: &tab.pending,
            editing: tab
                .editing
                .as_ref()
                .map(|e| (e.row, e.col, e.text.as_str(), e.input_id.clone())),
            on_double_click: Message::StartEdit,
            on_edit_input: Message::EditInput,
            on_edit_submit: Message::CommitEdit,
        }),
        _ => center(
            text(if ws.connection.is_none() {
                "Sem conexão."
            } else {
                "Execute uma consulta (Ctrl+Enter) para ver resultados."
            })
            .size(13)
            .style(muted),
        )
        .into(),
    };
    col.push(grid).height(Length::Fill).into()
}

/// Tooltip descritivo de um botão, com o atalho de teclado equivalente.
fn hint<'a>(
    content: impl Into<Element<'a, Message>>,
    description: &'a str,
    shortcut: &'a str,
) -> Element<'a, Message> {
    tooltip(
        content,
        container(
            column![
                text(description).size(12),
                text(format!("Atalho: {shortcut}")).size(12).font(BOLD),
            ]
            .spacing(4),
        )
        .padding(8)
        .max_width(320)
        .style(container::rounded_box),
        tooltip::Position::Bottom,
    )
    .into()
}

/// Barra de edição: alvo editável, célula selecionada e pendências.
fn edit_bar(tab: &Tab) -> Option<Element<'_, Message>> {
    let result = tab.run.result.as_ref()?;
    let target = tab.edit_target.as_ref()?;
    let mut bar = row![].spacing(6).align_y(Alignment::Center);
    match target {
        Err(reason) => {
            bar = bar.push(
                text(format!("Resultado não editável: {reason}"))
                    .size(12)
                    .style(muted),
            );
        }
        Ok(target) => {
            bar = bar.push(
                text(format!(
                    "Editável: {}.{} (duplo clique na célula)",
                    target.schema, target.table
                ))
                .size(12)
                .style(muted),
            );
            if let Some((row, col)) = tab.selected {
                if target.columns.get(col).is_some_and(Option::is_some) {
                    let original = &result.rows[row][col];
                    let original = if original.is_null() {
                        "NULL".to_string()
                    } else {
                        data_grid::cell_text(original)
                    };
                    match tab.pending.get(&(row, col)) {
                        Some(new) => {
                            let new = new.clone().unwrap_or_else(|| "NULL".into());
                            bar = bar
                                .push(text(format!("original: {original} → novo: {new}")).size(12));
                            bar = bar.push(
                                button(text("Reverter célula").size(12))
                                    .on_press(Message::RevertCell)
                                    .padding([2, 8])
                                    .style(button::secondary),
                            );
                        }
                        None => {
                            bar = bar.push(text(format!("valor: {original}")).size(12).style(muted))
                        }
                    }
                    bar = bar
                        .push(
                            button(text("Editar").size(12))
                                .on_press(Message::StartEdit(row, col))
                                .padding([2, 8])
                                .style(button::secondary),
                        )
                        .push(
                            button(text("NULL").size(12))
                                .on_press(Message::SetNull)
                                .padding([2, 8])
                                .style(button::secondary),
                        );
                }
            }
            if !tab.pending.is_empty() {
                bar = bar
                    .push(space::horizontal())
                    .push(
                        text(format!("{} alteração(ões) pendente(s)", tab.pending.len())).size(12),
                    )
                    .push(
                        button(text("Reverter tudo").size(12))
                            .on_press(Message::RevertAll)
                            .padding([2, 8])
                            .style(button::danger),
                    )
                    .push(
                        button(text("Revisar (Ctrl+S)").size(12))
                            .on_press(Message::Command(Command::Save))
                            .padding([2, 8])
                            .style(button::success),
                    );
            }
        }
    }
    Some(bar.into())
}

fn quick_open_view(q: &QuickOpen) -> Element<'_, Message> {
    let input = text_input("Buscar tabelas, views e arquivos SQL…", &q.query)
        .id(q.input_id.clone())
        .on_input(Message::QuickOpenQuery)
        .on_submit(Message::QuickOpenSubmit)
        .size(15);
    let mut list = Column::new().spacing(1);
    for (pos, &i) in q.matches.iter().enumerate() {
        let item = &q.items[i];
        let selected = pos == q.selected;
        let icon = match item.target {
            Target::Relation { kind, .. } => relation_icon(kind),
            Target::File(_) => "▤",
        };
        list = list.push(
            button(
                row![
                    text(icon).size(12).width(14),
                    text(&item.label).size(14),
                    space::horizontal(),
                    text(&item.detail).size(12).style(muted)
                ]
                .spacing(8),
            )
            .on_press(Message::QuickOpenPick(pos))
            .width(Length::Fill)
            .padding([3, 8])
            .style(move |theme: &Theme, s| tab_style(theme, s, selected)),
        );
    }
    if q.matches.is_empty() {
        list = list.push(text("Nada encontrado.").size(13).style(muted));
    }
    container(column![input, scrollable(list).height(Length::Shrink)].spacing(8))
        .padding(12)
        .width(640)
        .max_height(480)
        .style(container::rounded_box)
        .into()
}

// ----------------------------------------------------------------------
// Estilos

fn muted(theme: &Theme) -> text::Style {
    let p = theme.extended_palette();
    text::Style {
        color: Some(Color {
            a: 0.6,
            ..p.background.base.text
        }),
    }
}

fn danger_text(theme: &Theme) -> text::Style {
    text::Style {
        color: Some(theme.extended_palette().danger.strong.color),
    }
}

fn panel(theme: &Theme) -> container::Style {
    let p = theme.extended_palette();
    container::Style {
        background: Some(p.background.weakest.color.into()),
        ..container::Style::default()
    }
}

fn divider_style(theme: &Theme) -> container::Style {
    let p = theme.extended_palette();
    container::Style {
        background: Some(p.background.weak.color.into()),
        ..container::Style::default()
    }
}

fn tab_style(theme: &Theme, status: button::Status, active: bool) -> button::Style {
    let p = theme.extended_palette();
    let background = match (active, status) {
        (true, _) => p.primary.weak.color,
        (false, button::Status::Hovered) => p.background.weak.color,
        _ => Color::TRANSPARENT,
    };
    button::Style {
        background: Some(background.into()),
        text_color: if active {
            p.primary.weak.text
        } else {
            p.background.base.text
        },
        border: iced::Border {
            radius: 4.0.into(),
            ..iced::Border::default()
        },
        ..button::Style::default()
    }
}
