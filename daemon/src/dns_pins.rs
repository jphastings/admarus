use trust_dns_client::{
    client::{AsyncClient, ClientHandle},
    proto::iocompat::AsyncIoTokioAsStd,
    rr::{DNSClass, Name, RData, RecordType},
    tcp::TcpClientStream
};
use crate::prelude::*;

pub async fn manage_dns_pins(config: Arc<Args>) {
    if config.dns_pins.is_empty() {
        return;
    }
    if config.dns_pins.len() > 10 {
        warn!("You have a lot of DNS pins. Don't hesitate lowering the dns_pins_interval if you get rate limited by your DNS provider.")
    }
    let mut dns_pins_interval = config.dns_pins_interval;
    if dns_pins_interval < 60*3 {
        warn!("Your dns_pins_interval is too low. Increasing to 3 minutes.");
        dns_pins_interval = 60*3;
    }
    let dns_provider: SocketAddr = config.dns_provider.parse().expect("Invalid DNS provider address");

    // Find old pins and look for the previous DNS pins
    let old_pins = match list_pinned(&config.ipfs_rpc).await {
        Ok(pins) => pins,
        Err(err) => {
            error!("Failed to list old DNS pins: {err}");
            Vec::new()
        }
    };
    let mut previous_dns_pins = Vec::new();
    for cid in old_pins {
        let dag = match get_dag(&config.ipfs_rpc, &cid).await {
            Ok(dag) => dag,
            Err(err) => {
                error!("Failed to get DAG {cid}: {err}");
                continue;
            }
        };
        if let Some(serde_json::Value::Bool(true)) = dag.get("DNS-Pins") {
            previous_dns_pins.push(cid);
        }
    }

    loop {
        let start = Instant::now();

        // Init DNS client
        let (stream, sender) = TcpClientStream::<AsyncIoTokioAsStd<TokioTcpStream>>::new(dns_provider);
        let client = AsyncClient::new(stream, sender, None);
        let Ok((mut client, bg)) = client.await else {
            error!("Failed to connect to DNS provider");
            sleep(Duration::from_secs(dns_pins_interval)).await;
            continue;
        };
        tokio::spawn(bg);

        // Launch queries
        let mut queries = Vec::new();
        for dns_pin in &config.dns_pins {
            let dnslink_domain = format!("_dnslink.{dns_pin}");
            let Ok(name) = Name::from_str(&dnslink_domain) else {
                warn!("Invalid DNS pin name: {dns_pin}");
                continue;
            };
            let query = client.query(
                name,
                DNSClass::IN,
                RecordType::TXT,
           );
            queries.push(query);
        }

        // Read answers
        let results = join_all(queries).await;
        let mut values = HashMap::new();
        for (domain, result) in zip(config.dns_pins.iter(), results.into_iter()) {
            let response = match result {
                Ok(response) => response,
                Err(err) => {
                    warn!("Failed to query DNS pin {domain}: {err}");
                    continue;
                }
            };
            for answer in response.answers() {
                if let Some(RData::TXT(txt_data)) = answer.data() {
                    let mut value = String::new();
                    for data in txt_data.txt_data() {
                        value.push_str(&String::from_utf8_lossy(data));
                    }
                    if !value.starts_with("dnslink=/ipfs/") {
                        continue;
                    }
                    value = value.trim_start_matches("dnslink=/ipfs/").to_string();
                    values.entry(domain).or_insert_with(Vec::new).push(value);
                }
            }
        }
        for values in values.values_mut() {
            values.sort();
        }

        // Add dag to IPFS
        let mut dag_json = String::from(r#"{"DNS-Pins":true,"Data":{"/":{"bytes":"CAE"}},"Links":["#);
        for (domain, cids) in values {
            for (i, cid) in cids.iter().enumerate() {
                let name = match i {
                    0 => domain.to_owned(),
                    _ => format!("{domain} {i}"),
                };
                dag_json.push_str(&format!(r#"{{"Hash":{{"/":"{cid}"}},"Name":"{name}"}}"#)); // TODO size
            }
        }
        dag_json.push_str("]}");
        let cid = match put_dag(&config.ipfs_rpc, dag_json, true).await {
            Ok(cid) => cid,
            Err(err) => {
                error!("Failed to put DAG for DNS pins on IPFS: {err}");
                sleep(Duration::from_secs(dns_pins_interval)).await;
                continue;
            },
        };

        // Replace old dag with new one
        if !(previous_dns_pins.len() == 1 && previous_dns_pins[0] == cid) {
            if let Err(e) = add_pin(&config.ipfs_rpc, &cid).await {
                error!("Failed to pin new DNS pins: {e}");
                sleep(Duration::from_secs(dns_pins_interval)).await;
                continue;
            }
            for old_pin in previous_dns_pins {
                if let Err(e) = remove_pin(&config.ipfs_rpc, &old_pin).await {
                    error!("Failed to remove old DNS pin {old_pin}: {e}");
                }
            }
            previous_dns_pins = vec![cid];
        }

        sleep(Duration::from_secs(dns_pins_interval).saturating_sub(start.elapsed())).await;
    }
}