// holds everything related to displaying current status on CLI
use crossbeam::crossbeam_channel::{Receiver};
use std::io::{stdout, Write};

use crate::app::ThreadWorker;
use crate::logging::debug;
use std::net::IpAddr;

pub enum StatusUpdate {
    Connected,
    Announced,
    PeerConnect(IpAddr),
    PeerDisconnect(IpAddr),
}

pub fn init_status_worker(
    status_recv: Receiver<StatusUpdate>
) -> ThreadWorker {
    debug(String::from("initializing status worker thread"));

    let handle = std::thread::spawn(move || {
        let mut connected_peers = 0;
        print!("\rSTARTING...");
        loop {
            match status_recv.recv() {
                Ok(update) => {
                    match update {
                        StatusUpdate::Connected => {
                            print!("\rConnected...");
                        },
                        StatusUpdate::Announced => {
                            print!("\rConnected... Announced...Finding peers...");
                        },
                        StatusUpdate::PeerConnect(ip) => {
                            connected_peers += 1;
                            print!("\rConn Peers: {}", connected_peers);
                        },
                        StatusUpdate::PeerDisconnect(ip) => {
                            connected_peers -= 1;
                            print!("\rConn Peers: {}", connected_peers);
                        }
                    }
                    // rewrite status to std out
                    if let Err(e) = stdout().flush() {
                        println!("Error flushing stdout while updating status! {:?}", e);
                    }
                },
                Err(_e) => {
                    break;
                }
            }
        }
    });

    ThreadWorker {
        id: 0,
        thread: Some(handle),
    }
}