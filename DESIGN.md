---
name: phai
description: "φ phai — finances of the house, true intelligence. Rules-first, LLM-neutral personal finance agent."
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
    fontFamily: "Playfair Display"
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

## Brand & Style

**phai** (pronounced "fai") is a personal finance agent for families who think like engineers. It is not a dashboard. It is not a "5 tips to save money" app. It is a deterministic layer that puts your LLM on rails — rules first, AI second.

The brand sits at the intersection of three concepts: **φ** (phi — the golden ratio, divine proportion, universal balance), **fi** (finanças — household money, real-life expenses), and **ai** (intelligence — an agent that understands, organizes, and anticipates).

The visual identity is **deep void with neon accents** — dark, focused, precise. Purple for intelligence, cyan for clarity, amber for alerts. No gradients for gradients' sake. No illustrations of happy families holding piggy banks. This is a tool for analytical minds managing household finances.

### Target Audience

Parents, couples, and families who manage household money. They are analytical, dev/geek-adjacent, comfortable with terminals and data. They want control of their data, not gamification. They tried spreadsheets and abandoned them.

### Brand Personality

- **Precise** — rust_decimal end-to-end. No floating point lies.
- **Direct** — "spending up 23%", not "you're spending too much!"
- **Technical** — CLI-first, SQL-queryable, API-accessible.
- **Light** — no judgment, no gamification, no rocket emojis.
- **Proactive** — anticipates, alerts, suggests. Doesn't wait.

## Colors

The palette is rooted in a near-black void (`#08060B`) with three neon accent colors that each serve a specific semantic role. No more than one accent per view.

- **Void (#08060B):** The canvas. Deep, focused, infinite. Used for all page backgrounds.
- **Surface (#100C1A):** Elevated cards and containers. Slightly lighter than void to create depth without borders.
- **Purple (#A78BFA):** Intelligence, AI, the agent. Used for primary interactive elements and the "ai" in phai.
- **Cyan (#2DD4BF):** Clarity, data, precision. Used for the "fi" (finanças) concept and positive financial indicators.
- **Amber (#FBBF24):** Alerts, warnings, attention. Used sparingly — only for things that require immediate action.
- **Rose (#FB7185):** Danger, overspend, budget exceeded. The "red" of the system.
- **Green (#4ADE80):** Success, within budget, positive balance.
- **White (#F1F5F9):** Primary text. High contrast against void.
- **Muted (#7C7C9A):** Secondary text, descriptions, metadata.

### Design Tokens

Colors are applied via the token system. Semantic names take priority over visual names in implementation.

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
  --border: #1E1832;
}
```

## Typography

Three typefaces, three roles. Never mix roles.

- **Playfair Display (italic 700):** The phi (φ) symbol only. Ceremonial. Appears once per view — in the hero, in the footer. Never used for body text.
- **Space Grotesk (700, 500):** Display and headings. Geometric, modern, dev-forward. Used for the brand name, section titles, and the equation `φ + fi + ai = phai`.
- **Inter (400, 500, 600):** Body text. Clean, readable, neutral. Used for all prose.
- **JetBrains Mono (400):** Code, terminal output, CLI examples, domain names, data. Monospaced precision.

### Typography Tokens

- `display`: Space Grotesk 700, clamp(2.2rem, 7vw, 5rem), -0.03em tracking
- `body-lg`: Inter 400, 18px/1.7
- `body-md`: Inter 400, 15px/1.6
- `mono`: JetBrains Mono 400, 13px/1.85
- `label-caps`: Inter 600, 12px, 0.15em tracking
- `phi-display`: Playfair Display italic 700, 22vw, used once per page

## Layout & Spacing

The layout is **single-column, centered, max-width 780px**. Content flows vertically with generous whitespace (80px between sections). No sidebars. No multi-column layouts except for grid cards.

- **Container:** 24px horizontal padding, max-width 780px.
- **Section spacing:** 80px vertical rhythm.
- **Card grid:** CSS Grid with `repeat(auto-fit, minmax(180px, 1fr))` and 12px gap.
- **Cards:** 20px internal padding, 1px border, 14px border-radius.

The phi symbol (φ) is always centered, always large, always the first thing you see. It is the anchor of every page.

## Elevation & Depth

Depth is achieved through **tonal contrast**, not shadows. The `surface` color (`#100C1A`) sits slightly above the `void` background (`#08060B`). Cards use 1px borders (`#1E1832`) for edge definition. Hover states shift the border color to purple (`#A78BFA`) for interactive feedback.

No drop shadows. No blur. No glassmorphism. The aesthetic is **flat and precise**, like a terminal.

## Shapes

The shape language is **soft-rectilinear**. Cards and interactive elements use `14px` border radius. Pills (tags, badges) use `9999px` (fully rounded). Terminal windows use `16px`.

- **Cards:** 14px radius
- **Pills:** 9999px radius
- **Terminal:** 16px radius with 1px border
- **DNA items:** 14px radius, 3px cyan left border

## Components

### Cards

Standard container for content sections. Void background (`#08060B`), 1px muted border, 14px radius, 20px padding. On hover, border shifts to purple.

### Terminal

Simulated CLI output. Darker than surface (`#050208`), 16px radius, 1px border, JetBrains Mono font. Window chrome (traffic light dots) in the top bar. Content uses semantic color classes: `.c-p` (purple prompt), `.c-c` (cyan data), `.c-a` (amber alerts), `.c-g` (green success), `.c-r` (rose danger), `.c-m` (muted secondary).

### Pills

Small rounded tags for categorizing concepts. 100% border-radius, 1px border, muted text. Active/featured pills use purple tint.

### DNA Items

List items that define brand attributes. Left border accent (3px cyan), surface background, 14px radius. On hover, left border shifts to purple.

### Equation

The brand equation `φ + fi + ai = phai` is displayed as a horizontal flex row of bordered boxes connected by `+` and `=` operators. Each box is a surface card with 12px radius.

## Do's and Don'ts

- Do use the phi symbol (φ) as the single ceremonial element — once at the top, once at the bottom
- Do use cyan for positive financial data and purple for AI/intelligence concepts
- Do keep the terminal aesthetic: monospace, dark, precise
- Do show real CLI examples, not mockups
- Don't use more than one accent color per section
- Don't use illustrations, stock photos, or happy people
- Don't use gradients as backgrounds (gradients are for the phi symbol only)
- Don't use emojis except in terminal output examples
- Don't use drop shadows or blur effects
- Don't mix serif and sans-serif in the same text block
- Never use Playfair Display for anything except the φ symbol
