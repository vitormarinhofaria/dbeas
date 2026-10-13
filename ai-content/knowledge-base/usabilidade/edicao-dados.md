# Edição de dados pela grade

Spec §6.1–6.2. Código: `src/data_edit.rs` (regras puras, testadas),
`connection::apply_edits` (execução) e a seção "Edição de dados" de `src/app/mod.rs`.

## Quando um resultado é editável

- Todas as colunas que vêm de tabela (`table_oid` do protocolo) vêm da **mesma**.
- Ela é uma tabela (ou tabela particionada), não view.
- Tem chave primária e **todas** as colunas da chave estão no resultado,
  exatamente uma vez.
- O workspace não é `readOnly`.
- Colunas calculadas aparecem, mas não são editáveis. Sem chave primária não há
  edição (decisão: não usar `ctid`).
- A barra acima da grade mostra "Editável: schema.tabela" ou o motivo de não ser.

## Interação

- Duplo clique (ou botão Editar) abre o editor da célula; Enter confirma, Esc
  cancela, clicar em outra célula também confirma. Digitar o valor original desfaz
  a pendência.
- Botão NULL define NULL (texto vazio ≠ NULL). "Reverter célula" e "Reverter tudo"
  descartam pendências.
- Células alteradas ficam destacadas; a linha recebe ● no número; a barra mostra
  "original → novo" da célula selecionada.
- Enquanto houver pendências, executar o SQL da aba é bloqueado (para não perder
  as alterações em silêncio).
- **Ctrl+S** com pendências: popup com o SQL equivalente (literais) e os botões
  Copiar e fechar / Executar e fechar / Cancelar e reverter. Erro mantém o popup
  aberto e as pendências. Sem pendências, Ctrl+S salva o arquivo SQL.
- **Ctrl+Shift+S**: executa direto, sem popup.

## Execução

- Um `UPDATE ... SET col = $n::text::<tipo> WHERE pk = $m::text::<tipo> RETURNING
  cols` por linha; valores sempre em parâmetros (spec §12). O valor gravado volta
  pelo `RETURNING` e substitui o valor na grade, sem reexecutar a consulta.
- Cada `UPDATE` precisa afetar exatamente 1 linha; senão o lote inteiro é desfeito
  ("linha não encontrada (alterada ou removida por outra sessão?)").
- Com transação ativa: o lote roda sob `SAVEPOINT dbeas_grade`; falha volta ao
  savepoint e a transação do usuário continua válida. As alterações entram no
  resumo do COMMIT ("Alterações pela grade": chave, coluna, antes → depois).
- Sem transação, fora de `requireTransaction`: `BEGIN ... COMMIT` próprio (lote
  atômico). Com `requireTransaction` e sem transação: bloqueado; o popup oferece
  "Iniciar transação".

## Excluir linhas (pedido do usuário em 2026-10-13; não está na spec)

- **Seleção:** clique no número da linha seleciona só ela; Ctrl+clique alterna;
  Shift+clique seleciona o intervalo a partir da última linha clicada sem Shift
  (Ctrl+Shift soma o intervalo). Clicar numa célula limpa a seleção de linhas;
  executar uma consulta também. Regras em `data_grid::RowSelection` (testadas).
  Os modificadores vêm de `ModifiersChanged` (o `mouse_area` não os informa).
- **Foco da grade** (`Tab::grid_focused`, borda colorida em volta da grade):
  clicar numa célula ou num número de linha dá o foco à grade e tira o foco do
  editor SQL explicitamente (`App::focus_grid`; ver
  [estabilidade da árvore](../arquitetura/iced-estabilidade-da-arvore.md) para o
  motivo). Clicar ou digitar no editor devolve o foco a ele (rolar não).
- **Setas com a grade focada** (`data_grid::navigate`, testada): ←/→/↑/↓ movem a
  célula selecionada pelas colunas visíveis, na ordem exibida; com linhas
  selecionadas, ↑/↓ movem a linha e ←/→ voltam para a célula da primeira coluna;
  Shift+↑/↓ estende a seleção de linhas. A grade rola até a célula
  (`data_grid::reveal_cell`).
- Com linhas selecionadas num resultado editável, a barra de edição mostra
  "N linha(s) selecionada(s)" e o botão **Excluir linhas**. A tecla **Del** faz o
  mesmo com a grade focada e só quando nenhum widget capturou a tecla (no editor
  SQL ou no editor de célula, Del continua apagando texto). Com só uma célula
  selecionada, Del não exclui: avisa para selecionar linhas. Por isso Del não está
  em `commands::lookup`: o editor consulta esse mapa antes das próprias teclas.
- **Popup de confirmação:** quantidade de linhas, tabela, chave primária de cada
  linha (até 10, depois "… e mais N"), o SQL equivalente, se a exclusão é
  gravada na hora (irreversível) ou entra na transação ativa, e quantas
  alterações pendentes nessas linhas serão descartadas. Botões Cancelar /
  Excluir N linhas. Erro mantém o popup aberto.
- **Execução:** `data_edit::build_deletes` gera um `DELETE ... WHERE pk =
  $n::text::<tipo> RETURNING <pk>` por linha e reaproveita
  `connection::apply_edits`: cada instrução precisa atingir exatamente 1 linha,
  senão o lote inteiro é desfeito; mesmas regras de transação/savepoint/
  `requireTransaction` das edições (sem transação exigida, o popup oferece
  "Iniciar transação"). Na transação, entra no resumo do COMMIT.
- **Depois:** as linhas saem do resultado em memória, sem reexecutar; pendências
  e a célula selecionada são reindexadas (`data_edit::pending_after_delete`).

## Ainda não implementado

- Inserir linhas (a spec só pede editar células).
- Editor especializado por tipo (valores são digitados na sintaxe de entrada do
  PostgreSQL).
