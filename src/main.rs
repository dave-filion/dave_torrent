use failure::Error;

use dave_torrent::announce::*;
use dave_torrent::download::{Block};
use dave_torrent::pieces::*;
use dave_torrent::peer::*;
use dave_torrent::*;
use dave_torrent::app::App;


fn main() -> Result<(), Error>{
    let filename = "big-buck-bunny.torrent";
    let mut app : App = App::new();
    app.download(filename);
    Ok(())
}
