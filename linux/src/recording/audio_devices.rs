//! Microphones, as PulseAudio (or PipeWire's Pulse server) sources.

use gst::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Microphone {
    /// The source name `pulsesrc` takes as its `device`.
    pub id: String,
    pub name: String,
}

/// Every input device, without the "Monitor of …" sources that mirror outputs.
pub fn microphones() -> Vec<Microphone> {
    let Some(provider) = gst::DeviceProviderFactory::by_name("pulsedeviceprovider") else {
        crate::log::write("No pulsedeviceprovider: is the GStreamer PulseAudio plugin installed?");
        return Vec::new();
    };
    provider
        .devices()
        .into_iter()
        .filter(|device| device.has_classes("Audio/Source"))
        .filter(|device| {
            let class = device.properties().and_then(|p| p.get::<String>("device.class").ok());
            class.as_deref() != Some("monitor")
        })
        .filter_map(|device| {
            let element = device.create_element(None).ok()?;
            let id = element.property::<Option<String>>("device")?;
            Some(Microphone { id, name: device.display_name().to_string() })
        })
        .collect()
}

/// The name of the default input, for "System Default (…)".
pub fn default_microphone_name() -> Option<String> {
    let provider = gst::DeviceProviderFactory::by_name("pulsedeviceprovider")?;
    provider
        .devices()
        .into_iter()
        .filter(|device| device.has_classes("Audio/Source"))
        .find(|device| {
            device.properties().and_then(|p| p.get::<bool>("is-default").ok()).unwrap_or(false)
        })
        .map(|device| device.display_name().to_string())
}
