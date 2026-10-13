// SPDX-FileCopyrightText: 2026 Vitor Marinho Faria
// SPDX-License-Identifier: GPL-3.0-or-later

//! Edição gráfica das entradas de um `dbeas.json`.
//!
//! O `Document` reúne um formulário por entrada do arquivo (uma aba na tela) e
//! as entradas novas, acrescentadas ao fim do array ao gravar. Cada formulário
//! guarda o texto cru de cada campo (com as referências
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

    /// Nome descritivo exibido no editor, acima da chave.
    pub fn title(self) -> &'static str {
        match self {
            Field::Name => "Nome do workspace",
            Field::EnvFile => "Arquivo de variáveis",
            Field::DbHost => "Servidor",
            Field::DbPort => "Porta",
            Field::DbUser => "Usuário",
            Field::DbPass => "Senha",
            Field::DbName => "Nome do banco",
            Field::SqlFilesDir => "Diretório de SQL",
            Field::SslMode => "Modo TLS",
            Field::SslRootCert => "Certificado CA",
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

/// Todas as entradas de um `dbeas.json`, editadas juntas e gravadas de uma vez.
pub struct Document {
    pub file: PathBuf,
    original_text: String,
    /// Entradas do arquivo, na ordem do array, seguidas das novas. Itens do
    /// array que não são objetos não viram abas, mas são preservados.
    pub forms: Vec<Form>,
    /// Aba exibida.
    pub active: usize,
    base_dir: PathBuf,
    process_env: Vec<(String, String)>,
}

impl Document {
    /// Abre `file` com o ambiente do processo, exibindo a entrada `entry`.
    pub fn open(file: &Path, entry: usize) -> Result<Self, String> {
        let text = std::fs::read_to_string(file)
            .map_err(|e| format!("não foi possível ler {}: {e}", file.display()))?;
        Self::from_text(text, file, entry, std::env::vars().collect())
    }

    pub fn from_text(
        text: String,
        file: &Path,
        entry: usize,
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
        match items.get(entry) {
            Some(Value::Object(_)) => {}
            Some(_) => return Err("a entrada não é um objeto JSON".into()),
            None if items.is_empty() && entry == 0 => {}
            None => return Err("a entrada não existe mais no arquivo".into()),
        }
        let base_dir = file
            .parent()
            .unwrap_or_else(|| Path::new("/"))
            .to_path_buf();
        let forms: Vec<Form> = items
            .into_iter()
            .enumerate()
            .filter_map(|(i, item)| match item {
                Value::Object(map) => Some(Form::new(map, Some(i), &base_dir, &process_env)),
                _ => None,
            })
            .collect();
        let active = forms
            .iter()
            .position(|f| f.index == Some(entry))
            .unwrap_or(0);
        let mut doc = Self {
            file: file.to_path_buf(),
            original_text: text,
            forms,
            active,
            base_dir,
            process_env,
        };
        if doc.forms.is_empty() {
            doc.add_entry();
        }
        Ok(doc)
    }

    pub fn form(&self) -> &Form {
        &self.forms[self.active]
    }

    pub fn form_mut(&mut self) -> &mut Form {
        &mut self.forms[self.active]
    }

    pub fn select(&mut self, tab: usize) {
        if tab < self.forms.len() {
            self.active = tab;
        }
    }

    /// Diretório do `dbeas.json`, base dos caminhos relativos.
    pub fn base_dir(&self) -> &Path {
        &self.base_dir
    }

    /// Texto a gravar para um diretório escolhido no seletor: relativo ao
    /// diretório do `dbeas.json` (`./sql`, ou `.` para ele mesmo) quando
    /// estiver abaixo dele, absoluto caso contrário.
    pub fn dir_value(&self, picked: &Path) -> String {
        let canon = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
        let (base, picked) = (canon(&self.base_dir), canon(picked));
        match picked.strip_prefix(&base) {
            Ok(rel) if rel.as_os_str().is_empty() => ".".into(),
            Ok(rel) => format!("./{}", rel.display()),
            Err(_) => picked.display().to_string(),
        }
    }

    /// Acrescenta uma entrada vazia (gravada ao fim do array) e a exibe.
    pub fn add_entry(&mut self) {
        self.forms.push(Form::new(
            Map::new(),
            None,
            &self.base_dir,
            &self.process_env,
        ));
        self.active = self.forms.len() - 1;
    }

    /// Descarta uma entrada nova, ainda não gravada. Entradas do arquivo não
    /// são removidas por aqui.
    pub fn remove_new_entry(&mut self, tab: usize) {
        if self.forms.get(tab).is_some_and(|f| f.index.is_none()) && self.forms.len() > 1 {
            self.forms.remove(tab);
            if self.active >= tab && self.active > 0 {
                self.active -= 1;
            }
        }
    }

    /// Há algo a gravar: entrada alterada ou entrada nova.
    pub fn is_dirty(&self) -> bool {
        self.forms.iter().any(|f| f.index.is_none() || f.is_dirty())
    }

    /// Pode gravar: há alterações e nenhuma aba com erro bloqueante.
    pub fn can_save(&self) -> bool {
        self.is_dirty() && self.blocking().is_none()
    }

    /// Primeira aba com erro que impede gravar.
    pub fn blocking(&self) -> Option<(usize, String)> {
        self.forms
            .iter()
            .enumerate()
            .find_map(|(i, f)| f.check().err().map(|e| (i, e)))
    }

    /// Grava todas as entradas. Recusa se o arquivo mudou desde a abertura,
    /// para não sobrescrever edições feitas fora do DBeas.
    pub fn save(&self) -> Result<(), String> {
        if let Some((tab, e)) = self.blocking() {
            return Err(format!("{}: {e}", self.forms[tab].label()));
        }
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

    /// Texto completo do arquivo com as entradas substituídas e as novas
    /// acrescentadas, mantendo a indentação usada no original.
    fn render(&self, current: &str) -> Result<String, String> {
        let Ok(Value::Array(mut items)) = serde_json::from_str::<Value>(current) else {
            return Err("o arquivo não contém mais um array JSON válido".into());
        };
        for form in &self.forms {
            let value = Value::Object(form.to_object());
            match form.index {
                Some(i) => {
                    *items
                        .get_mut(i)
                        .ok_or("a entrada não existe mais no arquivo")? = value
                }
                None => items.push(value),
            }
        }

        let indent = detect_indent(current);
        let formatter = serde_json::ser::PrettyFormatter::with_indent(indent.as_bytes());
        let mut buf = Vec::new();
        let mut ser = serde_json::Serializer::with_formatter(&mut buf, formatter);
        serde::Serialize::serialize(&Value::Array(items), &mut ser).map_err(|e| e.to_string())?;
        let mut text = String::from_utf8(buf).map_err(|e| e.to_string())?;
        text.push('\n');
        Ok(text)
    }
}

/// Formulário de uma entrada (uma aba do editor).
pub struct Form {
    /// Posição da entrada no array do arquivo; `None` para entrada nova.
    pub index: Option<usize>,
    base_dir: PathBuf,
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
    fn new(
        original: Map<String, Value>,
        index: Option<usize>,
        base_dir: &Path,
        process_env: &[(String, String)],
    ) -> Self {
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
            index,
            base_dir: base_dir.to_path_buf(),
            values: initial.clone(),
            initial,
            env: initial_env.clone(),
            initial_env,
            read_only: initial_bools[0],
            require_transaction: initial_bools[1],
            initial_bools,
            original,
            process_env: process_env.to_vec(),
            file_vars: Ok(Vec::new()),
            env_map: EnvMap::new(),
            validation: Ok(()),
        };
        form.reload_env_file();
        form
    }

    /// Rótulo da aba: o `name` digitado, ou a posição da entrada.
    pub fn label(&self) -> String {
        let name = self.value(Field::Name).trim();
        match self.index {
            _ if !name.is_empty() => name.to_string(),
            Some(i) => format!("entrada {}", i + 1),
            None => "nova entrada".into(),
        }
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

    /// A entrada como ficará gravada, resolvida como na carga do arquivo
    /// (para testar a conexão antes de salvar).
    pub fn resolved(&self) -> Result<config::ResolvedConfig, String> {
        self.check()?;
        config::resolve_entry(
            &self.to_object(),
            &self.base_dir,
            self.process_env.iter().cloned(),
        )
        .map_err(|e| e.to_string())
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
        self.validation = self.resolved().map(|_| ());
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
        let mut doc =
            Document::from_text(json.to_string(), Path::new("/p/dbeas.json"), 0, env).unwrap();
        doc.forms.remove(0)
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
        let mut doc = Document::from_text(
            r#"[{"name":"a","dbName":"x","dbHost":"${env.H}"}]"#.into(),
            &dir.path().join("dbeas.json"),
            0,
            Vec::new(),
        )
        .unwrap();
        let f = doc.form_mut();
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

        let mut doc = Document::open(&path, 1).unwrap();
        assert_eq!(doc.active, 1);
        doc.form_mut().set(Field::DbHost, "db.local".into());
        doc.save().unwrap();
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
        let mut doc = Document::open(&path, 0).unwrap();
        doc.form_mut().set(Field::DbHost, "outro".into());
        std::fs::write(&path, text).unwrap();
        assert!(doc.save().is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
    }

    #[test]
    fn document_has_one_tab_per_object_entry() {
        let doc = Document::from_text(
            r#"[{"name":"a","dbName":"x"},42,{"dbName":"y"}]"#.into(),
            Path::new("/p/dbeas.json"),
            2,
            Vec::new(),
        )
        .unwrap();
        let labels: Vec<_> = doc.forms.iter().map(Form::label).collect();
        assert_eq!(labels, ["a", "entrada 3"]);
        assert_eq!(doc.active, 1);
        assert!(!doc.is_dirty());

        let err = Document::from_text("[1]".into(), Path::new("/p/dbeas.json"), 0, Vec::new());
        assert!(err.is_err());
    }

    #[test]
    fn empty_array_opens_with_a_new_entry() {
        let doc =
            Document::from_text("[]".into(), Path::new("/p/dbeas.json"), 0, Vec::new()).unwrap();
        assert_eq!(doc.forms.len(), 1);
        assert_eq!(doc.form().index, None);
        assert!(doc.is_dirty());
        assert!(doc.blocking().is_some());
    }

    #[test]
    fn new_entries_are_appended_and_edits_saved_together() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dbeas.json");
        std::fs::write(&path, "[{\"name\":\"a\",\"dbName\":\"x\"},true]").unwrap();

        let mut doc = Document::open(&path, 0).unwrap();
        doc.form_mut().set(Field::DbHost, "h".into());
        doc.add_entry();
        assert_eq!(doc.active, 1);
        assert_eq!(doc.form().label(), "nova entrada");
        // Entrada nova incompleta impede gravar e aponta a aba.
        assert_eq!(doc.blocking().map(|(tab, _)| tab), Some(1));
        assert!(doc.save().is_err());
        doc.form_mut().set(Field::Name, "b".into());
        doc.form_mut().set(Field::DbName, "y".into());
        doc.form_mut().set(Field::DbPort, "5433".into());
        assert!(doc.blocking().is_none());
        doc.save().unwrap();

        let value: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            value,
            serde_json::json!([
                {"name":"a","dbName":"x","dbHost":"h"},
                true,
                {"name":"b","dbPort":5433,"dbName":"y"}
            ])
        );
    }

    #[test]
    fn picked_dir_is_relative_to_the_json_when_below_it() {
        let tmp = std::env::temp_dir().join(format!("dbeas-dir-value-{}", std::process::id()));
        std::fs::create_dir_all(tmp.join("ws/sql/sub")).unwrap();
        let file = tmp.join("ws/dbeas.json");
        let doc = Document::from_text("[]".into(), &file, 0, vec![]).unwrap();
        assert_eq!(doc.dir_value(&tmp.join("ws")), ".");
        assert_eq!(doc.dir_value(&tmp.join("ws/sql/sub")), "./sql/sub");
        let outside = std::fs::canonicalize(&tmp).unwrap();
        assert_eq!(doc.dir_value(&tmp), outside.display().to_string());
        std::fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn only_new_entries_can_be_discarded() {
        let mut doc = Document::from_text(
            r#"[{"name":"a","dbName":"x"}]"#.into(),
            Path::new("/p/dbeas.json"),
            0,
            Vec::new(),
        )
        .unwrap();
        doc.remove_new_entry(0);
        assert_eq!(doc.forms.len(), 1);
        doc.add_entry();
        doc.add_entry();
        assert_eq!(doc.active, 2);
        doc.remove_new_entry(1);
        assert_eq!((doc.forms.len(), doc.active), (2, 1));
        doc.remove_new_entry(1);
        assert_eq!((doc.forms.len(), doc.active), (1, 0));
        assert!(!doc.is_dirty());
    }
}
