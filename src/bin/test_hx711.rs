use esp_idf_svc::hal::delay::FreeRtos;
use esp_idf_svc::hal::gpio::{Input, Output, PinDriver, Pull};
use esp_idf_svc::hal::peripherals::Peripherals;
use log::info;

// Test HX711 sur ESP32 de rechange (pas la Heltec), pour valider la lecture
// des 4 jauges avant de porter le code sur la carte finale. Broches choisies
// pour rester libres aussi sur la Heltec (voir mémoire projet) :
// DT = GPIO17, SCK = GPIO22. HX711 alimenté en 3,3V (pas 5V, sinon DT
// dépasserait la tension max tolérée par le GPIO).
//
// Protocole HX711 (pas besoin de bibliothèque externe, c'est simple) :
// - DT passe à l'état bas quand une mesure est prête
// - On envoie 24 impulsions d'horloge sur SCK pour lire les 24 bits de
//   données (MSB en premier), plus 1 impulsion supplémentaire pour fixer
//   le gain du prochain cycle (canal A, gain 128 = standard)
// - La valeur est en complément à deux sur 24 bits, à étendre en 32 bits

// Pente de calibration mesurée à la main (voir mémoire projet) : environ
// 23,7 points bruts par gramme, validée de 13g à 2,25kg avec un zéro frais.
const PENTE_POINTS_PAR_GRAMME: f32 = 23.672;
// Nombre de lectures brutes moyennées à chaque mesure (zéro et pesée), pour
// lisser le bruit naturel du capteur (±100-150 points d'une lecture à
// l'autre).
const NB_ECHANTILLONS: u32 = 10;
// Zone morte autour de zéro : en dessous de ce seuil (en grammes), le bruit
// résiduel du capteur est affiché comme 0g plutôt qu'une petite valeur
// parasite (+/- quelques grammes).
const SEUIL_ZERO_G: f32 = 4.0;

fn lire_brut(
    dt: &PinDriver<'_, Input>,
    sck: &mut PinDriver<'_, Output>,
) -> anyhow::Result<i32> {
    // Attend que DT passe à l'état bas (mesure prête)
    let mut tours_attente = 0;
    while dt.is_high() {
        FreeRtos::delay_ms(1);
        tours_attente += 1;
        if tours_attente > 1000 {
            info!("HX711 ne répond pas (DT toujours haut après 1s) - vérifier le câblage");
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

    // Extension de signe : 24 bits -> 32 bits (complément à deux)
    let valeur_signee: i32 = if valeur & 0x800000 != 0 {
        (valeur | 0xFF000000) as i32
    } else {
        valeur as i32
    };

    Ok(valeur_signee)
}

fn lire_moyenne(
    dt: &PinDriver<'_, Input>,
    sck: &mut PinDriver<'_, Output>,
) -> anyhow::Result<i32> {
    let mut somme: i64 = 0;
    for _ in 0..NB_ECHANTILLONS {
        somme += lire_brut(dt, sck)? as i64;
    }
    Ok((somme / NB_ECHANTILLONS as i64) as i32)
}

fn main() -> anyhow::Result<()> {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();
    info!("Démarrage TEST HX711");

    let peripherals = Peripherals::take()?;

    let dt = PinDriver::input(peripherals.pins.gpio17, Pull::Up)?;
    let mut sck = PinDriver::output(peripherals.pins.gpio22)?;
    sck.set_low()?;

    info!("HX711 initialisé, DT=GPIO17, SCK=GPIO22. Tarage (plateau vide)...");
    let zero = lire_moyenne(&dt, &mut sck)?;
    info!("Zéro mesuré : {} (référence pour le calcul du poids)", zero);

    info!("Lecture en boucle, poids affiché en grammes...");

    loop {
        let brut = lire_moyenne(&dt, &mut sck)?;
        let ecart = brut - zero;
        let poids_g = ecart as f32 / PENTE_POINTS_PAR_GRAMME;
        // Négatif ou trop proche de zéro (bruit résiduel) -> affiché comme 0 g
        let poids_affiche = if poids_g < SEUIL_ZERO_G { 0.0 } else { poids_g };

        info!(
            "Poids : {:.0} g (brut={}, zéro={}, écart={})",
            poids_affiche, brut, zero, ecart
        );
        FreeRtos::delay_ms(300);
    }
}
