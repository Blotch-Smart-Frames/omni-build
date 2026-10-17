// The one-bit desktop's motion: outlines drawn by inverting the pixels under
// them (XOR), so they show on paper, ink and dither alike. Zoom rectangles
// trace a window opening out of an icon; a dotted outline stands in for an
// icon being dragged. Both are short-lived elements on top of the page.

export const prefersReducedMotion = () =>
  typeof window !== 'undefined' &&
  window.matchMedia('(prefers-reduced-motion: reduce)').matches;

const outline = (rect, style) => {
  const node = document.createElement('div');
  Object.assign(node.style, {
    position: 'fixed',
    left: `${rect.left}px`,
    top: `${rect.top}px`,
    width: `${rect.width}px`,
    height: `${rect.height}px`,
    border: `2px ${style} #fff`,
    mixBlendMode: 'difference',
    pointerEvents: 'none',
    zIndex: 1000,
  });
  document.body.append(node);
  return node;
};

const lerp = (a, b, t) => a + (b - a) * t;

const between = (from, to, t) => ({
  left: lerp(from.left, to.left, t),
  top: lerp(from.top, to.top, t),
  width: lerp(from.width, to.width, t),
  height: lerp(from.height, to.height, t),
});

// Eight rectangles from one box to another, each shown briefly in turn.
export const zoomRects = (from, to, { duration = 420, count = 8 } = {}) => {
  if (prefersReducedMotion()) return Promise.resolve();
  const step = duration / count;
  const nodes = Array.from({ length: count }, (_, i) =>
    outline(between(from, to, (i + 1) / count), 'solid'),
  );
  nodes.forEach((node) => {
    node.style.visibility = 'hidden';
  });
  return Promise.all(
    nodes.map(
      (node, i) =>
        new Promise((resolve) => {
          setTimeout(() => {
            node.style.visibility = 'visible';
          }, i * step);
          setTimeout(
            () => {
              node.remove();
              resolve();
            },
            i * step + step * 2.5,
          );
        }),
    ),
  );
};

// A dotted outline carried from one box to another, as if dragged.
export const carryOutline = (from, to, { duration = 900 } = {}) => {
  if (prefersReducedMotion()) return Promise.resolve();
  const node = outline(from, 'dotted');
  const animation = node.animate(
    [
      { translate: '0 0' },
      {
        translate: `${to.left - from.left + (to.width - from.width) / 2}px ${
          to.top - from.top + (to.height - from.height) / 2
        }px`,
      },
    ],
    { duration, easing: 'cubic-bezier(0.45, 0, 0.2, 1)', fill: 'forwards' },
  );
  return animation.finished.then(() => node.remove());
};

// An outline that follows the pointer while the visitor drags an icon.
export const followOutline = (rect) => {
  const node = outline(rect, 'dotted');
  return {
    move: (dx, dy) => {
      node.style.translate = `${dx}px ${dy}px`;
    },
    remove: () => node.remove(),
  };
};

export const wait = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
