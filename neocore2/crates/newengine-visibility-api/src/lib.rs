#![forbid(unsafe_op_in_unsafe_fn)]

//! Stable provider-neutral DTO contract for the `engine.visibility` gateway.
//!
//! The engine owns candidate selection, history, hysteresis, significance and
//! downstream policy. A render backend may provide delayed occlusion knowledge,
//! but the DTO boundary never exposes Vulkan/D3D/Metal handles or query objects.

use serde::{Deserialize, Serialize};

pub mod binary;
pub use binary::{
    decode_visibility_query_batch_bin, decode_visibility_result_batch_bin,
    encode_visibility_query_batch_bin, encode_visibility_result_batch_bin,
};

pub const ENGINE_VISIBILITY_SERVICE_ID: &str = "engine.visibility";
pub const VISIBILITY_SERVICE_ID: &str = "visibility.api";
pub const VISIBILITY_BACKEND_CAPABILITY_ID: &str = "visibility.backend";
pub const VISIBILITY_RUNTIME_CONTRACT: &str = "newengine.visibility-api/v1";

pub mod visibility_method {
    pub const INFO_JSON: &str = newengine_service_api::SERVICE_METHOD_INFO_JSON;
    pub const INVOKE_JSON: &str = newengine_service_api::SERVICE_METHOD_INVOKE_JSON;
    pub const SHUTDOWN_V1: &str = newengine_service_api::SERVICE_METHOD_SHUTDOWN_V1;
    pub const QUERY_BATCH_JSON_V1: &str = "visibility.query_batch_json_v1";
    pub const QUERY_BATCH_BIN_V1: &str = "visibility.query_batch_bin_v1";
}

pub const VISIBILITY_SERVICE_METHODS: &[&str] = &[
    visibility_method::INFO_JSON,
    visibility_method::INVOKE_JSON,
    visibility_method::SHUTDOWN_V1,
    visibility_method::QUERY_BATCH_JSON_V1,
    visibility_method::QUERY_BATCH_BIN_V1,
];

pub const VISIBILITY_BACKEND_SERVICE_SPEC: newengine_service_api::BackendServiceSpec =
    newengine_service_api::BackendServiceSpec::new(
        "visibility",
        ENGINE_VISIBILITY_SERVICE_ID,
        VISIBILITY_SERVICE_ID,
        VISIBILITY_BACKEND_CAPABILITY_ID,
    );

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct VisibilityVec3V1 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct VisibilitySphereV1 {
    pub center: VisibilityVec3V1,
    pub radius: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct VisibilityViewV1 {
    pub position: VisibilityVec3V1,
    pub forward: VisibilityVec3V1,
    pub max_distance: f32,
    /// Wide coarse cone used only to avoid submitting obviously irrelevant
    /// candidates. A backend remains free to use an exact frustum/Hi-Z pyramid.
    pub coarse_cone_dot: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct VisibilityQueryCandidateV1 {
    pub subject_id: u64,
    pub bounds: VisibilitySphereV1,
    /// Higher values are serviced first when the query budget is saturated.
    pub priority: i32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VisibilityQueryBatchV1 {
    pub frame: u64,
    pub view: VisibilityViewV1,
    #[serde(default)]
    pub candidates: Vec<VisibilityQueryCandidateV1>,
    pub max_results: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VisibilityObservationV1 {
    Visible,
    Occluded,
    #[default]
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct VisibilitySubjectResultV1 {
    pub subject_id: u64,
    pub observation: VisibilityObservationV1,
    /// Provider confidence in the range [0, 1]. Values outside the range are
    /// sanitized by the control plane rather than becoming ABI validation gates.
    pub confidence: f32,
    /// Frame for which the provider produced this result. Delayed results are
    /// expected and intentionally part of the contract.
    pub produced_frame: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct VisibilityResultBatchV1 {
    pub provider_frame: u64,
    #[serde(default)]
    pub results: Vec<VisibilitySubjectResultV1>,
    #[serde(default)]
    pub diagnostics: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VisibilityServiceInfoV1 {
    pub protocol: String,
    pub provider: String,
    #[serde(default)]
    pub methods: Vec<String>,
    #[serde(default)]
    pub features: Vec<String>,
}

impl Default for VisibilityServiceInfoV1 {
    fn default() -> Self {
        Self {
            protocol: VISIBILITY_RUNTIME_CONTRACT.to_owned(),
            provider: "unbound".to_owned(),
            methods: VISIBILITY_SERVICE_METHODS
                .iter()
                .map(|method| (*method).to_owned())
                .collect(),
            features: vec![
                "delayed-results".to_owned(),
                "bounded-batches".to_owned(),
                "provider-neutral-bounds".to_owned(),
            ],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visibility_contract_exposes_backend_without_renderer_handles() {
        let info = VisibilityServiceInfoV1::default();
        assert_eq!(ENGINE_VISIBILITY_SERVICE_ID, "engine.visibility");
        assert_eq!(VISIBILITY_BACKEND_CAPABILITY_ID, "visibility.backend");
        assert!(info
            .features
            .iter()
            .any(|feature| feature == "delayed-results"));
    }

    #[test]
    fn result_batch_roundtrips_delayed_observation() {
        let batch = VisibilityResultBatchV1 {
            provider_frame: 14,
            results: vec![VisibilitySubjectResultV1 {
                subject_id: 7,
                observation: VisibilityObservationV1::Occluded,
                confidence: 0.91,
                produced_frame: 12,
            }],
            diagnostics: vec!["buffered two frames".to_owned()],
        };
        let json = serde_json::to_string(&batch).expect("serialize");
        let decoded: VisibilityResultBatchV1 = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(decoded, batch);
    }
}
