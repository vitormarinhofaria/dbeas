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
use tokio::sync::{mpsc, oneshot};
use tokio_postgres::{CancelToken, Client, NoTls, RowStream};

use crate::metadata::{self, Catalog};
use crate::query::{self, ColumnInfo, Outcome, QueryError, ResultSet, Value, classify, value};
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

#[derive(Clone)]
pub struct ConnectionHandle {
    requests: mpsc::UnboundedSender<Request>,
    cancel: CancelToken,
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
        reply: oneshot::Sender<Result<Outcome, QueryError>>,
    },
    FetchMore {
        cursor: u64,
        reply: oneshot::Sender<Result<Page, QueryError>>,
    },
    Catalog {
        reply: oneshot::Sender<Result<Catalog, String>>,
    },
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
    tokio::spawn(
        Actor {
            client,
            read_only: config.read_only,
            cursor: None,
            next_cursor: 1,
        }
        .run(rx),
    );

    Ok(ConnectionHandle {
        requests: tx,
        cancel,
    })
}

impl ConnectionHandle {
    pub async fn execute(
        &self,
        sql: String,
        statements: Vec<Range<usize>>,
    ) -> Result<Outcome, QueryError> {
        let (reply, rx) = oneshot::channel();
        self.send(Request::Execute {
            sql,
            statements,
            reply,
        })?;
        rx.await.map_err(|_| closed())?
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
    cursor: Option<OpenCursor>,
    next_cursor: u64,
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
                    reply,
                } => {
                    self.cursor = None;
                    let _ = reply.send(self.execute(&sql, statements).await);
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

    async fn execute(
        &mut self,
        sql: &str,
        statements: Vec<Range<usize>>,
    ) -> Result<Outcome, QueryError> {
        let started = Instant::now();
        let mut outcome = Outcome {
            result: None,
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

            let statement = self.client.prepare(text).await.map_err(with_context)?;
            if statement.columns().is_empty() {
                let affected = self
                    .client
                    .execute(&statement, &[])
                    .await
                    .map_err(with_context)?;
                outcome.messages.push(format!(
                    "{}: {affected} linha(s) afetada(s)",
                    command_word(text)
                ));
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
            let stream = self
                .client
                .query_raw(
                    &statement,
                    std::iter::empty::<&(dyn tokio_postgres::types::ToSql + Sync)>(),
                )
                .await
                .map_err(with_context)?;
            let mut cursor = OpenCursor {
                id: 0,
                stream: Box::pin(stream),
                loaded: 0,
                columns: columns.len(),
            };

            // Só a última instrução mantém o cursor aberto; o resultado das
            // intermediárias é substituído pelo das seguintes.
            let page = if is_last { query::PAGE_SIZE } else { 1 };
            let (rows, has_more) = read_rows(&mut cursor, page).await.map_err(with_context)?;
            outcome.messages.push(format!(
                "{} linha(s) lidas{}",
                rows.len(),
                if has_more { " (há mais)" } else { "" }
            ));
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
        let (rows, has_more) = read_rows(cursor, budget)
            .await
            .map_err(|e| QueryError::from_pg(&e, ""))?;
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
