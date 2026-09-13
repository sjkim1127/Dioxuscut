// Framework-free reference composition for registerThreeComposition().
export function register(api) {
  // Exercise the same readiness contract used by media-heavy compositions:
  // setup may briefly block capture while an async resource is prepared.
  const buffer = api.useBufferState?.();
  const unblock = buffer?.delayPlayback?.().unblock;
  if (unblock) setTimeout(unblock, 0);

  api.registerThreeComposition('three_lifecycle_preview', {
    setup({ THREE }) {
      const scene = new THREE.Scene();
      const camera = new THREE.PerspectiveCamera(45, 16 / 9, 0.1, 100);
      camera.position.z = 3;
      scene.add(new THREE.HemisphereLight(0x9bbcff, 0x182033, 2));
      const mesh = new THREE.Mesh(
        new THREE.TorusKnotGeometry(0.62, 0.2, 64, 16),
        new THREE.MeshStandardMaterial({ color: 0x49dcb1, roughness: 0.25, metalness: 0.4 }),
      );
      scene.add(mesh);
      return { scene, camera, mesh };
    },
    render({ renderer, scene, camera, mesh, frame, fps, width, height, props }) {
      const seconds = frame / Math.max(fps, 1);
      mesh.rotation.x = seconds * 0.7;
      mesh.rotation.y = seconds * 1.1;
      if (typeof props.color === 'string') mesh.material.color.set(props.color);
      if (Number.isFinite(width) && Number.isFinite(height)) {
        renderer.setSize(width, height, false);
        camera.aspect = width / Math.max(height, 1);
        camera.updateProjectionMatrix();
      }
      renderer.render(scene, camera);
    },
    dispose(instance) {
      instance?.mesh?.geometry?.dispose?.();
      instance?.mesh?.material?.dispose?.();
    },
  });

  // Deterministic audio-reactive reference: the same PCM shape exercises the
  // browser media-utils contract without requiring a network asset in smoke
  // tests or in AI-generated starter compositions.
  const sampleRate = 48_000;
  const audioWaveform = Float32Array.from({ length: sampleRate * 2 }, (_, index) =>
    Math.sin((2 * Math.PI * 440 * index) / sampleRate) * 0.5);
  const audioData = {
    channelWaveforms: [audioWaveform],
    sampleRate,
    durationInSeconds: 2,
    numberOfChannels: 1,
    resultId: 'three-audio-reactive-fixture',
  };
  const audioUrl = api.audioBufferToDataUrl?.({
    numberOfChannels: 1,
    sampleRate,
    getChannelData: () => audioWaveform,
  });
  if (!audioUrl?.startsWith('data:audio/wav;base64,')) {
    throw new Error('audioBufferToDataUrl did not return a WAV data URL');
  }
  api.registerThreeComposition('three_audio_reactive_preview', {
    setup({ THREE }) {
      const scene = new THREE.Scene();
      const camera = new THREE.PerspectiveCamera(45, 16 / 9, 0.1, 100);
      camera.position.z = 3;
      const mesh = new THREE.Mesh(
        new THREE.IcosahedronGeometry(0.6, 2),
        new THREE.MeshStandardMaterial({ color: 0xff8a3d, roughness: 0.3, metalness: 0.2 }),
      );
      scene.add(new THREE.AmbientLight(0xffffff, 2));
      scene.add(mesh);
      return { scene, camera, mesh };
    },
    render({ renderer, scene, camera, mesh, frame, fps, width, height }) {
      const spectrum = api.visualizeAudio({
        audioData, frame, fps, numberOfSamples: 32, smoothing: false,
      });
      const energy = spectrum.reduce((sum, value) => sum + value, 0) / spectrum.length;
      mesh.scale.setScalar(0.75 + energy * 2);
      mesh.rotation.y = frame / Math.max(fps, 1);
      renderer.setSize(width, height, false);
      camera.aspect = width / Math.max(height, 1);
      camera.updateProjectionMatrix();
      renderer.render(scene, camera);
    },
    dispose(instance) {
      instance?.mesh?.geometry?.dispose?.();
      instance?.mesh?.material?.dispose?.();
    },
  });
}
