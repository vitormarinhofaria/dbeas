# Iced: manter a estrutura da árvore de widgets estável

**Regra:** um widget com estado interno (foco do `text_editor`/`text_input`,
deslocamento de `scrollable`, etc.) precisa ficar na **mesma posição e com o
mesmo tipo** entre um `view` e o seguinte. Para mostrar algo condicional por cima
(popup, modal), use sempre o mesmo contêiner (`stack![base, camada]`) e troque
apenas a camada (`Space` quando vazia); para conteúdo condicional numa coluna,
acrescente filhos **depois** dos widgets com estado.

**Motivo:** `iced_core::widget::Tree::diff` compara a `tag` do widget; se o tipo
mudar naquela posição, o estado é recriado do zero (`*self = Self::new(new)`).

**Bug que originou a regra (2026-10-09):** o editor SQL retornava o `scrollable`
sozinho sem sugestões e um `stack` com a lista quando havia sugestões. Ao aparecer
a lista, o `text_editor` era recriado sem foco: o usuário não conseguia continuar
digitando, nem navegar ou aceitar sugestões. A raiz da janela tinha o mesmo
problema ao abrir modais e o Ctrl+P (perdia foco e rolagens). Corrigido em
`src/sql_editor.rs` e `src/app/view.rs`.

## Clique capturado não tira o foco de widgets abaixo

Um `stack` para de repassar o evento às camadas de baixo assim que
`shell.is_event_captured()` (`iced_widget::stack::update`), mesmo que quem
capturou esteja fora dele. A `column` repassa a todos, mas o shell é
compartilhado: na aba, a grade vem **antes** do editor, então o clique numa célula
(capturado pelo `mouse_area`) chega ao `stack` do editor SQL já capturado, e o
`text_editor` nunca vê o clique de fora que o desfocaria. Bug de 2026-10-13: Del
na grade apagava texto no editor. Solução: ao clicar na grade, tirar o foco com
`operate(focusable::unfocus())` (`App::focus_grid`).

**Teste de interface:** o ambiente é KDE/Wayland. Teclas sintéticas via XTEST
(app rodando com `env -u WAYLAND_DISPLAY WINIT_UNIX_BACKEND=x11`) disparam o
diálogo de permissão "Controle remoto" do KDE — precisa de aprovação do usuário.

## Capturas de tela para verificação

- `spectacle -a` captura a **janela ativa**: se o usuário estiver em outro
  programa, captura a tela dele. Não usar sem a janela do DBeas em foco; nunca usar
  `spectacle -r` (espera seleção do usuário e trava).
- Preferir captura interna num gancho temporário: `window::latest()
  .and_then(window::screenshot)` grava `Screenshot.rgba` (converter com PIL). A
  captura reflete o último quadro apresentado: espere (~400 ms), capture uma vez,
  espere de novo e use a segunda captura; antes disso o texto de widgets recém-
  criados pode sair em branco.
