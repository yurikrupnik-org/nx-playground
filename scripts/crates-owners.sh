#!/usr/bin/env bash
# Move crates.io ownership from the old personal account to the shluviza org.
#
# WHY THIS EXISTS
#   crates.io ownership is per-crate and is decided by whichever API token ran
#   the first `cargo publish` — it has nothing to do with the GitHub org the
#   code lives in. Every crate in CRATES below was first published with
#   `slavalslutkovsky`'s token, so publishing as `shluviza-admin` fails with:
#       403 Forbidden: this crate exists but you don't seem to be an owner
#   Published versions are immutable; only ownership can be changed, and only
#   by a *user* owner.
#
# THE TARGET SHAPE (both, they do different jobs)
#   user owner  shluviza-admin        -> owner-of-record. Only user owners can
#                                        add/remove owners and change crate
#                                        settings (incl. trusted publishing).
#                                        Keep at least one, forever.
#   team owner  github:shluviza:TEAM  -> publish rights follow GitHub team
#                                        membership: add/remove humans on
#                                        GitHub, no crates.io invites. Team
#                                        owners can publish and yank but
#                                        CANNOT manage owners — hence the user
#                                        owner above.
#
# TOKEN PRECEDENCE — THE TRAP THAT CAUSED THIS
#   cargo resolves the token as:
#       CARGO_REGISTRIES_CRATES_IO_TOKEN  >  CARGO_REGISTRY_TOKEN  >  ~/.cargo/credentials.toml
#   This repo's .envrc runs `dotenv_if_exists .env`, and .env sets
#   CARGO_REGISTRY_TOKEN. So inside this repo `cargo login` is silently
#   ignored — that is how butler 0.1.0 shipped under the wrong account.
#   This script therefore never relies on ambient state: it clears both env
#   vars and passes the token it was given explicitly.
#
# USAGE
#   scripts/crates-owners.sh status
#       Read-only. Prints the current owners of every crate. No token needed.
#
#   CRATES_IO_OLD_TOKEN=cio_... scripts/crates-owners.sh invite
#       Run with a token of the CURRENT owner (slavalslutkovsky) that has the
#       `change-owners` scope — publish-only tokens are rejected here. Adds
#       shluviza-admin (as an invite) and the GitHub team (effective at once).
#
#   -> then shluviza-admin accepts at https://crates.io/me/pending-invites
#      (website only; the API refuses: "this action can only be performed on
#       the crates.io website")
#
#   CRATES_IO_OLD_TOKEN=cio_... scripts/crates-owners.sh finish
#       Removes slavalslutkovsky once shluviza-admin is a confirmed owner.
#       crates.io requires >=1 owner at all times, so this must run last.
#
#   DRY_RUN=1 ...   prints the cargo commands instead of running them.
set -euo pipefail

# GitHub team that should get publish rights. Must already exist under the
# `shluviza` org, and the token holder must be a member of the org AND the team
# — crates.io verifies both against GitHub before accepting a team owner.
ORG="shluviza"
TEAM="${CRATES_TEAM:-crates-publishers}"

NEW_USER="shluviza-admin"        # new GitHub + crates.io account
OLD_USER="slavalslutkovsky"      # legacy account that owns everything today

# Crate NAMES (not `-p` flags). Keep in sync with `PUBLISHED_CRATES` in
# scripts/tasks/rust.yml; `butler` is extra — it is an app crate with
# publish = true in apps/butler/cli/Cargo.toml and is not in that list.
CRATES=(
  core_config
  core_retry
  core_strings
  field-selector
  oidc-auth
  api_resource
  sea_orm_resource
  selectable_fields
  core_proc_macros
  axum-helpers
  butler
)

DRY_RUN="${DRY_RUN:-0}"

run() {
  if [[ "$DRY_RUN" == 1 ]]; then
    echo "DRY  $*"
  else
    "$@"
  fi
}

# Every cargo call goes through here: the ambient .env token must not decide
# who we authenticate as.
cargo_owner_as() {
  local token="$1"; shift
  run env -u CARGO_REGISTRIES_CRATES_IO_TOKEN CARGO_REGISTRY_TOKEN="$token" cargo owner "$@"
}

require_old_token() {
  if [[ -z "${CRATES_IO_OLD_TOKEN:-}" ]]; then
    cat >&2 <<'EOF'
FATAL: CRATES_IO_OLD_TOKEN is not set.

Create it at https://crates.io/settings/tokens while logged in as the CURRENT
owner (slavalslutkovsky), with the `change-owners` scope ticked. A token with
only publish-new/publish-update cannot transfer ownership.

Pass it per-invocation (do not put it in .env — that file already shadows
cargo login for every command in this repo):

    CRATES_IO_OLD_TOKEN=cio_... scripts/crates-owners.sh invite

Shell history and copy/paste truncate tokens. Prefer reading it from a file:

    CRATES_IO_OLD_TOKEN="$(cat ~/.secrets/crates-slava.token)" scripts/crates-owners.sh invite
EOF
    exit 1
  fi

  # Shape check. crates.io tokens are `cio` + 32 random chars = 35 chars; a
  # 34-char paste is a dropped character, which the server reports only as a
  # generic "authentication failed" after it has already updated the index.
  local len=${#CRATES_IO_OLD_TOKEN}
  if [[ $len -ne 35 ]]; then
    echo "FATAL: token is $len chars, expected 35 (cio + 32) — truncated paste?" >&2
    exit 1
  fi

  # Liveness check. /api/v1/me/* is cookie-only, so it answers 403 for EVERY
  # API token — the status code says nothing. The body does:
  #   valid token   -> "this action can only be performed on the crates.io website"
  #   unknown token -> "authentication failed"
  # That distinguishes a revoked/mistyped token from a scope or ownership
  # problem, which would otherwise only surface after `cargo owner` has
  # updated the registry index.
  local body
  body=$(curl -s -H "Authorization: $CRATES_IO_OLD_TOKEN" \
    -H 'User-Agent: crates-owners.sh' https://crates.io/api/v1/me/updates)
  if [[ "$body" == *"authentication failed"* ]]; then
    echo "FATAL: crates.io does not recognize this token." >&2
    echo "       It is revoked, expired, or mistyped. Issue a new one at" >&2
    echo "       https://crates.io/settings/tokens as $OLD_USER, scope change-owners." >&2
    exit 1
  fi
}

# `cargo owner` on a crate that was never published dies with a 404 and, under
# `set -e`, would abort the whole loop. core_strings is exactly that case today:
# it is listed in `PUBLISHED_CRATES` (scripts/tasks/rust.yml) but has never
# shipped — crates.io answers `crate "core_strings" does not exist`.
crate_exists() {
  curl -sf -o /dev/null -H 'User-Agent: crates-owners.sh' \
    "https://crates.io/api/v1/crates/$1"
}

cmd_status() {
  printf '%-20s %s\n' CRATE OWNERS
  for c in "${CRATES[@]}"; do
    printf '%-20s ' "$c"
    curl -sf -H 'User-Agent: crates-owners.sh' \
      "https://crates.io/api/v1/crates/$c/owners" |
      jq -rc '[.users[] | if .kind == "team" then "team:" + .login else .login end] | join(", ")' ||
      echo "(not published on crates.io)"
    sleep 0.3   # crates.io rate-limits bursts of anonymous API calls with 403
  done
}

cmd_invite() {
  require_old_token
  local failed=()
  for c in "${CRATES[@]}"; do
    if ! crate_exists "$c"; then
      echo "SKIP $c — not published on crates.io, nothing to own"
      continue
    fi
    echo "== $c"
    # Per-crate faults must not abort the batch: a missing GitHub team or an
    # already-sent invitation on crate 1 would otherwise leave crates 2..N
    # untouched under `set -e`, which is exactly what happened on the first run.
    # User owner: crates.io sends an invitation; ownership is NOT active until
    # shluviza-admin accepts it on the website.
    cargo_owner_as "$CRATES_IO_OLD_TOKEN" --add "$NEW_USER" "$c" || failed+=("$c user")
    # Team owner: no invitation step, effective immediately. crates.io resolves
    # the team through the GitHub account behind the *calling* token, so that
    # account must be a member of both the org and the team.
    cargo_owner_as "$CRATES_IO_OLD_TOKEN" --add "github:$ORG:$TEAM" "$c" || failed+=("$c team")
  done
  if [[ ${#failed[@]} -gt 0 ]]; then
    printf '\nFAILED: %s\n' "${failed[*]}" >&2
  fi
  cat <<EOF

NEXT: log in to crates.io as $NEW_USER and accept the invitations:
    https://crates.io/me/pending-invites
Then verify with:  scripts/crates-owners.sh status
Only after $NEW_USER shows up as an owner, run:  scripts/crates-owners.sh finish
EOF
}

cmd_finish() {
  require_old_token
  for c in "${CRATES[@]}"; do
    if ! crate_exists "$c"; then
      echo "SKIP $c — not published on crates.io"
      continue
    fi
    # Guard: removing the last owner is rejected by crates.io, and removing the
    # old owner before the invite is accepted would strand the crate.
    if ! curl -sf -H 'User-Agent: crates-owners.sh' \
      "https://crates.io/api/v1/crates/$c/owners" |
      jq -e --arg u "$NEW_USER" '.users[] | select(.login == $u)' >/dev/null; then
      echo "SKIP $c — $NEW_USER is not an owner yet (invitation not accepted)"
      continue
    fi
    echo "== $c"
    cargo_owner_as "$CRATES_IO_OLD_TOKEN" --remove "$OLD_USER" "$c"
  done
}

# Undo the team-owner half of the transfer. Needed if you decide the team is
# ceremony (one maintainer) and want user ownership only.
#
# ORDER MATTERS: revoke crates.io team ownership FIRST, delete the GitHub team
# SECOND. A GitHub team that is deleted while still listed as a crate owner
# leaves crates.io holding an owner record it can no longer resolve.
#
# Takes the token of a CURRENT USER owner (shluviza-admin) — team owners cannot
# modify owners, so the team cannot remove itself.
cmd_drop_team() {
  if [[ -z "${CRATES_IO_TOKEN:-}" ]]; then
    echo "FATAL: set CRATES_IO_TOKEN to a $NEW_USER token with change-owners scope." >&2
    exit 1
  fi
  for c in "${CRATES[@]}"; do
    crate_exists "$c" || { echo "SKIP $c — not published"; continue; }
    echo "== $c"
    cargo_owner_as "$CRATES_IO_TOKEN" --remove "github:$ORG:$TEAM" "$c" || true
  done
  echo
  echo "Only after 'status' shows no team owner, delete the GitHub team:"
  echo "    gh api -X DELETE orgs/$ORG/teams/$TEAM"
}

case "${1:-}" in
  status) cmd_status ;;
  invite) cmd_invite ;;
  finish) cmd_finish ;;
  drop-team) cmd_drop_team ;;
  *)
    sed -n '2,60p' "$0" >&2
    exit 1
    ;;
esac

# AFTERWARDS — two loose ends this script deliberately does not touch:
#
# 1. Local: delete CARGO_REGISTRY_TOKEN from .env. While it is there, direnv
#    injects it into every shell in this repo and `cargo login` is dead weight.
#    Use `cargo publish --token ...` or this script's explicit-token pattern.
#
# 2. CI: .github/workflows/publish-crates.yml:48 uses
#    secrets.CARGO_REGISTRY_TOKEN — a long-lived personal token belonging to
#    whoever created it. Replace it with crates.io trusted publishing (OIDC):
#
#        permissions:
#          contents: read
#          id-token: write
#        steps:
#          - uses: rust-lang/crates-io-auth-action@v1
#            id: auth
#          - env:
#              CARGO_REGISTRY_TOKEN: ${{ steps.auth.outputs.token }}
#            run: cargo publish $CRATES
#
#    Prerequisite: a user owner registers the trusted publisher (repository +
#    workflow filename) per crate on the crates.io website. NOTE the repo is
#    still at github.com/yurikrupnik-org/nx-playground — register whatever
#    owner/name the repo actually has when you do this, or move the repo to the
#    shluviza org first, because the OIDC claim is matched against it.
