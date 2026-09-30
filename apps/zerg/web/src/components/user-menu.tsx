import { createSignal, onCleanup, onMount, Show } from 'solid-js';
import { useAuth } from '../lib/auth';
import { Button } from './ui/button';

export function UserMenu() {
  const auth = useAuth();
  const [isOpen, setIsOpen] = createSignal(false);

  const getInitials = (name: string) => {
    return name
      .split(' ')
      .map((n) => n[0])
      .join('')
      .toUpperCase()
      .slice(0, 2);
  };

  const handleLogout = () => {
    setIsOpen(false);
    auth.logout();
  };

  const handleKeyDown = (e: KeyboardEvent) => {
    if (e.key === 'Escape' && isOpen()) {
      setIsOpen(false);
    }
  };

  onMount(() => {
    document.addEventListener('keydown', handleKeyDown);
  });

  onCleanup(() => {
    document.removeEventListener('keydown', handleKeyDown);
  });

  return (
    <Show when={auth.isAuthenticated() && auth.user()}>
      {(user) => (
        <div class="relative">
          <Button
            variant="ghost"
            class="flex items-center gap-2"
            onClick={() => setIsOpen(!isOpen())}
          >
            <div class="h-8 w-8 rounded-full bg-primary text-primary-foreground flex items-center justify-center text-sm font-medium">
              <Show
                when={user().avatar_url}
                fallback={getInitials(user().name)}
              >
                {(avatarUrl) => (
                  <img
                    src={avatarUrl()}
                    alt={user().name}
                    class="h-full w-full rounded-full object-cover"
                  />
                )}
              </Show>
            </div>
            <Show when={window.innerWidth > 640}>
              <span class="text-sm font-medium">{user().name}</span>
            </Show>
          </Button>

          <Show when={isOpen()}>
            <div
              role="menu"
              class="absolute right-0 mt-2 w-56 rounded-md border bg-card shadow-lg z-50"
              onKeyDown={(e) => e.key === 'Escape' && setIsOpen(false)}
            >
              <div class="border-b px-3 py-2">
                <p class="text-sm font-semibold">{user().name}</p>
                <p class="text-xs text-muted-foreground">{user().email}</p>
              </div>
              <div class="p-1">
                <button
                  type="button"
                  onClick={handleLogout}
                  class="w-full text-left px-3 py-2 text-sm text-red-600 hover:bg-accent rounded-sm"
                >
                  Sign out
                </button>
              </div>
            </div>
            {/* Backdrop to close menu when clicking outside */}
            <button
              type="button"
              class="fixed inset-0 z-40 cursor-default bg-transparent border-none"
              onClick={() => setIsOpen(false)}
              aria-label="Close menu"
            />
          </Show>
        </div>
      )}
    </Show>
  );
}
