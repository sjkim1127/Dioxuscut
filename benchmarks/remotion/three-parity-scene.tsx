import React, {useEffect, useRef} from 'react';
import {AbsoluteFill, Composition, registerRoot, useCurrentFrame, useVideoConfig} from 'remotion';
import * as THREE from 'three';

const Scene = () => {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const frame = useCurrentFrame();
  const {fps, width, height} = useVideoConfig();

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return undefined;
    const renderer = new THREE.WebGLRenderer({canvas, antialias: true, alpha: false});
    renderer.setPixelRatio(1);
    renderer.setSize(width, height, false);
    renderer.setClearColor(0x0b1020);
    const scene = new THREE.Scene();
    const camera = new THREE.PerspectiveCamera(45, width / height, 0.1, 100);
    camera.position.z = 3;
    scene.add(new THREE.HemisphereLight(0x9bbcff, 0x182033, 2));
    const mesh = new THREE.Mesh(
      new THREE.TorusKnotGeometry(0.62, 0.2, 64, 16),
      new THREE.MeshStandardMaterial({color: 0x49dcb1, roughness: 0.25, metalness: 0.4}),
    );
    scene.add(mesh);
    mesh.rotation.x = (frame / Math.max(fps, 1)) * 0.7;
    mesh.rotation.y = (frame / Math.max(fps, 1)) * 1.1;
    renderer.render(scene, camera);
    return () => {
      mesh.geometry.dispose();
      (mesh.material as THREE.Material).dispose();
      renderer.dispose();
    };
  }, [frame, fps, height, width]);

  return <AbsoluteFill><canvas ref={canvasRef} width={width} height={height} /></AbsoluteFill>;
};

registerRoot(() => <Composition id="ThreeLifecycleParity" component={Scene}
  width={640} height={360} fps={30} durationInFrames={60} />);
