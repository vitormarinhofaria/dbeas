# Adicionar ou criar workspace

Pedido do usuário em 2026-10-13. Não está na spec. Lógica em
`src/workspace/add.rs` (`resolve`/`create`, testada); fluxo em
`App::add_workspace` (`src/app/mod.rs`); tela no `Modal::AddWorkspace`
(`src/app/view.rs`).

## Regra

O diálogo "Adicionar workspace" aceita um arquivo ou um diretório (botões
"Arquivo…" e "Diretório…"). `resolve` é refeito a cada alteração do caminho e
decide o botão:

- Arquivo existente, ou diretório com `dbeas.json` → **Adicionar** (registra).
- Diretório sem `dbeas.json` → **Criar** `<dir>/dbeas.json`.
- Caminho inexistente → **Criar**: termina em `.json` → esse arquivo; senão é
  tratado como diretório novo com `dbeas.json` dentro. Diretórios ausentes são
  criados. Para criar, o caminho precisa ser absoluto (`~/` é expandido).

Criar pede confirmação na própria janela ("Deseja criar dbeas.json em
{diretório}?", botões Voltar/Criar; Enter também confirma). Editar o caminho
cancela a confirmação.

O arquivo criado tem uma única entrada `{"name": "<nome do diretório>"}`,
gravada com `create_new` (nunca sobrescreve). Depois de registrar e selecionar
o workspace, o [editor gráfico](../configuracao/editor-grafico.md) abre nessa
entrada, com as mesmas abas e o botão "+ nova entrada" para já acrescentar
outras configurações ao arquivo criado. Até `dbName` ser preenchido, o workspace aparece com erro de
configuração na barra lateral (`dbName` é obrigatório).
