# /// script
# requires-python = ">=3.10"
# dependencies = ["kubernetes"]
# ///
"""Python SDK for DevEnvironment claims (platform.playground.io/v1alpha1).

Provisions and inspects Crossplane DevEnvironment claims which spin up
postgres + redis + nats in a dedicated namespace.
"""

import base64

from kubernetes import client, config

GROUP = "platform.playground.io"
VERSION = "v1alpha1"
PLURAL = "devenvironments"
KIND = "DevEnvironment"
FIELD_MANAGER = "devenv-sdk"

_loaded = False


def _api():
    global _loaded
    if not _loaded:
        config.load_kube_config()
        _loaded = True
    return client.CustomObjectsApi(), client.CoreV1Api()


def _claim_body(name, postgres, redis, nats):
    return {
        "apiVersion": f"{GROUP}/{VERSION}",
        "kind": KIND,
        "metadata": {"name": name},
        "spec": {
            "parameters": {
                "postgres": {"enabled": bool(postgres)},
                "redis": {"enabled": bool(redis)},
                "nats": {"enabled": bool(nats)},
            }
        },
    }


def create(name, namespace="default", postgres=True, redis=True, nats=True):
    """Server-side apply the DevEnvironment claim. Idempotent."""
    custom, _ = _api()
    return custom.patch_namespaced_custom_object(
        GROUP,
        VERSION,
        namespace,
        PLURAL,
        name,
        _claim_body(name, postgres, redis, nats),
        field_manager=FIELD_MANAGER,
        force=True,
        _content_type="application/apply-patch+yaml",
    )


def get(name, namespace="default"):
    """Fetch the claim: {ready, environment, conditions}.

    Tolerates a claim whose status has not been populated yet.
    """
    custom, _ = _api()
    obj = custom.get_namespaced_custom_object(GROUP, VERSION, namespace, PLURAL, name)
    status = obj.get("status") or {}
    conditions = status.get("conditions") or []
    ready = any(
        c.get("type") == "Ready" and c.get("status") == "True" for c in conditions
    )
    return {
        "ready": ready,
        "environment": status.get("environment"),
        "conditions": conditions,
    }


def connection(name, namespace="default"):
    """Resolve connection details for a ready environment.

    Reads the CNPG-format postgres secret from the environment namespace and
    returns {postgres: {uri, username, password, host, port, dbname},
    redis_host, nats_url}.
    """
    custom, core = _api()
    env = get(name, namespace)["environment"]
    if not env:
        raise RuntimeError(
            f"DevEnvironment {namespace}/{name} has no status.environment yet"
        )
    secret_name = env.get("postgresSecret")
    env_namespace = env.get("namespace")
    postgres = None
    if secret_name and env_namespace:
        secret = core.read_namespaced_secret(secret_name, env_namespace)
        data = {
            k: base64.b64decode(v).decode("utf-8")
            for k, v in (secret.data or {}).items()
        }
        postgres = {
            "uri": data.get("uri"),
            "username": data.get("username"),
            "password": data.get("password"),
            "host": data.get("host"),
            "port": data.get("port"),
            "dbname": data.get("dbname"),
        }
    return {
        "postgres": postgres,
        "redis_host": env.get("redisHost"),
        "nats_url": env.get("natsUrl"),
    }


def delete(name, namespace="default"):
    """Delete the DevEnvironment claim."""
    custom, _ = _api()
    return custom.delete_namespaced_custom_object(
        GROUP, VERSION, namespace, PLURAL, name
    )
