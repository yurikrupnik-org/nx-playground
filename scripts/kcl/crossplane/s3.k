"""
AWS S3 bucket resources via Crossplane upjet provider.

Uses typed schemas extracted from crossplane-provider-upjet-aws v1.23.0
into providers/aws_s3/ to avoid v1beta2 module name collisions.
"""

import .providers.aws_s3.bucket as s3

import .config

_s3_bucket = lambda c: config.S3Config -> s3.Bucket {
    s3.Bucket {
        metadata.name = "${c.environment}-${c.name}"
        metadata.labels = {
            environment = c.environment
            "managed-by" = "crossplane"
        } | c.labels
        spec = {
            forProvider = {
                region = c.region
                forceDestroy = c.force_destroy
                objectLockEnabled = c.object_lock
                tags = {
                    environment = c.environment
                    "managed-by" = "crossplane"
                } | c.labels
            }
            providerConfigRef.name = c.provider_config
            deletionPolicy = "Delete" if c.force_destroy else "Orphan"
        }
    }
}

buckets = [_s3_bucket(c) for c in config.s3_configs]
