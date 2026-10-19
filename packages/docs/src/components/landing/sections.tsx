import BrowserOnly from '@docusaurus/BrowserOnly';
import Link from '@docusaurus/Link';
import { useColorMode } from '@docusaurus/theme-common';
import clsx from 'clsx';

import Button from '../one-bit/button';
import PixelIcon from '../one-bit/pixel-icon';
import Window from '../one-bit/window';
import PixelBlast from '../react-bits/pixel-blast';
import styles from './sections.module.css';

const REPOSITORY = 'https://github.com/Blotch-Smart-Frames/omni-build';

// Every fact below is from the README, the plan (issue #1) or the e2e
// README. Nothing here is built yet, and the copy says so where it matters.

const VALUES = [
  [
    'App ID',
    'CFBundleIdentifier in Info.plist',
    'package in AndroidManifest.xml',
  ],
  [
    'Name',
    'CFBundleDisplayName in Info.plist',
    'the app label in resources.arsc',
  ],
  [
    'Version',
    'CFBundleShortVersionString, CFBundleVersion',
    'versionName, versionCode',
  ],
  ['Icon', 'Assets.car', 'PNGs per density'],
  ['Payload', 'main.lynx.bundle in the .app', 'assets/main.lynx.bundle'],
  ['Signature', 'Apple code signature', 'APK Signature Scheme v2/v3'],
];

const BEFORE_AFTER = [
  ['Build an Android store binary', 'A JDK and the Android SDK', 'Node'],
  ['Build an iOS store binary', 'A Mac with Xcode', 'Node, on any OS'],
  [
    'Install your build on a phone',
    'Xcode or adb',
    'ADB built in, or TestFlight',
  ],
  ['Upload to the stores', 'Transporter, the Play Console', 'The stores’ APIs'],
  [
    'Write your own Swift or Kotlin',
    'Xcode or Gradle',
    'Still Xcode or Gradle',
  ],
];

const TARGETS = [
  ['macOS', 'Apple silicon and Intel', 'darwin-arm64, darwin-x64'],
  ['Linux', 'ARM64 and x64, static', 'linux-arm64, linux-x64'],
  ['Windows', 'ARM64 and x64', 'win32-arm64, win32-x64'],
];

const IN_PLACE = [
  'Release builds for all six targets, each run on its own OS and CPU',
  '100% test coverage of the Rust and the TypeScript',
  'End-to-end runs on the iOS Simulator and an Android emulator',
];

const PHASES = [
  ['Spikes', 'Rewrite and sign a prebuilt shell on each platform; go or no-go'],
  ['Android', 'APK and AAB writing, signing, ADB install'],
  ['iOS', '.app and .ipa writing, Assets.car, signing, device install'],
  ['Upload', 'Upload to TestFlight and a Google Play track'],
  ['Native modules', 'Prebuilt modules from npm, no Xcode or Gradle'],
];

const Check = ({ on }: { on: boolean }) => (
  <span className={styles.checkbox} aria-hidden="true">
    {on && <PixelIcon name="check" scale={2} />}
  </span>
);

const HowItWorks = () => (
  <Window
    title="How it works"
    titleAs="h2"
    titleId="how-it-works"
    draggable
    className={clsx(styles.window, styles.how)}
    bodyClassName={styles.body}
    status={
      <>
        <span>6 values · 2 platforms · 0 compilers</span>
        <Link to="/docs/how-it-works">The full explanation</Link>
      </>
    }
  >
    <p className={styles.statement}>
      In a Lynx or WebView app, the native code is the same for every app. Only
      the data changes.
    </p>
    <p className={styles.copy}>
      So OmniBuild doesn&rsquo;t compile anything. It takes a shell its own CI
      built with Xcode and Gradle, writes your app&rsquo;s values into the
      shell&rsquo;s configuration, adds your payload, and signs the result.
      Every step is file-format work and cryptography.
    </p>
    <div className={styles.tableWrap}>
      <table className={styles.list}>
        <thead>
          <tr>
            <th scope="col">Your app&rsquo;s</th>
            <th scope="col">iOS</th>
            <th scope="col">Android</th>
          </tr>
        </thead>
        <tbody>
          {VALUES.map(([value, ios, android]) => (
            <tr key={value}>
              <th scope="row">{value}</th>
              <td data-label="iOS">{ios}</td>
              <td data-label="Android">{android}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  </Window>
);

const BeforeAfter = () => (
  <Window
    title="Before and after"
    titleAs="h2"
    titleId="before-and-after"
    draggable
    className={clsx(styles.window, styles.before)}
    bodyClassName={styles.body}
    status={<span>The right-hand column is the plan, not a release.</span>}
  >
    <p className={styles.statement}>
      A store binary should need what a website does: Node.
    </p>
    <div className={styles.tableWrap}>
      <table className={styles.list}>
        <thead>
          <tr>
            <th scope="col">To</th>
            <th scope="col">Today you install</th>
            <th scope="col">With OmniBuild</th>
          </tr>
        </thead>
        <tbody>
          {BEFORE_AFTER.map(([task, today, after], i) => (
            <tr
              key={task}
              className={clsx(
                i === BEFORE_AFTER.length - 1 && styles.unchanged,
              )}
            >
              <th scope="row">{task}</th>
              <td data-label="Today">{today}</td>
              <td data-label="With OmniBuild">{after}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  </Window>
);

const WhereItRuns = () => (
  <Window
    title="Where it runs"
    titleAs="h2"
    titleId="where-it-runs"
    draggable
    className={clsx(styles.window, styles.runs)}
    bodyClassName={styles.body}
    status={
      <span>6 targets, each built and run in CI on every pull request</span>
    }
  >
    <p className={styles.statement}>One static binary, no runtime.</p>
    <ul className={styles.targets}>
      {TARGETS.map(([os, cpus, keys]) => (
        <li key={os}>
          <span className={styles.os}>{os}</span>
          <span>{cpus}</span>
          <code>{keys}</code>
        </li>
      ))}
    </ul>
  </Window>
);

const Install = () => (
  <Window
    title="Install"
    titleAs="h2"
    titleId="install"
    draggable
    inactive
    className={clsx(styles.window, styles.install)}
    bodyClassName={styles.body}
    status={<Link to="/docs/installation">Installation, in full</Link>}
  >
    <p className={styles.fieldLabel} id="install-command">
      npm, once it&rsquo;s published
    </p>
    {/* Dimmed with a dither, as a disabled field was: there's nothing to
        install yet, and a live-looking command would say otherwise. */}
    <div
      className={styles.field}
      role="textbox"
      aria-readonly="true"
      aria-disabled="true"
      aria-labelledby="install-command"
    >
      <code>npm install --save-dev omni-build</code>
    </div>
    <p className={styles.copy}>
      Or <code>cargo install omni-build</code>, Homebrew, or a download from
      GitHub releases. Not published yet.
    </p>
  </Window>
);

const Roadmap = () => (
  <Window
    title="Roadmap"
    titleAs="h2"
    titleId="roadmap"
    draggable
    className={clsx(styles.window, styles.roadmap)}
    bodyClassName={clsx(styles.body, styles.roadmapBody)}
    status={
      <>
        <span>Android first: every Android format is public.</span>
        <Link to="/docs/roadmap">Phases, risks and open decisions</Link>
      </>
    }
  >
    <div>
      <h3 className={styles.listTitle}>In place</h3>
      <ul className={styles.checklist}>
        {IN_PLACE.map((item) => (
          <li key={item}>
            <Check on />
            <span>{item}</span>
          </li>
        ))}
      </ul>
    </div>
    <div>
      <h3 className={styles.listTitle}>Phases</h3>
      <ol className={styles.checklist} start={0}>
        {PHASES.map(([name, scope], i) => (
          <li key={name} className={clsx(i === 0 && styles.next)}>
            <Check on={false} />
            <span>
              <span className={styles.phase}>
                {i}. {name}
                {i === 0 && <span className={styles.nextTag}>next</span>}
              </span>
              <span className={styles.scope}>{scope}</span>
            </span>
          </li>
        ))}
      </ol>
    </div>
  </Window>
);

// The closing field: React Bits' PixelBlast with every soft edge turned
// off, so its waves stay hard one-bit pixels. It ripples where it's
// clicked.
const Field = () => {
  const { colorMode } = useColorMode();
  const ink = colorMode === 'dark' ? '#ffffff' : '#000000';
  return (
    <div className={styles.blast} aria-hidden="true">
      <BrowserOnly>
        {() => (
          <PixelBlast
            variant="square"
            pixelSize={4}
            color={ink}
            accentColor={ink}
            lightMode={colorMode !== 'dark'}
            transparent={false}
            patternScale={2.5}
            patternDensity={0.95}
            softness={0}
            glow={0}
            twinkle={0}
            edgeFade={0}
            pixelSizeJitter={0}
            noiseAmount={0}
            speed={0.35}
            hoverEffect="reveal"
            hoverRadius={120}
            enableRipples
          />
        )}
      </BrowserOnly>
    </div>
  );
};

const FollowAlong = () => (
  <Window
    title="Follow along"
    titleAs="h2"
    titleId="follow-along"
    className={clsx(styles.window, styles.dialog)}
    bodyClassName={styles.dialogBody}
  >
    <Field />
    <div className={styles.dialogText}>
      <PixelIcon name="omnibuild" scale={4} className={styles.dialogIcon} />
      <div>
        <p className={styles.statement}>OmniBuild is planned in the open.</p>
        <p className={styles.copy}>
          Watch the repository to see each phase land, or read the plan and
          weigh in on the decisions still open. Any app whose per-app part is
          data can be built with it.
        </p>
        <div className={styles.dialogActions}>
          <Button href={`${REPOSITORY}/issues/1`}>Read the plan</Button>
          <Button href={REPOSITORY} primary>
            Watch on GitHub
          </Button>
        </div>
      </div>
    </div>
  </Window>
);

const Sections = () => (
  <div className={styles.sections}>
    <HowItWorks />
    <BeforeAfter />
    <WhereItRuns />
    <Install />
    <Roadmap />
    <FollowAlong />
  </div>
);

export default Sections;
