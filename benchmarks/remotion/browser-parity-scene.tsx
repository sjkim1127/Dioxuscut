import React from 'react';
import {AbsoluteFill, Composition, registerRoot, spring, useCurrentFrame} from 'remotion';

const Scene = () => {
  const frame = useCurrentFrame();
  const scaleX = 320 / 1280;
  const scaleY = 180 / 720;
  return <AbsoluteFill style={{backgroundColor: 'rgb(15,23,42)'}}>
    {Array.from({length: 32}, (_, index) => {
      const progress = spring({frame: frame % 60, fps: 30, durationInFrames: 24, delay: (index % 8) * 2});
      const left = Math.round((60 + (index % 8) * 145 + progress * 40) * scaleX);
      const top = (80 + Math.floor(index / 8) * 140) * scaleY;
      return <div key={index} style={{position: 'absolute', left, top, width: 64 * scaleX, height: 64 * scaleY,
        backgroundColor: `rgb(${80 + index * 4},160,220)`}} />;
    })}
  </AbsoluteFill>;
};

registerRoot(() => <Composition id="SpringRectsBrowserParity" component={Scene}
  width={320} height={180} fps={30} durationInFrames={60} />);
