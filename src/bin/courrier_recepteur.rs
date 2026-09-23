use core::fmt::Write as _;
use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::hal::delay::FreeRtos;
use esp_idf_svc::hal::gpio::PinDriver;
use esp_idf_svc::hal::i2c::{I2cConfig, I2cDriver};
use esp_idf_svc::hal::peripherals::Peripherals;
use esp_idf_svc::hal::spi::{config::Config as SpiConfig, SpiDeviceDriver, SpiDriverConfig};
use esp_idf_svc::hal::units::FromValueType;
use esp_idf_svc::nvs::EspDefaultNvsPartition;
use esp_idf_svc::sntp::{EspSntp, SyncStatus};
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
use lora_boite_lettres::temps;

// Programme définitif du récepteur : écoute en continu (sur secteur, pas de
// contrainte d'énergie), répond par un ACK dès réception d'un message
// "COLIS:<grammes>" ou "RIEN" de l'émetteur, avant de rester en écoute.
//
// Correction d'horloge pour l'émetteur (10/09/2026) : l'émetteur n'a pas
// d'horloge propre (pas question d'ajouter du matériel/de la conso batterie
// juste pour ça) et dort par intervalle fixe (~12h), qui dérive lentement par
// rapport à l'heure murale. Le récepteur, lui, est sur secteur et peut se
// synchroniser en NTP sans contrainte. Donc : à chaque réveil de l'émetteur,
// qui envoie toujours un message (COLIS ou RIEN) et écoute l'ACK, le
// récepteur calcule ici combien de secondes il reste jusqu'au prochain 12h00
// ou 18h00 (heure de Paris) et glisse ce nombre à la fin de l'ACK. L'émetteur
// n'a plus qu'à l'utiliser comme durée de veille au lieu de sa constante fixe
// -> auto-correction de la dérive à chaque échange, sans horloge ni composant
// supplémentaire côté émetteur. Voir [[projet_boite_lettres_lora]].
const FREQUENCE_HZ: u32 = 433_000_000;

fn main() -> anyhow::Result<()> {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();
    info!("Démarrage COURRIER RECEPTEUR");

    let peripherals = Peripherals::take()?;

    // --- Wi-Fi + NTP (uniquement pour connaître l'heure murale, voir
    // commentaire en tête de fichier). Non bloquant : si la connexion échoue
    // (réseau introuvable, mauvais mot de passe...), le récepteur continue
    // de fonctionner normalement, juste sans correction d'horloge à donner à
    // l'émetteur (celui-ci retombe alors sur son intervalle fixe par défaut).
    let sys_loop = EspSystemEventLoop::take()?;
    let nvs = EspDefaultNvsPartition::take()?;
    let mut sntp = None;
    match lora_boite_lettres::wifi::connecter(peripherals.modem, sys_loop, nvs) {
        Ok(wifi) => {
            info!("Wi-Fi connecté, démarrage de la synchro NTP");
            sntp = Some(EspSntp::new_default()?);
            // `wifi` doit rester en vie tout le programme (sinon la connexion
            // retombe) : on la laisse fuiter volontairement, le récepteur ne
            // s'arrête jamais.
            core::mem::forget(wifi);
        }
        Err(e) => warn!("Wi-Fi indisponible ({:?}), pas de correction d'horloge possible", e),
    }

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

    if ecran_disponible {
        display.clear(BinaryColor::Off).ok();
        Text::new("Module LoRa ok!", Point::new(0, 8), style)
            .draw(&mut display)
            .ok();
        Text::new("En attente du facteur...", Point::new(0, 24), style)
            .draw(&mut display)
            .ok();
        display.flush().ok();
    }

    let mut nb_recus: u32 = 0;

    loop {
        match lora.recevoir() {
            Ok(Reception::Paquet { donnees, rssi_dbm }) => {
                let texte = core::str::from_utf8(&donnees).unwrap_or("(non-UTF8)");
                info!("Reçu : \"{}\" (RSSI {} dBm)", texte, rssi_dbm);

                // Le message a la forme "COLIS:<grammes>" (poids mesuré par
                // le HX711 côté émetteur) ou "RIEN" (réveil sans courrier,
                // envoyé quand même pour permettre la correction d'horloge
                // ci-dessous à chaque cycle, pas seulement quand il y a du
                // courrier).
                let poids_g: Option<i32> = texte
                    .strip_prefix("COLIS:")
                    .and_then(|reste| reste.parse::<i32>().ok());
                let message_valide = poids_g.is_some() || texte == "RIEN";

                // Répond par un ACK dès qu'un message valide est reçu, avant
                // de reprendre l'écoute. Si l'heure est connue (NTP
                // synchronisé), on ajoute à la fin le nombre de secondes
                // jusqu'au prochain réveil (12h/18h) pour que l'émetteur
                // recale sa propre veille dessus.
                if message_valide {
                    let correction_s = sntp
                        .as_ref()
                        .filter(|s| s.get_sync_status() == SyncStatus::Completed)
                        .and_then(|_| {
                            std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .ok()
                        })
                        .and_then(|d| chrono::DateTime::from_timestamp(d.as_secs() as i64, 0))
                        .map(|dt| temps::vers_heure_paris(dt.naive_utc()))
                        .map(temps::secondes_jusqu_au_prochain_reveil);

                    let mut ack: String<48> = String::new();
                    match correction_s {
                        Some(s) => write!(ack, "ACK_{}:{}", texte, s).ok(),
                        None => write!(ack, "ACK_{}", texte).ok(),
                    };
                    match lora.envoyer(ack.as_bytes()) {
                        Ok(()) => info!("ACK envoyé : \"{}\"", ack),
                        Err(e) => warn!("Échec d'envoi de l'ACK : {:?}", e),
                    }
                    lora.demarrer_ecoute()?;

                    if let Some(g) = poids_g {
                        nb_recus += 1;
                        info!("Courrier reçu, poids : {} g (total : {})", g, nb_recus);
                    } else {
                        info!("Réveil de l'émetteur sans courrier (RIEN)");
                    }
                }

                if ecran_disponible {
                    let mut ligne_message: String<32> = String::new();
                    match poids_g {
                        Some(g) => write!(ligne_message, "Colis recu : {} g", g).ok(),
                        None => write!(ligne_message, "{}", texte).ok(),
                    };
                    let mut ligne_rssi: String<32> = String::new();
                    write!(ligne_rssi, "RSSI {} dBm", rssi_dbm).ok();
                    let mut ligne_total: String<32> = String::new();
                    write!(ligne_total, "Total : {}", nb_recus).ok();

                    display.clear(BinaryColor::Off).ok();
                    Text::new(&ligne_message, Point::new(0, 12), style)
                        .draw(&mut display)
                        .ok();
                    Text::new(&ligne_rssi, Point::new(0, 28), style)
                        .draw(&mut display)
                        .ok();
                    Text::new(&ligne_total, Point::new(0, 44), style)
                        .draw(&mut display)
                        .ok();
                    display.flush().ok();
                }
            }
            Ok(Reception::ErreurCrc) => warn!("RxDone déclenché mais erreur CRC"),
            Ok(Reception::Rien) => {}
            Err(e) => warn!("Erreur de réception : {:?}", e),
        }

        FreeRtos::delay_ms(10);
    }
}
