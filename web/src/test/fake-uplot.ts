// Stand-in for uPlot in jsdom tests (jsdom has no canvas or matchMedia, which uPlot needs at
// import). Use with `vi.mock('uplot', () => import('../test/fake-uplot'))`; `plots` records every
// instance so tests can check what the component handed over.

import { vi } from 'vitest';

export interface FakePlot {
  opts: unknown;
  data: unknown[];
  destroyed: boolean;
  setData: ReturnType<typeof vi.fn>;
}

export const plots: FakePlot[] = [];

export default class FakeUPlot {
  static pxRatio = 1;
  over = { offsetLeft: 0, offsetTop: 0, clientWidth: 300 };
  height = 200;
  setData = vi.fn();
  setSize = vi.fn();
  readonly #record: FakePlot;

  constructor(opts: unknown, data: unknown[]) {
    this.#record = { opts, data, destroyed: false, setData: this.setData };
    plots.push(this.#record);
  }

  destroy() {
    this.#record.destroyed = true;
  }
}

/** jsdom lacks ResizeObserver; charts only need it to exist. */
export function installResizeObserver(): void {
  globalThis.ResizeObserver ??= class {
    observe() {}
    disconnect() {}
    unobserve() {}
  };
}
