use core::fmt::Write;
use heapless::String;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppEvent {
    // Coms
    KeepAlive,
    // Audio
    PlaybackStarted,
    PlaybackPaused,
    PlaybackStopped,
    VolumeChanged(u8),
    // User actions
    Key1Pressed,
    Key2Pressed,
    Key3Pressed,
    // Generic
    Error(&'static str),
}

pub struct MQTTTopics {
    device_id: &'static str,
}

impl MQTTTopics {
    pub const fn new(device_id: &'static str) -> Self {
        Self { device_id }
    }

    #[cfg(test)]
    pub fn button_press_topic(&self, button_id: u8) -> Result<String<64>, core::fmt::Error> {
        self.button_press(button_id)
    }

    fn build_topic(&self, event: &str) -> Result<String<64>, core::fmt::Error> {
        let mut topic = String::new();
        write!(&mut topic, "speaker/{}/{}", self.device_id, event)?;
        Ok(topic)
    }

    pub fn get_prefix(&self) -> Result<String<64>, core::fmt::Error> {
        self.build_topic("")
    }

    pub fn subscrive_wildcard(&self) -> Result<String<64>, core::fmt::Error> {
        self.build_topic("#")
    }

    pub fn volume_changed(&self) -> Result<String<64>, core::fmt::Error> {
        self.build_topic("volume")
    }

    pub fn status(&self) -> Result<String<64>, core::fmt::Error> {
        self.build_topic("status")
    }

    pub fn button_press(&self, button_id: u8) -> Result<String<64>, core::fmt::Error> {
        let mut topic = String::new();
        write!(
            &mut topic,
            "speaker/{}/button/{}",
            self.device_id, button_id
        )?;
        Ok(topic)
    }
}

#[cfg(test)]
mod tests {
    use super::MQTTTopics;

    #[test]
    fn button_press_topic_uses_button_id() {
        let topics = MQTTTopics::new("demo-device");
        let topic = topics.button_press_topic(3).unwrap();
        assert_eq!(topic.as_str(), "speaker/demo-device/button/3");
    }
}
