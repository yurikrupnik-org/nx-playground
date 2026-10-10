/**
 * `/xstate` — the todo loop driven by an XState machine.
 *
 * `@xstate/solid` peers on `solid-js@^1.6` (this app is on Solid 2), so the actor
 * is bridged to a signal here. That bridge is the whole integration: ~10 lines,
 * and it makes the version mismatch a non-issue.
 */
import { createSignal, onCleanup } from 'solid-js';
import { createActor } from 'xstate';

import { TodoView, type TodoViewStatus } from '../components/todo-view';
import { subscribeToTodoEvents } from '../lib/realtime';
import { type TodoSnapshot, todoMachine } from './todo-machine';

export default function XStateRoute() {
  const actor = createActor(todoMachine);
  const [snapshot, setSnapshot] = createSignal<TodoSnapshot>(
    actor.getSnapshot(),
    // Snapshots are new objects but `matches`/context may be structurally equal;
    // always notify so `For` sees the new array identity.
    { equals: false },
  );

  const subscription = actor.subscribe(setSnapshot);
  actor.start();

  // The stream is a set of events into the machine, not a second state owner.
  const unsubscribe = subscribeToTodoEvents(
    (event) => actor.send({ type: 'change', event }),
    {
      onOpen: () => actor.send({ type: 'streamOpen' }),
      onError: () => actor.send({ type: 'streamError' }),
    },
  );

  onCleanup(() => {
    unsubscribe();
    subscription.unsubscribe();
    actor.stop();
  });

  const status = (): TodoViewStatus => {
    const snap = snapshot();
    if (snap.matches('failed')) return 'error';
    return snap.matches('loading') ? 'loading' : 'ready';
  };

  return (
    <TodoView
      variant="XState"
      summary="one machine owns load, write and resync states"
      todos={snapshot().context.todos}
      status={status()}
      error={snapshot().context.error}
      busy={snapshot().matches({ ready: 'saving' })}
      live={snapshot().context.live}
      onAdd={(input) => actor.send({ type: 'add', input })}
      onToggle={(todo) => actor.send({ type: 'toggle', todo })}
      onRemove={(id) => actor.send({ type: 'remove', id })}
    />
  );
}
