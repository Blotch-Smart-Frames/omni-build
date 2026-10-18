import { render } from '@testing-library/react';
import {
  afterEach,
  beforeEach,
  describe,
  expect,
  it,
  vi,
  type Mock,
} from 'vitest';

import Dither, { type DitherMode, type DitherMouseEffect } from './dither';

// jsdom has no WebGL and no 2D canvas, and Dither is mostly a WebGL 2
// program. So each test gives it a stand-in context that compiles whatever it
// is given, reports the uniforms written in the shaders' source, and records
// the last value the component sends each uniform: the uniforms are what
// Dither draws with, so they're what the tests check.

const VERTEX_SHADER = 0x8b31;
const FRAGMENT_SHADER = 0x8b30;

// The calls Dither makes for their effect on the GPU, which the stand-in
// only records.
const RECORDED = [
  'compileShader',
  'deleteShader',
  'bindAttribLocation',
  'linkProgram',
  'deleteProgram',
  'createBuffer',
  'createVertexArray',
  'bindVertexArray',
  'bindBuffer',
  'bufferData',
  'enableVertexAttribArray',
  'vertexAttribPointer',
  'createTexture',
  'bindTexture',
  'texParameteri',
  'texImage2D',
  'createFramebuffer',
  'bindFramebuffer',
  'framebufferTexture2D',
  'viewport',
  'useProgram',
  'drawArrays',
  'activeTexture',
  'deleteBuffer',
  'deleteVertexArray',
  'deleteTexture',
  'deleteFramebuffer',
];

const UNIFORM_SETTERS = [
  'uniform1f',
  'uniform1i',
  'uniform2f',
  'uniform2i',
  'uniform3fv',
  'uniform4fv',
];

// What can go wrong on a real GPU, which no prop can bring about.
interface Failures {
  // The kind of shader the driver can't create.
  shader?: number;
  // Whether a shader with this source compiles.
  compiles?: (source: string) => boolean;
  program?: boolean;
  links?: boolean;
  // One more active uniform than the shaders declare, which the driver
  // can't describe.
  undescribed?: boolean;
  // No WEBGL_lose_context extension.
  noLoseContext?: boolean;
}

interface Shader {
  source: string;
}

interface Program {
  shaders: Shader[];
}

const fakeGl = (failures: Failures = {}) => {
  const values = new Map<string, unknown>();
  const loseContext = vi.fn();
  // Typed arrays are copied: Dither rewrites the same ones every frame.
  const record = (location: { name: string }, ...args: unknown[]) => {
    const [first] = args;
    values.set(
      location.name,
      args.length > 1
        ? args
        : typeof first === 'object'
          ? Array.from(first as ArrayLike<number>)
          : first,
    );
  };
  // As a driver reports them: an array by its first element, `name[0]`.
  const uniformsOf = (program: Program) =>
    program.shaders.flatMap(({ source }) =>
      [...source.matchAll(/uniform \w+ (\w+)(\[\d+\])?;/g)].map(
        ([, name, array]) => (array ? `${name}[0]` : name),
      ),
    );
  const fns: Record<string, Mock> = {
    ...Object.fromEntries(RECORDED.map((name) => [name, vi.fn()])),
    ...Object.fromEntries(UNIFORM_SETTERS.map((name) => [name, vi.fn(record)])),
    createShader: vi.fn((type: number) =>
      type === failures.shader ? null : { source: '' },
    ),
    shaderSource: vi.fn((shader: Shader, source: string) => {
      shader.source = source;
    }),
    getShaderParameter: vi.fn(
      (shader: Shader) => failures.compiles?.(shader.source) ?? true,
    ),
    createProgram: vi.fn(() =>
      failures.program === false ? null : { shaders: [] },
    ),
    attachShader: vi.fn((program: Program, shader: Shader) => {
      program.shaders.push(shader);
    }),
    getProgramParameter: vi.fn((program: Program, name: string) =>
      name === 'LINK_STATUS'
        ? (failures.links ?? true)
        : uniformsOf(program).length + (failures.undescribed ? 1 : 0),
    ),
    getActiveUniform: vi.fn((program: Program, index: number) => {
      const name = uniformsOf(program)[index];
      return name ? { name } : null;
    }),
    getUniformLocation: vi.fn((_program: Program, name: string) => ({
      name: name.replace('[0]', ''),
    })),
    getExtension: vi.fn(() =>
      failures.noLoseContext ? null : { loseContext },
    ),
  };
  const context = {
    VERTEX_SHADER,
    FRAGMENT_SHADER,
    COMPILE_STATUS: 'COMPILE_STATUS',
    LINK_STATUS: 'LINK_STATUS',
    ACTIVE_UNIFORMS: 'ACTIVE_UNIFORMS',
    ...fns,
  } as unknown as WebGL2RenderingContext;
  return { context, fns, values, loseContext };
};

// The 2D context Dither paints one pixel with to turn a CSS colour into RGB.
// Like a real one, it ignores a colour it can't parse and keeps the last.
const fake2d = () => {
  let colour = '#000000';
  return {
    clearRect: vi.fn(),
    fillRect: vi.fn(),
    get fillStyle() {
      return colour;
    },
    set fillStyle(value: string) {
      if (/^#[\da-f]{6}$/i.test(value)) colour = value;
    },
    getImageData: vi.fn(() => ({
      data: Uint8ClampedArray.from([
        ...(colour.slice(1).match(/../g) ?? []).map((hex) =>
          Number.parseInt(hex, 16),
        ),
        255,
      ]),
    })),
  };
};

let gl: ReturnType<typeof fakeGl> | null;
let canvas2d: ReturnType<typeof fake2d> | null;
let frames: Map<number, FrameRequestCallback>;
let clock: number;
let width: number;
let resizers: { callback: () => void; disconnect: Mock }[];
let watchers: {
  callback: (entries: { isIntersecting: boolean }[]) => void;
  disconnect: Mock;
}[];

// Runs the frame Dither asked for, `ms` after the last one.
const step = (ms = 16) => {
  clock += ms;
  const due = [...frames.values()];
  frames.clear();
  due.forEach((callback) => callback(clock));
};

const steps = (count: number, ms = 50) => {
  for (let i = 0; i < count; i += 1) step(ms);
};

const uniform = (name: string) => gl!.values.get(name);

const draws = () => gl!.fns.drawArrays.mock.calls.length;

// The field is 200 by 100 CSS pixels, 10 from the left of the viewport and
// 20 from the top.
const pointer = (type: string, x: number, y: number, pointerType = 'mouse') =>
  window.dispatchEvent(
    new PointerEvent(type, { clientX: x, clientY: y, pointerType }),
  );

const setIntersecting = (isIntersecting: boolean) =>
  watchers[0].callback([{ isIntersecting }]);

const reduceMotion = () =>
  vi.stubGlobal(
    'matchMedia',
    vi.fn(() => ({ matches: true })),
  );

beforeEach(() => {
  gl = fakeGl();
  canvas2d = fake2d();
  vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockImplementation(
    ((kind: string) =>
      (kind === 'webgl2' ? gl?.context : canvas2d) ?? null) as never,
  );

  frames = new Map();
  clock = 1000;
  let id = 0;
  vi.stubGlobal(
    'requestAnimationFrame',
    vi.fn((callback: FrameRequestCallback) => {
      id += 1;
      frames.set(id, callback);
      return id;
    }),
  );
  vi.stubGlobal(
    'cancelAnimationFrame',
    vi.fn((handle: number) => frames.delete(handle)),
  );

  resizers = [];
  watchers = [];
  const observer = (list: { callback: never; disconnect: Mock }[]) =>
    class {
      callback: never;
      observe = vi.fn();
      disconnect = vi.fn();
      constructor(callback: never) {
        this.callback = callback;
        list.push(this);
      }
    };
  vi.stubGlobal('ResizeObserver', observer(resizers as never));
  vi.stubGlobal('IntersectionObserver', observer(watchers as never));

  width = 200;
  vi.spyOn(Element.prototype, 'clientWidth', 'get').mockImplementation(
    () => width,
  );
  vi.spyOn(Element.prototype, 'clientHeight', 'get').mockReturnValue(100);
  vi.spyOn(Element.prototype, 'getBoundingClientRect').mockImplementation(
    () =>
      ({
        left: 10,
        top: 20,
        width,
        height: 100,
      }) as DOMRect,
  );
});

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

describe('without WebGL 2', () => {
  it("leaves an empty canvas for the page's own background to show through", () => {
    gl = null;

    const { container } = render(<Dither />);

    expect(container.firstChild).toHaveClass('dither-container', {
      exact: true,
    });
    expect(container.querySelector('canvas')).toHaveAttribute(
      'aria-hidden',
      'true',
    );
    expect(frames.size).toBe(0);
    expect(resizers).toEqual([]);
  });

  it.each<[string, Failures]>([
    ["the vertex shader can't be created", { shader: VERTEX_SHADER }],
    ["a fragment shader can't be created", { shader: FRAGMENT_SHADER }],
    [
      "the wave shader doesn't compile",
      { compiles: (s) => !s.includes('uWaveSpeed') },
    ],
    [
      "the dither shader doesn't compile",
      { compiles: (s) => !s.includes('uField') },
    ],
    ["there's no program to link them into", { program: false }],
    ["the program doesn't link", { links: false }],
  ])('draws nothing when %s', (_, failures) => {
    gl = fakeGl(failures);

    render(<Dither />);

    expect(draws()).toBe(0);
    expect(frames.size).toBe(0);
  });

  it("frees a shader that doesn't compile and a program that doesn't link", () => {
    gl = fakeGl({ compiles: (source) => !source.includes('uField') });
    render(<Dither />);
    expect(gl.fns.deleteShader).toHaveBeenCalled();

    gl = fakeGl({ links: false });
    render(<Dither />);
    expect(gl.fns.deleteProgram).toHaveBeenCalled();
  });
});

describe('drawing the field', () => {
  it('draws the wave into a field of cells, then dithers it onto the canvas', () => {
    const { container } = render(<Dither />);

    const canvas = container.querySelector('canvas')!;
    expect([canvas.width, canvas.height]).toEqual([200, 100]);
    // Two-pixel cells: a 100 by 50 field.
    expect(gl!.fns.texImage2D).toHaveBeenLastCalledWith(
      undefined,
      0,
      undefined,
      100,
      50,
      0,
      undefined,
      undefined,
      null,
    );
    expect(uniform('uResolution')).toEqual([200, 100]);
    expect(uniform('uCell')).toBe(2);
    expect(uniform('uFieldSize')).toEqual([100, 50]);
    expect(uniform('uCellDevice')).toBe(2);
    expect(uniform('uMode')).toBe(0);
    expect(uniform('uColorNum')).toBe(4);
    expect(uniform('uWave')).toEqual([0.5, 0.5, 0.5]);
    expect(uniform('uBackground')).toEqual([0, 0, 0]);
    expect(uniform('uTransparent')).toBe(0);
    expect(uniform('uPaletteCount')).toBe(0);
    expect(uniform('uEffect')).toBe(0);
    // The intro grows the field from the centre, from nothing, out to the
    // corners.
    expect(uniform('uIntro')).toBe(0);
    expect(uniform('uReach')).toBeCloseTo(0.5 * Math.hypot(2, 1) + 0.45);
    expect(draws()).toBe(2);
  });

  it('passes on its class and style', () => {
    const { container } = render(
      <Dither className="field" style={{ opacity: 0.5 }} />,
    );

    expect(container.firstChild).toHaveClass('dither-container field', {
      exact: true,
    });
    expect(container.firstChild).toHaveStyle({ opacity: '0.5' });
  });

  it("skips a uniform the driver can't describe", () => {
    gl = fakeGl({ undescribed: true });

    render(<Dither />);

    expect(uniform('uTime')).toBe(0);
    expect(uniform('uPaletteCount')).toBe(0);
  });

  it('keeps the settings in range', () => {
    render(
      <Dither
        colorNum={1}
        pixelSize={0.5}
        mouseRadius={0}
        scale={0}
        mouseStrength={-1}
        ditherMode="noise"
        mouseEffect="ripple"
      />,
    );

    expect(uniform('uColorNum')).toBe(2);
    expect(uniform('uCell')).toBe(1);
    expect(uniform('uRadius')).toBe(0.001);
    expect(uniform('uScale')).toBe(0.05);
    expect(uniform('uStrength')).toBe(0);
    expect(uniform('uMode')).toBe(6);
    expect(uniform('uEffect')).toBe(2);
  });

  it("falls back to Bayer dithering and erasing for a mode or effect it doesn't know", () => {
    render(
      <Dither
        ditherMode={'stipple' as DitherMode}
        mouseEffect={'smudge' as DitherMouseEffect}
      />,
    );

    expect(uniform('uMode')).toBe(0);
    expect(uniform('uEffect')).toBe(0);
  });

  it('brightens under the pointer for the reveal effect', () => {
    render(<Dither mouseEffect="reveal" />);

    expect(uniform('uEffect')).toBe(1);
  });

  it("renders at the screen's pixel ratio, up to 2, unless told one", () => {
    vi.stubGlobal('devicePixelRatio', 3);
    const { container, unmount } = render(<Dither />);
    expect(container.querySelector('canvas')).toHaveProperty('width', 400);
    // Still two CSS pixels a cell, four device pixels.
    expect(uniform('uCell')).toBe(2);
    expect(uniform('uCellDevice')).toBe(4);
    unmount();

    render(<Dither dpr={1.5} />);
    expect(document.querySelector('canvas')).toHaveProperty('width', 300);
  });

  it("assumes one device pixel to the CSS pixel when the browser doesn't say", () => {
    vi.stubGlobal('devicePixelRatio', 0);

    const { container } = render(<Dither />);

    expect(container.querySelector('canvas')).toHaveProperty('width', 200);
  });

  it('resizes the canvas with its container', () => {
    const { container } = render(<Dither />);

    width = 300;
    resizers[0].callback();

    expect(container.querySelector('canvas')).toHaveProperty('width', 300);
    expect(uniform('uFieldSize')).toEqual([150, 50]);
  });
});

describe('colours', () => {
  it("clamps an RGB triple to 0–1, reading what isn't a number as 0", () => {
    render(<Dither waveColor={[2, Number.NaN, -1]} />);

    expect(uniform('uWave')).toEqual([1, 0, 0]);
  });

  it('reads a CSS colour by painting it, in linear light', () => {
    render(<Dither waveColor="#ffffff" backgroundColor="#000000" />);

    expect(uniform('uWave')).toEqual([1, 1, 1]);
    expect(uniform('uBackground')).toEqual([0, 0, 0]);
  });

  it("reads a colour it can't parse as black, not as the last colour read", () => {
    render(<Dither waveColor="#ffffff" backgroundColor="paper" />);

    expect(uniform('uWave')).toEqual([1, 1, 1]);
    expect(uniform('uBackground')).toEqual([0, 0, 0]);
  });

  it('paints each colour once, however often it draws', () => {
    render(<Dither waveColor="#ffffff" backgroundColor="#000000" />);
    steps(3);

    expect(canvas2d!.getImageData).toHaveBeenCalledTimes(2);
  });

  it('reads every CSS colour as black without a 2D canvas to paint on', () => {
    canvas2d = null;

    render(<Dither waveColor="#ffffff" />);

    expect(uniform('uWave')).toEqual([0, 0, 0]);
  });

  it('draws the wave alone over a transparent background', () => {
    render(<Dither backgroundColor="transparent" />);

    expect(uniform('uTransparent')).toBe(1);
    expect(uniform('uBackground')).toEqual([0, 0, 0]);
  });

  it('dithers between up to 8 palette colours, skipping empty ones', () => {
    render(
      <Dither
        palette={['#ffffff', '', '#000000', ...Array(8).fill('#ffffff')]}
      />,
    );

    expect(uniform('uPaletteCount')).toBe(8);
    expect((uniform('uPalette') as number[]).slice(0, 9)).toEqual([
      1, 1, 1, 0, 0, 0, 1, 1, 1,
    ]);
  });
});

describe('animation', () => {
  it('keeps time with the frames it draws, at most 50 ms a frame', () => {
    render(<Dither />);

    // The first frame has no frame before it to measure from.
    step(500);
    expect(uniform('uTime')).toBeCloseTo(1 / 60);
    step(20);
    expect(uniform('uTime')).toBeCloseTo(1 / 60 + 0.02);
    step(500);
    expect(uniform('uTime')).toBeCloseTo(1 / 60 + 0.07);
    step(0.1);
    expect(uniform('uTime')).toBeCloseTo(1 / 60 + 0.0705);
    expect(frames.size).toBe(1);
  });

  it('grows the field in over 1.8 seconds, then leaves it whole', () => {
    render(<Dither />);

    step();
    const first = uniform('uIntro') as number;
    expect(first).toBeGreaterThan(0);
    expect(first).toBeLessThan(0.01);
    steps(36);
    expect(uniform('uIntro')).toBe(1);
    steps(2);
    expect(uniform('uIntro')).toBe(1);
  });

  it('shows the whole field at once without the intro', () => {
    render(<Dither intro={false} />);

    expect(uniform('uIntro')).toBe(1);
    step();
    expect(uniform('uIntro')).toBe(1);
  });

  it('stops asking for frames once a still field has nothing left to do', () => {
    render(<Dither disableAnimation intro={false} />);

    step();

    expect(uniform('uTime')).toBe(0);
    expect(frames.size).toBe(0);
  });

  it('holds still, whole and without ripples for a visitor who prefers reduced motion', () => {
    reduceMotion();
    render(<Dither />);
    expect(uniform('uIntro')).toBe(1);

    pointer('pointerdown', 160, 45);
    pointer('pointermove', 160, 45);
    step();

    expect(uniform('uTime')).toBe(0);
    expect(uniform('uIntro')).toBe(1);
    expect(uniform('uRippleCount')).toBe(0);
    expect(uniform('uTrailCount')).toBe(0);
    expect(frames.size).toBe(0);
  });

  it("pauses while the page is hidden and starts afresh when it's back", () => {
    const hidden = vi
      .spyOn(Document.prototype, 'hidden', 'get')
      .mockReturnValue(false);
    render(<Dither />);
    step();
    const drawn = draws();

    hidden.mockReturnValue(true);
    step(50);
    expect(draws()).toBe(drawn);
    expect(frames.size).toBe(0);
    document.dispatchEvent(new Event('visibilitychange'));
    expect(frames.size).toBe(0);

    hidden.mockReturnValue(false);
    document.dispatchEvent(new Event('visibilitychange'));
    expect(frames.size).toBe(1);
    const before = uniform('uTime') as number;
    step(5000);
    expect(uniform('uTime')).toBeCloseTo(before + 1 / 60);
  });

  it('pauses while scrolled out of view', () => {
    render(<Dither />);

    setIntersecting(false);
    step();
    expect(draws()).toBe(2);
    expect(frames.size).toBe(0);
    pointer('pointermove', 160, 45);
    expect(frames.size).toBe(0);

    setIntersecting(true);
    expect(frames.size).toBe(1);
    step();
    expect(draws()).toBe(4);
  });

  it('asks for one frame at a time', () => {
    render(<Dither />);

    pointer('pointermove', 160, 45);
    pointer('pointerdown', 160, 45);

    expect(frames.size).toBe(1);
  });
});

describe('new props', () => {
  it('draws them with the next frame when one is coming', () => {
    const { rerender } = render(<Dither contrast={1} />);

    rerender(<Dither contrast={2} />);
    expect(uniform('uContrast')).toBe(1);

    step();
    expect(uniform('uContrast')).toBe(2);
  });

  it('draws them at once on a still field, and animates again if they ask', () => {
    const { rerender } = render(<Dither disableAnimation intro={false} />);
    step();

    rerender(<Dither disableAnimation intro={false} contrast={2} />);
    expect(uniform('uContrast')).toBe(2);
    expect(frames.size).toBe(1);
  });

  it('reallocates the field for a new pixel size', () => {
    const { rerender } = render(<Dither />);

    rerender(<Dither pixelSize={4} />);
    step();

    expect(uniform('uFieldSize')).toEqual([50, 25]);
  });

  it('resizes the canvas for a new pixel ratio', () => {
    const { container, rerender } = render(<Dither dpr={1} />);

    rerender(<Dither dpr={2} />);

    expect(container.querySelector('canvas')).toHaveProperty('width', 400);
  });
});

describe('the pointer', () => {
  // (160, 45) on the page is (150, 25) in the field: in the field's own
  // units, half its height right of centre and a quarter of it up.
  const HERE = [0.5, 0.25];

  const head = () => (uniform('uTrail') as number[]).slice(0, 3);

  it('leaves a trail that starts where it is and grows stronger while it stays', () => {
    render(<Dither />);

    pointer('pointermove', 160, 45);
    step();
    expect(uniform('uTrailCount')).toBe(1);
    const [x, y, weight] = head();
    expect([x, y]).toEqual(HERE);
    expect(weight).toBeGreaterThan(0);

    step(50);
    expect(uniform('uTrailCount')).toBe(1);
    expect(head()[2]).toBeGreaterThan(weight);
  });

  it('adds a point to the trail as it moves, easing after the pointer', () => {
    render(<Dither />);
    pointer('pointermove', 160, 45);
    step();

    pointer('pointermove', 60, 45);
    step(50);

    expect(uniform('uTrailCount')).toBe(2);
    const [x] = head();
    expect(x).toBeLessThan(HERE[0]);
    expect(x).toBeGreaterThan(-0.5);
  });

  it('keeps the 16 most recent points of the trail', () => {
    render(<Dither trail={2} />);

    for (let i = 0; i < 24; i += 1) {
      pointer('pointermove', i % 2 ? 40 : 180, 45);
      step(50);
    }

    expect(uniform('uTrailCount')).toBe(16);
  });

  it('leaves no trail behind it with trail at 0', () => {
    render(<Dither trail={0} />);

    for (let i = 0; i < 4; i += 1) {
      pointer('pointermove', i % 2 ? 40 : 180, 45);
      step(50);
    }

    expect(uniform('uTrailCount')).toBe(1);
  });

  it('fades the trail out once it leaves the page, then stops', () => {
    render(<Dither disableAnimation intro={false} />);
    pointer('pointermove', 160, 45);
    steps(5);

    document.documentElement.dispatchEvent(new PointerEvent('pointerleave'));
    steps(100);

    expect(uniform('uTrailCount')).toBe(0);
    expect(frames.size).toBe(0);
  });

  it('lets go when a touch lifts, but not when a mouse button does', () => {
    render(<Dither />);
    pointer('pointermove', 160, 45);
    steps(2);

    pointer('pointerup', 160, 45, 'mouse');
    const held = head()[2];
    steps(2);
    expect(head()[2]).toBeGreaterThan(held);

    pointer('pointercancel', 160, 45, 'touch');
    const lifted = head()[2];
    steps(2);
    expect(head()[2]).toBeLessThan(lifted);
  });

  it('leaves no trail with mouse interaction off', () => {
    render(<Dither enableMouseInteraction={false} />);

    pointer('pointermove', 160, 45);
    steps(3);

    expect(uniform('uTrailCount')).toBe(0);
  });

  it('ignores a pointer outside the field', () => {
    render(<Dither />);

    pointer('pointermove', 500, 45);
    pointer('pointerdown', 500, 45);
    steps(3);

    expect(uniform('uTrailCount')).toBe(0);
    expect(uniform('uRippleCount')).toBe(0);
  });

  it('sends a ripple from where the visitor clicks, which dies out in 1.6 seconds', () => {
    render(<Dither />);

    pointer('pointerdown', 160, 45);
    step();
    expect(uniform('uRippleCount')).toBe(1);
    expect((uniform('uRipples') as number[]).slice(0, 4)).toEqual([
      0.5,
      0.25,
      Math.fround(1 / 60),
      1,
    ]);

    steps(32);
    expect(uniform('uRippleCount')).toBe(0);
  });

  it('keeps the 4 newest ripples, newest first', () => {
    render(<Dither />);

    for (const x of [20, 60, 100, 140, 180]) pointer('pointerdown', x, 45);
    step();

    expect(uniform('uRippleCount')).toBe(4);
    // (180, 45) on the page is 170 across the field: 0.7 right of centre.
    expect((uniform('uRipples') as number[])[0]).toBeCloseTo(0.7);
  });

  it('sends no ripple with click ripples off', () => {
    render(<Dither clickRipple={false} />);

    pointer('pointerdown', 160, 45);
    step();

    expect(uniform('uRippleCount')).toBe(0);
  });
});

describe('unmounting', () => {
  it('stops drawing, lets go of the page and frees the GPU', () => {
    const removed = vi.spyOn(window, 'removeEventListener');
    const { unmount } = render(<Dither />);

    unmount();

    expect(frames.size).toBe(0);
    expect(resizers[0].disconnect).toHaveBeenCalled();
    expect(watchers[0].disconnect).toHaveBeenCalled();
    expect(removed.mock.calls.map(([type]) => type)).toEqual([
      'pointermove',
      'pointerdown',
      'pointerup',
      'pointercancel',
    ]);
    expect(gl!.fns.deleteProgram).toHaveBeenCalledTimes(2);
    expect(gl!.fns.deleteBuffer).toHaveBeenCalled();
    expect(gl!.fns.deleteVertexArray).toHaveBeenCalled();
    expect(gl!.fns.deleteTexture).toHaveBeenCalled();
    expect(gl!.fns.deleteFramebuffer).toHaveBeenCalled();
    expect(gl!.loseContext).toHaveBeenCalled();

    pointer('pointermove', 160, 45);
    document.dispatchEvent(new Event('visibilitychange'));
    expect(frames.size).toBe(0);
  });

  it("doesn't need the extension that loses the context early", () => {
    gl = fakeGl({ noLoseContext: true });
    const { unmount } = render(<Dither />);

    expect(unmount).not.toThrow();
    expect(gl.fns.deleteProgram).toHaveBeenCalledTimes(2);
  });
});
