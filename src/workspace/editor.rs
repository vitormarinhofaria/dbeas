// SPDX-FileCopyrightText: 2026 Vitor Marinho Faria
// SPDX-License-Identifier: GPL-3.0-or-later

//! Edição gráfica de uma entrada do `dbeas.json`.
//!
//! O formulário guarda o texto cru de cada campo (com as referências
//! `${env.NOME}` intactas) e só reescreve no arquivo o que o usuário alterou:
//! campos intocados mantêm o valor e o tipo JSON originais, e chaves que o
//! DBeas não conhece são preservadas na posição em que estavam.

use std::fmt;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use super::config::{self, SslMode};
use super::env::{self, EnvMap};
use crate::fsutil;

/// Campos de texto do formulário, na ordem em que aparecem.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Name,
    EnvFile,
    DbHost,
    DbPort,
    DbUser,
    DbPass,
    DbName,
    SqlFilesDir,
    SslMode,
    SslRootCert,
}

impl Field {
    const ALL: [Field; 10] = [
        Field::Name,
        Field::EnvFile,
        Field::DbHost,
        Field::DbPort,
        Field::DbUser,
        Field::DbPass,
        Field::DbName,
        Field::SqlFilesDir,
        Field::SslMode,
        Field::SslRootCert,
    ];

    /// Nome da chave no `dbeas.json`.
    pub fn key(self) -> &'static str {
        match self {
            Field::Name => "name",
            Field::EnvFile => "envFile",
            Field::DbHost => "dbHost",
            Field::DbPort => "dbPort",
            Field::DbUser => "dbUser",
            Field::DbPass => "dbPass",
            Field::DbName => "dbName",
            Field::SqlFilesDir => "sqlFilesDir",
            Field::SslMode => "sslMode",
            Field::SslRootCert => "sslRootCert",
        }
    }

    /// Campos em que `${env.NOME}` é expandido (spec §3.2).
    pub fn interpolated(self) -> bool {
        matches!(
            self,
            Field::DbHost
                | Field::DbPort
                | Field::DbUser
                | Field::DbPass
                | Field::DbName
                | Field::SslMode
                | Field::SslRootCert
        )
    }

    fn slot(self) -> usize {
        self as usize
    }
}

/// Valor resolvido de um campo com `${env.NOME}`, para exibição.
#[derive(Debug, Clone, PartialEq)]
pub enum Preview {
    Value(String),
    /// Senha resolvida e não vazia: só se indica que existe valor.
    Masked,
    Error(String),
}

/// Opção do seletor de `sslMode`. A string vazia é "não informado" (padrão).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SslChoice(pub String);

impl fmt::Display for SslChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0.is_empty() {
            write!(f, "padrão ({})", SslMode::default().as_str())
        } else {
            f.write_str(&self.0)
        }
    }
}

pub struct Form {
    pub file: PathBuf,
    /// Posição da entrada no array do arquivo.
    pub index: usize,
    base_dir: PathBuf,
    original_text: String,
    original: Map<String, Value>,
    initial: [String; 10],
    values: [String; 10],
    initial_env: Vec<(String, String)>,
    pub env: Vec<(String, String)>,
    initial_bools: [bool; 2],
    pub read_only: bool,
    pub require_transaction: bool,
    process_env: Vec<(String, String)>,
    /// Variáveis do `envFile` informado no formulário (vazio quando não há).
    file_vars: Result<Vec<(String, String)>, String>,
    env_map: EnvMap,
    validation: Result<(), String>,
}

impl Form {
    /// Abre a entrada `index` de `file` com o ambiente do processo.
    pub fn open(file: &Path, index: usize) -> Result<Self, String> {
        let text = std::fs::read_to_string(file)
            .map_err(|e| format!("não foi possível ler {}: {e}", file.display()))?;
        Self::from_text(text, file, index, std::env::vars().collect())
    }

    pub fn from_text(
        text: String,
        file: &Path,
        index: usize,
        process_env: Vec<(String, String)>,
    ) -> Result<Self, String> {
        let items = match serde_json::from_str::<Value>(&text) {
            Ok(Value::Array(items)) => items,
            Ok(_) => return Err("o arquivo não contém um array JSON de workspaces".into()),
            Err(e) => {
                return Err(format!(
                    "JSON inválido em {} (linha {}, coluna {}); corrija o arquivo num editor de texto",
                    file.display(),
                    e.line(),
                    e.column()
                ));
            }
        };
        let original = match items.into_iter().nth(index) {
            Some(Value::Object(map)) => map,
            Some(_) => return Err("a entrada não é um objeto JSON".into()),
            None => return Err("a entrada não existe mais no arquivo".into()),
        };

        let mut initial: [String; 10] = Default::default();
        for field in Field::ALL {
            initial[field.slot()] = original.get(field.key()).map(text_of).unwrap_or_default();
        }
        // Nome antigo do campo: o editor grava com o nome novo.
        if initial[Field::EnvFile.slot()].is_empty() {
            if let Some(old) = original.get("env_file") {
                initial[Field::EnvFile.slot()] = text_of(old);
            }
        }
        let initial_env = match original.get("env") {
            Some(Value::Object(map)) => map.iter().map(|(k, v)| (k.clone(), text_of(v))).collect(),
            _ => Vec::new(),
        };
        let bool_of = |key| original.get(key).and_then(Value::as_bool).unwrap_or(false);
        let initial_bools = [bool_of("readOnly"), bool_of("requireTransaction")];

        let mut form = Self {
            file: file.to_path_buf(),
            index,
            base_dir: file
                .parent()
                .unwrap_or_else(|| Path::new("/"))
                .to_path_buf(),
            original_text: text,
            values: initial.clone(),
            initial,
            env: initial_env.clone(),
            initial_env,
            read_only: initial_bools[0],
            require_transaction: initial_bools[1],
            initial_bools,
            original,
            process_env,
            file_vars: Ok(Vec::new()),
            env_map: EnvMap::new(),
            validation: Ok(()),
        };
        form.reload_env_file();
        Ok(form)
    }

    pub fn value(&self, field: Field) -> &str {
        &self.values[field.slot()]
    }

    pub fn set(&mut self, field: Field, value: String) {
        self.values[field.slot()] = value;
        if field == Field::EnvFile {
            self.reload_env_file();
        } else {
            self.refresh();
        }
    }

    pub fn set_env_key(&mut self, row: usize, key: String) {
        if let Some(r) = self.env.get_mut(row) {
            r.0 = key;
            self.refresh();
        }
    }

    pub fn set_env_value(&mut self, row: usize, value: String) {
        if let Some(r) = self.env.get_mut(row) {
            r.1 = value;
            self.refresh();
        }
    }

    pub fn add_env(&mut self) {
        self.env.push((String::new(), String::new()));
    }

    pub fn remove_env(&mut self, row: usize) {
        if row < self.env.len() {
            self.env.remove(row);
            self.refresh();
        }
    }

    pub fn set_read_only(&mut self, value: bool) {
        self.read_only = value;
        self.refresh();
    }

    pub fn set_require_transaction(&mut self, value: bool) {
        self.require_transaction = value;
        self.refresh();
    }

    /// Opções do seletor de `sslMode`: padrão, os modos válidos e o valor
    /// atual quando ele não é um deles (ex.: `${env.SSL_MODE}`), para não ser
    /// perdido.
    pub fn ssl_choices(&self) -> Vec<SslChoice> {
        let mut out = vec![SslChoice(String::new())];
        out.extend(
            SslMode::ALL
                .iter()
                .map(|m| SslChoice(m.as_str().to_string())),
        );
        let current = self.value(Field::SslMode);
        if !out.iter().any(|c| c.0 == current) {
            out.push(SslChoice(current.to_string()));
        }
        out
    }

    pub fn ssl_choice(&self) -> SslChoice {
        SslChoice(self.value(Field::SslMode).to_string())
    }

    /// Valor resolvido de um campo que usa `${env.NOME}`. `None` quando o
    /// campo não usa expansão — ou, na senha, quando ela resolve para vazio.
    pub fn preview(&self, field: Field) -> Option<Preview> {
        let raw = self.value(field);
        if !field.interpolated() || !raw.contains("${env.") {
            return None;
        }
        if let Err(e) = &self.file_vars {
            return Some(Preview::Error(e.clone()));
        }
        match env::interpolate(field.key(), raw, &self.env_map) {
            Ok(value) if field == Field::DbPass => (!value.is_empty()).then_some(Preview::Masked),
            Ok(value) => Some(Preview::Value(value)),
            Err(e) => Some(Preview::Error(e.to_string())),
        }
    }

    /// Resultado da validação completa da entrada como ficará gravada.
    pub fn validation(&self) -> Result<(), &str> {
        self.validation.as_ref().map(|_| ()).map_err(String::as_str)
    }

    pub fn is_dirty(&self) -> bool {
        self.values != self.initial
            || self.env != self.initial_env
            || [self.read_only, self.require_transaction] != self.initial_bools
    }

    /// Erros que impedem gravar. Os demais (variável ausente, envFile
    /// inexistente...) só são avisados: podem depender da máquina.
    pub fn check(&self) -> Result<(), String> {
        for field in [Field::Name, Field::DbName] {
            if self.value(field).trim().is_empty() {
                return Err(format!("`{}` é obrigatório.", field.key()));
            }
        }
        let port = self.value(Field::DbPort).trim();
        if !port.is_empty()
            && !port.contains("${env.")
            && !port.parse::<u16>().is_ok_and(|p| p >= 1)
        {
            return Err("`dbPort` deve ser um inteiro entre 1 e 65535.".into());
        }
        let mut seen = std::collections::HashSet::new();
        for (key, _) in &self.env {
            let key = key.trim();
            if key.is_empty() {
                continue;
            }
            if !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                return Err(format!(
                    "nome de variável inválido em `env`: `{key}` (use letras, dígitos e _)."
                ));
            }
            if !seen.insert(key) {
                return Err(format!(
                    "a variável `{key}` aparece mais de uma vez em `env`."
                ));
            }
        }
        Ok(())
    }

    /// A entrada como será gravada.
    pub fn to_object(&self) -> Map<String, Value> {
        let mut updates: Vec<(&'static str, Option<Value>)> = Field::ALL
            .iter()
            .map(|&f| (f.key(), self.text_update(f)))
            .collect();
        updates.push(("env", self.env_update()));
        updates.push(("readOnly", self.bool_update("readOnly", 0, self.read_only)));
        updates.push((
            "requireTransaction",
            self.bool_update("requireTransaction", 1, self.require_transaction),
        ));

        let mut out = Map::new();
        for (key, value) in &self.original {
            let key = if key == "env_file" {
                "envFile"
            } else {
                key.as_str()
            };
            match updates.iter().find(|(k, _)| *k == key) {
                Some((k, Some(v))) => {
                    out.insert((*k).to_string(), v.clone());
                }
                Some((_, None)) => {}
                None => {
                    out.insert(key.to_string(), value.clone());
                }
            }
        }
        for (key, value) in updates {
            if let Some(v) = value {
                if !out.contains_key(key) {
                    out.insert(key.to_string(), v);
                }
            }
        }
        out
    }

    /// Grava a entrada no arquivo. Recusa se o arquivo mudou desde a abertura,
    /// para não sobrescrever edições feitas fora do DBeas.
    pub fn save(&self) -> Result<(), String> {
        self.check()?;
        let current = std::fs::read_to_string(&self.file)
            .map_err(|e| format!("não foi possível ler {}: {e}", self.file.display()))?;
        if current != self.original_text {
            return Err(
                "o arquivo foi alterado fora do editor depois que ele foi aberto. \
                 Feche e abra o editor de novo para não perder essas alterações."
                    .into(),
            );
        }
        let text = self.render(&current)?;
        let target = std::fs::canonicalize(&self.file).unwrap_or_else(|_| self.file.clone());
        fsutil::atomic_write(&target, text.as_bytes())
            .map_err(|e| format!("não foi possível gravar {}: {e}", target.display()))
    }

    /// Texto completo do arquivo com esta entrada substituída, mantendo a
    /// indentação usada no original.
    fn render(&self, current: &str) -> Result<String, String> {
        let Ok(Value::Array(mut items)) = serde_json::from_str::<Value>(current) else {
            return Err("o arquivo não contém mais um array JSON válido".into());
        };
        let slot = items
            .get_mut(self.index)
            .ok_or("a entrada não existe mais no arquivo")?;
        *slot = Value::Object(self.to_object());

        let indent = detect_indent(current);
        let formatter = serde_json::ser::PrettyFormatter::with_indent(indent.as_bytes());
        let mut buf = Vec::new();
        let mut ser = serde_json::Serializer::with_formatter(&mut buf, formatter);
        serde::Serialize::serialize(&Value::Array(items), &mut ser).map_err(|e| e.to_string())?;
        let mut text = String::from_utf8(buf).map_err(|e| e.to_string())?;
        text.push('\n');
        Ok(text)
    }

    fn text_update(&self, field: Field) -> Option<Value> {
        let text = self.value(field);
        if text == self.initial[field.slot()] {
            // Intocado: valor e tipo originais. O nome antigo `env_file`
            // é regravado como `envFile`.
            return match self.original.get(field.key()) {
                Some(v) => Some(v.clone()),
                None if field == Field::EnvFile && !text.is_empty() => {
                    Some(Value::String(text.to_string()))
                }
                None => None,
            };
        }
        if text.is_empty() {
            return None;
        }
        if field == Field::DbPort && !matches!(self.original.get("dbPort"), Some(Value::String(_)))
        {
            if let Ok(port) = text.trim().parse::<u16>() {
                return Some(Value::from(port));
            }
        }
        Some(Value::String(text.to_string()))
    }

    fn env_update(&self) -> Option<Value> {
        if self.env == self.initial_env {
            return self.original.get("env").cloned();
        }
        let map: Map<String, Value> = self
            .env
            .iter()
            .filter(|(k, _)| !k.trim().is_empty())
            .map(|(k, v)| (k.trim().to_string(), Value::String(v.clone())))
            .collect();
        (!map.is_empty()).then_some(Value::Object(map))
    }

    fn bool_update(&self, key: &str, slot: usize, value: bool) -> Option<Value> {
        let original = self.original.get(key);
        if value == self.initial_bools[slot] {
            return original.cloned();
        }
        // `false` é o padrão: só é escrito se a chave já existia.
        (value || original.is_some()).then_some(Value::Bool(value))
    }

    fn reload_env_file(&mut self) {
        let raw = self.value(Field::EnvFile);
        self.file_vars = if raw.is_empty() {
            Ok(Vec::new())
        } else {
            let path = fsutil::normalize(&self.base_dir, Path::new(raw));
            env::read_env_file(&path).map_err(|e| e.to_string())
        };
        self.refresh();
    }

    /// Recalcula o ambiente das prévias e a validação.
    fn refresh(&mut self) {
        let overrides: Vec<(String, String)> = self
            .env
            .iter()
            .filter(|(k, _)| !k.trim().is_empty())
            .map(|(k, v)| (k.trim().to_string(), v.clone()))
            .collect();
        let file_vars = self.file_vars.as_deref().unwrap_or_default();
        self.env_map = env::merge(self.process_env.iter().cloned(), file_vars, &overrides);
        self.validation = self.check().and_then(|()| {
            config::resolve_entry(
                &self.to_object(),
                &self.base_dir,
                self.process_env.iter().cloned(),
            )
            .map(|_| ())
            .map_err(|e| e.to_string())
        });
    }
}

/// Texto exibido num campo para um valor JSON qualquer.
fn text_of(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// Indentação da primeira linha indentada do arquivo; dois espaços se não há.
fn detect_indent(text: &str) -> String {
    text.lines()
        .find_map(|line| {
            let indent: String = line
                .chars()
                .take_while(|c| *c == ' ' || *c == '\t')
                .collect();
            (!indent.is_empty() && indent.len() < line.len()).then_some(indent)
        })
        .unwrap_or_else(|| "  ".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn form(json: &str, env: &[(&str, &str)]) -> Form {
        let env = env
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        Form::from_text(json.to_string(), Path::new("/p/dbeas.json"), 0, env).unwrap()
    }

    #[test]
    fn untouched_entry_is_written_back_unchanged() {
        let json = r#"[{"zzz":1,"name":"a","dbPort":5433,"dbName":"x","readOnly":false,"env":{"B":"1","A":"2"}}]"#;
        let f = form(json, &[]);
        assert!(!f.is_dirty());
        let original: Value = serde_json::from_str(json).unwrap();
        assert_eq!(Value::Object(f.to_object()), original[0]);
        // A ordem das chaves é preservada.
        let keys: Vec<_> = f.to_object().keys().cloned().collect();
        assert_eq!(keys, ["zzz", "name", "dbPort", "dbName", "readOnly", "env"]);
    }

    #[test]
    fn edits_replace_values_and_empty_removes() {
        let mut f = form(
            r#"[{"name":"a","dbHost":"h","dbPort":"5432","dbName":"x"}]"#,
            &[],
        );
        f.set(Field::DbHost, String::new());
        f.set(Field::DbPort, "6000".into());
        f.set(Field::SslMode, "require".into());
        f.set_read_only(true);
        let obj = f.to_object();
        assert!(!obj.contains_key("dbHost"));
        // A porta era string no arquivo: continua string.
        assert_eq!(obj["dbPort"], Value::from("6000"));
        assert_eq!(obj["sslMode"], Value::from("require"));
        assert_eq!(obj["readOnly"], Value::Bool(true));
        assert!(f.is_dirty());
    }

    #[test]
    fn new_numeric_port_is_written_as_number() {
        let mut f = form(r#"[{"name":"a","dbName":"x"}]"#, &[]);
        f.set(Field::DbPort, "5433".into());
        assert_eq!(f.to_object()["dbPort"], Value::from(5433));
        f.set(Field::DbPort, "${env.P}".into());
        assert_eq!(f.to_object()["dbPort"], Value::from("${env.P}"));
    }

    #[test]
    fn false_booleans_are_not_added() {
        let mut f = form(r#"[{"name":"a","dbName":"x"}]"#, &[]);
        f.set_read_only(true);
        f.set_read_only(false);
        assert!(!f.to_object().contains_key("readOnly"));
    }

    #[test]
    fn env_rows_round_trip() {
        let mut f = form(r#"[{"name":"a","dbName":"x","env":{"A":"1"}}]"#, &[]);
        f.add_env();
        f.set_env_key(1, "B".into());
        f.set_env_value(1, "2".into());
        assert_eq!(
            Value::Object(f.to_object())["env"],
            serde_json::json!({"A":"1","B":"2"})
        );
        f.remove_env(0);
        f.remove_env(0);
        assert!(!f.to_object().contains_key("env"));
    }

    #[test]
    fn previews_resolve_with_form_env() {
        let mut f = form(
            r#"[{"name":"a","dbName":"x","env":{"H":"obj"},"dbHost":"${env.H}","dbUser":"${env.U}","dbPass":"${env.S}"}]"#,
            &[("U", "proc"), ("S", "segredo")],
        );
        assert_eq!(f.preview(Field::DbHost), Some(Preview::Value("obj".into())));
        assert_eq!(
            f.preview(Field::DbUser),
            Some(Preview::Value("proc".into()))
        );
        // Senha: só indica que existe valor.
        assert_eq!(f.preview(Field::DbPass), Some(Preview::Masked));
        // Sem referência: sem prévia.
        assert_eq!(f.preview(Field::DbName), None);

        f.set_env_key(0, "X".into());
        assert!(matches!(f.preview(Field::DbHost), Some(Preview::Error(_))));
        assert!(f.validation().is_err());
    }

    #[test]
    fn empty_password_preview_shows_nothing() {
        let f = form(
            r#"[{"name":"a","dbName":"x","dbUser":"u","dbPass":"${env.S}"}]"#,
            &[("S", "")],
        );
        assert_eq!(f.preview(Field::DbPass), None);
        let f = form(
            r#"[{"name":"a","dbName":"x","dbUser":"u","dbPass":"${env.NOPE}"}]"#,
            &[],
        );
        match f.preview(Field::DbPass) {
            Some(Preview::Error(e)) => assert!(e.contains("NOPE")),
            other => panic!("prévia inesperada: {other:?}"),
        }
    }

    #[test]
    fn env_file_feeds_previews() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".env"), "H=do_arquivo\n").unwrap();
        let mut f = Form::from_text(
            r#"[{"name":"a","dbName":"x","dbHost":"${env.H}"}]"#.into(),
            &dir.path().join("dbeas.json"),
            0,
            Vec::new(),
        )
        .unwrap();
        assert!(matches!(f.preview(Field::DbHost), Some(Preview::Error(_))));
        f.set(Field::EnvFile, ".env".into());
        assert_eq!(
            f.preview(Field::DbHost),
            Some(Preview::Value("do_arquivo".into()))
        );
    }

    #[test]
    fn ssl_choices_keep_custom_value() {
        let f = form(
            r#"[{"name":"a","dbName":"x","sslMode":"${env.M}"}]"#,
            &[("M", "disable")],
        );
        let choices = f.ssl_choices();
        assert_eq!(choices.len(), 7);
        assert_eq!(choices.last().unwrap().0, "${env.M}");
        assert_eq!(
            f.preview(Field::SslMode),
            Some(Preview::Value("disable".into()))
        );
        assert_eq!(SslChoice(String::new()).to_string(), "padrão (prefer)");
    }

    #[test]
    fn check_blocks_required_fields_and_bad_literal_port() {
        let mut f = form(r#"[{"name":"a","dbName":"x"}]"#, &[]);
        assert!(f.check().is_ok());
        f.set(Field::DbPort, "abc".into());
        assert!(f.check().is_err());
        f.set(Field::DbPort, "${env.P}".into());
        assert!(f.check().is_ok());
        f.set(Field::Name, "  ".into());
        assert!(f.check().is_err());
    }

    #[test]
    fn legacy_env_file_is_renamed_in_place() {
        let f = form(r#"[{"name":"a","env_file":".env","dbName":"x"}]"#, &[]);
        assert_eq!(f.value(Field::EnvFile), ".env");
        let keys: Vec<_> = f.to_object().keys().cloned().collect();
        assert_eq!(keys, ["name", "envFile", "dbName"]);
    }

    #[test]
    fn save_replaces_only_the_entry_and_detects_external_changes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dbeas.json");
        let text = "[\n    {\"name\": \"a\", \"dbName\": \"x\"},\n    {\"name\": \"b\", \"dbName\": \"y\"}\n]\n";
        std::fs::write(&path, text).unwrap();

        let mut f = Form::open(&path, 1).unwrap();
        f.set(Field::DbHost, "db.local".into());
        f.save().unwrap();
        let saved = std::fs::read_to_string(&path).unwrap();
        assert!(
            saved.contains("\n    {\n        \"name\": \"a\""),
            "{saved}"
        );
        let value: Value = serde_json::from_str(&saved).unwrap();
        assert_eq!(value[0], serde_json::json!({"name":"a","dbName":"x"}));
        assert_eq!(
            value[1],
            serde_json::json!({"name":"b","dbName":"y","dbHost":"db.local"})
        );

        // O arquivo mudou depois da abertura: não grava.
        let mut f = Form::open(&path, 0).unwrap();
        f.set(Field::DbHost, "outro".into());
        std::fs::write(&path, text).unwrap();
        assert!(f.save().is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
    }
}
