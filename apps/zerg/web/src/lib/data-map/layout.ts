// Deterministic force-directed layout (Fruchterman–Reingold with gravity) for
// the relationship map. Pure: same nodes/edges in, same coordinates out, so
// the map does not jump between renders.

export interface LayoutBox {
  id: string;
  width: number;
  height: number;
}

export interface Placed extends LayoutBox {
  x: number; // centre
  y: number;
}

export function forceLayout(
  boxes: LayoutBox[],
  edges: { from: string; to: string }[],
  iterations = 300,
): Placed[] {
  const n = boxes.length;
  if (n === 0) return [];
  const index = new Map(boxes.map((b, i) => [b.id, i]));
  const links = edges
    .map((e) => [index.get(e.from), index.get(e.to)] as const)
    .filter(
      (l): l is readonly [number, number] =>
        l[0] !== undefined && l[1] !== undefined,
    );

  // Ideal edge length: tight enough to read at fit-zoom; the overlap pass
  // below separates the cards the forces leave touching.
  const widest = Math.max(...boxes.map((b) => b.width));
  const k = Math.max(110, widest * 0.6);
  const radius = Math.max(k, (n * k) / (2 * Math.PI));
  const x = boxes.map((_, i) => radius * Math.cos((2 * Math.PI * i) / n));
  const y = boxes.map((_, i) => radius * Math.sin((2 * Math.PI * i) / n));
  const dx = new Float64Array(n);
  const dy = new Float64Array(n);

  for (let iter = 0; iter < iterations; iter++) {
    const temperature = k * 2 * (1 - iter / iterations) + 1;
    dx.fill(0);
    dy.fill(0);
    for (let i = 0; i < n; i++) {
      for (let j = i + 1; j < n; j++) {
        let ddx = x[i] - x[j];
        let ddy = y[i] - y[j];
        let dist = Math.hypot(ddx, ddy);
        if (dist < 0.01) {
          // Coincident points: nudge apart along a stable direction.
          ddx = (i - j) * 0.1;
          ddy = 0.1;
          dist = Math.hypot(ddx, ddy);
        }
        const force = (k * k) / dist;
        dx[i] += (ddx / dist) * force;
        dy[i] += (ddy / dist) * force;
        dx[j] -= (ddx / dist) * force;
        dy[j] -= (ddy / dist) * force;
      }
    }
    for (const [a, b] of links) {
      const ddx = x[a] - x[b];
      const ddy = y[a] - y[b];
      const dist = Math.max(Math.hypot(ddx, ddy), 0.01);
      const force = (dist * dist) / k;
      dx[a] -= (ddx / dist) * force;
      dy[a] -= (ddy / dist) * force;
      dx[b] += (ddx / dist) * force;
      dy[b] += (ddy / dist) * force;
    }
    for (let i = 0; i < n; i++) {
      // Gravity balances the O(n) outward repulsion on the rim. Tuned on the
      // 58-schema zerg OpenAPI doc (~1k units wide once overlaps are removed).
      dx[i] -= x[i] * 3;
      dy[i] -= y[i] * 3;
      const len = Math.hypot(dx[i], dy[i]);
      if (len > 0) {
        const step = Math.min(len, temperature);
        x[i] += (dx[i] / len) * step;
        y[i] += (dy[i] / len) * step;
      }
    }
  }
  removeOverlaps(boxes, x, y);
  return boxes.map((b, i) => ({ ...b, x: x[i], y: y[i] }));
}

/** Push overlapping cards apart along their axis of least penetration. */
function removeOverlaps(boxes: LayoutBox[], x: number[], y: number[]) {
  const gap = 14;
  for (let pass = 0; pass < 100; pass++) {
    let moved = false;
    for (let i = 0; i < boxes.length; i++) {
      for (let j = i + 1; j < boxes.length; j++) {
        const ox =
          (boxes[i].width + boxes[j].width) / 2 + gap - Math.abs(x[i] - x[j]);
        const oy =
          (boxes[i].height + boxes[j].height) / 2 + gap - Math.abs(y[i] - y[j]);
        if (ox <= 0 || oy <= 0) continue;
        moved = true;
        if (ox < oy) {
          const s = (x[i] <= x[j] ? -1 : 1) * (ox / 2);
          x[i] += s;
          x[j] -= s;
        } else {
          const s = (y[i] <= y[j] ? -1 : 1) * (oy / 2);
          y[i] += s;
          y[j] -= s;
        }
      }
    }
    if (!moved) return;
  }
}
