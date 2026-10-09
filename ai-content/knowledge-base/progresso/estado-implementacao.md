# Estado da implementação

Arquivo de continuidade: leia antes de retomar o trabalho com um contexto novo.
Atualize ao fim de cada etapa.

**Última atualização:** 2026-10-09 (rodada 1 — fundação + UI básica)

## Como rodar e testar

- `cargo run` — abre o app usando `~/.config/dbeas/config` e
  `~/.local/state/dbeas/session.json`.
- Para não tocar na configuração real, isole com
  `XDG_CONFIG_HOME=/tmp/x/config XDG_STATE_HOME=/tmp/x/state cargo run`.
- `cargo test` — testes unitários (44).
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

## Pendente (próximas rodadas)

1. **Transformações de SQL** (§5.2–5.5) com `sqlparser`: ordenar por cabeçalho
   (reescreve e executa), ocultar coluna com o botão do meio, reordenar por arraste,
   restaurar colunas. Ainda não há clique no cabeçalho.
2. **Edição de dados** (§6): mapear o resultado para uma tabela (`ColumnInfo.table_oid`
   / `column_id` + PK do catálogo), alterações pendentes, `Ctrl+S` com popup de SQL,
   `Ctrl+Shift+S` direto, bloqueio em `readOnly`.
3. **requireTransaction** (§6.4): COMMIT/ROLLBACK explícitos, indicador de transação
   aberta, fluxo ao fechar/desconectar. **Decisão pendente com o usuário:** política
   para SQL arbitrário (scripts, comandos de transação do usuário). Hoje o SQL do
   editor roda em autocommit.
4. **LSP / autocomplete / diagnósticos antes da execução** (§7.2): hoje a barra de
   status informa "indisponível". O catálogo já tem tudo para autocomplete local.
5. TLS (`sslMode`) — decidido adiar; conexão usa `NoTls`.
6. Melhorias conhecidas: o editor não rola horizontalmente (sem quebra de linha,
   linhas longas ficam cortadas); `timestamptz` é exibido em UTC; a altura usada no
   arraste do divisor é aproximada (`CHROME_HEIGHT`).

## Ver também

- [Módulos](../arquitetura/modulos.md)
- [Decisões da rodada 1](../arquitetura/decisoes-rodada-1.md)
- [Regras do dbeas.json](../configuracao/dbeas-json.md)
- [Atalhos](../usabilidade/atalhos.md)
- [Execução de SQL](../usabilidade/execucao-sql.md)
