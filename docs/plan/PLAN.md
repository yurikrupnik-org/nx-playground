# Repo Plan: Managing Open-Source Rust Resources

## Goal

This repo doubles as a learning ground for managing open-source Rust dependencies
end-to-end: selecting crates, keeping them current, scanning for vulnerabilities,
enforcing license policy, and knowing when and why to pin. Every concern below maps
to tooling already wired into the workspace — no new tools, no generic advice.

## Current tooling inventory

| Concern | Tool | Entry point |
| --- | --- | --- |
| Updates | `upkg` (nu script in dotconfig `config/scripts/upkg.nu`) | `just upkg` (daily: OSV scans + build/lint/test) · `just upkg-fast` (bump only) · `just upkg-paranoid` (adds cargo-vet + Socket). Never raw `cargo upgrade --incompatible` — it bulldozes range pins and skips OSV scans and post-checks. |
| Vulnerability audit | cargo-audit + cargo-deny | `just audit` (defined in `scripts/just/rust.just`), deny config `.cargo/deny.toml` |
| OSV scanning | OSV-Scanner | part of `just verify`; ignore list in `osv-scanner.toml` |
| Outdated preview | cargo-outdated | `just outdated-rust` (read-only); `just outdated` covers every ecosystem |
| License policy | cargo-deny `[licenses]` allowlist | `.cargo/deny.toml` — MIT, Apache-2.0 (+ LLVM-exception), BSD-2/3-Clause, ISC, Zlib, 0BSD, Unicode-3.0, CC0-1.0, MPL-2.0, BSL-1.0, OpenSSL, CDLA-Permissive-2.0 |
| Registry sources | cargo-deny `[sources]` | `.cargo/deny.toml` — `unknown-registry = "deny"`, only the crates.io index allowed; unknown git sources warn |
| Dep table hygiene | cargo-sort | `just sort-deps` (runs `cargo fmt` first, then `cargo sort --workspace`) |
| Cadence | just aggregates | `just check` (everyday gate) · `just verify` (pre-push) · `just weekly` (paranoid update + remaining cross-major preview) |

## Standing policies

- **Exact pins are deliberate and documented at the pin.** `testcontainers = "=0.27.3"`
  in the workspace `Cargo.toml` exists because testcontainers-modules 0.15.0 (its latest
  release) requires testcontainers ^0.27; bumping testcontainers alone makes the workspace
  unresolvable. The `=` is what makes `cargo upgrade --incompatible` (run by upkg) skip it —
  comments alone don't. Any new exact pin gets the same treatment: reason + unpin condition
  in a comment above it.
- **Known-unactionable advisories are ignored in synced places, never silently.**
  The `audit` recipe in `scripts/just/rust.just` ignores RUSTSEC-2026-0235 (rkyv 0.7).
  `osv-scanner.toml` carries the rkyv ignore with the same reasoning, and its header
  points back at the justfile recipe. cargo-deny keeps its own list in
  `.cargo/deny.toml`: RUSTSEC-2025-0134 (rustls-pemfile unmaintained) and
  RUSTSEC-2026-0173 (proc-macro-error2 unmaintained, transitive via sea-orm-rc).
  Adding an ignore means: state the reason inline, and add it to every scanner that
  reports it. **Ignores expire:** RUSTSEC-2023-0071 (rsa) was dropped once rsa left
  `Cargo.lock` — re-check each ignore's dep is still in the graph when touching this.
- **Prove "not in the graph" before ignoring on those grounds.** The rkyv ignore is
  justified by `cargo tree -i rkyv -e all --target all` returning nothing — a lockfile-only
  optional dep of rust_decimal that is never compiled.
- **Only crates.io.** New git or alternate-registry dependencies are a deliberate decision,
  not a drive-by.

## Learning roadmap

### Understand the graph

- [ ] Pick a transitive dependency and run `cargo tree -i <crate> -e all --target all` to
      learn exactly why it is in the graph.
- [ ] Run `cargo deny check --config .cargo/deny.toml` and read one advisory finding and one
      license finding end-to-end, including which config section decided the outcome.
- [ ] Contrast ecosystems: `bun nx graph` for the web side vs. cargo's dependency graph for
      the Rust side.

### Practice the update loop

- [ ] Run `just outdated-rust` and `just outdated` (both read-only) and read what would move.
- [ ] On a throwaway branch, run `just upkg-fast`, then `just check` — upkg-fast skips scans
      and tests on purpose, so the gate afterwards is mandatory.
- [ ] Compare against a full `just upkg` run and note which extra scans it performs.

### Triage an advisory

- [ ] Next time `just audit` or the OSV scan in `just verify` flags something, decide between
      fix (upgrade), pin, or ignore.
- [ ] If the decision is ignore: write the reason inline and sync every scanner that reports
      it (justfile `audit` recipe, `osv-scanner.toml`, `.cargo/deny.toml` as applicable).
- [ ] If the decision is fix: confirm with `just check` that the upgrade is behaviour-neutral.

### Pin/unpin decisions

- [ ] Check whether testcontainers-modules has released 0.28 support (inspect the sparse
      index dependencies of testcontainers-modules). *Last checked 2026-08-31: latest
      modules is still 0.15.0 requiring testcontainers `^0.27.0` — the pin stays.*
- [ ] If it has: remove the `=` pin on `testcontainers`, run `just upkg`, confirm
      `cargo nextest run --workspace` is green (testcontainers-backed tests need Docker
      running), and delete the now-stale pin note in `AGENTS.md`.

Clearing the last item is the exit criterion for this roadmap.

## Cadence

- Everyday: `just check` — formatting + all linters + all tests + supply-chain audit.
- Before push: `just verify` — everything in `check` plus proto lint and the OSV scan.
- Weekly: `just weekly` — `upkg-paranoid` followed by a preview of remaining cross-major bumps.
