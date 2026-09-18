#!/usr/bin/env bash
# Shared Flagsmith REST helpers, sourced by flagsmith-bootstrap.sh and
# flagsmith-flags.sh. Not executable on its own.
#
# Two auth planes, do not mix them:
#   admin  -> `Authorization: Token <key>`  on /api/v1/{organisations,projects,environments}/...
#   sdk    -> `X-Environment-Key: <api_key>` on /api/v1/{flags,identities}/
#
# Every request goes through fs_admin/fs_sdk, which set FS_STATUS + FS_BODY
# instead of failing, so callers can treat "already exists" 400s as success.

FLAGSMITH_URL="${FLAGSMITH_URL:-http://localhost:8000}"
FLAGSMITH_ADMIN_EMAIL="${FLAGSMITH_ADMIN_EMAIL:-admin@nx-playground.local}"
# Flagsmith runs Django's password validators on signup, so the default must be
# long enough and not resemble the email (`nx-playground-dev` is rejected).
FLAGSMITH_ADMIN_PASSWORD="${FLAGSMITH_ADMIN_PASSWORD:-LocalDev12345!}"
FLAGSMITH_ORG="${FLAGSMITH_ORG:-nx-playground}"
FLAGSMITH_PROJECT="${FLAGSMITH_PROJECT:-todo}"
FLAGSMITH_ENV_NAME="${FLAGSMITH_ENV_NAME:-Development}"

# The six flags todo-api evaluates. `bool` -> default_enabled, `int` -> integer
# initial_value. Keep in sync with docs/feature-flags.md and the FlagDefaults
# builder in libs/core/feature-flags.
FS_FLAGS=(
  "todo_app_web|bool|on|todo-web SPA (:3100) availability"
  "todo_app_htmx|bool|on|todo-web-htmx (:3300) availability"
  "todo_app_astro|bool|on|todo-web-astro (:3200) availability"
  "todo_realtime|bool|on|live SSE/WS todo updates"
  "todo_write|bool|on|todo mutations (POST/PUT/DELETE)"
  "todo_max_items|int|-1|per-list todo cap, -1 = unlimited"
)

FS_TOKEN=""
FS_STATUS=""
FS_BODY=""
FS_AUTH_MODE=""
FS_PROJECT_ID=""
FS_ENV_KEY=""
_FS_BODY_FILE=""

fs_die() {
  printf 'FATAL: %s\n' "$*" >&2
  exit 1
}

fs_require_deps() {
  local dep
  for dep in curl jq; do
    command -v "$dep" >/dev/null 2>&1 || fs_die "$dep is required but not on PATH"
  done
  _FS_BODY_FILE=$(mktemp)
  # shellcheck disable=SC2064  # expand the path now, the var may be reassigned
  trap "rm -f '$_FS_BODY_FILE'" EXIT
}

# fs_curl METHOD URL JSON_BODY [extra curl args...]
# Sets FS_STATUS (HTTP code, or 000 when the connection failed) and FS_BODY.
# JSON_BODY is mandatory but may be the empty string for GETs.
fs_curl() {
  local method=$1 url=$2 body=$3
  shift 3
  local -a args=(
    -sS -X "$method" "$url"
    -H 'Accept: application/json'
    -o "$_FS_BODY_FILE" -w '%{http_code}'
    --max-time "${FLAGSMITH_TIMEOUT:-20}"
  )
  if [[ -n $body ]]; then
    args+=(-H 'Content-Type: application/json' --data-binary "$body")
  fi
  FS_STATUS=$(curl "${args[@]}" "$@" 2>/dev/null || echo 000)
  FS_BODY=$(cat "$_FS_BODY_FILE" 2>/dev/null || true)
}

# Admin-plane request (token auth). fs_admin METHOD PATH [JSON_BODY]
fs_admin() {
  local method=$1 path=$2 body=${3-}
  fs_curl "$method" "$FLAGSMITH_URL$path" "$body" -H "Authorization: Token $FS_TOKEN"
}

# SDK-plane request (environment key). fs_sdk METHOD PATH ENV_KEY [JSON_BODY]
fs_sdk() {
  local method=$1 path=$2 key=$3 body=${4-}
  fs_curl "$method" "$FLAGSMITH_URL$path" "$body" -H "X-Environment-Key: $key"
}

fs_ok() { [[ $FS_STATUS == 2?? ]]; }

# True when the 400 body is Flagsmith's "already exists" uniqueness complaint.
fs_already_exists() {
  [[ $FS_STATUS == 400 ]] && grep -qi 'already exists' <<<"$FS_BODY"
}

fs_json() { jq -r "$1" <<<"$FS_BODY"; }

# Run a jq program against FS_BODY normalised to a JSON array. Flagsmith is
# inconsistent: /projects/ answers with a bare array, everything else with a
# {count,results} page. fs_jq 'PROGRAM' [jq args...] hides that.
fs_jq() {
  local prog=$1
  shift
  jq -r "$@" "(if type == \"object\" then (.results // []) else . end) | $prog" <<<"$FS_BODY"
}

# fs_find MATCH_KEY MATCH_VALUE OUT_KEY -> the first matching item's OUT_KEY,
# or the empty string when nothing matches.
fs_find() {
  fs_jq 'map(select((.[$k] | tostring) == $v)) | (.[0][$o] // empty) | tostring' \
    --arg k "$1" --arg v "$2" --arg o "$3"
}

# fs_print_flags '<json array of SDK flag objects>'
# Renders `  name  on|off  <feature_state_value as JSON>`, sorted by name.
fs_print_flags() {
  jq -r 'sort_by(.feature.name)[]
         | (.feature.name) as $n
         | "  " + $n + (" " * ([18 - ($n | length), 1] | max))
         + (if .enabled then "on " else "off" end) + "  "
         + (.feature_state_value | tojson)' <<<"$1"
}

fs_wait_healthy() {
  local i
  for i in $(seq 1 60); do
    fs_curl GET "$FLAGSMITH_URL/health" ""
    [[ $FS_STATUS == 200 ]] && return 0
    sleep 0.5
  done
  fs_die "Flagsmith at $FLAGSMITH_URL never became healthy (last status $FS_STATUS).
       Start it with \`just docker-up\` and check \`docker logs flagsmith\`."
}

# Resolve an admin API token: log in if the user exists, self-sign-up if not.
# Login goes first so re-runs never surface a bogus signup validation error.
fs_login() {
  local login_err
  fs_curl POST "$FLAGSMITH_URL/api/v1/auth/login/" "$(jq -nc \
    --arg e "$FLAGSMITH_ADMIN_EMAIL" --arg p "$FLAGSMITH_ADMIN_PASSWORD" \
    '{email:$e, password:$p}')"
  if fs_ok; then
    FS_TOKEN=$(fs_json '.key')
    FS_AUTH_MODE=existing
  else
    login_err="HTTP $FS_STATUS: $FS_BODY"
    fs_curl POST "$FLAGSMITH_URL/api/v1/auth/users/" "$(jq -nc \
      --arg e "$FLAGSMITH_ADMIN_EMAIL" --arg p "$FLAGSMITH_ADMIN_PASSWORD" \
      '{email:$e, password:$p, first_name:"Local", last_name:"Admin", sign_up_type:"NO_INVITE"}')"
    fs_ok || fs_die "could not log in or sign up as $FLAGSMITH_ADMIN_EMAIL
       login  -> $login_err
       signup -> HTTP $FS_STATUS: $FS_BODY
       Flagsmith applies Django password validators (length, not too similar to
       the email). Override with FLAGSMITH_ADMIN_EMAIL/FLAGSMITH_ADMIN_PASSWORD."
    FS_TOKEN=$(fs_json '.key')
    FS_AUTH_MODE=created
  fi
  [[ -n $FS_TOKEN && $FS_TOKEN != null ]] || fs_die "auth succeeded but returned no token: $FS_BODY"
}

# Echo the id of organisation $FLAGSMITH_ORG, creating it when absent.
fs_ensure_org() {
  fs_admin GET /api/v1/organisations/
  fs_ok || fs_die "listing organisations failed (HTTP $FS_STATUS): $FS_BODY"
  local id
  id=$(fs_find name "$FLAGSMITH_ORG" id)
  if [[ -z $id ]]; then
    fs_admin POST /api/v1/organisations/ "$(jq -nc --arg n "$FLAGSMITH_ORG" '{name:$n}')"
    fs_ok || fs_die "creating organisation $FLAGSMITH_ORG failed (HTTP $FS_STATUS): $FS_BODY"
    id=$(fs_json '.id')
  fi
  printf '%s' "$id"
}

# fs_ensure_project ORG_ID -> project id
fs_ensure_project() {
  local org=$1 id
  fs_admin GET "/api/v1/projects/?organisation=$org"
  fs_ok || fs_die "listing projects failed (HTTP $FS_STATUS): $FS_BODY"
  id=$(fs_find name "$FLAGSMITH_PROJECT" id)
  if [[ -z $id ]]; then
    fs_admin POST /api/v1/projects/ "$(jq -nc --arg n "$FLAGSMITH_PROJECT" --argjson o "$org" \
      '{name:$n, organisation:$o}')"
    fs_ok || fs_die "creating project $FLAGSMITH_PROJECT failed (HTTP $FS_STATUS): $FS_BODY"
    id=$(fs_json '.id')
  fi
  printf '%s' "$id"
}

# fs_ensure_environment PROJECT_ID -> environment api_key
fs_ensure_environment() {
  local project=$1 key
  fs_admin GET "/api/v1/environments/?project=$project"
  fs_ok || fs_die "listing environments failed (HTTP $FS_STATUS): $FS_BODY"
  key=$(fs_find name "$FLAGSMITH_ENV_NAME" api_key)
  if [[ -z $key ]]; then
    fs_admin POST /api/v1/environments/ "$(jq -nc --arg n "$FLAGSMITH_ENV_NAME" --argjson p "$project" \
      '{name:$n, project:$p}')"
    fs_ok || fs_die "creating environment $FLAGSMITH_ENV_NAME failed (HTTP $FS_STATUS): $FS_BODY"
    key=$(fs_json '.api_key')
  fi
  [[ -n $key && $key != null ]] || fs_die "environment $FLAGSMITH_ENV_NAME has no api_key: $FS_BODY"
  printf '%s' "$key"
}

# Full admin-side discovery without creating anything. Sets FS_TOKEN,
# FS_PROJECT_ID, FS_ENV_KEY. Used by flags-list/flags-set.
fs_discover() {
  fs_login
  local org
  fs_admin GET /api/v1/organisations/
  fs_ok || fs_die "listing organisations failed (HTTP $FS_STATUS): $FS_BODY"
  org=$(fs_find name "$FLAGSMITH_ORG" id)
  [[ -n $org ]] || fs_die "organisation '$FLAGSMITH_ORG' not found — run \`just flags-bootstrap\` first"
  fs_admin GET "/api/v1/projects/?organisation=$org"
  fs_ok || fs_die "listing projects failed (HTTP $FS_STATUS): $FS_BODY"
  FS_PROJECT_ID=$(fs_find name "$FLAGSMITH_PROJECT" id)
  [[ -n $FS_PROJECT_ID ]] || fs_die "project '$FLAGSMITH_PROJECT' not found — run \`just flags-bootstrap\` first"
  fs_admin GET "/api/v1/environments/?project=$FS_PROJECT_ID"
  fs_ok || fs_die "listing environments failed (HTTP $FS_STATUS): $FS_BODY"
  FS_ENV_KEY=$(fs_find name "$FLAGSMITH_ENV_NAME" api_key)
  [[ -n $FS_ENV_KEY ]] || fs_die "environment '$FLAGSMITH_ENV_NAME' not found — run \`just flags-bootstrap\` first"
}

# Resolve the SDK environment key into FS_ENV_KEY. Prefers a real
# $FLAGSMITH_ENVIRONMENT_KEY (validated with one cheap SDK call), and falls back
# to admin-API discovery — so the recipes work straight after bootstrap with
# nothing in .env.local yet. `set dotenv-load` in the justfile means recipes
# inherit .env, which ships the `your-flagsmith-key` placeholder: never trust it.
fs_resolve_env_key() {
  local key=${FLAGSMITH_ENVIRONMENT_KEY:-}
  if [[ -n $key && $key != your-flagsmith-key ]]; then
    fs_sdk GET /api/v1/flags/ "$key"
    if fs_ok; then
      FS_ENV_KEY=$key
      return 0
    fi
    echo "warn FLAGSMITH_ENVIRONMENT_KEY is set but rejected (HTTP $FS_STATUS); discovering via admin API" >&2
  fi
  fs_discover
}
