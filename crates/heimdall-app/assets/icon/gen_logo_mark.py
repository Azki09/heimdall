"""Turns logo_mini.svg into crates/heimdall-app/src/logo_mark.rs: the
outlines of the title bar mark as polygons, normalized to a unit square.
The app fills them (even-odd) at the exact pixel size it needs.

Run from the repository root after changing the SVG:
    python crates/heimdall-app/assets/icon/gen_logo_mark.py

Handles what the SVG uses: absolute M, L, C and Z commands, one path.
"""

import re
from pathlib import Path

HERE = Path(__file__).resolve().parent
SVG = HERE / "logo_mini.svg"
OUT = HERE.parent.parent / "src" / "logo_mark.rs"
STEPS = 10  # line segments per cubic curve


def subpaths(d):
    tokens = re.findall(r"[MLCZ]|-?\d*\.?\d+(?:e-?\d+)?", d)
    paths, cur, i, cmd = [], [], 0, None
    while i < len(tokens):
        t = tokens[i]
        if t in "MLCZ":
            cmd = t
            i += 1
            if cmd == "Z":
                if cur:
                    paths.append(cur)
                cur = []
            continue
        if cmd in ("M", "L"):
            x, y = float(tokens[i]), float(tokens[i + 1])
            i += 2
            if cmd == "M" and cur:
                paths.append(cur)
                cur = []
            cur.append((x, y))
            if cmd == "M":
                cmd = "L"
        elif cmd == "C":
            x1, y1, x2, y2, x, y = map(float, tokens[i:i + 6])
            i += 6
            x0, y0 = cur[-1]
            for s in range(1, STEPS + 1):
                t = s / STEPS
                u = 1 - t
                cur.append((
                    u**3 * x0 + 3 * u * u * t * x1 + 3 * u * t * t * x2 + t**3 * x,
                    u**3 * y0 + 3 * u * u * t * y1 + 3 * u * t * t * y2 + t**3 * y,
                ))
        else:
            raise SystemExit(f"unsupported path data at token {t!r}")
    if cur:
        paths.append(cur)
    return paths


def main():
    d = re.search(r'\sd="([^"]+)"', SVG.read_text(encoding="utf-8")).group(1)
    paths = subpaths(d)
    xs = [x for p in paths for x, _ in p]
    ys = [y for p in paths for _, y in p]
    x0, y0 = min(xs), min(ys)
    side = max(max(xs) - x0, max(ys) - y0)
    # Center the drawing in the unit square.
    ox = (side - (max(xs) - x0)) / 2
    oy = (side - (max(ys) - y0)) / 2
    lines = [
        "//! The title bar mark (DESIGN §15), generated from",
        "//! `assets/icon/logo_mini.svg` by `assets/icon/gen_logo_mark.py`. Don't edit by hand.",
        "//! Closed polygons in a unit square, filled with the even-odd rule.",
        "",
        "// Coordinates, not math constants.",
        "#![allow(clippy::approx_constant)]",
        "",
        "#[rustfmt::skip]",
        "pub const POLYGONS: &[&[[f32; 2]]] = &[",
    ]
    for p in paths:
        pts = ", ".join(f"[{(x - x0 + ox) / side:.4f}, {(y - y0 + oy) / side:.4f}]" for x, y in p)
        lines.append(f"    &[{pts}],")
    lines.append("];")
    OUT.write_text("\n".join(lines) + "\n", encoding="utf-8", newline="\n")
    print(f"{OUT}: {len(paths)} polygons, {sum(map(len, paths))} points")


if __name__ == "__main__":
    main()
