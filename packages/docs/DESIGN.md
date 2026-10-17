---
name: OmniBuild
description: The one-bit desktop. Ink and paper only, every grey a dither, windows with hard shadows.
colors:
  ink: '#000000'
  paper: '#ffffff'
typography:
  display:
    fontFamily: "'Jersey 10', 'Tiny5', monospace"
    fontSize: 'clamp(3.5rem, 6.2vw, 5.75rem)'
    fontWeight: 400
    lineHeight: 0.92
    letterSpacing: '0.005em'
  headline:
    fontFamily: "'Jersey 10', 'Tiny5', monospace"
    fontSize: 'clamp(2.25rem, 3.4vw, 3.25rem)'
    fontWeight: 400
    lineHeight: 0.95
  display-compact:
    fontFamily: "'Jersey 10', 'Tiny5', monospace"
    fontSize: 'clamp(2.5rem, 11.5vw, 4.5rem)'
    fontWeight: 400
    lineHeight: 0.92
  doc-h1:
    fontFamily: "'Jersey 10', 'Tiny5', monospace"
    fontSize: '4rem'
    fontWeight: 400
    lineHeight: 1
  doc-h2:
    fontFamily: "'Jersey 10', 'Tiny5', monospace"
    fontSize: '2.75rem'
    fontWeight: 400
    lineHeight: 1
  title:
    fontFamily: "'Jersey 10', 'Tiny5', monospace"
    fontSize: '2rem'
    fontWeight: 400
    lineHeight: 1
  title-s:
    fontFamily: "'Jersey 10', 'Tiny5', monospace"
    fontSize: '1.75rem'
    fontWeight: 400
    lineHeight: 1
  title-xs:
    fontFamily: "'Jersey 10', 'Tiny5', monospace"
    fontSize: '1.5rem'
    fontWeight: 400
    lineHeight: 1.1
  body:
    fontFamily: "'Atkinson Hyperlegible Next Variable', system-ui, -apple-system, 'Segoe UI', sans-serif"
    fontSize: '1.0625rem'
    fontWeight: 400
    lineHeight: 1.6
  body-lede:
    fontFamily: "'Atkinson Hyperlegible Next Variable', system-ui, -apple-system, 'Segoe UI', sans-serif"
    fontSize: '1.1875rem'
    fontWeight: 400
    lineHeight: 1.55
  body-s:
    fontFamily: "'Atkinson Hyperlegible Next Variable', system-ui, -apple-system, 'Segoe UI', sans-serif"
    fontSize: '1rem'
    fontWeight: 400
    lineHeight: 1.45
  caption:
    fontFamily: "'Atkinson Hyperlegible Next Variable', system-ui, -apple-system, 'Segoe UI', sans-serif"
    fontSize: '0.75rem'
    fontWeight: 400
    lineHeight: 1.35
  status:
    fontFamily: "'Atkinson Hyperlegible Next Variable', system-ui, -apple-system, 'Segoe UI', sans-serif"
    fontSize: '0.875rem'
    fontWeight: 600
    lineHeight: 1.4
  label-m:
    fontFamily: "'Tiny5', monospace"
    fontSize: '20px'
    fontWeight: 400
    lineHeight: 1.2
  label-s:
    fontFamily: "'Tiny5', monospace"
    fontSize: '15px'
    fontWeight: 400
    lineHeight: 1.3
  mono:
    fontFamily: "ui-monospace, 'Cascadia Mono', Menlo, Consolas, monospace"
    fontSize: '0.9375rem'
    fontWeight: 400
rounded:
  none: '0'
spacing:
  px: '2px'
  xs: '4px'
  sm: '8px'
  md: '12px'
  lg: '16px'
  xl: '24px'
  2xl: '32px'
  3xl: '48px'
  4xl: '64px'
components:
  button:
    backgroundColor: '{colors.paper}'
    textColor: '{colors.ink}'
    typography: '{typography.label-m}'
    rounded: '{rounded.none}'
    padding: '8px 22px'
    height: '44px'
  button-hover:
    backgroundColor: '{colors.ink}'
    textColor: '{colors.paper}'
  button-small:
    backgroundColor: '{colors.paper}'
    textColor: '{colors.ink}'
    typography: '{typography.label-s}'
    rounded: '{rounded.none}'
    padding: '2px 10px'
  window:
    backgroundColor: '{colors.paper}'
    textColor: '{colors.ink}'
    rounded: '{rounded.none}'
  window-title:
    backgroundColor: '{colors.paper}'
    textColor: '{colors.ink}'
    typography: '{typography.label-m}'
    padding: '0 12px'
    height: '34px'
  window-close-box:
    backgroundColor: '{colors.paper}'
    rounded: '{rounded.none}'
    size: '22px'
  window-status:
    textColor: '{colors.ink}'
    typography: '{typography.status}'
    padding: '6px 14px'
  icon-label:
    backgroundColor: '{colors.paper}'
    textColor: '{colors.ink}'
    typography: '{typography.label-s}'
    padding: '1px 6px'
  icon-label-selected:
    backgroundColor: '{colors.ink}'
    textColor: '{colors.paper}'
  list-head:
    backgroundColor: '{colors.ink}'
    textColor: '{colors.paper}'
    typography: '{typography.label-s}'
    padding: '8px 12px'
  list-row-head:
    textColor: '{colors.ink}'
    typography: '{typography.label-m}'
    padding: '8px 12px'
  notice:
    backgroundColor: '{colors.ink}'
    textColor: '{colors.paper}'
    typography: '{typography.status}'
    padding: '5px 10px'
  inline-code:
    backgroundColor: '{colors.paper}'
    textColor: '{colors.ink}'
    rounded: '{rounded.none}'
    padding: '0 4px'
---

# Design System: OmniBuild

## Overview

**Creative North Star: "The One-Bit Desktop"**

The site is a working one-bit desktop, not a page dressed as one. Every surface is ink or paper, and every grey is an ordered dither of the two. Content lives in windows: white rectangles with a 2px ink frame, a striped title bar, a square close box and a hard, unblurred drop shadow. The windows sit on a desktop whose ground is a live dither field, and they behave as windows did: they drag by their title bars, roll up to their title bars, and zoom open out of icons. The documentation pages use the same chrome, but read on plain paper.

One pixel of this world is two CSS pixels (`--px`). Borders, rules, shadows, outlines and pattern cells are whole multiples of it, so lines stay as crisp as the dithers next to them. Patterns are SVG tiles rendered with `image-rendering: pixelated`, never CSS gradients, because a gradient's stops antialias into a blurred grey. Interaction comes from the medium's own grammar: pressing and hovering invert, a selected icon or row is drawn in inverse video, focus is marching ants, the current item in a list is a highlighted row, and the item that comes next wears animated marching ants.

The world keeps the medium's generic grammar and drops everything that belongs to Apple: no Apple logo, no Chicago or Geneva, no Finder, Trash or happy-computer icons. Every icon, the logo and the favicon are OmniBuild's own 16 by 16 pixel grids. Nothing imitates Apple's or Google's sites or product art.

**Key Characteristics:**

- Two values, ink and paper; greys only as 12, 25, 50 and 75% ordered dithers.
- Dark mode is inverse video: ink and paper swap, and the system stays one-bit.
- Windows with 2px frames, striped title bars, square close boxes and hard ink shadows.
- Three faces with fixed jobs: Jersey 10 for display, Tiny5 for chrome and labels, Atkinson Hyperlegible Next for every sentence.
- Motion is stepped and pixel-whole: XOR outlines, zoom rectangles, marching ants, barber-pole progress.

## Colors

A strictly one-bit palette: two values, and dither patterns standing in for every tone between them.

### Neutral

- **Ink** (`{colors.ink}`): text, frames, rules, shadows, title-bar stripes, dither dots, and the fill of anything inverted (hover, pressed, selected, current row, table heads, notices).
- **Paper** (`{colors.paper}`): window bodies, the page ground on docs, the slab behind every title and icon label, and the text on anything inverted.

In dark mode (`[data-theme='dark']`) the two custom properties `--ink` and `--paper` swap, and every pattern tile is redrawn in the swapped ink, so the same rules hold with no new colours. Canvas and WebGL material (the Dither field, PixelBlast, PixelTransition) cannot read CSS variables, so they receive the same two hex values from the colour mode.

### Patterns (the only greys)

- **Dither 12** (4 by 4 tile, two dots), **Dither 25**, **Dither 50** and **Dither 75** (2 by 2 tiles): the four tones, each shown with one tile cell per `--px` (`4px 4px` for the 2 by 2 tiles). The build so far uses only Dither 50; the other three are defined for lighter and darker tones. Dither 50 is the desktop ground behind the live field, the scrollbar track, the menu-bar status square and the frame band of a read-only field.
- **Stripes** (1 by 2 tile at `2px 4px`): the active window's title bar.
- **Diagonal** (6 by 6 tile at `12px 12px`): the barber-pole fill of a running progress bar.

### Named Rules

**The Two Values Rule.** Ink and paper are the only colours. A grey, a tint, an opacity fade or a coloured accent is never introduced; a tone between them is a dither tile at a whole multiple of `--px`, rendered pixelated.

**The Inverse Video Rule.** Emphasis and state are shown by swapping ink and paper: hover, press, selection, the active nav item, the current step, table heads and notices. Dark mode is the same swap applied to the whole screen.

**The Clear Glyph Rule.** No dither, stripe or pattern ever sits under glyphs. A title on a striped bar sits on its own paper slab; an icon label sits on paper over the dither ground; inline code is a 1px ink frame, not a shaded chip; a read-only field carries its 50% dither only in a band inside its frame.

## Typography

**Display Font:** Jersey 10 (with Tiny5, monospace)
**Label/Chrome Font:** Tiny5 (with monospace)
**Body Font:** Atkinson Hyperlegible Next Variable (with system-ui, Segoe UI, sans-serif)
**Mono Font:** ui-monospace (with Cascadia Mono, Menlo, Consolas)

All three faces are self-hosted from Fontsource, so the site makes no third-party font requests.

**Character:** A heavy pixel display face for statements, a crisp 5-pixel-grid face for the desktop's words and chrome, and a reading face built for legibility for everything that is a sentence. The pixel faces carry the world; the reading face carries the meaning.

### Hierarchy

- **Display** (Jersey 10, 400, `clamp(3.5rem, 6.2vw, 5.75rem)`, line-height 0.92, max 19ch): the landing headline in the front window. On narrow screens `clamp(2.5rem, 11.5vw, 4.5rem)`.
- **Headline** (Jersey 10, 400, `clamp(2.25rem, 3.4vw, 3.25rem)`, line-height 0.95, max 24ch): the one statement opening each section window. Docs h1 is 4rem and h2 2.75rem in the same face.
- **Title** (Jersey 10, 400, 2rem, line-height 1): list titles, target names, the navbar wordmark, docs h3. Footer titles and pagination labels at 1.75rem, docs h4 at 1.5rem.
- **Body** (Atkinson, 400, 1.0625rem, line-height 1.6, max 62 to 70ch): section copy and docs prose. The landing lede is 1.1875rem at 1.55, max 28em.
- **Body S** (Atkinson, 400, 1rem, line-height 1.45): roadmap checklist sentences. **Caption** (Atkinson, 400, 0.75rem): the demonstration's step details.
- **Status** (Atkinson, 600, 0.875rem, line-height 1.4): window status lines, the demonstration notice, results and field labels: any sentence that sits in chrome.
- **Illustration text is not on the ramp.** The drawn phone in the demonstration (14px corners, 0.625 to 0.6875rem screen text, a 1.375rem app name on phones) belongs to the drawing, as the icons' pixels do; the detector's advisories on those values are expected.
- **Label M** (Tiny5, 400, 20px): window titles, buttons, menu-bar and sidebar items, dropdown items, roadmap phase names, list row heads.
- **Label S** (Tiny5, 400, 15px): icon labels, table column heads, the table of contents, code-block titles, admonition headings, small buttons, tags, the footer.

### Named Rules

**The Five-Pixel Rule.** Tiny5 is drawn on a 5px grid and is set only at 15px (`--pixel-s`) and 20px (`--pixel-m`), so every stem is a whole number of pixels. It is never bolded, never set at other sizes and never used for running copy.

**The Sentence Face Rule.** Every sentence, status lines included, is set in Atkinson Hyperlegible Next. Tiny5 is for words and labels; when a sentence sits in chrome it is Atkinson at 600.

**The One-Bit Syntax Rule.** Code has no colours to tell tokens apart, so keywords, builtins, tags and selectors are bold, comments are italic and deletions are struck through, in ink on paper.

## Layout

The landing page is a desktop: a 12-column grid (24px gutters, max 1560px, 48px side insets) plus a 112px icon column on the right, over a fixed full-viewport dither field. The first screen holds the front window across 8 columns and a second window overlapping its lower right (columns 8 to 12, pushed down by `clamp(150px, 19vh, 260px)`). Below it, one window per section is staggered across the same 12 columns with 64px row gaps and small negative margins, so windows overlap a little, as windows left open do.

- **Below 1240px** the second window moves under the first, offset right, and the icon column narrows to 96px.
- **Below 1100px** the section windows go full width, except a 7 and 5 pair.
- **Below 996px** windows tile edge to edge in one column with 16px insets and a 6px right margin so their shadows show; the icons become a wrapped row of 84px cells between windows; tables become labelled records.
- **Below 520px** the demonstration's phone column narrows to 96px.

Window bodies pad generously (28 to 40px block, 32 to 44px inline; 18 to 22px on phones). Spacing steps are 4, 8, 12, 16, 24, 32, 48 and 64px. Docs pages keep Docusaurus's layout, with prose capped at 70ch.

## Elevation & Depth

Depth is the one-bit drop shadow: a solid ink offset down and to the right with zero blur, its size a whole multiple of `--px` that grows with the object's importance. Docusaurus's own soft shadows are turned off (`--ifm-global-shadow-*: none`). There are no gradients, glows or blurred shadows anywhere. Stacking order is shown by overlap and by an inactive window losing its title-bar stripes.

### Shadow Vocabulary

- **Menu** (`box-shadow: 2px 2px 0 var(--ink)`): dropdown menus off the menu bar.
- **Panel** (`box-shadow: 4px 4px 0 var(--ink)`): docs code blocks and admonitions, which are small windows.
- **Window** (`box-shadow: 6px 6px 0 var(--ink)`): every landing-page window.
- **Default ring** (`box-shadow: 0 0 0 2px var(--paper), 0 0 0 6px var(--ink)`): the default button's thick outer ring, not a lift.

### Named Rules

**The Hard Shadow Rule.** Shadows are solid ink at `--px` multiples (2, 4, 6px) with no blur and no spread. One-bit screens had no blur, so neither does this world.

## Shapes

Every UI corner is square (`--ifm-global-radius: 0`, and buttons, close boxes, menus, code and fields at 0). Borders are 2px (`--px`) ink; the default button's ring is 4px of ink outside a 2px paper gap. Inline code is the deliberate exception at a 1px frame, light enough to sit in a line of text. Rules and table lines are 2px. The only curve on the site is the drawn phone in the demonstration (a 6px frame with 14px corners), which depicts a device rather than shaping an interface element. Dropdown carets are pixel triangles clipped from ink, not icon-font glyphs.

## Components

### Buttons

Square, framed and inverting: a push button from the medium.

- **Shape:** square corners, 2px ink frame, min-height 44px, padding 8px 22px, label in Tiny5 20px.
- **Default:** paper with ink text. The default button (the one Return would press: Watch on GitHub) adds the default ring and a 6px margin to hold it.
- **Hover / Active:** hover inverts to ink with paper text; active nudges the button 2px down and right.
- **Focus:** a 2px dashed ink outline, offset 10px so it clears the default ring.
- **Small:** the in-chrome button (Run again) at Tiny5 15px, padding 2px 10px.
- **Disabled:** the frame turns dotted; the label stays solid ink.

### Windows

The signature container: a white rectangle with chrome.

- **Frame:** 2px ink, paper body, the Window shadow.
- **Title bar:** min-height 34px, a 2px ink rule beneath, Stripes inside a paper margin; the title in Tiny5 20px sits centred on its own paper slab. An inactive window drops the stripes.
- **Close box:** 22px square, 2px ink frame, at the bar's right. Its pixel icon shows on hover and focus; pressing fills it with ink.
- **Status line:** under a 2px ink rule, padding 6px 14px, set as Status (Atkinson 600, 0.875rem).
- **Drag:** by the title bar with a mouse or pen, never by touch. A dashed XOR outline follows the pointer and the window jumps there on release.

**The Window Shade Rule.** Every close box does something a reader can undo. When the page can reopen the window (the demonstration reopens from its icon), the close box closes it with zoom rectangles back to the icon. Otherwise it rolls the window up to its title bar, and a second press rolls it down. A section is never lost to a click.

### Desktop Icons

- **Glyph:** OmniBuild's own 16 by 16 pixel grid (omnibuild, folder, document, package, tides), drawn as SVG and scaled by whole numbers.
- **Label:** Tiny5 15px on a paper slab (padding 1px 6px), in a 112px cell.
- **Hover / Selected:** the label inverts; a selected glyph is drawn inverted.
- **Focus:** marching ants (2px dashed) around the label only.

### Menu Bar (Navigation)

- **Style:** paper bar with a 2px ink rule beneath, 3rem tall. Wordmark in Jersey 10 at 2rem beside the 32px pixel logo; items in Tiny5 20px.
- **States:** hover and an open menu invert; the active page is underlined 2px when not hovered.
- **Menus:** the Docs dropdown is a framed list with the Menu shadow, items inverting on hover.
- **Status:** the project's status at the right end, printed with a 10px Dither 50 square, not a badge.

### List Views

- **Head:** ink row with paper text, Tiny5 15px, inside a 2px framed wrapper.
- **Rows:** 2px ink rules between rows; the row head in Tiny5 20px at 26% width; a row inverts under the pointer, as a selected row did.
- **Phones:** each row becomes a small record: the row head in Jersey 10 at 1.5rem, every cell prefixed by its column name, underlined.

### Progress and Checklists

- **Progress bar:** 20px tall, 2px frame with a 2px paper gap. The filled part is solid ink at rest and the Diagonal barber pole while running, walking 2px at a time; width moves in 6 steps.
- **Steps and checklists:** the current step is an inverted row; done steps carry a pixel check; roadmap checkboxes are 20px framed squares. The next phase wears marching ants (8px dashes, 4 steps) and an inverted Tiny5 15px tag.

### Inline Code and Fields

- **Inline code:** a 1px ink frame on paper, square, 0 4px padding (0 0.2em in docs).
- **Read-only field:** the not-yet-published install command, in the mono face, inside a 2px ink outline with a 6px band of Dither 50 between outline and text.

### Motion Material

Motion is pixel-whole and stepped (`steps()` timing on every animation), and `prefers-reduced-motion` reduces it all to instant.

- **XOR outlines:** drag ghosts and zoom rectangles are 2px white borders with `mix-blend-mode: difference`, so they show on ink, paper and dither alike.
- **Desktop field:** React Bits Dither, Bayer-ordered, 2px pixels, ink and paper only; the pointer erases a patch and a click ripples. The static Dither 50 shows until the first frame and without WebGL 2.
- **Closing field:** React Bits PixelBlast with softness, glow, twinkle, edge fade, jitter and noise all at 0, so it stays hard square pixels.
- **Screen change:** React Bits PixelTransition dissolves in a 14-cell grid of ink.
- **Headline reveal:** React Bits DecryptedText resolves the display headline from block glyphs; the percentage uses React Bits Counter at 20px.

## Do's and Don'ts

### Do:

- **Do** use only `{colors.ink}` and `{colors.paper}`, and make every intermediate tone a dither tile at a whole multiple of `--px` with `image-rendering: pixelated`.
- **Do** put content in windows: 2px ink frame, striped title bar with the title on a paper slab, square close box, 6px hard shadow.
- **Do** show state by inverting: hover, press, selection, current row and active item swap ink and paper.
- **Do** set Tiny5 at 15px or 20px only, and every sentence in Atkinson Hyperlegible Next (600 in chrome).
- **Do** show focus as a 2px dashed outline (marching ants), and the next or selected item with animated ants where the page can animate them.
- **Do** show a disabled control with a dotted frame and a solid label.
- **Do** give every close box an undoable action: close when the page can reopen the window, roll up otherwise.
- **Do** draw new icons as OmniBuild's own pixel grids (`#` ink, `o` paper, `.` transparent) at whole-number scales.
- **Do** time animations with `steps()` and honour `prefers-reduced-motion`.

### Don't:

- **Don't** use any colour, grey hex, opacity fade, gradient or blur.
- **Don't** put a dither, stripe or pattern under text; frame it instead.
- **Don't** round UI corners or soften shadows; shadows are solid ink offsets of 2, 4 or 6px.
- **Don't** use the Apple logo, Chicago or Geneva, or Finder, Trash or happy-computer icons, or imitate Apple's or Google's sites or product art.
- **Don't** use icon fonts or glyph icons; Docusaurus's external-link arrows are hidden for that reason.
- **Don't** set Tiny5 for running copy, bolded, or at sizes other than 15px and 20px.
- **Don't** turn status into a badge or pill; it is a printed line.
- **Don't** let a window grab touch drags; on touch the page scrolls.
