# Identificação do workspace na barra lateral

**Regra:** a barra lateral identifica cada workspace pela propriedade `name` da
entrada correspondente no array do `dbeas.json` — nunca pelo nome do arquivo
`dbeas.json`.

**Desambiguação:** se dois workspaces tiverem o mesmo `name`, acrescentar o nome
do repositório Git ou, quando não for possível identificá-lo, o nome do diretório
pai.

**Motivo:** um mesmo `dbeas.json` contém várias entradas, então o nome do arquivo
não distingue workspaces.

**Referência:** `dbeas-spec-inicial.md` §4.1 (corrigido em 2026-10-09).
