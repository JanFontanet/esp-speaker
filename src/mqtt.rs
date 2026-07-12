use core::net::{Ipv4Addr, SocketAddr};
use core::num::NonZero;

use embassy_executor::Spawner;
use embassy_net::{Stack, tcp::TcpSocket};
use embassy_time::{Duration, Timer};
use rust_mqtt::config::{KeepAlive, SessionExpiryInterval};
use rust_mqtt::{
    buffer::AllocBuffer,
    client::{
        Client,
        event::Event,
        options::{ConnectOptions, DisconnectOptions},
    },
};

use crate::wifi::DeviceConfig;

#[derive(Debug, defmt::Format)]
pub enum MQTTError {
    ConnectionError,
    SpawnError,
}

pub fn mqtt_spawn(spawner: &Spawner, stack: Stack<'static>, config: &DeviceConfig) {
    let mqtt_address: Ipv4Addr = config.mqtt_address().parse().unwrap();
    let addr = SocketAddr::new(mqtt_address.into(), 1883);

    spawner.spawn(mqtt_task(stack, addr).unwrap());
}

#[embassy_executor::task]
async fn mqtt_task(stack: Stack<'static>, mqtt_address: SocketAddr) {
    loop {
        if let Err(_e) = run_mqtt(stack, mqtt_address).await {
            defmt::error!("MQTT error, reconnecting in 5s...");
            Timer::after(Duration::from_secs(5)).await;
        }
    }
}

async fn run_mqtt(stack: Stack<'static>, mqtt_address: SocketAddr) -> Result<(), ()> {
    let mut rx_buffer = [0u8; 4096];
    let mut tx_buffer = [0u8; 4096];
    let mut socket = TcpSocket::new(stack, &mut rx_buffer, &mut tx_buffer);
    socket.set_timeout(Some(Duration::from_secs(10)));

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
            socket,
            &ConnectOptions::new()
                .clean_start()
                .session_expiry_interval(SessionExpiryInterval::Seconds(60))
                .keep_alive(KeepAlive::Seconds(NonZero::new(30).unwrap())),
            None, // client identifier
        )
        .await
    {
        Ok(info) => defmt::info!("mqtt: connected, session_present={}", info.session_present),
        Err(_) => {
            defmt::error!("mqtt: CONNECT failed");
            return Err(());
        }
    }

    // Poll loop
    loop {
        match client.poll().await {
            Ok(Event::Publish(publish)) => {
                let topic = publish.topic.as_ref().as_str();
                let payload = core::str::from_utf8(publish.message.as_bytes()).unwrap_or("");

                defmt::info!("mqtt: [{}] {}", topic, payload);

                match topic {
                    "topic/thing" => handle_thing(payload),
                    "topic/command" => handle_command(payload),
                    other => defmt::warn!("mqtt: unknown topic '{}'", other),
                }
            }
            Ok(_) => {} // other events (ping, suback, etc.)
            Err(_e) => {
                defmt::error!("mqtt: poll error");
                break;
            }
        }
    }

    // Disconnect cleanly
    let _ = client.disconnect(&DisconnectOptions::new()).await;
    defmt::info!("mqtt: disconnected");
    Err(())
}

fn handle_thing(payload: &str) {
    defmt::info!("mqtt: thing payload = {}", payload);
    // TODO: implement
}

fn handle_command(payload: &str) {
    defmt::info!("mqtt: command payload = {}", payload);
    // TODO: implement
}
