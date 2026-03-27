import .schema

apps: [schema.AppConfig] = [
    {
        name = "api"
        component = "api-server"
        app_type = "be"
        port = 8080
        port_name = "http"
        image = "yurikrupnik/zerg-api"
        probes = {
            protocol = "http"
            health_path = "/health"
            ready_path = "/ready"
        }
        resources = {
            mem_request = "128Mi"
            cpu_request = "250m"
            mem_limit = "512Mi"
        }
        hpa = {enabled = True}
        tilt = {
            port_forward = "5221:8080"
            labels = ["backend"]
            deps = ["zerg-tasks", "zerg-shared-config"]
            build_arg_name = "zerg_api"
        }
    }
    {
        name = "tasks"
        component = "grpc-server"
        app_type = "be"
        port = 50051
        port_name = "grpc"
        image = "yurikrupnik/zerg-tasks"
        probes = {
            protocol = "grpc"
            port_name = "grpc"
        }
        resources = {
            mem_request = "128Mi"
            cpu_request = "250m"
            mem_limit = "512Mi"
        }
        hpa = {enabled = True}
        tilt = {
            port_forward = "50051:50051"
            labels = ["backend"]
            deps = ["zerg-shared-config"]
            build_arg_name = "zerg_tasks"
        }
    }
    {
        name = "email-nats"
        component = "email-worker"
        app_type = "worker"
        port = 8081
        port_name = "http"
        image = "yurikrupnik/zerg-email-nats"
        has_shared_secrets = False
        probes = {
            protocol = "http"
            health_path = "/health"
            ready_path = "/ready"
        }
        resources = {
            mem_request = "64Mi"
            cpu_request = "100m"
            mem_limit = "256Mi"
        }
        tilt = {
            labels = ["backend"]
            deps = ["zerg-shared-config"]
            build_arg_name = "zerg_email_nats"
        }
    }
    {
        name = "db-worker"
        component = "db-worker"
        app_type = "worker"
        port = 8081
        port_name = "http"
        image = "yurikrupnik/zerg-db-worker"
        has_shared_secrets = False
        probes = {
            protocol = "http"
            health_path = "/health"
            ready_path = "/ready"
        }
        resources = {
            mem_request = "64Mi"
            cpu_request = "100m"
            mem_limit = "256Mi"
        }
        tilt = {
            labels = ["backend"]
            deps = ["zerg-shared-config"]
            build_arg_name = "zerg_db_worker"
        }
    }
    {
        name = "web"
        component = "frontend"
        app_type = "fe"
        port = 8080
        port_name = "http"
        image = "yurikrupnik/zerg-web"
        run_as_user = 101
        termination_grace_period = 10
        probes = {
            protocol = "http"
            health_path = "/health"
            ready_path = "/health"
        }
        resources = {
            mem_request = "32Mi"
            cpu_request = "50m"
            mem_limit = "64Mi"
            cpu_limit = "100m"
        }
        tilt = {
            port_forward = "5206:8080"
            labels = ["frontend"]
            deps = []
            is_frontend = True
            dockerfile = "Dockerfile"
            docker_target = "nginx"
            build_cmd = "bun nx run zerg-web:build"
            build_arg_name = "zerg_web"
        }
    }
    # --- Matia apps ---
    {
        name = "api"
        component = "api-server"
        app_type = "be"
        namespace = "matia"
        port = 8080
        port_name = "http"
        image = "yurikrupnik/matia-api"
        probes = {
            protocol = "http"
            health_path = "/health"
            ready_path = "/ready"
        }
        resources = {
            mem_request = "128Mi"
            cpu_request = "250m"
            mem_limit = "512Mi"
        }
        tilt = {
            port_forward = "5230:8080"
            labels = ["backend"]
            deps = ["matia-shared-config"]
            build_arg_name = "matia_api"
        }
    }
    {
        name = "web"
        component = "frontend"
        app_type = "fe"
        namespace = "matia"
        port = 8080
        port_name = "http"
        image = "yurikrupnik/matia-web"
        run_as_user = 101
        termination_grace_period = 10
        probes = {
            protocol = "http"
            health_path = "/health"
            ready_path = "/health"
        }
        resources = {
            mem_request = "32Mi"
            cpu_request = "50m"
            mem_limit = "64Mi"
            cpu_limit = "100m"
        }
        tilt = {
            port_forward = "5231:8080"
            labels = ["frontend"]
            deps = []
            is_frontend = True
            dockerfile = "Dockerfile"
            docker_target = "nginx"
            build_cmd = "bun nx run matia-web:build"
            build_arg_name = "matia_web"
        }
    }
]
