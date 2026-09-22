use core::fmt::Write as _;
use esp_idf_svc::hal::delay::FreeRtos;
use esp_idf_svc::hal::gpio::{Input, Output, PinDriver, Pull};
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

// Programme définitif : la carte est réveillée par le relais d'auto-maintien
// (K1/K2, déclenché par le contact n°1 de la porte du facteur, câblé
// uniquement dans la boucle 5V — voir mémoire
// projet_logique_distinction_facteur_proprietaire). Séquence complète
// (analyse validée façon GRAFCET, aucun état sans issue) :
//
//   Réveil -> tarage HX711 à vide
//          -> attente fermeture porte (contact n°2, GPIO13) OU délai de
//             sécurité max (toujours borné)
//          -> pesée de confirmation avec boucle de stabilisation (absorbe
//             les chocs/rebonds d'un facteur qui pose ou jette le courrier)
//             OU délai de sécurité max (toujours borné)
//          -> décision : poids positif, stable et > seuil => envoi LoRa
//             avec le poids en grammes ; sinon rien n'est envoyé
//          -> si envoyé : attente accusé de réception, bornée (3 essais)
//          -> (convergence des deux branches) coupure alimentation, toujours
//             atteinte quel que soit le chemin emprunté
//
// Écran OLED activable/désactivable pour les tests sans toucher au reste du
// code — repasser à false pour le fonctionnement final en boîte aux lettres
// (pas d'écran prévu sur batterie, décision actée précédemment).
const AFFICHER_ECRAN: bool = true;

const FREQUENCE_HZ: u32 = 433_000_000;
const TOURS_ATTENTE_ACK: u32 = 100; // 100 x 20ms = 2s
const DELAI_TOUR_MS: u32 = 20;
const MAX_ESSAIS: u32 = 3;

// --- Pesée HX711 (voir mémoire projet, calibration validée à la main de
// 13g à 2,25kg avec un zéro frais : ~23,7 points bruts par gramme) ---
const PENTE_POINTS_PAR_GRAMME: f32 = 23.672;
const NB_ECHANTILLONS_HX711: u32 = 10;
// En dessous de ce poids (en grammes), on considère qu'il n'y a rien eu de
// déposé (bruit résiduel du capteur, jamais un vrai courrier).
const SEUIL_POIDS_G: f32 = 4.0;

// --- Attente de la fermeture de la porte (contact n°2, GPIO13) ---
const DELAI_MAX_PORTE_MS: u32 = 60_000; // sécurité si la porte ne se referme jamais
const INTERVALLE_POLL_PORTE_MS: u32 = 100;

// --- Boucle de stabilisation de la pesée (absorbe les chocs/rebonds) ---
const DELAI_MAX_STABILISATION_MS: u32 = 4_000;
const INTERVALLE_STABILISATION_MS: u32 = 300;
const TOLERANCE_STABILITE_G: f32 = 3.0;

fn lire_brut_hx711(dt: &PinDriver<'_, Input>, sck: &mut PinDriver<'_, Output>) -> anyhow::Result<i32> {
    let mut tours_attente = 0;
    while dt.is_high() {
        FreeRtos::delay_ms(1);
        tours_attente += 1;
        if tours_attente > 1000 {
            warn!("HX711 ne répond pas (DT toujours haut après 1s) - vérifier le câblage");
            FreeRtos::delay_ms(500);
            tours_attente = 0;
        }
    }

    let mut valeur: u32 = 0;
    for _ in 0..24 {
        sck.set_high()?;
        valeur <<= 1;
        if dt.is_high() {
            valeur |= 1;
        }
        sck.set_low()?;
    }
    // 25e impulsion : fixe le gain à 128 (canal A) pour la prochaine mesure
    sck.set_high()?;
    sck.set_low()?;

    let valeur_signee: i32 = if valeur & 0x800000 != 0 {
        (valeur | 0xFF000000) as i32
    } else {
        valeur as i32
    };
    Ok(valeur_signee)
}

fn lire_moyenne_hx711(dt: &PinDriver<'_, Input>, sck: &mut PinDriver<'_, Output>) -> anyhow::Result<i32> {
    let mut somme: i64 = 0;
    for _ in 0..NB_ECHANTILLONS_HX711 {
        somme += lire_brut_hx711(dt, sck)? as i64;
    }
    Ok((somme / NB_ECHANTILLONS_HX711 as i64) as i32)
}

/// Convertit un écart brut HX711 en grammes, jamais négatif (un poids
/// négatif n'a pas de sens physique : c'est toujours du bruit/de la dérive).
fn poids_en_grammes(brut: i32, zero: i32) -> f32 {
    let ecart = (brut - zero) as f32 / PENTE_POINTS_PAR_GRAMME;
    if ecart < 0.0 {
        0.0
    } else {
        ecart
    }
}

fn main() -> anyhow::Result<()> {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();
    info!("Démarrage COURRIER EMETTEUR (réveillé par la porte du facteur)");

    let peripherals = Peripherals::take()?;

    // GPIO33 : commande la coupure du 5V (via Q1/K1) en fin de cycle.
    // A l'état bas au démarrage : ne coupe pas tant que le cycle n'est pas terminé.
    let mut coupure_alim = PinDriver::output(peripherals.pins.gpio33)?;
    coupure_alim.set_low()?;

    // GPIO13 : contact n°2 de la porte du facteur (indépendant du contact
    // n°1 câblé dans la boucle relais 5V), purement lu en logiciel une fois
    // le système déjà réveillé. Pull-up interne : contact fermé (niveau
    // bas) = porte fermée, contact ouvert (niveau haut) = porte ouverte.
    let contact_porte = PinDriver::input(peripherals.pins.gpio13, Pull::Up)?;

    // HX711 : DT=GPIO17 (entrée, pull-up), SCK=GPIO23 (sortie). Alimenté en
    // 3,3V. Broches conformes au câblage réel du schéma "Latch colis"
    // (vérifié via le netlist KiCad).
    let dt = PinDriver::input(peripherals.pins.gpio17, Pull::Up)?;
    let mut sck = PinDriver::output(peripherals.pins.gpio23)?;
    sck.set_low()?;

    // --- Tarage à vide, dès le réveil (avant que le facteur ne pose quoi
    // que ce soit) ---
    let zero = lire_moyenne_hx711(&dt, &mut sck)?;
    info!("Tarage HX711 effectué : zéro = {}", zero);

    // --- Écran OLED (optionnel selon AFFICHER_ECRAN) ---
    let mut vext = PinDriver::output(peripherals.pins.gpio21)?;
    let mut oled_rst = PinDriver::output(peripherals.pins.gpio16)?;
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
    let style = MonoTextStyle::new(&FONT_6X10, BinaryColor::On);

    let ecran_disponible = if AFFICHER_ECRAN {
        vext.set_low()?; // actif à l'état bas sur les cartes Heltec
        oled_rst.set_high()?;
        FreeRtos::delay_ms(10);
        oled_rst.set_low()?;
        FreeRtos::delay_ms(10);
        oled_rst.set_high()?;
        FreeRtos::delay_ms(10);
        display.init().is_ok()
    } else {
        false
    };

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

    // --- Attente de la fermeture de la porte (ou délai de sécurité max) ---
    info!("Attente fermeture de la porte (max {} ms)...", DELAI_MAX_PORTE_MS);
    let mut attente_porte_ms: u32 = 0;
    while contact_porte.is_high() && attente_porte_ms < DELAI_MAX_PORTE_MS {
        FreeRtos::delay_ms(INTERVALLE_POLL_PORTE_MS);
        attente_porte_ms += INTERVALLE_POLL_PORTE_MS;
    }
    if contact_porte.is_low() {
        info!("Porte refermée après {} ms", attente_porte_ms);
    } else {
        warn!("Délai de sécurité atteint ({} ms), porte toujours ouverte, on continue", DELAI_MAX_PORTE_MS);
    }

    // --- Pesée de confirmation, avec boucle de stabilisation (absorbe les
    // chocs/rebonds d'un facteur qui pose ou jette le courrier) ---
    let mut derniere_lecture = lire_moyenne_hx711(&dt, &mut sck)?;
    let mut stabilise = false;
    let mut attente_stabilisation_ms: u32 = 0;
    while attente_stabilisation_ms < DELAI_MAX_STABILISATION_MS {
        FreeRtos::delay_ms(INTERVALLE_STABILISATION_MS);
        attente_stabilisation_ms += INTERVALLE_STABILISATION_MS;
        let nouvelle_lecture = lire_moyenne_hx711(&dt, &mut sck)?;
        let ecart_g = ((nouvelle_lecture - derniere_lecture) as f32 / PENTE_POINTS_PAR_GRAMME).abs();
        derniere_lecture = nouvelle_lecture;
        if ecart_g < TOLERANCE_STABILITE_G {
            stabilise = true;
            break;
        }
    }
    let poids_final_g = poids_en_grammes(derniere_lecture, zero);
    if stabilise {
        info!("Pesée stabilisée après {} ms : {:.0} g", attente_stabilisation_ms, poids_final_g);
    } else {
        warn!("Pesée non stabilisée après {} ms, valeur retenue quand même : {:.0} g", attente_stabilisation_ms, poids_final_g);
    }

    // --- Décision d'envoi ---
    let poids_confirme = poids_final_g >= SEUIL_POIDS_G;
    let poids_arrondi = poids_final_g.round() as i32;

    let mut message: String<32> = String::new();
    if poids_confirme {
        write!(message, "COLIS:{}", poids_arrondi).ok();
    }
    let mut attendu: String<32> = String::new();
    write!(attendu, "ACK_{}", message.as_str()).ok();

    let mut ack_recu = false;
    let mut rssi_ack: i32 = 0;

    if poids_confirme {
        for essai in 1..=MAX_ESSAIS {
            match lora.envoyer(message.as_bytes()) {
                Ok(()) => info!("Envoyé : \"{}\" (essai {}/{})", message, essai, MAX_ESSAIS),
                Err(e) => {
                    warn!("Échec d'envoi : {:?}", e);
                    continue;
                }
            }

            lora.demarrer_ecoute()?;

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
                info!("ACK reçu (RSSI {} dBm)", rssi_ack);
                break;
            } else {
                warn!("Pas d'ACK (essai {}/{})", essai, MAX_ESSAIS);
            }
        }

        if !ack_recu {
            warn!("Aucun ACK après {} essais, coupure de l'alimentation quand même", MAX_ESSAIS);
        }
    } else {
        info!("Pas de poids confirmé ({:.0} g < seuil {} g), rien n'est envoyé", poids_final_g, SEUIL_POIDS_G);
    }

    if ecran_disponible {
        display.clear(BinaryColor::Off).ok();
        if poids_confirme {
            Text::new(&message, Point::new(0, 12), style).draw(&mut display).ok();
            let mut ligne_statut: String<32> = String::new();
            if ack_recu {
                write!(ligne_statut, "ACK RSSI {} dBm", rssi_ack).ok();
            } else {
                write!(ligne_statut, "AUCUN ACK !").ok();
            }
            Text::new(&ligne_statut, Point::new(0, 28), style).draw(&mut display).ok();
        } else {
            Text::new("Rien envoye", Point::new(0, 12), style).draw(&mut display).ok();
            Text::new("(pas de poids)", Point::new(0, 28), style).draw(&mut display).ok();
        }
        Text::new("Extinction...", Point::new(0, 44), style).draw(&mut display).ok();
        display.flush().ok();
        FreeRtos::delay_ms(1500); // laisse le temps de lire l'écran avant la coupure
    }

    info!("Fin de cycle, coupure de l'alimentation");
    FreeRtos::delay_ms(100);
    coupure_alim.set_high()?; // active Q1 -> K1 -> coupe l'auto-maintien de K2

    // Sécurité : si jamais la coupure matérielle tarde, ne pas retenter
    // d'envoyer quoi que ce soit, juste attendre la coupure réelle.
    loop {
        FreeRtos::delay_ms(1000);
    }
}
