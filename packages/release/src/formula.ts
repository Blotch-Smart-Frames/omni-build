import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import { archiveName, readSums } from './archives.ts';
import type { Manifest } from './manifests.ts';

const REPO_ROOT = fileURLToPath(new URL('../../..', import.meta.url));

/**
 * The formula's name, and so its file's, `omni-build.rb`: the crate's and
 * the npm launcher's name. It installs the `omni` command.
 */
export const FORMULA = 'omni-build';

/**
 * The platforms Homebrew runs on, by their keys in PLATFORMS, under the
 * blocks a formula picks them with. Homebrew has no Windows.
 */
const SYSTEMS = [
  [
    'on_macos',
    [
      ['on_arm', 'darwin-arm64'],
      ['on_intel', 'darwin-x64'],
    ],
  ],
  [
    'on_linux',
    [
      ['on_arm', 'linux-arm64'],
      ['on_intel', 'linux-x64'],
    ],
  ],
] as const;

export interface FormulaText {
  version: string;
  /** The repository's page, the formula's homepage. */
  homepage: string;
  license: string;
  /** Where the archives are downloaded from: a URL, without a `/` at the end. */
  url: string;
  /** The archives' SHA-256s, by file name. */
  sums: Map<string, string>;
}

/**
 * The formula for the tap, which installs the release's prebuilt binaries
 * rather than building from source: the binary is what every other channel
 * installs too, and building it would need Rust and minutes per install.
 */
export const formulaText = ({
  version,
  homepage,
  license,
  url,
  sums,
}: FormulaText) => {
  const blocks = SYSTEMS.map(([system, cpus]) => {
    const urls = cpus.map(([cpu, key]) => {
      const name = archiveName(key);
      const sum = sums.get(name);
      if (!sum) {
        throw new Error(`There's no SHA-256 for ${name}.`);
      }
      return `    ${cpu} do
      url "${url}/${name}"
      sha256 "${sum}"
    end`;
    });
    return `  ${system} do\n${urls.join('\n')}\n  end`;
  });
  return `# Written by OmniBuild's release workflow for each version, from
# packages/release/src/formula.ts in ${homepage}:
# change it there.
class OmniBuild < Formula
  desc "Build, sign, install and ship iOS and Android apps with no native toolchain"
  homepage "${homepage}"
  version "${version}"
  license "${license}"

${blocks.join('\n\n')}

  def install
    bin.install "omni"
  end

  test do
    assert_equal "omni #{version}", shell_output("#{bin}/omni --version").strip
  end
end
`;
};

export interface FormulaOptions {
  /** Where `archive` wrote them, with their SHA256SUMS. */
  archives: string;
  /** Where to write the formula. */
  out: string;
  /**
   * Where the archives are downloaded from. The GitHub release of the
   * version without it; CI gives a `file://` URL, to install them with
   * Homebrew before there's a release.
   */
  url?: string;
  root?: string;
  log?: (line: string) => void;
}

/**
 * Writes the Homebrew formula for the archives `archive` wrote, at the
 * version and with the licence of the launcher's package.json.
 */
export const formula = ({
  archives,
  out,
  url,
  root = REPO_ROOT,
  log = console.log,
}: FormulaOptions) => {
  const launcher: Manifest = JSON.parse(
    readFileSync(
      path.join(root, 'packages', 'omni-build', 'package.json'),
      'utf8',
    ),
  );
  // git+https://github.com/<owner>/<repo>.git
  const homepage = (launcher.repository as { url: string }).url
    .replace(/^git\+/, '')
    .replace(/\.git$/, '');
  const text = formulaText({
    version: launcher.version,
    homepage,
    license: launcher.license as string,
    url: (url ?? `${homepage}/releases/download/v${launcher.version}`).replace(
      /\/$/,
      '',
    ),
    sums: readSums(archives),
  });
  mkdirSync(path.dirname(out), { recursive: true });
  writeFileSync(out, text);
  log(`Wrote the formula for ${FORMULA} ${launcher.version} to ${out}`);
};
