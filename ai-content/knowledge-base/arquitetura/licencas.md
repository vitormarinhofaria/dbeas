# Licença do projeto e das dependências

**Decisão (usuário, 2026-10-09):** o DBeas é distribuído sob **GPL-3.0-or-later**.

## Onde está declarada

- `LICENSE`: texto oficial da GPL-3.0 (gnu.org, SHA-256
  `3972dc9744f6499f0f9b2dbf76696f2ae7ad8af9b23dde66d6af86c9dfb36986`).
- `Cargo.toml`: `license = "GPL-3.0-or-later"`, `publish = false`.
- Todo arquivo `.rs` (src e tests) começa com:
  ```
  // SPDX-FileCopyrightText: 2026 Vitor Marinho Faria
  // SPDX-License-Identifier: GPL-3.0-or-later
  ```
  **Arquivos novos precisam receber o mesmo cabeçalho.**

## Compatibilidade das dependências (levantamento de 2026-10-09)

- ~500 crates com licenças permissivas (MIT, Apache-2.0, BSD, ISC, Zlib, BSL,
  Unicode-3.0, CC0, Unlicense, 0BSD): compatíveis.
- **Apache-2.0** (ex.: `sqlparser`): compatível com GPL-3, **incompatível com
  GPL-2.0-only** — por isso a versão 3.
- **MPL-2.0** (`option-ext`, via `dirs`): compatível (cláusula de Secondary License).
- Licenças duplas com opção GPL/LGPL (`self_cell`, `r-efi`): usa-se a opção
  permissiva.
- **rustls** usa o provedor `ring` (Apache-2.0 AND ISC). Não usar `aws-lc-rs`, que
  inclui código sob a licença OpenSSL antiga (incompatível com GPL-3).
- **libpg_query** (C embutido pelo `pg_query`): BSD-3-Clause + PostgreSQL License,
  compatíveis. Não é crate: os textos estão fixos em `about.hbs`.

## Verificação automática — cargo-deny

- `deny.toml`: lista `allow` de licenças compatíveis (GPL-2.0-only fora de
  propósito), avisos do RustSec, origem só do crates.io, sem wildcards.
- **Exceções de avisos (revisadas em 2026-10-09):** quatro avisos de "sem
  manutenção" (não são vulnerabilidades) em dependências transitivas do Iced,
  ignorados com motivo em `deny.toml`: `bincode` e `yaml-rust` (via `syntect`/
  `iced_highlighter`), `ttf-parser` (via `cosmic-text`) e `paste` (via
  `metal`/`wgpu-hal`). Revisar ao atualizar o Iced. Vulnerabilidades e avisos novos
  continuam falhando.
- Rodar `cargo deny check` ao adicionar ou atualizar dependências. Uma licença
  nova fora da lista falha a verificação: avaliar a compatibilidade com GPL-3
  antes de acrescentá-la (e acrescentar também em `about.toml`).

## Avisos de terceiros — cargo-about

- `about.toml` (mesma lista de licenças) + `about.hbs` (modelo HTML).
- Gerar ao distribuir binários:
  `cargo about generate about.hbs -o THIRD_PARTY_LICENSES.html`
- O arquivo gerado acompanha o binário; regenerar sempre que o `Cargo.lock` mudar.
  Ele não é versionado (`.gitignore`), para nunca ficar desatualizado no repositório.
- Ferramentas: `cargo install --locked cargo-deny` e
  `cargo install --locked cargo-about --features cli` (sem `cli` o binário não é
  instalado).
