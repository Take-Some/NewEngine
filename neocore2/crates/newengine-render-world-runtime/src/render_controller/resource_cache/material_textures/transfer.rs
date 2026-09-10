impl RuntimeRenderController {
    fn queue_dictionary_material_texture_decode(
        &mut self,
        path: String,
        thread_pool: Option<&ThreadPoolHandle>,
    ) {
        if self.gpu.material.texture_decode_jobs.contains_key(&path) {
            self.gpu.material.textures.insert(
                path,
                MaterialTextureGpuResidency::CpuDecoding {
                    requested_frame: self.frame.frame_index,
                },
            );
            return;
        }

        let Some(thread_pool) = thread_pool else {
            let message = "engine.threading unavailable for material texture decode".to_owned();
            newengine_ulog_api::ulog::warn!(
                "render controller: material texture decode skipped path='{}' err='{}'",
                path,
                message
            );
            self.gpu
                .material
                .textures
                .insert(path, MaterialTextureGpuResidency::Failed { message });
            return;
        };
        let worker_path = path.clone();
        let result = Arc::new(Mutex::new(None));
        let result_out = Arc::clone(&result);
        let priority = self
            .gpu
            .material
            .texture_priorities
            .get(&path)
            .copied()
            .unwrap_or_else(MaterialTexturePriority::secondary);
        let request = material_texture_decode_request(&path, self.frame.frame_index, priority);
        let host_context = newengine_plugin_host::current_host_context();
        let ticket = thread_pool.submit_request(request, move || {
            let decoded = newengine_plugin_host::with_host_context(&host_context, || {
                let assets = AssetServiceClient::new(default_host_api());
                assets.textures_entry_runtime_ref_v1_typed(&worker_path)
            });
            *result_out.lock() = Some(decoded);
        });

        self.gpu
            .material
            .texture_decode_jobs
            .insert(path.clone(), MaterialTextureDecodeJob { ticket, result });
        self.gpu.material.textures.insert(
            path,
            MaterialTextureGpuResidency::CpuDecoding {
                requested_frame: self.frame.frame_index,
            },
        );
    }

    fn upload_decoded_material_texture(
        &mut self,
        r: &mut dyn newengine_core::render::RenderApi,
        path: String,
        texture_asset: RuntimeTextureAsset,
    ) {
        let texture_width = texture_asset.width;
        let texture_height = texture_asset.height;
        let texture_format = texture_asset.format;
        let extent = Extent2D::new(texture_width, texture_height);
        let mip_levels = NonZeroU32::new(texture_asset.mips.len().max(1) as u32)
            .expect("runtime texture mip count is non-zero");

        // Uncompressed RGBA dictionaries are normalized to a base-level upload and
        // backend-generated mip chain. Sending many small RGBA BufferImageCopy regions
        // through the deferred explicit-mip path has proven driver-fragile and can
        // escalate from a bad submission to VK_ERROR_DEVICE_LOST. BCn payloads cannot
        // be blitted safely, so they keep their authored runtime mip chain.
        let (payload, mip_data, upload_contract) = match texture_format {
            RuntimeTextureFormat::Rgba8Unorm | RuntimeTextureFormat::Rgba8Srgb => {
                let Some(base_mip) = texture_asset
                    .mips
                    .iter()
                    .find(|mip| mip.level == 0)
                    .or_else(|| texture_asset.mips.first())
                else {
                    let message = "runtime RGBA texture has no base mip".to_owned();
                    newengine_ulog_api::ulog::warn!(
                        "render controller: material texture rejected path='{}' err='{}'",
                        path,
                        message
                    );
                    self.gpu
                        .material
                        .textures
                        .insert(path, MaterialTextureGpuResidency::Failed { message });
                    return;
                };
                let expected_base_bytes = (texture_width as usize)
                    .saturating_mul(texture_height as usize)
                    .saturating_mul(4);
                if base_mip.level != 0
                    || base_mip.width != texture_width
                    || base_mip.height != texture_height
                    || base_mip.bytes.len() != expected_base_bytes
                {
                    let message = format!(
                        "runtime RGBA base mip contract mismatch level={} extent={}x{} bytes={} expected_level=0 expected_extent={}x{} expected_bytes={}",
                        base_mip.level,
                        base_mip.width,
                        base_mip.height,
                        base_mip.bytes.len(),
                        texture_width,
                        texture_height,
                        expected_base_bytes,
                    );
                    newengine_ulog_api::ulog::warn!(
                        "render controller: material texture rejected path='{}' err='{}'",
                        path,
                        message
                    );
                    self.gpu
                        .material
                        .textures
                        .insert(path, MaterialTextureGpuResidency::Failed { message });
                    return;
                }
                (base_mip.bytes.clone(), Vec::new(), "rgba-base+backend-mips")
            }
            _ => {
                let (payload, layout) = texture_asset.into_concatenated_payload_and_layout();
                let mip_data = layout
                    .into_iter()
                    .map(|mip| {
                        TextureMipDataDesc::new(
                            mip.level,
                            mip.width,
                            mip.height,
                            mip.offset,
                            mip.byte_len,
                        )
                    })
                    .collect();
                (payload, mip_data, "bcn-authored-mips")
            }
        };
        let payload_bytes = payload.len();
        if payload_bytes > super::super::render_quality::MATERIAL_TEXTURE_MAX_UPLOAD_PAYLOAD_BYTES {
            let message = format!(
                "texture upload payload exceeds runtime safety limit bytes={} limit={} format={:?} extent={}x{}; use BC-compressed runtime assets",
                payload_bytes,
                super::super::render_quality::MATERIAL_TEXTURE_MAX_UPLOAD_PAYLOAD_BYTES,
                texture_format,
                texture_width,
                texture_height,
            );
            newengine_ulog_api::ulog::warn!(
                "render controller: material texture rejected path='{}' err='{}'",
                path,
                message
            );
            self.gpu
                .material
                .textures
                .insert(path, MaterialTextureGpuResidency::Failed { message });
            return;
        }
        let upload_started = Instant::now();
        let texture_desc = TextureDesc::new(
            extent,
            render_texture_format_from_runtime(texture_format),
            TextureUsage::Sampled,
        )
        .with_label(format!("material_tex:{path}"))
        .with_mips(mip_levels);
        let texture_desc = if mip_data.is_empty() {
            texture_desc.with_deferred_data(payload)
        } else {
            texture_desc.with_deferred_mip_data(mip_data, payload)
        };
        match r.create_texture(texture_desc) {
            Ok(texture) => {
                newengine_ulog_api::ulog::debug!(
                    "render controller: material texture packet upload queued path='{}' method='assets.textures.entry_runtime_v1' contract='{}' texture={:?} frame={}",
                    path,
                    upload_contract,
                    texture,
                    self.frame.frame_index
                );
                let upload_elapsed_ms = upload_started.elapsed().as_secs_f32() * 1000.0;
                if upload_elapsed_ms >= MATERIAL_TEXTURE_ALLOCATION_STALL_WARN_MS {
                    newengine_ulog_api::ulog::warn!(
                        "render controller: texture allocation exceeded frame budget path='{}' bytes={} elapsed_ms={:.2} budget_ms={:.2} stall_warn_ms={:.2}",
                        path,
                        payload_bytes,
                        upload_elapsed_ms,
                        super::super::render_quality::MATERIAL_TEXTURE_DECODE_PUMP_BUDGET_MS,
                        MATERIAL_TEXTURE_ALLOCATION_STALL_WARN_MS,
                    );
                } else if upload_elapsed_ms
                    >= super::super::render_quality::MATERIAL_TEXTURE_DECODE_PUMP_BUDGET_MS
                {
                    newengine_ulog_api::ulog::debug!(
                        "render controller: texture allocation above pump target path='{}' bytes={} elapsed_ms={:.2} budget_ms={:.2}",
                        path,
                        payload_bytes,
                        upload_elapsed_ms,
                        super::super::render_quality::MATERIAL_TEXTURE_DECODE_PUMP_BUDGET_MS,
                    );
                }
                self.gpu.material.textures.insert(
                    path,
                    MaterialTextureGpuResidency::GpuLoading {
                        texture,
                        requested_frame: self.frame.frame_index,
                        last_residency_poll_frame: None,
                    },
                );
            }
            Err(e) => {
                let message = e.to_string();
                newengine_ulog_api::ulog::warn!(
                    "render controller: material texture create failed path='{}' err='{}'",
                    path,
                    message
                );
                self.gpu
                    .material
                    .textures
                    .insert(path, MaterialTextureGpuResidency::Failed { message });
            }
        }
    }

    fn poll_material_texture_decode_jobs(&mut self, max_completions: u32) {
        let max_completions = max_completions.max(1) as usize;
        let ready_paths = self
            .gpu
            .material
            .texture_decode_jobs
            .iter()
            .filter(|(_path, job)| job.is_complete())
            .map(|(path, _job)| path.clone())
            .take(max_completions)
            .collect::<Vec<_>>();

        for path in ready_paths {
            let Some(job) = self.gpu.material.texture_decode_jobs.remove(&path) else {
                continue;
            };

            let Some(result) = job.take_result() else {
                let message = "material texture decode job completed without result".to_owned();
                newengine_ulog_api::ulog::warn!(
                    "render controller: material texture decode job completed without result path='{}'",
                    path
                );
                self.gpu
                    .material
                    .textures
                    .insert(path, MaterialTextureGpuResidency::Failed { message });
                continue;
            };

            match result {
                Ok(texture_asset) => {
                    let payload_bytes = runtime_texture_payload_bytes(&texture_asset);
                    let priority = self
                        .gpu
                        .material
                        .texture_priorities
                        .get(&path)
                        .copied()
                        .unwrap_or_else(MaterialTexturePriority::secondary);
                    let frame = self.frame.frame_index;
                    self.gpu.material.texture_upload_queue.insert(
                        path.clone(),
                        MaterialTextureUploadCandidate {
                            asset: texture_asset,
                            payload_bytes,
                            priority,
                            decoded_frame: frame,
                            last_touched_frame: frame,
                        },
                    );
                    self.gpu.material.textures.insert(
                        path,
                        MaterialTextureGpuResidency::GpuQueued {
                            payload_bytes,
                            requested_frame: frame,
                        },
                    );
                }
                Err(e) if e.kind == AssetErrorKind::NotReady => {
                    if let Some(id_hex32) = e.id_hex32.clone() {
                        self.gpu.material.textures.insert(
                            path.clone(),
                            MaterialTextureGpuResidency::AssetLoading {
                                id_hex32,
                                requested_frame: self.frame.frame_index,
                            },
                        );
                    } else {
                        self.gpu
                            .material
                            .textures
                            .insert(path.clone(), MaterialTextureGpuResidency::Requested);
                        let priority = self
                            .gpu
                            .material
                            .texture_priorities
                            .get(&path)
                            .copied()
                            .unwrap_or_else(MaterialTexturePriority::secondary);
                        self.enqueue_material_texture_path_with_priority(path, priority);
                    }
                }
                Err(e) => {
                    let message = format!("assets.textures.entry_runtime_v1 failed err='{e}'");
                    let line = format!(
                        "render controller: material texture packet lookup failed path='{}' method='assets.textures.entry_runtime_v1' kind='{}' err='{}'",
                        path, e.kind, e,
                    );
                    match e.kind {
                        AssetErrorKind::DecodeFailed | AssetErrorKind::UnsupportedFormat => {
                            newengine_ulog_api::ulog::debug!("{}", line)
                        }
                        _ => newengine_ulog_api::ulog::warn!("{}", line),
                    }
                    self.gpu
                        .material
                        .textures
                        .insert(path, MaterialTextureGpuResidency::Failed { message });
                }
            }
        }
    }
}
