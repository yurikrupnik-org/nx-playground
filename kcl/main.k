import json
import manifests
import .schemas

org: str = option("org") or ""
oidcIdInput: str = option("oidcId") or ""
clusterName: str = option("clusterName") or ""
clusterProvider: str = option("clusterProvider") or "kind"
clusterTypeLabel: str = option("clusterType") or "workload"
environment: str = option("environment") or "dev"
gcpProject: str = option("gcpProject") or ""
gcpSmSecretName: str = option("gcpSmSecretName") or "manager-secrets"
gkeLocation: str = option("gkeLocation") or ""
wifProjectNumber: int = option("wifProjectNumber") or 0
oidcBucket: str = option("oidcBucket") or ""
domain: str = option("domain") or ""
gatewayMode: str = option("gatewayMode") or "platform"
gatewaySuffix: str = option("gatewaySuffix") or ""
ingressClass: str = option("ingressClass") or "istio"
ingressEnabled: bool = option("ingressEnabled") or True
tlsEnabled: bool = option("tlsEnabled") or True
githubAccount: str = option("githubAccount") or ""
githubRepo: str = option("githubRepo") or ""
githubBranch: str = option("githubBranch") or "main"
policyActionInput: str = option("policyAction") or ""
tlsIssuerInput: str = option("tlsIssuer") or ""
acmeEmail: str = option("acmeEmail") or ""
workloads: [schemas.Workload] = option("workloads") or []

_target: str = option("target") or "cluster-config"

_isKind = clusterProvider == "kind"
_isGke = clusterProvider == "gke"
_policyAction = policyActionInput if policyActionInput != "" else ("Audit" if _isKind else "Enforce")
_tlsIssuer = tlsIssuerInput if tlsIssuerInput != "" else ("ca-local" if _isKind else "letsencrypt-prod")
_oidcId = oidcIdInput if oidcIdInput != "" else clusterName
_issuer = "https://storage.googleapis.com/${oidcBucket}/${_oidcId}" if _isKind else ""
_gatewaySuffix = gatewaySuffix if gatewaySuffix != "" else domain

_kindCluster = {
    kind = "Cluster"
    apiVersion = "kind.x-k8s.io/v1alpha4"
    nodes = [{role = "control-plane"}, {role = "worker"}]
    kubeadmConfigPatches = [
        "kind: ClusterConfiguration\napiServer:\n  extraArgs:\n    service-account-issuer: ${_issuer}\n"
    ]
}

_openidConfig = {
    issuer = _issuer
    jwks_uri = "${_issuer}/keys.json"
    response_types_supported = ["id_token"]
    subject_types_supported = ["public"]
    id_token_signing_alg_values_supported = ["RS256"]
}

_clusterConfig = {
    apiVersion = "v1"
    kind = "ConfigMap"
    metadata = {
        name = "cluster-config"
        namespace = "flux-system"
    }
    data = {
        CLUSTER_NAME = clusterName
        CLUSTER_PROVIDER = clusterProvider
        CLUSTER_TYPE = clusterTypeLabel
        ENVIRONMENT = environment
        ORG = org
        OIDC_ID = _oidcId
        GCP_PROJECT = gcpProject
        GCP_SM_SECRET_NAME = gcpSmSecretName
        GKE_LOCATION = gkeLocation
        WIF_PROJECT_NUMBER = "${wifProjectNumber}" if wifProjectNumber > 0 else ""
        OIDC_BUCKET = oidcBucket
        OIDC_ISSUER = _issuer
        DOMAIN = domain
        GATEWAY_MODE = gatewayMode
        GATEWAY_SUFFIX = _gatewaySuffix
        INGRESS_CLASS = ingressClass
        INGRESS_ENABLED = "true" if ingressEnabled else "false"
        TLS_ENABLED = "true" if tlsEnabled else "false"
        GITHUB_ACCOUNT = githubAccount
        GITHUB_REPO = githubRepo
        GITHUB_BRANCH = githubBranch
        POLICY_ACTION = _policyAction
        TLS_ISSUER = _tlsIssuer
        ACME_EMAIL = acmeEmail
    }
}

_gcpWorkloads = [w for w in workloads if w.clouds.gcp]
_awsWorkloads = [w for w in workloads if w.clouds.aws]

_namespaces = [{
    apiVersion = "v1"
    kind = "Namespace"
    metadata = {name = ns}
} for ns in {w.namespace: True for w in workloads}]

_kindKsa = lambda w: schemas.Workload -> {str:} {
    {
        apiVersion = "v1"
        kind = "ServiceAccount"
        metadata = {
            name = w.ksa
            namespace = w.namespace
        }
    }
}

_gkeKsa = lambda w: schemas.Workload -> {str:} {
    {
        apiVersion = "v1"
        kind = "ServiceAccount"
        metadata = {
            name = w.ksa
            namespace = w.namespace
            annotations = {"iam.gke.io/gcp-service-account" = w.clouds.gcp.gsaEmail}
        }
    }
}

_kindCredJson = lambda w: schemas.Workload -> str {
    _suffix = w.clouds.gcp.providerSuffix or w.name
    json.encode({
        "type" = "external_account"
        "audience" = "//iam.googleapis.com/projects/${wifProjectNumber}/locations/global/workloadIdentityPools/local-clusters/providers/kind-${_oidcId}-${_suffix}"
        "subject_token_type" = "urn:ietf:params:oauth:token-type:jwt"
        "token_url" = "https://sts.googleapis.com/v1/token"
        "service_account_impersonation_url" = "https://iamcredentials.googleapis.com/v1/projects/-/serviceAccounts/${w.clouds.gcp.gsaEmail}:generateAccessToken"
        "credential_source" = {
            "file" = "/var/run/secrets/gcp-token/token"
            "format" = {"type" = "text"}
        }
    })
}

_kindGcpSecret = lambda w: schemas.Workload -> {str:} {
    {
        apiVersion = "v1"
        kind = "Secret"
        metadata = {
            name = "gcp-wif-config"
            namespace = w.namespace
        }
        type = "Opaque"
        stringData = {
            "cred-config.json" = _kindCredJson(w)
        }
    }
}

_awsCredJson = lambda w: schemas.Workload -> str {
    """[default]
role_arn = ${w.clouds.aws.roleArn}
region = ${w.clouds.aws.region}
web_identity_token_file = /var/run/secrets/aws-token/token
"""
}

_kindAwsSecret = lambda w: schemas.Workload -> {str:} {
    {
        apiVersion = "v1"
        kind = "Secret"
        metadata = {
            name = "aws-wif-config"
            namespace = w.namespace
        }
        type = "Opaque"
        stringData = {
            "credentials" = _awsCredJson(w)
        }
    }
}

_kindWif = _namespaces \
    + [_kindKsa(w) for w in _gcpWorkloads] \
    + [_kindGcpSecret(w) for w in _gcpWorkloads] \
    + [_kindAwsSecret(w) for w in _awsWorkloads]

_gkeWif = _namespaces + [_gkeKsa(w) for w in _gcpWorkloads]

_outputs = {
    "kind-config" = [_kindCluster]
    "oidc-config" = [_openidConfig]
    "cluster-config" = [_clusterConfig]
    "wif" = _kindWif if _isKind else _gkeWif
}

manifests.yaml_stream(_outputs[_target])
