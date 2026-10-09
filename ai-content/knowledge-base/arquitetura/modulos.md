# Mapa de módulos

Segue a §11 da spec. Crate com `lib.rs` (tudo testável) e `main.rs` mínimo.

| Módulo | Responsabilidade |
| --- | --- |
| `app/mod.rs` | `App`, `Message`, `update`, subscriptions, ciclo de vida (fechar salva a sessão). |
| `app/state.rs` | `Workspace` e `Tab` em execução; conversão para/desde `session`. |
| `app/view.rs` | Layout: sidebar, cabeçalho, abas, resultados/editor, status, modais. |
| `app/quick_open.rs` | Itens e pontuação fuzzy do `Ctrl+P`. |
| `workspace/` | `config.rs` (parse/validação/resolução), `env.rs` (`${env.X}`), `mod.rs` (carregar registrados, ids, nomes). |
| `settings.rs` | Registro global de arquivos `dbeas.json`. |
| `session.rs` | Formato serializável da sessão (versão 1). |
| `connection.rs` | Ator por workspace: execução, cursor de paginação, catálogo, cancelamento. |
| `metadata.rs` | Consultas ao `pg_catalog`, `Catalog`, `quote_ident`, SELECT inicial de tabela. |
| `query/` | Tipos de resultado e erro, `value.rs` (decodificação binária), `split.rs`, `classify.rs` (readOnly). |
| `data_grid.rs` | Grade virtualizada. |
| `sql_editor.rs` | Editor com highlight e gutter. |
| `file_explorer.rs` | Listagem restrita ao `sqlFilesDir`, leitura/gravação, varredura para o `Ctrl+P`. |
| `commands.rs` | Mapa central de atalhos. |
| `fsutil.rs` | Gravação atômica e normalização de caminhos. |

## Fluxo assíncrono

Toda I/O passa por `Task::perform` e volta como `Message`. Respostas carregam
`WorkspaceId` + id da aba + `exec_id`; respostas de execuções antigas são
ignoradas. Exceções síncronas deliberadas: gravar `settings` (arquivo pequeno) e
gravar a sessão ao fechar a janela (precisa terminar antes de sair).

## Identidade

`WorkspaceId` = `"<caminho absoluto do dbeas.json>#<name>"` (com `#n` se o nome se
repetir no mesmo arquivo). É a chave da sessão.
