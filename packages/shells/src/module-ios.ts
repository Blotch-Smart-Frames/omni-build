import { existsSync, mkdirSync, rmSync } from 'node:fs';
import path from 'node:path';

import { DESTINATIONS } from './ios.ts';
import { run } from './process.ts';
import { checkName } from './variants.ts';

export interface BuildIosModuleOptions {
  /** The Xcode project, whose scheme `name` builds `name.framework`. */
  project: string;
  name: string;
  /** Where `ios/<name>.xcframework` goes. */
  out: string;
  /**
   * Xcode's derived data. Kept between builds, so a second one only builds
   * what changed.
   */
  work: string;
  /** See RunOptions.log. */
  log?: string;
}

/**
 * Builds a native module's framework with Xcode, in its Release
 * configuration, for devices and for the Simulator, and puts them together
 * as `out/ios/<name>.xcframework`, which the module's package.json names in
 * `omniBuild.ios.frameworks`. Returns its path.
 */
export const buildIosModule = async ({
  project,
  name,
  out,
  work,
  log,
}: BuildIosModuleOptions) => {
  checkName(name, 'native module');
  const derivedData = path.join(work, 'DerivedData');
  const frameworks: string[] = [];
  for (const sdk of ['iphoneos', 'iphonesimulator'] as const) {
    await run(
      'xcodebuild',
      [
        'build',
        '-project',
        project,
        '-scheme',
        name,
        '-configuration',
        'Release',
        '-destination',
        DESTINATIONS[sdk],
        '-derivedDataPath',
        derivedData,
        // Unsigned, with room for a signature, as a shell's build for
        // devices is: omni signs the framework with each app it goes in,
        // replacing a signature and never adding one. Xcode signs the
        // Simulator's ad hoc itself.
        ...(sdk === 'iphoneos'
          ? [
              'CODE_SIGNING_ALLOWED=NO',
              'OTHER_LDFLAGS=$(inherited) -Wl,-adhoc_codesign',
            ]
          : []),
      ],
      { log },
    );
    const products = path.join(derivedData, 'Build/Products', `Release-${sdk}`);
    const framework = path.join(products, `${name}.framework`);
    if (!existsSync(framework)) {
      throw new Error(
        `Xcode built no ${name}.framework in ${products}: the scheme ${name} has to build a framework of the same name, as Xcode's Framework template names them both after the project.`,
      );
    }
    frameworks.push(framework);
  }
  const xcframework = path.join(out, 'ios', `${name}.xcframework`);
  mkdirSync(path.dirname(xcframework), { recursive: true });
  // xcodebuild won't write over one.
  rmSync(xcframework, { recursive: true, force: true });
  await run(
    'xcodebuild',
    [
      '-create-xcframework',
      // Nothing compiles against a native module's framework: the shell
      // loads it and finds its classes by name. So it needs no Swift
      // interface, which xcodebuild otherwise asks for, and which only a
      // build for library evolution (BUILD_LIBRARY_FOR_DISTRIBUTION) has.
      '-allow-internal-distribution',
      ...frameworks.flatMap((framework) => ['-framework', framework]),
      '-output',
      xcframework,
    ],
    { log },
  );
  return xcframework;
};
