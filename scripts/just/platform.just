# DevEnvironment + CloudInventory platform (Crossplane) — imported by the root justfile.
# Slice 1: claim → postgres (CNPG) + redis + nats (helm) on kind.
# Slice 2: CloudInventory claim → observe-only mirrors of existing resources.

# Install everything the platform needs on the current kind cluster
[group('platform')]
platform-install:
    just cnpg-install
    kubectl apply -f platform/dev-env/providers.yaml
    kubectl wait --for=condition=Healthy --timeout=180s provider.pkg.crossplane.io/provider-kubernetes provider.pkg.crossplane.io/provider-helm
    just crossplane-functions-install
    kubectl apply -f platform/dev-env/providerconfigs.yaml
    kubectl apply -f platform/dev-env/xrd.yaml
    kubectl apply -f platform/dev-env/composition.yaml
    kubectl apply -f platform/cloud-inventory/xrd.yaml
    kubectl apply -f platform/cloud-inventory/composition.yaml
    @echo "platform installed — create an env with: just env-create <name>, an inventory with: just inventory-create"

# Re-apply XRD + composition after editing them (fast inner loop)
[group('platform')]
platform-apply:
    kubectl apply -f platform/dev-env/xrd.yaml
    kubectl apply -f platform/dev-env/composition.yaml
    kubectl apply -f platform/cloud-inventory/xrd.yaml
    kubectl apply -f platform/cloud-inventory/composition.yaml

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

# Apply the demo cloud inventory claim (observe-only, mutates nothing)
[group('platform')]
inventory-create:
    kubectl apply -f platform/cloud-inventory/examples/demo.yaml

# Claim + observed Object status for one inventory
[group('platform')]
inventory-status name="demo" namespace="default":
    kubectl get cloudinventory -n {{ namespace }} {{ name }} -o wide
    -kubectl get objects.kubernetes.crossplane.io -l platform.playground.io/inventory={{ name }}

# Delete an inventory claim — observed resources are left untouched
[group('platform')]
inventory-delete name="demo" namespace="default":
    kubectl delete cloudinventory -n {{ namespace }} {{ name }}

# List all cloud inventories across namespaces
[group('platform')]
inventory-list:
    kubectl get cloudinventories -A
