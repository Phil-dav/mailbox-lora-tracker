# Circuit d'auto-maintien MOSFET pour l'émetteur boîte aux lettres

Référence trouvée le 22/08/2026, à étudier avant de câbler le circuit
d'alimentation de l'émetteur (contact ILS + MOSFET canal P).

## Lien principal

[Latching Power Switch Circuit (Auto Power Off Circuit) — Random Nerd Tutorials](https://randomnerdtutorials.com/latching-power-switch-circuit-auto-power-off-circuit-esp32-esp8266-arduino/)

Circuit spécifiquement conçu pour ESP32/ESP8266/Arduino, coupe le "+" de
l'alimentation (pas le "-").

**Composants** : MOSFET canal P (NDP6020P), transistor NPN 2N3904 (traducteur
de signal GPIO → grille du MOSFET), 2 diodes Schottky 1N5819, résistances
220kΩ / 2×100kΩ / 10kΩ / 220Ω.

**Principe** : le contact (bouton ou, dans notre cas, l'ILS) tire
momentanément la grille du MOSFET P vers le bas → la carte démarre → une GPIO
pilote un petit transistor NPN qui prend le relais pour maintenir la grille
basse → auto-maintien tant que la GPIO reste active → la carte coupe
elle-même l'alimentation en repassant la GPIO à l'état bas.

**Manque par rapport à notre besoin** : pas de condensateur pour ponter un
contact bref (le volet du courrier peut se rouvrir vite) — à ajouter
nous-mêmes en parallèle sur l'alimentation de la carte, en aval du MOSFET.

## Autres pages trouvées dans la même recherche (non étudiées en détail)

- [Push Button ON-OFF Soft Latch Circuits — Mosaic Industries](http://www.mosaic-industries.com/embedded-systems/microcontroller-projects/electronic-circuits/push-button-switch-turn-on/latching-toggle-power-switch)
- [Latching power switch uses momentary pushbutton — EDN](https://www.edn.com/latching-power-switch-uses-momentary-pushbutton/)
- [Soft Latching Power Circuits — Circuit Cellar](https://circuitcellar.com/resources/quickbits/soft-latching-power-circuits/)
- [Ditch The Switch: A Soft Latching Circuit Roundup — Hackaday](https://hackaday.com/2019/06/24/ditch-the-switch-a-soft-latching-circuit-roundup/)
- [Soft Latch Switch Circuit — circuits-diy.com](https://www.circuits-diy.com/soft-latch-switch-circuit/)
- [PBSEQ-D2 — CIRCUITSTATE Electronics](https://www.circuitstate.com/projects/pbseq-d2-soft-latching-power-sequencer-circuit-using-mosfets-amp-push-button/)
