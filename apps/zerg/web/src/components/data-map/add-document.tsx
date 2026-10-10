import { createSignal, createUniqueId, Show } from 'solid-js';
import { documentSource } from '../../lib/data-map/derive';

export interface StoredDocument {
  id: string;
  name: string;
  text: string;
}

/** Paste or drop a JSON Schema, OpenAPI document or CRD/XRD (JSON or YAML). */
export function AddDocument(props: {
  onAdd: (doc: StoredDocument) => void;
  onClose: () => void;
}) {
  const nameId = createUniqueId();
  const textId = createUniqueId();
  const [name, setName] = createSignal('');
  const [text, setText] = createSignal('');
  const [error, setError] = createSignal<string>();
  const [dragging, setDragging] = createSignal(false);

  const loadFile = async (file: File) => {
    setText(await file.text());
    if (!name()) setName(file.name);
  };

  const submit = (e: SubmitEvent) => {
    e.preventDefault();
    const doc = {
      id: `custom:${crypto.randomUUID()}`,
      name: name().trim() || 'Untitled schema',
      text: text(),
    };
    try {
      const source = documentSource(doc.id, doc.name, doc.text);
      if (!source.nodes.length) throw new Error('no schemas found in document');
      props.onAdd(doc);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  };

  return (
    <div
      class="fixed inset-0 z-50 flex items-center justify-center bg-gray-900/40 p-4"
      role="dialog"
      aria-modal="true"
      aria-label="Add schema"
      onKeyDown={(e) => e.key === 'Escape' && props.onClose()}
    >
      <form
        class="w-full max-w-2xl space-y-4 rounded-xl bg-white p-6 shadow-xl"
        onSubmit={submit}
      >
        <div>
          <h3 class="text-lg font-semibold">Add a schema</h3>
          <p class="text-sm text-gray-500">
            JSON Schema, OpenAPI document, or Kubernetes CRD / Crossplane XRD —
            JSON or YAML. Stored in this browser only.
          </p>
        </div>
        <div class="space-y-1">
          <label for={nameId} class="text-sm font-medium">
            Name
          </label>
          <input
            id={nameId}
            class="w-full rounded-md border px-3 py-2 text-sm border-gray-200"
            placeholder="e.g. order.schema.json"
            value={name()}
            onInput={(e) => setName(e.currentTarget.value)}
          />
        </div>
        <div class="space-y-1">
          <label for={textId} class="text-sm font-medium">
            Document
          </label>
          <textarea
            id={textId}
            class={`h-64 w-full rounded-md border px-3 py-2 font-mono text-xs ${dragging() ? 'border-indigo-400 bg-indigo-50' : 'border-gray-200'}`}
            placeholder="Paste here, or drop a .json / .yaml file"
            value={text()}
            onInput={(e) => {
              setText(e.currentTarget.value);
              setError(undefined);
            }}
            onDragOver={(e) => {
              e.preventDefault();
              setDragging(true);
            }}
            onDragLeave={() => setDragging(false)}
            onDrop={(e) => {
              e.preventDefault();
              setDragging(false);
              const file = e.dataTransfer?.files[0];
              if (file) void loadFile(file);
            }}
          />
          <input
            type="file"
            accept=".json,.yaml,.yml,application/json,application/yaml"
            class="text-sm"
            onChange={(e) => {
              const file = e.currentTarget.files?.[0];
              if (file) void loadFile(file);
            }}
          />
        </div>
        <Show when={error()}>
          <p class="rounded-md bg-red-50 p-2 text-sm text-red-700">{error()}</p>
        </Show>
        <div class="flex justify-end gap-2">
          <button
            type="button"
            class="rounded-md border px-4 py-2 text-sm hover:bg-gray-50 border-gray-200"
            onClick={props.onClose}
          >
            Cancel
          </button>
          <button
            type="submit"
            class="rounded-md bg-indigo-600 px-4 py-2 text-sm font-medium text-white hover:bg-indigo-700 disabled:opacity-50"
            disabled={!text().trim()}
          >
            Add
          </button>
        </div>
      </form>
    </div>
  );
}
