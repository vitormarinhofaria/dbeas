#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Vitor Marinho Faria
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Gera o pacote .deb em target/debian/. Verifica licenças e regenera os avisos
# de terceiros (que vão em /usr/share/doc/dbeas) antes de empacotar.
#
# Uso: scripts/package-deb.sh          # em container Debian 12 (pede glibc 2.35):
#                                      # instala em Ubuntu 22.04+ e Debian 12+
#      scripts/package-deb.sh --local  # com o toolchain da máquina: o pacote
#                                      # exige a glibc da máquina ou mais nova
#
# --local requer: cargo install --locked cargo-deb cargo-deny
#                 cargo install --locked cargo-about --features cli
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
IMAGE=dbeas-deb-builder

if [[ "${1:-}" == "--local" ]]; then
    shift
    cd "$ROOT"
    cargo deny check licenses
    cargo about generate about.hbs -o THIRD_PARTY_LICENSES.html
    cargo deb "$@"
    exit 0
fi

docker build -t "$IMAGE" -f "$ROOT/packaging/Dockerfile.deb" "$ROOT/packaging"
# Alvo separado para não misturar artefatos da glibc do container com os locais;
# o cache do registro do cargo fica num volume para builds seguintes.
docker run --rm \
    -v "$ROOT":/src -w /src \
    -v dbeas-cargo-registry:/usr/local/cargo/registry \
    -e CARGO_TARGET_DIR=/src/target/deb-bookworm \
    "$IMAGE" scripts/package-deb.sh --local --output target/debian "$@"
