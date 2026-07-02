use crate::stt::gpu::GpuPreference;

pub fn effective_gpu_preference() -> GpuPreference {
    preference_from_env_value(std::env::var("YAP_USE_GPU").ok().as_deref())
}

fn preference_from_env_value(value: Option<&str>) -> GpuPreference {
    let Some(trimmed) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return GpuPreference::Auto;
    };
    crate::stt::gpu::gpu_preference_from(Some(trimmed))
}

pub fn polish_num_gpu_layers() -> u32 {
    match effective_gpu_preference() {
        GpuPreference::Cpu => 0,
        GpuPreference::Auto | GpuPreference::On => 99,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_settings_use_auto_gpu() {
        assert_eq!(preference_from_env_value(None), GpuPreference::Auto);
        assert_eq!(preference_from_env_value(Some(" ")), GpuPreference::Auto);
    }

    #[test]
    fn env_can_force_cpu() {
        assert_eq!(preference_from_env_value(Some("cpu")), GpuPreference::Cpu);
    }
}
