// Tenant-scoped cloud asset inventory (the Phase 3 sample resource), via the BFF.

import { csrfHeaders } from './csrf';

const API_BASE_URL = '/api';

export interface CloudAsset {
  id: string;
  org_id: string;
  provider: string;
  external_id: string;
  name: string;
  asset_type: string;
  region: string;
  status: string;
  monthly_cost: number;
  created_at: string;
  updated_at: string;
}

export interface NewAsset {
  provider: string;
  external_id: string;
  name: string;
  asset_type: string;
  region: string;
  status?: string;
  monthly_cost?: number;
}

export async function listAssets(): Promise<CloudAsset[]> {
  const res = await fetch(`${API_BASE_URL}/assets`, { credentials: 'include' });
  if (!res.ok) {
    throw new Error(`failed to list assets (${res.status})`);
  }
  return res.json();
}

// Assets discovered by a specific user, within the caller's organization. The org is
// always the authenticated caller's tenant (enforced server-side); `userId` only
// narrows within it — it can never reach another tenant's data.
export async function listAssetsByUser(userId: string): Promise<CloudAsset[]> {
  const res = await fetch(
    `${API_BASE_URL}/assets/by-user/${encodeURIComponent(userId)}`,
    {
      credentials: 'include',
    },
  );
  if (!res.ok) {
    throw new Error(`failed to list assets for user (${res.status})`);
  }
  return res.json();
}

export async function createAsset(input: NewAsset): Promise<CloudAsset> {
  const res = await fetch(`${API_BASE_URL}/assets`, {
    method: 'POST',
    credentials: 'include',
    headers: { 'content-type': 'application/json', ...csrfHeaders() },
    body: JSON.stringify(input),
  });
  if (!res.ok) {
    throw new Error(`failed to create asset (${res.status})`);
  }
  return res.json();
}
