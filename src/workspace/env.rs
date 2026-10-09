//! Montagem do ambiente de resolução e interpolação de `${env.NOME}` (spec §3.3).

use std::collections::HashMap;
use std::path::Path;

use super::config::ConfigError;

/// Ambiente resultante da precedência: processo < `env_file` < `env`.
pub type EnvMap = HashMap<String, String>;

pub fn build_env(
    process: impl IntoIterator<Item = (String, String)>,
    env_file: Option<&Path>,
    overrides: &[(String, String)],
) -> Result<EnvMap, ConfigError> {
    let mut env: EnvMap = process.into_iter().collect();

    if let Some(path) = env_file {
        let iter = dotenvy::from_path_iter(path).map_err(|e| ConfigError::EnvFile {
            path: path.to_path_buf(),
            reason: describe_dotenv_error(&e),
        })?;
        for item in iter {
            let (key, value) = item.map_err(|e| ConfigError::EnvFile {
                path: path.to_path_buf(),
                reason: describe_dotenv_error(&e),
            })?;
            env.insert(key, value);
        }
    }

    for (key, value) in overrides {
        env.insert(key.clone(), value.clone());
    }

    Ok(env)
}

/// Mensagem de erro do `.env` sem repetir o conteúdo da linha, que pode conter
/// segredos.
fn describe_dotenv_error(error: &dotenvy::Error) -> String {
    match error {
        dotenvy::Error::LineParse(_, index) => format!("linha malformada (posição {index})"),
        dotenvy::Error::Io(io) => io.to_string(),
        dotenvy::Error::EnvVar(_) => "variável inválida".to_string(),
        _ => "erro de leitura".to_string(),
    }
}

/// Substitui as referências `${env.NOME}` em `value`. Não há nenhuma outra
/// sintaxe de expansão: `$NOME`, `${NOME}` e afins permanecem literais.
pub fn interpolate(field: &str, value: &str, env: &EnvMap) -> Result<String, ConfigError> {
    const PREFIX: &str = "${env.";

    let mut out = String::with_capacity(value.len());
    let mut rest = value;

    while let Some(start) = rest.find(PREFIX) {
        out.push_str(&rest[..start]);
        let after = &rest[start + PREFIX.len()..];
        let end = after
            .find('}')
            .ok_or_else(|| ConfigError::InvalidReference {
                field: field.to_string(),
                reason: "referência `${env.` sem `}` de fechamento".to_string(),
            })?;
        let name = &after[..end];
        if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Err(ConfigError::InvalidReference {
                field: field.to_string(),
                reason: format!("nome de variável inválido: `{name}`"),
            });
        }
        let resolved = env
            .get(name)
            .ok_or_else(|| ConfigError::UnresolvedVariable {
                field: field.to_string(),
                variable: name.to_string(),
            })?;
        out.push_str(resolved);
        rest = &after[end + 1..];
    }

    out.push_str(rest);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn env(pairs: &[(&str, &str)]) -> EnvMap {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn interpolates_references() {
        let e = env(&[("HOST", "db"), ("PORT", "5433")]);
        assert_eq!(interpolate("dbHost", "${env.HOST}", &e).unwrap(), "db");
        assert_eq!(
            interpolate("x", "a-${env.HOST}:${env.PORT}-b", &e).unwrap(),
            "a-db:5433-b"
        );
        assert_eq!(
            interpolate("x", "$HOST ${HOST}", &e).unwrap(),
            "$HOST ${HOST}"
        );
    }

    #[test]
    fn unresolved_reference_names_field_and_variable() {
        let err = interpolate("dbPass", "${env.SECRET}", &EnvMap::new()).unwrap_err();
        match err {
            ConfigError::UnresolvedVariable { field, variable } => {
                assert_eq!(field, "dbPass");
                assert_eq!(variable, "SECRET");
            }
            other => panic!("erro inesperado: {other:?}"),
        }
    }

    #[test]
    fn rejects_malformed_references() {
        assert!(interpolate("x", "${env.A", &EnvMap::new()).is_err());
        assert!(interpolate("x", "${env.}", &EnvMap::new()).is_err());
        assert!(interpolate("x", "${env.$(rm)}", &EnvMap::new()).is_err());
    }

    #[test]
    fn precedence_process_then_file_then_env_object() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(".env");
        let mut f = std::fs::File::create(&file).unwrap();
        writeln!(f, "A=file\nB=file").unwrap();

        let process = vec![
            ("A".to_string(), "proc".to_string()),
            ("B".to_string(), "proc".to_string()),
            ("C".to_string(), "proc".to_string()),
        ];
        let overrides = vec![("B".to_string(), "obj".to_string())];
        let e = build_env(process, Some(&file), &overrides).unwrap();
        assert_eq!(e["A"], "file");
        assert_eq!(e["B"], "obj");
        assert_eq!(e["C"], "proc");
    }

    #[test]
    fn missing_env_file_is_an_error() {
        let err = build_env(Vec::new(), Some(Path::new("/nao/existe/.env")), &[]).unwrap_err();
        assert!(matches!(err, ConfigError::EnvFile { .. }));
    }
}
