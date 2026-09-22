use esp_idf_svc::hal::delay::FreeRtos;
use esp_idf_svc::hal::gpio::{Pull, PinDriver};
use esp_idf_svc::hal::peripherals::Peripherals;
use esp_idf_svc::hal::sleep::{rtc::RtcWakeLevel, DeepSleep};
use esp_idf_svc::hal::spi::{config::Config as SpiConfig, SpiDeviceDriver, SpiDriverConfig};
use esp_idf_svc::hal::units::FromValueType;
use log::{info, warn};

use lora_boite_lettres::sx127x::{Reception, Sx127x};

// 433 MHz, comme le reste du projet Heltec.
const FREQUENCE_HZ: u32 = 433_000_000;
// Message de test envoyé à chaque réveil. Le récepteur (recepteur.rs) répond
// "ACK 1" dès qu'il reçoit un message commençant par "PING ".
const MESSAGE: &str = "PING 1";
const ACK_ATTENDU: &str = "ACK 1";

const TOURS_ATTENTE_ACK: u32 = 100; // 100 x 20ms = 2s
const DELAI_TOUR_MS: u32 = 20;
const MAX_ESSAIS: u32 = 3;

/// Test de veille profonde pour l'émetteur boîte aux lettres : à chaque
/// démarrage (premier boot, ou réveil par le bouton sur GPIO33), envoie un
/// message avec accusé de réception, clignote la LED du boîtier pour
/// indiquer le résultat, puis repart en veille profonde en attendant le
/// prochain appui. Sert à valider le cycle réveil/envoi/ACK/re-veille sur
/// batterie, avant de câbler les vrais capteurs (reed/tapis) à la place du
/// bouton — voir mémoire projet_boite_lettres_lora.
fn main() -> anyhow::Result<()> {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();
    info!("Réveil de l'émetteur (bouton ou premier démarrage)");

    let peripherals = Peripherals::take()?;
    let mut led = PinDriver::output(peripherals.pins.gpio25)?;

    // Vext (GPIO21) alimente l'écran OLED intégré et d'autres périphériques
    // auxiliaires sur les cartes Heltec — actif à l'état bas. On ne l'utilise
    // pas dans ce programme, donc on le coupe explicitement (état haut) pour
    // ne pas laisser l'écran alimenté inutilement en veille.
    let mut vext = PinDriver::output(peripherals.pins.gpio21)?;
    vext.set_high()?;

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
    info!("LoRa initialisé à {} Hz", FREQUENCE_HZ);

    let mut ack_recu = false;

    for essai in 1..=MAX_ESSAIS {
        match lora.envoyer(MESSAGE.as_bytes()) {
            Ok(()) => info!("Envoyé : \"{}\" (essai {}/{})", MESSAGE, essai, MAX_ESSAIS),
            Err(e) => {
                warn!("Échec d'envoi : {:?}", e);
                continue;
            }
        }

        lora.demarrer_ecoute()?;
        for _ in 0..TOURS_ATTENTE_ACK {
            if let Ok(Reception::Paquet { donnees, rssi_dbm }) = lora.recevoir() {
                let texte = core::str::from_utf8(&donnees).unwrap_or("");
                if texte == ACK_ATTENDU {
                    ack_recu = true;
                    info!("ACK reçu (RSSI {} dBm)", rssi_dbm);
                    break;
                }
            }
            FreeRtos::delay_ms(DELAI_TOUR_MS);
        }

        if ack_recu {
            break;
        }
        warn!("Pas d'ACK (essai {}/{})", essai, MAX_ESSAIS);
    }

    // Retour visuel sur la LED du boîtier : 2 clignotements courts = succès,
    // 1 clignotement long = échec (pas besoin du moniteur série pour savoir).
    if ack_recu {
        info!("Résultat : OK");
        for _ in 0..2 {
            led.set_high()?;
            FreeRtos::delay_ms(150);
            led.set_low()?;
            FreeRtos::delay_ms(150);
        }
    } else {
        warn!("Résultat : ÉCHEC après {} essais", MAX_ESSAIS);
        led.set_high()?;
        FreeRtos::delay_ms(1000);
        led.set_low()?;
    }

    // Indispensable : sans ça, la puce radio reste en écoute (~10-12 mA en
    // continu) même une fois l'ESP32 endormi.
    if let Err(e) = lora.dormir() {
        warn!("Échec mise en veille de la puce radio : {:?}", e);
    }

    // En veille profonde, le domaine numérique se coupe et une broche de
    // sortie peut "flotter"/dériver, même si on vient de la mettre à l'état
    // bas juste avant — d'où la LED qui restait allumée malgré `set_low()`.
    // `gpio_hold_en` verrouille l'état de chaque broche pendant la veille ;
    // `gpio_deep_sleep_hold_en` est nécessaire en plus sur ESP32 classique
    // pour que ce verrouillage s'applique bien en veille profonde (pas
    // seulement en veille légère).
    unsafe {
        esp_idf_svc::sys::gpio_hold_en(led.pin() as i32);
        esp_idf_svc::sys::gpio_hold_en(vext.pin() as i32);
        esp_idf_svc::sys::gpio_deep_sleep_hold_en();
    }

    // Bouton de test sur GPIO33 (libre, capable RTC) : à la masse au repos
    // via la résistance de tirage interne, appui = niveau bas -> réveil.
    // Remplacera plus tard les vrais contacts (reed volet/porte, tapis colis).
    let bouton = PinDriver::rtc_input(peripherals.pins.gpio33, Pull::Up)?;
    let veille = DeepSleep::new()?.wakeup_on_rtc(&bouton, RtcWakeLevel::AllLow)?;

    info!("Entrée en veille profonde, réveil sur appui du bouton (GPIO33)");
    FreeRtos::delay_ms(50); // laisse le temps au dernier log de sortir
    veille.enter()
}
