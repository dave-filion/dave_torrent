use rand::Rng;

use byteorder::{BigEndian, ByteOrder};
use lava_torrent::torrent::v1::Torrent;
use std::net::{UdpSocket, IpAddr};
use std::time::Duration;
use std::{thread, fs};
use std::sync::mpsc::channel;
use failure::Error;

use dave_torrent::announce::*;
use dave_torrent::download::{Block};
use dave_torrent::pieces::*;
use dave_torrent::peer::*;
use dave_torrent::*;
use std::path::Path;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex, RwLock};

const CONNECT_WORKERS: usize = 4;

type PeerAddr = (IpAddr, u16);

pub struct ConnWorker {
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


pub fn join_connection_workers(mut workers:  Vec<ConnWorker>) {
    // wait for all con workers to be done
    for worker in &mut workers {
        println!("joining on conn worker {}", worker.id);
        if let Some(thread) = worker.thread.take() {
            thread.join().unwrap();
        }
    }
    println!("All conn workers shut down/joined");

}

fn main() -> Result<(), Error>{
    //*
    // OPEN TORRENT
    let filename = "big-buck-bunny.torrent";
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
    // make with ARC and R/w lock since multiple threads will use it
    let mut work_queue = Arc::new(RwLock::new(work_queue));

    //*
    // GET PEER LIST
    println!("\n");
    println!("*****************");
    println!("* GOT PEER LIST *");
    println!("*****************\n");
    let mut peer_queue = VecDeque::new();
    let peer_addrs = announce_resp.addresses;
    println!("List of {} peers:", peer_addrs.len());
    for p in &peer_addrs {
        let (addr, port) = p;
        println!("-> {:?}:{:?}", addr, port);
        peer_queue.push_front(p.clone());
    }
    println!("Peer queue: {:?}", peer_queue);
    // Queue of peers to try, put in arc with mutex
    let peer_queue = Arc::new(Mutex::new(peer_queue));

    // //*
    // // START PROCESSING THREAD AND MAKE CHANNELS
    let (block_sender, block_recv) = channel::<Block>();

    // let wq_clone = work_queue.clone();
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


    // create tx/rx channels for eligible peers
    let (peer_sender, peer_recv) = channel::<Peer>();

    // start eligible peer/downloading threads
    // TODO one dl thread for now, could spawn a thread for each peer
    let dl_thread = thread::spawn(move || {
        let mut wq_clone = work_queue.clone();
        loop {
            let block_sender_clone = block_sender.clone();
            match peer_recv.recv() {
                Ok(peer) => {
                    println!("got elible peer: {:?}, gonna start downloading", peer);
                    match attempt_peer_download(peer, &mut wq_clone, block_sender_clone) {
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

        println!("download thread exiting");
    });

    // Open connect worker threads
    let mut conn_workers = Vec::new();
    for i in 0..CONNECT_WORKERS {
        println!("Starting connection worker thread: {}", i);

        // make clones for threads
        let peer_sender_clone = peer_sender.clone();
        let pq = peer_queue.clone();

        let conn_thread_handle = thread::spawn(move || {
            loop {
                let mut guard = pq.lock().unwrap();
                match guard.pop_back() {
                    Some(peer_addr) => {
                        let (ip, port) = peer_addr.clone();

                        // release lock (need to do this otherwise lock will be held for whole call)
                        std::mem::drop(guard);

                        // to long running stuff
                        println!("Thread {} trying peer: {:?}", i, ip);
                        match attempt_peer_connect(
                            ip,
                            port,
                            &info_hash_array,
                            &peer_id,
                        ) {
                            Ok(peer) => {
                                // connected to peer ok, put on send to eligible channel
                                println!("Conn thread: {} CONNECT SUCCESS TO {:?}", i, ip);
                                peer_sender_clone.send(peer);
                            },
                            Err(_e) => {
                                println!("Conn thread: {} couldnt connect to ip: {:?}", i, ip);
                            }
                        }
                    },
                    None => {
                        break;
                    }
                }
            }
        });
        conn_workers.push(ConnWorker{
            id: i,
            thread: Some(conn_thread_handle),
        })
    }

    // Wait for all threads to finish and join
    join_connection_workers(conn_workers);

    // safe to shut down peer sender
    std::mem::drop(peer_sender);

    // join download thread
    dl_thread.join().unwrap();

    // wait for block thread
    println!("Waiting for block processing thead to join...");
    block_proc_handle.join().unwrap();

    println!("DONE");
    Ok(())
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

    // #[test]
    // fn test_connect() {
    //     let ip = "216.36.15.101:51413";
    //     let info_hash = [0x20, 0x9c, 0x82, 0x26, 0xb2, 0x99, 0xb3, 0x08, 0xbe, 0xaf, 0x2b, 0x9c, 0xd3, 0xfb, 0x49, 0x21, 0x2d, 0xbd, 0x13, 0xec];
    //     let peer_id = [0xF4, 0x8B, 0x39, 0xC9, 0x3F, 0x7A, 0xBE, 0xA3, 0xFD, 0x84, 0x96, 0xBC, 0xE, 0xB1, 0xF0, 0xFD, 0x7C, 0x3D, 0x8E, 0x42];
    //
    //     // make connection
    //     let mut stream = TcpStream::connect(ip).expect("connect");
    //
    //     // send handshake
    //     let handshake = make_handshake(&peer_id, &info_hash);
    //     stream.write(&handshake);
    //     println!("wrote handshake");
    //
    //     // listen for resp
    //     let mut buf = [0; 128];
    //     let read_res = stream.read(&mut buf);
    //     if let Ok(bytes_read) = read_res {
    //         println!("got result of {} bytes", bytes_read);
    //     } else {
    //         println!("failed to read response");
    //         return;
    //     }
    //
    //     let hs_resp = parse_handshake_response(&buf.to_vec());
    //     println!("HS response:");
    //     print_byte_array("infohash", &hs_resp.info_hash);
    //     print_byte_array("peerid", &hs_resp.peer_id);
    //
    //     // verify handshake is accurate
    //     println!("verifying handshake from {:?}", ip);
    //     if hs_resp.protocol != "BitTorrent protocol" {
    //         println!("Protocol incorrect: {}... breaking", hs_resp.protocol);
    //         return;
    //     } else {
    //         println!("...protocol OK")
    //     }
    //
    //     if hs_resp.info_hash != info_hash.to_vec() {
    //         println!("Info hashes dont match... breaking");
    //         return;
    //     } else {
    //         println!("...info hash OK");
    //     }
    //
    //     println!("Handshake looks good");
    //
    //     // listen for messages, blocking
    //     stream.set_nonblocking(false).expect("couldnt set to blocking");
    //
    //     for i in 0..10 {
    //         let mut msg_buf = [0; 32];
    //         let read_result = stream.read(&mut msg_buf).expect("Read failure");
    //         println!("msg recv of size: {}", read_result);
    //         print_byte_array("msg", &msg_buf);
    //
    //     }
    // }
}
