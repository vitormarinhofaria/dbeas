# Decisões técnicas da rodada 1

Decisões pendentes da §15 tomadas em 2026-10-09 (com o usuário quando indicado).

- **Driver:** `tokio-postgres` 0.7, sem TLS (`NoTls`). TLS/`sslMode` fica para depois
  (decisão do usuário).
- **Parser SQL:** `sqlparser` 0.63, dialeto PostgreSQL (decisão do usuário). Usado
  hoje só na classificação do `readOnly`; será a base das transformações (§5.5).
- **Divisão de scripts:** lexer próprio (`query/split.rs`), que respeita strings,
  `E''`, identificadores entre aspas, dollar quoting e comentários aninhados. O
  protocolo estendido não aceita várias instruções num envio.
- **Paginação:** `query_raw` + `RowStream`, 500 linhas por página, no máximo 20 000
  em memória por resultado. **Só um cursor aberto por conexão:** qualquer outro
  pedido descarta o cursor anterior, porque um `RowStream` parado bloqueia a conexão
  (o tokio-postgres aplica contrapressão). As linhas já carregadas continuam visíveis,
  e "carregar mais" informa que é preciso reexecutar.
- **Valores:** resultados chegam em formato binário e são decodificados por
  `query/value.rs`; tipos desconhecidos ficam em `Value::Other { type_name, raw }`.
  `timestamptz` é exibido em UTC (`+00`).
- **readOnly:** (1) `classify::check_read_only` no ator, antes de executar: só SELECT
  (sem `INTO`/`FOR UPDATE`/CTE com DML), VALUES, EXPLAIN sem ANALYZE e SHOW; SQL que
  não pode ser analisado é bloqueado; (2) a sessão é aberta com
  `-c default_transaction_read_only=on`.
- **Sessão:** `$XDG_STATE_HOME/dbeasy/session.json` (`~/.local/state/...`),
  autosave a cada 3 s quando há mudança, e gravação ao fechar. Arquivos SQL sem
  alterações são relidos do disco; com alterações, o conteúdo vai para a sessão,
  por isso fechar o app não pede confirmação.
- **Editor:** `text_editor` do Iced com highlighter syntect (token `sql`). O gutter é
  uma coluna de texto com a mesma altura de linha, dentro do mesmo `scrollable`.
  Consequência: sem quebra de linha e sem rolagem horizontal no editor.
- **Grade:** virtualização manual (espaçadores acima e abaixo das linhas visíveis).
  O cabeçalho é um `scrollable` horizontal sincronizado via `operation::scroll_to`.
- **Gravação de arquivo SQL:** `tokio::fs::write` no próprio arquivo (preserva dono e
  permissões); a gravação atômica é usada só para config e sessão.
