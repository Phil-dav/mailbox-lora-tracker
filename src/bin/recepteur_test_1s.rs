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

// Doit être identique à celle de l'émetteur (voir src/bin/emetteur.rs) — 433 MHz
// confirmé le 19/08 via le programme d'origine LoRaRecCourrier.ino/LoRaEmisCourrier.ino.
const FREQUENCE_HZ: u32 = 433_000_000;

fn main() -> anyhow::Result<()> {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();
    info!("Démarrage RECEPTEUR");

    let peripherals = Peripherals::take()?;
    let mut led = PinDriver::output(peripherals.pins.gpio25)?;

    // --- Écran OLED ---
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
    lora.demarrer_ecoute()?;
    info!("LoRa initialisé à {} Hz, en écoute", FREQUENCE_HZ);

    // Diagnostic : on relit ce que la puce a réellement enregistré, pour
    // écarter une écriture SPI silencieusement ratée. RegOpMode attendu :
    // 0x85 (LongRangeMode=1, mode=RX_CONTINUOUS=5).
    let mode_reel = lora.lire_op_mode()?;
    info!("RegOpMode relu après démarrage : 0x{:02X} (attendu 0x85)", mode_reel);

    let mut nb_recus: u32 = 0;
    let mut derniere_ligne: String<32> = String::new();
    write!(derniere_ligne, "(rien recu)").ok();
    let mut ligne_rssi_instant: String<32> = String::new();
    let mut compteur_boucle: u32 = 0;

    loop {
        compteur_boucle += 1;
        // Rafraîchi toutes les ~500 ms (boucle à 50 ms) : le RSSI instantané
        // bouge en permanence même sans paquet, pas besoin de le lire à
        // chaque tour.
        if compteur_boucle % 10 == 0 {
            if let Ok(rssi) = lora.rssi_instantane() {
                ligne_rssi_instant.clear();
                write!(ligne_rssi_instant, "Bruit: {} dBm", rssi).ok();
                info!("RSSI instantané (canal) : {} dBm", rssi);
            }
        }
        match lora.recevoir() {
            Ok(Reception::Paquet { donnees, rssi_dbm }) => {
                nb_recus += 1;
                led.set_high()?;

                let texte = core::str::from_utf8(&donnees).unwrap_or("(non-UTF8)");
                info!("Reçu #{} : \"{}\" (RSSI {} dBm)", nb_recus, texte, rssi_dbm);

                derniere_ligne.clear();
                write!(derniere_ligne, "{}", texte).ok();

                // Protocole ACK (21/08) : dès qu'un PING est reçu correctement,
                // on répond immédiatement par un accusé de réception portant le
                // même numéro, avant de reprendre l'écoute.
                if let Some(numero) = texte.strip_prefix("PING ") {
                    let mut ack: String<32> = String::new();
                    write!(ack, "ACK {}", numero).ok();
                    match lora.envoyer(ack.as_bytes()) {
                        Ok(()) => info!("ACK envoyé : \"{}\"", ack),
                        Err(e) => warn!("Échec d'envoi de l'ACK : {:?}", e),
                    }
                    lora.demarrer_ecoute()?;
                }

                FreeRtos::delay_ms(100);
                led.set_low()?;
            }
            Ok(Reception::ErreurCrc) => {
                // Important : ça prouve que le récepteur a bien synchronisé une
                // trame LoRa (préambule + en-tête corrects), juste le contenu
                // était corrompu ou mal calé — très différent de "rien reçu".
                warn!("RxDone déclenché mais erreur CRC (paquet détecté, contenu invalide)");
            }
            Ok(Reception::Rien) => {}
            Err(e) => warn!("Erreur de réception : {:?}", e),
        }

        if ecran_disponible {
            let mut ligne_compteur: String<32> = String::new();
            write!(ligne_compteur, "Recus: {}", nb_recus).ok();

            display.clear(BinaryColor::Off).ok();
            Text::new("RECEPTEUR", Point::new(0, 12), style)
                .draw(&mut display)
                .ok();
            Text::new(&ligne_compteur, Point::new(0, 28), style)
                .draw(&mut display)
                .ok();
            Text::new(&derniere_ligne, Point::new(0, 44), style)
                .draw(&mut display)
                .ok();
            Text::new(&ligne_rssi_instant, Point::new(0, 60), style)
                .draw(&mut display)
                .ok();
            display.flush().ok();
        }

        FreeRtos::delay_ms(50);
    }
}
