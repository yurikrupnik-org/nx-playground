schema Resources:
    mem_request: str = "128Mi"
    cpu_request: str = "250m"
    mem_limit: str = "512Mi"
    cpu_limit?: str = None

schema ProbeConfig:
    path?: str = None
    port_name: str = "http"
    protocol: str = "http"  # "http" or "grpc"
    startup_initial_delay: int = 2
    startup_period: int = 5
    startup_failure: int = 6
    startup_timeout: int = 3
    liveness_period: int = 10
    liveness_failure: int = 3
    liveness_timeout: int = 3
    readiness_period: int = 5
    readiness_failure: int = 3
    readiness_timeout: int = 2
    health_path?: str = None
    ready_path?: str = None

schema HPAConfig:
    enabled: bool = False
    min_replicas: int = 1
    max_replicas: int = 10
    cpu_target: int = 70
    memory_target: int = 80

schema TiltConfig:
    port_forward?: str = None
    labels: [str] = ["backend"]
    deps: [str] = ["zerg-shared-config"]
    dockerfile: str = "rust.Dockerfile"
    docker_target: str = "rust"
    build_arg_name?: str = None
    is_frontend: bool = False
    build_cmd?: str = None

schema AppConfig:
    name: str
    component: str
    app_type: str = "be"  # "be", "fe", "worker"
    port: int = 8080
    port_name: str = "http"
    image: str
    replicas: int = 1
    namespace: str = "zerg"
    run_as_user: int = 65534
    metrics_path: str = "/metrics"
    enable_service: bool = True
    has_shared_secrets: bool = True
    resources: Resources = Resources {}
    probes: ProbeConfig = ProbeConfig {}
    hpa: HPAConfig = HPAConfig {}
    tilt: TiltConfig = TiltConfig {}
    termination_grace_period: int = 30
