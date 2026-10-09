# Transformações de SQL pela grade

Spec §5.2–5.5. Código: `src/query/transform.rs` (funções puras, testadas) e
`src/app/mod.rs` (seção "Transformações pela grade").

## Mecanismo único

- `analyze(sql)` usa o `sqlparser` (PostgreSQL) para validar e descobrir a forma:
  uma única instrução `Query`, `SELECT` simples ou operação de conjunto, curingas,
  `DISTINCT`, aliases e ordinais em `ORDER BY`/`GROUP BY`.
- As posições vêm do tokenizer do `sqlparser`. Os tokens cobrem o texto inteiro
  (espaços e comentários inclusive) e o fim de um token é o início do próximo.
  Com a profundidade de parênteses, localiza-se o `SELECT` principal, seus itens
  (vírgulas no nível 0), o `ORDER BY` principal e o ponto de inserção (antes de
  `LIMIT`/`OFFSET`/`FETCH`/`FOR` ou no fim do último token significativo, para não
  cair dentro de um comentário `--`).
- Só os trechos alterados são reescritos. Toda saída é reanalisada pelo parser
  antes de ser aceita.

## Regras

| Operação | Aceita | Recusa (SQL fica intacto) |
| --- | --- | --- |
| Ordenar | qualquer SELECT, inclusive `*`, CTE e UNION; substitui o ORDER BY principal | mais de uma instrução, não-SELECT, SQL que não analisa |
| Ocultar (remover item) | SELECT simples | `*`, UNION, DISTINCT, ordinais, alias usado no ORDER BY, última coluna |
| Reordenar | SELECT simples, sem colunas ocultas só na grade | `*`, UNION, ordinais |

- A expressão de ordenação é o nome da coluna do resultado (`quote_ident`). Se o
  nome se repete no resultado, usa-se a posição do item.
- Ciclo do clique no cabeçalho: sem ordenação → ASC → DESC → sem (remove o
  ORDER BY principal, inclusive um escrito pelo usuário).

## Integração com a aba

- O ator informa em `Outcome.result_statement` qual instrução produziu o
  resultado. A aba guarda `Source { range, text }` no editor. Antes de transformar,
  o texto do editor naquela faixa precisa ser idêntico; caso contrário: "o SQL foi
  alterado desde a execução; execute novamente".
- `ColumnLayout.item_of` relaciona coluna do resultado → item atual do SELECT; é
  atualizado a cada remoção/reordenação sem reexecutar.
- **Ordenar reexecuta** a instrução (decisão do usuário). Ocultar e reordenar
  **não reexecutam**: os dados não mudam, só a grade e o SQL.
- Falha estrutural → a mudança fica apenas visual e a barra de status explica o
  motivo, identificando "consulta gerada pela aplicação" ou "editada pelo usuário"
  (`Tab.generated`).
- "Restaurar colunas" volta ao texto anterior à primeira remoção/reordenação
  (`Tab.pristine`) e reaplica a ordenação; reexecuta se o resultado atual não tiver
  as colunas. Editar o SQL descarta o `pristine` (a restauração passa a ser só
  visual).
- A sessão guarda, por aba, `hidden_columns` (nomes ocultos só na grade) e `sort`;
  os nomes ocultos são reaplicados após a próxima execução.
