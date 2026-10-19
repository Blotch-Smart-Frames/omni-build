import type { ComponentProps, ReactNode } from 'react';
import { act, fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import iconStyles from '../one-bit/desktop-icon.module.css';
import {
  carryOutline,
  followOutline,
  prefersReducedMotion,
  wait,
  zoomRects,
} from '../one-bit/xor';
import PixelTransition from '../react-bits/pixel-transition';
import Hero from './hero';
import styles from './hero.module.css';

// Docusaurus provides the colour mode from the site's theme. Each test says
// which mode the visitor is in.
const colorMode = vi.hoisted(() => ({ current: 'light' }));
vi.mock('@docusaurus/theme-common', () => ({
  useColorMode: () => ({ colorMode: colorMode.current }),
}));

// Docusaurus's Link needs the site's router. This one is a plain anchor that
// marks a page of the site, which the real one routes to without a reload.
vi.mock('@docusaurus/Link', () => ({
  default: ({ to, href, ...props }: { to?: string; href?: string }) => (
    <a href={to ?? href} data-to={to} {...props} />
  ),
}));

// The demonstration is a script of pauses and outlines. Each pause is held
// until the test lets it go, so a test can stop the script at any point and
// see what the page shows there. The outlines are xor's to draw and test.
vi.mock('../one-bit/xor', () => ({
  prefersReducedMotion: vi.fn(),
  wait: vi.fn(),
  carryOutline: vi.fn(),
  zoomRects: vi.fn(),
  followOutline: vi.fn(),
}));

// The animations draw on canvases and run on frames jsdom doesn't have, and
// each has its own test. These stand-ins show what they were given.
vi.mock('../react-bits/counter', () => ({
  default: ({ value, suffix }: { value: number; suffix: string }) => (
    <span data-testid="counter">
      {value}
      {suffix}
    </span>
  ),
}));
vi.mock('../react-bits/decrypted-text', () => ({
  default: ({ text }: { text: string }) => <>{text}</>,
}));
vi.mock('../react-bits/pixel-transition', () => ({
  default: vi.fn(
    ({
      active,
      firstContent,
      secondContent,
    }: {
      active: boolean;
      firstContent: ReactNode;
      secondContent: ReactNode;
    }) => (
      <div data-testid="phone">{active ? secondContent : firstContent}</div>
    ),
  ),
}));

// The real window, but for one test: the demonstration copes with its
// window never handing over its node, which a real window always does.
const windowRef = vi.hoisted(() => ({ kept: true }));
vi.mock('../one-bit/window', async (importOriginal) => {
  const { default: Window } =
    await importOriginal<typeof import('../one-bit/window')>();
  return {
    default: ({ ref, ...props }: ComponentProps<typeof Window>) => (
      <Window {...props} ref={windowRef.kept ? ref : undefined} />
    ),
  };
});

const REPOSITORY = 'https://github.com/Blotch-Smart-Frames/omni-build';

// jsdom lays nothing out, so each element the demonstration measures gets a
// box of its own: the two icons side by side, the window below them.
const TIDES = { left: 0, top: 0, right: 48, bottom: 60, width: 48, height: 60 };
const OMNI = {
  left: 100,
  top: 0,
  right: 148,
  bottom: 60,
  width: 48,
  height: 60,
};
const DEMO = {
  left: 0,
  top: 100,
  right: 400,
  bottom: 400,
  width: 400,
  height: 300,
};
const PACKAGE = {
  left: 200,
  top: 0,
  right: 248,
  bottom: 60,
  width: 48,
  height: 60,
};

let pauses: { ms: number; resolve: () => void }[] = [];
let observers: {
  callback: IntersectionObserverCallback;
  options?: IntersectionObserverInit;
  observe: ReturnType<typeof vi.fn>;
  disconnect: ReturnType<typeof vi.fn>;
}[] = [];

beforeEach(() => {
  vi.mocked(prefersReducedMotion).mockReturnValue(false);
  vi.mocked(wait).mockImplementation(
    (ms) =>
      new Promise<void>((resolve) => {
        pauses.push({ ms, resolve });
      }),
  );
  vi.mocked(carryOutline).mockResolvedValue(undefined);
  vi.mocked(zoomRects).mockResolvedValue(undefined);
  vi.mocked(followOutline).mockImplementation(() => ({
    move: vi.fn(),
    remove: vi.fn(),
  }));
  vi.stubGlobal(
    'IntersectionObserver',
    class {
      callback: IntersectionObserverCallback;
      options?: IntersectionObserverInit;
      observe = vi.fn();
      disconnect = vi.fn();
      constructor(
        callback: IntersectionObserverCallback,
        options?: IntersectionObserverInit,
      ) {
        this.callback = callback;
        this.options = options;
        observers.push(this);
      }
    },
  );
  // jsdom has no font loading.
  Object.defineProperty(document, 'fonts', {
    value: { ready: Promise.resolve() },
    configurable: true,
  });
  vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(
    function (this: HTMLElement) {
      const box =
        this.tagName === 'SECTION'
          ? DEMO
          : this.textContent === 'Tides'
            ? TIDES
            : this.textContent === 'OmniBuild'
              ? OMNI
              : PACKAGE;
      return box as DOMRect;
    },
  );
});

afterEach(() => {
  pauses = [];
  observers = [];
  colorMode.current = 'light';
  windowRef.kept = true;
  vi.clearAllMocks();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  Reflect.deleteProperty(document, 'fonts');
});

// Lets the script's promises and React's renders run until the script is
// held at its next pause.
const settle = () =>
  act(async () => {
    for (let i = 0; i < 20; i += 1) await Promise.resolve();
  });

// Ends the pause the script is held at, which must be the one expected.
const elapse = async (ms: number) => {
  const pause = pauses.shift();
  expect(pause?.ms).toBe(ms);
  pause!.resolve();
  await settle();
};

const tides = () => screen.getByRole('button', { name: 'Tides' });
const omni = () =>
  screen.getByRole('button', { name: 'OmniBuild: run the demonstration' });
const demo = () => screen.queryByRole('region', { name: 'Repackage Tides' });
const closeDemo = () =>
  fireEvent.click(
    screen.getByRole('button', { name: 'Close Repackage Tides' }),
  );
const runAgain = () => screen.getByRole('button', { name: 'Run again' });
const progress = () =>
  screen.getByRole('progressbar', { name: 'Repackaging Tides' });
const steps = () => within(demo()!).getAllByRole('listitem');
// What the phone screen says, line by line.
const phoneScreen = () =>
  [...screen.getByTestId('phone').querySelectorAll('span')].map(
    (line) => line.textContent,
  );
const current = () =>
  steps().findIndex((step) => step.classList.contains(styles.stepNow));
const ticked = () =>
  steps().filter((step) => step.querySelector(`.${styles.tick} svg`)).length;
const packages = () =>
  screen.queryAllByText(/^Tides\.(ipa|apk)$/).map((label) => label.textContent);
const isSelected = (icon: HTMLElement) =>
  icon.classList.contains(iconStyles.selected);

// Runs the script from a click on OmniBuild to the first step.
const startRun = async () => {
  fireEvent.click(omni());
  await settle();
  await elapse(450);
  await elapse(220);
  await elapse(16);
};

describe('the desktop', () => {
  it('opens on the OmniBuild window: the claim, what it means, and where to follow it', () => {
    render(<Hero />);

    expect(
      screen.getByRole('heading', {
        level: 1,
        name: 'iOS and Android apps, built with no native toolchain.',
      }),
    ).toBeInTheDocument();
    expect(
      screen.getByText(/^No Xcode, no JDK, no Android SDK\./),
    ).toBeInTheDocument();
    expect(
      screen.getByRole('link', { name: 'Watch on GitHub' }),
    ).toHaveAttribute('href', REPOSITORY);
    expect(screen.getByRole('link', { name: 'Read the plan' })).toHaveAttribute(
      'href',
      `${REPOSITORY}/issues/1`,
    );
    expect(
      screen.getByText('In planning. Nothing to install yet.'),
    ).toBeInTheDocument();
  });

  it('puts the docs and the plan on the desktop as documents', () => {
    render(<Hero />);

    expect(screen.getByRole('link', { name: 'Read me' })).toHaveAttribute(
      'data-to',
      '/docs',
    );
    expect(screen.getByRole('link', { name: 'The plan' })).toHaveAttribute(
      'href',
      `${REPOSITORY}/issues/1`,
    );
  });

  it('selects the Tides folder when it’s clicked', () => {
    render(<Hero />);

    fireEvent.click(tides());

    expect(isSelected(tides())).toBe(true);
    expect(isSelected(omni())).toBe(false);
  });
});

describe('the demonstration window', () => {
  it('starts on the shell, at 0%, with a notice that it’s a demonstration', () => {
    render(<Hero />);

    // First in the window, so it's read before the run.
    const notice = within(demo()!).getByText(
      'Demonstration: OmniBuild repackages Android shells, and iOS ones only for the Simulator, so far.',
    );
    expect(notice.parentElement!.firstElementChild).toBe(notice);
    expect(phoneScreen()).toEqual([
      'OmniBuildShell',
      'prebuilt in CI',
      'id: not set',
    ]);
    expect(
      steps().map((step) =>
        [styles.file, styles.detail].map(
          (part) => step.querySelector(`.${part}`)!.textContent,
        ),
      ),
    ).toEqual([
      ['OmniBuildShell', 'open the prebuilt shell'],
      ['Info.plist', 'CFBundleIdentifier = com.example.tides'],
      ['AndroidManifest.xml', 'package = com.example.tides'],
      ['Assets.car, resources.arsc', 'name Tides, and its icon'],
      ['main.lynx.bundle', 'add the payload'],
      ['Sign', 'Apple code signature, APK Signature Scheme v3'],
    ]);
    expect(current()).toBe(-1);
    expect(ticked()).toBe(0);
    expect(progress()).toHaveAttribute('aria-valuenow', '0');
    expect(progress()).not.toHaveClass(styles.busy);
    expect(screen.getByTestId('counter')).toHaveTextContent('0%');
    expect(runAgain()).toBeEnabled();
    expect(packages()).toEqual([]);
  });

  it('dissolves the phone screen in ink on paper, or paper on ink in dark mode', () => {
    const { unmount } = render(<Hero />);

    expect(vi.mocked(PixelTransition).mock.lastCall![0]).toMatchObject({
      pixelColor: '#000',
      gridSize: 14,
      aspectRatio: '9 / 16',
    });

    unmount();
    colorMode.current = 'dark';
    render(<Hero />);

    expect(vi.mocked(PixelTransition).mock.lastCall![0].pixelColor).toBe(
      '#fff',
    );
  });

  it('closes into the OmniBuild icon, and the icon reopens it', async () => {
    render(<Hero />);

    closeDemo();

    expect(demo()).toBeNull();
    expect(zoomRects).toHaveBeenCalledWith(DEMO, OMNI);

    fireEvent.click(omni());
    await settle();
    await elapse(450);
    await elapse(220);

    expect(demo()).toBeInTheDocument();
  });
});

describe('a run', () => {
  it('carries Tides to OmniBuild, opens the window out of it, works through each step and writes both packages', async () => {
    render(<Hero />);

    fireEvent.click(omni());
    await settle();

    // Tides is picked up...
    expect(isSelected(tides())).toBe(true);
    expect(runAgain()).toBeDisabled();
    expect(progress()).toHaveClass(styles.busy);

    await elapse(450);

    // ...carried onto OmniBuild...
    expect(carryOutline).toHaveBeenCalledWith(TIDES, OMNI);
    expect(isSelected(omni())).toBe(true);
    expect(isSelected(tides())).toBe(false);

    await elapse(220);
    await elapse(16);

    // ...which opens its window.
    expect(zoomRects).toHaveBeenCalledWith(OMNI, DEMO);
    expect(isSelected(omni())).toBe(false);
    expect(current()).toBe(0);

    // Each step in turn, ticked when it's done.
    await elapse(500);
    expect(current()).toBe(1);
    expect(ticked()).toBe(1);
    expect(progress()).toHaveAttribute('aria-valuenow', '17');

    await elapse(700);
    expect(phoneScreen()[0]).toBe('OmniBuildShell');

    // The name and icon step turns the shell into the app.
    await elapse(700);
    expect(current()).toBe(3);
    expect(progress()).toHaveAttribute('aria-valuenow', '50');
    expect(phoneScreen()).toEqual(['Tides', 'com.example.tides', '2.4.0 (41)']);

    await elapse(700);
    await elapse(700);
    await elapse(700);

    expect(current()).toBe(-1);
    expect(ticked()).toBe(6);
    expect(progress()).toHaveAttribute('aria-valuenow', '100');
    expect(screen.getByTestId('counter')).toHaveTextContent('100%');
    expect(screen.getByText('Wrote Tides.ipa and Tides.apk.')).toHaveAttribute(
      'aria-live',
      'polite',
    );
    expect(packages()).toEqual([]);

    await elapse(250);

    expect(packages()).toEqual(['Tides.ipa', 'Tides.apk']);

    await elapse(16);

    // Each package zooms out of the window.
    expect(zoomRects).toHaveBeenCalledWith(DEMO, PACKAGE, { count: 6 });
    expect(zoomRects).toHaveBeenCalledTimes(3);
    expect(runAgain()).toBeEnabled();
    expect(progress()).not.toHaveClass(styles.busy);
    expect(pauses).toEqual([]);
  });

  it('runs again from the window’s Run again button', async () => {
    render(<Hero />);

    fireEvent.click(runAgain());
    await settle();

    expect(isSelected(tides())).toBe(true);
    expect(pauses.map((pause) => pause.ms)).toEqual([450]);
  });

  it('jumps straight to the end when the visitor prefers reduced motion', async () => {
    vi.mocked(prefersReducedMotion).mockReturnValue(true);
    render(<Hero />);
    closeDemo();

    fireEvent.click(omni());
    await settle();

    expect(demo()).toBeInTheDocument();
    expect(ticked()).toBe(6);
    expect(packages()).toEqual(['Tides.ipa', 'Tides.apk']);
    expect(screen.getByText('Wrote Tides.ipa and Tides.apk.')).toBeVisible();
    expect(isSelected(tides())).toBe(false);
    expect(pauses).toEqual([]);
    expect(carryOutline).not.toHaveBeenCalled();
  });
});

describe('stopping a run', () => {
  // Closing the window stops the run, wherever it's held. Each test closes
  // it at one pause and checks the script goes no further when the pause
  // ends.

  it('before Tides is carried', async () => {
    render(<Hero />);
    fireEvent.click(omni());
    await settle();

    closeDemo();
    await elapse(450);

    expect(carryOutline).not.toHaveBeenCalled();
    expect(pauses).toEqual([]);
  });

  it('while Tides is carried', async () => {
    let land = () => {};
    vi.mocked(carryOutline).mockReturnValue(
      new Promise<void>((resolve) => {
        land = resolve;
      }),
    );
    render(<Hero />);
    fireEvent.click(omni());
    await settle();
    await elapse(450);

    closeDemo();
    land();
    await settle();

    expect(isSelected(omni())).toBe(false);
    expect(pauses).toEqual([]);
  });

  it('before the window opens', async () => {
    render(<Hero />);
    fireEvent.click(omni());
    await settle();
    await elapse(450);

    closeDemo();
    await elapse(220);

    expect(demo()).toBeNull();
    expect(pauses).toEqual([]);
  });

  it('as the window opens', async () => {
    render(<Hero />);
    fireEvent.click(omni());
    await settle();
    await elapse(450);
    await elapse(220);

    // Closed before it's traced opening: there's nothing left to trace.
    closeDemo();
    vi.mocked(zoomRects).mockClear();
    await elapse(16);

    expect(zoomRects).not.toHaveBeenCalled();
    expect(pauses).toEqual([]);
  });

  it('while the window is traced opening', async () => {
    let opened = () => {};
    render(<Hero />);
    fireEvent.click(omni());
    await settle();
    await elapse(450);
    await elapse(220);
    vi.mocked(zoomRects).mockReturnValueOnce(
      new Promise<void>((resolve) => {
        opened = resolve;
      }),
    );
    await elapse(16);

    closeDemo();
    opened();
    await settle();

    expect(pauses).toEqual([]);
  });

  it('between steps', async () => {
    render(<Hero />);
    await startRun();

    closeDemo();
    await elapse(500);

    expect(pauses).toEqual([]);
  });

  it('before the packages are written', async () => {
    render(<Hero />);
    await startRun();
    for (const ms of [500, 700, 700, 700, 700, 700]) await elapse(ms);

    closeDemo();
    await elapse(250);

    expect(packages()).toEqual([]);
    expect(pauses).toEqual([]);
  });

  it('by starting another as the packages are written', async () => {
    render(<Hero />);
    await startRun();
    for (const ms of [500, 700, 700, 700, 700, 700, 250]) await elapse(ms);
    vi.mocked(zoomRects).mockClear();

    // The new run takes the packages away before the old one traces them.
    fireEvent.click(omni());
    await settle();
    await elapse(16);

    expect(packages()).toEqual([]);
    expect(zoomRects).not.toHaveBeenCalled();
    // The new run is the one still going.
    expect(runAgain()).toBeDisabled();
    expect(pauses.map((pause) => pause.ms)).toEqual([450]);
  });

  it('by leaving the page', async () => {
    const { unmount } = render(<Hero />);
    fireEvent.click(omni());
    await settle();

    unmount();
    await elapse(450);

    expect(carryOutline).not.toHaveBeenCalled();
  });
});

describe('playing by itself', () => {
  const observer = () => observers[0];
  const scrollIntoView = (isIntersecting: boolean) =>
    act(() => {
      observer().callback(
        [{ isIntersecting } as IntersectionObserverEntry],
        observer() as unknown as IntersectionObserver,
      );
    });

  it('runs once the demonstration is mostly in view and the fonts have loaded', async () => {
    render(<Hero />);

    expect(observers).toHaveLength(1);
    expect(observer().options).toEqual({ threshold: 0.4 });
    expect(observer().observe).toHaveBeenCalledWith(demo());

    scrollIntoView(false);
    await settle();

    expect(pauses).toEqual([]);

    scrollIntoView(true);
    await settle();

    expect(observer().disconnect).toHaveBeenCalled();
    await elapse(700);
    expect(isSelected(tides())).toBe(true);
    expect(pauses.map((pause) => pause.ms)).toEqual([450]);
  });

  it('runs only once', async () => {
    render(<Hero />);

    scrollIntoView(true);
    scrollIntoView(true);
    await settle();

    expect(pauses.map((pause) => pause.ms)).toEqual([700]);
  });

  it('stops watching when the page is left', () => {
    const { unmount } = render(<Hero />);

    unmount();

    expect(observer().disconnect).toHaveBeenCalled();
  });

  it('copes with the demonstration window never handing over its node', () => {
    windowRef.kept = false;
    render(<Hero />);

    expect(observers).toEqual([]);

    closeDemo();

    expect(demo()).toBeNull();
    expect(zoomRects).not.toHaveBeenCalled();
  });
});

describe('dragging Tides', () => {
  const press = (init: PointerEventInit = {}) =>
    fireEvent.pointerDown(tides(), {
      button: 0,
      clientX: 10,
      clientY: 10,
      ...init,
    });
  const moveTo = (clientX: number, clientY: number) =>
    fireEvent.pointerMove(window, { clientX, clientY });
  const releaseAt = (clientX: number, clientY: number) =>
    fireEvent.pointerUp(window, { clientX, clientY });
  const outline = () => vi.mocked(followOutline).mock.results[0].value;

  it('onto OmniBuild runs the job, without carrying Tides there again', async () => {
    render(<Hero />);

    press();
    expect(isSelected(tides())).toBe(true);

    // A twitch isn't a drag.
    moveTo(12, 11);
    expect(followOutline).not.toHaveBeenCalled();

    moveTo(60, 30);
    expect(followOutline).toHaveBeenCalledOnce();
    expect(followOutline).toHaveBeenCalledWith(TIDES);
    expect(outline().move).toHaveBeenLastCalledWith(50, 20);
    expect(isSelected(tides())).toBe(true);

    // Over OmniBuild, OmniBuild is the one selected.
    moveTo(120, 30);
    expect(followOutline).toHaveBeenCalledOnce();
    expect(outline().move).toHaveBeenLastCalledWith(110, 20);
    expect(isSelected(omni())).toBe(true);
    expect(isSelected(tides())).toBe(false);

    releaseAt(120, 30);
    await settle();

    expect(outline().remove).toHaveBeenCalled();
    expect(carryOutline).not.toHaveBeenCalled();
    expect(pauses.map((pause) => pause.ms)).toEqual([220]);
  });

  it('elsewhere puts it back', async () => {
    render(<Hero />);

    press();
    moveTo(60, 200);
    releaseAt(60, 200);
    await settle();

    expect(outline().remove).toHaveBeenCalled();
    expect(isSelected(tides())).toBe(false);
    expect(pauses).toEqual([]);

    // The desktop stops listening once it's dropped.
    moveTo(120, 30);
    expect(isSelected(omni())).toBe(false);
  });

  it('a press and release without a drag only selects it', () => {
    render(<Hero />);

    press();
    releaseAt(10, 10);
    moveTo(120, 30);

    expect(followOutline).not.toHaveBeenCalled();
    expect(isSelected(tides())).toBe(true);
  });

  it('only with the main button, and not during a run', async () => {
    render(<Hero />);

    press({ button: 2 });
    moveTo(120, 30);
    expect(followOutline).not.toHaveBeenCalled();
    expect(isSelected(tides())).toBe(false);

    fireEvent.click(omni());
    await settle();
    await elapse(450);
    press();
    moveTo(60, 30);

    expect(followOutline).not.toHaveBeenCalled();
  });
});
