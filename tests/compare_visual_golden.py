#!/usr/bin/env python3
"""
Compares rendered test snapshots against committed golden reference frames.
Exits with code 0 if all frames are within tolerance (< 0.5% MAE), or code 1 on regression.
"""

import argparse
import sys
from pathlib import Path

try:
    from PIL import Image
    import numpy as np
except ImportError:
    print("::error::PIL (Pillow) and numpy are required to run visual golden comparison.")
    print("Run: pip install pillow numpy")
    sys.exit(1)


def compare_images(actual_path: Path, golden_path: Path, max_tolerance: float = 0.005) -> tuple[bool, float, str]:
    if not actual_path.is_file():
        return False, 1.0, f"Actual snapshot missing: {actual_path}"
    if not golden_path.is_file():
        return False, 1.0, f"Golden reference missing: {golden_path}"

    with Image.open(actual_path) as img_a, Image.open(golden_path) as img_g:
        if img_a.size != img_g.size:
            return False, 1.0, f"Dimension mismatch: actual {img_a.size} vs golden {img_g.size}"

        arr_a = np.asarray(img_a.convert("RGBA"), dtype=np.float32)
        arr_g = np.asarray(img_g.convert("RGBA"), dtype=np.float32)

        abs_diff = np.abs(arr_a - arr_g)
        mae = float(np.mean(abs_diff) / 255.0)

        passed = mae <= max_tolerance
        details = f"MAE: {mae * 100.0:.4f}% (tolerance: {max_tolerance * 100.0:.2f}%)"
        return passed, mae, details


def main():
    parser = argparse.ArgumentParser(description="Compare visual snapshots against golden references")
    parser.add_argument("--actual-dir", type=Path, default=Path("snapshots"), help="Directory with actual rendered PNGs")
    parser.add_argument("--golden-dir", type=Path, default=Path("tests/visual_golden"), help="Directory with golden reference PNGs")
    parser.add_argument("--tolerance", type=float, default=0.005, help="Maximum allowed Mean Absolute Error (default 0.005 = 0.5%)")
    args = parser.parse_args()

    golden_files = sorted(args.golden_dir.glob("*.png"))
    if not golden_files:
        print(f"::error::No golden images found in {args.golden_dir}")
        sys.exit(1)

    print(f"🎬 Comparing {len(golden_files)} snapshots against golden references...")
    all_passed = True
    results = []

    for golden_path in golden_files:
        filename = golden_path.name
        actual_path = args.actual_dir / filename
        passed, mae, details = compare_images(actual_path, golden_path, args.tolerance)

        status_icon = "✅" if passed else "❌"
        results.append((status_icon, filename, details))
        if not passed:
            all_passed = False

    print("\n| Status | Frame Snapshot | Metrics / Difference |")
    print("|:------:|:---------------|:---------------------|")
    for status, name, details in results:
        print(f"| {status} | `{name}` | {details} |")

    if all_passed:
        print("\n🎉 All visual regression tests passed within tolerance!")
        sys.exit(0)
    else:
        print("\n❌ Visual regression detected! One or more frames exceeded the difference tolerance.")
        sys.exit(1)


if __name__ == "__main__":
    main()
