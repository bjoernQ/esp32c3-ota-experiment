#![no_std]
#![no_main]

use core::str::from_utf8;

use embedded_svc::ipv4::Interface;
use embedded_svc::wifi::{ClientConfiguration, Configuration, Wifi};
use esp32c3_hal::clock::{ClockControl, CpuClock};
use esp32c3_hal::peripherals::Peripherals;
use esp32c3_hal::Rtc;
use esp32c3_hal::{prelude::*, Rng};
use esp_println::logger::init_logger;
use esp_println::{print, println};
use esp_wifi::wifi::utils::create_network_interface;
use esp_wifi::wifi::WifiMode;
use esp_wifi::wifi_interface::WifiStack;

use esp_backtrace as _;
use smoltcp::wire::Ipv4Address;

use crate::tiny_http::HttpClient;
use embedded_io::blocking::Read;

use smoltcp::iface::SocketStorage;

use partitions_macro::partition_offset;

mod ota;
mod tiny_http;

const THIS_VERSION: u32 = 1;
const PORT: u16 = 8080;

const SSID: &str = env!("SSID");
const PASSWORD: &str = env!("PASSWORD");
const HOST_IP: &str = env!("HOST_IP");

const OTA_0_OFFSET: u32 = partition_offset!("ota_0");
const OTA_1_OFFSET: u32 = partition_offset!("ota_1");
const OTA_OFFSETS: [u32; 2] = [OTA_0_OFFSET, OTA_1_OFFSET];

#[entry]
fn main() -> ! {
    init_logger(log::LevelFilter::Info);
    esp_wifi::init_heap();

    let peripherals = Peripherals::take();
    let mut system = peripherals.SYSTEM.split();
    let clocks = ClockControl::configure(system.clock_control, CpuClock::Clock160MHz).freeze();

    let mut rtc = Rtc::new(peripherals.RTC_CNTL);

    // Disable watchdog timers
    rtc.swd.disable();
    rtc.rwdt.disable();

    let (wifi, _) = peripherals.RADIO.split();
    let mut socket_set_entries: [SocketStorage; 3] = Default::default();
    let (iface, device, mut controller, sockets) =
        create_network_interface(wifi, WifiMode::Sta, &mut socket_set_entries);
    let wifi_stack = WifiStack::new(iface, device, sockets, current_millis);

    use esp32c3_hal::systimer::SystemTimer;
    let syst = SystemTimer::new(peripherals.SYSTIMER);
    esp_wifi::initialize(
        syst.alarm0,
        Rng::new(peripherals.RNG),
        system.radio_clock_control,
        &clocks,
    )
    .unwrap();

    println!("Call wifi_connect");
    let client_config = Configuration::Client(ClientConfiguration {
        ssid: SSID.into(),
        password: PASSWORD.into(),
        ..Default::default()
    });
    controller.set_configuration(&client_config).unwrap();
    controller.start().unwrap();
    controller.connect().unwrap();

    println!("Wait to get connected");
    loop {
        let res = controller.is_connected();
        match res {
            Ok(connected) => {
                if connected {
                    break;
                }
            }
            Err(err) => {
                println!("{:?}", err);
                loop {}
            }
        }
    }

    // wait for getting an ip address
    println!("Wait to get an ip address");
    loop {
        wifi_stack.work();

        if wifi_stack.is_iface_up() {
            println!("Got ip {:?}", wifi_stack.get_ip_info());
            break;
        }
    }

    println!("We are connected!");
    println!("Start busy loop on main");

    let mut storage = esp_storage::FlashStorage::new();
    let mut ota = ota::Ota::new(&mut storage);
    let current_slot = ota.current_slot();
    println!("Current Slot: {:?}", current_slot);

    let new_slot = current_slot.next();

    let mut rx_buffer = [0u8; 1536];
    let mut tx_buffer = [0u8; 1536];
    let mut socket = wifi_stack.get_socket(&mut rx_buffer, &mut tx_buffer);
    socket.open(parse_ip(HOST_IP), PORT).unwrap();
    let mut client = HttpClient::new("localhost", socket);
    let mut response = client
        .get::<0, 0>("/current.txt", None, None::<&[&str; 0]>)
        .unwrap();
    let mut buffer = [0u8; 128];
    let len = response.read(&mut buffer).unwrap();

    let version_str = from_utf8(&buffer[..len]).unwrap().trim();
    println!("Update firmware version '{}'", version_str);

    let version: u32 = version_str.parse().unwrap();
    println!(
        "Update firmware version {} - my version {}",
        version, THIS_VERSION
    );

    response.finish();
    let mut socket = client.finish();
    socket.disconnect();

    if version > THIS_VERSION {
        println!("Going to update");

        let mut flash_addr = OTA_OFFSETS[new_slot.number()];

        // do the update
        socket.open(parse_ip(HOST_IP), PORT).unwrap();
        let mut client = HttpClient::new("localhost", socket);

        let mut response = client
            .get::<0, 0>("/firmware.bin", None, None::<&[&str; 0]>)
            .unwrap();

        let mut response_data = [0u8; 4096]; // max one sector
        loop {
            let res = response.read(&mut response_data);

            match res {
                Ok(len) => {
                    if len > 0 {
                        print!(".");
                        ota.write(flash_addr, &response_data[..len]).unwrap();
                        flash_addr += len as u32;
                    }
                }
                Err(_) => break,
            }
        }
        println!();
        println!("Flashing done");

        println!("Setting the new OTA slot to {:?}", new_slot);
        ota.set_current_slot(new_slot);
    }

    ota.free();

    println!(
        "All done - going idle now - manually reset to boot into the (might be) newly installed firmware"
    );

    loop {}
}

pub fn current_millis() -> u64 {
    esp_wifi::timer::get_systimer_count() * 1000 / esp_wifi::timer::TICKS_PER_SECOND
}

fn parse_ip(ip: &str) -> Ipv4Address {
    let mut result = [0u8; 4];
    for (idx, octet) in ip.split(".").into_iter().enumerate() {
        result[idx] = u8::from_str_radix(octet, 10).unwrap();
    }

    Ipv4Address::from_bytes(&result)
}
