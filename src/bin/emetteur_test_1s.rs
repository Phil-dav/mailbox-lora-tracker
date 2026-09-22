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

use lora_boite_lettres::sx127x::{Reception, Sx127x};

// Test rapide avec antennes montées : envoie un PING toutes les secondes et
// attend l'accusé de réception, écran activé pour voir les résultats en
// direct sans moniteur série. Basé sur emetteur_test_portee.rs (test longue
// durée à 4h d'intervalle), juste l'intervalle qui change ici.
// LED : allumée 0,5s à chaque ACK reçu, rien si le cycle échoue —
// tant qu'elle continue à flasher, la communication fonctionne ; dès
// qu'elle s'arrête de flasher, la communication est coupée.
const FREQUENCE_HZ: u32 = 433_000_000;
const DELAI_ENTRE_ENVOIS_MS: u32 = 1000; // 1 seconde

const TOURS_ATTENTE_ACK: u32 = 100; // 100 x 20ms = 2s
const DELAI_TOUR_MS: u32 = 20;
const MAX_ESSAIS: u32 = 3;

fn main() -> anyhow::Result<()> {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();
    info!("Démarrage EMETTEUR (test rapide, envoi toutes les secondes)");

    let peripherals = Peripherals::take()?;
    let mut led = PinDriver::output(peripherals.pins.gpio25)?;

    // Test diagnostic temporaire : GPIO25 éteinte 5s puis allumée 5s, une
    // seule fois, pour lever tout doute sur la LED physique concernée.
    info!("DIAGNOSTIC LED : GPIO25 ETEINTE pendant 5s");
    led.set_low()?;
    FreeRtos::delay_ms(5000);
    info!("DIAGNOSTIC LED : GPIO25 ALLUMEE pendant 5s");
    led.set_high()?;
    FreeRtos::delay_ms(5000);
    led.set_low()?;
    info!("Fin du diagnostic LED, démarrage normal du programme");

    // --- Vext + écran OLED ---
    let mut vext = PinDriver::output(peripherals.pins.gpio21)?;
    vext.set_low()?; // actif à l'état bas sur les cartes Heltec

    let mut oled_rst = PinDriver::output(peripherals.pins.gpio16)?;
    oled_rst.set_high()?;
    FreeRtos::delay_ms(10);
    oled_rst.set_low()?;
    FreeRtos::delay_ms(10);
    oled_rst.set_high()?;
    FreeRtos::delay_ms(10);

    let i2c_config = I2cConfig::new().baudrate(400.kHz().into());
    let i2c = I2cDriver::new(
        peripherals.i2c0,
        peripherals.pins.gpio4,
        peripherals.pins.gpio15,
        &i2c_config,
    )?;
    let interface = I2CDisplayInterface::new(i2c);
    let mut display = Ssd1306::new(interface, DisplaySize128x64, DisplayRotation::Rotate0)
        .into_buffered_graphics_mode();
    let ecran_disponible = display.init().is_ok();
    let style = MonoTextStyle::new(&FONT_6X10, BinaryColor::On);

    // --- Module LoRa ---
    let lora_reset = PinDriver::output(peripherals.pins.gpio14)?;
    let spi_config = SpiConfig::new().baudrate(1.MHz().into());
    let lora_spi = SpiDeviceDriver::new_single(
        peripherals.spi2,
        peripherals.pins.gpio5,  // SCK
        peripherals.pins.gpio27, // MOSI
        Some(peripherals.pins.gpio19), // MISO
        Some(peripherals.pins.gpio18), // NSS
        &SpiDriverConfig::new(),
        &spi_config,
    )?;
    let mut lora = Sx127x::new(lora_spi, lora_reset);
    lora.init(FREQUENCE_HZ)?;
    info!("LoRa initialisé à {} Hz, prêt à émettre", FREQUENCE_HZ);

    let mut compteur: u32 = 0;
    let mut nb_ack: u32 = 0;
    let mut nb_manques: u32 = 0;

    loop {
        compteur += 1;

        let mut message: String<32> = String::new();
        write!(message, "PING {}", compteur).ok();

        let mut ack_recu = false;
        let mut rssi_ack: i32 = 0;

        for essai in 1..=MAX_ESSAIS {
            match lora.envoyer(message.as_bytes()) {
                Ok(()) => info!("Envoyé : \"{}\" (essai {}/{})", message, essai, MAX_ESSAIS),
                Err(e) => {
                    warn!("Échec d'envoi : {:?}", e);
                    continue;
                }
            }

            lora.demarrer_ecoute()?;
            let mut attendu: String<32> = String::new();
            write!(attendu, "ACK {}", compteur).ok();

            for _ in 0..TOURS_ATTENTE_ACK {
                if let Ok(Reception::Paquet { donnees, rssi_dbm }) = lora.recevoir() {
                    let texte = core::str::from_utf8(&donnees).unwrap_or("");
                    if texte == attendu.as_str() {
                        ack_recu = true;
                        rssi_ack = rssi_dbm;
                        break;
                    }
                }
                FreeRtos::delay_ms(DELAI_TOUR_MS);
            }

            if ack_recu {
                nb_ack += 1;
                info!("ACK reçu pour #{} (RSSI {} dBm)", compteur, rssi_ack);
                break;
            } else {
                warn!("Pas d'ACK pour #{} (essai {}/{})", compteur, essai, MAX_ESSAIS);
            }
        }

        if ack_recu {
            led.set_high()?;
            FreeRtos::delay_ms(500);
            led.set_low()?;
        } else {
            nb_manques += 1;
            warn!(
                "RATE : #{} après {} essais, aucun ACK reçu (total ratés : {})",
                compteur, MAX_ESSAIS, nb_manques
            );
        }

        if ecran_disponible {
            let mut ligne_compteur: String<32> = String::new();
            write!(ligne_compteur, "Essai #{}", compteur).ok();
            let mut ligne_stats: String<32> = String::new();
            write!(ligne_stats, "OK:{}  RATES:{}", nb_ack, nb_manques).ok();
            let mut ligne_statut: String<32> = String::new();
            if ack_recu {
                write!(ligne_statut, "RSSI {} dBm", rssi_ack).ok();
            } else {
                write!(ligne_statut, "AUCUN ACK !").ok();
            }

            display.clear(BinaryColor::Off).ok();
            Text::new("TEST 1s", Point::new(0, 12), style)
                .draw(&mut display)
                .ok();
            Text::new(&ligne_compteur, Point::new(0, 28), style)
                .draw(&mut display)
                .ok();
            Text::new(&ligne_stats, Point::new(0, 42), style)
                .draw(&mut display)
                .ok();
            Text::new(&ligne_statut, Point::new(0, 56), style)
                .draw(&mut display)
                .ok();
            display.flush().ok();
        }

        FreeRtos::delay_ms(DELAI_ENTRE_ENVOIS_MS);
    }
}
