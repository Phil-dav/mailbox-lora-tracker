use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::hal::modem::Modem;
use esp_idf_svc::nvs::EspDefaultNvsPartition;
use esp_idf_svc::wifi::{AuthMethod, BlockingWifi, ClientConfiguration, Configuration, EspWifi};
use log::{info, warn};

use crate::config::RESEAUX_WIFI;

/// Connecte l'ESP32 à l'un des réseaux Wi-Fi listés dans `config::RESEAUX_WIFI`,
/// essayés dans l'ordre. Utilisé uniquement côté récepteur (sur secteur), pour
/// la synchronisation NTP — voir [[projet_boite_lettres_lora]] pour le pourquoi
/// (l'émetteur, lui, reste sans Wi-Fi/horloge pour ne pas user la batterie).
pub fn connecter(
    modem: Modem<'static>,
    sys_loop: EspSystemEventLoop,
    nvs: EspDefaultNvsPartition,
) -> anyhow::Result<BlockingWifi<EspWifi<'static>>> {
    let mut wifi = BlockingWifi::wrap(EspWifi::new(modem, sys_loop.clone(), Some(nvs))?, sys_loop)?;

    wifi.set_configuration(&Configuration::Client(ClientConfiguration::default()))?;
    wifi.start()?;

    let reseaux_visibles = wifi.scan()?;

    let mut derniere_erreur = None;
    for (ssid, mot_de_passe) in RESEAUX_WIFI {
        info!("Wi-Fi : tentative sur le réseau {ssid}...");

        let reseau_trouve = reseaux_visibles.iter().find(|r| r.ssid.as_str() == *ssid);

        let config = match reseau_trouve {
            Some(r) => ClientConfiguration {
                ssid: (*ssid)
                    .try_into()
                    .map_err(|_| anyhow::anyhow!("SSID trop long : {ssid}"))?,
                password: (*mot_de_passe)
                    .try_into()
                    .map_err(|_| anyhow::anyhow!("Mot de passe trop long pour {ssid}"))?,
                channel: Some(r.channel),
                auth_method: r.auth_method.unwrap_or(AuthMethod::WPA2Personal),
                ..Default::default()
            },
            None => {
                info!("Réseau {ssid} non vu lors du scan, tentative avec les réglages par défaut");
                ClientConfiguration {
                    ssid: (*ssid)
                        .try_into()
                        .map_err(|_| anyhow::anyhow!("SSID trop long : {ssid}"))?,
                    password: (*mot_de_passe)
                        .try_into()
                        .map_err(|_| anyhow::anyhow!("Mot de passe trop long pour {ssid}"))?,
                    auth_method: AuthMethod::WPA2Personal,
                    ..Default::default()
                }
            }
        };

        if let Err(e) = wifi.set_configuration(&Configuration::Client(config)) {
            warn!("Wi-Fi : configuration invalide pour {ssid} : {e:?}");
            derniere_erreur = Some(e.into());
            continue;
        }

        match wifi.connect() {
            Ok(()) => {
                info!("Wi-Fi connecté sur {ssid}");
                wifi.wait_netif_up()?;
                return Ok(wifi);
            }
            Err(e) => {
                warn!("Wi-Fi : échec de connexion sur {ssid} : {e:?}, réseau suivant");
                derniere_erreur = Some(e.into());
            }
        }
    }

    Err(derniere_erreur
        .unwrap_or_else(|| anyhow::anyhow!("Liste de réseaux Wi-Fi vide (config::RESEAUX_WIFI)")))
}
