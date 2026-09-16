# Registre de travail

## En cours : passage du réveil par contact de porte à un réveil temporisé (RTC deep-sleep)

**Motivation** : problèmes mécaniques pour fixer les contacteurs de porte, et
perçage interdit (location). Décision : abandonner le contact de porte,
réveiller la carte émettrice par timer RTC (deep-sleep ESP32), 2 fois/jour
(midi et 18h).

**Logique validée avec l'utilisateur** (voir conversation du 2026-09-13) :
- À chaque réveil : pesée avec boucle de stabilisation (comme aujourd'hui),
  puis si poids >= seuil : envoi LoRa `COLIS:<poids>` + attente ACK (comme
  aujourd'hui), sinon rien.
- Pas de mémorisation entre les cycles (pas de poids de référence, pas de
  flag "déjà notifié") : on notifie à *chaque* réveil tant qu'il y a du
  poids, même si c'est le même courrier non récupéré ou un 2e colis empilé.
  Confirmé explicitement : ça n'a aucune importance de distinguer
  ancien/nouveau courrier.
- Une fois la boîte vidée par l'utilisateur, le prochain réveil pèsera ~0 et
  ne notifiera pas — pas de mécanisme explicite nécessaire pour ça, c'est
  une conséquence naturelle de la logique ci-dessus.

**GRAFCET cible** (remplace l'attente de fermeture de porte par l'attente de
réveil RTC) :
```
X0 : DEEP SLEEP, RTC timer armé sur prochaine heure cible (12h/18h)
  -> réveil RTC ->
X1 : Réveil, init HX711, init LoRa
  ->
X2 : Pesée avec boucle de stabilisation (tolérance/délai max identiques à
     aujourd'hui)
  -> stabilisée ou timeout ->
   poids >= seuil ?
     oui -> X3 : envoi LoRa "COLIS:<poids>", attente ACK (essais bornés,
            identique à aujourd'hui)
     non -> rien
  -> (les deux branches reconvergent) ->
     retour X0 (deep sleep, prochain réveil programmé)
```

## Pas encore fait

- [ ] Implémenter la version deep-sleep RTC dans `src/bin/courrier_emetteur.rs`
      (sauvegarde de l'ancienne version déjà faite, voir ci-dessous —
      l'implémentation elle-même n'a pas encore commencé).
- [x] Question tranchée (2026-09-13) : le circuit de relais K1/K2 devient
      **inutile** avec le réveil RTC. Principe actuel confirmé par
      l'utilisateur pour mémoire : porte s'ouvre -> K2 (auto-maintien) se
      ferme et s'auto-alimente -> le 5V passe en série par le contact NF de
      K1 (fermé au repos) -> en fin de cycle, GPIO33 haut -> Q1 conduit ->
      excite K1 -> son contact NF s'ouvre -> coupe l'auto-maintien de K2 ->
      extinction totale. Ce mécanisme entier (K1, K2, Q1, contact de porte
      n°1) disparaît dans la version RTC : la carte reste alimentée en
      continu par la pile et gère elle-même son extinction via deep-sleep
      logiciel. Conséquence à traiter :
      - Le schéma KiCad (`KiCAD/Latch colis/`) doit être révisé pour retirer
        ce circuit (ou le neutraliser) une fois la nouvelle version actée.
      - Autonomie batterie à revalider : deep-sleep ESP32 (quelques dizaines
        de µA en continu) au lieu de coupure totale (0 µA) — compromis déjà
        discuté et accepté par l'utilisateur.
- [ ] Caler l'horloge RTC de l'ESP32 au démarrage (pas de source de temps
      fiable sans WiFi/NTP disponible sur la carte alimentée par pile) —
      à décider : dérive acceptée sur plusieurs mois, ou autre solution.
- [ ] Réfléchir à la valeur du seuil de poids dans ce nouveau contexte (le
      seuil actuel de 4 g visait à filtrer le bruit du capteur au repos ;
      probablement toujours valable mais à revalider une fois le nouveau
      flux en place).

## Sauvegardes effectuées

- `src/bin/courrier_emetteur-backups/courrier_emetteur_2026-09-13_avant-veille-rtc.rs`
  — copie de l'ancien firmware (réveil par contact de porte), avant toute
  modification. Dossier ignoré par git (`**/*-backups/`), conservé via
  Dropbox.

Rappel convention : toujours sauvegarder un fichier avant modification,
jamais d'écrasement direct (voir ce fichier + mémoire de session).
