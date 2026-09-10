use super::*;

#[test]
fn texture_decode_preserves_semantic_scheduler_priority() {
    for (priority, expected) in [
        (
            MaterialTexturePriority::secondary(),
            TaskPriority::Background,
        ),
        (
            MaterialTexturePriority::streaming_visible(),
            TaskPriority::Interactive,
        ),
        (
            MaterialTexturePriority::launch_world(),
            TaskPriority::Critical,
        ),
    ] {
        let request =
            material_texture_decode_request("textures/characters/abby.ytd@m00_base", 42, priority);
        assert_eq!(request.lane, TaskLane::AssetIo);
        assert_eq!(request.priority, expected);
        assert_eq!(request.frame_id, Some(42));
        assert_eq!(request.task_domain, task_domain::ENGINE_ASSETS);
        assert_eq!(request.task_pass, task_pass::TEXTURE_DECODE);
        assert!(request
            .dependency_group
            .as_deref()
            .is_some_and(|group| group == "frame.42.asset-io.texture-decode"));
    }
}

#[test]
fn launch_priority_outranks_streaming_and_secondary() {
    use super::super::super::state::{MaterialTexturePriority, MaterialTextureQueueEntry};
    let frame = 100;
    let launch = MaterialTextureQueueEntry {
        priority: MaterialTexturePriority::launch_world(),
        enqueued_frame: 100,
        last_touched_frame: 100,
    };
    let streaming = MaterialTextureQueueEntry {
        priority: MaterialTexturePriority::streaming_visible(),
        enqueued_frame: 1,
        last_touched_frame: 100,
    };
    let secondary = MaterialTextureQueueEntry {
        priority: MaterialTexturePriority::secondary(),
        enqueued_frame: 1,
        last_touched_frame: 1,
    };
    assert!(
        RuntimeRenderController::material_texture_queue_rank(&launch, frame)
            > RuntimeRenderController::material_texture_queue_rank(&streaming, frame)
    );
    assert!(
        RuntimeRenderController::material_texture_queue_rank(&streaming, frame)
            > RuntimeRenderController::material_texture_queue_rank(&secondary, frame)
    );
}

#[test]
fn priority_merge_is_monotonic() {
    use super::super::super::state::MaterialTexturePriority;
    let merged = RuntimeRenderController::merge_material_texture_priority(
        MaterialTexturePriority::secondary(),
        MaterialTexturePriority::launch_player_weapon(),
    );
    assert_eq!(
        merged.class,
        super::super::super::state::MaterialTextureStreamingClass::LaunchCritical
    );
    assert!(merged.visible_now);
    assert_eq!(merged.player_weapon_relevance, u8::MAX);
}

#[test]
fn view_hint_score_prefers_large_near_visible_surface() {
    let near = streaming_priority_from_hints(
        MaterialTextureStreamingClass::StreamingCritical,
        true,
        0.5,
        2.0,
        128,
        0,
        128,
    );
    let far = streaming_priority_from_hints(
        MaterialTextureStreamingClass::StreamingCritical,
        true,
        0.02,
        40.0,
        128,
        0,
        128,
    );
    assert!(near.screen_coverage_q > far.screen_coverage_q);
    assert!(near.proximity_q > far.proximity_q);
}

#[test]
fn stale_visibility_stops_using_geometry_boost() {
    use super::super::super::state::MaterialTextureQueueEntry;
    let priority = streaming_priority_from_hints(
        MaterialTextureStreamingClass::StreamingCritical,
        true,
        1.0,
        1.0,
        100,
        0,
        100,
    );
    let entry = MaterialTextureQueueEntry {
        priority,
        enqueued_frame: 1,
        last_touched_frame: 1,
    };
    let recent = RuntimeRenderController::material_texture_queue_rank(&entry, 2);
    let stale = RuntimeRenderController::material_texture_queue_rank(&entry, 10);
    assert!(recent > stale);
}
