pub const CHANNEL_SIZE: usize = 8;
pub const AUDIO_QUEUE_DEPTH: usize = 8;

// Audio
pub const AUDIO_SAMPLE_RATE: u32 = 48_000;
pub const DEFAULT_VOLUME: u8 = 70;
pub const AUDIO_DMA_BUF_SIZE: usize = 4096;

// LEDs
pub const DEFAULT_LED_BRIGHTNESS: u8 = 10;

// Wi-Fi
pub const MAX_STA_FAILS: u32 = 3;
pub const DHCP_TIMEOUT_SECS: u64 = 15;
pub const AP_SSID: &str = "ESpeaker-Setup";
pub const AP_IP: &str = "192.168.4.1";
pub const AP_PORT: u16 = 80;
pub const AP_SUBNET_PREFIX_LEN: u8 = 24;

// MQTT
pub const MQTT_PORT: u16 = 1883;
pub const MQTT_KEEPALIVE_SECS: u16 = 30;
pub const MQTT_SESSION_EXPIRY_SECS: u32 = 60;
pub const MQTT_SOCKET_TIMEOUT_SECS: u16 = 10;
pub const MQTT_RECONNECT_DELAY_SECS: u8 = 5;
pub const MQTT_TOPIC_COMMANDS: &str = "speaker/commands";
pub const MQTT_TOPIC_STATUS: &str = "speaker/status";
pub const MQTT_TOPIC_VOLUME: &str = "speaker/volume";

// Time sync
pub const NTP_SERVER: &str = "pool.ntp.org";
pub const NTP_RESYNC_INTERVAL_SECS: u64 = 3600;
pub const SNTP_TIMEOUT_SECS: u64 = 5;

// Button / factory reset
pub const BUTTON_HOLD_DURATION_SECS: u64 = 3;
pub const FACTORY_RESET_REBOOT_DELAY_MS: u64 = 600;
pub const BUTTON_POLL_INTERVAL_MS: u64 = 50;
pub const BUTTON_DEBOUNCE_MS: u64 = 80;

// Boot delays
pub const WIFI_FAIL_REBOOT_DELAY_SECS: u64 = 2;
pub const PORTAL_REBOOT_DELAY_SECS: u64 = 1;
pub const IDLE_HEARTBEAT_INTERVAL_SECS: u64 = 30;

// Wi-Fi STA
pub const STA_RECONNECT_DELAY_SECS: u64 = 5;

// Wi-Fi AP
pub const AP_SOCKET_TIMEOUT_SECS: u16 = 10;
pub const AP_SERVE_CLOSE_DELAY_MS: u64 = 500;
pub const AP_EVENT_POLL_INTERVAL_MS: u64 = 5000;

// Audio codec timing
pub const CODEC_UNMUTE_AMP_ON_DELAY_MS: u64 = 10;
pub const CODEC_AMP_ON_PLAY_DELAY_MS: u64 = 30;
pub const CODEC_AMP_OFF_MUTE_DELAY_MS: u64 = 5;
