//! Editor SQL: highlight, números de linha, marcação de erro e atalhos.
//!
//! O `text_editor` do Iced não tem gutter próprio. O editor é renderizado com
//! altura igual ao conteúdo dentro de um `scrollable`, ao lado de uma coluna de
//! números com a mesma altura de linha; assim os dois rolam juntos. Por isso
//! as linhas não quebram (`Wrapping::None`).

use iced::keyboard::key;
use iced::widget::scrollable::Viewport;
use iced::widget::text::LineHeight;
use iced::widget::text_editor::{self, Binding, Content, KeyPress, Status};
use iced::widget::{Id, container, rich_text, row, scrollable, span, text_editor as editor};
use iced::{Element, Font, Length, Pixels};

use crate::commands::{self, Command};

pub const FONT_SIZE: f32 = 14.0;
pub const LINE_HEIGHT: f32 = 20.0;
pub const PADDING: f32 = 6.0;
const INDENT: &str = "    ";

pub struct EditorView<'a, M> {
    pub content: &'a Content,
    pub scroll_id: Id,
    /// Linha (a partir de 0) a destacar no gutter, ex.: erro da última execução.
    pub error_line: Option<usize>,
    pub on_action: fn(text_editor::Action) -> M,
    pub on_command: fn(Command) -> M,
    pub on_scroll: fn(Viewport) -> M,
    pub read_only_hint: Option<&'a str>,
    /// Altura mínima, para que a área clicável preencha o painel.
    pub min_height: f32,
}

pub fn view<'a, M: Clone + 'a>(v: EditorView<'a, M>) -> Element<'a, M> {
    let on_command = v.on_command;
    let line_count = v.content.line_count().max(1);

    let gutter = gutter(line_count, v.error_line);

    let input = editor(v.content)
        .font(Font::MONOSPACE)
        .size(FONT_SIZE)
        .line_height(LineHeight::Absolute(Pixels(LINE_HEIGHT)))
        .padding(PADDING)
        .height(Length::Shrink)
        .min_height(v.min_height)
        .wrapping(iced::widget::text::Wrapping::None)
        .placeholder(
            v.read_only_hint
                .unwrap_or("Escreva SQL aqui. Ctrl+Enter executa a instrução atual."),
        )
        .highlight("sql", iced::highlighter::Theme::Base16Mocha)
        .key_binding(move |press: KeyPress| key_binding(press, on_command))
        .on_action(v.on_action);

    scrollable(row![gutter, input].width(Length::Fill))
        .id(v.scroll_id)
        .on_scroll(v.on_scroll)
        .height(Length::Fill)
        .width(Length::Fill)
        .into()
}

fn gutter<'a, M: 'a>(line_count: usize, error_line: Option<usize>) -> Element<'a, M> {
    let digits = line_count.to_string().len().max(2);
    let numbers = |range: std::ops::Range<usize>| -> String {
        range.map(|n| format!("{:>digits$}\n", n + 1)).collect()
    };

    let spans: Vec<iced::widget::text::Span<'_, (), Font>> =
        match error_line.filter(|l| *l < line_count) {
            Some(line) => vec![
                span(numbers(0..line)),
                span(numbers(line..line + 1)).color(iced::Color::from_rgb(0.95, 0.35, 0.35)),
                span(numbers(line + 1..line_count)),
            ],
            None => vec![span(numbers(0..line_count))],
        };

    container(
        rich_text(spans)
            .font(Font::MONOSPACE)
            .size(FONT_SIZE)
            .line_height(LineHeight::Absolute(Pixels(LINE_HEIGHT))),
    )
    .padding([PADDING, 8.0])
    .style(|theme: &iced::Theme| {
        let palette = theme.extended_palette();
        container::Style {
            text_color: Some(iced::Color {
                a: 0.5,
                ..palette.background.base.text
            }),
            background: Some(palette.background.weak.color.into()),
            ..container::Style::default()
        }
    })
    .into()
}

fn key_binding<M: Clone>(press: KeyPress, on_command: fn(Command) -> M) -> Option<Binding<M>> {
    if !matches!(press.status, Status::Focused { .. }) {
        return None;
    }
    if let Some(command) = commands::lookup(&press.key, press.physical_key, press.modifiers) {
        return Some(Binding::Custom(on_command(command)));
    }
    if matches!(
        press.key.as_ref(),
        iced::keyboard::Key::Named(key::Named::Tab)
    ) && !press.modifiers.shift()
    {
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
