import { useCallback, useEffect, useRef, useState } from 'react';
import { useColorMode } from '@docusaurus/theme-common';
import clsx from 'clsx';

import Button from '../one-bit/button';
import DesktopIcon from '../one-bit/desktop-icon';
import PixelIcon from '../one-bit/pixel-icon';
import Window from '../one-bit/window';
import {
  carryOutline,
  followOutline,
  prefersReducedMotion,
  wait,
  zoomRects,
} from '../one-bit/xor';
import Counter from '../react-bits/counter';
import DecryptedText from '../react-bits/decrypted-text';
import PixelTransition from '../react-bits/pixel-transition';
import styles from './hero.module.css';

const REPOSITORY = 'https://github.com/Blotch-Smart-Frames/omni-build';

// What the demonstration does to the example app, in the order OmniBuild's
// plan does it (issue #1): open the prebuilt shell, rewrite each platform's
// configuration, add the name, icon and payload, then sign.
const STEPS = [
  { file: 'OmniBuildShell', detail: 'open the prebuilt shell' },
  { file: 'Info.plist', detail: 'CFBundleIdentifier = com.example.tides' },
  { file: 'AndroidManifest.xml', detail: 'package = com.example.tides' },
  { file: 'Assets.car, resources.arsc', detail: 'name Tides, and its icon' },
  { file: 'main.lynx.bundle', detail: 'add the payload' },
  { file: 'Sign', detail: 'Apple code signature, APK Signature Scheme v3' },
];

// The step at which the phone screen turns from the shell into the app.
const BRANDED_AT = 3;

const rectOf = (node) => node?.getBoundingClientRect();

const ShellScreen = () => (
  <div className={styles.screen}>
    <PixelIcon name="omnibuild" scale={4} />
    <span className={styles.screenName}>OmniBuildShell</span>
    <span className={styles.screenMeta}>prebuilt in CI</span>
    <span className={clsx(styles.screenMeta, styles.unset)}>id: not set</span>
  </div>
);

const AppScreen = () => (
  <div className={styles.screen}>
    <PixelIcon name="tides" scale={4} />
    <span className={styles.screenName}>Tides</span>
    <span className={styles.screenMeta}>com.example.tides</span>
    <span className={styles.screenMeta}>2.4.0 (41)</span>
  </div>
);

const Hero = () => {
  // The pixel dissolve draws on a canvas, which can't read CSS variables.
  const { colorMode } = useColorMode();
  const [selected, setSelected] = useState(null);
  const [demoOpen, setDemoOpen] = useState(true);
  const [step, setStep] = useState(-1);
  const [outputs, setOutputs] = useState(false);
  const [running, setRunning] = useState(false);
  const runRef = useRef(0);
  const tidesRef = useRef(null);
  const omniRef = useRef(null);
  const demoRef = useRef(null);
  const ipaRef = useRef(null);
  const apkRef = useRef(null);

  const finish = useCallback(() => {
    setStep(STEPS.length);
    setOutputs(true);
    setSelected(null);
  }, []);

  // One run of the job. `fromDrop` skips the carried outline when the
  // visitor dragged the folder there themselves.
  const run = useCallback(
    async ({ fromDrop = false } = {}) => {
      const token = ++runRef.current;
      const live = () => token === runRef.current;
      if (prefersReducedMotion()) {
        setDemoOpen(true);
        finish();
        return;
      }
      setRunning(true);
      setOutputs(false);
      setStep(-1);
      if (!fromDrop) {
        setSelected('tides');
        await wait(450);
        if (!live()) return;
        await carryOutline(rectOf(tidesRef.current), rectOf(omniRef.current));
        if (!live()) return;
      }
      setSelected('omnibuild');
      await wait(220);
      if (!live()) return;
      setDemoOpen(true);
      // The window is mounted now; trace it opening out of the icon.
      await wait(16);
      if (demoRef.current) {
        await zoomRects(rectOf(omniRef.current), rectOf(demoRef.current));
      }
      if (!live()) return;
      setSelected(null);
      for (let i = 0; i < STEPS.length; i += 1) {
        setStep(i);
        await wait(i === 0 ? 500 : 700);
        if (!live()) return;
      }
      setStep(STEPS.length);
      await wait(250);
      if (!live()) return;
      setOutputs(true);
      await wait(16);
      const from = rectOf(demoRef.current);
      await Promise.all(
        [ipaRef, apkRef].map((ref) =>
          ref.current ? zoomRects(from, rectOf(ref.current), { count: 6 }) : 0,
        ),
      );
      if (live()) setRunning(false);
    },
    [finish],
  );

  // Play once when the desktop is first in view, after the fonts settle so
  // the zoom rectangles trace the window's real size.
  useEffect(() => {
    const node = demoRef.current;
    if (!node) return undefined;
    let started = false;
    const observer = new IntersectionObserver(
      ([entry]) => {
        if (!entry.isIntersecting || started) return;
        started = true;
        observer.disconnect();
        document.fonts.ready.then(() => wait(700)).then(() => run());
      },
      { threshold: 0.4 },
    );
    observer.observe(node);
    return () => {
      observer.disconnect();
      runRef.current += 1;
    };
  }, [run]);

  // Dragging the Tides folder onto OmniBuild runs the job, as dropping a
  // document on an application did.
  const onTidesPointerDown = (event) => {
    if (event.button !== 0 || running) return;
    const start = { x: event.clientX, y: event.clientY };
    const origin = rectOf(tidesRef.current);
    let drag = null;
    setSelected('tides');

    const onMove = (moveEvent) => {
      const dx = moveEvent.clientX - start.x;
      const dy = moveEvent.clientY - start.y;
      if (!drag && Math.hypot(dx, dy) < 4) return;
      drag ??= followOutline(origin);
      drag.move(dx, dy);
      const target = rectOf(omniRef.current);
      const over =
        moveEvent.clientX >= target.left &&
        moveEvent.clientX <= target.right &&
        moveEvent.clientY >= target.top &&
        moveEvent.clientY <= target.bottom;
      setSelected(over ? 'omnibuild' : 'tides');
    };

    const onUp = (upEvent) => {
      window.removeEventListener('pointermove', onMove);
      window.removeEventListener('pointerup', onUp);
      if (!drag) return;
      drag.remove();
      const target = rectOf(omniRef.current);
      const dropped =
        upEvent.clientX >= target.left &&
        upEvent.clientX <= target.right &&
        upEvent.clientY >= target.top &&
        upEvent.clientY <= target.bottom;
      if (dropped) run({ fromDrop: true });
      else setSelected(null);
    };

    window.addEventListener('pointermove', onMove);
    window.addEventListener('pointerup', onUp);
  };

  const progress = Math.round(
    (Math.min(Math.max(step, 0), STEPS.length) / STEPS.length) * 100,
  );
  const done = step >= STEPS.length;

  return (
    <div className={styles.desktop}>
      <Window
        title="OmniBuild"
        titleId="omnibuild-window"
        draggable
        className={styles.main}
        bodyClassName={styles.mainBody}
        status={
          <span>
            In planning. Nothing to install yet; AngularLynx is its first user.
          </span>
        }
      >
        <h1 className={styles.headline}>
          <DecryptedText
            text="iOS and Android apps, built with no native toolchain."
            animateOn="view"
            sequential
            revealDirection="start"
            speed={28}
            characters="▖▗▘▙▚▛▜▝▞▟█▄▀░▒▓"
          />
        </h1>
        <p className={styles.lede}>
          No Xcode, no JDK, no Android SDK. OmniBuild repackages a prebuilt app
          shell with your app&rsquo;s config and assets, then signs it: one
          binary, on Linux, Windows or macOS.
        </p>
        <div className={styles.actions}>
          <Button href={REPOSITORY} primary>
            Watch on GitHub
          </Button>
          <Button href={`${REPOSITORY}/issues/1`}>Read the plan</Button>
        </div>
      </Window>

      {demoOpen && (
        <Window
          ref={demoRef}
          title="Repackage Tides"
          titleId="demo-window"
          draggable
          className={styles.demo}
          bodyClassName={styles.demoBody}
          onClose={() => {
            runRef.current += 1;
            setRunning(false);
            const from = rectOf(demoRef.current);
            setDemoOpen(false);
            if (from) zoomRects(from, rectOf(omniRef.current));
          }}
          status={
            <>
              <span>Tides is an example app.</span>
              <button
                type="button"
                className={styles.again}
                onClick={() => run()}
                disabled={running}
              >
                Run again
              </button>
            </>
          }
        >
          {/* First, so it's read before the run: a demonstration must never
              pass for a release. */}
          <p className={styles.notice}>
            Demonstration: OmniBuild only repackages Android shells so far.
          </p>
          <PixelTransition
            className={styles.phone}
            active={step >= BRANDED_AT}
            firstContent={<ShellScreen />}
            secondContent={<AppScreen />}
            gridSize={14}
            pixelColor={colorMode === 'dark' ? '#fff' : '#000'}
            animationStepDuration={0.45}
            aspectRatio="9 / 16"
          />
          <div className={styles.job}>
            <ol className={styles.steps}>
              {STEPS.map((item, i) => (
                <li
                  key={item.file}
                  className={i === step ? styles.stepNow : undefined}
                >
                  <span className={styles.tick} aria-hidden="true">
                    {i < step && <PixelIcon name="check" scale={2} />}
                  </span>
                  <span className={styles.file}>{item.file}</span>
                  <span className={styles.detail}>{item.detail}</span>
                </li>
              ))}
            </ol>
            <div className={styles.progressRow}>
              <div
                className={clsx(styles.progress, running && styles.busy)}
                role="progressbar"
                aria-label="Repackaging Tides"
                aria-valuemin={0}
                aria-valuemax={100}
                aria-valuenow={progress}
              >
                <span style={{ width: `${progress}%` }} />
              </div>
              <span className={styles.percent}>
                <Counter
                  value={progress}
                  fontSize={20}
                  intro={false}
                  motionBlur={false}
                  suffix="%"
                />
              </span>
            </div>
            <p className={styles.result} aria-live="polite">
              {done ? 'Wrote Tides.ipa and Tides.apk.' : ' '}
            </p>
          </div>
        </Window>
      )}

      <div className={styles.icons}>
        <DesktopIcon
          ref={tidesRef}
          icon="folder"
          label="Tides"
          className={styles.draggableIcon}
          selected={selected === 'tides'}
          onPointerDown={onTidesPointerDown}
          onClick={() => setSelected('tides')}
          title="Drag onto OmniBuild to repackage it"
        />
        <DesktopIcon
          ref={omniRef}
          icon="omnibuild"
          label="OmniBuild"
          selected={selected === 'omnibuild'}
          onClick={() => run()}
          aria-label="OmniBuild: run the demonstration"
        />
        <DesktopIcon icon="document" label="Read me" to="/docs" />
        <DesktopIcon
          icon="document"
          label="The plan"
          href={`${REPOSITORY}/issues/1`}
        />
        {outputs && (
          <>
            <DesktopIcon ref={ipaRef} icon="package" label="Tides.ipa" />
            <DesktopIcon ref={apkRef} icon="package" label="Tides.apk" />
          </>
        )}
      </div>
    </div>
  );
};

export default Hero;
