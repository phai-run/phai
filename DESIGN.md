---
name: phai
description: "φ phai — finanças da casa, inteligência de verdade. Rules-first, LLM-neutral personal finance agent."
colors:
  bg: "#FFFFFF"
  surface: "#F6F6FB"
  purple: "#6D4AFF"
  cyan: "#0D9488"
  amber: "#B45309"
  rose: "#E11D48"
  green: "#15803D"
  ink: "#15131F"
  muted: "#5B5B70"
  muted2: "#9A9AAE"
  border: "#E5E3EF"
typography:
  display:
    fontFamily: "Space Grotesk"
    fontSize: "clamp(2.2rem, 7vw, 5rem)"
    fontWeight: 700
    lineHeight: 1.1
    letterSpacing: -0.03em
  body-lg:
    fontFamily: Inter
    fontSize: "18px"
    fontWeight: 400
    lineHeight: 1.7
  body-md:
    fontFamily: Inter
    fontSize: "15px"
    fontWeight: 400
    lineHeight: 1.6
  mono:
    fontFamily: "JetBrains Mono"
    fontSize: "13px"
    fontWeight: 400
    lineHeight: 1.85
  label-caps:
    fontFamily: Inter
    fontSize: "12px"
    fontWeight: 600
    lineHeight: 1
    letterSpacing: 0.15em
  phi-display:
    # Canonical φ is the embedded vector path in assets/brand/*.svg.
    # Playfair Display ships no φ glyph; live HTML text falls back to this
    # high-contrast italic serif stack, which does carry φ.
    fontFamily: "Georgia, 'Times New Roman', serif"
    fontSize: "22vw"
    fontWeight: 700
    fontStyle: italic
    lineHeight: 1
rounded:
  sm: 8px
  md: 12px
  lg: 14px
  xl: 16px
  full: 9999px
spacing:
  unit: 8px
  card-padding: 20px
  section-gap: 80px
  container: 24px
motion:
  breathe:
    target: "the hero φ only"
    property: brightness
    duration: 6s
    easing: ease-in-out
    range: "0.85 → 1.0 → 0.85"
components:
  card:
    backgroundColor: "{colors.surface}"
    textColor: "{colors.white}"
    rounded: "{rounded.lg}"
    padding: "{spacing.card-padding}"
    border: "1px solid {colors.border}"
  card-hover:
    borderColor: "{colors.purple}"
  pill:
    backgroundColor: "rgba(255,255,255,0.02)"
    textColor: "{colors.muted}"
    rounded: "{rounded.full}"
    padding: "6px 18px"
    border: "1px solid {colors.border}"
  pill-active:
    borderColor: "{colors.purple}33"
    textColor: "{colors.purple}"
  terminal:
    backgroundColor: "#050208"
    textColor: "{colors.white}"
    rounded: "{rounded.xl}"
    border: "1px solid {colors.border}"
  dna-item:
    backgroundColor: "{colors.surface}"
    textColor: "{colors.white}"
    rounded: "{rounded.lg}"
    padding: "18px 20px"
    borderLeft: "3px solid {colors.cyan}"
  dna-item-hover:
    borderLeftColor: "{colors.purple}"
---

# φ phai — Design

> Canonical brand spec. If anything elsewhere disagrees with this file, this file wins.

## The equation

**phai** (pronounced *"fai"* — "fly" without the *l*) is one word built from three:

```
φ  +  fi  +  ai  =  phai
```

- **φ** — phi, the golden ratio. Proportion, equilibrium, the number that keeps things in balance.
- **fi** — *finanças*. Household money. Real expenses, real income, real life.
- **ai** — intelligence. An agent that reads, organizes, and anticipates.

Everything in this document serves that equation. The φ is the anchor; *fi* is the data; *ai* is the layer on rails. Keep the three legible and you keep the brand.

## Who it's for

Families who think like engineers — parents and couples who manage household money, are comfortable in a terminal, and want **control of their data, not gamification**. They tried spreadsheets and abandoned them. phai is a deterministic layer that puts an LLM on rails: **rules first, AI second.** It is not a dashboard and not a "5 tips to save money" app.

## Voice

| Do | Don't |
|----|-------|
| "Gastos subiram 23%." | "Você está gastando muito!" |
| "Saldo projetado: R$ 3.247,00." | "Parabéns! Você economizou R$ 12,50!" |
| "3 assinaturas recorrentes. Revisar?" | "Olá! Vamos ver suas financinhas?" |

Four rules: **never infantilize** (the user is an engineer), **data over opinion**, **terminal-first** (everything reads in 80 columns), **precise** (`rust_decimal` end-to-end — no floating-point lies).

### Taglines

| Tagline | Use |
|---------|-----|
| seu dinheiro em equilíbrio. | primary |
| finanças da casa, inteligência de verdade. | landing / banner |
| φ = 1.618. sua família também. | geek |
| menos planilha. mais phi. | direct |

### Anti-brand

Not a bank. Not a brokerage. No gamification, no congratulations, no 🚀, no "5 tips to…". phai informs; it does not cheer.

## Color

A bright **paper** canvas with focused, AA-contrast accents, each carrying one semantic role. **One accent per view** — never compete two against each other. (The original "void" dark palette is preserved at the foot of this section as the heritage theme; the shipping web app and CLI use the light palette below.)

| Token | Hex | Role |
|-------|-----|------|
| bg | `#FFFFFF` | Paper. Every page background. |
| surface | `#F6F6FB` | Elevated cards/panels, just above the paper. |
| purple | `#6D4AFF` | Intelligence / AI — the *ai* in phai. Primary interactive. |
| cyan | `#0D9488` | Clarity / data — the *fi*. Positive financial signal / inflow. |
| amber | `#B45309` | Alerts. Used sparingly — only for things needing action. |
| rose | `#E11D48` | Danger / expense — overspend, outflow, budget exceeded. |
| green | `#15803D` | Success — within budget, positive balance, income. |
| ink | `#15131F` | Primary text. High contrast on paper. (CSS var keeps the name `--white` for parity.) |
| muted | `#5B5B70` | Secondary text, metadata. |
| muted2 | `#9A9AAE` | Decorative only (hairlines, watermark φ). **Never body text** — see Accessibility. |
| border | `#E5E3EF` | Card and panel edges. |

### token → CSS var

The same tokens drive the CLI palette, the web app, and the site. Keep these names in sync everywhere. Note `ink` maps to the CSS var **`--white`** (name kept for backward parity — it carries the primary text color, dark on paper).

| Token | CSS var | Hex |
|-------|---------|-----|
| bg | `--bg` | `#FFFFFF` |
| surface | `--surface` | `#F6F6FB` |
| purple | `--purple` | `#6D4AFF` |
| cyan | `--cyan` | `#0D9488` |
| amber | `--amber` | `#B45309` |
| rose | `--rose` | `#E11D48` |
| green | `--green` | `#15803D` |
| ink | `--white` | `#15131F` |
| muted | `--muted` | `#5B5B70` |
| muted2 | `--muted2` | `#9A9AAE` |
| border | `--border` | `#E5E3EF` |

```css
:root {
  --bg: #ffffff;
  --surface: #f6f6fb;
  --purple: #6d4aff;
  --cyan: #0d9488;
  --amber: #b45309;
  --rose: #e11d48;
  --green: #15803d;
  --white: #15131f; /* primary ink — name kept for parity */
  --muted: #5b5b70;
  --muted2: #9a9aae;
  --border: #e5e3ef;
}
```

> **Heritage (dark "void") palette** — kept for the brand mark, social cards, and the landing hero, which remain on the dark canvas: void `#08060B`, surface `#100C1A`, purple `#A78BFA`, cyan `#2DD4BF`, amber `#FBBF24`, rose `#FB7185`, green `#4ADE80`, white `#F1F5F9`, muted `#7C7C9A`, muted2 `#4A4A5E`, border `#1E1832`. The φ gradient (cyan→purple→amber) is shared by both themes.

## Typography

Four typefaces, four roles — never mix roles in one block.

| Face | Role |
|------|------|
| **High-contrast italic serif** (Georgia bold italic) | The φ symbol *only*. Ceremonial — once at the top, once at the foot of a view. Never body text. The canonical φ is an embedded vector path (see *The φ symbol*); this stack is the live-text fallback. |
| **Space Grotesk** 700/500 | Display and headings: brand name, section titles, the equation. |
| **Inter** 400/500/600 | All prose. |
| **JetBrains Mono** 400 | Code, terminal output, CLI examples, domain names, data. |

Tokens: `display` Space Grotesk 700 / clamp(2.2rem, 7vw, 5rem) / −0.03em · `body-lg` Inter 400 18px/1.7 · `body-md` Inter 400 15px/1.6 · `mono` JetBrains Mono 400 13px/1.85 · `label-caps` Inter 600 12px / 0.15em · `phi-display` high-contrast italic serif (Georgia bold italic) 22vw — but prefer the embedded vector path.

## Layout

Two layout modes, by surface:

**Editorial surfaces** (landing, README hero, brand) — single column, centered, **max-width 780px**, 80px vertical rhythm. The φ is centered, large, the first thing you see.

**The web app** (`phai serve`) — a **fluid, full-width workspace** that earns the screen on large monitors. This is a data tool, not a brochure: on a wide display the user should see the chart, the month's plan, and the transaction list together, not a narrow ribbon of whitespace.

- App shell: `max-width: min(1680px, 96vw)`, centered, 24–32px gutters. Never a fixed 780px cap in the app.
- Responsive grid by breakpoint (content-driven, not device-driven):
  - `< 900px` — single column, stacked.
  - `900–1280px` — two columns (e.g. chart/plan beside the list).
  - `> 1280px` — multi-pane: a primary work area plus a sticky side rail for filters + running totals.
- Use CSS grid with `minmax()` + `clamp()` so panels grow with the viewport; avoid hard pixel widths.
- Density over decoration: tables and lists use the mono face and tight rows; whitespace serves scanning, not padding for its own sake.
- Card grid (where used): `repeat(auto-fit, minmax(200px, 1fr))`, 12px gap. Cards: 20px padding, 1px border, 14px radius.

## Web app — interaction model

The app is **LiveStore-first**: reads are reactive queries over an in-browser SQLite, writes are committed locally (optimistic) and flushed to the Rust bridge → BigQuery/SQLite. **Every interaction is instant — zero perceptible delay.** Never block the UI on the network; reflect the change immediately and reconcile on ack.

- **Unified Caixa + Previsões.** One view. The cash-evolution bar chart is the spine; **clicking a month's bar selects that month**, and the panel below shows that month's transactions and forecasts. Selecting a month is pure client state (instant). The selected month drives the whole view.
- **Forecast on the bars.** Each month's bar **stacks realized over forecast** in distinct fills (realized = solid accent; forecast = the same hue at a lighter tint / hatch). Hovering a bar opens a **popover** listing that month's forecasts (description · amount), inflow/outflow split, and the projected close.
- **Filters with live sums.** A persistent filter bar drives the list: by **category**, **unreviewed only**, **subscriptions** (`assinaturas`), and **installments** (`payment_status = installment`). Every active filter shows the **running sum of expenses / income** for the current selection, recomputed reactively from LiveStore (no round-trip).
- **Drag-and-drop planning.** Manual forecast expenses (everything **except** card installments and subscriptions) are **draggable between months** on the chart/plan. Dropping re-dates the forecast; the bars, the projected balance line, and all totals **update in real time** as you drag. The write flushes in the background. Installments and subscriptions are visually locked (not draggable).
- **Running totals everywhere.** Sums (per filter, per month, projected close) are derived queries — they recompute the instant the underlying data changes, including mid-drag.

The φ remains the anchor in the header (small, gradient), but the app is a workspace — the data is the hero here, not the glyph.

## Depth & shape

Depth comes from **tonal contrast, not shadows**: `surface` sits just above the `bg` paper, with 1px `border` for edge definition; hover shifts the border to purple. No glassmorphism. Flat and precise, like a terminal. (A single, very soft shadow is permissible on a dragged element to signal lift — the one exception, and only while dragging.)

Shape language is **soft-rectilinear**: cards 14px, terminal 16px, pills fully rounded (9999px), DNA items 14px with a 3px cyan left border.

## Motion

**Ambient** motion is almost absent — restraint *is* the aesthetic. The only thing that moves on its own is the hero φ breathing (brightness `0.85 → 1.0 → 0.85`, 6s `ease-in-out`, infinite). Nothing else animates ambiently.

**Functional** motion — feedback the user *causes* — is encouraged in the app, and must feel immediate:
- Hover/selection transitions ~120–150ms (border, fill, tint).
- Drag-and-drop: the dragged forecast follows the cursor 1:1; drop targets highlight; bars and totals update live, in the same frame as the data change (LiveStore reactivity, no spinner).
- Popovers appear on hover with no entrance delay.

Always honor `prefers-reduced-motion: reduce` — disable the breathe and any non-essential transition; functional feedback may remain but without easing flourishes.

```css
@keyframes breathe { 0%,100% { filter: brightness(0.85); } 50% { filter: brightness(1); } }
.phi-hero { animation: breathe 6s ease-in-out infinite; }
@media (prefers-reduced-motion: reduce) { .phi-hero { animation: none; } }
```

## Accessibility

- **Body text is `ink` (`#15131F`) or `muted` (`#5B5B70`) on paper (`#FFFFFF`).** Both clear WCAG AA. Accent colors used as text (amounts, links) are the darkened light-palette values, chosen for AA on white.
- **`muted2` (`#9A9AAE`) on paper is low contrast — decorative only.** Hairlines, the watermark φ, inert ornament. Never set it on text a user must read.
- Interactive elements (CTA pills, links) need a visible `:focus-visible` state — a 1px purple ring is enough.
- The breathe animation respects `prefers-reduced-motion` (see Motion).

## Iconography & emoji

**One rule, no exceptions:** decorative iconography uses **monoline glyphs**, not emoji. Emoji appear *only inside simulated terminal output*, where they mirror what the real CLI prints.

- Decorative set (UI, cards, section markers): `φ` `⊹` `⌨` `◇` `·` `→`. Monochrome, weight-matched to the text, drawn in `muted`/`cyan`/`purple` per the one-accent rule.
- Terminal blocks may use the emoji the CLI itself emits (e.g. category rows in `phai pulse`). They live behind the mono font, inside a terminal window — that is the only sanctioned home for color emoji.

> Why: emoji rendering varies per platform and undercuts the precise, engineered feel everywhere except the terminal, where they read as genuine program output.

## The φ symbol — rendering

The φ must look **identical on every surface** regardless of installed fonts. The canonical shape is the **embedded vector `<path>`** shipped in `assets/brand/`, extracted from a high-contrast italic serif (Georgia bold italic).

> ⚠️ **Playfair Display ships no φ glyph** (the family carries only Δ, Ω, μ, π). Any `font-family: "Playfair Display"` φ silently falls back to another serif — so never trust a font for the φ. Use the vector path.

- In **SVG assets** (logo, banner, favicon, OG image) the φ is an **embedded vector `<path>`**, never a `<text font-family=…>` reference. This removes the font dependency entirely.
- In **live HTML**, if the φ must be real text rather than the path, set it in the high-contrast italic serif stack (`Georgia, "Times New Roman", serif`) — a font that actually carries the glyph — not Playfair.
- **Favicon** → `assets/brand/phai-logo.svg` (256×256, rounded 48px, gradient-filled φ on void).
- **OG / social card** → `assets/brand/phai-banner.svg` (1200×630, φ + wordmark + tagline + `phai.run`).
- The φ gradient is cyan → purple → amber (the three legs of the equation), top-left to bottom-right.

## Components

- **Card** — surface bg, 1px border, 14px radius, 20px padding; border → purple on hover.
- **Terminal** — `#050208`, 16px radius, 1px border, JetBrains Mono, traffic-light chrome. Semantic classes: `.c-p` purple prompt, `.c-c` cyan data, `.c-a` amber alert, `.c-g` green success, `.c-r` rose danger, `.c-m` muted.
- **Pill** — fully rounded, 1px border, muted text; active pill takes a purple tint.
- **DNA item** — surface bg, 14px radius, 3px cyan left border → purple on hover.
- **Equation** — `φ + fi + ai = phai` as bordered surface boxes joined by `+` / `=` operators.

## CLI identity

`phai` is the binary. The `.run` in `phai.run` is the verb — *execute, roda, faça* — though the shipping commands are `phai sync`, `phai report`, `phai pulse`, etc. (no `run` subcommand today; add one deliberately if ever).

`--version` renders plain text (it gets piped and screenshotted — no ANSI):

```
φ phai v<version>
finanças da casa, inteligência de verdade.
phai.run · github.com/phai-run/phai
```

## Naming architecture (future)

`phai.run` (site) · `app.phai.run` (web app) · `api.phai.run` · `docs.phai.run` · `github.com/phai-run`.

## Quick don'ts

- Don't use more than one accent per section.
- Don't use illustrations, stock photos, or happy people.
- Don't use gradients as backgrounds (the φ is the only gradient).
- Don't use decorative emoji outside terminal blocks (use monoline glyphs).
- Don't use drop shadows or blur.
- Don't set body text in `muted2`.
- Don't render the φ from a font that lacks the glyph (e.g. Playfair) — use the embedded vector path.
