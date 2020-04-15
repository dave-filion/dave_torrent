use std::fs;
use std::io::prelude::*;
use std::io::Cursor;
use std::fs::File;
use dns_lookup::{lookup_host, getnameinfo};
use rand::Rng;

use lava_torrent::torrent::v1::Torrent;
use std::net::{UdpSocket, ToSocketAddrs, SocketAddr, IpAddr};
use bytebuffer::ByteBuffer;
use byteorder::{ReadBytesExt, WriteBytesExt, BigEndian, LittleEndian};

fn print_byte_array(header: &str, bytes: &[u8]) {
    print!("{} => [", header);
    for b in bytes {
        print!(" {:X?}, ", b);
    }
    println!("]");
}

fn make_connect_packet() -> Vec<u8> {
    let mut bytebuf = ByteBuffer::new();

    // write connection id
    let conn_id : u64= 0x41727101980; // constant for connecting
    bytebuf.write_u64(conn_id);

    // write action = 0
    let action: u32 = 0;
    bytebuf.write_u32(action);

    // write random tx id (4 bytes)
    let random_tx_id = rand::thread_rng().gen::<[u8; 4]>();
    bytebuf.write_bytes(&random_tx_id);

    bytebuf.to_bytes()
}

fn get_conn_id_from_connect_response(response_buf: &[u8]) -> (u64, Vec<u8>) {
    // get connection id from response (at index 7, 8 bytes)
    let mut conn_id_buff = [0; 8];
    let mut j = 0;
    for i in 8..16 {
        let byte = response_buf[i];
        conn_id_buff[j] = byte;
        j+=1;
    }
    // convert conn id to u64
    let mut byte_rdr = Cursor::new(conn_id_buff);
    (byte_rdr.read_u64::<BigEndian>().unwrap(), conn_id_buff.to_vec())
}

fn make_announce_packet(conn_id_bytes: &Vec<u8>,
                        info_hash_bytes: &Vec<u8>,
    torrent_size: u64,
    port: u16
) -> Vec<u8> {
    let mut buf = ByteBuffer::new();

    // conn id (8 bytes)
    buf.write_bytes(&conn_id_bytes);

    // action (1 for announce, 4 bytes)
    buf.write_u32(1);

    // transaction_id (random 4 bytes)
    let tx_id = rand::thread_rng().gen::<[u8; 4]>();
    print_byte_array("random tx id", &tx_id);
    buf.write_bytes(&tx_id);

    // info hash (20 bytes)
    buf.write_bytes(&info_hash_bytes);

    // peer id (random 20 bytes)
    let peer_id = rand::thread_rng().gen::<[u8; 20]>();
    print_byte_array("random peer id", &peer_id);
    buf.write_bytes(&peer_id);

    // downloaded (8 bytes, just zeros here)
    buf.write_bytes(&[0; 8]);

    // left (8 bytes) size of torrent
    buf.write_u64(torrent_size);

    // uploaded (8 bytes, just zeros again)
    buf.write_bytes(&[0; 8]);

    // event (4 byte)
    buf.write_u32(0);

    // ip address (4 byte, also 0)
    buf.write_u32(0);

    // key (4 bytes, random)
    let key = rand::thread_rng().gen::<[u8; 4]>();
    print_byte_array("key", &key);
    buf.write_bytes(&key);

    // num_want (4 bytes, -1 is default)
    buf.write_i32(-1);

    // port (2 bytes)
    buf.write_u16(port);

    buf.to_bytes()
}

fn get_socket_addr(announce_url : &str) -> SocketAddr {
    let pieces: Vec<String> = announce_url.split("://").map(|s| s.to_string()).collect(); // seperates protocol from url
    let protocol = pieces[0].as_str();
    if protocol != "udp" {
        panic!("cant handle non udp protocol: {}", protocol);
    }

    let url = pieces[1].as_str();
    url.to_socket_addrs().unwrap().next().unwrap()
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
    let local_address  = "0.0.0.0:34254";
    let sock = UdpSocket::bind(local_address).expect("Couldnt bind to address");
    println!("udp socket bound to local port: {:?}", sock);

    // // connect to remote addr
    let remote_addr = get_socket_addr(announce_url.as_str());
    sock.connect(remote_addr).expect("couldnt connect");
    println!("socket connected to remote addr = {:?}", sock);

    // make connection request packet
    let connect_packet = make_connect_packet();
    print_byte_array("Request", &connect_packet);

    // send message to remote udp port
    sock.send(&connect_packet);
    // sock.send_to(&connect_packet, url);

    // listen for response
    let mut response_buf = [0; 16];
    let _num_bytes = sock.recv(&mut response_buf).expect("Didnt recieve data");
    print_byte_array("Response", &response_buf);

    // extract conn id
    let (conn_id_int, conn_id_bytes) = get_conn_id_from_connect_response(&response_buf);
    print_byte_array("conn id", &conn_id_bytes);

    // now send announce message
    let announce_packet = make_announce_packet(
        &conn_id_bytes,
        &info_hash_bytes,
        torrent.length as u64,
        34254 // default local port
    );
    print_byte_array("Announce", &announce_packet);



}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_make_announce_packet() {
        let port = 6969;
        let torrent_size = 1; // 1 so its easy to see in byte form
        let conn_id_bytes = [0x10,  0x38,  0x94,  0xC3,  0x73,  0x6B,  0x76,  0xB2].to_vec();
        let info_hash_bytes = [0xAA, 0x16, 0x30, 0x38, 0x78, 0x53, 0x79, 0x81, 0x90, 0x75, 0x43, 0x56, 0x11, 0x51, 0x73, 0x33, 0x45, 0x89, 0x19, 0xCC].to_vec();
        let result = make_announce_packet(&conn_id_bytes, &info_hash_bytes, torrent_size, port);

        print_byte_array("result", &result);
        assert_eq!(result.len(), 98);
    }

}