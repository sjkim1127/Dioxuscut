# Dioxuscut 🦀

**Remotion** — 비디오를 React 컴포넌트로 만드는 JS 프레임워크 — 의 Rust 구현입니다.  
[Dioxus](https://dioxuslabs.com/) (Rust용 React-스타일 UI 프레임워크)를 사용해 선언적으로 비디오를 제작합니다.

---

## 구조

```
Dioxuscut/
├── Cargo.toml                    # Rust Workspace
│
├── crates/
│   ├── animation/                # interpolate(), spring(), easing  ← remotion/core
│   ├── core/                     # Composition, Sequence, AbsoluteFill, hooks
│   ├── media/                    # <Video>, <Audio>, <Img>
│   ├── player/                   # <Player> (재생 UI)
│   ├── renderer/                 # 프레임 렌더링 + FFmpeg 인코딩
│   └── transitions/              # <Fade>, <Slide>
│
├── apps/
│   ├── studio/                   # Dioxuscut Studio (데스크톱)
│   └── example/                  # Hello World 예제
│
└── vendor/remotion-4.0.495/      # 참고용 (gitignore)
```

---

## 빠른 시작

### 예제 (웹)

```bash
# Dioxus CLI 설치 (최초 1회)
cargo install dioxus-cli

# 웹 예제 실행
dx serve --package example
```

### 스튜디오 (데스크톱)

```bash
cargo run --package studio --features desktop
```

### 단위 테스트

```bash
cargo test -p dioxuscut-animation   # interpolate, spring 검증
cargo test --workspace              # 전체 테스트
```

---

## Remotion 대응표

| Remotion (JS)          | Dioxuscut (Rust)                           |
|------------------------|---------------------------------------------|
| `useCurrentFrame()`    | `use_current_frame()`                      |
| `useVideoConfig()`     | `use_video_config()`                       |
| `interpolate()`        | `dioxuscut_animation::interpolate()`       |
| `spring()`             | `dioxuscut_animation::spring()`            |
| `Easing.bezier()`      | `dioxuscut_animation::easing::bezier()`   |
| `<Composition>`        | `<Composition>` (core)                     |
| `<Sequence>`           | `<Sequence>` (core)                        |
| `<AbsoluteFill>`       | `<AbsoluteFill>` (core)                    |
| `<Freeze>`             | `<Freeze>` (core)                          |
| `<Video>`              | `<Video>` (media)                          |
| `<Audio>`              | `<Audio>` (media)                          |
| `<Img>`                | `<Img>` (media)                            |
| `@remotion/player`     | `dioxuscut-player`                         |
| `@remotion/transitions`| `dioxuscut-transitions`                    |
| `@remotion/renderer`   | `dioxuscut-renderer`                       |

---

## 예제 코드

```rust
use dioxus::prelude::*;
use dioxuscut_core::{AbsoluteFill, Composition, Sequence, hooks::use_current_frame};
use dioxuscut_animation::interpolate::{interpolate, ExtrapolateType, InterpolateOptions};
use dioxuscut_animation::spring::{spring, SpringConfig};
use dioxuscut_player::Player;

fn main() { dioxus::launch(App); }

#[component]
fn App() -> Element {
    rsx! {
        Player {
            width: 1920, height: 1080, fps: 30.0,
            duration_in_frames: 150, controls: true,
            MyVideo {}
        }
    }
}

#[component]
fn MyVideo() -> Element {
    rsx! {
        Sequence { from: 0, duration_in_frames: 60,
            Title {}
        }
    }
}

#[component]
fn Title() -> Element {
    let frame = use_current_frame();
    let opacity = interpolate(
        frame as f64, &[0.0, 30.0], &[0.0, 1.0],
        InterpolateOptions {
            extrapolate_right: ExtrapolateType::Clamp,
            ..Default::default()
        },
    );
    let scale = spring(frame, 30.0, SpringConfig::default());

    rsx! {
        AbsoluteFill {
            style: "background: #0d0d14; display: flex; align-items: center; justify-content: center;",
            div {
                style: "color: white; font-size: 72px; opacity: {opacity:.4}; transform: scale({scale:.4});",
                "Hello, Dioxuscut! 🦀"
            }
        }
    }
}
```

---

## 라이선스

MIT OR Apache-2.0