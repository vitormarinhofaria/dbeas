#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Vitor Marinho Faria
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Prepara o projeto para `cargo run --release`: instala o cargo-about (se faltar)
# e gera THIRD_PARTY_LICENSES.html (se ausente ou mais antigo que o Cargo.lock).
# Idempotente. Equivalente no Windows: scripts/setup.ps1.
#
# Uso: scripts/setup.sh           # só faz o que falta
#      scripts/setup.sh --force   # regenera os avisos mesmo se atuais
set -euo pipefail
cd "$(dirname "$0")/.."
FILE=THIRD_PARTY_LICENSES.html

if ! cargo about --version >/dev/null 2>&1; then
    echo ">> instalando cargo-about"
    cargo install --locked cargo-about --features cli
fi

if [[ "${1:-}" == "--force" || ! -f "$FILE" || Cargo.lock -nt "$FILE" ]]; then
    echo ">> gerando $FILE"
    cargo about generate about.hbs -o "$FILE"
else
    echo ">> $FILE já está atualizado"
fi
