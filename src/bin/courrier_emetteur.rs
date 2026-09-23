use core::fmt::Write as _;
use esp_idf_svc::hal::delay::FreeRtos;
use esp_idf_svc::hal::gpio::{Input, Output, PinDriver, Pull};
use esp_idf_svc::hal::peripherals::Peripherals;
use esp_idf_svc::hal::spi::{config::Config as SpiConfig, SpiDeviceDriver, SpiDriverConfig};
use esp_idf_svc::hal::units::FromValueType;
use heapless::String;
use log::{info, warn};

use lora_boite_lettres::sx127x::{Reception, Sx127x};

// Programme "sans contact de porte" : la carte Heltec reste alimentée en
// continu sur pile et gère elle-même son cycle réveil/sommeil (deep-sleep
// RTC de l'ESP32), au lieu d'être réveillée par un contact mécanique sur la
// porte du facteur (abandonné : problèmes de fixation + interdiction de
// percer, logement en location). Séquence (voir schéma KiCad
// "Latch colis sans inter", circuit Q1/Q2 validé) :
//
//   Réveil RTC (toutes les INTERVALLE_VEILLE_US, corrigé à chaque échange
//   réussi, voir plus bas) -> mise sous tension du HX711 (GPIO33 -> Q1 -> Q2,
//   MOSFET P-channel côté haut) -> pesée avec boucle de stabilisation ->
//   poids >= seuil ? "COLIS:<g>" : "RIEN" -> envoi LoRa systématique et
//   attente d'accusé de réception (essais bornés), qui porte aussi la
//   correction d'horloge du récepteur -> coupure de l'alimentation du HX711,
//   mise en sommeil du module LoRa, puis deep-sleep de l'ESP32 jusqu'au
//   prochain réveil. Aucune mémorisation entre les cycles pour la décision
//   d'envoi (pas de poids de référence "avant/après", pas de "déjà
//   notifié") : on notifie à chaque réveil tant qu'il y a du poids sur le
//   plateau, décision actée avec l'utilisateur (peu importe qu'il s'agisse
//   du même courrier non récupéré ou d'un colis supplémentaire empilé).
//
// Pas d'écran OLED sur cette version : décision actée (consommation inutile
// sur une carte alimentée par pile, aucune indication utile une fois la
// boîte fermée).
const FREQUENCE_HZ: u32 = 433_000_000;
const TOURS_ATTENTE_ACK: u32 = 100; // 100 x 20ms = 2s
const DELAI_TOUR_MS: u32 = 20;
const MAX_ESSAIS: u32 = 3;

// --- Intervalle entre deux réveils ---
// Pas d'horloge temps réel ni de Wi-Fi/NTP sur cette carte (pile seule, hors
// de question d'ajouter du matériel/de la consommation juste pour l'horloge
// — voir [[projet_boite_lettres_lora]]). Valeur de repli utilisée tant
// qu'aucune correction n'a encore été reçue (premier réveil, ou échec de
// l'ACK) : 12h donne "deux fois par jour" à peu près, sans dérive corrigée.
//
// Correction d'horloge (10/09/2026) : à chaque réveil, l'émetteur envoie
// systématiquement un message (COLIS ou RIEN, voir plus bas) et écoute
// l'ACK. Le récepteur, lui, est sur secteur et synchronisé en NTP — il glisse
// à la fin de son ACK le nombre de secondes jusqu'au prochain 12h00/18h00
// (heure de Paris). Si cette correction est reçue, elle remplace
// INTERVALLE_VEILLE_US pour la durée de veille qui suit, ce qui recale
// l'émetteur sur l'heure murale à chaque échange réussi sans qu'il ait
// besoin de connaître l'heure lui-même. Voir courrier_recepteur.rs.
const INTERVALLE_VEILLE_US: u64 = 12 * 60 * 60 * 1_000_000;

// Plancher de sécurité pour la durée de veille corrigée : évite un réveil
// quasi immédiat (boucle qui viderait la batterie) si jamais la correction
// reçue tombait tout près de zéro (réveil arrivé à quelques secondes d'un
// 12h/18h pile).
const DUREE_VEILLE_MIN_US: u64 = 5 * 60 * 1_000_000;

// --- Pesée HX711 (voir mémoire projet, calibration validée à la main de
// 13g à 2,25kg avec un zéro frais : ~23,7 points bruts par gramme) ---
const PENTE_POINTS_PAR_GRAMME: f32 = 23.672;
const NB_ECHANTILLONS_HX711: u32 = 10;
// En dessous de ce poids (en grammes), on considère qu'il n'y a rien eu de
// déposé (bruit résiduel du capteur, jamais un vrai courrier).
const SEUIL_POIDS_G: f32 = 4.0;

// --- Boucle de stabilisation de la pesée (absorbe les chocs/rebonds d'un
// facteur qui pose ou jette le courrier) ---
const DELAI_MAX_STABILISATION_MS: u32 = 4_000;
const INTERVALLE_STABILISATION_MS: u32 = 300;
const TOLERANCE_STABILITE_G: f32 = 3.0;

// Temps de stabilisation du HX711 après remise sous tension (régulateur et
// oscillateur internes), avant que les mesures soient fiables.
const DELAI_ALIMENTATION_HX711_MS: u32 = 400;

// Zéro (tare) du HX711, conservé en mémoire RTC lente : survit au
// deep-sleep mais repart à sa valeur d'initialisation (sentinelle) à chaque
// véritable mise sous tension/reset. On ne tare donc qu'une seule fois, à
// la toute première mise en service, jamais à chaque réveil -- sinon un
// courrier déjà présent sur le plateau serait inclus dans le "zéro" et ne
// serait jamais détecté.
#[link_section = ".rtc.data"]
static mut ZERO_HX711: i32 = i32::MIN;

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
    info!("Démarrage COURRIER EMETTEUR (réveil RTC, sans contact de porte)");

    let peripherals = Peripherals::take()?;

    // GPIO33 : commande la mise sous tension du HX711 (via Q1 -> Q2, MOSFET
    // P-channel côté haut) -- ce n'est plus une coupure de toute la carte
    // comme dans la version précédente à relais, seul le circuit balance
    // est concerné. A l'état bas au démarrage : HX711 éteint.
    let mut alimentation_hx711 = PinDriver::output(peripherals.pins.gpio33)?;
    alimentation_hx711.set_low()?;

    // HX711 : DT=GPIO17 (entrée, pull-up), SCK=GPIO23 (sortie). Broches
    // conformes au câblage réel du schéma "Latch colis sans inter".
    let dt = PinDriver::input(peripherals.pins.gpio17, Pull::Up)?;
    let mut sck = PinDriver::output(peripherals.pins.gpio23)?;
    sck.set_low()?;

    // --- Mise sous tension du HX711 et temps de stabilisation ---
    alimentation_hx711.set_high()?;
    FreeRtos::delay_ms(DELAI_ALIMENTATION_HX711_MS);

    // --- Zéro (tare) : une seule fois, jamais repris à chaque réveil (voir
    // commentaire sur ZERO_HX711) ---
    let zero = unsafe {
        if ZERO_HX711 == i32::MIN {
            let z = lire_moyenne_hx711(&dt, &mut sck)?;
            ZERO_HX711 = z;
            info!("Première mise en service : tarage HX711 effectué, zéro = {}", z);
            z
        } else {
            info!("Zéro HX711 repris de la mémoire RTC : {}", ZERO_HX711);
            ZERO_HX711
        }
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

    // --- Pesée avec boucle de stabilisation (absorbe les chocs/rebonds
    // d'un facteur qui pose ou jette le courrier) ---
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

    // --- Message envoyé à chaque réveil, avec ou sans poids détecté (décision
    // actée avec l'utilisateur : pas de distinction ancien/nouveau courrier).
    // "RIEN" est envoyé même sans poids pour que le récepteur ait toujours
    // l'occasion de renvoyer la correction d'horloge (voir plus haut) ---
    let poids_confirme = poids_final_g >= SEUIL_POIDS_G;
    let poids_arrondi = poids_final_g.round() as i32;

    let mut message: String<32> = String::new();
    if poids_confirme {
        write!(message, "COLIS:{}", poids_arrondi).ok();
    } else {
        write!(message, "RIEN").ok();
    }
    let mut attendu: String<32> = String::new();
    write!(attendu, "ACK_{}", message.as_str()).ok();

    let mut ack_recu = false;
    let mut duree_veille_us = INTERVALLE_VEILLE_US;

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
                if texte.starts_with(attendu.as_str()) {
                    ack_recu = true;
                    info!("ACK reçu (RSSI {} dBm)", rssi_dbm);

                    // Correction d'horloge optionnelle : le récepteur ajoute
                    // ":<secondes jusqu'au prochain 12h/18h>" à la fin de
                    // l'ACK quand il connaît l'heure (NTP synchronisé).
                    if let Some(reste) = texte.get(attendu.len()..) {
                        if let Some(secondes_texte) = reste.strip_prefix(':') {
                            match secondes_texte.parse::<u64>() {
                                Ok(secondes) => {
                                    duree_veille_us = secondes
                                        .saturating_mul(1_000_000)
                                        .max(DUREE_VEILLE_MIN_US);
                                    info!(
                                        "Correction d'horloge reçue : prochain réveil dans {} s",
                                        secondes
                                    );
                                }
                                Err(_) => warn!("Correction d'horloge illisible dans l'ACK : \"{}\"", reste),
                            }
                        }
                    }
                    break;
                }
            }
            FreeRtos::delay_ms(DELAI_TOUR_MS);
        }

        if ack_recu {
            break;
        } else {
            warn!("Pas d'ACK (essai {}/{})", essai, MAX_ESSAIS);
        }
    }

    if !ack_recu {
        warn!(
            "Aucun ACK après {} essais, pas de correction d'horloge, intervalle par défaut ({} h)",
            MAX_ESSAIS,
            INTERVALLE_VEILLE_US / 3_600_000_000
        );
    }

    // --- Fin de cycle (convergence des deux branches) : coupure de
    // l'alimentation du HX711, mise en sommeil du module LoRa (sinon il
    // continue de consommer plusieurs mA tout seul pendant le deep-sleep de
    // l'ESP32), puis deep-sleep de l'ESP32 jusqu'au prochain réveil ---
    lora.dormir()?;
    alimentation_hx711.set_low()?;

    info!(
        "Fin de cycle, entrée en deep-sleep pour {:.1} h",
        duree_veille_us as f64 / 3_600_000_000.0
    );
    unsafe {
        esp_idf_svc::sys::esp_deep_sleep(duree_veille_us);
    }

    // Ne devrait jamais être atteint (esp_deep_sleep ne revient pas) :
    // sécurité au cas où, pour ne rien retenter d'ici la veille réelle.
    #[allow(unreachable_code)]
    loop {
        FreeRtos::delay_ms(1000);
    }
}
