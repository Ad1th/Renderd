# Renderd Brand Identity

> Version 1.0.0 · August 2026

---

## Design Philosophy

Renderd's visual identity is derived directly from its engineering philosophy:
**zero overhead, precision timing, peer-to-peer.**

The design language uses **terminal geometry** — every shape is pixel-aligned,
every element is grid-constructed, nothing is decorative that isn't functional.
This mirrors the codebase itself: Rust, no allocations where none are needed.

---

## Color Palette

### Primary Palette

| Role               | Name             | Hex       | HSL                  | Usage                                  |
|--------------------|------------------|-----------|----------------------|----------------------------------------|
| Background         | `--bg-void`      | `#0D0D0D` | `hsl(0, 0%, 5%)`     | Primary canvas, icon backgrounds       |
| Background Raised  | `--bg-surface`   | `#111111` | `hsl(0, 0%, 7%)`     | Card surfaces, icon frames             |
| Grid Line          | `--bg-grid`      | `#1A1A1A` | `hsl(0, 0%, 10%)`    | Graph-paper grid lines                 |
| Primary Text       | `--text-primary` | `#E8F4FD` | `hsl(206, 76%, 95%)` | Wordmark, headings, primary content    |
| Secondary Text     | `--text-muted`   | `#6B8599` | `hsl(206, 18%, 52%)` | Taglines, descriptions, muted labels   |
| Accent Cyan        | `--accent`       | `#00E5FF` | `hsl(188, 100%, 50%)`| Datagram pixel, underlines, highlights |
| Accent Border      | `--accent-dim`   | `#003D45` | `hsl(188, 100%, 14%)`| Subtle accent borders, backgrounds     |

### Semantic Palette

| Role               | Hex       | Usage                              |
|--------------------|-----------|------------------------------------|
| Status: OK         | `#4CAF50` | MIT badge, passing CI, success     |
| Status: Warning    | `#FF6B35` | Rust/MSRV badge, pre-release       |
| Status: Info       | `#8888FF` | Pre-release, milestone tracking    |
| QUIC/Network       | `#4444FF` | Protocol badges, network status    |

### Rules

- **Never** use generic gradients (`linear-gradient` with arbitrary colors)
- **Never** use drop shadows; use border or opacity instead
- Backgrounds are **always** near-black (`#0D0D0D` or `#111111`)
- The cyan accent (`#00E5FF`) appears **at most once** per composition as a focal point
- All other elements are shades of `#E8F4FD` through `#1A1A1A`

---

## Typography

### Primary Typeface — Monospace (Required)

```
JetBrains Mono · https://www.jetbrains.com/legalnotice/fonts/
```

**Why:** Purpose-built for developer tools. Excellent hinting at small sizes,
distinct zero/O differentiation, and has the precise mechanical feel that
matches Renderd's engineering identity.

**Weights used:**
- `400` — body text, taglines, comments
- `600` — labels, badges, secondary headings
- `700` — wordmark, primary headings

**Fallback stack:**
```css
font-family: 'JetBrains Mono', 'Fira Code', 'Cascadia Code', 'SF Mono',
             'Inconsolata', 'Courier New', monospace;
```

### Secondary Typeface — Optional Prose

If prose documentation requires a proportional typeface (README, GitHub pages):

```
Inter · https://rsms.me/inter/
```

**Why:** The canonical "engineering document" sans-serif. Pairs cleanly with
JetBrains Mono without competing for visual authority.

**Fallback stack:**
```css
font-family: 'Inter', 'IBM Plex Sans', system-ui, -apple-system, sans-serif;
```

### Type Scale

| Level       | Size  | Weight | Usage                        |
|-------------|-------|--------|------------------------------|
| Display     | 120px | 700    | Social banner wordmark       |
| Heading 1   | 52px  | 700    | Logo wordmark                |
| Heading 2   | 32px  | 700    | Section headers              |
| Heading 3   | 24px  | 600    | Subsection, card headers     |
| Body        | 16px  | 400    | Documentation prose          |
| Caption     | 12px  | 400    | Badge labels, footnotes      |
| Micro        | 10px  | 400    | Icon labels (icon set)       |

### Letter Spacing

- Wordmarks: `letter-spacing: -1px` to `-3px` (tighten at large sizes)
- Badges/labels: `letter-spacing: 0` to `+0.5px`
- ALL CAPS labels: `letter-spacing: +2px` to `+4px`

---

## Logo System

### Primary Logo (`logo.svg`)

Full horizontal lockup: icon mark + wordmark. Use as the primary identity
in README headers, documentation, and project introductions.

- Minimum width: **240px**
- Clear space: **½ icon height** on all sides
- Background requirement: Dark (`#0D0D0D` or darker)

### Icon Mark (`icon.svg`)

Standalone icon without wordmark. Use for:
- GitHub repository avatar
- App icon (macOS .icns, Windows .ico)
- Compact README badges
- Minimum size: **32×32px**

### Monochrome Logo (`logo-monochrome.svg`)

White-on-black icon, black wordmark. Use for:
- Print (letterhead, stickers, press kits)
- Light backgrounds
- Single-color contexts

### Favicon (`favicon.svg`)

Optimized 32×32 pixel-grid version for browser tabs and bookmarks.
Browsers supporting SVG favicons use this directly; for `.ico` fallbacks,
export from Inkscape or a raster tool at 16, 32, and 48px.

---

## Icon Set

Renderd icons follow the **terminal-grid** construction rule:

1. Every icon is designed on a **24×24** base grid
2. All strokes are **2px** width
3. Corner radius: **0** (sharp) or **1-2px** only for visual aliasing
4. Fills use `#E8F4FD` for strokes, `#00E5FF` for a single accent element per icon
5. Icons are **not** filled — outline style only

### Included Icons (`icon-set.svg`)

| Icon    | Description                                    |
|---------|------------------------------------------------|
| stream  | Datagram/frame stream flow with forward arrow  |
| p2p     | Peer-to-peer link with bidirectional arrows    |
| encode  | Video codec/frame (film-frame + play triangle) |
| config  | Layered configuration sliders (`.toml` files)  |

### Recommended External Icon Libraries

When additional icons are needed, prefer:

1. **[Lucide](https://lucide.dev/)** — MIT licensed, minimal, outline-only. Best compatibility.
2. **[Phosphor Icons](https://phosphoricons.com/)** — Engineering-friendly, consistent weight.
3. **[Heroicons](https://heroicons.com/)** — Tailwind-adjacent but fully standalone, clean outline set.

**Avoid:** Font Awesome, Material Icons (too branded/rounded), any filled-style sets.

---

## Social Assets

### GitHub Social Banner (`social-banner.svg`)

- Dimensions: **1280×640**
- Usage: GitHub → Repository → Settings → Social preview
- Convert to PNG before uploading (GitHub does not accept SVG for social preview)
- Recommended tool: `rsvg-convert -w 1280 -h 640 social-banner.svg -o social-banner.png`

### Open Graph Image (`og-image.svg`)

- Dimensions: **1200×630**
- Usage: `<meta property="og:image" content="...">` on docs or website pages
- Convert to PNG similarly

---

## Asset File Tree

```text
renderd/
└── assets/
    └── brand/
        ├── logo.svg            ← Primary horizontal lockup (480×120)
        ├── logo-monochrome.svg ← Monochrome variant (480×120)
        ├── icon.svg            ← Standalone icon mark (120×120)
        ├── favicon.svg         ← Browser favicon (32×32)
        ├── social-banner.svg   ← GitHub social preview (1280×640)
        ├── og-image.svg        ← Open Graph image (1200×630)
        ├── icon-set.svg        ← System icon set preview (480×120)
        └── BRAND.md            ← This document
```

---

## Do / Don't

| Do ✅                                           | Don't ❌                                    |
|--------------------------------------------------|----------------------------------------------|
| Use JetBrains Mono for all identity text        | Use system-ui or sans-serif in the logo      |
| Keep icon on dark background always             | Place the icon on white without using mono variant |
| Use `#00E5FF` sparingly — one accent per layout | Use cyan as a fill for large areas           |
| Snap all shapes to the 8px grid                 | Use non-grid-aligned coordinates             |
| Use solid strokes, sharp corners                | Add drop-shadows, glows, or blur effects     |
| Prefer outline icons from Lucide/Phosphor       | Use emoji or filled-style icon sets          |

---

## Commit Convention for Brand Changes

Brand asset changes use the `brand(...)` scope per Conventional Commits:

```
brand(logo): add SVG primary wordmark and icon mark
brand(banner): create GitHub social preview 1280×640
brand(readme): integrate logo header and badge strip
brand(icons): add stream/p2p/encode/config icon set
```

---

*Renderd brand identity · MIT License · maintained by the Renderd contributors*
