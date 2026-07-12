use std::{sync::mpsc, time::Duration};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use super::state::LiveInputDeviceView;

const MICROPHONE_PERMISSION_DENIED_PREFIX: &str = "Microphone permission denied:";
const PREFLIGHT_INPUT_DEADLINE: Duration = Duration::from_millis(160);

#[derive(Debug, PartialEq, Eq)]
enum PreflightEvent {
    Input,
    StreamError(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MicrophoneOperation {
    DeviceEnumeration,
    DeviceMetadata,
    DefaultInputConfiguration,
    InputStreamBuild,
    InputStreamPlayback,
}

fn microphone_error(operation: MicrophoneOperation, error: impl std::fmt::Display) -> String {
    let detail = error.to_string();
    if is_permission_denied_detail(&detail) {
        return format!("{MICROPHONE_PERMISSION_DENIED_PREFIX} {detail}");
    }
    let operation = match operation {
        MicrophoneOperation::DeviceEnumeration => "device enumeration",
        MicrophoneOperation::DeviceMetadata => "device metadata",
        MicrophoneOperation::DefaultInputConfiguration => "default input configuration",
        MicrophoneOperation::InputStreamBuild => "input stream build",
        MicrophoneOperation::InputStreamPlayback => "input stream playback",
    };
    format!("Microphone {operation} failed: {detail}")
}

fn is_permission_denied_detail(detail: &str) -> bool {
    let normalized = detail.to_ascii_lowercase();
    normalized.contains("0x80070005")
        || normalized.contains("e_accessdenied")
        || normalized.contains("access is denied")
        || normalized.contains("permission denied")
}

fn microphone_result<T, E>(
    operation: MicrophoneOperation,
    result: Result<T, E>,
) -> Result<T, String>
where
    E: std::fmt::Display,
{
    result.map_err(|error| microphone_error(operation, error))
}

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

pub(crate) struct ResolvedCaptureDevice {
    pub(crate) selection: ResolvedInputDevice,
    pub(crate) device: cpal::Device,
    pub(crate) config: cpal::SupportedStreamConfig,
}

pub fn list_input_devices(selected_id: Option<&str>) -> Result<Vec<LiveInputDeviceView>, String> {
    let host = cpal::default_host();
    let devices = strict_input_devices(&host)?
        .into_iter()
        .map(|(info, _)| info)
        .collect::<Vec<_>>();
    let selected = select_input_device(&devices, selected_id);

    Ok(devices
        .into_iter()
        .map(|device| LiveInputDeviceView {
            id: device.id.clone(),
            label: device.label,
            is_default: device.is_default,
            selected: selected
                .as_ref()
                .is_some_and(|selected| selected.id == device.id),
        })
        .collect())
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
    let recovered =
        selected_id.is_some() && selected.as_ref().map(|device| device.id.as_str()) != selected_id;

    ResolvedInputDevice {
        id: selected.as_ref().map(|device| device.id.clone()),
        label: selected.map(|device| device.label),
        recovered,
    }
}

pub fn preflight_input_device(selected_id: Option<&str>) -> Result<ResolvedInputDevice, String> {
    let resolved = resolve_capture_device(selected_id)?;
    let (events, receiver) = mpsc::sync_channel(1);
    let input_events = events.clone();
    let stream = resolved
        .device
        .build_input_stream_raw(
            &resolved.config.config(),
            resolved.config.sample_format(),
            move |data, _| {
                if data.len() > 0 {
                    let _ = input_events.try_send(PreflightEvent::Input);
                }
            },
            move |error| {
                let _ = events.try_send(PreflightEvent::StreamError(microphone_error(
                    MicrophoneOperation::InputStreamPlayback,
                    error,
                )));
            },
            Some(Duration::from_millis(250)),
        )
        .map_err(|error| microphone_error(MicrophoneOperation::InputStreamBuild, error))?;
    microphone_result(MicrophoneOperation::InputStreamPlayback, stream.play())?;
    let preflight = wait_for_preflight_event(receiver, PREFLIGHT_INPUT_DEADLINE);
    drop(stream);
    preflight?;
    Ok(resolved.selection)
}

fn wait_for_preflight_event(
    receiver: mpsc::Receiver<PreflightEvent>,
    deadline: Duration,
) -> Result<(), String> {
    match receiver.recv_timeout(deadline) {
        Ok(PreflightEvent::Input) => Ok(()),
        Ok(PreflightEvent::StreamError(error)) => Err(error),
        Err(mpsc::RecvTimeoutError::Timeout) => Err("No input detected.".into()),
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            Err("Microphone preflight stopped before input was detected.".into())
        }
    }
}

pub(crate) fn resolve_capture_device(
    selected_id: Option<&str>,
) -> Result<ResolvedCaptureDevice, String> {
    let host = cpal::default_host();
    let devices = strict_input_devices(&host)?;
    let infos = devices
        .iter()
        .map(|(info, _)| info.clone())
        .collect::<Vec<_>>();
    let selected =
        select_input_device(&infos, selected_id).ok_or_else(|| "No input detected.".to_string())?;
    let recovered = selected_id.is_some_and(|requested| requested != selected.id);
    let (_, device) = devices
        .into_iter()
        .find(|(info, _)| info.id == selected.id)
        .ok_or_else(|| "Selected microphone is unavailable.".to_string())?;
    let config = device
        .default_input_config()
        .map_err(|error| microphone_error(MicrophoneOperation::DefaultInputConfiguration, error))?;
    Ok(ResolvedCaptureDevice {
        selection: ResolvedInputDevice {
            id: Some(selected.id),
            label: Some(selected.label),
            recovered,
        },
        device,
        config,
    })
}

fn input_device_infos(host: &cpal::Host) -> Vec<DeviceInfo> {
    strict_input_devices(host)
        .unwrap_or_default()
        .into_iter()
        .map(|(info, _)| info)
        .collect()
}

fn strict_input_devices(host: &cpal::Host) -> Result<Vec<(DeviceInfo, cpal::Device)>, String> {
    let default_name = host
        .default_input_device()
        .map(|device| microphone_result(MicrophoneOperation::DeviceMetadata, device.name()))
        .transpose()?;
    microphone_result(MicrophoneOperation::DeviceEnumeration, host.input_devices())?
        .enumerate()
        .map(|(index, device)| {
            let label = microphone_result(MicrophoneOperation::DeviceMetadata, device.name())?;
            Ok((
                DeviceInfo {
                    id: device_id(index, &label),
                    is_default: default_name.as_deref() == Some(label.as_str()),
                    label,
                },
                device,
            ))
        })
        .collect()
}

fn device_id(index: usize, label: &str) -> String {
    format!("{index}:{label}")
}

fn select_input_device(devices: &[DeviceInfo], selected_id: Option<&str>) -> Option<DeviceInfo> {
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
    fn preflight_accepts_input_before_a_stream_error() {
        let (events, receiver) = std::sync::mpsc::sync_channel(1);
        events.try_send(PreflightEvent::Input).unwrap();
        assert!(matches!(
            events.try_send(PreflightEvent::StreamError("too late".into())),
            Err(std::sync::mpsc::TrySendError::Full(_))
        ));

        assert_eq!(
            wait_for_preflight_event(receiver, Duration::from_millis(10)),
            Ok(())
        );
    }

    #[test]
    fn preflight_returns_the_first_actionable_stream_error() {
        for (detail, expected) in [
            (
                "E_ACCESSDENIED",
                "Microphone permission denied: E_ACCESSDENIED",
            ),
            (
                "device invalidated",
                "Microphone input stream playback failed: device invalidated",
            ),
        ] {
            let (events, receiver) = std::sync::mpsc::sync_channel(1);
            events
                .try_send(PreflightEvent::StreamError(microphone_error(
                    MicrophoneOperation::InputStreamPlayback,
                    detail,
                )))
                .unwrap();
            assert!(matches!(
                events.try_send(PreflightEvent::Input),
                Err(std::sync::mpsc::TrySendError::Full(_))
            ));

            assert_eq!(
                wait_for_preflight_event(receiver, Duration::from_millis(10)),
                Err(expected.into())
            );
        }
    }

    #[test]
    fn preflight_reports_no_input_only_after_the_deadline() {
        let (_events, receiver) = std::sync::mpsc::sync_channel(1);

        assert_eq!(
            wait_for_preflight_event(receiver, Duration::from_millis(1)),
            Err("No input detected.".into())
        );
    }

    #[test]
    fn preflight_ignores_a_callback_after_the_deadline() {
        let (events, receiver) = std::sync::mpsc::sync_channel(1);

        assert_eq!(
            wait_for_preflight_event(receiver, Duration::ZERO),
            Err("No input detected.".into())
        );
        assert!(matches!(
            events.try_send(PreflightEvent::Input),
            Err(std::sync::mpsc::TrySendError::Disconnected(_))
        ));
    }

    #[test]
    fn native_microphone_failures_name_the_exact_operation() {
        let cases = [
            (
                MicrophoneOperation::DeviceEnumeration,
                "Microphone device enumeration failed: backend unavailable",
            ),
            (
                MicrophoneOperation::DeviceMetadata,
                "Microphone device metadata failed: name unavailable",
            ),
            (
                MicrophoneOperation::DefaultInputConfiguration,
                "Microphone default input configuration failed: unsupported format",
            ),
            (
                MicrophoneOperation::InputStreamBuild,
                "Microphone input stream build failed: backend regression",
            ),
            (
                MicrophoneOperation::InputStreamPlayback,
                "Microphone input stream playback failed: device invalidated",
            ),
        ];

        for (operation, expected) in cases {
            let detail = expected.split_once(": ").unwrap().1;
            assert_eq!(microphone_error(operation, detail), expected);
            assert!(!microphone_error(operation, detail)
                .starts_with(MICROPHONE_PERMISSION_DENIED_PREFIX));
        }
    }

    #[test]
    fn only_narrow_permission_signatures_receive_the_skip_marker() {
        for detail in [
            "Access is denied. (0x80070005)",
            "E_ACCESSDENIED",
            "permission denied by operating system",
        ] {
            assert!(
                microphone_error(MicrophoneOperation::InputStreamBuild, detail)
                    .starts_with(MICROPHONE_PERMISSION_DENIED_PREFIX)
            );
        }

        for detail in [
            "backend unavailable",
            "device invalidated",
            "unsupported stream configuration",
            "No input detected.",
        ] {
            assert!(
                !microphone_error(MicrophoneOperation::InputStreamBuild, detail)
                    .starts_with(MICROPHONE_PERMISSION_DENIED_PREFIX)
            );
        }
    }

    #[test]
    fn device_enumeration_result_propagates_backend_failure() {
        let result = microphone_result::<Vec<DeviceInfo>, _>(
            MicrophoneOperation::DeviceEnumeration,
            Err("backend unavailable"),
        );

        assert_eq!(
            result.unwrap_err(),
            "Microphone device enumeration failed: backend unavailable"
        );
    }

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
