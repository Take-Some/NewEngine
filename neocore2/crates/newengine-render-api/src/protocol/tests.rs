use super::*;
use crate::{
    BufferId, DrawIndexedIndirectArgs, DrawIndexedIndirectCountArgs, Extent2D,
    FrameCameraContext, GpuVisibilityIndirectCullArgs, RenderDrawListKind,
    RenderFrameEnvelope, RenderGraphDesc, RenderGraphPassKind, TextureDesc, TextureId, UiDrawList,
    UiLayerDomain, UiLayerDrawPacket, UiLayerDrawPacketSet,
};
use std::num::NonZeroU32;

#[test]
fn binary_multi_adapter_mesh_packet_roundtrips() {
    let mut vertices = Vec::new();
    for value in [
        1.0_f32, 2.0, 3.0, 0.0, 2.0, 0.0, 0.25, 0.75, 4.0, 5.0, 6.0, 0.0, 0.0, 4.0, 0.5, 1.0,
    ] {
        vertices.extend_from_slice(&value.to_le_bytes());
    }
    let request = MultiAdapterMeshTranscodeRequest::new(vertices.clone()).unwrap();
    let request = decode_multi_adapter_mesh_transcode_request(
        &encode_multi_adapter_mesh_transcode_request(&request).unwrap(),
    )
    .unwrap();
    assert_eq!(request.vertex_count(), 2);
    assert_eq!(request.vertex_bytes, vertices);

    let response = MultiAdapterMeshTranscodeResult {
        worker_index: 1,
        invalid_vertex_count: 2,
        gpu_elapsed_ns: 42_000,
        vertex_bytes: vertices,
    };
    let response = decode_multi_adapter_mesh_transcode_result(
        &encode_multi_adapter_mesh_transcode_result(&response).unwrap(),
    )
    .unwrap();
    assert_eq!(response.worker_index, 1);
    assert_eq!(response.invalid_vertex_count, 2);
    assert_eq!(response.gpu_elapsed_ns, 42_000);
    assert_eq!(response.vertex_count(), 2);
}

#[test]
fn binary_create_texture_roundtrips_payload_and_response() {
    let desc = TextureDesc::new(
        Extent2D::new(8, 4),
        crate::TextureFormat::Bc3RgbaSrgb,
        crate::TextureUsage::Sampled,
    )
    .with_label("binary-texture")
    .with_mips(NonZeroU32::new(2).unwrap())
    .with_deferred_mip_data(
        vec![
            crate::TextureMipDataDesc::new(0, 8, 4, 0, 32),
            crate::TextureMipDataDesc::new(1, 4, 2, 32, 16),
        ],
        (0_u8..48).collect(),
    );
    let encoded = encode_create_texture_bin(&desc).unwrap();
    let decoded = decode_create_texture_bin(&encoded).unwrap();
    assert_eq!(decoded.label.as_deref(), Some("binary-texture"));
    assert_eq!(decoded.extent.width, 8);
    assert_eq!(decoded.extent.height, 4);
    assert_eq!(decoded.format, crate::TextureFormat::Bc3RgbaSrgb);
    assert_eq!(decoded.usage, crate::TextureUsage::Sampled);
    assert_eq!(decoded.mip_levels.get(), 2);
    assert_eq!(decoded.data_policy, crate::TextureDataPolicy::Deferred);
    assert_eq!(decoded.mip_data.len(), 2);
    assert_eq!(decoded.data.as_ref().map(Vec::len), Some(48));

    let id = TextureId::new(77);
    assert_eq!(
        decode_texture_id_bin(&encode_texture_id_bin(id)).unwrap(),
        id
    );
}

#[test]
fn render_protocol_v2_rejects_v1_binary_command_batches() {
    assert_eq!(RenderApiVersion::default(), RenderApiVersion::new(2, 0, 0));
    let encoded = encode_unit_command_batch_bin(&[RenderCommand::DiscardRecordedCommands]).unwrap();
    assert_eq!(&encoded[..8], b"NECB\x02\0\0\0");

    let mut legacy = encoded.clone();
    legacy[4] = 1;
    let error =
        decode_unit_command_batch_bin(&legacy).expect_err("v1 binary batch must be rejected");
    assert!(error.contains("invalid magic"));
}

#[test]
fn abort_frame_service_request_roundtrips() {
    let encoded = serde_json::to_vec(&RenderServiceRequest::AbortFrame)
        .expect("serialize abort-frame request");
    let decoded: RenderServiceRequest =
        serde_json::from_slice(&encoded).expect("deserialize abort-frame request");
    assert!(matches!(decoded, RenderServiceRequest::AbortFrame));
}

#[test]
fn binary_unit_batch_roundtrips_recording_scope_commands() {
    let encoded = encode_unit_command_batch_bin(&[
        RenderCommand::SetDrawListKind {
            kind: Some(RenderDrawListKind::OpaqueForward),
        },
        RenderCommand::SetRenderPhase {
            phase: Some(RenderGraphPassKind::UiComposite),
        },
        RenderCommand::SetDrawListKind { kind: None },
        RenderCommand::DiscardRecordedCommands,
    ])
    .unwrap();
    let decoded = decode_unit_command_batch_bin(&encoded).unwrap();

    assert!(matches!(
        decoded[0],
        RenderCommand::SetDrawListKind {
            kind: Some(RenderDrawListKind::OpaqueForward)
        }
    ));
    assert!(matches!(
        decoded[1],
        RenderCommand::SetRenderPhase {
            phase: Some(RenderGraphPassKind::UiComposite)
        }
    ));
    assert!(matches!(
        decoded[2],
        RenderCommand::SetDrawListKind { kind: None }
    ));
    assert!(matches!(decoded[3], RenderCommand::DiscardRecordedCommands));
}

#[test]
fn binary_unit_batch_roundtrips_indexed_indirect_commands() {
    let commands = BufferId::new(41);
    let count = BufferId::new(42);
    let encoded = encode_unit_command_batch_bin(&[
        RenderCommand::DrawIndexedIndirect(DrawIndexedIndirectArgs {
            buffer: commands,
            offset: 64,
            draw_count: 17,
            stride: 32,
        }),
        RenderCommand::DrawIndexedIndirectCount(DrawIndexedIndirectCountArgs {
            buffer: commands,
            offset: 128,
            count_buffer: count,
            count_offset: 16,
            max_draw_count: 4096,
            stride: DrawIndexedIndirectCountArgs::COMMAND_SIZE,
        }),
    ])
    .unwrap();
    let decoded = decode_unit_command_batch_bin(&encoded).unwrap();

    match decoded[0] {
        RenderCommand::DrawIndexedIndirect(args) => {
            assert_eq!(args.buffer, commands);
            assert_eq!(args.offset, 64);
            assert_eq!(args.draw_count, 17);
            assert_eq!(args.stride, 32);
        }
        ref other => panic!("unexpected first command: {other:?}"),
    }
    match decoded[1] {
        RenderCommand::DrawIndexedIndirectCount(args) => {
            assert_eq!(args.buffer, commands);
            assert_eq!(args.count_buffer, count);
            assert_eq!(args.max_draw_count, 4096);
            assert_eq!(args.stride, 20);
        }
        ref other => panic!("unexpected second command: {other:?}"),
    }
}

#[test]
fn binary_unit_batch_roundtrips_gpu_visibility_indirect_cull() {
    let args = GpuVisibilityIndirectCullArgs::new(
        BufferId::new(51),
        256,
        BufferId::new(52),
        512,
        91,
        [1920, 1080],
        FrameCameraContext {
            position_ws: [1.0, 2.0, 3.0],
            forward_ws: [0.0, 0.0, -1.0],
            up_ws: [0.0, 1.0, 0.0],
            fov_y: 1.1,
            near: 0.05,
            far: 1500.0,
        },
    );
    let encoded = encode_unit_command_batch_bin(&[
        RenderCommand::DispatchVisibilityIndirectCull(args),
    ])
    .unwrap();
    let decoded = decode_unit_command_batch_bin(&encoded).unwrap();
    match decoded[0] {
        RenderCommand::DispatchVisibilityIndirectCull(decoded) => assert_eq!(decoded, args),
        ref other => panic!("unexpected command: {other:?}"),
    }
}

#[test]
fn frame_envelope_roundtrips_ordered_ui_layer_packets() {
    let mut packets = UiLayerDrawPacketSet::new(41);
    let mut debug = UiDrawList::new();
    debug.screen_size_px = [1920, 1080];
    let mut game = UiDrawList::new();
    game.screen_size_px = [1920, 1080];
    packets.push(
        UiLayerDrawPacket::new(UiLayerDomain::Debug, 41, debug)
            .with_target("engine.render.surface.primary")
            .with_surfaces(["runtime.debug_overlay".to_owned()]),
    );
    packets.push(
        UiLayerDrawPacket::new(UiLayerDomain::GameViewport, 41, game)
            .with_target("engine.render.viewport.primary")
            .with_surfaces(["game.hud".to_owned()])
            .with_invalidation_revision(9),
    );

    let frame = RenderFrameEnvelope::new(
        41,
        [0.0, 0.0, 0.0, 1.0],
        Extent2D::new(1920, 1080),
        Extent2D::new(1920, 1080),
        true,
        RenderGraphDesc::new("layered-ui"),
    )
    .with_ui_layers(packets);
    let encoded = serde_json::to_vec(&frame).expect("serialize frame envelope");
    let decoded: RenderFrameEnvelope =
        serde_json::from_slice(&encoded).expect("deserialize frame envelope");

    assert_eq!(decoded.ui_layers.frame_index, 41);
    assert_eq!(decoded.ui_layers.packets.len(), 2);
    assert_eq!(
        decoded.ui_layers.packets[0].domain,
        UiLayerDomain::GameViewport
    );
    assert_eq!(decoded.ui_layers.packets[0].surface_ids, vec!["game.hud"]);
    assert_eq!(decoded.ui_layers.packets[0].invalidation_revision, 9);
    assert_eq!(decoded.ui_layers.packets[1].domain, UiLayerDomain::Debug);
    assert_eq!(
        decoded.ui_layers.packets[1].surface_ids,
        vec!["runtime.debug_overlay"]
    );
}
