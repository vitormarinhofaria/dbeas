# Transações em workspaces com `requireTransaction`

**Status:** implementado na rodada 3 (2026-10-09).
**Referência:** spec §6.4. Decisão do usuário em 2026-10-09.

## Regra

- A transação é **iniciada e confirmada manualmente** pelo usuário. A aplicação
  nunca faz `BEGIN` nem `COMMIT` por conta própria.
- Em workspaces com `requireTransaction: true`, a interface mostra controles de
  transação:
  - **Iniciar transação**, quando nenhuma está ativa;
  - **Confirmar** (`COMMIT`) e **Cancelar** (`ROLLBACK`), quando há uma ativa.
- Enquanto a transação está ativa, fazem parte dela:
  - as alterações feitas pela grade;
  - as consultas executadas manualmente no editor (Scratch e abas de tabela);
  - a execução de arquivos SQL.

## Consequências para a implementação

- A transação pertence à conexão do workspace (o ator em `connection.rs`), e não a
  uma aba: todas as abas do workspace compartilham a mesma transação ativa.
- O estado "transação ativa" precisa ficar visível no cabeçalho do workspace (spec
  §6.4: indicar transação aberta e alterações não confirmadas).
- Trocar de aba ou de workspace não confirma nada (spec §6.4).
- O cursor de paginação de resultados grandes convive com a transação: hoje
  qualquer pedido novo fecha o cursor anterior (ver
  [decisões da rodada 1](../arquitetura/decisoes-rodada-1.md)).

## Escrita sem transação ativa (decidido em 2026-10-09)

- **SQL manual e alterações pela grade:** exigem que o usuário tenha iniciado a
  transação explicitamente pela interface. Sem transação ativa, a execução não
  acontece e a interface pede para iniciar uma.
- **Arquivos SQL sem alterações não salvas:** antes de executar, informar que o
  arquivo será executado dentro de uma transação e pedir confirmação. Se o usuário
  aceitar, a aplicação inicia a transação e executa (o COMMIT continua manual).
- Arquivos **com** alterações não salvas: tratados como SQL manual (exigem a
  transação iniciada antes).
- **Consultas só de leitura (SELECT)** podem rodar sem transação ativa. A
  classificação usa o mesmo analisador do `readOnly` (`query/classify.rs`); SQL que
  não pode ser classificado como leitura é tratado como escrita.

## Resumo antes do COMMIT (decidido em 2026-10-09)

O botão Confirmar abre uma **etapa de confirmação**: exibe o resumo e só então o
usuário escolhe **Confirmar (COMMIT)** ou **Reverter (ROLLBACK)**. O resumo é
montado a partir de três fontes:

1. **Contagem por tabela vinda do servidor:** `pg_stat_xact_user_tables`
   (`n_tup_ins`, `n_tup_upd`, `n_tup_del` da transação corrente). Inclui efeitos
   indiretos (triggers, funções).
2. **Registro da aplicação:** cada instrução executada na transação, com o retorno
   do servidor e as linhas afetadas. É a única fonte para DDL e `TRUNCATE`, que a
   contagem por tabela não mostra.
3. **Edições pela grade:** linha/coluna com valor original e novo.

Limitações conhecidas: não há diff linha a linha para SQL livre; a contagem por
tabela cobre só tabelas de usuário; sequências consumidas não voltam com ROLLBACK.

## Demais regras (decididas em 2026-10-09)

- **`BEGIN`/`COMMIT`/`ROLLBACK`/`SAVEPOINT` etc.:** bloqueados no SQL manual (use os
  botões). **Permitidos em arquivos salvos.** Nesse caso, a aplicação precisa
  ressincronizar o indicador depois da execução, porque o arquivo pode ter
  confirmado ou aberto uma transação. Atenção na implementação: o tokio-postgres
  não expõe o status do `ReadyForQuery`; avaliar como detectar o estado
  (classificar as instruções executadas e/ou consultar o servidor).
- **Fechar a aplicação ou desconectar com transação ativa:** o fechamento fica
  bloqueado e a aplicação pergunta se o usuário quer confirmar (passando pelo
  resumo) ou abortar a transação (ROLLBACK). Fechar a caixa de diálogo cancela o
  fechamento.
- **Erro dentro da transação:** destacar o estado "transação com erro" (o
  PostgreSQL rejeita os comandos seguintes até o ROLLBACK) e oferecer apenas
  **Cancelar (ROLLBACK)**.

## Implementação

- Estado (`TxState`: Idle/Active/Failed) mantido pelo ator em `connection.rs` e
  publicado num `tokio::sync::watch`; o app o espelha após cada resposta
  (`Workspace::sync_tx`).
- O estado é derivado das instruções executadas: `classify::transaction_control`
  reconhece BEGIN/START, COMMIT/END, ROLLBACK/ABORT, ROLLBACK TO, SAVEPOINT,
  PREPARE TRANSACTION. Erro com transação ativa → Failed.
- A política é avaliada por instrução no ator (`check_policy`), com o estado do
  momento, pois um arquivo pode abrir ou fechar a transação no meio do script.
- Resumo: `pg_stat_xact_user_tables` + registro de escritas do ator + registro de
  alterações da grade mantido no app (`Workspace::grid_log`).
- Limitação: COMMIT/ROLLBACK executados dentro de procedures ou blocos `DO` não são
  detectados.
