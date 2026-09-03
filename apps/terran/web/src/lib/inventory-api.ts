// Read-only cloud inventory observed from the cluster by the Crossplane
// `CloudInventory` composition, via the BFF. There is no write path.

const API_BASE_URL = '/api';

export interface ObservedResource {
  id: string;
  claim: string;
  name: string;
  api_version: string;
  kind: string;
  namespace: string | null;
  resource_type: string;
  status: string;
  region: string;
  configuration: unknown;
  tags: { key: string; value: string }[];
}

function inventoryError(res: Response): Error {
  if (res.status === 503) {
    return new Error('inventory unavailable — is the cluster reachable?');
  }
  if (res.status === 502) {
    return new Error('cluster query failed');
  }
  return new Error(`failed to list cloud resources (${res.status})`);
}

export async function listCloudResources(): Promise<ObservedResource[]> {
  const res = await fetch(`${API_BASE_URL}/cloud-resources`, {
    credentials: 'include',
  });
  if (!res.ok) {
    throw inventoryError(res);
  }
  return res.json();
}

export async function getCloudResource(id: string): Promise<ObservedResource> {
  const res = await fetch(
    `${API_BASE_URL}/cloud-resources/${encodeURIComponent(id)}`,
    { credentials: 'include' },
  );
  if (!res.ok) {
    throw inventoryError(res);
  }
  return res.json();
}
