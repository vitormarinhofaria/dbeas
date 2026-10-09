//! Editor SQL: highlight, números de linha, marcação de erros, autocomplete e
//! atalhos.
//!
//! O `text_editor` do Iced não tem gutter próprio. O editor é renderizado com
//! altura igual ao conteúdo dentro de um `scrollable`, ao lado de uma coluna de
//! números com a mesma altura de linha; assim os dois rolam juntos. Por isso
//! as linhas não quebram (`Wrapping::None`).
//!
//! A lista de sugestões é desenhada numa camada acima do editor (`stack` +
//! `pin`), na posição do cursor calculada pela grade monoespaçada.

use iced::keyboard::{Key, key};
use iced::widget::scrollable::Viewport;
use iced::widget::text::LineHeight;
use iced::widget::text_editor::{self, Binding, Content, KeyPress, Status};
use iced::widget::{
    Column, Id, button, container, pin, rich_text, row, scrollable, space, span, stack, text,
    text_editor as editor,
};
use iced::{Color, Element, Font, Length, Pixels, Theme};

use crate::commands::{self, Command};
use crate::completion::Item;

pub const FONT_SIZE: f32 = 14.0;
pub const LINE_HEIGHT: f32 = 20.0;
pub const PADDING: f32 = 6.0;
/// Largura aproximada de um caractere da fonte monoespaçada em `FONT_SIZE`.
const CHAR_WIDTH: f32 = 8.4;
const GUTTER_PADDING: f32 = 8.0;
const INDENT: &str = "    ";
const POPUP_ROW: f32 = 22.0;
const POPUP_ROWS: usize = 8;

/// Lista de sugestões aberta.
pub struct CompletionView<'a> {
    pub items: &'a [Item],
    pub selected: usize,
    /// A lista foi ativada (Ctrl+Espaço ou setas): só então Enter/Tab aceitam
    /// e há item destacado. Aberta automaticamente, ela é apenas uma dica.
    pub active: bool,
    /// Posição do início do trecho a completar (linha a partir de 0, coluna em
    /// caracteres).
    pub line: usize,
    pub column: usize,
}

/// Mensagens produzidas pelas teclas enquanto a lista de sugestões está aberta.
#[derive(Clone)]
pub struct CompletionKeys<M> {
    pub up: M,
    pub down: M,
    pub accept: M,
    pub close: M,
}

pub struct EditorView<'a, M> {
    pub content: &'a Content,
    pub editor_id: Id,
    pub scroll_id: Id,
    /// Linhas (a partir de 0) a destacar no gutter: erro de execução e
    /// diagnósticos de sintaxe.
    pub error_lines: Vec<usize>,
    pub on_action: fn(text_editor::Action) -> M,
    pub on_command: fn(Command) -> M,
    pub on_scroll: fn(Viewport) -> M,
    pub read_only_hint: Option<&'a str>,
    /// Altura mínima, para que a área clicável preencha o painel.
    pub min_height: f32,
    /// Deslocamento vertical atual do `scrollable`, para posicionar a lista.
    pub scroll_y: f32,
    pub completion: Option<CompletionView<'a>>,
    pub completion_keys: CompletionKeys<M>,
    pub on_pick: fn(usize) -> M,
}

pub fn view<'a, M: Clone + 'a>(v: EditorView<'a, M>) -> Element<'a, M> {
    let on_command = v.on_command;
    let line_count = v.content.line_count().max(1);
    let digits = line_count.to_string().len().max(2);

    let gutter = gutter(line_count, digits, &v.error_lines);

    let keys = v
        .completion
        .as_ref()
        .map(|c| (v.completion_keys.clone(), c.active));
    let input = editor(v.content)
        .id(v.editor_id)
        .font(Font::MONOSPACE)
        .size(FONT_SIZE)
        .line_height(LineHeight::Absolute(Pixels(LINE_HEIGHT)))
        .padding(PADDING)
        .height(Length::Shrink)
        .min_height(v.min_height)
        .wrapping(iced::widget::text::Wrapping::None)
        .placeholder(v.read_only_hint.unwrap_or(
            "Escreva SQL aqui. Ctrl+Enter executa a instrução atual; Ctrl+Espaço sugere nomes.",
        ))
        .highlight("sql", iced::highlighter::Theme::Base16Mocha)
        .key_binding(move |press: KeyPress| {
            key_binding(
                press,
                on_command,
                keys.as_ref().map(|(k, active)| (k, *active)),
            )
        })
        .on_action(v.on_action);

    let body = scrollable(row![gutter, input].width(Length::Fill))
        .id(v.scroll_id)
        .on_scroll(v.on_scroll)
        .height(Length::Fill)
        .width(Length::Fill);

    // A estrutura é sempre a mesma (`stack` com o editor na primeira camada),
    // com ou sem lista de sugestões. O Iced associa o estado dos widgets — como o
    // foco do editor — à posição e ao tipo na árvore; trocar a raiz entre
    // `scrollable` e `stack` recriaria o editor e ele perderia o foco.
    let overlay: Element<'a, M> = match v.completion {
        Some(completion) => {
            let gutter_width = digits as f32 * CHAR_WIDTH + 2.0 * GUTTER_PADDING;
            let x = gutter_width + PADDING + completion.column as f32 * CHAR_WIDTH;
            let y = PADDING + (completion.line + 1) as f32 * LINE_HEIGHT - v.scroll_y;
            pin(popup(&completion, v.on_pick)).x(x).y(y.max(0.0)).into()
        }
        None => space().into(),
    };
    stack![body, overlay].into()
}

fn popup<'a, M: Clone + 'a>(c: &CompletionView<'a>, on_pick: fn(usize) -> M) -> Element<'a, M> {
    let mut list = Column::new();
    for (i, item) in c.items.iter().enumerate() {
        let selected = c.active && i == c.selected;
        list = list.push(
            button(
                row![
                    text(&item.label).font(Font::MONOSPACE).size(13),
                    space::horizontal(),
                    text(&item.detail)
                        .size(11)
                        .style(|theme: &Theme| text::Style {
                            color: Some(Color {
                                a: 0.6,
                                ..theme.extended_palette().background.base.text
                            }),
                        }),
                ]
                .spacing(12),
            )
            .on_press(on_pick(i))
            .width(Length::Fill)
            .height(POPUP_ROW)
            .padding([2, 8])
            .style(move |theme: &Theme, status| {
                let p = theme.extended_palette();
                let background = match (selected, status) {
                    (true, _) => p.primary.weak.color,
                    (false, button::Status::Hovered) => p.background.weak.color,
                    _ => Color::TRANSPARENT,
                };
                button::Style {
                    background: Some(background.into()),
                    text_color: if selected {
                        p.primary.weak.text
                    } else {
                        p.background.base.text
                    },
                    ..button::Style::default()
                }
            }),
        );
    }
    let height = (c.items.len().min(POPUP_ROWS) as f32) * POPUP_ROW;
    container(scrollable(list).height(height))
        .width(380)
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
        })
        .into()
}

fn gutter<'a, M: 'a>(line_count: usize, digits: usize, error_lines: &[usize]) -> Element<'a, M> {
    let number = |n: usize| format!("{:>digits$}\n", n + 1);
    let red = Color::from_rgb(0.95, 0.35, 0.35);

    // Agrupa linhas consecutivas no mesmo estado em um único trecho.
    let mut spans: Vec<iced::widget::text::Span<'_, (), Font>> = Vec::new();
    let mut current = String::new();
    let mut current_error = false;
    for line in 0..line_count {
        let is_error = error_lines.contains(&line);
        if is_error != current_error && !current.is_empty() {
            let chunk = std::mem::take(&mut current);
            spans.push(if current_error {
                span(chunk).color(red)
            } else {
                span(chunk)
            });
        }
        current_error = is_error;
        current.push_str(&number(line));
    }
    if !current.is_empty() {
        spans.push(if current_error {
            span(current).color(red)
        } else {
            span(current)
        });
    }

    container(
        rich_text(spans)
            .font(Font::MONOSPACE)
            .size(FONT_SIZE)
            .line_height(LineHeight::Absolute(Pixels(LINE_HEIGHT))),
    )
    .padding([PADDING, GUTTER_PADDING])
    .style(|theme: &Theme| {
        let palette = theme.extended_palette();
        container::Style {
            text_color: Some(Color {
                a: 0.5,
                ..palette.background.base.text
            }),
            background: Some(palette.background.weak.color.into()),
            ..container::Style::default()
        }
    })
    .into()
}

fn key_binding<M: Clone>(
    press: KeyPress,
    on_command: fn(Command) -> M,
    completion: Option<(&CompletionKeys<M>, bool)>,
) -> Option<Binding<M>> {
    if !matches!(press.status, Status::Focused { .. }) {
        return None;
    }
    // Setas ativam/navegam a lista; Enter/Tab só aceitam com a lista ativa,
    // senão mantêm o comportamento normal do editor.
    if let Some((keys, active)) = completion {
        let custom = match press.key.as_ref() {
            Key::Named(key::Named::ArrowUp) if press.modifiers.is_empty() => Some(keys.up.clone()),
            Key::Named(key::Named::ArrowDown) if press.modifiers.is_empty() => {
                Some(keys.down.clone())
            }
            Key::Named(key::Named::Enter | key::Named::Tab)
                if active && press.modifiers.is_empty() =>
            {
                Some(keys.accept.clone())
            }
            Key::Named(key::Named::Escape) => Some(keys.close.clone()),
            _ => None,
        };
        if let Some(message) = custom {
            return Some(Binding::Custom(message));
        }
    }
    if let Some(command) = commands::lookup(&press.key, press.physical_key, press.modifiers) {
        return Some(Binding::Custom(on_command(command)));
    }
    if matches!(press.key.as_ref(), Key::Named(key::Named::Tab)) && !press.modifiers.shift() {
        return Some(Binding::Sequence(
            INDENT.chars().map(Binding::Insert).collect(),
        ));
    }
    Binding::from_key_press(press)
}

/// Novo deslocamento vertical que mantém a linha do cursor visível, se ela
/// estiver fora da área rolada.
pub fn scroll_to_show(line: usize, viewport: Option<&Viewport>) -> Option<f32> {
    let viewport = viewport?;
    let top = viewport.absolute_offset().y;
    let height = viewport.bounds().height;
    let line_top = PADDING + line as f32 * LINE_HEIGHT;
    let line_bottom = line_top + LINE_HEIGHT + PADDING;
    if line_top < top {
        Some((line_top - PADDING).max(0.0))
    } else if line_bottom > top + height {
        Some(line_bottom - height)
    } else {
        None
    }
}
