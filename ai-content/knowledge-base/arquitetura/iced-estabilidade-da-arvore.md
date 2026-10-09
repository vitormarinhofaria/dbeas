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

**Teste de interface:** o ambiente é KDE/Wayland. Teclas sintéticas via XTEST
(app rodando com `env -u WAYLAND_DISPLAY WINIT_UNIX_BACKEND=x11`) disparam o
diálogo de permissão "Controle remoto" do KDE — precisa de aprovação do usuário.
