"""
GCP Cloud Storage bucket resources via Crossplane upjet provider.

Uses typed schemas extracted from crossplane-provider-upjet-gcp v1.0.5
into providers/gcp_storage/ to avoid v1beta1 module name collisions.
"""

import .providers.gcp_storage.bucket as gcs

import .config

_gcs_bucket = lambda c: config.GCSConfig -> gcs.Bucket {
    gcs.Bucket {
        metadata.name = "${c.environment}-${c.name}"
        metadata.labels = {
            environment = c.environment
            "managed-by" = "crossplane"
        } | c.labels
        spec = {
            forProvider = {
                project = c.project
                location = c.location
                storageClass = c.storage_class
                forceDestroy = c.force_destroy
                labels = {
                    environment = c.environment
                    "managed-by" = "crossplane"
                } | c.labels
                uniformBucketLevelAccess = c.uniform_access
                versioning = [
                    { enabled = c.versioning }
                ]
                publicAccessPrevention = "enforced"
            }
            providerConfigRef.name = c.provider_config
            deletionPolicy = "Delete" if c.force_destroy else "Orphan"
        }
    }
}

buckets = [_gcs_bucket(c) for c in config.gcs_configs]
