#!/usr/bin/env python3
"""Create an isolated, fictional data directory for screenshots and UI review.

Usage: demo_data.py DIR [--states]

--states also writes a running job, a finished sweep and a render log.

Writes DIR/mseries-image-studio/{images,recipes,models,engine} with generated
landscape images, matching recipe sidecars, empty placeholder model files and a
placeholder engine binary. Run the app with XDG_DATA_HOME=DIR so the real
library, models and any running job are never touched. Nothing here is a real
model or a real render.
"""
import json
import math
import random
import sys
import time
from pathlib import Path

from PIL import Image, ImageDraw, ImageFilter

PALETTES = [
    ("Misty alpine lake at dawn, soft pink light on the peaks, mirror reflection, cinematic landscape photography",
     (255, 190, 170), (120, 140, 190), (40, 52, 84)),
    ("Desert canyon at golden hour, long shadows, warm sandstone, clear sky, wide-angle photography",
     (255, 214, 140), (232, 140, 90), (96, 44, 52)),
    ("Northern lights over a frozen fjord, deep blue night, green aurora ribbons, long exposure",
     (90, 230, 190), (30, 70, 120), (8, 14, 36)),
    ("Quiet harbor village at blue hour, glowing windows, calm water, fishing boats, soft haze",
     (255, 170, 120), (84, 96, 150), (22, 26, 52)),
    ("Rolling lavender fields under a stormy sky, shafts of light, rich purple tones, painterly",
     (220, 200, 255), (130, 100, 180), (44, 30, 80)),
    ("Bamboo forest path after rain, drifting mist, fresh green light, serene, shallow depth of field",
     (210, 255, 200), (90, 160, 110), (20, 52, 44)),
    ("Minimal paper-cut mountains in layered teal and coral, flat illustration, poster composition",
     (255, 150, 140), (60, 160, 160), (20, 70, 90)),
    ("Sunlit studio still life, ceramic vase with dried flowers, soft window light, muted earth tones",
     (250, 232, 210), (210, 170, 140), (110, 80, 70)),
]


def mix(a, b, t):
    return tuple(int(a[i] + (b[i] - a[i]) * t) for i in range(3))


def paint(size, palette, seed):
    width, height = size
    sky, mid, deep = palette
    rng = random.Random(seed)
    image = Image.new("RGB", size)
    draw = ImageDraw.Draw(image)
    for y in range(height):
        t = y / height
        draw.line([(0, y), (width, y)], fill=mix(sky, mid, min(1.0, t * 1.4)))
    sun_x = rng.randint(width // 4, width * 3 // 4)
    sun_y = int(height * rng.uniform(0.25, 0.45))
    glow = Image.new("RGB", size, (0, 0, 0))
    glow_draw = ImageDraw.Draw(glow)
    radius = width // 6
    glow_draw.ellipse([sun_x - radius, sun_y - radius, sun_x + radius, sun_y + radius], fill=mix(sky, (255, 255, 255), 0.7))
    glow = glow.filter(ImageFilter.GaussianBlur(radius // 2))
    image = Image.blend(image, Image.composite(glow, image, glow.convert("L")), 0.55)
    draw = ImageDraw.Draw(image)
    for layer in range(4):
        t = layer / 3
        base = int(height * (0.52 + 0.13 * layer))
        color = mix(mix(mid, deep, 0.25), deep, t)
        points = [(0, height)]
        phase = rng.uniform(0, math.tau)
        for x in range(0, width + 8, 8):
            ridge = math.sin(x / (width / (2.2 + layer)) + phase) * (height * 0.05) \
                + math.sin(x / (width / (6 + layer * 2)) + phase * 2) * (height * 0.02)
            points.append((x, base + ridge))
        points.append((width, height))
        draw.polygon(points, fill=color)
    return image.filter(ImageFilter.GaussianBlur(0.6))


def main(root: Path) -> None:
    base = root / "mseries-image-studio"
    images = base / "images"
    for name in ("images", "recipes", "models", "engine", "logs"):
        (base / name).mkdir(parents=True, exist_ok=True)
    models = base / "models"
    for name in (
        "Realistic_Vision_V6.0_NV_B1_fp16.safetensors",
        "stable-diffusion-v1-5-pruned-emaonly-Q4_0.gguf",
        "vae-ft-mse-840000-ema-pruned.safetensors",
    ):
        (models / name).touch()
    (base / "engine" / "sd-cli").touch()
    sizes = [(512, 768), (512, 512), (768, 512), (512, 768), (512, 512), (512, 768), (768, 512), (512, 512)]
    now = int(time.time())
    for index, ((prompt, *palette), size) in enumerate(zip(PALETTES, sizes)):
        stem = f"local-{9000 - index:04d}"
        path = images / f"{stem}.png"
        paint(size, tuple(palette), index).save(path)
        recipe = {
            "schema_version": 1,
            "created_at_unix": now - index * 5400 - 120,
            "prompt": prompt,
            "negative_prompt": "text, watermark, logo, blurry, low quality, deformed",
            "intent": ["Illustration", "Graphic or poster", "Product photo"][index % 3] if index > 5 else "Illustration",
            "workflow": "Freeform",
            "model_id": str(models / "Realistic_Vision_V6.0_NV_B1_fp16.safetensors"),
            "width": size[0],
            "height": size[1],
            "seed": 424242 + index,
            "tags": ["landscape", "demo"],
            "engine_id": "stable-diffusion.cpp local Vulkan build",
            "image_path": str(path),
            "elapsed_seconds": 180.0 + index * 11.5,
            "peak_memory_mib": 900 + index * 8,
            "steps": 30,
            "cfg_scale": 5.0,
            "sampler": "dpm++2m",
            "scheduler": "karras",
            "preview_count": 30,
            "time_to_first_preview_seconds": 6.4,
            "settings_note": "Demo fixture. Not a real render.",
        }
        (images / f"{stem}.json").write_text(json.dumps(recipe, indent=2))
    (base / "recipes" / "recipe-demo-1.json").write_text(json.dumps({
        "schema_version": 1, "created_at_unix": now - 86400, "workflow": "Prompt type study",
        "intent": "Illustration", "prompt": PALETTES[6][0], "width": 512, "height": 512,
    }, indent=2))
    (base / "recipes" / "recipe-demo-2.json").write_text(json.dumps({
        "schema_version": 1, "created_at_unix": now - 3 * 86400, "workflow": "Model readiness",
        "intent": "Product photo", "prompt": PALETTES[7][0], "width": 512, "height": 768,
    }, indent=2))


def states(root: Path) -> None:
    """Write a running job, a finished sweep and a render log for state review."""
    base = root / "mseries-image-studio"
    images = sorted((base / "images").glob("*.png"))
    (base / "current-job.json").write_text(json.dumps({
        "status": "Generating preview",
        "running": True,
        "step": 12,
        "steps": 30,
        "engine": "stable-diffusion.cpp local Vulkan build",
        "prompt": "Misty alpine lake at dawn",
        "image_path": str(images[0]),
        "model": str(base / "models" / "Realistic_Vision_V6.0_NV_B1_fp16.safetensors"),
        "sampler": "dpm++2m",
        "scheduler": "karras",
        "elapsed_seconds": 74.2,
        "preview_count": 12,
    }))
    sweeps = base / "sweeps"
    sweeps.mkdir(exist_ok=True)
    runs = []
    for index, label in enumerate(["Soft window light", "Golden hour", "Overcast softbox"]):
        runs.append({
            "label": label,
            "status": "Complete" if index < 2 else "Failed",
            "image_path": str(images[index + 1]),
            "elapsed_seconds": 181.4 + index * 3.2,
            "peak_memory_mib": 912,
            "overall_score": 3.6 - index * 0.4 if index < 2 else None,
            "error": "Not enough available memory for this size." if index == 2 else None,
        })
    manifest = sweeps / "demo.json"
    manifest.write_text(json.dumps({"status": "Complete · 2 of 3 images", "runs": runs}))
    (base / "latest-sweep.txt").write_text(str(manifest))
    log = base / "logs" / "local-9001.jsonl"
    events = [
        {"event": "assets", "elapsed_seconds": 0.0, "model": {"path": str(base / "models" / "Realistic_Vision_V6.0_NV_B1_fp16.safetensors")}},
        {"event": "first_preview", "elapsed_seconds": 6.4, "preview_count": 1},
        {"event": "progress", "elapsed_seconds": 90.2, "preview_count": 15, "VmRSS_bytes": 943718400},
        {"event": "complete", "elapsed_seconds": 181.4, "status": "Complete", "VmHWM_bytes": 954204160},
    ]
    log.write_text("\n".join(json.dumps(event) for event in events))


if __name__ == "__main__":
    if len(sys.argv) not in (2, 3):
        sys.exit(__doc__)
    main(Path(sys.argv[1]))
    if len(sys.argv) == 3 and sys.argv[2] == "--states":
        states(Path(sys.argv[1]))
