import type { CreateTodo, Todo, TodoPriority, UpdateTodo } from '@domain/todo';

const API_BASE_URL = '/api/todos';

const JSON_HEADERS = { 'Content-Type': 'application/json' } as const;

// Re-export the ts-rs DTOs for convenience so callers can import them from here.
export type { Todo, CreateTodo, UpdateTodo, TodoPriority };

export const todoApi = {
  list: async (): Promise<Todo[]> => {
    const response = await fetch(`${API_BASE_URL}?limit=100000`);
    if (!response.ok) throw new Error('Failed to fetch todos');
    return response.json();
  },

  create: async (input: CreateTodo): Promise<Todo> => {
    const response = await fetch(API_BASE_URL, {
      method: 'POST',
      headers: JSON_HEADERS,
      body: JSON.stringify(input),
    });
    if (!response.ok) throw new Error('Failed to create todo');
    return response.json();
  },

  update: async (id: string, input: UpdateTodo): Promise<Todo> => {
    const response = await fetch(`${API_BASE_URL}/${id}`, {
      method: 'PUT',
      headers: JSON_HEADERS,
      body: JSON.stringify(input),
    });
    if (!response.ok) throw new Error('Failed to update todo');
    return response.json();
  },

  complete: async (id: string): Promise<Todo> => {
    const response = await fetch(`${API_BASE_URL}/${id}/complete`, {
      method: 'POST',
    });
    if (!response.ok) throw new Error('Failed to complete todo');
    return response.json();
  },

  uncomplete: async (id: string): Promise<Todo> => {
    const response = await fetch(`${API_BASE_URL}/${id}/uncomplete`, {
      method: 'POST',
    });
    if (!response.ok) throw new Error('Failed to uncomplete todo');
    return response.json();
  },

  remove: async (id: string): Promise<void> => {
    const response = await fetch(`${API_BASE_URL}/${id}`, {
      method: 'DELETE',
    });
    if (!response.ok) throw new Error('Failed to delete todo');
  },
};
