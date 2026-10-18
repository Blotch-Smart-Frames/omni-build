import { afterEach, describe, expect, it, vi } from 'vitest';

import {
  type Box,
  carryOutline,
  followOutline,
  prefersReducedMotion,
  wait,
  zoomRects,
} from './xor';

const FROM: Box = { left: 0, top: 0, width: 16, height: 16 };
const TO: Box = { left: 160, top: 80, width: 320, height: 240 };

// jsdom has neither, so each test says what the browser would.
const reduceMotion = (reduce: boolean) => {
  const matchMedia = vi.fn(() => ({ matches: reduce }));
  vi.stubGlobal('matchMedia', matchMedia);
  return matchMedia;
};

const stubAnimate = () => {
  let finish = () => {};
  const finished = new Promise<void>((resolve) => {
    finish = resolve;
  });
  const animate = vi.fn(() => ({ finished }));
  Object.defineProperty(HTMLElement.prototype, 'animate', {
    value: animate,
    configurable: true,
  });
  return { animate, finish };
};

const outlines = () => [...document.body.querySelectorAll('div')];

afterEach(() => {
  document.body.replaceChildren();
  vi.useRealTimers();
  vi.unstubAllGlobals();
  Reflect.deleteProperty(HTMLElement.prototype, 'animate');
});

describe('prefersReducedMotion', () => {
  it("follows the visitor's reduced motion setting", () => {
    const matchMedia = reduceMotion(true);

    expect(prefersReducedMotion()).toBe(true);
    expect(matchMedia).toHaveBeenCalledWith('(prefers-reduced-motion: reduce)');

    reduceMotion(false);

    expect(prefersReducedMotion()).toBe(false);
  });

  it('is false without a window, as in the server render', () => {
    vi.stubGlobal('window', undefined);

    expect(prefersReducedMotion()).toBe(false);
  });
});

describe('zoomRects', () => {
  it('draws eight outlines from one box to the other, all hidden at first', () => {
    reduceMotion(false);
    vi.useFakeTimers();

    void zoomRects(FROM, TO);

    const nodes = outlines();
    expect(nodes).toHaveLength(8);
    expect(nodes.map((node) => node.style.visibility)).toEqual(
      Array(8).fill('hidden'),
    );
    // The first is an eighth of the way there; the last is the target box.
    expect(nodes[0].style).toMatchObject({
      position: 'fixed',
      left: '20px',
      top: '10px',
      width: '54px',
      height: '44px',
    });
    expect(nodes[7].style).toMatchObject({
      left: '160px',
      top: '80px',
      width: '320px',
      height: '240px',
    });
    expect(nodes[0].style.border).toBe('2px solid rgb(255, 255, 255)');
  });

  it('shows each outline in turn, removes it, and resolves when all are gone', async () => {
    reduceMotion(false);
    vi.useFakeTimers();
    let done = false;

    void zoomRects(FROM, TO, { duration: 400, count: 4 }).then(() => {
      done = true;
    });
    const nodes = outlines();
    const shown = () => nodes.map((node) => node.style.visibility);

    // Every 100 ms the next one shows, and each stays for 250 ms.
    await vi.advanceTimersByTimeAsync(0);
    expect(shown()).toEqual(['visible', 'hidden', 'hidden', 'hidden']);
    await vi.advanceTimersByTimeAsync(100);
    expect(shown()).toEqual(['visible', 'visible', 'hidden', 'hidden']);
    await vi.advanceTimersByTimeAsync(150);
    expect(outlines()).toEqual(nodes.slice(1));
    await vi.advanceTimersByTimeAsync(299);
    expect(done).toBe(false);
    await vi.advanceTimersByTimeAsync(1);
    expect(outlines()).toEqual([]);
    expect(done).toBe(true);
  });

  it('draws nothing when the visitor prefers reduced motion', async () => {
    reduceMotion(true);

    await zoomRects(FROM, TO);

    expect(outlines()).toEqual([]);
  });
});

describe('carryOutline', () => {
  it("carries a dotted outline to the target box's centre, then removes it", async () => {
    reduceMotion(false);
    const { animate, finish } = stubAnimate();

    const carried = carryOutline(FROM, TO);

    const [node] = outlines();
    expect(node.style.border).toBe('2px dotted rgb(255, 255, 255)');
    expect(animate).toHaveBeenCalledWith(
      [{ translate: '0 0' }, { translate: '312px 192px' }],
      {
        duration: 900,
        easing: 'cubic-bezier(0.45, 0, 0.2, 1)',
        fill: 'forwards',
      },
    );
    expect(animate.mock.contexts[0]).toBe(node);

    finish();
    await carried;

    expect(outlines()).toEqual([]);
  });

  it('takes its duration from the options', () => {
    reduceMotion(false);
    const { animate } = stubAnimate();

    void carryOutline(FROM, TO, { duration: 300 });

    expect(animate).toHaveBeenCalledWith(
      expect.any(Array),
      expect.objectContaining({ duration: 300 }),
    );
  });

  it('draws nothing when the visitor prefers reduced motion', async () => {
    reduceMotion(true);
    const { animate } = stubAnimate();

    await carryOutline(FROM, TO);

    expect(outlines()).toEqual([]);
    expect(animate).not.toHaveBeenCalled();
  });
});

describe('followOutline', () => {
  it('draws a dotted outline over the box that moves with the pointer', () => {
    const outline = followOutline(TO);

    const [node] = outlines();
    expect(node.style).toMatchObject({ left: '160px', top: '80px' });
    expect(node.style.border).toBe('2px dotted rgb(255, 255, 255)');

    outline.move(12, -4);

    expect(node.style.translate).toBe('12px -4px');

    outline.remove();

    expect(outlines()).toEqual([]);
  });
});

describe('wait', () => {
  it('resolves after the given time', async () => {
    vi.useFakeTimers();
    let done = false;

    void wait(500).then(() => {
      done = true;
    });

    await vi.advanceTimersByTimeAsync(499);
    expect(done).toBe(false);
    await vi.advanceTimersByTimeAsync(1);
    expect(done).toBe(true);
  });
});
