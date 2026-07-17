# Gemini prompt — Eris icon (1 PNG + 1 SVG)

Copy everything inside the block below into Gemini. Attach your best existing logo image as reference.

---

```
TASK: Produce exactly TWO assets for the Eris open-source project:
  (A) one sharp hero raster image
  (B) one clean icon SVG

Do not deliver multiple concepts, mood boards, or variations unless I ask.
Pick the strongest single direction and execute it precisely.

Attach: [your current logo PNG]


═══════════════════════════════════════════════════════════════
WHAT ERIS IS
═══════════════════════════════════════════════════════════════

Eris — Greek goddess of strife; threw the golden apple of discord.
Open-source local-first AI agent: Rust binary, Markdown vault as memory,
grammar-enforced tool calls, developer instrument-panel aesthetic.

Website: eris-system.dev (light mode, monospace, flat borders).
The logo must look like it belongs in that UI header — not a generic AI brand.


═══════════════════════════════════════════════════════════════
SYMBOL (combine all of these into ONE coherent mark)
═══════════════════════════════════════════════════════════════

Required elements:
  • Apple silhouette formed by circuit-board traces (90° angles, junction nodes)
    → Apple of Discord (myth)
  • Curly braces { } centered inside the apple
    → JSON / code protocol
  • Minimal geometric eye above the stem
    → inference / judgment
  • One small amber accent on the stem tip OR one circuit node
    → golden apple (single highlight only)

Forbidden:
  • Subtitle text ("Episodic Reasoning…" or any acronym expansion)
  • Rose gold / copper metallic gradients
  • Purple, pink, neon, AI slop gradients
  • 3D emboss, drop shadows, lens flare, texture overlays
  • Robot brains, sparkles, generic LLM imagery
  • Serif or rounded "friendly" fonts


═══════════════════════════════════════════════════════════════
COLOR PALETTE — use EXACT hex values
═══════════════════════════════════════════════════════════════

PRIMARY (light background — default for both deliverables):

  Icon strokes / fills:     #0a6b55   (deep teal — main brand color)
  Wordmark "ERIS":          #1a2238   (charcoal) OR #0a6b55 (teal)
  Background (hero PNG):    #e8ecf4   (soft blue-grey) OR #ffffff
  Borders / hairlines:      #a8b4cc
  Muted tagline (optional): #4a5568
  Golden apple accent:      #9a6b00   (ONE element only — stem tip or one node)
  Soft teal tint (optional): #e6f5f0  (behind icon, subtle)

DARK variant (SVG must include this as a second <g> group or second file):

  Background:               #080a12
  Icon on dark:             #5ce5be   (mint teal)
  Amber accent on dark:     #c9a030

Rule: teal is brand. Amber is a whisper, not a second brand color.


═══════════════════════════════════════════════════════════════
DELIVERABLE A — HERO RASTER (one sharp image)
═══════════════════════════════════════════════════════════════

Format:   PNG, 2048 × 2048 px (square) OR 2400 × 1200 px (horizontal lockup)
Quality:  crisp vector-like edges — no blur, no JPEG artifacts, no compression noise
Content:  stacked lockup — icon centered above wordmark "ERIS"
          optional tiny tagline below in muted #4a5568:
          "local-first vault agent" (lowercase, small, monospace)
Background: solid #e8ecf4 or #ffffff — NO transparency, NO gradient

Composition:
  • Icon occupies ~55% of vertical space
  • "ERIS" wordmark below, monospace, all caps, letter-spacing +0.04em
  • Generous padding (min 8% margin on all sides)
  • Flat design — looks like a screenshot of a precision UI element

Export checklist:
  [ ] Edges are pixel-sharp at 100% zoom
  [ ] Teal #0a6b55 is exact (not shifted green or blue)
  [ ] Only ONE amber #9a6b00 accent visible
  [ ] No subtitle acronym
  [ ] Readable at 400px wide (hero embed size)


═══════════════════════════════════════════════════════════════
DELIVERABLE B — ICON SVG (favicon / app icon / GitHub avatar)
═══════════════════════════════════════════════════════════════

Format:   valid SVG 1.1 markup only — no embedded PNG, no filters, no masks
ViewBox:  0 0 512 512
Content:  ICON MARK ONLY — no wordmark text in the SVG
          (apple circuit + { } + eye — simplified for small sizes)

Technical requirements:
  • All shapes as <path> or <line> — no raster, no <image>
  • Strokes: vector paths (convert strokes to outlines if needed for favicon)
  • Max 2 fill colors + transparent background:
      fill/stroke primary: #0a6b55
      accent (one node):   #9a6b00
  • Include xmlns="http://www.w3.org/2000/svg" and viewBox="0 0 512 512"
  • File size target: under 8 KB
  • Must read clearly at 32 × 32 px (favicon) and 512 × 512 (app icon)

Simplify for small size:
  • Remove fine trace detail that disappears at 32px
  • Eye = circle + dot (minimum 8px at 512 scale)
  • Braces { } = bold, 3–4px stroke at 512 scale
  • Apple outline = max 12–16 trace segments (not 40)
  • Keep square safe zone: icon content in center 80% (40px margin at 512)

Provide TWO color groups in one SVG file:
  <g id="light"> … #0a6b55 paths … </g>
  <g id="dark" style="display:none"> … #5ce5be paths … </g>
  OR two separate SVG code blocks: icon-light.svg and icon-dark.svg

Output the complete SVG markup in a copy-pasteable code block.


═══════════════════════════════════════════════════════════════
TYPOGRAPHY (hero PNG wordmark only — NOT in SVG)
═══════════════════════════════════════════════════════════════

  ERIS — monospace (Menlo / Consolas / SF Mono style)
  Weight: semibold (600)
  All caps
  Color: #1a2238 or #0a6b55
  No kerning hacks, no decorative ligatures


═══════════════════════════════════════════════════════════════
OUTPUT ORDER
═══════════════════════════════════════════════════════════════

1. Show the hero PNG (2048×2048 or 2400×1200)
2. Paste complete SVG source for the icon
3. Brief note: confirm 32px favicon legibility (eye + braces + apple outline visible)
4. List hex used per layer

Do NOT output alternative concepts, dark hero PNGs, or wordmark SVGs unless asked.
```


---

## Follow-up (if first result is soft or off-palette)

```
The PNG is too soft / colors are wrong. Re-render with these fixes:

1. PNG: export at 2048×2048, solid #e8ecf4 background, zero blur, zero gradient
2. Icon color EXACTLY #0a6b55 — not turquoise, not forest green
3. ONE amber dot #9a6b00 on stem tip only
4. SVG: icon only, 512 viewBox, paths not strokes, under 8 KB
5. Remove all copper/rose-gold. Remove acronym subtitle.
6. At 32×32 preview: I must see { } and the apple outline. Simplify traces if not.
```

---

## If SVG quality is poor — trace prompt

```
Ignore raster. Write minimal SVG from scratch:

512×512, transparent background.
Apple = 14 orthogonal line segments forming a closed circuit-board silhouette.
Center: two path-based curly braces, fill #0a6b55.
Top center: eye = circle r=14 at (256,72) + circle r=5 at (256,72), fill #0a6b55.
Stem tip node: circle r=6 at (256,48), fill #9a6b00.
Stroke width equivalent 3px at 512 scale. No fonts. No filters.
Output ONLY the SVG XML.
```

---

## After Gemini — local checklist

| Check | Pass? |
|---|---|
| PNG sharp at 100% on `#e8ecf4` | |
| SVG validates (paste into browser) | |
| Favicon 32×32: apple + `{ }` readable | |
| No copper / purple / acronym in lockup | |
| Copy PNG → `assets/eris-hero.png`, SVG → `assets/icon.svg` in site repo | |

Reference palette source: `docs/TODO/HANDOVER-eris-system-dev-landing-page.md` §4.1
