/** A build configuration, as Xcode and Gradle name theirs (Debug, Release). */
export type Configuration = 'debug' | 'release';
export const CONFIGURATIONS: readonly Configuration[] = ['debug', 'release'];

export type Platform = 'android' | 'ios';
export const PLATFORMS: readonly Platform[] = ['android', 'ios'];

/**
 * What a variant is, as omni reports a shell in its `repackaged` event:
 * `app` is a zipped `.app`, for the iOS Simulator, and `ipa` an app for
 * devices.
 */
export type Format = 'apk' | 'aab' | 'ipa' | 'app';

/** One build of a shell, which omni makes apps of the same kind from. */
export interface Variant {
  /** Its file's name, in the folder build writes and on a release. */
  file: string;
  platform: Platform;
  format: Format;
  configuration: Configuration;
}

const FORMATS: Record<Platform, readonly Format[]> = {
  android: ['apk', 'aab'],
  ios: ['app', 'ipa'],
};

/**
 * Each format's end of a file name. The Simulator's app says it's for the
 * Simulator, as a zip says nothing about what's inside.
 */
const ENDINGS: Record<Format, string> = {
  apk: '.apk',
  aab: '.aab',
  ipa: '.ipa',
  app: '-simulator.zip',
};

/**
 * Letters, digits, `.`, `_` and `-`: a name every OS takes as a file's, a
 * GitHub release as an asset's, and a URL as it is.
 */
const NAME = /^[A-Za-z0-9][A-Za-z0-9._-]*$/;

/**
 * The variants of the shell `name` for a platform, in the order build
 * makes them: each configuration's, each format's in turn. Throws if the
 * name can't name its files.
 */
export const variants = (
  name: string,
  platform: Platform,
  configurations: readonly Configuration[] = CONFIGURATIONS,
): Variant[] => {
  checkName(name, 'shell');
  return configurations.flatMap((configuration) =>
    FORMATS[platform].map((format) => ({
      file: `${name}-${configuration}${ENDINGS[format]}`,
      platform,
      format,
      configuration,
    })),
  );
};

/**
 * Throws if `name` can't name the files of a `what`, a shell or a native
 * module.
 */
export const checkName = (name: string, what: string) => {
  if (!NAME.test(name)) {
    throw new Error(
      `'${name}' can't name a ${what}'s files: use letters, digits, '.', '_' and '-', starting with a letter or digit.`,
    );
  }
};

/** `debug` as Xcode and Gradle name it: `Debug`. */
export const capitalized = (configuration: Configuration) =>
  `${configuration[0].toUpperCase()}${configuration.slice(1)}`;
