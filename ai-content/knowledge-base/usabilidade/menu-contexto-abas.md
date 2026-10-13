# Menu de contexto das abas

**Regra:** clique direito numa aba (Scratch, tabela ou arquivo) do workspace
ativo abre um menu no ponto do cursor com: Fechar aba, Fechar abas à esquerda,
Fechar abas à direita, Fechar tudo. O × da aba continua fechando direto, sem
menu; clique esquerdo continua selecionando a aba.

- O Scratch nunca fecha: as ações o ignoram e itens sem nada a fechar ficam
  desabilitados (ex.: "Fechar aba" no Scratch, "à esquerda" na primeira aba
  depois dele).
- Se alguma aba a fechar tiver alterações não salvas, um único diálogo lista
  todas: Salvar (salva as sujas e fecha tudo), Descartar (fecha tudo) ou
  Cancelar (não fecha nenhuma). Ctrl+W e o × usam o mesmo caminho
  (`request_close_tabs` em `src/app/mod.rs`).
- Clicar fora (qualquer botão), Esc ou qualquer tecla fecha o menu.

**Implementação:** o `mouse_area` do Iced não informa a posição do clique e,
dentro do `scrollable` da barra de abas, o cursor vem deslocado pela rolagem.
`src/app/right_click.rs` envolve a barra por fora do `scrollable` e publica
`RightClickAt(posição)` antes do `TabMenu(índice)` do `mouse_area` da aba. O
menu é a terceira camada fixa do `stack` raiz (ver
[estabilidade da árvore](../arquitetura/iced-estabilidade-da-arvore.md)).

Referência: pedido do usuário em 2026-10-13.
