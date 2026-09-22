# Boîte aux lettres LoRa — carte Heltec WiFi LoRa 32

Paquet : `lora-boite-lettres`
Carte : **Heltec WiFi LoRa 32** (confirmé le 19/08 via le programme C++ d'origine,
bibliothèque `heltec.h`), cible `xtensa-esp32-espidf`.

Deuxième paire de cartes (LilyGO T95_V1.1, radio potentiellement différente) :
voir `c:\rust\8`, projet séparé (nécessaire à cause de la limite de longueur
de chemin Windows avec ESP-IDF — voir mémoire `projet_piege_chemins_courts_espidf`).

## Contexte

4 cartes LoRa en main, dont 2 Heltec (celles-ci). Un premier essai en C++
avait fonctionné il y a longtemps (programme retrouvé le 19/08 :
`E:\Documents\Arduino\Boîte aux lettres\Programmes\LoRaEmisCourrier.ino` /
`LoRaRecCourrier.ino`), mais ne fonctionnait plus au moment de la reprise.
Repris intégralement en Rust (esp-rs).

## Étape actuelle — RÉSOLU le 19/08

Communication LoRa établie dans les deux sens (433 MHz, PA_BOOST +20 dBm,
AGC). Cause du blocage initial : mauvaise fréquence supposée (868 puis
915 MHz essayés par déduction, alors que la vraie valeur — 433 MHz — était
dans le programme C++ d'origine). Voir mémoire `projet_boite_lettres_lora`
pour le détail complet du diagnostic.

Programmes disponibles (`src/bin/`) :
- `emetteur.rs` / `recepteur.rs` — bancs de test (compteur, RSSI), à garder
- `courrier_emetteur.rs` / `courrier_recepteur.rs` — portage du vrai
  programme métier (capteur de lumière, "Courrier reçu...")
- `test_t95` a été déplacé vers `c:\rust\8` (carte différente)

## Prochaines étapes

1. Câbler la photorésistance réelle sur l'émetteur.
2. Circuit d'auto-maintien par MOSFET (décidé le 19/08, pas encore câblé/codé).
3. Éventuellement plusieurs points de détection (volet, porte, colis).

## État

Démarré le 19/08/2026. Fonctionnel, reste le câblage physique définitif.
