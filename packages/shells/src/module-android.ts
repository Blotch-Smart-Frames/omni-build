import { cpSync, existsSync, mkdirSync, rmSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import { run } from './process.ts';

/** The Gradle init script that builds the library and says how to dex it. */
export const INIT_SCRIPT = fileURLToPath(
  new URL('module.gradle', import.meta.url),
);
/** What starts each line the init script prints for this tool. */
const SAID = 'omni-build-';

export interface BuildAndroidModuleOptions {
  /** The Gradle project's folder, with its wrapper. */
  project: string;
  /** The library module, by its name in the project, such as `haptics`. */
  library: string;
  /** Where `android/classes.dex` and `android/jni/` go. */
  out: string;
  /** Where the library's AAR is unpacked. */
  work: string;
  /** Gradle's environment: where the Android SDK is, in `ANDROID_HOME`. */
  env?: NodeJS.ProcessEnv;
  /** See RunOptions.log. */
  log?: string;
}

/**
 * Builds a native module's Android library with Gradle, in its release
 * build type, then compiles its classes into DEX with d8, as the Android
 * Gradle plugin would for an app, but without the libraries it uses, which
 * the app has: `out/android/classes.dex`, which the module's package.json
 * names in `omniBuild.android.dex`. Its native libraries, from the AAR's
 * `jni/`, go in `out/android/jni/`, by ABI, for `omniBuild.android.jniLibs`.
 * Returns `out/android`.
 */
export const buildAndroidModule = async ({
  project,
  library,
  out,
  work,
  env,
  log,
}: BuildAndroidModuleOptions) => {
  // Gradle runs in the project's folder, where a relative path to its
  // wrapper, or to the folder it unpacks into, would lead nowhere.
  const gradleProject = path.resolve(project);
  const unpacked = path.resolve(work, 'aar');
  rmSync(unpacked, { recursive: true, force: true });
  const { stdout } = await run(
    path.join(gradleProject, 'gradlew'),
    [
      '--init-script',
      INIT_SCRIPT,
      `:${library}:omniBuildModule`,
      `-PomniBuildModuleOut=${unpacked}`,
      // The init script's task isn't written for the configuration cache,
      // which a project may turn on for its own builds.
      '--no-configuration-cache',
      '--console=plain',
    ],
    { cwd: gradleProject, env, log },
  );
  const said = (what: string) =>
    stdout
      .split('\n')
      .filter((line) => line.startsWith(`${SAID}${what}: `))
      .map((line) => line.slice(SAID.length + what.length + 2).trim());
  const [d8] = said('d8');
  const [minSdk] = said('min-sdk');
  if (!d8 || !minSdk) {
    throw new Error(
      `Gradle didn't say how to compile :${library} for Android: it has to be an Android library module (com.android.library) with a release build type, as Android Studio's Android Library template makes one.`,
    );
  }
  // The Android Gradle plugin compiles an app's classes with a D8 of its
  // own, so it doesn't install the build tools whose d8 this runs.
  if (!existsSync(d8)) {
    throw new Error(
      `There's no d8 at ${d8}, in the Android SDK's build tools of the version the Android Gradle plugin uses: install them with the SDK Manager, or \`sdkmanager "build-tools;${path.basename(path.dirname(d8))}"\`.`,
    );
  }
  const android = path.join(out, 'android');
  rmSync(android, { recursive: true, force: true });
  mkdirSync(android, { recursive: true });
  await run(
    d8,
    [
      '--release',
      '--min-api',
      minSdk,
      ...said('lib').flatMap((jar) => ['--lib', jar]),
      ...said('classpath').flatMap((jar) => ['--classpath', jar]),
      '--output',
      android,
      path.join(unpacked, 'classes.jar'),
    ],
    { env, log },
  );
  const jni = path.join(unpacked, 'jni');
  if (existsSync(jni)) {
    cpSync(jni, path.join(android, 'jni'), { recursive: true });
  }
  return android;
};
