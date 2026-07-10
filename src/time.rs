//! SNTP time sync: fetch UTC over the network and store it in the RTC.

use crate::board::I2cBus;
use crate::rtc::{DateTime, Rtc};
use embassy_executor::Spawner;
use embassy_net::dns::DnsQueryType;
use embassy_net::udp::{PacketMetadata, UdpSocket};
use embassy_net::{IpEndpoint, Stack};
use embassy_time::{Duration, Timer, with_timeout};

const NTP_SERVER: &str = "pool.ntp.org";
const NTP_PORT: u16 = 123;
const LOCAL_PORT: u16 = 50123;
/// Seconds between the NTP epoch (1900-01-01) and the Unix epoch (1970-01-01).
const NTP_UNIX_DELTA: u64 = 2_208_988_800;
/// How often to re-sync once we're up.
const RESYNC_INTERVAL: Duration = Duration::from_secs(3600);

/// Spawn the time task: sync once now, then periodically.
pub fn time_spawn(spawner: &Spawner, stack: Stack<'static>, bus: &'static I2cBus) {
    spawner.spawn(time_task(stack, bus).unwrap());
}

#[embassy_executor::task]
async fn time_task(stack: Stack<'static>, bus: &'static I2cBus) {
    loop {
        sync(stack, bus).await;
        Timer::after(RESYNC_INTERVAL).await;
    }
}

/// Query SNTP and store the result in the RTC. Best-effort: logs on failure.
pub async fn sync(stack: Stack<'static>, bus: &'static I2cBus) {
    match sntp_unix(stack).await {
        Ok(unix) => {
            let dt = datetime_from_unix(unix);
            defmt::info!(
                "time: SNTP {}-{}-{} {}:{}:{} UTC",
                dt.year,
                dt.month,
                dt.day,
                dt.hour,
                dt.minute,
                dt.second
            );
            let mut rtc = Rtc::new(bus);
            match rtc.set(&dt).await {
                Ok(()) => defmt::info!("time: RTC updated"),
                Err(e) => defmt::error!("time: RTC set failed: {}", e),
            }
        }
        Err(e) => defmt::warn!("time: SNTP failed: {}", e),
    }
}

/// Query SNTP and return Unix time (seconds since 1970-01-01 UTC).
async fn sntp_unix(stack: Stack<'static>) -> Result<u64, &'static str> {
    let addrs = stack
        .dns_query(NTP_SERVER, DnsQueryType::A)
        .await
        .map_err(|_| "DNS query failed")?;
    let server = *addrs.first().ok_or("no DNS result")?;

    let mut rx_meta = [PacketMetadata::EMPTY; 4];
    let mut rx_buf = [0u8; 128];
    let mut tx_meta = [PacketMetadata::EMPTY; 4];
    let mut tx_buf = [0u8; 128];
    let mut socket = UdpSocket::new(stack, &mut rx_meta, &mut rx_buf, &mut tx_meta, &mut tx_buf);
    socket.bind(LOCAL_PORT).map_err(|_| "UDP bind failed")?;

    // NTP request: LI=0, VN=3, Mode=3 (client); the rest zero.
    let mut request = [0u8; 48];
    request[0] = 0x1B;
    socket
        .send_to(&request, IpEndpoint::new(server, NTP_PORT))
        .await
        .map_err(|_| "UDP send failed")?;

    let mut response = [0u8; 48];
    let (n, _) = with_timeout(Duration::from_secs(5), socket.recv_from(&mut response))
        .await
        .map_err(|_| "SNTP timeout")?
        .map_err(|_| "UDP recv failed")?;
    if n < 48 {
        return Err("short SNTP response");
    }

    // Transmit timestamp (seconds) is a big-endian u32 at bytes 40..44.
    let secs_1900 =
        u32::from_be_bytes([response[40], response[41], response[42], response[43]]) as u64;
    secs_1900
        .checked_sub(NTP_UNIX_DELTA)
        .ok_or("invalid SNTP time")
}

/// Convert Unix seconds to a UTC calendar date (Howard Hinnant's algorithm).
fn datetime_from_unix(unix: u64) -> DateTime {
    let days = (unix / 86_400) as i64;
    let secs = unix % 86_400;
    let hour = (secs / 3600) as u8;
    let minute = ((secs % 3600) / 60) as u8;
    let second = (secs % 60) as u8;

    let z = days + 719_468;
    let era = (if z >= 0 { z } else { z - 146_096 }) / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let day = (doy - (153 * mp + 2) / 5 + 1) as u8; // [1, 31]
    let month = (if mp < 10 { mp + 3 } else { mp - 9 }) as u8; // [1, 12]
    let leap_adjust: i64 = if month <= 2 { 1 } else { 0 };
    let year = (y + leap_adjust) as u16;

    DateTime {
        year,
        month,
        day,
        hour,
        minute,
        second,
    }
}
