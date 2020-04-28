// holds everything related to displaying current status on CLI
use crossbeam::crossbeam_channel::{Receiver};
use std::io::{stdout, Write};

use crate::app::ThreadWorker;
use crate::logging::debug;
use std::net::IpAddr;

pub enum StatusUpdate {
    PeerConnect(IpAddr),
    PeerDisconnect(IpAddr),
}

pub fn init_status_worker(
    status_recv: Receiver<StatusUpdate>
) -> ThreadWorker {
    debug(String::from("initializing status worker thread"));

    let handle = std::thread::spawn(move || {
        let mut connected_peers = 0;
        loop {
            match status_recv.recv() {
                Ok(update) => {
                    match update {
                        StatusUpdate::PeerConnect(ip) => {
                            connected_peers += 1;
                        },
                        StatusUpdate::PeerDisconnect(ip) => {
                            connected_peers -= 1;
                        }
                    }
                    // rewrite status to std out
                    print!("\r Conn Peers: {}", connected_peers);
                    stdout().flush();
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