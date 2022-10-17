# ESP32C3 Bare Metal OTA Experiment

## About

This is an experiment to do an OTA update on ESP32-C3 in bare-metal.

Most things are hard-coded (i.e. the adresses of the ota partition and the ota slots).

## Preparation

First set the ENV variables `SSID` and `PASSWORD` to match the settings of your WiFi access point.
Your PC needs to be connected to the same access point.

Change the address of your computer in `main.rs` as needed. (`HOST`)

Make sure you have installed [devserver](https://crates.io/crates/devserver)

Change `THIS_VERSION` in `main.rs` to `2`. Compile the application via `cargo build --release`.

Perform these steps
- `esptool --chip esp32c3 elf2image target\riscv32imac-unknown-none-elf\release\esp32c3_ota_experiment`
- `cp target\riscv32imac-unknown-none-elf\release\esp32c3_ota_experiment.bin firmware.bin`

Change `THIS_VERSION` in `main.rs` back to `1`.

## Run

Run `devserver --address 192.168.2.125:8080` in a separate terminal (in the project directory). Make sure to replace `192.168.2.125` by the IP of your PC.

To avoid problems run `esptool erase_flash` first. Now run the application via `cargo run --release`

The application should connect to your PC, pick up `current.txt` and see it's own version (1) is below what is available online (2).
Now it will download `firmware.bin` and flash it. After that it will set the OTA partition to use.

In this experiment the reset isn't done automatically. Reset the ESP32-C3 and see the new version boot.
The new version will see there is no later version online to flash.

## Please Note

Make sure to use the provided bootloader and partition table.

i.e. when flashing manually with espflash use `espflash --bootloader bootloader.bin --partition-table partitions.csv`
