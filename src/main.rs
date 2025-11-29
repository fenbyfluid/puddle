use std::net::{Ipv4Addr, UdpSocket};
use std::thread::sleep;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::Parser;

use linmot::{Request, Response, ResponseFlags, CONTROL_DRIVE_PORT, CONTROL_MASTER_PORT};

pub mod linmot;
mod reader;
mod writer;

#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Options {
    drive_address: String,
}

fn main() -> Result<()> {
    let options = Options::parse();

    connect_to_drive(&options.drive_address).context("Failed to connect to drive")?;

    Ok(())
}

fn connect_to_drive(addr: &str) -> Result<()> {
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, CONTROL_MASTER_PORT))?;
    socket.set_read_timeout(Some(Duration::from_secs(1)))?;
    socket.connect((addr, CONTROL_DRIVE_PORT))?;

    println!(
        "Connected to drive at {:?} from {:?}",
        socket.peer_addr(),
        socket.local_addr()
    );

    let mut buffer = [0u8; 64];

    loop {
        sleep(Duration::from_secs(1));

        let req = Request {
            response_flags: ResponseFlags::STATUS_WORD
                | ResponseFlags::STATE_VAR
                | ResponseFlags::ACTUAL_POSITION
                | ResponseFlags::DEMAND_POSITION
                | ResponseFlags::CURRENT
                | ResponseFlags::WARN_WORD
                | ResponseFlags::ERROR_CODE
                | ResponseFlags::MONITORING_CHANNEL,
            ..Default::default()
        };

        let to_send = match req.to_wire(&mut buffer) {
            Ok(n) => n,
            Err(e) => {
                println!("Failed to serialize request: {e:?}");
                continue;
            }
        };

        match socket.send(&buffer[..to_send]) {
            Ok(_) => (),
            Err(e) => {
                println!("{:?}", e);
                continue;
            }
        };

        match socket.recv(&mut buffer) {
            Ok(received) => match Response::from_wire(&buffer[..received]) {
                Ok(resp) => {
                    println!("{:?}", resp);
                }
                Err(e) => {
                    println!("Failed to parse response: {e:?}");
                    println!("Raw: {:?}", &buffer[..received]);
                }
            },
            Err(e) => println!("{:?}", e),
        }
    }
}
