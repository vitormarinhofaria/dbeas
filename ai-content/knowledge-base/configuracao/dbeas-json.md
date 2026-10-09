# Regras do `dbeas.json` implementadas

Referência: spec §3; código em `src/workspace/config.rs`.

- `name` e `dbName` são obrigatórios (decisão do usuário em 2026-10-09; a tabela da
  §3.2 foi corrigida). Sem eles, a entrada aparece com erro e as demais seguem
  funcionando.
- Erro no arquivo inteiro (leitura, JSON, não ser array) gera um único item com erro
  na sidebar, rotulado pelo diretório do arquivo.
- `dbHost`, `dbPort`, `dbUser`, `dbPass` e `dbName` aceitam `${env.NOME}`. Os
  valores dentro de `env` **não** são interpolados (são literais).
- `env_file` informado mas ausente ou ilegível é erro de configuração.
- `dbUser` ausente ou vazio: usa `$USER`, como o libpq. Referência não resolvida é
  erro.
- `dbPort` pode ser número ou string; vazio vira 5432; fora de 1–65535 é erro.
- `readOnly`/`requireTransaction` precisam ser booleanos JSON (`"true"` é erro).
- Caminhos relativos (`env_file`, `sqlFilesDir`) são resolvidos a partir do
  diretório do `dbeas.json`, com normalização léxica.
- Mensagens de erro nunca incluem valores resolvidos; `Secret` oculta a senha em
  `Debug`. Erros de JSON/`.env` não repetem o conteúdo da linha.
