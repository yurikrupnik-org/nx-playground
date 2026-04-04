import type { TaskPriority, TaskStatus } from '@domain/tasks';
import { useMutation, useQuery, useQueryClient } from '@tanstack/solid-query';
import { useNavigate, useParams } from '@tanstack/solid-router';
import { createSignal, createUniqueId, Show } from 'solid-js';
import { tasksApi, type UpdateTaskInput } from '../lib/api-client';

export function TaskDetailPage() {
  const params = useParams({ from: '/tasks/$id' });
  const navigate = useNavigate();
  const queryClient = useQueryClient();

  // Generate unique IDs for form fields
  const titleId = createUniqueId();
  const descriptionId = createUniqueId();
  const statusId = createUniqueId();
  const priorityId = createUniqueId();

  const taskQuery = useQuery(() => ({
    queryKey: ['tasks', params().id],
    queryFn: () => tasksApi.getById(params().id),
  }));

  const updateMutation = useMutation(() => ({
    mutationFn: ({ id, input }: { id: string; input: UpdateTaskInput }) =>
      tasksApi.update(id, input),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ['tasks'] });
      await queryClient.invalidateQueries({ queryKey: ['tasks', params().id] });
    },
  }));

  const [editing, setEditing] = createSignal(false);
  const [formData, setFormData] = createSignal<UpdateTaskInput>({
    title: null,
    description: null,
    project_id: null,
    priority: null,
    status: null,
    due_date: null,
  });

  const handleEdit = () => {
    const task = taskQuery.data;
    if (task) {
      setFormData({
        title: task.title,
        description: task.description,
        status: task.status,
        priority: task.priority,
        project_id: task.project_id,
        due_date: task.due_date,
      });
      setEditing(true);
    }
  };

  const handleSave = () => {
    updateMutation.mutate(
      { id: params().id, input: formData() },
      {
        onSuccess: () => {
          setEditing(false);
        },
      },
    );
  };

  return (
    <div class="container mx-auto p-4 max-w-4xl">
      <Show
        when={!taskQuery.isLoading && taskQuery.data}
        fallback={<div class="text-center py-8">Loading...</div>}
      >
        {(task) => (
          <div>
            <div class="flex justify-between items-center mb-6">
              <button
                type="button"
                onClick={() => navigate({ to: '/tasks' })}
                class="text-blue-500 hover:text-blue-700"
              >
                ← Back to Tasks
              </button>
              <Show when={!editing()}>
                <button
                  type="button"
                  onClick={handleEdit}
                  class="bg-blue-500 text-white px-4 py-2 rounded hover:bg-blue-600"
                >
                  Edit
                </button>
              </Show>
            </div>

            <Show
              when={editing()}
              fallback={
                <div class="bg-white rounded-lg shadow p-6">
                  <h1 class="text-3xl font-bold mb-4">{task().title}</h1>
                  <p class="text-gray-700 mb-4">{task().description}</p>
                  <div class="flex gap-4 mb-4 flex-wrap">
                    <div>
                      <span class="font-semibold">Status: </span>
                      <span class="capitalize">
                        {task().status.replace('_', ' ')}
                      </span>
                    </div>
                    <div>
                      <span class="font-semibold">Priority: </span>
                      <span class="capitalize">{task().priority}</span>
                    </div>
                    <div>
                      <span class="font-semibold">Completed: </span>
                      <span>{task().status === 'done' ? 'Yes' : 'No'}</span>
                    </div>
                  </div>
                  <Show when={task().due_date}>
                    {(dueDate) => (
                      <div class="mb-4">
                        <span class="font-semibold">Due Date: </span>
                        <span>{new Date(dueDate()).toLocaleDateString()}</span>
                      </div>
                    )}
                  </Show>
                  <div class="text-sm text-gray-500 space-y-1">
                    <div>
                      Created: {new Date(task().created_at).toLocaleString()}
                    </div>
                    <div>
                      Updated: {new Date(task().updated_at).toLocaleString()}
                    </div>
                  </div>
                </div>
              }
            >
              <div class="bg-white rounded-lg shadow p-6 space-y-4">
                <div>
                  <label for={titleId} class="block font-semibold mb-1">
                    Title
                  </label>
                  <input
                    id={titleId}
                    type="text"
                    value={formData().title || ''}
                    onInput={(e) =>
                      setFormData({
                        ...formData(),
                        title: e.currentTarget.value,
                      })
                    }
                    class="w-full border rounded px-3 py-2"
                  />
                </div>
                <div>
                  <label for={descriptionId} class="block font-semibold mb-1">
                    Description
                  </label>
                  <textarea
                    id={descriptionId}
                    value={formData().description || ''}
                    onInput={(e) =>
                      setFormData({
                        ...formData(),
                        description: e.currentTarget.value,
                      })
                    }
                    class="w-full border rounded px-3 py-2"
                    rows={4}
                  />
                </div>
                <div class="flex gap-4">
                  <div class="flex-1">
                    <label for={statusId} class="block font-semibold mb-1">
                      Status
                    </label>
                    <select
                      id={statusId}
                      value={formData().status || ''}
                      onChange={(e) =>
                        setFormData({
                          ...formData(),
                          status: e.currentTarget.value as TaskStatus,
                        })
                      }
                      class="w-full border rounded px-3 py-2"
                    >
                      <option value="todo">To Do</option>
                      <option value="in_progress">In Progress</option>
                      <option value="done">Done</option>
                    </select>
                  </div>
                  <div class="flex-1">
                    <label for={priorityId} class="block font-semibold mb-1">
                      Priority
                    </label>
                    <select
                      id={priorityId}
                      value={formData().priority || ''}
                      onChange={(e) =>
                        setFormData({
                          ...formData(),
                          priority: e.currentTarget.value as TaskPriority,
                        })
                      }
                      class="w-full border rounded px-3 py-2"
                    >
                      <option value="low">Low</option>
                      <option value="medium">Medium</option>
                      <option value="high">High</option>
                      <option value="urgent">Urgent</option>
                    </select>
                  </div>
                </div>
                <div class="flex items-center gap-2">
                  <label class="flex items-center gap-2">
                    <input
                      type="checkbox"
                      checked={formData().status === 'done'}
                      onChange={(e) =>
                        setFormData({
                          ...formData(),
                          status: e.currentTarget.checked ? 'done' : 'todo',
                        })
                      }
                      class="rounded"
                    />
                    Mark as completed
                  </label>
                </div>
                <div class="flex gap-2">
                  <button
                    type="button"
                    onClick={handleSave}
                    disabled={updateMutation.isPending}
                    class="bg-blue-500 text-white px-4 py-2 rounded hover:bg-blue-600 disabled:opacity-50"
                  >
                    {updateMutation.isPending ? 'Saving...' : 'Save'}
                  </button>
                  <button
                    type="button"
                    onClick={() => setEditing(false)}
                    class="bg-gray-300 px-4 py-2 rounded hover:bg-gray-400"
                  >
                    Cancel
                  </button>
                </div>
              </div>
            </Show>
          </div>
        )}
      </Show>
    </div>
  );
}
