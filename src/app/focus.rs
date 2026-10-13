// SPDX-FileCopyrightText: 2026 Vitor Marinho Faria
// SPDX-License-Identifier: GPL-3.0-or-later

//! Navegação por Tab restrita a um contêiner. O `focus_next` do Iced percorre a
//! árvore inteira (inclusive o editor SQL sob um modal), não volta ao início e
//! não rola até o campo focado.

use iced::Task;
use iced::advanced::widget::operation::focusable::{self, Count, Focusable};
use iced::advanced::widget::operation::scrollable::{AbsoluteOffset, Scrollable};
use iced::advanced::widget::operation::{self, Operation, Outcome};
use iced::advanced::widget::{self, Id};
use iced::{Rectangle, Vector, advanced};

/// Move o foco para o próximo (ou anterior) campo dentro do contêiner `scope`,
/// circularmente. Sem campo focado ali, foca o primeiro (ou o último). Se o
/// campo estiver dentro de um `scrollable` do contêiner, rola até ele.
pub fn cycle<T: Send + 'static>(scope: impl Into<Id>, forward: bool) -> Task<T> {
    let scope = scope.into();
    if forward {
        advanced::widget::operate(operation::scope(scope, cycle_op::<T, true>()))
    } else {
        advanced::widget::operate(operation::scope(scope, cycle_op::<T, false>()))
    }
}

// `operation::then` só aceita `fn`, sem captura: a direção vai como constante.
fn cycle_op<T: Send + 'static, const FORWARD: bool>() -> impl Operation<T> {
    operation::then(focusable::count(), |count| Cycle {
        count,
        current: 0,
        forward: FORWARD,
        viewport: None,
        reveal: None,
    })
}

/// Tira o foco de qualquer widget e foca o primeiro campo de `scope`.
pub fn first_in<T: Send + 'static>(scope: impl Into<Id>) -> Task<T> {
    advanced::widget::operate(focusable::unfocus()).chain(cycle(scope, true))
}

struct Cycle {
    count: Count,
    current: usize,
    forward: bool,
    /// `scrollable` mais recente: área visível e conteúdo (sem rolagem).
    viewport: Option<(Rectangle, Rectangle, Vector)>,
    /// Deslocamento vertical que deixa o campo focado visível.
    reveal: Option<f32>,
}

impl<T> Operation<T> for Cycle {
    fn focusable(
        &mut self,
        _id: Option<&widget::Id>,
        bounds: Rectangle,
        state: &mut dyn Focusable,
    ) {
        let total = self.count.total;
        let target = match (self.count.focused, self.forward) {
            (None, true) => 0,
            (None, false) => total - 1,
            (Some(f), true) => (f + 1) % total,
            (Some(f), false) => (f + total - 1) % total,
        };
        if self.current == target {
            state.focus();
            self.reveal = self
                .viewport
                .and_then(|(view, content, offset)| reveal(bounds, view, content, offset));
        } else if self.count.focused == Some(self.current) {
            state.unfocus();
        }
        self.current += 1;
    }

    fn scrollable(
        &mut self,
        _id: Option<&widget::Id>,
        bounds: Rectangle,
        content_bounds: Rectangle,
        translation: Vector,
        _state: &mut dyn Scrollable,
    ) {
        self.viewport = Some((bounds, content_bounds, translation));
    }

    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<T>)) {
        operate(self);
    }

    fn finish(&self) -> Outcome<T> {
        match self.reveal {
            Some(y) => Outcome::Chain(Box::new(ScrollTo(y))),
            None => Outcome::None,
        }
    }
}

/// Novo deslocamento para que `field` (em coordenadas do conteúdo sem rolagem)
/// apareça inteiro na área visível; `None` se já estiver visível.
fn reveal(field: Rectangle, view: Rectangle, content: Rectangle, offset: Vector) -> Option<f32> {
    let top = field.y - content.y;
    let bottom = top + field.height;
    if top < offset.y {
        Some(top)
    } else if bottom > offset.y + view.height {
        Some(bottom - view.height)
    } else {
        None
    }
}

/// Rola o primeiro `scrollable` encontrado até o deslocamento vertical dado.
struct ScrollTo(f32);

impl<T> Operation<T> for ScrollTo {
    fn scrollable(
        &mut self,
        _id: Option<&widget::Id>,
        _bounds: Rectangle,
        _content_bounds: Rectangle,
        _translation: Vector,
        state: &mut dyn Scrollable,
    ) {
        state.scroll_to(AbsoluteOffset {
            x: None,
            y: Some(self.0),
        });
    }

    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<T>)) {
        operate(self);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(y: f32, height: f32) -> Rectangle {
        Rectangle {
            x: 0.0,
            y,
            width: 100.0,
            height,
        }
    }

    #[test]
    fn reveal_scrolls_only_when_field_is_hidden() {
        let view = rect(100.0, 200.0);
        let content = rect(100.0, 1000.0);
        let at = |y| Vector::new(0.0, y);
        // Visível: nada muda.
        assert_eq!(reveal(rect(150.0, 30.0), view, content, at(0.0)), None);
        // Abaixo da área visível: alinha pela base.
        assert_eq!(
            reveal(rect(400.0, 30.0), view, content, at(0.0)),
            Some(130.0)
        );
        // Acima (já rolado): alinha pelo topo.
        assert_eq!(
            reveal(rect(120.0, 30.0), view, content, at(300.0)),
            Some(20.0)
        );
    }
}
