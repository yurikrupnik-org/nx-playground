"""
Crossplane cloud storage resources for GCP and AWS.

Generates Crossplane managed resources for cloud storage buckets
using upjet providers.

Usage:
    # Generate all storage resources
    kcl run main.k

    # Generate only GCP
    kcl run main.k -e gcs_buckets

    # Generate only AWS
    kcl run main.k -e s3_buckets
"""

import .gcs
import .s3

# GCP Cloud Storage buckets
gcs_buckets = gcs.buckets

# AWS S3 buckets
s3_buckets = s3.buckets
