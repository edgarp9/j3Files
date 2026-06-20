use std::cmp::Ordering;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DpiMetrics {
    current_dpi: u32,
}

impl DpiMetrics {
    pub const BASE_DPI: u32 = 96;

    pub fn new(current_dpi: u32) -> Self {
        Self {
            current_dpi: current_dpi.max(1),
        }
    }

    pub fn current_dpi(self) -> u32 {
        self.current_dpi
    }

    pub fn scale_factor(self) -> f64 {
        self.current_dpi as f64 / Self::BASE_DPI as f64
    }

    pub fn ui_scale(self) -> UiScale {
        UiScale::from_metrics(self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UiScale {
    dpi: u32,
}

impl UiScale {
    pub fn from_metrics(metrics: DpiMetrics) -> Self {
        Self {
            dpi: metrics.current_dpi(),
        }
    }

    pub fn px(self, value: i32) -> i32 {
        scale_pixels(value, self.dpi)
    }

    pub fn size(self, width: i32, height: i32) -> (i32, i32) {
        (self.px(width), self.px(height))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DpiAwarenessStep {
    api: &'static str,
    mode: &'static str,
    pub(super) operation: DpiAwarenessOperation,
}

impl DpiAwarenessStep {
    pub fn api(self) -> &'static str {
        self.api
    }

    pub fn mode(self) -> &'static str {
        self.mode
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DpiAwarenessFailureReason {
    Unavailable,
    Win32(u32),
    Hresult(i32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DpiAwarenessFailure {
    pub(super) step: DpiAwarenessStep,
    pub(super) reason: DpiAwarenessFailureReason,
}

impl DpiAwarenessFailure {
    pub fn step(self) -> DpiAwarenessStep {
        self.step
    }

    pub fn reason(self) -> DpiAwarenessFailureReason {
        self.reason
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DpiAwarenessOutcome {
    pub(super) applied: Option<DpiAwarenessStep>,
    pub(super) failures: Vec<DpiAwarenessFailure>,
}

impl DpiAwarenessOutcome {
    pub fn applied_step(&self) -> Option<DpiAwarenessStep> {
        self.applied
    }

    pub fn failures(&self) -> &[DpiAwarenessFailure] {
        &self.failures
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DpiAwarenessOperation {
    ContextSystemAware,
    ContextPerMonitorAware,
    ContextPerMonitorAwareV2,
    ProcessSystemAware,
    ProcessPerMonitorAware,
    ProcessDpiAware,
}

pub(super) const SYSTEM_AWARE_DPI_STEPS: [DpiAwarenessStep; 6] = [
    DpiAwarenessStep {
        api: "SetProcessDpiAwarenessContext",
        mode: "SYSTEM_AWARE",
        operation: DpiAwarenessOperation::ContextSystemAware,
    },
    DpiAwarenessStep {
        api: "SetProcessDpiAwarenessContext",
        mode: "PER_MONITOR_AWARE",
        operation: DpiAwarenessOperation::ContextPerMonitorAware,
    },
    DpiAwarenessStep {
        api: "SetProcessDpiAwarenessContext",
        mode: "PER_MONITOR_AWARE_V2",
        operation: DpiAwarenessOperation::ContextPerMonitorAwareV2,
    },
    DpiAwarenessStep {
        api: "SetProcessDpiAwareness",
        mode: "SYSTEM_DPI_AWARE",
        operation: DpiAwarenessOperation::ProcessSystemAware,
    },
    DpiAwarenessStep {
        api: "SetProcessDpiAwareness",
        mode: "PER_MONITOR_DPI_AWARE",
        operation: DpiAwarenessOperation::ProcessPerMonitorAware,
    },
    DpiAwarenessStep {
        api: "SetProcessDPIAware",
        mode: "PROCESS_DPI_AWARE",
        operation: DpiAwarenessOperation::ProcessDpiAware,
    },
];

fn scale_pixels(value: i32, dpi: u32) -> i32 {
    let numerator = i64::from(value) * i64::from(dpi.max(1));
    let denominator = i64::from(DpiMetrics::BASE_DPI);
    let rounded = if numerator >= 0 {
        (numerator + denominator / 2) / denominator
    } else {
        (numerator - denominator / 2) / denominator
    };
    let scaled = rounded.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32;

    match value.cmp(&0) {
        Ordering::Greater => scaled.max(1),
        Ordering::Less => scaled.min(-1),
        Ordering::Equal => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ui_scale_rounds_pixels_from_base_dpi() {
        let scale = DpiMetrics::new(144).ui_scale();

        assert_eq!(scale.px(0), 0);
        assert_eq!(scale.px(1), 2);
        assert_eq!(scale.px(20), 30);
        assert_eq!(scale.size(1000, 700), (1500, 1050));
    }

    #[test]
    fn system_aware_dpi_policy_uses_stable_fallback_order() {
        let labels = SYSTEM_AWARE_DPI_STEPS
            .iter()
            .map(|step| (step.api(), step.mode()))
            .collect::<Vec<_>>();

        assert_eq!(
            labels,
            vec![
                ("SetProcessDpiAwarenessContext", "SYSTEM_AWARE"),
                ("SetProcessDpiAwarenessContext", "PER_MONITOR_AWARE"),
                ("SetProcessDpiAwarenessContext", "PER_MONITOR_AWARE_V2"),
                ("SetProcessDpiAwareness", "SYSTEM_DPI_AWARE"),
                ("SetProcessDpiAwareness", "PER_MONITOR_DPI_AWARE"),
                ("SetProcessDPIAware", "PROCESS_DPI_AWARE"),
            ]
        );
    }
}
