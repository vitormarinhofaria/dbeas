#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Vitor Marinho Faria
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Testes de TLS (tests/tls.rs) contra PostgreSQL em containers, sem mapear
# portas. Cria uma CA de teste, um servidor com TLS (172.31.250.10) e um sem
# (172.31.250.11) numa rede Docker própria e roda os testes. Com Docker
# rootless, os IPs dos containers só são alcançáveis dentro do namespace de
# rede do RootlessKit; o script entra nele com nsenter.
#
# Uso: scripts/test-tls.sh            # cria o ambiente (se preciso) e testa
#      scripts/test-tls.sh --clean    # remove containers, rede e certificados
set -euo pipefail

NET=dbeas-test-net
SUBNET=172.31.250.0/24
TLS_IP=172.31.250.10
PLAIN_IP=172.31.250.11
DIR="${DBEAS_TLS_DIR:-${XDG_RUNTIME_DIR:-/tmp}/dbeas-tls-test}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"

if [[ "${1:-}" == "--clean" ]]; then
    docker rm -f dbeas-ssl-test dbeas-plain-test >/dev/null 2>&1 || true
    docker network rm "$NET" >/dev/null 2>&1 || true
    rm -rf "$DIR"
    echo "ambiente de teste TLS removido"
    exit 0
fi

mkdir -p "$DIR"
if [[ ! -f "$DIR/server.crt" ]]; then
    (
        cd "$DIR"
        openssl req -x509 -newkey rsa:2048 -nodes -days 30 -keyout ca.key -out ca.crt -subj "/CN=DBeas Test CA" 2>/dev/null
        openssl req -newkey rsa:2048 -nodes -keyout server.key -out server.csr -subj "/CN=$TLS_IP" 2>/dev/null
        printf 'subjectAltName=IP:%s\nbasicConstraints=CA:FALSE\nkeyUsage=digitalSignature,keyEncipherment\nextendedKeyUsage=serverAuth\n' "$TLS_IP" > ext.cnf
        openssl x509 -req -in server.csr -CA ca.crt -CAkey ca.key -CAcreateserial -days 30 -out server.crt -extfile ext.cnf 2>/dev/null
        openssl req -x509 -newkey rsa:2048 -nodes -days 30 -keyout other-ca.key -out other-ca.crt -subj "/CN=Outra CA" 2>/dev/null
        chmod 644 ca.crt server.crt other-ca.crt
        chmod 600 server.key
    )
fi

docker network inspect "$NET" >/dev/null 2>&1 || docker network create --subnet "$SUBNET" "$NET" >/dev/null

# Um servidor criado com certificados de outro diretório não confiaria na CA
# atual: recria.
if docker inspect dbeas-ssl-test >/dev/null 2>&1; then
    mounted=$(docker inspect -f '{{range .Mounts}}{{if eq .Destination "/tls/server.crt"}}{{.Source}}{{end}}{{end}}' dbeas-ssl-test)
    if [[ "$mounted" != "$DIR/server.crt" ]]; then
        docker rm -f dbeas-ssl-test >/dev/null
    fi
fi
if ! docker inspect dbeas-ssl-test >/dev/null 2>&1; then
    docker run -d --name dbeas-ssl-test --network "$NET" --ip "$TLS_IP" \
        -e POSTGRES_USER=dbeas -e POSTGRES_PASSWORD=dbeas -e POSTGRES_DB=dbeas_tls \
        -v "$DIR/server.crt:/tls/server.crt:ro" -v "$DIR/server.key:/tls/server.key.src:ro" \
        --entrypoint bash postgres:17 -c 'cp /tls/server.key.src /tmp/server.key && chown postgres /tmp/server.key && chmod 600 /tmp/server.key && exec docker-entrypoint.sh postgres -c ssl=on -c ssl_cert_file=/tls/server.crt -c ssl_key_file=/tmp/server.key' >/dev/null
fi
if ! docker inspect dbeas-plain-test >/dev/null 2>&1; then
    docker run -d --name dbeas-plain-test --network "$NET" --ip "$PLAIN_IP" \
        -e POSTGRES_USER=dbeas -e POSTGRES_PASSWORD=dbeas -e POSTGRES_DB=dbeas_tls postgres:17 >/dev/null
fi
docker start dbeas-ssl-test dbeas-plain-test >/dev/null

# Executável dos testes e o namespace de rede onde os IPs são alcançáveis.
BIN=$(cd "$ROOT" && cargo test --test tls --no-run --message-format=json 2>/dev/null \
    | python3 -c 'import json,sys
for line in sys.stdin:
    m = json.loads(line)
    if m.get("reason") == "compiler-artifact" and m["target"]["name"] == "tls" and m.get("executable"):
        print(m["executable"])' | tail -1)
CHILD_PID_FILE="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}/dockerd-rootless/child_pid"
run() {
    if [[ -f "$CHILD_PID_FILE" ]]; then
        nsenter -U --preserve-credentials -n -t "$(cat "$CHILD_PID_FILE")" "$@"
    else
        "$@"
    fi
}

# Espera os servidores aceitarem conexões.
for ip in "$TLS_IP" "$PLAIN_IP"; do
    for _ in $(seq 1 30); do
        run bash -c "echo > /dev/tcp/$ip/5432" 2>/dev/null && break
        sleep 1
    done
done
sleep 2

DBEAS_TEST_TLS="$TLS_IP,$PLAIN_IP,$DIR/ca.crt,$DIR/other-ca.crt" run "$BIN" "$@"
