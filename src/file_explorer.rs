//! Navegação e operações de arquivos SQL (spec §8).
//!
//! A navegação fica restrita ao `sqlFilesDir`: caminhos são canonicalizados
//! (resolvendo `..` e links simbólicos) e precisam continuar dentro da raiz.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEntry {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
}

/// Limite de arquivos coletados para o seletor rápido.
const SCAN_LIMIT: usize = 5_000;

pub fn is_sql(path: &Path) -> bool {
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("sql"))
}

/// `Some(caminho canônico)` se `path` estiver dentro de `root`.
pub fn resolve_within(root: &Path, path: &Path) -> Option<PathBuf> {
    let root = std::fs::canonicalize(root).ok()?;
    let path = std::fs::canonicalize(path).ok()?;
    path.starts_with(&root).then_some(path)
}

/// Lista diretórios e arquivos `.sql` de `dir`, que precisa estar dentro de
/// `root`. Diretórios primeiro, depois arquivos, ambos em ordem alfabética.
pub async fn list_dir(root: PathBuf, dir: PathBuf) -> Result<Vec<FileEntry>, String> {
    tokio::task::spawn_blocking(move || list_dir_sync(&root, &dir))
        .await
        .map_err(|e| e.to_string())?
}

fn list_dir_sync(root: &Path, dir: &Path) -> Result<Vec<FileEntry>, String> {
    if !root.is_dir() {
        return Err(format!(
            "o diretório configurado não existe ou não é acessível: {}",
            root.display()
        ));
    }
    let dir = resolve_within(root, dir)
        .ok_or_else(|| "o diretório está fora do sqlFilesDir".to_string())?;
    let read = std::fs::read_dir(&dir)
        .map_err(|e| format!("não foi possível ler {}: {e}", dir.display()))?;

    let mut entries: Vec<FileEntry> = read
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                return None;
            }
            let is_dir = path.is_dir();
            (is_dir || is_sql(&path)).then_some(FileEntry { name, path, is_dir })
        })
        .collect();
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(entries)
}

/// Coleta recursivamente os arquivos `.sql` sob `root`, sem seguir links que
/// saiam dela.
pub async fn scan_sql_files(root: PathBuf) -> Vec<PathBuf> {
    tokio::task::spawn_blocking(move || {
        let mut out = Vec::new();
        let Ok(canonical_root) = std::fs::canonicalize(&root) else {
            return out;
        };
        let mut stack = vec![canonical_root.clone()];
        while let Some(dir) = stack.pop() {
            let Ok(read) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in read.filter_map(Result::ok) {
                if entry.file_name().to_string_lossy().starts_with('.') {
                    continue;
                }
                let Ok(path) = std::fs::canonicalize(entry.path()) else {
                    continue;
                };
                if !path.starts_with(&canonical_root) {
                    continue;
                }
                if path.is_dir() {
                    stack.push(path);
                } else if is_sql(&path) {
                    out.push(path);
                    if out.len() >= SCAN_LIMIT {
                        return out;
                    }
                }
            }
        }
        out.sort();
        out
    })
    .await
    .unwrap_or_default()
}

pub async fn read_file(path: PathBuf) -> Result<String, String> {
    tokio::fs::read_to_string(&path)
        .await
        .map_err(|e| format!("não foi possível abrir {}: {e}", path.display()))
}

/// Grava no próprio arquivo (sem trocar o inode), preservando dono e
/// permissões do original.
pub async fn write_file(path: PathBuf, contents: String) -> Result<(), String> {
    tokio::fs::write(&path, contents)
        .await
        .map_err(|e| format!("não foi possível salvar {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn lists_sql_files_and_dirs_only() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("sub")).unwrap();
        fs::write(root.path().join("b.sql"), "").unwrap();
        fs::write(root.path().join("A.SQL"), "").unwrap();
        fs::write(root.path().join("notes.txt"), "").unwrap();
        fs::write(root.path().join(".hidden.sql"), "").unwrap();
        let names: Vec<_> = list_dir_sync(root.path(), root.path())
            .unwrap()
            .into_iter()
            .map(|e| e.name)
            .collect();
        assert_eq!(names, ["sub", "A.SQL", "b.sql"]);
    }

    #[test]
    fn refuses_directories_outside_root() {
        let root = tempfile::tempdir().unwrap();
        let inner = root.path().join("inner");
        fs::create_dir(&inner).unwrap();
        assert!(list_dir_sync(&inner, &inner.join("..")).is_err());
        assert!(list_dir_sync(&root.path().join("missing"), root.path()).is_err());
    }
}
