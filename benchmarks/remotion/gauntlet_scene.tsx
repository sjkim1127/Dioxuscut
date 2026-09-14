import React from 'react';
import {AbsoluteFill, Composition, registerRoot, useCurrentFrame, OffthreadVideo} from 'remotion';

// Import assets
// @ts-ignore
import testVideo from '../../target/assets/test_video.mp4';

// 20 test images
const imageAssets = Array.from({length: 20}, (_, i) => {
  const num = i.toString().padStart(2, '0');
  // @ts-ignore
  return require(`../../target/assets/img_${num}.png`);
});

export const GauntletScene: React.FC = () => {
  const frame = useCurrentFrame();
  const t = frame * 0.05;
  const gridOffset = ((frame * 2) % 60) * 0.4;

  return (
    <AbsoluteFill style={{backgroundColor: '#0a0c16', overflow: 'hidden', fontFamily: 'sans-serif'}}>
      {/* 0. Background Grid lines */}
      <svg width="1920" height="1080" style={{position: 'absolute', top: 0, left: 0}}>
        {Array.from({length: 15}).map((_, i) => {
          const y = 60 + i * 70 + gridOffset;
          return (
            <line
              key={i}
              x1="0"
              y1={y}
              x2="1920"
              y2={y}
              stroke="rgba(25, 35, 65, 0.31)"
              strokeWidth="1"
            />
          );
        })}
      </svg>

      {/* 1. Dual Video PiP Streams */}
      {/* Left Video PiP */}
      <div
        style={{
          position: 'absolute',
          left: 56,
          top: 76,
          width: 608,
          height: 345.5,
          border: '3px solid #00f0ff',
          borderRadius: 8,
          overflow: 'hidden',
          boxSizing: 'border-box',
        }}
      >
        <OffthreadVideo
          src={testVideo}
          loop
          style={{width: '100%', height: '100%', objectFit: 'cover'}}
        />
      </div>

      {/* Right Video PiP */}
      <div
        style={{
          position: 'absolute',
          left: 1256,
          top: 76,
          width: 608,
          height: 345.5,
          border: '3px solid #ff0080',
          borderRadius: 8,
          overflow: 'hidden',
          boxSizing: 'border-box',
          opacity: 0.9,
        }}
      >
        <OffthreadVideo
          src={testVideo}
          loop
          style={{width: '100%', height: '100%', objectFit: 'cover'}}
          playbackRate={1.5}
        />
      </div>

      {/* 2. 20 High-Res Image Tiles with Floating Motion */}
      {imageAssets.map((src, i) => {
        const row = Math.floor(i / 10);
        const col = i % 10;
        const baseX = 80 + col * 176;
        const baseY = row === 0 ? 460 : 610;
        const floatY = baseY + Math.sin(t + i * 0.5) * 12;

        return (
          <img
            key={i}
            src={src}
            alt=""
            style={{
              position: 'absolute',
              left: baseX,
              top: floatY,
              width: 140,
              height: 140,
              objectFit: 'contain',
              opacity: 0.95,
            }}
          />
        );
      })}

      {/* 3. 60 Floating Light Particle Orbs */}
      <svg width="1920" height="1080" style={{position: 'absolute', top: 0, left: 0, pointerEvents: 'none'}}>
        {Array.from({length: 60}).map((_, i) => {
          const angle = t * 0.8 + i * 0.35;
          const px = 960 + Math.cos(angle) * (300 + Math.sin(i * 8.0) * 180);
          const py = 540 + Math.sin(angle) * (220 + Math.cos(i * 6.0) * 140);
          const radius = 3.0 + (i % 5) * 1.5;
          const fill = i % 2 === 0 ? 'rgba(0, 240, 255, 0.55)' : 'rgba(255, 0, 128, 0.55)';

          return <circle key={i} cx={px} cy={py} r={radius} fill={fill} />;
        })}
      </svg>

      {/* 4. Heavy Blur Layer (Sigma 30 Blur Panel) */}
      <div
        style={{
          position: 'absolute',
          left: 100,
          top: 800,
          width: 1720,
          height: 220,
          backgroundColor: 'rgba(30, 20, 50, 0.86)',
          border: '2px solid #ffffff',
          borderRadius: 16,
          filter: 'blur(30px)',
          opacity: 0.85,
        }}
      />

      {/* 5. Multilingual Foreground Text (Over the blur panel) */}
      <div
        style={{
          position: 'absolute',
          left: 160,
          top: 835,
          color: '#ffffff',
          fontSize: 40,
          fontWeight: 700,
          letterSpacing: 1,
        }}
      >
        THE GAUNTLET // 극한 부하 스트레스 테스트
      </div>

      <div
        style={{
          position: 'absolute',
          left: 160,
          top: 915,
          color: '#00f0ff',
          fontSize: 24,
          fontWeight: 600,
        }}
      >
        900 Frames (30s) • 2x Video Decoders • 20 Images • Sigma 30.0 Blur • 60 Particles
      </div>
    </AbsoluteFill>
  );
};

registerRoot(() => (
  <Composition
    id="GauntletScene"
    component={GauntletScene}
    width={1920}
    height={1080}
    fps={30}
    durationInFrames={900}
  />
));
