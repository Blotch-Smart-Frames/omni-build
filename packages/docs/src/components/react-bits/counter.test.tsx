import { act, render } from '@testing-library/react';
import { createElement } from 'react';
import { renderToString } from 'react-dom/server';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import Counter, { type CounterProps } from './counter';

// Counter's DOM refs are always attached before its layout effects run, so
// its guards against a missing node can't be reached by rendering it. While
// `refs.detached` is set, each ref it starts as null is one React can't fill.
const refs = vi.hoisted(() => ({ detached: false }));

vi.mock('react', async (importOriginal) => {
  const React = await importOriginal<typeof import('react')>();
  const useRef = (initial: unknown) => {
    const ref = React.useRef(initial);
    const detached = React.useRef<{ current: unknown } | null>(null);
    if (!refs.detached || initial !== null) return ref;
    detached.current ??= {
      get current() {
        return null;
      },
      set current(_node: unknown) {},
    };
    return detached.current;
  };
  return { ...React, useRef };
});

const mod = (value: number, base: number) => ((value % base) + base) % base;

// Each digit is a strip of numerals, numeral n translated by (n - position)
// steps, so numeral 0's offset gives the strip's position.
const positionOf = (digit: Element) => {
  const zero = digit.querySelector<HTMLElement>('[data-numeral]')!;
  const offset = parseFloat(
    /translateY\((-?[\d.]+)%\)/.exec(zero.style.transform)![1],
  );
  return mod(-offset / 100, 10);
};

const digitsOf = (container: Element) => [
  ...container.querySelectorAll<HTMLElement>('.counter__digit'),
];

// What the row shows: each digit's numeral at its position, and every symbol
// and affix as written.
const reading = (container: Element) =>
  [...container.querySelector('.counter__row')!.children]
    .map((child) =>
      child.classList.contains('counter__digit')
        ? child.querySelectorAll('[data-numeral]')[
            Math.round(positionOf(child)) % 10
          ].textContent
        : child.textContent,
    )
    .join('');

const label = (container: Element) =>
  container.querySelector('.counter__sr')!.textContent;

// Animation frames and the clock they read, advanced a frame at a time. Each
// frame can update the component's state, so each runs inside act().
const useFrames = () =>
  vi.useFakeTimers({
    toFake: ['requestAnimationFrame', 'cancelAnimationFrame', 'performance'],
  });

const frames = (count = 1) => {
  for (let i = 0; i < count; i += 1) {
    act(() => {
      vi.advanceTimersToNextFrame();
    });
  }
};

// Frames until the counter stops asking for them: every digit has settled.
const settle = () => {
  for (let i = 0; i < 2000 && vi.getTimerCount() > 0; i += 1) frames();
  expect(vi.getTimerCount()).toBe(0);
};

const reduceMotion = (reduce: boolean) =>
  vi.stubGlobal(
    'matchMedia',
    vi.fn(() => ({ matches: reduce })),
  );

// jsdom lays nothing out. Numerals measure 0.6em, a 1 0.3em, and a symbol's
// text 0.25em, at the default 100px.
const layOut = () =>
  vi
    .spyOn(HTMLElement.prototype, 'getBoundingClientRect')
    .mockImplementation(function (this: HTMLElement) {
      const width = this.classList.contains('counter__symbol-text')
        ? 25
        : this.textContent === '1'
          ? 30
          : 60;
      return { width } as DOMRect;
    });

class FakeResizeObserver {
  static instances: FakeResizeObserver[] = [];
  observe = vi.fn();
  disconnect = vi.fn();
  constructor(readonly callback: () => void) {
    FakeResizeObserver.instances.push(this);
  }
}

class FakeIntersectionObserver {
  static instances: FakeIntersectionObserver[] = [];
  observe = vi.fn();
  disconnect = vi.fn();
  constructor(
    readonly callback: (entries: { isIntersecting: boolean }[]) => void,
    readonly options: IntersectionObserverInit,
  ) {
    FakeIntersectionObserver.instances.push(this);
  }
  show(isIntersecting: boolean) {
    this.callback([{ isIntersecting }]);
  }
}

// Stand-ins for the slots the counter looks up after each render. Only that
// lookup (`[data-slot]`) is changed; every other query is the real one.
const replaceSlots = (slots: Element[]) => {
  const { querySelectorAll } = Element.prototype;
  return vi
    .spyOn(Element.prototype, 'querySelectorAll')
    .mockImplementation(function (this: Element, selector: string) {
      if (selector === '[data-slot]') {
        return slots as unknown as NodeListOf<Element>;
      }
      return querySelectorAll.call(this, selector);
    });
};

const still: CounterProps = { intro: false };

beforeEach(() => {
  FakeResizeObserver.instances = [];
  FakeIntersectionObserver.instances = [];
});

afterEach(() => {
  refs.detached = false;
  vi.useRealTimers();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
  Reflect.deleteProperty(document, 'fonts');
});

describe('Counter', () => {
  it('labels the number for screen readers and shows its digits at rest', () => {
    useFrames();
    const { container } = render(<Counter value={1234.5} />);

    expect(label(container)).toBe('1,234.5');
    expect(reading(container)).toBe('1,234.5');
    const root = container.firstElementChild as HTMLElement;
    expect(root).toHaveClass('counter', { exact: true });
    expect(root.style).toMatchObject({
      fontSize: '100px',
      fontWeight: 'inherit',
      color: 'inherit',
      fontVariantNumeric: 'tabular-nums',
    });
    expect(root.style.getPropertyValue('--counter-fade')).toBe('0.3em');
    expect(root.style.getPropertyValue('--counter-gap')).toBe('0px');
    expect(root.style.getPropertyValue('--counter-padding')).toBe('0px');
    // The digits are drawn for the eye only; the label is what's read.
    expect(container.querySelector('.counter__row')).toHaveAttribute(
      'aria-hidden',
      'true',
    );
    expect(
      [...container.querySelectorAll('[data-slot]')].map(
        (slot) => (slot as HTMLElement).dataset.slot,
      ),
    ).toEqual(['i3', 'g3', 'i2', 'i1', 'i0', 'd', 'f0']);
  });

  it('takes its look from the props', () => {
    useFrames();
    const { container } = render(
      <Counter
        {...still}
        value={7}
        fontSize="2rem"
        fontWeight={700}
        textColor="red"
        fade={-1}
        gap={4}
        padding={2}
        tabular={false}
        className="extra"
        style={{ opacity: 0.5 }}
        digitStyle={{ color: 'blue' }}
      />,
    );

    const root = container.firstElementChild as HTMLElement;
    expect(root).toHaveClass('counter extra', { exact: true });
    expect(root.style).toMatchObject({
      fontSize: '2rem',
      fontWeight: '700',
      color: 'red',
      fontVariantNumeric: '',
      opacity: '0.5',
    });
    // Negative sizes are no size.
    expect(root.style.getPropertyValue('--counter-fade')).toBe('0em');
    expect(root.style.getPropertyValue('--counter-gap')).toBe('4px');
    expect(root.style.getPropertyValue('--counter-padding')).toBe('2px');
    expect(digitsOf(container)[0].style.color).toBe('blue');
  });

  describe('formatting', () => {
    it.each<[string, CounterProps, string]>([
      [
        'a currency before the number',
        { value: 1234.5, format: { style: 'currency', currency: 'USD' } },
        '$1,234.50',
      ],
      [
        'a percent sign after it',
        { value: 0.5, format: { style: 'percent' } },
        '50%',
      ],
      [
        'fixed decimals and leading zeros',
        { value: 7, decimals: 2.4, minDigits: 3 },
        '007.00',
      ],
      [
        'the locale’s separators',
        { value: 1234.5, locale: 'de-DE' },
        '1.234,5',
      ],
      ['a minus sign', { value: -42 }, '-42'],
      ['no sign on negative zero', { value: -0 }, '0'],
      [
        'a number given as a string',
        { value: '12' as unknown as number },
        '12',
      ],
      ['not a number', { value: Number.NaN }, 'NaN'],
      [
        'en-US for a locale Intl rejects',
        { value: 1234, locale: 'not a locale!' },
        '1,234',
      ],
    ])('writes %s', (_, props, written) => {
      useFrames();
      const { container } = render(<Counter {...still} {...props} />);

      expect(label(container)).toBe(written);
      expect(reading(container)).toBe(written);
    });

    it('draws the locale’s own numerals', () => {
      useFrames();
      const { container } = render(
        <Counter {...still} value={42} locale="ar-EG" />,
      );

      expect(label(container)).toBe('٤٢');
      expect(reading(container)).toBe('٤٢');
    });

    it('puts the prefix and suffix around the number, and leaves out empty ones', () => {
      useFrames();
      const { container, rerender } = render(
        <Counter {...still} value={5} prefix="~" suffix={null} />,
      );

      expect(label(container)).toBe('~5');
      expect(reading(container)).toBe('~5');
      expect(container.querySelectorAll('.counter__affix')).toHaveLength(1);

      rerender(<Counter {...still} value={5} prefix={false} suffix=" kg" />);

      expect(label(container)).toBe('5 kg');
      expect(reading(container)).toBe('5 kg');
      expect(container.querySelectorAll('.counter__affix')).toHaveLength(1);
    });
  });

  describe('changing value', () => {
    it('spins each digit to its new value', () => {
      useFrames();
      const { container, rerender } = render(<Counter {...still} value={10} />);

      rerender(<Counter {...still} value={25} />);
      frames(3);

      // Partway: the strips are between numerals.
      const [tens, ones] = digitsOf(container);
      expect(positionOf(tens) % 1).not.toBe(0);
      expect(positionOf(ones) % 1).not.toBe(0);

      settle();

      expect(reading(container)).toBe('25');
      expect(positionOf(tens)).toBe(2);
      expect(positionOf(ones)).toBe(5);
      expect(label(container)).toBe('25');
    });

    it('starts the lowest digit first and staggers the rest', () => {
      useFrames();
      const { container, rerender } = render(
        <Counter {...still} value={111} stagger={0.02} />,
      );

      rerender(<Counter {...still} value={999} stagger={0.02} />);
      frames(2);

      // Two frames are 32ms: past the tens' 20ms wait, short of the
      // hundreds' 40ms.
      const [hundreds, tens, ones] = digitsOf(container).map(positionOf);
      expect(ones).toBeGreaterThan(tens);
      expect(tens).toBeGreaterThan(1);
      expect(hundreds).toBe(1);

      settle();

      expect(reading(container)).toBe('999');
    });

    // The first frame's step shows which way a digit turns.
    const firstStep = (
      from: number,
      to: number,
      direction?: CounterProps['direction'],
    ) => {
      useFrames();
      const { container, rerender } = render(
        <Counter {...still} value={from} direction={direction} />,
      );
      rerender(<Counter {...still} value={to} direction={direction} />);
      const before = positionOf(digitsOf(container).at(-1)!);
      frames();
      const after = positionOf(digitsOf(container).at(-1)!);
      settle();
      expect(reading(container)).toBe(String(to));
      return wrapStep(after - before);
    };
    const wrapStep = (step: number) => mod(step + 5, 10) - 5;

    it('counts up as the number grows, the long way round if need be', () => {
      expect(firstStep(1, 9)).toBeGreaterThan(0);
    });

    it('counts down as the number shrinks', () => {
      expect(firstStep(9, 1)).toBeLessThan(0);
    });

    it.each<[CounterProps['direction'], number, number, 1 | -1]>([
      ['up', 9, 1, 1],
      ['down', 1, 9, -1],
      // 1 to 9 is two steps down, not eight up.
      ['shortest', 1, 9, -1],
      // Anything else is 'auto': 9 to 1 shrinks, so down.
      ['sideways' as CounterProps['direction'], 9, 1, -1],
    ])('turns %s when told to', (direction, from, to, sign) => {
      expect(Math.sign(firstStep(from, to, direction))).toBe(sign);
    });

    it('turns no digit that stays the same, whichever the direction', () => {
      useFrames();
      const { container, rerender } = render(
        <Counter {...still} value={15} direction="down" />,
      );

      rerender(<Counter {...still} value={25} direction="down" />);
      frames(3);

      expect(positionOf(digitsOf(container)[1])).toBe(5);
      settle();
      expect(reading(container)).toBe('25');
    });

    it('turns nothing when only the sign changes', () => {
      useFrames();
      const { container, rerender } = render(<Counter {...still} value={5} />);

      rerender(<Counter {...still} value={-5} />);
      frames();

      expect(positionOf(digitsOf(container)[0])).toBe(5);
      settle();
      expect(reading(container)).toBe('-5');
    });

    it('rerenders nothing when the written number is the same', () => {
      useFrames();
      const { container, rerender } = render(
        <Counter {...still} value={5} decimals={0} />,
      );
      const digit = digitsOf(container)[0];

      rerender(<Counter {...still} value={5.4} decimals={0} />);
      settle();

      expect(digitsOf(container)[0]).toBe(digit);
      expect(positionOf(digit)).toBe(5);
      expect(reading(container)).toBe('5');
    });

    it('grows new digits in from nothing, a zero included', () => {
      useFrames();
      layOut();
      const { container, rerender } = render(<Counter {...still} value={5} />);

      rerender(<Counter {...still} value={105} />);
      frames(4);

      // The new hundreds and tens fade and widen in.
      const [hundreds, tens] = digitsOf(container);
      for (const digit of [hundreds, tens]) {
        const shown = Number(digit.style.getPropertyValue('--counter-shown'));
        expect(shown).toBeGreaterThan(0);
        expect(shown).toBeLessThan(1);
        expect(digit.style.width).toMatch(/^\d+\.\d\dpx$/);
      }

      settle();

      expect(reading(container)).toBe('105');
      for (const digit of digitsOf(container)) {
        expect(digit.style.getPropertyValue('--counter-shown')).toBe('');
        expect(digit.style.width).toBe('');
      }
    });

    it('lets digits go when the number loses them, even mid-change', () => {
      useFrames();
      const { container, rerender } = render(
        <Counter {...still} value={100} />,
      );

      rerender(<Counter {...still} value={5} />);
      frames(2);
      // Still leaving; a second change mustn't bring them back.
      expect(digitsOf(container)).toHaveLength(3);
      rerender(<Counter {...still} value={6} />);
      settle();

      expect(digitsOf(container)).toHaveLength(1);
      expect(reading(container)).toBe('6');
    });

    it('brings a thousands separator in and out with its digit', () => {
      useFrames();
      layOut();
      const { container, rerender } = render(
        <Counter {...still} value={999} />,
      );

      rerender(<Counter {...still} value={1000} />);
      frames(2);

      const separator =
        container.querySelector<HTMLElement>('[data-slot="g3"]')!;
      // It waits for its digit, so it hasn't begun to show.
      expect(separator.style.getPropertyValue('--counter-shown')).toBe('0.000');
      expect(parseFloat(separator.style.width)).toBe(0);

      settle();

      expect(reading(container)).toBe('1,000');
      expect(separator.style.width).toBe('');

      // A change that keeps it leaves it be.
      rerender(<Counter {...still} value={1001} />);
      frames();
      expect(separator.style.getPropertyValue('--counter-shown')).toBe('');
      settle();

      rerender(<Counter {...still} value={999} />);
      frames(8);

      // Narrowing from its measured width.
      const width = parseFloat(separator.style.width);
      expect(width).toBeGreaterThan(0);
      expect(width).toBeLessThan(25);

      settle();

      expect(container.querySelector('[data-slot="g3"]')).toBeNull();
      expect(reading(container)).toBe('999');
    });

    it('widens and narrows proportional digits as they turn', () => {
      useFrames();
      layOut();
      const { container, rerender } = render(
        <Counter {...still} value={1} tabular={false} />,
      );
      const digit = digitsOf(container)[0];
      expect(digit.style.width).toBe('');

      rerender(<Counter {...still} value={2} tabular={false} />);
      frames(3);

      // Between a 1's width and a 2's.
      const width = parseFloat(digit.style.width);
      expect(width).toBeGreaterThan(30);
      expect(width).toBeLessThan(60);

      settle();

      expect(digit.style.width).toBe('');
    });

    it('jumps straight to the new value when the visitor prefers reduced motion', () => {
      useFrames();
      reduceMotion(true);
      const { container, rerender } = render(<Counter value={10} />);

      rerender(<Counter value={42} />);
      frames();

      expect(vi.getTimerCount()).toBe(0);
      expect(positionOf(digitsOf(container)[0])).toBe(4);
      expect(positionOf(digitsOf(container)[1])).toBe(2);
    });

    it('blurs a fast-turning digit along its motion, then clears it', () => {
      useFrames();
      // Measure the font size, so the blur is a share of it.
      vi.stubGlobal('ResizeObserver', FakeResizeObserver);
      const { container, rerender } = render(
        <Counter {...still} value={0} direction="up" />,
      );
      const strip = container.querySelector<HTMLElement>('[data-strip]')!;
      const blur = container.querySelector('feGaussianBlur')!;
      const filter = container.querySelector('filter')!;

      rerender(<Counter {...still} value={9} direction="up" />);
      let blurred = false;
      for (let i = 0; i < 30 && !blurred; i += 1) {
        frames();
        blurred = strip.style.filter !== '';
      }

      expect(strip.style.filter).toBe(`url("#${filter.id}")`);
      expect(blur.getAttribute('stdDeviation')).toMatch(/^0 \d+\.\d\d$/);

      settle();

      expect(strip.style.filter).toBe('');
      expect(reading(container)).toBe('9');
    });

    it('draws no blur when motion blur is off', () => {
      useFrames();
      const { container, rerender } = render(
        <Counter {...still} value={0} direction="up" />,
      );
      expect(container.querySelector('svg')).not.toBeNull();

      rerender(
        <Counter {...still} value={0} direction="up" motionBlur={false} />,
      );
      expect(container.querySelector('svg')).toBeNull();
      rerender(
        <Counter {...still} value={9} direction="up" motionBlur={false} />,
      );

      const strip = container.querySelector<HTMLElement>('[data-strip]')!;
      for (let i = 0; i < 30; i += 1) {
        frames();
        expect(strip.style.filter).toBe('');
      }
      settle();
      expect(reading(container)).toBe('9');
    });

    it('stops asking for frames once unmounted', () => {
      useFrames();
      const { rerender, unmount } = render(<Counter {...still} value={1} />);
      rerender(<Counter {...still} value={8} />);
      expect(vi.getTimerCount()).toBe(1);

      unmount();

      expect(vi.getTimerCount()).toBe(0);
    });
  });

  describe('intro', () => {
    it('spins every digit a full turn when it comes into view', () => {
      useFrames();
      vi.stubGlobal('IntersectionObserver', FakeIntersectionObserver);
      const { container, unmount } = render(<Counter value={42} />);
      const [observer] = FakeIntersectionObserver.instances;
      expect(observer.options).toEqual({ threshold: 0.5 });
      expect(observer.observe).toHaveBeenCalledWith(
        container.firstElementChild,
      );

      observer.show(false);
      expect(vi.getTimerCount()).toBe(0);

      observer.show(true);
      expect(observer.disconnect).toHaveBeenCalled();
      frames(3);
      const [tens, ones] = digitsOf(container).map(positionOf);
      expect(ones).toBeGreaterThan(2);
      expect(tens).not.toBe(4);
      settle();
      expect(reading(container)).toBe('42');

      unmount();
      expect(observer.disconnect).toHaveBeenCalledTimes(2);
    });

    it('spins backwards when counting down', () => {
      useFrames();
      const { container } = render(<Counter value={5} direction="down" />);

      frames(3);

      expect(positionOf(digitsOf(container)[0])).toBeLessThan(5);
      settle();
      expect(reading(container)).toBe('5');
    });

    it('spins straight away without IntersectionObserver', () => {
      useFrames();
      vi.stubGlobal('IntersectionObserver', undefined);
      const { container } = render(<Counter value={1.5} prefix="$" />);

      expect(vi.getTimerCount()).toBe(1);
      settle();
      expect(reading(container)).toBe('$1.5');
    });

    it('stays still when the visitor prefers reduced motion', () => {
      useFrames();
      reduceMotion(true);
      vi.stubGlobal('IntersectionObserver', FakeIntersectionObserver);
      render(<Counter value={42} />);

      expect(FakeIntersectionObserver.instances).toEqual([]);
      expect(vi.getTimerCount()).toBe(0);
    });
  });

  describe('measuring', () => {
    it('measures the numerals and the font size again whenever they resize', () => {
      useFrames();
      vi.stubGlobal('ResizeObserver', FakeResizeObserver);
      const rect = layOut();
      const { container, unmount } = render(<Counter {...still} value={1} />);
      const [observer] = FakeResizeObserver.instances;
      const measure = container.querySelector('.counter__measure')!;
      expect(observer.observe).toHaveBeenCalledWith(measure);
      expect(rect.mock.contexts).toEqual(
        expect.arrayContaining([...measure.children]),
      );

      rect.mockClear();
      observer.callback();
      expect(rect.mock.contexts).toEqual([...measure.children]);

      unmount();
      expect(observer.disconnect).toHaveBeenCalled();
    });

    it('measures once the fonts have loaded, unless it has unmounted', async () => {
      useFrames();
      let loaded = () => {};
      Object.defineProperty(document, 'fonts', {
        configurable: true,
        value: {
          ready: new Promise<void>((resolve) => {
            loaded = resolve;
          }),
        },
      });
      const rect = layOut();
      const { unmount } = render(<Counter {...still} value={1} />);
      unmount();
      rect.mockClear();

      loaded();
      await act(() => Promise.resolve());

      expect(rect).not.toHaveBeenCalled();
    });

    it('measures again when the fonts load, and shrugs off a failure', async () => {
      useFrames();
      const rect = layOut();
      Object.defineProperty(document, 'fonts', {
        configurable: true,
        value: { ready: Promise.resolve() },
      });
      render(<Counter {...still} value={1} />);
      rect.mockClear();
      await act(() => Promise.resolve());
      expect(rect).toHaveBeenCalledTimes(10);

      Object.defineProperty(document, 'fonts', {
        configurable: true,
        value: { ready: Promise.reject(new Error('no fonts')) },
      });
      render(<Counter {...still} value={1} />);
      await act(() => Promise.resolve());
    });

    it('blurs by a 16px font when the font size measures nothing', () => {
      useFrames();
      vi.stubGlobal('ResizeObserver', FakeResizeObserver);
      const { container, rerender } = render(
        <Counter {...still} value={0} direction="up" fontSize={0} />,
      );
      const strip = container.querySelector<HTMLElement>('[data-strip]')!;
      const blur = container.querySelector('feGaussianBlur')!;

      rerender(<Counter {...still} value={9} direction="up" fontSize={0} />);
      let deviation = 0;
      for (let i = 0; i < 30; i += 1) {
        frames();
        deviation = Math.max(
          deviation,
          Number(blur.getAttribute('stdDeviation')!.split(' ')[1]),
        );
      }

      // At most 0.09em of 16px.
      expect(deviation).toBeGreaterThan(0.25);
      expect(deviation).toBeLessThanOrEqual(1.44);
      settle();
      expect(strip.style.filter).toBe('');
    });
  });

  // React fills every ref and slot before the effects that read them, so
  // these paths only run when something has gone wrong with the page. Each
  // test breaks the page the way the guard expects.
  describe('when the page is not what it drew', () => {
    it('skips slots it has no digit for, and digits it has no slot for', () => {
      useFrames();
      const { container, rerender } = render(<Counter {...still} value={9} />);
      // A slot element without a key, standing in for the whole row.
      replaceSlots([document.createElement('span')]);

      rerender(<Counter {...still} value={10} />);
      settle();

      // The ones digit, found before, turns; the new tens digit was never
      // found, so it keeps the numeral it was drawn with.
      const [tens, ones] = digitsOf(container);
      expect(positionOf(ones)).toBe(0);
      expect(tens.style.getPropertyValue('--counter-shown')).toBe('');
      expect(label(container)).toBe('10');
    });

    it('takes a symbol with no text to have no width', () => {
      useFrames();
      const { rerender } = render(<Counter {...still} value={999} />);
      const empty = document.createElement('span');
      empty.dataset.slot = 'g3';
      replaceSlots([empty]);

      rerender(<Counter {...still} value={1000} />);
      frames();

      expect(parseFloat(empty.style.width)).toBe(0);
      expect(empty.style.getPropertyValue('--counter-shown')).toBe('0.000');
    });

    it('clears the blur on the next frame when motion blur is turned off mid-turn', () => {
      useFrames();
      vi.stubGlobal('ResizeObserver', FakeResizeObserver);
      const { container, rerender } = render(
        <Counter {...still} value={0} direction="up" />,
      );
      const strip = container.querySelector<HTMLElement>('[data-strip]')!;
      rerender(<Counter {...still} value={9} direction="up" />);
      for (let i = 0; i < 30 && strip.style.filter === ''; i += 1) frames();
      expect(strip.style.filter).not.toBe('');

      // Keeping the counter from finding its slots again leaves the frame,
      // not the slot lookup, to see the setting change.
      replaceSlots([]);
      rerender(
        <Counter {...still} value={9} direction="up" motionBlur={false} />,
      );
      frames();

      expect(strip.style.filter).toBe('');
    });

    it('widens a new digit from nothing before any numeral is measured', () => {
      useFrames();
      vi.stubGlobal('ResizeObserver', FakeResizeObserver);
      const { container, rerender } = render(<Counter {...still} value={9} />);
      // Measured again with no numerals to measure.
      container.querySelector('.counter__measure')!.replaceChildren();
      FakeResizeObserver.instances[0].callback();

      rerender(<Counter {...still} value={10} />);
      frames(2);

      const tens = digitsOf(container)[0];
      expect(tens.style.getPropertyValue('--counter-shown')).not.toBe('');
      expect(parseFloat(tens.style.width)).toBe(0);
    });

    it('draws without measuring when it has no elements to measure', () => {
      useFrames();
      vi.stubGlobal('ResizeObserver', FakeResizeObserver);
      refs.detached = true;
      const { container, rerender } = render(<Counter value={9} />);

      rerender(<Counter value={10} />);

      expect(FakeResizeObserver.instances).toEqual([]);
      expect(label(container)).toBe('10');
    });
  });

  it('renders on the server, where there is no window', async () => {
    vi.resetModules();
    vi.stubGlobal('window', undefined);
    const { default: ServerCounter } = await import('./counter');

    const html = renderToString(createElement(ServerCounter, { value: 42 }));

    expect(html).toContain('42');
    expect(html).toContain('class="counter__digit"');
  });
});
