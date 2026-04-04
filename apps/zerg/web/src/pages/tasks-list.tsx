import { useMutation, useQuery, useQueryClient } from '@tanstack/solid-query';
import { Link } from '@tanstack/solid-router';
import { createSignal, For, Show } from 'solid-js';
import { tasksApi } from '../lib/api-client';

export function TasksListPage() {
  const queryClient = useQueryClient();
  const [filter, setFilter] = createSignal<
    'all' | 'todo' | 'in_progress' | 'done'
  >('all');

  const tasksQuery = useQuery(() => ({
    queryKey: ['tasks'] as const,
    queryFn: tasksApi.list,
  }));

  const deleteMutation = useMutation(() => ({
    mutationFn: tasksApi.delete,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['tasks'] });
    },
  }));

  const filteredTasks = () => {
    const tasks = tasksQuery.data || [];
    const currentFilter = filter();
    if (currentFilter === 'all') return tasks;
    return tasks.filter((task) => task.status === currentFilter);
  };

  const getPriorityClass = (priority: string) => {
    switch (priority) {
      case 'urgent':
        return 'bg-red-100 text-red-800';
      case 'high':
        return 'bg-orange-100 text-orange-800';
      case 'medium':
        return 'bg-yellow-100 text-yellow-800';
      case 'low':
        return 'bg-green-100 text-green-800';
      default:
        return 'bg-gray-100 text-gray-800';
    }
  };

  const getStatusClass = (status: string) => {
    switch (status) {
      case 'done':
        return 'bg-green-100 text-green-800';
      case 'in_progress':
        return 'bg-blue-100 text-blue-800';
      case 'todo':
        return 'bg-gray-100 text-gray-800';
      default:
        return 'bg-gray-100 text-gray-800';
    }
  };

  return (
    <div class="container mx-auto p-4 max-w-6xl">
      <div class="flex justify-between items-center mb-6">
        <h1 class="text-3xl font-bold">Tasks</h1>
      </div>

      {/* Filters */}
      <div class="flex gap-2 mb-4">
        <button
          type="button"
          class={`px-4 py-2 rounded ${filter() === 'all' ? 'bg-blue-500 text-white' : 'bg-gray-200'}`}
          onClick={() => setFilter('all')}
        >
          All
        </button>
        <button
          type="button"
          class={`px-4 py-2 rounded ${filter() === 'todo' ? 'bg-blue-500 text-white' : 'bg-gray-200'}`}
          onClick={() => setFilter('todo')}
        >
          To Do
        </button>
        <button
          type="button"
          class={`px-4 py-2 rounded ${filter() === 'in_progress' ? 'bg-blue-500 text-white' : 'bg-gray-200'}`}
          onClick={() => setFilter('in_progress')}
        >
          In Progress
        </button>
        <button
          type="button"
          class={`px-4 py-2 rounded ${filter() === 'done' ? 'bg-blue-500 text-white' : 'bg-gray-200'}`}
          onClick={() => setFilter('done')}
        >
          Done
        </button>
      </div>

      {/* Task List */}
      <Show
        when={!tasksQuery.isLoading}
        fallback={<div class="text-center py-8">Loading tasks...</div>}
      >
        <Show
          when={filteredTasks().length > 0}
          fallback={
            <div class="text-center py-8 text-gray-500">No tasks found</div>
          }
        >
          <div class="space-y-4">
            <For each={filteredTasks()}>
              {(task) => (
                <div class="border rounded-lg p-4 hover:shadow-md transition-shadow bg-white">
                  <div class="flex justify-between items-start">
                    <div class="flex-1">
                      <h3 class="text-xl font-semibold">
                        <Link
                          to={`/tasks/${task.id}`}
                          class="hover:text-blue-500"
                        >
                          {task.title}
                        </Link>
                      </h3>
                      <p class="text-gray-600 mt-1">{task.description}</p>
                      <div class="flex gap-2 mt-2">
                        <span
                          class={`px-2 py-1 text-xs rounded ${getPriorityClass(task.priority)}`}
                        >
                          {task.priority}
                        </span>
                        <span
                          class={`px-2 py-1 text-xs rounded ${getStatusClass(task.status)}`}
                        >
                          {task.status.replace('_', ' ')}
                        </span>
                        {task.status === 'done' && (
                          <span class="px-2 py-1 text-xs rounded bg-green-100 text-green-800">
                            ✓ Completed
                          </span>
                        )}
                      </div>
                      {task.due_date && (
                        <div class="text-sm text-gray-500 mt-2">
                          Due: {new Date(task.due_date).toLocaleDateString()}
                        </div>
                      )}
                    </div>
                    <button
                      type="button"
                      onClick={() => deleteMutation.mutate(task.id)}
                      disabled={deleteMutation.isPending}
                      class="text-red-500 hover:text-red-700 ml-4 disabled:opacity-50"
                    >
                      Delete
                    </button>
                  </div>
                </div>
              )}
            </For>
          </div>
        </Show>
      </Show>
    </div>
  );
}
