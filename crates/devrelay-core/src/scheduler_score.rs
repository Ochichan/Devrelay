//! Scheduler candidate scoring.
//!
//! Constraint filtering determines whether a device can run a task. Scoring is
//! the next deterministic layer: normalize runtime signals into explainable
//! 0..1000 component scores and combine them with task-class weights.

use crate::{
    ForegroundLoad, ResourcePowerSource, SchedulerConstraintDecision, SchedulerDeviceSnapshot,
    SchedulerNetworkRouteQuality, TaskCacheMode, TaskDefinition, evaluate_scheduler_constraints,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

const DEFAULT_UNKNOWN_SCORE: u16 = 600;
const TRANSFER_COST_FULL_PENALTY_MIB: u64 = 8192;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum SchedulerTaskClass {
    Interactive,
    Test,
    Build,
    Batch,
    Background,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum SchedulerThermalPressure {
    #[default]
    Unknown,
    Nominal,
    Fair,
    Serious,
    Critical,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct SchedulerScoreMeasurements {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_warmth_per_mille: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_locality_per_mille: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network_route_quality: Option<SchedulerNetworkRouteQuality>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub historical_speed_per_mille: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_affinity_per_mille: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transfer_cost_mib: Option<u64>,
    #[serde(default)]
    pub thermal_pressure: SchedulerThermalPressure,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct SchedulerScoreWeights {
    pub cache_warmth: u16,
    pub idle_cpu: u16,
    pub free_memory: u16,
    pub power_preference: u16,
    pub data_locality: u16,
    pub network_quality: u16,
    pub historical_speed: u16,
    pub user_affinity: u16,
    pub transfer_cost: u16,
    pub foreground: u16,
    pub thermal: u16,
}

impl SchedulerScoreWeights {
    pub fn for_task_class(task_class: SchedulerTaskClass) -> Self {
        match task_class {
            SchedulerTaskClass::Interactive => Self {
                cache_warmth: 80,
                idle_cpu: 160,
                free_memory: 110,
                power_preference: 70,
                data_locality: 80,
                network_quality: 120,
                historical_speed: 80,
                user_affinity: 180,
                transfer_cost: 80,
                foreground: 170,
                thermal: 70,
            },
            SchedulerTaskClass::Test => Self {
                cache_warmth: 170,
                idle_cpu: 160,
                free_memory: 130,
                power_preference: 50,
                data_locality: 120,
                network_quality: 80,
                historical_speed: 130,
                user_affinity: 70,
                transfer_cost: 100,
                foreground: 90,
                thermal: 60,
            },
            SchedulerTaskClass::Build => Self {
                cache_warmth: 150,
                idle_cpu: 150,
                free_memory: 150,
                power_preference: 50,
                data_locality: 140,
                network_quality: 80,
                historical_speed: 140,
                user_affinity: 50,
                transfer_cost: 120,
                foreground: 80,
                thermal: 80,
            },
            SchedulerTaskClass::Batch => Self {
                cache_warmth: 100,
                idle_cpu: 130,
                free_memory: 120,
                power_preference: 90,
                data_locality: 120,
                network_quality: 90,
                historical_speed: 120,
                user_affinity: 60,
                transfer_cost: 140,
                foreground: 80,
                thermal: 90,
            },
            SchedulerTaskClass::Background => Self {
                cache_warmth: 90,
                idle_cpu: 100,
                free_memory: 100,
                power_preference: 170,
                data_locality: 120,
                network_quality: 80,
                historical_speed: 80,
                user_affinity: 50,
                transfer_cost: 130,
                foreground: 140,
                thermal: 130,
            },
        }
    }

    pub fn total(self) -> u64 {
        u64::from(self.cache_warmth)
            + u64::from(self.idle_cpu)
            + u64::from(self.free_memory)
            + u64::from(self.power_preference)
            + u64::from(self.data_locality)
            + u64::from(self.network_quality)
            + u64::from(self.historical_speed)
            + u64::from(self.user_affinity)
            + u64::from(self.transfer_cost)
            + u64::from(self.foreground)
            + u64::from(self.thermal)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SchedulerScore {
    pub device_id: String,
    pub eligible: bool,
    pub total_score_per_mille: u16,
    pub task_class: SchedulerTaskClass,
    pub constraint_decision: SchedulerConstraintDecision,
    pub components: Vec<SchedulerScoreComponent>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SchedulerTargetSelection {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_device_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_score_per_mille: Option<u16>,
    pub scores: Vec<SchedulerScore>,
    pub explanation: Vec<String>,
}

pub fn scheduler_selection_reason(selection: &SchedulerTargetSelection) -> &'static str {
    if selection.selected_device_id.is_some() {
        "highest-eligible-score"
    } else if selection.scores.is_empty() {
        "no-candidates"
    } else {
        "no-eligible-target"
    }
}

pub fn scheduler_selection_metadata(selection: &SchedulerTargetSelection) -> serde_json::Value {
    let choice_reason = scheduler_selection_reason(selection);
    serde_json::json!({
        "scheduler_choice_reason": choice_reason,
        "scheduler": {
            "choice_reason": choice_reason,
            "selected_device_id": selection.selected_device_id.clone(),
            "selected_score_per_mille": selection.selected_score_per_mille,
            "explanation": selection.explanation.clone(),
        },
        "scheduler_selection": selection.clone(),
    })
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SchedulerScoreComponent {
    pub kind: SchedulerScoreComponentKind,
    pub score_per_mille: u16,
    pub weight: u16,
    pub weighted_points: u64,
    pub explanation: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum SchedulerScoreComponentKind {
    CacheWarmth,
    IdleCpu,
    FreeMemory,
    PowerPreference,
    DataLocality,
    NetworkQuality,
    HistoricalSpeed,
    UserAffinity,
    TransferCostPenalty,
    ForegroundPenalty,
    ThermalPenalty,
}

pub fn infer_scheduler_task_class(definition: &TaskDefinition) -> SchedulerTaskClass {
    if definition.interactive {
        return SchedulerTaskClass::Interactive;
    }

    let command = definition.command.join(" ").to_ascii_lowercase();
    let task_name = definition.task_name.to_ascii_lowercase();
    if task_name.contains("test")
        || command.contains(" test")
        || command.contains("pytest")
        || command.contains("cargo test")
    {
        return SchedulerTaskClass::Test;
    }
    if !definition.outputs.is_empty()
        || matches!(
            definition.cache,
            Some(TaskCacheMode::Write | TaskCacheMode::ReadWrite)
        )
        || task_name.contains("build")
        || command.contains(" build")
        || command.contains("cargo build")
    {
        return SchedulerTaskClass::Build;
    }

    SchedulerTaskClass::Batch
}

pub fn score_scheduler_candidate(
    definition: &TaskDefinition,
    device: &SchedulerDeviceSnapshot,
    measurements: &SchedulerScoreMeasurements,
) -> SchedulerScore {
    score_scheduler_candidate_with_class(
        definition,
        device,
        measurements,
        infer_scheduler_task_class(definition),
    )
}

pub fn select_scheduler_target(
    definition: &TaskDefinition,
    devices: &[SchedulerDeviceSnapshot],
    measurements_by_device: &BTreeMap<String, SchedulerScoreMeasurements>,
) -> SchedulerTargetSelection {
    let scores = devices
        .iter()
        .map(|device| {
            let measurements = measurements_by_device
                .get(&device.device_id)
                .cloned()
                .unwrap_or_default();
            score_scheduler_candidate(definition, device, &measurements)
        })
        .collect::<Vec<_>>();

    let mut eligible = scores
        .iter()
        .filter(|score| score.eligible)
        .collect::<Vec<_>>();
    eligible.sort_by(|left, right| {
        right
            .total_score_per_mille
            .cmp(&left.total_score_per_mille)
            .then_with(|| left.device_id.cmp(&right.device_id))
    });

    let selected = eligible.first().copied();
    let explanation = selected
        .map(selection_explanation)
        .unwrap_or_else(|| no_selection_explanation(&scores));

    SchedulerTargetSelection {
        selected_device_id: selected.map(|score| score.device_id.clone()),
        selected_score_per_mille: selected.map(|score| score.total_score_per_mille),
        scores,
        explanation,
    }
}

pub fn score_scheduler_candidate_with_class(
    definition: &TaskDefinition,
    device: &SchedulerDeviceSnapshot,
    measurements: &SchedulerScoreMeasurements,
    task_class: SchedulerTaskClass,
) -> SchedulerScore {
    let constraint_decision = evaluate_scheduler_constraints(definition, device);
    if !constraint_decision.eligible {
        return SchedulerScore {
            device_id: device.device_id.clone(),
            eligible: false,
            total_score_per_mille: 0,
            task_class,
            constraint_decision,
            components: Vec::new(),
        };
    }

    let weights = SchedulerScoreWeights::for_task_class(task_class);
    let components = scheduler_score_components(definition, device, measurements, weights);
    let total_weight = weights.total().max(1);
    let total_points = components
        .iter()
        .map(|component| component.weighted_points)
        .sum::<u64>();

    SchedulerScore {
        device_id: device.device_id.clone(),
        eligible: true,
        total_score_per_mille: (total_points / total_weight).min(1000) as u16,
        task_class,
        constraint_decision,
        components,
    }
}

pub fn scheduler_score_components(
    definition: &TaskDefinition,
    device: &SchedulerDeviceSnapshot,
    measurements: &SchedulerScoreMeasurements,
    weights: SchedulerScoreWeights,
) -> Vec<SchedulerScoreComponent> {
    vec![
        component(
            SchedulerScoreComponentKind::CacheWarmth,
            bounded_signal(measurements.cache_warmth_per_mille, DEFAULT_UNKNOWN_SCORE),
            weights.cache_warmth,
            "cache warmth signal",
        ),
        component(
            SchedulerScoreComponentKind::IdleCpu,
            idle_cpu_score(device),
            weights.idle_cpu,
            "available CPU headroom",
        ),
        component(
            SchedulerScoreComponentKind::FreeMemory,
            free_memory_score(definition, device),
            weights.free_memory,
            "free memory against task requirement",
        ),
        component(
            SchedulerScoreComponentKind::PowerPreference,
            power_preference_score(device),
            weights.power_preference,
            "AC power and low-power mode preference",
        ),
        component(
            SchedulerScoreComponentKind::DataLocality,
            bounded_signal(measurements.data_locality_per_mille, DEFAULT_UNKNOWN_SCORE),
            weights.data_locality,
            "snapshot and dependency locality signal",
        ),
        component(
            SchedulerScoreComponentKind::NetworkQuality,
            network_quality_score(
                measurements
                    .network_route_quality
                    .unwrap_or(device.dynamic.network_route_quality),
            ),
            weights.network_quality,
            "network route quality signal",
        ),
        component(
            SchedulerScoreComponentKind::HistoricalSpeed,
            bounded_signal(
                measurements.historical_speed_per_mille,
                DEFAULT_UNKNOWN_SCORE,
            ),
            weights.historical_speed,
            "historical task speed signal",
        ),
        component(
            SchedulerScoreComponentKind::UserAffinity,
            bounded_signal(measurements.user_affinity_per_mille, DEFAULT_UNKNOWN_SCORE),
            weights.user_affinity,
            "user affinity signal",
        ),
        component(
            SchedulerScoreComponentKind::TransferCostPenalty,
            transfer_cost_score(measurements.transfer_cost_mib),
            weights.transfer_cost,
            "lower estimated transfer cost scores higher",
        ),
        component(
            SchedulerScoreComponentKind::ForegroundPenalty,
            foreground_score(device.dynamic.foreground_load),
            weights.foreground,
            "busy foreground activity scores lower",
        ),
        component(
            SchedulerScoreComponentKind::ThermalPenalty,
            thermal_score(measurements.thermal_pressure),
            weights.thermal,
            "thermal pressure placeholder",
        ),
    ]
}

fn selection_explanation(score: &SchedulerScore) -> Vec<String> {
    let mut explanation = vec![format!(
        "selected {} with score {}/1000",
        score.device_id, score.total_score_per_mille
    )];
    explanation.extend(score.components.iter().take(3).map(|component| {
        format!(
            "{:?}: {} ({}/1000, weight {})",
            component.kind, component.explanation, component.score_per_mille, component.weight
        )
    }));
    explanation
}

fn no_selection_explanation(scores: &[SchedulerScore]) -> Vec<String> {
    if scores.is_empty() {
        return vec!["no scheduler candidates were available".to_string()];
    }
    let rejected = scores.iter().filter(|score| !score.eligible).count();
    vec![format!(
        "no eligible scheduler target; {rejected} candidate(s) rejected by constraints"
    )]
}

fn component(
    kind: SchedulerScoreComponentKind,
    score_per_mille: u16,
    weight: u16,
    explanation: &str,
) -> SchedulerScoreComponent {
    SchedulerScoreComponent {
        kind,
        score_per_mille,
        weight,
        weighted_points: u64::from(score_per_mille) * u64::from(weight),
        explanation: explanation.to_string(),
    }
}

fn bounded_signal(value: Option<u16>, fallback: u16) -> u16 {
    value.unwrap_or(fallback).min(1000)
}

fn idle_cpu_score(device: &SchedulerDeviceSnapshot) -> u16 {
    let Some(load) = device.dynamic.cpu_load_1m_milli else {
        return DEFAULT_UNKNOWN_SCORE;
    };
    let capacity = device.cpu_cores.max(1) * 1000;
    let busy = load.min(capacity);
    ((capacity - busy) * 1000 / capacity) as u16
}

fn free_memory_score(definition: &TaskDefinition, device: &SchedulerDeviceSnapshot) -> u16 {
    let Some(free_mib) = device.dynamic.memory_free_mib else {
        return DEFAULT_UNKNOWN_SCORE;
    };
    if let Some(required_mib) = definition.memory_mib
        && required_mib > 0
    {
        return ((free_mib.min(required_mib) * 1000) / required_mib) as u16;
    }
    if let Some(total_mib) = device.memory_total_mib
        && total_mib > 0
    {
        return ((free_mib.min(total_mib) * 1000) / total_mib) as u16;
    }
    DEFAULT_UNKNOWN_SCORE
}

fn power_preference_score(device: &SchedulerDeviceSnapshot) -> u16 {
    if device.dynamic.low_power_mode {
        return 250;
    }
    match device.dynamic.power_source {
        ResourcePowerSource::Ac => 1000,
        ResourcePowerSource::Battery => 350,
        ResourcePowerSource::Unknown => 700,
    }
}

fn network_quality_score(quality: SchedulerNetworkRouteQuality) -> u16 {
    match quality {
        SchedulerNetworkRouteQuality::Excellent => 1000,
        SchedulerNetworkRouteQuality::Good => 800,
        SchedulerNetworkRouteQuality::Fair => 550,
        SchedulerNetworkRouteQuality::Poor => 250,
        SchedulerNetworkRouteQuality::Unknown => DEFAULT_UNKNOWN_SCORE,
    }
}

fn transfer_cost_score(transfer_cost_mib: Option<u64>) -> u16 {
    let Some(transfer_cost_mib) = transfer_cost_mib else {
        return DEFAULT_UNKNOWN_SCORE;
    };
    let penalty = transfer_cost_mib.min(TRANSFER_COST_FULL_PENALTY_MIB) * 1000
        / TRANSFER_COST_FULL_PENALTY_MIB;
    (1000 - penalty) as u16
}

fn foreground_score(foreground_load: ForegroundLoad) -> u16 {
    match foreground_load {
        ForegroundLoad::Idle => 1000,
        ForegroundLoad::Busy => 250,
        ForegroundLoad::Unknown => DEFAULT_UNKNOWN_SCORE,
    }
}

fn thermal_score(thermal_pressure: SchedulerThermalPressure) -> u16 {
    match thermal_pressure {
        SchedulerThermalPressure::Nominal => 1000,
        SchedulerThermalPressure::Fair => 750,
        SchedulerThermalPressure::Serious => 300,
        SchedulerThermalPressure::Critical => 0,
        SchedulerThermalPressure::Unknown => DEFAULT_UNKNOWN_SCORE,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        EnvironmentKind, SchedulerDevicePolicy, SchedulerDynamicResources, TaskCacheMode,
        TaskSandbox,
    };

    #[test]
    fn scores_candidate_with_explainable_components() {
        let definition = task_definition("test", false);
        let device = scheduler_device("fast");
        let measurements = SchedulerScoreMeasurements {
            cache_warmth_per_mille: Some(900),
            data_locality_per_mille: Some(800),
            network_route_quality: Some(SchedulerNetworkRouteQuality::Excellent),
            historical_speed_per_mille: Some(850),
            user_affinity_per_mille: Some(700),
            transfer_cost_mib: Some(256),
            thermal_pressure: SchedulerThermalPressure::Nominal,
        };

        let score = score_scheduler_candidate(&definition, &device, &measurements);

        assert!(score.eligible);
        assert_eq!(score.task_class, SchedulerTaskClass::Test);
        assert!(score.total_score_per_mille > 800);
        assert_eq!(score.components.len(), 11);
        assert_eq!(
            score
                .components
                .iter()
                .map(|component| component.kind)
                .collect::<Vec<_>>(),
            vec![
                SchedulerScoreComponentKind::CacheWarmth,
                SchedulerScoreComponentKind::IdleCpu,
                SchedulerScoreComponentKind::FreeMemory,
                SchedulerScoreComponentKind::PowerPreference,
                SchedulerScoreComponentKind::DataLocality,
                SchedulerScoreComponentKind::NetworkQuality,
                SchedulerScoreComponentKind::HistoricalSpeed,
                SchedulerScoreComponentKind::UserAffinity,
                SchedulerScoreComponentKind::TransferCostPenalty,
                SchedulerScoreComponentKind::ForegroundPenalty,
                SchedulerScoreComponentKind::ThermalPenalty,
            ]
        );
        assert!(
            score
                .components
                .iter()
                .all(|component| !component.explanation.is_empty())
        );
    }

    #[test]
    fn score_prefers_idle_ac_local_warm_candidate() {
        let definition = task_definition("build", false);
        let fast = scheduler_device("fast");
        let mut slow = scheduler_device("slow");
        slow.dynamic.cpu_load_1m_milli = Some(7800);
        slow.dynamic.memory_free_mib = Some(2048);
        slow.dynamic.power_source = ResourcePowerSource::Battery;
        slow.dynamic.foreground_load = ForegroundLoad::Busy;

        let fast_score = score_scheduler_candidate(
            &definition,
            &fast,
            &SchedulerScoreMeasurements {
                cache_warmth_per_mille: Some(900),
                data_locality_per_mille: Some(900),
                network_route_quality: Some(SchedulerNetworkRouteQuality::Good),
                historical_speed_per_mille: Some(850),
                user_affinity_per_mille: Some(500),
                transfer_cost_mib: Some(128),
                thermal_pressure: SchedulerThermalPressure::Nominal,
            },
        );
        let slow_score = score_scheduler_candidate(
            &definition,
            &slow,
            &SchedulerScoreMeasurements {
                cache_warmth_per_mille: Some(200),
                data_locality_per_mille: Some(250),
                network_route_quality: Some(SchedulerNetworkRouteQuality::Poor),
                historical_speed_per_mille: Some(300),
                user_affinity_per_mille: Some(500),
                transfer_cost_mib: Some(8192),
                thermal_pressure: SchedulerThermalPressure::Serious,
            },
        );

        assert!(fast_score.total_score_per_mille > slow_score.total_score_per_mille);
        assert!(fast_score.total_score_per_mille > 800);
        assert!(slow_score.total_score_per_mille < 400);
    }

    #[test]
    fn selects_highest_eligible_candidate_with_explanation() {
        let definition = task_definition("build", false);
        let fast = scheduler_device("fast");
        let mut slow = scheduler_device("slow");
        slow.dynamic.cpu_load_1m_milli = Some(7800);
        slow.dynamic.memory_free_mib = Some(2048);
        slow.dynamic.power_source = ResourcePowerSource::Battery;
        slow.dynamic.foreground_load = ForegroundLoad::Busy;

        let selection = select_scheduler_target(
            &definition,
            &[slow.clone(), fast.clone()],
            &BTreeMap::from([
                (
                    "fast".to_string(),
                    SchedulerScoreMeasurements {
                        cache_warmth_per_mille: Some(900),
                        data_locality_per_mille: Some(900),
                        network_route_quality: Some(SchedulerNetworkRouteQuality::Good),
                        historical_speed_per_mille: Some(850),
                        user_affinity_per_mille: Some(500),
                        transfer_cost_mib: Some(128),
                        thermal_pressure: SchedulerThermalPressure::Nominal,
                    },
                ),
                (
                    "slow".to_string(),
                    SchedulerScoreMeasurements {
                        cache_warmth_per_mille: Some(200),
                        data_locality_per_mille: Some(250),
                        network_route_quality: Some(SchedulerNetworkRouteQuality::Poor),
                        historical_speed_per_mille: Some(300),
                        user_affinity_per_mille: Some(500),
                        transfer_cost_mib: Some(8192),
                        thermal_pressure: SchedulerThermalPressure::Serious,
                    },
                ),
            ]),
        );

        assert_eq!(selection.selected_device_id.as_deref(), Some("fast"));
        assert_eq!(selection.scores.len(), 2);
        assert!(
            selection
                .selected_score_per_mille
                .is_some_and(|score| score > 800)
        );
        assert!(selection.explanation[0].contains("selected fast"));
        assert!(
            selection
                .explanation
                .iter()
                .any(|line| line.contains("cache warmth"))
        );
        let metadata = scheduler_selection_metadata(&selection);
        assert_eq!(
            metadata["scheduler_choice_reason"],
            "highest-eligible-score"
        );
        assert_eq!(
            metadata["scheduler"]["choice_reason"],
            "highest-eligible-score"
        );
        assert_eq!(metadata["scheduler"]["selected_device_id"], "fast");
        assert_eq!(
            metadata["scheduler_selection"]["selected_device_id"],
            "fast"
        );
    }

    #[test]
    fn explains_when_no_scheduler_target_is_eligible() {
        let mut definition = task_definition("test", false);
        definition.platforms = vec!["linux-*".to_string()];
        let device = scheduler_device("darwin");

        let selection = select_scheduler_target(&definition, &[device], &BTreeMap::new());

        assert_eq!(selection.selected_device_id, None);
        assert_eq!(selection.selected_score_per_mille, None);
        assert_eq!(selection.scores.len(), 1);
        assert!(
            selection
                .explanation
                .iter()
                .any(|line| line.contains("no eligible scheduler target"))
        );
        assert_eq!(scheduler_selection_reason(&selection), "no-eligible-target");
    }

    #[test]
    fn explains_when_no_scheduler_candidates_exist() {
        let definition = task_definition("test", false);

        let selection = select_scheduler_target(&definition, &[], &BTreeMap::new());

        assert_eq!(selection.selected_device_id, None);
        assert_eq!(selection.scores.len(), 0);
        assert_eq!(scheduler_selection_reason(&selection), "no-candidates");
        assert_eq!(
            scheduler_selection_metadata(&selection)["scheduler"]["choice_reason"],
            "no-candidates"
        );
    }

    #[test]
    fn ineligible_candidates_score_zero_and_keep_constraint_explanation() {
        let mut definition = task_definition("test", false);
        definition.platforms = vec!["linux-*".to_string()];
        let device = scheduler_device("darwin");

        let score =
            score_scheduler_candidate(&definition, &device, &SchedulerScoreMeasurements::default());

        assert!(!score.eligible);
        assert_eq!(score.total_score_per_mille, 0);
        assert!(score.components.is_empty());
        assert_eq!(score.constraint_decision.rejections.len(), 1);
    }

    #[test]
    fn task_class_weights_change_priority_shape() {
        let interactive = SchedulerScoreWeights::for_task_class(SchedulerTaskClass::Interactive);
        let background = SchedulerScoreWeights::for_task_class(SchedulerTaskClass::Background);
        let build = SchedulerScoreWeights::for_task_class(SchedulerTaskClass::Build);

        assert!(interactive.user_affinity > background.user_affinity);
        assert!(interactive.foreground > background.foreground);
        assert!(background.power_preference > interactive.power_preference);
        assert!(build.data_locality > interactive.data_locality);
        assert!(build.total() > 0);
    }

    fn task_definition(task_name: &str, interactive: bool) -> TaskDefinition {
        TaskDefinition {
            project_id: "12345678".to_string(),
            task_name: task_name.to_string(),
            profile_name: "dev".to_string(),
            profile_kind: EnvironmentKind::Native,
            command: if task_name == "test" {
                vec!["cargo".to_string(), "test".to_string()]
            } else {
                vec!["cargo".to_string(), "build".to_string()]
            },
            platforms: vec!["darwin-*".to_string()],
            cpu: Some(2),
            memory_mib: Some(2048),
            disk_mib: Some(2048),
            interactive,
            cache: Some(TaskCacheMode::ReadWrite),
            outputs: vec!["target/**".to_string()],
            features: vec!["rust".to_string()],
            sandbox: Some(TaskSandbox::Container),
            command_definition_hash: "b".repeat(64),
        }
    }

    fn scheduler_device(device_id: &str) -> SchedulerDeviceSnapshot {
        SchedulerDeviceSnapshot {
            device_id: device_id.to_string(),
            platform_key: "darwin-arm64".to_string(),
            os: "darwin".to_string(),
            architecture: "arm64".to_string(),
            cpu_cores: 8,
            memory_total_mib: Some(16_384),
            disk_total_mib: Some(1_000_000),
            features: vec!["rust".to_string(), "symlinks".to_string()],
            policy: SchedulerDevicePolicy::default(),
            dynamic: SchedulerDynamicResources {
                cpu_load_1m_milli: Some(1000),
                memory_free_mib: Some(8192),
                disk_free_mib: Some(500_000),
                power_source: ResourcePowerSource::Ac,
                low_power_mode: false,
                foreground_load: ForegroundLoad::Idle,
                network_route_quality: SchedulerNetworkRouteQuality::Unknown,
            },
        }
    }
}
