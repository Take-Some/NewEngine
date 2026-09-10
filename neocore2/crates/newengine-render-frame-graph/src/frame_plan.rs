use newengine_render_api::{
    RenderGraphCompilation, RenderGraphCompileReport, RenderGraphDesc, RenderGraphValidationReport,
};
use serde::{Deserialize, Serialize};

use crate::{
    DrawListDesc, DrawListRouteValidationIssue, DrawListRouteValidationReport, RenderPhaseDesc,
    StandardRenderPhase,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FramePlanExecutionMode {
    NativeGraph,
    ImmediateCallbacks,
    ValidateOnly,
}

impl Default for FramePlanExecutionMode {
    #[inline]
    fn default() -> Self {
        Self::ImmediateCallbacks
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderFramePlan {
    pub graph: RenderGraphDesc,
    pub phases: Vec<RenderPhaseDesc>,
    #[serde(default)]
    pub draw_lists: Vec<DrawListDesc>,
    #[serde(default)]
    pub execution_mode: FramePlanExecutionMode,
}

impl RenderFramePlan {
    #[inline]
    pub fn new(graph: RenderGraphDesc) -> Self {
        Self {
            graph,
            phases: Vec::new(),
            draw_lists: Vec::new(),
            execution_mode: FramePlanExecutionMode::ImmediateCallbacks,
        }
    }

    #[inline]
    pub fn contains_phase(&self, phase: StandardRenderPhase) -> bool {
        self.phases.iter().any(|it| it.phase == phase)
    }

    #[inline]
    pub fn phase_order(&self) -> impl Iterator<Item = StandardRenderPhase> + '_ {
        self.phases.iter().map(|it| it.phase)
    }

    #[inline]
    pub fn validate(&self) -> RenderGraphValidationReport {
        newengine_render_api::validate_and_compile_render_graph(&self.graph)
    }

    #[inline]
    pub fn compile(
        &self,
    ) -> Result<RenderGraphCompileReport, Vec<newengine_render_api::RenderGraphValidationIssue>>
    {
        newengine_render_api::compile_render_graph(&self.graph)
    }

    /// Returns the V2 compiled DAG, Phase-2 culling diagnostics and Phase-3
    /// live resource lifetime intervals in addition to the legacy summary.
    #[inline]
    pub fn compile_v2(
        &self,
    ) -> Result<RenderGraphCompilation, Vec<newengine_render_api::RenderGraphValidationIssue>> {
        crate::compile_frame_graph_v2(&self.graph)
    }

    pub fn validate_draw_list_routes(&self) -> DrawListRouteValidationReport {
        let mut errors = Vec::new();
        let mut warnings = Vec::new();

        for list in &self.draw_lists {
            let routes = self
                .graph
                .passes
                .iter()
                .filter(|pass| pass.draw_lists.contains(&list.kind))
                .map(|pass| pass.kind)
                .collect::<Vec<_>>();
            let route_count = routes.len();
            let intentional_deferred_opaque_split = list.kind
                == newengine_render_api::RenderDrawListKind::OpaqueForward
                && route_count == 2
                && routes.contains(&newengine_render_api::RenderGraphPassKind::GBuffer)
                && routes.contains(&newengine_render_api::RenderGraphPassKind::ForwardOpaque);

            if route_count == 0 {
                errors.push(
                    DrawListRouteValidationIssue::new(
                        "draw_list.unrouted",
                        format!(
                            "draw-list '{}' is declared in the frame plan but no render graph pass consumes it",
                            list.kind.label()
                        ),
                    )
                    .with_draw_list(list.kind),
                );
            } else if route_count > 1 && !intentional_deferred_opaque_split {
                warnings.push(
                    DrawListRouteValidationIssue::new(
                        "draw_list.multiple_routes",
                        format!(
                            "draw-list '{}' is consumed by {} render graph passes; ensure this is intentional",
                            list.kind.label(),
                            route_count
                        ),
                    )
                    .with_draw_list(list.kind),
                );
            }
        }

        for pass in &self.graph.passes {
            for routed_draw_list in &pass.draw_lists {
                if !self
                    .draw_lists
                    .iter()
                    .any(|list| list.kind == *routed_draw_list)
                {
                    warnings.push(
                        DrawListRouteValidationIssue::new(
                            "draw_list.route_without_declared_list",
                            format!(
                                "render graph pass '{}' consumes draw-list '{}' but the frame plan did not declare it",
                                pass.label,
                                routed_draw_list.label()
                            ),
                        )
                        .with_draw_list(*routed_draw_list),
                    );
                }
            }
        }

        DrawListRouteValidationReport {
            ok: errors.is_empty(),
            errors,
            warnings,
        }
    }
}
