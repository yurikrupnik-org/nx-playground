export function SettingsPage() {
  return (
    <div>
      <div class="mb-6">
        <h1 class="text-2xl font-bold">Settings</h1>
        <p class="text-sm text-muted mt-1">Platform configuration</p>
      </div>

      <div class="space-y-6 max-w-2xl">
        <section class="bg-surface border border-border rounded-lg p-5">
          <h2 class="font-medium text-slate-100 mb-4">General</h2>
          <div class="space-y-4">
            <div>
              <label class="block text-sm text-muted mb-1">Platform Name</label>
              <input
                type="text"
                value="Matia"
                class="w-full bg-surface-raised border border-border rounded-md px-3 py-2 text-sm text-slate-100 outline-none focus:border-primary"
              />
            </div>
            <div>
              <label class="block text-sm text-muted mb-1">Default Namespace</label>
              <input
                type="text"
                value="dbs"
                class="w-full bg-surface-raised border border-border rounded-md px-3 py-2 text-sm text-slate-100 outline-none focus:border-primary"
              />
            </div>
          </div>
        </section>

        <section class="bg-surface border border-border rounded-lg p-5">
          <h2 class="font-medium text-slate-100 mb-4">Scheduling</h2>
          <div class="space-y-4">
            <div>
              <label class="block text-sm text-muted mb-1">NATS URL</label>
              <input
                type="text"
                value="nats://localhost:4222"
                class="w-full bg-surface-raised border border-border rounded-md px-3 py-2 text-sm text-slate-100 font-mono outline-none focus:border-primary"
              />
            </div>
            <div>
              <label class="block text-sm text-muted mb-1">Dapr Pub/Sub Component</label>
              <input
                type="text"
                value="pubsub-nats"
                class="w-full bg-surface-raised border border-border rounded-md px-3 py-2 text-sm text-slate-100 font-mono outline-none focus:border-primary"
              />
            </div>
          </div>
        </section>

        <section class="bg-surface border border-border rounded-lg p-5">
          <h2 class="font-medium text-slate-100 mb-4">Data Quality</h2>
          <div class="space-y-4">
            <div>
              <label class="block text-sm text-muted mb-1">Default Freshness Threshold</label>
              <input
                type="text"
                value="1h"
                class="w-full bg-surface-raised border border-border rounded-md px-3 py-2 text-sm text-slate-100 outline-none focus:border-primary"
              />
            </div>
            <div class="flex items-center justify-between">
              <div>
                <p class="text-sm text-slate-100">Schema Drift Detection</p>
                <p class="text-xs text-muted">Alert when dataset schemas change between runs</p>
              </div>
              <div class="w-10 h-6 bg-primary rounded-full relative cursor-pointer">
                <div class="absolute right-0.5 top-0.5 w-5 h-5 bg-white rounded-full shadow" />
              </div>
            </div>
          </div>
        </section>
      </div>
    </div>
  );
}
