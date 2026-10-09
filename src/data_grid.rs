//! Grade de resultados virtualizada (spec §5.1).
//!
//! Só as linhas visíveis (mais uma margem) viram widgets; espaços em branco
//! acima e abaixo preservam a altura total para a barra de rolagem. O
//! cabeçalho fica num `scrollable` horizontal próprio, sem barra, sincronizado
//! com o deslocamento horizontal do corpo.

use iced::widget::scrollable::{Direction, Scrollbar, Viewport};
use iced::widget::text::Wrapping;
use iced::widget::{Column, Id, Row, column, container, mouse_area, scrollable, space, text};
use iced::{Color, Element, Font, Length, Theme};

use crate::query::{ResultSet, Value};

pub const ROW_HEIGHT: f32 = 24.0;
const HEADER_HEIGHT: f32 = 40.0;
const ROW_NUMBER_WIDTH: f32 = 56.0;
const CHAR_WIDTH: f32 = 7.5;
const MIN_WIDTH: f32 = 60.0;
const MAX_WIDTH: f32 = 360.0;
const OVERSCAN: usize = 10;
const MAX_CELL_CHARS: usize = 200;

/// Estado de rolagem da grade (transitório, não persistido).
#[derive(Debug, Clone, Default)]
pub struct ScrollState {
    pub offset_y: f32,
    pub viewport_height: f32,
}

/// Largura de cada coluna, estimada pelo cabeçalho e por uma amostra de linhas.
pub fn column_widths(result: &ResultSet) -> Vec<f32> {
    result
        .columns
        .iter()
        .enumerate()
        .map(|(i, col)| {
            let header = col.name.chars().count().max(col.type_name.chars().count());
            let sample = result
                .rows
                .iter()
                .take(200)
                .map(|r| {
                    r.get(i).map_or(4, |v| {
                        if v.is_null() {
                            4
                        } else {
                            cell_text(v).chars().count()
                        }
                    })
                })
                .max()
                .unwrap_or(0);
            (header.max(sample) as f32 * CHAR_WIDTH + 20.0).clamp(MIN_WIDTH, MAX_WIDTH)
        })
        .collect()
}

pub fn cell_text(value: &Value) -> String {
    let raw = value.display();
    let mut out: String = raw
        .chars()
        .take(MAX_CELL_CHARS)
        .map(|c| if c == '\n' { '↵' } else { c })
        .collect();
    if raw.chars().count() > MAX_CELL_CHARS {
        out.push('…');
    }
    out
}

pub struct GridView<'a, M> {
    pub result: &'a ResultSet,
    pub widths: &'a [f32],
    pub scroll: &'a ScrollState,
    pub header_id: Id,
    pub body_id: Id,
    pub selected: Option<(usize, usize)>,
    pub on_scroll: fn(Viewport) -> M,
    pub on_select: fn(usize, usize) -> M,
}

pub fn view<'a, M: Clone + 'a>(g: GridView<'a, M>) -> Element<'a, M> {
    let total_width: f32 = ROW_NUMBER_WIDTH + g.widths.iter().sum::<f32>();

    let mut header = Row::new().push(header_cell("#".into(), String::new(), ROW_NUMBER_WIDTH));
    for (col, width) in g.result.columns.iter().zip(g.widths) {
        header = header.push(header_cell(col.name.clone(), col.type_name.clone(), *width));
    }
    let header = scrollable(header)
        .id(g.header_id.clone())
        .direction(Direction::Horizontal(Scrollbar::hidden()))
        .width(Length::Fill);

    let rows = g.result.rows.len();
    let viewport = if g.scroll.viewport_height > 0.0 {
        g.scroll.viewport_height
    } else {
        800.0
    };
    let first = ((g.scroll.offset_y / ROW_HEIGHT) as usize)
        .saturating_sub(OVERSCAN)
        .min(rows);
    let visible = (viewport / ROW_HEIGHT).ceil() as usize + 2 * OVERSCAN;
    let last = (first + visible).min(rows);

    let mut body = Column::new().width(total_width);
    body = body.push(space().height(first as f32 * ROW_HEIGHT));
    for index in first..last {
        body = body.push(data_row(&g, index));
    }
    body = body.push(space().height((rows - last) as f32 * ROW_HEIGHT));

    let body = scrollable(body)
        .id(g.body_id.clone())
        .direction(Direction::Both {
            vertical: Scrollbar::default(),
            horizontal: Scrollbar::default(),
        })
        .on_scroll(g.on_scroll)
        .width(Length::Fill)
        .height(Length::Fill);

    column![header, body].into()
}

fn header_cell<'a, M: 'a>(name: String, type_name: String, width: f32) -> Element<'a, M> {
    container(
        column![
            text(name)
                .size(13)
                .font(Font {
                    weight: iced::font::Weight::Bold,
                    ..Font::default()
                })
                .wrapping(Wrapping::None),
            text(type_name)
                .size(11)
                .style(muted)
                .wrapping(Wrapping::None),
        ]
        .spacing(1),
    )
    .width(width)
    .height(HEADER_HEIGHT)
    .padding([3, 6])
    .clip(true)
    .style(|theme: &Theme| {
        let p = theme.extended_palette();
        container::Style {
            background: Some(p.background.weak.color.into()),
            border: iced::Border {
                color: p.background.strong.color,
                width: 0.5,
                radius: 0.0.into(),
            },
            ..container::Style::default()
        }
    })
    .into()
}

fn data_row<'a, M: Clone + 'a>(g: &GridView<'a, M>, index: usize) -> Element<'a, M> {
    let mut r = Row::new().height(ROW_HEIGHT).push(
        container(
            text((index + 1).to_string())
                .size(12)
                .style(muted)
                .font(Font::MONOSPACE),
        )
        .width(ROW_NUMBER_WIDTH)
        .height(ROW_HEIGHT)
        .padding([3, 6])
        .style(row_number_style),
    );
    for (col, width) in g.widths.iter().enumerate() {
        let value = &g.result.rows[index][col];
        let selected = g.selected == Some((index, col));
        let content: Element<'a, M> = if value.is_null() {
            text("NULL")
                .size(13)
                .font(Font {
                    style: iced::font::Style::Italic,
                    ..Font::MONOSPACE
                })
                .style(muted)
                .into()
        } else {
            text(cell_text(value))
                .size(13)
                .font(Font::MONOSPACE)
                .wrapping(Wrapping::None)
                .into()
        };
        let cell = container(content)
            .width(*width)
            .height(ROW_HEIGHT)
            .padding([3, 6])
            .clip(true)
            .style(move |theme: &Theme| cell_style(theme, index, selected));
        r = r.push(mouse_area(cell).on_press((g.on_select)(index, col)));
    }
    r.into()
}

fn muted(theme: &Theme) -> text::Style {
    let p = theme.extended_palette();
    text::Style {
        color: Some(Color {
            a: 0.6,
            ..p.background.base.text
        }),
    }
}

fn row_number_style(theme: &Theme) -> container::Style {
    let p = theme.extended_palette();
    container::Style {
        background: Some(p.background.weaker.color.into()),
        ..container::Style::default()
    }
}

fn cell_style(theme: &Theme, row: usize, selected: bool) -> container::Style {
    let p = theme.extended_palette();
    let background = if selected {
        p.primary.weak.color
    } else if row % 2 == 1 {
        p.background.weakest.color
    } else {
        Color::TRANSPARENT
    };
    container::Style {
        background: Some(background.into()),
        text_color: selected.then_some(p.primary.weak.text),
        border: iced::Border {
            color: p.background.weak.color,
            width: 0.5,
            radius: 0.0.into(),
        },
        ..container::Style::default()
    }
}

/// `true` quando a rolagem chegou perto do fim das linhas carregadas.
pub fn near_end(viewport: &Viewport) -> bool {
    let offset = viewport.absolute_offset().y;
    let visible = viewport.bounds().height;
    let content = viewport.content_bounds().height;
    content - (offset + visible) < ROW_HEIGHT * 20.0
}
