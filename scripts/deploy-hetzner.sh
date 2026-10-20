#!/usr/bin/env bash
# Deploys akatsuki-sync and akatsuki-api to the shared Hetzner host (docs/DEPLOY.md, BRIEF "Hosting").
#
# DRY RUN BY DEFAULT: without --apply it prints the plan and contacts nothing. Run it only with the owner's
# go-ahead in the current conversation.
#
#   scripts/deploy-hetzner.sh                     # print the plan (no network)
#   scripts/deploy-hetzner.sh --preflight         # read-only checks on the server (ssh, changes nothing)
#   scripts/deploy-hetzner.sh --apply             # deploy
#   scripts/deploy-hetzner.sh --apply --rollback  # switch back to the previous release's images
#
# What it may change on the server, and nothing else:
#   /opt/akatsuki/**                                        compose file, sources, release files, spool, ACME webroot
#   $NGINX_SITES_DIR/{sync,api}.akatsuki.vc.conf            the two vhosts (+ their sites-enabled symlinks)
#   /etc/letsencrypt/** for sync/api.akatsuki.vc only       via certbot, first deploy only
#   docker: the akatsuki-sync / akatsuki-api containers, the akatsuki-net network, the akatsuki-sync-spool
#           volume and akatsuki-{sync,api}:<tag> images
# It reloads nginx (never restarts it) only after `nginx -t` passed, and it never stops, restarts or prunes
# anything else. Env files (/opt/akatsuki/{sync,api}.env) are never read by this script; --upload-env copies
# them byte for byte with mode 0600.
set -euo pipefail

# ---------------------------------------------------------------- settings (override via environment)
SSH_HOST="${AKATSUKI_SSH_HOST:-hetzner}"
REMOTE_SUDO="${AKATSUKI_REMOTE_SUDO:-sudo}"          # empty when the ssh user is root
REMOTE_ROOT=/opt/akatsuki
NGINX_SITES_DIR="${AKATSUKI_NGINX_SITES_DIR:-/etc/nginx/sites-available}"
NGINX_ENABLED_DIR="${AKATSUKI_NGINX_ENABLED_DIR:-/etc/nginx/sites-enabled}"   # empty for a conf.d layout
NGINX_RELOAD="${AKATSUKI_NGINX_RELOAD:-systemctl reload nginx}"
CERT_METHOD="${AKATSUKI_CERT_METHOD:-webroot}"       # webroot | nginx (certbot's nginx authenticator)
CERT_EMAIL="${AKATSUKI_CERT_EMAIL:-}"
SYNC_PORT=47310
API_PORT=47311
HOSTS=(sync.akatsuki.vc api.akatsuki.vc)

MODE=plan
ROLLBACK=0
UPLOAD_ENV_DIR=""
TAG=""
SKIP_NGINX=0

usage() {
  sed -n '2,22p' "$0" | sed 's/^# \{0,1\}//'
  cat <<'EOF'

Options:
  --apply              execute (default: print the plan only)
  --preflight          run the read-only server checks and stop
  --rollback           with --apply: restore the previous release's images
  --tag TAG            image tag (default: git short sha, or a UTC timestamp)
  --upload-env DIR     copy DIR/sync.env and DIR/api.env to /opt/akatsuki (mode 0600) without reading them
  --skip-nginx         containers only; leave the vhosts alone
Environment: AKATSUKI_SSH_HOST, AKATSUKI_REMOTE_SUDO, AKATSUKI_NGINX_SITES_DIR, AKATSUKI_NGINX_ENABLED_DIR,
             AKATSUKI_NGINX_RELOAD, AKATSUKI_CERT_METHOD (webroot|nginx), AKATSUKI_CERT_EMAIL.
EOF
}

while [ $# -gt 0 ]; do
  case "$1" in
    --apply) MODE=apply ;;
    --preflight) MODE=preflight ;;
    --dry-run) MODE=plan ;;
    --rollback) ROLLBACK=1 ;;
    --tag) TAG="${2:?--tag needs a value}"; shift ;;
    --upload-env) UPLOAD_ENV_DIR="${2:?--upload-env needs a directory}"; shift ;;
    --skip-nginx) SKIP_NGINX=1 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "unknown argument: $1" >&2; usage >&2; exit 2 ;;
  esac
  shift
done

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo"

if [ -z "$TAG" ]; then
  TAG="$(git rev-parse --short=12 HEAD 2>/dev/null || date -u +%Y%m%d%H%M%S)"
fi
case "$TAG" in *[!A-Za-z0-9_.-]*) echo "invalid tag: $TAG" >&2; exit 2 ;; esac
case "$CERT_METHOD" in webroot|nginx) ;; *) echo "AKATSUKI_CERT_METHOD must be webroot or nginx" >&2; exit 2 ;; esac

say() { printf '\n==> %s\n' "$*"; }

# remote CMD...: run a command on the server (apply), or print it (plan).
remote() {
  if [ "$MODE" = apply ]; then
    ssh -o BatchMode=yes "$SSH_HOST" -- "$@"
  else
    printf '  [plan] ssh %s -- %s\n' "$SSH_HOST" "$*"
  fi
}
# local_run CMD...: run a local command that talks to the server (apply), or print it (plan).
local_run() {
  if [ "$MODE" = apply ]; then
    "$@"
  else
    printf '  [plan] %s\n' "$*"
  fi
}
S="${REMOTE_SUDO:+$REMOTE_SUDO }"
COMPOSE="${S}docker compose -p akatsuki --env-file $REMOTE_ROOT/release.env -f $REMOTE_ROOT/docker-compose.yml"

# ---------------------------------------------------------------- preflight (read-only on the server)
# Exits non-zero when anything that is not Akatsuki's holds a port, a name or a vhost we need.
preflight_script() {
  cat <<EOF
set -u
fail=0
bad() { echo "PREFLIGHT FAIL: \$*"; fail=1; }
ok() { echo "ok: \$*"; }
command -v docker >/dev/null || bad "docker missing"
${S}docker compose version >/dev/null 2>&1 || bad "docker compose v2 missing"
command -v nginx >/dev/null || bad "nginx missing"
for port in $SYNC_PORT $API_PORT; do
  holder=\$(${S}ss -ltnpH "( sport = :\$port )" 2>/dev/null)
  if [ -z "\$holder" ]; then ok "port \$port free"
  elif printf '%s' "\$holder" | grep -q docker-proxy && ${S}docker ps --format '{{.Names}} {{.Ports}}' | grep -E '^akatsuki-(sync|api) ' | grep -q ":\$port->"; then ok "port \$port held by our container"
  else bad "port \$port is in use by something else: \$holder"; fi
done
for name in akatsuki-sync akatsuki-api; do
  if ${S}docker container inspect "\$name" >/dev/null 2>&1; then
    proj=\$(${S}docker container inspect -f '{{index .Config.Labels "com.docker.compose.project"}}' "\$name")
    [ "\$proj" = akatsuki ] && ok "container \$name is ours" || bad "container \$name exists and is not from compose project akatsuki"
  else ok "container name \$name free"; fi
done
if ${S}docker network inspect akatsuki-net >/dev/null 2>&1; then
  proj=\$(${S}docker network inspect -f '{{index .Labels "com.docker.compose.project"}}' akatsuki-net)
  [ "\$proj" = akatsuki ] && ok "network akatsuki-net is ours" || bad "network akatsuki-net exists and is not ours"
fi
for host in ${HOSTS[*]}; do
  f="$NGINX_SITES_DIR/\$host.conf"
  if [ -e "\$f" ]; then
    ${S}grep -q '^# managed-by: akatsuki' "\$f" && ok "vhost \$f is ours" || bad "\$f exists and is not managed by akatsuki"
  else ok "vhost \$f free"; fi
  others=\$(${S}nginx -T 2>/dev/null | grep -E '^# configuration file ' | sed 's/^# configuration file //; s/:\$//' | while read -r cf; do
    case "\$cf" in "$NGINX_SITES_DIR/\$host.conf"|"$NGINX_ENABLED_DIR/\$host.conf") continue ;; esac
    ${S}grep -lE "server_name[^;]*[[:space:]]\$host([[:space:];]|\$)" "\$cf" 2>/dev/null
  done)
  [ -z "\$others" ] && ok "no other vhost serves \$host" || bad "\$host already appears in: \$others"
  if ${S}test -e "/etc/letsencrypt/live/\$host/fullchain.pem"; then ok "certificate for \$host exists"; else echo "note: no certificate for \$host yet (issued on first apply)"; fi
done
for envf in $REMOTE_ROOT/sync.env $REMOTE_ROOT/api.env; do
  if ${S}test -f "\$envf"; then
    mode=\$(${S}stat -c '%a' "\$envf"); [ "\$mode" = 600 ] && ok "\$envf present (0600)" || bad "\$envf has mode \$mode, expected 600"
  else echo "note: \$envf missing (create it or use --upload-env)"; fi
done
avail=\$(df -Pk /opt | awk 'NR==2 {print \$4}')
[ "\${avail:-0}" -gt 5242880 ] && ok "more than 5 GB free on /opt" || bad "less than 5 GB free on /opt"
exit \$fail
EOF
}

run_preflight() {
  say "Preflight on $SSH_HOST (read-only)"
  if [ "$MODE" = plan ]; then
    echo "  [plan] ssh $SSH_HOST bash -s <<preflight (ports $SYNC_PORT/$API_PORT, container/network names, vhost ownership, server_name clashes, certificates, env-file modes, disk)"
    return 0
  fi
  preflight_script | ssh -o BatchMode=yes "$SSH_HOST" -- bash -s
}

# ---------------------------------------------------------------- source tree to ship (never .env files)
source_list() {
  # Tracked and untracked-but-not-ignored files. .gitignore excludes every .env* except .env.example.
  git ls-files -z --cached --others --exclude-standard \
    | tr '\0' '\n' \
    | grep -Ev '(^|/)\.env($|\.)' \
    | grep -Ev '\.(pem|key)$' \
    | grep -Ev '^(target|node_modules)/' || true
  printf '.env.example\n'
}

check_source_list() {
  local list="$1"
  if grep -Eq '(^|/)\.env($|\.)' <(grep -v '^\.env\.example$' "$list"); then
    echo "refusing to ship: the file list contains an .env file" >&2
    exit 1
  fi
}

# ---------------------------------------------------------------- steps
step_dirs() {
  say "Directories under $REMOTE_ROOT"
  remote "${S}install -d -m 0755 $REMOTE_ROOT $REMOTE_ROOT/src $REMOTE_ROOT/nginx $REMOTE_ROOT/nginx/backup $REMOTE_ROOT/acme"
  remote "${S}install -d -m 0711 $REMOTE_ROOT/data"
  remote "${S}install -d -m 0700 -o 65532 -g 65532 $REMOTE_ROOT/data/sync-spool"
}

step_upload() {
  say "Upload sources, compose file and vhosts"
  local list
  list="$(mktemp)"
  # shellcheck disable=SC2064
  trap "rm -f '$list'" EXIT
  source_list | sort -u >"$list"
  check_source_list "$list"
  echo "  $(wc -l <"$list") files from the working tree (no .env*, keys or build output)"
  local rs=(rsync -az "--chmod=D755,F644" -e "ssh -o BatchMode=yes")
  if [ -n "$REMOTE_SUDO" ]; then rs+=(--rsync-path="$REMOTE_SUDO rsync"); fi
  # --delete only inside the destination directories, which belong to Akatsuki.
  # Each release gets a fresh build context (rsync --files-from never deletes stale files).
  remote "${S}rm -rf $REMOTE_ROOT/src/$TAG && ${S}install -d -m 0755 $REMOTE_ROOT/src/$TAG"
  local_run "${rs[@]}" --files-from="$list" ./ "$SSH_HOST:$REMOTE_ROOT/src/$TAG/"
  # Keep the three newest source trees, inside /opt/akatsuki/src only.
  remote "cd $REMOTE_ROOT/src && ls -1dt -- */ | tail -n +4 | ${S}xargs -r rm -rf --"
  local_run "${rs[@]}" infra/hetzner/docker-compose.yml "$SSH_HOST:$REMOTE_ROOT/docker-compose.yml"
  local_run "${rs[@]}" --delete infra/hetzner/nginx/ "$SSH_HOST:$REMOTE_ROOT/nginx/staged/"
  if [ -n "$UPLOAD_ENV_DIR" ]; then
    for f in sync.env api.env; do
      [ -f "$UPLOAD_ENV_DIR/$f" ] || { echo "missing $UPLOAD_ENV_DIR/$f" >&2; exit 1; }
      local_run rsync -a --chmod=F600 -e "ssh -o BatchMode=yes" ${REMOTE_SUDO:+--rsync-path="$REMOTE_SUDO rsync"} \
        "$UPLOAD_ENV_DIR/$f" "$SSH_HOST:$REMOTE_ROOT/$f"
    done
    remote "${S}chown root:root $REMOTE_ROOT/sync.env $REMOTE_ROOT/api.env && ${S}chmod 0600 $REMOTE_ROOT/sync.env $REMOTE_ROOT/api.env"
  fi
  remote "${S}test -f $REMOTE_ROOT/sync.env && ${S}test -f $REMOTE_ROOT/api.env"
}

step_build() {
  say "Build images akatsuki-sync:$TAG and akatsuki-api:$TAG on the server"
  # Builds run in the server's Docker daemon from /opt/akatsuki/src/<tag>, with each Dockerfile's allow-list
  # .dockerignore. Nothing is pushed anywhere.
  remote "cd $REMOTE_ROOT/src/$TAG && ${S}docker build -f apps/sync/Dockerfile -t akatsuki-sync:$TAG ."
  remote "cd $REMOTE_ROOT/src/$TAG && ${S}docker build -f apps/api/Dockerfile -t akatsuki-api:$TAG ."
}

step_release() {
  say "Release file $REMOTE_ROOT/release.env"
  if [ "$ROLLBACK" = 1 ]; then
    remote "${S}test -f $REMOTE_ROOT/release.prev.env && ${S}cp $REMOTE_ROOT/release.env $REMOTE_ROOT/release.failed.env && ${S}cp $REMOTE_ROOT/release.prev.env $REMOTE_ROOT/release.env"
    return
  fi
  remote "if ${S}test -f $REMOTE_ROOT/release.env; then ${S}cp $REMOTE_ROOT/release.env $REMOTE_ROOT/release.prev.env; fi"
  remote "printf 'AKATSUKI_SYNC_IMAGE=akatsuki-sync:%s\nAKATSUKI_API_IMAGE=akatsuki-api:%s\n' $TAG $TAG | ${S}tee $REMOTE_ROOT/release.env >/dev/null"
}

step_up() {
  say "Start containers (project akatsuki only)"
  remote "$COMPOSE config --quiet"
  remote "$COMPOSE up -d --wait --wait-timeout 120 --remove-orphans"
  remote "curl -fsS --max-time 5 http://127.0.0.1:$SYNC_PORT/healthz && curl -fsS --max-time 5 http://127.0.0.1:$API_PORT/healthz"
}

nginx_test_and_reload() {
  # $1 = host whose vhost was just installed; restores the backup when nginx -t fails.
  local host="$1"
  remote "if ${S}nginx -t; then ${S}$NGINX_RELOAD; else echo 'nginx -t failed, restoring $host'; \
if ${S}test -f $REMOTE_ROOT/nginx/backup/$host.conf; then ${S}cp $REMOTE_ROOT/nginx/backup/$host.conf $NGINX_SITES_DIR/$host.conf; \
else ${S}rm -f $NGINX_SITES_DIR/$host.conf ${NGINX_ENABLED_DIR:+$NGINX_ENABLED_DIR/$host.conf}; fi; ${S}nginx -t; exit 1; fi"
}

install_vhost() {
  # $1 = host, $2 = staged file relative to $REMOTE_ROOT/nginx/staged
  local host="$1" file="$2"
  remote "if ${S}test -f $NGINX_SITES_DIR/$host.conf; then ${S}cp $NGINX_SITES_DIR/$host.conf $REMOTE_ROOT/nginx/backup/$host.conf; fi"
  remote "${S}install -m 0644 $REMOTE_ROOT/nginx/staged/$file $NGINX_SITES_DIR/$host.conf"
  if [ -n "$NGINX_ENABLED_DIR" ]; then
    remote "${S}ln -sfn $NGINX_SITES_DIR/$host.conf $NGINX_ENABLED_DIR/$host.conf"
  fi
  nginx_test_and_reload "$host"
}

step_nginx() {
  [ "$SKIP_NGINX" = 1 ] && { say "nginx: skipped (--skip-nginx)"; return; }
  for host in "${HOSTS[@]}"; do
    say "nginx + TLS for $host"
    local certbot
    if [ "$CERT_METHOD" = webroot ]; then
      certbot="certbot certonly --webroot -w $REMOTE_ROOT/acme -d $host"
    else
      certbot="certbot certonly --nginx -d $host"
    fi
    certbot+=" --non-interactive --agree-tos --keep-until-expiring --deploy-hook '$NGINX_RELOAD'"
    if [ -n "$CERT_EMAIL" ]; then certbot+=" -m $CERT_EMAIL"; else certbot+=" --register-unsafely-without-email"; fi
    # First deploy: serve the ACME challenge from a port-80-only vhost, issue, then install the full vhost.
    if [ "$MODE" = apply ] && ssh -o BatchMode=yes "$SSH_HOST" -- "${S}test -e /etc/letsencrypt/live/$host/fullchain.pem"; then
      echo "  certificate exists"
    else
      [ "$MODE" = plan ] && echo "  (only when /etc/letsencrypt/live/$host is missing:)"
      install_vhost "$host" "acme-bootstrap/$host.conf"
      remote "${S}$certbot"
    fi
    install_vhost "$host" "$host.conf"
  done
}

step_public_smoke() {
  say "Public smoke test (needs the Cloudflare A records)"
  for host in "${HOSTS[@]}"; do
    local_run curl -fsS --max-time 10 "https://$host/healthz"
  done
}

# ---------------------------------------------------------------- main
echo "Akatsuki Hetzner deploy: mode=$MODE host=$SSH_HOST tag=$TAG rollback=$ROLLBACK cert=$CERT_METHOD"
[ "$MODE" = plan ] && echo "(dry run: nothing is contacted or changed; pass --apply to execute)"

run_preflight
[ "$MODE" = preflight ] && exit 0

step_dirs
if [ "$ROLLBACK" = 0 ]; then
  step_upload
  step_build
fi
step_release
step_up
step_nginx
step_public_smoke
say "Done. Roll back with: scripts/deploy-hetzner.sh --apply --rollback"
