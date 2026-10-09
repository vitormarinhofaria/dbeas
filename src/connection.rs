//! Conexão PostgreSQL de um workspace.
//!
//! Cada conexão é um ator: uma tarefa tokio dona do `Client` que atende
//! pedidos em ordem. A interface conversa com ele por um `ConnectionHandle`
//! clonável, cujos métodos assíncronos são usados em `Task::perform`, sem
//! bloquear a thread da interface (spec §2, §11).
//!
//! Resultados grandes são lidos aos poucos de um `RowStream`. Só existe um
//! cursor aberto por conexão: qualquer outro pedido descarta o cursor anterior,
//! pois o protocolo processa as respostas em ordem e um cursor parado
//! bloquearia a conexão. As linhas já carregadas continuam na grade.

use std::ops::Range;
use std::pin::Pin;
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use tokio::sync::{mpsc, oneshot, watch};
use tokio_postgres::{CancelToken, Client, NoTls, RowStream};

use crate::metadata::{self, Catalog};
use crate::query::classify::{self, TxControl};
use crate::query::{self, ColumnInfo, Outcome, QueryError, ResultSet, Value, value};
use crate::workspace::ResolvedConfig;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Disconnected,
    Connecting,
    Connected,
    Error,
}

impl Status {
    pub fn label(self) -> &'static str {
        match self {
            Status::Disconnected => "desconectado",
            Status::Connecting => "conectando",
            Status::Connected => "conectado",
            Status::Error => "erro",
        }
    }
}

/// Estado da transação explícita na conexão.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TxState {
    /// Sem bloco de transação: cada instrução é confirmada sozinha.
    #[default]
    Idle,
    Active,
    /// Um comando falhou dentro do bloco; o PostgreSQL rejeita tudo até o ROLLBACK.
    Failed,
}

/// O que uma transação altera, exibido antes do COMMIT.
#[derive(Debug, Clone, Default)]
pub struct TxSummary {
    /// Contagens de `pg_stat_xact_user_tables`, inclusive efeitos de triggers.
    pub tables: Vec<TableChanges>,
    /// Instruções de escrita executadas na transação, com o retorno do servidor.
    pub statements: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TableChanges {
    pub schema: String,
    pub table: String,
    pub inserted: i64,
    pub updated: i64,
    pub deleted: i64,
}

/// Opções de uma execução.
#[derive(Debug, Clone, Copy, Default)]
pub struct ExecOptions {
    /// Permite BEGIN/COMMIT/ROLLBACK no SQL (arquivos salvos). No SQL manual de
    /// workspaces com `requireTransaction`, eles são bloqueados.
    pub allow_tx_control: bool,
}

/// Um `UPDATE ... RETURNING` vindo da grade.
#[derive(Debug, Clone)]
pub struct EditStatement {
    pub sql: String,
    pub params: Vec<Option<String>>,
    /// Descrição para o registro da transação.
    pub label: String,
}

#[derive(Clone)]
pub struct ConnectionHandle {
    requests: mpsc::UnboundedSender<Request>,
    cancel: CancelToken,
    tx: watch::Receiver<TxState>,
}

impl std::fmt::Debug for ConnectionHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConnectionHandle").finish_non_exhaustive()
    }
}

/// Linhas adicionais de um cursor aberto.
#[derive(Debug, Clone)]
pub struct Page {
    pub rows: Vec<Vec<Value>>,
    pub has_more: bool,
    pub truncated: bool,
}

enum Request {
    Execute {
        sql: String,
        statements: Vec<Range<usize>>,
        options: ExecOptions,
        reply: oneshot::Sender<Result<Outcome, QueryError>>,
    },
    Transaction {
        command: TxCommand,
        reply: oneshot::Sender<Result<(), String>>,
    },
    Summary {
        reply: oneshot::Sender<Result<TxSummary, String>>,
    },
    ApplyEdits {
        statements: Vec<EditStatement>,
        reply: oneshot::Sender<Result<Vec<Vec<Value>>, QueryError>>,
    },
    FetchMore {
        cursor: u64,
        reply: oneshot::Sender<Result<Page, QueryError>>,
    },
    Catalog {
        reply: oneshot::Sender<Result<Catalog, String>>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TxCommand {
    Begin,
    Commit,
    Rollback,
}

pub async fn connect(config: ResolvedConfig) -> Result<ConnectionHandle, String> {
    let mut pg = tokio_postgres::Config::new();
    pg.host(&config.host)
        .port(config.port)
        .user(&config.user)
        .dbname(&config.db_name)
        .application_name("dbeas")
        .connect_timeout(Duration::from_secs(10));
    if let Some(password) = config.password.expose() {
        pg.password(password);
    }
    if config.read_only {
        // Segunda camada de proteção do modo somente leitura (spec §6.3).
        pg.options("-c default_transaction_read_only=on");
    }

    // A mensagem de erro do tokio-postgres não inclui a senha.
    let (client, connection) = pg.connect(NoTls).await.map_err(|e| e.to_string())?;

    tokio::spawn(async move {
        // Encerra quando o cliente é descartado ou a conexão cai.
        let _ = connection.await;
    });

    let cancel = client.cancel_token();
    let (tx, rx) = mpsc::unbounded_channel();
    let (tx_watch, tx_state) = watch::channel(TxState::Idle);
    tokio::spawn(
        Actor {
            client,
            read_only: config.read_only,
            require_transaction: config.require_transaction,
            cursor: None,
            next_cursor: 1,
            tx: TxState::Idle,
            tx_watch,
            log: Vec::new(),
        }
        .run(rx),
    );

    Ok(ConnectionHandle {
        requests: tx,
        cancel,
        tx: tx_state,
    })
}

impl ConnectionHandle {
    pub async fn execute(
        &self,
        sql: String,
        statements: Vec<Range<usize>>,
    ) -> Result<Outcome, QueryError> {
        self.execute_with(sql, statements, ExecOptions::default())
            .await
    }

    pub async fn execute_with(
        &self,
        sql: String,
        statements: Vec<Range<usize>>,
        options: ExecOptions,
    ) -> Result<Outcome, QueryError> {
        let (reply, rx) = oneshot::channel();
        self.send(Request::Execute {
            sql,
            statements,
            options,
            reply,
        })?;
        rx.await.map_err(|_| closed())?
    }

    pub async fn transaction(&self, command: TxCommand) -> Result<(), String> {
        let (reply, rx) = oneshot::channel();
        self.send(Request::Transaction { command, reply })
            .map_err(|e| e.message)?;
        rx.await.map_err(|_| closed().message)?
    }

    pub async fn summary(&self) -> Result<TxSummary, String> {
        let (reply, rx) = oneshot::channel();
        self.send(Request::Summary { reply })
            .map_err(|e| e.message)?;
        rx.await.map_err(|_| closed().message)?
    }

    /// Aplica alterações da grade. Com transação ativa, entram nela (sob um
    /// savepoint, para que uma falha não a deixe em erro); sem transação, só é
    /// permitido fora de `requireTransaction` e a aplicação usa um bloco próprio.
    /// Retorna os valores gravados de cada linha (`RETURNING`).
    pub async fn apply_edits(
        &self,
        statements: Vec<EditStatement>,
    ) -> Result<Vec<Vec<Value>>, QueryError> {
        let (reply, rx) = oneshot::channel();
        self.send(Request::ApplyEdits { statements, reply })?;
        rx.await.map_err(|_| closed())?
    }

    /// Estado atual da transação. Atualizado pelo ator antes de responder a
    /// cada pedido.
    pub fn tx_state(&self) -> TxState {
        *self.tx.borrow()
    }

    pub async fn fetch_more(&self, cursor: u64) -> Result<Page, QueryError> {
        let (reply, rx) = oneshot::channel();
        self.send(Request::FetchMore { cursor, reply })?;
        rx.await.map_err(|_| closed())?
    }

    pub async fn catalog(&self) -> Result<Catalog, String> {
        let (reply, rx) = oneshot::channel();
        self.send(Request::Catalog { reply })
            .map_err(|e| e.message)?;
        rx.await.map_err(|_| closed().message)?
    }

    /// Pede ao servidor que cancele a consulta em andamento.
    pub async fn cancel(&self) -> Result<(), String> {
        self.cancel
            .cancel_query(NoTls)
            .await
            .map_err(|e| e.to_string())
    }

    pub fn is_alive(&self) -> bool {
        !self.requests.is_closed()
    }

    fn send(&self, request: Request) -> Result<(), QueryError> {
        self.requests.send(request).map_err(|_| closed())
    }
}

fn closed() -> QueryError {
    QueryError::simple("a conexão foi encerrada")
}

struct OpenCursor {
    id: u64,
    stream: Pin<Box<RowStream>>,
    loaded: usize,
    columns: usize,
}

struct Actor {
    client: Client,
    read_only: bool,
    require_transaction: bool,
    cursor: Option<OpenCursor>,
    next_cursor: u64,
    tx: TxState,
    tx_watch: watch::Sender<TxState>,
    /// Instruções de escrita da transação atual, para o resumo.
    log: Vec<String>,
}

impl Actor {
    async fn run(mut self, mut rx: mpsc::UnboundedReceiver<Request>) {
        while let Some(request) = rx.recv().await {
            if self.client.is_closed() {
                break;
            }
            match request {
                Request::Execute {
                    sql,
                    statements,
                    options,
                    reply,
                } => {
                    self.cursor = None;
                    let _ = reply.send(self.execute(&sql, statements, options).await);
                }
                Request::Transaction { command, reply } => {
                    self.cursor = None;
                    let _ = reply.send(self.transaction(command).await);
                }
                Request::Summary { reply } => {
                    self.cursor = None;
                    let _ = reply.send(self.summary().await);
                }
                Request::ApplyEdits { statements, reply } => {
                    self.cursor = None;
                    let _ = reply.send(self.apply_edits(statements).await);
                }
                Request::FetchMore { cursor, reply } => {
                    let _ = reply.send(self.fetch_more(cursor).await);
                }
                Request::Catalog { reply } => {
                    self.cursor = None;
                    let _ = reply.send(metadata::load(&self.client).await);
                }
            }
        }
    }

    fn set_tx(&mut self, state: TxState) {
        if state == TxState::Idle {
            self.log.clear();
        }
        self.tx = state;
        let _ = self.tx_watch.send(state);
    }

    /// Erro durante um bloco de transação deixa o bloco em erro.
    fn on_error(&mut self) {
        if self.tx == TxState::Active {
            self.set_tx(TxState::Failed);
        }
    }

    /// Política de `requireTransaction` e de transação com erro para uma
    /// instrução, avaliada com o estado atual (um arquivo pode abrir ou fechar a
    /// transação no meio do script).
    fn check_policy(&self, text: &str, options: ExecOptions) -> Result<Option<TxControl>, String> {
        let control = classify::transaction_control(text);
        if self.tx == TxState::Failed
            && !matches!(
                control,
                Some(TxControl::Rollback | TxControl::RollbackToSavepoint | TxControl::Commit)
            )
        {
            return Err(
                "a transação está com erro e rejeita novos comandos; use Cancelar (ROLLBACK)"
                    .into(),
            );
        }
        if !self.require_transaction {
            return Ok(control);
        }
        if control.is_some() {
            if options.allow_tx_control {
                return Ok(control);
            }
            return Err(
                "neste workspace a transação é controlada pelos botões Iniciar/Confirmar/Cancelar; \
                 BEGIN/COMMIT/ROLLBACK no editor estão bloqueados"
                    .into(),
            );
        }
        if self.tx != TxState::Active && classify::check_read_only(text).is_err() {
            return Err(
                "este workspace exige transação para escrita: inicie uma transação antes de executar".into(),
            );
        }
        Ok(None)
    }

    fn after_success(&mut self, text: &str, control: Option<TxControl>, message: &str) {
        match control {
            Some(TxControl::Begin) if self.tx == TxState::Idle => self.set_tx(TxState::Active),
            Some(TxControl::Commit | TxControl::Rollback) => self.set_tx(TxState::Idle),
            Some(TxControl::RollbackToSavepoint) if self.tx == TxState::Failed => {
                self.set_tx(TxState::Active)
            }
            Some(TxControl::Other)
                if text
                    .trim_start()
                    .to_ascii_lowercase()
                    .starts_with("prepare") =>
            {
                self.set_tx(TxState::Idle)
            }
            None if self.tx == TxState::Active && classify::check_read_only(text).is_err() => {
                let preview: String = text
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ")
                    .chars()
                    .take(160)
                    .collect();
                self.log.push(format!("{preview} → {message}"));
            }
            _ => {}
        }
    }

    async fn transaction(&mut self, command: TxCommand) -> Result<(), String> {
        let (sql, allowed) = match command {
            TxCommand::Begin => ("BEGIN", self.tx == TxState::Idle),
            TxCommand::Commit => ("COMMIT", self.tx == TxState::Active),
            TxCommand::Rollback => ("ROLLBACK", self.tx != TxState::Idle),
        };
        if !allowed {
            return Err(match self.tx {
                TxState::Idle => "não há transação ativa".into(),
                TxState::Active => "já existe uma transação ativa".into(),
                TxState::Failed => "a transação está com erro; só é possível cancelar".into(),
            });
        }
        match self.client.batch_execute(sql).await {
            Ok(()) => {
                self.set_tx(if command == TxCommand::Begin {
                    TxState::Active
                } else {
                    TxState::Idle
                });
                Ok(())
            }
            Err(e) => {
                if command == TxCommand::Commit {
                    // Falha no COMMIT (ex.: restrição adiada) encerra o bloco.
                    self.set_tx(TxState::Idle);
                }
                Err(QueryError::from_pg(&e, sql).message)
            }
        }
    }

    async fn summary(&mut self) -> Result<TxSummary, String> {
        if self.tx != TxState::Active {
            return Err("não há transação ativa".into());
        }
        let rows = self
            .client
            .query(
                "SELECT schemaname::text, relname::text, n_tup_ins, n_tup_upd, n_tup_del \
                 FROM pg_stat_xact_user_tables \
                 WHERE n_tup_ins + n_tup_upd + n_tup_del > 0 \
                 ORDER BY schemaname, relname",
                &[],
            )
            .await;
        let rows = match rows {
            Ok(rows) => rows,
            Err(e) => {
                self.on_error();
                return Err(format!("não foi possível obter o resumo: {e}"));
            }
        };
        Ok(TxSummary {
            tables: rows
                .iter()
                .map(|r| TableChanges {
                    schema: r.get(0),
                    table: r.get(1),
                    inserted: r.get(2),
                    updated: r.get(3),
                    deleted: r.get(4),
                })
                .collect(),
            statements: self.log.clone(),
        })
    }

    async fn apply_edits(
        &mut self,
        statements: Vec<EditStatement>,
    ) -> Result<Vec<Vec<Value>>, QueryError> {
        if self.read_only {
            return Err(QueryError::simple(
                "workspace somente leitura: alterações de dados estão bloqueadas",
            ));
        }
        let own_block = match self.tx {
            TxState::Failed => {
                return Err(QueryError::simple(
                    "a transação está com erro; use Cancelar (ROLLBACK) antes de salvar alterações",
                ));
            }
            TxState::Active => false,
            TxState::Idle if self.require_transaction => {
                return Err(QueryError::simple(
                    "este workspace exige transação: inicie uma transação antes de salvar alterações",
                ));
            }
            TxState::Idle => true,
        };

        let (open, undo, close) = if own_block {
            ("BEGIN", "ROLLBACK", "COMMIT")
        } else {
            (
                "SAVEPOINT dbeas_grade",
                "ROLLBACK TO SAVEPOINT dbeas_grade; RELEASE SAVEPOINT dbeas_grade",
                "RELEASE SAVEPOINT dbeas_grade",
            )
        };
        let fail = |e: &tokio_postgres::Error| QueryError::from_pg(e, "");
        if let Err(e) = self.client.batch_execute(open).await {
            self.on_error();
            return Err(fail(&e));
        }

        let mut written = Vec::new();
        for statement in &statements {
            let params: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = statement
                .params
                .iter()
                .map(|p| p as &(dyn tokio_postgres::types::ToSql + Sync))
                .collect();
            let error = match self.client.query(&statement.sql, &params).await {
                Ok(rows) if rows.len() == 1 => {
                    let row = &rows[0];
                    written.push(
                        (0..row.len())
                            .map(|i| {
                                let ty = row.columns()[i].type_();
                                match row.try_get::<_, value::Raw>(i) {
                                    Ok(raw) => value::decode(ty, raw.0),
                                    Err(_) => Value::Other {
                                        type_name: ty.name().to_string(),
                                        raw: Vec::new(),
                                    },
                                }
                            })
                            .collect(),
                    );
                    continue;
                }
                Ok(rows) if rows.is_empty() => QueryError::simple(format!(
                    "{}: a linha não foi encontrada (alterada ou removida por outra sessão?)",
                    statement.label
                )),
                Ok(rows) => QueryError::simple(format!(
                    "{}: a condição atingiu {} linhas; nada foi gravado",
                    statement.label,
                    rows.len()
                )),
                Err(e) => fail(&e),
            };
            // Desfaz o lote inteiro; a transação do usuário continua válida.
            if self.client.batch_execute(undo).await.is_err() {
                self.on_error();
            }
            return Err(error);
        }

        if let Err(e) = self.client.batch_execute(close).await {
            if own_block {
                let _ = self.client.batch_execute("ROLLBACK").await;
            } else {
                self.on_error();
            }
            return Err(fail(&e));
        }
        if !own_block {
            for statement in &statements {
                self.log
                    .push(format!("{} → 1 linha (grade)", statement.label));
            }
        }
        Ok(written)
    }

    async fn execute(
        &mut self,
        sql: &str,
        statements: Vec<Range<usize>>,
        options: ExecOptions,
    ) -> Result<Outcome, QueryError> {
        let started = Instant::now();
        let mut outcome = Outcome {
            result: None,
            result_statement: None,
            cursor: None,
            messages: Vec::new(),
            elapsed_ms: 0,
        };

        if self.read_only {
            for range in &statements {
                if let Err(reason) = classify::check_read_only(&sql[range.clone()]) {
                    let mut e = QueryError::simple(format!(
                        "bloqueado pelo modo somente leitura do workspace: {reason}. \
                         Esta é uma proteção da ferramenta e não substitui permissões no PostgreSQL."
                    ));
                    e.statement = Some(range.clone());
                    return Err(e);
                }
            }
        }

        let count = statements.len();
        for (i, range) in statements.into_iter().enumerate() {
            let text = &sql[range.clone()];
            let is_last = i + 1 == count;
            let with_context = |e: tokio_postgres::Error| {
                let mut err = QueryError::from_pg(&e, text);
                err.offset = err.offset.map(|o| o + range.start);
                err.statement = Some(range.clone());
                err
            };

            let control = match self.check_policy(text, options) {
                Ok(control) => control,
                Err(reason) => {
                    let mut e = QueryError::simple(reason);
                    e.statement = Some(range.clone());
                    return Err(e);
                }
            };

            let statement = match self.client.prepare(text).await {
                Ok(statement) => statement,
                Err(e) => {
                    self.on_error();
                    return Err(with_context(e));
                }
            };
            if statement.columns().is_empty() {
                let affected = match self.client.execute(&statement, &[]).await {
                    Ok(n) => n,
                    Err(e) => {
                        self.on_error();
                        return Err(with_context(e));
                    }
                };
                let message = format!("{}: {affected} linha(s) afetada(s)", command_word(text));
                self.after_success(text, control, &message);
                outcome.messages.push(message);
                continue;
            }

            let columns: Vec<ColumnInfo> = statement
                .columns()
                .iter()
                .map(|c| ColumnInfo {
                    name: c.name().to_string(),
                    type_name: c.type_().name().to_string(),
                    table_oid: c.table_oid(),
                    column_id: c.column_id(),
                })
                .collect();
            let stream = match self
                .client
                .query_raw(
                    &statement,
                    std::iter::empty::<&(dyn tokio_postgres::types::ToSql + Sync)>(),
                )
                .await
            {
                Ok(stream) => stream,
                Err(e) => {
                    self.on_error();
                    return Err(with_context(e));
                }
            };
            let mut cursor = OpenCursor {
                id: 0,
                stream: Box::pin(stream),
                loaded: 0,
                columns: columns.len(),
            };

            // Só a última instrução mantém o cursor aberto; o resultado das
            // intermediárias é substituído pelo das seguintes.
            let page = if is_last { query::PAGE_SIZE } else { 1 };
            let (rows, has_more) = match read_rows(&mut cursor, page).await {
                Ok(read) => read,
                Err(e) => {
                    self.on_error();
                    return Err(with_context(e));
                }
            };
            // Ex.: UPDATE ... RETURNING dentro da transação.
            self.after_success(
                text,
                control,
                &format!("{} linha(s) retornada(s)", rows.len()),
            );
            outcome.messages.push(format!(
                "{} linha(s) lidas{}",
                rows.len(),
                if has_more { " (há mais)" } else { "" }
            ));
            outcome.result_statement = Some(range.clone());
            outcome.result = Some(ResultSet {
                columns,
                rows,
                has_more,
                truncated: false,
            });

            if is_last && has_more {
                cursor.id = self.next_cursor;
                self.next_cursor += 1;
                outcome.cursor = Some(cursor.id);
                self.cursor = Some(cursor);
            }
        }

        outcome.elapsed_ms = started.elapsed().as_millis();
        Ok(outcome)
    }

    async fn fetch_more(&mut self, id: u64) -> Result<Page, QueryError> {
        let Some(cursor) = self.cursor.as_mut().filter(|c| c.id == id) else {
            return Err(QueryError::simple(
                "o cursor deste resultado foi fechado por outra operação na conexão; execute a consulta novamente",
            ));
        };
        let budget = query::MAX_ROWS
            .saturating_sub(cursor.loaded)
            .min(query::PAGE_SIZE);
        let (rows, has_more) = match read_rows(cursor, budget).await {
            Ok(read) => read,
            Err(e) => {
                self.cursor = None;
                self.on_error();
                return Err(QueryError::from_pg(&e, ""));
            }
        };
        let Some(cursor) = self.cursor.as_mut() else {
            return Err(QueryError::simple("cursor fechado"));
        };
        let truncated = has_more && cursor.loaded >= query::MAX_ROWS;
        if !has_more || truncated {
            self.cursor = None;
        }
        Ok(Page {
            rows,
            has_more: has_more && !truncated,
            truncated,
        })
    }
}

async fn read_rows(
    cursor: &mut OpenCursor,
    limit: usize,
) -> Result<(Vec<Vec<Value>>, bool), tokio_postgres::Error> {
    let mut rows = Vec::new();
    while rows.len() < limit {
        match cursor.stream.next().await {
            Some(row) => {
                let row = row?;
                let values = (0..cursor.columns)
                    .map(|i| {
                        let ty = row.columns()[i].type_();
                        match row.try_get::<_, value::Raw>(i) {
                            Ok(raw) => value::decode(ty, raw.0),
                            Err(_) => Value::Other {
                                type_name: ty.name().to_string(),
                                raw: Vec::new(),
                            },
                        }
                    })
                    .collect();
                rows.push(values);
            }
            None => {
                cursor.loaded += rows.len();
                return Ok((rows, false));
            }
        }
    }
    cursor.loaded += rows.len();
    // Não sabemos se há mais sem ler a próxima linha; assumimos que sim e o
    // próximo `fetch_more` descobre o fim.
    Ok((rows, true))
}

fn command_word(sql: &str) -> String {
    sql.split_whitespace().next().unwrap_or("").to_uppercase()
}
