use super::super::RenderCommand;
use super::codec::*;
use crate::{
    BindGroupId, BufferId, BufferSlice, DispatchArgs, DrawArgs, DrawIndexedArgs,
    DrawIndexedIndirectArgs, DrawIndexedIndirectCountArgs, FrameCameraContext,
    GpuVisibilityIndirectCullArgs, PipelineId, RectI32, Viewport,
};

const COMMAND_BATCH_BIN_MAGIC: &[u8; 8] = b"NECB\x02\0\0\0";

/// Encodes frame-local unit render commands into a compact binary packet.
///
/// JSON remains the service control protocol. This packet is only for the
/// hot path commands that return `Unit`; commands that allocate ids or query
/// snapshots intentionally stay on the typed JSON request/response surface.
pub fn encode_unit_command_batch_bin(commands: &[RenderCommand]) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(16 + commands.len().saturating_mul(32));
    out.extend_from_slice(COMMAND_BATCH_BIN_MAGIC);
    let command_count = u32::try_from(commands.len())
        .map_err(|_| "render command binary batch contains too many commands".to_owned())?;
    put_u32(&mut out, command_count);
    for command in commands {
        encode_unit_command(&mut out, command)?;
    }
    Ok(out)
}

pub fn decode_unit_command_batch_bin(bytes: &[u8]) -> Result<Vec<RenderCommand>, String> {
    let mut r = BinReader::new(bytes);
    let magic = r.take(8)?;
    if magic != COMMAND_BATCH_BIN_MAGIC {
        return Err("render command batch binary packet has invalid magic".to_owned());
    }
    let count = r.u32()? as usize;
    let mut commands = Vec::with_capacity(count);
    for _ in 0..count {
        commands.push(decode_unit_command(&mut r)?);
    }
    if !r.is_eof() {
        return Err("render command batch binary packet has trailing bytes".to_owned());
    }
    Ok(commands)
}

fn encode_unit_command(out: &mut Vec<u8>, command: &RenderCommand) -> Result<(), String> {
    match command {
        RenderCommand::WriteBuffer { id, offset, data } => {
            put_u8(out, 1);
            put_u32(out, id.get());
            put_u64(out, *offset);
            let len = u32::try_from(data.len()).map_err(|_| {
                "render command binary write_buffer payload is too large".to_owned()
            })?;
            put_u32(out, len);
            out.extend_from_slice(data);
        }
        RenderCommand::SetViewport(vp) => {
            put_u8(out, 2);
            put_f32(out, vp.x);
            put_f32(out, vp.y);
            put_f32(out, vp.w);
            put_f32(out, vp.h);
            put_f32(out, vp.min_depth);
            put_f32(out, vp.max_depth);
        }
        RenderCommand::SetScissor(rect) => {
            put_u8(out, 3);
            put_i32(out, rect.x);
            put_i32(out, rect.y);
            put_i32(out, rect.w);
            put_i32(out, rect.h);
        }
        RenderCommand::SetPipeline { pipeline } => {
            put_u8(out, 4);
            put_u32(out, pipeline.get());
        }
        RenderCommand::SetBindGroup { index, group } => {
            put_u8(out, 5);
            put_u32(out, *index);
            put_u32(out, group.get());
        }
        RenderCommand::SetVertexBuffer { slot, slice } => {
            put_u8(out, 6);
            put_u32(out, *slot);
            put_u32(out, slice.buffer.get());
            put_u64(out, slice.offset);
        }
        RenderCommand::SetIndexBuffer { slice, format } => {
            put_u8(out, 7);
            put_u32(out, slice.buffer.get());
            put_u64(out, slice.offset);
            put_index_format(out, *format);
        }
        RenderCommand::Draw(args) => {
            put_u8(out, 8);
            put_u32(out, args.vertex_count);
            put_u32(out, args.instance_count);
            put_u32(out, args.first_vertex);
            put_u32(out, args.first_instance);
        }
        RenderCommand::DrawIndexed(args) => {
            put_u8(out, 9);
            put_u32(out, args.index_count);
            put_u32(out, args.instance_count);
            put_u32(out, args.first_index);
            put_i32(out, args.vertex_offset);
            put_u32(out, args.first_instance);
        }
        RenderCommand::SetRenderPhase { phase } => {
            put_u8(out, 10);
            put_optional_render_graph_pass_kind(out, *phase);
        }
        RenderCommand::SetDrawListKind { kind } => {
            put_u8(out, 11);
            put_optional_render_draw_list_kind(out, *kind);
        }
        RenderCommand::DiscardRecordedCommands => {
            put_u8(out, 12);
        }
        RenderCommand::Dispatch(args) => {
            put_u8(out, 13);
            put_u32(out, args.groups_x);
            put_u32(out, args.groups_y);
            put_u32(out, args.groups_z);
        }
        RenderCommand::DrawIndexedIndirect(args) => {
            put_u8(out, 14);
            put_u32(out, args.buffer.get());
            put_u64(out, args.offset);
            put_u32(out, args.draw_count);
            put_u32(out, args.stride);
        }
        RenderCommand::DrawIndexedIndirectCount(args) => {
            put_u8(out, 15);
            put_u32(out, args.buffer.get());
            put_u64(out, args.offset);
            put_u32(out, args.count_buffer.get());
            put_u64(out, args.count_offset);
            put_u32(out, args.max_draw_count);
            put_u32(out, args.stride);
        }
        RenderCommand::DispatchVisibilityIndirectCull(args) => {
            put_u8(out, 16);
            put_u32(out, args.candidate_buffer.get());
            put_u64(out, args.candidate_offset);
            put_u32(out, args.indirect_buffer.get());
            put_u64(out, args.indirect_offset);
            put_u32(out, args.candidate_count);
            put_u32(out, args.candidate_stride);
            put_u32(out, args.command_stride);
            put_u32(out, args.viewport_extent[0]);
            put_u32(out, args.viewport_extent[1]);
            for value in args.camera.position_ws { put_f32(out, value); }
            for value in args.camera.forward_ws { put_f32(out, value); }
            for value in args.camera.up_ws { put_f32(out, value); }
            put_f32(out, args.camera.fov_y);
            put_f32(out, args.camera.near);
            put_f32(out, args.camera.far);
        }
        _ => {
            return Err(format!(
                "render command is not supported by binary unit batch: {command:?}"
            ))
        }
    }
    Ok(())
}

fn decode_unit_command(r: &mut BinReader<'_>) -> Result<RenderCommand, String> {
    match r.u8()? {
        1 => {
            let id = BufferId::new(r.u32()?);
            let offset = r.u64()?;
            let len = r.u32()? as usize;
            let data = r.take(len)?.to_vec();
            Ok(RenderCommand::WriteBuffer { id, offset, data })
        }
        2 => Ok(RenderCommand::SetViewport(Viewport {
            x: r.f32()?,
            y: r.f32()?,
            w: r.f32()?,
            h: r.f32()?,
            min_depth: r.f32()?,
            max_depth: r.f32()?,
        })),
        3 => Ok(RenderCommand::SetScissor(RectI32 {
            x: r.i32()?,
            y: r.i32()?,
            w: r.i32()?,
            h: r.i32()?,
        })),
        4 => Ok(RenderCommand::SetPipeline {
            pipeline: PipelineId::new(r.u32()?),
        }),
        5 => Ok(RenderCommand::SetBindGroup {
            index: r.u32()?,
            group: BindGroupId::new(r.u32()?),
        }),
        6 => Ok(RenderCommand::SetVertexBuffer {
            slot: r.u32()?,
            slice: BufferSlice::new(BufferId::new(r.u32()?), r.u64()?),
        }),
        7 => Ok(RenderCommand::SetIndexBuffer {
            slice: BufferSlice::new(BufferId::new(r.u32()?), r.u64()?),
            format: get_index_format(r.u8()?)?,
        }),
        8 => Ok(RenderCommand::Draw(DrawArgs {
            vertex_count: r.u32()?,
            instance_count: r.u32()?,
            first_vertex: r.u32()?,
            first_instance: r.u32()?,
        })),
        9 => Ok(RenderCommand::DrawIndexed(DrawIndexedArgs {
            index_count: r.u32()?,
            instance_count: r.u32()?,
            first_index: r.u32()?,
            vertex_offset: r.i32()?,
            first_instance: r.u32()?,
        })),
        10 => Ok(RenderCommand::SetRenderPhase {
            phase: r.optional_render_graph_pass_kind()?,
        }),
        11 => Ok(RenderCommand::SetDrawListKind {
            kind: r.optional_render_draw_list_kind()?,
        }),
        12 => Ok(RenderCommand::DiscardRecordedCommands),
        13 => Ok(RenderCommand::Dispatch(DispatchArgs {
            groups_x: r.u32()?,
            groups_y: r.u32()?,
            groups_z: r.u32()?,
        })),
        14 => Ok(RenderCommand::DrawIndexedIndirect(DrawIndexedIndirectArgs {
            buffer: BufferId::new(r.u32()?),
            offset: r.u64()?,
            draw_count: r.u32()?,
            stride: r.u32()?,
        })),
        15 => Ok(RenderCommand::DrawIndexedIndirectCount(
            DrawIndexedIndirectCountArgs {
                buffer: BufferId::new(r.u32()?),
                offset: r.u64()?,
                count_buffer: BufferId::new(r.u32()?),
                count_offset: r.u64()?,
                max_draw_count: r.u32()?,
                stride: r.u32()?,
            },
        )),
        16 => {
            let candidate_buffer = BufferId::new(r.u32()?);
            let candidate_offset = r.u64()?;
            let indirect_buffer = BufferId::new(r.u32()?);
            let indirect_offset = r.u64()?;
            let candidate_count = r.u32()?;
            let candidate_stride = r.u32()?;
            let command_stride = r.u32()?;
            let viewport_extent = [r.u32()?, r.u32()?];
            let camera = FrameCameraContext {
                position_ws: [r.f32()?, r.f32()?, r.f32()?],
                forward_ws: [r.f32()?, r.f32()?, r.f32()?],
                up_ws: [r.f32()?, r.f32()?, r.f32()?],
                fov_y: r.f32()?,
                near: r.f32()?,
                far: r.f32()?,
            };
            Ok(RenderCommand::DispatchVisibilityIndirectCull(
                GpuVisibilityIndirectCullArgs {
                    candidate_buffer,
                    candidate_offset,
                    indirect_buffer,
                    indirect_offset,
                    candidate_count,
                    candidate_stride,
                    command_stride,
                    viewport_extent,
                    camera,
                },
            ))
        }
        tag => Err(format!("unknown render command batch binary tag {tag}")),
    }
}
