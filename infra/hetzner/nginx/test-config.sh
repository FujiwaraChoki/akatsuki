#!/usr/bin/env bash
# Runs `nginx -t` on the Akatsuki vhosts in a throwaway prefix, the way the shared host includes them
# (inside http{}, next to a stand-in for another site that also listens on 80/443).
#
#   infra/hetzner/nginx/test-config.sh [path/to/nginx]
#
# Certificates are self-signed stand-ins; /etc/letsencrypt, /var/log/nginx and /opt/akatsuki paths are
# rewritten into the temp prefix. Nothing outside the temp directory is read or written. Used by CI (the
# Ubuntu 24.04 runner installs the same nginx 1.24 as the server) and locally with any nginx binary.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
nginx_bin="${1:-${NGINX:-nginx}}"
if ! command -v "$nginx_bin" >/dev/null 2>&1 && [ ! -x "$nginx_bin" ]; then
  echo "nginx binary not found: $nginx_bin" >&2
  exit 2
fi

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
mkdir -p "$work"/{conf/sites,logs,acme,certs,tmp}

for host in sync.akatsuki.vc api.akatsuki.vc other.example; do
  mkdir -p "$work/certs/$host"
  openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes -days 1 \
    -subj "/CN=$host" -keyout "$work/certs/$host/privkey.pem" -out "$work/certs/$host/fullchain.pem" \
    >/dev/null 2>&1
done

rewrite() {
  sed -e "s#/etc/letsencrypt/live/#$work/certs/#g" \
      -e "s#/var/log/nginx/#$work/logs/#g" \
      -e "s#/opt/akatsuki/acme#$work/acme#g" \
      -e "s#listen 80;#listen 127.0.0.1:18080;#g" \
      -e "s#listen 443 ssl http2;#listen 127.0.0.1:18443 ssl http2;#g" "$1"
}

# Every compiled-in temp path must point into the prefix, or `nginx -t` tries the distro's /var/lib/nginx.
temp_paths() {
  local v; v="$("$nginx_bin" -V 2>&1)"
  for m in fastcgi uwsgi scgi; do
    case "$v" in *"--without-http_${m}_module"*) ;; *) echo "  ${m}_temp_path $work/tmp/$m;" ;; esac
  done
}

check() {
  local label="$1"; shift
  : >"$work/conf/sites/.keep"
  rm -f "$work"/conf/sites/*.conf
  for f in "$@"; do rewrite "$f" >"$work/conf/sites/$(basename "$f")"; done
  # A stand-in for an unrelated site on the same sockets, so listen-option conflicts show up here.
  cat >"$work/conf/sites/00-other.conf" <<EOF
server { listen 127.0.0.1:18080 default_server; server_name other.example; return 404; }
server {
  listen 127.0.0.1:18443 ssl http2 default_server; server_name other.example;
  ssl_certificate $work/certs/other.example/fullchain.pem;
  ssl_certificate_key $work/certs/other.example/privkey.pem;
  return 404;
}
EOF
  cat >"$work/conf/nginx.conf" <<EOF
worker_processes 1;
pid $work/tmp/nginx.pid;
error_log $work/logs/error.log warn;
events { worker_connections 64; }
http {
  default_type application/octet-stream;
  access_log $work/logs/access.log;
  client_body_temp_path $work/tmp/body;
  proxy_temp_path $work/tmp/proxy;
$(temp_paths)
  include $work/conf/sites/*.conf;
}
EOF
  echo "== nginx -t: $label"
  "$nginx_bin" -t -p "$work" -c "$work/conf/nginx.conf"
}

check "full vhosts" "$here/sync.akatsuki.vc.conf" "$here/api.akatsuki.vc.conf"
check "ACME bootstrap vhosts" "$here/acme-bootstrap/sync.akatsuki.vc.conf" "$here/acme-bootstrap/api.akatsuki.vc.conf"
check "bootstrap sync + full api" "$here/acme-bootstrap/sync.akatsuki.vc.conf" "$here/api.akatsuki.vc.conf"
echo "nginx configuration OK"
