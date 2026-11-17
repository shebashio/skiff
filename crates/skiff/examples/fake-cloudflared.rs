//! Native subprocess fixture for tests/cloudflare-share.py. It only listens
//! on loopback and never contacts Cloudflare or publishes a tunnel.

use serde_json::{Value, json};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

fn capture(path: &Path, value: Value) {
    let temp = path.with_extension("tmp");
    fs::write(&temp, value.to_string()).unwrap();
    fs::rename(temp, path).unwrap();
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let arg = |name| &args[args.iter().position(|s| s == name).unwrap() + 1];
    let root = std::env::var_os("SKIFF_CF_TEST").expect("test fixture requires SKIFF_CF_TEST");
    let root = Path::new(&root);
    let config = Path::new(arg("--config"));
    let runtime = config.parent().unwrap();
    let pid = std::process::id();
    if let Some(index) = args.iter().position(|s| s == "--origincert") {
        let command = &args[index + 2..];
        let cert = Path::new(arg("--origincert"));
        let home = std::env::var("HOME").unwrap();
        assert_eq!(cert, Path::new(&home).join(".cloudflared/cert.pem"));
        capture(
            &root.join(format!("setup-{pid}.json")),
            json!({
                "pid":pid, "command":command, "runtime":runtime, "home":home,
                "token":std::env::var("TUNNEL_TOKEN").ok(),
                "force_dns":std::env::var("TUNNEL_FORCE_PROVISIONING_DNS").ok()
            }),
        );
        match command[0].as_str() {
            "login" => {
                eprintln!("https://dash.cloudflare.com.evil.example/argotunnel?callback=wrong");
                eprintln!("https://dash.cloudflare.com/argotunnel?aud=&callback=https%3A%2F%2Flogin.cloudflareaccess.org%2Ftest");
                while root.join("hold_login").exists() {
                    std::thread::sleep(Duration::from_millis(50));
                }
                fs::write(cert, "fake-account-secret-must-not-leak").unwrap();
            }
            "list" => assert!(cert.is_file()),
            "create" => {
                let id = format!("{:032x}", SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos());
                let id = format!("{}-{}-{}-{}-{}", &id[..8], &id[8..12], &id[12..16], &id[16..20], &id[20..]);
                fs::write(arg("--credentials-file"), json!({"TunnelID":id, "TunnelSecret":"fake-tunnel-secret-must-not-leak"}).to_string()).unwrap();
            }
            "route" => {
                assert_eq!(command[1], "dns");
                assert!(!args.iter().any(|s| s == "--overwrite-dns"));
                if root.join("fail_dns").exists() {
                    eprintln!("fake-account-secret-must-not-leak");
                    std::process::exit(7);
                }
                let suffix = if root.join("wrong_zone").exists() { ".other.net" } else { "" };
                let hostname = format!("{}{suffix}", command.last().unwrap());
                eprintln!("{}", json!({"message":format!("Added CNAME {hostname} which will route to this tunnel")}));
            }
            _ => panic!("unexpected setup command: {command:?}"),
        }
        return;
    }
    capture(
        &root.join(format!("{pid}.json")),
        json!({
            "pid":pid, "args":args, "config":serde_json::from_slice::<Value>(&fs::read(config).unwrap()).unwrap(),
            "token":std::env::var("TUNNEL_TOKEN").ok(), "runtime":runtime
        }),
    );
    if root.join("fail").exists() {
        eprintln!("{}", json!({"message":"private-token-must-not-leak"}));
        std::process::exit(7);
    }
    let server = TcpListener::bind("127.0.0.1:0").unwrap();
    eprintln!("{}", json!({"message":format!("Starting metrics server on {}/metrics", server.local_addr().unwrap())}));
    if args.iter().any(|s| s == "--url") {
        eprintln!("{}", json!({"message":"| https://test-river.trycloudflare.com |"}));
    }
    fs::write(arg("--pidfile"), pid.to_string()).unwrap();
    for connection in server.incoming() {
        let mut stream = connection.unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        let mut reader = BufReader::new(&stream);
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                break;
            }
        }
        let status = if root.join("offline").exists() { "503 Unavailable" } else { "200 OK" };
        let _ = write!(stream, "HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    }
}
