use std::{fmt, str::FromStr};

use cpal::traits::{DeviceTrait, HostTrait};
use rodio::{DeviceSinkBuilder, MixerDeviceSink};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OutputDeviceChoice {
    pub id: Option<String>,
    pub label: String,
}

impl fmt::Display for OutputDeviceChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.id.is_none() {
            f.write_str(
                crate::i18n::translate(crate::i18n::current_language(), &self.label).as_ref(),
            )
        } else {
            f.write_str(&self.label)
        }
    }
}

pub fn enumerate_output_devices() -> Result<Vec<OutputDeviceChoice>, String> {
    let host = cpal::default_host();
    let default_id = host
        .default_output_device()
        .and_then(|device| device.id().ok())
        .map(|id| id.to_string());
    let mut choices = vec![OutputDeviceChoice {
        id: None,
        label: String::from("System default"),
    }];
    let devices = host.output_devices().map_err(|error| error.to_string())?;
    for device in devices {
        let Ok(id) = device.id() else {
            continue;
        };
        let id = id.to_string();
        if default_id.as_deref() == Some(&id) {
            continue;
        }
        let label = device
            .description()
            .map(|description| description.name().to_owned())
            .unwrap_or_else(|_| String::from("Audio output"));
        choices.push(OutputDeviceChoice {
            id: Some(id),
            label,
        });
    }
    choices[1..].sort_by(|left, right| {
        left.label
            .to_lowercase()
            .cmp(&right.label.to_lowercase())
            .then_with(|| left.id.cmp(&right.id))
    });
    Ok(choices)
}

pub fn open_output_device(selected_id: Option<&str>) -> Result<(MixerDeviceSink, String), String> {
    let host = cpal::default_host();
    let device = match selected_id {
        Some(id) => {
            let id = cpal::DeviceId::from_str(id).map_err(|error| error.to_string())?;
            host.device_by_id(&id)
                .ok_or_else(|| "The selected output device is no longer available".to_owned())?
        }
        None => host
            .default_output_device()
            .ok_or_else(|| "No system default output device is available".to_owned())?,
    };
    let label = device
        .description()
        .map(|description| description.name().to_owned())
        .unwrap_or_else(|_| String::from("Audio output"));
    // A bigger period means fewer device callbacks. Each callback wakes the audio server, which
    // dominates the output cost while paused. The fallback keeps the device's own buffer if
    // this size is refused.
    let sink = DeviceSinkBuilder::from_device(device)
        .map(|builder| builder.with_buffer_size(cpal::BufferSize::Fixed(4096)))
        .and_then(|builder| builder.open_sink_or_fallback())
        .map_err(|error| error.to_string())?;
    Ok((sink, label))
}

#[cfg(test)]
mod tests {
    use super::{OutputDeviceChoice, enumerate_output_devices, open_output_device};

    #[test]
    fn default_output_choice_has_a_stable_setting_label() {
        let default = OutputDeviceChoice {
            id: None,
            label: String::from("System default"),
        };
        assert_eq!(default.to_string(), "System default");
        assert_eq!(default.id, None);
    }

    #[test]
    fn device_enumeration_returns_a_default_choice_or_a_clear_backend_error() {
        match enumerate_output_devices() {
            Ok(choices) => {
                assert_eq!(
                    choices.first().and_then(|choice| choice.id.as_deref()),
                    None
                );
                assert_eq!(
                    choices.first().map(|choice| choice.label.as_str()),
                    Some("System default")
                );
                assert!(choices.iter().skip(1).all(|choice| choice.id.is_some()));
            }
            Err(error) => assert!(!error.is_empty()),
        }
    }

    #[test]
    fn malformed_output_device_ids_are_rejected_before_opening_a_device() {
        assert!(open_output_device(Some("not-a-valid-device-id")).is_err());
    }
}
