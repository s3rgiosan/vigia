#!/usr/bin/env python3
"""Render the menu bar icons at 2x into src-tauri/icons/tray.

The menu bar shows the app's mark, a porthole: a thick rim with four rivets and the sea filling
the lower part of the glass. The porthole is always a template image that macOS tints for the
menu bar; status color lives in a separate badge the app overlays on the lower right corner:

- `porthole.png`: the plain porthole, shown when idle;
- `paused.png`: the porthole dimmed, shown while paused;
- `porthole-badged.png`: the porthole with a transparent notch where the badge sits, so the two
  never touch;
- `badge-{light,dark}-{status}.png`: a colored circle with a glyph (failed ×, error !,
  running …, passing ✓), in the system colors for a light or a dark menu bar.

Run from the repo root after changing any value. `BADGE_CENTER` and `BADGE_SIZE` must match
`BADGE_CENTER` and `BADGE_SIZE` in src-tauri/src/tray.rs.
"""

from pathlib import Path

from PIL import Image, ImageDraw

SIZE = 18  # points; the menu bar is 22 pt tall and icons sit at about 18 pt
SCALE = 8  # render large, then downsample for smooth edges

TEMPLATE = (0, 0, 0, 255)
WHITE = (255, 255, 255, 255)
BLACK = (0, 0, 0, 255)

# System colors for each menu bar appearance, and the glyph color that reads on each.
STATUS = {
    "failed": {"light": (255, 59, 48), "dark": (255, 69, 58), "glyph": WHITE},
    "error": {"light": (255, 149, 0), "dark": (255, 159, 10), "glyph": WHITE},
    "running": {"light": (255, 204, 0), "dark": (255, 214, 10), "glyph": BLACK},
    "passing": {"light": (52, 199, 89), "dark": (48, 209, 88), "glyph": WHITE},
}

# Geometry on an 18 pt canvas, scaled up below.
CENTER = 8.6
RIM_OUTER = 7.8
RIM_WIDTH = 2.5  # a thick band reads as a porthole rim at menu bar size
RIVET_R = 0.62  # rivets are holes punched into the rim
HORIZON = 1.2  # horizon line height below the centre
BADGE_CENTER = (14.5, 14.5)  # from the top left of the porthole canvas
BADGE_SIZE = 7  # points; the badge image is a square this size
BADGE_GAP = 0.9  # transparent ring between the badge and the porthole
GLYPH_STROKE = 1.05


def porthole(draw: ImageDraw.ImageDraw, k: float, color) -> None:
    c = CENTER * k
    outer = RIM_OUTER * k
    inner = (RIM_OUTER - RIM_WIDTH) * k
    draw.ellipse([c - outer, c - outer, c + outer, c + outer], fill=color)
    draw.ellipse([c - inner, c - inner, c + inner, c + inner], fill=(0, 0, 0, 0))
    # Four rivets on the diagonals, centred in the rim band.
    mid = (RIM_OUTER - RIM_WIDTH / 2) * 0.7071 * k
    r = RIVET_R * k
    for dx, dy in ((-1, -1), (1, -1), (-1, 1), (1, 1)):
        x, y = c + dx * mid, c + dy * mid
        draw.ellipse([x - r, y - r, x + r, y + r], fill=(0, 0, 0, 0))
    # The sea: the lower part of the glass, below the horizon.
    sea = Image.new("L", draw.im.size, 0)
    sdraw = ImageDraw.Draw(sea)
    gap = 0.9 * k
    sdraw.ellipse([c - inner + gap, c - inner + gap, c + inner - gap, c + inner - gap], fill=255)
    sdraw.rectangle([0, 0, draw.im.size[0], c + HORIZON * k], fill=0)
    draw.bitmap((0, 0), sea, fill=color)


def downsample(img: Image.Image, size: int) -> Image.Image:
    return img.resize((size, size), Image.LANCZOS)


def render_porthole(scale: int, notch: bool = False, dim: bool = False) -> Image.Image:
    k = scale * SCALE
    img = Image.new("RGBA", (SIZE * k, SIZE * k), (0, 0, 0, 0))
    draw = ImageDraw.Draw(img)
    porthole(draw, k, TEMPLATE)
    if notch:
        bx, by = BADGE_CENTER[0] * k, BADGE_CENTER[1] * k
        r = (BADGE_SIZE / 2 + BADGE_GAP) * k
        draw.ellipse([bx - r, by - r, bx + r, by + r], fill=(0, 0, 0, 0))
    img = downsample(img, SIZE * scale)
    if dim:
        alpha = img.getchannel("A").point(lambda a: int(a * 0.45))
        img.putalpha(alpha)
    return img


def glyph(draw: ImageDraw.ImageDraw, status: str, k: float, color) -> None:
    """Draws the status glyph centred on a badge of `BADGE_SIZE` points scaled by `k`."""
    c = BADGE_SIZE / 2 * k
    w = round(GLYPH_STROKE * k)

    def p(x: float, y: float) -> tuple[float, float]:
        return (c + x * k, c + y * k)

    def dot(x: float, y: float, r: float) -> None:
        cx, cy = p(x, y)
        draw.ellipse([cx - r * k, cy - r * k, cx + r * k, cy + r * k], fill=color)

    def stroke(points) -> None:
        draw.line([p(x, y) for x, y in points], fill=color, width=w, joint="curve")
        for x, y in (points[0], points[-1]):
            dot(x, y, GLYPH_STROKE / 2)

    match status:
        case "failed":
            stroke([(-1.35, -1.35), (1.35, 1.35)])
            stroke([(-1.35, 1.35), (1.35, -1.35)])
        case "error":
            stroke([(0, -1.8), (0, 0.45)])
            dot(0, 1.65, 0.62)
        case "running":
            for x in (-1.75, 0, 1.75):
                dot(x, 0, 0.6)
        case "passing":
            stroke([(-1.6, 0.05), (-0.45, 1.2), (1.65, -1.25)])
        case _:
            raise ValueError(status)


def render_badge(status: str, appearance: str, scale: int) -> Image.Image:
    k = scale * SCALE
    s = BADGE_SIZE * k
    img = Image.new("RGBA", (s, s), (0, 0, 0, 0))
    draw = ImageDraw.Draw(img)
    colors = STATUS[status]
    draw.ellipse([0, 0, s - 1, s - 1], fill=colors[appearance] + (255,))
    glyph(draw, status, k, colors["glyph"])
    return downsample(img, BADGE_SIZE * scale)


def main() -> None:
    out = Path(__file__).resolve().parent.parent / "src-tauri" / "icons" / "tray"
    out.mkdir(parents=True, exist_ok=True)
    for old in out.glob("*.png"):
        old.unlink()
    count = 0
    for scale, suffix in ((2, "@2x"),):
        render_porthole(scale).save(out / f"porthole{suffix}.png")
        render_porthole(scale, dim=True).save(out / f"paused{suffix}.png")
        render_porthole(scale, notch=True).save(out / f"porthole-badged{suffix}.png")
        count += 3
        for status in STATUS:
            for appearance in ("light", "dark"):
                render_badge(status, appearance, scale).save(
                    out / f"badge-{appearance}-{status}{suffix}.png"
                )
                count += 1
    print(f"wrote {count} icons to {out}")


if __name__ == "__main__":
    main()
