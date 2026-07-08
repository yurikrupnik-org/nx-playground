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
