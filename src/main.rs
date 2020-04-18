use rand::Rng;
use std::io::{Read, Write};

use byteorder::{BigEndian, ByteOrder};
use lava_torrent::torrent::v1::Torrent;
use std::net::UdpSocket;
use std::time::Duration;
use std::thread;
use std::sync::mpsc::channel;
use failure::Error;

use dave_torrent::announce::*;
use dave_torrent::download::{make_work_queue, Block};
use dave_torrent::peer::*;
use dave_torrent::*;


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

fn main() -> Result<(), Error>{
    //*
    // OPEN TORRENT
    let filepath = "big-buck-bunny.torrent";

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
    let announce_url = torrent.announce.expect("Need announce");

    println!("TORRENT INFO");
    println!("------------------------------------");
    println!("Total size = {} bytes", total_size);
    println!("Piece size = {} bytes", piece_size);
    println!("Piece number = {}", torrent.pieces.len());
    println!("info hash = {}", info_hash);
    println!("announce url: {}", announce_url);
    println!("------------------------------------\n");

    //*
    // CONNECT TO TRACKER
    // // bind socket to local port
    let local_address = "0.0.0.0:34254";
    let sock = UdpSocket::bind(local_address).expect("Couldnt bind to address");
    println!("udp socket bound to local port: {:?}", sock);

    // set rw timemout on sock
    sock.set_write_timeout(Some(Duration::from_secs(2)));
    sock.set_read_timeout(Some(Duration::from_secs(2)));

    // connect to remote addr (retry on fail)
    print!("Connecting to tracker...");
    let remote_addr = get_socket_addr(announce_url.as_str());

    // TODO move to function
    let max_attempts = 5;
    let mut attempt = 1;
    loop {
        print!("({}) ...", attempt);
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
    // GET PEER LIST
    let peer_addrs = announce_resp.addresses;
    println!("List of {} peers:", peer_addrs.len());
    for p in &peer_addrs {
        let (addr, port) = p;
        println!("-> {:?}:{:?}", addr, port);
    }

    //*
    // GENERATE WORK QUEUE
    let chunk_size = BLOCK_SIZE;
    let mut work_queue = make_work_queue(
        torrent.pieces.len(),
        torrent.piece_length as u32,
        chunk_size);

    //*
    // START PROCESSING THREAD AND MAKE CHANNELS
    let (tx, rx) = channel::<Block>();
    let block_proc_handle =  thread::spawn(move || {
        println!("Block processing thread started!");

        loop {
            println!("Waiting for incoming blocks...");
            match rx.recv() {
                Ok(block) => {
                    println!("Recv block: {} for processing", block.block_id)
                    // TODO actually do something with block
                },
                Err(e) => {
                    println!("Recv Error: {:?}. Breaking", e);
                    break;
                }
            }
        }
    });

    //*
    // ATTEMPT CONNECTING TO EACH PEER SERIALLY
    for (ip, port) in &peer_addrs {
        if let Err(e) = attempt_peer_download(
            ip.clone(),
            port.clone(),
            &info_hash_array,
            &peer_id,
            &mut work_queue,
            tx.clone(),
        ) {
            println!("Couldn't download from peer.");
        }
    }

    // wait for processing thread
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
