"""
Shared configuration for Crossplane storage resources.
"""

schema StorageBucketConfig:
    """Common configuration for a storage bucket across providers."""
    name: str
    environment: str = "dev"
    labels: {str: str} = {}
    force_destroy: bool = False

schema GCSConfig(StorageBucketConfig):
    """GCP-specific bucket configuration."""
    project: str
    location: str = "US"
    storage_class: str = "STANDARD"
    uniform_access: bool = True
    versioning: bool = False
    provider_config: str = "default"

schema S3Config(StorageBucketConfig):
    """AWS-specific bucket configuration."""
    region: str = "us-east-1"
    object_lock: bool = False
    provider_config: str = "default"

# =============================================================================
# DEFINE YOUR BUCKETS HERE
# =============================================================================

GCP_PROJECT = "playground-447016"

gcs_configs: [GCSConfig] = [
    GCSConfig {
        name = "matia-data-lake"
        project = GCP_PROJECT
        location = "US"
        storage_class = "STANDARD"
        environment = "dev"
        versioning = True
        labels = {
            team = "data"
            app = "matia"
        }
    }
    GCSConfig {
        name = "matia-etl-staging"
        project = GCP_PROJECT
        location = "US"
        storage_class = "STANDARD"
        environment = "dev"
        force_destroy = True
        labels = {
            team = "data"
            app = "matia"
            purpose = "etl-staging"
        }
    }
]

s3_configs: [S3Config] = [
    S3Config {
        name = "matia-data-lake"
        region = "us-east-1"
        environment = "dev"
        labels = {
            team = "data"
            app = "matia"
        }
    }
    S3Config {
        name = "matia-etl-staging"
        region = "us-east-1"
        environment = "dev"
        force_destroy = True
        labels = {
            team = "data"
            app = "matia"
            purpose = "etl-staging"
        }
    }
]
