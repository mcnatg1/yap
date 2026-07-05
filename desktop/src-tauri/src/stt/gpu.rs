use std::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpuPreference {
    /// CPU-only via CrispASR `-ng`.
    Cpu,
    /// Use GPU acceleration when a graphics adapter is detected.
    Auto,
    /// Request GPU layers when available; otherwise CPU.
    On,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GpuStatus {
    pub available: bool,
    pub adapter_name: Option<String>,
    pub preference: GpuPreference,
    pub layers: u32,
}

impl GpuStatus {
    pub fn resolve() -> Self {
        let preference = crate::stt::settings::effective_gpu_preference();
        let adapter_name = detect_gpu_name();
        let available = adapter_name.is_some();
        let layers = gpu_layers(preference, available);
        Self {
            available,
            adapter_name,
            preference,
            layers,
        }
    }

    pub fn using_gpu(&self) -> bool {
        self.layers > 0
    }

    pub fn runner_label(&self) -> &'static str {
        if self.using_gpu() {
            "GPU-accelerated"
        } else if self.available {
            "CPU (GPU available)"
        } else {
            "CPU"
        }
    }
}

pub fn gpu_preference_from_env() -> GpuPreference {
    gpu_preference_from(std::env::var("YAP_USE_GPU").ok().as_deref())
}

pub fn gpu_preference_from(value: Option<&str>) -> GpuPreference {
    match value.map(str::trim).map(str::to_lowercase).as_deref() {
        Some("auto") => GpuPreference::Auto,
        Some("1") | Some("true") | Some("on") | Some("yes") => GpuPreference::On,
        _ => GpuPreference::Cpu,
    }
}

pub fn gpu_layers(preference: GpuPreference, gpu_available: bool) -> u32 {
    match preference {
        GpuPreference::Cpu => 0,
        GpuPreference::Auto | GpuPreference::On if gpu_available => 99,
        GpuPreference::Auto | GpuPreference::On => 0,
    }
}

pub fn detect_gpu_name() -> Option<String> {
    if let Some(name) = nvidia_gpu_name() {
        return Some(name);
    }
    if let Some(name) = platform_gpu_name() {
        return Some(name);
    }
    None
}

fn nvidia_gpu_name() -> Option<String> {
    let mut command = Command::new("nvidia-smi");
    command.args(["--query-gpu=name", "--format=csv,noheader"]);
    crate::stt::hide_child_console(&mut command);
    let output = command.output().ok()?;
    if !output.status.success() {
        return None;
    }
    let name = String::from_utf8_lossy(&output.stdout)
        .trim()
        .lines()
        .next()?
        .trim()
        .to_string();
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}

fn platform_gpu_name() -> Option<String> {
    #[cfg(windows)]
    {
        windows_gpu_name()
    }
    #[cfg(not(windows))]
    {
        None
    }
}

#[cfg(windows)]
fn windows_gpu_name() -> Option<String> {
    let script = "Get-CimInstance Win32_VideoController | Where-Object { $_.PNPDeviceID -like 'PCI*' -and $_.Name -notmatch 'Virtual|Mirror|Remote|Basic Display' } | Select-Object -First 1 -ExpandProperty Name";
    let mut command = Command::new("powershell");
    command.args(["-NoProfile", "-Command", script]);
    crate::stt::hide_child_console(&mut command);
    let output = command.output().ok()?;
    if !output.status.success() {
        return None;
    }
    let name = String::from_utf8_lossy(&output.stdout)
        .trim()
        .lines()
        .next()?
        .trim()
        .to_string();
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_preference_is_cpu() {
        assert_eq!(gpu_preference_from(None), GpuPreference::Cpu);
        assert_eq!(gpu_preference_from(Some("0")), GpuPreference::Cpu);
    }

    #[test]
    fn layers_default_to_zero_even_when_gpu_available() {
        assert_eq!(gpu_layers(GpuPreference::Cpu, true), 0);
    }

    #[test]
    fn auto_uses_gpu_layers_when_available() {
        assert_eq!(gpu_layers(GpuPreference::Auto, true), 99);
        assert_eq!(gpu_layers(GpuPreference::Auto, false), 0);
    }

    #[test]
    fn on_requests_gpu_when_available() {
        assert_eq!(gpu_layers(GpuPreference::On, true), 99);
        assert_eq!(gpu_layers(GpuPreference::On, false), 0);
    }
}
