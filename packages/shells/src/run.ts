import os from 'node:os';
import path from 'node:path';
import { parseArgs } from 'node:util';

import { buildAndroid as buildAndroidShell } from './android.ts';
import { check as checkShells } from './check.ts';
import { buildIos as buildIosShell } from './ios.ts';

export const HELP = `Usage: npm run shells -w @blotch/omni-build-shells -- <command> [options]

Builds an app shell's variants with Xcode or Gradle, and checks that omni
repackages each one, as the Shell workflow (.github/workflows/shell.yml)
does in CI.

Commands:
  build  Build the shell, debug and release: on iOS an app for the
         Simulator (<name>-<configuration>-simulator.zip, a zipped .app)
         and one for devices (<name>-<configuration>.ipa, unsigned, with
         room for omni's signature), and on Android an APK and an app
         bundle (<name>-<configuration>.apk and .aab).
         --platform <ios|android>
                           Required.
         --project <path>  The Xcode project (.xcodeproj), whose scheme
                           <name> builds <name>.app, or the Gradle
                           project's folder, with the app in its app
                           module. Required.
         --name <name>     The shell's name, which its files are named
                           after. Required.
         --out <dir>       Where to write them. Required.
  check  Repackage each variant in --shells with omni, as an app of its
         own, and write <name>.json beside them: each one's file, platform,
         format, configuration and SHA-256.
         --shells <dir>    Where build wrote them, for one platform or
                           both. Required.
         --name <name>     The shell's name. Required.
         --omni <file>     The omni binary. Default: omni, on the PATH.

Options:
  --help  Show this.`;

export interface RunOptions {
  buildIos?: typeof buildIosShell;
  buildAndroid?: typeof buildAndroidShell;
  check?: typeof checkShells;
}

export interface RunResult {
  status: number;
  /** For stderr. */
  message?: string;
}

/**
 * Runs the command `args` ask for (see HELP).
 *
 * The options default to the real commands, and exist for tests.
 */
export const run = async (
  args: string[],
  {
    buildIos = buildIosShell,
    buildAndroid = buildAndroidShell,
    check = checkShells,
  }: RunOptions = {},
): Promise<RunResult> => {
  let values;
  let positionals;
  try {
    ({ values, positionals } = parseArgs({
      args,
      allowPositionals: true,
      options: {
        platform: { type: 'string' },
        project: { type: 'string' },
        name: { type: 'string' },
        out: { type: 'string' },
        shells: { type: 'string' },
        omni: { type: 'string', default: 'omni' },
        help: { type: 'boolean', default: false },
      },
    }));
  } catch (error) {
    return { status: 2, message: `shells: ${(error as Error).message}` };
  }
  const [command, ...rest] = positionals;
  if (values.help) {
    console.log(HELP);
    return { status: 0 };
  }
  const usage = (problem: string) => ({
    status: 2,
    message: `shells: ${problem}. See --help.`,
  });
  if (rest.length > 0) {
    return usage(`Unexpected argument '${rest[0]}'`);
  }

  try {
    if (command === 'build') {
      const { platform, project, name, out } = values;
      if (!platform || !project || !name || !out) {
        return usage('build needs --platform, --project, --name and --out');
      }
      if (platform === 'ios') {
        // Xcode's derived data, kept between runs in the temporary folder,
        // so building again only builds what changed.
        const work = path.join(os.tmpdir(), 'omni-build-shells', name);
        await buildIos({ project, name, out, work });
      } else if (platform === 'android') {
        await buildAndroid({ project, name, out });
      } else {
        return usage(`--platform is ios or android, not '${platform}'`);
      }
    } else if (command === 'check') {
      const { shells, name, omni } = values;
      if (!shells || !name) {
        return usage('check needs --shells and --name');
      }
      await check({ shells, name, omni });
    } else {
      return usage(command ? `Unknown command '${command}'` : 'Name a command');
    }
  } catch (error) {
    return { status: 1, message: `shells: ${(error as Error).message}` };
  }
  return { status: 0 };
};
