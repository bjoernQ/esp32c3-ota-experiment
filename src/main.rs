#![no_std]
#![no_main]

use embedded_svc::wifi::{
    ClientConfiguration, ClientConnectionStatus, ClientIpStatus, ClientStatus, Configuration,
    Status, Wifi,
};
use esp32c3_hal::RtcCntl;
use esp_println::println;
use esp_wifi::wifi::initialize;
use esp_wifi::wifi::utils::create_network_interface;
use esp_wifi::wifi_interface::timestamp;
use esp_wifi::{create_network_stack_storage, network_stack_storage};

use esp32c3_hal::pac::Peripherals;
use esp_backtrace as _;
use riscv_rt::entry;
use smoltcp::wire::Ipv4Address;

use crate::ota::Slot;
use crate::tiny_http::{Buffer, HttpClient, PollResult};

extern crate alloc;

mod ota;
mod tiny_http;

const THIS_VERSION: u32 = 1;
const HOST: Ipv4Address = Ipv4Address::new(192, 168, 2, 125); // 192.168.2.125 ... change if needed!
const PORT: u16 = 8080;

const SSID: &str = env!("SSID");
const PASSWORD: &str = env!("PASSWORD");

#[entry]
fn main() -> ! {
    let mut peripherals = Peripherals::take().unwrap();

    let mut rtc_cntl = RtcCntl::new(peripherals.RTC_CNTL);

    // Disable watchdog timers
    rtc_cntl.set_super_wdt_enable(false);
    rtc_cntl.set_wdt_enable(false);

    let mut storage = create_network_stack_storage!(3, 8, 1);
    let ethernet = create_network_interface(network_stack_storage!(storage));
    let mut wifi_interface = esp_wifi::wifi_interface::Wifi::new(ethernet);

    init_logger();

    initialize(
        &mut peripherals.SYSTIMER,
        &mut peripherals.INTERRUPT_CORE0,
        peripherals.RNG,
    )
    .unwrap();

    println!("{:?}", wifi_interface.get_status());

    println!("Call wifi_connect");
    let client_config = Configuration::Client(ClientConfiguration {
        ssid: SSID.into(),
        password: PASSWORD.into(),
        ..Default::default()
    });
    let res = wifi_interface.set_configuration(&client_config);
    println!("wifi_connect returned {:?}", res);

    println!("{:?}", wifi_interface.get_capabilities());
    println!("{:?}", wifi_interface.get_status());

    // wait to get connected
    loop {
        if let Status(ClientStatus::Started(_), _) = wifi_interface.get_status() {
            break;
        }
    }
    println!("{:?}", wifi_interface.get_status());

    // wait to get connected and have an ip
    loop {
        wifi_interface.poll_dhcp().unwrap();

        wifi_interface
            .network_interface()
            .poll(timestamp())
            .unwrap();

        if let Status(
            ClientStatus::Started(ClientConnectionStatus::Connected(ClientIpStatus::Done(config))),
            _,
        ) = wifi_interface.get_status()
        {
            println!("got ip {:?}", config);
            break;
        }
    }

    println!("We are connected!");
    println!("Start busy loop on main");

    let mut storage = esp_storage::FlashStorage::new();
    let mut ota = ota::Ota::new(&mut storage);
    let current_slot = ota.current_slot();
    println!("Current Slot: {:?}", current_slot);

    let new_slot = if current_slot == Slot::None || current_slot == Slot::Slot1 {
        Slot::Slot0
    } else {
        Slot::Slot1
    };

    let mut client = Some(HttpClient::new(wifi_interface, current_millis));

    let mut response = client.unwrap().get(HOST, PORT, "/current.txt", "localhost");

    let mut response_data: Buffer<1024> = Buffer::new();

    loop {
        let res = response.poll();

        match res {
            PollResult::None => (),
            PollResult::Data(buffer) => {
                response_data.push(buffer.slice());
            }
            PollResult::Done => {
                client = Some(response.finalize());
                break;
            }
            PollResult::Err => {
                println!("Ooops some error occured!");
            }
        }
    }

    let version_str = response_data.next_line().unwrap();
    println!("Update firmware version {}", version_str);

    let version: u32 = version_str.parse().unwrap();
    println!(
        "Update firmware version {} - my version {}",
        version, THIS_VERSION
    );

    if version > THIS_VERSION {
        println!("going to update");

        let mut flash_addr = if new_slot == Slot::Slot0 {
            0x110000
        } else {
            0x210000
        };

        // do the update
        let mut response = client
            .unwrap()
            .get(HOST, PORT, "/firmware.bin", "localhost");

        let mut response_data: Buffer<4096> = Buffer::new(); // one sector
        loop {
            let res = response.poll();

            match res {
                PollResult::None => (),
                PollResult::Data(buffer) => {
                    let slice = buffer.slice();
                    let written = response_data.push(slice);

                    if response_data.is_full() {
                        println!(
                            "filled the buffer ... write {} bytes to {:x}",
                            response_data.slice().len(),
                            flash_addr
                        );

                        ota.write(flash_addr, response_data.slice()).unwrap();
                        flash_addr += 4096;
                        response_data.clear();
                    }

                    if written != slice.len() {
                        let buffer = buffer.split_right(written);

                        // let's hope our buffer is large enough
                        response_data.push(buffer.slice());
                    }
                }
                PollResult::Done => {
                    response.finalize();
                    break;
                }
                PollResult::Err => {
                    println!("Ooops some error occured!");
                }
            }
        }

        // write the remainder - make sure to align to 4 bytes
        println!(
            "last sector to write ... write {} bytes to {:x}",
            response_data.slice().len(),
            flash_addr
        );
        ota.write(flash_addr, response_data.slice()).unwrap();

        println!("Setting the new OTA slot to {:?}", new_slot);
        ota.set_current_slot(new_slot);
    }

    println!(
        "All done - going idle now - manually reset to boot into the (might be) newly installed firmware"
    );

    loop {}
}

pub fn current_millis() -> u32 {
    (esp_wifi::timer::get_systimer_count() * 1000 / esp_wifi::timer::TICKS_PER_SECOND) as u32
}

pub fn wait_ms(ms: u32) {
    let started = current_millis();
    while current_millis() < started + ms {
        // nothing
    }
}

pub fn init_logger() {
    unsafe {
        log::set_logger_racy(&LOGGER).unwrap();
        log::set_max_level(log::LevelFilter::Info);
    }
}

static LOGGER: SimpleLogger = SimpleLogger;
struct SimpleLogger;

impl log::Log for SimpleLogger {
    fn enabled(&self, _metadata: &log::Metadata) -> bool {
        true
    }

    fn log(&self, record: &log::Record) {
        println!("{} - {}", record.level(), record.args());
    }

    fn flush(&self) {}
}
