use rand::{Rng};
use std::io::{Cursor};

use bytebuffer::ByteBuffer;
use byteorder::{BigEndian, ByteOrder, ReadBytesExt};
use std::fmt::Error;
use std::net::{IpAddr, SocketAddr, TcpStream, ToSocketAddrs, UdpSocket};
use std::str::from_utf8;
use std::time::Duration;

pub mod download;
pub mod peer;
pub mod announce;


pub fn print_byte_array_len(header: &str, bytes: &[u8], until: usize) {
    print!("{} => [", header);
    for (i, b) in bytes.iter().enumerate() {
        if i == until {
            break;
        }

        if i == 0 {
            print!("0x{:X?}", b);
        } else {
            print!(", 0x{:X?}", b);
        }
    }
    println!("]");
}

pub fn print_byte_array(header: &str, bytes: &[u8]) {
    print!("{} => [", header);
    for (i, b) in bytes.iter().enumerate() {
        if i == 0 {
            print!("0x{:X?}", b);
        } else {
            print!(", 0x{:X?}", b);
        }
    }
    println!("]");
}

pub fn make_connect_packet() -> Vec<u8> {
    let mut bytebuf = ByteBuffer::new();

    // write connection id
    let conn_id: u64 = 0x41727101980; // constant for connecting
    bytebuf.write_u64(conn_id);

    // write action = 0
    let action: u32 = 0;
    bytebuf.write_u32(action);

    // write random tx id (4 bytes)
    let random_tx_id = rand::thread_rng().gen::<[u8; 4]>();
    bytebuf.write_bytes(&random_tx_id);

    bytebuf.to_bytes()
}

pub fn get_conn_id_from_connect_response(response_buf: &[u8]) -> (u64, Vec<u8>) {
    // get connection id from response (at index 7, 8 bytes)
    let mut conn_id_buff = [0; 8];
    let mut j = 0;
    for i in 8..16 {
        let byte = response_buf[i];
        conn_id_buff[j] = byte;
        j += 1;
    }
    // convert conn id to u64
    let mut byte_rdr = Cursor::new(conn_id_buff);
    (
        byte_rdr.read_u64::<BigEndian>().unwrap(),
        conn_id_buff.to_vec(),
    )
}


pub fn get_socket_addr(announce_url: &str) -> SocketAddr {
    let pieces: Vec<String> = announce_url.split("://").map(|s| s.to_string()).collect(); // seperates protocol from url
    let protocol = pieces[0].as_str();
    if protocol != "udp" {
        panic!("cant handle non udp protocol: {}", protocol);
    }

    let url = pieces[1].as_str();
    url.to_socket_addrs().unwrap().next().unwrap()
}

// Tries to send connect request and receive response back, returns connection id bytes
pub fn perform_connection(sock: &UdpSocket) -> Result<Vec<u8>, Error> {
    let connect_packet = make_connect_packet();
    //print_byte_array("Connect request", &connect_packet);

    let max_attempts = 5;
    let mut attempt = 1;

    loop {
        if attempt > max_attempts {
            println!("Max attempts reached... quitting");
            return Err(Error)
        }

        println!("> Perform connection attempt ({}):", attempt);

        print!("Sending connect request...");
        // send message to remote udp port
        match sock.send(&connect_packet) {
            Ok(_) => {
                println!("sent!");
            },
            Err(e) => {
                println!("Error sending conn request: {:?}", e);
                attempt += 1; // try again
                continue;
            }
        }

        print!("Waiting for response...");
        let mut response_buf = [0; 16];
        match sock.recv(&mut response_buf) {
            Ok(bytes_read) => {
                println!("got {} byte response!", bytes_read);
                // extract conn id
                let (_conn_id_int, conn_id_bytes) = get_conn_id_from_connect_response(&response_buf);
                return Ok(conn_id_bytes)
            },
            Err(e) => {
                println!("Error recv response: {:?}", e);
                attempt += 1; // try again
                continue
            }
        }
    }
}

pub fn get_u32_at(from: &[u8], index: usize) -> u32 {
    let mut buf = [0; 4];
    let mut j = 0;
    for i in index..index + 4 {
        let b = from[i];
        buf[j] = b;
        j += 1;
    }
    // conert from bytes to u32
    BigEndian::read_u32(&buf)
}



// reads n bytes from buf at index and returns vec, returns new index also
pub fn get_n_bytes_at(buf: &Vec<u8>, start_i: usize, n_bytes: usize) -> (Vec<u8>, usize) {
    let mut v = Vec::new();
    let mut j = start_i;
    for i in start_i..start_i + n_bytes {
        let byte = buf.get(i).unwrap();
        v.push(byte.clone());
        j = i;
    }
    (v, j)
}


#[cfg(test)]
mod test {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr};
    use std::str::from_utf8;
    use lava_torrent::torrent::v1::Torrent;
    use crate::peer::*;
    use crate::announce::*;
    use crate::download::*;

    #[test]
    fn test_make_announce_packet() {
        let port = 6969;
        let torrent_size = 1; // 1 so its easy to see in byte form
        let peer_id = rand::thread_rng().gen::<[u8; 20]>();
        let tx_id = [0x10, 0x20, 0x30, 0x40];
        let conn_id_bytes = [0x10, 0x38, 0x94, 0xC3, 0x73, 0x6B, 0x76, 0xB2].to_vec();
        let info_hash_bytes = [
            0xAA, 0x16, 0x30, 0x38, 0x78, 0x53, 0x79, 0x81, 0x90, 0x75, 0x43, 0x56, 0x11, 0x51,
            0x73, 0x33, 0x45, 0x89, 0x19, 0xCC,
        ]
        .to_vec();
        let result = make_announce_packet(
            &conn_id_bytes,
            &info_hash_bytes,
            torrent_size,
            port,
            &peer_id,
            &tx_id,
        );

        print_byte_array("result", &result);
        assert_eq!(result.len(), 98);
    }

    #[test]
    fn test_parse_announce_response() {
        let sample_announce_resp = [
            0x0, 0x0, 0x0, 0x1, 0x65, 0x48, 0x92, 0x2D, 0x0, 0x0, 0x6, 0xB8, 0x0, 0x0, 0x0, 0x1,
            0x0, 0x0, 0x0, 0x9, 0x42, 0x6C, 0x62, 0x33, 0x85, 0xD8, 0xDB, 0x5B, 0x8B, 0xEB, 0xE8,
            0x74, 0xDB, 0x5B, 0x8B, 0xEB, 0xC8, 0x54, 0xD8, 0x24, 0xF, 0x65, 0xC8, 0xD5, 0xD0,
            0x48, 0xC0, 0xE5, 0x83, 0x1E, 0x54, 0x11, 0x35, 0xA9, 0xC8, 0xD5, 0x4F, 0x58, 0xB2,
            0x94, 0x60, 0x87, 0x4A, 0x3A, 0x73, 0x24, 0xBF, 0x7F, 0x47, 0x3A, 0xFC, 0x7A, 0x1E,
            0xC9, 0x44, 0xA8, 0xB2, 0x15, 0xC8, 0xD5,
        ]
        .to_vec();

        let result = parse_announce_response(&sample_announce_resp);
        println!("result => {:?}", result);
        assert_eq!(result.action, 1);
        assert_eq!(result.leechers, 1);
        assert_eq!(result.seeders, 9);

        // try using From
        let result: AnnounceResponse = sample_announce_resp.into();
        println!("result => {:?}", result);
        assert_eq!(result.action, 1);
        assert_eq!(result.leechers, 1);
        assert_eq!(result.seeders, 9);

        // check addresses
        assert_eq!(result.addresses.len(), 10);
    }

    #[test]
    fn test_get_u32_at_position() {
        let buffer = [0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0, 0];
        let result = get_u32_at(&buffer, 4);
        assert_eq!(result, 2);

        let result = get_u32_at(&buffer, 0);
        assert_eq!(result, 0);
    }

    #[test]
    fn test_make_handshake() {
        // make random 20 byte peer id and info hash
        let peer_id = rand::thread_rng().gen::<[u8; 20]>();
        let info_hash = rand::thread_rng().gen::<[u8; 20]>();

        let packet = make_handshake(&peer_id, &info_hash);
        print_byte_array("handshake", &packet);
        print_byte_array("peer_id", &peer_id);
        let pstr = [
            0x42, 0x69, 0x74, 0x54, 0x6F, 0x72, 0x72, 0x65, 0x6E, 0x74, 0x20, 0x70, 0x72, 0x6F,
            0x74, 0x6F, 0x63, 0x6F, 0x6C,
        ];
        let s = from_utf8(&pstr).unwrap();
        println!("s => {}", s);
    }

    #[test]
    fn test_get_n_bytes_at() {
        let buf = [
            0x13, 0x42, 0x69, 0x74, 0x54, 0x6F, 0x72, 0x72, 0x65, 0x6E, 0x74, 0x20, 0x70, 0x72,
            0x6F, 0x74, 0x6F, 0x63, 0x6F, 0x6C, 0x0, 0x0, 0x0, 0x0, 0x0, 0x10, 0x0, 0x5, 0x20,
            0x9C, 0x82, 0x26, 0xB2, 0x99, 0xB3, 0x8, 0xBE, 0xAF, 0x2B, 0x9C, 0xD3, 0xFB, 0x49,
            0x21, 0x2D, 0xBD, 0x13, 0xEC, 0x2D, 0x54, 0x52, 0x32, 0x39, 0x34, 0x30, 0x2D, 0x74,
            0x65, 0x6A, 0x63, 0x67, 0x6D, 0x32, 0x6E, 0x74, 0x78, 0x74, 0x6F, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0,
        ]
        .to_vec();

        let (result, new_i) = get_n_bytes_at(&buf, 2, 7);
        print_byte_array("result", &result);
        assert_eq!(result.len(), 7);
        assert_eq!(result.get(0).unwrap(), &0x69);
        assert_eq!(result.get(6).unwrap(), &0x65);
        println!("new i = {}", new_i);
        assert_eq!(new_i, 8);
    }

    #[test]
    fn test_tcp_handshake_response() {
        let response = [
            0x13, 0x42, 0x69, 0x74, 0x54, 0x6F, 0x72, 0x72, 0x65, 0x6E, 0x74, 0x20, 0x70, 0x72,
            0x6F, 0x74, 0x6F, 0x63, 0x6F, 0x6C, 0x0, 0x0, 0x0, 0x0, 0x0, 0x10, 0x0, 0x5, 0x20,
            0x9C, 0x82, 0x26, 0xB2, 0x99, 0xB3, 0x8, 0xBE, 0xAF, 0x2B, 0x9C, 0xD3, 0xFB, 0x49,
            0x21, 0x2D, 0xBD, 0x13, 0xEC, 0x2D, 0x54, 0x52, 0x32, 0x39, 0x34, 0x30, 0x2D, 0x74,
            0x65, 0x6A, 0x63, 0x67, 0x6D, 0x32, 0x6E, 0x74, 0x78, 0x74, 0x6F, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0,
        ]
        .to_vec();

        let response = parse_handshake_response(&response);
        println!("response : {:?}", response);
        assert_eq!(response.protocol, "BitTorrent protocol".to_string());
        assert_eq!(
            response.info_hash,
            [
                0x20, 0x9C, 0x82, 0x26, 0xB2, 0x99, 0xB3, 0x8, 0xBE, 0xAF, 0x2B, 0x9C, 0xD3, 0xFB,
                0x49, 0x21, 0x2D, 0xBD, 0x13, 0xEC
            ]
            .to_vec()
        );
        assert_eq!(
            response.peer_id,
            [
                0x2D, 0x54, 0x52, 0x32, 0x39, 0x34, 0x30, 0x2D, 0x74, 0x65, 0x6A, 0x63, 0x67, 0x6D,
                0x32, 0x6E, 0x74, 0x78, 0x74, 0x6F
            ]
            .to_vec()
        );
    }

    #[test]
    fn connect_to_peer_live_test() {
        // List of 9 peers:
        // -> V4(66.108.98.51):34264
        // -> V4(219.91.135.231):60709
        // -> V4(216.36.15.101):51413
        // -> V4(208.72.192.229):33566
        // -> V4(84.17.53.169):51413
        // -> V4(79.88.178.148):24711
        // -> V4(74.58.115.36):49023
        // -> V4(71.58.252.122):7881
        // -> V4(68.168.178.21):51413
        let ip = Ipv4Addr::new(66, 108, 98, 51);
        let ip = IpAddr::V4(ip);
        let port = 34264;
        // connect_to_peer(ip, port);
    }

    #[test]
    fn test_make_choke_msg() {
        let msg = make_choke_msg();
        print_byte_array("choke", &msg);
        assert_eq!(msg.len(), 5); // should be 5 bytes long
        assert_eq!(msg.get(3).unwrap(), &1u8);
        assert_eq!(msg.get(4).unwrap(), &0u8);
    }

    #[test]
    fn test_make_unchoke_msg() {
        let msg = make_unchoke_msg();
        print_byte_array("unchoke", &msg);
        assert_eq!(msg.len(), 5); // should be 5 bytes long
        assert_eq!(msg.get(3).unwrap(), &1u8);
        assert_eq!(msg.get(4).unwrap(), &1u8);
    }

    #[test]
    fn test_make_have_msg() {
        let pi = 4;
        let msg = make_have_msg(pi);
        print_byte_array("have", &msg);
        assert_eq!(msg.len(), 9); // should be 9 bytes long
        assert_eq!(msg.get(3).unwrap(), &5u8);
        assert_eq!(msg.get(4).unwrap(), &4u8);
        assert_eq!(msg.get(8).unwrap(), &4u8);
    }

    #[test]
    fn test_bitfield() {
        let filepath = "big-buck-bunny.torrent";
        let torrent = Torrent::read_from_file(filepath).unwrap();

        let piece_size = torrent.piece_length;
        let num_pieces = torrent.pieces.len();
        println!("torrent has {} pieces of size: {}", num_pieces, piece_size);

        let resp = [0x0, 0x0, 0x0, 0x85, 0x5, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFE];
        let result = parse_peer_msg(&resp);
        println!("{:?}", result);
        assert_eq!(result.is_some(), true);
        match result.unwrap() {
            PeerMessage::Bitfield(id, bf) => {
                // ok
            },
            _ => {
                // not ok
                panic!("Should have been bitfield");
            }
        }

    }


}
