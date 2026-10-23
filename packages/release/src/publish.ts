import { readFileSync } from 'node:fs';
import path from 'node:path';

import { LAUNCHER } from './manifests.ts';
import { INDEX } from './pack.ts';
import { type Exec, exec as execCommand, npmCommand } from './process.ts';
import type { Packed } from './registry.ts';

export interface PublishOptions {
  /** Where `pack` wrote the tarballs. */
  packages: string;
  /**
   * The git tag the release is published for, which has to be `v` and the
   * packages' version.
   */
  tag?: string;
  exec?: Exec;
  log?: (line: string) => void;
}

/**
 * Publishes the tarballs `pack` wrote to npm, as public packages. The
 * platforms' packages go first and the launcher last, since the launcher
 * lists them as optional dependencies and npm couldn't install it before
 * they're there. A version npm already has is left as it is, so a release
 * that stopped partway can be run again.
 *
 * npm decides how to authenticate. In the release workflow it trades
 * GitHub's OIDC token for one that may publish these packages (trusted
 * publishing), and adds their provenance.
 */
export const publish = async ({
  packages,
  tag,
  exec = execCommand,
  log = console.log,
}: PublishOptions) => {
  const packed: Packed[] = JSON.parse(
    readFileSync(path.join(packages, INDEX), 'utf8'),
  );
  const launcher = packed.find(({ manifest }) => manifest.name === LAUNCHER);
  if (!launcher) {
    throw new Error(`There's no ${LAUNCHER} package in ${packages}.`);
  }
  const { version } = launcher.manifest;
  if (tag !== undefined && tag !== `v${version}`) {
    throw new Error(
      `The tag ${tag} isn't the packages' version, ${version}: a release of it is tagged v${version}.`,
    );
  }
  // npm points `latest`, what `npm install` gets, at whatever is published
  // without a tag, prereleases such as 1.0.0-beta.1 included.
  const distTag = version.includes('-') ? 'next' : 'latest';
  const [node, npm] = npmCommand();

  for (const { file, manifest } of [
    ...packed.filter((each) => each !== launcher),
    launcher,
  ]) {
    const spec = `${manifest.name}@${manifest.version}`;
    // Nothing, for a version npm doesn't have, and a failure, E404, for a
    // package it doesn't have at all.
    const published = await exec(node, [npm, 'view', spec, 'version']).catch(
      () => '',
    );
    if (published.trim() === manifest.version) {
      log(`${spec} is on npm already`);
      continue;
    }
    await exec(node, [
      npm,
      'publish',
      path.resolve(packages, file),
      '--access',
      'public',
      '--tag',
      distTag,
    ]);
    log(`Published ${spec}`);
  }
};
