# Empacotamento .deb (Debian/Ubuntu)

**Decisão (2026-10-13):** pacotes `.deb` gerados com `cargo-deb`, a partir dos
metadados em `[package.metadata.deb]` no `Cargo.toml`.

## Como gerar

- `scripts/package-deb.sh`: compila num container `rust:1-bookworm`
  (`packaging/Dockerfile.deb`). O `Depends` gerado pede `libc6 >= 2.35`, e o
  pacote foi instalado sem bibliotecas faltando em **Ubuntu 22.04, Ubuntu 24.04
  e Debian 12** (containers limpos, 2026-10-13). Saída: `target/debian/dbeas_<versão>-1_amd64.deb`.
- Verificação: instalar com `apt-get install --no-install-recommends ./…deb` em
  container limpo, conferir `ldd` e as libs de dlopen (`ldconfig -p`). Sem display
  o binário entra em panic ao criar o event loop (esperado). Com `xvfb` e
  `DISPLAY=:99`, a janela abre e o app segue rodando (Ubuntu 22.04, 2026-10-13).
- `scripts/package-deb.sh --local`: usa o toolchain da máquina. O `Depends` passa
  a exigir a glibc da máquina (no Ubuntu 26.04, `libc6 >= 2.43`), então só serve
  para uso local.
- O script roda `cargo deny check licenses` e regenera `THIRD_PARTY_LICENSES.html`,
  instalado em `/usr/share/doc/dbeas/` com o `copyright` (texto do `LICENSE`).
- O build do container usa `target/deb-bookworm` para não misturar artefatos com
  os locais. O registro do cargo fica no volume Docker `dbeas-cargo-registry`.
- O primeiro build leva ~10 min (imagem + ferramentas); os seguintes, ~3 min.

## Integração com o desktop

- ID da aplicação: `io.github.vitormarinhofaria.DBeas` (`APP_ID` em
  `src/app/mod.rs`). Ele vai para a janela como `application_id` (Wayland) e
  WM_CLASS (X11), e é também o nome do `.desktop` e do ícone em `packaging/`.
  Se os três nomes divergirem, o dock mostra a janela sem ícone, separada do
  lançador.
- Ícone: SVG provisório em `hicolor/scalable/apps`.

## Dependências declaradas

- `$auto` só enxerga as bibliotecas ligadas diretamente (libc). O winit carrega
  as bibliotecas de janela com `dlopen` (xkbcommon, wayland-client, X11, Xcursor,
  Xi, Xrandr), por isso elas ficam em `Depends` explicitamente.
- `Recommends`: Vulkan/EGL (o Iced cai no renderizador de software tiny-skia sem
  GPU), `xdg-desktop-portal` (diálogos de arquivo do `rfd`) e uma fonte.
