import BrowserOnly from '@docusaurus/BrowserOnly';
import { useColorMode } from '@docusaurus/theme-common';

import Dither from '../react-bits/dither';
import styles from './desktop-field.module.css';

// The desktop pattern, alive: React Bits' Dither shader drawing slow waves in
// ink and paper only, ordered-dithered like a one-bit screen's greys. The
// pointer erases a patch of it as it passes, and a click sends a ripple.
// Server rendering and browsers without WebGL 2 get the classic static 50%
// pattern instead.
const DesktopField = () => {
  const { colorMode } = useColorMode();
  const palette =
    colorMode === 'dark' ? ['#000000', '#ffffff'] : ['#ffffff', '#000000'];

  return (
    <div className={styles.frame} aria-hidden="true">
      <div className={styles.field}>
        <BrowserOnly>
          {() => (
            <Dither
              palette={palette}
              ditherMode="bayer"
              pixelSize={2}
              waveSpeed={0.025}
              waveFrequency={2.4}
              waveAmplitude={0.32}
              contrast={0.85}
              mouseEffect="erase"
              mouseRadius={0.35}
              mouseStrength={1.2}
              trail={0.6}
              clickRipple
            />
          )}
        </BrowserOnly>
      </div>
    </div>
  );
};

export default DesktopField;
