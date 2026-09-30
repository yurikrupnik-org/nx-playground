/**
 * `/effect` — the todo loop driven by an Effect program.
 *
 * There is no official Effect binding for Solid 2 (`@effect-atom/atom-solid`
 * peers on `solid-js >=1 <2`), so the bridge is here: `ManagedRuntime` owns fiber
 * lifetime, and `Stream.runForEach` over the ref's `changes` pushes state into a
 * signal. `runtime.dispose()` on cleanup interrupts every fiber, so an in-flight
 * retry cannot outlive the route.
 */
import { Effect, Layer, ManagedRuntime, Stream } from 'effect';
import { createSignal, onCleanup } from 'solid-js';

import { TodoView } from '../components/todo-view';
import { subscribeToTodoEvents } from '../lib/realtime';
import {
  addTodo,
  applyChange,
  deleteTodo,
  initialState,
  load,
  makeStateRef,
  setLive,
  type TodoState,
  type TodoStateRef,
  toggleTodo,
} from './todo-effect';

export default function EffectRoute() {
  const [state, setState] = createSignal<TodoState>(initialState, {
    equals: false,
  });
  const [busy, setBusy] = createSignal(false);

  const runtime = ManagedRuntime.make(Layer.empty);

  // The ref is created inside the runtime, so hand the route a promise for it
  // and serialise interactions behind it.
  const refPromise: Promise<TodoStateRef> = runtime.runPromise(
    Effect.gen(function* () {
      const ref = yield* makeStateRef;

      // Mirror state into a signal for rendering.
      yield* Effect.forkDaemon(
        Stream.runForEach(ref.changes, (next) =>
          Effect.sync(() => setState(next)),
        ),
      );

      yield* Effect.forkDaemon(load(ref));
      return ref;
    }),
  );

  /** Run one Effect against the ref; `busy` covers the write path only. */
  const dispatch = (
    build: (ref: TodoStateRef) => Effect.Effect<unknown, never>,
    tracked = false,
  ) => {
    if (tracked) setBusy(true);
    void refPromise
      .then((ref) => runtime.runPromise(build(ref)))
      .finally(() => {
        if (tracked) setBusy(false);
      });
  };

  const unsubscribe = subscribeToTodoEvents(
    (event) => dispatch((ref) => applyChange(ref, event)),
    {
      onOpen: () => {
        // NOTIFY has no backlog: a reconnect must resynchronise.
        dispatch((ref) => Effect.andThen(setLive(ref, true), load(ref)));
      },
      onError: () => dispatch((ref) => setLive(ref, false)),
    },
  );

  onCleanup(() => {
    unsubscribe();
    // Interrupts the mirror fiber, the initial load and any in-flight retry.
    void runtime.dispose();
  });

  return (
    <TodoView
      variant="Effect"
      summary="SubscriptionRef holds state; Stream feeds the UI"
      todos={state().todos}
      status={state().status}
      error={state().error}
      busy={busy()}
      live={state().live}
      onAdd={(input) => dispatch((ref) => addTodo(ref, input), true)}
      onToggle={(todo) => dispatch((ref) => toggleTodo(ref, todo), true)}
      onRemove={(id) => dispatch((ref) => deleteTodo(ref, id), true)}
    />
  );
}
