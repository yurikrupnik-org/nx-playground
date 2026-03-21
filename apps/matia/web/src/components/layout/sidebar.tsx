import { Link, useMatchRoute } from '@tanstack/solid-router';
import { For } from 'solid-js';
import { cn } from '../../lib/utils';

interface NavItem {
  label: string;
  href: string;
  icon: string;
  section?: string;
}

const navItems: NavItem[] = [
  { label: 'Catalog', href: '/catalog', icon: '\u{1F4DA}', section: 'Data' },
  { label: 'ETL', href: '/etl', icon: '\u{2B07}\u{FE0F}', section: 'Data' },
  { label: 'Reverse ETL', href: '/reverse-etl', icon: '\u{2B06}\u{FE0F}', section: 'Data' },
  { label: 'Observability', href: '/observability', icon: '\u{1F4CA}', section: 'Data' },
  { label: 'Issues', href: '/issues', icon: '\u{26A0}\u{FE0F}', section: 'Operate' },
  { label: 'Integrations', href: '/integrations', icon: '\u{1F50C}', section: 'Operate' },
  { label: 'Designs', href: '/designs', icon: '\u{1F3A8}', section: 'Build' },
  { label: 'Settings', href: '/settings', icon: '\u{2699}\u{FE0F}', section: 'System' },
];

function groupBySection(items: NavItem[]): Map<string, NavItem[]> {
  const groups = new Map<string, NavItem[]>();
  for (const item of items) {
    const section = item.section ?? '';
    if (!groups.has(section)) groups.set(section, []);
    groups.get(section)!.push(item);
  }
  return groups;
}

export function Sidebar() {
  const matchRoute = useMatchRoute();
  const sections = groupBySection(navItems);

  return (
    <aside class="fixed inset-y-0 left-0 w-56 bg-sidebar border-r border-border flex flex-col z-30">
      <div class="h-14 flex items-center px-4 border-b border-border">
        <span class="text-lg font-bold text-white tracking-tight">Matia</span>
        <span class="ml-2 text-xs text-muted bg-surface-raised px-1.5 py-0.5 rounded">beta</span>
      </div>

      <nav class="flex-1 overflow-y-auto py-3 px-2 space-y-4">
        <For each={[...sections.entries()]}>
          {([section, items]) => (
            <div>
              <p class="px-2 mb-1 text-[10px] font-semibold uppercase tracking-wider text-muted">
                {section}
              </p>
              <ul class="space-y-0.5">
                <For each={items}>
                  {(item) => {
                    const isActive = () => matchRoute({ to: item.href, fuzzy: true });
                    return (
                      <li>
                        <Link
                          to={item.href}
                          class={cn(
                            'flex items-center gap-2.5 rounded-md px-2 py-1.5 text-sm transition-colors',
                            isActive()
                              ? 'bg-sidebar-active text-sidebar-text-active'
                              : 'text-sidebar-text hover:bg-sidebar-hover hover:text-sidebar-text-active',
                          )}
                        >
                          <span class="w-5 text-center text-sm">{item.icon}</span>
                          {item.label}
                        </Link>
                      </li>
                    );
                  }}
                </For>
              </ul>
            </div>
          )}
        </For>
      </nav>

      <div class="p-3 border-t border-border">
        <div class="flex items-center gap-2 text-xs text-muted">
          <div class="w-6 h-6 rounded-full bg-primary/30 flex items-center justify-center text-primary text-[10px] font-bold">
            M
          </div>
          <span>matia v0.1.0</span>
        </div>
      </div>
    </aside>
  );
}
