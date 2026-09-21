---
name: deps-maintenance
description: Update dependencies or diagnose a failed upkg run. Use when the user asks to update/upgrade deps, runs just upkg/weekly, or pastes output containing BEGIN-UPKG-DIAGNOSIS or OSV-Scanner failures.
---

# Dependency maintenance

All dep updates go through `upkg` (from ~/dotconfig, source
`~/dotconfig/config/scripts/upkg.nu`). **Never** run raw
`cargo upgrade --incompatible` — it bulldozes range pins and skips scans/checks.

- `just upkg` — safe mode: OSV pre/post scan, --ignore-scripts, 7-day npm
  cooldown, transitive lockfile refresh, then `just check`
- `just upkg-fast` — no rails; follow with `just check`
- `just upkg-paranoid` — + cargo-vet, Socket org scan, sfw-proxied installs
- `just weekly` — upkg-paranoid + `just outdated` preview
- Preview only: `just outdated`

## Reading the diagnosis block

On failure upkg prints JSON between `BEGIN-UPKG-DIAGNOSIS`/`END-UPKG-DIAGNOSIS`:

- `updates[].ok == false` → that ecosystem's bump failed; typical cargo cause is
  a resolver conflict from a cross-major bump whose companion crate hasn't
  released support yet. Fix: exact-pin (`=x.y.z`) the lagging crate in the
  workspace Cargo.toml with a comment stating the unpin condition
  (`cargo upgrade` skips `=` pins; see the testcontainers precedent).
- `checks.failed` → post-update build/lint/test broke; run the named `just`
  recipe, fix code against new APIs. Pre-existing WIP breakage surfaces here too.
- `post_scan.ok == false` → vulnerabilities remain. Direct deps: bump. Transitive
  pinned by a parent: root package.json `overrides` (brace-expansion, uuid
  precedents). Second resolution fork keeping an old version alive: raise
  `requires-python` / the range floor that creates the fork (pytest precedent).
  Never-compiled lockfile-only deps: ignore it in the `osv-scanner.toml` sitting
  NEXT TO the lockfile osv-scanner cited — config is resolved per scanned file,
  so a root config never filters `apps/todo/web-leptos/Cargo.lock`. Mirror into
  the justfile `audit` / `.cargo/deny.toml` ignore lists ONLY if `cargo audit` /
  `cargo deny` report it too; both flag unmatched ignores as dead.
- `socket.ok == false` → usually auth: needs an API token for org `yuri`
  (`socket login`; scopes full-scans:create + repo:create). A non-empty
  `SOCKET_CLI_API_TOKEN` env var (shell or .env via dotenv-load) overrides the
  stored login. Note: the socket CLI exits 0 on some failures — upkg parses
  `"ok": false` from its JSON.

## Pinned crates

`testcontainers = "=0.27.3"` — unpin when testcontainers-modules requires ^0.28:
`curl -s https://index.crates.io/te/st/testcontainers-modules | tail -1`
