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

## Ainda não implementado

- Inserir e excluir linhas (a spec só pede editar células).
- Editor especializado por tipo (valores são digitados na sintaxe de entrada do
  PostgreSQL).
