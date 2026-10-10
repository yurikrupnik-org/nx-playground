import { useLocation } from '@solidjs/router';
import { For } from 'solid-js';

/**
 * Switches between the four state-management implementations of the same todo
 * loop. Plain anchors: the router intercepts them (`explicitLinks` defaults to
 * off), and hovering preloads the target chunk.
 */
const ROUTES = [
  {
    href: '/',
    label: 'TanStack Query',
    hint: 'baseline: query cache + signals',
  },
  { href: '/xstate', label: 'XState', hint: 'explicit state machine' },
  { href: '/effect', label: 'Effect 3', hint: 'SubscriptionRef + Stream' },
  {
    href: '/effect-v4',
    label: 'Effect 4',
    hint: 'SubscriptionRef + Stream, TodoApi service',
  },
] as const;

export function AppNav() {
  const location = useLocation();

  return (
    <nav class="app-nav" aria-label="state management implementation">
      <For each={ROUTES}>
        {(route) => (
          <a
            class={[
              'app-nav__link',
              { 'app-nav__link--active': location.pathname === route.href },
            ]}
            href={route.href}
            title={route.hint}
          >
            {route.label}
          </a>
        )}
      </For>
    </nav>
  );
}
