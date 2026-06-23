#![no_main]

use devrelay_core::{
    ApplySnapshotParams, CheckpointCreateParams, DiagnosticsExportParams, EventsSubscribeParams,
    METHOD_AGENT_HEALTH, METHOD_APPLY_SNAPSHOT, METHOD_CHECKPOINT_CREATE,
    METHOD_DIAGNOSTICS_EXPORT, METHOD_EVENTS_SUBSCRIBE, METHOD_PROJECTS_ADD, METHOD_PROJECTS_LIST,
    METHOD_PROJECTS_REMOVE, METHOD_PROJECTS_SHOW, METHOD_RECOVER_LIST, METHOD_RECOVER_OPEN,
    METHOD_RECOVER_SHOW, METHOD_RPC_NEGOTIATE, METHOD_SNAPSHOTS_LIST, METHOD_STATUS_GET,
    ProjectsAddParams, ProjectsRemoveParams, ProjectsShowParams, RecoverListParams,
    RecoverOpenParams, RecoverShowParams, RpcRequest, RpcVersionNegotiationParams,
    SnapshotsListParams, StatusGetParams,
};
use libfuzzer_sys::fuzz_target;
use serde::de::DeserializeOwned;

fuzz_target!(|data: &[u8]| {
    if let Ok(request) = RpcRequest::parse(data) {
        let _ = request.required_id();
        match request.method.as_str() {
            METHOD_RPC_NEGOTIATE => parse_params::<RpcVersionNegotiationParams>(&request.params),
            METHOD_AGENT_HEALTH | METHOD_PROJECTS_LIST => {}
            METHOD_STATUS_GET => parse_params::<StatusGetParams>(&request.params),
            METHOD_PROJECTS_ADD => parse_params::<ProjectsAddParams>(&request.params),
            METHOD_PROJECTS_SHOW => parse_params::<ProjectsShowParams>(&request.params),
            METHOD_PROJECTS_REMOVE => parse_params::<ProjectsRemoveParams>(&request.params),
            METHOD_CHECKPOINT_CREATE => parse_params::<CheckpointCreateParams>(&request.params),
            METHOD_SNAPSHOTS_LIST => parse_params::<SnapshotsListParams>(&request.params),
            METHOD_APPLY_SNAPSHOT => parse_params::<ApplySnapshotParams>(&request.params),
            METHOD_RECOVER_LIST => parse_params::<RecoverListParams>(&request.params),
            METHOD_RECOVER_SHOW => parse_params::<RecoverShowParams>(&request.params),
            METHOD_RECOVER_OPEN => parse_params::<RecoverOpenParams>(&request.params),
            METHOD_DIAGNOSTICS_EXPORT => parse_params::<DiagnosticsExportParams>(&request.params),
            METHOD_EVENTS_SUBSCRIBE => parse_params::<EventsSubscribeParams>(&request.params),
            _ => {}
        }
    }
});

fn parse_params<T: DeserializeOwned>(value: &serde_json::Value) {
    let _ = serde_json::from_value::<T>(value.clone());
}
