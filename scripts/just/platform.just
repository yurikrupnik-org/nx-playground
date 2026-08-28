# DevEnvironment platform (Crossplane) — imported by the root justfile.
# Slice 1: claim → postgres (CNPG) + redis + nats (helm) on kind.

# Install everything the platform needs on the current kind cluster
[group('platform')]
platform-install:
    just cnpg-install
    kubectl apply -f platform/dev-env/providers.yaml
    kubectl wait --for=condition=Healthy --timeout=180s provider.pkg.crossplane.io/provider-kubernetes provider.pkg.crossplane.io/provider-helm
    kubectl apply -f platform/dev-env/providerconfigs.yaml
    kubectl apply -f platform/dev-env/xrd.yaml
    kubectl apply -f platform/dev-env/composition.yaml
    @echo "platform installed — create an env with: just env-create <name>"

# Re-apply XRD + composition after editing them (fast inner loop)
[group('platform')]
platform-apply:
    kubectl apply -f platform/dev-env/xrd.yaml
    kubectl apply -f platform/dev-env/composition.yaml

# Create (or update) a dev environment claim
[group('platform')]
env-create name="demo" namespace="default":
    @printf 'apiVersion: platform.playground.io/v1alpha1\nkind: DevEnvironment\nmetadata:\n  name: %s\n  namespace: %s\nspec:\n  parameters: {}\n' "{{ name }}" "{{ namespace }}" | kubectl apply -f -

# Claim + composed resource status for one environment
[group('platform')]
env-status name="demo" namespace="default":
    kubectl get devenvironment -n {{ namespace }} {{ name }} -o wide
    -kubectl get objects.kubernetes.crossplane.io,releases.helm.crossplane.io -l crossplane.io/claim-name={{ name }}
    -kubectl get pods -n devenv-{{ name }}

# Tear an environment down
[group('platform')]
env-delete name="demo" namespace="default":
    kubectl delete devenvironment -n {{ namespace }} {{ name }}

# List all dev environments across namespaces
[group('platform')]
env-list:
    kubectl get devenvironments -A
