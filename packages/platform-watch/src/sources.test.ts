import { describe, expect, it } from 'vitest';

import {
  parseAndroidSdk,
  parseAppleReleases,
  parseAppleRequirements,
  parseGradleSdk,
  parsePlayTargetApi,
  parseXcodePin,
} from './sources.ts';

// Excerpts of the real pages, as they were in October 2026. When a parser
// stops reading a page, replace its excerpt with the page's new markup.

const APPLE_REQUIREMENTS = `
<article id="article-04282026a" class="article thumbnail-layout margin-bottom-small">
  <a class="article-title" href="/news/upcoming-requirements/?id=04282026a"><h2>SDK minimum requirements</h2></a>
  <div class="article-text-wrapper">
    <p class="lighter  article-date">Since April 28, 2026</p>
    <span class="article-text"><p>Apps uploaded to App&nbsp;Store&nbsp;Connect must be built with <a href="https://apps.apple.com/us/app/xcode/id497799835?mt=12" target="_blank" class="nowrap">Xcode 26</a> or later using an SDK for <a href="/ios/" target="_blank" class="nowrap">iOS 26</a>, <a href="/ipados/" target="_blank" class="nowrap">iPadOS 26</a>, <a href="/tvos/" target="_blank" class="nowrap">tvOS 26</a>, <a href="/visionos/" target="_blank" class="nowrap">visionOS 26</a>, or <a href="/watchos/" target="_blank" class="nowrap">watchOS 26</a>.</p></span>
  </div>
</article>
<article id="article-04292024a" class="article thumbnail-layout margin-bottom-small">
  <a class="article-title" href="/news/upcoming-requirements/?id=04292024a"><h2>Xcode 15</h2></a>
  <div class="article-text-wrapper">
    <p class="lighter  article-date">Since April 29, 2024</p>
    <span class="article-text"><p>Apps uploaded to App Store Connect must be built with <a href="https://apps.apple.com/us/app/xcode/id497799835?mt=12">Xcode 15</a> for <a href="https://developer.apple.com/ios/">iOS 17</a>, <a href="https://developer.apple.com/ipados/">iPadOS 17</a>, <a href="https://developer.apple.com/tvos/">tvOS 17</a>, or <a href="https://developer.apple.com/watchos/">watchOS 10</a> starting April 29, 2024.</p></span>
  </div>
</article>
<article id="article-11012023a">
  <p class="lighter  article-date">Since November 1, 2023</p>
  <span class="article-text"><p>If you notarize Mac software with the Apple notary service using the altool command-line utility or Xcode 13 or earlier, you’ll need to transition to the notarytool command-utility or upgrade to Xcode 14 or later.</p></span>
</article>`;

const APPLE_RELEASES = `<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0" xmlns:content="http://purl.org/rss/1.0/modules/content/">
<channel>
<title>Releases - Apple Developer</title>
<item>
<title>Xcode 27.2 beta 3 (27B5038j)</title>
<link>https://developer.apple.com/news/releases/?id=10142026a</link>
<pubDate>Wed, 14 Oct 2026 10:00:00 PDT</pubDate>
</item>
<item>
<title>Xcode 27.1 (27A9275)</title>
<link>https://developer.apple.com/news/releases/?id=10152026a</link>
<pubDate>Thu, 15 Oct 2026 13:00:00 PDT</pubDate>
</item>
<item>
<title>TestFlight Update</title>
<link>https://developer.apple.com/news/releases/?id=10142026c</link>
<pubDate>Wed, 14 Oct 2026 14:00:00 PDT</pubDate>
</item>
<item>
<title>iOS 27.0.1 (24A446)</title>
<link>https://developer.apple.com/news/releases/?id=09282026a</link>
<pubDate>Mon, 28 Sep 2026 10:00:00 PDT</pubDate>
</item>
<item>
<title>Xcode 26.6 (17F113)</title>
<link>https://developer.apple.com/news/releases/?id=07212026a</link>
<pubDate>Tue, 21 Jul 2026 10:00:00 PDT</pubDate>
</item>
</channel>
</rss>`;

const PLAY_TARGET_API = `
<p>When you upload an APK, it must meet Google Play&#39;s <a href="https://support.google.com/googleplay/android-developer/answer/11926878">target API level
requirements</a>.</p>

<p>Starting August 31 2026:</p>

<ul>
<li>New apps and app updates must target Android 16 (API level 36) or higher to be
submitted to Google Play; except for Wear OS and Android Automotive OS apps,
which must target Android 15 (API level 35) or higher, and Android TV and
Android XR apps, which must target Android 14 (API level 34) or higher.</li>
<li>Existing apps must target Android 15 (API level 35) or higher to remain
available to new users on devices running Android OS higher than your app&#39;s
target API level.</li>
</ul>`;

const remotePackage = (path: string, apiLevel: string, codename?: string) => `
  <remotePackage path="platforms;${path}">
    <type-details xsi:type="sdk:platformDetailsType">
      <api-level>${apiLevel}</api-level>
      ${codename === undefined ? '' : `<codename>${codename}</codename>`}
      <base-extension>true</base-extension>
    </type-details>
    <display-name>Android SDK Platform ${path}</display-name>
    <channelRef ref="channel-0"/>
  </remotePackage>`;

const ANDROID_SDK = `<sdk:sdk-repository>
  <channel id="channel-0">stable</channel>
  ${remotePackage('android-37.2', '37.2', '')}
  ${remotePackage('android-37.2-beta3', '37.1', 'DEV')}
  ${remotePackage('android-37.0', '37.0', '')}
  ${remotePackage('android-CANARY', '37.1', 'CANARY')}
  ${remotePackage('android-canary-20261007', '37.1', 'CANARY')}
  ${remotePackage('android-36-ext19', '36x')}
  ${remotePackage('android-36', '36')}
</sdk:sdk-repository>`;

describe('parseAppleRequirements', () => {
  it('reads the newest Xcode uploads must be built with, and its date', () => {
    expect(parseAppleRequirements(APPLE_REQUIREMENTS)).toEqual({
      major: 26,
      when: 'Since April 28, 2026',
    });
  });

  it('reads an announced requirement', () => {
    const announced = `<p class="article-date">Begins April 27, 2027</p>
      <p>Apps uploaded to App Store Connect must be built using Xcode 27 or later.</p>
      ${APPLE_REQUIREMENTS}`;

    expect(parseAppleRequirements(announced)).toEqual({
      major: 27,
      when: 'Begins April 27, 2027',
    });
  });

  it('reads a requirement without a date', () => {
    expect(
      parseAppleRequirements('<p>Apps must be built with Xcode 26.</p>'),
    ).toEqual({ major: 26, when: null });
  });

  it('throws when no sentence states the requirement', () => {
    expect(() =>
      parseAppleRequirements('<p>Apps need Xcode 26 or later.</p>'),
    ).toThrow('no sentence saying "must be built with Xcode N"');
  });
});

describe('parseAppleReleases', () => {
  it('reads the newest Xcode release, skipping betas', () => {
    expect(parseAppleReleases(APPLE_RELEASES)).toEqual({
      major: 27,
      version: '27.1',
    });
  });

  it('reads titles in CDATA sections', () => {
    const rss = '<item><title><![CDATA[Xcode 27 (27A266a)]]></title></item>';

    expect(parseAppleReleases(rss)).toEqual({ major: 27, version: '27' });
  });

  it('gives null when the feed has no Xcode release in it', () => {
    const rss = '<item><title>iOS 27.0.1 (24A446)</title></item>';

    expect(parseAppleReleases(rss)).toBeNull();
  });

  it('throws when no item is titled like a release', () => {
    expect(() =>
      parseAppleReleases('<item><title>TestFlight Update</title></item>'),
    ).toThrow('no items titled like "Xcode 27.1 (27A9275)"');
  });
});

describe('parsePlayTargetApi', () => {
  it('reads the level for phone and tablet apps, and its date', () => {
    expect(parsePlayTargetApi(PLAY_TARGET_API)).toEqual({
      api: 36,
      when: 'Starting August 31 2026',
    });
  });

  it('reads the newest level when the page lists more than one', () => {
    const page = `<p>Starting August 31 2027:</p>
      <p>New apps and app updates must target Android 17 (API level 37) or higher.</p>
      ${PLAY_TARGET_API}`;

    expect(parsePlayTargetApi(page)).toEqual({
      api: 37,
      when: 'Starting August 31 2027',
    });
  });

  it('reads the level when the page gives no Android version', () => {
    expect(
      parsePlayTargetApi(
        '<p>New apps and app updates must target API level 35 or higher.</p>',
      ),
    ).toEqual({ api: 35, when: null });
  });

  it('throws when no sentence states the requirement', () => {
    expect(() => parsePlayTargetApi('<p>Target a recent API.</p>')).toThrow(
      'no sentence saying "New apps and app updates must target Android N (API level N)"',
    );
  });
});

describe('parseAndroidSdk', () => {
  it('reads the newest stable level, skipping previews and extensions', () => {
    expect(parseAndroidSdk(ANDROID_SDK)).toEqual({ api: 37 });
  });

  it('throws when there is no stable platform', () => {
    expect(() =>
      parseAndroidSdk(remotePackage('android-CANARY', '37.1', 'CANARY')),
    ).toThrow('no stable "platforms;android-N" package');
  });
});

describe('parseXcodePin', () => {
  it('reads the Xcode the Shell workflow gets', () => {
    const workflow = `
    uses: ./.github/workflows/shell.yml
    with:
      developer-dir: /Applications/Xcode_26.6.app/Contents/Developer`;

    expect(parseXcodePin(workflow)).toEqual({ major: 26, version: '26.6' });
  });

  it('accepts the same Xcode in several jobs', () => {
    const line = 'developer-dir: /Applications/Xcode_27.app/Contents/Developer';

    expect(parseXcodePin(`${line}\n${line}`)).toEqual({
      major: 27,
      version: '27',
    });
  });

  it('throws when there is no pin', () => {
    expect(() => parseXcodePin('runs-on: macos-latest')).toThrow(
      'expected one "developer-dir: /Applications/Xcode_N.app/..." line, found 0 versions',
    );
  });

  it('throws when jobs pin different versions', () => {
    const workflow = `
      developer-dir: /Applications/Xcode_26.6.app/Contents/Developer
      developer-dir: /Applications/Xcode_27.app/Contents/Developer`;

    expect(() => parseXcodePin(workflow)).toThrow('found 2 versions');
  });
});

describe('parseGradleSdk', () => {
  it('reads compileSdk and targetSdk', () => {
    const gradle = `
    compileSdk = 37
    defaultConfig {
        minSdk = 24
        targetSdk = 36
    }`;

    expect(parseGradleSdk(gradle)).toEqual({ compileSdk: 37, targetSdk: 36 });
  });

  it('throws when one is missing', () => {
    expect(() => parseGradleSdk('compileSdk = 36')).toThrow(
      'no "targetSdk = N" line',
    );
  });
});
