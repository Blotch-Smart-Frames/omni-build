import Link from '@docusaurus/Link';
import clsx from 'clsx';

import styles from './button.module.css';

// A one-bit push button. The default button, the one Return would press,
// wears a thick outer ring; pressing inverts. Rendered as a link when it
// goes somewhere and as a button when it does something.
const Button = ({
  to,
  href,
  primary = false,
  className,
  children,
  ...rest
}) => {
  const classes = clsx(styles.button, primary && styles.primary, className);
  if (to || href) {
    return (
      <Link to={to} href={href} className={classes} {...rest}>
        {children}
      </Link>
    );
  }
  return (
    <button type="button" className={classes} {...rest}>
      {children}
    </button>
  );
};

export default Button;
