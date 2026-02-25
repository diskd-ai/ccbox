use ccbox_relay::server::run_http_server;
use ccbox_relay::store::{delete_pairing, make_store_paths, save_pairing, save_trusted_devices};
use ccbox_relay::types::{PairingRecord, TrustedDevice};
use ccbox_relay::util::{base32_no_pad, now_iso, random_nonce32};
use std::path::PathBuf;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

#[derive(Debug)]
enum CliCommand {
    Serve {
        port: u16,
        data_dir: PathBuf,
    },
    DevicesAdd {
        data_dir: PathBuf,
        device_id: String,
        public_key_b64: String,
        label: Option<String>,
    },
    PairCreate {
        data_dir: PathBuf,
        guid: String,
        ttl_seconds: u64,
    },
}

fn parse_args(argv: &[String]) -> Result<CliCommand, String> {
    let sub = argv.get(1).map(|s| s.as_str()).unwrap_or("serve");
    let args = &argv[1..];

    fn read_flag(args: &[String], name: &str) -> Option<String> {
        let idx = args.iter().position(|a| a == name)?;
        args.get(idx + 1).cloned()
    }

    fn read_number_flag(args: &[String], name: &str, default: u64) -> Result<u64, String> {
        match read_flag(args, name) {
            Some(raw) => raw
                .parse::<u64>()
                .map_err(|_| format!("Invalid {name}: {raw}")),
            None => Ok(default),
        }
    }

    let data_dir = read_flag(args, "--data-dir").unwrap_or_else(|| "./data".to_string());
    let data_dir = PathBuf::from(data_dir);

    match sub {
        "serve" => {
            let port = read_number_flag(args, "--port", 8787)?
                .try_into()
                .map_err(|_| "Invalid --port".to_string())?;
            Ok(CliCommand::Serve { port, data_dir })
        }
        "devices:add" => {
            let device_id = read_flag(args, "--device-id").ok_or("Missing --device-id")?;
            let public_key_b64 =
                read_flag(args, "--public-key-b64").ok_or("Missing --public-key-b64")?;
            let label = read_flag(args, "--label");
            Ok(CliCommand::DevicesAdd {
                data_dir,
                device_id,
                public_key_b64,
                label,
            })
        }
        "pair:create" => {
            let guid = read_flag(args, "--guid").ok_or("Missing --guid")?;
            let ttl_seconds = read_number_flag(args, "--ttl-seconds", 120)?;
            Ok(CliCommand::PairCreate {
                data_dir,
                guid,
                ttl_seconds,
            })
        }
        other => Err(format!("Unknown command: {other}")),
    }
}

fn main() {
    if let Err(error) = run_main() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

fn run_main() -> Result<(), String> {
    let argv = std::env::args().collect::<Vec<_>>();
    let cmd = parse_args(&argv)?;

    match cmd {
        CliCommand::Serve { port, data_dir } => {
            let store_paths = make_store_paths(&data_dir);
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .map_err(|error| error.to_string())?;
            rt.block_on(async move { run_http_server(port, store_paths).await })
        }
        CliCommand::DevicesAdd {
            data_dir,
            device_id,
            public_key_b64,
            label,
        } => {
            let paths = make_store_paths(&data_dir);
            let mut file =
                ccbox_relay::store::load_trusted_devices(&paths).map_err(|e| e.to_string())?;
            file.trusted_devices.retain(|d| d.device_id != device_id);
            file.trusted_devices.push(TrustedDevice {
                device_id: device_id.clone(),
                public_key_b64,
                created_at: now_iso(),
                last_seen_at: None,
                revoked: false,
                label,
            });
            save_trusted_devices(&paths, &file).map_err(|e| e.to_string())?;
            println!("added trusted device {device_id}");
            Ok(())
        }
        CliCommand::PairCreate {
            data_dir,
            guid,
            ttl_seconds,
        } => {
            let paths = make_store_paths(&data_dir);
            if let Some(existing) =
                ccbox_relay::store::load_pairing(&paths, &guid).map_err(|e| e.to_string())?
            {
                if is_pairing_active(&existing) {
                    return Err("PairingAlreadyActive".to_string());
                }
                delete_pairing(&paths, &guid).map_err(|e| e.to_string())?;
            }

            let secret = random_nonce32();
            let code = base32_no_pad(&secret).chars().take(10).collect::<String>();
            let created_at = now_iso();
            let expires_at = (OffsetDateTime::now_utc()
                + time::Duration::seconds(ttl_seconds as i64))
            .format(&time::format_description::well_known::Rfc3339)
            .map_err(|error| error.to_string())?;

            let record = PairingRecord {
                code_base32: code.clone(),
                created_at,
                expires_at,
                attempts_remaining: 5,
            };
            save_pairing(&paths, &guid, &record).map_err(|e| e.to_string())?;
            println!("{guid} {code}");
            Ok(())
        }
    }
}

fn is_pairing_active(record: &PairingRecord) -> bool {
    if record.attempts_remaining == 0 {
        return false;
    }
    let Ok(expires_at) = OffsetDateTime::parse(&record.expires_at, &Rfc3339) else {
        return false;
    };
    expires_at > OffsetDateTime::now_utc()
}
