# Base de conhecimento do DBeas

Fonte principal de conhecimento e memória do projeto. Consulte aqui antes de
trabalhar em qualquer parte do DBeas e registre aqui tudo o que for aprendido ou
decidido.

## Regras

- Um arquivo por assunto, pequeno e específico. Nome em kebab-case.
- Cada arquivo fica no subdiretório da sua categoria. Crie novas categorias quando
  nenhuma existente servir.
- Ao registrar algo, verifique se já existe um arquivo sobre o assunto e
  atualize-o em vez de duplicar. Remova o que deixar de ser verdade.
- Registre a regra, o motivo e a referência (seção da spec, arquivo, data).
- Mantenha o índice abaixo atualizado.

## Categorias

- `arquitetura/` — módulos, fronteiras, fluxos de dados, decisões técnicas.
- `configuracao/` — `dbeas.json`, configuração global, resolução de variáveis.
- `design/` — aparência, layout e componentes visuais.
- `usabilidade/` — comportamento da interface, interação e atalhos.
- `progresso/` — estado da implementação e próximos passos (continuidade entre sessões).

## Índice

- [Estado da implementação](progresso/estado-implementacao.md) — **comece por aqui ao retomar**: o que está pronto, como testar, pendências.
- [Mapa de módulos](arquitetura/modulos.md) — responsabilidade de cada módulo, fluxo assíncrono, identidade de workspace.
- [Decisões da rodada 1](arquitetura/decisoes-rodada-1.md) — driver, parser, paginação/cursor, readOnly, sessão, editor, grade.
- [Transformações de SQL](arquitetura/transformacoes-sql.md) — ordenar/ocultar/reordenar pela grade: mecanismo, regras de recusa, integração.
- [Regras do dbeas.json](configuracao/dbeas-json.md) — validação, interpolação e padrões como implementados.
- [Atalhos](usabilidade/atalhos.md) — mapa central e conflitos documentados.
- [Transações (requireTransaction)](usabilidade/transacoes.md) — transação manual com botões; grade, SQL manual e arquivos participam; resumo antes do COMMIT.
- [Edição de dados](usabilidade/edicao-dados.md) — quando um resultado é editável, interação, Ctrl+S/Ctrl+Shift+S, execução parametrizada.
- [Quando SQL é executado](usabilidade/execucao-sql.md) — conexão, abertura de tabela, nada de execução automática de SQL do usuário.

- [Identificação do workspace na barra lateral](usabilidade/identificacao-workspace-sidebar.md) — usar `name` da entrada do `dbeas.json`, não o nome do arquivo.
