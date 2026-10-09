//! Testes contra um PostgreSQL real. Executados apenas quando
//! `DBEAS_TEST_PG` estiver definido no formato `host:porta:usuario:senha:banco`.
//!
//! Exemplo:
//! docker run -d --name dbeas-test-pg -e POSTGRES_PASSWORD=dbeas -e POSTGRES_USER=dbeas \
//!   -e POSTGRES_DB=dbeas_dev -p 127.0.0.1:55432:5432 postgres:17
//! DBEAS_TEST_PG=127.0.0.1:55432:dbeas:dbeas:dbeas_dev cargo test --test postgres

use dbeas::connection;
use dbeas::query::{Value, split};
use dbeas::workspace::ResolvedConfig;
use dbeas::workspace::config::Secret;

fn config(read_only: bool) -> Option<ResolvedConfig> {
    let raw = std::env::var("DBEAS_TEST_PG").ok()?;
    let parts: Vec<&str> = raw.split(':').collect();
    let [host, port, user, pass, db] = parts.as_slice() else {
        panic!("DBEAS_TEST_PG malformado")
    };
    Some(ResolvedConfig {
        name: "teste".into(),
        host: host.to_string(),
        port: port.parse().unwrap(),
        user: user.to_string(),
        password: Secret::new(Some(pass.to_string())),
        db_name: db.to_string(),
        read_only,
        require_transaction: false,
        sql_files_dir: None,
    })
}

fn all(sql: &str) -> Vec<std::ops::Range<usize>> {
    split::statements(sql)
}

#[tokio::test]
async fn executes_queries_pages_and_reads_catalog() {
    let Some(cfg) = config(false) else { return };
    let conn = connection::connect(cfg).await.expect("conecta");

    let setup = "drop table if exists dbeas_t; \
                 create table dbeas_t (id serial primary key, nome text, valor numeric(10,2), criado timestamptz default now()); \
                 insert into dbeas_t (nome, valor) select 'n' || g, g * 1.5 from generate_series(1, 1200) g; \
                 insert into dbeas_t (nome, valor) values (null, null), ('', 0)";
    conn.execute(setup.into(), all(setup)).await.expect("setup");

    let sql = "select id, nome, valor from dbeas_t order by id".to_string();
    let len = sql.len();
    let out = conn.execute(sql, vec![0..len]).await.expect("select");
    let rs = out.result.expect("resultado");
    assert_eq!(rs.columns.len(), 3);
    assert_eq!(rs.columns[2].type_name, "numeric");
    assert!(rs.columns[0].table_oid.is_some());
    assert_eq!(rs.rows.len(), dbeas::query::PAGE_SIZE);
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

    let sql = "select nome from dbeas_t where id > 1200 order by id".to_string();
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
        .find("public", "dbeas_t")
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
    let setup = "create sequence if not exists dbeas_seq".to_string();
    let len = setup.len();
    rw.execute(setup, vec![0..len]).await.unwrap();
    let conn = connection::connect(cfg).await.unwrap();

    let sql = "create table nao_pode (a int)".to_string();
    let len = sql.len();
    let err = conn.execute(sql, vec![0..len]).await.unwrap_err();
    assert!(err.message.contains("somente leitura"), "{}", err.message);

    // Segunda camada: a sessão do servidor está em modo somente leitura.
    let sql = "select nextval('dbeas_seq')".to_string();
    let len = sql.len();
    let err = conn.execute(sql, vec![0..len]).await.unwrap_err();
    assert_eq!(err.code.as_deref(), Some("25006"), "{}", err.message);
}

#[tokio::test]
async fn transformed_queries_run_and_keep_their_meaning() {
    use dbeas::query::transform::{self, Direction};
    let Some(cfg) = config(false) else { return };
    let conn = connection::connect(cfg).await.unwrap();
    let run = |sql: String| {
        let conn = conn.clone();
        async move {
            let len = sql.len();
            conn.execute(sql, vec![0..len]).await.expect("executa").result.expect("linhas")
        }
    };

    let base = "SELECT g AS id, -- identificador\n       'n' || g AS nome,\n       g * 2 AS dobro\nFROM generate_series(1, 5) g\nLIMIT 4";
    let sorted = transform::set_order(base, "dobro", Some(Direction::Desc)).unwrap();
    let rs = run(sorted.clone()).await;
    assert_eq!(rs.rows[0][0], Value::Int(5));
    assert_eq!(rs.rows.len(), 4);

    let hidden = transform::remove_item(&sorted, 1).unwrap();
    let rs = run(hidden.clone()).await;
    assert_eq!(rs.columns.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["id", "dobro"]);

    let reordered = transform::reorder_items(&hidden, &[1, 0]).unwrap();
    let rs = run(reordered).await;
    assert_eq!(rs.columns.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["dobro", "id"]);
    assert_eq!(rs.rows[0][0], Value::Int(10));
}
