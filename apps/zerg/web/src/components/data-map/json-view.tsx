import { createMemo, createSignal, For } from 'solid-js';

// One regex pass over pretty-printed JSON: keys, strings, numbers, literals.
const TOKEN =
  /("(?:\\.|[^"\\])*"(?=\s*:))|("(?:\\.|[^"\\])*")|(-?\d+(?:\.\d+)?(?:[eE][+-]?\d+)?)|\b(true|false|null)\b/g;

const TOKEN_CLASSES = [
  'text-indigo-700', // key
  'text-emerald-700', // string
  'text-sky-700', // number
  'text-amber-700', // literal
];

/** Pretty, highlighted JSON with a copy button. */
export function JsonView(props: { value: unknown; class?: string }) {
  const text = createMemo(() => JSON.stringify(props.value, null, 2) ?? '');
  const tokens = createMemo(() => {
    const out: { text: string; class?: string }[] = [];
    let last = 0;
    for (const m of text().matchAll(TOKEN)) {
      if (m.index > last) out.push({ text: text().slice(last, m.index) });
      const group = m.slice(1).findIndex((g) => g !== undefined);
      out.push({ text: m[0], class: TOKEN_CLASSES[group] });
      last = m.index + m[0].length;
    }
    out.push({ text: text().slice(last) });
    return out;
  });
  const [copied, setCopied] = createSignal(false);

  return (
    <div class={`relative ${props.class ?? ''}`}>
      <button
        type="button"
        class="absolute right-2 top-2 rounded border bg-white px-2 py-0.5 text-xs text-gray-600 shadow-sm hover:bg-gray-50 border-gray-200"
        onClick={async () => {
          await navigator.clipboard.writeText(text());
          setCopied(true);
          setTimeout(() => setCopied(false), 1200);
        }}
      >
        {copied() ? 'Copied' : 'Copy'}
      </button>
      <pre class="overflow-auto rounded-lg bg-gray-50 p-4 font-mono text-xs leading-relaxed text-gray-700 max-h-[70vh]">
        <For each={tokens()}>
          {(t) => <span class={t.class}>{t.text}</span>}
        </For>
      </pre>
    </div>
  );
}
