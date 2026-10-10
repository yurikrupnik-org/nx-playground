/**
 * The todo loop as an explicit state machine (XState v5).
 *
 * The machine owns three things that are otherwise scattered across ad-hoc
 * signals in the baseline route:
 *
 * 1. **Load lifecycle** — `loading → ready | failed`, with `failed` able to
 *    retry rather than leaving the page stuck.
 * 2. **Write lifecycle** — writes queue behind `ready.saving`, so two rapid
 *    clicks cannot interleave two in-flight requests for the same row.
 * 3. **Resync obligation** — `NOTIFY` has no backlog, so a reconnect *must*
 *    refetch. As a state (`ready.resyncing`) that is impossible to forget; as a
 *    callback it is one `if` away from being dropped.
 *
 * Deliberately framework-free: no `@xstate/solid` (it peers on `solid-js@^1.6`
 * and this app runs Solid 2), so the route binds the actor to a signal itself.
 */
import type { CreateTodo, Todo, TodoEvent, TodoPriority } from '@domain/todo';
import { assign, fromPromise, type SnapshotFrom, setup } from 'xstate';

import { applyTodoEvent, removeTodo, upsertTodo } from '../lib/realtime';
import { todoApi } from '../lib/todo-api';

export interface TodoMachineContext {
  todos: Todo[];
  /** Last write failure, cleared when the next write starts. */
  error?: string;
  /** True once the event stream has connected at least once. */
  live: boolean;
}

export type TodoMachineEvent =
  | { type: 'retry' }
  | { type: 'add'; input: { title: string; priority: TodoPriority } }
  | { type: 'toggle'; todo: Todo }
  | { type: 'remove'; id: string }
  /** A database change arrived over SSE. */
  | { type: 'change'; event: TodoEvent }
  /** The stream (re)connected: whatever we missed must be refetched. */
  | { type: 'streamOpen' }
  | { type: 'streamError' };

/**
 * Writes the machine performs, as a single tagged input to one actor.
 *
 * Exported so tests can stub the actor with the real contract instead of a cast.
 */
export type WriteInput =
  | { kind: 'create'; input: CreateTodo }
  | { kind: 'toggle'; todo: Todo }
  | { kind: 'remove'; id: string };

export type WriteResult =
  | { kind: 'upsert'; todo: Todo }
  | { kind: 'remove'; id: string };

export const todoMachine = setup({
  types: {
    context: {} as TodoMachineContext,
    events: {} as TodoMachineEvent,
  },
  actors: {
    // XState hands promise actors an AbortSignal that fires when the actor is
    // stopped or the invoking state is exited, so a resync abandoned by a fast
    // navigation aborts its request instead of running to completion.
    loadTodos: fromPromise<Todo[]>(({ signal }) => todoApi.list(signal)),
    write: fromPromise(
      async ({ input }: { input: WriteInput }): Promise<WriteResult> => {
        switch (input.kind) {
          case 'create':
            return { kind: 'upsert', todo: await todoApi.create(input.input) };
          case 'toggle':
            return {
              kind: 'upsert',
              todo: input.todo.completed
                ? await todoApi.uncomplete(input.todo.id)
                : await todoApi.complete(input.todo.id),
            };
          case 'remove':
            await todoApi.remove(input.id);
            return { kind: 'remove', id: input.id };
        }
      },
    ),
  },
  actions: {
    /** Database events are merged with the same pure rules every route uses. */
    applyChange: assign({
      todos: ({ context, event }) =>
        event.type === 'change'
          ? applyTodoEvent(context.todos, event.event)
          : context.todos,
    }),
  },
}).createMachine({
  id: 'todo',
  initial: 'loading',
  context: { todos: [], live: false },
  // Stream signals are relevant in every state, so they live on the root.
  // `streamOpen` in particular MUST be handled here: the stream usually connects
  // while the first load is still in flight, and a root-less handler would drop
  // that event, leaving the UI permanently marked offline. Child states override
  // this to also refetch (`ready`) or retry (`failed`).
  on: {
    streamOpen: { actions: assign({ live: true }) },
    streamError: { actions: assign({ live: false }) },
  },
  states: {
    loading: {
      invoke: {
        src: 'loadTodos',
        onDone: {
          target: 'ready',
          actions: assign({
            todos: ({ event }) => event.output,
            error: undefined,
          }),
        },
        onError: 'failed',
      },
    },

    failed: {
      on: {
        retry: 'loading',
        // A reconnect is as good a reason to retry as a click.
        streamOpen: { target: 'loading', actions: assign({ live: true }) },
      },
    },

    ready: {
      initial: 'idle',
      // Changes apply in any `ready` substate, including mid-write.
      on: {
        change: { actions: 'applyChange' },
        streamOpen: { target: '.resyncing', actions: assign({ live: true }) },
      },
      states: {
        idle: {
          on: {
            add: {
              target: 'saving',
              actions: assign({ error: undefined }),
            },
            toggle: { target: 'saving', actions: assign({ error: undefined }) },
            remove: { target: 'saving', actions: assign({ error: undefined }) },
          },
        },

        /**
         * One write at a time. Further clicks are ignored rather than queued:
         * the row's next state arrives over the event stream anyway, so
         * replaying stale intent would fight the database.
         */
        saving: {
          invoke: {
            src: 'write',
            input: ({ event }): WriteInput => {
              switch (event.type) {
                case 'add':
                  return {
                    kind: 'create',
                    input: { ...event.input, description: '' },
                  };
                case 'toggle':
                  return { kind: 'toggle', todo: event.todo };
                case 'remove':
                  return { kind: 'remove', id: event.id };
                default:
                  throw new Error(`saving entered on ${event.type}`);
              }
            },
            onDone: {
              target: 'idle',
              actions: assign({
                todos: ({ context, event }) =>
                  event.output.kind === 'upsert'
                    ? upsertTodo(context.todos, event.output.todo)
                    : removeTodo(context.todos, event.output.id),
              }),
            },
            onError: {
              target: 'idle',
              actions: assign({
                error: ({ event }) =>
                  event.error instanceof Error
                    ? event.error.message
                    : 'write failed',
              }),
            },
          },
        },

        /** Reconnected: refetch, because missed notifications are unrecoverable. */
        resyncing: {
          invoke: {
            src: 'loadTodos',
            onDone: {
              target: 'idle',
              actions: assign({ todos: ({ event }) => event.output }),
            },
            // Keep showing what we have; the next reconnect tries again.
            onError: 'idle',
          },
        },
      },
    },
  },
});

/** The machine's snapshot, named here so consumers never reach for `ReturnType`. */
export type TodoSnapshot = SnapshotFrom<typeof todoMachine>;
