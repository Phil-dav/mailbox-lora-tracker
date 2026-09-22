use core::fmt::Write as _;
use esp_idf_svc::hal::delay::FreeRtos;
use esp_idf_svc::hal::gpio::PinDriver;
use esp_idf_svc::hal::i2c::{I2cConfig, I2cDriver};
use esp_idf_svc::hal::peripherals::Peripherals;
use esp_idf_svc::hal::spi::{config::Config as SpiConfig, SpiDeviceDriver, SpiDriverConfig};
use esp_idf_svc::hal::units::FromValueType;
use heapless::String;
use log::{info, warn};

use embedded_graphics::{
    mono_font::{ascii::FONT_6X10, MonoTextStyle},
    pixelcolor::BinaryColor,
    prelude::*,
    text::Text,
};
use ssd1306::{prelude::*, I2CDisplayInterface, Ssd1306};

// Brochage TTGO/Heltec LoRa32 (V1/V2) le plus courant. À ajuster une fois le
// modèle exact confirmé sur la sérigraphie de la carte :
// - LED intégrée : GPIO25 (absente sur certaines variantes Heltec)
// - Écran OLED (I2C) : SDA=GPIO4, SCL=GPIO15, RST=GPIO16
// - Module LoRa SX127x (SPI) : SCK=GPIO5, MISO=GPIO19, MOSI=GPIO27, NSS=GPIO18, RST=GPIO14

/// Lit le registre "version" (adresse 0x42) du SX127x. La valeur attendue est
/// 0x12 (SX1276/77/78/79) si la puce répond correctement sur le bus SPI —
/// sert juste à vérifier le câblage/driver, sans toucher à la radio elle-même.
fn lire_version_sx127x<'a>(spi: &mut SpiDeviceDriver<'a, esp_idf_svc::hal::spi::SpiDriver<'a>>) -> anyhow::Result<u8> {
    // Bit de poids fort à 0 = lecture ; l'octet suivant (émis à 0x00) reçoit la
    // valeur du registre, décalée par le SPI pendant qu'on envoie l'adresse.
    let mut buf = [0x42 & 0x7F, 0x00];
    spi.transfer_in_place(&mut buf)?;
    Ok(buf[1])
}
fn main() -> anyhow::Result<()> {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    info!("Démarrage du programme");

    let peripherals = Peripherals::take()?;
    let mut led = PinDriver::output(peripherals.pins.gpio25)?;

    // L'écran OLED de ces cartes a besoin d'une impulsion de reset matérielle
    // sur sa broche dédiée avant de répondre sur le bus I2C.
    let mut oled_rst = PinDriver::output(peripherals.pins.gpio16)?;
    oled_rst.set_high()?;
    FreeRtos::delay_ms(10);
    oled_rst.set_low()?;
    FreeRtos::delay_ms(10);
    oled_rst.set_high()?;
    FreeRtos::delay_ms(10);

    let sda = peripherals.pins.gpio4;
    let scl = peripherals.pins.gpio15;
    let i2c_config = I2cConfig::new().baudrate(400.kHz().into());
    let i2c = I2cDriver::new(peripherals.i2c0, sda, scl, &i2c_config)?;

    let interface = I2CDisplayInterface::new(i2c);
    let mut display = Ssd1306::new(interface, DisplaySize128x64, DisplayRotation::Rotate0)
        .into_buffered_graphics_mode();

    let mut ecran_disponible = true;
    if let Err(e) = display.init() {
        warn!("Écran OLED : initialisation échouée : {:?}", e);
        ecran_disponible = false;
    }

    // --- Test SPI du module LoRa (lecture du registre version, avant toute radio) ---
    let mut lora_rst = PinDriver::output(peripherals.pins.gpio14)?;
    lora_rst.set_low()?;
    FreeRtos::delay_ms(10);
    lora_rst.set_high()?;
    FreeRtos::delay_ms(10);

    let sclk = peripherals.pins.gpio5;
    let sdo = peripherals.pins.gpio27; // MOSI
    let sdi = peripherals.pins.gpio19; // MISO
    let cs = peripherals.pins.gpio18;
    let spi_config = SpiConfig::new().baudrate(1.MHz().into());
    let mut lora_spi = SpiDeviceDriver::new_single(
        peripherals.spi2,
        sclk,
        sdo,
        Some(sdi),
        Some(cs),
        &SpiDriverConfig::new(),
        &spi_config,
    )?;

    let version_lora = match lire_version_sx127x(&mut lora_spi) {
        Ok(v) => {
            if v == 0x12 {
                info!("SX127x : registre version = 0x{:02X} (OK, puce détectée)", v);
            } else {
                warn!(
                    "SX127x : registre version = 0x{:02X} (attendu 0x12 -> câblage/broches à vérifier)",
                    v
                );
            }
            v
        }
        Err(e) => {
            warn!("SX127x : lecture SPI échouée : {:?}", e);
            0
        }
    };

    let style = MonoTextStyle::new(&FONT_6X10, BinaryColor::On);
    let mut compteur: u32 = 0;

    loop {
        led.set_high()?;
        FreeRtos::delay_ms(500);
        led.set_low()?;
        FreeRtos::delay_ms(500);

        compteur += 1;
        info!("Compteur : {}", compteur);

        if ecran_disponible {
            let mut ligne: String<32> = String::new();
            write!(ligne, "Compteur : {}", compteur).ok();

            let mut ligne_lora: String<32> = String::new();
            write!(
                ligne_lora,
                "LoRa ver: 0x{:02X} {}",
                version_lora,
                if version_lora == 0x12 { "OK" } else { "?" }
            )
            .ok();

            display.clear(BinaryColor::Off).ok();
            Text::new("Test carte LoRa", Point::new(0, 12), style)
                .draw(&mut display)
                .ok();
            Text::new(&ligne, Point::new(0, 32), style)
                .draw(&mut display)
                .ok();
            Text::new(&ligne_lora, Point::new(0, 48), style)
                .draw(&mut display)
                .ok();
            if let Err(e) = display.flush() {
                warn!("Écran OLED : flush échoué : {:?}", e);
            }
        }
    }
}
