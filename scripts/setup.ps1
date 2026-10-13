# SPDX-FileCopyrightText: 2026 Vitor Marinho Faria
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Prepara o projeto para `cargo run --release`: instala o cargo-about (se faltar)
# e gera THIRD_PARTY_LICENSES.html (se ausente ou mais antigo que o Cargo.lock).
# Idempotente. Equivalente no Linux/macOS: scripts/setup.sh.
#
# Uso: scripts\setup.ps1           # só faz o que falta
#      scripts\setup.ps1 -Force    # regenera os avisos mesmo se atuais
param([switch]$Force)
$ErrorActionPreference = 'Stop'
Set-Location (Split-Path -Parent $PSScriptRoot)
$file = 'THIRD_PARTY_LICENSES.html'

cargo about --version *> $null
if ($LASTEXITCODE -ne 0) {
    Write-Host '>> instalando cargo-about'
    cargo install --locked cargo-about --features cli
    if ($LASTEXITCODE -ne 0) { throw 'falha ao instalar cargo-about' }
}

$stale = -not (Test-Path $file) -or
    ((Get-Item Cargo.lock).LastWriteTime -gt (Get-Item $file).LastWriteTime)
if ($Force -or $stale) {
    Write-Host ">> gerando $file"
    cargo about generate about.hbs -o $file
    if ($LASTEXITCODE -ne 0) { throw 'falha ao gerar os avisos de terceiros' }
} else {
    Write-Host ">> $file já está atualizado"
}
