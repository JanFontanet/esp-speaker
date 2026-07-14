use core::net::{Ipv4Addr, SocketAddr};
use core::num::NonZero;
use core::str::from_utf8;

use embassy_executor::Spawner;
use embassy_futures::select::{Either3, select3};
use embassy_net::{Stack, tcp::TcpSocket};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::{Receiver, Sender};
use embassy_time::{Duration, Timer};
use rust_mqtt::client::options::{PublicationOptions, SubscriptionOptions, TopicReference};
use rust_mqtt::config::{KeepAlive, SessionExpiryInterval};
use rust_mqtt::types::{MqttString, TopicFilter, TopicName};
use rust_mqtt::{
    buffer::AllocBuffer,
    client::{
        Client,
        event::Event,
        options::{ConnectOptions, DisconnectOptions},
    },
};
use static_cell::StaticCell;

use crate::audio::AudioCommand;
use crate::config::{
    CHANNEL_SIZE, MQTT_KEEPALIVE_SECS, MQTT_PORT, MQTT_RECONNECT_DELAY_SECS,
    MQTT_SESSION_EXPIRY_SECS, MQTT_SOCKET_TIMEOUT_SECS,
};
use crate::led::{Animation, Color, LedCommand, led_send};
use crate::mqtt::msg_protocol::{AppEvent, MQTTTopics};
use crate::wifi::DeviceConfig;

pub type AudioCommandSender = Sender<'static, CriticalSectionRawMutex, AudioCommand, CHANNEL_SIZE>;
pub type EventReceiver<AppEvent> =
    Receiver<'static, CriticalSectionRawMutex, AppEvent, CHANNEL_SIZE>;

pub struct CommandRouter {
    audio_tx: AudioCommandSender,
}

impl CommandRouter {
    pub fn new(audio_tx: AudioCommandSender) -> Self {
        Self { audio_tx }
    }

    pub async fn dispatch(&self, path: &str, payload: &str) {
        let Some((domain, operation)) = path.split_once('/') else {
            defmt::warn!("mqtt: malformed command topic '{}'", path);
            return;
        };

        if operation == "state" || operation == "attributes" {
            return;
        }

        match domain {
            "audio" => match parse_audio_command(operation, payload) {
                Some(command) => self.audio_tx.send(command).await,
                None => defmt::warn!("mqtt: invalid audio command '{}': '{}'", operation, payload),
            },
            "led" => match parse_led_command(operation, payload) {
                Some(command) => led_send(command),
                None => defmt::warn!("mqtt: invalid LED command '{}': '{}'", operation, payload),
            },
            "media_player" => self.handle_media_player_command(operation, payload).await,
            _ => defmt::warn!("mqtt: unknown command domain '{}'", domain),
        }
    }

    async fn handle_media_player_command(&self, operation: &str, payload: &str) {
        match operation {
            "set" => {
                if payload == "PLAY" || payload.contains("\"command\":\"play\"") {
                    let _ = self.audio_tx.send(AudioCommand::Play).await;
                } else if payload == "PAUSE" || payload.contains("\"command\":\"pause\"") {
                    let _ = self.audio_tx.send(AudioCommand::Pause).await;
                } else if payload == "STOP" || payload.contains("\"command\":\"stop\"") {
                    let _ = self.audio_tx.send(AudioCommand::Stop).await;
                } else if let Some(value) = payload.strip_prefix("{\"volume_level\":") {
                    if let Some(end) = value.find('}') {
                        if let Ok(level) = value[..end].parse::<u8>() {
                            let _ = self.audio_tx.send(AudioCommand::SetVolume(level)).await;
                        }
                    }
                } else if payload.starts_with("{\"announce\":\"") {
                    let announcement = payload
                        .trim_start_matches("{\"announce\":\"")
                        .trim_end_matches("\"}");
                    let _ = self
                        .audio_tx
                        .send(AudioCommand::PlayAnnouncement(
                            heapless::String::try_from(announcement).unwrap_or_default(),
                        ))
                        .await;
                }
            }
            _ => defmt::warn!("mqtt: unsupported media_player operation '{}'", operation),
        }
    }
}

static USER_CELL: StaticCell<heapless::String<32>> = StaticCell::new();
static PWD_CELL: StaticCell<heapless::String<64>> = StaticCell::new();

pub fn mqtt_spawn(
    spawner: &Spawner,
    stack: Stack<'static>,
    config: &DeviceConfig,
    client_id: &'static str,
    audio_tx: AudioCommandSender,
    event_rx: EventReceiver<AppEvent>,
) {
    let mqtt_address: Ipv4Addr = config.mqtt_address().parse().unwrap();
    let addr = SocketAddr::new(mqtt_address.into(), MQTT_PORT);
    let mqtt_user = USER_CELL
        .try_init(heapless::String::try_from(config.mqtt_user()).unwrap())
        .map(|s| s.as_str())
        .unwrap_or_else(|| unsafe { USER_CELL.uninit().assume_init_mut().as_str() });

    let mqtt_pwd = PWD_CELL
        .try_init(heapless::String::try_from(config.mqtt_pwd()).unwrap())
        .map(|s| s.as_str())
        .unwrap_or_else(|| unsafe { PWD_CELL.uninit().assume_init_mut().as_str() });

    spawner.spawn(
        mqtt_task(
            stack,
            addr,
            mqtt_user,
            mqtt_pwd,
            client_id,
            CommandRouter::new(audio_tx),
            event_rx,
        )
        .unwrap(),
    );
}

#[embassy_executor::task]
async fn mqtt_task(
    stack: Stack<'static>,
    mqtt_address: SocketAddr,
    mqtt_user: &'static str,
    mqtt_pwd: &'static str,
    client_id: &'static str,
    command_router: CommandRouter,
    event_rx: EventReceiver<AppEvent>,
) {
    loop {
        if let Err(_) = run_mqtt(
            stack,
            mqtt_address,
            mqtt_user,
            mqtt_pwd,
            client_id,
            &command_router,
            &event_rx,
        )
        .await
        {
            defmt::error!("MQTT error, reconnecting in 5s...");
            Timer::after(Duration::from_secs(MQTT_RECONNECT_DELAY_SECS as u64)).await;
        }
    }
}

async fn run_mqtt(
    stack: Stack<'static>,
    mqtt_address: SocketAddr,
    mqtt_user: &'static str,
    mqtt_pwd: &'static str,
    client_id: &'static str,
    command_router: &CommandRouter,
    event_rx: &EventReceiver<AppEvent>,
) -> Result<(), ()> {
    let mut rx_buffer = [0u8; 4096];
    let mut tx_buffer = [0u8; 4096];
    let mut socket = TcpSocket::new(stack, &mut rx_buffer, &mut tx_buffer);
    socket.set_timeout(Some(Duration::from_secs(MQTT_SOCKET_TIMEOUT_SECS as u64)));

    if let SocketAddr::V4(addr) = mqtt_address {
        let ip = addr.ip().octets();
        defmt::info!(
            "mqtt: connecting to {}.{}.{}.{}:{}",
            ip[0],
            ip[1],
            ip[2],
            ip[3],
            addr.port()
        );
    }
    socket
        .connect(mqtt_address)
        .await
        .map_err(|_| defmt::error!("mqtt: TCP connect failed"))?;

    defmt::info!("mqtt: TCP connected");

    let mut buffer = AllocBuffer;
    let mut client = Client::<'_, _, _, 1, 1, 1, 1>::new(&mut buffer);

    // Connect to broker
    match client
        .connect(
            &mut socket,
            &ConnectOptions::new()
                .clean_start()
                .session_expiry_interval(SessionExpiryInterval::Seconds(MQTT_SESSION_EXPIRY_SECS))
                .keep_alive(KeepAlive::Seconds(
                    NonZero::new(MQTT_KEEPALIVE_SECS).unwrap(),
                ))
                .user_name(MqttString::from_str(mqtt_user).unwrap())
                .password(MqttString::from_str(mqtt_pwd).unwrap().into()),
            Some(MqttString::from_str(client_id).unwrap()),
        )
        .await
    {
        Ok(info) => defmt::info!("mqtt: connected, session_present={}", info.session_present),
        Err(e) => {
            defmt::error!("mqtt: CONNECT failed {}", e);
            return Err(());
        }
    }

    let topics = MQTTTopics::new(client_id);
    let discovery_topic = topics.media_player_discovery().unwrap();
    let command_topic = topics.media_player_command().unwrap();
    let state_topic = topics.media_player_state().unwrap();
    let attributes_topic = topics.media_player_attributes().unwrap();
    let entity_id = topics.media_player_entity_id().unwrap();
    let discovery_payload = [
        br#"{"command_topic":""#,
        command_topic.as_str().as_bytes(),
        br#"","state_topic":""#,
        state_topic.as_str().as_bytes(),
        br#"","attributes_topic":""#,
        attributes_topic.as_str().as_bytes(),
        br#"","name":"ESpeaker","unique_id":""#,
        client_id.as_bytes(),
        br#"","entity_id":""#,
        entity_id.as_str().as_bytes(),
        br#"","device":{"identifiers":[""#,
        client_id.as_bytes(),
        br#""],"name":"ESpeaker","manufacturer":"espeaker"},"icon":"mdi:speaker","schema":"json","supported_features":5}"#,
    ];
    let discovery_payload = discovery_payload.concat();
    let discovery_payload = discovery_payload.as_slice();
    let _ = client
        .publish(
            &PublicationOptions::new(TopicReference::Name(
                TopicName::new(MqttString::from_str(discovery_topic.as_str()).unwrap()).unwrap(),
            )),
            rust_mqtt::Bytes::Borrowed(discovery_payload),
        )
        .await;

    let wildcard = topics.subscrive_wildcard().unwrap();
    let mqtt_wildcard = MqttString::from_str(wildcard.as_str()).unwrap();
    let command_topic = TopicFilter::new(mqtt_wildcard).unwrap();
    client
        .subscribe(
            command_topic.as_borrowed().into(),
            SubscriptionOptions::new(),
        )
        .await
        .map_err(|_| defmt::error!("mqtt subscrive failed"))?;

    // Poll loop (TODO: send pings)
    loop {
        let network_fut = client.poll();
        let event_fut = event_rx.receive();
        let ping_fut = Timer::after(Duration::from_secs((MQTT_KEEPALIVE_SECS - 5) as u64));

        match select3(network_fut, event_fut, ping_fut).await {
            Either3::First(Ok(Event::Publish(publish))) => {
                let topic = publish.topic.as_ref().as_str();
                let payload = from_utf8(publish.message.as_bytes()).unwrap_or("");
                defmt::info!("mqtt rx: [{}] {}", topic, payload);

                if let Some(path) = topic.strip_prefix(topics.get_prefix().unwrap().as_str()) {
                    command_router.dispatch(path, payload).await;
                }
            }
            Either3::First(Ok(item)) => {
                defmt::debug!("Received an uncontrolled Ok {}", item)
            }
            Either3::First(Err(e)) => {
                defmt::error!("mqtt: network poll error {}", e);
                break;
            }
            Either3::Second(app_event) => match app_event {
                AppEvent::PlaybackStarted => {
                    let state_topic = topics.media_player_state().unwrap();
                    let topic = TopicName::new(MqttString::from_str(state_topic.as_str()).unwrap())
                        .unwrap();
                    let _ = client
                        .publish(
                            &PublicationOptions::new(TopicReference::Name(topic)),
                            rust_mqtt::Bytes::Borrowed(b"playing" as &[u8]),
                        )
                        .await;
                    let attrs_topic = topics.media_player_attributes().unwrap();
                    let attrs = TopicName::new(MqttString::from_str(attrs_topic.as_str()).unwrap())
                        .unwrap();
                    let payload =
                        br#"{"state":"playing","volume_level":0.7,"media_title":"announcement"}"#;
                    let _ = client
                        .publish(
                            &PublicationOptions::new(TopicReference::Name(attrs)),
                            rust_mqtt::Bytes::Borrowed(payload),
                        )
                        .await;
                }
                AppEvent::PlaybackStopped => {
                    let state_topic = topics.media_player_state().unwrap();
                    let topic = TopicName::new(MqttString::from_str(state_topic.as_str()).unwrap())
                        .unwrap();
                    let _ = client
                        .publish(
                            &PublicationOptions::new(TopicReference::Name(topic)),
                            rust_mqtt::Bytes::Borrowed(b"stopped" as &[u8]),
                        )
                        .await;
                    let attrs_topic = topics.media_player_attributes().unwrap();
                    let attrs = TopicName::new(MqttString::from_str(attrs_topic.as_str()).unwrap())
                        .unwrap();
                    let payload = br#"{"state":"stopped","volume_level":0.7,"media_title":"idle"}"#;
                    let _ = client
                        .publish(
                            &PublicationOptions::new(TopicReference::Name(attrs)),
                            rust_mqtt::Bytes::Borrowed(payload),
                        )
                        .await;
                }
                AppEvent::VolumeChanged(vol) => {
                    let topic_str = topics.volume_changed().unwrap();
                    let topic =
                        TopicName::new(MqttString::from_str(topic_str.as_str()).unwrap()).unwrap();

                    let _ = client
                        .publish(
                            &PublicationOptions::new(TopicReference::Name(topic)),
                            rust_mqtt::Bytes::Borrowed(&[vol]),
                        )
                        .await;
                }
                AppEvent::Key1Pressed => {
                    let topic_str = topics.button_press(1).unwrap();
                    let topic =
                        TopicName::new(MqttString::from_str(topic_str.as_str()).unwrap()).unwrap();
                    let _ = client
                        .publish(
                            &PublicationOptions::new(TopicReference::Name(topic)),
                            rust_mqtt::Bytes::Borrowed(b"pressed" as &[u8]),
                        )
                        .await;
                }
                AppEvent::Key2Pressed => {
                    let topic_str = topics.button_press(2).unwrap();
                    let topic =
                        TopicName::new(MqttString::from_str(topic_str.as_str()).unwrap()).unwrap();
                    let _ = client
                        .publish(
                            &PublicationOptions::new(TopicReference::Name(topic)),
                            rust_mqtt::Bytes::Borrowed(b"pressed" as &[u8]),
                        )
                        .await;
                }
                AppEvent::Key3Pressed => {
                    let topic_str = topics.button_press(3).unwrap();
                    let topic =
                        TopicName::new(MqttString::from_str(topic_str.as_str()).unwrap()).unwrap();
                    let _ = client
                        .publish(
                            &PublicationOptions::new(TopicReference::Name(topic)),
                            rust_mqtt::Bytes::Borrowed(b"pressed" as &[u8]),
                        )
                        .await;
                }
                _ => {}
            },
            Either3::Third(_) => {
                defmt::debug!("mqtt: sending ping");
                if let Err(e) = client.ping().await {
                    defmt::error!("mqtt: ping failed {}", e);
                    break;
                }
            }
        }
    }

    // Disconnect cleanly
    let _ = client.disconnect(&DisconnectOptions::new()).await;
    defmt::info!("mqtt: disconnected");
    Err(())
}

fn parse_audio_command(operation: &str, payload: &str) -> Option<AudioCommand> {
    match operation {
        "play" => Some(AudioCommand::Play),
        "pause" => Some(AudioCommand::Pause),
        "stop" => Some(AudioCommand::Stop),
        "volume" => payload.parse().ok().map(AudioCommand::SetVolume),
        "stream" => heapless::String::try_from(payload)
            .ok()
            .map(AudioCommand::PlayUrl),
        "announce" => heapless::String::try_from(payload)
            .ok()
            .map(AudioCommand::PlayAnnouncement),
        _ => None,
    }
}

fn parse_led_command(operation: &str, payload: &str) -> Option<LedCommand> {
    match operation {
        "clear" => Some(LedCommand::Clear),
        "brightness" => payload.parse().ok().map(LedCommand::Brightness),
        "color" => parse_color(payload).map(LedCommand::SetAll),
        "animate" => parse_animation(payload).map(LedCommand::Loop),
        _ => None,
    }
}

fn parse_color(payload: &str) -> Option<Color> {
    let mut components = payload.split(',');
    let red = components.next()?.parse().ok()?;
    let green = components.next()?.parse().ok()?;
    let blue = components.next()?.parse().ok()?;

    if components.next().is_some() {
        return None;
    }

    Some(Color::new(red, green, blue))
}

fn parse_animation(payload: &str) -> Option<Animation> {
    let (action, params) = payload.split_once("(").unwrap_or((payload, ""));
    match action {
        "rainbow" => {
            let speed = params.split(")").next().unwrap_or("3").parse().ok()?;
            Some(Animation::Rainbow { speed: speed })
        }
        "pulse" => {
            let mut p = params.split(")").next().unwrap_or("").split(",");
            let red = p.next().unwrap_or("0").parse().ok()?;
            let green = p.next().unwrap_or("50").parse().ok()?;
            let blue = p.next().unwrap_or("100").parse().ok()?;
            let speed = p.next().unwrap_or("3").parse().ok()?;

            Some(Animation::Pulse {
                color: Color::new(red, green, blue),
                speed: speed,
            })
        }
        _ => None,
    }
}
