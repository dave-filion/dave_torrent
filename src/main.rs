use rand::Rng;
use std::io::{Cursor};

use bytebuffer::ByteBuffer;
use byteorder::{BigEndian, ByteOrder, ReadBytesExt};
use lava_torrent::torrent::v1::Torrent;
use std::net::{SocketAddr, ToSocketAddrs, UdpSocket, TcpStream, IpAddr};

use dave_torrent::*;

// tcp connection to peer
fn connect_to_peer(ip: IpAddr, port: u16) {
    let sock_addr = SocketAddr::new(ip, port);
    println!("connecting to remote socket at addr: {:?}", sock_addr);
    if let Ok(stream) = TcpStream::connect(sock_addr) {
        println!("connected to peer @ {:?}", sock_addr);
    } else {
        println!("Failed to connect to peer");
    }
}

fn main() {
    // load torrent file data
    let filepath = "tears-of-steel.torrent";

    let torrent = Torrent::read_from_file(filepath).unwrap();

    let info_hash = torrent.info_hash();
    let info_hash_bytes = torrent.info_hash_bytes();
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

    // send announce request
    let announce_resp = send_announce_req(
        &sock,
        &conn_id_bytes,
        &info_hash_bytes,
        torrent.length as u64,
        34264,
    );
    println!("Got announce result: {:?}", announce_resp);

    // get list of peers
    let peer_addrs = announce_resp.addresses;
    println!("List of {} peers:", peer_addrs.len());
    for p in &peer_addrs {
        let (addr, port) = p;
        println!("-> {:?}:{:?}", addr, port);
    }

    // try connecting to peers
    for (ip, port) in &peer_addrs {
        connect_to_peer(ip.clone(), port.clone());
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr};

    #[test]
    fn test_make_announce_packet() {
        let port = 6969;
        let torrent_size = 1; // 1 so its easy to see in byte form
        let conn_id_bytes = [0x10, 0x38, 0x94, 0xC3, 0x73, 0x6B, 0x76, 0xB2].to_vec();
        let info_hash_bytes = [
            0xAA, 0x16, 0x30, 0x38, 0x78, 0x53, 0x79, 0x81, 0x90, 0x75, 0x43, 0x56, 0x11, 0x51,
            0x73, 0x33, 0x45, 0x89, 0x19, 0xCC,
        ]
        .to_vec();
        let result = make_announce_packet(&conn_id_bytes, &info_hash_bytes, torrent_size, port);

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
        ].to_vec();

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
        let ip = Ipv4Addr::new(66,108,98,51);
        let ip = IpAddr::V4(ip);
        let port = 34264;
        connect_to_peer(ip, port);

    }
}
