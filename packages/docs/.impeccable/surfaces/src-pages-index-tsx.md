---
version: 1
slug: "src-pages-index-tsx"
primary_target: "src/pages/index.tsx"
related_targets: []
---

# Landing page (src/pages/index.tsx)

Mode: Persuade. Audience: web developers who want to ship iOS and Android apps without installing Xcode, a JDK or the Android SDK; framework authors; would-be contributors. Job: understand what OmniBuild will do and how, believe the mechanism (repackage a prebuilt shell, rewrite config, sign), and follow the project. Primary action: Watch on GitHub. Second: Read the plan (issue #1). Constraints: planning stage, nothing built; every claim comes from README, the plan or the e2e README; the repackaging demo is labelled as a demonstration. User steer from the direction rounds: more spectacle and motion, less literal metaphor; React Bits animations required.

The docs pages (Read mode) inherit the same one-bit world with restraint: plain white paper for prose, menu-bar navbar, one-bit chrome only on code, admonitions, tables and controls.

## Direction contract

THESIS: The site is a working one-bit desktop. OmniBuild is shown doing its job in the medium's own rituals (select, drag onto an application, a progress window, new files on the desktop) rather than as a hero headline over feature cards.

OWN-WORLD: Black and white pixels only; every grey is an ordered dither (25, 50, 75%). Windows are white rectangles with 1px black chrome, striped title bars, square close boxes, and a hard one-bit drop shadow. Our own pixel display face for titles, a crisp pixel face for labels, our own 1-bit icons (no Apple marks, no Chicago, no Finder or Trash icons). Pressing inverts, selections march, disabled is dimmed dither, chosen is a checkmark, focus is marching ants.

STORY: A visitor sees, in one viewport, that OmniBuild turns an app's files plus a prebuilt shell into a signed .ipa and .apk with no toolchain, watches it happen (labelled as a demonstration), then reads the before and after, where it runs, and the roadmap, and follows on GitHub.

FIRST VIEWPORT: A menu bar carries the nav (OmniBuild, Docs menu, The plan, GitHub) and a status line (Planning, Phase 0). Desktop ground is a live React Bits Dither field in black and white. Front window "OmniBuild" holds the headline at display scale, the one-line explanation, the default button Watch on GitHub and the button Read the plan. A second window "Repackage" runs the demo: the Tides folder is selected with marching ants, dragged onto the OmniBuild application icon, zoom rectangles open the window, a striped progress bar walks through Info.plist, AndroidManifest.xml, resources.arsc, Assets.car and the signatures, then Tides.ipa and Tides.apk land on the desktop. Icons sit in a column at the right.

FORM: One-bit desktop (medium-native-one-bit-desktop), dealt challenger from round 1, adopted by the user after four rounds; seed key fe75d538. Signature interaction: drag the app's folder onto OmniBuild to repackage it, zoom rectangles and dither moire as motion grammar.

FINISH: unreviewed and undocumented is unfinished; this build ends with the finish review, the verdict, DESIGN.md, and every shipping raster carrying its provenance
