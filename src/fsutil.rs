// SPDX-FileCopyrightText: 2026 Vitor Marinho Faria
// SPDX-License-Identifier: GPL-3.0-or-later

//! Utilitários de sistema de arquivos compartilhados pelos módulos de persistência.

use std::fs;
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};

/// Grava `contents` em `path` de forma atômica: escreve num arquivo temporário
/// no mesmo diretório, faz `fsync` e renomeia por cima do destino. Assim o
/// arquivo nunca fica parcialmente escrito (spec §3.4, §10.1).
pub fn atomic_write(path: &Path, contents: &[u8]) -> io::Result<()> {
    let dir = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "caminho sem diretório pai"))?;
    fs::create_dir_all(dir)?;

    let file_name = path.file_name().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "caminho sem nome de arquivo")
    })?;
    let mut tmp_name = file_name.to_os_string();
    tmp_name.push(format!(".tmp-{}", std::process::id()));
    let tmp_path = dir.join(tmp_name);

    let result = (|| {
        let mut file = fs::File::create(&tmp_path)?;
        // Mantém as permissões do arquivo substituído: um dbeas.json com
        // senha pode estar restrito ao dono (0600).
        if let Ok(meta) = fs::metadata(path) {
            file.set_permissions(meta.permissions())?;
        }
        file.write_all(contents)?;
        file.sync_all()?;
        fs::rename(&tmp_path, path)?;
        // Garante que a renomeação em si foi persistida.
        if let Ok(dir_file) = fs::File::open(dir) {
            let _ = dir_file.sync_all();
        }
        Ok(())
    })();

    if result.is_err() {
        let _ = fs::remove_file(&tmp_path);
    }
    result
}

/// Normaliza um caminho lexicamente (remove `.` e resolve `..`) sem tocar no
/// sistema de arquivos. Caminhos relativos são resolvidos a partir de `base`.
pub fn normalize(base: &Path, path: &Path) -> PathBuf {
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    };

    let mut out = PathBuf::new();
    for component in joined.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Caminho absoluto e canônico quando o arquivo existe; normalização léxica
/// caso contrário. Usado para evitar entradas duplicadas na configuração global.
pub fn canonical_or_normalized(path: &Path) -> PathBuf {
    match fs::canonicalize(path) {
        Ok(p) => p,
        Err(_) => {
            let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
            normalize(&cwd, path)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_write_replaces_content() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub").join("file.json");
        atomic_write(&path, b"one").unwrap();
        atomic_write(&path, b"two").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "two");
        let leftovers: Vec<_> = fs::read_dir(path.parent().unwrap()).unwrap().collect();
        assert_eq!(leftovers.len(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn atomic_write_keeps_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dbeas.json");
        fs::write(&path, b"[]").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        atomic_write(&path, b"[{}]").unwrap();
        let mode = fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn normalize_resolves_relative_and_dots() {
        let base = Path::new("/home/user/proj");
        assert_eq!(
            normalize(base, Path::new("./sql")),
            PathBuf::from("/home/user/proj/sql")
        );
        assert_eq!(
            normalize(base, Path::new("../x/.env")),
            PathBuf::from("/home/user/x/.env")
        );
        assert_eq!(
            normalize(base, Path::new("/abs/p")),
            PathBuf::from("/abs/p")
        );
    }
}
