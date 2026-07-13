#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioCommand {
    Play,
    Pause,
    Stop,
    SetVolume(u8),
    PlayUrl(&'static str), // Or a heapless::String<64>
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppEvent {
    PlaybackStarted,
    PlaybackPaused,
    PlaybackStopped,
    VolumeChanged(u8),
    Error(&'static str),
}
