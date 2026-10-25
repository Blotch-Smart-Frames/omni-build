import { copyFileSync, existsSync, mkdirSync, readdirSync } from 'node:fs';
import path from 'node:path';

import { run } from './process.ts';
import {
  CONFIGURATIONS,
  type Configuration,
  capitalized,
  variants,
} from './variants.ts';

export interface BuildAndroidOptions {
  /** The Gradle project's folder, with its wrapper and the app in `app/`. */
  project: string;
  name: string;
  /** Where the variants go. */
  out: string;
  configurations?: readonly Configuration[];
  /** Gradle's environment: where the Android SDK is, in `ANDROID_HOME`. */
  env?: NodeJS.ProcessEnv;
  /** See RunOptions.log. */
  log?: string;
}

/**
 * Builds a shell with Gradle as an APK and an app bundle in each
 * configuration, copies them into `out` (see variants()), and returns the
 * copies' paths.
 */
export const buildAndroid = async ({
  project,
  name,
  out,
  configurations = CONFIGURATIONS,
  env,
  log,
}: BuildAndroidOptions) => {
  const shells = variants(name, 'android', configurations);
  mkdirSync(out, { recursive: true });
  // Gradle runs in the project's folder, where a relative path to its
  // wrapper would lead nowhere.
  const gradleProject = path.resolve(project);
  await run(
    path.join(gradleProject, 'gradlew'),
    [
      ...configurations.flatMap((configuration) => [
        `assemble${capitalized(configuration)}`,
        `bundle${capitalized(configuration)}`,
      ]),
      '--console=plain',
    ],
    { cwd: gradleProject, env, log },
  );
  const outputs = path.join(gradleProject, 'app/build/outputs');
  return shells.map(({ file, format, configuration }) => {
    // Gradle names the file after the module and the variant, and an APK
    // signed with no key app-release-unsigned.apk, so it's the one there.
    const folder = path.join(
      outputs,
      format === 'apk' ? 'apk' : 'bundle',
      configuration,
    );
    const built = existsSync(folder)
      ? readdirSync(folder).filter((entry) => entry.endsWith(`.${format}`))
      : [];
    if (built.length !== 1) {
      throw new Error(
        `Gradle wrote ${built.length === 0 ? 'no' : built.length} .${format} files to ${folder}, where it writes the app module's one: the app has to be the module app, as Android Studio's templates make it.`,
      );
    }
    const copy = path.join(out, file);
    copyFileSync(path.join(folder, built[0]), copy);
    return copy;
  });
};
