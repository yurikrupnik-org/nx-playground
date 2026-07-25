import type { CreateTask, Task, UpdateTask } from '@domain/tasks';
import { csrfHeaders } from './csrf';

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
  Task,
  CreateTask as CreateTaskInput,
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
  list: async (params?: { user_id?: string }): Promise<Task[]> => {
    const search = new URLSearchParams();
    if (params?.user_id) search.set('user_id', params.user_id);
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
