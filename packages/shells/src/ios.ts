import { existsSync, mkdirSync, rmSync } from 'node:fs';
import path from 'node:path';

import { run } from './process.ts';
import {
  CONFIGURATIONS,
  type Configuration,
  capitalized,
  variants,
} from './variants.ts';

/**
 * Where each SDK's builds run: anywhere, on a simulator or a device. The
 * destination alone picks the SDK, so that each target builds for its own
 * platform: a watch app for watchOS, where `-sdk iphoneos` would build it
 * for iOS.
 */
const DESTINATIONS = {
  iphonesimulator: 'generic/platform=iOS Simulator',
  iphoneos: 'generic/platform=iOS',
};

export interface BuildIosOptions {
  /** The Xcode project, whose scheme `name` builds `name.app`. */
  project: string;
  name: string;
  /** Where the variants go. */
  out: string;
  /**
   * Xcode's derived data, and the .ipa's Payload folder. Kept between
   * builds, so a second one only builds what changed.
   */
  work: string;
  configurations?: readonly Configuration[];
  /** See RunOptions.log. */
  log?: string;
}

/**
 * Builds a shell with Xcode in each configuration, for the Simulator and
 * for devices, zips each into `out` (see variants()), and returns their
 * paths: the Simulator's a zipped .app, and the devices' an .ipa, a zip of
 * a Payload folder with the .app.
 */
export const buildIos = async ({
  project,
  name,
  out,
  work,
  configurations = CONFIGURATIONS,
  log,
}: BuildIosOptions) => {
  const shells = variants(name, 'ios', configurations);
  mkdirSync(out, { recursive: true });
  const derivedData = path.join(work, 'DerivedData');
  const built: string[] = [];
  for (const { file, format, configuration } of shells) {
    const sdk = format === 'app' ? 'iphonesimulator' : 'iphoneos';
    const xcodeConfiguration = capitalized(configuration);
    await run(
      'xcodebuild',
      [
        'build',
        '-project',
        project,
        '-scheme',
        name,
        '-configuration',
        xcodeConfiguration,
        '-destination',
        DESTINATIONS[sdk],
        '-derivedDataPath',
        derivedData,
        // Xcode's Debug builds put the app's code in a dylib, for Previews,
        // and its executable is a stub that loads it, which the linker
        // leaves unsigned, and so omni can't sign. A shell never runs in
        // Previews, so its debug builds are built whole, as release ones are.
        ...(configuration === 'debug' ? ['ENABLE_DEBUG_DYLIB=NO'] : []),
        // Unsigned, as a shell is signed for each app, but with room for a
        // signature, which the linker reserves when asked: omni replaces a
        // signature, and never adds one. After the project's own flags,
        // which a command line's setting would otherwise replace.
        ...(sdk === 'iphoneos'
          ? [
              'CODE_SIGNING_ALLOWED=NO',
              'OTHER_LDFLAGS=$(inherited) -Wl,-adhoc_codesign',
            ]
          : []),
      ],
      { log },
    );
    const products = path.join(
      derivedData,
      'Build/Products',
      `${xcodeConfiguration}-${sdk}`,
    );
    const app = path.join(products, `${name}.app`);
    if (!existsSync(app)) {
      throw new Error(
        `Xcode built no ${name}.app in ${products}: the scheme ${name} has to build an app of the same name, as Xcode's iOS App template names them both after the project.`,
      );
    }

    // A .app is a folder, and its executable's mode wouldn't survive a
    // workflow artifact or a Windows file system. So the shells travel as
    // zips, which ditto writes with every file's mode.
    const zip = path.join(out, file);
    rmSync(zip, { force: true });
    if (format === 'app') {
      await run('ditto', ['-c', '-k', '--keepParent', app, zip], { log });
    } else {
      const payload = path.join(work, 'ipa', 'Payload');
      rmSync(payload, { recursive: true, force: true });
      await run('ditto', [app, path.join(payload, `${name}.app`)], { log });
      await run('ditto', ['-c', '-k', '--keepParent', payload, zip], { log });
    }
    built.push(zip);
  }
  return built;
};
