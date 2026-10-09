// SPDX-FileCopyrightText: 2026 Vitor Marinho Faria
// SPDX-License-Identifier: GPL-3.0-or-later

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

/// Caminho do novo arquivo `name`, relativo a `dir` (o diretório exibido),
/// que precisa estar dentro de `root` (`sqlFilesDir`). Acrescenta `.sql` se
/// faltar; recusa caminho absoluto, saída da raiz (por `..` ou link simbólico),
/// diretório inexistente e arquivo já existente (spec §8).
pub fn resolve_new_file(root: &Path, dir: &Path, name: &str) -> Result<PathBuf, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("informe o nome do arquivo".into());
    }
    if name.ends_with('/') || name.ends_with(std::path::MAIN_SEPARATOR) {
        return Err("o nome termina em separador de diretório; informe um arquivo".into());
    }
    if Path::new(name).is_absolute() {
        return Err("use um nome relativo ao diretório de arquivos SQL".into());
    }
    let root = std::fs::canonicalize(root)
        .map_err(|e| format!("o diretório de arquivos SQL não está acessível ({e})"))?;
    let dir = resolve_within(&root, dir).ok_or("o diretório atual está fora do sqlFilesDir")?;

    let mut file = crate::fsutil::normalize(&dir, Path::new(name));
    if !is_sql(&file) {
        let mut with_extension = file.into_os_string();
        with_extension.push(".sql");
        file = PathBuf::from(with_extension);
    }
    if !file.starts_with(&root) {
        return Err("o arquivo ficaria fora do diretório de arquivos SQL".into());
    }

    // O diretório de destino precisa existir e, resolvidos os links, continuar
    // dentro da raiz.
    let parent = file.parent().ok_or("caminho inválido")?;
    let parent = std::fs::canonicalize(parent)
        .map_err(|_| format!("o diretório {} não existe", parent.display()))?;
    if !parent.starts_with(&root) {
        return Err("o arquivo ficaria fora do diretório de arquivos SQL".into());
    }
    let file = parent.join(file.file_name().ok_or("caminho inválido")?);
    if file.symlink_metadata().is_ok() {
        return Err(format!(
            "já existe um arquivo chamado {}; escolha outro nome",
            file.file_name().unwrap_or_default().to_string_lossy()
        ));
    }
    Ok(file)
}

/// Cria um arquivo SQL vazio sem sobrescrever nada: `create_new` falha se o
/// caminho já existir, mesmo que ele apareça entre a validação e a criação.
pub async fn create_file(root: PathBuf, dir: PathBuf, name: String) -> Result<PathBuf, String> {
    tokio::task::spawn_blocking(move || {
        let path = resolve_new_file(&root, &dir, &name)?;
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|e| match e.kind() {
                std::io::ErrorKind::AlreadyExists => {
                    "já existe um arquivo com esse nome; escolha outro".to_string()
                }
                _ => format!("não foi possível criar {}: {e}", path.display()),
            })?;
        Ok(path)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Item (arquivo ou diretório) dentro de `root`, mas não a própria raiz.
fn managed_item(root: &Path, path: &Path) -> Result<(PathBuf, PathBuf), String> {
    let root = std::fs::canonicalize(root)
        .map_err(|e| format!("o diretório de arquivos SQL não está acessível ({e})"))?;
    // Canonicaliza o diretório pai para não seguir um link no próprio item.
    let name = path.file_name().ok_or("caminho inválido")?;
    let parent = path.parent().ok_or("caminho inválido")?;
    let parent =
        resolve_within(&root, parent).ok_or("o item está fora do diretório de arquivos SQL")?;
    let item = parent.join(name);
    if item == root {
        return Err("o diretório de arquivos SQL não pode ser alterado".into());
    }
    if item.symlink_metadata().is_err() {
        return Err(format!("{} não existe mais", item.display()));
    }
    Ok((root, item))
}

/// Destino de renomear/mover `path` para `new_path`, um caminho **relativo
/// ao `sqlFilesDir`** (ex.: `relatorios/mensal.sql`). Permite mover para
/// qualquer diretório existente abaixo da raiz, mas recusa `..` (mesmo que o
/// caminho voltasse para dentro dela), caminho absoluto, saída da raiz por link
/// simbólico, diretório de destino inexistente, mover um diretório para dentro
/// dele mesmo e destino já existente. Arquivos mantêm `.sql` (acrescentado se
/// faltar).
pub fn resolve_rename(root: &Path, path: &Path, new_path: &str) -> Result<PathBuf, String> {
    use std::path::Component;

    let (root, item) = managed_item(root, path)?;
    let new_path = new_path
        .trim()
        .trim_end_matches(['/', std::path::MAIN_SEPARATOR]);
    if new_path.is_empty() {
        return Err("informe o novo nome".into());
    }
    let relative = Path::new(new_path);
    if relative.is_absolute() {
        return Err("use um caminho relativo ao diretório de arquivos SQL".into());
    }
    if relative
        .components()
        .any(|c| matches!(c, Component::ParentDir))
    {
        return Err("caminhos com .. não são permitidos; só é possível mover para diretórios abaixo do diretório de arquivos SQL".into());
    }
    let mut target = crate::fsutil::normalize(&root, relative);
    if target == root {
        return Err("informe um nome de arquivo ou diretório".into());
    }

    let is_dir = item.symlink_metadata().map(|m| m.is_dir()).unwrap_or(false);
    if !is_dir && !is_sql(&target) {
        let mut with_extension = target.into_os_string();
        with_extension.push(".sql");
        target = PathBuf::from(with_extension);
    }

    // Diretório de destino: precisa existir e, resolvidos os links, continuar
    // dentro da raiz.
    let parent = target.parent().ok_or("caminho inválido")?;
    let parent = std::fs::canonicalize(parent)
        .map_err(|_| format!("o diretório {} não existe", parent.display()))?;
    if !parent.starts_with(&root) {
        return Err("o destino ficaria fora do diretório de arquivos SQL".into());
    }
    let target = parent.join(target.file_name().ok_or("caminho inválido")?);

    if target == item {
        return Err("o nome é o mesmo".into());
    }
    if is_dir && target.starts_with(&item) {
        return Err("não é possível mover um diretório para dentro dele mesmo".into());
    }
    if target.symlink_metadata().is_ok() {
        let shown = target
            .strip_prefix(&root)
            .unwrap_or(&target)
            .display()
            .to_string();
        return Err(format!("já existe um item em {shown}; escolha outro nome"));
    }
    Ok(target)
}

/// Caminho de um item relativo ao `sqlFilesDir`, como é digitado ao renomear.
pub fn relative_to_root(root: &Path, path: &Path) -> String {
    let root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    path.strip_prefix(&root)
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| {
            path.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        })
}

pub async fn rename(root: PathBuf, path: PathBuf, new_name: String) -> Result<PathBuf, String> {
    tokio::task::spawn_blocking(move || {
        let target = resolve_rename(&root, &path, &new_name)?;
        let (_, item) = managed_item(&root, &path)?;
        std::fs::rename(&item, &target).map_err(|e| format!("não foi possível renomear: {e}"))?;
        Ok(target)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Quantidade de arquivos `.sql` dentro de um diretório (recursivo), para a
/// confirmação de exclusão.
pub fn count_sql_files(dir: &Path) -> usize {
    let mut count = 0;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(read) = std::fs::read_dir(&d) else {
            continue;
        };
        for entry in read.filter_map(Result::ok) {
            let Ok(meta) = entry.path().symlink_metadata() else {
                continue;
            };
            if meta.is_dir() {
                stack.push(entry.path());
            } else if is_sql(&entry.path()) {
                count += 1;
            }
        }
    }
    count
}

/// Move o arquivo ou diretório para a lixeira do sistema (recuperável).
pub async fn move_to_trash(root: PathBuf, path: PathBuf) -> Result<PathBuf, String> {
    tokio::task::spawn_blocking(move || {
        let (_, item) = managed_item(&root, &path)?;
        trash::delete(&item).map_err(|e| format!("não foi possível mover para a lixeira: {e}"))?;
        Ok(item)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn rename_rules() {
        let root = tempfile::tempdir().unwrap();
        let r = fs::canonicalize(root.path()).unwrap();
        fs::create_dir_all(r.join("dir/sub")).unwrap();
        fs::write(r.join("a.sql"), "").unwrap();
        fs::write(r.join("b.sql"), "").unwrap();
        fs::write(r.join("dir/c.sql"), "").unwrap();

        // Renomear no lugar (caminho relativo à raiz).
        assert_eq!(
            resolve_rename(&r, &r.join("a.sql"), "c").unwrap(),
            r.join("c.sql")
        );
        assert_eq!(
            resolve_rename(&r, &r.join("a.sql"), "c.SQL").unwrap(),
            r.join("c.SQL")
        );
        assert_eq!(
            resolve_rename(&r, &r.join("dir"), "outro").unwrap(),
            r.join("outro")
        );
        assert_eq!(
            resolve_rename(&r, &r.join("dir/c.sql"), "dir/d").unwrap(),
            r.join("dir/d.sql")
        );
        // Mover para diretórios abaixo da raiz, inclusive de volta à raiz.
        assert_eq!(
            resolve_rename(&r, &r.join("a.sql"), "dir/sub/a").unwrap(),
            r.join("dir/sub/a.sql")
        );
        assert_eq!(
            resolve_rename(&r, &r.join("dir/c.sql"), "c").unwrap(),
            r.join("c.sql")
        );
        assert_eq!(
            resolve_rename(&r, &r.join("dir/c.sql"), "./x.sql").unwrap(),
            r.join("x.sql")
        );

        // Recusas.
        let err = |p: &str, to: &str| resolve_rename(&r, &r.join(p), to).unwrap_err();
        assert!(err("a.sql", "../fora").contains(".."));
        assert!(err("a.sql", "dir/../b2").contains(".."));
        assert!(err("a.sql", "/tmp/x").contains("relativo"));
        assert!(err("a.sql", "nao/existe/x").contains("não existe"));
        assert!(err("a.sql", "b").contains("já existe"));
        assert!(err("a.sql", "a").contains("mesmo"));
        assert!(err("a.sql", " ").contains("informe"));
        assert!(err("a.sql", ".").contains("informe"));
        assert!(err("dir", "dir/sub/dir").contains("dentro dele mesmo"));
        assert!(resolve_rename(&r, &r, "x").is_err());
        assert!(err("sumiu.sql", "x").contains("não existe"));
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("f.sql"), "").unwrap();
        assert!(resolve_rename(&r, &outside.path().join("f.sql"), "x").is_err());
        std::os::unix::fs::symlink(outside.path(), r.join("link")).unwrap();
        assert!(err("a.sql", "link/a").contains("fora"));
    }

    #[test]
    fn relative_paths_for_the_rename_field() {
        let root = tempfile::tempdir().unwrap();
        let r = fs::canonicalize(root.path()).unwrap();
        assert_eq!(relative_to_root(&r, &r.join("dir/a.sql")), "dir/a.sql");
    }

    #[tokio::test]
    async fn renames_on_disk() {
        let root = tempfile::tempdir().unwrap();
        let r = fs::canonicalize(root.path()).unwrap();
        fs::write(r.join("a.sql"), "x").unwrap();
        let target = rename(r.clone(), r.join("a.sql"), "b".into())
            .await
            .unwrap();
        assert_eq!(fs::read_to_string(target).unwrap(), "x");
        assert!(!r.join("a.sql").exists());
    }

    #[test]
    fn counts_sql_files_recursively() {
        let root = tempfile::tempdir().unwrap();
        let r = root.path();
        fs::create_dir_all(r.join("x/y")).unwrap();
        fs::write(r.join("x/a.sql"), "").unwrap();
        fs::write(r.join("x/y/b.sql"), "").unwrap();
        fs::write(r.join("x/notas.txt"), "").unwrap();
        assert_eq!(count_sql_files(&r.join("x")), 2);
    }

    #[test]
    fn trash_refuses_root_and_outside_items() {
        let root = tempfile::tempdir().unwrap();
        let r = fs::canonicalize(root.path()).unwrap();
        assert!(managed_item(&r, &r).is_err());
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("f.sql"), "").unwrap();
        assert!(managed_item(&r, &outside.path().join("f.sql")).is_err());
        // Um link dentro da raiz é tratado como item próprio (não segue o destino).
        std::os::unix::fs::symlink(outside.path(), r.join("link")).unwrap();
        assert_eq!(managed_item(&r, &r.join("link")).unwrap().1, r.join("link"));
    }

    #[test]
    fn new_file_names_resolve_inside_root() {
        let root = tempfile::tempdir().unwrap();
        let r = fs::canonicalize(root.path()).unwrap();
        fs::create_dir(r.join("sub")).unwrap();
        fs::write(r.join("existe.sql"), "").unwrap();

        assert_eq!(
            resolve_new_file(&r, &r, "consulta").unwrap(),
            r.join("consulta.sql")
        );
        assert_eq!(
            resolve_new_file(&r, &r, " a.SQL ").unwrap(),
            r.join("a.SQL")
        );
        assert_eq!(
            resolve_new_file(&r, &r, "notas.txt").unwrap(),
            r.join("notas.txt.sql")
        );
        assert_eq!(
            resolve_new_file(&r, &r.join("sub"), "x").unwrap(),
            r.join("sub").join("x.sql")
        );
        assert_eq!(
            resolve_new_file(&r, &r, "sub/y").unwrap(),
            r.join("sub").join("y.sql")
        );
        assert_eq!(
            resolve_new_file(&r, &r.join("sub"), "../z").unwrap(),
            r.join("z.sql")
        );
    }

    #[test]
    fn new_file_rejects_escapes_conflicts_and_bad_names() {
        let root = tempfile::tempdir().unwrap();
        let r = fs::canonicalize(root.path()).unwrap();
        fs::write(r.join("existe.sql"), "").unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), r.join("link")).unwrap();

        assert!(resolve_new_file(&r, &r, "").is_err());
        assert!(resolve_new_file(&r, &r, "   ").is_err());
        assert!(resolve_new_file(&r, &r, "dir/").is_err());
        assert!(resolve_new_file(&r, &r, "/tmp/x").is_err());
        assert!(
            resolve_new_file(&r, &r, "../fora")
                .unwrap_err()
                .contains("fora do diretório")
        );
        assert!(
            resolve_new_file(&r, &r, "link/x")
                .unwrap_err()
                .contains("fora do diretório")
        );
        assert!(
            resolve_new_file(&r, &r, "nao/existe/x")
                .unwrap_err()
                .contains("não existe")
        );
        assert!(
            resolve_new_file(&r, &r, "existe")
                .unwrap_err()
                .contains("já existe")
        );
        assert!(resolve_new_file(&r, &r, "EXISTE.sql").is_ok() || cfg!(target_os = "macos"));
    }

    #[tokio::test]
    async fn create_file_never_overwrites() {
        let root = tempfile::tempdir().unwrap();
        let r = fs::canonicalize(root.path()).unwrap();
        let created = create_file(r.clone(), r.clone(), "novo".into())
            .await
            .unwrap();
        assert_eq!(created, r.join("novo.sql"));
        assert_eq!(fs::read_to_string(&created).unwrap(), "");
        fs::write(&created, "conteúdo").unwrap();
        assert!(
            create_file(r.clone(), r.clone(), "novo".into())
                .await
                .is_err()
        );
        assert_eq!(fs::read_to_string(&created).unwrap(), "conteúdo");
    }

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
