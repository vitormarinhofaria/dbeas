# Conexões TLS

Decisões do usuário (2026-10-13): padrão `prefer` (como o psql); modos do libpq
+ `sslRootCert`, sem certificado de cliente; biblioteca **rustls**.

## Implementação — `src/tls.rs`

- rustls 0.23 com o provedor **ring** (não `aws-lc-rs`: o aws-lc incorpora código
  sob a licença OpenSSL antiga, incompatível com GPL-3) e `tokio-postgres-rustls`.
- `verification(modo, tem_sslRootCert)`:
  - `prefer`/`require` → sem verificação (`NoVerification`: aceita o certificado,
    mas confere a assinatura do handshake);
  - `require` **com** `sslRootCert` → verifica a CA (igual ao libpq);
  - `verify-ca` → `WebPkiServerVerifier`, ignorando só `NotValidForName`;
  - `verify-full` → `WebPkiServerVerifier` completo.
- `negotiation(modo)`: só `prefer` usa `SslMode::Prefer` do tokio-postgres (aceita
  servidor sem TLS); `require`/`verify-*` exigem TLS.
- Raízes: PEM do `sslRootCert` (um ou mais certificados) ou
  `rustls-native-certs` (CAs do sistema).
- Os verificadores marcam um `AtomicBool` quando o handshake acontece:
  `ConnectionHandle::uses_tls()` diz se a conexão está cifrada (em `prefer` o
  servidor pode recusar).
- O cancelamento de consultas abre uma conexão nova com o mesmo modo TLS.
- Erros: o tokio-postgres mostra só "error performing TLS handshake"; a causa
  (ex.: `UnknownIssuer`) está na cadeia `source()`. `tls::error_chain` monta o texto
  completo e `tls::explain_error` acrescenta uma dica (CA não confiável, nome
  divergente, servidor sem TLS, certificado expirado).

## Interface

Selo no cabeçalho do workspace conectado: "TLS verificado" (verify-full), "TLS
(CA verificada)" (verify-ca / require com CA), "TLS sem verificação"
(prefer/require) ou "sem TLS" — com tooltip explicando o risco e a configuração.

## Testes

- Unitários em `src/tls.rs` e `workspace/config.rs`.
- Integração em `tests/tls.rs` (todos os modos, CA certa/errada, CAs do sistema,
  fallback do `prefer`, nome divergente via `hostaddr`). Rodam com
  **`scripts/test-tls.sh`**: cria uma CA de teste, a rede `dbeasy-test-net`
  (172.31.250.0/24) e dois containers PostgreSQL 17 **sem porta mapeada** (com TLS
  em .10, sem TLS em .11) e executa os testes. `scripts/test-tls.sh --clean`
  remove tudo.
- O Docker do ambiente é **rootless**: os IPs dos containers só existem no
  namespace de rede do RootlessKit. O script roda o binário de teste com
  `nsenter -U --preserve-credentials -n -t $(cat $XDG_RUNTIME_DIR/dockerd-rootless/child_pid)`
  (o PID de `docker.pid` não serve: o namespace é do processo filho). Fora do
  rootless, roda direto.
- O app também pode ser executado no namespace para teste manual:
  `nsenter ... ./target/debug/dbeasy` (o Wayland continua acessível).
