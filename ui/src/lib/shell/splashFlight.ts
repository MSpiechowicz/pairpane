/*
 * The geometry and motion of the startup splash, as pure functions of time so they can be tested without a
 * browser. Units are those of the splash's SVG viewBox. The final frame is the app logo
 * (src-tauri/icons/source/logo.svg): the panes and rows below are its proportions.
 */

/** The logo's fixed brand colors; they are the same in both themes. */
export const BLUE = '#5b9cff';
export const AMBER = '#f3b64a';
const GREY = '#9aa3b5';
const FILE_WHITE = '#ffffff';
const FILE_CREAM = '#ffecc8';

export interface PaneShape {
  side: 'left' | 'right';
  x: number;
  color: string;
  /** The color and opacity of the rows written into the pane. */
  rowColor: string;
  rowOpacity: number;
}

export interface Rect {
  x: number;
  y: number;
  width: number;
  height: number;
}

export const PANES: readonly PaneShape[] = [
  { side: 'left', x: 30, color: BLUE, rowColor: '#ffffff', rowOpacity: 0.9 },
  { side: 'right', x: 106, color: AMBER, rowColor: '#1d1606', rowOpacity: 0.55 },
];

export const PANE = { y: 10, width: 64, height: 80, radius: 11 } as const;
/** The rows' full widths (each right row is as long as the left one it copies), and where and how tall they are. */
export const ROW_WIDTHS: readonly number[] = [44, 34, 40, 26];
export const ROW = { inset: 10, top: 16, pitch: 13, height: 5, radius: 2.5 } as const;

/** The row `index` of a pane, `width` long (its full width by default). */
export function rowRect(pane: PaneShape, index: number, width = ROW_WIDTHS[index]): Rect {
  return { x: pane.x + ROW.inset, y: PANE.y + ROW.top + index * ROW.pitch, width, height: ROW.height };
}

interface Point {
  x: number;
  y: number;
}

/** The arc the files glide along: a quadratic Bézier from above the left pane to above the right one. */
const START: Point = { x: 66, y: 6 };
const CONTROL: Point = { x: 100, y: -42 };
const END: Point = { x: 134, y: 6 };

export function bezier(t: number): Point {
  const a = (1 - t) ** 2;
  const b = 2 * (1 - t) * t;
  const c = t * t;

  return { x: a * START.x + b * CONTROL.x + c * END.x, y: a * START.y + b * CONTROL.y + c * END.y };
}

export const clamp = (t: number): number => Math.max(0, Math.min(1, t));
const lerp = (a: number, b: number, t: number): number => a + (b - a) * t;

/** Slow-fast-slow: how a file glides along the arc. */
export const easeInOut = (t: number): number => (t < 0.5 ? 4 * t * t * t : 1 - (-2 * t + 2) ** 3 / 2);

/** Fast then settling: how things are written and fade in. */
export const easeOut = (t: number): number => 1 - (1 - t) ** 3;

/** The color `t` of the way from `from` to `to` (both `#rrggbb`), as a CSS color. */
export function mixColor(from: string, to: string, t: number): string {
  const channel = (hex: string, i: number) => parseInt(hex.slice(1 + i * 2, 3 + i * 2), 16);
  const mixed = [0, 1, 2].map((i) => Math.round(lerp(channel(from, i), channel(to, i), t)));

  return `rgb(${mixed.join(',')})`;
}

/** The dotted arc: where its dots sit along the Bézier, and the color each takes once files fly (blue to amber). */
export const DOTS: readonly (Point & { color: string })[] = [0.14, 0.3, 0.5, 0.7, 0.86].map((t) => {
  const point = bezier(t);

  return { ...point, color: mixColor(BLUE, AMBER, (point.x - START.x) / (END.x - START.x)) };
});
/** The dots' color before any file flew. */
export const DOT_IDLE_COLOR = GREY;

/** How the files travel: one every `gap` ms, each taking `duration` ms and landing (its row starts) at `landing`. */
export const FLIGHT = { duration: 1000, gap: 320, landing: 0.82 } as const;

const APPEAR = 0.15;
const SINK = 0.85;
const FILE = { width: 9, height: 11 } as const;
const TRAIL = { count: 3, lag: 0.064, from: 0.12, until: 0.9 } as const;

export interface FileFrame extends Rect {
  /** The glide's center; the halo follows it. */
  at: Point;
  /** 0 to 1: the file grows in at the start of its flight and shrinks as it sinks into the pane. */
  scale: number;
  /** The file turns from white to cream on its way. */
  color: string;
}

/** The file at `k` (0 to 1) of its flight: it appears above the left pane, glides along the arc and sinks. */
export function fileFrame(k: number): FileFrame {
  const at = bezier(easeInOut(k));
  const scale = k < APPEAR ? easeOut(k / APPEAR) : k > SINK ? 1 - easeOut((k - SINK) / (1 - SINK)) : 1;
  const sink = k > SINK ? ((k - SINK) / (1 - SINK)) * 4 : 0;
  const width = FILE.width * scale;
  const height = FILE.height * scale;

  return {
    at,
    scale,
    width,
    height,
    x: at.x - width / 2,
    y: at.y - height / 2 + sink,
    color: mixColor(FILE_WHITE, FILE_CREAM, k),
  };
}

/** The soft trail: fainter, smaller copies of where the file just was. Empty at the very start and end. */
export function trailFrames(k: number): (Rect & { opacity: number })[] {
  if (k < TRAIL.from || k > TRAIL.until) {
    return [];
  }

  const frames: (Rect & { opacity: number })[] = [];
  for (let i = 0; i < TRAIL.count; i++) {
    const past = k - (i + 1) * TRAIL.lag;
    if (past < 0) {
      continue;
    }

    const file = fileFrame(past);
    frames.push({
      x: file.x + file.width * 0.2,
      y: file.y + file.height * 0.2,
      width: file.width * 0.6,
      height: file.height * 0.6,
      opacity: 0.32 - i * 0.1,
    });
  }

  return frames;
}

/** How close the file at `k` is to a dot: 0 (far) to 1 (on it); the dot swells and brightens as a file passes. */
export function proximity(dot: Point, k: number): number {
  const file = fileFrame(k);

  return Math.max(0, 1 - Math.hypot(file.at.x - dot.x, file.at.y - dot.y) / 10) * file.scale;
}
