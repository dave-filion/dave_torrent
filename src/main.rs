use rand::Rng;

use byteorder::{BigEndian, ByteOrder};
use lava_torrent::torrent::v1::Torrent;
use std::net::{UdpSocket, IpAddr};
use std::time::Duration;
use std::{thread, fs};
use failure::Error;
use crossbeam::crossbeam_channel::unbounded;

use dave_torrent::announce::*;
use dave_torrent::download::{Block};
use dave_torrent::pieces::*;
use dave_torrent::peer::*;
use dave_torrent::*;
use std::path::Path;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex, RwLock};
use dave_torrent::app::App;


fn main() -> Result<(), Error>{
    let filename = "big-buck-bunny.torrent";
    let app = App:new();
    app.download(filename);
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

}
