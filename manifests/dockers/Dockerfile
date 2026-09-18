# Static SPA server images. Every target has the same contract so k8s manifests
# and benchmarks can swap them freely:
#   - serves DIST_PATH on :8080 as an unprivileged user
#   - GET /health -> 200 (probes)
#   - SPA fallback: unknown paths serve index.html
#   - gzip compression
#   - /api reverse proxy to $UPSTREAM_HOST:$UPSTREAM_PORT (nginx + caddy only;
#     static-web-server is static-only)
# Compare them with: just test-web-servers / just bench-web-compare

# Using nginx-unprivileged for better K8s security
# - Runs as non-root (UID 101)
# - Listens on port 8080 (unprivileged)
# - Compatible with K8s restricted pod security standards
FROM nginxinc/nginx-unprivileged:alpine AS nginx
WORKDIR /app
ARG DIST_PATH
RUN test -n "$DIST_PATH" || (echo "DIST_PATH not set" && false)
COPY ./$DIST_PATH /usr/share/nginx/html
COPY manifests/dockers/default.conf.template /etc/nginx/templates/default.conf.template
ENV PORT=8080
EXPOSE ${PORT}
CMD ["nginx", "-g", "daemon off;"]

# ============================================================================
# Caddy — single Go binary, config via Caddyfile ({$ENV} resolved at runtime).
# ============================================================================
FROM caddy:2-alpine AS caddy
ARG DIST_PATH
RUN test -n "$DIST_PATH" || (echo "DIST_PATH not set" && false)
COPY ./$DIST_PATH /srv
COPY manifests/dockers/Caddyfile /etc/caddy/Caddyfile
# Run unprivileged (parity with nginx-unprivileged); admin API + config
# persistence are off in the Caddyfile, so only XDG dirs need to be writable.
ENV XDG_CONFIG_HOME=/tmp/caddy-config \
    XDG_DATA_HOME=/tmp/caddy-data \
    PORT=8080
USER 65534:65534
EXPOSE ${PORT}
CMD ["caddy", "run", "--config", "/etc/caddy/Caddyfile"]

# ============================================================================
# static-web-server — tiny Rust binary, static-only (NO /api proxy; pair it
# with a Gateway/HTTPRoute that routes /api separately if used in k8s).
# ============================================================================
FROM ghcr.io/static-web-server/static-web-server:2-alpine AS static-web-server
ARG DIST_PATH
RUN test -n "$DIST_PATH" || (echo "DIST_PATH not set" && false)
COPY ./$DIST_PATH /public
ENV SERVER_PORT=8080 \
    SERVER_ROOT=/public \
    SERVER_FALLBACK_PAGE=/public/index.html \
    SERVER_HEALTH=true \
    SERVER_COMPRESSION=true \
    SERVER_CACHE_CONTROL_HEADERS=true
USER 65534:65534
EXPOSE 8080
CMD ["static-web-server"]
