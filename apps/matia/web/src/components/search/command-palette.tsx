import { useNavigate } from '@tanstack/solid-router';
import { createSignal, For, onCleanup, onMount, Show } from 'solid-js';
import { cn } from '../../lib/utils';

interface CommandItem {
  id: string;
  label: string;
  section: string;
  href: string;
  icon: string;
}

const commands: CommandItem[] = [
  { id: 'catalog', label: 'Data Catalog', section: 'Navigate', href: '/catalog', icon: '\u{1F4DA}' },
  { id: 'etl', label: 'ETL Pipelines', section: 'Navigate', href: '/etl', icon: '\u{2B07}\u{FE0F}' },
  { id: 'retl', label: 'Reverse ETL', section: 'Navigate', href: '/reverse-etl', icon: '\u{2B06}\u{FE0F}' },
  { id: 'obs', label: 'Observability', section: 'Navigate', href: '/observability', icon: '\u{1F4CA}' },
  { id: 'issues', label: 'Issues', section: 'Navigate', href: '/issues', icon: '\u{26A0}\u{FE0F}' },
  { id: 'integrations', label: 'Integrations', section: 'Navigate', href: '/integrations', icon: '\u{1F50C}' },
  { id: 'designs', label: 'Designs', section: 'Navigate', href: '/designs', icon: '\u{1F3A8}' },
  { id: 'settings', label: 'Settings', section: 'Navigate', href: '/settings', icon: '\u{2699}\u{FE0F}' },
];

interface Props {
  open: boolean;
  onClose: () => void;
}

export function CommandPalette(props: Props) {
  const navigate = useNavigate();
  const [query, setQuery] = createSignal('');
  const [activeIndex, setActiveIndex] = createSignal(0);
  let inputRef: HTMLInputElement | undefined;

  const filtered = () => {
    const q = query().toLowerCase();
    if (!q) return commands;
    return commands.filter(
      (c) =>
        c.label.toLowerCase().includes(q) ||
        c.section.toLowerCase().includes(q),
    );
  };

  const handleKeyDown = (e: KeyboardEvent) => {
    if (e.key === 'Escape') {
      props.onClose();
    } else if (e.key === 'ArrowDown') {
      e.preventDefault();
      setActiveIndex((i) => Math.min(i + 1, filtered().length - 1));
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      setActiveIndex((i) => Math.max(i - 1, 0));
    } else if (e.key === 'Enter') {
      const item = filtered()[activeIndex()];
      if (item) {
        navigate({ to: item.href });
        props.onClose();
      }
    }
  };

  onMount(() => {
    if (props.open) inputRef?.focus();
  });

  return (
    <Show when={props.open}>
      <div
        class="fixed inset-0 z-50 bg-black/60 flex items-start justify-center pt-[20vh]"
        onClick={() => props.onClose()}
      >
        <div
          class="w-full max-w-lg bg-surface border border-border rounded-xl shadow-2xl overflow-hidden"
          onClick={(e) => e.stopPropagation()}
        >
          <div class="flex items-center border-b border-border px-4">
            <svg class="w-4 h-4 text-muted mr-2 shrink-0" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M21 21l-6-6m2-5a7 7 0 11-14 0 7 7 0 0114 0z" />
            </svg>
            <input
              ref={inputRef}
              type="text"
              placeholder="Search pages, datasets, pipelines..."
              class="flex-1 bg-transparent py-3 text-sm text-slate-100 placeholder:text-muted outline-none"
              value={query()}
              onInput={(e) => {
                setQuery(e.currentTarget.value);
                setActiveIndex(0);
              }}
              onKeyDown={handleKeyDown}
            />
          </div>

          <ul class="max-h-72 overflow-y-auto py-2">
            <For each={filtered()}>
              {(item, index) => (
                <li>
                  <button
                    type="button"
                    class={cn(
                      'w-full flex items-center gap-3 px-4 py-2 text-sm text-left transition-colors',
                      index() === activeIndex()
                        ? 'bg-sidebar-active text-white'
                        : 'text-sidebar-text hover:bg-sidebar-hover',
                    )}
                    onClick={() => {
                      navigate({ to: item.href });
                      props.onClose();
                    }}
                    onMouseEnter={() => setActiveIndex(index())}
                  >
                    <span class="w-5 text-center">{item.icon}</span>
                    <span>{item.label}</span>
                    <span class="ml-auto text-xs text-muted">{item.section}</span>
                  </button>
                </li>
              )}
            </For>
            <Show when={filtered().length === 0}>
              <li class="px-4 py-6 text-center text-sm text-muted">
                No results found
              </li>
            </Show>
          </ul>

          <div class="border-t border-border px-4 py-2 flex gap-4 text-[10px] text-muted">
            <span><kbd class="bg-surface-raised rounded px-1 py-0.5 font-mono">\u2191\u2193</kbd> navigate</span>
            <span><kbd class="bg-surface-raised rounded px-1 py-0.5 font-mono">\u23CE</kbd> select</span>
            <span><kbd class="bg-surface-raised rounded px-1 py-0.5 font-mono">esc</kbd> close</span>
          </div>
        </div>
      </div>
    </Show>
  );
}
