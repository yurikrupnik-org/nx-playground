import { Outlet } from '@tanstack/solid-router';
import { createSignal } from 'solid-js';
import { CommandPalette } from '../search/command-palette';
import { Header } from './header';
import { Sidebar } from './sidebar';

export function AppShell() {
  const [searchOpen, setSearchOpen] = createSignal(false);

  return (
    <div class="h-screen flex bg-slate-950 text-slate-100">
      <Sidebar />
      <div class="flex-1 ml-56 flex flex-col">
        <Header onOpenSearch={() => setSearchOpen(true)} />
        <main class="flex-1 overflow-y-auto p-6">
          <Outlet />
        </main>
      </div>
      <CommandPalette
        open={searchOpen()}
        onClose={() => setSearchOpen(false)}
      />
    </div>
  );
}
