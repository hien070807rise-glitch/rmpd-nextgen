#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
render_app_logo.py
Render high-DPI anti-aliased (4x supersampled) PNG previews and multi-resolution
Windows .ico for RMPD-NextGen (Verdant Engawa Edition) from the exact SVG geometry.
"""
from pathlib import Path
from PIL import Image, ImageDraw

OUT_DIR = Path(r"C:\Users\Le Ke Hien\.gemini\antigravity\scratch\rmpd-nextgen\assets\logo")
SCALE = 4
SIZE = 512 * SCALE


def s(v: float) -> float:
    return v * SCALE


def render_variant(dark: bool) -> Image.Image:
    if dark:
        bg = (31, 30, 28, 255)         # Basalt Dark #1F1E1C
        frame = (52, 50, 46, 255)      # #34322E
        arcs = [
            ((72, 69, 63, 255), 3),
            ((110, 105, 95, 255), 3),
            ((155, 149, 138, 255), 3),
            ((240, 236, 228, 224), 4),
        ]
        diag_main = (142, 182, 150, 255)  # Soft Sage Moss #8EB696
        diag_hair = (130, 112, 90, 255)   # Warm Bronze #82705A
        bar_dim = (72, 69, 63, 255)
        bar_hi = (142, 182, 150, 255)
        spindle_fill = (62, 92, 68, 255)  # Deep Moss #3E5C44
        spindle_ring = (31, 30, 28, 255)
        core_ring = (243, 240, 234, 255)
        core_dot = (31, 30, 28, 255)
    else:
        bg = (243, 240, 234, 255)      # Washi Light #F3F0EA
        frame = (200, 194, 182, 255)   # #C8C2B6
        arcs = [
            ((200, 194, 182, 255), 3),
            ((166, 159, 146, 255), 3),
            ((110, 105, 95, 255), 3),
            ((29, 28, 26, 224), 4),
        ]
        diag_main = (62, 92, 68, 255)     # Deep Moss #3E5C44
        diag_hair = (130, 112, 90, 255)
        bar_dim = (29, 28, 26, 255)
        bar_hi = (62, 92, 68, 255)
        spindle_fill = (29, 28, 26, 255)
        spindle_ring = (243, 240, 234, 255)
        core_ring = (243, 240, 234, 255)
        core_dot = (62, 92, 68, 255)

    img = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
    draw = ImageDraw.Draw(img)

    # Squircle tile + crisp inner architectural border
    draw.rounded_rectangle([s(16), s(16), s(496), s(496)], radius=s(104), fill=bg)
    draw.rounded_rectangle([s(32), s(32), s(480), s(480)], radius=s(90), outline=frame, width=int(s(2)))

    # Concentric 270-degree acoustic arcs (open in upper-right from 270..360 deg in Pillow coords)
    radii = [124, 148, 172, 196]
    for rad, (col, w) in zip(radii, arcs):
        bbox = [s(256 - rad), s(256 - rad), s(256 + rad), s(256 + rad)]
        draw.arc(bbox, start=0, end=270, fill=col, width=int(s(w)))

    # 3 Harmonic vertical bars in open upper-right quadrant (clean Ma gap above diagonal)
    draw.rectangle([s(276), s(110), s(288), s(152)], fill=bar_dim)
    draw.rectangle([s(298), s(88), s(310), s(152)], fill=bar_hi)
    draw.rectangle([s(320), s(122), s(332), s(152)], fill=bar_dim)

    # Diagonal tonearm bar + parallel bronze hairline (like pl_03)
    draw.line([s(120), s(392), s(392), s(120)], fill=diag_main, width=int(s(18)))
    draw.line([s(130), s(402), s(402), s(130)], fill=diag_hair, width=int(s(3)))

    # Center spindle & hollow ring
    r_sp = s(52)
    draw.ellipse([s(256) - r_sp, s(256) - r_sp, s(256) + r_sp, s(256) + r_sp], fill=spindle_fill, outline=spindle_ring, width=int(s(6)))
    r_in = s(16)
    draw.ellipse([s(256) - r_in, s(256) - r_in, s(256) + r_in, s(256) + r_in], fill=core_ring)
    r_dot = s(6)
    draw.ellipse([s(256) - r_dot, s(256) - r_dot, s(256) + r_dot, s(256) + r_dot], fill=core_dot)

    return img.resize((512, 512), Image.Resampling.LANCZOS)


def main():
    OUT_DIR.mkdir(parents=True, exist_ok=True)
    dark_img = render_variant(dark=True)
    washi_img = render_variant(dark=False)

    dark_512 = OUT_DIR / "rmpd-nextgen-512.png"
    washi_512 = OUT_DIR / "rmpd-nextgen-washi-512.png"
    ico_path = OUT_DIR / "rmpd-nextgen.ico"

    dark_img.save(dark_512, format="PNG")
    washi_img.save(washi_512, format="PNG")
    dark_img.save(ico_path, format="ICO", sizes=[(256, 256), (128, 128), (64, 64), (48, 48), (32, 32), (16, 16)])

    print(f"Generated:\n- {dark_512}\n- {washi_512}\n- {ico_path}")


if __name__ == "__main__":
    main()
