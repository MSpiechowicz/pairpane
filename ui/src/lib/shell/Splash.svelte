<script lang="ts">
  import { onMount, tick } from 'svelte';

  import {
    clamp,
    DOT_IDLE_COLOR,
    DOTS,
    easeOut,
    FLIGHT,
    fileFrame,
    PANE,
    PANES,
    proximity,
    ROW,
    ROW_WIDTHS,
    rowRect,
    trailFrames,
  } from './splashFlight';
  import { browserStorage, markPlayed, prefersReducedMotion, shouldPlay } from './splashGate';

  /**
   * The startup splash (B1): two panes draw in and fill, rows are written into the left one, files glide along a
   * dotted arc into the right one, the name fades in, then one plain fade into the app. The last frame is the
   * app logo. It is a decorative overlay above the app, which loads underneath; a click or key press ends it early.
   */
  let {
    version = __APP_VERSION__,
    onEnd,
  }: {
    /** The app's version: shown below the logo, and what the splash remembers having played for. */
    version?: string;
    /** Called once the splash is gone from the DOM. */
    onEnd?: () => void;
  } = $props();

  const TAGLINE = 'One file manager · Windows · Linux · macOS';
  /** The input the splash swallows: it ends the splash and never reaches the app underneath. */
  const INPUT_EVENTS = ['keydown', 'mousedown', 'click'] as const;
  const FADE_OUT_MS = 320;
  const HOLD_MS = 650;
  /** The whole sequence takes about 5 s; past this it is cut short (see `run`). */
  const SEQUENCE_LIMIT_MS = 10_000;
  /** The dots of the arc fade in one after another. */
  const DOT_FADE_MS = 180;
  const DOT_STAGGER_MS = 60;
  const DOTS_IN_MS = DOT_FADE_MS + (DOTS.length - 1) * DOT_STAGGER_MS;

  /**
   * Decided once, before the first paint, so the app never flashes first. Starting the splash records the
   * version. Without the Web Animations API (an old webview) the splash is over before it begins.
   */
  let visible = $state(
    (() => {
      const storage = browserStorage();
      if (!shouldPlay(storage, version, prefersReducedMotion())) {
        return false;
      }

      markPlayed(storage, version);
      return typeof Element.prototype.animate === 'function';
    })(),
  );

  let root = $state<HTMLElement>();
  let word = $state<HTMLElement>();
  let outlines = $state<SVGElement[]>([]);
  let fills = $state<SVGElement[]>([]);
  /** How much of each row is written, left pane then right pane. */
  let written = $state(PANES.map(() => ROW_WIDTHS.map(() => 0)));
  /** How far each file is in its flight (0: not started or already gone, 1: landed). */
  let flights = $state(ROW_WIDTHS.map(() => 0));
  /** How visible each dot of the arc is (0 to 1). */
  let dotLevels = $state(DOTS.map(() => 0));

  /** How close a file is to each dot (the dot swells and brightens as one passes). */
  const near = $derived(DOTS.map((dot) => Math.max(0, ...flights.map((k) => (k > 0 && k < 1 ? proximity(dot, k) : 0)))));
  const flying = $derived(flights[0] > 0);

  const skip = new AbortController();
  /** Resolves as soon as the splash is skipped. */
  const skipped = new Promise<void>((resolve) => skip.signal.addEventListener('abort', () => resolve()));
  let destroyed = false;
  /** What every step returns once the splash is skipped: the sequence stops there instead of running on. */
  const halted = new Promise<never>(() => {});

  function wait(ms: number): Promise<void> {
    if (skip.signal.aborted) {
      return halted;
    }

    return new Promise((resolve) => setTimeout(resolve, ms));
  }

  /** Calls `frame` with the linear progress (0 to 1) of every animation frame for `ms`, ending on 1. */
  function tween(ms: number, frame: (progress: number) => void): Promise<void> {
    if (skip.signal.aborted) {
      return halted;
    }

    return new Promise((resolve) => {
      let start: number | undefined;
      const step = (now: number) => {
        if (skip.signal.aborted) {
          return;
        }

        start ??= now;
        const progress = clamp((now - start) / ms);
        frame(progress);
        if (progress < 1) {
          requestAnimationFrame(step);
        } else {
          resolve();
        }
      };

      requestAnimationFrame(step);
    });
  }

  /** Animates `element` with the Web Animations API, keeping the end state. */
  function animate(element: Element, keyframes: Keyframe[], options: KeyframeAnimationOptions): Promise<unknown> {
    if (skip.signal.aborted) {
      return halted;
    }

    return element.animate(keyframes, { fill: 'both', easing: 'ease-out', ...options }).finished;
  }

  const fade = (element: Element, from: number, to: number, duration: number, delay = 0) =>
    animate(element, [{ opacity: from }, { opacity: to }], { duration, delay });

  /** Writes a row left to right. */
  async function writeRow(pane: number, row: number, ms: number, delay = 0) {
    await wait(delay);
    await tween(ms, (progress) => (written[pane][row] = ROW_WIDTHS[row] * easeOut(progress)));
  }

  /** One file: it glides along the arc and sinks into the right pane, where its row is written as it lands. */
  async function send(row: number) {
    await wait(row * FLIGHT.gap);

    let landed: Promise<void> | undefined;
    await tween(FLIGHT.duration, (progress) => {
      flights[row] = progress;
      if (progress > FLIGHT.landing) {
        landed ??= writeRow(1, row, 380);
      }
    });
    await landed;
  }

  async function sequence() {
    if (!root || !word) {
      return;
    }

    // 1. The outlines draw in, then the panes fill.
    await Promise.all(
      outlines.map((outline, i) =>
        animate(outline, [{ strokeDashoffset: 1 }, { strokeDashoffset: 0 }], { duration: 520, delay: i * 90 }),
      ),
    );
    await Promise.all([...fills.map((fill) => fade(fill, 0, 1, 220)), ...outlines.map((line) => fade(line, 1, 0, 220))]);

    // 2. The left rows are written.
    await Promise.all(ROW_WIDTHS.map((_, row) => writeRow(0, row, 340, row * 110)));

    // 3. The arc's dots appear, and the files fly along it.
    await tween(DOTS_IN_MS, (progress) => {
      const elapsed = progress * DOTS_IN_MS;
      dotLevels = DOTS.map((_, i) => easeOut(clamp((elapsed - i * DOT_STAGGER_MS) / DOT_FADE_MS)));
    });
    await Promise.all(ROW_WIDTHS.map((_, row) => send(row)));

    // 4. The dots fade away, which leaves the logo, and the name and tagline fade in once.
    await Promise.all([
      tween(260, (progress) => (dotLevels = DOTS.map(() => 1 - easeOut(progress)))),
      fade(word, 0, 1, 380, 80),
    ]);
    await wait(HOLD_MS);
  }

  /**
   * Plays the sequence (or as much of it as ran before a skip), then fades into the app and removes itself. Timers
   * bound both stages: animation frames and animations can stall (a minimised window, a throttled webview), and the
   * splash holds the app's input while it is up, so it must end whatever happens.
   */
  async function run() {
    try {
      await Promise.race([sequence(), skipped, delay(SEQUENCE_LIMIT_MS)]);
    } catch {
      // A step failed: the splash must not hold the app back, so it fades out.
    }

    if (destroyed || !root) {
      return;
    }

    try {
      const fading = root.animate([{ opacity: 1 }, { opacity: 0 }], { duration: FADE_OUT_MS, easing: 'ease-out' });
      await Promise.race([fading.finished, delay(FADE_OUT_MS * 2)]);
    } catch {
      // The fade was cancelled: the splash goes all the same.
    } finally {
      stopSwallowing();
      if (!destroyed) {
        visible = false;
        await tick();
        onEnd?.();
      }
    }
  }

  /** A plain timer, unlike `wait` not stopped by a skip: it is what ends a splash that stalls. */
  const delay = (ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms));

  /** Any click or key press ends the splash and is not passed on to the app. */
  function swallow(event: Event) {
    event.preventDefault();
    event.stopPropagation();
    skip.abort();
  }

  /** From here on the app gets its input again. */
  function stopSwallowing() {
    for (const type of INPUT_EVENTS) {
      window.removeEventListener(type, swallow, { capture: true });
    }
  }

  onMount(() => {
    if (!visible) {
      return;
    }

    for (const type of INPUT_EVENTS) {
      window.addEventListener(type, swallow, { capture: true });
    }
    void run();

    return () => {
      destroyed = true;
      skip.abort();
      stopSwallowing();
    };
  });
</script>

{#if visible}
  <div class="splash" aria-hidden="true" bind:this={root}>
    <svg class="stage" viewBox="0 -34 200 134" focusable="false">
      <defs>
        <filter id="splash-glow" x="-150%" y="-150%" width="400%" height="400%">
          <feGaussianBlur stdDeviation="2.2" result="blur" />
          <feMerge>
            <feMergeNode in="blur" />
            <feMergeNode in="SourceGraphic" />
          </feMerge>
        </filter>
        <radialGradient id="splash-halo">
          <stop offset="0" stop-color="#fff" stop-opacity=".55" />
          <stop offset="1" stop-color="#fff" stop-opacity="0" />
        </radialGradient>
      </defs>

      {#each PANES as pane, side (pane.side)}
        <rect
          class="edge"
          bind:this={outlines[side]}
          x={pane.x}
          y={PANE.y}
          width={PANE.width}
          height={PANE.height}
          rx={PANE.radius}
          pathLength="1"
          fill="none"
          stroke={pane.color}
          stroke-width="2.5"
        />
        <rect
          bind:this={fills[side]}
          x={pane.x}
          y={PANE.y}
          width={PANE.width}
          height={PANE.height}
          rx={PANE.radius}
          fill={pane.color}
          opacity="0"
        />
        {#each ROW_WIDTHS as _, row (row)}
          {@const rect = rowRect(pane, row, written[side][row])}
          <rect {...rect} rx={ROW.radius} fill={pane.rowColor} fill-opacity={pane.rowOpacity} />
        {/each}
      {/each}

      {#each DOTS as dot, i (i)}
        <circle
          cx={dot.x}
          cy={dot.y}
          r={1.7 + near[i] * 1.3}
          fill={flying ? dot.color : DOT_IDLE_COLOR}
          opacity={dotLevels[i] * (0.5 + near[i] * 0.5)}
        />
      {/each}

      {#each flights as k, i (i)}
        {#if k > 0 && k < 1}
          {@const file = fileFrame(k)}
          {#each trailFrames(k) as trail, j (j)}
            <rect {...trail} rx="2" fill="#fff" filter="url(#splash-glow)" />
          {/each}
          <circle cx={file.at.x} cy={file.at.y} r="9" fill="url(#splash-halo)" opacity={file.scale * 0.9} />
          <rect
            x={file.x}
            y={file.y}
            width={file.width}
            height={file.height}
            rx="2"
            fill={file.color}
            stroke="var(--dim)"
            stroke-width="0.5"
            opacity={Math.min(1, file.scale * 1.3)}
            filter="url(#splash-glow)"
          />
        {/if}
      {/each}
    </svg>

    <div class="word" bind:this={word}>
      <strong>Pairpane</strong>
      <span>{TAGLINE}</span>
    </div>
    <div class="version">v{version}</div>
  </div>
{/if}

<style>
  .splash {
    position: fixed;
    inset: 0;
    z-index: 1000;
    background: var(--bg);
    color: var(--text);
    font-family: var(--font-ui);
    cursor: default;
  }

  .stage {
    position: absolute;
    left: 50%;
    top: 50%;
    width: 220px;
    height: 147px;
    overflow: visible;
    transform: translate(-50%, -74%);
  }

  /* The outline is drawn by animating the dash offset along the rect's normalized path length. */
  .edge {
    stroke-dasharray: 1;
    stroke-dashoffset: 1;
  }

  .word {
    position: absolute;
    left: 50%;
    top: calc(50% + 40px);
    transform: translateX(-50%);
    text-align: center;
    white-space: nowrap;
    opacity: 0;
  }

  .word strong {
    display: block;
    font-size: 30px;
    font-weight: 650;
    letter-spacing: -0.01em;
  }

  .word span {
    display: block;
    margin-top: 6px;
    font-size: 13px;
    color: var(--dim);
  }

  .version {
    position: absolute;
    bottom: 22px;
    left: 0;
    right: 0;
    text-align: center;
    font-size: 11.5px;
    color: var(--dim);
    opacity: 0.7;
  }
</style>
