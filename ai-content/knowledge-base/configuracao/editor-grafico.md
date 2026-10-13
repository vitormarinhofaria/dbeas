# Editor gráfico do `dbeas.json`

Pedido do usuário em 2026-10-13. Ainda não está descrito na spec.
Lógica em `src/workspace/editor.rs` (`Form`, testada); tela em
`settings_view` (`src/app/view.rs`); fluxo em `App::save_workspace_settings`.

## Interação

- Ícone ⚙ ao lado do "−" (remover) em cada workspace da barra lateral abre o
  editor da entrada correspondente (`Descriptor::entry` = posição no array).
  Arquivo ilegível ou JSON inválido: não abre; avisa para corrigir num editor
  de texto.
- Todos os campos são texto, **inclusive `dbPort`**. `sslMode` é um dropdown
  (padrão + os 5 modos); se o valor atual não for um deles (ex.:
  `${env.SSL_MODE}`), ele entra como opção extra para não ser perdido.
  `readOnly`/`requireTransaction` são checkboxes; `env` é uma lista de pares
  nome/valor.
- Campos com `${env.NOME}` mostram a prévia do valor resolvido abaixo do campo
  (processo < `envFile` do formulário < `env` do formulário, como na conexão).
  **`dbPass` nunca mostra o valor:** `********` se resolver para algo não vazio,
  nada se resolver para vazio. Senha literal fica oculta no campo (botão
  Mostrar); referência `${env.X}` fica legível.
- Validação ao vivo com o mesmo `config::resolve_entry` da carga. Bloqueiam
  gravar: `name`/`dbName` vazios, porta literal inválida, nome de variável
  inválido ou repetido em `env`. Os demais erros (variável ausente, `envFile`
  inexistente) só avisam: podem depender da máquina.
- Clique fora do modal não fecha; Esc não fecha se houver alterações.

## Gravação

- Só o que mudou é reescrito: campo intocado mantém valor e tipo JSON originais;
  chaves desconhecidas ficam na mesma posição (`serde_json` com
  `preserve_order`). Campo de texto esvaziado remove a chave; `false` só é
  escrito se a chave já existia. Porta nova numérica vira número JSON, salvo se
  o original era string.
- `env_file` (nome antigo) é regravado como `envFile` no mesmo lugar.
- Recusa gravar se o arquivo mudou no disco desde a abertura (compara o texto).
- Mantém a indentação do arquivo; `fsutil::atomic_write` preserva as permissões
  do arquivo substituído (um `dbeas.json` com senha pode ser `0600`).
- Depois de gravar, recarrega os workspaces. Renomear (`name` faz parte do
  `WorkspaceId`) mantém abas, conexão e sessão: `reload_workspaces_keeping`
  casa pela posição no arquivo e migra as chaves da sessão.
- Conexão aberta **não** é refeita (pode haver transação): o aviso pede para
  desconectar e conectar. Mudança de `sqlFilesDir` reinicia o explorador de
  arquivos.

## Verificação

Verificado no app em 2026-10-13 com captura interna (ver
[estabilidade da árvore](../arquitetura/iced-estabilidade-da-arvore.md)):
prévias, asteriscos, erro bloqueante, renomear mantendo o workspace ativo.
