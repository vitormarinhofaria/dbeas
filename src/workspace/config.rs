//! Leitura, validação e resolução do arquivo `dbeas.json` (spec §3).

use std::fmt;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};
use thiserror::Error;

use super::env::{self, EnvMap};
use crate::fsutil;

pub const DEFAULT_HOST: &str = "localhost";
pub const DEFAULT_PORT: u16 = 5432;

/// Erros de configuração. Nenhuma variante carrega valores resolvidos, para
/// que mensagens nunca exponham senhas ou outros segredos.
#[derive(Debug, Clone, Error, PartialEq)]
pub enum ConfigError {
    #[error("não foi possível ler {path}: {reason}")]
    Read { path: PathBuf, reason: String },
    #[error("JSON inválido em {path}: {reason}")]
    Json { path: PathBuf, reason: String },
    #[error("o arquivo deve conter um array JSON de workspaces")]
    NotAnArray,
    #[error("a entrada deve ser um objeto JSON")]
    EntryNotObject,
    #[error("campo obrigatório `{0}` ausente ou vazio")]
    MissingField(&'static str),
    #[error("campo `{field}` deve ser {expected}")]
    WrongType {
        field: &'static str,
        expected: &'static str,
    },
    #[error("campo `{field}` referencia a variável `{variable}`, que não está definida")]
    UnresolvedVariable { field: String, variable: String },
    #[error("campo `{field}`: {reason}")]
    InvalidReference { field: String, reason: String },
    #[error("`dbPort` deve ser um inteiro entre 1 e 65535")]
    InvalidPort,
    #[error("não foi possível ler o env_file {path}: {reason}")]
    EnvFile { path: PathBuf, reason: String },
    #[error("`dbUser` não definido e não foi possível obter o usuário do sistema")]
    MissingUser,
}

/// Senha que não aparece em `Debug`/`Display`.
#[derive(Clone, Default, PartialEq)]
pub struct Secret(Option<String>);

impl Secret {
    pub fn new(value: Option<String>) -> Self {
        Self(value)
    }

    pub fn expose(&self) -> Option<&str> {
        self.0.as_deref()
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            Some(_) => f.write_str("Secret(***)"),
            None => f.write_str("Secret(None)"),
        }
    }
}

/// Configuração resolvida e validada, pronta para conectar.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedConfig {
    pub name: String,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: Secret,
    pub db_name: String,
    pub read_only: bool,
    pub require_transaction: bool,
    /// `None` quando não configurado. A existência do diretório é verificada
    /// pelo explorador de arquivos, que informa o problema na própria área.
    pub sql_files_dir: Option<PathBuf>,
}

/// Uma entrada do array do `dbeas.json`: válida ou com erro. O nome é mantido
/// sempre que possível para identificar a entrada na barra lateral.
#[derive(Debug, Clone)]
pub struct Entry {
    pub index: usize,
    pub name: Option<String>,
    pub result: Result<ResolvedConfig, ConfigError>,
}

/// Carrega um `dbeas.json`. Erros no arquivo inteiro (leitura, JSON) retornam
/// `Err`; erros de uma entrada ficam na própria entrada, sem afetar as demais.
pub fn load_file(path: &Path) -> Result<Vec<Entry>, ConfigError> {
    let text = std::fs::read_to_string(path).map_err(|e| ConfigError::Read {
        path: path.to_path_buf(),
        reason: e.to_string(),
    })?;
    let base_dir = path.parent().unwrap_or_else(|| Path::new("/"));
    parse(&text, path, base_dir, std::env::vars())
}

pub fn parse(
    text: &str,
    path: &Path,
    base_dir: &Path,
    process_env: impl IntoIterator<Item = (String, String)>,
) -> Result<Vec<Entry>, ConfigError> {
    let value: Value = serde_json::from_str(text).map_err(|e| ConfigError::Json {
        path: path.to_path_buf(),
        reason: format!(
            "linha {}, coluna {}: {}",
            e.line(),
            e.column(),
            classify_json_error(&e)
        ),
    })?;
    let Value::Array(items) = value else {
        return Err(ConfigError::NotAnArray);
    };

    let process_env: Vec<(String, String)> = process_env.into_iter().collect();

    Ok(items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let name = item
                .get("name")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string);
            let result = match item {
                Value::Object(obj) => resolve_entry(obj, base_dir, process_env.iter().cloned()),
                _ => Err(ConfigError::EntryNotObject),
            };
            Entry {
                index,
                name,
                result,
            }
        })
        .collect())
}

/// Descrição do erro de JSON sem ecoar o conteúdo do arquivo.
fn classify_json_error(e: &serde_json::Error) -> &'static str {
    use serde_json::error::Category;
    match e.classify() {
        Category::Io => "erro de leitura",
        Category::Syntax => "erro de sintaxe",
        Category::Data => "dados inválidos",
        Category::Eof => "fim inesperado do arquivo",
    }
}

fn resolve_entry(
    obj: &Map<String, Value>,
    base_dir: &Path,
    process_env: impl IntoIterator<Item = (String, String)>,
) -> Result<ResolvedConfig, ConfigError> {
    let name = opt_str(obj, "name")?
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or(ConfigError::MissingField("name"))?
        .to_string();

    let env_file = opt_str(obj, "env_file")?
        .filter(|s| !s.is_empty())
        .map(|p| fsutil::normalize(base_dir, Path::new(p)));

    let overrides = match obj.get("env") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Object(map)) => map
            .iter()
            .map(|(k, v)| match v {
                Value::String(s) => Ok((k.clone(), s.clone())),
                _ => Err(ConfigError::WrongType {
                    field: "env",
                    expected: "um objeto de strings",
                }),
            })
            .collect::<Result<_, _>>()?,
        Some(_) => {
            return Err(ConfigError::WrongType {
                field: "env",
                expected: "um objeto de strings",
            });
        }
    };

    let env = env::build_env(process_env, env_file.as_deref(), &overrides)?;

    let host = resolve_str(obj, "dbHost", &env)?
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_HOST.to_string());

    let port = resolve_port(obj, &env)?;

    let user = match resolve_str(obj, "dbUser", &env)?.filter(|s| !s.is_empty()) {
        Some(user) => user,
        // Mesmo comportamento do libpq: sem usuário, usa o do sistema.
        None => env
            .get("USER")
            .cloned()
            .filter(|s| !s.is_empty())
            .ok_or(ConfigError::MissingUser)?,
    };

    let password = Secret(resolve_str(obj, "dbPass", &env)?.filter(|s| !s.is_empty()));

    let db_name = resolve_str(obj, "dbName", &env)?
        .filter(|s| !s.is_empty())
        .ok_or(ConfigError::MissingField("dbName"))?;

    let read_only = opt_bool(obj, "readOnly")?.unwrap_or(false);
    let require_transaction = opt_bool(obj, "requireTransaction")?.unwrap_or(false);

    let sql_files_dir = opt_str(obj, "sqlFilesDir")?
        .filter(|s| !s.is_empty())
        .map(|p| fsutil::normalize(base_dir, Path::new(p)));

    Ok(ResolvedConfig {
        name,
        host,
        port,
        user,
        password,
        db_name,
        read_only,
        require_transaction,
        sql_files_dir,
    })
}

fn opt_str<'a>(
    obj: &'a Map<String, Value>,
    field: &'static str,
) -> Result<Option<&'a str>, ConfigError> {
    match obj.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s)),
        Some(_) => Err(ConfigError::WrongType {
            field,
            expected: "uma string",
        }),
    }
}

fn opt_bool(obj: &Map<String, Value>, field: &'static str) -> Result<Option<bool>, ConfigError> {
    match obj.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(b)) => Ok(Some(*b)),
        Some(_) => Err(ConfigError::WrongType {
            field,
            expected: "um booleano JSON (true/false)",
        }),
    }
}

fn resolve_str(
    obj: &Map<String, Value>,
    field: &'static str,
    env: &EnvMap,
) -> Result<Option<String>, ConfigError> {
    opt_str(obj, field)?
        .map(|raw| env::interpolate(field, raw, env))
        .transpose()
}

fn resolve_port(obj: &Map<String, Value>, env: &EnvMap) -> Result<u16, ConfigError> {
    let text = match obj.get("dbPort") {
        None | Some(Value::Null) => return Ok(DEFAULT_PORT),
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::String(s)) => env::interpolate("dbPort", s, env)?,
        Some(_) => {
            return Err(ConfigError::WrongType {
                field: "dbPort",
                expected: "um número ou string numérica",
            });
        }
    };
    let text = text.trim();
    if text.is_empty() {
        return Ok(DEFAULT_PORT);
    }
    match text.parse::<u16>() {
        Ok(port) if port >= 1 => Ok(port),
        _ => Err(ConfigError::InvalidPort),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_one(json: &str, env: &[(&str, &str)]) -> Result<ResolvedConfig, ConfigError> {
        let env: Vec<(String, String)> = env
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        let entries = parse(json, Path::new("/p/dbeas.json"), Path::new("/p"), env).unwrap();
        assert_eq!(entries.len(), 1);
        entries.into_iter().next().unwrap().result
    }

    #[test]
    fn defaults_for_host_and_port() {
        let c = parse_one(r#"[{"name":"a","dbName":"db","dbUser":"u"}]"#, &[]).unwrap();
        assert_eq!(c.host, "localhost");
        assert_eq!(c.port, 5432);
        assert!(!c.read_only);
        assert!(!c.require_transaction);
        assert_eq!(c.password.expose(), None);
        assert_eq!(c.sql_files_dir, None);

        let c = parse_one(
            r#"[{"name":"a","dbName":"db","dbUser":"u","dbHost":"","dbPort":""}]"#,
            &[],
        )
        .unwrap();
        assert_eq!(c.host, "localhost");
        assert_eq!(c.port, 5432);
    }

    #[test]
    fn name_and_db_name_are_required() {
        assert_eq!(
            parse_one(r#"[{"dbName":"db","dbUser":"u"}]"#, &[]).unwrap_err(),
            ConfigError::MissingField("name")
        );
        assert_eq!(
            parse_one(r#"[{"name":"a","dbName":"","dbUser":"u"}]"#, &[]).unwrap_err(),
            ConfigError::MissingField("dbName")
        );
    }

    #[test]
    fn resolves_env_references_with_object_precedence() {
        let c = parse_one(
            r#"[{"name":"a","env":{"H":"obj"},"dbHost":"${env.H}","dbPort":"${env.P}","dbUser":"${env.U}","dbPass":"${env.S}","dbName":"x"}]"#,
            &[("H", "proc"), ("P", "6543"), ("U", "me"), ("S", "pw")],
        )
        .unwrap();
        assert_eq!(c.host, "obj");
        assert_eq!(c.port, 6543);
        assert_eq!(c.user, "me");
        assert_eq!(c.password.expose(), Some("pw"));
    }

    #[test]
    fn port_validation() {
        assert_eq!(
            parse_one(
                r#"[{"name":"a","dbName":"x","dbUser":"u","dbPort":5433}]"#,
                &[]
            )
            .unwrap()
            .port,
            5433
        );
        for bad in ["0", "70000", "\"abc\"", "-1", "1.5"] {
            let json = format!(r#"[{{"name":"a","dbName":"x","dbUser":"u","dbPort":{bad}}}]"#);
            assert_eq!(
                parse_one(&json, &[]).unwrap_err(),
                ConfigError::InvalidPort,
                "porta {bad}"
            );
        }
    }

    #[test]
    fn booleans_must_be_json_booleans() {
        let err = parse_one(
            r#"[{"name":"a","dbName":"x","dbUser":"u","readOnly":"true"}]"#,
            &[],
        )
        .unwrap_err();
        assert!(matches!(
            err,
            ConfigError::WrongType {
                field: "readOnly",
                ..
            }
        ));
        let c = parse_one(
            r#"[{"name":"a","dbName":"x","dbUser":"u","readOnly":true,"requireTransaction":true}]"#,
            &[],
        )
        .unwrap();
        assert!(c.read_only && c.require_transaction);
    }

    #[test]
    fn relative_paths_resolve_from_config_dir() {
        let c = parse_one(
            r#"[{"name":"a","dbName":"x","dbUser":"u","sqlFilesDir":"./sql/../queries"}]"#,
            &[],
        )
        .unwrap();
        assert_eq!(c.sql_files_dir, Some(PathBuf::from("/p/queries")));
    }

    #[test]
    fn user_falls_back_to_system_user() {
        let c = parse_one(r#"[{"name":"a","dbName":"x"}]"#, &[("USER", "alice")]).unwrap();
        assert_eq!(c.user, "alice");
        assert_eq!(
            parse_one(r#"[{"name":"a","dbName":"x"}]"#, &[]).unwrap_err(),
            ConfigError::MissingUser
        );
    }

    #[test]
    fn error_messages_never_contain_password() {
        let err = parse_one(
            r#"[{"name":"a","dbName":"${env.NOPE}","dbUser":"u","dbPass":"hunter2"}]"#,
            &[],
        )
        .unwrap_err();
        assert!(!err.to_string().contains("hunter2"));
        let c = parse_one(
            r#"[{"name":"a","dbName":"x","dbUser":"u","dbPass":"hunter2"}]"#,
            &[],
        )
        .unwrap();
        assert!(!format!("{c:?}").contains("hunter2"));
    }

    #[test]
    fn one_bad_entry_does_not_affect_others() {
        let entries = parse(
            r#"[{"name":"ok","dbName":"x","dbUser":"u"}, {"name":"bad"}, 42]"#,
            Path::new("/p/dbeas.json"),
            Path::new("/p"),
            Vec::new(),
        )
        .unwrap();
        assert!(entries[0].result.is_ok());
        assert_eq!(entries[1].name.as_deref(), Some("bad"));
        assert!(entries[1].result.is_err());
        assert_eq!(entries[2].result, Err(ConfigError::EntryNotObject));
    }

    #[test]
    fn file_level_errors() {
        assert!(matches!(
            parse("{", Path::new("/p/dbeas.json"), Path::new("/p"), Vec::new()),
            Err(ConfigError::Json { .. })
        ));
        assert_eq!(
            parse(
                "{}",
                Path::new("/p/dbeas.json"),
                Path::new("/p"),
                Vec::new()
            )
            .unwrap_err(),
            ConfigError::NotAnArray
        );
    }
}
