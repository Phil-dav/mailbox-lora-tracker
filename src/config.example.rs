// Gabarit à copier en `src/config.rs` (fichier réel, gitignoré, jamais
// committé) et à remplir avec tes propres identifiants. `src/config.rs` n'est
// utilisé que par le récepteur (sur secteur), pour la synchro NTP qui sert à
// calculer l'heure du prochain réveil de l'émetteur (12h/18h) — voir
// [[projet_boite_lettres_lora]].

// Wi-Fi — liste de réseaux (SSID, mot de passe), essayés dans l'ordre.
pub const RESEAUX_WIFI: &[(&str, &str)] = &[
    ("TON_SSID_ICI", "TON_MOT_DE_PASSE_ICI"),
    // ("AUTRE_SSID", "AUTRE_MOT_DE_PASSE"),
];
