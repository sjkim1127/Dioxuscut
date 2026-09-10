import React from 'react';
import {AbsoluteFill, Composition, registerRoot, spring, useCurrentFrame} from 'remotion';

export const CyberpunkScene: React.FC = () => {
  const frame = useCurrentFrame();

  // 1. Grid lines animation
  const gridOffset = ((frame * 2) % 60) * 0.5;

  // 2. Cyan laser position
  const laserX = 200 + ((frame * 8) % 1520);

  // 3. Spring for title entrance
  const progress = spring({
    frame: frame % 90,
    fps: 30,
    config: {
      damping: 12,
      mass: 1,
      stiffness: 100,
      overshootClamping: false,
    },
    durationInFrames: 45,
  });

  const textX = 200 + (1 - progress) * -120;
  const chroma = frame % 15 === 0 ? 6 : 2.5;

  return (
    <AbsoluteFill style={{backgroundColor: '#0b0d19', overflow: 'hidden', fontFamily: 'sans-serif'}}>
      {/* Background animated grid lines */}
      <svg width="1920" height="1080" style={{position: 'absolute', top: 0, left: 0}}>
        {Array.from({length: 12}).map((_, i) => {
          const y = 100 + i * 80 + gridOffset;
          return (
            <line
              key={i}
              x1="100"
              y1={y}
              x2="1820"
              y2={y}
              stroke="rgba(30, 45, 80, 0.47)"
              strokeWidth="1"
            />
          );
        })}
        {/* Laser line */}
        <line x1="200" y1="320" x2="1720" y2="320" stroke="#00f0ff" strokeWidth="3" opacity="0.9" />
      </svg>

      {/* Laser dot */}
      <div
        style={{
          position: 'absolute',
          left: laserX,
          top: 317,
          width: 40,
          height: 9,
          backgroundColor: '#fff',
          border: '2px solid #00f0ff',
          borderRadius: 2,
        }}
      />

      {/* Auto-scaled title with drop-shadow & simulated chromatic aberration */}
      <div
        style={{
          position: 'absolute',
          left: textX,
          top: 380,
          width: 1400,
          height: 280,
          opacity: Math.max(0, Math.min(1, progress)),
          filter: 'brightness(1.2)',
        }}
      >
        {/* Red/Blue chroma shift layers */}
        <div
          style={{
            position: 'absolute',
            left: chroma,
            top: 1,
            color: 'rgba(255, 0, 80, 0.7)',
            fontSize: 68,
            fontWeight: 700,
            lineHeight: 1.2,
            whiteSpace: 'pre-line',
            mixBlendMode: 'screen',
          }}
        >
          {"CYBERPUNK 2088\nNEON PROTOCOL"}
        </div>
        <div
          style={{
            position: 'absolute',
            left: -chroma,
            top: -1,
            color: 'rgba(0, 240, 255, 0.7)',
            fontSize: 68,
            fontWeight: 700,
            lineHeight: 1.2,
            whiteSpace: 'pre-line',
            mixBlendMode: 'screen',
          }}
        >
          {"CYBERPUNK 2088\nNEON PROTOCOL"}
        </div>
        <div
          style={{
            position: 'relative',
            color: '#ffffff',
            fontSize: 68,
            fontWeight: 700,
            lineHeight: 1.2,
            whiteSpace: 'pre-line',
            textShadow: '0 0 14px rgba(255, 0, 128, 0.86)',
          }}
        >
          {"CYBERPUNK 2088\nNEON PROTOCOL"}
        </div>
      </div>

      {/* Cyberpunk badge */}
      <div
        style={{
          position: 'absolute',
          left: 200,
          top: 740,
          width: 360,
          height: 56,
          backgroundColor: 'rgba(255, 0, 128, 0.18)',
          border: '2px solid rgb(255, 0, 128)',
          borderRadius: 8,
          display: 'flex',
          alignItems: 'center',
          paddingLeft: 30,
        }}
      >
        <span style={{color: '#00f0ff', fontSize: 24, fontWeight: 600}}>
          STATUS // ONLINE
        </span>
      </div>

      {/* Radial Vignette Overlay */}
      <div
        style={{
          position: 'absolute',
          top: 0,
          left: 0,
          width: 1920,
          height: 1080,
          pointerEvents: 'none',
          background: 'radial-gradient(ellipse at center, rgba(0,0,0,0) 30%, rgba(0,0,0,0.65) 100%)',
        }}
      />
    </AbsoluteFill>
  );
};

registerRoot(() => (
  <Composition
    id="CyberpunkScene"
    component={CyberpunkScene}
    width={1920}
    height={1080}
    fps={30}
    durationInFrames={180}
  />
));
