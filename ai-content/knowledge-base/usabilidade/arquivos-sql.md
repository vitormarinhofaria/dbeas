# Explorador de arquivos SQL

Spec §8 e §4.2. Código: `src/file_explorer.rs` (regras de caminho, testadas) e o
modo "Arquivos" da barra lateral (`src/app/view.rs::files_explorer`).

## Navegação

- Começa em `sqlFilesDir`; lista diretórios e arquivos `.sql` (ocultos não
  aparecem). Caminhos são canonicalizados e precisam ficar dentro da raiz: `..`
  e links simbólicos para fora são recusados (`resolve_within`).
- Abrir um arquivo não o executa. Arquivos salvos podem conter BEGIN/COMMIT (ver
  [transações](transacoes.md)).

## Criar arquivo (acrescentado à spec em 2026-10-09)

- Botão "+ Novo arquivo" ao lado do caminho exibido. Sem `sqlFilesDir` ou com o
  diretório inacessível, aparece desabilitado com o motivo no tooltip.
- O nome é relativo ao **diretório exibido** no explorador (dentro de
  `sqlFilesDir`); subdiretórios existentes são aceitos (`relatorios/mensal`), mas
  não são criados. `.sql` é acrescentado quando a extensão não é `.sql`.
- Recusados com mensagem no próprio diálogo: nome vazio, terminado em `/`,
  caminho absoluto, saída da raiz (`..` ou link), diretório inexistente e
  arquivo já existente.
- Criação com `OpenOptions::create_new` (falha atômica se o arquivo surgir entre
  a validação e a criação): nunca sobrescreve.
- Depois de criar: fecha o diálogo, atualiza a listagem e abre o arquivo vazio em
  nova aba, sem executar.

## Menu de contexto: renomear e excluir (pedido do usuário, 2026-10-09)

- Clique direito num arquivo ou diretório abre o menu logo abaixo do item (dentro
  da lista, sem posicionamento por coordenadas). Fecha com Esc, ao escolher uma
  opção, ao clicar em outro item/aba/editor ou em área vazia
  (`closes_file_menu` + clique esquerdo ignorado pelos widgets).
- **Renomear/mover** (`file_explorer::resolve_rename`/`rename`; regra do usuário
  em 2026-10-09): o campo traz o caminho do item **relativo ao `sqlFilesDir`**
  (`relative_to_root`). Mudar só o nome renomeia; mudar os diretórios move o item
  para qualquer diretório **existente abaixo do `sqlFilesDir`**. Qualquer `..` é
  recusado, mesmo que o caminho voltasse para dentro da raiz; também são
  recusados caminho absoluto, destino fora da raiz por link simbólico, diretório
  de destino inexistente (não é criado), mover um diretório para dentro dele
  mesmo, nome igual e destino existente. Arquivos mantêm `.sql` (acrescentado se
  faltar). Abas abertas e o diretório atual acompanham o novo caminho (inclusive
  arquivos dentro de diretório movido). Limitação: entre a verificação e o
  `rename` outro processo pode criar o destino (o `rename` do sistema
  sobrescreveria arquivos).
- **Excluir** (`file_explorer::move_to_trash`): move para a **lixeira do sistema**
  (crate `trash`, recuperável), nunca apaga definitivamente. Diretórios podem ser
  excluídos com o conteúdo; a confirmação informa quantos `.sql` há dentro, quais
  abas serão fechadas e se alguma tem alterações não salvas. Abas dos itens
  removidos são fechadas (o Scratch nunca); se o diretório atual foi removido, o
  explorador volta à raiz. Links simbólicos são tratados como o próprio item (não
  se segue o destino).

## Exibição de caminhos

- Mensagens e a barra de status mostram o caminho relativo ao `sqlFilesDir`
  (`display_path`); textos da barra de status não quebram linha.
