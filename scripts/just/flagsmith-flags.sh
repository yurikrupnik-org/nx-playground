#!/usr/bin/env bash
# Read and flip todo feature flags from the CLI. Invoked by `just flags-list`
# and `just flags-set`.
#
#   flagsmith-flags.sh list [identity]
#       Print the flags exactly as todo-api sees them: GET /api/v1/flags/ for
#       the anonymous/environment view, GET /api/v1/identities/?identifier=<id>
#       when an identity is given (this also registers the identity).
#
#   flagsmith-flags.sh set <flag> <on|off> [identity]
#       No identity  -> PATCH the environment-level feature state (affects
#                       everyone without an override).
#       With identity -> create or PATCH that identity's feature-state override
#                       under /api/v1/environments/<env_key>/identities/<id>/
#                       featurestates/, which wins over the environment value.
#
# The environment key comes from $FLAGSMITH_ENVIRONMENT_KEY when it is set to a
# real value; otherwise it is discovered through the admin API, so these recipes
# work straight after `just flags-bootstrap` with nothing in .env.local yet.
set -euo pipefail

# shellcheck source=scripts/just/flagsmith-lib.sh
source "$(dirname "${BASH_SOURCE[0]}")/flagsmith-lib.sh"

usage() {
  sed -n '2,19p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//' >&2
  exit 2
}

# Echo the numeric id of feature $1 in project $FS_PROJECT_ID.
feature_id() {
  local name=$1 id
  fs_admin GET "/api/v1/projects/$FS_PROJECT_ID/features/"
  fs_ok || fs_die "listing features failed (HTTP $FS_STATUS): $FS_BODY"
  id=$(fs_find name "$name" id)
  [[ -n $id ]] || fs_die "unknown flag '$name'. Known flags:
$(printf '       %s\n' "${FS_FLAGS[@]%%|*}")
       Run \`just flags-bootstrap\` if the project has not been provisioned."
  printf '%s' "$id"
}

# Echo the numeric id of identity $1, creating it when absent.
identity_id() {
  local ident=$1 id
  fs_admin GET "/api/v1/environments/$FS_ENV_KEY/identities/?q=$ident"
  fs_ok || fs_die "searching identities failed (HTTP $FS_STATUS): $FS_BODY"
  id=$(fs_find identifier "$ident" id)
  if [[ -z $id ]]; then
    fs_admin POST "/api/v1/environments/$FS_ENV_KEY/identities/" \
      "$(jq -nc --arg i "$ident" '{identifier:$i}')"
    if fs_ok; then
      id=$(fs_json '.id')
    elif fs_already_exists; then
      fs_admin GET "/api/v1/environments/$FS_ENV_KEY/identities/?q=$ident"
      id=$(fs_find identifier "$ident" id)
    else
      fs_die "creating identity '$ident' failed (HTTP $FS_STATUS): $FS_BODY"
    fi
  fi
  [[ -n $id ]] || fs_die "could not resolve identity '$ident'"
  printf '%s' "$id"
}

cmd_list() {
  local ident=${1-}
  fs_resolve_env_key
  if [[ -n $ident ]]; then
    fs_sdk GET "/api/v1/identities/?identifier=$ident" "$FS_ENV_KEY"
    fs_ok || fs_die "identity flag lookup failed (HTTP $FS_STATUS): $FS_BODY"
    echo "flags for identity '$ident' (environment $FS_ENV_KEY):"
    fs_print_flags "$(jq -c '.flags' <<<"$FS_BODY")"
  else
    fs_sdk GET /api/v1/flags/ "$FS_ENV_KEY"
    fs_ok || fs_die "environment flag lookup failed (HTTP $FS_STATUS): $FS_BODY"
    echo "flags for the anonymous/environment view (environment $FS_ENV_KEY):"
    fs_print_flags "$FS_BODY"
  fi
}

cmd_set() {
  local name=${1-} state=${2-} ident=${3-}
  [[ -n $name && -n $state ]] || usage
  local enabled
  case $state in
    on | true | enabled) enabled=true ;;
    off | false | disabled) enabled=false ;;
    *) fs_die "state must be 'on' or 'off', got '$state'" ;;
  esac

  fs_discover
  local fid
  fid=$(feature_id "$name")

  if [[ -z $ident ]]; then
    fs_admin GET "/api/v1/environments/$FS_ENV_KEY/featurestates/?feature=$fid"
    fs_ok || fs_die "listing environment feature states failed (HTTP $FS_STATUS): $FS_BODY"
    local fsid
    fsid=$(fs_jq 'map(select(.identity == null and .feature_segment == null)) | (.[0].id // empty)')
    [[ -n $fsid ]] || fs_die "no environment feature state for '$name' — re-run \`just flags-bootstrap\`"
    fs_admin PATCH "/api/v1/environments/$FS_ENV_KEY/featurestates/$fsid/" \
      "$(jq -nc --argjson e "$enabled" '{enabled:$e}')"
    fs_ok || fs_die "updating environment feature state failed (HTTP $FS_STATUS): $FS_BODY"
    echo "set $name = $state for the whole $FLAGSMITH_ENV_NAME environment"
  else
    local iid
    iid=$(identity_id "$ident")
    fs_admin GET "/api/v1/environments/$FS_ENV_KEY/identities/$iid/featurestates/"
    fs_ok || fs_die "listing identity feature states failed (HTTP $FS_STATUS): $FS_BODY"
    local fsid
    fsid=$(fs_find feature "$fid" id)
    if [[ -n $fsid ]]; then
      fs_admin PATCH "/api/v1/environments/$FS_ENV_KEY/identities/$iid/featurestates/$fsid/" \
        "$(jq -nc --argjson e "$enabled" '{enabled:$e}')"
      fs_ok || fs_die "updating identity override failed (HTTP $FS_STATUS): $FS_BODY"
      echo "set $name = $state for identity '$ident' (updated existing override)"
    else
      fs_admin POST "/api/v1/environments/$FS_ENV_KEY/identities/$iid/featurestates/" \
        "$(jq -nc --argjson f "$fid" --argjson e "$enabled" '{feature:$f, enabled:$e}')"
      fs_ok || fs_die "creating identity override failed (HTTP $FS_STATUS): $FS_BODY"
      echo "set $name = $state for identity '$ident' (created override)"
    fi
  fi
}

fs_require_deps
case ${1-} in
  list)
    shift
    cmd_list "$@"
    ;;
  set)
    shift
    cmd_set "$@"
    ;;
  *) usage ;;
esac
