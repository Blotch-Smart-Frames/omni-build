import Link from '@docusaurus/Link';
import clsx from 'clsx';

import PixelIcon from './pixel-icon';
import styles from './desktop-icon.module.css';

// An icon on the desktop: a 16-pixel glyph at three times size over a
// paper label plate. Selected, both invert. It's a link when it opens a
// page, a button when it acts, and a plain figure when it only shows a file.
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
}) => {
  const classes = clsx(styles.icon, selected && styles.selected, className);
  const content = (
    <>
      <PixelIcon name={icon} scale={3} className={styles.glyph} />
      <span className={styles.label}>{label}</span>
    </>
  );
  if (to || href) {
    return (
      <Link ref={ref} to={to} href={href} className={classes} {...rest}>
        {content}
      </Link>
    );
  }
  if (onClick) {
    return (
      <button
        ref={ref}
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
    <div ref={ref} className={classes} {...rest}>
      {content}
    </div>
  );
};

export default DesktopIcon;
