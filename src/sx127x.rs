//! Pilote minimal du module LoRa SX127x, en mode "polling" (pas d'interruption
//! DIO0) : suffisant pour un premier test émetteur/récepteur, à faire évoluer
//! plus tard si besoin (arrêt profond, interruptions, etc.).
//!
//! Paramètres radio fixés en dur pour que les deux cartes se comprennent :
//! BW 125 kHz, SF7, CR 4/5, en-tête explicite, CRC activé, sync word 0x12.

use esp_idf_svc::hal::delay::FreeRtos;
use esp_idf_svc::hal::gpio::{Output, PinDriver};
use esp_idf_svc::hal::spi::{SpiDeviceDriver, SpiDriver};
use heapless::Vec;

// Registres SX127x utilisés (adresses du datasheet Semtech).
const REG_FIFO: u8 = 0x00;
const REG_OP_MODE: u8 = 0x01;
const REG_FRF_MSB: u8 = 0x06;
const REG_PA_CONFIG: u8 = 0x09;
const REG_LNA: u8 = 0x0C;
const REG_FIFO_ADDR_PTR: u8 = 0x0D;
const REG_FIFO_TX_BASE_ADDR: u8 = 0x0E;
const REG_FIFO_RX_BASE_ADDR: u8 = 0x0F;
const REG_FIFO_RX_CURRENT_ADDR: u8 = 0x10;
const REG_IRQ_FLAGS: u8 = 0x12;
const REG_RX_NB_BYTES: u8 = 0x13;
const REG_PKT_RSSI_VALUE: u8 = 0x1A;
const REG_RSSI_VALUE: u8 = 0x1B;
const REG_MODEM_CONFIG_1: u8 = 0x1D;
const REG_MODEM_CONFIG_2: u8 = 0x1E;
const REG_MODEM_CONFIG_3: u8 = 0x26;
const REG_PREAMBLE_MSB: u8 = 0x20;
const REG_PAYLOAD_LENGTH: u8 = 0x22;
const REG_SYNC_WORD: u8 = 0x39;
const REG_VERSION: u8 = 0x42;
const REG_PA_DAC: u8 = 0x4D;
const REG_OCP: u8 = 0x0B;

const MODE_LONG_RANGE: u8 = 0x80; // bit "LongRangeMode" (mode LoRa, pas FSK)
const MODE_SLEEP: u8 = 0x00;
const MODE_STDBY: u8 = 0x01;
const MODE_TX: u8 = 0x03;
const MODE_RX_CONTINUOUS: u8 = 0x05;

const IRQ_TX_DONE: u8 = 0x08;
const IRQ_RX_DONE: u8 = 0x40;
const IRQ_PAYLOAD_CRC_ERROR: u8 = 0x20;

/// Résultat d'une sonde de réception, voir `Sx127x::recevoir`.
pub enum Reception {
    Rien,
    ErreurCrc,
    Paquet { donnees: Vec<u8, 32>, rssi_dbm: i32 },
}

pub struct Sx127x<'d> {
    spi: SpiDeviceDriver<'d, SpiDriver<'d>>,
    reset: PinDriver<'d, Output>,
}

impl<'d> Sx127x<'d> {
    pub fn new(
        spi: SpiDeviceDriver<'d, SpiDriver<'d>>,
        reset: PinDriver<'d, Output>,
    ) -> Self {
        Self { spi, reset }
    }

    fn lire_registre(&mut self, addr: u8) -> anyhow::Result<u8> {
        let mut buf = [addr & 0x7F, 0x00];
        self.spi.transfer_in_place(&mut buf)?;
        Ok(buf[1])
    }

    fn ecrire_registre(&mut self, addr: u8, valeur: u8) -> anyhow::Result<()> {
        let mut buf = [addr | 0x80, valeur];
        self.spi.transfer_in_place(&mut buf)?;
        Ok(())
    }

    fn ecrire_rafale(&mut self, addr: u8, donnees: &[u8]) -> anyhow::Result<()> {
        let mut buf: Vec<u8, 33> = Vec::new();
        buf.push(addr | 0x80).ok();
        buf.extend_from_slice(donnees).ok();
        self.spi.transfer_in_place(&mut buf)?;
        Ok(())
    }

    fn lire_rafale(&mut self, addr: u8, longueur: usize) -> anyhow::Result<Vec<u8, 32>> {
        let mut buf: Vec<u8, 33> = Vec::new();
        buf.push(addr & 0x7F).ok();
        buf.resize(1 + longueur, 0).ok();
        self.spi.transfer_in_place(&mut buf)?;
        let mut sortie: Vec<u8, 32> = Vec::new();
        sortie.extend_from_slice(&buf[1..1 + longueur]).ok();
        Ok(sortie)
    }

    /// Initialise le module : reset matériel, vérification de présence
    /// (registre version = 0x12), puis configuration radio fixe.
    pub fn init(&mut self, frequence_hz: u32) -> anyhow::Result<()> {
        self.reset.set_low()?;
        FreeRtos::delay_ms(10);
        self.reset.set_high()?;
        FreeRtos::delay_ms(10);

        let version = self.lire_registre(REG_VERSION)?;
        if version != 0x12 {
            anyhow::bail!("SX127x absent ou non reconnu (version lue = 0x{:02X})", version);
        }

        // Le bit LongRangeMode ne peut être changé qu'en mode Sleep.
        self.ecrire_registre(REG_OP_MODE, MODE_LONG_RANGE | MODE_SLEEP)?;
        FreeRtos::delay_ms(10);
        self.ecrire_registre(REG_OP_MODE, MODE_LONG_RANGE | MODE_STDBY)?;

        // Fréquence : Frf = frequence_hz * 2^19 / 32 MHz (registre 24 bits).
        let frf = ((frequence_hz as u64) << 19) / 32_000_000;
        self.ecrire_registre(REG_FRF_MSB, (frf >> 16) as u8)?;
        self.ecrire_registre(REG_FRF_MSB + 1, (frf >> 8) as u8)?;
        self.ecrire_registre(REG_FRF_MSB + 2, frf as u8)?;

        self.ecrire_registre(REG_FIFO_TX_BASE_ADDR, 0x00)?;
        self.ecrire_registre(REG_FIFO_RX_BASE_ADDR, 0x00)?;

        // Gain LNA maximal + boost HF.
        self.ecrire_registre(REG_LNA, 0x23)?;

        // BW 125 kHz (0111) | CR 4/5 (001) | en-tête explicite (0).
        self.ecrire_registre(REG_MODEM_CONFIG_1, 0x72)?;
        // SF7 (0111) | CRC activé (bit2).
        self.ecrire_registre(REG_MODEM_CONFIG_2, 0x74)?;
        // AGC automatique activé (bit2) : sans ça le gain d'entrée reste figé au
        // maximum (voir RegLna ci-dessous), ce qui peut saturer le récepteur et
        // empêcher tout décodage à courte distance. Oubli identifié le 19/08
        // après échec symétrique en émission/réception malgré une config par
        // ailleurs conforme — voir mémoire projet_boite_lettres_lora.
        self.ecrire_registre(REG_MODEM_CONFIG_3, 0x04)?;

        self.ecrire_registre(REG_PREAMBLE_MSB, 0x00)?;
        self.ecrire_registre(REG_PREAMBLE_MSB + 1, 0x08)?;

        self.ecrire_registre(REG_SYNC_WORD, 0x12)?;

        // PA_BOOST en mode haute puissance +20 dBm (confirmé par le programme
        // d'origine de l'utilisateur, LoRaEmisCourrier.ino : setTxPower(20,
        // PABOOST)) — nécessite REG_PA_DAC=0x87 et un seuil OCP relevé à
        // 140 mA (formule de la bibliothèque LoRa de Sandeep Mistry).
        self.ecrire_registre(REG_PA_CONFIG, 0x8F)?;
        self.ecrire_registre(REG_PA_DAC, 0x87)?;
        self.ecrire_registre(REG_OCP, 0x20 | 0x11)?; // OcpTrim=17 -> 140 mA

        self.ecrire_registre(REG_IRQ_FLAGS, 0xFF)?; // toutes les IRQ à 1 = on les efface

        Ok(())
    }

    /// Émission bloquante (attend la fin d'envoi, avec un timeout de sécurité).
    pub fn envoyer(&mut self, donnees: &[u8]) -> anyhow::Result<()> {
        self.ecrire_registre(REG_OP_MODE, MODE_LONG_RANGE | MODE_STDBY)?;
        self.ecrire_registre(REG_FIFO_ADDR_PTR, 0x00)?;
        self.ecrire_registre(REG_PAYLOAD_LENGTH, donnees.len() as u8)?;
        self.ecrire_rafale(REG_FIFO, donnees)?;
        self.ecrire_registre(REG_OP_MODE, MODE_LONG_RANGE | MODE_TX)?;

        for _ in 0..200 {
            // 200 x 10ms = 2s de timeout, largement suffisant pour un petit paquet.
            let irq = self.lire_registre(REG_IRQ_FLAGS)?;
            if irq & IRQ_TX_DONE != 0 {
                self.ecrire_registre(REG_IRQ_FLAGS, 0xFF)?;
                return Ok(());
            }
            FreeRtos::delay_ms(10);
        }
        anyhow::bail!("Timeout : TxDone jamais reçu")
    }

    /// Passe en écoute continue (à appeler une seule fois avant de sonder
    /// régulièrement avec `recevoir`).
    pub fn demarrer_ecoute(&mut self) -> anyhow::Result<()> {
        self.ecrire_registre(REG_OP_MODE, MODE_LONG_RANGE | MODE_RX_CONTINUOUS)
    }

    /// Sonde non bloquante : à appeler en boucle. Distingue explicitement
    /// "rien détecté" de "un paquet a déclenché RxDone mais a échoué au CRC" —
    /// ce deuxième cas est un diagnostic important (le récepteur voit bien
    /// une trame LoRa arriver, juste corrompue ou mal accordée en détail).
    pub fn recevoir(&mut self) -> anyhow::Result<Reception> {
        let irq = self.lire_registre(REG_IRQ_FLAGS)?;
        if irq & IRQ_RX_DONE == 0 {
            return Ok(Reception::Rien);
        }

        if irq & IRQ_PAYLOAD_CRC_ERROR != 0 {
            self.ecrire_registre(REG_IRQ_FLAGS, 0xFF)?;
            return Ok(Reception::ErreurCrc);
        }

        let addr_courante = self.lire_registre(REG_FIFO_RX_CURRENT_ADDR)?;
        self.ecrire_registre(REG_FIFO_ADDR_PTR, addr_courante)?;
        let longueur = self.lire_registre(REG_RX_NB_BYTES)? as usize;
        let donnees = self.lire_rafale(REG_FIFO, longueur)?;

        let rssi_brut = self.lire_registre(REG_PKT_RSSI_VALUE)?;
        // Formule datasheet pour la voie HF (>525 MHz, cas de 868 MHz).
        let rssi_dbm = -157 + rssi_brut as i32;

        self.ecrire_registre(REG_IRQ_FLAGS, 0xFF)?;

        Ok(Reception::Paquet { donnees, rssi_dbm })
    }

    /// RSSI "instantané" (bruit ambiant du canal, indépendant de toute
    /// réception de paquet) : utile pour vérifier qu'une onde arrive vraiment
    /// jusqu'à l'antenne, même sans paquet LoRa valide décodé.
    pub fn rssi_instantane(&mut self) -> anyhow::Result<i32> {
        let rssi_brut = self.lire_registre(REG_RSSI_VALUE)?;
        Ok(-157 + rssi_brut as i32)
    }

    /// Relit le registre RegOpMode tel qu'il est réellement sur la puce
    /// (diagnostic : vérifie qu'une écriture précédente a bien "tenu").
    pub fn lire_op_mode(&mut self) -> anyhow::Result<u8> {
        self.lire_registre(REG_OP_MODE)
    }

    /// Passe la puce radio en mode SLEEP (consommation ~µA, contre ~10-12 mA
    /// en écoute continue). À appeler avant que l'ESP32 lui-même entre en
    /// veille profonde — sinon le SX127x continue de tourner tout seul,
    /// indépendamment de l'état de l'ESP32 (identifié le 22/08 : 11 mA
    /// mesurés en "veille" à cause de ça, voir mémoire
    /// projet_boite_lettres_lora).
    pub fn dormir(&mut self) -> anyhow::Result<()> {
        self.ecrire_registre(REG_OP_MODE, MODE_LONG_RANGE | MODE_SLEEP)
    }
}
