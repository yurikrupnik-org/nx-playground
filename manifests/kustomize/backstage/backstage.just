# Backstage deploy/setup — imported by the root justfile.

[group('backstage')]
backstage-dev:
    kubectl apply -k manifests/kustomize/backstage/overlays/dev

[group('backstage')]
backstage-prod:
    kubectl apply -k manifests/kustomize/backstage/overlays/prod

[group('backstage')]
backstage-logs:
    kubectl logs -n backstage deployment/backstage -f

[group('backstage')]
backstage-catalog-generate:
    nu scripts/nu/generate-backstage-catalog.nu

[group('backstage')]
backstage-setup-github:
    nu scripts/nu/backstage-setup-providers.nu github

[group('backstage')]
backstage-setup-aws:
    nu scripts/nu/backstage-setup-providers.nu aws

[group('backstage')]
backstage-setup-gcp:
    nu scripts/nu/backstage-setup-providers.nu gcp

[group('backstage')]
backstage-setup-cloudflare:
    nu scripts/nu/backstage-setup-providers.nu cloudflare

[group('backstage')]
backstage-setup-all:
    nu scripts/nu/backstage-setup-providers.nu all

[group('backstage')]
backstage-restart:
    kubectl rollout restart deployment/backstage -n backstage
    kubectl rollout status deployment/backstage -n backstage
