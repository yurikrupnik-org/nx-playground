import type { CreateTodo, Todo, TodoPriority, UpdateTodo } from '@domain/todo';

import { identityHeaders } from './identity';

const API_BASE_URL = '/api/todos';

// Re-export the ts-rs DTOs for convenience so callers can import them from here.
export type { CreateTodo, Todo, TodoPriority, UpdateTodo };

/** Identity headers plus a JSON body content type, per request. */
const jsonHeaders = (): Record<string, string> => ({
  ...identityHeaders(),
  'Content-Type': 'application/json',
});

/**
 * Turn a failed response into an error the UI can show verbatim.
 *
 * todo-api enforces the flag catalogue server-side, so the two flag statuses
 * get their own messages instead of a generic "failed to …".
 */
function requestError(response: Response, fallback: string): Error {
  if (response.status === 403) {
    return new Error('writes are disabled by feature flag (todo_write)');
  }
  if (response.status === 429) {
    return new Error('todo limit reached (todo_max_items)');
  }
  return new Error(fallback);
}

export const todoApi = {
  /**
   * `signal` lets a caller abort an in-flight (or retrying) load — the route
   * implementations pass the one their runtime/actor cancels on unmount, so a
   * navigation does not leave a request running.
   */
  list: async (signal?: AbortSignal): Promise<Todo[]> => {
    const response = await fetch(`${API_BASE_URL}?limit=100000`, {
      headers: identityHeaders(),
      signal,
    });
    if (!response.ok) throw requestError(response, 'Failed to fetch todos');
    return response.json();
  },

  create: async (input: CreateTodo): Promise<Todo> => {
    const response = await fetch(API_BASE_URL, {
      method: 'POST',
      headers: jsonHeaders(),
      body: JSON.stringify(input),
    });
    if (!response.ok) throw requestError(response, 'Failed to create todo');
    return response.json();
  },

  update: async (id: string, input: UpdateTodo): Promise<Todo> => {
    const response = await fetch(`${API_BASE_URL}/${id}`, {
      method: 'PUT',
      headers: jsonHeaders(),
      body: JSON.stringify(input),
    });
    if (!response.ok) throw requestError(response, 'Failed to update todo');
    return response.json();
  },

  complete: async (id: string): Promise<Todo> => {
    const response = await fetch(`${API_BASE_URL}/${id}/complete`, {
      method: 'POST',
      headers: identityHeaders(),
    });
    if (!response.ok) throw requestError(response, 'Failed to complete todo');
    return response.json();
  },

  uncomplete: async (id: string): Promise<Todo> => {
    const response = await fetch(`${API_BASE_URL}/${id}/uncomplete`, {
      method: 'POST',
      headers: identityHeaders(),
    });
    if (!response.ok) throw requestError(response, 'Failed to uncomplete todo');
    return response.json();
  },

  remove: async (id: string): Promise<void> => {
    const response = await fetch(`${API_BASE_URL}/${id}`, {
      method: 'DELETE',
      headers: identityHeaders(),
    });
    if (!response.ok) throw requestError(response, 'Failed to delete todo');
  },
};
