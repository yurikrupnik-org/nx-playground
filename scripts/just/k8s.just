# Kubernetes operator installs (CNPG, Atlas, Crossplane) — imported by the root justfile.

# Composition functions (shared by all XRDs) — see platform/functions.yaml
[group('k8s-operators')]
crossplane-functions-install:
    kubectl apply -f platform/functions.yaml
    kubectl wait --for=condition=Healthy --timeout=300s function.pkg.crossplane.io/function-kcl function.pkg.crossplane.io/function-auto-ready

# Install CNPG operator (using server-side apply for large CRDs)
[group('k8s-operators')]
cnpg-install:
    kubectl apply --server-side -f https://raw.githubusercontent.com/cloudnative-pg/cloudnative-pg/release-1.24/releases/cnpg-1.24.0.yaml
    @echo "CNPG operator installed. Waiting for it to be ready..."
    kubectl wait --for=condition=available --timeout=120s deployment/cnpg-controller-manager -n cnpg-system

# Install Atlas Kubernetes operator
[group('k8s-operators')]
atlas-operator-install:
    helm install atlas-operator oci://ghcr.io/ariga/charts/atlas-operator --namespace atlas-operator --create-namespace
    @echo "Atlas operator installed"

# Install both operators (CNPG + Atlas) - run this first on a new cluster
[group('k8s-operators')]
operators-install:
    @echo "=== Installing Kubernetes Operators ==="
    @echo ""
    @echo "1. Installing CNPG operator..."
    just cnpg-install
    @echo ""
    @echo "2. Installing Atlas operator..."
    just atlas-operator-install
    @echo ""
    @echo "=== Operators Ready ==="
