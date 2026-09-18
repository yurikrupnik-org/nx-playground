#!/usr/bin/env bash
# Provision the local Flagsmith instance for the todo product, end to end.
# Invoked by `just flags-bootstrap`.
#
# Creates (idempotently — every step looks up before it creates):
#   admin user  $FLAGSMITH_ADMIN_EMAIL      (self-signup on a virgin instance)
#   org         $FLAGSMITH_ORG              (default nx-playground)
#   project     $FLAGSMITH_PROJECT          (default todo)
#   environment $FLAGSMITH_ENV_NAME         (default Development)
#   features    the six flags in FS_FLAGS (see scripts/just/flagsmith-lib.sh)
#
# Re-running never duplicates anything and never resets a flag you have flipped
# by hand: existing features are reported, not rewritten. Prints the SDK
# environment key to paste into .env.local.
#
# Overridable: FLAGSMITH_URL FLAGSMITH_ADMIN_EMAIL FLAGSMITH_ADMIN_PASSWORD
#              FLAGSMITH_ORG FLAGSMITH_PROJECT FLAGSMITH_ENV_NAME
set -euo pipefail

# shellcheck source=scripts/just/flagsmith-lib.sh
source "$(dirname "${BASH_SOURCE[0]}")/flagsmith-lib.sh"

fs_require_deps
fs_wait_healthy
echo "ok   Flagsmith healthy at $FLAGSMITH_URL"

fs_login
echo "ok   admin user $FLAGSMITH_ADMIN_EMAIL ($FS_AUTH_MODE), token acquired"

org_id=$(fs_ensure_org)
echo "ok   organisation $FLAGSMITH_ORG (id $org_id)"

project_id=$(fs_ensure_project "$org_id")
echo "ok   project $FLAGSMITH_PROJECT (id $project_id)"

env_key=$(fs_ensure_environment "$project_id")
echo "ok   environment $FLAGSMITH_ENV_NAME (key $env_key)"

# One listing, then create only what is missing.
fs_admin GET "/api/v1/projects/$project_id/features/"
fs_ok || fs_die "listing features failed (HTTP $FS_STATUS): $FS_BODY"
existing=$(fs_jq '.[].name')

for spec in "${FS_FLAGS[@]}"; do
  IFS='|' read -r name kind default desc <<<"$spec"
  if grep -qxF "$name" <<<"$existing"; then
    echo "ok   feature $name already exists (left as-is)"
    continue
  fi
  if [[ $kind == int ]]; then
    payload=$(jq -nc --arg n "$name" --arg d "$desc" --argjson v "$default" \
      '{name:$n, description:$d, type:"STANDARD", default_enabled:true, initial_value:$v}')
  else
    payload=$(jq -nc --arg n "$name" --arg d "$desc" \
      --argjson e "$([[ $default == on ]] && echo true || echo false)" \
      '{name:$n, description:$d, type:"STANDARD", default_enabled:$e}')
  fi
  fs_admin POST "/api/v1/projects/$project_id/features/" "$payload"
  if fs_ok; then
    echo "ok   feature $name created ($kind, default $default)"
  elif fs_already_exists; then
    echo "ok   feature $name already exists (race, left as-is)"
  else
    fs_die "creating feature $name failed (HTTP $FS_STATUS): $FS_BODY"
  fi
done

# Read the result back through the SDK plane — proves the environment key works.
fs_sdk GET /api/v1/flags/ "$env_key"
fs_ok || fs_die "SDK readback with the new environment key failed (HTTP $FS_STATUS): $FS_BODY"
echo
echo "effective environment flags:"
fs_print_flags "$FS_BODY"

cat <<EOF

Flagsmith is provisioned. UI: $FLAGSMITH_URL  (login: $FLAGSMITH_ADMIN_EMAIL)

Environment key: $env_key

Add this to .env.local (or export it in your shell) so todo-api reads real flags —
leave it unset/empty and every app falls back to its built-in defaults:

export FLAGSMITH_ENVIRONMENT_KEY=$env_key
EOF
