import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import PixelBlast, { type PixelBlastProps } from './pixel-blast';

// jsdom has no WebGL, so each test draws on a stand-in context. It keeps what
// the component asked of it: the shader sources, each uniform's values and
// what was created and deleted. Shaders compile and programs link unless a
// test says otherwise, and a program reports the uniforms its fragment shader
// declares, arrays as `name[0]` as a real driver does.

const VERTEX_SHADER = 0x8b31;
const FRAGMENT_SHADER = 0x8b30;
const COMPILE_STATUS = 0x8b81;
const LINK_STATUS = 0x8b82;
const ACTIVE_UNIFORMS = 0x8b86;

interface Shader {
  type: number;
  source: string;
}

interface Program {
  id: number;
  shaders: Shader[];
}

// A uniform's location: the program it's in, counted from 1 in the order
// they're created (the cell pass, then the main pass), and its name.
interface Location {
  program: number;
  name: string;
}

interface GlOptions {
  // Shader kinds createShader can't make, as when the context is lost.
  noShader?: number[];
  // Shader kinds that fail to compile.
  badShader?: number[];
  noProgram?: boolean;
  // Programs, counted from 1, that fail to link.
  badProgram?: number[];
  // One more active uniform than the shader declares, which the driver
  // can't describe: getActiveUniform returns null for it.
  phantomUniform?: boolean;
  // Whether the WEBGL_lose_context extension is there.
  loseContext?: boolean;
}

type Method = (...args: never[]) => unknown;

const uniformsOf = (program: Program) =>
  program.shaders
    .filter((shader) => shader.type === FRAGMENT_SHADER)
    .flatMap((shader) =>
      [...shader.source.matchAll(/^uniform \w+ (\w+)(\[\d+\])?;$/gm)].map(
        ([, name, array]) => (array ? `${name}[0]` : name),
      ),
    );

const fakeGl = (options: GlOptions = {}) => {
  let programs = 0;
  const loseContext = vi.fn();
  const behaviour: Record<string, Method> = {
    createShader: (type: number) =>
      options.noShader?.includes(type) ? null : { type, source: '' },
    shaderSource: (shader: Shader, source: string) => {
      shader.source = source;
    },
    getShaderParameter: (shader: Shader, parameter: number) =>
      parameter === COMPILE_STATUS && !options.badShader?.includes(shader.type),
    createProgram: () =>
      options.noProgram ? null : { id: (programs += 1), shaders: [] },
    attachShader: (program: Program, shader: Shader) => {
      program.shaders.push(shader);
    },
    getProgramParameter: (program: Program, parameter: number) =>
      parameter === LINK_STATUS
        ? !options.badProgram?.includes(program.id)
        : parameter === ACTIVE_UNIFORMS &&
          uniformsOf(program).length + (options.phantomUniform ? 1 : 0),
    getActiveUniform: (program: Program, index: number) => {
      const name = uniformsOf(program)[index];
      return name === undefined ? null : { name };
    },
    getUniformLocation: (program: Program, name: string): Location => ({
      program: program.id,
      name,
    }),
    getExtension: (name: string) =>
      name === 'WEBGL_lose_context' && options.loseContext !== false
        ? { loseContext }
        : null,
    createVertexArray: () => ({ kind: 'vertex array' }),
    createTexture: () => ({ kind: 'texture' }),
    createFramebuffer: () => ({ kind: 'framebuffer' }),
  };
  const inert = [
    'compileShader',
    'linkProgram',
    'deleteShader',
    'deleteProgram',
    'bindTexture',
    'texParameteri',
    'texImage2D',
    'bindFramebuffer',
    'framebufferTexture2D',
    'viewport',
    'disable',
    'useProgram',
    'activeTexture',
    'uniform1i',
    'uniform1f',
    'uniform2f',
    'uniform2i',
    'uniform1fv',
    'uniform2fv',
    'uniform3fv',
    'uniformMatrix3fv',
    'bindVertexArray',
    'drawArrays',
    'clearColor',
    'clear',
    'deleteTexture',
    'deleteFramebuffer',
    'deleteVertexArray',
  ];
  const mocks = Object.fromEntries([
    ...Object.entries(behaviour).map(([name, method]) => [name, vi.fn(method)]),
    ...inert.map((name) => [name, vi.fn()]),
  ]) as Record<string, ReturnType<typeof vi.fn>>;
  const context = {
    VERTEX_SHADER,
    FRAGMENT_SHADER,
    COMPILE_STATUS,
    LINK_STATUS,
    ACTIVE_UNIFORMS,
    ...mocks,
  };
  vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue(
    context as unknown as WebGL2RenderingContext,
  );
  return { mocks, loseContext };
};

let gl: ReturnType<typeof fakeGl>;

const SETTERS = [
  'uniform1i',
  'uniform1f',
  'uniform2f',
  'uniform2i',
  'uniform1fv',
  'uniform2fv',
  'uniform3fv',
  'uniformMatrix3fv',
];

// The arguments the component last gave a uniform, after its location.
const uniform = (name: string, pass: 'cell' | 'main' = 'main') => {
  const program = pass === 'cell' ? 1 : 2;
  return SETTERS.flatMap((setter) => gl.mocks[setter].mock.calls)
    .filter(([location]) => {
      const { program: of, name: called } = location as Location;
      return of === program && called === name;
    })
    .at(-1)
    ?.slice(1);
};

// Animation frames run when a test steps them, 50 ms apart unless it says
// otherwise, so a test controls exactly how much time passes.
const frames = new Map<number, FrameRequestCallback>();
let lastFrame = 0;
let now = 0;

const tick = (ms = 50) => {
  now += ms;
  const due = [...frames.values()];
  frames.clear();
  due.forEach((callback) => callback(now));
};

const ticks = (count: number, ms?: number) => {
  for (let i = 0; i < count; i += 1) tick(ms);
};

// Steps frames until the component stops asking for them, and says how many
// that took.
const settle = () => {
  let count = 0;
  while (frames.size && count < 5000) {
    tick();
    count += 1;
  }
  return count;
};

// jsdom has neither observer. These keep their callbacks, so a test can say
// when the container is resized or scrolled out of view.
class FakeResizeObserver {
  static last: FakeResizeObserver;
  callback: () => void;
  observe = vi.fn();
  disconnect = vi.fn();
  constructor(callback: () => void) {
    this.callback = callback;
    FakeResizeObserver.last = this;
  }
}

class FakeIntersectionObserver {
  static last: FakeIntersectionObserver;
  callback: (entries: Partial<IntersectionObserverEntry>[]) => void;
  observe = vi.fn();
  disconnect = vi.fn();
  constructor(callback: FakeIntersectionObserver['callback']) {
    this.callback = callback;
    FakeIntersectionObserver.last = this;
  }
}

const scrollInView = (isIntersecting: boolean) =>
  FakeIntersectionObserver.last.callback([{ isIntersecting }]);

const reduceMotion = () =>
  vi.stubGlobal('matchMedia', () => ({ matches: true }));

// The container is 200 by 100 CSS pixels, 10 from the left of the viewport
// and 20 from the top.
const RECT = { left: 10, top: 20, width: 200, height: 100 };

const at = (x: number, y: number) => ({
  clientX: RECT.left + x,
  clientY: RECT.top + y,
});

const field = () => screen.getByLabelText('PixelBlast interactive background');
const canvasOf = () => field().querySelector('canvas')!;

// Still settings: nothing moves unless the visitor does something, so the
// component stops asking for frames.
const STILL: PixelBlastProps = { speed: 0, drift: 0, twinkle: 0, intro: false };

beforeEach(() => {
  frames.clear();
  lastFrame = 0;
  now = 0;
  vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => {
    lastFrame += 1;
    frames.set(lastFrame, callback);
    return lastFrame;
  });
  vi.stubGlobal('cancelAnimationFrame', (id: number) => frames.delete(id));
  vi.stubGlobal('ResizeObserver', FakeResizeObserver);
  vi.stubGlobal('IntersectionObserver', FakeIntersectionObserver);
  vi.stubGlobal('devicePixelRatio', 1);
  vi.spyOn(HTMLElement.prototype, 'clientWidth', 'get').mockReturnValue(200);
  vi.spyOn(HTMLElement.prototype, 'clientHeight', 'get').mockReturnValue(100);
  vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockReturnValue(
    RECT as DOMRect,
  );
  // A zero from the random source starts the pattern at time 0, so the
  // uniforms are predictable.
  vi.spyOn(crypto, 'getRandomValues').mockImplementation((array) => array);
  gl = fakeGl();
});

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

describe('setting up WebGL', () => {
  it('stays an empty container without WebGL 2', () => {
    vi.mocked(HTMLCanvasElement.prototype.getContext).mockReturnValue(null);

    const { rerender } = render(<PixelBlast className="field" />);
    rerender(<PixelBlast className="field" speed={1} />);

    expect(field()).toHaveClass('pixel-blast-container field', {
      exact: true,
    });
    expect(canvasOf()).toHaveClass('pixel-blast-canvas');
    expect(canvasOf()).toHaveAttribute('aria-hidden', 'true');
    expect(frames.size).toBe(0);
  });

  it('asks for a WebGL 2 context without antialiasing, depth or stencil', () => {
    render(<PixelBlast />);

    expect(HTMLCanvasElement.prototype.getContext).toHaveBeenCalledWith(
      'webgl2',
      {
        alpha: true,
        premultipliedAlpha: true,
        antialias: false,
        depth: false,
        stencil: false,
      },
    );
    expect(field()).toHaveClass('pixel-blast-container', { exact: true });
  });

  it.each<[string, GlOptions]>([
    ["the vertex shader can't be created", { noShader: [VERTEX_SHADER] }],
    ["the fragment shader doesn't compile", { badShader: [FRAGMENT_SHADER] }],
    ["the program can't be created", { noProgram: true }],
    ["the cell pass doesn't link", { badProgram: [1] }],
    ["the main pass doesn't link", { badProgram: [2] }],
  ])('gives up when %s', (_, options) => {
    gl = fakeGl(options);

    render(<PixelBlast />);

    expect(gl.mocks.createVertexArray).not.toHaveBeenCalled();
    expect(gl.mocks.drawArrays).not.toHaveBeenCalled();
    expect(frames.size).toBe(0);
  });

  it('deletes a shader that fails to compile', () => {
    gl = fakeGl({ badShader: [VERTEX_SHADER] });

    render(<PixelBlast />);

    expect(gl.mocks.deleteShader).toHaveBeenCalledWith(
      gl.mocks.createShader.mock.results[0].value,
    );
  });

  it('deletes a program that fails to link', () => {
    gl = fakeGl({ badProgram: [1] });

    render(<PixelBlast />);

    expect(gl.mocks.deleteProgram).toHaveBeenCalledWith(
      gl.mocks.createProgram.mock.results[0].value,
    );
  });

  it("skips a uniform the driver can't describe", () => {
    gl = fakeGl({ phantomUniform: true });

    render(<PixelBlast />);

    expect(gl.mocks.getActiveUniform).toHaveReturnedWith(null);
    expect(uniform('uShapeType')).toEqual([0]);
  });

  it('looks up array uniforms by their name without the [0]', () => {
    render(<PixelBlast />);

    expect(gl.mocks.getUniformLocation).toHaveBeenCalledWith(
      expect.objectContaining({ id: 1 }),
      'uClickPos',
    );
    expect(uniform('uClickPos', 'cell')).toEqual([
      new Float32Array(20).fill(-1),
    ]);
  });

  it('makes two cell textures to draw between, each with a framebuffer', () => {
    render(<PixelBlast />);

    expect(gl.mocks.createTexture).toHaveBeenCalledTimes(2);
    expect(gl.mocks.framebufferTexture2D).toHaveBeenCalledTimes(2);
  });
});

describe('settings', () => {
  it('draws with the defaults when given no props', () => {
    render(<PixelBlast />);

    const canvas = canvasOf();
    expect([canvas.width, canvas.height]).toEqual([200, 100]);
    const [r, g, b] = uniform('uColor')![0] as number[];
    expect([r * 255, g * 255, b * 255].map(Math.round)).toEqual([
      0xb4, 0x97, 0xcf,
    ]);
    expect(uniform('uAccent')).toEqual(uniform('uColor'));
    expect(uniform('uShapeType')).toEqual([0]);
    expect(uniform('uPixelSize')).toEqual([3]);
    expect(uniform('uScale', 'cell')).toEqual([2]);
    expect(uniform('uDensity', 'cell')).toEqual([1]);
    expect(uniform('uPixelJitter')).toEqual([0]);
    expect(uniform('uEnableRipples', 'cell')).toEqual([1]);
    expect(uniform('uRippleIntensity', 'cell')).toEqual([1]);
    expect(uniform('uRippleThickness', 'cell')).toEqual([0.1]);
    expect(uniform('uRippleSpeed', 'cell')).toEqual([0.3]);
    expect(uniform('uTransparent')).toEqual([1]);
    expect(uniform('uEdgeFade')).toEqual([0.5]);
    expect(uniform('uNoise')).toEqual([0]);
    expect(uniform('uSoftness')).toEqual([0.6]);
    expect(uniform('uSmoothing')).toEqual([1]);
    expect(uniform('uGlow')).toEqual([0.4]);
    expect(uniform('uTwinkle')).toEqual([0.25]);
    expect(uniform('uHover')).toEqual([2]);
    expect(uniform('uMouseRadius', 'cell')).toEqual([140]);
    expect(uniform('uIntro', 'cell')).toEqual([0]);
    expect(uniform('uLightMode')).toEqual([0]);
    expect(uniform('uTilted')).toEqual([0]);
    expect(uniform('uTime', 'cell')).toEqual([0]);
  });

  it('takes every setting from its props, clamped to what the shaders handle', () => {
    render(
      <PixelBlast
        variant="ring"
        pixelSize={0.25}
        color="#ff0000"
        accentColor="#0000ff"
        className="field"
        style={{ opacity: 0.5 }}
        patternScale={4}
        patternDensity={0.3}
        pixelSizeJitter={0.2}
        enableRipples={false}
        rippleIntensityScale={2}
        rippleThickness={0}
        rippleSpeed={0.6}
        autoPauseOffscreen={false}
        speed={2}
        transparent={false}
        edgeFade={3}
        noiseAmount={0.1}
        softness={-1}
        smoothing={5}
        glow={9}
        twinkle={7}
        drift={0}
        driftAngle={45}
        hoverEffect="grow"
        hoverRadius={2}
        autoRipple={-5}
        tiltX={0}
        tiltY={0}
        intro={false}
        variation={1}
        lightMode
        dpr={2}
      />,
    );

    expect(field()).toHaveClass('pixel-blast-container field', {
      exact: true,
    });
    expect(field()).toHaveStyle({ opacity: '0.5' });
    const canvas = canvasOf();
    expect([canvas.width, canvas.height]).toEqual([400, 200]);
    expect(uniform('uShapeType')).toEqual([5]);
    // Half a CSS pixel at the smallest, at two device pixels each.
    expect(uniform('uPixelSize')).toEqual([1]);
    expect(uniform('uColor')).toEqual([[1, 0, 0]]);
    expect(uniform('uAccent')).toEqual([[0, 0, 1]]);
    expect(uniform('uScale', 'cell')).toEqual([4]);
    expect(uniform('uDensity', 'cell')).toEqual([0.3]);
    expect(uniform('uPixelJitter')).toEqual([0.2]);
    expect(uniform('uEnableRipples', 'cell')).toEqual([0]);
    expect(uniform('uRippleIntensity', 'cell')).toEqual([2]);
    expect(uniform('uRippleThickness', 'cell')).toEqual([0.001]);
    expect(uniform('uRippleSpeed', 'cell')).toEqual([0.6]);
    expect(uniform('uTransparent')).toEqual([0]);
    expect(uniform('uEdgeFade')).toEqual([1]);
    expect(uniform('uNoise')).toEqual([0.1]);
    expect(uniform('uSoftness')).toEqual([0]);
    expect(uniform('uSmoothing')).toEqual([1]);
    expect(uniform('uGlow')).toEqual([2]);
    expect(uniform('uTwinkle')).toEqual([1]);
    expect(uniform('uHover')).toEqual([1]);
    expect(uniform('uHover', 'cell')).toEqual([1]);
    // At least 10 CSS pixels, at two device pixels each.
    expect(uniform('uMouseRadius', 'cell')).toEqual([20]);
    expect(uniform('uIntro', 'cell')).toEqual([1]);
    expect(uniform('uLightMode')).toEqual([1]);
    expect(uniform('uTilted')).toEqual([0]);
    // A variation fixes where in the pattern it starts: 37 per step.
    expect(uniform('uTime', 'cell')).toEqual([37]);
  });

  it("falls back to squares and no hover effect for names it doesn't know", () => {
    render(
      <PixelBlast
        variant={'hexagon' as PixelBlastProps['variant']}
        hoverEffect={'sparkle' as PixelBlastProps['hoverEffect']}
      />,
    );

    expect(uniform('uShapeType')).toEqual([0]);
    expect(uniform('uHover')).toEqual([0]);
  });

  it('starts at a random point in the pattern without a variation', () => {
    vi.mocked(crypto.getRandomValues).mockImplementation((array) => {
      (array as Uint32Array)[0] = 0xffffffff / 4;
      return array;
    });

    render(<PixelBlast />);

    expect(uniform('uTime', 'cell')![0]).toBeCloseTo(250);
  });

  it('falls back to Math.random without crypto.getRandomValues', () => {
    vi.stubGlobal('crypto', undefined);
    vi.spyOn(Math, 'random').mockReturnValue(0.5);

    render(<PixelBlast />);

    expect(uniform('uTime', 'cell')).toEqual([500]);
  });

  it.each([
    ['#fff', [1, 1, 1]],
    ['#0f08', [0, 1, 0]],
    ['#123456', [0x12 / 255, 0x34 / 255, 0x56 / 255]],
    ['#33669980', [0x33 / 255, 0x66 / 255, 0x99 / 255]],
    // An empty colour is white, and so is anything that isn't hex.
    ['', [1, 1, 1]],
    ['zzz', [1, 1, 1]],
  ])('reads the colour %j as hex', (color, rgb) => {
    render(<PixelBlast color={color} />);

    expect(uniform('uColor')).toEqual([rgb]);
    // The accent is the colour unless it's given.
    expect(uniform('uAccent')).toEqual([rgb]);
  });

  it('keeps each colour it has read, and forgets them all past 64', () => {
    const { rerender } = render(<PixelBlast />);
    const draw = (color: string) => {
      rerender(<PixelBlast color={color} />);
      tick();
      return uniform('uColor')![0];
    };
    const many = (from: number) => {
      for (let i = 0; i < 70; i += 1)
        draw(`#${(from + i).toString(16).padStart(6, '0')}`);
    };
    // Other tests' colours are in the cache too, so fill it past its limit
    // first: it's then nearly empty, whatever came before.
    many(0x100000);

    const first = draw('#010203');
    draw('#010204');

    expect(draw('#010203')).toBe(first);

    many(0x200000);

    const again = draw('#010203');
    expect(again).not.toBe(first);
    expect(again).toEqual(first);
  });
});

describe('sizing', () => {
  it("follows the screen's pixel ratio, up to 2", () => {
    vi.stubGlobal('devicePixelRatio', 3);

    render(<PixelBlast />);

    expect([canvasOf().width, canvasOf().height]).toEqual([400, 200]);
  });

  it('draws at one device pixel per CSS pixel without a pixel ratio', () => {
    vi.stubGlobal('devicePixelRatio', 0);

    render(<PixelBlast />);

    expect([canvasOf().width, canvasOf().height]).toEqual([200, 100]);
  });

  it('is at least one pixel in each direction', () => {
    vi.mocked(
      Object.getOwnPropertyDescriptor(HTMLElement.prototype, 'clientWidth')!
        .get!,
    ).mockReturnValue(0);

    render(<PixelBlast />);

    expect(canvasOf().width).toBe(1);
  });

  it('lays out a cell for every 8 pixels across the canvas, and a cell over', () => {
    render(<PixelBlast />);

    // 200 by 100 at 3-pixel pixels, 24-pixel cells, a cell of margin each
    // side of the centre.
    expect(uniform('uOrigin', 'cell')).toEqual([-6, -4]);
    expect(uniform('uCellCount')).toEqual([12, 8]);
  });

  it('resizes the canvas with its container, keeping the pointer where it was', () => {
    render(<PixelBlast />);
    expect(FakeResizeObserver.last.observe).toHaveBeenCalledWith(field());
    expect(uniform('uMouse', 'cell')).toEqual([100, 50]);

    vi.mocked(
      Object.getOwnPropertyDescriptor(HTMLElement.prototype, 'clientWidth')!
        .get!,
    ).mockReturnValue(300);
    FakeResizeObserver.last.callback();

    expect(canvasOf().width).toBe(300);
    expect(uniform('uResolution', 'cell')).toEqual([300, 100]);
    expect(uniform('uMouse', 'cell')).toEqual([100, 50]);
    expect(uniform('uCellCount')).toEqual([16, 8]);
  });
});

describe('animation', () => {
  it('moves the pattern along at its speed, a frame at a time', () => {
    render(<PixelBlast speed={2} />);

    // The first frame after waking starts the clock.
    tick();
    expect(uniform('uTime', 'cell')).toEqual([0]);
    tick();
    expect(uniform('uTime', 'cell')![0]).toBeCloseTo(0.1);
    // A frame late by a second counts as a twentieth, so a stall doesn't
    // jump the pattern.
    tick(1000);
    expect(uniform('uTime', 'cell')![0]).toBeCloseTo(0.2);
    expect(uniform('uTime')).toEqual(uniform('uTime', 'cell'));
  });

  it('drifts the pattern at driftAngle', () => {
    render(<PixelBlast drift={2} driftAngle={90} />);

    ticks(3);

    const [x, y] = uniform('uDrift', 'cell') as number[];
    expect(x).toBeCloseTo(0);
    expect(y).toBeCloseTo(2 * 0.1 * 0.05);
  });

  it('opens with a ring spreading out from the centre over 1.8 seconds', () => {
    render(<PixelBlast />);

    // The clock starts on the first frame, and each frame draws before it
    // moves the intro on.
    ticks(3);
    expect(uniform('uIntro', 'cell')![0]).toBeCloseTo(0.05 / 1.8);

    ticks(40);
    expect(uniform('uIntro', 'cell')).toEqual([1]);
  });

  it('stops asking for frames once nothing has moved for 1.5 seconds', () => {
    const { rerender } = render(<PixelBlast {...STILL} />);

    expect(settle() * 0.05).toBeGreaterThanOrEqual(1.5);

    // A new prop wakes it.
    rerender(<PixelBlast {...STILL} color="#000000" />);
    expect(frames.size).toBe(1);
  });

  it.each<[string, PixelBlastProps]>([
    ['a speed', { speed: 0.1 }],
    ['a drift', { drift: 0.1 }],
    ['twinkling', { twinkle: 0.1 }],
    ['automatic ripples', { autoRipple: 3 }],
  ])('keeps going with %s', (_, props) => {
    render(<PixelBlast {...STILL} {...props} />);

    ticks(200);

    expect(frames.size).toBeGreaterThan(0);
  });

  it('stops with automatic ripples but ripples turned off', () => {
    render(<PixelBlast {...STILL} autoRipple={3} enableRipples={false} />);

    expect(settle()).toBeLessThan(40);
  });

  it('holds still for a visitor who prefers reduced motion', () => {
    reduceMotion();
    render(<PixelBlast drift={1} autoRipple={0.5} />);

    // No intro, no twinkle, and the pattern stays where it started.
    expect(uniform('uIntro', 'cell')).toEqual([1]);
    expect(settle()).toBeLessThan(40);
    expect(uniform('uTime', 'cell')).toEqual([0]);
    expect(uniform('uDrift', 'cell')).toEqual([0, 0]);
    expect(uniform('uTwinkle')).toEqual([0]);
    expect(uniform('uClickPos', 'cell')).toEqual([
      new Float32Array(20).fill(-1),
    ]);
  });

  it('pauses while scrolled out of view, unless autoPauseOffscreen is off', () => {
    const { rerender } = render(<PixelBlast />);
    expect(FakeIntersectionObserver.last.observe).toHaveBeenCalledWith(field());

    scrollInView(false);
    tick();
    expect(frames.size).toBe(0);

    // The pointer doesn't wake it while it's out of view.
    fireEvent.pointerMove(window, at(50, 50));
    expect(frames.size).toBe(0);

    rerender(<PixelBlast autoPauseOffscreen={false} />);
    expect(frames.size).toBe(1);
    tick();
    expect(frames.size).toBe(1);
  });

  it('starts again when scrolled back into view', () => {
    render(<PixelBlast />);
    scrollInView(false);
    tick();

    scrollInView(true);

    expect(frames.size).toBe(1);
  });

  it('pauses while the page is hidden, and starts again when it shows', () => {
    render(<PixelBlast />);
    const hidden = vi.spyOn(document, 'hidden', 'get').mockReturnValue(true);

    tick();
    expect(frames.size).toBe(0);
    fireEvent.pointerMove(window, at(50, 50));
    expect(frames.size).toBe(0);

    hidden.mockReturnValue(false);
    fireEvent(document, new Event('visibilitychange'));

    expect(frames.size).toBe(1);
  });
});

describe('the pointer', () => {
  it('brushes where the pointer is, following it smoothly', () => {
    render(<PixelBlast />);

    // Arriving, the brush jumps to the pointer, in device pixels from the
    // bottom left. It fades in from the first frame that has a time step.
    fireEvent.pointerMove(window, at(100, 50));
    tick();
    expect(uniform('uMouse', 'cell')).toEqual([100, 50]);
    expect(uniform('uMousePresence', 'cell')).toEqual([0]);

    tick();
    expect(uniform('uMousePresence', 'cell')![0]).toBeCloseTo(
      1 - Math.exp(-0.05 / 0.25),
    );

    // Moving, it eases toward the pointer.
    fireEvent.pointerMove(window, at(150, 10));
    tick();
    const follow = 1 - Math.exp(-0.05 / 0.08);
    const [x, y] = uniform('uMouse', 'cell') as number[];
    expect(x).toBeCloseTo(100 + 50 * follow);
    expect(y).toBeCloseTo(50 + 40 * follow);
  });

  it('fades the brush when the pointer leaves, by the edge or the page', () => {
    render(<PixelBlast />);
    fireEvent.pointerMove(window, at(100, 50));
    ticks(20);
    const inside = uniform('uMousePresence', 'cell')![0] as number;

    fireEvent.pointerMove(window, at(-5, 50));
    ticks(5);
    const outside = uniform('uMousePresence', 'cell')![0] as number;
    expect(outside).toBeLessThan(inside);

    fireEvent.pointerMove(window, at(100, 50));
    ticks(20);
    fireEvent.pointerLeave(document.documentElement);
    ticks(5);
    expect(uniform('uMousePresence', 'cell')![0]).toBeLessThan(inside);
  });

  it("doesn't wake for a pointer that stays outside", () => {
    render(<PixelBlast {...STILL} />);
    settle();

    fireEvent.pointerMove(window, at(250, 50));
    fireEvent.pointerMove(window, at(50, 150));
    expect(frames.size).toBe(0);

    fireEvent.pointerMove(window, at(50, 50));
    expect(frames.size).toBe(1);
  });

  it("lets the brush's trail fade only with a hover effect", () => {
    const { rerender } = render(<PixelBlast hoverEffect="none" />);
    tick();
    tick();
    expect(uniform('uDecay', 'cell')).toEqual([0]);

    rerender(<PixelBlast hoverEffect="reveal" />);
    tick();
    expect(uniform('uDecay', 'cell')![0]).toBeCloseTo(Math.exp(-0.05 / 0.45));
  });
});

describe('ripples', () => {
  it('ripples where the container is pressed', () => {
    render(<PixelBlast />);

    fireEvent.pointerDown(window, at(-1, 50));
    tick();
    expect(uniform('uClickPos', 'cell')).toEqual([
      new Float32Array(20).fill(-1),
    ]);

    fireEvent.pointerDown(window, at(30, 20));
    tick();
    const [positions] = uniform('uClickPos', 'cell') as [Float32Array];
    expect(Array.from(positions.slice(0, 4))).toEqual([30, 80, -1, -1]);
    const [ages] = uniform('uClickAge', 'cell') as [Float32Array];
    // Half the time since the press, in seconds.
    expect(ages[0]).toBeCloseTo(0.025);
  });

  it('remembers the last ten presses', () => {
    render(<PixelBlast />);

    for (let i = 0; i <= 10; i += 1) fireEvent.pointerDown(window, at(i, 0));
    tick();

    const [positions] = uniform('uClickPos', 'cell') as [Float32Array];
    expect(positions[0]).toBe(10);
    expect(positions[2]).toBe(1);
    expect(positions[18]).toBe(9);
  });

  it("doesn't ripple when ripples are off, or for reduced motion", () => {
    const { unmount } = render(<PixelBlast enableRipples={false} />);
    fireEvent.pointerDown(window, at(30, 20));
    tick();
    expect(uniform('uClickPos', 'cell')).toEqual([
      new Float32Array(20).fill(-1),
    ]);
    unmount();

    reduceMotion();
    render(<PixelBlast />);
    fireEvent.pointerDown(window, at(30, 20));
    tick();
    expect(uniform('uClickPos', 'cell')).toEqual([
      new Float32Array(20).fill(-1),
    ]);
  });

  it('keeps animating until the last ripple has spread', () => {
    render(<PixelBlast {...STILL} />);
    fireEvent.pointerDown(window, at(30, 20));

    // A ripple lasts while its age, half the time since the press, is under
    // 8: 16 seconds, 320 frames, then 1.5 seconds of quiet.
    expect(settle()).toBeGreaterThanOrEqual(350);
  });

  it('ripples by itself every autoRipple seconds or so, once the intro is over', () => {
    // The middle of each random range: the middle of the field, every
    // autoRipple seconds exactly.
    vi.spyOn(Math, 'random').mockReturnValue(0.5);
    render(<PixelBlast autoRipple={1} />);

    ticks(36);
    const [positions] = uniform('uClickPos', 'cell') as [Float32Array];
    expect(positions[0]).toBe(-1);

    // 1.2 seconds after the intro for the first.
    ticks(30);
    expect(Array.from(positions.slice(0, 2))).toEqual([100, 50]);
    expect(positions[2]).toBe(-1);

    ticks(25);
    expect(Array.from(positions.slice(2, 4))).toEqual([100, 50]);
  });
});

describe('tilt', () => {
  // The plane's matrix, as the shader gets it (by column), back by row.
  const plane = () => {
    const [transpose, columns] = uniform('uPlane') as [boolean, number[]];
    expect(transpose).toBe(false);
    return [0, 1, 2].flatMap((row) =>
      [0, 1, 2].map((column) => columns[column * 3 + row]),
    );
  };

  // A tilt turns the plane about the canvas's centre, so the centre stays
  // where it is.
  const expectCentreFixed = (m: number[]) => {
    const [x, y] = [100, 50];
    const z = m[6] * x + m[7] * y + m[8];
    expect((m[0] * x + m[1] * y + m[2]) / z).toBeCloseTo(x);
    expect((m[3] * x + m[4] * y + m[5]) / z).toBeCloseTo(y);
  };

  it.each([
    ['tiltX', { tiltX: 30 }],
    ['tiltY', { tiltY: -30 }],
  ])('tilts the plane when %s is set', (_, tilt) => {
    const { rerender } = render(<PixelBlast />);
    expect(uniform('uTilted')).toEqual([0]);
    expect(gl.mocks.uniformMatrix3fv).not.toHaveBeenCalled();

    rerender(<PixelBlast {...tilt} />);
    tick();

    expect(uniform('uTilted')).toEqual([1]);
    expectCentreFixed(plane());
  });

  it('tilts no further than 70 degrees', () => {
    const { rerender } = render(<PixelBlast tiltX={70} />);
    tick();
    const steepest = plane();

    rerender(<PixelBlast tiltX={89} />);
    tick();

    expect(plane()).toEqual(steepest);
  });

  it('lays cells out over all of the plane that a steep tilt brings into view', () => {
    // Square, so that tilted 70 degrees the top of the canvas is beyond the
    // horizon. (The plane's depth is 1.25 times the canvas's longer side, so
    // a wide canvas tilted toward its short side never gets that far.)
    vi.mocked(
      Object.getOwnPropertyDescriptor(HTMLElement.prototype, 'clientHeight')!
        .get!,
    ).mockReturnValue(200);
    render(<PixelBlast tiltX={70} />);

    // Flat, it's 12 by 12 cells from (-6, -6). Tilted, the near edge
    // stretches a row further down, and the rows beyond the horizon are left
    // out.
    expect(uniform('uOrigin', 'cell')).toEqual([-6, -7]);
    expect(uniform('uCellCount')).toEqual([12, 8]);

    // A press beyond the horizon lands nowhere on the plane; one in the
    // middle lands in the middle.
    fireEvent.pointerDown(window, at(100, 0));
    fireEvent.pointerDown(window, at(100, 100));
    tick();
    const [positions] = uniform('uClickPos', 'cell') as [Float32Array];
    expect(Array.from(positions.slice(0, 2))).toEqual([0, -1e6]);
    expect(positions[2]).toBeCloseTo(100);
    expect(positions[3]).toBeCloseTo(100);
  });

  it('keeps the flat layout for a tilt that is not a number', () => {
    // NaN is the one tilt with no plane to invert: every other angle up to
    // 70 degrees leaves the matrix invertible and some of the canvas in
    // front of the horizon. The component falls back on its flat layout.
    render(<PixelBlast tiltX={Number.NaN} tiltY={10} />);

    expect(uniform('uTilted')).toEqual([1]);
    expect(uniform('uOrigin', 'cell')).toEqual([-6, -4]);
    expect(uniform('uCellCount')).toEqual([12, 8]);
  });
});

describe('unmounting', () => {
  it('lets go of everything it took', () => {
    const add = vi.spyOn(window, 'addEventListener');
    const remove = vi.spyOn(window, 'removeEventListener');
    const addLeave = vi.spyOn(document.documentElement, 'addEventListener');
    const removeLeave = vi.spyOn(
      document.documentElement,
      'removeEventListener',
    );
    const addVisibility = vi.spyOn(document, 'addEventListener');
    const removeVisibility = vi.spyOn(document, 'removeEventListener');
    const { unmount } = render(<PixelBlast />);
    const resize = FakeResizeObserver.last;
    const intersection = FakeIntersectionObserver.last;
    const listeners = (spy: typeof add, types: string[]) =>
      spy.mock.calls
        .filter(([type]) => types.includes(type))
        .map(([type, listener]) => [type, listener]);

    unmount();

    expect(frames.size).toBe(0);
    expect(resize.disconnect).toHaveBeenCalled();
    expect(intersection.disconnect).toHaveBeenCalled();
    const pointer = ['pointermove', 'pointerdown'];
    expect(listeners(add, pointer)).toHaveLength(2);
    expect(listeners(remove, pointer)).toEqual(listeners(add, pointer));
    expect(listeners(removeLeave, ['pointerleave'])).toEqual(
      listeners(addLeave, ['pointerleave']),
    );
    expect(listeners(removeVisibility, ['visibilitychange'])).toEqual(
      listeners(addVisibility, ['visibilitychange']),
    );
    expect(gl.mocks.deleteTexture).toHaveBeenCalledTimes(2);
    expect(gl.mocks.deleteFramebuffer).toHaveBeenCalledTimes(2);
    expect(gl.mocks.deleteVertexArray).toHaveBeenCalledOnce();
    expect(gl.mocks.deleteProgram).toHaveBeenCalledTimes(2);
    expect(gl.loseContext).toHaveBeenCalledOnce();
  });

  it('unmounts without WEBGL_lose_context', () => {
    gl = fakeGl({ loseContext: false });
    const { unmount } = render(<PixelBlast />);

    unmount();

    expect(gl.mocks.getExtension).toHaveBeenCalledWith('WEBGL_lose_context');
    expect(gl.loseContext).not.toHaveBeenCalled();
  });
});
