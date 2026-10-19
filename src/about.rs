// SPDX-FileCopyrightText: 2026 Vitor Marinho Faria
// SPDX-License-Identifier: GPL-3.0-or-later

//! Informações de versão, copyright e licenças exibidas na tela "Sobre".

use std::ffi::OsStr;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::fsutil;

pub const NAME: &str = "DBeasy";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const DESCRIPTION: &str = env!("CARGO_PKG_DESCRIPTION");
pub const COPYRIGHT: &str = "Copyright © 2026 Vitor Marinho Faria";
pub const LICENSE_ID: &str = "GPL-3.0-or-later";
pub const SOURCE_URL: &str = "https://github.com/vitormarinhofaria/dbeasy";

/// Aviso recomendado pela GPL para programas interativos (seção
/// "How to Apply These Terms to Your New Programs").
pub const NOTICE: &str = "Este programa é software livre: você pode redistribuí-lo e/ou \
modificá-lo sob os termos da GNU General Public License, publicada pela Free Software \
Foundation, na versão 3 da licença ou (a seu critério) em qualquer versão posterior.\n\n\
Este programa é distribuído na esperança de que seja útil, mas SEM NENHUMA GARANTIA, nem \
mesmo a garantia implícita de COMERCIABILIDADE ou de ADEQUAÇÃO A UM PROPÓSITO ESPECÍFICO. \
Veja o texto da licença para mais detalhes.";

/// Texto integral da GPL-3.0, embutido no binário para estar sempre disponível.
pub const LICENSE_TEXT: &str = include_str!("../LICENSE");

/// Avisos de terceiros gerados pelo cargo-about, embutidos pelo `build.rs`.
pub const THIRD_PARTY_HTML: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/THIRD_PARTY_LICENSES.html"));

/// Grava os avisos embutidos em `dir` e devolve o caminho. Regrava a cada
/// chamada: o que se abre é sempre o conteúdo do binário, mesmo que alguém
/// tenha alterado o arquivo de uma abertura anterior.
pub fn write_third_party(dir: &Path) -> io::Result<PathBuf> {
    let path = dir.join(format!("dbeasy-{VERSION}-THIRD_PARTY_LICENSES.html"));
    fsutil::atomic_write(&path, THIRD_PARTY_HTML)?;
    Ok(path)
}

/// Abre os avisos de terceiros embutidos no navegador. O arquivo vai para um
/// diretório privado do usuário (runtime do XDG ou cache), nunca o /tmp
/// compartilhado.
pub fn open_third_party() -> Result<(), String> {
    let dir = dirs::runtime_dir()
        .or_else(dirs::cache_dir)
        .ok_or("diretório para o arquivo temporário não encontrado")?
        .join("dbeasy");
    let path = write_third_party(&dir)
        .map_err(|e| format!("não foi possível gravar as licenças de terceiros: {e}"))?;
    open_external(path)
}

/// Abre um arquivo ou URL no programa padrão do sistema.
pub fn open_external(target: impl AsRef<OsStr>) -> Result<(), String> {
    let target = target.as_ref();
    let mut child = Command::new(opener())
        .arg(target)
        .spawn()
        .map_err(|e| format!("não foi possível abrir {}: {e}", target.to_string_lossy()))?;
    // O abridor termina logo; recolhê-lo evita um processo zumbi.
    std::thread::spawn(move || child.wait());
    Ok(())
}

fn opener() -> &'static str {
    if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn third_party_file_is_rewritten_from_the_binary() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_third_party(dir.path()).unwrap();
        std::fs::write(&path, "alterado").unwrap();
        let again = write_third_party(dir.path()).unwrap();
        assert_eq!(again, path);
        assert_eq!(std::fs::read(&path).unwrap(), THIRD_PARTY_HTML);
    }

    #[test]
    fn embedded_license_is_the_gpl3() {
        assert!(LICENSE_TEXT.contains("GNU GENERAL PUBLIC LICENSE"));
        assert!(LICENSE_TEXT.contains("Version 3, 29 June 2007"));
    }
}
