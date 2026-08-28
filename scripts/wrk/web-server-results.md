# Static web server comparison — nginx vs caddy vs static-web-server

Images from `manifests/dockers/Dockerfile` targets (`nginx`, `caddy`,
`static-web-server`), all serving the identical `apps/todo/web/dist` SPA build.

Reproduce:

```bash
just test-web-servers     # behavior-parity checks (health, SPA fallback, gzip, caching)
just bench-web-compare    # this benchmark
```

## 2026-08-26 — Apple M4 Max, Docker Desktop (arm64), wrk -t4 -c64 -d10s

| server | path | req/s | p50 | p75 | p90 | p99 |
|---|---|---|---|---|---|---|
| nginx | / | 47671 | 1.16ms | 1.59ms | 2.49ms | 12.38ms |
| caddy | / | 32879 | 1.65ms | 2.55ms | 4.17ms | 9.41ms |
| static-web-server | / | 72749 | 0.83ms | 1.00ms | 1.21ms | 2.41ms |
| nginx | /assets/index-*.js | 44725 | 1.35ms | 1.56ms | 1.89ms | 3.35ms |
| caddy | /assets/index-*.js | 32823 | 1.83ms | 2.29ms | 2.90ms | 5.52ms |
| static-web-server | /assets/index-*.js | 43025 | 1.35ms | 1.72ms | 2.24ms | 5.68ms |

## Takeaways

- **static-web-server** (Rust) is the fastest on `index.html` (~1.5x nginx
  throughput, p99 2.4ms vs 12.4ms) and matches nginx on the JS asset. It is
  static-only: no `/api` reverse proxy, so it needs a gateway/HTTPRoute to
  route `/api` separately.
- **nginx** (current default) stays the best all-rounder: near-top throughput
  plus the `/api` proxy the SPA deployments rely on.
- **caddy** trails ~30% on throughput but has the tightest index p99 among
  the proxy-capable options and the simplest config surface.

All three pass the same parity suite (health endpoint, SPA fallback,
content types, immutable asset caching, gzip) — `just test-web-servers`.
