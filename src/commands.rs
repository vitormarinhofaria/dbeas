//! Mapa central de atalhos (spec §9).
//!
//! Todos os atalhos da aplicação são resolvidos aqui. O editor SQL consulta
//! este mapa antes das suas próprias teclas, de modo que um atalho da
//! aplicação vence a ação de edição padrão; os demais componentes não
//! definem atalhos próprios.

use iced::keyboard::{Key, Modifiers, key};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    /// Executa a seleção ou, sem seleção, a instrução sob o cursor.
    ExecuteCurrent,
    /// Executa todas as instruções do editor, em ordem.
    ExecuteScript,
    /// Salva o arquivo SQL da aba ativa.
    Save,
    /// Cancela a consulta em andamento na aba ativa.
    CancelQuery,
    QuickOpen,
    NextWorkspace,
    PreviousWorkspace,
    CloseTab,
    RefreshMetadata,
}

pub struct Shortcut {
    pub keys: &'static str,
    pub command: Command,
    pub description: &'static str,
}

/// Tabela exibida na ajuda e documentada em
/// `ai-content/knowledge-base/usabilidade/atalhos.md`.
pub const SHORTCUTS: &[Shortcut] = &[
    Shortcut {
        keys: "Ctrl+Enter",
        command: Command::ExecuteCurrent,
        description: "Executar seleção ou instrução atual",
    },
    Shortcut {
        keys: "Ctrl+Shift+Enter",
        command: Command::ExecuteScript,
        description: "Executar script inteiro",
    },
    Shortcut {
        keys: "Ctrl+S",
        command: Command::Save,
        description: "Salvar arquivo SQL",
    },
    Shortcut {
        keys: "Ctrl+.",
        command: Command::CancelQuery,
        description: "Cancelar consulta",
    },
    Shortcut {
        keys: "Ctrl+P",
        command: Command::QuickOpen,
        description: "Abrir tabela/view ou arquivo",
    },
    Shortcut {
        keys: "Ctrl+Tab",
        command: Command::NextWorkspace,
        description: "Próximo workspace",
    },
    Shortcut {
        keys: "Ctrl+Shift+Tab",
        command: Command::PreviousWorkspace,
        description: "Workspace anterior",
    },
    Shortcut {
        keys: "Ctrl+W",
        command: Command::CloseTab,
        description: "Fechar aba",
    },
    Shortcut {
        keys: "F5",
        command: Command::RefreshMetadata,
        description: "Atualizar metadados",
    },
];

pub fn lookup(key: &Key, physical: key::Physical, modifiers: Modifiers) -> Option<Command> {
    let ctrl = modifiers.command();
    let shift = modifiers.shift();
    let alt = modifiers.alt();
    if alt {
        return None;
    }

    match key.as_ref() {
        Key::Named(key::Named::Enter) if ctrl && shift => Some(Command::ExecuteScript),
        Key::Named(key::Named::Enter) if ctrl => Some(Command::ExecuteCurrent),
        Key::Named(key::Named::Tab) if ctrl && shift => Some(Command::PreviousWorkspace),
        Key::Named(key::Named::Tab) if ctrl => Some(Command::NextWorkspace),
        Key::Named(key::Named::F5) if !ctrl && !shift => Some(Command::RefreshMetadata),
        _ if ctrl && !shift => match key.to_latin(physical) {
            Some('s') => Some(Command::Save),
            Some('p') => Some(Command::QuickOpen),
            Some('w') => Some(Command::CloseTab),
            Some('.') => Some(Command::CancelQuery),
            _ => None,
        },
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced::keyboard::key::{Code, NativeCode, Physical};

    fn ctrl() -> Modifiers {
        Modifiers::CTRL
    }

    #[test]
    fn resolves_shortcuts() {
        let unknown = Physical::Unidentified(NativeCode::Unidentified);
        assert_eq!(
            lookup(&Key::Named(key::Named::Enter), unknown, ctrl()),
            Some(Command::ExecuteCurrent)
        );
        assert_eq!(
            lookup(
                &Key::Named(key::Named::Enter),
                unknown,
                ctrl() | Modifiers::SHIFT
            ),
            Some(Command::ExecuteScript)
        );
        assert_eq!(
            lookup(
                &Key::Character("p".into()),
                Physical::Code(Code::KeyP),
                ctrl()
            ),
            Some(Command::QuickOpen)
        );
        assert_eq!(
            lookup(&Key::Named(key::Named::Tab), unknown, ctrl()),
            Some(Command::NextWorkspace)
        );
        assert_eq!(
            lookup(
                &Key::Character("p".into()),
                Physical::Code(Code::KeyP),
                Modifiers::empty()
            ),
            None
        );
    }
}
