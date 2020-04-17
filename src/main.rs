use rand::Rng;
use std::io::{Write, Read};

use byteorder::{BigEndian, ByteOrder};
use lava_torrent::torrent::v1::Torrent;
use std::net::{UdpSocket};
use std::time::Duration;

use dave_torrent::*;


fn get_torrent_size(t : &Torrent) -> i64 {
    // calculate how many files and total torrent size
        if t.files.is_none() {
            // sum all file sizes
            t.files.as_ref().unwrap().iter().map(|f| f.length).sum()
        } else {
            // report length
            t.length
        }
}


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

    let total_size = get_torrent_size(&torrent);
    let piece_size = torrent.piece_length;
    let announce_url = torrent.announce.expect("Need announce");


    println!("TORRENT INFO");
    println!("------------------------------------");
    println!("Total size = {} bytes", total_size);
    println!("Piece size = {} bytes", piece_size);
    println!("info hash = {}", info_hash);
    println!("announce url: {}", announce_url);
    println!("------------------------------------\n");

    println!("Connecting to tracker");
    // // bind socket to local port
    let local_address = "0.0.0.0:34254";
    let sock = UdpSocket::bind(local_address).expect("Couldnt bind to address");
    println!("udp socket bound to local port: {:?}", sock);

    // set rw timemout on sock (5 sec timeout)
    sock.set_write_timeout(Some(Duration::from_secs(5)));
    sock.set_read_timeout(Some(Duration::from_secs(5)));

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
    //print_byte_array("peer_id", &peer_id);
    //print_byte_array("tx_id", &tx_id);

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
        println!("Connecting to peer: {:?}:{}", ip, port);
        match connect_to_peer(ip.clone(), port.clone(), &peer_id) {
            Ok(mut stream) => {
                // send handshake packet
                let handshake = make_handshake(&peer_id, &info_hash_array);
                // print_byte_array("tcp handshake", &handshake);

                println!("Connected to {:?}:{}, writing handshake to stream...", ip, port);
                // write handshake to stream
                let write_result = stream.write(&handshake);
                if let Ok(bytes_wrote) = write_result {

                    println!("Waiting for response from {:?}:{}", ip, port);
                    // listen for response
                    let mut resp_buf = [0; 128]; // needs to be more then 64
                    let read_result = stream.read(&mut resp_buf);
                    if let Ok(bytes_read) = read_result {
                        println!("Recieved {} byte handshake response from {:?}:{}", bytes_read, ip, port);
                        // parse response
                        // print_byte_array("handshake response", &resp_buf);
                        let handshake_response = parse_handshake_response(&resp_buf.to_vec());
                        // verify response is accurate
                        println!("verifying handshake from {:?}:{}", ip, port);
                        if handshake_response.protocol != "BitTorrent protocol" {
                            println!("Protocol incorrect: {}", handshake_response.protocol);
                            // release connection
                        } else {
                            println!("...protocol OK")
                        }

                        if handshake_response.info_hash != info_hash_bytes {
                            println!("Info hashes dont match...");
                        } else {
                            println!("...info hash OK");
                        }

                        print_byte_array("remote peer id", &handshake_response.peer_id);

                        // handshake is fine, start listening for have message
                        println!("Handshake to {:?}:{} SUCCESS, listening for messages", ip, port);

                        // loop TODO this doesnt work, just returns 0 bytes read over and over
                        loop {
                            let mut buf = [0; 128];
                            let read_result = stream.read(&mut buf);
                            if let Ok(bytes_read) = read_result {
                                println!("Read {} byte message from {:?}:{}", bytes_read, ip, port);
                                print_byte_array("peer msg", &buf);

                                // get len of message (first 4 bytes)
                                let (len_bytes, _) = get_n_bytes_at(&buf.to_vec(), 0, 4);
                                let len = BigEndian::read_u32(&len_bytes);

                                println!("total msg len is : {}", len);

                            } else {
                                println!("Failed to read more");
                                break;
                            }
                        }

                    } else {
                        println!("Reading handshake response from stream failed for {:?}:{}", ip, port);
                    }
                } else {
                    println!("Writing handshake to stream failed for {:?}:{}", ip, port);
                }
            },
            Err(_) => {
                println!("couldnt connect to {:?}", ip);
            }
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_get_torrent_size() {
        // load torrent file data
        let filepath = "tears-of-steel.torrent";
        let torrent = Torrent::read_from_file(filepath).unwrap();
        let size = get_torrent_size(&torrent);
        println!("size: {:?} bytes",size);
        println!("reported length: {:?}", torrent.length);
        assert_eq!(size, 571426507);
        println!("{} kb, {} mb, {} gb", size / 1024, (size/1024)/1024, ((size/1024)/1024)/1024);
        for f in torrent.files.unwrap() {
            println!("file={:?}, size={:?} bytes", f.path, f.length);
        }
    }
}