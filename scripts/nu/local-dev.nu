#!/usr/bin/env nu

# Local Development Commands
# Docker Compose and container management utilities

use common.nu *

# Resolve compose file(s) in current directory or by path
def resolve_compose_files [--file (-f): string]: nothing -> list<string> {
    if ($file | is-not-empty) {
        let abs_path = ($file | path expand)
        if not ($abs_path | path exists) {
            error $"Compose file not found: ($abs_path)"
            exit 1
        }
        return [$abs_path]
    }

    let standard_names = ["docker-compose.yml", "docker-compose.yaml", "compose.yml", "compose.yaml"]
    let found_files = (
        $standard_names
        | where { |name| [$env.PWD, $name] | path join | path exists }
        | each { |name| [$env.PWD, $name] | path join }
    )

    if ($found_files | is-empty) {
        # Check manifests/dockers as fallback
        let manifest_compose = "manifests/dockers/compose.yaml"
        if ($manifest_compose | path exists) {
            return [($manifest_compose | path expand)]
        }
        error "No compose file found. Use --file to specify a custom path."
        exit 1
    }

    [($found_files | get 0)]
}

# Build docker compose args
def build_docker_compose_args [
    --file (-f): string
    subcmd: string
    ...rest
]: nothing -> list<string> {
    let files = (resolve_compose_files --file $file)
    let file_args = ($files | each { |f| ["-f", $f] } | flatten)
    $file_args ++ [$subcmd] ++ $rest
}

# Start docker compose services
export def "main up" [
    --file (-f): string  # Custom compose file path
    --detach (-d)        # Run in background
    ...rest              # Additional args
] {
    require-bin "docker"

    let args = if $detach {
        (build_docker_compose_args --file $file "up" "-d" ...$rest)
    } else {
        (build_docker_compose_args --file $file "up" ...$rest)
    }

    info "Starting services..."
    docker compose ...$args
}

# Stop docker compose services
export def "main down" [
    --file (-f): string  # Custom compose file path
    --volumes (-v)       # Remove volumes
    ...rest              # Additional args
] {
    require-bin "docker"

    let args = if $volumes {
        (build_docker_compose_args --file $file "down" "-v" ...$rest)
    } else {
        (build_docker_compose_args --file $file "down" ...$rest)
    }

    info "Stopping services..."
    docker compose ...$args
}

# Show logs
export def "main logs" [
    --file (-f): string  # Custom compose file path
    --follow             # Follow log output
    service?: string     # Specific service
] {
    require-bin "docker"

    let args = if $follow {
        if ($service | is-not-empty) {
            (build_docker_compose_args --file $file "logs" "-f" $service)
        } else {
            (build_docker_compose_args --file $file "logs" "-f")
        }
    } else {
        if ($service | is-not-empty) {
            (build_docker_compose_args --file $file "logs" $service)
        } else {
            (build_docker_compose_args --file $file "logs")
        }
    }

    docker compose ...$args
}

# Show status
export def "main ps" [
    --file (-f): string  # Custom compose file path
] {
    require-bin "docker"

    let args = (build_docker_compose_args --file $file "ps")
    docker compose ...$args
}

# Restart services
export def "main restart" [
    --file (-f): string  # Custom compose file path
    service?: string     # Specific service
] {
    require-bin "docker"

    let args = if ($service | is-not-empty) {
        (build_docker_compose_args --file $file "restart" $service)
    } else {
        (build_docker_compose_args --file $file "restart")
    }

    info "Restarting services..."
    docker compose ...$args
}

# Prune docker resources
export def "main prune" [
    --all (-a)  # Remove all unused images, not just dangling
] {
    require-bin "docker"

    warn "This will remove unused Docker resources"

    if $all {
        docker system prune -af
        docker volume prune -af
    } else {
        docker system prune -f
        docker volume prune -f
    }

    success "Docker resources cleaned"
}

# Convert compose to Kubernetes manifests
export def "main kompose" [
    --file (-f): string  # Custom compose file path
    --namespace (-n): string = "default"  # Target namespace
    --stdout             # Output to stdout instead of files
] {
    require-bin "kompose"

    let files = (resolve_compose_files --file $file)
    let first_file = ($files | get 0)

    info $"Converting ($first_file) to Kubernetes manifests..."

    if $stdout {
        kompose convert --file $first_file --namespace $namespace --stdout
    } else {
        kompose convert --file $first_file --namespace $namespace
        success "Manifests generated"
    }
}

# Reset local environment
export def "main reset" [
    --file (-f): string  # Custom compose file path
] {
    warn "Resetting local environment (removing volumes)..."

    main down --file $file --volumes
    main prune
    main up --file $file --detach

    success "Local environment reset complete"
}

# ============================================================================
# mprocs — run dev processes via mprocs
# ============================================================================

const MPROCS_DIR = "manifests/mprocs"

# List available mprocs project configs
export def "main mprocs list" [] {
    let root = (repo-root)
    let dir = $"($root)/($MPROCS_DIR)"
    let preset_names = ["kind"]
    let all_files = (ls $dir | where name =~ '\.yaml$' | get name | path basename | each {|f| $f | str replace '.yaml' ''})
    let projects = ($all_files | where {|name| not ($name in $preset_names)})
    let presets = ($all_files | where {|name| $name in $preset_names})

    info "Project configs (can be merged):"
    for f in $projects { print $"  - ($f)" }

    if not ($presets | is-empty) {
        print ""
        info "Standalone presets:"
        for f in $presets { print $"  - ($f) — run: mprocs -c manifests/mprocs/($f).yaml" }
    }
}

# Launch mprocs with one or more project configs merged together
export def "main mprocs" [
    ...projects: string  # Project names to run (e.g. zerg matia). Omit for all.
] {
    require-bin "mprocs"

    let root = (repo-root)
    let dir = $"($root)/($MPROCS_DIR)"

    # Discover available project configs (exclude kind.yaml — standalone preset)
    let preset_names = ["kind"]
    let available = (ls $dir
        | where name =~ '\.yaml$'
        | get name
        | path basename
        | each {|f| $f | str replace '.yaml' ''}
        | where {|name| not ($name in $preset_names)}
    )

    # Determine which configs to merge
    let selected = if ($projects | is-empty) {
        # No args → merge all project configs
        $available
    } else {
        # Validate requested projects exist
        for p in $projects {
            let cfg_path = $"($dir)/($p).yaml"
            if not ($cfg_path | path exists) {
                error $"Config not found: ($p).yaml. Available: ($available | str join ', ')"
                return
            }
        }
        $projects
    }

    if ($selected | is-empty) {
        error "No project configs found"
        return
    }

    info $"Merging configs: ($selected | str join ', ')"

    # Merge procs sections from all selected configs
    mut merged_procs = {}
    for project in $selected {
        let cfg_path = $"($dir)/($project).yaml"
        let cfg = (open $cfg_path)
        let procs = ($cfg | get procs)
        $merged_procs = ($merged_procs | merge $procs)
    }

    let merged = { procs: $merged_procs }

    # Write to temp file and launch mprocs
    let tmp = (tmpfile "mprocs-merged")
    let tmp_yaml = $"($tmp).yaml"
    $merged | to yaml | save -f $tmp_yaml

    info $"Launching mprocs with ($merged_procs | columns | length) processes..."
    info $"  Processes: ($merged_procs | columns | str join ', ')"

    mprocs --config $tmp_yaml

    # Cleanup after mprocs exits
    rm -f $tmp_yaml
}

# Main help
def main [] {
    print "Local Development Commands"
    print ""
    print "Usage: nu scripts/nu/local-dev.nu <command>"
    print ""
    print "Commands:"
    print "  up [--file] [--detach]  - Start services"
    print "  down [--file] [--volumes] - Stop services"
    print "  logs [--file] [--follow] [service] - Show logs"
    print "  ps [--file]             - Show status"
    print "  restart [--file] [service] - Restart services"
    print "  prune [--all]           - Clean Docker resources"
    print "  kompose [--file] [--namespace] - Convert to K8s"
    print "  reset [--file]          - Reset environment"
    print "  mprocs [projects...]    - Launch mprocs (zerg, matia, or all)"
    print "  mprocs list             - Show available configs"
    print ""
    print "Examples:"
    print "  nu scripts/nu/local-dev.nu up -d"
    print "  nu scripts/nu/local-dev.nu logs --follow postgres"
    print "  nu scripts/nu/local-dev.nu reset"
    print "  nu scripts/nu/local-dev.nu mprocs              # all projects"
    print "  nu scripts/nu/local-dev.nu mprocs zerg         # zerg only"
    print "  nu scripts/nu/local-dev.nu mprocs zerg matia   # both"
    print "  nu scripts/nu/local-dev.nu mprocs list         # show available"
}
