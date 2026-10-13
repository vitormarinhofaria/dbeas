# Quando SQL é executado

Decisões do usuário em 2026-10-09 que interpretam a §12 ("não executar SQL
automaticamente") junto da §4.3.

- **Conexão:** acontece ao selecionar o workspace pela primeira vez, seguida da
  leitura do catálogo (só leitura). Ao iniciar o app, conecta apenas o workspace
  ativo restaurado. Erro de conexão deixa o estado "erro"; o botão Conectar tenta
  de novo.
- **Abrir tabela/view** (explorador ou Ctrl+P): gera `SELECT <colunas> FROM
  schema.tabela` e **executa na hora**, porque é SQL gerado pela aplicação e só de
  leitura. Se a aba já existe, é reutilizada.
- **Nunca executa sozinho:** SQL do usuário e arquivos SQL abertos.
- **Abas restauradas da sessão** (decisão de 2026-10-13): voltam sem resultado e
  com indicador de "em branco" (título esmaecido, `◌` e dica ao passar o mouse).
  A sessão guarda a instrução do último resultado (`last_query`: deslocamento +
  texto). Ao **clicar** na aba, ela é reexecutada só se a aba for de **tabela** e
  a instrução for um único `SELECT` somente leitura (`classify::is_select`: nada
  de UPDATE/INSERT, CTE com escrita, `FOR UPDATE`, EXPLAIN ou várias instruções).
  Scratch e arquivos nunca reexecutam; continuam com o indicador até o usuário
  executar. A aba ativa na abertura não executa sozinha: só ao clicar. Clicada
  antes da conexão terminar, executa quando ela chega. Sessões antigas sem
  `last_query` usam a única instrução do SQL da aba de tabela. Código:
  `Tab::restorable_query` (`src/app/state.rs`), `App::restore_blank_tab`.
- **Ordenação por cabeçalho:** reescreve o ORDER BY e reexecuta a instrução que
  gerou o resultado. Ocultar e reordenar colunas alteram o SQL sem reexecutar. Ver
  [transformações](../arquitetura/transformacoes-sql.md).
- **Erros:** o último resultado válido continua visível; a mensagem mostra
  linha:coluna, o cursor vai para a posição e a linha fica marcada no gutter.
