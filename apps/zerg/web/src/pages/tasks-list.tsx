import type { TaskPriority, TaskStatus } from '@domain/tasks';
import { useMutation, useQuery, useQueryClient } from '@tanstack/solid-query';
import { Link } from '@tanstack/solid-router';
import { createSignal, createUniqueId, For, Show } from 'solid-js';
import type { CreateTaskInput } from '../lib/api-client';
import { tasksApi } from '../lib/api-client';
import { useAuth } from '../lib/auth-context';

export function TasksListPage() {
  const queryClient = useQueryClient();
  const auth = useAuth();
  const [filter, setFilter] = createSignal<
    'all' | 'todo' | 'in_progress' | 'done'
  >('all');
  // Org-wide vs "only mine" — meaningful only for real orgs (personal
  // workspaces are single-user by construction).
  const [scope, setScope] = createSignal<'all' | 'mine'>('all');
  const [showCreate, setShowCreate] = createSignal(false);

  const tasksQuery = useQuery(() => ({
    queryKey: ['tasks', scope()] as const,
    queryFn: () => {
      const user = auth.user();
      return tasksApi.list(
        scope() === 'mine' && user ? { user_id: user.id } : undefined,
      );
    },
  }));

  const deleteMutation = useMutation(() => ({
    mutationFn: tasksApi.delete,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['tasks'] });
    },
  }));

  // Create-task form state (pattern from task-detail.tsx / terran assets.tsx)
  const [title, setTitle] = createSignal('');
  const [description, setDescription] = createSignal('');
  const [priority, setPriority] = createSignal<TaskPriority>('medium');
  const [status, setStatus] = createSignal<TaskStatus>('todo');
  const [dueDate, setDueDate] = createSignal('');
  const titleId = createUniqueId();
  const descriptionId = createUniqueId();
  const priorityId = createUniqueId();
  const statusId = createUniqueId();
  const dueDateId = createUniqueId();

  const createMutation = useMutation(() => ({
    mutationFn: (input: CreateTaskInput) => tasksApi.create(input),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['tasks'] });
      setTitle('');
      setDescription('');
      setPriority('medium');
      setStatus('todo');
      setDueDate('');
      setShowCreate(false);
    },
  }));

  const submitCreate = (e: Event) => {
    e.preventDefault();
    if (!title().trim()) return;
    createMutation.mutate({
      title: title().trim(),
      description: description(),
      project_id: null,
      priority: priority(),
      status: status(),
      due_date: dueDate() ? new Date(dueDate()).toISOString() : null,
    });
  };

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
        <div>
          <h1 class="text-3xl font-bold">Tasks</h1>
          <Show when={auth.user()?.org} keyed>
            {(org) => (
              <p class="text-sm text-gray-500 mt-1">
                {org.name}
                {' · '}
                {org.is_personal ? 'personal workspace' : `role ${org.role}`}
              </p>
            )}
          </Show>
        </div>
        <button
          type="button"
          class="px-4 py-2 rounded bg-blue-600 text-white hover:bg-blue-700"
          onClick={() => setShowCreate(!showCreate())}
        >
          {showCreate() ? 'Cancel' : 'New Task'}
        </button>
      </div>

      {/* Create form */}
      <Show when={showCreate()}>
        <form
          onSubmit={submitCreate}
          class="border rounded-lg p-4 mb-6 bg-white space-y-4"
        >
          <div>
            <label class="block text-sm font-medium mb-1" for={titleId}>
              Title
            </label>
            <input
              id={titleId}
              class="w-full border rounded px-3 py-2"
              value={title()}
              onInput={(e) => setTitle(e.currentTarget.value)}
              placeholder="What needs doing?"
              required
            />
          </div>
          <div>
            <label class="block text-sm font-medium mb-1" for={descriptionId}>
              Description
            </label>
            <textarea
              id={descriptionId}
              class="w-full border rounded px-3 py-2"
              rows="2"
              value={description()}
              onInput={(e) => setDescription(e.currentTarget.value)}
            />
          </div>
          <div class="flex gap-4">
            <div>
              <label class="block text-sm font-medium mb-1" for={priorityId}>
                Priority
              </label>
              <select
                id={priorityId}
                class="border rounded px-3 py-2"
                value={priority()}
                onChange={(e) =>
                  setPriority(e.currentTarget.value as TaskPriority)
                }
              >
                <option value="low">Low</option>
                <option value="medium">Medium</option>
                <option value="high">High</option>
                <option value="urgent">Urgent</option>
              </select>
            </div>
            <div>
              <label class="block text-sm font-medium mb-1" for={statusId}>
                Status
              </label>
              <select
                id={statusId}
                class="border rounded px-3 py-2"
                value={status()}
                onChange={(e) => setStatus(e.currentTarget.value as TaskStatus)}
              >
                <option value="todo">To Do</option>
                <option value="in_progress">In Progress</option>
                <option value="done">Done</option>
              </select>
            </div>
            <div>
              <label class="block text-sm font-medium mb-1" for={dueDateId}>
                Due date
              </label>
              <input
                id={dueDateId}
                type="date"
                class="border rounded px-3 py-2"
                value={dueDate()}
                onInput={(e) => setDueDate(e.currentTarget.value)}
              />
            </div>
          </div>
          <Show when={createMutation.isError}>
            <p class="text-sm text-red-600">
              {createMutation.error?.message ?? 'Failed to create task'}
            </p>
          </Show>
          <button
            type="submit"
            disabled={createMutation.isPending || !title().trim()}
            class="px-4 py-2 rounded bg-blue-600 text-white hover:bg-blue-700 disabled:opacity-50"
          >
            {createMutation.isPending ? 'Creating…' : 'Create Task'}
          </button>
        </form>
      </Show>

      {/* Filters */}
      <div class="flex gap-2 mb-4 items-center">
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
        {/* Org scope toggle: hidden for personal workspaces */}
        <Show when={auth.user()?.org.is_personal === false}>
          <div class="ml-auto flex gap-2">
            <button
              type="button"
              class={`px-4 py-2 rounded ${scope() === 'all' ? 'bg-indigo-500 text-white' : 'bg-gray-200'}`}
              onClick={() => setScope('all')}
            >
              Everyone
            </button>
            <button
              type="button"
              class={`px-4 py-2 rounded ${scope() === 'mine' ? 'bg-indigo-500 text-white' : 'bg-gray-200'}`}
              onClick={() => setScope('mine')}
            >
              Mine
            </button>
          </div>
        </Show>
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
                        {task.completed && (
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
