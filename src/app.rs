use std::path::Path;
use lava_torrent::torrent::v1::Torrent;
use std::net::UdpSocket;
use std::time::Duration;
use crate::{get_socket_addr, perform_connection};
use crate::announce::perform_announce;
use crate::pieces::PieceManager;
use std::thread;
use std::sync::{Arc, RwLock};
use crate::peer::{attempt_peer_download, attempt_peer_connect};

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

    pub fn download(filename: &str) {
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
        // make with ARC and R/w lock since multiple threads will use it
        let mut work_queue = Arc::new(RwLock::new(work_queue));

        let (peer_sender, peer_recv) = unbounded();


        // //*
        // // START PROCESSING THREAD AND MAKE CHANNELS
        let (block_sender, block_recv) = unbounded();

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


        // start eligible peer/downloading threads
        let mut dl_workers = Vec::new();
        for i in 0..DL_WORKERS {
            let mut wq_clone = work_queue.clone();
            let block_sender_clone = block_sender.clone();
            let peer_recv_clone = peer_recv_arc.clone();

            let dl_thread = thread::spawn(move || {
                loop {
                    let guard = peer_recv_clone.lock().unwrap();
                    let next = guard.recv();
                    // let go of lock
                    std::mem::drop(guard);

                    match next {
                        Ok(peer) => {
                            // kick off new thread and try downloading
                            println!("DL-thread-{} got peer: {:?}, gonna start downloading", i, peer);
                            match attempt_peer_download(peer, &mut wq_clone, &block_sender_clone) {
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

        //*
        // GET PEER LIST
        println!("\n");
        println!("*****************");
        println!("* GOT PEER LIST *");
        println!("*****************\n");
        let peer_addrs = announce_resp.addresses;
        println!("List of {} peers:", peer_addrs.len());
        for p in &peer_addrs {
            let (addr, port) = p;
            println!("-> {:?}:{:?}", addr, port);
            peer_sender.send(p);
        }

        // Open connect worker threads
        let mut conn_workers = Vec::new();
        println!("Started looking for eligible peers to connect to...");
        for i in 0..CONNECT_WORKERS {
            // make clones for threads

            let conn_thread_handle = thread::spawn(move || {
                loop {
                    let mut guard = pq.lock().unwrap();
                    match guard.pop_back() {
                        Some(peer_addr) => {
                            let (ip, port) = peer_addr.clone();

                            // release lock (need to do this otherwise lock will be held for whole call)
                            std::mem::drop(guard);

                            // to long running stuff
                            match attempt_peer_connect(
                                ip,
                                port,
                                &info_hash_array,
                                &peer_id,
                            ) {
                                Ok(peer) => {
                                    // connected to peer ok, put on send to eligible channel
                                    println!("Conn-thread-{} connected to {:?}", i, ip);
                                    peer_sender_clone.send(peer);
                                },
                                Err(_e) => {
                                    // println!("Conn thread: {} couldnt connect to ip: {:?}", i, ip);
                                }
                            }
                        },
                        None => {
                            break;
                        }
                    }
                }
            });
            conn_workers.push(ThreadWorker {
                id: i,
                thread: Some(conn_thread_handle),
            })
        }

        // Wait for all threads to finish and join
        // TODO need beter signal to start shutting down
        thread::sleep(Duration::from_secs(1));
        join_connection_workers("conn", conn_workers);

        // safe to shut down peer sender
        std::mem::drop(peer_sender);

        // join download threads
        join_connection_workers("download", dl_workers);

        // wait for block thread
        println!("Waiting for block processing thead to join...");
        block_proc_handle.join().unwrap();

        println!("DONE");
    }
}