/**
 * Catalog of OCI image/package registries: the managed cloud offerings,
 * hosted SaaS registries, and self-hosted options. Static reference data —
 * rendered by pages/registries.tsx.
 */

export type RegistryKind = 'cloud' | 'saas' | 'self-hosted';

export interface OciRegistry {
  id: string;
  name: string;
  vendor: string;
  /** Registry host pattern, `<placeholders>` for account-specific parts. */
  host: string;
  kinds: RegistryKind[];
  /** How you authenticate the docker/oras CLI. */
  login: string;
  /** Example image reference. */
  example: string;
  /** Supports arbitrary OCI artifacts (Helm charts, SBOMs, signatures, WASM). */
  ociArtifacts: boolean;
  docsUrl: string;
  notes?: string;
}

export const KIND_LABELS: Record<RegistryKind, string> = {
  cloud: 'Cloud provider',
  saas: 'Hosted SaaS',
  'self-hosted': 'Self-hosted',
};

export const OCI_REGISTRIES: OciRegistry[] = [
  {
    id: 'docker-hub',
    name: 'Docker Hub',
    vendor: 'Docker',
    host: 'docker.io',
    kinds: ['saas'],
    login: 'docker login',
    example: 'docker.io/library/nginx:1.27',
    ociArtifacts: true,
    docsUrl: 'https://docs.docker.com/docker-hub/',
    notes:
      'The default registry for the docker CLI. Anonymous pulls are rate-limited; official and verified-publisher images.',
  },
  {
    id: 'ghcr',
    name: 'GitHub Container Registry',
    vendor: 'GitHub (Packages)',
    host: 'ghcr.io',
    kinds: ['saas'],
    login: 'echo $GH_PAT | docker login ghcr.io -u <username> --password-stdin',
    example: 'ghcr.io/<owner>/<image>:<tag>',
    ociArtifacts: true,
    docsUrl:
      'https://docs.github.com/en/packages/working-with-a-github-packages-registry/working-with-the-container-registry',
    notes:
      'Part of GitHub Packages. Permissions ride on repos/orgs; GITHUB_TOKEN works in Actions.',
  },
  {
    id: 'aws-ecr',
    name: 'Elastic Container Registry (ECR)',
    vendor: 'AWS',
    host: '<account-id>.dkr.ecr.<region>.amazonaws.com',
    kinds: ['cloud'],
    login:
      'aws ecr get-login-password --region <region> | docker login --username AWS --password-stdin <account-id>.dkr.ecr.<region>.amazonaws.com',
    example: '<account-id>.dkr.ecr.us-east-1.amazonaws.com/app:latest',
    ociArtifacts: true,
    docsUrl: 'https://docs.aws.amazon.com/AmazonECR/latest/userguide/',
    notes:
      'IAM-scoped private repos with image scanning, lifecycle policies, and cross-region/cross-account replication.',
  },
  {
    id: 'aws-ecr-public',
    name: 'ECR Public Gallery',
    vendor: 'AWS',
    host: 'public.ecr.aws',
    kinds: ['cloud'],
    login:
      'aws ecr-public get-login-password --region us-east-1 | docker login --username AWS --password-stdin public.ecr.aws',
    example: 'public.ecr.aws/<alias>/<image>:<tag>',
    ociArtifacts: true,
    docsUrl: 'https://docs.aws.amazon.com/AmazonECR/latest/public/',
    notes: 'Public counterpart of ECR; anonymous pulls with generous quotas.',
  },
  {
    id: 'gcp-artifact-registry',
    name: 'Artifact Registry',
    vendor: 'Google Cloud',
    host: '<location>-docker.pkg.dev',
    kinds: ['cloud'],
    login: 'gcloud auth configure-docker <location>-docker.pkg.dev',
    example: 'us-central1-docker.pkg.dev/<project>/<repo>/<image>:<tag>',
    ociArtifacts: true,
    docsUrl: 'https://cloud.google.com/artifact-registry/docs',
    notes:
      'Successor to Container Registry (gcr.io, shut down 2025). Also hosts npm, Maven, Python, and OS packages.',
  },
  {
    id: 'azure-acr',
    name: 'Azure Container Registry (ACR)',
    vendor: 'Microsoft Azure',
    host: '<name>.azurecr.io',
    kinds: ['cloud'],
    login: 'az acr login --name <name>',
    example: '<name>.azurecr.io/<image>:<tag>',
    ociArtifacts: true,
    docsUrl: 'https://learn.microsoft.com/en-us/azure/container-registry/',
    notes:
      'Entra ID auth, geo-replication, and ACR Tasks for in-registry builds.',
  },
  {
    id: 'gitlab',
    name: 'GitLab Container Registry',
    vendor: 'GitLab',
    host: 'registry.gitlab.com',
    kinds: ['saas', 'self-hosted'],
    login: 'docker login registry.gitlab.com',
    example: 'registry.gitlab.com/<group>/<project>/<image>:<tag>',
    ociArtifacts: true,
    docsUrl: 'https://docs.gitlab.com/ee/user/packages/container_registry/',
    notes:
      'Bundled with every project on gitlab.com and self-managed instances; CI_JOB_TOKEN works in pipelines.',
  },
  {
    id: 'quay',
    name: 'Quay.io / Red Hat Quay',
    vendor: 'Red Hat',
    host: 'quay.io',
    kinds: ['saas', 'self-hosted'],
    login: 'docker login quay.io',
    example: 'quay.io/<org>/<image>:<tag>',
    ociArtifacts: true,
    docsUrl: 'https://docs.redhat.com/en/documentation/red_hat_quay/',
    notes:
      'Clair vulnerability scanning built in; Red Hat Quay is the self-hosted distribution.',
  },
  {
    id: 'harbor',
    name: 'Harbor',
    vendor: 'CNCF (graduated)',
    host: '<your-harbor-host>',
    kinds: ['self-hosted'],
    login: 'docker login <your-harbor-host>',
    example: 'harbor.example.com/<project>/<image>:<tag>',
    ociArtifacts: true,
    docsUrl: 'https://goharbor.io/docs/',
    notes:
      'Full-featured self-hosted registry: RBAC, replication to/from other registries, scanning, signing, quotas.',
  },
  {
    id: 'artifactory',
    name: 'Artifactory',
    vendor: 'JFrog',
    host: '<name>.jfrog.io',
    kinds: ['saas', 'self-hosted'],
    login: 'docker login <name>.jfrog.io',
    example: '<name>.jfrog.io/<repo>/<image>:<tag>',
    ociArtifacts: true,
    docsUrl: 'https://jfrog.com/help/r/jfrog-artifactory-documentation',
    notes:
      'Universal artifact manager; Docker/OCI repos alongside 30+ other package formats, with remote proxying.',
  },
  {
    id: 'do-registry',
    name: 'DigitalOcean Container Registry',
    vendor: 'DigitalOcean',
    host: 'registry.digitalocean.com',
    kinds: ['cloud'],
    login: 'doctl registry login',
    example: 'registry.digitalocean.com/<registry>/<image>:<tag>',
    ociArtifacts: false,
    docsUrl: 'https://docs.digitalocean.com/products/container-registry/',
    notes: 'Private-only registry integrated with DOKS clusters.',
  },
  {
    id: 'cloudsmith',
    name: 'Cloudsmith',
    vendor: 'Cloudsmith',
    host: 'docker.cloudsmith.io',
    kinds: ['saas'],
    login: 'docker login docker.cloudsmith.io',
    example: 'docker.cloudsmith.io/<org>/<repo>/<image>:<tag>',
    ociArtifacts: true,
    docsUrl: 'https://help.cloudsmith.io/docs/docker-registry',
    notes: 'Multi-format package SaaS with Docker/OCI support.',
  },
  {
    id: 'distribution',
    name: 'CNCF Distribution (registry)',
    vendor: 'CNCF',
    host: 'localhost:5000',
    kinds: ['self-hosted'],
    login: 'docker run -d -p 5000:5000 registry:3',
    example: 'localhost:5000/<image>:<tag>',
    ociArtifacts: true,
    docsUrl: 'https://distribution.github.io/distribution/',
    notes:
      'The reference implementation of the OCI Distribution spec; the `registry` image. No UI or auth out of the box.',
  },
  {
    id: 'zot',
    name: 'zot',
    vendor: 'CNCF (sandbox)',
    host: '<your-zot-host>',
    kinds: ['self-hosted'],
    login: 'docker login <your-zot-host>',
    example: 'zot.example.com/<image>:<tag>',
    ociArtifacts: true,
    docsUrl: 'https://zotregistry.dev/',
    notes:
      'OCI-native, vendor-neutral registry focused purely on the OCI image and distribution specs.',
  },
];
