import { act, fireEvent, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import Window from './window';
import styles from './window.module.css';
import { zoomRects } from './xor';

// The zoom rectangles are xor's to draw and test; here it's enough to know
// the window asked for them, and between which boxes.
vi.mock('./xor', () => ({ zoomRects: vi.fn() }));

// The shade traces the window's two sizes a frame after it re-lays out, so
// each test runs that frame when it chooses.
let frames: FrameRequestCallback[] = [];
const runFrames = () => {
  const pending = frames;
  frames = [];
  pending.forEach((callback) => callback(0));
};

beforeEach(() => {
  vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => {
    frames.push(callback);
    return frames.length;
  });
  // jsdom has no pointer capture.
  HTMLElement.prototype.setPointerCapture = vi.fn();
});

afterEach(() => {
  frames = [];
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
  vi.mocked(zoomRects).mockReset();
  Reflect.deleteProperty(HTMLElement.prototype, 'setPointerCapture');
});

const titleBar = () => document.querySelector(`.${styles.titleBar}`)!;
const section = () => document.querySelector('section')!;

describe('Window', () => {
  it('frames its content under a title, with a status line', () => {
    render(
      <Window
        title="Install"
        titleId="install"
        status="Not published yet"
        className="extra"
        bodyClassName="body"
        style={{ color: 'red' }}
        data-testid="window"
      >
        <p>npm install</p>
      </Window>,
    );

    const window = screen.getByTestId('window');
    expect(window.tagName).toBe('SECTION');
    expect(window).toHaveClass(`${styles.window} extra`, { exact: true });
    expect(window).toHaveStyle({ color: 'rgb(255, 0, 0)' });
    expect(window).toHaveAccessibleName('Install');
    // A plain paragraph unless the window is a section of the outline.
    const title = screen.getByText('Install');
    expect(title.tagName).toBe('P');
    expect(title).toHaveAttribute('id', 'install');
    expect(screen.getByText('npm install').parentElement).toHaveClass(
      `${styles.body} body`,
      { exact: true },
    );
    expect(screen.getByText('Not published yet')).toHaveClass(styles.status);
  });

  it('titles a section of the outline with a heading', () => {
    render(
      <Window title="Roadmap" titleAs="h2" inactive>
        Phases
      </Window>,
    );

    expect(screen.getByRole('heading', { level: 2 })).toHaveTextContent(
      'Roadmap',
    );
    expect(section()).toHaveClass(`${styles.window} ${styles.inactive}`, {
      exact: true,
    });
    expect(document.querySelector(`.${styles.status}`)).toBeNull();
  });

  it('hands its node to a ref object or a ref callback', () => {
    const object = { current: null as HTMLElement | null };
    const callback = vi.fn();
    const { unmount } = render(
      <>
        <Window title="One" ref={object} />
        <Window title="Two" ref={callback} />
      </>,
    );

    const [one, two] = document.querySelectorAll('section');
    expect(object.current).toBe(one);
    expect(callback).toHaveBeenLastCalledWith(two);

    unmount();

    expect(object.current).toBeNull();
    expect(callback).toHaveBeenLastCalledWith(null);
  });
});

describe('the close box', () => {
  it('closes the window when the page can bring it back', async () => {
    const onClose = vi.fn();
    render(<Window title="Repackage Tides" onClose={onClose} />);

    await userEvent.click(
      screen.getByRole('button', { name: 'Close Repackage Tides' }),
    );

    expect(onClose).toHaveBeenCalledOnce();
    expect(screen.queryByRole('button', { name: /^Roll/ })).toBeNull();
  });

  it('takes its name from the caller, or calls a titled-by-markup window a window', () => {
    render(
      <>
        <Window title="Demo" onClose={() => {}} closeLabel="Stop the demo" />
        <Window title={<em>Demo</em>} onClose={() => {}} />
      </>,
    );

    expect(
      screen.getByRole('button', { name: 'Stop the demo' }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole('button', { name: 'Close window' }),
    ).toBeInTheDocument();
  });

  it('otherwise rolls the window up to its title bar and back down', async () => {
    render(
      <Window title="How it works" status="6 values">
        The table
      </Window>,
    );
    const body = screen.getByText('The table');
    const status = screen.getByText('6 values');

    await userEvent.click(
      screen.getByRole('button', { name: 'Roll up How it works' }),
    );

    const shade = screen.getByRole('button', {
      name: 'Roll down How it works',
    });
    expect(shade).toHaveAttribute('aria-expanded', 'false');
    expect(body).not.toBeVisible();
    expect(status).not.toBeVisible();

    await userEvent.click(shade);

    expect(shade).toHaveAccessibleName('Roll up How it works');
    expect(shade).toHaveAttribute('aria-expanded', 'true');
    expect(body).toBeVisible();
    expect(status).toBeVisible();
  });

  it('traces the window rolling between its two sizes once it has re-laid out', async () => {
    const tall = { left: 0, top: 0, width: 300, height: 200 } as DOMRect;
    const short = { left: 0, top: 0, width: 300, height: 24 } as DOMRect;
    render(<Window title="Roadmap">Phases</Window>);
    const measure = vi
      .spyOn(section(), 'getBoundingClientRect')
      .mockReturnValueOnce(tall)
      .mockReturnValueOnce(short);

    await userEvent.click(
      screen.getByRole('button', { name: 'Roll up Roadmap' }),
    );

    expect(zoomRects).not.toHaveBeenCalled();

    runFrames();

    expect(measure).toHaveBeenCalledTimes(2);
    expect(zoomRects).toHaveBeenCalledWith(tall, short, { count: 6 });
  });

  it('traces nothing if the window has gone by the next frame', async () => {
    const { unmount } = render(<Window title="Roadmap">Phases</Window>);

    await userEvent.click(
      screen.getByRole('button', { name: 'Roll up Roadmap' }),
    );
    unmount();
    runFrames();

    expect(zoomRects).not.toHaveBeenCalled();
  });
});

describe('dragging', () => {
  const press = (target: Element, init: PointerEventInit = {}) =>
    fireEvent.pointerDown(target, {
      pointerId: 1,
      pointerType: 'mouse',
      button: 0,
      clientX: 10,
      clientY: 10,
      ...init,
    });
  const ghost = () => document.querySelector(`.${styles.ghost}`);

  it('drags a dotted outline, and the window jumps there when let go', () => {
    render(
      <Window title="Before and after" draggable style={{ color: 'red' }}>
        Table
      </Window>,
    );
    expect(titleBar()).toHaveClass(`${styles.titleBar} ${styles.draggable}`, {
      exact: true,
    });

    press(titleBar());

    expect(titleBar().setPointerCapture).toHaveBeenCalledWith(1);
    expect(ghost()).toHaveAttribute('aria-hidden', 'true');
    expect(ghost()).toHaveStyle({ translate: '0px 0px' });

    fireEvent.pointerMove(titleBar(), { clientX: 50, clientY: 30 });

    // The outline follows; the window stays put until it's dropped.
    expect(ghost()).toHaveStyle({ translate: '40px 20px' });
    expect(section().style.translate).toBe('');

    fireEvent.pointerUp(titleBar());

    expect(ghost()).toBeNull();
    expect(section()).toHaveStyle({
      translate: '40px 20px',
      zIndex: '5',
      color: 'rgb(255, 0, 0)',
    });

    // A second drag starts from where the first left it.
    press(titleBar(), { clientX: 0, clientY: 0 });
    fireEvent.pointerMove(titleBar(), { clientX: -10, clientY: 5 });
    fireEvent.pointerCancel(titleBar());

    expect(section()).toHaveStyle({ translate: '30px 25px' });
  });

  it('drops in place if let go before the outline is drawn', () => {
    render(<Window title="Install" draggable />);

    // Both in one batch, so the release sees no outline yet.
    act(() => {
      press(titleBar());
      fireEvent.pointerUp(titleBar());
    });

    expect(ghost()).toBeNull();
    expect(section().style.translate).toBe('');
  });

  it('ignores the pointer unless it drags the title bar of a draggable window', () => {
    render(
      <>
        <Window title="Still" />
        <Window title="Movable" draggable />
      </>,
    );
    const [still, movable] = [
      ...document.querySelectorAll(`.${styles.titleBar}`),
    ];

    press(still);
    // Touch scrolls the page instead, and only the main button drags.
    press(movable, { pointerType: 'touch' });
    press(movable, { button: 2 });
    // The close box is a button in the title bar, not a handle.
    press(screen.getByRole('button', { name: 'Roll up Movable' }));
    fireEvent.pointerMove(movable, { clientX: 99, clientY: 99 });
    fireEvent.pointerUp(movable);

    expect(ghost()).toBeNull();
    expect(HTMLElement.prototype.setPointerCapture).not.toHaveBeenCalled();
    document.querySelectorAll('section').forEach((node) => {
      expect(node.style.translate).toBe('');
    });
  });
});
