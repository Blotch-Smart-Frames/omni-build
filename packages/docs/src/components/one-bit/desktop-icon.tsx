import type { HTMLAttributes, Ref } from 'react';
import Link from '@docusaurus/Link';
import clsx from 'clsx';

import PixelIcon, { type IconName } from './pixel-icon';
import styles from './desktop-icon.module.css';

// An icon on the desktop: a 16-pixel glyph at three times size over a
// paper label plate. Selected, both invert. It's a link when it opens a
// page, a button when it acts, and a plain figure when it only shows a file.
interface DesktopIconProps extends HTMLAttributes<HTMLElement> {
  icon: IconName;
  label: string;
  selected?: boolean;
  // A page of the site, or an address elsewhere.
  to?: string;
  href?: string;
  // Whichever element the icon renders as: the desktop only measures it.
  ref?: Ref<HTMLElement>;
}

const DesktopIcon = ({
  icon,
  label,
  selected = false,
  to,
  href,
  onClick,
  className,
  ref,
  ...rest
}: DesktopIconProps) => {
  const classes = clsx(styles.icon, selected && styles.selected, className);
  // The caller's ref holds an HTMLElement, which each of the elements below
  // is. React types a ref by its exact element, so it can't tell that a ref
  // for any HTMLElement takes a link, a button or a div.
  const anyRef = ref as Ref<
    HTMLAnchorElement & HTMLButtonElement & HTMLDivElement
  >;
  const content = (
    <>
      <PixelIcon name={icon} scale={3} className={styles.glyph} />
      <span className={styles.label}>{label}</span>
    </>
  );
  if (to || href) {
    return (
      <Link ref={anyRef} to={to} href={href} className={classes} {...rest}>
        {content}
      </Link>
    );
  }
  if (onClick) {
    return (
      <button
        ref={anyRef}
        type="button"
        className={classes}
        onClick={onClick}
        {...rest}
      >
        {content}
      </button>
    );
  }
  return (
    <div ref={anyRef} className={classes} {...rest}>
      {content}
    </div>
  );
};

export default DesktopIcon;
