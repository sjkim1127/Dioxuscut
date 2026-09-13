import React from 'react';
import {AbsoluteFill, Composition, registerRoot, spring, useCurrentFrame} from 'remotion';

const Scene = () => {
  const frame = useCurrentFrame();
  return <AbsoluteFill style={{backgroundColor: 'rgb(15,23,42)'}}>
    {Array.from({length: 32}, (_, index) => {
      const progress = spring({frame: frame % 60, fps: 30, durationInFrames: 24, delay: (index % 8) * 2});
      const left = 15 + (index % 8) * 36.25 + progress * 10;
      const top = 20 + Math.floor(index / 8) * 35;
      return <div key={index} style={{position: 'absolute', left, top, width: 16, height: 16,
        backgroundColor: `rgb(${80 + index * 4},160,220)`}} />;
    })}
  </AbsoluteFill>;
};

registerRoot(() => <Composition id="SpringRectsBrowserParity" component={Scene}
  width={320} height={180} fps={30} durationInFrames={60} />);
