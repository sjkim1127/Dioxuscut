use super::*;
use crate::scene::{Color, ImageFit};
use crate::video_cache::canonical_local_path;
use wgpu::util::DeviceExt;

pub(crate) struct FrameSubmission<'a> {
    pub(crate) commands: &'a [DrawCommand],
    pub(crate) path_mask: Option<&'a GpuPathMask>,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) sampling_fps: f64,
    pub(crate) slot: &'a GpuFrameSlot,
    pub(crate) shader_suffix: Option<&'a [SceneNode]>,
    pub(crate) readback: bool,
}

pub(crate) struct SubmittedFrame {
    pub(crate) submission_index: wgpu::SubmissionIndex,
    pub(crate) readback_rx: Option<std::sync::mpsc::Receiver<Result<(), wgpu::BufferAsyncError>>>,
}

impl WgpuBackend {
    pub(crate) fn offscreen_layer_slot(&self, width: u32, height: u32) -> Arc<Mutex<GpuFrameSlot>> {
        let mut pool = self
            .layer_resources
            .lock()
            .expect("GPU layer resource pool mutex poisoned");
        pool.entry((width, height))
            .or_insert_with(|| {
                Arc::new(Mutex::new(GpuFrameSlot::new(
                    &self.ctx.device,
                    width,
                    height,
                )))
            })
            .clone()
    }

    /// Create the shared texture bind group used by image, video, text-atlas,
    /// and future resolved offscreen-layer inputs. The texture view remains
    /// owned by the caller, which is required for layer-slot ownership.
    pub(crate) fn image_bind_group(
        &self,
        view: &wgpu::TextureView,
        sampler: &wgpu::Sampler,
        label: &'static str,
    ) -> wgpu::BindGroup {
        self.ctx
            .device
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(label),
                layout: &self.ctx.image_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(sampler),
                    },
                ],
            })
    }

    #[allow(dead_code)]
    pub(crate) fn offscreen_layer_binding(&self, slot: &GpuFrameSlot) -> GpuExternalTexture {
        let sampler = self.ctx.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("offscreen_layer_sampler"),
            ..Default::default()
        });
        let bind_group =
            self.image_bind_group(&slot.texture_view, &sampler, "offscreen_layer_texture_bg");
        GpuExternalTexture {
            _sampler: sampler,
            bind_group,
        }
    }

    /// Composite a resolved layer texture into another resolved target. This
    /// pass intentionally uses the single-sample pipeline: the destination is
    /// already resolved, so loading it as an MSAA attachment would be invalid.
    #[allow(dead_code)]
    pub(crate) fn composite_external_texture(
        &self,
        source: &GpuFrameSlot,
        destination: &GpuFrameSlot,
        width: u32,
        height: u32,
        opacity: f32,
    ) -> Result<wgpu::SubmissionIndex, RasterError> {
        if !opacity.is_finite() || !(0.0..=1.0).contains(&opacity) {
            return Err(RasterError::Scene("invalid offscreen layer opacity".into()));
        }
        let device = &self.ctx.device;
        let globals = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("layer_composite_globals"),
            contents: bytemuck_cast(&[width as f32, height as f32, 0.0, 0.0]),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let globals_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("layer_composite_globals_bg"),
            layout: &self.ctx.globals_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: globals.as_entire_binding(),
            }],
        });
        let mut instance = GpuInstance::solid(Color::WHITE, opacity, Transform::identity());
        instance.kind_data[0] = 5;
        instance.bounds = [0.0, 0.0, width as f32, height as f32];
        instance.shape_bounds = instance.bounds;
        instance.params = [0.0, 0.0, 1.0, 1.0];
        let instances = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("layer_composite_instance"),
            contents: bytemuck_cast(&[instance]),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let instances_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("layer_composite_instances_bg"),
            layout: &self.ctx.instance_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: instances.as_entire_binding(),
            }],
        });
        let external = self.offscreen_layer_binding(source);
        let mask_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("layer_composite_dummy_mask"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let mask_view = mask_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let mask_sampler = device.create_sampler(&wgpu::SamplerDescriptor::default());
        let mask_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("layer_composite_dummy_mask_bg"),
            layout: &self.ctx.path_mask_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&mask_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&mask_sampler),
                },
            ],
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("layer_composite_encoder"),
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("layer_composite_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &destination.texture_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(&self.ctx.image_composite_pipeline);
            pass.set_bind_group(0, &globals_bg, &[]);
            pass.set_bind_group(1, &instances_bg, &[]);
            pass.set_bind_group(2, &external.bind_group, &[]);
            pass.set_bind_group(3, &mask_bg, &[]);
            pass.draw(0..6, 0..1);
        }
        Ok(self.ctx.queue.submit(Some(encoder.finish())))
    }

    pub(crate) fn readback_resolved_slot(
        &self,
        slot: &GpuFrameSlot,
        width: u32,
        height: u32,
    ) -> Result<RgbaImage, RasterError> {
        let mut encoder = self
            .ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("layer_composite_readback_encoder"),
            });
        encoder.copy_texture_to_buffer(
            slot.texture.as_image_copy(),
            wgpu::ImageCopyBuffer {
                buffer: &slot.readback,
                layout: wgpu::ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(slot.bytes_per_row),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        let submission = self.ctx.queue.submit(Some(encoder.finish()));
        let (tx, rx) = std::sync::mpsc::channel();
        slot.readback
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = tx.send(result);
            });
        let _ = submission;
        self.ctx.device.poll(wgpu::Maintain::Wait);
        rx.recv()
            .map_err(|error| RasterError::Scene(format!("layer readback channel failed: {error}")))?
            .map_err(|error| RasterError::Scene(format!("layer readback map failed: {error}")))?;
        let slice = slot.readback.slice(..);
        let mapped = slice.get_mapped_range();
        let mut pixels = Vec::with_capacity((width * height * 4) as usize);
        for row in 0..height {
            let start = (row * slot.bytes_per_row) as usize;
            pixels.extend_from_slice(&mapped[start..start + (width * 4) as usize]);
        }
        drop(mapped);
        slot.readback.unmap();
        RgbaImage::from_raw(width, height, pixels)
            .ok_or_else(|| RasterError::ImageEncode("Invalid layer composite readback".into()))
    }

    pub(crate) fn render_trailing_overlap_layer(
        &self,
        base_scene: &Scene,
        layer_scene: &Scene,
        layer_opacity: f32,
        config: &FrameConfig,
    ) -> Result<RgbaImage, RasterError> {
        let Some((base_commands, base_mask)) =
            compile_scene_with_path_mask(base_scene, &self.fallback)
        else {
            return Err(RasterError::Scene(
                "overlap base scene requires CPU fallback".into(),
            ));
        };
        let Some((layer_commands, layer_mask)) =
            compile_scene_with_path_mask(layer_scene, &self.fallback)
        else {
            return Err(RasterError::Scene(
                "overlap layer requires CPU fallback".into(),
            ));
        };
        if base_mask.is_some() || layer_mask.is_some() {
            return Err(RasterError::Scene(
                "overlap layer path masks are not yet supported".into(),
            ));
        }
        let resource = {
            let mut pool = self
                .frame_resources
                .lock()
                .map_err(|_| RasterError::Init("GPU resource pool mutex poisoned".into()))?;
            pool.entry((config.width, config.height))
                .or_insert_with(|| {
                    Arc::new(Mutex::new(GpuFrameResources::new(
                        &self.ctx.device,
                        config.width,
                        config.height,
                    )))
                })
                .clone()
        };
        let mut resources = resource
            .lock()
            .map_err(|_| RasterError::Init("GPU frame resource mutex poisoned".into()))?;
        let output_index = resources.active_index % RING_BUFFER_SIZE;
        resources.active_index = resources.active_index.wrapping_add(1);
        let layer_slot = self.offscreen_layer_slot(config.width, config.height);
        let layer_slot = layer_slot
            .lock()
            .map_err(|_| RasterError::Init("GPU layer resource mutex poisoned".into()))?;
        let output_slot = &resources.slots[output_index];
        let base_frame = self.submit_frame_to_slot(FrameSubmission {
            commands: &base_commands,
            path_mask: None,
            width: config.width,
            height: config.height,
            sampling_fps: config.fps,
            slot: output_slot,
            shader_suffix: None,
            readback: false,
        })?;
        let base_submission = base_frame.submission_index;
        let layer_frame = self.submit_frame_to_slot(FrameSubmission {
            commands: &layer_commands,
            path_mask: None,
            width: config.width,
            height: config.height,
            sampling_fps: config.fps,
            slot: &layer_slot,
            shader_suffix: None,
            readback: false,
        })?;
        let layer_submission = layer_frame.submission_index;
        self.ctx
            .device
            .poll(wgpu::Maintain::wait_for(base_submission));
        self.ctx
            .device
            .poll(wgpu::Maintain::wait_for(layer_submission));
        let composite_submission = self.composite_external_texture(
            &layer_slot,
            output_slot,
            config.width,
            config.height,
            layer_opacity,
        )?;
        self.ctx
            .device
            .poll(wgpu::Maintain::wait_for(composite_submission));
        let image = self.readback_resolved_slot(output_slot, config.width, config.height)?;
        self.gpu_frame_count.fetch_add(1, Ordering::Relaxed);
        Ok(image)
    }

    pub(crate) fn gpu_pixels(
        &self,
        key: &str,
        decoded: &image::RgbaImage,
    ) -> Result<Arc<GpuImageResource>, RasterError> {
        let mut cache = self
            .gpu_images
            .lock()
            .expect("GPU image cache lock poisoned");
        if let Some(image) = cache.images.get(key).cloned() {
            self.gpu_texture_cache_hits.fetch_add(1, Ordering::Relaxed);
            cache.lru.retain(|entry| entry != key);
            cache.lru.push_back(key.to_string());
            return Ok(image);
        }
        drop(cache);
        self.gpu_texture_cache_misses
            .fetch_add(1, Ordering::Relaxed);
        let upload_start = Instant::now();

        let device = &self.ctx.device;
        let queue = &self.ctx.queue;
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("image_texture"),
            size: wgpu::Extent3d {
                width: decoded.width(),
                height: decoded.height(),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: RENDER_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            texture.as_image_copy(),
            decoded.as_raw(),
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(decoded.width() * 4),
                rows_per_image: Some(decoded.height()),
            },
            wgpu::Extent3d {
                width: decoded.width(),
                height: decoded.height(),
                depth_or_array_layers: 1,
            },
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor::default());
        let bind_group = self.image_bind_group(&view, &sampler, "image_bg");
        let resource = Arc::new(GpuImageResource {
            _texture: texture,
            _view: view,
            _sampler: sampler,
            bind_group,
            width: decoded.width(),
            height: decoded.height(),
        });
        self.texture_upload_ns
            .fetch_add(upload_start.elapsed().as_nanos() as u64, Ordering::Relaxed);
        self.gpu_texture_uploads.fetch_add(1, Ordering::Relaxed);
        self.gpu_texture_upload_bytes.fetch_add(
            u64::from(resource.width) * u64::from(resource.height) * 4,
            Ordering::Relaxed,
        );
        let mut cache = self
            .gpu_images
            .lock()
            .expect("GPU image cache lock poisoned");
        if let Some(existing) = cache.images.get(key).cloned() {
            cache.lru.retain(|entry| entry != key);
            cache.lru.push_back(key.to_string());
            return Ok(existing);
        }
        let bytes = resource.width as usize * resource.height as usize * 4;
        cache.images.insert(key.to_string(), Arc::clone(&resource));
        cache.lru.push_back(key.to_string());
        cache.bytes = cache.bytes.saturating_add(bytes);
        cache.trim_to_budget();
        Ok(resource)
    }

    pub(crate) fn gpu_image(&self, src: &str) -> Result<Arc<GpuImageResource>, RasterError> {
        let decoded = self.image_cache.load(src)?;
        // ImageCache canonicalizes local paths before indexing them. Keep the
        // GPU cache on the same identity so alternate spellings of one asset
        // (relative, absolute, or file://) do not trigger duplicate uploads.
        let key = if src.trim_start().starts_with("data:") {
            src.trim().to_string()
        } else {
            canonical_local_path(src)?.display().to_string()
        };
        self.gpu_pixels(&key, &decoded)
    }

    pub(crate) fn gpu_text_atlas(
        &self,
        snapshot: &crate::text_atlas::TextAtlasSnapshot,
    ) -> Arc<GpuTextAtlasResource> {
        let queue = &self.ctx.queue;
        let mut cache = self
            .text_atlas
            .lock()
            .expect("text atlas GPU lock poisoned");
        if let Some(resource) = cache.as_ref() {
            if resource.width == snapshot.width && resource.height == snapshot.height {
                let generation = resource.generation.load(Ordering::Acquire);
                if generation == snapshot.generation {
                    self.text_atlas_cache_hits.fetch_add(1, Ordering::Relaxed);
                    return resource.clone();
                }
                if let Some(rect) = snapshot.dirty {
                    let mut upload = Vec::with_capacity((rect.width * rect.height) as usize);
                    for row in 0..rect.height {
                        let start = ((rect.y + row) * snapshot.width + rect.x) as usize;
                        upload.extend_from_slice(
                            &snapshot.pixels[start..start + rect.width as usize],
                        );
                    }
                    queue.write_texture(
                        wgpu::ImageCopyTexture {
                            texture: &resource._texture,
                            mip_level: 0,
                            origin: wgpu::Origin3d {
                                x: rect.x,
                                y: rect.y,
                                z: 0,
                            },
                            aspect: wgpu::TextureAspect::All,
                        },
                        &upload,
                        wgpu::ImageDataLayout {
                            offset: 0,
                            bytes_per_row: Some(rect.width),
                            rows_per_image: Some(rect.height),
                        },
                        wgpu::Extent3d {
                            width: rect.width,
                            height: rect.height,
                            depth_or_array_layers: 1,
                        },
                    );
                    self.text_atlas_upload_bytes.fetch_add(
                        u64::from(rect.width) * u64::from(rect.height),
                        Ordering::Relaxed,
                    );
                    self.text_atlas_dirty_uploads
                        .fetch_add(1, Ordering::Relaxed);
                    self.text_atlas_dirty_upload_bytes.fetch_add(
                        u64::from(rect.width) * u64::from(rect.height),
                        Ordering::Relaxed,
                    );
                    resource
                        .generation
                        .store(snapshot.generation, Ordering::Release);
                    return resource.clone();
                }
            }
        }
        let device = &self.ctx.device;
        let queue = &self.ctx.queue;
        self.text_atlas_cache_misses.fetch_add(1, Ordering::Relaxed);
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("text_atlas_texture"),
            size: wgpu::Extent3d {
                width: snapshot.width,
                height: snapshot.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            texture.as_image_copy(),
            &snapshot.pixels,
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(snapshot.width),
                rows_per_image: Some(snapshot.height),
            },
            wgpu::Extent3d {
                width: snapshot.width,
                height: snapshot.height,
                depth_or_array_layers: 1,
            },
        );
        self.text_atlas_upload_bytes.fetch_add(
            u64::from(snapshot.width) * u64::from(snapshot.height),
            Ordering::Relaxed,
        );
        self.text_atlas_full_uploads.fetch_add(1, Ordering::Relaxed);
        self.text_atlas_full_upload_bytes.fetch_add(
            u64::from(snapshot.width) * u64::from(snapshot.height),
            Ordering::Relaxed,
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor::default());
        let bind_group = self.image_bind_group(&view, &sampler, "text_atlas_bg");
        let resource = Arc::new(GpuTextAtlasResource {
            _texture: texture,
            _view: view,
            _sampler: sampler,
            bind_group,
            width: snapshot.width,
            height: snapshot.height,
            generation: AtomicU64::new(snapshot.generation),
        });
        *cache = Some(resource.clone());
        resource
    }

    pub(crate) fn gpu_video(
        &self,
        src: &str,
        time: f64,
        sampling_fps: f64,
        looped: bool,
    ) -> Result<Arc<GpuImageResource>, RasterError> {
        let decode_start = Instant::now();
        let frame = self.video_cache.load(src, time, sampling_fps, looped)?;
        self.video_decode_ns
            .fetch_add(decode_start.elapsed().as_nanos() as u64, Ordering::Relaxed);
        let frame_index = self
            .video_cache
            .frame_index_for(src, time, sampling_fps, looped)?;
        // VideoFrameCache canonicalizes local paths, so the GPU cache must use
        // the same identity. Otherwise `clip.mkv` and `file:///.../clip.mkv`
        // decode to the same frame but upload duplicate GPU textures.
        let canonical = canonical_local_path(src)?;
        let key = format!(
            "video:{}:{sampling_fps:.6}:{frame_index}",
            canonical.display()
        );
        self.gpu_pixels(&key, &frame)
    }

    #[cfg(test)]
    pub(crate) fn gpu_image_cache_len(&self) -> usize {
        self.gpu_images
            .lock()
            .expect("GPU image cache lock poisoned")
            .images
            .len()
    }

    #[cfg(test)]
    pub(crate) fn text_atlas_cache_generation(&self) -> Option<u64> {
        self.text_atlas
            .lock()
            .expect("text atlas GPU lock poisoned")
            .as_ref()
            .map(|resource| resource.generation.load(Ordering::Acquire))
    }

    /// Render a frame directly to a GPU texture view, passing it to `consume`.
    /// The rendered texture view is valid for the duration of
    /// the callback only; callers must not retain it after returning.
    pub fn render_frame_gpu<F>(
        &self,
        scene: &Scene,
        config: &FrameConfig,
        consume: F,
    ) -> Result<(), RasterError>
    where
        F: FnOnce(&wgpu::TextureView, u32, u32),
    {
        if config.width > self.ctx.max_texture_dimension_2d
            || config.height > self.ctx.max_texture_dimension_2d
        {
            return Err(RasterError::Scene(
                "GPU-native frame dimensions exceed the device limit".into(),
            ));
        }
        if scene
            .nodes
            .iter()
            .any(|node| matches!(node, SceneNode::Shader { .. }))
        {
            return Err(RasterError::Scene(
                "GPU-native frame sink does not yet support shader nodes".into(),
            ));
        }
        let Some((commands, path_mask)) = compile_scene_with_path_mask(scene, &self.fallback)
        else {
            return Err(RasterError::Scene(
                "scene requires the CPU fallback and has no GPU texture handle".into(),
            ));
        };

        let resource = {
            let mut pool = self
                .frame_resources
                .lock()
                .map_err(|_| RasterError::Init("GPU resource pool mutex poisoned".into()))?;
            pool.entry((config.width, config.height))
                .or_insert_with(|| {
                    Arc::new(Mutex::new(GpuFrameResources::new(
                        &self.ctx.device,
                        config.width,
                        config.height,
                    )))
                })
                .clone()
        };
        let mut resources = resource
            .lock()
            .map_err(|_| RasterError::Init("GPU frame resource mutex poisoned".into()))?;
        let slot_idx = resources.active_index % RING_BUFFER_SIZE;
        resources.active_index = resources.active_index.wrapping_add(1);
        let submitted = self.submit_frame_to_slot(FrameSubmission {
            commands: &commands,
            path_mask: path_mask.as_ref(),
            width: config.width,
            height: config.height,
            sampling_fps: config.fps,
            slot: &resources.slots[slot_idx],
            shader_suffix: None,
            readback: false,
        })?;
        let submission_index = submitted.submission_index;
        let rx = submitted.readback_rx;
        debug_assert!(rx.is_none());
        let gpu_submit_start = Instant::now();
        self.ctx
            .device
            .poll(wgpu::Maintain::wait_for(submission_index));
        self.gpu_submit_no_readback_ns.fetch_add(
            gpu_submit_start.elapsed().as_nanos() as u64,
            Ordering::Relaxed,
        );
        consume(
            &resources.slots[slot_idx].texture_view,
            config.width,
            config.height,
        );
        self.gpu_frame_count.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    /// Stream GPU-compatible frames directly to texture-view consumers.
    ///
    /// Frames are delivered in order and never copied to the CPU. The view is
    /// valid only for the duration of `consume`; a consumer that needs to
    /// retain GPU work must enqueue it before returning from the callback.
    pub fn render_stream_gpu<S, C, F>(
        &self,
        total: u32,
        scene_fn: &S,
        config_fn: &C,
        mut consume: F,
    ) -> Result<(), RasterError>
    where
        S: Fn(u32) -> Result<Scene, RasterError> + Sync,
        C: Fn(u32) -> FrameConfig + Sync,
        F: FnMut(u32, &wgpu::TextureView, u32, u32),
    {
        if total == 0 {
            return Ok(());
        }
        let first = config_fn(0);
        if first.width > self.ctx.max_texture_dimension_2d
            || first.height > self.ctx.max_texture_dimension_2d
        {
            return Err(RasterError::Scene(
                "GPU-native stream dimensions exceed the device limit".into(),
            ));
        }
        let resource = {
            let mut pool = self
                .frame_resources
                .lock()
                .map_err(|_| RasterError::Init("GPU resource pool mutex poisoned".into()))?;
            pool.entry((first.width, first.height))
                .or_insert_with(|| {
                    Arc::new(Mutex::new(GpuFrameResources::new(
                        &self.ctx.device,
                        first.width,
                        first.height,
                    )))
                })
                .clone()
        };
        let mut resources = resource
            .lock()
            .map_err(|_| RasterError::Init("GPU frame resource mutex poisoned".into()))?;
        let mut in_flight: std::collections::VecDeque<(u32, usize, wgpu::SubmissionIndex)> =
            std::collections::VecDeque::with_capacity(RING_BUFFER_SIZE);

        for frame in 0..total {
            let scene = scene_fn(frame)?;
            let config = config_fn(frame);
            if (config.width, config.height) != (first.width, first.height) {
                return Err(RasterError::Scene(
                    "GPU-native stream cannot change dimensions mid-stream".into(),
                ));
            }
            let Some((commands, path_mask)) = compile_scene_with_path_mask(&scene, &self.fallback)
            else {
                return Err(RasterError::Scene(format!(
                    "frame {frame} requires the CPU fallback"
                )));
            };
            if scene
                .nodes
                .iter()
                .any(|node| matches!(node, SceneNode::Shader { .. }))
            {
                return Err(RasterError::Scene(format!(
                    "frame {frame} contains unsupported shader nodes"
                )));
            }
            if in_flight.len() == RING_BUFFER_SIZE {
                let (queued_frame, queued_slot, submission_index) = in_flight
                    .pop_front()
                    .expect("GPU-native in-flight ring length was checked");
                let gpu_submit_start = Instant::now();
                self.ctx
                    .device
                    .poll(wgpu::Maintain::wait_for(submission_index));
                self.gpu_submit_no_readback_ns.fetch_add(
                    gpu_submit_start.elapsed().as_nanos() as u64,
                    Ordering::Relaxed,
                );
                consume(
                    queued_frame,
                    &resources.slots[queued_slot].texture_view,
                    first.width,
                    first.height,
                );
            }
            let slot_idx = resources.active_index % RING_BUFFER_SIZE;
            resources.active_index = resources.active_index.wrapping_add(1);
            let submitted = self.submit_frame_to_slot(FrameSubmission {
                commands: &commands,
                path_mask: path_mask.as_ref(),
                width: first.width,
                height: first.height,
                sampling_fps: config.fps,
                slot: &resources.slots[slot_idx],
                shader_suffix: None,
                readback: false,
            })?;
            let submission_index = submitted.submission_index;
            let rx = submitted.readback_rx;
            debug_assert!(rx.is_none());
            in_flight.push_back((frame, slot_idx, submission_index));
            self.gpu_frame_count.fetch_add(1, Ordering::Relaxed);
        }
        while let Some((queued_frame, queued_slot, submission_index)) = in_flight.pop_front() {
            let gpu_submit_start = Instant::now();
            self.ctx
                .device
                .poll(wgpu::Maintain::wait_for(submission_index));
            self.gpu_submit_no_readback_ns.fetch_add(
                gpu_submit_start.elapsed().as_nanos() as u64,
                Ordering::Relaxed,
            );
            consume(
                queued_frame,
                &resources.slots[queued_slot].texture_view,
                first.width,
                first.height,
            );
        }
        Ok(())
    }

    pub(crate) fn submit_frame_to_slot(
        &self,
        submission: FrameSubmission<'_>,
    ) -> Result<SubmittedFrame, RasterError> {
        let commands = submission.commands;
        let path_mask = submission.path_mask;
        let width = submission.width;
        let height = submission.height;
        let sampling_fps = submission.sampling_fps;
        let slot = submission.slot;
        let shader_suffix = submission.shader_suffix;
        let readback = submission.readback;
        let device = &self.ctx.device;
        let queue = &self.ctx.queue;

        // Globals uniform
        let globals_data: [f32; 4] = [width as f32, height as f32, 0.0, 0.0];
        let globals_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("globals_buf"),
            contents: bytemuck_cast(&globals_data),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let globals_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("globals_bg"),
            layout: &self.ctx.globals_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: globals_buf.as_entire_binding(),
            }],
        });

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("frame_encoder"),
        });

        let mut all_instances: Vec<GpuInstance> = commands.iter().map(|c| *c.instance()).collect();
        let mut image_resources = Vec::with_capacity(commands.len());
        let atlas_snapshot = self.fallback.take_text_atlas_snapshot();
        let atlas_resource = self.gpu_text_atlas(&atlas_snapshot);
        for (index, command) in commands.iter().enumerate() {
            let (source, fit) = match command {
                DrawCommand::Image { src, fit, .. } => (self.gpu_image(src)?, *fit),
                DrawCommand::Video {
                    src,
                    time,
                    looped,
                    fit,
                    ..
                } => (self.gpu_video(src, *time, sampling_fps, *looped)?, *fit),
                DrawCommand::Lottie { key, image, .. } => {
                    (self.gpu_pixels(key, image)?, ImageFit::Contain)
                }
                DrawCommand::Gif {
                    key, image, fit, ..
                } => (self.gpu_pixels(key, image)?, *fit),
                DrawCommand::Emoji { key, image, .. } => {
                    (self.gpu_pixels(key, image)?, ImageFit::Fill)
                }
                DrawCommand::AudioVisualizer { key, image, .. } => {
                    (self.gpu_pixels(key, image)?, ImageFit::Fill)
                }
                DrawCommand::Text { entry, .. } => {
                    all_instances[index].params[0] = entry.x as f32 / atlas_snapshot.width as f32;
                    all_instances[index].params[1] = entry.y as f32 / atlas_snapshot.height as f32;
                    all_instances[index].params[2] =
                        (entry.x + entry.width) as f32 / atlas_snapshot.width as f32;
                    all_instances[index].params[3] =
                        (entry.y + entry.height) as f32 / atlas_snapshot.height as f32;
                    image_resources.push(None);
                    continue;
                }
                _ => {
                    image_resources.push(None);
                    continue;
                }
            };
            let placement = image_placement(
                fit,
                source.width as f32,
                source.height as f32,
                all_instances[index].shape_bounds[0],
                all_instances[index].shape_bounds[1],
                all_instances[index].shape_bounds[2],
                all_instances[index].shape_bounds[3],
            )
            .ok_or_else(|| RasterError::ImageAsset {
                path: "video/image".into(),
                reason: "invalid image placement dimensions".into(),
            })?;
            all_instances[index].bounds = placement.destination;
            all_instances[index].shape_bounds = placement.destination;
            all_instances[index].params[0..4].copy_from_slice(&placement.source_uv);

            image_resources.push(Some(source));
        }

        let path_mask_instance_index = all_instances.len() as u32;
        if let Some(path_mask) = path_mask {
            all_instances.push(path_mask.instance);
        }

        if !all_instances.is_empty() {
            let instance_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("instances_storage_buf"),
                contents: bytemuck_cast(&all_instances),
                usage: wgpu::BufferUsages::STORAGE,
            });
            let instance_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("instances_bg"),
                layout: &self.ctx.instance_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: instance_buf.as_entire_binding(),
                }],
            });

            // The shared pipeline layout requires group 2 even for analytic
            // and mesh draws. A 1x1 dummy texture keeps those pipelines valid;
            // image draws replace it with their per-command texture group.
            let dummy_texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("dummy_image_texture"),
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            queue.write_texture(
                dummy_texture.as_image_copy(),
                &[0, 0, 0, 0],
                wgpu::ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(4),
                    rows_per_image: Some(1),
                },
                wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
            );
            let dummy_view = dummy_texture.create_view(&wgpu::TextureViewDescriptor::default());
            let dummy_sampler = device.create_sampler(&wgpu::SamplerDescriptor::default());
            let dummy_image_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("dummy_image_bg"),
                layout: &self.ctx.image_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&dummy_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&dummy_sampler),
                    },
                ],
            });

            let default_path_mask_texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("default_path_mask_texture"),
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::R8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            queue.write_texture(
                default_path_mask_texture.as_image_copy(),
                &[255],
                wgpu::ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(1),
                    rows_per_image: Some(1),
                },
                wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
            );
            let default_path_mask_view =
                default_path_mask_texture.create_view(&wgpu::TextureViewDescriptor::default());
            let default_path_mask_sampler =
                device.create_sampler(&wgpu::SamplerDescriptor::default());
            let default_path_mask_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("default_path_mask_bg"),
                layout: &self.ctx.path_mask_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&default_path_mask_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&default_path_mask_sampler),
                    },
                ],
            });

            let (_path_mask_texture, path_mask_view) = if path_mask.is_some() {
                let texture = device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("path_mask_texture"),
                    size: wgpu::Extent3d {
                        width,
                        height,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::R8Unorm,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                });
                let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
                (texture, view)
            } else {
                let texture = device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("default_path_mask_texture"),
                    size: wgpu::Extent3d {
                        width: 1,
                        height: 1,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::R8Unorm,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                    view_formats: &[],
                });
                queue.write_texture(
                    texture.as_image_copy(),
                    &[255],
                    wgpu::ImageDataLayout {
                        offset: 0,
                        bytes_per_row: Some(1),
                        rows_per_image: Some(1),
                    },
                    wgpu::Extent3d {
                        width: 1,
                        height: 1,
                        depth_or_array_layers: 1,
                    },
                );
                let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
                (texture, view)
            };
            let path_mask_sampler = device.create_sampler(&wgpu::SamplerDescriptor::default());
            let path_mask_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("path_mask_bg"),
                layout: &self.ctx.path_mask_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&path_mask_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&path_mask_sampler),
                    },
                ],
            });

            let mesh_buffers: Vec<Option<(wgpu::Buffer, wgpu::Buffer)>> = commands
                .iter()
                .map(|cmd| match cmd {
                    DrawCommand::Mesh {
                        vertices, indices, ..
                    } => {
                        let vb = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some("path_vertices"),
                            contents: bytemuck_cast(vertices),
                            usage: wgpu::BufferUsages::VERTEX,
                        });
                        let ib = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some("path_indices"),
                            contents: bytemuck_cast(indices),
                            usage: wgpu::BufferUsages::INDEX,
                        });
                        Some((vb, ib))
                    }
                    DrawCommand::Analytic { .. }
                    | DrawCommand::Image { .. }
                    | DrawCommand::Video { .. }
                    | DrawCommand::Lottie { .. }
                    | DrawCommand::Gif { .. }
                    | DrawCommand::Emoji { .. }
                    | DrawCommand::AudioVisualizer { .. }
                    | DrawCommand::Text { .. } => None,
                })
                .collect();

            let path_mask_buffers = path_mask.map(|mask| {
                let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("path_mask_vertices"),
                    contents: bytemuck_cast(&mask.vertices),
                    usage: wgpu::BufferUsages::VERTEX,
                });
                let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("path_mask_indices"),
                    contents: bytemuck_cast(&mask.indices),
                    usage: wgpu::BufferUsages::INDEX,
                });
                (vertex_buffer, index_buffer)
            });

            if let (Some(mask), Some((vertex_buffer, index_buffer))) =
                (path_mask, path_mask_buffers.as_ref())
            {
                let mut mask_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("path_mask_pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &path_mask_view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    ..Default::default()
                });
                mask_pass.set_pipeline(&self.ctx.path_mask_pipeline);
                mask_pass.set_bind_group(0, &globals_bg, &[]);
                mask_pass.set_bind_group(1, &instance_bg, &[]);
                mask_pass.set_bind_group(2, &dummy_image_bg, &[]);
                mask_pass.set_bind_group(3, &default_path_mask_bg, &[]);
                mask_pass.set_vertex_buffer(0, vertex_buffer.slice(..));
                mask_pass.set_index_buffer(index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                mask_pass.draw_indexed(
                    0..mask.indices.len() as u32,
                    0,
                    path_mask_instance_index..path_mask_instance_index + 1,
                );
            }

            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("frame_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &slot.msaa_view,
                    resolve_target: Some(&slot.texture_view),
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });

            pass.set_bind_group(0, &globals_bg, &[]);
            pass.set_bind_group(1, &instance_bg, &[]);
            pass.set_bind_group(2, &dummy_image_bg, &[]);
            pass.set_bind_group(3, &path_mask_bg, &[]);

            let mut i = 0;
            while i < commands.len() {
                match &commands[i] {
                    DrawCommand::Analytic { .. } => {
                        let start = i;
                        while i < commands.len()
                            && matches!(commands[i], DrawCommand::Analytic { .. })
                            && commands[i].instance().kind_data[2]
                                == commands[start].instance().kind_data[2]
                        {
                            i += 1;
                        }
                        let pipeline = match commands[start].instance().kind_data[2] {
                            1 => &self.ctx.multiply_pipeline,
                            2 => &self.ctx.screen_pipeline,
                            3 => &self.ctx.darken_pipeline,
                            4 => &self.ctx.lighten_pipeline,
                            _ => &self.ctx.pipeline,
                        };
                        pass.set_pipeline(pipeline);
                        pass.draw(0..6, start as u32..i as u32);
                    }
                    DrawCommand::Mesh { indices, .. } => {
                        let (vb, ib) = mesh_buffers[i].as_ref().expect("mesh buffers allocated");
                        let pipeline = match commands[i].instance().kind_data[2] {
                            1 => &self.ctx.multiply_mesh_pipeline,
                            2 => &self.ctx.screen_mesh_pipeline,
                            3 => &self.ctx.darken_mesh_pipeline,
                            4 => &self.ctx.lighten_mesh_pipeline,
                            _ => &self.ctx.mesh_pipeline,
                        };
                        pass.set_pipeline(pipeline);
                        pass.set_vertex_buffer(0, vb.slice(..));
                        pass.set_index_buffer(ib.slice(..), wgpu::IndexFormat::Uint32);
                        pass.draw_indexed(0..indices.len() as u32, 0, i as u32..i as u32 + 1);
                        i += 1;
                    }
                    DrawCommand::Image { .. }
                    | DrawCommand::Video { .. }
                    | DrawCommand::Lottie { .. }
                    | DrawCommand::Gif { .. }
                    | DrawCommand::Emoji { .. }
                    | DrawCommand::AudioVisualizer { .. }
                    | DrawCommand::Text { .. } => {
                        if matches!(commands[i], DrawCommand::Text { .. }) {
                            let pipeline = match commands[i].instance().kind_data[2] {
                                1 => &self.ctx.multiply_text_pipeline,
                                2 => &self.ctx.screen_text_pipeline,
                                3 => &self.ctx.darken_text_pipeline,
                                4 => &self.ctx.lighten_text_pipeline,
                                _ => &self.ctx.text_pipeline,
                            };
                            pass.set_pipeline(pipeline);
                            pass.set_bind_group(2, &atlas_resource.bind_group, &[]);
                        } else {
                            let pipeline = match commands[i].instance().kind_data[2] {
                                1 => &self.ctx.multiply_image_pipeline,
                                2 => &self.ctx.screen_image_pipeline,
                                3 => &self.ctx.darken_image_pipeline,
                                4 => &self.ctx.lighten_image_pipeline,
                                _ => &self.ctx.image_pipeline,
                            };
                            pass.set_pipeline(pipeline);
                            pass.set_bind_group(
                                2,
                                &image_resources[i]
                                    .as_ref()
                                    .expect("image resource")
                                    .bind_group,
                                &[],
                            );
                        }
                        pass.draw(0..6, i as u32..i as u32 + 1);
                        i += 1;
                    }
                }
            }
        } else {
            let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("clear_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &slot.msaa_view,
                    resolve_target: Some(&slot.texture_view),
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
        }

        if let Some(shader_nodes) = shader_suffix {
            for node in shader_nodes {
                let SceneNode::Shader {
                    x,
                    y,
                    w,
                    h,
                    source,
                    time,
                    params,
                    opacity,
                } = node
                else {
                    return Err(RasterError::Scene(
                        "GPU shader suffix contains a non-shader node".into(),
                    ));
                };
                if ![*x, *y, *w, *h, *time, *opacity]
                    .iter()
                    .all(|value| value.is_finite())
                    || !(*opacity >= 0.0 && *opacity <= 1.0)
                    || *x < 0.0
                    || *y < 0.0
                    || *w <= 0.0
                    || *h <= 0.0
                    || *x + *w > width as f32
                    || *y + *h > height as f32
                {
                    return Err(RasterError::Scene(
                        "invalid GPU shader suffix region".into(),
                    ));
                }
                let shader_source = shader_source_with_opacity(source, *opacity);
                self.shader_runner.render_into_region(
                    &mut encoder,
                    &slot.texture_view,
                    crate::shader::ShaderRenderRegion {
                        target_width: width,
                        target_height: height,
                        x: *x,
                        y: *y,
                        width: *w,
                        height: *h,
                    },
                    crate::shader::ShaderRenderParams {
                        time: *time,
                        params: *params,
                        source: &shader_source,
                        load: wgpu::LoadOp::Load,
                    },
                )?;
            }
        }

        if readback {
            encoder.copy_texture_to_buffer(
                slot.texture.as_image_copy(),
                wgpu::ImageCopyBuffer {
                    buffer: &slot.readback,
                    layout: wgpu::ImageDataLayout {
                        offset: 0,
                        bytes_per_row: Some(slot.bytes_per_row),
                        rows_per_image: Some(height),
                    },
                },
                wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
            );
        }

        let submission_index = queue.submit([encoder.finish()]);

        let rx = if readback {
            let (tx, rx) = std::sync::mpsc::channel();
            slot.readback
                .slice(..)
                .map_async(wgpu::MapMode::Read, move |result| {
                    let _ = tx.send(result);
                });
            Some(rx)
        } else {
            None
        };

        Ok(SubmittedFrame {
            submission_index,
            readback_rx: rx,
        })
    }

    #[allow(clippy::type_complexity)]
    pub(crate) fn drain_slot(
        &self,
        in_flight: InFlight,
        res: &GpuFrameResources,
        width: u32,
        height: u32,
        scratch: &mut Vec<u8>,
        sink: &mut dyn FrameSink,
    ) -> Result<(), RasterError> {
        self.ctx
            .device
            .poll(wgpu::Maintain::wait_for(in_flight.submission_index));
        in_flight
            .rx
            .recv()
            .map_err(|_| RasterError::Frame {
                frame: in_flight.frame_idx,
                reason: "GPU readback channel error".into(),
            })?
            .map_err(|e| RasterError::Frame {
                frame: in_flight.frame_idx,
                reason: format!("GPU map error: {e:?}"),
            })?;

        let slot = &res.slots[in_flight.slot_idx];
        let bytes_per_row = slot.bytes_per_row;
        let expected_row_bytes = (width * 4) as usize;
        let total_bytes = expected_row_bytes * height as usize;
        let slice = slot.readback.slice(..);
        let data = slice.get_mapped_range();

        let res = if bytes_per_row as usize == expected_row_bytes {
            sink.consume(in_flight.frame_idx, &data[..total_bytes])
        } else {
            scratch.clear();
            scratch.reserve_exact(total_bytes);
            for row in 0..height {
                let start = (row * bytes_per_row) as usize;
                let end = start + expected_row_bytes;
                scratch.extend_from_slice(&data[start..end]);
            }
            sink.consume(in_flight.frame_idx, scratch)
        };
        drop(data);
        slot.readback.unmap();

        res
    }
}

impl WgpuBackend {
    pub(crate) fn render_interleaved_shader_scene(
        &self,
        scene: &Scene,
        config: &FrameConfig,
    ) -> Result<RgbaImage, RasterError> {
        let mut output = RgbaImage::new(config.width, config.height);
        let mut regular_nodes = Vec::new();
        for node in &scene.nodes {
            if matches!(node, SceneNode::Shader { .. }) {
                if !regular_nodes.is_empty() {
                    let segment = Scene {
                        nodes: std::mem::take(&mut regular_nodes),
                    };
                    let rendered = self.render_frame(&segment, config)?;
                    image::imageops::overlay(&mut output, &rendered, 0, 0);
                }
                let shader_scene = Scene {
                    nodes: vec![node.clone()],
                };
                let rendered = match node {
                    SceneNode::Shader {
                        x,
                        y,
                        w,
                        h,
                        source,
                        opacity,
                        ..
                    } if *opacity >= 0.0
                        && *opacity <= 1.0
                        && *x >= 0.0
                        && *y >= 0.0
                        && *w > 0.0
                        && *h > 0.0
                        && *x + *w <= config.width as f32
                        && *y + *h <= config.height as f32
                        && shader_opacity_supported(source) =>
                    {
                        self.render_shader_layers_direct(&shader_scene, config)?
                    }
                    _ => self.render_shader_layers(&shader_scene, config)?,
                };
                image::imageops::overlay(&mut output, &rendered, 0, 0);
            } else {
                regular_nodes.push(node.clone());
            }
        }
        if !regular_nodes.is_empty() {
            let rendered = self.render_frame(
                &Scene {
                    nodes: regular_nodes,
                },
                config,
            )?;
            image::imageops::overlay(&mut output, &rendered, 0, 0);
        }
        Ok(output)
    }

    /// Render opaque shader layers directly into one GPU target and read it
    /// back once. Opacity layers retain the compatibility path until opacity
    /// is carried as a target-pass uniform.
    pub(crate) fn render_shader_layers_direct(
        &self,
        scene: &Scene,
        config: &FrameConfig,
    ) -> Result<RgbaImage, RasterError> {
        let width = config.width;
        let height = config.height;
        let texture = self.ctx.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("shader_scene_target"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("shader_scene_encoder"),
            });
        for (index, node) in scene.nodes.iter().enumerate() {
            let SceneNode::Shader {
                x,
                y,
                w,
                h,
                source,
                time,
                params,
                opacity,
            } = node
            else {
                unreachable!("direct shader path was checked before rendering");
            };
            if ![*x, *y, *w, *h, *time, *opacity]
                .iter()
                .all(|value| value.is_finite())
                || !(*opacity >= 0.0 && *opacity <= 1.0)
                || *w <= 0.0
                || *h <= 0.0
                || *x < 0.0
                || *y < 0.0
            {
                return Err(RasterError::Scene("invalid direct shader region".into()));
            }
            let shader_source = shader_source_with_opacity(source, *opacity);
            self.shader_runner.render_into_region(
                &mut encoder,
                &view,
                crate::shader::ShaderRenderRegion {
                    target_width: width,
                    target_height: height,
                    x: *x,
                    y: *y,
                    width: *w,
                    height: *h,
                },
                crate::shader::ShaderRenderParams {
                    time: *time,
                    params: *params,
                    source: &shader_source,
                    load: if index == 0 {
                        wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT)
                    } else {
                        wgpu::LoadOp::Load
                    },
                },
            )?;
        }
        let bytes_per_row = (width * 4 + 255) & !255;
        let staging = self.ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("shader_scene_readback"),
            size: (bytes_per_row * height) as wgpu::BufferAddress,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            wgpu::ImageCopyTexture {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::ImageCopyBuffer {
                buffer: &staging,
                layout: wgpu::ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        self.ctx.queue.submit(Some(encoder.finish()));
        let slice = staging.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = tx.send(result);
        });
        self.ctx.device.poll(wgpu::Maintain::Wait);
        rx.recv()
            .map_err(|error| RasterError::Scene(format!("GPU readback channel failed: {error}")))?
            .map_err(|error| RasterError::Scene(format!("GPU readback map failed: {error}")))?;
        let mapped = slice.get_mapped_range();
        let mut pixels = Vec::with_capacity((width * height * 4) as usize);
        for row in 0..height {
            let start = (row * bytes_per_row) as usize;
            pixels.extend_from_slice(&mapped[start..start + (width * 4) as usize]);
        }
        drop(mapped);
        staging.unmap();
        RgbaImage::from_raw(width, height, pixels)
            .ok_or_else(|| RasterError::ImageEncode("Invalid shader scene readback".into()))
    }

    /// Render shader nodes on the GPU and composite their rectangular outputs
    /// in scene order. Non-shader nodes deliberately remain outside this path
    /// until an offscreen GPU texture is available for general scene blending.
    pub(crate) fn render_shader_layers(
        &self,
        scene: &Scene,
        config: &FrameConfig,
    ) -> Result<RgbaImage, RasterError> {
        let mut output = RgbaImage::new(config.width, config.height);
        self.composite_shader_layers(&mut output, &scene.nodes)?;
        Ok(output)
    }

    pub(crate) fn composite_shader_layers(
        &self,
        output: &mut RgbaImage,
        nodes: &[SceneNode],
    ) -> Result<(), RasterError> {
        for node in nodes {
            let SceneNode::Shader {
                x,
                y,
                w,
                h,
                source,
                time,
                params,
                opacity,
            } = node
            else {
                unreachable!("shader layer path was checked before rendering");
            };
            if ![*x, *y, *w, *h, *opacity]
                .iter()
                .all(|value| value.is_finite())
                || *w <= 0.0
                || *h <= 0.0
                || !(0.0..=1.0).contains(opacity)
            {
                return Err(RasterError::Scene(
                    "invalid shader layer bounds or opacity".into(),
                ));
            }
            let width = (*w).round().max(1.0) as u32;
            let height = (*h).round().max(1.0) as u32;
            let mut layer = self
                .shader_runner
                .render(width, height, *time, *params, source)?;
            if *opacity < 1.0 {
                for pixel in layer.pixels_mut() {
                    pixel[3] = (f32::from(pixel[3]) * *opacity).round() as u8;
                }
            }
            image::imageops::overlay(output, &layer, (*x).round() as i64, (*y).round() as i64);
        }
        Ok(())
    }
}
