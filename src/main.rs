use std::fs;
use std::io::prelude::*;
use std::io::Cursor;
use std::fs::File;
use dns_lookup::{lookup_host, getnameinfo};

use lava_torrent::torrent::v1::Torrent;
use std::net::{UdpSocket, ToSocketAddrs, SocketAddr, IpAddr};

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

    // split host and port
    let url_pieces: Vec<String> = url.split(":").map(|s| s.to_string()).collect();
    println!("url pieces: {:?}", url_pieces);
    let host = url_pieces[0].as_str();
    let port = url_pieces[1].as_str();

    // lookup host ip
    let ips: Vec<IpAddr> = lookup_host(host).unwrap();
    println!("remote ips: {:?}", ips);
    // get the first one
    let ip = ips.get(0).unwrap().clone();
    let port= port.parse::<u16>().unwrap();
    let remote_sock = SocketAddr::new(ip, port);

    let (name, service) = match getnameinfo(&remote_sock, 0) {
        Ok((n, s)) => (n, s),
        Err(e) => panic!("failed to lookup sock: {:?}", e),
    };

    println!("socket name={} service={}", name, service);




    // println!("using remote sock addr: {:?}", remote_sock_addr);


    // // bind socket to local port
    let local_address  = "0.0.0.0:34254";
    let sock = UdpSocket::bind(local_address).expect("Couldnt bind to address");
    println!("udp socket bound to local port: {:?}", sock);

    // // connect to remote addr
    sock.connect(remote_sock).expect("couldnt connect");
    //
    // // send message to remote udp port
    // let msg_buf = "hello".as_bytes();
    // sock.send_to(msg_buf, url);
    //
    // let mut response_buf = [0; 512];
    // let num_bytes = sock.recv(&mut response_buf).expect("Didnt recieve data");
    // println!("recv message back {}", num_bytes);

}
