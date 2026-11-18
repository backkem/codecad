# CodeCAD Brand Guide

CodeCAD: CAD you can program. Exact 2D drawings and 3D solid models
from one script. Drawings are what you hand over. Models are where you
think.

One product, one brand. 2D and 3D are two kinds of view of the same
work, not two products: the mark, palette, type and voice below cover
both, and only the kind-of-view accent (see "Kind of view") tells a
drawing from a model.

## Brand Story

CAD interfaces haven't changed since the 1980s. The crosshair cursor,
the command line, the black viewport, vector lines on void. CodeCAD
keeps one element from that lineage: the full-screen orthogonal
crosshair. Everything else is rebuilt for agent-driven drawing, where
code is the input and live geometry is the output.

The crosshair is the brand. It represents two things simultaneously:
**precision** (the drafting heritage of exact coordinates) and
**intersection** (where code meets design, where human intent meets
agent execution). The surrounding interface steps back. The crosshair
is the singular focal point.

## Visual Identity

### Palette

Three colors. No gradients.

| Role | Color | Hex | Usage |
|------|-------|-----|-------|
| Void | Near-black | `#0A0E14` | All backgrounds (app, docs, marketing) |
| Accent | Amber | `#FFB000` | Crosshair only. The one splash of color. |
| Text | Off-white | `#E8E8E8` | Body text, geometry lines, UI chrome |

Secondary tones (use sparingly):

| Role | Hex | Usage |
|------|-----|-------|
| Muted | `#6B7280` | Disabled states, secondary labels |
| Grid | `#151A22` | Dot grid overlays, subtle backgrounds |
| Success | `#34D399` | Status indicators only |
| Error | `#EF4444` | Status indicators only |

**Rule: amber appears ONLY on the crosshair element**, and on the marks
of a Drawing (below). Buttons, links, highlights, selections all use
white or gray. This is what makes it the brand's signature.

### Kind of view

The site and the viewers say which kind of view a thing is with one
accent each. They mark the kind, not a second product.

| Kind | Color | Hex | Used on |
|------|-------|-----|---------|
| Drawing (2D: sheets, DWG) | Amber | `#FFB000` | Drawing badges, the Drawing tab and shelf filter |
| Model (3D: solid parts) | Blue | `#5EA2FF` | Model badges, the Model tab and shelf filter, the 3D viewer's Working/Presentation control |

- **Only on badges, tabs and the 3D viewer's mode control.** Never on
  buttons, links, text, geometry or backgrounds.
- **One variable.** The site defines the model accent once,
  `--codecad-model` in `site/site.css`; the embed viewer reads the same
  variable (its fallback carries the same value for other pages).
- **Why blue.** It is amber's complement (hue 216 against 41), so the two
  never read as one; it stays apart from amber for red-green colour
  blindness (the blue-yellow axis is kept); it is clear of the green and
  red status colours; contrast on the void is 7.5:1. Cyan and violet were
  the other candidates (cyan sits close to the success green, violet has
  the lowest contrast).
- **Working views use the dark background.** A 3D view in working
  quality on the site sits on the viewer's dark background, next to the
  2D viewer. Presentation renders (path traced) keep their own look:
  their environment, ground and light.

### The Crosshair

The hero element. Usage rules:

1. **Extends to edges.** Lines always run to the viewport/container
   boundary. Never floating in space.
2. **Amber only.** `#FFB000`, 1-2px weight. No glow, no shadow.
3. **Orthogonal only.** Strictly horizontal + vertical. Never rotated,
   never diagonal.
4. **Circle at intersection** (optional). A thin amber circle at the
   crossing point, used in the icon mark. Omit in app viewport.
5. **One per composition.** Never multiple crosshairs in the same image.
6. **Placement varies.** The crosshair can sit at center, off-center
   (rule-of-thirds), or in a corner. It does not need to be centered.

### Typography

| Role | Font | Weight | Source |
|------|------|--------|--------|
| Headings | Jost | 500 Medium | [Google Fonts](https://fonts.google.com/specimen/Jost) |
| Body | Jost | 400 Regular | Same |
| Code / UI | JetBrains Mono | 400 Regular | [Google Fonts](https://fonts.google.com/specimen/JetBrains+Mono) |

- Headings: Jost, geometric sans-serif. Clean, structural, no
  personality beyond precision.
- Code: JetBrains Mono everywhere code appears (command line, entity
  IDs, coordinates, API examples).
- Never use serif fonts anywhere in the brand.
- The site loads nothing from other hosts, fonts included: its CSS names
  Jost and JetBrains Mono and falls back to the system's sans and mono
  where they are not installed.

### Logo Variants

All in `docs/brand/`:

| File | Usage |
|------|-------|
| `logo.png` | Primary: crosshair + "CodeCAD" wordmark |
| `icon-mark.png` | Square mark: crosshair + circle. Favicons, avatars, app icons. |
| `docs-header.png` | Wide banner with ghosted floor plan geometry. README, docs pages. |

### App UI Rules

See `app-ui-mockup.png` for reference.

- **Viewport background**: `#0A0E14` (the void)
- **Geometry lines**: `#E8E8E8` off-white, 1px
- **Crosshair**: amber `#FFB000`, extends full viewport, tracks cursor
  or agent focus point
- **Command area**: bottom panel, `JetBrains Mono`, shows JS code
  input and output
- **Sidebar**: minimal layer list, `JetBrains Mono` small, no icons
- **Status bar**: bottom edge, entity count, coordinates, grid size,
  layer name
- **No toolbar icons.** Commands are typed, not clicked. The command
  line is the UI.

## Voice

Terse. Technical. Confident. Imperative mood.

**Do:**
- "Generate radial array."
- "State intent. Get geometry."
- "Agents write code. Geometry appears."

**Don't:**
- "Let's help you create amazing designs!"
- "Our powerful AI assistant will..."
- "Simply click the intuitive toolbar..."

Write like a commit message. The diff is the detail.

No filler words: "comprehensive", "robust", "seamless", "leverage",
"cutting-edge". If it sounds like AI wrote it, rewrite it.

## Taglines

Pick one per context:

| Tagline | Best for |
|---------|----------|
| CAD you can program. | Product line: title, landing page, README |
| Drawings are what you hand over. Models are where you think. | Next to the product line |
| Code compiled to live geometry. | Technical docs |
| State intent. Generate precise CAD. | Marketing |
| The programmable canvas. | Shorthand, social |

## Assets

```
docs/brand/
  brand-guide.md        # this file
  logo.png              # primary logo (crosshair + wordmark)
  icon-mark.png         # square icon (crosshair + circle)
  app-ui-mockup.png     # reference UI mockup
  brand-cheatsheet.png  # visual summary poster
  docs-header.png       # wide banner for docs/README
```
