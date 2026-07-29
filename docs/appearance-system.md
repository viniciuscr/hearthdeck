# Appearance System

Hearthdeck separates color identity from background treatment. A palette should
not implicitly choose a decorative pattern, and a backdrop should not change
the meaning of status, focus, or actions.

## Tokens

`TvPalette` provides semantic roles rather than page-specific color values. It
uses a fixed role scale inspired by Radix-style step allocation, not Material
seed generation:

- `canvas`, `surface`, `surfaceMuted`, and `surfaceRaised` form the neutral
  elevation ladder. Surfaces always get lighter as they stack on a dark canvas.
- `borderSubtle` and `borderStrong` are separate from text opacity so dividers,
  control boundaries, and focus-adjacent edges remain intentional.
- `primaryText` and `secondaryText` carry readable content.
- `focus` is reserved only for remote focus rings and focused fills.
- `action`, `actionMuted`, and `onAction` are for primary actions and selected
  states. They are never reused as focus or success colors.
- `success`, `warning`, and `info` retain semantic meaning across every
  palette and are never decorative accents.

Curated palettes are tuned dark families: Aurora, Ember, Indigo, and Noir.
Noir uses near-black neutrals, cool-silver focus, and restrained violet actions.
System colors adapt platform-provided colors into the same roles. Artwork
gradients are content data and do not use the application palette.

## Backdrops

Every application route uses `TvBackdrop`, with a route-specific origin only
for the Edge wash treatment.

| Treatment | Intent | Cost |
| --- | --- | --- |
| Solid | Pure neutral canvas for the clearest reading environment. | One fill. |
| Edge wash | A restrained static field that gives a route subtle depth. | One radial gradient. |
| Quiet grid | A low-alpha architectural pattern with no semantic content. | One static custom paint. |

Backdrops are static. Do not add video, shaders, procedural animation, image
downloads, wallpaper extraction, or periodic repainting here. Content detail
pages may continue to use item artwork as a separate, content-led treatment.

## Accessibility

Text and controls must remain readable without relying on hue. Curated palettes
are tested at 4.5:1 for primary and secondary text, and 3:1 for focus/action
indicators. Focus has both a high-contrast border and a surface change.

## Persistence And Sync

The daemon stores the canonical host-user appearance settings in the existing
SQLite database. The client keeps a compact versioned `SharedPreferences`
snapshot containing the palette, backdrop, daemon revision, and pending flag.

1. Startup reads only the local snapshot, so normal launch has no network or
   daemon wait and no late palette flicker.
2. A user change writes the complete appearance snapshot locally before the
   visible theme changes.
3. One asynchronous, bounded request saves that snapshot to the daemon.
4. A failed write remains pending and is retried once after the next launch.
5. A daemon revision conflict returns the canonical revision; the client
   retries its complete local appearance choice once with that revision.

There is no polling and no persistent bearer token. Refreshing remote state is
reserved for an explicit user action or a conflict response, so constrained
hardware does not repeatedly pair, read, or repaint while idle.

## Sources

- W3C WCAG 2.2 contrast minimum: <https://www.w3.org/WAI/WCAG22/Understanding/contrast-minimum.html>
- Refactoring UI, palette scales and neutral-first construction: <https://www.refactoringui.com/previews/building-your-color-palette>
- Radix Colors, step-to-role allocation: <https://www.radix-ui.com/colors/docs/palette-composition/understanding-the-scale>
- Fluent 2, neutral, brand, and semantic palette separation: <https://fluent2.microsoft.design/color>
- Open Color, fixed UI-oriented scales: <https://yeun.github.io/open-color/>
