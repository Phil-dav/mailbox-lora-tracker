use chrono::{Datelike, Duration as ChronoDuration, NaiveDate, NaiveDateTime, Weekday};

/// Renvoie le dernier dimanche du mois donné (mars ou octobre ont tous deux 31 jours).
fn dernier_dimanche(annee: i32, mois: u32) -> NaiveDate {
    let mut jour = NaiveDate::from_ymd_opt(annee, mois, 31).unwrap();
    while jour.weekday() != Weekday::Sun {
        jour = jour.pred_opt().unwrap();
    }
    jour
}

/// Règle européenne : heure d'été du dernier dimanche de mars 01h00 UTC
/// au dernier dimanche d'octobre 01h00 UTC.
fn heure_ete_active(utc: NaiveDateTime) -> bool {
    let annee = utc.year();
    let debut = dernier_dimanche(annee, 3).and_hms_opt(1, 0, 0).unwrap();
    let fin = dernier_dimanche(annee, 10).and_hms_opt(1, 0, 0).unwrap();
    utc >= debut && utc < fin
}

/// Convertit une date/heure UTC en heure locale Europe/Paris (CET/CEST).
pub fn vers_heure_paris(utc: NaiveDateTime) -> NaiveDateTime {
    let decalage = if heure_ete_active(utc) { 2 } else { 1 };
    utc + ChronoDuration::hours(decalage)
}

/// Nombre de secondes entre `local` (heure locale Paris) et le prochain 12h00
/// ou 18h00 (le plus proche des deux, aujourd'hui ou demain si les deux sont
/// déjà passés). Utilisé pour dire à l'émetteur (qui n'a pas d'horloge propre)
/// combien de temps dormir pour retomber pile sur l'un de ces deux horaires.
pub fn secondes_jusqu_au_prochain_reveil(local: NaiveDateTime) -> i64 {
    let jour = NaiveDate::from_ymd_opt(local.year(), local.month(), local.day())
        .expect("date locale toujours valide");
    let candidats = [
        jour.and_hms_opt(12, 0, 0).unwrap(),
        jour.and_hms_opt(18, 0, 0).unwrap(),
        (jour + ChronoDuration::days(1)).and_hms_opt(12, 0, 0).unwrap(),
    ];
    candidats
        .into_iter()
        .filter(|&c| c > local)
        .min()
        .expect("le candidat de demain 12h est toujours dans le futur")
        .signed_duration_since(local)
        .num_seconds()
}
