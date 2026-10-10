/**
 * `/effect-v4` — the todo loop driven by an Effect 4 program. Same bridge shape
 * as `/effect` (Effect 3), so the pair measures the library version.
 *
 * `ManagedRuntime` provides `TodoApiLive`. Every fiber the route starts — the
 * state→signal mirror, the initial load, each click and stream event — runs
 * through one `FiberSet` promise runner whose scope the route owns; closing it
 * on cleanup interrupts them all, so an in-flight retry (and its HTTP request)
 * cannot outlive the route.
 */
import {
  Effect,
  Exit,
  FiberSet,
  ManagedRuntime,
  Scope,
  Stream,
  SubscriptionRef,
} from 'effect';
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
  type TodoApi,
  TodoApiLive,
  type TodoState,
  type TodoStateRef,
  toggleTodo,
} from './todo-effect-v4';

export default function EffectV4Route() {
  const [state, setState] = createSignal<TodoState>(initialState, {
    equals: false,
  });
  const [busy, setBusy] = createSignal(false);

  const runtime = ManagedRuntime.make(TodoApiLive);
  const scope = Scope.makeUnsafe();

  // The ref and the runner are created inside the runtime, so hand the route a
  // promise for them and serialise interactions behind it.
  const ready = runtime.runPromise(
    Effect.gen(function* () {
      const run = yield* FiberSet.makeRuntimePromise<TodoApi>();
      const ref = yield* makeStateRef;
      return { ref, run };
    }).pipe(Scope.provide(scope)),
  );

  let disposed = false;

  /**
   * Run one Effect against the ref; `busy` covers the write path only.
   *
   * Always via `ready.then`, i.e. after the component body has returned: Effect
   * 4 runs a new fiber synchronously up to its first async boundary, so a fiber
   * started from the body would write the signal inside Solid's owned scope,
   * which Solid 2 rejects (`REACTIVE_WRITE_IN_OWNED_SCOPE`). `/effect` forks
   * from inside Effect 3's `runPromise`, which does not hit this.
   */
  const dispatch = (
    build: (ref: TodoStateRef) => Effect.Effect<unknown, never, TodoApi>,
    tracked = false,
  ) => {
    if (tracked) setBusy(true);
    void ready
      .then(({ ref, run }) => run(build(ref)))
      .catch((defect: unknown) => {
        // Every program here has `E = never`: a rejection is either cleanup
        // interrupting the fiber (expected) or a defect (a bug — surface it).
        if (!disposed) console.error('[effect-v4] fiber died', defect);
      })
      .finally(() => {
        if (tracked) setBusy(false);
      });
  };

  // Mirror state into a signal for rendering, then load.
  dispatch((ref) =>
    Stream.runForEach(SubscriptionRef.changes(ref), (next) =>
      Effect.sync(() => setState(next)),
    ),
  );
  dispatch(load);

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
    disposed = true;
    unsubscribe();
    // Interrupts the mirror fiber, the initial load and any in-flight write or
    // retry, then releases the runtime.
    void runtime
      .runPromise(Scope.close(scope, Exit.void))
      .finally(() => runtime.dispose());
  });

  return (
    <TodoView
      variant="Effect v4"
      summary="Effect 4: SubscriptionRef + Stream, TodoApi as a service"
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
