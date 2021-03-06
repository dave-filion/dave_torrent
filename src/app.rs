use rand::Rng;
use std::path::Path;
use std::collections::{VecDeque, HashSet};
use byteorder::{BigEndian, ByteOrder};
use lava_torrent::torrent::v1::Torrent;
use std::net::{UdpSocket, IpAddr};
use std::time::Duration;
use std::{thread, fs};
use failure::Error;
use crossbeam::crossbeam_channel::{unbounded, Sender, Receiver};
use reqwest::Url;
use hex::encode;

use crate::peer::{attempt_peer_download, attempt_peer_connect};
use crate::{get_socket_addr, perform_connection, get_protocol};
use crate::announce::{perform_announce, AnnounceResponse};
use crate::pieces::PieceManager;
use crate::download::{Block, WorkChunk};
use crate::logging::{debug};
use std::sync::{Arc, Mutex};
use crate::status_update::{init_status_worker, StatusUpdate};
use failure::_core::str::from_utf8;

const DL_WORKERS: usize = 1;
const PEER_CONNECT_REFRESH: usize = 15; // try refreshing peers ever N secs
const DO_PEER_REATTEMPT: bool = false; // should we attempt to reconnect to peers?

type PeerAddr = (IpAddr, u16);

#[derive(Debug)]
pub enum TrackerProtocol {
    UDP,
    HTTP,
    Unknown(String),
}
pub struct ThreadWorker {
    pub id: usize,
    pub thread: Option<thread::JoinHandle<()>>
}

fn get_torrent_size(t: &Torrent) -> i64 {
    // calculate how many files and total torrent size
    if t.files.is_some() {
        // sum all file sizes
        t.files.as_ref().unwrap().iter().map(|f| f.length).sum()
    } else {
        // report length
        t.length
    }
}

pub fn make_specific_output_dir(dir: &str, torrent_name: &str) -> String {
    let output_dir = format!("{}/{}_output", dir, torrent_name);
    debug(format!("Creating output dir: {}", output_dir));
    if let Err(_e) = fs::create_dir_all(output_dir.clone()) {
        panic!("Couldnt create output dir: {:?}", &output_dir);
    }
    output_dir
}

pub fn make_output_dir(filename: &str) -> String {
    let output_dir = format!("download/{}_output", filename);
    debug(format!("Creating output dir: {}", output_dir));
    if let Err(_e) = fs::create_dir_all(output_dir.clone()) {
        panic!("Couldnt create output dir: {:?}", &output_dir);
    }
    output_dir
}


pub fn join_connection_workers(worker_name: &str, mut workers:  Vec<ThreadWorker>) {
    // wait for all con workers to be done
    for worker in &mut workers {
        if let Some(thread) = worker.thread.take() {
            thread.join().unwrap();
            debug(format!("{}-{} joined", worker_name, worker.id));
        }
    }
    debug(format!("All {} workers shut down/joined", worker_name));
}

pub fn announce_http(announce_url: &str,
                     status_sender: &Sender<StatusUpdate>,
                     info_hash: &str,
                     torrent_length: i64,
                     peer_id: &str,
                     tx_id: &[u8; 4]) -> Result<AnnounceResponse, Error> {
    debug(format!("http announce url : {:?}", announce_url));

    let mut url = Url::parse(announce_url)?;
    url.query_pairs_mut()
        .append_pair("peer_id", peer_id)
        .append_pair("info_hash", info_hash)
        .append_pair("port", "6881")
        .append_pair("uploaded", "0")
        .append_pair("downloaded", "0")
        .append_pair("left", format!("{}", torrent_length).as_str());


    println!("url with query = {:?}", url.as_str());

    // create http request
    let response = reqwest::blocking::get(url);
    match response {
        Ok(body) => {
            println!("got response: {:?}", body.text());
        },
        Err(e) => {
            println!("error : {:?}", e);
        }
    }


    unimplemented!("u");
    // just make http request to tracker url?

    Ok(AnnounceResponse{
        action:0 ,
        transaction_id: 0,
        interval: 0,
        leechers: 0,
        seeders: 0,
        addresses: vec![]
    })
}

pub fn peer_id_str(p: &[u8; 20]) -> String {
    hex::encode(p)
}

// connect and announce to tracker via udp
pub fn announce_udp(announce_url: &str,
                    status_sender: &Sender<StatusUpdate>,
                    info_hash_bytes: &Vec<u8>,
                    torrent_length: i64,
                    peer_id: &[u8; 20],
                    tx_id: &[u8; 4]) -> Result<AnnounceResponse, Error> {
    let local_address = "0.0.0.0:34254";
    let sock = UdpSocket::bind(local_address).expect("Couldnt bind to address");

    // set rw timemout on sock
    sock.set_write_timeout(Some(Duration::from_secs(2)))?;
    sock.set_read_timeout(Some(Duration::from_secs(2)))?;

    // connect to remote addr (retry on fail)
    let remote_addr = get_socket_addr(announce_url);

    debug(format!("> Connecting to tracker"));
    let max_attempts = 5;
    let mut attempt = 1;
    loop {
        print!("({}): ", attempt);
        match sock.connect(remote_addr) {
            Ok(_) => {
                debug(format!("connected!"));
                break;
            },
            Err(e) => debug(format!("{:?}... trying again...", e)),
        }
        attempt += 1;
        if attempt > max_attempts {
            debug(format!("max attempts reached, quitting"));
            panic!();
        }
    }

    // send request packet and return connection id
    let conn_id_bytes = perform_connection(&sock)?;
    status_sender.send(StatusUpdate::Connected)?;

    let announce_resp = perform_announce(
        &sock,
        &conn_id_bytes,
        info_hash_bytes,
        torrent_length as u64,
        34264,
        peer_id,
        tx_id,
    )?;

    // check that tx id is the same
    let tx_id_int = BigEndian::read_u32(tx_id);
    if tx_id_int != announce_resp.transaction_id {
        panic!("TX id did not equal announce response, quitting");
    }
    Ok(announce_resp)
}

pub struct App {
    pub connected_peers: Arc<Mutex<HashSet<IpAddr>>>,
    pub possible_peers: HashSet<PeerAddr>,
}

impl App {
    pub fn new() -> Self {
        App {
            connected_peers: Arc::new(Mutex::new(HashSet::new())),
            possible_peers: HashSet::new(),
        }
    }

    fn seed_work_channel(&mut self, work_queue: VecDeque<WorkChunk>, work_sender: Sender<WorkChunk>) {
        debug(format!("Seeding work channel with {} entries", work_queue.len()));
        for work in work_queue {
            let _result = work_sender.send(work);
        }
        debug(format!("All work seeded"));
    }

    fn seed_peer_channel(&mut self, peer_addrs: Vec<PeerAddr>, peer_sender: Sender<PeerAddr>) {
        debug(format!("Seeding peer channel with {} peers", peer_addrs.len()));
        // put all peers on channel
        for p in &peer_addrs {
            let (addr, port) = p;
            debug(format!("-> {:?}:{:?}", addr, port));
            self.possible_peers.insert(p.clone());
            let _result = peer_sender.send(p.clone());
        }
    }

    fn init_conn_dl_workers(&mut self,
                            info_hash_array: [u8;20],
                            peer_id: [u8;20],
                            peer_recv: Receiver<PeerAddr>,
                            block_sender: Sender<Block>,
                            work_sender: Sender<WorkChunk>,
                            work_recv: Receiver<WorkChunk>,
        status_sender: Sender<StatusUpdate>,
    ) -> Vec<ThreadWorker>{
        debug(format!("Initializing connections and download threads/workers"));

        // start eligible peer/downloading threads
        let mut dl_workers = Vec::new();
        for i in 0..DL_WORKERS {
            let block_sender_clone = block_sender.clone();
            let peer_recv_clone = peer_recv.clone();
            let work_recv_clone = work_recv.clone();
            let work_sender_clone = work_sender.clone();
            let status_sender_clone = status_sender.clone();

            // thread local copy of connected peers hash
            let connected_peers = self.connected_peers.clone();

            let dl_thread = thread::spawn(move || {
                loop {
                    let next = peer_recv_clone.recv();

                    match next {
                        Ok((ip, port)) => {
                            // connect to peer and attempt download
                            let connect_result = attempt_peer_connect(ip.clone(), port.clone(), &info_hash_array, &peer_id);
                            if let Err(e) = connect_result {
                                // couldnt connect to this peer, continue to next
                                // debug(format!("DL-{} Couldnt connect to peer: {:?}", i, ip);
                                debug(format!("couldnt connect to peer: {:?}", ip));
                                continue;
                            }
                            let peer = connect_result.unwrap();

                            // send status update
                            let _r = status_sender_clone.send(StatusUpdate::PeerConnect(ip.clone()));
                            connected_peers.lock().unwrap().insert(ip.clone());

                            // kick off new thread and try downloading
                            debug(format!("DL-{} starting download from peer: {:?}", i, peer));
                            match attempt_peer_download(peer, &work_sender_clone, &work_recv_clone, &block_sender_clone) {
                                Ok(()) => {
                                    debug(format!("DL-{} Successful peer download, disconnecting from peer {:?}", i, ip));
                                    // self.remove_connected_peer(&ip);
                                    connected_peers.lock().unwrap().remove(&ip);
                                    let _r = status_sender_clone.send(StatusUpdate::PeerDisconnect(ip.clone()));
                                },
                                Err(e) => {
                                    debug(format!("DL-{} Error peer download: {:?}", i, e));
                                    // self.remove_connected_peer(&ip);
                                    connected_peers.lock().unwrap().remove(&ip);
                                    let _r = status_sender_clone.send(StatusUpdate::PeerDisconnect(ip.clone()));
                                }
                            }

                        },
                        Err(e) => {
                            debug(format!("DL-{} error recv: {:?}", i, e));
                            break;
                        }
                    }
                }

                debug(format!("download thread {} exiting", i));
            });
            dl_workers.push(ThreadWorker{
                id: i,
                thread: Some(dl_thread),
            })
        }

        dl_workers
    }

    fn init_assembly_workers(&mut self, mut piece_man: PieceManager, block_recv: Receiver<Block>) -> Vec<ThreadWorker> {
        debug(format!("Initializing assembly threads/workers"));
        // TODO: be able to put failed blocks back on work queue
        let block_proc_handle =  thread::spawn(move || {
            debug(format!("Block processing thread started!"));

            loop {
                match block_recv.recv() {
                    Ok(block) => {
                        piece_man.add_block(block);
                        // TODO if something goes wrong adding block, put back on work queue
                    },
                    Err(e) => {
                        debug(format!("Recv Error: {:?}. Breaking", e));
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

    pub fn connected_to_peer (&self, ip: &IpAddr)  -> bool {
        self.connected_peers.lock().unwrap().contains(ip)
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

        let _total_size = get_torrent_size(&torrent);
        let _piece_size = torrent.piece_length;
        let announce_url = torrent.announce.as_ref().expect("Need announce");

        // open output dir if not created
        let output_dir = make_output_dir(filename);

        // init status worker channel and thread
        let (status_sender, status_recv) = unbounded();
        let status_worker = init_status_worker(status_recv);

        // generate persistent peer id and tx id
        let peer_id = rand::thread_rng().gen::<[u8; 20]>();
        let tx_id = rand::thread_rng().gen::<[u8; 4]>();

        // determine announce url to use
        let (prot, announce_url) = get_announce_url(&torrent);
        debug(format!("protocol is {:?}, url={}", prot, announce_url));

        let announce_resp = match prot {
            TrackerProtocol::UDP => announce_udp(
                announce_url.as_str(),
                &status_sender,
                &info_hash_bytes,
                torrent.length,
                &peer_id,
                &tx_id),
            TrackerProtocol::HTTP => announce_http(
                announce_url.as_str(),
                &status_sender,
                info_hash.as_str(),
                torrent.length,
                peer_id_str(&peer_id).as_str(),
                &tx_id
            ),
            TrackerProtocol::Unknown(p) => {
                panic!("unknown protocol {:?}, cant connect", p);
            }
        }?;

        // TODO send seeder and leecher values
        status_sender.send(StatusUpdate::Announced);
        println!("ANNOUNCE RESP: {:?}", announce_resp);

        //*
        // GENERATE PIECE MANAGER AND WORK QUE
        let mut piece_man = PieceManager::init_from_torrent(&torrent, output_dir.clone());
        let work_queue = piece_man.init_work_queue();
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
            peer_recv.clone(),
            block_sender.clone(),
            work_sender.clone(),
            work_recv.clone(),
            status_sender.clone(),
        );

        // seed channels with data from announce response
        self.seed_peer_channel(announce_resp.addresses, peer_sender.clone());
        self.seed_work_channel(work_queue, work_sender.clone());

        // Download happening now, need a way to refresh peers to try reconnect
        // Do peer refresh
        if DO_PEER_REATTEMPT {
            loop {
                // wait a bit
                thread::sleep(Duration::from_secs(PEER_CONNECT_REFRESH as u64));

                debug(format!("Doing peer refresh"));
                for (ip, port) in &self.possible_peers {
                    debug(format!("checking peer {:?}", ip));
                    if self.connected_to_peer(ip) {
                        debug(format!("already connected to peer, skipping!"));
                    } else {
                        debug(format!("not connected, adding to list"));
                        // send to peer connection
                        let _result = peer_sender.send((ip.clone(), port.clone()));
                    }
                }
            }
        }

        thread::sleep(Duration::from_secs(30));

        // Shutdown
        drop(peer_sender);
        join_connection_workers("conn/dl", dl_workers);
        join_connection_workers("assembly", asm_workers);

        debug(format!("Done"));

        Ok(())
    }
}

pub fn get_announce_url(t: &Torrent) -> (TrackerProtocol, String){
    // check if announce list is present
    if let Some(announce_list) = &t.announce_list {
        for list in announce_list {
            for announce in list {
                let protocol = get_protocol(announce.as_str());
                if let TrackerProtocol::UDP = protocol {
                    println!("found a udp protocol, returning");
                    return (TrackerProtocol::UDP, announce.clone())
                }
            }
        }
    }

    // otherwise just return the http
    match &t.announce {
        Some(url) => {
            return (TrackerProtocol::HTTP, url.clone())
        },
        None => {
            panic!("Dont know what announce url to use!")
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use std::net::{IpAddr, SocketAddr, TcpStream};
    use crate::print_torrent_info;

    #[test]
    fn test_manjaro_torrent_percent_encoding() {
        let filepath = "manjaro-kde-20.0-200426-linux56.iso.torrent";
        let torrent = Torrent::read_from_file(filepath).unwrap();
        print_torrent_info(&torrent);
        let (prot, url) = get_announce_url(&torrent);
        println!("pro = {:?} url = {}", prot, url);
    }

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
