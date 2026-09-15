export function register(api) {
  api.registerComposition('webm_decode', async ({ props, width, height }) => {
    const frames = await api.decodeWebmVideo(props.source, {
      codec: 'vp8',
      maxSamples: 1,
    });
    const decoded = frames[0];
    try {
      const rgba = await api.videoFrameToRgba(decoded);
      const canvas = document.querySelector('#preview-canvas');
      canvas.width = rgba.width;
      canvas.height = rgba.height;
      const context = canvas.getContext('2d');
      if (!context) throw new Error('WebM test composition requires a 2D canvas');
      context.putImageData(
        new ImageData(new Uint8ClampedArray(rgba.rgba), rgba.width, rgba.height),
        0,
        0,
      );
      if (width !== undefined && height !== undefined) {
        canvas.dataset.requestedSize = `${width}x${height}`;
      }
    } finally {
      decoded.close();
    }
  });
}
