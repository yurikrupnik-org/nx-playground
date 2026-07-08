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

export const tasksApi = {
  list: async (): Promise<Task[]> => {
    const response = await fetch(`${API_BASE_URL}/tasks`, {
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
