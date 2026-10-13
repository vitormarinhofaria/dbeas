// SPDX-FileCopyrightText: 2026 Vitor Marinho Faria
// SPDX-License-Identifier: GPL-3.0-or-later

//! Testes de TLS contra servidores reais. Rodam apenas com `DBEAS_TEST_TLS`
//! definido como `ip_com_tls,ip_sem_tls,ca.crt,outra-ca.crt`. Procedimento
//! (containers sem porta mapeada): ai-content/knowledge-base/arquitetura/tls.md

use std::net::IpAddr;
use std::path::PathBuf;

use dbeas::connection;
use dbeas::query::Value;
use dbeas::tls;
use dbeas::workspace::config::Secret;
use dbeas::workspace::{ResolvedConfig, SslMode};

struct Env {
    tls_host: String,
    plain_host: String,
    ca: PathBuf,
    other_ca: PathBuf,
}

fn env() -> Option<Env> {
    let raw = std::env::var("DBEAS_TEST_TLS").ok()?;
    let parts: Vec<&str> = raw.split(',').collect();
    let [tls_host, plain_host, ca, other_ca] = parts.as_slice() else {
        panic!("DBEAS_TEST_TLS malformado")
    };
    Some(Env {
        tls_host: tls_host.to_string(),
        plain_host: plain_host.to_string(),
        ca: PathBuf::from(ca),
        other_ca: PathBuf::from(other_ca),
    })
}

fn config(host: &str, ssl_mode: SslMode, root: Option<&PathBuf>) -> ResolvedConfig {
    ResolvedConfig {
        name: "tls".into(),
        host: host.into(),
        port: 5432,
        user: "dbeas".into(),
        password: Secret::new(Some("dbeas".into())),
        db_name: "dbeas_tls".into(),
        read_only: false,
        require_transaction: false,
        sql_files_dir: None,
        ssl_mode,
        ssl_root_cert: root.cloned(),
    }
}

/// Conecta e confirma, pelo lado do servidor, se a sessão usa TLS.
async fn server_side_tls(cfg: ResolvedConfig) -> Result<(bool, bool), String> {
    let conn = connection::connect(cfg).await?;
    let sql = "select ssl from pg_stat_ssl where pid = pg_backend_pid()".to_string();
    let len = sql.len();
    let rs = conn
        .execute(sql, vec![0..len])
        .await
        .map_err(|e| e.message)?
        .result
        .unwrap();
    Ok((rs.rows[0][0] == Value::Bool(true), conn.uses_tls()))
}

#[tokio::test]
async fn modes_against_tls_and_plain_servers() {
    let Some(e) = env() else { return };

    // disable: nunca usa TLS.
    assert_eq!(
        server_side_tls(config(&e.tls_host, SslMode::Disable, None))
            .await
            .unwrap(),
        (false, false)
    );

    // prefer: TLS quando o servidor oferece, texto puro quando não.
    assert_eq!(
        server_side_tls(config(&e.tls_host, SslMode::Prefer, None))
            .await
            .unwrap(),
        (true, true)
    );
    assert_eq!(
        server_side_tls(config(&e.plain_host, SslMode::Prefer, None))
            .await
            .unwrap(),
        (false, false)
    );

    // require: exige TLS, mas não verifica o certificado (CA de teste não é do sistema).
    assert_eq!(
        server_side_tls(config(&e.tls_host, SslMode::Require, None))
            .await
            .unwrap(),
        (true, true)
    );
    let err = server_side_tls(config(&e.plain_host, SslMode::Require, None))
        .await
        .unwrap_err();
    assert!(err.contains("não aceita TLS"), "{err}");

    // require + sslRootCert se comporta como verify-ca (igual ao libpq).
    assert!(
        server_side_tls(config(&e.tls_host, SslMode::Require, Some(&e.ca)))
            .await
            .is_ok()
    );
    let err = server_side_tls(config(&e.tls_host, SslMode::Require, Some(&e.other_ca)))
        .await
        .unwrap_err();
    assert!(err.contains("CA confiável"), "{err}");

    // verify-ca / verify-full com a CA correta (certificado com SAN do IP).
    assert_eq!(
        server_side_tls(config(&e.tls_host, SslMode::VerifyCa, Some(&e.ca)))
            .await
            .unwrap(),
        (true, true)
    );
    assert_eq!(
        server_side_tls(config(&e.tls_host, SslMode::VerifyFull, Some(&e.ca)))
            .await
            .unwrap(),
        (true, true)
    );

    // CA errada e CAs do sistema não confiam no certificado de teste.
    let err = server_side_tls(config(&e.tls_host, SslMode::VerifyCa, Some(&e.other_ca)))
        .await
        .unwrap_err();
    assert!(err.contains("CA confiável"), "{err}");
    let err = server_side_tls(config(&e.tls_host, SslMode::VerifyFull, None))
        .await
        .unwrap_err();
    assert!(err.contains("CA confiável"), "{err}");
}

/// Nome divergente: conecta no IP certo, mas apresenta outro nome ao TLS
/// (`hostaddr`). verify-full recusa; verify-ca aceita.
#[tokio::test]
async fn verify_full_checks_the_host_name() {
    let Some(e) = env() else { return };
    let ip: IpAddr = e.tls_host.parse().expect("IP do servidor TLS");

    let attempt = |mode: SslMode| {
        let ca = e.ca.clone();
        async move {
            let mut pg = tokio_postgres::Config::new();
            pg.host("outro-nome.dbeas.test")
                .hostaddr(ip)
                .port(5432)
                .user("dbeas")
                .password("dbeas")
                .dbname("dbeas_tls")
                .ssl_mode(tls::negotiation(mode));
            let (connector, _) = tls::connector(mode, Some(&ca)).unwrap();
            pg.connect(connector).await.map(|(client, conn)| {
                tokio::spawn(conn);
                client
            })
        }
    };

    let err = attempt(SslMode::VerifyFull)
        .await
        .err()
        .expect("verify-full deve recusar");
    let message = tls::explain_error(&tls::error_chain(&err), SslMode::VerifyFull);
    assert!(message.contains("não corresponde a dbHost"), "{message}");
    let client = attempt(SslMode::VerifyCa)
        .await
        .expect("verify-ca ignora o nome");
    let row = client
        .query_one(
            "select ssl from pg_stat_ssl where pid = pg_backend_pid()",
            &[],
        )
        .await
        .unwrap();
    assert!(row.get::<_, bool>(0));
}
