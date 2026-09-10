#!/usr/bin/env python3
"""
Comprehensive tests for Dioxuscut Python SDK bindings.
"""

import os
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent.parent.parent
TARGET_DIR = ROOT / "target/python_test_output"
TARGET_DIR.mkdir(parents=True, exist_ok=True)

import dioxuscut


def test_metadata():
    print("[*] Testing metadata and version...")
    version = dioxuscut.__version__
    assert version == "0.1.3", f"Unexpected version: {version}"
    compositions = dioxuscut.list_compositions()
    print(f"[+] Version: {version}")
    print(f"[+] Registered compositions: {compositions}")
    assert "HelloWorld" in compositions, "HelloWorld should be registered"
    print("[✓] Metadata test passed.")


def test_render_still():
    print("\n[*] Testing still frame render (PNG)...")
    out_png = TARGET_DIR / "test_still.png"
    if out_png.exists():
        out_png.unlink()

    dioxuscut.render_still(
        composition="HelloWorld",
        frame=15,
        output=out_png,
        width=640,
        height=360,
        fps=30.0,
    )

    assert out_png.exists(), "Output PNG was not created"
    file_size = out_png.stat().st_size
    print(f"[+] Rendered still PNG size: {file_size} bytes")
    assert file_size > 1000, "Still PNG file is suspiciously small"
    print("[✓] Still frame render test passed.")


def test_render_video():
    print("\n[*] Testing video render (MP4)...")
    out_mp4 = TARGET_DIR / "test_video.mp4"
    if out_mp4.exists():
        out_mp4.unlink()

    dioxuscut.render(
        composition="HelloWorld",
        output=out_mp4,
        props={"title": "Python AI Video", "subtitle": "Ultra Fast Native"},
        width=640,
        height=360,
        fps=30.0,
        duration=60, # 2 seconds
        codec="h264",
    )

    assert out_mp4.exists(), "Output MP4 was not created"
    file_size = out_mp4.stat().st_size
    print(f"[+] Rendered MP4 size: {file_size} bytes")
    assert file_size > 5000, "MP4 file is suspiciously small"
    print("[✓] Video render test passed.")


def test_render_script():
    print("\n[*] Testing dynamic Rhai script render...")
    rhai_script = ROOT / "examples/hello.rhai"
    if not rhai_script.exists():
        print("[-] Skipping script test (examples/hello.rhai not found)")
        return

    out_rhai_mp4 = TARGET_DIR / "test_rhai.mp4"
    if out_rhai_mp4.exists():
        out_rhai_mp4.unlink()

    dioxuscut.render_script(
        script_path=rhai_script,
        output=out_rhai_mp4,
        props={
            "title": "Dynamic Rhai Script",
            "subtitle": "Python AI Generation",
            "background": "#0b0d19",
            "accent": "#00f0ff88",
        },
        width=640,
        height=360,
        fps=30.0,
        duration=45,
    )

    assert out_rhai_mp4.exists(), "Output Rhai MP4 was not created"
    print(f"[+] Rendered Rhai MP4 size: {out_rhai_mp4.stat().st_size} bytes")
    print("[✓] Rhai script render test passed.")


def test_composition_class():
    print("\n[*] Testing Composition OOP class...")
    out_comp = TARGET_DIR / "test_oop.mp4"
    if out_comp.exists():
        out_comp.unlink()

    comp = dioxuscut.Composition("HelloWorld", width=640, height=360, duration=30)
    print(f"[+] Initialized {comp}")
    comp.render(out_comp, props={"title": "OOP API"})

    assert out_comp.exists(), "Output OOP MP4 was not created"
    print(f"[+] Rendered OOP MP4 size: {out_comp.stat().st_size} bytes")
    print("[✓] Composition OOP class test passed.")


def main():
    print("=" * 60)
    print("🚀 Running Dioxuscut Python SDK Test Suite")
    print("=" * 60)
    test_metadata()
    test_render_still()
    test_render_video()
    test_render_script()
    test_composition_class()
    print("\n" + "=" * 60)
    print("🎉 ALL PYTHON SDK TESTS PASSED PERFECTLY!")
    print("=" * 60)


if __name__ == "__main__":
    main()
