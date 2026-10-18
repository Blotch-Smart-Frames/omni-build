import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import PixelIcon, { ICONS, type IconName } from './pixel-icon';

const paths = (svg: Element) => [...svg.querySelectorAll('path')];

describe('ICONS', () => {
  it.each(Object.entries(ICONS))(
    '%s is a rectangle of ink, paper and transparent pixels',
    (_, rows) => {
      // A ragged row would draw the glyph skewed; any other character would
      // be dropped from both paths, leaving a hole.
      expect(new Set(rows.map((row) => row.length)).size).toBe(1);
      expect(rows.join('')).toMatch(/^[#o.]+$/);
    },
  );
});

describe('PixelIcon', () => {
  it('draws one path for the paper and one for the ink, merging runs of a row', () => {
    const { container } = render(<PixelIcon name="close" />);

    const svg = container.querySelector('svg')!;
    // close has no paper, and its ink is two single pixels a row but for
    // the middle one.
    const [paper, ink] = paths(svg);
    expect(paper).toHaveAttribute('d', '');
    expect(paper).toHaveAttribute('fill', 'var(--paper)');
    expect(ink).toHaveAttribute('fill', 'var(--ink)');
    expect(ink.getAttribute('d')).toMatch(
      /^M0 0h1v1h-1zM6 0h1v1h-1zM1 1h1v1h-1z.*M3 3h1v1h-1z/,
    );
  });

  it('merges a run of same-coloured pixels into one rectangle', () => {
    const { container } = render(<PixelIcon name="shade" />);

    // shade is three full rows of ink, between rows of nothing.
    const [, ink] = paths(container.querySelector('svg')!);
    expect(ink).toHaveAttribute('d', 'M0 1h7v1h-7zM0 3h7v1h-7zM0 5h7v1h-7z');
  });

  it('draws the grid at three times size by default, crisply', () => {
    const { container } = render(<PixelIcon name="folder" className="glyph" />);

    const svg = container.querySelector('svg')!;
    expect(svg).toHaveAttribute('width', '48');
    expect(svg).toHaveAttribute('height', '48');
    expect(svg).toHaveAttribute('viewBox', '0 0 16 16');
    expect(svg).toHaveAttribute('shape-rendering', 'crispEdges');
    expect(svg).toHaveClass('glyph');
  });

  it('scales the grid by the given factor', () => {
    const { container } = render(<PixelIcon name="check" scale={2} />);

    const svg = container.querySelector('svg')!;
    expect(svg).toHaveAttribute('width', '16');
    expect(svg).toHaveAttribute('viewBox', '0 0 8 8');
  });

  it('is decoration, hidden from assistive technology, without a title', () => {
    const { container } = render(<PixelIcon name="omnibuild" />);

    const svg = container.querySelector('svg')!;
    expect(svg).toHaveAttribute('aria-hidden', 'true');
    expect(svg).not.toHaveAttribute('role');
    expect(svg).not.toHaveAttribute('aria-label');
  });

  it('is an image with that name when given a title', () => {
    render(<PixelIcon name="tides" title="Tides" />);

    const image = screen.getByRole('img', { name: 'Tides' });
    expect(image).not.toHaveAttribute('aria-hidden');
  });

  it.each(Object.keys(ICONS) as IconName[])('draws %s', (name) => {
    const { container } = render(<PixelIcon name={name} />);

    expect(paths(container.querySelector('svg')!)).toHaveLength(2);
  });
});
