# Autocomplete e diagnóstico do editor

Spec §7.1–7.2. Decisão do usuário (2026-10-09): implementação **embutida**;
servidor LSP externo fica como camada opcional futura.

## Diagnóstico de sintaxe — `src/diagnostics.rs`

- Usa a libpg_query (parser real do PostgreSQL), ligada estaticamente pelo crate
  `pg_query` 6. O wrapper `pg_query::parse` descarta a posição do erro; por isso há
  uma FFI mínima para `pg_query_parse`/`pg_query_free_parse_result` lendo
  `PgQueryError.cursorpos` (1-based, em caracteres). Se o crate mudar a estrutura
  `PgQueryError`, essa declaração precisa acompanhar (`libpg_query/pg_query.h`).
- O texto é dividido com `query::split::statements` e cada instrução é analisada;
  a posição vira linha/coluna no documento.
- Disparo: 400 ms depois da última edição (`schedule_diagnostics` + `text_rev`:
  respostas de revisões antigas são descartadas), ao trocar de aba/workspace e ao
  abrir arquivo. Roda em `spawn_blocking`.
- Exibição: linhas com erro em vermelho no gutter e rodapé "Sintaxe — linha L,
  coluna C: mensagem (+n)".
- Limitação: só sintaxe. Tabelas/colunas inexistentes aparecem apenas ao executar
  (erro do servidor, com linha/coluna).
- Exige `clang`/`libclang` para compilar (build do `pg_query`).

## Autocomplete — `src/completion.rs`

- Fonte: o `Catalog` do workspace ativo (nunca mistura workspaces). Sem catálogo,
  só palavras-chave.
- Contexto léxico, tolerante a SQL incompleto, restrito à instrução do cursor:
  `alias.`/`tabela.` → colunas; `schema.` → tabelas do schema; após
  FROM/JOIN/UPDATE/INTO/TABLE (e vírgula na lista do FROM) → tabelas, views e
  schemas; nos demais lugares → colunas das tabelas citadas + palavras-chave (caixa
  segue o prefixo). Nada dentro de strings/comentários.
- Ordenação: começa com o prefixo antes de contém; colunas antes de palavras-chave;
  máximo de 50 itens.
- Disparo: ao digitar letra/dígito/`_`/`.` (sem prefixo só após `.`), e
  Ctrl+Espaço. Mover o cursor ou digitar algo que não é nome fecha a lista.
- **Lista ativa × dica** (pedido do usuário, 2026-10-09): aberta automaticamente,
  a lista é só uma dica — sem item destacado, Enter quebra linha e Tab indenta
  normalmente. Ela fica **ativa** quando aberta por Ctrl+Espaço ou quando o usuário
  usa ↑/↓ (↓ vai ao primeiro item, ↑ ao último); só então Enter/Tab aceitam. Uma
  vez ativa, continua ativa enquanto se digita. Esc fecha nos dois estados.
  Estado em `CompletionState.active`.
- Aceitar seleciona o prefixo com `Select(Left)` e cola o texto (`Edit::Paste`),
  usando as ações do próprio editor. Identificadores são citados com `quote_ident`.
- A lista é desenhada com `stack` + `pin` sobre o editor, na posição calculada
  pela grade monoespaçada (`CHAR_WIDTH` ≈ 8,4 px em 14 px). Fontes muito diferentes
  podem deslocar um pouco o popup.

## Futuro: LSP opcional

Possível integração com `postgres-language-server` via stdio, como camada extra;
o editor precisa continuar funcionando sem ele (spec §7.2).
