use rand::Rng;
use std::path::Path;
use std::collections::VecDeque;
use byteorder::{BigEndian, ByteOrder};
use lava_torrent::torrent::v1::Torrent;
use std::net::{UdpSocket, IpAddr};
use std::time::Duration;
use std::{thread, fs};
use failure::Error;
use crossbeam::crossbeam_channel::{unbounded, Sender, Receiver};

use crate::peer::{attempt_peer_download, attempt_peer_connect};
use crate::{get_socket_addr, perform_connection};
use crate::announce::perform_announce;
use crate::pieces::PieceManager;
use crate::download::{Block, WorkChunk};

const CONNECT_WORKERS: usize = 4;
const DL_WORKERS: usize = 4;

type PeerAddr = (IpAddr, u16);

pub struct ThreadWorker {
    pub id: usize,
    pub thread: Option<thread::JoinHandle<()>>
}

fn get_torrent_size(t: &Torrent) -> i64 {
    // calculate how many files and total torrent size
    if t.files.is_none() {
        // sum all file sizes
        t.files.as_ref().unwrap().iter().map(|f| f.length).sum()
    } else {
        // report length
        t.length
    }
}

fn make_output_dir(filename: &str) -> String {
    let output_dir = format!("download/{}_output", filename);
    println!("Creating output dir: {}", output_dir);
    if let Err(_e) = fs::create_dir_all(output_dir.clone()) {
        panic!("Couldnt create output dir: {:?}", &output_dir);
    }
    output_dir
}


pub fn join_connection_workers(worker_name: &str, mut workers:  Vec<ThreadWorker>) {
    // wait for all con workers to be done
    for worker in &mut workers {
        println!("joining on {} worker {}", worker_name, worker.id);
        if let Some(thread) = worker.thread.take() {
            thread.join().unwrap();
        }
    }
    println!("All {} workers shut down/joined", worker_name);
}

pub struct App {

}

impl App {
    pub fn new() -> Self {
        App {
        }
    }

    fn seed_work_channel(&mut self, work_queue: VecDeque<WorkChunk>, work_sender: Sender<WorkChunk>) {
        println!("Seeding work channel with {} entries", work_queue.len());
        for work in work_queue {
            work_sender.send(work);
        }
        println!("All work seeded");
    }

    fn seed_peer_channel(&mut self, peer_addrs: Vec<PeerAddr>, peer_sender: Sender<PeerAddr>) {
        println!("Seeding peer channel with {} peers", peer_addrs.len());
        // put all peers on channel
        for p in &peer_addrs {
            let (addr, port) = p;
            println!("-> {:?}:{:?}", addr, port);
            peer_sender.send(p.clone());
        }
    }

    fn init_conn_dl_workers(&mut self,
                            info_hash_array: [u8;20],
                            peer_id: [u8;20],
                            peer_recv: Receiver<PeerAddr>,
                            block_sender: Sender<Block>,
                            work_recv: Receiver<WorkChunk>) -> Vec<ThreadWorker>{
        println!("Initializing connections and download threads/workers");

        // start eligible peer/downloading threads
        let mut dl_workers = Vec::new();
        for i in 0..DL_WORKERS {
            let block_sender_clone = block_sender.clone();
            let peer_recv_clone = peer_recv.clone();
            let work_recv_clone = work_recv.clone();

            let dl_thread = thread::spawn(move || {
                loop {
                    let next = peer_recv_clone.recv();

                    match next {
                        Ok((ip, port)) => {
                            // connect to peer and attempt download
                            let connect_result = attempt_peer_connect(ip.clone(), port.clone(), &info_hash_array, &peer_id);
                            if let Err(e) = connect_result {
                                // couldnt connect to this peer, continue to next
                                println!("Couldnt connect to peer: {:?}", ip);
                                continue;
                            }
                            let mut peer = connect_result.unwrap();

                            // kick off new thread and try downloading
                            println!("DL-thread-{} starting download from peer: {:?}", i, peer);
                            match attempt_peer_download(peer, &work_recv_clone, &block_sender_clone) {
                                Ok(()) => {
                                    println!("Successful peer download");
                                },
                                Err(e) => {
                                    println!("Error peer download: {:?}", e);
                                }
                            }

                        },
                        Err(e) => {
                            println!("error recv: {:?}", e);
                            break;
                        }
                    }
                }

                println!("download thread {} exiting", i);
            });
            dl_workers.push(ThreadWorker{
                id: i,
                thread: Some(dl_thread),
            })
        }

        dl_workers
    }

    fn init_assembly_workers(&mut self, mut piece_man: PieceManager, block_recv: Receiver<Block>) -> Vec<ThreadWorker> {
        println!("Initializing assembly threads/workers");
        // TODO: be able to put failed blocks back on work queue
        let block_proc_handle =  thread::spawn(move || {
            println!("Block processing thread started!");

            loop {
                match block_recv.recv() {
                    Ok(block) => {
                        piece_man.add_block(block);
                        // TODO if something goes wrong adding block, put back on work queue
                    },
                    Err(e) => {
                        println!("Recv Error: {:?}. Breaking", e);
                        break;
                    }
                }
            }
        });

        let mut workers = Vec::new();
        workers.push(ThreadWorker{
            id: 0,
            thread: Some(block_proc_handle)
        });
        workers
    }

    pub fn download(&mut self, filename: &str) -> Result<(), Error>{
        //*
        // OPEN TORRENT
        let filepath = Path::new(filename);

        let torrent = Torrent::read_from_file(filepath).unwrap();

        let info_hash = torrent.info_hash();
        let info_hash_bytes = torrent.info_hash_bytes();
        // turn info hash from vec into byte array of length 20
        let mut info_hash_array = [0u8; 20];
        for i in 0..20 {
            info_hash_array[i] = info_hash_bytes.get(i).unwrap().clone();
        }

        let total_size = get_torrent_size(&torrent);
        let piece_size = torrent.piece_length;
        let announce_url = torrent.announce.as_ref().expect("Need announce");

        // open output dir if not created
        let output_dir = make_output_dir(filename);

        //*
        // CONNECT TO TRACKER
        // // bind socket to local port
        let local_address = "0.0.0.0:34254";
        let sock = UdpSocket::bind(local_address).expect("Couldnt bind to address");

        // set rw timemout on sock
        sock.set_write_timeout(Some(Duration::from_secs(2)));
        sock.set_read_timeout(Some(Duration::from_secs(2)));

        // connect to remote addr (retry on fail)
        let remote_addr = get_socket_addr(announce_url.as_str());

        println!("\n");
        println!("***********************************");
        println!("* TRACKER CONNECTION AND ANNOUNCE *");
        println!("***********************************");

        print!("> Connecting to tracker");
        let max_attempts = 5;
        let mut attempt = 1;
        loop {
            print!("({}): ", attempt);
            match sock.connect(remote_addr) {
                Ok(_) => {
                    println!("connected!");
                    break;
                },
                Err(e) => print!("{:?}... trying again...", e),
            }
            attempt += 1;
            if attempt > max_attempts {
                println!("max attempts reached, quitting");
                panic!();
            }
        }

        // send request packet and return connection id
        let conn_id_bytes = perform_connection(&sock)?;

        // generate persistent peer id and tx id
        let peer_id = rand::thread_rng().gen::<[u8; 20]>();
        let tx_id = rand::thread_rng().gen::<[u8; 4]>();

        //*
        // SEND ANNOUNCE REQUEST
        let announce_resp = perform_announce(
            &sock,
            &conn_id_bytes,
            &info_hash_bytes,
            torrent.length as u64,
            34264,
            &peer_id,
            &tx_id,
        )?;

        // check that tx id is the same
        let tx_id_int = BigEndian::read_u32(&tx_id);
        if tx_id_int != announce_resp.transaction_id {
            panic!("TX id did not equal announce response, quitting");
        }

        //*
        // GENERATE PIECE MANAGER AND WORK QUE
        let mut piece_man = PieceManager::init_from_torrent(&torrent, output_dir.clone());
        let mut work_queue = piece_man.init_work_queue();
        // work_sender puts block chunks on channel, peer channels read

        // make all channels
        let (work_sender, work_recv) = unbounded();
        let (block_sender, block_recv) = unbounded();
        let (peer_sender, peer_recv) = unbounded();

        // initialize all worker threads
        let asm_workers = self.init_assembly_workers(piece_man, block_recv);
        let dl_workers = self.init_conn_dl_workers(
            info_hash_array.clone(),
            peer_id.clone(),
            peer_recv,
            block_sender,
            work_recv);

        // seed peer channel with all peers
        self.seed_peer_channel(announce_resp.addresses, peer_sender);

        // seed work channel with all work chunks
        self.seed_work_channel(work_queue, work_sender);

        // Download happening now

        // start joining worker threads
        join_connection_workers("conn/dl", dl_workers);

        // TODO shut down senders/recvs
        join_connection_workers("assembly", asm_workers);

        println!("Done");

        Ok(())
    }
}


#[cfg(test)]
mod test {
    use super::*;
    use std::net::{IpAddr, SocketAddr, TcpStream};

    #[test]
    fn test_get_torrent_size() {
        // load torrent file data
        let filepath = "tears-of-steel.torrent";
        let torrent = Torrent::read_from_file(filepath).unwrap();
        let size = get_torrent_size(&torrent);
        println!("size: {:?} bytes", size);
        println!("reported length: {:?}", torrent.length);
        assert_eq!(size, 571426507);
        println!(
            "{} kb, {} mb, {} gb",
            size / 1024,
            (size / 1024) / 1024,
            ((size / 1024) / 1024) / 1024
        );
        for f in torrent.files.unwrap() {
            println!("file={:?}, size={:?} bytes", f.path, f.length);
        }
    }

}
