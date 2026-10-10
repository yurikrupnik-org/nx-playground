import { createMemo, createSignal, For, Show } from 'solid-js';
import { forceLayout } from '../../lib/data-map/layout';
import type { MapEdge, MapNode } from '../../lib/data-map/types';

const PALETTE = [
  ['#eef2ff', '#6366f1'],
  ['#ecfdf5', '#10b981'],
  ['#fff7ed', '#f97316'],
  ['#fdf2f8', '#ec4899'],
  ['#f0f9ff', '#0ea5e9'],
  ['#fefce8', '#eab308'],
  ['#f5f3ff', '#8b5cf6'],
] as const;

const NODE_HEIGHT = 30;
const CHAR_WIDTH = 7.2;

function groupColor(group: string | undefined, groups: string[]) {
  const i = group === undefined ? 0 : groups.indexOf(group);
  return PALETTE[Math.max(i, 0) % PALETTE.length];
}

/** Pan/zoom SVG map of nodes and their references (FKs, `$ref`s). */
export function MapGraph(props: {
  nodes: MapNode[];
  edges: MapEdge[];
  selected?: string;
  onSelect: (id: string) => void;
}) {
  const placed = createMemo(() =>
    forceLayout(
      props.nodes.map((n) => ({
        id: n.id,
        width: Math.max(80, n.label.length * CHAR_WIDTH + 28),
        height: NODE_HEIGHT,
      })),
      props.edges,
    ),
  );
  const byId = createMemo(() => new Map(placed().map((p) => [p.id, p])));
  const nodeById = createMemo(() => new Map(props.nodes.map((n) => [n.id, n])));
  const groups = createMemo(() => [
    ...new Set(props.nodes.map((n) => n.group ?? '')),
  ]);
  const bounds = createMemo(() => {
    const ps = placed();
    if (!ps.length) return { x: 0, y: 0, w: 100, h: 100 };
    const pad = 60;
    const minX = Math.min(...ps.map((p) => p.x - p.width / 2)) - pad;
    const maxX = Math.max(...ps.map((p) => p.x + p.width / 2)) + pad;
    const minY = Math.min(...ps.map((p) => p.y - p.height / 2)) - pad;
    const maxY = Math.max(...ps.map((p) => p.y + p.height / 2)) + pad;
    // A floor on the fitted area keeps 2–3 node graphs at natural text size.
    const w = Math.max(maxX - minX, 640);
    const h = Math.max(maxY - minY, 320);
    return {
      x: (minX + maxX - w) / 2,
      y: (minY + maxY - h) / 2,
      w,
      h,
    };
  });

  const [hover, setHover] = createSignal<string>();
  const focus = () => hover() ?? props.selected;
  const neighbours = createMemo(() => {
    const f = focus();
    const out = new Set<string>();
    if (!f) return out;
    out.add(f);
    for (const e of props.edges) {
      if (e.from === f) out.add(e.to);
      if (e.to === f) out.add(e.from);
    }
    return out;
  });

  // View transform on top of the fitted viewBox.
  const [zoom, setZoom] = createSignal(1);
  const [pan, setPan] = createSignal({ x: 0, y: 0 });
  let svg: SVGSVGElement | undefined;
  let drag: { x: number; y: number; px: number; py: number } | undefined;

  const viewBox = () => {
    const b = bounds();
    const z = zoom();
    const w = b.w / z;
    const h = b.h / z;
    const p = pan();
    return `${b.x + (b.w - w) / 2 + p.x} ${b.y + (b.h - h) / 2 + p.y} ${w} ${h}`;
  };
  /** Screen pixels → SVG user units at the current zoom. */
  const unitsPerPixel = () => {
    const rect = svg?.getBoundingClientRect();
    if (!rect || rect.width === 0) return 1;
    const b = bounds();
    return Math.max(b.w / rect.width, b.h / rect.height) / zoom();
  };

  return (
    <div class="relative h-full w-full select-none overflow-hidden rounded-lg border bg-[radial-gradient(circle,#e5e7eb_1px,transparent_1px)] [background-size:18px_18px] border-gray-200">
      <svg
        ref={svg}
        class="h-full w-full cursor-grab active:cursor-grabbing"
        viewBox={viewBox()}
        role="img"
        aria-label="Relationship map"
        onWheel={(e) => {
          e.preventDefault();
          const factor = e.deltaY < 0 ? 1.15 : 1 / 1.15;
          setZoom((z) => Math.min(8, Math.max(0.2, z * factor)));
        }}
        onPointerDown={(e) => {
          drag = { x: e.clientX, y: e.clientY, px: pan().x, py: pan().y };
          (e.currentTarget as Element).setPointerCapture(e.pointerId);
        }}
        onPointerMove={(e) => {
          if (!drag) return;
          const u = unitsPerPixel();
          setPan({
            x: drag.px - (e.clientX - drag.x) * u,
            y: drag.py - (e.clientY - drag.y) * u,
          });
        }}
        onPointerUp={() => {
          drag = undefined;
        }}
      >
        <defs>
          <marker
            id="dm-arrow"
            viewBox="0 0 10 10"
            refX="9"
            refY="5"
            markerWidth="7"
            markerHeight="7"
            orient="auto-start-reverse"
          >
            <path d="M0,0 L10,5 L0,10 z" fill="#94a3b8" />
          </marker>
          <marker
            id="dm-arrow-hi"
            viewBox="0 0 10 10"
            refX="9"
            refY="5"
            markerWidth="7"
            markerHeight="7"
            orient="auto-start-reverse"
          >
            <path d="M0,0 L10,5 L0,10 z" fill="#4f46e5" />
          </marker>
        </defs>
        <For each={props.edges}>
          {(edge) => {
            const a = () => byId().get(edge.from);
            const b = () => byId().get(edge.to);
            const active = () =>
              !!focus() && (edge.from === focus() || edge.to === focus());
            const dim = () => !!focus() && !active();
            const path = () => {
              const p = a();
              const q = b();
              if (!p || !q) return '';
              // Stop at the target card's border, not its centre.
              const dx = q.x - p.x;
              const dy = q.y - p.y;
              const t = Math.min(
                Math.abs(dx) > 0 ? q.width / 2 / Math.abs(dx) : 1,
                Math.abs(dy) > 0 ? q.height / 2 / Math.abs(dy) : 1,
              );
              const ex = q.x - dx * Math.min(t, 1);
              const ey = q.y - dy * Math.min(t, 1);
              const mx = (p.x + ex) / 2 - dy * 0.12;
              const my = (p.y + ey) / 2 + dx * 0.12;
              return `M${p.x},${p.y} Q${mx},${my} ${ex},${ey}`;
            };
            return (
              <g>
                <path
                  d={path()}
                  fill="none"
                  stroke={active() ? '#4f46e5' : '#cbd5e1'}
                  stroke-width={active() ? 2 : 1.2}
                  opacity={dim() ? 0.25 : 1}
                  marker-end={`url(#${active() ? 'dm-arrow-hi' : 'dm-arrow'})`}
                />
                <Show when={active() && edge.label}>
                  <text
                    x={((a()?.x ?? 0) + (b()?.x ?? 0)) / 2}
                    y={((a()?.y ?? 0) + (b()?.y ?? 0)) / 2 - 6}
                    text-anchor="middle"
                    class="fill-indigo-700 font-mono text-[11px]"
                  >
                    {edge.label}
                  </text>
                </Show>
              </g>
            );
          }}
        </For>
        <For each={placed()}>
          {(p) => {
            const node = () => nodeById().get(p.id);
            const colors = () => groupColor(node()?.group ?? '', groups());
            const isSelected = () => props.selected === p.id;
            const dim = () => !!focus() && !neighbours().has(p.id);
            return (
              // biome-ignore lint/a11y/useSemanticElements: <button> is not valid inside <svg>
              <g
                transform={`translate(${p.x - p.width / 2},${p.y - p.height / 2})`}
                class="cursor-pointer outline-none"
                role="button"
                tabIndex={0}
                aria-label={node()?.label}
                opacity={dim() ? 0.3 : 1}
                onPointerDown={(e) => e.stopPropagation()}
                onClick={() => props.onSelect(p.id)}
                onKeyDown={(e) => {
                  if (e.key === 'Enter' || e.key === ' ') props.onSelect(p.id);
                }}
                onMouseEnter={() => setHover(p.id)}
                onMouseLeave={() => setHover(undefined)}
                onFocus={() => setHover(p.id)}
                onBlur={() => setHover(undefined)}
              >
                <rect
                  width={p.width}
                  height={p.height}
                  rx="8"
                  fill={colors()[0]}
                  stroke={isSelected() ? '#111827' : colors()[1]}
                  stroke-width={isSelected() ? 2.5 : 1.2}
                />
                <circle cx="12" cy={p.height / 2} r="4" fill={colors()[1]} />
                <text
                  x="22"
                  y={p.height / 2 + 4}
                  class="fill-gray-800 font-mono text-[12px]"
                >
                  {node()?.label}
                </text>
              </g>
            );
          }}
        </For>
      </svg>
      <div class="absolute bottom-3 right-3 flex overflow-hidden rounded-md border bg-white text-sm shadow-sm border-gray-200">
        <button
          type="button"
          class="px-2.5 py-1 hover:bg-gray-50"
          onClick={() => setZoom((z) => Math.min(8, z * 1.25))}
        >
          +
        </button>
        <button
          type="button"
          class="border-x px-2.5 py-1 hover:bg-gray-50 border-gray-200"
          onClick={() => {
            setZoom(1);
            setPan({ x: 0, y: 0 });
          }}
        >
          fit
        </button>
        <button
          type="button"
          class="px-2.5 py-1 hover:bg-gray-50"
          onClick={() => setZoom((z) => Math.max(0.2, z / 1.25))}
        >
          −
        </button>
      </div>
      <Show when={groups().filter(Boolean).length > 1}>
        <div class="absolute left-3 top-3 flex flex-wrap gap-2 rounded-md border bg-white/90 px-2 py-1 text-xs shadow-sm border-gray-200">
          <For each={groups().filter(Boolean)}>
            {(g) => (
              <span class="flex items-center gap-1">
                <span
                  class="inline-block h-2.5 w-2.5 rounded-full"
                  style={{ background: groupColor(g, groups())[1] }}
                />
                {g}
              </span>
            )}
          </For>
        </div>
      </Show>
    </div>
  );
}
