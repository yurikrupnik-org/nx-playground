import .schema

_make_labels = lambda cfg: schema.AppConfig -> {str:str} {
    {
        app = "${cfg.namespace}-${cfg.name}"
        "app.kubernetes.io/name" = "${cfg.namespace}-${cfg.name}"
        "app.kubernetes.io/component" = cfg.component
        "app.kubernetes.io/part-of" = cfg.namespace
    }
}

_make_match_labels = lambda cfg: schema.AppConfig -> {str:str} {
    {app = "${cfg.namespace}-${cfg.name}"}
}

make_service_account = lambda cfg: schema.AppConfig -> {str:} {
    _full_name = "${cfg.namespace}-${cfg.name}"
    {
        apiVersion = "v1"
        kind = "ServiceAccount"
        metadata = {
            name = _full_name
            namespace = cfg.namespace
            labels = _make_labels(cfg)
        }
    }
}

_make_probe = lambda cfg: schema.AppConfig, probe_type: str -> {str:} {
    _p = cfg.probes
    _is_startup = probe_type == "startup"
    _is_liveness = probe_type == "liveness"
    _is_readiness = probe_type == "readiness"
    _path = _p.ready_path if _is_readiness and _p.ready_path else _p.health_path

    _probe_action = {
        if _p.protocol == "grpc":
            grpc = {port = cfg.port}
        else:
            httpGet = {
                path = _path
                port = _p.port_name
            }
    }

    _probe_action | {
        if _is_startup:
            initialDelaySeconds = _p.startup_initial_delay
            periodSeconds = _p.startup_period
            failureThreshold = _p.startup_failure
            timeoutSeconds = _p.startup_timeout
        if _is_liveness:
            periodSeconds = _p.liveness_period
            failureThreshold = _p.liveness_failure
            timeoutSeconds = _p.liveness_timeout
        if _is_readiness:
            periodSeconds = _p.readiness_period
            failureThreshold = _p.readiness_failure
            timeoutSeconds = _p.readiness_timeout
    }
}

_make_env_from = lambda cfg: schema.AppConfig -> [{str:}] {
    _ns = cfg.namespace
    _items = [
        {configMapRef = {name = "${_ns}-shared-config"}}
        {configMapRef = {name = "${_ns}-${cfg.name}-config"}}
    ]
    _with_shared_secrets = _items + [{secretRef = {name = "${_ns}-shared-secrets", optional = True}}] if cfg.has_shared_secrets else _items
    _with_app_secrets = _with_shared_secrets + [{secretRef = {name = "${_ns}-${cfg.name}-secrets", optional = True}}]
    _with_app_secrets
}

make_deployment = lambda cfg: schema.AppConfig -> {str:} {
    _full_name = "${cfg.namespace}-${cfg.name}"
    _labels = _make_labels(cfg)
    _resources_spec = {
        requests = {
            memory = cfg.resources.mem_request
            cpu = cfg.resources.cpu_request
        }
        limits = {
            memory = cfg.resources.mem_limit
            if cfg.resources.cpu_limit:
                cpu = cfg.resources.cpu_limit
        }
    }
    {
        apiVersion = "apps/v1"
        kind = "Deployment"
        metadata = {
            name = _full_name
            namespace = cfg.namespace
            labels = _labels
        }
        spec = {
            replicas = cfg.replicas
            selector.matchLabels = _make_match_labels(cfg)
            template = {
                metadata = {
                    labels = _labels
                    annotations = {
                        "prometheus.io/scrape" = "true"
                        "prometheus.io/port" = str(cfg.port)
                        "prometheus.io/path" = cfg.metrics_path
                    }
                }
                spec = {
                    securityContext = {
                        runAsNonRoot = True
                        runAsUser = cfg.run_as_user
                        fsGroup = cfg.run_as_user
                        seccompProfile = {type = "RuntimeDefault"}
                    }
                    containers = [{
                        name = _full_name
                        image = "${cfg.image}:latest"
                        imagePullPolicy = "IfNotPresent"
                        ports = [{
                            name = cfg.port_name
                            containerPort = cfg.port
                            protocol = "TCP"
                        }]
                        envFrom = _make_env_from(cfg)
                        startupProbe = _make_probe(cfg, "startup")
                        livenessProbe = _make_probe(cfg, "liveness")
                        readinessProbe = _make_probe(cfg, "readiness")
                        resources = _resources_spec
                        securityContext = {
                            allowPrivilegeEscalation = False
                            readOnlyRootFilesystem = True
                            runAsNonRoot = True
                            runAsUser = cfg.run_as_user
                            capabilities.drop = ["ALL"]
                        }
                        volumeMounts = [{name = "tmp", mountPath = "/tmp"}]
                    }]
                    volumes = [{name = "tmp", emptyDir = {}}]
                    terminationGracePeriodSeconds = cfg.termination_grace_period
                    restartPolicy = "Always"
                }
            }
        }
    }
}

make_service = lambda cfg: schema.AppConfig -> {str:} {
    _full_name = "${cfg.namespace}-${cfg.name}"
    {
        apiVersion = "v1"
        kind = "Service"
        metadata = {
            name = _full_name
            namespace = cfg.namespace
            labels = _make_labels(cfg)
        }
        spec = {
            selector = _make_match_labels(cfg)
            ports = [{
                name = cfg.port_name
                port = cfg.port
                targetPort = cfg.port_name
                protocol = "TCP"
            }]
        }
    }
}

make_hpa = lambda cfg: schema.AppConfig -> {str:} {
    _full_name = "${cfg.namespace}-${cfg.name}"
    {
        apiVersion = "autoscaling/v2"
        kind = "HorizontalPodAutoscaler"
        metadata = {
            name = _full_name
            namespace = cfg.namespace
            labels = _make_labels(cfg)
        }
        spec = {
            scaleTargetRef = {
                apiVersion = "apps/v1"
                kind = "Deployment"
                name = _full_name
            }
            minReplicas = cfg.hpa.min_replicas
            maxReplicas = cfg.hpa.max_replicas
            metrics = [
                {
                    type = "Resource"
                    resource = {
                        name = "cpu"
                        target = {
                            type = "Utilization"
                            averageUtilization = cfg.hpa.cpu_target
                        }
                    }
                }
                {
                    type = "Resource"
                    resource = {
                        name = "memory"
                        target = {
                            type = "Utilization"
                            averageUtilization = cfg.hpa.memory_target
                        }
                    }
                }
            ]
            behavior = {
                scaleDown = {
                    stabilizationWindowSeconds = 300
                    policies = [{type = "Percent", value = 10, periodSeconds = 60}]
                }
                scaleUp = {
                    stabilizationWindowSeconds = 0
                    policies = [
                        {type = "Percent", value = 100, periodSeconds = 15}
                        {type = "Pods", value = 4, periodSeconds = 15}
                    ]
                    selectPolicy = "Max"
                }
            }
        }
    }
}
