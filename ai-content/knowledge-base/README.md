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

## Índice

- [Identificação do workspace na barra lateral](usabilidade/identificacao-workspace-sidebar.md) — usar `name` da entrada do `dbeas.json`, não o nome do arquivo.
