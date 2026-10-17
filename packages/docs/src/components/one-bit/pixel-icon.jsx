// OmniBuild's own one-bit icons, drawn as pixel grids: `#` is ink, `o` is
// paper, `.` is transparent. They're ours on purpose. The desktop borrows the
// medium's grammar, not any vendor's icons (PRODUCT.md: nothing may look like
// Apple's or Google's).

export const ICONS = {
  // The OmniBuild mark: one ring, one core. Also the logo and the favicon.
  omnibuild: [
    '................',
    '..############..',
    '.#oooooooooooo#.',
    '#oooo######oooo#',
    '#ooo##oooo##ooo#',
    '#oo##oooooo##oo#',
    '#oo#oooooooo#oo#',
    '#oo#ooo##ooo#oo#',
    '#oo#ooo##ooo#oo#',
    '#oo#oooooooo#oo#',
    '#oo##oooooo##oo#',
    '#ooo##oooo##ooo#',
    '#oooo######oooo#',
    '.#oooooooooooo#.',
    '..############..',
    '................',
  ],
  folder: [
    '................',
    '................',
    '.#####..........',
    '#ooooo#.........',
    '#oooooo########.',
    '#oooooooooooooo#',
    '################',
    '#oooooooooooooo#',
    '#oooooooooooooo#',
    '#oooooooooooooo#',
    '#oooooooooooooo#',
    '#oooooooooooooo#',
    '#oooooooooooooo#',
    '################',
    '................',
    '................',
  ],
  document: [
    '..##########....',
    '..#oooooooo##...',
    '..#oooooooo#o#..',
    '..#oooooooo#oo#.',
    '..#oooooooo####.',
    '..#ooooooooooo#.',
    '..#oo#######oo#.',
    '..#ooooooooooo#.',
    '..#oo#######oo#.',
    '..#ooooooooooo#.',
    '..#oo#######oo#.',
    '..#ooooooooooo#.',
    '..#oo#####oooo#.',
    '..#ooooooooooo#.',
    '..#############.',
    '................',
  ],
  // A sealed package: what a signed .ipa or .apk is.
  package: [
    '................',
    '.##############.',
    '.#oooooooooooo#.',
    '.#oooooo#ooooo#.',
    '.##############.',
    '..#ooooo#oooo#..',
    '..#ooooo#oooo#..',
    '..#oooo###ooo#..',
    '..#oooo#o#ooo#..',
    '..#oooo###ooo#..',
    '..#oooooooooo#..',
    '..#oooooooooo#..',
    '..#oooooooooo#..',
    '..############..',
    '................',
    '................',
  ],
  // The example app's icon in the demonstration.
  tides: [
    '................',
    '.##############.',
    '#oooooooooooooo#',
    '#oooooooooooooo#',
    '#oo###ooo###ooo#',
    '#o#ooo#o#ooo#oo#',
    '##ooooo#ooooo#o#',
    '#oooooooooooooo#',
    '#oo###ooo###ooo#',
    '#o#ooo#o#ooo#oo#',
    '##ooooo#ooooo#o#',
    '#oooooooooooooo#',
    '#oooooooooooooo#',
    '.##############.',
    '................',
    '................',
  ],
  check: [
    '........',
    '.......#',
    '......##',
    '#....##.',
    '##..##..',
    '.####...',
    '..##....',
    '........',
  ],
  // The close box's glyphs when it rolls a window up and back down.
  shade: [
    '.......',
    '#######',
    '.......',
    '#######',
    '.......',
    '#######',
    '.......',
  ],
  unshade: [
    '#######',
    '#ooooo#',
    '#ooooo#',
    '#ooooo#',
    '#ooooo#',
    '#ooooo#',
    '#######',
  ],
  close: [
    '#.....#',
    '.#...#.',
    '..#.#..',
    '...#...',
    '..#.#..',
    '.#...#.',
    '#.....#',
  ],
};

// One SVG path per colour, with a run of same-coloured pixels on a row
// merged into one rectangle, so an icon is two elements, not hundreds.
const toPath = (rows, char) =>
  rows
    .flatMap((row, y) => {
      const runs = [];
      let x = 0;
      while (x < row.length) {
        if (row[x] !== char) {
          x += 1;
          continue;
        }
        const start = x;
        while (row[x] === char) x += 1;
        runs.push(`M${start} ${y}h${x - start}v1h${start - x}z`);
      }
      return runs;
    })
    .join('');

const PixelIcon = ({ name, scale = 3, className, title }) => {
  const rows = ICONS[name];
  const width = rows[0].length;
  const height = rows.length;
  return (
    <svg
      className={className}
      width={width * scale}
      height={height * scale}
      viewBox={`0 0 ${width} ${height}`}
      shapeRendering="crispEdges"
      role={title ? 'img' : undefined}
      aria-label={title}
      aria-hidden={title ? undefined : true}
    >
      <path d={toPath(rows, 'o')} fill="var(--paper)" />
      <path d={toPath(rows, '#')} fill="var(--ink)" />
    </svg>
  );
};

export default PixelIcon;
