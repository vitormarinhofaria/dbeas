# Estado da implementação

Arquivo de continuidade: leia antes de retomar o trabalho com um contexto novo.
Atualize ao fim de cada etapa.

**Última atualização:** 2026-10-09 (rodada 4 — autocomplete e diagnóstico)

## Como rodar e testar

- `cargo run` — abre o app usando `~/.config/dbeas/config` e
  `~/.local/state/dbeas/session.json`.
- Para não tocar na configuração real, isole com
  `XDG_CONFIG_HOME=/tmp/x/config XDG_STATE_HOME=/tmp/x/state cargo run`.
- `cargo test` — testes unitários (77) + 7 de integração com PG. O build do
  `pg_query` precisa de `clang`.
- Testes com PostgreSQL real (`tests/postgres.rs`), ignorados sem a variável:
  ```
  docker run -d --name dbeas-test-pg -e POSTGRES_PASSWORD=dbeas -e POSTGRES_USER=dbeas \
    -e POSTGRES_DB=dbeas_dev -p 127.0.0.1:55432:5432 postgres:17
  DBEAS_TEST_PG=127.0.0.1:55432:dbeas:dbeas:dbeas_dev cargo test
  ```
- Não há ferramenta de injeção de teclado/mouse no ambiente (Wayland/KDE, sem
  ydotool). A verificação visual foi feita com `spectacle -b -n -a -o arquivo.png`.

## Feito (rodada 1)

| Área | Situação | Onde |
| --- | --- | --- |
| `dbeas.json`: parse, validação, `${env.X}`, precedência, padrões, erros sem segredos | pronto, testado | `src/workspace/{config,env}.rs` |
| Registro global (`~/.config/dbeas/config`), dedup, remoção, gravação atômica | pronto, testado | `src/settings.rs`, `src/fsutil.rs` |
| Nomes duplicados → sufixo com repo Git/diretório pai | pronto, testado | `src/workspace/mod.rs` |
| Sessão por workspace (abas, ordem, ativa, SQL, explorador, split, largura da sidebar) | pronto, testado, verificado no app | `src/session.rs`, `src/app/state.rs` |
| Conexão (ator tokio), estados, cancelamento, readOnly em 2 camadas | pronto, testado com PG | `src/connection.rs`, `src/query/classify.rs` |
| Valores tipados (numeric exato, datas, arrays, NULL ≠ vazio, `Other` com bytes) | pronto, testado | `src/query/value.rs` |
| Divisão de scripts / instrução sob o cursor / posição de erro → linha:coluna | pronto, testado | `src/query/split.rs` |
| Catálogo (schemas, tabelas, views, mat. views, colunas, PK, funções, sequências) | pronto, testado com PG | `src/metadata.rs` |
| UI: sidebar de workspaces com estado, exploradores Tabelas/Objetos/Arquivos | pronto, verificado visualmente | `src/app/view.rs` |
| Abas por workspace, Scratch fixo, fechar com confirmação se arquivo sujo | pronto | `src/app/mod.rs` |
| Editor: highlight (syntect), números de linha, linha do erro marcada, atalhos | pronto | `src/sql_editor.rs` |
| Grade virtualizada, paginação (500/página, teto de 20 000 linhas) | pronto, verificado visualmente | `src/data_grid.rs` |
| Divisores arrastáveis (sidebar e resultados/editor) | implementado, não verificado por arraste | `src/app/mod.rs` |
| Arquivos SQL: navegar (restrito ao `sqlFilesDir`), abrir, editar, salvar, executar | implementado, sem teste de clique | `src/file_explorer.rs` |
| `Ctrl+P` seletor rápido, `Ctrl+Tab` workspaces, mapa central de atalhos | implementado, sem teste de teclado | `src/commands.rs`, `src/app/quick_open.rs` |

## Feito (rodada 2)

| Área | Situação | Onde |
| --- | --- | --- |
| Análise estrutural de SELECT (parser + tokenizer com posições) | pronto, testado | `src/query/transform.rs` |
| Ordenar pelo cabeçalho (ASC → DESC → sem), reescreve o ORDER BY e reexecuta | pronto, verificado no app e no PG | `transform::set_order`, `App::toggle_sort` |
| Ocultar coluna (botão do meio) removendo o item do SELECT, sem reexecutar | pronto, verificado no app e no PG | `transform::remove_item`, `App::hide_column` |
| Reordenar arrastando cabeçalhos (soltar sobre outro cabeçalho) | implementado, testado no PG; arraste não testado na UI | `transform::reorder_items`, `App::move_column` |
| Restaurar colunas, recusa segura com explicação, origem gerada/editada | implementado | `App::restore_columns` |
| Sessão guarda colunas ocultas e ordenação por aba | pronto, testado | `src/session.rs` |

## Feito (rodada 3 — transações e edição de dados)

- [x] `classify::transaction_control` (BEGIN/COMMIT/ROLLBACK/... por análise léxica).
- [x] Catálogo com `oid` da tabela e `attnum` das colunas.
- [x] `src/data_edit.rs`: tabela editável (uma tabela, com PK, todas as colunas da PK
  no resultado), `UPDATE ... RETURNING` parametrizado (`$n::text::tipo`) e texto
  equivalente com literais para o popup.
- [x] Ator: `TxState` (Idle/Active/Failed) publicado num `watch`, política do
  `requireTransaction` por instrução, `transaction(Begin/Commit/Rollback)`,
  `summary()` (`pg_stat_xact_user_tables` + registro), `apply_edits` (savepoint na
  transação ativa; bloco próprio fora de `requireTransaction`). Testado com PG.
- [x] UI de transação: indicador e botões no cabeçalho (Iniciar / Cancelar /
  Confirmar…; "transação com erro" só oferece Cancelar), resumo antes do COMMIT
  (verificado no app), confirmação para executar arquivo salvo, bloqueio ao
  fechar/desconectar (implementado, sem teste de clique).
- [x] UI de edição: duplo clique edita, NULL, destaque de pendências, reverter
  célula/tudo, Ctrl+S com popup (verificado no app), Ctrl+Shift+S. Ver
  [edição de dados](../usabilidade/edicao-dados.md).

## Feito (rodada 4 — editor SQL: autocomplete e diagnóstico)

Decisão do usuário (2026-10-09): autocomplete embutido a partir do catálogo +
diagnóstico de sintaxe com libpg_query; LSP externo fica como camada opcional
futura. Autocomplete automático (após `.` e 2+ letras) e por Ctrl+Espaço.

- [x] `src/diagnostics.rs`: `pg_query_parse` via FFI (o wrapper do crate descarta
  `cursorpos`), posição por instrução → linha/coluna no documento. Testado.
- [x] `src/completion.rs`: contexto léxico (alias., schema., FROM/JOIN, colunas das
  tabelas citadas, palavras-chave). Testado.
- [x] Integração no editor: popup junto ao cursor (verificado no app), teclas,
  Ctrl+Espaço, diagnóstico 400 ms após editar com marcação no gutter e rodapé
  (verificado no app). Ver [editor inteligente](../arquitetura/editor-inteligente.md).
- [x] Sublinhado vermelho no trecho errado (diagnóstico e erro de execução com
  posição), alinhado pelo próprio layout de texto. Verificado no app.

## Pendente (próximas rodadas)

1. ~~Transformações de SQL~~ — feito na rodada 2 (ver
   [transformações](../arquitetura/transformacoes-sql.md)).
2. ~~Edição de dados~~ — feito na rodada 3 (ver [edição](../usabilidade/edicao-dados.md)).
   Falta: inserir/excluir linhas.
3. ~~requireTransaction~~ — feito na rodada 3 (ver [transações](../usabilidade/transacoes.md)).
4. ~~Autocomplete / diagnóstico~~ — feito na rodada 4 (embutido). Pendente opcional:
   integração com servidor LSP externo.
5. TLS (`sslMode`) — decidido adiar; conexão usa `NoTls`.
5a. ~~Criar arquivo SQL pelo explorador~~ — feito em 2026-10-09 (spec §8). Ver
   [arquivos SQL](../usabilidade/arquivos-sql.md).
5b. ~~Menu de contexto (renomear/mover e excluir) no explorador~~ — feito em
   2026-10-09; renomear também move dentro do `sqlFilesDir` (sem `..`); excluir
   move para a lixeira. Ainda não está descrito na spec.
6. Melhorias conhecidas: o editor não rola horizontalmente (sem quebra de linha,
   linhas longas ficam cortadas); `timestamptz` é exibido em UTC; a altura usada no
   arraste do divisor é aproximada (`CHROME_HEIGHT`).
7. Grade: soltar o arraste de cabeçalho fora da grade deixa o cabeçalho destacado
   até o próximo clique; reordenações só visuais se perdem ao reexecutar.
8. Transações: o estado é rastreado pelas instruções executadas (o tokio-postgres
   não expõe o status do ReadyForQuery). COMMIT dentro de função/procedure ou
   `DO` não é detectado. Resultado paginado e transação compartilham a conexão: um
   novo comando fecha o cursor do resultado anterior.

## Ver também

- [Módulos](../arquitetura/modulos.md)
- [Decisões da rodada 1](../arquitetura/decisoes-rodada-1.md)
- [Transformações de SQL](../arquitetura/transformacoes-sql.md)
- [Regras do dbeas.json](../configuracao/dbeas-json.md)
- [Atalhos](../usabilidade/atalhos.md)
- [Execução de SQL](../usabilidade/execucao-sql.md)
- [Transações](../usabilidade/transacoes.md)
- [Edição de dados](../usabilidade/edicao-dados.md)
- [Arquivos SQL](../usabilidade/arquivos-sql.md)
- [Editor inteligente](../arquitetura/editor-inteligente.md)
