import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
} from '@testing-library/react';
import { useLayoutEffect, type ReactNode } from 'react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import PixelTransition, { type PixelTransitionProps } from './pixel-transition';

// jsdom has no 2D canvas, no Path2D, no ResizeObserver and no layout, so the
// test stands in for each. The fakes record what the transition draws, one
// list of fills per clearRect, and are put back after every test.

type Op = [string, ...number[]];

interface Fill {
  alpha: number;
  style: string;
  ops: Op[];
}

class FakePath {
  ops: Op[] = [];
  roundRect?: (...args: number[]) => void = (...args) => {
    this.ops.push(['roundRect', ...args]);
  };
  rect(...args: number[]) {
    this.ops.push(['rect', ...args]);
  }
  arc(...args: number[]) {
    this.ops.push(['arc', ...args]);
  }
  moveTo(...args: number[]) {
    this.ops.push(['moveTo', ...args]);
  }
}

// Browsers from before 2023 have no roundRect.
class PathWithoutRoundRect extends FakePath {
  roundRect = undefined;
}

// What a colour probe reads back from a pixel filled with `style`, which is
// the computed colour, rgb() or rgba().
const pixelOf = (style: string) => {
  const [r, g, b, a = 1] = (style.match(/[\d.]+/g) ?? []).map(Number);
  return new Uint8ClampedArray([r, g, b, a * 255]);
};

let draws: Fill[][] = [];

const fakeCanvas = ({ context = true, probe = true } = {}) => {
  const main = {
    globalAlpha: 1,
    fillStyle: '',
    clearRect: () => {
      draws.push([]);
    },
    fill(path: FakePath) {
      draws.at(-1)?.push({
        alpha: main.globalAlpha,
        style: main.fillStyle,
        ops: path.ops,
      });
    },
  };
  const colourProbe = {
    fillStyle: '',
    clearRect: () => {},
    fillRect: () => {},
    getImageData: () => ({ data: pixelOf(colourProbe.fillStyle) }),
  };
  // The card asks for its colour probe, a 1×1 canvas it reads back, with
  // willReadFrequently, and for the canvas it draws on without.
  vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockImplementation(((
    _type: string,
    options?: CanvasRenderingContext2DSettings,
  ) => {
    if (options?.willReadFrequently) return probe ? colourProbe : null;
    return context ? main : null;
  }) as unknown as HTMLCanvasElement['getContext']);
  return main;
};

// Animation frames run only when a test steps them, with the timestamp it
// chooses.
let pending = new Map<number, FrameRequestCallback>();
let lastFrameId = 0;
const requestFrame = vi.fn((callback: FrameRequestCallback) => {
  lastFrameId += 1;
  pending.set(lastFrameId, callback);
  return lastFrameId;
});

const step = (count = 1, now = 0) => {
  for (let i = 0; i < count; i += 1) {
    const due = [...pending.values()];
    pending = new Map();
    due.forEach((callback) => callback(now));
  }
};

const resizeObservers: { callback: () => void; disconnect: () => void }[] = [];

class FakeResizeObserver {
  callback: () => void;
  disconnect = vi.fn();
  observe = vi.fn();
  constructor(callback: () => void) {
    this.callback = callback;
    resizeObservers.push(this);
  }
}

// A step of 4/60 s makes every frame stamped 0 an exact eighth of the
// transition. With no stamp before it, the card counts a frame as a 60th of
// a second, and 1/60 over 2 × 4/60 is exactly 1/8 in floating point, so the
// fourth frame lands exactly halfway, where the cover is sealed.
const EIGHTH = (1 / 60) * 4;

// A 100px square card in a 4×4 grid has 25px cells, so a whole cell drawn at
// (x, y) is the cell at row y/25, column x/25.
const cellsIn = (fills: Fill[] = draws.at(-1) ?? []) =>
  fills
    .flatMap((fill) => fill.ops)
    .filter(([op]) => op === 'rect')
    .map(([, x, y]) => `${y / 25},${x / 25}`)
    .sort();

const ALL_CELLS = Array.from({ length: 16 }, (_, i) =>
  [Math.floor(i / 4), i % 4].join(),
).sort();

const opsIn = (name: string, fills: Fill[] = draws.at(-1) ?? []) =>
  fills.flatMap((fill) => fill.ops).filter(([op]) => op === name);

const card = () =>
  screen.getByText('Shell').closest<HTMLElement>('.pixelated-image-card')!;

const pointer = (
  event: 'pointerEnter' | 'pointerLeave' | 'pointerDown',
  { x = 50, y = 50, type = 'mouse' } = {},
) => fireEvent[event](card(), { pointerType: type, clientX: x, clientY: y });

// jsdom guesses :focus-visible from the events it has seen, which carry over
// from one test to the next, so each focus test says whether the focus came
// from the keyboard.
const focusVisible = (visible: boolean) => {
  const { matches } = Element.prototype;
  return vi.spyOn(Element.prototype, 'matches').mockImplementation(function (
    this: Element,
    selector: string,
  ) {
    return selector === ':focus-visible'
      ? visible
      : matches.call(this, selector);
  });
};

const renderCard = (props: Partial<PixelTransitionProps> = {}) =>
  render(
    <PixelTransition
      firstContent={<p>Shell</p>}
      secondContent={<p>App</p>}
      gridSize={4}
      pixelColor="#000"
      animationStepDuration={EIGHTH}
      {...props}
    />,
  );

beforeEach(() => {
  draws = [];
  pending = new Map();
  resizeObservers.length = 0;
  vi.stubGlobal('requestAnimationFrame', requestFrame);
  vi.stubGlobal(
    'cancelAnimationFrame',
    vi.fn((id: number) => pending.delete(id)),
  );
  vi.stubGlobal('ResizeObserver', FakeResizeObserver);
  vi.stubGlobal('Path2D', FakePath);
  vi.spyOn(Element.prototype, 'clientWidth', 'get').mockReturnValue(100);
  vi.spyOn(Element.prototype, 'clientHeight', 'get').mockReturnValue(100);
  vi.spyOn(Element.prototype, 'getBoundingClientRect').mockReturnValue(
    DOMRect.fromRect({ x: 0, y: 0, width: 100, height: 100 }),
  );
  // The seed of the random pattern, so each test draws the same cells.
  vi.spyOn(Math, 'random').mockReturnValue(0.5);
});

afterEach(() => {
  // Unmounting cancels frames and disconnects the observer, so it has to
  // happen before their fakes go.
  cleanup();
  requestFrame.mockClear();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

describe('the card', () => {
  it('shows the first content over a canvas, as a square', () => {
    fakeCanvas();
    render(
      <PixelTransition
        firstContent={<p>Shell</p>}
        secondContent={<p>App</p>}
      />,
    );

    const root = card();
    expect(root).toHaveAttribute('class', 'pixelated-image-card');
    expect(root).toHaveAttribute('tabindex', '0');
    expect(root).not.toHaveAttribute('role');
    expect(root.firstElementChild).toHaveStyle({ paddingTop: '100%' });
    expect(screen.getByText('Shell')).toBeVisible();
    expect(screen.getByText('App')).not.toBeVisible();
    const canvas = root.querySelector('canvas')!;
    expect(canvas).toHaveAttribute('aria-hidden', 'true');
    // Sized to the card on mount, and drawn empty.
    expect([canvas.width, canvas.height]).toEqual([100, 100]);
    expect(draws).toEqual([[]]);
  });

  it('takes a class, a style and an aspect ratio', () => {
    fakeCanvas();
    renderCard({
      className: 'phone',
      style: { color: 'red' },
      aspectRatio: '9 / 16',
    });

    const root = card();
    expect(root).toHaveAttribute('class', 'pixelated-image-card phone');
    expect(root.style.aspectRatio).toBe('9 / 16');
    expect(root).toHaveStyle({ color: 'rgb(255, 0, 0)' });
    // An aspect ratio needs no padding to hold the card's height.
    expect(root.children).toHaveLength(3);
  });

  it('holds a percentage ratio open with padding instead', () => {
    fakeCanvas();
    renderCard({ aspectRatio: ' 75% ', style: { color: 'red' } });

    const root = card();
    expect(root.style.aspectRatio).toBe('');
    expect(root.firstElementChild).toHaveStyle({ paddingTop: '75%' });
  });

  it('starts on the second content when it is active from the start', () => {
    fakeCanvas();
    renderCard({ active: true });

    expect(screen.getByText('Shell')).not.toBeVisible();
    expect(screen.getByText('App')).toBeVisible();
  });

  it('only shows the first content where there is no 2D canvas', () => {
    fakeCanvas({ context: false });
    const onActiveChange = vi.fn();
    renderCard({ onActiveChange });

    pointer('pointerEnter');
    pointer('pointerDown', { type: 'touch' });
    fireEvent.click(card());

    expect(onActiveChange).not.toHaveBeenCalled();
    expect(requestFrame).not.toHaveBeenCalled();
    expect(screen.getByText('Shell')).toBeVisible();
  });

  it('follows the canvas to the size of the card, at up to twice its pixels', () => {
    fakeCanvas();
    renderCard();
    const canvas = card().querySelector('canvas')!;
    const [observer] = resizeObservers;

    vi.spyOn(Element.prototype, 'clientWidth', 'get').mockReturnValue(50);
    vi.stubGlobal('devicePixelRatio', 3);
    observer.callback();

    expect([canvas.width, canvas.height]).toEqual([100, 200]);
    expect(draws).toEqual([[], []]);

    // A browser that reports no ratio gets one pixel per CSS pixel.
    vi.stubGlobal('devicePixelRatio', 0);
    observer.callback();

    expect([canvas.width, canvas.height]).toEqual([50, 100]);
  });
});

describe('on hover', () => {
  it('dissolves into the second content and back', () => {
    fakeCanvas();
    const onActiveChange = vi.fn();
    renderCard({ onActiveChange });

    pointer('pointerEnter');

    expect(onActiveChange).toHaveBeenLastCalledWith(true);
    step(3);
    // Covering: the first content stays until the cover is complete.
    expect(screen.getByText('Shell')).toBeVisible();
    expect(cellsIn().length).toBeGreaterThan(0);
    expect(draws.at(-1)?.[0]).toMatchObject({
      alpha: 1,
      style: 'rgba(0, 0, 0, 1)',
    });
    step();
    // Halfway, every cell is drawn and the second content is under them.
    expect(screen.getByText('Shell')).not.toBeVisible();
    expect(screen.getByText('App')).toBeVisible();
    expect(cellsIn()).toEqual(ALL_CELLS);
    step(4);
    expect(draws.at(-1)).toEqual([]);
    expect(pending.size).toBe(0);

    pointer('pointerLeave');

    expect(onActiveChange).toHaveBeenLastCalledWith(false);
    step(8);
    expect(screen.getByText('Shell')).toBeVisible();
    expect(screen.getByText('App')).not.toBeVisible();
    expect(pending.size).toBe(0);
  });

  it('turns back when the pointer leaves before the cover is complete', () => {
    fakeCanvas();
    renderCard();

    pointer('pointerEnter');
    step(2);
    const covered = cellsIn();
    pointer('pointerLeave');
    step();

    // The cover recedes the way it came, without ever showing the second
    // content.
    expect(covered.length).toBeGreaterThan(cellsIn().length);
    expect(cellsIn().every((cell) => covered.includes(cell))).toBe(true);
    step();
    expect(draws.at(-1)).toEqual([]);
    expect(pending.size).toBe(0);
    expect(screen.getByText('Shell')).toBeVisible();
  });

  it('draws nothing when the pointer leaves before the first frame', () => {
    fakeCanvas();
    renderCard();

    pointer('pointerEnter');
    pointer('pointerLeave');
    step();

    expect(draws.at(-1)).toEqual([]);
    expect(pending.size).toBe(0);
    expect(screen.getByText('Shell')).toBeVisible();
  });

  it('leaves touch to taps, which toggle it', () => {
    fakeCanvas();
    const onActiveChange = vi.fn();
    renderCard({ onActiveChange });

    // A touch that starts or ends over the card isn't a hover.
    pointer('pointerEnter', { type: 'touch' });
    pointer('pointerLeave', { type: 'touch' });
    // A click with a mouse comes after a hover that already played it.
    pointer('pointerDown');
    fireEvent.click(card());

    expect(onActiveChange).not.toHaveBeenCalled();

    pointer('pointerDown', { type: 'touch' });
    fireEvent.click(card());

    expect(onActiveChange).toHaveBeenLastCalledWith(true);
    step(8);
    expect(screen.getByText('App')).toBeVisible();

    fireEvent.click(card());

    expect(onActiveChange).toHaveBeenLastCalledWith(false);
    step(8);
    expect(screen.getByText('Shell')).toBeVisible();
  });

  it('plays on keyboard focus and turns back on blur', () => {
    fakeCanvas();
    const onActiveChange = vi.fn();
    renderCard({ onActiveChange, pattern: 'wipe', randomness: 0 });
    focusVisible(true);

    act(() => card().focus());

    expect(onActiveChange).toHaveBeenLastCalledWith(true);
    // With no pointer to come from, the wipe starts at the left edge.
    step();
    expect(cellsIn()).toEqual(['0,0', '1,0', '2,0', '3,0']);

    act(() => card().blur());

    expect(onActiveChange).toHaveBeenLastCalledWith(false);
  });

  it('stays while either the pointer or the keyboard focus is on it', () => {
    fakeCanvas();
    const onActiveChange = vi.fn();
    renderCard({ onActiveChange });
    focusVisible(true);

    pointer('pointerEnter');
    act(() => card().focus());
    act(() => card().blur());

    expect(onActiveChange.mock.calls).toEqual([[true], [true]]);

    act(() => card().focus());
    pointer('pointerLeave');

    expect(onActiveChange.mock.calls).toEqual([[true], [true], [true]]);
  });

  it('ignores focus from a click, and focus moving within the card', () => {
    fakeCanvas();
    const onActiveChange = vi.fn();
    render(
      <PixelTransition
        firstContent={<button type="button">Shell</button>}
        secondContent={<p>App</p>}
        onActiveChange={onActiveChange}
      />,
    );
    const inside = screen.getByRole('button', { name: 'Shell' });
    const root = inside.closest<HTMLElement>('.pixelated-image-card')!;

    // Focus a click gives isn't :focus-visible, and the hover has it.
    const fromClick = focusVisible(false);
    act(() => root.focus());
    act(() => root.blur());
    fromClick.mockRestore();

    expect(onActiveChange).not.toHaveBeenCalled();

    focusVisible(true);

    act(() => inside.focus());
    act(() => root.focus());
    act(() => inside.focus());

    expect(onActiveChange.mock.calls).toEqual([[true]]);

    act(() => inside.blur());

    expect(onActiveChange.mock.calls).toEqual([[true], [false]]);
  });
});

describe('on click', () => {
  it('is a button that toggles on a click, Enter or Space', () => {
    fakeCanvas();
    const onActiveChange = vi.fn();
    renderCard({ trigger: 'click', onActiveChange });
    const root = screen.getByRole('button');

    // Neither the pointer nor the focus passing over it plays it.
    pointer('pointerEnter');
    pointer('pointerLeave');
    act(() => root.focus());
    act(() => root.blur());

    expect(onActiveChange).not.toHaveBeenCalled();

    pointer('pointerDown');
    fireEvent.click(root);

    expect(onActiveChange).toHaveBeenLastCalledWith(true);
    step(8);
    expect(screen.getByText('App')).toBeVisible();

    // Each key that presses a button toggles it, and keeps the page from
    // scrolling on Space.
    expect(fireEvent.keyDown(root, { key: 'Enter' })).toBe(false);
    expect(onActiveChange).toHaveBeenLastCalledWith(false);
    expect(fireEvent.keyDown(root, { key: ' ' })).toBe(false);
    expect(onActiveChange).toHaveBeenLastCalledWith(true);

    // Other keys, and keys pressed in the content, are left alone.
    expect(fireEvent.keyDown(root, { key: 'a' })).toBe(true);
    expect(fireEvent.keyDown(screen.getByText('Shell'), { key: 'Enter' })).toBe(
      true,
    );
    expect(onActiveChange).toHaveBeenCalledTimes(3);
  });
});

describe('controlled by `active`', () => {
  it('reports the pointer, and plays from where it came in when told to', () => {
    fakeCanvas();
    const onActiveChange = vi.fn();
    const props = { onActiveChange, pattern: 'wipe', randomness: 0 } as const;
    const { rerender } = renderCard({ ...props, active: false });
    const again = (active: boolean) =>
      rerender(
        <PixelTransition
          firstContent={<p>Shell</p>}
          secondContent={<p>App</p>}
          gridSize={4}
          pixelColor="#000"
          animationStepDuration={EIGHTH}
          {...props}
          active={active}
        />,
      );

    pointer('pointerEnter', { x: 98 });

    // The parent decides; the card only reports.
    expect(onActiveChange).toHaveBeenLastCalledWith(true);
    expect(requestFrame).not.toHaveBeenCalled();

    again(true);
    step();

    // In from the right edge, so the wipe starts there.
    expect(cellsIn()).toEqual(['0,3', '1,3', '2,3', '3,3']);

    again(false);
    step(2);

    expect(screen.getByText('Shell')).toBeVisible();
    expect(pending.size).toBe(0);

    // Told again with no pointer since, it starts from the left.
    again(true);
    step();

    expect(cellsIn()).toEqual(['0,0', '1,0', '2,0', '3,0']);
  });

  it('stays on the second content once played, with `once`', () => {
    fakeCanvas();
    const onActiveChange = vi.fn();
    renderCard({ once: true, onActiveChange });

    // Leaving before it ever played still reports, as there's nothing to
    // keep.
    pointer('pointerLeave');
    expect(onActiveChange).toHaveBeenLastCalledWith(false);

    pointer('pointerEnter');
    step(8);
    pointer('pointerLeave');

    expect(onActiveChange).toHaveBeenLastCalledWith(true);
    expect(pending.size).toBe(0);
    expect(screen.getByText('App')).toBeVisible();
  });

  it('ignores being told to turn back, with `once`', () => {
    fakeCanvas();
    const { rerender } = renderCard({ once: true, active: true });

    rerender(
      <PixelTransition
        firstContent={<p>Shell</p>}
        secondContent={<p>App</p>}
        once
        active={false}
      />,
    );

    expect(requestFrame).not.toHaveBeenCalled();
    expect(screen.getByText('App')).toBeVisible();
  });
});

describe('the cover', () => {
  it('uncovers the cells in the order it covered them, by default', () => {
    fakeCanvas();
    renderCard();

    pointer('pointerEnter');
    step();
    const first = cellsIn();
    step(4);
    const later = cellsIn();

    // A quarter of the way in, a random part is covered; a quarter of the
    // way out, the rest is still covered.
    expect(first.length).toBeGreaterThan(0);
    expect(first.length).toBeLessThan(16);
    expect([...first, ...later].sort()).toEqual(ALL_CELLS);
    // Whole cells, on whole pixels.
    opsIn('rect').forEach(([, x, y, width, height]) => {
      expect([x % 25, y % 25, width, height]).toEqual([0, 0, 25, 25]);
    });
  });

  it('covers in Bayer order with the dither pattern', () => {
    fakeCanvas();
    renderCard({ pattern: 'dither' });

    pointer('pointerEnter');
    step();

    // A quarter of the way, the cells whose Bayer thresholds are below 4 of
    // 16, and only those.
    expect(cellsIn()).toEqual(['0,0', '0,2', '2,0', '2,2']);
  });

  it('ripples out from where the pointer came in', () => {
    fakeCanvas();
    renderCard({ pattern: 'ripple', randomness: 0 });

    pointer('pointerEnter', { x: 0, y: 0 });
    step();

    expect(cellsIn()).toEqual(['0,0']);
  });

  it.each([
    ['left', { x: 2 }, ['0,0', '1,0', '2,0', '3,0']],
    ['right', { x: 98 }, ['0,3', '1,3', '2,3', '3,3']],
    ['top', { y: 2 }, ['0,0', '0,1', '0,2', '0,3']],
    ['bottom', { y: 98 }, ['3,0', '3,1', '3,2', '3,3']],
  ])(
    'wipes in from the %s edge when the pointer comes in there',
    (_edge, at, cells) => {
      fakeCanvas();
      renderCard({ pattern: 'wipe', randomness: 0 });

      pointer('pointerEnter', at);
      step();

      expect(cellsIn()).toEqual(cells);
    },
  );

  it('grows each cell from its centre, leaving out cells smaller than the gap', () => {
    fakeCanvas();
    renderCard({ pixelAnimation: 'grow', gridSize: 8, gap: 4 });

    pointer('pointerEnter');
    step(2);

    const squares = opsIn('rect');
    expect(squares.length).toBeGreaterThan(0);
    squares.forEach(([, x, y, width, height]) => {
      expect(width).toBe(height);
      expect(width).toBeLessThan(12.5);
      // Centred on a cell's centre.
      expect((x + width / 2 - 6.25) % 12.5).toBeCloseTo(0);
      expect((y + height / 2 - 6.25) % 12.5).toBeCloseTo(0);
    });
  });

  it('fades each cell in and out through sixteen levels', () => {
    const context = fakeCanvas();
    renderCard({ pixelAnimation: 'fade' });
    const alphas = () => (draws.at(-1) ?? []).map((fill) => fill.alpha);

    pointer('pointerEnter');
    // A quarter of the way in, then a quarter of the way out.
    for (const frames of [1, 4]) {
      step(frames);

      expect(alphas().some((alpha) => alpha < 1)).toBe(true);
      alphas().forEach((alpha) => {
        expect(Number.isInteger(alpha * 16)).toBe(true);
      });
      expect(context.globalAlpha).toBe(1);
    }
  });

  it('draws circles that swell to seal the cover', () => {
    fakeCanvas();
    renderCard({ pixelShape: 'circle' });

    pointer('pointerEnter');
    step();

    expect(opsIn('arc')[0]).toEqual(['arc', 12.5, 12.5, 12.5, 0, Math.PI * 2]);
    expect(opsIn('moveTo')[0]).toEqual(['moveTo', 25, 12.5]);

    step(3);

    expect(opsIn('arc')[0][3]).toBeCloseTo(12.5 * 1.45);
  });

  it('rounds the cells, squaring them where the cover seals', () => {
    fakeCanvas();
    renderCard({ pixelShape: 'rounded' });

    pointer('pointerEnter');
    step();

    expect(opsIn('roundRect')[0]).toEqual(['roundRect', 0, 0, 25, 25, 7.5]);

    step(3);

    expect(opsIn('roundRect')).toEqual([]);
    expect(cellsIn()).toEqual(ALL_CELLS);
  });

  it('draws rounded cells square where the browser has no roundRect', () => {
    fakeCanvas();
    vi.stubGlobal('Path2D', PathWithoutRoundRect);
    renderCard({ pixelShape: 'rounded' });

    pointer('pointerEnter');
    step();

    expect(opsIn('roundRect')).toEqual([]);
    expect(cellsIn().length).toBeGreaterThan(0);
  });

  it('separates the cells by the gap until the cover seals', () => {
    fakeCanvas();
    renderCard({ gap: 4 });

    pointer('pointerEnter');
    step();

    expect(opsIn('rect')[0]).toEqual(['rect', 2, 2, 21, 21]);

    step(3);

    expect(cellsIn()).toEqual(ALL_CELLS);
  });

  it.each([
    ['black', '#000', [0, 32, 64, 96, 128]],
    ['white', '#fff', [255, 223, 191, 159, 128]],
  ])(
    'mixes %s cells toward the other end for a mosaic',
    (_name, pixelColor, levels) => {
      fakeCanvas();
      renderCard({ pixelColor, mosaic: 1 });

      pointer('pointerEnter');
      step(4);

      const styles = new Set((draws.at(-1) ?? []).map((fill) => fill.style));
      const tones = levels.map((v) => `rgba(${v}, ${v}, ${v}, 1)`);
      expect(styles.size).toBeGreaterThan(1);
      styles.forEach((style) => expect(tones).toContain(style));
    },
  );

  it('takes a new colour mid-transition', () => {
    fakeCanvas();
    const { rerender } = renderCard();

    pointer('pointerEnter');
    step();
    rerender(
      <PixelTransition
        firstContent={<p>Shell</p>}
        secondContent={<p>App</p>}
        gridSize={4}
        pixelColor="#fff"
        animationStepDuration={EIGHTH}
      />,
    );
    step();

    expect(draws.at(-1)?.[0].style).toBe('rgba(255, 255, 255, 1)');
  });

  it('draws white where it cannot read its colour back', () => {
    fakeCanvas({ probe: false });
    renderCard({ pixelColor: '#000' });

    pointer('pointerEnter');
    step();

    expect(draws.at(-1)?.[0].style).toBe('rgba(255, 255, 255, 1)');
  });

  it('draws a mosaic tone it has no colour for in the first tone', () => {
    fakeCanvas();
    const { rerender } = renderCard({ mosaic: 1 });
    pointer('pointerEnter');
    step();
    rerender(
      <PixelTransition
        firstContent={<p>Shell</p>}
        secondContent={<p>App</p>}
        gridSize={4}
        pixelColor="#fff"
        mosaic={1}
        animationStepDuration={EIGHTH}
      />,
    );

    // The card works out a colour for each of its five tones, so every tone
    // it picks has one. Only a list of tones cut short, which is what this
    // Array.from makes of it, leaves a tone without, and then the first
    // stands in.
    vi.spyOn(Array, 'from').mockImplementationOnce(() => ['rgba(1, 2, 3, 1)']);
    step(3);

    const styles = new Set((draws.at(-1) ?? []).map((fill) => fill.style));
    expect([...styles]).toEqual(['rgba(1, 2, 3, 1)']);
  });

  it('falls back to its defaults for a pattern, animation or shape it does not know', () => {
    fakeCanvas();
    const { unmount } = renderCard();
    pointer('pointerEnter');
    step(2);
    const defaults = draws.at(-1);
    unmount();

    renderCard({
      pattern: 'spiral' as never,
      pixelAnimation: 'spin' as never,
      pixelShape: 'star' as never,
    });
    pointer('pointerEnter');
    step(2);

    expect(draws.at(-1)).toEqual(defaults);
  });
});

describe('timing', () => {
  it('draws at most `fps` frames a second', () => {
    fakeCanvas();
    renderCard({ fps: 10, animationStepDuration: 0.4 });

    pointer('pointerEnter');
    step(1, 1000);
    const drawn = draws.length;
    step(1, 1016);

    expect(draws).toHaveLength(drawn);

    // A late frame counts a tenth of a second at most.
    step(1, 2000);

    expect(draws).toHaveLength(drawn + 1);
  });

  it('jumps straight to the second content for reduced motion', () => {
    fakeCanvas();
    vi.stubGlobal('matchMedia', () => ({ matches: true }));
    renderCard();

    pointer('pointerEnter');
    step();

    expect(screen.getByText('App')).toBeVisible();
    expect(draws.flat()).toEqual([]);
    expect(pending.size).toBe(0);
  });
});

describe('unmounting', () => {
  it('cancels its frame and stops observing the card', () => {
    fakeCanvas();
    const { unmount } = renderCard();
    pointer('pointerEnter');
    const stale = [...pending.values()];

    unmount();

    expect(pending.size).toBe(0);
    expect(resizeObservers[0].disconnect).toHaveBeenCalledOnce();
    // A frame the browser had already started does nothing.
    const drawn = draws.length;
    stale.forEach((callback) => callback(0));
    expect(draws).toHaveLength(drawn);
  });

  it('survives a frame that lands after React has taken the contents away', () => {
    fakeCanvas();
    // Reduced motion, so the one frame swaps the contents.
    vi.stubGlobal('matchMedia', () => ({ matches: true }));
    // React detaches the card's refs to its contents before it runs their
    // layout cleanups, and stops the card's own effect only after. A frame
    // run from a cleanup in the second content is the one moment a frame can
    // find both contents gone.
    const RunOnUnmount = ({ children }: { children: ReactNode }) => {
      useLayoutEffect(() => () => step(), []);
      return children;
    };
    const { unmount } = render(
      <PixelTransition
        firstContent={<p>Shell</p>}
        secondContent={
          <RunOnUnmount>
            <p>App</p>
          </RunOnUnmount>
        }
      />,
    );
    pointer('pointerEnter');
    const drawn = draws.length;

    expect(() => unmount()).not.toThrow();
    expect(draws).toHaveLength(drawn + 1);
  });
});
