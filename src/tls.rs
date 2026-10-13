// SPDX-FileCopyrightText: 2026 Vitor Marinho Faria
// SPDX-License-Identifier: GPL-3.0-or-later

//! TLS das conexões PostgreSQL (rustls com o provedor criptográfico `ring`).
//!
//! Semântica do `sslmode` do libpq:
//! - `disable`: sem TLS;
//! - `prefer`: tenta TLS e, se o servidor não suportar, conecta sem; não
//!   verifica o certificado;
//! - `require`: exige TLS sem verificar o certificado — com `sslRootCert`,
//!   comporta-se como `verify-ca` (igual ao libpq);
//! - `verify-ca`: exige TLS e uma cadeia emitida por CA confiável;
//! - `verify-full`: como `verify-ca` e o certificado precisa corresponder ao host.
//!
//! Sem verificação, a conexão é cifrada mas não protege contra intermediários;
//! a assinatura do handshake continua sendo checada. As CAs vêm de
//! `sslRootCert` ou, sem ele, do repositório do sistema.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use rustls::client::WebPkiServerVerifier;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::WebPkiSupportedAlgorithms;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{
    CertificateError, ClientConfig, DigitallySignedStruct, Error, RootCertStore, SignatureScheme,
};
use tokio_postgres_rustls::MakeRustlsConnect;

use crate::workspace::SslMode;

/// Como o certificado do servidor é verificado.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verification {
    None,
    Ca,
    Full,
}

/// Verificação efetiva para o modo e a presença de `sslRootCert`. `None`
/// para `disable`.
pub fn verification(mode: SslMode, has_root_cert: bool) -> Option<Verification> {
    match mode {
        SslMode::Disable => None,
        SslMode::Prefer => Some(Verification::None),
        SslMode::Require if has_root_cert => Some(Verification::Ca),
        SslMode::Require => Some(Verification::None),
        SslMode::VerifyCa => Some(Verification::Ca),
        SslMode::VerifyFull => Some(Verification::Full),
    }
}

/// Modo de negociação do tokio-postgres: só `prefer` aceita cair para texto
/// puro.
pub fn negotiation(mode: SslMode) -> tokio_postgres::config::SslMode {
    match mode {
        SslMode::Disable => tokio_postgres::config::SslMode::Disable,
        SslMode::Prefer => tokio_postgres::config::SslMode::Prefer,
        _ => tokio_postgres::config::SslMode::Require,
    }
}

/// Conector TLS e um indicador que fica `true` quando o handshake TLS de fato
/// acontece (em `prefer`, o servidor pode recusar TLS).
pub fn connector(
    mode: SslMode,
    root_cert: Option<&Path>,
) -> Result<(MakeRustlsConnect, Arc<AtomicBool>), String> {
    let verification = verification(mode, root_cert.is_some()).ok_or("TLS desabilitado")?;
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let negotiated = Arc::new(AtomicBool::new(false));

    let verifier: Arc<dyn ServerCertVerifier> = match verification {
        Verification::None => Arc::new(NoVerification {
            algorithms: provider.signature_verification_algorithms,
            negotiated: negotiated.clone(),
        }),
        Verification::Ca | Verification::Full => {
            let roots = root_store(root_cert)?;
            let webpki =
                WebPkiServerVerifier::builder_with_provider(Arc::new(roots), provider.clone())
                    .build()
                    .map_err(|e| format!("configuração de verificação TLS inválida: {e}"))?;
            Arc::new(WebPki {
                inner: webpki,
                check_name: verification == Verification::Full,
                negotiated: negotiated.clone(),
            })
        }
    };

    let config = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| format!("configuração TLS inválida: {e}"))?
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_no_client_auth();
    Ok((MakeRustlsConnect::new(config), negotiated))
}

/// CAs confiáveis: o arquivo `sslRootCert` (PEM, um ou mais certificados) ou,
/// sem ele, as do sistema.
fn root_store(root_cert: Option<&Path>) -> Result<RootCertStore, String> {
    let mut store = RootCertStore::empty();
    match root_cert {
        Some(path) => {
            let certs: Vec<CertificateDer<'static>> = CertificateDer::pem_file_iter(path)
                .map_err(|e| format!("não foi possível ler sslRootCert {}: {e}", path.display()))?
                .collect::<Result<_, _>>()
                .map_err(|e| format!("sslRootCert {} não é um PEM válido: {e}", path.display()))?;
            if certs.is_empty() {
                return Err(format!(
                    "sslRootCert {} não contém certificados",
                    path.display()
                ));
            }
            for cert in certs {
                store
                    .add(cert)
                    .map_err(|e| format!("certificado inválido em {}: {e}", path.display()))?;
            }
        }
        None => {
            let native = rustls_native_certs::load_native_certs();
            store.add_parsable_certificates(native.certs);
            if store.is_empty() {
                return Err(
                    "nenhuma CA do sistema encontrada; informe sslRootCert no dbeas.json".into(),
                );
            }
        }
    }
    Ok(store)
}

/// Aceita qualquer certificado (modos sem verificação), mas confere a
/// assinatura do handshake com a chave do certificado apresentado.
#[derive(Debug)]
struct NoVerification {
    algorithms: WebPkiSupportedAlgorithms,
    negotiated: Arc<AtomicBool>,
}

impl ServerCertVerifier for NoVerification {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, Error> {
        self.negotiated.store(true, Ordering::Relaxed);
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algorithms.supported_schemes()
    }
}

/// Verificação pela cadeia de CAs; em `verify-ca`, ignora apenas a
/// divergência entre o nome do certificado e o host.
#[derive(Debug)]
struct WebPki {
    inner: Arc<WebPkiServerVerifier>,
    check_name: bool,
    negotiated: Arc<AtomicBool>,
}

impl ServerCertVerifier for WebPki {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, Error> {
        self.negotiated.store(true, Ordering::Relaxed);
        match self.inner.verify_server_cert(
            end_entity,
            intermediates,
            server_name,
            ocsp_response,
            now,
        ) {
            Err(Error::InvalidCertificate(
                CertificateError::NotValidForName | CertificateError::NotValidForNameContext { .. },
            )) if !self.check_name => Ok(ServerCertVerified::assertion()),
            other => other,
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        self.inner.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        self.inner.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.inner.supported_verify_schemes()
    }
}

/// Texto do erro com toda a cadeia de causas: o tokio-postgres mostra só
/// "error performing TLS handshake" e guarda o motivo real (ex.:
/// `UnknownIssuer`) como causa.
pub fn error_chain(error: &dyn std::error::Error) -> String {
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        let cause_text = cause.to_string();
        if !text.contains(&cause_text) {
            text.push_str(": ");
            text.push_str(&cause_text);
        }
        source = cause.source();
    }
    text
}

/// Mensagem de falha de conexão com uma dica para os erros de TLS mais comuns.
/// Mantém o texto original do driver (que não contém a senha).
pub fn explain_error(message: &str, mode: SslMode) -> String {
    let hint = if message.contains("UnknownIssuer") || message.contains("unknown issuer") {
        Some(
            "o certificado do servidor não foi emitido por uma CA confiável; configure sslRootCert com a CA do servidor",
        )
    } else if message.contains("NotValidForName") || message.contains("not valid for name") {
        Some(
            "o certificado do servidor não corresponde a dbHost; ajuste dbHost para o nome do certificado ou use sslMode verify-ca",
        )
    } else if message.contains("server does not support TLS") {
        Some(
            "o servidor não aceita TLS; use sslMode prefer ou disable, ou habilite ssl no PostgreSQL",
        )
    } else if message.contains("Expired") || message.contains("expired") {
        Some("o certificado do servidor está expirado")
    } else {
        None
    };
    match hint {
        Some(hint) => format!("{hint} (sslMode {}): {message}", mode.as_str()),
        None => message.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verification_follows_libpq() {
        assert_eq!(verification(SslMode::Disable, true), None);
        assert_eq!(
            verification(SslMode::Prefer, true),
            Some(Verification::None)
        );
        assert_eq!(
            verification(SslMode::Require, false),
            Some(Verification::None)
        );
        assert_eq!(verification(SslMode::Require, true), Some(Verification::Ca));
        assert_eq!(
            verification(SslMode::VerifyCa, false),
            Some(Verification::Ca)
        );
        assert_eq!(
            verification(SslMode::VerifyFull, false),
            Some(Verification::Full)
        );
    }

    #[test]
    fn only_prefer_falls_back_to_plain_text() {
        use tokio_postgres::config::SslMode as Pg;
        assert_eq!(negotiation(SslMode::Prefer), Pg::Prefer);
        assert_eq!(negotiation(SslMode::Require), Pg::Require);
        assert_eq!(negotiation(SslMode::VerifyFull), Pg::Require);
        assert_eq!(negotiation(SslMode::Disable), Pg::Disable);
    }

    #[test]
    fn bad_root_cert_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        let empty = dir.path().join("vazio.pem");
        std::fs::write(&empty, "não é um certificado").unwrap();
        let err = connector(SslMode::VerifyFull, Some(&empty)).err().unwrap();
        assert!(
            err.contains("não contém certificados") || err.contains("PEM"),
            "{err}"
        );
        let err = connector(
            SslMode::VerifyFull,
            Some(&dir.path().join("nao-existe.pem")),
        )
        .err()
        .unwrap();
        assert!(err.contains("não foi possível ler"), "{err}");
    }

    #[test]
    fn hints_for_common_errors() {
        let e = explain_error(
            "error performing TLS handshake: invalid peer certificate: UnknownIssuer",
            SslMode::VerifyFull,
        );
        assert!(e.contains("sslRootCert") && e.contains("UnknownIssuer"));
        assert_eq!(
            explain_error("password authentication failed", SslMode::Prefer),
            "password authentication failed"
        );
    }
}
