import { act, fireEvent, render } from '@testing-library/react';
import type { ReactElement } from 'react';
import {
  afterEach,
  beforeEach,
  describe,
  expect,
  it,
  vi,
  type Mock,
} from 'vitest';

// DecryptedText guards each per-letter lookup in case its frame (its state)
// and its text fall out of step, and its measuring in case its element is
// gone. React keeps all of that in step, so to reach those guards a test has
// to hold the state's setter and the refs. The module's useState and useRef
// pass straight through to React's, remembering what they hand out.
const hooks = vi.hoisted(() => ({
  setFrame: (_frame: unknown) => {},
  refs: [] as { current: unknown }[],
}));

vi.mock('react', async (importOriginal) => {
  const actual = await importOriginal<typeof import('react')>();
  return {
    ...actual,
    useState: ((initial: unknown) => {
      const state = actual.useState(initial);
      // Its only state, made before any of its refs: each render's refs
      // follow it.
      hooks.setFrame = state[1];
      hooks.refs = [];
      return state;
    }) as typeof actual.useState,
    useRef: ((initial: unknown) => {
      const ref = actual.useRef(initial);
      hooks.refs.push(ref);
      return ref;
    }) as typeof actual.useRef,
  };
});

type Component = typeof import('./decrypted-text').default;

// A fresh module for every test: it caches glyph widths, and the canvas it
// measures them with, for as long as it's loaded.
const load = async () => {
  vi.resetModules();
  return (await import('./decrypted-text')).default;
};

let DecryptedText: Component;
let context: { font: string; measureText: Mock };
let widths: Record<string, number>;

const renderText = (ui: ReactElement) => {
  const result = render(ui);
  return { ...result, root: result.container.firstElementChild as HTMLElement };
};

const cellsOf = (root: Element) => [
  ...root.querySelectorAll<HTMLElement>('[aria-hidden="true"] > span > span'),
];

// What the visitor sees: each letter, or the glyph standing in for it, with
// the spaces between the words.
const shown = (root: Element) =>
  [...root.querySelector('[aria-hidden="true"]')!.childNodes]
    .map((node) =>
      node instanceof Element
        ? [...node.children]
            .map((cell) => (cell.lastElementChild ?? cell).textContent)
            .join('')
        : node.textContent,
    )
    .join('');

// A scrambled letter keeps the real one, hidden, to hold its width, with
// the glyph laid over it.
const scrambled = (root: Element) =>
  cellsOf(root).filter((cell) => cell.childElementCount > 0);

const glyphOf = (cell: HTMLElement) => cell.lastElementChild as HTMLElement;

const tick = (ms = 50) =>
  act(() => {
    vi.advanceTimersByTime(ms);
  });

const reduceMotion = (reduce: boolean) =>
  vi.stubGlobal(
    'matchMedia',
    vi.fn(() => ({ matches: reduce })),
  );

interface Observed {
  callback: (entries: unknown[]) => void;
  options: unknown;
  observe: Mock;
  disconnect: Mock;
}

// jsdom has neither observer. Each fake keeps what it was made with, so the
// test can say when the text scrolls into view or changes size.
const stubObserver = (name: 'IntersectionObserver' | 'ResizeObserver') => {
  const made: Observed[] = [];
  vi.stubGlobal(
    name,
    class {
      observe = vi.fn();
      disconnect = vi.fn();
      constructor(callback: Observed['callback'], options?: unknown) {
        made.push({
          callback,
          options,
          observe: this.observe,
          disconnect: this.disconnect,
        });
      }
    },
  );
  return made;
};

const seen = (observers: Observed[], isIntersecting: boolean) =>
  act(() => observers.at(-1)!.callback([{ isIntersecting }]));

beforeEach(async () => {
  DecryptedText = await load();
  vi.useFakeTimers();
  // jsdom has no canvas. This context measures each glyph at the width the
  // test gives it, 10px otherwise.
  widths = {};
  context = {
    font: '',
    measureText: vi.fn((glyph: string) => ({ width: widths[glyph] ?? 10 })),
  };
  vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue(
    context as never,
  );
});

afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
  Reflect.deleteProperty(HTMLElement.prototype, 'animate');
});

describe('at rest', () => {
  it('shows the text, whole for screen readers, and passes other props to its wrapper', () => {
    const { root } = renderText(
      <DecryptedText
        text="Hi yo"
        parentClassName="wrap"
        className="plain"
        title="Greeting"
      />,
    );

    expect(root).toHaveClass('wrap');
    expect(root).toHaveAttribute('title', 'Greeting');
    expect(root.firstElementChild).toHaveTextContent('Hi yo');
    expect(root.lastElementChild).toHaveAttribute('aria-hidden', 'true');
    expect(shown(root)).toBe('Hi yo');
    // A cell per letter, none for the space, so words wrap whole.
    expect(cellsOf(root)).toHaveLength(4);
    cellsOf(root).forEach((cell) => expect(cell).toHaveClass('plain'));
    // Nothing moves until it's hovered.
    tick(1000);
    expect(shown(root)).toBe('Hi yo');
    expect(vi.getTimerCount()).toBe(0);
  });

  it('shows nothing for no text', () => {
    const { root } = renderText(
      <DecryptedText text={undefined as unknown as string} />,
    );

    expect(root).toHaveTextContent('');
    expect(cellsOf(root)).toEqual([]);
  });
});

describe('on hover', () => {
  it('scrambles every letter, then reveals them all after maxIterations ticks', () => {
    const { root } = renderText(
      <DecryptedText
        text="Hi yo"
        characters="#"
        encryptedClassName="secret"
        maxIterations={3}
      />,
    );

    fireEvent.mouseEnter(root);

    expect(shown(root)).toBe('## ##');
    scrambled(root).forEach((cell) => expect(cell).toHaveClass('secret'));

    // Hovering again, or moving about, doesn't restart the run.
    tick();
    fireEvent.mouseEnter(root);
    fireEvent.mouseMove(root);
    tick();
    expect(shown(root)).toBe('## ##');
    tick();
    expect(shown(root)).toBe('Hi yo');
    // The ticker stops once there's nothing left to do.
    expect(vi.getTimerCount()).toBe(0);
  });

  it('reveals everything at once when the pointer leaves', () => {
    const { root } = renderText(<DecryptedText text="Hi yo" characters="#" />);

    fireEvent.mouseEnter(root);
    tick();
    fireEvent.mouseLeave(root);

    expect(shown(root)).toBe('Hi yo');
    // The ticker runs once more, finds nothing to do, and stops.
    tick();
    expect(vi.getTimerCount()).toBe(0);
  });

  it.each([
    ['start', ['a## #', 'ab# #', 'abc #', 'abc d']],
    ['end', ['### d', '##c d', '#bc d', 'abc d']],
    // Nearest the middle letter first, ties in reading order.
    ['center', ['##c #', '#bc #', 'abc #', 'abc d']],
    // With Math.random() at 0, the shuffle puts the first letter last.
    ['random', ['#b# #', '#bc #', '#bc d', 'abc d']],
  ] as const)(
    'reveals one letter a tick from the %s when sequential',
    (direction, frames) => {
      vi.spyOn(Math, 'random').mockReturnValue(0);
      const { root } = renderText(
        <DecryptedText
          text="abc d"
          characters="#"
          sequential
          revealDirection={direction}
        />,
      );

      fireEvent.mouseEnter(root);
      expect(shown(root)).toBe('### #');

      const seenFrames = frames.map(() => {
        tick();
        return shown(root);
      });

      expect(seenFrames).toEqual(frames);
      expect(vi.getTimerCount()).toBe(0);
    },
  );

  it('finishes at once when there are no letters to reveal one by one', () => {
    const { root } = renderText(<DecryptedText text="  " sequential />);

    fireEvent.mouseEnter(root);
    expect(vi.getTimerCount()).toBe(1);
    tick();

    expect(shown(root)).toBe('  ');
    expect(vi.getTimerCount()).toBe(0);
  });
});

describe('on click', () => {
  it('starts scrambled and reveals itself once', () => {
    const { root } = renderText(
      <DecryptedText
        text="ok"
        animateOn="click"
        characters="#"
        maxIterations={2}
      />,
    );
    expect(shown(root)).toBe('##');

    fireEvent.click(root);
    tick();
    // Clicking during the run, or after it, changes nothing.
    fireEvent.click(root);
    tick();
    expect(shown(root)).toBe('ok');
    fireEvent.click(root);

    expect(shown(root)).toBe('ok');
    expect(vi.getTimerCount()).toBe(0);
  });

  it('toggles between revealed and scrambled in toggle mode', () => {
    const { root } = renderText(
      <DecryptedText
        text="ab cde"
        animateOn="click"
        clickMode="toggle"
        characters="#"
        maxIterations={2}
      />,
    );

    fireEvent.click(root);
    tick(100);
    expect(shown(root)).toBe('ab cde');

    // Scrambling back hides ceil(5 / 2) = 3 letters a tick, at random.
    fireEvent.click(root);
    expect(scrambled(root)).toHaveLength(0);
    tick();
    expect(scrambled(root)).toHaveLength(3);
    tick();
    expect(shown(root)).toBe('## ###');
    expect(vi.getTimerCount()).toBe(0);
  });

  it('stays scrambled when toggled away, even when it loops', () => {
    const { root } = renderText(
      <DecryptedText
        text="ab"
        animateOn="click"
        clickMode="toggle"
        characters="#"
        maxIterations={1}
        loop
        loopDelay={500}
      />,
    );

    fireEvent.click(root);
    tick();
    expect(shown(root)).toBe('ab');
    // Toggled away before the loop's pause ends: the pause is cancelled, and
    // a run back to scrambled doesn't start another cycle.
    fireEvent.click(root);
    tick();
    expect(shown(root)).toBe('##');
    tick(5000);

    expect(shown(root)).toBe('##');
    expect(vi.getTimerCount()).toBe(0);
  });
});

describe('in view', () => {
  it('starts scrambled and reveals itself once it scrolls into view', () => {
    const intersections = stubObserver('IntersectionObserver');
    const { root, unmount } = renderText(
      <DecryptedText
        text="Hi"
        animateOn="view"
        characters="#"
        maxIterations={1}
      />,
    );
    const [observer] = intersections;
    expect(observer.observe).toHaveBeenCalledWith(root);
    expect(observer.options).toEqual({ threshold: 0.1 });

    seen(intersections, false);
    tick();
    expect(shown(root)).toBe('##');

    seen(intersections, true);
    expect(observer.disconnect).toHaveBeenCalled();
    tick();
    expect(shown(root)).toBe('Hi');

    // It plays once: in view again, nothing restarts.
    seen(intersections, true);
    expect(shown(root)).toBe('Hi');
    expect(vi.getTimerCount()).toBe(0);
    // Clicks are for animateOn="click".
    fireEvent.click(root);
    expect(vi.getTimerCount()).toBe(0);

    observer.disconnect.mockClear();
    unmount();
    expect(observer.disconnect).toHaveBeenCalled();
  });

  it('stays scrambled where the browser has no IntersectionObserver', () => {
    const { root } = renderText(
      <DecryptedText text="Hi" animateOn="view" characters="#" />,
    );

    tick(1000);

    expect(shown(root)).toBe('##');
  });

  it('plays again when the text changes', () => {
    const intersections = stubObserver('IntersectionObserver');
    const { root, rerender } = renderText(
      <DecryptedText
        text="Hi"
        animateOn="view"
        characters="#"
        maxIterations={1}
      />,
    );
    seen(intersections, true);
    tick();
    expect(shown(root)).toBe('Hi');

    rerender(
      <DecryptedText
        text="Yo"
        animateOn="view"
        characters="#"
        maxIterations={1}
      />,
    );
    expect(shown(root)).toBe('##');
    expect(intersections).toHaveLength(2);
    seen(intersections, true);
    tick();

    expect(shown(root)).toBe('Yo');
  });
});

describe('looping', () => {
  it('scrambles away after loopDelay, letter by letter, and comes back after a pause', () => {
    const intersections = stubObserver('IntersectionObserver');
    const { root } = renderText(
      <DecryptedText
        text="ab"
        animateOn="view"
        characters="#"
        sequential
        loop
        loopDelay={1000}
      />,
    );

    seen(intersections, true);
    // It keeps watching, to hold the loop while it's out of view.
    expect(intersections[0].disconnect).not.toHaveBeenCalled();
    tick();
    expect(shown(root)).toBe('a#');
    tick();
    expect(shown(root)).toBe('ab');

    tick(999);
    expect(shown(root)).toBe('ab');
    tick(1);
    // Backwards, from the last letter.
    tick();
    expect(shown(root)).toBe('a#');
    tick();
    expect(shown(root)).toBe('##');

    // Then it waits max(2, 320ms / 50ms) = 6 ticks before revealing again.
    tick(6 * 50);
    expect(shown(root)).toBe('##');
    tick();
    expect(shown(root)).toBe('a#');
    tick();
    expect(shown(root)).toBe('ab');
  });

  it('holds the next cycle while out of view, and resumes it when seen', () => {
    const intersections = stubObserver('IntersectionObserver');
    const { root } = renderText(
      <DecryptedText
        text="ab"
        animateOn="view"
        characters="#"
        maxIterations={1}
        loop
        loopDelay={-100}
      />,
    );
    seen(intersections, true);
    tick();
    expect(shown(root)).toBe('ab');

    // Out of view when the pause (no pause, for a negative loopDelay) ends.
    seen(intersections, false);
    tick(0);
    tick(5000);
    expect(shown(root)).toBe('ab');

    seen(intersections, true);
    tick();
    expect(shown(root)).toBe('##');
    // Not sequential, so there's no wait before revealing again.
    tick();
    expect(shown(root)).toBe('ab');
  });

  it('stops looping when loop is turned off during the pause', () => {
    const intersections = stubObserver('IntersectionObserver');
    const props = {
      text: 'ab',
      animateOn: 'view',
      characters: '#',
      maxIterations: 1,
      loopDelay: 500,
    } as const;
    const { root, rerender } = renderText(<DecryptedText {...props} loop />);
    seen(intersections, true);
    tick();

    rerender(<DecryptedText {...props} loop={false} />);
    tick(5000);

    expect(shown(root)).toBe('ab');
    expect(vi.getTimerCount()).toBe(0);
  });

  it('scrambles away after loopDelay once the pointer leaves, in inViewHover', () => {
    const { root } = renderText(
      <DecryptedText
        text="ab"
        animateOn="inViewHover"
        characters="#"
        maxIterations={1}
        loop
        loopDelay={500}
      />,
    );
    expect(shown(root)).toBe('##');

    fireEvent.mouseEnter(root);
    tick();
    expect(shown(root)).toBe('ab');
    fireEvent.mouseLeave(root);
    tick(499);
    expect(shown(root)).toBe('ab');
    tick(1);
    tick();

    expect(shown(root)).toBe('##');
  });
});

describe('with a hover radius', () => {
  // Each letter's cell is 10px square, side by side from the left of the
  // text: the first letter's centre is at (5, 5).
  const layOut = () =>
    vi
      .spyOn(HTMLElement.prototype, 'getBoundingClientRect')
      .mockImplementation(function (this: HTMLElement) {
        const at = cellsOf(document.body).indexOf(this);
        return {
          left: Math.max(0, at) * 10,
          top: 0,
          width: at < 0 ? 40 : 10,
          height: 10,
        } as DOMRect;
      });

  it('scrambles the letters near the pointer, more the nearer, and settles once it leaves', () => {
    layOut();
    // Below 2 × a letter's heat, the letter scrambles this tick.
    vi.spyOn(Math, 'random').mockReturnValue(0.3);
    const { root } = renderText(
      <DecryptedText
        text="ab c"
        characters="#"
        hoverRadius={15}
        scrambleColor="red"
        effect="fade"
      />,
    );
    const [a, b, c] = cellsOf(root);

    fireEvent.mouseEnter(root, { clientX: 5, clientY: 5 });
    expect(shown(root)).toBe('ab c');
    tick();

    // `a` is under the pointer, `b` 10px away, and `c`, 20px away, is
    // outside the 15px radius.
    expect(shown(root)).toBe('## c');
    expect(a.style.color).toBe('red');
    expect(parseFloat(a.style.opacity)).toBeCloseTo(0.16);
    // b's heat is 1 − d²(3 − 2d) for d = 10/15: 0.259, tinted 39%.
    expect(b.style.color).toMatch(
      /^color-mix\(in srgb, red 39%, currentcolor\)$/i,
    );
    expect(parseFloat(b.style.opacity)).toBeCloseTo(1 - 0.84 * 0.3889, 3);

    // Moving on warms `c`, and `a` cools slowly behind the pointer.
    fireEvent.mouseMove(root, { clientX: 25, clientY: 5 });
    tick();
    expect(shown(root)).toBe('## #');
    expect(c.childElementCount).toBe(2);

    fireEvent.mouseLeave(root);
    tick(20 * 50);
    expect(shown(root)).toBe('ab c');
    expect(vi.getTimerCount()).toBe(0);
  });

  it('ignores the pointer when the visitor prefers reduced motion', () => {
    reduceMotion(true);
    const { root } = renderText(
      <DecryptedText text="ab" characters="#" hoverRadius={15} />,
    );

    fireEvent.mouseEnter(root, { clientX: 5, clientY: 5 });

    expect(vi.getTimerCount()).toBe(0);
    expect(shown(root)).toBe('ab');
  });
});

describe('with reduced motion', () => {
  it('shows the text from the start instead of revealing it in view', () => {
    reduceMotion(true);
    const intersections = stubObserver('IntersectionObserver');
    const { root } = renderText(
      <DecryptedText text="Hi" animateOn="view" characters="#" />,
    );

    expect(shown(root)).toBe('Hi');
    seen(intersections, true);
    expect(shown(root)).toBe('Hi');
    expect(vi.getTimerCount()).toBe(0);
  });

  it('reveals and scrambles at once on click, with no ticks between', () => {
    reduceMotion(true);
    const { root } = renderText(
      <DecryptedText
        text="o k"
        animateOn="click"
        clickMode="toggle"
        characters="#"
      />,
    );
    expect(shown(root)).toBe('# #');

    fireEvent.click(root);
    expect(shown(root)).toBe('o k');
    fireEvent.click(root);
    expect(shown(root)).toBe('# #');
    expect(vi.getTimerCount()).toBe(0);
  });
});

describe('effects', () => {
  const stubAnimate = () => {
    const animate = vi.fn();
    Object.defineProperty(HTMLElement.prototype, 'animate', {
      value: animate,
      configurable: true,
    });
    return animate;
  };

  const flip = (angle: number, opacity: number) => [
    {
      transform: `perspective(4em) translateZ(-0.42em) rotateX(${angle}deg) translateZ(0.42em)`,
      opacity,
    },
    {
      transform:
        'perspective(4em) translateZ(-0.42em) rotateX(0deg) translateZ(0.42em)',
      opacity: 1,
    },
  ];

  it('flips each letter as it scrambles, and flips it upright as it settles', () => {
    const animate = stubAnimate();
    vi.spyOn(Math, 'random').mockReturnValue(0.5);
    const { root } = renderText(
      <DecryptedText
        text="a b"
        characters="#"
        effect="flip"
        maxIterations={1}
      />,
    );
    const [a, b] = cellsOf(root);
    // Laid out to turn, revealed or not.
    expect(a.style.position).toBe('relative');
    expect(animate).not.toHaveBeenCalled();

    fireEvent.mouseEnter(root);

    // The letters, not the space: 58° + 0.5 × 26°, quickly.
    expect(animate).toHaveBeenCalledTimes(2);
    expect(animate.mock.contexts).toEqual([a, b]);
    expect(animate).toHaveBeenCalledWith(flip(71, 0.3), {
      duration: 46,
      easing: 'cubic-bezier(0.3, 0.7, 0.4, 1)',
    });

    animate.mockClear();
    tick();
    expect(animate).toHaveBeenCalledTimes(2);
    expect(animate).toHaveBeenCalledWith(flip(90, 0), {
      duration: 360,
      easing: 'cubic-bezier(0.34, 1.45, 0.64, 1)',
    });

    // Revealed already, nothing turns.
    animate.mockClear();
    fireEvent.mouseLeave(root);
    expect(animate).not.toHaveBeenCalled();
  });

  it("doesn't flip where the browser can't animate, or the visitor prefers reduced motion", () => {
    const { root, unmount } = renderText(
      <DecryptedText text="ab" characters="#" effect="flip" />,
    );
    fireEvent.mouseEnter(root);
    expect(shown(root)).toBe('##');
    unmount();

    const animate = stubAnimate();
    reduceMotion(true);
    const second = renderText(
      <DecryptedText text="ab" characters="#" effect="flip" />,
    );
    fireEvent.mouseEnter(second.root);

    expect(animate).not.toHaveBeenCalled();
  });

  it('fades scrambled letters in as the run goes on, tinted, and settles them', () => {
    const { root } = renderText(
      <DecryptedText
        text="ab"
        animateOn="click"
        characters="#"
        effect="fade"
        scrambleColor="red"
        maxIterations={2}
      />,
    );
    const [a] = cellsOf(root);
    expect(a.style.opacity).toBe('0.16');
    expect(a.style.color).toBe('red');
    expect(a.style.transition).toBe('color 0.12s linear, opacity 50ms linear');

    fireEvent.click(root);
    tick();
    // Halfway through the run, halfway from 0.16 to 0.62.
    expect(parseFloat(a.style.opacity)).toBeCloseTo(0.39);
    tick();

    expect(shown(root)).toBe('ab');
    expect(a.style.opacity).toBe('1');
    expect(a.style.color).toBe('');
    expect(a.style.position).toBe('');
    expect(a.style.transition).toContain('color 0.7s');
  });

  it('fades sequential letters in by their place in the queue', () => {
    const { root } = renderText(
      <DecryptedText
        text="abcdefgh"
        animateOn="click"
        characters="#"
        effect="fade"
        sequential
      />,
    );
    fireEvent.click(root);

    // The next letter is brightest; six or more places back, dimmest.
    const opacities = cellsOf(root).map((cell) =>
      parseFloat(cell.style.opacity),
    );
    expect(opacities[0]).toBeCloseTo(0.62);
    expect(opacities[3]).toBeCloseTo(0.16 + 0.46 * 0.5);
    expect(opacities[6]).toBeCloseTo(0.16);
    expect(opacities[7]).toBeCloseTo(0.16);
  });
});

describe('glyphs', () => {
  it.each([
    ['the given characters', { characters: '%' }, '%%'],
    ['the letters, digits and symbols by default', {}, 'AA'],
    [
      "the text's own letters, when asked",
      { useOriginalCharsOnly: true },
      'bb',
    ],
    ["the text's own letters, given no characters", { characters: '' }, 'bb'],
  ])('scrambles with %s', (_, props, expected) => {
    vi.spyOn(Math, 'random').mockReturnValue(0);
    const { root } = renderText(
      <DecryptedText text="ba" animateOn="click" {...props} />,
    );

    expect(shown(root)).toBe(expected);
  });

  it('squeezes a glyph wider than its letter to fit, measured once a font', () => {
    widths = { i: 4, W: 12 };
    const { root } = renderText(
      <DecryptedText text="i i" animateOn="click" characters="W" />,
    );

    // 4px of room for 12px of glyph.
    scrambled(root).forEach((cell) =>
      expect(glyphOf(cell).style.transform).toBe(
        'translateX(-50%) scaleX(0.333)',
      ),
    );
    expect(context.measureText.mock.calls).toEqual([['i'], ['W']]);
  });

  it("leaves a glyph that fits as it is, and makes room for the font's letter spacing", () => {
    widths = { i: 4, W: 12 };
    const { root } = renderText(
      <DecryptedText
        text="Wi"
        animateOn="click"
        characters="i"
        style={{ letterSpacing: '8px' }}
      />,
    );
    const [w, i] = scrambled(root);

    expect(glyphOf(w).style.transform).toBe('translateX(-50%)');
    expect(glyphOf(i).style.transform).toBe('translateX(-50%)');
  });

  it("doesn't squeeze glyphs it can't measure, without a 2D canvas", () => {
    vi.mocked(HTMLCanvasElement.prototype.getContext).mockReturnValue(null);
    widths = { i: 4, W: 12 };
    const { root } = renderText(
      <DecryptedText text="i" animateOn="click" characters="W" />,
    );

    expect(glyphOf(scrambled(root)[0]).style.transform).toBe(
      'translateX(-50%)',
    );
    expect(context.measureText).not.toHaveBeenCalled();
  });

  it('measures again when its size changes, as when the font loads or changes', () => {
    const resizes = stubObserver('ResizeObserver');
    widths = { i: 4, W: 12 };
    const { root, rerender, unmount } = renderText(
      <DecryptedText text="i i" animateOn="click" characters="W" />,
    );
    expect(resizes[0].observe).toHaveBeenCalledWith(root);
    expect(glyphOf(scrambled(root)[0]).style.transform).toContain(
      'scaleX(0.333)',
    );

    // 2px more each: 6px of room for 14px.
    rerender(
      <DecryptedText
        text="i i"
        animateOn="click"
        characters="W"
        style={{ letterSpacing: '2px' }}
      />,
    );
    context.measureText.mockClear();
    act(() => resizes[0].callback([]));

    scrambled(root).forEach((cell) =>
      expect(glyphOf(cell).style.transform).toBe(
        'translateX(-50%) scaleX(0.429)',
      ),
    );
    // The font's widths were forgotten, so they're measured again.
    expect(context.measureText.mock.calls).toEqual([['i'], ['W']]);

    unmount();
    expect(resizes[0].disconnect).toHaveBeenCalled();
  });
});

describe('changing props', () => {
  it('ticks at the new speed when speed changes during a run, and never faster than 16ms', () => {
    const { root, rerender } = renderText(
      <DecryptedText text="ab" characters="#" speed={5} maxIterations={1} />,
    );
    fireEvent.mouseEnter(root);

    rerender(
      <DecryptedText text="ab" characters="#" speed={100} maxIterations={1} />,
    );
    tick(16);
    expect(shown(root)).toBe('##');
    tick(84);

    expect(shown(root)).toBe('ab');
  });

  it('shows new text as it is when it changes mid-run on hover', () => {
    const { root, rerender } = renderText(
      <DecryptedText text="ab" characters="#" />,
    );
    fireEvent.mouseEnter(root);
    expect(shown(root)).toBe('##');

    rerender(<DecryptedText text="xyz" characters="#" />);

    expect(shown(root)).toBe('xyz');
    tick();
    expect(vi.getTimerCount()).toBe(0);
  });

  it('stops its timers when it unmounts', () => {
    const intersections = stubObserver('IntersectionObserver');
    const { root, unmount } = renderText(
      <DecryptedText
        text="ab"
        animateOn="view"
        characters="#"
        maxIterations={1}
        loop
      />,
    );
    seen(intersections, true);
    tick();
    // The loop's pause, and a run on top of it from a hover.
    fireEvent.mouseEnter(root);
    expect(vi.getTimerCount()).toBe(1);

    unmount();

    expect(vi.getTimerCount()).toBe(0);
  });
});

describe('when its state and the page fall out of step', () => {
  // Its frame, the state it renders each letter from.
  interface Frame {
    shown: string[];
    revealed: boolean[];
    tint: number[];
    dim: number[];
    squeeze: number[];
  }
  const setFrame = (frame: Frame) => act(() => hooks.setFrame(frame));
  const refHolding = (test: (value: unknown) => boolean) =>
    hooks.refs.find((ref) => test(ref.current))!;

  it('falls back to the letter itself and plain styles for a letter its frame has no entry for', () => {
    const { root } = renderText(
      <DecryptedText
        text="ab"
        animateOn="click"
        effect="fade"
        scrambleColor="red"
      />,
    );

    setFrame({
      shown: [],
      revealed: [false, false],
      tint: [],
      dim: [],
      squeeze: [],
    });

    const [a] = cellsOf(root);
    expect(shown(root)).toBe('ab');
    expect(a.style.color).toBe('red');
    expect(a.style.opacity).toBe('0.16');
    expect(glyphOf(a).style.transform).toBe('translateX(-50%)');
  });

  it('skips entries of its frame past the end of the text', () => {
    const animate = vi.fn();
    Object.defineProperty(HTMLElement.prototype, 'animate', {
      value: animate,
      configurable: true,
    });
    const resizes = stubObserver('ResizeObserver');
    const { root } = renderText(
      <DecryptedText text="ab" characters="#" effect="flip" />,
    );
    const [a, b] = cellsOf(root);
    // A cell left over from longer text, as if React hadn't cleared it.
    const cells = refHolding(
      (value) => Array.isArray(value) && value.includes(a),
    ) as { current: (HTMLElement | null)[] };
    cells.current[2] = document.createElement('span');

    setFrame({
      shown: ['#', '#', 'W'],
      revealed: [false, false, false],
      tint: [1, 1, 1],
      dim: [1, 1, 1],
      squeeze: [1, 1, 1],
    });
    expect(animate.mock.contexts).toEqual([a, b]);

    // Refitting measures the stray glyph against itself.
    context.measureText.mockClear();
    act(() => resizes[0].callback([]));
    expect(context.measureText).toHaveBeenCalledWith('W');
  });

  it('measures nothing once its element is gone', () => {
    const { root } = renderText(
      <DecryptedText text="ab" characters="#" hoverRadius={15} />,
    );
    refHolding((value) => value === root).current = null;

    fireEvent.mouseEnter(root, { clientX: 5, clientY: 5 });
    tick();

    expect(shown(root)).toBe('ab');
    expect(vi.getTimerCount()).toBe(0);
  });

  it("doesn't squeeze glyphs without its element to read the font from", () => {
    widths = { a: 4, W: 12 };
    const { root } = renderText(<DecryptedText text="a" characters="W" />);
    refHolding((value) => value === root).current = null;

    fireEvent.mouseEnter(root);

    expect(glyphOf(scrambled(root)[0]).style.transform).toBe(
      'translateX(-50%)',
    );
    expect(context.measureText).not.toHaveBeenCalled();
  });

  it('keeps its frame when new text comes and it has no engine to repaint with', () => {
    const { root, rerender } = renderText(
      <DecryptedText text="ab" animateOn="click" characters="#" />,
    );
    // Its engine, the functions its timers and observers call, is set before
    // each render's other effects, so it's never missing. Made to be, here.
    const engine = refHolding(
      (value) => typeof value === 'object' && value !== null && 'tick' in value,
    );
    Object.defineProperty(engine, 'current', {
      get: () => null,
      set: () => {},
    });

    rerender(<DecryptedText text="abcd" animateOn="click" characters="#" />);

    // The new letters have no entry in the old frame, so they show as they
    // are.
    expect(shown(root)).toBe('##cd');
  });
});

describe('rendered on the server', () => {
  const renderOnServer = async (props: Record<string, unknown>) => {
    vi.stubGlobal('window', undefined);
    const Server = await load();
    const { renderToString } = await import('react-dom/server');
    const { createElement } = await import('react');
    return renderToString(createElement(Server, { text: 'ab', ...props }));
  };

  it('renders the text scrambled for a reveal, with glyphs picked from each letter', async () => {
    // 'a' (97) and 'b' (98 + 7) index into a pool of three.
    const html = await renderOnServer({ animateOn: 'view', characters: 'xyz' });

    expect(html).toContain('>ab</span>');
    expect(html).toMatch(/visibility:hidden">a<\/span><span [^>]+>y</);
    expect(html).toMatch(/visibility:hidden">b<\/span><span [^>]+>x</);
  });

  it('picks from the start of the pool for a letter with no code point', async () => {
    // A string's letters always have one; this one is made not to, for the
    // fallback.
    const codePointAt = String.prototype.codePointAt;
    vi.spyOn(String.prototype, 'codePointAt').mockImplementation(function (
      this: string,
      position: number,
    ) {
      return this === 'a' ? undefined : codePointAt.call(this, position);
    });

    const html = await renderOnServer({
      animateOn: 'click',
      characters: 'xyz',
    });

    expect(html).toMatch(/visibility:hidden">a<\/span><span [^>]+>x</);
  });
});
