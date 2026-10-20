import { parseArgs } from 'node:util';

import { check as checkPackages } from './check.ts';
import { pack as packPackages } from './pack.ts';

export const HELP = `Usage: npm run release -w @omni-build/release -- <command> [options]

Makes the npm packages a release publishes, and checks that npm installs
them as it should, without publishing anything.

Commands:
  pack   Pack the launcher, omni-build, and a package per platform holding
         its binary, which the launcher lists as optional dependencies.
         Needs the launcher built (npm run build -w omni-build).
         --binaries <dir>  The six release binaries, each in a folder named
                           omni-build-<Rust target>, as CI's build job
                           uploads them. Required.
         --out <dir>       Where to write the tarballs. Required.
  check  Install omni-build from the tarballs pack wrote, as a user would,
         from a registry on this computer, and check that npm installs this
         machine's binary alone, and that the command and the API run it.
         --packages <dir>  Where pack wrote them. Required.

Options:
  --help  Show this.`;

export interface RunOptions {
  pack?: typeof packPackages;
  check?: typeof checkPackages;
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
  { pack = packPackages, check = checkPackages }: RunOptions = {},
): Promise<RunResult> => {
  let values;
  let positionals;
  try {
    ({ values, positionals } = parseArgs({
      args,
      allowPositionals: true,
      options: {
        binaries: { type: 'string' },
        out: { type: 'string' },
        packages: { type: 'string' },
        help: { type: 'boolean', default: false },
      },
    }));
  } catch (error) {
    return { status: 2, message: `release: ${(error as Error).message}` };
  }
  const [command, ...rest] = positionals;
  if (values.help) {
    console.log(HELP);
    return { status: 0 };
  }
  const usage = (problem: string) => ({
    status: 2,
    message: `release: ${problem}. See --help.`,
  });
  if (rest.length > 0) {
    return usage(`Unexpected argument '${rest[0]}'`);
  }

  try {
    if (command === 'pack') {
      if (!values.binaries || !values.out) {
        return usage('pack needs --binaries and --out');
      }
      await pack({ binaries: values.binaries, out: values.out });
    } else if (command === 'check') {
      if (!values.packages) {
        return usage('check needs --packages');
      }
      await check({ packages: values.packages });
    } else {
      return usage(command ? `Unknown command '${command}'` : 'Name a command');
    }
  } catch (error) {
    return { status: 1, message: `release: ${(error as Error).message}` };
  }
  return { status: 0 };
};
