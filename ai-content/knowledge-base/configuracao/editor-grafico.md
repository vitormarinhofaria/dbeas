# Editor gráfico do `dbeasy.json`

Pedido do usuário em 2026-10-13. Ainda não está descrito na spec.
Lógica em `src/workspace/editor.rs` (`Document` com um `Form` por entrada,
testada); tela em `settings_view`/`settings_tabs` (`src/app/view.rs`); fluxo em
`App::save_workspace_settings`.

## Interação

- Ícone ⚙ ao lado do "−" (remover) em cada workspace da barra lateral abre o
  editor do **arquivo inteiro**, com uma aba por entrada do array, já na aba da
  entrada clicada (`Descriptor::entry` = posição no array). Itens do array que
  não são objetos não viram abas, mas são preservados. Arquivo ilegível ou JSON
  inválido: não abre; avisa para corrigir num editor de texto.
- Abas (pedido do usuário em 2026-10-13): rótulo = `name` digitado (ou
  "entrada N"/"nova entrada"); `●` = alterada ou nova; `⚠` = erro que impede
  gravar. "+ nova entrada" cria uma aba vazia, acrescentada ao **fim** do array
  ao salvar; só abas novas têm "×" (descartar). Remover entradas existentes não
  é feito pelo editor. Trocar de aba volta a ocultar a senha.
- Salvar grava todas as abas de uma vez; fica desabilitado se alguma aba tiver
  erro bloqueante (a mensagem diz qual aba). Arquivo `[]` abre com uma aba nova.
- Rótulos (pedido do usuário em 2026-10-13): nome descritivo em português
  (`Field::title`, ex.: "Nome do banco") e, logo abaixo, a chave do json
  (`dbName`) menor, monoespaçada e mais apagada (`faint`, alfa 0.4). Montado por
  `field_label`/`labeled` em `src/app/view.rs`; vale também para `env`,
  `readOnly` e `requireTransaction`.
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
- `sqlFilesDir` tem o botão "Escolher…" (pedido do usuário em 2026-10-13):
  seletor de diretório (`rfd::pick_folder`) que abre no diretório do
  `dbeasy.json` editado. O escolhido vira `./sub` (ou `.`) se estiver abaixo do
  diretório do json, senão caminho absoluto (`Document::dir_value`, compara
  caminhos canonicalizados). O resultado vai para a aba que estava ativa ao
  abrir o seletor. O campo continua editável como texto.
- Validação ao vivo com o mesmo `config::resolve_entry` da carga. Bloqueiam
  gravar: `name`/`dbName` vazios, porta literal inválida, nome de variável
  inválido ou repetido em `env`. Os demais erros (variável ausente, `envFile`
  inexistente) só avisam: podem depender da máquina.
- Clique fora do modal não fecha; Esc não fecha se houver alterações.
- Teclado (pedido do usuário em 2026-10-13): ao abrir, o foco vai para o
  primeiro campo (tirando-o do editor SQL sob o modal, que receberia Tab/Enter).
  Tab/Shift+Tab percorrem só os campos de texto do diálogo, circularmente, e
  rolam a área até o campo focado (`src/app/focus.rs`, escopo = contêiner
  `SETTINGS_FIELDS`; o `focus_next` do Iced não serve: percorre a árvore
  inteira, não volta ao início e não rola). Enter salva quando o botão Salvar
  está habilitado (`Document::can_save`), com qualquer campo focado ou sem foco.

## Testar conexão

Pedido do usuário em 2026-10-13. Botão "Testar conexão" (canto inferior
esquerdo) testa a aba exibida **com as alterações ainda não salvas** (resolve
com `Form::resolved`, o mesmo `resolve_entry` da carga). Desabilitado enquanto a
validação falha ou durante o teste.

- `connection::probe` abre um `Client` próprio (`open_client`, compartilhado
  com `connect`, sem ator), independente da conexão do workspace, e o fecha ao
  terminar. `readOnly` vale também no teste.
- Resumo exibido: tempo de conexão (inclui TLS) e se usou TLS; latência
  mediana/mín/máx de 5 × `SELECT 1`; versão do PostgreSQL, primário/réplica
  (`pg_is_in_recovery`), tempo no ar; usuário@banco, tamanho do banco, sessão
  somente leitura; conexões de cliente / `max_connections`, encoding, TimeZone.
  Falha na consulta dos dados do servidor não invalida o teste (só avisa).
- O resultado é descartado ao editar qualquer campo ou trocar de aba (ids
  descartam respostas antigas). A altura reservada para o resumo é descontada
  da área rolável para que os botões não sejam cortados.
- Testado contra PG real em `tests/postgres.rs`
  (`probe_measures_latency_and_reads_server_info`).

## Gravação

- Só o que mudou é reescrito: campo intocado mantém valor e tipo JSON originais;
  chaves desconhecidas ficam na mesma posição (`serde_json` com
  `preserve_order`). Campo de texto esvaziado remove a chave; `false` só é
  escrito se a chave já existia. Porta nova numérica vira número JSON, salvo se
  o original era string.
- `env_file` (nome antigo) é regravado como `envFile` no mesmo lugar.
- Recusa gravar se o arquivo mudou no disco desde a abertura (compara o texto).
- Mantém a indentação do arquivo; `fsutil::atomic_write` preserva as permissões
  do arquivo substituído (um `dbeasy.json` com senha pode ser `0600`).
- Depois de gravar, recarrega os workspaces. Renomear (`name` faz parte do
  `WorkspaceId`) mantém abas, conexão e sessão: `reload_workspaces_keeping`
  casa todas as entradas do arquivo gravado pela posição (o editor nunca remove
  nem reordena) e migra as chaves da sessão. Entradas novas aparecem na barra
  lateral como workspaces novos.
- Conexão aberta **não** é refeita (pode haver transação): o aviso pede para
  desconectar e conectar. Mudança de `sqlFilesDir` reinicia o explorador de
  arquivos.

## Verificação

Verificado no app em 2026-10-13 com captura interna (ver
[estabilidade da árvore](../arquitetura/iced-estabilidade-da-arvore.md)):
prévias, asteriscos, erro bloqueante, renomear mantendo o workspace ativo.
Abas: abrir pela 2ª entrada seleciona a aba dela; nova entrada com ⚠/●/×;
gravar acrescenta a entrada ao fim e ela aparece na barra lateral.
