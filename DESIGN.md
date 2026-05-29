---
name: phai
description: "φ phai — finanças da casa, inteligência de verdade. Rules-first, LLM-neutral personal finance agent."
colors:
  void: "#08060B"
  surface: "#100C1A"
  purple: "#A78BFA"
  cyan: "#2DD4BF"
  amber: "#FBBF24"
  rose: "#FB7185"
  green: "#4ADE80"
  white: "#F1F5F9"
  muted: "#7C7C9A"
  muted2: "#4A4A5E"
  border: "#1E1832"
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

A near-black **void** with neon accents, each carrying one semantic role. **One accent per view** — never compete two against each other.

| Token | Hex | Role |
|-------|-----|------|
| void | `#08060B` | Canvas. Every page background. |
| surface | `#100C1A` | Elevated cards, just above the void. |
| purple | `#A78BFA` | Intelligence / AI — the *ai* in phai. Primary interactive. |
| cyan | `#2DD4BF` | Clarity / data — the *fi*. Positive financial signal. |
| amber | `#FBBF24` | Alerts. Used sparingly — only for things needing action. |
| rose | `#FB7185` | Danger — overspend, budget exceeded. |
| green | `#4ADE80` | Success — within budget, positive balance. |
| white | `#F1F5F9` | Primary text. High contrast on void. |
| muted | `#7C7C9A` | Secondary text, metadata. |
| muted2 | `#4A4A5E` | Decorative only (hairlines, watermark φ). **Never text** — see Accessibility. |
| border | `#1E1832` | Card and terminal edges. |

### token → CSS var

The same tokens drive the CLI palette, the web app, and the site. Keep these names in sync everywhere.

| Token | CSS var | Hex |
|-------|---------|-----|
| void | `--bg` | `#08060B` |
| surface | `--surface` | `#100C1A` |
| purple | `--purple` | `#A78BFA` |
| cyan | `--cyan` | `#2DD4BF` |
| amber | `--amber` | `#FBBF24` |
| rose | `--rose` | `#FB7185` |
| green | `--green` | `#4ADE80` |
| white | `--white` | `#F1F5F9` |
| muted | `--muted` | `#7C7C9A` |
| muted2 | `--muted2` | `#4A4A5E` |
| border | `--border` | `#1E1832` |

```css
:root {
  --bg: #08060B;
  --surface: #100C1A;
  --purple: #A78BFA;
  --cyan: #2DD4BF;
  --amber: #FBBF24;
  --rose: #FB7185;
  --green: #4ADE80;
  --white: #F1F5F9;
  --muted: #7C7C9A;
  --muted2: #4A4A5E;
  --border: #1E1832;
}
```

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

Single column, centered, **max-width 780px**. No sidebars, no multi-column except card grids. Content breathes on an **80px vertical rhythm**.

- Container: 24px horizontal padding, 780px max.
- Card grid: `repeat(auto-fit, minmax(180px, 1fr))`, 12px gap.
- Cards: 20px padding, 1px border, 14px radius.

The φ is always centered, always large, always the first thing you see — the anchor of every view.

## Depth & shape

Depth comes from **tonal contrast, not shadows**: `surface` sits just above `void`, with 1px `border` for edge definition; hover shifts the border to purple. No drop shadows, no blur, no glassmorphism. Flat and precise, like a terminal.

Shape language is **soft-rectilinear**: cards 14px, terminal 16px, pills fully rounded (9999px), DNA items 14px with a 3px cyan left border.

## Motion

Motion is almost absent — restraint *is* the aesthetic. **Exactly one thing moves: the hero φ breathes.**

- Property: `brightness` (or opacity), `0.85 → 1.0 → 0.85`.
- Duration: 6s, `ease-in-out`, infinite.
- Nothing else animates on its own. Hover transitions (border color, ~150ms) are fine; ambient motion is not.
- Always honor `prefers-reduced-motion: reduce` — disable the breathe entirely.

```css
@keyframes breathe { 0%,100% { filter: brightness(0.85); } 50% { filter: brightness(1); } }
.phi-hero { animation: breathe 6s ease-in-out infinite; }
@media (prefers-reduced-motion: reduce) { .phi-hero { animation: none; } }
```

## Accessibility

- **Body text is `white` (`#F1F5F9`) or `muted` (`#7C7C9A`) on void.** Both clear WCAG AA.
- **`muted2` (`#4A4A5E`) on void is ~2:1 contrast — decorative only.** Hairlines, the watermark φ, inert ornament. Never set it on text a user must read.
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
