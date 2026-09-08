# Mailbox LoRa Tracker

A weight-sensing mailbox notifier built around a Heltec WiFi LoRa 32 (V2) and
an HX711 + 4 load cells. A door-triggered latching relay wakes the board with
zero standby current, it weighs whatever was just dropped in, sends the
result over LoRa to a receiver elsewhere in the house, waits for an ACK, then
cuts its own power.

Built for a shared/collective mailbox (multiple households behind one
postal-carrier door), where a simple door contact alone can't tell "someone
delivered to me" from "a neighbor's delivery, same door."

## How it works

```
Door opens (mechanical contact, no power needed)
  -> latching relay powers the board
  -> tare the HX711 (empty platform reference)
  -> wait for the door to close, or a safety timeout
  -> weigh again, with a settling/stability loop
     (absorbs shocks from a dropped/thrown package)
  -> if weight > threshold: send "COLIS:<grams>" over LoRa,
     wait for an ACK (bounded retries)
  -> cut its own power (transistor + relay), whatever happened above
```

Every branch converges on the same final power-cutoff step, so the board
always ends up switching itself off in bounded time, regardless of whether
a package was actually detected or an ACK came back.

## Hardware

- Heltec WiFi LoRa 32 V2 (ESP32 + SX127x), one per side (sender/receiver)
- HX711 + 4x 50kg half-bridge load cell sensors (bathroom-scale style)
- 2-relay latching power circuit (door contact triggers wake, transistor
  cuts power at end of cycle)
- 2nd door contact (independent GPIO) to detect when the door closes

### GPIO map (sender)

| GPIO | Function |
|---|---|
| 33 | Power cutoff (drives the relay-latch transistor) |
| 13 | 2nd door contact (detects door close) |
| 17 | HX711 DT |
| 23 | HX711 SCK |
| 5 / 27 / 19 / 18 | LoRa SPI (SCK / MOSI / MISO / NSS) — fixed on-module wiring |
| 14 | LoRa reset — fixed on-module wiring |
| 4 / 15 | OLED I2C (SDA / SCL) — fixed on-module wiring |
| 21 / 16 | OLED Vext / reset — fixed on-module wiring |

GPIO 34/35 are hardwired on the Heltec module to the SX127x's DIO2/DIO1 and
must never be reused for anything external.

## Repo contents

- `src/bin/courrier_emetteur.rs` — sender firmware (in the mailbox)
- `src/bin/courrier_recepteur.rs` — receiver firmware (indoors, mains-powered)
- `src/sx127x.rs` — minimal SX127x LoRa driver (SPI, ping/ack style)
- `KiCAD/Latch colis/` — schematic + PCB for the power-latch/relay board,
  plus etching files (single-sided, hand-etched)

> The schematic uses a couple of custom symbols/footprints (HX711 module,
> Heltec pinout) from a personal KiCad library that isn't included here yet
> — expect missing-footprint warnings if you open the project directly.

## Status

Weighing is calibrated and validated on the bench (13g to 2.25kg, <1% error
against a reference scale). Mechanical assembly (load cell mounting, door
contacts on the shared mailbox door) is still in progress.

## License

MIT — see [LICENSE](LICENSE).
