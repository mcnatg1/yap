use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use super::state::LiveInputDeviceView;

#[derive(Debug, Clone, PartialEq, Eq)]
struct DeviceInfo {
    id: String,
    label: String,
    is_default: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedInputDevice {
    pub id: Option<String>,
    pub label: Option<String>,
    pub recovered: bool,
}

pub fn list_input_devices(selected_id: Option<&str>) -> Vec<LiveInputDeviceView> {
    let host = cpal::default_host();
    let devices = input_device_infos(&host);
    let selected = select_input_device(&devices, selected_id);

    devices
        .into_iter()
        .map(|device| LiveInputDeviceView {
            id: device.id.clone(),
            label: device.label,
            is_default: device.is_default,
            selected: selected.as_ref().is_some_and(|selected| selected.id == device.id),
        })
        .collect()
}

pub fn resolve_input_device(selected_id: Option<&str>) -> ResolvedInputDevice {
    let host = cpal::default_host();
    resolve_input_device_from_infos(&input_device_infos(&host), selected_id)
}

fn resolve_input_device_from_infos(
    devices: &[DeviceInfo],
    selected_id: Option<&str>,
) -> ResolvedInputDevice {
    let selected = select_input_device(devices, selected_id);
    let recovered = selected_id.is_some() && selected.as_ref().map(|device| device.id.as_str()) != selected_id;

    ResolvedInputDevice {
        id: selected.as_ref().map(|device| device.id.clone()),
        label: selected.map(|device| device.label),
        recovered,
    }
}

pub fn preflight_input_device(selected_id: Option<&str>) -> Result<ResolvedInputDevice, String> {
    let host = cpal::default_host();
    let resolved = resolve_input_device(selected_id);
    let Some(selected_id) = resolved.id.as_deref() else {
        return Err("No input detected.".into());
    };
    let device = host
        .input_devices()
        .map_err(|err| format!("Microphone access failed: {err}"))?
        .enumerate()
        .find_map(|(index, device)| {
            let name = device.name().ok()?;
            (device_id(index, &name) == selected_id).then_some(device)
        })
        .ok_or_else(|| "Selected microphone is unavailable.".to_string())?;
    let config = device
        .default_input_config()
        .map_err(|err| format!("Microphone access failed: {err}"))?;
    let heard_input = Arc::new(AtomicBool::new(false));
    let heard_input_for_callback = Arc::clone(&heard_input);
    let stream = device
        .build_input_stream_raw(
            &config.config(),
            config.sample_format(),
            move |data, _| {
                if data.len() > 0 {
                    heard_input_for_callback.store(true, Ordering::Relaxed);
                }
            },
            |_| {},
            Some(Duration::from_millis(250)),
        )
        .map_err(|err| format!("Microphone access failed: {err}"))?;
    stream
        .play()
        .map_err(|err| format!("Microphone access failed: {err}"))?;
    std::thread::sleep(Duration::from_millis(160));
    drop(stream);
    if !heard_input.load(Ordering::Relaxed) {
        return Err("No input detected.".into());
    }
    Ok(resolved)
}

fn input_device_infos(host: &cpal::Host) -> Vec<DeviceInfo> {
    let default_name = host
        .default_input_device()
        .and_then(|device| device.name().ok());
    host.input_devices()
        .ok()
        .into_iter()
        .flatten()
        .enumerate()
        .filter_map(|(index, device)| {
            let label = device.name().ok()?;
            Some(DeviceInfo {
                id: device_id(index, &label),
                is_default: default_name.as_deref() == Some(label.as_str()),
                label,
            })
        })
        .collect()
}

fn device_id(index: usize, label: &str) -> String {
    format!("{index}:{label}")
}

fn select_input_device(
    devices: &[DeviceInfo],
    selected_id: Option<&str>,
) -> Option<DeviceInfo> {
    if let Some(selected) = selected_id {
        if let Some(device) = devices.iter().find(|device| device.id == selected) {
            return Some(device.clone());
        }
    }
    if let Some(device) = devices.iter().find(|device| device.is_default) {
        return Some(device.clone());
    }
    devices.first().cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_selected_device_recovers_to_default() {
        let devices = vec![
            DeviceInfo {
                id: "0:Built-in".into(),
                label: "Built-in".into(),
                is_default: true,
            },
            DeviceInfo {
                id: "1:USB mic".into(),
                label: "USB mic".into(),
                is_default: false,
            },
        ];

        let resolved = resolve_input_device_from_infos(&devices, Some("Gone"));

        assert_eq!(resolved.id.as_deref(), Some("0:Built-in"));
        assert!(resolved.recovered);
    }

    #[test]
    fn selected_device_wins_when_present() {
        let devices = vec![
            DeviceInfo {
                id: "0:Built-in".into(),
                label: "Built-in".into(),
                is_default: true,
            },
            DeviceInfo {
                id: "1:USB mic".into(),
                label: "USB mic".into(),
                is_default: false,
            },
        ];

        let resolved = resolve_input_device_from_infos(&devices, Some("1:USB mic"));

        assert_eq!(resolved.id.as_deref(), Some("1:USB mic"));
        assert!(!resolved.recovered);
    }
}
