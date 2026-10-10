import type { CreateTask, Task, UpdateTask } from '@contract/tasks';
import { csrfHeaders } from './auth';
import type { CatalogResponse, RowsPage } from './data-map/types';

const API_BASE_URL = '/api';

// Helper to check if the response is 401 and redirect to login
function checkAuth(response: Response): Response {
  if (response.status === 401) {
    // Redirect to login page if not authenticated
    window.location.href = '/login';
  }
  return response;
}

// Re-export types for convenience
export type {
  CreateTask as CreateTaskInput,
  Task,
  UpdateTask as UpdateTaskInput,
};

export interface Org {
  id: string;
  external_id: string;
  name: string;
  role: string;
  is_personal: boolean;
}

export interface OrgMember {
  workos_user_id: string;
  email: string;
  name: string;
  role: string;
  status: string;
}

export interface OrgInvitation {
  id: string;
  email: string;
  state: string;
  expires_at?: string | null;
  organization_id?: string | null;
}

export const tasksApi = {
  list: async (params?: { mine?: boolean }): Promise<Task[]> => {
    const search = new URLSearchParams();
    // Only ever "narrow to me" - the API has no way to request another user's
    // tasks, and the server derives who "me" is from the session's token.
    if (params?.mine) search.set('mine', 'true');
    const qs = search.size > 0 ? `?${search}` : '';
    const response = await fetch(`${API_BASE_URL}/tasks${qs}`, {
      credentials: 'include', // Include session cookies
    });
    checkAuth(response);
    if (!response.ok) throw new Error('Failed to fetch tasks');
    return response.json();
  },

  getById: async (id: string): Promise<Task> => {
    const response = await fetch(`${API_BASE_URL}/tasks/${id}`, {
      credentials: 'include',
    });
    checkAuth(response);
    if (!response.ok) throw new Error('Failed to fetch task');
    return response.json();
  },

  create: async (input: CreateTask): Promise<Task> => {
    const response = await fetch(`${API_BASE_URL}/tasks`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json', ...csrfHeaders() },
      credentials: 'include',
      body: JSON.stringify(input),
    });
    checkAuth(response);
    if (!response.ok) throw new Error('Failed to create task');
    return response.json();
  },

  update: async (id: string, input: UpdateTask): Promise<Task> => {
    const response = await fetch(`${API_BASE_URL}/tasks/${id}`, {
      method: 'PUT',
      headers: { 'Content-Type': 'application/json', ...csrfHeaders() },
      credentials: 'include',
      body: JSON.stringify(input),
    });
    checkAuth(response);
    if (!response.ok) throw new Error('Failed to update task');
    return response.json();
  },

  delete: async (id: string): Promise<void> => {
    const response = await fetch(`${API_BASE_URL}/tasks/${id}`, {
      method: 'DELETE',
      headers: { ...csrfHeaders() },
      credentials: 'include',
    });
    checkAuth(response);
    if (!response.ok) throw new Error('Failed to delete task');
  },
};

async function orgJson<T>(response: Response, action: string): Promise<T> {
  checkAuth(response);
  if (!response.ok) {
    // Backend guards return terse text bodies (e.g. WorkOS failures as 502).
    const detail = await response.text().catch(() => '');
    throw new Error(detail || `Failed to ${action}`);
  }
  return response.json() as Promise<T>;
}

export const orgApi = {
  get: async (): Promise<Org> => {
    const response = await fetch(`${API_BASE_URL}/org`, {
      credentials: 'include',
    });
    return orgJson(response, 'fetch organization');
  },

  create: async (name: string): Promise<Org> => {
    const response = await fetch(`${API_BASE_URL}/org`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json', ...csrfHeaders() },
      credentials: 'include',
      body: JSON.stringify({ name }),
    });
    return orgJson(response, 'create organization');
  },

  members: async (): Promise<OrgMember[]> => {
    const response = await fetch(`${API_BASE_URL}/org/members`, {
      credentials: 'include',
    });
    return orgJson(response, 'fetch members');
  },

  invitations: async (): Promise<OrgInvitation[]> => {
    const response = await fetch(`${API_BASE_URL}/org/invitations`, {
      credentials: 'include',
    });
    return orgJson(response, 'fetch invitations');
  },

  invite: async (email: string, role?: string): Promise<OrgInvitation> => {
    const response = await fetch(`${API_BASE_URL}/org/invitations`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json', ...csrfHeaders() },
      credentials: 'include',
      body: JSON.stringify({ email, role }),
    });
    return orgJson(response, 'send invitation');
  },

  revokeInvitation: async (id: string): Promise<void> => {
    const response = await fetch(
      `${API_BASE_URL}/org/invitations/${id}/revoke`,
      {
        method: 'POST',
        headers: { ...csrfHeaders() },
        credentials: 'include',
      },
    );
    await orgJson(response, 'revoke invitation');
  },
};

/** Why the catalog could not be read. The data-map page is public (repo
 *  schemas need no backend), so a 401 is surfaced, not redirected to /login. */
export type CatalogErrorReason =
  | 'unauthenticated' // no zerg session
  | 'disabled' // 404: dev-only routes, zerg-api not in APP_ENV=development
  | 'unavailable' // vite proxy could not reach zerg-api
  | 'failed';

export class CatalogError extends Error {
  constructor(
    readonly reason: CatalogErrorReason,
    message: string,
  ) {
    super(message);
  }
}

async function catalogJson<T>(response: Response, action: string): Promise<T> {
  if (response.ok) return response.json() as Promise<T>;
  const detail = await response.text().catch(() => '');
  if (response.status === 401) {
    throw new CatalogError('unauthenticated', 'Sign in to browse databases.');
  }
  if (response.status === 404 && action === 'databases') {
    throw new CatalogError(
      'disabled',
      'Database catalog is disabled: zerg-api is not running with APP_ENV=development.',
    );
  }
  // Vite's proxy answers 5xx with an empty body when zerg-api is down.
  if (response.status >= 500 && !detail) {
    throw new CatalogError(
      'unavailable',
      'zerg-api is not reachable on :8080 — start the stack with `task web`.',
    );
  }
  throw new CatalogError('failed', detail || `Failed to fetch ${action}`);
}

export const catalogApi = {
  databases: async (): Promise<CatalogResponse> => {
    const response = await fetch(`${API_BASE_URL}/catalog/databases`, {
      credentials: 'include',
    });
    return catalogJson(response, 'databases');
  },

  rows: async (
    db: string,
    schema: string,
    table: string,
    page: { limit: number; offset: number },
  ): Promise<RowsPage> => {
    const path = [db, 'tables', schema, table]
      .map(encodeURIComponent)
      .join('/');
    const response = await fetch(
      `${API_BASE_URL}/catalog/databases/${path}/rows?limit=${page.limit}&offset=${page.offset}`,
      { credentials: 'include' },
    );
    return catalogJson(response, `rows of ${schema}.${table}`);
  },
};
