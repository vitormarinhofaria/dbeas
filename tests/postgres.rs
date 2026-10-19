// SPDX-FileCopyrightText: 2026 Vitor Marinho Faria
// SPDX-License-Identifier: GPL-3.0-or-later

//! Testes contra um PostgreSQL real. Executados apenas quando
//! `DBEASY_TEST_PG` estiver definido no formato `host:porta:usuario:senha:banco`.
//!
//! Exemplo:
//! docker run -d --name dbeasy-test-pg -e POSTGRES_PASSWORD=dbeasy -e POSTGRES_USER=dbeasy \
//!   -e POSTGRES_DB=dbeasy_dev -p 127.0.0.1:55432:5432 postgres:17
//! DBEASY_TEST_PG=127.0.0.1:55432:dbeasy:dbeasy:dbeasy_dev cargo test --test postgres

use dbeasy::connection;
use dbeasy::query::{Value, split};
use dbeasy::workspace::ResolvedConfig;
use dbeasy::workspace::config::Secret;

fn config(read_only: bool) -> Option<ResolvedConfig> {
    config_with(read_only, false)
}

fn config_with(read_only: bool, require_transaction: bool) -> Option<ResolvedConfig> {
    let raw = std::env::var("DBEASY_TEST_PG").ok()?;
    let parts: Vec<&str> = raw.split(':').collect();
    let [host, port, user, pass, db] = parts.as_slice() else {
        panic!("DBEASY_TEST_PG malformado")
    };
    Some(ResolvedConfig {
        name: "teste".into(),
        host: host.to_string(),
        port: port.parse().unwrap(),
        user: user.to_string(),
        password: Secret::new(Some(pass.to_string())),
        db_name: db.to_string(),
        read_only,
        require_transaction,
        sql_files_dir: None,
        ssl_mode: dbeasy::workspace::SslMode::Disable,
        ssl_root_cert: None,
    })
}

fn all(sql: &str) -> Vec<std::ops::Range<usize>> {
    split::statements(sql)
}

#[tokio::test]
async fn executes_queries_pages_and_reads_catalog() {
    let Some(cfg) = config(false) else { return };
    let conn = connection::connect(cfg).await.expect("conecta");

    let setup = "drop table if exists dbeasy_t; \
                 create table dbeasy_t (id serial primary key, nome text, valor numeric(10,2), criado timestamptz default now()); \
                 insert into dbeasy_t (nome, valor) select 'n' || g, g * 1.5 from generate_series(1, 1200) g; \
                 insert into dbeasy_t (nome, valor) values (null, null), ('', 0)";
    conn.execute(setup.into(), all(setup)).await.expect("setup");

    let sql = "select id, nome, valor from dbeasy_t order by id".to_string();
    let len = sql.len();
    let out = conn.execute(sql, vec![0..len]).await.expect("select");
    let rs = out.result.expect("resultado");
    assert_eq!(rs.columns.len(), 3);
    assert_eq!(rs.columns[2].type_name, "numeric");
    assert!(rs.columns[0].table_oid.is_some());
    assert_eq!(rs.rows.len(), dbeasy::query::PAGE_SIZE);
    assert!(rs.has_more);
    assert_eq!(rs.rows[0][2], Value::Numeric("1.50".into()));

    let cursor = out.cursor.expect("cursor aberto");
    let mut total = rs.rows.len();
    loop {
        let page = conn.fetch_more(cursor).await.expect("fetch");
        total += page.rows.len();
        if !page.has_more {
            break;
        }
    }
    assert_eq!(total, 1202);

    let sql = "select nome from dbeasy_t where id > 1200 order by id".to_string();
    let len = sql.len();
    let rs = conn
        .execute(sql, vec![0..len])
        .await
        .unwrap()
        .result
        .unwrap();
    assert_eq!(rs.rows[0][0], Value::Null);
    assert_eq!(rs.rows[1][0], Value::Text(String::new()));

    let catalog = conn.catalog().await.expect("catálogo");
    let rel = catalog
        .find("public", "dbeasy_t")
        .expect("tabela no catálogo");
    assert_eq!(
        rel.columns
            .iter()
            .map(|c| c.name.as_str())
            .collect::<Vec<_>>(),
        ["id", "nome", "valor", "criado"]
    );
    assert!(rel.columns[0].primary_key);
}

#[tokio::test]
async fn error_positions_are_reported_in_script_coordinates() {
    let Some(cfg) = config(false) else { return };
    let conn = connection::connect(cfg).await.unwrap();
    let script = "select 1;\nselect nao_existe from pg_class;";
    let err = conn.execute(script.into(), all(script)).await.unwrap_err();
    let offset = err.offset.expect("posição");
    assert_eq!(&script[offset..offset + 10], "nao_existe");
    assert_eq!(split::line_col(script, offset), (2, 8));
}

#[tokio::test]
async fn read_only_blocks_writes_in_two_layers() {
    let Some(cfg) = config(true) else { return };
    let rw = connection::connect(config(false).unwrap()).await.unwrap();
    let setup = "create sequence if not exists dbeasy_seq".to_string();
    let len = setup.len();
    rw.execute(setup, vec![0..len]).await.unwrap();
    let conn = connection::connect(cfg).await.unwrap();

    let sql = "create table nao_pode (a int)".to_string();
    let len = sql.len();
    let err = conn.execute(sql, vec![0..len]).await.unwrap_err();
    assert!(err.message.contains("somente leitura"), "{}", err.message);

    // Segunda camada: a sessão do servidor está em modo somente leitura.
    let sql = "select nextval('dbeasy_seq')".to_string();
    let len = sql.len();
    let err = conn.execute(sql, vec![0..len]).await.unwrap_err();
    assert_eq!(err.code.as_deref(), Some("25006"), "{}", err.message);
}

#[tokio::test]
async fn transformed_queries_run_and_keep_their_meaning() {
    use dbeasy::query::transform::{self, Direction};
    let Some(cfg) = config(false) else { return };
    let conn = connection::connect(cfg).await.unwrap();
    let run = |sql: String| {
        let conn = conn.clone();
        async move {
            let len = sql.len();
            conn.execute(sql, vec![0..len])
                .await
                .expect("executa")
                .result
                .expect("linhas")
        }
    };

    let base = "SELECT g AS id, -- identificador\n       'n' || g AS nome,\n       g * 2 AS dobro\nFROM generate_series(1, 5) g\nLIMIT 4";
    let sorted = transform::set_order(base, "dobro", Some(Direction::Desc)).unwrap();
    let rs = run(sorted.clone()).await;
    assert_eq!(rs.rows[0][0], Value::Int(5));
    assert_eq!(rs.rows.len(), 4);

    let hidden = transform::remove_item(&sorted, 1).unwrap();
    let rs = run(hidden.clone()).await;
    assert_eq!(
        rs.columns
            .iter()
            .map(|c| c.name.as_str())
            .collect::<Vec<_>>(),
        ["id", "dobro"]
    );

    let reordered = transform::reorder_items(&hidden, &[1, 0]).unwrap();
    let rs = run(reordered).await;
    assert_eq!(
        rs.columns
            .iter()
            .map(|c| c.name.as_str())
            .collect::<Vec<_>>(),
        ["dobro", "id"]
    );
    assert_eq!(rs.rows[0][0], Value::Int(10));
}

async fn run(
    conn: &connection::ConnectionHandle,
    sql: &str,
) -> Result<dbeasy::query::Outcome, dbeasy::query::QueryError> {
    let len = sql.len();
    conn.execute(sql.to_string(), vec![0..len]).await
}

#[tokio::test]
async fn require_transaction_policy_and_summary() {
    use connection::{TxCommand, TxState};
    let Some(cfg) = config_with(false, true) else {
        return;
    };
    let setup = connection::connect(config(false).unwrap()).await.unwrap();
    run(
        &setup,
        "create table if not exists dbeasy_tx (id int primary key, v text)",
    )
    .await
    .unwrap();
    run(&setup, "delete from dbeasy_tx").await.unwrap();

    let conn = connection::connect(cfg).await.unwrap();
    // Leitura sem transação é permitida; escrita não.
    run(&conn, "select count(*) from dbeasy_tx").await.unwrap();
    let err = run(&conn, "insert into dbeasy_tx values (1, 'a')")
        .await
        .unwrap_err();
    assert!(
        err.message.contains("inicie uma transação"),
        "{}",
        err.message
    );
    // Controle de transação pelo editor é bloqueado.
    let err = run(&conn, "begin").await.unwrap_err();
    assert!(err.message.contains("botões"), "{}", err.message);

    conn.transaction(TxCommand::Begin).await.unwrap();
    assert_eq!(conn.tx_state(), TxState::Active);
    run(&conn, "insert into dbeasy_tx values (1, 'a'), (2, 'b')")
        .await
        .unwrap();
    run(&conn, "update dbeasy_tx set v = 'c' where id = 2")
        .await
        .unwrap();
    let summary = conn.summary().await.unwrap();
    let t = summary
        .tables
        .iter()
        .find(|t| t.table == "dbeasy_tx")
        .expect("tabela no resumo");
    assert_eq!((t.inserted, t.updated, t.deleted), (2, 1, 0));
    assert_eq!(summary.statements.len(), 2);

    // Erro dentro da transação: estado de erro, só ROLLBACK é aceito.
    run(&conn, "insert into dbeasy_tx values (1, 'dup')")
        .await
        .unwrap_err();
    assert_eq!(conn.tx_state(), TxState::Failed);
    let err = run(&conn, "select 1").await.unwrap_err();
    assert!(err.message.contains("com erro"), "{}", err.message);
    assert!(conn.transaction(TxCommand::Commit).await.is_err());
    conn.transaction(TxCommand::Rollback).await.unwrap();
    assert_eq!(conn.tx_state(), TxState::Idle);
    let rs = run(&conn, "select count(*) from dbeasy_tx")
        .await
        .unwrap()
        .result
        .unwrap();
    assert_eq!(rs.rows[0][0], Value::Int(0));
}

#[tokio::test]
async fn saved_files_may_control_the_transaction() {
    use connection::{ExecOptions, TxCommand, TxState};
    let Some(cfg) = config_with(false, true) else {
        return;
    };
    let conn = connection::connect(cfg).await.unwrap();
    conn.transaction(TxCommand::Begin).await.unwrap();
    let script =
        "create temp table dbeasy_f (a int); insert into dbeasy_f values (1); commit; select 1";
    conn.execute_with(
        script.into(),
        all(script),
        ExecOptions {
            allow_tx_control: true,
        },
    )
    .await
    .unwrap();
    assert_eq!(conn.tx_state(), TxState::Idle);
}

#[tokio::test]
async fn grid_edits_are_atomic_and_join_the_transaction() {
    use connection::{EditStatement, TxCommand, TxState};
    let Some(cfg) = config(false) else { return };
    let conn = connection::connect(cfg).await.unwrap();
    run(
        &conn,
        "create table if not exists dbeasy_ed (id int primary key, nome text)",
    )
    .await
    .unwrap();
    run(&conn, "delete from dbeasy_ed").await.unwrap();
    run(&conn, "insert into dbeasy_ed values (1, 'a'), (2, 'b')")
        .await
        .unwrap();

    let edit = |id: &str, nome: Option<&str>| EditStatement {
        sql:
            "UPDATE dbeasy_ed SET nome = $1::text::text WHERE id = $2::text::integer RETURNING nome"
                .into(),
        params: vec![nome.map(str::to_string), Some(id.to_string())],
        label: format!("id {id}"),
    };

    // Sem transação (workspace comum): bloco próprio, confirmado.
    let written = conn
        .apply_edits(vec![edit("1", Some("x")), edit("2", None)])
        .await
        .unwrap();
    assert_eq!(
        written,
        vec![vec![Value::Text("x".into())], vec![Value::Null]]
    );
    assert_eq!(conn.tx_state(), TxState::Idle);

    // Linha inexistente desfaz o lote inteiro.
    let err = conn
        .apply_edits(vec![edit("1", Some("y")), edit("99", Some("z"))])
        .await
        .unwrap_err();
    assert!(
        err.message.contains("não foi encontrada"),
        "{}",
        err.message
    );
    let rs = run(&conn, "select nome from dbeasy_ed where id = 1")
        .await
        .unwrap()
        .result
        .unwrap();
    assert_eq!(rs.rows[0][0], Value::Text("x".into()));

    // Com transação ativa: entra nela; falha não a deixa em erro.
    conn.transaction(TxCommand::Begin).await.unwrap();
    conn.apply_edits(vec![edit("1", Some("t"))]).await.unwrap();
    conn.apply_edits(vec![edit("99", Some("z"))])
        .await
        .unwrap_err();
    assert_eq!(conn.tx_state(), TxState::Active);
    assert_eq!(
        conn.summary().await.unwrap().statements,
        vec!["id 1 → 1 linha (grade)".to_string()]
    );
    conn.transaction(TxCommand::Rollback).await.unwrap();
    let rs = run(&conn, "select nome from dbeasy_ed where id = 1")
        .await
        .unwrap()
        .result
        .unwrap();
    assert_eq!(rs.rows[0][0], Value::Text("x".into()));
}

#[tokio::test]
async fn selected_rows_are_deleted_by_primary_key() {
    use connection::{EditStatement, TxCommand, TxState};
    use dbeasy::data_edit;
    use std::collections::BTreeSet;
    let Some(cfg) = config(false) else { return };
    let conn = connection::connect(cfg).await.unwrap();
    let setup = "drop table if exists dbeasy_del; \
                 create table dbeasy_del (a int, b text, nome text, primary key (a, b)); \
                 insert into dbeasy_del values (1, 'x', 'um'), (2, 'y', 'dois'), (3, 'z', 'tres')";
    conn.execute(setup.into(), all(setup)).await.unwrap();
    let catalog = conn.catalog().await.unwrap();
    let select = |conn| async move {
        run(conn, "select nome, b, a from dbeasy_del order by a")
            .await
            .unwrap()
            .result
            .unwrap()
    };
    let statements = |rs: &dbeasy::query::ResultSet, rows: &[usize]| {
        let target = data_edit::edit_target(rs, &catalog).unwrap();
        data_edit::build_deletes(&target, rs, &rows.iter().copied().collect::<BTreeSet<_>>())
            .unwrap()
            .into_iter()
            .map(|d| EditStatement {
                sql: d.sql,
                params: d.params,
                label: d.key,
            })
            .collect::<Vec<_>>()
    };

    // Exclui a 1ª e a 3ª linhas pela chave composta, fora de transação.
    let rs = select(&conn).await;
    conn.apply_edits(statements(&rs, &[0, 2])).await.unwrap();
    assert_eq!(conn.tx_state(), TxState::Idle);
    let rs = select(&conn).await;
    assert_eq!(
        rs.rows,
        vec![vec![
            Value::Text("dois".into()),
            Value::Text("y".into()),
            Value::Int(2)
        ]]
    );

    // Linha já removida por outra sessão: o lote inteiro é desfeito.
    let stale = statements(&rs, &[0]);
    run(&conn, "insert into dbeasy_del values (4, 'w', 'quatro')")
        .await
        .unwrap();
    let rs = select(&conn).await;
    let mut batch = statements(&rs, &[1]);
    run(&conn, "delete from dbeasy_del where a = 2")
        .await
        .unwrap();
    batch.extend(stale);
    let err = conn.apply_edits(batch).await.unwrap_err();
    assert!(
        err.message.contains("não foi encontrada"),
        "{}",
        err.message
    );
    assert_eq!(select(&conn).await.rows.len(), 1);

    // Na transação: só some de vez com o COMMIT; ROLLBACK traz de volta.
    conn.transaction(TxCommand::Begin).await.unwrap();
    let rs = select(&conn).await;
    conn.apply_edits(statements(&rs, &[0])).await.unwrap();
    assert!(select(&conn).await.rows.is_empty());
    conn.transaction(TxCommand::Rollback).await.unwrap();
    assert_eq!(select(&conn).await.rows.len(), 1);
}

#[tokio::test]
async fn probe_measures_latency_and_reads_server_info() {
    let Some(cfg) = config(true) else { return };
    let user = cfg.user.clone();
    let probe = connection::probe(cfg).await.expect("testa a conexão");
    assert_eq!(probe.round_trips.len(), 5);
    assert!(probe.latency().is_some());
    assert!(!probe.tls);
    let server = probe.server.expect("dados do servidor");
    assert_eq!(server.user, user);
    assert!(server.read_only);
    assert!(!server.replica);
    assert!(server.connections >= 1 && server.max_connections >= server.connections);
    assert!(server.version.starts_with(char::is_numeric), "{}", server.version);

    let mut bad = config(false).unwrap();
    bad.password = Secret::new(Some("senha-errada".into()));
    assert!(connection::probe(bad).await.is_err());
}
