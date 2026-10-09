# DBeas --- Especificação Inicial do Produto

> **Status:** especificação inicial\
> **Stack definida:** Rust + Iced\
> **Escopo desta etapa:** cliente desktop para PostgreSQL com
> workspaces, editor SQL, navegação de objetos, visualização/edição de
> dados e persistência de sessão.\
> **Fora do escopo inicial:** IntelliSense de HQL/JPQL, inspeção de JARs
> e integração com processos Java.

## 1. Objetivo

Construir uma aplicação desktop para conectar a bancos PostgreSQL e
consultar, navegar e editar dados por meio de uma interface inspirada em
clientes de banco como o DBeaver.

A aplicação deve ser organizada em **workspaces**. Cada workspace é
definido por um arquivo `dbeas.json`, normalmente versionado junto a um
repositório Git. O usuário registra cada arquivo de workspace uma única
vez; a aplicação mantém uma lista dos workspaces registrados e restaura
o estado de trabalho entre execuções.

## 2. Princípios de implementação

-   **Rust + Iced:** interface e lógica principal da aplicação, sem
    WebView.
-   **Separação de responsabilidades:** interface, configuração,
    conexões, execução SQL, metadados, edição de dados e persistência de
    sessão devem ser módulos separados.
-   **PostgreSQL primeiro:** a primeira versão implementa apenas
    PostgreSQL, mas evita acoplar a interface a particularidades do
    driver.
-   **Dados tipados:** os valores devem manter seu tipo de banco
    internamente, mesmo que a primeira apresentação seja textual/bruta.
-   **Operações explícitas:** mudanças em dados não são enviadas ao
    banco sem uma ação explícita do usuário.
-   **Isolamento por workspace:** consultas, abas, metadados e estado
    visual pertencem ao workspace correspondente.
-   **Responsividade:** consultas, leitura de metadados e outras
    operações potencialmente lentas não devem bloquear a thread da
    interface.
-   **Segredos:** não registrar senhas ou conteúdo sensível em logs. O
    arquivo `dbeas.json` pode conter referências a variáveis de
    ambiente; não deve exigir que segredos sejam versionados.

## 3. Configuração de workspace

### 3.1. Arquivo `dbeas.json`

O arquivo contém um array JSON de configurações de workspace. Cada item
representa uma conexão/workspace exibida na aplicação.

Exemplo:

``` json
[
  {
    "name": "Desenvolvimento",
    "env_file": ".env",
    "env": {
      "DB_HOST": "localhost",
      "DB_PORT": "5432"
    },
    "dbHost": "${env.DB_HOST}",
    "dbPort": "${env.DB_PORT}",
    "dbUser": "${env.DB_USER}",
    "dbPass": "${env.DB_PASS}",
    "dbName": "app_dev",
    "readOnly": false,
    "requireTransaction": true,
    "sqlFilesDir": "./sql"
  }
]
```

Todos os campos apresentados são opcionais, exceto `dbName` e `name`, que são
obrigatórios e não possuem valores padrão. Em caso de colisão de `name` entre workspaces, deve utilizar o nome do repositório para exibir de forma unica.
### 3.2. Campos

  | Campo | Tipo | Obrigatório | Comportamento |
| --- | --- | --- | --- |
| `name` | string | **Sim** | Nome exibível do workspace. Valor vazio ou ausente torna a entrada inválida. |
| `env_file` | string | Não | Caminho para arquivo `.env`; caminho relativo é resolvido a partir do diretório do `dbeas.json`. |
| `env` | objeto string → string | Não | Variáveis adicionais ou sobrescritas usadas na resolução de configuração. |
| `dbHost` | string | Não | Host PostgreSQL. Se ausente ou vazio, usar `localhost`. Aceita `${env.NOME}`. |
| `dbPort` | número ou string numérica | Não | Porta PostgreSQL. Se ausente ou vazia, usar `5432`. Aceita `${env.NOME}`. |
| `dbUser` | string | Não | Usuário PostgreSQL. Aceita `${env.NOME}`. Se não resolvido, apresentar erro de configuração antes de conectar. |
| `dbPass` | string | Não | Senha PostgreSQL. Aceita `${env.NOME}`. Se omitido ou vazio, tentar conexão sem senha fornecida; o servidor ainda pode exigir autenticação e rejeitar a conexão. |
| `dbName` | string | **Sim** | Nome do banco de dados. Valor vazio ou ausente torna a configuração inválida. Aceita `${env.NOME}`. |
| `readOnly` | boolean | Não | Padrão `false`. Quando `true`, a aplicação deve tratar o workspace como somente leitura e impedir operações de escrita pela UI. |
| `requireTransaction` | boolean | Não | Padrão `false`. Quando `true`, alterações de dados devem ser executadas em transação explícita e confirmadas pelo usuário antes do `COMMIT`. |
| `sqlFilesDir` | string | Não | Diretório inicial do explorador de arquivos SQL. Caminho relativo é resolvido a partir do diretório do `dbeas.json`. Se omitido ou inválido, a área de arquivos informa que não há diretório configurado. |

### 3.3. Resolução de variáveis e precedência

Valores de configuração podem referenciar variáveis com a sintaxe
`${env.NOME}`. Exemplo: `"dbHost": "${env.DB_HOST}"`.

A resolução deve seguir esta ordem, da menor para a maior precedência:

1.  Variáveis herdadas do ambiente do processo.
2.  Variáveis carregadas de `env_file`.
3.  Entradas do objeto `env` no workspace.

Assim, `env` pode sobrescrever valores existentes no ambiente ou no
`.env`. Referências `${env.NOME}` são resolvidas usando o ambiente
resultante. Uma referência não resolvida deve produzir um erro de
configuração claro, indicando o campo e a variável ausente, sem imprimir
segredos.

Outras regras:

-   Não executar expansão de shell, substituição de comandos ou sintaxe
    além de `${env.NOME}`.
-   Não interpretar automaticamente valores arbitrários como booleanos.
    `readOnly` e `requireTransaction` devem ser booleanos JSON.
-   Validar `dbPort` como inteiro entre 1 e 65535 depois da resolução.
-   Normalizar caminhos relativos com base no diretório que contém o
    `dbeas.json`, não no diretório de trabalho do processo.
-   Não incluir a senha em mensagens de erro, logs ou diagnósticos.

### 3.4. Configuração global da aplicação

Ao adicionar um arquivo de workspace, registrar seu caminho absoluto no
arquivo:

`~/.config/dbeas/config`

O formato interno ainda pode evoluir, mas deve conter uma lista de
caminhos dos arquivos `dbeas.json` registrados e ser versionado por uma
propriedade de versão de esquema. Exemplo provisório:

``` json
{
  "version": 1,
  "workspaces": [
    "/home/user/projeto-a/dbeas.json",
    "/home/user/projeto-b/dbeas.json"
  ]
}
```

Requisitos:

-   O usuário adiciona cada arquivo de configuração uma vez.
-   Evitar entradas duplicadas após normalização dos caminhos.
-   Permitir remover um workspace da lista sem apagar o arquivo do
    projeto.
-   Ao iniciar, carregar todos os arquivos registrados que ainda
    existam.
-   Se um arquivo não existir ou estiver inválido, mostrar o workspace
    como indisponível/com erro, sem impedir a abertura dos demais.
-   Persistir alterações do arquivo global de forma segura, evitando
    deixar o arquivo parcialmente escrito.
-   A configuração global guarda referências aos arquivos de workspace;
    o estado das abas e consultas deve ficar em armazenamento separado.

## 4. Modelo de interface

A janela principal deve ter três regiões:

1.  **Barra lateral esquerda:** seleção de workspace e exploradores.
2.  **Área central:** abas de tabelas, consultas scratch e arquivos SQL.
3.  **Área de consulta e resultados:** cada aba SQL/tabela contém
    resultados na parte superior e editor SQL na parte inferior.

A interface deve permitir redimensionar a barra lateral e a divisão
vertical entre resultados e editor. O layout exato pode evoluir, mas
deve manter visível o contexto do workspace ativo.

### 4.1. Barra lateral: workspaces

-   Exibir todos os workspaces registrados/abertos em uma faixa
    vertical.
-   Mostrar a propriedade `name` da entrada correspondente no
    `dbeas.json` como identificação primária.
-   Se houver valores de `name` duplicados entre workspaces, acrescentar
    o nome do repositório Git ou, quando não for possível identificá-lo,
    o nome do diretório pai para evitar ambiguidades.
-   O item ativo deve ser visualmente destacado e o nome do workspace
    ativo deve aparecer de forma clara no cabeçalho da área central.
-   Cada item deve exibir explicitamente o estado da conexão, com ícone
    e texto/tooltip acessível. Estados mínimos: **desconectado**,
    **conectando**, **conectado** e **erro**.
-   Selecionar um workspace troca o contexto central para o estado desse
    workspace, sem misturar suas abas com as de outros workspaces.
-   `Ctrl+Tab` percorre circularmente os workspaces registrados na
    sessão, avançando para o próximo workspace e voltando ao primeiro ao
    atingir o último.

### 4.2. Barra lateral: exploradores

Abaixo ou junto à seleção de workspace, oferecer modos alternáveis:

1.  **Tabelas:** lista das tabelas e views acessíveis no banco ativo.
    Clicar em um objeto abre sua aba de dados.
2.  **Objetos do banco:** visão mais geral, inspirada no navegador do
    DBeaver. A arquitetura deve permitir acrescentar categorias como
    schemas, views, funções, sequências e outros objetos PostgreSQL.
    Implementar inicialmente as categorias essenciais disponíveis com
    segurança; categorias ainda não implementadas não devem aparecer
    como funcionais.
3.  **Arquivos SQL:** explorador iniciado em `sqlFilesDir`. Permite
    navegar, abrir, editar, salvar e executar arquivos `.sql`.

A troca do modo de exploração não deve fechar nem alterar as abas
abertas.

### 4.3. Área central e abas

-   A área central é única para a janela, mas o estado das abas é
    mantido separadamente por workspace.
-   A primeira aba lógica de cada workspace é sempre o documento
    **Scratch**, para consultas manuais sem associação obrigatória a uma
    tabela.
-   As demais abas podem representar tabelas/views, arquivos SQL e,
    futuramente, outros tipos de documento.
-   A barra horizontal de abas deve permitir selecionar e fechar abas,
    preservando o Scratch.
-   Cada aba SQL/tabela possui:
    -   grade de resultados na parte superior;
    -   editor SQL na parte inferior;
    -   separador redimensionável entre as regiões.
-   Abrir uma tabela gera uma consulta inicial que seleciona suas
    colunas. O comportamento de edição de dados depende da capacidade de
    mapear o resultado para uma única tabela atualizável.
-   Abrir um arquivo SQL carrega seu conteúdo no editor; salvar atualiza
    o arquivo original, respeitando permissões e erros de escrita.
-   Alterações não salvas devem ser indicadas visualmente e confirmadas
    antes de descartar conteúdo.
-   Ao trocar de workspace, apresentar as abas pertencentes ao workspace
    selecionado. Nenhuma aba deve ser transferida automaticamente para
    outro workspace.

## 5. Grade de dados

### 5.1. Apresentação e tipos

-   Na primeira versão, exibir os valores em uma representação
    bruta/textual adequada ao tipo retornado pelo PostgreSQL.
-   Preservar internamente tipo, valor nulo e metadados da coluna para
    permitir renderizadores específicos no futuro.
-   Distinguir claramente `NULL` de texto vazio.
-   Suportar resultados grandes sem carregar ou renderizar todos os
    registros de uma vez; usar paginação ou carregamento incremental e
    virtualização.
-   Mostrar erros de consulta sem destruir desnecessariamente o último
    resultado válido, quando isso for viável.

### 5.2. Ordenação por cabeçalho

-   Clicar em um cabeçalho alterna a ordenação da coluna
    (ascendente/descendente; um terceiro estado sem ordenação é
    opcional).
-   A ordenação deve ser refletida no SQL exibido no editor.
-   Quando o SQL original for uma consulta arbitrária, a transformação
    deve ser feita por análise estrutural de SQL, não por concatenação
    ingênua de texto.
-   Se não for possível transformar a consulta com segurança, informar o
    motivo e não alterar silenciosamente a consulta.
-   A implementação deve definir se a ordenação é executada
    imediatamente ou apenas altera a consulta até o usuário executá-la;
    a experiência precisa ser consistente e explícita.

### 5.3. Ocultar colunas com o botão do meio

-   Clicar com o botão do meio sobre o cabeçalho de uma coluna oculta
    essa coluna na grade.
-   A área do cabeçalho correspondente deve ser reduzida/ocultada e a
    coluna de dados correspondente não deve continuar visível.
-   A remoção da coluna deve ser refletida no SQL exibido.
-   Se a consulta não permitir determinar de forma segura a expressão
    que representa a coluna, não modificar o SQL automaticamente;
    informar ao usuário.
-   Deve haver uma forma de restaurar colunas ocultadas, por exemplo por
    um menu contextual ou ação de restaurar colunas.

### 5.4. Reordenação de colunas

-   Permitir arrastar cabeçalhos para reordenar colunas.
-   A ordem visual deve ser refletida na lista de colunas do `SELECT` no
    editor SQL.
-   A transformação deve preservar expressões, aliases e demais partes
    da consulta.
-   Se a transformação não puder ser aplicada com segurança, manter a
    alteração apenas visual e indicar que a consulta não foi modificada.
    Não corromper SQL manual.

### 5.5. Regras para transformações de SQL

Ordenação, ocultação e reordenação precisam compartilhar um mecanismo de
transformação de consulta. Não implementar cada recurso por
substituições independentes de strings.

-   Usar um parser/AST SQL quando disponível e adequado ao PostgreSQL.
-   Preservar o texto original e os trechos que não precisarem mudar.
-   Distinguir consulta gerada pela aplicação de consulta livre editada
    pelo usuário.
-   Para consultas com `SELECT *`, joins, aliases duplicados, CTEs,
    expressões calculadas, agrupamento ou consultas não atualizáveis,
    aplicar somente transformações que possam ser mapeadas sem
    ambiguidade.
-   Se não houver transformação segura, explicar a limitação e oferecer
    a operação visual quando isso não contrariar o pedido explícito de
    refletir no SQL.

## 6. Edição de dados e salvamento

### 6.1. Alterações na grade

-   Permitir editar células em resultados que possam ser mapeados de
    forma inequívoca para uma linha e coluna de uma tabela PostgreSQL.
-   Não permitir edição direta de resultados arbitrários que não possam
    ser mapeados com segurança.
-   Manter um estado de alterações pendentes separado dos valores
    originais.
-   Indicar explicitamente as células e linhas alteradas, incluindo
    valor original e valor proposto.
-   Oferecer reversão das alterações pendentes antes de enviá-las ao
    banco.

### 6.2. Atalhos e confirmação

-   `Ctrl+S`: preparar as alterações pendentes, gerar o SQL
    correspondente e abrir um popup de confirmação.
-   O popup deve exibir o SQL que será executado, com opção de:
    -   **Copiar e fechar:** copia o SQL e fecha o popup, sem executar.
    -   **Executar e fechar:** executa as alterações e fecha o popup se
        houver sucesso.
    -   **Cancelar e reverter:** descarta as alterações pendentes da
        operação e fecha o popup.
-   `Ctrl+Shift+S`: executar diretamente as alterações pendentes, sem
    popup, conforme as regras transacionais do workspace.
-   Se não houver alterações pendentes, os atalhos não devem executar
    operações de escrita.
-   Erros de execução devem ser apresentados sem perder silenciosamente
    as alterações pendentes.

### 6.3. Modo somente leitura

Quando `readOnly: true`:

-   Desabilitar edição de células e operações de escrita pela interface.
-   Bloquear a execução de comandos de escrita identificados como tais.
-   Usar, quando suportado, uma conexão/transação PostgreSQL em modo
    somente leitura como segunda camada de proteção.
-   Não confiar exclusivamente em análise textual para classificar SQL;
    comandos com efeitos indiretos ou múltiplas instruções exigem
    tratamento conservador.
-   Explicar que a opção da aplicação é uma proteção da ferramenta, não
    substitui permissões somente leitura configuradas no próprio
    PostgreSQL.

### 6.4. Transações obrigatórias

Quando `requireTransaction: true`:

-   Alterações de dados devem ocorrer dentro de uma transação explícita.
-   A aplicação deve permitir confirmar (`COMMIT`) ou reverter
    (`ROLLBACK`) as mudanças.
-   Exibir claramente quando existe uma transação aberta e quando há
    alterações ainda não confirmadas.
-   Não confirmar automaticamente uma transação aberta apenas porque o
    usuário trocou de aba ou workspace.
-   Definir um fluxo explícito para fechar a aplicação ou desconectar
    com transação pendente: confirmar, reverter ou cancelar o
    fechamento.
-   Para a primeira versão, priorizar a segurança das alterações feitas
    pela grade. O comportamento de transações para scripts SQL
    arbitrários deve ser documentado e consistente, sem envolver
    implicitamente comandos de controle de transação enviados pelo
    usuário.

## 7. Editor SQL

### 7.1. Recursos essenciais

-   Editor de texto SQL integrado em cada aba de consulta.
-   Highlight de sintaxe.
-   Números de linha.
-   Exibição de erros e diagnósticos com linha e coluna.
-   Autocomplete de tabelas, views, schemas e colunas reais obtidas da
    conexão ativa.
-   Atalhos de edição e execução de consulta, com conflitos de atalhos
    documentados.
-   Executar a consulta atual ou a seleção de texto, se houver seleção.
-   Resultados e mensagens de erro devem ser associados à aba que
    iniciou a execução.

### 7.2. Parser, diagnósticos e LSP

-   Avaliar Tree-sitter para realce incremental e estrutura sintática.
    Tree-sitter, por si só, não substitui a validação semântica do
    PostgreSQL.
-   Integrar um servidor LSP SQL apropriado para diagnóstico e
    autocomplete quando viável. A implementação deve avaliar
    compatibilidade com PostgreSQL, manutenção, configuração e
    capacidade de obter metadados do banco.
-   Se o servidor LSP não estiver disponível, o editor deve continuar
    utilizável; destacar a funcionalidade indisponível sem impedir a
    execução manual de SQL.
-   Os diagnósticos precisam ser mapeados corretamente para linha e
    coluna no documento ativo.
-   Autocomplete de objetos do banco deve usar metadados obtidos pela
    conexão do workspace ativo, sem misturar metadados de outros
    workspaces.
-   Atualizar/invalidate o cache de metadados quando o usuário solicitar
    atualização ou quando a aplicação detectar que ele está
    desatualizado.

## 8. Exploração de arquivos SQL

-   O explorador começa em `sqlFilesDir` do workspace ativo.
-   Permitir navegar por diretórios, abrir arquivos `.sql`, editar,
    salvar e executar o conteúdo ou a seleção.
-   Indicar alterações não salvas.
-   Lidar com diretórios ausentes ou inacessíveis exibindo uma mensagem
    útil, sem impedir o restante do workspace.
-   Restringir a navegação inicial ao diretório configurado por padrão.
    Se permitir navegar para fora dele, isso deve ser uma ação explícita
    e não ocorrer acidentalmente por resolução de caminho.
-   Não executar automaticamente um arquivo ao abri-lo.

## 9. Navegação rápida e atalhos

### `Ctrl+P` --- seletor rápido

-   Abrir um seletor semelhante ao do VS Code para buscar tabelas/views
    e arquivos SQL do workspace ativo.
-   Filtrar conforme o usuário digita.
-   Selecionar um objeto abre sua aba; selecionar um arquivo abre o
    editor correspondente.
-   O seletor deve indicar o tipo do resultado e evitar duplicar abas do
    mesmo recurso quando o comportamento de reutilização for apropriado.

### `Ctrl+Tab` --- alternar workspace

-   Alternar circularmente entre workspaces registrados/abertos na
    sessão.
-   A mudança deve atualizar a barra lateral, o cabeçalho e a área
    central.
-   Cada workspace restaura suas próprias abas e estado de navegação.

Os atalhos devem ser centralizados em um mapa de comandos, evitando
implementações conflitantes em cada componente. A aplicação deve
respeitar o foco do editor quando isso for necessário e documentar
exceções.

## 10. Persistência e ciclo de vida

### 10.1. Estado persistente

Ao fechar e reabrir a aplicação, restaurar por workspace:

-   abas abertas e sua ordem;
-   aba ativa;
-   conteúdo de consultas scratch;
-   conteúdo e caminho dos arquivos SQL abertos;
-   estado necessário para restaurar abas de tabelas, incluindo consulta
    e configuração visual;
-   modo ativo do explorador lateral e preferências de layout
    relevantes.

O estado de sessão deve ser salvo em um arquivo separado da configuração
global, com versão de esquema e gravação segura. O caminho e o formato
exatos podem ser definidos durante a implementação.

Não persistir como se fossem válidos resultados antigos de consultas nem
o estado de uma conexão ativa. Resultados podem ser descartados ao
reabrir, salvo se houver uma decisão explícita posterior para
armazená-los.

### 10.2. Memória e recursos

-   Manter os estados dos workspaces da sessão para que a troca seja
    fluida.
-   Não manter indefinidamente todas as conexões, cursores e resultados
    completos em memória.
-   Separar estado serializável da sessão de recursos transitórios, como
    conexão, cursor, execução em andamento e cache de metadados.
-   Aplicar limites ou políticas de descarte para resultados grandes e
    caches.
-   Ao desconectar ou remover um workspace, cancelar ou finalizar
    operações com segurança e liberar seus recursos.
-   Ao remover um workspace da configuração global, não apagar arquivos
    do projeto nem arquivos SQL.

## 11. Arquitetura lógica sugerida

Organizar o código em módulos com interfaces explícitas. A estrutura
final pode variar, mas deve manter responsabilidades equivalentes a:

-   `app`: estado global, navegação e ciclo de vida da janela.
-   `workspace`: carregamento, validação, resolução de variáveis e
    estado por workspace.
-   `settings`: registro global de workspaces e preferências.
-   `connection`: conexão PostgreSQL, reconexão, estado e desconexão.
-   `metadata`: schemas, tabelas, colunas, views e metadados usados pelo
    autocomplete.
-   `query`: execução SQL, cancelamento, resultados, diagnósticos e
    transformações de SQL.
-   `data_grid`: apresentação virtualizada, ordenação, ocultação,
    reordenação e edição.
-   `sql_editor`: texto, highlight, diagnósticos, autocomplete e
    comandos do editor.
-   `file_explorer`: navegação e operações de arquivos SQL.
-   `session`: persistência e restauração das abas por workspace.
-   `commands`: mapa central de atalhos e ações da aplicação.

Operações de I/O e consultas devem ocorrer fora da atualização síncrona
da interface Iced, usando tarefas/comandos assíncronos e retornando
mensagens de resultado para atualizar o estado.

## 12. Segurança e confiabilidade

-   Não registrar senhas, strings de conexão com senha ou conteúdo
    sensível em logs.
-   Não exibir senha em mensagens de erro ou no popup de SQL.
-   Tratar `dbeas.json`, `.env`, arquivos SQL e metadados externos como
    entradas que podem estar malformadas.
-   Validar configurações antes de tentar conectar.
-   Não executar SQL automaticamente ao abrir um workspace, uma aba ou
    um arquivo.
-   Nunca executar escrita silenciosamente como efeito colateral de
    ordenação, ocultação ou reordenação da grade.
-   Em caso de transformação SQL ambígua, falhar de forma segura e
    informar o usuário.
-   Usar parâmetros de consulta quando aplicável, em especial para
    operações de edição de dados, evitando concatenar valores
    diretamente no SQL.
-   Não confiar no modo `readOnly` da aplicação como substituto para
    privilégios PostgreSQL.

## 13. Critérios de aceitação da primeira versão

1.  O usuário consegue registrar um arquivo `dbeas.json`, reiniciar a
    aplicação e ver o workspace novamente.
2.  Configurações válidas conectam ao PostgreSQL; configurações
    inválidas mostram erros acionáveis sem impedir a abertura de outros
    workspaces.
3.  Host e porta assumem `localhost` e `5432` quando ausentes/vazios;
    `dbName` é obrigatório.
4.  A resolução de variáveis respeita a precedência documentada e não
    expõe segredos.
5.  É possível alternar workspaces, visualizar seus estados de conexão e
    restaurar suas abas separadamente.
6.  É possível navegar por tabelas/views e abrir uma grade com
    resultados e editor SQL.
7.  O editor oferece highlight de sintaxe, diagnóstico com linha/coluna
    e autocomplete baseado no banco ativo, ou informa claramente quais
    desses serviços estão indisponíveis.
8.  Ordenar, ocultar e reordenar colunas não corrompe SQL. Quando a
    transformação não for segura, a aplicação informa a limitação.
9.  Edições de dados são identificadas como pendentes; `Ctrl+S`
    apresenta SQL para confirmação e `Ctrl+Shift+S` executa diretamente.
10. `readOnly` impede operações de escrita iniciadas pela UI;
    `requireTransaction` oferece um fluxo explícito de
    confirmação/reversão.
11. É possível abrir, editar, salvar e executar arquivos SQL do
    diretório configurado.
12. `Ctrl+P` busca tabelas/views e arquivos SQL do workspace ativo;
    `Ctrl+Tab` alterna workspaces circularmente.
13. A sessão restaura abas e consultas por workspace sem manter conexões
    ou resultados obsoletos como se ainda fossem válidos.
14. Consultas lentas e resultados grandes não bloqueiam a interface nem
    exigem carregar todo o resultado em memória.

## 14. Fora do escopo inicial

-   IntelliSense, parsing ou validação de HQL/JPQL.
-   Inspeção de classes ou JARs Java.
-   Processo auxiliar Java ou comunicação com JVM.
-   Suporte a bancos diferentes de PostgreSQL.
-   Sistema de plugins.
-   Renderizadores especializados por tipo de dado.
-   Edição genérica de resultados SQL que não possam ser mapeados
    inequivocamente para uma tabela.
-   Sincronização de sessão entre máquinas.
-   Persistência offline de resultados de consultas.

## 15. Decisões pendentes que não devem bloquear o esqueleto

-   Escolha e integração do driver PostgreSQL para Rust.
-   Servidor LSP SQL específico e estratégia de instalação/configuração.
-   Parser/AST para transformações de consultas PostgreSQL.
-   Formato definitivo e migração do arquivo de sessão.
-   Estratégia de paginação e carregamento incremental da grade.
-   Escopo exato das categorias do explorador de objetos PostgreSQL na
    primeira entrega.
-   Política para SQL arbitrário quando `requireTransaction` está
    habilitado, especialmente scripts com múltiplas instruções e
    comandos explícitos de transação.

Ao implementar decisões pendentes, documentar a escolha e preservar as
interfaces entre módulos para permitir substituição posterior.
