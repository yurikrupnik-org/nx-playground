#!/usr/bin/env nu

# Kubernetes Cluster Management
# Kind cluster creation, management, and post-setup

use common.nu *

# Create a local Kind cluster using KCL configuration
export def "main create" [
    --name (-n): string = "kind"     # Cluster name
    --workers (-w): int = 0          # Number of worker nodes
    --db-workers (-d): int = 0       # Number of database-dedicated worker nodes (tainted)
    --ingress (-i)                   # Enable ingress (ports 80, 443)
    --verbose (-v)                   # Verbose output
] {
    require-bin "kind"
    require-bin "kubectl"
    require-bin "kcl"

    if (cluster-exists $name) {
        info $"Kind cluster '($name)' already exists - skipping creation"
        return
    }

    info $"Creating Kind cluster: ($name) [workers: ($workers), db-workers: ($db_workers), ingress: ($ingress)]"

    # Generate cluster config using KCL
    let tmp = (tmpfile $"kind-config-($name)")

    let cluster_kcl_package = "oci://europe-west1-docker.pkg.dev/yk-artifact-registry/kcl/cluster:0.0.1"
    let kcl_response = (kcl run $cluster_kcl_package -D workers=($workers) -D db_workers=($db_workers) -D ingress=($ingress) -D name=($name) | lines | skip while {|l| not ($l | str starts-with "items:")} | str join "\n" | from yaml)
    let config = $kcl_response | get items.0
    $config | to yaml | save -f $tmp --force

    kind create cluster --name $name --config $tmp

    rm -f $tmp
    return
    if $env.LAST_EXIT_CODE? == 1 {
        error "Failed to create cluster"
        exit 1
    }

    # Wait for cluster to be ready
    kubectl cluster-info --context $"kind-($name)"
    kubectl wait --for=condition=Ready nodes --all --timeout=180s
    kubectl -n kube-system rollout status deploy/coredns --timeout=180s

    success $"Kind cluster '($name)' created successfully"

    if $ingress {
      #istioctl install --set profile=demo -y
      #istioctl install --set profile=ambient --set values.pilot.env.PILOT_ENABLE_GATEWAY_API=true -y
      #kubectl apply -f https://raw.githubusercontent.com/istio/istio/release-1.28/samples/addons/kiali.yaml
      #kubectl apply -f https://raw.githubusercontent.com/istio/istio/release-1.28/samples/addons/prometheus.yaml
      #kubectl apply -f https://raw.githubusercontent.com/istio/istio/release-1.28/samples/addons/grafana.yaml

      # Wait for Istio
      #kubectl wait -n istio-system deployment/istiod --for=condition=Available --timeout=300s

      log info "Istio installed with Gateway API support"
    }
}

# Delete a Kind cluster
export def "main delete" [
    name?: string  # Cluster name (defaults to "kind")
] {
    require-bin "kind"

    let cluster_name = ($name | default "kind")

    if not (cluster-exists $cluster_name) {
        warn $"Cluster '($cluster_name)' does not exist"
        return
    }

    info $"Deleting Kind cluster: ($cluster_name)"
    kind delete cluster --name $cluster_name
    success $"Cluster '($cluster_name)' deleted"
}

# List all Kind clusters
export def "main list" [] {
    require-bin "kind"

    let clusters = (kind get clusters | lines | where {|it| $it | is-not-empty})

    if ($clusters | is-empty) {
        info "No Kind clusters found"
        return []
    }

    info $"Found ($clusters | length) Kind cluster(s):"
    $clusters | each {|c| print $"  - ($c)"}
    $clusters
}

# Get cluster status and context info
export def "main status" [
    --name (-n): string  # Specific cluster name
] {
    require-bin "kubectl"

    let result = require-cluster-connectivity

    print ""
    print $"Context: ($result.context)"
    print $"Nodes: ($result.nodes | length)"
    $result.nodes | each {|node| print $"  - ($node)"}

    # Get namespace summary
    let namespaces = (kubectl get ns -o jsonpath='{.items[*].metadata.name}' | split row ' ')
    print ""
    print $"Namespaces: ($namespaces | length)"
}

# Post-cluster setup - deploy common infrastructure
export def "main setup" [
    --flux                           # Bootstrap Flux GitOps
    --flux-repo: string = "gitops"   # Flux repository name
    --istio                          # Install Istio
    --dbs                            # Deploy database services from compose
] {
    require-bin "kubectl"
    require-cluster-connectivity

    if $dbs {
        info "Deploying database services..."
        require-bin "kompose"

        let compose_file = "manifests/dockers/compose.yaml"
        if ($compose_file | path exists) {
            do { kubectl create namespace dbs } | complete
            let manifests = (kompose convert --file $compose_file --namespace dbs --stdout)
            # Patch manifests for node placement and Istio protocol detection
            # Port names must use tcp-* prefix so the client-side Istio sidecar
            # (in zerg namespace) treats non-HTTP traffic as raw TCP passthrough.
            let patched = ($manifests
                | split row "---"
                | where {|s| ($s | str trim) != ""}
                | each {|s|
                    let doc = ($s | from yaml)
                    if ($doc.kind? == "Deployment") {
                        $doc | upsert spec.template.spec.tolerations [{
                            key: "dedicated"
                            value: "database"
                            effect: "NoSchedule"
                        }] | upsert spec.template.spec.nodeSelector { dedicated: "database" }
                    } else if ($doc.kind? == "Service") {
                        let svc_name = ($doc.metadata.name? | default "unknown")
                        $doc | upsert spec.ports ($doc.spec.ports | each {|p|
                            $p | upsert name $"tcp-($svc_name)-($p.port)"
                        })
                    } else {
                        $doc
                    } | to yaml
                }
                | str join "---\n")
            $patched | kubectl apply -f -
            success "Database services deployed to 'dbs' namespace (on db-worker nodes)"
        } else {
            warn "Compose file not found at $compose_file"
        }
    }

    if $istio {
        info "Installing Istio..."
        require-bin "istioctl"
        istioctl install --set profile=ambient --skip-confirmation
        success "Istio installed"
    }

    if $flux {
        info "Bootstrapping Flux..."
        require-bin "flux"
        require-bin "gh"

        let token_result = (do { gh auth token } | complete)
        if $token_result.exit_code != 0 {
            error "GitHub CLI not authenticated. Run 'gh auth login' first."
            exit 1
        }

        let owner = (gh api user --jq '.login' | str trim)
        let token = ($token_result.stdout | str trim)

        with-env { GITHUB_TOKEN: $token } {
            flux bootstrap github --owner $owner --repository $flux_repo --branch main --path clusters/local --personal
        }
        success "Flux bootstrapped"
    }
}

# Run migrations against cluster database
export def "main migrate" [
    --port (-p): int = 5433          # Database port
    --user (-u): string = "myuser"   # Database user
    --password: string = "mypassword"  # Database password
    --database (-d): string = "mydatabase"  # Database name
] {
    require-bin "cargo"

    let db_url = $"postgres://($user):($password)@localhost:($port)/($database)"
    info $"Running migrations against localhost:($port)/($database)"

    with-env { DATABASE_URL: $db_url } {
        cargo run -p migration -- up
    }

    success "Migrations complete"
}

# Deploy GitOps resources using Kustomize
export def "main gitops" [
    --target (-e): string = "dev"     # Environment: dev or prod
    --dry-run                          # Preview without applying
] {
    require-bin "kubectl"
    require-cluster-connectivity

    let gitops_path = $"k8s/gitops/overlays/($target)"

    if not ($gitops_path | path exists) {
        error $"GitOps overlay not found: ($gitops_path)"
        exit 1
    }

    info $"Deploying GitOps resources for ($target) environment..."

    if $dry_run {
        kubectl apply -k $gitops_path --dry-run=client
    } else {
        kubectl apply -k $gitops_path
        success $"GitOps resources deployed for ($target)"
    }
}

# Deploy observability stack (Prometheus/Grafana)
export def "main observability" [
    --target (-e): string = "dev"     # Environment: dev or prod
    --dry-run                          # Preview without applying
] {
    require-bin "kubectl"
    require-cluster-connectivity

    let obs_path = $"k8s/observability/overlays/($target)"

    if not ($obs_path | path exists) {
        error $"Observability overlay not found: ($obs_path)"
        exit 1
    }

    info $"Deploying observability stack for ($target) environment..."

    if $dry_run {
        kubectl apply -k $obs_path --dry-run=client
    } else {
        # Create monitoring namespace first
        do { kubectl create namespace monitoring } | complete

        kubectl apply -k $obs_path
        success $"Observability stack deployed for ($target)"

        if $target == "dev" {
            info "Prometheus will be available after Flux reconciles the HelmRelease"
            info "Check status: flux get helmreleases -n monitoring"
        }
    }
}

# Full local dev environment setup
export def "main local-dev" [
    --name (-n): string = "dev"    # Cluster name
    --workers (-w): int = 1        # Number of worker nodes
    --skip-cluster                  # Skip cluster creation
    --skip-dbs                      # Skip database deployment
    --skip-observability            # Skip observability stack
] {
    info "Setting up full local development environment..."

    # Step 1: Create Kind cluster
    if not $skip_cluster {
        main create --name $name --workers $workers --ingress
    }

    # Step 2: Deploy databases
    if not $skip_dbs {
        main setup --dbs
    }

    # Step 3: Deploy core infrastructure via GitOps
    info "Deploying core infrastructure..."
    kubectl apply -k k8s/core/overlays/dev

    # Step 4: Deploy observability
    if not $skip_observability {
        main observability --target dev
    }

    # Step 5: Wait for services
    info "Waiting for services to be ready..."
    kubectl -n dbs wait --for=condition=Available deployment --all --timeout=300s 2>/dev/null | complete

    success "Local development environment ready!"
    print ""
    print "Available endpoints:"
    print "  - API: http://localhost:8080"
    print "  - Grafana: http://localhost:3000 (after flux reconcile)"
    print "  - Prometheus: http://localhost:9090 (after flux reconcile)"
    print ""
    print "Port forward commands:"
    print "  kubectl port-forward -n monitoring svc/kube-prometheus-stack-grafana 3000:80"
    print "  kubectl port-forward -n monitoring svc/kube-prometheus-stack-prometheus 9090:9090"
}

# Teardown local dev environment
export def "main teardown" [
    --name (-n): string = "dev"    # Cluster name
    --keep-cluster                  # Keep the cluster, only remove resources
] {
    require-bin "kubectl"

    warn "Tearing down local development environment..."

    if $keep_cluster {
        info "Removing resources but keeping cluster..."
        do { kubectl delete -k k8s/observability/overlays/dev } | complete
        do { kubectl delete -k k8s/core/overlays/dev } | complete
        do { kubectl delete ns dbs } | complete
        success "Resources removed, cluster kept"
    } else {
        main delete $name
        success "Local development environment torn down"
    }
}

# Main help
def main [] {
    print "Kubernetes Cluster Management"
    print ""
    print "Usage: nu scripts/nu/cluster.nu <command>"
    print ""
    print "Commands:"
    print "  create [--name] [--workers] [--ingress]  - Create Kind cluster"
    print "  delete [name]                            - Delete Kind cluster"
    print "  list                                     - List all Kind clusters"
    print "  status [--name]                          - Show cluster status"
    print "  setup [--flux] [--istio] [--dbs]         - Post-cluster setup"
    print "  migrate [--port] [--user] [--database]   - Run DB migrations"
    print "  gitops [--target] [--dry-run]            - Deploy GitOps resources"
    print "  observability [--target] [--dry-run]     - Deploy observability stack"
    print "  local-dev [--name] [--workers]           - Full local dev setup"
    print "  teardown [--name] [--keep-cluster]       - Teardown local env"
    print ""
    print "Examples:"
    print "  nu scripts/nu/cluster.nu create -n dev -w 2 -i"
    print "  nu scripts/nu/cluster.nu local-dev -n dev -w 1"
    print "  nu scripts/nu/cluster.nu gitops -e dev --dry-run"
    print "  nu scripts/nu/cluster.nu observability -e dev"
    print "  nu scripts/nu/cluster.nu teardown -n dev"
}
