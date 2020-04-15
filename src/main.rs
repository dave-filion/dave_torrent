use std::fs;
use std::io::prelude::*;
use std::io::Cursor;
use std::fs::File;
use dns_lookup::{lookup_host, getnameinfo};
use rand::Rng;

use lava_torrent::torrent::v1::Torrent;
use std::net::{UdpSocket, ToSocketAddrs, SocketAddr, IpAddr};
use bytebuffer::ByteBuffer;

fn main() {
    // load torrent file data
    let filepath = "tears-of-steel.torrent";

    let torrent = Torrent::read_from_file(filepath).unwrap();

    println!("{}", torrent);
    println!("info hash: {}", torrent.info_hash());

    let total_size = torrent.length;
    let piece_size = torrent.piece_length;
    println!("Total size = {} bytes", total_size);
    println!("Piece size = {} bytes", piece_size);

    let all_files = torrent.files.expect("Should be files");
    println!("{} files:", all_files.len());
    for f in all_files {
        println!("{:?} : {} bytes", f.path, f.length);
    }

    let pieces = torrent.pieces;
    println!("{} pieces", pieces.len());

    let announce_url = torrent.announce.expect("Need announce");
    println!("announce: {}", announce_url);

    // check protocol of announce
    let pieces: Vec<String> = announce_url.split("://").map(|s| s.to_string()).collect(); // seperates protocol from url
    println!("pieces = {:?}", pieces);
    let protocol = pieces[0].as_str();

    if protocol != "udp" {
        panic!("cant handle non udp protocol: {}", protocol);
    }

    let url = pieces[1].as_str();
    println!("remote tracking url: {}", url);

    // convert to remote sock addr
    let remote_addr = url.to_socket_addrs().unwrap().next().unwrap();
    println!("remove addr {:?}", remote_addr);

    // // bind socket to local port
    let local_address  = "0.0.0.0:34254";
    let sock = UdpSocket::bind(local_address).expect("Couldnt bind to address");
    println!("udp socket bound to local port: {:?}", sock);

    // // connect to remote addr
    sock.connect(remote_addr).expect("couldnt connect");
    println!("socket connected to remote addr = {:?}", sock);

    // make connection request packet
    let mut bytebuf = ByteBuffer::new();

    // write connection id
    let conn_id : u64= 0x41727101980;
    bytebuf.write_u64(conn_id);

    // write action = 0
    let action: u32 = 0;
    bytebuf.write_u32(action);

    // write random tx id (4 bytes)
    let random_tx_id = rand::thread_rng().gen::<[u8; 4]>();
    println!("tx-id= {:X?}", random_tx_id);
    bytebuf.write_bytes(&random_tx_id);
    println!("buf len: {:?}", bytebuf.len());

    let packet = bytebuf.to_bytes();

    print!("Request => [");
    for b in &packet {
        print!(" {:X?}, ", b);
    }
    println!("]");

    // send message to remote udp port
    sock.send_to(&packet, url);

    // listen for response
    let mut response_buf = [0; 16];
    let _num_bytes = sock.recv(&mut response_buf).expect("Didnt recieve data");

    print!("Response => [");
    for i in 0..response_buf.len() {
        let byte = response_buf[i];
        print!(" {:X?}, ", byte);
    }
    println!("]");

    // get connection id from response (at index 8, 8 bytes)
    let mut conn_id_buff = [0; 8];
    let mut j = 0;
    for i in 8..16 {
        let byte = response_buf[i];
        conn_id_buff[j] = byte;
        j+=1;
    }

    print!("Conn id => [");
    for i in 0..conn_id_buff.len() {
        let byte = conn_id_buff[i];
        print!(" {:X?}, ", byte);
    }
    println!("]");


}
