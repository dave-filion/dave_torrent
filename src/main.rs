use rand::Rng;
use std::io::{Write, Read};

use byteorder::{BigEndian, ByteOrder};
use lava_torrent::torrent::v1::Torrent;
use std::net::{UdpSocket};

use dave_torrent::*;



fn main() {
    // load torrent file data
    let filepath = "tears-of-steel.torrent";

    let torrent = Torrent::read_from_file(filepath).unwrap();

    let info_hash = torrent.info_hash();
    let info_hash_bytes = torrent.info_hash_bytes();
    // turn info hash from vec into byte array of length 20
    let mut info_hash_array = [0u8; 20];
    for i in 0..20 {
        info_hash_array[i] = info_hash_bytes.get(i).unwrap().clone();
    }
    print_byte_array("info hash array",&info_hash_array);

    let total_size = torrent.length;
    let piece_size = torrent.piece_length;
    let announce_url = torrent.announce.expect("Need announce");

    println!("Total size = {} bytes", total_size);
    println!("Piece size = {} bytes", piece_size);
    println!("info hash = {}", info_hash);
    print_byte_array("info hash bytes", &info_hash_bytes);
    println!("announce: {}", announce_url);

    // // bind socket to local port
    let local_address = "0.0.0.0:34254";
    let sock = UdpSocket::bind(local_address).expect("Couldnt bind to address");
    println!("udp socket bound to local port: {:?}", sock);

    // connect to remote addr
    let remote_addr = get_socket_addr(announce_url.as_str());
    sock.connect(remote_addr).expect("couldnt connect");
    println!("socket connected to remote addr = {:?}", sock);

    // send request packet and return connection id
    let conn_id_bytes = send_connect_req(&sock);
    print_byte_array("conn id", &conn_id_bytes);

    // generate persistent peer id and tx id
    let peer_id = rand::thread_rng().gen::<[u8; 20]>();
    let tx_id = rand::thread_rng().gen::<[u8; 4]>();
    print_byte_array("peer_id", &peer_id);
    print_byte_array("tx_id", &tx_id);

    // send announce request
    let announce_resp = send_announce_req(
        &sock,
        &conn_id_bytes,
        &info_hash_bytes,
        torrent.length as u64,
        34264,
        &peer_id,
        &tx_id
    );
    println!("Got announce result: {:?}", announce_resp);
    // check that tx id is the same
    let tx_id_int = BigEndian::read_u32(&tx_id);
    println!("compare tx ids:\nsent: {:?}\nrecv: {:?}", tx_id_int, announce_resp.transaction_id);

    // get list of peers
    let peer_addrs = announce_resp.addresses;
    println!("List of {} peers:", peer_addrs.len());
    for p in &peer_addrs {
        let (addr, port) = p;
        println!("-> {:?}:{:?}", addr, port);
    }

    // try connecting to all peers serially
    for (ip, port) in &peer_addrs {
        match connect_to_peer(ip.clone(), port.clone(), &peer_id) {
            Ok(mut stream) => {
                println!("got tcp stream");
                // send handshake packet
                let handshake = make_handshake(&peer_id, &info_hash_array);
                print_byte_array("tcp handshake", &handshake);

                // write handshake to stream
                let write_result = stream.write(&handshake);
                if let Ok(bytes_wrote) = write_result {
                    println!("wrote {} bytes for tcp handshake", bytes_wrote);

                    // listen for response
                    let mut resp_buf = [0; 512];
                    let read_result = stream.read(&mut resp_buf);
                    if let Ok(bytes_read) = read_result {
                        println!("Read {} bytes from tcp stream", bytes_read);
                        // parse response
                        print_byte_array("handshake response", &resp_buf);
                    } else {
                        println!("Handshake read error");
                    }
                } else {
                    println!("Handshake failed");
                }
            },
            Err(_) => {
                println!("couldnt connect to {:?}", ip);
            }
        }
    }
}
