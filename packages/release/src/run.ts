import { parseArgs } from 'node:util';

import {
  archive as archiveBinaries,
  checkArchive as checkArchives,
} from './archives.ts';
import { check as checkPackages } from './check.ts';
import { formula as writeFormula } from './formula.ts';
import { pack as packPackages } from './pack.ts';
import { publish as publishPackages } from './publish.ts';

export const HELP = `Usage: npm run release -w @blotch/omni-build-release -- <command> [options]

Makes the npm packages a release publishes, checks that npm installs them
as it should, and publishes them. Makes the archives for the GitHub
release, and the Homebrew formula that installs them.

Commands:
  pack   Pack the launcher, @blotch/omni-build, and a package per platform
         holding its binary, which the launcher lists as optional
         dependencies.
         Needs the launcher built (npm run build -w @blotch/omni-build).
         --binaries <dir>  The six release binaries, each in a folder named
                           omni-build-<Rust target>, as CI's build job
                           uploads them. Required.
         --out <dir>       Where to write the tarballs. Required.
  check  Install @blotch/omni-build from the tarballs pack wrote, as a user
         would, from a registry on this computer, and check that npm
         installs this machine's binary alone, and that the command and
         the API run it.
         --packages <dir>  Where pack wrote them. Required.
  publish
         Publish the tarballs pack wrote to npm, the platforms' packages
         first and the launcher last, leaving any version npm already has.
         The release workflow runs it, which npm trusts to publish them.
         --packages <dir>  Where pack wrote them. Required.
         --tag <tag>       The git tag of the release, which has to be v and
                           the packages' version.
  archive
         Put each platform's binary in an archive for the GitHub release,
         a zip for Windows and a gzipped tar elsewhere, and write their
         SHA-256s beside them in SHA256SUMS.
         --binaries <dir>  The six release binaries, as for pack. Required.
         --out <dir>       Where to write the archives. Required.
  check-archive
         Check this machine's archive against SHA256SUMS, unpack it as a
         user would, and check that its omni runs.
         --archives <dir>  Where archive wrote them. Required.
  formula
         Write the Homebrew formula that installs the archives.
         --archives <dir>  Where archive wrote them. Required.
         --out <file>      Where to write the formula. Required.
         --url <url>       Where the archives are downloaded from. Without
                           it, the GitHub release of this version.

Options:
  --help  Show this.`;

export interface RunOptions {
  pack?: typeof packPackages;
  check?: typeof checkPackages;
  publish?: typeof publishPackages;
  archive?: typeof archiveBinaries;
  checkArchive?: typeof checkArchives;
  formula?: typeof writeFormula;
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
    pack = packPackages,
    check = checkPackages,
    publish = publishPackages,
    archive = archiveBinaries,
    checkArchive = checkArchives,
    formula = writeFormula,
  }: RunOptions = {},
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
        tag: { type: 'string' },
        archives: { type: 'string' },
        url: { type: 'string' },
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
    } else if (command === 'publish') {
      if (!values.packages) {
        return usage('publish needs --packages');
      }
      await publish({ packages: values.packages, tag: values.tag });
    } else if (command === 'archive') {
      if (!values.binaries || !values.out) {
        return usage('archive needs --binaries and --out');
      }
      archive({ binaries: values.binaries, out: values.out });
    } else if (command === 'check-archive') {
      if (!values.archives) {
        return usage('check-archive needs --archives');
      }
      await checkArchive({ archives: values.archives });
    } else if (command === 'formula') {
      if (!values.archives || !values.out) {
        return usage('formula needs --archives and --out');
      }
      formula({ archives: values.archives, out: values.out, url: values.url });
    } else {
      return usage(command ? `Unknown command '${command}'` : 'Name a command');
    }
  } catch (error) {
    return { status: 1, message: `release: ${(error as Error).message}` };
  }
  return { status: 0 };
};
