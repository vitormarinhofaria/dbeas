# Configurações da aplicação (tema)

Seção **CONFIGURAÇÕES** na barra lateral, logo acima de Workspaces
(`App::app_settings` em `src/app/view.rs`). Clicar no título expande/recolhe.

## Tema

- Lista **Padrão (sistema)** em primeiro e depois todos os temas embutidos do Iced
  (`Theme::ALL`), via `settings::ThemeChoice`. **Padrão (sistema)** é a opção
  inicial (nada configurado): resolve para Light ou Dark conforme o modo do SO
  (`iced::system::theme()` no boot e `theme_changes()` na subscription;
  `App::system_mode`). `ICED_THEME` no ambiente sobrepõe o resultado (comportamento do Iced).
- Escolher um tema na lista **pré-visualiza** na interface inteira
  (`App::theme_preview`); aparecem **Aplicar** e **Reverter**. Recolher a seção
  também descarta a pré-visualização.
- **Aplicar** grava `theme` (nome via `Display`) em `~/.config/dbeasy/config`
  (`Settings`), persistindo entre sessões. Escolher o padrão remove o campo. Nome desconhecido ou ausente = Padrão (sistema).
- O realce de sintaxe do editor SQL continua fixo (`Base16Mocha`); não acompanha o tema.

Corpo da seção é sempre um widget (`Space` quando recolhida) para manter a árvore
estável — ver [Iced: estabilidade da árvore](../arquitetura/iced-estabilidade-da-arvore.md).
Decisão de 2026-10-21.

## Seção Workspaces recolhível

A seção **WORKSPACES** também recolhe/expande ao clicar no título
(`App::workspaces_open`, `Message::ToggleWorkspaces`; botão **+** continua no
cabeçalho). Começa **recolhida**: mostra só o ponto de status e o nome do
workspace ativo (`display_name`); expandida, mostra a lista completa. O corpo é
sempre um widget (lista ou resumo) para manter a árvore estável. Decisão de 2026-10-21.
