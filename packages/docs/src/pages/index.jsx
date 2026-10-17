import Layout from '@theme/Layout';

import DesktopField from '../components/landing/desktop-field';
import Hero from '../components/landing/hero';
import Sections from '../components/landing/sections';
import styles from './index.module.css';

// The landing page is a one-bit desktop: a live dithered field behind
// windows that each hold a part of the story, from the demonstration in the
// first screen to the roadmap and the invitation to follow along.
const Home = () => (
  <Layout
    title="iOS and Android apps with no native toolchain"
    description="OmniBuild builds, signs, installs and ships iOS and Android apps with no Xcode, no JDK and no Android SDK, by repackaging a prebuilt app shell. In planning."
  >
    <main className={styles.page}>
      <DesktopField />
      <Hero />
      <Sections />
    </main>
  </Layout>
);

export default Home;
