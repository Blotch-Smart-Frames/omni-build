import {
  useRef,
  useState,
  type HTMLAttributes,
  type PointerEvent,
  type ReactNode,
  type Ref,
} from 'react';
import clsx from 'clsx';

import PixelIcon from './pixel-icon';
import styles from './window.module.css';
import { zoomRects } from './xor';

// A one-bit window: striped title bar, square close box, 2px frame and a
// hard ink shadow. A window can be dragged by its title bar, and moves the
// way it did on a one-bit desktop: a dotted outline follows the pointer and
// the window jumps there when it's let go, rather than repainting every
// frame.
//
// The close box closes the window when the page can bring it back (the
// demonstration reopens from its icon). Otherwise it rolls the window up to
// its title bar, the window-shade ritual, and a second press rolls it down:
// a section of the page is never lost to a click.
interface WindowProps extends Omit<
  HTMLAttributes<HTMLElement>,
  'title' | 'draggable'
> {
  title: ReactNode;
  // A heading when the window is a section of the page's outline.
  titleAs?: 'p' | 'h1' | 'h2' | 'h3' | 'h4' | 'h5' | 'h6';
  titleId?: string;
  status?: ReactNode;
  // Given when the page can bring the window back; see above.
  onClose?: () => void;
  closeLabel?: string;
  draggable?: boolean;
  inactive?: boolean;
  bodyClassName?: string;
  ref?: Ref<HTMLElement>;
}

interface Point {
  x: number;
  y: number;
}

const Window = ({
  title,
  titleAs: Title = 'p',
  titleId,
  status,
  onClose,
  closeLabel,
  draggable = false,
  inactive = false,
  className,
  bodyClassName,
  style,
  ref,
  children,
  ...rest
}: WindowProps) => {
  const [offset, setOffset] = useState<Point>({ x: 0, y: 0 });
  const [ghost, setGhost] = useState<Point | null>(null);
  const [rolledUp, setRolledUp] = useState(false);
  const startRef = useRef<Point | null>(null);
  const nodeRef = useRef<HTMLElement | null>(null);

  const setNode = (node: HTMLElement | null) => {
    nodeRef.current = node;
    if (typeof ref === 'function') ref(node);
    else if (ref) ref.current = node;
  };

  const onPointerDown = (event: PointerEvent<HTMLDivElement>) => {
    // Touch scrolls the page instead: a window that grabs every swipe on a
    // phone traps the reader.
    if (!draggable || event.pointerType === 'touch' || event.button !== 0) {
      return;
    }
    if ((event.target as Element).closest('button, a')) return;
    event.currentTarget.setPointerCapture(event.pointerId);
    startRef.current = { x: event.clientX, y: event.clientY };
    setGhost({ x: 0, y: 0 });
  };

  const onPointerMove = (event: PointerEvent<HTMLDivElement>) => {
    if (!startRef.current) return;
    setGhost({
      x: event.clientX - startRef.current.x,
      y: event.clientY - startRef.current.y,
    });
  };

  const onPointerUp = () => {
    if (!startRef.current) return;
    startRef.current = null;
    setOffset((current) => ({
      x: current.x + (ghost?.x ?? 0),
      y: current.y + (ghost?.y ?? 0),
    }));
    setGhost(null);
  };

  const toggleShade = () => {
    const before = nodeRef.current?.getBoundingClientRect();
    setRolledUp((current) => !current);
    // Trace the window rolling between its two sizes once it has re-laid
    // out.
    requestAnimationFrame(() => {
      const after = nodeRef.current?.getBoundingClientRect();
      if (before && after) zoomRects(before, after, { count: 6 });
    });
  };

  const moved = offset.x !== 0 || offset.y !== 0;
  const label = typeof title === 'string' ? title : 'window';

  return (
    <section
      ref={setNode}
      className={clsx(styles.window, inactive && styles.inactive, className)}
      style={{
        ...style,
        ...(moved && {
          translate: `${offset.x}px ${offset.y}px`,
          zIndex: 5,
        }),
      }}
      aria-labelledby={titleId}
      {...rest}
    >
      <div
        className={clsx(styles.titleBar, draggable && styles.draggable)}
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        onPointerCancel={onPointerUp}
      >
        <Title id={titleId} className={styles.title}>
          {title}
        </Title>
        {onClose ? (
          <button
            type="button"
            className={styles.closeBox}
            onClick={onClose}
            aria-label={closeLabel ?? `Close ${label}`}
          >
            <PixelIcon name="close" scale={2} />
          </button>
        ) : (
          <button
            type="button"
            className={styles.closeBox}
            onClick={toggleShade}
            aria-expanded={!rolledUp}
            aria-label={`${rolledUp ? 'Roll down' : 'Roll up'} ${label}`}
          >
            <PixelIcon name={rolledUp ? 'unshade' : 'shade'} scale={2} />
          </button>
        )}
      </div>
      <div className={clsx(styles.body, bodyClassName)} hidden={rolledUp}>
        {children}
      </div>
      {status && (
        <div className={styles.status} hidden={rolledUp}>
          {status}
        </div>
      )}
      {ghost && (
        <div
          className={styles.ghost}
          style={{ translate: `${ghost.x}px ${ghost.y}px` }}
          aria-hidden="true"
        />
      )}
    </section>
  );
};

export default Window;
