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
- **Nunca executa sozinho:** SQL do usuário, arquivos SQL abertos e abas
  restauradas da sessão (a aba de tabela volta com o SQL, sem resultado).
- **Ordenação por cabeçalho:** reescreve o ORDER BY e reexecuta a instrução que
  gerou o resultado. Ocultar e reordenar colunas alteram o SQL sem reexecutar. Ver
  [transformações](../arquitetura/transformacoes-sql.md).
- **Erros:** o último resultado válido continua visível; a mensagem mostra
  linha:coluna, o cursor vai para a posição e a linha fica marcada no gutter.
