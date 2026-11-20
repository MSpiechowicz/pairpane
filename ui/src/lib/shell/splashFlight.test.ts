import { describe, expect, it } from 'vitest';

import logo from '../../../../src-tauri/icons/source/logo.svg?raw';
import { bezier, DOTS, fileFrame, PANE, PANES, rowRect, trailFrames } from './splashFlight';

describe('the final frame', () => {
  const rects = [...logo.matchAll(/<rect ([^>]*)\/>/g)].map(([, attributes]) =>
    Object.fromEntries([...attributes.matchAll(/([\w-]+)="([^"]*)"/g)].map(([, name, value]) => [name, value])),
  );
  // The logo's tile comes first, then each pane followed by its four rows.
  const [, ...logoShapes] = rects;
  const logoPanes = [logoShapes.slice(0, 5), logoShapes.slice(5, 10)];

  it('has the logo`s panes: the same aspect, colors and roundness', () => {
    for (const [side, [pane]] of logoPanes.entries()) {
      expect(PANE.width / PANE.height).toBeCloseTo(Number(pane.width) / Number(pane.height), 5);
      expect(PANE.radius / PANE.height).toBeCloseTo(Number(pane.rx) / Number(pane.height), 5);
      expect(PANES[side].color).toBe(pane.fill);
    }
  });

  it('has the logo`s rows: the same widths, positions, heights and colors, relative to their pane', () => {
    for (const [side, [pane, ...rows]] of logoPanes.entries()) {
      for (const [i, row] of rows.entries()) {
        const mine = rowRect(PANES[side], i);

        expect(mine.width / PANE.width).toBeCloseTo(Number(row.width) / Number(pane.width), 3);
        expect((mine.x - PANES[side].x) / PANE.width).toBeCloseTo((Number(row.x) - Number(pane.x)) / Number(pane.width), 3);
        expect((mine.y - PANE.y) / PANE.height).toBeCloseTo((Number(row.y) - Number(pane.y)) / Number(pane.height), 3);
        expect(mine.height / PANE.height).toBeCloseTo(Number(row.height) / Number(pane.height), 3);
        expect(PANES[side].rowColor).toBe(row.fill);
        expect(PANES[side].rowOpacity).toBeCloseTo(Number(row['fill-opacity']), 5);
      }
    }
  });
});

describe('the flight of a file', () => {
  it('appears above the left pane and sinks into the right one', () => {
    const start = fileFrame(0);
    const end = fileFrame(1);

    expect(start.scale).toBe(0);
    expect(start.at.x).toBeLessThan(PANES[1].x);
    expect(end.scale).toBeCloseTo(0, 5);
    expect(end.at.x).toBeGreaterThan(PANES[1].x);
  });

  it('glides above both panes at full size in between', () => {
    const middle = fileFrame(0.5);

    expect(middle.scale).toBe(1);
    expect(middle.at.y).toBeLessThan(PANE.y);
    expect(bezier(0.5).x).toBeCloseTo(100, 5);
  });

  it('has a trail only while it glides', () => {
    expect(trailFrames(0.02)).toEqual([]);
    expect(trailFrames(0.97)).toEqual([]);
    expect(trailFrames(0.6).length).toBeGreaterThan(0);
  });

  it('has the dots of the arc above the panes, between where the files start and land', () => {
    for (const dot of DOTS) {
      expect(dot.y).toBeLessThan(PANE.y);
      expect(dot.x).toBeGreaterThan(PANES[0].x);
      expect(dot.x).toBeLessThan(PANES[1].x + PANE.width);
    }
  });
});

