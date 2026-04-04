#!/usr/bin/env nu

# Monorepo Nu Scripts - Main Entry Point
# Usage: nu scripts/nu/mod.nu <command> [args]

export use common.nu *

const SCRIPT_DIR = path self

# ============================================================================
# Configuration
# ============================================================================

const CONFIG = {
    gitops_repo: "~/really-private/gitops-v1"
    gcp_credentials: "~/dotconfig/tmp/secret-puller.json"
    default_cluster: "dev"
    default_workers: 2
}

# ============================================================================
# Prerequisites Check
# ============================================================================

# Check all required tools are installed
def check-prerequisites []: nothing -> bool {
    let required = ["kind" "kubectl" "tilt" "kcl" "kompose", "istioctl"]
    let optional = ["flux" "gh" "helm"]

    info "Checking prerequisites..."

    mut all_found = true
    for cmd in $required {
        if (command-exists $cmd) {
            success $"  ($cmd)"
        } else {
            error $"  ($cmd) - NOT FOUND (required)"
            $all_found = false
        }
    }

    for cmd in $optional {
        if (command-exists $cmd) {
            success $"  ($cmd)"
        } else {
            warn $"  ($cmd) - not found (optional)"
        }
    }

    $all_found
}

export def "main frankenstien" [
  --name (-n): string = "dev"      # Cluster name
  --workers (-w): int = 2          # Number of worker nodes
  --skip-dbs                       # Skip database deployment
  --skip-secrets                   # Skip external-secrets setup
  --skip-obs                       # Skip observability stack
  --skip-tilt                      # Skip starting Tilt
  --flux                           # Bootstrap Flux GitOps
  --dry-run                        # Preview without executing
  --verbose (-v)
] {
  create-app-namespaces
}
# ============================================================================
# Top-level up/down commands - Full environment lifecycle
# ============================================================================

# Bring up the full local development environment
export def "main up" [
    --name (-n): string = "dev"      # Cluster name
    --workers (-w): int = 2          # Number of worker nodes
    --skip-dbs                       # Skip database deployment
    --skip-secrets                   # Skip external-secrets setup
    --skip-obs                       # Skip observability stack
    --skip-tilt                      # Skip starting Tilt
    --flux                           # Bootstrap Flux GitOps
    --dry-run                        # Preview without executing
    --verbose (-v)                   # Verbose output
] {
    if not (check-prerequisites) {
        error "Missing required prerequisites. Please install them first."
        exit 1
    }

    if $dry_run {
        info "[DRY-RUN] Would create Kind cluster and deploy services"
        info $"  Cluster: ($name), Workers: ($workers)"
        info $"  DBs: (not $skip_dbs), Secrets: (not $skip_secrets), Obs: (not $skip_obs)"
        return
    }

    let start_time = date now

    # Step 1: Create Kind cluster
    info $"Step 1/6: Creating Kind cluster '($name)' with ($workers) workers..."
    ^nu ($SCRIPT_DIR | path dirname | path join "cluster.nu") create -n $name -w $workers --ingress -d 1

    # Step 2: Create app namespaces
    info "Step 2/6: Creating app namespaces..."
    create-app-namespaces

    # Step 3: Setup External Secrets
    if not $skip_secrets {
        info "Step 3/6: Setting up External Secrets..."
        setup-external-secrets
    } else {
        info "Step 3/6: Skipping External Secrets setup"
    }

    # Step 4: Deploy databases
    if not $skip_dbs {
        info "Step 4/6: Deploying database services..."
        ^nu ($SCRIPT_DIR | path dirname | path join "cluster.nu") setup --dbs
        wait-for-databases
    } else {
        info "Step 4/6: Skipping database deployment"
    }

    # Step 5: Deploy observability (optional)
    #if not $skip_obs {
    #    info "Step 5/6: Deploying observability stack..."
    #    do { ^nu ($SCRIPT_DIR | path dirname | path join "cluster.nu") observability --target dev } | complete
    #} else {
    #    info "Step 5/6: Skipping observability stack"
    #}

    # Step 6: Bootstrap Flux (optional)
    if $flux {
        info "Step 6/6: Bootstrapping Flux GitOps..."
        bootstrap-flux
    } else {
        info "Step 6/6: Skipping Flux bootstrap (use --flux to enable)"
    }

    let elapsed = (date now) - $start_time

    success $"Environment '($name)' is ready! (($elapsed | format duration sec))"
    print ""
    print-endpoints

    # Start Tilt
    if not $skip_tilt {
        print ""
        info "Starting Tilt..."
        ^tilt up
    } else {
        print ""
        print "Next steps:"
        print "  - Run 'tilt up' to start application development"
        print "  - Run 'just local-down' to tear down the environment"
    }
}

# Tear down the local development environment
export def "main down" [
    --name (-n): string = "dev"      # Cluster name
    --keep-cluster                   # Keep cluster, only remove resources
    --verbose (-v)                   # Verbose output
] {
    require-bin "kind"

    # Stop Tilt first
    info "Stopping Tilt..."
    do { ^tilt down } | complete

    if $keep_cluster {
        info "Removing resources but keeping cluster..."
        do { kubectl delete ns zerg --ignore-not-found } | complete
        do { kubectl delete ns dbs --ignore-not-found } | complete
        do { kubectl delete ns monitoring --ignore-not-found } | complete
        do { kubectl delete ns external-secrets --ignore-not-found } | complete
        success "Resources removed, cluster kept"
        print ""
        print "Cluster '($name)' is still running. To delete it:"
        print $"  just local-down  # or: kind delete cluster --name ($name)"
    } else {
        info $"Deleting Kind cluster: ($name)"
        ^nu ($SCRIPT_DIR | path dirname | path join "cluster.nu") delete $name
        success $"Environment '($name)' is down"
    }
}

# ============================================================================
# Helper Functions
# ============================================================================

# Setup External Secrets with GCP credentials
def setup-external-secrets [] {
    let creds_path = ($CONFIG.gcp_credentials | path expand)

    if not ($creds_path | path exists) {
        warn $"GCP credentials not found at ($creds_path)"
        warn "External Secrets will not be able to pull from GCP Secret Manager"
        return
    }

    do { kubectl create namespace external-secrets } | complete

    let result = do {
        kubectl create secret generic gcp-sm-credentials -n external-secrets --from-file=credentials=($creds_path)
    } | complete

    if $result.exit_code == 0 {
        success "External Secrets configured with GCP credentials"
    } else if ($result.stderr | str contains "already exists") {
        info "External Secrets credentials already configured"
    } else {
        warn $"Failed to create secret: ($result.stderr)"
    }
}

# Bootstrap Flux GitOps
def bootstrap-flux [
  repository: string = $CONFIG.gitops_repo
  branch: string = "main"
  path: string = "/clusters/mgmt"
] {
    require-bin "flux"
    require-bin "gh"

    let token_result = do { gh auth token } | complete
    if $token_result.exit_code != 0 {
        error "GitHub CLI not authenticated. Run 'gh auth login' first."
        return
    }

    let owner = (gh api user --jq '.login' | str trim)
    let token = ($token_result.stdout | str trim)

    info $"Bootstrapping Flux for ($owner)/gitops-v1..."

    with-env { GITHUB_TOKEN: $token } {
        flux bootstrap github --owner $owner --repository gitops-v1 --branch main --path /clusters/mgmt --personal --components-extra image-reflector-controller,image-automation-controller
    }

    success "Flux bootstrapped successfully"
}

# Wait for database pods to be ready
def wait-for-databases [] {
    info "Waiting for databases to be ready..."

    let result = do {
        kubectl wait --for=condition=Available deployment --all -n dbs --timeout=120s
    } | complete

    if $result.exit_code == 0 {
        success "All databases are ready"
    } else {
        warn "Some databases may not be ready yet"
    }
}

# Print available endpoints
def print-endpoints [] {
    print "Available endpoints (after Tilt starts):"
    print "  ┌─────────────────────────────────────────────┐"
    print "  │ Tilt UI:      http://localhost:10350        │"
    print "  │ API:          http://localhost:5221/api     │"
    print "  │ Web:          http://localhost:5173         │"
    print "  │ Postgres:     localhost:5433                │"
    print "  │ Redis:        localhost:6379                │"
    print "  │ MongoDB:      localhost:27017               │"
    print "  └─────────────────────────────────────────────┘"
}

# Show environment status
export def "main status" [
    --cloud (-c): string = "local"   # Cloud provider
] {
    match $cloud {
        "local" => {
            ^nu ($SCRIPT_DIR | path dirname | path join "cluster.nu") list
            ^nu ($SCRIPT_DIR | path dirname | path join "cluster.nu") status
        }
        _ => {
            warn $"Status for ($cloud) not yet implemented"
        }
    }
}

# Validate cloud provider
def validate-provider [cloud: string] {
    let valid = ["local" "aws" "gcp" "azure"]
    if not ($cloud in $valid) {
        error $"Invalid cloud provider: ($cloud). Valid options: ($valid | str join ', ')"
        exit 1
    }
}

# Create app namespaces based on apps directory structure
def create-app-namespaces [] {
    let apps_dir = "apps"

    if not ($apps_dir | path exists) {
        warn "apps/ directory not found, skipping namespace creation"
        return
    }

    # Get top-level directories in apps/ - these become namespaces
    let namespaces = (ls $apps_dir | where type == dir | get name | path basename)

    for ns in $namespaces {
        info $"  Creating namespace: ($ns)"
        do { kubectl create namespace $ns } | complete
    }

    success $"Created ($namespaces | length) app namespaces: ($namespaces | str join ', ')"
}

# ============================================================================
# Subcommand delegators
# ============================================================================

# Setup commands - install dependencies, build, check
export def --wrapped "main setup" [...args] {
    let script = ($SCRIPT_DIR | path dirname | path join "setup.nu")
    ^nu $script ...$args
}

# Local development - docker compose, prune, kompose
export def --wrapped "main dev" [...args] {
    let script = ($SCRIPT_DIR | path dirname | path join "local-dev.nu")
    ^nu $script ...$args
}

# Cluster management - create, delete, status, gitops
export def --wrapped "main cluster" [...args] {
    let script = ($SCRIPT_DIR | path dirname | path join "cluster.nu")
    ^nu $script ...$args
}

# Secrets management - fetch, verify, load
export def --wrapped "main secrets" [...args] {
    let script = ($SCRIPT_DIR | path dirname | path join "secrets.nu")
    ^nu $script ...$args
}

# Infrastructure mapping - compose → helm/kcl/kompose
export def --wrapped "main infra" [...args] {
    let script = ($SCRIPT_DIR | path dirname | path join "infra.nu")
    ^nu $script ...$args
}

# Main help
def main [] {
    print "Monorepo Nu Scripts"
    print ""
    print "Usage: nu scripts/nu/mod.nu <command> [args]"
    print ""
    print "Quick Start:"
    print "  up      - Bring up full dev environment (cluster + services)"
    print "  down    - Tear down dev environment"
    print "  status  - Show environment status"
    print ""
    print "Subcommands:"
    print "  setup   - Project setup (install, build, check, test)"
    print "  dev     - Docker compose (up, down, logs, kompose, prune)"
    print "  cluster - Kind cluster (create, delete, gitops, observability)"
    print "  secrets - Secrets management (fetch, verify, load)"
    print "  infra   - Infrastructure mapping (compose → helm/kcl/kompose)"
    print ""
    print "Examples:"
    print "  nu scripts/nu/mod.nu up                    # Local Kind cluster + services"
    print "  nu scripts/nu/mod.nu up -c aws -n prod     # AWS EKS cluster"
    print "  nu scripts/nu/mod.nu down                  # Tear down local env"
    print "  nu scripts/nu/mod.nu status                # Show cluster status"
    print ""
    print "  nu scripts/nu/mod.nu setup install --all"
    print "  nu scripts/nu/mod.nu dev up -d"
    print "  nu scripts/nu/mod.nu cluster create -n dev -w 2"
    print "  nu scripts/nu/mod.nu secrets fetch"
}
