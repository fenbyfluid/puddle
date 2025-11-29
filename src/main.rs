use anyhow::{Context, Result};
use clap::Parser;
use linmot::udp::{BUFFER_SIZE, DRIVE_PORT, MASTER_PORT, Request, Response, ResponseFlags};
use std::net::{Ipv4Addr, UdpSocket};
use std::thread::sleep;
use std::time::Duration;

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
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, MASTER_PORT))?;
    socket.set_read_timeout(Some(Duration::from_secs(1)))?;
    socket.connect((addr, DRIVE_PORT))?;

    println!("Connected to drive at {:?} from {:?}", socket.peer_addr(), socket.local_addr());

    let mut buffer = [0u8; BUFFER_SIZE];

    loop {
        sleep(Duration::from_secs(1));

        let req = Request {
            response_flags: ResponseFlags::STATUS_FLAGS
                | ResponseFlags::STATE
                | ResponseFlags::ACTUAL_POSITION
                | ResponseFlags::DEMAND_POSITION
                | ResponseFlags::CURRENT
                | ResponseFlags::WARNING_FLAGS
                | ResponseFlags::ERROR_CODE,
            ..Default::default()
        };

        let to_send = match req.to_wire(&mut buffer) {
            Ok(n) => n,
            Err(e) => {
                eprintln!("Failed to serialize request: {e:?}");
                continue;
            }
        };

        if let Err(e) = socket.send(&buffer[..to_send]) {
            eprintln!("{e:?}");
            continue;
        }

        match socket.recv(&mut buffer) {
            Ok(received) => match Response::from_wire(&buffer[..received]) {
                Ok(resp) => {
                    println!("{resp:?}");
                }
                Err(e) => {
                    eprintln!("Failed to parse response: {e:?}");
                    eprintln!("Raw: {:?}", &buffer[..received]);
                }
            },
            Err(e) => eprintln!("{e:?}"),
        }
    }
}
