# Todo vertical on the local kind cluster — imported by the root justfile.
#
#   just todo-deploy    build images -> kind load -> kustomize apply -> wait
#   just todo-smoke     end-to-end check through the todo-web nginx proxy
#   just todo-destroy   delete the todo namespace

todo_kind_cluster := env("KIND_CLUSTER", "kcl-e2e")

todo_images := "yurikrupnik/todo-web:dev yurikrupnik/todo-api:dev yurikrupnik/todo-worker:dev"

# Build the three todo images (web dist via nx, rust via cargo-chef Dockerfile).
# Hand-written on purpose: unlike the zerg/terran apps, the todo projects have no
# nx `container` target, and these fixed local `:dev` tags are what `todo-load`
# pushes into kind. If todo ever gains container targets, collapse this onto them.
[group('todo')]
todo-build:
    bun nx run todo-web:build
    docker build --build-arg DIST_PATH=apps/todo/web/dist --target nginx -f manifests/dockers/Dockerfile -t yurikrupnik/todo-web:dev .
    docker build --build-arg APP_NAME=todo_api -f manifests/dockers/rust.Dockerfile -t yurikrupnik/todo-api:dev .
    docker build --build-arg APP_NAME=todo_worker -f manifests/dockers/rust.Dockerfile -t yurikrupnik/todo-worker:dev .

# Load the todo images into the kind cluster
[group('todo')]
todo-load:
    kind load docker-image --name {{ todo_kind_cluster }} {{ todo_images }}

# Apply infra (namespace/postgres/nats) + the generated app manifests, wait for
# rollout. The apps come from manifests/k8s/apps (one rendered file per app,
# regenerate with `just k8s-gen`), so this recipe applies exactly what CI and
# Tilt build — it no longer names per-app overlays that no longer exist.
[group('todo')]
todo-apply:
    kubectl apply -k manifests/kustomize/todo
    kubectl apply -k manifests/k8s/apps
    for d in postgres nats todo-api todo-worker todo-web; do \
        kubectl -n todo rollout status deployment/$d --timeout=180s || exit 1; \
    done

# Full deploy: build -> load -> apply
[group('todo')]
todo-deploy: todo-build todo-load todo-apply

# End-to-end smoke test through the todo-web nginx proxy (create/list/complete)
[group('todo')]
todo-smoke:
    bash scripts/just/todo-smoke.sh

# Tear the todo namespace down
[group('todo')]
todo-destroy:
    kubectl delete namespace todo
