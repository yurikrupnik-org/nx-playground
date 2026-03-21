import { createSignal, onCleanup, onMount } from 'solid-js';

interface HeaderProps {
  onOpenSearch: () => void;
}

export function Header(props: HeaderProps) {
  const [isMac, setIsMac] = createSignal(true);

  onMount(() => {
    setIsMac(navigator.platform.toUpperCase().includes('MAC'));

    const handler = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key === 'k') {
        e.preventDefault();
        props.onOpenSearch();
      }
    };
    window.addEventListener('keydown', handler);
    onCleanup(() => window.removeEventListener('keydown', handler));
  });

  return (
    <header class="h-14 border-b border-border bg-sidebar flex items-center justify-between px-4">
      <div />
      <button
        type="button"
        onClick={props.onOpenSearch}
        class="flex items-center gap-2 rounded-md border border-border bg-surface px-3 py-1.5 text-sm text-muted hover:text-slate-300 hover:border-slate-600 transition-colors"
      >
        <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M21 21l-6-6m2-5a7 7 0 11-14 0 7 7 0 0114 0z" />
        </svg>
        <span>Search...</span>
        <kbd class="ml-4 text-xs bg-surface-raised rounded px-1.5 py-0.5 font-mono text-muted">
          {isMac() ? '\u2318' : 'Ctrl+'}K
        </kbd>
      </button>
      <div />
    </header>
  );
}
