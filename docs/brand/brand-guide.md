# CodeCAD Brand Guide

Agent-first 2D CAD. State intent, generate precise geometry.

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

**Rule: amber appears ONLY on the crosshair element.** Buttons, links,
highlights, selections all use white or gray. The crosshair owns the
color exclusively. This is what makes it the brand's signature.

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
| Code compiled to live geometry. | Technical docs, README |
| State intent. Generate precise CAD. | Marketing, landing page |
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
