// Framework-free reference composition for registerThreeComposition().
export function register(api) {
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
}
