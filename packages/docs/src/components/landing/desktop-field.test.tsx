import type { ReactNode } from 'react';
import { render } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';

import Dither from '../react-bits/dither';
import DesktopField from './desktop-field';
import styles from './desktop-field.module.css';

// Docusaurus provides the colour mode from the site's theme. Each test says
// which mode the visitor is in.
const colorMode = vi.hoisted(() => ({ current: 'light' }));
vi.mock('@docusaurus/theme-common', () => ({
  useColorMode: () => ({ colorMode: colorMode.current }),
}));

// BrowserOnly renders its children only once the page has hydrated, which
// it always has in a browser.
vi.mock('@docusaurus/BrowserOnly', () => ({
  default: ({ children }: { children: () => ReactNode }) => <>{children()}</>,
}));

// The shader needs WebGL 2, which jsdom hasn't got, and has its own test.
// Here it's enough to know what the field asks it to draw.
vi.mock('../react-bits/dither', () => ({ default: vi.fn(() => null) }));

const ditherProps = () => vi.mocked(Dither).mock.lastCall![0];

afterEach(() => {
  colorMode.current = 'light';
  vi.mocked(Dither).mockClear();
});

it('draws ink waves on paper, ordered-dithered like a one-bit screen', () => {
  render(<DesktopField />);

  expect(ditherProps()).toEqual({
    palette: ['#ffffff', '#000000'],
    ditherMode: 'bayer',
    pixelSize: 2,
    waveSpeed: 0.025,
    waveFrequency: 2.4,
    waveAmplitude: 0.32,
    contrast: 0.85,
    mouseEffect: 'erase',
    mouseRadius: 0.35,
    mouseStrength: 1.2,
    trail: 0.6,
    clickRipple: true,
  });
});

it('swaps ink and paper in dark mode', () => {
  colorMode.current = 'dark';

  render(<DesktopField />);

  expect(ditherProps().palette).toEqual(['#000000', '#ffffff']);
});

it('is decoration, hidden from assistive technology', () => {
  const { container } = render(<DesktopField />);

  const frame = container.firstElementChild!;
  expect(frame).toHaveClass(styles.frame, { exact: true });
  expect(frame).toHaveAttribute('aria-hidden', 'true');
  expect(frame.firstElementChild).toHaveClass(styles.field, { exact: true });
});
