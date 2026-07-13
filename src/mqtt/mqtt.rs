use core::net::{Ipv4Addr, SocketAddr};
use core::num::NonZero;
use core::str::from_utf8;

use embassy_executor::Spawner;
use embassy_futures::select::{Either, select};
use embassy_net::{Stack, tcp::TcpSocket};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::{Receiver, Sender};
use embassy_time::{Duration, Timer};
use rust_mqtt::client::options::{PublicationOptions, SubscriptionOptions, TopicReference};
use rust_mqtt::config::{KeepAlive, SessionExpiryInterval};
use rust_mqtt::types::{MqttString, TopicName};
use rust_mqtt::{
    buffer::AllocBuffer,
    client::{
        Client,
        event::Event,
        options::{ConnectOptions, DisconnectOptions},
    },
};

use crate::config::{
    CHANNEL_SIZE, MQTT_KEEPALIVE_SECS, MQTT_PORT, MQTT_RECONNECT_DELAY_SECS,
    MQTT_SESSION_EXPIRY_SECS, MQTT_SOCKET_TIMEOUT_SECS, MQTT_TOPIC_COMMANDS, MQTT_TOPIC_STATUS,
    MQTT_TOPIC_VOLUME,
};
use crate::mqtt::msg_protocol::{AppEvent, AudioCommand};
use crate::wifi::DeviceConfig;

pub type CmdSender = Sender<'static, CriticalSectionRawMutex, AudioCommand, CHANNEL_SIZE>;
pub type EventReceiver<AppEvent> =
    Receiver<'static, CriticalSectionRawMutex, AppEvent, CHANNEL_SIZE>;

#[macro_export]
macro_rules! route_mqtt {
    // 1. Entry point of the macro: processes a list of routing mappings
    (
        $topic:expr, $payload:expr, $cmd_tx:expr;
        {
            $( $mqtt_topic:literal => $variant:ident $( ( $parse_fn:path ) )? ),* $(,)?
        }
    ) => {
        match $topic {
            $(
                // For each rule, we route to our internal helper arm (prefixed with @route)
                $mqtt_topic => {
                    $crate::route_mqtt!(
                        @route
                        $payload,
                        $cmd_tx,
                        $variant
                        $( ( $parse_fn ) )?
                    );
                }
            )*
            _ => {
                defmt::warn!("mqtt: unhandled topic '{}'", $topic);
            }
        }
    };

    // 2. Internal helper: Matches commands WITH a parsing function
    ( @route $payload:expr, $cmd_tx:expr, $variant:ident ( $parse_fn:path ) ) => {
        if let Some(parsed_val) = $parse_fn($payload) {
            let cmd = AudioCommand::$variant(parsed_val);
            let _ = $cmd_tx.send(cmd).await;
        } else {
            defmt::warn!(
                "Failed to parse payload '{}' for command variant '{}'",
                $payload,
                stringify!($variant)
            );
        }
    };

    // 3. Internal helper: Matches simple commands (WITHOUT a parsing function)
    ( @route $payload:expr, $cmd_tx:expr, $variant:ident ) => {
        let _ = $cmd_tx.send(AudioCommand::$variant).await;
    };
}
#[derive(Debug, defmt::Format)]
pub enum MQTTError {
    ConnectionError,
    SpawnError,
}

pub fn mqtt_spawn(
    spawner: &Spawner,
    stack: Stack<'static>,
    config: &DeviceConfig,
    client_id: &'static str,
    cmd_tx: CmdSender,
    event_rx: EventReceiver<AppEvent>,
) {
    let mqtt_address: Ipv4Addr = config.mqtt_address().parse().unwrap();
    let addr = SocketAddr::new(mqtt_address.into(), MQTT_PORT);

    spawner.spawn(mqtt_task(stack, addr, client_id, cmd_tx, event_rx).unwrap());
}

#[embassy_executor::task]
async fn mqtt_task(
    stack: Stack<'static>,
    mqtt_address: SocketAddr,
    client_id: &'static str,
    cmd_tx: CmdSender,
    event_rx: EventReceiver<AppEvent>,
) {
    loop {
        if let Err(_e) = run_mqtt(stack, mqtt_address, client_id, &cmd_tx, &event_rx).await {
            defmt::error!("MQTT error, reconnecting in 5s...");
            Timer::after(Duration::from_secs(MQTT_RECONNECT_DELAY_SECS as u64)).await;
        }
    }
}

async fn run_mqtt(
    stack: Stack<'static>,
    mqtt_address: SocketAddr,
    client_id: &'static str,
    cmd_tx: &CmdSender,
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
                )),
            Some(MqttString::from_str(client_id).unwrap()),
        )
        .await
    {
        Ok(info) => defmt::info!("mqtt: connected, session_present={}", info.session_present),
        Err(_) => {
            defmt::error!("mqtt: CONNECT failed");
            return Err(());
        }
    }

    let command_topic = TopicName::new(MqttString::from_str(MQTT_TOPIC_COMMANDS).unwrap()).unwrap();
    client
        .subscribe(
            command_topic.as_borrowed().into(),
            SubscriptionOptions::new(),
        )
        .await
        .map_err(|_| defmt::error!("mqtt subscrive failed"))?;
    // Poll loop
    loop {
        let network_fut = client.poll();
        let event_fut = event_rx.receive();

        match select(network_fut, event_fut).await {
            Either::First(Ok(Event::Publish(publish))) => {
                let topic = publish.topic.as_ref().as_str();
                let payload = from_utf8(publish.message.as_bytes()).unwrap_or("");
                defmt::info!("mqtt rx: [{}] {}", topic, payload);

                route_mqtt!(
                    topic, payload, cmd_tx;
                    {
                        "speaker/commands/play"   => Play,
                        "speaker/commands/pause"  => Pause,
                        "speaker/commands/stop"   => Stop,
                        "speaker/commands/volume" => SetVolume(parse_u8),
                        "speaker/commands/stream" => PlayUrl(parse_str),
                    }
                );
            }
            Either::First(Ok(_)) => {}
            Either::First(Err(_e)) => {
                defmt::error!("mqtt: network poll error");
                break;
            }
            Either::Second(app_event) => match app_event {
                AppEvent::PlaybackStarted => {
                    let topic =
                        TopicName::new(MqttString::from_str(MQTT_TOPIC_STATUS).unwrap()).unwrap();
                    let _ = client
                        .publish(
                            &PublicationOptions::new(TopicReference::Name(topic)),
                            rust_mqtt::Bytes::Borrowed(b"playing" as &[u8]),
                        )
                        .await;
                }
                AppEvent::PlaybackStopped => {
                    let topic =
                        TopicName::new(MqttString::from_str(MQTT_TOPIC_STATUS).unwrap()).unwrap();
                    let _ = client
                        .publish(
                            &PublicationOptions::new(TopicReference::Name(topic)),
                            rust_mqtt::Bytes::Borrowed(b"stopped" as &[u8]),
                        )
                        .await;
                }
                AppEvent::VolumeChanged(vol) => {
                    let topic =
                        TopicName::new(MqttString::from_str(MQTT_TOPIC_VOLUME).unwrap()).unwrap();

                    let _ = client
                        .publish(
                            &PublicationOptions::new(TopicReference::Name(topic)),
                            rust_mqtt::Bytes::Borrowed(&[vol]),
                        )
                        .await;
                }
                _ => {}
            },
            // _ => {}
        }
    }

    // Disconnect cleanly
    let _ = client.disconnect(&DisconnectOptions::new()).await;
    defmt::info!("mqtt: disconnected");
    Err(())
}

fn parse_u8(payload: &str) -> Option<u8> {
    payload.parse::<u8>().ok()
}

fn parse_str(_payload: &str) -> Option<&'static str> {
    // In a real application, you'd map this to a static or copy it to a heapless::String.
    // For simplicity, we leak/cast or use an existing pre-allocated static pool. TODO
    Some(core::hint::black_box("http://example.com/stream.mp3"))
}
