// SPDX-FileCopyrightText: 2026 Vitor Marinho Faria
// SPDX-License-Identifier: GPL-3.0-or-later

//! Grade de resultados virtualizada (spec §5.1).
//!
//! Só as linhas visíveis (mais uma margem) viram widgets; espaços em branco
//! acima e abaixo preservam a altura total para a barra de rolagem. O
//! cabeçalho fica num `scrollable` horizontal próprio, sem barra, sincronizado
//! com o deslocamento horizontal do corpo.

use iced::widget::scrollable::{Direction, Scrollbar, Viewport};
use iced::widget::text::Wrapping;
use iced::widget::{
    Column, Id, Row, column, container, mouse_area, scrollable, space, text, text_input,
};
use iced::{Color, Element, Font, Length, Theme};

use std::collections::BTreeSet;

use crate::data_edit::Pending;
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
    pub offset_x: f32,
    pub offset_y: f32,
    pub viewport_width: f32,
    pub viewport_height: f32,
}

/// Linhas selecionadas pela coluna de números (transitório, não persistido).
#[derive(Debug, Clone, Default)]
pub struct RowSelection {
    pub rows: BTreeSet<usize>,
    /// Última linha clicada sem Shift: origem do intervalo com Shift.
    pub anchor: Option<usize>,
    /// Linha atual (último clique ou seta), ponto de partida das setas.
    pub cursor: Option<usize>,
}

impl RowSelection {
    /// Clique no número da linha: sozinho seleciona só ela; com Ctrl alterna;
    /// com Shift seleciona o intervalo a partir da âncora (somando à seleção
    /// se Ctrl também estiver pressionado).
    pub fn click(&mut self, row: usize, ctrl: bool, shift: bool) {
        match (shift, self.anchor) {
            (true, Some(anchor)) => {
                if !ctrl {
                    self.rows.clear();
                }
                self.rows.extend(anchor.min(row)..=anchor.max(row));
                self.cursor = Some(row);
            }
            _ if ctrl => {
                if !self.rows.remove(&row) {
                    self.rows.insert(row);
                }
                self.anchor = Some(row);
                self.cursor = Some(row);
            }
            _ => {
                self.rows.clear();
                self.rows.insert(row);
                self.anchor = Some(row);
                self.cursor = Some(row);
            }
        }
    }

    pub fn clear(&mut self) {
        self.rows.clear();
        self.anchor = None;
        self.cursor = None;
    }
}

/// Tecla de navegação na grade focada.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arrow {
    Up,
    Down,
    Left,
    Right,
}

/// Move a seleção da grade com as setas. Com uma célula selecionada, move a
/// célula (entre as colunas visíveis, na ordem exibida). Com linhas
/// selecionadas, ↑/↓ movem a linha e ←/→ passam a selecionar a célula da
/// primeira coluna. Shift+↑/↓ estende a seleção de linhas a partir da âncora.
/// Sem seleção, começa pela primeira célula. Devolve a célula a mostrar.
pub fn navigate(
    selected: &mut Option<(usize, usize)>,
    rows: &mut RowSelection,
    order: &[usize],
    row_count: usize,
    arrow: Arrow,
    shift: bool,
) -> Option<(usize, usize)> {
    let (&first, &last) = (order.first()?, order.last()?);
    if row_count == 0 {
        return None;
    }
    let (row, col) = match (*selected, rows.cursor) {
        (Some(cell), _) => cell,
        (None, Some(row)) => (row, first),
        (None, None) => {
            *selected = Some((0, first));
            rows.clear();
            return *selected;
        }
    };
    let row = row.min(row_count - 1);
    let vertical = |row: usize| match arrow {
        Arrow::Up => row.saturating_sub(1),
        Arrow::Down => (row + 1).min(row_count - 1),
        _ => row,
    };
    let rows_mode = selected.is_none();
    match arrow {
        Arrow::Up | Arrow::Down if shift => {
            let target = vertical(row);
            let anchor = rows.anchor.filter(|_| rows_mode).unwrap_or(row);
            rows.rows = (anchor.min(target)..=anchor.max(target)).collect();
            rows.anchor = Some(anchor);
            rows.cursor = Some(target);
            *selected = None;
            Some((target, col))
        }
        Arrow::Up | Arrow::Down if rows_mode => {
            let target = vertical(row);
            rows.click(target, false, false);
            Some((target, col))
        }
        Arrow::Up | Arrow::Down => {
            *selected = Some((vertical(row), col));
            *selected
        }
        Arrow::Left | Arrow::Right => {
            let position = order.iter().position(|c| *c == col).unwrap_or(0);
            let col = if rows_mode {
                first
            } else if arrow == Arrow::Left {
                order[position.saturating_sub(1)]
            } else {
                order.get(position + 1).copied().unwrap_or(last)
            };
            rows.clear();
            *selected = Some((row, col));
            *selected
        }
    }
}

/// Novo deslocamento para que o trecho `[start, start + len)` fique visível
/// numa área de tamanho `view` rolada até `offset`; `None` se já estiver.
pub fn reveal(start: f32, len: f32, offset: f32, view: f32) -> Option<f32> {
    if view <= 0.0 {
        return None;
    }
    if start < offset {
        Some(start)
    } else if start + len > offset + view {
        Some((start + len - view).max(0.0))
    } else {
        None
    }
}

/// Deslocamento (x, y) que mostra a célula, se ela estiver fora da área visível.
pub fn reveal_cell(
    scroll: &ScrollState,
    widths: &[f32],
    order: &[usize],
    (row, col): (usize, usize),
) -> (Option<f32>, Option<f32>) {
    let y = reveal(
        row as f32 * ROW_HEIGHT,
        ROW_HEIGHT,
        scroll.offset_y,
        scroll.viewport_height,
    );
    let before: f32 = order
        .iter()
        .take_while(|c| **c != col)
        .map(|c| widths.get(*c).copied().unwrap_or(0.0))
        .sum();
    let width = widths.get(col).copied().unwrap_or(0.0);
    // A coluna de números rola junto; ao voltar para a primeira coluna, mostra-a.
    let start = if before == 0.0 {
        0.0
    } else {
        ROW_NUMBER_WIDTH + before
    };
    let x = reveal(
        start,
        ROW_NUMBER_WIDTH + before + width - start,
        scroll.offset_x,
        scroll.viewport_width,
    );
    (x, y)
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
    /// Colunas do resultado visíveis, na ordem exibida.
    pub order: &'a [usize],
    /// Coluna ordenada (nome) e se é descendente.
    pub sort: Option<(&'a str, bool)>,
    /// Cabeçalho pressionado (arraste em andamento).
    pub pressed: Option<usize>,
    pub on_scroll: fn(Viewport) -> M,
    pub on_select: fn(usize, usize) -> M,
    /// Botão esquerdo pressionado/solto sobre um cabeçalho: clique ordena,
    /// soltar sobre outro cabeçalho move a coluna.
    pub on_header_press: fn(usize) -> M,
    pub on_header_release: fn(usize) -> M,
    /// Botão do meio sobre um cabeçalho oculta a coluna.
    pub on_header_middle: fn(usize) -> M,
    /// Alterações pendentes (linha, coluna) → novo valor (`None` = NULL).
    pub pending: &'a Pending,
    /// Célula em edição: linha, coluna, texto e id do campo.
    pub editing: Option<(usize, usize, &'a str, Id)>,
    pub on_double_click: fn(usize, usize) -> M,
    /// Linhas selecionadas e clique no número da linha.
    pub selected_rows: &'a BTreeSet<usize>,
    pub on_row_select: fn(usize) -> M,
    pub on_edit_input: fn(String) -> M,
    pub on_edit_submit: M,
}

pub fn view<'a, M: Clone + 'a>(g: GridView<'a, M>) -> Element<'a, M> {
    let total_width: f32 = ROW_NUMBER_WIDTH + g.order.iter().map(|c| g.widths[*c]).sum::<f32>();

    let mut header = Row::new().push(header_cell(
        "#".into(),
        String::new(),
        ROW_NUMBER_WIDTH,
        false,
    ));
    for &c in g.order {
        let col = &g.result.columns[c];
        let mut name = col.name.clone();
        if let Some((sorted, desc)) = g.sort {
            if sorted == col.name {
                name.push_str(if desc { " ▼" } else { " ▲" });
            }
        }
        let cell = header_cell(
            name,
            col.type_name.clone(),
            g.widths[c],
            g.pressed == Some(c),
        );
        header = header.push(
            mouse_area(cell)
                .on_press((g.on_header_press)(c))
                .on_release((g.on_header_release)(c))
                .on_middle_press((g.on_header_middle)(c))
                .interaction(iced::mouse::Interaction::Pointer),
        );
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

fn header_cell<'a, M: 'a>(
    name: String,
    type_name: String,
    width: f32,
    pressed: bool,
) -> Element<'a, M> {
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
    .style(move |theme: &Theme| {
        let p = theme.extended_palette();
        container::Style {
            background: Some(
                if pressed {
                    p.primary.weak.color
                } else {
                    p.background.weak.color
                }
                .into(),
            ),
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
    let row_changed = g.pending.range((index, 0)..(index + 1, 0)).next().is_some();
    let label = if row_changed {
        format!("● {}", index + 1)
    } else {
        (index + 1).to_string()
    };
    let row_selected = g.selected_rows.contains(&index);
    let number = container(text(label).size(12).font(Font::MONOSPACE).style(
        move |theme: &Theme| {
            if row_selected {
                text::Style {
                    color: Some(theme.extended_palette().primary.strong.text),
                }
            } else {
                muted(theme)
            }
        },
    ))
    .width(ROW_NUMBER_WIDTH)
    .height(ROW_HEIGHT)
    .padding([3, 6])
    .style(move |theme: &Theme| row_number_style(theme, row_selected));
    let mut r = Row::new().height(ROW_HEIGHT).push(
        mouse_area(number)
            .on_press((g.on_row_select)(index))
            .interaction(iced::mouse::Interaction::Pointer),
    );
    for &col in g.order {
        let width = &g.widths[col];
        let selected = g.selected == Some((index, col)) || row_selected;
        if let Some((_, _, current, id)) = g.editing.as_ref().filter(|e| e.0 == index && e.1 == col)
        {
            let input = text_input("", current)
                .id(id.clone())
                .on_input(g.on_edit_input)
                .on_submit(g.on_edit_submit.clone())
                .font(Font::MONOSPACE)
                .size(13)
                .padding([2, 4])
                .width(*width);
            r = r.push(container(input).width(*width).height(ROW_HEIGHT));
            continue;
        }
        let changed = g.pending.get(&(index, col));
        let display_null = Value::Null;
        let edited_value;
        let value = match changed {
            Some(None) => &display_null,
            Some(Some(text)) => {
                edited_value = Value::Text(text.clone());
                &edited_value
            }
            None => &g.result.rows[index][col],
        };
        let changed = changed.is_some();
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
            .style(move |theme: &Theme| cell_style(theme, index, selected, changed));
        r = r.push(
            mouse_area(cell)
                .on_press((g.on_select)(index, col))
                .on_double_click((g.on_double_click)(index, col)),
        );
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

fn row_number_style(theme: &Theme, selected: bool) -> container::Style {
    let p = theme.extended_palette();
    container::Style {
        background: Some(
            if selected {
                p.primary.strong.color
            } else {
                p.background.weaker.color
            }
            .into(),
        ),
        ..container::Style::default()
    }
}

fn cell_style(theme: &Theme, row: usize, selected: bool, changed: bool) -> container::Style {
    let p = theme.extended_palette();
    let background = if changed && selected {
        Color {
            a: 0.55,
            ..p.warning.base.color
        }
    } else if changed {
        Color {
            a: 0.3,
            ..p.warning.base.color
        }
    } else if selected {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(s: &RowSelection) -> Vec<usize> {
        s.rows.iter().copied().collect()
    }

    #[test]
    fn row_selection_follows_ctrl_and_shift() {
        let mut s = RowSelection::default();
        s.click(3, false, false);
        assert_eq!(rows(&s), vec![3]);
        s.click(5, true, false);
        assert_eq!(rows(&s), vec![3, 5]);
        s.click(3, true, false);
        assert_eq!(rows(&s), vec![5]);
        // Âncora na 3 (último Ctrl+clique): Shift seleciona 1..=3.
        s.click(1, false, true);
        assert_eq!(rows(&s), vec![1, 2, 3]);
        // Ctrl+Shift soma o intervalo à seleção.
        s.click(6, true, true);
        assert_eq!(rows(&s), vec![1, 2, 3, 4, 5, 6]);
        s.click(8, false, false);
        assert_eq!(rows(&s), vec![8]);
        s.clear();
        // Shift sem âncora seleciona só a linha.
        s.click(2, false, true);
        assert_eq!(rows(&s), vec![2]);
    }

    #[test]
    fn arrows_move_cells_and_rows() {
        let order = [2, 0, 1];
        let mut cell = None;
        let mut sel = RowSelection::default();
        let go = |cell: &mut Option<(usize, usize)>, sel: &mut RowSelection, a, shift| {
            navigate(cell, sel, &order, 5, a, shift)
        };
        // Sem seleção: primeira célula exibida.
        assert_eq!(go(&mut cell, &mut sel, Arrow::Down, false), Some((0, 2)));
        assert_eq!(go(&mut cell, &mut sel, Arrow::Right, false), Some((0, 0)));
        assert_eq!(go(&mut cell, &mut sel, Arrow::Right, false), Some((0, 1)));
        assert_eq!(go(&mut cell, &mut sel, Arrow::Right, false), Some((0, 1)));
        assert_eq!(go(&mut cell, &mut sel, Arrow::Up, false), Some((0, 1)));
        assert_eq!(go(&mut cell, &mut sel, Arrow::Down, false), Some((1, 1)));
        // Shift+↓ passa a selecionar linhas a partir da linha da célula.
        go(&mut cell, &mut sel, Arrow::Down, true);
        go(&mut cell, &mut sel, Arrow::Down, true);
        assert_eq!(cell, None);
        assert_eq!(rows(&sel), vec![1, 2, 3]);
        go(&mut cell, &mut sel, Arrow::Up, true);
        assert_eq!(rows(&sel), vec![1, 2]);
        // ↓ sem Shift: só a próxima linha.
        go(&mut cell, &mut sel, Arrow::Down, false);
        assert_eq!(rows(&sel), vec![3]);
        // ← volta para a célula da primeira coluna, na linha atual.
        assert_eq!(go(&mut cell, &mut sel, Arrow::Left, false), Some((3, 2)));
        assert!(sel.rows.is_empty());
        // Fim do resultado.
        cell = Some((4, 0));
        assert_eq!(go(&mut cell, &mut sel, Arrow::Down, false), Some((4, 0)));
    }

    #[test]
    fn reveal_keeps_cell_visible() {
        assert_eq!(reveal(100.0, 24.0, 0.0, 200.0), None);
        assert_eq!(reveal(300.0, 24.0, 0.0, 200.0), Some(124.0));
        assert_eq!(reveal(48.0, 24.0, 100.0, 200.0), Some(48.0));
        let scroll = ScrollState {
            offset_x: 0.0,
            offset_y: 0.0,
            viewport_width: 300.0,
            viewport_height: 240.0,
        };
        let widths = [100.0, 200.0, 150.0];
        let order = [0, 1, 2];
        // Coluna 2 termina em 56 + 300 + 150 = 506: rola até 506 - 300.
        assert_eq!(
            reveal_cell(&scroll, &widths, &order, (20, 2)),
            (Some(206.0), Some(24.0 * 21.0 - 240.0))
        );
        assert_eq!(reveal_cell(&scroll, &widths, &order, (0, 0)), (None, None));
    }
}
