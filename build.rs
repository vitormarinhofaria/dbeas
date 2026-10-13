// SPDX-FileCopyrightText: 2026 Vitor Marinho Faria
// SPDX-License-Identifier: GPL-3.0-or-later

//! Embute os avisos de terceiros (`THIRD_PARTY_LICENSES.html`, gerado pelo
//! cargo-about) no binário, para a tela "Sobre" sempre mostrá-los.
//!
//! Build de release falha sem o arquivo, ou com ele mais antigo que o
//! `Cargo.lock` (dependências mudaram depois de gerado): um binário distribuído
//! precisa dos avisos corretos. Em debug entra uma página explicando como gerá-lo.

use std::path::Path;
use std::{env, fs};

const FILE: &str = "THIRD_PARTY_LICENSES.html";

fn main() {
    println!("cargo::rerun-if-changed={FILE}");
    println!("cargo::rerun-if-changed=Cargo.lock");
    let out = Path::new(&env::var("OUT_DIR").unwrap()).join(FILE);
    let root = Path::new(&env::var("CARGO_MANIFEST_DIR").unwrap()).to_path_buf();
    let source = root.join(FILE);
    let release = env::var("PROFILE").as_deref() == Ok("release");
    let generate = format!(
        "Prepare o projeto com scripts/setup.sh (Linux/macOS) ou scripts\\setup.ps1 (Windows).\n\
         Manualmente: cargo about generate about.hbs -o {FILE}"
    );

    if source.is_file() {
        if release && older_than(&source, &root.join("Cargo.lock")) {
            panic!("{FILE} é mais antigo que o Cargo.lock: as dependências mudaram.\n{generate}");
        }
        fs::copy(&source, &out).expect("copiar THIRD_PARTY_LICENSES.html");
        return;
    }
    if release {
        panic!(
            "{FILE} não encontrado: build de release precisa dos avisos de terceiros.\n{generate}"
        );
    }
    fs::write(
        &out,
        "<!doctype html><meta charset=\"utf-8\"><title>Licenças de terceiros</title>\
         <p>Build de desenvolvimento sem os avisos de terceiros. Rode \
         <code>scripts/setup.sh</code> (ou <code>scripts\\setup.ps1</code>) \
         e recompile.</p>",
    )
    .expect("gravar página provisória");
}

fn older_than(a: &Path, b: &Path) -> bool {
    let modified = |p: &Path| fs::metadata(p).and_then(|m| m.modified()).ok();
    matches!((modified(a), modified(b)), (Some(a), Some(b)) if a < b)
}
