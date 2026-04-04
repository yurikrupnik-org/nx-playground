#!/usr/bin/env nu

# Infrastructure Mapping Commands
# Maps Docker Compose services to Helm charts, KCL packages, or kompose fallback.
# Config: manifests/infra-map.nuon

use common.nu *

const COMPOSE_FILE = "manifests/dockers/compose.yaml"

# Resolve the infra map file path
def infra-map-path []: nothing -> string {
    let root = (repo-root)
    $"($root)/manifests/infra-map.nuon"
}

# Load the infra mapping config
def load-map []: nothing -> record {
    let path = (infra-map-path)
    if not ($path | path exists) {
        error $"Infra map not found: ($path)"
        exit 1
    }
    open $path
}

# Save the infra mapping config
def save-map [map: record] {
    let path = (infra-map-path)
    $map | save -f $path
}

# Parse compose file and return service names
def compose-services []: nothing -> list<string> {
    let root = (repo-root)
    let compose_path = $"($root)/($COMPOSE_FILE)"
    if not ($compose_path | path exists) {
        error $"Compose file not found: ($compose_path)"
        return []
    }
    open $compose_path | get services | columns
}

# Build helm --set args from a values record
def build-helm-set-args [values: record]: nothing -> list<string> {
    $values | transpose key val | each {|row|
        ["--set" $"($row.key)=($row.val)"]
    } | flatten
}

# Check if a helm release exists
def helm-release-exists [name: string, namespace: string]: nothing -> bool {
    let result = (do { helm status $name -n $namespace } | complete)
    $result.exit_code == 0
}

# ============================================================================
# Commands
# ============================================================================

# List all infrastructure mappings with status
export def "main list" [
    --status (-s)  # Check deployed status (slower, queries cluster)
] {
    let map = (load-map)
    let compose_svcs = (compose-services)

    let rows = ($map | transpose name config | each {|entry|
        let cfg = $entry.config
        let compose_svc = ($cfg.compose_service? | default $entry.name)
        let in_compose = $compose_svc in $compose_svcs

        let deployed = if $status {
            match ($cfg.type? | default "kompose") {
                "helm" => {
                    let ns = ($cfg.namespace? | default "dbs")
                    if (helm-release-exists $entry.name $ns) { "yes" } else { "no" }
                }
                _ => "?"
            }
        } else {
            "-"
        }

        {
            name: $entry.name
            type: ($cfg.type? | default "kompose")
            namespace: ($cfg.namespace? | default "dbs")
            compose: $compose_svc
            in_compose: $in_compose
            deployed: $deployed
        }
    })

    $rows | table
}

# Show details for a specific service mapping
export def "main show" [
    name: string  # Service name from infra-map
] {
    let map = (load-map)
    if not ($name in ($map | columns)) {
        error $"Service '($name)' not found in infra map"
        return
    }
    let cfg = ($map | get $name)
    info $"Service: ($name)"
    $cfg
}

# Show compose services that are NOT mapped in infra-map
export def "main sync" [] {
    let map = (load-map)
    let compose_svcs = (compose-services)
    let mapped_compose = ($map | transpose name config | each {|e| $e.config.compose_service? | default $e.name})

    let unmapped = ($compose_svcs | where {|svc| not ($svc in $mapped_compose)})
    let orphaned = ($mapped_compose | where {|svc| not ($svc in $compose_svcs)})

    if ($unmapped | is-empty) {
        success "All compose services are mapped"
    } else {
        warn "Unmapped compose services:"
        $unmapped | each {|s| print $"  - ($s)"}
    }

    if not ($orphaned | is-empty) {
        warn "Mapped but not in compose (orphaned):"
        $orphaned | each {|s| print $"  - ($s)"}
    }
}

# Deploy a service using its mapped strategy
export def "main deploy" [
    name?: string            # Service name (omit for --all)
    --all (-a)               # Deploy all mapped services
    --namespace (-n): string # Override namespace
    --dry-run                # Preview without executing
    --force (-f)             # Force re-deploy even if exists
] {
    require-bin "kubectl"
    let map = (load-map)

    let targets = if $all {
        $map | transpose name config
    } else if ($name | is-not-empty) {
        if not ($name in ($map | columns)) {
            error $"Service '($name)' not found in infra map. Run 'infra list' to see available services."
            return
        }
        [{name: $name, config: ($map | get $name)}]
    } else {
        error "Provide a service name or use --all"
        return
    }

    for target in $targets {
        let cfg = $target.config
        let ns = ($namespace | default ($cfg.namespace? | default "dbs"))
        let svc_type = ($cfg.type? | default "kompose")

        info $"Deploying ($target.name) via ($svc_type) → namespace ($ns)"

        # Ensure namespace exists
        do { kubectl create namespace $ns } | complete

        match $svc_type {
            "helm" => {
                deploy-helm $target.name $cfg $ns $dry_run $force
            }
            "kcl" => {
                deploy-kcl $target.name $cfg $ns $dry_run
            }
            "kustomize" => {
                deploy-kustomize $target.name $cfg $ns $dry_run
            }
            "kompose" => {
                deploy-kompose $target.name $cfg $ns $dry_run
            }
            _ => {
                warn $"Unknown type '($svc_type)' for ($target.name), skipping"
            }
        }
    }
}

# Remove a deployed service
export def "main remove" [
    name: string             # Service name
    --namespace (-n): string # Override namespace
] {
    let map = (load-map)
    if not ($name in ($map | columns)) {
        error $"Service '($name)' not found in infra map"
        return
    }

    let cfg = ($map | get $name)
    let ns = ($namespace | default ($cfg.namespace? | default "dbs"))
    let svc_type = ($cfg.type? | default "kompose")

    match $svc_type {
        "helm" => {
            require-bin "helm"
            info $"Uninstalling helm release: ($name) from ($ns)"
            let result = (do { helm uninstall $name -n $ns } | complete)
            if $result.exit_code == 0 {
                success $"Removed ($name)"
            } else {
                warn $"Failed or not found: ($result.stderr)"
            }
        }
        _ => {
            warn $"Remove not implemented for type '($svc_type)'. Use kubectl delete manually."
        }
    }
}

# Add or update a service mapping
export def "main add" [
    name: string                          # Service name
    --type (-t): string = "helm"          # Deployment type: helm, kcl, kompose, kustomize
    --chart (-c): string                  # Helm chart reference
    --repo (-r): string                   # Helm repo URL (for non-OCI charts)
    --version: string                     # Chart version
    --namespace (-n): string = "dbs"      # Target namespace
    --compose-service: string             # Compose service name (defaults to name)
    --kcl-package: string                 # KCL package OCI ref
    --kustomize-path: string              # Kustomize overlay path
] {
    mut map = (load-map)

    let entry = match $type {
        "helm" => {
            if ($chart | is-empty) {
                error "Helm type requires --chart"
                return
            }
            mut cfg = {
                type: helm
                chart: $chart
                namespace: $namespace
                values: {}
                compose_service: ($compose_service | default $name)
            }
            if ($repo | is-not-empty) { $cfg = ($cfg | insert repo $repo) }
            if ($version | is-not-empty) { $cfg = ($cfg | insert version $version) } else { $cfg = ($cfg | insert version null) }
            $cfg
        }
        "kcl" => {
            if ($kcl_package | is-empty) {
                error "KCL type requires --kcl-package"
                return
            }
            {
                type: kcl
                package: $kcl_package
                namespace: $namespace
                args: {}
                compose_service: ($compose_service | default $name)
            }
        }
        "kustomize" => {
            {
                type: kustomize
                path: ($kustomize_path | default $"manifests/($name)/kustomize")
                namespace: $namespace
                compose_service: ($compose_service | default $name)
            }
        }
        "kompose" => {
            {
                type: kompose
                namespace: $namespace
                compose_service: ($compose_service | default $name)
            }
        }
        _ => {
            error $"Unknown type: ($type). Use: helm, kcl, kompose, kustomize"
            return
        }
    }

    $map = ($map | upsert $name $entry)
    save-map $map
    success $"Mapping '($name)' saved (type: ($type))"
}

# ============================================================================
# Deployment strategies
# ============================================================================

def deploy-helm [name: string, cfg: record, ns: string, dry_run: bool, force: bool] {
    require-bin "helm"

    let chart = ($cfg.chart? | default "")
    if ($chart | is-empty) {
        error $"No chart defined for ($name)"
        return
    }

    # Add repo if needed (non-OCI charts)
    if ($cfg.repo? | is-not-empty) {
        let repo_name = ($chart | split row "/" | get 0)
        let result = (do { helm repo add $repo_name ($cfg.repo) } | complete)
        if $result.exit_code == 0 {
            do { helm repo update $repo_name } | complete
        }
    }

    # Build command args
    mut args = ["upgrade" "--install" $name $chart "-n" $ns "--create-namespace"]

    if ($cfg.version? | is-not-empty) and ($cfg.version != null) {
        $args = ($args | append ["--version" $cfg.version])
    }

    if ($cfg.values? | is-not-empty) and ($cfg.values | columns | length) > 0 {
        let set_args = (build-helm-set-args $cfg.values)
        $args = ($args | append $set_args)
    }

    if $dry_run {
        $args = ($args | append "--dry-run")
    }

    if $force {
        $args = ($args | append "--force")
    }

    let final_args = $args
    info $"  helm ($final_args | str join ' ')"
    let result = (do { helm ...$final_args } | complete)

    if $result.exit_code == 0 {
        success $"  ($name) deployed via helm"
    } else {
        error $"  Failed: ($result.stderr)"
    }
}

def deploy-kcl [name: string, cfg: record, ns: string, dry_run: bool] {
    require-bin "kcl"

    let package = ($cfg.package? | default "")
    if ($package | is-empty) {
        error $"No KCL package defined for ($name)"
        return
    }

    # Build KCL args
    mut kcl_args = ["run" $package]
    if ($cfg.args? | is-not-empty) {
        let dargs = ($cfg.args | transpose key val | each {|r| $"-D ($r.key)=($r.val)"})
        $kcl_args = ($kcl_args | append $dargs)
    }

    let final_kcl_args = $kcl_args
    info $"  kcl ($final_kcl_args | str join ' ') | kubectl apply"
    let manifests = (do { kcl ...$final_kcl_args } | complete)
    if $manifests.exit_code != 0 {
        error $"  KCL render failed: ($manifests.stderr)"
        return
    }

    if $dry_run {
        $manifests.stdout | kubectl apply -n $ns -f - --dry-run=client
    } else {
        $manifests.stdout | kubectl apply -n $ns -f -
    }

    success $"  ($name) deployed via kcl"
}

def deploy-kustomize [name: string, cfg: record, ns: string, dry_run: bool] {
    let kpath = ($cfg.path? | default "")
    if ($kpath | is-empty) {
        error $"No kustomize path defined for ($name)"
        return
    }

    let root = (repo-root)
    let full_path = $"($root)/($kpath)"

    if not ($full_path | path exists) {
        error $"Kustomize path not found: ($full_path)"
        return
    }

    info $"  kubectl apply -k ($kpath) -n ($ns)"
    if $dry_run {
        kubectl apply -k $full_path -n $ns --dry-run=client
    } else {
        kubectl apply -k $full_path -n $ns
    }

    success $"  ($name) deployed via kustomize"
}

def deploy-kompose [name: string, cfg: record, ns: string, dry_run: bool] {
    require-bin "kompose"

    let root = (repo-root)
    let compose_path = $"($root)/($COMPOSE_FILE)"
    let compose_svc = ($cfg.compose_service? | default $name)

    info $"  Converting ($compose_svc) from compose via kompose"

    # kompose doesn't support single-service export, so we convert all and filter
    let manifests = (do { kompose convert --file $compose_path --namespace $ns --stdout } | complete)
    if $manifests.exit_code != 0 {
        error $"  Kompose failed: ($manifests.stderr)"
        return
    }

    # Filter manifests matching this service name
    let docs = ($manifests.stdout
        | split row "---"
        | where {|s| ($s | str trim) != ""}
        | where {|s|
            let doc = (try { $s | from yaml } catch { null })
            if ($doc | is-empty) { false } else {
                let meta_name = ($doc.metadata?.name? | default "")
                $meta_name == $compose_svc or ($meta_name | str starts-with $"($compose_svc)-")
            }
        }
        | str join "\n---\n"
    )

    if ($docs | str trim | is-empty) {
        warn $"  No manifests found for compose service '($compose_svc)'"
        return
    }

    if $dry_run {
        $docs | kubectl apply -n $ns -f - --dry-run=client
    } else {
        $docs | kubectl apply -n $ns -f -
    }

    success $"  ($name) deployed via kompose"
}

# ============================================================================
# Help
# ============================================================================

def main [] {
    print "Infrastructure Mapping Commands"
    print ""
    print "Maps Docker Compose services to Helm charts, KCL packages, or kompose."
    print "Config: manifests/infra-map.nuon"
    print ""
    print "Commands:"
    print "  list [--status]                 - List all service mappings"
    print "  show <name>                     - Show mapping details"
    print "  sync                            - Compare compose vs mapped services"
    print "  deploy <name> [--dry-run]       - Deploy a service"
    print "  deploy --all [--dry-run]        - Deploy all services"
    print "  remove <name>                   - Remove a deployed service"
    print "  add <name> --type helm --chart  - Add/update a mapping"
    print ""
    print "Examples:"
    print "  nu scripts/nu/infra.nu list --status"
    print "  nu scripts/nu/infra.nu deploy postgres --dry-run"
    print "  nu scripts/nu/infra.nu deploy --all"
    print "  nu scripts/nu/infra.nu sync"
    print "  nu scripts/nu/infra.nu add neo4j -t helm -c neo4j/neo4j -r https://helm.neo4j.com/neo4j"
    print "  nu scripts/nu/infra.nu remove redis"
}
