# Tela "Sobre" (versão, copyright e licenças)

Botão **Sobre** na barra de status, ao lado de "Atalhos". Abre o modal
`Modal::About` (`src/app/view.rs`, `about_view`). Clicar fora fecha.

## Conteúdo

- Nome, versão (`CARGO_PKG_VERSION`), descrição, copyright e `GPL-3.0-or-later`.
- Aviso curto da GPL ("software livre… SEM NENHUMA GARANTIA") e a URL do
  código-fonte. Para programas interativos, a GPL recomenda exibir esse aviso.
- **Licença (GPL-3.0)**: mostra o texto integral dentro do modal, numa área com
  rolagem. O texto vem do `LICENSE`, embutido no binário com `include_str!`, então
  nunca falta. O botão vira **Voltar**.
- **Licenças de terceiros**: o `THIRD_PARTY_LICENSES.html` vai **embutido no
  binário** (`about::THIRD_PARTY_HTML`, copiado pelo `build.rs`). Ao clicar, o
  conteúdo embutido é regravado (`about::write_third_party`) em
  `$XDG_RUNTIME_DIR/dbeasy/` (ou `~/.cache/dbeasy/`, nunca o `/tmp` compartilhado)
  e aberto com `xdg-open`. Como o arquivo é regravado a cada clique, o que se vê é
  sempre o conteúdo do binário: alterar o arquivo em disco não muda nada. Decisão do
  usuário (2026-10-13): sempre disponível e só alterável trocando o binário.
- **Código-fonte**: abre a URL do repositório no navegador.
- Se não for possível abrir algo (arquivo ausente, sem `xdg-open`), o erro
  aparece em vermelho no próprio modal.

Constantes e lógica ficam em `src/about.rs`. Decisão de 2026-10-13. Ver também
[Licença e dependências](../arquitetura/licencas.md) e
[Empacotamento .deb](../arquitetura/empacotamento-deb.md).
